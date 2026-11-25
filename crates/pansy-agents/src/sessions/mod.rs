//! The agents' sessions on this computer, read from their own files as
//! magpie's Sessions page reads them: Claude Code's
//! `~/.claude/projects/<folder>/<id>.jsonl`, its subagents' beside it in
//! `<id>/subagents/`, Codex's rollout files under `$CODEX_HOME/sessions/`,
//! and OpenCode's database (or its older JSON files).
//! Each session's folder, title, times, models, messages and the tokens it
//! spent (its subagents' included), the command that resumes it, and what
//! was said in it.
//!
//! The agents' files are only ever read, except by Delete, which moves a
//! session's files into pansy's trash ([`trash`]) and erases nothing;
//! OpenCode's sessions are rows of a database it keeps open, and are only
//! listed. A file's reading (or a row's) is kept in memory by its size and
//! time, so the list comes back at once the next time; one that changed is
//! read again.

mod claude;
mod codex;
pub mod markdown;
mod opencode;
#[cfg(test)]
mod tests;
pub mod trash;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, SystemTime};

use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_json::value::RawValue;

/// The agents whose sessions pansy reads, by their ids in `agent.rs`.
pub const AGENTS: &[&str] = &["claude-code", "codex", "opencode"];

/// How recently a session's files may have been written for it to count as
/// running: such a one is not deleted.
pub const ACTIVE: Duration = Duration::from_secs(60);

/// Tokens a session spent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tokens {
    /// Read fresh: what came from the cache is apart.
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    /// Reasoning, inside output.
    pub reasoning: u64,
}

impl Tokens {
    fn add(&mut self, t: &Tokens) {
        self.input += t.input;
        self.output += t.output;
        self.cache_read += t.cache_read;
        self.cache_write += t.cache_write;
        self.reasoning += t.reasoning;
    }

    /// Every token, read or written.
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }
}

/// One session of an agent, as the Sessions page and `pansy sessions` list it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub agent: &'static str,
    pub id: String,
    /// The folder it ran in; empty when its agent kept none.
    pub cwd: String,
    /// The name it was given, else its agent's title, else its first prompt.
    pub title: String,
    pub start: Option<DateTime<Utc>>,
    pub last: Option<DateTime<Utc>>,
    pub models: Vec<String>,
    /// The prompts and the replies.
    pub messages: u32,
    /// What it and its subagents spent.
    pub tokens: Tokens,
    /// Its files' bytes.
    pub size: u64,
    /// Its main file.
    pub path: String,
    /// The shell line that picks it up again in its folder.
    pub resume: String,
    /// Written to in the last minute: its agent may be running it.
    pub active: bool,
    /// Delete can move it into the trash: its files are its own (not
    /// OpenCode's rows, in a database OpenCode keeps open).
    pub deletable: bool,
    pub subagents: Vec<Subagent>,
    /// Its files and folders: what Delete moves.
    #[serde(skip)]
    files: Vec<PathBuf>,
}

/// A conversation a session's agent started inside it: a Claude Code
/// subagent, or a Codex thread it spawned.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Subagent {
    pub id: String,
    /// What it was asked to do.
    pub title: String,
    /// The kind its agent named it (Claude Code's `Explore`), if any.
    pub kind: String,
    pub last: Option<DateTime<Utc>>,
    pub messages: u32,
    pub tokens: Tokens,
}

/// One turn of a conversation.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Turn {
    /// `user`, `assistant`, `reasoning` (what the model thought, as its
    /// agent kept it), `tool` (a tool the agent called: its name, and what
    /// it was given in a line), `compact` (the conversation was compacted
    /// here), or, kept only for [`Keep::TIMED`], `end` (a turn ended here:
    /// `ms` it took where the agent wrote it, `status` `stopped` when the
    /// user stopped it).
    pub role: &'static str,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub text: String,
    /// A tool's: what it was given, whole (JSON laid out, or a patch), cut
    /// at [`Keep::tool`] characters.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub input: String,
    /// A tool's: what it gave back, cut at [`Keep::tool`] characters.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub output: String,
    /// A tool's: `done`, `error`, or empty when no result was kept (it was
    /// cut off, or is still running).
    #[serde(skip_serializing_if = "str::is_empty")]
    pub status: &'static str,
    pub at: Option<DateTime<Utc>>,
    /// An `end`'s: how long the turn took, as its agent wrote it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ms: Option<u64>,
}

