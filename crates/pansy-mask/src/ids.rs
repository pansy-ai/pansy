//! An id a backend made (a UUID, 32 hex digits or more: a row's, a
//! request's, a commit's) goes as written: the model reads it, matches it
//! and writes it back into a tool call, and a backend looks it up by every
//! one of its characters. A piece of one is never a value of its own (the
//! phone rule found 13800138000 at the end of …-a13800138000). A whole one
//! is masked only as a credential, in a field named for one (password=,
//! "refresh_token":), never in one named for an id, a cursor, a request or
//! a digest. What the user chose (their words, their rules) stays theirs.
//!
//! An element token cua-driver made for computer use (s0000002a:14) is one
//! too: the model clicks by it.

use std::sync::LazyLock;

use regex::Regex;

use crate::mask::{By, Span};
use crate::re::compile;
use crate::text::word_byte;

static UUID: LazyLock<Regex> = LazyLock::new(|| {
    compile(r"^(?i)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
});

/// `found` without what is in an id of `s`, but for what the user chose
/// and the credentials named for one.
pub(crate) fn no_ids(s: &str, found: &mut Vec<Span>) {
    found.retain(|f| {
        if f.by == By::Word || f.by == By::Yours {
            return true;
        }
        let Some((l, r)) = id_around(s, f.start, f.end) else {
            return true;
        };
        if f.start > l || f.end < r {
            return false; // a piece of an id
        }
        match f.by {
            // masked before as what it is in full: a credential. Not an
            // element token
            By::Known => !element_token(s, l, r),
            _ => match f.kind.as_str() {
                "PASSWORD" | "API_KEY" | "TOKEN" | "PRIVATE_KEY" | "WALLET_KEY" => true,
                "SECRET" => {
                    // a short digest after a key's name in a sentence
                    // (密钥为 5d41402abc4b2a76) is that key; in a field, as a
                    // long id
                    digest_shaped(&s[l..r]) && field_words(s, l).is_empty()
                        || credential_field(s, l)
                }
                _ => false,
            },
        }
    });
}

/// The id that `s[a..b]` is in, or is: the run of hex digits and hyphens
/// around it, when that run is a UUID or 32 hex digits or more, or
/// `s[a..b]` when it is an element token.
fn id_around(s: &str, a: usize, b: usize) -> Option<(usize, usize)> {
    if element_token(s, a, b) {
        return Some((a, b));
    }
    let bs = s.as_bytes();
    let (mut l, mut r) = (a, b);
    while l > 0 && hex_or_hyphen(bs[l - 1]) {
        l -= 1;
    }
    while r < s.len() && hex_or_hyphen(bs[r]) {
        r += 1;
    }
    while l < r && bs[l] == b'-' {
        l += 1;
    }
    while r > l && bs[r - 1] == b'-' {
        r -= 1;
    }
    if a < l || b > r || !bs[l..r].iter().all(|&c| hex_or_hyphen(c)) || !id_shaped(&s[l..r]) {
        return None;
    }
    // a short digest is a word of its own, not the hex end of a longer one
    if digest_shaped(&s[l..r]) && (l > 0 && word_byte(bs[l - 1]) || r < s.len() && word_byte(bs[r]))
    {
        return None;
    }
    Some((l, r))
}

/// Says `s[a..b]` is an element token, or its snapshot's part: s and 8
/// lowercase hex digits, then a colon and the index, with no letter, digit
/// or _ on either side. Without an index, only an element_token field's
/// value is one.
fn element_token(s: &str, a: usize, b: usize) -> bool {
    let bs = s.as_bytes();
    if b - a < 9 || bs[a] != b's' || a > 0 && word_byte(bs[a - 1]) {
        return false;
    }
    if !bs[a + 1..a + 9]
        .iter()
        .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c))
    {
        return false;
    }
    let mut end = a + 9;
    if end < s.len() && bs[end] == b':' {
        let mut i = end + 1;
        while i < s.len() && bs[i].is_ascii_digit() {
            i += 1;
        }
        if i == end + 1 {
            return false; // a colon and no index
        }
        end = i;
    }
    if b != a + 9 && b != end || end < s.len() && word_byte(bs[end]) {
        return false;
    }
    end > a + 9 || field_words(s, a) == ["element", "token"]
}

fn hex_or_hyphen(c: u8) -> bool {
    c == b'-' || c.is_ascii_hexdigit()
}

/// Says `v`, hex digits and hyphens, is a UUID or 32 hex digits or more
/// without a hyphen, or a shorter digest.
fn id_shaped(v: &str) -> bool {
    UUID.is_match(v) || v.len() >= 32 && !v.contains('-') || digest_shaped(v)
}

