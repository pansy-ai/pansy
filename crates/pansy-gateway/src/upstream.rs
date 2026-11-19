//! Requests to providers: their URLs and headers per protocol, an HTTP
//! client per proxy, and what a failed answer means for the next attempt.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use pansy_core::provider::{AuthHeader, Protocol};
use pansy_core::settings::use_proxy;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

use crate::route::Attempt;

pub const USER_AGENT: &str = concat!("pansy/", env!("CARGO_PKG_VERSION"));

/// HTTP clients, one per proxy (none for the system's, `direct`, or an
/// address): building one is not free.
#[derive(Default)]
pub struct Clients {
    clients: Mutex<HashMap<String, reqwest::Client>>,
}

impl Clients {
    pub fn get(&self, proxy: Option<&str>) -> Result<reqwest::Client, String> {
        let key = proxy.unwrap_or_default().to_owned();
        let mut clients = self.clients.lock().unwrap();
        if let Some(client) = clients.get(&key) {
            return Ok(client.clone());
        }
        let builder = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(20))
            .tcp_keepalive(Duration::from_secs(30))
            .pool_idle_timeout(Duration::from_secs(90));
        let client = use_proxy(builder, &key)?
            .build()
            .map_err(|e| e.to_string())?;
        clients.insert(key, client.clone());
        Ok(client)
    }
}

fn join(base: &str, path: &str) -> String {
    format!("{}{}", base.trim_end_matches('/'), path)
}

/// The URL of `attempt`'s endpoint for one request.
pub fn url(attempt: &Attempt, stream: bool) -> String {
    let base = attempt.endpoint.base_url.trim_end_matches('/');
    if let Some(url) = attempt
        .subscription
        .and_then(|kind| pansy_core::subscription::url(kind, base, stream))
    {
        return url;
    }
    match attempt.endpoint.protocol {
        Protocol::Anthropic => {
            if base.ends_with("/v1") {
                join(base, "/messages")
            } else {
                join(base, "/v1/messages")
            }
        }
        Protocol::Chat => join(base, "/chat/completions"),
        Protocol::Responses => join(base, "/responses"),
        Protocol::Gemini => {
            let root = if base.ends_with("/v1beta") || base.ends_with("/v1") {
                base.to_owned()
            } else {
                join(base, "/v1beta")
            };
            let action = if stream {
                "streamGenerateContent?alt=sse"
            } else {
                "generateContent"
            };
            format!("{root}/models/{}:{action}", attempt.model)
        }
    }
}

fn key_of(attempt: &Attempt) -> &str {
    attempt
        .account
        .as_ref()
        .map(|a| a.key.as_str())
        .unwrap_or("")
}

/// Whether `attempt` sends its key in both `x-api-key` and `Authorization`,
/// as pansy does by default to a third-party Anthropic API (as magpie does).
pub fn sends_both(attempt: &Attempt) -> bool {
    attempt.endpoint.protocol == Protocol::Anthropic
        && attempt.auth.is_none()
        && !key_of(attempt).is_empty()
}

/// A 401 whose body says `x-api-key` and `Authorization` can't both be
/// sent, in Chinese or English: the vendor takes the key in one alone.
pub fn both_refused(status: u16, body: &[u8]) -> bool {
    if status != 401 {
        return false;
    }
    let body = String::from_utf8_lossy(body).to_lowercase();
    if !body.contains("x-api-key") || !body.contains("authorization") {
        return false;
    }
    [
        "同时",
        "并存",
        "共存",
        "只能使用一个",
        "只能选择一个",
        "只能有一个",
        "cannot both",
        "can't both",
        "can not both",
        "cannot be both",
        "not both",
        "together",
        "at the same time",
        "simultaneous",
        "mutually exclusive",
        "only one of",
        "only one auth",
        "only one header",
        "both present",
        "both be present",
        "both provided",
        "both be provided",
    ]
    .iter()
    .any(|s| body.contains(s))
}

