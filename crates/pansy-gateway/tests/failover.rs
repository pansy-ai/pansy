//! What a failure moves a request to: a model that turns the request itself
//! away is passed over for the next model, not asked again on its other
//! keys; a key that hit a model's limit rests for that model alone; and a
//! connection that broke is asked once more before the request moves at
//! all. In a scratch `PANSY_HOME`.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use pansy_core::config::{save_groups, save_providers};
use pansy_core::group::{Group, Routing};
use pansy_core::provider::{Endpoint, Model, Protocol, Provider};
use pansy_gateway::Gateway;
use serde_json::{Value, json};
use tower::ServiceExt;

/// A model, the key it answers so (any when empty), a status and its words.
type Rule = (&'static str, &'static str, u16, &'static str);

const RULES: &[Rule] = &[
    (
        "strict",
        "",
        400,
        "Inappropriate input/output rejected for security reasons",
    ),
    ("down", "", 503, "no capacity"),
    ("limited", "sk-r1", 429, "slow down"),
];

/// A Chat vendor that answers by `RULES`, and keeps the key and model of
/// each request it got.
async fn vendor(seen: Arc<Mutex<Vec<(String, String)>>>) -> SocketAddr {
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |headers: HeaderMap, body: axum::Json<Value>| {
            let seen = Arc::clone(&seen);
            async move {
                let key = headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.strip_prefix("Bearer "))
                    .unwrap_or_default()
                    .to_owned();
                let model = body["model"].as_str().unwrap_or_default().to_owned();
                seen.lock().unwrap().push((key.clone(), model.clone()));
                let rule = RULES
                    .iter()
                    .find(|(m, k, ..)| *m == model && (k.is_empty() || *k == key));
                match rule {
                    Some((_, _, status, words)) => (
                        StatusCode::from_u16(*status).unwrap(),
                        axum::Json(json!({"error": {"message": words}})),
                    )
                        .into_response(),
                    None => axum::Json(json!({
                        "id": "c1", "model": model,
                        "choices": [{"index": 0, "message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 1, "completion_tokens": 1},
                    }))
                    .into_response(),
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

/// A line to the vendor at `to` that breaks: it closes the next `drops`
/// connections made to it unanswered, and carries the ones after.
async fn shaky_line(to: SocketAddr, drops: Arc<AtomicUsize>) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let dropped = drops
                .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .is_ok();
            if dropped {
                continue;
            }
            tokio::spawn(async move {
                let mut vendor = tokio::net::TcpStream::connect(to).await.unwrap();
                let _ = tokio::io::copy_bidirectional(&mut socket, &mut vendor).await;
            });
        }
    });
    addr
}

fn provider(id: &str, addr: SocketAddr, models: &[&str], keys: &[&str]) -> Provider {
    let mut provider: Provider =
        serde_json::from_value(json!({"id": id, "name": id, "endpoints": []})).unwrap();
    provider.endpoints = vec![Endpoint {
        protocol: Protocol::Chat,
        base_url: format!("http://{addr}/v1"),
    }];
    provider.models = models.iter().map(|m| Model::new(m)).collect();
    for key in keys {
        provider.add_account(key, key);
    }
    provider
}

fn group(id: &str, members: &[&str]) -> Group {
    Group {
        id: id.into(),
        name: id.into(),
        members: members.iter().map(|m| (*m).to_owned()).collect(),
        routing: Routing::Order,
        strategy: None,
    }
}

#[tokio::test]
async fn a_failure_moves_a_request_as_far_as_it_reaches() {
    let home = tempfile::tempdir().unwrap();
    // SAFETY: the only test in this binary; nothing else reads the env.
    unsafe {
        std::env::set_var("PANSY_HOME", home.path());
        std::env::set_var("HOME", home.path());
    }
    let seen = Arc::new(Mutex::new(Vec::new()));
    let addr = vendor(Arc::clone(&seen)).await;
    // Two lines to it that break: once, and twice.
    let once = Arc::new(AtomicUsize::new(1));
    let shaky = shaky_line(addr, Arc::clone(&once)).await;
    let twice = Arc::new(AtomicUsize::new(2));
    let cut = shaky_line(addr, Arc::clone(&twice)).await;
    save_providers(&[
        provider("shaky", shaky, &["open"], &["sk-s"]),
        provider("cut", cut, &["open"], &["sk-c"]),
        provider(
            "relay",
            addr,
            &["strict", "limited", "open"],
            &["sk-r1", "sk-r2"],
        ),
        provider("other", addr, &["spare", "down"], &["sk-o"]),
    ])
    .unwrap();
    save_groups(&[
        group("pair", &["relay/strict", "other/spare"]),
        group("sad", &["relay/strict", "other/down"]),
        group("backed", &["shaky/open", "other/spare"]),
        group("spared", &["cut/open", "other/spare"]),
    ])
    .unwrap();
    let gateway = Gateway::new();
    let ask = |model: &'static str| {
        let gateway = Arc::clone(&gateway);
        async move {
            let body = json!({"model": model, "messages": [{"role": "user", "content": "hi"}]});
            let request = Request::post("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap();
            let response = gateway.router().oneshot(request).await.unwrap();
            let status = response.status().as_u16();
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            (status, String::from_utf8(bytes.to_vec()).unwrap())
        }
    };
    // The requests the vendor got since the last look, as `key model`.
    let asked = || -> Vec<String> {
        std::mem::take(&mut *seen.lock().unwrap())
            .into_iter()
            .map(|(key, model)| format!("{key} {model}"))
            .collect()
    };
    let record = || pansy_core::usage::recent(1).pop().unwrap();
    let tried = |r: &pansy_core::usage::Record| -> Vec<String> {
        r.tried
            .iter()
            .map(|t| format!("{}/{} {}", t.provider, t.model, t.status))
            .collect()
    };

    // A model turns the request away: the group's next model answers it,
    // and the first model's other key is not asked.
    let (status, text) = ask("group/pair").await;
    assert_eq!(status, 200, "{text}");
    assert_eq!(asked(), ["sk-r1 strict", "sk-o spare"]);
    let r = record();
    assert_eq!((r.provider.as_str(), r.status), ("other", 200));
    assert_eq!(tried(&r), ["relay/strict 400"]);
    // Nobody rests for it: the request was what the model turned away.
    assert!(gateway.health().resting_keys().is_empty());

    // Asked for alone, the client hears it at once, as it did before.
    let (status, text) = ask("relay/strict").await;
    assert_eq!(status, 400);
    assert!(text.contains("Inappropriate input/output"), "{text}");
    assert_eq!(asked(), ["sk-r1 strict"]);
    let r = record();
    assert_eq!((r.provider.as_str(), r.status), ("relay", 400));
    assert!(tried(&r).is_empty());

    // No other model answers either: the client hears the model that turned
    // the request away, not the failure after it, and the record is its.
    let (status, text) = ask("group/sad").await;
    assert_eq!(status, 400, "{text}");
    assert!(text.contains("Inappropriate input/output"), "{text}");
    assert_eq!(asked(), ["sk-r1 strict", "sk-o down"]);
    let r = record();
    assert_eq!(
        (r.provider.as_str(), r.model.as_str(), r.status),
        ("relay", "strict", 400)
    );
    assert_eq!(tried(&r), ["other/down 503"]);

    // A key at one model's limit: the next key answers, and the first rests
    // for that model alone.
    let (status, text) = ask("relay/limited").await;
    assert_eq!(status, 200, "{text}");
    assert_eq!(asked(), ["sk-r1 limited", "sk-r2 limited"]);
    let resting: Vec<String> = gateway
        .health()
        .resting_keys()
        .into_iter()
        .map(|(key, _)| key)
        .filter(|key| key.starts_with("relay:"))
        .collect();
    assert_eq!(resting.len(), 1, "{resting:?}");
    assert!(resting[0].ends_with(" · limited"), "{resting:?}");
    // Its other model is asked of it first, as before.
    let (status, _) = ask("relay/open").await;
    assert_eq!(status, 200);
    assert_eq!(asked(), ["sk-r1 open"]);
    // The model it rests for goes to the other key first.
    let (status, _) = ask("relay/limited").await;
    assert_eq!(status, 200);
    assert_eq!(asked(), ["sk-r2 limited"]);

    // A connection that breaks once: the same key is asked once more and
    // answers, so the request never leaves for the group's next model, and
    // nobody rests.
    let resting = |provider: &str| -> Vec<String> {
        gateway
            .health()
            .resting_keys()
            .into_iter()
            .map(|(key, _)| key)
            .filter(|key| key.starts_with(provider))
            .collect()
    };
    let (status, text) = ask("group/backed").await;
    assert_eq!(status, 200, "{text}");
    assert_eq!(asked(), ["sk-s open"]);
    let r = record();
    assert_eq!((r.provider.as_str(), r.status), ("shaky", 200));
    assert!(tried(&r).is_empty());
    assert_eq!(once.load(Ordering::SeqCst), 0);
    assert!(resting("shaky:").is_empty());
    let route = gateway.trace().routes().pop().unwrap();
    assert_eq!(
        route.tries.iter().map(|t| t.again).collect::<Vec<_>>(),
        [true]
    );

    // It breaks twice: asked no third time, the key rests and the next
    // model answers.
    let (status, text) = ask("group/spared").await;
    assert_eq!(status, 200, "{text}");
    assert_eq!(asked(), ["sk-o spare"]);
    assert_eq!(twice.load(Ordering::SeqCst), 0);
    let r = record();
    assert_eq!((r.provider.as_str(), r.status), ("other", 200));
    assert_eq!(tried(&r), ["cut/open 502"]);
    assert_eq!(resting("cut:").len(), 1);
}
