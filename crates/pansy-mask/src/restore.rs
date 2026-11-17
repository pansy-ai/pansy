//! Putting values back in what a vendor answers.
//!
//! A model doesn't always write a placeholder back as it was given: it
//! writes {{ PHONE_b5puzbfv }} as a template has it, drops a brace, or the
//! braces, or changes the case. Any of those is put back too when what is
//! left names a placeholder this masker made: its kind and the 8 of its
//! hash, which no other text has by chance.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;

use crate::Masker;
use crate::json::json_escape;
use crate::options::MAX_KIND;
use crate::re::compile;
use crate::text::word_byte;

/// A placeholder without its braces, as a model may write one back:
/// API_KEY_k3v7x2mq, phone_b5puzbfv, PHONE_B5PUZBFV.
static PLACEHOLDER_BODY: LazyLock<Regex> =
    LazyLock::new(|| compile(r"[A-Za-z][A-Za-z0-9_]*_[A-Za-z2-7]{8}"));

/// The longest placeholder: {{, a kind, _, the 8 of the hash, }}.
const MAX_PLACEHOLDER: usize = 2 + MAX_KIND + 1 + 8 + 2;

impl Masker {
    /// Puts the values back in `s`, counting in `put` each placeholder put
    /// back. `escaped` is for text that is itself JSON (a tool call's
    /// arguments), where a value goes in escaped.
    pub(crate) fn restore_into(
        &self,
        s: &str,
        escaped: bool,
        mut put: Option<&mut BTreeMap<String, usize>>,
    ) -> String {
        if !s.contains('_') {
            return s.to_owned();
        }
        let st = self.store.read().unwrap_or_else(|e| e.into_inner());
        if st.values.is_empty() {
            return s.to_owned();
        }
        let b = s.as_bytes();
        let mut out = String::new();
        let mut last = 0;
        for m in PLACEHOLDER_BODY.find_iter(s) {
            let (a, z) = (m.start(), m.end());
            if a < last || a > 0 && word_byte(b[a - 1]) || z < s.len() && word_byte(b[z]) {
                continue;
            }
            let p = canonical(&s[a..z]);
            let Some(v) = st.values.get(&p) else {
                continue;
            };
            let (a, z) = braces(s, a, z);
            if a < last {
                continue;
            }
            if let Some(put) = put.as_deref_mut() {
                *put.entry(p).or_default() += 1;
            }
            out.push_str(&s[last..a]);
            if escaped {
                out.push_str(&json_escape(v));
            } else {
                out.push_str(v);
            }
            last = z;
        }
        if last == 0 {
            return s.to_owned();
        }
        out.push_str(&s[last..]);
        out
    }

    /// How much of the end of `s` may be the start of a placeholder the
    /// next piece of a stream finishes, in any of the ways restoring takes
    /// one back: {{API_, {{ phone_b5, PHONE_b5puzbfv} — or, with no brace
    /// before it, a last word that starts as a placeholder of a kind held
    /// does (PHO, PHONE_b5pu).
    pub(crate) fn partial_tail(&self, s: &str) -> usize {
        match brace_tail(s) {
            0 => self.word_tail(s),
            n => n,
        }
    }

    /// The last word of `s`, where it may be a placeholder's body as a
    /// model writes one without braces: the start of one of the kinds held,
    /// up to the 8 of its hash. A word of text that only starts like a kind
    /// is held for an event and then goes on, unchanged.
    fn word_tail(&self, s: &str) -> usize {
        let b = s.as_bytes();
        let mut i = s.len();
        while i > 0 && word_byte(b[i - 1]) {
            i -= 1;
        }
        let w = &s[i..];
        if w.is_empty() || w.len() > MAX_KIND + 9 || !w.as_bytes()[0].is_ascii_uppercase() {
            return 0;
        }
        let u = w.to_ascii_uppercase();
        let st = self.store.read().unwrap_or_else(|e| e.into_inner());
        for k in &st.kinds {
            let k = format!("{k}_");
            if k.starts_with(&u) {
                return w.len();
            }
            if u.starts_with(&k)
                && u.len() - k.len() <= 8
                && u[k.len()..]
                    .bytes()
                    .all(|c| c.is_ascii_uppercase() || (b'2'..=b'7').contains(&c))
            {
                return w.len();
            }
        }
        0
    }
}

