//! Providers another app has set up, offered to bring over as magpie offers
//! them: magpie's own, CC Switch's, Alma's, Claude Code's and Codex's
//! settings, and OpenCode's sign-ins. Each app is only read. Every entry is
//! made into the provider pansy would add and matched against the providers
//! here: new, already here, or at an id in use (or at an address one has,
//! to join it as one more key). Nothing is added until the user picks, and
//! the keys stay here: the window sees them masked, and an import reads the
//! apps again.

pub mod link;
pub mod paste;
mod readers;

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use pansy_core::config;
use pansy_core::presets::{self, Kind as PresetKind, Preset, Region};
use pansy_core::provider::{
    Endpoint, Model, Protocol, Provider, azure_base, host_of, mask_key, slug,
};
use pansy_core::subscription::Kind as Subscription;

/// One app pansy can import from, with what it holds.
#[derive(Debug, Clone)]
pub struct Source {
    pub id: &'static str,
    pub name: &'static str,
    pub path: PathBuf,
    pub found: bool,
    pub error: Option<String>,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// pansy has nothing like it.
    #[default]
    New,
    /// pansy has it: the same key at the same address.
    Same,
    /// Its id is another provider's here.
    Taken,
}

/// One provider an app holds, as pansy would add it.
#[derive(Debug, Clone, Default)]
pub struct Item {
    /// Which it is within its app, the same on every read.
    pub reference: String,
    /// Where in the app: Claude Code, Codex, `settings.json`…
    pub from: String,
    pub name: String,
    /// The provider pansy would add; none when it can't be imported.
    pub provider: Option<Provider>,
    pub status: Status,
    /// The provider here it matches or whose id it has.
    pub existing: String,
    /// A provider here at the same address with another key: this one can
    /// join it as one more key.
    pub key_of: String,
    /// Its id is a subscription's or pansy's own: it comes in only beside it.
    pub reserved: bool,
    /// Why it isn't picked by default.
    pub off: String,
    /// Why it can't be imported at all.
    pub skip: String,
}

/// How a picked entry comes in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// A provider of its own, beside any with its id.
    #[default]
    Add,
    /// In place of the provider with its id.
    Replace,
    /// One more key of the provider `key_of` names.
    Key,
}

/// One entry the user picked; `name` when they renamed it in the picker.
#[derive(Debug, Clone, Deserialize)]
pub struct Pick {
    pub source: String,
    #[serde(rename = "ref")]
    pub reference: String,
    #[serde(default)]
    pub mode: Mode,
    #[serde(default)]
    pub name: String,
}

/// What an import added: the names, as the window says them, and the new
/// providers' ids, whose model lists are fetched next.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Added {
    pub names: Vec<String>,
    pub ids: Vec<String>,
}

/// The base URLs an entry names, by protocol.
#[derive(Debug, Clone, Default)]
pub(crate) struct Eps {
    pub chat: String,
    pub responses: String,
    pub anthropic: String,
    pub gemini: String,
}

impl Eps {
    fn each(&self) -> [(Protocol, &String); 4] {
        [
            (Protocol::Anthropic, &self.anthropic),
            (Protocol::Chat, &self.chat),
            (Protocol::Responses, &self.responses),
            (Protocol::Gemini, &self.gemini),
        ]
    }

    fn each_mut(&mut self) -> [&mut String; 4] {
        [
            &mut self.anthropic,
            &mut self.chat,
            &mut self.responses,
            &mut self.gemini,
        ]
    }

    pub(crate) fn of(preset: &Preset) -> Eps {
        let mut e = Eps::default();
        for (protocol, url) in preset.urls_at(None) {
            let url = (*url).to_owned();
            match protocol {
                Protocol::Anthropic => e.anthropic = url,
                Protocol::Chat => e.chat = url,
                Protocol::Responses => e.responses = url,
                Protocol::Gemini => e.gemini = url,
            }
        }
        e
    }
}

