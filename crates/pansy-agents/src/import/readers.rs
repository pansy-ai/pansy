//! Reading each app pansy imports from, as magpie reads them. Their files
//! and databases are opened read only.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Map, Value};

use pansy_core::paths::user_home;
use pansy_core::presets;
use pansy_core::provider::host_of;

use super::{Eps, Item, entry, skipped};
use crate::jsonedit;
use crate::state::State;

pub(super) struct Reader {
    pub id: &'static str,
    pub name: &'static str,
    pub path: fn() -> PathBuf,
    pub read: fn(&Path, u16) -> Result<Vec<Item>>,
    /// Read only when the user opens the import.
    pub asked: bool,
}

pub(super) const READERS: &[Reader] = &[
    Reader {
        id: "cc-switch",
        name: "CC Switch",
        path: cc_switch_path,
        read: read_cc_switch,
        asked: false,
    },
    Reader {
        id: "claude-code",
        name: "Claude Code",
        path: claude_path,
        read: read_claude,
        asked: false,
    },
    Reader {
        id: "codex",
        name: "Codex",
        path: codex_path,
        read: read_codex,
        asked: false,
    },
    Reader {
        id: "opencode",
        name: "OpenCode",
        path: opencode_path,
        read: read_opencode,
        asked: true,
    },
];

const SIGN_IN: &str = "it is the agent's own sign-in, not a key; add the subscription in pansy";

fn text(v: &Value, keys: &[&str]) -> String {
    keys.iter()
        .filter_map(|k| v.get(*k)?.as_str())
        .map(str::trim)
        .find(|s| !s.is_empty())
        .unwrap_or_default()
        .to_owned()
}

fn strings(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|m| {
                m.as_str()
                    .map(str::to_owned)
                    .or_else(|| m.get("id")?.as_str().map(str::to_owned))
            })
            .collect(),
        Some(Value::Object(m)) => m.keys().cloned().collect(),
        _ => Vec::new(),
    }
}

fn headers(v: Option<&Value>) -> BTreeMap<String, String> {
    v.and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
                .collect()
        })
        .unwrap_or_default()
}

fn open_read_only(path: &Path) -> Result<Connection> {
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("can't open {}", path.display()))?;
    db.busy_timeout(Duration::from_secs(3))?;
    Ok(db)
}

// ---- magpie ----------------------------------------------------------------

// Not offered for now: no entry in `READERS`. The reader is kept, and tested
// on its own, for the day it is offered again.

#[allow(dead_code)]
fn magpie_path() -> PathBuf {
    pansy_plugins::import::magpie_dir().join("providers.json")
}

/// magpie's own providers, by the ids agents know them by: a key and the
/// others it switches to, endpoints, models, headers and proxy.
#[allow(dead_code)]
pub(super) fn read_magpie(path: &Path, port: u16) -> Result<Vec<Item>> {
    let file: Value =
        serde_json::from_str(&std::fs::read_to_string(path)?).context("magpie's providers.json")?;
    let mut out = Vec::new();
    for p in file
        .get("providers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if p.get("hidden").and_then(Value::as_bool) == Some(true) {
            continue; // a sign-in the user removed from magpie
        }
        let id = text(p, &["id"]);
        let name = Some(text(p, &["name"]))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| id.clone());
        if text(p, &["preset"]) == "google-vertex" {
            out.push(skipped(
                &id,
                "providers.json",
                &name,
                "pansy can't sign in to Google Cloud for Vertex AI yet",
            ));
            continue;
        }
        let eps = Eps {
            chat: text(p, &["chat"]),
            responses: text(p, &["responses"]),
            anthropic: text(p, &["anthropic"]),
            gemini: text(p, &["gemini"]),
        };
        let mut it = entry(
            &id,
            "providers.json",
            &name,
            &text(p, &["key"]),
            eps,
            strings(p.get("models")),
            port,
        );
        if let Some(pr) = it.provider.as_mut() {
            if !id.is_empty() {
                pr.id = id.clone(); // agents name its models by it
            }
            if let Some(first) = pr.accounts.first_mut() {
                first.label = text(p, &["keyName"]);
            }
            for k in p
                .get("keys")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let key = text(k, &["key"]);
                if key.is_empty() || pr.accounts.iter().any(|a| a.key == key) {
                    continue;
                }
                let n = pr.add_account(&key, &text(k, &["name"])).id.clone();
                let off = k.get("off").and_then(Value::as_bool) == Some(true);
                if let Some(a) = pr.accounts.iter_mut().find(|a| a.id == n) {
                    a.enabled = !off;
                }
            }
            pr.headers = headers(p.get("headers"));
            pr.proxy = Some(text(p, &["proxy"])).filter(|x| !x.is_empty());
            if p.get("off").and_then(Value::as_bool) == Some(true) {
                it.off = "switched off in magpie".to_owned();
            }
        }
        out.push(it);
    }
    Ok(out)
}

