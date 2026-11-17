//! The four protocols the gateway speaks, each read into and written from
//! [`crate::ir`]: OpenAI Chat Completions, OpenAI Responses, Anthropic
//! Messages and Gemini.

pub mod anthropic;
pub mod chat;
pub mod gemini;
pub mod responses;

use serde_json::{Value, json};

use crate::ir::{Event, Request, Response};
use crate::sse::SseEvent;

pub use pansy_core::provider::Protocol;

/// Reads an upstream's stream into events, frame by frame.
pub trait StreamReader: Send {
    /// The events of one frame; an error event of the upstream is an `Err`.
    fn frame(&mut self, frame: &SseEvent) -> Result<Vec<Event>, String>;
    /// Whether the upstream said the answer is over.
    fn done(&self) -> bool;
}

/// Writes events as the client's stream.
pub trait StreamWriter: Send {
    /// The bytes that open the stream, if the protocol has such.
    fn begin(&mut self) -> String {
        String::new()
    }
    fn event(&mut self, event: Event) -> String;
    /// Closes what is open and ends the stream.
    fn finish(&mut self) -> String;
    /// Ends the stream with an error the client can show.
    fn error(&mut self, status: u16, message: &str) -> String;
}

/// Reads a client's request body.
pub fn parse_request(protocol: Protocol, body: &Value) -> Result<Request, String> {
    match protocol {
        Protocol::Anthropic => anthropic::parse_request(body),
        Protocol::Chat => chat::parse_request(body),
        Protocol::Responses => responses::parse_request(body),
        Protocol::Gemini => gemini::parse_request(body),
    }
}

/// The body to send upstream.
pub fn build_request(protocol: Protocol, request: &Request, model: &str, quirks: &Quirks) -> Value {
    match protocol {
        Protocol::Anthropic => anthropic::build_request(request, model),
        Protocol::Chat => chat::build_request(request, model, quirks),
        Protocol::Responses => responses::build_request(request, model),
        Protocol::Gemini => gemini::build_request(request),
    }
}

/// Reads an upstream's whole answer.
pub fn parse_response(protocol: Protocol, body: &Value) -> Result<Response, String> {
    match protocol {
        Protocol::Anthropic => anthropic::parse_response(body),
        Protocol::Chat => chat::parse_response(body),
        Protocol::Responses => responses::parse_response(body),
        Protocol::Gemini => gemini::parse_response(body),
    }
}

/// The client's whole answer.
pub fn build_response(protocol: Protocol, response: &Response, request: &Request) -> Value {
    match protocol {
        Protocol::Anthropic => anthropic::build_response(response),
        Protocol::Chat => chat::build_response(response),
        Protocol::Responses => responses::build_response(response, request),
        Protocol::Gemini => gemini::build_response(response),
    }
}

pub fn reader(protocol: Protocol) -> Box<dyn StreamReader> {
    match protocol {
        Protocol::Anthropic => Box::<anthropic::Reader>::default(),
        Protocol::Chat => Box::<chat::Reader>::default(),
        Protocol::Responses => Box::<responses::Reader>::default(),
        Protocol::Gemini => Box::<gemini::Reader>::default(),
    }
}

/// `json_array`: Gemini's stream without `alt=sse` is one JSON array.
pub fn writer(
    protocol: Protocol,
    request: &Request,
    model: &str,
    json_array: bool,
) -> Box<dyn StreamWriter> {
    match protocol {
        Protocol::Anthropic => Box::new(anthropic::Writer::new(model)),
        Protocol::Chat => Box::new(chat::Writer::new(model, request)),
        Protocol::Responses => Box::new(responses::Writer::new(model, request)),
        Protocol::Gemini => Box::new(gemini::Writer::new(model, json_array)),
    }
}

/// An error body in the client's protocol.
pub fn error_body(protocol: Protocol, status: u16, message: &str) -> Value {
    match protocol {
        Protocol::Anthropic => json!({
            "type": "error",
            "error": { "type": anthropic::error_type(status), "message": message },
        }),
        Protocol::Chat | Protocol::Responses => json!({
            "error": { "message": message, "type": openai_error_type(status), "code": Value::Null },
        }),
        Protocol::Gemini => json!({
            "error": { "code": status, "message": message, "status": gemini::error_status(status) },
        }),
    }
}

/// The frame that tells a client of `protocol` its stream failed, for a
/// stream passed through as the vendor sent it (a translated one ends with
/// its writer's own).
pub fn stream_error(protocol: Protocol, status: u16, message: &str) -> String {
    match protocol {
        Protocol::Anthropic => crate::sse::named("error", &error_body(protocol, status, message)),
        Protocol::Chat | Protocol::Gemini => {
            crate::sse::data(&error_body(protocol, status, message).to_string())
        }
        Protocol::Responses => crate::sse::named(
            "error",
            &json!({"type": "error", "code": if status == 429 { "rate_limit_exceeded" } else { "server_error" }, "message": message}),
        ),
    }
}

/// A body passed through to a vendor of the client's protocol has the seals
/// the gateway's writers gave the client: that vendor's own as they were
/// made, another's tagged (or none, from a vendor that has none). The
/// vendor gets its own back as it made them, and nothing of another's.
pub fn scrub_seals(protocol: Protocol, body: &mut Value) {
    match protocol {
        Protocol::Anthropic => anthropic::scrub_seals(body),
        Protocol::Responses => responses::scrub_seals(body),
        Protocol::Gemini => gemini::scrub_seals(body),
        Protocol::Chat => {}
    }
}

