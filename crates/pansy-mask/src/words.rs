//! The user's own words: names, codenames, hosts. A word is found as
//! written, or in any case; anywhere, or only as a whole word. A word that
//! is a date matches that calendar date however it is written: the user's
//! birthday 1990-05-12 masks 1990/5/12, 1990年5月12日 and 19900512 too.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use regex::Regex;

use crate::mask::{By, Span};
use crate::options::{Options, WORD_KIND};
use crate::re::compile;

/// Where `s` has the user's words.
pub(crate) fn word_spans(s: &str, o: &Options) -> Vec<Span> {
    let mut found = Vec::new();
    for w in o.words() {
        if let Some(spans) = date_spans(s, w) {
            found.extend(spans);
            continue;
        }
        let re = o.words_any_case.then(|| any_case(w));
        // the next one at or after i; one inside a longer word, where the
        // user asked for whole words, is passed over a letter at a time,
        // so "XABC ABC" still finds the second
        let next = |i: usize| -> Option<(usize, usize)> {
            match &re {
                Some(re) => re.find_at(s, i).map(|m| (m.start(), m.end())),
                None => s[i..].find(w).map(|j| (i + j, i + j + w.len())),
            }
        };
        let mut i = 0;
        while i <= s.len() {
            let Some((a, b)) = next(i) else {
                break;
            };
            if o.words_whole && !whole_word(s, a, b) {
                i = a + s[a..].chars().next().map_or(1, char::len_utf8);
                continue;
            }
            found.push(Span::new(a, b, WORD_KIND, By::Word));
            i = b.max(a + 1);
            while i < s.len() && !s.is_char_boundary(i) {
                i += 1;
            }
        }
    }
    found
}

/// The patterns that find a word in any case, by word.
static ANY_CASE: LazyLock<Mutex<HashMap<String, Regex>>> = LazyLock::new(Default::default);

/// Finds `w` as written but in any case (Unicode's simple folding).
fn any_case(w: &str) -> Regex {
    let mut m = ANY_CASE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(re) = m.get(w) {
        return re.clone();
    }
    if m.len() > 4096 {
        m.clear();
    }
    let re = Regex::new(&format!("(?i){}", regex::escape(w))).expect("a quoted word compiles");
    m.insert(w.to_owned(), re.clone());
    re
}

/// Says `r` is a character of a script that writes no spaces between its
/// words: Chinese and Japanese.
fn spaceless(r: char) -> bool {
    matches!(r as u32,
        0x2E80..=0x2FDF | 0x3005 | 0x3007 | 0x3021..=0x3029 | 0x3038..=0x303B | 0x3040..=0x30FF | 0x31F0..=0x31FF
        | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0xFF66..=0xFF9F | 0x20000..=0x323AF)
}

fn hangul(r: char) -> bool {
    matches!(r as u32, 0x1100..=0x11FF | 0x3130..=0x318F | 0xA960..=0xA97F | 0xAC00..=0xD7AF | 0xD7B0..=0xD7FF)
}

/// Says `r` is a letter, a digit or _ of a script that puts spaces between
/// words. Chinese and Japanese don't, so a word of theirs is found inside
/// their text, and a Latin word right beside a Chinese letter ("用ABC做") is
/// still whole.
fn word_rune(r: char) -> bool {
    r == '_' || (r.is_alphanumeric() && !spaceless(r))
}

/// Says `s[a..b]`, a word the user asked to mask, is a word of its own in
/// `s`. A number is one with its decimals: 123456 is not whole in 0.123456
/// or 123456.78, while "v1." ends a sentence.
pub(crate) fn whole_word(s: &str, a: usize, b: usize) -> bool {
    let w = &s[a..b];
    let (Some(first), Some(last)) = (w.chars().next(), w.chars().next_back()) else {
        return true;
    };
    if word_rune(first) && a > 0 {
        let r = s[..a].chars().next_back().unwrap_or('\0');
        if word_rune(r) {
            return false;
        }
        if first.is_numeric()
            && r == '.'
            && s[..a - 1].chars().next_back().is_some_and(char::is_numeric)
        {
            return false;
        }
    }
    if word_rune(last) && b < s.len() {
        let r = s[b..].chars().next().unwrap_or('\0');
        if word_rune(r) {
            return false;
        }
        if last.is_numeric() && r == '.' && s[b + 1..].chars().next().is_some_and(char::is_numeric)
        {
            return false;
        }
    }
    true
}

/// What stands between a date's numbers.
const DATE_SEP: &str = r"(?:[ \t]*[-/.、／－][ \t]*|[ \t])";

