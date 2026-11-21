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

pub mod auth;
pub mod claude;
pub mod codex;
pub mod config;
pub mod git;
pub mod hub;
mod notify;
mod rpc;
pub mod server;
pub mod store;
mod workspace;

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
    /// What the user sent, so every device shows it.
    #[serde(rename_all = "camelCase")]
    User { text: String },
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
        /// A command's exit code, where the agent tells it (Codex).
        #[serde(skip_serializing_if = "Option::is_none")]
        exit: Option<i64>,
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
    /// The agent asks the user to choose (Claude Code's AskUserQuestion):
    /// `questions` as it gave them, `[{question, header, options: [{label,
    /// description}], multiSelect}]`; answered with [`Answer::Choices`].
    #[serde(rename_all = "camelCase")]
    Question {
        request: String,
        questions: serde_json::Value,
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

/// How freely the agent works, as T3 Code names the modes; an agent that
/// lacks one runs in [`Mode::Ask`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    /// Ask before commands and edits (Supervised).
    Ask,
    /// Edits go ahead, commands are asked (Auto-accept edits).
    Edits,
    /// The agent's own review decides (Auto).
    Auto,
    /// Never ask (Full access): new threads start here, as in T3.
    #[default]
    Full,
}

/// What a waiting request is answered with: a decision for an approval,
/// the choices for a question (`{question: label}`, several labels joined
/// by ", ").
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    Decision(Decision),
    Choices(serde_json::Value),
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
