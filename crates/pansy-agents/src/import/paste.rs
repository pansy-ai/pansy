//! What a paste is read as, magpie's `curlOf` and `envOf`: a curl command
//! from a vendor's docs (@mark_li21 on X), or the environment lines a
//! vendor's page or a README gives for its API (WJaunita45604 on X). Either
//! gives the base URLs by protocol, the key (or the stand-in a page shows
//! for one), a model, and a name from the site. Nothing is kept or sent:
//! the window fills a custom provider's form with it.

use serde::Serialize;

use pansy_core::provider::Protocol;

/// What a pasted text gives.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pasted {
    /// `curl` or `env`.
    pub kind: &'static str,
    /// Base URLs by protocol, the first the one the text is mostly about.
    pub bases: Vec<Base>,
    /// The API key, when it is a key of the user's own.
    pub key: String,
    /// A key that only stands for one (`$OPENAI_API_KEY`, `YOUR_API_KEY`).
    pub placeholder: String,
    /// The model the text names.
    pub model: String,
    /// What to call the provider: the name in the first base URL's host.
    pub name: String,
    /// A part of the base URL to replace (`{WorkspaceId}`).
    pub url_hole: String,
}

/// One base URL and its protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Base {
    pub protocol: Protocol,
    pub url: String,
}

/// What `text` is read as: a curl command, else environment lines; `None`
/// for neither.
pub fn read(text: &str) -> Option<Pasted> {
    curl(text).or_else(|| env(text))
}

/// A pasted curl command: the base URL from the request's URL with its
/// endpoint cut off, the key from its auth header, the model from its JSON
/// body. Bash's `\`, cmd's `^` and PowerShell's `` ` `` continuations and
/// quoting are read; `None` if it isn't curl.
pub fn curl(text: &str) -> Option<Pasted> {
    let s = text.trim();
    let lower = s.to_ascii_lowercase();
    let head = ["curl.exe", "curl"]
        .into_iter()
        .find(|h| lower.starts_with(h) && s[h.len()..].starts_with(char::is_whitespace))?;
    let cmd = continued(s, '^');
    let ps = continued(s, '`') || head == "curl.exe";
    let s = join_lines(&s[head.len()..]);
    let words = words(&s, cmd, ps);

    // the options curl reads a value after; the others stand alone
    const VALUED: [&str; 35] = [
        "-H",
        "-d",
        "-X",
        "-u",
        "-o",
        "-A",
        "-e",
        "-F",
        "-b",
        "-c",
        "-m",
        "-T",
        "--header",
        "--data",
        "--data-raw",
        "--data-binary",
        "--data-ascii",
        "--data-urlencode",
        "--json",
        "--request",
        "--url",
        "--user",
        "--output",
        "--user-agent",
        "--referer",
        "--form",
        "--cookie",
        "--max-time",
        "--connect-timeout",
        "--proxy",
        "--upload-file",
        "--retry",
        "--cert",
        "--key",
        "--cacert",
    ];
    let mut url = String::new();
    let mut body = String::new();
    let mut headers = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let x = &words[i];
        i += 1;
        let (opt, val) = if (x.starts_with("-H") || x.starts_with("-d")) && x.len() > 2 {
            (&x[..2], x[2..].to_owned())
        } else if VALUED.contains(&x.as_str()) {
            let v = words.get(i).cloned().unwrap_or_default();
            i += 1;
            (x.as_str(), v)
        } else if x.starts_with('-') {
            continue;
        } else {
            if url.is_empty() {
                url = x.clone();
            }
            continue;
        };
        match opt {
            "-H" | "--header" => headers.push(val),
            "--url" => url = val,
            "-d" | "--data" | "--data-raw" | "--data-binary" | "--data-ascii" | "--json"
                if !val.starts_with('@') =>
            {
                body = val
            }
            _ => {}
        }
    }

    let mut out = Pasted {
        kind: "curl",
        ..Pasted::default()
    };
    if let Some((protocol, base)) = endpoint_of(&url) {
        let base = respell(&base, protocol);
        out.url_hole = hole(&base);
        out.name = site_name(&base);
        out.bases.push(Base {
            protocol,
            url: base,
        });
    }
    for h in &headers {
        let Some((name, v)) = h.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        if !matches!(name.as_str(), "authorization" | "x-api-key" | "api-key") {
            continue;
        }
        let mut v = v.trim();
        if name == "authorization" && v.len() > 7 && v[..7].eq_ignore_ascii_case("bearer ") {
            v = v[7..].trim_start();
        }
        if v.is_empty() {
            continue;
        }
        if stands_in(v) {
            out.placeholder = v.to_owned();
        } else {
            out.key = v.to_owned();
        }
        break;
    }
    // a body for PowerShell 5 often escapes its quotes for curl.exe:
    // '{\"model\": …}'
    for b in [body.clone(), body.replace("\\\"", "\"")] {
        if let Ok(j) = serde_json::from_str::<serde_json::Value>(&b) {
            if let Some(m) = j.get("model").and_then(|m| m.as_str()) {
                out.model = m.trim().to_owned();
            }
            break;
        }
    }
    Some(out)
}

