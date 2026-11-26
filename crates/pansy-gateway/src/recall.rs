//! Reasoning a client dropped, put back. DeepSeek, Kimi and GLM take a turn
//! with tool calls only with the reasoning that led to them beside it
//! (`reasoning_content`), and some agents don't keep it: Antigravity's
//! `agy` sends its tool calls back bare. The gateway keeps the reasoning of
//! each answer such a vendor gave with tool calls, by the calls' ids, and
//! puts it back on an assistant turn that comes with those calls and none
//! of its own, to the provider that wrote it alone.
//!
//! Kept in memory only, as the vendor sent it (masked values masked), the
//! newest first to stay; a restart forgets it, and the turn goes on without.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use pansy_core::provider::Protocol;
use serde_json::{Value, json};

use crate::ir::{Event, Part};
use crate::route::Attempt;

/// The most reasoning kept, in bytes, before the oldest goes.
const KEPT_BYTES: usize = 32 * 1024 * 1024;

/// Each answer's reasoning, by the provider and a tool call's id.
#[derive(Default)]
pub struct Recall {
    kept: Mutex<Kept>,
}

#[derive(Default)]
struct Kept {
    by_call: HashMap<(String, String), Arc<str>>,
    /// The keys in the order kept, each with its reasoning's size.
    order: VecDeque<((String, String), usize)>,
    bytes: usize,
}

impl Recall {
    /// Whether an attempt's vendor wants reasoning back: a Chat endpoint
    /// that takes `reasoning_content`.
    pub fn wanted(attempt: &Attempt) -> bool {
        attempt.endpoint.protocol == Protocol::Chat && attempt.quirks.reasoning_content
    }

    /// Keeps `reasoning` for each of `calls`, made by `provider`.
    pub fn keep(&self, provider: &str, calls: &[String], reasoning: &str) {
        if reasoning.is_empty() || calls.is_empty() {
            return;
        }
        let text: Arc<str> = Arc::from(reasoning);
        let mut kept = self.kept.lock().unwrap();
        // A call's id is the vendor's: the first answer with it keeps it.
        for id in calls.iter().filter(|id| !id.is_empty()) {
            let key = (provider.to_owned(), id.clone());
            if kept.by_call.contains_key(&key) {
                continue;
            }
            kept.by_call.insert(key.clone(), Arc::clone(&text));
            kept.order.push_back((key, text.len()));
            kept.bytes += text.len();
        }
        while kept.bytes > KEPT_BYTES {
            let Some((key, size)) = kept.order.pop_front() else {
                break;
            };
            kept.by_call.remove(&key);
            kept.bytes -= size;
        }
    }

    /// Puts kept reasoning back in a Chat body bound for `provider`: on each
    /// assistant turn with tool calls and no `reasoning_content`, the one
    /// kept for its first call that has one. How many turns got it back.
    pub fn put_back(&self, provider: &str, body: &mut Value) -> usize {
        let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
            return 0;
        };
        let kept = self.kept.lock().unwrap();
        if kept.by_call.is_empty() {
            return 0;
        }
        let mut put = 0;
        for message in messages {
            if message["role"] != "assistant" {
                continue;
            }
            let has_own = ["reasoning_content", "reasoning"].iter().any(|key| {
                message
                    .get(*key)
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.is_empty())
            });
            if has_own {
                continue;
            }
            let Some(calls) = message.get("tool_calls").and_then(Value::as_array) else {
                continue;
            };
            let found = calls
                .iter()
                .filter_map(|call| call.get("id").and_then(Value::as_str))
                .find_map(|id| kept.by_call.get(&(provider.to_owned(), id.to_owned())));
            if let Some(text) = found {
                message["reasoning_content"] = json!(text.as_ref());
                put += 1;
            }
        }
        put
    }
}

/// One answer's reasoning and tool calls as they come, kept when it ends.
pub struct Keeper {
    recall: Arc<Recall>,
    provider: String,
    reasoning: String,
    calls: Vec<String>,
}

impl Keeper {
    /// A keeper for an attempt whose vendor wants reasoning back; none
    /// for one that doesn't.
    pub fn of(recall: &Arc<Recall>, attempt: &Attempt) -> Option<Keeper> {
        Recall::wanted(attempt).then(|| Keeper {
            recall: Arc::clone(recall),
            provider: attempt.provider.clone(),
            reasoning: String::new(),
            calls: Vec::new(),
        })
    }

    pub fn event(&mut self, event: &Event) {
        match event {
            Event::Thinking(text) => self.reasoning.push_str(text),
            Event::ToolStart { id, .. } => self.calls.push(id.clone()),
            _ => {}
        }
    }

    pub fn parts(&mut self, parts: &[Part]) {
        for part in parts {
            match part {
                Part::Thinking { text, .. } => self.reasoning.push_str(text),
                Part::ToolCall(call) => self.calls.push(call.id.clone()),
                _ => {}
            }
        }
    }

    /// The answer ended whole: what it reasoned is kept for its calls.
    pub fn finish(self) {
        self.recall
            .keep(&self.provider, &self.calls, &self.reasoning);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(calls: &[&str], reasoning: Option<&str>) -> Value {
        let mut m = json!({"role": "assistant", "content": null,
            "tool_calls": calls.iter().map(|id| json!({"id": id, "type": "function",
                "function": {"name": "Bash", "arguments": "{}"}})).collect::<Vec<_>>()});
        if let Some(r) = reasoning {
            m["reasoning_content"] = json!(r);
        }
        m
    }

    #[test]
    fn puts_back_only_what_the_provider_wrote_for_those_calls() {
        let recall = Recall::default();
        recall.keep(
            "deepseek",
            &["call_1".into(), "call_2".into()],
            "Read it first.",
        );
        let mut body = json!({"messages": [
            {"role": "user", "content": "read it"},
            turn(&["call_2"], None),
            {"role": "tool", "tool_call_id": "call_2", "content": "x"},
            turn(&["call_9"], None),
            turn(&["call_1"], Some("Its own.")),
            {"role": "assistant", "content": "done"},
        ]});
        assert_eq!(recall.put_back("deepseek", &mut body), 1);
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages[1]["reasoning_content"], "Read it first.");
        assert!(messages[3].get("reasoning_content").is_none());
        assert_eq!(messages[4]["reasoning_content"], "Its own.");
        assert!(messages[5].get("reasoning_content").is_none());

        // Another provider's turn with the same id gets nothing.
        let mut other = json!({"messages": [turn(&["call_1"], None)]});
        assert_eq!(recall.put_back("moonshot", &mut other), 0);
        assert!(other["messages"][0].get("reasoning_content").is_none());
    }

    #[test]
    fn an_answer_without_calls_or_reasoning_is_not_kept() {
        let recall = Recall::default();
        recall.keep("deepseek", &[], "Thought.");
        recall.keep("deepseek", &["call_1".into()], "");
        assert!(recall.kept.lock().unwrap().by_call.is_empty());
    }

    #[test]
    fn the_oldest_goes_first() {
        let recall = Recall::default();
        let big = "x".repeat(KEPT_BYTES / 2 + 1);
        recall.keep("deepseek", &["a".into()], &big);
        recall.keep("deepseek", &["b".into()], &big);
        let kept = recall.kept.lock().unwrap();
        assert!(!kept.by_call.contains_key(&("deepseek".into(), "a".into())));
        assert!(kept.by_call.contains_key(&("deepseek".into(), "b".into())));
        assert_eq!(kept.bytes, big.len());
    }
}
