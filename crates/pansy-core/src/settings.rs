//! pansy's own settings (`settings.json`). Every key has a default, so a
//! missing or older file reads as one with the defaults.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The gateway's port: P-A-N-S on a phone's keypad.
pub const DEFAULT_PORT: u16 = 7267;

/// Points `builder` at `proxy`: empty for the system's (reqwest reads
/// `HTTPS_PROXY` and the like), `direct` for none, else that proxy.
pub fn use_proxy(
    builder: reqwest::ClientBuilder,
    proxy: &str,
) -> Result<reqwest::ClientBuilder, String> {
    match proxy.trim() {
        "" => Ok(builder),
        "direct" => Ok(builder.no_proxy()),
        proxy => Ok(builder
            .proxy(reqwest::Proxy::all(proxy).map_err(|e| format!("the proxy {proxy}: {e}"))?)),
    }
}

/// Which of the gateway's models one agent lists (Agents › Model list ›
/// Pick). A model kept from an agent still answers when it asks for it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentModels {
    /// The models taken out of its list, as `provider/model`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hidden: Vec<String>,
    /// Only the models in `picked` are listed: one that comes later, of any
    /// provider, stays off until it is picked.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub only_picked: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub picked: Vec<String>,
}

impl AgentModels {
    /// Whether the agent lists `model`.
    pub fn lists(&self, model: &str) -> bool {
        if self.only_picked {
            self.picked.iter().any(|m| m == model)
        } else {
            !self.hidden.iter().any(|m| m == model)
        }
    }

    /// Whether it says nothing: every model listed.
    pub fn is_empty(&self) -> bool {
        !self.only_picked && self.hidden.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub port: u16,
    /// Listen on every interface, not only this computer's, for other
    /// computers of the local network; they need `gatewayKey`.
    pub lan: bool,
    /// The key LAN clients send (`Authorization: Bearer …`, `x-api-key`).
    /// Requests from this computer need none.
    pub gateway_key: String,
    /// `system`, `light` or `dark`.
    pub theme: String,
    /// `system`, `en` or `zh-Hans`.
    pub language: String,
    /// The proxy for every upstream request: empty for the system's
    /// (`HTTPS_PROXY` and the like), `direct` for none, or an HTTP or SOCKS
    /// proxy's address. A provider's or an account's own proxy comes first.
    pub proxy: String,
    /// Keep the window's page in the tray when it is closed.
    pub close_to_tray: bool,
    /// What a click on the tray icon opens: `panel`, the quick panel by the
    /// icon, or `window`, the main window.
    pub tray_click: String,
    /// Seconds a key that hit its rate limit rests before it is tried again.
    pub cooldown_secs: u64,
    /// How many accounts or members one request may try.
    pub max_attempts: u32,
    /// Seconds to wait for an upstream's first byte.
    pub first_byte_timeout_secs: u64,
    /// The window's sidebar shows; hidden, its pages are tabs in the title
    /// bar (the title bar's sidebar button).
    pub sidebar: bool,
    /// The window's pages the sidebar leaves out.
    pub hidden_pages: Vec<String>,
    /// `/v1/models` lists every model by its bare id too, for agents that
    /// take no `provider/` before it.
    pub bare_model_ids: bool,
    /// Web pages (`https://host`) that may call the gateway from a browser,
    /// with the gateway key. Any other page is refused.
    pub origins: Vec<String>,
    /// What is masked on the way to a vendor.
    pub masking: pansy_mask::Options,
    /// Tell the user the first time a kind of value is masked.
    pub mask_alert: bool,
    /// The Agents page's order, by agent id; one it doesn't name follows.
    pub agent_order: Vec<String>,
    /// The agents put away under Hidden on the Agents page (their configs
    /// as they are).
    pub agents_hidden: Vec<String>,
    /// Serve the subscriptions the agents are signed in to (Codex's
    /// ChatGPT, Copilot's GitHub), reading their sign-ins where they keep
    /// them. Off, only the accounts signed in from pansy are.
    pub agent_sign_ins: bool,
    /// The other apps' provider configurations the Providers page no longer
    /// offers to import, by their fingerprints (`pansy-agents`' `import`).
    pub import_ignored: Vec<String>,
    /// The Plugins page's 「国内镜像」 switch, magpie's: the plugin list,
    /// Bun and plugins' packages asked of mirrors in China first
    /// (jsDelivr, npmmirror), their official addresses after.
    pub china_mirror: bool,
    /// Where API keys are kept: `file` (`providers.json`, 0600) or `system`
    /// (the login keyring or Keychain; the file then says only where).
    pub key_store: String,
    /// A model is listed with its provider's name before its own
    /// (`DeepSeek · DeepSeek V3`), in the gateway's model list and the lists
    /// pansy writes into the agents' files; off, by its own name alone.
    pub provider_in_names: bool,
    /// The agents that have it their own way, by agent id.
    pub agent_names: BTreeMap<String, bool>,
    /// Which models each agent lists, by agent id; one not here lists all.
    pub agent_models: BTreeMap<String, AgentModels>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            port: DEFAULT_PORT,
            lan: false,
            gateway_key: String::new(),
            theme: "system".to_owned(),
            language: "system".to_owned(),
            proxy: String::new(),
            close_to_tray: true,
            tray_click: "panel".to_owned(),
            cooldown_secs: 60,
            max_attempts: 6,
            first_byte_timeout_secs: 300,
            sidebar: true,
            hidden_pages: Vec::new(),
            bare_model_ids: false,
            origins: Vec::new(),
            masking: pansy_mask::Options::default(),
            mask_alert: true,
            agent_order: Vec::new(),
            agents_hidden: Vec::new(),
            agent_sign_ins: true,
            import_ignored: Vec::new(),
            china_mirror: false,
            key_store: "file".into(),
            provider_in_names: true,
            agent_names: BTreeMap::new(),
            agent_models: BTreeMap::new(),
        }
    }
}

