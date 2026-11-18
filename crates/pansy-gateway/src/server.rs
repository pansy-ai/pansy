//! The HTTP side: the routes agents call, the attempts behind each request,
//! and the stream back.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures::StreamExt;
use pansy_core::config::{Config, Live};
use pansy_core::provider::{AuthHeader, Protocol};
use pansy_core::subscription;
use pansy_core::usage;
use serde_json::{Value, json};

use crate::ir::{self, Event, Usage};
use crate::mask;
use crate::plugins;
use crate::prompt;
use crate::proto::{self, Quirks};
use crate::route::{self, Attempt, Health, NoRoute};
use crate::sse;
use crate::trace::{Trace, Tracing};
use crate::upstream::{self, Clients, Verdict};

/// The most a request body may be: long conversations carry images.
const BODY_LIMIT: usize = 128 * 1024 * 1024;

#[derive(Default)]
pub struct Gateway {
    live: Live,
    health: Health,
    clients: Clients,
    trace: Arc<Trace>,
}

/// A Gemini call: its model and method are in the URL.
struct GeminiCall {
    model: String,
    method: String,
    sse: bool,
}

/// What a finished request is recorded with.
struct Meta {
    started: Instant,
    agent: String,
    /// The folder `pansy run` started the agent in, when it did.
    project: String,
    /// A subagent's request.
    subagent: bool,
    requested: String,
    client: Protocol,
    attempt: Attempt,
    stream: bool,
    failovers: u32,
    /// The accounts tried before this one, and what each answered.
    tried: Vec<usage::Tried>,
    /// Its place on the Routing page.
    tracing: Option<Tracing>,
    /// The masker that puts the values back in the answer, while masking
    /// is on, and the masking record of the request when it had any.
    unmask: Option<(Arc<pansy_mask::Masker>, Option<u64>)>,
    /// What the answer has used so far, and when its first byte came.
    usage: Usage,
    first_ms: u64,
    /// The answer is on its way: dropped now (the agent went away, its
    /// request cancelled), the request goes on record as cancelled.
    armed: bool,
}

impl Meta {
    fn new(
        started: Instant,
        agent: &str,
        requested: &str,
        client: Protocol,
        attempt: &Attempt,
        stream: bool,
        failovers: u32,
    ) -> Meta {
        Meta {
            started,
            agent: agent.to_owned(),
            project: String::new(),
            subagent: false,
            requested: requested.to_owned(),
            client,
            attempt: attempt.clone(),
            stream,
            failovers,
            tried: Vec::new(),
            tracing: None,
            unmask: None,
            usage: Usage::default(),
            first_ms: 0,
            armed: false,
        }
    }

    /// The answer failed before it began: the next attempt answers, and
    /// this one is no request of its own.
    fn fail<T>(&mut self, status: u16, message: impl Into<String>) -> Result<T, (u16, String)> {
        self.armed = false;
        Err((status, message.into()))
    }

    fn record(&mut self, status: u16, error: &str) {
        self.armed = false;
        let usage = self.usage;
        let first_ms = self.first_ms;
        let record = usage::Record {
            at: chrono::Utc::now(),
            agent: self.agent.clone(),
            project: self.project.clone(),
            subagent: self.subagent,
            requested: self.requested.clone(),
            provider: self.attempt.provider.clone(),
            account: self
                .attempt
                .account
                .as_ref()
                .map(|a| a.display())
                .unwrap_or_default(),
            model: self.attempt.model.clone(),
            client: self.client.as_str().to_owned(),
            upstream: self.attempt.endpoint.protocol.as_str().to_owned(),
            status,
            stream: self.stream,
            input: usage.input,
            output: usage.output,
            cache_read: usage.cache_read,
            cache_write: usage.cache_write,
            reasoning: usage.reasoning,
            ms: self.started.elapsed().as_millis() as u64,
            first_ms,
            failovers: self.failovers,
            error: error.to_owned(),
            tried: std::mem::take(&mut self.tried),
        };
        if let Err(e) = usage::append(&record) {
            tracing::warn!("recording usage: {e:#}");
        }
        // Each middleware's onUsage, off the request's path.
        pansy_plugins::middleware::usage(&record);
        if let Some(tracing) = &self.tracing {
            tracing.end(status, error, &usage, first_ms);
        }
    }
}

impl Drop for Meta {
    fn drop(&mut self) {
        if self.armed {
            self.record(499, "the agent went away before the answer ended");
        }
    }
}

/// A request's trace while no answer has it: dropped with it (the agent
/// went away), the request ends there as cancelled.
struct Open(Option<Tracing>);

impl Drop for Open {
    fn drop(&mut self) {
        if let Some(tracing) = self.0.take() {
            tracing.gone();
        }
    }
}

/// `attempt` as a usage record keeps a try: who, and what it answered.
fn tried_of(attempt: &Attempt, status: u16) -> usage::Tried {
    usage::Tried {
        provider: attempt.provider.clone(),
        account: attempt
            .account
            .as_ref()
            .map(|a| a.display())
            .unwrap_or_default(),
        model: attempt.model.clone(),
        status,
    }
}

fn is_content(event: &Event) -> bool {
    !matches!(event, Event::Start { .. } | Event::Usage(_))
}

/// The header an agent sends the model its subagents go on in, when its
/// own config can't put them there (Codex, whose lead names their models
/// itself): the gateway puts a subagent's request on it.
pub const SUBAGENT_MODEL: &str = "x-pansy-subagent-model";

/// Whether a request is a subagent's, as its agent says: Claude Code names
/// the subagent (`x-claude-code-agent-id`), Codex what the call is for
/// (`x-openai-subagent: collab_spawn`). Only what the agent sends counts,
/// never the prompt.
pub fn subagent_of(headers: &HeaderMap) -> bool {
    headers.contains_key("x-claude-code-agent-id")
        || headers
            .get("x-openai-subagent")
            .is_some_and(|v| v.as_bytes() == b"collab_spawn")
}

/// Whether a Responses request carries a task its lead's vendor sealed: an
/// `agent_message` whose content is encrypted, which only that vendor opens.
fn sealed_task(value: &Value) -> bool {
    value
        .get("input")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("agent_message"))
        .filter_map(|item| item.get("content").and_then(Value::as_array))
        .flatten()
        .any(|part| {
            part.get("type").and_then(Value::as_str) == Some("encrypted_content")
                && part
                    .get("encrypted_content")
                    .and_then(Value::as_str)
                    .is_some_and(|sealed| !sealed.is_empty())
        })
}

