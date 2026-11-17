//! A host: Bun running one of the scripts, spoken to over its stdin and
//! stdout, one JSON message a line (the protocol at the top of `host.js`).
//!
//! `{id, method, params}` is answered `{id, result}` or `{id, error}`. A
//! fetch's reply streams first: `{id, event: "head", status, headers}`, then
//! `{id, event: "chunk", data}` (base64), then `{id, result: null}`; the
//! reader gives back what it took (`credit`), so the host sends no more
//! than its window ahead of it, and a reader that lets go aborts it
//! (`abort`). A message without an id is an event of the host's own.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use bytes::Bytes;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{broadcast, mpsc, oneshot};

/// What one frame costs a fetch's window besides its bytes (host.js's
/// FRAME_OVERHEAD).
const FRAME_OVERHEAD: i64 = 2048;

/// The window a fetch may run ahead of its reader.
pub const WINDOW: i64 = 512 << 10;

/// A fetch's reply as the host sends it.
enum Frame {
    Head(u16, Vec<(String, String)>),
    Chunk(Bytes),
    End,
    Failed(String),
}

enum Pending {
    Call(oneshot::Sender<Result<Value>>),
    Fetch {
        frames: mpsc::UnboundedSender<Frame>,
    },
}

type PendingMap = Arc<Mutex<HashMap<i64, Pending>>>;

pub struct Rpc {
    lines: mpsc::UnboundedSender<String>,
    pending: PendingMap,
    next: AtomicI64,
    events: broadcast::Sender<Value>,
    dead: Arc<AtomicBool>,
    pid: Option<u32>,
    child: Mutex<Option<tokio::process::Child>>,
    /// The last lines it wrote to stderr: what its plugins logged, and why
    /// it died when it did.
    stderr: Arc<Mutex<VecDeque<String>>>,
}

impl Rpc {
    /// Starts `bun run <script>` in `dir` with `env` added. Must run on the
    /// plugins' runtime, which its I/O lives on.
    pub fn start(
        bun: &Path,
        script: &Path,
        dir: &Path,
        env: &[(String, String)],
        name: &'static str,
    ) -> Result<Arc<Rpc>> {
        let mut cmd = tokio::process::Command::new(bun);
        cmd.arg("run")
            .arg(script)
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for (k, v) in env {
            cmd.env(k, v);
        }
        // In a process group of its own: what its plugins start ends with it.
        #[cfg(unix)]
        cmd.process_group(0);
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow!("starting {}: {e}", bun.display()))?;
        let pid = child.id();
        let mut stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        let stderr = child.stderr.take().expect("stderr");

