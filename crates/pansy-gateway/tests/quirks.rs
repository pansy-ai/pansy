//! `quirks.toml` through the gateway: a rule changes a request the gateway
//! translated for the vendor, and leaves one in the vendor's own protocol
//! as the agent sent it. In a scratch `PANSY_HOME`.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::post;
use pansy_core::config::save_providers;
use pansy_core::provider::{Endpoint, Model, Protocol, Provider};
use pansy_gateway::Gateway;
use serde_json::{Value, json};
use tower::ServiceExt;

/// A Chat vendor that keeps the requests it got.
async fn vendor(seen: Arc<Mutex<Vec<Value>>>) -> SocketAddr {
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |body: axum::Json<Value>| {
            let seen = Arc::clone(&seen);
            async move {
                seen.lock().unwrap().push(body.0);
                axum::Json(json!({
                    "id": "c1", "model": "big-1",
                    "choices": [{"index": 0, "message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}],
                    "usage": {"prompt_tokens": 1, "completion_tokens": 1},
                }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

#[tokio::test]
async fn a_rule_changes_a_translated_request_and_no_other() {
    let home = tempfile::tempdir().unwrap();
    // SAFETY: the only test in this binary; nothing else reads the env.
    unsafe {
        std::env::set_var("PANSY_HOME", home.path());
        std::env::set_var("HOME", home.path());
    }
    std::fs::write(
        home.path().join("quirks.toml"),
        "[[rule]]\nprovider = \"acme\"\nmodel = \"big-*\"\ndrop = [\"temperature\"]\nmost = { max_tokens = 100 }\nwhy = \"acme refuses a temperature\"\n",
    )
    .unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let addr = vendor(Arc::clone(&seen)).await;
    let mut acme: Provider = serde_json::from_value(json!({
        "id": "acme", "name": "Acme",
        "endpoints": [],
    }))
    .unwrap();
    acme.endpoints = vec![Endpoint {
        protocol: Protocol::Chat,
        base_url: format!("http://{addr}/v1"),
    }];
    acme.models = vec![Model::new("big-1"), Model::new("small-1")];
    acme.add_account("sk-acme", "acme");
    save_providers(&[acme]).unwrap();
    let gateway = Gateway::new();
    let send = |path: &'static str, body: Value| {
        let gateway = Arc::clone(&gateway);
        async move {
            let request = Request::post(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap();
            let status = gateway.router().oneshot(request).await.unwrap().status();
            assert_eq!(status, StatusCode::OK);
        }
    };
    let last = || seen.lock().unwrap().last().cloned().unwrap();

    // Claude Code's request, translated to Chat: the rule's keys change.
    let messages = json!([{"role": "user", "content": "hi"}]);
    send(
        "/v1/messages",
        json!({"model": "acme/big-1", "max_tokens": 5000, "temperature": 0.2, "messages": messages}),
    )
    .await;
    let got = last();
    assert!(got.get("temperature").is_none(), "{got}");
    assert_eq!(got["max_tokens"], 100);
    assert_eq!(got["model"], "big-1");

    // A model the rule is not for: translated, and as asked.
    send(
        "/v1/messages",
        json!({"model": "acme/small-1", "max_tokens": 5000, "temperature": 0.2, "messages": messages}),
    )
    .await;
    let got = last();
    assert_eq!(
        (got["temperature"].as_f64(), got["max_tokens"].as_u64()),
        (Some(0.2), Some(5000))
    );

    // The vendor's own protocol: passed through as the agent sent it.
    send(
        "/v1/chat/completions",
        json!({"model": "acme/big-1", "max_tokens": 5000, "temperature": 0.2, "messages": messages}),
    )
    .await;
    let got = last();
    assert_eq!(
        (got["temperature"].as_f64(), got["max_tokens"].as_u64()),
        (Some(0.2), Some(5000))
    );
}