/// The agent behind a request, as its user agent names it.
pub fn agent_of(headers: &HeaderMap) -> String {
    if let Some(agent) = headers.get("x-pansy-agent").and_then(|v| v.to_str().ok()) {
        return agent.to_owned();
    }
    let ua = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    const KNOWN: &[(&str, &str)] = &[
        ("claude-cli", "claude-code"),
        ("claude-code", "claude-code"),
        ("codex", "codex"),
        ("geminicli", "gemini-cli"),
        ("gemini-cli", "gemini-cli"),
        ("opencode", "opencode"),
        ("qwen", "qwen-code"),
        ("crush", "crush"),
        ("kimi", "kimi-cli"),
        ("goose", "goose"),
        ("factory", "droid"),
        ("droid", "droid"),
        ("copilot", "copilot-cli"),
        ("cursor", "cursor"),
        ("zed", "zed"),
        ("cline", "cline"),
        ("roo", "roo-code"),
        ("kilo", "kilo-code"),
        ("pi-coding-agent", "pi"),
    ];
    KNOWN
        .iter()
        .find(|(needle, _)| ua.contains(needle))
        .map(|(_, agent)| (*agent).to_owned())
        .unwrap_or_else(|| "other".to_owned())
}

fn error(protocol: Protocol, status: u16, message: &str) -> Response {
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
    (
        status,
        axum::Json(proto::error_body(protocol, status.as_u16(), message)),
    )
        .into_response()
}

fn json_response(value: &Value) -> Response {
    axum::Json(value).into_response()
}

/// Sends `attempt`'s request. A vendor that refuses its key in both
/// `x-api-key` and `Authorization` is asked again with each alone, and the
/// one it takes is kept for the provider, as magpie learns it.
async fn send(
    http: &reqwest::Client,
    url: &str,
    attempt: &Attempt,
    client: &HeaderMap,
    passthrough: bool,
    body: Vec<u8>,
) -> reqwest::Result<reqwest::Response> {
    let post = |a: &Attempt| {
        http.post(url)
            .headers(upstream::headers(a, client, passthrough))
            .body(body.clone())
            .send()
    };
    let response = post(attempt).await?;
    if !upstream::sends_both(attempt) || response.status().as_u16() != 401 {
        return Ok(response);
    }
    let status = response.status();
    let headers = response.headers().clone();
    let text = response.bytes().await?;
    if !upstream::both_refused(status.as_u16(), &text) {
        let mut again = axum::http::Response::new(text);
        *again.status_mut() = status;
        *again.headers_mut() = headers;
        return Ok(reqwest::Response::from(again));
    }
    let mut last = None;
    for only in [AuthHeader::XApiKey, AuthHeader::Bearer] {
        let alone = Attempt {
            auth: Some(only),
            ..attempt.clone()
        };
        let response = post(&alone).await?;
        if !matches!(response.status().as_u16(), 401 | 403) {
            let provider = attempt.provider.clone();
            tokio::task::spawn_blocking(move || {
                if let Err(e) = pansy_core::manage::learn_auth_header(&provider, only) {
                    tracing::warn!("could not keep {provider}'s key header: {e:#}");
                }
            });
            return Ok(response);
        }
        last = Some(response);
    }
    Ok(last.expect("two tries"))
}

/// Why an attempt got no answer to read.
enum Unsent {
    /// Its subscription's sign-in couldn't be used (signed out, a refresh
    /// refused or not reached).
    Sign(String),
    /// The request didn't reach the vendor, or broke off.
    Send(reqwest::Error),
    /// The plugin that serves the provider couldn't send it.
    Plugin(String),
}

/// `attempt` signed with its subscription's account: the headers its
/// backend wants with the attempt's own, its address the one the vendor
/// gave; the token sent; and the body, finished with what only the account
/// knows. An attempt on a key is as it was.
async fn signed(
    attempt: &Attempt,
    http: &reqwest::Client,
    body: Vec<u8>,
    client: &HeaderMap,
) -> Result<(Attempt, String, Vec<u8>), Unsent> {
    let Some(kind) = attempt.subscription else {
        return Ok((attempt.clone(), String::new(), body));
    };
    let account = attempt
        .account
        .as_ref()
        .map(|a| a.id.as_str())
        .unwrap_or_default();
    // The Codex version a Codex coming through says it is.
    let version = client.get("version").and_then(|v| v.to_str().ok());
    let base = attempt.endpoint.base_url.as_str();
    let signed = subscription::sign(kind, account, base, http, &body, version)
        .await
        .map_err(|e| Unsent::Sign(format!("{e:#}")))?;
    let mut attempt = attempt.clone();
    attempt.headers.extend(signed.headers);
    if let Some(base) = signed.base {
        attempt.endpoint.base_url = base;
    }
    Ok((attempt, signed.token, signed.body.unwrap_or(body)))
}

/// Sends `attempt`'s request, signed. A subscription's token the backend
/// refuses (401) is refreshed, or its session traded for again, and the
/// request sent once more.
async fn exchange(
    http: &reqwest::Client,
    attempt: &Attempt,
    client: &HeaderMap,
    passthrough: bool,
    stream: bool,
    body: Vec<u8>,
) -> Result<reqwest::Response, Unsent> {
    // A plugin's provider: the plugin signs and sends it.
    if attempt.plugin.is_some() {
        return plugins::exchange(attempt, client, passthrough, stream, body)
            .await
            .map_err(Unsent::Plugin);
    }
    let (sent, token, signed_body) = signed(attempt, http, body.clone(), client).await?;
    let url = upstream::url(&sent, stream);
    let response = send(http, &url, &sent, client, passthrough, signed_body)
        .await
        .map_err(Unsent::Send)?;
    let Some(kind) = attempt.subscription else {
        return Ok(response);
    };
    if response.status().as_u16() != 401 {
        return Ok(response);
    }
    subscription::refused(kind, &token);
    let (sent, _, body) = signed(attempt, http, body, client).await?;
    let url = upstream::url(&sent, stream);
    send(http, &url, &sent, client, passthrough, body)
        .await
        .map_err(Unsent::Send)
}

/// A translated request as the attempt's vendor takes it: written in its
/// protocol, then what its models refuse taken out (`quirks.toml`). A
/// request that passes through never comes here.
fn build_for(attempt: &Attempt, request: &ir::Request) -> Value {
    let protocol = attempt.endpoint.protocol;
    let mut body =
        proto::build_request(protocol, request, attempt.wire_model(), &quirks_of(attempt));
    let why = pansy_core::quirk::apply(
        &mut body,
        &attempt.provider,
        attempt.preset.as_deref(),
        attempt.wire_model(),
        protocol,
    );
    if !why.is_empty() {
        tracing::debug!(provider = %attempt.provider, model = %attempt.model, "changed for the vendor: {}", why.join(" "));
    }
    body
}

