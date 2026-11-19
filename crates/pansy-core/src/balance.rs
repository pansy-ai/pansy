//! What is left on an API key, as magpie reads it beside the subscriptions'
//! allowances: DeepSeek, Kimi (Moonshot), OpenRouter, StepFun and AiHubMix
//! keys are asked at their vendor's balance query, known by its host. A
//! relay added by hand (no preset, or a relay's) is asked where relays tell
//! a key its balance with the key alone — a sub2api relay's `/v1/usage`,
//! then a new-api (one-api) relay's `/api/usage/token` — and what answered
//! is kept in memory for the next reads: a day, six hours when neither
//! told a balance, five minutes when the relay couldn't be reached. A
//! sub2api key given spending limits tells its 5-hour, day and week
//! windows too.
//!
//! Read on demand (the window), kept a minute; nothing is written, and a
//! key is kept only as a hash of it, beside the relay's address.

use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::presets::{self, Kind};
use crate::provider::{Account, Provider};
use crate::subscription::allowance::{Window, span_name};

/// One key's balance.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Balance {
    pub provider: String,
    pub account: String,
    /// The provider's name.
    pub name: String,
    /// The key's name, when the provider has several keys on.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub user: String,
    /// What is left: `$12.50`, `¥110.00 · $3.00`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub amount: String,
    /// The key's own spending windows, when its relay gives it some.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub windows: Vec<Window>,
    /// Why it couldn't be read.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
    pub read_at: DateTime<Utc>,
}

/// How a reply tells the balance: a vendor's own, or a relay's query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reply {
    DeepSeek,
    /// Kimi's, in this currency.
    Moonshot(&'static str),
    OpenRouter,
    StepFun(&'static str),
    AiHubMix,
    Sub2Api,
    NewApi,
}

/// A sub2api relay's query for a key: what is left on it, in dollars,
/// and the key's spending windows.
const SUB2API: &str = "/v1/usage";
/// A new-api (one-api) relay's query for a key, in its units ($1 is 500000).
const NEWAPI: &str = "/api/usage/token";
const NEWAPI_UNIT: f64 = 500_000.0;

/// Where a provider's balance is asked, and how the reply reads.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Source {
    url: String,
    reply: Reply,
}

/// The lowered hosts of a provider's endpoints.
fn hosts(p: &Provider) -> Vec<String> {
    p.endpoints
        .iter()
        .filter_map(|e| reqwest::Url::parse(e.base_url.trim()).ok())
        .filter_map(|u| u.host_str().map(str::to_lowercase))
        .collect()
}

/// The vendor's own balance query, known by the provider's host.
fn known(p: &Provider) -> Option<Source> {
    hosts(p).iter().find_map(|h| {
        let (url, reply) = match h.as_str() {
            "api.deepseek.com" => ("https://api.deepseek.com/user/balance", Reply::DeepSeek),
            "api.moonshot.cn" => (
                "https://api.moonshot.cn/v1/users/me/balance",
                Reply::Moonshot("¥"),
            ),
            "api.moonshot.ai" => (
                "https://api.moonshot.ai/v1/users/me/balance",
                Reply::Moonshot("$"),
            ),
            "openrouter.ai" => ("https://openrouter.ai/api/v1/credits", Reply::OpenRouter),
            "api.stepfun.com" => ("https://api.stepfun.com/v1/accounts", Reply::StepFun("¥")),
            "api.stepfun.ai" => ("https://api.stepfun.ai/v1/accounts", Reply::StepFun("$")),
            "aihubmix.com" => (
                "https://aihubmix.com/dashboard/billing/remain",
                Reply::AiHubMix,
            ),
            _ => return None,
        };
        Some(Source {
            url: url.to_owned(),
            reply,
        })
    })
}

/// The hosts a vendor's or a local server's preset names: never asked a
/// relay's query.
static VENDOR_HOSTS: LazyLock<HashSet<String>> = LazyLock::new(|| {
    presets::PRESETS
        .iter()
        .filter(|d| d.kind != Kind::Relay)
        .flat_map(|d| {
            d.urls
                .iter()
                .chain(d.regions.iter().flat_map(|r| r.urls.iter()))
        })
        .filter_map(|(_, url)| reqwest::Url::parse(url).ok())
        .filter_map(|u| u.host_str().map(str::to_lowercase))
        .collect()
});

