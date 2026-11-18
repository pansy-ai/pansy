//! What a request's prompt holds, as the Routing page's context window
//! draws it: the agent's system prompt, its tools' definitions, the skills
//! it lists, its memory (instruction files and reminders), the files it
//! read, its other tools' results and the conversation, each by its size in
//! tokens (estimated, as `count` does) and the items in it by name: a tool's,
//! a file's path, a turn's number. Never the text itself.

use serde::Serialize;
use serde_json::Value;

use crate::count::{estimate, part_tokens};
use crate::ir::{Part, Request, Role};

/// The parts, in the order the window draws them.
pub const KINDS: [&str; 7] = [
    "system", "tools", "skills", "memory", "files", "results", "chat",
];
const SYSTEM: usize = 0;
const TOOLS: usize = 1;
const SKILLS: usize = 2;
const MEMORY: usize = 3;
const FILES: usize = 4;
const RESULTS: usize = 5;
const CHAT: usize = 6;

/// The most items a part lists; the smallest of the rest are one ("12 more").
const ITEMS: usize = 40;
/// The latest turns, listed one by one; the earlier ones are one item.
const TURNS: usize = 12;

/// A request's prompt, part by part.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Prompt {
    /// All the parts' tokens, estimated.
    pub tokens: u64,
    /// The parts with anything in them, in `KINDS`' order.
    pub parts: Vec<PromptPart>,
    /// The turns of the conversation: each prompt of the user's.
    #[serde(skip_serializing_if = "is_zero")]
    pub turns: u32,
    /// The conversation it is of, by a hash of its first prompt: its
    /// requests together draw how the window filled.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub conv: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PromptPart {
    pub kind: &'static str,
    pub tokens: u64,
    pub items: Vec<Item>,
}

/// What a part holds, by name: a tool, a file, a turn (`tag` `turn`, its
/// number), `n` earlier turns (`turns`) or the `n` smallest left (`more`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Item {
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub tag: String,
    pub tokens: u64,
    /// How many it stands for: a tool's results, reminders, images.
    #[serde(skip_serializing_if = "one")]
    pub n: u32,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

fn one(n: &u32) -> bool {
    *n <= 1
}

/// The tools that read a file whole, by their name lowercased: what they
/// return is the file's.
const READERS: [&str; 7] = [
    "read",
    "read_file",
    "readfile",
    "read_text_file",
    "view",
    "view_file",
    "open_file",
];
/// The arguments a reader names its file in.
const PATHS: [&str; 5] = [
    "file_path",
    "path",
    "filePath",
    "absolute_path",
    "target_file",
];
/// The tools that run a command: their results are tagged `shell`.
const SHELLS: [&str; 7] = [
    "bash",
    "shell",
    "exec_command",
    "local_shell",
    "run_shell_command",
    "run_terminal_cmd",
    "terminal",
];

/// What a call asked for: its tool's name, and the file it reads.
struct Call {
    name: String,
    file: Option<String>,
}

#[derive(Default)]
struct Tally {
    parts: [Vec<Item>; 7],
    /// The turn the conversation is on; 0 before the first prompt.
    turn: u32,
    calls: Vec<(String, Call)>,
    first: Option<String>,
}

impl Tally {
    /// Adds tokens to an item; `count`, as one more of it.
    fn add(&mut self, part: usize, name: &str, tag: &str, tokens: u64, count: bool) {
        if tokens == 0 {
            return;
        }
        let items = &mut self.parts[part];
        match items.iter_mut().find(|i| i.name == name && i.tag == tag) {
            Some(item) => {
                item.tokens += tokens;
                if count {
                    item.n += 1;
                }
            }
            None => items.push(Item {
                name: name.to_owned(),
                tag: tag.to_owned(),
                tokens,
                n: 1,
            }),
        }
    }

    /// Tokens of the turn the conversation is on.
    fn chat(&mut self, tokens: u64) {
        let turn = self.turn.max(1).to_string();
        self.add(CHAT, &turn, "turn", tokens, false);
    }

