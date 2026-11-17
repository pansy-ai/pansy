//! Sign-ins found where agents keep them and where pansy does, served as
//! providers, and requests made the way each backend takes them. Every
//! file is in a scratch home.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use base64::Engine;
use serde_json::{Value, json};

use super::*;
use crate::provider::Protocol;

static ENV: Mutex<()> = Mutex::new(());

/// Points pansy's home and the user's at `dir` (no agent's sign-in found
/// elsewhere), for as long as the guard is held: tests that read the
/// environment take turns.
pub(crate) fn scratch_home(dir: &Path) -> MutexGuard<'static, ()> {
    let guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: every test of this crate that reads these holds the guard.
    unsafe {
        std::env::set_var("HOME", dir);
        std::env::set_var("PANSY_HOME", dir.join(".config/pansy"));
        std::env::remove_var("CODEX_HOME");
        std::env::remove_var("XDG_CONFIG_HOME");
    }
    guard
}

fn jwt(claims: Value) -> String {
    let b64 = |v: &Value| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v.to_string());
    format!("{}.{}.sig", b64(&json!({"alg": "none"})), b64(&claims))
}

fn codex_auth(email: &str, plan: &str, exp: i64) -> Value {
    json!({
        "OPENAI_API_KEY": null,
        "auth_mode": "chatgpt",
        "tokens": {
            "id_token": jwt(json!({"email": email, "https://api.openai.com/auth": {
                "chatgpt_plan_type": plan, "chatgpt_account_id": "acct-1"}})),
            "access_token": jwt(json!({"exp": exp})),
            "refresh_token": "rt-1",
            "account_id": "",
        },
        "last_refresh": "2026-01-01T00:00:00Z",
        "kept": "as Codex wrote it",
    })
}

fn write(path: &Path, value: &Value) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

#[test]
fn finds_the_agents_sign_ins_and_pansys() {
    let home = tempfile::tempdir().unwrap();
    let _env = scratch_home(home.path());
    let later = chrono::Utc::now().timestamp() + 86400;
    write(
        &home.path().join(".codex/auth.json"),
        &codex_auth("ada@example.com", "plus", later),
    );
    write(
        &home.path().join(".config/github-copilot/apps.json"),
        &json!({"github.com:Iv1.b507a08c87ecfe98": {"user": "octocat", "oauth_token": "gho_1"}}),
    );
    // One of pansy's is Codex's own account: Codex's stands for it.
    add_login(Kind::Codex, "ADA@example.com", "plus", json!({})).unwrap();
    let other = add_login(Kind::Codex, "bob@example.com", "pro", json!({})).unwrap();
    assert_eq!(other, "s2");
    note_plan(Kind::Copilot, "octocat", "Pro");

    let found = found();
    let ids: Vec<(Kind, &str, &str)> = found
        .iter()
        .map(|f| (f.sign_in.kind, f.id.as_str(), f.sign_in.user.as_str()))
        .collect();
    assert_eq!(
        ids,
        [
            (Kind::Codex, "own", "ada@example.com"),
            (Kind::Codex, "s2", "bob@example.com"),
            (Kind::Copilot, "own", "octocat"),
        ]
    );
    assert_eq!(found[0].sign_in.plan, "plus");
    assert_eq!(found[2].sign_in.plan, "Pro");

    // Served as providers, whose accounts keep no key and the user's order.
    let providers = crate::config::load_providers().unwrap();
    let codex = providers.iter().find(|p| p.id == "codex").unwrap();
    assert_eq!(codex.subscription, Some(Kind::Codex));
    assert_eq!(codex.endpoints[0].protocol, Protocol::Responses);
    assert_eq!(codex.accounts.len(), 2);
    assert!(codex.accounts.iter().all(|a| a.key.is_empty()));
    assert_eq!(codex.accounts[0].display(), "ada@example.com");
    crate::manage::move_key("codex", "s2", -1).unwrap();
    crate::manage::set_key_enabled("codex", "own", false).unwrap();
    let saved = std::fs::read_to_string(crate::paths::providers()).unwrap();
    assert!(!saved.contains("gho_1") && !saved.contains("rt-1"));
    let codex = crate::config::load_providers()
        .unwrap()
        .into_iter()
        .find(|p| p.id == "codex")
        .unwrap();
    let order: Vec<(&str, bool)> = codex
        .accounts
        .iter()
        .map(|a| (a.id.as_str(), a.enabled))
        .collect();
    assert_eq!(order, [("s2", true), ("own", false)]);

    // Signed out of Copilot, its provider is gone; Codex's own can't be
    // signed out from pansy.
    std::fs::remove_file(home.path().join(".config/github-copilot/apps.json")).unwrap();
    assert!(sign_out(Kind::Codex, OWN).is_err());
    sign_out(Kind::Codex, "s2").unwrap();
    let providers = crate::config::load_providers().unwrap();
    assert!(providers.iter().all(|p| p.id != "copilot"));
    assert_eq!(
        providers
            .iter()
            .find(|p| p.id == "codex")
            .unwrap()
            .accounts
            .len(),
        1
    );

    // Settings can leave the agents' sign-ins alone.
    crate::config::update_settings(|s| {
        s.agent_sign_ins = false;
        Ok(())
    })
    .unwrap();
    let found = super::found();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].sign_in.source, Source::Pansy);
}

