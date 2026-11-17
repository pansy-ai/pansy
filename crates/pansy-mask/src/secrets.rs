//! Secrets: private keys, the API keys whose prefixes say whose they are,
//! JWTs, a URL's password, a value assigned to a secret's name (password =
//! …, API_KEY: …, 密码：…), an Authorization or Cookie header.

use std::sync::LazyLock;

use regex::Regex;

use crate::fieldvalue::{field_at, path_value, timestamp_value, version_value};
use crate::finder::Finder;
use crate::re::compile;
use crate::text::{ALNUM, TOKEN_CH, all_digits, is_in, trim, trim_left, trim_right};

/// A field's or a value's quote, if it has one: " or ', or \" in JSON
/// written inside a string.
const QUOTE: &str = r#"(?:\\?["'])?"#;
/// The characters of a secret's value.
const SECRET_CH: &str = r"[A-Za-z0-9_\-./+=~!@#%^&*]";

/// What a field holding a secret has in its name, as a pattern and as
/// text any match of it has in it: DB_PASSWORD, apiKey, client_secret.
const SECRET_NAMES: &[(&str, &str)] = &[
    ("password", "pass"),
    ("passwd", "pass"),
    // an exchange's API passphrase (OKX, Coinbase, KuCoin)
    ("passphrase", "pass"),
    ("secret", "secret"),
    ("token", "token"),
    ("api[_-]?key", "key"),
    ("access[_-]?key", "key"),
    ("private[_-]?key", "key"),
    ("priv[_-]?key", "key"),
    ("credential", "credential"),
];

fn names_pattern(names: &[(&str, &str)]) -> String {
    names.iter().map(|n| n.0).collect::<Vec<_>>().join("|")
}

/// Each name's marker as a name is written in lower case, in upper case
/// and capitalised: password, PASSWORD, Password.
fn name_markers(names: &[(&str, &str)]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (_, m) in names {
        let cap = m[..1].to_uppercase() + &m[1..];
        for x in [m.to_string(), m.to_uppercase(), cap] {
            if !out.contains(&x) {
                out.push(x);
            }
        }
    }
    out
}

/// What a password is called in Chinese, Japanese and Korean.
const CJK_PASSWORD_NAMES: &[&str] = &[
    "密码",
    "密碼",
    "口令",
    "PIN码",
    "PIN碼",
    "PIN 码",
    "安全码",
    "安全碼",
    "パスワード",
    "暗証番号",
    "비밀번호",
    "암호",
    "비번",
];

/// What a key, a secret or a token is called in Chinese, Japanese and
/// Korean.
const CJK_KEY_NAMES: &[&str] = &[
    "密钥",
    "密鑰",
    "秘钥",
    "秘鑰",
    "私钥",
    "私鑰",
    "金钥",
    "金鑰",
    "令牌",
    "权杖",
    "權杖",
    "授权码",
    "授權碼",
    "秘密鍵",
    "トークン",
    "シークレット",
    "APIキー",
    "アクセスキー",
    "토큰",
    "비밀키",
    "API 키",
    "API키",
];

/// What stands between a name and its value in a Chinese message.
const CJK_COPULAS: &[&str] = &["为", "為", "是", "："];

/// A value after one of `names`: after ：, :, =, 为, 為, 是, は or 는,
/// quoted or not, or after nothing but a space.
fn cjk_named(names: &[&str]) -> String {
    format!(
        r#"(?i)(?:{})[ \t]*(?:[:：=][ \t]*|[为為是][ \t]*[:：]?[ \t]*|は[ \t]*|는[ \t]*)?(?:\\?["'“「])?({SECRET_CH}{{3,63}}[A-Za-z0-9_\-/+=~!@#%^&*])"#,
        names.join("|")
    )
}

