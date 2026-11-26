//! Reasoning an agent dropped from its tool calls goes back to the vendor
//! that wrote it: passed through (Chat to Chat, streamed and whole) and
//! translated (Anthropic to Chat), and never to a vendor that doesn't take
//! it. Runs in a scratch `PANSY_HOME`.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use pansy_core::config::save_providers;
use pansy_core::provider::{Endpoint, Protocol, Provider, Quirks};
use pansy_gateway::Gateway;
use serde_json::{Value, json};
use tower::ServiceExt;

/// A vendor that thinks and calls a tool when the last message is the
/// user's (each call a new id), and answers in words after a tool's result;
/// it keeps every body it was sent.
async fn thinking_vendor(seen: Arc<Mutex<Vec<Value>>>) -> SocketAddr {
    let calls = Arc::new(AtomicUsize::new(0));
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |body: axum::Json<Value>| {
            let seen = Arc::clone(&seen);
            let calls = Arc::clone(&calls);
            async move {
                seen.lock().unwrap().push(body.0.clone());
                let last = body["messages"].as_array().and_then(|m| m.last()).cloned().unwrap_or_default();
                if last["role"] != "user" {
                    return axum::Json(json!({
                        "id": "a", "model": "thinker",
                        "choices": [{"index": 0, "message": {"role": "assistant", "content": "Done."}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 1, "completion_tokens": 1},
                    }))
                    .into_response();
                }
                let id = format!("call_{}", calls.fetch_add(1, Ordering::SeqCst));
                let thought = format!("Read the file first ({id}).");
                if body["stream"] == true {
                    let chunks = [
                        json!({"id": "s", "model": "thinker", "choices": [{"delta": {"role": "assistant", "reasoning_content": "Read the file "}}]}),
                        json!({"choices": [{"delta": {"reasoning_content": &thought["Read the file ".len()..]}}]}),
                        json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "id": &id, "type": "function", "function": {"name": "read", "arguments": "{}"}}]}}]}),
                        json!({"choices": [{"delta": {}, "finish_reason": "tool_calls"}], "usage": {"prompt_tokens": 1, "completion_tokens": 1}}),
                    ];
                    let mut text: String = chunks.iter().map(|c| format!("data: {c}\n\n")).collect();
                    text.push_str("data: [DONE]\n\n");
                    return ([("content-type", "text/event-stream")], text).into_response();
                }
                axum::Json(json!({
                    "id": "w", "model": "thinker",
                    "choices": [{"index": 0, "message": {"role": "assistant", "content": null, "reasoning_content": thought,
                        "tool_calls": [{"id": id, "type": "function", "function": {"name": "read", "arguments": "{}"}}]},
                        "finish_reason": "tool_calls"}],
                    "usage": {"prompt_tokens": 1, "completion_tokens": 1},
                }))
                .into_response()
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

fn provider(id: &str, addr: SocketAddr, reasoning_content: bool) -> Provider {
    let mut p = Provider {
        id: id.into(),
        name: id.into(),
        preset: None,
        endpoints: vec![Endpoint {
            protocol: Protocol::Chat,
            base_url: format!("http://{addr}/v1"),
        }],
        accounts: Vec::new(),
        models: vec![pansy_core::provider::Model::new("thinker")],
        manual_models: Vec::new(),
        hidden_models: Vec::new(),
        enabled: true,
        headers: Default::default(),
        proxy: None,
        auth_header: None,
        quirks: Quirks {
            reasoning_content,
            ..Default::default()
        },
        subscription: None,
        plugin: None,
    };
    p.add_account("sk-test", "test");
    p
}

async fn call(gateway: &Arc<Gateway>, path: &str, body: Value) -> String {
    let request = Request::post(path)
        .header("content-type", "application/json")
        .header("user-agent", "agy/1.5.13")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = gateway.router().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert_eq!(status, StatusCode::OK, "{text}");
    text
}

