//! More kinds of personal data, each a category the user turns on or off:
//! numbers that check themselves (an SSN's ranges, an IBAN's mod 97),
//! numbers that are only one by the words before them (a passport's, a
//! serial number, a birthday), and what names the user's machine and
//! storage: the user name in a home folder, user@host, a MAC address, a
//! public IP address, a cloud bucket.
//!
//! Each finds as little as it can be sure of. A date is a birthday only
//! after 生日, DOB or born; a dotted quad after "version" is no address; a
//! bucket called my-bucket is an example.

use std::net::IpAddr;
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;

use crate::finder::Finder;
use crate::re::compile;
use crate::text::{ALNUM, DIGITS, ceil, is_in, word_byte};

/// What may stand between a label and its value: Passport No.: …
const SEP: &str = r#"["']?[ \t]*(?:[:#=：][ \t]*)?"#;
/// A month's name as a date writes it.
const MONTHS: &str = r"(?i:jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-z]*\.?";
/// A user name in a path: anything a folder's name can have, in any
/// script, but no $USER, %USERNAME%, <name> or {user}.
const PATH_NAME: &str = r#"[^\s/\\:*?"'<>|,;()\[\]{}`$%]{1,64}"#;
/// A host's name, dotted or not.
const HOST: &str = r"[A-Za-z0-9][A-Za-z0-9-]{0,62}(?:\.[A-Za-z0-9-]{1,63})*";
/// A user name as Unix has one.
const UNIX_USER: &str = r"[a-z_][a-z0-9_.-]{0,31}";
/// A user name on a line of its own, as a login has it.
const LINE_USER: &str = r"[A-Za-z0-9_][A-Za-z0-9._-]{0,31}";
/// A bucket's or a container's name, without a dot or a hyphen at its end.
const BUCKET: &str = r"[A-Za-z0-9](?:[A-Za-z0-9._-]{0,61}[A-Za-z0-9])?";

const BUCKET_HOSTS: &[&str] = &[
    "amazonaws.com",
    "googleapis.com",
    "cloud.google.com",
    "aliyuncs.com",
    "myqcloud.com",
    "windows.net",
    "cloudflarestorage.com",
    "digitaloceanspaces.com",
    "myhuaweicloud.com",
    "volces.com",
];

