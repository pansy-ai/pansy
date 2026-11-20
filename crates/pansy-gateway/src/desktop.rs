//! Claude Desktop on the gateway, as magpie's gateway/desktop.go serves it.
//!
//! Pointed at a third-party gateway, Desktop lists `/v1/models` and keeps a
//! model only when its id reads as Anthropic's: it says claude, sonnet,
//! opus, haiku, fable, mythos or anthropic, and names no other vendor's
//! model (its check runs on the list, on a model typed in by hand and on
//! the model a session starts on). Every other model is shown to it by an
//! alias, `anthropic/pansy-<number>`, the number a hash of pansy's id, so it
//! stays the model's while the list changes around it; the name Desktop
//! shows is the `display_name`. Desktop offers its effort picker only for
//! a model it knows, so a model that reasons is shown by
//! `mythos-pansy-<number>` (a Claude model by
//! `pansy-<number>.anthropic.<claude model>`), which it reads as one of
//! those. A request names the alias; the gateway serves the model it
//! stands for, with the effort Desktop sends.
//!
//! Desktop also asks on models of its own choosing: its check that the
//! gateway answers, a session's title, and the Code tab's Claude Code its
//! own `claude-haiku-…` for small tasks and subagents. These go to the
//! model the session is on ([`Desktop::turn`]).

use std::sync::{LazyLock, Mutex};

use pansy_core::config::Config;
use regex::Regex;
use serde_json::{Value, json};

use crate::route;

/// The agent's id.
pub const AGENT: &str = "claude-desktop";

/// Desktop's own client: its Electron session's user agent
/// (`Mozilla/5.0 … Claude/2.7032.0 Chrome/…`).
pub fn is_desktop(user_agent: &str) -> bool {
    user_agent.starts_with("Mozilla/") && user_agent.contains(" Claude/")
}

/// A Desktop build that keeps the word mythos: 2 and up, and a request that
/// names no build (the Code tab's Claude Code). One before 2 would drop a
/// `mythos-pansy-<number>` whole, and is shown the plain alias.
fn knows_mythos(user_agent: &str) -> bool {
    static VERSION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r" Claude/(\d+)").unwrap());
    VERSION
        .captures(user_agent)
        .and_then(|c| c[1].parse::<u32>().ok())
        .is_none_or(|major| major >= 2)
}

/// Other vendors' words, which Desktop turns a model away for (2.7032's
/// list).
static DENIED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"ark-code|astron|command-r|deepseek|doubao|gemini|gemma|glm|gpt|grok|hermes|hy3|kimi|lfm|\bling\b|llama|longcat|mimo|minimax|mistral|mixtral|moonshot|nemotron|openai|phi-|qianfan|qwen|tc-code|\bunic\b|yi-|stepfun|step-3|seed-|bytedance|hunyuan|granite|amazon\.nova|nova-|devstral|ministral|ernie|codex|arcee|trinity|abab|phi\d|\bk2\.|\bm2\.|jamba|arctic|solar|mercury|zamba|kat-coder|\bds-|dpsk").unwrap()
});
static TIER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(sonnet|opus|haiku|fable|mythos)(-[\d.]+)?$").unwrap());
const WORDS: [&str; 7] = [
    "claude",
    "sonnet",
    "opus",
    "haiku",
    "fable",
    "mythos",
    "anthropic",
];

/// Desktop's check of a gateway model's id.
pub fn accepts(id: &str) -> bool {
    let id = id.to_lowercase();
    TIER.is_match(&id) || !DENIED.is_match(&id) && WORDS.iter().any(|w| id.contains(w))
}

const ALIAS: &str = "anthropic/pansy-";
const EFFORT_ALIAS: &str = "mythos-pansy-";
const CLAUDE_INFIX: &str = ".anthropic.";

/// The number of an id's alias: FNV-1a's hash, in ten digits.
fn number(id: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in id.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{:010}", hash % 10_000_000_000)
}

/// The Claude model a model's own id is, as Desktop's table of them knows
/// it (`claude-opus-4-8`: lowercased, a dotted version dashed, a date
/// taken off), or none.
fn claude_model(model: &str) -> Option<String> {
    static CLAUDE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^claude-(?:opus|sonnet|haiku|fable|mythos)-\d+(?:-\d+)?$").unwrap()
    });
    static DATED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"-\d{8}$").unwrap());
    let model = model.to_lowercase();
    let model = model
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .replace('.', "-");
    let model = DATED.replace(&model, "");
    CLAUDE.is_match(&model).then(|| model.into_owned())
}

/// The id Desktop is shown a model by: one that gets its effort picker when
/// the model reasons, else the model's own when Desktop keeps it, else its
/// alias. A group is never shown as the Claude model it starts with:
/// Desktop would name it after its own catalog.
fn shown_id(id: &str, reasoning: bool, mythos: bool) -> String {
    let n = number(id);
    if reasoning {
        if !id.starts_with("group/")
            && let Some(claude) = claude_model(id)
        {
            return format!("pansy-{n}{CLAUDE_INFIX}{claude}");
        }
        if mythos {
            return format!("{EFFORT_ALIAS}{n}");
        }
    }
    // Claude Code in the Code tab reads a dotted version (claude-opus-4.6)
    // as another model.
    if accepts(id) && !id.contains('.') && digits(id).is_none() {
        return id.to_owned();
    }
    format!("{ALIAS}{n}")
}

