//! Patterns as magpie wrote them, for RE2. Rust's regex reads nearly the
//! same syntax, but its `\d`, `\s`, `\w` and `\b` are Unicode's, where
//! RE2's are ASCII's: a Chinese letter is no word character to RE2, so
//! `\bssn` still starts a word after one. [`go`] writes the ASCII classes
//! out, and [`Pat`] compiles a pattern the first time it is used.

use std::sync::OnceLock;

use regex::Regex;

/// A pattern, compiled when first used.
pub(crate) struct Pat {
    src: String,
    re: OnceLock<Regex>,
}

impl Pat {
    pub(crate) fn new(src: impl Into<String>) -> Pat {
        Pat {
            src: src.into(),
            re: OnceLock::new(),
        }
    }

    /// One already compiled.
    pub(crate) fn of(re: Regex) -> Pat {
        let cell = OnceLock::new();
        let _ = cell.set(re.clone());
        Pat {
            src: re.as_str().to_owned(),
            re: cell,
        }
    }

    pub(crate) fn get(&self) -> &Regex {
        self.re.get_or_init(|| compile(&self.src))
    }
}

/// Compiles a pattern of magpie's, which is known to be good.
pub(crate) fn compile(src: &str) -> Regex {
    match Regex::new(&go(src)) {
        Ok(re) => re,
        Err(e) => panic!("pattern {src:?}: {e}"),
    }
}

/// Compiles a pattern the user wrote, as RE2 reads it.
pub(crate) fn user(src: &str) -> Result<Regex, regex::Error> {
    regex::RegexBuilder::new(&go(src))
        .size_limit(1 << 22)
        .build()
}

/// A pattern of RE2's with its Perl classes and word boundaries made ASCII
/// for Rust's regex: `\d` is `[0-9]`, `\s` is `[\t\n\f\r ]`, `\w` is
/// `[0-9A-Za-z_]`, `\b` is `(?-u:\b)`, inside a class or out of one.
pub(crate) fn go(src: &str) -> String {
    let mut out = String::with_capacity(src.len() + 16);
    let mut depth = 0usize; // classes open, Rust's nested ones counted
    let mut chars = src.chars().peekable();
    let mut class_start = false; // right after [ or [^, where ] is literal
    while let Some(c) = chars.next() {
        let at_start = class_start;
        class_start = false;
        match c {
            '\\' => {
                let Some(e) = chars.next() else {
                    out.push('\\');
                    break;
                };
                let set = match e {
                    'd' | 'D' => "0-9",
                    's' | 'S' => r"\t\n\x0C\r ",
                    'w' | 'W' => "0-9A-Za-z_",
                    'b' | 'B' if depth == 0 => {
                        out.push_str(if e == 'b' { r"(?-u:\b)" } else { r"(?-u:\B)" });
                        continue;
                    }
                    _ => {
                        out.push('\\');
                        out.push(e);
                        continue;
                    }
                };
                let neg = e.is_ascii_uppercase();
                if depth > 0 && !neg {
                    out.push_str(set);
                } else {
                    out.push('[');
                    if neg {
                        out.push('^');
                    }
                    out.push_str(set);
                    out.push(']');
                }
            }
            '[' => {
                if depth > 0 && chars.peek() == Some(&':') {
                    // a POSIX class, [:alpha:], copied to its :]
                    out.push('[');
                    for p in chars.by_ref() {
                        out.push(p);
                        if p == ']' {
                            break;
                        }
                    }
                    continue;
                }
                depth += 1;
                out.push('[');
                if chars.peek() == Some(&'^') {
                    out.push('^');
                    chars.next();
                }
                class_start = true;
            }
            ']' if depth > 0 && !at_start => {
                depth -= 1;
                out.push(']');
            }
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_classes() {
        assert_eq!(go(r"\d{3}[\dXx]"), r"[0-9]{3}[0-9Xx]");
        assert_eq!(go(r"[^\s/]"), r"[^\t\n\x0C\r /]");
        assert_eq!(go(r"[\s\S]+?"), r"[\t\n\x0C\r [^\t\n\x0C\r ]]+?");
        assert_eq!(go(r"\bssn"), r"(?-u:\b)ssn");
        assert_eq!(go(r"[]a]\d"), r"[]a][0-9]");
        let re = compile(r"(?i:\bssn)\s*(\d{3})");
        assert!(re.is_match("社保ssn 123"));
        assert!(!re.is_match("xssn 123"));
        assert!(!compile(r"\d").is_match("٣"));
    }
}
