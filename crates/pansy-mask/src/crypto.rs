//! Crypto wallet secrets, masked with the other secrets: the private keys
//! wallets and chain tools export, in the shapes they export them in.
//!
//! A shape alone is no key where a hash or a signature has it too, so a
//! shape with no prefix of its own is taken only when the key proves
//! itself: Bitcoin's WIF and BIP-0032's xprv carry a base58check checksum,
//! a Stellar seed a CRC16, and a Solana key is the 32-byte seed followed by
//! its own public key. A shape with a prefix no other value has
//! (suiprivkey1, nsec1, AGE-SECRET-KEY-1) is taken by that prefix.
//!
//! An EVM key is 0x and 64 hex digits, which is also a transaction hash,
//! so it is taken only after a name that says it is a key: PRIVATE_KEY,
//! --private-key, signer, deployer, 私钥. A seed phrase is taken after a
//! word that says what it is, and only when each of its 12 to 25 words is
//! in BIP-0039's English list.

use std::collections::HashSet;
use std::sync::LazyLock;

use sha2::{Digest, Sha256};

use crate::finder::Finder;
use crate::text::{ALNUM, ceil, is_letter, lower_ascii};

pub(crate) fn rules() -> Vec<Finder> {
    vec![
        // Ethereum and the chains like it: 0x and the 32 bytes in hex, or
        // the 64 digits alone, after a key's name
        Finder::new("WALLET_KEY", r"(?i)0x[0-9a-f]{56,64}|[0-9a-f]{64}")
            .fold(WALLET_KEY_CUES)
            .window(64, 160)
            .bound(ALNUM)
            .at(wallet_key_at),
        // WIF: a run of 51 or 52 base58 characters whose checksum adds up
        Finder::new("WIF_KEY", r"[1-9A-HJ-NP-Za-km-z]{51,}").bound(ALNUM).ok(wif_key).may(|s| b58_run(s, 51)),
        // a BIP32 extended private key: xprv, yprv, zprv, tprv and the
        // multisig ones; 78 bytes and a checksum. An xpub goes as written.
        Finder::new("BIP32_KEY", r"[xyztuvYZUV]prv[1-9A-HJ-NP-Za-km-z]{107}").markers(&["prv"]).bound(ALNUM).ok(bip32_key),
        // Solana, as Phantom and Solflare export it: the 64-byte secret key
        // in base58, its second half the first half's public key
        Finder::new("SOLANA_KEY", r"[1-9A-HJ-NP-Za-km-z]{85,}").bound(ALNUM).ok(solana_key).may(|s| b58_run(s, 85)),
        // Solana's own keyfile: the same 64 bytes as 64 numbers
        Finder::new("SOLANA_KEYPAIR", r"\[\s*\d{1,3}(?:\s*,\s*\d{1,3}){63}\s*\]").markers(&["["]).ok(solana_keypair),
        // NEAR: ed25519: and the 64-byte secret key in base58
        Finder::new("NEAR_KEY", r"ed25519:[1-9A-HJ-NP-Za-km-z]{86,88}").markers(&["ed25519:"]).bound(ALNUM),
        Finder::new("SUI_KEY", r"suiprivkey1[0-9a-z]{58}").markers(&["suiprivkey1"]).bound(ALNUM),
        // Aptos, in AIP-80's form
        Finder::new("APTOS_KEY", r"(?:ed25519|secp256k1)-priv-0x[0-9a-fA-F]{64}").markers(&["-priv-0x"]).bound(ALNUM),
        // Stellar's secret seed: S and 55 base32 characters, version byte
        // and CRC16 checked
        Finder::new("STELLAR_KEY", r"S[A-Z2-7]{55}").bound(ALNUM).ok(stellar_seed).may(|s| base32_run(s, 56)),
        Finder::new("NOSTR_KEY", r"nsec1[02-9ac-hj-np-z]{58}").markers(&["nsec1"]).bound(ALNUM),
        Finder::new("AGE_KEY", r"AGE-SECRET-KEY-1[0-9A-Z]{58}").markers(&["AGE-SECRET-KEY-"]).bound(ALNUM),
        Finder::new("ZCASH_KEY", r"secret-extended-key-(?:main|test)1[0-9a-z]{150,320}")
            .markers(&["secret-extended-key-"])
            .bound(ALNUM),
        // a seed phrase after the words that say what it is; the fix keeps
        // the longest run of BIP-0039 words, if it is long enough
        Finder::new(
            "SEED_PHRASE",
            r"(?i)(?:seed|mnemonic|phrase|recovery|助记词|助記詞|シード|ニーモニック|시드|니모닉)[^.\n]{0,24}?(?:\n[ \t]*)?\b([a-z]{3,8}(?:[ \t]+[a-z]{3,8}){11,40})\b",
        )
        .fold(SEED_CUES)
        .window(32, 400)
        .fix(seed_phrase_at),
    ]
}

