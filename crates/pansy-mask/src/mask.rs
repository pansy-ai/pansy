//! Masking one text: every finder's values, judged together, swapped for
//! their placeholders.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use regex::Regex;
use sha2::{Digest, Sha256};

use crate::finder::{Finder, rule_spans};
use crate::options::{Action, Options, Rule, RuleKind};
use crate::re::{Pat, compile};
use crate::store::{MIN_KNOWN, hint};
use crate::text::{DIGITS, is_in, is_space};
use crate::{Hit, Masker};

/// Which finder found a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum By {
    /// the rules built in
    Rule,
    /// the user's own rules
    Yours,
    /// the user's words
    Word,
    /// a value masked before
    Known,
}

impl By {
    pub fn name(self) -> &'static str {
        match self {
            By::Rule => "rule",
            By::Yours => "yours",
            By::Word => "word",
            By::Known => "known",
        }
    }
}

/// A value found: where it is in the text, its kind and its finder.
#[derive(Debug, Clone)]
pub(crate) struct Span {
    pub start: usize,
    pub end: usize,
    pub kind: String,
    pub by: By,
    /// a whole bare value of JSON text, whose placeholder goes in quoted
    pub bare: bool,
}

impl Span {
    pub fn new(start: usize, end: usize, kind: &str, by: By) -> Span {
        Span {
            start,
            end,
            kind: kind.to_owned(),
            by,
            bare: false,
        }
    }
}

/// What one call's texts share: the user's rules, compiled.
pub(crate) struct Cx {
    custom: Vec<Finder>,
}

impl Cx {
    pub fn new(o: &Options) -> Cx {
        Cx {
            custom: if o.secrets {
                custom_finders(&o.rules)
            } else {
                Vec::new()
            },
        }
    }
}

/// What masking did: the values found, each value masked with its
/// placeholder, and whether one of them holds the request back.
#[derive(Default)]
pub(crate) struct Out {
    pub hits: Vec<Hit>,
    pub seen: Vec<(String, String)>,
    pub blocked: bool,
}

/// A placeholder as masking writes it.
pub(crate) static PLACEHOLDER: LazyLock<Regex> =
    LazyLock::new(|| compile(r"\{\{[A-Z][A-Z0-9_]*_[a-z2-7]{8}\}\}"));

/// A rule's pattern, by its kind and value; `None` when it doesn't compile.
type Compiled = HashMap<(RuleKind, String), Option<Regex>>;

/// The user's rules, compiled once each: masking runs on every string of
/// every request.
static COMPILED: LazyLock<Mutex<Compiled>> = LazyLock::new(Default::default);

/// The shortest value a user's rule masks.
const MIN_CUSTOM_LEN: usize = 4;

/// The user's rules as finders, the ones that work.
fn custom_finders(rules: &[Rule]) -> Vec<Finder> {
    let mut out = Vec::new();
    for r in rules {
        let value = r.value.trim();
        if value.is_empty() || value.len() > crate::options::MAX_REGEX {
            continue;
        }
        let r = Rule {
            value: value.to_owned(),
            ..r.clone()
        };
        let re = {
            let mut m = COMPILED.lock().unwrap_or_else(|e| e.into_inner());
            if m.len() > 1024 {
                m.clear();
            }
            m.entry((r.kind, r.value.clone()))
                .or_insert_with(|| r.compile().ok().filter(|re| !re.is_match("")))
                .clone()
        };
        let Some(re) = re else {
            continue;
        };
        let mut f = Finder::new(&r.placeholder_kind(), "").ok(|v| v.len() >= MIN_CUSTOM_LEN);
        f.re = Pat::of(re);
        f.yours = true;
        if r.kind == RuleKind::Prefix {
            f.markers = vec![r.value.clone()];
        }
        out.push(f);
    }
    out
}

/// Two generations of what masking a text found known in it the first
/// time, by a hash of the text and of what the options cover.
#[derive(Default)]
pub(crate) struct Sent {
    cur: HashMap<[u8; 32], Vec<Span>>,
    old: HashMap<[u8; 32], Vec<Span>>,
}

/// How many texts a generation of `Sent` holds.
const SENT_ENTRIES: usize = 1 << 17;

impl Sent {
    fn get(&mut self, k: &[u8; 32]) -> Option<Vec<Span>> {
        if let Some(sp) = self.cur.get(k) {
            return Some(sp.clone());
        }
        let sp = self.old.remove(k)?;
        self.add(*k, sp.clone()); // still in use: kept past the next turnover
        Some(sp)
    }

