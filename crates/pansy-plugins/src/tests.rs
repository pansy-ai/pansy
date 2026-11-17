//! The hosts on Bun, end to end: a provider plugin (magpie's fake), a
//! middleware, request hooks and an agent plugin, each loaded from a path
//! as `plugins.json` names it, against a vendor on loopback. A test passes
//! without running when there is no Bun (`PANSY_BUN`, or `bun` on `PATH`).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::post;
use bytes::Bytes;
use pansy_core::plugin::Listing;
use serde_json::{Value, json};

use crate::middleware::{self, Event, Info, To};
use crate::store::{Entry, List};
use crate::testing::{bun, fixture, scratch};
use crate::{agents, block, host, signin};

/// A vendor on loopback that says which key it was sent.
#[derive(Default)]
struct Vendor {
    seen: Mutex<Vec<String>>,
}

async fn chat(
    State(v): State<Arc<Vendor>>,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    let auth = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_owned();
    v.seen.lock().unwrap().push(auth);
    let req: Value = serde_json::from_slice(&body).unwrap_or_default();
    if req["stream"] == true {
        let events = "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\ndata: [DONE]\n\n";
        return axum::response::Response::builder()
            .header("content-type", "text/event-stream")
            .body(Body::from(events))
            .unwrap();
    }
    axum::response::Response::builder()
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"choices": [{"message": {"role": "assistant", "content": "hello"}}]})
                .to_string(),
        ))
        .unwrap()
}

fn vendor() -> (Arc<Vendor>, String) {
    let v = Arc::new(Vendor::default());
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let app = Router::new()
        .route("/v1/chat/completions", post(chat))
        .with_state(v.clone());
    crate::rt().spawn(async move {
        let listener = tokio::net::TcpListener::from_std(listener).unwrap();
        axum::serve(listener, app).await.unwrap();
    });
    (v, base)
}

fn plugins(entries: Vec<Entry>) {
    let list = List {
        plugins: entries,
        ..List::default()
    };
    pansy_core::fsutil::write_json(&pansy_core::paths::plugins(), &list, true).unwrap();
}

fn path(spec: &std::path::Path) -> Entry {
    Entry {
        spec: spec.display().to_string(),
        ..Entry::default()
    }
}