/// Whether `s` has lines continued with `mark` (`^` in cmd, `` ` `` in
/// PowerShell).
fn continued(s: &str, mark: char) -> bool {
    let lines: Vec<&str> = s.split('\n').collect();
    lines[..lines.len() - 1].iter().any(|line| {
        line.trim_end_matches('\r')
            .trim_end_matches([' ', '\t'])
            .ends_with(mark)
    })
}

/// The command on one line: a `\`, `^` or `` ` `` ending a line, and the
/// line break after it, become a space.
fn join_lines(s: &str) -> String {
    let lines: Vec<&str> = s.split('\n').collect();
    let mut out = String::new();
    for (n, line) in lines.iter().enumerate() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let last = n + 1 == lines.len();
        let bare = line.trim_end_matches([' ', '\t']);
        match bare.chars().last() {
            Some('\\' | '^' | '`') if !last => {
                out.push_str(&bare[..bare.len() - 1]);
                out.push(' ');
            }
            _ => {
                out.push_str(line);
                if !last {
                    out.push('\n');
                }
            }
        }
    }
    out
}

/// The words as the shell hands them to curl: quotes taken off, escapes read.
fn words(s: &str, cmd: bool, ps: bool) -> Vec<String> {
    let c: Vec<char> = s.chars().collect();
    let mut words = Vec::new();
    let mut w: Option<String> = None;
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            if let Some(word) = w.take() {
                words.push(word);
            }
            i += 1;
            continue;
        }
        let word = w.get_or_insert_with(String::new);
        if ch == '\'' {
            // PowerShell doubles a quote inside single quotes
            let mut j = i + 1;
            loop {
                match c[j.min(c.len())..].iter().position(|&x| x == '\'') {
                    None => {
                        word.extend(&c[j.min(c.len())..]);
                        i = c.len();
                        break;
                    }
                    Some(k) => {
                        let k = j + k;
                        if ps && c.get(k + 1) == Some(&'\'') {
                            word.extend(&c[j..k]);
                            word.push('\'');
                            j = k + 2;
                            continue;
                        }
                        word.extend(&c[j..k]);
                        i = k + 1;
                        break;
                    }
                }
            }
        } else if ch == '"' {
            i += 1;
            while i < c.len() && c[i] != '"' {
                let esc = (c[i] == '\\' && c.get(i + 1).is_some_and(|n| "\"\\$`".contains(*n)))
                    || (ps && c[i] == '`');
                if esc {
                    i += 1;
                }
                if let Some(&x) = c.get(i) {
                    word.push(x);
                }
                i += 1;
            }
            i += 1;
        } else if (ch == '\\' && !cmd && !ps) || (ch == '^' && cmd) {
            if let Some(&x) = c.get(i + 1) {
                word.push(x);
            }
            i += 2;
        } else {
            word.push(ch);
            i += 1;
        }
    }
    if let Some(word) = w {
        words.push(word);
    }
    words
}