static DATE_YMD: LazyLock<Regex> = LazyLock::new(|| {
    compile(&format!(
        r"^(\d{{4}}){DATE_SEP}(\d{{1,2}}){DATE_SEP}(\d{{1,2}})\.?$"
    ))
});
static DATE_CJK: LazyLock<Regex> = LazyLock::new(|| {
    compile(r"^(\d{4})[ \t]*[年년][ \t]*(\d{1,2})[ \t]*[月월][ \t]*(\d{1,2})[ \t]*[日号號일]?$")
});
static DATE_DMY: LazyLock<Regex> = LazyLock::new(|| {
    compile(&format!(
        r"^(\d{{1,2}}){DATE_SEP}(\d{{1,2}}){DATE_SEP}(\d{{4}})$"
    ))
});
static DATE_NAME_F: LazyLock<Regex> =
    LazyLock::new(|| compile(r"^([A-Za-z]+)\.?[ \t]+(\d{1,2})(?:st|nd|rd|th)?,?[ \t]+(\d{4})$"));
static DATE_NAME_B: LazyLock<Regex> = LazyLock::new(|| {
    compile(r"^(\d{1,2})(?:st|nd|rd|th)?(?:[ \t]+|-)([A-Za-z]+)\.?,?(?:[ \t]+|-)(\d{4})$")
});

const MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// The month a name or its first three letters say, 0 for none.
fn month_of(name: &str) -> u32 {
    let l = name.to_lowercase();
    if l == "sept" {
        return 9;
    }
    for (i, m) in MONTH_NAMES.iter().enumerate() {
        let ml = m.to_lowercase();
        if l == ml || l.len() == 3 && ml.starts_with(&l) {
            return i as u32 + 1;
        }
    }
    0
}

/// Says y-m-d is a day of the calendar.
fn real_date(y: u32, m: u32, d: u32) -> bool {
    if !(1000..=2999).contains(&y) || !(1..=12).contains(&m) || d < 1 {
        return false;
    }
    let leap = y.is_multiple_of(4) && (!y.is_multiple_of(100) || y.is_multiple_of(400));
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    d <= days[m as usize - 1]
}

/// The date the word `w` is, where it is one.
fn parse_date_word(w: &str) -> Option<(u32, u32, u32)> {
    let n = |s: &str| s.parse::<u32>().unwrap_or(0);
    let (y, m, d);
    if let Some(g) = DATE_YMD.captures(w).or_else(|| DATE_CJK.captures(w)) {
        (y, m, d) = (n(&g[1]), n(&g[2]), n(&g[3]));
    } else if let Some(g) = DATE_DMY.captures(w) {
        let (p, q) = (n(&g[1]), n(&g[2]));
        y = n(&g[3]);
        (m, d) = if p == q {
            (p, q)
        } else if p > 12 && q <= 12 {
            (q, p)
        } else if q > 12 && p <= 12 {
            (p, q)
        } else {
            return None; // 05/12/1990: May 12th or December 5th
        };
    } else if let Some(g) = DATE_NAME_F.captures(w) {
        (m, d, y) = (month_of(&g[1]), n(&g[2]), n(&g[3]));
    } else {
        let g = DATE_NAME_B.captures(w)?;
        (d, m, y) = (n(&g[1]), month_of(&g[2]), n(&g[3]));
    }
    real_date(y, m, d).then_some((y, m, d))
}

/// A pattern for the date y-m-d in each form a date word matches, and the
/// word itself as written. Day-first and month-first only where no other
/// date reads the same.
fn date_forms(y: u32, m: u32, d: u32, w: &str) -> String {
    let num = |v: u32| {
        if v < 10 {
            format!("0?{v}")
        } else {
            v.to_string()
        }
    };
    let (yy, mm, dd) = (y.to_string(), num(m), num(d));
    let full = MONTH_NAMES[m as usize - 1];
    let abbr = if m == 9 {
        "Sept|Sep".to_owned()
    } else {
        full[..3].to_owned()
    };
    let name = format!(r"(?i:{full}|(?:{abbr})\.?)");
    let ord = "(?:st|nd|rd|th)?";
    let mut forms = vec![
        format!("{yy}{DATE_SEP}{mm}{DATE_SEP}{dd}"),
        format!(r"{yy}[ \t]*年[ \t]*{mm}[ \t]*月[ \t]*{dd}(?:[ \t]*[日号號])?"),
        format!(r"{yy}[ \t]*년[ \t]*{mm}[ \t]*월[ \t]*{dd}(?:[ \t]*일)?"),
        format!("{yy}{m:02}{d:02}"),
        format!(r"{name}[ \t]+{dd}{ord},?[ \t]+{yy}"),
        format!(r"{dd}{ord}(?:[ \t]+|-){name},?(?:[ \t]+|-){yy}"),
    ];
    if d > 12 || d == m {
        forms.push(format!("{dd}{DATE_SEP}{mm}{DATE_SEP}{yy}"));
        if d != m {
            forms.push(format!("{mm}{DATE_SEP}{dd}{DATE_SEP}{yy}"));
        }
    }
    format!("(?:{}|{})", forms.join("|"), regex::escape(w))
}

