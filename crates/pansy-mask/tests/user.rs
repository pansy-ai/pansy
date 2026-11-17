//! What the user chooses: their own words, their own rules, what is done
//! with a kind found, the values let go, and a word that is a date. The Go
//! tests of magpie's internal/redact, ported; what hangs on Go's internals
//! (Preview, the stream's partialTail) is checked by what it does.

use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock};

use pansy_mask::{Action, CATEGORIES, Masker, Options, Rule, RuleKind, rule_kind};
use regex::Regex;

static PLACEHOLDER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{\{[A-Z][A-Z0-9_]*_[a-z2-7]{8}\}\}").unwrap());

const OPENAI_KEY: &str = "sk-proj-abcdEFGH1234ijklMNOP5678qrst";

fn masker() -> Masker {
    Masker::new(b"test key")
}

/// `s` without its placeholders. A placeholder's suffix is a keyed hash, so
/// it can spell a short value a test checks is gone.
fn unmarked(s: &str) -> String {
    PLACEHOLDER.replace_all(s, "").into_owned()
}

fn first_placeholder(s: &str) -> &str {
    PLACEHOLDER.find(s).map_or("", |m| m.as_str())
}

/// Masks `s` as magpie's Mask did: the text, and how many values went as
/// placeholders. One the user is only warned of isn't counted.
fn mask(m: &Masker, s: &str, o: &Options) -> (String, usize) {
    let out = m.mask_text(s, o);
    let n = out.hits.iter().filter(|h| h.action != Action::Warn).count();
    (out.text, n)
}

fn mask_json(m: &Masker, body: &str, o: &Options) -> (String, usize) {
    let out = m.mask_json(body.as_bytes(), o);
    let n = out.hits.iter().filter(|h| h.action != Action::Warn).count();
    (String::from_utf8(out.body).unwrap(), n)
}

fn restore(m: &Masker, s: &str) -> String {
    m.restore(s).0
}

fn words(ws: &[&str]) -> Options {
    Options {
        words: ws.iter().map(|w| w.to_string()).collect(),
        ..Options::default()
    }
}

fn prefix(name: &str, value: &str) -> Rule {
    Rule {
        kind: RuleKind::Prefix,
        value: value.into(),
        name: name.into(),
    }
}

fn regex(name: &str, value: &str) -> Rule {
    Rule {
        kind: RuleKind::Regex,
        value: value.into(),
        name: name.into(),
    }
}

/// The failures of a table's cases, told together when the test ends, as
/// Go's t.Errorf has them.
#[derive(Default)]
struct Errors(Vec<String>);

impl Drop for Errors {
    fn drop(&mut self) {
        if !self.0.is_empty() && !std::thread::panicking() {
            panic!("{} failed:\n{}", self.0.len(), self.0.join("\n"));
        }
    }
}

macro_rules! errorf {
    ($t:expr, $($a:tt)*) => { $t.0.push(format!($($a)*)) };
}

/// The text a stream of Anthropic text deltas comes to once its values are
/// put back, the stream cut into one event before `cut` and one after.
fn streamed(m: &Arc<Masker>, text: &str, cut: usize) -> String {
    let event = |s: &str| {
        let data = serde_json::json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": s}});
        format!("event: content_block_delta\ndata: {data}\n\n")
    };
    let mut r = m.stream();
    let mut out = r.push(event(&text[..cut]).as_bytes());
    out.extend(r.push(event(&text[cut..]).as_bytes()));
    out.extend(r.finish());
    let out = String::from_utf8(out).unwrap();
    let mut got = String::new();
    for ev in out.split("\n\n") {
        let Some((_, d)) = ev.split_once("data: ") else {
            continue;
        };
        let v: serde_json::Value = serde_json::from_str(d).unwrap();
        got += v["delta"]["text"].as_str().unwrap_or("");
    }
    got
}

// rules_test.go: the user's own rules

#[test]
fn rule_kinds() {
    let mut t = Errors::default();
    for (input, want) in [
        ("", "CUSTOM"),
        ("api key", "API_KEY"),
        ("oc-sk", "OC_SK"),
        ("{{x}}", "X"),
        ("9lives", "CUSTOM_9LIVES"),
        ("数据库", "CUSTOM"),
        (
            "a_very_long_kind_name_that_goes_on",
            "A_VERY_LONG_KIND_NAME_TH",
        ),
    ] {
        let got = rule_kind(input);
        if got != want {
            errorf!(t, "rule_kind({input:?}) = {got:?}, want {want:?}");
        }
    }
}

