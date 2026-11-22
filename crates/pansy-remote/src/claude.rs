//! Claude Code, driven through the Agent SDK on pansy's Bun. Tried on SDK
//! 0.3.330 and Claude Code 2.1.330 (docs/plans/phone-p0.md,
//! `spikes/remote/claude.mjs`).
//!
//! The SDK is Anthropic's and isn't shipped with pansy: [`ensure_sdk`]
//! installs it on first use. `host/claude.mjs` runs its `query()` on the
//! user's own `claude` and speaks JSON-RPC lines on its stdio, as Codex's
//! `app-server` does (`rpc.rs`):
//!
//! ```text
//! start {options}                     → {} once Claude Code answered
//! send {text, uuid, model?, effort?}  → started {uuid}, message {…} … message {type: result}
//!   ← canUseTool {tool, input, suggestions, …} → PermissionResult, resolved {request}
//! interrupt {}                        → message {type: result}, maybe ended {error, carried}
//! mode {mode}, stop {}
//! ```
//!
//! Claude Code has no steering: a steer interrupts the turn and sends anew,
//! as T3 Code does. Its turns have no ids either; ours are made here, and
//! each message carries a uuid the result names it by.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex as Shared};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use pansy_agents::run::Launch;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, Notify, mpsc};

use crate::rpc::{Incoming, Rpc};
use crate::{Decision, Event, Mode};

/// The Agent SDK installed, the one this driver was tried on.
pub const SDK_VERSION: &str = "0.3.330";

const HOST: &str = include_str!("../host/claude.mjs");

/// The longest tool output an event carries.
const OUTPUT: usize = 4000;

/// How long an interrupt waits for its turn to end.
const SETTLE: Duration = Duration::from_secs(15);

/// Claude Code's permission mode for a [`Mode`].
fn permission_mode(mode: Mode) -> &'static str {
    match mode {
        Mode::Ask => "default",
        Mode::Edits => "acceptEdits",
        Mode::Auto => "auto",
        Mode::Full => "bypassPermissions",
    }
}

/// One Claude Code session and the host that holds it.
pub struct Claude {
    rpc: Arc<Rpc>,
    thread: String,
    turns: Arc<Turns>,
    /// The approvals waiting.
    waiting: Arc<Shared<Vec<Waiting>>>,
    child: Mutex<Option<Child>>,
}

/// A turn sent and not yet finished.
struct Turn {
    id: String,
    /// What the message was sent as, and the result names.
    uuid: String,
    interrupted: bool,
}

#[derive(Default)]
struct Turns {
    open: Shared<Open>,
    /// Told whenever turns finish.
    ended: Notify,
}

#[derive(Default)]
struct Open {
    list: VecDeque<Turn>,
    next: u64,
}

impl Turns {
    fn is_open(&self, turn: &str) -> bool {
        self.open.lock().unwrap().list.iter().any(|t| t.id == turn)
    }
}

/// An approval asked, and what answering it needs.
struct Waiting {
    request: String,
    id: Value,
    input: Value,
    suggestions: Value,
}

