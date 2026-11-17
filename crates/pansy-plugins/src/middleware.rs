//! Host B: pansy's `mw-host.js` on Bun, which runs magpie's gateway
//! middleware (`magpie.middleware`) on the requests agents send and the
//! replies they get, the routings middleware give (`strategies`), each
//! usage row (`onUsage`), and the agents plugins add ([`crate::agents`]).
//!
//! The gateway knows only [`Engine`]: Bun's is the one there is, and one
//! that runs middleware shut in (QuickJS) can take its place. Every hook
//! fails open: a hook that fails, or doesn't answer in its time, leaves the
//! request, the event or the reply as it was. A host that stops answering
//! altogether is started again.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, SystemTime};

use anyhow::{Result, anyhow};
use hmac::{Hmac, Mac};
use pansy_core::paths;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Sha256;

use crate::rpc::Rpc;
use crate::store::{self, Installed};

const REQUEST_LIMIT: Duration = Duration::from_millis(250);
const EVENT_LIMIT: Duration = Duration::from_millis(50);
const STRATEGY_LIMIT: Duration = Duration::from_millis(20);
/// A host that answers nothing this long after a hook ran out of time is
/// started again.
const STUCK: Duration = Duration::from_secs(5);

/// One middleware as host B loaded it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Loaded {
    /// What it is called by: its routings' prefix, its account refs' salt.
    pub name: String,
    pub spec: String,
    pub hooks: Vec<String>,
    pub events: Option<Vec<String>>,
    pub strategies: Vec<String>,
    pub usage: bool,
    pub error: Option<String>,
}

pub(crate) struct Host {
    rpc: Arc<Rpc>,
    home: PathBuf,
    stamp: Option<SystemTime>,
    pub(crate) middleware: Vec<Loaded>,
    checking: AtomicBool,
}

static CURRENT: LazyLock<tokio::sync::Mutex<Option<Arc<Host>>>> =
    LazyLock::new(|| tokio::sync::Mutex::new(None));

/// Stops the host this process runs.
pub fn forget() {
    if let Ok(mut current) = CURRENT.try_lock()
        && let Some(host) = current.take()
    {
        host.rpc.kill();
    }
}

/// The name a middleware goes by: its package's short name, or a file's
/// base name without `.middleware`/`.gateway` and its extension, made of
/// `A-Za-z0-9._-`.
pub fn name_of(i: &Installed) -> String {
    let raw = if store::is_path(&i.entry.spec) && i.target.is_file() {
        let file = i
            .target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        file.split('.').next().unwrap_or(&file).to_owned()
    } else {
        store::short_name(&i.entry.package())
    };
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._-".contains(c) {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// The plugins host B loads: those on that are middleware or agents.
pub fn plugins() -> Result<Vec<Installed>> {
    Ok(store::installed()?
        .into_iter()
        .filter(|i| !i.entry.off && (i.kinds.middleware.is_some() || i.kinds.agent.is_some()))
        .collect())
}

/// Some middleware is on: requests go through host B.
pub fn wanted() -> bool {
    plugins()
        .unwrap_or_default()
        .iter()
        .any(|i| i.kinds.middleware.is_some())
}

async fn start(chosen: Vec<Installed>) -> Result<Arc<Host>> {
    let bun = crate::bun::ensure().await?;
    let script = crate::scripts::write("mw-host", crate::scripts::MW)?;
    let home = paths::home();
    let middleware: Vec<Value> = chosen
        .iter()
        .filter_map(|i| {
            Some(json!({
                "name": name_of(i),
                "spec": i.entry.spec,
                "file": i.kinds.middleware.as_ref()?,
                "options": i.entry.options_value(),
            }))
        })
        .collect();
    let agents: Vec<Value> = chosen
        .iter()
        .filter_map(|i| {
            Some(json!({
                "spec": i.entry.spec,
                "file": i.kinds.agent.as_ref()?,
                "options": i.entry.options_value(),
            }))
        })
        .collect();
    let rpc = Rpc::start(&bun, &script, &home, &[], "middleware host")?;
    let init = rpc
        .call(
            "init",
            json!({"middleware": middleware, "agents": agents}),
            Duration::from_secs(30),
        )
        .await
        .map_err(|e| {
            rpc.kill();
            anyhow!("starting the middleware: {e:#}")
        })?;
    let loaded: Vec<Loaded> =
        serde_json::from_value(init["middleware"].clone()).unwrap_or_default();
    for m in &loaded {
        if let Some(e) = &m.error {
            tracing::warn!("middleware {}: {e}", m.name);
        }
    }
    crate::agents::keep(&init["agents"]);
    Ok(Arc::new(Host {
        rpc,
        home,
        stamp: store::stamp(),
        middleware: loaded,
        checking: AtomicBool::new(false),
    }))
}

/// Host B, started when there is none (or `plugins.json` changed, or it
/// died); `None` when no middleware or agent plugin is on.
pub(crate) async fn get() -> Result<Option<Arc<Host>>> {
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
        }
        let chosen = plugins()?;
        if chosen.is_empty() {
            crate::agents::keep(&json!([]));
            return Ok(None);
        }
        let host = start(chosen).await?;
        *current = Some(host.clone());
        Ok(Some(host))
    })
    .await
}

