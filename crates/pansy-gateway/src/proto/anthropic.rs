//! Anthropic Messages (`POST /v1/messages`): what Claude Code speaks.

use serde_json::{Map, Value, json};

use super::{ANTHROPIC, StreamReader, StreamWriter, for_client, tag, u64_at, untag};
use crate::ir::*;
use crate::sse::{self, SseEvent};

/// What `max_tokens` is when the client left it out: Anthropic requires it.
const DEFAULT_MAX_TOKENS: u64 = 32_000;

pub fn parse_request(body: &Value) -> Result<Request, String> {
    let mut req = Request {
        model: body
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        max_tokens: body.get("max_tokens").and_then(Value::as_u64),
        temperature: body.get("temperature").and_then(Value::as_f64),
        top_p: body.get("top_p").and_then(Value::as_f64),
        stream: body.get("stream").and_then(Value::as_bool).unwrap_or(false),
        user: body
            .pointer("/metadata/user_id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        ..Request::default()
    };
    match body.get("system") {
        Some(Value::String(s)) if !s.is_empty() => req.system.push(s.clone()),
        Some(Value::Array(blocks)) => {
            for block in blocks {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    req.system.push(text.to_owned());
                }
            }
        }
        _ => {}
    }
    if let Some(stops) = body.get("stop_sequences").and_then(Value::as_array) {
        req.stop = stops
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
    }
    let messages = body
        .get("messages")
        .and_then(Value::as_array)
        .ok_or("messages: expected an array")?;
    for message in messages {
        let role = match message.get("role").and_then(Value::as_str) {
            Some("assistant") => Role::Assistant,
            _ => Role::User,
        };
        let parts = match message.get("content") {
            Some(Value::String(text)) => vec![Part::Text(text.clone())],
            Some(Value::Array(blocks)) => blocks.iter().filter_map(parse_block).collect(),
            _ => Vec::new(),
        };
        req.messages.push(Message { role, parts });
    }
    if let Some(tools) = body.get("tools").and_then(Value::as_array) {
        for tool in tools {
            // Server tools (web search, code execution) have a type and no
            // schema: only Anthropic runs them.
            let Some(schema) = tool.get("input_schema") else {
                continue;
            };
            req.tools.push(Tool {
                name: tool
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                description: tool
                    .get("description")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                parameters: schema.clone(),
                custom: false,
            });
        }
    }
    if let Some(choice) = body.get("tool_choice") {
        req.tool_choice = match choice.get("type").and_then(Value::as_str) {
            Some("any") => Some(ToolChoice::Required),
            Some("none") => Some(ToolChoice::None),
            Some("tool") => choice
                .get("name")
                .and_then(Value::as_str)
                .map(|n| ToolChoice::Tool(n.to_owned())),
            Some("auto") => Some(ToolChoice::Auto),
            _ => None,
        };
        if choice
            .get("disable_parallel_tool_use")
            .and_then(Value::as_bool)
            == Some(true)
        {
            req.parallel_tool_calls = Some(false);
        }
    }
    if let Some(thinking) = body.get("thinking") {
        req.reasoning = match thinking.get("type").and_then(Value::as_str) {
            Some("enabled") => Some(Reasoning {
                budget: thinking.get("budget_tokens").and_then(Value::as_u64),
                ..Reasoning::default()
            }),
            Some("adaptive") => Some(Reasoning {
                effort: body
                    .pointer("/output_config/effort")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .or(Some("high".to_owned())),
                ..Reasoning::default()
            }),
            Some("disabled") => Some(Reasoning {
                effort: Some("none".to_owned()),
                ..Reasoning::default()
            }),
            _ => None,
        };
    }
    Ok(req)
}

