//! What to mask: the settings' half of masking, and their checks.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Says what to mask.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Options {
    /// API keys, private keys, tokens, passwords, wallet keys.
    pub secrets: bool,
    /// Emails, phone numbers, ID and bank card numbers, and the categories
    /// of [`CATEGORIES`] that are on.
    pub personal: bool,
    /// The categories of [`CATEGORIES`] on or off by id, where the user
    /// chose; one not here is as its `on` says. None is masked while
    /// `personal` is off.
    pub kinds: BTreeMap<String, bool>,
    /// The user's own words: names, codenames, hosts. A word that is a
    /// date matches that date however it is written.
    pub words: Vec<String>,
    /// Matches `words` in any case: Nightjar masks NIGHTJAR and nightjar.
    pub words_any_case: bool,
    /// Matches `words` only as whole words: ABC masks "ABC IS" but not
    /// XABCD, and 123456 no part of 0.123456.
    pub words_whole: bool,
    /// The user's own rules, masked while `secrets` is on.
    pub rules: Vec<Rule>,
    /// What is done with a value of a kind (a [`crate::Hit`]'s kind), where
    /// the user chose; a kind not here is masked.
    pub actions: BTreeMap<String, Action>,
    /// Values that always go as written, whichever finder finds them: each
    /// a whole value, in any case. The user's words still mask.
    pub allow: Vec<String>,
    /// Send a key as a made-up key of its form. Not ported: keys go as
    /// placeholders whatever this says.
    pub shape: bool,
}

/// One of the user's rules, for a secret the built-in rules don't know (a
/// gateway's own key, say).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    pub kind: RuleKind,
    /// The prefix a value starts with, or the pattern (RE2's syntax) that
    /// finds one; a pattern's value is its group 1 when it has one.
    pub value: String,
    /// What its placeholders are called: `{{NAME_…}}`, upper case letters,
    /// digits and `_`. Empty is `CUSTOM`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleKind {
    /// A token that starts with the value: the value and 8 or more of
    /// letters, digits, `_` and `-`.
    Prefix,
    /// A regular expression.
    Regex,
}

/// What is done with a value found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    /// Sent as a placeholder.
    #[default]
    Mask,
    /// Sent as written, and reported.
    Warn,
    /// Masked, and the request held back.
    Block,
}

/// A category of personal data the user turns on or off on its own: its id
/// in [`Options::kinds`], the kinds of its placeholders, and whether it is
/// masked while the user hasn't chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Category {
    pub id: &'static str,
    pub kinds: &'static [&'static str],
    pub on: bool,
}

/// The categories of personal data, in the order a settings page lists
/// them. The ones that would turn up in nearly every coding request (the
/// home folder is in every agent's system prompt) are off until turned on.
pub const CATEGORIES: &[Category] = &[
    Category {
        id: "ssn",
        kinds: &["SSN"],
        on: true,
    },
    Category {
        id: "passport",
        kinds: &["PASSPORT"],
        on: true,
    },
    Category {
        id: "iban",
        kinds: &["IBAN"],
        on: true,
    },
    Category {
        id: "birthday",
        kinds: &["BIRTHDAY"],
        on: true,
    },
    Category {
        id: "mac",
        kinds: &["MAC"],
        on: true,
    },
    Category {
        id: "serial",
        kinds: &["SERIAL"],
        on: true,
    },
    Category {
        id: "home",
        kinds: &["USERNAME"],
        on: false,
    },
    Category {
        id: "userhost",
        kinds: &["USERNAME", "HOSTNAME"],
        on: false,
    },
    Category {
        id: "ip",
        kinds: &["IP"],
        on: false,
    },
    Category {
        id: "bucket",
        kinds: &["BUCKET"],
        on: false,
    },
];

