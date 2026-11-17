//! GitHub Copilot's subscription: a GitHub token, traded for a short-lived
//! Copilot session, and its API (`api.githubcopilot.com`), which serves
//! each model on some of OpenAI Chat, OpenAI Responses and Anthropic
//! Messages, as its model list says. Requests go as Copilot Chat's do.
//!
//! Copilot's editors keep their sign-in in `github-copilot/apps.json` (or
//! the older `hosts.json`) under the config directory; pansy reads it and
//! never writes it. GitHub's tokens don't run out, so there is nothing to
//! refresh; a session is traded for again shortly before it ends.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use super::allowance::{self, Reading, Window};
use super::{Held, Kind, SignIn, Signed, Source, said};
use crate::provider::{Model, Protocol};

/// Where Copilot's API is when a session names none.
pub const BASE: &str = "https://api.githubcopilot.com";
const TOKEN_URL: &str = "https://api.github.com/copilot_internal/v2/token";
const USER_URL: &str = "https://api.github.com/user";
const ENTITLEMENT_URL: &str = "https://api.github.com/copilot_internal/user";
/// The OAuth app of Copilot's editors, and GitHub's device flow.
const CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";
const DEVICE_URL: &str = "https://github.com/login/device/code";
const OAUTH_URL: &str = "https://github.com/login/oauth/access_token";
pub(crate) const DEVICE_PAGE: &str = "https://github.com/login/device";

/// What Copilot Chat in VS Code sends.
const HEADERS: [(&str, &str); 4] = [
    ("editor-version", "vscode/1.140.0"),
    ("editor-plugin-version", "copilot-chat/0.68.0"),
    ("copilot-integration-id", "vscode-chat"),
    ("user-agent", "GitHubCopilotChat/0.68.0"),
];
/// Copilot Chat's API version, on every request to Copilot's API.
const API_VERSION: &str = "2026-01-09";

/// The folder Copilot's editors keep their sign-in in.
fn config_dir() -> PathBuf {
    #[cfg(windows)]
    if let Some(dir) = std::env::var_os("LOCALAPPDATA").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir).join("github-copilot");
    }
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::paths::user_home().join(".config"))
        .join("github-copilot")
}

/// The files Copilot's editors keep their sign-in in, the newer first.
pub(crate) fn files() -> Vec<PathBuf> {
    let dir = config_dir();
    vec![dir.join("apps.json"), dir.join("hosts.json")]
}

/// Copilot's editors' sign-in on github.com, and its token as
/// `{"user", "oauth_token"}`.
pub fn own() -> Option<(SignIn, Value)> {
    for file in files() {
        let Some(apps) = std::fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Map<String, Value>>(&b).ok())
        else {
            continue;
        };
        let mut keys: Vec<&String> = apps.keys().collect();
        keys.sort();
        for key in keys {
            let host = key.split(':').next().unwrap_or_default();
            let app = &apps[key];
            let token = app["oauth_token"].as_str().unwrap_or_default();
            if host != "github.com" || token.is_empty() {
                continue;
            }
            let user = app["user"].as_str().unwrap_or("GitHub").to_owned();
            let sign_in = SignIn {
                kind: Kind::Copilot,
                user: user.clone(),
                plan: String::new(),
                source: Source::Agent,
                project: String::new(),
            };
            return Some((sign_in, json!({"user": user, "oauth_token": token})));
        }
    }
    None
}

/// A Copilot session: its token, when it ends, and where its API is.
#[derive(Debug, Clone)]
struct Session {
    token: String,
    expires_at: i64,
    api: String,
}

/// Sessions by the GitHub token they were traded for.
static SESSIONS: LazyLock<Mutex<HashMap<String, Session>>> = LazyLock::new(Default::default);

/// The session `token` names is refused: the next request trades for another.
pub(crate) fn forget(token: &str) {
    SESSIONS.lock().unwrap().retain(|_, s| s.token != token);
}

