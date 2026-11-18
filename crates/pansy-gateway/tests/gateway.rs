//! The gateway against fake upstreams on this computer: an agent's request
//! in one protocol reaches a provider in another, keys fail over, the usage
//! is recorded, a key is masked on the way out and put back on the way
//! home, and web pages are kept out. Everything runs in a scratch
//! `PANSY_HOME`.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::routing::post;
use pansy_core::config::{save_groups, save_providers};
use pansy_core::group::{Group, Routing};
use pansy_core::provider::{Endpoint, Protocol, Provider};
use pansy_gateway::Gateway;
use serde_json::{Value, json};
use tower::ServiceExt;

/// A fake OpenAI-compatible vendor: the first key is out of quota, the
/// second answers with reasoning, text and a tool call.
async fn fake_chat_vendor(hits: Arc<AtomicUsize>) -> SocketAddr {
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |headers: HeaderMap, body: axum::Json<Value>| {
            let hits = Arc::clone(&hits);
            async move {
                hits.fetch_add(1, Ordering::SeqCst);
                let auth = headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or_default().to_owned();
                if auth == "Bearer sk-empty" {
                    return (StatusCode::TOO_MANY_REQUESTS, axum::Json(json!({"error": {"message": "rate limited"}}))).into_response();
                }
                assert_eq!(body["model"], "fake-chat");
                if body["stream"] == true {
                    assert_eq!(body["stream_options"]["include_usage"], true);
                    let chunks = [
                        json!({"id": "c1", "model": "fake-chat", "choices": [{"delta": {"role": "assistant", "reasoning_content": "Thinking."}}]}),
                        json!({"choices": [{"delta": {"content": "Hello"}}]}),
                        json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "id": "call_1", "function": {"name": "Bash", "arguments": "{\"command\""}}]}}]}),
                        json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "function": {"arguments": ":\"ls\"}"}}]}}]}),
                        json!({"choices": [{"delta": {}, "finish_reason": "tool_calls"}], "usage": {"prompt_tokens": 12, "completion_tokens": 7}}),
                    ];
                    let mut text: String = chunks.iter().map(|c| format!("data: {c}\n\n")).collect();
                    text.push_str("data: [DONE]\n\n");
                    return ([("content-type", "text/event-stream")], text).into_response();
                }
                axum::Json(json!({
                    "id": "c2", "model": "fake-chat",
                    "choices": [{"index": 0, "message": {"role": "assistant", "content": "Hi there"}, "finish_reason": "stop"}],
                    "usage": {"prompt_tokens": 5, "completion_tokens": 2},
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

/// A fake vendor that says back what it was sent, the last message's text,
/// split in the middle of the reply when streamed; it keeps what it saw.
async fn echo_vendor(seen: Arc<std::sync::Mutex<Vec<String>>>) -> SocketAddr {
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |body: axum::Json<Value>| {
            let seen = Arc::clone(&seen);
            async move {
                seen.lock().unwrap().push(body.0.to_string());
                let said = body["messages"]
                    .as_array()
                    .and_then(|m| m.last())
                    .and_then(|m| m["content"].as_str())
                    .unwrap_or_default()
                    .to_owned();
                let reply = format!("You said: {said}");
                if body["stream"] == true {
                    // Cut inside the placeholder, as a vendor's tokens may.
                    let cut = reply.find("{{").map_or(reply.len() / 2, |at| at + 5);
                    let chunks = [
                        json!({"choices": [{"delta": {"role": "assistant", "content": &reply[..cut]}}]}),
                        json!({"choices": [{"delta": {"content": &reply[cut..]}}]}),
                        json!({"choices": [{"delta": {}, "finish_reason": "stop"}], "usage": {"prompt_tokens": 3, "completion_tokens": 4}}),
                    ];
                    let mut text: String = chunks.iter().map(|c| format!("data: {c}\n\n")).collect();
                    text.push_str("data: [DONE]\n\n");
                    return ([("content-type", "text/event-stream")], text).into_response();
                }
                axum::Json(json!({
                    "id": "e1", "model": "echo",
                    "choices": [{"index": 0, "message": {"role": "assistant", "content": reply}, "finish_reason": "stop"}],
                    "usage": {"prompt_tokens": 3, "completion_tokens": 4},
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

use axum::response::IntoResponse;

async fn call(gateway: &Arc<Gateway>, path: &str, ua: &str, body: Value) -> (StatusCode, String) {
    let request = Request::post(path)
        .header("content-type", "application/json")
        .header("user-agent", ua)
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = gateway.router().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

fn sse_types(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .filter_map(|d| serde_json::from_str::<Value>(d).ok())
        .filter_map(|v| v.get("type").and_then(Value::as_str).map(str::to_owned))
        .collect()
}

#[tokio::test]
async fn agents_reach_a_chat_vendor_through_the_gateway() {
    let home = tempfile::tempdir().unwrap();
    // SAFETY: the only test in this binary; nothing else reads the env.
    unsafe {
        std::env::set_var("PANSY_HOME", home.path());
        std::env::set_var("HOME", home.path());
        std::env::remove_var("CODEX_HOME");
        std::env::remove_var("XDG_CONFIG_HOME");
    }
    let hits = Arc::new(AtomicUsize::new(0));
    let addr = fake_chat_vendor(Arc::clone(&hits)).await;
    let mut vendor = Provider {
        id: "fake".into(),
        name: "Fake".into(),
        preset: None,
        endpoints: vec![Endpoint {
            protocol: Protocol::Chat,
            base_url: format!("http://{addr}/v1"),
        }],
        accounts: Vec::new(),
        models: vec![pansy_core::provider::Model::new("fake-chat")],
        manual_models: Vec::new(),
        hidden_models: Vec::new(),
        enabled: true,
        headers: Default::default(),
        proxy: None,
        auth_header: None,
        quirks: Default::default(),
        subscription: None,
        plugin: None,
    };
    vendor.add_account("sk-empty", "empty");
    vendor.add_account("sk-good", "good");
    save_providers(&[vendor]).unwrap();
    save_groups(&[Group {
        id: "daily".into(),
        name: "Daily".into(),
        members: vec!["fake/fake-chat".into()],
        routing: Routing::Order,
        strategy: None,
    }])
    .unwrap();
    let gateway = Gateway::new();

    // Claude Code (Anthropic) → OpenAI Chat, streamed; the first key fails over.
    let (status, text) = call(
        &gateway,
        "/v1/messages",
        "claude-cli/2.0.0",
        json!({"model": "fake/fake-chat", "max_tokens": 100, "stream": true,
               "messages": [{"role": "user", "content": "list files"}],
               "tools": [{"name": "Bash", "input_schema": {"type": "object"}}]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let types = sse_types(&text);
    assert_eq!(types.first().map(String::as_str), Some("message_start"));
    assert_eq!(types.last().map(String::as_str), Some("message_stop"));
    assert!(text.contains("\"thinking_delta\""), "{text}");
    assert!(text.contains("\"partial_json\":\":\\\"ls\\\"}\""), "{text}");
    assert!(text.contains("\"stop_reason\":\"tool_use\""), "{text}");
    assert_eq!(hits.load(Ordering::SeqCst), 2);

    // The Routing page sees both tries: the empty key refused and resting,
    // the good one answering.
    let routes = gateway.trace().routes();
    let route = routes.last().unwrap();
    assert!(route.done);
    assert_eq!((route.status, route.agent.as_str()), (200, "claude-code"));
    let tries: Vec<(u16, &str)> = route
        .tries
        .iter()
        .map(|t| (t.status, route.lineup[t.slot].account.as_str()))
        .collect();
    assert_eq!(tries, [(429, "empty"), (200, "good")]);
    assert!(route.tries[0].rest > 0);
    assert!(route.tries[1].us > 0);
    assert_eq!((route.input, route.output), (12, 7));
    // And what its prompt held, by parts: the tool, the turn; never the words.
    let prompt = route.prompt.as_ref().unwrap();
    let kinds: Vec<&str> = prompt.parts.iter().map(|p| p.kind).collect();
    assert_eq!(kinds, ["tools", "chat"]);
    assert_eq!(prompt.parts[0].items[0].name, "Bash");
    assert_eq!(prompt.turns, 1);
    assert!(
        !serde_json::to_string(prompt)
            .unwrap()
            .contains("list files")
    );

    // The resting key is tried last now: one hit.
    let (status, text) = call(
        &gateway,
        "/v1/responses",
        "codex_cli_rs/0.60.0",
        json!({"model": "group/daily", "stream": true, "instructions": "be brief",
               "input": [{"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hi"}]}],
               "tools": [{"type": "function", "name": "Bash", "parameters": {"type": "object"}}]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(hits.load(Ordering::SeqCst), 3);
    let types = sse_types(&text);
    assert_eq!(types.first().map(String::as_str), Some("response.created"));
    assert_eq!(types.last().map(String::as_str), Some("response.completed"));
    assert!(
        types
            .iter()
            .any(|t| t == "response.function_call_arguments.delta")
    );

    // Gemini CLI, whole answer.
    let (status, text) = call(
        &gateway,
        "/v1beta/models/fake%2Ffake-chat:generateContent",
        "GeminiCLI/0.10.0",
        json!({"contents": [{"role": "user", "parts": [{"text": "hi"}]}]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let answer: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        answer["candidates"][0]["content"]["parts"][0]["text"],
        "Hi there"
    );
    assert_eq!(answer["usageMetadata"]["promptTokenCount"], 5);

    // OpenAI Chat client: passes through as it came.
    let (status, text) = call(
        &gateway,
        "/v1/chat/completions",
        "opencode/1.0",
        json!({"model": "fake-chat", "messages": [{"role": "user", "content": "hi"}]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert!(text.contains("Hi there"));

    // A model no provider has.
    let (status, text) = call(
        &gateway,
        "/v1/messages",
        "claude-cli/2",
        json!({"model": "nope/x", "max_tokens": 5, "messages": []}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{text}");
    assert!(text.contains("\"type\":\"error\""));

    // The model list.
    let response = gateway
        .router()
        .oneshot(Request::get("/v1/models").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let list: Value = serde_json::from_slice(&bytes).unwrap();
    let ids: Vec<&str> = list["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["fake/fake-chat", "group/daily"]);

    // What was used is on record.
    let records = pansy_core::usage::recent(1);
    assert_eq!(records.len(), 4, "{records:#?}");
    assert_eq!(records[0].agent, "claude-code");
    assert_eq!(records[0].failovers, 1);
    assert_eq!(records[0].tried.len(), 1);
    assert_eq!(
        (
            records[0].tried[0].account.as_str(),
            records[0].tried[0].status
        ),
        ("empty", 429)
    );
    assert_eq!((records[0].input, records[0].output), (12, 7));
    assert_eq!(records[1].agent, "codex");
    assert_eq!(records[2].agent, "gemini-cli");
    assert!(records.iter().all(|r| r.project.is_empty()));

    // An agent `pansy run` started sends a key that names its folder: the
    // request is counted under it, in a header or in Gemini's URL.
    let key = pansy_core::project::key_for(std::path::Path::new("/code/shop"));
    let run = |path: &str, header: Option<(&str, String)>| {
        let mut request = Request::post(path).header("content-type", "application/json");
        if let Some((name, value)) = header {
            request = request.header(name, value);
        }
        request
            .body(Body::from(
                json!({"model": "fake/fake-chat", "max_tokens": 10,
                       "messages": [{"role": "user", "content": "hi"}],
                       "contents": [{"role": "user", "parts": [{"text": "hi"}]}]})
                .to_string(),
            ))
            .unwrap()
    };
    for request in [
        run(
            "/v1/messages",
            Some(("authorization", format!("Bearer {key}"))),
        ),
        run("/v1/messages", Some(("x-api-key", key.clone()))),
        run(
            &format!("/v1beta/models/fake/fake-chat:generateContent?key={key}"),
            None,
        ),
        run("/v1/messages", Some(("x-api-key", "pansy".into()))),
    ] {
        let response = gateway.router().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
    let projects: Vec<String> = pansy_core::usage::recent(1)
        .into_iter()
        .skip(4)
        .map(|r| r.project)
        .collect();
    assert_eq!(projects, ["/code/shop", "/code/shop", "/code/shop", ""]);
    assert_eq!(
        pansy_core::usage::summary(1).by_project["/code/shop"].requests,
        3
    );

    // A subagent of Codex's goes on the model Codex sends for its subagents,
    // whatever its lead named for it; the lead's own turns stay where they are.
    let codex = |model: &str, headers: &[(&str, &str)], input: Value| {
        let mut request = Request::post("/v1/responses")
            .header("content-type", "application/json")
            .header("user-agent", "codex_cli_rs/0.169.0");
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        request
            .body(Body::from(
                json!({"model": model, "input": input}).to_string(),
            ))
            .unwrap()
    };
    let spawned = ("x-openai-subagent", "collab_spawn");
    let on_daily = (pansy_gateway::server::SUBAGENT_MODEL, "group/daily");
    let last = || gateway.trace().routes().pop().unwrap();
    let response = gateway
        .router()
        .oneshot(codex("gpt-5.6-sol", &[spawned, on_daily], json!("hi")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let route = last();
    assert!(route.subagent);
    assert_eq!(
        (
            route.requested.as_str(),
            route.asked.as_str(),
            route.kept.as_str()
        ),
        ("group/daily", "gpt-5.6-sol", "")
    );
    assert_eq!(route.group.as_ref().map(|g| g.id.as_str()), Some("daily"));
    let record = pansy_core::usage::recent(1).pop().unwrap();
    assert!(record.subagent);
    assert_eq!(
        (record.requested.as_str(), record.model.as_str()),
        ("group/daily", "fake-chat")
    );
    // The lead's turn, with the same header: not a subagent's, not moved.
    let response = gateway
        .router()
        .oneshot(codex("fake/fake-chat", &[on_daily], json!("hi")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let route = last();
    assert!(!route.subagent);
    assert_eq!(
        (route.requested.as_str(), route.asked.as_str()),
        ("fake/fake-chat", "")
    );
    assert!(!pansy_core::usage::recent(1).pop().unwrap().subagent);
    // A task its lead's vendor sealed opens there alone: it stays, and says why.
    let sealed = json!([{"type": "agent_message", "content": [
        {"type": "encrypted_content", "encrypted_content": "gAAAA-sealed"}]},
        {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hi"}]}]);
    let response = gateway
        .router()
        .oneshot(codex("fake/fake-chat", &[spawned, on_daily], sealed))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let route = last();
    assert!(route.subagent);
    assert_eq!(
        (route.requested.as_str(), route.asked.as_str()),
        ("fake/fake-chat", "")
    );
    assert!(route.kept.contains("sealed"), "{}", route.kept);
    // So does one whose model nobody serves now.
    let gone = (pansy_gateway::server::SUBAGENT_MODEL, "nobody/none");
    let response = gateway
        .router()
        .oneshot(codex("fake/fake-chat", &[spawned, gone], json!("hi")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let route = last();
    assert_eq!(route.requested, "fake/fake-chat");
    assert!(
        route.kept.starts_with("nobody/none can't take it"),
        "{}",
        route.kept
    );
    // Claude Code names its subagent; it sets their model itself, so the
    // request is marked and goes where it asked.
    let request = Request::post("/v1/messages")
        .header("content-type", "application/json")
        .header("user-agent", "claude-cli/2.1.326")
        .header("x-claude-code-agent-id", "a1b2")
        .body(Body::from(
            json!({"model": "fake/fake-chat", "max_tokens": 10,
                   "messages": [{"role": "user", "content": "hi"}]})
            .to_string(),
        ))
        .unwrap();
    assert_eq!(
        gateway.router().oneshot(request).await.unwrap().status(),
        StatusCode::OK
    );
    let route = last();
    assert!(route.subagent);
    assert_eq!(
        (
            route.agent.as_str(),
            route.asked.as_str(),
            route.kept.as_str()
        ),
        ("claude-code", "", "")
    );

    // A web page elsewhere is refused; one on this computer is let in.
    let from = |origin: &str| {
        Request::post("/v1/chat/completions")
            .header("content-type", "application/json")
            .header("origin", origin)
            .body(Body::from(
                json!({"model": "fake-chat", "messages": [{"role": "user", "content": "hi"}]})
                    .to_string(),
            ))
            .unwrap()
    };
    let response = gateway
        .router()
        .oneshot(from("https://evil.example"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = gateway
        .router()
        .oneshot(from("http://localhost:5173"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["access-control-allow-origin"],
        "http://localhost:5173"
    );

    // Masking: the key goes to the vendor as a placeholder, and comes back.
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let echo_addr = echo_vendor(Arc::clone(&seen)).await;
    let mut echo = Provider {
        id: "echo".into(),
        name: "Echo".into(),
        preset: None,
        endpoints: vec![Endpoint {
            protocol: Protocol::Chat,
            base_url: format!("http://{echo_addr}/v1"),
        }],
        accounts: Vec::new(),
        models: vec![pansy_core::provider::Model::new("echo")],
        manual_models: Vec::new(),
        hidden_models: Vec::new(),
        enabled: true,
        headers: Default::default(),
        proxy: None,
        auth_header: None,
        quirks: Default::default(),
        subscription: None,
        plugin: None,
    };
    echo.add_account("sk-echo", "echo");
    let mut providers = pansy_core::config::load_providers().unwrap();
    providers.push(echo);
    save_providers(&providers).unwrap();
    pansy_core::config::update_settings(|s| {
        s.set("masking", r#"{"secrets": true}"#)
            .map_err(anyhow::Error::msg)
    })
    .unwrap();
    let gateway = Gateway::new();
    let key = "sk-proj-Q2xhdWRlIGNvZGUgbWFza3MgdGhpcyBrZXk0V3Nz";
    let said = format!("deploy with OPENAI_API_KEY={key} now");
    for stream in [true, false] {
        let (status, text) = call(
            &gateway,
            "/v1/chat/completions",
            "codex_cli_rs/0.60.0",
            json!({"model": "echo/echo", "stream": stream,
                   "messages": [{"role": "user", "content": said}]}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{text}");
        let vendor_saw = seen.lock().unwrap().last().cloned().unwrap();
        assert!(
            !vendor_saw.contains(key),
            "the vendor saw the key: {vendor_saw}"
        );
        assert!(vendor_saw.contains("{{"), "{vendor_saw}");
        // The agent reads the key again, whole, however the reply was cut.
        let reply: String = if stream {
            text.lines()
                .filter_map(|l| l.strip_prefix("data: "))
                .filter_map(|d| serde_json::from_str::<Value>(d).ok())
                .filter_map(|v| {
                    v["choices"][0]["delta"]["content"]
                        .as_str()
                        .map(str::to_owned)
                })
                .collect()
        } else {
            serde_json::from_str::<Value>(&text).unwrap()["choices"][0]["message"]["content"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        assert_eq!(reply, format!("You said: {said}"), "{text}");
    }
    let masked = pansy_core::masking::summary(1);
    assert_eq!(masked.requests, 2, "{masked:#?}");
    assert_eq!(masked.distinct, 1);
    let record = &masked.records[0];
    assert_eq!(record.agent, "codex");
    assert_eq!(record.provider, "echo");
    assert!(!serde_json::to_string(record).unwrap().contains(key));
    assert_eq!(record.back.values().sum::<usize>(), 1, "{record:#?}");

    // An effort after a model's name (Claude Code's tiers: a/x:high) is the
    // request's, passed through or translated; the model is asked by its name.
    let saw = || serde_json::from_str::<Value>(&seen.lock().unwrap().last().cloned().unwrap());
    let (status, text) = call(
        &gateway,
        "/v1/chat/completions",
        "opencode/1.0",
        json!({"model": "echo/echo:low", "messages": [{"role": "user", "content": "hi"}]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let sent = saw().unwrap();
    assert_eq!(
        (&sent["model"], &sent["reasoning_effort"]),
        (&json!("echo"), &json!("low"))
    );
    let (status, text) = call(
        &gateway,
        "/v1/messages",
        "claude-cli/2.1.326",
        json!({"model": "echo/echo:high", "max_tokens": 100,
               "thinking": {"type": "enabled", "budget_tokens": 2000},
               "messages": [{"role": "user", "content": "hi"}]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    // (Translated to a vendor's chat API, an effort goes only to those
    // that take one.)
    assert_eq!(saw().unwrap()["model"], "echo");
    assert_eq!(
        pansy_core::usage::recent(1).pop().unwrap().requested,
        "echo/echo"
    );
    // What ends a name that is no effort is the model's own name.
    let (status, text) = call(
        &gateway,
        "/v1/chat/completions",
        "opencode/1.0",
        json!({"model": "echo/echo:free", "messages": [{"role": "user", "content": "hi"}]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let sent = saw().unwrap();
    assert_eq!(
        (&sent["model"], &sent["reasoning_effort"]),
        (&json!("echo:free"), &Value::Null)
    );

    // A vendor that refuses the key in both headers at once is asked again
    // with each alone, and the one it takes is kept for the next request.
    let picky_hits = Arc::new(AtomicUsize::new(0));
    let picky_addr = picky_anthropic_vendor(Arc::clone(&picky_hits)).await;
    let mut picky = Provider {
        id: "picky".into(),
        name: "Picky".into(),
        endpoints: vec![Endpoint {
            protocol: Protocol::Anthropic,
            base_url: format!("http://{picky_addr}/anthropic"),
        }],
        models: vec![pansy_core::provider::Model::new("picky-1")],
        ..pansy_core::config::load_providers().unwrap()[0].clone()
    };
    picky.accounts.clear();
    picky.add_account("sk-picky", "");
    let mut providers = pansy_core::config::load_providers().unwrap();
    providers.push(picky);
    save_providers(&providers).unwrap();
    pansy_core::config::update_settings(|s| {
        s.set("masking", r#"{"secrets": false}"#)
            .map_err(anyhow::Error::msg)
    })
    .unwrap();
    let gateway = Gateway::new();
    let ask = json!({"model": "picky/picky-1", "max_tokens": 5, "messages": [{"role": "user", "content": "hi"}]});
    let (status, text) = call(&gateway, "/v1/messages", "claude-cli/2.0.0", ask.clone()).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(picky_hits.load(Ordering::SeqCst), 2);
    let learned = || {
        pansy_core::config::load_providers()
            .unwrap()
            .into_iter()
            .find(|p| p.id == "picky")
            .unwrap()
            .auth_header
    };
    for _ in 0..50 {
        if learned().is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(learned(), Some(pansy_core::provider::AuthHeader::XApiKey));
    let gateway = Gateway::new();
    let (status, text) = call(&gateway, "/v1/messages", "claude-cli/2.0.0", ask).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(picky_hits.load(Ordering::SeqCst), 3);

    chatgpt_through_codexs_sign_in(home.path()).await;
    gemini_through_code_assist(home.path()).await;
}

/// A fake Code Assist: the account's project, and an answer streamed in
/// Code Assist's envelope to a request in it.
async fn fake_code_assist(
    seen: Arc<std::sync::Mutex<Vec<(String, HeaderMap, Value)>>>,
) -> SocketAddr {
    let app = Router::new().fallback(post(
        move |uri: axum::http::Uri, headers: HeaderMap, body: axum::Json<Value>| {
            let seen = Arc::clone(&seen);
            async move {
                let path = uri.path().to_owned();
                seen.lock().unwrap().push((path.clone(), headers, body.0));
                match path.as_str() {
                    "/v1internal:loadCodeAssist" => axum::Json(json!({
                        "currentTier": {"id": "standard-tier", "name": "Gemini Code Assist Standard"},
                        "cloudaicompanionProject": "proj-1",
                    }))
                    .into_response(),
                    "/v1internal:streamGenerateContent" => {
                        let chunks = [
                            json!({"response": {"responseId": "r1", "modelVersion": "gemini-2.5-pro",
                                "candidates": [{"content": {"role": "model", "parts": [{"text": "Hello from "}]}}]}, "traceId": "t"}),
                            json!({"response": {"candidates": [{"content": {"role": "model", "parts": [{"text": "Code Assist"}]},
                                "finishReason": "STOP"}],
                                "usageMetadata": {"promptTokenCount": 6, "candidatesTokenCount": 3}}, "traceId": "t"}),
                        ];
                        let text: String = chunks.iter().map(|c| format!("data: {c}\r\n\r\n")).collect();
                        ([("content-type", "text/event-stream")], text).into_response()
                    }
                    _ => StatusCode::NOT_FOUND.into_response(),
                }
            }
        },
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

/// Gemini CLI is signed in to Google: its Code Assist is a provider, which
/// an OpenAI client streams from and a Gemini client gets a whole answer of.
async fn gemini_through_code_assist(home: &std::path::Path) {
    let later = chrono::Utc::now().timestamp_millis() + 86_400_000;
    std::fs::create_dir_all(home.join(".gemini")).unwrap();
    std::fs::write(
        home.join(".gemini/oauth_creds.json"),
        json!({"access_token": "ya29.live", "refresh_token": "1//r", "expiry_date": later})
            .to_string(),
    )
    .unwrap();
    std::fs::write(
        home.join(".gemini/google_accounts.json"),
        r#"{"active":"ada@example.com"}"#,
    )
    .unwrap();
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let addr = fake_code_assist(Arc::clone(&seen)).await;
    let mut providers = pansy_core::config::load_providers().unwrap();
    let gemini = providers
        .iter_mut()
        .find(|p| p.id == "gemini-cli")
        .expect("Gemini CLI's sign-in is a provider");
    gemini.endpoints[0].base_url = format!("http://{addr}");
    save_providers(&providers).unwrap();

    let gateway = Gateway::new();
    let (status, text) = call(
        &gateway,
        "/v1/chat/completions",
        "opencode/1.0",
        json!({"model": "gemini-cli/gemini-2.5-pro", "stream": true,
               "messages": [{"role": "system", "content": "Be brief."}, {"role": "user", "content": "hi"}]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let said: String = text
        .lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .filter_map(|d| serde_json::from_str::<Value>(d).ok())
        .filter_map(|v| {
            v["choices"][0]["delta"]["content"]
                .as_str()
                .map(str::to_owned)
        })
        .collect();
    assert_eq!(said, "Hello from Code Assist", "{text}");
    let (path, headers, body) = seen.lock().unwrap().last().cloned().unwrap();
    assert_eq!(path, "/v1internal:streamGenerateContent");
    assert_eq!(headers["authorization"], "Bearer ya29.live");
    assert!(
        headers["user-agent"]
            .to_str()
            .unwrap()
            .starts_with("GeminiCLI/")
    );
    assert_eq!(
        (body["model"].as_str(), body["project"].as_str()),
        (Some("gemini-2.5-pro"), Some("proj-1"))
    );
    assert!(body["user_prompt_id"].is_string());
    assert_eq!(body["request"]["contents"][0]["parts"][0]["text"], "hi");
    assert_eq!(
        body["request"]["systemInstruction"]["parts"][0]["text"],
        "Be brief."
    );

    // A Gemini client asking for a whole answer: streamed, gathered, and
    // given back unwrapped.
    let (status, text) = call(
        &gateway,
        "/v1beta/models/gemini-cli%2Fgemini-2.5-pro:generateContent",
        "GeminiCLI/0.61.0",
        json!({"contents": [{"role": "user", "parts": [{"text": "hi"}]}]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let answer: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        answer["candidates"][0]["content"]["parts"][0]["text"], "Hello from Code Assist",
        "{text}"
    );
    // The project was asked for once.
    let loads = seen
        .lock()
        .unwrap()
        .iter()
        .filter(|(p, _, _)| p.ends_with("loadCodeAssist"))
        .count();
    assert_eq!(loads, 1);
}

fn jwt(claims: Value) -> String {
    use base64::Engine;
    let b64 = |v: Value| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v.to_string());
    format!("{}.{}.sig", b64(json!({"alg": "none"})), b64(claims))
}

/// A fake Codex backend: it answers in a stream alone, to a request signed
/// with the account's token and made as Codex makes it.
async fn fake_codex_backend(seen: Arc<std::sync::Mutex<Vec<(HeaderMap, Value)>>>) -> SocketAddr {
    let app = Router::new()
        .route(
            "/backend-api/wham/usage",
            axum::routing::get(|headers: HeaderMap| async move {
                assert_eq!(headers["chatgpt-account-id"], "acct-1");
                let resets = chrono::Utc::now().timestamp() + 3600;
                axum::Json(json!({"plan_type": "plus", "rate_limit": {"allowed": false,
                    "primary_window": {"used_percent": 100.0, "limit_window_seconds": 18000, "reset_at": resets},
                    "secondary_window": {"used_percent": 40.0, "limit_window_seconds": 604800, "reset_at": resets + 86400}},
                    "credits": {"has_credits": false}}))
            }),
        )
        .route(
        "/backend-api/codex/responses",
        post(move |headers: HeaderMap, body: axum::Json<Value>| {
            let seen = Arc::clone(&seen);
            async move {
                seen.lock().unwrap().push((headers, body.0.clone()));
                if body["stream"] != true {
                    return (StatusCode::BAD_REQUEST, "Stream must be set to true").into_response();
                }
                let events = [
                    json!({"type": "response.created", "response": {"id": "resp_1", "model": "gpt-5.5"}}),
                    json!({"type": "response.output_item.added", "output_index": 0,
                           "item": {"type": "message", "id": "m1", "role": "assistant", "content": []}}),
                    json!({"type": "response.output_text.delta", "output_index": 0, "delta": "Hello from ChatGPT"}),
                    json!({"type": "response.output_item.done", "output_index": 0,
                           "item": {"type": "message", "id": "m1", "role": "assistant",
                                    "content": [{"type": "output_text", "text": "Hello from ChatGPT"}]}}),
                    json!({"type": "response.completed", "response": {"id": "resp_1", "status": "completed",
                           "usage": {"input_tokens": 9, "output_tokens": 4}}}),
                ];
                let text: String = events
                    .iter()
                    .map(|e| format!("event: {}\ndata: {e}\n\n", e["type"].as_str().unwrap()))
                    .collect();
                ([("content-type", "text/event-stream")], text).into_response()
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

/// Codex is signed in to ChatGPT: the subscription is a provider, and an
/// agent of another protocol, asking for a whole answer, is served by it.
async fn chatgpt_through_codexs_sign_in(home: &std::path::Path) {
    let later = chrono::Utc::now().timestamp() + 86400;
    let access = jwt(json!({"exp": later}));
    let auth = json!({
        "auth_mode": "chatgpt",
        "tokens": {
            "id_token": jwt(json!({"email": "ada@example.com",
                "https://api.openai.com/auth": {"chatgpt_plan_type": "plus", "chatgpt_account_id": "acct-1"}})),
            "access_token": access,
            "refresh_token": "rt-1",
        },
    });
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    std::fs::write(home.join(".codex/auth.json"), auth.to_string()).unwrap();
    let mut providers = pansy_core::config::load_providers().unwrap();
    let codex = providers
        .iter_mut()
        .find(|p| p.id == "codex")
        .expect("Codex's sign-in is a provider");
    assert_eq!(codex.accounts[0].display(), "ada@example.com");
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let addr = fake_codex_backend(Arc::clone(&seen)).await;
    codex.endpoints[0].base_url = format!("http://{addr}/backend-api/codex");
    codex.models = vec![pansy_core::provider::Model::new("gpt-5.5")];
    save_providers(&providers).unwrap();

    let gateway = Gateway::new();
    let (status, text) = call(
        &gateway,
        "/v1/messages",
        "claude-cli/2.0.0",
        json!({"model": "codex/gpt-5.5", "max_tokens": 50, "system": "Be brief.",
               "messages": [{"role": "user", "content": "hi"}]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let answer: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(answer["content"][0]["text"], "Hello from ChatGPT", "{text}");
    assert_eq!(answer["usage"]["output_tokens"], 4);
    let (headers, body) = seen.lock().unwrap().last().cloned().unwrap();
    assert_eq!(
        headers["authorization"],
        format!("Bearer {access}").as_str()
    );
    assert_eq!(headers["chatgpt-account-id"], "acct-1");
    assert_eq!(headers["originator"], "codex_cli_rs");
    assert!(
        headers["user-agent"]
            .to_str()
            .unwrap()
            .starts_with("codex_cli_rs/")
    );
    assert_eq!(
        (body["store"].clone(), body["stream"].clone()),
        (json!(false), json!(true))
    );
    assert!(body["instructions"].as_str().unwrap().contains("Codex CLI"));
    assert_eq!(body["input"][0]["role"], "developer");
    assert!(body.get("max_output_tokens").is_none());
    // The usage record names the account.
    let last = pansy_core::usage::recent(1).pop().unwrap();
    assert_eq!(
        (last.provider.as_str(), last.account.as_str()),
        ("codex", "ada@example.com")
    );

    // Its allowance: the 5 hours spent, so the account is held until then.
    let all = pansy_core::subscription::allowance::allowances(true).await;
    let a = all.iter().find(|a| a.provider == "codex").unwrap();
    assert_eq!(a.error, "");
    assert_eq!(
        a.windows.iter().map(|w| w.used).collect::<Vec<_>>(),
        [100.0, 40.0]
    );
    let held = a.held_for().unwrap().as_secs();
    assert!((3500..=3600).contains(&held), "{held}");
}

/// A fake Anthropic API that takes its key in x-api-key alone, and says so
/// when it is sent in Authorization too.
async fn picky_anthropic_vendor(hits: Arc<AtomicUsize>) -> SocketAddr {
    let app = Router::new().route(
        "/anthropic/v1/messages",
        post(move |headers: HeaderMap| {
            let hits = Arc::clone(&hits);
            async move {
                hits.fetch_add(1, Ordering::SeqCst);
                if headers.contains_key("authorization") && headers.contains_key("x-api-key") {
                    let refused = json!({"error": {"message": "鉴权 header， x-api-key 和 Authorization 不可以同时存在"}});
                    return (StatusCode::UNAUTHORIZED, axum::Json(refused)).into_response();
                }
                if headers.get("x-api-key").is_none_or(|k| k != "sk-picky") {
                    return (StatusCode::UNAUTHORIZED, axum::Json(json!({"error": {"message": "no key"}}))).into_response();
                }
                axum::Json(json!({
                    "id": "msg_1", "type": "message", "role": "assistant", "model": "picky-1",
                    "content": [{"type": "text", "text": "ok"}], "stop_reason": "end_turn",
                    "usage": {"input_tokens": 1, "output_tokens": 1},
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
