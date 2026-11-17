//! Gemini (`POST /v1beta/models/{model}:generateContent` and
//! `:streamGenerateContent`): what Gemini CLI speaks.

use std::collections::HashMap;

use serde_json::{Map, Value, json};

use super::{GEMINI, StreamReader, StreamWriter, for_client, tag, u64_at, untag};
use crate::ir::*;
use crate::proto::chat::custom_parameters;
use crate::sse::{self, SseEvent};

/// The seal Gemini takes for a function call made by another model:
/// Google's documented value for history it did not write.
const NO_SIGNATURE: &str = "skip_thought_signature_validator";

/// Reads a request; the model and whether it streams are in the URL.
pub fn parse_request(body: &Value) -> Result<Request, String> {
    let config = body.get("generationConfig").unwrap_or(&Value::Null);
    let mut req = Request {
        max_tokens: config.get("maxOutputTokens").and_then(Value::as_u64),
        temperature: config.get("temperature").and_then(Value::as_f64),
        top_p: config.get("topP").and_then(Value::as_f64),
        ..Request::default()
    };
    if let Some(stops) = config.get("stopSequences").and_then(Value::as_array) {
        req.stop = stops
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
    }
    if let Some(thinking) = config.get("thinkingConfig") {
        req.reasoning = Some(Reasoning {
            effort: thinking
                .get("thinkingLevel")
                .and_then(Value::as_str)
                .map(str::to_lowercase),
            budget: thinking
                .get("thinkingBudget")
                .and_then(Value::as_i64)
                .filter(|b| *b >= 0)
                .map(|b| b as u64),
            summary: thinking
                .get("includeThoughts")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        });
    }
    if let Some(system) = body
        .get("systemInstruction")
        .or_else(|| body.get("system_instruction"))
    {
        let text = parts_text(system.get("parts"));
        if !text.is_empty() {
            req.system.push(text);
        }
    }
    // Calls without ids get one; a response without one answers the
    // earliest call of its name still unanswered.
    let mut pending: Vec<(String, String)> = Vec::new();
    let mut count = 0;
    for content in body
        .get("contents")
        .and_then(Value::as_array)
        .ok_or("contents: expected an array")?
    {
        let role = if content.get("role").and_then(Value::as_str) == Some("model") {
            Role::Assistant
        } else {
            Role::User
        };
        let mut parts = Vec::new();
        for part in content
            .get("parts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let signature = part
                .get("thoughtSignature")
                .and_then(Value::as_str)
                .map(|s| tag(GEMINI, s));
            if part.get("thought").and_then(Value::as_bool) == Some(true) {
                let text = part
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                parts.push(Part::Thinking { text, signature });
                continue;
            }
            if let Some(sig) = signature {
                parts.push(Part::Thinking {
                    text: String::new(),
                    signature: Some(sig),
                });
            }
            if let Some(text) = part.get("text").and_then(Value::as_str) {
                if !text.is_empty() {
                    parts.push(Part::Text(text.to_owned()));
                }
            } else if let Some(data) = part.get("inlineData").or_else(|| part.get("inline_data")) {
                parts.push(Part::Image(Image::Base64 {
                    media_type: data
                        .get("mimeType")
                        .and_then(Value::as_str)
                        .unwrap_or("image/png")
                        .to_owned(),
                    data: data
                        .get("data")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                }));
            } else if let Some(call) = part.get("functionCall") {
                let name = call
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let id = call
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        count += 1;
                        format!("call_{count}")
                    });
                pending.push((id.clone(), name.clone()));
                parts.push(Part::ToolCall(ToolCall {
                    id,
                    name,
                    arguments: args_text(call.get("args").unwrap_or(&Value::Null)),
                    custom: false,
                }));
            } else if let Some(response) = part.get("functionResponse") {
                let name = response
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let id = match response.get("id").and_then(Value::as_str) {
                    Some(id) => {
                        pending.retain(|(p, _)| p != id);
                        id.to_owned()
                    }
                    None => match pending.iter().position(|(_, n)| *n == name) {
                        Some(i) => pending.remove(i).0,
                        None => {
                            count += 1;
                            format!("call_{count}")
                        }
                    },
                };
                let body = response.get("response").unwrap_or(&Value::Null);
                let (text, is_error) =
                    match (body.get("output"), body.get("error"), body.get("content")) {
                        (Some(Value::String(s)), _, _) => (s.clone(), false),
                        (_, Some(error), _) => (
                            error
                                .as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| error.to_string()),
                            true,
                        ),
                        (_, _, Some(content)) => (super::text_of(content), false),
                        _ => (body.to_string(), false),
                    };
                parts.push(Part::ToolResult(ToolResult {
                    id,
                    name: Some(name),
                    content: vec![Part::Text(text)],
                    is_error,
                }));
            }
        }
        match req.messages.last_mut() {
            Some(last) if last.role == role => last.parts.extend(parts),
            _ if parts.is_empty() => {}
            _ => req.messages.push(Message { role, parts }),
        }
    }
    for tool in body
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let declarations = tool
            .get("functionDeclarations")
            .or_else(|| tool.get("function_declarations"));
        for decl in declarations.and_then(Value::as_array).into_iter().flatten() {
            req.tools.push(Tool {
                name: decl
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                description: decl
                    .get("description")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                parameters: decl
                    .get("parametersJsonSchema")
                    .or_else(|| decl.get("parameters"))
                    .map(lower_types)
                    .unwrap_or(json!({"type": "object", "properties": {}})),
                custom: false,
            });
        }
    }
    if let Some(config) = body.pointer("/toolConfig/functionCallingConfig") {
        let allowed = config.get("allowedFunctionNames").and_then(Value::as_array);
        req.tool_choice = match config.get("mode").and_then(Value::as_str) {
            Some("NONE") => Some(ToolChoice::None),
            Some("ANY") => match allowed.filter(|a| a.len() == 1).and_then(|a| a[0].as_str()) {
                Some(name) => Some(ToolChoice::Tool(name.to_owned())),
                None => Some(ToolChoice::Required),
            },
            Some("AUTO") => Some(ToolChoice::Auto),
            _ => None,
        };
    }
    Ok(req)
}