fn editor(request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    HEADERS.iter().fold(request, |r, (k, v)| r.header(*k, *v))
}

/// A session for the GitHub token, traded for again shortly before it ends.
async fn session(github: &str, http: &reqwest::Client) -> Result<Session> {
    let now = chrono::Utc::now().timestamp();
    if let Some(s) = SESSIONS.lock().unwrap().get(github)
        && s.expires_at - now > 120
    {
        return Ok(s.clone());
    }
    let response = editor(http.get(TOKEN_URL))
        .header("authorization", format!("token {github}"))
        .header("accept", "application/json")
        .send()
        .await
        .map_err(|e| anyhow!("Copilot's sign-in: {e}"))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    let v: Value = serde_json::from_str(&text).unwrap_or_default();
    let token = v["token"].as_str().unwrap_or_default();
    if !status.is_success() || token.is_empty() {
        bail!(
            "Copilot is signed out ({}); sign in to Copilot again",
            said(status, &text)
        );
    }
    let session = Session {
        token: token.to_owned(),
        expires_at: v["expires_at"].as_i64().unwrap_or(now + 600),
        api: v["endpoints"]["api"]
            .as_str()
            .unwrap_or(BASE)
            .trim_end_matches('/')
            .to_owned(),
    };
    SESSIONS
        .lock()
        .unwrap()
        .insert(github.to_owned(), session.clone());
    Ok(session)
}

fn github_token(held: &Held) -> Result<String> {
    let auth = held.read(Kind::Copilot)?;
    auth["oauth_token"]
        .as_str()
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("Copilot's sign-in has no token; sign in again"))
}

/// What a request with `session` carries, whatever it asks.
fn session_headers(session: &Session) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = HEADERS
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    out.push(("authorization".into(), format!("Bearer {}", session.token)));
    out.push(("x-github-api-version".into(), API_VERSION.into()));
    out
}

/// Signs a request to Copilot's API as Copilot Chat signs its own: a turn
/// the user typed is billed as one, a tool's answer is the agent's.
pub(crate) async fn sign(held: &Held, http: &reqwest::Client, body: &[u8]) -> Result<Signed> {
    let session = session(&github_token(held)?, http).await?;
    let mut headers = session_headers(&session);
    headers.push(("openai-intent".into(), "conversation-panel".into()));
    let initiator = if last_role(body) == "user" {
        "user"
    } else {
        "agent"
    };
    headers.push(("x-initiator".into(), initiator.into()));
    if has_image(body) {
        headers.push(("copilot-vision-request".into(), "true".into()));
    }
    Ok(Signed {
        headers,
        base: (session.api != BASE).then(|| session.api.clone()),
        token: session.token,
        body: None,
    })
}

