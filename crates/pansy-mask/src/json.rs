//! Masking a request body, and putting values back in a response body,
//! string by string: everything else in the JSON is kept byte for byte.
//!
//! In a request, every string a model reads is masked: the messages, the
//! system prompt, instructions, tool results, a tool call's arguments
//! (JSON in a string too). What the vendor needs as it is stays: ids,
//! names, kinds and roles, the model, what is sealed or encoded (a thinking
//! block's signature, reasoning's encrypted content, an image's data). What
//! a vendor sealed with a signature goes back exactly as the vendor wrote
//! it: the values masked elsewhere in the request become their placeholders
//! again there, which is what the vendor wrote, and nothing else in it is
//! touched, or the vendor turns the signature away.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use crate::fieldvalue::timestamp_value;
use crate::mask::{By, Cx, Out, Span};
use crate::re::compile;
use crate::secrets::not_a_variable;
use crate::{Change, MaskedBody, Masker, Options};

/// The keys whose strings are left alone: ids, names and kinds the vendor
/// needs as they are, and what is sealed or encoded. A tool is named by its
/// name and, for an MCP tool, its server's.
pub(crate) const KEPT: &[&str] = &[
    "signature",
    "encrypted_content",
    "data",
    "thoughtSignature",
    "thought_signature",
    "model",
    "id",
    "tool_use_id",
    "call_id",
    "item_id",
    "type",
    "role",
    "name",
    "namespace",
    "server_label",
    "server_name",
    "previous_response_id",
    "prompt_cache_key",
    "media_type",
    "mime_type",
    "mimeType",
    "url",
    "image_url",
    "file_id",
    "reasoning_effort",
    "effort",
    "stop_reason",
    "finish_reason",
    "status",
    "object",
    "event",
];

/// Says a request's string under `key`, at `path`, is left as it is. In a
/// tool call's arguments (`args`) the keys are the tool's, so only an
/// encoded data: value is kept; in what the agent defines for the vendor to
/// check (`defs`: its tools and their schemas, the output's schema) only a
/// description is masked.
fn keep_in(path: &str, key: &str, s: &str, args: &[String], defs: &[String]) -> bool {
    if s.starts_with("data:") {
        return true;
    }
    if under(path, args) {
        return false;
    }
    if under(path, defs) {
        return key != "description";
    }
    KEPT.contains(&key) || is_id(key)
}

/// Says a key holds an identifier the vendor matches against another: a
/// tool result's tool_call_id against its call's id.
fn is_id(key: &str) -> bool {
    key == "id"
        || key == "ids"
        || ["_id", "_ids", "Id", "Ids"]
            .iter()
            .any(|e| key.ends_with(e))
}

/// The keys of what a vendor seals the content beside it with.
pub(crate) const SIGNATURE_KEYS: &[&str] = &[
    "signature",
    "thoughtSignature",
    "thought_signature",
    "encrypted_content",
];

/// The request's own keys whose values the agent defines for the vendor to
/// read as structure: the tools and the choice among them, the output's
/// schema, the reasoning's settings, the client's metadata.
const DEF_KEYS: &[&str] = &[
    "tools",
    "tool_choice",
    "functions",
    "function_call",
    "toolConfig",
    "tool_config",
    "response_format",
    "text",
    "reasoning",
    "include",
    "output_format",
    "output_config",
    "generationConfig",
    "generation_config",
    "stream_options",
    "client_metadata",
];

/// Text any body with something for `scan` to find has in it.
static SCAN_MARKS: LazyLock<regex::bytes::Regex> = LazyLock::new(|| {
    let mut alts: Vec<String> = SIGNATURE_KEYS
        .iter()
        .chain(&[
            "functionCall",
            "function_call",
            "functionResponse",
            "function_response",
            "arguments",
        ])
        .chain(DEF_KEYS)
        .map(|k| regex::escape(&format!("\"{k}\"")))
        .collect();
    alts.push(regex::escape("tool_use\""));
    alts.push(regex::escape("_call\""));
    regex::bytes::Regex::new(&alts.join("|")).expect("literals compile")
});