/// How much of each turn a transcript keeps, in characters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Keep {
    /// A prompt's, a reply's, a thought's.
    pub text: usize,
    /// A tool's input, and its output.
    pub tool: usize,
    /// Where each turn ended, as `end` turns.
    pub ends: bool,
}

impl Keep {
    /// What the window shows.
    pub const SHOWN: Keep = Keep {
        text: 8000,
        tool: 4000,
        ends: false,
    };
    /// What the phone shows: the window's, and where each turn ended.
    pub const TIMED: Keep = Keep {
        ends: true,
        ..Keep::SHOWN
    };
    /// All of it: an export.
    pub const WHOLE: Keep = Keep {
        text: usize::MAX,
        tool: usize::MAX,
        ends: false,
    };
}

/// A session found by what was said in it: where, in a few words.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub agent: &'static str,
    pub id: String,
    pub text: String,
}

/// Every session of the agents pansy reads, the latest first.
pub fn list() -> Vec<Session> {
    let mut seen = HashSet::new();
    let mut out = claude::list(&mut seen);
    out.extend(codex::list(&mut seen));
    out.extend(opencode::list(&mut seen));
    READS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|path, _| seen.contains(path));
    out.sort_by(|a, b| b.last.cmp(&a.last).then_with(|| a.id.cmp(&b.id)));
    out
}

/// The session `id` of `agent`.
pub fn find(agent: &str, id: &str) -> Option<Session> {
    let mut seen = HashSet::new();
    let all = match agent {
        "claude-code" => claude::list(&mut seen),
        "codex" => codex::list(&mut seen),
        "opencode" => opencode::list(&mut seen),
        _ => return None,
    };
    all.into_iter().find(|s| s.id == id)
}

/// What was said in the session `id` of `agent`, or in its subagent `sub`.
pub fn transcript(agent: &str, id: &str, sub: &str) -> Result<Vec<Turn>> {
    let session = find(agent, id).ok_or_else(|| anyhow!("no {agent} session {id}"))?;
    if sub.is_empty() {
        return transcript_of(&session, Keep::SHOWN);
    }
    if !plain_id(sub) || !session.subagents.iter().any(|s| s.id == sub) {
        bail!("no subagent {sub} in {agent} session {id}");
    }
    match agent {
        "claude-code" => {
            claude::transcript(&claude::subagent_file(&session, sub), false, Keep::SHOWN)
        }
        "opencode" => opencode::transcript(sub, Keep::SHOWN),
        _ => {
            let path = codex::path_of(sub).ok_or_else(|| anyhow!("no Codex thread {sub}"))?;
            codex::transcript(&path, Keep::SHOWN)
        }
    }
}

/// What was said in the session `id` of `agent`, with where each turn
/// ended (`end` turns): the phone's thread.
pub fn timed_transcript(agent: &str, id: &str) -> Result<Vec<Turn>> {
    let session = find(agent, id).ok_or_else(|| anyhow!("no {agent} session {id}"))?;
    transcript_of(&session, Keep::TIMED)
}

/// What was said in a session listed, as much of each turn as `keep` says.
fn transcript_of(s: &Session, keep: Keep) -> Result<Vec<Turn>> {
    match s.agent {
        "claude-code" => claude::transcript(Path::new(&s.path), true, keep),
        "opencode" => opencode::transcript(&s.id, keep),
        _ => codex::transcript(Path::new(&s.path), keep),
    }
}