// The settings' check tidies the rules (names as a placeholder has them,
// the empty and the repeated dropped) and turns away one that doesn't
// compile, has a lookbehind RE2 lacks, matches nothing at all, has too
// short a prefix or too long a pattern, and too many. Go's rule of both a
// prefix and a pattern can't be written here.
#[test]
fn check_rules() {
    let mut o = Options {
        rules: vec![
            prefix(" gw key ", " acme_ "),
            regex("", "  "),
            prefix("GW_KEY", "acme_"),
            regex("ns", "ns-([0-9a-f]{12})"),
        ],
        ..Options::default()
    };
    let res = o.check();
    assert!(
        res.is_ok()
            && o.rules.len() == 2
            && o.rules[0] == prefix("GW_KEY", "acme_")
            && o.rules[1].name == "NS",
        "{:?} {res:?}",
        o.rules
    );
    let mut t = Errors::default();
    for bad in [
        regex("", "ns-([0-9a-f"),
        regex("", "(?P<x>a)(?<!b)"),
        regex("", "x*"),
        prefix("", "ab"),
        // longer than the 300 a pattern may have
        regex("", &"a".repeat(301)),
    ] {
        let mut o = Options {
            rules: vec![bad.clone()],
            ..Options::default()
        };
        if o.check().is_ok() {
            errorf!(t, "{bad:?} accepted");
        }
    }
    // one more than the 32 a user may have
    let many = (0..33)
        .map(|i| prefix("", &format!("p{}", "x".repeat(i + 2))))
        .collect();
    let mut o = Options {
        rules: many,
        ..Options::default()
    };
    if o.check().is_ok() {
        errorf!(t, "too many rules accepted");
    }
}