/// Says `v` is a hash's hex digits short of 32: 16 to 31 of them in one
/// case, letters and digits both, as a short commit or a 64-bit hash is
/// written (5d41402abc4b2a76). Digits alone are a number's.
pub(crate) fn digest_shaped(v: &str) -> bool {
    if v.len() < 16 || v.len() >= 32 {
        return false;
    }
    let (mut lower, mut upper, mut digits) = (0, 0, 0);
    for c in v.bytes() {
        match c {
            b'0'..=b'9' => digits += 1,
            b'a'..=b'f' => lower += 1,
            b'A'..=b'F' => upper += 1,
            _ => return false,
        }
    }
    // a hash's digits are random: of 16, about 6 are letters and 10 digits
    let letters = lower + upper;
    let least = 3.max(v.len() / 8);
    (lower == 0 || upper == 0) && letters >= least && digits >= least
}

/// Says the field whose value starts at `s[at]` is named for a credential:
/// refresh_token, DB_PASSWORD, x-api-key; not one named for an id, a
/// cursor, a request or a lookup, and not a bare "key".
fn credential_field(s: &str, at: usize) -> bool {
    let words = field_words(s, at);
    if words.len() == 1 && words[0] == "key" && key_phrase(s, at) {
        return true; // api key: …, Secret Key = …
    }
    if words.len() < 2 && (words.is_empty() || words[0] == "key") || words == ["element", "token"] {
        return false;
    }
    if matches!(
        words[words.len() - 1].as_str(),
        "id" | "ids" | "uuid" | "guid"
    ) {
        return false;
    }
    !words.iter().any(|w| {
        matches!(
            w.as_str(),
            "page"
                | "next"
                | "cursor"
                | "continuation"
                | "csrf"
                | "xsrf"
                | "trace"
                | "request"
                | "idempotency"
                | "sync"
                | "nonce"
                | "hash"
                | "digest"
                | "checksum"
                | "sha"
                | "fingerprint"
                | "etag"
                | "cache"
                | "commit"
                | "object"
                | "blob"
                | "file"
                | "content"
                | "revision"
                | "rev"
                | "partition"
                | "shard"
                | "bucket"
                | "dedup"
                | "dedupe"
                | "lookup"
                | "row"
                | "sort"
        )
    })
}

/// Says the bare key whose value starts at `s[at]` is the second word of a
/// credential's name in words: api key, access key, secret key.
fn key_phrase(s: &str, at: usize) -> bool {
    let before = crate::text::sub(s, at.saturating_sub(24), at).to_lowercase();
    let Some(i) = before.rfind("key") else {
        return false;
    };
    let before = before[..i].trim_end_matches(' ');
    [
        "api", "access", "secret", "private", "signing", "license", "licence",
    ]
    .iter()
    .any(|w| crate::text::word_end(before, w))
}

/// The words of the name of the field whose value starts at `s[at]`, none
/// where no field's name is before it.
pub(crate) fn field_words(s: &str, at: usize) -> Vec<String> {
    let b = s.as_bytes();
    let mut i = at;
    while i > 0 && crate::text::is_in(" \t\"'\\", b[i - 1]) {
        i -= 1;
    }
    if i == 0 || b[i - 1] != b':' && b[i - 1] != b'=' {
        return Vec::new();
    }
    i -= 1;
    while i > 0 && crate::text::is_in(" \t\"'\\", b[i - 1]) {
        i -= 1;
    }
    let mut j = i;
    while j > 0 && crate::fieldvalue::name_byte(b[j - 1]) {
        j -= 1;
    }
    name_words(&s[j..i])
}

/// A field's name's words in lower case: next_page_token, nextPageToken and
/// X-Next-Page-Token are next, page, token.
pub(crate) fn name_words(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut w: Vec<u8> = Vec::new();
    let b = name.as_bytes();
    let mut flush = |w: &mut Vec<u8>| {
        if !w.is_empty() {
            out.push(String::from_utf8_lossy(w).into_owned());
            w.clear();
        }
    };
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'_' | b'-' | b'.' => flush(&mut w),
            b'A'..=b'Z' => {
                if i > 0 && b[i - 1].is_ascii_lowercase() {
                    flush(&mut w);
                }
                w.push(c.to_ascii_lowercase());
            }
            _ => w.push(c),
        }
    }
    flush(&mut w);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_of_names() {
        assert_eq!(
            name_words("X-Next-Page-Token"),
            ["x", "next", "page", "token"]
        );
        assert_eq!(name_words("nextPageToken"), ["next", "page", "token"]);
        assert_eq!(name_words("手机号"), ["手机号"]);
    }

    #[test]
    fn digests() {
        assert!(digest_shaped("5d41402abc4b2a76"));
        assert!(!digest_shaped("1234567890123456"));
        assert!(!digest_shaped("5D41402abc4b2a76"));
    }
}
