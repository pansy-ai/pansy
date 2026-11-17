//! What each subscription account has left of its allowance, as its vendor
//! says: ChatGPT's 5-hour and weekly windows (and credits past them),
//! Copilot's monthly chat, completions and premium requests, and Code
//! Assist's daily allowance of each model, as magpie reads them.
//!
//! Read on demand (the window, `pansy limits`) and by the gateway every few
//! minutes, kept a minute in memory. A ChatGPT account its vendor holds
//! (`spent_until`) rests until its window renews, so the next account
//! answers; nothing of an allowance is written to disk.

use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::Serialize;

use super::{Held, Kind, codex, copilot, gemini};
use crate::provider::{Account, Protocol, Provider};

/// One window of an allowance.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Window {
    /// `5 hours`, `7 days`, `Premium requests`, or a model's id.
    pub name: String,
    /// The share used, 0 to 100.
    pub used: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<DateTime<Utc>>,
    /// How long the window runs, in seconds; 0 when not said.
    #[serde(skip_serializing_if = "is_zero")]
    pub span_secs: u64,
    /// A count, when the vendor counts it (`120 / 300`).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub display: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub unlimited: bool,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// What a vendor said of one account.
#[derive(Debug, Clone, Default)]
pub(crate) struct Reading {
    pub plan: String,
    pub windows: Vec<Window>,
    /// Held by its vendor until then: every request is refused.
    pub spent_until: Option<DateTime<Utc>>,
    /// Anything else it said (credits left).
    pub note: String,
}

/// One account's allowance.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Allowance {
    pub provider: String,
    pub account: String,
    pub kind: Kind,
    pub user: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub plan: String,
    pub enabled: bool,
    pub windows: Vec<Window>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spent_until: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// Why it couldn't be read.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
    pub read_at: DateTime<Utc>,
}

impl Allowance {
    /// The account's key in the gateway's health (`codex:own`).
    pub fn key_id(&self) -> String {
        format!("{}:{}", self.provider, self.account)
    }

    /// How long it is held from now, when it is.
    pub fn held_for(&self) -> Option<Duration> {
        let until = self.spent_until?;
        (until - Utc::now()).to_std().ok()
    }
}

/// Reads one account's allowance.
pub async fn read(provider: &Provider, account: &Account) -> Allowance {
    let kind = provider.subscription.unwrap_or(Kind::Codex);
    let sign_in = account.sign_in.clone();
    let mut out = Allowance {
        provider: provider.id.clone(),
        account: account.id.clone(),
        kind,
        user: account.display(),
        plan: sign_in.as_ref().map(|s| s.plan.clone()).unwrap_or_default(),
        enabled: provider.enabled && account.enabled,
        windows: Vec::new(),
        spent_until: None,
        note: String::new(),
        error: String::new(),
        read_at: Utc::now(),
    };
    let reading = async {
        let http = super::client(account.proxy.as_deref().or(provider.proxy.as_deref()))?;
        let held = Held::of(&account.id);
        let base = |protocol, default: &'static str| {
            provider
                .endpoint(protocol)
                .map_or(default.to_owned(), |e| e.base_url.clone())
        };
        match kind {
            Kind::Codex => {
                codex::usage(&held, &base(Protocol::Responses, codex::BASE), &http).await
            }
            Kind::Copilot => copilot::usage(&held, &http).await,
            Kind::GeminiCli => {
                gemini::usage(&held, &base(Protocol::Gemini, gemini::BASE), &http).await
            }
        }
    }
    .await;
    match reading {
        Ok(r) => {
            if !r.plan.is_empty() {
                out.plan = r.plan;
                if let Some(s) = &sign_in {
                    super::note_plan(kind, &s.user, &out.plan);
                }
            }
            out.windows = r.windows;
            out.spent_until = r.spent_until.filter(|t| *t > Utc::now());
            out.note = r.note;
        }
        Err(e) => out.error = format!("{e:#}"),
    }
    out
}

/// The allowances as last read, and when.
type Kept = Option<(Instant, Vec<Allowance>)>;
static READ: LazyLock<Mutex<Kept>> = LazyLock::new(Default::default);

/// How long a reading is used before the vendors are asked again.
const KEPT: Duration = Duration::from_secs(60);

/// Every subscription account's allowance, in the providers' order: as read
/// within the last minute, or asked again (`fresh`: asked again now).
pub async fn allowances(fresh: bool) -> Vec<Allowance> {
    if !fresh
        && let Some((at, kept)) = READ.lock().unwrap().as_ref()
        && at.elapsed() < KEPT
    {
        return kept.clone();
    }
    let providers = crate::config::load_providers().unwrap_or_default();
    let mut tasks = tokio::task::JoinSet::new();
    let mut n = 0;
    for p in providers.into_iter().filter(|p| p.subscription.is_some()) {
        for a in p.accounts.clone() {
            let p = p.clone();
            tasks.spawn(async move { (n, read(&p, &a).await) });
            n += 1;
        }
    }
    let mut all = Vec::with_capacity(n);
    while let Some(done) = tasks.join_next().await {
        if let Ok(one) = done {
            all.push(one);
        }
    }
    all.sort_by_key(|(i, _)| *i);
    let all: Vec<Allowance> = all.into_iter().map(|(_, a)| a).collect();
    *READ.lock().unwrap() = Some((Instant::now(), all.clone()));
    all
}

/// `5 hours`, `7 days`: a window's span as its name.
pub(crate) fn span_name(secs: u64) -> String {
    const DAY: u64 = 24 * 60 * 60;
    match secs {
        0 => "Allowance".to_owned(),
        s if s % DAY == 0 => format!("{} days", s / DAY),
        s if s % 3600 == 0 => format!("{} hours", s / 3600),
        s => format!("{} minutes", s / 60),
    }
}

/// `120`, `1.2K`: a count as Copilot's are told.
pub(crate) fn compact(n: f64) -> String {
    if n >= 1000.0 {
        format!("{:.1}K", n / 1000.0).replace(".0K", "K")
    } else {
        format!("{}", n.round() as i64)
    }
}
