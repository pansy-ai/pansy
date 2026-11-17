//! Host A: magpie's `host.js` on Bun, which runs the OpenCode provider
//! plugins and pi packages (they sign in, list models and send requests)
//! and the request hooks (`magpie.hooks`).
//!
//! One host per process, started the first time it is wanted and again
//! when `plugins.json` changes or it died. What it lists is kept in
//! `plugin-providers.json` ([`pansy_core::plugin::Listing`]), which the
//! providers are merged from.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, SystemTime};

use anyhow::{Result, anyhow, bail};
use pansy_core::paths;
use pansy_core::plugin::{Listed, Listing, Loaded};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Semaphore;

use crate::rpc::Rpc;
use crate::store::{self, Installed};

/// How long a plugin's provider may take to answer a request's head.
const HEAD_WAIT: Duration = Duration::from_secs(600);
/// How long the host may take to load the plugins.
const INIT_WAIT: Duration = Duration::from_secs(120);

struct Host {
    rpc: Arc<Rpc>,
    home: PathBuf,
    stamp: Option<SystemTime>,
    loaded: Vec<Loaded>,
}

static CURRENT: LazyLock<tokio::sync::Mutex<Option<Arc<Host>>>> =
    LazyLock::new(|| tokio::sync::Mutex::new(None));

/// The base URL each account's loader gave, by `provider\0account`.
static BASES: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(Default::default);

/// Each provider's limit of requests at once, as its plugin says.
static LIMITS: LazyLock<Mutex<HashMap<String, Arc<Semaphore>>>> = LazyLock::new(Default::default);

/// Stops the host this process runs (the tests' scratch homes).
pub fn forget() {
    if let Ok(mut current) = CURRENT.try_lock()
        && let Some(host) = current.take()
    {
        host.rpc.kill();
    }
    BASES.lock().unwrap_or_else(|e| e.into_inner()).clear();
    LIMITS.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// The plugins host A loads: those on that are provider plugins, pi
/// packages or request hooks.
pub fn plugins() -> Result<Vec<Installed>> {
    Ok(store::installed()?
        .into_iter()
        .filter(|i| !i.entry.off && i.kinds.in_host())
        .collect())
}

/// Whether some plugin that is on has request hooks.
pub fn has_hooks() -> bool {
    plugins()
        .unwrap_or_default()
        .iter()
        .any(|i| i.kinds.hooks.is_some())
}

/// The proxy pansy's settings name, as the host's `MAGPIE_*_PROXY`.
fn proxy_env() -> Vec<(String, String)> {
    let proxy = pansy_core::config::load_settings()
        .unwrap_or_default()
        .proxy;
    let proxy = proxy.trim();
    if proxy.is_empty() || proxy == "direct" {
        return Vec::new();
    }
    ["HTTPS_PROXY", "HTTP_PROXY", "ALL_PROXY"]
        .iter()
        .map(|k| (format!("MAGPIE_{k}"), proxy.to_owned()))
        .collect()
}

/// Fetches models.dev's catalog in the background when there is none from
/// the last day: a plugin's provider is filled in from it, as in OpenCode.
fn refresh_models_dev() {
    let path = paths::models_dev();
    let fresh = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|at| SystemTime::now().duration_since(at).ok())
        .is_some_and(|age| age < Duration::from_secs(24 * 3600));
    if fresh {
        return;
    }
    crate::rt().spawn(async move {
        let got = async {
            let text = reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .ok()?
                .get("https://models.dev/api.json")
                .send()
                .await
                .ok()?
                .error_for_status()
                .ok()?
                .bytes()
                .await
                .ok()?;
            serde_json::from_slice::<Value>(&text).ok()?;
            pansy_core::fsutil::write_atomic(&path, &text, false).ok()
        };
        if got.await.is_none() {
            tracing::debug!("models.dev couldn't be read; plugins list their models without it");
        }
    });
}

