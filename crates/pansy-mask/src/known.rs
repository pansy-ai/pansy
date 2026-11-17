//! A value masked once is masked again wherever it is in a request, as it
//! is, whatever is around it. The rules find a value by what is around it
//! (DB_PASSWORD=, a bound an email may not touch), and the vendor answers
//! about a placeholder in words of its own: "the password is {{SECRET_…}}".
//! The agent reads the value there, and on the next turn that text comes
//! back in the history, where no rule matches it. Matched as known, it goes
//! as the same placeholder again.

use crate::mask::{By, Span};
use crate::options::{CATEGORIES, Options, PERSONAL};
use crate::store::{KnownValue, MIN_KNOWN, Store, gram};
use crate::text::{ALNUM, DIGITS, is_in};

/// The kinds always masked with `personal`.
const ALWAYS_PERSONAL: &[&str] = &["EMAIL", "PHONE", "ID_CARD", "BANK_CARD"];

/// The categories a kind of personal data is found under: a user name is
/// under both home folders and user@host. None for a secret's kind, the
/// user's rules' among them.
fn categories(kind: &str) -> Vec<&'static str> {
    if ALWAYS_PERSONAL.contains(&kind) {
        return vec![PERSONAL];
    }
    CATEGORIES
        .iter()
        .filter(|c| c.kinds.contains(&kind))
        .map(|c| c.id)
        .collect()
}

/// Says `kind` is personal data's.
pub(crate) fn personal(kind: &str) -> bool {
    !categories(kind).is_empty()
}

/// Says `o` masks a known value of `kind`: a personal one when one of the
/// categories it is found under is on, any other when `o` masks secrets.
fn covers_kind(o: &Options, kind: &str) -> bool {
    let cats = categories(kind);
    if cats.is_empty() {
        return o.secrets;
    }
    cats.iter().any(|c| o.covers(c))
}

/// Where `s` has a known value `o` masks; where two start at the same
/// place, the longer.
pub(crate) fn known_spans(s: &str, o: &Options, st: &Store) -> Vec<Span> {
    if !o.secrets && !o.personal || s.len() < MIN_KNOWN || st.known.is_empty() {
        return Vec::new();
    }
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + MIN_KNOWN <= b.len() {
        let g = gram(b, i);
        if st.known_bits[(g / 64) as usize] & (1 << (g % 64)) == 0 {
            i += 1;
            continue;
        }
        let mut best: Option<&KnownValue> = None;
        if let Some(list) = st.known.get(&b[i..i + MIN_KNOWN]) {
            for k in list {
                if !covers_kind(o, &k.kind)
                    || best.is_some_and(|x| k.v.len() <= x.v.len())
                    || !b[i..].starts_with(k.v.as_bytes())
                    || !stands_alone(&k.kind, s, i, i + k.v.len())
                {
                    continue;
                }
                best = Some(k);
            }
        }
        match best {
            Some(k) => {
                out.push(Span::new(i, i + k.v.len(), &k.kind, By::Known));
                i += k.v.len();
            }
            None => i += 1,
        }
    }
    out
}

/// Says `s[a..b]` is the known value itself, not a piece of a longer one: a
/// number with no letter or digit beside it, an email that is not part of
/// a longer address, a secret with no letter or digit against it.
fn stands_alone(kind: &str, s: &str, a: usize, b: usize) -> bool {
    let bs = s.as_bytes();
    let before = if a > 0 { bs[a - 1] } else { 0 };
    let after = if b < s.len() { bs[b] } else { 0 };
    let is = |c: u8, set: &str| c != 0 && is_in(set, c);
    let alnum_and = |extra: &str| format!("{ALNUM}{extra}");
    match kind {
        "PHONE" | "BANK_CARD" | "ID_CARD" | "IBAN" | "SSN" | "PASSPORT" | "SERIAL" => {
            !is(before, ALNUM) && !is(after, ALNUM)
        }
        "IP" | "MAC" => {
            // not a piece of a longer address or a version
            if is(before, &alnum_and(".:")) || is(after, &alnum_and(":")) {
                return false;
            }
            after != b'.' || b + 1 >= s.len() || !is(bs[b + 1], DIGITS)
        }
        "USERNAME" => crate::more::name_alone(s, a, b),
        "EMAIL" => {
            if is(before, &alnum_and("._%+-")) || is(after, &alnum_and("_%+")) {
                return false;
            }
            // a.b@c.com.cn is another address; a.b@c.com. ends a sentence
            !is(after, ".-") || b + 1 >= s.len() || !is(bs[b + 1], ALNUM)
        }
        _ => {
            !(is(before, ALNUM) && is(bs[a], ALNUM)) && !(is(after, ALNUM) && is(bs[b - 1], ALNUM))
        }
    }
}
