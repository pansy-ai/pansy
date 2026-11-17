//! Providers: a vendor's endpoints, the user's accounts (API keys, or the
//! sign-ins of a subscription) with it, and its models. Saved in
//! `providers.json`, readable only by the user.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::plugin::{PluginAccount, PluginRef};
use crate::presets::{self, Preset, Region};
use crate::subscription::{Kind, SignIn};

/// The API protocols the gateway speaks and providers answer in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    /// Anthropic Messages (`/v1/messages`).
    Anthropic,
    /// OpenAI Chat Completions (`/v1/chat/completions`).
    Chat,
    /// OpenAI Responses (`/v1/responses`).
    Responses,
    /// Gemini (`/v1beta/models/{model}:generateContent`).
    Gemini,
}

impl Protocol {
    pub const ALL: [Protocol; 4] = [
        Protocol::Anthropic,
        Protocol::Chat,
        Protocol::Responses,
        Protocol::Gemini,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Protocol::Anthropic => "anthropic",
            Protocol::Chat => "chat",
            Protocol::Responses => "responses",
            Protocol::Gemini => "gemini",
        }
    }

    pub fn parse(s: &str) -> Option<Protocol> {
        match s {
            "anthropic" | "claude" | "messages" => Some(Protocol::Anthropic),
            "chat" | "openai" | "openai-chat" | "chat-completions" => Some(Protocol::Chat),
            "responses" | "openai-responses" => Some(Protocol::Responses),
            "gemini" | "google" => Some(Protocol::Gemini),
            _ => None,
        }
    }

    /// The name people know it by.
    pub fn label(self) -> &'static str {
        match self {
            Protocol::Anthropic => "Anthropic Messages",
            Protocol::Chat => "OpenAI Chat",
            Protocol::Responses => "OpenAI Responses",
            Protocol::Gemini => "Gemini",
        }
    }
}

/// Where a provider answers one protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Endpoint {
    pub protocol: Protocol,
    /// The base the protocol's path is joined to: `https://api.deepseek.com/v1`
    /// for Chat (`/chat/completions`), `https://api.deepseek.com/anthropic`
    /// for Anthropic (`/v1/messages`).
    pub base_url: String,
}

/// One API key with a provider, or one sign-in of a subscription.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    /// What the user calls it ("work", "personal"); empty shows the key's end.
    #[serde(default)]
    pub label: String,
    /// Empty for a sign-in, whose tokens stay where they are kept.
    #[serde(default)]
    pub key: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// An HTTP or SOCKS proxy for this key's requests, over the provider's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
    /// The subscription's sign-in this account is: who, on which plan, kept
    /// where. Read again from where it is kept each time pansy loads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sign_in: Option<SignIn>,
    /// The plugin's sign-in this account is: where its host keeps it, and
    /// who. Read again from the plugin's last list each time pansy loads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin: Option<PluginAccount>,
}

fn yes() -> bool {
    true
}

impl Account {
    /// The label, the signed-in user, or the key's last four characters.
    pub fn display(&self) -> String {
        if !self.label.is_empty() {
            return self.label.clone();
        }
        if let Some(s) = &self.sign_in {
            return s.user.clone();
        }
        if let Some(p) = &self.plugin {
            return [&p.user, &p.hint, &p.key]
                .into_iter()
                .find(|s| !s.is_empty())
                .cloned()
                .unwrap_or_default();
        }
        let tail: String = self
            .key
            .chars()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("…{tail}")
    }
}

/// A model a provider offers.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Context window in tokens, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<u64>,
    /// Most tokens of one answer, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output: Option<u64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reasoning: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub vision: bool,
    /// The APIs it is served on, when its vendor says (Copilot serves its
    /// newest GPT models on Responses alone); empty for any of the
    /// provider's.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub apis: Vec<Protocol>,
    /// Its id where it is asked, when that isn't `id` (a plugin's model).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_id: Option<String>,
}

impl Model {
    pub fn new(id: &str) -> Model {
        Model {
            id: id.to_owned(),
            ..Model::default()
        }
    }
}

/// The one header a key goes in on a provider's Anthropic API, for a
/// vendor that refuses it in both `x-api-key` and `Authorization`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthHeader {
    #[serde(rename = "x-api-key")]
    XApiKey,
    #[serde(rename = "bearer")]
    Bearer,
}

impl AuthHeader {
    pub fn as_str(self) -> &'static str {
        match self {
            AuthHeader::XApiKey => "x-api-key",
            AuthHeader::Bearer => "bearer",
        }
    }

    pub fn parse(s: &str) -> Option<AuthHeader> {
        match s.trim().to_ascii_lowercase().as_str() {
            "x-api-key" => Some(AuthHeader::XApiKey),
            "bearer" | "authorization" => Some(AuthHeader::Bearer),
            _ => None,
        }
    }
}