    /// A text, its reminders apart: where it is said (`system`, or the
    /// conversation), unless it is what an agent puts in a known place.
    fn text(&mut self, text: &str, system: bool) -> bool {
        let mut rest = String::new();
        let mut at = text;
        while let Some(open) = at.find("<system-reminder>") {
            rest.push_str(&at[..open]);
            let inner = &at[open + "<system-reminder>".len()..];
            let (reminder, after) = match inner.find("</system-reminder>") {
                Some(close) => (
                    &inner[..close],
                    &inner[close + "</system-reminder>".len()..],
                ),
                None => (inner, ""),
            };
            self.reminder(reminder);
            at = after;
        }
        rest.push_str(at);
        self.plain(&rest, system)
    }

    /// A reminder: instruction files, the skills listed, or a reminder.
    fn reminder(&mut self, text: &str) {
        if text.contains("Contents of ") {
            // "Contents of /work/CLAUDE.md (project instructions, …):" each.
            let starts: Vec<usize> = text.match_indices("Contents of ").map(|(i, _)| i).collect();
            for (n, &start) in starts.iter().enumerate() {
                let from = if n == 0 { 0 } else { start };
                let to = starts.get(n + 1).copied().unwrap_or(text.len());
                let section = &text[from..to];
                let head = &text[start + "Contents of ".len()..to];
                let line = head.lines().next().unwrap_or_default();
                let path = line
                    .split(" (")
                    .next()
                    .unwrap_or_default()
                    .trim_end_matches(':')
                    .trim();
                let tag = if line.contains("private global") {
                    "user"
                } else if line.contains("private project") || line.contains("not checked in") {
                    "local"
                } else if line.contains("project instructions") {
                    "project"
                } else {
                    "memory"
                };
                self.add(MEMORY, path, tag, estimate(section), true);
            }
        } else if skills(text) {
            self.add(SKILLS, "skills", "skills", estimate(text), true);
        } else {
            self.add(MEMORY, "reminders", "reminder", estimate(text), true);
        }
    }

    /// Text outside reminders; whether it is a prompt of the user's.
    fn plain(&mut self, text: &str, system: bool) -> bool {
        let t = text.trim_start();
        if t.trim_end().is_empty() {
            return false;
        }
        let tokens = estimate(text);
        if t.starts_with("x-anthropic-billing-header") {
            self.add(SYSTEM, "billing", "", tokens, false);
        } else if let Some(dir) = t.strip_prefix("# AGENTS.md instructions for ") {
            let dir = dir
                .lines()
                .next()
                .unwrap_or_default()
                .trim()
                .trim_end_matches('/');
            self.add(MEMORY, &format!("{dir}/AGENTS.md"), "project", tokens, true);
        } else if t.starts_with("<user_instructions>") {
            self.add(MEMORY, "AGENTS.md", "project", tokens, true);
        } else if t.starts_with("<environment_context>") {
            self.add(MEMORY, "environment", "", tokens, false);
        } else if t.starts_with("<skills_instructions>") || skills(t) {
            self.add(SKILLS, "skills", "skills", tokens, true);
        } else if t.starts_with("This session is being continued from a previous conversation") {
            self.add(CHAT, "compacted", "compacted", tokens, false);
        } else if system {
            // The agent's own instructions; what a harness adds in tags
            // (Codex's permissions, collaboration mode) apart.
            let name = if t.starts_with('<') {
                "harness"
            } else {
                "prompt"
            };
            self.add(SYSTEM, name, "", tokens, false);
        } else {
            self.chat(tokens);
            return true;
        }
        false
    }

