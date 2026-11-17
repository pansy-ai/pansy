//! What follows a secret's name and a colon or = is not always the secret.
//! The SECRET rules take a name with password, secret, token or api_key in
//! it, then : or =, then the value; a prompt and a session's code have that
//! shape where nothing is secret:
//!
//! - a place in a file: `internal/auth/token.go:L643-669` names lines of
//!   token.go, a file's name and not a field's;
//! - a path: `PRIVATE_KEY_PATH=~/.ssh/id_ed25519` names where a secret is
//!   kept, and the agent needs it as written to read it;
//! - code naming a secret: `access_token = get_access_token_v2`;
//! - a package's version: `"js-tokens": "^4.0.0-beta.1"`;
//! - the next field, when the one named has no value: `env -i UI_PASSWORD=
//!   UI_ADDR=127.0.0.1:39117`;
//! - a moment: `"access_token_expires_at": "2026-08-30T11:44:31+08:00"`.
//!
//! A secret written down still masks: `password=hunter2pass9`,
//! `API_KEY=AbC123dEf456GhI789`, `token: t0k3nValue2`.

use std::sync::LazyLock;

use regex::Regex;

use crate::re::compile;
use crate::secrets::{not_a_slug, not_code};
use crate::text::{ALNUM, is_in, word_byte};

/// Where a value the SECRET rules found can be a secret: not code, and not
/// a place in a file, a path or a name in code.
pub(crate) fn field_at(s: &str, a: usize, b: usize) -> bool {
    if !not_code(s, a, b) {
        return false;
    }
    let v = &s[a..b];
    if v.starts_with('=') {
        return false; // Go's := with no space after it: secretKey:=base64Encode(raw)
    }
    !file_field(s, a)
        && !line_ref(v)
        && !path_value(v)
        && !names_a_secret(v)
        && !skill_name(s, a, b)
        && !version_value(v)
        && !timestamp_value(v)
}

/// A release's version as a manifest or a lock file pins it: a range's
/// operator, numbers joined by dots, then a pre-release's or a build's
/// parts (-beta.1, -rc1, .post2, b1, +build.5) or a number alone.
static VERSION_RANGE: LazyLock<Regex> = LazyLock::new(|| {
    compile(
        r"^(?:[~^]|[<>=!~]=?|==)?[vV]?\d+(?:\.\d+)+(?:[-+._]?(?:(?i:alpha|beta|rc|pre|preview|post|dev|canary|next|nightly|snapshot|release|final|ga|stable|build|incubating)\d*|[abcmABCM]\d+|\d+))*$",
    )
});

/// Says `v` is a release's version, which no secret is.
pub(crate) fn version_value(v: &str) -> bool {
    VERSION_RANGE.is_match(v)
}

/// A moment as RFC 3339 writes one, whole or the head of one to the hour.
static STAMP: LazyLock<Regex> = LazyLock::new(|| {
    compile(
        r"^\d{4}-\d{1,2}-\d{1,2}[Tt]\d{1,2}(?::\d{1,2}){0,2}(?:\.\d+)?(?:[Zz]|[+-]\d{2}:?\d{2})?$",
    )
});

/// Says `v` is a moment's writing, which no secret is: an expiry, a
/// renewal, an update.
pub(crate) fn timestamp_value(v: &str) -> bool {
    STAMP.is_match(v)
}

static SKILL_SLUG: LazyLock<Regex> = LazyLock::new(|| compile(r"^[a-z0-9]+(?:-[a-z0-9]+)+$"));

/// Says `s[a..b]` is the second half of plugin:skill, as Claude Code names
/// a plugin's skill (design-tokens:generate-v2): words in lower case joined
/// by hyphens on both sides of a colon with no space around it.
fn skill_name(s: &str, a: usize, b: usize) -> bool {
    let bs = s.as_bytes();
    if a < 2 || bs[a - 1] != b':' || !SKILL_SLUG.is_match(&s[a..b]) || not_a_slug(&s[a..b]) {
        return false;
    }
    let mut i = a - 1;
    while i > 0 && is_in("abcdefghijklmnopqrstuvwxyz0123456789-", bs[i - 1]) {
        i -= 1;
    }
    i < a - 1
        && (i == 0 || !(word_byte(bs[i - 1]) || bs[i - 1] == b'.'))
        && SKILL_SLUG.is_match(&s[i..a - 1])
}