#[test]
fn a_fresh_token_is_used_as_kept() {
    let home = tempfile::tempdir().unwrap();
    let _env = scratch_home(home.path());
    let later = chrono::Utc::now().timestamp() + 86400;
    let auth = codex_auth("ada@example.com", "team", later);
    write(&home.path().join(".codex/auth.json"), &auth);
    assert_eq!(codex::own().unwrap().user, "ada@example.com · Team");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let http = reqwest::Client::new();
    let body = br#"{"prompt_cache_key":"conv-1"}"#;
    let signed = runtime
        .block_on(sign(
            Kind::Codex,
            OWN,
            codex::BASE,
            &http,
            body,
            Some("codex_cli_rs/0.999.1"),
        ))
        .unwrap();
    let header = |name: &str| {
        signed
            .headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    };
    assert_eq!(
        header("authorization"),
        Some(
            format!(
                "Bearer {}",
                auth["tokens"]["access_token"].as_str().unwrap()
            )
            .as_str()
        )
    );
    assert_eq!(header("chatgpt-account-id"), Some("acct-1"));
    assert_eq!(header("version"), Some("0.999.1"));
    assert_eq!(header("session-id"), Some("conv-1"));
    assert!(
        header("user-agent")
            .unwrap()
            .starts_with("codex_cli_rs/0.999.1 (")
    );
}

#[test]
fn merging_keeps_a_users_own_provider_of_the_same_id() {
    let mut providers = vec![Provider {
        subscription: None,
        ..provider(Kind::Codex)
    }];
    providers[0].name = "My codex relay".into();
    let found = vec![Found {
        id: OWN.into(),
        sign_in: SignIn {
            kind: Kind::Codex,
            user: "a@b.c".into(),
            plan: String::new(),
            source: Source::Agent,
            project: String::new(),
        },
    }];
    merge(&mut providers, &found);
    assert_eq!(providers.len(), 1);
    assert!(providers[0].accounts.is_empty());
}