/// Relays are found by these: OpenCode's Zen and the Cline API tell no key
/// its balance this way.
const NOT_RELAYS: &[&str] = &["opencode.ai", "api.cline.bot"];

/// This computer, never asked (but by the tests, `local`).
fn loopback(host: &str) -> bool {
    match host.trim_matches(['[', ']']).parse::<std::net::IpAddr>() {
        Ok(ip) => ip.is_loopback() || ip.is_unspecified(),
        Err(_) => host == "localhost" || host.ends_with(".localhost"),
    }
}

/// The scheme and host of each base the provider names, each once, but a
/// vendor's own host and this computer.
fn origins(p: &Provider, local: bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for e in &p.endpoints {
        let Ok(u) = reqwest::Url::parse(e.base_url.trim()) else {
            continue;
        };
        let Some(host) = u.host_str().map(str::to_lowercase) else {
            continue;
        };
        if !matches!(u.scheme(), "http" | "https")
            || (!local && loopback(&host))
            || VENDOR_HOSTS.contains(&host)
            || NOT_RELAYS.contains(&host.as_str())
        {
            continue;
        }
        let origin = match u.port() {
            Some(port) => format!("{}://{host}:{port}", u.scheme()),
            None => format!("{}://{host}", u.scheme()),
        };
        if !out.contains(&origin) {
            out.push(origin);
        }
    }
    out
}

/// Whether the provider's balance is looked for at a relay's queries: an
/// API-key provider added by hand or from a relay's preset, whose vendor's
/// own query isn't known.
fn relay(p: &Provider, local: bool) -> bool {
    p.subscription.is_none()
        && p.plugin.is_none()
        && p.preset
            .as_deref()
            .is_none_or(|id| presets::find(id).is_some_and(|d| d.kind == Kind::Relay))
        && known(p).is_none()
        && !origins(p, local).is_empty()
}

/// What a relay's look found: the query that told a balance (none), and
/// until when that holds.
struct Found {
    url: Option<String>,
    until: Instant,
}

const FOUND_FOR: Duration = Duration::from_secs(24 * 3600);
const NONE_FOR: Duration = Duration::from_secs(6 * 3600);
const DOWN_FOR: Duration = Duration::from_secs(5 * 60);

static PROBED: LazyLock<Mutex<HashMap<String, Found>>> = LazyLock::new(Default::default);

/// What a look is kept by: the relay's origins and a hash of the key.
fn probe_key(origins: &[String], key: &str) -> String {
    let hash = Sha256::digest(key.as_bytes());
    let tag: String = hash[..8].iter().map(|b| format!("{b:02x}")).collect();
    format!("{} {tag}", origins.join(" "))
}

fn reply_at(url: &str) -> Reply {
    if url.ends_with(NEWAPI) {
        Reply::NewApi
    } else {
        Reply::Sub2Api
    }
}

/// The query of the provider's relay that tells `key` its balance: as
/// found before, or asked of each origin's sub2api and new-api queries in
/// turn. None when neither told one.
async fn probe(http: &reqwest::Client, p: &Provider, key: &str, local: bool) -> Option<String> {
    let origins = origins(p, local);
    let k = probe_key(&origins, key);
    if let Some(f) = PROBED.lock().unwrap().get(&k)
        && f.until > Instant::now()
    {
        return f.url.clone();
    }
    let (mut found, mut reached) = (None, false);
    'origins: for o in &origins {
        for path in [SUB2API, NEWAPI] {
            let url = format!("{o}{path}");
            let Ok((status, body)) = ask(http, &url, p, key).await else {
                continue;
            };
            reached = true;
            if status == 200 && read(reply_at(&url), &body).is_ok() {
                found = Some(url);
                break 'origins;
            }
        }
    }
    let wait = match (&found, reached) {
        (Some(_), _) => FOUND_FOR,
        (None, true) => NONE_FOR,
        (None, false) => DOWN_FOR,
    };
    PROBED.lock().unwrap().insert(
        k,
        Found {
            url: found.clone(),
            until: Instant::now() + wait,
        },
    );
    found
}

/// Asks `url` with the key as a bearer and the provider's headers: the
/// status and up to a megabyte of the reply.
async fn ask(http: &reqwest::Client, url: &str, p: &Provider, key: &str) -> Result<(u16, Vec<u8>)> {
    let mut req = http
        .get(url)
        .header("Accept", "application/json")
        .bearer_auth(key);
    for (k, v) in &p.headers {
        req = req.header(k, v);
    }
    let res = req.send().await?;
    let status = res.status().as_u16();
    let mut body = res.bytes().await?.to_vec();
    body.truncate(1 << 20);
    Ok((status, body))
}