pub(crate) fn rules() -> Vec<Finder> {
    let alnum_dash: &'static str =
        "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-";
    vec![
        // US Social Security numbers: 123-45-6789 as written, or nine
        // digits after SSN; in the ranges the SSA gives out
        Finder::new("SSN", r"\d{3}-\d{2}-\d{4}").cat("ssn").markers(&["-"]).bound(alnum_dash).ok(ssn_ok),
        Finder::new(
            "SSN",
            format!(r#"(?i:\bssn|\bsocial[ \t]+security(?:[ \t]+(?:number|no\.?|#))?){SEP}(?:(?i:is)[ \t]+)?["']?(\d{{9}}|\d{{3}} \d{{2}} \d{{4}})"#),
        )
        .cat("ssn")
        .fold(&["ssn", "social"])
        .window(8, 80)
        .bound(DIGITS)
        .ok(ssn_ok),
        Finder::new(
            "PASSPORT",
            format!(
                r#"(?i:\bpassport(?:[ \t]*(?:no\.?|number|num|#))?|\breisepass(?:nummer)?){SEP}(?:(?i:is|was)[ \t]+)?["']?([A-Z0-9]{{6,9}})|(?:护照(?:号码|号)?|護照(?:號碼|号码|番号)?|パスポート(?:番号)?)[ \t]*(?:[:：=][ \t]*)?(?:是[ \t]*|は[ \t]*)?["']?([A-Z0-9]{{6,9}})"#
            ),
        )
        .cat("passport")
        .any_group()
        .bound(ALNUM)
        .ok(passport_ok)
        .fold(&["passport", "reisepass", "护照", "護照", "パスポート"])
        .window(8, 80),
        // an IBAN, as one word or printed in groups of four, by its checksum
        Finder::new("IBAN", r"[A-Z]{2}\d{2}(?:[A-Z0-9]{11,30}|(?: [A-Z0-9]{4}){2,7}(?: [A-Z0-9]{1,3})?)")
            .cat("iban")
            .bound(ALNUM)
            .fix(iban_at),
        // a date after the words that say it is a birthday; a date alone
        // is not
        Finder::new(
            "BIRTHDAY",
            format!(
                r#"(?:(?i:\bdate[ \t]+of[ \t]+birth|\bbirth[ \t_-]?date|\bbirthday|\bdob\b|\bd\.o\.b\.?|\bborn(?:[ \t]+on)?|\bgeburtsdatum|\bgeboren(?:[ \t]+am)?)|生日|出生日期|出生年月日|出生于|出生於|出生|誕生日|生年月日){SEP}(?:(?i:is|was)[ \t]+|是[ \t]*|为[ \t]*|為[ \t]*|は[ \t]*)?["']?(\d{{4}}[-/.]\d{{1,2}}[-/.]\d{{1,2}}|\d{{4}}[ \t]*年[ \t]*\d{{1,2}}[ \t]*月[ \t]*\d{{1,2}}[ \t]*日?|\d{{1,2}}[-/.]\d{{1,2}}[-/.]\d{{4}}|{MONTHS}[ \t]+\d{{1,2}}(?:st|nd|rd|th)?,?[ \t]+\d{{4}}|\d{{1,2}}(?:st|nd|rd|th)?[ \t]+{MONTHS},?[ \t]+\d{{4}})"#
            ),
        )
        .cat("birthday")
        .bound(DIGITS)
        .ok(date_ok)
        .fold(&["birth", "dob", "d.o.b", "born", "geburt", "geboren", "生日", "出生", "誕生日", "生年月日"])
        .window(16, 80),
        // a MAC address; not a piece of an IPv6 address or a fingerprint
        Finder::new("MAC", r"[0-9A-Fa-f]{2}(?::[0-9A-Fa-f]{2}){5}|[0-9A-Fa-f]{2}(?:-[0-9A-Fa-f]{2}){5}")
            .cat("mac")
            .bound("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789:-")
            .ok(mac_ok),
        // a serial number after S/N, Serial or 序列号, and a separator; not
        // "serial port" or Serial.println
        Finder::new(
            "SERIAL",
            r#"(?i:\bserial[ \t_-]?(?:number|num|no\.?|#)?|\bs/n|\bsn)(?:[ \t]*\([^)\n]{1,20}\))?["']?[ \t]*[:#=：][ \t]*["']?([A-Za-z0-9][A-Za-z0-9-]{4,39})|(?:序列号|序列號|シリアル番号|(?i:seriennummer))[ \t]*[:：=]?[ \t]*["']?([A-Za-z0-9][A-Za-z0-9-]{4,39})"#,
        )
        .cat("serial")
        .any_group()
        .bound(alnum_dash)
        .ok(serial_ok)
        .fold(&["serial", "s/n", "sn", "序列", "シリアル", "seriennummer"])
        .window(8, 80),
        // the user's name in a home folder: /Users/<name>, /home/<name>,
        // C:\Users\<name>; the rest of the path stays
        Finder::new("USERNAME", format!(r"(?:^|[^A-Za-z0-9_.~/\\-])(?:file://)?(?:/mnt/[a-z])?/(?:Users|home)/({PATH_NAME})"))
            .cat("home")
            .fix(home_at)
            .fold(&["/users/", "/home/"])
            .window(16, 80),
        Finder::new(
            "USERNAME",
            format!(
                r"(?:^|[^A-Za-z0-9])[A-Za-z]:(?:\\{{1,4}}|/)(?i:users|documents and settings)(?:\\{{1,4}}|/)(?:([A-Za-z0-9._-]+(?: [A-Za-z0-9._-]+)+)(?:\\|/)|({PATH_NAME}))"
            ),
        )
        .cat("home")
        .any_group()
        .fix(home_at)
        .fold(&["users", "documents and settings"])
        .window(12, 100),
        // user@host as a shell prompt has it (user@host:~$) or as ssh, scp
        // and rsync are given it; a package pinned at its version
        // (js-tokens@4.0.0: in pnpm-lock.yaml) is no login
        Finder::new(
            "",
            format!(r"(?m)(?:^|[^A-Za-z0-9._%+-])({UNIX_USER})@({HOST})(?::[~/]|:[ \t]*$|[ \t]+[~/]|[ \t]*[$%#>][ \t]|[ \t]*[$%#>]$)"),
        )
        .cat("userhost")
        .groups(&[("USERNAME", user_ok), ("HOSTNAME", host_ok)])
        .markers(&["@"])
        .at(not_package_pin),
        Finder::new("", format!(r#"(?i:\b(?:ssh|scp|sftp|rsync|mosh|ssh-copy-id))\b[^\n@]{{0,80}}?[ \t"']({UNIX_USER})@({HOST})"#))
            .cat("userhost")
            .groups(&[("USERNAME", user_ok), ("HOSTNAME", host_ok)])
            .markers(&["@"])
            .at(not_package_pin),
        // hostname: x, HOSTNAME=x; or ssh config's HostName x under its Host
        Finder::new(
            "HOSTNAME",
            format!(
                r#"(?m)^[ \t]*["']?(?:(?i:host[ \t_-]?name|computer[ \t_-]?name|machine[ \t_-]?name|device[ \t_-]?name)|主机名|主機名|計算機名|コンピュータ名|コンピューター名)["']?[ \t]*[:=：][ \t]*(?:["']({HOST})["'][ \t]*[,;]?|({HOST}))[ \t]*$|^[ \t]+HostName[ \t]+({HOST})[ \t]*$"#
            ),
        )
        .cat("userhost")
        .any_group()
        .ok(line_host_ok)
        .fold(&["name", "主机", "主機", "計算機", "コンピュータ"]),
        // User: x, USER=x; or ssh config's User x under its Host
        Finder::new(
            "USERNAME",
            format!(
                r#"(?m)^[ \t]*["']?(?:(?i:user[ \t_-]?name|login[ \t_-]?name|user|login|logname)|用户名|用戶名|ユーザー名)["']?[ \t]*[:=：][ \t]*(?:["']({LINE_USER})["'][ \t]*[,;]?|({LINE_USER}))[ \t]*$|^[ \t]+User[ \t]+({LINE_USER})[ \t]*$"#
            ),
        )
        .cat("userhost")
        .any_group()
        .ok(line_user_ok)
        .at(not_line_ident)
        .fold(&["user", "login", "logname", "用户", "用戶", "ユーザー"]),
        // a public IP address; not a private, loopback, link-local, CGNAT
        // or documentation one, nor a version
        Finder::new("IP", r"(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)(?:\.(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)){3}")
            .cat("ip")
            .markers(&["."])
            .at(ipv4_at),
        Finder::new("IP", r"[0-9A-Fa-f]{1,4}(?::[0-9A-Fa-f]{0,4}){2,7}")
            .cat("ip")
            .markers(&[":"])
            .bound("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789:_")
            .ok(ipv6_ok),
        // a cloud bucket: s3://<b>, gs://<b>, abfss://<c>@<account>…,
        // <b>.s3.amazonaws.com, s3.amazonaws.com/<b>
        Finder::new(
            "",
            format!(
                r"(?i:\b(?:s3a?|s3n|gs|oss|cosn?|obs|r2|ks3|bos|tos|az|azure|wasbs?|abfss?)://)({BUCKET})(?:@([a-z0-9]{{3,24}})\.(?:blob|dfs)\.core\.windows\.net)?"
            ),
        )
        .cat("bucket")
        .groups(&[("BUCKET", bucket_ok), ("BUCKET", bucket_ok)])
        .markers(&["://"]),
        Finder::new(
            "BUCKET",
            r"(?:^|[^A-Za-z0-9.-])([a-z0-9](?:[a-z0-9.-]{0,61}[a-z0-9])?)\.(?:s3(?:[.-](?:dualstack\.)?[a-z0-9-]+)?\.amazonaws\.com(?:\.cn)?|s3-accelerate\.amazonaws\.com|storage\.googleapis\.com|oss-[a-z0-9-]+\.aliyuncs\.com|cos\.[a-z0-9-]+\.myqcloud\.com|obs\.[a-z0-9-]+\.myhuaweicloud\.com|r2\.cloudflarestorage\.com|(?:blob|dfs)\.core\.windows\.net|[a-z0-9-]+\.digitaloceanspaces\.com|tos-[a-z0-9-]+\.volces\.com)",
        )
        .cat("bucket")
        .ok(bucket_ok)
        .fold(BUCKET_HOSTS)
        .window(128, 32),
        Finder::new(
            "BUCKET",
            format!(
                r"(?:^|[^A-Za-z0-9.-])(?:s3(?:[.-](?:dualstack\.)?[a-z0-9-]+)?\.amazonaws\.com(?:\.cn)?|storage\.googleapis\.com|storage\.cloud\.google\.com|[a-z0-9]{{3,24}}\.blob\.core\.windows\.net|oss-[a-z0-9-]+\.aliyuncs\.com|[a-z0-9-]+\.digitaloceanspaces\.com)/({BUCKET})"
            ),
        )
        .cat("bucket")
        .ok(bucket_ok)
        .fold(BUCKET_HOSTS)
        .window(48, 80),
    ]
}

fn ssn_ok(v: &str) -> bool {
    let d: String = v.chars().filter(|c| *c != '-' && *c != ' ').collect();
    if d.len() != 9
        || &d[..3] == "000"
        || &d[..3] == "666"
        || d.starts_with('9')
        || &d[3..5] == "00"
        || &d[5..] == "0000"
    {
        return false;
    }
    // the ones printed in ads and examples, and one digit nine times
    if matches!(d.as_str(), "123456789" | "078051120" | "219099999") {
        return false;
    }
    d.bytes().any(|c| c != d.as_bytes()[0])
}

fn passport_ok(v: &str) -> bool {
    v.bytes().filter(|c| c.is_ascii_digit()).count() >= 5 && v.bytes().any(|c| c != v.as_bytes()[0])
}

/// How long each country's IBAN is.
const IBAN_LEN: &[(&str, usize)] = &[
    ("AD", 24),
    ("AE", 23),
    ("AL", 28),
    ("AT", 20),
    ("AZ", 28),
    ("BA", 20),
    ("BE", 16),
    ("BG", 22),
    ("BH", 22),
    ("BI", 27),
    ("BR", 29),
    ("BY", 28),
    ("CH", 21),
    ("CR", 22),
    ("CY", 28),
    ("CZ", 24),
    ("DE", 22),
    ("DJ", 27),
    ("DK", 18),
    ("DO", 28),
    ("EE", 20),
    ("EG", 29),
    ("ES", 24),
    ("FI", 18),
    ("FK", 18),
    ("FO", 18),
    ("FR", 27),
    ("GB", 22),
    ("GE", 22),
    ("GI", 23),
    ("GL", 18),
    ("GR", 27),
    ("GT", 28),
    ("HN", 28),
    ("HR", 21),
    ("HU", 28),
    ("IE", 22),
    ("IL", 23),
    ("IQ", 23),
    ("IS", 26),
    ("IT", 27),
    ("JO", 30),
    ("KW", 30),
    ("KZ", 20),
    ("LB", 28),
    ("LC", 32),
    ("LI", 21),
    ("LT", 20),
    ("LU", 20),
    ("LV", 21),
    ("LY", 25),
    ("MC", 27),
    ("MD", 24),
    ("ME", 22),
    ("MK", 19),
    ("MN", 20),
    ("MR", 27),
    ("MT", 31),
    ("MU", 30),
    ("NI", 28),
    ("NL", 18),
    ("NO", 15),
    ("OM", 23),
    ("PK", 24),
    ("PL", 28),
    ("PS", 29),
    ("PT", 25),
    ("QA", 29),
    ("RO", 24),
    ("RS", 22),
    ("RU", 33),
    ("SA", 24),
    ("SC", 31),
    ("SD", 18),
    ("SE", 24),
    ("SI", 19),
    ("SK", 24),
    ("SM", 27),
    ("SO", 23),
    ("ST", 25),
    ("SV", 28),
    ("TL", 23),
    ("TN", 24),
    ("TR", 26),
    ("UA", 29),
    ("VA", 22),
    ("VG", 24),
    ("XK", 20),
    ("YE", 30),
];

/// Says `v`, spaces and all, is an IBAN: its country's length, and mod 97
/// of it, its first four moved to its end, is 1.
fn iban_ok(v: &str) -> bool {
    let v: String = v.chars().filter(|c| *c != ' ').collect();
    if v.len() < 4
        || !v.is_ascii()
        || IBAN_LEN
            .iter()
            .find(|(c, _)| *c == &v[..2])
            .is_none_or(|(_, n)| *n != v.len())
    {
        return false;
    }
    let mut m = 0u32;
    for c in v[4..].bytes().chain(v[..4].bytes()) {
        m = match c {
            b'0'..=b'9' => (m * 10 + (c - b'0') as u32) % 97,
            b'A'..=b'Z' => (m * 100 + (c - b'A') as u32 + 10) % 97,
            _ => return false,
        };
    }
    m == 1
}

/// The IBAN at `s[a..b]`, or the one before a word in capitals the pattern
/// took as its last group: BE68 5390 0754 7034 THEN.
fn iban_at(s: &str, a: usize, mut b: usize) -> Option<(usize, usize)> {
    loop {
        if iban_ok(&s[a..b]) {
            return Some((a, b));
        }
        b = a + s[a..b].rfind(' ')?;
    }
}

static DATE_NUMS: LazyLock<Regex> = LazyLock::new(|| compile(r"\d+"));

/// Today, as year, month and day (UTC).
pub(crate) fn today() -> (i64, i64, i64) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    civil(secs.div_euclid(86_400))
}

/// The date `days` after 1970-01-01.
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + if m <= 2 { 1 } else { 0 }, m, d)
}

