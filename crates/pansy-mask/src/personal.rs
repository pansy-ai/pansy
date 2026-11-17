//! Personal data masked whenever `personal` is: emails, mainland Chinese
//! ID card and mobile numbers, bank card numbers.
//!
//! Eleven digits that start 13 to 19 are a mobile number's form, and an
//! id's too (13800138000 as order_id, WHERE id = 18612345678), so the form
//! alone is a phone only where nothing says it is something else (`phone_at`):
//! with +86 before it it is one wherever it is; as a field's value only
//! when the field's name has a phone's word in it; in prose unless an id's
//! word is right before it (订单号 13912345678 is an order).

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;

use crate::finder::Finder;
use crate::ids::name_words;
use crate::options::{PERSONAL, eq_fold};
use crate::re::compile;
use crate::text::{ALNUM, TOKEN_CH, ceil, is_in, last_char, trim, trim_right};

pub(crate) fn rules() -> Vec<Finder> {
    vec![
        Finder::new(
            "EMAIL",
            r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}",
        )
        .markers(&["@"])
        .bound("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_%+-")
        .ok(real_email)
        .at(email_at)
        .cat(PERSONAL),
        Finder::new(
            "ID_CARD",
            r"[1-9]\d{5}(?:18|19|20)\d{2}(?:0[1-9]|1[0-2])(?:0[1-9]|[12]\d|3[01])\d{3}[\dXx]",
        )
        .bound(ALNUM)
        .ok(chinese_id)
        .cat(PERSONAL),
        // Written whole or 3-4-4 (138 0013 8000), as people write them.
        Finder::new(
            "PHONE",
            r"(?:\+?86[- ]?)?1[3-9]\d(?:\d{8}|[- ]\d{4}[- ]\d{4})",
        )
        .bound(ALNUM)
        .at(phone_at)
        .cat(PERSONAL),
        Finder::new(
            "BANK_CARD",
            r"[3-6]\d{3}(?:[ -]?\d{4}){2,3}(?:[ -]?\d{1,3})?",
        )
        .bound(ALNUM)
        .ok(luhn)
        .cat(PERSONAL),
    ]
}

/// Where an email at `s[a..b]` can be: not in a URL's userinfo, and with no
/// dot touching it that makes it part of a longer name. A dot after it
/// that ends a sentence is the sentence's.
fn email_at(s: &str, a: usize, b: usize) -> bool {
    let bs = s.as_bytes();
    if a > 0 && bs[a - 1] == b'.' {
        return false;
    }
    if b + 1 < s.len() && bs[b] == b'.' && is_in(TOKEN_CH, bs[b + 1]) {
        return false;
    }
    !in_userinfo(s, a)
}

/// Says `s[a]` is inside a URL's authority, after its scheme:// and before
/// the host: postgres://app:hunter2pass@db.acme-corp.cn has a password and
/// a host there, no address.
fn in_userinfo(s: &str, a: usize) -> bool {
    let b = s.as_bytes();
    let mut i = a;
    while i > 0 && !is_in(" \t\r\n/?#\"'<>@`", b[i - 1]) {
        i -= 1;
    }
    if i < 3 || b[i - 1] != b'/' || b[i - 2] != b'/' || b[i - 3] != b':' {
        return false;
    }
    let j = i - 3; // the scheme ends at s[j], its colon
    let mut k = j;
    while k > 0 && (b[k - 1].is_ascii_alphanumeric() || is_in("+.-", b[k - 1])) {
        k -= 1;
    }
    while k < j && !b[k].is_ascii_alphabetic() {
        k += 1; // a scheme starts with a letter
    }
    k < j
}