async fn start(chosen: Vec<Installed>) -> Result<Arc<Host>> {
    let bun = crate::bun::ensure().await?;
    let script = crate::scripts::write("host", crate::scripts::HOST)?;
    let pi = crate::scripts::write("pi", crate::scripts::PI)?;
    let home = paths::home();
    std::fs::create_dir_all(&home)?;
    refresh_models_dev();
    let list = store::List::load()?;
    let items: Vec<Value> = chosen
        .iter()
        .map(|i| {
            let mut item = json!({
                "spec": i.entry.spec,
                "target": i.target,
                "options": i.entry.options_value(),
            });
            if let Some(hooks) = &i.kinds.hooks {
                item["gateway"] = json!(hooks);
            }
            item
        })
        .collect();
    let models_dev = paths::models_dev();
    let rpc = Rpc::start(&bun, &script, &home, &proxy_env(), "plugin host")?;
    let init = rpc
        .call(
            "init",
            json!({
                "authPath": paths::plugin_auth(),
                "modelsDevPath": if models_dev.exists() { json!(models_dev) } else { json!("") },
                "piPath": pi,
                "directory": home,
                "config": list.config,
                "prefer": list.prefer,
                "options": {},
                "plugins": items,
            }),
            INIT_WAIT,
        )
        .await
        .map_err(|e| {
            let said = rpc.said();
            rpc.kill();
            if said.is_empty() {
                anyhow!("starting the plugins: {e:#}")
            } else {
                anyhow!("starting the plugins: {e:#}\n{said}")
            }
        })?;
    let loaded: Vec<Loaded> = serde_json::from_value(init["plugins"].clone()).unwrap_or_default();
    for l in &loaded {
        if let Some(e) = &l.error {
            tracing::warn!("plugin {}: {e}", l.spec);
        }
    }
    let host = Arc::new(Host {
        rpc,
        home,
        stamp: store::stamp(),
        loaded,
    });
    watch(&host);
    Ok(host)
}

/// Follows what the host says unasked: a sign-in saved, renewed or gone
/// lists the providers again, and its loader is asked again.
fn watch(host: &Arc<Host>) {
    let mut events = host.rpc.events();
    let weak = Arc::downgrade(host);
    tokio::spawn(async move {
        let mut due = false;
        loop {
            let event = tokio::time::timeout(Duration::from_millis(800), events.recv()).await;
            match event {
                Ok(Ok(e)) => match e["event"].as_str() {
                    Some("auth") | Some("signIn") => {
                        let key = format!(
                            "{}\0{}",
                            e["provider"].as_str().unwrap_or_default(),
                            e["account"].as_str().unwrap_or_default()
                        );
                        BASES.lock().unwrap_or_else(|e| e.into_inner()).remove(&key);
                        due = true;
                    }
                    Some("exit") => break,
                    _ => {}
                },
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => due = true,
                Ok(Err(_)) => break,
                // Quiet a moment after a change: list once for several.
                Err(_) if due => {
                    due = false;
                    if weak.upgrade().is_none() {
                        break;
                    }
                    if let Err(e) = list_now().await {
                        tracing::warn!("listing the plugins' providers: {e:#}");
                    }
                }
                Err(_) => {}
            }
        }
    });
}

/// The host, started when there is none (or `plugins.json` changed, or it
/// died); `None` when no plugin it runs is on.
async fn get() -> Result<Option<Arc<Host>>> {
    crate::on_rt(async {
        let mut current = CURRENT.lock().await;
        let stamp = store::stamp();
        let home = paths::home();
        if let Some(h) = current.as_ref()
            && h.rpc.alive()
            && h.stamp == stamp
            && h.home == home
        {
            return Ok(Some(h.clone()));
        }
        if let Some(old) = current.take() {
            old.rpc.kill();
            BASES.lock().unwrap_or_else(|e| e.into_inner()).clear();
        }
        let chosen = plugins()?;
        if chosen.is_empty() {
            return Ok(None);
        }
        let host = start(chosen).await?;
        *current = Some(host.clone());
        Ok(Some(host))
    })
    .await
}

async fn host() -> Result<Arc<Host>> {
    get()
        .await?
        .ok_or_else(|| anyhow!("no provider plugin is on: `pansy plugin add` one"))
}

/// How each plugin the host runs loaded: the providers it signs in to, or
/// why it didn't.
pub async fn loaded() -> Result<Vec<Loaded>> {
    Ok(get().await?.map(|h| h.loaded.clone()).unwrap_or_default())
}

/// Asks the plugins for their providers, accounts and models, and keeps
/// what they said in `plugin-providers.json`; with no plugin on, keeps that
/// there are none.
pub async fn list_now() -> Result<Listing> {
    let Some(host) = get().await? else {
        let listing = Listing {
            at: Some(chrono::Utc::now()),
            ..Listing::default()
        };
        listing.save()?;
        return Ok(listing);
    };
    let rows = host
        .rpc
        .call("providers", json!({}), Duration::from_secs(90))
        .await?;
    let providers: Vec<Listed> = serde_json::from_value(rows)?;
    {
        let mut limits = LIMITS.lock().unwrap_or_else(|e| e.into_inner());
        limits.clear();
        for p in providers.iter().filter(|p| p.max_concurrency > 0) {
            limits.insert(
                p.id.clone(),
                Arc::new(Semaphore::new(p.max_concurrency as usize)),
            );
        }
    }
    let listing = Listing {
        at: Some(chrono::Utc::now()),
        providers,
        loaded: host.loaded.clone(),
    };
    listing.save()?;
    Ok(listing)
}