        let (lines, mut queued) = mpsc::unbounded_channel::<String>();
        tokio::spawn(async move {
            while let Some(line) = queued.recv().await {
                if stdin.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
            }
        });
        let said = Arc::new(Mutex::new(VecDeque::new()));
        let kept = said.clone();
        tokio::spawn(async move {
            let mut err = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = err.next_line().await {
                tracing::debug!(target: "pansy_plugins::host", "{name}: {line}");
                let mut kept = kept.lock().unwrap_or_else(|e| e.into_inner());
                if kept.len() >= 40 {
                    kept.pop_front();
                }
                kept.push_back(line);
            }
        });
        let (events, _) = broadcast::channel(256);
        let rpc = Arc::new(Rpc {
            lines,
            pending: Arc::new(Mutex::new(HashMap::new())),
            next: AtomicI64::new(1),
            events: events.clone(),
            dead: Arc::new(AtomicBool::new(false)),
            pid,
            child: Mutex::new(Some(child)),
            stderr: said,
        });
        let (pending, dead) = (rpc.pending.clone(), rpc.dead.clone());
        tokio::spawn(async move {
            let mut out = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = out.next_line().await {
                let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                    tracing::debug!("{name} wrote what isn't JSON: {line}");
                    continue;
                };
                route(&pending, &events, msg);
            }
            dead.store(true, Ordering::SeqCst);
            for (_, p) in pending.lock().unwrap_or_else(|e| e.into_inner()).drain() {
                match p {
                    Pending::Call(tx) => {
                        let _ = tx.send(Err(anyhow!("the plugin host exited")));
                    }
                    Pending::Fetch { frames } => {
                        let _ = frames.send(Frame::Failed("the plugin host exited".into()));
                    }
                }
            }
            let _ = events.send(json!({"event": "exit"}));
        });
        Ok(rpc)
    }

    pub fn alive(&self) -> bool {
        !self.dead.load(Ordering::SeqCst)
    }

    /// What it wrote to stderr lately.
    pub fn said(&self) -> String {
        let said = self.stderr.lock().unwrap_or_else(|e| e.into_inner());
        said.iter().cloned().collect::<Vec<_>>().join("\n")
    }

    pub fn events(&self) -> broadcast::Receiver<Value> {
        self.events.subscribe()
    }

    fn send(&self, msg: &Value) {
        let _ = self.lines.send(format!("{msg}\n"));
    }

    /// A message the host doesn't answer (`credit`, `abort`).
    pub fn notify(&self, method: &str, params: Value) {
        self.send(&json!({"method": method, "params": params}));
    }

    /// Calls `method`, waiting at most `wait`. Given up on (a timeout, the
    /// caller gone), the host is told to stop it.
    pub async fn call(
        self: &Arc<Self>,
        method: &str,
        params: Value,
        wait: Duration,
    ) -> Result<Value> {
        if !self.alive() {
            bail!("{method}: the plugin host exited");
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, Pending::Call(tx));
        let mut guard = Abort {
            rpc: self.clone(),
            id,
            done: false,
        };
        self.send(&json!({"id": id, "method": method, "params": params}));
        let answer = tokio::time::timeout(wait, rx).await;
        guard.done = matches!(answer, Ok(Ok(_)));
        match answer {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => bail!("{method}: the plugin host dropped the call"),
            Err(_) => bail!(
                "{method}: the plugin host didn't answer in {}s",
                wait.as_secs_f32()
            ),
        }
    }

    /// Sends a fetch and waits for its head (at most `wait`): its status,
    /// headers and a body that credits what it hands on, and aborts the
    /// fetch when dropped before its end.
    pub async fn fetch(
        self: &Arc<Self>,
        mut params: Value,
        body: &[u8],
        wait: Duration,
    ) -> Result<(u16, Vec<(String, String)>, Body)> {
        if !self.alive() {
            bail!("the plugin host exited");
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, mut rx) = mpsc::unbounded_channel();
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, Pending::Fetch { frames: tx });
        if !body.is_empty() {
            params["body"] = json!(B64.encode(body));
        }
        params["window"] = json!(WINDOW);
        let mut guard = Abort {
            rpc: self.clone(),
            id,
            done: false,
        };
        self.send(&json!({"id": id, "method": "fetch", "params": params}));
        let (status, headers) = match tokio::time::timeout(wait, rx.recv()).await {
            Err(_) => bail!("the plugin didn't answer in {}s", wait.as_secs()),
            Ok(Some(Frame::Head(s, h))) => (s, h),
            Ok(Some(Frame::Failed(e))) => {
                guard.done = true;
                bail!("{e}")
            }
            Ok(Some(_)) => bail!("the plugin host sent a body before its head"),
            Ok(None) => bail!("the plugin host dropped the request"),
        };
        Ok((
            status,
            headers,
            Body {
                rx,
                rpc: self.clone(),
                guard: Some(guard),
            },
        ))
    }

    /// Ends the host and what it started.
    pub fn kill(&self) {
        if let Some(mut child) = self.child.lock().unwrap_or_else(|e| e.into_inner()).take() {
            #[cfg(unix)]
            if let Some(pid) = self.pid {
                // SAFETY: signals the process group the host leads.
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGKILL);
                }
            }
            let _ = child.start_kill();
        }
        self.dead.store(true, Ordering::SeqCst);
    }
}

impl Drop for Rpc {
    fn drop(&mut self) {
        self.kill();
    }
}