fn quirks_of(attempt: &Attempt) -> Quirks {
    Quirks {
        max_completion_tokens: attempt.quirks.openai,
        reasoning_content: attempt.quirks.reasoning_content,
    }
}

/// The client's body for a provider that speaks its protocol: as it came,
/// with the provider's name for the model, and only that vendor's seals.
fn passthrough_body(body: &Value, client: Protocol, model: &str, stream: bool) -> Value {
    let mut body = body.clone();
    proto::scrub_seals(client, &mut body);
    if client != Protocol::Gemini {
        body["model"] = json!(model);
    }
    if client == Protocol::Chat && stream {
        // Usage comes in the last chunk only when asked for.
        body["stream_options"] = json!({"include_usage": true});
    }
    body
}

impl Gateway {
    pub fn new() -> Arc<Gateway> {
        Arc::new(Gateway::default())
    }

    pub fn config(&self) -> Arc<Config> {
        self.live.get()
    }

    pub fn health(&self) -> &Health {
        &self.health
    }

    /// The requests it routed lately, for the Routing page.
    pub fn trace(&self) -> &Arc<Trace> {
        &self.trace
    }

    pub fn router(self: &Arc<Self>) -> Router {
        Router::new()
            .route("/", get(root))
            .route("/health", get(root))
            .route(
                "/v1/messages",
                post(|s, r| relay(s, r, Protocol::Anthropic)),
            )
            .route("/v1/messages/count_tokens", post(count_tokens))
            .route(
                "/v1/chat/completions",
                post(|s, r| relay(s, r, Protocol::Chat)),
            )
            .route(
                "/chat/completions",
                post(|s, r| relay(s, r, Protocol::Chat)),
            )
            .route(
                "/v1/responses",
                post(|s, r| relay(s, r, Protocol::Responses)),
            )
            .route("/responses", post(|s, r| relay(s, r, Protocol::Responses)))
            .route("/v1beta/models/{*call}", post(gemini))
            .route("/v1/models/{*call}", post(gemini).get(model))
            .route("/v1/models", get(models))
            .route("/models", get(models))
            .route("/v1beta/models", get(gemini_models))
            .layer(middleware::from_fn_with_state(Arc::clone(self), web_pages))
            .with_state(Arc::clone(self))
    }