fn has_image(body: &[u8]) -> bool {
    let has = |needle: &[u8]| body.windows(needle.len()).any(|w| w == needle);
    has(br#""image_url""#) || has(br#""input_image""#) || has(br#""type":"image""#)
}

/// The role of the last message of a Chat, Anthropic or Responses request;
/// `tool` for an Anthropic user turn of tool results alone.
pub(crate) fn last_role(body: &[u8]) -> String {
    let Ok(v) = serde_json::from_slice::<Value>(body) else {
        return String::new();
    };
    if let Some(last) = v["messages"].as_array().and_then(|m| m.last()) {
        let role = last["role"].as_str().unwrap_or_default();
        if role == "user"
            && let Some(blocks) = last["content"].as_array()
            && !blocks.is_empty()
            && blocks.iter().all(|b| b["type"] == "tool_result")
        {
            return "tool".into();
        }
        return role.to_owned();
    }
    match &v["input"] {
        Value::String(_) => "user".into(),
        Value::Array(items) => items
            .last()
            .and_then(|i| i["role"].as_str())
            .unwrap_or_default()
            .to_owned(),
        _ => String::new(),
    }
}

/// The APIs of a model's `supported_endpoints`; none listed is chat
/// completions alone (the older models).
fn apis_of(endpoints: &[Value]) -> Vec<Protocol> {
    let mut out: Vec<Protocol> = endpoints
        .iter()
        .filter_map(Value::as_str)
        .filter_map(|e| match e {
            "/chat/completions" => Some(Protocol::Chat),
            "/responses" => Some(Protocol::Responses),
            "/v1/messages" => Some(Protocol::Anthropic),
            _ => None,
        })
        .collect();
    if out.is_empty() {
        out.push(Protocol::Chat);
    }
    out.dedup();
    out
}

/// A model id nobody picks: Copilot's own helpers.
fn internal(id: &str) -> bool {
    ["copilot-search", "exec-agent", "trajectory"]
        .iter()
        .any(|p| id.starts_with(p))
        || ["-secondary", "-tertiary", "-4th", "-free-auto"]
            .iter()
            .any(|s| id.ends_with(s))
}

/// The chat models of a `/models` answer the account may use: those its
/// picker offers and its policy has on.
pub(crate) fn parse_models(list: &Value) -> Vec<Model> {
    let mut out = Vec::new();
    for m in list["data"].as_array().into_iter().flatten() {
        let id = m["id"].as_str().unwrap_or_default();
        let caps = &m["capabilities"];
        if id.is_empty() || caps["type"] != "chat" || internal(id) {
            continue;
        }
        let picker = m["model_picker_enabled"].as_bool().unwrap_or(false);
        let category = m["model_picker_category"].as_str().unwrap_or_default();
        if !picker && (category.is_empty() || m["vendor"] == "Experimental") {
            continue;
        }
        let usable = match m["policy"]["state"].as_str() {
            Some(state) => state == "enabled",
            None => picker,
        };
        if !usable || out.iter().any(|o: &Model| o.id == id) {
            continue;
        }
        let supports = &caps["supports"];
        out.push(Model {
            id: id.to_owned(),
            name: m["name"].as_str().unwrap_or_default().to_owned(),
            context: caps["limits"]["max_prompt_tokens"]
                .as_u64()
                .or(caps["limits"]["max_context_window_tokens"].as_u64())
                .filter(|c| *c > 0),
            max_output: caps["limits"]["max_output_tokens"]
                .as_u64()
                .filter(|c| *c > 0),
            reasoning: supports["reasoning_effort"]
                .as_array()
                .is_some_and(|e| !e.is_empty())
                || supports["adaptive_thinking"] == true,
            vision: supports["vision"] == true,
            apis: apis_of(m["supported_endpoints"].as_array().map_or(&[], |v| v)),
            api_id: None,
        });
    }
    out
}

/// Copilot's plans as GitHub names them.
pub(crate) fn plan_name(plan: &str, sku: &str) -> String {
    match sku {
        "free_educational_quota" => return "Education".into(),
        "free_limited_copilot" => return "Free".into(),
        _ => {}
    }
    match plan.to_ascii_lowercase().as_str() {
        "" => String::new(),
        "free" => "Free".into(),
        "individual" => "Pro".into(),
        "individual_pro" => "Pro+".into(),
        "individual_max" => "Max".into(),
        "individual_edu" => "Education".into(),
        "business" => "Business".into(),
        "enterprise" => "Enterprise".into(),
        other => {
            let mut c = other.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect())
                .unwrap_or_default()
        }
    }
}

/// The plan the GitHub token's account has; an error when it has none.
async fn plan(github: &str, http: &reqwest::Client) -> Result<String> {
    let response = http
        .get(ENTITLEMENT_URL)
        .header("authorization", format!("token {github}"))
        .header("accept", "application/json")
        .send()
        .await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if matches!(status.as_u16(), 401 | 403 | 404) {
        bail!("this GitHub account has no Copilot subscription");
    }
    if !status.is_success() {
        bail!("Copilot: {}", said(status, &text));
    }
    let v: Value = serde_json::from_str(&text).unwrap_or_default();
    Ok(plan_name(
        v["copilot_plan"].as_str().unwrap_or_default(),
        v["access_type_sku"].as_str().unwrap_or_default(),
    ))
}