// ---- CC Switch -------------------------------------------------------------

fn cc_switch_path() -> PathBuf {
    let dir = user_home().join(".cc-switch");
    let db = dir.join("cc-switch.db");
    if !db.exists() && dir.join("config.json").exists() {
        return dir.join("config.json");
    }
    db
}

/// The apps CC Switch sets up, as the picker names them.
fn cc_app(app: &str) -> String {
    match app {
        "claude" => "Claude Code",
        "claude-desktop" => "Claude Desktop",
        "codex" => "Codex",
        "gemini" => "Gemini CLI",
        "opencode" => "OpenCode",
        "openclaw" => "OpenClaw",
        "pi" => "Pi",
        "hermes" => "Hermes",
        other => other,
    }
    .to_owned()
}

fn read_cc_switch(path: &Path, port: u16) -> Result<Vec<Item>> {
    // (id, app, name, settings)
    let mut entries: Vec<(String, String, String, Value)> = Vec::new();
    if path.extension().is_some_and(|x| x == "db") {
        let db = open_read_only(path)?;
        let mut q = db
            .prepare("SELECT id, app_type, name, settings_config FROM providers ORDER BY app_type, COALESCE(sort_index, 0), created_at")
            .map_err(|e| anyhow!("CC Switch's database: {e}"))?;
        let rows = q.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        for (id, app, name, raw) in rows.flatten() {
            entries.push((
                id,
                app,
                name,
                serde_json::from_str(&raw).unwrap_or(Value::Null),
            ));
        }
    } else {
        // CC Switch before its database: a section per app
        let cfg = jsonedit::to_value(&std::fs::read_to_string(path)?)
            .map_err(|e| anyhow!("CC Switch's config.json: {e}"))?;
        for (app, sec) in cfg.as_object().into_iter().flatten() {
            for (id, p) in sec
                .get("providers")
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
            {
                let settings = p.get("settingsConfig").cloned().unwrap_or(Value::Null);
                entries.push((id.clone(), app.clone(), text(p, &["name"]), settings));
            }
        }
        entries.sort_by(|a, b| (&a.1, &a.0).cmp(&(&b.1, &b.0)));
    }
    let mut out: Vec<Item> = Vec::new();
    for (id, app, name, s) in entries {
        let reference = format!("{app}/{id}");
        let it = match cc_entry(&app, &s) {
            Ok((key, eps, models)) => {
                entry(&reference, &cc_app(&app), &name, &key, eps, models, port)
            }
            Err(why) => skipped(&reference, &cc_app(&app), &name, why),
        };
        merge(&mut out, it);
    }
    Ok(out)
}

/// Folds an entry into one read already for the same key at the same host
/// (a relay set up for Claude Code and for Codex is one provider with two
/// endpoints), or adds it.
fn merge(have: &mut Vec<Item>, it: Item) {
    let host = |it: &Item| {
        it.provider
            .as_ref()
            .and_then(|p| p.endpoints.first())
            .map(|e| host_of(&e.base_url))
    };
    if let Some(p) = &it.provider {
        let key = super::key_of(p);
        if let Some(h) = have.iter_mut().find(|h| {
            h.provider
                .as_ref()
                .is_some_and(|q| !key.is_empty() && super::key_of(q) == key)
                && host(h) == host(&it)
        }) {
            let q = h.provider.as_mut().unwrap();
            for e in &p.endpoints {
                if q.endpoint(e.protocol).is_none() {
                    q.endpoints.push(e.clone());
                }
            }
            for m in &p.models {
                if !q.has_model(&m.id) {
                    q.models.push(m.clone());
                }
            }
            for m in &p.manual_models {
                if !q.manual_models.contains(m) {
                    q.manual_models.push(m.clone());
                }
            }
            h.from = format!("{}, {}", h.from, it.from);
            return;
        }
    }
    have.push(it);
}