impl Host {
    pub(crate) fn rpc_ref(&self) -> &Arc<Rpc> {
        &self.rpc
    }

    /// Calls `method` within `limit`; given no answer, checks the host still
    /// answers anything, and starts it again when it doesn't.
    async fn call(self: &Arc<Self>, method: &str, params: Value, limit: Duration) -> Option<Value> {
        match self.rpc.call(method, params, limit).await {
            Ok(v) => Some(v),
            Err(e) => {
                tracing::debug!("middleware {method}: {e:#}");
                self.check();
                None
            }
        }
    }

    fn check(self: &Arc<Self>) {
        if self.checking.swap(true, Ordering::SeqCst) {
            return;
        }
        let host = self.clone();
        crate::rt().spawn(async move {
            if host.rpc.call("ping", json!({}), STUCK).await.is_err() && host.rpc.alive() {
                tracing::warn!(
                    "the middleware host stopped answering (a hook that never ends?); starting it again"
                );
                host.rpc.kill();
            }
            host.checking.store(false, Ordering::SeqCst);
        });
    }

    fn has(&self, hook: &str) -> usize {
        self.middleware
            .iter()
            .filter(|m| m.hooks.iter().any(|h| h == hook))
            .count()
    }
}

// --- The engine the gateway runs ------------------------------------------------------

/// What a request's hooks are told of it (`ctx`).
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    /// The agent's API: `anthropic`, `chat`, `responses`, `gemini`.
    pub protocol: String,
    pub model: String,
    pub stream: bool,
    pub path: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub agent: String,
}

/// A request a middleware turned away (`ctx.reject`).
#[derive(Debug, Clone)]
pub struct Reject {
    pub status: u16,
    pub message: String,
}

/// Where one try goes (`ctx.provider`).
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct To {
    pub id: String,
    pub name: String,
    /// The subscription's kind, or the plugin's provider; empty for a key.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub account: String,
    /// The model asked of it, by its id there.
    pub model: String,
}

/// The account a try goes to, as the gateway knows it: who it is never
/// reaches a middleware, only a ref made from it.
#[derive(Debug, Clone, Default)]
pub struct Account {
    /// The subscription's kind (`codex`), or the plugin's provider.
    pub kind: String,
    pub user: String,
    pub plan: String,
    /// The provider's id and name, and the model asked of it.
    pub provider: String,
    pub name: String,
    pub model: String,
}

/// What an event's hooks made of it.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Keep,
    Replace(Value),
    Drop,
}

/// Runs middleware on requests: Bun's host B, or another.
pub trait Engine: Send + Sync {
    fn begin(&self, info: Info) -> futures::future::BoxFuture<'static, Option<Run>>;
}