/// The kinds of secrets the built-in rules find.
pub const SECRET_KINDS: &[&str] = &[
    "API_KEY",
    "PRIVATE_KEY",
    "TOKEN",
    "PASSWORD",
    "SECRET",
    "WALLET_KEY",
    "WIF_KEY",
    "BIP32_KEY",
    "SOLANA_KEY",
    "SOLANA_KEYPAIR",
    "NEAR_KEY",
    "SUI_KEY",
    "APTOS_KEY",
    "STELLAR_KEY",
    "NOSTR_KEY",
    "AGE_KEY",
    "ZCASH_KEY",
    "SEED_PHRASE",
];

/// The kinds of personal data: the four always masked with `personal`,
/// then the categories'.
pub const PERSONAL_KINDS: &[&str] = &[
    "EMAIL",
    "PHONE",
    "ID_CARD",
    "BANK_CARD",
    "SSN",
    "PASSPORT",
    "IBAN",
    "BIRTHDAY",
    "MAC",
    "SERIAL",
    "USERNAME",
    "HOSTNAME",
    "IP",
    "BUCKET",
];

/// The kind of a value the user's own words found.
pub const WORD_KIND: &str = "TERM";

/// How many values `allow` may hold, and how long each.
const MAX_ALLOW: usize = 200;
const MAX_ALLOW_LEN: usize = 256;

/// How many rules a user may have, and how long each part of one.
pub(crate) const MAX_RULES: usize = 32;
pub(crate) const MAX_KIND: usize = 24;
const MIN_PREFIX: usize = 3;
const MAX_PREFIX: usize = 64;
pub(crate) const MAX_REGEX: usize = 300;
const CUSTOM_KIND: &str = "CUSTOM";

impl Options {
    /// Says there is anything to mask. The user's rules mask only with
    /// `secrets`.
    pub fn wanted(&self) -> bool {
        self.secrets || self.personal || self.words.iter().any(|w| w.trim().chars().count() >= 2)
    }

    /// Tidies the options as settings keep them (spaces trimmed, the empty
    /// and the repeated dropped, a rule's name as a placeholder has it) and
    /// says what is wrong with them: a rule whose pattern doesn't compile
    /// or matches nothing at all, a prefix too short, too many or too long
    /// a rule or an allowed value, an action for something that isn't a
    /// kind.
    pub fn check(&mut self) -> Result<(), String> {
        let mut words: Vec<String> = Vec::new();
        for w in &self.words {
            let w = w.trim();
            if !w.is_empty() && !words.iter().any(|x| x == w) {
                words.push(w.to_owned());
            }
        }
        self.words = words;

        let mut allow: Vec<String> = Vec::new();
        for v in &self.allow {
            let v = v.trim();
            if v.is_empty() || allow.iter().any(|a| a.to_lowercase() == v.to_lowercase()) {
                continue;
            }
            if v.chars().count() > MAX_ALLOW_LEN {
                return Err(format!(
                    "a value the masking lets go is at most {MAX_ALLOW_LEN} characters"
                ));
            }
            allow.push(v.to_owned());
        }
        if allow.len() > MAX_ALLOW {
            return Err(format!("the masking lets at most {MAX_ALLOW} values go"));
        }
        self.allow = allow;

        let mut rules: Vec<Rule> = Vec::new();
        for r in &self.rules {
            let mut r = r.clone();
            r.value = r.value.trim().to_owned();
            if r.value.is_empty() {
                continue;
            }
            r.name = rule_kind(&r.name);
            match r.kind {
                RuleKind::Prefix => {
                    let n = r.value.chars().count();
                    if !(MIN_PREFIX..=MAX_PREFIX).contains(&n) {
                        return Err(format!(
                            "a masking rule's prefix is {MIN_PREFIX} to {MAX_PREFIX} characters long, not {:?}",
                            r.value
                        ));
                    }
                }
                RuleKind::Regex => {
                    if r.value.len() > MAX_REGEX {
                        return Err(format!(
                            "a masking rule's regular expression is at most {MAX_REGEX} characters long"
                        ));
                    }
                }
            }
            let re = r.compile().map_err(|e| {
                format!(
                    "the masking rule {} has a regular expression that doesn't work: {e}",
                    r.name
                )
            })?;
            if re.is_match("") {
                return Err(format!(
                    "the masking rule {} matches nothing at all, so it would mask everywhere: {}",
                    r.name, r.value
                ));
            }
            if !rules.contains(&r) {
                rules.push(r);
            }
        }
        if rules.len() > MAX_RULES {
            return Err(format!(
                "at most {MAX_RULES} masking rules, not {}",
                rules.len()
            ));
        }
        self.rules = rules;

        let mut actions = BTreeMap::new();
        for (k, a) in &self.actions {
            let k = k.trim();
            if !kind_name(k) {
                return Err(format!("{k:?} isn't a kind of value masking finds"));
            }
            if *a != Action::Mask {
                actions.insert(k.to_owned(), *a);
            }
        }
        self.actions = actions;
        Ok(())
    }

