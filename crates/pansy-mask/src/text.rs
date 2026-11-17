//! Small helpers for reading text by its bytes, as the finders do: a
//! place in a text is a byte's, and what is around it is looked at a byte
//! at a time where only ASCII matters.

pub(crate) const ALNUM: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
pub(crate) const TOKEN_CH: &str =
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-";
pub(crate) const DIGITS: &str = "0123456789";

/// Says `c` is one of the bytes of `set`.
pub(crate) fn is_in(set: &str, c: u8) -> bool {
    set.as_bytes().contains(&c)
}

pub(crate) fn word_byte(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphanumeric()
}

pub(crate) fn is_letter(c: u8) -> bool {
    c.is_ascii_alphabetic()
}

pub(crate) fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r')
}

/// Says `s` has one of `subs`.
pub(crate) fn contains_one<S: AsRef<str>>(s: &str, subs: &[S]) -> bool {
    subs.iter().any(|x| s.contains(x.as_ref()))
}

/// `s` without the characters of `set` at either end.
pub(crate) fn trim<'a>(s: &'a str, set: &str) -> &'a str {
    s.trim_matches(|c| set.contains(c))
}

pub(crate) fn trim_left<'a>(s: &'a str, set: &str) -> &'a str {
    s.trim_start_matches(|c| set.contains(c))
}

pub(crate) fn trim_right<'a>(s: &'a str, set: &str) -> &'a str {
    s.trim_end_matches(|c| set.contains(c))
}

/// The nearest place at or before `i` where a character starts.
pub(crate) fn floor(s: &str, mut i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// The nearest place at or after `i` where a character starts.
pub(crate) fn ceil(s: &str, mut i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    while !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// `s[a..b]` with each end moved in to where a character starts.
pub(crate) fn sub(s: &str, a: usize, b: usize) -> &str {
    let a = ceil(s, a);
    let b = floor(s, b).max(a);
    &s[a..b]
}

/// The last character of `s`, or NUL.
pub(crate) fn last_char(s: &str) -> char {
    s.chars().next_back().unwrap_or('\0')
}

/// `s` with its ASCII letters in lower case, as long as `s`, so a place in
/// one is the same place in the other.
pub(crate) fn lower_ascii(s: &str) -> String {
    s.to_ascii_lowercase()
}

/// Says `s` is digits only, and not empty.
pub(crate) fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit())
}

/// Says `s` ends in the word `w`: no letter, digit or `_` before it.
pub(crate) fn word_end(s: &str, w: &str) -> bool {
    s.ends_with(w) && (s.len() == w.len() || !word_byte(s.as_bytes()[s.len() - w.len() - 1]))
}
