//! `pansy run`: one agent on one model for one run, its config left alone;
//! and `pansy project`, the file that says which model a folder's agents use.

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Result, anyhow, bail};
use pansy_agents::run::{self, FILE, Launch, ProjectFile, Source};
use pansy_core::provider::mask_key;

use crate::print_json;

const USAGE: &str = "usage: pansy run [--model <provider/model>] [--small <provider/model>] [--subagent <provider/model>] [--show] [--yes] <agent> [arguments for the agent]";

/// What `pansy run` was asked.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Asked {
    pub model: Option<String>,
    pub small: Option<String>,
    /// For the agent's subagents that name no model.
    pub subagent: Option<String>,
    /// Say what would start, and start nothing.
    pub show: bool,
    /// Say yes to the project file without asking.
    pub yes: bool,
    pub json: bool,
    pub agent: String,
    /// Everything after the agent's name, the agent's own.
    pub rest: Vec<String>,
}

/// pansy's options come before the agent's name; what follows it is the
/// agent's, as it is (a `--` first is dropped).
pub(crate) fn parse(args: &[String]) -> Result<Asked> {
    let mut asked = Asked::default();
    let mut at = 0;
    while at < args.len() {
        let arg = args[at].as_str();
        let mut value = |name: &str| -> Result<String> {
            at += 1;
            args.get(at)
                .cloned()
                .ok_or_else(|| anyhow!("{name} needs a model; {USAGE}"))
        };
        match arg {
            "--model" | "-m" => asked.model = Some(value("--model")?),
            "--small" => asked.small = Some(value("--small")?),
            "--subagent" => asked.subagent = Some(value("--subagent")?),
            "--show" => asked.show = true,
            "--yes" | "-y" => asked.yes = true,
            "--json" => asked.json = true,
            _ if arg.starts_with("--model=") => asked.model = Some(arg[8..].to_owned()),
            _ if arg.starts_with("--small=") => asked.small = Some(arg[8..].to_owned()),
            _ if arg.starts_with("--subagent=") => asked.subagent = Some(arg[11..].to_owned()),
            _ if arg.starts_with('-') => bail!("pansy run has no option {arg}; {USAGE}"),
            _ => {
                asked.agent = arg.to_owned();
                let rest = &args[at + 1..];
                let rest = match rest.first().map(String::as_str) {
                    Some("--") => &rest[1..],
                    _ => rest,
                };
                asked.rest = rest.to_vec();
                return Ok(asked);
            }
        }
        at += 1;
    }
    bail!("{USAGE}")
}

/// What a project file gives the agents, a line each.
fn lines_of(file: &ProjectFile) -> Vec<String> {
    let mut out = Vec::new();
    let line = |who: &str, c: &run::Choice| {
        let mut line = format!("  {who:14}");
        if let Some(model) = &c.model {
            line.push_str(&format!(" {model}"));
        }
        if let Some(small) = &c.small {
            line.push_str(&format!("  (quick tasks: {small})"));
        }
        if let Some(subagent) = &c.subagent {
            line.push_str(&format!("  (subagents: {subagent})"));
        }
        line
    };
    if file.all != run::Choice::default() {
        out.push(line("every agent", &file.all));
    }
    for (id, choice) in &file.agents {
        let name = pansy_agents::agent::find(id).map(|a| a.name).unwrap_or(id);
        out.push(line(name, choice));
    }
    out
}

/// Asks the user whether a project file they haven't said yes to counts.
fn ask(file: &ProjectFile) -> Result<bool> {
    let mut err = std::io::stderr();
    writeln!(
        err,
        "{} sets the models for this folder, and you haven't said yes to it as it is now:",
        file.path.display()
    )?;
    for line in lines_of(file) {
        writeln!(err, "{line}")?;
    }
    write!(
        err,
        "Use it? Your code goes to these models' vendors. [y/N] "
    )?;
    err.flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes" | "Yes"))
}