/// An amount given as a JSON number or as a string of one.
fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// An amount with its currency's sign in front: `¥12.34`.
fn money(sign: &str, v: f64) -> String {
    format!("{sign}{v:.2}")
}

fn currency_sign(code: &str) -> String {
    match code.to_uppercase().as_str() {
        "CNY" | "RMB" => "¥".to_owned(),
        "USD" => "$".to_owned(),
        "" => String::new(),
        other => format!("{other} "),
    }
}

/// What a balance reply says: the amount, and a key's windows.
#[derive(Debug, Default, PartialEq)]
struct Read {
    amount: String,
    windows: Vec<Window>,
}

fn amount(amount: String) -> Result<Read> {
    Ok(Read {
        amount,
        windows: Vec::new(),
    })
}

/// The balance a reply tells, as `reply` gives it.
fn read(reply: Reply, body: &[u8]) -> Result<Read> {
    let v: Value = serde_json::from_slice(body).map_err(|_| anyhow!("the reply is not JSON"))?;
    let none = || anyhow!("no balance in the reply");
    match reply {
        // {"balance_infos":[{"currency":"CNY","total_balance":"110.00"}]}:
        // a currency with money left first, then by its code, so they keep
        // their places from one read to the next
        Reply::DeepSeek => {
            let mut amounts: Vec<(String, f64)> = v["balance_infos"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|i| {
                    let total = number(&i["total_balance"])?;
                    Some((i["currency"].as_str().unwrap_or("").to_uppercase(), total))
                })
                .collect();
            if amounts.is_empty() {
                return Err(none());
            }
            amounts.sort_by(|a, b| (b.1 > 0.0).cmp(&(a.1 > 0.0)).then_with(|| a.0.cmp(&b.0)));
            amount(
                amounts
                    .iter()
                    .map(|(code, n)| money(&currency_sign(code), *n))
                    .collect::<Vec<_>>()
                    .join(" · "),
            )
        }
        // {"data":{"available_balance":49.58}}
        Reply::Moonshot(sign) => amount(money(
            sign,
            number(&v["data"]["available_balance"]).ok_or_else(none)?,
        )),
        // {"data":{"total_credits":20,"total_usage":3.5}}, in dollars
        Reply::OpenRouter => {
            let credits = number(&v["data"]["total_credits"])
                .ok_or_else(|| anyhow!("no credits in the reply"))?;
            let used = number(&v["data"]["total_usage"]).unwrap_or(0.0);
            amount(money("$", credits - used))
        }
        // {"balance":12.5}
        Reply::StepFun(sign) => amount(money(sign, number(&v["balance"]).ok_or_else(none)?)),
        // {"total_usage":3.2}: what is left on the key; -1 for a key with no limit
        Reply::AiHubMix => {
            let left = number(&v["total_usage"]).ok_or_else(none)?;
            if left < 0.0 {
                bail!(
                    "this key has no limit, and AiHubMix tells a key only what is left on it: give the key a limit in AiHubMix's console"
                );
            }
            amount(money("$", left))
        }
        // {"remaining":787.5,"rate_limits":[{"window":"7d","limit":800,"used":12.5,"reset_at":…}]}
        Reply::Sub2Api => {
            let windows = sub2api_windows(&v);
            let left = number(&v["remaining"]);
            if left.is_none() && windows.is_empty() {
                return Err(none());
            }
            Ok(Read {
                amount: left.map(|n| money("$", n)).unwrap_or_default(),
                windows,
            })
        }
        // {"data":{"total_available":500000}}
        Reply::NewApi => amount(money(
            "$",
            number(&v["data"]["total_available"]).ok_or_else(none)? / NEWAPI_UNIT,
        )),
    }
}