/// Whether `v`, matched as an address, is someone's. git@ is the user
/// every git host is reached by; a file's name (icon@2x.png), a linker's
/// table (system@plt), a doc an agent reads (@AGENTS.md), an example's
/// domain and a no-reply address are no one's.
pub(crate) fn real_email(v: &str) -> bool {
    let Some(at) = v.rfind('@') else {
        return false;
    };
    let local = &v[..at];
    if eq_fold(local, "git") {
        return false;
    }
    let mut host = v[at + 1..].to_lowercase();
    if !mail_host(local, &host) {
        return false;
    }
    if file_host(&host) {
        // an address before a file's extension (alice.smith@acme.com.pdf)
        // is someone's all the same; a scale or a word (@2x.jpg) no domain
        host = file_stem(&host).to_owned();
        if !host.contains('.') || file_host(&host) || !mail_host(local, &host) {
            return false;
        }
    }
    let tld = &host[host.rfind('.').map_or(0, |d| d + 1)..];
    if DOC_FILES.contains(&host.as_str()) || !mail_tld(tld) {
        return false;
    }
    for fake in [
        "example.com",
        "example.org",
        "example.net",
        "localhost",
        ".test",
        ".invalid",
        ".example",
    ] {
        if host == fake.trim_start_matches('.') || host.ends_with(fake) {
            return false;
        }
    }
    !host.starts_with("noreply") && !v.contains("noreply@")
}

/// Whether `host`, after `local` and an @, can be a mail domain: a dot,
/// labels of letters, digits and hyphens (or a script's own), and not only
/// a linker's tables (system@plt, puts@got.plt, memcpy@GLIBC_2.14).
fn mail_host(local: &str, host: &str) -> bool {
    if local.ends_with('@') || !host.contains('.') || trim(host, ".") != host {
        return false;
    }
    let mut linker = true;
    for l in host.split('.') {
        if l.is_empty()
            || l.chars()
                .any(|r| r.is_ascii() && r != '-' && !r.is_ascii_lowercase() && !r.is_ascii_digit())
        {
            return false;
        }
        if !["plt", "got", "sec", "iplt", "igot"].contains(&l) {
            linker = false;
        }
    }
    !linker
}

/// The files agents read by name, on a domain's ending (.md).
const DOC_FILES: &[&str] = &[
    "agents.md",
    "claude.md",
    "gemini.md",
    "readme.md",
    "skill.md",
    "changelog.md",
    "contributing.md",
    "license.md",
    "lessons.md",
    "todo.md",
    "copilot-instructions.md",
    "crush.md",
    "qwen.md",
    "iflow.md",
    "warp.md",
];

/// Names an organisation's own mail can end in that no registry has.
const LOCAL_TLDS: &[&str] = &[
    "local",
    "internal",
    "lan",
    "corp",
    "home",
    "intranet",
    "localdomain",
];

/// The top-level domains of the public suffix list's ICANN part.
static TLDS: LazyLock<HashSet<&'static str>> =
    LazyLock::new(|| include_str!("tlds.txt").lines().collect());

/// Whether `tld`, an address's last label, is one mail can be sent to: a
/// registry's, or an organisation's own. A decorator after a name
/// (x@pytest.fixture) ends in a word no registry has.
fn mail_tld(tld: &str) -> bool {
    LOCAL_TLDS.contains(&tld) || TLDS.contains(tld)
}

/// File name extensions that are no top-level domain: what ends in one is
/// a file's name (261020_14-16-11@2x.jpg), not an address.
const FILE_EXT: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "svg", "heic", "heif", "avif", "bmp", "tif", "tiff",
    "ico", "icns", "pdf", "svgz", "apng", "jxl", "psd", "eps", "lottie", "mp4", "m4v", "webm",
    "mkv", "avi", "mp3", "m4a", "wav", "ogg", "flac", "aac", "ttf", "otf", "woff", "woff2", "eot",
    "json", "yaml", "yml", "toml", "xml", "txt", "csv", "tsv", "log", "html", "htm", "css", "scss",
    "less", "js", "mjs", "cjs", "jsx", "ts", "tsx", "doc", "docx", "xls", "xlsx", "ppt", "pptx",
    "gz", "tgz", "tar", "rar", "7z", "dmg",
];

/// File extensions that are top-level domains too: a name ending in one is
/// a file's only with a scale before it (icon@2x.zip).
const SCALE_EXT: &[&str] = &["zip", "mov", "md", "map"];

static SCALE: LazyLock<Regex> = LazyLock::new(|| compile(r"^[0-9]+(?:\.[0-9]+)?x$"));

/// Says `host`, what follows the @ of what looks like an address, is the
/// end of a file's name.
fn file_host(host: &str) -> bool {
    match host.rfind('.') {
        Some(dot) => {
            let ext = &host[dot + 1..];
            FILE_EXT.contains(&ext)
                || dot > 0 && SCALE_EXT.contains(&ext) && SCALE.is_match(&host[..dot])
        }
        None => FILE_EXT.contains(&host),
    }
}