/// The number in one of the aliases, with or without Claude Code's `[1m]`.
fn digits(id: &str) -> Option<&str> {
    let id = id.strip_suffix("[1m]").unwrap_or(id);
    let n = if let Some(n) = id.strip_prefix(ALIAS) {
        n
    } else if let Some(n) = id.strip_prefix(EFFORT_ALIAS) {
        n
    } else {
        let rest = id.strip_prefix("pansy-")?;
        &rest[..rest.find(CLAUDE_INFIX)?]
    };
    (n.len() == 10 && n.bytes().all(|b| b.is_ascii_digit())).then_some(n)
}

/// The model an alias stands for, by pansy's id; none for an id that is
/// no alias, or one no model has any longer.
pub fn resolve(ids: &[(String, String)], requested: &str) -> Option<String> {
    let n = digits(requested)?;
    ids.iter()
        .find(|(id, _)| number(id) == n)
        .map(|(id, _)| id.clone())
}

/// `/v1/models` as Desktop is shown it: each model the user left in its
/// list, by the id it keeps, named as the agents' lists name models. Its
/// menu shows the name alone and folds rows of one name into one, so two
/// alike have pansy's id after them.
pub fn models(config: &Config, ids: &[(String, String)], user_agent: &str) -> Value {
    let mythos = knows_mythos(user_agent);
    let lower: Vec<String> = ids.iter().map(|(_, name)| name.to_lowercase()).collect();
    let data: Vec<Value> = ids
        .iter()
        .enumerate()
        .map(|(i, (id, name))| {
            let model = model_of(config, id);
            let reasoning = model.is_some_and(|m| m.reasoning);
            let name = if lower.iter().filter(|n| **n == lower[i]).count() > 1 {
                format!("{name} ({id})")
            } else {
                name.clone()
            };
            let mut row = json!({
                "type": "model",
                "id": shown_id(id, reasoning, mythos),
                "display_name": name,
                "created_at": "2026-01-01T00:00:00Z",
            });
            // Desktop adds a 1M entry beside a model of a million or more.
            if let Some(context) = model.and_then(|m| m.context) {
                row["max_input_tokens"] = json!(context);
            }
            row
        })
        .collect();
    let first = data.first().map(|m| m["id"].clone());
    let last = data.last().map(|m| m["id"].clone());
    json!({"data": data, "has_more": false, "first_id": first, "last_id": last})
}

/// A provider's model by pansy's id (`provider/model`).
fn model_of<'c>(config: &'c Config, id: &str) -> Option<&'c pansy_core::provider::Model> {
    let (provider, model) = id.split_once('/')?;
    config
        .provider(provider)?
        .models
        .iter()
        .find(|m| m.id == model)
}

/// What Desktop's requests on a model of its own choosing go to: the model
/// its session went on last, by pansy's id. Kept in memory: before the
/// first turn after the gateway starts they go to the first model listed,
/// the one a new Desktop session starts on.
#[derive(Default)]
pub struct Desktop {
    session: Mutex<String>,
}

impl Desktop {
    /// The model a request of Desktop's (or its Code tab's Claude Code,
    /// `by_claude_code`) for `asked` goes on, when not that one, and why;
    /// `first` is the first model Desktop is listed.
    ///
    /// A session's turns carry tools, and the latest one's model is the one
    /// the user picked (a subagent's is its tier's, never the session's).
    /// Desktop's check that the gateway answers goes to its default model;
    /// a title (a short request without tools on a Claude-named model)
    /// and anything on a model pansy doesn't serve go to the session's; so
    /// does a call Claude Code makes on its own by Claude's own id
    /// (`claude-haiku-4-5`), which some provider may serve while the user
    /// picked another's model.
    pub fn turn(
        &self,
        config: &Config,
        asked: &str,
        body: &Value,
        by_claude_code: bool,
        subagent: bool,
        first: Option<&str>,
    ) -> Option<(String, &'static str)> {
        let other =
            |to: Option<&str>, why| to.filter(|to| *to != asked).map(|to| (to.to_owned(), why));
        if probe(body) {
            return other(
                first,
                "Claude Desktop's check that the gateway answers, on its first model",
            );
        }
        let tools = body
            .get("tools")
            .and_then(Value::as_array)
            .is_some_and(|t| !t.is_empty());
        let mut session = self.session.lock().unwrap();
        if *session == asked {
            return None;
        }
        let aside = if by_claude_code {
            title(body)
        } else {
            body.get("max_tokens")
                .and_then(Value::as_u64)
                .is_some_and(|n| n > 0 && n <= 4096)
        };
        let titled = !tools && aside && accepts(asked);
        let gone = !route::listed(config, asked);
        let bare = asked.strip_suffix("[1m]").unwrap_or(asked);
        let own = !session.is_empty()
            && (!tools || subagent)
            && !bare.contains('/')
            && claude_model(bare).is_some();
        let picked = (!session.is_empty()).then_some(session.as_str()).or(first);
        if gone {
            return other(picked, "a model pansy doesn't serve, on the session's");
        }
        if titled {
            return other(
                picked,
                "Claude Desktop's own short call, on the session's model",
            );
        }
        if own {
            return other(picked, "Claude Code's own call, on the session's model");
        }
        if tools && !subagent {
            *session = asked.to_owned();
        }
        None
    }
}