impl Settings {
    /// The address agents are pointed at.
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Whether `agent` lists a model with its provider's name before it.
    pub fn names_provider(&self, agent: &str) -> bool {
        self.agent_names
            .get(agent)
            .copied()
            .unwrap_or(self.provider_in_names)
    }

    /// Whether `agent` lists `model` (`provider/model`).
    pub fn lists(&self, agent: &str, model: &str) -> bool {
        self.agent_models
            .get(agent)
            .is_none_or(|models| models.lists(model))
    }

    /// The keys `pansy settings set` knows, with what each holds.
    pub const KEYS: &'static [(&'static str, &'static str)] = &[
        ("port", "the gateway's port"),
        ("lan", "share the gateway on the local network (true/false)"),
        ("gatewayKey", "the key LAN clients send"),
        ("theme", "system, light or dark"),
        ("language", "system, en or zh-Hans"),
        (
            "proxy",
            "upstream requests' proxy: empty for the system's, direct, or http:// or socks5://",
        ),
        (
            "closeToTray",
            "closing the window keeps pansy in the tray (true/false)",
        ),
        (
            "trayClick",
            "a click on the tray icon opens the quick panel or the main window (panel/window)",
        ),
        ("cooldownSecs", "seconds a rate-limited key rests"),
        ("maxAttempts", "accounts or members one request may try"),
        (
            "firstByteTimeoutSecs",
            "seconds to wait for an upstream's first byte",
        ),
        (
            "sidebar",
            "the window's sidebar shows, else its pages are tabs in the title bar (true/false)",
        ),
        (
            "hiddenPages",
            "the window's pages the sidebar leaves out (a JSON list)",
        ),
        (
            "bareModelIds",
            "/v1/models lists bare model ids too (true/false)",
        ),
        (
            "origins",
            "web pages that may call the gateway (a JSON list of https://host)",
        ),
        ("masking", "what is masked on the way to a vendor (JSON)"),
        (
            "maskAlert",
            "tell the first time a kind of value is masked (true/false)",
        ),
        (
            "agentOrder",
            "the Agents page's order of agents (a JSON list of ids)",
        ),
        (
            "agentsHidden",
            "the agents hidden on the Agents page (a JSON list of ids)",
        ),
        (
            "agentSignIns",
            "serve the subscriptions Codex and Copilot are signed in to (true/false)",
        ),
        (
            "chinaMirror",
            "fetch the plugin list, Bun and plugins from mirrors in China first (true/false)",
        ),
        (
            "keyStore",
            "where API keys are kept: file (providers.json) or system (the login keyring or Keychain)",
        ),
        (
            "providerInNames",
            "a model is listed with its provider's name before its own (true/false)",
        ),
        (
            "agentNames",
            "the agents that have providerInNames their own way (JSON: {\"opencode\": false})",
        ),
        (
            "agentModels",
            "which models each agent lists (JSON: {\"opencode\": {\"hidden\": [\"provider/model\"]}})",
        ),
    ];

    /// Sets `key` from text, checked against its type.
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        let mut json = serde_json::to_value(&*self).map_err(|e| e.to_string())?;
        let map = json.as_object_mut().unwrap();
        let current = map
            .get(key)
            .ok_or_else(|| format!("no setting named {key}"))?;
        let number = current.is_number();
        let parsed = match current {
            serde_json::Value::Bool(_) => match value {
                "true" | "on" | "yes" | "1" => serde_json::Value::Bool(true),
                "false" | "off" | "no" | "0" => serde_json::Value::Bool(false),
                _ => return Err(format!("{key} is true or false")),
            },
            serde_json::Value::Number(_) => serde_json::Value::Number(
                value
                    .parse::<u64>()
                    .map_err(|_| format!("{key} is a whole number"))?
                    .into(),
            ),
            serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
                serde_json::from_str(value).map_err(|e| format!("{key} is JSON: {e}"))?
            }
            _ => serde_json::Value::String(value.to_owned()),
        };
        map.insert(key.to_owned(), parsed);
        let mut next: Settings = serde_json::from_value(json).map_err(|e| {
            if number {
                format!("{key} is out of range")
            } else {
                format!("{key}: {e}")
            }
        })?;
        next.check()?;
        *self = next;
        Ok(())
    }

    pub fn get(&self, key: &str) -> Option<String> {
        let json = serde_json::to_value(self).ok()?;
        Some(match json.get(key)? {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        })
    }

    /// Whether a browser page from `origin` may call the gateway (with the
    /// gateway key).
    pub fn allows_origin(&self, origin: &str) -> bool {
        let origin = origin.trim_end_matches('/');
        self.origins
            .iter()
            .any(|o| o.trim_end_matches('/').eq_ignore_ascii_case(origin))
    }

    pub fn check(&mut self) -> Result<(), String> {
        if self.port < 1024 {
            return Err("the port is 1024 to 65535".into());
        }
        if self.cooldown_secs < 1 {
            return Err("cooldownSecs is 1 or more".into());
        }
        if self.max_attempts < 1 {
            return Err("maxAttempts is 1 or more".into());
        }
        if self.first_byte_timeout_secs < 10 {
            return Err("firstByteTimeoutSecs is 10 or more".into());
        }
        if !["system", "light", "dark"].contains(&self.theme.as_str()) {
            return Err("the theme is system, light or dark".into());
        }
        if !["system", "en", "zh-Hans"].contains(&self.language.as_str()) {
            return Err("the language is system, en or zh-Hans".into());
        }
        if !["panel", "window"].contains(&self.tray_click.as_str()) {
            return Err("trayClick is panel or window".into());
        }
        if !["file", "system"].contains(&self.key_store.as_str()) {
            return Err("keyStore is file or system".into());
        }
        let proxy = self.proxy.trim();
        if !proxy.is_empty()
            && proxy != "direct"
            && !["http://", "https://", "socks5://", "socks5h://"]
                .iter()
                .any(|scheme| proxy.starts_with(scheme))
        {
            return Err("the proxy is empty, direct, or starts http:// or socks5://".into());
        }
        self.proxy = proxy.to_owned();
        for origin in &mut self.origins {
            let trimmed = origin.trim().trim_end_matches('/');
            let host = trimmed
                .strip_prefix("https://")
                .or_else(|| trimmed.strip_prefix("http://"))
                .unwrap_or("");
            if host.is_empty() || host.contains('/') {
                return Err(format!(
                    "{trimmed} is not a web page's origin (https://host)"
                ));
            }
            *origin = trimmed.to_ascii_lowercase();
        }
        self.origins.dedup();
        self.hidden_pages.dedup();
        for list in [&mut self.agent_order, &mut self.agents_hidden] {
            let mut seen = std::collections::BTreeSet::new();
            list.retain(|id| seen.insert(id.clone()));
        }
        for models in self.agent_models.values_mut() {
            for list in [&mut models.hidden, &mut models.picked] {
                let mut seen = std::collections::BTreeSet::new();
                list.retain(|id| !id.trim().is_empty() && seen.insert(id.clone()));
            }
            if models.only_picked {
                models.hidden.clear();
            } else {
                models.picked.clear();
            }
        }
        self.agent_models.retain(|_, models| !models.is_empty());
        self.masking.check()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_by_type() {
        let mut s = Settings::default();
        s.set("port", "8080").unwrap();
        s.set("lan", "on").unwrap();
        s.set("theme", "dark").unwrap();
        assert_eq!((s.port, s.lan, s.theme.as_str()), (8080, true, "dark"));
        assert!(s.set("port", "abc").is_err());
        assert_eq!(s.set("port", "70000"), Err("port is out of range".into()));
        assert!(s.set("maxAttempts", "0").is_err());
        assert!(s.set("firstByteTimeoutSecs", "2").is_err());
        assert!(s.set("theme", "blue").is_err());
        assert!(s.set("nope", "1").is_err());
        assert_eq!(s.get("port").as_deref(), Some("8080"));
        assert!(s.sidebar);
        s.set("sidebar", "false").unwrap();
        assert!(!s.sidebar);
        let old: Settings = serde_json::from_str(r#"{"port": 9000}"#).unwrap();
        assert_eq!(old.cooldown_secs, 60);
    }

    #[test]
    fn sets_lists_and_objects_as_json() {
        let mut s = Settings::default();
        s.set("origins", r#"["https://Example.com/"]"#).unwrap();
        assert_eq!(s.origins, ["https://example.com"]);
        assert!(s.allows_origin("https://example.com"));
        assert!(!s.allows_origin("https://evil.example"));
        assert!(s.set("origins", r#"["example.com"]"#).is_err());
        assert!(s.set("origins", "not json").is_err());
        s.set("hiddenPages", r#"["usage"]"#).unwrap();
        assert_eq!(s.hidden_pages, ["usage"]);
        s.set("masking", r#"{"secrets": false, "words": ["Acme"]}"#)
            .unwrap();
        assert!(!s.masking.secrets);
        assert_eq!(s.masking.words, ["Acme"]);
        assert!(
            s.set("masking", r#"{"rules": [{"kind": "regex", "value": "("}]}"#)
                .is_err()
        );
    }

    #[test]
    fn an_agent_lists_the_models_left_to_it() {
        let mut s = Settings::default();
        assert!(s.lists("opencode", "a/x") && s.names_provider("opencode"));
        s.set(
            "agentModels",
            r#"{"opencode": {"hidden": ["a/x", "a/x", ""]}, "crush": {"onlyPicked": true, "picked": ["b/y"], "hidden": ["a/x"]}, "codex": {}}"#,
        )
        .unwrap();
        assert_eq!(s.agent_models["opencode"].hidden, ["a/x"]);
        assert!(!s.lists("opencode", "a/x") && s.lists("opencode", "b/y"));
        // Only the picked ones: a model that comes later stays off.
        assert!(s.lists("crush", "b/y") && !s.lists("crush", "c/new"));
        assert!(s.agent_models["crush"].hidden.is_empty());
        // One that says nothing is not kept.
        assert!(!s.agent_models.contains_key("codex"));
        assert!(s.lists("codex", "a/x"));

        s.set("providerInNames", "off").unwrap();
        s.set("agentNames", r#"{"crush": true}"#).unwrap();
        assert!(!s.names_provider("opencode") && s.names_provider("crush"));
        let old: Settings = serde_json::from_str(r#"{"port": 9000}"#).unwrap();
        assert!(old.provider_in_names && old.agent_models.is_empty());
    }

    #[test]
    fn checks_the_proxy() {
        let mut s = Settings::default();
        s.set("proxy", "direct").unwrap();
        s.set("proxy", " socks5://127.0.0.1:1080 ").unwrap();
        assert_eq!(s.proxy, "socks5://127.0.0.1:1080");
        s.set("proxy", "").unwrap();
        assert!(s.set("proxy", "ftp://x").is_err());
    }

    #[test]
    fn a_tray_click_opens_the_panel_or_the_window() {
        let mut s = Settings::default();
        assert_eq!(s.tray_click, "panel");
        s.set("trayClick", "window").unwrap();
        assert_eq!(s.tray_click, "window");
        assert!(s.set("trayClick", "menu").is_err());
        assert_eq!(s.tray_click, "window");
    }
}
