//! The values masked, by placeholder, held in memory only.
//!
//! A placeholder is the same for the same value every time (a keyed hash
//! of it), so a conversation's history reads the same turn after turn:
//! what a vendor cached of it stays good, and a thinking block's signature
//! still matches the text it was made for. A restart loses nothing, since
//! the next request brings the values again.
//!
//! A value once masked is masked wherever a request has it, whatever words
//! are around it (`known`): the vendor answers about a placeholder in words
//! no rule knows ("the password is {{SECRET_…}}"), and the agent sends
//! that text back with the value in it.

use std::collections::{HashMap, HashSet};

use hmac::{Hmac, Mac};
use sha2::Sha256;

/// How many values are held; past it the half used longest ago is let go,
/// and a request that still has them brings them back.
const MAX_VALUES: usize = 200_000;

/// The shortest value matched as known: shorter ones (a password of three
/// letters in a URL) would turn up all over ordinary text.
pub(crate) const MIN_KNOWN: usize = 8;

#[derive(Clone)]
pub(crate) struct KnownValue {
    pub v: String,
    pub p: String,
    pub kind: String,
}

pub(crate) struct Store {
    /// placeholder → value
    pub values: HashMap<String, String>,
    /// the kinds of the values, for a tail of a stream that may be one
    pub kinds: HashSet<String>,
    /// when each placeholder was last made or met again, by clock
    used: HashMap<String, u64>,
    clock: u64,
    /// the values matched as known, by their first MIN_KNOWN bytes
    pub known: HashMap<Vec<u8>, Vec<KnownValue>>,
    /// a bit for the first four bytes of each known value, so most places
    /// in a text are passed over without a lookup
    pub known_bits: Vec<u64>,
}

impl Store {
    pub fn new() -> Store {
        Store {
            values: HashMap::new(),
            kinds: HashSet::new(),
            used: HashMap::new(),
            clock: 0,
            known: HashMap::new(),
            known_bits: vec![0; 1 << 14],
        }
    }

    /// Holds `v` under `p`, unless `p` holds a value already.
    pub fn put(&mut self, p: &str, kind: &str, v: &str) {
        if !self.values.contains_key(p) {
            self.make_room();
            self.values.insert(p.to_owned(), v.to_owned());
            self.kinds.insert(kind.to_owned());
            self.add_known(p, kind, v);
        }
        self.clock += 1;
        self.used.insert(p.to_owned(), self.clock);
    }

    /// Lets the half of the values used longest ago go when they are full.
    fn make_room(&mut self) {
        if self.values.len() < MAX_VALUES {
            return;
        }
        let mut ps: Vec<(u64, String)> = self
            .values
            .keys()
            .map(|p| (self.used.get(p).copied().unwrap_or(0), p.clone()))
            .collect();
        ps.sort();
        for (_, p) in &ps[..ps.len() / 2] {
            self.values.remove(p);
            self.used.remove(p);
        }
        self.known.clear();
        self.known_bits.iter_mut().for_each(|b| *b = 0);
        let all: Vec<(String, String)> = self
            .values
            .iter()
            .map(|(p, v)| (p.clone(), v.clone()))
            .collect();
        for (p, v) in all {
            let kind = kind_of(&p).to_owned();
            self.add_known(&p, &kind, &v);
        }
    }

    /// Adds the value of placeholder `p` to what is known. The user's own
    /// words are left to the options, which match them anywhere already.
    fn add_known(&mut self, p: &str, kind: &str, v: &str) {
        if kind == crate::WORD_KIND || v.len() < MIN_KNOWN {
            return;
        }
        let head = v.as_bytes()[..MIN_KNOWN].to_vec();
        let list = self.known.entry(head).or_default();
        if let Some(k) = list.iter_mut().find(|k| k.v == v) {
            // one value under two kinds: the same one wins whichever came
            // first
            if p < k.p.as_str() {
                *k = KnownValue {
                    v: v.to_owned(),
                    p: p.to_owned(),
                    kind: kind.to_owned(),
                };
            }
            return;
        }
        list.push(KnownValue {
            v: v.to_owned(),
            p: p.to_owned(),
            kind: kind.to_owned(),
        });
        let g = gram(v.as_bytes(), 0);
        self.known_bits[(g / 64) as usize] |= 1 << (g % 64);
    }
}

