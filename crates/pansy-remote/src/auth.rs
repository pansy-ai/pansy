//! Pairing, device sessions and DPoP (docs/plans/phone-protocol.md, "鉴权").
//!
//! One table of devices, two kinds of session:
//!
//! - **DPoP** (the app, and the web page on HTTPS or localhost): a request
//!   carries `Authorization: DPoP <token>` and a `DPoP` proof signed by the
//!   device's P-256 key; the token is bound to that key's thumbprint (`jkt`,
//!   RFC 7638). An invalid proof is refused, never taken as a plain bearer.
//! - **cookie** (the web page on a LAN's plain HTTP, which has no WebCrypto):
//!   a `pansy_session` cookie, `HttpOnly` and `SameSite=Strict`.
//!
//! `remote.json` (0600) holds hashes, thumbprints, scopes and times: no
//! pairing secret, token or cookie that could be used from it. WebSocket
//! tickets and the proofs seen (`jti`) are in memory only.
//!
//! The server (`server.rs`) calls this with each request's method, URL and
//! headers ([`Request`]); nothing here knows axum. `pansy pair` and
//! `pansy devices` may change the file while the server runs: every change
//! here reads it again first, and a request sees the file anew when it
//! changed.

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use chrono::{DateTime, TimeDelta, Utc};
use p256::ecdsa::signature::Verifier;
use p256::ecdsa::{Signature, VerifyingKey};
use pansy_core::{fsutil, paths};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// How long a pairing link is good for when nothing else is asked.
pub const PAIRING_TTL: Duration = Duration::from_secs(10 * 60);
/// An access token's life, in seconds.
pub const TOKEN_TTL: u64 = 3600;
/// A session dies after this many days unused; each use renews it.
const SESSION_DAYS: i64 = 30;
/// The cookie's `Max-Age`: the session's 30 days.
pub const SESSION_MAX_AGE: u64 = 30 * 24 * 3600;
/// The browser session's cookie.
pub const COOKIE: &str = "pansy_session";
const TICKET_SECS: i64 = 30;
/// How far a proof's `iat` may be from our clock, either way.
const SKEW_SECS: i64 = 60;
/// How long a proof's `jti` is remembered.
const REPLAY_SECS: i64 = 5 * 60;
/// Seeing a device again writes `remote.json` at most this often.
const TOUCH_SECS: i64 = 60;
/// A cookie session is sent again (its `Max-Age` renewed) when its last use
/// was longer ago than this.
const RENEW_COOKIE_SECS: i64 = 24 * 3600;

/// Why a request was refused, as the server answers it.
#[derive(Debug)]
pub enum Error {
    /// No session, or one not good (401): pair, or refresh the token.
    Unauthorized(String),
    /// A session without the scope, or waiting to be approved (403).
    Forbidden(String),
    BadRequest(String),
    /// No such device.
    NotFound(String),
    /// `remote.json` could not be read or written.
    Internal(anyhow::Error),
}

impl Error {
    /// The protocol's error code; `unauthorized` is HTTP's 401.
    pub fn code(&self) -> &'static str {
        match self {
            Error::Unauthorized(_) => "unauthorized",
            Error::Forbidden(_) => "forbidden",
            Error::BadRequest(_) => "bad_request",
            Error::NotFound(_) => "not_found",
            Error::Internal(_) => "internal",
        }
    }

    /// The HTTP status to answer with.
    pub fn status(&self) -> u16 {
        match self {
            Error::Unauthorized(_) => 401,
            Error::Forbidden(_) => 403,
            Error::BadRequest(_) => 400,
            Error::NotFound(_) => 404,
            Error::Internal(_) => 500,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unauthorized(m)
            | Error::Forbidden(m)
            | Error::BadRequest(m)
            | Error::NotFound(m) => f.write_str(m),
            Error::Internal(e) => write!(f, "{e:#}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Internal(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

impl From<anyhow::Error> for Error {
    fn from(e: anyhow::Error) -> Error {
        Error::Internal(e)
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

fn unauthorized(why: impl Into<String>) -> Error {
    Error::Unauthorized(why.into())
}

/// What a session may do, named as T3 Code names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Scope {
    #[serde(rename = "access:read")]
    AccessRead,
    #[serde(rename = "access:write")]
    AccessWrite,
    #[serde(rename = "orchestration:read")]
    OrchestrationRead,
    #[serde(rename = "orchestration:operate")]
    OrchestrationOperate,
    #[serde(rename = "source-control:write")]
    SourceControlWrite,
    #[serde(rename = "filesystem:read")]
    FilesystemRead,
    #[serde(rename = "terminal:read")]
    TerminalRead,
    #[serde(rename = "terminal:operate")]
    TerminalOperate,
    #[serde(rename = "settings:write")]
    SettingsWrite,
    #[serde(rename = "providers:manage")]
    ProvidersManage,
}

impl Scope {
    pub const ALL: [Scope; 10] = [
        Scope::AccessRead,
        Scope::AccessWrite,
        Scope::OrchestrationRead,
        Scope::OrchestrationOperate,
        Scope::SourceControlWrite,
        Scope::FilesystemRead,
        Scope::TerminalRead,
        Scope::TerminalOperate,
        Scope::SettingsWrite,
        Scope::ProvidersManage,
    ];

    /// Every scope: what a pairing gives unless narrowed (as in T3).
    pub fn all() -> Vec<Scope> {
        Scope::ALL.to_vec()
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Scope::AccessRead => "access:read",
            Scope::AccessWrite => "access:write",
            Scope::OrchestrationRead => "orchestration:read",
            Scope::OrchestrationOperate => "orchestration:operate",
            Scope::SourceControlWrite => "source-control:write",
            Scope::FilesystemRead => "filesystem:read",
            Scope::TerminalRead => "terminal:read",
            Scope::TerminalOperate => "terminal:operate",
            Scope::SettingsWrite => "settings:write",
            Scope::ProvidersManage => "providers:manage",
        }
    }

    pub fn parse(word: &str) -> Option<Scope> {
        Scope::ALL.into_iter().find(|s| s.as_str() == word)
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Scopes in one order, each once.
fn normalize(scopes: &[Scope]) -> Vec<Scope> {
    let mut scopes = scopes.to_vec();
    scopes.sort();
    scopes.dedup();
    scopes
}

/// What of a request authentication reads.
#[derive(Debug, Clone, Copy, Default)]
pub struct Request<'a> {
    pub method: &'a str,
    /// Absolute, as the client used it: `scheme://host:port/path`. A query
    /// or fragment is ignored; `ws` is taken as `http`, `wss` as `https`.
    pub url: &'a str,
    /// The `Authorization` header.
    pub authorization: Option<&'a str>,
    /// The `DPoP` header (for `/ws`, the `dpop` query parameter).
    pub dpop: Option<&'a str>,
    /// The `Cookie` header.
    pub cookie: Option<&'a str>,
}

impl Request<'_> {
    fn https(&self) -> bool {
        let scheme = self.url.split("://").next().unwrap_or_default();
        scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("wss")
    }

    /// The `pansy_session` cookie's value.
    fn session_cookie(&self) -> Option<&str> {
        self.cookie?
            .split(';')
            .filter_map(|kv| kv.trim().split_once('='))
            .find(|(name, _)| *name == COOKIE)
            .map(|(_, value)| value.trim())
            .filter(|v| !v.is_empty())
    }

    /// The token of `Authorization: DPoP <token>`. Another scheme is an
    /// error, never a bearer taken without its proof.
    fn dpop_token(&self) -> Result<Option<&str>> {
        let Some(header) = self.authorization else {
            return Ok(None);
        };
        match header.trim().split_once(' ') {
            Some((scheme, token))
                if scheme.eq_ignore_ascii_case("DPoP") && !token.trim().is_empty() =>
            {
                Ok(Some(token.trim()))
            }
            _ => Err(unauthorized("only DPoP tokens are taken here")),
        }
    }
}

/// Who a request is from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    pub device: String,
    pub scopes: Vec<Scope>,
    /// False while the device waits to be approved: then it may only ask
    /// who it is (`GET /api/auth/me`).
    pub approved: bool,
    /// For a cookie session not used for a day: a `Set-Cookie` value that
    /// renews the cookie's `Max-Age` as the session was renewed.
    pub renew_cookie: Option<String>,
}

impl Principal {
    /// Approved, and given `scope`.
    pub fn allows(&self, scope: Scope) -> bool {
        self.approved && self.scopes.contains(&scope)
    }

    pub fn require(&self, scope: Scope) -> Result<()> {
        if !self.approved {
            return Err(Error::Forbidden(
                "this device waits to be approved on the computer".into(),
            ));
        }
        if !self.scopes.contains(&scope) {
            return Err(Error::Forbidden(format!(
                "this device was not given {scope}"
            )));
        }
        Ok(())
    }
}

/// A device just paired.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paired {
    pub device: String,
    /// DPoP only.
    pub access_token: Option<String>,
    /// The token's life, or the cookie session's.
    pub expires_in: u64,
    pub scopes: Vec<Scope>,
    pub approved: bool,
    /// The short code shown for approval ([`short_code`]).
    pub code: String,
    /// Cookie only: the `Set-Cookie` header's value.
    pub cookie: Option<String>,
}

impl Paired {
    /// The answer to `POST /api/auth/pair`.
    pub fn body(&self) -> Value {
        let mut body = json!({
            "deviceId": self.device,
            "expiresIn": self.expires_in,
            "scopes": self.scopes,
            "approval": if self.approved { Approval::Approved } else { Approval::Pending },
            "code": self.code,
        });
        if let Some(token) = &self.access_token {
            body["accessToken"] = json!(token);
        }
        body
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Approval {
    Approved,
    Pending,
}

/// What a device's session is bound to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Binding {
    /// The thumbprint of the device's P-256 key.
    Dpop {
        jkt: String,
    },
    Cookie,
}

/// A paired device, as the protocol's `Device` (the server adds `current`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub name: String,
    /// `ios`, `android` or `web`.
    pub platform: String,
    pub scopes: Vec<Scope>,
    pub approval: Approval,
    pub created_at: DateTime<Utc>,
    /// Its last use: the session dies 30 days after.
    pub last_seen: DateTime<Utc>,
    pub binding: Binding,
}

/// `remote.json`.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct State {
    /// The machine's id (`m_…`): the same across restarts and addresses.
    #[serde(default)]
    machine: String,
    #[serde(default)]
    require_approval: bool,
    #[serde(default)]
    pairings: Vec<Pairing>,
    #[serde(default)]
    devices: Vec<Stored>,
}

