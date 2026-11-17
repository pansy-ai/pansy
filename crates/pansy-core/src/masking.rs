//! What masking kept from vendors: one JSON line per masked request in a file
//! per day (`masking/2026-11-13.jsonl`), kept 30 days, for the Masking page.
//! A record names each value's kind, its first characters and its length,
//! never the value; what the request said around it, before and after, stays
//! in memory for the latest requests only.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use chrono::{DateTime, Duration, Local, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::{fsutil, paths};

/// How long records are kept.
pub const KEEP_DAYS: u32 = 30;

/// How many of the latest requests keep their text in memory.
const DETAILS: usize = 100;

/// One masked value, as often as the request had it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Value {
    /// Its placeholder (`{{EMAIL_k3j9x2ab}}`).
    pub p: String,
    /// Its kind (`EMAIL`, `OPENAI_KEY`).
    pub k: String,
    /// What found it: `rule`, `yours` (a masking rule of the user's), `word`
    /// (a masked word) or `known` (masked before, in another request).
    pub b: String,
    /// Its first characters, for a secret's prefix (`sk-pr`); empty for
    /// personal data.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub h: String,
    /// Its length.
    pub l: usize,
    /// The places the request had it.
    pub n: usize,
    /// `warn` (sent as written) or `block` (the request was held back);
    /// absent for masked.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub a: String,
    /// Where in the request it was (`messages[0].content`), the first few
    /// places.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub w: Vec<String>,
}

/// One request's masking.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Record {
    /// The record's own id: unique in this install.
    pub key: u64,
    pub t: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub agent: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub provider: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    /// The places masked in all.
    pub n: usize,
    /// The request's size in bytes, as the agent sent it.
    #[serde(default)]
    pub size: usize,
    pub values: Vec<Value>,
    /// Held back: a value set to block was found.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub blocked: bool,
    /// Microseconds masking took.
    #[serde(default)]
    pub took: u64,
    /// In memory only: by placeholder, the places the answer had it put back.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub back: BTreeMap<String, usize>,
}

/// A text masking changed in a request.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Change {
    /// Where in the request (`messages[2].content[0].text`).
    pub path: String,
    pub before: String,
    pub after: String,
}

/// The days' masking, for the Masking page.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub days: u32,
    /// Places masked.
    pub values: usize,
    /// Different values masked.
    pub distinct: usize,
    /// Requests that had any.
    pub requests: usize,
    /// `YYYY-MM-DD` → places masked, every day of the range present.
    pub by_day: BTreeMap<String, usize>,
    pub by_agent: BTreeMap<String, usize>,
    pub by_provider: BTreeMap<String, usize>,
    pub by_kind: BTreeMap<String, usize>,
    /// The requests, newest first.
    pub records: Vec<Record>,
}

struct Detail {
    key: u64,
    changes: Vec<Change>,
    back: BTreeMap<String, usize>,
}

static DETAIL: Mutex<VecDeque<Detail>> = Mutex::new(VecDeque::new());
/// The kinds masked before, and those masked for the first time since the
/// window last asked, each with its places.
type Firsts = (BTreeSet<String>, Vec<(String, usize)>);

/// None until the records are first read.
static FIRSTS: Mutex<Option<Firsts>> = Mutex::new(None);
static LAST_KEY: AtomicU64 = AtomicU64::new(0);
static PRUNED: Mutex<Option<NaiveDate>> = Mutex::new(None);

fn dir() -> PathBuf {
    paths::home().join("masking")
}

fn file_for(day: NaiveDate) -> PathBuf {
    dir().join(format!("{}.jsonl", day.format("%Y-%m-%d")))
}

/// A new record's key: the time in microseconds, made unique.
pub fn next_key() -> u64 {
    let now = Utc::now().timestamp_micros().max(0) as u64;
    let mut last = LAST_KEY.load(Ordering::Relaxed);
    loop {
        let key = now.max(last + 1);
        match LAST_KEY.compare_exchange(last, key, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return key,
            Err(seen) => last = seen,
        }
    }
}