/// The project file for `folder`, once the user said yes to it: asked now
/// at a terminal, taken with `--yes`, and otherwise an error that says how.
/// A file that would give this run nothing isn't asked about.
fn file_for(
    folder: &Path,
    asked: &Asked,
    agent: &pansy_agents::agent::Agent,
) -> Result<Option<ProjectFile>> {
    let Some(path) = run::find(folder) else {
        return Ok(None);
    };
    let file = run::read(&path)?;
    let gives = file.choice(agent.id);
    let wanted = (asked.model.is_none() && gives.model.is_some())
        || (asked.small.is_none() && agent.small_model && gives.small.is_some())
        || (asked.subagent.is_none() && agent.subagent_model && gives.subagent.is_some());
    if !wanted {
        return Ok(None);
    }
    if file.trusted {
        return Ok(Some(file));
    }
    if asked.yes {
        run::trust(&file)?;
        return Ok(Some(file));
    }
    if asked.show {
        bail!(
            "{} sets this folder's models and you haven't said yes to it: pansy project trust",
            path.display()
        );
    }
    if !(std::io::stdin().is_terminal() && std::io::stderr().is_terminal()) {
        bail!(
            "{} sets this folder's models and you haven't said yes to it: run `pansy project trust` here, or give --yes or --model",
            path.display()
        );
    }
    if !ask(&file)? {
        bail!("not started; give --model to pick one yourself");
    }
    run::trust(&file)?;
    Ok(Some(file))
}

fn source(l: &Launch) -> String {
    match &l.source {
        Source::Flag => "--model".to_owned(),
        Source::File(path) => path.display().to_string(),
        Source::Connected => "the model it is connected with".to_owned(),
    }
}

/// The plan as text: the project's key is no secret, but any other value
/// named like one is masked.
fn show(l: &Launch) {
    println!("{} on {} (from {})", l.name, l.model, source(l));
    if let Some(small) = &l.small {
        println!("Quick tasks  {small}");
    }
    if let Some(subagent) = &l.subagent {
        println!("Subagents    {subagent}");
    }
    println!("Project      {}", l.project.display());
    println!("Program      {}", l.program.display());
    if !l.args.is_empty() {
        println!("Arguments    {}", l.args.join(" "));
    }
    for (name, value) in &l.env {
        let secret = ["KEY", "TOKEN", "SECRET"].iter().any(|w| name.contains(w))
            && !value.starts_with(pansy_core::project::KEY_PREFIX);
        let value = if secret {
            mask_key(value)
        } else {
            value.clone()
        };
        println!("Environment  {name}={value}");
    }
    match l.verified {
        Some(version) => println!("Tried on     {} {version}", l.name),
        None => println!(
            "Tried on     no {} yet: this is how it should take them",
            l.name
        ),
    }
    if let Some(note) = l.note {
        println!("Note         {note}");
    }
}

/// Whether the gateway listens.
fn listening() -> bool {
    let settings = pansy_core::config::load_settings().unwrap_or_default();
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], settings.port));
    std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(300)).is_ok()
}