    fn add(&mut self, k: [u8; 32], sp: Vec<Span>) {
        if self.cur.len() >= SENT_ENTRIES {
            self.old = std::mem::take(&mut self.cur);
        }
        self.cur.insert(k, sp);
    }
}

impl Masker {
    /// Swaps what `o` covers in `raw` for placeholders.
    ///
    /// `escaped` says `raw` holds JSON text of its own (a tool call's
    /// arguments), which the vendor reads after one more unescaping. There
    /// the finders read it as the vendor does (`unescaped_view`): the \n
    /// before an address is a newline, not an n that starts it. No value
    /// starts or ends inside an escape, none crosses a string's quotes, and
    /// what is kept to put back is the value as it reads unescaped.
    pub(crate) fn mask_str(
        &self,
        raw: &str,
        o: &Options,
        cx: &Cx,
        out: &mut Out,
        escaped: bool,
    ) -> String {
        if raw.len() < 3 {
            return raw.to_owned();
        }
        let view;
        let s = if escaped {
            view = unescaped_view(raw);
            view.as_str()
        } else {
            raw
        };
        let mut found = crate::words::word_spans(s, o);
        found.extend(rule_spans(s, o, &cx.custom));
        // after the rules', so where a rule finds the same value it keeps
        // the kind it has always had
        found.extend(self.sent_known(s, o));
        if escaped {
            found = in_strings(raw, outside_escapes(raw, found));
        }
        crate::ids::no_ids(s, &mut found);
        no_numbers(s, &mut found);
        self.apply(raw, found, o, out, escaped)
    }

    /// The known values in `s`, as they were the first time `s` was
    /// masked under what `o` covers. An agent sends its whole conversation
    /// on every turn, and a vendor caches it by its bytes: a value that
    /// became known since, masked in an earlier message that had it as
    /// written, would change the prefix the vendor cached and keep nothing
    /// from it, since the text went to it as written. So a text sent before
    /// goes again as it went, and a value known since is masked in new text
    /// only.
    pub(crate) fn sent_known(&self, s: &str, o: &Options) -> Vec<crate::mask::Span> {
        if !o.secrets && !o.personal || s.len() < MIN_KNOWN {
            return Vec::new();
        }
        let mut h = Sha256::new();
        h.update(format!("known\0{}\0{}\0", o.secrets, o.personal));
        for (k, on) in &o.kinds {
            h.update(format!("{k}={on},"));
        }
        h.update([0]);
        h.update(s.as_bytes());
        let key: [u8; 32] = h.finalize().into();
        let mut sent = self.sent.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(sp) = sent.get(&key) {
            return sp;
        }
        let sp =
            crate::known::known_spans(s, o, &self.store.read().unwrap_or_else(|e| e.into_inner()));
        sent.add(key, sp.clone());
        sp
    }

    /// Swaps each of `found` in `s` for its placeholder, the first and
    /// longest where they overlap, nothing inside a placeholder already
    /// there. A value `o` allows is left as it is (the user's words still
    /// mask, inside one too), and one of a kind `o` warns of is left and
    /// told.
    pub(crate) fn apply(
        &self,
        s: &str,
        mut found: Vec<Span>,
        o: &Options,
        out: &mut Out,
        escaped: bool,
    ) -> String {
        if found.is_empty() {
            return s.to_owned();
        }
        if s.contains("{{") {
            for p in PLACEHOLDER.find_iter(s) {
                found.retain(|f| !(f.start < p.end() && f.end > p.start()));
            }
            if found.is_empty() {
                return s.to_owned();
            }
        }
        // the first to start, and of those the longest, wins; of two the
        // same, the one found first, so a value's placeholder is the same
        // every time (an API key in API_KEY=… is an API_KEY, not a SECRET)
        found.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));
        let mut b = String::with_capacity(s.len() + 32);
        let (mut last, mut next, mut kept) = (0, 0, 0);
        for f in found {
            if f.start < next || f.start < kept && f.by != By::Word {
                continue;
            }
            let raw = &s[f.start..f.end];
            if f.by != By::Word && o.allows(raw) {
                kept = kept.max(f.end);
                continue;
            }
            next = f.end;
            let v = if escaped {
                json_unescape(raw)
            } else {
                raw.to_owned()
            };
            let action = o.action(&f.kind);
            let (head, len) = hint(&f.kind, &v);
            let mut h = Hit {
                placeholder: String::new(),
                kind: f.kind.clone(),
                by: f.by.name().to_owned(),
                head,
                len,
                action,
                path: String::new(),
            };
            if action == Action::Warn {
                h.placeholder = crate::store::tag(&self.key, &f.kind, &v);
                out.hits.push(h);
                continue;
            }
            b.push_str(&s[last..f.start]);
            h.placeholder = self.put(&f.kind, &v);
            if f.bare {
                b.push('"');
                b.push_str(&h.placeholder);
                b.push('"');
            } else {
                b.push_str(&h.placeholder);
            }
            out.seen.push((v, h.placeholder.clone()));
            out.blocked |= action == Action::Block;
            out.hits.push(h);
            last = f.end;
        }
        b.push_str(&s[last..]);
        b
    }

    /// The placeholder `v` of `kind` goes as, with `v` held to put back.
    pub(crate) fn put(&self, kind: &str, v: &str) -> String {
        let p = crate::store::tag(&self.key, kind, v);
        self.store
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .put(&p, kind, v);
        p
    }
}

