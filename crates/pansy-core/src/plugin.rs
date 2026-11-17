//! The providers plugins serve: OpenCode provider plugins and pi packages,
//! which `pansy-plugins` runs on Bun. Their host lists the providers they
//! sign in to, with each one's accounts and models; the last list is kept
//! in `plugin-providers.json`, and each provider signed in to is merged into
//! the providers here as a subscription's is, its accounts the plugin's
//! sign-ins. The tokens stay in `plugin-auth.json`, the host's.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::fsutil::{read_json, write_json};
use crate::paths;
use crate::provider::{Account, Endpoint, Model, Protocol, Provider, Quirks};

/// The plugin a provider is served by.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRef {
    /// The plugin, as it was added (`@scope/name`, a path).
    pub spec: String,
    /// The provider's id in the plugin, which the provider's own id is too
    /// unless pansy has another provider by it (`google-plugin`).
    pub id: String,
}

/// An account a plugin signed in to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginAccount {
    /// Where the host keeps it in `plugin-auth.json`: the provider's id for
    /// the first, `id#2` and on for the others.
    pub key: String,
    /// `oauth` or `api`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    /// Who it is, as the plugin says.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub user: String,
    /// The end of its key, for one with no name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub hint: String,
}

/// A way to sign in, as the plugin offers it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Method {
    /// `oauth` (a browser, or a code) or `api` (a key).
    #[serde(rename = "type")]
    pub kind: String,
    pub label: String,
    /// Its label by language (`zh`, `ja`).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub labels: BTreeMap<String, String>,
    /// An `api` method that needs no key typed.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub keyless: bool,
    /// An `api` method whose own questions ask for the key.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub key_asked: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub placeholder: String,
}

/// An account as the host lists it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ListedAccount {
    pub key: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub account_id: String,
    pub hint: String,
    pub list_error: String,
}

/// A model as the host lists it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ListedModel {
    pub id: String,
    pub name: String,
    /// The AI SDK package it is asked through: the protocol it speaks.
    pub npm: String,
    pub url: String,
    /// Its id at the vendor, when not `id`.
    pub api_id: String,
    pub context: u64,
    pub output: u64,
    pub reasoning: bool,
    pub image: bool,
}

/// A provider a plugin signs in to, as the host lists it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Listed {
    pub id: String,
    pub spec: String,
    pub name: String,
    pub npm: String,
    /// Where its requests go, as the plugin says (its loader may say
    /// another, which the gateway asks for).
    pub api: String,
    pub methods: Vec<Method>,
    /// One account at most.
    pub single: bool,
    /// An `https://` picture or a `data:image` URI.
    pub icon: String,
    /// How many of its requests may run at once; 0 for any.
    pub max_concurrency: u32,
    /// Its sign-ins are renewed in the background as they come due.
    pub renews: bool,
    pub accounts: Vec<ListedAccount>,
    pub models: Vec<ListedModel>,
    /// Its models couldn't be listed now: these are the ones it fell back on.
    pub list_error: String,
}

/// A plugin as the host loaded it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Loaded {
    pub spec: String,
    /// The providers it signs in to.
    pub provides: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// What the plugins' host listed last (`plugin-providers.json`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Listing {
    pub at: Option<chrono::DateTime<chrono::Utc>>,
    pub providers: Vec<Listed>,
    pub loaded: Vec<Loaded>,
}

impl Listing {
    pub fn load() -> Listing {
        read_json(&paths::plugin_providers()).unwrap_or_default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        write_json(&paths::plugin_providers(), self, true)
    }
}

/// The protocol a model asked through an AI SDK package is spoken in;
/// `None` for one pansy doesn't speak (Bedrock's, Vertex's).
pub fn protocol_of(npm: &str) -> Option<Protocol> {
    match npm {
        "@ai-sdk/openai-compatible" | "" => Some(Protocol::Chat),
        "@ai-sdk/openai" => Some(Protocol::Responses),
        "@ai-sdk/anthropic" => Some(Protocol::Anthropic),
        "@ai-sdk/google" => Some(Protocol::Gemini),
        _ => None,
    }
}