/// Says `v` is a date that can be someone's birthday: a real month and day,
/// in a year from 1900 on, and over a year ago. A date in the last year is
/// the day an agent runs on, a deadline or a log's.
fn date_ok(v: &str) -> bool {
    let ns: Vec<&str> = DATE_NUMS.find_iter(v).map(|m| m.as_str()).collect();
    let num = |s: &str| s.parse::<i64>().unwrap_or(0);
    let (ty, tm, td) = today();
    let valid = |y: i64, m: i64, d: i64| {
        (1900..=2099).contains(&y)
            && (1..=12).contains(&m)
            && (1..=31).contains(&d)
            && (y, m, d) < (ty - 1, tm, td)
    };
    if ns.len() == 2 {
        // March 5, 1990 or 5 March 1990
        let l = v.to_lowercase();
        for (i, m) in [
            "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
        ]
        .iter()
        .enumerate()
        {
            if l.contains(m) {
                let (mut d, mut y) = (num(ns[0]), num(ns[1]));
                if ns[0].len() == 4 {
                    (d, y) = (y, d);
                }
                return valid(y, i as i64 + 1, d);
            }
        }
        return false;
    }
    if ns.len() != 3 {
        return false;
    }
    if ns[0].len() == 4 {
        return valid(num(ns[0]), num(ns[1]), num(ns[2]));
    }
    let (p, q, y) = (num(ns[0]), num(ns[1]), num(ns[2]));
    valid(y, p, q) || valid(y, q, p)
}

