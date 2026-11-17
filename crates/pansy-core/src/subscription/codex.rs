//! ChatGPT's subscription, as Codex uses it: the Codex backend at
//! `chatgpt.com`, which answers OpenAI Responses in a stream, to a request
//! made the way Codex CLI makes it (its headers, its instructions, nothing
//! stored), as magpie makes it.
//!
//! Codex keeps its sign-in in `$CODEX_HOME/auth.json`; one signed in from
//! pansy is kept the same way in `logins.json`. An access token about to
//! run out is refreshed with Codex's own OAuth client, and the new tokens
//! go back where they were read, every other field as it was.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::SystemTime;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use super::allowance::{self, Reading, Window};
use super::{Held, Kind, SignIn, Signed, Source, said};
use crate::provider::Model;

/// Where a ChatGPT account's Codex requests go.
pub const BASE: &str = "https://chatgpt.com/backend-api/codex";
/// Codex CLI's own OAuth client, and where it signs in.
pub(crate) const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub(crate) const ISSUER: &str = "https://auth.openai.com";
/// The Codex CLI version requests say they come from when no newer one is
/// known: the backend leaves out models newer than the client asking.
const VERSION: &str = "0.166.0";
/// Codex's instructions, for a model whose own the backend hasn't given
/// (openai/codex, Apache-2.0, as magpie carries them).
const PROMPT: &str = include_str!("codex_prompt.md");

/// `CODEX_HOME`, else `~/.codex`.
pub fn home() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::paths::user_home().join(".codex"))
}

/// Codex's sign-in.
pub fn auth_path() -> PathBuf {
    home().join("auth.json")
}

pub(crate) fn read_own() -> Result<Value> {
    let path = auth_path();
    let bytes = std::fs::read(&path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => anyhow!("Codex is signed out; run codex login"),
        _ => anyhow!("reading {}: {e}", path.display()),
    })?;
    serde_json::from_slice(&bytes).with_context(|| format!("reading {}", path.display()))
}

/// Puts a refreshed sign-in back in Codex's `auth.json`, whole or not at all.
pub(crate) fn write_own(auth: &Value) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(auth)?;
    bytes.push(b'\n');
    crate::fsutil::write_atomic(&auth_path(), &bytes, true)
}

/// Codex's own sign-in to ChatGPT, when it has one (an API key is not).
pub fn own() -> Option<SignIn> {
    let auth = read_own().ok()?;
    if auth["auth_mode"] == "apikey" || access(&auth).is_empty() {
        return None;
    }
    let (user, plan) = who(&auth);
    Some(SignIn {
        kind: Kind::Codex,
        user,
        plan,
        source: Source::Agent,
        project: String::new(),
    })
}

fn access(auth: &Value) -> &str {
    auth["tokens"]["access_token"].as_str().unwrap_or_default()
}

fn refresh_token(auth: &Value) -> &str {
    auth["tokens"]["refresh_token"].as_str().unwrap_or_default()
}