/// The sessions whose prompts or replies say `query` (case aside), the
/// latest first, `limit` at most.
pub fn search(query: &str, limit: usize) -> Vec<Hit> {
    let needle = query.split_whitespace().collect::<Vec<_>>().join(" ");
    if needle.chars().count() < 2 {
        return Vec::new();
    }
    let lower = needle.to_lowercase();
    // A file is looked through as it is first: a needle JSON writes as it
    // is can only be in a file that has it, case aside.
    let raw = !needle.contains(['"', '\\']) && !needle.chars().any(char::is_control);
    let raw_needle = needle.to_ascii_lowercase().into_bytes();
    // OpenCode's database is asked which of its sessions have it
    let in_db = if raw {
        opencode::matching(&needle)
    } else {
        None
    };
    let sessions = list();
    let found = parallel(&sessions, |s| {
        let maybe = match (s.agent, &in_db) {
            ("opencode", Some(ids)) => ids.contains(&s.id),
            ("opencode", None) => true,
            _ => !raw || any_line_has(Path::new(&s.path), &raw_needle),
        };
        if !maybe {
            return None;
        }
        let turns = transcript_of(s, Keep::SHOWN).ok()?;
        turns
            .iter()
            .filter(|t| t.role == "user" || t.role == "assistant")
            .find_map(|t| snippet(&t.text, &lower))
            .map(|text| Hit {
                agent: s.agent,
                id: s.id.clone(),
                text,
            })
    });
    found.into_iter().flatten().take(limit).collect()
}

/// A few words of `text` around where it says `lower`.
fn snippet(text: &str, lower: &str) -> Option<String> {
    let low = text.to_lowercase();
    let at = low.find(lower)?;
    // `to_lowercase` can change a text's length: count in characters.
    let before = low[..at].chars().count();
    let chars: Vec<char> = text.chars().collect();
    let from = before.saturating_sub(40);
    let to = (before + lower.chars().count() + 80).min(chars.len());
    let mut out: String = chars[from..to].iter().collect();
    out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if from > 0 {
        out.insert(0, '…');
    }
    if to < chars.len() {
        out.push('…');
    }
    Some(out)
}

// --- Reading files --------------------------------------------------------------

/// What one file of a session says.
#[derive(Debug, Clone, Default)]
struct Read {
    id: String,
    cwd: String,
    /// The best title the file gave, and how good: 3 a name the user gave
    /// it, 2 its agent's title, 1 a summary.
    title: String,
    rank: u8,
    /// Its first prompt.
    prompt: String,
    start: Option<DateTime<Utc>>,
    last: Option<DateTime<Utc>>,
    models: Vec<String>,
    messages: u32,
    tokens: Tokens,
    /// Codex: a thread another started, and that one's id when it says.
    subagent: bool,
    parent: String,
}

impl Read {
    fn saw(&mut self, at: Option<DateTime<Utc>>) {
        let Some(at) = at else { return };
        if self.start.is_none_or(|s| at < s) {
            self.start = Some(at);
        }
        if self.last.is_none_or(|l| at > l) {
            self.last = Some(at);
        }
    }

    fn titled(&mut self, rank: u8, text: &str) {
        let text = one_line(text, 200);
        if !text.is_empty() && rank >= self.rank {
            self.title = text;
            self.rank = rank;
        }
    }

    fn prompted(&mut self, text: &str) {
        if self.prompt.is_empty() {
            self.prompt = one_line(&untagged(text), 200);
        }
    }

    fn model(&mut self, model: &str) {
        if !model.is_empty() && !self.models.iter().any(|m| m == model) {
            self.models.push(model.to_owned());
        }
    }

    /// Its title: the best it gave, else its first prompt.
    fn title(&self) -> String {
        if self.title.is_empty() {
            self.prompt.clone()
        } else {
            self.title.clone()
        }
    }
}

/// A file as it was when read: its size and time.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Stamp {
    len: u64,
    modified: Option<SystemTime>,
}

fn stamp(path: &Path) -> Option<Stamp> {
    let meta = fs::metadata(path).ok()?;
    Some(Stamp {
        len: meta.len(),
        modified: meta.modified().ok(),
    })
}

/// Each file read: as it was, and what it said.
type Kept = HashMap<PathBuf, (Stamp, Option<Read>)>;

static READS: LazyLock<Mutex<Kept>> = LazyLock::new(Default::default);

/// What the file or row `key` said, if it was read as it is now.
fn kept(key: &Path, stamp: Stamp) -> Option<Option<Read>> {
    let kept = READS.lock().unwrap_or_else(|e| e.into_inner());
    kept.get(key)
        .filter(|(then, _)| *then == stamp)
        .map(|(_, read)| read.clone())
}