#[test]
fn a_provider_plugin_signs_in_and_answers() {
    let home = tempfile::tempdir().unwrap();
    let _env = scratch(home.path());
    if bun().is_none() {
        eprintln!("no Bun: skipped");
        return;
    }
    let (vendor, base) = vendor();
    // SAFETY: the ENV guard is held.
    unsafe { std::env::set_var("FAKE_BASE", &base) };
    plugins(vec![path(&fixture("fake"))]);

    let listing = block(host::list_now()).unwrap();
    let fake = listing.providers.iter().find(|p| p.id == "fakeco").unwrap();
    assert!(fake.accounts.is_empty());
    assert_eq!(fake.methods[0].kind, "api");
    assert_eq!(listing.loaded[0].provides, ["fakeco"]);
    // Not signed in: no provider yet.
    assert!(
        pansy_core::config::load_providers()
            .unwrap()
            .iter()
            .all(|p| p.id != "fakeco")
    );

    let flow = block(signin::start("fakeco", 0)).unwrap();
    assert_eq!(flow.state, "key");
    let id = flow.id.clone();
    let flow = block(async move { signin::answer(&id, "sk-spike").await }).unwrap();
    assert_eq!(flow.state, "done", "{}", flow.error);
    let providers = pansy_core::config::load_providers().unwrap();
    let p = providers.iter().find(|p| p.id == "fakeco").unwrap();
    assert_eq!(p.accounts.len(), 1);
    assert_eq!(p.accounts[0].plugin.as_ref().unwrap().key, "fakeco");
    assert!(p.models.iter().any(|m| m.id == "fake-1"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(pansy_core::paths::plugin_auth())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    assert_eq!(
        block(host::base_url("fakeco", "fakeco"))
            .unwrap()
            .as_deref(),
        Some(base.as_str())
    );
    let url = format!("{base}/chat/completions");
    let request = move |stream: bool| host::Request {
        provider: "fakeco".into(),
        account: "fakeco".into(),
        model: "fake-1".into(),
        npm: "@ai-sdk/openai-compatible".into(),
        url: url.clone(),
        headers: vec![("content-type".into(), "application/json".into())],
        body: json!({"model": "fake-1", "stream": stream, "messages": []})
            .to_string()
            .into_bytes(),
        ..host::Request::default()
    };
    let (r1, r2) = (request(false), request(true));
    let whole = block(async move {
        let r = host::fetch(r1).await.unwrap();
        (r.status().as_u16(), r.json::<Value>().await.unwrap())
    });
    assert_eq!(whole.0, 200);
    assert_eq!(whole.1["choices"][0]["message"]["content"], "hello");
    assert_eq!(
        vendor.seen.lock().unwrap().last().unwrap(),
        "Bearer sk-spike"
    );
    let streamed = block(async move { host::fetch(r2).await.unwrap().text().await.unwrap() });
    assert!(streamed.contains("\"hi\"") && streamed.ends_with("[DONE]\n\n"));

    block(host::sign_out("fakeco", None)).unwrap();
    assert!(Listing::load().providers[0].accounts.is_empty());
    assert!(
        pansy_core::config::load_providers()
            .unwrap()
            .iter()
            .all(|p| p.id != "fakeco")
    );
    // SAFETY: the ENV guard is held.
    unsafe { std::env::remove_var("FAKE_BASE") };
}

fn info() -> Info {
    Info {
        protocol: "anthropic".into(),
        model: "x/y".into(),
        stream: true,
        path: "/v1/messages".into(),
        agent: "claude-code".into(),
    }
}

#[test]
fn middleware_runs_every_hook_and_fails_open() {
    let home = tempfile::tempdir().unwrap();
    let _env = scratch(home.path());
    if bun().is_none() {
        eprintln!("no Bun: skipped");
        return;
    }
    let usage_file = home.path().join("usage.jsonl");
    let mut guard = path(&fixture("plugins/guard.middleware.js"));
    guard.options = Some(
        json!({"usageFile": usage_file})
            .as_object()
            .unwrap()
            .clone(),
    );
    plugins(vec![guard]);
    assert!(middleware::wanted());

    block(async move {
        let run = middleware::begin(info()).await.expect("a run");
        let body = json!({"messages": [{"role": "user", "content": "my secret"}]});
        let out = run.request(&body).await.unwrap().unwrap();
        assert_eq!(out["messages"][0]["content"], "my [guarded]");
        // Nothing to change: as it came.
        assert!(
            run.request(&json!({"messages": []}))
                .await
                .unwrap()
                .is_none()
        );
        let no = run
            .request(&json!({"messages": [{"content": "forbidden"}]}))
            .await
            .unwrap_err();
        assert_eq!(
            (no.status, no.message.as_str()),
            (451, "guard: no forbidden words")
        );

        let skip = middleware::Account {
            kind: "codex".into(),
            user: "a@b".into(),
            model: "skip-me".into(),
            ..Default::default()
        };
        assert_eq!(run.account(Some(&skip)).await.as_deref(), Some("guard"));
        let keep = middleware::Account {
            model: "ok".into(),
            ..skip.clone()
        };
        assert_eq!(run.account(Some(&keep)).await, None);

        let to = To {
            id: "deepseek".into(),
            name: "DeepSeek".into(),
            model: "deepseek-chat".into(),
            ..Default::default()
        };
        let sent = run.provider(&json!({"model": "m"}), &to).await.unwrap();
        // ctx.state is the request's own, across its hooks.
        assert_eq!(sent["metadata"], json!({"to": "deepseek", "seen": 3}));

        assert!(run.wants_events() && run.wants_response());
        assert_eq!(
            run.event(
                "content_block_delta",
                &json!({"type": "content_block_delta", "delta": {"text": "hey"}})
            )
            .await,
            Event::Replace(json!({"type": "content_block_delta", "delta": {"text": "HEY"}}))
        );
        assert_eq!(run.event("", &json!({"type": "ping"})).await, Event::Drop);
        // An event it didn't ask for isn't sent to it.
        assert_eq!(
            run.event("message_stop", &json!({"type": "ping"})).await,
            Event::Keep
        );
        assert_eq!(
            run.response(&json!({"id": "r"}), 200).await,
            Some(json!({"id": "r", "guarded": 200}))
        );

        // A hook that throws leaves the request as it came (a new run: the
        // one above was turned away).
        let run = middleware::begin(info()).await.unwrap();
        assert!(
            run.request(&json!({"messages": [{"content": "throw-please secret"}]}))
                .await
                .unwrap()
                .is_none()
        );

        let seats = vec![
            middleware::Seat {
                key: "a".into(),
                ..Default::default()
            },
            middleware::Seat {
                key: "b".into(),
                ..Default::default()
            },
        ];
        assert_eq!(
            middleware::routings().await,
            ["guard/reverse", "guard/broken"]
        );
        assert_eq!(
            middleware::order("guard/reverse", "group/g", &seats).await,
            Some(vec![1, 0])
        );
        assert_eq!(
            middleware::order("guard/broken", "group/g", &seats).await,
            None
        );
        assert_eq!(middleware::order("nobody/x", "group/g", &seats).await, None);
        let states = middleware::states().await.unwrap();
        assert!(states["guard"]["failures"].as_u64().unwrap() >= 2);
    });

    let record = pansy_core::usage::Record {
        at: chrono::Utc::now(),
        agent: "claude-code".into(),
        provider: "deepseek".into(),
        model: "deepseek-chat".into(),
        status: 200,
        input: 12,
        ..Default::default()
    };
    middleware::usage(&record);
    let mut rows = String::new();
    for _ in 0..50 {
        rows = std::fs::read_to_string(&usage_file).unwrap_or_default();
        if !rows.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let row: Value = serde_json::from_str(rows.lines().next().unwrap()).unwrap();
    assert_eq!(
        (row["provider"].as_str(), row["input"].as_u64()),
        (Some("deepseek"), Some(12))
    );
}

#[test]
fn a_middleware_past_its_time_is_let_go_and_the_host_goes_on() {
    let home = tempfile::tempdir().unwrap();
    let _env = scratch(home.path());
    if bun().is_none() {
        eprintln!("no Bun: skipped");
        return;
    }
    let mut guard = path(&fixture("plugins/guard.middleware.js"));
    guard.options = Some(json!({"slow": 400}).as_object().unwrap().clone());
    plugins(vec![guard]);
    block(async move {
        let run = middleware::begin(info()).await.unwrap();
        let started = std::time::Instant::now();
        let out = run
            .request(&json!({"messages": [{"content": "secret"}]}))
            .await
            .unwrap();
        assert!(out.is_none(), "a slow hook's answer isn't taken");
        assert!(started.elapsed() < Duration::from_millis(390));
        // The host answers again once the hook is done.
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(middleware::begin(info()).await.is_some());
    });
}

#[test]
fn request_hooks_add_context_and_hear_the_reply() {
    let home = tempfile::tempdir().unwrap();
    let _env = scratch(home.path());
    if bun().is_none() {
        eprintln!("no Bun: skipped");
        return;
    }
    let reply_file = home.path().join("reply.json");
    let mut hooks = path(&fixture("plugins/notes.hooks.mjs"));
    hooks.options = Some(
        json!({"replyFile": reply_file})
            .as_object()
            .unwrap()
            .clone(),
    );
    plugins(vec![hooks]);
    assert!(host::has_hooks());
    let context = host::HookContext {
        protocol: "chat".into(),
        origin: "local".into(),
        ..Default::default()
    };
    let (body, run) = block(async move {
        host::hooks_begin(&json!({"model": "m", "messages": []}), &context)
            .await
            .unwrap()
    });
    assert_eq!(body["messages"][0]["content"], "note from local");
    run.finish(json!({"text": "done"}), true);
    let mut said = String::new();
    for _ in 0..50 {
        said = std::fs::read_to_string(&reply_file).unwrap_or_default();
        if !said.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let said: Value = serde_json::from_str(&said).unwrap();
    assert_eq!(said, json!({"reply": {"text": "done"}, "success": true}));
}

#[test]
fn an_agent_plugin_says_where_its_model_goes() {
    let home = tempfile::tempdir().unwrap();
    let _env = scratch(home.path());
    if bun().is_none() {
        eprintln!("no Bun: skipped");
        return;
    }
    plugins(vec![path(&fixture("plugins/aider.agent.js"))]);
    let kept = block(agents::reload()).unwrap();
    assert!(kept.errors.is_empty(), "{:?}", kept.errors);
    let aider = &kept.agents[0];
    assert_eq!(
        (
            aider.id.as_str(),
            aider.config.as_str(),
            aider.model.as_str(),
            aider.prefix.as_str()
        ),
        ("aider", "~/.aider.conf.yml", "model", "openai/")
    );
    assert!(aider.connect);
    let keys = block(agents::connect(
        "aider",
        "http://127.0.0.1:7267",
        "pansy",
        Some("deepseek/deepseek-chat"),
        &[],
    ))
    .unwrap();
    assert_eq!(keys["openai-api-base"], "http://127.0.0.1:7267/v1");
    assert_eq!(keys["openai-api-key"], "pansy");
    assert_eq!(keys["show-model-warnings"], false);
}
