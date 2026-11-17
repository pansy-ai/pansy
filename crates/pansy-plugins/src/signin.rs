//! Signing in to a plugin's provider from the window or the command line:
//! the method's questions one at a time, then a page to open (and a code
//! to paste back, for some), or a key; the account is the host's to keep.
//!
//! A sign-in is a [`Flow`] the caller polls ([`status`]) and answers
//! ([`answer`]); the waits run on the plugins' runtime.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use anyhow::{Result, anyhow, bail};
use pansy_core::plugin::{Listing, Method};
use serde::Serialize;
use serde_json::{Value, json};

use crate::host;

/// Where a sign-in stands.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Flow {
    pub id: String,
    pub provider: String,
    pub method: usize,
    /// `asking` (a question, in `prompt`), `key` (a key to type), `waiting`
    /// (the page in `url` is open; it ends by itself), `code` (the code the
    /// page gives is to be pasted), `done`, `failed` or `canceled`.
    pub state: String,
    /// The question: `{type: "text"|"select", key, message, placeholder,
    /// options: [{label, value, hint}]}`.
    pub prompt: Option<Value>,
    /// Why the last answer won't do.
    pub invalid: String,
    pub url: String,
    pub instructions: String,
    /// The plugin opened the page itself.
    pub opened: bool,
    /// The account signed in to, once done.
    pub account: String,
    pub error: String,
    #[serde(skip)]
    inputs: serde_json::Map<String, Value>,
    #[serde(skip)]
    session: String,
    #[serde(skip)]
    kind: String,
    #[serde(skip)]
    plain: bool,
}

static FLOWS: LazyLock<Mutex<HashMap<String, Flow>>> = LazyLock::new(Default::default);
/// The waits on the browser, to stop when a sign-in is given up.
static WAITS: LazyLock<Mutex<HashMap<String, tokio::task::AbortHandle>>> =
    LazyLock::new(Default::default);

fn put(flow: &Flow) {
    FLOWS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(flow.id.clone(), flow.clone());
}

fn get(id: &str) -> Option<Flow> {
    FLOWS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(id)
        .cloned()
}

/// The ways `provider` (the plugin's id) is signed in to.
pub fn methods(provider: &str) -> Result<Vec<Method>> {
    Listing::load()
        .providers
        .into_iter()
        .find(|p| p.id == provider)
        .map(|p| p.methods)
        .ok_or_else(|| {
            anyhow!("no plugin signs in to {provider}: `pansy plugin refresh` lists them again")
        })
}

/// Begins signing in to `provider` by its `method`th way.
pub async fn start(provider: &str, method: usize) -> Result<Flow> {
    let methods = methods(provider)?;
    let m = methods
        .get(method)
        .ok_or_else(|| anyhow!("{provider} has no sign-in method {method}"))?;
    let mut flow = Flow {
        id: uuid::Uuid::new_v4().simple().to_string()[..12].to_owned(),
        provider: provider.to_owned(),
        method,
        state: "asking".into(),
        kind: m.kind.clone(),
        plain: m.kind == "api" && !m.keyless && !m.key_asked,
        ..Flow::default()
    };
    step(&mut flow).await;
    put(&flow);
    Ok(flow)
}

/// Asks the next question, or goes on once there is none left.
async fn step(flow: &mut Flow) {
    let inputs = Value::Object(flow.inputs.clone());
    match host::prompt(&flow.provider, flow.method, &inputs).await {
        Ok(Some(p)) => {
            flow.state = "asking".into();
            flow.prompt = Some(p);
            return;
        }
        Ok(None) => flow.prompt = None,
        Err(e) => return fail(flow, e),
    }
    if flow.kind == "oauth" {
        match host::authorize(&flow.provider, flow.method, &inputs, None).await {
            Ok(a) if a.ok => {
                flow.url = a.url;
                flow.instructions = a.instructions;
                flow.opened = a.opened;
                flow.session = a.session;
                if a.method == "code" {
                    flow.state = "code".into();
                } else {
                    flow.state = "waiting".into();
                    wait(flow.clone());
                }
            }
            Ok(a) => fail(flow, anyhow!(a.error)),
            Err(e) => fail(flow, e),
        }
    } else if flow.plain {
        flow.state = "key".into();
    } else {
        key(flow, "").await;
    }
}