fn parts_text(parts: Option<&Value>) -> String {
    parts
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// Gemini's OpenAPI schemas spell types in capitals (`OBJECT`); JSON Schema
/// in lower case.
fn lower_types(schema: &Value) -> Value {
    match schema {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| {
                    let v = match (k.as_str(), v) {
                        ("type", Value::String(t)) => Value::String(t.to_lowercase()),
                        _ => lower_types(v),
                    };
                    (k.clone(), v)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(lower_types).collect()),
        other => other.clone(),
    }
}

/// A body passed through: thought signatures another vendor's model made
/// go, Gemini's own are untagged.
pub fn scrub_seals(body: &mut Value) {
    let Some(contents) = body.get_mut("contents").and_then(Value::as_array_mut) else {
        return;
    };
    for content in contents {
        let Some(parts) = content.get_mut("parts").and_then(Value::as_array_mut) else {
            continue;
        };
        for part in parts {
            let Some(part) = part.as_object_mut() else {
                continue;
            };
            for key in ["thoughtSignature", "thought_signature"] {
                let Some(sealed) = part.get(key).and_then(Value::as_str) else {
                    continue;
                };
                if !sealed.starts_with(super::SIGNATURE_TAG) {
                    continue;
                }
                match untag(GEMINI, sealed).map(str::to_owned) {
                    Some(own) => {
                        part.insert(key.to_owned(), json!(own));
                    }
                    None => {
                        part.remove(key);
                    }
                }
            }
        }
    }
}

pub fn build_request(req: &Request) -> Value {
    let names: HashMap<&str, &str> = req
        .messages
        .iter()
        .flat_map(|m| &m.parts)
        .filter_map(|p| match p {
            Part::ToolCall(c) => Some((c.id.as_str(), c.name.as_str())),
            _ => None,
        })
        .collect();
    let mut contents: Vec<Value> = Vec::new();
    for message in &req.messages {
        let role = if message.role == Role::User {
            "user"
        } else {
            "model"
        };
        let mut parts: Vec<Value> = Vec::new();
        let mut seal: Option<String> = None;
        for part in &message.parts {
            let mut out = match part {
                Part::Thinking { signature, .. } => {
                    if let Some(own) = signature.as_deref().and_then(|s| untag(GEMINI, s)) {
                        seal = Some(own.to_owned());
                    }
                    continue;
                }
                Part::RedactedThinking(_) => continue,
                Part::Text(text) if text.is_empty() => continue,
                Part::Text(text) => json!({"text": text}),
                Part::Image(Image::Base64 { media_type, data }) => {
                    json!({"inlineData": {"mimeType": media_type, "data": data}})
                }
                Part::Image(Image::Url(url)) => json!({"fileData": {"fileUri": url}}),
                Part::ToolCall(call) => {
                    let args = if call.custom {
                        json!({"input": call.arguments})
                    } else {
                        args_object(&call.arguments)
                    };
                    let p = json!({"functionCall": {"name": call.name, "args": args}});
                    if message.role == Role::Assistant && seal.is_none() {
                        seal = Some(NO_SIGNATURE.to_owned());
                    }
                    p
                }
                Part::ToolResult(result) => {
                    let name = result
                        .name
                        .as_deref()
                        .or_else(|| names.get(result.id.as_str()).copied())
                        .unwrap_or("tool");
                    let key = if result.is_error { "error" } else { "output" };
                    json!({"functionResponse": {"name": name, "response": {key: result.text()}}})
                }
            };
            if let Some(sig) = seal.take() {
                out["thoughtSignature"] = json!(sig);
            }
            parts.push(out);
        }
        if parts.is_empty() {
            continue;
        }
        match contents.last_mut() {
            Some(last) if last["role"] == role => {
                last["parts"].as_array_mut().unwrap().extend(parts)
            }
            _ => contents.push(json!({"role": role, "parts": parts})),
        }
    }
    let mut body = Map::new();
    body.insert("contents".into(), Value::Array(contents));
    if !req.system.is_empty() {
        body.insert(
            "systemInstruction".into(),
            json!({"parts": [{"text": req.system.join("\n\n")}]}),
        );
    }
    let mut config = Map::new();
    if let Some(max) = req.max_tokens {
        config.insert("maxOutputTokens".into(), json!(max));
    }
    if let Some(t) = req.temperature {
        config.insert("temperature".into(), json!(t));
    }
    if let Some(p) = req.top_p {
        config.insert("topP".into(), json!(p));
    }
    if !req.stop.is_empty() {
        config.insert("stopSequences".into(), json!(req.stop));
    }
    if let Some(reasoning) = req.reasoning.as_ref().filter(|r| !r.disabled()) {
        let mut thinking = json!({"includeThoughts": true});
        if let Some(budget) = reasoning.budget {
            thinking["thinkingBudget"] = json!(budget);
        } else if let Some(effort) = reasoning.effort() {
            thinking["thinkingLevel"] = json!(if effort == "low" || effort == "minimal" {
                "low"
            } else {
                "high"
            });
        }
        config.insert("thinkingConfig".into(), thinking);
    }
    if !config.is_empty() {
        body.insert("generationConfig".into(), Value::Object(config));
    }
    if !req.tools.is_empty() {
        let declarations: Vec<Value> = req
            .tools
            .iter()
            .map(|tool| {
                let schema = if tool.custom {
                    custom_parameters()
                } else {
                    tool.parameters.clone()
                };
                let mut d = json!({"name": tool.name, "parametersJsonSchema": schema});
                if let Some(desc) = &tool.description {
                    d["description"] = json!(desc);
                }
                d
            })
            .collect();
        body.insert(
            "tools".into(),
            json!([{"functionDeclarations": declarations}]),
        );
        if let Some(choice) = &req.tool_choice {
            let config = match choice {
                ToolChoice::Auto => json!({"mode": "AUTO"}),
                ToolChoice::None => json!({"mode": "NONE"}),
                ToolChoice::Required => json!({"mode": "ANY"}),
                ToolChoice::Tool(name) => json!({"mode": "ANY", "allowedFunctionNames": [name]}),
            };
            body.insert(
                "toolConfig".into(),
                json!({"functionCallingConfig": config}),
            );
        }
    }
    Value::Object(body)
}

pub fn parse_usage(usage: &Value) -> Usage {
    let reasoning = u64_at(usage, "thoughtsTokenCount");
    Usage {
        input: u64_at(usage, "promptTokenCount"),
        output: u64_at(usage, "candidatesTokenCount") + reasoning,
        cache_read: u64_at(usage, "cachedContentTokenCount"),
        cache_write: 0,
        reasoning,
    }
}

fn usage_json(usage: &Usage) -> Value {
    json!({
        "promptTokenCount": usage.input,
        "candidatesTokenCount": usage.output.saturating_sub(usage.reasoning),
        "thoughtsTokenCount": usage.reasoning,
        "cachedContentTokenCount": usage.cache_read,
        "totalTokenCount": usage.input + usage.output,
    })
}

fn parse_finish(reason: Option<&str>) -> Option<Stop> {
    Some(match reason? {
        "MAX_TOKENS" => Stop::MaxTokens,
        "SAFETY" | "RECITATION" | "PROHIBITED_CONTENT" | "BLOCKLIST" | "SPII" | "IMAGE_SAFETY" => {
            Stop::Refusal
        }
        "FINISH_REASON_UNSPECIFIED" => return None,
        _ => Stop::End,
    })
}

fn finish_str(stop: Stop) -> &'static str {
    match stop {
        Stop::MaxTokens => "MAX_TOKENS",
        Stop::Refusal => "SAFETY",
        _ => "STOP",
    }
}