/// `host` with the file extensions at its end taken off.
fn file_stem(mut host: &str) -> &str {
    while let Some(dot) = host.rfind('.') {
        if !FILE_EXT.contains(&&host[dot + 1..]) {
            break;
        }
        host = &host[..dot];
    }
    host
}

/// A mainland ID number's checksum.
fn chinese_id(v: &str) -> bool {
    const W: [u32; 17] = [7, 9, 10, 5, 8, 4, 2, 1, 6, 3, 7, 9, 10, 5, 8, 4, 2];
    let b = v.as_bytes();
    if b.len() != 18 {
        return false;
    }
    let sum: u32 = (0..17).map(|i| (b[i] - b'0') as u32 * W[i]).sum();
    b[17].to_ascii_uppercase() == b"10X98765432"[(sum % 11) as usize]
}

/// A card number's Luhn checksum, over 15 to 19 digits.
pub(crate) fn luhn(v: &str) -> bool {
    let ds: Vec<u32> = v
        .bytes()
        .filter(|c| c.is_ascii_digit())
        .map(|c| (c - b'0') as u32)
        .collect();
    if ds.len() < 15 || ds.len() > 19 {
        return false;
    }
    let sum: u32 = ds
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &d)| {
            if i % 2 == 1 {
                let d = d * 2;
                if d > 9 { d - 9 } else { d }
            } else {
                d
            }
        })
        .sum();
    sum.is_multiple_of(10)
}

/// The words of a field's name that make its value a phone.
const PHONE_WORDS: &[&str] = &[
    "phone",
    "phones",
    "mobile",
    "mobiles",
    "tel",
    "telephone",
    "cell",
    "cellphone",
    "contact",
    "contacts",
    "whatsapp",
    "sms",
    "msisdn",
    "fax",
];

/// In a field's name in Chinese, these make its value a phone.
const PHONE_MARKS: &[&str] = &["手机", "电话", "電話", "联系", "聯繫", "联络"];

/// Right before a number in prose, these make it an id.
const ID_WORDS: &[&str] = &[
    "order",
    "id",
    "uid",
    "no",
    "no.",
    "#",
    "number",
    "ticket",
    "invoice",
    "订单",
    "订单号",
    "单号",
    "编号",
    "流水号",
    "工单",
    "工单号",
    "运单号",
    "快递单号",
    "订单编号",
    "交易号",
    "序号",
    "账单号",
    "id号",
];

/// Says the PHONE rule's `s[a..b]` is a phone, not an id of the same form.
fn phone_at(s: &str, a: usize, b: usize) -> bool {
    if b - a > 11 {
        return true; // +86, 86
    }
    if let Some((name, prev)) = field_name(s, a) {
        return phone_name(name) || !prev.is_empty() && phone_name(prev) || eq_fold(name, "number");
    }
    if a > 0 && s.as_bytes()[a - 1] == b'#' {
        return false; // #13912345678
    }
    let mut before = trim_right(&s[ceil(s, a.saturating_sub(32))..a], " \t(\"'“「").to_lowercase();
    for c in ["是", "为", "為"] {
        let t = before.strip_suffix(c).unwrap_or(&before);
        before = trim_right(t, " \t").to_owned();
    }
    for w in ID_WORDS {
        let Some(rest) = before.strip_suffix(w) else {
            continue;
        };
        // a whole word: "order", not the end of "recorder"
        let r = last_char(rest);
        if !rest.is_empty()
            && w.as_bytes()[0] < 0x80
            && (r.is_ascii_lowercase() || r.is_ascii_digit() || r == '_')
        {
            continue;
        }
        // but a phone's number: phone no. 13800138000
        if matches!(*w, "no" | "no." | "number") {
            let rest = rest.trim_end_matches(' ');
            if phone_name(&rest[name_start(rest, rest.len())..]) {
                return true;
            }
        }
        return false;
    }
    true
}

/// Says a field's name is a phone's: one of its words is one of
/// PHONE_WORDS (phone_number, mobileNo), or it has one of PHONE_MARKS.
fn phone_name(name: &str) -> bool {
    name_words(name)
        .iter()
        .any(|w| PHONE_WORDS.contains(&w.as_str()))
        || PHONE_MARKS.iter().any(|m| name.contains(m))
}