/// One CC Switch entry: its settings are the agent's own configuration,
/// which differs per app.
fn cc_entry(app: &str, s: &Value) -> Result<(String, Eps, Vec<String>), &'static str> {
    let env = s.get("env").cloned().unwrap_or(Value::Null);
    let mut eps = Eps::default();
    let mut models = Vec::new();
    let key;
    match app {
        "claude" | "claude-desktop" => {
            let base = text(&env, &["ANTHROPIC_BASE_URL"]);
            key = text(&env, &["ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY"]);
            if base.is_empty() && key.is_empty() {
                return Err(SIGN_IN);
            }
            eps.anthropic = if base.is_empty() {
                "https://api.anthropic.com".to_owned()
            } else {
                base
            };
            models = claude_models(&env);
        }
        "codex" => {
            let auth_key = s
                .get("auth")
                .map(|a| text(a, &["OPENAI_API_KEY"]))
                .unwrap_or_default();
            let cfg = toml_value(s.get("config").and_then(Value::as_str).unwrap_or_default());
            let id = text(&cfg, &["model_provider"]);
            let mp = cfg
                .get("model_providers")
                .and_then(|m| m.get(&id))
                .cloned()
                .unwrap_or(Value::Null);
            let base = text(&mp, &["base_url"]);
            key = if auth_key.is_empty() {
                text(&mp, &["experimental_bearer_token"])
            } else {
                auth_key
            };
            if base.is_empty() && key.is_empty() {
                return Err(SIGN_IN);
            }
            let base = if base.is_empty() {
                "https://api.openai.com/v1".to_owned()
            } else {
                base
            };
            if text(&mp, &["wire_api"]) == "chat" {
                eps.chat = base;
            } else {
                eps.responses = base;
            }
            models.push(text(&cfg, &["model"]));
        }
        "gemini" => {
            key = text(&env, &["GEMINI_API_KEY", "GOOGLE_API_KEY"]);
            match text(&env, &["GOOGLE_GEMINI_BASE_URL"]) {
                // Gemini CLI puts v1beta after its base; pansy's has it
                base if !base.is_empty() => eps.gemini = gemini_base(&base),
                _ if key.is_empty() => return Err(SIGN_IN),
                _ => eps = presets::find("google").map(Eps::of).unwrap_or_default(),
            }
            models.push(text(&env, &["GEMINI_MODEL"]));
        }
        _ => {
            // OpenCode, OpenClaw, Pi…: a base URL and a key, with the API's
            // shape named by an api or npm field
            let opts = s.get("options").cloned().unwrap_or(Value::Null);
            let mut base = text(s, &["baseUrl", "baseURL", "base_url"]);
            if base.is_empty() {
                base = text(&opts, &["baseURL", "baseUrl", "base_url"]);
            }
            let mut k = text(s, &["apiKey", "api_key"]);
            if k.is_empty() {
                k = text(&opts, &["apiKey", "api_key"]);
            }
            key = k;
            if base.is_empty() {
                return Err("it has no base URL");
            }
            let api = text(s, &["api", "npm", "type"]).to_ascii_lowercase();
            if api.contains("anthropic") {
                eps.anthropic = base;
            } else if api.contains("responses") {
                eps.responses = base;
            } else {
                eps.chat = base;
            }
            models = strings(s.get("models"));
            models.sort();
        }
    }
    Ok((key, eps, models))
}

