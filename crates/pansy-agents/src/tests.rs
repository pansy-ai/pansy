//! Connecting and disconnecting agents in a scratch home: what pansy sets,
//! and that disconnecting leaves the user's own settings as they were.

use std::fs;

use serde_json::Value;

use super::*;

/// Every test runs in a scratch home of its own, one at a time: they share
/// the process's env. The test holds the guard while it runs.
pub(crate) fn scratch() -> (tempfile::TempDir, std::sync::MutexGuard<'static, ()>) {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: tests of this crate that touch the env hold LOCK.
    unsafe {
        std::env::set_var("HOME", dir.path());
        std::env::set_var("PANSY_HOME", dir.path().join(".config/pansy"));
        std::env::remove_var("CODEX_HOME");
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("XDG_DATA_HOME");
        std::env::remove_var("OPENCODE_DB");
    }
    (dir, guard)
}

#[test]
fn wires_and_unwires_every_agent() {
    let (home, _lock) = scratch();
    let claude = home.path().join(".claude/settings.json");
    fs::create_dir_all(claude.parent().unwrap()).unwrap();
    fs::write(
        &claude,
        "{\n  // mine\n  \"theme\": \"dark\",\n  \"env\": {\"ANTHROPIC_MODEL\": \"opus\"}\n}\n",
    )
    .unwrap();
    let codex = home.path().join(".codex/config.toml");
    fs::create_dir_all(codex.parent().unwrap()).unwrap();
    fs::write(
        &codex,
        "model = \"gpt-5\" # mine\napproval_policy = \"never\"\n",
    )
    .unwrap();

    for agent in AGENTS {
        connect(agent.id, "deepseek/deepseek-chat").unwrap();
        let v = view(agent, &State::load().unwrap());
        assert!(v.connected, "{} is not connected", agent.id);
        assert_eq!(
            v.model.as_deref(),
            Some("deepseek/deepseek-chat"),
            "{}",
            agent.id
        );
    }
    let text = fs::read_to_string(&claude).unwrap();
    let json = jsonedit::to_value(&text).unwrap();
    assert_eq!(json["env"]["ANTHROPIC_BASE_URL"], "http://127.0.0.1:7267");
    assert_eq!(
        json["env"]["ANTHROPIC_DEFAULT_HAIKU_MODEL"],
        "deepseek/deepseek-chat"
    );
    assert!(text.contains("// mine"));
    let toml = fs::read_to_string(&codex).unwrap();
    assert!(toml.contains("model = \"deepseek/deepseek-chat\" # mine"));
    assert!(toml.contains("[model_providers.pansy]"));
    let env = fs::read_to_string(home.path().join(".gemini/.env")).unwrap();
    assert!(env.contains("GOOGLE_GEMINI_BASE_URL=http://127.0.0.1:7267"));
    let opencode: Value = serde_json::from_str(
        &fs::read_to_string(home.path().join(".config/opencode/opencode.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(opencode["model"], "pansy/deepseek/deepseek-chat");

    // A quick-task model, then a new main model keeps it.
    set_small_model("claude-code", Some("deepseek/deepseek-reasoner")).unwrap();
    connect("claude-code", "moonshot/kimi-k2").unwrap();
    let json = jsonedit::to_value(&fs::read_to_string(&claude).unwrap()).unwrap();
    assert_eq!(json["env"]["ANTHROPIC_MODEL"], "moonshot/kimi-k2");
    assert_eq!(
        json["env"]["ANTHROPIC_SMALL_FAST_MODEL"],
        "deepseek/deepseek-reasoner"
    );

    // A profile, then disconnecting everything.
    profile::save("work").unwrap();
    assert!(profile::list().unwrap()[0].active);
    for agent in AGENTS {
        disconnect(agent.id).unwrap();
        assert!(
            !view(agent, &State::load().unwrap()).connected,
            "{} is still connected",
            agent.id
        );
    }
    assert_eq!(
        fs::read_to_string(&claude).unwrap(),
        "{\n  // mine\n  \"theme\": \"dark\",\n  \"env\": {\"ANTHROPIC_MODEL\": \"opus\"}\n}\n"
    );
    assert_eq!(
        fs::read_to_string(&codex).unwrap(),
        "model = \"gpt-5\" # mine\napproval_policy = \"never\"\n"
    );
    assert!(
        home.path()
            .join(".config/pansy/backups/claude-code/settings.json.before-pansy")
            .exists()
    );

    // The files pansy made are gone again.
    assert!(!home.path().join(".gemini/.env").exists());
    assert!(!home.path().join(".config/opencode/opencode.json").exists());

    // The profile puts them back.
    assert!(profile::apply("work").unwrap().is_empty());
    let json = jsonedit::to_value(&fs::read_to_string(&claude).unwrap()).unwrap();
    assert_eq!(json["env"]["ANTHROPIC_MODEL"], "moonshot/kimi-k2");
}

#[test]
fn puts_a_file_back_whole_unless_the_user_changed_it() {
    let (home, _lock) = scratch();
    let claude = home.path().join(".claude/settings.json");
    fs::create_dir_all(claude.parent().unwrap()).unwrap();
    let mine = "{\r\n  \"theme\": \"dark\",\r\n}\r\n";
    fs::write(&claude, mine).unwrap();
    connect("claude-code", "deepseek/deepseek-chat").unwrap();
    disconnect("claude-code").unwrap();
    assert_eq!(fs::read_to_string(&claude).unwrap(), mine, "byte for byte");

    // Changed while connected: only pansy's keys go, the change stays.
    connect("claude-code", "deepseek/deepseek-chat").unwrap();
    let text = fs::read_to_string(&claude).unwrap();
    fs::write(&claude, text.replace("\"dark\"", "\"light\"")).unwrap();
    disconnect("claude-code").unwrap();
    let json = jsonedit::to_value(&fs::read_to_string(&claude).unwrap()).unwrap();
    assert_eq!(json["theme"], "light");
    assert!(json["env"].get("ANTHROPIC_BASE_URL").is_none(), "{json}");
}

#[test]
fn puts_an_agent_back_for_now_and_connects_it_again() {
    let (home, _lock) = scratch();
    let claude = home.path().join(".claude/settings.json");
    fs::create_dir_all(claude.parent().unwrap()).unwrap();
    let mine = "{\n  \"theme\": \"dark\"\n}\n";
    fs::write(&claude, mine).unwrap();
    connect("claude-code", "deepseek/deepseek-chat").unwrap();
    connect("codex", "moonshot/kimi-k2").unwrap();
    assert!(resume("claude-code").is_err(), "not put back");

    pause("claude-code").unwrap();
    assert_eq!(fs::read_to_string(&claude).unwrap(), mine, "as it was");
    let v = view(agent::find("claude-code").unwrap(), &State::load().unwrap());
    assert!(!v.connected);
    assert_eq!(v.paused.as_deref(), Some("deepseek/deepseek-chat"));
    assert!(pause("claude-code").is_err(), "not connected");
    // Connecting the port again leaves it off pansy.
    assert!(reconnect_all().unwrap().is_empty());
    assert!(!view(agent::find("claude-code").unwrap(), &State::load().unwrap()).connected);

    resume("claude-code").unwrap();
    let v = view(agent::find("claude-code").unwrap(), &State::load().unwrap());
    assert!(v.connected);
    assert_eq!(v.model.as_deref(), Some("deepseek/deepseek-chat"));
    assert_eq!(v.paused, None);

    // All at once; disconnecting one forgets its model.
    assert_eq!(pause_all().unwrap(), 2);
    assert_eq!(fs::read_to_string(&claude).unwrap(), mine);
    disconnect("codex").unwrap();
    assert_eq!(resume_all().unwrap(), 1);
    let state = State::load().unwrap();
    assert!(view(agent::find("claude-code").unwrap(), &state).connected);
    let codex = view(agent::find("codex").unwrap(), &state);
    assert!(!codex.connected && codex.paused.is_none());
}

/// A port nothing listens on, in place of 7267 (a pansy may run there).
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// Something listening on a port of its own, answering every asker `answer`.
fn answering(answer: &'static str) -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut buf = [0u8; 512];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(answer.as_bytes());
        }
    });
    url
}