/// `s`, JSON text, as long as `s` and read as the vendor reads it once
/// unescaped, as far as the finders tell: an escape that stands for a
/// newline, a tab or another control character has its letter read as one
/// (\n as \ and a newline), and a \u escape's u and digits as line breaks.
/// A backslash, a quote and a slash escaped stay as written.
pub(crate) fn unescaped_view(s: &str) -> String {
    if !s.contains('\\') {
        return s.to_owned();
    }
    let mut b = s.as_bytes().to_vec();
    let mut i = 0;
    while i + 1 < b.len() {
        if b[i] != b'\\' {
            i += 1;
            continue;
        }
        i += 1;
        match b[i] {
            b'n' | b'b' | b'f' => b[i] = b'\n',
            b'r' => b[i] = b'\r',
            b't' => b[i] = b'\t',
            b'u' => {
                let mut j = i;
                while j < i + 5 && j < b.len() && b[j] < 0x80 {
                    b[j] = b'\n';
                    j += 1;
                }
                i += 4;
            }
            _ => {}
        }
        i += 1;
    }
    String::from_utf8(b).unwrap_or_else(|_| s.to_owned())
}

/// `v`, a JSON string's text, unescaped; as it is where it isn't one.
pub(crate) fn json_unescape(v: &str) -> String {
    if !v.contains('\\') {
        return v.to_owned();
    }
    crate::json::unquote(v.as_bytes()).unwrap_or_else(|| v.to_owned())
}

/// `found` with no value starting or ending inside one of the escapes of
/// `s`, JSON text: after an escape's backslash, as the n of \n, or among
/// the digits of a \u escape. A value is cut back to the escapes' edges, so
/// what it holds is still masked; one left with nothing goes.
fn outside_escapes(s: &str, found: Vec<Span>) -> Vec<Span> {
    if found.is_empty() || !s.contains('\\') {
        return found;
    }
    // esc[i], for i inside an escape past its backslash: where the escape
    // starts and ends, plus one each so a zero is outside one
    let b = s.as_bytes();
    let mut esc = vec![(0usize, 0usize); s.len() + 1];
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'\\' {
            i += 1;
            continue;
        }
        let n = if i + 1 < b.len() && b[i + 1] == b'u' {
            6
        } else {
            2
        };
        let end = (i + n).min(b.len());
        for e in &mut esc[i + 1..end] {
            *e = (i + 1, end + 1);
        }
        i = end;
    }
    found
        .into_iter()
        .filter_map(|mut f| {
            if esc[f.start].1 > 0 {
                f.start = esc[f.start].1 - 1;
            }
            if esc[f.end].0 > 0 {
                f.end = esc[f.end].0 - 1;
            }
            (f.start < f.end && s.is_char_boundary(f.start) && s.is_char_boundary(f.end))
                .then_some(f)
        })
        .collect()
}