fn claude_models(env: &Value) -> Vec<String> {
    let keys = [
        "ANTHROPIC_MODEL",
        "ANTHROPIC_DEFAULT_OPUS_MODEL",
        "ANTHROPIC_DEFAULT_SONNET_MODEL",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL",
        "ANTHROPIC_SMALL_FAST_MODEL",
    ];
    keys.iter()
        .map(|k| text(env, &[k]))
        .filter(|m| !m.is_empty())
        .collect()
}

fn gemini_base(u: &str) -> String {
    let u = u.trim().trim_end_matches('/');
    if u.ends_with("/v1beta") {
        u.to_owned()
    } else {
        format!("{u}/v1beta")
    }
}

/// A TOML document as JSON, inline tables and arrays as well.
fn toml_value(text: &str) -> Value {
    fn value(v: &toml_edit::Value) -> Value {
        match v {
            toml_edit::Value::String(s) => Value::String(s.value().clone()),
            toml_edit::Value::Integer(i) => Value::from(*i.value()),
            toml_edit::Value::Boolean(b) => Value::Bool(*b.value()),
            toml_edit::Value::Array(a) => Value::Array(a.iter().map(value).collect()),
            toml_edit::Value::InlineTable(t) => {
                Value::Object(t.iter().map(|(k, v)| (k.to_owned(), value(v))).collect())
            }
            other => Value::String(other.to_string().trim().to_owned()),
        }
    }
    fn item(i: &toml_edit::Item) -> Value {
        match i {
            toml_edit::Item::Value(v) => value(v),
            toml_edit::Item::Table(t) => Value::Object(
                t.iter()
                    .map(|(k, v)| (k.to_owned(), item(v)))
                    .collect::<Map<_, _>>(),
            ),
            toml_edit::Item::ArrayOfTables(a) => Value::Array(
                a.iter()
                    .map(|t| item(&toml_edit::Item::Table(t.clone())))
                    .collect(),
            ),
            toml_edit::Item::None => Value::Null,
        }
    }
    text.parse::<toml_edit::DocumentMut>()
        .map_or(Value::Null, |d| item(d.as_item()))
}

// ---- Claude Code and Codex ---------------------------------------------------

fn claude_path() -> PathBuf {
    user_home().join(".claude").join("settings.json")
}

/// What pansy put aside when it connected an agent: the value each key had.
fn before(agent: &str, path: &[&str]) -> Option<String> {
    let state = State::load().ok()?;
    let t = state
        .agents
        .get(agent)?
        .touched
        .iter()
        .find(|t| t.path.iter().map(String::as_str).eq(path.iter().copied()))?;
    t.before.as_ref()?.as_str().map(str::to_owned)
}

/// The relay Claude Code was pointed at in its settings.json: its base URL
/// and token under env. While it goes through pansy, what pansy put aside.
fn read_claude(path: &Path, port: u16) -> Result<Vec<Item>> {
    let s = jsonedit::to_value(&std::fs::read_to_string(path)?)
        .map_err(|e| anyhow!("Claude Code's settings.json: {e}"))?;
    let mut env = s.get("env").cloned().unwrap_or(Value::Null);
    let base = text(&env, &["ANTHROPIC_BASE_URL"]);
    if host_of(&base) == format!("127.0.0.1:{port}") {
        let mut was = Map::new();
        for k in [
            "ANTHROPIC_BASE_URL",
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_MODEL",
        ] {
            if let Some(v) = before("claude-code", &["env", k]) {
                was.insert(k.to_owned(), Value::String(v));
            }
        }
        env = Value::Object(was);
    }
    let base = text(&env, &["ANTHROPIC_BASE_URL"]);
    if base.is_empty() {
        return Ok(Vec::new()); // Anthropic's own endpoint, the agent's sign-in
    }
    let (key, eps, models) =
        cc_entry("claude", &serde_json::json!({ "env": env })).unwrap_or_default();
    Ok(vec![entry(
        "settings",
        "settings.json",
        &host_of(&base),
        &key,
        eps,
        models,
        port,
    )])
}

fn codex_path() -> PathBuf {
    crate::agent::codex_home().join("config.toml")
}