/// Headers of `attempt`'s request: its key the way the protocol takes it,
/// the provider's own headers, and from the client, when its request passes
/// through, those that go with its body (`anthropic-beta`) and its user
/// agent (a coding plan may serve only coding agents).
pub fn headers(attempt: &Attempt, client: &HeaderMap, passthrough: bool) -> HeaderMap {
    let mut out = HeaderMap::new();
    out.insert("content-type", HeaderValue::from_static("application/json"));
    let key = key_of(attempt);
    let bearer = |key: &str| HeaderValue::from_str(&format!("Bearer {key}")).ok();
    match attempt.endpoint.protocol {
        Protocol::Anthropic => {
            if !key.is_empty() {
                if attempt.auth != Some(AuthHeader::XApiKey)
                    && let Some(v) = bearer(key)
                {
                    out.insert("authorization", v);
                }
                if attempt.auth != Some(AuthHeader::Bearer)
                    && let Ok(v) = HeaderValue::from_str(key)
                {
                    out.insert("x-api-key", v);
                }
            }
            let version = client
                .get("anthropic-version")
                .filter(|_| passthrough)
                .cloned()
                .unwrap_or(HeaderValue::from_static("2023-06-01"));
            out.insert("anthropic-version", version);
            if passthrough && let Some(beta) = client.get("anthropic-beta") {
                out.insert("anthropic-beta", beta.clone());
            }
        }
        // Azure's key goes in api-key: a Bearer there is an Entra ID token.
        Protocol::Chat | Protocol::Responses if attempt.azure => {
            if let Ok(v) = HeaderValue::from_str(key)
                && !key.is_empty()
            {
                out.insert("api-key", v);
            }
        }
        Protocol::Chat | Protocol::Responses => {
            if !key.is_empty()
                && let Some(v) = bearer(key)
            {
                out.insert("authorization", v);
            }
        }
        Protocol::Gemini => {
            if let Ok(v) = HeaderValue::from_str(key)
                && !key.is_empty()
            {
                out.insert("x-goog-api-key", v);
            }
        }
    }
    if passthrough && let Some(agent) = client.get("user-agent") {
        out.insert("user-agent", agent.clone());
    }
    for (name, value) in &attempt.headers {
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            out.insert(name, value);
        }
    }
    out
}

/// `text` (a vendor's error, a connection's) without the key `attempt`
/// used: some vendors say back the key they refused.
pub fn hide_key(text: &str, attempt: &Attempt) -> String {
    match attempt.account.as_ref().map(|a| a.key.as_str()) {
        Some(key) if key.len() >= 8 => text.replace(key, &pansy_core::provider::mask_key(key)),
        _ => text.to_owned(),
    }
}

/// What the gateway does after a failed answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Try the next attempt; the key rests first, as far as `Rest` says.
    Next(Rest),
    /// The model turned the request itself away, as its other keys would:
    /// the next model is tried, and the client hears this if none answers.
    Refused,
}

/// How far a key rests after a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rest {
    /// Not at all.
    No,
    /// For the model asked: the key's other models are asked as before.
    Model(Duration),
    /// For every model: the key itself was refused, or is out of credit.
    Key(Duration),
}

impl Rest {
    pub fn secs(self) -> u64 {
        match self {
            Rest::No => 0,
            Rest::Model(rest) | Rest::Key(rest) => rest.as_secs(),
        }
    }
}

