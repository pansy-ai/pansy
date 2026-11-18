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
            read_key(&file, format, &path).is_some_and(|url| is_ours(&url, &base))
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
        cli_missing: agent.has_settings() && !agent.on_path(),
        install: agent.install(),
        picks_in: agent.picks_in(),
        connected,
        model,
        small_model: saved.and_then(|s| s.small_model.clone()),
        has_small_model: agent.small_model,
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

/// The agents whose versions are being asked, and what each said.
struct Asking {
    running: BTreeSet<&'static str>,
    said: BTreeMap<&'static str, Option<String>>,
}

static ASKING: Mutex<Asking> = Mutex::new(Asking {
    running: BTreeSet::new(),
    said: BTreeMap::new(),
});
static ANSWERED: Condvar = Condvar::new();

/// The version of each agent's program on `PATH`, as far as it is known
/// within `wait`, and whether some are still being asked: those go on, and
/// are ready the next time. One asking per agent at a time.
pub fn versions(wait: Duration) -> (BTreeMap<&'static str, String>, bool) {
    let here: Vec<&'static Agent> = agent::all().into_iter().filter(|a| a.on_path()).collect();
    let Ok(mut asking) = ASKING.lock() else {
        return (BTreeMap::new(), false);
    };
    for &a in &here {
        if asking.running.insert(a.id) {
            std::thread::spawn(move || {
                let said = cli::version_of(a.programs);
                if let Ok(mut asking) = ASKING.lock() {
                    asking.running.remove(a.id);
                    asking.said.insert(a.id, said);
                }
                ANSWERED.notify_all();
            });
        }
    }
    let running = |asking: &mut Asking| here.iter().any(|a| asking.running.contains(a.id));
    let Ok((asking, _)) = ANSWERED.wait_timeout_while(asking, wait, running) else {
        return (BTreeMap::new(), true);
    };
    let known = here
        .iter()
        .filter_map(|a| Some((a.id, asking.said.get(a.id)?.clone()?)))
        .collect();
    (known, here.iter().any(|a| asking.running.contains(a.id)))
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
    out.extend(
        agent
            .edits(&agent::Wiring {
                base_url: "",
                key: "",
                model: "",
                small_model: None,
                choices: &[],
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

/// Sets the agent's keys for `model` (and its quick-task model), noting
/// what each replaced.
fn wire(agent: &Agent, model: &str, small: Option<&str>, state: &mut State) -> Result<()> {
    let base = gateway_url();
    let choices: Vec<String> = pansy_core::manage::choices()
        .unwrap_or_default()
        .into_iter()
        .map(|c| c.id)
        .collect();
    let wiring = agent::Wiring {
        base_url: &base,
        key: AGENT_KEY,
        model,
        small_model: small,
        choices: &choices,
    };
    let edits = agent.edits_checked(&wiring)?;
    let mut by_file: BTreeMap<PathBuf, Vec<&agent::Edit>> = BTreeMap::new();
    for e in &edits {
        by_file.entry(e.file.clone()).or_default().push(e);
    }
    let record = state.agent(agent.id);
    for (file, edits) in by_file {
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
        for e in edits {
            let path: Vec<&str> = e.path.iter().map(String::as_str).collect();
            if !record
                .touched
                .iter()
                .any(|t| t.file == file_key && t.path == e.path)
            {
                let before = files::get(e.format, &text, &path)
                    .with_context(|| format!("reading {}", file.display()))?;
                record.touched.push(Touched {
                    file: file_key.clone(),
                    path: e.path.clone(),
                    before,
                });
            }
            text = files::set(e.format, &text, &path, Some(&e.value))
                .with_context(|| format!("editing {}", file.display()))?;
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
    let small = state.agents.get(id).and_then(|s| s.small_model.clone());
    wire(agent, model, small.as_deref(), &mut state)?;
    state.save()
}

/// Sets the agent's quick-task model; `None` follows the main model.
pub fn set_small_model(id: &str, model: Option<&str>) -> Result<()> {
    let agent = find(id)?;
    let mut state = State::load()?;
    let current = view(agent, &state)
        .model
        .ok_or_else(|| anyhow!("{} is not connected to pansy", agent.name))?;
    wire(agent, &current, model, &mut state)?;
    state.save()
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