    /// Whether a request may use the gateway: this computer's always, the
    /// local network's with the gateway key when sharing is on.
    fn authorized(&self, request: &Request, config: &Config) -> bool {
        let peer = request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|c| c.0);
        if peer.is_none_or(|p| p.ip().is_loopback()) {
            return true;
        }
        let settings = &config.settings;
        if !settings.lan || settings.gateway_key.is_empty() {
            return false;
        }
        // Gemini's clients may send it in the URL.
        let in_url = request.uri().query().is_some_and(|q| {
            q.split('&').any(|p| {
                p.strip_prefix("key=").map(percent_decode).as_deref()
                    == Some(settings.gateway_key.as_str())
            })
        });
        in_url || has_key(request.headers(), &settings.gateway_key)
    }

    async fn handle(
        self: Arc<Self>,
        request: Request,
        client: Protocol,
        gemini: Option<GeminiCall>,
    ) -> Response {
        let started = Instant::now();
        let config = self.live.get();
        if !self.authorized(&request, &config) {
            return error(
                client,
                401,
                "this gateway needs its key from other computers",
            );
        }
        let local = request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .is_none_or(|c| c.0.ip().is_loopback());
        let (parts, body) = request.into_parts();
        let path = parts.uri.path().to_owned();
        let headers = parts.headers;
        // A key `pansy run` gave names the folder; only this computer's count.
        let project = if local {
            project_of(&headers, parts.uri.query())
        } else {
            String::new()
        };
        let body = match axum::body::to_bytes(body, BODY_LIMIT).await {
            Ok(body) => body,
            Err(e) => return error(client, 413, &format!("the request could not be read: {e}")),
        };
        let mut value: Value = match serde_json::from_slice(&body) {
            Ok(value) => value,
            Err(e) => return error(client, 400, &format!("the request is not JSON: {e}")),
        };
        let (requested, stream, json_array) = match &gemini {
            Some(call) => (
                call.model.clone(),
                call.method == "streamGenerateContent",
                !call.sse,
            ),
            None => (
                value
                    .get("model")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                value
                    .get("stream")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                false,
            ),
        };
        let agent = agent_of(&headers);
        if requested.trim().is_empty() {
            return error(client, 400, "the request names no model");
        }
        // The agents' middleware and request hooks see the request first,
        // in its own protocol, before it is masked and routed.
        let (mut requested, mut body) = (requested, body);
        let hooked = match plugins::before(
            &mut value, client, &requested, stream, &path, &agent, &headers, local,
        )
        .await
        {
            Ok(hooked) => hooked,
            Err((status, message)) => {
                tracing::info!(%agent, %requested, "a middleware turned the request away: {message}");
                return error(client, status, &message);
            }
        };
        if hooked.middleware.is_some() || hooked.hooks.is_some() {
            body = Bytes::from(serde_json::to_vec(&value).unwrap_or_default());
            // A middleware may name another model (Model Map's).
            if gemini.is_none()
                && let Some(model) = value.get("model").and_then(Value::as_str)
                && !model.trim().is_empty()
            {
                requested = model.to_owned();
            }
        }
        // A subagent goes on the model its agent set for its subagents
        // (SUBAGENT_MODEL), whatever it asked for; on the one it asked for
        // when that one can't take it, the trace saying why.
        let subagent = subagent_of(&headers);
        let (mut asked, mut kept) = (String::new(), String::new());
        let mut planned = None;
        if subagent
            && let Some(to) = headers
                .get(SUBAGENT_MODEL)
                .and_then(|v| v.to_str().ok())
                .map(str::trim)
                .filter(|to| !to.is_empty() && *to != requested)
        {
            if sealed_task(&value) {
                kept = format!(
                    "its task was sealed by the vendor that answered its lead, which {to} can't open"
                );
            } else {
                let orders = plugins::orders(&config, to, &self.health).await;
                match route::plan_with(&config, to, client, &self.health, &orders) {
                    Ok(lineup) => {
                        tracing::debug!(%agent, asked = %requested, %to, "a subagent, on the model set for them");
                        asked = std::mem::replace(&mut requested, to.to_owned());
                        planned = Some(lineup);
                    }
                    Err(NoRoute(message)) => kept = format!("{to} can't take it: {message}"),
                }
            }
        }
        let begin = |lineup: &[Attempt]| {
            let tracing = Tracing::begin(
                &self.trace,
                started,
                &config,
                &self.health,
                &agent,
                &requested,
                lineup,
            );
            if subagent {
                tracing.subagent(&asked, &kept);
            }
            tracing
        };
        let lineup = match planned {
            Some(lineup) => lineup,
            None => {
                let orders = plugins::orders(&config, &requested, &self.health).await;
                match route::plan_with(&config, &requested, client, &self.health, &orders) {
                    Ok(lineup) => lineup,
                    Err(NoRoute(message)) => {
                        tracing::info!(%agent, %requested, "no route: {message}");
                        begin(&[]).end(404, &message, &Usage::default(), 0);
                        return error(client, 404, &message);
                    }
                }
            }
        };
        let trace = begin(&lineup);
        let mut open = Open(Some(trace.clone()));
        let settings = &config.settings;
        let attempts = &lineup[..lineup.len().min(settings.max_attempts.max(1) as usize)];
        // Masked once, before any attempt: every vendor tried gets the same
        // placeholders, and every answer has the values back.
        let mut unmask = None;
        if settings.masking.wanted()
            && let Some(masker) = mask::masker()
        {
            let first = &attempts[0];
            let masked = mask::request(
                &masker,
                &body,
                &settings.masking,
                &agent,
                &first.provider,
                &first.model,
            );
            if !masked.blocked.is_empty() {
                tracing::info!(%agent, kinds = ?masked.blocked, "masking held a request back");
                let message = format!(
                    "pansy held this request back: it has {}, which Settings › Privacy blocks. Take it out, or set it to Mask there",
                    masked.blocked.join(", ")
                );
                trace.end(403, &message, &Usage::default(), 0);
                return error(client, 403, &message);
            }
            if let Some(body) = masked.body {
                value = match serde_json::from_slice(&body) {
                    Ok(value) => value,
                    Err(e) => {
                        let message = format!("masking broke the request: {e}");
                        trace.end(500, &message, &Usage::default(), 0);
                        return error(client, 500, &message);
                    }
                };
            }
            unmask = Some((masker, masked.key));
            trace.masked();
        }
        let cooldown = Duration::from_secs(settings.cooldown_secs.max(1));
        let first_byte = Duration::from_secs(settings.first_byte_timeout_secs.max(10));
        // Read once, as masked: what its prompt holds, for Routing's context
        // window, and the request a try that translates is built from. One
        // that can't be read passes through all the same, and fails a
        // translating try with why below.
        let mut ir_request: Option<ir::Request> =
            proto::parse_request(client, &value).ok().map(|mut r| {
                r.model = requested.clone();
                r.stream = stream;
                r
            });
        if let Some(request) = &ir_request {
            trace.prompt(prompt::of(request));
        }
        let mut last = (502u16, String::from("no provider answered"));
        let mut tried: Vec<usage::Tried> = Vec::new();
        let mut asked = false;
        for (n, attempt) in attempts.iter().enumerate() {
            trace.asking(n);
            // A middleware may pass the account over for this request: the
            // next is asked, and this one isn't rested.
            if let Some(run) = hooked.middleware.as_ref()
                && let Some(by) = run
                    .account(plugins::account_of(attempt, &config).as_ref())
                    .await
            {
                let message = format!("{}: passed over by middleware {by}", attempt.provider);
                trace.failed(409, &message, 0);
                if !asked {
                    last = (
                        404,
                        format!("none of {requested}'s models is ready: {message}"),
                    );
                }
                continue;
            }
            asked = true;
            // Each try's body, as the middleware make it for where it goes.
            let tried_value = match hooked.middleware.as_ref() {
                Some(run) => {
                    run.provider(&value, &plugins::to_of(attempt, &config))
                        .await
                }
                None => None,
            };
            let value = tried_value.as_ref().unwrap_or(&value);
            let mut fresh_ir: Option<ir::Request> = None;
            // Gemini's stream as one JSON array is written by the gateway
            // (vendors are asked for SSE).
            let passthrough = attempt.endpoint.protocol == client
                && !(stream && json_array)
                && attempt.subscription.is_none_or(|k| k.passes_through());
            let mut upstream_body = if passthrough {
                passthrough_body(value, client, attempt.wire_model(), stream)
            } else if tried_value.is_some() {
                match proto::parse_request(client, value) {
                    Ok(mut r) => {
                        r.model = requested.clone();
                        r.stream = stream;
                        let built = build_for(attempt, &r);
                        fresh_ir = Some(r);
                        built
                    }
                    Err(e) => {
                        trace.end(400, &e, &Usage::default(), 0);
                        return error(client, 400, &e);
                    }
                }
            } else {
                if ir_request.is_none() {
                    match proto::parse_request(client, value) {
                        Ok(mut r) => {
                            r.model = requested.clone();
                            r.stream = stream;
                            ir_request = Some(r);
                        }
                        Err(e) => {
                            trace.end(400, &e, &Usage::default(), 0);
                            return error(client, 400, &e);
                        }
                    }
                }
                build_for(attempt, ir_request.as_ref().unwrap())
            };
            let request_ir = fresh_ir.as_ref().or(ir_request.as_ref());
            // A subscription's backend takes the request its own way; one
            // that answers in a stream alone has a whole answer gathered.
            let mut gather = false;
            if let Some(kind) = attempt.subscription {
                subscription::prepare(
                    kind,
                    attempt.endpoint.protocol,
                    &attempt.model,
                    &mut upstream_body,
                );
                gather = !stream && kind.streams_only();
            }
            // The account's or provider's proxy, else pansy's (empty for the
            // system's).
            let proxy = attempt
                .proxy
                .clone()
                .filter(|p| !p.is_empty())
                .or_else(|| Some(settings.proxy.clone()).filter(|p| !p.is_empty()));
            let http = match self.clients.get(proxy.as_deref()) {
                Ok(http) => http,
                Err(e) => {
                    trace.failed(502, &e, 0);
                    tried.push(tried_of(attempt, 502));
                    last = (502, e);
                    continue;
                }
            };
            let send = exchange(
                &http,
                attempt,
                &headers,
                passthrough,
                stream || gather,
                serde_json::to_vec(&upstream_body).unwrap_or_default(),
            );
            let key = attempt.key_id();
            let response = match tokio::time::timeout(first_byte, send).await {
                Err(_) => {
                    self.health.rest(&key, Duration::from_secs(15));
                    last = (504, format!("{} did not answer in time", attempt.provider));
                    trace.failed(504, &last.1, 15);
                    tried.push(tried_of(attempt, 504));
                    continue;
                }
                Ok(Err(Unsent::Sign(e))) => {
                    self.health.rest(&key, cooldown);
                    last = (401, format!("{}: {e}", attempt.provider));
                    trace.failed(401, &last.1, cooldown.as_secs());
                    tried.push(tried_of(attempt, 401));
                    continue;
                }
                Ok(Err(Unsent::Plugin(e))) => {
                    self.health.rest(&key, Duration::from_secs(15));
                    last = (502, format!("{}: {e}", attempt.provider));
                    trace.failed(502, &last.1, 15);
                    tried.push(tried_of(attempt, 502));
                    continue;
                }
                Ok(Err(Unsent::Send(e))) => {
                    self.health.rest(&key, Duration::from_secs(15));
                    last = (
                        502,
                        upstream::hide_key(&format!("{}: {e}", attempt.provider), attempt),
                    );
                    trace.failed(502, &last.1, 15);
                    tried.push(tried_of(attempt, 502));
                    continue;
                }
                Ok(Ok(response)) => response,
            };
            let status = response.status().as_u16();
            trace.heard(status);
            let mut meta = Meta::new(
                started, &agent, &requested, client, attempt, stream, n as u32,
            );
            meta.project = project.clone();
            meta.subagent = subagent;
            meta.unmask = unmask.clone();
            meta.tried = tried.clone();
            meta.tracing = Some(trace.clone());
            if !response.status().is_success() {
                let retry = upstream::retry_after(response.headers());
                let text = response.text().await.unwrap_or_default();
                let retry = retry.or_else(|| upstream::retry_in(&text));
                let message = upstream::hide_key(
                    &format!("{}: {}", attempt.provider, upstream::error_message(&text)),
                    attempt,
                );
                tracing::info!(%agent, provider = %attempt.provider, status, "upstream refused: {message}");
                match upstream::verdict(status, &text, retry, cooldown) {
                    Verdict::Next(rest) => {
                        if let Some(rest) = rest {
                            self.health.rest(&key, rest);
                        }
                        trace.failed(status, &message, rest.map_or(0, |r| r.as_secs()));
                        tried.push(tried_of(attempt, status));
                        last = (status, message);
                        continue;
                    }
                    Verdict::Stop => {
                        meta.record(status, &message);
                        return error(client, status, &message);
                    }
                }
            }
            let answered = if gather {
                gathered(response, meta, request_ir).await
            } else if stream {
                self.stream(
                    response,
                    meta,
                    passthrough,
                    request_ir,
                    json_array,
                    first_byte,
                )
                .await
            } else {
                whole(response, meta, passthrough, request_ir).await
            };
            match answered {
                Ok(response) => {
                    self.health.healthy(&key);
                    self.health
                        .count(&format!("{}/{}", attempt.provider, attempt.model));
                    // The answer has the request now, to its end.
                    open.0 = None;
                    return plugins::wrap(response, client, hooked);
                }
                Err((status, message)) => {
                    tracing::info!(provider = %attempt.provider, "answer failed before it began: {message}");
                    self.health.rest(&key, cooldown);
                    last = (
                        status,
                        upstream::hide_key(&format!("{}: {message}", attempt.provider), attempt),
                    );
                    trace.failed(status, &last.1, cooldown.as_secs());
                    tried.push(tried_of(attempt, status));
                }
            }
        }
        if let Some(attempt) = attempts.last() {
            let failovers = attempts.len().saturating_sub(1) as u32;
            let mut meta = Meta::new(
                started, &agent, &requested, client, attempt, stream, failovers,
            );
            meta.project = project;
            meta.subagent = subagent;
            // The last try is the record's own.
            tried.pop();
            meta.tried = tried;
            meta.tracing = Some(trace);
            meta.record(last.0, &last.1);
        }
        error(client, last.0, &last.1)
    }

    /// Streams the answer. Until it has begun, an error is the attempt's
    /// (the next one is tried); after, the client hears it.
    async fn stream(
        &self,
        response: reqwest::Response,
        mut meta: Meta,
        passthrough: bool,
        request: Option<&ir::Request>,
        json_array: bool,
        first_byte: Duration,
    ) -> Result<Response, (u16, String)> {
        meta.armed = true;
        let upstream_protocol = meta.attempt.endpoint.protocol;
        let mut bytes = response.bytes_stream();
        let mut decoder = sse::Decoder::default();
        let mut reader = proto::reader(upstream_protocol);
        let mut writer = match (passthrough, request) {
            (false, Some(request)) => Some(proto::writer(
                meta.client,
                request,
                &meta.requested,
                json_array,
            )),
            _ => None,
        };
        let mut out: Vec<Bytes> = Vec::new();
        if let Some(writer) = writer.as_mut() {
            let begin = writer.begin();
            if !begin.is_empty() {
                out.push(Bytes::from(begin));
            }
        }
        let content_type = if meta.client == Protocol::Gemini && json_array {
            "application/json"
        } else {
            "text/event-stream"
        };
        let mut began = false;
        let mut ended = false;
        while !began {
            let chunk = match tokio::time::timeout(first_byte, bytes.next()).await {
                Err(_) => return meta.fail(504, "no answer in time"),
                Ok(None) => {
                    ended = true;
                    break;
                }
                Ok(Some(Err(e))) => return meta.fail(502, e.to_string()),
                Ok(Some(Ok(chunk))) => chunk,
            };
            if meta.first_ms == 0 {
                meta.first_ms = meta.started.elapsed().as_millis() as u64;
            }
            if passthrough {
                out.push(chunk.clone());
            }
            for frame in decoder.push(&chunk) {
                let events = match reader.frame(&frame) {
                    Ok(events) => events,
                    Err(message) => return meta.fail(502, message),
                };
                for event in events {
                    began |= is_content(&event);
                    if let Event::Usage(u) = &event {
                        meta.usage.merge(*u);
                    }
                    if let Some(writer) = writer.as_mut() {
                        let text = writer.event(event);
                        if !text.is_empty() {
                            out.push(Bytes::from(text));
                        }
                    }
                }
            }
            if reader.done() {
                ended = true;
                break;
            }
        }
        // The values masked go back in what the agent reads, event by event.
        let mut unmask = meta
            .unmask
            .as_ref()
            .map(|(masker, key)| (masker.stream(), *key));
        let body = async_stream::stream! {
            for chunk in out {
                match unmask.as_mut() {
                    Some((restorer, _)) => {
                        let chunk = restorer.push(&chunk);
                        if !chunk.is_empty() {
                            yield Ok::<Bytes, std::io::Error>(Bytes::from(chunk));
                        }
                    }
                    None => yield Ok::<Bytes, std::io::Error>(chunk),
                }
            }
            let mut failure = String::new();
            if !ended {
                'read: while let Some(chunk) = bytes.next().await {
                    let chunk = match chunk {
                        Ok(chunk) => chunk,
                        Err(e) => {
                            failure = format!("the upstream broke off: {e}");
                            break;
                        }
                    };
                    if passthrough {
                        match unmask.as_mut() {
                            Some((restorer, _)) => {
                                let back = restorer.push(&chunk);
                                if !back.is_empty() {
                                    yield Ok(Bytes::from(back));
                                }
                            }
                            None => yield Ok(chunk.clone()),
                        }
                    }
                    for frame in decoder.push(&chunk) {
                        match reader.frame(&frame) {
                            Ok(events) => {
                                for event in events {
                                    if let Event::Usage(u) = &event {
                                        meta.usage.merge(*u);
                                    }
                                    if let Some(writer) = writer.as_mut() {
                                        let text = writer.event(event);
                                        if !text.is_empty() {
                                            match unmask.as_mut() {
                                                Some((restorer, _)) => {
                                                    let back = restorer.push(text.as_bytes());
                                                    if !back.is_empty() {
                                                        yield Ok(Bytes::from(back));
                                                    }
                                                }
                                                None => yield Ok(Bytes::from(text)),
                                            }
                                        }
                                    }
                                }
                            }
                            Err(message) => {
                                failure = message;
                                break 'read;
                            }
                        }
                    }
                    if reader.done() {
                        break;
                    }
                }
            }
            let mut tail = String::new();
            if failure.is_empty() {
                for frame in decoder.finish() {
                    if let Ok(events) = reader.frame(&frame) {
                        for event in events {
                            if let Event::Usage(u) = &event {
                                meta.usage.merge(*u);
                            }
                            if let Some(writer) = writer.as_mut() {
                                tail.push_str(&writer.event(event));
                            }
                        }
                    }
                }
            }
            match writer.as_mut() {
                Some(writer) => {
                    tail.push_str(&if failure.is_empty() { writer.finish() } else { writer.error(502, &failure) });
                }
                // Passed through and broken off: the agent hears it, in its
                // protocol, rather than an answer that only stops.
                None if !failure.is_empty() => {
                    tail.push_str(&proto::stream_error(meta.client, 502, &failure));
                }
                None => {}
            }
            match unmask.take() {
                Some((mut restorer, key)) => {
                    let mut back = restorer.push(tail.as_bytes());
                    back.extend(restorer.finish());
                    if !back.is_empty() {
                        yield Ok(Bytes::from(back));
                    }
                    if let Some(key) = key {
                        pansy_core::masking::put_back(key, &restorer.put_back());
                    }
                }
                None if !tail.is_empty() => yield Ok(Bytes::from(tail)),
                None => {}
            }
            meta.record(if failure.is_empty() { 200 } else { 502 }, &failure);
        };
        let mut response = Response::new(Body::from_stream(body));
        let headers = response.headers_mut();
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        headers.insert("x-accel-buffering", HeaderValue::from_static("no"));
        Ok(response)
    }
}

