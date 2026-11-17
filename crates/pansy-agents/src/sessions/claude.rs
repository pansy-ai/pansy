//! Claude Code's sessions: a file each, `projects/<folder>/<id>.jsonl`, of a
//! JSON line per event — a prompt as a `user` line, each block of a reply
//! as an `assistant` line carrying the whole message's usage (so a
//! message's later blocks repeat it), and titles of its own. A session's
//! subagents write files of their own in `<id>/subagents/`, each with what
//! it was asked beside it (`agent-….meta.json`); older versions wrote their
//! lines into the session's file, marked `isSidechain`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use pansy_core::paths::user_home;
use serde::Deserialize;
use serde_json::Value;
use serde_json::value::RawValue;

use super::{
    Keep, Read, Session, Subagent, Tokens, Turn, cut, lines, reads, recent, resume, size_of,
    string, time, tool_args, tool_input, tool_result,
};

fn projects() -> PathBuf {
    user_home().join(".claude").join("projects")
}

/// Each session's file, in the folders of `projects/`.
fn files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in fs::read_dir(projects()).into_iter().flatten().flatten() {
        if !dir.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        for file in fs::read_dir(dir.path()).into_iter().flatten().flatten() {
            let name = file.file_name();
            let name = name.to_string_lossy();
            if name.ends_with(".jsonl")
                && !name.starts_with("agent-")
                && file.file_type().is_ok_and(|t| t.is_file())
            {
                out.push(file.path());
            }
        }
    }
    out
}

/// The folder beside a session's file: `<id>/`, its subagents and tool
/// results.
fn folder(main: &Path) -> PathBuf {
    main.with_file_name(main.file_stem().unwrap_or_default())
}

/// A session's subagents' files.
fn subagent_files(main: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(folder(main).join("subagents"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy())
                .is_some_and(|n| n.starts_with("agent-") && n.ends_with(".jsonl"))
        })
        .collect();
    out.sort();
    out
}

/// The file of `session`'s subagent `sub`.
pub(super) fn subagent_file(session: &Session, sub: &str) -> PathBuf {
    folder(Path::new(&session.path))
        .join("subagents")
        .join(format!("{sub}.jsonl"))
}

/// What a subagent was asked, and its kind (`agent-….meta.json`).
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Meta {
    description: String,
    agent_type: String,
}

