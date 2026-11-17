//! Signing in to a subscription from pansy, as magpie does: ChatGPT in the
//! browser, with Codex's own OAuth client sending it back to this computer
//! (Codex's port, 1455), or by a code typed on ChatGPT's device page; and
//! GitHub by a code typed at github.com/login/device, as Copilot's editors
//! sign in; Google in the browser, with Gemini CLI's OAuth client, back at a
//! port of this computer's. The account is kept in `logins.json`.
//!
//! A sign-in runs on a thread of its own: [`start`] returns once there is a
//! page to open (and a code to type), and [`status`] says where it stands.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, mpsc};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use serde::Serialize;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::watch;

use super::{Kind, codex, copilot, gemini};

/// Where a sign-in stands, for the window and the command line to show.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub id: String,
    pub kind: Kind,
    /// `browser`, or `code` for a code typed on the vendor's page.
    pub how: String,
    /// The vendor's page, to open.
    pub url: String,
    /// What to type there.
    pub code: String,
    /// `starting`, `waiting`, `done`, `failed` or `canceled`.
    pub state: String,
    pub user: String,
    pub plan: String,
    /// The account's id under its subscription's provider, once done.
    pub account: String,
    pub error: String,
}

struct Flow {
    state: State,
    cancel: watch::Sender<bool>,
}

static FLOWS: LazyLock<Mutex<HashMap<String, Flow>>> = LazyLock::new(Default::default);

/// How long a sign-in waits for the browser or the code.
const WAIT: Duration = Duration::from_secs(15 * 60);

fn set(id: &str, change: impl FnOnce(&mut State)) {
    if let Some(flow) = FLOWS.lock().unwrap().get_mut(id) {
        change(&mut flow.state);
    }
}

/// Begins signing in to `kind`: in the browser, or by a code (`by_code`).
/// Returns once the page to open is known.
pub fn start(kind: Kind, by_code: bool) -> Result<State> {
    if by_code && !kind.signs_in_by_code() {
        bail!("{} has no sign-in by code", kind.name());
    }
    if !by_code && !kind.signs_in_by_browser() {
        bail!("{} signs in by a code typed on its page", kind.name());
    }
    let id = uuid::Uuid::new_v4().simple().to_string()[..12].to_owned();
    let (stop, canceled) = watch::channel(false);
    let state = State {
        id: id.clone(),
        kind,
        how: if by_code { "code" } else { "browser" }.to_owned(),
        url: String::new(),
        code: String::new(),
        state: "starting".to_owned(),
        user: String::new(),
        plan: String::new(),
        account: String::new(),
        error: String::new(),
    };
    FLOWS.lock().unwrap().insert(
        id.clone(),
        Flow {
            state,
            cancel: stop,
        },
    );
    let (ready, is_ready) = mpsc::channel::<Result<(), String>>();
    let flow = id.clone();
    std::thread::Builder::new()
        .name("pansy-sign-in".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(e) => {
                    let _ = ready.send(Err(e.to_string()));
                    return;
                }
            };
            runtime.block_on(run(flow, kind, by_code, canceled, ready));
        })?;
    match is_ready.recv_timeout(Duration::from_secs(60)) {
        Ok(Ok(())) => status(&id).ok_or_else(|| anyhow!("the sign-in is gone")),
        Ok(Err(message)) => {
            FLOWS.lock().unwrap().remove(&id);
            Err(anyhow!(message))
        }
        Err(_) => {
            cancel(&id);
            bail!("the sign-in didn't begin in time")
        }
    }
}

/// Where a sign-in stands.
pub fn status(id: &str) -> Option<State> {
    FLOWS.lock().unwrap().get(id).map(|f| f.state.clone())
}

/// Stops a sign-in that is still waiting.
pub fn cancel(id: &str) {
    if let Some(flow) = FLOWS.lock().unwrap().get_mut(id) {
        let _ = flow.cancel.send(true);
        if matches!(flow.state.state.as_str(), "starting" | "waiting") {
            flow.state.state = "canceled".to_owned();
        }
    }
}