fn parse_block(block: &Value) -> Option<Part> {
    let kind = block.get("type").and_then(Value::as_str)?;
    Some(match kind {
        "text" => Part::Text(
            block
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        ),
        "image" => Part::Image(parse_image(block.get("source")?)?),
        "thinking" => Part::Thinking {
            text: block
                .get("thinking")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            signature: block
                .get("signature")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(|s| tag(ANTHROPIC, s)),
        },
        "redacted_thinking" => Part::RedactedThinking(
            block
                .get("data")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        ),
        "tool_use" => Part::ToolCall(ToolCall {
            id: block
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            name: block
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            arguments: args_text(block.get("input").unwrap_or(&Value::Null)),
            custom: false,
        }),
        "tool_result" => {
            let content = match block.get("content") {
                Some(Value::String(text)) => vec![Part::Text(text.clone())],
                Some(Value::Array(items)) => items.iter().filter_map(parse_block).collect(),
                _ => Vec::new(),
            };
            Part::ToolResult(ToolResult {
                id: block
                    .get("tool_use_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                name: None,
                content,
                is_error: block
                    .get("is_error")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        }
        "document" => {
            // Plain-text documents read as text; PDFs only Anthropic reads.
            let source = block.get("source")?;
            match source.get("type").and_then(Value::as_str) {
                Some("text") => Part::Text(source.get("data").and_then(Value::as_str)?.to_owned()),
                _ => return None,
            }
        }
        _ => return None,
    })
}

fn parse_image(source: &Value) -> Option<Image> {
    match source.get("type").and_then(Value::as_str)? {
        "base64" => Some(Image::Base64 {
            media_type: source.get("media_type").and_then(Value::as_str)?.to_owned(),
            data: source.get("data").and_then(Value::as_str)?.to_owned(),
        }),
        "url" => Some(Image::Url(
            source.get("url").and_then(Value::as_str)?.to_owned(),
        )),
        _ => None,
    }
}

fn image_block(image: &Image) -> Value {
    match image {
        Image::Base64 { media_type, data } => {
            json!({"type": "image", "source": {"type": "base64", "media_type": media_type, "data": data}})
        }
        Image::Url(url) => match super::split_data_url(url) {
            Some((media_type, data)) => {
                json!({"type": "image", "source": {"type": "base64", "media_type": media_type, "data": data}})
            }
            None => json!({"type": "image", "source": {"type": "url", "url": url}}),
        },
    }
}

/// A body passed through: thinking blocks another vendor's model made (a
/// tagged seal, or none) go, Anthropic's own get their seal untagged. When
/// the last tool-using turn is left without its signed thinking, the
/// request goes on without thinking, as [`build_request`] does.
pub fn scrub_seals(body: &mut Value) {
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    let mut dropped = false;
    for message in messages.iter_mut() {
        let Some(blocks) = message.get_mut("content").and_then(Value::as_array_mut) else {
            continue;
        };
        blocks.retain_mut(|block| {
            if block.get("type").and_then(Value::as_str) != Some("thinking") {
                return true;
            }
            let signature = block
                .get("signature")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            if !signature.is_empty() && !signature.starts_with(super::SIGNATURE_TAG) {
                return true;
            }
            match untag(ANTHROPIC, &signature) {
                Some(own) if !own.is_empty() => {
                    block["signature"] = json!(own);
                    true
                }
                _ => {
                    dropped = true;
                    false
                }
            }
        });
    }
    if !dropped {
        return;
    }
    messages.retain(|m| {
        m.get("content")
            .and_then(Value::as_array)
            .is_none_or(|c| !c.is_empty())
    });
    let unsigned_tool_turn = messages
        .iter()
        .rev()
        .find(|m| m["role"] == "assistant")
        .and_then(|m| m["content"].as_array())
        .is_some_and(|blocks| {
            let has = |kind: &str| blocks.iter().any(|b| b["type"] == kind);
            has("tool_use") && !has("thinking") && !has("redacted_thinking")
        });
    if unsigned_tool_turn && let Some(body) = body.as_object_mut() {
        body.remove("thinking");
    }
}

/// A thinking block Anthropic accepts back: one it signed itself.
fn own_signature(signature: &Option<String>) -> Option<&str> {
    untag(ANTHROPIC, signature.as_deref()?)
}

fn block_of(part: &Part) -> Option<Value> {
    Some(match part {
        Part::Text(text) if text.is_empty() => return None,
        Part::Text(text) => json!({"type": "text", "text": text}),
        Part::Image(image) => image_block(image),
        Part::Thinking { text, signature } => {
            let signature = own_signature(signature)?;
            json!({"type": "thinking", "thinking": text, "signature": signature})
        }
        Part::RedactedThinking(data) => json!({"type": "redacted_thinking", "data": data}),
        Part::ToolCall(call) => {
            let input = if call.custom {
                json!({"input": call.arguments})
            } else {
                args_object(&call.arguments)
            };
            json!({"type": "tool_use", "id": call.id, "name": call.name, "input": input})
        }
        Part::ToolResult(result) => {
            let content: Vec<Value> = result.content.iter().filter_map(block_of).collect();
            let mut block =
                json!({"type": "tool_result", "tool_use_id": result.id, "content": content});
            if result.is_error {
                block["is_error"] = json!(true);
            }
            block
        }
    })
}

pub fn build_request(req: &Request, model: &str) -> Value {
    let mut messages: Vec<Value> = Vec::new();
    for message in &req.messages {
        let role = match message.role {
            Role::User => "user",
            Role::Assistant => "assistant",
        };
        let blocks: Vec<Value> = message.parts.iter().filter_map(block_of).collect();
        if blocks.is_empty() {
            continue;
        }
        // Turns of one role in a row become one.
        match messages.last_mut() {
            Some(last) if last["role"] == role => {
                last["content"].as_array_mut().unwrap().extend(blocks);
            }
            _ => messages.push(json!({"role": role, "content": blocks})),
        }
    }
    let mut body = Map::new();
    body.insert("model".into(), json!(model));
    body.insert("messages".into(), Value::Array(messages));
    if !req.system.is_empty() {
        let blocks: Vec<Value> = req
            .system
            .iter()
            .map(|s| json!({"type": "text", "text": s}))
            .collect();
        body.insert("system".into(), Value::Array(blocks));
    }
    let mut max_tokens = req.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS);
    if let Some(reasoning) = req.reasoning.as_ref().filter(|r| !r.disabled()) {
        // Anthropic wants each tool-using assistant turn to open with its own
        // signed thinking: a conversation from another model has none, so it
        // goes on without thinking.
        let unsigned_tool_turn = req
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::Assistant)
            .is_some_and(|m| {
                m.parts.iter().any(|p| matches!(p, Part::ToolCall(_)))
                    && !m.parts.iter().any(|p| match p {
                        Part::Thinking { signature, .. } => own_signature(signature).is_some(),
                        Part::RedactedThinking(_) => true,
                        _ => false,
                    })
            });
        if !unsigned_tool_turn {
            let budget = reasoning.budget().unwrap_or(10_240).max(1024);
            if max_tokens <= budget {
                max_tokens = budget + 4096;
            }
            body.insert(
                "thinking".into(),
                json!({"type": "enabled", "budget_tokens": budget}),
            );
        }
    }
    body.insert("max_tokens".into(), json!(max_tokens));
    if !body.contains_key("thinking") {
        if let Some(t) = req.temperature {
            body.insert("temperature".into(), json!(t));
        }
        if let Some(p) = req.top_p {
            body.insert("top_p".into(), json!(p));
        }
    }
    if !req.stop.is_empty() {
        body.insert("stop_sequences".into(), json!(req.stop));
    }
    if !req.tools.is_empty() {
        let tools: Vec<Value> = req
            .tools
            .iter()
            .map(|tool| {
                let mut t = json!({"name": tool.name, "input_schema": object_schema(&tool.parameters, tool.custom)});
                if let Some(d) = &tool.description {
                    t["description"] = json!(d);
                }
                t
            })
            .collect();
        body.insert("tools".into(), Value::Array(tools));
        let mut choice = match &req.tool_choice {
            Some(ToolChoice::Required) => json!({"type": "any"}),
            Some(ToolChoice::None) => json!({"type": "none"}),
            Some(ToolChoice::Tool(name)) => json!({"type": "tool", "name": name}),
            _ => json!({"type": "auto"}),
        };
        if req.parallel_tool_calls == Some(false) {
            choice["disable_parallel_tool_use"] = json!(true);
        }
        // Forcing a tool is refused with thinking on.
        if body.contains_key("thinking")
            && matches!(
                req.tool_choice,
                Some(ToolChoice::Required | ToolChoice::Tool(_))
            )
        {
            choice = json!({"type": "auto"});
        }
        body.insert("tool_choice".into(), choice);
    }
    if let Some(user) = &req.user {
        body.insert("metadata".into(), json!({"user_id": user}));
    }
    if req.stream {
        body.insert("stream".into(), json!(true));
    }
    Value::Object(body)
}

/// A tool's schema as Anthropic takes it: an object schema. A free-form
/// tool takes its text as `input`.
fn object_schema(schema: &Value, custom: bool) -> Value {
    if custom {
        return json!({"type": "object", "properties": {"input": {"type": "string"}}, "required": ["input"]});
    }
    match schema {
        Value::Object(map) if !map.is_empty() => {
            let mut map = map.clone();
            map.entry("type").or_insert(json!("object"));
            Value::Object(map)
        }
        _ => json!({"type": "object", "properties": {}}),
    }
}

pub fn parse_usage(usage: &Value) -> Usage {
    let cache_read = u64_at(usage, "cache_read_input_tokens");
    let cache_write = u64_at(usage, "cache_creation_input_tokens");
    Usage {
        input: u64_at(usage, "input_tokens") + cache_read + cache_write,
        output: u64_at(usage, "output_tokens"),
        cache_read,
        cache_write,
        reasoning: 0,
    }
}

fn usage_json(usage: &Usage) -> Value {
    json!({
        "input_tokens": usage.input.saturating_sub(usage.cache_read + usage.cache_write),
        "output_tokens": usage.output,
        "cache_creation_input_tokens": usage.cache_write,
        "cache_read_input_tokens": usage.cache_read,
    })
}

fn parse_stop(reason: Option<&str>) -> Stop {
    match reason {
        Some("max_tokens") | Some("model_context_window_exceeded") => Stop::MaxTokens,
        Some("tool_use") => Stop::ToolUse,
        Some("stop_sequence") => Stop::StopSequence,
        Some("refusal") => Stop::Refusal,
        _ => Stop::End,
    }
}

fn stop_str(stop: Stop) -> &'static str {
    match stop {
        Stop::End => "end_turn",
        Stop::MaxTokens => "max_tokens",
        Stop::ToolUse => "tool_use",
        Stop::StopSequence => "stop_sequence",
        Stop::Refusal => "refusal",
    }
}