/// The base URL `account` of the plugin's `provider` is asked at, as its
/// loader says; `None` when it says none (the provider's own is used).
pub async fn base_url(provider: &str, account: &str) -> Result<Option<String>> {
    let key = format!("{provider}\0{account}");
    if let Some(b) = BASES.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return Ok(Some(b.clone()).filter(|b| !b.is_empty()));
    }
    let host = host().await?;
    let loaded = host
        .rpc
        .call(
            "load",
            json!({"provider": provider, "account": account}),
            Duration::from_secs(60),
        )
        .await?;
    let base = loaded["baseURL"].as_str().unwrap_or_default().to_owned();
    BASES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key, base.clone());
    Ok(Some(base).filter(|b| !b.is_empty()))
}

/// A request a plugin's provider is to send.
#[derive(Debug, Clone, Default)]
pub struct Request {
    /// The provider's id in the plugin.
    pub provider: String,
    /// The account, as the host keeps it (`id`, `id#2`).
    pub account: String,
    /// The model's id where it is asked.
    pub model: String,
    /// The AI SDK package its requests are made through.
    pub npm: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub session: String,
    /// The provider's or account's proxy: `direct`, a URL, or empty for
    /// pansy's.
    pub proxy: String,
}

/// Sends `request` through its plugin; the vendor's reply as it streams.
pub async fn fetch(request: Request) -> Result<reqwest::Response> {
    let host = host().await?;
    let limit = LIMITS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&request.provider)
        .cloned();
    let permit = match limit {
        Some(s) => Some(s.acquire_owned().await?),
        None => None,
    };
    let headers: serde_json::Map<String, Value> = request
        .headers
        .iter()
        .map(|(k, v)| (k.clone(), json!(v)))
        .collect();
    let params = json!({
        "provider": request.provider,
        "account": request.account,
        "model": request.model,
        "npm": request.npm,
        "url": request.url,
        "method": "POST",
        "headers": headers,
        "session": request.session,
        "proxy": request.proxy,
    });
    let (status, head, body) = host.rpc.fetch(params, &request.body, HEAD_WAIT).await?;
    let stream = futures::stream::unfold(
        (Box::pin(body.into_stream()), permit),
        |(mut s, permit)| async move {
            use futures::StreamExt;
            let next = s.next().await?;
            Some((next, (s, permit)))
        },
    );
    let mut response = http::Response::new(reqwest::Body::wrap_stream(stream));
    *response.status_mut() = http::StatusCode::from_u16(status)?;
    for (k, v) in head {
        if let (Ok(k), Ok(v)) = (
            http::HeaderName::from_bytes(k.as_bytes()),
            http::HeaderValue::from_str(&v),
        ) {
            response.headers_mut().append(k, v);
        }
    }
    Ok(reqwest::Response::from(response))
}

// --- Signing in ---------------------------------------------------------------------

/// What `authorize` answered: a page to open, and how its sign-in ends.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Authorized {
    pub ok: bool,
    pub error: String,
    pub session: String,
    pub url: String,
    pub instructions: String,
    /// `auto` (it ends by itself) or `code` (a code is pasted back).
    pub method: String,
    /// The plugin opened the page itself.
    pub opened: bool,
}

/// A sign-in's end.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Saved {
    pub ok: bool,
    pub error: String,
    pub provider: String,
    pub account: String,
}

/// The next question a sign-in method asks, given the answers so far;
/// `None` when it has none left.
pub async fn prompt(provider: &str, method: usize, inputs: &Value) -> Result<Option<Value>> {
    let host = host().await?;
    let r = host
        .rpc
        .call(
            "prompt",
            json!({"provider": provider, "method": method, "inputs": inputs}),
            Duration::from_secs(60),
        )
        .await?;
    Ok(Some(r["prompt"].clone()).filter(|p| !p.is_null()))
}

/// Why an answer to a question won't do; `None` when it will.
pub async fn validate(
    provider: &str,
    method: usize,
    key: &str,
    value: &str,
) -> Result<Option<String>> {
    let host = host().await?;
    let r = host
        .rpc
        .call(
            "validate",
            json!({"provider": provider, "method": method, "key": key, "value": value}),
            Duration::from_secs(30),
        )
        .await?;
    Ok(r["error"]
        .as_str()
        .map(str::to_owned)
        .filter(|e| !e.is_empty()))
}

/// Begins an OAuth sign-in.
pub async fn authorize(
    provider: &str,
    method: usize,
    inputs: &Value,
    account: Option<&str>,
) -> Result<Authorized> {
    let host = host().await?;
    let r = host
        .rpc
        .call(
            "authorize",
            json!({"provider": provider, "method": method, "inputs": inputs, "account": account.unwrap_or("new")}),
            Duration::from_secs(120),
        )
        .await?;
    Ok(serde_json::from_value(r)?)
}