impl Claude {
    /// Starts Claude Code as `launch` says, through the SDK's host, in
    /// `folder`, on a new session or resuming `resume`.
    pub async fn start(
        launch: &Launch,
        folder: &Path,
        resume: Option<&str>,
        mode: Mode,
    ) -> Result<(Claude, mpsc::UnboundedReceiver<Event>)> {
        let bun = pansy_plugins::bun::ensure().await?;
        let sdk = ensure_sdk().await?;
        let host = write_host()?;
        let mut child = Command::new(&bun)
            .arg(&host)
            .arg(&sdk)
            .envs(launch.env.iter().map(|(k, v)| (k, v)))
            .envs(settings_env(&launch.args))
            // The gateway's token is the only key.
            .env_remove("ANTHROPIC_API_KEY")
            .current_dir(folder)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("starting {}", bun.display()))?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::debug!("claude: {line}");
                }
            });
        }
        let (rpc, incoming) = Rpc::new(stdout, stdin);
        let options = options(launch, folder, resume, mode);
        let (claude, events) = Claude::open(rpc, incoming, options, &launch.model).await?;
        *claude.child.lock().await = Some(child);
        Ok((claude, events))
    }

    /// Opens the session over a connection already made (the host's stdio,
    /// or a test's pipe), with the SDK's `options`.
    async fn open(
        rpc: Arc<Rpc>,
        incoming: mpsc::UnboundedReceiver<Incoming>,
        options: Value,
        model: &str,
    ) -> Result<(Claude, mpsc::UnboundedReceiver<Event>)> {
        let thread = options["sessionId"]
            .as_str()
            .or(options["resume"].as_str())
            .ok_or_else(|| anyhow!("no session to open"))?
            .to_owned();
        let (tx, events) = mpsc::unbounded_channel();
        let waiting: Arc<Shared<Vec<Waiting>>> = Arc::default();
        let turns: Arc<Turns> = Arc::default();
        rpc.request("start", json!({"options": options}))
            .await
            .context("starting Claude Code")?;
        let _ = tx.send(Event::Thread {
            thread: thread.clone(),
            model: model.to_owned(),
        });
        // What the host said meanwhile waited, so the thread comes first.
        tokio::spawn(translate(
            incoming,
            tx,
            Arc::clone(&waiting),
            Arc::clone(&turns),
        ));
        Ok((
            Claude {
                rpc,
                thread,
                turns,
                waiting,
                child: Mutex::new(None),
            },
            events,
        ))
    }

    /// The session's id: what resumes it.
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
        let uuid = uuid::Uuid::new_v4().to_string();
        let turn = {
            let mut open = self.turns.open.lock().unwrap();
            open.next += 1;
            let id = format!("u{}", open.next);
            open.list.push_back(Turn {
                id: id.clone(),
                uuid: uuid.clone(),
                interrupted: false,
            });
            id
        };
        let mut params = json!({"text": text, "uuid": uuid});
        if let Some(model) = model {
            params["model"] = json!(model);
        }
        if let Some(effort) = effort {
            params["effort"] = json!(effort);
        }
        if let Err(e) = self.rpc.request("send", params).await {
            self.turns
                .open
                .lock()
                .unwrap()
                .list
                .retain(|t| t.id != turn);
            return Err(e);
        }
        Ok(turn)
    }

    /// Adds `text` to the running turn `turn`: Claude Code can't take it
    /// mid-turn, so the turn is interrupted and `text` sent as the next.
    pub async fn steer(&self, turn: &str, text: &str) -> Result<()> {
        self.interrupt(turn).await?;
        self.send(text, None, None).await?;
        Ok(())
    }

    /// Stops the turn `turn`, and waits a while for it to end.
    pub async fn interrupt(&self, turn: &str) -> Result<()> {
        {
            let mut open = self.turns.open.lock().unwrap();
            let Some(t) = open.list.iter_mut().find(|t| t.id == turn) else {
                return Ok(());
            };
            t.interrupted = true;
        }
        self.rpc.request("interrupt", json!({})).await?;
        let deadline = tokio::time::Instant::now() + SETTLE;
        loop {
            let ended = self.turns.ended.notified();
            if !self.turns.is_open(turn) {
                return Ok(());
            }
            if tokio::time::timeout_at(deadline, ended).await.is_err() {
                tracing::debug!("turn {turn} still running after its interrupt");
                return Ok(());
            }
        }
    }

    /// From now on works as `mode` says.
    pub async fn set_mode(&self, mode: Mode) -> Result<()> {
        self.rpc
            .request("mode", json!({"mode": permission_mode(mode)}))
            .await?;
        Ok(())
    }

    /// Answers the approval `request` ([`Event::Approval`]).
    pub async fn answer(&self, request: &str, decision: Decision) -> Result<()> {
        let w = {
            let mut waiting = self.waiting.lock().unwrap();
            let at = waiting
                .iter()
                .position(|w| w.request == request)
                .ok_or_else(|| anyhow!("no approval {request} is waiting"))?;
            waiting.remove(at)
        };
        if decision == Decision::Cancel
            && let Some(t) = self.turns.open.lock().unwrap().list.front_mut()
        {
            t.interrupted = true;
        }
        self.rpc
            .respond(w.id, permission(decision, w.input, w.suggestions))
            .await
    }

    /// Answers the question `request` ([`Event::Question`]) with
    /// `answers`, `{question: label}`.
    pub async fn answer_question(&self, request: &str, answers: Value) -> Result<()> {
        let w = {
            let mut waiting = self.waiting.lock().unwrap();
            let at = waiting
                .iter()
                .position(|w| w.request == request)
                .ok_or_else(|| anyhow!("no question {request} is waiting"))?;
            waiting.remove(at)
        };
        let mut input = w.input;
        input["answers"] = answers;
        self.rpc
            .respond(w.id, json!({"behavior": "allow", "updatedInput": input}))
            .await
    }

    /// Ends the host, and Claude Code with it.
    pub async fn stop(&self) {
        let _ =
            tokio::time::timeout(Duration::from_secs(2), self.rpc.request("stop", json!({}))).await;
        if let Some(mut child) = self.child.lock().await.take() {
            let _ = child.kill().await;
        }
    }
}

/// What the SDK is told for a [`Decision`].
fn permission(decision: Decision, input: Value, suggestions: Value) -> Value {
    const DECLINED: &str = "The user declined.";
    match decision {
        Decision::Accept => json!({"behavior": "allow", "updatedInput": input}),
        Decision::AcceptForSession => json!({
            "behavior": "allow",
            "updatedInput": input,
            "updatedPermissions": suggestions,
        }),
        Decision::Decline => json!({"behavior": "deny", "message": DECLINED}),
        Decision::Cancel => json!({"behavior": "deny", "message": DECLINED, "interrupt": true}),
    }
}

/// The SDK's options for a session: on the gateway as `launch` says, with
/// the user's own settings, CLAUDE.md and all.
fn options(launch: &Launch, folder: &Path, resume: Option<&str>, mode: Mode) -> Value {
    let extra = sdk_args(&launch.args);
    let mut options = json!({
        "cwd": folder.to_string_lossy(),
        "pathToClaudeCodeExecutable": launch.program.to_string_lossy(),
        "permissionMode": permission_mode(mode),
        // So a session may be put in Full later; the mode decides.
        "allowDangerouslySkipPermissions": true,
        "settingSources": ["user", "project", "local"],
    });
    if !launch.model.is_empty() {
        options["model"] = json!(launch.model);
    }
    if !extra.is_empty() {
        options["extraArgs"] = Value::Object(extra);
    }
    match resume {
        Some(id) => options["resume"] = json!(id),
        None => options["sessionId"] = json!(uuid::Uuid::new_v4().to_string()),
    }
    options
}