/// How an OpenAI-compatible vendor differs from OpenAI.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Quirks {
    /// `max_completion_tokens` instead of `max_tokens`, and `reasoning_effort`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub openai: bool,
    /// Reasoning goes back on assistant turns as `reasoning_content`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub reasoning_content: bool,
    /// Its Anthropic endpoint takes the key as a bearer token alone (from
    /// before `Provider::auth_header`, which says it now).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub anthropic_bearer: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provider {
    /// Its id in model names (`deepseek` in `deepseek/deepseek-chat`):
    /// lowercase letters, digits and `-`.
    pub id: String,
    pub name: String,
    /// The preset it was made from; `None` for a provider of the user's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    pub endpoints: Vec<Endpoint>,
    #[serde(default)]
    pub accounts: Vec<Account>,
    /// Models the user added by hand, and those the vendor listed last.
    #[serde(default)]
    pub models: Vec<Model>,
    /// Models the user added by hand: kept when the vendor's list is fetched.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub manual_models: Vec<String>,
    /// Models hidden from pickers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden_models: Vec<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Headers sent with every request.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
    /// The one header its key goes in on the Anthropic API; none sends it
    /// in both, as most third-party Anthropic APIs take it either way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_header: Option<AuthHeader>,
    #[serde(default)]
    pub quirks: Quirks,
    /// The subscription it serves (ChatGPT's through Codex, Copilot's):
    /// its accounts are sign-ins, found where they are kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription: Option<Kind>,
    /// The plugin that serves it (`pansy-plugins`): its accounts are the
    /// plugin's sign-ins, and its requests go through the plugin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin: Option<PluginRef>,
}

impl Provider {
    /// A provider from `preset` with one key.
    pub fn from_preset(preset: &Preset, key: Option<&str>) -> Provider {
        Provider::from_preset_at(preset, None, key)
    }

    /// A provider from `preset` at one of its plans or regions.
    pub fn from_preset_at(preset: &Preset, region: Option<&Region>, key: Option<&str>) -> Provider {
        let mut provider = Provider {
            id: preset.id.to_owned(),
            name: preset.name.to_owned(),
            preset: Some(preset.id.to_owned()),
            endpoints: preset.endpoints_at(region),
            accounts: Vec::new(),
            models: preset
                .models_at(region)
                .iter()
                .map(|m| Model::new(m))
                .collect(),
            manual_models: Vec::new(),
            hidden_models: Vec::new(),
            enabled: true,
            headers: BTreeMap::new(),
            proxy: None,
            auth_header: None,
            quirks: preset.quirks(),
            subscription: None,
            plugin: None,
        };
        if let Some(key) = key {
            provider.add_account(key, "");
        }
        provider
    }