/// Turns away what stands for an address in docs and code, the broadcast
/// one, and twelve decimal digits, which a MAC address almost never is.
fn mac_ok(v: &str) -> bool {
    let l = v.replace('-', ":").to_ascii_lowercase();
    if matches!(
        l.as_str(),
        "00:00:00:00:00:00"
            | "ff:ff:ff:ff:ff:ff"
            | "00:11:22:33:44:55"
            | "aa:bb:cc:dd:ee:ff"
            | "01:23:45:67:89:ab"
            | "12:34:56:78:9a:bc"
    ) {
        return false;
    }
    l.contains(['a', 'b', 'c', 'd', 'e', 'f'])
}

fn serial_ok(v: &str) -> bool {
    v.len() >= 6 && v.bytes().any(|c| c.is_ascii_digit()) && v.bytes().any(|c| c != v.as_bytes()[0])
}

/// What a doc or a container writes for a user's name, and the folders
/// under /Users that are no one's.
const NOT_NAMES: &[&str] = &[
    "shared",
    "public",
    "guest",
    "default",
    "default user",
    "all users",
    "user",
    "username",
    "user_name",
    "yourname",
    "your-name",
    "your_name",
    "yourusername",
    "you",
    "me",
    "name",
    "example",
    "foo",
    "bar",
    "someone",
    "john",
    "jdoe",
    "runner",
    "node",
    "vscode",
    "codespace",
    "linuxbrew",
    "...",
];

