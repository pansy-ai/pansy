//! The agents pansy wires: where each keeps its config, and the keys that
//! point it at the gateway with a model.
//!
//! An agent is data: a manifest (`agents/*.toml`, read by [`crate::manifest`])
//! says how it is found, which files it keeps its config in, the keys to set
//! there and how one run of it is pointed elsewhere. pansy's own six are
//! built in; the user's are the files in `agents/` of pansy's folder; an
//! agent plugin (`magpie.agent`) adds the rest.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use pansy_core::paths::user_home;
use pansy_core::provider::Protocol;
use serde::Serialize;
use serde_json::{Value, json};

use crate::files::Format;
use crate::manifest::{self, Manifest, Vars};

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

/// How one run of an agent is pointed at the gateway with a model, its
/// config left alone: what its environment gains and the arguments put
/// before the user's.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Run {
    pub env: Vec<(String, String)>,
    pub args: Vec<String>,
    /// The agent's version this was tried on; `None`: not tried on one.
    pub verified: Option<&'static str>,
    /// Something the user should know about a run of it.
    pub note: Option<&'static str>,
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

#[derive(Debug, Clone, Copy)]
enum Kind {
    /// One a manifest describes.
    Listed(&'static Manifest),
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
pub(crate) fn format_of(format: &str, file: &Path) -> Format {
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

/// Every agent: pansy's own, the user's manifests', then those plugins add.
pub fn all() -> Vec<&'static Agent> {
    let mut out: Vec<&'static Agent> = AGENTS.iter().collect();
    for agent in listed() {
        if !out.iter().any(|a| a.id == agent.id) {
            out.push(agent);
        }
    }
    for agent in plugged() {
        if !out.iter().any(|a| a.id == agent.id) {
            out.push(agent);
        }
    }
    out
}

fn agent_of(m: &'static Manifest) -> Agent {
    Agent {
        id: &m.id,
        name: &m.name,
        protocol: m.protocol,
        programs: &m.programs,
        dirs: &m.dirs,
        small_model: m.small_model,
        note: m.note,
        kind: Kind::Listed(m),
    }
}

/// pansy's own agents, from the manifests built in.
pub static AGENTS: LazyLock<Vec<Agent>> =
    LazyLock::new(|| manifest::builtin().iter().map(agent_of).collect());

/// The agents the user's manifests describe (`agents/*.toml` in pansy's
/// folder), in their files' order: each made once for a file's text, so
/// reading them again leaks nothing more. One at an id pansy has is passed
/// over; [`problems`] says so.
pub fn listed() -> Vec<&'static Agent> {
    use std::collections::HashMap;
    use std::sync::Mutex;
    static MADE: LazyLock<Mutex<HashMap<String, &'static Agent>>> = LazyLock::new(Default::default);
    let mut made = MADE.lock().unwrap_or_else(|e| e.into_inner());
    let mut out: Vec<&'static Agent> = Vec::new();
    for (_, text) in manifest::user_files() {
        let agent = match made.get(&text) {
            Some(agent) => agent,
            None => {
                let Ok(m) = manifest::parse(&text) else {
                    continue;
                };
                let agent: &'static Agent = Box::leak(Box::new(agent_of(Box::leak(Box::new(m)))));
                made.insert(text, agent);
                agent
            }
        };
        if AGENTS.iter().any(|a| a.id == agent.id) || out.iter().any(|a| a.id == agent.id) {
            continue;
        }
        out.push(agent);
    }
    out
}

/// What is wrong with the user's manifests, a line each: one that can't be
/// read, or whose id another agent has.
pub fn problems() -> Vec<String> {
    let mut seen: Vec<String> = AGENTS.iter().map(|a| a.id.to_owned()).collect();
    let mut out = Vec::new();
    for (file, text) in manifest::user_files() {
        match manifest::parse(&text) {
            Err(e) => out.push(format!("{}: {e:#}", file.display())),
            Ok(m) if seen.contains(&m.id) => out.push(format!(
                "{}: pansy already has an agent {}",
                file.display(),
                m.id
            )),
            Ok(m) => seen.push(m.id),
        }
    }
    out
}

pub fn find(id: &str) -> Option<&'static Agent> {
    all().into_iter().find(|a| a.id == id)
}

fn home(rel: &str) -> PathBuf {
    let mut path = user_home();
    for part in rel.split('/') {
        path.push(part);
    }
    path
}