/// The pattern for the date each masked word is, None where it is none.
static DATE_WORDS: LazyLock<Mutex<HashMap<String, Option<Regex>>>> =
    LazyLock::new(Default::default);

fn date_word(w: &str) -> Option<Regex> {
    let mut m = DATE_WORDS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(re) = m.get(w) {
        return re.clone();
    }
    if m.len() > 4096 {
        m.clear();
    }
    let re = parse_date_word(w).map(|(y, mo, d)| compile(&date_forms(y, mo, d, w)));
    m.insert(w.to_owned(), re.clone());
    re
}

/// Says the date at `s[a..b]` is no part of a longer number: no digit beside
/// it, no dot and a digit (a version's 1990.5.12.1), and no letter before a
/// month's name (xMay), save in a script that puts no spaces between words.
fn date_whole(s: &str, a: usize, b: usize) -> bool {
    let bs = s.as_bytes();
    if a > 0 {
        if bs[a - 1].is_ascii_digit() || bs[a - 1] == b'.' && a > 1 && bs[a - 2].is_ascii_digit() {
            return false;
        }
        let r = s[..a].chars().next_back().unwrap_or('\0');
        let first = s[a..].chars().next().unwrap_or('\0');
        if first.is_alphabetic() && r.is_alphabetic() && !spaceless(r) && !hangul(r) {
            return false;
        }
    }
    if b < s.len()
        && (bs[b].is_ascii_digit()
            || bs[b] == b'.' && b + 1 < s.len() && bs[b + 1].is_ascii_digit())
    {
        return false;
    }
    true
}

/// Where `s` has the date the masked word `w` is, None where `w` is no date.
fn date_spans(s: &str, w: &str) -> Option<Vec<Span>> {
    let re = date_word(w)?;
    Some(
        re.find_iter(s)
            .filter(|m| date_whole(s, m.start(), m.end()))
            .map(|m| Span::new(m.start(), m.end(), WORD_KIND, By::Word))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found<'a>(s: &'a str, o: &Options) -> Vec<&'a str> {
        word_spans(s, o)
            .iter()
            .map(|f| &s[f.start..f.end])
            .collect()
    }

    #[test]
    fn whole_and_any_case() {
        let o = Options {
            words: vec!["ABC".into()],
            words_whole: true,
            ..Options::default()
        };
        assert_eq!(
            found("XABC ABC 'ABC' ABC IS XABCD 用ABC做", &o),
            ["ABC", "ABC", "ABC", "ABC"]
        );
        let o = Options {
            words: vec!["123456".into()],
            words_whole: true,
            ..Options::default()
        };
        assert!(found("91234567 0.123456 123456.78", &o).is_empty());
        assert_eq!(found("pin 123456.", &o), ["123456"]);
        let o = Options {
            words: vec!["Nightjar".into()],
            words_any_case: true,
            ..Options::default()
        };
        assert_eq!(found("NIGHTJAR nightjar Nightjar", &o).len(), 3);
        let o = Options {
            words: vec!["Nightjar".into()],
            ..Options::default()
        };
        assert_eq!(found("NIGHTJAR nightjar Nightjar", &o), ["Nightjar"]);
    }

    #[test]
    fn dates() {
        let o = Options {
            words: vec!["1990-05-12".into()],
            ..Options::default()
        };
        assert_eq!(
            found(
                "1990/5/12, 1990年5月12日, 19900512, May 12, 1990, 12 May 1990 but not 1990.5.12.1 or 05/12/1990",
                &o
            ),
            [
                "1990/5/12",
                "1990年5月12日",
                "19900512",
                "May 12, 1990",
                "12 May 1990"
            ]
        );
        assert_eq!(parse_date_word("05/12/1990"), None);
        assert_eq!(parse_date_word("13/05/1990"), Some((1990, 5, 13)));
        assert_eq!(parse_date_word("1990-02-30"), None);
        assert_eq!(parse_date_word("19900512"), None);
    }
}
