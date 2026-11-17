//! What a plugin is, from its files: as magpie reads them, a package may be
//! several things at once.
//!
//! - an OpenCode provider plugin: a package with a server entry (its
//!   `exports["./server"]`, `main`, `exports["."]` or an `index.*`), or a
//!   file of its own;
//! - a pi package: a `pi` manifest, the `pi-package` keyword, or pi's coding
//!   agent among what it needs;
//! - gateway middleware: `magpie.middleware` (or `magpie.gateway`), or a
//!   `*.middleware.js` / `*.gateway.js` file;
//! - request hooks: `magpie.hooks`, or a `*.hooks.{js,mjs,ts,mts}` file;
//! - an agent: `magpie.agent`, or a `*.agent.js` file.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

const INDEX_FILES: [&str; 5] = [
    "index.ts",
    "index.tsx",
    "index.js",
    "index.mjs",
    "index.cjs",
];
const PI_AGENT: &str = "@earendil-works/pi-coding-agent";
const PI_AGENT_OLD: &str = "@mariozechner/pi-coding-agent";

/// What a plugin's files say it is.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Kinds {
    /// Its package's name (a file's path for a file).
    pub name: String,
    pub version: String,
    pub description: String,
    /// An OpenCode plugin: host A loads its server entry.
    pub provider: bool,
    /// A pi package: host A loads it with pi's own loader.
    pub pi: bool,
    pub middleware: Option<PathBuf>,
    pub hooks: Option<PathBuf>,
    pub agent: Option<PathBuf>,
    /// Written for OpenCode's next plugin API (`@opencode/plugin`'s
    /// `Plugin.define`), which magpie's host doesn't run.
    pub v2: bool,
    /// package.json's `magpie.options`: an example of its options.
    pub options: Option<Value>,
    pub icon: Option<String>,
}

impl Kinds {
    /// It is something pansy runs.
    pub fn any(&self) -> bool {
        self.provider
            || self.pi
            || self.middleware.is_some()
            || self.hooks.is_some()
            || self.agent.is_some()
    }

    /// Host A loads it: a provider plugin, a pi package or request hooks.
    pub fn in_host(&self) -> bool {
        self.provider || self.pi || self.hooks.is_some()
    }

