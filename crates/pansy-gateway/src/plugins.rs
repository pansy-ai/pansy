//! Where magpie's community plugins (`pansy-plugins`) meet a request:
//!
//! - a provider a plugin serves is asked through the plugin's host, which
//!   signs the request and sends it ([`exchange`]);
//! - the agents' middleware and request hooks see the request before it is
//!   masked and routed ([`before`]), each try's body and account, and the
//!   reply the agent gets ([`wrap`]);
//! - a group a middleware's routing orders is ordered by it ([`orders`]).
//!
//! Every hook fails open: what fails, or doesn't answer in time, leaves the
//! request, the event or the reply as it was.

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, header};
use axum::response::Response;
use futures::StreamExt;
use pansy_core::config::Config;
use pansy_core::provider::Protocol;
use pansy_plugins::host::{self as plugin_host, HookContext, Hooks};
use pansy_plugins::middleware::{self, Event as MwEvent, Info, Run};
use serde_json::Value;

use crate::ir;
use crate::proto;
use crate::route::{self, Attempt, Health};
use crate::sse;
use crate::upstream;

/// The AI SDK package a protocol is asked through, as a plugin's `fetch`
/// is told it.
pub fn npm_of(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::Chat => "@ai-sdk/openai-compatible",
        Protocol::Responses => "@ai-sdk/openai",
        Protocol::Anthropic => "@ai-sdk/anthropic",
        Protocol::Gemini => "@ai-sdk/google",
    }
}

/// Sends `attempt`'s request through the plugin that serves its provider:
/// at the address the account's loader gives (else the provider's), signed
/// by the plugin.
pub async fn exchange(
    attempt: &Attempt,
    client: &HeaderMap,
    passthrough: bool,
    stream: bool,
    body: Vec<u8>,
) -> Result<reqwest::Response, String> {
    let plugin = attempt.plugin.as_ref().ok_or("not a plugin's provider")?;
    let account = attempt
        .account
        .as_ref()
        .and_then(|a| a.plugin.as_ref())
        .map(|p| p.key.clone())
        .unwrap_or_default();
    let mut sent = attempt.clone();
    match plugin_host::base_url(&plugin.id, &account).await {
        Ok(Some(base)) => sent.endpoint.base_url = base,
        Ok(None) => {}
        Err(e) => return Err(format!("{e:#}")),
    }
    let url = upstream::url(&sent, stream);
    let headers = upstream::headers(&sent, client, passthrough)
        .iter()
        .filter_map(|(k, v)| Some((k.as_str().to_owned(), v.to_str().ok()?.to_owned())))
        .collect();
    let session = client
        .get("x-claude-code-session-id")
        .or_else(|| client.get("session_id"))
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    plugin_host::fetch(plugin_host::Request {
        provider: plugin.id.clone(),
        account,
        model: attempt.model.clone(),
        npm: npm_of(attempt.endpoint.protocol).to_owned(),
        url,
        headers,
        body,
        session,
        proxy: attempt.proxy.clone().unwrap_or_default(),
    })
    .await
    .map_err(|e| format!("{e:#}"))
}

/// What a request's plugins do with it beyond routing.
#[derive(Default)]
pub struct Hooked {
    pub middleware: Option<Arc<Run>>,
    pub hooks: Option<Hooks>,
}

