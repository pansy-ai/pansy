//! Gemini Code Assist, as Gemini CLI uses it: a Google account signed in
//! with Gemini CLI's own OAuth client, and Code Assist's API
//! (`cloudcode-pa.googleapis.com`), which takes a Gemini request in an
//! envelope (`{model, project, user_prompt_id, request}`) and streams its
//! answer the same way (`{response}`), as magpie does it.
//!
//! Gemini CLI keeps its sign-in in `~/.gemini/oauth_creds.json`, which pansy
//! reads and never writes: Google doesn't turn a refresh token over, so an
//! access token refreshed here is kept in memory alone. The Google Cloud
//! project requests go to is the one Code Assist gives the account (set up
//! the way Gemini CLI sets it up, the first time), or one the user names:
//! Code Assist Standard and Enterprise ask for one.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

use super::allowance::{Reading, Window};
use super::{Held, Kind, SignIn, Signed, Source, said};
use crate::provider::Model;

/// Code Assist's API.
pub const BASE: &str = "https://cloudcode-pa.googleapis.com";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const AUTHORIZE_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v2/userinfo?alt=json";
/// Gemini CLI's own OAuth client, an installed app's: its "secret" ships in
/// the CLI and is no secret.
const CLIENT_ID: &str = "681255809395-oo8ft2oprdrnp9e3aqf6av3hmdib135j.apps.googleusercontent.com";
const CLIENT_SECRET: &str = "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";
const SCOPES: &str = "https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/userinfo.profile";
/// The Gemini CLI version requests say they come from.
const VERSION: &str = "0.61.0";
/// Where the browser comes back to, on a port of this computer's.
pub(crate) const CALLBACK: &str = "/oauth2callback";

/// The models Gemini CLI offers, until Code Assist says which this account has.
const KNOWN: [(&str, &str); 5] = [
    ("gemini-2.5-pro", "Gemini 2.5 Pro"),
    ("gemini-2.5-flash", "Gemini 2.5 Flash"),
    ("gemini-2.5-flash-lite", "Gemini 2.5 Flash Lite"),
    ("gemini-3-pro-preview", "Gemini 3 Pro Preview"),
    ("gemini-3-flash-preview", "Gemini 3 Flash Preview"),
];

/// Where Gemini CLI keeps its sign-in.
fn dir() -> PathBuf {
    crate::paths::user_home().join(".gemini")
}

/// The files Gemini CLI's sign-in is found in.
pub(crate) fn files() -> Vec<PathBuf> {
    let dir = dir();
    vec![
        dir.join("oauth_creds.json"),
        dir.join("google_accounts.json"),
        dir.join(".env"),
    ]
}

fn read(path: PathBuf) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// `KEY=value` of a dotenv file.
fn env_value(path: PathBuf, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines()
        .find_map(|line| {
            let line = line.trim().strip_prefix("export ").unwrap_or(line.trim());
            let (k, v) = line.split_once('=')?;
            (k.trim() == key).then(|| v.trim().trim_matches(|c| c == '"' || c == '\'').to_owned())
        })
        .filter(|v| !v.is_empty())
}

/// Gemini CLI's own sign-in with Google, and its tokens with the project
/// Gemini CLI is told to use. One minted for another app's client is left
/// alone: Google refreshes a token only for the client that minted it.
pub(crate) fn own() -> Option<(SignIn, Value)> {
    let mut auth = read(dir().join("oauth_creds.json"))?;
    if auth["refresh_token"].as_str().is_none_or(str::is_empty) {
        return None;
    }
    let claims = super::jwt_claims(auth["id_token"].as_str().unwrap_or_default());
    let client = claims["azp"]
        .as_str()
        .or(claims["aud"].as_str())
        .unwrap_or_default();
    if !client.is_empty() && client != CLIENT_ID {
        return None;
    }
    let user = read(dir().join("google_accounts.json"))
        .and_then(|a| a["active"].as_str().map(str::to_owned))
        .filter(|u| !u.is_empty())
        .or_else(|| claims["email"].as_str().map(str::to_owned))
        .unwrap_or_else(|| "Google".to_owned());
    let project = env_value(dir().join(".env"), "GOOGLE_CLOUD_PROJECT").or_else(|| {
        super::load_logins()
            .ok()?
            .projects
            .get(&super::plan_key(Kind::GeminiCli, &user))
            .cloned()
    });
    if let Some(project) = project {
        auth["project"] = json!(project);
    }
    let sign_in = SignIn {
        kind: Kind::GeminiCli,
        user,
        plan: String::new(),
        source: Source::Agent,
        project: String::new(),
    };
    Some((sign_in, auth))
}

