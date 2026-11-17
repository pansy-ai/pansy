//! A session's whole conversation as Markdown, as magpie's Export Markdown
//! writes it: its title, what it is (its agent, id, folder, times, models
//! and file), then each prompt and reply under who said it, what the model
//! thought and each tool's result folded in `<details>`, a tool's input and
//! output fenced. Read from the agent's own file, every turn whole, and
//! saved into the user's Downloads folder; the agent's file is only read.

use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use anyhow::{Result, anyhow};
use chrono::{DateTime, Local, Utc};

use super::{Keep, Session, Turn, find, one_line, transcript_of};

/// The session `id` of `agent` as Markdown, and the name its file is saved
/// under (without `.md`).
pub fn markdown(agent: &str, id: &str) -> Result<(String, String)> {
    let s = find(agent, id).ok_or_else(|| anyhow!("no {agent} session {id}"))?;
    let turns = transcript_of(&s, Keep::WHOLE)?;
    Ok((stem(&s), write(&s, &turns)))
}

/// Saves the session `id` of `agent` as Markdown in the Downloads folder,
/// beside any file of the same name: where it went.
pub fn export(agent: &str, id: &str) -> Result<PathBuf> {
    let (stem, md) = markdown(agent, id)?;
    let dir = pansy_core::paths::downloads();
    fs::create_dir_all(&dir)?;
    let path = (1..)
        .map(|n| match n {
            1 => dir.join(format!("{stem}.md")),
            n => dir.join(format!("{stem} ({n}).md")),
        })
        .find(|p| !p.exists())
        .expect("a free name");
    fs::write(&path, md)?;
    Ok(path)
}

/// `pansy-session-<agent>-<the id's first 12 characters>`, in characters a
/// file name can have anywhere.
fn stem(s: &Session) -> String {
    let short: String = s.id.chars().take(12).collect();
    let safe: String = format!("{}-{short}", s.agent)
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!("pansy-session-{safe}")
}

fn write(s: &Session, turns: &[Turn]) -> String {
    let mut b = String::new();
    let title = one_line(&s.title, 200);
    let title = if title.is_empty() {
        "(no prompt)"
    } else {
        &title
    };
    let _ = writeln!(b, "# {title}\n");
    let mut meta = |k: &str, v: &str| {
        if !v.is_empty() {
            let _ = writeln!(b, "- **{k}:** {v}");
        }
    };
    let agent = crate::agent::find(s.agent).map_or(s.agent, |a| a.name);
    meta("Agent", agent);
    meta("Session ID", &format!("`{}`", s.id));
    meta("Folder", &s.cwd);
    meta("Started", &when(s.start));
    meta("Last active", &when(s.last));
    meta("Models", &s.models.join(", "));
    meta("File", &s.path);
    b.push_str("\n---\n");
    // who the last heading named: a tool's call and result are its reply's
    let mut who = "";
    for t in turns {
        let role = match t.role {
            "user" => "user",
            "compact" => "compact",
            _ => "assistant",
        };
        if role != who && role != "compact" {
            who = role;
            b.push_str(if role == "user" {
                "\n## User\n"
            } else {
                "\n## Assistant\n"
            });
        }
        b.push('\n');
        match t.role {
            "compact" => {
                who = "";
                b.push_str("*The conversation was compacted here.*\n");
            }
            "reasoning" => folded(&mut b, "Thinking", t.text.trim(), false),
            "tool" => {
                let name = if t.name.is_empty() { "tool" } else { &t.name };
                let _ = writeln!(b, "**Tool call: {}**\n", one_line(name, 100));
                let input = if t.input.is_empty() {
                    &t.text
                } else {
                    &t.input
                };
                if !input.is_empty() {
                    fenced(&mut b, language(input), input);
                }
                if !t.output.is_empty() {
                    let label = match t.status {
                        "error" => format!("Tool error: {}", one_line(name, 100)),
                        _ => format!("Tool result: {}", one_line(name, 100)),
                    };
                    b.push('\n');
                    folded(&mut b, &label, &t.output, true);
                }
            }
            _ => {
                b.push_str(t.text.trim());
                b.push('\n');
            }
        }
    }
    if turns.is_empty() {
        b.push_str("\n*Nothing was said in this session.*\n");
    }
    b
}

/// A time in the computer's own zone: `2026-11-10 10:00`.
fn when(at: Option<DateTime<Utc>>) -> String {
    at.map(|t| t.with_timezone(&Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

/// The language `text` is in, as far as its start tells: JSON, or a patch.
fn language(text: &str) -> &'static str {
    let t = text.trim_start();
    if t.starts_with('{') || t.starts_with('[') {
        "json"
    } else if t.starts_with("*** Begin Patch") {
        "diff"
    } else {
        ""
    }
}

/// `text` in a code block, its fence longer than any run of backticks in it.
fn fenced(b: &mut String, language: &str, text: &str) {
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    let _ = writeln!(b, "{fence}{language}\n{}\n{fence}", text.trim_end());
}

/// A `<details>` block under `label`: its words as they are, or, `code`, in
/// a fence (a tool's output holds tags and Markdown of its own, which would
/// otherwise be drawn as this file's).
fn folded(b: &mut String, label: &str, text: &str, code: bool) {
    let _ = writeln!(b, "<details>\n<summary>{}</summary>\n", escape(label));
    if code {
        fenced(b, "", text);
    } else {
        b.push_str(text);
        b.push('\n');
    }
    b.push_str("\n</details>\n");
}

/// `text` as HTML's text.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
