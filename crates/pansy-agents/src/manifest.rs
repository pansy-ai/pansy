//! An agent as a file: how it is found, where it keeps its config, the keys
//! that point it at the gateway and how one run of it is pointed elsewhere.
//!
//! ```toml
//! id = "codex"
//! name = "Codex"
//! protocol = "responses"            # what it speaks to the gateway
//! programs = ["codex"]              # on PATH: it is installed
//! dirs = [".codex"]                 # under the home directory: it was
//!
//! [files]                           # the main one first
//! config = "{env:CODEX_HOME|~/.codex}/config.toml"
//!
//! [[set]]                           # one key to set, in order
//! file = "config"
//! key = ["model"]
//! value = "{model}"
//!
//! [base_url]                        # where its base URL is read back
//! file = "config"
//! key = ["model_providers", "pansy", "base_url"]
//!
//! [model]                           # and its model
//! file = "config"
//! key = ["model"]
//!
//! [run]                             # `pansy run`: one run, no file written
//! args = ["-c", "model={model:q}"]
//! ```
//!
//! A value is text with `{base_url}`, `{v1}` (the base URL with `/v1`),
//! `{key}`, `{model}` and `{small}` filled in (`{model:q}`: in quotes, as
//! JSON and TOML write a string), or a table or list of such. A table whose
//! one key is `"$each_model"` becomes one entry for each model the gateway
//! offers, by its id (`{id}`); alone in a list, one item for each.
//!
//! A file's format is its ending's: `.toml`, `.yaml`, `.env`, else JSON
//! (with comments). A path's `~/` is the home directory, and
//! `{env:NAME|else}` a variable's value or, without one, what follows the
//! bar; several paths are the first that is there, else the last.

use std::path::PathBuf;
use std::sync::LazyLock;

use anyhow::{Context, Result, anyhow, bail};
use pansy_core::paths::{self, user_home};
use pansy_core::provider::Protocol;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::agent::Wiring;

/// A config file's path as a manifest writes it: the first that is there,
/// else the last.
#[derive(Debug, Clone, PartialEq)]
pub struct FilePath(Vec<String>);

impl FilePath {
    pub fn resolve(&self) -> PathBuf {
        let mut last = PathBuf::new();
        for candidate in &self.0 {
            last = expand(candidate);
            if last.exists() {
                break;
            }
        }
        last
    }
}

/// `~/` is the home directory; `{env:NAME|else}` the variable's value, or
/// what follows the bar when it has none.
fn expand(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("{env:")
        && let Some((inside, after)) = rest.split_once('}')
    {
        let (name, fallback) = inside.split_once('|').unwrap_or((inside, ""));
        let base = std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| expand(fallback));
        return join(base, after.trim_start_matches('/'));
    }
    match path.strip_prefix("~/") {
        Some(rest) => join(user_home(), rest),
        None if path == "~" => user_home(),
        None => PathBuf::from(path),
    }
}

fn join(mut base: PathBuf, rel: &str) -> PathBuf {
    for part in rel.split('/').filter(|p| !p.is_empty()) {
        base.push(part);
    }
    base
}

/// One key to set: in which of the manifest's files, and its value before
/// it is filled in.
#[derive(Debug, Clone, PartialEq)]
pub struct Set {
    pub file: usize,
    pub key: Vec<&'static str>,
    pub value: Value,
}

/// Where a value is read back.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyAt {
    pub file: usize,
    pub key: Vec<&'static str>,
    /// What the agent puts before a model's id there (`pansy/`).
    pub prefix: &'static str,
}

/// One way to install the agent, and the systems it is for (all, when none).
#[derive(Debug, Clone, PartialEq)]
pub struct InstallOn {
    pub via: &'static str,
    pub command: &'static str,
    pub os: Vec<String>,
}

impl InstallOn {
    /// Whether it is for this system.
    pub fn here(&self) -> bool {
        let here = if cfg!(windows) {
            "windows"
        } else if cfg!(target_os = "macos") {
            "macos"
        } else {
            "linux"
        };
        self.os.is_empty() || self.os.iter().any(|os| os == here)
    }
}