/// Keeps what the file or row `key` said, as it was.
fn keep(key: PathBuf, stamp: Stamp, read: Option<Read>) {
    READS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key, (stamp, read));
}

/// What each of `files` says, from memory when it is as it was when read,
/// else read with `read`, several at once. Each is noted in `seen`.
fn reads(
    files: &[PathBuf],
    read: fn(&Path) -> Option<Read>,
    seen: &mut HashSet<PathBuf>,
) -> Vec<Option<Read>> {
    let stamps: Vec<Option<Stamp>> = files.iter().map(|f| stamp(f)).collect();
    let mut out = vec![None; files.len()];
    let mut todo = Vec::new();
    {
        let kept = READS.lock().unwrap_or_else(|e| e.into_inner());
        for (i, file) in files.iter().enumerate() {
            seen.insert(file.clone());
            match (stamps[i], kept.get(file)) {
                (Some(now), Some((then, read))) if now == *then => out[i] = read.clone(),
                (Some(_), _) => todo.push(i),
                (None, _) => {}
            }
        }
    }
    let done = parallel(&todo, |&i| read(&files[i]));
    let mut kept = READS.lock().unwrap_or_else(|e| e.into_inner());
    for (&i, read) in todo.iter().zip(done) {
        if let Some(stamp) = stamps[i] {
            kept.insert(files[i].clone(), (stamp, read.clone()));
        }
        out[i] = read;
    }
    out
}

/// `work` on each of `items` on a few threads, the results in order.
fn parallel<T: Sync, R: Send>(items: &[T], work: impl Fn(&T) -> R + Sync) -> Vec<R> {
    if items.len() < 2 {
        return items.iter().map(&work).collect();
    }
    let threads = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .clamp(1, 8)
        .min(items.len());
    let next = AtomicUsize::new(0);
    let mut done: Vec<(usize, R)> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut out = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(i) else { break };
                        out.push((i, work(item)));
                    }
                    out
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().unwrap_or_default())
            .collect()
    });
    done.sort_by_key(|(i, _)| *i);
    done.into_iter().map(|(_, r)| r).collect()
}

/// Each line of the file at `path`, as bytes; a file that can't be opened has none.
fn lines(path: &Path, mut each: impl FnMut(&[u8])) -> Result<()> {
    use std::io::BufRead;
    let file = fs::File::open(path)?;
    let mut reader = std::io::BufReader::with_capacity(1 << 16, file);
    let mut line = Vec::with_capacity(1 << 12);
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        if line.len() > 1 {
            each(&line);
        }
    }
}

/// Whether a line of the file at `path` has `needle` (ASCII lowercase) in
/// it, ASCII case aside: read a line at a time, up to the first that has.
fn any_line_has(path: &Path, needle: &[u8]) -> bool {
    use std::io::BufRead;
    let Some(&first) = needle.first() else {
        return true;
    };
    let Ok(file) = fs::File::open(path) else {
        return false;
    };
    let mut reader = std::io::BufReader::with_capacity(1 << 16, file);
    let mut line = Vec::with_capacity(1 << 12);
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => return false,
            Ok(_) => {}
        }
        let upper = first.to_ascii_uppercase();
        let found = line
            .iter()
            .enumerate()
            .filter(|&(_, &b)| b == first || b == upper)
            .any(|(at, _)| {
                line.get(at..at + needle.len())
                    .is_some_and(|w| w.eq_ignore_ascii_case(needle))
            });
        if found {
            return true;
        }
    }
}

/// An RFC 3339 time.
fn time(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// A JSON value that is a string, else nothing: for keys whose values vary
/// from line to line.
fn string(raw: Option<&RawValue>) -> String {
    raw.filter(|r| r.get().starts_with('"'))
        .and_then(|r| serde_json::from_str(r.get()).ok())
        .unwrap_or_default()
}

/// `text` on one line, `max` characters at most.
fn one_line(text: &str, max: usize) -> String {
    let mut out = String::new();
    for word in text.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
        if out.len() > max * 4 {
            break;
        }
    }
    cut(&out, max)
}