#[test]
fn tells_pansy_from_another_program_on_the_port() {
    use doctor::{Probe, probe};
    let pansy = answering("HTTP/1.0 200 OK\r\n\r\n{\"name\":\"pansy\",\"ok\":true}");
    assert_eq!(probe(&pansy), Probe::Pansy);
    let other = answering("HTTP/1.0 404 Not Found\r\n\r\nnope");
    assert_eq!(probe(&other), Probe::Other);
    assert_eq!(
        probe(&format!("http://127.0.0.1:{}", free_port())),
        Probe::Nothing
    );
}

#[test]
fn diagnoses_the_gateway_providers_and_agents() {
    let (_home, _lock) = scratch();
    let port = free_port();
    pansy_core::config::update_settings(|s| {
        s.port = port;
        Ok(())
    })
    .unwrap();
    connect("claude-code", "deepseek/deepseek-chat").unwrap();
    connect("codex", "moonshot/kimi-k2").unwrap();
    pause("codex").unwrap();

    let found = doctor::run();
    assert_eq!(code_of(&found, "gateway"), ["gateway-stopped"]);
    assert_eq!(found[0].args, [format!("http://127.0.0.1:{port}")]);
    assert_eq!(code_of(&found, "providers"), ["providers-none"]);
    // No provider serves its model.
    assert_eq!(code_of(&found, "claude-code"), ["agent-model-gone"]);
    let codex = found.iter().find(|f| f.about == "codex").unwrap();
    assert_eq!(
        (codex.code, codex.level),
        ("agent-paused", doctor::Level::Note)
    );
    assert_eq!(codex.args, ["Codex", "moonshot/kimi-k2"]);
    assert!(code_of(&found, "agents").is_empty());
    assert!(code_of(&found, "files").is_empty(), "{found:?}");
    assert!(doctor::say(codex).contains("pansy resume"));

    // The port moved without connecting again: its config points elsewhere.
    pansy_core::config::update_settings(|s| {
        s.port = free_port();
        Ok(())
    })
    .unwrap();
    assert_eq!(code_of(&doctor::run(), "claude-code"), ["agent-drifted"]);
}

