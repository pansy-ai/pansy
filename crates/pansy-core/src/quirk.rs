//! What vendors' models refuse, as rules (`quirks.toml`): keys to rename,
//! drop, set or hold down in a request the gateway translated, by provider,
//! model and protocol. pansy's own are built in; the user's are
//! `quirks.toml` in pansy's folder, read again when the file changes, so a
//! vendor's new refusal needs no new pansy.
//!
//! A request that passes through, in the vendor's own protocol, is never
//! touched by a rule.

use std::path::Path;
use std::sync::Mutex;
use std::time::SystemTime;

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::paths;
use crate::provider::Protocol;

/// pansy's own rules.
pub const BUILT_IN: &str = include_str!("quirks.toml");

/// One or several of something, as a rule may write it.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum Some1<T> {
    One(T),
    Many(Vec<T>),
}

impl<T> Some1<T> {
    fn list(self) -> Vec<T> {
        match self {
            Some1::One(one) => vec![one],
            Some1::Many(many) => many,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRule {
    provider: Option<Some1<String>>,
    model: Option<Some1<String>>,
    protocol: Option<String>,
    when: Option<String>,
    #[serde(default)]
    rename: Map<String, Value>,
    #[serde(default)]
    drop: Vec<String>,
    #[serde(default)]
    set: Map<String, Value>,
    #[serde(default)]
    most: Map<String, Value>,
    #[serde(default)]
    why: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    #[serde(default)]
    rule: Vec<RawRule>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    /// The providers it is for, by id or preset; all, when empty.
    pub providers: Vec<String>,
    /// The models it is for, `*` any run of characters; all, when empty.
    pub models: Vec<String>,
    pub protocol: Option<Protocol>,
    /// A key the request must have for the rule to count.
    pub when: Option<String>,
    pub rename: Vec<(String, String)>,
    pub drop: Vec<String>,
    pub set: Vec<(String, Value)>,
    pub most: Vec<(String, f64)>,
    pub why: String,
    /// One of the user's, not built in.
    pub own: bool,
}

/// Reads rules from a file's text.
pub fn parse(text: &str, own: bool) -> Result<Vec<Rule>> {
    let file: RawFile = toml_edit::de::from_str(text).map_err(|e| anyhow!("{}", e.message()))?;
    let mut rules = Vec::new();
    for (n, raw) in file.rule.into_iter().enumerate() {
        let n = n + 1;
        let protocol = match raw.protocol.as_deref() {
            None => None,
            Some(p) => Some(Protocol::parse(p).ok_or_else(|| {
                anyhow!("rule {n}: no protocol {p}: anthropic, chat, responses or gemini")
            })?),
        };
        let mut rename = Vec::new();
        for (from, to) in raw.rename {
            let Value::String(to) = to else {
                bail!("rule {n}: rename.{from} is not a key's name");
            };
            rename.push((from, to));
        }
        let mut most = Vec::new();
        for (key, value) in raw.most {
            let Some(value) = value.as_f64() else {
                bail!("rule {n}: most.{key} is not a number");
            };
            most.push((key, value));
        }
        let rule = Rule {
            providers: raw.provider.map(Some1::list).unwrap_or_default(),
            models: raw.model.map(Some1::list).unwrap_or_default(),
            protocol,
            when: raw.when,
            rename,
            drop: raw.drop,
            set: raw.set.into_iter().collect(),
            most,
            why: raw.why,
            own,
        };
        if rule.rename.is_empty()
            && rule.drop.is_empty()
            && rule.set.is_empty()
            && rule.most.is_empty()
        {
            bail!("rule {n} does nothing: give it rename, drop, set or most");
        }
        rules.push(rule);
    }
    Ok(rules)
}

/// Whether `text` matches `pattern`, where `*` is any run of characters;
/// letters match whatever their case.
pub fn glob(pattern: &str, text: &str) -> bool {
    fn at(p: &[u8], t: &[u8]) -> bool {
        match p.split_first() {
            None => t.is_empty(),
            Some((b'*', rest)) => (0..=t.len()).any(|skip| at(rest, &t[skip..])),
            Some((c, rest)) => t
                .split_first()
                .is_some_and(|(d, more)| c.eq_ignore_ascii_case(d) && at(rest, more)),
        }
    }
    at(pattern.as_bytes(), text.as_bytes())
}

fn path_of(key: &str) -> Vec<&str> {
    key.split('.').filter(|p| !p.is_empty()).collect()
}

fn get<'a>(body: &'a Value, key: &str) -> Option<&'a Value> {
    path_of(key).iter().try_fold(body, |at, part| at.get(part))
}

/// The table that holds `key`'s last part, and that part.
fn parent<'a, 'k>(
    body: &'a mut Value,
    key: &'k str,
    make: bool,
) -> Option<(&'a mut Map<String, Value>, &'k str)> {
    let path = path_of(key);
    let (last, above) = path.split_last()?;
    let mut at = body;
    for part in above {
        let map = at.as_object_mut()?;
        if make && !map.contains_key(*part) {
            map.insert((*part).to_owned(), Value::Object(Map::new()));
        }
        at = map.get_mut(*part)?;
    }
    Some((at.as_object_mut()?, last))
}

fn take(body: &mut Value, key: &str) -> Option<Value> {
    let (map, last) = parent(body, key, false)?;
    map.shift_remove(last)
}

fn put(body: &mut Value, key: &str, value: Value) -> bool {
    match parent(body, key, true) {
        Some((map, last)) => {
            let changed = map.get(last) != Some(&value);
            map.insert(last.to_owned(), value);
            changed
        }
        None => false,
    }
}

impl Rule {
    /// Whether the rule is for this provider (its id, or its preset's), model
    /// and protocol.
    pub fn is_for(
        &self,
        provider: &str,
        preset: Option<&str>,
        model: &str,
        protocol: Protocol,
    ) -> bool {
        let provider_ok = self.providers.is_empty()
            || self
                .providers
                .iter()
                .any(|p| p == provider || Some(p.as_str()) == preset);
        // A model may be named with its vendor before it (`openai/gpt-5`).
        let bare = model.rsplit('/').next().unwrap_or(model);
        let model_ok =
            self.models.is_empty() || self.models.iter().any(|m| glob(m, model) || glob(m, bare));
        provider_ok && model_ok && self.protocol.is_none_or(|p| p == protocol)
    }