/// `text` without the tags an app wrapped it in: Claude Desktop's
/// `<pasted_content id="…">` around what was pasted.
fn untagged(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('<') {
        out.push_str(&rest[..at]);
        let tag = &rest[at..];
        let opens = tag[1..].starts_with(|c: char| c.is_ascii_alphabetic() || c == '/');
        match tag.find('>') {
            Some(end) if opens && end <= 120 && !tag[1..end].contains(['<', '\n']) => {
                out.push(' ');
                rest = &tag[end + 1..];
            }
            _ => {
                out.push('<');
                rest = &tag[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// `text`, `max` characters at most, an ellipsis ending one that was longer.
fn cut(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", text[..at].trim_end()),
        None => text.to_owned(),
    }
}

/// What a tool was given, in a line: its command, file, pattern or address
/// when it has one, else its input as JSON.
fn tool_input(input: &Value) -> String {
    const KEYS: &[&str] = &[
        "command",
        "cmd",
        "file_path",
        "path",
        "pattern",
        "url",
        "query",
        "description",
        "prompt",
    ];
    for key in KEYS {
        match input.get(key) {
            Some(Value::String(s)) if !s.trim().is_empty() => return one_line(s, 300),
            Some(Value::Array(parts)) if !parts.is_empty() => {
                let words: Vec<&str> = parts.iter().filter_map(Value::as_str).collect();
                // `["bash", "-lc", "ls"]` is `ls`.
                let words = match words.as_slice() {
                    [shell, "-lc" | "-c", rest @ ..] if shell.ends_with("sh") => rest.to_vec(),
                    _ => words,
                };
                return one_line(&words.join(" "), 300);
            }
            _ => {}
        }
    }
    match input {
        Value::Null => String::new(),
        Value::String(s) => one_line(s, 300),
        other => cut(&other.to_string(), 300),
    }
}

/// What a tool was given, whole: a string as it is, else JSON laid out;
/// `max` characters at most.
fn tool_args(input: &Value, max: usize) -> String {
    let text = match input {
        Value::Null => return String::new(),
        Value::String(s) => s.trim().to_owned(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    };
    cut(&text, max)
}

/// What a tool gave back, as text: a string as it is, the texts of a list
/// of blocks (`{"type": "text", "text": …}`, an image as `[image]`), else
/// its JSON; `max` characters at most.
fn tool_result(output: &Value, max: usize) -> String {
    let text = match output {
        Value::Null => String::new(),
        Value::String(s) => s.trim().to_owned(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| match b.get("type").and_then(Value::as_str) {
                Some("text" | "input_text" | "output_text") => {
                    b.get("text").and_then(Value::as_str).map(str::trim)
                }
                Some("image" | "input_image") => Some("[image]"),
                _ => b.as_str(),
            })
            .collect::<Vec<_>>()
            .join("\n\n"),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    };
    cut(&text, max)
}

/// An id that can go on a command line and into a path as it is.
fn plain_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The shell line that picks session `id` of `agent` up again in `cwd`.
fn resume(agent: &str, id: &str, cwd: &str) -> String {
    if !plain_id(id) {
        return String::new();
    }
    let run = match agent {
        "claude-code" => format!("claude --resume {id}"),
        "codex" => format!("codex resume {id}"),
        "opencode" => format!("opencode --session {id}"),
        _ => return String::new(),
    };
    if cwd.is_empty() {
        run
    } else if cfg!(windows) {
        format!(
            "Set-Location -LiteralPath '{}'; {run}",
            cwd.replace('\'', "''")
        )
    } else {
        format!("cd '{}' && {run}", cwd.replace('\'', r"'\''"))
    }
}

/// The bytes of a file, or of a folder and everything in it.
fn size_of(path: &Path) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return 0;
    };
    if !meta.is_dir() {
        return meta.len();
    }
    fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| size_of(&e.path()))
        .sum()
}

/// Whether a file was written in the last minute.
fn recent(path: &Path) -> bool {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .is_ok_and(|t| t.elapsed().map_or(true, |age| age < ACTIVE))
}
