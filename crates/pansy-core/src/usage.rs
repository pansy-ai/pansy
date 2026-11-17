//! What each request through the gateway used: one JSON line per request in
//! a file per day (`usage/2026-11-12.jsonl`), read back for the Usage page
//! and `pansy usage`. Prompts and replies are never written.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

use anyhow::Result;
use chrono::{DateTime, Duration, Local, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub at: DateTime<Utc>,
    /// The agent that asked, as its user agent tells (`claude-code`, `codex`), or `other`.
    pub agent: String,
    /// The model it asked for (`deepseek/deepseek-chat`, `group/daily`).
    pub requested: String,
    pub provider: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub account: String,
    /// The model the provider was asked for.
    pub model: String,
    /// The protocol the agent spoke and the one the provider was spoken to in.
    pub client: String,
    pub upstream: String,
    pub status: u16,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub input: u64,
    #[serde(default)]
    pub output: u64,
    #[serde(default)]
    pub cache_read: u64,
    #[serde(default)]
    pub cache_write: u64,
    #[serde(default)]
    pub reasoning: u64,
    pub ms: u64,
    /// Time to the first byte of the answer.
    #[serde(default)]
    pub first_ms: u64,
    /// Accounts or members tried before this one answered.
    #[serde(default)]
    pub failovers: u32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
    /// Those tries, in order: who each was and what it answered.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tried: Vec<Tried>,
}

/// An account tried before the one a request is recorded under.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Tried {
    pub provider: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub account: String,
    pub model: String,
    pub status: u16,
}

fn dir() -> PathBuf {
    paths::home().join("usage")
}

fn file_for(day: NaiveDate) -> PathBuf {
    dir().join(format!("{}.jsonl", day.format("%Y-%m-%d")))
}

/// Appends `record` to its day's file.
pub fn append(record: &Record) -> Result<()> {
    let path = file_for(record.at.with_timezone(&Local).date_naive());
    fs::create_dir_all(path.parent().unwrap())?;
    let mut line = serde_json::to_vec(record)?;
    line.push(b'\n');
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    file.write_all(&line)?;
    Ok(())
}

/// The records of the last `days` days (today included), oldest first.
pub fn recent(days: u32) -> Vec<Record> {
    let today = Local::now().date_naive();
    (0..days.max(1) as i64)
        .rev()
        .flat_map(|back| on(today - Duration::days(back)))
        .collect()
}

/// The records of the day `YYYY-MM-DD`, oldest first.
pub fn day(name: &str) -> Vec<Record> {
    NaiveDate::parse_from_str(name, "%Y-%m-%d")
        .map(on)
        .unwrap_or_default()
}

/// The last `days` days that had requests, newest first: each `YYYY-MM-DD`
/// with how many.
pub fn busy_days(days: u32) -> Vec<(String, usize)> {
    let today = Local::now().date_naive();
    (0..days.max(1) as i64)
        .map(|back| today - Duration::days(back))
        .map(|day| (day.format("%Y-%m-%d").to_string(), on(day).len()))
        .filter(|(_, n)| *n > 0)
        .collect()
}

/// The records of `day`, oldest first.
pub fn on(day: NaiveDate) -> Vec<Record> {
    let Ok(file) = fs::File::open(file_for(day)) else {
        return Vec::new();
    };
    BufReader::new(file)
        .lines()
        .map_while(|l| l.ok())
        .filter_map(|line| serde_json::from_str::<Record>(&line).ok())
        .collect()
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Totals {
    pub requests: u64,
    pub errors: u64,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub reasoning: u64,
}

impl Totals {
    fn add(&mut self, r: &Record) {
        self.requests += 1;
        if r.status >= 400 {
            self.errors += 1;
        }
        self.input += r.input;
        self.output += r.output;
        self.cache_read += r.cache_read;
        self.cache_write += r.cache_write;
        self.reasoning += r.reasoning;
    }
}

/// The Usage page: totals, by day, by model, by agent and by provider.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub days: u32,
    pub total: Totals,
    /// `YYYY-MM-DD` → totals, every day of the range present.
    pub by_day: BTreeMap<String, Totals>,
    pub by_model: BTreeMap<String, Totals>,
    pub by_agent: BTreeMap<String, Totals>,
    pub by_provider: BTreeMap<String, Totals>,
    /// The latest requests, newest first.
    pub latest: Vec<Record>,
}

pub fn summary(days: u32) -> Summary {
    summarize(&recent(days), days, Local::now().date_naive())
}

pub fn summarize(records: &[Record], days: u32, today: NaiveDate) -> Summary {
    let mut s = Summary {
        days,
        ..Summary::default()
    };
    for back in 0..days.max(1) as i64 {
        s.by_day.insert(
            (today - Duration::days(back))
                .format("%Y-%m-%d")
                .to_string(),
            Totals::default(),
        );
    }
    for r in records {
        s.total.add(r);
        let day =
            r.at.with_timezone(&Local)
                .date_naive()
                .format("%Y-%m-%d")
                .to_string();
        s.by_day.entry(day).or_default().add(r);
        s.by_model
            .entry(format!("{}/{}", r.provider, r.model))
            .or_default()
            .add(r);
        s.by_agent.entry(r.agent.clone()).or_default().add(r);
        s.by_provider.entry(r.provider.clone()).or_default().add(r);
    }
    s.latest = records.iter().rev().take(50).cloned().collect();
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums_by_day_model_and_agent() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 12).unwrap();
        let at = Local::now().with_timezone(&Utc);
        let day = at.with_timezone(&Local).date_naive();
        let r = |agent: &str, status: u16| Record {
            at,
            agent: agent.into(),
            provider: "deepseek".into(),
            model: "deepseek-chat".into(),
            status,
            input: 10,
            output: 2,
            ..Record::default()
        };
        let s = summarize(&[r("codex", 200), r("claude-code", 429)], 7, day.max(today));
        assert_eq!(s.total.requests, 2);
        assert_eq!(s.total.errors, 1);
        assert_eq!(s.by_model["deepseek/deepseek-chat"].input, 20);
        assert_eq!(s.by_agent.len(), 2);
        assert!(s.by_day.len() >= 7);
    }
}
