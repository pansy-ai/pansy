//! The coding agents pansy wires to its gateway: finding them, pointing
//! them at it with a model, and putting their configs back.
//!
//! Every key pansy sets is recorded with the value it replaced
//! (`agents.json`), and the first time pansy changes a file a copy of it is
//! kept (`backups/`). Disconnecting sets those keys back and leaves the rest
//! of the file, changed by the user since or not, as it is.

pub mod agent;
pub mod cli;
pub mod doctor;
pub mod files;
pub mod import;
pub mod jsonedit;
pub mod manifest;
pub mod preview;
pub mod profile;
pub mod run;
pub mod sessions;
pub mod state;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use pansy_core::config;
use pansy_core::fsutil::write_atomic;
use pansy_core::paths;
use pansy_core::provider::Protocol;
use serde::Serialize;
use serde_json::Value;

pub use agent::{AGENTS, Agent, Install};
use state::{State, Touched};

/// The key agents send the gateway; it takes any from this computer.
pub const AGENT_KEY: &str = "pansy";

/// One of an agent's fields as the window and the command line show it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldView {
    pub key: &'static str,
    pub kind: manifest::FieldKind,
    /// An effort's levels, the lowest first.
    pub levels: Vec<&'static str>,
    /// With none given, the main model's.
    pub follows: bool,
    /// The least and the most a size may be.
    pub min: Option<u64>,
    pub max: Option<u64>,
    /// What the user gave it, while the agent is connected.
    pub value: Option<String>,
}

/// One agent as the window and `pansy agents` show it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentView {
    pub id: &'static str,
    pub name: &'static str,
    pub protocol: Protocol,
    pub installed: bool,
    /// Its settings are here, its program isn't (uninstalled, its folder
    /// left behind).
    pub cli_missing: bool,
    /// How to install it, for one that isn't here.
    pub install: Vec<Install>,
    /// Where it picks among the gateway's models, for one that lists them.
    pub picks_in: Option<&'static str>,
    /// Its config points at this pansy's gateway.
    pub connected: bool,
    pub model: Option<String>,
    pub small_model: Option<String>,
    pub has_small_model: bool,
    /// The model its subagents start on, as pansy set it.
    pub subagent_model: Option<String>,
    /// It has a key for that model.
    pub has_subagent_model: bool,
    /// Every subagent of it goes on that model, whatever one its lead
    /// names: the gateway sees to it.
    pub subagent_every: bool,
    /// What else may be set for it: a tier's model, an effort, a setting
    /// of its own.
    pub fields: Vec<FieldView>,
    /// It lists the gateway's models itself: which ones is the user's pick.
    pub lists_models: bool,
    /// Not connected, it is signed in to its vendor (its Claude
    /// subscription, ChatGPT): what it is on now.
    pub signed_in: bool,
    pub config_files: Vec<String>,
    pub recent: Vec<String>,
    pub note: Option<&'static str>,
    /// Put back for now, off the gateway: the model to connect it again with.
    pub paused: Option<String>,
}

fn read_key(file: &Path, format: files::Format, path: &[&str]) -> Option<String> {
    let text = files::read_text(file).ok()?;
    match files::get(format, &text, path).ok()?? {
        Value::String(s) => Some(s),
        other => Some(other.to_string()),
    }
}

pub(crate) fn gateway_url() -> String {
    config::load_settings().unwrap_or_default().base_url()
}

/// Whether `url` is this pansy's gateway (with or without `/v1`).
fn is_ours(url: &str, base: &str) -> bool {
    let url = url.trim_end_matches('/');
    url == base || url == format!("{base}/v1") || url == base.replace("127.0.0.1", "localhost")
}