/// The first four bytes at `i`, hashed to 20 bits.
pub(crate) fn gram(s: &[u8], i: usize) -> u32 {
    let g = u32::from_le_bytes([s[i], s[i + 1], s[i + 2], s[i + 3]]);
    g.wrapping_mul(2654435761) >> 12
}

/// The kind in a placeholder: SECRET of {{SECRET_abcdefgh}}.
pub(crate) fn kind_of(p: &str) -> &str {
    match p.rfind('_') {
        Some(i) if i > 2 => &p[2..i],
        _ => "",
    }
}

/// The placeholder `v` of `kind` goes as: `{{KIND_` and the first 8 of the
/// base32 of an HMAC-SHA256 of the kind and the value, in lower case, `}}`.
pub(crate) fn tag(key: &[u8], kind: &str, v: &str) -> String {
    let mut h = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes any key");
    h.update(kind.as_bytes());
    h.update(b"\0");
    h.update(v.as_bytes());
    let sum = h.finalize().into_bytes();
    format!("{{{{{kind}_{}}}}}", base32_head(&sum[..5]))
}

/// The 8 letters of base32 (RFC 4648's, in lower case) that 5 bytes are.
fn base32_head(b: &[u8]) -> String {
    const ABC: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let n = b.iter().fold(0u64, |n, &x| n << 8 | x as u64);
    (0..8)
        .map(|i| ABC[(n >> (35 - 5 * i) & 31) as usize] as char)
        .collect()
}

/// As much of `v` as may be kept where `v` itself may not: its first
/// character (the first few of a key's, which name its kind: sk-, ghp_)
/// and how many characters it has.
pub(crate) fn hint(kind: &str, v: &str) -> (String, usize) {
    let n = v.chars().count();
    if n < 2 {
        return (String::new(), n);
    }
    let mut k = 1;
    if !crate::known::personal(kind) && kind != crate::WORD_KIND && n >= 16 {
        // a key's prefix, up to where its random part starts
        k = 0;
        for c in v.chars() {
            if k >= 4 || !(c.is_ascii_alphabetic() || c == '-' || c == '_') {
                break;
            }
            k += 1;
            if c == '-' || c == '_' {
                break;
            }
        }
        k = k.max(1);
    }
    (v.chars().take(k).collect(), n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags() {
        let a = tag(b"k", "API_KEY", "sk-1");
        assert_eq!(a, tag(b"k", "API_KEY", "sk-1"));
        assert_ne!(a, tag(b"other", "API_KEY", "sk-1"));
        assert_ne!(a, tag(b"k", "SECRET", "sk-1"));
        assert!(
            a.starts_with("{{API_KEY_") && a.ends_with("}}") && a.len() == "{{API_KEY_}}".len() + 8
        );
        assert_eq!(base32_head(&[0, 0, 0, 0, 0]), "aaaaaaaa");
        assert_eq!(base32_head(&[0xff; 5]), "77777777");
        // RFC 4648: "fooba" is MZXW6YTB
        assert_eq!(base32_head(b"fooba"), "mzxw6ytb");
    }

    #[test]
    fn hints() {
        assert_eq!(
            hint("API_KEY", "sk-proj-abcdEFGH1234ijkl"),
            ("sk-".into(), 24)
        );
        assert_eq!(
            hint("API_KEY", "ghp_0123456789abcdefghij"),
            ("ghp_".into(), 24)
        );
        assert_eq!(hint("EMAIL", "jane.doe@acme.io"), ("j".into(), 16));
        assert_eq!(hint("SECRET", "hunter2"), ("h".into(), 7));
        assert_eq!(hint("TERM", "张三"), ("张".into(), 2));
        assert_eq!(hint("TERM", "x"), (String::new(), 1));
    }
}