/// The user name in a home folder at `s[a..b]`, without the full stop of a
/// sentence after it.
fn home_at(s: &str, a: usize, mut b: usize) -> Option<(usize, usize)> {
    while b > a && s.as_bytes()[b - 1] == b'.' {
        b -= 1;
    }
    if b == a || s.as_bytes()[a] == b'.' || NOT_NAMES.contains(&s[a..b].to_lowercase().as_str()) {
        return None;
    }
    Some((a, b))
}

/// The users of a service or a cloud image, which say nothing of who the
/// user is, and the words a "User:" line in a chat has.
const NOT_USERS: &[&str] = &[
    "git",
    "root",
    "ec2-user",
    "ubuntu",
    "admin",
    "postgres",
    "www-data",
    "nobody",
    "centos",
    "debian",
    "fedora",
    "azureuser",
    "opc",
    "core",
    "vagrant",
    "docker",
    "str",
    "string",
    "int",
    "bool",
    "any",
    "none",
    "null",
    "nil",
    "true",
    "false",
    "undefined",
    "hi",
    "hello",
    "hey",
    "yes",
    "no",
    "ok",
    "okay",
    "thanks",
    "continue",
    "go",
    "stop",
    "help",
    "test",
    "y",
    "n",
];

fn user_ok(v: &str) -> bool {
    let l = v.to_lowercase();
    v.len() >= 2 && !NOT_USERS.contains(&l.as_str()) && !NOT_NAMES.contains(&l.as_str())
}

/// What a value in code is read from: user = request.user.
const CODE_OBJECTS: &[&str] = &[
    "self", "this", "os", "process", "request", "req", "ctx", "context", "config", "cfg", "conf",
    "settings", "env", "args", "opts", "options", "props", "params", "session", "socket",
    "platform", "window", "location", "url", "u", "r", "c", "s", "m", "p", "x", "data", "info",
    "user",
];

/// Says a dotted value on a "User:" or "hostname:" line is read from an
/// object in code: request.user, os.hostname.
fn in_code(v: &str) -> bool {
    let l = v.to_lowercase();
    let Some((first, _)) = l.split_once('.') else {
        return false;
    };
    let last = &l[l.rfind('.').map_or(0, |i| i + 1)..];
    CODE_OBJECTS.contains(&first)
        || NOT_USERS.contains(&last)
        || NOT_NAMES.contains(&last)
        || PUBLIC_HOSTS.contains(&last)
        || last == "login"
        || last == "username"
}