    fn message(&mut self, role: Role, parts: &[Part]) {
        // A prompt of the user's begins a turn: its first words begin it.
        let mut asked = false;
        for part in parts {
            match part {
                Part::Text(text) if role == Role::User => {
                    // Counted on the next turn, unless it is no prompt
                    // (reminders, instructions): then the turn stays.
                    let turn = self.turn;
                    if !asked {
                        self.turn += 1;
                    }
                    if self.text(text, false) {
                        if !asked && self.first.is_none() {
                            self.first = Some(strip_reminders(text));
                        }
                        asked = true;
                    } else if !asked {
                        self.turn = turn;
                    }
                }
                Part::Text(text) => self.chat(estimate(text)),
                Part::Thinking { text, .. } => self.chat(estimate(text)),
                Part::RedactedThinking(data) => self.chat(data.len() as u64 / 4),
                Part::Image(_) => self.add(FILES, "image", "image", 1_000, true),
                Part::ToolCall(call) => {
                    self.chat(estimate(&call.name) + estimate(&call.arguments));
                    let file = READERS
                        .contains(&call.name.to_lowercase().as_str())
                        .then(|| file_of(&call.arguments))
                        .flatten();
                    self.calls.push((
                        call.id.clone(),
                        Call {
                            name: call.name.clone(),
                            file,
                        },
                    ));
                }
                Part::ToolResult(result) => {
                    let call = self.calls.iter().rev().find(|(id, _)| *id == result.id);
                    let name = call
                        .map(|(_, c)| c.name.clone())
                        .or_else(|| result.name.clone())
                        .unwrap_or_else(|| "tool".to_owned());
                    let file = call.and_then(|(_, c)| c.file.clone());
                    let mut tokens = 4;
                    for part in &result.content {
                        match part {
                            Part::Image(_) => self.add(FILES, "image", "image", 1_000, true),
                            other => tokens += part_tokens(other),
                        }
                    }
                    match file {
                        Some(path) => self.add(FILES, &path, "read", tokens, true),
                        None => self.add(RESULTS, &name, tag_of(&name), tokens, true),
                    }
                }
            }
        }
        self.chat(4);
    }

    fn finish(mut self) -> Prompt {
        let turns = self.turn;
        // The latest turns one by one, the earlier together.
        let chat = std::mem::take(&mut self.parts[CHAT]);
        let (numbered, other): (Vec<Item>, Vec<Item>) =
            chat.into_iter().partition(|i| i.tag == "turn");
        let mut kept = other;
        if numbered.len() > TURNS {
            let earlier = &numbered[..numbered.len() - TURNS];
            kept.push(Item {
                name: String::new(),
                tag: "turns".into(),
                tokens: earlier.iter().map(|i| i.tokens).sum(),
                n: earlier.len() as u32,
            });
            kept.extend_from_slice(&numbered[numbered.len() - TURNS..]);
        } else {
            kept.extend(numbered);
        }
        self.parts[CHAT] = kept;
        let mut parts = Vec::new();
        for (n, mut items) in self.parts.into_iter().enumerate() {
            let tokens: u64 = items.iter().map(|i| i.tokens).sum();
            if tokens == 0 {
                continue;
            }
            if n != CHAT && items.len() > ITEMS {
                items.sort_by_key(|i| std::cmp::Reverse(i.tokens));
                let rest = items.split_off(ITEMS - 1);
                items.push(Item {
                    name: String::new(),
                    tag: "more".into(),
                    tokens: rest.iter().map(|i| i.tokens).sum(),
                    n: rest.len() as u32,
                });
            }
            parts.push(PromptPart {
                kind: KINDS[n],
                tokens,
                items,
            });
        }
        Prompt {
            tokens: parts.iter().map(|p| p.tokens).sum(),
            parts,
            turns,
            conv: self.first.map(|f| hash(&f)).unwrap_or_default(),
        }
    }
}

/// A list of the skills an agent can load.
fn skills(text: &str) -> bool {
    text.contains("skills are available")
        || text.contains("<available_skills>")
        || text.contains("## Skills\n")
}

fn strip_reminders(text: &str) -> String {
    let mut out = String::new();
    let mut at = text;
    while let Some(open) = at.find("<system-reminder>") {
        out.push_str(&at[..open]);
        at = match at[open..].find("</system-reminder>") {
            Some(close) => &at[open + close + "</system-reminder>".len()..],
            None => "",
        };
    }
    out.push_str(at);
    out.trim().to_owned()
}

/// The file a reader's arguments name.
fn file_of(arguments: &str) -> Option<String> {
    let args: Value = serde_json::from_str(arguments).ok()?;
    PATHS
        .iter()
        .find_map(|k| args.get(k).and_then(Value::as_str))
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
}

fn tag_of(tool: &str) -> &'static str {
    if tool.starts_with("mcp__") {
        "mcp"
    } else if SHELLS.contains(&tool.to_lowercase().as_str()) {
        "shell"
    } else {
        ""
    }
}

/// FNV-1a, 48 bits in hex: which conversation, not what it said.
fn hash(text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{:012x}", h >> 16)
}