    /// Does what the rule says to `body`; whether anything changed.
    pub fn apply(&self, body: &mut Value) -> bool {
        if let Some(when) = &self.when
            && get(body, when).is_none_or(Value::is_null)
        {
            return false;
        }
        let mut changed = false;
        for (from, to) in &self.rename {
            if let Some(value) = take(body, from) {
                put(body, to, value);
                changed = true;
            }
        }
        for key in &self.drop {
            changed |= take(body, key).is_some();
        }
        for (key, value) in &self.set {
            changed |= put(body, key, value.clone());
        }
        for (key, most) in &self.most {
            if let Some(now) = get(body, key).and_then(Value::as_f64)
                && now > *most
            {
                // A whole number stays one.
                let held = if most.fract() == 0.0 {
                    Value::from(*most as u64)
                } else {
                    Value::from(*most)
                };
                changed |= put(body, key, held);
            }
        }
        changed
    }
}

/// The user's rules file.
pub fn own_file() -> std::path::PathBuf {
    paths::home().join("quirks.toml")
}

fn built_in() -> Vec<Rule> {
    parse(BUILT_IN, false).expect("pansy's own quirks.toml")
}

fn changed_at(file: &Path) -> Option<SystemTime> {
    std::fs::metadata(file).and_then(|m| m.modified()).ok()
}

/// The user's rules, or what is wrong with their file.
pub fn own() -> Result<Vec<Rule>> {
    match std::fs::read_to_string(own_file()) {
        Ok(text) => parse(&text, true).map_err(|e| anyhow!("{}: {e}", own_file().display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(anyhow!("{}: {e}", own_file().display())),
    }
}

/// Every rule: pansy's own, then the user's. Read once for a file as it is;
/// a file of the user's that can't be read counts for nothing ([`own`] says
/// why).
pub fn rules() -> std::sync::Arc<Vec<Rule>> {
    type Kept = (
        std::path::PathBuf,
        Option<SystemTime>,
        std::sync::Arc<Vec<Rule>>,
    );
    static KEPT: Mutex<Option<Kept>> = Mutex::new(None);
    let file = own_file();
    let at = changed_at(&file);
    let mut kept = KEPT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((f, t, rules)) = kept.as_ref()
        && *f == file
        && *t == at
    {
        return std::sync::Arc::clone(rules);
    }
    let mut all = built_in();
    match own() {
        Ok(mine) => all.extend(mine),
        Err(e) => tracing::warn!("{e:#}"),
    }
    let all = std::sync::Arc::new(all);
    *kept = Some((file, at, std::sync::Arc::clone(&all)));
    all
}

/// Applies every rule for the provider, model and protocol to a translated
/// request; the reasons of those that changed it.
pub fn apply(
    body: &mut Value,
    provider: &str,
    preset: Option<&str>,
    model: &str,
    protocol: Protocol,
) -> Vec<String> {
    let mut applied = Vec::new();
    for rule in rules().iter() {
        if rule.is_for(provider, preset, model, protocol) && rule.apply(body) {
            applied.push(rule.why.clone());
        }
    }
    applied
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn globs() {
        assert!(glob("gpt-5*", "gpt-5.1-codex"));
        assert!(glob("GPT-5*", "gpt-5"));
        assert!(glob("*-reasoner", "deepseek-reasoner"));
        assert!(glob("o3*", "o3"));
        assert!(!glob("o3*", "gpt-4o3"));
        assert!(glob("*", ""));
        assert!(!glob("kimi-k2", "kimi-k2-0905"));
    }

    #[test]
    fn pansys_own_rules_read_and_fit() {
        let rules = built_in();
        assert!(!rules.is_empty());
        assert!(rules.iter().all(|r| !r.own && !r.why.is_empty()));
        // OpenAI's reasoning models lose the temperature; its others keep it.
        let mut body = json!({"model": "gpt-5", "temperature": 0.2, "top_p": 0.9, "messages": []});
        let said =
            |body: &mut Value, provider: &str, preset: Option<&str>, model: &str, protocol| {
                rules
                    .iter()
                    .filter(|r| r.is_for(provider, preset, model, protocol) && r.apply(body))
                    .count()
            };
        assert_eq!(
            said(&mut body, "openai", Some("openai"), "gpt-5", Protocol::Chat),
            1
        );
        assert_eq!(body, json!({"model": "gpt-5", "messages": []}));
        let mut body = json!({"model": "gpt-4.1", "temperature": 0.2});
        assert_eq!(
            said(
                &mut body,
                "openai",
                Some("openai"),
                "gpt-4.1",
                Protocol::Chat
            ),
            0
        );
        assert_eq!(body["temperature"], 0.2);
        // A provider of the user's made from the preset, under another id.
        let mut body = json!({"temperature": 0.2});
        assert_eq!(
            said(
                &mut body,
                "work",
                Some("openai"),
                "o3-mini",
                Protocol::Responses
            ),
            1
        );
        // Another vendor's gpt-5 is not OpenAI's.
        let mut body = json!({"temperature": 0.2});
        assert_eq!(
            said(
                &mut body,
                "openrouter",
                Some("openrouter"),
                "openai/gpt-5",
                Protocol::Chat
            ),
            0
        );
        // Anthropic with thinking on, and without.
        let mut body =
            json!({"thinking": {"type": "enabled"}, "temperature": 0.2, "max_tokens": 9});
        assert_eq!(
            said(
                &mut body,
                "anthropic",
                Some("anthropic"),
                "claude",
                Protocol::Anthropic
            ),
            1
        );
        assert_eq!(
            body,
            json!({"thinking": {"type": "enabled"}, "max_tokens": 9})
        );
        let mut body = json!({"temperature": 0.2});
        assert_eq!(
            said(
                &mut body,
                "anthropic",
                Some("anthropic"),
                "claude",
                Protocol::Anthropic
            ),
            0
        );
    }

    #[test]
    fn a_rule_renames_drops_sets_and_holds_down() {
        let rules = parse(
            r#"
[[rule]]
provider = ["acme", "acme-cn"]
model = "*/big-*"
protocol = "chat"
when = "tools"
rename = { max_tokens = "max_completion_tokens" }
drop = ["stream_options.include_usage", "nothing.here"]
set = { "reasoning.enabled" = true, parallel_tool_calls = false }
most = { max_completion_tokens = 8192, temperature = 1.5 }
why = "acme"
"#,
            true,
        )
        .unwrap();
        let rule = &rules[0];
        assert!(rule.own);
        assert!(rule.is_for("acme-cn", None, "x/big-1", Protocol::Chat));
        assert!(!rule.is_for("acme", None, "x/big-1", Protocol::Anthropic));
        assert!(!rule.is_for("acme", None, "small", Protocol::Chat));
        assert!(!rule.is_for("other", Some("other"), "x/big-1", Protocol::Chat));

        let mut body = json!({
            "max_tokens": 20000, "temperature": 2.0, "tools": [1],
            "stream_options": {"include_usage": true},
        });
        assert!(rule.apply(&mut body));
        assert_eq!(
            body,
            json!({
                "temperature": 1.5, "tools": [1], "stream_options": {},
                "max_completion_tokens": 8192,
                "reasoning": {"enabled": true}, "parallel_tool_calls": false,
            })
        );
        assert!(!rule.apply(&mut body), "nothing left to change");
        // Without the key `when` names, the rule lets the request be.
        let mut plain = json!({"max_tokens": 20000});
        assert!(!rule.apply(&mut plain));
        assert_eq!(plain, json!({"max_tokens": 20000}));
    }

    #[test]
    fn says_what_is_wrong_with_a_rule() {
        for (text, says) in [
            ("[[rule]]\nwhy = \"x\"\n", "does nothing"),
            (
                "[[rule]]\nprotocol = \"smoke\"\ndrop = [\"a\"]\n",
                "no protocol smoke",
            ),
            ("[[rule]]\nmost = { a = \"b\" }\n", "not a number"),
            ("[[rule]]\ndorp = [\"a\"]\n", "dorp"),
        ] {
            let error = parse(text, true).unwrap_err().to_string();
            assert!(error.contains(says), "{text:?}: {error}");
        }
    }
}