    pub fn preset(&self) -> Option<&'static Preset> {
        presets::find(self.preset.as_deref()?)
    }

    /// The plan or region of its preset it is at, by its chat endpoint, as
    /// magpie tells it: none when it was moved elsewhere.
    pub fn region(&self) -> Option<&'static Region> {
        let chat = self.endpoint(Protocol::Chat)?.base_url.as_str();
        self.preset()?.regions.iter().find(|r| {
            r.urls
                .iter()
                .any(|(p, url)| *p == Protocol::Chat && *url == chat)
        })
    }

    /// The page its keys are made on, at its region.
    pub fn key_url(&self) -> &'static str {
        self.preset().map_or("", |p| p.key_url_at(self.region()))
    }

    /// Its vendor's site, at its region.
    pub fn website(&self) -> &'static str {
        if let Some(kind) = self.subscription {
            return kind.website();
        }
        self.preset().map_or("", |p| p.website_at(self.region()))
    }

    /// Its logo's name (`desktop/ui/logos`); `generic` for one of the user's own.
    pub fn icon(&self) -> &'static str {
        if let Some(kind) = self.subscription {
            return kind.icon();
        }
        self.preset().map_or("generic", |p| p.icon)
    }

    /// Whether its vendor's model list is asked for.
    pub fn lists_models(&self) -> bool {
        if self.plugin.is_some() {
            return false;
        }
        self.subscription.is_some() || self.preset().is_none_or(|p| p.lists_at(self.region()))
    }

    /// Azure OpenAI: made from its preset, or at a resource's host. Its key
    /// goes in `api-key`.
    pub fn is_azure(&self) -> bool {
        self.preset().is_some_and(|p| p.id == presets::AZURE)
            || self
                .endpoints
                .iter()
                .any(|e| azure_host(&host_of(&e.base_url)))
    }

    /// Amazon Bedrock's runtime or Mantle, whose Anthropic API takes the
    /// key in `x-api-key` alone.
    pub fn is_bedrock(&self) -> bool {
        self.endpoints.iter().any(|e| {
            let host = host_of(&e.base_url);
            host.starts_with("bedrock-runtime.") || host.starts_with("bedrock-mantle.")
        })
    }

    /// The one header its key goes in on the Anthropic API, or none for both:
    /// Anthropic's own, Bedrock and Azure take `x-api-key` alone; another
    /// vendor the header the user picked or pansy learned.
    pub fn anthropic_auth(&self) -> Option<AuthHeader> {
        let anthropic = self
            .endpoint(Protocol::Anthropic)
            .is_some_and(|e| host_of(&e.base_url).ends_with("anthropic.com"));
        if anthropic || self.is_bedrock() || self.is_azure() {
            return Some(AuthHeader::XApiKey);
        }
        self.auth_header
            .or(self.quirks.anthropic_bearer.then_some(AuthHeader::Bearer))
    }

    /// The address its endpoints are at (`http://localhost:11434`), for a
    /// server of the user's own.
    pub fn address(&self) -> String {
        self.endpoints
            .first()
            .and_then(|e| reqwest::Url::parse(&e.base_url).ok())
            .map(|u| u.origin().ascii_serialization())
            .unwrap_or_default()
    }

    /// Moves every endpoint to `address`, each keeping its path.
    pub fn move_to(&mut self, address: &str) -> anyhow::Result<()> {
        let origin = parse_address(address)?;
        for e in &mut self.endpoints {
            let mut url = reqwest::Url::parse(&e.base_url)
                .map_err(|err| anyhow::anyhow!("{} can't be moved: {err}", e.base_url))?;
            let _ = url.set_scheme(origin.scheme());
            let _ = url.set_host(origin.host_str());
            let _ = url.set_port(origin.port());
            e.base_url = url.as_str().trim_end_matches('/').to_owned();
        }
        Ok(())
    }

    pub fn endpoint(&self, protocol: Protocol) -> Option<&Endpoint> {
        self.endpoints.iter().find(|e| e.protocol == protocol)
    }

    pub fn protocols(&self) -> Vec<Protocol> {
        self.endpoints.iter().map(|e| e.protocol).collect()
    }

    /// Adds a key; an id is made for it.
    pub fn add_account(&mut self, key: &str, label: &str) -> &Account {
        let n = (1..)
            .find(|n| !self.accounts.iter().any(|a| a.id == format!("k{n}")))
            .unwrap();
        self.accounts.push(Account {
            id: format!("k{n}"),
            label: label.to_owned(),
            key: key.trim().to_owned(),
            enabled: true,
            proxy: None,
            sign_in: None,
            plugin: None,
        });
        self.accounts.last().unwrap()
    }

    pub fn enabled_accounts(&self) -> impl Iterator<Item = &Account> {
        self.accounts.iter().filter(|a| a.enabled)
    }

    /// Whether it needs no key: a local server (Ollama, LM Studio), or a
    /// provider of the user's own added without one.
    pub fn keyless(&self) -> bool {
        if self.subscription.is_some() || self.plugin.is_some() {
            return false;
        }
        match self.preset() {
            Some(preset) => preset.keyless,
            None => self.preset.is_none() && self.accounts.is_empty(),
        }
    }

    /// The models pickers offer.
    pub fn visible_models(&self) -> impl Iterator<Item = &Model> {
        self.models
            .iter()
            .filter(|m| !self.hidden_models.contains(&m.id))
    }

    pub fn has_model(&self, id: &str) -> bool {
        self.models.iter().any(|m| m.id == id)
    }
}

/// `https://api.deepseek.com/anthropic` → `api.deepseek.com`.
pub fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.split(['/', '?', '#'])
        .next()
        .unwrap_or(rest)
        .to_ascii_lowercase()
}

/// The hosts an Azure OpenAI resource is served at.
const AZURE_HOSTS: [&str; 3] = [
    ".openai.azure.com",
    ".cognitiveservices.azure.com",
    ".services.ai.azure.com",
];

fn azure_host(host: &str) -> bool {
    let host = host.split(':').next().unwrap_or(host);
    AZURE_HOSTS
        .iter()
        .any(|s| host.ends_with(s) && host.len() > s.len())
}

