//! P0 of docs/plans/plugin-host.md: magpie's plugin host (host.js on Bun),
//! driven from Rust over its stdio protocol, one JSON message a line. It
//! starts a made-up vendor on loopback, loads magpie's fake plugin, signs
//! in with a key and sends requests through the plugin's fetch, each reply
//! read as the `reqwest::Response` pansy's gateway reads, and checks what
//! the plan depends on: init, providers, apiKey, load, fetch (whole and
//! streamed), the credit window, abort reaching the vendor, requests at
//! once, and a host that dies under a request.
//!
//! cargo run -- <bun> <scratch>   (run.sh sets the scratch directory up)

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail, ensure};
use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::post;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use bytes::Bytes;
use futures::StreamExt;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot};

/// What one frame costs the credit window besides its bytes (host.js's
/// FRAME_OVERHEAD).
const FRAME_OVERHEAD: i64 = 2048;

// --- The host ---------------------------------------------------------------

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
        /// Charged bytes the host has sent and the reader not yet credited.
        owed: Arc<AtomicI64>,
        most: Arc<AtomicI64>,
    },
}

struct Host {
    child: Mutex<Option<Child>>,
    lines: mpsc::UnboundedSender<String>,
    pending: Arc<Mutex<HashMap<i64, Pending>>>,
    next: AtomicI64,
    events: Arc<Mutex<Vec<Value>>>,
    /// The host's output ended: it is gone.
    dead: Arc<AtomicBool>,
}

impl Host {
    async fn start(
        bun: &Path,
        script: &Path,
        home: &Path,
        env: &[(&str, String)],
    ) -> Result<Arc<Host>> {
        let mut cmd = Command::new(bun);
        cmd.arg("run")
            .arg(script)
            .current_dir(home)
            .env("HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().context("starting bun")?;
        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        let (lines, mut queued) = mpsc::unbounded_channel::<String>();
        tokio::spawn(async move {
            while let Some(line) = queued.recv().await {
                if stdin.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
            }
        });
        let log = home.join("host-stderr.txt");
        tokio::spawn(async move {
            let mut out = tokio::fs::File::create(log).await.unwrap();
            let mut err = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = err.next_line().await {
                let _ = out.write_all(format!("{line}\n").as_bytes()).await;
            }
        });

        let host = Arc::new(Host {
            child: Mutex::new(Some(child)),
            lines,
            pending: Arc::new(Mutex::new(HashMap::new())),
            next: AtomicI64::new(1),
            events: Arc::new(Mutex::new(Vec::new())),
            dead: Arc::new(AtomicBool::new(false)),
        });
        let (pending, events, dead) =
            (host.pending.clone(), host.events.clone(), host.dead.clone());
        tokio::spawn(async move {
            let mut out = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = out.next_line().await {
                let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                    eprintln!("  host wrote what isn't JSON: {line}");
                    continue;
                };
                route(&pending, &events, msg);
            }
            // The host is gone: every call still waiting fails now.
            dead.store(true, Ordering::SeqCst);
            for (_, p) in pending.lock().unwrap().drain() {
                match p {
                    Pending::Call(tx) => {
                        let _ = tx.send(Err(anyhow!("the host exited")));
                    }
                    Pending::Fetch { frames, .. } => {
                        let _ = frames.send(Frame::Failed("the host exited".into()));
                    }
                }
            }
        });
        Ok(host)
    }

