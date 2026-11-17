//! `plugins.json`: the plugins added, in their order, each on or off with
//! its options, in magpie's shape, so magpie's file reads here as it is.
//! npm packages and git repositories are installed with Bun under
//! `plugins/` (`bun add --ignore-scripts`: no install script runs); a path
//! to a folder or a file is loaded from where it is.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use pansy_core::fsutil::write_json;
use pansy_core::paths;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::kind::{self, Kinds};

/// One plugin added.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// How it was added: an npm package (`name`, `name@version`), a git
    /// repository (`github:owner/repo`, `git+https://…`) or a path.
    pub spec: String,
    /// Switched off by the user.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub off: bool,
    /// Handed to the plugin: OpenCode's `["name", {options}]`, a
    /// middleware's `ctx.options`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<serde_json::Map<String, Value>>,
    /// The package's name when the spec doesn't say it (a git repository).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// `plugins.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct List {
    #[serde(default)]
    pub plugins: Vec<Entry>,
    /// OpenCode's config the plugins are handed (`provider.google.options`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub config: BTreeMap<String, Value>,
    /// The plugin that serves a provider id more than one signs in to:
    /// provider id → that plugin's spec.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub prefer: BTreeMap<String, String>,
}

impl List {
    /// `plugins.json`; an error, never an empty list, when it can't be read.
    pub fn load() -> Result<List> {
        pansy_core::fsutil::read_json(&paths::plugins())
    }

    fn save(&self) -> Result<()> {
        write_json(&paths::plugins(), self, true)
    }

    /// The entry `word` names: its spec, its package's name, or its short
    /// name (`workbuddy` for `@magpie-community/opencode-workbuddy-auth`).
    pub fn find(&self, word: &str) -> Option<usize> {
        let word = word.trim();
        let lower = word.to_ascii_lowercase();
        let by = |f: &dyn Fn(&Entry) -> String| self.plugins.iter().position(|e| f(e) == lower);
        by(&|e| e.spec.to_ascii_lowercase())
            .or_else(|| by(&|e| e.package().to_ascii_lowercase()))
            .or_else(|| by(&|e| short_name(&e.package())))
            // A path by its file's or folder's name: `guard` for
            // `~/dev/guard.middleware.js`.
            .or_else(|| {
                by(&|e| {
                    if !is_path(&e.spec) {
                        return String::new();
                    }
                    let last = e.spec.trim_end_matches(['/', '\\']);
                    let last = last.rsplit(['/', '\\']).next().unwrap_or(last);
                    last.split('.').next().unwrap_or(last).to_ascii_lowercase()
                })
            })
    }
}

/// Changes `plugins.json` under its lock: read, change, write.
pub fn update<R>(change: impl FnOnce(&mut List) -> Result<R>) -> Result<R> {
    locked(|| {
        let mut list = List::load()?;
        let out = change(&mut list)?;
        list.save()?;
        Ok(out)
    })
}

fn locked<R>(f: impl FnOnce() -> Result<R>) -> Result<R> {
    let file = paths::plugins();
    let dir = file.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(".plugins.json.lock"))?;
    lock.lock()?;
    let out = f();
    let _ = lock.unlock();
    out
}

/// `spec` names a file or folder rather than a package.
pub fn is_path(spec: &str) -> bool {
    let s = spec.trim();
    s.starts_with('/')
        || s.starts_with("./")
        || s.starts_with("../")
        || s.starts_with('~')
        || s.starts_with("file:")
        || (s.len() > 2 && s.as_bytes()[1] == b':' && s.as_bytes()[0].is_ascii_alphabetic())
        || s.starts_with('\\')
}

/// `spec` names a git repository rather than an npm package.
pub fn is_git(spec: &str) -> bool {
    let s = spec.trim();
    !is_path(s)
        && (s.starts_with("github:")
            || s.starts_with("gitlab:")
            || s.starts_with("bitbucket:")
            || s.starts_with("git+")
            || s.starts_with("git://")
            || s.ends_with(".git")
            || s.starts_with("https://github.com/")
            || s.starts_with("https://gitlab.com/"))
}

/// The package an npm spec names, without its version: `@scope/x` of
/// `@scope/x@1.2`.
pub fn package_of(spec: &str) -> String {
    let s = spec.trim();
    let s = s.strip_prefix("npm:").unwrap_or(s);
    match s.strip_prefix('@') {
        Some(rest) => match rest.split_once('@') {
            Some((name, _)) => format!("@{name}"),
            None => s.to_owned(),
        },
        None => s.split('@').next().unwrap_or(s).to_owned(),
    }
}

