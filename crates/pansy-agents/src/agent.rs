//! The agents pansy wires: where each keeps its config, and the keys that
//! point it at the gateway with a model.

use std::path::{Path, PathBuf};

use pansy_core::paths::user_home;
use pansy_core::provider::Protocol;
use serde::Serialize;
use serde_json::{Value, json};

use crate::files::Format;

/// One key to set in an agent's config.
#[derive(Debug, Clone, PartialEq)]
pub struct Edit {
    pub file: PathBuf,
    pub format: Format,
    pub path: Vec<String>,
    pub value: Value,
}

fn edit(file: &Path, format: Format, path: &[&str], value: Value) -> Edit {
    Edit {
        file: file.to_path_buf(),
        format,
        path: path.iter().map(|s| s.to_string()).collect(),
        value,
    }
}

/// What an agent is connected with.
#[derive(Debug, Clone)]
pub struct Wiring<'a> {
    /// The gateway, `http://127.0.0.1:7267`.
    pub base_url: &'a str,
    /// The key agents send (the gateway takes any from this computer).
    pub key: &'a str,
    pub model: &'a str,
    /// The model for quick tasks; the main one when `None`.
    pub small_model: Option<&'a str>,
    /// Every model the gateway offers, for agents that list them.
    pub choices: &'a [String],
}

/// One way to install an agent, as its vendor gives it: `via` is what runs
/// the command (`script`, `powershell`, `brew` or `npm`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Install {
    pub via: &'static str,
    pub command: &'static str,
}

const fn install(via: &'static str, command: &'static str) -> Install {
    Install { via, command }
}

/// An agent pansy knows.
#[derive(Debug, Clone, Copy)]
pub struct Agent {
    pub id: &'static str,
    pub name: &'static str,
    /// The protocol it speaks to the gateway.
    pub protocol: Protocol,
    /// Programs on `PATH` that mean it is installed.
    pub programs: &'static [&'static str],
    /// Directories under the home directory that mean it is installed.
    pub dirs: &'static [&'static str],
    /// It has a second model for quick tasks.
    pub small_model: bool,
    /// Something the user should know about wiring it.
    pub note: Option<&'static str>,
    kind: Kind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    ClaudeCode,
    Codex,
    GeminiCli,
    QwenCode,
    OpenCode,
    Crush,
    /// An agent a plugin adds (`magpie.agent`).
    Plugin(&'static Plugged),
}

/// Where an agent keeps the model a new session starts on: the file, its
/// format, the key, and the keys beside it.
pub type Startup = (PathBuf, Format, Vec<String>, serde_json::Map<String, Value>);

/// Where an agent a plugin adds keeps its model, as its module says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plugged {
    pub config: PathBuf,
    pub format: Format,
    /// The key its model is at; empty for one connected without a model.
    pub model: Vec<String>,
    pub prefix: String,
    /// Where it keeps the model a new session starts on, apart from its
    /// providers: the file, its format, the key, and the keys beside it.
    pub startup: Option<Startup>,
    pub connect: bool,
    /// The key its model is read from: the startup's when it has one.
    model_key: &'static [&'static str],
}

/// A path as an agent plugin writes it: `~/` is the home directory.
fn expand(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => home(rest),
        None if path == "~" => user_home(),
        None => PathBuf::from(path),
    }
}

/// A plugin's `format`, else the file's ending's.
fn format_of(format: &str, file: &Path) -> Format {
    match format.to_ascii_lowercase().as_str() {
        "json" | "jsonc" => Format::Json,
        "toml" => Format::Toml,
        "yaml" | "yml" => Format::Yaml,
        "env" | "dotenv" => Format::Env,
        _ => match file
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .as_deref()
        {
            Some("toml") => Format::Toml,
            Some("yaml" | "yml") => Format::Yaml,
            Some("env") => Format::Env,
            _ if file.file_name().is_some_and(|n| n == ".env") => Format::Env,
            _ => Format::Json,
        },
    }
}

fn leak(s: &str) -> &'static str {
    Box::leak(s.to_owned().into_boxed_str())
}