impl Agent {
    /// The plugin's description of an agent a plugin adds.
    pub fn plugged(&self) -> Option<&'static Plugged> {
        match self.kind {
            Kind::Plugin(p) => Some(p),
            Kind::Listed(_) => None,
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
            Kind::Listed(m) => m.files.iter().map(|(_, f)| f.resolve()).collect(),
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
        match self.kind {
            Kind::Listed(m) => m
                .install
                .iter()
                .filter(|i| i.here())
                .map(|i| Install {
                    via: i.via,
                    command: i.command,
                })
                .collect(),
            Kind::Plugin(_) => Vec::new(),
        }
    }

    /// Whether its settings folder is here (`~/.claude`).
    pub fn has_settings(&self) -> bool {
        self.dirs.iter().any(|d| home(d).is_dir())
    }

    /// Where it picks among the gateway's models, when it lists them:
    /// OpenCode's `/models`, Crush's `ctrl+p`.
    pub fn picks_in(&self) -> Option<&'static str> {
        match self.kind {
            Kind::Listed(m) => m.picks_in,
            Kind::Plugin(_) => None,
        }
    }

    /// The keys that point the agent at the gateway with `w.model`.
    pub fn edits(&self, w: &Wiring) -> Vec<Edit> {
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
            Kind::Listed(m) => {
                let vars = Vars::of(w);
                m.set
                    .iter()
                    .map(|s| {
                        let file = m.file(s.file);
                        Edit {
                            format: format_of("", &file),
                            file,
                            path: s.key.iter().map(|k| (*k).to_owned()).collect(),
                            value: vars.fill(&s.value),
                        }
                    })
                    .collect()
            }
        }
    }

    /// How one run of the agent is pointed at the gateway with `w.model`,
    /// its config left alone; why it can't be, for one that reads its
    /// config files alone.
    pub fn run(&self, w: &Wiring) -> Result<Run, String> {
        let Kind::Listed(m) = self.kind else {
            return Err(format!(
                "{} is added by a plugin, which doesn't say how one run of it is pointed elsewhere; connect it instead",
                self.name
            ));
        };
        let Some(run) = &m.run else {
            return Err(format!(
                "{}'s manifest doesn't say how one run of it is pointed elsewhere; connect it instead",
                self.name
            ));
        };
        if let Some(why) = run.unsupported {
            return Err(why.to_owned());
        }
        let vars = Vars::of(w);
        let mut env: Vec<(String, String)> = run
            .env
            .iter()
            .map(|(k, v)| ((*k).to_owned(), vars.text(v)))
            .collect();
        for (name, value) in &run.json_env {
            let mut value = vars.fill(value);
            // Laid over what the variable holds already, when both are objects.
            if let Some(mut theirs) = std::env::var(name)
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .filter(Value::is_object)
            {
                manifest::lay_over(&mut theirs, &value);
                value = theirs;
            }
            env.push(((*name).to_owned(), value.to_string()));
        }
        let mut args = Vec::new();
        for (flag, value) in &run.json_arg {
            args.push((*flag).to_owned());
            args.push(vars.fill(value).to_string());
        }
        args.extend(run.args.iter().map(|a| vars.text(a)));
        Ok(Run {
            env,
            args,
            verified: run.verified,
            note: run.note,
        })
    }

    /// Where the agent's base URL is, to tell whether it points at pansy.
    pub fn base_url_key(&self) -> (PathBuf, Format, Vec<&'static str>) {
        match self.kind {
            Kind::Plugin(_) => self.model_key(),
            Kind::Listed(m) => {
                let file = m.file(m.base_url.file);
                let format = format_of("", &file);
                (file, format, m.base_url.key.clone())
            }
        }
    }

    /// Where the agent's model is.
    pub fn model_key(&self) -> (PathBuf, Format, Vec<&'static str>) {
        match self.kind {
            Kind::Plugin(p) => {
                let (file, format) = match &p.startup {
                    Some((file, format, key, _)) if !key.is_empty() => (file.clone(), *format),
                    _ => (p.config.clone(), p.format),
                };
                (file, format, p.model_key.to_vec())
            }
            Kind::Listed(m) => {
                let file = m.file(m.model.file);
                let format = format_of("", &file);
                (file, format, m.model.key.clone())
            }
        }
    }

    /// The model as the agent's config names it, without what the agent adds.
    pub fn model_of(&self, raw: &str) -> String {
        let prefix = match self.kind {
            Kind::Plugin(p) => p.prefix.as_str(),
            Kind::Listed(m) => m.model.prefix,
        };
        raw.strip_prefix(prefix).unwrap_or(raw).to_owned()
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