/// The name people call a plugin by: its package's without the scope and
/// the words every community plugin's has (`opencode-`, `-auth`,
/// `middleware-`, `agent-`, …).
pub fn short_name(package: &str) -> String {
    let mut s = package
        .rsplit('/')
        .next()
        .unwrap_or(package)
        .to_ascii_lowercase();
    for prefix in [
        "opencode-",
        "middleware-",
        "gateway-",
        "hooks-",
        "agent-",
        "tools-",
        "page-",
        "magpie-",
    ] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.to_owned();
            break;
        }
    }
    if let Some(rest) = s.strip_suffix("-auth") {
        s = rest.to_owned();
    }
    s
}

fn expand(path: &str) -> PathBuf {
    let path = path.strip_prefix("file:").unwrap_or(path);
    match path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        Some(rest) => paths::user_home().join(rest),
        None if path == "~" => paths::user_home(),
        None => PathBuf::from(path),
    }
}

impl Entry {
    /// Its package's name (a path for a path).
    pub fn package(&self) -> String {
        if let Some(name) = &self.name {
            return name.clone();
        }
        if is_path(&self.spec) || is_git(&self.spec) {
            return self.spec.clone();
        }
        package_of(&self.spec)
    }

    /// Where it is loaded from.
    pub fn target(&self) -> PathBuf {
        if is_path(&self.spec) {
            return expand(&self.spec);
        }
        let name = self
            .name
            .clone()
            .or_else(|| (!is_git(&self.spec)).then(|| package_of(&self.spec)))
            .or_else(|| installed_name(&self.spec))
            .unwrap_or_else(|| self.spec.clone());
        paths::plugins_dir().join("node_modules").join(name)
    }

    /// What its files say it is.
    pub fn kinds(&self) -> Kinds {
        kind::of(&self.target())
    }

    /// Its options as an object (`{}` when none).
    pub fn options_value(&self) -> Value {
        Value::Object(self.options.clone().unwrap_or_default())
    }
}

/// The package `plugins/package.json` installed from `spec` (a git spec,
/// whose name only the install tells).
fn installed_name(spec: &str) -> Option<String> {
    let pkg: Value =
        serde_json::from_slice(&std::fs::read(paths::plugins_dir().join("package.json")).ok()?)
            .ok()?;
    let deps = pkg["dependencies"].as_object()?;
    deps.iter()
        .find(|(_, v)| {
            v.as_str()
                .is_some_and(|v| v == spec || spec.ends_with(v) || v.ends_with(spec))
        })
        .map(|(k, _)| k.clone())
}