/// A sub2api key's spending windows, as its panel gives a key a limit per
/// 5 hours, day and 7 days: each one's spend of its limit, and when it
/// starts again (none: it starts at the key's next request).
fn sub2api_windows(v: &Value) -> Vec<Window> {
    let limits = v["rate_limits"].as_array().cloned().unwrap_or_default();
    [("5h", 5 * 3600), ("1d", 24 * 3600), ("7d", 7 * 24 * 3600)]
        .into_iter()
        .filter_map(|(name, span)| {
            let l = limits
                .iter()
                .find(|l| l["window"].as_str().map(str::trim) == Some(name))?;
            let limit = number(&l["limit"]).filter(|n| *n > 0.0)?;
            let used = number(&l["used"]).unwrap_or(0.0).max(0.0);
            Some(Window {
                name: span_name(span),
                used: (used / limit * 100.0).clamp(0.0, 100.0),
                resets_at: l["reset_at"]
                    .as_str()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|t| t.with_timezone(&Utc)),
                span_secs: span,
                display: format!("{} / {}", money("$", used), money("$", limit)),
                unlimited: false,
            })
        })
        .collect()
}

/// What a relay's `{"success":false,"message":…}` says: a new-api relay's
/// way of turning a request down, with a 401 and a 200 alike.
fn refusal(body: &[u8]) -> Option<String> {
    let v: Value = serde_json::from_slice(body).ok()?;
    if v["success"].as_bool() != Some(false) {
        return None;
    }
    let message = v["message"].as_str()?.trim();
    (!message.is_empty()).then(|| message.to_owned())
}

/// Asks the source for the key's balance.
async fn fetch(http: &reqwest::Client, source: &Source, p: &Provider, key: &str) -> Result<Read> {
    let (status, body) = ask(http, &source.url, p, key).await?;
    let said = |msg: String| {
        if status >= 300 {
            let reason = reqwest::StatusCode::from_u16(status)
                .ok()
                .and_then(|s| s.canonical_reason())
                .unwrap_or("");
            format!("{status} {reason}: {msg}").replace("  ", " ")
        } else {
            msg
        }
    };
    if let Some(msg) = refusal(&body) {
        bail!(said(msg));
    }
    if status >= 300 {
        // what a JSON reply says, on one line; a page of HTML says nothing
        let text = String::from_utf8_lossy(&body);
        let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if !line.starts_with('{') {
            bail!(said(String::new()).trim_end_matches([':', ' ']).to_owned());
        }
        let line: String = if line.chars().count() > 200 {
            line.chars().take(200).chain(['…']).collect()
        } else {
            line
        };
        bail!(said(line));
    }
    read(source.reply, &body)
}

/// One key's balance, or none for a relay where no query told one.
async fn read_one(p: &Provider, a: &Account, several: bool, local: bool) -> Option<Balance> {
    let mut out = Balance {
        provider: p.id.clone(),
        account: a.id.clone(),
        name: p.name.clone(),
        user: if several { a.display() } else { String::new() },
        read_at: Utc::now(),
        ..Balance::default()
    };
    let http = match crate::subscription::client(a.proxy.as_deref().or(p.proxy.as_deref())) {
        Ok(http) => http,
        Err(e) => {
            out.error = format!("{e:#}");
            return Some(out);
        }
    };
    let source = match known(p) {
        Some(s) => s,
        None => {
            let url = probe(&http, p, &a.key, local).await?;
            Source {
                reply: reply_at(&url),
                url,
            }
        }
    };
    match fetch(&http, &source, p, &a.key).await {
        Ok(r) => {
            out.amount = r.amount;
            out.windows = r.windows;
        }
        Err(e) => out.error = format!("{e:#}"),
    }
    Some(out)
}

/// The balances of `providers`' keys that one can be read of, in their order.
async fn read_all(providers: Vec<Provider>, local: bool) -> Vec<Balance> {
    let mut tasks = tokio::task::JoinSet::new();
    let mut n = 0;
    for p in providers {
        if !p.enabled || (known(&p).is_none() && !relay(&p, local)) {
            continue;
        }
        let keys: Vec<Account> = p
            .accounts
            .iter()
            .filter(|a| a.enabled && !a.key.is_empty())
            .cloned()
            .collect();
        let several = keys.len() > 1;
        for a in keys {
            let p = p.clone();
            tasks.spawn(async move { (n, read_one(&p, &a, several, local).await) });
            n += 1;
        }
    }
    let mut all = Vec::with_capacity(n);
    while let Some(done) = tasks.join_next().await {
        if let Ok((i, Some(b))) = done {
            all.push((i, b));
        }
    }
    all.sort_by_key(|(i, _)| *i);
    all.into_iter().map(|(_, b)| b).collect()
}