/// A whole (non-streamed) answer, translated when the protocols differ.
async fn whole(
    response: reqwest::Response,
    mut meta: Meta,
    passthrough: bool,
    request: Option<&ir::Request>,
) -> Result<Response, (u16, String)> {
    meta.armed = true;
    meta.first_ms = meta.started.elapsed().as_millis() as u64;
    let bytes = match response.bytes().await {
        Ok(bytes) => bytes,
        Err(e) => return meta.fail(502, e.to_string()),
    };
    let upstream_protocol = meta.attempt.endpoint.protocol;
    let value: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(e) => return meta.fail(502, format!("the answer is not JSON: {e}")),
    };
    let parsed = match proto::parse_response(upstream_protocol, &value) {
        Ok(parsed) => parsed,
        Err(message) => return meta.fail(502, message),
    };
    meta.usage = parsed.usage;
    meta.record(200, "");
    let body = if passthrough {
        bytes.to_vec()
    } else {
        let mut parsed = parsed;
        parsed.model = meta.requested.clone();
        let empty = ir::Request::default();
        serde_json::to_vec(&proto::build_response(
            meta.client,
            &parsed,
            request.unwrap_or(&empty),
        ))
        .unwrap_or_default()
    };
    // The values masked go back in the answer.
    let body = match &meta.unmask {
        Some((masker, key)) => {
            let (body, back) = masker.restore_json(&body);
            if let Some(key) = key {
                pansy_core::masking::put_back(*key, &back);
            }
            body
        }
        None => body,
    };
    let mut response = Response::new(Body::from(body));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    Ok(response)
}

