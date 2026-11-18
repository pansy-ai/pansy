//! What pansy wrote into each agent's config (`agents.json`, private: it
//! holds the values pansy replaced, keys among them): the model chosen,
//! every key it set with the value that was there before, and each file as
//! it was, so disconnecting puts back exactly what pansy changed and
//! nothing else.

use std::collections::BTreeMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pansy_core::fsutil::{read_json, write_json};
use pansy_core::paths;

/// One key pansy set: in which file, at which path, over which value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Touched {
    pub file: String,
    pub path: Vec<String>,
    /// What was there before pansy first set it; `None`: nothing.
    pub before: Option<Value>,
}

/// A file pansy changed: as it was, and as pansy left it. While nobody else
/// changed it since, disconnecting puts it back whole, as it was byte for
/// byte (or removes it, when pansy made it).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FileState {
    /// The whole file before pansy first changed it; `None`: there was none.
    pub before: Option<String>,
    /// The file as pansy last wrote it ([`digest`]).
    pub wrote: String,
}

/// A text's fingerprint (FNV-1a), the same on every run and version.
pub fn digest(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentState {
    /// The model the agent is on, as pansy set it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The model for the agent's quick tasks, where it has a second one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub small_model: Option<String>,
    /// The model the agent's subagents start on, where it has a key for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_model: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub touched: Vec<Touched>,
    /// The files pansy changed, by path, while the agent is connected.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub files: BTreeMap<String, FileState>,
    /// Models the user keeps at hand for this agent, most recent first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent: Vec<String>,
    /// Put back for now (Diagnostics' Restore for now, `pansy pause`): the
    /// model it was on, to connect it again with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub agents: BTreeMap<String, AgentState>,
}

impl State {
    pub fn load() -> Result<State> {
        read_json(&paths::agents())
    }

    pub fn save(&self) -> Result<()> {
        write_json(&paths::agents(), self, true)
    }

    pub fn agent(&mut self, id: &str) -> &mut AgentState {
        self.agents.entry(id.to_owned()).or_default()
    }
}

impl AgentState {
    /// Notes `model` among the recent ones (eight kept).
    pub fn remember(&mut self, model: &str) {
        self.recent.retain(|m| m != model);
        self.recent.insert(0, model.to_owned());
        self.recent.truncate(8);
    }
}