/// `pansy run …`: the agent's exit code, or pansy's own failure.
pub fn command(args: &[String]) -> ExitCode {
    match start(args) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("pansy: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn start(args: &[String]) -> Result<ExitCode> {
    let asked = parse(args)?;
    let agent = run::by_name(&asked.agent).ok_or_else(|| {
        anyhow!(
            "pansy knows no agent {}; `pansy agents` lists them",
            asked.agent
        )
    })?;
    let folder = std::env::current_dir()?;
    let file = file_for(&folder, &asked, agent)?;
    let launch = run::launch(
        agent,
        &folder,
        file.as_ref(),
        run::Models {
            model: asked.model.as_deref(),
            small: asked.small.as_deref(),
            subagent: asked.subagent.as_deref(),
        },
        &asked.rest,
    )?;
    if asked.show {
        if asked.json {
            print_json(&launch)?;
        } else {
            show(&launch);
        }
        return Ok(ExitCode::SUCCESS);
    }
    if !listening() {
        eprintln!(
            "pansy: note: nothing answers at the gateway yet: open pansy or run `pansy serve`"
        );
    }
    eprintln!(
        "pansy: {} on {} (from {})",
        launch.name,
        launch.model,
        source(&launch)
    );
    let mut command = std::process::Command::new(&launch.program);
    command.args(&launch.args).envs(launch.env.iter().cloned());
    exec(command, &launch.program)
}

/// Becomes the agent, so the terminal and its signals are the agent's own.
#[cfg(unix)]
fn exec(mut command: std::process::Command, program: &Path) -> Result<ExitCode> {
    use std::os::unix::process::CommandExt;
    let error = command.exec();
    Err(anyhow!("starting {}: {error}", program.display()))
}

/// Starts the agent and waits for it; its exit code is pansy's.
#[cfg(not(unix))]
fn exec(mut command: std::process::Command, program: &Path) -> Result<ExitCode> {
    let status = command
        .status()
        .map_err(|e| anyhow!("starting {}: {e}", program.display()))?;
    Ok(ExitCode::from(
        status.code().unwrap_or(1).clamp(0, 255) as u8
    ))
}

fn folder_of(arg: Option<&str>) -> Result<PathBuf> {
    let here = std::env::current_dir()?;
    Ok(match arg {
        Some(dir) => here.join(dir),
        None => here,
    })
}

fn found(folder: &Path) -> Result<ProjectFile> {
    let path = run::find(folder).ok_or_else(|| {
        anyhow!(
            "no {FILE} here or above {}; `pansy project init` writes one",
            folder.display()
        )
    })?;
    run::read(&path)
}

/// `pansy project …`.
pub fn project(words: &[&str], json: bool) -> Result<()> {
    match words {
        [] | ["show"] | ["show", _] => {
            let folder = folder_of(words.get(1).copied())?;
            let Some(path) = run::find(&folder) else {
                if json {
                    return print_json(
                        &serde_json::json!({"project": run::root(&folder), "file": null}),
                    );
                }
                println!("Project   {}", run::root(&folder).display());
                println!(
                    "No {FILE} here or above: `pansy run` uses --model or the model an agent is connected with; `pansy project init` writes one."
                );
                return Ok(());
            };
            let file = run::read(&path)?;
            if json {
                return print_json(&file);
            }
            println!("Project   {}", file.folder().display());
            println!(
                "File      {}{}",
                path.display(),
                if file.trusted {
                    ""
                } else {
                    "  (not used yet: you haven't said yes to it; pansy project trust)"
                }
            );
            for line in lines_of(&file) {
                println!("{line}");
            }
            Ok(())
        }
        ["init"] | ["init", _] => {
            let folder = folder_of(words.get(1).copied())?;
            let file = run::init(&folder)?;
            println!("Wrote {}:", file.path.display());
            let lines = lines_of(&file);
            if lines.is_empty() {
                println!(
                    "  no agent is connected yet: write the models into it, then `pansy project trust`"
                );
            }
            for line in lines {
                println!("{line}");
            }
            Ok(())
        }
        ["trust"] | ["trust", _] => {
            let file = found(&folder_of(words.get(1).copied())?)?;
            run::trust(&file)?;
            println!("{} counts from now on, as it is:", file.path.display());
            for line in lines_of(&file) {
                println!("{line}");
            }
            Ok(())
        }
        ["forget"] | ["forget", _] => {
            let file = found(&folder_of(words.get(1).copied())?)?;
            if run::forget(&file.path)? {
                println!("{} doesn't count any more.", file.path.display());
            } else {
                println!("{} didn't count.", file.path.display());
            }
            Ok(())
        }
        _ => bail!("usage: pansy project [show|init|trust|forget] [folder]"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn pansys_options_come_before_the_agent_and_the_rest_is_the_agents() {
        let asked = parse(&args(&[
            "--model",
            "a/b",
            "--small=c/d",
            "--subagent",
            "e/f",
            "claude",
            "--json",
            "-p",
            "hi",
            "--model",
            "opus",
        ]))
        .unwrap();
        assert_eq!(asked.model.as_deref(), Some("a/b"));
        assert_eq!(asked.small.as_deref(), Some("c/d"));
        assert_eq!(asked.subagent.as_deref(), Some("e/f"));
        assert_eq!(asked.agent, "claude");
        assert!(!asked.json, "--json after the agent is the agent's");
        assert_eq!(asked.rest, ["--json", "-p", "hi", "--model", "opus"]);

        let asked = parse(&args(&["--subagent=g/h", "codex"])).unwrap();
        assert_eq!(asked.subagent.as_deref(), Some("g/h"));
        let asked = parse(&args(&["--show", "-y", "codex", "--", "exec", "hi"])).unwrap();
        assert!(asked.show && asked.yes);
        assert_eq!(asked.rest, ["exec", "hi"]);

        assert!(parse(&args(&[])).is_err());
        assert!(parse(&args(&["--model"])).is_err());
        assert!(parse(&args(&["--fast", "claude"])).is_err());
    }
}