pub fn parse_response(body: &Value) -> Result<Response, String> {
    if body.get("type").and_then(Value::as_str) == Some("error") {
        return Err(body
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("error")
            .to_owned());
    }
    let parts = body
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| blocks.iter().filter_map(parse_block).collect())
        .unwrap_or_default();
    Ok(Response {
        id: body
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        model: body
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        parts,
        stop: parse_stop(body.get("stop_reason").and_then(Value::as_str)),
        usage: body.get("usage").map(parse_usage).unwrap_or_default(),
    })
}

pub fn build_response(resp: &Response) -> Value {
    let content: Vec<Value> = resp
        .parts
        .iter()
        .filter_map(|part| match part {
            // The client hands thinking back as it got it; a foreign
            // signature tells the gateway whose it was.
            Part::Thinking { text, signature } => Some(json!({
                "type": "thinking", "thinking": text,
                "signature": signature.as_deref().map(|s| for_client(ANTHROPIC, s)).unwrap_or_default(),
            })),
            other => block_of(other),
        })
        .collect();
    json!({
        "id": if resp.id.is_empty() { new_id("msg_") } else { resp.id.clone() },
        "type": "message",
        "role": "assistant",
        "model": resp.model,
        "content": content,
        "stop_reason": stop_str(resp.stop),
        "stop_sequence": Value::Null,
        "usage": usage_json(&resp.usage),
    })
}

