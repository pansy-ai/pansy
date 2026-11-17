//! Subscriptions: ChatGPT's (as Codex signs in to it), GitHub Copilot's and
//! Gemini Code Assist's (as Gemini CLI signs in to it), served as providers.
//! As magpie does it, pansy reads the sign-in an agent keeps where the agent
//! keeps it (Codex's `auth.json`, Copilot's `apps.json`, Gemini CLI's
//! `oauth_creds.json`) and keeps nothing of it; the accounts signed in from
//! pansy itself are kept in `logins.json`, readable by the user alone.
//!
//! Each subscription is one provider (`codex`, `copilot`, `gemini-cli`) whose accounts
//! are its sign-ins, tried in order like keys. Its sign-ins are found again
//! each time pansy loads its providers: sign out of the agent and that
//! account is gone, the provider too when it was the last.
//!
//! A refreshed ChatGPT sign-in goes back where it was found, every other
//! field as it was: OpenAI turns the refresh token over on each refresh,
//! and Codex would be signed out holding the old one.

pub mod allowance;
pub mod codex;
pub mod copilot;
pub mod flow;
pub mod gemini;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::fsutil::{read_json, write_json};
use crate::paths;
use crate::provider::{Account, Endpoint, Model, Protocol, Provider, Quirks};

/// A subscription pansy serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// ChatGPT's, as Codex uses it: OpenAI's models on the Codex backend.
    Codex,
    /// GitHub Copilot's: OpenAI's, Anthropic's and Google's models.
    Copilot,
    /// Gemini Code Assist's, as Gemini CLI uses it: Google's models on
    /// Code Assist.
    #[serde(rename = "gemini-cli")]
    GeminiCli,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Codex, Kind::Copilot, Kind::GeminiCli];

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Codex => "codex",
            Kind::Copilot => "copilot",
            Kind::GeminiCli => "gemini-cli",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        match s.trim().to_ascii_lowercase().as_str() {
            "codex" | "chatgpt" => Some(Kind::Codex),
            "copilot" | "github" | "github-copilot" => Some(Kind::Copilot),
            "gemini-cli" | "gemini" | "code-assist" | "google" => Some(Kind::GeminiCli),
            _ => None,
        }
    }

    /// Its provider's id, and so what agents pick its models by (`codex/…`).
    pub fn id(self) -> &'static str {
        self.as_str()
    }

    /// Its provider's name.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Codex => "Codex",
            Kind::Copilot => "Copilot",
            Kind::GeminiCli => "Gemini CLI",
        }
    }

    /// What an account of it signs in to.
    pub fn vendor(self) -> &'static str {
        match self {
            Kind::Codex => "ChatGPT",
            Kind::Copilot => "GitHub",
            Kind::GeminiCli => "Google",
        }
    }

    /// The agent whose own sign-in pansy reads.
    pub fn agent(self) -> &'static str {
        match self {
            Kind::Codex => "Codex",
            Kind::Copilot => "Copilot",
            Kind::GeminiCli => "Gemini CLI",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Kind::Codex => "codex-color",
            Kind::Copilot => "githubcopilot",
            Kind::GeminiCli => "geminicli-color",
        }
    }

    pub fn website(self) -> &'static str {
        match self {
            Kind::Codex => "https://chatgpt.com/codex",
            Kind::Copilot => "https://github.com/features/copilot",
            Kind::GeminiCli => "https://codeassist.google",
        }
    }

    pub fn endpoints(self) -> Vec<Endpoint> {
        let at = |protocol, base: &str| Endpoint {
            protocol,
            base_url: base.to_owned(),
        };
        match self {
            Kind::Codex => vec![at(Protocol::Responses, codex::BASE)],
            Kind::Copilot => vec![
                at(Protocol::Chat, copilot::BASE),
                at(Protocol::Responses, copilot::BASE),
                at(Protocol::Anthropic, copilot::BASE),
            ],
            Kind::GeminiCli => vec![at(Protocol::Gemini, gemini::BASE)],
        }
    }

    /// Its backend answers in a stream alone (or pansy asks it for one): a
    /// request for a whole answer is streamed and gathered.
    pub fn streams_only(self) -> bool {
        matches!(self, Kind::Codex | Kind::GeminiCli)
    }

    /// Whether pansy can sign in to it by a code typed on the vendor's page.
    pub fn signs_in_by_code(self) -> bool {
        matches!(self, Kind::Codex | Kind::Copilot)
    }

    /// Whether pansy can sign in to it in the browser, back on this computer.
    pub fn signs_in_by_browser(self) -> bool {
        matches!(self, Kind::Codex | Kind::GeminiCli)
    }

    /// Whether a request of its backend's protocol passes through: Code
    /// Assist's answers come wrapped, and are read and written again.
    pub fn passes_through(self) -> bool {
        self != Kind::GeminiCli
    }

    /// Whether its accounts name the Google Cloud project they bill.
    pub fn has_projects(self) -> bool {
        self == Kind::GeminiCli
    }
}

