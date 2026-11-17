//! Putting values back in a streamed answer, event by event as it comes.
//!
//! A placeholder a stream splits between two events ({{API_ in one text
//! delta, KEY_k3v9x2mq}} in the next) is put back whole: an event whose
//! text ends in what may be the start of one is held until the next, and
//! if that one goes on with the same text the start moves over to it.
//! Nothing is added or dropped, so the text the agent puts together is the
//! same. A comment between events (`: keepalive`) and a wait event (ping)
//! go on as they come, ahead of an event held.
//!
//! The stream is server-sent events, or, as Gemini streams without
//! `alt=sse`, one JSON array whose elements come one by one; a body that
//! starts as a JSON object is held whole and put back at the end.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use crate::Masker;
use crate::json::{KEPT, SIGNATURE_KEYS, Walked, as_json, type_of, walk};

/// Puts values back in a stream; made by [`Masker::stream`]. It owns its
/// masker's `Arc`, so it is `'static` and `Send`, for an async stream.
pub struct StreamRestorer {
    m: Arc<Masker>,
    mode: Mode,
    buf: Vec<u8>,
    pending: Option<Held>,
    put: BTreeMap<String, usize>,
    // a JSON array's scan: where it is, how deep, and in a string
    pos: usize,
    depth: usize,
    in_str: bool,
    esc: bool,
    start: Option<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Unknown,
    Sse,
    Array,
    Whole,
}

/// An event one or more of whose texts ended in what may be the start of a
/// placeholder: each one's tail by its path.
struct Held {
    ctx: String,
    tails: HashMap<String, String>,
    /// the event as it is, and without the tails
    full: Vec<u8>,
    cut: Vec<u8>,
}

/// The events a stream sends while the model is slow, which say nothing of
/// the answer: Anthropic's ping, Responses' keepalive, in_progress and
/// queued.
const WAIT_EVENTS: &[&str] = &[
    "ping",
    "keepalive",
    "response.in_progress",
    "response.queued",
];

impl StreamRestorer {
    pub fn new(m: Arc<Masker>) -> StreamRestorer {
        StreamRestorer {
            m,
            mode: Mode::Unknown,
            buf: Vec::new(),
            pending: None,
            put: BTreeMap::new(),
            pos: 0,
            depth: 0,
            in_str: false,
            esc: false,
            start: None,
        }
    }