/// What a field or a flag holding a wallet's private key has in its name,
/// in lower case.
const WALLET_KEY_CUES: &[&str] = &[
    "priv",
    "secret",
    "signer",
    "signing",
    "deployer",
    "wallet",
    "pk",
    "私钥",
    "私鑰",
    "秘密鍵",
    "개인키",
    "비밀키",
];

/// Between a cue and the hex after it, these say the hex is something else
/// of the key's: privateKeyToPublic(0x…), wallet tx hash 0x… .
const NOT_KEY_WORDS: &[&str] = &[
    "hash",
    "sha",
    "digest",
    "sum",
    "pub",
    "addr",
    "tx",
    "fingerprint",
    "id",
];

/// Says the hex at `s[a..b]` comes after a wallet key's name, on its own
/// line and at most 48 bytes before it, with nothing between that says it
/// is a hash, an address or a public key.
fn wallet_key_at(s: &str, a: usize, _b: usize) -> bool {
    let mut start = ceil(s, a.saturating_sub(48));
    if let Some(nl) = s.as_bytes()[start..a].iter().rposition(|&c| c == b'\n') {
        start += nl + 1;
    }
    let before = lower_ascii(&s[start..a]);
    let bb = before.as_bytes();
    let (mut cue, mut end): (Option<usize>, usize) = (None, 0);
    for c in WALLET_KEY_CUES {
        let mut i = 0;
        while let Some(j) = before[i..].find(c) {
            let at = i + j;
            i = at + c.len();
            // pk only as a word of a name: PK, DEPLOYER_PK, pk=, not pkg
            if *c == "pk" && (at > 0 && is_letter(bb[at - 1]) || i < bb.len() && is_letter(bb[i])) {
                continue;
            }
            if cue.is_none_or(|q| at > q) {
                (cue, end) = (Some(at), i);
            }
        }
    }
    if cue.is_none() {
        return false;
    }
    let between = &before[end..];
    for w in NOT_KEY_WORDS {
        if *w == "id" {
            // an id's name: wallet_id, walletId, wallet id
            let t = between.trim_start_matches(['_', '-', ' ']);
            if t.starts_with("id") && (t.len() == 2 || !is_letter(t.as_bytes()[2])) {
                return false;
            }
            continue;
        }
        if between.contains(w) {
            return false;
        }
    }
    true
}

const B58: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

fn b58_digit(c: u8) -> Option<u32> {
    B58.iter().position(|&x| x == c).map(|i| i as u32)
}

/// `s` decoded from base58, or None where it has a character outside it.
fn base58(s: &str) -> Option<Vec<u8>> {
    let mut out: Vec<u8> = Vec::new(); // little-endian
    for c in s.bytes() {
        let mut carry = b58_digit(c)?;
        for x in out.iter_mut() {
            carry += *x as u32 * 58;
            *x = carry as u8;
            carry >>= 8;
        }
        while carry > 0 {
            out.push(carry as u8);
            carry >>= 8;
        }
    }
    let zeros = s.bytes().take_while(|&c| c == b'1').count();
    out.extend(std::iter::repeat_n(0, zeros));
    out.reverse();
    Some(out)
}

/// The payload of `s`, when it is base58 and its last four bytes are the
/// double sha256 of the rest.
fn base58check(s: &str) -> Option<Vec<u8>> {
    let b = base58(s)?;
    if b.len() < 5 {
        return None;
    }
    let (p, sum) = b.split_at(b.len() - 4);
    let h = Sha256::digest(Sha256::digest(p));
    (h[..4] == *sum).then(|| p.to_vec())
}

/// A private key in WIF: a version byte and 32 bytes, and 0x01 after them
/// for a compressed key, with its checksum.
fn wif_key(v: &str) -> bool {
    if v.len() != 51 && v.len() != 52 {
        return false;
    }
    base58check(v).is_some_and(|p| p.len() == 33 || p.len() == 34 && p[33] == 1)
}

/// A BIP-0032 extended key, 78 bytes with its checksum.
fn bip32_key(v: &str) -> bool {
    base58check(v).is_some_and(|p| p.len() == 78)
}

/// A 64-byte ed25519 secret key in base58: a seed and the public key it
/// derives.
fn solana_key(v: &str) -> bool {
    (85..=88).contains(&v.len()) && base58(v).is_some_and(|b| seed_and_public(&b))
}

/// Says `b` is 32 bytes of seed and the ed25519 public key it derives.
fn seed_and_public(b: &[u8]) -> bool {
    b.len() == 64 && crate::ed25519::public_key(&b[..32]) == b[32..]
}