/// A whole answer gathered from a backend that answers in a stream alone
/// (Codex's), for a client that asked for a whole one.
async fn gathered(
    response: reqwest::Response,
    mut meta: Meta,
    request: Option<&ir::Request>,
) -> Result<Response, (u16, String)> {
    meta.armed = true;
    let mut bytes = response.bytes_stream();
    let mut decoder = sse::Decoder::default();
    let mut reader = proto::reader(meta.attempt.endpoint.protocol);
    let mut collector = ir::Collector::default();
    let mut take = |events: Vec<Event>, meta: &mut Meta| {
        for event in events {
            if let Event::Usage(u) = &event {
                meta.usage.merge(*u);
            }
            collector.push(event);
        }
    };
    while let Some(chunk) = bytes.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(e) => return meta.fail(502, format!("the upstream broke off: {e}")),
        };
        if meta.first_ms == 0 {
            meta.first_ms = meta.started.elapsed().as_millis() as u64;
        }
        for frame in decoder.push(&chunk) {
            match reader.frame(&frame) {
                Ok(events) => take(events, &mut meta),
                Err(message) => return meta.fail(502, message),
            }
        }
        if reader.done() {
            break;
        }
    }
    for frame in decoder.finish() {
        if let Ok(events) = reader.frame(&frame) {
            take(events, &mut meta);
        }
    }
    let mut answer = collector.finish();
    if answer.parts.is_empty() && answer.usage.is_empty() {
        return meta.fail(502, "the answer was empty");
    }
    answer.model = meta.requested.clone();
    meta.record(200, "");
    let empty = ir::Request::default();
    let body = serde_json::to_vec(&proto::build_response(
        meta.client,
        &answer,
        request.unwrap_or(&empty),
    ))
    .unwrap_or_default();
    let body = match &meta.unmask {
        Some((masker, key)) => {
            let (body, back) = masker.restore_json(&body);
            if let Some(key) = key {
                pansy_core::masking::put_back(*key, &back);
            }
            body
        }
        None => body,
    };
    let mut response = Response::new(Body::from(body));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    Ok(response)
}