/// The middleware of one request.
pub struct Run {
    id: String,
    host: Arc<Host>,
    ended: bool,
}

/// Begins `info`'s middleware; `None` when none is on (or it can't run now:
/// the request goes as it came).
pub async fn begin(info: Info) -> Option<Run> {
    if !wanted() {
        return None;
    }
    let host = match tokio::time::timeout(Duration::from_secs(15), get()).await {
        Ok(Ok(Some(h))) => h,
        Ok(Err(e)) => {
            tracing::warn!("middleware: {e:#}");
            return None;
        }
        _ => return None,
    };
    if host.middleware.iter().all(|m| m.hooks.is_empty()) {
        return None;
    }
    let r = host
        .call("begin", json!({"info": info}), REQUEST_LIMIT)
        .await?;
    Some(Run {
        id: r["run"].as_str()?.to_owned(),
        host,
        ended: false,
    })
}

/// Bun's engine.
pub struct Bun;

impl Engine for Bun {
    fn begin(&self, info: Info) -> futures::future::BoxFuture<'static, Option<Run>> {
        Box::pin(begin(info))
    }
}

fn limit(n: usize, each: Duration) -> Duration {
    each * n.max(1) as u32 + Duration::from_millis(100)
}

impl Run {
    /// Runs each `onRequest`: the body they leave (`None`: as it came), or
    /// a request turned away.
    pub async fn request(&self, body: &Value) -> Result<Option<Value>, Reject> {
        let n = self.host.has("onRequest");
        if n == 0 {
            return Ok(None);
        }
        let Some(r) = self
            .host
            .call(
                "request",
                json!({"run": self.id, "body": body}),
                limit(n, REQUEST_LIMIT),
            )
            .await
        else {
            return Ok(None);
        };
        if let Some(j) = r.get("reject").filter(|j| j.is_object()) {
            return Err(Reject {
                status: j["status"].as_u64().unwrap_or(403) as u16,
                message: j["message"]
                    .as_str()
                    .unwrap_or("turned away by a gateway middleware")
                    .to_owned(),
            });
        }
        Ok(r.get("body").cloned())
    }

    /// Runs each `onAccount` before a try on `account` (`None` for a key):
    /// the middleware that passed it over, if one did.
    pub async fn account(&self, account: Option<&Account>) -> Option<String> {
        let n = self.host.has("onAccount");
        if n == 0 {
            return None;
        }
        let (a, refs) = match account {
            Some(a) => {
                let refs: serde_json::Map<String, Value> = self
                    .host
                    .middleware
                    .iter()
                    .map(|m| (m.name.clone(), json!(account_ref(&m.name, a))))
                    .collect();
                (
                    json!({"provider": a.provider, "name": a.name, "kind": a.kind, "model": a.model}),
                    Value::Object(refs),
                )
            }
            None => (Value::Null, json!({})),
        };
        let r = self
            .host
            .call(
                "account",
                json!({"run": self.id, "account": a, "refs": refs}),
                limit(n, REQUEST_LIMIT),
            )
            .await?;
        (r["skip"] == true).then(|| r["by"].as_str().unwrap_or_default().to_owned())
    }

    /// Runs each `onProvider` on one try's body: the body that try is sent
    /// (`None`: as it was).
    pub async fn provider(&self, body: &Value, to: &To) -> Option<Value> {
        let n = self.host.has("onProvider");
        if n == 0 {
            return None;
        }
        let r = self
            .host
            .call(
                "provider",
                json!({"run": self.id, "body": body, "provider": to}),
                limit(n, REQUEST_LIMIT),
            )
            .await?;
        r.get("body").cloned()
    }

    /// Some middleware sees streamed events.
    pub fn wants_events(&self) -> bool {
        self.host.has("onEvent") > 0
    }

    /// Whether some middleware wants events named `name`.
    pub fn wants_event(&self, name: &str) -> bool {
        self.host.middleware.iter().any(|m| {
            m.hooks.iter().any(|h| h == "onEvent")
                && m.events
                    .as_ref()
                    .is_none_or(|e| e.iter().any(|e| e == name))
        })
    }