#[test]
fn custom_rules() {
    let rules = vec![
        prefix("GW_KEY", "acme-"),
        regex("NS_TOKEN", "nstok:([0-9a-f]{12})"),
        // a settings file written by hand: left out
        regex("BROKEN", "x("),
    ];
    let o = Options {
        secrets: true,
        rules: rules.clone(),
        ..Options::default()
    };
    let m = masker();
    let s = format!("use acme-Zx9_ab-12cd then nstok:0123456789ab and {OPENAI_KEY}");
    let (out, n) = mask(&m, &s, &o);
    assert!(
        n == 3
            && !out.contains("acme-Zx9")
            && !out.contains("0123456789ab")
            && out.contains("{{GW_KEY_")
            && out.contains("nstok:{{NS_TOKEN_")
            && out.contains("{{API_KEY_"),
        "{n}: {out:?}"
    );
    assert_eq!(restore(&m, &out), s);
    // the same placeholder turn after turn
    let (again, _) = mask(&m, "again: acme-Zx9_ab-12cd", &o);
    let (p1, p2) = (first_placeholder(&out), first_placeholder(&again));
    assert!(
        !p1.is_empty() && p1 == p2,
        "placeholders differ: {out:?} {again:?}"
    );
    // a prefix wants 8 more characters after it
    assert_eq!(mask(&masker(), "acme-short", &o).1, 0, "short value masked");
    // only with the secrets
    assert_eq!(
        mask(
            &masker(),
            &s,
            &Options {
                rules,
                ..Options::default()
            }
        )
        .1,
        0,
        "masked with secrets off"
    );

    // where the user's and the built-in rules find the same value, the
    // user's name wins; where they overlap, the first to start and the
    // longest
    let oc_key = "oc_sk_AAAABBBBccccDDDD1234eeeeFFFF5678";
    let o = Options {
        secrets: true,
        rules: vec![prefix("OC", "oc_sk_")],
        ..Options::default()
    };
    let (out, _) = mask(&masker(), &format!(r#""key": "{oc_key}""#), &o);
    assert!(
        out.contains("{{OC_") && !out.contains("API_KEY"),
        "user's kind lost: {out:?}"
    );
    let o = Options {
        secrets: true,
        rules: vec![regex("TAIL", "BBBBcccc[A-Za-z0-9]+")],
        ..Options::default()
    };
    let (out, n) = mask(&masker(), oc_key, &o);
    assert!(n == 1 && out.starts_with("{{API_KEY_"), "overlap: {out:?}");

    // a kind with digits restores, in a stream too: a placeholder cut
    // three characters from its end is held for the rest
    let m = Arc::new(masker());
    let o = Options {
        secrets: true,
        rules: vec![prefix("K8S", "k8s-tok-")],
        ..Options::default()
    };
    let (out, _) = mask(&m, "k8s-tok-abcdef123456", &o);
    assert!(
        out.starts_with("{{K8S_") && restore(&m, &out) == "k8s-tok-abcdef123456",
        "digits in kind: {out:?}"
    );
    let got = streamed(&m, &out, out.len() - 3);
    assert_eq!(
        got, "k8s-tok-abcdef123456",
        "digits in kind, streamed: {out:?}"
    );

    // a kind as long as a rule may have is held in a stream up to its last
    // character, as a short one is
    let m = Arc::new(masker());
    let o = Options {
        secrets: true,
        rules: vec![prefix("internal gateway token x", "igt-")],
        ..Options::default()
    };
    let (out, _) = mask(&m, "igt-abcdef123456", &o);
    assert!(
        out.starts_with("{{INTERNAL_GATEWAY_TOKEN_X_") && out.len() == 2 + 24 + 1 + 8 + 2,
        "long kind: {out:?}"
    );
    let text = format!("text {out}");
    let mut t = Errors::default();
    for i in 1..out.len() {
        let got = streamed(&m, &text, 5 + i);
        if got != "text igt-abcdef123456" {
            errorf!(t, "long kind cut at {i}: {got:?}");
        }
    }
}

// actions_test.go

// A kind the user set to warn of goes as written: told, with a placeholder
// no one remembers, and not counted as masked. The rest is masked as
// before.
#[test]
fn action_warn_sends_as_written() {
    let (key, mail) = ("sk-proj-Wn4rT7yQ2mLx9VbK3cZp", "warned.only@kelvarn.io");
    let o = Options {
        secrets: true,
        personal: true,
        actions: BTreeMap::from([("EMAIL".to_owned(), Action::Warn)]),
        ..Options::default()
    };
    let m = masker();
    let r = m.mask_text(&format!("key {key} mail {mail}"), &o);
    let n = r.hits.iter().filter(|h| h.action != Action::Warn).count();
    assert!(
        r.text.contains(mail) && !r.text.contains(key) && n == 1,
        "out {:?} n {n}",
        r.text
    );
    let warned = r.hits.iter().find(|h| h.kind == "EMAIL");
    assert!(
        r.hits.len() == 2
            && warned
                .is_some_and(|w| w.action == Action::Warn && w.head == "w" && w.len == mail.len()),
        "hits {:?}",
        r.hits
    );
    let warned = warned.unwrap();
    assert!(
        m.value(&warned.placeholder).is_none() && !r.text.contains(&warned.placeholder),
        "a value sent as written is remembered or placed: {warned:?} in {:?}",
        r.text
    );

    // in a JSON body the same
    let b = m.mask_json(
        format!(r#"{{"messages":[{{"role":"user","content":"mail {mail}"}}]}}"#).as_bytes(),
        &o,
    );
    let body = String::from_utf8_lossy(&b.body);
    let n = b.hits.iter().filter(|h| h.action != Action::Warn).count();
    assert!(
        body.contains(mail) && n == 0 && b.hits.len() == 1 && b.hits[0].action == Action::Warn,
        "json {body} n {n} hits {:?}",
        b.hits
    );
}

// A kind the user set to block is masked all the same, and told as
// blocked, so the gateway holds the request back.
#[test]
fn action_block_masks() {
    let key = "sk-proj-Bl0cKq8Wm3Rt6YxN2vLp";
    let m = masker();
    let o = Options {
        secrets: true,
        actions: BTreeMap::from([("API_KEY".to_owned(), Action::Block)]),
        ..Options::default()
    };
    let r = m.mask_text(&format!("key {key}"), &o);
    assert!(
        !r.text.contains(key)
            && r.hits.len() == 1
            && r.hits[0].action == Action::Block
            && r.hits[0].kind == "API_KEY",
        "out {:?} hits {:?}",
        r.text,
        r.hits
    );
    assert_eq!(
        m.value(&r.hits[0].placeholder).as_deref(),
        Some(key),
        "a blocked value isn't masked as any other"
    );
}

// A value on the allow list goes as written, whole, in any case, and isn't
// told; a value it is only part of is masked.
#[test]
fn allow_lets_a_value_go() {
    let (mail, key) = ("Team.Inbox@kelvarn.io", "sk-proj-Al1oWq8Wm3Rt6YxN2vLp");
    let o = Options {
        secrets: true,
        personal: true,
        allow: vec!["  team.inbox@KELVARN.io ".into(), key.into()],
        ..Options::default()
    };
    let m = masker();
    let r = m.mask_text(
        &format!("write {mail} with {key} or other.person@kelvarn.io"),
        &o,
    );
    let n = r.hits.iter().filter(|h| h.action != Action::Warn).count();
    assert!(
        r.text.contains(mail)
            && r.text.contains(key)
            && !r.text.contains("other.person")
            && n == 1
            && r.hits.len() == 1,
        "out {:?} n {n} hits {:?}",
        r.text,
        r.hits
    );
    let (out, _) = mask(&m, &format!("cc x{mail}"), &o);
    assert!(
        !out.contains(&format!("x{mail}")),
        "a value the allowed one is part of went as written: {out:?}"
    );
}

// The user's words win over the values let go: a value on both lists is
// masked, and so is a word inside a value let go. A value let go wins over
// the user's own rules.
#[test]
fn masked_words_win_over_allow() {
    let mail = "team.inbox@kelvarn.io";
    let o = Options {
        secrets: true,
        personal: true,
        words: vec!["build-01".into(), "kelvarn".into()],
        allow: vec!["build-01".into(), mail.into(), "PROJ-0001".into()],
        rules: vec![regex("TICKET", r"PROJ-\d{4}")],
        ..Options::default()
    };
    let (out, _) = mask(
        &masker(),
        &format!("deploy on build-01, mail {mail}, tickets PROJ-0001 and PROJ-0002"),
        &o,
    );
    let mut t = Errors::default();
    if out.contains("build-01") {
        errorf!(t, "a value on both lists went as written: {out:?}");
    }
    if out.contains("kelvarn") || !out.contains("team.inbox@{{TERM_") {
        errorf!(t, "a masked word inside an allowed value: {out:?}");
    }
    if !out.contains("PROJ-0001") || out.contains("PROJ-0002") {
        errorf!(t, "a value let go against the user's rule: {out:?}");
    }
}

// anycase_test.go

// A masked word matches in any case only when the user asked for it; each
// comes back as it was written.
#[test]
fn words_any_case() {
    let s = "Nightjar, NIGHTJAR and nightjar; МОСКВА is Москва. A nightjars' night.";
    let mut t = Errors::default();
    for (name, any_case, count, gone, kept) in [
        (
            "as written",
            false,
            1,
            &["Nightjar,"][..],
            &["NIGHTJAR", "nightjar;", "МОСКВА", "nightjars'"][..],
        ),
        (
            "any case",
            true,
            6,
            &["Nightjar", "NIGHTJAR", "nightjar", "МОСКВА", "Москва"][..],
            &["A ", "s' night."][..],
        ),
    ] {
        let o = Options {
            words_any_case: any_case,
            ..words(&["Nightjar", " москва "])
        };
        let m = masker();
        let (out, n) = mask(&m, s, &o);
        if n != count {
            errorf!(t, "{name}: {n} masked, want {count}: {out:?}");
        }
        for g in gone {
            if unmarked(&out).contains(g) {
                errorf!(t, "{name}: {g:?} still in {out:?}");
            }
        }
        for k in kept {
            if !out.contains(k) {
                errorf!(t, "{name}: {k:?} gone from {out:?}");
            }
        }
        let back = restore(&m, &out);
        if back != s {
            errorf!(t, "{name}: round trip {out:?} -> {back:?}");
        }
        // and in a request body, as the gateway masks it
        let (body, _) = mask_json(
            &m,
            r#"{"messages":[{"role":"user","content":"ship NIGHTJAR"}]}"#,
            &o,
        );
        if !body.contains("NIGHTJAR") != any_case {
            errorf!(
                t,
                "{name}: NIGHTJAR in a request masked={}: {body}",
                !body.contains("NIGHTJAR")
            );
        }
    }
}

// wholeword_test.go

// Masked words as whole words only, when the user asks for it: ABC masks
// "ABC IS xxx" and name='ABC' but no part of XABCD or ABCDEF, and 123456
// masks a 123456 of its own but no part of 91234567, 0.123456 or
// 123456.78. Off, a word is found inside longer ones. Each masked one comes
// back as it was written.
#[test]
fn words_whole() {
    // the text, the word, any case, how many masked with whole words on
    // and off, and what whole words keep
    type Case = (
        &'static str,
        &'static str,
        bool,
        usize,
        usize,
        &'static [&'static str],
    );
    let cases: &[Case] = &[
        ("ABC IS xxx", "ABC", false, 1, 1, &[]),
        ("name='ABC'", "ABC", false, 1, 1, &[]),
        ("XABCD", "ABC", false, 0, 1, &["XABCD"]),
        ("ABCDEF", "ABC", false, 0, 1, &["ABCDEF"]),
        ("ABC_DEF and ABC1", "ABC", false, 0, 2, &["ABC_DEF", "ABC1"]),
        ("XABCD then ABC.", "ABC", false, 1, 2, &["XABCD then "]),
        ("id 123456 here", "123456", false, 1, 1, &[]),
        ("91234567", "123456", false, 0, 1, &["91234567"]),
        ("0.123456", "123456", false, 0, 1, &["0.123456"]),
        ("123456.78", "123456", false, 0, 1, &["123456.78"]),
        // a full stop that ends a sentence is no decimal point
        ("the code is 123456.", "123456", false, 1, 1, &[]),
        ("v.123456 and 123456,7", "123456", false, 2, 2, &[]),
        // Chinese and Japanese put no spaces between words: a word of
        // theirs is found inside their text, and a Latin one beside it
        ("我的同事张三今天来了", "张三", false, 1, 1, &[]),
        ("用ABC做的项目", "ABC", false, 1, 1, &[]),
        ("これはABCです", "ABC", false, 1, 1, &[]),
        ("用XABC做的", "ABC", false, 0, 1, &["XABC"]),
        // with any case too, and a word inside another is passed over to
        // the next whole one
        ("xabc then Abc!", "ABC", true, 1, 2, &["xabc then "]),
        ("Москва, москвабург", "москва", true, 1, 2, &["бург"]),
    ];
    let mut t = Errors::default();
    for &(s, word, any_case, whole_n, anywhere_n, kept_whole) in cases {
        for whole in [true, false] {
            let o = Options {
                words_any_case: any_case,
                words_whole: whole,
                ..words(&[word])
            };
            let m = masker();
            let (out, n) = mask(&m, s, &o);
            let want = if whole { whole_n } else { anywhere_n };
            if n != want {
                errorf!(
                    t,
                    "{word:?} in {s:?}, whole {whole}: {n} masked, want {want}: {out:?}"
                );
            }
            if whole {
                for k in kept_whole {
                    if !out.contains(k) {
                        errorf!(t, "{word:?} in {s:?}: {k:?} gone from {out:?}");
                    }
                }
            }
            let back = restore(&m, &out);
            if back != s {
                errorf!(
                    t,
                    "{word:?} in {s:?}, whole {whole}: round trip {out:?} -> {back:?}"
                );
            }
        }
    }
}

// In a request body, a tool call's arguments included: the escapes around a
// word are read as the vendor reads them.
#[test]
fn words_whole_json() {
    let o = Options {
        words_whole: true,
        ..words(&["ABC", "123456"])
    };
    let body = concat!(
        r#"{"messages":[{"role":"user","content":"name='ABC' XABCD 0.123456 pin 123456"},"#,
        r#"{"role":"assistant","tool_calls":[{"type":"function","function":{"name":"f","arguments":"{\"q\":\"ABC\\nXABCD 123456.78\"}"}}]}]}"#,
    );
    let m = masker();
    let (s, _) = mask_json(&m, body, &o);
    let mut t = Errors::default();
    for (k, want) in [("XABCD", 2), ("0.123456", 1), ("123456.78", 1)] {
        if s.matches(k).count() != want {
            errorf!(t, "{k:?} not kept as written: {s}");
        }
    }
    if unmarked(&s).contains("'ABC'")
        || unmarked(&s).contains("pin 123456")
        || s.contains(r#"\"ABC\\n"#)
    {
        errorf!(t, "a whole word went unmasked: {s}");
    }
    let back = restore(&m, &s);
    if back == s || !back.contains("name='ABC'") {
        errorf!(t, "not put back: {back}");
    }
}

// dateword_test.go

// A date word matches that calendar date in every common form, and never
// another date or a longer number.
#[test]
fn date_word_every_form() {
    let forms = [
        "1990-05-12",
        "1990-5-12",
        "1990/05/12",
        "1990/5/12",
        "1990.05.12",
        "1990.5.12",
        "1990、5、12",
        "1990 5 12",
        "1990 / 05 / 12",
        "1990. 5. 12",
        "1990年5月12日",
        "1990年05月12日",
        "1990 年 5 月 12 日",
        "1990年5月12号",
        "1990年5月12",
        "1990년 5월 12일",
        "19900512",
        "May 12, 1990",
        "May 12th 1990",
        "may 12 1990",
        "12 May 1990",
        "12-May-1990",
    ];
    let mut t = Errors::default();
    for word in [
        "1990-05-12",
        "1990/5/12",
        "1990年5月12日",
        "May 12, 1990",
        "12 May 1990",
    ] {
        let o = words(&[word]);
        for f in forms {
            for s in [
                f.to_owned(),
                format!("生日是{f}，记得"),
                format!("born {f}."),
                format!(r#"{{"dob":"{f}"}}"#),
                format!("IMG_{f}_1.jpg"),
                format!("{f}T08:00:00Z"),
            ] {
                let m = masker();
                let (out, n) = mask(&m, &s, &o);
                if n != 1 || out.contains(f) {
                    errorf!(t, "word {word:?}: {s:?} masked {n} as {out:?}");
                    continue;
                }
                let back = restore(&m, &out);
                if back != s {
                    errorf!(t, "word {word:?}: {s:?} came back as {back:?}");
                }
            }
        }
    }
}

// Day-first and month-first forms only where one reading is a date.
#[test]
fn date_word_day_month_order() {
    let mut t = Errors::default();
    for (word, s, want) in [
        ("1990-05-13", "13/05/1990", 1),
        ("1990-05-13", "05/13/1990", 1),
        ("1990-05-13", "13.5.1990", 1),
        ("13.05.1990", "1990年5月13日", 1),
        ("05/13/1990", "1990-05-13", 1),
        ("1990-05-05", "05/05/1990", 1),
        ("1990-05-12", "05/12/1990", 0),
        ("1990-05-12", "12/05/1990", 0),
        ("1990-05-12", "12.05.1990", 0),
        // May 12th or December 5th: no date
        ("05/12/1990", "1990-05-12", 0),
        ("05/12/1990", "1990-12-05", 0),
        // still the word as written
        ("05/12/1990", "on 05/12/1990.", 1),
    ] {
        let (_, n) = mask(&masker(), s, &words(&[word]));
        if n != want {
            errorf!(t, "word {word:?}: {s:?} masked {n}, want {want}");
        }
    }
}

// A date word never matches another date, a longer number, or a version
// that starts as the date does.
#[test]
fn date_word_not_another_date() {
    let mut t = Errors::default();
    for word in ["1990-05-12", "1990.5.12", "1990年5月12日"] {
        for s in [
            "1990-05-13",
            "1990-12-05",
            "1991-05-12",
            "1990年5月1日",
            "1990年5月120日",
            "1990/05/1",
            "1990.5.12.1",
            "v1990.5.12.1",
            "3.1990.5.12",
            "11990-05-12",
            "1990-05-123",
            "199005123",
            "2199005120",
            "19900513",
            "1990512",
            "May 13, 1990",
            "Mayday 12 1990",
            "12/05/1990",
            "05/12/1990",
        ] {
            let (out, n) = mask(&masker(), s, &words(&[word]));
            if n != 0 || out != s {
                errorf!(t, "word {word:?}: {s:?} masked as {out:?}");
            }
        }
    }
    // a full stop after the date ends a sentence; it is no version
    if mask(&masker(), "生日 1990.5.12.", &words(&["1990-05-12"])).1 != 1 {
        errorf!(t, "date before a full stop not masked");
    }
}

// What is no date stays a word as before: eight digits, a date that isn't
// one, a word with a date in it.
#[test]
fn date_word_only_a_date() {
    let mut t = Errors::default();
    for (word, s) in [
        ("19900512", "1990-05-12"),
        ("1990-02-30", "1990/2/30"),
        ("1990-13-01", "1990/13/1"),
        ("release-1990-05-12", "1990-05-12"),
    ] {
        let (out, n) = mask(&masker(), s, &words(&[word]));
        if n != 0 {
            errorf!(t, "word {word:?}: {s:?} masked as {out:?}");
        }
    }
    // and is found inside a longer one as before
    if mask(&masker(), "x19900512y", &words(&["19900512"])).1 != 1 {
        errorf!(t, "eight-digit word no longer found inside a longer one");
    }
}

// The birthday rule needs a birthday's word right before the date, so a
// log's or a task's own date is not one, however old. A date of birth
// stored as a time is still a birthday. Go held its clock at 2026-10-27;
// these dates are years before any day the tests run.
#[test]
fn birthday_log_dates() {
    let mut o = Options {
        secrets: true,
        personal: true,
        ..Options::default()
    };
    for c in CATEGORIES {
        o.kinds.insert(c.id.to_owned(), true);
    }
    let mut t = Errors::default();
    for s in [
        "2019-03-02 10:00:01 INFO birthday reminder sent to user 42",
        "[2019-03-02T10:00:01Z] 生日提醒已发送",
        "logs/2019-03-02.log: born-again job finished",
        "任务创建于 2019-03-02，生日活动筹备",
        "2019-03-02 生日",
    ] {
        let (out, n) = mask(&masker(), s, &o);
        if n != 0 || out != s {
            errorf!(t, "{s:?} masked as {out:?}");
        }
    }
    for s in [
        r#"{"birthday":"1990-05-11T16:00:00.000Z"}"#,
        "出生日期：1990-05-12 00:00:00",
        "生日：2019-03-02",
    ] {
        let (out, _) = mask(&masker(), s, &o);
        if !out.contains("BIRTHDAY") {
            errorf!(t, "{s:?} not masked: {out:?}");
        }
    }
}

// vendorchecks_test.go: the cases with the local model are left out

// A user's word that is part of an MCP server's name stays in the tool's
// server name, which the vendor checks; the same word in the user's
// message is still masked.
#[test]
fn mask_json_keeps_tool_server_names() {
    let o = Options {
        secrets: true,
        personal: true,
        ..words(&["codex_app", "list_threads"])
    };
    let mut t = Errors::default();
    for (name, body, keep) in [
        (
            "responses namespace",
            concat!(
                r#"{"input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"ask codex_app to list threads"}]},"#,
                r#"{"type":"function_call","call_id":"c1","name":"list_threads","namespace":"mcp__codex_app","arguments":"{}"}]}"#,
            ),
            r#""namespace":"mcp__codex_app""#,
        ),
        (
            "responses mcp_call",
            concat!(
                r#"{"input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"ask codex_app to list threads"}]},"#,
                r#"{"type":"mcp_call","id":"mcp_1","server_label":"codex_app","name":"list_threads","arguments":"{}"}]}"#,
            ),
            r#""server_label":"codex_app""#,
        ),
        (
            "anthropic mcp_tool_use",
            concat!(
                r#"{"messages":[{"role":"user","content":"ask codex_app to list threads"},"#,
                r#"{"role":"assistant","content":[{"type":"mcp_tool_use","id":"mcptoolu_1","name":"list_threads","server_name":"codex_app","input":{}}]}]}"#,
            ),
            r#""server_name":"codex_app""#,
        ),
    ] {
        let (out, _) = mask_json(&masker(), body, &o);
        if !out.contains(keep) {
            errorf!(t, "{name}: server's name masked: {out}");
        }
        if !out.contains(r#""ask {{TERM_"#) {
            errorf!(t, "{name}: the word in the message kept: {out}");
        }
    }
}

#[test]
fn a_mobile_number_in_groups_is_masked_too() {
    let m = masker();
    let o = Options {
        personal: true,
        ..Options::default()
    };
    for text in [
        "call me on +86 138 0013 8000 if it fails",
        "手机 138-0013-8000",
        "call 13800138000",
    ] {
        let (out, n) = mask(&m, text, &o);
        assert_eq!(n, 1, "{text} → {out}");
        assert!(!unmarked(&out).contains("8000"), "{out}");
    }
    // A card's digits in fours are not a phone's.
    let (out, _) = mask(&m, "build 1380 0138 0001 2", &o);
    assert!(!out.contains("{{PHONE_"), "{out}");
}