type Kept = Option<(Instant, Vec<Balance>)>;
static READ: LazyLock<Mutex<Kept>> = LazyLock::new(Default::default);
const KEPT: Duration = Duration::from_secs(60);

/// Every API key's balance that pansy can read, in the providers' order: as
/// read within the last minute, or asked again (`fresh`: asked again now).
pub async fn balances(fresh: bool) -> Vec<Balance> {
    if !fresh
        && let Some((at, kept)) = READ.lock().unwrap().as_ref()
        && at.elapsed() < KEPT
    {
        return kept.clone();
    }
    let all = read_all(crate::config::load_providers().unwrap_or_default(), false).await;
    *READ.lock().unwrap() = Some((Instant::now(), all.clone()));
    all
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{Endpoint, Protocol};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn provider(id: &str, preset: Option<&str>, base: &str) -> Provider {
        let mut p: Provider = serde_json::from_value(serde_json::json!({
            "id": id, "name": id, "endpoints": [], "accounts": [],
        }))
        .unwrap();
        p.preset = preset.map(str::to_owned);
        p.endpoints = vec![Endpoint {
            protocol: Protocol::Chat,
            base_url: base.to_owned(),
        }];
        p.add_account("sk-test-1234", "");
        p
    }

    #[test]
    fn reads_each_vendors_reply() {
        let deepseek = br#"{"is_available":true,"balance_infos":[{"currency":"USD","total_balance":"0.00"},{"currency":"CNY","total_balance":"110.00"}]}"#;
        assert_eq!(
            read(Reply::DeepSeek, deepseek).unwrap().amount,
            "¥110.00 · $0.00"
        );
        let moonshot = br#"{"code":0,"data":{"available_balance":49.58}}"#;
        assert_eq!(
            read(Reply::Moonshot("¥"), moonshot).unwrap().amount,
            "¥49.58"
        );
        let openrouter = br#"{"data":{"total_credits":20,"total_usage":3.5}}"#;
        assert_eq!(
            read(Reply::OpenRouter, openrouter).unwrap().amount,
            "$16.50"
        );
        assert_eq!(
            read(Reply::StepFun("$"), br#"{"balance":"12.5"}"#)
                .unwrap()
                .amount,
            "$12.50"
        );
        assert!(read(Reply::AiHubMix, br#"{"total_usage":-1}"#).is_err());
        let newapi = br#"{"code":true,"data":{"total_available":2500000}}"#;
        assert_eq!(read(Reply::NewApi, newapi).unwrap().amount, "$5.00");
        assert!(read(Reply::Sub2Api, b"<html>").is_err());
        assert!(read(Reply::Sub2Api, br#"{"mode":"unrestricted"}"#).is_err());
    }

    #[test]
    fn reads_a_sub2api_keys_windows() {
        let body = br#"{"remaining":787.5,"rate_limits":[
            {"window":"7d","limit":800,"used":12.5,"reset_at":"2026-11-20T08:00:00Z"},
            {"window":"5h","limit":0,"used":0},
            {"window":"1d","limit":100,"used":50}]}"#;
        let r = read(Reply::Sub2Api, body).unwrap();
        assert_eq!(r.amount, "$787.50");
        let names: Vec<&str> = r.windows.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names, ["1 days", "7 days"]);
        assert_eq!(r.windows[0].used, 50.0);
        assert!(r.windows[0].resets_at.is_none());
        assert_eq!(r.windows[1].display, "$12.50 / $800.00");
        assert!(r.windows[1].resets_at.is_some());
    }

    #[test]
    fn looks_only_at_relays() {
        // a vendor's own host is never asked a relay's query
        assert!(!relay(
            &provider("ds", None, "https://api.deepseek.com/v1"),
            false
        ));
        assert!(known(&provider("ds", None, "https://api.deepseek.com/v1")).is_some());
        assert!(!relay(
            &provider("oa", None, "https://api.openai.com/v1"),
            false
        ));
        // this computer neither, but in a test
        assert!(!relay(
            &provider("me", None, "http://127.0.0.1:8080/v1"),
            false
        ));
        assert!(!relay(
            &provider("me", None, "http://localhost:8080/v1"),
            false
        ));
        assert!(relay(
            &provider("relay", None, "https://api.nofx.one/v1"),
            false
        ));
        assert_eq!(
            origins(&provider("relay", None, "https://API.nofx.one/v1"), false),
            ["https://api.nofx.one"]
        );
        // a vendor's preset is its vendor's, wherever it points
        assert!(!relay(
            &provider("ds", Some("deepseek"), "https://relay.example/v1"),
            false
        ));
    }

    #[test]
    fn keeps_a_key_only_as_a_hash() {
        let k = probe_key(&["https://relay.example".to_owned()], "sk-secret-key");
        assert!(!k.contains("secret"));
        assert!(k.starts_with("https://relay.example "));
    }

    /// A relay on loopback answering `routes` (path → status and body), once
    /// each ask, for as many asks as it gets.
    async fn relay_at(routes: Vec<(&'static str, u16, &'static str)>) -> String {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let at = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            while let Ok((mut s, _)) = listener.accept().await {
                let routes = routes.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0; 4096];
                    let n = s.read(&mut buf).await.unwrap_or(0);
                    let head = String::from_utf8_lossy(&buf[..n]);
                    let path = head.split_whitespace().nth(1).unwrap_or("").to_owned();
                    let authorized = head
                        .to_lowercase()
                        .contains("authorization: bearer sk-test-1234");
                    let (status, body) = routes
                        .iter()
                        .find(|r| r.0 == path)
                        .map(|r| (r.1, r.2))
                        .unwrap_or((404, r#"{"error":"not found"}"#));
                    let status = if authorized { status } else { 401 };
                    let reply = format!(
                        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = s.write_all(reply.as_bytes()).await;
                });
            }
        });
        at
    }

    #[tokio::test]
    async fn finds_a_relays_query_and_reads_the_key_there() {
        // a new-api relay: sub2api's query isn't there, new-api's is
        let at = relay_at(vec![(
            NEWAPI,
            200,
            r#"{"data":{"total_available":5000000}}"#,
        )])
        .await;
        let p = provider("relay", None, &format!("{at}/v1"));
        let got = read_all(vec![p.clone()], true).await;
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].amount, "$10.00");
        assert_eq!(got[0].name, "relay");
        assert!(got[0].user.is_empty());
        // found once, asked there again
        let k = probe_key(&origins(&p, true), "sk-test-1234");
        assert_eq!(
            PROBED.lock().unwrap()[&k].url.as_deref(),
            Some(format!("{at}{NEWAPI}").as_str())
        );
    }

    #[tokio::test]
    async fn a_relay_that_tells_no_balance_has_no_card() {
        let at = relay_at(vec![]).await;
        let p = provider("quiet", None, &format!("{at}/v1"));
        assert!(read_all(vec![p], true).await.is_empty());
    }

    #[tokio::test]
    async fn names_each_key_when_there_are_several() {
        let at = relay_at(vec![(SUB2API, 200, r#"{"remaining":3}"#)]).await;
        let mut p = provider("relay2", None, &format!("{at}/v1"));
        p.add_account("sk-test-1234", "work");
        let got = read_all(vec![p], true).await;
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].amount, "$3.00");
        assert_eq!(got[0].user, "…1234");
        assert_eq!(got[1].user, "work");
    }

    #[tokio::test]
    async fn says_why_a_balance_couldnt_be_read() {
        let at = relay_at(vec![
            (
                SUB2API,
                401,
                r#"{"success":false,"message":"invalid token"}"#,
            ),
            (NEWAPI, 500, "<html>oops</html>"),
        ])
        .await;
        let p = provider("relay3", None, &format!("{at}/v1"));
        let http = reqwest::Client::new();
        let refused = fetch(
            &http,
            &Source {
                url: format!("{at}{SUB2API}"),
                reply: Reply::Sub2Api,
            },
            &p,
            "sk-test-1234",
        )
        .await;
        assert_eq!(
            refused.unwrap_err().to_string(),
            "401 Unauthorized: invalid token"
        );
        let broken = fetch(
            &http,
            &Source {
                url: format!("{at}{NEWAPI}"),
                reply: Reply::NewApi,
            },
            &p,
            "sk-test-1234",
        )
        .await;
        assert_eq!(broken.unwrap_err().to_string(), "500 Internal Server Error");
    }

    #[test]
    fn words_a_refusal_as_the_relay_says_it() {
        assert_eq!(
            refusal(br#"{"success":false,"message":"invalid token"}"#).as_deref(),
            Some("invalid token")
        );
        assert!(refusal(br#"{"success":true,"message":"ok"}"#).is_none());
        assert!(refusal(br#"{"remaining":3}"#).is_none());
    }
}