/// Finds three kinds of objects in a body. `signed` are those a vendor
/// sealed with a signature; `args` a tool call's arguments and a Gemini
/// function's structured result, whose keys are the tool's own; `defs` what
/// the agent defines for the vendor rather than says to the model.
fn scan(body: &[u8]) -> (Vec<String>, Vec<String>, Vec<String>) {
    let (mut signed, mut args) = (Vec::new(), Vec::new());
    if !SCAN_MARKS.is_match(body) {
        return (signed, args, Vec::new());
    }
    let Ok(v) = serde_json::from_slice::<Value>(body) else {
        return (signed, args, Vec::new());
    };
    let defs = definitions(&v);
    visit("", "", &v, &mut signed, &mut args);
    (signed, args, defs)
}

fn visit(path: &str, key: &str, v: &Value, signed: &mut Vec<String>, args: &mut Vec<String>) {
    match v {
        Value::Object(x) => {
            if sealed(x) {
                signed.push(path.to_owned());
                return;
            }
            let t = x.get("type").and_then(Value::as_str).unwrap_or("");
            for (k, c) in x {
                if tool_args(t, key, k)
                    || k == "response" && (key == "functionResponse" || key == "function_response")
                {
                    args.push(join(path, k));
                    continue;
                }
                visit(&join(path, k), k, c, signed, args);
            }
        }
        Value::Array(xs) => {
            for (i, c) in xs.iter().enumerate() {
                visit(&join(path, &i.to_string()), key, c, signed, args);
            }
        }
        _ => {}
    }
}

/// Says the value under `k`, in an object of type `t` under `key`, is what
/// the model wrote for a tool to run, in any protocol: Anthropic's input
/// of every *_tool_use, a Responses *_call's arguments, input, action or
/// operation, a Chat tool call's function.arguments, Gemini's
/// functionCall args.
fn tool_args(t: &str, key: &str, k: &str) -> bool {
    if t.ends_with("tool_use") {
        k == "input"
    } else if t.ends_with("_call") {
        matches!(k, "arguments" | "input" | "action" | "operation")
    } else if matches!(key, "function" | "functionCall" | "function_call") {
        k == "arguments" || k == "args"
    } else {
        false
    }
}

/// The paths at which the agent defines rather than says: the request's
/// own DEF_KEYS, and the tools of Responses' additional_tools and
/// tool_search_output items.
fn definitions(v: &Value) -> Vec<String> {
    let Some(m) = v.as_object() else {
        return Vec::new();
    };
    let mut defs = Vec::new();
    for k in DEF_KEYS {
        match m.get(*k) {
            // Responses' text is the output's settings; a prompt's text is
            // a string
            Some(Value::Object(_)) => defs.push(k.to_string()),
            Some(_) if *k != "text" => defs.push(k.to_string()),
            _ => {}
        }
    }
    if let Some(items) = m.get("input").and_then(Value::as_array) {
        for (i, it) in items.iter().enumerate() {
            let t = it.get("type").and_then(Value::as_str).unwrap_or("");
            if t == "additional_tools" || t == "tool_search_output" {
                defs.push(format!("input.{i}.tools"));
            }
        }
    }
    defs
}

/// Says `x` is a block the vendor signed: a part with Gemini's thought
/// signature, or a thinking or reasoning block with Anthropic's signature
/// or Responses' encrypted content.
fn sealed(x: &serde_json::Map<String, Value>) -> bool {
    let t = x.get("type").and_then(Value::as_str).unwrap_or("");
    SIGNATURE_KEYS.iter().any(|k| {
        x.get(*k)
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty())
            && (*k == "thoughtSignature"
                || *k == "thought_signature"
                || t == "thinking"
                || t == "reasoning"
                || t.starts_with("reasoning."))
    })
}

/// Says `path` is in one of the objects at `roots`.
fn under(path: &str, roots: &[String]) -> bool {
    roots.iter().any(|r| {
        r.is_empty()
            || path == r
            || path.len() > r.len()
                && path.starts_with(r.as_str())
                && path.as_bytes()[r.len()] == b'.'
    })
}

pub(crate) fn join(path: &str, k: &str) -> String {
    if path.is_empty() {
        k.to_owned()
    } else {
        format!("{path}.{k}")
    }
}

/// A path as a person reads it: messages[0].content.
fn display_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len() + 4);
    for seg in path.split('.') {
        if !seg.is_empty() && seg.bytes().all(|c| c.is_ascii_digit()) {
            out.push('[');
            out.push_str(seg);
            out.push(']');
        } else {
            if !out.is_empty() {
                out.push('.');
            }
            out.push_str(seg);
        }
    }
    out
}