    fn send(&self, msg: Value) {
        let _ = self.lines.send(format!("{msg}\n"));
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value> {
        ensure!(
            !self.dead.load(Ordering::SeqCst),
            "{method}: the host exited"
        );
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, Pending::Call(tx));
        self.send(json!({"id": id, "method": method, "params": params}));
        match tokio::time::timeout(Duration::from_secs(60), rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => bail!("{method}: the host dropped the call"),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                self.send(json!({"method": "abort", "params": {"id": id}}));
                bail!("{method}: no answer in 60s")
            }
        }
    }

    /// Sends a fetch and reads its head; the body is a `reqwest::Response`
    /// that credits what it hands on and aborts the fetch when dropped
    /// before its end.
    async fn fetch(
        self: &Arc<Self>,
        mut params: Value,
        body: &[u8],
        window: i64,
    ) -> Result<(reqwest::Response, Arc<AtomicI64>)> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let owed = Arc::new(AtomicI64::new(0));
        let most = Arc::new(AtomicI64::new(0));
        self.pending.lock().unwrap().insert(
            id,
            Pending::Fetch {
                frames: tx,
                owed: owed.clone(),
                most: most.clone(),
            },
        );
        params["body"] = json!(B64.encode(body));
        params["window"] = json!(window);
        self.send(json!({"id": id, "method": "fetch", "params": params}));

        let mut guard = Abort {
            host: self.clone(),
            id,
            done: false,
        };
        let (status, headers) = match rx.recv().await {
            Some(Frame::Head(s, h)) => (s, h),
            Some(Frame::Failed(e)) => {
                guard.done = true;
                bail!("fetch failed before its head: {e}")
            }
            Some(_) => bail!("fetch: a body before its head"),
            None => bail!("fetch: the host dropped it"),
        };
        let host = self.clone();
        let stream = async_stream(rx, host, id, owed, guard);
        let mut response = http::Response::new(reqwest::Body::wrap_stream(stream));
        *response.status_mut() = http::StatusCode::from_u16(status)?;
        for (k, v) in headers {
            response.headers_mut().append(
                http::HeaderName::from_bytes(k.as_bytes())?,
                http::HeaderValue::from_str(&v)?,
            );
        }
        Ok((reqwest::Response::from(response), most))
    }

    fn kill(&self) {
        if let Some(mut c) = self.child.lock().unwrap().take() {
            let _ = c.start_kill();
        }
    }
}

/// The body of a fetch as a stream of bytes: each chunk handed on is
/// credited back; the end or an error ends it.
fn async_stream(
    rx: mpsc::UnboundedReceiver<Frame>,
    host: Arc<Host>,
    id: i64,
    owed: Arc<AtomicI64>,
    guard: Abort,
) -> impl futures::Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static {
    futures::stream::unfold(
        (rx, host, owed, guard),
        move |(mut rx, host, owed, mut guard)| async move {
            match rx.recv().await {
                Some(Frame::Chunk(b)) => {
                    let n = b.len() as i64 + FRAME_OVERHEAD;
                    owed.fetch_sub(n, Ordering::Relaxed);
                    host.send(json!({"method": "credit", "params": {"id": id, "n": n}}));
                    Some((Ok(b), (rx, host, owed, guard)))
                }
                Some(Frame::End) | None => {
                    guard.done = true;
                    drop(guard);
                    None
                }
                Some(Frame::Failed(e)) => {
                    guard.done = true;
                    Some((Err(std::io::Error::other(e)), (rx, host, owed, guard)))
                }
                Some(Frame::Head(..)) => Some((
                    Err(std::io::Error::other("a second head")),
                    (rx, host, owed, guard),
                )),
            }
        },
    )
}

/// Aborts a fetch whose reader let it go before its end.
struct Abort {
    host: Arc<Host>,
    id: i64,
    done: bool,
}

impl Drop for Abort {
    fn drop(&mut self) {
        if !self.done {
            self.host.pending.lock().unwrap().remove(&self.id);
            self.host
                .send(json!({"method": "abort", "params": {"id": self.id}}));
        }
    }
}

