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
//! offers, by its id (`{id}`) and the name it is listed by (`{name}`: its
//! own, after its provider's unless Provider in model names is off); alone
//! in a list, one item for each. The user picks which ones an agent lists
//! (Agents › Model list › Pick): the rest are left out, but those the agent
//! is set to.
//!
//! `{subagent}` is the model the agent's subagents start on, for an agent
//! that has a key for it (`subagent_model = true`). The user may give none:
//! a text that names it is then left out — the key to set, the entry of a
//! table, the item of a list, the variable of a run — and a key pansy set
//! for one before goes back to what it was. A run's arguments for it are
//! `subagent_args`, added when there is one.
//!
//! An agent whose lead names its subagents' models itself, over its config,
//! sends the model in the header `x-pansy-subagent-model` (Codex's
//! `http_headers`): the gateway puts a subagent's request on it whatever it
//! asked for. `subagent_every = true` says so, for the window's words.
//!
//! A `[[field]]` is one more thing the user may set for the agent, by its
//! `key` (`pansy claude-code opus <provider/model>`): a model of the
//! gateway's (`kind = "model"`) or how hard a model thinks (`kind =
//! "effort"`, one of its `levels`). A value names it as `{opus}`. With none
//! given, a text that names it is left out as one naming `{subagent}` is;
//! `follows = true` fills in the main model instead (Claude Code's tiers),
//! and `joins = ":"` nothing at all, the value being written after what it
//! goes with (`"{opus}{opus_effort}"` is `a/x:high`: the gateway reads the
//! effort off the model's name).
//!
//! Three more kinds are settings of the agent's own, each a row of its
//! sheet on the Agents page: `tokens` (a size, `272k` or `272000`; `min`
//! and `max` bound it), `percent` (1 to 100) and `switch` (`on`, or none).
//! `{compact_at:n}` alone in a text writes the number, not its text. A
//! `[[set]]` with `when = "<field>"` is left out, as one naming a field
//! with no value is, until that field has one: what a switch turns on.
//!
//! `[sign_in]` says where the agent keeps its own sign-in to its vendor
//! (`file`, a path of its own) and the `key` that is there while it is
//! signed in: pansy looks whether it is there, never at what it holds, to
//! say what an agent not connected is on now.
//!
//! A file's format is its ending's: `.toml`, `.yaml`, `.env`, else JSON
//! (with comments). A path's `~/` is the home directory, and
//! `{env:NAME|else}` a variable's value or, without one, what follows the
//! bar; `{config}` is where the system keeps a desktop app's settings
//! (`~/.config`, `~/Library/Application Support`, `%APPDATA%`) and
//! `{local}` the same but `%LOCALAPPDATA%` on Windows. Several paths are the
//! first that is there, else the last. `dirs` are under the home directory,
//! or such a path.
//!
//! `lists_models = true` says the agent lists the gateway's `/v1/models`
//! itself rather than models written into its files: the user picks which
//! it lists all the same.

use std::path::PathBuf;
use std::sync::LazyLock;