/// `pansy run`'s arguments for `claude`, as the SDK's `extraArgs`: on the
/// command line as `pansy run` gives them. `--settings` especially (the
/// gateway's address, token and models in its `env`): the SDK's own
/// `settings` option comes only after Claude Code has started, and found
/// itself signed out.
fn sdk_args(args: &[String]) -> Map<String, Value> {
    let mut extra = Map::new();
    let mut args = args.iter().peekable();
    while let Some(arg) = args.next() {
        let Some(flag) = arg.strip_prefix("--") else {
            tracing::debug!("claude: argument {arg} left out");
            continue;
        };
        let (name, value) = match flag.split_once('=') {
            Some((name, value)) => (name, Some(value.to_owned())),
            None => (flag, args.next_if(|next| !next.starts_with("--")).cloned()),
        };
        extra.insert(name.to_owned(), value.map_or(Value::Null, Value::String));
    }
    extra
}

/// The `env` of `--settings` in `args`. Claude Code started fresh (a
/// `CLAUDE_CONFIG_DIR` never signed in) finds itself signed out with the
/// gateway's token in `--settings` alone, so it is in its environment too;
/// `--settings` still wins over the user's settings.
fn settings_env(args: &[String]) -> Vec<(String, String)> {
    let settings = args
        .iter()
        .position(|a| a == "--settings")
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
        .or_else(|| args.iter().find_map(|a| a.strip_prefix("--settings=")));
    let Some(Ok(Value::Object(settings))) = settings.map(serde_json::from_str::<Value>) else {
        return Vec::new();
    };
    let Some(Value::Object(env)) = settings.get("env") else {
        return Vec::new();
    };
    env.iter()
        .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
        .collect()
}

// --- The SDK ------------------------------------------------------------------------

fn remote_dir() -> PathBuf {
    pansy_core::paths::home().join("remote")
}

/// Where the SDK is installed.
fn sdk_dir() -> PathBuf {
    remote_dir().join("claude-sdk")
}

/// The SDK's version installed in `dir`, if any.
fn installed(dir: &Path) -> Option<String> {
    let manifest = dir.join("node_modules/@anthropic-ai/claude-agent-sdk/package.json");
    let manifest: Value = serde_json::from_slice(&std::fs::read(manifest).ok()?).ok()?;
    manifest["version"].as_str().map(str::to_owned)
}