/// Waits for a sign-in to end, for the command line.
pub fn wait(id: &str) -> State {
    loop {
        match status(id) {
            Some(s) if matches!(s.state.as_str(), "starting" | "waiting") => {
                std::thread::sleep(Duration::from_millis(400))
            }
            Some(s) => return s,
            None => {
                return State {
                    id: id.to_owned(),
                    kind: Kind::Codex,
                    how: String::new(),
                    url: String::new(),
                    code: String::new(),
                    state: "failed".to_owned(),
                    user: String::new(),
                    plan: String::new(),
                    account: String::new(),
                    error: "the sign-in is gone".to_owned(),
                };
            }
        }
    }
}

/// Says the page is ready (once), or why it couldn't be.
struct Ready(Option<mpsc::Sender<Result<(), String>>>);

impl Ready {
    fn page(&mut self, id: &str, url: String, code: String) {
        set(id, |s| {
            s.url = url;
            s.code = code;
            if s.state == "starting" {
                s.state = "waiting".to_owned();
            }
        });
        if let Some(ready) = self.0.take() {
            let _ = ready.send(Ok(()));
        }
    }

    /// Whether the error went to [`start`] (before the page was ready).
    fn failed(&mut self, message: &str) -> bool {
        match self.0.take() {
            Some(ready) => {
                let _ = ready.send(Err(message.to_owned()));
                true
            }
            None => false,
        }
    }
}

/// The account signed in to: its tokens, who it is, its plan.
type Signed = (Value, String, String);

async fn run(
    id: String,
    kind: Kind,
    by_code: bool,
    mut canceled: watch::Receiver<bool>,
    ready: mpsc::Sender<Result<(), String>>,
) {
    let mut ready = Ready(Some(ready));
    let outcome = async {
        let http = super::client(None)?;
        match (kind, by_code) {
            (Kind::Codex, false) => codex_browser(&id, &http, &mut canceled, &mut ready).await,
            (Kind::Codex, true) => codex_code(&id, &http, &mut canceled, &mut ready).await,
            (Kind::Copilot, _) => copilot_code(&id, &http, &mut canceled, &mut ready).await,
            (Kind::GeminiCli, _) => gemini_browser(&id, &http, &mut canceled, &mut ready).await,
        }
    }
    .await;
    let outcome = outcome.and_then(|signed| match signed {
        Some((auth, user, plan)) => {
            let account = super::add_login(kind, &user, &plan, auth)?;
            Ok(Some((user, plan, account)))
        }
        None => Ok(None),
    });
    match outcome {
        Ok(Some((user, plan, account))) => set(&id, |s| {
            s.state = "done".to_owned();
            s.user = user;
            s.plan = plan;
            // The agent's own sign-in of the same account stands for it.
            s.account = if super::found()
                .iter()
                .any(|f| f.sign_in.kind == kind && f.id == account)
            {
                account
            } else {
                super::OWN.to_owned()
            };
        }),
        Ok(None) => set(&id, |s| s.state = "canceled".to_owned()),
        Err(e) => {
            let message = format!("{e:#}");
            tracing::info!("signing in to {} failed: {message}", kind.name());
            if !ready.failed(&message) {
                set(&id, |s| {
                    s.state = "failed".to_owned();
                    s.error = message;
                });
            }
        }
    }
}