/// What a walk made of a document.
pub(crate) enum Walked {
    /// it isn't JSON
    NotJson,
    /// nothing changed
    Same,
    Changed(Vec<u8>),
}

/// Nesting deeper than this is taken for no JSON.
const MAX_DEPTH: usize = 512;

/// Calls `f` with every string value in the JSON document `b`, its path
/// (keys and indexes joined by dots) and the key it is under, and writes
/// what `f` returns in its place. Everything else is kept byte for byte.
pub(crate) fn walk(b: &[u8], f: &mut dyn FnMut(&str, &str, &str) -> Option<String>) -> Walked {
    let mut w = Walker {
        inp: b,
        out: Vec::new(),
        last: 0,
        f,
        depth: 0,
    };
    let i = w.ws(0);
    match w.value(i, "", "") {
        Some(end) if w.ws(end) == b.len() => {}
        _ => return Walked::NotJson,
    }
    if w.last == 0 {
        return Walked::Same;
    }
    w.out.extend_from_slice(&b[w.last..]);
    Walked::Changed(w.out)
}

struct Walker<'a, 'f> {
    inp: &'a [u8],
    out: Vec<u8>,
    last: usize,
    f: &'f mut dyn FnMut(&str, &str, &str) -> Option<String>,
    depth: usize,
}

impl Walker<'_, '_> {
    fn ws(&self, mut i: usize) -> usize {
        while i < self.inp.len() && matches!(self.inp[i], b' ' | b'\t' | b'\n' | b'\r') {
            i += 1;
        }
        i
    }

    /// The end of the string literal that starts at `i`, and whether it has
    /// an escape.
    fn str_end(&self, i: usize) -> Option<(usize, bool)> {
        let mut esc = false;
        let mut j = i + 1;
        while j < self.inp.len() {
            match self.inp[j] {
                b'\\' => {
                    esc = true;
                    j += 1;
                }
                b'"' => return Some((j + 1, esc)),
                _ => {}
            }
            j += 1;
        }
        None
    }

    fn text(&self, i: usize, end: usize, esc: bool) -> Option<String> {
        let raw = &self.inp[i + 1..end - 1];
        if esc {
            unquote(raw)
        } else {
            std::str::from_utf8(raw).ok().map(str::to_owned)
        }
    }

    fn value(&mut self, i: usize, path: &str, key: &str) -> Option<usize> {
        match *self.inp.get(i)? {
            b'{' => {
                self.depth += 1;
                if self.depth > MAX_DEPTH {
                    return None;
                }
                let mut i = self.ws(i + 1);
                if self.inp.get(i) == Some(&b'}') {
                    self.depth -= 1;
                    return Some(i + 1);
                }
                loop {
                    if self.inp.get(i) != Some(&b'"') {
                        return None;
                    }
                    let (end, esc) = self.str_end(i)?;
                    let k = self.text(i, end, esc)?;
                    i = self.ws(end);
                    if self.inp.get(i) != Some(&b':') {
                        return None;
                    }
                    let p = join(path, &k);
                    i = self.value(self.ws(i + 1), &p, &k)?;
                    i = self.ws(i);
                    match self.inp.get(i) {
                        Some(b',') => i = self.ws(i + 1),
                        Some(b'}') => {
                            self.depth -= 1;
                            return Some(i + 1);
                        }
                        _ => return None,
                    }
                }
            }
            b'[' => {
                self.depth += 1;
                if self.depth > MAX_DEPTH {
                    return None;
                }
                let mut i = self.ws(i + 1);
                if self.inp.get(i) == Some(&b']') {
                    self.depth -= 1;
                    return Some(i + 1);
                }
                let mut n = 0;
                loop {
                    // an array's strings are under the key the array is
                    i = self.value(i, &join(path, &n.to_string()), key)?;
                    i = self.ws(i);
                    match self.inp.get(i) {
                        Some(b',') => i = self.ws(i + 1),
                        Some(b']') => {
                            self.depth -= 1;
                            return Some(i + 1);
                        }
                        _ => return None,
                    }
                    n += 1;
                }
            }
            b'"' => {
                let (end, esc) = self.str_end(i)?;
                let s = self.text(i, end, esc)?;
                if let Some(t) = (self.f)(path, key, &s)
                    && t != s
                {
                    if self.out.capacity() == 0 {
                        self.out.reserve(self.inp.len() + self.inp.len() / 16 + 64);
                    }
                    self.out.extend_from_slice(&self.inp[self.last..i]);
                    self.out.push(b'"');
                    self.out.extend_from_slice(json_escape(&t).as_bytes());
                    self.out.push(b'"');
                    self.last = end;
                }
                Some(end)
            }
            _ => {
                let mut j = i;
                while j < self.inp.len() && !b",]} \t\r\n".contains(&self.inp[j]) {
                    j += 1;
                }
                (j > i).then_some(j)
            }
        }
    }
}