/// Writes `record` to its day's file and keeps `changes` in memory.
pub fn append(record: &Record, changes: Vec<Change>) -> Result<()> {
    {
        let mut detail = DETAIL.lock().unwrap();
        detail.push_back(Detail {
            key: record.key,
            changes,
            back: BTreeMap::new(),
        });
        while detail.len() > DETAILS {
            detail.pop_front();
        }
    }
    note_firsts(record);
    let today = record.t.with_timezone(&Local).date_naive();
    prune(today);
    let path = file_for(today);
    fs::create_dir_all(path.parent().unwrap())?;
    let mut on_disk = record.clone();
    on_disk.back.clear();
    let mut line = serde_json::to_vec(&on_disk)?;
    line.push(b'\n');
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    file.write_all(&line)?;
    Ok(())
}

/// Notes the kinds `record` has that were never masked before.
fn note_firsts(record: &Record) {
    let mut firsts = FIRSTS.lock().unwrap();
    let (seen, new) = firsts.get_or_insert_with(|| {
        let seen = recent(KEEP_DAYS)
            .iter()
            .flat_map(|r| r.values.iter().map(|v| v.k.clone()))
            .collect();
        (seen, Vec::new())
    });
    for v in &record.values {
        if !seen.contains(&v.k) {
            let n = record
                .values
                .iter()
                .filter(|x| x.k == v.k)
                .map(|x| x.n)
                .sum();
            seen.insert(v.k.clone());
            new.push((v.k.clone(), n));
        }
    }
}

/// The kinds masked for the first time since this was last called, each
/// with how many places the request had (for Settings › Privacy's masking
/// alert: never the values).
pub fn take_firsts() -> Vec<(String, usize)> {
    FIRSTS
        .lock()
        .unwrap()
        .as_mut()
        .map(|(_, new)| std::mem::take(new))
        .unwrap_or_default()
}

/// Notes what the answer to the request `key` had put back.
pub fn put_back(key: u64, back: &BTreeMap<String, usize>) {
    if back.is_empty() {
        return;
    }
    let mut detail = DETAIL.lock().unwrap();
    if let Some(d) = detail.iter_mut().find(|d| d.key == key) {
        for (placeholder, n) in back {
            *d.back.entry(placeholder.clone()).or_default() += n;
        }
    }
}

/// What the request `key` said where masking changed it, if it is among the
/// latest.
pub fn changes(key: u64) -> Option<Vec<Change>> {
    let detail = DETAIL.lock().unwrap();
    detail
        .iter()
        .find(|d| d.key == key)
        .map(|d| d.changes.clone())
}

