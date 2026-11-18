//! The changes the window and the command line make to providers and
//! groups, each a read, a change and a write of the file.

use std::collections::BTreeMap;

use anyhow::{Result, anyhow, bail};

use crate::config::{self, update_groups, update_providers};
use crate::group::{Group, Routing};
use crate::models;
use crate::presets;
use crate::provider::{AuthHeader, Endpoint, Model, Protocol, Provider, azure_base, slug};

/// An id not taken yet: `base`, `base-2`, `base-3`…
fn free_id(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_owned();
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|id| !taken(id))
        .unwrap()
}

/// Where a new provider from a preset is: one of its plans or regions, and
/// the address of a server of the user's own (or of an Azure resource).
#[derive(Debug, Clone, Default)]
pub struct At<'a> {
    pub region: Option<&'a str>,
    pub address: Option<&'a str>,
}

/// Adds a key for `preset`: to its provider when there is one, else a new
/// provider made from the preset, at `at`. Its id.
pub fn add_preset(preset: &str, key: Option<&str>, label: &str, at: At) -> Result<String> {
    let region = match at.region.filter(|r| !r.is_empty()) {
        Some(r) => Some(
            presets::find(preset)
                .and_then(|p| p.region(r))
                .ok_or_else(|| anyhow!("{preset} has no plan or region {r}"))?,
        ),
        None => presets::alias_region(preset),
    };
    let preset = presets::find(preset).ok_or_else(|| {
        anyhow!("there is no preset {preset}; `pansy provider presets` lists them")
    })?;
    let key = key.map(str::trim).filter(|k| !k.is_empty());
    if key.is_none() && !preset.keyless {
        bail!(
            "{} needs an API key ({})",
            preset.name,
            preset.key_url_at(region)
        );
    }
    let address = at.address.map(str::trim).filter(|a| !a.is_empty());
    if !preset.endpoint.is_empty() && address.is_none() {
        bail!(
            "{} needs your endpoint, like {}",
            preset.name,
            preset.endpoint
        );
    }
    update_providers(|providers| {
        if let Some(existing) = providers
            .iter_mut()
            .find(|p| p.preset().is_some_and(|p| p.id == preset.id))
        {
            if let Some(key) = key
                && !existing.accounts.iter().any(|a| a.key == key)
            {
                existing.add_account(key, label);
            }
            return Ok(existing.id.clone());
        }
        let mut provider = Provider::from_preset_at(preset, region, None);
        provider.id = free_id(preset.id, |id| providers.iter().any(|p| p.id == id));
        if let Some(address) = address {
            place(&mut provider, address)?;
        }
        if let Some(key) = key {
            provider.add_account(key, label);
        }
        let id = provider.id.clone();
        providers.push(provider);
        Ok(id)
    })
}

/// Puts a provider at `address`: an Azure resource's v1 API, else its
/// server's address, each endpoint keeping its path.
fn place(p: &mut Provider, address: &str) -> Result<()> {
    if p.preset().is_some_and(|pr| !pr.endpoint.is_empty()) {
        let base = azure_base(address)
            .ok_or_else(|| anyhow!("{address} is no Azure OpenAI resource's endpoint"))?;
        p.endpoints = [Protocol::Chat, Protocol::Responses]
            .into_iter()
            .map(|protocol| Endpoint {
                protocol,
                base_url: base.clone(),
            })
            .collect();
        return Ok(());
    }
    p.move_to(address)
}