/// The models the account may use, and its plan.
pub(crate) async fn models(held: &Held, http: &reqwest::Client) -> Result<(Vec<Model>, String)> {
    let github = github_token(held)?;
    let session = session(&github, http).await?;
    let mut request = http.get(format!("{}/models", session.api));
    for (k, v) in session_headers(&session) {
        request = request.header(k, v);
    }
    let response = request.send().await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        if status.as_u16() == 401 {
            forget(&session.token);
        }
        bail!("Copilot's models: {}", said(status, &text));
    }
    let models = parse_models(&serde_json::from_str(&text).unwrap_or_default());
    if models.is_empty() {
        bail!("Copilot listed no chat models for this account");
    }
    let plan = plan(&github, http).await.unwrap_or_default();
    Ok((models, plan))
}

// --- Allowance ----------------------------------------------------------------------

/// A JSON number, or a string of one (as GitHub sends them under
/// token-based billing).
fn number(v: &Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
        .filter(|n: &f64| n.is_finite())
}

/// The account's monthly allowances, as Copilot's editors read them.
pub(crate) async fn usage(held: &Held, http: &reqwest::Client) -> Result<Reading> {
    let github = github_token(held)?;
    let response = editor(http.get(ENTITLEMENT_URL))
        .header("authorization", format!("token {github}"))
        .header("accept", "application/json")
        .send()
        .await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("Copilot's usage: {}", said(status, &text));
    }
    Ok(parse_usage(
        &serde_json::from_str(&text).unwrap_or_default(),
    ))
}

pub(crate) fn parse_usage(v: &Value) -> Reading {
    let month = v["quota_reset_date_utc"]
        .as_str()
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.with_timezone(&chrono::Utc))
        .or_else(|| {
            v["quota_reset_date"]
                .as_str()
                .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
                .and_then(|d| d.and_hms_opt(0, 0, 0))
                .map(|t| t.and_utc())
        });
    let mut windows = Vec::new();
    let mut unlimited = Vec::new();
    for (id, name) in [
        ("premium_interactions", "Premium requests"),
        ("chat", "Chat requests"),
        ("completions", "Completions"),
    ] {
        let w = &v["quota_snapshots"][id];
        if !w.is_object() {
            continue;
        }
        let resets_at = number(&w["quota_reset_at"])
            .filter(|t| *t > 0.0)
            .and_then(|t| chrono::DateTime::from_timestamp(t as i64, 0))
            .or(month);
        let window = Window {
            name: name.to_owned(),
            resets_at,
            span_secs: 30 * 24 * 3600,
            ..Window::default()
        };
        if w["unlimited"] == true {
            // An organization's pooled premium allowance reads unlimited,
            // and has_quota false once the pool is spent.
            if id == "premium_interactions" && w["has_quota"] == false {
                windows.push(Window {
                    used: 100.0,
                    ..window
                });
            } else {
                unlimited.push(Window {
                    unlimited: true,
                    resets_at: None,
                    ..window
                });
            }
            continue;
        }
        let entitlement = number(&w["entitlement"]);
        if entitlement.is_some_and(|e| e <= 0.0) {
            continue;
        }
        let (used, display) = match (entitlement, number(&w["quota_remaining"])) {
            (Some(e), Some(left)) => {
                let n = (e - left).max(0.0);
                (
                    (100.0 * n / e).min(100.0),
                    format!("{} / {}", allowance::compact(n), allowance::compact(e)),
                )
            }
            _ => match number(&w["percent_remaining"]) {
                Some(p) => ((100.0 - p).clamp(0.0, 100.0), String::new()),
                None => continue,
            },
        };
        windows.push(Window {
            used,
            display,
            ..window
        });
    }
    windows.extend(unlimited);
    Reading {
        plan: plan_name(
            v["copilot_plan"].as_str().unwrap_or_default(),
            v["access_type_sku"].as_str().unwrap_or_default(),
        ),
        windows,
        // Spent premium requests leave the base models: not held.
        spent_until: None,
        note: String::new(),
    }
}