    /// Says `cat` is masked: "" are the secrets, "personal" the personal
    /// data always masked with it, any other one of [`CATEGORIES`].
    pub(crate) fn covers(&self, cat: &str) -> bool {
        match cat {
            "" => self.secrets,
            PERSONAL => self.personal,
            _ if !self.personal => false,
            _ => match self.kinds.get(cat) {
                Some(on) => *on,
                None => CATEGORIES.iter().any(|c| c.id == cat && c.on),
            },
        }
    }

    /// Says the user lets `v` go as written.
    pub(crate) fn allows(&self, v: &str) -> bool {
        self.allow.iter().any(|a| eq_fold(a.trim(), v))
    }

    pub(crate) fn action(&self, kind: &str) -> Action {
        self.actions.get(kind).copied().unwrap_or_default()
    }

    /// The words to look for: trimmed, two characters or more.
    pub(crate) fn words(&self) -> impl Iterator<Item = &str> {
        self.words.iter().map(|w| w.trim()).filter(|w| w.len() >= 2)
    }
}

/// The category of the personal data masked whenever `personal` is.
pub(crate) const PERSONAL: &str = "personal";

impl Rule {
    /// The kind its placeholders have.
    pub fn placeholder_kind(&self) -> String {
        rule_kind(&self.name)
    }

    pub(crate) fn compile(&self) -> Result<regex::Regex, regex::Error> {
        match self.kind {
            RuleKind::Prefix => crate::re::user(&(regex::escape(&self.value) + PREFIX_TAIL)),
            RuleKind::Regex => crate::re::user(&self.value),
        }
    }
}

/// What follows a rule's prefix.
const PREFIX_TAIL: &str = r"[A-Za-z0-9_\-]{8,}";

/// A kind as a placeholder can have it: upper case letters, digits and `_`,
/// starting with a letter, at most 24 long; "" is CUSTOM.
pub fn rule_kind(kind: &str) -> String {
    let mut b = String::new();
    for r in kind.trim().to_uppercase().chars() {
        match r {
            'A'..='Z' | '0'..='9' | '_' => b.push(r),
            ' ' | '-' | '.' => b.push('_'),
            _ => {}
        }
    }
    let mut k = b.trim_matches('_').to_owned();
    if k.is_empty() {
        return CUSTOM_KIND.to_owned();
    }
    if !k.starts_with(|c: char| c.is_ascii_uppercase()) {
        k = format!("{CUSTOM_KIND}_{k}");
    }
    if k.len() > MAX_KIND {
        k = k[..MAX_KIND].trim_end_matches('_').to_owned();
    }
    k
}

/// Says `k` is written as a placeholder names a kind, as a setting keyed by
/// kind must name it.
pub fn kind_name(k: &str) -> bool {
    !k.is_empty() && rule_kind(k) == k
}

