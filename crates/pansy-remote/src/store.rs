//! The threads pansy keeps: `threads.json` in pansy's home. A thread is
//! pansy's record of an agent's conversation: which agent, where it works
//! (a project, a worktree), on which model, and the agent's own session id
//! that resumes it. No conversation is in it; that stays in the agent's own
//! session files, read through [`pansy_agents::sessions`].

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::Result;
use chrono::{DateTime, Utc};
use pansy_core::fsutil::{read_json, write_json};
use pansy_core::paths;
use serde::{Deserialize, Serialize};

use crate::Mode;

/// A pull request a thread opened or was linked to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PullRequest {
    pub number: u64,
    pub url: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub id: String,
    /// The agent's id (`codex`, `claude-code`).
    pub agent: String,
    pub title: String,
    /// The project: a worktree thread's original repository.
    pub folder: String,
    /// Where the agent works: the project, or its worktree.
    pub workdir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default)]
    pub worktree: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default)]
    pub mode: Mode,
    /// The agent's session id, once it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub archived: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snoozed_until: Option<DateTime<Utc>>,
    #[serde(default)]
    pub unread: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<PullRequest>,
    /// How the last turn ended: `completed`, `interrupted`, `failed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Turns finished: the next checkpoint's number.
    #[serde(default)]
    pub turns: u32,
    /// The branch was named after the first prompt already.
    #[serde(default)]
    pub named: bool,
}

#[derive(Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    threads: BTreeMap<String, Thread>,
}

pub fn path() -> PathBuf {
    paths::home().join("threads.json")
}

/// Every thread, by id.
pub fn load() -> Result<BTreeMap<String, Thread>> {
    Ok(read_json::<File>(&path())?.threads)
}

pub fn save(threads: &BTreeMap<String, Thread>) -> Result<()> {
    write_json(
        &path(),
        &File {
            threads: threads.clone(),
        },
        false,
    )
}

/// A new thread's id.
pub fn new_id() -> String {
    format!("t_{}", &uuid::Uuid::new_v4().simple().to_string()[..12])
}
