//! Codex's sessions: a rollout file each, under
//! `sessions/<year>/<month>/<day>/rollout-<time>-<id>.jsonl`, of a JSON line
//! per item: `session_meta` (its id, its folder, and where it came from: a
//! thread another spawned names that one), `turn_context` (the model),
//! `response_item` (the prompts, the replies and the tools called) and
//! `event_msg` (among them the tokens spent so far). The names threads were
//! given are in `session_index.jsonl`. A thread another spawned is listed
//! under that one, as its subagent; one whose thread is gone is not listed.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use serde_json::value::RawValue;

use super::{
    Keep, Read, Session, Subagent, Tokens, Turn, cut, lines, one_line, reads, recent, resume,
    size_of, string, time, tool_args, tool_input, tool_result,
};
use crate::agent::codex_home;

/// Each rollout file under `sessions/`.
fn files() -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if kind.is_dir() && depth < 4 {
                walk(&entry.path(), depth + 1, out);
            } else if kind.is_file() && name.starts_with("rollout-") && name.ends_with(".jsonl") {
                out.push(entry.path());
            }
        }
    }
    let mut out = Vec::new();
    walk(&codex_home().join("sessions"), 0, &mut out);
    out
}

/// The names threads were given, by id: the latest of each.
fn names() -> HashMap<String, String> {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Named {
        id: String,
        thread_name: String,
    }
    let mut out = HashMap::new();
    let _ = lines(&codex_home().join("session_index.jsonl"), |bytes| {
        if let Ok(n) = serde_json::from_slice::<Named>(bytes)
            && !n.id.is_empty()
            && !n.thread_name.trim().is_empty()
        {
            out.insert(n.id, one_line(&n.thread_name, 200));
        }
    });
    out
}