/// The folder the Agent SDK is installed in ([`SDK_VERSION`]), installed
/// now with pansy's Bun when it isn't. Without its optional packages: they
/// hold a `claude` of its own, and the user's is the one driven.
pub async fn ensure_sdk() -> Result<PathBuf> {
    static INSTALLING: Mutex<()> = Mutex::const_new(());
    let _one = INSTALLING.lock().await;
    let dir = sdk_dir();
    if installed(&dir).as_deref() == Some(SDK_VERSION) {
        return Ok(dir);
    }
    let bun = pansy_plugins::bun::ensure().await?;
    private_dir(&remote_dir())?;
    // Made beside it and renamed into place, so an install cut short
    // leaves nothing half made.
    let tmp = remote_dir().join(format!("claude-sdk.{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp)?;
    let installing = async {
        let manifest = json!({
            "name": "pansy-remote-claude",
            "private": true,
            "dependencies": {"@anthropic-ai/claude-agent-sdk": SDK_VERSION},
        });
        std::fs::write(
            tmp.join("package.json"),
            serde_json::to_vec_pretty(&manifest)?,
        )?;
        tracing::info!("installing the Agent SDK {SDK_VERSION} to drive Claude Code");
        let mut command = Command::new(&bun);
        command
            .args(["install", "--omit=optional"])
            .current_dir(&tmp)
            .env(
                "BUN_INSTALL_CACHE_DIR",
                pansy_core::paths::home().join("cache").join("bun-install"),
            )
            .stdin(Stdio::null());
        // npmmirror with the 「国内镜像」 switch on, as the plugins' installs
        if let Some(registry) = pansy_plugins::mirror::bun_registry() {
            command.env("BUN_CONFIG_REGISTRY", registry);
        }
        let out = command
            .output()
            .await
            .with_context(|| format!("running {}", bun.display()))?;
        if !out.status.success() {
            bail!(
                "installing the Agent SDK: {}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        match installed(&tmp) {
            Some(v) if v == SDK_VERSION => Ok(()),
            got => bail!("installing the Agent SDK {SDK_VERSION} gave {got:?}"),
        }
    };
    if let Err(e) = installing.await {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e);
    }
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    std::fs::rename(&tmp, &dir)?;
    Ok(dir)
}

/// `host/claude.mjs`, written where Bun can run it, named by what it holds.
fn write_host() -> Result<PathBuf> {
    let sum = Sha256::digest(HOST.as_bytes());
    let hex: String = sum[..6].iter().map(|b| format!("{b:02x}")).collect();
    let dir = remote_dir();
    let path = dir.join(format!("claude-host-{hex}.mjs"));
    if std::fs::read_to_string(&path).is_ok_and(|s| s == HOST) {
        return Ok(path);
    }
    private_dir(&dir)?;
    pansy_core::fsutil::write_atomic(&path, HOST.as_bytes(), false)?;
    Ok(path)
}

/// Makes `dir` only the user may look into: what runs from it runs as them.
fn private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

// --- What the host says, as events ---------------------------------------------------

/// The host's messages, as [`Event`]s.
async fn translate(
    mut incoming: mpsc::UnboundedReceiver<Incoming>,
    events: mpsc::UnboundedSender<Event>,
    waiting: Arc<Shared<Vec<Waiting>>>,
    turns: Arc<Turns>,
) {
    let mut reading = Reading::default();
    while let Some(m) = incoming.recv().await {
        let mut out = Vec::new();
        match m {
            Incoming::Request { id, method, params } if method == "canUseTool" => {
                let e = if params["tool"] == "AskUserQuestion" {
                    Event::Question {
                        request: request_name(&id),
                        questions: params["input"]["questions"].clone(),
                    }
                } else {
                    approval(&id, &params)
                };
                if let Event::Approval { request, .. } | Event::Question { request, .. } = &e {
                    waiting.lock().unwrap().push(Waiting {
                        request: request.clone(),
                        id,
                        input: params["input"].clone(),
                        suggestions: params["suggestions"].clone(),
                    });
                }
                out.push(e);
            }
            Incoming::Request { method, .. } => {
                tracing::debug!("claude host asked {method}");
            }
            Incoming::Notification { method, params } => {
                let mut open = turns.open.lock().unwrap();
                let before = open.list.len();
                match method.as_str() {
                    "started" => out.extend(
                        open.list
                            .iter()
                            .find(|t| params["uuid"] == t.uuid.as_str())
                            .map(|t| Event::TurnStarted { turn: t.id.clone() }),
                    ),
                    "message" => out = reading.message(&params, &mut open.list),
                    "resolved" => {
                        let request = request_name(&params["request"]);
                        waiting.lock().unwrap().retain(|w| w.request != request);
                        out.push(Event::Resolved { request });
                    }
                    "ended" => out = ended(&params, &mut open.list),
                    _ => {}
                }
                if open.list.len() < before {
                    turns.ended.notify_waiters();
                }
            }
        }
        for e in out {
            let _ = events.send(e);
        }
    }
    let _ = events.send(Event::Exited { code: None });
}

/// What a session's messages leave to remember: the tools running, the
/// reply streaming.
#[derive(Default)]
struct Reading {
    tools: HashMap<String, (String, String)>,
    /// The message the deltas are of.
    item: String,
    /// The thoughts seen, to name each its own item.
    thoughts: usize,
}

impl Reading {
    fn message(&mut self, m: &Value, open: &mut VecDeque<Turn>) -> Vec<Event> {
        let s = |v: &Value| v.as_str().unwrap_or_default().to_owned();
        // A subagent's work shows as its tool.
        if !m["parent_tool_use_id"].is_null() {
            return Vec::new();
        }
        match m["type"].as_str().unwrap_or_default() {
            "stream_event" => {
                let e = &m["event"];
                match e["type"].as_str().unwrap_or_default() {
                    "message_start" => self.item = s(&e["message"]["id"]),
                    "content_block_delta" if e["delta"]["type"] == "text_delta" => {
                        return vec![Event::Delta {
                            item: self.item.clone(),
                            text: s(&e["delta"]["text"]),
                        }];
                    }
                    _ => {}
                }
                Vec::new()
            }
            "assistant" => {
                let item = s(&m["message"]["id"]);
                let blocks = m["message"]["content"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                blocks
                    .iter()
                    .flat_map(|b| match b["type"].as_str().unwrap_or_default() {
                        "text" => vec![Event::Message {
                            item: item.clone(),
                            text: s(&b["text"]),
                        }],
                        // a thought, whole as it comes: begun and done at once
                        "thinking" if !s(&b["thinking"]).trim().is_empty() => {
                            self.thoughts += 1;
                            let id = format!("{item}:thought{}", self.thoughts);
                            vec![
                                Event::ToolStarted {
                                    item: id.clone(),
                                    kind: "reasoning".into(),
                                    title: "Thinking".into(),
                                },
                                Event::ToolFinished {
                                    item: id,
                                    kind: "reasoning".into(),
                                    title: "Thinking".into(),
                                    status: "completed".into(),
                                    output: cut(s(&b["thinking"]).trim()),
                                    exit: None,
                                },
                            ]
                        }
                        "tool_use" => {
                            let kind = s(&b["name"]);
                            let title = summary(&kind, &b["input"]);
                            self.tools
                                .insert(s(&b["id"]), (kind.clone(), title.clone()));
                            vec![Event::ToolStarted {
                                item: s(&b["id"]),
                                kind,
                                title,
                            }]
                        }
                        _ => Vec::new(),
                    })
                    .collect()
            }
            "user" => {
                let blocks = m["message"]["content"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                blocks
                    .iter()
                    .filter(|b| b["type"] == "tool_result")
                    .map(|b| {
                        let item = s(&b["tool_use_id"]);
                        let (kind, title) = self.tools.remove(&item).unwrap_or_default();
                        Event::ToolFinished {
                            item,
                            kind,
                            title,
                            status: if b["is_error"] == true {
                                "failed"
                            } else {
                                "completed"
                            }
                            .into(),
                            output: cut(&result_text(&b["content"])),
                            exit: None,
                        }
                    })
                    .collect()
            }
            "result" => finished(m, open),
            _ => Vec::new(),
        }
    }
}

/// The turns a result ends: those whose messages it names (several sent
/// close together run as one), and any before them; the oldest when it
/// names none (an older Claude Code).
fn finished(m: &Value, open: &mut VecDeque<Turn>) -> Vec<Event> {
    let mut named: Vec<&str> = m["user_message_uuids"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    named.extend(m["user_message_uuid"].as_str());
    let last = if named.is_empty() {
        if open.is_empty() {
            return Vec::new();
        }
        0
    } else {
        match open.iter().rposition(|t| named.contains(&t.uuid.as_str())) {
            Some(i) => i,
            None => return Vec::new(),
        }
    };
    let ok = m["subtype"] == "success" && m["is_error"] != true;
    let error = if ok {
        String::new()
    } else {
        let errors: Vec<&str> = m["errors"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        [
            errors.join("; "),
            m["result"].as_str().unwrap_or_default().to_owned(),
        ]
        .into_iter()
        .find(|e| !e.is_empty())
        .unwrap_or_else(|| m["subtype"].as_str().unwrap_or("error").to_owned())
    };
    open.drain(..=last)
        .map(|t| turn_finished(t, if ok { "completed" } else { "failed" }, &error))
        .collect()
}

/// A query ended: its turns end with it, but for those `carried` to the
/// next.
fn ended(p: &Value, open: &mut VecDeque<Turn>) -> Vec<Event> {
    let carried: Vec<&str> = p["carried"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let error = match p["error"].as_str().unwrap_or_default() {
        "" => "Claude Code stopped",
        e => e,
    };
    let (done, kept): (Vec<Turn>, Vec<Turn>) = open
        .drain(..)
        .partition(|t| !carried.contains(&t.uuid.as_str()));
    open.extend(kept);
    done.into_iter()
        .map(|t| turn_finished(t, "failed", error))
        .collect()
}

fn turn_finished(t: Turn, status: &str, error: &str) -> Event {
    if t.interrupted {
        return Event::TurnFinished {
            turn: t.id,
            status: "interrupted".into(),
            error: String::new(),
        };
    }
    Event::TurnFinished {
        turn: t.id,
        status: status.into(),
        error: if status == "completed" {
            String::new()
        } else {
            error.to_owned()
        },
    }
}

/// The host's id for an approval, as the request's name.
fn request_name(id: &Value) -> String {
    match id {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn approval(id: &Value, p: &Value) -> Event {
    let tool = p["tool"].as_str().unwrap_or_default();
    let input = &p["input"];
    let kind = match tool {
        "Bash" => "command",
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => "fileChange",
        other => other,
    };
    let detail = ["command", "file_path", "notebook_path", "path"]
        .iter()
        .find_map(|k| input[k].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| input.to_string());
    let title = match p["title"].as_str().unwrap_or_default() {
        "" => match kind {
            "command" => "Run a command".to_owned(),
            "fileChange" => "Change files".to_owned(),
            _ => summary(tool, input),
        },
        t => t.to_owned(),
    };
    Event::Approval {
        request: request_name(id),
        kind: kind.into(),
        title,
        detail,
        reason: p["reason"].as_str().unwrap_or_default().to_owned(),
    }
}

/// A line for what a tool call does.
fn summary(tool: &str, input: &Value) -> String {
    let key = match tool {
        "Bash" => "command",
        "Read" | "Edit" | "Write" | "MultiEdit" => "file_path",
        "NotebookEdit" => "notebook_path",
        "Grep" | "Glob" => "pattern",
        "WebFetch" => "url",
        "WebSearch" => "query",
        "Task" | "Agent" => "description",
        _ => return tool.to_owned(),
    };
    input[key]
        .as_str()
        .and_then(|v| v.lines().next())
        .filter(|v| !v.trim().is_empty())
        .unwrap_or(tool)
        .to_owned()
}

/// A tool result's text: a string, or text blocks.
fn result_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
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
    use tokio::io::{AsyncWriteExt, DuplexStream, Lines, ReadHalf, WriteHalf};

    type Their = (
        Lines<BufReader<ReadHalf<DuplexStream>>>,
        WriteHalf<DuplexStream>,
    );

    /// A pipe: our end for [`Rpc`], theirs for the host the test plays.
    fn pipe() -> (Arc<Rpc>, mpsc::UnboundedReceiver<Incoming>, Their) {
        let (ours, theirs) = tokio::io::duplex(1 << 16);
        let (our_read, our_write) = tokio::io::split(ours);
        let (their_read, their_write) = tokio::io::split(theirs);
        let (rpc, incoming) = Rpc::new(our_read, our_write);
        (
            rpc,
            incoming,
            (BufReader::new(their_read).lines(), their_write),
        )
    }

    async fn put(w: &mut WriteHalf<DuplexStream>, v: Value) {
        w.write_all(format!("{v}\n").as_bytes()).await.unwrap();
    }

    fn message(m: Value) -> Value {
        json!({"method": "message", "params": m})
    }

    fn new_session() -> Value {
        json!({"sessionId": "s1", "permissionMode": "bypassPermissions"})
    }

    /// The host's side of a turn: a stream, an approval, a tool, a reply.
    #[tokio::test]
    async fn a_turn_with_an_approval() {
        let (rpc, incoming, (mut lines, mut w)) = pipe();
        let host = tokio::spawn(async move {
            let mut got = Vec::new();
            let mut uuid = String::new();
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
                    "start" => {
                        assert_eq!(m["params"]["options"]["sessionId"], "s1");
                        put(&mut w, json!({"id": id, "result": {}})).await;
                    }
                    "send" => {
                        assert_eq!(m["params"]["text"], "hi");
                        assert_eq!(m["params"]["model"], "c/d");
                        assert_eq!(m["params"]["effort"], "high");
                        uuid = m["params"]["uuid"].as_str().unwrap().to_owned();
                        put(
                            &mut w,
                            json!({"method": "started", "params": {"uuid": uuid}}),
                        )
                        .await;
                        put(&mut w, json!({"id": id, "result": {}})).await;
                        for v in [
                            message(
                                json!({"type": "system", "subtype": "init", "session_id": "s1", "parent_tool_use_id": null}),
                            ),
                            message(
                                json!({"type": "stream_event", "parent_tool_use_id": null, "event": {"type": "message_start", "message": {"id": "m1"}}}),
                            ),
                            message(
                                json!({"type": "stream_event", "parent_tool_use_id": null, "event": {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "do"}}}),
                            ),
                            json!({"id": "a1", "method": "canUseTool", "params": {"tool": "Bash", "input": {"command": "ls", "description": "list"},
                                "suggestions": [{"type": "addRules"}], "title": "", "reason": "look"}}),
                        ] {
                            put(&mut w, v).await;
                        }
                    }
                    "" => {
                        assert_eq!(m["id"], "a1");
                        for v in [
                            json!({"method": "resolved", "params": {"request": "a1"}}),
                            message(
                                json!({"type": "assistant", "parent_tool_use_id": null, "message": {"id": "m1",
                                "content": [{"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "ls\nmore"}}]}}),
                            ),
                            message(
                                json!({"type": "user", "parent_tool_use_id": null, "message": {"role": "user",
                                "content": [{"type": "tool_result", "tool_use_id": "t1", "content": "a\nb", "is_error": false}]}}),
                            ),
                            message(
                                json!({"type": "assistant", "parent_tool_use_id": "t9", "message": {"id": "x", "content": [{"type": "text", "text": "a subagent's"}]}}),
                            ),
                            message(
                                json!({"type": "assistant", "parent_tool_use_id": null, "message": {"id": "m2", "content": [{"type": "thinking", "thinking": "**Wrapping up** now."}, {"type": "text", "text": "done"}]}}),
                            ),
                            message(
                                json!({"type": "result", "subtype": "success", "is_error": false, "result": "done", "user_message_uuids": [uuid]}),
                            ),
                        ] {
                            put(&mut w, v).await;
                        }
                        break;
                    }
                    _ => {}
                }
            }
            got
        });

        let (claude, mut events) = Claude::open(rpc, incoming, new_session(), "a/b")
            .await
            .unwrap();
        assert_eq!(claude.thread(), "s1");
        assert_eq!(
            events.recv().await,
            Some(Event::Thread {
                thread: "s1".into(),
                model: "a/b".into()
            })
        );
        assert_eq!(
            claude.send("hi", Some("c/d"), Some("high")).await.unwrap(),
            "u1"
        );
        assert_eq!(
            events.recv().await,
            Some(Event::TurnStarted { turn: "u1".into() })
        );
        assert_eq!(
            events.recv().await,
            Some(Event::Delta {
                item: "m1".into(),
                text: "do".into()
            })
        );
        assert_eq!(
            events.recv().await,
            Some(Event::Approval {
                request: "a1".into(),
                kind: "command".into(),
                title: "Run a command".into(),
                detail: "ls".into(),
                reason: "look".into(),
            })
        );
        claude
            .answer("a1", Decision::AcceptForSession)
            .await
            .unwrap();
        assert!(
            claude.answer("a1", Decision::Accept).await.is_err(),
            "answered once"
        );
        let want = [
            Event::Resolved {
                request: "a1".into(),
            },
            Event::ToolStarted {
                item: "t1".into(),
                kind: "Bash".into(),
                title: "ls".into(),
            },
            Event::ToolFinished {
                item: "t1".into(),
                kind: "Bash".into(),
                title: "ls".into(),
                status: "completed".into(),
                output: "a\nb".into(),
                exit: None,
            },
            // a thought, begun and done at once
            Event::ToolStarted {
                item: "m2:thought1".into(),
                kind: "reasoning".into(),
                title: "Thinking".into(),
            },
            Event::ToolFinished {
                item: "m2:thought1".into(),
                kind: "reasoning".into(),
                title: "Thinking".into(),
                status: "completed".into(),
                output: "**Wrapping up** now.".into(),
                exit: None,
            },
            Event::Message {
                item: "m2".into(),
                text: "done".into(),
            },
            Event::TurnFinished {
                turn: "u1".into(),
                status: "completed".into(),
                error: String::new(),
            },
        ];
        for e in want {
            assert_eq!(events.recv().await, Some(e));
        }
        let got = host.await.unwrap();
        assert_eq!(
            got,
            [
                "start",
                "send",
                "answer {\"behavior\":\"allow\",\"updatedInput\":{\"command\":\"ls\",\"description\":\"list\"},\"updatedPermissions\":[{\"type\":\"addRules\"}]}"
            ]
        );
        assert_eq!(events.recv().await, Some(Event::Exited { code: None }));
    }

    /// AskUserQuestion is a question, answered with the choices made.
    #[tokio::test]
    async fn a_question_answered_with_choices() {
        let (rpc, incoming, (mut lines, mut w)) = pipe();
        let host = tokio::spawn(async move {
            while let Ok(Some(line)) = lines.next_line().await {
                let m: Value = serde_json::from_str(&line).unwrap();
                match m["method"].as_str() {
                    Some("start") => {
                        put(&mut w, json!({"id": m["id"], "result": {}})).await;
                        put(&mut w, json!({"id": "q1", "method": "canUseTool", "params": {"tool": "AskUserQuestion",
                            "input": {"questions": [{"question": "Which?", "header": "Pick", "multiSelect": false,
                                "options": [{"label": "A", "description": "a"}, {"label": "B", "description": "b"}]}]},
                            "suggestions": []}}))
                        .await;
                    }
                    None => return m["result"].clone(),
                    _ => {}
                }
            }
            Value::Null
        });
        let (claude, mut events) = Claude::open(rpc, incoming, new_session(), "")
            .await
            .unwrap();
        events.recv().await;
        let Some(Event::Question { request, questions }) = events.recv().await else {
            panic!()
        };
        assert_eq!(questions[0]["options"][1]["label"], "B");
        claude
            .answer_question(&request, json!({"Which?": "B"}))
            .await
            .unwrap();
        let result = host.await.unwrap();
        assert_eq!(result["behavior"], "allow");
        assert_eq!(result["updatedInput"]["answers"], json!({"Which?": "B"}));
        assert_eq!(result["updatedInput"]["questions"][0]["question"], "Which?");
    }

    /// Each answer, as the SDK is told it.
    #[tokio::test]
    async fn every_decision_answered() {
        let (rpc, incoming, (mut lines, mut w)) = pipe();
        let host = tokio::spawn(async move {
            let mut answers = Vec::new();
            while let Ok(Some(line)) = lines.next_line().await {
                let m: Value = serde_json::from_str(&line).unwrap();
                match m["method"].as_str() {
                    Some("start") => {
                        put(&mut w, json!({"id": m["id"], "result": {}})).await;
                        for (i, tool) in ["Write", "Bash", "WebFetch", "Edit"].iter().enumerate() {
                            put(&mut w, json!({"id": format!("a{i}"), "method": "canUseTool",
                                "params": {"tool": tool, "input": {"file_path": "/w/x"}, "suggestions": []}}))
                            .await;
                        }
                    }
                    None => {
                        answers.push((m["id"].as_str().unwrap().to_owned(), m["result"].clone()));
                        if answers.len() == 4 {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            answers
        });
        let (claude, mut events) = Claude::open(rpc, incoming, new_session(), "")
            .await
            .unwrap();
        events.recv().await;
        let mut kinds = Vec::new();
        for _ in 0..4 {
            let Some(Event::Approval { kind, detail, .. }) = events.recv().await else {
                panic!()
            };
            assert_eq!(detail, "/w/x");
            kinds.push(kind);
        }
        assert_eq!(kinds, ["fileChange", "command", "WebFetch", "fileChange"]);
        for (request, decision) in [
            ("a0", Decision::Accept),
            ("a1", Decision::AcceptForSession),
            ("a2", Decision::Decline),
            ("a3", Decision::Cancel),
        ] {
            claude.answer(request, decision).await.unwrap();
        }
        let answers = host.await.unwrap();
        let input = json!({"file_path": "/w/x"});
        assert_eq!(
            answers,
            [
                (
                    "a0".to_owned(),
                    json!({"behavior": "allow", "updatedInput": input})
                ),
                (
                    "a1".to_owned(),
                    json!({"behavior": "allow", "updatedInput": input, "updatedPermissions": []})
                ),
                (
                    "a2".to_owned(),
                    json!({"behavior": "deny", "message": "The user declined."})
                ),
                (
                    "a3".to_owned(),
                    json!({"behavior": "deny", "message": "The user declined.", "interrupt": true})
                ),
            ]
        );
    }

    /// A steer interrupts the turn, waits for it to end, and sends again;
    /// the query that ends after an interrupt ends nothing more.
    #[tokio::test]
    async fn steer_is_interrupt_then_send() {
        let (rpc, incoming, (mut lines, mut w)) = pipe();
        let host = tokio::spawn(async move {
            let mut got = Vec::new();
            let mut uuids = Vec::new();
            while let Ok(Some(line)) = lines.next_line().await {
                let m: Value = serde_json::from_str(&line).unwrap();
                let method = m["method"].as_str().unwrap_or_default().to_owned();
                got.push(method.clone());
                let id = m["id"].clone();
                match method.as_str() {
                    "start" => {
                        assert_eq!(m["params"]["options"]["resume"], "s0");
                        put(&mut w, json!({"id": id, "result": {}})).await;
                    }
                    "send" => {
                        let uuid = m["params"]["uuid"].as_str().unwrap().to_owned();
                        uuids.push(uuid.clone());
                        put(
                            &mut w,
                            json!({"method": "started", "params": {"uuid": uuid}}),
                        )
                        .await;
                        put(&mut w, json!({"id": id, "result": {}})).await;
                        if uuids.len() == 2 {
                            assert_eq!(m["params"]["text"], "and more");
                            // An older Claude Code: the result names no message.
                            put(&mut w, message(json!({"type": "result", "subtype": "success", "is_error": false}))).await;
                            break;
                        }
                    }
                    "interrupt" => {
                        put(&mut w, json!({"id": id, "result": {}})).await;
                        put(
                            &mut w,
                            message(
                                json!({"type": "result", "subtype": "error_during_execution",
                            "is_error": true, "errors": [], "user_message_uuids": [uuids[0]]}),
                            ),
                        )
                        .await;
                        put(&mut w, json!({"method": "ended", "params": {"error": "Claude Code returned an error result", "carried": []}})).await;
                    }
                    _ => {}
                }
            }
            got
        });
        let options = json!({"resume": "s0"});
        let (claude, mut events) = Claude::open(rpc, incoming, options, "a/b").await.unwrap();
        assert_eq!(claude.thread(), "s0");
        events.recv().await;
        let turn = claude.send("count", None, None).await.unwrap();
        assert_eq!(
            events.recv().await,
            Some(Event::TurnStarted { turn: "u1".into() })
        );
        claude.steer(&turn, "and more").await.unwrap();
        let want = [
            Event::TurnFinished {
                turn: "u1".into(),
                status: "interrupted".into(),
                error: String::new(),
            },
            Event::TurnStarted { turn: "u2".into() },
            Event::TurnFinished {
                turn: "u2".into(),
                status: "completed".into(),
                error: String::new(),
            },
        ];
        for e in want {
            assert_eq!(events.recv().await, Some(e));
        }
        assert_eq!(host.await.unwrap(), ["start", "send", "interrupt", "send"]);
        // A turn already over has nothing to interrupt.
        claude.interrupt("u1").await.unwrap();
    }

    #[test]
    fn results_and_ends_finish_the_turns_they_name() {
        let turn = |id: &str, uuid: &str| Turn {
            id: id.into(),
            uuid: uuid.into(),
            interrupted: false,
        };
        let mut open: VecDeque<Turn> =
            [turn("u1", "x1"), turn("u2", "x2"), turn("u3", "x3")].into();
        // Two sent close together ran as one, and it failed.
        let e = finished(
            &json!({"type": "result", "subtype": "success", "is_error": true, "result": "502 upstream", "user_message_uuids": ["x2"]}),
            &mut open,
        );
        assert_eq!(
            e,
            ["u1", "u2"].map(|t| Event::TurnFinished {
                turn: t.into(),
                status: "failed".into(),
                error: "502 upstream".into()
            })
        );
        assert!(finished(&json!({"user_message_uuids": ["nobody"]}), &mut open).is_empty());
        open.push_back(turn("u4", "x4"));
        let e = ended(&json!({"error": "", "carried": ["x4"]}), &mut open);
        assert_eq!(
            e,
            [Event::TurnFinished {
                turn: "u3".into(),
                status: "failed".into(),
                error: "Claude Code stopped".into()
            }]
        );
        assert_eq!(open.len(), 1);
        assert_eq!(summary("Grep", &json!({"pattern": "fn main"})), "fn main");
        assert_eq!(summary("TodoWrite", &json!({})), "TodoWrite");
        assert_eq!(
            result_text(
                &json!([{"type": "text", "text": "a"}, {"type": "image"}, {"type": "text", "text": "b"}])
            ),
            "a\nb"
        );
        assert_eq!(cut(&"x".repeat(OUTPUT + 5)).chars().count(), OUTPUT + 1);
    }

    #[test]
    fn the_gateway_settings_go_to_the_sdk() {
        let settings = r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:7267"}}"#;
        let launch = Launch {
            agent: "claude-code",
            name: "Claude Code",
            program: PathBuf::from("/bin/claude"),
            args: vec![
                "--settings".into(),
                settings.into(),
                "--verbose".into(),
                "--add-dir=/x".into(),
            ],
            env: Vec::new(),
            model: "a/b".into(),
            small: None,
            subagent: None,
            source: pansy_agents::run::Source::Connected,
            project: PathBuf::from("/w"),
            verified: None,
            note: None,
        };
        let o = options(&launch, Path::new("/w"), None, Mode::Edits);
        assert_eq!(
            o["extraArgs"],
            json!({"settings": settings, "verbose": null, "add-dir": "/x"})
        );
        assert!(o.get("settings").is_none());
        assert_eq!(
            settings_env(&launch.args),
            [(
                "ANTHROPIC_BASE_URL".to_owned(),
                "http://127.0.0.1:7267".to_owned()
            )]
        );
        assert!(settings_env(&["--settings".into(), "/a/file.json".into()]).is_empty());
        assert_eq!(o["permissionMode"], "acceptEdits");
        assert_eq!(o["pathToClaudeCodeExecutable"], "/bin/claude");
        assert_eq!(o["model"], "a/b");
        assert!(o["sessionId"].as_str().is_some_and(|s| s.len() == 36));
        assert!(o.get("resume").is_none());
        let o = options(&launch, Path::new("/w"), Some("s1"), Mode::Full);
        assert_eq!(
            (o["resume"].as_str(), o.get("sessionId")),
            (Some("s1"), None)
        );
        assert_eq!(o["permissionMode"], "bypassPermissions");
        assert_eq!(permission_mode(Mode::Ask), "default");
        assert_eq!(permission_mode(Mode::Auto), "auto");
    }
}
