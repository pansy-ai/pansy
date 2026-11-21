//! The agents driven for another device (docs/plans/phone.md): each agent's
//! own headless protocol, turned into one stream of [`Event`]s and taking
//! the same few commands (send, steer, interrupt, answer an approval).
//!
//! An agent is started as `pansy run` starts it ([`pansy_agents::run`]): on
//! pansy's gateway and the model asked for, its config left alone. Its
//! sessions stay its own files; this crate only starts it and speaks to it.
//!
//! [`codex`] drives Codex's `app-server`. Claude Code's driver (the Agent
//! SDK on pansy's Bun) comes next; when it does, what both share becomes a
//! trait.

pub mod codex;
mod rpc;

use serde::Serialize;

/// What happened in a driven agent, as the phone is told it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Event {
    /// The agent's conversation, new or resumed: its id is what resumes it.
    #[serde(rename_all = "camelCase")]
    Thread { thread: String, model: String },
    #[serde(rename_all = "camelCase")]
    TurnStarted { turn: String },
    /// A piece of the reply as it streams.
    #[serde(rename_all = "camelCase")]
    Delta { item: String, text: String },
    /// A whole reply.
    #[serde(rename_all = "camelCase")]
    Message { item: String, text: String },
    /// Something the agent began doing: `kind` is the agent's (a command,
    /// a file change, a search, its reasoning), `title` a line for it.
    #[serde(rename_all = "camelCase")]
    ToolStarted {
        item: String,
        kind: String,
        title: String,
    },
    #[serde(rename_all = "camelCase")]
    ToolFinished {
        item: String,
        kind: String,
        title: String,
        /// `completed`, `failed`, `declined`, or the agent's word.
        status: String,
        /// What it gave back, cut short.
        #[serde(skip_serializing_if = "String::is_empty")]
        output: String,
    },
    /// The agent waits for a yes or no ([`Decision`]).
    #[serde(rename_all = "camelCase")]
    Approval {
        request: String,
        /// `command`, `fileChange` or `permissions`.
        kind: String,
        title: String,
        /// The command, the files, what is asked: as the agent gave it.
        detail: String,
        #[serde(skip_serializing_if = "String::is_empty")]
        reason: String,
    },
    /// An approval was answered (here or by the agent itself).
    #[serde(rename_all = "camelCase")]
    Resolved { request: String },
    /// `completed`, `interrupted` or `failed`.
    #[serde(rename_all = "camelCase")]
    TurnFinished {
        turn: String,
        status: String,
        #[serde(skip_serializing_if = "String::is_empty")]
        error: String,
    },
    /// The agent's process ended.
    Exited { code: Option<i32> },
}

/// An answer to an [`Event::Approval`], named as T3 Code names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// This once.
    Accept,
    /// This and the same again in this session.
    AcceptForSession,
    Decline,
    /// Decline and stop the turn.
    Cancel,
}

impl Decision {
    pub fn parse(word: &str) -> Option<Decision> {
        Some(match word {
            "accept" => Decision::Accept,
            "acceptForSession" => Decision::AcceptForSession,
            "decline" => Decision::Decline,
            "cancel" => Decision::Cancel,
            _ => return None,
        })
    }
}