// --- Signing in ---------------------------------------------------------------------

/// A code to type at github.com/login/device.
pub(crate) struct DeviceCode {
    pub device: String,
    pub code: String,
    pub page: String,
    pub interval: u64,
}

async fn github_form(http: &reqwest::Client, url: &str, form: &[(&str, &str)]) -> Result<Value> {
    let response = http
        .post(url)
        .header("accept", "application/json")
        .form(form)
        .send()
        .await
        .map_err(|e| anyhow!("couldn't reach GitHub: {e}"))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("GitHub: {}", said(status, &text));
    }
    Ok(serde_json::from_str(&text).unwrap_or_default())
}

/// Asks GitHub for a device code, as Copilot's editors sign in.
pub(crate) async fn device_code(http: &reqwest::Client) -> Result<DeviceCode> {
    let v = github_form(
        http,
        DEVICE_URL,
        &[("client_id", CLIENT_ID), ("scope", "read:user")],
    )
    .await?;
    let device = v["device_code"].as_str().unwrap_or_default();
    let code = v["user_code"].as_str().unwrap_or_default();
    if device.is_empty() || code.is_empty() {
        bail!("GitHub gave no device code");
    }
    Ok(DeviceCode {
        device: device.to_owned(),
        code: code.to_owned(),
        page: v["verification_uri"]
            .as_str()
            .unwrap_or(DEVICE_PAGE)
            .to_owned(),
        interval: v["interval"].as_u64().unwrap_or(5).max(1),
    })
}

/// What one ask for the device's token came to.
pub(crate) enum Poll {
    /// The code is not typed yet.
    Waiting,
    /// GitHub asks to be asked less often.
    Slower,
    /// Signed in: the account's token, who it is and its plan.
    Done(Value, String, String),
}

pub(crate) async fn device_poll(http: &reqwest::Client, device: &DeviceCode) -> Result<Poll> {
    let v = github_form(
        http,
        OAUTH_URL,
        &[
            ("client_id", CLIENT_ID),
            ("device_code", device.device.as_str()),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ],
    )
    .await?;
    match v["error"].as_str() {
        Some("authorization_pending") => return Ok(Poll::Waiting),
        Some("slow_down") => return Ok(Poll::Slower),
        Some("expired_token") => bail!("the code expired; start again"),
        Some("access_denied") => bail!("the sign-in was declined on GitHub"),
        Some(other) => bail!(
            "GitHub: {other} {}",
            v["error_description"].as_str().unwrap_or_default()
        ),
        None => {}
    }
    let token = v["access_token"]
        .as_str()
        .filter(|t| !t.is_empty())
        .ok_or_else(|| anyhow!("GitHub gave no token"))?;
    let user = github_user(token, http).await?;
    let plan = plan(token, http)
        .await
        .map_err(|e| anyhow!("{user}: {e}"))?;
    Ok(Poll::Done(
        json!({"user": user, "oauth_token": token}),
        user,
        plan,
    ))
}

/// Whose a GitHub token is.
async fn github_user(token: &str, http: &reqwest::Client) -> Result<String> {
    let response = http
        .get(USER_URL)
        .header("authorization", format!("token {token}"))
        .header("accept", "application/json")
        .send()
        .await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    let v: Value = serde_json::from_str(&text).unwrap_or_default();
    v["login"]
        .as_str()
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            anyhow!(
                "GitHub didn't say whose account this is ({})",
                said(status, &text)
            )
        })
}