/// What an upstream's error answer means.
pub fn verdict(
    status: u16,
    body: &str,
    retry_after: Option<Duration>,
    cooldown: Duration,
) -> Verdict {
    let text = body.to_ascii_lowercase();
    let quota = [
        "insufficient",
        "quota",
        "balance",
        "credit",
        "billing",
        "exceeded your",
        "余额",
        "额度",
    ]
    .iter()
    .any(|w| text.contains(w));
    match status {
        // A limit is a model's as often as a key's (a relay's channel, a
        // plan's window for each model), and its words don't say which.
        429 => Verdict::Next(Rest::Model(retry_after.unwrap_or(if quota {
            Duration::from_secs(3600)
        } else {
            cooldown
        }))),
        401 | 403 => Verdict::Next(Rest::Key(Duration::from_secs(600))),
        402 => Verdict::Next(Rest::Key(Duration::from_secs(3600))),
        // The model is not this provider's, or not by this name.
        404 => Verdict::Next(Rest::No),
        408 | 409 | 425 | 500..=599 => {
            Verdict::Next(Rest::Model(retry_after.unwrap_or(Duration::from_secs(15))))
        }
        400 if quota => Verdict::Next(Rest::Key(Duration::from_secs(3600))),
        _ => Verdict::Refused,
    }
}

/// `Retry-After` in seconds.
pub fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let secs: u64 = headers
        .get("retry-after")?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some(Duration::from_secs(secs.min(3600)))
}

/// When an error body says the limit renews: ChatGPT's `resets_in_seconds`
/// or `resets_at`, Google's `retryDelay` (`"3600s"`). Up to 8 days: a
/// weekly window renews within one.
pub fn retry_in(body: &str) -> Option<Duration> {
    fn find<'a>(v: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
        match v {
            serde_json::Value::Object(m) => {
                m.get(key).or_else(|| m.values().find_map(|v| find(v, key)))
            }
            serde_json::Value::Array(a) => a.iter().find_map(|v| find(v, key)),
            _ => None,
        }
    }
    let json: serde_json::Value = serde_json::from_str(body).ok()?;
    let secs = find(&json, "resets_in_seconds")
        .and_then(serde_json::Value::as_f64)
        .or_else(|| {
            let at = find(&json, "resets_at")?.as_f64()?;
            Some(at - chrono::Utc::now().timestamp() as f64)
        })
        .or_else(|| {
            find(&json, "retryDelay")?
                .as_str()?
                .trim_end_matches('s')
                .parse()
                .ok()
        })?;
    (secs > 0.0).then(|| Duration::from_secs_f64(secs.min(8.0 * 86400.0).ceil()))
}