/// The events of one response chunk (or of a whole response).
fn chunk_events(data: &Value, calls: &mut u64) -> Result<Vec<Event>, String> {
    if let Some(error) = data.get("error") {
        return Err(error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("upstream error")
            .to_owned());
    }
    let mut out = Vec::new();
    if let Some(reason) = data
        .pointer("/promptFeedback/blockReason")
        .and_then(Value::as_str)
    {
        return Err(format!("the prompt was blocked: {reason}"));
    }
    let candidate = data.pointer("/candidates/0");
    for part in candidate
        .and_then(|c| c.pointer("/content/parts"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let signature = part.get("thoughtSignature").and_then(Value::as_str);
        if part.get("thought").and_then(Value::as_bool) == Some(true) {
            if let Some(text) = part
                .get("text")
                .and_then(Value::as_str)
                .filter(|t| !t.is_empty())
            {
                out.push(Event::Thinking(text.to_owned()));
            }
            if let Some(sig) = signature {
                out.push(Event::ThinkingSignature(tag(GEMINI, sig)));
            }
            continue;
        }
        if let Some(sig) = signature {
            out.push(Event::ThinkingSignature(tag(GEMINI, sig)));
        }
        if let Some(text) = part.get("text").and_then(Value::as_str) {
            if !text.is_empty() {
                out.push(Event::Text(text.to_owned()));
            }
        } else if let Some(call) = part.get("functionCall") {
            *calls += 1;
            let id = call
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| new_id("call_"));
            out.push(Event::ToolStart {
                id,
                name: call
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                custom: false,
            });
            out.push(Event::ToolArgs(args_text(
                call.get("args").unwrap_or(&Value::Null),
            )));
        }
    }
    if let Some(usage) = data.get("usageMetadata") {
        out.push(Event::Usage(parse_usage(usage)));
    }
    if let Some(stop) = parse_finish(
        candidate
            .and_then(|c| c.get("finishReason"))
            .and_then(Value::as_str),
    ) {
        out.push(Event::Stop(if stop == Stop::End && *calls > 0 {
            Stop::ToolUse
        } else {
            stop
        }));
    }
    Ok(out)
}