/// The text between a JSON string's quotes, unescaped. A lone surrogate is
/// U+FFFD; an escape JSON doesn't have is no string.
pub(crate) fn unquote(raw: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(raw).ok()?;
    if !s.contains('\\') {
        return Some(s.to_owned());
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    let hex4 = |chars: &mut std::str::Chars| -> Option<u32> {
        let h: String = chars.by_ref().take(4).collect();
        if h.len() != 4 {
            return None;
        }
        u32::from_str_radix(&h, 16).ok()
    };
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next()? {
            '"' => out.push('"'),
            '\\' => out.push('\\'),
            '/' => out.push('/'),
            'b' => out.push('\u{8}'),
            'f' => out.push('\u{c}'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            'u' => {
                let u = hex4(&mut chars)?;
                if (0xD800..0xDC00).contains(&u) {
                    let mut ahead = chars.clone();
                    if ahead.next() == Some('\\')
                        && ahead.next() == Some('u')
                        && let Some(l) = hex4(&mut ahead)
                        && (0xDC00..0xE000).contains(&l)
                    {
                        out.push(
                            char::from_u32(0x10000 + ((u - 0xD800) << 10) + (l - 0xDC00))
                                .unwrap_or('\u{FFFD}'),
                        );
                        chars = ahead;
                        continue;
                    }
                    out.push('\u{FFFD}');
                } else {
                    out.push(char::from_u32(u).unwrap_or('\u{FFFD}'));
                }
            }
            _ => return None,
        }
    }
    Some(out)
}

/// `s` as it goes between the quotes of a JSON string.
pub(crate) fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// A field's or a header's name that says its value is a secret. A name
/// that ends in pass (a db_pass) is one too; a name that only ends in key
/// is not: a sort key, a cache key, a public key hold no secret.
static SECRET_NAME: LazyLock<Regex> = LazyLock::new(|| {
    compile(
        r"(?i)password|passwd|passphrase|secret|token|api[_-]?key|access[_-]?key|private[_-]?key|priv[_-]?key|credential|authori[sz]ation|cookie|^key$|^pass$|_pass$",
    )
});

/// Says a field or a header called `name` holds a secret: a token, a key,
/// a password or a cookie; not a count of tokens (max_tokens).
fn secret_name(name: &str) -> bool {
    let n = name.to_lowercase();
    SECRET_NAME.is_match(&n) && !n.ends_with("tokens") && !n.ends_with("token_count")
}

/// Says a string under a field named for a secret holds one, whatever it
/// looks like: {"api_key": "…"} with a value no rule knows, an AKIA under
/// access_key_id. Asked only of a tool call's arguments and a tool's
/// structured result, whose keys are the tool's: a key the vendor's
/// protocol names is the vendor's to read.
fn named_secret(key: &str, s: &str) -> bool {
    !key.is_empty() && s.len() >= 8 && not_a_variable(s) && !timestamp_value(s) && secret_name(key)
}

/// Says a string under `key` holds JSON text of its own, so a value put
/// back in it goes in escaped: a tool call's arguments, whole or streamed
/// in pieces.
pub(crate) fn as_json(key: &str, event_type: &str) -> bool {
    key == "arguments"
        || key == "partial_json"
        || key == "delta" && event_type.contains("arguments")
}

/// The type a JSON object says it is, wherever its "type" is among its
/// keys.
pub(crate) fn type_of(data: &[u8]) -> String {
    let mut typ = String::new();
    walk(data, &mut |path, _, s| {
        if path == "type" {
            typ = s.to_owned();
        }
        None
    });
    typ
}

/// Swaps each value of `pairs` for its placeholder, the longest value first
/// where two start at the same place.
fn replace_seen(s: &str, pairs: &[(String, String)]) -> String {
    if pairs.is_empty() {
        return s.to_owned();
    }
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    'outer: while i < s.len() {
        for (v, p) in pairs {
            if !v.is_empty() && s.as_bytes()[i..].starts_with(v.as_bytes()) {
                out.push_str(p);
                i += v.len();
                continue 'outer;
            }
        }
        let c = s[i..].chars().next().unwrap_or('\0');
        out.push(c);
        i += c.len_utf8().max(1);
    }
    out
}