fn openai_error_type(status: u16) -> &'static str {
    match status {
        400 | 404 | 413 | 422 => "invalid_request_error",
        401 | 403 => "authentication_error",
        429 => "rate_limit_error",
        _ => "api_error",
    }
}

/// What an OpenAI-compatible vendor does its own way.
#[derive(Debug, Clone, Default)]
pub struct Quirks {
    /// Sends `max_completion_tokens` (OpenAI's newer models refuse `max_tokens`).
    pub max_completion_tokens: bool,
    /// Hands reasoning back on assistant turns as `reasoning_content`
    /// (DeepSeek and Kimi require it on turns with tool calls).
    pub reasoning_content: bool,
}

/// Reasoning seals (Anthropic's thinking signatures, OpenAI's encrypted
/// reasoning, Gemini's thought signatures) are only good with the vendor
/// that made them. In [`crate::ir`] each carries its vendor,
/// `pansy:<vendor>:<seal>`: readers tag what they read, builders hand a
/// vendor only its own, and writers give a client its own vendor's seals
/// as they were and others' tagged, so a later turn of the conversation
/// still says whose they are. A conversation moved to another model goes
/// on without the old model's seals.
pub const SIGNATURE_TAG: &str = "pansy:";

pub const ANTHROPIC: &str = "anthropic";
pub const OPENAI: &str = "openai";
pub const GEMINI: &str = "gemini";

/// `signature` as made by `vendor`, unless it already says whose it is.
pub fn tag(vendor: &str, signature: &str) -> String {
    if signature.starts_with(SIGNATURE_TAG) {
        signature.to_owned()
    } else {
        format!("{SIGNATURE_TAG}{vendor}:{signature}")
    }
}

/// The vendor's own seal, if `signature` was made by `vendor`.
pub fn untag<'a>(vendor: &str, signature: &'a str) -> Option<&'a str> {
    signature
        .strip_prefix(SIGNATURE_TAG)?
        .strip_prefix(vendor)?
        .strip_prefix(':')
}

/// What a client of `vendor`'s protocol is given: its own vendor's seal as
/// it was made, another's tagged.
pub fn for_client(vendor: &str, signature: &str) -> String {
    untag(vendor, signature).unwrap_or(signature).to_owned()
}

/// Text of a string or of an array of `{type: "text", text}` blocks.
pub(crate) fn text_of(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| match item {
                Value::String(s) => Some(s.as_str()),
                _ => item.get("text").and_then(Value::as_str),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

pub(crate) fn u64_at(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

/// Splits a `data:` URL into its media type and base64 data.
pub(crate) fn split_data_url(url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;
    let media_type = meta.strip_suffix(";base64")?;
    Some((media_type.to_owned(), data.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_passed_through_body_keeps_only_its_vendors_seals() {
        let mut body = json!({"thinking": {"type": "enabled", "budget_tokens": 2048}, "messages": [
            {"role": "user", "content": "hi"},
            {"role": "assistant", "content": [
                {"type": "thinking", "thinking": "a", "signature": "pansy:gemini:g1"},
                {"type": "tool_use", "id": "t1", "name": "Bash", "input": {}},
            ]},
            {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t1", "content": "ok"}]},
        ]});
        scrub_seals(Protocol::Anthropic, &mut body);
        assert!(!body.to_string().contains("pansy:"), "{body}");
        assert!(
            body.get("thinking").is_none(),
            "an unsigned tool turn goes on without thinking"
        );

        let mut body = json!({"messages": [{"role": "assistant", "content": [
            {"type": "thinking", "thinking": "a", "signature": "pansy:anthropic:own"},
            {"type": "thinking", "thinking": "b", "signature": "untagged-own"},
            {"type": "thinking", "thinking": "c", "signature": ""},
        ]}]});
        scrub_seals(Protocol::Anthropic, &mut body);
        let blocks = body["messages"][0]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["signature"], "own");
        assert_eq!(blocks[1]["signature"], "untagged-own");

        let mut body = json!({"input": [
            {"type": "reasoning", "summary": [], "encrypted_content": "pansy:anthropic:a1"},
            {"type": "reasoning", "summary": [], "encrypted_content": "pansy:openai:o1"},
            {"type": "message", "role": "user", "content": "hi"},
        ]});
        scrub_seals(Protocol::Responses, &mut body);
        assert_eq!(body["input"].as_array().unwrap().len(), 2);
        assert_eq!(body["input"][0]["encrypted_content"], "o1");

        let mut body = json!({"contents": [{"role": "model", "parts": [
            {"functionCall": {"name": "f", "args": {}}, "thoughtSignature": "pansy:openai:x"},
            {"text": "t", "thoughtSignature": "pansy:gemini:own"},
        ]}]});
        scrub_seals(Protocol::Gemini, &mut body);
        let parts = &body["contents"][0]["parts"];
        assert!(parts[0].get("thoughtSignature").is_none());
        assert_eq!(parts[1]["thoughtSignature"], "own");
    }
}