/// Where a sign-in is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// The agent's own (Codex's `auth.json`, Copilot's `apps.json`): pansy
    /// reads it there, and puts a refreshed one back there.
    Agent,
    /// Signed in from pansy, kept in `logins.json`.
    Pansy,
}

/// A subscription's account: who is signed in, on which plan, kept where.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignIn {
    pub kind: Kind,
    /// An email, a GitHub login.
    pub user: String,
    /// The plan, as its vendor names it: `plus`, `pro`, `Pro+`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub plan: String,
    pub source: Source,
    /// The Google Cloud project it bills, when the user named one.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub project: String,
}

/// The account id of the agent's own sign-in.
pub const OWN: &str = "own";

/// A sign-in found where it is kept, with the id its account goes by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub id: String,
    pub sign_in: SignIn,
}

// --- What pansy keeps: logins.json ----------------------------------------------

/// An account signed in from pansy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Login {
    pub id: String,
    pub kind: Kind,
    pub user: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub plan: String,
    /// Its tokens, as the agent would keep them: Codex's `auth.json`, or
    /// `{"user", "oauth_token"}` as Copilot's `apps.json` has them.
    pub auth: Value,
    pub added: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Logins {
    #[serde(default)]
    pub logins: Vec<Login>,
    /// The plans of the agents' own sign-ins, as their vendor last said
    /// (`copilot:octocat` → `Pro`): their files don't say.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub plans: BTreeMap<String, String>,
    /// The Google Cloud projects the user named for the agents' own
    /// sign-ins (`gemini-cli:ada@example.com` → `my-project`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub projects: BTreeMap<String, String>,
}

pub fn load_logins() -> Result<Logins> {
    read_json(&paths::logins())
}

/// Changes `logins.json`: read, change, write (readable by the user alone).
pub fn update_logins<R>(change: impl FnOnce(&mut Logins) -> Result<R>) -> Result<R> {
    crate::config::locked(&paths::logins(), || {
        let mut logins = load_logins()?;
        let out = change(&mut logins)?;
        write_json(&paths::logins(), &logins, true)?;
        Ok(out)
    })
}

/// Keeps an account signed in from pansy: one of the same user is signed
/// in to again (its tokens replaced), another is added. Its account id.
pub fn add_login(kind: Kind, user: &str, plan: &str, auth: Value) -> Result<String> {
    update_logins(|logins| {
        if let Some(l) = logins
            .logins
            .iter_mut()
            .find(|l| l.kind == kind && l.user.eq_ignore_ascii_case(user))
        {
            l.auth = auth;
            if !plan.is_empty() {
                l.plan = plan.to_owned();
            }
            return Ok(l.id.clone());
        }
        let n = (1..)
            .find(|n| !logins.logins.iter().any(|l| l.id == format!("s{n}")))
            .unwrap();
        logins.logins.push(Login {
            id: format!("s{n}"),
            kind,
            user: user.to_owned(),
            plan: plan.to_owned(),
            auth,
            added: chrono::Utc::now(),
        });
        Ok(format!("s{n}"))
    })
}

/// Signs an account signed in from pansy out: its tokens are deleted. The
/// agent's own sign-in is the agent's to sign out of.
pub fn sign_out(kind: Kind, account: &str) -> Result<()> {
    if account == OWN {
        bail!(
            "this is {}'s own sign-in: sign out in {} itself, or switch the account off here",
            kind.agent(),
            kind.agent()
        );
    }
    update_logins(|logins| {
        let before = logins.logins.len();
        logins
            .logins
            .retain(|l| !(l.kind == kind && l.id == account));
        if logins.logins.len() == before {
            bail!("{} has no account {account}", kind.name());
        }
        Ok(())
    })
}