/// A chunk of Code Assist's (`{response, traceId}`) as the Gemini chunk it
/// carries; any other as it is.
fn unwrapped(data: Value) -> Value {
    match data {
        Value::Object(mut m)
            if m.get("response").is_some_and(Value::is_object) && !m.contains_key("candidates") =>
        {
            m.remove("response").unwrap_or_default()
        }
        other => other,
    }
}

pub fn parse_response(body: &Value) -> Result<Response, String> {
    let unwrapped = unwrapped(body.clone());
    let body = &unwrapped;
    let mut collector = Collector::default();
    collector.push(Event::Start {
        id: body
            .get("responseId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        model: body
            .get("modelVersion")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    });
    for event in chunk_events(body, &mut 0)? {
        collector.push(event);
    }
    Ok(collector.finish())
}

fn part_values(parts: &[Part]) -> Vec<Value> {
    let mut out = Vec::new();
    let mut seal: Option<String> = None;
    for part in parts {
        let mut value = match part {
            Part::Text(text) => json!({"text": text}),
            Part::Thinking { text, signature } => {
                let sig = signature.as_deref().map(|s| for_client(GEMINI, s));
                if text.is_empty() {
                    seal = sig;
                    continue;
                }
                let mut v = json!({"text": text, "thought": true});
                if let Some(sig) = sig {
                    v["thoughtSignature"] = json!(sig);
                }
                v
            }
            Part::ToolCall(call) => {
                let args = if call.custom {
                    json!({"input": call.arguments})
                } else {
                    args_object(&call.arguments)
                };
                json!({"functionCall": {"id": call.id, "name": call.name, "args": args}})
            }
            _ => continue,
        };
        if let Some(sig) = seal.take() {
            value["thoughtSignature"] = json!(sig);
        }
        out.push(value);
    }
    if let Some(sig) = seal {
        out.push(json!({"text": "", "thoughtSignature": sig}));
    }
    out
}

pub fn build_response(resp: &Response) -> Value {
    json!({
        "candidates": [{
            "content": {"role": "model", "parts": part_values(&resp.parts)},
            "finishReason": finish_str(resp.stop),
            "index": 0,
        }],
        "usageMetadata": usage_json(&resp.usage),
        "modelVersion": resp.model,
        "responseId": if resp.id.is_empty() { new_id("") } else { resp.id.clone() },
    })
}

pub fn error_status(status: u16) -> &'static str {
    match status {
        400 => "INVALID_ARGUMENT",
        401 => "UNAUTHENTICATED",
        403 => "PERMISSION_DENIED",
        404 => "NOT_FOUND",
        429 => "RESOURCE_EXHAUSTED",
        503 => "UNAVAILABLE",
        _ => "INTERNAL",
    }
}