/// The agents plugins add, as host B last loaded them: each one pansy has
/// no agent of its own by, after pansy's own. One made once for each
/// description, so reading them again leaks nothing more.
pub fn plugged() -> Vec<&'static Agent> {
    use std::collections::HashMap;
    use std::sync::{LazyLock, Mutex};
    static MADE: LazyLock<Mutex<HashMap<String, &'static Agent>>> = LazyLock::new(Default::default);
    let kept = pansy_plugins::agents::cached();
    let mut made = MADE.lock().unwrap_or_else(|e| e.into_inner());
    let mut out: Vec<&'static Agent> = Vec::new();
    for a in kept.agents {
        if AGENTS.iter().any(|b| b.id == a.id) || out.iter().any(|b| b.id == a.id) {
            continue;
        }
        let key = serde_json::to_string(&a).unwrap_or_default();
        if let Some(agent) = made.get(&key) {
            out.push(agent);
            continue;
        }
        let config = expand(&a.config);
        let split = |k: &str| -> Vec<String> {
            k.split('.')
                .filter(|p| !p.is_empty())
                .map(str::to_owned)
                .collect()
        };
        let startup = a
            .startup
            .as_ref()
            .filter(|s| !s.config.is_empty())
            .map(|s| {
                let file = expand(&s.config);
                let format = format_of(&s.format, &file);
                (file, format, split(&s.model), s.keys.clone())
            });
        let read_at: Vec<&'static str> = match &startup {
            Some((_, _, key, _)) if !key.is_empty() => key.iter().map(|k| leak(k)).collect(),
            _ => split(&a.model).iter().map(|k| leak(k)).collect(),
        };
        let plugged: &'static Plugged = Box::leak(Box::new(Plugged {
            model_key: Box::leak(read_at.into_boxed_slice()),
            format: format_of(&a.format, &config),
            config,
            model: split(&a.model),
            prefix: a.prefix.clone(),
            startup,
            connect: a.connect,
        }));
        let programs: &'static [&'static str] = if a.bin.is_empty() {
            &[]
        } else {
            Box::leak(vec![leak(&a.bin)].into_boxed_slice())
        };
        let dir = a.dir.strip_prefix("~/").unwrap_or(&a.dir);
        let dirs: &'static [&'static str] = if dir.is_empty() {
            &[]
        } else {
            Box::leak(vec![leak(dir)].into_boxed_slice())
        };
        let agent: &'static Agent = Box::leak(Box::new(Agent {
            id: leak(&a.id),
            name: leak(if a.name.is_empty() { &a.id } else { &a.name }),
            protocol: Protocol::Chat,
            programs,
            dirs,
            small_model: false,
            note: (!a.notice.is_empty()).then(|| leak(&a.notice)),
            kind: Kind::Plugin(plugged),
        }));
        made.insert(key, agent);
        out.push(agent);
    }
    out
}

/// Every agent: pansy's own, then those plugins add.
pub fn all() -> Vec<&'static Agent> {
    AGENTS.iter().chain(plugged()).collect()
}

pub static AGENTS: &[Agent] = &[
    Agent {
        id: "claude-code",
        name: "Claude Code",
        protocol: Protocol::Anthropic,
        programs: &["claude"],
        dirs: &[".claude"],
        small_model: true,
        note: None,
        kind: Kind::ClaudeCode,
    },
    Agent {
        id: "codex",
        name: "Codex",
        protocol: Protocol::Responses,
        programs: &["codex"],
        dirs: &[".codex"],
        small_model: false,
        note: None,
        kind: Kind::Codex,
    },
    Agent {
        id: "gemini-cli",
        name: "Gemini CLI",
        protocol: Protocol::Gemini,
        programs: &["gemini"],
        dirs: &[".gemini"],
        small_model: false,
        note: None,
        kind: Kind::GeminiCli,
    },
    Agent {
        id: "opencode",
        name: "OpenCode",
        protocol: Protocol::Chat,
        programs: &["opencode"],
        dirs: &[".config/opencode"],
        small_model: true,
        note: Some(
            "OpenCode lists every model of pansy under the Pansy provider; /models switches between them.",
        ),
        kind: Kind::OpenCode,
    },
    Agent {
        id: "qwen-code",
        name: "Qwen Code",
        protocol: Protocol::Chat,
        programs: &["qwen"],
        dirs: &[".qwen"],
        small_model: false,
        note: None,
        kind: Kind::QwenCode,
    },
    Agent {
        id: "crush",
        name: "Crush",
        protocol: Protocol::Chat,
        programs: &["crush"],
        dirs: &[".config/crush"],
        small_model: true,
        note: None,
        kind: Kind::Crush,
    },
];

pub fn find(id: &str) -> Option<&'static Agent> {
    AGENTS
        .iter()
        .find(|a| a.id == id)
        .or_else(|| plugged().into_iter().find(|a| a.id == id))
}

fn home(rel: &str) -> PathBuf {
    let mut path = user_home();
    for part in rel.split('/') {
        path.push(part);
    }
    path
}

