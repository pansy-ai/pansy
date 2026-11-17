//! The gateway's own shape of a conversation. Every protocol parses into it
//! and is written from it, so N protocols need N readers and N writers
//! instead of N² translators. Fields no protocol shares stay in the
//! protocol's module.

use serde_json::Value;

/// One request, whatever protocol it came in.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Request {
    /// The model the client named (`provider/model`, `group/x`, a bare id).
    pub model: String,
    /// System instructions, in order.
    pub system: Vec<String>,
    pub messages: Vec<Message>,
    pub tools: Vec<Tool>,
    pub tool_choice: Option<ToolChoice>,
    pub max_tokens: Option<u64>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub stop: Vec<String>,
    pub stream: bool,
    pub reasoning: Option<Reasoning>,
    pub parallel_tool_calls: Option<bool>,
    /// The client's user/session id, passed on where the target has one.
    pub user: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub role: Role,
    pub parts: Vec<Part>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Part {
    Text(String),
    /// An image: base64 `data` with its media type, or a URL.
    Image(Image),
    /// The model's reasoning; `signature` is the vendor's seal over it
    /// (Anthropic's signature, Gemini's thought signature, an encrypted
    /// Responses reasoning item), handed back only to the vendor that made it.
    Thinking {
        text: String,
        signature: Option<String>,
    },
    /// Reasoning the vendor sent sealed (Anthropic `redacted_thinking`).
    RedactedThinking(String),
    ToolCall(ToolCall),
    ToolResult(ToolResult),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Image {
    Base64 { media_type: String, data: String },
    Url(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// The arguments as JSON text, as the model wrote them.
    pub arguments: String,
    /// A free-form (Responses `custom`) tool's raw input instead of JSON.
    pub custom: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolResult {
    pub id: String,
    /// The tool's name where the request carries it (Gemini needs it back).
    pub name: Option<String>,
    pub content: Vec<Part>,
    pub is_error: bool,
}

impl ToolResult {
    /// The result's text parts joined, for protocols that take a string.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for part in &self.content {
            if let Part::Text(text) = part {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(text);
            }
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tool {
    pub name: String,
    pub description: Option<String>,
    /// JSON Schema of the arguments.
    pub parameters: Value,
    /// A Responses `custom` tool: its input is free text (apply_patch), and
    /// targets without such tools get a function with one `input` string.
    pub custom: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToolChoice {
    Auto,
    None,
    /// Some tool must be called.
    Required,
    /// This tool must be called.
    Tool(String),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Reasoning {
    /// `minimal`, `low`, `medium`, `high`, `xhigh`, or `none`.
    pub effort: Option<String>,
    /// A token budget (Anthropic's `budget_tokens`, Gemini's `thinkingBudget`).
    pub budget: Option<u64>,
    /// Whether the client asked to see a summary of it (Responses).
    pub summary: bool,
}

impl Reasoning {
    /// The effort, from the budget when only that was given.
    pub fn effort(&self) -> Option<String> {
        if let Some(effort) = &self.effort {
            return Some(effort.clone());
        }
        self.budget.map(|b| {
            match b {
                0 => "none",
                1..=4095 => "low",
                4096..=16383 => "medium",
                _ => "high",
            }
            .to_owned()
        })
    }

    /// The budget, from the effort when only that was given.
    pub fn budget(&self) -> Option<u64> {
        if let Some(budget) = self.budget {
            return Some(budget);
        }
        self.effort.as_deref().map(|e| match e {
            "none" => 0,
            "minimal" => 1024,
            "low" => 4096,
            "medium" => 10240,
            "xhigh" => 63999,
            _ => 31999,
        })
    }

    pub fn disabled(&self) -> bool {
        self.effort.as_deref() == Some("none") || self.budget == Some(0)
    }
}

/// A whole answer (non-streaming).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Response {
    pub id: String,
    pub model: String,
    pub parts: Vec<Part>,
    pub stop: Stop,
    pub usage: Usage,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Stop {
    #[default]
    End,
    MaxTokens,
    ToolUse,
    StopSequence,
    Refusal,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    /// Prompt tokens, cache reads and writes included.
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    /// Of `output`, how many were reasoning.
    pub reasoning: u64,
}

impl Usage {
    /// Keeps what each report knows: streams report usage in pieces.
    pub fn merge(&mut self, other: Usage) {
        let max = |a: &mut u64, b: u64| *a = (*a).max(b);
        max(&mut self.input, other.input);
        max(&mut self.output, other.output);
        max(&mut self.cache_read, other.cache_read);
        max(&mut self.cache_write, other.cache_write);
        max(&mut self.reasoning, other.reasoning);
    }

    pub fn is_empty(&self) -> bool {
        *self == Usage::default()
    }
}

/// One step of a streamed answer. Readers turn the upstream's stream into
/// these; writers turn them into the client's. Parts arrive in order: text,
/// thinking and tool calls never interleave within one part.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The answer's id and model, when the upstream says them.
    Start {
        id: String,
        model: String,
    },
    Text(String),
    Thinking(String),
    /// The seal over the reasoning streamed so far.
    ThinkingSignature(String),
    RedactedThinking(String),
    /// A tool call begins; its arguments follow as `ToolArgs`.
    ToolStart {
        id: String,
        name: String,
        custom: bool,
    },
    ToolArgs(String),
    Usage(Usage),
    Stop(Stop),
}

/// Collects a stream into a whole answer: a stream from upstream answers a
/// client that did not ask for one.
#[derive(Debug, Default)]
pub struct Collector {
    pub response: Response,
}

impl Collector {
    pub fn push(&mut self, event: Event) {
        let parts = &mut self.response.parts;
        match event {
            Event::Start { id, model } => {
                if !id.is_empty() {
                    self.response.id = id;
                }
                if !model.is_empty() {
                    self.response.model = model;
                }
            }
            Event::Text(text) => match parts.last_mut() {
                Some(Part::Text(last)) => last.push_str(&text),
                _ => parts.push(Part::Text(text)),
            },
            Event::Thinking(text) => match parts.last_mut() {
                Some(Part::Thinking {
                    text: last,
                    signature: None,
                }) => last.push_str(&text),
                _ => parts.push(Part::Thinking {
                    text,
                    signature: None,
                }),
            },
            Event::ThinkingSignature(sig) => match parts.last_mut() {
                Some(Part::Thinking { signature, .. }) => {
                    signature.get_or_insert_with(String::new).push_str(&sig)
                }
                _ => parts.push(Part::Thinking {
                    text: String::new(),
                    signature: Some(sig),
                }),
            },
            Event::RedactedThinking(data) => parts.push(Part::RedactedThinking(data)),
            Event::ToolStart { id, name, custom } => parts.push(Part::ToolCall(ToolCall {
                id,
                name,
                arguments: String::new(),
                custom,
            })),
            Event::ToolArgs(args) => {
                if let Some(Part::ToolCall(call)) = parts.last_mut() {
                    call.arguments.push_str(&args);
                }
            }
            Event::Usage(usage) => self.response.usage.merge(usage),
            Event::Stop(stop) => self.response.stop = stop,
        }
    }

    pub fn finish(mut self) -> Response {
        if self.response.stop == Stop::End
            && self
                .response
                .parts
                .iter()
                .any(|p| matches!(p, Part::ToolCall(_)))
        {
            self.response.stop = Stop::ToolUse;
        }
        self.response
    }
}

/// Replays a whole answer as a stream: a whole answer from upstream answers
/// a client that asked for a stream.
pub fn events_of(response: &Response) -> Vec<Event> {
    let mut events = vec![Event::Start {
        id: response.id.clone(),
        model: response.model.clone(),
    }];
    for part in &response.parts {
        match part {
            Part::Text(text) => events.push(Event::Text(text.clone())),
            Part::Thinking { text, signature } => {
                if !text.is_empty() {
                    events.push(Event::Thinking(text.clone()));
                }
                if let Some(sig) = signature {
                    events.push(Event::ThinkingSignature(sig.clone()));
                }
            }
            Part::RedactedThinking(data) => events.push(Event::RedactedThinking(data.clone())),
            Part::ToolCall(call) => {
                events.push(Event::ToolStart {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    custom: call.custom,
                });
                events.push(Event::ToolArgs(call.arguments.clone()));
            }
            Part::Image(_) | Part::ToolResult(_) => {}
        }
    }
    events.push(Event::Usage(response.usage));
    events.push(Event::Stop(response.stop));
    events
}

/// A fresh id with `prefix` (`msg_`, `call_`, `resp_`).
pub fn new_id(prefix: &str) -> String {
    format!("{prefix}{}", uuid::Uuid::new_v4().simple())
}

/// JSON text of `value`, `{}` for null: what a tool call's arguments are when
/// a protocol carries them as an object.
pub fn args_text(value: &Value) -> String {
    match value {
        Value::Null => "{}".to_owned(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// A tool call's arguments as an object, for protocols that carry them so.
/// Text that is not a JSON object (a model cut off mid-call) becomes `{}`.
pub fn args_object(text: &str) -> Value {
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(map)) => Value::Object(map),
        _ => Value::Object(Default::default()),
    }
}