/// A Solana keyfile: 64 numbers, each a byte, together a seed and its
/// public key.
fn solana_keypair(v: &str) -> bool {
    let parts: Vec<&str> = v
        .trim_matches(['[', ']', ' ', '\t', '\r', '\n'])
        .split(',')
        .collect();
    if parts.len() != 64 {
        return false;
    }
    let mut b = Vec::with_capacity(64);
    for p in parts {
        match p.trim().parse::<u8>() {
            Ok(n) => b.push(n),
            Err(_) => return false,
        }
    }
    seed_and_public(&b)
}

/// A Stellar secret seed in StrKey: the seed's version byte (18 << 3), 32
/// bytes and a CRC16-XModem, little-endian.
fn stellar_seed(v: &str) -> bool {
    let Some(b) = base32_decode(v) else {
        return false;
    };
    if b.len() != 35 || b[0] != 18 << 3 {
        return false;
    }
    let c = crc16_xmodem(&b[..33]);
    b[33] == c as u8 && b[34] == (c >> 8) as u8
}

/// RFC 4648 base32, in capitals and without padding, decoded.
fn base32_decode(v: &str) -> Option<Vec<u8>> {
    if !v.len().is_multiple_of(8) {
        return None;
    }
    let mut out = Vec::with_capacity(v.len() * 5 / 8);
    let (mut acc, mut bits) = (0u64, 0);
    for c in v.bytes() {
        let d = match c {
            b'A'..=b'Z' => c - b'A',
            b'2'..=b'7' => c - b'2' + 26,
            _ => return None,
        };
        acc = acc << 5 | d as u64;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

fn crc16_xmodem(b: &[u8]) -> u16 {
    let mut c: u16 = 0;
    for &x in b {
        c ^= (x as u16) << 8;
        for _ in 0..8 {
            c = if c & 0x8000 != 0 {
                c << 1 ^ 0x1021
            } else {
                c << 1
            };
        }
    }
    c
}

/// What a seed phrase is called.
const SEED_CUES: &[&str] = &[
    "seed",
    "mnemonic",
    "phrase",
    "recovery",
    "助记词",
    "助記詞",
    "シード",
    "ニーモニック",
    "시드",
    "니모닉",
];

/// BIP-0039's English wordlist, which Electrum and Algorand use too.
static BIP39: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    include_str!("bip39english.txt")
        .split_whitespace()
        .collect()
});

/// How many words a seed phrase has: BIP-0039's 12 to 24, Algorand's 25.
const PHRASE_LENGTHS: &[usize] = &[25, 24, 21, 18, 15, 12];

/// The seed phrase in the words at `s[a..b]`: the longest run of BIP-0039
/// words there, cut to a phrase's length.
fn seed_phrase_at(s: &str, a: usize, b: usize) -> Option<(usize, usize)> {
    let bs = s.as_bytes();
    let (mut best_a, mut best_n, mut best_end) = (0, 0, 0);
    let (mut run_a, mut run_n): (Option<usize>, usize) = (None, 0);
    let mut ends: Vec<usize> = Vec::new();
    let mut i = a;
    while i < b {
        while i < b && matches!(bs[i], b' ' | b'\t') {
            i += 1;
        }
        let mut j = i;
        while j < b && !matches!(bs[j], b' ' | b'\t') {
            j += 1;
        }
        if i == j {
            break;
        }
        if BIP39.contains(s[i..j].to_lowercase().as_str()) {
            if run_a.is_none() {
                (run_a, run_n) = (Some(i), 0);
                ends.clear();
            }
            run_n += 1;
            ends.push(j);
            if run_n > best_n {
                (best_a, best_n, best_end) = (run_a.unwrap_or(i), run_n, j);
                if let Some(l) = PHRASE_LENGTHS.iter().find(|&&l| run_n >= l) {
                    best_end = ends[l - 1];
                }
            }
        } else {
            run_a = None;
        }
        i = j;
    }
    (best_n >= 12).then_some((best_a, best_end))
}

/// Says `s` has `n` base58 letters in a row.
fn b58_run(s: &str, n: usize) -> bool {
    let mut run = 0;
    for c in s.bytes() {
        if B58.contains(&c) {
            run += 1;
            if run >= n {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

/// Says `s` has `n` letters of base32 in capitals in a row.
fn base32_run(s: &str, n: usize) -> bool {
    let mut run = 0;
    for c in s.bytes() {
        if c.is_ascii_uppercase() || (b'2'..=b'7').contains(&c) {
            run += 1;
            if run >= n {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodings() {
        assert_eq!(base58("1112").unwrap(), [0, 0, 0, 1]);
        assert_eq!(base58("5R").unwrap(), [0x01, 0x00]);
        assert!(base58("0OIl").is_none());
        assert_eq!(
            base32_decode("MZXW6YTBOI======".trim_end_matches('=')),
            None
        );
        assert_eq!(base32_decode("MZXW6YTB").unwrap(), b"fooba");
        assert_eq!(crc16_xmodem(b"123456789"), 0x31c3);
        assert_eq!(BIP39.len(), 2048);
    }
}