    /// Takes the next bytes of the stream, and gives back the bytes ready
    /// to send on.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<u8> {
        self.buf.extend_from_slice(chunk);
        if self.mode == Mode::Unknown {
            match self.buf.iter().find(|c| !c.is_ascii_whitespace()) {
                None => return Vec::new(),
                Some(b'[') => self.mode = Mode::Array,
                Some(b'{') => self.mode = Mode::Whole,
                Some(_) => self.mode = Mode::Sse,
            }
        }
        match self.mode {
            Mode::Sse => self.push_sse(),
            Mode::Array => self.push_array(),
            _ => Vec::new(),
        }
    }

    /// Gives back what is still held, at the stream's end.
    pub fn finish(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        if self.mode == Mode::Whole {
            let body = std::mem::take(&mut self.buf);
            return self.m.restore_body(&body, &mut self.put);
        }
        if let Some(p) = self.pending.take() {
            out.extend_from_slice(&p.full);
        }
        if !self.buf.is_empty() {
            let rest = std::mem::take(&mut self.buf);
            out.extend_from_slice(&self.restore_text(&rest));
        }
        self.pos = 0;
        self.start = None;
        out
    }

    /// How many places of each placeholder were put back so far.
    pub fn put_back(&self) -> BTreeMap<String, usize> {
        self.put.clone()
    }

    fn restore_text(&mut self, b: &[u8]) -> Vec<u8> {
        match std::str::from_utf8(b) {
            Ok(s) => self
                .m
                .restore_into(s, false, Some(&mut self.put))
                .into_bytes(),
            Err(_) => b.to_vec(),
        }
    }

    fn push_sse(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            // a comment between two events goes on as it comes: held for
            // the event after it, an agent behind a proxy heard nothing
            // while its vendor reasoned silently, and the proxy hung up
            let c = comment_lines(&self.buf);
            if c > 0 {
                out.extend(self.buf.drain(..c));
                continue;
            }
            let Some((i, n)) = event_end(&self.buf) else {
                break;
            };
            let ev: Vec<u8> = self.buf.drain(..i + n).collect();
            let done = self.event(&ev);
            out.extend_from_slice(&done);
        }
        out
    }

    /// Finds the array's elements as they complete; each goes as an event
    /// does, with what is before it (the [ or a comma) as its head.
    fn push_array(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut done = 0; // bytes of buf gone out
        while self.pos < self.buf.len() {
            let c = self.buf[self.pos];
            if self.in_str {
                if self.esc {
                    self.esc = false;
                } else if c == b'\\' {
                    self.esc = true;
                } else if c == b'"' {
                    self.in_str = false;
                }
                self.pos += 1;
                continue;
            }
            match c {
                b'"' => self.in_str = true,
                b'{' | b'[' => {
                    if self.depth == 1 && self.start.is_none() {
                        self.start = Some(self.pos);
                    }
                    self.depth += 1;
                }
                b'}' | b']' => {
                    self.depth = self.depth.saturating_sub(1);
                    if self.depth == 1
                        && let Some(st) = self.start.take()
                    {
                        let pre = self.buf[done..st].to_vec();
                        let data = self.buf[st..=self.pos].to_vec();
                        let ev = self.data_event("", &pre, &data, b"");
                        out.extend_from_slice(&ev);
                        done = self.pos + 1;
                    } else if self.depth == 0 {
                        if let Some(p) = self.pending.take() {
                            out.extend_from_slice(&p.full);
                        }
                        out.extend_from_slice(&self.buf[done..=self.pos]);
                        done = self.pos + 1;
                    }
                }
                _ => {}
            }
            self.pos += 1;
        }
        self.buf.drain(..done);
        self.pos -= done;
        if let Some(st) = self.start.as_mut() {
            *st -= done;
        }
        out
    }

    /// Puts the values back in one event and gives it back, or holds it.
    fn event(&mut self, ev: &[u8]) -> Vec<u8> {
        if self.pending.is_none() && !self.m.holding() {
            return ev.to_vec(); // nothing masked to put back
        }
        let Some((name, pre, data, post)) = split_event(ev) else {
            let mut out = self.pending.take().map(|p| p.full).unwrap_or_default();
            out.extend_from_slice(&self.restore_text(ev));
            return out;
        };
        let (name, pre, data, post) = (name.to_owned(), pre.to_vec(), data.to_vec(), post.to_vec());
        self.data_event(&name, &pre, &data, &post)
    }

    /// Every text of an event is put back but what a vendor sealed. Only a
    /// text is held for its tail, never an id or Responses' random
    /// obfuscation, and each text's tail is held for itself.
    fn data_event(&mut self, name: &str, pre: &[u8], data: &[u8], post: &[u8]) -> Vec<u8> {
        if self.pending.is_none() && !self.m.holding() {
            return rebuild(pre, data, post);
        }
        if self.pending.is_some() && waiting(name, data) {
            // a keepalive between two pieces of a text goes on ahead of the
            // piece held
            let restored = self.m.restore_body(data, &mut self.put);
            return rebuild(pre, &restored, post);
        }
        let mut out = Vec::new();
        let ctx = format!("{name}|{}", event_ctx(data));
        // the starts of placeholders the last event ended in go on here
        let mut carry: HashMap<String, String> = HashMap::new();
        if let Some(p) = self.pending.take() {
            if p.ctx == ctx && has_paths(data, &p.tails) {
                out.extend_from_slice(&p.cut);
                carry = p.tails;
            } else {
                out.extend_from_slice(&p.full);
            }
        }
        let mut typ = type_of(data);
        if typ.is_empty() {
            typ = name.to_owned();
        }
        let m = Arc::clone(&self.m);
        let put = &mut self.put;
        let mut tails: HashMap<String, String> = HashMap::new();
        let mut cuts: HashMap<String, String> = HashMap::new();
        let walked = walk(data, &mut |path, key, s| {
            let joined;
            let s = match carry.get(path) {
                Some(c) => {
                    joined = format!("{c}{s}");
                    joined.as_str()
                }
                None => s,
            };
            if sealed_text(key, s) {
                return Some(s.to_owned());
            }
            // the tail is looked for before anything is put back, so a
            // placeholder whose last brace is still to come isn't put back
            // without it and the brace left behind
            if !KEPT.contains(&key) && key != "obfuscation" {
                let n = m.partial_tail(s);
                if n > 0 {
                    let cut = s.len() - n;
                    tails.insert(path.to_owned(), s[cut..].to_owned());
                    cuts.insert(
                        path.to_owned(),
                        m.restore_into(&s[..cut], as_json(key, &typ), None),
                    );
                }
            }
            // counted here only: whichever of full and cut goes out, the
            // places are these
            Some(m.restore_into(s, as_json(key, &typ), Some(&mut *put)))
        });
        let restored = match walked {
            Walked::Changed(b) => b,
            Walked::Same => data.to_vec(),
            Walked::NotJson => {
                out.extend_from_slice(&self.restore_text(&rebuild(pre, data, post)));
                return out;
            }
        };
        let full = rebuild(pre, &restored, post);
        if tails.is_empty() {
            out.extend_from_slice(&full);
            return out;
        }
        let cut_data = match walk(&restored, &mut |path, _, _| cuts.get(path).cloned()) {
            Walked::Changed(b) => b,
            _ => restored.clone(),
        };
        self.pending = Some(Held {
            ctx,
            tails,
            full,
            cut: rebuild(pre, &cut_data, post),
        });
        out
    }
}