/// Names the Google Cloud project an account's requests go to (empty: the
/// one Code Assist gives it).
pub fn set_project(account: &str, project: &str) -> Result<()> {
    let project = project.trim();
    match Held::of(account) {
        Held::Agent => {
            let (sign_in, _) =
                own().ok_or_else(|| anyhow!("Gemini CLI is signed out; run gemini and sign in"))?;
            super::update_logins(|logins| {
                let key = super::plan_key(Kind::GeminiCli, &sign_in.user);
                if project.is_empty() {
                    logins.projects.remove(&key);
                } else {
                    logins.projects.insert(key, project.to_owned());
                }
                Ok(())
            })?;
        }
        Held::Pansy(id) => super::update_logins(|logins| {
            let l = logins
                .logins
                .iter_mut()
                .find(|l| l.kind == Kind::GeminiCli && l.id == id)
                .ok_or_else(|| anyhow!("Gemini CLI has no account {id}"))?;
            if project.is_empty() {
                if let Some(auth) = l.auth.as_object_mut() {
                    auth.remove("project");
                }
            } else {
                l.auth["project"] = json!(project);
            }
            Ok(())
        })?,
    }
    PROJECTS.lock().unwrap().clear();
    Ok(())
}

// --- Tokens and the account's project ----------------------------------------------

/// Access tokens refreshed here, by the refresh token: Gemini CLI's file is
/// not written.
static TOKENS: LazyLock<Mutex<HashMap<String, (String, i64)>>> = LazyLock::new(Default::default);
/// The project and plan Code Assist set each account up with, by refresh
/// token and the project named.
static PROJECTS: LazyLock<Mutex<HashMap<String, (String, String)>>> =
    LazyLock::new(Default::default);

/// The access token `token` names was refused: refreshed on the next request.
pub(crate) fn forget(token: &str) {
    TOKENS.lock().unwrap().retain(|_, (t, _)| t != token);
}

/// A live access token for the account's tokens.
async fn token(auth: &Value, http: &reqwest::Client) -> Result<String> {
    let refresh = auth["refresh_token"].as_str().unwrap_or_default();
    if refresh.is_empty() {
        bail!("Gemini CLI's sign-in has no refresh token; sign in again");
    }
    let now = chrono::Utc::now().timestamp_millis();
    let kept = TOKENS.lock().unwrap().get(refresh).cloned();
    let (access, expiry) = kept.unwrap_or_else(|| {
        (
            auth["access_token"].as_str().unwrap_or_default().to_owned(),
            auth["expiry_date"].as_i64().unwrap_or(0),
        )
    });
    if !access.is_empty() && expiry - now > 2 * 60 * 1000 {
        return Ok(access);
    }
    let form = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh),
        ("client_id", CLIENT_ID),
        ("client_secret", CLIENT_SECRET),
    ];
    let response = http
        .post(TOKEN_URL)
        .form(&form)
        .send()
        .await
        .map_err(|e| anyhow!("refreshing the Google sign-in: {e}"))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    let fresh: Value = serde_json::from_str(&text).unwrap_or_default();
    let Some(access) = fresh["access_token"].as_str().filter(|t| !t.is_empty()) else {
        bail!(
            "Google signed this account out ({}); sign in to it again",
            said(status, &text)
        );
    };
    let expiry = now + fresh["expires_in"].as_i64().unwrap_or(3600) * 1000;
    TOKENS
        .lock()
        .unwrap()
        .insert(refresh.to_owned(), (access.to_owned(), expiry));
    Ok(access.to_owned())
}