impl Masker {
    pub(crate) fn mask_body(&self, body: &[u8], o: &Options) -> MaskedBody {
        let same = || MaskedBody {
            body: body.to_vec(),
            hits: Vec::new(),
            changes: Vec::new(),
            blocked: false,
        };
        if !o.wanted() {
            return same();
        }
        let (signed, args, defs) = scan(body);
        let cx = Cx::new(o);
        let mut out = Out::default();
        let mut changes: Vec<Change> = Vec::new();
        let masked = |path: &str, key: &str, s: &str| {
            !keep_in(path, key, s, &args, &defs) && !under(path, &signed)
        };

        // note each change and where each value was found
        let mut record =
            |path: &str, s: &str, t: String, from: usize, out: &mut Out| -> Option<String> {
                let shown = display_path(path);
                for h in &mut out.hits[from..] {
                    h.path = shown.clone();
                }
                if t == s {
                    return None;
                }
                changes.push(Change {
                    path: shown,
                    before: s.to_owned(),
                    after: t.clone(),
                });
                Some(t)
            };

        let mut current = Cow::Borrowed(body);
        // what is said, then what the agent defines, so a value found in
        // what is said is masked in the definitions too, as known
        let said = walk(&current, &mut |path, key, s| {
            if s.starts_with("data:") || under(path, &signed) {
                return None;
            }
            let from = out.hits.len();
            let t = if o.secrets && under(path, &args) && named_secret(key, s) {
                self.apply(
                    s,
                    vec![Span::new(0, s.len(), "SECRET", By::Rule)],
                    o,
                    &mut out,
                    false,
                )
            } else if !masked(path, key, s) || under(path, &defs) {
                return None;
            } else {
                self.mask_str(s, o, &cx, &mut out, as_json(key, ""))
            };
            record(path, s, t, from, &mut out)
        });
        match said {
            Walked::NotJson => return same(),
            Walked::Changed(b) => current = Cow::Owned(b),
            Walked::Same => {}
        }
        if !defs.is_empty()
            && let Walked::Changed(b) = walk(&current, &mut |path, key, s| {
                if !masked(path, key, s) || !under(path, &defs) {
                    return None;
                }
                let from = out.hits.len();
                let t = self.mask_str(s, o, &cx, &mut out, as_json(key, ""));
                record(path, s, t, from, &mut out)
            })
        {
            current = Cow::Owned(b);
        }
        if !signed.is_empty() {
            // the values masked elsewhere become their placeholders again,
            // as the vendor wrote them, and so does a secret masked before
            let mut pairs = out.seen.clone();
            pairs.sort_by_key(|p| std::cmp::Reverse(p.0.len()));
            let mut have = std::collections::HashSet::new();
            pairs.retain(|(v, _)| have.insert(v.clone()));
            let ko = Options {
                secrets: o.secrets,
                ..Options::default()
            };
            if let Walked::Changed(b) = walk(&current, &mut |path, key, s| {
                if keep_in(path, key, s, &args, &defs) || !under(path, &signed) {
                    return None;
                }
                let from = out.hits.len();
                let t = replace_seen(s, &pairs);
                let known = self.sent_known(&t, &ko);
                let t = self.apply(&t, known, o, &mut out, false);
                record(path, s, t, from, &mut out)
            }) {
                current = Cow::Owned(b);
            }
        }
        MaskedBody {
            body: current.into_owned(),
            hits: out.hits,
            changes,
            blocked: out.blocked,
        }
    }