pub fn view(agent: &Agent, state: &State) -> AgentView {
    let saved = state.agents.get(agent.id);
    let (mfile, mformat, mpath) = agent.model_key();
    // An agent a plugin adds says nothing of the gateway where pansy can
    // read it: it is connected while pansy's keys are in its config.
    let connected = match agent.plugged() {
        Some(_) => saved.is_some_and(|s| s.model.is_some() && !s.touched.is_empty()),
        None => {
            let base = gateway_url();
            let (file, format, path) = agent.base_url_key();
            if path.is_empty() {
                // No key says where it points: by what pansy wrote.
                saved.is_some_and(|s| s.model.is_some() && !s.touched.is_empty())
            } else {
                read_key(&file, format, &path).is_some_and(|url| is_ours(&url, &base))
            }
        }
    };
    let model = if !connected {
        None
    } else if mpath.is_empty() {
        saved.and_then(|s| s.model.clone())
    } else {
        read_key(&mfile, mformat, &mpath).map(|m| agent.model_of(&m))
    };
    AgentView {
        id: agent.id,
        name: agent.name,
        protocol: agent.protocol,
        installed: agent.installed(),
        cli_missing: agent.cli_missing(),
        install: agent.install(),
        picks_in: agent.picks_in(),
        connected,
        model,
        small_model: saved.and_then(|s| s.small_model.clone()),
        has_small_model: agent.small_model,
        subagent_model: saved
            .and_then(|s| s.subagent_model.clone())
            .filter(|_| connected && agent.subagent_model),
        has_subagent_model: agent.subagent_model,
        subagent_every: agent.subagent_every,
        fields: agent
            .fields()
            .iter()
            .map(|f| FieldView {
                key: f.key,
                kind: f.kind,
                levels: f.levels.clone(),
                follows: f.follows,
                min: f.min,
                max: f.max,
                value: saved
                    .and_then(|s| s.fields.get(f.key).cloned())
                    .filter(|_| connected),
            })
            .collect(),
        lists_models: agent.lists_models(),
        signed_in: !connected && agent.signed_in(),
        config_files: agent
            .files()
            .iter()
            .map(|f| f.display().to_string())
            .collect(),
        recent: saved.map(|s| s.recent.clone()).unwrap_or_default(),
        note: agent.note,
        paused: saved.and_then(|s| s.paused.clone()).filter(|_| !connected),
    }
}

/// Every agent pansy knows, installed ones first.
pub fn list() -> Vec<AgentView> {
    let state = State::load().unwrap_or_default();
    let mut views: Vec<AgentView> = agent::all().into_iter().map(|a| view(a, &state)).collect();
    views.sort_by_key(|v| !(v.installed || v.connected));
    views
}

/// The agents whose programs are being looked at, and what is known of each.
struct Asking {
    /// Its version is being read.
    running: BTreeSet<&'static str>,
    /// The version that is out is being asked, or it is being updated.
    looking: BTreeSet<&'static str>,
    said: BTreeMap<&'static str, cli::Cli>,
}

static ASKING: Mutex<Asking> = Mutex::new(Asking {
    running: BTreeSet::new(),
    looking: BTreeSet::new(),
    said: BTreeMap::new(),
});
static ANSWERED: Condvar = Condvar::new();

/// Each agent's program on `PATH`: its version as far as it is known
/// within `wait`, how it is updated and the version that is out, and
/// whether some are still being asked: those go on, and are ready the next
/// time. One asking per agent at a time; the version that is out, asked
/// over the network, never holds a version up.
pub fn clis(wait: Duration) -> (BTreeMap<&'static str, cli::Cli>, bool) {
    let here: Vec<&'static Agent> = agent::all().into_iter().filter(|a| a.on_path()).collect();
    let Ok(mut asking) = ASKING.lock() else {
        return (BTreeMap::new(), false);
    };
    for &a in &here {
        if asking.looking.contains(a.id) || !asking.running.insert(a.id) {
            continue;
        }
        std::thread::spawn(move || {
            let (mut cli, how) = cli::installed(a.programs, a.own_updater());
            if let Ok(mut asking) = ASKING.lock() {
                asking.running.remove(a.id);
                // What was out stays said while it is asked again.
                if let Some(was) = asking.said.get(a.id).filter(|w| w.version == cli.version) {
                    cli.latest = was.latest.clone();
                    cli.update = was.update;
                }
                asking.said.insert(a.id, cli.clone());
                if how.is_some() {
                    asking.looking.insert(a.id);
                }
            }
            ANSWERED.notify_all();
            let Some(how) = how else {
                return;
            };
            let cli = cli::with_latest(cli, &how);
            if let Ok(mut asking) = ASKING.lock() {
                asking.looking.remove(a.id);
                asking.said.insert(a.id, cli);
            }
            ANSWERED.notify_all();
        });
    }
    let running = |asking: &mut Asking| here.iter().any(|a| asking.running.contains(a.id));
    let Ok((asking, _)) = ANSWERED.wait_timeout_while(asking, wait, running) else {
        return (BTreeMap::new(), true);
    };
    let known = here
        .iter()
        .filter_map(|a| Some((a.id, asking.said.get(a.id)?.clone())))
        .collect();
    let pending = here
        .iter()
        .any(|a| asking.running.contains(a.id) || asking.looking.contains(a.id));
    (known, pending)
}