    /// Its kinds as words: `provider`, `pi`, `middleware`, `hooks`, `agent`.
    pub fn words(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.provider {
            out.push("provider");
        }
        if self.pi {
            out.push("pi");
        }
        if self.middleware.is_some() {
            out.push("middleware");
        }
        if self.hooks.is_some() {
            out.push("hooks");
        }
        if self.agent.is_some() {
            out.push("agent");
        }
        out
    }
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// What the plugin at `target` (its folder, or a file) is.
pub fn of(target: &Path) -> Kinds {
    if target.is_file() {
        return of_file(target);
    }
    let pkg = read_json(&target.join("package.json")).unwrap_or(Value::Null);
    let text = |v: &Value| v.as_str().map(str::to_owned).unwrap_or_default();
    let mut k = Kinds {
        name: text(&pkg["name"]),
        version: text(&pkg["version"]),
        description: text(&pkg["description"]),
        ..Kinds::default()
    };
    if k.name.is_empty() {
        k.name = target.display().to_string();
    }
    let magpie = &pkg["magpie"];
    let file = |v: &Value| {
        v.as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| target.join(s))
    };
    k.middleware = file(&magpie["middleware"]).or_else(|| file(&magpie["gateway"]));
    k.hooks = file(&magpie["hooks"]);
    k.agent = file(&magpie["agent"]);
    k.options = magpie.get("options").cloned();
    k.icon = magpie["icon"].as_str().map(str::to_owned);
    let deps = |key: &str| {
        pkg[key]
            .as_object()
            .map(|m| m.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default()
    };
    let needs: Vec<String> = [deps("dependencies"), deps("peerDependencies")].concat();
    k.pi = pkg["pi"].is_object()
        || pkg["keywords"]
            .as_array()
            .is_some_and(|w| w.iter().any(|w| w == "pi-package"))
        || needs.iter().any(|d| d == PI_AGENT || d == PI_AGENT_OLD)
        || (pkg.is_null()
            && target.join("extensions").is_dir()
            && !target.join("index.ts").exists()
            && !target.join("index.js").exists());
    let has_entry = pkg["main"].is_string()
        || pkg["exports"].is_string()
        || pkg["exports"].is_object()
        || INDEX_FILES.iter().any(|f| target.join(f).exists());
    k.provider = has_entry && !k.pi;
    // A package of middleware, hooks or an agent alone has no entry of
    // OpenCode's.
    k.v2 = needs.iter().any(|d| d == "@opencode/plugin")
        && !needs.iter().any(|d| d == "@opencode-ai/plugin");
    k
}

fn of_file(file: &Path) -> Kinds {
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let mut k = Kinds {
        name: file.display().to_string(),
        ..Kinds::default()
    };
    let ends = |suffixes: &[&str]| suffixes.iter().any(|s| name.ends_with(s));
    if ends(&[
        ".middleware.js",
        ".middleware.mjs",
        ".gateway.js",
        ".gateway.mjs",
    ]) {
        k.middleware = Some(file.to_path_buf());
    } else if ends(&[".hooks.js", ".hooks.mjs", ".hooks.ts", ".hooks.mts"]) {
        k.hooks = Some(file.to_path_buf());
    } else if ends(&[".agent.js", ".agent.mjs"]) {
        k.agent = Some(file.to_path_buf());
    } else {
        let src = std::fs::read_to_string(file).unwrap_or_default();
        k.pi = [PI_AGENT, PI_AGENT_OLD]
            .iter()
            .any(|p| src.contains(&format!("\"{p}\"")) || src.contains(&format!("'{p}'")));
        k.provider = !k.pi;
    }
    k
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(dir: &Path, json: &str, files: &[&str]) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("package.json"), json).unwrap();
        for f in files {
            std::fs::write(dir.join(f), "export {}\n").unwrap();
        }
    }

    #[test]
    fn reads_what_a_package_is() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path();

        let p = dir.join("auth");
        package(
            &p,
            r#"{"name":"opencode-acme-auth","main":"dist/index.js","peerDependencies":{"@opencode-ai/plugin":"*"}}"#,
            &[],
        );
        let k = of(&p);
        assert!(k.provider && !k.pi && !k.v2);
        assert_eq!(k.words(), ["provider"]);

        let p = dir.join("guard");
        package(
            &p,
            r#"{"name":"word-guard","magpie":{"middleware":"./word-guard.middleware.js","options":{"words":[]}}}"#,
            &["word-guard.middleware.js"],
        );
        let k = of(&p);
        assert!(!k.provider);
        assert_eq!(k.middleware, Some(p.join("./word-guard.middleware.js")));
        assert_eq!(k.options, Some(serde_json::json!({"words": []})));

        let p = dir.join("pi");
        package(
            &p,
            r#"{"name":"pi-x","pi":{"extensions":["./src/index.ts"]}}"#,
            &["index.ts"],
        );
        let k = of(&p);
        assert!(k.pi && !k.provider);

        let p = dir.join("v2");
        package(
            &p,
            r#"{"name":"gitlab","main":"dist/index.js","peerDependencies":{"@opencode/plugin":"*"}}"#,
            &[],
        );
        assert!(of(&p).v2);

        let p = dir.join("both");
        package(
            &p,
            r#"{"name":"x","magpie":{"hooks":"./x.hooks.mjs","agent":"./x.agent.js"}}"#,
            &[],
        );
        assert_eq!(of(&p).words(), ["hooks", "agent"]);
    }

    #[test]
    fn reads_what_a_file_is() {
        let root = tempfile::tempdir().unwrap();
        for (name, kind) in [
            ("a.middleware.js", "middleware"),
            ("a.gateway.mjs", "middleware"),
            ("a.hooks.ts", "hooks"),
            ("a.agent.js", "agent"),
            ("plain.js", "provider"),
        ] {
            let f = root.path().join(name);
            std::fs::write(&f, "export {}\n").unwrap();
            assert_eq!(of(&f).words(), [kind], "{name}");
        }
    }
}
