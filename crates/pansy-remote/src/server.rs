//! The remote interface: HTTP and WebSocket on a port of its own
//! (docs/plans/phone-protocol.md). Every request but the public descriptor
//! and the phone's web files carries a device's session; a connection from
//! outside this computer, a private network or a tailnet is closed before
//! it is read. It never shares the gateway's port: the gateway lets this
//! computer in without a key, and a tunnel's requests come from here.

use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::Router;
use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc};

use crate::auth::{self, Auth, Principal, Scope};
use crate::hub::{Coded, Dispatch, Hub, NewThread, Push, Update, thread_param};
use crate::{Answer, Decision, Mode, config, git};

/// What every handler shares.
pub struct App {
    pub hub: Hub,
    pub auth: Arc<Auth>,
    /// The phone's web build, served at `/`.
    pub web: Option<PathBuf>,
}

/// Serves until `shutdown` resolves.
pub async fn serve(
    app: Arc<App>,
    settings: config::Settings,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<()> {
    let ip: IpAddr = if settings.lan {
        [0, 0, 0, 0].into()
    } else {
        [127, 0, 0, 1].into()
    };
    let addr = SocketAddr::new(ip, settings.port);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("listening on {addr} (is another pansy running?)"))?;
    tracing::info!("the remote interface listens on http://{addr}");
    let hub = app.hub.clone();
    axum::serve(
        listener,
        router(app).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await?;
    hub.shutdown().await;
    Ok(())
}

pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/.well-known/pansy/environment", get(descriptor))
        .route("/api/auth/pair", post(pair))
        .route("/api/auth/refresh", post(refresh))
        .route("/api/auth/me", get(me))
        .route("/api/auth/ws-ticket", post(ticket))
        .route("/ws", get(socket))
        .fallback(get(files))
        .layer(middleware::from_fn(only_near))
        .with_state(app)
}

