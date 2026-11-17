//! A value masked once is masked wherever it is next: the Go tests of
//! magpie's internal/redact that hang on what was masked before, ported,
//! each test with one masker as Go's had one store. Go sent a user name as
//! a made-up name; the port sends it as a placeholder, so where Go wanted
//! the made-up name these want the placeholder the name went as first.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use pansy_mask::{Action, Masker, Options};
use regex::Regex;

static PLACEHOLDER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{\{[A-Z][A-Z0-9_]*_[a-z2-7]{8}\}\}").unwrap());

fn masker() -> Masker {
    Masker::new(b"test key")
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

fn secrets() -> Options {
    Options {
        secrets: true,
        ..Options::default()
    }
}

fn personal() -> Options {
    Options {
        personal: true,
        ..Options::default()
    }
}

fn secrets_and_personal() -> Options {
    Options {
        secrets: true,
        personal: true,
        ..Options::default()
    }
}

/// Personal data with the categories of `ids` on.
fn personal_with(ids: &[&str]) -> Options {
    Options {
        personal: true,
        kinds: ids
            .iter()
            .map(|id| (id.to_string(), true))
            .collect::<BTreeMap<_, _>>(),
        ..Options::default()
    }
}

/// `s` as a JSON string.
fn json_string(s: &str) -> String {
    serde_json::to_string(s).unwrap()
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

// known_test.go

// A value masked once goes as its placeholder wherever it is next, in
// whatever words are around it: what the vendor wrote about it, as the
// agent sends it back.
#[test]
fn known_value_masked_in_other_words() {
    let m = masker();
    let o = secrets_and_personal();
    let (secret, email, phone) = ("Zq8hunterPass42", "rina.k@acme-labs.io", "13912345670");
    let (ps, n) = mask(&m, &format!("DB_PASSWORD={secret}"), &o);
    let (pe, _) = mask(&m, &format!("I am {email}"), &o);
    let (pp, _) = mask(&m, &format!("call {phone}"), &o);
    assert!(
        n == 1
            && !format!("{ps}{pe}{pp}").contains(secret)
            && !pe.contains(email)
            && !pp.contains(phone),
        "not masked to begin with: {ps:?} {pe:?} {pp:?}"
    );
    let ps = ps.strip_prefix("DB_PASSWORD=").unwrap();
    let pe = pe.strip_prefix("I am ").unwrap();
    let pp = pp.strip_prefix("call ").unwrap();

    // the vendor's reply as the agent has it back: no rule matches these
    let reply = format!("Your password is {secret}, and I will write to {email}. Or call {phone}!");
    let (out, n) = mask(&m, &reply, &o);
    assert!(
        !out.contains(secret) && !out.contains(email) && !out.contains(phone),
        "a known value went out: {out:?}"
    );
    let want = format!("Your password is {ps}, and I will write to {pe}. Or call {pp}!");
    assert!(
        out == want && n == 3,
        "{n} masked: {out:?}, want the same placeholders as before: {want:?}"
    );
    assert_eq!(restore(&m, &out), reply, "round trip");

    // in a request body, as the next turn's history has it
    let body = format!(
        r#"{{"messages":[{{"role":"assistant","content":"{reply}"}},{{"role":"user","content":"thanks"}}]}}"#
    );
    let (got, n) = mask_json(&m, &body, &o);
    assert!(
        !got.contains(secret) && !got.contains(email) && !got.contains(phone) && n == 3,
        "{n} masked: {got}"
    );

    // only what the settings mask: personal data off, secrets off
    let (out, _) = mask(&m, &reply, &secrets());
    assert!(
        out.contains(email) && out.contains(phone) && !out.contains(secret),
        "personal data off: {out:?}"
    );
    let (out, _) = mask(&m, &reply, &personal());
    assert!(
        out.contains(secret) && !out.contains(email),
        "secrets off: {out:?}"
    );

    let mut t = Errors::default();
    // a piece of a longer number or address is another one
    for s in [
        format!("order 2{phone}"),
        format!("order {phone}9"),
        format!("mail {email}.cn now"),
        format!("mail x{email}"),
    ] {
        let (out, _) = mask(&m, &s, &personal());
        if out.contains(pp) || out.contains(pe) {
            errorf!(t, "{s:?}: masked as the known value: {out:?}");
        }
    }
    // a secret in other punctuation is the secret still; inside a longer
    // word it is another word
    let (out, _) = mask(&m, &format!("`{secret}`/{secret}_v2"), &secrets());
    if out.contains(secret) {
        errorf!(t, "secret left: {out:?}");
    }
    let s = format!("pass{secret}x");
    let (out, _) = mask(&m, &s, &secrets());
    if out != s {
        errorf!(t, "a longer word masked: {out:?}");
    }
}

// A value too short to be told from ordinary text is masked where a rule
// finds it, and nowhere else.
#[test]
fn short_value_not_masked_everywhere() {
    let m = masker();
    let (out, n) = mask(&m, "postgres://app:ab12cd@db/app", &secrets());
    assert!(
        n == 1 && !out.contains("ab12cd"),
        "the rule should mask it: {out:?}"
    );
    let (out, n) = mask(&m, "hex ab12cd0 and ab12cd", &secrets());
    assert!(
        n == 0 && out == "hex ab12cd0 and ab12cd",
        "a short value masked everywhere: {out:?}"
    );
}

// What a vendor sealed has a secret masked before as its placeholder, the
// way the vendor wrote it, though nothing else in the request has it; a
// number of its own it keeps, though one masked before.
#[test]
fn known_in_signed() {
    let m = masker();
    let o = secrets_and_personal();
    let (secret, phone) = ("Wv7otterPass93", "13612345674");
    mask(&m, &format!("DB_PASSWORD={secret}"), &o);
    mask(&m, &format!("call {phone}"), &o);
    let body = format!(
        r#"{{"messages":[{{"role":"assistant","content":[{{"type":"thinking","thinking":"the password is {secret}; order {phone}","signature":"Eqk1"}}]}}]}}"#
    );
    let (out, _) = mask_json(&m, &body, &o);
    assert!(
        !out.contains(secret) && out.contains(&format!("order {phone}")),
        "{out}"
    );
}

// A value held is matched in other words as the same placeholder. Go let
// some values go and indexed the rest again first; nothing here lets a
// value go, so this is what is left of it.
#[test]
fn known_matched_again() {
    let m = masker();
    let secret = "Kp3heronPass71";
    let (p, _) = mask(&m, &format!("API_TOKEN={secret}"), &secrets());
    let (out, _) = mask(&m, &format!("it is {secret}"), &secrets());
    assert_eq!(
        out,
        format!("it is {}", p.strip_prefix("API_TOKEN=").unwrap())
    );
}

// identname_test.go

// A user name a rule found in a home folder is masked again wherever it
// stands as the user's name, but not where code has the same word as a
// name of its own: a parameter, a variable, a JSON key, a part of a
// snake_case or camelCase name.
#[test]
fn user_name_in_code_is_no_user_name() {
    let m = masker();
    let o = personal_with(&["home", "userhost"]);
    let mut t = Errors::default();
    for name in ["developer", "rinakato"] {
        // the system prompt's working folder: the name is known from here on
        let (out, _) = mask(&m, &format!("Working directory: /Users/{name}/proj"), &o);
        let f = first_placeholder(&out).to_owned();
        assert!(
            f.starts_with("{{USERNAME_") && out == format!("Working directory: /Users/{f}/proj"),
            "{name}: home folder sent as {out:?}"
        );
        let r = |s: &str| s.replace("NAME", name);
        for s in [
            // a file read
            r(
                "def run(NAME, NAME_dir: str):\n    NAME_mode = True\n    self.NAME = NAME\n    return NAME\n",
            ),
            r(
                "const NAME = loadUser();\nlet NAMEConfig = {};\nfunction greet(NAME) { return NAME(1) }\n",
            ),
            r(r#"{"NAME": true, "NAME_id": 3, "NAME-config": "x"}"#),
            r(
                "import NAME from './NAME.js'\nfunc New(NAME string, opts Options) {}\nNAME := cfg.NAME\n",
            ),
            // a diff
            r(
                "@@ -1,3 +1,3 @@\n-    NAME_count := 0\n+    NAMECount := 0\n     var NAME int\n     if NAME == 0 {\n",
            ),
        ] {
            let (out, _) = mask(&m, &s, &o);
            if out != s {
                errorf!(t, "{name}: code sent as\n{out}\nwant\n{s}");
            }
        }
        // a tool call's arguments
        let args = json_string(&r(
            r#"{"file_path":"src/app.py","old_string":"def run(NAME):","new_string":"def run(NAME, NAME_dir):"}"#,
        ));
        let body = format!(
            r#"{{"messages":[{{"role":"assistant","tool_calls":[{{"id":"call_1","type":"function","function":{{"name":"edit","arguments":{args}}}}}]}}]}}"#
        );
        let (out, _) = mask_json(&m, &body, &o);
        if out != body {
            errorf!(t, "{name}: arguments sent as\n{out}");
        }

        // and where it is the user's name, it goes as its placeholder
        for (s, want) in [
            ("cat /Users/NAME/.zshrc", "cat /Users/F/.zshrc"),
            ("git log --author=NAME", "git log --author=F"),
            (
                "drwxr-xr-x  5 NAME  staff  160 Nov  2 10:00 proj",
                "drwxr-xr-x  5 F  staff  160 Nov  2 10:00 proj",
            ),
            ("uid=501(NAME) gid=20(staff)", "uid=501(F) gid=20(staff)"),
            (
                "Thanks, NAME! The build is green.",
                "Thanks, F! The build is green.",
            ),
            ("I'm NAME.", "I'm F."),
            (r#"os.chown(path, "NAME")"#, r#"os.chown(path, "F")"#),
            (r#"{"owner": "NAME"}"#, r#"{"owner": "F"}"#),
            ("chown NAME:staff app.log", "chown F:staff app.log"),
        ] {
            let (s, want) = (r(s), want.replace('F', &f));
            let (out, _) = mask(&m, &s, &o);
            if out != want {
                errorf!(t, "{name}: {s:?} sent as {out:?}, want {want:?}");
            }
        }
        let args = json_string(&r(r#"{"command":"ls -la ~NAME && echo NAME"}"#));
        let body = format!(
            r#"{{"messages":[{{"role":"assistant","tool_calls":[{{"id":"call_2","type":"function","function":{{"name":"bash","arguments":{args}}}}}]}}]}}"#
        );
        let (out, _) = mask_json(&m, &body, &o);
        if out.contains(name) || !out.contains(&format!("~{f} && echo {f}")) {
            errorf!(t, "{name}: arguments sent as\n{out}");
        }
    }
}

// A "User:" line's value in code is a variable's or a type's name, not a
// login. Written out (quoted, or a login of words of its own), it is
// masked.
#[test]
fn user_line_in_code_is_no_user_name() {
    let m = masker();
    let o = personal_with(&["userhost"]);
    let mut t = Errors::default();
    for s in [
        "user = current_user",
        "    user: UserInfo",
        "login = loginName",
        "username = target_name",
        "user = user_obj",
        "type Session struct {\n\tUser   UserProfile\n}",
        "export interface Props {\n  user: CurrentUser\n}",
    ] {
        let (out, n) = mask(&m, s, &o);
        if n != 0 {
            errorf!(t, "{s:?} sent as {out:?}");
        }
    }
    for (s, v) in [
        ("User: deploy_bot", "deploy_bot"),
        ("USER=rinak", "rinak"),
        (r#"user: "current_user""#, "current_user"),
        ("Host box\n  User rinak", "rinak"),
        ("user = john_doe", "john_doe"),
    ] {
        let (out, n) = mask(&m, s, &o);
        if n != 1 || out.contains(v) {
            errorf!(t, "{s:?} sent as {out:?}, want {v} masked");
        }
    }
}