/// How one run is pointed at the gateway.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunSpec {
    pub verified: Option<&'static str>,
    pub unsupported: Option<&'static str>,
    pub note: Option<&'static str>,
    pub args: Vec<&'static str>,
    pub env: Vec<(&'static str, &'static str)>,
    /// Variables whose value is JSON.
    pub json_env: Vec<(&'static str, Value)>,
    /// A flag and the JSON that follows it.
    pub json_arg: Vec<(&'static str, Value)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub protocol: Protocol,
    pub programs: Vec<&'static str>,
    pub dirs: Vec<&'static str>,
    pub small_model: bool,
    pub note: Option<&'static str>,
    pub picks_in: Option<&'static str>,
    pub files: Vec<(String, FilePath)>,
    pub set: Vec<Set>,
    pub base_url: KeyAt,
    pub model: KeyAt,
    pub install: Vec<InstallOn>,
    pub run: Option<RunSpec>,
}

impl Manifest {
    pub fn file(&self, index: usize) -> PathBuf {
        self.files[index].1.resolve()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    id: String,
    name: String,
    protocol: String,
    #[serde(default)]
    programs: Vec<String>,
    #[serde(default)]
    dirs: Vec<String>,
    #[serde(default)]
    small_model: bool,
    note: Option<String>,
    picks_in: Option<String>,
    files: Map<String, Value>,
    #[serde(default)]
    set: Vec<RawSet>,
    base_url: RawKey,
    model: RawKey,
    #[serde(default)]
    install: Vec<RawInstall>,
    run: Option<RawRun>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSet {
    file: String,
    key: Vec<String>,
    value: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawKey {
    file: String,
    key: Vec<String>,
    #[serde(default)]
    prefix: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawInstall {
    via: String,
    command: String,
    #[serde(default)]
    os: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRun {
    verified: Option<String>,
    unsupported: Option<String>,
    note: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: Map<String, Value>,
    #[serde(default)]
    json_env: Map<String, Value>,
    #[serde(default)]
    json_arg: Vec<RawJsonArg>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawJsonArg {
    flag: String,
    value: Value,
}

fn leak(s: &str) -> &'static str {
    Box::leak(s.to_owned().into_boxed_str())
}

fn leak_all(items: &[String]) -> Vec<&'static str> {
    items.iter().map(|s| leak(s)).collect()
}

/// Reads a manifest. What it keeps for the program's life is kept once for
/// a text: its callers remember the manifest they made of one.
pub fn parse(text: &str) -> Result<Manifest> {
    let raw: Raw = toml_edit::de::from_str(text).map_err(|e| anyhow!("{}", e.message()))?;
    let id_ok = !raw.id.is_empty()
        && raw
            .id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !id_ok {
        bail!("an agent's id is small letters, digits and dashes");
    }
    if raw.name.trim().is_empty() {
        bail!("the agent has no name");
    }
    let protocol = Protocol::parse(&raw.protocol).ok_or_else(|| {
        anyhow!(
            "no protocol {}: anthropic, chat, responses or gemini",
            raw.protocol
        )
    })?;
    let mut files = Vec::new();
    for (name, value) in &raw.files {
        let candidates: Vec<String> = match value {
            Value::String(path) => vec![path.clone()],
            Value::Array(paths) => paths
                .iter()
                .filter_map(|p| p.as_str().map(str::to_owned))
                .collect(),
            _ => Vec::new(),
        };
        if candidates.is_empty() || candidates.iter().any(|c| c.trim().is_empty()) {
            bail!("the file {name} has no path");
        }
        files.push((name.clone(), FilePath(candidates)));
    }
    if files.is_empty() {
        bail!("the agent has no config file");
    }
    let index = |name: &str| {
        files
            .iter()
            .position(|(n, _)| n == name)
            .ok_or_else(|| anyhow!("no file named {name} among the agent's files"))
    };
    let key_at = |raw: &RawKey, what: &str| -> Result<KeyAt> {
        if raw.key.is_empty() {
            bail!("{what} has no key");
        }
        Ok(KeyAt {
            file: index(&raw.file)?,
            key: leak_all(&raw.key),
            prefix: leak(&raw.prefix),
        })
    };
    let mut set = Vec::new();
    for s in &raw.set {
        if s.key.is_empty() {
            bail!("a key to set is empty");
        }
        set.push(Set {
            file: index(&s.file)?,
            key: leak_all(&s.key),
            value: s.value.clone(),
        });
    }
    let run = match &raw.run {
        None => None,
        Some(run) => {
            let mut env = Vec::new();
            for (name, value) in &run.env {
                let value = value
                    .as_str()
                    .with_context(|| format!("run.env.{name} is not text"))?;
                env.push((leak(name), leak(value)));
            }
            Some(RunSpec {
                verified: run.verified.as_deref().map(leak),
                unsupported: run.unsupported.as_deref().map(leak),
                note: run.note.as_deref().map(leak),
                args: leak_all(&run.args),
                env,
                json_env: run
                    .json_env
                    .iter()
                    .map(|(k, v)| (leak(k), v.clone()))
                    .collect(),
                json_arg: run
                    .json_arg
                    .iter()
                    .map(|a| (leak(&a.flag), a.value.clone()))
                    .collect(),
            })
        }
    };
    Ok(Manifest {
        base_url: key_at(&raw.base_url, "base_url")?,
        model: key_at(&raw.model, "model")?,
        id: raw.id,
        name: raw.name,
        protocol,
        programs: leak_all(&raw.programs),
        dirs: leak_all(&raw.dirs),
        small_model: raw.small_model,
        note: raw.note.as_deref().map(leak),
        picks_in: raw.picks_in.as_deref().map(leak),
        files,
        set,
        install: raw
            .install
            .iter()
            .map(|i| InstallOn {
                via: leak(&i.via),
                command: leak(&i.command),
                os: i.os.clone(),
            })
            .collect(),
        run,
    })
}

/// pansy's own agents, in the order the Agents page lists them.
pub fn builtin() -> &'static [Manifest] {
    static BUILTIN: LazyLock<Vec<Manifest>> = LazyLock::new(|| {
        [
            include_str!("../agents/claude-code.toml"),
            include_str!("../agents/codex.toml"),
            include_str!("../agents/gemini-cli.toml"),
            include_str!("../agents/opencode.toml"),
            include_str!("../agents/qwen-code.toml"),
            include_str!("../agents/crush.toml"),
        ]
        .iter()
        .map(|text| parse(text).expect("a built-in agent's manifest"))
        .collect()
    });
    &BUILTIN
}

/// The user's manifests: each `.toml` in `agents/` of pansy's folder, by
/// name, with its text.
pub fn user_files() -> Vec<(PathBuf, String)> {
    let Ok(entries) = std::fs::read_dir(paths::agent_manifests()) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|file| {
            let text = std::fs::read_to_string(&file).ok()?;
            Some((file, text))
        })
        .collect()
}

/// What a manifest's values are filled in with.
pub struct Vars<'a> {
    base_url: &'a str,
    v1: String,
    key: &'a str,
    model: &'a str,
    small: &'a str,
    /// Every model the gateway offers, then the agent's own two.
    models: Vec<&'a str>,
}

impl<'a> Vars<'a> {
    pub fn of(w: &'a Wiring<'a>) -> Vars<'a> {
        let small = w.small_model.unwrap_or(w.model);
        let mut models: Vec<&str> = Vec::new();
        for id in w.choices.iter().map(String::as_str).chain([w.model, small]) {
            if !models.contains(&id) {
                models.push(id);
            }
        }
        Vars {
            base_url: w.base_url,
            v1: format!("{}/v1", w.base_url),
            key: w.key,
            model: w.model,
            small,
            models,
        }
    }

    fn var<'s>(&'s self, name: &str, id: Option<&'s str>) -> Option<&'s str> {
        Some(match name {
            "base_url" => self.base_url,
            "v1" => &self.v1,
            "key" => self.key,
            "model" => self.model,
            "small" => self.small,
            "id" => id?,
            _ => return None,
        })
    }

    fn text_with(&self, template: &str, id: Option<&str>) -> String {
        let mut out = String::with_capacity(template.len());
        let mut rest = template;
        while let Some(open) = rest.find('{') {
            out.push_str(&rest[..open]);
            let after = &rest[open + 1..];
            let filled = after.find('}').and_then(|close| {
                let inside = &after[..close];
                let (name, quoted) = match inside.strip_suffix(":q") {
                    Some(name) => (name, true),
                    None => (inside, false),
                };
                let value = self.var(name, id)?;
                Some((
                    if quoted {
                        Value::String(value.to_owned()).to_string()
                    } else {
                        value.to_owned()
                    },
                    close,
                ))
            });
            match filled {
                Some((value, close)) => {
                    out.push_str(&value);
                    rest = &after[close + 1..];
                }
                None => {
                    out.push('{');
                    rest = after;
                }
            }
        }
        out.push_str(rest);
        out
    }

    /// A text with its variables filled in; a brace that names none stays.
    pub fn text(&self, template: &str) -> String {
        self.text_with(template, None)
    }

    fn fill_with(&self, value: &Value, id: Option<&str>) -> Value {
        match value {
            Value::String(s) => Value::String(self.text_with(s, id)),
            Value::Array(items) => {
                if let [Value::Object(only)] = items.as_slice()
                    && let Some(each) = each_model(only)
                {
                    return Value::Array(
                        self.models
                            .iter()
                            .map(|m| self.fill_with(each, Some(m)))
                            .collect(),
                    );
                }
                Value::Array(items.iter().map(|v| self.fill_with(v, id)).collect())
            }
            Value::Object(map) => {
                if let Some(each) = each_model(map) {
                    return Value::Object(
                        self.models
                            .iter()
                            .map(|m| ((*m).to_owned(), self.fill_with(each, Some(m))))
                            .collect(),
                    );
                }
                Value::Object(
                    map.iter()
                        .map(|(k, v)| (k.clone(), self.fill_with(v, id)))
                        .collect(),
                )
            }
            other => other.clone(),
        }
    }

    /// A value with its variables filled in and its `$each_model` spread.
    pub fn fill(&self, value: &Value) -> Value {
        self.fill_with(value, None)
    }
}

/// The value under `$each_model`, when that is the table's one key.
fn each_model(map: &Map<String, Value>) -> Option<&Value> {
    if map.len() == 1 {
        map.get("$each_model")
    } else {
        None
    }
}

/// Lays `over` on `base`: a table's keys one by one, anything else whole.
pub fn lay_over(base: &mut Value, over: &Value) {
    match (base, over) {
        (Value::Object(base), Value::Object(over)) => {
            for (k, v) in over {
                match base.get_mut(k) {
                    Some(mine) => lay_over(mine, v),
                    None => {
                        base.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (base, over) => *base = over.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn wiring<'a>(choices: &'a [String]) -> Wiring<'a> {
        Wiring {
            base_url: "http://127.0.0.1:7267",
            key: "pansy",
            model: "a/x",
            small_model: Some("b/y"),
            choices,
        }
    }

    #[test]
    fn fills_text_and_spreads_the_models() {
        let choices = vec!["a/x".to_owned(), "c/z".to_owned()];
        let w = wiring(&choices);
        let vars = Vars::of(&w);
        assert_eq!(
            vars.text("{v1} {model} {small}"),
            "http://127.0.0.1:7267/v1 a/x b/y"
        );
        assert_eq!(vars.text("model={model:q}"), "model=\"a/x\"");
        assert_eq!(vars.text("{\"kept\": {nothing}}"), "{\"kept\": {nothing}}");
        let filled = vars.fill(&json!({
            "map": {"$each_model": {"name": "{id}"}},
            "list": [{"$each_model": {"id": "{id}", "n": 1}}],
        }));
        assert_eq!(
            filled,
            json!({
                "map": {"a/x": {"name": "a/x"}, "c/z": {"name": "c/z"}, "b/y": {"name": "b/y"}},
                "list": [{"id": "a/x", "n": 1}, {"id": "c/z", "n": 1}, {"id": "b/y", "n": 1}],
            })
        );
    }

    #[test]
    fn reads_the_agents_built_in() {
        let ids: Vec<&str> = builtin().iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "claude-code",
                "codex",
                "gemini-cli",
                "opencode",
                "qwen-code",
                "crush"
            ]
        );
    }

    #[test]
    fn says_what_is_wrong_with_a_manifest() {
        let good = "id = \"x\"\nname = \"X\"\nprotocol = \"chat\"\n[files]\nc = \"~/.x/c.json\"\n[base_url]\nfile = \"c\"\nkey = [\"url\"]\n[model]\nfile = \"c\"\nkey = [\"model\"]\n";
        assert_eq!(parse(good).unwrap().id, "x");
        let no_file = good.replace(
            "file = \"c\"\nkey = [\"url\"]",
            "file = \"d\"\nkey = [\"url\"]",
        );
        assert!(format!("{:#}", parse(&no_file).unwrap_err()).contains("no file named d"));
        let protocol = good.replace("\"chat\"", "\"smoke\"");
        assert!(format!("{:#}", parse(&protocol).unwrap_err()).contains("no protocol smoke"));
        let unknown = format!("{good}\n[extra]\na = 1\n");
        assert!(parse(&unknown).is_err());
    }

    #[test]
    fn lays_one_table_over_another() {
        let mut base = json!({"a": {"b": 1, "c": 2}, "d": [1]});
        lay_over(&mut base, &json!({"a": {"c": 3}, "d": [2], "e": true}));
        assert_eq!(base, json!({"a": {"b": 1, "c": 3}, "d": [2], "e": true}));
    }
}