#[test]
fn shapes_a_request_as_codex_sends_it() {
    let long_id = "x".repeat(80);
    let mut body = json!({
        "model": "gpt-5.5",
        "instructions": "You are pi, a coding agent.",
        "max_output_tokens": 1000,
        "temperature": 0.2,
        "service_tier": "flex",
        "reasoning": {"effort": "high"},
        "input": [
            {"type": "message", "role": "system", "content": "be brief", "id": "m0"},
            {"type": "message", "role": "user", "content": "hi", "id": "m1", "status": "completed"},
            {"type": "item_reference", "id": "r1"},
            {"type": "function_call", "call_id": long_id, "name": "ls", "arguments": "{}", "id": "fc1"},
            {"type": "function_call_output", "call_id": long_id, "output": "a.txt"},
            {"type": "function_call_output", "call_id": "gone", "name": "cat", "output": "b"},
        ],
    });
    codex::shape(&mut body);
    assert_eq!(body["store"], false);
    assert_eq!(body["stream"], true);
    assert!(body.get("max_output_tokens").is_none() && body.get("temperature").is_none());
    assert!(body.get("service_tier").is_none());
    assert!(
        body["instructions"]
            .as_str()
            .unwrap()
            .starts_with("You are a coding agent running in the Codex CLI")
    );
    assert_eq!(body["reasoning"]["summary"], "auto");
    assert_eq!(body["include"], json!(["reasoning.encrypted_content"]));
    assert!(body["prompt_cache_key"].as_str().unwrap().len() == 36);
    let input = body["input"].as_array().unwrap();
    assert_eq!(input[0]["role"], "developer");
    assert_eq!(
        input[0]["content"][0]["text"],
        "You are pi, a coding agent."
    );
    assert_eq!(input[1]["role"], "developer");
    assert!(
        input
            .iter()
            .all(|i| i.get("id").is_none() && i.get("status").is_none())
    );
    assert!(input.iter().all(|i| i["type"] != "item_reference"));
    let bound = codex::bound_call_id(&long_id);
    assert_eq!(bound.len(), 64);
    assert_eq!(input[3]["call_id"], bound.as_str());
    assert_eq!(input[4]["call_id"], bound.as_str());
    assert_eq!(input[5]["role"], "assistant");
    assert!(
        input[5]["content"]
            .as_str()
            .unwrap()
            .starts_with("[Previous cat result; call_id=gone]")
    );
    // The same conversation has the same key on every turn.
    let again = codex::conversation_key("x", &[json!({"role": "user", "content": "hi"})]);
    assert_eq!(
        again,
        codex::conversation_key("x", &[json!({"role": "user", "content": "hi"})])
    );
}

#[test]
fn reads_the_backends_model_lists() {
    let codex = codex::parse_models(
        json!({"models": [
            {"slug": "gpt-5.5-mini", "display_name": "GPT-5.5 mini", "priority": 2, "context_window": 272000,
             "supported_reasoning_levels": [{"effort": "low"}], "input_modalities": ["text", "image"]},
            {"slug": "gpt-5.5", "display_name": "GPT-5.5", "priority": 1},
            {"slug": "internal", "visibility": "hide", "priority": 0},
        ]})
        .to_string()
        .as_bytes(),
    );
    let ids: Vec<&str> = codex.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["gpt-5.5", "gpt-5.5-mini"]);
    assert!(codex[1].reasoning && codex[1].vision);
    assert_eq!(codex[1].context, Some(272000));

    let copilot = copilot::parse_models(&json!({"data": [
        {"id": "gpt-5.5", "name": "GPT-5.5", "model_picker_enabled": true, "capabilities": {"type": "chat",
         "limits": {"max_prompt_tokens": 128000}, "supports": {"vision": true}},
         "supported_endpoints": ["/responses", "ws:/responses"], "policy": {"state": "enabled"}},
        {"id": "claude-sonnet-5", "model_picker_enabled": true, "capabilities": {"type": "chat"},
         "supported_endpoints": ["/chat/completions", "/v1/messages"]},
        {"id": "gpt-4.1", "model_picker_enabled": true, "capabilities": {"type": "chat"}},
        {"id": "gemini-4-pro", "model_picker_enabled": true, "capabilities": {"type": "chat"},
         "policy": {"state": "unconfigured", "terms": "…"}},
        {"id": "text-embedding-3", "capabilities": {"type": "embeddings"}},
        {"id": "gpt-4o-free-auto", "model_picker_enabled": true, "capabilities": {"type": "chat"}},
    ]}));
    let got: Vec<(&str, Vec<Protocol>)> = copilot
        .iter()
        .map(|m| (m.id.as_str(), m.apis.clone()))
        .collect();
    assert_eq!(
        got,
        [
            ("gpt-5.5", vec![Protocol::Responses]),
            ("claude-sonnet-5", vec![Protocol::Chat, Protocol::Anthropic]),
            ("gpt-4.1", vec![Protocol::Chat]),
        ]
    );
    assert_eq!(copilot[0].context, Some(128000));
    assert!(copilot[0].vision);
    assert_eq!(copilot::plan_name("individual_pro", ""), "Pro+");
    assert_eq!(
        copilot::plan_name("individual", "free_educational_quota"),
        "Education"
    );
}