/// What `request`'s prompt holds.
pub fn of(request: &Request) -> Prompt {
    let mut tally = Tally::default();
    for text in &request.system {
        tally.text(text, true);
    }
    for tool in &request.tools {
        let tokens = estimate(&tool.name)
            + estimate(tool.description.as_deref().unwrap_or(""))
            + estimate(&tool.parameters.to_string());
        let tag = if tool.name.starts_with("mcp__") {
            "mcp"
        } else {
            ""
        };
        tally.add(TOOLS, &tool.name, tag, tokens, false);
    }
    for message in &request.messages {
        tally.message(message.role, &message.parts);
    }
    tally.finish()
}

/// The context window of `model` on `provider`, in tokens: as the
/// provider's model list says, else as its family's is known; 0 when
/// neither says.
pub fn window(config: &pansy_core::config::Config, provider: &str, model: &str) -> u64 {
    if let Some(context) = config
        .provider(provider)
        .and_then(|p| p.models.iter().find(|m| m.id == model))
        .and_then(|m| m.context)
    {
        return context;
    }
    let m = model.rsplit('/').next().unwrap_or(model).to_lowercase();
    if m.contains("claude") {
        if m.contains("[1m]") || m.ends_with("-1m") {
            1_000_000
        } else {
            200_000
        }
    } else if m.starts_with("gpt-5") {
        400_000
    } else if m.starts_with("gpt-4.1") {
        1_047_576
    } else if m.starts_with("gpt-4o") {
        128_000
    } else if m.starts_with("gemini-2") || m.starts_with("gemini-3") {
        1_048_576
    } else if m.starts_with("deepseek") {
        128_000
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Message, Tool, ToolCall, ToolResult};
    use serde_json::json;

    fn user(parts: Vec<Part>) -> Message {
        Message {
            role: Role::User,
            parts,
        }
    }

    fn assistant(parts: Vec<Part>) -> Message {
        Message {
            role: Role::Assistant,
            parts,
        }
    }

    fn part<'a>(p: &'a Prompt, kind: &str) -> &'a PromptPart {
        p.parts.iter().find(|x| x.kind == kind).unwrap()
    }

    fn item<'a>(p: &'a Prompt, kind: &str, name: &str) -> &'a Item {
        part(p, kind).items.iter().find(|i| i.name == name).unwrap()
    }

    #[test]
    fn sorts_a_prompt_into_its_parts() {
        let request = Request {
            system: vec![
                "x-anthropic-billing-header: cc_version=2".into(),
                "You are an agent. ".repeat(40),
            ],
            tools: vec![
                Tool {
                    name: "Read".into(),
                    description: Some("Reads a file".into()),
                    parameters: json!({"type": "object"}),
                    custom: false,
                },
                Tool {
                    name: "mcp__docs__search".into(),
                    description: None,
                    parameters: json!({}),
                    custom: false,
                },
            ],
            messages: vec![
                user(vec![
                    Part::Text("<system-reminder>The following skills are available for use: pdf</system-reminder>".into()),
                    Part::Text("<system-reminder>\nContents of /work/CLAUDE.md (project instructions, checked into the codebase):\n\nUse tabs.\n</system-reminder>".into()),
                    Part::Text("Fix the login bug".into()),
                ]),
                assistant(vec![
                    Part::Text("Looking.".into()),
                    Part::ToolCall(ToolCall {
                        id: "t1".into(),
                        name: "Read".into(),
                        arguments: r#"{"file_path":"/work/src/login.ts"}"#.into(),
                        custom: false,
                    }),
                    Part::ToolCall(ToolCall {
                        id: "t2".into(),
                        name: "Bash".into(),
                        arguments: r#"{"command":"ls"}"#.into(),
                        custom: false,
                    }),
                ]),
                user(vec![
                    Part::ToolResult(ToolResult {
                        id: "t1".into(),
                        name: None,
                        content: vec![Part::Text("export function login() {}".repeat(10))],
                        is_error: false,
                    }),
                    Part::ToolResult(ToolResult {
                        id: "t2".into(),
                        name: None,
                        content: vec![Part::Text("src\nREADME.md".into())],
                        is_error: false,
                    }),
                ]),
                user(vec![Part::Text("Now add a test".into())]),
            ],
            ..Request::default()
        };
        let p = of(&request);
        let kinds: Vec<&str> = p.parts.iter().map(|x| x.kind).collect();
        assert_eq!(
            kinds,
            [
                "system", "tools", "skills", "memory", "files", "results", "chat"
            ]
        );
        assert_eq!(p.tokens, p.parts.iter().map(|x| x.tokens).sum::<u64>());
        assert!(item(&p, "system", "billing").tokens > 0);
        assert!(item(&p, "system", "prompt").tokens > 100);
        assert_eq!(item(&p, "tools", "mcp__docs__search").tag, "mcp");
        assert_eq!(item(&p, "memory", "/work/CLAUDE.md").tag, "project");
        let file = item(&p, "files", "/work/src/login.ts");
        assert_eq!((file.tag.as_str(), file.n), ("read", 1));
        assert_eq!(item(&p, "results", "Bash").tag, "shell");
        // Two prompts: the tool results between them are no turn of their own.
        assert_eq!(p.turns, 2);
        assert!(item(&p, "chat", "1").tokens > 0);
        assert!(item(&p, "chat", "2").tokens > 0);
        // No text is kept: only names and sizes.
        let said = serde_json::to_string(&p).unwrap();
        assert!(
            !said.contains("Fix the login")
                && !said.contains("Use tabs")
                && !said.contains("export function")
        );
    }

    #[test]
    fn knows_a_conversation_by_its_first_prompt() {
        let ask = |words: &str, more: bool| {
            let mut messages = vec![user(vec![
                Part::Text("<system-reminder>today</system-reminder>".into()),
                Part::Text(words.into()),
            ])];
            if more {
                messages.push(assistant(vec![Part::Text("Done.".into())]));
                messages.push(user(vec![Part::Text("Thanks".into())]));
            }
            of(&Request {
                messages,
                ..Request::default()
            })
        };
        let first = ask("Fix the login bug", false);
        assert_eq!(first.conv.len(), 12);
        assert_eq!(first.conv, ask("Fix the login bug", true).conv);
        assert_ne!(first.conv, ask("Write a README", false).conv);
    }

    #[test]
    fn gathers_the_earlier_turns_and_the_smallest_items() {
        let mut messages = Vec::new();
        for n in 0..20 {
            messages.push(user(vec![Part::Text(format!("question {n}"))]));
            messages.push(assistant(vec![Part::Text("answer".into())]));
        }
        let tools = (0..50)
            .map(|n| Tool {
                name: format!("tool{n}"),
                description: Some("x".repeat(n * 4)),
                parameters: json!({}),
                custom: false,
            })
            .collect();
        let p = of(&Request {
            messages,
            tools,
            ..Request::default()
        });
        assert_eq!(p.turns, 20);
        let chat = part(&p, "chat");
        assert_eq!(chat.items.len(), TURNS + 1);
        let earlier = chat.items.iter().find(|i| i.tag == "turns").unwrap();
        assert_eq!(earlier.n, 8);
        let tools = part(&p, "tools");
        assert_eq!(tools.items.len(), ITEMS);
        assert_eq!(tools.items.last().unwrap().tag, "more");
        assert_eq!(
            tools.items.iter().map(|i| i.tokens).sum::<u64>(),
            tools.tokens
        );
    }

    #[test]
    fn reads_codex_instructions() {
        let p = of(&Request {
            system: vec!["<permissions instructions>sandboxed</permissions instructions>".into()],
            messages: vec![user(vec![
                Part::Text("# AGENTS.md instructions for /work/app\n\n<INSTRUCTIONS>Be brief</INSTRUCTIONS>".into()),
                Part::Text("<environment_context><cwd>/work/app</cwd></environment_context>".into()),
                Part::Text("Add a health check".into()),
            ])],
            ..Request::default()
        });
        assert!(item(&p, "system", "harness").tokens > 0);
        assert_eq!(item(&p, "memory", "/work/app/AGENTS.md").tag, "project");
        assert!(item(&p, "memory", "environment").tokens > 0);
        assert_eq!(p.turns, 1);
    }

    #[test]
    fn knows_a_models_window() {
        let config = pansy_core::config::Config::default();
        assert_eq!(window(&config, "anthropic", "claude-sonnet-4-5"), 200_000);
        assert_eq!(window(&config, "openai", "gpt-5.1-codex"), 400_000);
        assert_eq!(window(&config, "x", "some-model"), 0);
    }
}
