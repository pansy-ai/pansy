//! OpenAI Responses (`POST /v1/responses`): what Codex speaks.

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use super::{OPENAI, StreamReader, StreamWriter, for_client, tag, text_of, u64_at, untag};
use crate::ir::*;
use crate::proto::chat::{image_of_url, parse_tool_choice, url_of_image};
use crate::sse::{self, SseEvent};

pub fn parse_request(body: &Value) -> Result<Request, String> {
    let mut req = Request {
        model: body
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        max_tokens: body.get("max_output_tokens").and_then(Value::as_u64),
        temperature: body.get("temperature").and_then(Value::as_f64),
        top_p: body.get("top_p").and_then(Value::as_f64),
        stream: body.get("stream").and_then(Value::as_bool).unwrap_or(false),
        parallel_tool_calls: body.get("parallel_tool_calls").and_then(Value::as_bool),
        user: body
            .get("prompt_cache_key")
            .or_else(|| body.get("user"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        ..Request::default()
    };
    if let Some(instructions) = body
        .get("instructions")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        req.system.push(instructions.to_owned());
    }
    if let Some(reasoning) = body.get("reasoning").filter(|r| r.is_object()) {
        req.reasoning = Some(Reasoning {
            effort: reasoning
                .get("effort")
                .and_then(Value::as_str)
                .map(str::to_owned),
            budget: None,
            summary: reasoning
                .get("summary")
                .is_some_and(|s| !s.is_null() && s != "none"),
        });
    }
    match body.get("input") {
        Some(Value::String(text)) => push(&mut req.messages, Role::User, Part::Text(text.clone())),
        Some(Value::Array(items)) => {
            for item in items {
                read_item(item, &mut req);
            }
        }
        _ => {}
    }
    if let Some(tools) = body.get("tools").and_then(Value::as_array) {
        for tool in tools {
            match tool.get("type").and_then(Value::as_str) {
                Some("function") => req.tools.push(Tool {
                    name: tool
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    description: tool
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    parameters: tool
                        .get("parameters")
                        .cloned()
                        .unwrap_or(json!({"type": "object"})),
                    custom: false,
                }),
                Some("custom") => {
                    // The format (a grammar) is kept as the parameters; other
                    // vendors read it from the description.
                    let format = tool.get("format").cloned().unwrap_or(Value::Null);
                    let mut description = tool
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    if let Some(definition) = format.get("definition").and_then(Value::as_str) {
                        description.push_str("\n\nThe input follows this grammar:\n");
                        description.push_str(definition);
                    }
                    req.tools.push(Tool {
                        name: tool
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        description: Some(description),
                        parameters: format,
                        custom: true,
                    });
                }
                // Hosted tools (web search, file search) run at OpenAI only.
                _ => {}
            }
        }
    }
    req.tool_choice = body.get("tool_choice").and_then(parse_tool_choice);
    Ok(req)
}

fn push(messages: &mut Vec<Message>, role: Role, part: Part) {
    match messages.last_mut() {
        Some(last) if last.role == role => last.parts.push(part),
        _ => messages.push(Message {
            role,
            parts: vec![part],
        }),
    }
}

fn read_item(item: &Value, req: &mut Request) {
    let kind = item
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("message");
    let s = |k: &str| {
        item.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    match kind {
        "message" => {
            let role = item.get("role").and_then(Value::as_str).unwrap_or("user");
            let content = item.get("content").unwrap_or(&Value::Null);
            if role == "system" || role == "developer" {
                let text = text_of(content);
                if !text.is_empty() {
                    // Codex sends its instructions to the model as developer
                    // messages within the conversation: they stay there, as
                    // the user's, so their place in it is kept.
                    if req.messages.is_empty() {
                        req.system.push(text);
                    } else {
                        push(&mut req.messages, Role::User, Part::Text(text));
                    }
                }
                return;
            }
            let role = if role == "assistant" {
                Role::Assistant
            } else {
                Role::User
            };
            for part in content_parts(content) {
                push(&mut req.messages, role, part);
            }
        }
        "function_call" => push(
            &mut req.messages,
            Role::Assistant,
            Part::ToolCall(ToolCall {
                id: s("call_id"),
                name: s("name"),
                arguments: args_text(item.get("arguments").unwrap_or(&Value::Null)),
                custom: false,
            }),
        ),
        "custom_tool_call" => push(
            &mut req.messages,
            Role::Assistant,
            Part::ToolCall(ToolCall {
                id: s("call_id"),
                name: s("name"),
                arguments: s("input"),
                custom: true,
            }),
        ),
        "function_call_output" | "custom_tool_call_output" => {
            let output = item.get("output").unwrap_or(&Value::Null);
            let content = match output {
                Value::String(text) => vec![Part::Text(text.clone())],
                Value::Array(_) => content_parts(output),
                other => vec![Part::Text(other.to_string())],
            };
            push(
                &mut req.messages,
                Role::User,
                Part::ToolResult(ToolResult {
                    id: s("call_id"),
                    name: None,
                    content,
                    is_error: false,
                }),
            );
        }
        "reasoning" => {
            let mut text = String::new();
            for key in ["content", "summary"] {
                if let Some(parts) = item.get(key).and_then(Value::as_array) {
                    for part in parts {
                        if let Some(t) = part.get("text").and_then(Value::as_str) {
                            if !text.is_empty() {
                                text.push_str("\n\n");
                            }
                            text.push_str(t);
                        }
                    }
                }
                if !text.is_empty() {
                    break;
                }
            }
            let signature = item
                .get("encrypted_content")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(|s| tag(OPENAI, s));
            push(
                &mut req.messages,
                Role::Assistant,
                Part::Thinking { text, signature },
            );
        }
        _ => {}
    }
}

fn content_parts(content: &Value) -> Vec<Part> {
    match content {
        Value::String(s) if s.is_empty() => Vec::new(),
        Value::String(s) => vec![Part::Text(s.clone())],
        Value::Array(items) => items
            .iter()
            .filter_map(|item| match item.get("type").and_then(Value::as_str) {
                Some("input_text" | "output_text" | "text") => Some(Part::Text(
                    item.get("text").and_then(Value::as_str)?.to_owned(),
                )),
                Some("refusal") => Some(Part::Text(
                    item.get("refusal").and_then(Value::as_str)?.to_owned(),
                )),
                Some("input_image") => {
                    let url = item.get("image_url").and_then(Value::as_str)?;
                    Some(Part::Image(image_of_url(url)))
                }
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// A body passed through: reasoning items another vendor's model sealed
/// go, OpenAI's own get their seal untagged.
pub fn scrub_seals(body: &mut Value) {
    let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) else {
        return;
    };
    input.retain_mut(|item| {
        if item.get("type").and_then(Value::as_str) != Some("reasoning") {
            return true;
        }
        let Some(sealed) = item.get("encrypted_content").and_then(Value::as_str) else {
            return true;
        };
        if !sealed.starts_with(super::SIGNATURE_TAG) {
            return true;
        }
        match untag(OPENAI, sealed).map(str::to_owned) {
            Some(own) => {
                item["encrypted_content"] = json!(own);
                true
            }
            None => false,
        }
    });
}

pub fn build_request(req: &Request, model: &str) -> Value {
    let mut input = Vec::new();
    let custom_calls: HashSet<&str> = req
        .messages
        .iter()
        .flat_map(|m| &m.parts)
        .filter_map(|p| match p {
            Part::ToolCall(c) if c.custom => Some(c.id.as_str()),
            _ => None,
        })
        .collect();
    for message in &req.messages {
        let mut content = Vec::new();
        let flush = |content: &mut Vec<Value>, input: &mut Vec<Value>| {
            if !content.is_empty() {
                let role = if message.role == Role::User {
                    "user"
                } else {
                    "assistant"
                };
                input.push(
                    json!({"type": "message", "role": role, "content": std::mem::take(content)}),
                );
            }
        };
        for part in &message.parts {
            match (message.role, part) {
                (Role::User, Part::Text(t)) => {
                    content.push(json!({"type": "input_text", "text": t}))
                }
                (Role::Assistant, Part::Text(t)) => {
                    content.push(json!({"type": "output_text", "text": t}))
                }
                (_, Part::Image(i)) => {
                    content.push(json!({"type": "input_image", "image_url": url_of_image(i)}))
                }
                (_, Part::Thinking { text, signature }) => {
                    let Some(sealed) = signature.as_deref().and_then(|s| untag(OPENAI, s)) else {
                        continue;
                    };
                    flush(&mut content, &mut input);
                    let summary: Vec<Value> = if text.is_empty() {
                        Vec::new()
                    } else {
                        vec![json!({"type": "summary_text", "text": text})]
                    };
                    input.push(json!({"type": "reasoning", "summary": summary, "encrypted_content": sealed}));
                }
                (_, Part::ToolCall(call)) => {
                    flush(&mut content, &mut input);
                    input.push(if call.custom {
                        json!({"type": "custom_tool_call", "call_id": call.id, "name": call.name, "input": call.arguments})
                    } else {
                        json!({"type": "function_call", "call_id": call.id, "name": call.name, "arguments": call.arguments})
                    });
                }
                (_, Part::ToolResult(result)) => {
                    flush(&mut content, &mut input);
                    let kind = if custom_calls.contains(result.id.as_str()) {
                        "custom_tool_call_output"
                    } else {
                        "function_call_output"
                    };
                    input
                        .push(json!({"type": kind, "call_id": result.id, "output": result.text()}));
                }
                (_, Part::RedactedThinking(_)) => {}
            }
        }
        flush(&mut content, &mut input);
    }
    let mut body = Map::new();
    body.insert("model".into(), json!(model));
    if !req.system.is_empty() {
        body.insert("instructions".into(), json!(req.system.join("\n\n")));
    }
    body.insert("input".into(), Value::Array(input));
    body.insert("store".into(), json!(false));
    if let Some(max) = req.max_tokens {
        body.insert("max_output_tokens".into(), json!(max));
    }
    if let Some(t) = req.temperature {
        body.insert("temperature".into(), json!(t));
    }
    if let Some(p) = req.top_p {
        body.insert("top_p".into(), json!(p));
    }
    if let Some(reasoning) = &req.reasoning {
        let mut r = json!({"summary": "auto"});
        if let Some(effort) = reasoning.effort() {
            r["effort"] = json!(effort);
        }
        body.insert("reasoning".into(), r);
        body.insert("include".into(), json!(["reasoning.encrypted_content"]));
    }
    if !req.tools.is_empty() {
        let tools: Vec<Value> = req
            .tools
            .iter()
            .map(|tool| {
                if tool.custom {
                    let mut t = json!({"type": "custom", "name": tool.name});
                    if !tool.parameters.is_null() {
                        t["format"] = tool.parameters.clone();
                    }
                    if let Some(d) = &tool.description {
                        t["description"] = json!(d.split("\n\nThe input follows this grammar:\n").next().unwrap_or(d));
                    }
                    t
                } else {
                    let mut t = json!({"type": "function", "name": tool.name, "parameters": tool.parameters, "strict": false});
                    if let Some(d) = &tool.description {
                        t["description"] = json!(d);
                    }
                    t
                }
            })
            .collect();
        body.insert("tools".into(), Value::Array(tools));
        if let Some(choice) = &req.tool_choice {
            body.insert(
                "tool_choice".into(),
                match choice {
                    ToolChoice::Auto => json!("auto"),
                    ToolChoice::None => json!("none"),
                    ToolChoice::Required => json!("required"),
                    ToolChoice::Tool(name) => json!({"type": "function", "name": name}),
                },
            );
        }
        if let Some(parallel) = req.parallel_tool_calls {
            body.insert("parallel_tool_calls".into(), json!(parallel));
        }
    }
    if let Some(user) = &req.user {
        body.insert("prompt_cache_key".into(), json!(user));
    }
    if req.stream {
        body.insert("stream".into(), json!(true));
    }
    Value::Object(body)
}

pub fn parse_usage(usage: &Value) -> Usage {
    Usage {
        input: u64_at(usage, "input_tokens"),
        output: u64_at(usage, "output_tokens"),
        cache_read: usage
            .pointer("/input_tokens_details/cached_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cache_write: 0,
        reasoning: usage
            .pointer("/output_tokens_details/reasoning_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    }
}

fn usage_json(usage: &Usage) -> Value {
    json!({
        "input_tokens": usage.input,
        "input_tokens_details": {"cached_tokens": usage.cache_read},
        "output_tokens": usage.output,
        "output_tokens_details": {"reasoning_tokens": usage.reasoning},
        "total_tokens": usage.input + usage.output,
    })
}

fn stop_of(response: &Value) -> Stop {
    match response
        .pointer("/incomplete_details/reason")
        .and_then(Value::as_str)
    {
        Some("max_output_tokens") => Stop::MaxTokens,
        Some("content_filter") => Stop::Refusal,
        _ => Stop::End,
    }
}

pub fn parse_response(body: &Value) -> Result<Response, String> {
    if let Some(error) = body.get("error").filter(|e| !e.is_null()) {
        return Err(error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("upstream error")
            .to_owned());
    }
    let mut parts = Vec::new();
    for item in body
        .get("output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let s = |k: &str| {
            item.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        match item.get("type").and_then(Value::as_str) {
            Some("message") => {
                let text = text_of(item.get("content").unwrap_or(&Value::Null));
                if !text.is_empty() {
                    parts.push(Part::Text(text));
                }
            }
            Some("reasoning") => {
                let text = item
                    .get("summary")
                    .and_then(Value::as_array)
                    .map(|p| {
                        p.iter()
                            .filter_map(|x| x.get("text").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                            .join("\n\n")
                    })
                    .unwrap_or_default();
                let signature = item
                    .get("encrypted_content")
                    .and_then(Value::as_str)
                    .map(|s| tag(OPENAI, s));
                parts.push(Part::Thinking { text, signature });
            }
            Some("function_call") => parts.push(Part::ToolCall(ToolCall {
                id: s("call_id"),
                name: s("name"),
                arguments: s("arguments"),
                custom: false,
            })),
            Some("custom_tool_call") => parts.push(Part::ToolCall(ToolCall {
                id: s("call_id"),
                name: s("name"),
                arguments: s("input"),
                custom: true,
            })),
            _ => {}
        }
    }
    let mut stop = stop_of(body);
    if stop == Stop::End && parts.iter().any(|p| matches!(p, Part::ToolCall(_))) {
        stop = Stop::ToolUse;
    }
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
        stop,
        usage: body.get("usage").map(parse_usage).unwrap_or_default(),
    })
}

/// The output items of an answer, as Responses lists them. `custom` names
/// the free-form tools: a vendor without them called a function with an
/// `input` argument, which becomes the call's raw input again.
fn items_of(parts: &[Part], custom: &HashSet<String>) -> Vec<Value> {
    let mut items = Vec::new();
    for part in parts {
        match part {
            Part::Text(text) => items.push(json!({
                "id": new_id("msg_"), "type": "message", "status": "completed", "role": "assistant",
                "content": [{"type": "output_text", "text": text, "annotations": []}],
            })),
            Part::Thinking { text, signature } => {
                let mut item = json!({"id": new_id("rs_"), "type": "reasoning", "summary": []});
                if !text.is_empty() {
                    item["summary"] = json!([{"type": "summary_text", "text": text}]);
                }
                if let Some(sig) = signature {
                    item["encrypted_content"] = json!(for_client(OPENAI, sig));
                }
                items.push(item);
            }
            Part::ToolCall(call) => {
                if call.custom || custom.contains(&call.name) {
                    items.push(json!({
                        "id": new_id("ctc_"), "type": "custom_tool_call", "status": "completed",
                        "call_id": call.id, "name": call.name, "input": custom_input(call),
                    }));
                } else {
                    items.push(json!({
                        "id": new_id("fc_"), "type": "function_call", "status": "completed",
                        "call_id": call.id, "name": call.name, "arguments": call.arguments,
                    }));
                }
            }
            _ => {}
        }
    }
    items
}

/// A free-form call's input: as it came from a vendor that has such tools,
/// else the `input` argument of the function that stood in for it.
fn custom_input(call: &ToolCall) -> String {
    if call.custom {
        return call.arguments.clone();
    }
    match serde_json::from_str::<Value>(&call.arguments) {
        Ok(Value::Object(map)) => match map.get("input") {
            Some(Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
            None => call.arguments.clone(),
        },
        _ => call.arguments.clone(),
    }
}

fn custom_tools(request: &Request) -> HashSet<String> {
    request
        .tools
        .iter()
        .filter(|t| t.custom)
        .map(|t| t.name.clone())
        .collect()
}

fn response_json(
    id: &str,
    model: &str,
    created: i64,
    status: &str,
    output: Vec<Value>,
    usage: Option<&Usage>,
    stop: Stop,
) -> Value {
    let mut r = json!({
        "id": id, "object": "response", "created_at": created, "status": status, "model": model,
        "output": output, "parallel_tool_calls": true, "tool_choice": "auto", "tools": [],
        "error": Value::Null, "incomplete_details": Value::Null,
    });
    if let Some(usage) = usage {
        r["usage"] = usage_json(usage);
    }
    if status == "incomplete" {
        r["incomplete_details"] = json!({"reason": if stop == Stop::Refusal { "content_filter" } else { "max_output_tokens" }});
    }
    r
}

pub fn build_response(resp: &Response, request: &Request) -> Value {
    let status = if matches!(resp.stop, Stop::MaxTokens | Stop::Refusal) {
        "incomplete"
    } else {
        "completed"
    };
    let id = if resp.id.starts_with("resp_") {
        resp.id.clone()
    } else {
        new_id("resp_")
    };
    response_json(
        &id,
        &resp.model,
        chrono::Utc::now().timestamp(),
        status,
        items_of(&resp.parts, &custom_tools(request)),
        Some(&resp.usage),
        resp.stop,
    )
}

/// Reads a Responses stream.
#[derive(Default)]
pub struct Reader {
    done: bool,
    /// Whether the current call's arguments came in deltas, or only whole
    /// in its `output_item.done`.
    args_streamed: bool,
    /// Whether a reasoning item's summary has a part already.
    summary_parts: u64,
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
        let delta = || {
            data.get("delta")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let mut out = Vec::new();
        match kind {
            "response.created" => out.push(Event::Start {
                id: data
                    .pointer("/response/id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                model: data
                    .pointer("/response/model")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            }),
            "response.output_item.added" => {
                let item = &data["item"];
                let s = |k: &str| {
                    item.get(k)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned()
                };
                match item.get("type").and_then(Value::as_str) {
                    Some("function_call") => {
                        self.args_streamed = false;
                        out.push(Event::ToolStart {
                            id: s("call_id"),
                            name: s("name"),
                            custom: false,
                        });
                    }
                    Some("custom_tool_call") => {
                        self.args_streamed = false;
                        out.push(Event::ToolStart {
                            id: s("call_id"),
                            name: s("name"),
                            custom: true,
                        });
                    }
                    Some("reasoning") => self.summary_parts = 0,
                    _ => {}
                }
            }
            "response.output_text.delta" | "response.refusal.delta" => {
                out.push(Event::Text(delta()))
            }
            "response.reasoning_summary_part.added" => {
                if self.summary_parts > 0 {
                    out.push(Event::Thinking("\n\n".to_owned()));
                }
                self.summary_parts += 1;
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                out.push(Event::Thinking(delta()))
            }
            "response.function_call_arguments.delta" | "response.custom_tool_call_input.delta" => {
                self.args_streamed = true;
                out.push(Event::ToolArgs(delta()));
            }
            "response.output_item.done" => {
                let item = &data["item"];
                match item.get("type").and_then(Value::as_str) {
                    Some("reasoning") => {
                        if let Some(sig) = item
                            .get("encrypted_content")
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                        {
                            out.push(Event::ThinkingSignature(tag(OPENAI, sig)));
                        }
                    }
                    Some("function_call") if !self.args_streamed => {
                        out.push(Event::ToolArgs(
                            item.get("arguments")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                        ));
                    }
                    Some("custom_tool_call") if !self.args_streamed => {
                        out.push(Event::ToolArgs(
                            item.get("input")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                        ));
                    }
                    _ => {}
                }
            }
            "response.completed" | "response.incomplete" => {
                let response = &data["response"];
                if let Some(usage) = response.get("usage").filter(|u| u.is_object()) {
                    out.push(Event::Usage(parse_usage(usage)));
                }
                out.push(Event::Stop(stop_of(response)));
                self.done = true;
            }
            "response.failed" => {
                return Err(data
                    .pointer("/response/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("the upstream failed")
                    .to_owned());
            }
            "error" => {
                return Err(data
                    .get("message")
                    .or_else(|| data.pointer("/error/message"))
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

#[derive(Clone, Copy, PartialEq)]
enum Item {
    Message,
    Reasoning,
    Call,
    Custom,
}

/// Writes a Responses stream, the way Codex reads it.
pub struct Writer {
    id: String,
    model: String,
    created: i64,
    seq: u64,
    output: Vec<Value>,
    /// The open item: its kind, id, and what it has gathered so far.
    open: Option<(Item, String)>,
    text: String,
    signature: Option<String>,
    call: (String, String),
    custom: HashSet<String>,
    usage: Usage,
    stop: Option<Stop>,
    began: bool,
}

impl Writer {
    pub fn new(model: &str, request: &Request) -> Writer {
        Writer {
            id: new_id("resp_"),
            model: model.to_owned(),
            created: chrono::Utc::now().timestamp(),
            seq: 0,
            output: Vec::new(),
            open: None,
            text: String::new(),
            signature: None,
            call: (String::new(), String::new()),
            custom: custom_tools(request),
            usage: Usage::default(),
            stop: None,
            began: false,
        }
    }

    fn emit(&mut self, kind: &str, mut data: Value, out: &mut String) {
        data["type"] = json!(kind);
        data["sequence_number"] = json!(self.seq);
        self.seq += 1;
        out.push_str(&sse::named(kind, &data));
    }

    fn index(&self) -> usize {
        self.output.len()
    }

    fn open_item(&mut self, kind: Item, item: Value, out: &mut String) {
        self.close(out);
        let id = item["id"].as_str().unwrap_or_default().to_owned();
        let index = self.index();
        self.emit(
            "response.output_item.added",
            json!({"output_index": index, "item": item}),
            out,
        );
        match kind {
            Item::Message => self.emit(
                "response.content_part.added",
                json!({"item_id": id, "output_index": index, "content_index": 0,
                       "part": {"type": "output_text", "text": "", "annotations": []}}),
                out,
            ),
            Item::Reasoning => self.emit(
                "response.reasoning_summary_part.added",
                json!({"item_id": id, "output_index": index, "summary_index": 0,
                       "part": {"type": "summary_text", "text": ""}}),
                out,
            ),
            _ => {}
        }
        self.open = Some((kind, id));
        self.text.clear();
        self.signature = None;
    }

    fn close(&mut self, out: &mut String) {
        let Some((kind, id)) = self.open.take() else {
            return;
        };
        let index = self.index();
        let text = std::mem::take(&mut self.text);
        let item = match kind {
            Item::Message => {
                self.emit(
                    "response.output_text.done",
                    json!({"item_id": id, "output_index": index, "content_index": 0, "text": text}),
                    out,
                );
                let part = json!({"type": "output_text", "text": text, "annotations": []});
                self.emit(
                    "response.content_part.done",
                    json!({"item_id": id, "output_index": index, "content_index": 0, "part": part}),
                    out,
                );
                json!({"id": id, "type": "message", "status": "completed", "role": "assistant", "content": [part]})
            }
            Item::Reasoning => {
                let mut summary = Vec::new();
                if !text.is_empty() {
                    self.emit("response.reasoning_summary_text.done", json!({"item_id": id, "output_index": index, "summary_index": 0, "text": text}), out);
                    let part = json!({"type": "summary_text", "text": text});
                    self.emit("response.reasoning_summary_part.done", json!({"item_id": id, "output_index": index, "summary_index": 0, "part": part}), out);
                    summary.push(part);
                }
                let mut item = json!({"id": id, "type": "reasoning", "summary": summary});
                if let Some(sig) = self.signature.take() {
                    item["encrypted_content"] = json!(for_client(OPENAI, &sig));
                }
                item
            }
            Item::Call => {
                let (call_id, name) = std::mem::take(&mut self.call);
                let arguments = if text.trim().is_empty() {
                    "{}".to_owned()
                } else {
                    text
                };
                self.emit(
                    "response.function_call_arguments.done",
                    json!({"item_id": id, "output_index": index, "arguments": arguments}),
                    out,
                );
                json!({"id": id, "type": "function_call", "status": "completed", "call_id": call_id, "name": name, "arguments": arguments})
            }
            Item::Custom => {
                let (call_id, name) = std::mem::take(&mut self.call);
                // A stand-in function's arguments were held back: its input
                // goes out whole.
                let input = custom_input(&ToolCall {
                    id: call_id.clone(),
                    name: name.clone(),
                    arguments: text,
                    custom: false,
                });
                self.emit(
                    "response.custom_tool_call_input.delta",
                    json!({"item_id": id, "output_index": index, "delta": input}),
                    out,
                );
                self.emit(
                    "response.custom_tool_call_input.done",
                    json!({"item_id": id, "output_index": index, "input": input}),
                    out,
                );
                json!({"id": id, "type": "custom_tool_call", "status": "completed", "call_id": call_id, "name": name, "input": input})
            }
        };
        self.emit(
            "response.output_item.done",
            json!({"output_index": index, "item": item}),
            out,
        );
        self.output.push(item);
    }

    fn begin_once(&mut self, out: &mut String) {
        if self.began {
            return;
        }
        self.began = true;
        let r = response_json(
            &self.id,
            &self.model,
            self.created,
            "in_progress",
            Vec::new(),
            None,
            Stop::End,
        );
        self.emit("response.created", json!({"response": r}), out);
        self.emit("response.in_progress", json!({"response": r}), out);
    }
}

impl StreamWriter for Writer {
    fn begin(&mut self) -> String {
        let mut out = String::new();
        self.begin_once(&mut out);
        out
    }

    fn event(&mut self, event: Event) -> String {
        let mut out = String::new();
        self.begin_once(&mut out);
        match event {
            Event::Start { .. } | Event::RedactedThinking(_) => {}
            Event::Usage(usage) => self.usage.merge(usage),
            Event::Stop(stop) => self.stop = Some(stop),
            Event::Text(text) => {
                if !matches!(self.open, Some((Item::Message, _))) {
                    let item = json!({"id": new_id("msg_"), "type": "message", "status": "in_progress", "role": "assistant", "content": []});
                    self.open_item(Item::Message, item, &mut out);
                }
                let id = self.open.as_ref().map(|o| o.1.clone()).unwrap_or_default();
                let index = self.index();
                self.text.push_str(&text);
                self.emit("response.output_text.delta", json!({"item_id": id, "output_index": index, "content_index": 0, "delta": text}), &mut out);
            }
            Event::Thinking(text) => {
                if !matches!(self.open, Some((Item::Reasoning, _))) {
                    let item = json!({"id": new_id("rs_"), "type": "reasoning", "summary": []});
                    self.open_item(Item::Reasoning, item, &mut out);
                }
                let id = self.open.as_ref().map(|o| o.1.clone()).unwrap_or_default();
                let index = self.index();
                self.text.push_str(&text);
                self.emit("response.reasoning_summary_text.delta", json!({"item_id": id, "output_index": index, "summary_index": 0, "delta": text}), &mut out);
            }
            Event::ThinkingSignature(sig) => {
                if !matches!(self.open, Some((Item::Reasoning, _))) {
                    let item = json!({"id": new_id("rs_"), "type": "reasoning", "summary": []});
                    self.open_item(Item::Reasoning, item, &mut out);
                }
                self.signature
                    .get_or_insert_with(String::new)
                    .push_str(&sig);
            }
            Event::ToolStart { id, name, custom } => {
                let call_id = if id.is_empty() { new_id("call_") } else { id };
                let custom = custom || self.custom.contains(&name);
                let item = if custom {
                    json!({"id": new_id("ctc_"), "type": "custom_tool_call", "status": "in_progress", "call_id": call_id, "name": name, "input": ""})
                } else {
                    json!({"id": new_id("fc_"), "type": "function_call", "status": "in_progress", "call_id": call_id, "name": name, "arguments": ""})
                };
                self.open_item(
                    if custom { Item::Custom } else { Item::Call },
                    item,
                    &mut out,
                );
                self.call = (call_id, name);
            }
            Event::ToolArgs(args) => match &self.open {
                Some((Item::Call, id)) => {
                    let id = id.clone();
                    let index = self.index();
                    self.text.push_str(&args);
                    self.emit(
                        "response.function_call_arguments.delta",
                        json!({"item_id": id, "output_index": index, "delta": args}),
                        &mut out,
                    );
                }
                Some((Item::Custom, _)) => self.text.push_str(&args),
                _ => {}
            },
        }
        out
    }

    fn finish(&mut self) -> String {
        let mut out = String::new();
        self.begin_once(&mut out);
        self.close(&mut out);
        let stop = self.stop.unwrap_or_default();
        let status = if matches!(stop, Stop::MaxTokens | Stop::Refusal) {
            "incomplete"
        } else {
            "completed"
        };
        let r = response_json(
            &self.id,
            &self.model,
            self.created,
            status,
            self.output.clone(),
            Some(&self.usage),
            stop,
        );
        let kind = if status == "completed" {
            "response.completed"
        } else {
            "response.incomplete"
        };
        self.emit(kind, json!({"response": r}), &mut out);
        out
    }

    fn error(&mut self, status: u16, message: &str) -> String {
        let mut out = String::new();
        self.begin_once(&mut out);
        let mut r = response_json(
            &self.id,
            &self.model,
            self.created,
            "failed",
            self.output.clone(),
            Some(&self.usage),
            Stop::End,
        );
        let code = match status {
            429 => "rate_limit_exceeded",
            400..=499 => "invalid_request",
            _ => "server_error",
        };
        r["error"] = json!({"code": code, "message": message});
        self.emit("response.failed", json!({"response": r}), &mut out);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codex_request() -> Value {
        json!({
            "model": "deepseek/deepseek-chat",
            "instructions": "You are Codex.",
            "input": [
                {"type": "message", "role": "developer", "content": [{"type": "input_text", "text": "env"}]},
                {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "fix it"}]},
                {"type": "reasoning", "summary": [{"type": "summary_text", "text": "look"}], "encrypted_content": "enc"},
                {"type": "custom_tool_call", "call_id": "c1", "name": "apply_patch", "input": "*** Begin Patch"},
                {"type": "custom_tool_call_output", "call_id": "c1", "output": "ok"},
                {"type": "function_call", "call_id": "c2", "name": "shell", "arguments": "{\"command\":[\"ls\"]}"},
                {"type": "function_call_output", "call_id": "c2", "output": "a.txt"}
            ],
            "tools": [
                {"type": "function", "name": "shell", "parameters": {"type": "object"}},
                {"type": "custom", "name": "apply_patch", "description": "Patch", "format": {"type": "grammar", "syntax": "lark", "definition": "start: x"}},
                {"type": "web_search"}
            ],
            "reasoning": {"effort": "high", "summary": "auto"},
            "stream": true
        })
    }

    #[test]
    fn reads_codexs_items() {
        let req = parse_request(&codex_request()).unwrap();
        assert_eq!(req.system, ["You are Codex.", "env"]);
        assert_eq!(req.tools.len(), 2);
        assert!(
            req.tools[1].custom
                && req.tools[1]
                    .description
                    .as_deref()
                    .unwrap()
                    .contains("start: x")
        );
        assert_eq!(req.messages.len(), 5);
        assert!(
            matches!(&req.messages[1].parts[0], Part::Thinking { signature: Some(s), .. } if s == "pansy:openai:enc")
        );
        // Back to OpenAI the items are as they were.
        let back = build_request(&req, "gpt-5");
        let kinds: Vec<&str> = back["input"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["type"].as_str().unwrap())
            .collect();
        assert_eq!(
            kinds,
            [
                "message",
                "reasoning",
                "custom_tool_call",
                "custom_tool_call_output",
                "function_call",
                "function_call_output"
            ]
        );
        assert_eq!(back["input"][1]["encrypted_content"], "enc");
        assert_eq!(back["tools"][1]["format"]["definition"], "start: x");
    }

    #[test]
    fn a_stand_in_function_streams_as_a_custom_call() {
        let req = parse_request(&codex_request()).unwrap();
        let mut w = Writer::new("m", &req);
        let mut out = w.begin();
        for e in [
            Event::Thinking("plan".into()),
            Event::ToolStart {
                id: "c9".into(),
                name: "apply_patch".into(),
                custom: false,
            },
            Event::ToolArgs("{\"input\":\"*** Begin".into()),
            Event::ToolArgs(" Patch\"}".into()),
            Event::Stop(Stop::ToolUse),
        ] {
            out.push_str(&w.event(e));
        }
        out.push_str(&w.finish());
        let mut d = crate::sse::Decoder::default();
        let frames = d.push(out.as_bytes());
        let last: Value = serde_json::from_str(&frames.last().unwrap().data).unwrap();
        assert_eq!(last["type"], "response.completed");
        assert_eq!(last["response"]["output"][1]["type"], "custom_tool_call");
        assert_eq!(last["response"]["output"][1]["input"], "*** Begin Patch");
        let seqs: Vec<u64> = frames
            .iter()
            .map(|f| {
                serde_json::from_str::<Value>(&f.data).unwrap()["sequence_number"]
                    .as_u64()
                    .unwrap()
            })
            .collect();
        assert!(seqs.windows(2).all(|w| w[1] == w[0] + 1));
    }
}