fn code_of(found: &[doctor::Finding], about: &str) -> Vec<&'static str> {
    found
        .iter()
        .filter(|f| f.about == about)
        .map(|f| f.code)
        .collect()
}

#[test]
fn sends_a_test_request_through_the_gateway() {
    let (_home, _lock) = scratch();
    assert!(doctor::send(None).is_err(), "no model to send to");
    let gateway_at = |url: &str| {
        let port = url.rsplit(':').next().unwrap().parse().unwrap();
        pansy_core::config::update_settings(|s| {
            s.port = port;
            Ok(())
        })
        .unwrap();
    };

    gateway_at(&answering(
        "HTTP/1.0 200 OK\r\nContent-Type: application/json\r\n\r\n{\"choices\":[{\"message\":{\"role\":\"assistant\",\"content\":\" OK \"}}]}",
    ));
    let sent = doctor::send(Some("local-relay/model-b")).unwrap();
    assert!(sent.ok, "{sent:?}");
    assert_eq!((sent.status, sent.said.as_str()), (200, "OK"));
    assert_eq!(sent.model, "local-relay/model-b");

    gateway_at(&answering(
        "HTTP/1.0 502 Bad Gateway\r\n\r\n{\"error\":{\"message\":\"every key failed\"}}",
    ));
    let sent = doctor::send(Some("local-relay/model-b")).unwrap();
    assert!(!sent.ok);
    assert_eq!((sent.status, sent.said.as_str()), (502, "every key failed"));

    gateway_at(&format!("http://127.0.0.1:{}", free_port()));
    let sent = doctor::send(Some("local-relay/model-b")).unwrap();
    assert_eq!((sent.ok, sent.status, sent.said.as_str()), (false, 0, ""));
}

#[test]
fn says_what_it_changed_with_the_users_keys_masked() {
    let (home, _lock) = scratch();
    let claude = home.path().join(".claude/settings.json");
    fs::create_dir_all(claude.parent().unwrap()).unwrap();
    fs::write(
        &claude,
        r#"{"env": {"ANTHROPIC_AUTH_TOKEN": "sk-ant-mine-0123456789", "ANTHROPIC_MODEL": "opus"}}"#,
    )
    .unwrap();
    assert!(changes("claude-code").unwrap().is_empty());
    connect("claude-code", "deepseek/deepseek-chat").unwrap();
    // OpenCode's file is pansy's own: it goes when disconnected.
    connect("opencode", "deepseek/deepseek-chat").unwrap();

    let files = changes("claude-code").unwrap();
    assert_eq!(files.len(), 1);
    assert!(!files[0].created);
    let key = |k: &str| files[0].keys.iter().find(|c| c.key == k).unwrap().clone();
    let token = key("env.ANTHROPIC_AUTH_TOKEN");
    assert_eq!(token.before.as_deref(), Some("sk-a••••6789"));
    assert_eq!(token.after.as_deref(), Some(AGENT_KEY));
    let model = key("env.ANTHROPIC_MODEL");
    assert_eq!(model.before.as_deref(), Some("opus"));
    assert_eq!(model.after.as_deref(), Some("deepseek/deepseek-chat"));
    let url = key("env.ANTHROPIC_BASE_URL");
    assert_eq!(url.before, None);
    let shown = serde_json::to_string(&files).unwrap();
    assert!(!shown.contains("0123456789"), "{shown}");

    let opencode = changes("opencode").unwrap();
    assert!(opencode[0].created);

    disconnect("claude-code").unwrap();
    assert!(changes("claude-code").unwrap().is_empty());
}

