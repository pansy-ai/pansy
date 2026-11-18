//! One run of an agent on a model, its config left alone (`pansy run`), and
//! the project file that says which model each agent uses in a folder.
//!
//! ```toml
//! # .pansy.toml, in the project's folder: model names alone, no keys
//! model = "group/daily"                 # every agent, unless it has its own
//!
//! [claude]
//! model = "deepseek/deepseek-chat"
//! small = "group/cheap"                 # its quick tasks
//! subagent = "group/cheap"              # its subagents that name no model
//! ```
//!
//! The file is found from the folder the agent starts in, upward. One that
//! came with a repository could send the user's code to a vendor they
//! never picked, so a file counts only once the user said yes to it, as it
//! is: `projects.json` keeps its fingerprint, and a changed file asks again.
//!
//! The agent is started with a key that names the folder
//! ([`pansy_core::project`]), so the gateway counts its requests under it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use pansy_core::fsutil::{read_json, write_atomic, write_json};
use pansy_core::paths;
use serde::{Deserialize, Serialize};

use crate::agent::{self, Agent, Wiring};
use crate::state::{self, State};

/// The project file's name.
pub const FILE: &str = ".pansy.toml";

/// The models a project file gives one agent.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Choice {
    pub model: Option<String>,
    pub small: Option<String>,
}

/// A project file, read.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectFile {
    pub path: PathBuf,
    /// Its fingerprint, as `projects.json` keeps it.
    pub digest: String,
    /// What it gives every agent without models of its own.
    pub all: Choice,
    /// Agent id → its models.
    pub agents: BTreeMap<String, Choice>,
    /// The user said yes to it as it is now.
    pub trusted: bool,
}

impl ProjectFile {
    /// The models it gives `agent`: its own, else every agent's.
    pub fn choice(&self, agent: &str) -> Choice {
        let own = self.agents.get(agent).cloned().unwrap_or_default();
        Choice {
            model: own.model.or_else(|| self.all.model.clone()),
            small: own.small.or_else(|| self.all.small.clone()),
        }
    }

    /// The project's folder.
    pub fn folder(&self) -> &Path {
        self.path.parent().unwrap_or(Path::new("."))
    }
}

/// The agent a name on the command line or in a project file means: its
/// id, or its program's name (`claude` is Claude Code).
pub fn by_name(name: &str) -> Option<&'static Agent> {
    let all = agent::all();
    all.iter()
        .find(|a| a.id == name)
        .or_else(|| all.iter().find(|a| a.programs.contains(&name)))
        .copied()
}

#[derive(Default, Serialize, Deserialize)]
struct Trusted {
    /// A project file's path → its fingerprint when the user said yes.
    #[serde(default)]
    trusted: BTreeMap<String, String>,
}

fn text_of(value: &toml_edit::Item, what: &str) -> Result<Option<String>> {
    match value {
        toml_edit::Item::None => Ok(None),
        toml_edit::Item::Value(toml_edit::Value::String(s)) => {
            let model = s.value().trim();
            if model.is_empty() {
                bail!("{what} is empty");
            }
            Ok(Some(model.to_owned()))
        }
        _ => bail!("{what} is not a model's name"),
    }
}

/// Reads the project file at `path`. It may name models and nothing else:
/// a key it doesn't know is an error, not passed over.
pub fn read(path: &Path) -> Result<ProjectFile> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| anyhow!("{}: {}", path.display(), e.message()))?;
    let mut file = ProjectFile {
        path: path.to_path_buf(),
        digest: state::digest(&text),
        all: Choice::default(),
        agents: BTreeMap::new(),
        trusted: false,
    };
    for (key, item) in doc.iter() {
        match key {
            "model" => file.all.model = text_of(item, "model")?,
            "small" => file.all.small = text_of(item, "small")?,
            name => {
                let agent = by_name(name).ok_or_else(|| {
                    anyhow!(
                        "{}: pansy knows no agent {name}; a project file holds `model`, `small` and a table for each agent",
                        path.display()
                    )
                })?;
                let table = item
                    .as_table_like()
                    .ok_or_else(|| anyhow!("{}: {name} is not a table", path.display()))?;
                let mut choice = Choice::default();
                for (key, item) in table.iter() {
                    match key {
                        "model" => choice.model = text_of(item, &format!("{name}.model"))?,
                        "small" => choice.small = text_of(item, &format!("{name}.small"))?,
                        other => bail!(
                            "{}: {name}.{other} is not something a project file holds (model, small)",
                            path.display()
                        ),
                    }
                }
                file.agents.insert(agent.id.to_owned(), choice);
            }
        }
    }
    let trusted: Trusted = read_json(&paths::projects()).unwrap_or_default();
    file.trusted = trusted.trusted.get(&key_of(path)) == Some(&file.digest);
    Ok(file)
}