/// What Gemini CLI says it is to Code Assist.
fn user_agent(model: &str) -> String {
    let platform = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    };
    let model = if model.is_empty() {
        "gemini-2.5-pro"
    } else {
        model
    };
    format!("GeminiCLI/{VERSION}/{model} ({platform}; {arch}; terminal)")
}

/// Posts one of Code Assist's methods.
async fn call(
    http: &reqwest::Client,
    base: &str,
    access: &str,
    method: &str,
    body: &Value,
) -> Result<Value, (u16, String)> {
    let response = http
        .post(format!(
            "{}/v1internal:{method}",
            base.trim_end_matches('/')
        ))
        .bearer_auth(access)
        .header("user-agent", user_agent(""))
        .json(body)
        .send()
        .await
        .map_err(|e| (0, format!("couldn't reach Code Assist: {e}")))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err((
            status.as_u16(),
            format!("Code Assist: {}", said(status, &text)),
        ));
    }
    Ok(serde_json::from_str(&text).unwrap_or_default())
}

/// Code Assist's project, which comes as an id or as `{id, name}`.
fn project_id(v: &Value) -> String {
    v.as_str()
        .or(v["id"].as_str())
        .unwrap_or_default()
        .to_owned()
}

fn tier_name(t: &Value) -> String {
    t["name"]
        .as_str()
        .or(t["id"].as_str())
        .unwrap_or_default()
        .to_owned()
}

/// What a Code Assist Standard or Enterprise account without a project is told.
fn needs_project(why: &str) -> anyhow::Error {
    let say = "Google serves this account's Gemini Code Assist on a Google Cloud project: name it in the account's Project (pansy accounts project gemini-cli <account> <project-id>), or GOOGLE_CLOUD_PROJECT in ~/.gemini/.env";
    if why.is_empty() {
        anyhow!(say)
    } else {
        anyhow!("{why} — {say}")
    }
}