/// The version of each agent's program on `PATH`, as far as it is known
/// within `wait`, and whether some are still being read ([`clis`]).
pub fn versions(wait: Duration) -> (BTreeMap<&'static str, String>, bool) {
    let (known, _) = clis(wait);
    let reading = ASKING
        .lock()
        .map(|asking| !asking.running.is_empty())
        .unwrap_or(false);
    let known = known
        .into_iter()
        .filter_map(|(id, cli)| Some((id, cli.version?)))
        .collect();
    (known, reading)
}

/// Updates the agent's program with the command its install tells
/// ([`cli::how`]), which may take minutes, and reads its version again.
/// One update of an agent at a time.
pub fn update_cli(id: &str) -> Result<cli::Cli> {
    let agent = find(id)?;
    let (was, how) = cli::installed(agent.programs, agent.own_updater());
    let how = how.ok_or_else(|| {
        anyhow!(
            "pansy can't tell how {} was installed: update it the way you installed it",
            agent.name
        )
    })?;
    {
        let mut asking = ASKING.lock().map_err(|_| anyhow!("the agents' state"))?;
        if !asking.looking.insert(agent.id) {
            bail!("{} is being looked at or updated already", agent.name);
        }
    }
    let before = cli::real_of(agent.programs);
    let done = cli::update(&how);
    // Its version where the update left it: the program may be another
    // file, or the same one beside a package that changed.
    for real in before.into_iter().chain(cli::real_of(agent.programs)) {
        cli::forget(&real);
    }
    let (now, how_now) = cli::installed(agent.programs, agent.own_updater());
    let now = match &how_now {
        Some(how) => cli::with_latest(now, how),
        None => now,
    };
    if let Ok(mut asking) = ASKING.lock() {
        asking.looking.remove(agent.id);
        asking.said.insert(agent.id, now.clone());
    }
    ANSWERED.notify_all();
    done.map_err(|why| anyhow!(why))?;
    if now.version.is_some() && now.version == was.version && now.update {
        bail!(
            "{} is still {} after `{}`",
            agent.name,
            now.version.as_deref().unwrap_or_default(),
            how.command.join(" ")
        );
    }
    Ok(now)
}

/// One key pansy set in an agent's config: what was there before, and
/// what is there now (`None`: nothing). Keys are masked.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KeyChange {
    pub key: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

/// The keys pansy set in one file of an agent's config.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    pub path: String,
    /// pansy made the file: disconnecting removes it.
    pub created: bool,
    pub keys: Vec<KeyChange>,
}

/// Whether the value at a key named `name` is a secret to mask.
fn secret_key(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["key", "token", "secret", "password"]
        .iter()
        .any(|w| n.contains(w))
}

/// `value` as a change shows it: a secret masked (pansy's own key is
/// none), an object as one line, cut short.
pub(crate) fn shown(name: &str, value: &Value) -> String {
    fn hide(name: &str, value: &Value) -> Value {
        match value {
            Value::String(s) if secret_key(name) && s != AGENT_KEY => {
                Value::String(pansy_core::provider::mask_key(s))
            }
            Value::Object(map) => {
                Value::Object(map.iter().map(|(k, v)| (k.clone(), hide(k, v))).collect())
            }
            Value::Array(items) => Value::Array(items.iter().map(|v| hide(name, v)).collect()),
            other => other.clone(),
        }
    }
    let text = match hide(name, value) {
        Value::String(s) => s,
        other => other.to_string(),
    };
    const MAX: usize = 120;
    if text.chars().count() > MAX {
        let cut: String = text.chars().take(MAX).collect();
        format!("{cut}…")
    } else {
        text
    }
}

/// The format of each file pansy edits for `agent`, by its path.
fn formats(agent: &Agent) -> BTreeMap<String, files::Format> {
    let mut out: BTreeMap<String, files::Format> = BTreeMap::new();
    if let Some(p) = agent.plugged() {
        out.insert(p.config.display().to_string(), p.format);
        if let Some((file, format, ..)) = &p.startup {
            out.insert(file.display().to_string(), *format);
        }
    }
    // With a model for subagents and every field given, so the keys for
    // them are among them.
    let fields: BTreeMap<String, String> = agent
        .fields()
        .iter()
        .map(|f| (f.key.to_owned(), String::new()))
        .collect();
    out.extend(
        agent
            .edits(&agent::Wiring {
                base_url: "",
                key: "",
                model: "",
                small_model: None,
                subagent_model: Some(""),
                fields: &fields,
                choices: &[],
                names: agent::no_fields(),
            })
            .into_iter()
            .map(|e| (e.file.display().to_string(), e.format)),
    );
    out
}