/// Reads Gemini's stream (`alt=sse`).
#[derive(Default)]
pub struct Reader {
    done: bool,
    started: bool,
    calls: u64,
}

impl StreamReader for Reader {
    fn frame(&mut self, frame: &SseEvent) -> Result<Vec<Event>, String> {
        let Ok(data) = serde_json::from_str::<Value>(&frame.data) else {
            return Ok(Vec::new());
        };
        let data = unwrapped(data);
        let mut out = Vec::new();
        if !self.started {
            self.started = true;
            out.push(Event::Start {
                id: data
                    .get("responseId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                model: data
                    .get("modelVersion")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            });
        }
        let events = chunk_events(&data, &mut self.calls)?;
        if events.iter().any(|e| matches!(e, Event::Stop(_))) {
            self.done = true;
        }
        out.extend(events);
        Ok(out)
    }

    fn done(&self) -> bool {
        self.done
    }
}

/// Writes Gemini's stream: SSE, or one JSON array without `alt=sse`.
pub struct Writer {
    model: String,
    id: String,
    json_array: bool,
    chunks: u64,
    /// A tool call gathering its arguments: Gemini sends calls whole.
    call: Option<(String, String, String)>,
    seal: Option<String>,
    usage: Usage,
    stop: Option<Stop>,
    tool_used: bool,
}

impl Writer {
    pub fn new(model: &str, json_array: bool) -> Writer {
        Writer {
            model: model.to_owned(),
            id: new_id(""),
            json_array,
            chunks: 0,
            call: None,
            seal: None,
            usage: Usage::default(),
            stop: None,
            tool_used: false,
        }
    }

    fn chunk(&mut self, parts: Vec<Value>, finish: Option<Stop>) -> String {
        let mut candidate = json!({"content": {"role": "model", "parts": parts}, "index": 0});
        let mut chunk = json!({"modelVersion": self.model, "responseId": self.id});
        if let Some(stop) = finish {
            candidate["finishReason"] = json!(finish_str(stop));
            chunk["usageMetadata"] = usage_json(&self.usage);
        }
        chunk["candidates"] = json!([candidate]);
        self.chunks += 1;
        if self.json_array {
            let sep = if self.chunks == 1 { "[" } else { ",\r\n" };
            format!("{sep}{chunk}")
        } else {
            sse::data(&chunk.to_string())
        }
    }

    fn part(&mut self, mut part: Value) -> String {
        if let Some(sig) = self.seal.take() {
            part["thoughtSignature"] = json!(sig);
        }
        self.chunk(vec![part], None)
    }