fn meta_of(file: &Path) -> Meta {
    fs::read(file.with_extension("meta.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub(super) fn list(seen: &mut HashSet<PathBuf>) -> Vec<Session> {
    let mains = files();
    let subs: Vec<Vec<PathBuf>> = mains.iter().map(|m| subagent_files(m)).collect();
    let main_reads = reads(&mains, read_main, seen);
    let sub_files: Vec<PathBuf> = subs.iter().flatten().cloned().collect();
    let mut sub_reads = reads(&sub_files, read_subagent, seen).into_iter();
    let mut out = Vec::new();
    for ((main, read), subs) in mains.iter().zip(main_reads).zip(&subs) {
        let sub_reads: Vec<Option<Read>> = sub_reads.by_ref().take(subs.len()).collect();
        let Some(r) = read else { continue };
        if r.messages == 0 {
            // nothing was said in it
            continue;
        }
        let id = main
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let mut tokens = r.tokens;
        let mut subagents = Vec::new();
        for (file, sub) in subs.iter().zip(sub_reads) {
            let Some(sub) = sub else { continue };
            tokens.add(&sub.tokens);
            let meta = meta_of(file);
            subagents.push(Subagent {
                id: file
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                title: if meta.description.is_empty() {
                    sub.title()
                } else {
                    super::one_line(&meta.description, 200)
                },
                kind: meta.agent_type,
                last: sub.last,
                messages: sub.messages,
                tokens: sub.tokens,
            });
        }
        let mut files = vec![main.clone()];
        if folder(main).is_dir() {
            files.push(folder(main));
        }
        out.push(Session {
            agent: "claude-code",
            resume: resume("claude-code", &id, &r.cwd),
            id,
            title: r.title(),
            cwd: r.cwd,
            start: r.start,
            last: r.last,
            models: r.models,
            messages: r.messages,
            tokens,
            size: files.iter().map(|f| size_of(f)).sum(),
            path: main.display().to_string(),
            active: recent(main) || subs.iter().any(|f| recent(f)),
            deletable: true,
            subagents,
            files,
        });
    }
    out
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Line<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    timestamp: &'a str,
    #[serde(borrow)]
    cwd: Option<&'a RawValue>,
    is_meta: bool,
    is_sidechain: bool,
    is_compact_summary: bool,
    #[serde(borrow)]
    custom_title: Option<&'a RawValue>,
    #[serde(borrow)]
    ai_title: Option<&'a RawValue>,
    #[serde(borrow)]
    summary: Option<&'a RawValue>,
    #[serde(borrow)]
    message: Option<&'a RawValue>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Message<'a> {
    id: String,
    model: String,
    #[serde(borrow)]
    content: Option<&'a RawValue>,
    usage: Option<Usage>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Usage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
}

impl Usage {
    fn tokens(&self) -> Tokens {
        Tokens {
            input: self.input_tokens.unwrap_or(0),
            output: self.output_tokens.unwrap_or(0),
            cache_read: self.cache_read_input_tokens.unwrap_or(0),
            cache_write: self.cache_creation_input_tokens.unwrap_or(0),
            reasoning: 0,
        }
    }
}

/// A block of a message's content.
#[derive(Deserialize, Default)]
#[serde(default)]
struct Block {
    #[serde(rename = "type")]
    kind: String,
    text: String,
    thinking: String,
    /// A `tool_use`'s id, which its `tool_result` answers in `tool_use_id`.
    id: String,
    name: String,
    input: Value,
    tool_use_id: String,
    content: Value,
    is_error: bool,
}

/// What a `user` line says.
enum Said {
    Prompt(String),
    /// A slash command the user ran: `/model opus`.
    Command(String),
    /// A tool's result, a command's output, a reminder Claude Code put in.
    Nothing,
}

fn said(content: &RawValue) -> Said {
    // most `user` lines are tools' results: told apart by their first block
    let head = &content.get().as_bytes()[..content.get().len().min(256)];
    let has = |pat: &[u8]| head.windows(pat.len()).any(|w| w == pat);
    if has(br#""type":"tool_result""#) || has(br#""tool_use_id":"#) {
        return Said::Nothing;
    }
    let text = if content.get().starts_with('"') {
        string(Some(content))
    } else {
        let Ok(blocks) = serde_json::from_str::<Vec<Block>>(content.get()) else {
            return Said::Nothing;
        };
        if blocks.iter().any(|b| b.kind == "tool_result") {
            return Said::Nothing;
        }
        let parts: Vec<&str> = blocks
            .iter()
            .filter_map(|b| match b.kind.as_str() {
                "text" if !b.text.trim_start().starts_with("<system-reminder>") => {
                    Some(b.text.as_str())
                }
                "image" => Some("[image]"),
                _ => None,
            })
            .collect();
        parts.join("\n\n")
    };
    let text = text.trim();
    if let Some(name) = between(text, "<command-name>", "</command-name>") {
        let args = between(text, "<command-args>", "</command-args>").unwrap_or("");
        let name = if name.starts_with('/') {
            name.to_owned()
        } else {
            format!("/{name}")
        };
        return Said::Command(format!("{name} {args}").trim().to_owned());
    }
    if let Some(command) = between(text, "<bash-input>", "</bash-input>") {
        return Said::Command(format!("! {command}"));
    }
    if text.is_empty()
        || text.starts_with("<local-command-")
        || text.starts_with("<bash-std")
        || text.starts_with("Caveat: The messages below were generated")
    {
        return Said::Nothing;
    }
    Said::Prompt(text.to_owned())
}

/// What is between `open` and `close` in `text`.
fn between<'t>(text: &'t str, open: &str, close: &str) -> Option<&'t str> {
    let from = text.find(open)? + open.len();
    let to = text[from..].find(close)? + from;
    Some(text[from..to].trim())
}

/// A reply Claude Code made up itself (an error, an interruption), on no model.
const SYNTHETIC: &str = "<synthetic>";

fn read_main(path: &Path) -> Option<super::Read> {
    read(path, true)
}

fn read_subagent(path: &Path) -> Option<super::Read> {
    read(path, false)
}

/// What a session's file says. `main`: the session's own file, whose
/// `isSidechain` lines are a subagent's (only their tokens are counted).
fn read(path: &Path, main: bool) -> Option<super::Read> {
    let mut r = super::Read::default();
    // a message's usage, by its id: its later blocks repeat it
    let mut usage: HashMap<String, Tokens> = HashMap::new();
    let mut replies: HashSet<String> = HashSet::new();
    lines(path, |bytes| {
        let Ok(line) = serde_json::from_slice::<Line>(bytes) else {
            return;
        };
        r.saw(time(line.timestamp));
        if r.cwd.is_empty() {
            r.cwd = string(line.cwd);
        }
        let own = !main || !line.is_sidechain;
        match line.kind {
            "custom-title" => r.titled(3, &string(line.custom_title)),
            "ai-title" => r.titled(2, &string(line.ai_title)),
            "summary" => r.titled(1, &string(line.summary)),
            "user" if own && !line.is_meta && !line.is_compact_summary => {
                let Some(message) = line.message else { return };
                let Ok(message) = serde_json::from_str::<Message>(message.get()) else {
                    return;
                };
                if let Some(Said::Prompt(text)) = message.content.map(said) {
                    r.messages += 1;
                    r.prompted(&text);
                }
            }
            "assistant" => {
                let Some(message) = line.message else { return };
                let Ok(message) = serde_json::from_str::<Message>(message.get()) else {
                    return;
                };
                if message.model == SYNTHETIC {
                    return;
                }
                if own {
                    r.model(&message.model);
                    if message.id.is_empty() || replies.insert(message.id.clone()) {
                        r.messages += 1;
                    }
                }
                if let Some(u) = &message.usage {
                    if message.id.is_empty() {
                        r.tokens.add(&u.tokens());
                    } else {
                        usage.insert(message.id, u.tokens());
                    }
                }
            }
            _ => {}
        }
    })
    .ok()?;
    for t in usage.values() {
        r.tokens.add(t);
    }
    Some(r)
}

/// What was said in a session's file (`main`) or a subagent's.
pub(super) fn transcript(path: &Path, main: bool, keep: Keep) -> Result<Vec<Turn>> {
    let mut turns: Vec<Turn> = Vec::new();
    // the reply the last turn is of, to put its blocks together
    let mut reply = String::new();
    // each tool call's turn, by its id, for its result to find
    let mut calls: HashMap<String, usize> = HashMap::new();
    lines(path, |bytes| {
        let Ok(line) = serde_json::from_slice::<Line>(bytes) else {
            return;
        };
        if (main && line.is_sidechain) || line.is_meta {
            return;
        }
        let at = time(line.timestamp);
        let Some(message) = line.message else { return };
        let Ok(message) = serde_json::from_str::<Message>(message.get()) else {
            return;
        };
        match line.kind {
            "user" if line.is_compact_summary => {
                reply.clear();
                turns.push(Turn {
                    role: "compact",
                    at,
                    ..Turn::default()
                });
            }
            "user" => {
                let Some(content) = message.content else {
                    return;
                };
                if answered(content, &calls, &mut turns, keep) {
                    return;
                }
                let text = match said(content) {
                    Said::Prompt(text) | Said::Command(text) => text,
                    Said::Nothing => return,
                };
                reply.clear();
                turns.push(Turn {
                    role: "user",
                    text: cut(&text, keep.text),
                    at,
                    ..Turn::default()
                });
            }
            "assistant" if message.model != SYNTHETIC => {
                let Some(content) = message.content else {
                    return;
                };
                let Ok(blocks) = serde_json::from_str::<Vec<Block>>(content.get()) else {
                    return;
                };
                for block in blocks {
                    match block.kind.as_str() {
                        "text" if !block.text.trim().is_empty() => {
                            match turns.last_mut() {
                                Some(last)
                                    if last.role == "assistant"
                                        && !message.id.is_empty()
                                        && reply == message.id =>
                                {
                                    last.text = cut(
                                        &format!("{}\n\n{}", last.text, block.text.trim()),
                                        keep.text,
                                    );
                                }
                                _ => turns.push(Turn {
                                    role: "assistant",
                                    text: cut(block.text.trim(), keep.text),
                                    at,
                                    ..Turn::default()
                                }),
                            }
                            reply.clone_from(&message.id);
                        }
                        "thinking" if !block.thinking.trim().is_empty() => {
                            turns.push(Turn {
                                role: "reasoning",
                                text: cut(block.thinking.trim(), keep.text),
                                at,
                                ..Turn::default()
                            });
                            reply.clear();
                        }
                        "tool_use" => {
                            if !block.id.is_empty() {
                                calls.insert(block.id, turns.len());
                            }
                            turns.push(Turn {
                                role: "tool",
                                text: tool_input(&block.input),
                                input: tool_args(&block.input, keep.tool),
                                name: block.name,
                                at,
                                ..Turn::default()
                            });
                            reply.clear();
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    })?;
    Ok(turns)
}

/// Puts the tools' results a `user` line gives back on the calls they
/// answer; whether it gave any.
fn answered(
    content: &RawValue,
    calls: &HashMap<String, usize>,
    turns: &mut [Turn],
    keep: Keep,
) -> bool {
    if !content.get().starts_with('[') {
        return false;
    }
    let Ok(blocks) = serde_json::from_str::<Vec<Block>>(content.get()) else {
        return false;
    };
    let mut any = false;
    for b in blocks.iter().filter(|b| b.kind == "tool_result") {
        any = true;
        if let Some(turn) = calls.get(&b.tool_use_id).and_then(|&i| turns.get_mut(i)) {
            turn.output = tool_result(&b.content, keep.tool);
            turn.status = if b.is_error { "error" } else { "done" };
        }
    }
    any
}