/// What pansy changed in the agent's config, file by file: what
/// disconnecting puts back.
pub fn changes(id: &str) -> Result<Vec<FileChange>> {
    let agent = find(id)?;
    let state = State::load()?;
    let Some(record) = state.agents.get(id) else {
        return Ok(Vec::new());
    };
    let formats = formats(agent);
    let mut out: Vec<FileChange> = Vec::new();
    for t in &record.touched {
        let Some(format) = formats.get(&t.file).copied() else {
            continue;
        };
        let text = files::read_text(Path::new(&t.file))?;
        let key: Vec<&str> = t.path.iter().map(String::as_str).collect();
        let now = files::get(format, &text, &key).ok().flatten();
        let name = t.path.last().map(String::as_str).unwrap_or_default();
        let change = KeyChange {
            key: t.path.join("."),
            before: t.before.as_ref().map(|v| shown(name, v)),
            after: now.as_ref().map(|v| shown(name, v)),
        };
        match out.iter_mut().find(|f| f.path == t.file) {
            Some(f) => f.keys.push(change),
            None => out.push(FileChange {
                path: t.file.clone(),
                created: record
                    .files
                    .get(&t.file)
                    .is_some_and(|f| f.before.is_none()),
                keys: vec![change],
            }),
        }
    }
    Ok(out)
}

pub(crate) fn find(id: &str) -> Result<&'static Agent> {
    agent::find(id)
        .ok_or_else(|| anyhow!("pansy does not know an agent {id}; `pansy agents` lists them"))
}

/// The gateway's models as the agent `id` lists them: the ones the user left
/// in its list (Model list › Pick), and the name each model goes by there,
/// after its provider's unless Provider in model names is off for it.
pub(crate) fn offered(id: &str) -> (Vec<String>, BTreeMap<String, String>) {
    let settings = config::load_settings().unwrap_or_default();
    let with_provider = settings.names_provider(id);
    let all = pansy_core::manage::choices().unwrap_or_default();
    let names = all
        .iter()
        .map(|c| (c.id.clone(), c.label(with_provider)))
        .collect();
    let choices = all
        .into_iter()
        .map(|c| c.id)
        .filter(|model| settings.lists(id, model))
        .collect();
    (choices, names)
}

/// Copies `file` into `backups/<agent>/` the first time pansy changes it.
fn back_up(agent: &str, file: &Path) -> Result<()> {
    if !file.exists() {
        return Ok(());
    }
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "config".into());
    let dir = paths::backups().join(agent);
    let copy = dir.join(format!("{name}.before-pansy"));
    if copy.exists() {
        return Ok(());
    }
    fs::create_dir_all(&dir)?;
    fs::copy(file, &copy).with_context(|| format!("backing up {}", file.display()))?;
    Ok(())
}

/// What an agent is wired with beside its main model.
#[derive(Debug, Clone, Default)]
struct Others<'a> {
    /// The model for quick tasks.
    small: Option<&'a str>,
    /// The model for subagents that name none.
    subagent: Option<&'a str>,
    /// Its fields' values, by their keys.
    fields: BTreeMap<String, String>,
}