/// Moves a provider to another of its preset's plans or regions: its
/// endpoints, and the plan's models in place of the ones it gave.
pub fn set_region(id: &str, region: &str) -> Result<()> {
    with_provider(id, |p| {
        let preset = p
            .preset()
            .ok_or_else(|| anyhow!("{} is no preset's", p.name))?;
        let to = preset
            .region(region)
            .ok_or_else(|| anyhow!("{} has no plan or region {region}", preset.name))?;
        let given = preset.models_at(p.region());
        p.endpoints = preset.endpoints_at(Some(to));
        // The models the plan before gave go; the user's and the vendor's stay.
        p.models
            .retain(|m| !given.contains(&m.id.as_str()) || p.manual_models.contains(&m.id));
        for m in preset.models_at(Some(to)) {
            if !p.has_model(m) {
                p.models.push(Model::new(m));
            }
        }
        Ok(())
    })
}

/// Moves a server of the user's own (or an Azure resource) to `address`.
pub fn set_address(id: &str, address: &str) -> Result<()> {
    with_provider(id, |p| place(p, address))
}

/// The one header its key goes in on the Anthropic API; none for both.
pub fn set_auth_header(id: &str, header: Option<AuthHeader>) -> Result<()> {
    with_provider(id, |p| {
        p.auth_header = header;
        p.quirks.anthropic_bearer = false;
        Ok(())
    })
}

/// Keeps the header a vendor took the key in alone, when it refused both:
/// unless the user picked one since.
pub fn learn_auth_header(id: &str, header: AuthHeader) -> Result<()> {
    with_provider(id, |p| {
        if p.auth_header.is_none() && !p.quirks.anthropic_bearer {
            p.auth_header = Some(header);
        }
        Ok(())
    })
}

/// The headers sent with every request to the provider.
pub fn set_headers(id: &str, headers: BTreeMap<String, String>) -> Result<()> {
    for name in headers.keys() {
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_graphic() && b != b':') {
            bail!("{name:?} is no header name");
        }
    }
    with_provider(id, |p| {
        p.headers = headers;
        Ok(())
    })
}

/// Puts a provider at `to` in the list: a model several providers serve
/// (a bare model id) goes to the higher one first.
pub fn move_provider(id: &str, to: usize) -> Result<()> {
    update_providers(|providers| {
        let at = providers
            .iter()
            .position(|p| p.id == id)
            .ok_or_else(|| anyhow!("there is no provider {id}"))?;
        let p = providers.remove(at);
        providers.insert(to.min(providers.len()), p);
        Ok(())
    })
}

/// A provider of the user's own: any OpenAI-, Anthropic- or Gemini-
/// compatible URL.
pub fn add_custom(
    name: &str,
    protocol: Protocol,
    base_url: &str,
    key: Option<&str>,
    models: &[String],
) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        bail!("a provider needs a name");
    }
    let base_url = base_url.trim().trim_end_matches('/');
    if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
        bail!("the base URL starts with http:// or https://");
    }
    update_providers(|providers| {
        let base = slug(name);
        let base = if base.is_empty() || base == "group" {
            "custom".to_owned()
        } else {
            base
        };
        let id = free_id(&base, |id| providers.iter().any(|p| p.id == id));
        let mut provider = Provider {
            id: id.clone(),
            name: name.to_owned(),
            preset: None,
            endpoints: vec![Endpoint {
                protocol,
                base_url: base_url.to_owned(),
            }],
            accounts: Vec::new(),
            models: models.iter().map(|m| Model::new(m)).collect(),
            manual_models: models.to_vec(),
            hidden_models: Vec::new(),
            enabled: true,
            headers: Default::default(),
            proxy: None,
            auth_header: None,
            quirks: Default::default(),
            subscription: None,
            plugin: None,
        };
        if let Some(key) = key.map(str::trim).filter(|k| !k.is_empty()) {
            provider.add_account(key, "");
        }
        providers.push(provider);
        Ok(id)
    })
}

fn with_provider<R>(id: &str, change: impl FnOnce(&mut Provider) -> Result<R>) -> Result<R> {
    update_providers(|providers| {
        let provider = providers
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or_else(|| anyhow!("there is no provider {id}"))?;
        change(provider)
    })
}