/// The words of the last user message, for request hooks' `ctx.query`.
fn query_of(value: &Value) -> String {
    let text_of = |c: &Value| -> String {
        match c {
            Value::String(s) => s.clone(),
            Value::Array(parts) => parts
                .iter()
                .filter_map(|p| p["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        }
    };
    if let Some(messages) = value["messages"].as_array()
        && let Some(m) = messages.iter().rev().find(|m| m["role"] == "user")
    {
        return text_of(&m["content"]);
    }
    if let Some(input) = value["input"].as_array()
        && let Some(m) = input.iter().rev().find(|m| m["role"] == "user")
    {
        return text_of(&m["content"]);
    }
    if let Some(s) = value["input"].as_str() {
        return s.to_owned();
    }
    if let Some(contents) = value["contents"].as_array()
        && let Some(m) = contents.iter().rev().find(|m| m["role"] == "user")
    {
        return text_of(&m["parts"]);
    }
    String::new()
}

/// Runs the agents' middleware (`onRequest`) and request hooks on the
/// request, in the agent's protocol, before it is masked and routed: the
/// body they leave in `value`. `Err` is a request a middleware turned away.
#[allow(clippy::too_many_arguments)]
pub async fn before(
    value: &mut Value,
    client: Protocol,
    model: &str,
    stream: bool,
    path: &str,
    agent: &str,
    headers: &HeaderMap,
    local: bool,
) -> Result<Hooked, (u16, String)> {
    let mut out = Hooked::default();
    if let Some(run) = middleware::begin(Info {
        protocol: client.as_str().to_owned(),
        model: model.to_owned(),
        stream,
        path: path.to_owned(),
        agent: agent.to_owned(),
    })
    .await
    {
        match run.request(value).await {
            Ok(Some(v)) => *value = v,
            Ok(None) => {}
            Err(rejected) => return Err((rejected.status, rejected.message)),
        }
        out.middleware = Some(Arc::new(run));
    }
    if plugin_host::has_hooks() {
        let keyed = headers.contains_key(header::AUTHORIZATION) && !local;
        let subagent = crate::server::subagent_of(headers);
        let context = HookContext {
            protocol: client.as_str().to_owned(),
            model: model.to_owned(),
            stream,
            path: path.to_owned(),
            origin: if local {
                "local"
            } else if keyed {
                "gateway-key"
            } else {
                "remote"
            }
            .to_owned(),
            agent: agent.to_owned(),
            session: headers
                .get("x-claude-code-session-id")
                .or_else(|| headers.get("session_id"))
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_owned(),
            cwd: String::new(),
            query: query_of(value),
            kind: if subagent { "subagent" } else { "" }.to_owned(),
        };
        if let Some((body, hooks)) = plugin_host::hooks_begin(value, &context).await {
            if body.is_object() {
                *value = body;
            }
            out.hooks = Some(hooks);
        }
    }
    Ok(out)
}

/// The middleware's `onAccount` for a try: who it goes to, never who that
/// is but as a ref. `None` for a key.
pub fn account_of(attempt: &Attempt, config: &Config) -> Option<middleware::Account> {
    let account = attempt.account.as_ref()?;
    let name = config
        .provider(&attempt.provider)
        .map(|p| p.name.clone())
        .unwrap_or_default();
    if let Some(s) = &account.sign_in {
        return Some(middleware::Account {
            kind: s.kind.as_str().to_owned(),
            user: s.user.clone(),
            plan: s.plan.clone(),
            provider: attempt.provider.clone(),
            name,
            model: attempt.model.clone(),
        });
    }
    let p = account.plugin.as_ref()?;
    Some(middleware::Account {
        kind: attempt
            .plugin
            .as_ref()
            .map(|r| r.id.clone())
            .unwrap_or_default(),
        user: if p.user.is_empty() {
            p.key.clone()
        } else {
            p.user.clone()
        },
        plan: String::new(),
        provider: attempt.provider.clone(),
        name,
        model: attempt.model.clone(),
    })
}

/// `onProvider`'s `ctx.provider` for a try.
pub fn to_of(attempt: &Attempt, config: &Config) -> middleware::To {
    middleware::To {
        id: attempt.provider.clone(),
        name: config
            .provider(&attempt.provider)
            .map(|p| p.name.clone())
            .unwrap_or_default(),
        account: attempt
            .subscription
            .map(|k| k.as_str().to_owned())
            .or_else(|| attempt.plugin.as_ref().map(|r| r.id.clone()))
            .unwrap_or_default(),
        model: attempt.wire_model().to_owned(),
    }
}

/// The order each group a middleware's routing orders gets, by group id;
/// one whose routing fails, or that no plugin serves, keeps its order.
pub async fn orders(
    config: &Config,
    requested: &str,
    health: &Health,
) -> HashMap<String, Vec<usize>> {
    let mut out = HashMap::new();
    for (id, strategy, members) in route::strategy_pools(config, requested) {
        let seats: Vec<middleware::Seat> = members
            .iter()
            .map(|m| {
                let provider = m.split('/').next().unwrap_or_default();
                let resting = config.provider(provider).is_some_and(|p| {
                    !p.accounts.is_empty()
                        && p.enabled_accounts()
                            .all(|a| health.resting(&format!("{provider}:{}", a.id)))
                });
                middleware::Seat {
                    key: m.clone(),
                    weight: 1,
                    resting,
                    tokens: health.used(m) as f64,
                    ..Default::default()
                }
            })
            .collect();
        if let Some(order) = middleware::order(&strategy, &format!("group/{id}"), &seats).await {
            out.insert(id, order);
        }
    }
    out
}

/// Where an event of an SSE stream ends in `b`, and the blank line's length.
fn event_end(b: &[u8]) -> Option<(usize, usize)> {
    let lf = b.windows(2).position(|w| w == b"\n\n");
    let crlf = b.windows(4).position(|w| w == b"\r\n\r\n");
    match (lf, crlf) {
        (None, None) => None,
        (Some(i), None) => Some((i, 2)),
        (None, Some(j)) => Some((j, 4)),
        (Some(i), Some(j)) => Some(if j < i { (j, 4) } else { (i, 2) }),
    }
}

/// How long the comment and blank lines at the start of `b` are: they go
/// on as they come, as no hook sees them (a keepalive held back for the
/// event after it left a proxy hearing nothing).
fn comment_lines(b: &[u8]) -> usize {
    let mut n = 0;
    while n < b.len() && matches!(b[n], b':' | b'\n' | b'\r') {
        match b[n..].iter().position(|&c| c == b'\n') {
            Some(i) if !(b[n] == b'\r' && i != 1) => n += i + 1,
            _ => break,
        }
    }
    n
}

/// An event's name, what comes before its JSON, the JSON, and what after.
type Split<'a> = (String, &'a [u8], &'a [u8], &'a [u8]);

/// An event's one `data:` line holding JSON: its name, what comes before
/// the JSON, the JSON, and what comes after; `None` for any other event
/// (`[DONE]`, data over several lines).
fn split_event(ev: &[u8]) -> Option<Split<'_>> {
    let mut name = String::new();
    let mut at: Option<(usize, usize)> = None;
    let mut off = 0;
    for line in ev.split_inclusive(|&c| c == b'\n') {
        let t = line.strip_suffix(b"\n").unwrap_or(line);
        let t = t.strip_suffix(b"\r").unwrap_or(t);
        if let Some(rest) = t.strip_prefix(b"event:") {
            name = String::from_utf8_lossy(rest).trim().to_owned();
        } else if t.starts_with(b"data:") {
            if at.is_some() {
                return None;
            }
            at = Some((off, t.len()));
        }
        off += line.len();
    }
    let (start, len) = at?;
    let line = &ev[start..start + len];
    let skip = if line.get(5) == Some(&b' ') { 6 } else { 5 };
    let data = &line[skip..];
    if data.first() != Some(&b'{') {
        return None;
    }
    Some((name, &ev[..start + skip], data, &ev[start + len..]))
}

/// The reply the agent gets, through each `onEvent` and `onResponse`, and
/// told whole to the request hooks once it ends.
pub fn wrap(response: Response, client: Protocol, hooked: Hooked) -> Response {
    let Hooked { middleware, hooks } = hooked;
    let events = middleware.as_ref().is_some_and(|m| m.wants_events());
    let whole = middleware.as_ref().is_some_and(|m| m.wants_response());
    if !events && !whole && hooks.is_none() {
        return response;
    }
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let status = response.status().as_u16();
    let (mut parts, body) = response.into_parts();
    if content_type.contains("event-stream") {
        let mut data = body.into_data_stream();
        let stream = async_stream::stream! {
            let mut buf: Vec<u8> = Vec::new();
            let mut decoder = sse::Decoder::default();
            let mut reader = proto::reader(client);
            let mut collector = ir::Collector::default();
            let mut ok = true;
            let mut tell = |bytes: &[u8], collector: &mut ir::Collector| {
                if hooks.is_some() {
                    for frame in decoder.push(bytes) {
                        if let Ok(events) = reader.frame(&frame) {
                            for e in events {
                                collector.push(e);
                            }
                        }
                    }
                }
            };
            while let Some(chunk) = data.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => {
                        ok = false;
                        yield Err(std::io::Error::other(e));
                        break;
                    }
                };
                if !events {
                    tell(&chunk, &mut collector);
                    yield Ok::<Bytes, std::io::Error>(chunk);
                    continue;
                }
                buf.extend_from_slice(&chunk);
                loop {
                    let c = comment_lines(&buf);
                    if c > 0 {
                        let out: Vec<u8> = buf.drain(..c).collect();
                        yield Ok(Bytes::from(out));
                        continue;
                    }
                    let Some((i, n)) = event_end(&buf) else { break };
                    let ev: Vec<u8> = buf.drain(..i + n).collect();
                    let out = match split_event(&ev) {
                        Some((name, pre, json, post)) => {
                            let value: Option<Value> = serde_json::from_slice(json).ok();
                            match (value, middleware.as_ref()) {
                                (Some(v), Some(run)) => match run.event(&name, &v).await {
                                    MwEvent::Keep => Some(ev.clone()),
                                    MwEvent::Drop => None,
                                    MwEvent::Replace(new) => {
                                        let mut o = pre.to_vec();
                                        o.extend_from_slice(new.to_string().as_bytes());
                                        o.extend_from_slice(post);
                                        Some(o)
                                    }
                                },
                                _ => Some(ev.clone()),
                            }
                        }
                        None => Some(ev.clone()),
                    };
                    if let Some(out) = out {
                        tell(&out, &mut collector);
                        yield Ok(Bytes::from(out));
                    }
                }
            }
            if !buf.is_empty() {
                tell(&buf, &mut collector);
                yield Ok(Bytes::from(std::mem::take(&mut buf)));
            }
            if let Some(hooks) = hooks {
                let mut answer = collector.finish();
                answer.model = String::new();
                let reply = proto::build_response(client, &answer, &ir::Request::default());
                hooks.finish(reply, ok);
            }
            drop(middleware);
        };
        parts.headers.remove(header::CONTENT_LENGTH);
        return Response::from_parts(parts, Body::from_stream(stream));
    }
    if content_type.contains("json") {
        let stream = async_stream::stream! {
            let bytes = match axum::body::to_bytes(body, usize::MAX).await {
                Ok(b) => b,
                Err(e) => {
                    yield Err(std::io::Error::other(e));
                    return;
                }
            };
            let mut out = bytes.to_vec();
            if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
                let mut value = value;
                if whole
                    && let Some(run) = middleware.as_ref()
                    && let Some(new) = run.response(&value, status).await
                {
                    out = serde_json::to_vec(&new).unwrap_or(out);
                    value = new;
                }
                if let Some(hooks) = hooks {
                    hooks.finish(value, (200..300).contains(&status));
                }
            }
            yield Ok::<Bytes, std::io::Error>(Bytes::from(out));
        };
        parts.headers.remove(header::CONTENT_LENGTH);
        return Response::from_parts(parts, Body::from_stream(stream));
    }
    Response::from_parts(parts, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_an_sse_stream_as_magpie_does() {
        let b = b": keepalive\n\nevent: content_block_delta\ndata: {\"a\":1}\n\ndata: [DONE]\n\n";
        assert_eq!(comment_lines(b), 13);
        let rest = &b[13..];
        let (i, n) = event_end(rest).unwrap();
        let ev = &rest[..i + n];
        let (name, pre, json, post) = split_event(ev).unwrap();
        assert_eq!(name, "content_block_delta");
        assert_eq!(pre, b"event: content_block_delta\ndata: ");
        assert_eq!(json, b"{\"a\":1}");
        assert_eq!(post, b"\n\n");
        assert!(split_event(b"data: [DONE]\n\n").is_none());
        assert!(split_event(b"data: {\"a\":\ndata: 1}\n\n").is_none());
    }

    #[test]
    fn finds_the_last_user_words() {
        let chat = serde_json::json!({"messages": [{"role": "user", "content": "one"}, {"role": "assistant", "content": "x"}, {"role": "user", "content": [{"type": "text", "text": "two"}]}]});
        assert_eq!(query_of(&chat), "two");
        assert_eq!(query_of(&serde_json::json!({"input": "hey"})), "hey");
        let gemini = serde_json::json!({"contents": [{"role": "user", "parts": [{"text": "g"}]}]});
        assert_eq!(query_of(&gemini), "g");
    }
}