pub fn error_type(status: u16) -> &'static str {
    match status {
        400 | 413 | 422 => "invalid_request_error",
        401 => "authentication_error",
        403 => "permission_error",
        404 => "not_found_error",
        429 => "rate_limit_error",
        529 | 503 => "overloaded_error",
        _ => "api_error",
    }
}

/// Reads Anthropic's stream.
#[derive(Default)]
pub struct Reader {
    done: bool,
    /// The kind of each open block by index: tool deltas carry the index only.
    blocks: Vec<(u64, &'static str)>,
}

impl StreamReader for Reader {
    fn frame(&mut self, frame: &SseEvent) -> Result<Vec<Event>, String> {
        let Ok(data) = serde_json::from_str::<Value>(&frame.data) else {
            return Ok(Vec::new());
        };
        let kind = data
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or(frame.event.as_str());
        let mut out = Vec::new();
        match kind {
            "message_start" => {
                let message = &data["message"];
                out.push(Event::Start {
                    id: message
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    model: message
                        .get("model")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                });
                if let Some(usage) = message.get("usage") {
                    out.push(Event::Usage(parse_usage(usage)));
                }
            }
            "content_block_start" => {
                let index = u64_at(&data, "index");
                let block = &data["content_block"];
                let block_kind = match block.get("type").and_then(Value::as_str) {
                    Some("tool_use") => {
                        out.push(Event::ToolStart {
                            id: block
                                .get("id")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                            name: block
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                            custom: false,
                        });
                        "tool"
                    }
                    Some("thinking") => "thinking",
                    Some("redacted_thinking") => {
                        out.push(Event::RedactedThinking(
                            block
                                .get("data")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                        ));
                        "redacted"
                    }
                    Some("text") => {
                        if let Some(text) = block
                            .get("text")
                            .and_then(Value::as_str)
                            .filter(|t| !t.is_empty())
                        {
                            out.push(Event::Text(text.to_owned()));
                        }
                        "text"
                    }
                    _ => "other",
                };
                self.blocks.push((index, block_kind));
            }
            "content_block_delta" => {
                let delta = &data["delta"];
                let field = |k: &str| {
                    delta
                        .get(k)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned()
                };
                match delta.get("type").and_then(Value::as_str) {
                    Some("text_delta") => out.push(Event::Text(field("text"))),
                    Some("thinking_delta") => out.push(Event::Thinking(field("thinking"))),
                    Some("signature_delta") => out.push(Event::ThinkingSignature(tag(
                        ANTHROPIC,
                        &field("signature"),
                    ))),
                    Some("input_json_delta") => out.push(Event::ToolArgs(field("partial_json"))),
                    _ => {}
                }
            }
            "message_delta" => {
                if let Some(usage) = data.get("usage") {
                    out.push(Event::Usage(parse_usage(usage)));
                }
                if let Some(reason) = data.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    out.push(Event::Stop(parse_stop(Some(reason))));
                }
            }
            "message_stop" => self.done = true,
            "error" => {
                return Err(data
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("upstream error")
                    .to_owned());
            }
            _ => {}
        }
        Ok(out)
    }