/// The protocol and base URL of a request's URL ending in
/// `/chat/completions`, `/messages` or `/responses`.
fn endpoint_of(url: &str) -> Option<(Protocol, String)> {
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return None;
    }
    let url = &url[..url.find(['?', '#']).unwrap_or(url.len())];
    let url = url.trim_end_matches('/');
    let lower = url.to_ascii_lowercase();
    [
        ("/chat/completions", Protocol::Chat),
        ("/messages", Protocol::Anthropic),
        ("/responses", Protocol::Responses),
    ]
    .into_iter()
    .find(|(end, _)| lower.ends_with(end))
    .map(|(end, p)| {
        (
            p,
            url[..url.len() - end.len()]
                .trim_end_matches('/')
                .to_owned(),
        )
    })
    .filter(|(_, base)| {
        base.split_once("://")
            .is_some_and(|(_, rest)| !rest.is_empty())
    })
}

/// Whether a key only stands for one: `$KEY`, `%KEY%`, `<token>`,
/// `{{key}}`, `[key]`, `YOUR_API_KEY`, `sk-xxxxxxxx`, `sk-...`.
fn stands_in(v: &str) -> bool {
    let l = v.to_ascii_lowercase();
    let wrapped =
        |a: &str, b: &str| l.starts_with(a) && l.ends_with(b) && l.len() >= a.len() + b.len();
    l.starts_with('$')
        || (wrapped("%", "%")
            && l.len() > 2
            && l[1..l.len() - 1]
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_'))
        || wrapped("<", ">")
        || wrapped("{{", "}}")
        || wrapped("[", "]")
        || l.contains("your")
        || ["api_key", "api-key", "apikey"]
            .iter()
            .any(|y| l.contains(y))
        || l.contains("xxxxxx")
        || l.contains("...")
        || l.contains('…')
}

/// The first part of a base URL to fill in: `{WorkspaceId}`, `<region>`,
/// `$REGION`, `${REGION}`, `%REGION%`.
fn hole(base: &str) -> String {
    let c: Vec<char> = base.chars().collect();
    for i in 0..c.len() {
        let close = match c[i] {
            '{' => Some('}'),
            '<' => Some('>'),
            '%' => Some('%'),
            '$' => None,
            _ => continue,
        };
        match close {
            Some(close) => {
                if let Some(k) = c[i + 1..].iter().position(|&x| x == close) {
                    let inner = &c[i + 1..i + 1 + k];
                    if close != '%'
                        || (!inner.is_empty()
                            && inner.iter().all(|x| x.is_alphanumeric() || *x == '_'))
                    {
                        return c[i..i + 2 + k].iter().collect();
                    }
                }
            }
            None => {
                let mut j = i + 1;
                let braced = c.get(j) == Some(&'{');
                if braced {
                    j += 1;
                }
                let start = j;
                while j < c.len() && (c[j].is_alphanumeric() || c[j] == '_') {
                    j += 1;
                }
                if j > start {
                    if braced && c.get(j) == Some(&'}') {
                        j += 1;
                    }
                    return c[i..j].iter().collect();
                }
            }
        }
    }
    String::new()
}

/// A URL as its protocol wants it: Anthropic's without `/v1`, Gemini's
/// at `/v1beta`, OpenAI's at a version (`/v1` on a bare host).
fn respell(u: &str, protocol: Protocol) -> String {
    let u = with_scheme(u);
    let u = u.trim_end_matches('/');
    let bare = |u: &str| {
        u.split_once("://")
            .is_some_and(|(_, rest)| !rest.is_empty() && !rest.contains('/'))
    };
    match protocol {
        Protocol::Anthropic => u.strip_suffix("/v1").unwrap_or(u).to_owned(),
        Protocol::Gemini => match u.strip_suffix("/v1") {
            Some(root) if bare(root) => format!("{u}beta"),
            _ if bare(u) => format!("{u}/v1beta"),
            _ => u.to_owned(),
        },
        Protocol::Chat | Protocol::Responses => {
            let last = u.rsplit('/').next().unwrap_or("");
            let versioned = last.len() > 1
                && last.starts_with('v')
                && last[1..]
                    .trim_end_matches(|c: char| c.is_ascii_lowercase())
                    .chars()
                    .all(|c| c.is_ascii_digit())
                && last[1..].starts_with(|c: char| c.is_ascii_digit());
            if versioned || !bare(u) {
                u.to_owned()
            } else {
                format!("{u}/v1")
            }
        }
    }
}

/// `https://https://x` → `https://x`, and the scheme in lowercase.
fn with_scheme(u: &str) -> String {
    let mut u = u.trim();
    let mut scheme = "";
    loop {
        let l = u.to_ascii_lowercase();
        if l.starts_with("https://") {
            (scheme, u) = ("https", &u[8..]);
        } else if l.starts_with("http://") {
            (scheme, u) = ("http", &u[7..]);
        } else {
            break;
        }
    }
    if scheme.is_empty() {
        u.to_owned()
    } else {
        format!("{scheme}://{u}")
    }
}

/// What to call the provider at a URL: the name in its host,
/// `api.moonshot.cn` → Moonshot, `open.bigmodel.cn` → Bigmodel; an
/// address stays as it is.
pub fn site_name(u: &str) -> String {
    let Some(host) = u
        .split_once("://")
        .map(|(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or(""))
        .map(|h| {
            h.rsplit_once('@')
                .map_or(h, |(_, h)| h)
                .to_ascii_lowercase()
        })
        // the port is no part of the name
        .map(|h| match h.rsplit_once(':') {
            Some((name, port))
                if !h.starts_with('[') && port.chars().all(|c| c.is_ascii_digit()) =>
            {
                name.to_owned()
            }
            _ => h,
        })
    else {
        return String::new();
    };
    if host.is_empty() {
        return String::new();
    }
    if host.chars().all(|c| c.is_ascii_digit() || c == '.')
        || host.contains(':')
        || host == "localhost"
    {
        return host;
    }
    let mut l: Vec<&str> = host.split('.').collect();
    if l.len() > 1 {
        l.pop(); // the top-level domain
    }
    if l.len() > 1
        && matches!(
            *l.last().unwrap(),
            "co" | "com" | "org" | "net" | "ac" | "gov" | "edu"
        )
    {
        l.pop();
    }
    let generic = |x: &&str| matches!(*x, "www" | "api" | "open" | "openapi" | "gateway" | "llm");
    let w = l
        .iter()
        .rfind(|x| !generic(x))
        .or(l.last())
        .copied()
        .unwrap_or(&host);
    let mut c = w.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

/// Environment lines: `export ANTHROPIC_BASE_URL="…"`, `set` and `setx`,
/// `$env:NAME = "…"`, Docker's `ENV`, `NAME=value` in a `.env` file, or a
/// JSON object of them (Claude Code's settings.json `env`). The names say
/// the protocol: `ANTHROPIC_*` Anthropic's, `OPENAI_*` OpenAI's chat,
/// `GEMINI_*` Gemini's. `None` for text with none of those names.
pub fn env(text: &str) -> Option<Pasted> {
    let body = text.trim();
    let mut vars: Vec<(String, String)> = Vec::new();
    let mut put = |k: &str, v: &str| {
        let v = v.trim();
        if !v.is_empty() && !vars.iter().any(|(x, _)| x == k) {
            vars.push((k.to_owned(), v.to_owned()));
        }
    };
    match serde_json::from_str::<serde_json::Value>(body) {
        Ok(json @ (serde_json::Value::Object(_) | serde_json::Value::Array(_))) => {
            walk(&json, 0, &mut put)
        }
        _ => {
            for line in body.lines() {
                if let Some((k, v)) = assignment(line) {
                    put(&k, &v);
                }
            }
        }
    }

    let mut out = Pasted {
        kind: "env",
        ..Pasted::default()
    };
    for (k, v) in &vars {
        let up = k.to_ascii_uppercase();
        let family = |names: &[&str], ends: &[&str]| {
            names.iter().any(|n| {
                up.strip_prefix(n)
                    .and_then(|rest| rest.strip_prefix('_'))
                    .is_some_and(|rest| ends.contains(&rest))
            })
        };
        if family(
            &["ANTHROPIC", "OPENAI", "GEMINI", "GOOGLE_GEMINI"],
            &["BASE_URL", "API_BASE", "API_URL", "BASE", "ENDPOINT"],
        ) {
            let protocol = if up.starts_with("ANTHROPIC") {
                Protocol::Anthropic
            } else if up.starts_with("OPENAI") {
                Protocol::Chat
            } else {
                Protocol::Gemini
            };
            // a host written without its scheme is https
            let u = if has_scheme(v) {
                v.clone()
            } else if looks_like_host(v) {
                format!("https://{v}")
            } else {
                continue;
            };
            if out.bases.iter().any(|b| b.protocol == protocol) {
                continue;
            }
            // …/v1/chat/completions given whole is cut back to the base
            let mut cut = u.trim_end_matches('/').to_owned();
            let lower = cut.to_ascii_lowercase();
            for end in ["/chat/completions", "/messages", "/responses"] {
                if lower.ends_with(end) {
                    cut.truncate(cut.len() - end.len());
                    break;
                }
            }
            out.bases.push(Base {
                protocol,
                url: respell(&cut, protocol),
            });
        } else if family(
            &["ANTHROPIC", "OPENAI", "GEMINI", "GOOGLE"],
            &["AUTH_TOKEN", "API_KEY"],
        ) {
            if !out.key.is_empty() || !out.placeholder.is_empty() {
                continue;
            }
            if stands_in(v) {
                out.placeholder = v.clone();
            } else {
                out.key = v.clone();
            }
        } else if family(&["ANTHROPIC", "OPENAI", "GEMINI"], &["MODEL"]) && out.model.is_empty() {
            out.model = v.clone();
        }
    }
    if out.bases.is_empty() && out.key.is_empty() && out.placeholder.is_empty() {
        return None;
    }
    if let Some(first) = out.bases.first() {
        out.url_hole = hole(&first.url);
        out.name = site_name(&first.url);
    }
    Some(out)
}

fn walk(v: &serde_json::Value, depth: usize, put: &mut impl FnMut(&str, &str)) {
    if depth > 3 {
        return;
    }
    let mut each = |k: &str, v: &serde_json::Value| match v {
        serde_json::Value::String(s) => put(k, s),
        _ => walk(v, depth + 1, put),
    };
    match v {
        serde_json::Value::Object(o) => o.iter().for_each(|(k, v)| each(k, v)),
        serde_json::Value::Array(a) => a
            .iter()
            .enumerate()
            .for_each(|(i, v)| each(&i.to_string(), v)),
        _ => {}
    }
}

/// One line's `NAME=value`, with the shells' ways of setting it: `export`,
/// `set`, `setx`, `env`, `$env:`, quotes, a trailing `;` or `# comment`.
fn assignment(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    let lower = line.to_ascii_lowercase();
    let prefixed = ["export", "set", "setx", "env"]
        .into_iter()
        .find(|p| lower.starts_with(p) && line[p.len()..].starts_with(char::is_whitespace))
        .map(|p| line[p.len()..].trim_start())
        .or_else(|| lower.starts_with("$env:").then(|| &line[5..]));
    prefixed.and_then(name_value).or_else(|| name_value(line))
}

fn name_value(s: &str) -> Option<(String, String)> {
    let end = s
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(s.len());
    let name = &s[..end];
    if name.is_empty() || name.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let rest = &s[end..];
    let after = rest.trim_start();
    let rest = match after.strip_prefix('=') {
        Some(r) => r,
        // a space alone sets it too (Docker's ENV NAME value)
        None if rest.len() > after.len() => after,
        None => return None,
    };
    let mut v = rest.trim();
    v = v.strip_suffix(';').unwrap_or(v).trim_end();
    let quoted = v.len() >= 2
        && (v.starts_with('"') && v.ends_with('"') || v.starts_with('\'') && v.ends_with('\''));
    let v = if quoted {
        v[1..v.len() - 1].to_owned()
    } else {
        // a comment after the value
        match v.find(" #").or_else(|| v.find("\t#")) {
            Some(i) => v[..i].trim_end().to_owned(),
            None => v.to_owned(),
        }
    };
    Some((name.to_owned(), v))
}

fn has_scheme(v: &str) -> bool {
    v.split_once("://").is_some_and(|(s, _)| {
        s.starts_with(|c: char| c.is_ascii_alphabetic())
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
    })
}

fn looks_like_host(v: &str) -> bool {
    let first = v.split('/').next().unwrap_or("");
    (first.contains('.')
        && !first.starts_with('.')
        && !first.ends_with('.')
        && !first.contains(char::is_whitespace))
        || v.to_ascii_lowercase().starts_with("localhost")
}