    /// Some middleware sees whole replies.
    pub fn wants_response(&self) -> bool {
        self.host.has("onResponse") > 0
    }

    /// Runs each `onEvent` that wants the event `name` on its `data`.
    pub async fn event(&self, name: &str, data: &Value) -> Event {
        if !self.wants_event(name) {
            return Event::Keep;
        }
        let n = self.host.has("onEvent");
        let Some(r) = self
            .host
            .call(
                "event",
                json!({"run": self.id, "name": name, "data": data}),
                limit(n, EVENT_LIMIT),
            )
            .await
        else {
            return Event::Keep;
        };
        if r["drop"] == true {
            return Event::Drop;
        }
        match r.get("data") {
            Some(d) => Event::Replace(d.clone()),
            None => Event::Keep,
        }
    }

    /// Runs each `onResponse` on a whole reply: the reply the agent gets
    /// (`None`: as it was).
    pub async fn response(&self, body: &Value, status: u16) -> Option<Value> {
        let n = self.host.has("onResponse");
        if n == 0 {
            return None;
        }
        let r = self
            .host
            .call(
                "response",
                json!({"run": self.id, "body": body, "status": status}),
                limit(n, REQUEST_LIMIT),
            )
            .await?;
        r.get("body").cloned()
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        if !self.ended {
            self.ended = true;
            let (host, id) = (self.host.clone(), self.id.clone());
            crate::rt().spawn(async move {
                let _ = host
                    .rpc
                    .call("end", json!({"run": id}), Duration::from_secs(1))
                    .await;
            });
        }
    }
}

// --- Account refs ----------------------------------------------------------------------

/// The key file read last, and its key.
type RefKey = Option<(PathBuf, Vec<u8>)>;

static REF_KEY: LazyLock<Mutex<RefKey>> = LazyLock::new(Default::default);

/// The key refs are made with: random, made the first time and kept in
/// `account-ref.key`, so a ref is the same after a restart. One that can't
/// be read is never written over; this process makes do with its own.
fn ref_key() -> Vec<u8> {
    let path = paths::account_ref_key();
    let mut kept = REF_KEY.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((p, k)) = kept.as_ref()
        && *p == path
    {
        return k.clone();
    }
    let key = match std::fs::read(&path) {
        Ok(k) if k.len() >= 32 => k,
        Ok(_) | Err(_) if path.exists() => {
            tracing::warn!(
                "{} can't be read: account refs are this run's only",
                path.display()
            );
            random_key()
        }
        _ => {
            let k = random_key();
            if let Err(e) = pansy_core::fsutil::write_atomic(&path, &k, true) {
                tracing::warn!("keeping the account ref key: {e:#}");
            }
            k
        }
    };
    *kept = Some((path, key.clone()));
    key
}

fn random_key() -> Vec<u8> {
    let mut k = Vec::with_capacity(32);
    k.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    k.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    k
}

/// `account` as middleware `name` is told it: a keyed hash of who it is
/// (its kind, user and plan), the same for every request and start,
/// different for each middleware, and no way back to the user without
/// this computer's key.
pub fn account_ref(name: &str, account: &Account) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(&ref_key()).expect("any key");
    for s in [
        "magpie account ref v1",
        name,
        &account.kind,
        &account.user.to_lowercase(),
        &account.plan.to_lowercase(),
    ] {
        mac.update(s.as_bytes());
        mac.update(&[0]);
    }
    mac.finalize().into_bytes()[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// --- Routings and usage -----------------------------------------------------------------

/// One seat of a pool a middleware's routing orders, in magpie's shape.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Seat {
    pub key: String,
    pub account: bool,
    pub known: bool,
    pub learns: bool,
    pub used: f64,
    pub soon: Vec<i64>,
    pub pace: f64,
    pub weight: i64,
    pub resting: bool,
    pub tokens: f64,
}

