//! Server-sent events: a decoder for upstream streams, fed bytes as they
//! arrive, and the frame writer for the client's.

/// One event: its `event:` name (empty when unnamed) and its `data:` lines
/// joined with newlines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: String,
    pub data: String,
}

/// Splits a byte stream into events. Bytes may break anywhere, inside a
/// line or a UTF-8 character.
#[derive(Debug, Default)]
pub struct Decoder {
    buf: Vec<u8>,
    event: String,
    data: Vec<String>,
}

impl Decoder {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(end) = self.buf.iter().position(|&b| b == b'\n') {
            let mut line: Vec<u8> = self.buf.drain(..=end).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = String::from_utf8_lossy(&line).into_owned();
            self.line(&line, &mut out);
        }
        out
    }

    /// The last event, when the stream ends without a blank line after it.
    pub fn finish(&mut self) -> Vec<SseEvent> {
        let mut out = Vec::new();
        if !self.buf.is_empty() {
            let rest = std::mem::take(&mut self.buf);
            let line = String::from_utf8_lossy(&rest).into_owned();
            self.line(line.trim_end_matches('\r'), &mut out);
        }
        self.line("", &mut out);
        out
    }

    fn line(&mut self, line: &str, out: &mut Vec<SseEvent>) {
        if line.is_empty() {
            if !self.data.is_empty() || !self.event.is_empty() {
                out.push(SseEvent {
                    event: std::mem::take(&mut self.event),
                    data: std::mem::take(&mut self.data).join("\n"),
                });
            }
            return;
        }
        if line.starts_with(':') {
            return;
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "event" => self.event = value.to_owned(),
            "data" => self.data.push(value.to_owned()),
            _ => {}
        }
    }
}

/// A frame with an event name (Anthropic, Responses).
pub fn named(event: &str, data: &serde_json::Value) -> String {
    format!("event: {event}\ndata: {data}\n\n")
}

/// A frame with data only (OpenAI Chat, Gemini).
pub fn data(data: &str) -> String {
    format!("data: {data}\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_events_across_chunks() {
        let mut d = Decoder::default();
        let mut got = d.push(b"event: a\ndata: {\"x\"");
        assert!(got.is_empty());
        got.extend(d.push(b":1}\n\ndata: two\r\ndata: lines\r\n\r\n: ping\n\n"));
        got.extend(d.push("data: \u{4f60}".as_bytes()));
        got.extend(d.finish());
        assert_eq!(
            got,
            vec![
                SseEvent {
                    event: "a".into(),
                    data: "{\"x\":1}".into()
                },
                SseEvent {
                    event: "".into(),
                    data: "two\nlines".into()
                },
                SseEvent {
                    event: "".into(),
                    data: "\u{4f60}".into()
                },
            ]
        );
    }

    #[test]
    fn keeps_a_character_split_between_chunks() {
        let bytes = "data: 你好\n\n".as_bytes();
        let mut d = Decoder::default();
        let mut got = d.push(&bytes[..8]);
        got.extend(d.push(&bytes[8..]));
        assert_eq!(got[0].data, "你好");
    }
}
