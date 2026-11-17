//! Import links, magpie's: a vendor or a relay hands its users a
//! ready-made provider.
//!
//! ```text
//! pansy://import?preset=deepseek&key=sk-…
//! pansy://import?name=My%20Relay&chat=https://relay.example/v1&key=sk-…
//! ```
//!
//! magpie's own (`magpie://import?…`) read the same. Parameters, each value
//! URL-encoded:
//!
//! - `preset`: a preset's id, whose endpoints are used; `region`, one of its
//!   plans or regions.
//! - `name`: the provider's name, needed without a preset; `id`, its id
//!   (from the name without one).
//! - `key`: the API key; without one the user pastes it.
//! - `chat`, `responses`, `anthropic`, `gemini`: base URLs, https (plain
//!   http only on this computer or the local network).
//! - `models`: model ids, comma separated.
//!
//! magpie's `catalog`, `website`, `keys`, `icon` and `balance` are let be:
//! pansy fetches no picture or balance on a link's say. A header or an
//! account-wide token is never taken from a link: a URL is kept in browser
//! histories and logs. A link saves nothing by itself: the window, or
//! `pansy import`, shows what it would add, and the user says yes.

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use url::Url;

use pansy_core::config;
use pansy_core::presets;
use pansy_core::provider::{Model, Provider, host_of, mask_key, slug};

use super::{Added, Eps, custom, free_id, own_network, reserved, split_host};

/// The schemes of import links: pansy's, and magpie's that vendors publish.
pub const SCHEMES: [&str; 2] = ["pansy", "magpie"];

/// Whether `text` is a link of [`SCHEMES`], to read as one.
pub fn is_link(text: &str) -> bool {
    text.trim()
        .split_once(':')
        .is_some_and(|(scheme, _)| SCHEMES.iter().any(|s| s.eq_ignore_ascii_case(scheme)))
}

/// The provider an import link describes, its key included when it has one.
pub fn read(link: &str) -> Result<Provider> {
    let link = link.trim();
    if !is_link(link) {
        bail!("not an import link: they start pansy://import? or magpie://import?");
    }
    let u = Url::parse(link).map_err(|_| anyhow!("the link can't be read"))?;
    // pansy://import?…, or pansy:import?… and pansy:///import?…
    let action = match u.host_str() {
        Some(h) if !h.is_empty() => h.to_owned(),
        _ => u.path().trim_matches('/').to_owned(),
    };
    if !action.eq_ignore_ascii_case("import") {
        bail!(
            "{}://{action} is not something pansy knows; links start {}://import?",
            u.scheme(),
            u.scheme()
        );
    }
    let get = |k: &str| {
        u.query_pairs()
            .find(|(name, _)| name == k)
            .map(|(_, v)| v.trim().to_owned())
            .unwrap_or_default()
    };
    let preset_id = get("preset").to_ascii_lowercase();
    let mut p = if !preset_id.is_empty() {
        let pr =
            presets::find(&preset_id).ok_or_else(|| anyhow!("pansy has no preset {preset_id}"))?;
        if !pr.endpoint.is_empty() {
            bail!(
                "{} is added with your own address, not from a link: add it in pansy",
                pr.name
            );
        }
        let region = match get("region") {
            r if r.is_empty() => presets::alias_region(&preset_id),
            r => Some(
                pr.region(&r)
                    .ok_or_else(|| anyhow!("{} has no region {r:?}", pr.name))?,
            ),
        };
        let mut p = Provider::from_preset_at(pr, region, None);
        let name = get("name");
        if !name.is_empty() {
            p.name = name;
        }
        p
    } else {
        let name = get("name");
        if name.is_empty() {
            bail!("the link names no provider: it needs preset= or name=");
        }
        let mut e = Eps::default();
        for (k, dst) in [
            ("chat", &mut e.chat),
            ("responses", &mut e.responses),
            ("anthropic", &mut e.anthropic),
            ("gemini", &mut e.gemini),
        ] {
            *dst = endpoint(k, &get(k))?;
        }
        if e.each().iter().all(|(_, u)| u.is_empty()) {
            bail!("the link gives no base URL: it needs chat=, responses=, anthropic= or gemini=");
        }
        let mut p = custom(&slug(&name), &e);
        p.name = name;
        p
    };
    let id = get("id");
    if !id.is_empty() {
        p.id = slug(&id);
    }
    if p.id.is_empty() {
        bail!("the link's name has no letters or digits to make an id from");
    }
    if p.id == "pansy" {
        bail!("\"pansy\" is what agents call the gateway itself; the link needs another id");
    }
    if p.name.len() > 80 {
        // 80 bytes at most, cut where a character begins
        let cut = (0..=80)
            .rev()
            .find(|&i| p.name.is_char_boundary(i))
            .unwrap_or(0);
        p.name.truncate(cut);
    }
    let port = config::load_settings().map_or(pansy_core::settings::DEFAULT_PORT, |s| s.port);
    for e in &p.endpoints {
        let host = host_of(&e.base_url);
        let (name, at) = split_host(&host);
        if matches!(name, "127.0.0.1" | "localhost") && at == port.to_string() {
            bail!("the link points at pansy itself");
        }
    }
    let key = get("key");
    if key.len() > 4096 || key.chars().any(char::is_control) {
        bail!("the link's key is not a key");
    }
    if !key.is_empty() {
        p.add_account(&key, "");
    }
    for m in get("models")
        .split(',')
        .map(str::trim)
        .filter(|m| !m.is_empty())
    {
        if p.models.len() >= 200 {
            break;
        }
        if !p.has_model(m) {
            p.models.push(Model::new(m));
        }
        if p.preset.is_none() && !p.manual_models.iter().any(|x| x == m) {
            p.manual_models.push(m.to_owned());
        }
    }
    Ok(p)
}