use anyhow::{Context, Result, anyhow, bail};
use pansy_core::paths::{self, user_home};
use pansy_core::provider::Protocol;
use serde::{Deserialize, Serialize};
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
/// what follows the bar when it has none; `{config}` and `{local}` the
/// folders this system keeps a desktop app's settings in.
pub(crate) fn expand(path: &str) -> PathBuf {
    for (name, local) in [("{config}", false), ("{local}", true)] {
        if let Some(rest) = path.strip_prefix(name) {
            return join(app_folder(local), rest.trim_start_matches('/'));
        }
    }
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

/// Where a desktop app keeps its settings: `$XDG_CONFIG_HOME` (`~/.config`)
/// on Linux, `~/Library/Application Support` on macOS, `%APPDATA%` on
/// Windows, or `%LOCALAPPDATA%` there for what stays on this computer
/// (`local`).
fn app_folder(local: bool) -> PathBuf {
    if cfg!(target_os = "macos") {
        expand("~/Library/Application Support")
    } else if cfg!(windows) {
        if local {
            expand("{env:LOCALAPPDATA|~/AppData/Local}")
        } else {
            expand("{env:APPDATA|~/AppData/Roaming}")
        }
    } else {
        expand("{env:XDG_CONFIG_HOME|~/.config}")
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
    /// The field that must have a value for the key to be set.
    pub when: Option<&'static str>,
    /// One key for each model the agent lists: `{id}` and `{name}` in the
    /// key and the value are that model's.
    pub each_model: bool,
    /// The key is taken out while the agent is connected, not set.
    pub remove: bool,
    /// Set only where what holds the key (the list, for an item of one) is
    /// in the file already.
    pub if_there: bool,
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
    /// The arguments after those, when there is a model for subagents.
    pub subagent_args: Vec<&'static str>,
    pub env: Vec<(&'static str, &'static str)>,
    /// Variables whose value is JSON.
    pub json_env: Vec<(&'static str, Value)>,
    /// A flag and the JSON that follows it.
    pub json_arg: Vec<(&'static str, Value)>,
}

/// What a field holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldKind {
    /// One of the gateway's models.
    Model,
    /// How hard a model thinks: one of the field's levels.
    Effort,
    /// A size in tokens.
    Tokens,
    /// A share, 1 to 100.
    Percent,
    /// On, or not given.
    Switch,
}

impl FieldKind {
    /// Whether it is a setting of the agent's own, a row of its sheet.
    pub fn is_setting(self) -> bool {
        matches!(
            self,
            FieldKind::Tokens | FieldKind::Percent | FieldKind::Switch
        )
    }
}

/// A size as the user writes it: `272000`, `272k`, `1m`, `1.5M`.
pub fn parse_tokens(text: &str) -> Option<u64> {
    let t = text
        .trim()
        .replace([',', '_', ' '], "")
        .to_ascii_lowercase();
    let (number, scale) = match t.as_bytes().last()? {
        b'k' => (&t[..t.len() - 1], 1_000.0),
        b'm' => (&t[..t.len() - 1], 1_000_000.0),
        _ => (t.as_str(), 1.0),
    };
    let n: f64 = number.parse().ok()?;
    (n.is_finite() && n >= 0.0 && (scale > 1.0 || n.fract() == 0.0))
        .then(|| (n * scale).round() as u64)
}

/// One more thing the user may set for the agent.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub key: &'static str,
    pub kind: FieldKind,
    /// An effort's levels, the lowest first.
    pub levels: Vec<&'static str>,
    /// With none given, the main model's.
    pub follows: bool,
    /// What is written before its value, which goes after another's; with
    /// none given, nothing is.
    pub joins: &'static str,
    /// The least and the most a size may be.
    pub min: Option<u64>,
    pub max: Option<u64>,
}

impl Field {
    /// What its name is filled in with: `None`, a text naming it is left out.
    fn filled(&self, given: Option<&str>, model: &str) -> Option<String> {
        match given {
            Some(value) => Some(format!("{}{value}", self.joins)),
            None if self.follows => Some(model.to_owned()),
            None if !self.joins.is_empty() => Some(String::new()),
            None => None,
        }
    }

    /// `value` as it is kept (a size as its digits), or why it can't be
    /// given to the field.
    pub fn accept(&self, value: &str) -> Result<String, String> {
        match self.kind {
            FieldKind::Model => Ok(value.to_owned()),
            FieldKind::Effort if self.levels.contains(&value) => Ok(value.to_owned()),
            FieldKind::Effort => Err(format!(
                "{} is one of {}, not {value}",
                self.key,
                self.levels.join(", ")
            )),
            FieldKind::Tokens => {
                let n = parse_tokens(value).ok_or_else(|| {
                    format!("{} is a number of tokens like 272k, not {value}", self.key)
                })?;
                if self.min.is_some_and(|min| n < min) || self.max.is_some_and(|max| n > max) {
                    return Err(match (self.min, self.max) {
                        (Some(min), Some(max)) => {
                            format!("{} is {min} to {max} tokens, not {n}", self.key)
                        }
                        (Some(min), None) => {
                            format!("{} is {min} tokens or more, not {n}", self.key)
                        }
                        (_, Some(max)) => format!("{} is {max} tokens at most, not {n}", self.key),
                        (None, None) => unreachable!("a bound was passed"),
                    });
                }
                Ok(n.to_string())
            }
            FieldKind::Percent => value
                .trim()
                .trim_end_matches('%')
                .trim()
                .parse::<u64>()
                .ok()
                .filter(|n| (1..=100).contains(n))
                .map(|n| n.to_string())
                .ok_or_else(|| format!("{} is a percent from 1 to 100, not {value}", self.key)),
            FieldKind::Switch if value == "on" => Ok(value.to_owned()),
            FieldKind::Switch => Err(format!("{} is on or off, not {value}", self.key)),
        }
    }

    /// Why `value` can't be given to it, if it can't.
    pub fn refuses(&self, value: &str) -> Option<String> {
        self.accept(value).err()
    }
}

/// The names a value's text may have in braces beside a field's.
const VARS: [&str; 8] = [
    "base_url", "v1", "key", "model", "small", "subagent", "id", "name",
];

#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub protocol: Protocol,
    pub programs: Vec<&'static str>,
    pub dirs: Vec<&'static str>,
    pub small_model: bool,
    /// It has a key for the model its subagents start on.
    pub subagent_model: bool,
    /// The gateway puts every subagent of it on that model, whatever one
    /// its lead names.
    pub subagent_every: bool,
    /// It lists the gateway's `/v1/models` itself.
    pub lists_models: bool,
    pub note: Option<&'static str>,
    pub picks_in: Option<&'static str>,
    /// Its npm package: where the version that is out is asked.
    pub npm: Option<&'static str>,
    /// The arguments of its own updater (`claude update`), for an install
    /// its own installer made.
    pub self_update: Vec<&'static str>,
    pub fields: Vec<Field>,
    pub files: Vec<(String, FilePath)>,
    pub set: Vec<Set>,
    pub base_url: KeyAt,
    pub model: KeyAt,
    pub install: Vec<InstallOn>,
    pub run: Option<RunSpec>,
    /// Where its own sign-in to its vendor is kept: the file, and the key
    /// that is there while it is signed in.
    pub sign_in: Option<(FilePath, Vec<&'static str>)>,
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
    #[serde(default)]
    subagent_model: bool,
    #[serde(default)]
    subagent_every: bool,
    #[serde(default)]
    lists_models: bool,
    note: Option<String>,
    picks_in: Option<String>,
    npm: Option<String>,
    #[serde(default)]
    self_update: Vec<String>,
    #[serde(default)]
    field: Vec<RawField>,
    files: Map<String, Value>,
    #[serde(default)]
    set: Vec<RawSet>,
    base_url: Option<RawKey>,
    model: Option<RawKey>,
    #[serde(default)]
    install: Vec<RawInstall>,
    run: Option<RawRun>,
    sign_in: Option<RawSignIn>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSignIn {
    file: String,
    key: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawField {
    key: String,
    kind: String,
    #[serde(default)]
    levels: Vec<String>,
    #[serde(default)]
    follows: bool,
    #[serde(default)]
    joins: String,
    min: Option<u64>,
    max: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSet {
    file: String,
    key: Vec<String>,
    value: Option<Value>,
    when: Option<String>,
    #[serde(default)]
    each_model: bool,
    #[serde(default)]
    remove: bool,
    #[serde(default)]
    if_there: bool,
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
    subagent_args: Vec<String>,
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
    let key_at = |raw: &Option<RawKey>, what: &str| -> Result<KeyAt> {
        // Without one, pansy goes by what it wrote: the agent keeps no such
        // key where it can be read back.
        let Some(raw) = raw else {
            return Ok(KeyAt {
                file: 0,
                key: Vec::new(),
                prefix: "",
            });
        };
        if raw.key.is_empty() {
            bail!("{what} has no key");
        }
        Ok(KeyAt {
            file: index(&raw.file)?,
            key: leak_all(&raw.key),
            prefix: leak(&raw.prefix),
        })
    };
    let mut fields: Vec<Field> = Vec::new();
    for f in &raw.field {
        let key_ok = !f.key.is_empty()
            && f.key
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !key_ok {
            bail!("a field's key is small letters, digits and underscores");
        }
        if VARS.contains(&f.key.as_str()) || fields.iter().any(|x| x.key == f.key) {
            bail!("the field {} has a name already taken", f.key);
        }
        let kind = match f.kind.as_str() {
            "model" => FieldKind::Model,
            "effort" => FieldKind::Effort,
            "tokens" => FieldKind::Tokens,
            "percent" => FieldKind::Percent,
            "switch" => FieldKind::Switch,
            other => bail!("no kind of field {other}: model, effort, tokens, percent or switch"),
        };
        if (kind == FieldKind::Effort) == f.levels.is_empty() {
            bail!(
                "the field {}: an effort has levels, and only an effort",
                f.key
            );
        }
        if kind.is_setting() && !f.joins.is_empty() {
            bail!("the field {}: joins is for a model or an effort", f.key);
        }
        if (f.min.is_some() || f.max.is_some()) && kind != FieldKind::Tokens {
            bail!("the field {}: min and max are for tokens", f.key);
        }
        if f.follows && (kind != FieldKind::Model || !f.joins.is_empty()) {
            bail!(
                "the field {}: follows = true is for a model, written by itself",
                f.key
            );
        }
        fields.push(Field {
            key: leak(&f.key),
            kind,
            levels: leak_all(&f.levels),
            follows: f.follows,
            joins: leak(&f.joins),
            min: f.min,
            max: f.max,
        });
    }
    let mut set = Vec::new();
    for s in &raw.set {
        if s.key.is_empty() {
            bail!("a key to set is empty");
        }
        if let Some(when) = &s.when
            && !fields.iter().any(|f| f.key == when)
        {
            bail!("when = \"{when}\" names no field of the agent");
        }
        if s.remove == s.value.is_some() {
            bail!("the key {} has a value, or remove = true", s.key.join("."));
        }
        let per_model = |text: &str| named(text).any(|n| n == "id" || n == "name");
        if s.each_model && !s.key.iter().any(|k| per_model(k)) {
            bail!(
                "each_model = true: the key {} names no {{id}}",
                s.key.join(".")
            );
        }
        if !s.each_model && s.key.iter().any(|k| per_model(k)) {
            bail!(
                "the key {} names a model of the list: each_model = true",
                s.key.join(".")
            );
        }
        set.push(Set {
            file: index(&s.file)?,
            key: leak_all(&s.key),
            value: s.value.clone().unwrap_or(Value::Null),
            when: s.when.as_deref().map(leak),
            each_model: s.each_model,
            remove: s.remove,
            if_there: s.if_there,
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
                subagent_args: leak_all(&run.subagent_args),
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
    let names_it = set.iter().any(|s| names_subagent(&s.value))
        || run.as_ref().is_some_and(|r| {
            !r.subagent_args.is_empty()
                || r.args
                    .iter()
                    .chain(r.env.iter().map(|(_, v)| v))
                    .any(|t| subagent_in(t))
                || r.json_env
                    .iter()
                    .chain(&r.json_arg)
                    .any(|(_, v)| names_subagent(v))
        });
    if names_it && !raw.subagent_model {
        bail!(
            "{{subagent}} is named, but the agent doesn't say it has a model for subagents (subagent_model = true)"
        );
    }
    if raw.subagent_model && !set.iter().any(|s| names_subagent(&s.value)) {
        bail!("subagent_model = true, but no key to set names {{subagent}}");
    }
    if run
        .as_ref()
        .is_some_and(|r| r.args.iter().any(|a| subagent_in(a)))
    {
        bail!("a run's arguments for the subagents' model go in subagent_args");
    }
    if !raw.self_update.is_empty() && raw.npm.is_none() {
        bail!("self_update is for an agent that names its npm package (npm = \"…\")");
    }
    if raw.subagent_every && !raw.subagent_model {
        bail!(
            "subagent_every = true is for an agent with a model for subagents (subagent_model = true)"
        );
    }
    if raw
        .sign_in
        .as_ref()
        .is_some_and(|s| s.file.trim().is_empty() || s.key.is_empty())
    {
        bail!("sign_in names a file and the key there");
    }
    Ok(Manifest {
        sign_in: raw
            .sign_in
            .as_ref()
            .map(|s| (FilePath(vec![s.file.clone()]), leak_all(&s.key))),
        base_url: key_at(&raw.base_url, "base_url")?,
        model: key_at(&raw.model, "model")?,
        id: raw.id,
        name: raw.name,
        protocol,
        programs: leak_all(&raw.programs),
        dirs: leak_all(&raw.dirs),
        small_model: raw.small_model,
        subagent_model: raw.subagent_model,
        subagent_every: raw.subagent_every,
        lists_models: raw.lists_models,
        note: raw.note.as_deref().map(leak),
        picks_in: raw.picks_in.as_deref().map(leak),
        npm: raw.npm.as_deref().map(leak),
        self_update: leak_all(&raw.self_update),
        fields,
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

/// Whether a text names `{subagent}`.
fn subagent_in(text: &str) -> bool {
    text.contains("{subagent}") || text.contains("{subagent:q}")
}

/// Whether a value names `{subagent}`, at any depth.
fn names_subagent(value: &Value) -> bool {
    match value {
        Value::String(s) => subagent_in(s),
        Value::Array(items) => items.iter().any(names_subagent),
        Value::Object(map) => map.values().any(names_subagent),
        _ => false,
    }
}

/// pansy's own agents, in the order the Agents page lists them: magpie's.
pub fn builtin() -> &'static [Manifest] {
    static BUILTIN: LazyLock<Vec<Manifest>> = LazyLock::new(|| {
        [
            include_str!("../agents/claude-code.toml"),
            include_str!("../agents/codex.toml"),
            include_str!("../agents/gemini-cli.toml"),
            include_str!("../agents/opencode.toml"),
            include_str!("../agents/qwen-code.toml"),
            include_str!("../agents/crush.toml"),
            include_str!("../agents/claude-desktop.toml"),
            include_str!("../agents/antigravity.toml"),
            include_str!("../agents/mimocode.toml"),
            include_str!("../agents/deveco.toml"),
            include_str!("../agents/pi.toml"),
            include_str!("../agents/stepcode.toml"),
            include_str!("../agents/mothx.toml"),
            include_str!("../agents/penguin.toml"),
            include_str!("../agents/omo.toml"),
            include_str!("../agents/prime-agent.toml"),
            include_str!("../agents/mu.toml"),
            include_str!("../agents/goose.toml"),
            include_str!("../agents/vscode.toml"),
            include_str!("../agents/vscode-insiders.toml"),
            include_str!("../agents/copilot-jetbrains.toml"),
            include_str!("../agents/copilot-cli.toml"),
            include_str!("../agents/reasonix.toml"),
            include_str!("../agents/commandcode.toml"),
            include_str!("../agents/fx.toml"),
            include_str!("../agents/hermes.toml"),
            include_str!("../agents/morph.toml"),
            include_str!("../agents/kimi-cli.toml"),
            include_str!("../agents/muse.toml"),
            include_str!("../agents/empryo.toml"),
            include_str!("../agents/nanobot.toml"),
            include_str!("../agents/miniq.toml"),
            include_str!("../agents/paimon.toml"),
            include_str!("../agents/ante.toml"),
            include_str!("../agents/qwenpaw.toml"),
            include_str!("../agents/openclaw.toml"),
            include_str!("../agents/jcode.toml"),
            include_str!("../agents/vibe.toml"),
            include_str!("../agents/minimax-code.toml"),
            include_str!("../agents/droid.toml"),
            include_str!("../agents/qoder.toml"),
            include_str!("../agents/qoder-cn.toml"),
            include_str!("../agents/grok.toml"),
            include_str!("../agents/grok-app.toml"),
            include_str!("../agents/zcode.toml"),
            include_str!("../agents/workbuddy.toml"),
            include_str!("../agents/workbuddy-ai.toml"),
            include_str!("../agents/codebuddy.toml"),
            include_str!("../agents/pencil.toml"),
            include_str!("../agents/openpencil.toml"),
            include_str!("../agents/tuanjie-cowork.toml"),
            include_str!("../agents/liveagent.toml"),
            include_str!("../agents/t3code.toml"),
            include_str!("../agents/atomcode.toml"),
            include_str!("../agents/snow.toml"),
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
    subagent: Option<&'a str>,
    /// The manifest's fields, each with what its name is filled in with.
    fields: Vec<(&'static str, Option<String>)>,
    /// Every model the gateway offers, then the agent's own.
    models: Vec<&'a str>,
    /// The name each is listed by, by its id.
    names: &'a std::collections::BTreeMap<String, String>,
}

/// The names in braces in `text` (`{model}`, `{model:q}`).
fn named(text: &str) -> impl Iterator<Item = &str> {
    text.split('{').skip(1).filter_map(|after| {
        let inside = &after[..after.find('}')?];
        Some(
            inside
                .strip_suffix(":q")
                .or_else(|| inside.strip_suffix(":n"))
                .unwrap_or(inside),
        )
    })
}

impl<'a> Vars<'a> {
    /// `w` for an agent with these fields.
    pub fn of(w: &'a Wiring<'a>, fields: &[Field]) -> Vars<'a> {
        let small = w.small_model.unwrap_or(w.model);
        let mut models: Vec<&str> = Vec::new();
        let given = |f: &Field| w.fields.get(f.key).map(String::as_str);
        let picked = fields
            .iter()
            .filter(|f| f.kind == FieldKind::Model)
            .filter_map(given);
        let own = [w.model, small]
            .into_iter()
            .chain(w.subagent_model)
            .chain(picked);
        for id in w.choices.iter().map(String::as_str).chain(own) {
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
            subagent: w.subagent_model,
            fields: fields
                .iter()
                .map(|f| (f.key, f.filled(given(f), w.model)))
                .collect(),
            models,
            names: w.names,
        }
    }

    /// Whether the key `set` is left out: its value is ([`Vars::leaves_out`]),
    /// or the field it waits for has none.
    pub fn skips(&self, set: &Set) -> bool {
        self.leaves_out(&set.value)
            || set.when.is_some_and(|when| {
                self.fields
                    .iter()
                    .any(|(key, filled)| *key == when && filled.is_none())
            })
    }

    /// Whether `value` is a text left out: it names `{subagent}` or a
    /// field, and the user gave none.
    pub fn leaves_out(&self, value: &Value) -> bool {
        value.as_str().is_some_and(|t| self.leaves_out_text(t))
    }

    /// [`Vars::leaves_out`], of a text.
    pub fn leaves_out_text(&self, text: &str) -> bool {
        named(text).any(|name| match name {
            "subagent" => self.subagent.is_none(),
            _ => self
                .fields
                .iter()
                .any(|(key, filled)| *key == name && filled.is_none()),
        })
    }

    /// The models the agent lists: the gateway's, then its own.
    pub fn models(&self) -> &[&'a str] {
        &self.models
    }

    /// A key's step with its variables filled in; `id`, the model of the
    /// list it is for.
    pub fn step(&self, step: &str, id: Option<&str>) -> String {
        self.text_with(step, id)
    }

    /// [`Vars::fill`], for one model of the list.
    pub fn fill_for(&self, value: &Value, id: &str) -> Value {
        self.fill_with(value, Some(id))
    }

    /// Whether there is a model for subagents.
    pub fn has_subagent(&self) -> bool {
        self.subagent.is_some()
    }

    fn var<'s>(&'s self, name: &str, id: Option<&'s str>) -> Option<&'s str> {
        Some(match name {
            "base_url" => self.base_url,
            "v1" => &self.v1,
            "key" => self.key,
            "model" => self.model,
            "small" => self.small,
            "subagent" => self.subagent?,
            "id" => id?,
            "name" => {
                let id = id?;
                self.names.get(id).map_or(id, String::as_str)
            }
            _ => self
                .fields
                .iter()
                .find(|(key, _)| *key == name)?
                .1
                .as_deref()?,
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
            Value::String(s) => {
                // `{name:n}` alone: the number, not its text.
                if let Some(name) = s.strip_prefix('{').and_then(|r| r.strip_suffix(":n}"))
                    && let Some(n) = self.var(name, id).and_then(|v| v.parse::<u64>().ok())
                {
                    return Value::Number(n.into());
                }
                Value::String(self.text_with(s, id))
            }
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
                Value::Array(
                    items
                        .iter()
                        .filter(|v| !self.leaves_out(v))
                        .map(|v| self.fill_with(v, id))
                        .collect(),
                )
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
                        .filter(|(_, v)| !self.leaves_out(v))
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
            subagent_model: None,
            fields: crate::agent::no_fields(),
            choices,
            names: crate::agent::no_fields(),
        }
    }

    #[test]
    fn leaves_out_what_names_a_subagent_model_there_is_none_of() {
        let value = json!({
            "env": {"MODEL": "{model}", "SUB": "{subagent}"},
            "list": ["{model}", "sub={subagent:q}"],
        });
        let mut w = wiring(&[]);
        let vars = Vars::of(&w, &[]);
        assert_eq!(
            vars.fill(&value),
            json!({"env": {"MODEL": "a/x"}, "list": ["a/x"]})
        );
        assert!(vars.leaves_out(&json!("{subagent}")));
        assert!(!vars.leaves_out(&json!("{model}")));
        w.subagent_model = Some("c/z");
        let vars = Vars::of(&w, &[]);
        assert_eq!(
            vars.fill(&value),
            json!({"env": {"MODEL": "a/x", "SUB": "c/z"}, "list": ["a/x", "sub=\"c/z\""]})
        );
        assert!(!vars.leaves_out(&json!("{subagent}")));
    }

    #[test]
    fn fills_text_and_spreads_the_models() {
        let choices = vec!["a/x".to_owned(), "c/z".to_owned()];
        let w = wiring(&choices);
        let vars = Vars::of(&w, &[]);
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
        // pansy's first six, then magpie's others; none twice
        assert_eq!(
            ids[..6],
            [
                "claude-code",
                "codex",
                "gemini-cli",
                "opencode",
                "qwen-code",
                "crush"
            ]
        );
        assert_eq!(ids.len(), 55);
        let mut once = ids.clone();
        once.sort_unstable();
        once.dedup();
        assert_eq!(once.len(), ids.len());
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