/// The routings middleware give, `<name>/<strategy>`.
pub async fn routings() -> Vec<String> {
    match get().await {
        Ok(Some(h)) => h
            .middleware
            .iter()
            .flat_map(|m| m.strategies.iter().map(move |s| format!("{}/{s}", m.name)))
            .collect(),
        _ => Vec::new(),
    }
}

/// The order `routing` (`<name>/<strategy>`) gives `seats` of `pool`; `None`
/// for their listed order (no such routing, or it failed).
pub async fn order(routing: &str, pool: &str, seats: &[Seat]) -> Option<Vec<usize>> {
    let (name, strategy) = routing.split_once('/')?;
    let host = get().await.ok()??;
    let r = host
        .call(
            "strategy",
            json!({"name": name, "strategy": strategy, "pool": pool, "seats": seats}),
            STRATEGY_LIMIT + Duration::from_millis(30),
        )
        .await?;
    serde_json::from_value(r["order"].clone()).ok()
}

static USAGE: LazyLock<Mutex<Option<tokio::sync::mpsc::Sender<Value>>>> =
    LazyLock::new(Default::default);
static DROPPED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Hands a usage row to each `onUsage`, off the request's path: queued
/// (1024), a full queue dropping it.
pub fn usage(record: &pansy_core::usage::Record) {
    if !wanted() {
        return;
    }
    let row = json!({
        "time": record.at.timestamp_millis(),
        "agent": record.agent,
        "provider": record.provider,
        "providerAccount": record.account,
        "model": record.model,
        "requested": record.requested,
        "input": record.input,
        "output": record.output,
        "cacheRead": record.cache_read,
        "cacheWrite": record.cache_write,
        "reasoning": record.reasoning,
        "ms": record.ms,
        "ttftMs": record.first_ms,
        "status": record.status,
        "tries": record.failovers + 1,
        "rejected": record.status == 403,
        "cost": null,
    });
    let mut queue = USAGE.lock().unwrap_or_else(|e| e.into_inner());
    let tx = queue.get_or_insert_with(|| {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Value>(1024);
        crate::rt().spawn(async move {
            while let Some(row) = rx.recv().await {
                let Ok(Some(host)) = get().await else {
                    continue;
                };
                if host.middleware.iter().any(|m| m.usage) {
                    let n = host.middleware.iter().filter(|m| m.usage).count();
                    let _ = host
                        .call("usage", json!({"row": row}), limit(n, REQUEST_LIMIT))
                        .await;
                }
            }
        });
        tx
    });
    if tx.try_send(row).is_err() {
        DROPPED.fetch_add(1, Ordering::Relaxed);
    }
}

/// Each middleware as it loaded, with its calls, failures, time a call and
/// last error.
pub async fn states() -> Result<Value> {
    let Some(host) = get().await? else {
        return Ok(json!({}));
    };
    let mut stats = host
        .rpc
        .call("stats", json!({}), Duration::from_secs(5))
        .await
        .unwrap_or_else(|_| json!({}));
    if let Some(o) = stats.as_object_mut() {
        o.insert(
            "_usageDropped".into(),
            json!(DROPPED.load(Ordering::Relaxed)),
        );
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_account_ref_is_the_same_each_time_and_each_middlewares_own() {
        let home = tempfile::tempdir().unwrap();
        let _env = crate::testing::scratch(home.path());
        let a = Account {
            kind: "codex".into(),
            user: "Ada@Example.com".into(),
            plan: "Pro".into(),
            ..Account::default()
        };
        let one = account_ref("guard", &a);
        assert_eq!(one.len(), 32);
        assert_eq!(one, account_ref("guard", &a));
        assert_ne!(one, account_ref("other", &a));
        let lower = Account {
            user: "ada@example.com".into(),
            plan: "pro".into(),
            ..a.clone()
        };
        assert_eq!(one, account_ref("guard", &lower));
        // Kept: the same after a restart.
        *REF_KEY.lock().unwrap() = None;
        assert_eq!(one, account_ref("guard", &a));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(paths::account_ref_key())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