/// The name of the field whose value starts at `s[a]`: the word before :,
/// =, ： or an XML element's >, with quotes and spaces between, and the
/// word before it with one space between (the phone of "phone no: …").
/// None in prose, where no such separator is right before the value.
pub(crate) fn field_name(s: &str, a: usize) -> Option<(&str, &str)> {
    let b = s.as_bytes();
    let mut i = a;
    while i > 0 && is_in(" \t\"'", b[i - 1]) {
        i -= 1;
    }
    if i > 0 && b[i - 1] == b'>' {
        // <tel>13800138000</tel>: the element's name
        let seg = &s[ceil(s, i.saturating_sub(65))..i - 1];
        let k = seg.rfind('<')?;
        let mut tag = &seg[k + 1..];
        if let Some(sp) = tag.find([' ', '\t']) {
            tag = &tag[..sp];
        }
        if tag.is_empty()
            || tag
                .chars()
                .any(|r| !(r.is_ascii_alphanumeric() || "_.:-".contains(r)))
        {
            return None;
        }
        return Some((tag, ""));
    } else if i > 0 && (b[i - 1] == b':' || b[i - 1] == b'=') {
        i -= 1;
    } else if s[..i].ends_with('：') {
        i -= '：'.len_utf8();
    } else {
        return None;
    }
    while i > 0 && is_in(" \t\"'", b[i - 1]) {
        i -= 1;
    }
    let j = name_start(s, i);
    if j == i {
        return None;
    }
    let mut name = &s[j..i];
    let n = name.chars().count();
    if n > 6 && has_cjk(name) {
        // a long run of CJK before a colon is a sentence's: its last few
        // characters name the value (我的手机号是：…)
        let skip: usize = name.chars().take(n - 6).map(char::len_utf8).sum();
        name = &name[skip..];
    }
    let prev = if j >= 2 && b[j - 1] == b' ' && b[j - 2] != b' ' {
        &s[name_start(s, j - 1)..j - 1]
    } else {
        ""
    };
    Some((name, prev))
}

/// Where the name that ends at `s[i]` starts: letters, digits, _, . and -
/// back from i, or Chinese characters.
fn name_start(s: &str, i: usize) -> usize {
    let mut j = i;
    for r in s[..i].chars().rev() {
        if !(r.is_ascii_alphanumeric()
            || "_.-".contains(r)
            || ('\u{4e00}'..='\u{9fff}').contains(&r))
        {
            break;
        }
        j -= r.len_utf8();
    }
    j
}

fn has_cjk(s: &str) -> bool {
    s.chars().any(|r| ('\u{4e00}'..='\u{9fff}').contains(&r))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksums() {
        assert!(chinese_id("11010519491231002X") && !chinese_id("110105194912310021"));
        assert!(luhn("4111 1111 1111 1111") && !luhn("4111111111111112"));
    }

    #[test]
    fn emails() {
        for v in [
            "jane.doe@acme.io",
            "jo@quill.dev",
            "a@b.cn",
            "ops@files.zip",
            "alice.smith@acme.com.pdf",
            "x@corp.internal",
        ] {
            assert!(real_email(v), "{v}");
        }
        for v in [
            "git@github.com",
            "user@example.com",
            "icon@2x.png",
            "intro@1.5x.mov",
            "system@plt",
            "puts@got.plt",
            "x@pytest.fixture",
            "xxx@Agents.md",
            "noreply@acme.io",
            "a@b.test",
        ] {
            assert!(!real_email(v), "{v}");
        }
    }

    #[test]
    fn phones_and_ids() {
        let at = |s: &str| {
            let a = s.find(|c: char| c.is_ascii_digit()).unwrap();
            phone_at(s, a, a + 11)
        };
        assert!(at("我的手机号是 13800138000"));
        assert!(at("call 13800138000, thanks"));
        assert!(at(r#""phone": "13800138000""#));
        assert!(at("<tel>13800138000</tel>"));
        assert!(at("phone no. 13800138000"));
        assert!(!at("订单号 13912345678 已发货"));
        assert!(!at(r#""order_id": "13800138000""#));
        assert!(!at("WHERE id = 18612345678"));
        assert!(!at("#13912345678"));
        assert!(!at("order 13800138000"));
    }
}