/// OpenCode reads `opencode.jsonc` when there is one, else `opencode.json`.
fn opencode_config() -> PathBuf {
    let dir = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|d| !d.is_empty())
        .map(|d| PathBuf::from(d).join("opencode"))
        .unwrap_or_else(|| home(".config/opencode"));
    let jsonc = dir.join("opencode.jsonc");
    if jsonc.exists() {
        jsonc
    } else {
        dir.join("opencode.json")
    }
}

fn crush_config() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|d| !d.is_empty())
        .map(|d| PathBuf::from(d).join("crush"))
        .unwrap_or_else(|| home(".config/crush"))
        .join("crush.json")
}

impl Agent {
    /// The plugin's description of an agent a plugin adds.
    pub fn plugged(&self) -> Option<&'static Plugged> {
        match self.kind {
            Kind::Plugin(p) => Some(p),
            _ => None,
        }
    }

    /// The keys that point the agent at the gateway, or why they can't be
    /// known: an agent a plugin adds asks its plugin's `connect`.
    pub fn edits_checked(&self, w: &Wiring) -> anyhow::Result<Vec<Edit>> {
        let Kind::Plugin(p) = self.kind else {
            return Ok(self.edits(w));
        };
        let mut out = self.edits(w);
        if p.connect {
            let models: Vec<(String, String)> =
                w.choices.iter().map(|c| (c.clone(), c.clone())).collect();
            let (id, url, key) = (self.id.to_owned(), w.base_url.to_owned(), w.key.to_owned());
            let model = (!p.model.is_empty() || p.startup.is_some()).then(|| w.model.to_owned());
            let keys = pansy_plugins::block(async move {
                pansy_plugins::agents::connect(&id, &url, &key, model.as_deref(), &models).await
            })?;
            for (k, v) in keys {
                let path: Vec<&str> = k.split('.').filter(|s| !s.is_empty()).collect();
                if !path.is_empty() {
                    out.push(edit(&p.config, p.format, &path, v));
                }
            }
        }
        Ok(out)
    }

    /// The files pansy edits, the main one first.
    pub fn files(&self) -> Vec<PathBuf> {
        match self.kind {
            Kind::Plugin(p) => {
                let mut out = vec![p.config.clone()];
                if let Some((file, ..)) = &p.startup {
                    out.push(file.clone());
                }
                out
            }
            Kind::ClaudeCode => vec![home(".claude/settings.json")],
            Kind::Codex => vec![codex_home().join("config.toml")],
            Kind::GeminiCli => vec![home(".gemini/settings.json"), home(".gemini/.env")],
            Kind::QwenCode => vec![home(".qwen/settings.json"), home(".qwen/.env")],
            Kind::OpenCode => vec![opencode_config()],
            Kind::Crush => vec![crush_config()],
        }
    }

    /// Whether the agent is on this computer.
    pub fn installed(&self) -> bool {
        self.on_path() || self.has_settings()
    }

    /// Whether its program is on `PATH` (a version manager's shim of it
    /// counts).
    pub fn on_path(&self) -> bool {
        self.programs.iter().any(|p| on_path(p))
    }

    /// How to install it here: its vendor's installer first, then its npm
    /// package (pansy doesn't run them; the user does, in a terminal).
    pub fn install(&self) -> Vec<Install> {
        let windows = cfg!(windows);
        let mac = cfg!(target_os = "macos");
        let mut out = Vec::new();
        match self.kind {
            Kind::ClaudeCode => {
                out.push(if windows {
                    install("powershell", "irm https://claude.ai/install.ps1 | iex")
                } else {
                    install("script", "curl -fsSL https://claude.ai/install.sh | bash")
                });
                out.push(install(
                    "npm",
                    "npm install -g --allow-scripts=@anthropic-ai/claude-code @anthropic-ai/claude-code",
                ));
            }
            Kind::Codex => {
                if windows {
                    out.push(install(
                        "powershell",
                        r#"powershell -ExecutionPolicy ByPass -c "irm https://chatgpt.com/codex/install.ps1 | iex""#,
                    ));
                } else {
                    out.push(install(
                        "script",
                        "curl -fsSL https://chatgpt.com/codex/install.sh | sh",
                    ));
                    if mac {
                        out.push(install("brew", "brew install --cask codex"));
                    }
                }
                out.push(install("npm", "npm install -g @openai/codex"));
            }
            Kind::GeminiCli => {
                if !windows {
                    out.push(install("brew", "brew install gemini-cli"));
                }
                out.push(install("npm", "npm install -g @google/gemini-cli"));
            }
            Kind::OpenCode => {
                if !windows {
                    out.push(install(
                        "script",
                        "curl -fsSL https://opencode.ai/v2/install | bash",
                    ));
                }
                out.push(install(
                    "npm",
                    "npm install -g --allow-scripts=@opencode/cli @opencode/cli",
                ));
            }
            Kind::QwenCode => out.push(install("npm", "npm install -g @qwen-code/qwen-code")),
            Kind::Crush => out.push(install(
                "npm",
                "npm install -g --allow-scripts=@charmland/crush @charmland/crush",
            )),
            Kind::Plugin(_) => {}
        }
        out
    }

    /// Whether its settings folder is here (`~/.claude`).
    pub fn has_settings(&self) -> bool {
        self.dirs.iter().any(|d| home(d).is_dir())
    }

    /// Where it picks among the gateway's models, when it lists them:
    /// OpenCode's `/models`, Crush's `ctrl+p`.
    pub fn picks_in(&self) -> Option<&'static str> {
        match self.kind {
            Kind::OpenCode => Some("/models"),
            Kind::Crush => Some("ctrl+p"),
            _ => None,
        }
    }

    /// The keys that point the agent at the gateway with `w.model`.
    pub fn edits(&self, w: &Wiring) -> Vec<Edit> {
        let files = self.files();
        let main = &files[0];
        let small = w.small_model.unwrap_or(w.model);
        let v1 = format!("{}/v1", w.base_url);
        match self.kind {
            // The model's keys; what `connect` gives comes in edits_checked.
            Kind::Plugin(p) => {
                let model = format!("{}{}", p.prefix, w.model);
                let mut out = Vec::new();
                if !p.model.is_empty() {
                    let path: Vec<&str> = p.model.iter().map(String::as_str).collect();
                    out.push(edit(&p.config, p.format, &path, json!(model)));
                }
                if let Some((file, format, key, keys)) = &p.startup {
                    if !key.is_empty() {
                        let path: Vec<&str> = key.iter().map(String::as_str).collect();
                        out.push(edit(file, *format, &path, json!(model)));
                    }
                    for (k, v) in keys {
                        let path: Vec<&str> = k.split('.').filter(|s| !s.is_empty()).collect();
                        if !path.is_empty() {
                            out.push(edit(file, *format, &path, v.clone()));
                        }
                    }
                }
                out
            }
            Kind::ClaudeCode => {
                let env =
                    |key: &str, value: &str| edit(main, Format::Json, &["env", key], json!(value));
                vec![
                    env("ANTHROPIC_BASE_URL", w.base_url),
                    env("ANTHROPIC_AUTH_TOKEN", w.key),
                    env("ANTHROPIC_MODEL", w.model),
                    env("ANTHROPIC_DEFAULT_OPUS_MODEL", w.model),
                    env("ANTHROPIC_DEFAULT_SONNET_MODEL", w.model),
                    env("ANTHROPIC_DEFAULT_HAIKU_MODEL", small),
                    env("ANTHROPIC_SMALL_FAST_MODEL", small),
                ]
            }
            Kind::Codex => vec![
                edit(main, Format::Toml, &["model"], json!(w.model)),
                edit(main, Format::Toml, &["model_provider"], json!("pansy")),
                edit(
                    main,
                    Format::Toml,
                    &["model_providers", "pansy", "name"],
                    json!("Pansy"),
                ),
                edit(
                    main,
                    Format::Toml,
                    &["model_providers", "pansy", "base_url"],
                    json!(v1),
                ),
                edit(
                    main,
                    Format::Toml,
                    &["model_providers", "pansy", "wire_api"],
                    json!("responses"),
                ),
                edit(
                    main,
                    Format::Toml,
                    &["model_providers", "pansy", "experimental_bearer_token"],
                    json!(w.key),
                ),
            ],
            Kind::GeminiCli => vec![
                edit(main, Format::Json, &["model", "name"], json!(w.model)),
                edit(
                    main,
                    Format::Json,
                    &["security", "auth", "selectedType"],
                    json!("gemini-api-key"),
                ),
                edit(
                    &files[1],
                    Format::Env,
                    &["GOOGLE_GEMINI_BASE_URL"],
                    json!(w.base_url),
                ),
                edit(&files[1], Format::Env, &["GEMINI_API_KEY"], json!(w.key)),
            ],
            Kind::QwenCode => vec![
                edit(main, Format::Json, &["model", "name"], json!(w.model)),
                edit(
                    main,
                    Format::Json,
                    &["security", "auth", "selectedType"],
                    json!("openai"),
                ),
                edit(&files[1], Format::Env, &["OPENAI_BASE_URL"], json!(v1)),
                edit(&files[1], Format::Env, &["OPENAI_API_KEY"], json!(w.key)),
                edit(&files[1], Format::Env, &["OPENAI_MODEL"], json!(w.model)),
            ],
            Kind::OpenCode => {
                let mut models = serde_json::Map::new();
                for id in w
                    .choices
                    .iter()
                    .chain([w.model.to_owned(), small.to_owned()].iter())
                {
                    models.insert(id.clone(), json!({"name": id}));
                }
                vec![
                    edit(
                        main,
                        Format::Json,
                        &["provider", "pansy"],
                        json!({
                            "npm": "@ai-sdk/openai-compatible",
                            "name": "Pansy",
                            "options": {"baseURL": v1, "apiKey": w.key},
                            "models": models,
                        }),
                    ),
                    edit(
                        main,
                        Format::Json,
                        &["model"],
                        json!(format!("pansy/{}", w.model)),
                    ),
                    edit(
                        main,
                        Format::Json,
                        &["small_model"],
                        json!(format!("pansy/{small}")),
                    ),
                ]
            }
            Kind::Crush => {
                let mut ids: Vec<&str> = w.choices.iter().map(String::as_str).collect();
                for id in [w.model, small] {
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                }
                let models: Vec<Value> = ids
                    .iter()
                    .map(|id| json!({"id": id, "name": id, "context_window": 128000, "default_max_tokens": 16000}))
                    .collect();
                vec![
                    edit(
                        main,
                        Format::Json,
                        &["providers", "pansy"],
                        json!({"name": "Pansy", "type": "openai", "base_url": v1, "api_key": w.key, "models": models}),
                    ),
                    edit(
                        main,
                        Format::Json,
                        &["models", "large"],
                        json!({"model": w.model, "provider": "pansy"}),
                    ),
                    edit(
                        main,
                        Format::Json,
                        &["models", "small"],
                        json!({"model": small, "provider": "pansy"}),
                    ),
                ]
            }
        }
    }

    /// Where the agent's base URL is, to tell whether it points at pansy.
    pub fn base_url_key(&self) -> (PathBuf, Format, Vec<&'static str>) {
        let files = self.files();
        match self.kind {
            Kind::Plugin(_) => self.model_key(),
            Kind::ClaudeCode => (
                files[0].clone(),
                Format::Json,
                vec!["env", "ANTHROPIC_BASE_URL"],
            ),
            Kind::Codex => (
                files[0].clone(),
                Format::Toml,
                vec!["model_providers", "pansy", "base_url"],
            ),
            Kind::GeminiCli => (
                files[1].clone(),
                Format::Env,
                vec!["GOOGLE_GEMINI_BASE_URL"],
            ),
            Kind::QwenCode => (files[1].clone(), Format::Env, vec!["OPENAI_BASE_URL"]),
            Kind::OpenCode => (
                files[0].clone(),
                Format::Json,
                vec!["provider", "pansy", "options", "baseURL"],
            ),
            Kind::Crush => (
                files[0].clone(),
                Format::Json,
                vec!["providers", "pansy", "base_url"],
            ),
        }
    }

    /// Where the agent's model is.
    pub fn model_key(&self) -> (PathBuf, Format, Vec<&'static str>) {
        let files = self.files();
        match self.kind {
            Kind::Plugin(p) => {
                let (file, format) = match &p.startup {
                    Some((file, format, key, _)) if !key.is_empty() => (file.clone(), *format),
                    _ => (p.config.clone(), p.format),
                };
                (file, format, p.model_key.to_vec())
            }
            Kind::ClaudeCode => (
                files[0].clone(),
                Format::Json,
                vec!["env", "ANTHROPIC_MODEL"],
            ),
            Kind::Codex => (files[0].clone(), Format::Toml, vec!["model"]),
            Kind::GeminiCli | Kind::QwenCode => {
                (files[0].clone(), Format::Json, vec!["model", "name"])
            }
            Kind::OpenCode => (files[0].clone(), Format::Json, vec!["model"]),
            Kind::Crush => (
                files[0].clone(),
                Format::Json,
                vec!["models", "large", "model"],
            ),
        }
    }

    /// The model as the agent's config names it, without what the agent adds.
    pub fn model_of(&self, raw: &str) -> String {
        match self.kind {
            Kind::Plugin(p) => raw
                .strip_prefix(p.prefix.as_str())
                .unwrap_or(raw)
                .to_owned(),
            Kind::OpenCode => raw.strip_prefix("pansy/").unwrap_or(raw).to_owned(),
            _ => raw.to_owned(),
        }
    }
}

/// `CODEX_HOME`, else `~/.codex`.
pub(crate) fn codex_home() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home(".codex"))
}

/// Whether `program` is on `PATH`.
pub fn on_path(program: &str) -> bool {
    crate::cli::which(program).is_some()
}