fn key_of(path: &Path) -> String {
    path.display().to_string()
}

/// The project file for `folder`: the one there, or in the nearest folder
/// above it.
pub fn find(folder: &Path) -> Option<PathBuf> {
    folder
        .ancestors()
        .map(|dir| dir.join(FILE))
        .find(|file| file.is_file())
}

/// The project `folder` is in: where its project file is, else where its
/// repository begins, else the folder.
pub fn root(folder: &Path) -> PathBuf {
    if let Some(file) = find(folder)
        && let Some(dir) = file.parent()
    {
        return dir.to_path_buf();
    }
    folder
        .ancestors()
        .find(|dir| dir.join(".git").exists())
        .unwrap_or(folder)
        .to_path_buf()
}

/// The user says yes to the project file as it is now.
pub fn trust(file: &ProjectFile) -> Result<()> {
    let mut trusted: Trusted = read_json(&paths::projects())?;
    trusted
        .trusted
        .insert(key_of(&file.path), file.digest.clone());
    write_json(&paths::projects(), &trusted, true)
}

/// Takes the yes back; whether there was one.
pub fn forget(path: &Path) -> Result<bool> {
    let mut trusted: Trusted = read_json(&paths::projects())?;
    let was = trusted.trusted.remove(&key_of(path)).is_some();
    if was {
        write_json(&paths::projects(), &trusted, true)?;
    }
    Ok(was)
}

/// Writes a project file into `folder` with the models the connected agents
/// are on now, and says yes to it: the user's own. Refuses over one there.
pub fn init(folder: &Path) -> Result<ProjectFile> {
    let path = folder.join(FILE);
    if path.exists() {
        bail!("{} is there already", path.display());
    }
    let mut text = String::from(
        "# The model each agent uses in this folder, for `pansy run <agent>`.\n# Model names alone: no keys, so the file can go into the repository.\n",
    );
    let mut any = false;
    for view in crate::list() {
        let Some(model) = view.model.as_ref().or(view.paused.as_ref()) else {
            continue;
        };
        any = true;
        let name = agent::find(view.id)
            .and_then(|a| a.programs.first().copied())
            .unwrap_or(view.id);
        text.push_str(&format!("\n[{name}]\nmodel = {}\n", quoted(model)));
        if let Some(small) = view.small_model.as_ref().filter(|_| view.has_small_model) {
            text.push_str(&format!("small = {}\n", quoted(small)));
        }
    }
    if !any {
        text.push_str("\n# model = \"provider/model\"        every agent\n\n# [claude]\n# model = \"provider/model\"\n# small = \"provider/model\"\n");
    }
    write_atomic(&path, text.as_bytes(), false)?;
    let file = read(&path)?;
    trust(&file)?;
    read(&path)
}

fn quoted(text: &str) -> String {
    serde_json::Value::String(text.to_owned()).to_string()
}

/// Where a run's model came from.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    /// `--model`.
    Flag,
    /// The project file.
    File(PathBuf),
    /// The model the agent is connected with.
    Connected,
}

/// An agent about to start.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Launch {
    pub agent: &'static str,
    pub name: &'static str,
    pub program: PathBuf,
    /// The arguments pansy puts before the user's, then the user's.
    pub args: Vec<String>,
    /// What its environment gains.
    pub env: Vec<(String, String)>,
    pub model: String,
    pub small: Option<String>,
    pub source: Source,
    /// The folder its requests are counted under.
    pub project: PathBuf,
    /// The agent's version this way of starting it was tried on.
    pub verified: Option<&'static str>,
    pub note: Option<&'static str>,
}