/// A fetch's body as it streams from the host.
pub struct Body {
    rx: mpsc::UnboundedReceiver<Frame>,
    rpc: Arc<Rpc>,
    guard: Option<Abort>,
}

impl Body {
    /// The next chunk; `None` at the end. Each chunk handed on is credited.
    pub async fn next(&mut self) -> Option<Result<Bytes, String>> {
        self.guard.as_ref()?;
        match self.rx.recv().await {
            Some(Frame::Chunk(b)) => {
                let id = self.guard.as_ref().map(|g| g.id).unwrap_or_default();
                self.rpc.notify(
                    "credit",
                    json!({"id": id, "n": b.len() as i64 + FRAME_OVERHEAD}),
                );
                Some(Ok(b))
            }
            Some(Frame::End) | None => {
                if let Some(mut g) = self.guard.take() {
                    g.done = true;
                }
                None
            }
            Some(Frame::Failed(e)) => {
                if let Some(mut g) = self.guard.take() {
                    g.done = true;
                }
                Some(Err(e))
            }
            Some(Frame::Head(..)) => Some(Err("the plugin host sent a second head".into())),
        }
    }

    /// The body as a stream of bytes.
    pub fn into_stream(
        self,
    ) -> impl futures::Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static {
        futures::stream::unfold(self, |mut body| async move {
            match body.next().await? {
                Ok(b) => Some((Ok(b), body)),
                Err(e) => Some((Err(std::io::Error::other(e)), body)),
            }
        })
    }
}

/// Stops a call or a fetch its caller let go of.
struct Abort {
    rpc: Arc<Rpc>,
    id: i64,
    done: bool,
}

impl Drop for Abort {
    fn drop(&mut self) {
        if !self.done {
            self.rpc
                .pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&self.id);
            self.rpc.notify("abort", json!({"id": self.id}));
        }
    }
}

fn route(pending: &Mutex<HashMap<i64, Pending>>, events: &broadcast::Sender<Value>, msg: Value) {
    let Some(id) = msg.get("id").and_then(Value::as_i64) else {
        if let Some(level) = msg.get("level").and_then(Value::as_str)
            && msg.get("event").and_then(Value::as_str) == Some("log")
        {
            let text = msg["message"].as_str().unwrap_or_default();
            match level {
                "error" => tracing::warn!(target: "pansy_plugins::host", "{text}"),
                _ => tracing::info!(target: "pansy_plugins::host", "{text}"),
            }
        }
        let _ = events.send(msg);
        return;
    };
    let mut pending = pending.lock().unwrap_or_else(|e| e.into_inner());
    let event = msg.get("event").and_then(Value::as_str);
    match (event, pending.get(&id)) {
        (Some("head"), Some(Pending::Fetch { frames })) => {
            let status = msg["status"].as_u64().unwrap_or(0) as u16;
            let headers = msg["headers"]
                .as_object()
                .map(|h| {
                    h.iter()
                        .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_owned()))
                        .collect()
                })
                .unwrap_or_default();
            let _ = frames.send(Frame::Head(status, headers));
        }
        (Some("chunk"), Some(Pending::Fetch { frames })) => {
            let data = B64
                .decode(msg["data"].as_str().unwrap_or(""))
                .unwrap_or_default();
            let _ = frames.send(Frame::Chunk(Bytes::from(data)));
        }
        (None, Some(_)) => {
            let error = msg
                .get("error")
                .and_then(|e| e["message"].as_str())
                .map(str::to_owned);
            match pending.remove(&id) {
                Some(Pending::Call(tx)) => {
                    let _ = tx.send(match error {
                        Some(e) => Err(anyhow!(e)),
                        None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
                    });
                }
                Some(Pending::Fetch { frames }) => {
                    let _ = frames.send(match error {
                        Some(e) => Frame::Failed(e),
                        None => Frame::End,
                    });
                }
                None => {}
            }
        }
        // An answer to a call given up on: nobody's.
        _ => {}
    }
}