/// Ends an OAuth sign-in: waits for the browser, or takes the code pasted.
pub async fn callback(session: &str, code: Option<&str>) -> Result<Saved> {
    let host = host().await?;
    let mut params = json!({"session": session});
    if let Some(code) = code {
        params["code"] = json!(code);
    }
    let r = host
        .rpc
        .call("callback", params, Duration::from_secs(15 * 60))
        .await?;
    let saved: Saved = serde_json::from_value(r)?;
    let _ = list_now().await;
    Ok(saved)
}

/// Signs in with a key (empty for a method that needs none, or asks it).
pub async fn api_key(
    provider: &str,
    method: usize,
    inputs: &Value,
    key: &str,
    account: Option<&str>,
) -> Result<Saved> {
    let host = host().await?;
    let r = host
        .rpc
        .call(
            "apiKey",
            json!({"provider": provider, "method": method, "inputs": inputs, "key": key, "account": account.unwrap_or("new")}),
            Duration::from_secs(120),
        )
        .await?;
    let saved: Saved = serde_json::from_value(r)?;
    let _ = list_now().await;
    Ok(saved)
}

/// Signs out an account, or every account of the provider.
pub async fn sign_out(provider: &str, account: Option<&str>) -> Result<()> {
    let host = host().await?;
    let mut params = json!({"provider": provider});
    if let Some(a) = account {
        params["account"] = json!(a);
    }
    host.rpc
        .call("signOut", params, Duration::from_secs(30))
        .await?;
    list_now().await?;
    Ok(())
}

/// Renews every renewing account that comes due, once a minute, while the
/// gateway serves.
pub async fn keep_signed_in() {
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
        let listing = Listing::load();
        for p in listing.providers.iter().filter(|p| p.renews) {
            for a in &p.accounts {
                let Ok(Some(host)) = get().await else {
                    return;
                };
                let said = host
                    .rpc
                    .call(
                        "keepSignedIn",
                        json!({"provider": p.id, "account": a.key}),
                        Duration::from_secs(60),
                    )
                    .await;
                match said {
                    Ok(r) if r["gone"] == true => {
                        tracing::warn!("{} {}: signed out ({})", p.id, a.key, r["error"]);
                    }
                    Ok(_) => {}
                    Err(e) => tracing::debug!("renewing {} {}: {e:#}", p.id, a.key),
                }
            }
        }
    }
}

// --- Request hooks (`magpie.hooks`) ----------------------------------------------------

/// The request hooks of one request, begun.
pub struct Hooks {
    id: String,
    token: String,
    host: Arc<Host>,
    finished: bool,
}

/// What the hooks are told of a request.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookContext {
    pub protocol: String,
    pub model: String,
    pub stream: bool,
    pub path: String,
    /// `local` for this computer without a key, else `gateway-key` or
    /// `remote`.
    pub origin: String,
    pub agent: String,
    pub session: String,
    pub cwd: String,
    pub query: String,
    pub kind: String,
}

/// Runs each hook's `onRequest` on `body` before it is routed: the body
/// they leave, and the run their `onResponse` is told the reply on. `None`
/// when no hook is on, or the host can't run them now (the request goes as
/// it came).
pub async fn hooks_begin(body: &Value, context: &HookContext) -> Option<(Value, Hooks)> {
    if !has_hooks() {
        return None;
    }
    let host = match tokio::time::timeout(Duration::from_secs(11), host()).await {
        Ok(Ok(h)) => h,
        _ => return None,
    };
    let r = host
        .rpc
        .call(
            "gatewayRequest",
            json!({"operation": "begin", "body": body, "context": context}),
            Duration::from_secs(31),
        )
        .await
        .ok()?;
    let id = r["id"].as_str()?.to_owned();
    let token = r["token"].as_str()?.to_owned();
    Some((
        r["body"].clone(),
        Hooks {
            id,
            token,
            host,
            finished: false,
        },
    ))
}

impl Hooks {
    /// Tells the hooks the reply the agent got, once it is whole.
    pub fn finish(mut self, reply: Value, success: bool) {
        self.finished = true;
        let (rpc, id, token) = (self.host.rpc.clone(), self.id.clone(), self.token.clone());
        crate::rt().spawn(async move {
            let _ = rpc
                .call(
                    "gatewayRequest",
                    json!({"operation": "finish", "id": id, "token": token, "reply": reply, "success": success}),
                    Duration::from_secs(31),
                )
                .await;
        });
    }
}

impl Drop for Hooks {
    fn drop(&mut self) {
        if !self.finished {
            self.host.rpc.notify(
                "gatewayRequest",
                json!({"operation": "close", "id": self.id, "token": self.token}),
            );
        }
    }
}

/// Fails when no provider plugin is on, for callers that need one.
pub async fn require() -> Result<()> {
    if get().await?.is_none() {
        bail!("no provider plugin is on");
    }
    Ok(())
}