/// Says the user or the host of user@host at `s[a..b]` is not half of a
/// package pinned at its version (js-tokens@4.0.0: in pnpm-lock.yaml). A
/// host that is an address (alice@10.0.0.5) is a host still.
fn not_package_pin(s: &str, a: usize, b: usize) -> bool {
    let bs = s.as_bytes();
    if b < s.len() && bs[b] == b'@' {
        // the user: what follows the @ is the host
        let mut e = b + 1;
        while e < s.len() && (bs[e].is_ascii_alphanumeric() || bs[e] == b'-' || bs[e] == b'.') {
            e += 1;
        }
        return !version_host(s[b + 1..e].trim_end_matches('.'));
    }
    !(a > 0 && bs[a - 1] == b'@' && version_host(&s[a..b]))
}

static VERSION_HOST: LazyLock<Regex> =
    LazyLock::new(|| compile(r"^v?\d+(?:\.\d+)+(?:-[0-9A-Za-z.-]+)?$"));

/// Says `h`, after a user's @, is a version and not a host: no host's name
/// is numbers and dots alone, but an address is.
fn version_host(h: &str) -> bool {
    VERSION_HOST.is_match(h) && h.parse::<IpAddr>().is_err()
}

fn line_user_ok(v: &str) -> bool {
    let l = v.to_lowercase();
    user_ok(v)
        && !in_code(v)
        && !CHAT_WORDS.contains(&l.as_str())
        && !IDENT_PARTS.contains(&l.as_str())
}

/// The words of a variable's or a type's name that holds a user, and never
/// a login of its own: current_user, UserInfo, loginName.
const IDENT_PARTS: &[&str] = &[
    "user", "users", "usr", "username", "name", "names", "login", "logname", "account", "acct",
    "owner", "author", "member", "person", "profile", "current", "cur", "default", "target", "new",
    "old", "my", "the", "obj", "object", "info", "id", "model", "data", "record", "entity", "row",
    "item", "input", "arg", "args", "param", "value", "val", "str", "type", "dto", "db", "auth",
    "session", "self",
];

/// Says the value of a "User:" line at `s[a..b]` is written out, not a
/// variable's or a type's name in code. Unquoted, a value of words one of
/// which is IDENT_PARTS is code's (current_user, UserInfo); a quoted value
/// is a string, and a login of words of its own (deploy_bot) still masks.
fn not_line_ident(s: &str, a: usize, b: usize) -> bool {
    if a > 0 && matches!(s.as_bytes()[a - 1], b'"' | b'\'') {
        return true;
    }
    let w = crate::fieldvalue::ident_words(&s[a..b]);
    w.len() < 2
        || !w
            .iter()
            .any(|x| IDENT_PARTS.contains(&x.to_lowercase().as_str()))
}

/// Says the user name known at `s[a..b]` is the user's name there, not a
/// name in code that happens to be the same word: joined to a name's other
/// parts (developer_dir, self.developer), declared, assigned or called
/// (const developer, developer = 1), a JSON key, or a parameter.
pub(crate) fn name_alone(s: &str, a: usize, b: usize) -> bool {
    let bs = s.as_bytes();
    let ident = |c: u8| word_byte(c) || c == b'$';
    if a > 0 && ident(bs[a - 1]) || b < s.len() && ident(bs[b]) {
        return false;
    }
    if a > 1 && matches!(bs[a - 1], b'.' | b'-') && ident(bs[a - 2])
        || b + 1 < s.len() && matches!(bs[b], b'.' | b'-') && ident(bs[b + 1])
    {
        return false;
    }
    let quoted = a > 0 && b < s.len() && matches!(bs[a - 1], b'"' | b'\'') && bs[b] == bs[a - 1];
    if quoted {
        // a key: "developer": …
        return !s[b + 1..].trim_start_matches([' ', '\t']).starts_with(':');
    }
    if a > 0 && matches!(bs[a - 1], b'"' | b'\'' | b'`') {
        return true; // a string's first word
    }
    let rest = s[b..].trim_start_matches([' ', '\t']);
    if rest.starts_with(['(', '[', '=']) || rest.starts_with(":=") {
        return false;
    }
    let head = s[..a].trim_end_matches([' ', '\t']);
    if head.len() < a {
        // a space before it: a keyword that declares it
        for kw in DECL_WORDS {
            if head.ends_with(kw)
                && (head.len() == kw.len() || !ident(head.as_bytes()[head.len() - kw.len() - 1]))
            {
                return false;
            }
        }
    }
    if head.ends_with(" =") || head.ends_with("\t=") || head.ends_with(":=") {
        return false;
    }
    let hb = head.as_bytes();
    if let Some(&last) = hb.last()
        && (last == b'(' || last == b',')
        && PARAM_REST.is_match(rest)
    {
        // but uid=501(developer) gid=20(staff): id's output names the user
        return last == b'(' && hb.len() > 1 && hb[hb.len() - 2].is_ascii_digit();
    }
    true
}