/// The placeholder masking wrote for `body`, whatever its case.
fn canonical(body: &str) -> String {
    let cut = body.len() - 8;
    format!(
        "{{{{{}{}}}}}",
        body[..cut].to_ascii_uppercase(),
        body[cut..].to_ascii_lowercase()
    )
}

/// `s[a..z]`, a placeholder's body, widened over the braces the model wrote
/// around it, and the spaces inside them: both of {{ and }}, or one of them
/// dropped ({{X} or {X}}), or just {{ or just }}. A single brace on one
/// side alone is the text's own ({X as in code) and stays.
fn braces(s: &str, mut a: usize, mut z: usize) -> (usize, usize) {
    let b = s.as_bytes();
    let (mut la, mut l) = (a, 0);
    while la > 0 && matches!(b[la - 1], b' ' | b'\t') {
        la -= 1;
    }
    while l < 2 && la > 0 && b[la - 1] == b'{' {
        la -= 1;
        l += 1;
    }
    let (mut rz, mut r) = (z, 0);
    while rz < s.len() && matches!(b[rz], b' ' | b'\t') {
        rz += 1;
    }
    while r < 2 && rz < s.len() && b[rz] == b'}' {
        rz += 1;
        r += 1;
    }
    if l == 2 || l == 1 && r >= 1 {
        a = la;
    }
    if r == 2 || r == 1 && l >= 1 {
        z = rz;
    }
    (a, z)
}

/// A tail from { or {{ on: then spaces, a placeholder's body or the start
/// of one, then spaces and one } at most.
fn brace_tail(s: &str) -> usize {
    let Some(mut i) = s.rfind('{') else {
        return 0;
    };
    if i > 0 && s.as_bytes()[i - 1] == b'{' {
        i -= 1;
    }
    let t = &s[i..];
    if t.len() >= MAX_PLACEHOLDER + 4 || t.contains("}}") {
        return 0;
    }
    let body = t.trim_start_matches('{').trim_start_matches([' ', '\t']);
    if body.is_empty() {
        return t.len(); // {, {{, {{␠
    }
    if !body.as_bytes()[0].is_ascii_alphabetic() {
        return 0;
    }
    let j = body.bytes().take_while(|&c| word_byte(c)).count();
    let rest = body[j..].trim_start_matches([' ', '\t']);
    if !rest.is_empty() && rest != "}" {
        return 0;
    }
    if j < body.len()
        && PLACEHOLDER_BODY
            .find(&body[..j])
            .is_none_or(|m| m.range() != (0..j))
    {
        return 0; // a space or a brace after something that is no placeholder
    }
    t.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tails() {
        assert_eq!(brace_tail("use {{API_"), 6);
        assert_eq!(brace_tail("use {"), 1);
        assert_eq!(brace_tail("use {{ "), 3);
        assert_eq!(
            brace_tail("x {{API_KEY_k3v7x2mq}"),
            "{{API_KEY_k3v7x2mq}".len()
        );
        assert_eq!(brace_tail("x {{API_KEY_k3v7x2mq}} done"), 0);
        assert_eq!(brace_tail("fn main() {"), 1);
        assert_eq!(brace_tail("{ \"a\": 1"), 0);
        assert_eq!(braces("a {{ X_abcdefgh }} b", 5, 15), (2, 18));
        assert_eq!(braces("a {X_abcdefgh b", 3, 13), (3, 13));
        assert_eq!(canonical("api_key_K3V7X2MQ"), "{{API_KEY_k3v7x2mq}}");
    }
}
