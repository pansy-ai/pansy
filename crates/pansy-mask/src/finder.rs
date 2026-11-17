//! A finder is one rule: a pattern for a kind of value, and the checks a
//! match has to pass to be one. The match is group 1 when the pattern has
//! one, else all of it.

use std::sync::LazyLock;

use crate::mask::{By, Span};
use crate::options::Options;
use crate::re::Pat;
use crate::text::{ceil, contains_one, floor, is_in, lower_ascii};

/// Checks a value.
pub(crate) type Check = fn(&str) -> bool;
/// Checks the value at `s[a..b]` by what is around it.
pub(crate) type AtCheck = fn(&str, usize, usize) -> bool;
/// Checks the value at `s[a..b]` and may shorten it.
pub(crate) type Fix = fn(&str, usize, usize) -> Option<(usize, usize)>;

/// A match's groups, the whole match first, as places in the text.
pub(crate) type Groups = Vec<Option<(usize, usize)>>;

pub(crate) struct Finder {
    pub kind: String,
    pub re: Pat,
    /// one of these is in any text it can match
    pub markers: Vec<String>,
    /// markers too, looked for in the text in lower case, for a pattern
    /// that matches its words in any case (Passport, PASSPORT)
    pub fold: Vec<String>,
    /// the characters that may not touch the value on either side: a key
    /// inside a longer word is no key
    pub bound: &'static str,
    pub ok: Option<Check>,
    /// checks the value by what is around it: a dotted quad right after
    /// "version" is no address
    pub at: Option<AtCheck>,
    /// checks the value in place of ok, and may shorten it
    pub fix: Option<Fix>,
    /// how far before and after one of fold a match can be, where ahead is
    /// set: the pattern is only run there, as one that takes its words in
    /// any case is slow over all of a long text
    pub back: usize,
    pub ahead: usize,
    /// what turns it on (Options::covers): "" the secrets
    pub cat: &'static str,
    /// takes a value after a field's name and = or :, a space allowed
    /// between them: a value that is the next field's assignment is not
    /// this one's (field_matches)
    pub field: bool,
    /// a rule the user wrote
    pub yours: bool,
    /// takes the first group that matched, for a pattern with a group in
    /// each of its alternatives
    pub any_group: bool,
    /// the kinds of a match's groups, each masked as a value of its own
    /// (the user and the host of user@host), with each one's check
    pub groups: Vec<(&'static str, Check)>,
    /// a cheap scan that is true of any text the pattern can match, for a
    /// rule with no marker to look for
    pub may: Option<Check>,
}

impl Finder {
    /// A finder of `kind` by `pattern`, with nothing else set.
    pub fn new(kind: &str, pattern: impl Into<String>) -> Finder {
        Finder {
            kind: kind.to_owned(),
            re: Pat::new(pattern),
            markers: Vec::new(),
            fold: Vec::new(),
            bound: "",
            ok: None,
            at: None,
            fix: None,
            back: 0,
            ahead: 0,
            cat: "",
            field: false,
            yours: false,
            any_group: false,
            groups: Vec::new(),
            may: None,
        }
    }