#[test]
fn copilot_bills_a_typed_turn_to_the_user() {
    let role = |v: Value| copilot::last_role(v.to_string().as_bytes());
    assert_eq!(
        role(json!({"messages": [{"role": "user", "content": "hi"}]})),
        "user"
    );
    assert_eq!(
        role(
            json!({"messages": [{"role": "user", "content": [{"type": "tool_result", "content": "ok"}]}]})
        ),
        "tool"
    );
    assert_eq!(role(json!({"input": "hi"})), "user");
    assert_eq!(
        role(json!({"input": [{"type": "function_call_output"}]})),
        ""
    );
}

#[test]
fn finds_gemini_clis_sign_in_and_its_project() {
    let home = tempfile::tempdir().unwrap();
    let _env = scratch_home(home.path());
    let gemini = home.path().join(".gemini");
    let client = "681255809395-oo8ft2oprdrnp9e3aqf6av3hmdib135j.apps.googleusercontent.com";
    let creds = |azp: &str| {
        json!({"access_token": "ya29.a", "refresh_token": "1//r", "expiry_date": 1,
               "id_token": jwt(json!({"azp": azp, "email": "ada@example.com"}))})
    };
    write(&gemini.join("oauth_creds.json"), &creds("someone-else"));
    // A token another app's client minted is left alone.
    assert!(found().iter().all(|f| f.sign_in.kind != Kind::GeminiCli));
    write(&gemini.join("oauth_creds.json"), &creds(client));
    write(
        &gemini.join("google_accounts.json"),
        &json!({"active": "ada@example.com", "old": []}),
    );
    let found = found();
    let own = found
        .iter()
        .find(|f| f.sign_in.kind == Kind::GeminiCli)
        .unwrap();
    assert_eq!(
        (own.id.as_str(), own.sign_in.user.as_str()),
        (OWN, "ada@example.com")
    );
    let providers = crate::config::load_providers().unwrap();
    let p = providers.iter().find(|p| p.id == "gemini-cli").unwrap();
    assert_eq!(p.endpoints[0].protocol, Protocol::Gemini);
    assert!(p.has_model("gemini-2.5-pro"));

    // The project: the one named here, else Gemini CLI's .env's.
    let project = || Held::Agent.read(Kind::GeminiCli).unwrap()["project"].clone();
    assert_eq!(project(), Value::Null);
    std::fs::write(
        gemini.join(".env"),
        "export GOOGLE_CLOUD_PROJECT=\"from-env\"\n",
    )
    .unwrap();
    assert_eq!(project(), "from-env");
    std::fs::remove_file(gemini.join(".env")).unwrap();
    set_project(Kind::GeminiCli, OWN, "named-here").unwrap();
    assert_eq!(project(), "named-here");
    // Its file is never written.
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(gemini.join("oauth_creds.json")).unwrap())
            .unwrap(),
        creds(client)
    );
    assert!(Held::Agent.write(Kind::GeminiCli, &json!({})).is_err());
}