/// Desktop's check that a gateway answers, as it starts and from Test
/// connection: `{"max_tokens":1,"messages":[{"role":"user","content":"."}]}`.
pub fn probe(body: &Value) -> bool {
    body.get("max_tokens").and_then(Value::as_u64) == Some(1)
        && body.get("system").is_none()
        && body.get("tools").is_none()
        && body
            .get("messages")
            .and_then(Value::as_array)
            .is_some_and(|m| m.len() == 1 && m[0] == json!({"role": "user", "content": "."}))
}

/// The output tokens Desktop's check is asked for upstream: OpenAI's models
/// refuse fewer than 16.
pub const PROBE_TOKENS: u64 = 16;

/// The request's system prompt is the one Desktop asks a session's title
/// with, sent by Claude Code when Desktop's own request fails.
fn title(body: &Value) -> bool {
    const PROMPT: &str = "You write short session titles.";
    match body.get("system") {
        Some(Value::String(text)) => text.starts_with(PROMPT),
        Some(Value::Array(blocks)) => blocks.iter().any(|b| {
            b.get("text")
                .and_then(Value::as_str)
                .is_some_and(|t| t.starts_with(PROMPT))
        }),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_keeps_only_ids_that_read_as_anthropics() {
        assert!(accepts("anthropic/claude-sonnet-4-5"));
        assert!(accepts("sonnet"));
        assert!(accepts("mythos-pansy-0123456789"));
        assert!(!accepts("deepseek/deepseek-chat"));
        assert!(!accepts("anthropic/deepseek/deepseek-flash"));
        assert!(!accepts("openrouter/gpt-5"));
    }

    #[test]
    fn a_model_is_shown_by_an_id_desktop_keeps_and_read_back() {
        let ids = vec![
            (
                "deepseek/deepseek-chat".to_owned(),
                "DeepSeek Chat".to_owned(),
            ),
            ("copilot/claude-opus-4.6".to_owned(), "Opus".to_owned()),
            ("a/claude-sonnet-4-5".to_owned(), "Sonnet".to_owned()),
        ];
        let alias = shown_id(&ids[0].0, false, true);
        assert!(alias.starts_with("anthropic/pansy-") && accepts(&alias));
        assert_eq!(
            resolve(&ids, &alias).as_deref(),
            Some("deepseek/deepseek-chat")
        );
        assert_eq!(
            resolve(&ids, &format!("{alias}[1m]")).as_deref(),
            Some("deepseek/deepseek-chat")
        );
        // the effort picker's aliases
        let effort = shown_id(&ids[0].0, true, true);
        assert!(effort.starts_with("mythos-pansy-"));
        assert_eq!(
            resolve(&ids, &effort).as_deref(),
            Some("deepseek/deepseek-chat")
        );
        assert!(shown_id(&ids[0].0, true, false).starts_with("anthropic/pansy-"));
        let opus = shown_id(&ids[1].0, true, true);
        assert!(opus.ends_with(".anthropic.claude-opus-4-6"), "{opus}");
        assert_eq!(
            resolve(&ids, &opus).as_deref(),
            Some("copilot/claude-opus-4.6")
        );
        // a dotted version is aliased; a Claude id Desktop keeps stays
        assert!(shown_id(&ids[1].0, false, true).starts_with("anthropic/pansy-"));
        assert_eq!(shown_id(&ids[2].0, false, true), "a/claude-sonnet-4-5");
        assert_eq!(resolve(&ids, "a/claude-sonnet-4-5"), None);
        assert_eq!(resolve(&ids, "anthropic/pansy-12"), None);
    }

    #[test]
    fn an_old_desktop_gets_no_mythos() {
        assert!(knows_mythos(
            "Mozilla/5.0 (Macintosh) Claude/2.7032.0 Chrome/138"
        ));
        assert!(!knows_mythos(
            "Mozilla/5.0 (Windows) Claude/1.6608.2 Chrome/130"
        ));
        assert!(knows_mythos("claude-cli/2.1.0 (external, claude-desktop)"));
        assert!(is_desktop(
            "Mozilla/5.0 (Macintosh) Claude/2.7032.0 Chrome/138"
        ));
        assert!(!is_desktop("Mozilla/5.0 (X11) Firefox/140"));
    }

    #[test]
    fn its_check_is_told_from_a_turn() {
        assert!(probe(
            &json!({"model": "x", "max_tokens": 1, "messages": [{"role": "user", "content": "."}]})
        ));
        assert!(!probe(
            &json!({"model": "x", "max_tokens": 1, "messages": [{"role": "user", "content": "hi"}]})
        ));
        assert!(title(
            &json!({"system": [{"type": "text", "text": "You write short session titles. Keep…"}]})
        ));
    }
}