pub fn remove_provider(id: &str) -> Result<()> {
    update_providers(|providers| {
        if let Some(p) = providers
            .iter()
            .find(|p| p.id == id && p.subscription.is_some())
        {
            bail!(
                "{} is a subscription, there while it is signed in to: switch it off, or sign its accounts out",
                p.name
            );
        }
        let before = providers.len();
        providers.retain(|p| p.id != id);
        if providers.len() == before {
            bail!("there is no provider {id}");
        }
        Ok(())
    })
}

pub fn set_provider_enabled(id: &str, on: bool) -> Result<()> {
    with_provider(id, |p| {
        p.enabled = on;
        Ok(())
    })
}

pub fn rename_provider(id: &str, name: &str) -> Result<()> {
    with_provider(id, |p| {
        p.name = name.trim().to_owned();
        Ok(())
    })
}

/// Sets (or with an empty URL removes) the provider's endpoint for `protocol`.
pub fn set_endpoint(id: &str, protocol: Protocol, base_url: &str) -> Result<()> {
    with_provider(id, |p| {
        let url = base_url.trim().trim_end_matches('/');
        p.endpoints.retain(|e| e.protocol != protocol);
        if !url.is_empty() {
            p.endpoints.push(Endpoint {
                protocol,
                base_url: url.to_owned(),
            });
        }
        if p.endpoints.is_empty() {
            bail!("a provider needs an endpoint");
        }
        Ok(())
    })
}

pub fn set_proxy(id: &str, proxy: &str) -> Result<()> {
    let proxy = proxy_of(proxy)?;
    with_provider(id, |p| {
        p.proxy = proxy;
        Ok(())
    })
}

/// Adds a key; its account id.
pub fn add_key(id: &str, key: &str, label: &str) -> Result<String> {
    let key = key.trim();
    if key.is_empty() {
        bail!("the key is empty");
    }
    with_provider(id, |p| {
        if p.subscription.is_some() {
            bail!("{} takes sign-ins, not keys: pansy login {}", p.name, p.id);
        }
        if p.accounts.iter().any(|a| a.key == key) {
            bail!("{} has this key already", p.name);
        }
        Ok(p.add_account(key, label).id.clone())
    })
}

pub fn remove_key(id: &str, account: &str) -> Result<()> {
    with_provider(id, |p| {
        let before = p.accounts.len();
        p.accounts.retain(|a| a.id != account);
        if p.accounts.len() == before {
            bail!("{} has no key {account}", p.name);
        }
        Ok(())
    })
}

pub fn set_key_enabled(id: &str, account: &str, on: bool) -> Result<()> {
    with_provider(id, |p| {
        let a = p
            .accounts
            .iter_mut()
            .find(|a| a.id == account)
            .ok_or_else(|| anyhow!("no key {account}"))?;
        a.enabled = on;
        Ok(())
    })
}

pub fn set_key_label(id: &str, account: &str, label: &str) -> Result<()> {
    with_provider(id, |p| {
        let a = p
            .accounts
            .iter_mut()
            .find(|a| a.id == account)
            .ok_or_else(|| anyhow!("no key {account}"))?;
        a.label = label.trim().to_owned();
        Ok(())
    })
}

/// A proxy as a provider or a key may have one: none, `direct`, or an HTTP
/// or SOCKS proxy's address that reads as one.
fn proxy_of(proxy: &str) -> Result<Option<String>> {
    let proxy = proxy.trim();
    if proxy.is_empty() {
        return Ok(None);
    }
    if proxy != "direct" {
        reqwest::Proxy::all(proxy).map_err(|e| anyhow!("the proxy {proxy}: {e}"))?;
    }
    Ok(Some(proxy.to_owned()))
}

