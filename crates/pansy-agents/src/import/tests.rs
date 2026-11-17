//! Importing from each app in a scratch home: what is read, how it stands
//! against the providers here, and what an import adds.

use std::fs;

use rusqlite::Connection;

use super::*;
use crate::tests::scratch;

fn item<'a>(sources: &'a [Source], source: &str, reference: &str) -> &'a Item {
    sources
        .iter()
        .find(|s| s.id == source)
        .and_then(|s| s.items.iter().find(|it| it.reference == reference))
        .unwrap_or_else(|| panic!("no {source} {reference}"))
}

fn pick(source: &str, reference: &str, mode: Mode) -> Pick {
    Pick {
        source: source.into(),
        reference: reference.into(),
        mode,
        name: String::new(),
    }
}

#[test]
fn reads_every_app_and_imports_the_picks() {
    let (home, _lock) = scratch();
    let h = home.path();

    fs::create_dir_all(h.join(".config/magpie")).unwrap();
    fs::write(
        h.join(".config/magpie/providers.json"),
        r#"{"providers": [
          {"id": "nofx", "name": "nofx-claude", "key": "sk-nofx-1", "keyName": "main", "anthropic": "https://api.nofx.one",
           "keys": [{"name": "spare", "key": "sk-nofx-2", "off": true}], "models": ["claude-x"], "headers": {"X-Team": "a"}},
          {"id": "deepseek", "name": "DeepSeek", "preset": "deepseek", "key": "sk-ds", "chat": "https://api.deepseek.com/v1", "anthropic": "https://api.deepseek.com/anthropic"},
          {"id": "vx", "name": "Vertex", "preset": "google-vertex"},
          {"id": "gone", "name": "Gone", "hidden": true}
        ]}"#,
    )
    .unwrap();

    fs::create_dir_all(h.join(".cc-switch")).unwrap();
    let db = Connection::open(h.join(".cc-switch/cc-switch.db")).unwrap();
    db.execute_batch(
        "CREATE TABLE providers (id TEXT, app_type TEXT, name TEXT, settings_config TEXT, website_url TEXT, category TEXT, created_at INTEGER, sort_index INTEGER);
         INSERT INTO providers VALUES ('a', 'claude', 'Relay', '{\"env\":{\"ANTHROPIC_BASE_URL\":\"https://relay.example.com\",\"ANTHROPIC_AUTH_TOKEN\":\"sk-relay\",\"ANTHROPIC_MODEL\":\"glm-5\"}}', '', '', 1, 0);
         INSERT INTO providers VALUES ('b', 'codex', 'Relay', '{\"auth\":{\"OPENAI_API_KEY\":\"sk-relay\"},\"config\":\"model_provider = \\\"r\\\"\\nmodel = \\\"gpt-x\\\"\\n[model_providers.r]\\nbase_url = \\\"https://relay.example.com/v1\\\"\\n\"}', '', '', 2, 0);
         INSERT INTO providers VALUES ('c', 'claude', 'Official', '{\"env\":{}}', '', '', 3, 0);
         INSERT INTO providers VALUES ('d', 'claude', 'Loop', '{\"env\":{\"ANTHROPIC_BASE_URL\":\"http://127.0.0.1:7267\",\"ANTHROPIC_AUTH_TOKEN\":\"x\"}}', '', '', 4, 0);
         INSERT INTO providers VALUES ('e', 'claude', 'NVIDIA #1', '{\"env\":{\"ANTHROPIC_BASE_URL\":\"https://nim.example.com\",\"ANTHROPIC_AUTH_TOKEN\":\"sk-n1\"}}', '', '', 5, 0);
         INSERT INTO providers VALUES ('f', 'claude', 'NVIDIA #2', '{\"env\":{\"ANTHROPIC_BASE_URL\":\"https://nim.example.com\",\"ANTHROPIC_AUTH_TOKEN\":\"sk-n2\"}}', '', '', 6, 0);",
    )
    .unwrap();
    drop(db);

    fs::create_dir_all(h.join(".codex")).unwrap();
    fs::write(
        h.join(".codex/config.toml"),
        "model_provider = \"mine\"\nmodel = \"m1\"\n[model_providers.mine]\nname = \"Mine\"\nbase_url = \"https://mine.example.com/v1\"\nwire_api = \"chat\"\nexperimental_bearer_token = \"sk-mine\"\nhttp_headers = { \"X-A\" = \"1\" }\n[model_providers.pansy]\nbase_url = \"http://127.0.0.1:7267/v1\"\n",
    )
    .unwrap();

    fs::create_dir_all(h.join(".local/share/opencode")).unwrap();
    fs::write(
        h.join(".local/share/opencode/auth.json"),
        r#"{"deepseek": {"type": "api", "key": "sk-ds2"}, "github-copilot": {"type": "oauth"}, "nowhere": {"type": "api", "key": "k"}}"#,
    )
    .unwrap();

    // One here already: DeepSeek with magpie's key.
    pansy_core::manage::add_preset("deepseek", Some("sk-ds"), "", Default::default()).unwrap();

    let all = sources(true);
    assert_eq!(
        all.iter().map(|s| s.id).collect::<Vec<_>>(),
        [
            "magpie",
            "cc-switch",
            "alma",
            "claude-code",
            "codex",
            "opencode"
        ]
    );
    assert!(!all.iter().find(|s| s.id == "alma").unwrap().found);

    let nofx = item(&all, "magpie", "nofx");
    let p = nofx.provider.as_ref().unwrap();
    assert_eq!(
        (p.id.as_str(), p.name.as_str(), nofx.status),
        ("nofx", "nofx-claude", Status::New)
    );
    assert_eq!(p.accounts.len(), 2);
    assert_eq!(
        (p.accounts[0].label.as_str(), p.accounts[1].enabled),
        ("main", false)
    );
    assert_eq!(p.headers.get("X-Team").map(String::as_str), Some("a"));
    assert_eq!(p.manual_models, ["claude-x"]);
    let ds = item(&all, "magpie", "deepseek");
    assert_eq!(
        (ds.status, ds.existing.as_str()),
        (Status::Same, "DeepSeek")
    );
    assert_eq!(
        ds.provider.as_ref().unwrap().preset.as_deref(),
        Some("deepseek")
    );
    assert!(!item(&all, "magpie", "vx").skip.is_empty());
    assert!(all[0].items.iter().all(|it| it.reference != "gone"));

    // The relay set up for Claude Code and for Codex is one provider.
    let relay = item(&all, "cc-switch", "claude/a");
    let p = relay.provider.as_ref().unwrap();
    assert_eq!(relay.from, "Claude Code, Codex");
    assert_eq!(p.protocols(), [Protocol::Anthropic, Protocol::Responses]);
    assert!(all[1].items.iter().all(|it| it.reference != "codex/b"));
    assert_eq!(item(&all, "cc-switch", "claude/c").skip, readers_sign_in());
    assert_eq!(
        item(&all, "cc-switch", "claude/d").skip,
        "it points at pansy itself"
    );

    let mine = item(&all, "codex", "mine");
    let p = mine.provider.as_ref().unwrap();
    assert_eq!(
        (p.name.as_str(), p.protocols()),
        ("Mine", vec![Protocol::Chat])
    );
    assert_eq!(p.headers.get("X-A").map(String::as_str), Some("1"));
    assert_eq!(p.manual_models, ["m1"]);
    assert!(
        all.iter()
            .find(|s| s.id == "codex")
            .unwrap()
            .items
            .iter()
            .all(|it| it.reference != "pansy")
    );

    // OpenCode's DeepSeek key is another key of the DeepSeek here.
    let oc = item(&all, "opencode", "deepseek");
    assert_eq!((oc.status, oc.key_of.as_str()), (Status::New, "deepseek"));
    assert!(picked(oc));
    assert!(!item(&all, "opencode", "github-copilot").skip.is_empty());
    assert_eq!(item(&all, "opencode", "nowhere").skip, "it has no base URL");

    // Found unasked: magpie's, CC Switch's and Codex's, not OpenCode's.
    let found = found().unwrap();
    assert_eq!(found.len(), 5, "{found:?}");
    ignore(&[found[0]["fingerprint"].as_str().unwrap().to_owned()]).unwrap();
    assert_eq!(super::found().unwrap().len(), 4);

    let added = import(&[
        pick("magpie", "nofx", Mode::Add),
        pick("magpie", "deepseek", Mode::Add),
        pick("cc-switch", "claude/a", Mode::Add),
        pick("cc-switch", "claude/e", Mode::Add),
        pick("cc-switch", "claude/f", Mode::Add),
        pick("opencode", "deepseek", Mode::Key),
    ])
    .unwrap();
    assert_eq!(
        added.names,
        [
            "nofx-claude",
            "Relay",
            "NVIDIA #1",
            "NVIDIA #2",
            "DeepSeek (DeepSeek)"
        ]
    );
    let ps = pansy_core::config::load_providers().unwrap();
    assert_eq!(
        ps.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        ["deepseek", "nofx", "relay", "nvidia-1"]
    );
    assert_eq!(added.ids, ["nofx", "relay", "nvidia-1"]);
    // the two NVIDIA entries, a key each, are one provider with both keys
    let nim = &ps[3];
    assert_eq!(
        nim.accounts
            .iter()
            .map(|a| (a.key.as_str(), a.label.as_str()))
            .collect::<Vec<_>>(),
        [("sk-n1", "NVIDIA #1"), ("sk-n2", "NVIDIA #2")]
    );
    assert_eq!(ps[0].accounts.len(), 2);

    // Read again, they are all here.
    let all = sources(true);
    assert_eq!(item(&all, "magpie", "nofx").status, Status::Same);
    assert_eq!(item(&all, "opencode", "deepseek").status, Status::Same);
    // the apps' files are as they were
    assert!(
        fs::read_to_string(h.join(".codex/config.toml"))
            .unwrap()
            .contains("sk-mine")
    );
}