/// Closes connections from beyond this computer, its private networks and
/// tailnets: a machine with a public address doesn't show itself to the
/// internet. Not a firewall: every request is still authenticated.
async fn only_near(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Response {
    if !near(peer.ip()) {
        return StatusCode::FORBIDDEN.into_response();
    }
    next.run(request).await
}

pub fn near(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || (a == 100 && (64..128).contains(&b))
                || v4.is_link_local()
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return near(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            v6.is_loopback() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
    }
}

// --- Errors -----------------------------------------------------------------

struct Fail(StatusCode, &'static str, String);

impl IntoResponse for Fail {
    fn into_response(self) -> Response {
        (
            self.0,
            axum::Json(json!({"error": {"code": self.1, "message": self.2}})),
        )
            .into_response()
    }
}

/// The protocol's name for an error, and its HTTP status.
fn kind(e: &anyhow::Error) -> (StatusCode, &'static str) {
    if let Some(Coded(code, _)) = e.downcast_ref::<Coded>() {
        let status = match *code {
            "not_found" => StatusCode::NOT_FOUND,
            "conflict" => StatusCode::CONFLICT,
            "agent" => StatusCode::BAD_GATEWAY,
            _ => StatusCode::BAD_REQUEST,
        };
        return (status, code);
    }
    match e.downcast_ref::<auth::Error>() {
        Some(a) => (
            StatusCode::from_u16(a.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            a.code(),
        ),
        None => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
    }
}

impl From<auth::Error> for Fail {
    fn from(e: auth::Error) -> Fail {
        let status = StatusCode::from_u16(e.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        Fail(status, e.code(), e.to_string())
    }
}

impl From<anyhow::Error> for Fail {
    fn from(e: anyhow::Error) -> Fail {
        let (status, code) = kind(&e);
        Fail(status, code, format!("{e:#}"))
    }
}

// --- HTTP -------------------------------------------------------------------

/// The URL the client used, as its DPoP proof names it: the scheme a
/// tunnel says it took, the host it asked for, the path.
fn url_of(headers: &HeaderMap, uri: &Uri) -> String {
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .filter(|s| *s == "https")
        .unwrap_or("http");
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost");
    format!("{scheme}://{host}{}", uri.path())
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

fn request<'a>(method: &'a str, url: &'a str, headers: &'a HeaderMap) -> auth::Request<'a> {
    auth::Request {
        method,
        url,
        authorization: header_str(headers, "authorization"),
        dpop: header_str(headers, "dpop"),
        cookie: header_str(headers, "cookie"),
    }
}

async fn descriptor(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    let settings = config::load().unwrap_or_default();
    let mut caps = vec!["codex", "git", "github"];
    if pansy_agents::run::by_name("claude-code").is_some() {
        caps.push("claude");
    }
    let host = header_str(&headers, "host").unwrap_or_default().to_owned();
    axum::Json(json!({
        "id": app.auth.machine_id(),
        "name": config::machine_name(&settings),
        "version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "protocol": 1,
        "capabilities": caps,
        "routes": routes(settings.port, settings.lan, &host),
    }))
    .into_response()
}

/// This computer's addresses on its networks, for the phone to try in
/// order: private networks, then tailnets.
fn routes(port: u16, lan: bool, asked: &str) -> Vec<Value> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for ip in local_addresses().into_iter().filter(|_| lan) {
        let kind = match ip {
            IpAddr::V4(v4) if v4.octets()[0] == 100 && (64..128).contains(&v4.octets()[1]) => {
                "tailnet"
            }
            IpAddr::V4(v4) if v4.is_private() => "lan",
            _ => continue,
        };
        let url = format!("http://{ip}:{port}");
        if seen.insert(url.clone()) {
            out.push(json!({"kind": kind, "url": url}));
        }
    }
    out.sort_by_key(|r| (r["kind"] != "lan") as u8);
    if !asked.is_empty()
        && !out
            .iter()
            .any(|r| r["url"].as_str().is_some_and(|u| u.ends_with(asked)))
    {
        out.insert(
            0,
            json!({"kind": "asked", "url": format!("http://{asked}")}),
        );
    }
    out
}

/// `address:port` of this computer on its private networks and tailnets:
/// what a pairing link names. None while it answers this computer alone.
pub fn pair_hosts(port: u16, lan: bool) -> Vec<String> {
    local_addresses()
        .into_iter()
        .filter(|_| lan)
        .filter(|ip| near(*ip) && !ip.is_loopback())
        .map(|ip| format!("{ip}:{port}"))
        .collect()
}

/// Where the phone's web build is: `PANSY_MOBILE_WEB`, beside this
/// program (`mobile-web`, or a package's `share/pansy/mobile-web` and
/// `Resources/mobile-web`), else a checkout's `mobile/dist/web/Pansy`
/// (`nsh package --target web -o dist/web`).
pub fn web_dir() -> Option<PathBuf> {
    let index = |d: &PathBuf| d.join("index.html").is_file();
    if let Some(d) = std::env::var_os("PANSY_MOBILE_WEB").map(PathBuf::from) {
        return Some(d).filter(index);
    }
    if let Some(exe) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
    {
        for d in [
            exe.join("mobile-web"),
            exe.join("../share/pansy/mobile-web"),
            exe.join("../Resources/mobile-web"),
        ] {
            if index(&d) {
                return Some(d);
            }
        }
    }
    Some(PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../mobile/dist/web/Pansy"
    )))
    .filter(index)
}

/// The addresses of this computer's interfaces, as the system routes to a
/// few well-known destinations (no list of interfaces in std).
fn local_addresses() -> Vec<IpAddr> {
    let mut out = Vec::new();
    for probe in [
        "192.168.0.1:9",
        "10.0.0.1:9",
        "172.16.0.1:9",
        "100.100.100.100:9",
    ] {
        if let Ok(sock) = std::net::UdpSocket::bind("0.0.0.0:0")
            && sock.connect(probe).is_ok()
            && let Ok(a) = sock.local_addr()
            && !out.contains(&a.ip())
            && !a.ip().is_unspecified()
        {
            out.push(a.ip());
        }
    }
    out
}