/// Says a and b are the same in any case, as Go's EqualFold does.
pub(crate) fn eq_fold(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.eq_ignore_ascii_case(b) || a.to_lowercase() == b.to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_kinds() {
        assert_eq!(rule_kind(""), "CUSTOM");
        assert_eq!(rule_kind(" open code-key "), "OPEN_CODE_KEY");
        assert_eq!(rule_kind("9lives"), "CUSTOM_9LIVES");
        assert_eq!(
            rule_kind("a very long name for a kind of key"),
            "A_VERY_LONG_NAME_FOR_A_K"
        );
        assert!(kind_name("API_KEY") && !kind_name("api_key") && !kind_name(""));
    }

    #[test]
    fn check_tidies_and_refuses() {
        let mut o = Options {
            words: vec![" Nightjar ".into(), "".into(), "Nightjar".into()],
            allow: vec!["a".into(), " A ".into(), "b".into()],
            rules: vec![
                Rule {
                    kind: RuleKind::Prefix,
                    value: " oc_sk_ ".into(),
                    name: "open code".into(),
                },
                Rule {
                    kind: RuleKind::Prefix,
                    value: "oc_sk_".into(),
                    name: "OPEN_CODE".into(),
                },
                Rule {
                    kind: RuleKind::Regex,
                    value: "  ".into(),
                    name: String::new(),
                },
            ],
            actions: BTreeMap::from([
                ("API_KEY".into(), Action::Warn),
                ("EMAIL".into(), Action::Mask),
            ]),
            ..Options::default()
        };
        o.check().unwrap();
        assert_eq!(o.words, ["Nightjar"]);
        assert_eq!(o.allow, ["a", "b"]);
        assert_eq!(o.rules.len(), 1);
        assert_eq!(o.rules[0].name, "OPEN_CODE");
        assert_eq!(
            o.actions,
            BTreeMap::from([("API_KEY".into(), Action::Warn)])
        );

        for bad in [
            Options {
                rules: vec![Rule {
                    kind: RuleKind::Regex,
                    value: "(".into(),
                    name: String::new(),
                }],
                ..Options::default()
            },
            Options {
                rules: vec![Rule {
                    kind: RuleKind::Regex,
                    value: "x*".into(),
                    name: String::new(),
                }],
                ..Options::default()
            },
            Options {
                rules: vec![Rule {
                    kind: RuleKind::Prefix,
                    value: "ab".into(),
                    name: String::new(),
                }],
                ..Options::default()
            },
            Options {
                allow: vec!["x".repeat(257)],
                ..Options::default()
            },
            Options {
                allow: (0..201).map(|i| i.to_string()).collect(),
                ..Options::default()
            },
            Options {
                actions: BTreeMap::from([("api key".into(), Action::Block)]),
                ..Options::default()
            },
            Options {
                rules: (0..33)
                    .map(|i| Rule {
                        kind: RuleKind::Prefix,
                        value: format!("p{i:02}_"),
                        name: String::new(),
                    })
                    .collect(),
                ..Options::default()
            },
        ] {
            let mut o = bad;
            assert!(o.check().is_err(), "{o:?}");
        }
    }

    #[test]
    fn serde_shape() {
        let o: Options = serde_json::from_str(
            r#"{"secrets":true,"kinds":{"ip":true},"wordsAnyCase":true,"rules":[{"kind":"prefix","value":"oc_sk_"}],"actions":{"EMAIL":"block"}}"#,
        )
        .unwrap();
        assert!(o.secrets && o.words_any_case && o.kinds["ip"]);
        assert_eq!(o.rules[0].kind, RuleKind::Prefix);
        assert_eq!(o.actions["EMAIL"], Action::Block);
        assert!(o.wanted());
        assert!(!Options::default().wanted());
        assert!(
            Options {
                words: vec!["ab".into()],
                ..Options::default()
            }
            .wanted()
        );
    }

    #[test]
    fn categories_default() {
        let o = Options {
            personal: true,
            ..Options::default()
        };
        assert!(o.covers("ssn") && o.covers(PERSONAL) && !o.covers("ip") && !o.covers(""));
        let o = Options {
            personal: true,
            kinds: BTreeMap::from([("ip".into(), true), ("ssn".into(), false)]),
            ..Options::default()
        };
        assert!(o.covers("ip") && !o.covers("ssn"));
        let o = Options {
            kinds: BTreeMap::from([("ip".into(), true)]),
            ..Options::default()
        };
        assert!(!o.covers("ip"));
    }
}
