//! OpenCode's sessions. Since 1.2 they are rows in its SQLite database,
//! `opencode.db`: `session`, `message` and `part`, each message and part a
//! JSON document. OpenCode 2 keeps them in tables of its own in the same
//! file, `session_v2` and `session_message` (a message's kind, its order
//! and a JSON document: a prompt's text, a reply's model, content and
//! tokens), and copies the old ones over on its first start, noting
//! `migration.v1-v2` in `kv` when it is done; until then, an old session
//! not copied yet is read from the old tables. Before 1.2 they were JSON
//! files under `storage/`: `session/<project>/<id>.json`,
//! `message/<session>/<id>.json` and `part/<message>/<id>.json`. The
//! database is made from those once and they stay behind, so where there is
//! a database only it is read.
//!
//! A reply carries its model and its tokens, input without the cache and
//! output without the reasoning. A subagent's work is a session of its own
//! whose parent is the session it ran in, listed under that one.
//!
//! The database is opened read only. OpenCode keeps it open and writes it,
//! so its sessions are listed, never deleted.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use pansy_core::paths::user_home;
use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;
use serde_json::Value;

use super::{
    ACTIVE, Keep, Read, Session, Stamp, Subagent, Tokens, Turn, cut, keep, kept, resume, tool_args,
    tool_input, tool_result,
};

/// OpenCode's data folder: `$XDG_DATA_HOME/opencode`, else
/// `~/.local/share/opencode` (on Windows too).
fn data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| user_home().join(".local").join("share"))
        .join("opencode")
}

/// Its database: `$OPENCODE_DB` (in the data folder unless absolute), else
/// `opencode.db` there.
fn db_path() -> PathBuf {
    match std::env::var("OPENCODE_DB") {
        Ok(p) if !p.is_empty() && p != ":memory:" => {
            let p = PathBuf::from(p);
            if p.is_absolute() {
                p
            } else {
                data_dir().join(p)
            }
        }
        _ => data_dir().join("opencode.db"),
    }
}

fn open() -> Option<Connection> {
    let path = db_path();
    if !path.is_file() {
        return None;
    }
    let db = Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    let _ = db.busy_timeout(Duration::from_secs(3));
    Some(db)
}

/// Where a session is kept.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    /// OpenCode 1.2's tables.
    V1,
    /// OpenCode 2's.
    V2,
    /// The JSON files from before the database.
    Files,
}

/// A session as listed, before its messages are read.
#[derive(Debug, Clone)]
struct Row {
    kind: Kind,
    id: String,
    parent: String,
    cwd: String,
    title: String,
    created: i64,
    /// Its or its messages' latest change (ms).
    updated: i64,
    /// Its messages' bytes.
    size: u64,
    /// Its file: the database, or the session's JSON file.
    path: PathBuf,
}

impl Row {
    /// What it is kept in memory by, and as.
    fn key(&self) -> PathBuf {
        let tag = match self.kind {
            Kind::V1 | Kind::Files => "",
            Kind::V2 => "v2/",
        };
        PathBuf::from(format!("{}#{tag}{}", self.path.display(), self.id))
    }

    fn stamp(&self) -> Stamp {
        Stamp {
            len: self.size,
            modified: Some(
                SystemTime::UNIX_EPOCH + Duration::from_millis(self.updated.max(0) as u64),
            ),
        }
    }
}

fn has_table(db: &Connection, name: &str) -> bool {
    db.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [name],
        |r| r.get::<_, i64>(0),
    )
    .is_ok_and(|n| n > 0)
}

/// Whether OpenCode 2 has copied every old session into its tables.
fn migrated(db: &Connection) -> bool {
    #[derive(Deserialize)]
    struct Phase {
        phase: String,
    }
    db.query_row(
        "SELECT value FROM kv WHERE key = 'migration.v1-v2'",
        [],
        |r| r.get::<_, String>(0),
    )
    .ok()
    .and_then(|v| serde_json::from_str::<Phase>(&v).ok())
    .is_some_and(|p| p.phase == "completed")
}