fn route(pending: &Mutex<HashMap<i64, Pending>>, events: &Mutex<Vec<Value>>, msg: Value) {
    let Some(id) = msg.get("id").and_then(Value::as_i64) else {
        events.lock().unwrap().push(msg);
        return;
    };
    let mut pending = pending.lock().unwrap();
    let event = msg.get("event").and_then(Value::as_str);
    match (event, pending.get(&id)) {
        (Some("head"), Some(Pending::Fetch { frames, .. })) => {
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
        (Some("chunk"), Some(Pending::Fetch { frames, owed, most })) => {
            let data = B64
                .decode(msg["data"].as_str().unwrap_or(""))
                .unwrap_or_default();
            let now = owed.fetch_add(data.len() as i64 + FRAME_OVERHEAD, Ordering::Relaxed)
                + data.len() as i64
                + FRAME_OVERHEAD;
            most.fetch_max(now, Ordering::Relaxed);
            let _ = frames.send(Frame::Chunk(Bytes::from(data)));
        }
        (None, Some(_)) => {
            let error = msg
                .get("error")
                .and_then(|e| e["message"].as_str())
                .map(str::to_owned);
            match pending.remove(&id).unwrap() {
                Pending::Call(tx) => {
                    let _ = tx.send(match error {
                        Some(e) => Err(anyhow!(e)),
                        None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
                    });
                }
                Pending::Fetch { frames, .. } => {
                    let _ = frames.send(match error {
                        Some(e) => Frame::Failed(e),
                        None => Frame::End,
                    });
                }
            }
        }
        // An answer to a call given up on: nobody's.
        _ => {}
    }
}

// --- A made-up vendor ---------------------------------------------------------

#[derive(Default)]
struct Vendor {
    /// The authorization header of each request, as the vendor got it.
    seen: Mutex<Vec<String>>,
    /// Streams the vendor is still writing.
    open: AtomicUsize,
    sent: AtomicUsize,
}

struct Open(Arc<Vendor>);
impl Drop for Open {
    fn drop(&mut self) {
        self.0.open.fetch_sub(1, Ordering::SeqCst);
    }
}

/// `/v1/chat/completions`: `{"stream": false}` answers JSON; a stream sends
/// `chunks` events of `size` bytes, or never ends with `"endless": true`.
async fn chat(
    State(v): State<Arc<Vendor>>,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    let auth = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_owned();
    v.seen.lock().unwrap().push(auth);
    let req: Value = serde_json::from_slice(&body).unwrap_or_default();
    if !req["stream"].as_bool().unwrap_or(false) {
        let reply = json!({"id": "x", "object": "chat.completion", "choices": [{"index": 0, "message": {"role": "assistant", "content": "hello from the vendor"}, "finish_reason": "stop"}]});
        return axum::response::Response::builder()
            .header("content-type", "application/json")
            .body(Body::from(reply.to_string()))
            .unwrap();
    }
    let chunks = req["chunks"].as_u64().unwrap_or(10) as usize;
    let size = req["size"].as_u64().unwrap_or(100) as usize;
    let endless = req["endless"].as_bool().unwrap_or(false);
    v.open.fetch_add(1, Ordering::SeqCst);
    let open = Open(v.clone());
    let line = format!(
        "data: {{\"pad\":\"{}\"}}\n\n",
        "x".repeat(size.saturating_sub(14))
    );
    let stream = futures::stream::unfold((0usize, open), move |(i, open)| {
        let line = line.clone();
        async move {
            if !endless && i >= chunks {
                return None;
            }
            if endless {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            open.0.sent.fetch_add(line.len(), Ordering::Relaxed);
            let out = if !endless && i + 1 == chunks {
                format!("{line}data: [DONE]\n\n")
            } else {
                line
            };
            Some((Ok::<_, std::io::Error>(Bytes::from(out)), (i + 1, open)))
        }
    });
    axum::response::Response::builder()
        .header("content-type", "text/event-stream")
        .body(Body::from_stream(stream))
        .unwrap()
}

// --- The checks ---------------------------------------------------------------

/// The protocol a model's AI SDK package says pansy speaks to it in.
fn protocol(npm: &str) -> &'static str {
    match npm {
        "@ai-sdk/openai-compatible" => "chat",
        "@ai-sdk/openai" => "responses",
        "@ai-sdk/anthropic" => "anthropic",
        "@ai-sdk/google" => "gemini",
        _ => "unsupported",
    }
}

fn ok(what: &str, since: Instant) {
    println!("  ok  {what} ({} ms)", since.elapsed().as_millis());
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    ensure!(args.len() == 3, "usage: plugin-host-spike <bun> <scratch>");
    let bun = PathBuf::from(&args[1]);
    let scratch = PathBuf::from(&args[2]);
    let home = scratch.join("home");
    let _ = std::fs::remove_file(home.join("plugin-auth.json"));
    let script = scratch.join("host/host.js");
    let plugin = scratch.join("plugins/fake");

    let vendor = Arc::new(Vendor::default());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}/v1", listener.local_addr()?);
    let app = Router::new()
        .route("/v1/chat/completions", post(chat))
        .with_state(vendor.clone());
    tokio::spawn(async move { axum::serve(listener, app).await });

    println!("P0: magpie's plugin host from Rust ({})", bun.display());
    let t = Instant::now();
    let host = Host::start(&bun, &script, &home, &[("FAKE_BASE", base.clone())]).await?;
    let auth_path = home.join("plugin-auth.json");
    let init = host
        .call(
            "init",
            json!({
                "authPath": auth_path,
                "directory": home,
                "plugins": [{"spec": plugin, "target": plugin}],
            }),
        )
        .await?;
    ensure!(
        init["plugins"][0]["provides"] == json!(["fakeco"]),
        "init: {init}"
    );
    ok("init: the fake plugin loaded, provides fakeco", t);

    let t = Instant::now();
    let providers = host.call("providers", json!({})).await?;
    let fake = &providers[0];
    ensure!(fake["id"] == "fakeco", "providers: {providers}");
    let models: Vec<String> = fake["models"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            format!(
                "{} → {}",
                m["id"].as_str().unwrap(),
                protocol(m["npm"].as_str().unwrap())
            )
        })
        .collect();
    ok(
        &format!(
            "providers: fakeco, methods {}, models [{}]",
            fake["methods"],
            models.join(", ")
        ),
        t,
    );

    let t = Instant::now();
    let signed = host
        .call(
            "apiKey",
            json!({"provider": "fakeco", "method": 0, "key": "sk-spike"}),
        )
        .await?;
    let account = signed["account"]
        .as_str()
        .context("apiKey gave no account")?
        .to_owned();
    ensure!(signed["ok"] == true, "apiKey: {signed}");
    let mode = std::fs::metadata(&auth_path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            mode.permissions().mode() & 0o777 == 0o600,
            "plugin-auth.json is {:o}",
            mode.permissions().mode()
        );
    }
    ok(
        &format!("apiKey: signed in as account {account:?}; plugin-auth.json is 0600"),
        t,
    );

    let t = Instant::now();
    let loaded = host
        .call("load", json!({"provider": "fakeco", "account": account}))
        .await?;
    ensure!(loaded["baseURL"] == base.as_str(), "load: {loaded}");
    ok(
        &format!(
            "load: the loader's baseURL is the vendor's, fetch hook {}",
            loaded["fetch"]
        ),
        t,
    );

    let url = format!("{base}/chat/completions");
    let fetch_params = |stream: bool| {
        json!({
            "provider": "fakeco", "account": account, "model": "fake-1",
            "npm": "@ai-sdk/openai-compatible", "url": url, "method": "POST",
            "headers": {"content-type": "application/json"},
            "session": if stream { "s-1" } else { "" },
        })
    };

    // A whole reply, read as the gateway reads one.
    let t = Instant::now();
    let body = json!({"model": "fake-1", "stream": false, "messages": [{"role": "user", "content": "hi"}]});
    let (response, _) = host
        .fetch(fetch_params(false), body.to_string().as_bytes(), 512 << 10)
        .await?;
    ensure!(response.status() == 200, "status {}", response.status());
    let reply: Value = response.json().await?;
    ensure!(
        reply["choices"][0]["message"]["content"] == "hello from the vendor",
        "{reply}"
    );
    let seen = vendor
        .seen
        .lock()
        .unwrap()
        .last()
        .cloned()
        .unwrap_or_default();
    ensure!(seen == "Bearer sk-spike", "the vendor saw {seen:?}");
    ok(
        "fetch, whole: 200, the vendor's JSON, signed by the plugin (Bearer sk-spike)",
        t,
    );

    // Overhead of a small request through the host.
    let t = Instant::now();
    let rounds = 50;
    for _ in 0..rounds {
        let (r, _) = host
            .fetch(fetch_params(false), body.to_string().as_bytes(), 512 << 10)
            .await?;
        r.bytes().await?;
    }
    println!(
        "  ..  {rounds} small requests in a row: {:.2} ms each",
        t.elapsed().as_secs_f64() * 1000.0 / rounds as f64
    );

    // A long stream to a slow reader: the host stays within the window.
    let t = Instant::now();
    let window = 128 << 10;
    let body = json!({"model": "fake-1", "stream": true, "chunks": 2048, "size": 2048});
    let (response, most) = host
        .fetch(fetch_params(true), body.to_string().as_bytes(), window)
        .await?;
    let mut bytes = response.bytes_stream();
    let (mut got, mut n) = (0usize, 0usize);
    while let Some(chunk) = bytes.next().await {
        got += chunk?.len();
        n += 1;
        if n % 8 == 0 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }
    let sent = vendor.sent.swap(0, Ordering::Relaxed) + "data: [DONE]\n\n".len();
    ensure!(got == sent, "read {got} of {sent} bytes");
    let most = most.load(Ordering::Relaxed);
    ensure!(
        most <= window,
        "the host ran {most} past a window of {window}"
    );
    let secs = t.elapsed().as_secs_f64();
    ok(
        &format!(
            "fetch, streamed: {:.1} MiB to a slow reader, {:.0} MiB/s, at most {} KiB owed of a {} KiB window",
            got as f64 / 1048576.0,
            got as f64 / 1048576.0 / secs,
            most >> 10,
            window >> 10
        ),
        t,
    );

    // Letting a stream go aborts the plugin's fetch, and the vendor's.
    let t = Instant::now();
    let body = json!({"model": "fake-1", "stream": true, "endless": true, "size": 256});
    let (response, _) = host
        .fetch(fetch_params(true), body.to_string().as_bytes(), 512 << 10)
        .await?;
    let mut bytes = response.bytes_stream();
    for _ in 0..10 {
        bytes.next().await.context("the endless stream ended")??;
    }
    ensure!(
        vendor.open.load(Ordering::SeqCst) == 1,
        "the vendor isn't streaming"
    );
    drop(bytes);
    let gone = Instant::now();
    while vendor.open.load(Ordering::SeqCst) != 0 {
        ensure!(
            gone.elapsed() < Duration::from_secs(5),
            "the vendor still streams 5s after the abort"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    ok(
        &format!(
            "abort: the vendor's stream closed {} ms after the reader let go",
            gone.elapsed().as_millis()
        ),
        t,
    );

    // Requests at once.
    let t = Instant::now();
    let tasks: Vec<_> = (0..8)
        .map(|_| {
            let host = host.clone();
            let p = fetch_params(true);
            tokio::spawn(async move {
                let body = json!({"model": "fake-1", "stream": true, "chunks": 256, "size": 2048});
                let (r, _) = host
                    .fetch(p, body.to_string().as_bytes(), 128 << 10)
                    .await?;
                Ok::<_, anyhow::Error>(r.bytes().await?.len())
            })
        })
        .collect();
    let mut total = 0;
    for task in tasks {
        let n = task.await??;
        ensure!(
            n == 256 * (2048 + 4) + "data: [DONE]\n\n".len(),
            "a stream read {n} bytes"
        );
        total += n;
    }
    ok(
        &format!(
            "8 streams at once: {:.1} MiB, every byte",
            total as f64 / 1048576.0
        ),
        t,
    );

    // Events the host sent unasked.
    let events = host.events.lock().unwrap().clone();
    let mut kinds: Vec<String> = events
        .iter()
        .filter_map(|e| e["event"].as_str().map(str::to_owned))
        .collect();
    kinds.sort();
    kinds.dedup();
    println!("  ..  events the host sent unasked: {kinds:?}");

    // A host that dies under a request fails it, at once.
    let t = Instant::now();
    let body = json!({"model": "fake-1", "stream": true, "endless": true, "size": 256});
    let (response, _) = host
        .fetch(fetch_params(true), body.to_string().as_bytes(), 512 << 10)
        .await?;
    let mut bytes = response.bytes_stream();
    bytes.next().await.context("no first chunk")??;
    host.kill();
    let died = Instant::now();
    let mut failed = false;
    while let Some(chunk) = tokio::time::timeout(Duration::from_secs(5), bytes.next()).await? {
        if chunk.is_err() {
            failed = true;
            break;
        }
    }
    ensure!(failed, "the stream ended cleanly though the host died");
    ok(
        &format!(
            "host killed mid-stream: the reader got an error {} ms later",
            died.elapsed().as_millis()
        ),
        t,
    );
    match host.call("providers", json!({})).await {
        Err(e) => println!("  ok  a call to the dead host fails: {e}"),
        Ok(_) => bail!("a dead host answered"),
    }

    println!("P0 passed.");
    Ok(())
}