static LINE_REF: LazyLock<Regex> = LazyLock::new(|| compile(r"^L?\d+(?:[-–:]L?\d+)*[.,;:]?$"));

/// Says `v` is a line or a range of lines (L643, L643-669, 643:12), never a
/// secret.
fn line_ref(v: &str) -> bool {
    LINE_REF.is_match(v)
}

/// Says the name before the value at `s[a]` is a file's, or a path's last
/// part, not a field's: token.go:L643-669, src/secret_store.ts:…,
/// skills/secret-scanner:… .
fn file_field(s: &str, a: usize) -> bool {
    let b = s.as_bytes();
    let mut i = a;
    while i > 0 && is_in(" \t\"'\\", b[i - 1]) {
        i -= 1;
    }
    if i == 0 || b[i - 1] != b':' && b[i - 1] != b'=' {
        return false;
    }
    i -= 1;
    while i > 0 && is_in(" \t\"'\\", b[i - 1]) {
        i -= 1;
    }
    let end = i;
    while i > 0 && (b[i - 1].is_ascii_alphanumeric() || is_in("_.-", b[i - 1])) {
        i -= 1;
    }
    let name = &s[i..end];
    if name.is_empty() {
        return false;
    }
    if i > 0 && (b[i - 1] == b'/' || b[i - 1] == b'\\') {
        return true; // a path's last part
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 => SOURCE_EXT.contains(&name[dot + 1..].to_ascii_lowercase().as_str()),
        _ => false,
    }
}

/// The extensions of files an agent reads and names with a line after
/// them: code, docs and configs. Not .env, .txt, .key or .pem, whose
/// content is what a secret is kept in.
const SOURCE_EXT: &[&str] = &[
    "go", "ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs", "py", "pyi", "rb", "rs", "java",
    "kt", "kts", "swift", "c", "h", "cc", "cpp", "cxx", "hpp", "hh", "m", "mm", "cs", "fs", "php",
    "scala", "dart", "lua", "zig", "nim", "ex", "exs", "erl", "hs", "ml", "clj", "r", "jl", "sh",
    "bash", "zsh", "fish", "ps1", "vue", "svelte", "astro", "md", "mdx", "rst", "sql", "proto",
    "graphql", "gradle", "html", "css", "scss", "less", "json", "jsonc", "yaml", "yml", "toml",
    "xml", "tf", "ipynb",
];

/// The folders an absolute path starts at.
const PATH_ROOTS: &[&str] = &[
    "Users",
    "home",
    "tmp",
    "private",
    "var",
    "etc",
    "run",
    "opt",
    "usr",
    "mnt",
    "Volumes",
    "root",
    "srv",
    "data",
    "workspace",
    "workspaces",
    "app",
    "Applications",
    "Library",
    "System",
    "proc",
    "dev",
    "sys",
    "nix",
];

/// Says `v` is a file's or a folder's path: one from home (~/), from here
/// (./, ../), or from a root folder (/Users/…, /tmp/…), with no + or = in
/// it as base64 has.
pub(crate) fn path_value(v: &str) -> bool {
    if v.contains(['+', '=']) {
        return false;
    }
    for p in ["~/", "./", "../"] {
        if v.starts_with(p) && v.len() > p.len() {
            return true;
        }
    }
    let Some(rest) = v.strip_prefix('/') else {
        return false;
    };
    match rest.split_once('/') {
        Some((first, rest)) if !rest.is_empty() => PATH_ROOTS.contains(&first),
        _ => false,
    }
}

