//! OpenAI Chat Completions (`POST /v1/chat/completions`): what most
//! OpenAI-compatible vendors and agents speak.

use serde_json::{Map, Value, json};

use super::{Quirks, StreamReader, StreamWriter, split_data_url, text_of, u64_at};
use crate::ir::*;
use crate::sse::{self, SseEvent};

pub fn parse_request(body: &Value) -> Result<Request, String> {
    let mut req = Request {
        model: body
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        max_tokens: body
            .get("max_completion_tokens")
            .or_else(|| body.get("max_tokens"))
            .and_then(Value::as_u64),
        temperature: body.get("temperature").and_then(Value::as_f64),
        top_p: body.get("top_p").and_then(Value::as_f64),
        stream: body.get("stream").and_then(Value::as_bool).unwrap_or(false),
        parallel_tool_calls: body.get("parallel_tool_calls").and_then(Value::as_bool),
        user: body.get("user").and_then(Value::as_str).map(str::to_owned),
        ..Request::default()
    };
    match body.get("stop") {
        Some(Value::String(s)) => req.stop.push(s.clone()),
        Some(Value::Array(items)) => {
            req.stop = items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        }
        _ => {}
    }
    if let Some(effort) = body.get("reasoning_effort").and_then(Value::as_str) {
        req.reasoning = Some(Reasoning {
            effort: Some(effort.to_owned()),
            ..Reasoning::default()
        });
    } else if let Some(effort) = body.pointer("/reasoning/effort").and_then(Value::as_str) {
        req.reasoning = Some(Reasoning {
            effort: Some(effort.to_owned()),
            ..Reasoning::default()
        });
    }
    let messages = body
        .get("messages")
        .and_then(Value::as_array)
        .ok_or("messages: expected an array")?;
    for message in messages {
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("user");
        let content = message.get("content").unwrap_or(&Value::Null);
        match role {
            "system" | "developer" => {
                let text = text_of(content);
                if !text.is_empty() {
                    req.system.push(text);
                }
            }
            "tool" | "function" => {
                let result = Part::ToolResult(ToolResult {
                    id: message
                        .get("tool_call_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    name: message
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    content: content_parts(content),
                    is_error: false,
                });
                push(&mut req.messages, Role::User, vec![result]);
            }
            "assistant" => {
                let mut parts = Vec::new();
                let reasoning = message
                    .get("reasoning_content")
                    .or_else(|| message.get("reasoning"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !reasoning.is_empty() {
                    parts.push(Part::Thinking {
                        text: reasoning.to_owned(),
                        signature: None,
                    });
                }
                parts.extend(content_parts(content));
                if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
                    for call in calls {
                        parts.push(Part::ToolCall(ToolCall {
                            id: call
                                .get("id")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                            name: call
                                .pointer("/function/name")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                            arguments: args_text(
                                call.pointer("/function/arguments").unwrap_or(&Value::Null),
                            ),
                            custom: false,
                        }));
                    }
                }
                push(&mut req.messages, Role::Assistant, parts);
            }
            _ => push(&mut req.messages, Role::User, content_parts(content)),
        }
    }
    if let Some(tools) = body.get("tools").and_then(Value::as_array) {
        for tool in tools {
            let Some(function) = tool.get("function") else {
                continue;
            };
            req.tools.push(Tool {
                name: function
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                description: function
                    .get("description")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                parameters: function
                    .get("parameters")
                    .cloned()
                    .unwrap_or(json!({"type": "object"})),
                custom: false,
            });
        }
    }
    req.tool_choice = body.get("tool_choice").and_then(parse_tool_choice);
    Ok(req)
}

pub(crate) fn parse_tool_choice(choice: &Value) -> Option<ToolChoice> {
    match choice {
        Value::String(s) => match s.as_str() {
            "none" => Some(ToolChoice::None),
            "required" | "any" => Some(ToolChoice::Required),
            _ => Some(ToolChoice::Auto),
        },
        Value::Object(_) => choice
            .pointer("/function/name")
            .or_else(|| choice.get("name"))
            .and_then(Value::as_str)
            .map(|name| ToolChoice::Tool(name.to_owned())),
        _ => None,
    }
}

/// Appends parts to the last message when it has the same role: Anthropic
/// and Gemini take one turn per role.
fn push(messages: &mut Vec<Message>, role: Role, parts: Vec<Part>) {
    if parts.is_empty() {
        return;
    }
    match messages.last_mut() {
        Some(last) if last.role == role => last.parts.extend(parts),
        _ => messages.push(Message { role, parts }),
    }
}

fn content_parts(content: &Value) -> Vec<Part> {
    match content {
        Value::String(s) if s.is_empty() => Vec::new(),
        Value::String(s) => vec![Part::Text(s.clone())],
        Value::Array(items) => items
            .iter()
            .filter_map(|item| match item.get("type").and_then(Value::as_str) {
                Some("text") | Some("input_text") | Some("output_text") => Some(Part::Text(
                    item.get("text").and_then(Value::as_str)?.to_owned(),
                )),
                Some("image_url") => {
                    let url = item
                        .pointer("/image_url/url")
                        .or_else(|| item.get("image_url"))
                        .and_then(Value::as_str)?;
                    Some(Part::Image(image_of_url(url)))
                }
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

pub(crate) fn image_of_url(url: &str) -> Image {
    match split_data_url(url) {
        Some((media_type, data)) => Image::Base64 { media_type, data },
        None => Image::Url(url.to_owned()),
    }
}

pub(crate) fn url_of_image(image: &Image) -> String {
    match image {
        Image::Base64 { media_type, data } => format!("data:{media_type};base64,{data}"),
        Image::Url(url) => url.clone(),
    }
}

/// Parameters of a function standing in for a free-form tool.
pub(crate) fn custom_parameters() -> Value {
    json!({
        "type": "object",
        "properties": {"input": {"type": "string", "description": "The tool's raw input."}},
        "required": ["input"],
    })
}

pub fn build_request(req: &Request, model: &str, quirks: &Quirks) -> Value {
    let mut messages: Vec<Value> = Vec::new();
    if !req.system.is_empty() {
        messages.push(json!({"role": "system", "content": req.system.join("\n\n")}));
    }
    for message in &req.messages {
        match message.role {
            Role::User => {
                // Tool results answer the assistant turn before: they come first.
                for part in &message.parts {
                    if let Part::ToolResult(result) = part {
                        messages.push(json!({"role": "tool", "tool_call_id": result.id, "content": result.text()}));
                    }
                }
                let rest: Vec<&Part> = message
                    .parts
                    .iter()
                    .filter(|p| !matches!(p, Part::ToolResult(_)))
                    .collect();
                // Images a tool returned go along as the user's.
                let images: Vec<&Image> = message
                    .parts
                    .iter()
                    .filter_map(|p| match p {
                        Part::ToolResult(r) => Some(r.content.iter().filter_map(|c| match c {
                            Part::Image(i) => Some(i),
                            _ => None,
                        })),
                        _ => None,
                    })
                    .flatten()
                    .collect();
                if rest.is_empty() && images.is_empty() {
                    continue;
                }
                let only_text =
                    images.is_empty() && rest.iter().all(|p| matches!(p, Part::Text(_)));
                let content = if only_text {
                    json!(
                        rest.iter()
                            .filter_map(|p| match p {
                                Part::Text(t) => Some(t.as_str()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    )
                } else {
                    let mut items: Vec<Value> = rest
                        .iter()
                        .filter_map(|p| match p {
                            Part::Text(t) => Some(json!({"type": "text", "text": t})),
                            Part::Image(i) => Some(
                                json!({"type": "image_url", "image_url": {"url": url_of_image(i)}}),
                            ),
                            _ => None,
                        })
                        .collect();
                    items.extend(images.iter().map(
                        |i| json!({"type": "image_url", "image_url": {"url": url_of_image(i)}}),
                    ));
                    Value::Array(items)
                };
                messages.push(json!({"role": "user", "content": content}));
            }
            Role::Assistant => {
                let mut text = String::new();
                let mut reasoning = String::new();
                let mut calls = Vec::new();
                for part in &message.parts {
                    match part {
                        Part::Text(t) => text.push_str(t),
                        Part::Thinking { text: t, .. } => reasoning.push_str(t),
                        Part::ToolCall(call) => {
                            let arguments = if call.custom {
                                json!({"input": call.arguments}).to_string()
                            } else if call.arguments.trim().is_empty() {
                                "{}".to_owned()
                            } else {
                                call.arguments.clone()
                            };
                            calls.push(json!({
                                "id": call.id, "type": "function",
                                "function": {"name": call.name, "arguments": arguments},
                            }));
                        }
                        _ => {}
                    }
                }
                let mut m = json!({"role": "assistant", "content": if text.is_empty() && !calls.is_empty() { Value::Null } else { json!(text) }});
                if !calls.is_empty() {
                    m["tool_calls"] = Value::Array(calls);
                }
                if quirks.reasoning_content && !reasoning.is_empty() {
                    m["reasoning_content"] = json!(reasoning);
                }
                messages.push(m);
            }
        }
    }
    let mut body = Map::new();
    body.insert("model".into(), json!(model));
    body.insert("messages".into(), Value::Array(messages));
    if let Some(max) = req.max_tokens {
        let key = if quirks.max_completion_tokens {
            "max_completion_tokens"
        } else {
            "max_tokens"
        };
        body.insert(key.into(), json!(max));
    }
    if let Some(t) = req.temperature {
        body.insert("temperature".into(), json!(t));
    }
    if let Some(p) = req.top_p {
        body.insert("top_p".into(), json!(p));
    }
    if !req.stop.is_empty() {
        body.insert("stop".into(), json!(req.stop));
    }
    if let Some(effort) = req.reasoning.as_ref().and_then(Reasoning::effort)
        && quirks.max_completion_tokens
    {
        // OpenAI's own reasoning models.
        body.insert(
            "reasoning_effort".into(),
            json!(if effort == "none" {
                "minimal".to_owned()
            } else {
                effort
            }),
        );
    }
    if !req.tools.is_empty() {
        let tools: Vec<Value> = req
            .tools
            .iter()
            .map(|tool| {
                let parameters = if tool.custom {
                    custom_parameters()
                } else {
                    tool.parameters.clone()
                };
                let mut function = json!({"name": tool.name, "parameters": parameters});
                if let Some(d) = &tool.description {
                    function["description"] = json!(d);
                }
                json!({"type": "function", "function": function})
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
                    ToolChoice::Tool(name) => {
                        json!({"type": "function", "function": {"name": name}})
                    }
                },
            );
        }
        if let Some(parallel) = req.parallel_tool_calls {
            body.insert("parallel_tool_calls".into(), json!(parallel));
        }
    }
    if let Some(user) = &req.user {
        body.insert("user".into(), json!(user));
    }
    if req.stream {
        body.insert("stream".into(), json!(true));
        body.insert("stream_options".into(), json!({"include_usage": true}));
    }
    Value::Object(body)
}

pub fn parse_usage(usage: &Value) -> Usage {
    let cache_read = usage
        .pointer("/prompt_tokens_details/cached_tokens")
        .and_then(Value::as_u64)
        .or_else(|| usage.get("prompt_cache_hit_tokens").and_then(Value::as_u64))
        .or_else(|| usage.get("cached_tokens").and_then(Value::as_u64))
        .unwrap_or(0);
    Usage {
        input: u64_at(usage, "prompt_tokens"),
        output: u64_at(usage, "completion_tokens"),
        cache_read,
        cache_write: 0,
        reasoning: usage
            .pointer("/completion_tokens_details/reasoning_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    }
}

fn usage_json(usage: &Usage) -> Value {
    json!({
        "prompt_tokens": usage.input,
        "completion_tokens": usage.output,
        "total_tokens": usage.input + usage.output,
        "prompt_tokens_details": {"cached_tokens": usage.cache_read},
        "completion_tokens_details": {"reasoning_tokens": usage.reasoning},
    })
}

fn parse_finish(reason: Option<&str>) -> Option<Stop> {
    Some(match reason? {
        "length" => Stop::MaxTokens,
        "tool_calls" | "function_call" => Stop::ToolUse,
        "content_filter" => Stop::Refusal,
        _ => Stop::End,
    })
}

fn finish_str(stop: Stop) -> &'static str {
    match stop {
        Stop::End | Stop::StopSequence => "stop",
        Stop::MaxTokens => "length",
        Stop::ToolUse => "tool_calls",
        Stop::Refusal => "content_filter",
    }
}

pub fn parse_response(body: &Value) -> Result<Response, String> {
    if let Some(error) = body.get("error") {
        return Err(error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("upstream error")
            .to_owned());
    }
    let choice = body
        .pointer("/choices/0")
        .ok_or("no choices in the answer")?;
    let message = &choice["message"];
    let mut parts = Vec::new();
    let reasoning = message
        .get("reasoning_content")
        .or_else(|| message.get("reasoning"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut content = text_of(message.get("content").unwrap_or(&Value::Null));
    let mut thinking = reasoning.to_owned();
    if thinking.is_empty()
        && let Some((think, rest)) = split_think_tags(&content)
    {
        thinking = think;
        content = rest;
    }
    if !thinking.is_empty() {
        parts.push(Part::Thinking {
            text: thinking,
            signature: None,
        });
    }
    if !content.is_empty() {
        parts.push(Part::Text(content));
    }
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            parts.push(Part::ToolCall(ToolCall {
                id: call
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| new_id("call_")),
                name: call
                    .pointer("/function/name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                arguments: args_text(call.pointer("/function/arguments").unwrap_or(&Value::Null)),
                custom: false,
            }));
        }
    }
    let mut stop =
        parse_finish(choice.get("finish_reason").and_then(Value::as_str)).unwrap_or_default();
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

/// `<think>…</think>` at the start of a reply (MiniMax, Qwen, local models):
/// the reasoning and the rest.
fn split_think_tags(content: &str) -> Option<(String, String)> {
    let rest = content.trim_start().strip_prefix("<think>")?;
    let (think, after) = rest.split_once("</think>")?;
    Some((think.trim().to_owned(), after.trim_start().to_owned()))
}

pub fn build_response(resp: &Response) -> Value {
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut calls = Vec::new();
    for part in &resp.parts {
        match part {
            Part::Text(t) => text.push_str(t),
            Part::Thinking { text: t, .. } => reasoning.push_str(t),
            Part::ToolCall(call) => calls.push(json!({
                "id": call.id, "type": "function",
                "function": {"name": call.name, "arguments": if call.custom { json!({"input": call.arguments}).to_string() } else { call.arguments.clone() }},
            })),
            _ => {}
        }
    }
    let mut message = json!({"role": "assistant", "content": if text.is_empty() && !calls.is_empty() { Value::Null } else { json!(text) }});
    if !reasoning.is_empty() {
        message["reasoning_content"] = json!(reasoning);
    }
    if !calls.is_empty() {
        message["tool_calls"] = Value::Array(calls);
    }
    json!({
        "id": if resp.id.is_empty() { new_id("chatcmpl-") } else { resp.id.clone() },
        "object": "chat.completion",
        "created": now(),
        "model": resp.model,
        "choices": [{"index": 0, "message": message, "finish_reason": finish_str(resp.stop)}],
        "usage": usage_json(&resp.usage),
    })
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Where a reply's `<think>` tags stand while it streams.
#[derive(Default, PartialEq)]
enum Think {
    /// Nothing seen yet: a reply may open with `<think>`.
    #[default]
    Undecided,
    Inside,
    Outside,
}

/// Reads an OpenAI-compatible stream.
#[derive(Default)]
pub struct Reader {
    done: bool,
    started: bool,
    /// The index and id of the tool call being streamed.
    tool: Option<(u64, String)>,
    think: Think,
    /// Text held back while it might still be the start of `<think>`.
    pending: String,
    stopped: bool,
}

impl Reader {
    fn content(&mut self, text: &str, out: &mut Vec<Event>) {
        if self.think == Think::Outside {
            out.push(Event::Text(text.to_owned()));
            return;
        }
        self.pending.push_str(text);
        loop {
            match self.think {
                Think::Undecided => {
                    let trimmed = self.pending.trim_start();
                    if let Some(rest) = trimmed.strip_prefix("<think>") {
                        self.pending = rest.to_owned();
                        self.think = Think::Inside;
                    } else if "<think>".starts_with(trimmed) {
                        return; // could still be the tag
                    } else {
                        self.think = Think::Outside;
                        out.push(Event::Text(std::mem::take(&mut self.pending)));
                        return;
                    }
                }
                Think::Inside => {
                    if let Some((think, rest)) = self.pending.split_once("</think>") {
                        if !think.is_empty() {
                            out.push(Event::Thinking(think.to_owned()));
                        }
                        let rest = rest.trim_start_matches('\n').to_owned();
                        self.pending.clear();
                        self.think = Think::Outside;
                        if !rest.is_empty() {
                            out.push(Event::Text(rest));
                        }
                        return;
                    }
                    // Keep what could be the start of `</think>`.
                    let keep = (1.."</think>".len())
                        .rev()
                        .find(|&n| self.pending.ends_with(&"</think>"[..n]))
                        .unwrap_or(0);
                    let emit = self.pending.len() - keep;
                    if emit > 0 && self.pending.is_char_boundary(emit) {
                        out.push(Event::Thinking(self.pending[..emit].to_owned()));
                        self.pending.drain(..emit);
                    }
                    return;
                }
                Think::Outside => return,
            }
        }
    }
}

impl StreamReader for Reader {
    fn frame(&mut self, frame: &SseEvent) -> Result<Vec<Event>, String> {
        let mut out = Vec::new();
        if frame.data.trim() == "[DONE]" {
            self.done = true;
            if !self.pending.is_empty() {
                let rest = std::mem::take(&mut self.pending);
                out.push(if self.think == Think::Inside {
                    Event::Thinking(rest)
                } else {
                    Event::Text(rest)
                });
            }
            return Ok(out);
        }
        let Ok(data) = serde_json::from_str::<Value>(&frame.data) else {
            return Ok(out);
        };
        if let Some(error) = data.get("error") {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| error.to_string());
            return Err(message);
        }
        if !self.started {
            self.started = true;
            out.push(Event::Start {
                id: data
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                model: data
                    .get("model")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            });
        }
        if let Some(choice) = data.pointer("/choices/0") {
            let delta = choice
                .get("delta")
                .or_else(|| choice.get("message"))
                .unwrap_or(&Value::Null);
            for key in ["reasoning_content", "reasoning"] {
                if let Some(text) = delta
                    .get(key)
                    .and_then(Value::as_str)
                    .filter(|t| !t.is_empty())
                {
                    out.push(Event::Thinking(text.to_owned()));
                    break;
                }
            }
            if let Some(text) = delta
                .get("content")
                .and_then(Value::as_str)
                .filter(|t| !t.is_empty())
            {
                self.content(text, &mut out);
            }
            if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
                for call in calls {
                    let index = call.get("index").and_then(Value::as_u64).unwrap_or(0);
                    let id = call
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty());
                    let new = match (&self.tool, id) {
                        (None, _) => true,
                        (Some((i, current)), Some(id)) => *i != index || current != id,
                        (Some((i, _)), None) => *i != index,
                    };
                    if new {
                        let id = id.map(str::to_owned).unwrap_or_else(|| new_id("call_"));
                        out.push(Event::ToolStart {
                            id: id.clone(),
                            name: call
                                .pointer("/function/name")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                            custom: false,
                        });
                        self.tool = Some((index, id));
                    }
                    match call.pointer("/function/arguments") {
                        Some(Value::String(args)) if !args.is_empty() => {
                            out.push(Event::ToolArgs(args.clone()))
                        }
                        Some(args @ Value::Object(_)) => {
                            out.push(Event::ToolArgs(args.to_string()))
                        }
                        _ => {}
                    }
                }
            }
            if let Some(usage) = choice.get("usage").filter(|u| u.is_object()) {
                out.push(Event::Usage(parse_usage(usage)));
            }
            if let Some(stop) = parse_finish(choice.get("finish_reason").and_then(Value::as_str))
                && !self.stopped
            {
                self.stopped = true;
                out.push(Event::Stop(stop));
            }
        }
        if let Some(usage) = data.get("usage").filter(|u| u.is_object()) {
            out.push(Event::Usage(parse_usage(usage)));
        }
        Ok(out)
    }

    fn done(&self) -> bool {
        self.done
    }
}

/// Writes an OpenAI-compatible stream.
pub struct Writer {
    id: String,
    model: String,
    created: i64,
    started: bool,
    tools: i64,
    usage: Usage,
    stop: Option<Stop>,
}

impl Writer {
    pub fn new(model: &str, _request: &Request) -> Writer {
        Writer {
            id: new_id("chatcmpl-"),
            model: model.to_owned(),
            created: now(),
            started: false,
            tools: -1,
            usage: Usage::default(),
            stop: None,
        }
    }

    fn chunk(&self, delta: Value, finish: Option<&str>) -> String {
        let mut chunk = json!({
            "id": self.id, "object": "chat.completion.chunk", "created": self.created, "model": self.model,
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
        });
        if finish.is_some() {
            chunk["usage"] = usage_json(&self.usage);
        }
        sse::data(&chunk.to_string())
    }

    fn start(&mut self, out: &mut String) {
        if !self.started {
            self.started = true;
            out.push_str(&self.chunk(json!({"role": "assistant", "content": ""}), None));
        }
    }
}

impl StreamWriter for Writer {
    fn event(&mut self, event: Event) -> String {
        let mut out = String::new();
        match event {
            Event::Usage(usage) => {
                self.usage.merge(usage);
                return out;
            }
            Event::Stop(stop) => {
                self.stop = Some(stop);
                return out;
            }
            Event::Start { .. } | Event::ThinkingSignature(_) | Event::RedactedThinking(_) => {}
            Event::Text(text) => {
                self.start(&mut out);
                out.push_str(&self.chunk(json!({"content": text}), None));
            }
            Event::Thinking(text) => {
                self.start(&mut out);
                out.push_str(&self.chunk(json!({"reasoning_content": text}), None));
            }
            Event::ToolStart { id, name, .. } => {
                self.start(&mut out);
                self.tools += 1;
                let id = if id.is_empty() { new_id("call_") } else { id };
                out.push_str(&self.chunk(
                    json!({"tool_calls": [{"index": self.tools, "id": id, "type": "function", "function": {"name": name, "arguments": ""}}]}),
                    None,
                ));
            }
            Event::ToolArgs(args) => {
                if self.tools >= 0 && !args.is_empty() {
                    out.push_str(&self.chunk(json!({"tool_calls": [{"index": self.tools, "function": {"arguments": args}}]}), None));
                }
            }
        }
        out
    }

    fn finish(&mut self) -> String {
        let mut out = String::new();
        self.start(&mut out);
        let stop = match self.stop {
            Some(Stop::End) | None if self.tools >= 0 => Stop::ToolUse,
            Some(stop) => stop,
            None => Stop::End,
        };
        out.push_str(&self.chunk(json!({}), Some(finish_str(stop))));
        out.push_str(&sse::data("[DONE]"));
        out
    }

    fn error(&mut self, status: u16, message: &str) -> String {
        sse::data(&super::error_body(super::Protocol::Chat, status, message).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_results_come_before_the_users_text() {
        let req = Request {
            messages: vec![
                Message {
                    role: Role::Assistant,
                    parts: vec![Part::ToolCall(ToolCall {
                        id: "c1".into(),
                        name: "ls".into(),
                        arguments: "{}".into(),
                        custom: false,
                    })],
                },
                Message {
                    role: Role::User,
                    parts: vec![
                        Part::ToolResult(ToolResult {
                            id: "c1".into(),
                            name: None,
                            content: vec![Part::Text("a".into())],
                            is_error: false,
                        }),
                        Part::Text("go on".into()),
                    ],
                },
            ],
            ..Default::default()
        };
        let body = build_request(&req, "m", &Quirks::default());
        let roles: Vec<&str> = body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["role"].as_str().unwrap())
            .collect();
        assert_eq!(roles, ["assistant", "tool", "user"]);
        assert!(body["messages"][0]["content"].is_null());
    }

    #[test]
    fn reads_reasoning_tool_calls_and_usage_from_a_stream() {
        let mut r = Reader::default();
        let mut events = Vec::new();
        for data in [
            r#"{"id":"x","model":"deepseek-chat","choices":[{"delta":{"role":"assistant","reasoning_content":"hmm"}}]}"#,
            r#"{"choices":[{"delta":{"content":"ok"}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"ls","arguments":""}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{}"}}]}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":9,"completion_tokens":3}}"#,
            "[DONE]",
        ] {
            events.extend(
                r.frame(&SseEvent {
                    event: String::new(),
                    data: data.into(),
                })
                .unwrap(),
            );
        }
        assert!(r.done());
        assert_eq!(
            events[1..],
            [
                Event::Thinking("hmm".into()),
                Event::Text("ok".into()),
                Event::ToolStart {
                    id: "c1".into(),
                    name: "ls".into(),
                    custom: false
                },
                Event::ToolArgs("{}".into()),
                Event::Stop(Stop::ToolUse),
                Event::Usage(Usage {
                    input: 9,
                    output: 3,
                    ..Default::default()
                }),
            ]
        );
    }

    #[test]
    fn splits_think_tags_across_chunks() {
        let mut r = Reader::default();
        let mut events = Vec::new();
        for piece in ["<thi", "nk>plan", " it</th", "ink>\nAnswer"] {
            let data = json!({"choices": [{"delta": {"content": piece}}]}).to_string();
            events.extend(
                r.frame(&SseEvent {
                    event: String::new(),
                    data,
                })
                .unwrap(),
            );
        }
        let thinking: String = events
            .iter()
            .filter_map(|e| match e {
                Event::Thinking(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        let text: String = events
            .iter()
            .filter_map(|e| match e {
                Event::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(thinking, "plan it");
        assert_eq!(text, "Answer");
    }
}