    pub fn markers(mut self, m: &[&str]) -> Finder {
        self.markers = m.iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn fold(mut self, f: &[&str]) -> Finder {
        self.fold = f.iter().map(|s| s.to_lowercase()).collect();
        self
    }

    pub fn window(mut self, back: usize, ahead: usize) -> Finder {
        self.back = back;
        self.ahead = ahead;
        self
    }

    pub fn bound(mut self, b: &'static str) -> Finder {
        self.bound = b;
        self
    }

    pub fn ok(mut self, f: Check) -> Finder {
        self.ok = Some(f);
        self
    }

    pub fn at(mut self, f: AtCheck) -> Finder {
        self.at = Some(f);
        self
    }

    pub fn fix(mut self, f: Fix) -> Finder {
        self.fix = Some(f);
        self
    }

    pub fn cat(mut self, c: &'static str) -> Finder {
        self.cat = c;
        self
    }

    pub fn field(mut self) -> Finder {
        self.field = true;
        self
    }

    pub fn any_group(mut self) -> Finder {
        self.any_group = true;
        self
    }

    pub fn groups(mut self, g: &[(&'static str, Check)]) -> Finder {
        self.groups = g.to_vec();
        self
    }

    pub fn may(mut self, f: Check) -> Finder {
        self.may = Some(f);
        self
    }

    fn by(&self) -> By {
        if self.yours { By::Yours } else { By::Rule }
    }

    /// The matches in `s`, `lower` being `s` in lower case: in all of
    /// `s`, or, for a finder with ahead set, only around its fold markers.
    fn matches(&self, s: &str, lower: &str) -> Vec<Groups> {
        if self.ahead == 0 {
            return self.find_all(s);
        }
        let b = s.as_bytes();
        let mut wins: Vec<(usize, usize)> = Vec::new();
        for f in &self.fold {
            let mut i = 0;
            while let Some(j) = lower[i..].find(f.as_str()) {
                let at = i + j;
                i = at + f.len();
                // from the line's start, or from where a word starts no
                // further back than back: what the pattern takes for the
                // text's start is where a line or a word starts in s too
                let mut start = at.saturating_sub(self.back);
                if let Some(nl) = b[start..at].iter().rposition(|&c| c == b'\n') {
                    start += nl + 1;
                } else if start > 0 {
                    while start < at
                        && (is_in(
                            "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_.-",
                            b[start - 1],
                        ) || b[start] & 0xc0 == 0x80)
                    {
                        start += 1;
                    }
                    if start == at && crate::text::word_byte(b[at - 1]) {
                        continue; // inside a longer word
                    }
                }
                let end = floor(s, (i + self.ahead).min(s.len()));
                wins.push((ceil(s, start), end.max(i)));
            }
        }
        wins.sort();
        let mut out = Vec::new();
        let mut k = 0;
        while k < wins.len() {
            let mut w = wins[k];
            while k + 1 < wins.len() && wins[k + 1].0 <= w.1 {
                k += 1;
                w.1 = w.1.max(wins[k].1);
            }
            for mut m in self.find_all(&s[w.0..w.1]) {
                for g in m.iter_mut().flatten() {
                    g.0 += w.0;
                    g.1 += w.0;
                }
                out.push(m);
            }
            k += 1;
        }
        out
    }

    /// The matches in `s`: each of them, or, for a finder that reads a
    /// field's value after its name, none whose value is the next field.
    fn find_all(&self, s: &str) -> Vec<Groups> {
        if self.field {
            return field_matches(self.re.get(), s);
        }
        captures(self.re.get(), s, 0)
    }

    /// Says `s[a..b]` is a value this finder masks: nothing it may not
    /// touch beside it, and its checks pass.
    fn takes(&self, s: &str, a: usize, b: usize, ok: Option<Check>) -> bool {
        if b <= a {
            return false;
        }
        let bytes = s.as_bytes();
        if !self.bound.is_empty()
            && (a > 0 && is_in(self.bound, bytes[a - 1])
                || b < s.len() && is_in(self.bound, bytes[b]))
        {
            return false;
        }
        if let Some(ok) = ok
            && !ok(&s[a..b])
        {
            return false;
        }
        self.at.is_none_or(|at| at(s, a, b))
    }
}

/// The matches of `re` in `s[from..]`, as places in `s`.
pub(crate) fn captures(re: &regex::Regex, s: &str, from: usize) -> Vec<Groups> {
    re.captures_iter(&s[from..])
        .map(|c| {
            c.iter()
                .map(|g| g.map(|g| (g.start() + from, g.end() + from)))
                .collect()
        })
        .collect()
}

/// The matches of `re` in `s`, but where a match's value (group 1) is the
/// next field (`next_field`) the match is dropped and the search goes on
/// from that value, so the next field is read with its own name:
/// `UI_PASSWORD= UI_TOKEN=t0k3nValue2` masks the token as UI_TOKEN's, and
/// `UI_PASSWORD= UI_ADDR=127.0.0.1` masks nothing.
pub(crate) fn field_matches(re: &regex::Regex, s: &str) -> Vec<Groups> {
    let mut out = Vec::new();
    let mut at = 0;
    loop {
        let mut again = None;
        for m in captures(re, s, at) {
            if let Some(Some((a, b))) = m.get(1)
                && *a > at
                && crate::fieldvalue::next_field(s, *a, *b)
            {
                again = Some(*a);
                break;
            }
            out.push(m);
        }
        match again {
            Some(a) => at = a,
            None => return out,
        }
    }
}

/// The rules built in, compiled when first used.
pub(crate) static RULES: LazyLock<Vec<Finder>> = LazyLock::new(|| {
    let mut all = crate::secrets::rules();
    all.extend(crate::personal::rules());
    all.extend(crate::more::rules());
    all.extend(crate::crypto::rules());
    all
});

/// What the rules, the built-in ones and the user's (`custom`), find in
/// `s`.
pub(crate) fn rule_spans(s: &str, o: &Options, custom: &[Finder]) -> Vec<Span> {
    let mut found = Vec::new();
    let mut lower: Option<String> = None;
    // matches a rule took apart into values of their own (rkato@box17 as a
    // user and a host): what another rule found from the same place in
    // them, an email, goes
    let mut split: Vec<(usize, usize)> = Vec::new();
    // the user's first: where one of theirs and one built in find the same
    // value, it goes by the name they gave it
    let custom: &[Finder] = if o.secrets { custom } else { &[] };
    for r in custom.iter().chain(RULES.iter()) {
        if !o.covers(r.cat) {
            continue;
        }
        if !r.markers.is_empty() && !contains_one(s, &r.markers) {
            continue;
        }
        if let Some(may) = r.may
            && !may(s)
        {
            continue;
        }
        if !r.fold.is_empty() {
            let l = lower.get_or_insert_with(|| lower_ascii(s));
            if !contains_one(l, &r.fold) {
                continue;
            }
        }
        let l = if r.ahead > 0 {
            lower.get_or_insert_with(|| lower_ascii(s)).as_str()
        } else {
            ""
        };
        for m in r.matches(s, l) {
            if !r.groups.is_empty() {
                let mut took = false;
                for (g, (kind, ok)) in r.groups.iter().enumerate() {
                    if let Some(Some((a, b))) = m.get(g + 1)
                        && r.takes(s, *a, *b, Some(*ok))
                    {
                        found.push(Span::new(*a, *b, kind, r.by()));
                        took = true;
                    }
                }
                if took && let (Some(Some((a, _))), Some((_, z))) = (m.get(1), m[0]) {
                    split.push((*a, z));
                }
                continue;
            }
            let (mut a, mut b) = m[0].unwrap_or((0, 0));
            for g in m.iter().skip(1) {
                if let Some((ga, gb)) = g {
                    (a, b) = (*ga, *gb);
                    break;
                }
                if !r.any_group {
                    break;
                }
            }
            let mut ok = r.ok;
            if let Some(fix) = r.fix {
                match fix(s, a, b) {
                    Some((fa, fb)) => (a, b) = (fa, fb),
                    None => continue,
                }
                ok = None;
            }
            if r.takes(s, a, b, ok) {
                found.push(Span::new(a, b, &r.kind, r.by()));
            }
        }
    }
    for (a, z) in split {
        found.retain(|f| !(f.kind == "EMAIL" && f.start == a && f.end <= z));
    }
    found
}