/// `found`, for `s` JSON text (a tool call's arguments), with each value
/// inside the one string it starts in: cut at that string's closing quote,
/// or started at the first string inside it. A whole bare value (a number
/// under "password":) is kept, and its placeholder goes in quoted; one in
/// nothing else, across a key's quotes, a colon or a brace, goes. Text that
/// isn't JSON is left as it is.
fn in_strings(s: &str, found: Vec<Span>) -> Vec<Span> {
    if found.is_empty() || serde_json::from_str::<serde::de::IgnoredAny>(s).is_err() {
        return found;
    }
    // inside[i] is 1 + which string literal byte i is the text of, 0 for
    // one outside every string (its quotes among them)
    let b = s.as_bytes();
    let mut inside = vec![0u32; s.len()];
    let mut n = 0;
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'"' {
            i += 1;
            continue;
        }
        n += 1;
        let mut j = i + 1;
        while j < b.len() && b[j] != b'"' {
            inside[j] = n;
            if b[j] == b'\\' && j + 1 < b.len() {
                j += 1;
                inside[j] = n;
            }
            j += 1;
        }
        i = j + 1;
    }
    let mut out = Vec::with_capacity(found.len());
    for mut f in found {
        let mut a = f.start;
        while a < f.end && inside[a] == 0 {
            a += 1;
        }
        if a == f.end {
            if bare_value(s, f.start, f.end) {
                f.bare = true;
                out.push(f);
            }
            continue;
        }
        let mut z = a;
        while z < f.end && inside[z] == inside[a] {
            z += 1;
        }
        if s.is_char_boundary(a) && s.is_char_boundary(z) {
            (f.start, f.end) = (a, z);
            out.push(f);
        }
    }
    out
}

/// Says `s[a..b]`, JSON text outside its strings, is all of one bare value:
/// an object's (after :) or an array's (after [ or ,), with the , } or ]
/// that ends it after it.
fn bare_value(s: &str, a: usize, b: usize) -> bool {
    let bs = s.as_bytes();
    if a >= b || bs[a..b].iter().any(|&c| is_in(" \t\r\n{}[]\",:", c)) {
        return false;
    }
    let mut i = a;
    while i > 0 && is_space(bs[i - 1]) {
        i -= 1;
    }
    let mut k = b;
    while k < s.len() && is_space(bs[k]) {
        k += 1;
    }
    i > 0 && is_in(":[,", bs[i - 1]) && (k == s.len() || is_in(",}]", bs[k]))
}

/// `found` without what is a piece of a decimal number of `s` (the digits
/// after the point of 5.13800138000), or a number that stands bare as a
/// JSON value ("timestamp": 17929036837). Only personal data's kinds, by a
/// finder other than the user's: a secret is masked wherever it is, and
/// what the user chose stays theirs.
fn no_numbers(s: &str, found: &mut Vec<Span>) {
    found.retain(|f| {
        !(crate::known::personal(&f.kind)
            && f.by != By::Word
            && f.by != By::Yours
            && (in_decimal(s, f.start, f.end) || json_number(s, f.start, f.end)))
    });
}

/// Says `s[a..b]` is all of a bare JSON number, by where it sits: digits
/// only, an object's value or an element of an array of numbers, and
/// nothing after it but the , } or ] that ends a value.
fn json_number(s: &str, a: usize, b: usize) -> bool {
    let bs = s.as_bytes();
    if a >= b || !bs[a..b].iter().all(u8::is_ascii_digit) {
        return false;
    }
    let mut k = b;
    while k < s.len() && is_space(bs[k]) {
        k += 1;
    }
    if k < s.len() && !is_in(",}]", bs[k]) {
        return false;
    }
    let mut i = a;
    while i > 0 && is_space(bs[i - 1]) {
        i -= 1;
    }
    if i == 0 {
        return false;
    }
    match bs[i - 1] {
        b':' => {
            let mut j = i - 1;
            while j > 0 && is_space(bs[j - 1]) {
                j -= 1;
            }
            j > 0 && bs[j - 1] == b'"'
        }
        b'[' | b',' => {
            // back over the numbers before it to the array's [
            for &c in bs[..i].iter().rev() {
                if c == b'[' {
                    return true;
                }
                if !is_in(DIGITS, c) && !is_in(",.-+eE", c) && !is_space(c) {
                    return false;
                }
            }
            false
        }
        _ => false,
    }
}

/// Says `s[a..b]` starts with a digit that is a decimal's fraction or ends
/// with a digit that is its whole part.
fn in_decimal(s: &str, a: usize, b: usize) -> bool {
    let bs = s.as_bytes();
    let digit = |i: usize| i < bs.len() && bs[i].is_ascii_digit();
    digit(a) && a >= 2 && bs[a - 1] == b'.' && digit(a - 2)
        || b >= 1 && digit(b - 1) && b + 1 < bs.len() && bs[b] == b'.' && digit(b + 1)
}
