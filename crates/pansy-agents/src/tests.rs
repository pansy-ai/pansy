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

    for agent in AGENTS.iter() {
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
    for agent in AGENTS.iter() {
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

    // Its subagents' model, which no provider serves either.
    set_subagent_model("claude-code", Some("group/cheap")).unwrap();
    let found = doctor::run();
    assert_eq!(
        code_of(&found, "claude-code"),
        ["agent-model-gone", "agent-subagent-gone"]
    );
    let gone = found
        .iter()
        .find(|f| f.code == "agent-subagent-gone")
        .unwrap();
    assert_eq!(gone.args, ["Claude Code", "group/cheap"]);
    assert!(doctor::say(gone).contains("pansy claude-code --subagent"));
    set_subagent_model("claude-code", None).unwrap();

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

/// A registry that says `version` is out for every package, and how many
/// times it was asked.
fn fake_registry(
    version: &'static str,
) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let asked = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = std::sync::Arc::clone(&asked);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0u8; 2048];
            let _ = stream.read(&mut buf);
            count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let body = format!("{{\"name\":\"x\",\"version\":\"{version}\"}}");
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    (format!("http://{addr}"), asked)
}

#[cfg(unix)]
#[test]
fn says_an_agents_update_and_runs_it_when_asked() {
    use std::os::unix::fs::PermissionsExt;
    let (home, _lock) = scratch();
    // Codex as npm installs it under a prefix, and an npm that "installs"
    // the version it is asked for by writing the package's file.
    let prefix = home.path().join("prefix");
    let pkg = prefix.join("lib/node_modules/@openai/codex");
    fs::create_dir_all(pkg.join("bin")).unwrap();
    fs::create_dir_all(prefix.join("bin")).unwrap();
    fs::write(
        pkg.join("package.json"),
        r#"{"name":"@openai/codex","version":"0.168.0"}"#,
    )
    .unwrap();
    fs::write(pkg.join("bin/codex.js"), "#!/bin/sh\n").unwrap();
    std::os::unix::fs::symlink(pkg.join("bin/codex.js"), prefix.join("bin/codex")).unwrap();
    let ran = home.path().join("npm-ran");
    let npm = format!(
        "#!/bin/sh\necho \"$@\" >> '{}'\nprintf '{{\"version\":\"0.170.0\"}}' > '{}'\n",
        ran.display(),
        pkg.join("package.json").display()
    );
    for (file, text) in [
        (prefix.join("bin/npm"), npm.as_str()),
        (pkg.join("bin/codex.js"), "#!/bin/sh\n"),
    ] {
        fs::write(&file, text).unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let (registry, asked) = fake_registry("0.170.0");
    let path = std::env::var_os("PATH");
    // SAFETY: the test holds LOCK.
    unsafe {
        std::env::set_var("PATH", prefix.join("bin"));
        std::env::set_var("PANSY_NPM_REGISTRY", &registry);
    }

    // Its place tells how it was installed, and so how it is updated.
    let real = cli::real_of(&["codex"]).unwrap();
    let how = cli::how(&real, "codex", None).unwrap();
    assert_eq!(how.via, "npm");
    assert_eq!(how.latest, cli::Latest::Npm("@openai/codex".into()));
    assert_eq!(
        how.command,
        [
            prefix.join("bin/npm").display().to_string().as_str(),
            "install",
            "-g",
            "@openai/codex@latest"
        ]
    );
    // The version first, the one that is out once the registry said it.
    let mut codex = cli::Cli::default();
    for _ in 0..50 {
        let (known, pending) = clis(Duration::from_secs(5));
        codex = known.get("codex").cloned().unwrap();
        assert_eq!(codex.version.as_deref(), Some("0.168.0"));
        if !pending {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(codex.latest.as_deref(), Some("0.170.0"));
    assert!(codex.update);
    assert_eq!(codex.via, Some("npm"));
    assert!(!ran.exists(), "nothing is run before the user says so");
    assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 1);

    // Asked: the command is run, and the version read again.
    let now = update_cli("codex").unwrap();
    assert_eq!(now.version.as_deref(), Some("0.170.0"));
    assert!(!now.update);
    assert_eq!(
        fs::read_to_string(&ran).unwrap().trim(),
        "install -g @openai/codex@latest"
    );
    // One whose install tells nothing has no update to run.
    let error = update_cli("claude-code").unwrap_err();
    assert!(error.to_string().contains("can't tell how"), "{error}");

    // SAFETY: as above.
    unsafe {
        std::env::remove_var("PANSY_NPM_REGISTRY");
        match path {
            Some(p) => std::env::set_var("PATH", p),
            None => std::env::remove_var("PATH"),
        }
    }
}

#[test]
fn tells_how_a_program_was_installed_by_its_place() {
    use std::path::Path;
    let how = |path: &str, own| cli::how(Path::new(path), "claude", own);
    let mise = how(
        "/home/u/.local/share/mise/installs/codex/0.169.0/bin/codex",
        None,
    )
    .unwrap();
    assert_eq!(
        (mise.via, mise.command.join(" ").as_str()),
        ("mise", "mise upgrade codex")
    );
    assert_eq!(mise.latest, cli::Latest::Mise("codex".into()));
    let bun = how(
        "/home/u/.bun/install/global/node_modules/@google/gemini-cli/dist/index.js",
        None,
    )
    .unwrap();
    assert_eq!(
        (bun.via, bun.command.join(" ").as_str()),
        ("bun", "bun add -g @google/gemini-cli@latest")
    );
    let pnpm = how(
        "/home/u/.local/share/pnpm/global/5/node_modules/opencode-ai/bin/opencode",
        None,
    )
    .unwrap();
    assert_eq!(
        (pnpm.via, pnpm.command.join(" ").as_str()),
        ("pnpm", "pnpm add -g opencode-ai@latest")
    );
    // Claude Code's own installer keeps a folder for each version; its own
    // updater updates it.
    let own: Option<(&[&str], &str)> = Some((&["update"], "@anthropic-ai/claude-code"));
    let claude = how("/home/u/.local/share/claude/versions/2.1.3", own).unwrap();
    assert_eq!(
        (claude.via, claude.command.join(" ").as_str()),
        ("own", "claude update")
    );
    assert!(how("/usr/bin/claude", own).is_none());
    assert!(how("/usr/bin/codex", None).is_none());

    assert!(cli::newer("0.170.0", "0.168.0"));
    assert!(cli::newer("2.1.10", "2.1.9"));
    assert!(cli::newer("1.0.0", "1.0.0-beta.2"));
    assert!(!cli::newer("1.0.0", "1.0.0"));
    assert!(!cli::newer("0.9", "0.10.1"));
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

/// What every built-in agent sets, reads back and installs with, as text:
/// the manifests in `agents/` say what the code they replaced did. After a
/// change that is meant, `PANSY_BLESS=1 cargo test -p pansy-agents` writes
/// the file again.
#[test]
fn the_built_in_agents_set_what_their_manifests_always_did() {
    let (home, _lock) = scratch();
    let choices = vec!["a/x".to_owned(), "c/z".to_owned()];
    let names: BTreeMap<String, String> = [("a/x", "A · X"), ("c/z", "Z")]
        .into_iter()
        .map(|(id, name)| (id.to_owned(), name.to_owned()))
        .collect();
    let mut out = String::new();
    for agent in AGENTS.iter() {
        for small in [None, Some("b/y")] {
            let w = agent::Wiring {
                base_url: "http://127.0.0.1:7267",
                key: "pansy",
                model: "a/x",
                small_model: small,
                subagent_model: None,
                fields: agent::no_fields(),
                choices: &choices,
                names: &names,
            };
            out.push_str(&format!("{} {:?}\n", agent.id, agent.edits(&w)));
        }
        out.push_str(&format!(
            "{:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {}\n",
            agent.name,
            agent.protocol,
            agent.programs,
            agent.dirs,
            agent.small_model,
            agent.note,
            agent.files(),
            agent.install(),
            agent.picks_in(),
            (agent.base_url_key(), agent.model_key()),
            agent.model_of("pansy/q")
        ));
    }
    let out = out.replace(&home.path().display().to_string(), "HOME");
    let golden = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("agents/edits.golden");
    if std::env::var_os("PANSY_BLESS").is_some() {
        fs::write(&golden, &out).unwrap();
    }
    let expected = fs::read_to_string(&golden).unwrap_or_default();
    // The installers differ by system; the golden file is Linux's.
    if cfg!(target_os = "linux") {
        assert_eq!(out, expected, "see the test's comment");
    }
}

#[test]
fn an_agent_is_added_by_a_manifest_of_the_users() {
    let (home, _lock) = scratch();
    let dir = home.path().join(".config/pansy/agents");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("demo.toml"),
        r#"
id = "demo"
name = "Demo Agent"
protocol = "chat"
programs = ["demo"]
dirs = [".demo"]

[files]
config = "~/.demo/config.yaml"

[[set]]
file = "config"
key = ["api", "base"]
value = "{v1}"

[[set]]
file = "config"
key = ["api", "model"]
value = "pansy:{model}"

[base_url]
file = "config"
key = ["api", "base"]

[model]
file = "config"
key = ["api", "model"]
prefix = "pansy:"

[run]
args = ["--model", "{model}"]

[run.env]
DEMO_BASE = "{v1}"
"#,
    )
    .unwrap();
    fs::write(dir.join("broken.toml"), "id = \"broken\"\n").unwrap();
    fs::write(
        dir.join("codex.toml"),
        fs::read_to_string(dir.join("demo.toml"))
            .unwrap()
            .replace("id = \"demo\"", "id = \"codex\""),
    )
    .unwrap();

    let ids: Vec<&str> = agent::all().iter().map(|a| a.id).collect();
    assert_eq!(ids.iter().filter(|id| **id == "codex").count(), 1);
    assert!(ids.contains(&"demo"), "{ids:?}");
    assert_eq!(
        agent::find("codex").unwrap().name,
        "Codex",
        "pansy's own stays"
    );
    let problems = agent::problems();
    assert_eq!(problems.len(), 2, "{problems:?}");
    assert!(problems.iter().any(|p| p.contains("broken.toml")));
    assert!(
        problems
            .iter()
            .any(|p| p.contains("already has an agent codex"))
    );

    // It connects and goes back like pansy's own.
    let config = home.path().join(".demo/config.yaml");
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    fs::write(&config, "theme: dark\n").unwrap();
    connect("demo", "deepseek/deepseek-chat").unwrap();
    let demo = agent::find("demo").unwrap();
    let v = view(demo, &State::load().unwrap());
    assert!(v.connected);
    assert_eq!(v.model.as_deref(), Some("deepseek/deepseek-chat"));
    assert!(fs::read_to_string(&config).unwrap().contains("theme: dark"));
    disconnect("demo").unwrap();
    assert_eq!(fs::read_to_string(&config).unwrap(), "theme: dark\n");

    // And is run as its manifest says.
    let run = demo
        .run(&agent::Wiring {
            base_url: "http://127.0.0.1:7267",
            key: "k",
            model: "a/x",
            small_model: None,
            subagent_model: None,
            fields: agent::no_fields(),
            choices: &[],
            names: agent::no_fields(),
        })
        .unwrap();
    assert_eq!(run.args, ["--model", "a/x"]);
    assert_eq!(
        run.env,
        [(
            "DEMO_BASE".to_owned(),
            "http://127.0.0.1:7267/v1".to_owned()
        )]
    );
    assert_eq!(run.verified, None);
    assert_eq!(crate::run::by_name("demo").unwrap().id, "demo");
}

#[test]
fn a_subagent_model_is_a_key_set_and_put_back() {
    let (home, _lock) = scratch();
    let claude = home.path().join(".claude/settings.json");
    fs::create_dir_all(claude.parent().unwrap()).unwrap();
    // The user's own model for Claude Code's subagents.
    let mine = "{\n  // mine\n  \"theme\": \"dark\",\n  \"env\": {\"CLAUDE_CODE_SUBAGENT_MODEL\": \"haiku\"}\n}\n";
    fs::write(&claude, mine).unwrap();
    let codex = home.path().join(".codex/config.toml");
    fs::create_dir_all(codex.parent().unwrap()).unwrap();
    // And for Codex's, one of Codex's own models: the only kind that key
    // takes, so it is theirs and never pansy's to write.
    let theirs = "[agents]\nmax_threads = 4\ndefault_subagent_model = \"gpt-5-mini\" # mine\n";
    fs::write(&codex, theirs).unwrap();
    let env = |key: &str| -> Value {
        jsonedit::to_value(&fs::read_to_string(&claude).unwrap()).unwrap()["env"][key].clone()
    };

    // Not while the agent isn't connected.
    let error = set_subagent_model("claude-code", Some("group/cheap")).unwrap_err();
    assert!(error.to_string().contains("not connected"), "{error}");

    // Connected with none: the keys are as the user has them.
    connect("claude-code", "deepseek/deepseek-chat").unwrap();
    connect("codex", "deepseek/deepseek-chat").unwrap();
    assert_eq!(env("CLAUDE_CODE_SUBAGENT_MODEL"), "haiku");
    let toml = fs::read_to_string(&codex).unwrap();
    assert!(
        toml.contains(theirs) && !toml.contains("http_headers"),
        "{toml}"
    );
    let v = view(agent::find("claude-code").unwrap(), &State::load().unwrap());
    assert!(v.has_subagent_model);
    assert_eq!(v.subagent_model, None);

    // Set: the key, in both; a new main model keeps it.
    set_subagent_model("claude-code", Some(" group/cheap ")).unwrap();
    set_subagent_model("codex", Some("group/cheap")).unwrap();
    assert_eq!(env("CLAUDE_CODE_SUBAGENT_MODEL"), "group/cheap");
    connect("claude-code", "moonshot/kimi-k2").unwrap();
    assert_eq!(env("ANTHROPIC_MODEL"), "moonshot/kimi-k2");
    assert_eq!(env("CLAUDE_CODE_SUBAGENT_MODEL"), "group/cheap");
    let toml = fs::read_to_string(&codex).unwrap();
    // Codex's lead names its subagents' models itself: the model goes to
    // the gateway in a header, which puts every one of them on it.
    assert!(toml.contains(theirs), "{toml}");
    assert!(
        toml.contains("x-pansy-subagent-model = \"group/cheap\""),
        "{toml}"
    );
    let v = view(agent::find("codex").unwrap(), &State::load().unwrap());
    assert!(v.subagent_every);
    let v = view(agent::find("claude-code").unwrap(), &State::load().unwrap());
    assert!(!v.subagent_every);
    assert_eq!(v.subagent_model.as_deref(), Some("group/cheap"));
    assert!(
        changes("claude-code")
            .unwrap()
            .iter()
            .flat_map(|f| &f.keys)
            .any(|k| k.key == "env.CLAUDE_CODE_SUBAGENT_MODEL"
                && k.before.as_deref() == Some("haiku")
                && k.after.as_deref() == Some("group/cheap"))
    );

    // A profile keeps it, and put back for now and connected again it is there.
    profile::save("work").unwrap();
    pause("claude-code").unwrap();
    assert_eq!(fs::read_to_string(&claude).unwrap(), mine);
    resume("claude-code").unwrap();
    assert_eq!(env("CLAUDE_CODE_SUBAGENT_MODEL"), "group/cheap");

    // Cleared: that key alone goes back to what it was, the others stay.
    set_subagent_model("claude-code", None).unwrap();
    set_subagent_model("codex", None).unwrap();
    assert_eq!(env("CLAUDE_CODE_SUBAGENT_MODEL"), "haiku");
    assert_eq!(env("ANTHROPIC_MODEL"), "moonshot/kimi-k2");
    let toml = fs::read_to_string(&codex).unwrap();
    assert!(toml.contains(theirs), "{toml}");
    assert!(toml.contains("model = \"deepseek/deepseek-chat\""));
    assert!(!toml.contains("http_headers"), "{toml}");
    assert!(
        !changes("claude-code")
            .unwrap()
            .iter()
            .chain(&changes("codex").unwrap())
            .flat_map(|f| &f.keys)
            .any(|k| k.key.to_ascii_lowercase().contains("subagent"))
    );

    // The profile puts it on again; disconnecting leaves the files as they were.
    assert!(profile::apply("work").unwrap().is_empty());
    assert_eq!(env("CLAUDE_CODE_SUBAGENT_MODEL"), "group/cheap");
    disconnect("claude-code").unwrap();
    disconnect("codex").unwrap();
    assert_eq!(fs::read_to_string(&claude).unwrap(), mine);
    assert_eq!(fs::read_to_string(&codex).unwrap(), theirs);

    // An agent without a key for it says so.
    connect("gemini-cli", "deepseek/deepseek-chat").unwrap();
    let error = set_subagent_model("gemini-cli", Some("group/cheap")).unwrap_err();
    assert!(error.to_string().contains("no setting"), "{error}");
}

#[test]
fn a_manifest_names_the_subagent_model_only_when_it_has_one() {
    let base = "id = \"demo\"\nname = \"Demo\"\nprotocol = \"chat\"\nSUB\n[files]\nconfig = \"~/.demo/config.json\"\n[[set]]\nfile = \"config\"\nkey = [\"model\"]\nvalue = \"{model}\"\nSET\n[base_url]\nfile = \"config\"\nkey = [\"url\"]\n[model]\nfile = \"config\"\nkey = [\"model\"]\nRUN";
    let set = "[[set]]\nfile = \"config\"\nkey = [\"sub\"]\nvalue = \"{subagent}\"";
    let of = |sub: &str, set: &str, run: &str| {
        manifest::parse(
            &base
                .replace("SUB", sub)
                .replace("SET", set)
                .replace("RUN", run),
        )
    };
    let m = of(
        "subagent_model = true",
        set,
        "[run]\nargs = [\"-m\", \"{model}\"]\nsubagent_args = [\"--sub\", \"{subagent:q}\"]",
    )
    .unwrap();
    assert!(m.subagent_model);
    for (sub, set, run, says) in [
        ("", set, "", "doesn't say it has a model for subagents"),
        ("subagent_model = true", "", "", "no key to set names"),
        (
            "subagent_model = true",
            set,
            "[run]\nargs = [\"--sub\", \"{subagent}\"]",
            "go in subagent_args",
        ),
    ] {
        let error = of(sub, set, run).unwrap_err().to_string();
        assert!(error.contains(says), "{error}");
    }
}

#[test]
fn a_tiers_model_and_an_effort_are_keys_set_and_put_back() {
    let (home, _lock) = scratch();
    let claude = home.path().join(".claude/settings.json");
    fs::create_dir_all(claude.parent().unwrap()).unwrap();
    // The user's own effort.
    let mine = "{\n  // mine\n  \"effortLevel\": \"low\",\n  \"env\": {\"ANTHROPIC_DEFAULT_OPUS_MODEL\": \"opus\"}\n}\n";
    fs::write(&claude, mine).unwrap();
    let codex = home.path().join(".codex/config.toml");
    fs::create_dir_all(codex.parent().unwrap()).unwrap();
    fs::write(&codex, "approval_policy = \"never\"\n").unwrap();
    let settings =
        || -> Value { jsonedit::to_value(&fs::read_to_string(&claude).unwrap()).unwrap() };
    let env = |key: &str| settings()["env"][key].clone();

    // Not while the agent isn't connected, and not what it has none of.
    let error = set_field("claude-code", "effort", Some("high")).unwrap_err();
    assert!(error.to_string().contains("not connected"), "{error}");
    connect("claude-code", "deepseek/deepseek-chat").unwrap();
    connect("codex", "deepseek/deepseek-chat").unwrap();
    let error = set_field("claude-code", "mood", Some("high")).unwrap_err();
    assert!(error.to_string().contains("no setting mood"), "{error}");
    let error = set_field("claude-code", "effort", Some("frantic")).unwrap_err();
    assert!(error.to_string().contains("low, medium, high"), "{error}");

    // Connected with none: every tier on the main model, the effort the user's.
    assert_eq!(
        env("ANTHROPIC_DEFAULT_OPUS_MODEL"),
        "deepseek/deepseek-chat"
    );
    assert_eq!(settings()["effortLevel"], "low");
    assert_eq!(env("ANTHROPIC_DEFAULT_FABLE_MODEL"), Value::Null);
    assert_eq!(env("CLAUDE_CODE_AUTO_MODE_MODEL"), Value::Null);

    // A tier's own model, its effort after it; the session's effort; the
    // model auto mode checks with.
    set_field("claude-code", "opus", Some(" anthropic/claude-opus-4-5 ")).unwrap();
    set_field("claude-code", "opus_effort", Some("max")).unwrap();
    set_field("claude-code", "sonnet_effort", Some("high")).unwrap();
    set_field("claude-code", "haiku_effort", Some("low")).unwrap();
    set_field("claude-code", "effort", Some("xhigh")).unwrap();
    set_field("claude-code", "auto_mode", Some("group/cheap")).unwrap();
    assert_eq!(
        env("ANTHROPIC_DEFAULT_OPUS_MODEL"),
        "anthropic/claude-opus-4-5:max"
    );
    assert_eq!(
        env("ANTHROPIC_DEFAULT_SONNET_MODEL"),
        "deepseek/deepseek-chat:high"
    );
    assert_eq!(
        env("ANTHROPIC_DEFAULT_HAIKU_MODEL"),
        "deepseek/deepseek-chat:low"
    );
    assert_eq!(env("CLAUDE_CODE_AUTO_MODE_MODEL"), "group/cheap");
    assert_eq!(settings()["effortLevel"], "xhigh");
    // A tier with no model of its own takes no effort alone, and the
    // subagents' goes with their model.
    set_field("claude-code", "fable_effort", Some("high")).unwrap();
    set_field("claude-code", "subagent_effort", Some("medium")).unwrap();
    assert_eq!(env("ANTHROPIC_DEFAULT_FABLE_MODEL"), Value::Null);
    assert_eq!(env("CLAUDE_CODE_SUBAGENT_MODEL"), Value::Null);
    set_subagent_model("claude-code", Some("group/cheap")).unwrap();
    assert_eq!(env("CLAUDE_CODE_SUBAGENT_MODEL"), "group/cheap:medium");

    // A new main model keeps them, and moves the tiers that follow it.
    connect("claude-code", "moonshot/kimi-k2").unwrap();
    assert_eq!(
        env("ANTHROPIC_DEFAULT_OPUS_MODEL"),
        "anthropic/claude-opus-4-5:max"
    );
    assert_eq!(
        env("ANTHROPIC_DEFAULT_SONNET_MODEL"),
        "moonshot/kimi-k2:high"
    );
    let v = view(agent::find("claude-code").unwrap(), &State::load().unwrap());
    let field = |key: &str| v.fields.iter().find(|f| f.key == key).unwrap();
    assert_eq!(
        field("opus").value.as_deref(),
        Some("anthropic/claude-opus-4-5")
    );
    assert!(field("opus").follows && field("sonnet").value.is_none());
    assert_eq!(
        field("effort").levels,
        ["low", "medium", "high", "xhigh", "max"]
    );

    // Codex's effort is a key of its config.
    set_field("codex", "effort", Some("high")).unwrap();
    let toml = fs::read_to_string(&codex).unwrap();
    assert!(toml.contains("model_reasoning_effort = \"high\""), "{toml}");

    // A profile keeps them; put back for now and connected again they are there.
    profile::save("work").unwrap();
    pause("claude-code").unwrap();
    assert_eq!(fs::read_to_string(&claude).unwrap(), mine);
    resume("claude-code").unwrap();
    assert_eq!(settings()["effortLevel"], "xhigh");

    // Taken back: each key alone goes back, to what it was or to the main model.
    set_field("claude-code", "effort", None).unwrap();
    set_field("claude-code", "opus", None).unwrap();
    set_field("claude-code", "auto_mode", None).unwrap();
    set_field("codex", "effort", None).unwrap();
    assert_eq!(settings()["effortLevel"], "low");
    assert_eq!(env("ANTHROPIC_DEFAULT_OPUS_MODEL"), "moonshot/kimi-k2:max");
    assert_eq!(env("CLAUDE_CODE_AUTO_MODE_MODEL"), Value::Null);
    assert!(!fs::read_to_string(&codex).unwrap().contains("effort"));
    profile::apply("work").unwrap();
    assert_eq!(settings()["effortLevel"], "xhigh");
    assert_eq!(
        env("ANTHROPIC_DEFAULT_OPUS_MODEL"),
        "anthropic/claude-opus-4-5:max"
    );

    // Disconnected: the files as they were.
    disconnect("claude-code").unwrap();
    disconnect("codex").unwrap();
    assert_eq!(fs::read_to_string(&claude).unwrap(), mine);
    assert_eq!(
        fs::read_to_string(&codex).unwrap(),
        "approval_policy = \"never\"\n"
    );
}

#[test]
fn a_manifests_fields_are_checked() {
    let good = "id = \"x\"\nname = \"X\"\nprotocol = \"chat\"\n[files]\nc = \"~/.x/c.json\"\n[base_url]\nfile = \"c\"\nkey = [\"url\"]\n[model]\nfile = \"c\"\nkey = [\"model\"]\n";
    let with = |field: &str| manifest::parse(&format!("{good}\n[[field]]\n{field}\n"));
    let m = with("key = \"effort\"\nkind = \"effort\"\nlevels = [\"low\", \"high\"]").unwrap();
    assert_eq!(m.fields[0].levels, ["low", "high"]);
    assert!(m.fields[0].refuses("high").is_none());
    assert!(m.fields[0].refuses("max").is_some());
    let said = |field: &str| format!("{:#}", with(field).unwrap_err());
    assert!(said("key = \"model\"\nkind = \"model\"").contains("already taken"));
    assert!(said("key = \"Effort\"\nkind = \"effort\"").contains("small letters"));
    assert!(said("key = \"e\"\nkind = \"effort\"").contains("has levels"));
    assert!(said("key = \"m\"\nkind = \"model\"\nlevels = [\"a\"]").contains("has levels"));
    assert!(said("key = \"m\"\nkind = \"mood\"").contains("no kind of field mood"));
    assert!(
        said("key = \"e\"\nkind = \"effort\"\nlevels = [\"a\"]\nfollows = true")
            .contains("for a model")
    );
    // A size, a share and a switch: what each takes, kept as its digits.
    let m = with("key = \"at\"\nkind = \"tokens\"\nmin = 1000\nmax = 2000000").unwrap();
    assert_eq!(m.fields[0].accept("272k").as_deref(), Ok("272000"));
    assert_eq!(m.fields[0].accept(" 1.5M ").as_deref(), Ok("1500000"));
    assert_eq!(m.fields[0].accept("128,000").as_deref(), Ok("128000"));
    assert!(m.fields[0].accept("1.5").is_err() && m.fields[0].accept("3m").is_err());
    assert!(said("key = \"p\"\nkind = \"percent\"\nmin = 1").contains("for tokens"));
    assert!(said("key = \"s\"\nkind = \"switch\"\njoins = \":\"").contains("joins is for"));
    let when =
        format!("{good}\n[[set]]\nfile = \"c\"\nkey = [\"k\"]\nvalue = true\nwhen = \"s\"\n");
    assert!(format!("{:#}", manifest::parse(&when).unwrap_err()).contains("names no field"));
}

#[test]
fn an_agents_own_settings_are_keys_set_and_put_back() {
    let (home, _lock) = scratch();
    let codex = home.path().join(".codex/config.toml");
    fs::create_dir_all(codex.parent().unwrap()).unwrap();
    // The user's own background compaction, as a switch alone.
    let mine = "approval_policy = \"never\"\n\n[features]\nasync_compaction = false\n";
    fs::write(&codex, mine).unwrap();
    let claude = home.path().join(".claude/settings.json");
    connect("codex", "deepseek/deepseek-chat").unwrap();
    connect("claude-code", "deepseek/deepseek-chat").unwrap();
    let toml =
        || -> toml_edit::DocumentMut { fs::read_to_string(&codex).unwrap().parse().unwrap() };

    // A size is a number of tokens, written as a number; a share is a percent.
    let error = set_field("codex", "compact_at", Some("lots")).unwrap_err();
    assert!(error.to_string().contains("tokens like 272k"), "{error}");
    let error = set_field("codex", "compact_at", Some("900")).unwrap_err();
    assert!(
        error.to_string().contains("10000 tokens or more"),
        "{error}"
    );
    let error = set_field("codex", "background_compaction_at", Some("140")).unwrap_err();
    assert!(error.to_string().contains("1 to 100"), "{error}");
    let error = set_field("codex", "background_compaction", Some("maybe")).unwrap_err();
    assert!(error.to_string().contains("on or off"), "{error}");
    set_field("codex", "compact_at", Some("200k")).unwrap();
    set_field("codex", "context_window", Some("1m")).unwrap();
    set_field("codex", "subagent_effort", Some("low")).unwrap();
    assert_eq!(
        toml()["model_auto_compact_token_limit"].as_integer(),
        Some(200_000)
    );
    assert_eq!(toml()["model_context_window"].as_integer(), Some(1_000_000));
    assert_eq!(
        toml()["agents"]["default_subagent_reasoning_effort"].as_str(),
        Some("low")
    );

    // The share alone sets nothing: the switch turns the table on, with it.
    set_field("codex", "background_compaction_at", Some("70%")).unwrap();
    assert_eq!(
        toml()["features"]["async_compaction"].as_bool(),
        Some(false)
    );
    set_field("codex", "background_compaction", Some("on")).unwrap();
    let table = toml()["features"]["async_compaction"].clone();
    assert_eq!(table["enabled"].as_bool(), Some(true));
    assert_eq!(
        table["model_async_compact_threshold_percent"].as_integer(),
        Some(70)
    );
    set_field("codex", "background_compaction_at", None).unwrap();
    let table = toml()["features"]["async_compaction"].clone();
    assert_eq!(table["enabled"].as_bool(), Some(true));
    assert!(table.get("model_async_compact_threshold_percent").is_none());
    let v = view(agent::find("codex").unwrap(), &State::load().unwrap());
    let field = |key: &str| v.fields.iter().find(|f| f.key == key).unwrap();
    assert_eq!(field("compact_at").value.as_deref(), Some("200000"));
    assert_eq!(field("compact_at").min, Some(10_000));
    assert_eq!(field("background_compaction").value.as_deref(), Some("on"));

    // Off again: the user's own switch is back.
    set_field("codex", "background_compaction", None).unwrap();
    assert_eq!(
        toml()["features"]["async_compaction"].as_bool(),
        Some(false)
    );

    // The user's own table, written in one line and changed by them since
    // pansy wrote the file: put back key by key, a table again.
    let one_line =
        "async_compaction = { enabled = false, model_async_compact_threshold_percent = 50 }";
    let theirs = fs::read_to_string(&codex)
        .unwrap()
        .replace("async_compaction = false", one_line);
    fs::write(&codex, theirs).unwrap();
    set_field("codex", "background_compaction", Some("on")).unwrap();
    assert_eq!(
        toml()["features"]["async_compaction"]["enabled"].as_bool(),
        Some(true)
    );
    set_field("codex", "background_compaction", None).unwrap();
    let table = toml()["features"]["async_compaction"].clone();
    assert_eq!(table["enabled"].as_bool(), Some(false));
    assert_eq!(
        table["model_async_compact_threshold_percent"].as_integer(),
        Some(50)
    );
    let theirs = fs::read_to_string(&codex)
        .unwrap()
        .replace(one_line, "async_compaction = false");
    fs::write(&codex, theirs).unwrap();

    // Claude Code's is a variable of its settings, as text.
    let error = set_field("claude-code", "compact_at", Some("50k")).unwrap_err();
    assert!(
        error.to_string().contains("100000 tokens or more"),
        "{error}"
    );
    set_field("claude-code", "compact_at", Some("272k")).unwrap();
    let json = jsonedit::to_value(&fs::read_to_string(&claude).unwrap()).unwrap();
    assert_eq!(json["env"]["CLAUDE_CODE_AUTO_COMPACT_WINDOW"], "272000");
    set_field("claude-code", "compact_at", None).unwrap();
    let json = jsonedit::to_value(&fs::read_to_string(&claude).unwrap()).unwrap();
    assert_eq!(json["env"]["CLAUDE_CODE_AUTO_COMPACT_WINDOW"], Value::Null);

    disconnect("codex").unwrap();
    disconnect("claude-code").unwrap();
    assert_eq!(fs::read_to_string(&codex).unwrap(), mine);
    assert!(!claude.exists());
}

#[test]
fn an_agent_lists_the_models_picked_for_it_by_the_names_asked() {
    use pansy_core::manage;
    use pansy_core::settings::AgentModels;
    let (home, _lock) = scratch();
    let acme = manage::add_custom(
        "Acme",
        Protocol::Chat,
        "https://acme.example/v1",
        None,
        &["large".to_owned(), "small".to_owned(), "old".to_owned()],
    )
    .unwrap();
    let id = |model: &str| format!("{acme}/{model}");
    let config = home.path().join(".config/opencode/opencode.json");
    let listed = || -> Vec<(String, String)> {
        let json: Value = serde_json::from_str(&fs::read_to_string(&config).unwrap()).unwrap();
        json["provider"]["pansy"]["models"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(id, m)| (id.clone(), m["name"].as_str().unwrap().to_owned()))
            .collect()
    };
    let ids = || -> Vec<String> { listed().into_iter().map(|(id, _)| id).collect() };

    // Every model, each after its provider's name.
    connect("opencode", &id("large")).unwrap();
    assert_eq!(ids(), [id("large"), id("small"), id("old")]);
    assert_eq!(listed()[0].1, "Acme · large");
    let v = view(agent::find("opencode").unwrap(), &State::load().unwrap());
    assert!(v.lists_models);
    assert!(!view(agent::find("codex").unwrap(), &State::load().unwrap()).lists_models);

    // One taken out is gone from the file at once; the one it is on stays.
    let hidden = AgentModels {
        hidden: vec![id("old"), id("large")],
        ..AgentModels::default()
    };
    set_listed("opencode", hidden).unwrap();
    assert_eq!(ids(), [id("small"), id("large")]);
    // Only the picked ones: a model that comes later stays off.
    let only = AgentModels {
        only_picked: true,
        picked: vec![id("small")],
        ..AgentModels::default()
    };
    set_listed("opencode", only).unwrap();
    manage::add_model(&acme, "new").unwrap();
    connect("opencode", &id("small")).unwrap();
    assert_eq!(ids(), [id("small")]);
    // Another agent lists them all still.
    assert_eq!(offered("crush").0.len(), 4);
    let error = set_listed("codex", AgentModels::default()).unwrap_err();
    assert!(error.to_string().contains("lists no models"), "{error}");

    // By its own name alone, for this agent or for every one.
    set_names("opencode", Some(false)).unwrap();
    assert_eq!(listed()[0].1, "small");
    assert_eq!(offered("crush").1[&id("small")], "Acme · small");
    set_names("opencode", None).unwrap();
    assert_eq!(listed()[0].1, "Acme · small");
    pansy_core::config::update_settings(|s| {
        s.set("providerInNames", "off").map_err(|e| anyhow!(e))
    })
    .unwrap();
    reconnect_all().unwrap();
    assert_eq!(listed()[0].1, "small");

    // Every model again; disconnected, the file pansy made is gone.
    set_listed("opencode", AgentModels::default()).unwrap();
    assert_eq!(ids().len(), 4);
    disconnect("opencode").unwrap();
    assert!(!config.exists());
}

#[test]
fn says_an_agent_is_signed_in_by_the_key_alone() {
    let (home, _lock) = scratch();
    let claude = agent::find("claude-code").unwrap();
    let state = State::default();
    assert!(!view(claude, &state).signed_in);
    let file = home.path().join(".claude/.credentials.json");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, "{\"mcpOAuth\": {}}").unwrap();
    assert!(!view(claude, &state).signed_in);
    fs::write(
        &file,
        "{\"claudeAiOauth\": {\"accessToken\": \"sk-ant-oat01-secret\"}}",
    )
    .unwrap();
    assert!(view(claude, &state).signed_in);
    // Nothing of it reaches what the window is handed.
    let said = serde_json::to_string(&view(claude, &state)).unwrap();
    assert!(!said.contains("secret"), "{said}");
    // Connected, it is on pansy; reading sign-ins off, it isn't looked for.
    connect("claude-code", "deepseek/deepseek-chat").unwrap();
    assert!(!view(claude, &State::load().unwrap()).signed_in);
    disconnect("claude-code").unwrap();
    pansy_core::config::update_settings(|s| s.set("agentSignIns", "off").map_err(|e| anyhow!(e)))
        .unwrap();
    assert!(!view(claude, &State::load().unwrap()).signed_in);
    assert!(!view(agent::find("opencode").unwrap(), &state).signed_in);
}