/// The key a request's headers carry.
fn key_in(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim_start_matches("Bearer ").trim())
        .or_else(|| headers.get("x-api-key").and_then(|v| v.to_str().ok()))
        .or_else(|| headers.get("x-goog-api-key").and_then(|v| v.to_str().ok()))
}

/// Whether `headers` carry the gateway key `key` (never, when it is empty).
fn has_key(headers: &HeaderMap, key: &str) -> bool {
    !key.is_empty() && key_in(headers) == Some(key)
}

/// The folder a request's key names (`pansy run`'s), in a header or, as
/// Gemini's clients may send it, in the URL; empty for any other key.
pub fn project_of(headers: &HeaderMap, query: Option<&str>) -> String {
    key_in(headers)
        .and_then(pansy_core::project::of_key)
        .or_else(|| {
            query?
                .split('&')
                .filter_map(|p| p.strip_prefix("key="))
                .find_map(|key| pansy_core::project::of_key(&percent_decode(key)))
        })
        .unwrap_or_default()
}

/// A web page's origin on this computer (`http://localhost:5173`).
fn is_local_origin(origin: &str) -> bool {
    let Some(rest) = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    else {
        return false;
    };
    let host = match rest.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or_default(),
        None => rest.split(':').next().unwrap_or_default(),
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// Requests from browsers. A web page anywhere can send this computer's
/// gateway a request, so one from a page elsewhere (or a page that hides
/// where it is, `Origin: null`) is refused unless Settings › Network lists
/// it, and then needs the gateway key. Pages on this computer, and apps
/// with their own scheme (`vscode-file://`), may call it.
async fn web_pages(State(gateway): State<Arc<Gateway>>, request: Request, next: Next) -> Response {
    let Some(origin) = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
    else {
        return next.run(request).await;
    };
    let config = gateway.live.get();
    let settings = &config.settings;
    let web = origin == "null" || origin.starts_with("http://") || origin.starts_with("https://");
    let listed = web && !is_local_origin(&origin) && settings.allows_origin(&origin);
    if web && !is_local_origin(&origin) && !listed {
        tracing::info!(%origin, "refused a web page");
        return error(
            Protocol::Chat,
            403,
            &format!(
                "{origin} may not use this gateway: add it under Settings › Network and sharing"
            ),
        );
    }
    let allow = |response: &mut Response| {
        let headers = response.headers_mut();
        if let Ok(value) = HeaderValue::from_str(&origin) {
            headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, value);
        }
        headers.insert(header::VARY, HeaderValue::from_static("Origin"));
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET, POST, OPTIONS"),
        );
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("*"),
        );
        headers.insert(
            header::ACCESS_CONTROL_MAX_AGE,
            HeaderValue::from_static("600"),
        );
    };
    if request.method() == Method::OPTIONS {
        let mut response = StatusCode::NO_CONTENT.into_response();
        allow(&mut response);
        return response;
    }
    if listed && !has_key(request.headers(), &settings.gateway_key) {
        let mut response = error(
            Protocol::Chat,
            401,
            "a web page needs the gateway key (Settings › Network and sharing)",
        );
        allow(&mut response);
        return response;
    }
    let mut response = next.run(request).await;
    allow(&mut response);
    response
}

async fn relay(
    State(gateway): State<Arc<Gateway>>,
    request: Request,
    client: Protocol,
) -> Response {
    gateway.handle(request, client, None).await
}

