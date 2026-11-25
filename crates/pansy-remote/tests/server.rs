//! The remote interface over HTTP and its WebSocket, as a browser on the
//! phone uses it: paired by a one-time secret with a cookie session, a
//! ticket for the socket, and the protocol's methods. Everything in a
//! scratch home; no agent is started.

use std::net::SocketAddr;
use std::sync::Arc;

use futures::{SinkExt, StreamExt};
use pansy_remote::auth::{Auth, PAIRING_TTL};
use pansy_remote::hub::Hub;
use pansy_remote::server::{App, router};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

async fn call<S>(ws: &mut S, id: u64, method: &str, params: Value) -> Value
where
    S: futures::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + futures::Sink<Message, Error = tokio_tungstenite::tungstenite::Error>
        + Unpin,
{
    let req = json!({"id": id, "method": method, "params": params});
    ws.send(Message::text(req.to_string())).await.unwrap();
    loop {
        let Some(Ok(Message::Text(text))) = ws.next().await else {
            panic!("the socket closed waiting for {method}");
        };
        let v: Value = serde_json::from_str(&text).unwrap();
        if v["id"] == json!(id) {
            return v;
        }
    }
}

#[tokio::test]
async fn pairs_a_browser_and_answers_on_its_socket() {
    let home = tempfile::tempdir().unwrap();
    // SAFETY: the only test in this binary; set before anything reads them.
    unsafe {
        std::env::set_var("PANSY_HOME", home.path().join("pansy"));
        std::env::set_var("HOME", home.path());
    }
    let auth = Arc::new(Auth::load().unwrap());
    let app = Arc::new(App {
        hub: Hub::load().unwrap(),
        auth: Arc::clone(&auth),
        web: None,
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            router(app).into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let base = format!("http://{addr}");
    let http = reqwest::Client::builder().no_proxy().build().unwrap();

    let d: Value = http
        .get(format!("{base}/.well-known/pansy/environment"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(d["protocol"], 1);
    assert_eq!(d["id"], json!(auth.machine_id()));

    // Paired once, with a cookie: the secret is spent.
    let (secret, _) = auth.create_pairing(None, PAIRING_TTL).unwrap();
    let pair = |secret: String| {
        http.post(format!("{base}/api/auth/pair"))
            .json(&json!({"secret": secret, "deviceName": "Test phone", "platform": "web", "cookie": true}))
            .send()
    };
    let res = pair(secret.clone()).await.unwrap();
    assert!(res.status().is_success());
    let cookie = res.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let body: Value = res.json().await.unwrap();
    assert_eq!(body["approval"], "approved");
    assert!(
        body.get("accessToken").is_none(),
        "a cookie session has no token"
    );
    assert_eq!(pair(secret).await.unwrap().status(), 401);

    // Without the cookie there is no ticket.
    let none = http
        .post(format!("{base}/api/auth/ws-ticket"))
        .send()
        .await
        .unwrap();
    assert_eq!(none.status(), 401);
    let ticket: Value = http
        .post(format!("{base}/api/auth/ws-ticket"))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ticket = ticket["ticket"].as_str().unwrap();

    let mut req = format!("ws://{addr}/ws?ticket={ticket}")
        .into_client_request()
        .unwrap();
    req.headers_mut().insert("cookie", cookie.parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();

    let shell = call(&mut ws, 1, "shell.subscribe", json!({})).await;
    assert_eq!(shell["result"]["machine"]["id"], json!(auth.machine_id()));
    assert!(shell["result"]["threads"].is_array());

    let missing = call(
        &mut ws,
        2,
        "git.status",
        json!({"threadId": "t_000000000000"}),
    )
    .await;
    assert_eq!(missing["error"]["code"], "not_found");

    let unknown = call(&mut ws, 3, "no.such", json!({})).await;
    assert_eq!(unknown["error"]["code"], "bad_request");

    let empty = call(
        &mut ws,
        4,
        "thread.create",
        json!({"folder": home.path(), "agent": "codex", "text": "  "}),
    )
    .await;
    assert_eq!(empty["error"]["code"], "bad_request");

    let devices = call(&mut ws, 5, "devices.list", json!({})).await;
    let list = devices["result"]["devices"].as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["name"], "Test phone");
    assert_eq!(list[0]["current"], true);

    // What the gateway counted, by day and by model, never a request's text.
    pansy_core::usage::append(&pansy_core::usage::Record {
        at: chrono::Utc::now(),
        agent: "codex".into(),
        provider: "deepseek".into(),
        model: "deepseek-chat".into(),
        status: 200,
        input: 120,
        output: 30,
        ..Default::default()
    })
    .unwrap();
    let usage = call(&mut ws, 7, "usage.summary", json!({"days": 7})).await;
    let usage = &usage["result"];
    assert_eq!(usage["total"]["requests"], 1);
    assert_eq!(usage["byModel"]["deepseek/deepseek-chat"]["input"], 120);
    assert_eq!(usage["byAgent"]["codex"]["output"], 30);
    assert_eq!(usage["byDay"].as_object().unwrap().len(), 7);
    assert!(usage.get("latest").is_none());

    // Revoked, the device is cut off and its cookie is worth nothing.
    let id = list[0]["id"].as_str().unwrap().to_owned();
    let gone = call(&mut ws, 6, "devices.revoke", json!({"deviceId": id})).await;
    assert!(gone.get("error").is_none(), "{gone}");
    let after = http
        .post(format!("{base}/api/auth/ws-ticket"))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(after.status(), 401);
}