/// `listed`'s models pansy can ask, as a provider's.
fn models_of(listed: &Listed) -> Vec<Model> {
    listed
        .models
        .iter()
        .filter_map(|m| {
            let npm = if m.npm.is_empty() {
                &listed.npm
            } else {
                &m.npm
            };
            let protocol = protocol_of(npm)?;
            Some(Model {
                id: m.id.clone(),
                name: m.name.clone(),
                context: (m.context > 0).then_some(m.context),
                max_output: (m.output > 0).then_some(m.output),
                reasoning: m.reasoning,
                vision: m.image,
                apis: vec![protocol],
                api_id: (!m.api_id.is_empty() && m.api_id != m.id).then(|| m.api_id.clone()),
            })
        })
        .collect()
}

/// An endpoint for each protocol `listed`'s models speak, at the address
/// the plugin gives (the gateway asks its loader for the account's own).
fn endpoints_of(listed: &Listed, models: &[Model]) -> Vec<Endpoint> {
    fn add(out: &mut Vec<Endpoint>, protocol: Protocol, url: &str) {
        if !out.iter().any(|e| e.protocol == protocol) {
            out.push(Endpoint {
                protocol,
                base_url: url.trim_end_matches('/').to_owned(),
            });
        }
    }
    let mut out: Vec<Endpoint> = Vec::new();
    for m in models {
        let url = listed
            .models
            .iter()
            .find(|l| l.id == m.id)
            .map(|l| l.url.as_str())
            .filter(|u| !u.is_empty())
            .unwrap_or(&listed.api);
        for p in &m.apis {
            add(&mut out, *p, url);
        }
    }
    if out.is_empty()
        && let Some(p) = protocol_of(&listed.npm)
    {
        add(&mut out, p, &listed.api);
    }
    out
}

/// A provider for `listed`, new.
pub fn provider(listed: &Listed, id: &str) -> Provider {
    Provider {
        id: id.to_owned(),
        name: if listed.name.is_empty() {
            listed.id.clone()
        } else {
            listed.name.clone()
        },
        preset: None,
        endpoints: Vec::new(),
        accounts: Vec::new(),
        models: Vec::new(),
        manual_models: Vec::new(),
        hidden_models: Vec::new(),
        enabled: true,
        headers: BTreeMap::new(),
        proxy: None,
        auth_header: None,
        quirks: Quirks::default(),
        subscription: None,
        plugin: Some(PluginRef {
            spec: listed.spec.clone(),
            id: listed.id.clone(),
        }),
    }
}

/// The id a plugin's provider goes by: its own, unless another provider
/// of pansy's has it (`google-plugin`, `google-plugin-2`).
fn free_id(providers: &[Provider], want: &str) -> String {
    let taken = |id: &str| providers.iter().any(|p| p.id == id);
    let base = if taken(want) {
        format!("{want}-plugin")
    } else {
        return want.to_owned();
    };
    if !taken(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|id| !taken(id))
        .unwrap()
}