fn db_rows(db: &Connection) -> Vec<Row> {
    let path = db_path();
    let query = |kind: Kind, session: &str, message: &str, only: &str| -> Vec<Row> {
        let sql = format!(
            "SELECT s.id, COALESCE(s.parent_id, ''), COALESCE(s.directory, ''), COALESCE(s.title, ''),
                    COALESCE(s.time_created, 0), COALESCE(s.time_updated, 0),
                    COALESCE(MAX(m.time_updated), 0), COALESCE(SUM(LENGTH(m.data)), 0)
             FROM {session} s LEFT JOIN {message} m ON m.session_id = s.id {only} GROUP BY s.id"
        );
        let Ok(mut stmt) = db.prepare(&sql) else {
            return Vec::new();
        };
        let rows = stmt.query_map([], |r| {
            Ok(Row {
                kind,
                id: r.get(0)?,
                parent: r.get(1)?,
                cwd: r.get(2)?,
                title: r.get(3)?,
                created: r.get(4)?,
                updated: r.get::<_, i64>(5)?.max(r.get(6)?),
                size: r.get::<_, i64>(7)?.max(0) as u64,
                path: path.clone(),
            })
        });
        rows.map(|rows| rows.flatten().collect())
            .unwrap_or_default()
    };
    let mut out = Vec::new();
    let mut old = has_table(db, "session").then_some("");
    if has_table(db, "session_v2") {
        out = query(Kind::V2, "session_v2", "session_message", "");
        if old.is_some() {
            old = (!migrated(db)).then_some("WHERE s.id NOT IN (SELECT id FROM session_v2)");
        }
    }
    if let Some(only) = old {
        out.extend(query(Kind::V1, "session", "message", only));
    }
    out
}

/// A session's JSON file.
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Info {
    id: String,
    #[serde(rename = "parentID")]
    parent_id: String,
    directory: String,
    title: String,
    time: Times,
}

#[derive(Deserialize, Default, Clone, Copy)]
#[serde(default)]
struct Times {
    created: Option<i64>,
    updated: Option<i64>,
    completed: Option<i64>,
}

/// The JSON files of `storage/<dir>`, by name.
fn docs(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    out.sort();
    out
}

fn file_rows(storage: &Path) -> Vec<Row> {
    let mut out = Vec::new();
    for project in fs::read_dir(storage.join("session"))
        .into_iter()
        .flatten()
        .flatten()
    {
        for path in docs(&project.path()) {
            let Some(info) = fs::read(&path)
                .ok()
                .and_then(|b| serde_json::from_slice::<Info>(&b).ok())
                .filter(|i| !i.id.is_empty())
            else {
                continue;
            };
            let mut updated = info.time.updated.unwrap_or(0);
            let mut size = 0;
            for m in docs(&storage.join("message").join(&info.id)) {
                if let Ok(meta) = fs::metadata(&m) {
                    size += meta.len();
                    if let Some(ms) = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                    {
                        updated = updated.max(ms.as_millis() as i64);
                    }
                }
            }
            out.push(Row {
                kind: Kind::Files,
                id: info.id,
                parent: info.parent_id,
                cwd: info.directory,
                title: info.title,
                created: info.time.created.unwrap_or(0),
                updated,
                size,
                path,
            });
        }
    }
    out
}

/// The tokens a reply spent, as OpenCode counts them.
#[derive(Deserialize, Default, Clone, Copy)]
#[serde(default)]
struct Spent {
    input: Option<u64>,
    output: Option<u64>,
    reasoning: Option<u64>,
    cache: Option<Cache>,
}

#[derive(Deserialize, Default, Clone, Copy)]
#[serde(default)]
struct Cache {
    read: Option<u64>,
    write: Option<u64>,
}

impl Spent {
    fn tokens(&self) -> Tokens {
        let reasoning = self.reasoning.unwrap_or(0);
        let cache = self.cache.unwrap_or_default();
        Tokens {
            input: self.input.unwrap_or(0),
            output: self.output.unwrap_or(0) + reasoning,
            cache_read: cache.read.unwrap_or(0),
            cache_write: cache.write.unwrap_or(0),
            reasoning,
        }
    }
}