pub(crate) fn plan_key(kind: Kind, user: &str) -> String {
    format!("{}:{}", kind.as_str(), user.to_lowercase())
}

/// Remembers the plan an agent's own sign-in is on, which its file doesn't say.
pub fn note_plan(kind: Kind, user: &str, plan: &str) {
    if plan.is_empty() || user.is_empty() {
        return;
    }
    let key = plan_key(kind, user);
    let known = load_logins().ok().and_then(|l| l.plans.get(&key).cloned());
    if known.as_deref() == Some(plan) {
        return;
    }
    let saved = update_logins(|logins| {
        logins.plans.insert(key, plan.to_owned());
        if let Some(l) = logins
            .logins
            .iter_mut()
            .find(|l| l.kind == kind && l.user.eq_ignore_ascii_case(user))
        {
            l.plan = plan.to_owned();
        }
        Ok(())
    });
    if let Err(e) = saved {
        tracing::warn!("keeping {}'s plan: {e:#}", kind.name());
    }
}

// --- Finding them ---------------------------------------------------------------

/// The files sign-ins are found in, for the gateway to read again when one
/// changes.
pub fn watched() -> Vec<PathBuf> {
    let mut out = vec![paths::logins(), codex::auth_path()];
    out.extend(copilot::files());
    out.extend(gemini::files());
    out
}

/// Every sign-in: each subscription's agent's own first (unless Settings
/// leaves the agents' sign-ins alone), then those signed in from pansy. One
/// of pansy's of the agent's own account is passed over.
pub fn found() -> Vec<Found> {
    let use_agents = crate::config::load_settings()
        .map(|s| s.agent_sign_ins)
        .unwrap_or(true);
    let logins = load_logins().unwrap_or_else(|e| {
        tracing::warn!("reading the subscriptions' sign-ins: {e:#}");
        Logins::default()
    });
    let mut out = Vec::new();
    for kind in Kind::ALL {
        let own = if use_agents {
            match kind {
                Kind::Codex => codex::own(),
                Kind::Copilot => copilot::own().map(|(s, _)| s),
                Kind::GeminiCli => gemini::own().map(|(mut s, auth)| {
                    s.project = auth["project"].as_str().unwrap_or_default().to_owned();
                    s
                }),
            }
        } else {
            None
        };
        if let Some(mut s) = own.clone() {
            if s.plan.is_empty() {
                s.plan = logins
                    .plans
                    .get(&plan_key(kind, &s.user))
                    .cloned()
                    .unwrap_or_default();
            }
            out.push(Found {
                id: OWN.to_owned(),
                sign_in: s,
            });
        }
        for l in logins.logins.iter().filter(|l| l.kind == kind) {
            if own
                .as_ref()
                .is_some_and(|o| o.user.eq_ignore_ascii_case(&l.user))
            {
                continue;
            }
            out.push(Found {
                id: l.id.clone(),
                sign_in: SignIn {
                    kind,
                    user: l.user.clone(),
                    plan: l.plan.clone(),
                    source: Source::Pansy,
                    project: l.auth["project"].as_str().unwrap_or_default().to_owned(),
                },
            });
        }
    }
    out
}

/// A subscription's provider, new: its endpoints and the models known
/// before its list is fetched.
pub fn provider(kind: Kind) -> Provider {
    Provider {
        id: kind.id().to_owned(),
        name: kind.name().to_owned(),
        preset: None,
        endpoints: kind.endpoints(),
        accounts: Vec::new(),
        models: match kind {
            Kind::Codex => codex::known_models(),
            Kind::Copilot => Vec::new(),
            Kind::GeminiCli => gemini::known_models(),
        },
        manual_models: Vec::new(),
        hidden_models: Vec::new(),
        enabled: true,
        headers: BTreeMap::new(),
        proxy: None,
        auth_header: None,
        quirks: Quirks::default(),
        subscription: Some(kind),
        plugin: None,
    }
}

