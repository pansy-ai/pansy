//! Codex, driven through its `app-server`: JSON-RPC on the process's stdio.
//! Tried on Codex 0.171 (docs/plans/phone-p0.md, `spikes/remote/codex.mjs`).
//!
//! ```text
//! initialize, initialized
//! thread/start {cwd, approvalPolicy, sandbox} | thread/resume {threadId}
//! turn/start {threadId, input, model?, effort?}  → turn/started … turn/completed
//!   ← item/commandExecution/requestApproval      → {decision}
//! turn/steer {threadId, expectedTurnId, input}
//! turn/interrupt {threadId, turnId}              → turn/completed: interrupted
//! ```
//!
//! The process runs with the user's own `CODEX_HOME`, so its sessions are
//! where the user's Codex keeps them and pansy's Sessions list them.

use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use pansy_agents::run::Launch;
use serde_json::{Value, json};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, mpsc};

use crate::rpc::{Incoming, Rpc};
use crate::{Decision, Event};

/// The longest tool output an event carries.
const OUTPUT: usize = 4000;

/// Who may write where, and when to ask: Codex's own two settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Ask before commands and edits (T3's Supervised).
    Ask,
    /// Edits in the folder, commands asked (Auto-accept edits).
    Edits,
    /// Never ask (Full access).
    Full,
}

impl Mode {
    fn approval(self) -> &'static str {
        match self {
            Mode::Ask => "untrusted",
            Mode::Edits => "on-request",
            Mode::Full => "never",
        }
    }

    fn sandbox(self) -> &'static str {
        match self {
            Mode::Ask | Mode::Edits => "workspace-write",
            Mode::Full => "danger-full-access",
        }
    }
}

/// One Codex conversation and the process that holds it.
pub struct Codex {
    rpc: Arc<Rpc>,
    thread: String,
    /// The approvals waiting, by the id they were sent as: the JSON-RPC id to
    /// answer under.
    waiting: Arc<Mutex<Vec<(String, Value)>>>,
    child: Mutex<Option<Child>>,
}

impl Codex {
    /// Starts `codex app-server` as `launch` says, in `folder`, and opens a
    /// conversation there, or resumes `resume`.
    pub async fn start(
        launch: &Launch,
        folder: &Path,
        resume: Option<&str>,
        mode: Mode,
    ) -> Result<(Codex, mpsc::UnboundedReceiver<Event>)> {
        let mut child = Command::new(&launch.program)
            .arg("app-server")
            .args(&launch.args)
            .envs(launch.env.iter().map(|(k, v)| (k, v)))
            .current_dir(folder)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("starting {} app-server", launch.program.display()))?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
        let (rpc, incoming) = Rpc::new(stdout, stdin);
        let (codex, events) = Codex::open(rpc, incoming, folder, resume, mode).await?;
        *codex.child.lock().await = Some(child);
        Ok((codex, events))
    }

    /// Opens the conversation over a connection already made (a process's
    /// stdio, or a test's pipe).
    async fn open(
        rpc: Arc<Rpc>,
        incoming: mpsc::UnboundedReceiver<Incoming>,
        folder: &Path,
        resume: Option<&str>,
        mode: Mode,
    ) -> Result<(Codex, mpsc::UnboundedReceiver<Event>)> {
        let (tx, events) = mpsc::unbounded_channel();
        let waiting: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
        tokio::spawn(translate(incoming, tx.clone(), Arc::clone(&waiting)));
        rpc.request(
            "initialize",
            json!({
                "clientInfo": {"name": "pansy", "title": "pansy", "version": env!("CARGO_PKG_VERSION")},
                "capabilities": {"experimentalApi": true, "requestAttestation": false},
            }),
        )
        .await
        .context("initializing codex app-server")?;
        rpc.notify("initialized", json!({})).await?;
        let opened = match resume {
            Some(id) => {
                rpc.request(
                    "thread/resume",
                    json!({"threadId": id, "approvalPolicy": mode.approval(), "sandbox": mode.sandbox()}),
                )
                .await?
            }
            None => {
                rpc.request(
                    "thread/start",
                    json!({
                        "cwd": folder.to_string_lossy(),
                        "approvalPolicy": mode.approval(),
                        "sandbox": mode.sandbox(),
                    }),
                )
                .await?
            }
        };
        let thread = opened["thread"]["id"]
            .as_str()
            .ok_or_else(|| anyhow!("codex answered without a thread"))?
            .to_owned();
        let _ = tx.send(Event::Thread {
            thread: thread.clone(),
            model: opened["model"].as_str().unwrap_or_default().to_owned(),
        });
        Ok((
            Codex {
                rpc,
                thread,
                waiting,
                child: Mutex::new(None),
            },
            events,
        ))
    }

    /// The conversation's id: what resumes it.
    pub fn thread(&self) -> &str {
        &self.thread
    }

    /// Starts a turn with `text`; the model and effort, when given, from this
    /// turn on. The turn's id is returned.
    pub async fn send(
        &self,
        text: &str,
        model: Option<&str>,
        effort: Option<&str>,
    ) -> Result<String> {
        let mut params = json!({"threadId": self.thread, "input": input(text)});
        if let Some(model) = model {
            params["model"] = json!(model);
        }
        if let Some(effort) = effort {
            params["effort"] = json!(effort);
        }
        let started = self.rpc.request("turn/start", params).await?;
        Ok(started["turn"]["id"]
            .as_str()
            .unwrap_or_default()
            .to_owned())
    }

    /// Adds `text` to the running turn `turn`.
    pub async fn steer(&self, turn: &str, text: &str) -> Result<()> {
        self.rpc
            .request(
                "turn/steer",
                json!({"threadId": self.thread, "expectedTurnId": turn, "input": input(text)}),
            )
            .await?;
        Ok(())
    }

    pub async fn interrupt(&self, turn: &str) -> Result<()> {
        self.rpc
            .request(
                "turn/interrupt",
                json!({"threadId": self.thread, "turnId": turn}),
            )
            .await?;
        Ok(())
    }

    /// Answers the approval `request` ([`Event::Approval`]).
    pub async fn answer(&self, request: &str, decision: Decision) -> Result<()> {
        let id = {
            let mut waiting = self.waiting.lock().await;
            let at = waiting
                .iter()
                .position(|(r, _)| r == request)
                .ok_or_else(|| anyhow!("no approval {request} is waiting"))?;
            waiting.remove(at).1
        };
        let decision = match decision {
            Decision::Accept => "accept",
            Decision::AcceptForSession => "acceptForSession",
            Decision::Decline => "decline",
            Decision::Cancel => "cancel",
        };
        self.rpc.respond(id, json!({"decision": decision})).await
    }

    /// Ends the process.
    pub async fn stop(&self) {
        if let Some(mut child) = self.child.lock().await.take() {
            let _ = child.kill().await;
        }
    }
}