    fn done(&self) -> bool {
        self.done
    }
}

#[derive(PartialEq, Clone, Copy)]
enum Block {
    Text,
    Thinking,
    Tool,
}

/// Writes Anthropic's stream.
pub struct Writer {
    model: String,
    id: String,
    started: bool,
    open: Option<Block>,
    index: u64,
    usage: Usage,
    stop: Option<Stop>,
    tool_used: bool,
}

impl Writer {
    pub fn new(model: &str) -> Writer {
        Writer {
            model: model.to_owned(),
            id: new_id("msg_"),
            started: false,
            open: None,
            index: 0,
            usage: Usage::default(),
            stop: None,
            tool_used: false,
        }
    }

    fn start(&mut self, out: &mut String) {
        if self.started {
            return;
        }
        self.started = true;
        out.push_str(&sse::named(
            "message_start",
            &json!({
                "type": "message_start",
                "message": {
                    "id": self.id, "type": "message", "role": "assistant", "model": self.model,
                    "content": [], "stop_reason": Value::Null, "stop_sequence": Value::Null,
                    "usage": usage_json(&self.usage),
                },
            }),
        ));
    }

    fn close(&mut self, out: &mut String) {
        if self.open.take().is_some() {
            out.push_str(&sse::named(
                "content_block_stop",
                &json!({"type": "content_block_stop", "index": self.index}),
            ));
            self.index += 1;
        }
    }