/// Puts the sign-ins `found` in `providers`: each subscription's provider
/// has one account per sign-in, in the order the user left them, the new
/// ones last; an account signed out is gone, and a provider left with none.
/// A provider of the user's own that has a subscription's id keeps it, and
/// that subscription is not served.
pub fn merge(providers: &mut Vec<Provider>, found: &[Found]) {
    for kind in Kind::ALL {
        let mine: Vec<&Found> = found.iter().filter(|f| f.sign_in.kind == kind).collect();
        let at = providers.iter().position(|p| p.subscription == Some(kind));
        if mine.is_empty() {
            if let Some(at) = at {
                providers.remove(at);
            }
            continue;
        }
        let at = match at {
            Some(at) => at,
            None if providers.iter().any(|p| p.id == kind.id()) => continue,
            None => {
                providers.push(provider(kind));
                providers.len() - 1
            }
        };
        let p = &mut providers[at];
        if p.endpoints.is_empty() {
            p.endpoints = kind.endpoints();
        }
        p.accounts.retain(|a| mine.iter().any(|f| f.id == a.id));
        for f in mine {
            match p.accounts.iter_mut().find(|a| a.id == f.id) {
                Some(a) => {
                    a.key.clear();
                    a.sign_in = Some(f.sign_in.clone());
                }
                None => p.accounts.push(Account {
                    id: f.id.clone(),
                    label: String::new(),
                    key: String::new(),
                    enabled: true,
                    proxy: None,
                    sign_in: Some(f.sign_in.clone()),
                    plugin: None,
                }),
            }
        }
    }
}

// --- Tokens ---------------------------------------------------------------------

/// What a request on a sign-in is sent with: its headers (the token among
/// them), and where it goes when the vendor says another address.
#[derive(Debug, Clone, Default)]
pub struct Signed {
    pub headers: Vec<(String, String)>,
    pub base: Option<String>,
    /// The token sent, for [`refused`] to name.
    pub token: String,
    /// The body to send instead, finished with what only the account
    /// knows (Code Assist's project).
    pub body: Option<Vec<u8>>,
}

/// Where an account's tokens are kept.
pub(crate) enum Held {
    /// The agent's own.
    Agent,
    /// pansy's, by its id in `logins.json`.
    Pansy(String),
}

impl Held {
    pub(crate) fn of(account: &str) -> Held {
        if account == OWN {
            Held::Agent
        } else {
            Held::Pansy(account.to_owned())
        }
    }

    /// The tokens as kept.
    pub(crate) fn read(&self, kind: Kind) -> Result<Value> {
        match self {
            Held::Agent => match kind {
                Kind::Codex => codex::read_own(),
                Kind::Copilot => copilot::own()
                    .map(|(_, auth)| auth)
                    .ok_or_else(|| anyhow!("Copilot is signed out; sign in in your editor")),
                Kind::GeminiCli => gemini::own()
                    .map(|(_, auth)| auth)
                    .ok_or_else(|| anyhow!("Gemini CLI is signed out; run gemini and sign in")),
            },
            Held::Pansy(id) => load_logins()?
                .logins
                .into_iter()
                .find(|l| l.kind == kind && &l.id == id)
                .map(|l| l.auth)
                .ok_or_else(|| anyhow!("{} has no account {id}; sign in again", kind.name())),
        }
    }

    /// Puts refreshed tokens back where they were read.
    pub(crate) fn write(&self, kind: Kind, auth: &Value) -> Result<()> {
        match self {
            Held::Agent => match kind {
                Kind::Codex => codex::write_own(auth),
                Kind::Copilot | Kind::GeminiCli => {
                    bail!("{}'s own sign-in is not written", kind.agent())
                }
            },
            Held::Pansy(id) => update_logins(|logins| {
                let l = logins
                    .logins
                    .iter_mut()
                    .find(|l| l.kind == kind && &l.id == id)
                    .ok_or_else(|| anyhow!("{} has no account {id}", kind.name()))?;
                l.auth = auth.clone();
                Ok(())
            }),
        }
    }
}

/// Signs a request on `account` of the subscription to its backend at
/// `base`: its token, refreshed or traded for a session first when it needs
/// it, and the headers its backend wants. `body` is the request's,
/// `client_version` the Codex version a Codex coming through says it is.
pub async fn sign(
    kind: Kind,
    account: &str,
    base: &str,
    http: &reqwest::Client,
    body: &[u8],
    client_version: Option<&str>,
) -> Result<Signed> {
    let held = Held::of(account);
    match kind {
        Kind::Codex => codex::sign(&held, http, body, client_version).await,
        Kind::Copilot => copilot::sign(&held, http, body).await,
        Kind::GeminiCli => gemini::sign(&held, base, http, body).await,
    }
}