/// What is after a parameter's name in a list: the list goes on or ends,
/// or the parameter's type comes first (developer: str).
static PARAM_REST: LazyLock<Regex> =
    LazyLock::new(|| compile(r"^(?:[,):]|[*\[\]]*[A-Za-z_][\w.*\[\]]*[ \t]*[,)])"));

/// Before a name with a space, these declare it.
const DECL_WORDS: &[&str] = &[
    "const",
    "let",
    "var",
    "val",
    "def",
    "func",
    "fn",
    "function",
    "class",
    "struct",
    "interface",
    "enum",
    "return",
    "import",
    "typeof",
];

fn line_host_ok(v: &str) -> bool {
    host_ok(v) && !in_code(v)
}

/// What a "User:" line in a chat's transcript says on its own.
const CHAT_WORDS: &[&str] = &[
    "done",
    "why",
    "what",
    "how",
    "sure",
    "please",
    "retry",
    "next",
    "again",
    "great",
    "good",
    "nice",
    "cool",
    "fix",
    "go-ahead",
    "proceed",
    "lgtm",
    "required",
    "optional",
    "unknown",
    "anonymous",
    "guest",
    "current",
    "me",
];

/// Hosts everyone's ssh and git go to, and hostname values that are none.
const PUBLIC_HOSTS: &[&str] = &[
    "github.com",
    "gitlab.com",
    "bitbucket.org",
    "ssh.dev.azure.com",
    "vs-ssh.visualstudio.com",
    "localhost",
    "host",
    "hostname",
    "server",
    "remote",
    "example",
    "your-server",
    "str",
    "string",
    "none",
    "null",
    "true",
    "false",
    "undefined",
];

fn host_ok(v: &str) -> bool {
    let l = v.trim_end_matches('.').to_lowercase();
    if l.len() < 2
        || PUBLIC_HOSTS.contains(&l.as_str())
        || l.starts_with("process.")
        || l.starts_with("os.")
        || l.starts_with("env.")
    {
        return false;
    }
    if l.parse::<IpAddr>().is_ok() {
        return false; // an address is the IP category's
    }
    for ex in [
        "example.com",
        "example.org",
        "example.net",
        ".example",
        ".test",
        ".invalid",
        ".localhost",
    ] {
        if l == ex.trim_start_matches('.') || l.ends_with(ex) {
            return false;
        }
    }
    true
}

/// Public DNS resolvers everyone uses, which say nothing of the user.
const RESOLVERS: &[&str] = &[
    "8.8.8.8",
    "8.8.4.4",
    "1.1.1.1",
    "1.0.0.1",
    "9.9.9.9",
    "149.112.112.112",
    "208.67.222.222",
    "208.67.220.220",
    "114.114.114.114",
    "223.5.5.5",
    "223.6.6.6",
    "119.29.29.29",
    "180.76.76.76",
    "2001:4860:4860::8888",
    "2001:4860:4860::8844",
    "2606:4700:4700::1111",
    "2606:4700:4700::1001",
];

/// Says an address is someone's own: not private, shared (CGNAT),
/// loopback, link-local, multicast, documentation, benchmarking, reserved,
/// or a public resolver.
fn public_ip(a: IpAddr) -> bool {
    let a = match a {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => a,
        },
        _ => a,
    };
    if RESOLVERS.contains(&a.to_string().as_str()) {
        return false;
    }
    match a {
        IpAddr::V4(v4) => {
            let n = u32::from(v4);
            let within = |p: u32, bits: u32| n >> (32 - bits) == p >> (32 - bits);
            ![
                (0x0000_0000, 8),  // 0.0.0.0/8
                (0x0a00_0000, 8),  // 10/8
                (0x6440_0000, 10), // 100.64/10
                (0x7f00_0000, 8),  // loopback
                (0xa9fe_0000, 16), // link-local
                (0xac10_0000, 12), // 172.16/12
                (0xc000_0000, 24), // 192.0.0/24
                (0xc000_0200, 24), // 192.0.2/24
                (0xc0a8_0000, 16), // 192.168/16
                (0xc612_0000, 15), // 198.18/15
                (0xc633_6400, 24), // 198.51.100/24
                (0xcb00_7100, 24), // 203.0.113/24
                (0xe000_0000, 4),  // multicast
                (0xf000_0000, 4),  // 240/4
            ]
            .iter()
            .any(|&(p, bits)| within(p, bits))
        }
        IpAddr::V6(v6) => {
            let n = u128::from(v6);
            // only global unicast, 2000::/3
            if (n >> 120) as u8 & 0xe0 != 0x20 {
                return false;
            }
            let within = |p: u128, bits: u32| n >> (128 - bits) == p >> (128 - bits);
            !within(0x2001_0db8 << 96, 32)
        }
    }
}