/// A message: OpenCode 1.2's document (and the JSON files'), or OpenCode
/// 2's, which keeps its kind apart and a prompt's text in it.
#[derive(Deserialize, Default)]
#[serde(default)]
struct Doc {
    id: String,
    role: String,
    #[serde(rename = "modelID")]
    model_id: String,
    tokens: Option<Spent>,
    time: Times,
    path: Option<Where>,
    // OpenCode 2
    model: Option<Model>,
    text: String,
    content: Vec<Value>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Where {
    cwd: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Model {
    id: String,
}

impl Doc {
    fn model(&self) -> &str {
        match &self.model {
            Some(m) if !m.id.is_empty() => &m.id,
            _ => &self.model_id,
        }
    }
}

/// A part of a 1.2 message: its text, a tool called, where it was compacted.
#[derive(Deserialize, Default)]
#[serde(default)]
struct Part {
    #[serde(rename = "type")]
    kind: String,
    text: String,
    synthetic: bool,
    ignored: bool,
    tool: String,
    state: Option<State>,
    description: String,
    agent: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct State {
    input: Value,
    /// `pending`, `running`, `completed` or `error`.
    status: String,
    output: Value,
    error: String,
}

impl State {
    /// The turn of a call of `tool` in this state.
    fn turn(self, tool: String, at: Option<DateTime<Utc>>, keep: Keep) -> Turn {
        let (output, status) = match self.status.as_str() {
            "completed" => (tool_result(&self.output, keep.tool), "done"),
            "error" => (cut(self.error.trim(), keep.tool), "error"),
            _ => (String::new(), ""),
        };
        Turn {
            role: "tool",
            name: tool,
            text: tool_input(&self.input),
            input: tool_args(&self.input, keep.tool),
            output,
            status,
            at,
        }
    }
}

/// A session's messages in order.
fn messages(db: Option<&Connection>, row: &Row) -> Vec<Doc> {
    let parse = |id: String, kind: Option<String>, data: &str| {
        serde_json::from_str::<Doc>(data).ok().map(|mut d| {
            if d.id.is_empty() {
                d.id = id;
            }
            if let Some(kind) = kind {
                d.role = kind;
            }
            d
        })
    };
    let mut out: Vec<Doc> = match (row.kind, db) {
        (Kind::V2, Some(db)) => rows(
            db,
            "SELECT id, type, data FROM session_message
             WHERE session_id = ?1 AND type IN ('user', 'assistant', 'compaction') ORDER BY seq",
            &row.id,
            |r| Ok((r.get(0)?, Some(r.get(1)?), r.get::<_, String>(2)?)),
        ),
        (Kind::V1, Some(db)) => rows(
            db,
            "SELECT id, NULL, data FROM message WHERE session_id = ?1",
            &row.id,
            |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, String>(2)?)),
        ),
        (Kind::Files, _) => docs(&storage().join("message").join(&row.id))
            .into_iter()
            .filter_map(|p| Some((String::new(), None, fs::read_to_string(p).ok()?)))
            .collect(),
        _ => Vec::new(),
    }
    .into_iter()
    .filter_map(|(id, kind, data)| parse(id, kind, &data))
    .collect();
    if row.kind != Kind::V2 {
        out.sort_by(|a, b| {
            (a.time.created.unwrap_or(0), &a.id).cmp(&(b.time.created.unwrap_or(0), &b.id))
        });
    }
    out
}

fn storage() -> PathBuf {
    data_dir().join("storage")
}

/// The rows a query of one session's things gives.
fn rows<T>(
    db: &Connection,
    sql: &str,
    id: &str,
    each: impl FnMut(&rusqlite::Row) -> rusqlite::Result<T>,
) -> Vec<T> {
    let Ok(mut stmt) = db.prepare(sql) else {
        return Vec::new();
    };
    stmt.query_map([id], each)
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
}

/// A 1.2 message's parts, in order.
fn parts(db: Option<&Connection>, row: &Row, message: &str) -> Vec<Part> {
    let texts: Vec<String> = match (row.kind, db) {
        (Kind::V1, Some(db)) => rows(
            db,
            "SELECT data FROM part WHERE message_id = ?1 ORDER BY id",
            message,
            |r| r.get(0),
        ),
        (Kind::Files, _) => docs(&storage().join("part").join(message))
            .into_iter()
            .filter_map(|p| fs::read_to_string(p).ok())
            .collect(),
        _ => Vec::new(),
    };
    texts
        .iter()
        .filter_map(|t| serde_json::from_str(t).ok())
        .collect()
}

/// What the user typed in a prompt: its text parts, not what OpenCode put
/// in beside them (a file read, a command's template) unless that is all.
fn prompt(db: Option<&Connection>, row: &Row, m: &Doc) -> String {
    if row.kind == Kind::V2 {
        return m.text.trim().to_owned();
    }
    let parts = parts(db, row, &m.id);
    let said = |synthetic: bool| {
        parts
            .iter()
            .filter(|p| p.kind == "text" && !p.ignored && p.synthetic == synthetic)
            .map(|p| p.text.trim())
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    };
    let typed = said(false);
    if typed.is_empty() { said(true) } else { typed }
}

/// The title OpenCode gives a session before it names it.
fn untitled(title: &str) -> bool {
    title.starts_with("New session - ") || title.starts_with("Child session - ")
}

fn ms(n: i64) -> Option<DateTime<Utc>> {
    (n > 0)
        .then(|| DateTime::from_timestamp_millis(n))
        .flatten()
}

fn read(db: Option<&Connection>, row: &Row) -> Read {
    let mut r = Read {
        id: row.id.clone(),
        cwd: row.cwd.clone(),
        ..Read::default()
    };
    if !untitled(&row.title) {
        r.titled(2, &row.title);
    }
    r.saw(ms(row.created));
    for m in messages(db, row) {
        r.saw(m.time.created.and_then(ms));
        r.saw(m.time.completed.and_then(ms));
        match m.role.as_str() {
            "user" => {
                r.messages += 1;
                if r.prompt.is_empty() {
                    r.prompted(&prompt(db, row, &m));
                }
            }
            "assistant" => {
                r.messages += 1;
                r.model(m.model());
                if r.cwd.is_empty()
                    && let Some(w) = &m.path
                {
                    r.cwd.clone_from(&w.cwd);
                }
            }
            _ => {}
        }
        if let Some(t) = m.tokens {
            r.tokens.add(&t.tokens());
        }
    }
    r
}

/// OpenCode titles a subagent's session `<what it was asked> (@<agent> subagent)`.
fn subagent_title(title: &str) -> (String, String) {
    if let Some(rest) = title.strip_suffix(" subagent)")
        && let Some((what, kind)) = rest.rsplit_once(" (@")
    {
        return (what.to_owned(), kind.to_owned());
    }
    (title.to_owned(), String::new())
}

/// Every session's row, and the database they are in when there is one.
fn listed() -> (Option<Connection>, Vec<Row>) {
    let db = open();
    let rows = match &db {
        Some(db) => db_rows(db),
        None => file_rows(&storage()),
    };
    (db, rows)
}

pub(super) fn list(seen: &mut HashSet<PathBuf>) -> Vec<Session> {
    let (db, rows) = listed();
    let reads: Vec<Read> = rows
        .iter()
        .map(|row| {
            let (key, stamp) = (row.key(), row.stamp());
            seen.insert(key.clone());
            if let Some(Some(r)) = kept(&key, stamp) {
                return r;
            }
            let r = read(db.as_ref(), row);
            keep(key, stamp, Some(r.clone()));
            r
        })
        .collect();
    let by_id: HashMap<&str, usize> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| (r.id.as_str(), i))
        .collect();
    // the session each is under, following parents
    let root_of = |mut at: usize| -> usize {
        for _ in 0..16 {
            match by_id.get(rows[at].parent.as_str()) {
                Some(&up) if up != at => at = up,
                _ => break,
            }
        }
        at
    };
    let mut under: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..rows.len() {
        let root = root_of(i);
        if root != i {
            under.entry(root).or_default().push(i);
        }
    }
    let now = Utc::now().timestamp_millis();
    let recent = |row: &Row| now - row.updated < ACTIVE.as_millis() as i64;
    let mut out = Vec::new();
    for (i, (row, r)) in rows.iter().zip(&reads).enumerate() {
        if root_of(i) != i || r.messages == 0 {
            continue;
        }
        let kids = under.get(&i).map(Vec::as_slice).unwrap_or_default();
        let mut tokens = r.tokens;
        let mut size = row.size;
        let mut subagents = Vec::new();
        for &k in kids {
            let kid = &reads[k];
            tokens.add(&kid.tokens);
            size += rows[k].size;
            let (title, kind) = subagent_title(&rows[k].title);
            subagents.push(Subagent {
                id: rows[k].id.clone(),
                title: if untitled(&title) { kid.title() } else { title },
                kind,
                last: kid.last,
                messages: kid.messages,
                tokens: kid.tokens,
            });
        }
        subagents.sort_by_key(|a| a.last);
        out.push(Session {
            agent: "opencode",
            id: row.id.clone(),
            cwd: r.cwd.clone(),
            title: r.title(),
            start: r.start,
            last: r.last,
            models: r.models.clone(),
            messages: r.messages,
            tokens,
            size,
            path: row.path.display().to_string(),
            resume: resume("opencode", &row.id, &r.cwd),
            active: recent(row) || kids.iter().any(|&k| recent(&rows[k])),
            deletable: false,
            subagents,
            files: Vec::new(),
        });
    }
    out
}