/// Removes every record, on disk and in memory.
pub fn clear() -> Result<()> {
    DETAIL.lock().unwrap().clear();
    *FIRSTS.lock().unwrap() = None;
    match fs::remove_dir_all(dir()) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

/// Removes the days older than [`KEEP_DAYS`], once a day.
fn prune(today: NaiveDate) {
    let mut pruned = PRUNED.lock().unwrap();
    if *pruned == Some(today) {
        return;
    }
    *pruned = Some(today);
    let oldest = today - Duration::days(KEEP_DAYS as i64 - 1);
    let Ok(entries) = fs::read_dir(dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(day) = name
            .to_str()
            .and_then(|n| n.strip_suffix(".jsonl"))
            .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        else {
            continue;
        };
        if day < oldest {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// The records of the last `days` days (today included), oldest first, with
/// what their answers had put back while they are in memory.
pub fn recent(days: u32) -> Vec<Record> {
    let today = Local::now().date_naive();
    let mut out = Vec::new();
    for back in (0..days.clamp(1, KEEP_DAYS) as i64).rev() {
        let Ok(file) = fs::File::open(file_for(today - Duration::days(back))) else {
            continue;
        };
        for line in BufReader::new(file).lines().map_while(|l| l.ok()) {
            if let Ok(record) = serde_json::from_str::<Record>(&line) {
                out.push(record);
            }
        }
    }
    let detail = DETAIL.lock().unwrap();
    for record in &mut out {
        if let Some(d) = detail.iter().find(|d| d.key == record.key) {
            record.back = d.back.clone();
        }
    }
    out
}

pub fn summary(days: u32) -> Summary {
    summarize(recent(days), days, Local::now().date_naive())
}

pub fn summarize(mut records: Vec<Record>, days: u32, today: NaiveDate) -> Summary {
    let days = days.clamp(1, KEEP_DAYS);
    let mut s = Summary {
        days,
        ..Default::default()
    };
    for back in 0..days as i64 {
        let day = today - Duration::days(back);
        s.by_day.insert(day.format("%Y-%m-%d").to_string(), 0);
    }
    let mut distinct = BTreeSet::new();
    for r in &records {
        s.requests += 1;
        let day = r.t.with_timezone(&Local).format("%Y-%m-%d").to_string();
        for v in &r.values {
            if !v.a.is_empty() && v.a != "mask" {
                continue;
            }
            distinct.insert(v.p.as_str());
            s.values += v.n;
            *s.by_day.entry(day.clone()).or_default() += v.n;
            *s.by_kind.entry(v.k.clone()).or_default() += v.n;
            if !r.agent.is_empty() {
                *s.by_agent.entry(r.agent.clone()).or_default() += v.n;
            }
            if !r.provider.is_empty() {
                *s.by_provider.entry(r.provider.clone()).or_default() += v.n;
            }
        }
    }
    s.distinct = distinct.len();
    records.reverse();
    s.records = records;
    s
}

/// The key masking makes its placeholders with (`mask.key`, private),
/// made the first time it is asked for: the same value then has the same
/// placeholder in every request, and only on this computer.
pub fn key() -> Result<Vec<u8>> {
    let path = paths::home().join("mask.key");
    if let Ok(text) = fs::read_to_string(&path) {
        let text = text.trim();
        if text.len() == 64
            && let Some(bytes) = unhex(text)
        {
            return Ok(bytes);
        }
    }
    let mut bytes = Vec::with_capacity(32);
    bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    let text: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    fs::create_dir_all(paths::home())?;
    fsutil::write_atomic(&path, text.as_bytes(), true)?;
    Ok(bytes)
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(p: &str, k: &str, n: usize) -> Value {
        Value {
            p: p.into(),
            k: k.into(),
            b: "rule".into(),
            l: 10,
            n,
            ..Default::default()
        }
    }

    #[test]
    fn sums_the_days() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 13).unwrap();
        let at = today
            .and_hms_opt(12, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .unwrap()
            .with_timezone(&Utc);
        let records = vec![
            Record {
                key: 1,
                t: at,
                agent: "codex".into(),
                provider: "openai".into(),
                n: 3,
                values: vec![value("{{EMAIL_a}}", "EMAIL", 2), value("{{IP_b}}", "IP", 1)],
                ..Default::default()
            },
            Record {
                key: 2,
                t: at,
                agent: "codex".into(),
                provider: "deepseek".into(),
                n: 1,
                values: vec![value("{{EMAIL_a}}", "EMAIL", 1)],
                ..Default::default()
            },
        ];
        let s = summarize(records, 7, today);
        assert_eq!((s.values, s.distinct, s.requests), (4, 2, 2));
        assert_eq!(s.by_day.len(), 7);
        assert_eq!(s.by_day["2026-11-13"], 4);
        assert_eq!(s.by_kind["EMAIL"], 3);
        assert_eq!(s.by_provider["deepseek"], 1);
        assert_eq!(s.records[0].key, 2, "newest first");
    }

    #[test]
    fn keys_are_unique() {
        let a = next_key();
        let b = next_key();
        assert!(b > a);
    }
}