/// The message of an upstream's error body, whatever its protocol.
pub fn error_message(body: &str) -> String {
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(body) {
        let json = if json.is_array() {
            json[0].clone()
        } else {
            json
        };
        for pointer in ["/error/message", "/message", "/error", "/detail", "/msg"] {
            if let Some(text) = json.pointer(pointer).and_then(|v| v.as_str()) {
                return text.to_owned();
            }
        }
    }
    let body = body.trim();
    if body.is_empty() {
        "no answer".to_owned()
    } else {
        body.chars().take(500).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pansy_core::provider::Endpoint;

    fn attempt(protocol: Protocol, base: &str) -> Attempt {
        Attempt {
            provider: "x".into(),
            preset: None,
            account: None,
            endpoint: Endpoint {
                protocol,
                base_url: base.into(),
            },
            model: "m-1".into(),
            headers: Default::default(),
            proxy: None,
            quirks: Default::default(),
            auth: None,
            azure: false,
            subscription: None,
            plugin: None,
            api_id: None,
        }
    }

    #[test]
    fn sends_the_key_as_the_vendor_takes_it() {
        let mut a = attempt(Protocol::Anthropic, "https://api.deepseek.com/anthropic");
        a.account = Some(pansy_core::provider::Account {
            id: "k1".into(),
            label: String::new(),
            key: "sk-1".into(),
            enabled: true,
            proxy: None,
            sign_in: None,
            plugin: None,
        });
        let both = headers(&a, &HeaderMap::new(), false);
        assert_eq!(both["x-api-key"], "sk-1");
        assert_eq!(both["authorization"], "Bearer sk-1");
        assert!(sends_both(&a));
        a.auth = Some(AuthHeader::Bearer);
        let bearer = headers(&a, &HeaderMap::new(), false);
        assert!(bearer.get("x-api-key").is_none());
        a.auth = Some(AuthHeader::XApiKey);
        assert!(
            headers(&a, &HeaderMap::new(), false)
                .get("authorization")
                .is_none()
        );
        a.endpoint.protocol = Protocol::Chat;
        a.azure = true;
        let azure = headers(&a, &HeaderMap::new(), false);
        assert_eq!(azure["api-key"], "sk-1");
        assert!(azure.get("authorization").is_none());
        assert!(both_refused(
            401,
            "鉴权 header， x-api-key 和 Authorization 不可以同时存在".as_bytes()
        ));
        assert!(!both_refused(401, b"invalid x-api-key"));
    }

    #[test]
    fn joins_paths_per_protocol() {
        assert_eq!(
            url(
                &attempt(Protocol::Anthropic, "https://api.deepseek.com/anthropic"),
                true
            ),
            "https://api.deepseek.com/anthropic/v1/messages"
        );
        assert_eq!(
            url(&attempt(Protocol::Anthropic, "https://x/v1/"), true),
            "https://x/v1/messages"
        );
        assert_eq!(
            url(&attempt(Protocol::Chat, "https://api.openai.com/v1"), true),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            url(
                &attempt(
                    Protocol::Gemini,
                    "https://generativelanguage.googleapis.com"
                ),
                true
            ),
            "https://generativelanguage.googleapis.com/v1beta/models/m-1:streamGenerateContent?alt=sse"
        );
    }

    #[test]
    fn reads_when_a_limit_renews() {
        let codex = r#"{"error":{"type":"usage_limit_reached","message":"The usage limit has been reached","resets_in_seconds":7200}}"#;
        assert_eq!(retry_in(codex), Some(Duration::from_secs(7200)));
        let google = r#"[{"error":{"code":429,"status":"RESOURCE_EXHAUSTED","details":[{"@type":"type.googleapis.com/google.rpc.RetryInfo","retryDelay":"36.5s"}]}}]"#;
        assert_eq!(retry_in(google), Some(Duration::from_secs(37)));
        assert_eq!(retry_in(r#"{"error":{"message":"slow down"}}"#), None);
        assert_eq!(
            verdict(429, codex, retry_in(codex), Duration::from_secs(60)),
            Verdict::Next(Rest::Model(Duration::from_secs(7200)))
        );
    }

    #[test]
    fn judges_failures() {
        let cool = Duration::from_secs(60);
        // A limit and a server's error rest the key for the model asked.
        assert_eq!(
            verdict(429, "", None, cool),
            Verdict::Next(Rest::Model(cool))
        );
        assert_eq!(
            verdict(429, "insufficient_quota", None, cool),
            Verdict::Next(Rest::Model(Duration::from_secs(3600)))
        );
        assert_eq!(
            verdict(503, "", Some(Duration::from_secs(3)), cool),
            Verdict::Next(Rest::Model(Duration::from_secs(3)))
        );
        // A key refused or out of credit rests for every model.
        assert_eq!(
            verdict(401, "bad key", None, cool),
            Verdict::Next(Rest::Key(Duration::from_secs(600)))
        );
        assert_eq!(
            verdict(400, "Insufficient Balance", None, cool),
            Verdict::Next(Rest::Key(Duration::from_secs(3600)))
        );
        assert_eq!(
            verdict(404, "no such model", None, cool),
            Verdict::Next(Rest::No)
        );
        // The request turned away: another model may take it.
        for (status, body) in [
            (400, "context too long"),
            (
                400,
                "Inappropriate input/output rejected for security reasons",
            ),
            (
                400,
                "request requires unsupported input or output modalities",
            ),
            (413, ""),
            (422, "extra_forbidden"),
        ] {
            assert_eq!(
                verdict(status, body, None, cool),
                Verdict::Refused,
                "{status} {body}"
            );
        }
        assert_eq!(
            error_message(r#"{"error":{"message":"bad key"}}"#),
            "bad key"
        );
    }
}