/// A pairing not used yet.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pairing {
    /// The secret's SHA-256.
    hash: String,
    scopes: Vec<Scope>,
    expires_at: DateTime<Utc>,
    /// The device that made it (`pair.create`): revoking it drops it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    created_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    #[serde(flatten)]
    device: Device,
    /// DPoP: the newest token's hash last, the one before kept until it
    /// expires so requests in flight while refreshing still pass. Only the
    /// newest refreshes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tokens: Vec<Token>,
    /// Cookie: the session secret's hash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session: Option<String>,
}

impl Stored {
    fn alive(&self, now: DateTime<Utc>) -> bool {
        now - self.device.last_seen < TimeDelta::days(SESSION_DAYS)
    }

    fn jkt(&self) -> Option<&str> {
        match &self.device.binding {
            Binding::Dpop { jkt } => Some(jkt),
            Binding::Cookie => None,
        }
    }

    fn code(&self) -> String {
        short_code(self.jkt().unwrap_or(&self.device.id))
    }

    fn principal(&self, renew_cookie: Option<String>) -> Principal {
        Principal {
            device: self.device.id.clone(),
            scopes: self.device.scopes.clone(),
            approved: self.device.approval == Approval::Approved,
            renew_cookie,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Token {
    hash: String,
    expires_at: DateTime<Utc>,
}

struct Ticket {
    device: String,
    jkt: Option<String>,
    expires_at: DateTime<Utc>,
}

struct Inner {
    state: State,
    /// `remote.json`'s modification time and length as last read or written.
    stamp: Option<(SystemTime, u64)>,
    /// When it was last written.
    saved: Option<DateTime<Utc>>,
    tickets: HashMap<String, Ticket>,
    /// `jkt:jti` of the proofs taken, and when.
    seen: HashMap<String, DateTime<Utc>>,
    /// Added to the clock (tests).
    offset: TimeDelta,
}

impl Inner {
    fn now(&self) -> DateTime<Utc> {
        Utc::now() + self.offset
    }

    /// Takes a proof's `jti` once in five minutes.
    fn replay(&mut self, proof: &Proof, now: DateTime<Utc>) -> Result<()> {
        self.seen
            .retain(|_, at| now - *at < TimeDelta::seconds(REPLAY_SECS));
        let key = format!("{}:{}", proof.jkt, proof.jti);
        if self.seen.contains_key(&key) {
            return Err(unauthorized("this DPoP proof was used already"));
        }
        self.seen.insert(key, now);
        Ok(())
    }

    fn device(&self, id: &str) -> Option<usize> {
        self.state.devices.iter().position(|d| d.device.id == id)
    }
}

/// The devices paired with this machine and their sessions.
pub struct Auth {
    path: PathBuf,
    inner: Mutex<Inner>,
}

impl Auth {
    /// `remote.json` in pansy's directory, made (0600, with a new machine
    /// id) on first use.
    pub fn load() -> Result<Auth> {
        Auth::load_at(paths::home().join("remote.json"))
    }

    pub fn load_at(path: impl Into<PathBuf>) -> Result<Auth> {
        let path = path.into();
        let state: State = fsutil::read_json(&path)?;
        let auth = Auth {
            inner: Mutex::new(Inner {
                state,
                stamp: stamp(&path),
                saved: None,
                tickets: HashMap::new(),
                seen: HashMap::new(),
                offset: TimeDelta::zero(),
            }),
            path,
        };
        {
            let mut inner = auth.lock();
            if inner.state.machine.is_empty() {
                inner.state.machine = format!("m_{}", hex(&random::<8>()));
                auth.save(&mut inner)?;
            }
        }
        Ok(auth)
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Reads `remote.json` again when another process changed it (or always,
    /// `force`, before a change), keeping the later of each device's
    /// `last_seen` (ours are written at most once a minute).
    fn sync(&self, inner: &mut Inner, force: bool) -> Result<()> {
        let now_stamp = stamp(&self.path);
        if !force && now_stamp == inner.stamp {
            return Ok(());
        }
        let mut disk: State = fsutil::read_json(&self.path)?;
        for d in &mut disk.devices {
            if let Some(i) = inner.device(&d.device.id) {
                let ours = inner.state.devices[i].device.last_seen;
                d.device.last_seen = d.device.last_seen.max(ours);
            }
        }
        if disk.machine.is_empty() {
            disk.machine = std::mem::take(&mut inner.state.machine);
        }
        inner.state = disk;
        inner.stamp = now_stamp;
        Ok(())
    }

    /// [`Auth::sync`] for what only reads: a file that cannot be read keeps
    /// what was read before.
    fn fresh(&self) -> MutexGuard<'_, Inner> {
        let mut inner = self.lock();
        if let Err(e) = self.sync(&mut inner, false) {
            tracing::warn!("reading {}: {e}", self.path.display());
        }
        inner
    }

    /// Writes `remote.json` (0600), without what has expired.
    fn save(&self, inner: &mut Inner) -> Result<()> {
        let now = inner.now();
        inner.state.pairings.retain(|p| p.expires_at > now);
        inner.state.devices.retain(|d| d.alive(now));
        fsutil::write_json(&self.path, &inner.state, true)?;
        inner.stamp = stamp(&self.path);
        inner.saved = Some(now);
        Ok(())
    }

    pub fn machine_id(&self) -> String {
        self.lock().state.machine.clone()
    }

    /// Whether a new device waits to be approved on the computer (default
    /// off, as in T3).
    pub fn require_approval(&self) -> bool {
        self.fresh().state.require_approval
    }

    /// Devices paired before it is turned on stay approved; turning it off
    /// leaves those waiting to be approved by hand.
    pub fn set_require_approval(&self, on: bool) -> Result<()> {
        let mut inner = self.lock();
        self.sync(&mut inner, true)?;
        inner.state.require_approval = on;
        self.save(&mut inner)
    }

    /// A pairing secret, from the computer (`pansy pair`, the window): every
    /// scope unless `scopes` narrows them. Shown once; only its hash is kept.
    pub fn create_pairing(
        &self,
        scopes: Option<&[Scope]>,
        ttl: Duration,
    ) -> Result<(String, DateTime<Utc>)> {
        self.pairing(None, scopes.map(normalize).unwrap_or_else(Scope::all), ttl)
    }

    /// A pairing secret asked for by a device (`pair.create`): it needs
    /// `access:write` and every scope it gives; it gives its own unless
    /// `scopes` narrows them. Revoking the device drops the pairing.
    pub fn create_pairing_by(
        &self,
        by: &Principal,
        scopes: Option<&[Scope]>,
        ttl: Duration,
    ) -> Result<(String, DateTime<Utc>)> {
        by.require(Scope::AccessWrite)?;
        let scopes = normalize(scopes.unwrap_or(&by.scopes));
        if let Some(s) = scopes.iter().find(|s| !by.scopes.contains(s)) {
            return Err(Error::Forbidden(format!(
                "a device can give only scopes it has, and it lacks {s}"
            )));
        }
        self.pairing(Some(by.device.clone()), scopes, ttl)
    }

    fn pairing(
        &self,
        by: Option<String>,
        scopes: Vec<Scope>,
        ttl: Duration,
    ) -> Result<(String, DateTime<Utc>)> {
        if scopes.is_empty() {
            return Err(Error::BadRequest("a pairing needs a scope".into()));
        }
        let ttl = TimeDelta::from_std(ttl)
            .map_err(|_| Error::BadRequest("a pairing's life is too long".into()))?;
        let mut inner = self.lock();
        self.sync(&mut inner, true)?;
        let expires_at = inner.now() + ttl;
        let secret = fresh_secret();
        inner.state.pairings.push(Pairing {
            hash: hash(&secret),
            scopes,
            expires_at,
            created_by: by,
        });
        self.save(&mut inner)?;
        Ok((secret, expires_at))
    }

    /// `POST /api/auth/pair`. With `cookie`, a browser session; else the
    /// request's DPoP proof (without `ath`) binds the device to its key. The
    /// secret is used up only when the device is paired.
    pub fn pair(
        &self,
        req: &Request,
        secret: &str,
        device_name: &str,
        platform: &str,
        cookie: bool,
    ) -> Result<Paired> {
        let name: String = device_name.trim().chars().take(64).collect();
        if name.is_empty() {
            return Err(Error::BadRequest("a device needs a name".into()));
        }
        if !matches!(platform, "ios" | "android" | "web") {
            return Err(Error::BadRequest(format!(
                "a platform is ios, android or web, not {platform:?}"
            )));
        }
        let mut inner = self.lock();
        self.sync(&mut inner, true)?;
        let now = inner.now();
        let hashed = hash(secret.trim());
        let at = inner
            .state
            .pairings
            .iter()
            .position(|p| p.hash == hashed && p.expires_at > now)
            .ok_or_else(|| {
                unauthorized("this pairing link was used, has expired, or is not this machine's")
            })?;
        let binding = if cookie {
            Binding::Cookie
        } else {
            let proof = req
                .dpop
                .ok_or_else(|| unauthorized("pairing without a cookie needs a DPoP proof"))?;
            let proof = check_proof(proof, req.method, req.url, None, now)?;
            inner.replay(&proof, now)?;
            Binding::Dpop { jkt: proof.jkt }
        };
        let pairing = inner.state.pairings.remove(at);
        let approved = !inner.state.require_approval;
        let mut stored = Stored {
            device: Device {
                id: format!("d_{}", hex(&random::<8>())),
                name,
                platform: platform.to_owned(),
                scopes: pairing.scopes,
                approval: if approved {
                    Approval::Approved
                } else {
                    Approval::Pending
                },
                created_at: now,
                last_seen: now,
                binding,
            },
            tokens: Vec::new(),
            session: None,
        };
        let (access_token, set_cookie, expires_in) = if cookie {
            let value = fresh_secret();
            stored.session = Some(hash(&value));
            (
                None,
                Some(cookie_header(&value, req.https())),
                SESSION_MAX_AGE,
            )
        } else {
            let token = fresh_secret();
            stored.tokens.push(new_token(&token, now));
            (Some(token), None, TOKEN_TTL)
        };
        let paired = Paired {
            device: stored.device.id.clone(),
            access_token,
            expires_in,
            scopes: stored.device.scopes.clone(),
            approved,
            code: stored.code(),
            cookie: set_cookie,
        };
        inner.state.devices.push(stored);
        self.save(&mut inner)?;
        Ok(paired)
    }

    /// `POST /api/auth/refresh`: `Authorization: DPoP <token>` with the
    /// device's newest token (expired or not) and a proof by its key (with
    /// `ath`). A new token, and the session renewed.
    pub fn refresh(&self, req: &Request) -> Result<(String, u64)> {
        let token = req
            .dpop_token()?
            .ok_or_else(|| unauthorized("refreshing needs Authorization: DPoP <token>"))?;
        let proof = req
            .dpop
            .ok_or_else(|| unauthorized("refreshing needs a DPoP proof"))?;
        let mut inner = self.lock();
        self.sync(&mut inner, true)?;
        let now = inner.now();
        let proof = check_proof(proof, req.method, req.url, Some(token), now)?;
        let hashed = hash(token);
        let i = inner
            .state
            .devices
            .iter()
            .position(|d| d.tokens.last().is_some_and(|t| t.hash == hashed))
            .ok_or_else(|| {
                unauthorized("this token was replaced, revoked, or is not this machine's")
            })?;
        let stored = &inner.state.devices[i];
        if !stored.alive(now) {
            return Err(unauthorized("this device's session ended: pair it again"));
        }
        if stored.jkt() != Some(&proof.jkt) {
            return Err(unauthorized("the proof is not signed by this device's key"));
        }
        inner.replay(&proof, now)?;
        let fresh = fresh_secret();
        let stored = &mut inner.state.devices[i];
        stored.tokens.push(new_token(&fresh, now));
        let extra = stored.tokens.len().saturating_sub(2);
        stored.tokens.drain(..extra);
        stored.device.last_seen = now;
        self.save(&mut inner)?;
        Ok((fresh, TOKEN_TTL))
    }

    /// Who sent `req`: a DPoP token with its proof (`ath` required), or the
    /// session cookie. A device waiting for approval comes back with
    /// `approved` false.
    pub fn authenticate(&self, req: &Request) -> Result<Principal> {
        let mut inner = self.lock();
        self.sync(&mut inner, false)?;
        let now = inner.now();
        if let Some(token) = req.dpop_token()? {
            let proof = req
                .dpop
                .ok_or_else(|| unauthorized("a DPoP token needs a DPoP proof"))?;
            let proof = check_proof(proof, req.method, req.url, Some(token), now)?;
            let hashed = hash(token);
            let i = inner
                .state
                .devices
                .iter()
                .position(|d| {
                    d.tokens
                        .iter()
                        .any(|t| t.hash == hashed && t.expires_at > now)
                })
                .ok_or_else(|| {
                    unauthorized("this token has expired, was revoked, or is not this machine's")
                })?;
            let stored = &inner.state.devices[i];
            if !stored.alive(now) {
                return Err(unauthorized("this device's session ended: pair it again"));
            }
            if stored.jkt() != Some(&proof.jkt) {
                return Err(unauthorized("the proof is not signed by this device's key"));
            }
            inner.replay(&proof, now)?;
            Ok(self.touch(&mut inner, i, now, None))
        } else if let Some(value) = req.session_cookie() {
            let hashed = hash(value);
            let i = inner
                .state
                .devices
                .iter()
                .position(|d| d.session.as_deref() == Some(&hashed))
                .ok_or_else(|| {
                    unauthorized("this browser's session was revoked or is not this machine's")
                })?;
            let stored = &inner.state.devices[i];
            if !stored.alive(now) {
                return Err(unauthorized("this browser's session ended: pair it again"));
            }
            let renew = (now - stored.device.last_seen > TimeDelta::seconds(RENEW_COOKIE_SECS))
                .then(|| cookie_header(value, req.https()));
            Ok(self.touch(&mut inner, i, now, renew))
        } else {
            Err(unauthorized("pair this device first"))
        }
    }

    /// Marks device `i` seen (renewing its session), writing that at most
    /// once a minute.
    fn touch(
        &self,
        inner: &mut Inner,
        i: usize,
        now: DateTime<Utc>,
        renew_cookie: Option<String>,
    ) -> Principal {
        inner.state.devices[i].device.last_seen = now;
        let principal = inner.state.devices[i].principal(renew_cookie);
        let due = inner
            .saved
            .is_none_or(|at| now - at >= TimeDelta::seconds(TOUCH_SECS));
        if due && let Err(e) = self.save(inner) {
            tracing::warn!("writing {}: {e}", self.path.display());
        }
        principal
    }

    /// A ticket for `GET /ws?ticket=…`: 30 seconds, once, in memory only,
    /// bound to the device and its key.
    pub fn ws_ticket(&self, p: &Principal) -> String {
        let mut inner = self.lock();
        let now = inner.now();
        let jkt = inner
            .device(&p.device)
            .and_then(|i| inner.state.devices[i].jkt().map(str::to_owned));
        inner.tickets.retain(|_, t| t.expires_at > now);
        let ticket = fresh_secret();
        inner.tickets.insert(
            ticket.clone(),
            Ticket {
                device: p.device.clone(),
                jkt,
                expires_at: now + TimeDelta::seconds(TICKET_SECS),
            },
        );
        ticket
    }

    /// The WebSocket's upgrade. A DPoP device's request carries a proof by
    /// its key (`req.dpop`, from `&dpop=`) for `GET` of the `/ws` URL, no
    /// `ath`. The ticket is used up whatever the outcome.
    pub fn redeem_ticket(&self, ticket: &str, req: &Request) -> Result<Principal> {
        let mut inner = self.lock();
        self.sync(&mut inner, false)?;
        let now = inner.now();
        inner.tickets.retain(|_, t| t.expires_at > now);
        let ticket = inner.tickets.remove(ticket).ok_or_else(|| {
            unauthorized("this ticket was used, has expired, or is not this machine's")
        })?;
        let i = inner
            .device(&ticket.device)
            .ok_or_else(|| unauthorized("this device was revoked"))?;
        let stored = &inner.state.devices[i];
        if !stored.alive(now) {
            return Err(unauthorized("this device's session ended: pair it again"));
        }
        if let Some(jkt) = stored.jkt() {
            let proof = req
                .dpop
                .ok_or_else(|| unauthorized("this device's WebSocket needs a DPoP proof"))?;
            let proof = check_proof(proof, req.method, req.url, None, now)?;
            if proof.jkt != jkt || ticket.jkt.as_deref() != Some(jkt) {
                return Err(unauthorized("the proof is not signed by this device's key"));
            }
            inner.replay(&proof, now)?;
        }
        Ok(self.touch(&mut inner, i, now, None))
    }

    /// The devices whose sessions are alive, oldest first.
    pub fn devices(&self) -> Vec<Device> {
        let inner = self.fresh();
        let now = inner.now();
        inner
            .state
            .devices
            .iter()
            .filter(|d| d.alive(now))
            .map(|d| d.device.clone())
            .collect()
    }

    pub fn device(&self, id: &str) -> Option<Device> {
        self.devices().into_iter().find(|d| d.id == id)
    }

    pub fn approve(&self, device: &str) -> Result<()> {
        let mut inner = self.lock();
        self.sync(&mut inner, true)?;
        let now = inner.now();
        let i = inner
            .device(device)
            .filter(|&i| inner.state.devices[i].alive(now))
            .ok_or_else(|| Error::NotFound(format!("no device {device}")))?;
        inner.state.devices[i].device.approval = Approval::Approved;
        self.save(&mut inner)
    }

    /// Forgets the device: its tokens, cookie, tickets and the pairings it
    /// made die with it. The server closes its sockets.
    pub fn revoke(&self, device: &str) -> Result<()> {
        let mut inner = self.lock();
        self.sync(&mut inner, true)?;
        let i = inner
            .device(device)
            .ok_or_else(|| Error::NotFound(format!("no device {device}")))?;
        inner.state.devices.remove(i);
        inner
            .state
            .pairings
            .retain(|p| p.created_by.as_deref() != Some(device));
        inner.tickets.retain(|_, t| t.device != device);
        self.save(&mut inner)
    }

    /// The short code to compare when approving ([`short_code`]).
    pub fn code_for(&self, device: &str) -> Option<String> {
        let inner = self.fresh();
        inner.device(device).map(|i| inner.state.devices[i].code())
    }

    #[cfg(test)]
    fn advance(&self, by: TimeDelta) {
        self.lock().offset += by;
    }

    #[cfg(test)]
    fn now(&self) -> DateTime<Utc> {
        self.lock().now()
    }
}

/// The code a person compares on both screens: SHA-256 of the key's
/// thumbprint (a cookie session's device id), its first 30 bits in
/// Crockford's base32, as `K7Q-42M`. The phone works it out the same way
/// from its own `jkt`.
pub fn short_code(of: &str) -> String {
    const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let digest = Sha256::digest(of.as_bytes());
    let bits = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) >> 2;
    let mut code = String::with_capacity(7);
    for i in 0..6 {
        if i == 3 {
            code.push('-');
        }
        code.push(ALPHABET[((bits >> (25 - 5 * i)) & 31) as usize] as char);
    }
    code
}

/// A proof that checked out.
#[derive(Debug)]
struct Proof {
    jkt: String,
    jti: String,
}

#[derive(Deserialize)]
struct Header {
    typ: String,
    alg: String,
    jwk: Jwk,
}

#[derive(Deserialize)]
struct Jwk {
    kty: String,
    crv: String,
    x: String,
    y: String,
    /// A private key has no place in a proof.
    #[serde(default)]
    d: Option<Value>,
}

#[derive(Deserialize)]
struct Claims {
    jti: String,
    htm: String,
    htu: String,
    iat: i64,
    #[serde(default)]
    ath: Option<String>,
}

/// Checks a DPoP proof for a request (`method`, `url`) and returns its key's
/// thumbprint (RFC 7638). ES256 only; `typ` `dpop+jwt`; `iat` within 60
/// seconds of `now`; `htu` compared without query or fragment, scheme and
/// host in any case; `ath` the token's hash when `access_token` is given.
/// Replays are caught by [`Auth`], which remembers each `jti`.
pub fn verify_dpop(
    proof: &str,
    method: &str,
    url: &str,
    access_token: Option<&str>,
    now: DateTime<Utc>,
) -> Result<String> {
    check_proof(proof, method, url, access_token, now).map(|p| p.jkt)
}

fn check_proof(
    proof: &str,
    method: &str,
    url: &str,
    access_token: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Proof> {
    let bad = |why: &str| unauthorized(format!("DPoP proof: {why}"));
    if proof.len() > 4096 {
        return Err(bad("too long"));
    }
    let mut parts = proof.trim().split('.');
    let (Some(head), Some(body), Some(sig), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(bad("not a JWT"));
    };
    let header: Header = decode_json(head).ok_or_else(|| bad("its header is not readable"))?;
    let claims: Claims = decode_json(body).ok_or_else(|| bad("its claims are not readable"))?;
    if header.typ != "dpop+jwt" {
        return Err(bad("typ is not dpop+jwt"));
    }
    if header.alg != "ES256" {
        return Err(bad("only ES256 is taken"));
    }
    let jwk = header.jwk;
    if jwk.kty != "EC" || jwk.crv != "P-256" || jwk.d.is_some() {
        return Err(bad("its key is not a public P-256 key"));
    }
    let coordinate = |c: &str| B64.decode(c).ok().filter(|b| b.len() == 32);
    let (Some(x), Some(y)) = (coordinate(&jwk.x), coordinate(&jwk.y)) else {
        return Err(bad("its key is not a public P-256 key"));
    };
    let point = p256::EncodedPoint::from_affine_coordinates(
        p256::FieldBytes::from_slice(&x),
        p256::FieldBytes::from_slice(&y),
        false,
    );
    let key =
        VerifyingKey::from_encoded_point(&point).map_err(|_| bad("its key is not on P-256"))?;
    let signature = B64
        .decode(sig)
        .ok()
        .and_then(|s| Signature::from_slice(&s).ok())
        .ok_or_else(|| bad("its signature is not readable"))?;
    key.verify(format!("{head}.{body}").as_bytes(), &signature)
        .map_err(|_| bad("its signature does not verify"))?;
    if !claims.htm.eq_ignore_ascii_case(method) {
        return Err(bad("htm is not this request's method"));
    }
    match (htu(&claims.htu), htu(url)) {
        (Some(a), Some(b)) if a == b => {}
        _ => return Err(bad("htu is not this request's URL")),
    }
    if (now.timestamp() - claims.iat).abs() > SKEW_SECS {
        return Err(bad("iat is too far from now"));
    }
    if let Some(token) = access_token
        && claims.ath.as_deref() != Some(&B64.encode(Sha256::digest(token.as_bytes())))
    {
        return Err(bad("ath is not this token's hash"));
    }
    if claims.jti.is_empty() || claims.jti.len() > 128 {
        return Err(bad("jti is missing or too long"));
    }
    // x and y are base64url, so nothing in them needs escaping.
    let thumbprint = format!(
        r#"{{"crv":"P-256","kty":"EC","x":"{}","y":"{}"}}"#,
        jwk.x, jwk.y
    );
    Ok(Proof {
        jkt: B64.encode(Sha256::digest(thumbprint.as_bytes())),
        jti: claims.jti,
    })
}

fn decode_json<T: serde::de::DeserializeOwned>(part: &str) -> Option<T> {
    serde_json::from_slice(&B64.decode(part).ok()?).ok()
}

/// A URL as `htu` is compared: no query or fragment; scheme and host in
/// lower case, `ws` as `http` and `wss` as `https`; no default port; `/`
/// for an empty path.
fn htu(url: &str) -> Option<String> {
    let url = url.split('#').next()?.split('?').next()?;
    let (scheme, rest) = url.split_once("://")?;
    let scheme = match scheme.to_ascii_lowercase().as_str() {
        "http" | "ws" => "http",
        "https" | "wss" => "https",
        _ => return None,
    };
    let (host, path) = match rest.find('/') {
        Some(at) => rest.split_at(at),
        None => (rest, "/"),
    };
    let host = host.to_ascii_lowercase();
    if host.is_empty() || host.contains('@') {
        return None;
    }
    let default = if scheme == "http" { ":80" } else { ":443" };
    let host = host.strip_suffix(default).unwrap_or(&host);
    Some(format!("{scheme}://{host}{path}"))
}

fn cookie_header(value: &str, secure: bool) -> String {
    let mut cookie =
        format!("{COOKIE}={value}; HttpOnly; SameSite=Strict; Path=/; Max-Age={SESSION_MAX_AGE}");
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

fn new_token(token: &str, now: DateTime<Utc>) -> Token {
    Token {
        hash: hash(token),
        expires_at: now + TimeDelta::seconds(TOKEN_TTL as i64),
    }
}

fn stamp(path: &std::path::Path) -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0; N];
    getrandom::getrandom(&mut bytes).expect("the system's random numbers");
    bytes
}

/// 32 random bytes in base64url: pairing secrets, tokens, cookies, tickets.
fn fresh_secret() -> String {
    B64.encode(random::<32>())
}

/// What `remote.json` keeps of a secret.
fn hash(secret: &str) -> String {
    B64.encode(Sha256::digest(secret.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::SigningKey;
    use p256::ecdsa::signature::Signer;

    const API: &str = "http://192.168.1.20:7268/api/auth/pair";
    const ME: &str = "http://192.168.1.20:7268/api/auth/me";
    const REFRESH: &str = "http://192.168.1.20:7268/api/auth/refresh";
    const WS: &str = "ws://192.168.1.20:7268/ws";

    /// A device's key, signing proofs as the phone does.
    struct Key(SigningKey);

    impl Key {
        fn new() -> Key {
            Key(SigningKey::from_slice(&random::<32>()).unwrap())
        }

        fn jwk(&self) -> Value {
            let point = self.0.verifying_key().to_encoded_point(false);
            json!({
                "kty": "EC", "crv": "P-256",
                "x": B64.encode(point.x().unwrap()), "y": B64.encode(point.y().unwrap()),
            })
        }

        fn sign(&self, header: &Value, claims: &Value) -> String {
            let head = B64.encode(header.to_string());
            let body = B64.encode(claims.to_string());
            let sig: Signature = self.0.sign(format!("{head}.{body}").as_bytes());
            format!("{head}.{body}.{}", B64.encode(sig.to_bytes()))
        }

        fn claims(method: &str, url: &str, token: Option<&str>, iat: i64) -> Value {
            let mut claims = json!({"jti": fresh_secret(), "htm": method, "htu": url, "iat": iat});
            if let Some(token) = token {
                claims["ath"] = json!(B64.encode(Sha256::digest(token.as_bytes())));
            }
            claims
        }

        fn proof_at(&self, method: &str, url: &str, token: Option<&str>, iat: i64) -> String {
            let header = json!({"typ": "dpop+jwt", "alg": "ES256", "jwk": self.jwk()});
            self.sign(&header, &Key::claims(method, url, token, iat))
        }

        fn proof(&self, auth: &Auth, method: &str, url: &str, token: Option<&str>) -> String {
            self.proof_at(method, url, token, auth.now().timestamp())
        }

        fn jkt(&self) -> String {
            verify_dpop(
                &self.proof_at("GET", ME, None, Utc::now().timestamp()),
                "GET",
                ME,
                None,
                Utc::now(),
            )
            .unwrap()
        }
    }

    fn scratch() -> (tempfile::TempDir, Auth) {
        let dir = tempfile::tempdir().unwrap();
        let auth = Auth::load_at(dir.path().join("remote.json")).unwrap();
        (dir, auth)
    }

    fn pair_dpop(auth: &Auth, key: &Key, scopes: Option<&[Scope]>) -> Paired {
        let (secret, _) = auth.create_pairing(scopes, PAIRING_TTL).unwrap();
        let proof = key.proof(auth, "POST", API, None);
        let req = Request {
            method: "POST",
            url: API,
            dpop: Some(&proof),
            ..Request::default()
        };
        auth.pair(&req, &secret, "iPhone", "ios", false).unwrap()
    }

    fn me(auth: &Auth, key: &Key, token: &str) -> Result<Principal> {
        let authorization = format!("DPoP {token}");
        let proof = key.proof(auth, "GET", ME, Some(token));
        auth.authenticate(&Request {
            method: "GET",
            url: ME,
            authorization: Some(&authorization),
            dpop: Some(&proof),
            cookie: None,
        })
    }

    fn refresh(auth: &Auth, key: &Key, token: &str) -> Result<(String, u64)> {
        let authorization = format!("DPoP {token}");
        let proof = key.proof(auth, "POST", REFRESH, Some(token));
        auth.refresh(&Request {
            method: "POST",
            url: REFRESH,
            authorization: Some(&authorization),
            dpop: Some(&proof),
            cookie: None,
        })
    }

    fn unauthorized(r: Result<impl fmt::Debug>) -> String {
        match r {
            Err(Error::Unauthorized(why)) => why,
            other => panic!("expected unauthorized, got {other:?}"),
        }
    }

    #[test]
    fn pairs_with_dpop_and_keeps_no_secret_on_disk() {
        let (dir, auth) = scratch();
        assert!(auth.machine_id().starts_with("m_"));
        let key = Key::new();
        let (secret, expires) = auth.create_pairing(None, PAIRING_TTL).unwrap();
        assert!(expires > Utc::now() + TimeDelta::minutes(9));
        let proof = key.proof(&auth, "POST", API, None);
        let req = Request {
            method: "POST",
            url: API,
            dpop: Some(&proof),
            ..Request::default()
        };
        let paired = auth
            .pair(&req, &secret, "  iPhone  ", "ios", false)
            .unwrap();
        let token = paired.access_token.clone().unwrap();
        assert!(paired.device.starts_with("d_"));
        assert_eq!(paired.expires_in, 3600);
        assert_eq!(paired.scopes, Scope::all());
        assert!(paired.approved && paired.cookie.is_none());
        assert_eq!(paired.code, short_code(&key.jkt()));
        assert_eq!(auth.code_for(&paired.device).unwrap(), paired.code);
        let code = paired.code.as_bytes();
        assert_eq!((code.len(), code[3]), (7, b'-'));
        let body = paired.body();
        assert_eq!(body["approval"], "approved");
        assert_eq!(body["accessToken"], token.as_str());
        assert_eq!(body["scopes"][0], "access:read");

        let p = me(&auth, &key, &token).unwrap();
        assert_eq!(p.device, paired.device);
        assert!(p.allows(Scope::ProvidersManage));
        let devices = auth.devices();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, "iPhone");
        assert_eq!(devices[0].binding, Binding::Dpop { jkt: key.jkt() });

        // The same proof twice, a proof by another key, no proof, a bearer.
        let authorization = format!("DPoP {token}");
        let proof = key.proof(&auth, "GET", ME, Some(&token));
        let req = Request {
            method: "GET",
            url: ME,
            authorization: Some(&authorization),
            dpop: Some(&proof),
            cookie: None,
        };
        auth.authenticate(&req).unwrap();
        assert!(unauthorized(auth.authenticate(&req)).contains("used already"));
        assert!(unauthorized(me(&auth, &Key::new(), &token)).contains("key"));
        let no_proof = Request { dpop: None, ..req };
        unauthorized(auth.authenticate(&no_proof));
        let bearer = format!("Bearer {token}");
        let proof = key.proof(&auth, "GET", ME, Some(&token));
        let req = Request {
            authorization: Some(&bearer),
            dpop: Some(&proof),
            ..req
        };
        assert!(unauthorized(auth.authenticate(&req)).contains("only DPoP"));
        // A token needs ath.
        let proof = key.proof(&auth, "GET", ME, None);
        let req = Request {
            authorization: Some(&authorization),
            dpop: Some(&proof),
            ..req
        };
        assert!(unauthorized(auth.authenticate(&req)).contains("ath"));
        unauthorized(auth.authenticate(&Request {
            method: "GET",
            url: ME,
            ..Request::default()
        }));

        let path = dir.path().join("remote.json");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains(&secret) && !text.contains(&token));
        assert!(text.contains(&hash(&token)));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // Read again, the machine and the device are the same.
        let again = Auth::load_at(&path).unwrap();
        assert_eq!(again.machine_id(), auth.machine_id());
        assert!(me(&again, &key, &token).is_ok());
    }

    #[test]
    fn pairs_a_browser_with_a_cookie() {
        let (dir, auth) = scratch();
        let (secret, _) = auth.create_pairing(None, PAIRING_TTL).unwrap();
        let req = Request {
            method: "POST",
            url: API,
            ..Request::default()
        };
        let paired = auth.pair(&req, &secret, "Firefox", "web", true).unwrap();
        assert!(paired.access_token.is_none() && paired.body().get("accessToken").is_none());
        assert_eq!(paired.code, short_code(&paired.device));
        let set = paired.cookie.unwrap();
        assert!(
            set.ends_with("; HttpOnly; SameSite=Strict; Path=/; Max-Age=2592000"),
            "{set}"
        );
        let value = set
            .split(';')
            .next()
            .unwrap()
            .strip_prefix("pansy_session=")
            .unwrap()
            .to_owned();
        assert_eq!(auth.devices()[0].binding, Binding::Cookie);

        let cookie = format!("theme=dark; pansy_session={value}");
        let req = Request {
            method: "GET",
            url: ME,
            cookie: Some(&cookie),
            ..Request::default()
        };
        let p = auth.authenticate(&req).unwrap();
        assert_eq!(p.device, paired.device);
        assert!(p.renew_cookie.is_none());
        let wrong = Request {
            cookie: Some("pansy_session=nope"),
            ..req
        };
        unauthorized(auth.authenticate(&wrong));
        // Used again after two days: the session and the cookie renewed.
        auth.advance(TimeDelta::days(2));
        let renewed = auth.authenticate(&req).unwrap().renew_cookie.unwrap();
        assert!(renewed.starts_with(&format!("pansy_session={value};")));
        // Thirty days unused: gone.
        auth.advance(TimeDelta::days(31));
        assert!(unauthorized(auth.authenticate(&req)).contains("ended"));
        assert!(auth.devices().is_empty());

        // Over HTTPS the cookie is Secure.
        let (secret, _) = auth.create_pairing(None, PAIRING_TTL).unwrap();
        let req = Request {
            method: "POST",
            url: "https://pansy.example.com/api/auth/pair",
            ..Request::default()
        };
        let set = auth
            .pair(&req, &secret, "Safari", "web", true)
            .unwrap()
            .cookie
            .unwrap();
        assert!(set.ends_with("; Secure"));
        let text = std::fs::read_to_string(dir.path().join("remote.json")).unwrap();
        assert!(!text.contains(&value) && !text.contains(&secret));
    }

    #[test]
    fn a_pairing_secret_is_used_once_and_expires() {
        let (_dir, auth) = scratch();
        let key = Key::new();
        let (secret, _) = auth.create_pairing(None, PAIRING_TTL).unwrap();
        let pair = |secret: &str, proof: Option<&str>| {
            let req = Request {
                method: "POST",
                url: API,
                dpop: proof,
                ..Request::default()
            };
            auth.pair(&req, secret, "Pixel", "android", false)
        };
        // A bad proof, or none, leaves the secret as it was.
        unauthorized(pair(&secret, None));
        let wrong = key.proof(&auth, "POST", "http://192.168.1.20:7268/api/other", None);
        unauthorized(pair(&secret, Some(&wrong)));
        unauthorized(pair(
            "not-the-secret",
            Some(&key.proof(&auth, "POST", API, None)),
        ));
        pair(&secret, Some(&key.proof(&auth, "POST", API, None))).unwrap();
        assert!(
            unauthorized(pair(&secret, Some(&key.proof(&auth, "POST", API, None))))
                .contains("used")
        );

        let (secret, _) = auth.create_pairing(None, PAIRING_TTL).unwrap();
        auth.advance(TimeDelta::minutes(11));
        unauthorized(pair(&secret, Some(&key.proof(&auth, "POST", API, None))));

        let (secret, _) = auth.create_pairing(None, PAIRING_TTL).unwrap();
        let proof = key.proof(&auth, "POST", API, None);
        let req = Request {
            method: "POST",
            url: API,
            dpop: Some(&proof),
            ..Request::default()
        };
        assert!(matches!(
            auth.pair(&req, &secret, " ", "ios", false),
            Err(Error::BadRequest(_))
        ));
        assert!(matches!(
            auth.pair(&req, &secret, "Mac", "macos", false),
            Err(Error::BadRequest(_))
        ));
    }

    #[test]
    fn bad_proofs_are_refused() {
        let key = Key::new();
        let now = Utc::now();
        let t = now.timestamp();
        let check = |proof: &str| verify_dpop(proof, "GET", ME, None, now);
        let good = key.proof_at("GET", ME, None, t);
        assert_eq!(check(&good).unwrap(), key.jkt());
        // The same URL written otherwise: case, default port, query, ws.
        assert!(
            verify_dpop(
                &good,
                "get",
                "HTTP://192.168.1.20:7268/api/auth/me?x=1#y",
                None,
                now
            )
            .is_ok()
        );
        let p = key.proof_at("GET", "https://Pansy.Example.com:443/ws?ticket=1", None, t);
        assert!(verify_dpop(&p, "GET", "wss://pansy.example.com/ws", None, now).is_ok());

        let why = |r: Result<String>| unauthorized(r);
        assert!(why(verify_dpop(&good, "POST", ME, None, now)).contains("htm"));
        assert!(
            why(verify_dpop(
                &good,
                "GET",
                "http://192.168.1.20:7268/api/auth/mee",
                None,
                now
            ))
            .contains("htu")
        );
        assert!(
            why(verify_dpop(
                &good,
                "GET",
                "http://192.168.1.21:7268/api/auth/me",
                None,
                now
            ))
            .contains("htu")
        );
        assert!(
            why(verify_dpop(
                &good,
                "GET",
                "https://192.168.1.20:7268/api/auth/me",
                None,
                now
            ))
            .contains("htu")
        );
        assert!(why(check(&key.proof_at("GET", ME, None, t - 61))).contains("iat"));
        assert!(why(check(&key.proof_at("GET", ME, None, t + 61))).contains("iat"));
        assert!(check(&key.proof_at("GET", ME, None, t - 59)).is_ok());
        assert!(why(verify_dpop(&good, "GET", ME, Some("token"), now)).contains("ath"));
        let with_ath = key.proof_at("GET", ME, Some("token"), t);
        assert!(verify_dpop(&with_ath, "GET", ME, Some("token"), now).is_ok());
        assert!(why(verify_dpop(&with_ath, "GET", ME, Some("other"), now)).contains("ath"));

        // Claims changed after signing.
        let parts: Vec<&str> = good.split('.').collect();
        let mut claims: Value = decode_json(parts[1]).unwrap();
        claims["htm"] = json!("POST");
        let tampered = format!(
            "{}.{}.{}",
            parts[0],
            B64.encode(claims.to_string()),
            parts[2]
        );
        assert!(why(verify_dpop(&tampered, "POST", ME, None, now)).contains("signature"));
        // Signed by one key, claiming another.
        let other = Key::new();
        let header = json!({"typ": "dpop+jwt", "alg": "ES256", "jwk": other.jwk()});
        let lying = key.sign(&header, &Key::claims("GET", ME, None, t));
        assert!(why(check(&lying)).contains("signature"));
        // Header fields.
        let claims = Key::claims("GET", ME, None, t);
        let header = json!({"typ": "jwt", "alg": "ES256", "jwk": key.jwk()});
        assert!(why(check(&key.sign(&header, &claims))).contains("typ"));
        let header = json!({"typ": "dpop+jwt", "alg": "HS256", "jwk": key.jwk()});
        assert!(why(check(&key.sign(&header, &claims))).contains("ES256"));
        let mut jwk = key.jwk();
        jwk["d"] = json!("secret");
        let header = json!({"typ": "dpop+jwt", "alg": "ES256", "jwk": jwk});
        assert!(why(check(&key.sign(&header, &claims))).contains("public"));
        why(check("a.b"));
        why(check(&format!("{good}.x")));
    }

    #[test]
    fn refreshes_an_expired_token_until_revoked() {
        let (_dir, auth) = scratch();
        let key = Key::new();
        let paired = pair_dpop(&auth, &key, None);
        let first = paired.access_token.unwrap();
        auth.advance(TimeDelta::hours(2));
        assert!(unauthorized(me(&auth, &key, &first)).contains("expired"));
        // Another key cannot refresh it.
        assert!(unauthorized(refresh(&auth, &Key::new(), &first)).contains("key"));
        let (second, ttl) = refresh(&auth, &key, &first).unwrap();
        assert_eq!(ttl, 3600);
        assert!(me(&auth, &key, &second).is_ok());
        // Only the newest token refreshes.
        assert!(unauthorized(refresh(&auth, &key, &first)).contains("replaced"));
        let (third, _) = refresh(&auth, &key, &second).unwrap();
        // The one before stays good until it expires.
        assert!(me(&auth, &key, &second).is_ok());
        assert!(me(&auth, &key, &third).is_ok());

        // Thirty idle days end the session.
        let other = Key::new();
        let idle = pair_dpop(&auth, &other, None).access_token.unwrap();
        auth.advance(TimeDelta::days(30));
        assert!(unauthorized(refresh(&auth, &other, &idle)).contains("ended"));

        let key2 = Key::new();
        let p = pair_dpop(&auth, &key2, None);
        let token = p.access_token.unwrap();
        auth.revoke(&p.device).unwrap();
        unauthorized(refresh(&auth, &key2, &token));
        unauthorized(me(&auth, &key2, &token));
        assert!(matches!(auth.revoke(&p.device), Err(Error::NotFound(_))));
    }

    #[test]
    fn tickets_are_used_once_and_expire() {
        let (_dir, auth) = scratch();
        let key = Key::new();
        let token = pair_dpop(&auth, &key, None).access_token.unwrap();
        let p = me(&auth, &key, &token).unwrap();
        let ws = |ticket: &str, proof: Option<&str>| {
            auth.redeem_ticket(
                ticket,
                &Request {
                    method: "GET",
                    url: "http://192.168.1.20:7268/ws",
                    dpop: proof,
                    ..Request::default()
                },
            )
        };

        let ticket = auth.ws_ticket(&p);
        assert_eq!(
            ws(&ticket, Some(&key.proof(&auth, "GET", WS, None)))
                .unwrap()
                .device,
            p.device
        );
        assert!(
            unauthorized(ws(&ticket, Some(&key.proof(&auth, "GET", WS, None)))).contains("used")
        );
        // Without a proof, or with another key's: refused, and used up.
        let ticket = auth.ws_ticket(&p);
        unauthorized(ws(&ticket, None));
        unauthorized(ws(&ticket, Some(&key.proof(&auth, "GET", WS, None))));
        let ticket = auth.ws_ticket(&p);
        assert!(
            unauthorized(ws(&ticket, Some(&Key::new().proof(&auth, "GET", WS, None))))
                .contains("key")
        );
        let ticket = auth.ws_ticket(&p);
        auth.advance(TimeDelta::seconds(31));
        unauthorized(ws(&ticket, Some(&key.proof(&auth, "GET", WS, None))));
        // Revoked before it is used.
        let ticket = auth.ws_ticket(&p);
        auth.revoke(&p.device).unwrap();
        unauthorized(ws(&ticket, Some(&key.proof(&auth, "GET", WS, None))));

        // A browser's ticket needs no proof.
        let (secret, _) = auth.create_pairing(None, PAIRING_TTL).unwrap();
        let req = Request {
            method: "POST",
            url: API,
            ..Request::default()
        };
        let browser = auth.pair(&req, &secret, "Firefox", "web", true).unwrap();
        let cookie = browser.cookie.unwrap();
        let cookie = cookie.split(';').next().unwrap();
        let p = auth
            .authenticate(&Request {
                method: "POST",
                url: API,
                cookie: Some(cookie),
                ..Request::default()
            })
            .unwrap();
        let ticket = auth.ws_ticket(&p);
        assert_eq!(ws(&ticket, None).unwrap().device, browser.device);
    }

    #[test]
    fn a_new_device_waits_for_approval() {
        let (_dir, auth) = scratch();
        let before = pair_dpop(&auth, &Key::new(), None);
        assert!(!auth.require_approval());
        auth.set_require_approval(true).unwrap();
        assert!(auth.require_approval());
        let key = Key::new();
        let paired = pair_dpop(&auth, &key, None);
        assert!(!paired.approved);
        assert_eq!(paired.body()["approval"], "pending");
        let p = me(&auth, &key, paired.access_token.as_deref().unwrap()).unwrap();
        assert!(!p.approved && !p.allows(Scope::OrchestrationRead));
        assert!(matches!(
            p.require(Scope::OrchestrationRead),
            Err(Error::Forbidden(_))
        ));
        let listed = auth.devices();
        let approval = |id: &str| listed.iter().find(|d| d.id == id).unwrap().approval;
        assert_eq!(approval(&before.device), Approval::Approved);
        assert_eq!(approval(&paired.device), Approval::Pending);

        auth.approve(&paired.device).unwrap();
        let p = me(&auth, &key, paired.access_token.as_deref().unwrap()).unwrap();
        assert!(p.approved && p.require(Scope::OrchestrationRead).is_ok());
        assert!(matches!(auth.approve("d_none"), Err(Error::NotFound(_))));
    }

    #[test]
    fn scopes_narrow_at_pairing() {
        for s in Scope::ALL {
            assert_eq!(Scope::parse(s.as_str()), Some(s));
            assert_eq!(serde_json::to_value(s).unwrap(), s.as_str());
        }
        assert_eq!(Scope::parse("root"), None);

        let (_dir, auth) = scratch();
        assert!(matches!(
            auth.create_pairing(Some(&[]), PAIRING_TTL),
            Err(Error::BadRequest(_))
        ));
        let key = Key::new();
        let narrow = [Scope::OrchestrationRead, Scope::OrchestrationRead];
        let paired = pair_dpop(&auth, &key, Some(&narrow));
        assert_eq!(paired.scopes, [Scope::OrchestrationRead]);
        let p = me(&auth, &key, paired.access_token.as_deref().unwrap()).unwrap();
        assert!(p.require(Scope::OrchestrationRead).is_ok());
        let e = p.require(Scope::OrchestrationOperate).unwrap_err();
        assert_eq!((e.code(), e.status()), ("forbidden", 403));
        // It cannot make pairings, lacking access:write.
        assert!(matches!(
            auth.create_pairing_by(&p, None, PAIRING_TTL),
            Err(Error::Forbidden(_))
        ));

        // A device gives only what it has, and its own by default.
        let admin_key = Key::new();
        let admin = pair_dpop(
            &auth,
            &admin_key,
            Some(&[Scope::AccessWrite, Scope::OrchestrationRead]),
        );
        let admin = me(&auth, &admin_key, admin.access_token.as_deref().unwrap()).unwrap();
        assert!(matches!(
            auth.create_pairing_by(&admin, Some(&[Scope::TerminalOperate]), PAIRING_TTL),
            Err(Error::Forbidden(_))
        ));
        let (secret, _) = auth.create_pairing_by(&admin, None, PAIRING_TTL).unwrap();
        let k = Key::new();
        let proof = k.proof(&auth, "POST", API, None);
        let req = Request {
            method: "POST",
            url: API,
            dpop: Some(&proof),
            ..Request::default()
        };
        assert_eq!(
            auth.pair(&req, &secret, "iPad", "ios", false)
                .unwrap()
                .scopes,
            [Scope::AccessWrite, Scope::OrchestrationRead]
        );
        // Revoking it drops the pairings it made and did not see used.
        let (secret, _) = auth
            .create_pairing_by(&admin, Some(&[Scope::OrchestrationRead]), PAIRING_TTL)
            .unwrap();
        auth.revoke(&admin.device).unwrap();
        let proof = k.proof(&auth, "POST", API, None);
        let req = Request {
            method: "POST",
            url: API,
            dpop: Some(&proof),
            ..Request::default()
        };
        unauthorized(auth.pair(&req, &secret, "iPad", "ios", false));
    }

    /// `pansy pair` and `pansy devices revoke` change the file while the
    /// server runs.
    #[test]
    fn sees_what_another_process_wrote() {
        let (dir, server) = scratch();
        let cli = Auth::load_at(dir.path().join("remote.json")).unwrap();
        assert_eq!(cli.machine_id(), server.machine_id());
        let key = Key::new();
        let (secret, _) = cli.create_pairing(None, PAIRING_TTL).unwrap();
        let proof = key.proof(&server, "POST", API, None);
        let req = Request {
            method: "POST",
            url: API,
            dpop: Some(&proof),
            ..Request::default()
        };
        let paired = server.pair(&req, &secret, "iPhone", "ios", false).unwrap();
        let token = paired.access_token.unwrap();
        assert!(me(&server, &key, &token).is_ok());
        assert_eq!(cli.devices().len(), 1);
        cli.revoke(&paired.device).unwrap();
        unauthorized(me(&server, &key, &token));
        assert!(server.devices().is_empty());
    }
}