/// The proxy for one key's requests, over the provider's and pansy's:
/// empty for none of its own, `direct` to use no proxy.
pub fn set_key_proxy(id: &str, account: &str, proxy: &str) -> Result<()> {
    let proxy = proxy_of(proxy)?;
    with_provider(id, |p| {
        let a = p
            .accounts
            .iter_mut()
            .find(|a| a.id == account)
            .ok_or_else(|| anyhow!("no key {account}"))?;
        a.proxy = proxy;
        Ok(())
    })
}

/// Moves a key one place up (`-1`) or down (`1`): keys are tried in order.
pub fn move_key(id: &str, account: &str, by: i32) -> Result<()> {
    with_provider(id, |p| {
        let at = p
            .accounts
            .iter()
            .position(|a| a.id == account)
            .ok_or_else(|| anyhow!("no key {account}"))?;
        let to = (at as i64 + by as i64).clamp(0, p.accounts.len() as i64 - 1) as usize;
        let a = p.accounts.remove(at);
        p.accounts.insert(to, a);
        Ok(())
    })
}

/// Adds a model by hand: it stays when the vendor's list is fetched.
pub fn add_model(id: &str, model: &str) -> Result<()> {
    let model = model.trim();
    if model.is_empty() {
        bail!("the model id is empty");
    }
    with_provider(id, |p| {
        if !p.has_model(model) {
            p.models.push(Model::new(model));
        }
        if !p.manual_models.iter().any(|m| m == model) {
            p.manual_models.push(model.to_owned());
        }
        p.hidden_models.retain(|m| m != model);
        Ok(())
    })
}

pub fn remove_model(id: &str, model: &str) -> Result<()> {
    with_provider(id, |p| {
        p.models.retain(|m| m.id != model);
        p.manual_models.retain(|m| m != model);
        Ok(())
    })
}

/// Hides a model from pickers, or shows it again.
pub fn set_model_hidden(id: &str, model: &str, hidden: bool) -> Result<()> {
    with_provider(id, |p| {
        p.hidden_models.retain(|m| m != model);
        if hidden {
            p.hidden_models.push(model.to_owned());
        }
        Ok(())
    })
}

/// Fetches the vendor's model list into the provider; how many it has now.
pub async fn refresh_models(id: &str) -> Result<usize> {
    let provider = config::load_providers()?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| anyhow!("there is no provider {id}"))?;
    let fetched = models::fetch(&provider).await?;
    with_provider(id, |p| {
        p.models = models::merge(&p.models, fetched, &p.manual_models);
        Ok(p.models.len())
    })
}

/// Saves a group (new when `id` is `None`); its id.
pub fn save_group(
    id: Option<&str>,
    name: &str,
    members: &[String],
    routing: Routing,
) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        bail!("a group needs a name");
    }
    if members.is_empty() {
        bail!("a group needs at least one model");
    }
    for m in members {
        if !m.contains('/') {
            bail!("{m}: members are provider/model or group/<id>");
        }
    }
    update_groups(|groups| {
        let id = match id {
            Some(id) => {
                let group = groups
                    .iter_mut()
                    .find(|g| g.id == id)
                    .ok_or_else(|| anyhow!("there is no group {id}"))?;
                if members.iter().any(|m| m == &format!("group/{id}")) {
                    bail!("a group cannot contain itself");
                }
                group.name = name.to_owned();
                group.members = members.to_vec();
                group.routing = routing;
                id.to_owned()
            }
            None => {
                let base = slug(name);
                let base = if base.is_empty() {
                    "group".to_owned()
                } else {
                    base
                };
                let id = free_id(&base, |id| groups.iter().any(|g| g.id == id));
                groups.push(Group {
                    id: id.clone(),
                    name: name.to_owned(),
                    members: members.to_vec(),
                    routing,
                    strategy: None,
                });
                id
            }
        };
        Ok(id)
    })
}

pub fn remove_group(id: &str) -> Result<()> {
    update_groups(|groups| {
        let before = groups.len();
        groups.retain(|g| g.id != id);
        if groups.len() == before {
            bail!("there is no group {id}");
        }
        for g in groups.iter_mut() {
            g.members.retain(|m| m != &format!("group/{id}"));
        }
        Ok(())
    })
}