async fn pair(
    State(app): State<Arc<App>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Result<Response, Fail> {
    let url = url_of(&headers, &uri);
    let req = request(method.as_str(), &url, &headers);
    let s = |k: &str| body[k].as_str().unwrap_or_default().to_owned();
    let cookie = body["cookie"].as_bool().unwrap_or(false);
    let paired = app
        .auth
        .pair(&req, &s("secret"), &s("deviceName"), &s("platform"), cookie)?;
    let mut response = axum::Json(paired.body()).into_response();
    if let Some(c) = paired.cookie.and_then(|c| HeaderValue::from_str(&c).ok()) {
        response.headers_mut().insert(header::SET_COOKIE, c);
    }
    Ok(response)
}

async fn refresh(
    State(app): State<Arc<App>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Result<Response, Fail> {
    let url = url_of(&headers, &uri);
    let (token, expires) = app
        .auth
        .refresh(&request(method.as_str(), &url, &headers))?;
    Ok(axum::Json(json!({"accessToken": token, "expiresIn": expires})).into_response())
}

fn authenticate(
    app: &App,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
) -> Result<Principal, Fail> {
    let url = url_of(headers, uri);
    Ok(app
        .auth
        .authenticate(&request(method.as_str(), &url, headers))?)
}

async fn me(
    State(app): State<Arc<App>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Result<Response, Fail> {
    let p = authenticate(&app, &method, &uri, &headers)?;
    let name = app
        .auth
        .device(&p.device)
        .map(|d| d.name)
        .unwrap_or_default();
    Ok(with_cookie(
        axum::Json(json!({
            "deviceId": p.device, "name": name,
            "scopes": p.scopes.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            "approval": if p.approved { "approved" } else { "pending" },
            "code": app.auth.code_for(&p.device),
        }))
        .into_response(),
        &p,
    ))
}

async fn ticket(
    State(app): State<Arc<App>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Result<Response, Fail> {
    let p = authenticate(&app, &method, &uri, &headers)?;
    if !p.approved {
        return Err(Fail(
            StatusCode::FORBIDDEN,
            "forbidden",
            "this device waits to be approved on the computer".into(),
        ));
    }
    Ok(with_cookie(
        axum::Json(json!({"ticket": app.auth.ws_ticket(&p)})).into_response(),
        &p,
    ))
}

/// A cookie session sent again, its age renewed, when auth says so.
fn with_cookie(mut r: Response, p: &Principal) -> Response {
    if let Some(c) = p
        .renew_cookie
        .as_deref()
        .and_then(|c| HeaderValue::from_str(c).ok())
    {
        r.headers_mut().insert(header::SET_COOKIE, c);
    }
    r
}

#[derive(serde::Deserialize)]
struct WsQuery {
    ticket: String,
    #[serde(default)]
    dpop: Option<String>,
}

async fn socket(
    State(app): State<Arc<App>>,
    uri: Uri,
    headers: HeaderMap,
    Query(q): Query<WsQuery>,
    ws: WebSocketUpgrade,
) -> Result<Response, Fail> {
    let url = url_of(&headers, &uri);
    let mut h = headers.clone();
    if let Some(proof) = q
        .dpop
        .as_deref()
        .and_then(|p| HeaderValue::from_str(p).ok())
    {
        h.insert("dpop", proof);
    }
    let principal = app
        .auth
        .redeem_ticket(&q.ticket, &request("GET", &url, &h))?;
    Ok(ws.on_upgrade(move |socket| connection(app, principal, socket)))
}

/// The phone's web build: a file under it, else its `index.html`.
async fn files(State(app): State<Arc<App>>, uri: Uri, headers: HeaderMap) -> Response {
    let Some(root) = app.web.as_ref() else {
        return (
            StatusCode::NOT_FOUND,
            "the phone's web app isn't built into this pansy",
        )
            .into_response();
    };
    let rel = uri.path().trim_start_matches('/');
    let safe = !rel.split('/').any(|p| p == ".." || p.starts_with('.'));
    let mut path = if safe && !rel.is_empty() {
        root.join(rel)
    } else {
        root.join("index.html")
    };
    if !path.is_file() {
        path = root.join("index.html");
    }
    // The build's own compressed copies, when the browser takes them.
    let accepts = header_str(&headers, "accept-encoding").unwrap_or_default();
    let mut encoding = None;
    let mut file = path.clone();
    for (enc, ext) in [("br", "br"), ("gzip", "gz")] {
        let packed = PathBuf::from(format!("{}.{ext}", path.display()));
        if accepts.split(',').any(|a| a.trim().starts_with(enc)) && packed.is_file() {
            (encoding, file) = (Some(enc), packed);
            break;
        }
    }
    match tokio::fs::read(&file).await {
        Ok(bytes) => {
            let mut r = Response::new(Body::from(bytes));
            let h = r.headers_mut();
            h.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static(content_type(&path)),
            );
            if let Some(enc) = encoding {
                h.insert(header::CONTENT_ENCODING, HeaderValue::from_static(enc));
                h.insert(header::VARY, HeaderValue::from_static("accept-encoding"));
            }
            h.insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static(if hashed(&path) {
                    "public, max-age=31536000, immutable"
                } else {
                    "no-cache"
                }),
            );
            r
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// A file named with its content's hash (`app.ad850b0d51.js`), or in a
/// folder so named: it never changes.
fn hashed(path: &Path) -> bool {
    path.components().any(|c| {
        c.as_os_str().to_str().is_some_and(|n| {
            n.split('.')
                .any(|p| p.len() == 10 && p.bytes().all(|b| b.is_ascii_hexdigit()))
        })
    })
}

fn content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
    {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "wasm" => "application/wasm",
        "json" | "webmanifest" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "ttf" => "font/ttf",
        "bin" => "application/octet-stream",
        "jpg" | "jpeg" => "image/jpeg",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

// --- WebSocket ----------------------------------------------------------------

/// One device's connection: its requests, and what it subscribed to.
async fn connection(app: Arc<App>, principal: Principal, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();
    let (out, mut outgoing) = mpsc::unbounded_channel::<Value>();
    let writer = tokio::spawn(async move {
        while let Some(v) = outgoing.recv().await {
            if sink
                .send(Message::Text(v.to_string().into()))
                .await
                .is_err()
            {
                break;
            }
        }
    });
    let mut pushes = app.hub.listen();
    let mut shell = false;
    let mut watching: HashSet<String> = HashSet::new();
    let mut ping = tokio::time::interval(Duration::from_secs(20));
    loop {
        tokio::select! {
            m = stream.next() => {
                let Some(Ok(m)) = m else { break };
                let Message::Text(text) = m else { continue };
                let Ok(req) = serde_json::from_str::<Value>(&text) else { continue };
                // A revoked device is cut off at its next word.
                if app.auth.device(&principal.device).is_none() {
                    break;
                }
                let method = req["method"].as_str().unwrap_or_default().to_owned();
                let params = req.get("params").cloned().unwrap_or(json!({}));
                let id = req["id"].clone();
                match method.as_str() {
                    "shell.subscribe" => shell = true,
                    "thread.subscribe" => {
                        if let Ok(t) = thread_param(&params)
                            && watching.insert(t.clone()) {
                                app.hub.watch(&t, true);
                            }
                    }
                    "thread.unsubscribe" => {
                        if let Ok(t) = thread_param(&params)
                            && watching.remove(&t) {
                                app.hub.watch(&t, false);
                            }
                    }
                    _ => {}
                }
                let (app, principal, out) = (Arc::clone(&app), principal.clone(), out.clone());
                tokio::spawn(async move {
                    let reply = match call(&app, &principal, &method, params).await {
                        Ok(result) => json!({"id": id, "result": result}),
                        Err(e) => {
                            let (_, code) = kind(&e);
                            json!({"id": id, "error": {"code": code, "message": format!("{e:#}")}})
                        }
                    };
                    let _ = out.send(reply);
                });
            }
            p = pushes.recv() => {
                match p {
                    Ok(p) => {
                        if let Some(v) = push_for(&p, shell, &watching) {
                            let _ = out.send(v);
                        }
                    }
                    // Too slow to keep up: the device subscribes again and asks
                    // for what it missed.
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        let _ = out.send(json!({"event": "resync", "data": {}}));
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            _ = ping.tick() => {
                let _ = out.send(json!({"event": "ping", "data": {}}));
            }
        }
    }
    for t in watching {
        app.hub.watch(&t, false);
    }
    drop(out);
    let _ = writer.await;
}

fn push_for(p: &Push, shell: bool, watching: &HashSet<String>) -> Option<Value> {
    Some(match p {
        Push::Thread(s) if shell => json!({"event": "shell.thread", "data": s}),
        Push::Removed(id) if shell => json!({"event": "shell.removed", "data": {"threadId": id}}),
        Push::Event { thread, seq, event } if watching.contains(thread) => {
            json!({"event": "thread.event", "data": {"threadId": thread, "seq": seq, "event": event}})
        }
        Push::Git {
            thread,
            step,
            status,
            detail,
        } if watching.contains(thread) || shell => json!({
            "event": "git.progress",
            "data": {"threadId": thread, "step": step, "status": status, "detail": detail},
        }),
        _ => return None,
    })
}

fn bad(message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(Coded("bad_request", message.into()))
}

fn forbidden(scope: Scope) -> anyhow::Error {
    anyhow::Error::new(Coded(
        "forbidden",
        format!("this device may not {}", scope.as_str()),
    ))
}

/// The scope each method needs.
fn scope_of(method: &str) -> Option<Scope> {
    Some(match method {
        "shell.subscribe" | "thread.subscribe" | "thread.unsubscribe" | "models.list"
        | "projects.list" | "git.status" | "git.diff" => Scope::OrchestrationRead,
        "thread.create"
        | "message.dispatch"
        | "runtime-request.respond"
        | "run.interrupt"
        | "thread.update"
        | "thread.delete" => Scope::OrchestrationOperate,
        "git.generate" | "git.run" => Scope::SourceControlWrite,
        "devices.list" => Scope::AccessRead,
        "devices.revoke"
        | "devices.approve"
        | "devices.requireApproval"
        | "devices.setRequireApproval"
        | "pair.create" => Scope::AccessWrite,
        _ => return None,
    })
}

async fn call(app: &App, p: &Principal, method: &str, params: Value) -> Result<Value> {
    let scope = scope_of(method).ok_or_else(|| bad(format!("no method {method}")))?;
    if !p.allows(scope) {
        return Err(forbidden(scope));
    }
    let hub = &app.hub;
    let s = |k: &str| params[k].as_str().map(str::to_owned);
    Ok(match method {
        "shell.subscribe" => {
            let machine = json!({
                "id": app.auth.machine_id(),
                "name": config::machine_name(&config::load().unwrap_or_default()),
            });
            let threads = tokio::task::spawn_blocking({
                let hub = hub.clone();
                move || hub.shell()
            })
            .await?;
            json!({"machine": machine, "threads": threads})
        }
        "thread.subscribe" => {
            let id = thread_param(&params)?;
            let after = params["after"].as_u64();
            let hub2 = hub.clone();
            let (summary, history, live, seq) =
                tokio::task::spawn_blocking(move || hub2.open(&id, after)).await??;
            let live: Vec<Value> = live
                .into_iter()
                .map(|(seq, event)| json!({"seq": seq, "event": event}))
                .collect();
            json!({"thread": summary, "history": history, "live": live, "seq": seq})
        }
        "thread.unsubscribe" => json!({}),
        "thread.create" => {
            let new = NewThread {
                folder: PathBuf::from(s("folder").ok_or_else(|| bad("folder is missing"))?),
                agent: s("agent").unwrap_or_else(|| "codex".into()),
                model: s("model").filter(|m| !m.is_empty()),
                effort: s("effort").filter(|m| !m.is_empty()),
                mode: mode_param(&params)?.unwrap_or_default(),
                workspace: params["workspace"]["kind"]
                    .as_str()
                    .unwrap_or("current")
                    .to_owned(),
                base: params["workspace"]["base"]
                    .as_str()
                    .map(str::to_owned)
                    .filter(|b| !b.is_empty()),
                text: s("text").unwrap_or_default(),
            };
            if new.text.trim().is_empty() {
                return Err(bad("say something to start the thread with"));
            }
            json!({"threadId": hub.create(new).await?})
        }
        "message.dispatch" => {
            let id = thread_param(&params)?;
            let text = s("text")
                .filter(|t| !t.trim().is_empty())
                .ok_or_else(|| bad("text is empty"))?;
            let how = Dispatch::parse(params["dispatchMode"].as_str().unwrap_or("start"))
                .ok_or_else(|| bad("dispatchMode: start, queue, steer or restart"))?;
            let id = hub
                .dispatch(&id, &text, how, s("model"), s("effort"))
                .await?;
            json!({"threadId": id})
        }
        "runtime-request.respond" => {
            let id = thread_param(&params)?;
            let request = s("requestId").ok_or_else(|| bad("requestId is missing"))?;
            let answer = if params["answers"].is_object() {
                Answer::Choices(params["answers"].clone())
            } else {
                Answer::Decision(
                    Decision::parse(params["decision"].as_str().unwrap_or_default()).ok_or_else(
                        || bad("decision: accept, acceptForSession, decline or cancel"),
                    )?,
                )
            };
            hub.respond(&id, &request, answer).await?;
            if let Some(feedback) = s("feedback").filter(|f| !f.trim().is_empty()) {
                hub.dispatch(&id, &feedback, Dispatch::Queue, None, None)
                    .await?;
            }
            json!({})
        }
        "run.interrupt" => {
            let force = params["force"].as_bool().unwrap_or(false);
            hub.interrupt(&thread_param(&params)?, force).await?;
            json!({})
        }
        "thread.update" => {
            let id = thread_param(&params)?;
            let snoozed = match params.get("snoozedUntil") {
                None => None,
                Some(Value::Null) => Some(None),
                Some(v) => Some(Some(
                    v.as_str()
                        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                        .map(|t| t.with_timezone(&chrono::Utc))
                        .ok_or_else(|| bad("snoozedUntil: an RFC 3339 time"))?,
                )),
            };
            let u = Update {
                title: s("title"),
                model: s("model"),
                effort: s("effort"),
                mode: mode_param(&params)?,
                pinned: params["pinned"].as_bool(),
                archived: params["archived"].as_bool(),
                snoozed_until: snoozed,
                unread: params["unread"].as_bool(),
            };
            json!({"thread": hub.update(&id, u).await?})
        }
        "thread.delete" => {
            let force = params["force"].as_bool().unwrap_or(false);
            let kept = hub.delete(&thread_param(&params)?, force).await?;
            json!({"worktreeKept": kept})
        }
        "models.list" => tokio::task::spawn_blocking(models).await??,
        "projects.list" => {
            let hub = hub.clone();
            tokio::task::spawn_blocking(move || projects(&hub)).await?
        }
        "git.status" => {
            let t = hub.thread(&thread_param(&params)?)?;
            let dir = PathBuf::from(&t.workdir);
            let mut status = serde_json::to_value(
                tokio::task::spawn_blocking(move || git::status(&dir)).await??,
            )?;
            if let Some(pr) = &t.pr {
                status["pr"] = serde_json::to_value(pr)?;
            }
            status
        }
        "git.diff" => {
            let t = hub.thread(&thread_param(&params)?)?;
            let scope = diff_scope(&t, params["scope"].as_str().unwrap_or("uncommitted"))?;
            let dir = PathBuf::from(&t.workdir);
            serde_json::to_value(
                tokio::task::spawn_blocking(move || git::diff(&dir, &scope)).await??,
            )?
        }
        "git.generate" => {
            let t = hub.thread(&thread_param(&params)?)?;
            let kind = match params["kind"].as_str().unwrap_or("commit") {
                "pr" => git::Writing::PullRequest,
                _ => git::Writing::Commit,
            };
            let (title, body) = generate(&t, kind).await?;
            json!({"title": title, "body": body})
        }
        "git.run" => run_git(app, &params).await?,
        "devices.list" => {
            let devices: Vec<Value> = app
                .auth
                .devices()
                .into_iter()
                .map(|d| {
                    let mut v = serde_json::to_value(&d).unwrap_or_default();
                    v["current"] = json!(d.id == p.device);
                    v
                })
                .collect();
            json!({"devices": devices, "requireApproval": app.auth.require_approval()})
        }
        "devices.revoke" => {
            app.auth
                .revoke(&s("deviceId").ok_or_else(|| bad("deviceId is missing"))?)?;
            json!({})
        }
        "devices.approve" => {
            app.auth
                .approve(&s("deviceId").ok_or_else(|| bad("deviceId is missing"))?)?;
            json!({})
        }
        "devices.requireApproval" | "devices.setRequireApproval" => {
            let on = params["requireApproval"]
                .as_bool()
                .or(params["on"].as_bool())
                .unwrap_or(false);
            app.auth.set_require_approval(on)?;
            json!({})
        }
        "pair.create" => {
            let scopes: Option<Vec<Scope>> = params["scopes"].as_array().map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str().and_then(Scope::parse))
                    .collect()
            });
            let (secret, expires) =
                app.auth
                    .create_pairing_by(p, scopes.as_deref(), auth::PAIRING_TTL)?;
            let settings = config::load().unwrap_or_default();
            let mut hosts = pair_hosts(settings.port, settings.lan);
            if hosts.is_empty() {
                hosts.push(format!("127.0.0.1:{}", settings.port));
            }
            let urls: Vec<String> = hosts
                .iter()
                .map(|h| format!("http://{h}/#pair={secret}"))
                .collect();
            json!({"secret": secret, "urls": urls, "expiresAt": expires})
        }
        _ => return Err(bad(format!("no method {method}"))),
    })
}

fn mode_param(params: &Value) -> Result<Option<Mode>> {
    match params.get("mode") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => Ok(Some(
            serde_json::from_value(v.clone()).map_err(|_| bad("mode: ask, edits, auto or full"))?,
        )),
    }
}

fn diff_scope(t: &crate::store::Thread, scope: &str) -> Result<git::Scope> {
    let n = t.turns;
    Ok(match scope {
        "uncommitted" => git::Scope::Uncommitted,
        "base" => git::Scope::Base(t.base.clone().unwrap_or_else(|| {
            git::default_branch(Path::new(&t.workdir)).unwrap_or_else(|_| "HEAD".into())
        })),
        "turn" | "lastTurn" if t.worktree && n >= 1 => {
            // The last turn: from the checkpoint before it (0 is the
            // worktree as made) to its own.
            git::Scope::Range {
                from: git::checkpoint_ref(&t.id, n - 1),
                to: git::checkpoint_ref(&t.id, n),
            }
        }
        "turn" | "lastTurn" => git::Scope::Uncommitted,
        other => return Err(bad(format!("no diff scope {other}"))),
    })
}

/// A summary of what changed, cut short, for the writer model.
async fn changes_text(dir: PathBuf) -> String {
    tokio::task::spawn_blocking(move || {
        let Ok(diff) = git::diff(&dir, &git::Scope::Uncommitted) else {
            return String::new();
        };
        let mut text = String::new();
        for f in &diff.files {
            text += &format!("--- {} (+{} -{})\n", f.path, f.add, f.del);
            for h in &f.hunks {
                text += &h.header;
                text.push('\n');
                for l in &h.lines {
                    text += &format!("{}{}\n", l.kind, l.text);
                }
            }
            if text.len() > 12_000 {
                break;
            }
        }
        text
    })
    .await
    .unwrap_or_default()
}

async fn generate(t: &crate::store::Thread, kind: git::Writing) -> Result<(String, String)> {
    let diff = changes_text(PathBuf::from(&t.workdir)).await;
    let (base, key) = crate::workspace::gateway(t);
    git::generate(
        kind,
        &diff,
        &t.title,
        &base,
        &key,
        &crate::workspace::writer(t),
    )
    .await
}

/// Whether nothing in `dir` waits to be committed.
async fn commit_clean(dir: &Path) -> bool {
    let d = dir.to_path_buf();
    tokio::task::spawn_blocking(move || git::status(&d))
        .await
        .ok()
        .and_then(Result::ok)
        .is_some_and(|s| s.files.is_empty())
}

async fn run_git(app: &App, params: &Value) -> Result<Value> {
    let hub = &app.hub;
    let id = thread_param(params)?;
    let t = hub.thread(&id)?;
    let action = params["action"].as_str().unwrap_or("commit").to_owned();
    if action == "pull" {
        hub.git_progress(&id, "pull", "running", String::new());
        let d = PathBuf::from(&t.workdir);
        return match tokio::task::spawn_blocking(move || git::pull(&d)).await? {
            Ok(said) => {
                hub.git_progress(&id, "pull", "done", said.clone());
                Ok(json!({"pull": said}))
            }
            Err(e) => {
                hub.git_progress(&id, "pull", "failed", format!("{e:#}"));
                Err(e)
            }
        };
    }
    let (commit, push, pr) = match action.as_str() {
        "commit" => (true, false, false),
        "push" => (false, true, false),
        "commit_push" => (true, true, false),
        "create_pr" => (false, true, true),
        "commit_push_pr" => (true, true, true),
        other => return Err(bad(format!("no git action {other}"))),
    };
    let dir = PathBuf::from(&t.workdir);
    let files: Option<Vec<String>> = params["files"].as_array().map(|a| {
        a.iter()
            .filter_map(|f| f.as_str().map(str::to_owned))
            .collect()
    });
    let mut result = json!({});
    // Nothing to commit: commit alone says so, commit and push pushes.
    let commit = commit && !commit_clean(&dir).await;
    if !commit && !push {
        return Err(anyhow::Error::new(crate::hub::Coded(
            "conflict",
            "nothing to commit".into(),
        )));
    }
    if commit {
        hub.git_progress(&id, "commit", "running", String::new());
        let message = match params["message"].as_str().filter(|m| !m.trim().is_empty()) {
            Some(m) => m.to_owned(),
            None => {
                let (title, body) = generate(&t, git::Writing::Commit).await?;
                if body.is_empty() {
                    title
                } else {
                    format!("{title}\n\n{body}")
                }
            }
        };
        let d = dir.clone();
        match tokio::task::spawn_blocking(move || git::commit(&d, files.as_deref(), &message))
            .await?
        {
            Ok(sha) => {
                hub.git_progress(&id, "commit", "done", sha.clone());
                result["commit"] = json!(sha);
            }
            Err(e) => {
                hub.git_progress(&id, "commit", "failed", format!("{e:#}"));
                return Err(e);
            }
        }
    }
    if push {
        hub.git_progress(&id, "push", "running", String::new());
        let d = dir.clone();
        match tokio::task::spawn_blocking(move || git::push(&d)).await? {
            Ok(out) => hub.git_progress(&id, "push", "done", out),
            Err(e) => {
                hub.git_progress(&id, "push", "failed", format!("{e:#}"));
                return Err(e);
            }
        }
    }
    if pr {
        hub.git_progress(&id, "pr", "running", String::new());
        let given = &params["pr"];
        let (mut title, mut body) = (
            given["title"].as_str().unwrap_or_default().to_owned(),
            given["body"].as_str().unwrap_or_default().to_owned(),
        );
        if title.trim().is_empty() {
            (title, body) = generate(&t, git::Writing::PullRequest).await?;
        }
        let base = given["base"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| {
                t.base
                    .clone()
                    .map(|b| b.trim_start_matches("origin/").to_owned())
            })
            .unwrap_or_else(|| git::default_branch(&dir).unwrap_or_else(|_| "main".into()));
        match git::create_pr(&dir, &title, &body, &base, "https://api.github.com").await {
            Ok(pr) => {
                hub.git_progress(&id, "pr", "done", pr.url.clone());
                hub.set_pr(
                    &id,
                    crate::store::PullRequest {
                        number: pr.number,
                        url: pr.url.clone(),
                        state: pr.state.clone(),
                    },
                );
                result["pr"] = json!({"number": pr.number, "url": pr.url, "state": pr.state});
            }
            Err(e) => {
                hub.git_progress(&id, "pr", "failed", format!("{e:#}"));
                return Err(e);
            }
        }
    }
    hub.changed(&id);
    Ok(result)
}

/// pansy's routing groups and every enabled provider's models.
fn models() -> Result<Value> {
    let config = pansy_core::config::Config::load()?;
    let groups: Vec<Value> = config
        .groups
        .iter()
        .map(|g| {
            json!({"id": format!("group/{}", g.id), "name": g.name, "members": g.members.len(),
                   "routing": g.strategy.clone().unwrap_or_else(|| g.routing.as_str().to_owned())})
        })
        .collect();
    let providers: Vec<Value> = config
        .providers
        .iter()
        .filter(|p| p.enabled)
        .map(|p| {
            let models: Vec<Value> = p
                .visible_models()
                .map(|m| {
                    json!({"id": format!("{}/{}", p.id, m.id),
                           "name": if m.name.is_empty() { m.id.clone() } else { m.name.clone() },
                           "context": m.context, "reasoning": m.reasoning, "vision": m.vision})
                })
                .collect();
            json!({"id": p.id, "name": p.name, "models": models})
        })
        .collect();
    Ok(json!({"groups": groups, "providers": providers}))
}

/// The folders the agents worked in, the latest first.
fn projects(hub: &Hub) -> Value {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for s in hub.shell() {
        if !seen.insert(s.folder.clone()) || !Path::new(&s.folder).is_dir() {
            continue;
        }
        let dir = Path::new(&s.folder);
        let is_git = git::is_repo(dir);
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        out.push(json!({"folder": s.folder, "name": name, "git": is_git}));
    }
    json!({"projects": out})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn near_is_this_computer_private_networks_and_tailnets() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.20.0.1",
            "192.168.1.20",
            "100.88.1.2",
            "::1",
            "fd7a:115c::1",
            "::ffff:192.168.1.2",
        ] {
            assert!(near(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "8.8.8.8",
            "100.128.0.1",
            "2001:4860::8888",
            "::ffff:1.1.1.1",
        ] {
            assert!(!near(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn every_method_names_a_scope() {
        for m in [
            "shell.subscribe",
            "thread.create",
            "git.run",
            "devices.revoke",
            "pair.create",
        ] {
            assert!(scope_of(m).is_some(), "{m}");
        }
        assert!(scope_of("terminal.open").is_none());
    }
}