/// Every app pansy can import from; `asked` reads OpenCode's sign-ins too,
/// which only the user's own look reads (they hold every sign-in it has).
pub fn sources(asked: bool) -> Vec<Source> {
    let have = config::load_providers().unwrap_or_default();
    let port = config::load_settings().map_or(pansy_core::settings::DEFAULT_PORT, |s| s.port);
    let mut used = HashSet::new();
    let mut out = Vec::new();
    for r in readers::READERS {
        if r.asked && !asked {
            continue;
        }
        let path = (r.path)();
        let mut s = Source {
            id: r.id,
            name: r.name,
            path: path.clone(),
            found: false,
            error: None,
            items: Vec::new(),
        };
        match std::fs::metadata(&path) {
            Ok(_) => {
                s.found = true;
                match (r.read)(&path, port) {
                    Ok(items) => s.items = settle(items, &have, &mut used),
                    Err(e) => s.error = Some(format!("{e:#}")),
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            // there, but not to be looked at: not "not found", which would
            // say the app has nothing
            Err(e) => (s.found, s.error) = (true, Some(e.to_string())),
        }
        out.push(s);
    }
    out
}

/// An entry's fingerprint: the window remembers the ones the user ignored
/// by it, without keeping their keys, addresses or paths.
pub fn fingerprint(source: &str, item: &Item) -> String {
    let provider = item.provider.as_ref().map(|p| {
        let mut p = p.clone();
        p.id.clear(); // settle's id moves when another app is added
        p
    });
    let text =
        serde_json::to_string(&json!([source, item.reference, provider])).unwrap_or_default();
    Sha256::digest(text.as_bytes())[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Whether an entry is picked when the picker opens: importable, new, on
/// in its app, and needing no choice of how it comes in.
pub fn picked(item: &Item) -> bool {
    item.skip.is_empty()
        && item.status != Status::Same
        && item.off.is_empty()
        && (item.status != Status::Taken || !item.key_of.is_empty() || item.reserved)
}

/// What the Providers page offers by itself: the entries of the apps it
/// looks in unasked that would be picked, and that the user hasn't ignored.
/// `[{fingerprint, source}]`.
pub fn found() -> Result<Vec<Value>> {
    let ignored: HashSet<String> = config::load_settings()?
        .import_ignored
        .into_iter()
        .collect();
    let mut out = Vec::new();
    for s in sources(false) {
        for it in &s.items {
            let f = fingerprint(s.id, it);
            if picked(it) && !ignored.contains(&f) {
                out.push(json!({"fingerprint": f, "source": s.name}));
            }
        }
    }
    Ok(out)
}

/// Hides these entries from the Providers page until they change.
pub fn ignore(fingerprints: &[String]) -> Result<()> {
    config::update_settings(|s| {
        for f in fingerprints {
            if !s.import_ignored.contains(f) {
                s.import_ignored.push(f.clone());
            }
        }
        Ok(())
    })
}

/// The apps as the picker shows them, keys masked (JSON).
pub fn view(sources: &[Source]) -> Value {
    let home = pansy_core::paths::user_home();
    Value::Array(
        sources
            .iter()
            .map(|s| {
                let path = match s.path.strip_prefix(&home) {
                    Ok(rest) => format!("~/{}", rest.display()),
                    Err(_) => s.path.display().to_string(),
                };
                json!({
                    "id": s.id, "name": s.name, "path": path, "found": s.found, "error": s.error,
                    "items": s.items.iter().map(|it| {
                        let p = it.provider.as_ref();
                        json!({
                            "ref": it.reference, "from": it.from, "name": it.name,
                            "icon": p.map_or("generic", Provider::icon),
                            "host": p.and_then(|p| p.endpoints.first()).map(|e| host_of(&e.base_url)).unwrap_or_default(),
                            "key": p.and_then(|p| p.accounts.first()).map(|a| mask_key(&a.key)).unwrap_or_default(),
                            "keys": p.map_or(0, |p| p.accounts.len()),
                            "models": p.map_or(0, |p| p.models.len()),
                            "status": it.status, "existing": it.existing, "keyOf": it.key_of,
                            "reserved": it.reserved, "off": it.off, "skip": it.skip,
                            "picked": picked(it), "fingerprint": fingerprint(s.id, it),
                        })
                    }).collect::<Vec<_>>(),
                })
            })
            .collect(),
    )
}

/// An entry read from an app: the provider pansy would add, or why it
/// can't be imported.
pub(crate) fn entry(
    reference: &str,
    from: &str,
    name: &str,
    key: &str,
    eps: Eps,
    models: Vec<String>,
    port: u16,
) -> Item {
    let name = name.trim();
    let mut it = Item {
        reference: reference.to_owned(),
        from: from.to_owned(),
        name: name.to_owned(),
        ..Item::default()
    };
    match imported(name, key.trim(), eps, models, port) {
        Ok(p) => {
            it.name = p.name.clone();
            it.provider = Some(p);
        }
        Err(why) => it.skip = why.to_owned(),
    }
    it
}

/// An entry that can't be imported, and why.
pub(crate) fn skipped(reference: &str, from: &str, name: &str, why: &str) -> Item {
    Item {
        reference: reference.to_owned(),
        from: from.to_owned(),
        name: name.trim().to_owned(),
        skip: why.to_owned(),
        ..Item::default()
    }
}

fn clean_base(u: &str) -> String {
    let u = u.trim().trim_end_matches('/');
    if !u.is_empty() && !u.contains("://") {
        return format!("https://{u}");
    }
    u.to_owned()
}

/// `api.example.com:8443` → (`api.example.com`, `8443`).
fn split_host(host: &str) -> (&str, &str) {
    if let Some(rest) = host.strip_prefix('[') {
        return rest
            .split_once("]:")
            .unwrap_or((rest.trim_end_matches(']'), ""));
    }
    host.rsplit_once(':').unwrap_or((host, ""))
}

/// A host on this computer or the user's own network, where a server
/// needs no key.
fn own_network(host: &str) -> bool {
    let (name, _) = split_host(host);
    let octets: Vec<u8> = name.split('.').filter_map(|s| s.parse().ok()).collect();
    name == "localhost"
        || name == "::1"
        || name.ends_with(".local")
        || octets.len() == 4
            && matches!(
                octets[..2],
                [127, _] | [10, _] | [192, 168] | [169, 254] | [172, 16..=31] | [100, 64..=127]
            )
}

/// The provider for an entry: a preset's when it is at one pansy knows (its
/// own endpoints, or one of its plans' or regions'), else one of the
/// user's own.
pub(crate) fn imported(
    name: &str,
    key: &str,
    mut e: Eps,
    models: Vec<String>,
    port: u16,
) -> Result<Provider, &'static str> {
    for u in e.each_mut() {
        *u = clean_base(u);
    }
    let urls: Vec<String> = e
        .each()
        .iter()
        .map(|(_, u)| (*u).clone())
        .filter(|u| !u.is_empty())
        .collect();
    if urls.is_empty() {
        return Err("it has no base URL");
    }
    for u in &urls {
        let host = host_of(u);
        let (name, at) = split_host(&host);
        let web = u.starts_with("http://") || u.starts_with("https://");
        if !web
            || name.is_empty()
            || !(name.contains('.') || name.contains(':') || name == "localhost")
        {
            return Err("its base URL isn't valid");
        }
        if matches!(name, "127.0.0.1" | "localhost") && at == port.to_string() {
            return Err("it points at pansy itself");
        }
    }
    let mut models: Vec<String> = models
        .into_iter()
        .map(|m| m.trim().to_owned())
        .filter(|m| !m.is_empty())
        .collect();
    models.dedup();
    let azure = urls
        .iter()
        .find_map(|u| azure_base(u).filter(|_| host_of(u).contains(".azure.com")));
    let mut p = if let Some(base) = azure {
        // an Azure OpenAI resource: the preset's, on the resource's v1 API
        let mut p =
            Provider::from_preset(presets::find(presets::AZURE).expect("Azure's preset"), None);
        p.endpoints = [Protocol::Chat, Protocol::Responses]
            .into_iter()
            .map(|protocol| Endpoint {
                protocol,
                base_url: base.clone(),
            })
            .collect();
        p.models.clear();
        p
    } else {
        match preset_at(&e) {
            Some((pr, region, true)) => {
                let mut p = Provider::from_preset_at(pr, region, None);
                if !e.gemini.is_empty() && p.endpoint(Protocol::Gemini).is_none() {
                    p.endpoints.push(Endpoint {
                        protocol: Protocol::Gemini,
                        base_url: e.gemini.clone(),
                    });
                }
                p
            }
            // another path at a vendor's host (a plan pansy has no preset
            // of): the user's own, under the vendor's id
            Some((pr, _, false)) => custom(pr.id, &e),
            None => custom(&slug(name), &e),
        }
    };
    if !name.is_empty() && !name.eq_ignore_ascii_case("default") {
        p.name = name.to_owned();
    } else if p.name.is_empty() {
        p.name = urls.first().map(|u| host_of(u)).unwrap_or_default();
    }
    for m in &models {
        if !p.has_model(m) {
            p.models.push(Model::new(m));
        }
        if p.preset.is_none() && !p.manual_models.contains(m) {
            p.manual_models.push(m.clone());
        }
    }
    if !key.is_empty() {
        p.add_account(key, "");
    } else if !(p.preset().is_some_and(|pr| pr.keyless)
        || p.endpoints
            .iter()
            .all(|x| own_network(&host_of(&x.base_url))))
    {
        return Err("it has no API key");
    }
    Ok(p)
}

fn custom(id: &str, e: &Eps) -> Provider {
    Provider {
        id: id.to_owned(),
        name: String::new(),
        preset: None,
        endpoints: e
            .each()
            .into_iter()
            .filter(|(_, u)| !u.is_empty())
            .map(|(protocol, u)| Endpoint {
                protocol,
                base_url: u.clone(),
            })
            .collect(),
        accounts: Vec::new(),
        models: Vec::new(),
        manual_models: Vec::new(),
        hidden_models: Vec::new(),
        enabled: true,
        headers: BTreeMap::new(),
        proxy: None,
        auth_header: None,
        quirks: Default::default(),
        subscription: None,
        plugin: None,
    }
}

/// The preset serving an entry's host, the plan or region of it the entry
/// is at, and whether the entry uses exactly those endpoints. One whose
/// endpoints the entry uses wins over an earlier one at the same host.
fn preset_at(e: &Eps) -> Option<(&'static Preset, Option<&'static Region>, bool)> {
    let given: Vec<(Protocol, &String)> = e
        .each()
        .into_iter()
        .filter(|(p, u)| *p != Protocol::Gemini && !u.is_empty())
        .collect();
    if given.is_empty() {
        return None;
    }
    let at = |own: &[(Protocol, &str)]| {
        let hit = given
            .iter()
            .any(|(_, u)| own.iter().any(|(_, o)| host_of(o) == host_of(u)));
        let exact = given.iter().all(|(p, u)| {
            own.iter()
                .any(|(q, o)| q == p && o.trim_end_matches('/').eq_ignore_ascii_case(u))
        });
        (hit, exact)
    };
    let mut near = None;
    for pr in presets::PRESETS
        .iter()
        .filter(|p| p.kind != PresetKind::Local && p.endpoint.is_empty())
    {
        let own_urls = (!pr.urls.is_empty()).then_some((None, pr.urls));
        let places = own_urls
            .into_iter()
            .chain(pr.regions.iter().map(|r| (Some(r), r.urls)));
        for (region, own) in places {
            match at(own) {
                (true, true) => return Some((pr, region, true)),
                (true, false) if near.is_none() => near = Some((pr, region, false)),
                _ => {}
            }
        }
    }
    near
}

fn clean_headers(h: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    h.iter()
        .filter(|(k, v)| !k.trim().is_empty() && !v.trim().is_empty())
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect()
}

/// Whether `b` is at an address of `a`'s, with the same headers.
fn same_place(a: &Provider, b: &Provider) -> bool {
    if a.subscription.is_some()
        || a.plugin.is_some()
        || clean_headers(&a.headers) != clean_headers(&b.headers)
    {
        return false;
    }
    let hosts: HashSet<String> = a.endpoints.iter().map(|e| host_of(&e.base_url)).collect();
    b.endpoints
        .iter()
        .any(|e| hosts.contains(&host_of(&e.base_url)))
}

fn key_of(p: &Provider) -> &str {
    p.accounts.first().map_or("", |a| a.key.as_str())
}

/// The same account: `b`'s key with `a`, at an address of `a`'s.
fn same_provider(a: &Provider, b: &Provider) -> bool {
    let key = key_of(b);
    let has = if key.is_empty() {
        a.accounts.iter().all(|x| x.key.is_empty())
    } else {
        a.accounts.iter().any(|x| x.key == key)
    };
    has && same_place(a, b)
}

/// An id no imported provider takes: pansy's own, a subscription's, a
/// plugin's.
fn reserved(id: &str, have: &[Provider]) -> bool {
    id == "pansy"
        || Subscription::ALL.iter().any(|k| k.id() == id)
        || have
            .iter()
            .any(|p| p.id == id && (p.subscription.is_some() || p.plugin.is_some()))
}

/// Gives each importable entry an id of its own and says how it stands
/// against the providers here.
fn settle(mut items: Vec<Item>, have: &[Provider], used: &mut HashSet<String>) -> Vec<Item> {
    for it in &mut items {
        let Some(p) = it.provider.as_mut() else {
            continue;
        };
        if p.id.is_empty() || p.id == "pansy" {
            p.id = slug(&p.name);
        }
        if p.id.is_empty() || p.id == "pansy" {
            p.id = "imported".to_owned();
        }
        let base = p.id.clone();
        for n in 2.. {
            if !used.contains(&p.id) {
                break;
            }
            p.id = format!("{base}-{n}");
        }
        used.insert(p.id.clone());
        it.status = Status::New;
        for h in have {
            if same_provider(h, p) {
                (it.status, it.existing) = (Status::Same, h.name.clone());
                break;
            }
            if h.id == p.id {
                (it.status, it.existing) = (Status::Taken, h.name.clone());
            }
            if it.key_of.is_empty()
                && !key_of(p).is_empty()
                && !h.accounts.is_empty()
                && same_place(h, p)
            {
                it.key_of = h.id.clone();
            }
        }
        if it.status == Status::Same {
            it.key_of.clear();
        } else if !it.key_of.is_empty() && it.existing.is_empty() {
            it.existing = have
                .iter()
                .find(|h| h.id == it.key_of)
                .map(|h| h.name.clone())
                .unwrap_or_default();
        }
        if it.status != Status::Same && reserved(&p.id, have) {
            // named as a subscription: only ever added beside it
            (it.status, it.reserved) = (Status::Taken, true);
            if it.existing.is_empty() {
                it.existing = have
                    .iter()
                    .find(|h| h.id == p.id)
                    .map_or(p.id.clone(), |h| h.name.clone());
            }
        }
    }
    // importable first; what can't be imported goes last
    items.sort_by_key(|it| !it.skip.is_empty());
    items
}

fn free_id(id: &str, have: &[Provider]) -> String {
    let taken = |x: &str| reserved(x, have) || have.iter().any(|p| p.id == x);
    if !taken(id) {
        return id.to_owned();
    }
    (2..)
        .map(|n| format!("{id}-{n}"))
        .find(|x| !taken(x))
        .unwrap()
}

fn free_name(name: &str, have: &[Provider]) -> String {
    let taken = |x: &str| have.iter().any(|p| p.name.eq_ignore_ascii_case(x));
    if !taken(name) {
        return name.to_owned();
    }
    (2..)
        .map(|n| format!("{name} {n}"))
        .find(|x| !taken(x))
        .unwrap()
}

/// Adds what the user picked, reading the apps again so keys never pass
/// through the window. The providers it adds alike but for their keys (an
/// app that keeps an entry a key, as CC Switch does) are folded into one
/// with those keys.
pub fn import(picks: &[Pick]) -> Result<Added> {
    let sources = sources(true);
    let mut added = Added::default();
    let result = add_picks(&sources, picks, &mut added);
    fold(&added.ids)?;
    added
        .ids
        .retain(|id| config::load_providers().is_ok_and(|ps| ps.iter().any(|p| &p.id == id)));
    result.map(|()| added)
}

fn add_picks(sources: &[Source], picks: &[Pick], added: &mut Added) -> Result<()> {
    for pk in picks {
        let Some(it) = sources
            .iter()
            .filter(|s| s.id == pk.source)
            .flat_map(|s| &s.items)
            .find(|it| it.reference == pk.reference)
        else {
            bail!(
                "{} no longer has that provider; open the import again",
                pk.source
            );
        };
        let Some(mut p) = it.provider.clone().filter(|_| it.status != Status::Same) else {
            continue;
        };
        let mut mode = pk.mode;
        let name = pk.name.trim();
        if !name.is_empty() && name != p.name {
            // renamed in the picker: its id follows the new name, unless it
            // is a preset's
            p.name = name.to_owned();
            if p.preset.is_none() {
                p.id = Some(slug(name)).filter(|s| !s.is_empty()).unwrap_or(p.id);
            }
            if mode != Mode::Key {
                mode = Mode::Add;
            }
        }
        let label = p.name.clone();
        match mode {
            Mode::Key if !it.key_of.is_empty() => {
                let have = config::load_providers()?;
                let Some(h) = have.iter().find(|h| h.id == it.key_of) else {
                    bail!("{label}: there is no provider {} any more", it.key_of);
                };
                if same_provider(h, &p) {
                    continue; // picked twice, from two apps
                }
                // a key joins a provider only at its own address
                if !same_place(h, &p) {
                    bail!(
                        "{label}: {} isn't at {}; add it as a provider of its own",
                        h.name,
                        p.endpoints
                            .first()
                            .map(|e| host_of(&e.base_url))
                            .unwrap_or_default()
                    );
                }
                pansy_core::manage::add_key(&h.id, key_of(&p), &label)?;
                added.names.push(format!("{label} ({})", h.name));
            }
            Mode::Replace if !it.reserved => {
                config::update_providers(|ps| {
                    match ps.iter().position(|h| h.id == p.id) {
                        Some(i) if ps[i].subscription.is_some() || ps[i].plugin.is_some() => {
                            bail!(
                                "{label}: {} is a subscription in pansy and can't be replaced; add it as a new provider",
                                ps[i].name
                            )
                        }
                        Some(i) => ps[i] = p.clone(),
                        None => ps.push(p.clone()),
                    }
                    Ok(())
                })?;
                added.names.push(label);
                added.ids.push(p.id);
            }
            Mode::Replace => bail!(
                "{label}: {} is a subscription in pansy and can't be replaced; add it as a new provider",
                it.existing
            ),
            _ => {
                let new = config::update_providers(|ps| {
                    // one an earlier pick brought in already (the same relay
                    // in CC Switch and in Claude Code's settings)
                    if ps.iter().any(|h| same_provider(h, &p)) {
                        return Ok(None);
                    }
                    p.id = free_id(&p.id, ps);
                    p.name = free_name(&p.name, ps);
                    ps.push(p.clone());
                    Ok(Some((p.id.clone(), p.name.clone())))
                })?;
                if let Some((id, name)) = new {
                    added.names.push(name);
                    added.ids.push(id);
                }
            }
        }
    }
    Ok(())
}

/// Folds the providers with ids among `ids` that are alike but for their
/// keys into the first of them: one provider with each one's key, named as
/// it was, off when it was.
fn fold(ids: &[String]) -> Result<()> {
    if ids.len() < 2 {
        return Ok(());
    }
    config::update_providers(|ps| {
        let shape = |p: &Provider| {
            let mut p = p.clone();
            (p.id, p.name, p.accounts, p.enabled) =
                (String::new(), String::new(), Vec::new(), true);
            serde_json::to_string(&p).unwrap_or_default()
        };
        let mut first: BTreeMap<String, usize> = BTreeMap::new();
        let mut gone = Vec::new();
        for i in 0..ps.len() {
            if !ids.contains(&ps[i].id) || ps[i].accounts.len() != 1 {
                continue;
            }
            let s = shape(&ps[i]);
            let Some(&into) = first.get(&s) else {
                first.insert(s, i);
                continue;
            };
            let from = ps[i].clone();
            let a = &from.accounts[0];
            if ps[into].accounts.iter().any(|x| x.key == a.key) {
                gone.push(i);
                continue;
            }
            if ps[into].accounts.len() == 1 && ps[into].accounts[0].label.is_empty() {
                ps[into].accounts[0].label = ps[into].name.clone();
            }
            let label = if a.label.is_empty() {
                from.name.clone()
            } else {
                a.label.clone()
            };
            let id = ps[into].add_account(&a.key, &label).id.clone();
            if let Some(x) = ps[into].accounts.iter_mut().find(|x| x.id == id) {
                x.enabled = a.enabled && from.enabled;
            }
            gone.push(i);
        }
        for i in gone.into_iter().rev() {
            ps.remove(i);
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests;