fn dependencies() -> BTreeMap<String, String> {
    std::fs::read(paths::plugins_dir().join("package.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v["dependencies"].as_object().cloned())
        .map(|m| {
            m.into_iter()
                .map(|(k, v)| (k, v.as_str().unwrap_or_default().to_owned()))
                .collect()
        })
        .unwrap_or_default()
}

/// One plugin as the list has it and its files say.
#[derive(Debug, Clone)]
pub struct Installed {
    pub entry: Entry,
    pub target: PathBuf,
    pub kinds: Kinds,
}

/// Every plugin added, in order, with what each is.
pub fn installed() -> Result<Vec<Installed>> {
    Ok(List::load()?
        .plugins
        .into_iter()
        .map(|entry| {
            let target = entry.target();
            let kinds = kind::of(&target);
            Installed {
                entry,
                target,
                kinds,
            }
        })
        .collect())
}

/// When `plugins.json` last changed: the hosts start again when it does.
pub fn stamp() -> Option<std::time::SystemTime> {
    std::fs::metadata(paths::plugins())
        .and_then(|m| m.modified())
        .ok()
}

// --- Bun's package manager ----------------------------------------------------------

/// Runs `bun <args>` in `plugins/`: its output, or why it failed. Bun's
/// download cache is pansy's too.
pub async fn run_bun(args: &[&str]) -> Result<String> {
    let bun = crate::bun::ensure().await?;
    let dir = paths::plugins_dir();
    crate::private_dir(&dir)?;
    let manifest = dir.join("package.json");
    if !manifest.exists() {
        std::fs::write(
            &manifest,
            "{\n  \"name\": \"pansy-plugins\",\n  \"private\": true\n}\n",
        )?;
    }
    let mut command = tokio::process::Command::new(&bun);
    command.args(args).current_dir(&dir).env(
        "BUN_INSTALL_CACHE_DIR",
        paths::home().join("cache").join("bun-install"),
    );
    // npmmirror with the 「国内镜像」 switch on, unless the user named a
    // registry of their own
    if let Some(registry) = crate::mirror::bun_registry() {
        command.env("BUN_CONFIG_REGISTRY", registry);
    }
    let out = command
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .with_context(|| format!("running {}", bun.display()))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if !out.status.success() {
        bail!("bun {}: {}", args.join(" "), text.trim());
    }
    Ok(text)
}

/// `package.json` and `bun.lock` as they are, to put back.
fn snapshot() -> Vec<(PathBuf, Option<Vec<u8>>)> {
    ["package.json", "bun.lock", "bun.lockb"]
        .iter()
        .map(|f| {
            let p = paths::plugins_dir().join(f);
            let b = std::fs::read(&p).ok();
            (p, b)
        })
        .collect()
}

fn restore(snap: &[(PathBuf, Option<Vec<u8>>)]) {
    for (p, b) in snap {
        match b {
            Some(b) => {
                let _ = std::fs::write(p, b);
            }
            None => {
                let _ = std::fs::remove_file(p);
            }
        }
    }
}

const PI_PACKAGES: [&str; 2] = ["@earendil-works/pi-coding-agent", "@earendil-works/pi-ai"];

/// Installs pi beside the plugins, the first time a pi package needs it.
async fn ensure_pi() -> Result<()> {
    let deps = dependencies();
    let missing: Vec<&str> = PI_PACKAGES
        .iter()
        .copied()
        .filter(|p| !deps.contains_key(*p))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    tracing::info!("installing pi for a pi package");
    let mut args = vec!["add", "--ignore-scripts"];
    args.extend(missing);
    run_bun(&args).await.map(|_| ())
}

/// Adds the plugin `spec` names: a community plugin by its name, an npm
/// package, a git repository or a path. It is installed (but a path), and
/// kept in the list after the others, in place of one of the same package.
pub async fn add(spec: &str) -> Result<Installed> {
    let spec = crate::market::resolve(spec.trim()).await;
    if spec.is_empty() {
        bail!("no plugin named");
    }
    let mut entry = Entry {
        spec: spec.clone(),
        ..Entry::default()
    };
    let snap = snapshot();
    if is_path(&spec) {
        let target = entry.target();
        if !target.exists() {
            bail!("there is no {}", target.display());
        }
        entry.spec = target.display().to_string();
    } else {
        let before = dependencies();
        run_bun(&["add", "--ignore-scripts", &spec]).await?;
        let after = dependencies();
        let name = after
            .keys()
            .find(|k| !before.contains_key(*k))
            .cloned()
            .or_else(|| (!is_git(&spec)).then(|| package_of(&spec)))
            .or_else(|| installed_name(&spec));
        if is_git(&spec) {
            entry.name = name;
        }
    }
    let target = entry.target();
    let kinds = kind::of(&target);
    let refuse = |why: String| {
        restore(&snap);
        Err(anyhow!(why))
    };
    if kinds.v2 && !kinds.pi {
        return refuse(format!(
            "{} is an OpenCode V2 plugin (@opencode/plugin), which pansy doesn't run yet: add a version written for OpenCode's first plugin API",
            kinds.name
        ));
    }
    if !kinds.any() {
        return refuse(format!(
            "{} is neither an OpenCode or pi provider plugin nor magpie middleware, request hooks or an agent",
            kinds.name
        ));
    }
    if kinds.pi
        && let Err(e) = ensure_pi().await
    {
        return refuse(format!("installing pi for {}: {e:#}", kinds.name));
    }
    let package = entry.package();
    update(|list| {
        if let Some(at) = list.plugins.iter().position(|e| e.package() == package) {
            let old = list.plugins.remove(at);
            entry.options = old.options;
            entry.off = false;
        }
        list.plugins.push(entry.clone());
        Ok(())
    })?;
    Ok(Installed {
        entry,
        target,
        kinds,
    })
}

/// Removes the plugin `word` names, and uninstalls its package.
pub async fn remove(word: &str) -> Result<Entry> {
    let entry = update(|list| {
        let at = list
            .find(word)
            .ok_or_else(|| anyhow!("no plugin {word} was added"))?;
        Ok(list.plugins.remove(at))
    })?;
    if !is_path(&entry.spec) {
        let name = entry.package();
        if dependencies().contains_key(&name)
            && let Err(e) = run_bun(&["remove", &name]).await
        {
            tracing::warn!("uninstalling {name}: {e:#}");
        }
    }
    Ok(entry)
}

/// Switches the plugin `word` names on or off.
pub fn set_off(word: &str, off: bool) -> Result<Entry> {
    update(|list| {
        let at = list
            .find(word)
            .ok_or_else(|| anyhow!("no plugin {word} was added"))?;
        list.plugins[at].off = off;
        Ok(list.plugins[at].clone())
    })
}

/// Sets the plugin's options (an object; `{}` for none).
pub fn set_options(word: &str, options: Value) -> Result<Entry> {
    let Value::Object(options) = options else {
        bail!("a plugin's options are a JSON object");
    };
    update(|list| {
        let at = list
            .find(word)
            .ok_or_else(|| anyhow!("no plugin {word} was added"))?;
        list.plugins[at].options = (!options.is_empty()).then_some(options);
        Ok(list.plugins[at].clone())
    })
}

/// Has the plugin `word` names serve `provider`, where another signs in
/// to it too.
pub fn use_for(word: &str, provider: &str) -> Result<()> {
    update(|list| {
        let at = list
            .find(word)
            .ok_or_else(|| anyhow!("no plugin {word} was added"))?;
        let spec = list.plugins[at].spec.clone();
        list.prefer.insert(provider.to_owned(), spec);
        Ok(())
    })
}

/// Updates the plugin `word` names, or every installed one: an npm package
/// without a version to its latest, a git repository to its head.
pub async fn update_plugins(word: Option<&str>) -> Result<Vec<String>> {
    let list = List::load()?;
    let chosen: Vec<Entry> = match word {
        Some(w) => vec![
            list.plugins[list
                .find(w)
                .ok_or_else(|| anyhow!("no plugin {w} was added"))?]
            .clone(),
        ],
        None => list.plugins.clone(),
    };
    let mut done = Vec::new();
    for e in chosen.iter().filter(|e| !is_path(&e.spec)) {
        let spec = if is_git(&e.spec) || e.spec.trim_start_matches('@').contains('@') {
            e.spec.clone()
        } else {
            format!("{}@latest", e.package())
        };
        run_bun(&["add", "--ignore-scripts", &spec]).await?;
        done.push(e.package());
    }
    if !done.is_empty() {
        // The hosts load what is installed now.
        update(|_| Ok(()))?;
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_specs() {
        assert_eq!(package_of("@scope/x@1.2.3"), "@scope/x");
        assert_eq!(package_of("x@latest"), "x");
        assert_eq!(package_of("x"), "x");
        assert!(is_path("~/dev/mine") && is_path("./x") && is_path("C:\\x"));
        assert!(is_git("github:owner/repo") && is_git("git+https://example.com/x.git"));
        assert!(!is_git("@scope/x"));
        assert_eq!(
            short_name("@magpie-community/opencode-workbuddy-auth"),
            "workbuddy"
        );
        assert_eq!(
            short_name("@magpie-community/middleware-word-guard"),
            "word-guard"
        );
        assert_eq!(short_name("opencode-gemini-auth"), "gemini");
    }

    #[test]
    fn finds_an_entry_by_any_of_its_names() {
        let list = List {
            plugins: vec![
                Entry {
                    spec: "@magpie-community/opencode-workbuddy-auth@0.2".into(),
                    ..Entry::default()
                },
                Entry {
                    spec: "/home/me/think-tags.middleware.js".into(),
                    ..Entry::default()
                },
            ],
            ..List::default()
        };
        assert_eq!(list.find("workbuddy"), Some(0));
        assert_eq!(
            list.find("@magpie-community/opencode-workbuddy-auth"),
            Some(0)
        );
        assert_eq!(list.find("/home/me/think-tags.middleware.js"), Some(1));
        assert_eq!(list.find("think-tags"), Some(1));
        assert_eq!(list.find("nothing"), None);
    }

    #[test]
    fn keeps_magpies_shape() {
        let text = r#"{"plugins":[{"spec":"a","off":true,"options":{"k":1}},{"spec":"b"}],"prefer":{"google":"a"}}"#;
        let list: List = serde_json::from_str(text).unwrap();
        assert!(list.plugins[0].off);
        assert_eq!(list.prefer["google"], "a");
        let back = serde_json::to_value(&list).unwrap();
        assert_eq!(back["plugins"][1], serde_json::json!({"spec": "b"}));
    }
}