    fn open(&mut self, block: Block, content_block: Value, out: &mut String) {
        self.close(out);
        out.push_str(&sse::named(
            "content_block_start",
            &json!({"type": "content_block_start", "index": self.index, "content_block": content_block}),
        ));
        self.open = Some(block);
    }

    fn delta(&self, delta: Value, out: &mut String) {
        out.push_str(&sse::named(
            "content_block_delta",
            &json!({"type": "content_block_delta", "index": self.index, "delta": delta}),
        ));
    }
}

impl StreamWriter for Writer {
    fn event(&mut self, event: Event) -> String {
        let mut out = String::new();
        if let Event::Start { id, .. } = &event
            && !self.started
            && !id.is_empty()
            && id.starts_with("msg_")
        {
            self.id = id.clone();
        }
        if let Event::Usage(usage) = &event {
            // The first usage (the prompt's) goes in message_start.
            self.usage.merge(*usage);
        }
        self.start(&mut out);
        match event {
            Event::Start { .. } | Event::Usage(_) => {}
            Event::Text(text) => {
                if self.open != Some(Block::Text) {
                    self.open(Block::Text, json!({"type": "text", "text": ""}), &mut out);
                }
                self.delta(json!({"type": "text_delta", "text": text}), &mut out);
            }
            Event::Thinking(text) => {
                if self.open != Some(Block::Thinking) {
                    self.open(
                        Block::Thinking,
                        json!({"type": "thinking", "thinking": "", "signature": ""}),
                        &mut out,
                    );
                }
                self.delta(
                    json!({"type": "thinking_delta", "thinking": text}),
                    &mut out,
                );
            }
            Event::ThinkingSignature(signature) => {
                if self.open != Some(Block::Thinking) {
                    self.open(
                        Block::Thinking,
                        json!({"type": "thinking", "thinking": "", "signature": ""}),
                        &mut out,
                    );
                }
                self.delta(json!({"type": "signature_delta", "signature": for_client(ANTHROPIC, &signature)}), &mut out);
            }
            Event::RedactedThinking(data) => {
                self.close(&mut out);
                out.push_str(&sse::named(
                    "content_block_start",
                    &json!({"type": "content_block_start", "index": self.index,
                            "content_block": {"type": "redacted_thinking", "data": data}}),
                ));
                self.open = Some(Block::Thinking);
                self.close(&mut out);
            }
            Event::ToolStart { id, name, .. } => {
                self.tool_used = true;
                let id = if id.is_empty() { new_id("toolu_") } else { id };
                self.open(
                    Block::Tool,
                    json!({"type": "tool_use", "id": id, "name": name, "input": {}}),
                    &mut out,
                );
            }
            Event::ToolArgs(args) => {
                if self.open == Some(Block::Tool) && !args.is_empty() {
                    self.delta(
                        json!({"type": "input_json_delta", "partial_json": args}),
                        &mut out,
                    );
                }
            }
            Event::Stop(stop) => self.stop = Some(stop),
        }
        out
    }

    fn finish(&mut self) -> String {
        let mut out = String::new();
        self.start(&mut out);
        self.close(&mut out);
        let stop = match self.stop {
            Some(Stop::End) | None if self.tool_used => Stop::ToolUse,
            Some(stop) => stop,
            None => Stop::End,
        };
        out.push_str(&sse::named(
            "message_delta",
            &json!({
                "type": "message_delta",
                "delta": {"stop_reason": stop_str(stop), "stop_sequence": Value::Null},
                "usage": usage_json(&self.usage),
            }),
        ));
        out.push_str(&sse::named(
            "message_stop",
            &json!({"type": "message_stop"}),
        ));
        out
    }

