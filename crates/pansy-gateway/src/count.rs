//! Token counts the gateway estimates itself: Anthropic's `count_tokens`
//! and Gemini's `countTokens` for upstreams that have no such endpoint.

use crate::ir::{Part, Request};

/// About four characters a token, CJK one a token: what agents use to size
/// their context, not what a bill is made of.
pub fn estimate(text: &str) -> u64 {
    let mut ascii = 0u64;
    let mut wide = 0u64;
    for c in text.chars() {
        if c.is_ascii() {
            ascii += 1;
        } else {
            wide += 1;
        }
    }
    ascii.div_ceil(4) + wide
}

pub fn request_tokens(req: &Request) -> u64 {
    let mut total = 0;
    for s in &req.system {
        total += estimate(s);
    }
    for message in &req.messages {
        total += 4;
        for part in &message.parts {
            total += part_tokens(part);
        }
    }
    for tool in &req.tools {
        total += estimate(&tool.name)
            + estimate(tool.description.as_deref().unwrap_or(""))
            + estimate(&tool.parameters.to_string());
    }
    total
}

pub(crate) fn part_tokens(part: &Part) -> u64 {
    match part {
        Part::Text(t) => estimate(t),
        Part::Thinking { text, .. } => estimate(text),
        Part::RedactedThinking(d) => (d.len() as u64) / 4,
        // Vendors bill an image at a size-dependent rate; this is a middle.
        Part::Image(_) => 1_000,
        Part::ToolCall(c) => estimate(&c.name) + estimate(&c.arguments),
        Part::ToolResult(r) => r.content.iter().map(part_tokens).sum::<u64>() + 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_ascii_by_four_and_cjk_by_one() {
        assert_eq!(estimate("abcdefgh"), 2);
        assert_eq!(estimate("你好"), 2);
        assert_eq!(estimate(""), 0);
    }
}