/// The v1 API of the Azure OpenAI resource an endpoint names, however it
/// was pasted from the portal: the bare resource, `…/openai`, a classic
/// deployment's URL with its api-version, or the resource's name alone.
pub fn azure_base(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let mut host = host_of(raw);
    if !host.contains(['.', ':']) {
        host.push_str(".openai.azure.com");
    }
    azure_host(&host).then(|| format!("https://{host}/openai/v1"))
}

/// What a user types for a server's address: the scheme is optional
/// (http), and a path pasted with it (`…/v1`) is dropped.
pub fn parse_address(address: &str) -> anyhow::Result<reqwest::Url> {
    let address = address.trim();
    let bad = || anyhow::anyhow!("{address:?} is not an address like http://localhost:11434");
    if address.is_empty() || address.contains(char::is_whitespace) {
        return Err(bad());
    }
    let with_scheme = if address.contains("://") {
        address.to_owned()
    } else {
        format!("http://{address}")
    };
    let url = reqwest::Url::parse(&with_scheme).map_err(|_| bad())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none_or(str::is_empty) {
        return Err(bad());
    }
    reqwest::Url::parse(&url.origin().ascii_serialization()).map_err(|_| bad())
}

/// An id made from a name: lowercase letters, digits and single dashes.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Masks a key for display: its first four and last four characters.
pub fn mask_key(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 10 {
        return "•".repeat(chars.len().max(4));
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}••••{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_and_masks() {
        assert_eq!(slug("  My Daily Coding!! "), "my-daily-coding");
        assert_eq!(slug("Kimi (Moonshot)"), "kimi-moonshot");
        assert_eq!(mask_key("sk-1234567890abcdef"), "sk-1••••cdef");
        assert_eq!(mask_key("short"), "•••••");
    }

    #[test]
    fn knows_its_region_and_key_header() {
        let ark = presets::find("volcengine").unwrap();
        let mut p = Provider::from_preset_at(ark, ark.region("agent"), Some("k"));
        assert_eq!(p.region().map(|r| r.id), Some("agent"));
        assert_eq!(p.models[0].id, "ark-code-latest");
        assert!(!p.lists_models());
        p.endpoints = ark.endpoints_at(ark.region("api"));
        assert!(p.lists_models());
        assert_eq!(p.anthropic_auth(), None);
        p.auth_header = Some(AuthHeader::Bearer);
        assert_eq!(p.anthropic_auth(), Some(AuthHeader::Bearer));
        let anthropic = Provider::from_preset(presets::find("anthropic").unwrap(), None);
        assert_eq!(anthropic.anthropic_auth(), Some(AuthHeader::XApiKey));
        let bedrock = Provider::from_preset(presets::find("bedrock").unwrap(), None);
        assert_eq!(bedrock.anthropic_auth(), Some(AuthHeader::XApiKey));
        // An old id still finds its preset, and the region it was.
        let old = Provider {
            preset: Some("siliconflow-intl".into()),
            endpoints: vec![Endpoint {
                protocol: Protocol::Chat,
                base_url: "https://api.siliconflow.com/v1".into(),
            }],
            ..Provider::from_preset(presets::find("siliconflow").unwrap(), None)
        };
        assert_eq!(old.region().map(|r| r.id), Some("intl"));
        assert_eq!(old.icon(), "siliconcloud-color");
    }

    #[test]
    fn moves_to_an_address_and_reads_azure_endpoints() {
        let mut ollama = Provider::from_preset(presets::find("ollama").unwrap(), None);
        ollama.move_to("192.168.1.20:11500/v1").unwrap();
        assert_eq!(ollama.endpoints[0].base_url, "http://192.168.1.20:11500/v1");
        assert_eq!(ollama.endpoints[1].base_url, "http://192.168.1.20:11500");
        assert_eq!(ollama.address(), "http://192.168.1.20:11500");
        assert!(ollama.move_to("not an address").is_err());
        assert_eq!(
            azure_base("https://acme.openai.azure.com/openai/deployments/gpt/chat/completions?api-version=2024-10-21").as_deref(),
            Some("https://acme.openai.azure.com/openai/v1")
        );
        assert_eq!(
            azure_base("acme").as_deref(),
            Some("https://acme.openai.azure.com/openai/v1")
        );
        assert_eq!(azure_base("https://example.com"), None);
    }

    #[test]
    fn numbers_accounts() {
        let mut p = Provider::from_preset(presets::find("deepseek").unwrap(), Some(" sk-a "));
        p.add_account("sk-b", "work");
        assert_eq!(p.accounts[0].key, "sk-a");
        assert_eq!(p.accounts[1].id, "k2");
        assert_eq!(p.accounts[1].display(), "work");
    }
}
