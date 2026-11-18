//! What the Routing page plays: the requests the gateway routed lately,
//! each with the accounts routing lined up for it, every try and what it
//! answered, and where the time went. Kept in memory only, and never a
//! prompt or a reply; a day's requests come back from the usage records.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use chrono::{DateTime, Utc};
use pansy_core::config::Config;
use pansy_core::group::Routing;
use pansy_core::usage::{Record, Tried};
use serde::Serialize;

use crate::ir::Usage;
use crate::route::{Attempt, Health};

/// How many requests the trace keeps.
pub const KEEP: usize = 60;

/// One request, as routing saw it.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Route {
    pub id: u64,
    pub at: DateTime<Utc>,
    pub agent: String,
    /// The model it asked for (`group/coding`, `deepseek/deepseek-chat`).
    pub requested: String,
    /// The routing group it asked for, when it asked for one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<Way>,
    /// A subagent's request, as its agent says.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub subagent: bool,
    /// The model the subagent asked for, when the gateway put it on the
    /// one set for the agent's subagents (`requested`).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub asked: String,
    /// Why it stayed on the model it asked for, though one is set for the
    /// agent's subagents.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub kept: String,
    /// Every account routing lined up, in the order it tries them.
    pub lineup: Vec<Slot>,
    /// The tries made, in order.
    pub tries: Vec<Try>,
    /// Masking the request, before any try (µs).
    pub masking_us: u64,
    pub done: bool,
    pub status: u16,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
    /// The whole request (µs), and to its answer's first byte.
    pub us: u64,
    pub first_us: u64,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
}

/// A routing group a request asked for.
#[derive(Debug, Clone, Serialize)]
pub struct Way {
    pub id: String,
    pub name: String,
    pub routing: Routing,
    /// Its members as set, `provider/model` or `group/<id>`.
    pub members: Vec<String>,
}

/// One account routing lined up.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Slot {
    pub provider: String,
    /// The key's label or end (`…a1b2`); empty for a provider that takes none.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub account: String,
    pub model: String,
    /// Seconds it had left to rest when the request came; 0 when ready.
    pub resting: u64,
    /// Requests its model had answered (Least used first goes by it).
    pub used: u64,
    /// Past the most tries a request makes: lined up, never tried.
    pub spare: bool,
}

/// One try: an account asked, and what it answered.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Try {
    /// Its place in the lineup.
    pub slot: usize,
    /// From the request to this try going out (µs).
    pub at_us: u64,
    /// 0 while it waits.
    pub status: u16,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
    /// From going out to the vendor's answer beginning (µs).
    pub head_us: u64,
    /// The try, to its answer's end (µs).
    pub us: u64,
    /// Seconds it rests after failing.
    pub rest: u64,
}

/// The requests the gateway routed lately, newest last.
#[derive(Debug, Default)]
pub struct Trace {
    routes: Mutex<VecDeque<Route>>,
    next: AtomicU64,
    version: AtomicU64,
}