impl<'a> Others<'a> {
    /// Those `state` keeps for the agent `id`.
    fn saved(state: &'a State, id: &str) -> Others<'a> {
        let saved = state.agents.get(id);
        Others {
            small: saved.and_then(|s| s.small_model.as_deref()),
            subagent: saved.and_then(|s| s.subagent_model.as_deref()),
            fields: saved.map(|s| s.fields.clone()).unwrap_or_default(),
        }
    }
}

/// Sets the agent's keys for `model` (and its other models), noting what
/// each replaced. A key for a model the user gives none of any more goes
/// back to what it was.
fn wire(agent: &Agent, model: &str, others: Others, state: &mut State) -> Result<()> {
    let small = others.small;
    let subagent = others.subagent.filter(|_| agent.subagent_model);
    // A field the agent has no more (its manifest changed) is let go.
    let mut fields = others.fields;
    fields.retain(|key, _| agent.fields().iter().any(|f| f.key == key));
    let base = gateway_url();
    let (choices, names) = offered(agent.id);
    let wiring = agent::Wiring {
        base_url: &base,
        key: AGENT_KEY,
        model,
        small_model: small,
        subagent_model: subagent,
        fields: &fields,
        choices: &choices,
        names: &names,
    };
    let edits = agent.edits_checked(&wiring)?;
    let left_out = agent.left_out(&wiring);
    let mut by_file: BTreeMap<PathBuf, (Vec<&agent::Edit>, Vec<&agent::Edit>)> = BTreeMap::new();
    for e in &edits {
        by_file.entry(e.file.clone()).or_default().0.push(e);
    }
    for e in &left_out {
        by_file.entry(e.file.clone()).or_default().1.push(e);
    }
    // A file pansy set keys in and sets none in now is looked at too: its
    // keys go back.
    let formats = formats(agent);
    let record = state.agent(agent.id);
    for t in &record.touched {
        by_file.entry(PathBuf::from(&t.file)).or_default();
    }
    for (file, (edits, left_out)) in by_file {
        let mut text = files::read_text(&file)?;
        let original = text.clone();
        let file_key = file.display().to_string();
        let first = !record.touched.iter().any(|t| t.file == file_key);
        // Changed by someone else since pansy wrote it: what it was before
        // is not known whole any more, only key by key.
        if !first
            && record
                .files
                .get(&file_key)
                .is_some_and(|f| f.wrote != state::digest(&original))
        {
            record.files.remove(&file_key);
        }
        let mut set: Vec<&Vec<String>> = Vec::new();
        for e in edits {
            let path: Vec<&str> = e.path.iter().map(String::as_str).collect();
            let mine = record
                .touched
                .iter()
                .any(|t| t.file == file_key && t.path == e.path);
            if !mine {
                if e.if_there && !holds(e.format, &text, &path)? {
                    continue;
                }
                let before = files::get(e.format, &text, &path)
                    .with_context(|| format!("reading {}", file.display()))?;
                if e.remove && before.is_none() {
                    continue;
                }
                record.touched.push(Touched {
                    file: file_key.clone(),
                    path: e.path.clone(),
                    before,
                });
            }
            set.push(&e.path);
            text = files::set(e.format, &text, &path, (!e.remove).then_some(&e.value))
                .with_context(|| format!("editing {}", file.display()))?;
        }
        // A key pansy set for a model there is none of now: as it was.
        for e in left_out {
            let Some(at) = record
                .touched
                .iter()
                .position(|t| t.file == file_key && t.path == e.path)
            else {
                continue;
            };
            let was = record.touched.remove(at);
            let path: Vec<&str> = e.path.iter().map(String::as_str).collect();
            text = files::set(e.format, &text, &path, was.before.as_ref())
                .with_context(|| format!("editing {}", file.display()))?;
        }
        // And a key a manifest set before and sets no more (a model gone
        // from the list, a table's key): as it was, the last set first.
        if agent.plugged().is_none()
            && let Some(format) = formats.get(&file_key).copied()
        {
            let mut at = record.touched.len();
            while at > 0 {
                at -= 1;
                let t = &record.touched[at];
                if t.file != file_key || set.contains(&&t.path) {
                    continue;
                }
                let was = record.touched.remove(at);
                let path: Vec<&str> = was.path.iter().map(String::as_str).collect();
                text = files::set(format, &text, &path, was.before.as_ref())
                    .with_context(|| format!("editing {}", file.display()))?;
            }
        }
        if text != original {
            back_up(agent.id, &file)?;
            let existed = file.exists();
            if first {
                record.files.insert(
                    file_key.clone(),
                    state::FileState {
                        before: existed.then(|| original.clone()),
                        wrote: String::new(),
                    },
                );
            }
            // A `.env` pansy makes holds a key: only the user reads it. One
            // that was there keeps its mode.
            let private = !existed
                && (file.extension().is_some_and(|x| x == "env")
                    || file.file_name().is_some_and(|n| n == ".env"));
            write_atomic(&file, text.as_bytes(), private)?;
            if let Some(f) = record.files.get_mut(&file_key) {
                f.wrote = state::digest(&text);
            }
        }
    }
    record.model = Some(model.to_owned());
    record.small_model = small.map(str::to_owned);
    record.subagent_model = subagent.map(str::to_owned);
    record.fields = fields;
    record.paused = None;
    record.remember(model);
    Ok(())
}

/// Points the agent at the gateway with `model`.
pub fn connect(id: &str, model: &str) -> Result<()> {
    let agent = find(id)?;
    let model = model.trim();
    if model.is_empty() {
        bail!("no model given");
    }
    let mut state = State::load()?;
    let saved = state.clone();
    wire(agent, model, Others::saved(&saved, id), &mut state)?;
    state.save()
}

/// Sets the agent's quick-task model; `None` follows the main model.
pub fn set_small_model(id: &str, model: Option<&str>) -> Result<()> {
    let agent = find(id)?;
    let mut state = State::load()?;
    let current = view(agent, &state)
        .model
        .ok_or_else(|| anyhow!("{} is not connected to pansy", agent.name))?;
    let saved = state.clone();
    let others = Others {
        small: model,
        ..Others::saved(&saved, id)
    };
    wire(agent, &current, others, &mut state)?;
    state.save()
}

/// Sets the model the agent's subagents start on when they name none;
/// `None` leaves that to the agent, its key as it was before pansy.
pub fn set_subagent_model(id: &str, model: Option<&str>) -> Result<()> {
    let agent = find(id)?;
    if !agent.subagent_model {
        bail!(
            "{} has no setting for its subagents' model that pansy knows",
            agent.name
        );
    }
    let model = model.map(str::trim).filter(|m| !m.is_empty());
    let mut state = State::load()?;
    let current = view(agent, &state)
        .model
        .ok_or_else(|| anyhow!("{} is not connected to pansy", agent.name))?;
    let saved = state.clone();
    let others = Others {
        subagent: model,
        ..Others::saved(&saved, id)
    };
    wire(agent, &current, others, &mut state)?;
    state.save()
}

/// Gives the agent's field `key` a value (a tier's model, an effort);
/// `None` takes it back: its keys go back to what they were, or to what
/// they are without it.
pub fn set_field(id: &str, key: &str, value: Option<&str>) -> Result<()> {
    let agent = find(id)?;
    let field = agent
        .fields()
        .iter()
        .find(|f| f.key == key)
        .ok_or_else(|| anyhow!("{} has no setting {key} that pansy knows", agent.name))?;
    let value = match value.map(str::trim).filter(|v| !v.is_empty()) {
        Some(value) => Some(field.accept(value).map_err(|why| anyhow!(why))?),
        None => None,
    };
    let mut state = State::load()?;
    let current = view(agent, &state)
        .model
        .ok_or_else(|| anyhow!("{} is not connected to pansy", agent.name))?;
    let saved = state.clone();
    let mut others = Others::saved(&saved, id);
    match value {
        Some(value) => others.fields.insert(key.to_owned(), value),
        None => others.fields.remove(key),
    };
    wire(agent, &current, others, &mut state)?;
    state.save()
}

/// Whether what holds the key at `path` (the table it is in, the list it
/// is an item of) is in `text`.
pub(crate) fn holds(format: files::Format, text: &str, path: &[&str]) -> Result<bool> {
    match path.split_last() {
        Some((_, [])) | None => Ok(!text.trim().is_empty()),
        Some((_, parents)) => Ok(files::get(format, text, parents)?.is_some()),
    }
}

/// Connects the agent again, when it is connected: what it lists changed.
fn rewire(agent: &Agent) -> Result<()> {
    match view(agent, &State::load()?).model {
        Some(model) => connect(agent.id, &model),
        None => Ok(()),
    }
}

/// Picks which of the gateway's models the agent lists; a connected one's
/// list is written again. The models it is set to stay in it.
pub fn set_listed(id: &str, models: pansy_core::settings::AgentModels) -> Result<()> {
    let agent = find(id)?;
    if !agent.lists_models() {
        bail!(
            "{} lists no models of pansy's: it starts on the one picked for it",
            agent.name
        );
    }
    config::update_settings(|s| {
        s.agent_models.insert(agent.id.to_owned(), models);
        s.check().map_err(|e| anyhow!(e))
    })?;
    rewire(agent)
}

/// Whether the agent lists a model with its provider's name before it;
/// `None` follows Settings' Provider in model names again.
pub fn set_names(id: &str, with_provider: Option<bool>) -> Result<()> {
    let agent = find(id)?;
    if !agent.lists_models() {
        bail!("{} shows no names of pansy's for its models", agent.name);
    }
    config::update_settings(|s| {
        match with_provider {
            Some(on) => s.agent_names.insert(agent.id.to_owned(), on),
            None => s.agent_names.remove(agent.id),
        };
        Ok(())
    })?;
    rewire(agent)
}

/// Puts back every key pansy set in the agent's config.
pub fn disconnect(id: &str) -> Result<()> {
    let agent = find(id)?;
    let mut state = State::load()?;
    let record = state.agent(agent.id);
    let touched = std::mem::take(&mut record.touched);
    let saved = std::mem::take(&mut record.files);
    let formats = formats(agent);
    let mut by_file: BTreeMap<String, Vec<Touched>> = BTreeMap::new();
    for t in touched {
        by_file.entry(t.file.clone()).or_default().push(t);
    }
    for (file, keys) in by_file {
        let path = PathBuf::from(&file);
        let Some(format) = formats.get(&file).copied() else {
            continue;
        };
        let mut text = files::read_text(&path)?;
        let original = text.clone();
        // As pansy left it: the file goes back whole, as it was.
        if let Some(saved) = saved.get(&file)
            && path.exists()
            && saved.wrote == state::digest(&text)
        {
            match &saved.before {
                Some(before) if *before != text => write_atomic(&path, before.as_bytes(), false)?,
                Some(_) => {}
                None => fs::remove_file(&path)
                    .with_context(|| format!("removing {}", path.display()))?,
            }
            continue;
        }
        for t in keys.iter().rev() {
            let key: Vec<&str> = t.path.iter().map(String::as_str).collect();
            text = files::set(format, &text, &key, t.before.as_ref())?;
        }
        if text != original {
            write_atomic(&path, text.as_bytes(), false)?;
        }
    }
    record.model = None;
    record.paused = None;
    state.save()
}

/// Puts back the agent's config as it was before pansy, for now: it works
/// without the gateway, and [`resume`] connects it again with the model it
/// was on.
pub fn pause(id: &str) -> Result<()> {
    let agent = find(id)?;
    let model = view(agent, &State::load()?)
        .model
        .ok_or_else(|| anyhow!("{} is not connected to pansy", agent.name))?;
    disconnect(id)?;
    let mut state = State::load()?;
    state.agent(agent.id).paused = Some(model);
    state.save()
}

/// Connects an agent [`pause`] put back again, with the model it was on.
pub fn resume(id: &str) -> Result<()> {
    let agent = find(id)?;
    let model = State::load()?
        .agents
        .get(agent.id)
        .and_then(|s| s.paused.clone())
        .ok_or_else(|| anyhow!("{} was not put back for now", agent.name))?;
    connect(agent.id, &model)
}

/// [`pause`] for every connected agent: how many were put back. A failure
/// leaves the others put back, and names each agent that failed.
pub fn pause_all() -> Result<usize> {
    let ids: Vec<&str> = list()
        .into_iter()
        .filter(|v| v.connected)
        .map(|v| v.id)
        .collect();
    each(&ids, pause)
}

/// [`resume`] for every agent put back for now: how many were connected.
pub fn resume_all() -> Result<usize> {
    let ids: Vec<&str> = list()
        .into_iter()
        .filter(|v| v.paused.is_some())
        .map(|v| v.id)
        .collect();
    each(&ids, resume)
}

fn each(ids: &[&str], f: fn(&str) -> Result<()>) -> Result<usize> {
    let problems: Vec<String> = ids
        .iter()
        .filter_map(|id| f(id).err().map(|e| format!("{id}: {e:#}")))
        .collect();
    if !problems.is_empty() {
        bail!("{}", problems.join("; "));
    }
    Ok(ids.len())
}

/// Connects every connected agent again: after the port changed, or a
/// provider's models (OpenCode and Crush list them).
pub fn reconnect_all() -> Result<Vec<String>> {
    let mut problems = Vec::new();
    let state = State::load()?;
    for (id, saved) in &state.agents {
        let Some(model) = saved.model.clone() else {
            continue;
        };
        if saved.touched.is_empty() {
            continue;
        }
        if let Err(e) = connect(id, &model) {
            problems.push(format!("{id}: {e:#}"));
        }
    }
    Ok(problems)
}

#[cfg(test)]
pub(crate) mod tests;