fn fail(flow: &mut Flow, e: anyhow::Error) {
    flow.state = "failed".into();
    flow.error = format!("{e:#}");
}

/// Waits on the browser for an `auto` sign-in, on the plugins' runtime.
fn wait(flow: Flow) {
    let id = flow.id.clone();
    let task = crate::rt().spawn(async move {
        let saved = host::callback(&flow.session, None).await;
        let mut flows = FLOWS.lock().unwrap_or_else(|e| e.into_inner());
        let Some(f) = flows.get_mut(&flow.id) else {
            return;
        };
        if f.state != "waiting" {
            return;
        }
        match saved {
            Ok(s) if s.ok => {
                f.state = "done".into();
                f.account = s.account;
            }
            Ok(s) => {
                f.state = "failed".into();
                f.error = if s.error.is_empty() {
                    "the sign-in didn't finish".into()
                } else {
                    s.error
                };
            }
            Err(e) => {
                f.state = "failed".into();
                f.error = format!("{e:#}");
            }
        }
    });
    WAITS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id, task.abort_handle());
}

async fn key(flow: &mut Flow, key: &str) {
    let inputs = Value::Object(flow.inputs.clone());
    match host::api_key(&flow.provider, flow.method, &inputs, key, None).await {
        Ok(s) if s.ok => {
            flow.state = "done".into();
            flow.account = s.account;
        }
        Ok(s) => fail(
            flow,
            anyhow!(if s.error.is_empty() {
                "the key wasn't taken".into()
            } else {
                s.error
            }),
        ),
        Err(e) => fail(flow, e),
    }
}

/// Answers what the sign-in waits for: the question asked (`asking`), the
/// key (`key`) or the code the page gave (`code`).
pub async fn answer(id: &str, value: &str) -> Result<Flow> {
    let mut flow = get(id).ok_or_else(|| anyhow!("no sign-in {id}"))?;
    match flow.state.as_str() {
        "asking" => {
            let p = flow.prompt.clone().unwrap_or(Value::Null);
            let name = p["key"].as_str().unwrap_or_default().to_owned();
            match host::validate(&flow.provider, flow.method, &name, value).await? {
                Some(why) => flow.invalid = why,
                None => {
                    flow.invalid.clear();
                    flow.inputs.insert(name, json!(value));
                    step(&mut flow).await;
                }
            }
        }
        "key" => {
            if value.trim().is_empty() {
                flow.invalid = "type the key".into();
            } else {
                flow.invalid.clear();
                key(&mut flow, value.trim()).await;
            }
        }
        "code" => {
            let saved = host::callback(&flow.session, Some(value.trim())).await;
            match saved {
                Ok(s) if s.ok => {
                    flow.state = "done".into();
                    flow.account = s.account;
                }
                Ok(s) => fail(&mut flow, anyhow!(s.error)),
                Err(e) => fail(&mut flow, e),
            }
        }
        other => bail!("the sign-in is {other}, not waiting for an answer"),
    }
    put(&flow);
    Ok(flow)
}

/// Where a sign-in stands; `None` when it is gone.
pub fn status(id: &str) -> Option<Flow> {
    get(id)
}

/// Gives a sign-in up.
pub fn cancel(id: &str) {
    if let Some(f) = FLOWS.lock().unwrap_or_else(|e| e.into_inner()).get_mut(id)
        && !matches!(f.state.as_str(), "done" | "failed")
    {
        f.state = "canceled".into();
    }
    // Its call let go of, the host stops the sign-in (and the port it
    // waits on).
    if let Some(task) = WAITS.lock().unwrap_or_else(|e| e.into_inner()).remove(id) {
        task.abort();
    }
}