async fn gemini(State(gateway): State<Arc<Gateway>>, request: Request) -> Response {
    if !gateway.authorized(&request, &gateway.live.get()) {
        return error(
            Protocol::Gemini,
            401,
            "this gateway needs its key from other computers",
        );
    }
    let path = request.uri().path().to_owned();
    let call = path
        .split_once("/models/")
        .map(|(_, c)| c)
        .unwrap_or_default();
    let Some((model, method)) = call.rsplit_once(':') else {
        return error(
            Protocol::Gemini,
            404,
            "expected /v1beta/models/{model}:{method}",
        );
    };
    let model = percent_decode(model);
    let sse = request
        .uri()
        .query()
        .is_some_and(|q| q.split('&').any(|p| p == "alt=sse"));
    match method {
        "generateContent" | "streamGenerateContent" => {
            let call = GeminiCall {
                model,
                method: method.to_owned(),
                sse,
            };
            gateway.handle(request, Protocol::Gemini, Some(call)).await
        }
        "countTokens" => {
            let body = axum::body::to_bytes(request.into_body(), BODY_LIMIT)
                .await
                .unwrap_or_default();
            let value: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            // countTokens may wrap the request as `generateContentRequest`.
            let inner = value.get("generateContentRequest").unwrap_or(&value);
            let tokens = proto::gemini::parse_request(inner)
                .map(|r| crate::count::request_tokens(&r))
                .unwrap_or(0);
            json_response(&json!({"totalTokens": tokens}))
        }
        _ => error(
            Protocol::Gemini,
            404,
            &format!("pansy does not answer {method}"),
        ),
    }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(&text[i + 1..i + 3], 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

async fn count_tokens(State(gateway): State<Arc<Gateway>>, request: Request) -> Response {
    let config = gateway.live.get();
    if !gateway.authorized(&request, &config) {
        return error(
            Protocol::Anthropic,
            401,
            "this gateway needs its key from other computers",
        );
    }
    let body = axum::body::to_bytes(request.into_body(), BODY_LIMIT)
        .await
        .unwrap_or_default();
    let value: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    match proto::anthropic::parse_request(&value) {
        Ok(r) => json_response(&json!({"input_tokens": crate::count::request_tokens(&r)})),
        Err(e) => error(Protocol::Anthropic, 400, &e),
    }
}

async fn root() -> Response {
    json_response(&json!({"name": "pansy", "version": env!("CARGO_PKG_VERSION"), "ok": true}))
}

/// Every model an agent can name: each provider's as `provider/model`, the
/// groups, and with `bareModelIds` each model's own id once.
fn model_ids(config: &Config) -> Vec<(String, String)> {
    let mut ids = Vec::new();
    let mut bare = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for provider in config.providers.iter().filter(|p| p.enabled) {
        for model in provider.visible_models() {
            let name = if model.name.is_empty() {
                model.id.clone()
            } else {
                model.name.clone()
            };
            ids.push((
                format!("{}/{}", provider.id, model.id),
                format!("{} · {name}", provider.name),
            ));
            if config.settings.bare_model_ids && seen.insert(model.id.clone()) {
                bare.push((model.id.clone(), name));
            }
        }
    }
    for group in &config.groups {
        ids.push((format!("group/{}", group.id), group.name.clone()));
    }
    ids.extend(bare);
    ids
}

async fn models(State(gateway): State<Arc<Gateway>>, request: Request) -> Response {
    let config = gateway.live.get();
    if !gateway.authorized(&request, &config) {
        return error(
            Protocol::Chat,
            401,
            "this gateway needs its key from other computers",
        );
    }
    let headers = request.headers();
    let ids = model_ids(&config);
    if headers.contains_key("anthropic-version") {
        let data: Vec<Value> = ids
            .iter()
            .map(|(id, name)| json!({"type": "model", "id": id, "display_name": name, "created_at": "2026-01-01T00:00:00Z"}))
            .collect();
        return json_response(&json!({
            "data": data, "has_more": false,
            "first_id": ids.first().map(|i| i.0.clone()), "last_id": ids.last().map(|i| i.0.clone()),
        }));
    }
    let data: Vec<Value> = ids
        .iter()
        .map(|(id, _)| {
            let owner = id.split_once('/').map_or("pansy", |(owner, _)| owner);
            json!({"id": id, "object": "model", "created": 0, "owned_by": owner})
        })
        .collect();
    json_response(&json!({"object": "list", "data": data}))
}

/// `GET /v1/models/{id}`: one model of the list.
async fn model(State(gateway): State<Arc<Gateway>>, request: Request) -> Response {
    let config = gateway.live.get();
    if !gateway.authorized(&request, &config) {
        return error(
            Protocol::Chat,
            401,
            "this gateway needs its key from other computers",
        );
    }
    let id = percent_decode(
        request
            .uri()
            .path()
            .split_once("/models/")
            .map(|(_, id)| id)
            .unwrap_or_default(),
    );
    match model_ids(&config).into_iter().find(|(m, _)| *m == id) {
        Some((id, _)) => {
            let owner = id.split_once('/').map_or("pansy", |(owner, _)| owner);
            json_response(&json!({"id": id, "object": "model", "created": 0, "owned_by": owner}))
        }
        None => error(Protocol::Chat, 404, &format!("no model {id}")),
    }
}

async fn gemini_models(State(gateway): State<Arc<Gateway>>, request: Request) -> Response {
    let config = gateway.live.get();
    if !gateway.authorized(&request, &config) {
        return error(
            Protocol::Gemini,
            401,
            "this gateway needs its key from other computers",
        );
    }
    let models: Vec<Value> = model_ids(&config)
        .iter()
        .map(|(id, name)| {
            json!({"name": format!("models/{id}"), "displayName": name,
                   "supportedGenerationMethods": ["generateContent", "streamGenerateContent", "countTokens"]})
        })
        .collect();
    json_response(&json!({"models": models}))
}

/// Runs the gateway until `shutdown` resolves.
pub async fn serve(
    gateway: Arc<Gateway>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    let settings = gateway.live.get().settings.clone();
    let ip = if settings.lan {
        [0, 0, 0, 0]
    } else {
        [127, 0, 0, 1]
    };
    let addr = SocketAddr::from((ip, settings.port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("listening on {addr} (is another pansy running?)"))?;
    tracing::info!("the gateway listens on http://{addr}");
    let watching = tokio::spawn(watch_allowances(Arc::clone(&gateway)));
    let plugins = tokio::spawn(keep_plugins());
    let served = axum::serve(
        listener,
        gateway
            .router()
            .into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await;
    watching.abort();
    plugins.abort();
    served?;
    Ok(())
}

/// While serving: the plugins' providers listed again now and then (their
/// models, and sign-ins made elsewhere), and their sign-ins renewed as they
/// come due.
async fn keep_plugins() {
    if pansy_plugins::host::plugins().is_ok_and(|p| p.is_empty()) {
        return;
    }
    let renewing = tokio::spawn(pansy_plugins::host::keep_signed_in());
    loop {
        if let Err(e) = pansy_plugins::host::list_now().await {
            tracing::warn!("listing the plugins' providers: {e:#}");
        }
        tokio::time::sleep(Duration::from_secs(30 * 60)).await;
        if renewing.is_finished() {
            break;
        }
    }
}

/// How often the subscriptions' allowances are read while serving.
const ALLOWANCES_EVERY: Duration = Duration::from_secs(5 * 60);

/// Reads the subscriptions' allowances now and then, and rests an account
/// its vendor holds until its window renews: the next account answers
/// rather than each request being refused first.
async fn watch_allowances(gateway: Arc<Gateway>) {
    tokio::time::sleep(Duration::from_secs(10)).await;
    loop {
        let config = gateway.config();
        let any = config
            .providers
            .iter()
            .any(|p| p.enabled && p.subscription.is_some());
        if any {
            for a in subscription::allowance::allowances(false).await {
                if let Some(rest) = a.held_for().filter(|_| a.enabled) {
                    tracing::info!(account = %a.key_id(), "held by its vendor for {}s", rest.as_secs());
                    gateway.health.rest(&a.key_id(), rest);
                }
            }
        }
        tokio::time::sleep(ALLOWANCES_EVERY).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_agents_by_their_user_agent() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::USER_AGENT,
            HeaderValue::from_static("claude-cli/2.1.0 (external, cli)"),
        );
        assert_eq!(agent_of(&headers), "claude-code");
        headers.insert(
            header::USER_AGENT,
            HeaderValue::from_static("codex_cli_rs/0.50.0"),
        );
        assert_eq!(agent_of(&headers), "codex");
        headers.insert(header::USER_AGENT, HeaderValue::from_static("curl/8"));
        assert_eq!(agent_of(&headers), "other");
    }

    #[test]
    fn knows_pages_on_this_computer() {
        assert!(is_local_origin("http://localhost:5173"));
        assert!(is_local_origin("http://127.0.0.1"));
        assert!(is_local_origin("http://[::1]:3000"));
        assert!(!is_local_origin("https://localhost.evil.example"));
        assert!(!is_local_origin("https://example.com"));
        assert!(!is_local_origin("null"));
    }

    #[test]
    fn checks_the_gateway_key() {
        let mut headers = HeaderMap::new();
        assert!(!has_key(&headers, "k"));
        headers.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer k"));
        assert!(has_key(&headers, "k"));
        assert!(!has_key(&headers, ""), "no key, no entry");
        assert!(!has_key(&headers, "other"));
    }

    #[test]
    fn decodes_model_names_in_urls() {
        assert_eq!(
            percent_decode("deepseek%2Fdeepseek-chat"),
            "deepseek/deepseek-chat"
        );
        assert_eq!(percent_decode("a%"), "a%");
    }
}