/// The words of a name in code that say it holds a secret.
const SECRET_WORDS: &[&str] = &[
    "token",
    "tokens",
    "secret",
    "secrets",
    "password",
    "passwords",
    "passwd",
    "pwd",
    "key",
    "keys",
    "apikey",
    "credential",
    "credentials",
];

static IDENT: LazyLock<Regex> =
    LazyLock::new(|| compile(r"^[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)*$"));

/// Says `v` is a name in code for a secret, not one: two words or more,
/// one of them token, secret, password or key, written as code writes a
/// name. refreshToken2 and env.SECRET_V2 are names; Passw0rd123 is one
/// word, and SuperSecret2024 a password.
fn names_a_secret(v: &str) -> bool {
    if !IDENT.is_match(v) {
        return false;
    }
    if !v.as_bytes()[0].is_ascii_lowercase() && !v.contains(['_', '.']) {
        return false;
    }
    let (mut words, mut named) = (0, false);
    for w in ident_words(v) {
        if !w.bytes().any(|c| c.is_ascii_alphabetic()) {
            continue;
        }
        words += 1;
        let w = match w.find(|c: char| c.is_ascii_digit()) {
            Some(i) => &w[..i], // countTokens4o: Tokens
            None => w,
        };
        if SECRET_WORDS.contains(&w.to_ascii_lowercase().as_str()) {
            named = true;
        }
    }
    words >= 2 && named
}

/// The words of `v`: split at _ and ., and where a capital starts a word in
/// camelCase (refreshToken2 is refresh and Token2).
pub(crate) fn ident_words(v: &str) -> Vec<&str> {
    let mut out = Vec::new();
    for part in v.split(['_', '.']).filter(|p| !p.is_empty()) {
        let b = part.as_bytes();
        let mut start = 0;
        for i in 1..b.len() {
            if b[i].is_ascii_uppercase() && b[i - 1].is_ascii_lowercase() {
                out.push(&part[start..i]);
                start = i;
            }
        }
        out.push(&part[start..]);
    }
    out
}

/// A field being set at a value's start, as a command line, a logfmt line
/// or a flag sets one, then = and something other than another =.
static NEXT_FIELD: LazyLock<Regex> = LazyLock::new(|| {
    compile(r"^(?:[A-Z_][A-Z0-9_]*|[a-z_][a-z0-9_-]*|--?[A-Za-z][A-Za-z0-9_-]*)=[^=]")
});

/// Says the value at `s[a..b]` is not the value of the field before it but
/// the next field: the name before it was set to nothing.
pub(crate) fn next_field(s: &str, a: usize, b: usize) -> bool {
    a > 0 && matches!(s.as_bytes()[a - 1], b' ' | b'\t') && NEXT_FIELD.is_match(&s[a..b])
}

/// Says `c` may be in a field's name.
pub(crate) fn name_byte(c: u8) -> bool {
    is_in(ALNUM, c) || is_in("_.-", c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_that_are_no_secret() {
        for v in [
            "^4.0.0-beta.1",
            "9.2.0-beta1",
            "2.4.0-beta02",
            "v1.2.3",
            "1.0.0.RELEASE",
        ] {
            assert!(version_value(v), "{v}");
        }
        assert!(!version_value("3.14159abc"));
        assert!(
            timestamp_value("2026-08-30T11:44:31.312645+08:00") && timestamp_value("2026-08-30T11")
        );
        assert!(line_ref("L643-669.") && line_ref("643:12") && !line_ref("L643x"));
        assert!(
            path_value("~/.ssh/id_ed25519")
                && path_value("/Users/me/x")
                && !path_value("/x")
                && !path_value("/Users/a+b")
        );
        assert!(
            names_a_secret("refreshToken2")
                && names_a_secret("env.SECRET_V2")
                && names_a_secret("get_access_token_v2")
        );
        assert!(
            !names_a_secret("Passw0rd123")
                && !names_a_secret("SuperSecret2024")
                && !names_a_secret("password123")
        );
        assert_eq!(ident_words("refreshToken2"), ["refresh", "Token2"]);
    }
}