/// The models asked for on the command line, each over the project file's.
#[derive(Debug, Clone, Copy, Default)]
pub struct Models<'a> {
    pub model: Option<&'a str>,
    pub small: Option<&'a str>,
}

/// Plans a run of `agent` in `folder`: the model from `asked`, else the
/// project file's (`file`, one the user said yes to), else the one the agent
/// is connected with.
pub fn launch(
    agent: &'static Agent,
    folder: &Path,
    file: Option<&ProjectFile>,
    asked: Models,
    user_args: &[String],
) -> Result<Launch> {
    let Models { model, small } = asked;

    let given = file.map(|f| f.choice(agent.id)).unwrap_or_default();
    let saved = State::load()
        .ok()
        .and_then(|s| s.agents.get(agent.id).cloned());
    let (model, source) = match (model, given.model, &saved) {
        (Some(model), _, _) => (model.trim().to_owned(), Source::Flag),
        (None, Some(model), _) => (
            model,
            Source::File(file.map(|f| f.path.clone()).unwrap_or_default()),
        ),
        (None, None, Some(saved)) if saved.model.is_some() || saved.paused.is_some() => (
            saved
                .model
                .clone()
                .or_else(|| saved.paused.clone())
                .unwrap_or_default(),
            Source::Connected,
        ),
        _ => bail!(
            "no model for {}: give one (pansy run --model <provider/model> {}), or write a {FILE} here (pansy project init)",
            agent.name,
            agent.programs.first().copied().unwrap_or(agent.id)
        ),
    };
    if model.is_empty() {
        bail!("no model given");
    }
    let small = small
        .map(|s| s.trim().to_owned())
        .or(given.small)
        .or_else(|| {
            saved
                .as_ref()
                .filter(|_| source == Source::Connected)
                .and_then(|s| s.small_model.clone())
        })
        .filter(|s| !s.is_empty() && agent.small_model);
    let program = agent
        .programs
        .iter()
        .find_map(|p| crate::cli::which(p))
        .ok_or_else(|| anyhow!("{} is not installed here", agent.name))?;
    let project = root(folder);
    let key = pansy_core::project::key_for(&project);
    let base = crate::gateway_url();
    let choices: Vec<String> = pansy_core::manage::choices()
        .unwrap_or_default()
        .into_iter()
        .map(|c| c.id)
        .collect();
    let run = agent
        .run(&Wiring {
            base_url: &base,
            key: &key,
            model: &model,
            small_model: small.as_deref(),
            choices: &choices,
        })
        .map_err(|why| anyhow!(why))?;
    let mut args = run.args;
    args.extend(user_args.iter().cloned());
    Ok(Launch {
        agent: agent.id,
        name: agent.name,
        program,
        args,
        env: run.env,
        model,
        small,
        source,
        project,
        verified: run.verified,
        note: run.note,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::tests::scratch;

    /// `PATH` as it was, put back when the test ends.
    struct Path0(Option<std::ffi::OsString>);

    impl Drop for Path0 {
        fn drop(&mut self) {
            // SAFETY: the test still holds the scratch home's lock.
            unsafe {
                match &self.0 {
                    Some(path) => std::env::set_var("PATH", path),
                    None => std::env::remove_var("PATH"),
                }
            }
        }
    }

    /// A program named `name` alone on `PATH`.
    fn program(home: &Path, name: &str) -> Path0 {
        let was = Path0(std::env::var_os("PATH"));
        let bin = home.join("bin").join(name);
        fs::create_dir_all(&bin).unwrap();
        let file = bin.join(name);
        fs::write(&file, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
        }
        // SAFETY: the test holds the scratch home's lock.
        unsafe { std::env::set_var("PATH", &bin) };
        was
    }

    #[test]
    fn reads_a_project_file_and_wants_a_yes() {
        let (home, _lock) = scratch();
        let project = home.path().join("code/shop");
        fs::create_dir_all(project.join("src/deep")).unwrap();
        fs::write(
            project.join(FILE),
            "model = \"group/daily\"\n\n[claude]\nmodel = \"deepseek/deepseek-chat\"\nsmall = \"group/cheap\"\n",
        )
        .unwrap();
        let found = find(&project.join("src/deep")).unwrap();
        assert_eq!(found, project.join(FILE));
        assert_eq!(root(&project.join("src/deep")), project);
        let file = read(&found).unwrap();
        assert!(!file.trusted);
        assert_eq!(
            file.choice("claude-code"),
            Choice {
                model: Some("deepseek/deepseek-chat".into()),
                small: Some("group/cheap".into()),
            }
        );
        assert_eq!(file.choice("codex").model.as_deref(), Some("group/daily"));

        trust(&file).unwrap();
        assert!(read(&found).unwrap().trusted);
        // Changed since: it asks again.
        fs::write(&found, "model = \"expensive/model\"\n").unwrap();
        assert!(!read(&found).unwrap().trusted);
        assert!(forget(&found).unwrap());
        assert!(!forget(&found).unwrap());
    }

    #[test]
    fn a_project_file_holds_models_alone() {
        let (home, _lock) = scratch();
        let file = home.path().join(FILE);
        for (text, says) in [
            ("[nobody]\nmodel = \"a/b\"\n", "no agent nobody"),
            (
                "[codex]\nbase_url = \"http://evil\"\n",
                "codex.base_url is not",
            ),
            ("[codex]\nmodel = 3\n", "not a model's name"),
            ("model = \"\"\n", "model is empty"),
            ("model = \n", ".pansy.toml"),
        ] {
            fs::write(&file, text).unwrap();
            let error = format!("{:#}", read(&file).unwrap_err());
            assert!(error.contains(says), "{text:?}: {error}");
        }
    }

    #[test]
    fn the_project_is_the_repository_without_a_file() {
        let (home, _lock) = scratch();
        let repo = home.path().join("code/api");
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::create_dir_all(repo.join("a/b")).unwrap();
        assert_eq!(root(&repo.join("a/b")), repo);
        let plain = home.path().join("notes");
        fs::create_dir_all(&plain).unwrap();
        assert_eq!(root(&plain), plain);
    }

    #[test]
    fn plans_a_run_of_each_agent_without_writing_its_config() {
        let (home, _lock) = scratch();
        let project = home.path().join("code/shop");
        fs::create_dir_all(&project).unwrap();
        fs::write(
            project.join(FILE),
            "model = \"group/daily\"\n[claude]\nmodel = \"deepseek/deepseek-chat\"\nsmall = \"group/cheap\"\n",
        )
        .unwrap();
        let file = read(&project.join(FILE)).unwrap();
        let key = pansy_core::project::key_for(&project);
        let env_of = |l: &Launch, name: &str| {
            l.env
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        };

        // Claude Code: `--settings`, which wins over its settings' env.
        let _path1 = program(home.path(), "claude");
        let claude = by_name("claude").unwrap();
        let l = launch(
            claude,
            &project,
            Some(&file),
            Models::default(),
            &["-p".into(), "hi".into()],
        )
        .unwrap();
        assert_eq!(l.source, Source::File(project.join(FILE)));
        assert_eq!(l.args[0], "--settings");
        let settings: serde_json::Value = serde_json::from_str(&l.args[1]).unwrap();
        assert_eq!(
            settings["env"]["ANTHROPIC_BASE_URL"],
            "http://127.0.0.1:7267"
        );
        assert_eq!(settings["env"]["ANTHROPIC_AUTH_TOKEN"], key.as_str());
        assert_eq!(settings["env"]["ANTHROPIC_MODEL"], "deepseek/deepseek-chat");
        assert_eq!(settings["env"]["ANTHROPIC_SMALL_FAST_MODEL"], "group/cheap");
        assert_eq!(l.args[2..], ["-p", "hi"]);
        assert_eq!(l.project, project);

        // Codex: `-c`, the flag's model over the file's.
        let _path2 = program(home.path(), "codex");
        let codex = by_name("codex").unwrap();
        let l = launch(
            codex,
            &project,
            Some(&file),
            Models {
                model: Some("moonshot/kimi \"k2\""),
                ..Models::default()
            },
            &[],
        )
        .unwrap();
        assert_eq!(l.source, Source::Flag);
        assert_eq!(l.args[..2], ["-c", "model=\"moonshot/kimi \\\"k2\\\"\""]);
        assert!(
            l.args.contains(
                &"model_providers.pansy.base_url=\"http://127.0.0.1:7267/v1\"".to_owned()
            )
        );
        assert!(l.args.contains(&format!(
            "model_providers.pansy.experimental_bearer_token=\"{key}\""
        )));
        assert_eq!(l.small, None, "Codex has no quick-task model");

        // Gemini CLI: the environment.
        let _path3 = program(home.path(), "gemini");
        let l = launch(
            by_name("gemini").unwrap(),
            &project,
            Some(&file),
            Models::default(),
            &[],
        )
        .unwrap();
        assert_eq!(l.model, "group/daily");
        assert_eq!(
            env_of(&l, "GOOGLE_GEMINI_BASE_URL"),
            "http://127.0.0.1:7267"
        );
        assert_eq!(env_of(&l, "GEMINI_API_KEY"), key);
        assert_eq!(env_of(&l, "GEMINI_MODEL"), "group/daily");
        assert!(l.args.is_empty());

        // OpenCode: a config of its own in a variable.
        let _path4 = program(home.path(), "opencode");
        let l = launch(
            by_name("opencode").unwrap(),
            &project,
            Some(&file),
            Models::default(),
            &[],
        )
        .unwrap();
        let content: serde_json::Value =
            serde_json::from_str(&env_of(&l, "OPENCODE_CONFIG_CONTENT")).unwrap();
        assert_eq!(content["model"], "pansy/group/daily");
        assert_eq!(
            content["provider"]["pansy"]["options"]["baseURL"],
            "http://127.0.0.1:7267/v1"
        );
        assert_eq!(
            content["provider"]["pansy"]["options"]["apiKey"],
            key.as_str()
        );
        assert!(content["provider"]["pansy"]["models"]["group/daily"].is_object());

        // Crush reads its files alone: it says so.
        let _path5 = program(home.path(), "crush");
        let error = launch(
            by_name("crush").unwrap(),
            &project,
            Some(&file),
            Models::default(),
            &[],
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("connect it instead"), "{error}");

        // Nothing of the agents' was written.
        for config in [
            ".claude",
            ".codex",
            ".gemini",
            ".config/opencode",
            ".config/crush",
        ] {
            assert!(!home.path().join(config).exists(), "{config} was made");
        }
    }

    #[test]
    fn a_run_falls_back_on_the_connected_model_or_says_how() {
        let (home, _lock) = scratch();
        let folder = home.path().join("notes");
        fs::create_dir_all(&folder).unwrap();
        let _path6 = program(home.path(), "claude");
        let claude = by_name("claude").unwrap();
        let error = launch(claude, &folder, None, Models::default(), &[])
            .unwrap_err()
            .to_string();
        assert!(error.contains("pansy project init"), "{error}");

        crate::connect("claude-code", "deepseek/deepseek-chat").unwrap();
        crate::set_small_model("claude-code", Some("deepseek/deepseek-reasoner")).unwrap();
        let l = launch(claude, &folder, None, Models::default(), &[]).unwrap();
        assert_eq!(l.source, Source::Connected);
        assert_eq!(l.model, "deepseek/deepseek-chat");
        assert_eq!(l.small.as_deref(), Some("deepseek/deepseek-reasoner"));

        // An agent not on PATH.
        let error = launch(
            by_name("codex").unwrap(),
            &folder,
            None,
            Models {
                model: Some("a/b"),
                ..Models::default()
            },
            &[],
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("not installed"), "{error}");
    }

    #[test]
    fn init_writes_the_connected_agents_models() {
        let (home, _lock) = scratch();
        let folder = home.path().join("code/shop");
        fs::create_dir_all(&folder).unwrap();
        crate::connect("claude-code", "deepseek/deepseek-chat").unwrap();
        crate::connect("codex", "group/daily").unwrap();
        let file = init(&folder).unwrap();
        assert!(file.trusted);
        assert_eq!(
            file.choice("claude-code").model.as_deref(),
            Some("deepseek/deepseek-chat")
        );
        assert_eq!(file.choice("codex").model.as_deref(), Some("group/daily"));
        assert!(init(&folder).is_err(), "not over one that is there");
    }
}