/// Sets the middleware routing (`<plugin>/<name>`) that orders a group's
/// members; `None` for its own routing again.
pub fn set_group_strategy(id: &str, strategy: Option<&str>) -> Result<()> {
    let strategy = strategy.map(str::trim).filter(|s| !s.is_empty());
    if let Some(s) = strategy {
        let ok = s.split_once('/').is_some_and(|(a, b)| {
            let name = |t: &str| {
                !t.is_empty()
                    && t.chars()
                        .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
            };
            name(a) && name(b)
        });
        if !ok {
            bail!("{s}: a middleware's routing is <plugin>/<name>");
        }
    }
    update_groups(|groups| {
        let group = groups
            .iter_mut()
            .find(|g| g.id == id)
            .ok_or_else(|| anyhow!("there is no group {id}"))?;
        group.strategy = strategy.map(str::to_owned);
        Ok(())
    })
}

/// Every model an agent can be given: `provider/model` of each enabled
/// provider, then the groups.
pub fn choices() -> Result<Vec<Choice>> {
    let config = config::Config::load()?;
    let mut out = Vec::new();
    for p in config.providers.iter().filter(|p| p.enabled) {
        for m in p.visible_models() {
            out.push(Choice {
                id: format!("{}/{}", p.id, m.id),
                provider: p.name.clone(),
                name: if m.name.is_empty() {
                    m.id.clone()
                } else {
                    m.name.clone()
                },
                group: false,
            });
        }
    }
    for g in &config.groups {
        out.push(Choice {
            id: format!("group/{}", g.id),
            provider: String::new(),
            name: g.name.clone(),
            group: true,
        });
    }
    Ok(out)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Choice {
    pub id: String,
    pub provider: String,
    pub name: String,
    pub group: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_has_a_proxy_of_its_own() {
        let home = tempfile::tempdir().unwrap();
        let _lock = crate::subscription::tests::scratch_home(home.path());
        let id = add_custom("Acme", Protocol::Chat, "https://acme.example/v1", None, &[]).unwrap();
        let first = add_key(&id, "sk-one", "").unwrap();
        let second = add_key(&id, "sk-two", "").unwrap();
        set_proxy(&id, "http://127.0.0.1:8080").unwrap();
        set_key_proxy(&id, &second, " socks5://127.0.0.1:1080 ").unwrap();
        let config = crate::config::Config::load().unwrap();
        let p = config.provider(&id).unwrap();
        assert_eq!(p.proxy.as_deref(), Some("http://127.0.0.1:8080"));
        let of = |account: &str| {
            p.accounts
                .iter()
                .find(|a| a.id == account)
                .unwrap()
                .proxy
                .clone()
        };
        assert_eq!(of(&first), None);
        assert_eq!(of(&second).as_deref(), Some("socks5://127.0.0.1:1080"));

        // Not an address: refused, and what was there stays.
        let error = set_key_proxy(&id, &second, "not a proxy")
            .unwrap_err()
            .to_string();
        assert!(error.contains("the proxy not a proxy"), "{error}");
        assert!(set_key_proxy(&id, "nobody", "direct").is_err());
        set_key_proxy(&id, &second, "direct").unwrap();
        set_key_proxy(&id, &first, "").unwrap();
        let config = crate::config::Config::load().unwrap();
        let p = config.provider(&id).unwrap();
        assert_eq!(p.accounts[1].proxy.as_deref(), Some("direct"));
        assert_eq!(p.accounts[0].proxy, None);
    }

    #[test]
    fn free_ids_count_up() {
        let taken = ["a", "a-2"];
        assert_eq!(free_id("a", |id| taken.contains(&id)), "a-3");
        assert_eq!(free_id("b", |id| taken.contains(&id)), "b");
    }
}