/// Puts the providers the plugins serve in `providers`: each one signed in
/// to is a provider whose accounts are its sign-ins, in the order the user
/// left them, the new ones last; its models are the plugin's. A provider a
/// plugin no longer serves, or whose last account is signed out, is gone.
pub fn merge(providers: &mut Vec<Provider>, listing: &Listing) {
    providers.retain(|p| match &p.plugin {
        Some(r) => listing
            .providers
            .iter()
            .any(|l| l.id == r.id && !l.accounts.is_empty()),
        None => true,
    });
    for listed in listing.providers.iter().filter(|l| !l.accounts.is_empty()) {
        let at = match providers
            .iter()
            .position(|p| p.plugin.as_ref().is_some_and(|r| r.id == listed.id))
        {
            Some(at) => at,
            None => {
                let id = free_id(providers, &crate::provider::slug(&listed.id));
                providers.push(provider(listed, &id));
                providers.len() - 1
            }
        };
        let p = &mut providers[at];
        p.plugin = Some(PluginRef {
            spec: listed.spec.clone(),
            id: listed.id.clone(),
        });
        let models = models_of(listed);
        if !models.is_empty() || listed.list_error.is_empty() {
            let mut all = models;
            for manual in &p.manual_models {
                if !all.iter().any(|m| &m.id == manual) {
                    all.push(Model::new(manual));
                }
            }
            p.endpoints = endpoints_of(listed, &all);
            p.models = all;
        }
        if p.endpoints.is_empty() {
            p.endpoints = endpoints_of(listed, &p.models);
        }
        p.accounts.retain(|a| {
            a.plugin
                .as_ref()
                .is_some_and(|pa| listed.accounts.iter().any(|l| l.key == pa.key))
        });
        for l in &listed.accounts {
            let plugin = PluginAccount {
                key: l.key.clone(),
                kind: l.kind.clone(),
                user: l.account_id.clone(),
                hint: l.hint.clone(),
            };
            match p
                .accounts
                .iter_mut()
                .find(|a| a.plugin.as_ref().is_some_and(|pa| pa.key == l.key))
            {
                Some(a) => {
                    a.key.clear();
                    a.plugin = Some(plugin);
                }
                None => p.accounts.push(Account {
                    id: l.key.clone(),
                    label: String::new(),
                    key: String::new(),
                    enabled: true,
                    proxy: None,
                    sign_in: None,
                    plugin: Some(plugin),
                }),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listed(id: &str, accounts: &[&str]) -> Listed {
        Listed {
            id: id.into(),
            spec: format!("@x/{id}"),
            name: format!("{id} Co"),
            npm: "@ai-sdk/openai-compatible".into(),
            api: "https://api.example.com/v1/".into(),
            accounts: accounts
                .iter()
                .map(|k| ListedAccount {
                    key: (*k).into(),
                    kind: "api".into(),
                    account_id: format!("{k}@example.com"),
                    ..Default::default()
                })
                .collect(),
            models: vec![
                ListedModel {
                    id: "chat-1".into(),
                    name: "Chat One".into(),
                    context: 1000,
                    ..Default::default()
                },
                ListedModel {
                    id: "claude-x".into(),
                    npm: "@ai-sdk/anthropic".into(),
                    api_id: "claude-x-2026".into(),
                    image: true,
                    ..Default::default()
                },
                ListedModel {
                    id: "bedrock-y".into(),
                    npm: "@ai-sdk/amazon-bedrock".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn a_plugins_provider_is_its_sign_ins() {
        let google = Provider::from_preset(crate::presets::find("google").unwrap(), Some("k"));
        let mut providers = vec![google];
        let listing = Listing {
            providers: vec![
                listed("google", &["google", "google#2"]),
                listed("nobody", &[]),
            ],
            ..Default::default()
        };
        merge(&mut providers, &listing);
        assert_eq!(providers.len(), 2);
        let p = &providers[1];
        // pansy has a google: the plugin's is another.
        assert_eq!(p.id, "google-plugin");
        assert_eq!(p.plugin.as_ref().unwrap().id, "google");
        assert_eq!(p.accounts.len(), 2);
        assert_eq!(p.accounts[1].id, "google#2");
        assert_eq!(p.accounts[1].display(), "google#2@example.com");
        // Bedrock's models aren't pansy's to ask.
        let ids: Vec<&str> = p.models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["chat-1", "claude-x"]);
        assert_eq!(p.models[1].apis, [Protocol::Anthropic]);
        assert_eq!(p.models[1].api_id.as_deref(), Some("claude-x-2026"));
        let protocols: Vec<Protocol> = p.endpoints.iter().map(|e| e.protocol).collect();
        assert_eq!(protocols, [Protocol::Chat, Protocol::Anthropic]);
        assert_eq!(p.endpoints[0].base_url, "https://api.example.com/v1");

        // The user's order and switches stay; a signed-out account goes.
        providers[1].accounts[0].enabled = false;
        providers[1].hidden_models.push("chat-1".into());
        let listing = Listing {
            providers: vec![listed("google", &["google"])],
            ..Default::default()
        };
        merge(&mut providers, &listing);
        assert_eq!(providers[1].accounts.len(), 1);
        assert!(!providers[1].accounts[0].enabled);
        assert_eq!(providers[1].hidden_models, ["chat-1"]);

        // Every account signed out, or the plugin gone: so is the provider.
        merge(&mut providers, &Listing::default());
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].id, "google");
    }

    #[test]
    fn a_list_that_failed_keeps_the_models() {
        let mut providers = Vec::new();
        merge(
            &mut providers,
            &Listing {
                providers: vec![listed("acme", &["acme"])],
                ..Default::default()
            },
        );
        let mut failed = listed("acme", &["acme"]);
        failed.models.clear();
        failed.list_error = "acme is down".into();
        merge(
            &mut providers,
            &Listing {
                providers: vec![failed],
                ..Default::default()
            },
        );
        assert_eq!(providers[0].models.len(), 2);
    }
}