/// The backend refused `token` (401): the next request refreshes it, or
/// trades for a new session, rather than send it again.
pub fn refused(kind: Kind, token: &str) {
    match kind {
        Kind::Codex => codex::stale(token),
        Kind::Copilot => copilot::forget(token),
        Kind::GeminiCli => gemini::forget(token),
    }
}

/// Makes a request's body the one the backend takes, for `model`.
pub fn prepare(kind: Kind, protocol: Protocol, model: &str, body: &mut Value) {
    match (kind, protocol) {
        (Kind::Codex, Protocol::Responses) => codex::shape(body),
        (Kind::GeminiCli, Protocol::Gemini) => gemini::envelope(model, body),
        _ => {}
    }
}

/// The address of a request to the subscription's backend at `base`, when
/// it isn't the protocol's own (Code Assist's methods).
pub fn url(kind: Kind, base: &str, stream: bool) -> Option<String> {
    (kind == Kind::GeminiCli).then(|| {
        let method = if stream {
            "streamGenerateContent?alt=sse"
        } else {
            "generateContent"
        };
        format!("{}/v1internal:{method}", base.trim_end_matches('/'))
    })
}

/// Names the Google Cloud project an account bills (empty: the one its
/// vendor gives it).
pub fn set_project(kind: Kind, account: &str, project: &str) -> Result<()> {
    match kind {
        Kind::GeminiCli => gemini::set_project(account, project),
        _ => bail!("{} accounts name no project", kind.name()),
    }
}

/// A client for the subscription's own calls (sign-in, refresh, lists):
/// the account's proxy, else the provider's, else pansy's.
pub(crate) fn client(proxy: Option<&str>) -> Result<reqwest::Client> {
    let proxy = proxy
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .or_else(|| crate::config::load_settings().ok().map(|s| s.proxy))
        .unwrap_or_default();
    let builder = reqwest::Client::builder()
        .user_agent(concat!("pansy/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(60));
    Ok(crate::settings::use_proxy(builder, &proxy)
        .map_err(anyhow::Error::msg)?
        .build()?)
}

/// The models a subscription's provider serves, as its first account's
/// plan has them.
pub async fn models(provider: &Provider) -> Result<Vec<Model>> {
    let kind = provider
        .subscription
        .ok_or_else(|| anyhow!("{} is no subscription", provider.name))?;
    let account = provider
        .enabled_accounts()
        .next()
        .ok_or_else(|| anyhow!("{} has no account switched on", provider.name))?;
    let http = client(account.proxy.as_deref().or(provider.proxy.as_deref()))?;
    let held = Held::of(&account.id);
    match kind {
        Kind::Codex => codex::models(&held, &http).await,
        Kind::Copilot | Kind::GeminiCli => {
            let (models, plan) = match kind {
                Kind::Copilot => copilot::models(&held, &http).await?,
                _ => {
                    let base = provider
                        .endpoint(Protocol::Gemini)
                        .map_or(gemini::BASE, |e| e.base_url.as_str());
                    gemini::models(&held, base, &http).await?
                }
            };
            if let Some(s) = &account.sign_in {
                note_plan(kind, &s.user, &plan);
            }
            Ok(models)
        }
    }
}

// --- Small helpers --------------------------------------------------------------

/// The claims of a JWT, unchecked: the tokens are the user's own, and only
/// their expiry, account and email are read.
pub(crate) fn jwt_claims(token: &str) -> Value {
    use base64::Engine;
    let Some(payload) = token.split('.').nth(1) else {
        return Value::Null;
    };
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null)
}

/// An error answer's message, for a sign-in's or a refresh's error.
pub(crate) fn said(status: reqwest::StatusCode, body: &str) -> String {
    let text = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            ["/error_description", "/error/message", "/message", "/error"]
                .iter()
                .find_map(|p| v.pointer(p).and_then(Value::as_str).map(str::to_owned))
        })
        .unwrap_or_else(|| body.trim().chars().take(200).collect());
    if text.is_empty() {
        status.to_string()
    } else {
        format!("{status}: {text}")
    }
}

#[cfg(test)]
pub(crate) mod tests;