/// Codex's own model providers in its config.toml, but pansy's.
fn read_codex(path: &Path, port: u16) -> Result<Vec<Item>> {
    let cfg = toml_value(&std::fs::read_to_string(path)?);
    let mut current = text(&cfg, &["model_provider"]);
    if current == "pansy" {
        current = before("codex", &["model_provider"]).unwrap_or_default();
    }
    let mut out = Vec::new();
    for (id, mp) in cfg
        .get("model_providers")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        let name = Some(text(mp, &["name"]))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| id.clone());
        match id.as_str() {
            "pansy" => continue,
            "magpie" => {
                out.push(skipped(id, "config.toml", &name, "it points at magpie"));
                continue;
            }
            _ => {}
        }
        let mut models = Vec::new();
        if current == *id {
            models.push(text(&cfg, &["model"]));
        }
        for p in cfg
            .get("profiles")
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(|m| m.values())
        {
            let of = Some(text(p, &["model_provider"]))
                .filter(|x| !x.is_empty())
                .unwrap_or_else(|| current.clone());
            if of == *id {
                models.push(text(p, &["model"]));
            }
        }
        let base = text(mp, &["base_url"]);
        let eps = if text(mp, &["wire_api"]) == "chat" {
            Eps {
                chat: base,
                ..Eps::default()
            }
        } else {
            Eps {
                responses: base,
                ..Eps::default()
            }
        };
        let mut it = entry(
            id,
            "config.toml",
            &name,
            &text(mp, &["experimental_bearer_token"]),
            eps,
            models,
            port,
        );
        if let Some(p) = it.provider.as_mut() {
            p.headers = headers(mp.get("http_headers"));
        }
        out.push(it);
    }
    Ok(out)
}

// ---- OpenCode ----------------------------------------------------------------

fn opencode_path() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .filter(|d| !d.is_empty())
        .map_or_else(|| user_home().join(".local").join("share"), PathBuf::from)
        .join("opencode")
        .join("auth.json")
}

/// OpenCode's provider ids (models.dev's) by pansy's preset at the same
/// address; auth.json keeps only keys, never where they go.
fn opencode_preset(id: &str) -> Option<&'static presets::Preset> {
    let preset = match id {
        "opencode" => "opencode-zen",
        "moonshotai" => "moonshot",
        "moonshotai-cn" => "moonshot-cn",
        "togetherai" => "together",
        "fireworks-ai" => "fireworks",
        "siliconflow-cn" => "siliconflow",
        "zhipuai" => "zhipu",
        "alibaba" => "qwen",
        "alibaba-cn" => "qwen-cn",
        "cline-pass" => "clinepass",
        "kimi-code-plan-cn" => "kimi-code-cn",
        "kimi-code-plan-global" => "kimi-code",
        "stepfun-step-plan" => "stepfun-cn",
        "stepfun-ai-step-plan" => "stepfun",
        "volcengine-coding-plan" => "volcengine",
        "openai" | "anthropic" | "google" | "deepseek" | "xai" | "mistral" | "groq"
        | "openrouter" | "nvidia" | "aihubmix" | "302ai" | "ollama-cloud" | "vercel" | "zai"
        | "minimax" | "modelscope" | "kilo" | "longcat" | "opencode-go" => id,
        _ => return None,
    };
    presets::find(preset)
}

fn read_opencode(path: &Path, port: u16) -> Result<Vec<Item>> {
    let all: Map<String, Value> =
        serde_json::from_str(&std::fs::read_to_string(path)?).context("OpenCode's auth.json")?;
    let mut out = Vec::new();
    for (id, e) in &all {
        let preset = opencode_preset(id);
        let name = preset.map_or(id.as_str(), |p| p.name);
        let it = match (text(e, &["type"]).as_str(), preset) {
            ("oauth" | "wellknown", _) => skipped(
                id,
                "auth.json",
                name,
                "it is a sign-in, not a key; add the subscription in pansy",
            ),
            ("api", None) => skipped(id, "auth.json", name, "it has no base URL"),
            ("api", Some(p)) => entry(
                id,
                "auth.json",
                name,
                &text(e, &["key"]),
                Eps::of(p),
                Vec::new(),
                port,
            ),
            _ => continue,
        };
        out.push(it);
    }
    Ok(out)
}