/// How long the whole comment lines (`: keepalive`) and blank lines at the
/// start of `b` are: `b` starts where an event does, so none of them is
/// part of one.
fn comment_lines(b: &[u8]) -> usize {
    let mut n = 0;
    while n < b.len() && matches!(b[n], b':' | b'\n' | b'\r') {
        let Some(i) = b[n..].iter().position(|&c| c == b'\n') else {
            break;
        };
        if b[n] == b'\r' && i != 1 {
            break;
        }
        n += i + 1;
    }
    n
}

/// Where the first complete event in `b` ends, and the length of the blank
/// line that ends it.
fn event_end(b: &[u8]) -> Option<(usize, usize)> {
    let find = |pat: &[u8]| b.windows(pat.len()).position(|w| w == pat);
    match (find(b"\n\n"), find(b"\r\n\r\n")) {
        (None, None) => None,
        (i, Some(j)) if i.is_none_or(|i| j < i) => Some((j, 4)),
        (Some(i), _) => Some((i, 2)),
        _ => None,
    }
}

/// An event cut around its JSON: the event's name, what comes before the
/// JSON, the JSON, and what comes after it.
type EventParts<'a> = (&'a str, &'a [u8], &'a [u8], &'a [u8]);

/// The one data line of an event that has JSON in it, cut around its JSON.
fn split_event(ev: &[u8]) -> Option<EventParts<'_>> {
    let mut name = "";
    let mut at: Option<(usize, usize)> = None; // the data line's start and length
    let mut off = 0;
    for l in ev.split_inclusive(|&c| c == b'\n') {
        let t = trim_line(l);
        if let Some(n) = t.strip_prefix(b"event:") {
            name = std::str::from_utf8(n).ok()?.trim();
        } else if t.starts_with(b"data:") {
            if at.is_some() {
                return None; // data over more than one line
            }
            at = Some((off, t.len()));
        }
        off += l.len();
    }
    let (off, len) = at?;
    let t = &ev[off..off + len];
    let start = if t.len() > 5 && t[5] == b' ' { 6 } else { 5 };
    let d = &t[start..];
    if d.is_empty() || d[0] != b'{' && d[0] != b'[' {
        return None;
    }
    Some((name, &ev[..off + start], d, &ev[off + len..]))
}

fn trim_line(l: &[u8]) -> &[u8] {
    let mut e = l.len();
    while e > 0 && matches!(l[e - 1], b'\r' | b'\n') {
        e -= 1;
    }
    &l[..e]
}

fn rebuild(pre: &[u8], data: &[u8], post: &[u8]) -> Vec<u8> {
    [pre, data, post].concat()
}

/// Says an event is one of WAIT_EVENTS.
fn waiting(name: &str, data: &[u8]) -> bool {
    let typ = type_of(data);
    WAIT_EVENTS.contains(&if typ.is_empty() { name } else { typ.as_str() })
}

/// Says a string of a vendor's answer goes on as it came: what the vendor
/// sealed, and a file's encoded data.
fn sealed_text(key: &str, s: &str) -> bool {
    SIGNATURE_KEYS.contains(&key)
        || s.starts_with("data:")
        || key == "data" && s.len() >= 64 && !s.contains([' ', '\n'])
}

/// What says which text an event goes on with: its type, its item, and
/// the indexes of its output and content.
fn event_ctx(data: &[u8]) -> String {
    let mut b = String::new();
    walk(data, &mut |path, _, s| {
        if path == "type" || path == "item_id" {
            b.push_str(&format!("{path}={s};"));
        }
        None
    });
    for k in [
        &b"\"index\":"[..],
        b"\"output_index\":",
        b"\"content_index\":",
    ] {
        if let Some(i) = data.windows(k.len()).position(|w| w == k) {
            let j = i + k.len();
            let mut e = j;
            while e < data.len() && (data[e].is_ascii_digit() || data[e] == b' ') {
                e += 1;
            }
            b.push_str(&String::from_utf8_lossy(k));
            b.push_str(&String::from_utf8_lossy(&data[j..e]));
            b.push(';');
        }
    }
    b
}

/// Says `data` has a string at each of the paths.
fn has_paths(data: &[u8], paths: &HashMap<String, String>) -> bool {
    let mut found = 0;
    walk(data, &mut |p, _, _| {
        if paths.contains_key(p) {
            found += 1;
        }
        None
    });
    found == paths.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framing() {
        assert_eq!(comment_lines(b": keepalive\n\nevent: x"), 13);
        assert_eq!(comment_lines(b": keep"), 0);
        assert_eq!(event_end(b"data: {}\n\nrest"), Some((8, 2)));
        assert_eq!(event_end(b"data: {}\r\n\r\n"), Some((8, 4)));
        assert_eq!(event_end(b"data: {}\n"), None);
        let (name, pre, data, post) = split_event(b"event: delta\ndata: {\"a\":1}\n\n").unwrap();
        assert_eq!(
            (name, pre, data, post),
            (
                "delta",
                &b"event: delta\ndata: "[..],
                &b"{\"a\":1}"[..],
                &b"\n\n"[..]
            )
        );
        assert!(split_event(b"data: DONE\n\n").is_none());
    }
}