    fn flush_call(&mut self) -> String {
        match self.call.take() {
            Some((id, name, args)) => {
                let args = args_object(&args);
                self.part(json!({"functionCall": {"id": id, "name": name, "args": args}}))
            }
            None => String::new(),
        }
    }
}

impl StreamWriter for Writer {
    fn event(&mut self, event: Event) -> String {
        if let Event::ToolArgs(args) = &event {
            if let Some(call) = self.call.as_mut() {
                call.2.push_str(args);
            }
            return String::new();
        }
        let mut out = self.flush_call();
        match event {
            Event::Start { .. } | Event::RedactedThinking(_) | Event::ToolArgs(_) => {}
            Event::Usage(usage) => self.usage.merge(usage),
            Event::Stop(stop) => self.stop = Some(stop),
            Event::Text(text) => out.push_str(&self.part(json!({"text": text}))),
            Event::Thinking(text) => {
                out.push_str(&self.part(json!({"text": text, "thought": true})))
            }
            Event::ThinkingSignature(sig) => {
                self.seal
                    .get_or_insert_with(String::new)
                    .push_str(&for_client(GEMINI, &sig));
            }
            Event::ToolStart { id, name, .. } => {
                self.tool_used = true;
                let id = if id.is_empty() { new_id("call_") } else { id };
                self.call = Some((id, name, String::new()));
            }
        }
        out
    }

    fn finish(&mut self) -> String {
        let mut out = self.flush_call();
        let stop = match self.stop {
            Some(Stop::ToolUse) => Stop::End,
            Some(stop) => stop,
            None => Stop::End,
        };
        let parts = match self.seal.take() {
            Some(sig) => vec![json!({"text": "", "thoughtSignature": sig})],
            None => Vec::new(),
        };
        out.push_str(&self.chunk(parts, Some(stop)));
        if self.json_array {
            out.push(']');
        }
        out
    }

    fn error(&mut self, status: u16, message: &str) -> String {
        let body = super::error_body(super::Protocol::Gemini, status, message);
        if self.json_array {
            let sep = if self.chunks == 0 { "[" } else { ",\r\n" };
            format!("{sep}{body}]")
        } else {
            sse::data(&body.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_responses_with_calls_that_have_no_ids() {
        let body = json!({
            "contents": [
                {"role": "user", "parts": [{"text": "read it"}]},
                {"role": "model", "parts": [{"functionCall": {"name": "read_file", "args": {"path": "a"}}, "thoughtSignature": "s1"}]},
                {"role": "user", "parts": [{"functionResponse": {"name": "read_file", "response": {"output": "hello"}}}]}
            ],
            "tools": [{"functionDeclarations": [{"name": "read_file", "parameters": {"type": "OBJECT", "properties": {"path": {"type": "STRING"}}}}]}],
            "generationConfig": {"thinkingConfig": {"thinkingBudget": -1, "includeThoughts": true}}
        });
        let req = parse_request(&body).unwrap();
        let Part::ToolCall(call) = &req.messages[1].parts[1] else {
            panic!()
        };
        let Part::ToolResult(result) = &req.messages[2].parts[0] else {
            panic!()
        };
        assert_eq!(call.id, result.id);
        assert_eq!(
            req.tools[0].parameters["properties"]["path"]["type"],
            "string"
        );
        // Back to Gemini, the seal goes on the call again.
        let back = build_request(&req);
        assert_eq!(back["contents"][1]["parts"][0]["thoughtSignature"], "s1");
        assert_eq!(
            back["contents"][2]["parts"][0]["functionResponse"]["name"],
            "read_file"
        );
    }

    #[test]
    fn a_call_from_another_model_gets_the_placeholder_seal() {
        let req = Request {
            messages: vec![Message {
                role: Role::Assistant,
                parts: vec![Part::ToolCall(ToolCall {
                    id: "x".into(),
                    name: "f".into(),
                    arguments: "{}".into(),
                    custom: false,
                })],
            }],
            ..Default::default()
        };
        assert_eq!(
            build_request(&req)["contents"][0]["parts"][0]["thoughtSignature"],
            NO_SIGNATURE
        );
    }

    #[test]
    fn writes_a_whole_call_and_a_json_array() {
        let mut w = Writer::new("m", true);
        let mut out = String::new();
        for e in [
            Event::Text("Hi".into()),
            Event::ToolStart {
                id: "c".into(),
                name: "f".into(),
                custom: false,
            },
            Event::ToolArgs("{\"a\":".into()),
            Event::ToolArgs("1}".into()),
        ] {
            out.push_str(&w.event(e));
        }
        out.push_str(&w.finish());
        let chunks: Vec<Value> = serde_json::from_str(&out).unwrap();
        assert_eq!(chunks.len(), 3);
        assert_eq!(
            chunks[1]["candidates"][0]["content"]["parts"][0]["functionCall"]["args"]["a"],
            1
        );
        assert_eq!(chunks[2]["candidates"][0]["finishReason"], "STOP");
    }
}