fn readers_sign_in() -> &'static str {
    "it is the agent's own sign-in, not a key; add the subscription in pansy"
}

#[test]
fn reads_claude_codes_relay_put_aside_by_pansy() {
    let (home, _lock) = scratch();
    let claude = home.path().join(".claude/settings.json");
    fs::create_dir_all(claude.parent().unwrap()).unwrap();
    fs::write(&claude, r#"{"env": {"ANTHROPIC_BASE_URL": "https://relay.example.com/api", "ANTHROPIC_AUTH_TOKEN": "sk-r"}}"#).unwrap();
    let read = || {
        let all = sources(false);
        item(&all, "claude-code", "settings").clone()
    };
    let it = read();
    assert_eq!(
        (it.name.as_str(), it.from.as_str()),
        ("relay.example.com", "settings.json")
    );
    crate::connect("claude-code", "deepseek/deepseek-chat").unwrap();
    // while Claude Code goes through pansy, the relay it had
    let now = read();
    assert_eq!(
        now.provider
            .as_ref()
            .map(|p| p.endpoints[0].base_url.clone()),
        Some("https://relay.example.com/api".into())
    );
    assert_eq!(
        now.provider.as_ref().map(|p| p.accounts[0].key.clone()),
        Some("sk-r".into())
    );
}

/// What a curl command reads as: the protocol, base URL, part to replace,
/// key, stand-in key and model, as magpie's `curlOf` reads them.
fn curl_of(text: &str) -> (Option<Protocol>, String, String, String, String, String) {
    let c = paste::curl(text).expect("a curl command");
    let first = c.bases.first();
    (
        first.map(|b| b.protocol),
        first.map(|b| b.url.clone()).unwrap_or_default(),
        c.url_hole,
        c.key,
        c.placeholder,
        c.model,
    )
}

#[test]
fn reads_the_vendors_curl_examples_as_their_docs_show_them() {
    use Protocol::*;
    let s = |x: &str| x.to_owned();
    // magpie's fixtures, copied from the vendors' pages on 2026-10-16
    let want = [
        (
            "openai-chat.sh",
            (
                Some(Chat),
                s("https://api.openai.com/v1"),
                s(""),
                s(""),
                s("$OPENAI_API_KEY"),
                s(""),
            ),
        ),
        (
            "openai-responses.sh",
            (
                Some(Responses),
                s("https://api.openai.com/v1"),
                s(""),
                s(""),
                s("$OPENAI_API_KEY"),
                s("gpt-6-astra"),
            ),
        ),
        (
            "anthropic.sh",
            (
                Some(Anthropic),
                s("https://api.anthropic.com"),
                s(""),
                s(""),
                s("$ANTHROPIC_API_KEY"),
                s("claude-opus-5-5"),
            ),
        ),
        (
            "deepseek.sh",
            (
                Some(Chat),
                s("https://api.deepseek.com/v1"),
                s(""),
                s(""),
                s("${DEEPSEEK_API_KEY}"),
                s("deepseek-flash"),
            ),
        ),
        (
            "zhipu.sh",
            (
                Some(Chat),
                s("https://open.bigmodel.cn/api/paas/v4"),
                s(""),
                s(""),
                s("YOUR_API_KEY"),
                s("glm-5.3"),
            ),
        ),
        (
            "moonshot.sh",
            (
                Some(Chat),
                s("https://api.moonshot.cn/v1"),
                s(""),
                s(""),
                s("$MOONSHOT_API_KEY"),
                s("kimi-k3"),
            ),
        ),
        (
            "openrouter.sh",
            (
                Some(Chat),
                s("https://openrouter.ai/api/v1"),
                s(""),
                s(""),
                s("$OPENROUTER_API_KEY"),
                s("~openai/gpt-sol-latest"),
            ),
        ),
        (
            "bailian-cmd.bat",
            (
                Some(Chat),
                s("https://{WorkspaceId}.cn-beijing.maas.aliyuncs.com/compatible-mode/v1"),
                s("{WorkspaceId}"),
                s(""),
                s("%DASHSCOPE_API_KEY%"),
                s("qwen3.8-max"),
            ),
        ),
        (
            "sap-ai-core.ps1",
            (
                None,
                s(""),
                s(""),
                s(""),
                s("<token>"),
                s("meta--llama3.1-70b-instruct"),
            ),
        ),
        (
            "unsloth-studio.ps1",
            (
                Some(Chat),
                s("http://127.0.0.1:8888/v1"),
                s(""),
                s(""),
                s("sk-unsloth-YOUR_KEY"),
                s(""),
            ),
        ),
    ];
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/import/testdata/curl");
    for (file, w) in want {
        let text = fs::read_to_string(dir.join(file)).unwrap();
        assert_eq!(curl_of(&text), w, "{file}");
        // and as a Windows clipboard gives it
        assert_eq!(curl_of(&text.replace('\n', "\r\n")), w, "{file} (CRLF)");
    }
    let own = "sk-ant-api03-Qx7vT2mK9pLw4RzN8bYc";
    let text = fs::read_to_string(dir.join("anthropic.sh"))
        .unwrap()
        .replace("$ANTHROPIC_API_KEY", own);
    let c = paste::curl(&text).unwrap();
    assert_eq!(
        (c.key.as_str(), c.placeholder.as_str(), c.name.as_str()),
        (own, "", "Anthropic")
    );
    for s in [
        "https://api.moonshot.cn/v1",
        "sk-a\nsk-b",
        "curly braces",
        "  ",
        "echo curl https://api.openai.com/v1/chat/completions",
    ] {
        assert_eq!(paste::curl(s), None, "{s:?}");
    }
}

#[test]
fn reads_environment_lines_in_every_shells_form() {
    let own = "sk-ant-api03-Qx7vT2mK9pLw4RzN8bYc";
    let forms = [
        ("export", format!("export ANTHROPIC_BASE_URL=\"https://api.moonshot.cn/anthropic\"\nexport ANTHROPIC_AUTH_TOKEN=\"{own}\"\n")),
        ("cmd", format!("set ANTHROPIC_BASE_URL=https://api.moonshot.cn/anthropic\r\nset ANTHROPIC_AUTH_TOKEN={own}")),
        ("ps", format!("$env:ANTHROPIC_BASE_URL = \"https://api.moonshot.cn/anthropic\"\n$env:ANTHROPIC_AUTH_TOKEN = '{own}'")),
        ("json", serde_json::json!({"env": {"ANTHROPIC_BASE_URL": "https://api.moonshot.cn/anthropic", "ANTHROPIC_AUTH_TOKEN": own, "ANTHROPIC_MODEL": "kimi-k3"}}).to_string()),
        ("dotenv", format!("# my relay\nANTHROPIC_BASE_URL=https://api.moonshot.cn/anthropic # note\nANTHROPIC_API_KEY={own}\n")),
        ("docker", format!("ENV ANTHROPIC_BASE_URL https://api.moonshot.cn/anthropic\nENV ANTHROPIC_API_KEY {own}")),
    ];
    for (name, text) in forms {
        let c = paste::env(&text).unwrap_or_else(|| panic!("{name}"));
        assert_eq!(
            c.bases,
            vec![paste::Base {
                protocol: Protocol::Anthropic,
                url: "https://api.moonshot.cn/anthropic".into()
            }],
            "{name}"
        );
        assert_eq!(
            (c.key.as_str(), c.name.as_str()),
            (own, "Moonshot"),
            "{name}"
        );
        if name == "json" {
            assert_eq!(c.model, "kimi-k3");
        }
    }
    // the page's own stand-ins: no address, and a key that stands for one
    let c = paste::env("export ANTHROPIC_BASE_URL=\"example-xxx\"\nexport ANTHROPIC_AUTH_TOKEN=\"sk-xxxxxxxxxxxxxxx\"").unwrap();
    assert_eq!(
        (c.bases.len(), c.key.as_str(), c.placeholder.as_str()),
        (0, "", "sk-xxxxxxxxxxxxxxx")
    );
    // two protocols, the first named first; a host without its scheme
    let c = paste::env("export OPENAI_BASE_URL=https://openrouter.ai/api/v1\nexport OPENAI_API_KEY=sk-or-v1-abcdef0123\nexport ANTHROPIC_BASE_URL=openrouter.ai/api").unwrap();
    let bases: Vec<(Protocol, &str)> = c
        .bases
        .iter()
        .map(|b| (b.protocol, b.url.as_str()))
        .collect();
    assert_eq!(
        bases,
        [
            (Protocol::Chat, "https://openrouter.ai/api/v1"),
            (Protocol::Anthropic, "https://openrouter.ai/api")
        ]
    );
    assert_eq!(
        (c.name.as_str(), c.key.as_str()),
        ("Openrouter", "sk-or-v1-abcdef0123")
    );
    // …/v1/chat/completions given whole is cut back to the base
    let c = paste::env("OPENAI_API_BASE=https://api.deepseek.com/v1/chat/completions").unwrap();
    assert_eq!(c.bases[0].url, "https://api.deepseek.com/v1");
    for s in [
        "",
        "sk-a\nsk-b",
        "https://api.moonshot.cn/v1",
        "PATH=/usr/bin\nHOME=/root",
        "export FOO=bar",
    ] {
        assert_eq!(paste::env(s), None, "{s:?}");
    }
    // read() takes curl first
    assert_eq!(
        paste::read("curl https://api.x.ai/v1/chat/completions").map(|p| p.kind),
        Some("curl")
    );
    assert_eq!(paste::site_name("http://127.0.0.1:8888/v1"), "127.0.0.1");
}

#[test]
fn reads_import_links_and_adds_what_they_describe() {
    let (_home, _lock) = scratch();
    // a preset, at one of its regions, with its key and a model
    let p = link::read(
        "pansy://import?preset=deepseek&key=sk-link-0123456789abcdef&models=deepseek-flash",
    )
    .unwrap();
    assert_eq!(
        (p.id.as_str(), p.preset.as_deref()),
        ("deepseek", Some("deepseek"))
    );
    assert_eq!(p.accounts[0].key, "sk-link-0123456789abcdef");
    assert!(p.has_model("deepseek-flash"));
    // a relay of its own, as magpie's links describe it; + is a space
    let relay = "magpie://import?name=My+Relay&chat=https%3A%2F%2Frelay.example.com%2Fv1%2F&anthropic=https://relay.example.com&key=sk-relay-0123456789&models=a,%20b";
    let p = link::read(relay).unwrap();
    assert_eq!(
        (p.id.as_str(), p.name.as_str(), p.preset.as_deref()),
        ("my-relay", "My Relay", None)
    );
    let eps: Vec<(Protocol, &str)> = p
        .endpoints
        .iter()
        .map(|e| (e.protocol, e.base_url.as_str()))
        .collect();
    assert_eq!(
        eps,
        [
            (Protocol::Anthropic, "https://relay.example.com"),
            (Protocol::Chat, "https://relay.example.com/v1")
        ]
    );
    assert_eq!(p.manual_models, ["a", "b"]);
    assert!(link::read("magpie:import?name=Local&chat=http://192.168.1.9:8000/v1").is_ok());
    assert!(
        link::read("pansy:///import?name=X&gemini=https://g.example.com/v1beta&key=k-0123456789")
            .is_ok()
    );

    // what a link can't be
    for (bad, says) in [
        (
            "https://example.com/import?preset=deepseek",
            "not an import link",
        ),
        ("pansy://share?x=1", "not something pansy knows"),
        ("pansy://import?key=sk-1", "needs preset= or name="),
        ("pansy://import?preset=nope", "no preset nope"),
        ("pansy://import?preset=deepseek&region=mars", "no region"),
        ("pansy://import?name=R", "no base URL"),
        (
            "pansy://import?name=R&chat=http://relay.example.com/v1",
            "must use https",
        ),
        (
            "pansy://import?name=R&chat=https://relay.example.com/v1?x=1",
            "not a base URL",
        ),
        (
            "pansy://import?name=R&chat=https://u:p@relay.example.com/v1",
            "not a base URL",
        ),
        (
            "pansy://import?name=R&id=pansy&chat=https://relay.example.com/v1",
            "the gateway itself",
        ),
        (
            "pansy://import?name=%E2%9C%A8&chat=https://relay.example.com/v1",
            "no letters or digits",
        ),
        (
            "pansy://import?name=R&chat=http://127.0.0.1:7267/v1",
            "pansy itself",
        ),
        (
            "pansy://import?name=R&chat=https://relay.example.com/v1&key=a%0Ab",
            "not a key",
        ),
    ] {
        let e = link::read(bad).expect_err(bad).to_string();
        assert!(e.contains(says), "{bad}: {e}");
    }
    let long = format!(
        "pansy://import?name={}&id=long&chat=https://relay.example.com/v1",
        "%E6%B5%8B".repeat(40)
    );
    assert_eq!(
        link::read(&long).unwrap().name.len(),
        78,
        "cut at 80 bytes where a character begins"
    );

    // the window's view: the key masked, the hosts the key goes to
    let v = link::view(relay);
    assert_eq!(v["key"], "sk-r••••6789");
    assert_eq!(v["hosts"], serde_json::json!(["relay.example.com"]));
    assert_eq!(v["replaces"], "");
    assert!(link::view("pansy://import?name=R").get("error").is_some());

    // added, then the same id again replaces it in place
    let a = link::add(relay, "", "").unwrap();
    assert_eq!(
        (a.names, a.ids),
        (vec!["My Relay".to_owned()], vec!["my-relay".to_owned()])
    );
    assert_eq!(link::view(relay)["replaces"], "My Relay");
    link::add(relay, "Renamed", "sk-typed-in-the-window-0").unwrap();
    let ps = config::load_providers().unwrap();
    let mine: Vec<&Provider> = ps.iter().filter(|p| p.id == "my-relay").collect();
    assert_eq!(mine.len(), 1);
    assert_eq!(
        (mine[0].name.as_str(), mine[0].accounts[0].key.as_str()),
        ("Renamed", "sk-typed-in-the-window-0")
    );
    // one without a key is not added; a subscription's id is kept clear of
    let e = link::add(
        "pansy://import?name=Bare&chat=https://bare.example.com/v1",
        "",
        "",
    )
    .unwrap_err();
    assert!(e.to_string().contains("needs an API key"), "{e}");
    let a = link::add(
        "pansy://import?name=Codex&id=codex&chat=https://relay.example.com/v1&key=sk-codex-relay-0",
        "",
        "",
    )
    .unwrap();
    assert_eq!(a.ids, ["codex-2"]);
}