/// What was said in the session `id`.
pub(super) fn transcript(id: &str, keep: Keep) -> Result<Vec<Turn>> {
    let (db, rows) = listed();
    let row = rows
        .iter()
        .find(|r| r.id == id)
        .ok_or_else(|| anyhow!("no OpenCode session {id}"))?;
    let db = db.as_ref();
    let mut turns: Vec<Turn> = Vec::new();
    let mut say = |turn: Turn| {
        // a reply's texts, one after another, are one turn
        if turn.role == "assistant"
            && let Some(last) = turns.last_mut()
            && last.role == "assistant"
            && last.at == turn.at
        {
            last.text = cut(&format!("{}\n\n{}", last.text, turn.text), keep.text);
            return;
        }
        turns.push(turn);
    };
    let said = |role: &'static str, text: &str, at| Turn {
        role,
        text: cut(text, keep.text),
        at,
        ..Turn::default()
    };
    for m in messages(db, row) {
        let at = m.time.created.and_then(ms);
        if m.role == "compaction" {
            say(said("compact", "", at));
            continue;
        }
        let role = if m.role == "user" {
            "user"
        } else {
            "assistant"
        };
        if row.kind == Kind::V2 {
            if role == "user" {
                if !m.text.trim().is_empty() {
                    say(said("user", m.text.trim(), at));
                }
                continue;
            }
            for c in &m.content {
                let text = || c.get("text").and_then(Value::as_str).unwrap_or("").trim();
                match c.get("type").and_then(Value::as_str) {
                    Some("text") if !text().is_empty() => say(said("assistant", text(), at)),
                    Some("reasoning") if !text().is_empty() => say(said("reasoning", text(), at)),
                    Some("tool") => {
                        let name = c.get("name").and_then(Value::as_str).unwrap_or("tool");
                        let state = c
                            .get("state")
                            .and_then(|s| State::deserialize(s).ok())
                            .unwrap_or_default();
                        say(state.turn(name.to_owned(), at, keep));
                    }
                    _ => {}
                }
            }
            continue;
        }
        for p in parts(db, row, &m.id) {
            match p.kind.as_str() {
                "text" if !p.synthetic && !p.ignored && !p.text.trim().is_empty() => {
                    say(said(role, p.text.trim(), at));
                }
                "reasoning" if !p.text.trim().is_empty() => {
                    say(said("reasoning", p.text.trim(), at));
                }
                "tool" => say(p.state.unwrap_or_default().turn(p.tool, at, keep)),
                "subtask" => {
                    let name = if p.agent.is_empty() {
                        "task".to_owned()
                    } else {
                        p.agent
                    };
                    say(Turn {
                        role: "tool",
                        name,
                        text: super::one_line(&p.description, 300),
                        at,
                        ..Turn::default()
                    });
                }
                "compaction" => say(said("compact", "", at)),
                _ => {}
            }
        }
    }
    Ok(turns)
}

/// The sessions whose stored words have `needle` in them, ASCII case
/// aside; none for the JSON files, which are each looked through instead.
pub(super) fn matching(needle: &str) -> Option<HashSet<String>> {
    let db = open()?;
    let like = format!(
        "%{}%",
        needle
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    let mut out = HashSet::new();
    for (table, present) in [
        ("part", has_table(&db, "part")),
        ("session_message", has_table(&db, "session_message")),
    ] {
        if !present {
            continue;
        }
        let sql = format!("SELECT DISTINCT session_id FROM {table} WHERE data LIKE ?1 ESCAPE '\\'");
        out.extend(rows(&db, &sql, &like, |r| r.get::<_, String>(0)));
    }
    Some(out)
}
