//! magpie's community plugins through the gateway, on Bun: a provider a
//! plugin serves (magpie's fake, signed in with a key), a middleware on the
//! request, each try, the stream and the reply, and request hooks. Passes
//! without running when there is no Bun (`PANSY_BUN`, or `bun` on `PATH`).
//! Everything is under a scratch `HOME` and `PANSY_HOME`.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use pansy_gateway::Gateway;
use pansy_plugins::store::{Entry, List};
use serde_json::{Value, json};
use tower::ServiceExt;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../pansy-plugins/tests/fixtures")
        .join(name)
}

fn bun() -> Option<PathBuf> {
    if let Some(b) = std::env::var_os("PANSY_BUN").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(b));
    }
    let found = std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join("bun"))
        .find(|p| p.is_file())?;
    // SAFETY: the only test in this binary.
    unsafe { std::env::set_var("PANSY_BUN", &found) };
    Some(found)
}

/// A vendor that echoes the last message and the key it was sent.
async fn vendor(seen: Arc<Mutex<Vec<Value>>>) -> SocketAddr {
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |headers: HeaderMap, body: axum::Json<Value>| {
            let seen = Arc::clone(&seen);
            async move {
                let auth = headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or_default()
                    .to_owned();
                seen.lock().unwrap().push(json!({"auth": auth, "body": body.0}));
                if body["stream"] == true {
                    let chunks = [
                        json!({"choices": [{"delta": {"role": "assistant", "content": "quiet words"}}]}),
                        json!({"choices": [{"delta": {}, "finish_reason": "stop"}], "usage": {"prompt_tokens": 3, "completion_tokens": 2}}),
                    ];
                    let mut text: String = chunks.iter().map(|c| format!("data: {c}\n\n")).collect();
                    text.push_str("data: [DONE]\n\n");
                    return ([("content-type", "text/event-stream")], text).into_response();
                }
                axum::Json(json!({
                    "id": "p1", "model": "fake-1",
                    "choices": [{"index": 0, "message": {"role": "assistant", "content": "plain reply"}, "finish_reason": "stop"}],
                    "usage": {"prompt_tokens": 3, "completion_tokens": 2},
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

async fn call(gateway: &Arc<Gateway>, body: Value) -> (StatusCode, String) {
    let request = Request::post("/v1/chat/completions")
        .header("content-type", "application/json")
        .header("user-agent", "opencode/1.0")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = gateway.router().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

fn entry(path: PathBuf, options: Value) -> Entry {
    Entry {
        spec: path.display().to_string(),
        options: options.as_object().cloned().filter(|o| !o.is_empty()),
        ..Entry::default()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn plugins_serve_and_shape_requests_through_the_gateway() {
    let home = tempfile::tempdir().unwrap();
    // SAFETY: the only test in this binary; nothing else reads the env.
    unsafe {
        std::env::set_var("PANSY_HOME", home.path().join("pansy"));
        std::env::set_var("HOME", home.path());
        std::env::remove_var("XDG_CONFIG_HOME");
    }
    if bun().is_none() {
        eprintln!("no Bun: skipped");
        return;
    }
    let seen = Arc::new(Mutex::new(Vec::new()));
    let addr = vendor(Arc::clone(&seen)).await;
    // SAFETY: as above.
    unsafe { std::env::set_var("FAKE_BASE", format!("http://{addr}/v1")) };
    let usage_file = home.path().join("usage.jsonl");
    let reply_file = home.path().join("reply.json");
    let list = List {
        plugins: vec![
            entry(fixture("fake"), json!({})),
            entry(
                fixture("plugins/guard.middleware.js"),
                json!({"usageFile": usage_file}),
            ),
            entry(
                fixture("plugins/notes.hooks.mjs"),
                json!({"replyFile": reply_file}),
            ),
        ],
        ..List::default()
    };
    pansy_core::fsutil::write_json(&pansy_core::paths::plugins(), &list, true).unwrap();
    pansy_plugins::host::list_now().await.unwrap();
    let flow = pansy_plugins::signin::start("fakeco", 0).await.unwrap();
    let flow = pansy_plugins::signin::answer(&flow.id, "sk-plug")
        .await
        .unwrap();
    assert_eq!(flow.state, "done", "{}", flow.error);

    let gateway = Gateway::new();
    let (status, text) = call(
        &gateway,
        json!({"model": "fakeco/fake-1", "messages": [{"role": "user", "content": "my secret plan"}]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let reply: Value = serde_json::from_str(&text).unwrap();
    // onResponse saw the whole reply.
    assert_eq!(reply["guarded"], 200);
    assert_eq!(reply["choices"][0]["message"]["content"], "plain reply");
    {
        let seen = seen.lock().unwrap();
        let last = seen.last().unwrap();
        // The plugin signed it; the middleware and the hook shaped it.
        assert_eq!(last["auth"], "Bearer sk-plug");
        let body = last["body"].to_string();
        assert!(body.contains("my [guarded] plan"), "{body}");
        assert!(body.contains("note from local"), "{body}");
        assert_eq!(last["body"]["metadata"]["to"], "fakeco");
    }

    let (status, text) = call(
        &gateway,
        json!({"model": "fakeco/fake-1", "stream": true, "messages": [{"role": "user", "content": "hi"}]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // onEvent made the stream loud.
    assert!(text.contains("QUIET WORDS"), "{text}");

    let (status, text) = call(
        &gateway,
        json!({"model": "fakeco/fake-1", "messages": [{"role": "user", "content": "forbidden"}]}),
    )
    .await;
    assert_eq!(status.as_u16(), 451, "{text}");
    assert!(text.contains("no forbidden words"));

    // The hooks heard the stream's reply, whole.
    let mut said = String::new();
    for _ in 0..50 {
        said = std::fs::read_to_string(&reply_file).unwrap_or_default();
        if said.contains("QUIET WORDS") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(said.contains("QUIET WORDS"), "{said}");
    // And onUsage each row.
    let mut rows = String::new();
    for _ in 0..50 {
        rows = std::fs::read_to_string(&usage_file).unwrap_or_default();
        if rows.lines().count() >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(rows.contains("\"provider\":\"fakeco\""), "{rows}");
    pansy_plugins::host::forget();
    pansy_plugins::middleware::forget();
}