/// Who a sign-in is, by its ID token: the email (with a workspace's plan
/// after it, to tell its seat from a personal plan of the same email), and
/// the plan.
pub(crate) fn who(auth: &Value) -> (String, String) {
    let claims = super::jwt_claims(auth["tokens"]["id_token"].as_str().unwrap_or_default());
    let plan = claims["https://api.openai.com/auth"]["chatgpt_plan_type"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let email = claims["email"].as_str().unwrap_or_default();
    let user = match plan.as_str() {
        "team" | "business" | "enterprise" | "edu" if !email.is_empty() => {
            format!("{email} · {}", capitalized(&plan))
        }
        _ if email.is_empty() => "ChatGPT".to_owned(),
        _ => email.to_owned(),
    };
    (user, plan)
}

fn capitalized(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

/// The ChatGPT account a sign-in's requests name.
fn account_id(auth: &Value) -> String {
    if let Some(id) = auth["tokens"]["account_id"]
        .as_str()
        .filter(|s| !s.is_empty())
    {
        return id.to_owned();
    }
    super::jwt_claims(auth["tokens"]["id_token"].as_str().unwrap_or_default())
        ["https://api.openai.com/auth"]["chatgpt_account_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

// --- Tokens ---------------------------------------------------------------------

/// Access tokens the backend refused while their time was not up: the next
/// request refreshes them.
static STALE: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Default::default);
/// One refresh at a time: OpenAI turns the refresh token over, and a second
/// refresh with the one the first spent is refused.
static REFRESHING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(crate) fn stale(token: &str) {
    STALE.lock().unwrap().insert(token.to_owned());
}

fn fresh(token: &str) -> bool {
    if STALE.lock().unwrap().contains(token) {
        return false;
    }
    match super::jwt_claims(token)["exp"].as_i64() {
        Some(exp) => exp - chrono::Utc::now().timestamp() > 5 * 60,
        None => true,
    }
}

/// A usable access token of the account, and its ChatGPT account id:
/// refreshed first when it is about to run out or the backend refused it.
pub(crate) async fn token(held: &Held, http: &reqwest::Client) -> Result<(String, String)> {
    let auth = held.read(Kind::Codex)?;
    if !access(&auth).is_empty() && fresh(access(&auth)) {
        return Ok((access(&auth).to_owned(), account_id(&auth)));
    }
    let _one = REFRESHING.lock().await;
    // Read again: refreshed meanwhile, by another request or by Codex.
    let mut auth = held.read(Kind::Codex)?;
    if access(&auth).is_empty() {
        bail!("Codex is signed out; run codex login");
    }
    if fresh(access(&auth)) {
        return Ok((access(&auth).to_owned(), account_id(&auth)));
    }
    let reason = if STALE.lock().unwrap().contains(access(&auth)) {
        "unauthorized_recovery"
    } else {
        "proactive_expiry"
    };
    let spent = refresh_token(&auth).to_owned();
    let token = refresh(http, &mut auth, reason).await?;
    // Only over the sign-in the refresh began from: one signed in to since
    // (another account, or signed out) is not written over. The new token
    // still serves this request.
    match held.read(Kind::Codex) {
        Ok(now) if refresh_token(&now) == spent => {
            if let Err(e) = held.write(Kind::Codex, &auth) {
                tracing::warn!("keeping Codex's refreshed sign-in: {e:#}");
            }
        }
        _ => tracing::info!("Codex's sign-in changed while it was refreshed; left as it is"),
    }
    Ok((token, account_id(&auth)))
}

/// Renews the tokens of an `auth.json`-shaped sign-in in place, as codex-rs
/// asks for them; the new access token.
async fn refresh(http: &reqwest::Client, auth: &mut Value, reason: &str) -> Result<String> {
    let body = json!({
        "client_id": CLIENT_ID,
        "grant_type": "refresh_token",
        "refresh_token": refresh_token(auth),
        "refresh_reason": reason,
    });
    let response = client_headers(http.post(format!("{ISSUER}/oauth/token")), &version(None))
        .json(&body)
        .send()
        .await
        .map_err(|e| anyhow!("refreshing Codex's sign-in: {e}"))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    let fresh: Value = serde_json::from_str(&text).unwrap_or_default();
    let Some(token) = fresh["access_token"].as_str().filter(|t| !t.is_empty()) else {
        if matches!(status.as_u16(), 400 | 401) {
            bail!(
                "ChatGPT signed this account out ({}); sign in to it again",
                said(status, &text)
            );
        }
        bail!(
            "refreshing Codex's sign-in failed ({})",
            said(status, &text)
        );
    };
    let tokens = auth["tokens"]
        .as_object_mut()
        .ok_or_else(|| anyhow!("Codex's sign-in has no tokens"))?;
    tokens.insert("access_token".into(), json!(token));
    for key in ["id_token", "refresh_token"] {
        if let Some(v) = fresh[key].as_str().filter(|v| !v.is_empty()) {
            tokens.insert(key.into(), json!(v));
        }
    }
    auth["last_refresh"] = json!(chrono::Utc::now().to_rfc3339());
    Ok(token.to_owned())
}

/// What every request Codex CLI makes carries: its originator and its
/// User-Agent.
fn client_headers(request: reqwest::RequestBuilder, version: &str) -> reqwest::RequestBuilder {
    request
        .header("originator", "codex_cli_rs")
        .header("user-agent", user_agent(version))
}

/// The User-Agent Codex CLI of `version` sends:
/// `codex_cli_rs/0.166.0 (Linux 6.8.0; x86_64) xterm-256color`.
fn user_agent(version: &str) -> String {
    static OS: LazyLock<String> = LazyLock::new(|| match std::env::consts::OS {
        "macos" => "Mac OS".to_owned(),
        "windows" => "Windows".to_owned(),
        _ => std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .ok()
            .and_then(|r| r.split('-').next().map(|v| format!("Linux {}", v.trim())))
            .unwrap_or_else(|| "Linux".to_owned()),
    });
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        other => other,
    };
    let term = match std::env::consts::OS {
        "macos" => "Apple_Terminal/455",
        "windows" => "WindowsTerminal",
        _ => "xterm-256color",
    };
    format!("codex_cli_rs/{version} ({}; {arch}) {term}", *OS)
}

/// The Codex CLI version requests name: the newest of pansy's, the one Codex
/// last asked for its models with, and `seen`, the one a Codex coming
/// through the gateway said it is.
pub fn version(seen: Option<&str>) -> String {
    let mut best = VERSION.to_owned();
    let cached = std::fs::read(home().join("models_cache.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v["client_version"].as_str().map(str::to_owned));
    for v in cached.iter().map(String::as_str).chain(seen) {
        if let Some(v) = semver(v)
            && newer(&v, &best)
        {
            best = v;
        }
    }
    best
}

/// The first `1.2.3` in `text`.
fn semver(text: &str) -> Option<String> {
    let start = text.find(|c: char| c.is_ascii_digit())?;
    let v: String = text[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let v = v.trim_end_matches('.');
    (v.split('.').count() >= 2).then(|| v.to_owned())
}

fn newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> { v.split('.').map(|p| p.parse().unwrap_or(0)).collect() };
    parts(a) > parts(b)
}

/// Signs a request to the backend as Codex CLI signs its own.
pub(crate) async fn sign(
    held: &Held,
    http: &reqwest::Client,
    body: &[u8],
    client_version: Option<&str>,
) -> Result<Signed> {
    let (token, account) = token(held, http).await?;
    let version = version(client_version);
    let mut headers = vec![
        ("authorization".to_owned(), format!("Bearer {token}")),
        (
            "openai-beta".to_owned(),
            "responses=experimental".to_owned(),
        ),
        ("originator".to_owned(), "codex_cli_rs".to_owned()),
        ("user-agent".to_owned(), user_agent(&version)),
        ("version".to_owned(), version),
        ("accept".to_owned(), "text/event-stream".to_owned()),
    ];
    if !account.is_empty() {
        headers.push(("chatgpt-account-id".to_owned(), account));
    }
    // The backend finds a conversation's cached prompt by these.
    if let Some(key) = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| v["prompt_cache_key"].as_str().map(str::to_owned))
        .filter(|k| !k.is_empty())
    {
        headers.push(("session-id".to_owned(), key.clone()));
        headers.push(("thread-id".to_owned(), key));
    }
    Ok(Signed {
        headers,
        base: None,
        token,
        body: None,
    })
}

// --- The request as Codex sends it ------------------------------------------------

/// Makes a Responses request the one Codex CLI would send: streamed,
/// stored nowhere (so its items carry no ids), with Codex's instructions,
/// the client's own first in the input, and without the knobs the backend
/// turns away.
pub fn shape(body: &mut Value) {
    let Some(m) = body.as_object_mut() else {
        return;
    };
    for key in [
        "max_output_tokens",
        "max_completion_tokens",
        "temperature",
        "top_p",
        "previous_response_id",
        "user",
        "safety_identifier",
        "prompt_cache_options",
        "metadata",
    ] {
        m.remove(key);
    }
    // Codex's Fast mode asks for "priority"; no other tier is taken.
    if !matches!(
        m.get("service_tier").and_then(Value::as_str),
        Some("priority" | "ultrafast")
    ) {
        m.remove("service_tier");
    }
    let model = m
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let mut input = match m.remove("input") {
        Some(Value::Array(items)) => items,
        Some(Value::String(text)) => vec![json!({"type": "message", "role": "user",
            "content": [{"type": "input_text", "text": text}]})],
        _ => Vec::new(),
    };
    input = fit_input(input);
    let own = m
        .get("instructions")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    if m.get("prompt_cache_key")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
        && let Some(key) = conversation_key(&own, &input)
    {
        m.insert("prompt_cache_key".into(), json!(key));
    }
    // The client's instructions go first as a developer message, Codex's in
    // their place, unless they are Codex's (from Codex itself).
    let mut codex = instructions(&model);
    if !own.trim().is_empty() {
        if first_line(&own) == first_line(&codex) {
            codex = own;
        } else {
            input.insert(
                0,
                json!({"type": "message", "role": "developer",
                    "content": [{"type": "input_text", "text": own}]}),
            );
        }
    }
    m.insert("input".into(), Value::Array(input));
    m.insert("instructions".into(), json!(codex));
    m.insert("store".into(), json!(false));
    m.insert("stream".into(), json!(true));
    m.insert("tool_choice".into(), json!("auto"));
    m.entry("parallel_tool_calls").or_insert(json!(true));
    m.entry("text").or_insert(json!({"verbosity": "medium"}));
    // Reasoning, and its sealed content to carry across turns, unless it
    // is turned off.
    let effort = m
        .get("reasoning")
        .and_then(|r| r["effort"].as_str())
        .map(str::to_owned);
    if effort.as_deref() == Some("none") {
        m.remove("include");
    } else {
        if let Some(reasoning) = m.get_mut("reasoning").and_then(Value::as_object_mut) {
            if effort.as_deref() == Some("ultra") {
                reasoning.insert("effort".into(), json!("max"));
            }
            reasoning.entry("summary").or_insert(json!("auto"));
        }
        let include = m.entry("include").or_insert(json!([]));
        if !include.is_array() {
            *include = json!([]);
        }
        let list = include.as_array_mut().unwrap();
        if !list.iter().any(|v| v == "reasoning.encrypted_content") {
            list.push(json!("reasoning.encrypted_content"));
        }
    }
}

fn first_line(s: &str) -> &str {
    s.trim().lines().next().unwrap_or_default()
}

/// A call's id as the backend takes one: as it is within 64 characters,
/// else its SHA-256, the same each time, so a call and its output stay
/// paired.
pub(super) fn bound_call_id(id: &str) -> String {
    if id.len() <= 64 {
        return id.to_owned();
    }
    hex(&Sha256::digest(id.trim().as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Input items that fit a request that keeps nothing: no references to
/// stored items, no ids (nor a message's status, which goes with its id),
/// no system role (developer in its place), and a tool's output whose call
/// isn't there told as a note of the assistant's.
pub(super) fn fit_input(input: Vec<Value>) -> Vec<Value> {
    const CALLS: [(&str, &str); 3] = [
        ("function_call", "function_call_output"),
        ("local_shell_call", "local_shell_call_output"),
        ("custom_tool_call", "custom_tool_call_output"),
    ];
    let mut calls: HashMap<String, &str> = HashMap::new();
    for item in &input {
        let kind = item["type"].as_str().unwrap_or_default();
        if let Some((_, output)) = CALLS.iter().find(|(call, _)| *call == kind) {
            let id = bound_call_id(item["call_id"].as_str().unwrap_or_default());
            calls.insert(id.trim().to_owned(), output);
        }
    }
    let mut out = Vec::with_capacity(input.len());
    for mut item in input {
        let Some(it) = item.as_object_mut() else {
            out.push(item);
            continue;
        };
        let kind = it
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if kind == "item_reference" {
            continue;
        }
        if let Some(id) = it.get("call_id").and_then(Value::as_str) {
            let bound = bound_call_id(id);
            if bound != id {
                it.insert("call_id".into(), json!(bound));
            }
        }
        it.remove("id");
        if matches!(kind.as_str(), "" | "message" | "reasoning") {
            it.remove("status");
        }
        if kind.ends_with("_output") {
            let id = it
                .get("call_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_owned();
            let want = calls.get(&id).copied();
            let paired = want == Some(kind.as_str())
                || (kind == "function_call_output" && want == Some("local_shell_call_output"));
            if !id.is_empty() && !paired {
                out.push(orphan_output(it, &id));
                continue;
            }
        }
        if it.get("role").and_then(Value::as_str) == Some("system")
            && matches!(kind.as_str(), "" | "message")
        {
            it.insert("role".into(), json!("developer"));
        }
        out.push(item);
    }
    out
}

/// A tool's output whose call isn't in the input, as the assistant's note.
fn orphan_output(it: &Map<String, Value>, id: &str) -> Value {
    let name = it.get("name").and_then(Value::as_str).unwrap_or("tool");
    let mut text = match it.get("output") {
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    };
    if text.len() > 16000 {
        let mut cut = 16000;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        text.push_str("\n...[truncated]");
    }
    json!({"type": "message", "role": "assistant",
        "content": format!("[Previous {name} result; call_id={id}]: {text}")})
}

/// The conversation's key, for a client that names none: taken from how it
/// starts (the instructions and the input up to the first user message),
/// which each later turn repeats.
pub(super) fn conversation_key(instructions: &str, input: &[Value]) -> Option<String> {
    let mut hash = Sha256::new();
    hash.update(instructions.as_bytes());
    let mut user = false;
    for item in input {
        hash.update(item.to_string().as_bytes());
        if item["role"] == "user" {
            user = true;
            break;
        }
    }
    if !user {
        return None;
    }
    let h = hex(&hash.finalize());
    Some(format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    ))
}

/// A file of instructions by model, read again when it changes.
fn prompts_in(path: &Path, parse: fn(&[u8]) -> HashMap<String, String>) -> HashMap<String, String> {
    type Memo = HashMap<PathBuf, (Option<SystemTime>, HashMap<String, String>)>;
    static MEMO: LazyLock<Mutex<Memo>> = LazyLock::new(Default::default);
    let stamp = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let mut memo = MEMO.lock().unwrap();
    if let Some((at, prompts)) = memo.get(path)
        && *at == stamp
    {
        return prompts.clone();
    }
    let prompts = std::fs::read(path).map(|b| parse(&b)).unwrap_or_default();
    memo.insert(path.to_owned(), (stamp, prompts.clone()));
    prompts
}

/// The instructions each model is sent with, out of a `/models` answer
/// (or Codex's `models_cache.json`, which is one).
fn prompts_of(bytes: &[u8]) -> HashMap<String, String> {
    let Ok(list) = serde_json::from_slice::<Value>(bytes) else {
        return HashMap::new();
    };
    list["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let slug = m["slug"].as_str()?;
            let text = m["model_messages"]["instructions_template"]
                .as_str()
                .filter(|s| !s.is_empty())
                .or_else(|| m["base_instructions"].as_str().filter(|s| !s.is_empty()))?;
            Some((slug.to_owned(), text.to_owned()))
        })
        .collect()
}

fn prompts_path() -> PathBuf {
    crate::paths::home()
        .join("cache")
        .join("codex-prompts.json")
}

/// What Codex CLI would send as a model's instructions: the account's model
/// list's, Codex's own copy of it, else Codex's prompt.
fn instructions(model: &str) -> String {
    let saved = prompts_in(&prompts_path(), |b| {
        serde_json::from_slice(b).unwrap_or_default()
    });
    if let Some(s) = saved.get(model) {
        return s.clone();
    }
    if let Some(s) = prompts_in(&home().join("models_cache.json"), prompts_of).get(model) {
        return s.clone();
    }
    PROMPT.to_owned()
}

// --- Models -----------------------------------------------------------------------

/// The models of a `/models` answer (or of `models_cache.json`), in the
/// backend's order, those it hides left out.
pub(super) fn parse_models(bytes: &[u8]) -> Vec<Model> {
    let Ok(list) = serde_json::from_slice::<Value>(bytes) else {
        return Vec::new();
    };
    let mut models: Vec<&Value> = list["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["slug"].as_str().is_some_and(|s| !s.is_empty()) && m["visibility"] != "hide")
        .collect();
    models.sort_by_key(|m| m["priority"].as_i64().unwrap_or(i64::MAX));
    models
        .into_iter()
        .map(|m| Model {
            id: m["slug"].as_str().unwrap_or_default().to_owned(),
            name: m["display_name"].as_str().unwrap_or_default().to_owned(),
            context: m["context_window"].as_u64().filter(|c| *c > 0),
            reasoning: m["supported_reasoning_levels"]
                .as_array()
                .is_some_and(|l| !l.is_empty()),
            vision: m["input_modalities"]
                .as_array()
                .is_some_and(|i| i.iter().any(|x| x == "image")),
            ..Model::default()
        })
        .collect()
}

/// The models Codex last listed for its account, before pansy asks.
pub fn known_models() -> Vec<Model> {
    std::fs::read(home().join("models_cache.json"))
        .map(|b| parse_models(&b))
        .unwrap_or_default()
}

/// Asks the backend which models the account's plan has, and keeps their
/// instructions for its requests.
pub(crate) async fn models(held: &Held, http: &reqwest::Client) -> Result<Vec<Model>> {
    let signed = sign(held, http, b"", None).await?;
    let version = version(None);
    let mut request = http
        .get(format!("{BASE}/models"))
        .query(&[("client_version", version.as_str())]);
    for (k, v) in &signed.headers {
        if k != "accept" {
            request = request.header(k, v);
        }
    }
    let response = request.send().await?;
    let status = response.status();
    let bytes = response.bytes().await?;
    if !status.is_success() {
        if status.as_u16() == 401 {
            stale(&signed.token);
        }
        bail!(
            "ChatGPT's models: {}",
            said(status, &String::from_utf8_lossy(&bytes))
        );
    }
    let fresh = prompts_of(&bytes);
    if !fresh.is_empty() {
        let mut all: HashMap<String, String> = std::fs::read(prompts_path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        all.extend(fresh);
        crate::fsutil::write_json(&prompts_path(), &all, false)?;
    }
    let models = parse_models(&bytes);
    if models.is_empty() {
        bail!("ChatGPT listed no Codex models");
    }
    Ok(models)
}

// --- Allowance ----------------------------------------------------------------------

/// What `/wham/usage` says of the account: its windows, its credits, and
/// whether it is held (the backend no longer allows it, with no credits
/// to go on with), as Codex reads it for its `/status`.
pub(crate) async fn usage(held: &Held, base: &str, http: &reqwest::Client) -> Result<Reading> {
    let (token, account) = token(held, http).await?;
    let root = base.trim_end_matches('/').trim_end_matches("/codex");
    let mut request = client_headers(http.get(format!("{root}/wham/usage")), &version(None))
        .bearer_auth(&token)
        .header("accept", "application/json");
    if !account.is_empty() {
        request = request.header("chatgpt-account-id", account);
    }
    let response = request.send().await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        if status.as_u16() == 401 {
            stale(&token);
        }
        bail!("ChatGPT's usage: {}", said(status, &text));
    }
    Ok(parse_usage(
        &serde_json::from_str(&text).unwrap_or_default(),
    ))
}

pub(super) fn parse_usage(v: &Value) -> Reading {
    let mut windows = Vec::new();
    for key in ["primary_window", "secondary_window"] {
        let w = &v["rate_limit"][key];
        if !w.is_object() {
            continue;
        }
        let span = w["limit_window_seconds"].as_u64().unwrap_or(0);
        let resets_at = w["reset_at"]
            .as_i64()
            .filter(|t| *t > 0)
            .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
            .or_else(|| {
                w["reset_after_seconds"]
                    .as_i64()
                    .filter(|s| *s > 0)
                    .map(|s| chrono::Utc::now() + chrono::Duration::seconds(s))
            });
        windows.push(Window {
            name: allowance::span_name(span),
            used: w["used_percent"].as_f64().unwrap_or(0.0).clamp(0.0, 100.0),
            resets_at,
            span_secs: span,
            ..Window::default()
        });
    }
    let credits = &v["credits"];
    let on_credits = credits["has_credits"] == true || credits["unlimited"] == true;
    let note = if credits["unlimited"] == true {
        "Unlimited credits".to_owned()
    } else {
        credits["balance"]
            .as_str()
            .and_then(|b| b.trim().parse::<f64>().ok())
            .filter(|n| *n > 0.0 && credits["has_credits"] == true)
            .map(|n| format!("{} credits", allowance::compact(n)))
            .unwrap_or_default()
    };
    // Held: the backend allows it no more, and it has no credits to go on.
    let spent_until = (v["rate_limit"]["allowed"] == false && !on_credits)
        .then(|| {
            let full = windows.iter().filter(|w| w.used >= 100.0);
            full.filter_map(|w| w.resets_at)
                .max()
                .or_else(|| windows.iter().filter_map(|w| w.resets_at).max())
        })
        .flatten();
    Reading {
        plan: v["plan_type"].as_str().unwrap_or_default().to_owned(),
        windows,
        spent_until,
        note,
    }
}

// --- Signing in ---------------------------------------------------------------------

/// Where ChatGPT sends the browser back: Codex's port, the only one its
/// OAuth client may use.
pub(crate) const CALLBACK_PORT: u16 = 1455;

/// The page that signs in to ChatGPT, as `codex login` opens it.
pub(crate) fn authorize_url(challenge: &str, state: &str, redirect: &str) -> String {
    let params = [
        ("response_type", "code"),
        ("client_id", CLIENT_ID),
        ("redirect_uri", redirect),
        (
            "scope",
            "openid profile email offline_access api.connectors.read api.connectors.invoke",
        ),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("id_token_add_organizations", "true"),
        ("codex_cli_simplified_flow", "true"),
        ("state", state),
        ("originator", "codex_cli_rs"),
    ];
    reqwest::Url::parse_with_params(&format!("{ISSUER}/oauth/authorize"), params)
        .map(String::from)
        .unwrap_or_default()
}

/// Trades the code the sign-in page gave for the account's tokens: a sign-in
/// shaped as Codex keeps one, with who it is and the plan.
pub(crate) async fn exchange(
    http: &reqwest::Client,
    code: &str,
    verifier: &str,
    redirect: &str,
) -> Result<(Value, String, String)> {
    let form = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect),
        ("client_id", CLIENT_ID),
        ("code_verifier", verifier),
    ];
    let response = client_headers(http.post(format!("{ISSUER}/oauth/token")), &version(None))
        .form(&form)
        .send()
        .await
        .map_err(|e| anyhow!("couldn't reach ChatGPT: {e}"))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    let tokens: Value = serde_json::from_str(&text).unwrap_or_default();
    let get = |k: &str| tokens[k].as_str().unwrap_or_default().to_owned();
    if !status.is_success() || get("access_token").is_empty() || get("id_token").is_empty() {
        bail!("ChatGPT sent back no token ({})", said(status, &text));
    }
    let auth = json!({
        "OPENAI_API_KEY": null,
        "auth_mode": "chatgpt",
        "tokens": {
            "id_token": get("id_token"),
            "access_token": get("access_token"),
            "refresh_token": get("refresh_token"),
            "account_id": "",
        },
        "last_refresh": chrono::Utc::now().to_rfc3339(),
    });
    let mut auth = auth;
    auth["tokens"]["account_id"] = json!(account_id(&auth));
    let (user, plan) = who(&auth);
    if user == "ChatGPT" {
        bail!("ChatGPT didn't say which account signed in");
    }
    Ok((auth, user, plan))
}

/// A code to type on ChatGPT's device page.
pub(crate) struct DeviceCode {
    pub id: String,
    pub code: String,
    pub interval: u64,
}

/// The page a device code is typed on.
pub(crate) fn device_page() -> String {
    format!("{ISSUER}/codex/device")
}

/// Asks ChatGPT for a device code, as `codex login --device-auth` does.
pub(crate) async fn device_code(http: &reqwest::Client) -> Result<DeviceCode> {
    let response = client_headers(
        http.post(format!("{ISSUER}/api/accounts/deviceauth/usercode")),
        &version(None),
    )
    .json(&json!({"client_id": CLIENT_ID}))
    .send()
    .await
    .map_err(|e| anyhow!("couldn't reach ChatGPT for a sign-in code: {e}"))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if status.as_u16() == 404 {
        bail!("ChatGPT doesn't give a sign-in code for Codex now; sign in in the browser");
    }
    if !status.is_success() {
        bail!("ChatGPT gave no sign-in code ({})", said(status, &text));
    }
    let v: Value = serde_json::from_str(&text).unwrap_or_default();
    let code = v["user_code"]
        .as_str()
        .or(v["usercode"].as_str())
        .unwrap_or_default();
    let id = v["device_auth_id"].as_str().unwrap_or_default();
    if id.is_empty() || code.is_empty() {
        bail!("ChatGPT gave no sign-in code");
    }
    let interval = match &v["interval"] {
        Value::String(s) => s.trim().parse().unwrap_or(5),
        other => other.as_u64().unwrap_or(5),
    };
    Ok(DeviceCode {
        id: id.to_owned(),
        code: code.to_owned(),
        interval: interval.max(1),
    })
}

/// Whether the code was typed: the sign-in when it was, `None` while not yet.
pub(crate) async fn device_poll(
    http: &reqwest::Client,
    device: &DeviceCode,
) -> Result<Option<(Value, String, String)>> {
    let response = client_headers(
        http.post(format!("{ISSUER}/api/accounts/deviceauth/token")),
        &version(None),
    )
    .json(&json!({"device_auth_id": device.id, "user_code": device.code}))
    .send()
    .await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if matches!(status.as_u16(), 403 | 404) {
        return Ok(None);
    }
    if !status.is_success() {
        bail!("ChatGPT refused the sign-in ({})", said(status, &text));
    }
    let v: Value = serde_json::from_str(&text).unwrap_or_default();
    let (Some(code), Some(verifier)) = (
        v["authorization_code"].as_str(),
        v["code_verifier"].as_str(),
    ) else {
        bail!("ChatGPT finished the sign-in but sent no code");
    };
    let redirect = format!("{ISSUER}/deviceauth/callback");
    exchange(http, code, verifier, &redirect).await.map(Some)
}