/// 64 random hex characters.
fn random() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// A sign-in in the browser: the vendor's page, opened with a PKCE
/// challenge, comes back to `listener` at `path` with a code, which
/// `exchange` trades for the account (given the code and the verifier).
#[allow(clippy::too_many_arguments)]
async fn in_browser<F, Fut>(
    id: &str,
    canceled: &mut watch::Receiver<bool>,
    ready: &mut Ready,
    listener: tokio::net::TcpListener,
    path: &str,
    page_of: impl FnOnce(&str, &str) -> String,
    exchange: F,
) -> Result<Option<Signed>>
where
    F: FnOnce(String, String) -> Fut,
    Fut: std::future::Future<Output = Result<Signed>>,
{
    use base64::Engine;
    use sha2::Digest;
    let verifier = random();
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(sha2::Sha256::digest(verifier.as_bytes()));
    let state = random();
    ready.page(id, page_of(&challenge, &state), String::new());
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let (mut stream, _) = tokio::select! {
            _ = canceled.changed() => return Ok(None),
            _ = tokio::time::sleep_until(deadline) => bail!("the sign-in waited too long; start again"),
            accepted = listener.accept() => accepted?,
        };
        let Some(target) = request_target(&mut stream).await else {
            continue;
        };
        let url = reqwest::Url::parse(&format!("http://localhost{target}"))?;
        let query: HashMap<String, String> = url.query_pairs().into_owned().collect();
        match url.path() {
            p if p == path => {}
            // Another `codex login` asks for the port.
            "/cancel" => {
                reply(&mut stream, "204 No Content", "").await;
                return Ok(None);
            }
            _ => {
                reply(&mut stream, "404 Not Found", "").await;
                continue;
            }
        }
        if let Some(error) = query.get("error") {
            let message = query.get("error_description").unwrap_or(error).clone();
            page(&mut stream, false, "Sign-in didn't finish", &message).await;
            bail!(message);
        }
        let code = query.get("code").cloned().unwrap_or_default();
        if query.get("state") != Some(&state) || code.is_empty() {
            page(
                &mut stream,
                false,
                "This link isn't from pansy's sign-in",
                "Start it again from pansy.",
            )
            .await;
            continue;
        }
        return match exchange(code, verifier).await {
            Ok(signed) => {
                let said = format!("{} is added to pansy. You can close this tab.", signed.1);
                page(&mut stream, true, "You're signed in", &said).await;
                Ok(Some(signed))
            }
            Err(e) => {
                page(
                    &mut stream,
                    false,
                    "Sign-in didn't finish",
                    &format!("{e:#}"),
                )
                .await;
                Err(e)
            }
        };
    }
}

/// ChatGPT's sign-in in the browser, back at Codex's port.
async fn codex_browser(
    id: &str,
    http: &reqwest::Client,
    canceled: &mut watch::Receiver<bool>,
    ready: &mut Ready,
) -> Result<Option<Signed>> {
    let listener = listen_callback().await?;
    let redirect = format!("http://localhost:{}/auth/callback", codex::CALLBACK_PORT);
    in_browser(
        id,
        canceled,
        ready,
        listener,
        "/auth/callback",
        |challenge, state| codex::authorize_url(challenge, state, &redirect),
        |code, verifier| {
            let redirect = &redirect;
            async move { codex::exchange(http, &code, &verifier, redirect).await }
        },
    )
    .await
}

/// Google's sign-in for Gemini CLI's client, back at a port of this
/// computer's, as Gemini CLI signs in.
async fn gemini_browser(
    id: &str,
    http: &reqwest::Client,
    canceled: &mut watch::Receiver<bool>,
    ready: &mut Ready,
) -> Result<Option<Signed>> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let redirect = format!(
        "http://127.0.0.1:{}{}",
        listener.local_addr()?.port(),
        gemini::CALLBACK
    );
    in_browser(
        id,
        canceled,
        ready,
        listener,
        gemini::CALLBACK,
        |challenge, state| gemini::authorize_url(challenge, state, &redirect),
        |code, verifier| {
            let redirect = &redirect;
            async move { gemini::exchange(http, &code, &verifier, redirect).await }
        },
    )
    .await
}