/// Says the dotted quad at `s[a..b]` is a public address, not a piece of a
/// longer one or of a version: 1.2.3.4.5, v1.2.3.4, ==1.2.3.4,
/// Chrome/120.0.6099.109, version 2.31.0.1.
fn ipv4_at(s: &str, a: usize, b: usize) -> bool {
    let bs = s.as_bytes();
    if a > 0 {
        let c = bs[a - 1];
        if c.is_ascii_alphanumeric() || is_in("._-=~^", c) {
            return false;
        }
        // a product's version, not a URL's host
        if c == b'/' && a >= 2 && (bs[a - 2].is_ascii_alphanumeric() || is_in("._-", bs[a - 2])) {
            return false;
        }
    }
    if b < s.len() {
        let c = bs[b];
        if c.is_ascii_alphanumeric()
            || c == b'_'
            || c == b'-'
            || c == b'.' && b + 1 < s.len() && bs[b + 1].is_ascii_digit()
        {
            return false;
        }
    }
    let before = s[ceil(s, a.saturating_sub(16))..a]
        .trim_end_matches([' ', '\t', ':', '=', '"', '\''])
        .to_lowercase();
    for w in ["version", "ver", "release", "build", "rev", "版本"] {
        // the word itself, not the end of another: server, observer
        if before.ends_with(w)
            && (before.len() == w.len()
                || !word_byte(before.as_bytes()[before.len() - w.len() - 1]))
        {
            return false;
        }
    }
    s[a..b].parse::<IpAddr>().is_ok_and(public_ip)
}

fn ipv6_ok(v: &str) -> bool {
    v.matches(':').count() >= 2
        && matches!(v.parse::<IpAddr>(), Ok(ip @ IpAddr::V6(_)) if public_ip(ip))
}

/// The buckets docs and examples name, and an API path's first folder.
const NOT_BUCKETS: &[&str] = &[
    "bucket",
    "my-bucket",
    "mybucket",
    "bucket-name",
    "bucketname",
    "my-bucket-name",
    "your-bucket",
    "your-bucket-name",
    "yourbucket",
    "example-bucket",
    "examplebucket",
    "amzn-s3-demo-bucket",
    "doc-example-bucket",
    "s3",
    "www",
    "storage",
    "container",
    "mycontainer",
    "my-container",
    "mystorageaccount",
    "account",
    "upload",
    "download",
    "batch",
    "b",
    "v1",
    "v2",
    "api",
];

fn bucket_ok(v: &str) -> bool {
    let l = v.to_lowercase();
    l.len() >= 3 && !NOT_BUCKETS.contains(&l.as_str()) && !l.bytes().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks() {
        assert!(
            ssn_ok("536-22-1234")
                && !ssn_ok("123-45-6789")
                && !ssn_ok("666-12-3456")
                && !ssn_ok("111-11-1111")
        );
        assert!(
            iban_ok("DE89 3704 0044 0532 0130 00")
                && iban_ok("GB82WEST12345698765432")
                && !iban_ok("DE89370400440532013001")
        );
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(19_723), (2024, 1, 1));
        assert!(
            date_ok("1990-05-12")
                && date_ok("March 5, 1990")
                && date_ok("12/31/1985")
                && !date_ok("1890-01-01")
        );
        let (y, m, d) = today();
        assert!(!date_ok(&format!("{y}-{m}-{d}")));
        assert!(
            mac_ok("3c:22:fb:12:ab:9e")
                && !mac_ok("00:11:22:33:44:55")
                && !mac_ok("12:34:56:78:90:12")
        );
        for ip in [
            "8.8.8.8",
            "10.1.2.3",
            "192.168.1.1",
            "100.64.0.1",
            "203.0.113.5",
            "127.0.0.1",
        ] {
            assert!(!public_ip(ip.parse().unwrap()), "{ip}");
        }
        assert!(
            public_ip("54.23.11.9".parse().unwrap())
                && public_ip("2a00:1450:4001:80b::200e".parse().unwrap())
        );
        assert!(
            !public_ip("2001:db8::1".parse().unwrap()) && !public_ip("fe80::1".parse().unwrap())
        );
        assert!(
            name_alone("cd /Users/developer/x", 10, 19)
                && !name_alone("def run(developer, x)", 8, 17)
        );
    }
}
