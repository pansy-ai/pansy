//! `pansy drive`: an agent driven as the phone will drive it
//! ([`pansy_remote`]), from the terminal, to try a driver before the phone
//! is there. Lines typed are sent (or steer the turn that runs); an
//! approval is answered with y, s (this session), n or c (and stop).

use std::io::Write;
use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};
use pansy_remote::Mode;
use pansy_remote::codex::Codex;
use pansy_remote::{Decision, Event};
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::run::{Asked, file_for};

const USAGE: &str = "usage: pansy drive [--model <provider/model>] [--resume <thread>] [--mode ask|edits|full] [--yes] [--json] codex [folder]";

struct Options {
    asked: Asked,
    resume: Option<String>,
    mode: Mode,
    folder: PathBuf,
}

fn parse(args: &[String]) -> Result<Options> {
    let mut asked = Asked::default();
    let mut resume = None;
    let mut mode = Mode::Full;
    let mut rest = Vec::new();
    let mut words = args.iter();
    while let Some(arg) = words.next() {
        let mut value = || {
            words
                .next()
                .cloned()
                .ok_or_else(|| anyhow!("{arg} needs a value; {USAGE}"))
        };
        match arg.as_str() {
            "--model" | "-m" => asked.model = Some(value()?),
            "--resume" => resume = Some(value()?),
            "--mode" => {
                mode = match value()?.as_str() {
                    "ask" => Mode::Ask,
                    "edits" => Mode::Edits,
                    "auto" => Mode::Auto,
                    "full" => Mode::Full,
                    other => bail!("no mode {other}: ask, edits or full"),
                }
            }
            "--yes" | "-y" => asked.yes = true,
            "--json" => asked.json = true,
            a if a.starts_with('-') => bail!("pansy drive has no option {a}; {USAGE}"),
            _ => rest.push(arg.clone()),
        }
    }
    let Some(agent) = rest.first() else {
        bail!("{USAGE}")
    };
    asked.agent = agent.clone();
    let folder = match rest.get(1) {
        Some(f) => PathBuf::from(f),
        None => std::env::current_dir()?,
    };
    Ok(Options {
        asked,
        resume,
        mode,
        folder: std::fs::canonicalize(&folder).map_err(|e| anyhow!("{}: {e}", folder.display()))?,
    })
}

pub(crate) fn command(args: &[String]) -> Result<()> {
    let o = parse(args)?;
    let agent = pansy_agents::run::by_name(&o.asked.agent)
        .ok_or_else(|| anyhow!("no agent {}", o.asked.agent))?;
    if agent.id != "codex" {
        bail!("pansy drive drives Codex for now; Claude Code comes next");
    }
    let file = file_for(&o.folder, &o.asked, agent)?;
    let launch = pansy_agents::run::launch(
        agent,
        &o.folder,
        file.as_ref(),
        pansy_agents::run::Models {
            model: o.asked.model.as_deref(),
            ..Default::default()
        },
        &[],
    )?;
    crate::runtime()?.block_on(drive(o, launch))
}

async fn drive(o: Options, launch: pansy_agents::run::Launch) -> Result<()> {
    let json = o.asked.json;
    let (codex, mut events) = Codex::start(&launch, &o.folder, o.resume.as_deref(), o.mode).await?;
    if !json {
        eprintln!(
            "{} on {} in {} ({}). Type to send; /stop interrupts, /quit ends.",
            launch.name,
            launch.model,
            o.folder.display(),
            match o.mode {
                Mode::Ask => "asks before commands and edits",
                Mode::Edits => "asks before commands",
                Mode::Auto => "the agent's own review decides",
                Mode::Full => "full access",
            }
        );
    }
    let mut input = BufReader::new(tokio::io::stdin()).lines();
    let mut turn: Option<String> = None;
    let mut waiting: Vec<String> = Vec::new();
    loop {
        tokio::select! {
            event = events.recv() => {
                let Some(event) = event else { break };
                match &event {
                    Event::TurnStarted { turn: t } => turn = Some(t.clone()),
                    Event::TurnFinished { .. } => turn = None,
                    Event::Approval { request, .. } => waiting.push(request.clone()),
                    Event::Resolved { request } => waiting.retain(|r| r != request),
                    _ => {}
                }
                show(&event, json)?;
                if matches!(event, Event::Exited { .. }) {
                    break;
                }
            }
            line = input.next_line() => {
                let Some(line) = line? else { break };
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if line == "/quit" {
                    break;
                }
                if line == "/stop" {
                    match &turn {
                        Some(t) => codex.interrupt(t).await?,
                        None => eprintln!("nothing runs"),
                    }
                    continue;
                }
                if let Some(request) = waiting.first().cloned() {
                    let decision = match line {
                        "y" => Some(Decision::Accept),
                        "s" => Some(Decision::AcceptForSession),
                        "n" => Some(Decision::Decline),
                        "c" => Some(Decision::Cancel),
                        _ => None,
                    };
                    if let Some(decision) = decision {
                        codex.answer(&request, decision).await?;
                        waiting.retain(|r| *r != request);
                        continue;
                    }
                }
                match &turn {
                    Some(t) => codex.steer(t, line).await?,
                    None => {
                        codex.send(line, None, None).await?;
                    }
                }
            }
        }
    }
    if !json {
        eprintln!(
            "\nResume it: pansy drive --resume {} codex {}",
            codex.thread(),
            o.folder.display()
        );
    }
    codex.stop().await;
    Ok(())
}

fn show(event: &Event, json: bool) -> Result<()> {
    let mut out = std::io::stdout();
    if json {
        writeln!(out, "{}", serde_json::to_string(event)?)?;
        return Ok(());
    }
    match event {
        Event::Thread { thread, model } => writeln!(out, "· thread {thread} on {model}")?,
        Event::TurnStarted { .. } | Event::User { .. } => {}
        Event::Delta { text, .. } => {
            write!(out, "{text}")?;
            out.flush()?;
        }
        Event::Message { .. } => writeln!(out)?,
        Event::ToolStarted { kind, title, .. } => writeln!(out, "· {kind}: {title}")?,
        Event::ToolFinished { kind, status, .. } if status != "completed" => {
            writeln!(out, "· {kind} {status}")?
        }
        Event::ToolFinished { .. } => {}
        Event::Approval {
            title,
            detail,
            reason,
            ..
        } => {
            writeln!(out, "? {title}: {detail}")?;
            if !reason.is_empty() {
                writeln!(out, "  {reason}")?;
            }
            writeln!(
                out,
                "  y: yes, s: yes for this session, n: no, c: no and stop"
            )?;
        }
        Event::Resolved { .. } => {}
        Event::TurnFinished { status, error, .. } => {
            if error.is_empty() {
                writeln!(out, "· {status}")?
            } else {
                writeln!(out, "· {status}: {error}")?
            }
        }
        Event::Exited { code } => writeln!(
            out,
            "· codex ended{}",
            code.map(|c| format!(" ({c})")).unwrap_or_default()
        )?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_then_the_agent_and_a_folder() {
        let args: Vec<String> = [
            "--model", "a/b", "--resume", "t1", "--mode", "ask", "codex", ".",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let o = parse(&args).unwrap();
        assert_eq!(o.asked.model.as_deref(), Some("a/b"));
        assert_eq!(o.resume.as_deref(), Some("t1"));
        assert_eq!(o.mode, Mode::Ask);
        assert_eq!(o.asked.agent, "codex");
        assert!(o.folder.is_absolute());
        assert!(parse(&["--mode".into(), "wild".into(), "codex".into()]).is_err());
        assert!(parse(&[]).is_err());
    }
}