#[test]
fn renames_a_profile() {
    let (_home, _lock) = scratch();
    connect("codex", "deepseek/deepseek-chat").unwrap();
    profile::save("work").unwrap();
    profile::save("play").unwrap();
    assert!(profile::rename("work", "play").is_err(), "a name taken");
    profile::rename("work", "focus").unwrap();
    let names: Vec<String> = profile::list()
        .unwrap()
        .into_iter()
        .map(|p| p.profile.name)
        .collect();
    assert_eq!(names, ["focus", "play"]);
}

#[cfg(unix)]
#[test]
fn asks_versions_without_holding_up_the_list() {
    use std::os::unix::fs::PermissionsExt;
    let (home, _lock) = scratch();
    let bin = home.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    // PATH is only `bin` below: no agent of this computer's is asked
    let sleep = cli::which("sleep").unwrap();
    for (name, script) in [
        (
            "claude",
            "#!/bin/sh\necho '2.1.0 (Claude Code)'\n".to_owned(),
        ),
        (
            "codex",
            format!(
                "#!/bin/sh\n{} 2\necho 'codex-cli 0.168.0'\n",
                sleep.display()
            ),
        ),
    ] {
        let file = bin.join(name);
        fs::write(&file, script).unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = std::env::var_os("PATH");
    // SAFETY: the test holds LOCK.
    unsafe { std::env::set_var("PATH", &bin) };

    // found on PATH, without anything run
    let listed = list();
    let claude = listed.iter().find(|v| v.id == "claude-code").unwrap();
    assert!(claude.installed && !claude.cli_missing);
    assert!(
        !listed
            .iter()
            .find(|v| v.id == "gemini-cli")
            .unwrap()
            .installed
    );

    let (known, pending) = versions(Duration::from_millis(1000));
    assert_eq!(known.get("claude-code").map(String::as_str), Some("2.1.0"));
    assert!(!known.contains_key("codex"));
    assert!(pending, "codex is still being asked");
    let (known, pending) = versions(Duration::from_secs(10));
    assert_eq!(known.get("codex").map(String::as_str), Some("0.168.0"));
    assert!(!pending);

    // SAFETY: as above.
    unsafe {
        match path {
            Some(p) => std::env::set_var("PATH", p),
            None => std::env::remove_var("PATH"),
        }
    }
}

/// Bun for the tests that run plugins: `PANSY_BUN`, else one on `PATH`.
fn bun() -> Option<std::path::PathBuf> {
    if let Some(b) = std::env::var_os("PANSY_BUN").filter(|v| !v.is_empty()) {
        return Some(b.into());
    }
    let found = std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join("bun"))
        .find(|p| p.is_file())?;
    // SAFETY: the caller holds the scratch guard.
    unsafe { std::env::set_var("PANSY_BUN", &found) };
    Some(found)
}

#[test]
fn wires_and_unwires_an_agent_a_plugin_adds() {
    let (home, _lock) = scratch();
    if bun().is_none() {
        eprintln!("no Bun: skipped");
        return;
    }
    let aider = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../pansy-plugins/tests/fixtures/plugins/aider.agent.js");
    let added = pansy_plugins::store::List {
        plugins: vec![pansy_plugins::store::Entry {
            spec: aider.display().to_string(),
            ..Default::default()
        }],
        ..Default::default()
    };
    pansy_core::fsutil::write_json(&paths::plugins(), &added, true).unwrap();
    let kept = pansy_plugins::block(pansy_plugins::agents::reload()).unwrap();
    assert_eq!(kept.agents.len(), 1, "{:?}", kept.errors);

    let config = home.path().join(".aider.conf.yml");
    fs::write(&config, "# mine\ndark-mode: true\n").unwrap();
    connect("aider", "deepseek/deepseek-chat").unwrap();
    let text = fs::read_to_string(&config).unwrap();
    assert!(text.starts_with("# mine\ndark-mode: true\n"), "{text}");
    assert!(
        text.contains("model: openai/deepseek/deepseek-chat"),
        "{text}"
    );
    assert!(
        text.contains("openai-api-base: \"http://127.0.0.1:"),
        "{text}"
    );
    assert!(text.contains("openai-api-key: pansy"), "{text}");
    assert!(text.contains("show-model-warnings: false"), "{text}");
    let row = list().into_iter().find(|a| a.id == "aider").unwrap();
    assert!(row.connected);
    assert_eq!(row.model.as_deref(), Some("deepseek/deepseek-chat"));
    assert!(row.note.is_some());

    disconnect("aider").unwrap();
    assert_eq!(
        fs::read_to_string(&config).unwrap(),
        "# mine\ndark-mode: true\n"
    );
    assert!(
        !list()
            .into_iter()
            .find(|a| a.id == "aider")
            .unwrap()
            .connected
    );
    pansy_plugins::middleware::forget();
}