pub(super) fn list(seen: &mut HashSet<PathBuf>) -> Vec<Session> {
    let files = files();
    let rows: Vec<(PathBuf, Read)> = files
        .iter()
        .cloned()
        .zip(reads(&files, read, seen))
        .filter_map(|(path, read)| Some((path, read?)))
        .filter(|(_, r)| !r.id.is_empty())
        .collect();
    let names = names();
    let title = |r: &Read| names.get(&r.id).cloned().unwrap_or_else(|| r.title());
    let by_id: HashMap<&str, usize> = rows
        .iter()
        .enumerate()
        .map(|(i, (_, r))| (r.id.as_str(), i))
        .collect();
    // The thread each is under, following parents; none for a subagent
    // whose thread is not here.
    let root_of = |mut at: usize| -> Option<usize> {
        for _ in 0..16 {
            let r = &rows[at].1;
            if !r.subagent {
                return Some(at);
            }
            at = *by_id.get(r.parent.as_str())?;
        }
        None
    };
    let mut under: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..rows.len() {
        match root_of(i) {
            Some(root) if root != i => under.entry(root).or_default().push(i),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for (i, (path, r)) in rows.iter().enumerate() {
        if root_of(i) != Some(i) || r.messages == 0 {
            continue;
        }
        let kids = under.get(&i).map(Vec::as_slice).unwrap_or_default();
        let mut tokens = r.tokens;
        let mut files = vec![path.clone()];
        let mut subagents = Vec::new();
        for &k in kids {
            let (kid_path, kid) = &rows[k];
            tokens.add(&kid.tokens);
            files.push(kid_path.clone());
            subagents.push(Subagent {
                id: kid.id.clone(),
                title: title(kid),
                kind: String::new(),
                last: kid.last,
                messages: kid.messages,
                tokens: kid.tokens,
            });
        }
        subagents.sort_by_key(|a| a.last);
        out.push(Session {
            agent: "codex",
            id: r.id.clone(),
            cwd: r.cwd.clone(),
            title: title(r),
            start: r.start,
            last: r.last,
            models: r.models.clone(),
            messages: r.messages,
            tokens,
            size: files.iter().map(|f| size_of(f)).sum(),
            path: path.display().to_string(),
            resume: resume("codex", &r.id, &r.cwd),
            active: files.iter().any(|f| recent(f)),
            deletable: true,
            subagents,
            files,
        });
    }
    out
}

/// The rollout file of the thread `id`.
pub(super) fn path_of(id: &str) -> Option<PathBuf> {
    let files = files();
    let reads = reads(&files, read, &mut HashSet::new());
    files
        .into_iter()
        .zip(reads)
        .find(|(_, r)| r.as_ref().is_some_and(|r| r.id == id))
        .map(|(path, _)| path)
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Line<'a> {
    timestamp: &'a str,
    #[serde(rename = "type")]
    kind: &'a str,
    #[serde(borrow)]
    payload: Option<&'a RawValue>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Meta {
    id: String,
    cwd: String,
    timestamp: String,
    source: Value,
    parent_thread_id: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Context {
    model: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Event<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    info: Option<Info>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Info {
    total_token_usage: Option<Usage>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Record {
    thread_token_usage: Option<Usage>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Usage {
    input_tokens: Option<u64>,
    cached_input_tokens: Option<u64>,
    cache_write_input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    reasoning_output_tokens: Option<u64>,
}

impl Usage {
    fn tokens(&self) -> Tokens {
        let cached = self.cached_input_tokens.unwrap_or(0);
        Tokens {
            input: self.input_tokens.unwrap_or(0).saturating_sub(cached),
            output: self.output_tokens.unwrap_or(0),
            cache_read: cached,
            cache_write: self.cache_write_input_tokens.unwrap_or(0),
            reasoning: self.reasoning_output_tokens.unwrap_or(0),
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Item<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    role: &'a str,
    #[serde(borrow)]
    content: Option<&'a RawValue>,
    #[serde(borrow)]
    name: Option<&'a RawValue>,
    #[serde(borrow)]
    arguments: Option<&'a RawValue>,
    #[serde(borrow)]
    input: Option<&'a RawValue>,
    action: Value,
    /// A tool call's id, which its `…_output` item answers.
    #[serde(borrow)]
    call_id: Option<&'a RawValue>,
    #[serde(borrow)]
    output: Option<&'a RawValue>,
    #[serde(borrow)]
    status: Option<&'a RawValue>,
    /// A `reasoning` item's summary: blocks of `summary_text`.
    #[serde(borrow)]
    summary: Option<&'a RawValue>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Block {
    #[serde(rename = "type")]
    kind: String,
    text: String,
}

/// Text Codex puts in a user message of its own: the folder and shell, the
/// project's instructions, its internal context.
fn context(text: &str) -> bool {
    text.is_empty()
        || text.starts_with("# AGENTS.md instructions")
        || text.starts_with('<') && text.ends_with('>')
}

/// The words of a message's content: a prompt's, without what Codex put
/// in, or a reply's.
fn words(content: Option<&RawValue>, user: bool) -> String {
    let Some(blocks) = content.and_then(|c| serde_json::from_str::<Vec<Block>>(c.get()).ok())
    else {
        return String::new();
    };
    let parts: Vec<&str> = blocks
        .iter()
        .filter_map(|b| match b.kind.as_str() {
            "input_text" if user && !context(b.text.trim()) => Some(b.text.trim()),
            "input_image" if user => Some("[image]"),
            "output_text" if !user => Some(b.text.trim()),
            _ => None,
        })
        .filter(|t| !t.is_empty())
        .collect();
    parts.join("\n\n")
}

/// The thread that spawned this one, from its `source` and
/// `parent_thread_id`: whether it was spawned, and by which.
fn spawned(meta: &Meta) -> (bool, String) {
    let sub = meta.source.get("subagent");
    let parent = sub
        .and_then(|s| s.get("thread_spawn"))
        .and_then(|t| t.get("parent_thread_id"))
        .and_then(Value::as_str)
        .or(meta.parent_thread_id.as_deref())
        .unwrap_or_default()
        .to_owned();
    (sub.is_some() || !parent.is_empty(), parent)
}

fn read(path: &Path) -> Option<Read> {
    let mut r = Read::default();
    let mut meta = false;
    let mut total: Option<Tokens> = None;
    lines(path, |bytes| {
        let Ok(line) = serde_json::from_slice::<Line>(bytes) else {
            return;
        };
        r.saw(time(line.timestamp));
        let Some(payload) = line.payload else { return };
        match line.kind {
            "session_meta" if !meta => {
                let Ok(m) = serde_json::from_str::<Meta>(payload.get()) else {
                    return;
                };
                meta = true;
                r.saw(time(&m.timestamp));
                (r.subagent, r.parent) = spawned(&m);
                r.id = m.id;
                r.cwd = m.cwd;
            }
            "turn_context" => {
                if let Ok(c) = serde_json::from_str::<Context>(payload.get()) {
                    r.model(&c.model);
                }
            }
            "event_msg" => {
                if let Ok(Event {
                    kind: "token_count",
                    info:
                        Some(Info {
                            total_token_usage: Some(u),
                        }),
                }) = serde_json::from_str::<Event>(payload.get())
                {
                    total = Some(u.tokens());
                }
            }
            "token_usage_record" => {
                if let Ok(Record {
                    thread_token_usage: Some(u),
                }) = serde_json::from_str::<Record>(payload.get())
                {
                    total = Some(u.tokens());
                }
            }
            "response_item" => {
                let Ok(item) = serde_json::from_str::<Item>(payload.get()) else {
                    return;
                };
                if item.kind != "message" {
                    return;
                }
                match item.role {
                    "user" => {
                        let text = words(item.content, true);
                        if !text.is_empty() {
                            r.messages += 1;
                            r.prompted(&text);
                        }
                    }
                    "assistant" => r.messages += 1,
                    _ => {}
                }
            }
            _ => {}
        }
    })
    .ok()?;
    if r.id.is_empty() {
        // a rollout names its thread at its end too: `…-<id>.jsonl`
        let stem = path.file_stem()?.to_string_lossy();
        if stem.len() > 36 {
            r.id = stem[stem.len() - 36..].to_owned();
        }
    }
    r.tokens = total.unwrap_or_default();
    Some(r)
}

/// What a tool call was given, in a line.
fn call(item: &Item) -> String {
    match item.kind {
        "function_call" => {
            let args = string(item.arguments);
            match serde_json::from_str::<Value>(&args) {
                Ok(v) => tool_input(&v),
                Err(_) => one_line(&args, 300),
            }
        }
        "custom_tool_call" => {
            let input = string(item.input);
            // a patch, by the files it changes
            let files: Vec<&str> = input
                .lines()
                .filter_map(|l| {
                    l.strip_prefix("*** Update File: ")
                        .or_else(|| l.strip_prefix("*** Add File: "))
                        .or_else(|| l.strip_prefix("*** Delete File: "))
                })
                .collect();
            if files.is_empty() {
                one_line(&input, 300)
            } else {
                one_line(&files.join(", "), 300)
            }
        }
        _ => tool_input(&item.action),
    }
}

/// What a tool call was given, whole: its arguments laid out, its patch.
fn call_args(item: &Item, max: usize) -> String {
    match item.kind {
        "function_call" => {
            let args = string(item.arguments);
            match serde_json::from_str::<Value>(&args) {
                Ok(v) => tool_args(&v, max),
                Err(_) => tool_args(&Value::String(args), max),
            }
        }
        "custom_tool_call" => tool_args(&Value::String(string(item.input)), max),
        _ => tool_args(&item.action, max),
    }
}

/// What a tool gave back, and whether it failed: a command that exited
/// with another code than 0 (as `{"output", "metadata": {"exit_code"}}`,
/// or a line `Exit code: 1` or `Process exited with code 1`), or a result
/// that says `"success": false`. A command's output is what it printed.
fn outcome(output: Option<&RawValue>, max: usize) -> (String, bool) {
    let Some(raw) = output else {
        return (String::new(), false);
    };
    let value: Value = serde_json::from_str(raw.get()).unwrap_or(Value::Null);
    // a string may be JSON of its own
    let value = match &value {
        Value::String(s) if s.starts_with('{') => serde_json::from_str(s).unwrap_or(value),
        _ => value,
    };
    let failed = value.get("success").and_then(Value::as_bool) == Some(false)
        || value
            .pointer("/metadata/exit_code")
            .and_then(Value::as_i64)
            .is_some_and(|c| c != 0);
    let body = value
        .get("output")
        .or_else(|| value.get("content"))
        .unwrap_or(&value);
    let text = tool_result(body, max);
    let exited = text.lines().take(6).any(|l| {
        l.strip_prefix("Exit code: ")
            .or_else(|| l.strip_prefix("Process exited with code "))
            .and_then(|c| c.trim().parse::<i64>().ok())
            .is_some_and(|c| c != 0)
    });
    // what the command printed, without the lines Codex put before it
    // (`Chunk ID`, `Wall time`, the exit code, `Original token count`)
    let printed = text
        .split_once("\nOutput:\n")
        .filter(|(head, _)| {
            head.lines().count() <= 6
                && head
                    .lines()
                    .all(|l| l.contains(": ") || l.starts_with("Process exited"))
        })
        .map_or(text.as_str(), |(_, out)| out)
        .trim()
        .to_owned();
    (printed, failed || exited)
}

/// A `reasoning` item's summary, its blocks one after another.
fn reasoning(item: &Item) -> String {
    let Some(blocks) = item
        .summary
        .and_then(|c| serde_json::from_str::<Vec<Block>>(c.get()).ok())
    else {
        return String::new();
    };
    let parts: Vec<&str> = blocks
        .iter()
        .map(|b| b.text.trim())
        .filter(|t| !t.is_empty())
        .collect();
    parts.join("\n\n")
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct TaskEvent<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    duration_ms: Option<u64>,
    reason: &'a str,
}

/// A turn's end (`task_complete`, `turn_aborted`), how long it took as
/// Codex wrote it.
fn turn_end(payload: Option<&RawValue>, at: Option<DateTime<Utc>>) -> Option<Turn> {
    let e = serde_json::from_str::<TaskEvent>(payload?.get()).ok()?;
    let status = match e.kind {
        "task_complete" => "",
        "turn_aborted" if e.reason == "interrupted" => "stopped",
        "turn_aborted" => "",
        _ => return None,
    };
    Some(Turn {
        role: "end",
        status,
        ms: e.duration_ms,
        at,
        ..Turn::default()
    })
}

/// What was said in a rollout file.
pub(super) fn transcript(path: &Path, keep: Keep) -> Result<Vec<Turn>> {
    let mut turns: Vec<Turn> = Vec::new();
    // each tool call's turn, by its id, for its output to find
    let mut calls: HashMap<String, usize> = HashMap::new();
    lines(path, |bytes| {
        let Ok(line) = serde_json::from_slice::<Line>(bytes) else {
            return;
        };
        let at = time(line.timestamp);
        if line.kind == "compacted" {
            turns.push(Turn {
                role: "compact",
                at,
                ..Turn::default()
            });
            return;
        }
        if line.kind == "event_msg" && keep.ends {
            if let Some(end) = turn_end(line.payload, at) {
                turns.push(end);
            }
            return;
        }
        if line.kind != "response_item" {
            return;
        }
        let Some(Ok(item)) = line.payload.map(|p| serde_json::from_str::<Item>(p.get())) else {
            return;
        };
        if item.kind.ends_with("_output") {
            let (output, failed) = outcome(item.output, keep.tool);
            if let Some(turn) = calls
                .get(&string(item.call_id))
                .and_then(|&i| turns.get_mut(i))
            {
                turn.output = output;
                turn.status = if failed { "error" } else { "done" };
            }
            return;
        }
        let (role, name, text) = match (item.kind, item.role) {
            ("message", "user") => ("user", String::new(), words(item.content, true)),
            ("message", "assistant") => ("assistant", String::new(), words(item.content, false)),
            ("reasoning", _) => ("reasoning", String::new(), reasoning(&item)),
            ("function_call" | "custom_tool_call", _) => ("tool", string(item.name), call(&item)),
            ("local_shell_call", _) => ("tool", "shell".to_owned(), call(&item)),
            ("web_search_call", _) => ("tool", "web_search".to_owned(), call(&item)),
            _ => return,
        };
        if text.is_empty() && role != "tool" {
            return;
        }
        let mut turn = Turn {
            role,
            name,
            text: cut(&text, keep.text),
            at,
            ..Turn::default()
        };
        if role == "tool" {
            turn.input = call_args(&item, keep.tool);
            // a search keeps no output: only that it was done
            if string(item.status) == "completed" {
                turn.status = "done";
            }
            let id = string(item.call_id);
            if !id.is_empty() {
                calls.insert(id, turns.len());
            }
        }
        turns.push(turn);
    })?;
    Ok(turns)
}