/// Codex's callback port; a `codex login` left waiting there is asked to
/// stop first, as Codex itself does.
async fn listen_callback() -> Result<tokio::net::TcpListener> {
    let address = ("127.0.0.1", codex::CALLBACK_PORT);
    let mut last = None;
    for attempt in 0..10 {
        match tokio::net::TcpListener::bind(address).await {
            Ok(listener) => return Ok(listener),
            Err(e) => last = Some(e),
        }
        if attempt == 0
            && let Ok(mut stream) = tokio::net::TcpStream::connect(address).await
        {
            let _ = stream
                .write_all(b"GET /cancel HTTP/1.0\r\nHost: localhost\r\n\r\n")
                .await;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    bail!(
        "port {} is taken ({}): another sign-in may be waiting there; sign in with a code instead",
        codex::CALLBACK_PORT,
        last.map(|e| e.to_string()).unwrap_or_default()
    )
}

/// The target of an HTTP request's first line (`/auth/callback?code=…`).
async fn request_target(stream: &mut tokio::net::TcpStream) -> Option<String> {
    let mut head = Vec::new();
    let mut buf = [0u8; 2048];
    let read = async {
        while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < 16 * 1024 {
            let n = stream.read(&mut buf).await.ok()?;
            if n == 0 {
                break;
            }
            head.extend_from_slice(&buf[..n]);
        }
        Some(())
    };
    tokio::time::timeout(Duration::from_secs(10), read)
        .await
        .ok()??;
    let text = String::from_utf8_lossy(&head);
    let mut parts = text.lines().next()?.split_whitespace();
    let _method = parts.next()?;
    Some(parts.next()?.to_owned())
}

async fn reply(stream: &mut tokio::net::TcpStream, status: &str, html: &str) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        html.len()
    );
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(html.as_bytes()).await;
    let _ = stream.shutdown().await;
}

fn escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The page the browser ends on.
async fn page(stream: &mut tokio::net::TcpStream, ok: bool, title: &str, message: &str) {
    let mark = if ok { "✓" } else { "!" };
    let html = format!(
        "<!doctype html><meta charset=utf-8><title>pansy</title>\
         <body style=\"font:15px system-ui;margin:20vh auto;max-width:28em;text-align:center;color:#333\">\
         <div style=\"font-size:32px\">{mark}</div><h2>{}</h2><p>{}</p></body>",
        escaped(title),
        escaped(message)
    );
    reply(stream, if ok { "200 OK" } else { "400 Bad Request" }, &html).await;
}

/// ChatGPT's sign-in by a code typed on its device page.
async fn codex_code(
    id: &str,
    http: &reqwest::Client,
    canceled: &mut watch::Receiver<bool>,
    ready: &mut Ready,
) -> Result<Option<Signed>> {
    let device = codex::device_code(http).await?;
    ready.page(id, codex::device_page(), device.code.clone());
    let deadline = Instant::now() + WAIT;
    let mut misses = 0;
    loop {
        tokio::select! {
            _ = canceled.changed() => return Ok(None),
            _ = tokio::time::sleep(Duration::from_secs(device.interval)) => {}
        }
        if Instant::now() > deadline {
            bail!("the code expired; start again");
        }
        match codex::device_poll(http, &device).await {
            Ok(None) => misses = 0,
            Ok(Some(signed)) => return Ok(Some(signed)),
            Err(e) => {
                misses += 1;
                if misses >= 5 {
                    return Err(e);
                }
            }
        }
    }
}

/// GitHub's sign-in by a code typed at github.com/login/device.
async fn copilot_code(
    id: &str,
    http: &reqwest::Client,
    canceled: &mut watch::Receiver<bool>,
    ready: &mut Ready,
) -> Result<Option<Signed>> {
    let device = copilot::device_code(http).await?;
    ready.page(id, device.page.clone(), device.code.clone());
    let deadline = Instant::now() + WAIT;
    let mut interval = device.interval;
    let mut misses = 0;
    loop {
        tokio::select! {
            _ = canceled.changed() => return Ok(None),
            _ = tokio::time::sleep(Duration::from_secs(interval)) => {}
        }
        if Instant::now() > deadline {
            bail!("the code expired; start again");
        }
        match copilot::device_poll(http, &device).await {
            Ok(copilot::Poll::Waiting) => misses = 0,
            Ok(copilot::Poll::Slower) => interval += 5,
            Ok(copilot::Poll::Done(auth, user, plan)) => return Ok(Some((auth, user, plan))),
            Err(e) if e.to_string().starts_with("couldn't reach") => {
                misses += 1;
                if misses >= 5 {
                    return Err(e);
                }
            }
            Err(e) => return Err(e),
        }
    }
}