/// A link's base URL: https, or plain http to this computer or the local
/// network, where model servers run without TLS.
fn endpoint(name: &str, raw: &str) -> Result<String> {
    if raw.is_empty() {
        return Ok(String::new());
    }
    let bad = || anyhow!("the link's {name}= is not a base URL: {raw:?}");
    let u = Url::parse(raw).map_err(|_| bad())?;
    if u.host_str().is_none_or(str::is_empty)
        || !u.username().is_empty()
        || u.password().is_some()
        || u.query().is_some()
        || u.fragment().is_some()
    {
        return Err(bad());
    }
    match u.scheme() {
        "https" => {}
        "http" if own_network(&host_of(raw)) => {}
        _ => bail!("the link's {name}= must use https: {raw:?}"),
    }
    Ok(raw.trim_end_matches('/').to_owned())
}

/// Whether a provider the link describes needs no key: a keyless preset's,
/// or one on this computer or the local network.
fn keyless(p: &Provider) -> bool {
    p.preset().is_some_and(|pr| pr.keyless)
        || p.endpoints
            .iter()
            .all(|e| own_network(&host_of(&e.base_url)))
}

/// What the window shows of a link before the user says yes: the provider
/// with its key masked, the hosts the prompts and the key would go to, and
/// the provider of the same id it would replace. `{error}` for a link that
/// can't be imported.
pub fn view(link: &str) -> Value {
    let p = match read(link) {
        Ok(p) => p,
        Err(e) => return json!({ "error": format!("{e:#}") }),
    };
    let have = config::load_providers().unwrap_or_default();
    // one named as a subscription or a plugin's is added beside it
    let beside = reserved(&p.id, &have);
    let replaces = have
        .iter()
        .find(|h| h.id == p.id && !beside)
        .map(|h| h.name.clone())
        .unwrap_or_default();
    let mut hosts: Vec<String> = Vec::new();
    for e in &p.endpoints {
        let h = host_of(&e.base_url);
        if !hosts.contains(&h) {
            hosts.push(h);
        }
    }
    json!({
        "id": p.id,
        "name": p.name,
        "icon": p.icon(),
        "endpoints": p.endpoints.iter().map(|e| json!({ "protocol": e.protocol, "url": e.base_url })).collect::<Vec<_>>(),
        "hosts": hosts,
        "key": p.accounts.first().map(|a| mask_key(&a.key)).unwrap_or_default(),
        "keyless": keyless(&p),
        "keyUrl": p.key_url(),
        "models": p.models.iter().map(|m| m.id.clone()).collect::<Vec<_>>(),
        "replaces": replaces,
    })
}

/// Adds the provider a link describes, read from the link again: `name`
/// and `key` when the user gave them in the window. One of the same id is
/// replaced in its place, as magpie does; one named as a subscription or a
/// plugin's provider gets an id of its own beside it.
pub fn add(link: &str, name: &str, key: &str) -> Result<Added> {
    let mut p = read(link)?;
    let name = name.trim();
    if !name.is_empty() {
        p.name = name.to_owned();
    }
    let key = key.trim();
    if !key.is_empty() {
        p.accounts.clear();
        p.add_account(key, "");
    }
    if p.accounts.is_empty() && !keyless(&p) {
        bail!("{} needs an API key: paste one", p.name);
    }
    let added = config::update_providers(|ps| {
        if reserved(&p.id, ps) {
            p.id = free_id(&p.id, ps);
        }
        match ps.iter().position(|h| h.id == p.id) {
            Some(i) => ps[i] = p.clone(),
            None => ps.push(p.clone()),
        }
        Ok(Added {
            names: vec![p.name.clone()],
            ids: vec![p.id.clone()],
        })
    })?;
    Ok(added)
}