    pub(crate) fn restore_body(&self, body: &[u8], put: &mut BTreeMap<String, usize>) -> Vec<u8> {
        if !self.held_in(body) {
            return body.to_vec();
        }
        let typ = type_of(body);
        let walked = walk(body, &mut |_, key, s| {
            let t = self.restore_into(s, as_json(key, &typ), Some(&mut *put));
            (t != s).then_some(t)
        });
        match walked {
            Walked::Changed(b) => b,
            Walked::Same => body.to_vec(),
            Walked::NotJson => match std::str::from_utf8(body) {
                Ok(s) => self.restore_into(s, false, Some(put)).into_bytes(),
                Err(_) => body.to_vec(),
            },
        }
    }

    /// Says `b` may have a placeholder of a value held, in any of the ways
    /// restoring takes one back: a kind, _ and the 8 of its hash, in any
    /// case, with its braces or without.
    pub(crate) fn held_in(&self, b: &[u8]) -> bool {
        let st = self.store.read().unwrap_or_else(|e| e.into_inner());
        if st.values.is_empty() {
            return false;
        }
        let mut i = 0;
        while let Some(j) = b[i..].iter().position(|&c| c == b'_') {
            i += j + 1;
            let z = i + 8;
            if z > b.len()
                || z < b.len() && crate::text::word_byte(b[z])
                || !b[i..z]
                    .iter()
                    .all(|c| c.is_ascii_alphabetic() || (b'2'..=b'7').contains(c))
            {
                continue;
            }
            let hash = String::from_utf8_lossy(&b[i..z]).to_lowercase();
            for k in &st.kinds {
                let n = k.len();
                if i > n
                    && b[i - 1 - n..i - 1].eq_ignore_ascii_case(k.as_bytes())
                    && st.values.contains_key(&format!("{{{{{k}_{hash}}}}}"))
                {
                    return true;
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(b: &str) -> Vec<(String, String, String)> {
        let mut out = Vec::new();
        walk(b.as_bytes(), &mut |p, k, s| {
            out.push((p.to_owned(), k.to_owned(), s.to_owned()));
            None
        });
        out
    }

    #[test]
    fn walks() {
        let got = strings(r#"{"a":[{"b":"x\ny"},"z"],"c\"":"😀\ud800"}"#);
        assert_eq!(got[0], ("a.0.b".into(), "b".into(), "x\ny".into()));
        assert_eq!(got[1], ("a.1".into(), "a".into(), "z".into()));
        assert_eq!(got[2], ("c\"".into(), "c\"".into(), "😀\u{FFFD}".into()));
        for bad in [
            r#"{"a":1"#,
            r#"{"a" 1}"#,
            "nope here",
            r#"["\x"]"#,
            r#"{"a":1} x"#,
        ] {
            assert!(
                matches!(walk(bad.as_bytes(), &mut |_, _, _| None), Walked::NotJson),
                "{bad}"
            );
        }
        let b = r#"{ "a" : [1, 2.5e3, true, null, "xé"] }"#.as_bytes();
        assert!(matches!(
            walk(b, &mut |_, _, s| Some(s.to_owned())),
            Walked::Same
        ));
        match walk(b, &mut |_, _, _| Some("q\"\u{1}".into())) {
            Walked::Changed(v) => assert_eq!(
                v,
                r#"{ "a" : [1, 2.5e3, true, null, "q\"\u0001"] }"#.as_bytes()
            ),
            _ => panic!(),
        }
    }

    #[test]
    fn paths() {
        assert_eq!(
            display_path("messages.0.content.1.text"),
            "messages[0].content[1].text"
        );
        assert_eq!(display_path("0.candidates"), "[0].candidates");
        assert_eq!(display_path("system"), "system");
        assert!(
            under("messages.0.content", &["messages.0".into()])
                && !under("messages.01", &["messages.0".into()])
        );
    }

    #[test]
    fn names() {
        for n in [
            "api_key",
            "client_secret",
            "db_pass",
            "access_key_id",
            "Authorization",
            "Cookie",
            "apiKey",
            "password",
        ] {
            assert!(secret_name(n), "{n}");
        }
        for n in [
            "sort_key",
            "cache_key",
            "prompt_cache_key",
            "public_key",
            "max_tokens",
            "keyboard",
        ] {
            assert!(!secret_name(n), "{n}");
        }
    }
}