/// The project the account's requests go to and its plan, set up as Gemini
/// CLI sets it up (its `setupUser`) the first time.
async fn project(
    auth: &Value,
    access: &str,
    base: &str,
    http: &reqwest::Client,
) -> Result<(String, String)> {
    let want = auth["project"].as_str().unwrap_or_default().to_owned();
    let key = format!(
        "{}\0{want}",
        auth["refresh_token"].as_str().unwrap_or_default()
    );
    if let Some(kept) = PROJECTS.lock().unwrap().get(&key) {
        return Ok(kept.clone());
    }
    let mut meta = json!({"ideType": "IDE_UNSPECIFIED", "platform": "PLATFORM_UNSPECIFIED", "pluginType": "GEMINI"});
    let mut body = json!({"metadata": meta});
    if !want.is_empty() {
        meta["duetProject"] = json!(want);
        body = json!({"metadata": meta, "cloudaicompanionProject": want});
    }
    let load = call(http, base, access, "loadCodeAssist", &body)
        .await
        .map_err(|(_, e)| anyhow!(e))?;
    let plan_of = |l: &Value| {
        let paid = tier_name(&l["paidTier"]);
        if paid.is_empty() {
            tier_name(&l["currentTier"])
        } else {
            paid
        }
    };
    let found = if load["currentTier"].is_object() {
        let id = project_id(&load["cloudaicompanionProject"]);
        let id = if id.is_empty() { want.clone() } else { id };
        if id.is_empty() {
            return Err(needs_project(""));
        }
        (id, plan_of(&load))
    } else {
        // Not set up yet: on the tier Code Assist offers it first.
        let tiers = load["allowedTiers"].as_array().cloned().unwrap_or_default();
        let tier = tiers
            .iter()
            .find(|t| t["isDefault"] == true)
            .or(tiers.first())
            .cloned()
            .unwrap_or_else(
                || json!({"id": "legacy-tier", "userDefinedCloudaicompanionProject": true}),
            );
        if tier["userDefinedCloudaicompanionProject"] == true && want.is_empty() {
            let why = load["ineligibleTiers"]
                .as_array()
                .into_iter()
                .flatten()
                .find_map(|t| t["reasonMessage"].as_str())
                .unwrap_or_default();
            return Err(needs_project(why));
        }
        let mut onboard = json!({"tierId": tier["id"], "metadata": meta});
        if tier["id"] != "free-tier" {
            onboard["cloudaicompanionProject"] = json!(want);
        }
        let mut id = String::new();
        for _ in 0..5 {
            let lro = call(http, base, access, "onboardUser", &onboard)
                .await
                .map_err(|(_, e)| anyhow!(e))?;
            if lro["done"] == true {
                id = project_id(&lro["response"]["cloudaicompanionProject"]);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
        let id = if id.is_empty() { want.clone() } else { id };
        if id.is_empty() {
            return Err(needs_project(""));
        }
        (id, tier_name(&tier))
    };
    PROJECTS.lock().unwrap().insert(key, found.clone());
    Ok(found)
}

/// Signs a request to Code Assist: the account's token, and the envelope
/// with its project and the ids Gemini CLI sends along.
pub(crate) async fn sign(
    held: &Held,
    base: &str,
    http: &reqwest::Client,
    body: &[u8],
) -> Result<Signed> {
    let auth = held.read(Kind::GeminiCli)?;
    let access = token(&auth, http).await?;
    let (project, _) = project(&auth, &access, base, http).await?;
    let mut envelope: Value = serde_json::from_slice(body).context("the request is not JSON")?;
    let model = envelope["model"].as_str().unwrap_or_default().to_owned();
    envelope["project"] = json!(project);
    envelope["user_prompt_id"] = json!(&uuid::Uuid::new_v4().simple().to_string()[..13]);
    Ok(Signed {
        headers: vec![
            ("authorization".to_owned(), format!("Bearer {access}")),
            ("user-agent".to_owned(), user_agent(&model)),
        ],
        base: None,
        token: access,
        body: Some(serde_json::to_vec(&envelope)?),
    })
}

/// A Gemini request in Code Assist's envelope, the project left for the
/// account to add.
pub fn envelope(model: &str, body: &mut Value) {
    let request = std::mem::take(body);
    *body = json!({"model": model, "request": request});
}

/// The models of the account's quota buckets: the ids Code Assist serves it,
/// those of Vertex AI left out; and its plan.
pub(crate) async fn models(
    held: &Held,
    base: &str,
    http: &reqwest::Client,
) -> Result<(Vec<Model>, String)> {
    let auth = held.read(Kind::GeminiCli)?;
    let access = token(&auth, http).await?;
    let (project, plan) = project(&auth, &access, base, http).await?;
    let quota = call(
        http,
        base,
        &access,
        "retrieveUserQuota",
        &json!({"project": project}),
    )
    .await
    .map_err(|(_, e)| anyhow!(e))?;
    let mut ids: Vec<String> = quota["buckets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|b| b["modelId"].as_str())
        .filter(|id| !id.is_empty() && !id.ends_with("_vertex"))
        .map(str::to_owned)
        .collect();
    ids.sort();
    ids.dedup();
    let models = if ids.is_empty() {
        known_models()
    } else {
        ids.iter().map(|id| model(id)).collect()
    };
    Ok((models, plan))
}

fn model(id: &str) -> Model {
    Model {
        name: KNOWN
            .iter()
            .find(|(k, _)| *k == id)
            .map(|(_, n)| (*n).to_owned())
            .unwrap_or_default(),
        vision: true,
        reasoning: id.contains("-pro") || id.contains("-flash"),
        ..Model::new(id)
    }
}

/// The models Gemini CLI offers, before the account's are asked for.
pub fn known_models() -> Vec<Model> {
    KNOWN.iter().map(|(id, _)| model(id)).collect()
}

// --- Allowance ----------------------------------------------------------------------

/// Each model's daily allowance: Code Assist's quota buckets, those of
/// Vertex AI left out.
pub(crate) async fn usage(held: &Held, base: &str, http: &reqwest::Client) -> Result<Reading> {
    let auth = held.read(Kind::GeminiCli)?;
    let access = token(&auth, http).await?;
    let (project, plan) = project(&auth, &access, base, http).await?;
    let quota = call(
        http,
        base,
        &access,
        "retrieveUserQuota",
        &json!({"project": project}),
    )
    .await
    .map_err(|(_, e)| anyhow!(e))?;
    Ok(Reading {
        plan,
        ..parse_usage(&quota)
    })
}

pub(crate) fn parse_usage(quota: &Value) -> Reading {
    let mut windows: Vec<Window> = Vec::new();
    for b in quota["buckets"].as_array().into_iter().flatten() {
        let id = b["modelId"].as_str().unwrap_or_default();
        let Some(left) = b["remainingFraction"].as_f64() else {
            continue;
        };
        if id.is_empty() || id.ends_with("_vertex") || windows.iter().any(|w| w.name == id) {
            continue;
        }
        windows.push(Window {
            name: id.to_owned(),
            used: (100.0 * (1.0 - left)).clamp(0.0, 100.0),
            resets_at: b["resetTime"]
                .as_str()
                .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                .map(|t| t.with_timezone(&chrono::Utc)),
            span_secs: 24 * 3600,
            ..Window::default()
        });
    }
    windows.sort_by(|a, b| a.name.cmp(&b.name));
    Reading {
        windows,
        ..Reading::default()
    }
}

// --- Signing in ---------------------------------------------------------------------

/// Google's sign-in page for Gemini CLI's client, back at `redirect`.
pub(crate) fn authorize_url(challenge: &str, state: &str, redirect: &str) -> String {
    let params = [
        ("client_id", CLIENT_ID),
        ("redirect_uri", redirect),
        ("response_type", "code"),
        ("scope", SCOPES),
        ("access_type", "offline"),
        ("prompt", "consent"),
        ("state", state),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
    ];
    reqwest::Url::parse_with_params(AUTHORIZE_URL, params)
        .map(String::from)
        .unwrap_or_default()
}

/// Trades the page's code for the account's tokens, finds whose account it
/// is, and sets it up for Code Assist when it can be without a project.
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
        ("client_secret", CLIENT_SECRET),
        ("code_verifier", verifier),
    ];
    let response = http
        .post(TOKEN_URL)
        .form(&form)
        .send()
        .await
        .map_err(|e| anyhow!("couldn't reach Google: {e}"))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    let tokens: Value = serde_json::from_str(&text).unwrap_or_default();
    let access = tokens["access_token"].as_str().unwrap_or_default();
    if !status.is_success() || access.is_empty() || tokens["refresh_token"].as_str().is_none() {
        bail!("Google sent back no token ({})", said(status, &text));
    }
    let who: Value = http
        .get(USERINFO_URL)
        .bearer_auth(access)
        .send()
        .await?
        .json()
        .await
        .unwrap_or_default();
    let user = who["email"]
        .as_str()
        .filter(|e| !e.is_empty())
        .ok_or_else(|| anyhow!("Google didn't say whose account this is"))?
        .to_owned();
    let mut auth = json!({
        "access_token": access,
        "refresh_token": tokens["refresh_token"],
        "expiry_date": chrono::Utc::now().timestamp_millis() + tokens["expires_in"].as_i64().unwrap_or(3600) * 1000,
    });
    // Kept even when Code Assist wants a project first: one is named after.
    let plan = match project(&auth, access, BASE, http).await {
        Ok((id, plan)) => {
            auth["project"] = json!(id);
            plan
        }
        Err(e) => {
            tracing::info!("{user}'s Code Assist isn't set up yet: {e:#}");
            String::new()
        }
    };
    Ok((auth, user, plan))
}