#[test]
fn wraps_a_gemini_request_for_code_assist() {
    let mut body = json!({"contents": [{"role": "user", "parts": [{"text": "hi"}]}]});
    prepare(
        Kind::GeminiCli,
        Protocol::Gemini,
        "gemini-2.5-pro",
        &mut body,
    );
    assert_eq!(body["model"], "gemini-2.5-pro");
    assert_eq!(body["request"]["contents"][0]["parts"][0]["text"], "hi");
    assert_eq!(
        url(
            Kind::GeminiCli,
            "https://cloudcode-pa.googleapis.com/",
            true
        )
        .as_deref(),
        Some("https://cloudcode-pa.googleapis.com/v1internal:streamGenerateContent?alt=sse")
    );
    assert_eq!(url(Kind::Codex, codex::BASE, true), None);
    assert_eq!(Kind::parse("gemini"), Some(Kind::GeminiCli));
    assert_eq!(serde_json::to_value(Kind::GeminiCli).unwrap(), "gemini-cli");
}

#[test]
fn reads_each_vendors_allowance() {
    // ChatGPT: its 5 hours spent, no credits: held until the window renews.
    let resets = chrono::Utc::now().timestamp() + 3600;
    let held = codex::parse_usage(&json!({"plan_type": "plus", "rate_limit": {
        "allowed": false, "limit_reached": true,
        "primary_window": {"used_percent": 100.0, "limit_window_seconds": 18000, "reset_at": resets},
        "secondary_window": {"used_percent": 63.0, "limit_window_seconds": 604800, "reset_after_seconds": 90000},
    }, "credits": {"has_credits": false, "unlimited": false, "balance": "0"}}));
    assert_eq!(held.plan, "plus");
    let names: Vec<(&str, f64)> = held
        .windows
        .iter()
        .map(|w| (w.name.as_str(), w.used))
        .collect();
    assert_eq!(names, [("5 hours", 100.0), ("7 days", 63.0)]);
    assert_eq!(held.spent_until.map(|t| t.timestamp()), Some(resets));
    // ... and not held while it has credits to go on.
    let on_credits = codex::parse_usage(&json!({"rate_limit": {"allowed": false,
        "primary_window": {"used_percent": 100.0, "limit_window_seconds": 18000, "reset_at": resets}},
        "credits": {"has_credits": true, "balance": "1234.5"}}));
    assert_eq!(on_credits.spent_until, None);
    assert_eq!(on_credits.note, "1.2K credits");

    // Copilot: counts as numbers or strings, unlimited last, a pooled
    // premium allowance spent.
    let copilot = copilot::parse_usage(
        &json!({"copilot_plan": "individual", "quota_reset_date": "2026-12-01",
        "quota_snapshots": {
            "premium_interactions": {"entitlement": "300", "quota_remaining": "180", "unlimited": false},
            "chat": {"unlimited": true},
            "completions": {"entitlement": 0, "unlimited": false},
        }}),
    );
    assert_eq!(copilot.plan, "Pro");
    assert_eq!(copilot.windows.len(), 2);
    assert_eq!(
        (copilot.windows[0].used, copilot.windows[0].display.as_str()),
        (40.0, "120 / 300")
    );
    assert_eq!(
        copilot.windows[0].resets_at.unwrap().to_rfc3339(),
        "2026-12-01T00:00:00+00:00"
    );
    assert!(copilot.windows[1].unlimited);
    let pooled = copilot::parse_usage(&json!({"quota_snapshots": {
        "premium_interactions": {"unlimited": true, "has_quota": false}}}));
    assert_eq!(pooled.windows[0].used, 100.0);

    // Code Assist: each model's day, Vertex AI's left out.
    let gemini = gemini::parse_usage(&json!({"buckets": [
        {"modelId": "gemini-2.5-pro", "remainingFraction": 0.25, "resetTime": "2026-11-16T00:00:00Z", "tokenType": "REQUESTS"},
        {"modelId": "gemini-2.5-pro_vertex", "remainingFraction": 1.0},
        {"modelId": "gemini-2.5-flash", "remainingFraction": 1.0},
    ]}));
    let got: Vec<(&str, f64)> = gemini
        .windows
        .iter()
        .map(|w| (w.name.as_str(), w.used))
        .collect();
    assert_eq!(got, [("gemini-2.5-flash", 0.0), ("gemini-2.5-pro", 75.0)]);
}