fn input(text: &str) -> Value {
    json!([{"type": "text", "text": text, "text_elements": []}])
}

/// Codex's messages, as [`Event`]s.
async fn translate(
    mut incoming: mpsc::UnboundedReceiver<Incoming>,
    events: mpsc::UnboundedSender<Event>,
    waiting: Arc<Mutex<Vec<(String, Value)>>>,
) {
    while let Some(m) = incoming.recv().await {
        match m {
            Incoming::Request { id, method, params } => {
                if let Some(e) = approval(&id, &method, &params) {
                    if let Event::Approval { request, .. } = &e {
                        waiting.lock().await.push((request.clone(), id));
                    }
                    let _ = events.send(e);
                }
            }
            Incoming::Notification { method, params } => {
                if let Some(e) = notification(&method, &params) {
                    if let Event::Resolved { request } = &e {
                        waiting.lock().await.retain(|(r, _)| r != request);
                    }
                    let _ = events.send(e);
                }
            }
        }
    }
    let _ = events.send(Event::Exited { code: None });
}

/// The JSON-RPC id an approval came under, as the request's name.
fn request_name(id: &Value) -> String {
    match id {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn approval(id: &Value, method: &str, p: &Value) -> Option<Event> {
    let s = |v: &Value| v.as_str().unwrap_or_default().to_owned();
    let (kind, title, detail) = match method {
        "item/commandExecution/requestApproval" => (
            "command",
            "Run a command".to_owned(),
            p["commandActions"]
                .as_array()
                .and_then(|a| a.first())
                .map(|a| s(&a["command"]))
                .filter(|c| !c.is_empty())
                .unwrap_or_else(|| s(&p["command"])),
        ),
        "item/fileChange/requestApproval" => (
            "fileChange",
            "Change files".to_owned(),
            p["grantRoot"]
                .as_str()
                .map(|r| format!("write under {r}"))
                .unwrap_or_default(),
        ),
        "item/permissions/requestApproval" => (
            "permissions",
            "More permissions".to_owned(),
            p["permissions"].to_string(),
        ),
        _ => return None,
    };
    Some(Event::Approval {
        request: request_name(id),
        kind: kind.into(),
        title,
        detail,
        reason: s(&p["reason"]),
    })
}

fn notification(method: &str, p: &Value) -> Option<Event> {
    let s = |v: &Value| v.as_str().unwrap_or_default().to_owned();
    Some(match method {
        "turn/started" => Event::TurnStarted {
            turn: s(&p["turn"]["id"]),
        },
        "item/agentMessage/delta" => Event::Delta {
            item: s(&p["itemId"]),
            text: s(&p["delta"]),
        },
        "item/started" | "item/completed" => {
            let item = &p["item"];
            let id = s(&item["id"]);
            let kind = s(&item["type"]);
            if kind == "agentMessage" {
                return (method == "item/completed").then(|| Event::Message {
                    item: id,
                    text: s(&item["text"]),
                });
            }
            if matches!(kind.as_str(), "userMessage" | "hookPrompt") {
                return None;
            }
            let title = item_title(item);
            if method == "item/started" {
                Event::ToolStarted {
                    item: id,
                    kind,
                    title,
                }
            } else {
                Event::ToolFinished {
                    item: id,
                    kind,
                    title,
                    status: item["status"].as_str().unwrap_or("completed").to_owned(),
                    output: cut(item["aggregatedOutput"].as_str().unwrap_or_default()),
                }
            }
        }
        "serverRequest/resolved" => Event::Resolved {
            request: request_name(&p["requestId"]),
        },
        "turn/completed" => Event::TurnFinished {
            turn: s(&p["turn"]["id"]),
            status: s(&p["turn"]["status"]),
            error: s(&p["turn"]["error"]["message"]),
        },
        _ => return None,
    })
}

/// A line for what an item does.
fn item_title(item: &Value) -> String {
    let s = |v: &Value| v.as_str().unwrap_or_default().to_owned();
    match item["type"].as_str().unwrap_or_default() {
        "commandExecution" => s(&item["command"]),
        "fileChange" => {
            let paths: Vec<String> = item["changes"]
                .as_array()
                .map(|c| c.iter().map(|c| s(&c["path"])).collect())
                .unwrap_or_default();
            paths.join(", ")
        }
        "mcpToolCall" => format!("{}.{}", s(&item["server"]), s(&item["tool"])),
        "dynamicToolCall" => s(&item["tool"]),
        "webSearch" => s(&item["query"]),
        "reasoning" => "Thinking".into(),
        "plan" => "Plan".into(),
        "collabAgentToolCall" => s(&item["prompt"]),
        other => other.to_owned(),
    }
}

fn cut(text: &str) -> String {
    if text.chars().count() <= OUTPUT {
        return text.to_owned();
    }
    text.chars().take(OUTPUT).collect::<String>() + "…"
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    /// A Codex app-server played by the test, over a pipe.
    #[tokio::test]
    async fn a_turn_with_an_approval_steer_and_interrupt() {
        let (ours, theirs) = tokio::io::duplex(1 << 16);
        let (our_read, our_write) = tokio::io::split(ours);
        let (their_read, mut their_write) = tokio::io::split(theirs);
        let mut lines = BufReader::new(their_read).lines();

        let server = tokio::spawn(async move {
            let mut got = Vec::new();
            let reply = async |w: &mut tokio::io::WriteHalf<tokio::io::DuplexStream>, v: Value| {
                w.write_all(format!("{v}\n").as_bytes()).await.unwrap();
            };
            while let Ok(Some(line)) = lines.next_line().await {
                let m: Value = serde_json::from_str(&line).unwrap();
                let method = m["method"].as_str().unwrap_or_default().to_owned();
                got.push(if method.is_empty() {
                    format!("answer {}", m["result"])
                } else {
                    method.clone()
                });
                let id = m["id"].clone();
                match method.as_str() {
                    "initialize" => {
                        reply(
                            &mut their_write,
                            json!({"id": id, "result": {"userAgent": "fake"}}),
                        )
                        .await
                    }
                    "thread/start" => {
                        assert_eq!(m["params"]["approvalPolicy"], "never");
                        assert_eq!(m["params"]["sandbox"], "danger-full-access");
                        reply(
                            &mut their_write,
                            json!({"id": id, "result": {"thread": {"id": "t1"}, "model": "a/b"}}),
                        )
                        .await
                    }
                    "turn/start" => {
                        assert_eq!(m["params"]["model"], "c/d");
                        reply(
                            &mut their_write,
                            json!({"id": id, "result": {"turn": {"id": "u1"}}}),
                        )
                        .await;
                        reply(
                            &mut their_write,
                            json!({"method": "turn/started", "params": {"turn": {"id": "u1"}}}),
                        )
                        .await;
                        reply(&mut their_write, json!({"id": 7, "method": "item/commandExecution/requestApproval",
                            "params": {"command": "bash -lc 'ls'", "commandActions": [{"command": "ls"}], "reason": "look"}})).await;
                    }
                    "" if m["id"] == 7 => {
                        assert_eq!(m["result"]["decision"], "acceptForSession");
                        for v in [
                            json!({"method": "serverRequest/resolved", "params": {"requestId": 7}}),
                            json!({"method": "item/started", "params": {"item": {"type": "commandExecution", "id": "c1", "command": "ls"}}}),
                            json!({"method": "item/completed", "params": {"item": {"type": "commandExecution", "id": "c1", "command": "ls", "status": "completed", "aggregatedOutput": "a\nb"}}}),
                            json!({"method": "item/agentMessage/delta", "params": {"itemId": "m1", "delta": "do"}}),
                        ] {
                            reply(&mut their_write, v).await;
                        }
                    }
                    "turn/steer" => {
                        assert_eq!(m["params"]["expectedTurnId"], "u1");
                        reply(
                            &mut their_write,
                            json!({"id": id, "result": {"turnId": "u1"}}),
                        )
                        .await
                    }
                    "turn/interrupt" => {
                        reply(&mut their_write, json!({"id": id, "result": {}})).await;
                        reply(&mut their_write, json!({"method": "turn/completed", "params": {"turn": {"id": "u1", "status": "interrupted"}}})).await;
                        break;
                    }
                    _ => {}
                }
            }
            got
        });

        let (rpc, incoming) = Rpc::new(our_read, our_write);
        let (codex, mut events) = Codex::open(rpc, incoming, Path::new("/w"), None, Mode::Full)
            .await
            .unwrap();
        assert_eq!(codex.thread(), "t1");
        assert_eq!(
            events.recv().await,
            Some(Event::Thread {
                thread: "t1".into(),
                model: "a/b".into()
            })
        );
        assert_eq!(codex.send("hi", Some("c/d"), None).await.unwrap(), "u1");
        assert_eq!(
            events.recv().await,
            Some(Event::TurnStarted { turn: "u1".into() })
        );
        let Some(Event::Approval {
            request,
            kind,
            detail,
            reason,
            ..
        }) = events.recv().await
        else {
            panic!()
        };
        assert_eq!(
            (
                request.as_str(),
                kind.as_str(),
                detail.as_str(),
                reason.as_str()
            ),
            ("7", "command", "ls", "look")
        );
        codex
            .answer(&request, Decision::AcceptForSession)
            .await
            .unwrap();
        assert!(
            codex.answer(&request, Decision::Accept).await.is_err(),
            "answered once"
        );
        assert_eq!(
            events.recv().await,
            Some(Event::Resolved {
                request: "7".into()
            })
        );
        assert!(
            matches!(events.recv().await, Some(Event::ToolStarted { kind, .. }) if kind == "commandExecution")
        );
        assert!(
            matches!(events.recv().await, Some(Event::ToolFinished { output, status, .. }) if output == "a\nb" && status == "completed")
        );
        assert_eq!(
            events.recv().await,
            Some(Event::Delta {
                item: "m1".into(),
                text: "do".into()
            })
        );
        codex.steer("u1", "and more").await.unwrap();
        codex.interrupt("u1").await.unwrap();
        assert_eq!(
            events.recv().await,
            Some(Event::TurnFinished {
                turn: "u1".into(),
                status: "interrupted".into(),
                error: String::new()
            })
        );
        let got = server.await.unwrap();
        assert_eq!(
            got,
            [
                "initialize",
                "initialized",
                "thread/start",
                "turn/start",
                "answer {\"decision\":\"acceptForSession\"}",
                "turn/steer",
                "turn/interrupt"
            ]
        );
    }

    #[test]
    fn approvals_of_each_kind_and_unknown_methods() {
        let e = approval(
            &json!("x"),
            "item/fileChange/requestApproval",
            &json!({"grantRoot": "/w"}),
        )
        .unwrap();
        assert!(
            matches!(e, Event::Approval { request, kind, detail, .. } if request == "x" && kind == "fileChange" && detail == "write under /w")
        );
        assert!(approval(&json!(1), "item/tool/requestUserInput", &json!({})).is_none());
        assert!(notification("thread/tokenUsage/updated", &json!({})).is_none());
        assert!(
            notification(
                "item/started",
                &json!({"item": {"type": "agentMessage", "id": "m"}})
            )
            .is_none()
        );
        assert!(matches!(
            notification("item/completed", &json!({"item": {"type": "agentMessage", "id": "m", "text": "done"}})),
            Some(Event::Message { text, .. }) if text == "done"
        ));
        assert_eq!(cut(&"x".repeat(OUTPUT + 5)).chars().count(), OUTPUT + 1);
    }
}