pub(crate) fn rules() -> Vec<Finder> {
    let secret_names = names_pattern(SECRET_NAMES);
    let mut assigned = Finder::new(
        "SECRET",
        format!(r"(?i)[A-Za-z0-9_.-]*(?:{secret_names})[A-Za-z0-9_.-]*{QUOTE}[ \t]*[:=][ \t]*{QUOTE}({SECRET_CH}{{8,}})"),
    )
    .ok(secret_value)
    .at(field_at)
    .field();
    assigned.markers = name_markers(SECRET_NAMES);
    // an env var's name says what it holds in the upper case: a
    // SERVICE_X_KEY, a DB_PASS. In the lower case a name that ends in key
    // is code's: a sort key, a cache key, a public key.
    let mut env_named = Finder::new(
        "SECRET",
        format!(r"(?:^|[^A-Za-z0-9_.-])[A-Za-z0-9_.-]*_(?:KEY|PASS){QUOTE}[ \t]*[:=][ \t]*{QUOTE}({SECRET_CH}{{8,}})"),
    )
    .ok(secret_value)
    .at(named_secret_at)
    .field();
    env_named.markers = name_markers(&[("key", "key"), ("pass", "pass")]);
    // an Authorization or Cookie header's value is a secret whatever it
    // looks like: an opaque bearer token, a session value
    let mut header = Finder::new(
        "SECRET",
        r"(?im)^[ \t]*(?:authorization|cookie)[ \t]*:[ \t]*(\S[^\r\n]{7,})",
    )
    .ok(header_value);
    header.markers = name_markers(&[("authorization", "authorization"), ("cookie", "cookie")]);
    // a field called key and nothing more: "key": "oc_sk_…". Only a long
    // value of letters and digits both, since code calls anything a key
    let mut bare_key = Finder::new(
        "SECRET",
        format!(
            r"(?i)(?:^|[^A-Za-z0-9_.-]){QUOTE}key{QUOTE}[ \t]*[:=][ \t]*{QUOTE}({SECRET_CH}{{16,}})"
        ),
    )
    .ok(secret_value)
    .at(field_at)
    .field();
    bare_key.markers = name_markers(&[("key", "key")]);

    vec![
        Finder::new(
            "PRIVATE_KEY",
            r"-----BEGIN (?:[A-Z0-9]+ )*PRIVATE KEY(?: BLOCK)?-----[\s\S]+?-----END (?:[A-Z0-9]+ )*PRIVATE KEY(?: BLOCK)?-----",
        )
        .markers(&["PRIVATE KEY"]),
        Finder::new("API_KEY", r"sk-(?:ant-|proj-|or-|svcacct-|admin-)?[A-Za-z0-9_-]{20,}")
            .markers(&["sk-"])
            .bound(TOKEN_CH)
            .ok(not_a_slug),
        // a gateway's or a relay's own: oc_sk_…, or_sk_…
        Finder::new("API_KEY", r"[a-z]{2,4}_sk_[A-Za-z0-9_-]{20,}")
            .markers(&["_sk_"])
            .bound(TOKEN_CH)
            .ok(secret_value),
        // Volcano Ark and the relays in front of it
        Finder::new("API_KEY", r"ark-[A-Za-z0-9][A-Za-z0-9_-]{15,}")
            .markers(&["ark-"])
            .bound(TOKEN_CH)
            .ok(secret_value),
        Finder::new("API_KEY", r"(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{40,}|glpat-[A-Za-z0-9_-]{20,})")
            .markers(&["gh", "github_pat_", "glpat-"])
            .bound(TOKEN_CH),
        Finder::new("API_KEY", r"AIza[0-9A-Za-z_-]{35}").markers(&["AIza"]).bound(TOKEN_CH),
        Finder::new("API_KEY", r"xox[abposr]-[0-9A-Za-z-]{10,}").markers(&["xox"]).bound(TOKEN_CH),
        Finder::new("API_KEY", r"(?:sk|rk)_live_[0-9A-Za-z]{20,}").markers(&["_live_"]).bound(TOKEN_CH),
        Finder::new("API_KEY", r"(?:AKIA|ASIA)[0-9A-Z]{16}").markers(&["AKIA", "ASIA"]).bound(ALNUM),
        Finder::new(
            "API_KEY",
            r"(?:hf_[A-Za-z0-9]{30,}|gsk_[A-Za-z0-9]{40,}|xai-[A-Za-z0-9]{40,}|npm_[A-Za-z0-9]{36}|pypi-[A-Za-z0-9_-]{50,}|dop_v1_[a-f0-9]{64}|SG\.[A-Za-z0-9_-]{22}\.[A-Za-z0-9_-]{43})",
        )
        .markers(&["hf_", "gsk_", "xai-", "npm_", "pypi-", "dop_v1_", "SG."])
        .bound(TOKEN_CH),
        // Cloudflare's scannable prefixes (cfut_, cfat_, cfk_, cfoat_), an
        // Origin CA key (v1.0-, two runs), a tunnel token (base64 of
        // {"a":account,"t":tunnel,"s":secret}, so it starts eyJhIjoi)
        Finder::new("API_KEY", r"cf(?:ut|at|k|oat)_[A-Za-z0-9_-]{40,}")
            .markers(&["cfut_", "cfat_", "cfk_", "cfoat_"])
            .bound(TOKEN_CH),
        Finder::new("API_KEY", r"v1\.0-[A-Za-z0-9]{16,}-[A-Za-z0-9]{40,}").markers(&["v1.0-"]).bound(TOKEN_CH),
        Finder::new("TOKEN", r"eyJhIjoi[A-Za-z0-9+/_-]{40,}={0,2}").markers(&["eyJhIjoi"]).bound(TOKEN_CH),
        // X-Auth-Key and X-Auth-User-Service-Key headers, whatever the key
        // looks like
        Finder::new("SECRET", r#"(?i)x-auth-(?:user-service-)?key["']?[ \t]*[:=][ \t]*["']?([A-Za-z0-9._-]{16,})"#)
            .fold(&["x-auth-"])
            .window(0, 120)
            .ok(not_a_variable),
        Finder::new("TOKEN", r"eyJ[A-Za-z0-9_-]{8,}\.eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}")
            .markers(&["eyJ"])
            .bound(TOKEN_CH),
        // the password of user:password@host, or of :password@host as a
        // Redis URL has it with no user
        Finder::new("PASSWORD", r#"[A-Za-z][A-Za-z0-9+.-]*://[^\s:@/?#"'<>]*:([^\s@/?#"'<>]{3,})@"#)
            .markers(&["://"])
            .ok(not_a_variable)
            .at(hidden_in_url),
        // password = …, API_KEY: "…", as .env files and configs have them
        assigned,
        env_named,
        header,
        bare_key,
        // a password after its name in Chinese, Japanese or Korean, as
        // people write one into a message: 密码：786kih. Nothing in code is
        // called 密码, so any value of four or more is taken
        Finder::new("PASSWORD", cjk_named(CJK_PASSWORD_NAMES))
            .fold(CJK_PASSWORD_NAMES)
            .window(64, 100)
            .ok(not_a_variable)
            .field(),
        // a key, a secret or a token after its name the same way: 密钥为 …
        Finder::new("SECRET", cjk_named(CJK_KEY_NAMES))
            .fold(CJK_KEY_NAMES)
            .window(64, 100)
            .at(key_at)
            .field(),
        // an English name for one before 为, 為, 是 or ：, as a Chinese
        // message has it: apikey 为 …, token 是 …
        Finder::new(
            "SECRET",
            format!(
                r#"(?i)(?:{secret_names}|pwd|api[ ]key|access[ ]key|secret[ ]key|key|\bpin(?:[ ]?code)?){QUOTE}[ \t]*(?:[为為是][ \t]*[:：]?|：)[ \t]*(?:\\?["'“「])?({SECRET_CH}{{3,}}[A-Za-z0-9_\-/+=~!@#%^&*])"#
            ),
        )
        .fold(CJK_COPULAS)
        .window(24, 260)
        .at(key_at)
        .field(),
    ]
}

/// Turns away what stands in for a value: ${PASS}, <password>, ****.
pub(crate) fn not_a_variable(v: &str) -> bool {
    match v.as_bytes().first() {
        None => false,
        Some(c) if is_in("$%{<[*", *c) => false,
        _ => !trim(v, "*xX.").is_empty(),
    }
}

/// A key's or a token's value, not a word or a number: letters and digits
/// both, or 16 characters or more that aren't all digits.
fn key_value(v: &str) -> bool {
    if !not_a_variable(v) {
        return false;
    }
    let letter = v.bytes().any(|c| c.is_ascii_alphabetic());
    letter && (v.bytes().any(|c| c.is_ascii_digit()) || v.len() >= 16)
}

/// What stands between a name and a key's value.
const KEY_SEPARATORS: &[&str] = &["为", "為", "是", "：", ":", "=", "は", "는"];

/// What a number counts when one follows it: bits, bytes, a time, a count.
const QUANTITY_UNITS: &[&str] = &[
    "位",
    "比特",
    "字节",
    "字節",
    "个",
    "個",
    "次",
    "秒",
    "毫秒",
    "分钟",
    "分鐘",
    "小时",
    "小時",
    "天",
    "年",
    "bit",
    "byte",
    "ms",
    "sec",
    "min",
    "ビット",
    "バイト",
    "비트",
    "바이트",
];

/// key_value for `s[a..b]`, or six digits or more right after 为, 是, ：,
/// :, = and the like that count nothing: 密钥为12231241 is a key, while 密钥为
/// 2048 位 is a size.
pub(crate) fn key_at(s: &str, a: usize, b: usize) -> bool {
    let v = &s[a..b];
    if path_value(v) || version_value(v) || timestamp_value(v) {
        return false;
    }
    if key_value(v) {
        return true;
    }
    if v.len() < 6 || !all_digits(v) {
        return false;
    }
    let before = trim_right(&s[..a], " \t\"'“「\\");
    let after = trim_left(&s[b..], " \t");
    KEY_SEPARATORS.iter().any(|c| before.ends_with(c))
        && !QUANTITY_UNITS.iter().any(|u| after.starts_with(u))
}

/// A value that looks like a secret, not code: letters and digits both,
/// and not a call, a reference or a placeholder of its own.
pub(crate) fn secret_value(v: &str) -> bool {
    if !not_a_variable(v) || v.starts_with("process.env") || v.starts_with("os.") {
        return false;
    }
    v.bytes().any(|c| c.is_ascii_digit()) && v.bytes().any(|c| c.is_ascii_alphabetic())
}

/// A name in code, or a field read through one: Uint8Array, details.2.
static IDENT_PATH: LazyLock<Regex> =
    LazyLock::new(|| compile(r"^[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z0-9_]+)*$"));

/// Turns away a value at `s[a..b]` that is code naming a secret, not the
/// secret: a call (privateKey: toBase64Url(…)), or, unquoted after a colon,
/// a type or a read code goes on from (secretKey: Uint8Array;). A secret
/// written in code is a string, so in quotes; one after = or at a line's
/// end (.env files, logs, YAML) is kept.
pub(crate) fn not_code(s: &str, a: usize, b: usize) -> bool {
    if !IDENT_PATH.is_match(&s[a..b]) {
        return true;
    }
    let rest = trim_left(&s[b..], " \t");
    if rest.starts_with('(') {
        return false;
    }
    let bs = s.as_bytes();
    if a > 0 && (bs[a - 1] == b'"' || bs[a - 1] == b'\'')
        || !trim_right(&s[..a], " \t").ends_with(':')
    {
        return true;
    }
    rest.is_empty() || !is_in(";,)}]|?&<>", rest.as_bytes()[0])
}

/// Says the password at `s[a..b]`, in a URL's user:password@, is hidden by
/// masking it: it is not also written as it is elsewhere in the same URL,
/// as its user name, a part of its host or of its path. A test database's
/// URL often gives one word for all three (postgres:postgres@…/postgres),
/// and anyone who reads the user reads the password.
fn hidden_in_url(s: &str, a: usize, b: usize) -> bool {
    let pw = &s[a..b];
    let bs = s.as_bytes();
    let Some(i) = s[..a].rfind("//") else {
        return true;
    };
    if bs[a - 1] != b':' || i + 2 > a - 1 {
        return true;
    }
    if &s[i + 2..a - 1] == pw {
        return false; // the user name
    }
    // the host and the path, to the URL's end
    let mut end = b + 1;
    while end < s.len() && !is_in(" \t\r\n\"'<>`\\)]}", bs[end]) {
        end += 1;
    }
    if b + 1 > end || end > s.len() {
        return true;
    }
    !s[b + 1..end]
        .split(|c| "/:.?#&=;,@".contains(c))
        .any(|part| !part.is_empty() && part == pw)
}

/// False for words in lower case joined by hyphens, as a page's address
/// names an article (sk-hynix-third-quarter-profit). A key's characters are
/// random: it has a capital, or a run of 16 or more with no hyphen, or hex
/// digits alone, as a UUID's groups are.
pub(crate) fn not_a_slug(v: &str) -> bool {
    if v.bytes().any(|c| c.is_ascii_uppercase()) {
        return true;
    }
    let t = v.strip_prefix("sk-").unwrap_or(v);
    if !t
        .chars()
        .any(|r| r != '-' && r != '_' && !r.is_ascii_digit() && !('a'..='f').contains(&r))
    {
        return true;
    }
    v.split(['-', '_']).any(|w| w.len() >= 16)
}

/// Says the rest of an Authorization or Cookie line is a header's value,
/// not prose that begins with the word ("Authorization: the user must sign
/// in first"): a credential, after a scheme word or alone (Bearer …), or a
/// cookie's name=value pairs. One that stands for a variable is left.
fn header_value(v: &str) -> bool {
    let v = v.trim();
    if v.is_empty() || !not_a_variable(v) {
        return false;
    }
    let not_letter = |c: char| !c.is_ascii_alphabetic();
    let f: Vec<&str> = v.split_whitespace().collect();
    if f.len() == 1 || f.len() == 2 && !f[0].chars().any(not_letter) {
        // a word alone is prose's: a credential has a digit or a sign in it
        let cred = f[f.len() - 1];
        return cred.len() >= 8 && not_a_variable(cred) && cred.chars().any(not_letter);
    }
    for kv in v.split(';') {
        let kv = kv.trim();
        if kv.is_empty() {
            continue;
        }
        match kv.split_once('=') {
            Some((name, _)) if !name.is_empty() && !name.contains([' ', '\t']) => {}
            _ => return false,
        }
    }
    v.contains('=')
}

/// Turns away the upper-case name before the value at `s[..a]` when it
/// only ends in key and holds no secret: a sort key, a cache key, a public
/// key.
fn not_a_plain_key(s: &str, a: usize) -> bool {
    let bs = s.as_bytes();
    let mut i = a;
    while i > 0 && is_in("\"' \t:=", bs[i - 1]) {
        i -= 1;
    }
    let end = i;
    while i > 0 && (bs[i - 1].is_ascii_alphanumeric() || is_in("_.-", bs[i - 1])) {
        i -= 1;
    }
    let name = s[i..end].to_ascii_lowercase();
    !["sort_key", "cache_key", "prompt_cache_key", "public_key"]
        .iter()
        .any(|k| name.ends_with(k))
}

/// The checks of a name that only ends in _KEY or _PASS: no plain key, no
/// code naming a secret, and what the SECRET rules' values pass.
fn named_secret_at(s: &str, a: usize, b: usize) -> bool {
    not_a_plain_key(s, a) && not_code(s, a, b) && field_at(s, a, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks() {
        assert!(
            !not_a_variable("${PASS}")
                && !not_a_variable("<password>")
                && !not_a_variable("****")
                && !not_a_variable("xxx")
        );
        assert!(not_a_variable("hunter2"));
        assert!(
            secret_value("hunter2x") && !secret_value("hunter") && !secret_value("process.env.X1")
        );
        assert!(
            not_a_slug("sk-proj-abcdEFGH")
                && not_a_slug("sk-0123456789abcdef0123")
                && !not_a_slug("sk-hynix-third-quarter-profit")
        );
        assert!(
            header_value("Bearer zq83Mw2vNx7kRt5")
                && header_value("sid=abc; theme=dark")
                && !header_value("the user must sign in first")
        );
        assert!(!header_value("Bearer ${TOKEN}"));
        assert!(key_at(
            "密钥为12231241",
            "密钥为".len(),
            "密钥为12231241".len()
        ));
        assert!(!key_at(
            "密钥为 2048 位",
            "密钥为 ".len(),
            "密钥为 2048".len()
        ));
    }
}
