//! The agents plugins add (`magpie.agent`): where an agent pansy has no
//! setup of its own keeps its model (`config`, `format`, `model`, `prefix`),
//! and a `connect` that gives the other keys to write. Host B loads them;
//! what it loaded is kept in `agent-plugins.json`, so listing the agents
//! starts no host.

use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use pansy_core::fsutil::{read_json, write_json};
use pansy_core::paths;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Where an agent keeps the model a new session starts on, apart from
/// where its providers are.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Startup {
    pub config: String,
    pub format: String,
    pub model: String,
    /// Written beside the model, as they are.
    pub keys: serde_json::Map<String, Value>,
}

/// An agent a plugin adds, as its module says it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PluginAgent {
    /// The plugin it comes from.
    pub spec: String,
    pub id: String,
    pub name: String,
    /// Its config file (`~/.aider.conf.yml`).
    pub config: String,
    /// `json`, `jsonc`, `toml`, `yaml` or `env`; else by the file's ending.
    pub format: String,
    /// The key its model is at (dots for a nested one); empty for an agent
    /// connected without one.
    pub model: String,
    /// What goes before pansy's model id (`openai/`).
    pub prefix: String,
    /// Its program, which says it is installed.
    pub bin: String,
    /// Its folder, which says so too.
    pub dir: String,
    pub icon: String,
    pub notice: String,
    pub startup: Option<Startup>,
    /// It has a `connect`.
    pub connect: bool,
}

/// What host B last loaded.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Kept {
    pub agents: Vec<PluginAgent>,
    /// The plugins that didn't load, and why.
    pub errors: Vec<(String, String)>,
}

/// Keeps what host B's `init` said of the agents.
pub(crate) fn keep(said: &Value) {
    let mut kept = Kept::default();
    for a in said.as_array().into_iter().flatten() {
        let spec = a["spec"].as_str().unwrap_or_default().to_owned();
        if let Some(e) = a["error"].as_str() {
            kept.errors.push((spec, e.to_owned()));
            continue;
        }
        let Ok(mut agent) = serde_json::from_value::<PluginAgent>(a["agent"].clone()) else {
            kept.errors
                .push((spec, "its agent isn't one pansy reads".into()));
            continue;
        };
        agent.spec = spec;
        agent.connect = a["connect"] == true;
        kept.agents.push(agent);
    }
    if let Err(e) = write_json(&paths::agent_plugins(), &kept, true) {
        tracing::warn!("keeping the plugins' agents: {e:#}");
    }
}

/// The agents plugins add, as last loaded.
pub fn cached() -> Kept {
    read_json(&paths::agent_plugins()).unwrap_or_default()
}

/// Loads the agent plugins again (host B starts if it isn't running).
pub async fn reload() -> Result<Kept> {
    crate::middleware::get().await?;
    Ok(cached())
}

/// What `connect` of the agent `id` gives: the keys to write, each a dotted
/// path, for the gateway at `url` (`http://127.0.0.1:7267`) with `key`, the
/// model picked (`None` for an agent connected without one) and every model
/// it may be given.
pub async fn connect(
    id: &str,
    url: &str,
    key: &str,
    model: Option<&str>,
    models: &[(String, String)],
) -> Result<serde_json::Map<String, Value>> {
    let agent = cached()
        .agents
        .into_iter()
        .find(|a| a.id == id)
        .ok_or_else(|| anyhow!("no plugin adds an agent {id}"))?;
    if !agent.connect {
        return Ok(Default::default());
    }
    let host = crate::middleware::get()
        .await?
        .ok_or_else(|| anyhow!("the plugin that adds {id} is off"))?;
    let url = url.trim_end_matches('/');
    let args = json!({
        "gateway": {"url": url, "v1": format!("{url}/v1"), "key": key},
        "model": model,
        "models": models.iter().map(|(id, name)| json!({"id": id, "name": name})).collect::<Vec<_>>(),
    });
    let keys = host
        .rpc_call(
            "connect",
            json!({"spec": agent.spec, "id": id, "args": args}),
        )
        .await?;
    match keys {
        Value::Object(m) => Ok(m),
        Value::Null => Ok(Default::default()),
        _ => bail!("{id}'s connect gave what isn't an object of keys"),
    }
}

impl crate::middleware::Host {
    pub(crate) async fn rpc_call(&self, method: &str, params: Value) -> Result<Value> {
        self.rpc_ref()
            .call(method, params, Duration::from_secs(10))
            .await
    }
}