impl Trace {
    /// Changes every time a request is routed, tried or answered.
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Relaxed)
    }

    /// The requests kept, oldest first.
    pub fn routes(&self) -> Vec<Route> {
        self.routes.lock().unwrap().iter().cloned().collect()
    }

    fn begin(&self, mut route: Route) -> u64 {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        route.id = id;
        let mut routes = self.routes.lock().unwrap();
        routes.push_back(route);
        while routes.len() > KEEP {
            routes.pop_front();
        }
        self.version.fetch_add(1, Ordering::Relaxed);
        id
    }

    fn update(&self, id: u64, change: impl FnOnce(&mut Route)) {
        let mut routes = self.routes.lock().unwrap();
        if let Some(route) = routes.iter_mut().rev().find(|r| r.id == id) {
            change(route);
            self.version.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn us(since: Instant) -> u64 {
    since.elapsed().as_micros() as u64
}

/// One request's place in the trace, from routing to its answer's end.
#[derive(Debug, Clone)]
pub struct Tracing {
    trace: Arc<Trace>,
    id: u64,
    started: Instant,
}

impl Tracing {
    /// A request routed: what it asked for, and the accounts lined up for it.
    #[allow(clippy::too_many_arguments)]
    pub fn begin(
        trace: &Arc<Trace>,
        started: Instant,
        config: &Config,
        health: &Health,
        agent: &str,
        requested: &str,
        attempts: &[Attempt],
    ) -> Tracing {
        let tries = config.settings.max_attempts.max(1) as usize;
        let lineup = attempts
            .iter()
            .enumerate()
            .map(|(n, a)| Slot {
                provider: a.provider.clone(),
                account: a.account.as_ref().map(|a| a.display()).unwrap_or_default(),
                model: a.model.clone(),
                resting: health.rest_left(&a.key_id()),
                used: health.used(&format!("{}/{}", a.provider, a.model)),
                spare: n >= tries,
            })
            .collect();
        let route = Route {
            at: Utc::now(),
            agent: agent.to_owned(),
            requested: requested.to_owned(),
            group: way(config, requested),
            lineup,
            ..Route::default()
        };
        Tracing {
            trace: Arc::clone(trace),
            id: trace.begin(route),
            started,
        }
    }

    /// The request is a subagent's: the model it asked for when it was
    /// put on another (`asked`), or why it wasn't (`kept`).
    pub fn subagent(&self, asked: &str, kept: &str) {
        self.trace.update(self.id, |r| {
            r.subagent = true;
            r.asked = asked.to_owned();
            r.kept = kept.to_owned();
        });
    }

    pub fn masked(&self) {
        let at = us(self.started);
        self.trace.update(self.id, |r| r.masking_us = at);
    }

    /// The account at `slot` is asked.
    pub fn asking(&self, slot: usize) {
        let at = us(self.started);
        self.trace.update(self.id, |r| {
            r.tries.push(Try {
                slot,
                at_us: at,
                ..Try::default()
            })
        });
    }

    /// The account asked last began its answer.
    pub fn heard(&self, status: u16) {
        let now = us(self.started);
        self.trace.update(self.id, |r| {
            if let Some(t) = r.tries.last_mut() {
                t.status = status;
                t.head_us = now.saturating_sub(t.at_us);
            }
        });
    }

    /// The account asked last could not answer: it rests `rest` seconds.
    pub fn failed(&self, status: u16, error: &str, rest: u64) {
        let now = us(self.started);
        self.trace.update(self.id, |r| {
            if let Some(t) = r.tries.last_mut() {
                if t.head_us == 0 {
                    t.head_us = now.saturating_sub(t.at_us);
                }
                t.status = status;
                t.error = error.to_owned();
                t.us = now.saturating_sub(t.at_us);
                t.rest = rest;
            }
        });
    }

    /// The request is over: what the agent got, and what it used.
    pub fn end(&self, status: u16, error: &str, usage: &Usage, first_ms: u64) {
        let now = us(self.started);
        self.trace.update(self.id, |r| {
            if r.done {
                return;
            }
            r.done = true;
            r.status = status;
            r.error = error.to_owned();
            r.us = now;
            r.first_us = first_ms * 1000;
            r.input = usage.input;
            r.output = usage.output;
            r.cache_read = usage.cache_read;
            if let Some(t) = r.tries.last_mut().filter(|t| t.us == 0) {
                if t.status == 0 {
                    t.status = status;
                }
                if t.status >= 400 && t.error.is_empty() {
                    t.error = error.to_owned();
                }
                t.us = now.saturating_sub(t.at_us);
            }
        });
    }

    /// The request ended before an answer took it over: the agent went away.
    pub fn gone(&self) {
        self.end(
            499,
            "the agent went away before the answer began",
            &Usage::default(),
            0,
        );
    }
}

/// The routing group `requested` names, when it names one.
fn way(config: &Config, requested: &str) -> Option<Way> {
    let id = requested.strip_prefix("group/")?;
    let group = config.group(id)?;
    Some(Way {
        id: group.id.clone(),
        name: group.name.clone(),
        routing: group.routing,
        members: group.members.clone(),
    })
}

/// A day's requests as the trace tells them, from their usage records:
/// who was tried and what each answered, without the times of each try.
pub fn from_records(records: &[Record], config: &Config) -> Vec<Route> {
    records
        .iter()
        .enumerate()
        .map(|(n, r)| {
            let last = Tried {
                provider: r.provider.clone(),
                account: r.account.clone(),
                model: r.model.clone(),
                // Cancelled while the vendor answered: it had answered.
                status: if r.status == 499 { 200 } else { r.status },
            };
            let tried: Vec<&Tried> = r.tried.iter().chain([&last]).collect();
            Route {
                id: n as u64 + 1,
                at: r.at,
                agent: r.agent.clone(),
                requested: r.requested.clone(),
                group: way(config, &r.requested),
                lineup: tried
                    .iter()
                    .map(|t| Slot {
                        provider: t.provider.clone(),
                        account: t.account.clone(),
                        model: t.model.clone(),
                        ..Slot::default()
                    })
                    .collect(),
                tries: tried
                    .iter()
                    .enumerate()
                    .map(|(slot, t)| Try {
                        slot,
                        status: t.status,
                        error: if slot + 1 == tried.len() && r.status >= 400 {
                            r.error.clone()
                        } else {
                            String::new()
                        },
                        ..Try::default()
                    })
                    .collect(),
                done: true,
                status: r.status,
                error: r.error.clone(),
                us: r.ms * 1000,
                first_us: r.first_ms * 1000,
                input: r.input,
                output: r.output,
                cache_read: r.cache_read,
                ..Route::default()
            }
        })
        .collect()
}

/// How an account or a model fared over some requests: Accounts and keys.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tally {
    pub provider: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub account: String,
    pub model: String,
    pub tried: u32,
    pub answered: u32,
    pub failed: u32,
    /// When it last answered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last: Option<DateTime<Utc>>,
}

/// Each account and model's tries over `records`, answers and failures.
pub fn tally(records: &[Record]) -> Vec<Tally> {
    let mut out: BTreeMap<(String, String, String), Tally> = BTreeMap::new();
    let mut add = |provider: &str, account: &str, model: &str, status: u16, at| {
        let t = out
            .entry((provider.to_owned(), account.to_owned(), model.to_owned()))
            .or_insert_with(|| Tally {
                provider: provider.to_owned(),
                account: account.to_owned(),
                model: model.to_owned(),
                ..Tally::default()
            });
        t.tried += 1;
        if status < 400 || status == 499 {
            t.answered += 1;
            t.last = t.last.max(Some(at));
        } else {
            t.failed += 1;
        }
    };
    for r in records {
        for t in &r.tried {
            add(&t.provider, &t.account, &t.model, t.status, r.at);
        }
        add(&r.provider, &r.account, &r.model, r.status, r.at);
    }
    out.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_latest_requests_and_counts_changes() {
        let trace = Trace::default();
        let first = trace.begin(Route::default());
        let version = trace.version();
        trace.update(first, |r| r.status = 200);
        assert!(trace.version() > version);
        for _ in 0..KEEP {
            trace.begin(Route::default());
        }
        let routes = trace.routes();
        assert_eq!(routes.len(), KEEP);
        assert_eq!(routes[0].id, first + 1);
        // A request no longer kept changes nothing.
        let version = trace.version();
        trace.update(first, |r| r.status = 500);
        assert_eq!(trace.version(), version);
    }

    #[test]
    fn tells_a_days_requests_and_tallies_their_tries() {
        let record = Record {
            agent: "claude-code".into(),
            requested: "group/coding".into(),
            provider: "relay".into(),
            account: "backup".into(),
            model: "claude-sonnet-4-6".into(),
            status: 200,
            ms: 487,
            failovers: 1,
            tried: vec![Tried {
                provider: "relay".into(),
                account: "main".into(),
                model: "claude-sonnet-4-6".into(),
                status: 429,
            }],
            ..Record::default()
        };
        let cancelled = Record {
            status: 499,
            tried: Vec::new(),
            ..record.clone()
        };
        let routes = from_records(&[record.clone(), cancelled.clone()], &Config::default());
        assert_eq!(routes[0].lineup.len(), 2);
        assert_eq!(routes[0].lineup[1].account, "backup");
        assert_eq!(routes[0].tries[0].status, 429);
        assert_eq!(routes[0].us, 487_000);
        assert_eq!(routes[1].tries[0].status, 200);
        assert_eq!(routes[1].status, 499);

        let tally = tally(&[record, cancelled]);
        let backup = tally.iter().find(|t| t.account == "backup").unwrap();
        assert_eq!((backup.tried, backup.answered, backup.failed), (2, 2, 0));
        let main = tally.iter().find(|t| t.account == "main").unwrap();
        assert_eq!((main.tried, main.answered, main.failed), (1, 0, 1));
        assert!(main.last.is_none());
    }
}