/// The id of the tool call an answer made.
fn call_id(text: &str) -> String {
    let at = text.find("call_").expect(text);
    text[at..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect()
}

/// A Chat conversation that carries the call back bare, with its result.
fn bare_chat(model: &str, id: &str) -> Value {
    json!({"model": model, "messages": [
        {"role": "user", "content": "read it"},
        {"role": "assistant", "content": null, "tool_calls": [{"id": id, "type": "function", "function": {"name": "read", "arguments": "{}"}}]},
        {"role": "tool", "tool_call_id": id, "content": "the file"},
    ], "tools": [{"type": "function", "function": {"name": "read", "parameters": {"type": "object"}}}]})
}

/// The reasoning the last turn with tool calls carried to the vendor.
fn handed_back(seen: &Mutex<Vec<Value>>) -> Option<String> {
    let seen = seen.lock().unwrap();
    let body = seen.last().unwrap();
    body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|m| m["role"] == "assistant")
        .and_then(|m| m["reasoning_content"].as_str())
        .map(str::to_owned)
}

#[tokio::test]
async fn dropped_reasoning_goes_back_to_its_vendor() {
    let home = tempfile::tempdir().unwrap();
    // SAFETY: the only test in this binary; nothing else reads the env.
    unsafe {
        std::env::set_var("PANSY_HOME", home.path());
        std::env::set_var("HOME", home.path());
    }
    let seen = Arc::new(Mutex::new(Vec::new()));
    let addr = thinking_vendor(Arc::clone(&seen)).await;
    save_providers(&[
        provider("deepseek", addr, true),
        provider("plain", addr, false),
    ])
    .unwrap();
    let gateway = Gateway::new();
    let tools = json!([{"type": "function", "function": {"name": "read", "parameters": {"type": "object"}}}]);

    // Chat to Chat, streamed: passed through, the call made, its reasoning
    // dropped by the agent and put back.
    let text = call(
        &gateway,
        "/v1/chat/completions",
        json!({"model": "deepseek/thinker", "stream": true, "tools": tools,
               "messages": [{"role": "user", "content": "read it"}]}),
    )
    .await;
    let id = call_id(&text);
    call(
        &gateway,
        "/v1/chat/completions",
        bare_chat("deepseek/thinker", &id),
    )
    .await;
    assert_eq!(
        handed_back(&seen).as_deref(),
        Some(format!("Read the file first ({id}).").as_str())
    );

    // Whole, the same.
    let text = call(
        &gateway,
        "/v1/chat/completions",
        json!({"model": "deepseek/thinker", "tools": tools,
               "messages": [{"role": "user", "content": "read it"}]}),
    )
    .await;
    let id = call_id(&text);
    call(
        &gateway,
        "/v1/chat/completions",
        bare_chat("deepseek/thinker", &id),
    )
    .await;
    assert_eq!(
        handed_back(&seen).as_deref(),
        Some(format!("Read the file first ({id}).").as_str())
    );

    // An agent that kept its own is left as it sent it.
    let mut own = bare_chat("deepseek/thinker", &id);
    own["messages"][1]["reasoning_content"] = json!("Mine.");
    call(&gateway, "/v1/chat/completions", own).await;
    assert_eq!(handed_back(&seen).as_deref(), Some("Mine."));

    // Anthropic to Chat: Claude Code's turn without its thinking block.
    let text = call(
        &gateway,
        "/v1/messages",
        json!({"model": "deepseek/thinker", "max_tokens": 100,
               "tools": [{"name": "read", "input_schema": {"type": "object"}}],
               "messages": [{"role": "user", "content": "read it"}]}),
    )
    .await;
    let id = call_id(&text);
    call(
        &gateway,
        "/v1/messages",
        json!({"model": "deepseek/thinker", "max_tokens": 100,
               "tools": [{"name": "read", "input_schema": {"type": "object"}}],
               "messages": [
                   {"role": "user", "content": "read it"},
                   {"role": "assistant", "content": [{"type": "tool_use", "id": id, "name": "read", "input": {}}]},
                   {"role": "user", "content": [{"type": "tool_result", "tool_use_id": id, "content": "the file"}]},
               ]}),
    )
    .await;
    assert_eq!(
        handed_back(&seen).as_deref(),
        Some(format!("Read the file first ({id}).").as_str())
    );

    // A vendor that doesn't take reasoning back gets none, and another
    // provider never gets deepseek's.
    let text = call(
        &gateway,
        "/v1/chat/completions",
        json!({"model": "plain/thinker", "tools": tools,
               "messages": [{"role": "user", "content": "read it"}]}),
    )
    .await;
    let id = call_id(&text);
    call(
        &gateway,
        "/v1/chat/completions",
        bare_chat("plain/thinker", &id),
    )
    .await;
    assert_eq!(handed_back(&seen), None);
}