    fn error(&mut self, status: u16, message: &str) -> String {
        sse::named(
            "error",
            &json!({"type": "error", "error": {"type": error_type(status), "message": message}}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_request_with_tools_and_results() {
        let body = json!({
            "model": "deepseek/deepseek-chat",
            "max_tokens": 1000,
            "system": [{"type": "text", "text": "Be brief."}],
            "messages": [
                {"role": "user", "content": "List files"},
                {"role": "assistant", "content": [
                    {"type": "thinking", "thinking": "use ls", "signature": "sig"},
                    {"type": "tool_use", "id": "toolu_1", "name": "Bash", "input": {"command": "ls"}}
                ]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_1", "content": "a.txt"}
                ]}
            ],
            "tools": [{"name": "Bash", "description": "Run", "input_schema": {"type": "object"}},
                      {"type": "web_search_20250305", "name": "web_search"}],
            "thinking": {"type": "enabled", "budget_tokens": 2048},
            "stream": true
        });
        let req = parse_request(&body).unwrap();
        assert_eq!(req.system, vec!["Be brief."]);
        assert_eq!(req.tools.len(), 1);
        assert_eq!(req.reasoning.as_ref().unwrap().budget, Some(2048));
        assert!(
            matches!(&req.messages[1].parts[1], Part::ToolCall(c) if c.arguments == r#"{"command":"ls"}"#)
        );
        assert!(matches!(&req.messages[2].parts[0], Part::ToolResult(r) if r.text() == "a.txt"));
        let back = build_request(&req, "claude-x");
        assert_eq!(back["thinking"]["budget_tokens"], 2048);
        assert_eq!(back["messages"][1]["content"][0]["signature"], "sig");
        assert!(
            matches!(&req.messages[1].parts[0], Part::Thinking { signature: Some(s), .. } if s == "pansy:anthropic:sig")
        );
    }

    #[test]
    fn drops_thinking_for_a_tool_turn_without_its_signature() {
        let req = Request {
            reasoning: Some(Reasoning {
                budget: Some(4096),
                ..Default::default()
            }),
            messages: vec![
                Message {
                    role: Role::User,
                    parts: vec![Part::Text("hi".into())],
                },
                Message {
                    role: Role::Assistant,
                    parts: vec![
                        Part::Thinking {
                            text: "x".into(),
                            signature: Some("pansy:gemini:abc".into()),
                        },
                        Part::ToolCall(ToolCall {
                            id: "1".into(),
                            name: "t".into(),
                            arguments: "{}".into(),
                            custom: false,
                        }),
                    ],
                },
            ],
            ..Default::default()
        };
        let body = build_request(&req, "claude");
        assert!(body.get("thinking").is_none());
        assert_eq!(body["messages"][1]["content"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn writes_a_stream_with_text_and_a_tool_call() {
        let mut w = Writer::new("m");
        let mut out = String::new();
        for e in [
            Event::Usage(Usage {
                input: 10,
                ..Default::default()
            }),
            Event::Text("Hi".into()),
            Event::ToolStart {
                id: "c1".into(),
                name: "Bash".into(),
                custom: false,
            },
            Event::ToolArgs("{\"a\":1}".into()),
            Event::Usage(Usage {
                output: 5,
                ..Default::default()
            }),
        ] {
            out.push_str(&w.event(e));
        }
        out.push_str(&w.finish());
        let mut d = crate::sse::Decoder::default();
        let events: Vec<String> = d
            .push(out.as_bytes())
            .into_iter()
            .map(|e| e.event)
            .collect();
        assert_eq!(
            events,
            [
                "message_start",
                "content_block_start",
                "content_block_delta",
                "content_block_stop",
                "content_block_start",
                "content_block_delta",
                "content_block_stop",
                "message_delta",
                "message_stop"
            ]
        );
        assert!(out.contains(r#""stop_reason":"tool_use""#));
        assert!(out.contains(r#""output_tokens":5"#));
    }
}
