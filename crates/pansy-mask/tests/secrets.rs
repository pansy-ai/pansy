//! The secrets the rules built in find, and what only looks like one: the
//! Go tests of magpie's internal/redact, ported. The cases that hang on
//! the local model, on made-up keys (Shape) or on what Go keeps inside the
//! package are left out.
//!
//! Go kept one masker for the whole package; here each case has its own
//! unless the case needs a value masked before it.

use std::sync::LazyLock;

use pansy_mask::{Action, CATEGORIES, Masker, Options};
use regex::Regex;
use sha2::{Digest, Sha256, Sha512};

// Just enough Ed25519 to make a Solana key whose second half is the first
// half's public key, as the masking checks one. The port's own, tested
// there against RFC 8032's vectors.
#[path = "../src/ed25519.rs"]
mod ed25519;

static PLACEHOLDER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{\{[A-Z][A-Z0-9_]*_[a-z2-7]{8}\}\}").unwrap());

const OPENAI_KEY: &str = "sk-proj-abcdEFGH1234ijklMNOP5678qrst";
const GH_TOKEN: &str = "ghp_0123456789abcdefghijABCDEFGHIJ0123";

fn masker() -> Masker {
    Masker::new(b"test key")
}

/// `s` without its placeholders. A placeholder's suffix is a keyed hash, so
/// it can spell a short value a test checks is gone.
fn unmarked(s: &str) -> String {
    PLACEHOLDER.replace_all(s, "").into_owned()
}

/// The first placeholder in `s`, or "".
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

/// Masks a request body, as magpie's MaskJSON did.
fn mask_json(m: &Masker, body: &str, o: &Options) -> (String, usize) {
    let out = m.mask_json(body.as_bytes(), o);
    let n = out.hits.iter().filter(|h| h.action != Action::Warn).count();
    (String::from_utf8(out.body).unwrap(), n)
}

fn restore(m: &Masker, s: &str) -> String {
    m.restore(s).0
}

fn restore_json(m: &Masker, s: &str) -> String {
    String::from_utf8(m.restore_json(s.as_bytes()).0).unwrap()
}

fn secrets() -> Options {
    Options {
        secrets: true,
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

/// Every category on, the ones off by default (the IP address, the user
/// and host) too.
fn everything() -> Options {
    let mut o = secrets_and_personal();
    for c in CATEGORIES {
        o.kinds.insert(c.id.to_owned(), true);
    }
    o
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

/// Says no case of `ins` is masked with `o`, each with a masker of its own.
fn none_masked(t: &mut Errors, ins: &[&str], o: &Options) {
    for &s in ins {
        let (out, n) = mask(&masker(), s, o);
        if n != 0 || out != s {
            errorf!(t, "{s:?}: {n} masked: {out:?}");
        }
    }
}

/// Says the value of each case is masked, one value, with a masker of its
/// own.
fn one_masked(t: &mut Errors, cases: &[(&str, &str)], o: &Options) {
    for &(s, v) in cases {
        let (out, n) = mask(&masker(), s, o);
        if n != 1 || unmarked(&out).contains(v) {
            errorf!(t, "{s:?}: {n} masked: {out:?}");
        }
    }
}

// redact_test.go

#[test]
fn mask_finds_secrets_and_personal_data() {
    struct Case {
        input: String,
        o: Options,
        gone: Vec<String>,
        kept: Vec<&'static str>,
        count: usize,
    }
    let personal = secrets_and_personal;
    let case = |input: &str, o: Options, gone: &[&str], kept: &[&'static str], count: usize| Case {
        input: input.to_owned(),
        o,
        gone: gone.iter().map(|s| s.to_string()).collect(),
        kept: kept.to_vec(),
        count,
    };
    let cases = [
        case(
            &format!("export OPENAI_API_KEY={OPENAI_KEY}"),
            Options::default(),
            &[OPENAI_KEY],
            &["export OPENAI_API_KEY="],
            1,
        ),
        case(
            &format!("token {GH_TOKEN} and AKIAIOSFODNN7EXAMPLE"),
            Options::default(),
            &[GH_TOKEN, "AKIAIOSFODNN7EXAMPLE"],
            &[],
            2,
        ),
        case(
            "postgres://app:s3cretPass@db.internal:5432/app",
            Options::default(),
            &["s3cretPass"],
            &["postgres://app:", "@db.internal"],
            1,
        ),
        case(
            "postgres://app:${DB_PASS}@db/app",
            Options::default(),
            &[],
            &["${DB_PASS}"],
            0,
        ),
        case(
            r#"DB_PASSWORD="hunter2hunter2x9""#,
            Options::default(),
            &["hunter2hunter2x9"],
            &[],
            1,
        ),
        case(
            "password = getPassword()",
            Options::default(),
            &[],
            &["getPassword()"],
            0,
        ),
        case(
            "the tokenizer splits words",
            Options::default(),
            &[],
            &["tokenizer"],
            0,
        ),
        case(
            "-----BEGIN RSA PRIVATE KEY-----\nMIIEow\n-----END RSA PRIVATE KEY-----",
            Options::default(),
            &["MIIEow"],
            &[],
            1,
        ),
        // personal data off
        case(
            "mail me: jane.doe@acme.io",
            Options::default(),
            &[],
            &["jane.doe@acme.io"],
            0,
        ),
        case(
            "mail me: jane.doe@acme.io or 13812345678",
            personal(),
            &["jane.doe@acme.io", "13812345678"],
            &[],
            2,
        ),
        case(
            "see user@example.com",
            personal(),
            &[],
            &["user@example.com"],
            0,
        ),
        case(
            "id 11010519491231002X card 4111 1111 1111 1111",
            personal(),
            &["11010519491231002X", "4111 1111 1111 1111"],
            &[],
            2,
        ),
        // fails Luhn
        case(
            "order 4111111111111112",
            personal(),
            &[],
            &["4111111111111112"],
            0,
        ),
        case(
            "Project Nightjar ships",
            Options {
                secrets: true,
                words: vec!["Nightjar".into()],
                ..Options::default()
            },
            &["Nightjar"],
            &[],
            1,
        ),
    ];
    let mut t = Errors::default();
    for mut c in cases {
        if !c.o.personal && c.o.words.is_empty() {
            c.o.secrets = true;
        }
        let m = masker();
        let (out, n) = mask(&m, &c.input, &c.o);
        if n != c.count {
            errorf!(t, "{:?}: {n} masked, want {}: {out:?}", c.input, c.count);
        }
        for g in &c.gone {
            if unmarked(&out).contains(g.as_str()) {
                errorf!(t, "{:?}: {g:?} still in {out:?}", c.input);
            }
        }
        for k in &c.kept {
            if !out.contains(k) {
                errorf!(t, "{:?}: {k:?} gone from {out:?}", c.input);
            }
        }
        let back = restore(&m, &out);
        if back != c.input {
            errorf!(t, "round trip: {:?} -> {out:?} -> {back:?}", c.input);
        }
    }
    // the same value, the same placeholder
    let m = masker();
    let (a, _) = mask(&m, &format!("k={OPENAI_KEY}"), &secrets());
    let (b, _) = mask(&m, &format!("other {OPENAI_KEY}"), &secrets());
    let (pa, pb) = (first_placeholder(&a), first_placeholder(&b));
    if pa.is_empty() || pa != pb {
        errorf!(t, "placeholders differ: {a:?} {b:?}");
    }
}

// In a tool call's arguments a field named for a secret holds one, whatever
// its value looks like. A name that only ends in key is no secret: a sort
// key, a cache key, a public key stay, and so does a variable.
#[test]
fn mask_json_named_secrets() {
    let cases = [
        ("api_key", "Qm27wVb9nLx4tRk8Ps3d", true),
        ("client_secret", "randomvalue42x", true),
        ("db_pass", "Hr7jTm2qwXz94", true),
        ("access_key_id", "AKIAIOSFODNN7EXAMPLE", true),
        ("Authorization", "Bearer zq83Mw2vNx7kRt5", true),
        ("Cookie", "sid=Yn41Pc8wLd2mQs6", true),
        ("apiKey", "plainrandomvalue1", true),
        ("sort_key", "created_at_2024", false),
        ("cache_key", "lru_items_01", false),
        ("prompt_cache_key", "conv_12345678", false),
        ("public_key", "04a1b2c3d4e5f6a7", false),
        ("password", "${DB_PASSWORD}", false),
        ("password", "short", false),
    ];
    let wires = [
        (
            "anthropic tool_use",
            r#"{"messages":[{"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"deploy","input":{"$K":"$V"}}]}]}"#,
        ),
        (
            "gemini functionCall",
            r#"{"contents":[{"role":"model","parts":[{"functionCall":{"name":"deploy","args":{"cfg":{"$K":"$V"}}}}]}]}"#,
        ),
        (
            "gemini functionResponse",
            r#"{"contents":[{"role":"user","parts":[{"functionResponse":{"name":"env","response":{"$K":"$V"}}}]}]}"#,
        ),
    ];
    let mut t = Errors::default();
    for (key, value, gone) in cases {
        let m = masker();
        for (wire, template) in wires {
            let body = template.replace("$K", key).replace("$V", value);
            let (out, n) = mask_json(&m, &body, &secrets());
            if !gone {
                if n != 0 || out != body {
                    errorf!(t, "{wire} {key}: {n} masked: {out}");
                    continue;
                }
            } else if out.contains(value) || !out.contains("{{SECRET_") {
                errorf!(t, "{wire} {key}: not masked as a secret: {out}");
                continue;
            }
            let back = restore_json(&m, &out);
            if back != body {
                errorf!(t, "{wire} {key}: restored {back}");
            }
        }
    }
}

// A key the vendor's protocol names is the vendor's to read, whatever it is
// called: only a value's shape masks there. With secret masking off no
// name masks anything.
#[test]
fn mask_json_secret_names_only_in_tool_args() {
    let mut t = Errors::default();
    for body in [
        r#"{"mcp_servers":[{"type":"url","url":"https://mcp.example.com/sse","name":"ex","authorization_token":"opaquevalue42abc"}],"messages":[{"role":"user","content":"hi"}]}"#,
        r#"{"api_key":"opaquevalue42abc","messages":[{"role":"user","content":"hi"}]}"#,
        r#"{"metadata":{"session_token":"opaquevalue42abc"}}"#,
    ] {
        let (out, n) = mask_json(&masker(), body, &secrets());
        if n != 0 || out != body {
            errorf!(t, "a protocol key was masked by its name: {n} {out}");
        }
    }
    let body = r#"{"messages":[{"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"deploy","input":{"api_key":"opaquevalue42abc"}}]}]}"#;
    let (out, n) = mask_json(
        &masker(),
        body,
        &Options {
            personal: true,
            ..Options::default()
        },
    );
    if n != 0 || out != body {
        errorf!(t, "secret masking off, a name masked: {n} {out}");
    }
}

// An id goes as the vendor gave it; the same digits in text are masked.
#[test]
fn mask_json_keeps_ids() {
    let o = secrets_and_personal();
    // 19 digits that pass Luhn
    let id = "call_-6090648166611204696";
    let mut t = Errors::default();
    for k in [
        "id",
        "tool_call_id",
        "tool_use_id",
        "call_id",
        "approval_request_id",
        "toolCallId",
    ] {
        let body = format!(r#"{{"{k}":"{id}"}}"#);
        let (out, _) = mask_json(&masker(), &body, &o);
        if out != body {
            errorf!(t, "{k} masked: {out}");
        }
    }
    let digits = &id[6..];
    let (out, _) = mask_json(&masker(), &format!(r#"{{"content":"card {digits}"}}"#), &o);
    if out.contains(digits) {
        errorf!(t, "the same digits in text should be masked: {out}");
    }
}

// A tool call's arguments are the agent's: a key there named like a
// signature or an id doesn't keep its value, or its neighbours', from the
// rules.
#[test]
fn mask_json_tool_args_masked() {
    let o = secrets_and_personal();
    let mut t = Errors::default();
    for (name, body) in [
        (
            "signature key",
            r#"{"messages":[{"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"send_email","input":{"signature":"Best, Bob","body":"DB_PASSWORD=hunter2abc1 call 13800138000"}}]}]}"#,
        ),
        (
            "id keys",
            r#"{"messages":[{"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"lookup","input":{"user_id":"DB_PASSWORD=hunter2abc1","phone_id":"13800138000"}}]}]}"#,
        ),
        (
            "gemini args",
            r#"{"contents":[{"role":"model","parts":[{"functionCall":{"name":"lookup","args":{"thoughtSignature":"x","customer_phone":"13800138000","note":"DB_PASSWORD=hunter2abc1"}}}]}]}"#,
        ),
        (
            "text block",
            r#"{"messages":[{"role":"user","content":[{"type":"text","text":"DB_PASSWORD=hunter2abc1 call 13800138000","signature":"x"}]}]}"#,
        ),
    ] {
        let (out, _) = mask_json(&masker(), body, &o);
        if out.contains("hunter2abc1") || out.contains("13800138000") {
            errorf!(t, "{name}: a value went out: {out}");
        }
        if !out.contains(r#""toolu_1""#) && body.contains("toolu_1") {
            errorf!(t, "{name}: the call's id changed: {out}");
        }
    }
}

// url_password_test.go

// The README's and the workflow's lines of a PostGIS test, whose URL's
// password is its user and its database too.
const GEO_URL_LINES: &str = concat!(
    "GEODATAFUSION_POSTGIS_URL=postgresql://geodatafusion:geodatafusion@127.0.0.1:55432/geodatafusion \\\n",
    "  GEODATAFUSION_POSTGIS_URL: postgresql://geodatafusion:geodatafusion@127.0.0.1:55432/geodatafusion\n",
);

// A summary an agent wrote of the same project.
const GEO_SUMMARY: &str = concat!(
    "**当前状态**\n",
    "- 工作树: `/Users/xuanwo/Code/worktrees/f745/geodatafusion`\n",
    "- 分支: `xuanwo/sqllogictest-coverage`\n",
    "- 构建固定使用 `CARGO_TARGET_DIR=/Users/xuanwo/.cache/geodatafusion-goal/target`、`CARGO_BUILD_JOBS=4`、`CARGO_INCREMENTAL=0`\n",
    "**未提交改动**\n",
    "- `rust/geodatafusion/tests/geos_runtime.rs`: 新增 GEOS runtime 最低版本断言; 设置 `GEODATAFUSION_EXPECT_GEOS_VERSION` 时要求精确版本。\n",
    "- `geodatafusion.toml`: `ST_LineMerge`、`ST_ConcaveHull` 从 verified 降为 partial\n",
    "```bash\n",
    "export GEOS_LIB_DIR=/Users/xuanwo/.cache/geodatafusion-goal/geos-3.11.5-prefix/lib\n",
    "CARGO_TARGET_DIR=/Users/xuanwo/.cache/geodatafusion-goal/target \\\n",
    "  cargo test -p geodatafusion --features geos-3_11 --locked --lib --no-run\n",
    "BIN=$(ls -t /Users/xuanwo/.cache/geodatafusion-goal/target/debug/deps/geodatafusion-* | grep -v '\\.d$' | head -1)\n",
    "```\n",
);

// A URL's password that is its user or its database hides nothing by being
// masked, so it goes as written, and the project's name it is goes on
// unknown.
#[test]
fn url_password_written_elsewhere_goes_as_written() {
    let m = masker();
    let o = secrets_and_personal();
    let mut t = Errors::default();
    let (out, n) = mask(&m, GEO_URL_LINES, &o);
    if out != GEO_URL_LINES || n != 0 {
        errorf!(t, "{n} masked: {out:?}");
    }
    let (out, n) = mask(&m, GEO_SUMMARY, &o);
    if out != GEO_SUMMARY || n != 0 {
        errorf!(t, "{n} masked in the summary: {out:?}");
    }
    let body = concat!(
        r#"{"input":[{"type":"function_call","name":"exec_command","arguments":"{\"cmd\":\"psql postgresql://geodatafusion:geodatafusion@127.0.0.1:55432/geodatafusion -c 'select 1'\"}","call_id":"call_1"},"#,
        r#"{"type":"message","role":"user","content":[{"type":"input_text","text":"cargo test -p geodatafusion"}]}]}"#,
    );
    let (out, n) = mask_json(&m, body, &o);
    if out != body || n != 0 {
        errorf!(t, "{n} masked in a body: {out}");
    }
    // a password that is another part of its URL: a user's, a host's
    // label, the database
    for s in [
        "postgres://postgres:postgres@localhost:5432/app",
        "mysql://root:root@127.0.0.1/shop",
        "amqp://guest:guest@rabbit:5672/",
        "mysql://app:shopdb@db.internal/shopdb?ssl=true",
        "redis://:cachebox@cachebox:6379/0",
        r#""url": "postgresql://geo:geo@db/geo""#,
    ] {
        let (out, n) = mask(&m, s, &o);
        if out != s || n != 0 {
            errorf!(t, "{n} masked: {s:?} went as {out:?}");
        }
    }
}

// A password not written elsewhere in its URL is masked there, and as a
// known value wherever it is next.
#[test]
fn url_password_still_masked() {
    let m = masker();
    let o = secrets_and_personal();
    let ph = r"\{\{PASSWORD_[a-z2-7]{8}\}\}";
    let mut t = Errors::default();
    for (s, want) in [
        (
            "postgres://app:hunter2pass@db.acme-corp.cn:5432/x",
            format!(r"^postgres://app:{ph}@db\.acme-corp\.cn:5432/x$"),
        ),
        (
            "postgresql://geo:opensesame@127.0.0.1:55432/geodata",
            format!(r"^postgresql://geo:{ph}@127\.0\.0\.1:55432/geodata$"),
        ),
        // a part of a part is no part: the user geo, the database geodb
        (
            "postgresql://geo:geod@127.0.0.1/geodb",
            format!(r"^postgresql://geo:{ph}@127\.0\.0\.1/geodb$"),
        ),
        (
            "REDIS_URL=redis://:r3disPass@cache.acme-corp.cn:6379/0",
            format!(r"^REDIS_URL=redis://:{ph}@cache\.acme-corp\.cn:6379/0$"),
        ),
    ] {
        let (out, _) = mask(&m, s, &o);
        if !Regex::new(&want).unwrap().is_match(&out) {
            errorf!(t, "{s:?} went as {out:?}");
        }
    }
    // in a tool call's arguments too
    let body = r#"{"input":[{"type":"function_call","name":"exec_command","arguments":"{\"cmd\":\"psql postgresql://geo:Tr0ub4dor3@10.0.0.5/geodata\"}","call_id":"call_1"}]}"#;
    let (out, n) = mask_json(&m, body, &o);
    if out.contains("Tr0ub4dor3") || n != 1 {
        errorf!(t, "{n} masked in a tool call: {out}");
    }
    // known: the word goes masked in other words afterwards
    let (out, _) = mask(&m, "cd ~/src/opensesame && ls", &o);
    if out.contains("opensesame") {
        errorf!(t, "a known password went as written: {out:?}");
    }
}

// cloudflare_test.go

// Made-up credentials in Cloudflare's documented shapes.
const CF_USER_TOK: &str = concat!("cfut_Kq3vXm9ZpL2wRt7YbN4cHd8sJf6GaE1uVoTi5MxA", "9f3c1b7e");
const CF_ACCT_TOK: &str = concat!("cfat_Zr8dQw2LmN5vXc7BtY1pHk4sJg6FaE3uVoTi9MxP", "02a8d4c6");
const CF_GLOBAL_NEW: &str = concat!("cfk_Hb4nXw7ZpL2qRt9YcN3vMd8sJf6GaE1uKoTi5MyQ", "5e1b7a90");
const CF_ORIGIN_CA: &str = concat!(
    "v1.0-3f9a1c7e5b2d4a6c8e0f1a2b-",
    "4d6f8a0c2e4b6d8f0a1c3e5a7b9d1f3a5c7e9b1d3f5a7c9e1b3d5f7a9c1e3b5d7f9a1c3e5b7d9f1a3c5e7b9d1f3a5c7e9b1d3f5a7c9e1b3d5f7a9c1e3b5d7f",
);
// 37 hex digits
const CF_GLOBAL_OLD: &str = "c2547eb745079dac9320b638f5e225cf483cc";
// 40 letters and digits
const CF_LEGACY_TOK: &str = "8M7wS6hCpXQ5QeL2Ek4xj3Bt9nVa1uRyZfDgHi0O";
// 32 hex digits
const CF_R2_KEY_ID: &str = "9d1f3b5a7c2e4a6c8e0b1d3f5a7c9e2b";
// 64 hex digits
const CF_R2_SECRET: &str = "a3f5c7e9b1d3f5a7c9e1b3d5f7a9c1e3b5d7f9a1c3e5b7d9f1a3c5e7b9d1f3a5";

/// A tunnel token as cloudflared has one: base64 of its account, tunnel
/// and secret as JSON. These are made up.
fn cf_tunnel_token() -> String {
    base64_std(br#"{"a":"0a4c3f9e7b1d2c6e8f5a9047b3c1e2d4","t":"7c9e1b3a-4d6f-4a8c-9e0f-1a2b3c4d5e6f","s":"ZXhhbXBsZXNlY3JldHZhbHVlMTIzNDU2Nzg5MA=="}"#)
}

#[test]
fn cloudflare_credentials() {
    let tunnel = cf_tunnel_token();
    let oauth = format!("cfoat_{}12345678", "aB3dE5gH7j".repeat(5));
    let cases: Vec<(&str, String, &str)> = vec![
        (
            "user token",
            format!("token is {CF_USER_TOK} ok"),
            CF_USER_TOK,
        ),
        ("account token", format!("use {CF_ACCT_TOK}."), CF_ACCT_TOK),
        ("global key", format!("key={CF_GLOBAL_NEW}"), CF_GLOBAL_NEW),
        (
            "origin ca",
            format!("X-Auth-User-Service-Key: {CF_ORIGIN_CA}"),
            CF_ORIGIN_CA,
        ),
        (
            "origin ca bare",
            format!("see {CF_ORIGIN_CA} here"),
            CF_ORIGIN_CA,
        ),
        ("wrangler oauth", oauth.clone(), "cfoat_"),
        (
            "tunnel token flag",
            format!("cloudflared tunnel run --token {tunnel}"),
            &tunnel,
        ),
        (
            "tunnel token env",
            format!("TUNNEL_TOKEN={tunnel}"),
            &tunnel,
        ),
        ("tunnel token bare", format!("run {tunnel} now"), &tunnel),
        (
            "legacy token env",
            format!("CLOUDFLARE_API_TOKEN={CF_LEGACY_TOK}"),
            CF_LEGACY_TOK,
        ),
        (
            "legacy token cf",
            format!("CF_API_TOKEN: {CF_LEGACY_TOK}"),
            CF_LEGACY_TOK,
        ),
        (
            "bearer",
            format!(
                "curl -H 'Authorization: Bearer {CF_LEGACY_TOK}' https://api.cloudflare.com/client/v4/user/tokens/verify"
            ),
            CF_LEGACY_TOK,
        ),
        (
            "global key header",
            format!("curl -H 'X-Auth-Key: {CF_GLOBAL_OLD}' -H 'X-Auth-Email: a@b.co'"),
            CF_GLOBAL_OLD,
        ),
        (
            "global key env",
            format!("CLOUDFLARE_API_KEY={CF_GLOBAL_OLD}"),
            CF_GLOBAL_OLD,
        ),
        (
            "r2 secret env",
            format!("R2_SECRET_ACCESS_KEY={CF_R2_SECRET}"),
            CF_R2_SECRET,
        ),
        (
            "r2 aws env",
            format!(
                "AWS_ACCESS_KEY_ID={CF_R2_KEY_ID}\nAWS_SECRET_ACCESS_KEY={CF_R2_SECRET}\nendpoint https://acc.r2.cloudflarestorage.com"
            ),
            CF_R2_SECRET,
        ),
        (
            "origin ca env",
            format!("CLOUDFLARE_ORIGIN_CA_KEY={CF_ORIGIN_CA}"),
            CF_ORIGIN_CA,
        ),
    ];
    // One masker for all, as Go had: the bearer row is masked only as the
    // legacy token the rows before it made known. No rule takes an
    // Authorization header in the middle of a line, in Go or here.
    let m = masker();
    let mut t = Errors::default();
    for (name, s, gone) in &cases {
        let (out, n) = mask(&m, s, &secrets());
        if unmarked(&out).contains(gone) || n == 0 {
            errorf!(t, "{name}: not masked ({n}): {out:?}");
        }
    }
}

#[test]
fn cloudflare_lookalikes_kept() {
    let sha1 = "da39a3ee5e6b4b0d3255bfef95601890afd80709";
    let sha256 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    let md5 = "d41d8cd98f00b204e9800998ecf8427e";
    let b62 = "Zm9vYmFyYmF6cXV4Z3JhdWx0aGVsbG93b3JsZDEyMzQ1Ng";
    let ins = [
        format!("commit {sha1}"),
        format!("sha256: {sha256}"),
        format!("etag {md5}"),
        format!("digest {sha256} and {sha1}"),
        format!("id {b62}"),
        "version v1.0-beta.2 released, v1.0-rc1".to_owned(),
        format!("cloudflare zone id {md5} in the dashboard"),
        format!("cf account {CF_R2_KEY_ID}"),
        format!("R2_ACCESS_KEY_ID={CF_R2_KEY_ID}"),
        format!("AWS_ACCESS_KEY_ID={CF_R2_KEY_ID}"),
        "cfut_ is the prefix of user tokens".to_owned(),
        "cfk_short".to_owned(),
        format!("Image digest sha256:{sha256}"),
        format!("CLOUDFLARE_ACCOUNT_ID={CF_R2_KEY_ID}"),
        "eyJhbGciOiJub25lIn0 is a jwt header".to_owned(),
    ];
    let mut t = Errors::default();
    none_masked(
        &mut t,
        &ins.iter().map(String::as_str).collect::<Vec<_>>(),
        &secrets(),
    );
}

// zhpassword_test.go

/// Says each case masks one value, keeps the words around it, and comes
/// back as it was.
fn masked_and_kept(t: &mut Errors, cases: &[(&str, &str, &[&str])]) {
    for &(s, gone, kept) in cases {
        let m = masker();
        let (out, n) = mask(&m, s, &secrets());
        if n != 1 || unmarked(&out).contains(gone) {
            errorf!(t, "{s:?}: {n} masked -> {out:?}");
        }
        for k in kept {
            if !out.contains(k) {
                errorf!(t, "{s:?}: {k:?} gone from {out:?}");
            }
        }
        let back = restore(&m, &out);
        if back != s {
            errorf!(t, "round trip: {s:?} -> {out:?} -> {back:?}");
        }
    }
}

/// Says a request body with one value in it has it masked, and put back.
fn body_masked(t: &mut Errors, body: &str, v: &str) {
    let m = masker();
    let (out, n) = mask_json(&m, body, &secrets());
    if n != 1 || out.contains(v) || restore_json(&m, &out) != body {
        errorf!(t, "body: {n} {out}");
    }
}

// A password written after its name in Chinese, Japanese or Korean is
// masked, a short one and one of digits only too. The name, the words
// around it and a sentence's closing stop stay.
#[test]
fn cjk_password_masked() {
    let mut t = Errors::default();
    masked_and_kept(
        &mut t,
        &[
            ("密码：786kih", "786kih", &["密码："]),
            (
                "服务器密码是 hunter2，记得改",
                "hunter2",
                &["服务器密码是 ", "，记得改"],
            ),
            ("WiFi 密碼: 12345678。", "12345678", &["密碼: ", "。"]),
            ("口令=Abc!2024.", "Abc!2024", &["口令=", "."]),
            (
                "パスワードは pa55word です",
                "pa55word",
                &["パスワードは ", " です"],
            ),
            ("비밀번호: qwer1234", "qwer1234", &["비밀번호: "]),
            (
                r#"{"text":"账号 admin 密码 \"s3cr3t\""}"#,
                "s3cr3t",
                &["账号 admin 密码 "],
            ),
        ],
    );
    body_masked(
        &mut t,
        r#"{"messages":[{"role":"user","content":"密码：786kih"}]}"#,
        "786kih",
    );
    // no value, a placeholder, words, too short
    none_masked(
        &mut t,
        &[
            "请输入密码：",
            "密码：${DB_PASS}",
            "密码：****",
            "忘记密码了怎么办",
            "修改密码功能",
            "密码：abc",
        ],
        &secrets(),
    );
}

// A key, a secret or a token written after its name as a Chinese message
// has it is masked, by a Chinese name or an English one before 为, 為, 是
// or ：.
#[test]
fn cjk_secret_names_masked() {
    let mut t = Errors::default();
    masked_and_kept(
        &mut t,
        &[
            ("密钥为 a8f3k2m9q1", "a8f3k2m9q1", &["密钥为 "]),
            ("apikey 为 AKdx93kfLq0z", "AKdx93kfLq0z", &["apikey 为 "]),
            ("密钥是 7f3e9c2ab41d", "7f3e9c2ab41d", &["密钥是 "]),
            ("密钥：Zt7qP2vX9wLm", "Zt7qP2vX9wLm", &["密钥："]),
            ("秘钥 q8w7e6r5t4y3", "q8w7e6r5t4y3", &["秘钥 "]),
            ("私钥是 MIIEvQIBADANBgkq", "MIIEvQIBADANBgkq", &["私钥是 "]),
            (
                "访问令牌：tk_8d7f6g5h4j3k",
                "tk_8d7f6g5h4j3k",
                &["访问令牌："],
            ),
            (
                "token 是 a1b2c3d4e5f6，别外传",
                "a1b2c3d4e5f6",
                &["token 是 "],
            ),
            (
                "我的 API Key 为 xk-2f9d8s7a6q5w。",
                "xk-2f9d8s7a6q5w",
                &["API Key 为 "],
            ),
            ("API Key：xk2f9d8s7a6q5w", "xk2f9d8s7a6q5w", &["API Key："]),
            ("账号密码 admin123", "admin123", &["账号密码 "]),
            ("口令是 Zx!2025q", "Zx!2025q", &["口令是 "]),
            ("密码为 hunter22", "hunter22", &["密码为 "]),
            ("金鑰為 9Kd8s7Fq6w5e", "9Kd8s7Fq6w5e", &["金鑰為 "]),
            ("密鑰：Pq9w8e7r6t5y", "Pq9w8e7r6t5y", &["密鑰："]),
            (
                "access_token 是 ya29a0AfH6SMB",
                "ya29a0AfH6SMB",
                &["access_token 是 "],
            ),
            (
                "Secret 為「s3cr3tValue9」",
                "s3cr3tValue9",
                &["Secret 為「"],
            ),
            ("WiFi密码：20261025", "20261025", &["WiFi密码："]),
            ("admin密钥是k9j8h7g6f5", "k9j8h7g6f5", &["admin密钥是"]),
        ],
    );
    body_masked(
        &mut t,
        r#"{"messages":[{"role":"user","content":"apikey 为 AKdx93kfLq0z"}]}"#,
        "AKdx93kfLq0z",
    );
    // no value, words about keys, a placeholder, a number, a word, too short
    none_masked(
        &mut t,
        &[
            "请输入密钥",
            "请输入密钥：",
            "密钥管理",
            "忘记密钥了怎么办",
            "令牌桶算法",
            "私钥文件放在哪",
            "密钥：${API_KEY}",
            "密钥：****",
            "token 是 ****",
            "apikey 为 ${OPENAI_API_KEY}",
            "RSA 密钥为 2048 位",
            "token 是 expired",
            "这个 key 是 value",
            "token 是 abc",
            "monkey 是 动物",
            "key 是什么",
            "API Key 为空",
        ],
        &secrets(),
    );
}

// A key of digits only after its name and 为, 是, ：, : or = is masked
// when it has six digits or more; a number that counts something is left.
// The names of PINs and codes are masked whatever their length.
#[test]
fn cjk_digit_keys_masked() {
    let mut t = Errors::default();
    masked_and_kept(
        &mut t,
        &[
            ("密钥为12231241", "12231241", &["密钥为"]),
            ("密钥是 12231241，记一下", "12231241", &["密钥是 "]),
            ("密钥：12231241", "12231241", &["密钥："]),
            ("秘钥=12231241", "12231241", &["秘钥="]),
            ("令牌为「20261025」", "20261025", &["令牌为「"]),
            ("金鑰為 12231241。", "12231241", &["金鑰為 "]),
            ("トークンは 12231241", "12231241", &["トークンは "]),
            ("토큰: 12231241", "12231241", &["토큰: "]),
            ("token 为 12231241", "12231241", &["token 为 "]),
            ("apikey：12231241", "12231241", &["apikey："]),
            ("API Key 是 9988776655", "9988776655", &["API Key 是 "]),
            ("secret 為 123456", "123456", &["secret 為 "]),
            ("pin 为 123456", "123456", &["pin 为 "]),
            ("PIN码：1234", "1234", &["PIN码："]),
            ("pin码是 8520", "8520", &["pin码是 "]),
            ("信用卡安全码：8765", "8765", &["安全码："]),
            (
                "邮箱授权码为 abcdefghijklmnop",
                "abcdefghijklmnop",
                &["授权码为 "],
            ),
            ("暗証番号は 4321", "4321", &["暗証番号は "]),
            ("비번: 1q2w3e4r", "1q2w3e4r", &["비번: "]),
            // the password names took digits only already
            ("密码为12231241", "12231241", &["密码为"]),
            ("口令：12231241", "12231241", &["口令："]),
        ],
    );
    body_masked(
        &mut t,
        r#"{"messages":[{"role":"user","content":"密钥为12231241"}]}"#,
        "12231241",
    );
    // too short, a size, a time, a count, nothing between the name and it,
    // code
    none_masked(
        &mut t,
        &[
            "RSA 密钥为 2048 位",
            "密钥为 12345",
            "令牌有效期是 86400",
            "令牌是 300000 毫秒",
            "密钥长度为 4096000 位",
            "token 是 3600000 ms",
            "key 是 1048576 byte",
            "令牌桶容量为 100000",
            "密钥 55661237",
            "spin 是 778812",
            "pin 为 1934",
            "const token = 44102939;",
            r#"{"token": 44102939}"#,
            "maxTokens: 128000",
        ],
        &secrets(),
    );
}

// fieldvalue_test.go

// A place in a file goes as written, though the file's name has token,
// secret or password in it.
#[test]
fn line_range_is_no_secret() {
    let mut t = Errors::default();
    none_masked(
        &mut t,
        &[
            "看一下 internal/auth/token.go:L643-669.",
            "L643-669.",
            "参考 src/secret_store.ts:L643-669. 和 L12-40",
            "见 tokenizer.rs:L643-669。",
            "cmd/apikey/main.go:L643-669, cmd/apikey/main.go:L12-40.",
            "see internal/redact/password_test.go:643-669 and #L643-L669",
            "src/credentials.py:L12-L40",
            "token_test.go:p2fa9c1d",
            "token.go:-1-2-3a",
            "internal/auth/token.go:L12-40",
        ],
        &secrets_and_personal(),
    );
}

// A name made of words, one of them token, secret or password, after = or
// : is a function's or a variable's.
#[test]
fn name_in_code_is_no_secret() {
    let mut t = Errors::default();
    none_masked(
        &mut t,
        &[
            "const accessToken2 = refreshToken2;",
            "let secretV2 = loadSecretV2;",
            "secretKey:=base64Encode(raw)",
            "password = hashPassword256;",
            "self.token = generate_token_v2",
            r#"TOKEN_V2 = "TOKEN_V2""#,
            "tokenCount = countTokens4o",
            "  secret: env.SECRET_V2",
            "max_tokens=max_tokens_v3",
            "access_token = get_access_token_v2",
            "passwordInput.value = newPassword2",
            "fn(token=token_v2)",
            r#"const TOKEN_KEY = "access_token_v2""#,
        ],
        &secrets(),
    );
}

// A path to where a secret is kept goes as written, and so does a plugin's
// skill.
#[test]
fn path_and_skill_are_no_secret() {
    let mut t = Errors::default();
    none_masked(
        &mut t,
        &[
            "PRIVATE_KEY_PATH=~/.ssh/id_ed25519",
            "GOOGLE_APPLICATION_CREDENTIALS=/Users/alice/.config/gcloud/legacy_credentials/x7k2p9q1/adc.json",
            r#""credentials_file": "/Users/alice/.aws/sso/cache/3f9a2c7b1e.json""#,
            "私钥 ~/.ssh/id_ed25519_2024",
            "token_dir=/private/tmp/claude-501/x7k2p9q1-skill",
            "--token-file=/tmp/tok-a8f3k2x9/token",
            "skills/secret-scanner:x7k2p9q1",
            "- design-tokens:generate-v2: Generate tokens",
            "/secret-scan:audit-x7k2p9q1",
        ],
        &secrets(),
    );
}

// A moment goes as written, under a field whose name has token in it too.
#[test]
fn timestamp_is_no_secret() {
    let mut t = Errors::default();
    none_masked(
        &mut t,
        &[
            r#""access_token_expires_at": "2026-08-30T11:44:31.312645+08:00","#,
            r#""access_token": "2026-08-30T11:44:31Z""#,
            r#""token_expiry": "2026-08-30T11:44:31Z""#,
            r#""secret_updated_at": "2026-08-30T11:44:31.312645+08:00""#,
            r#""credential_expires": "2026-08-30T11:44:31Z""#,
            "token_expiry: 2026-08-30T11:44:31",
            "SERVER_KEY: 2026-08-30T11:44:31Z",
            "DB_PASS: 2026-08-30T11:44:31Z",
            "令牌是 2026-08-30T11:44:31",
            "密钥为 2026-08-30T11:44:31+08:00",
            "token 是 2026-08-30T11:44:31",
            "apikey 为 2026-08-30t11:44:31z",
        ],
        &secrets(),
    );
    // In a tool call's arguments a field named for a secret takes its value
    // whatever it looks like, but not a moment; the token beside it masks.
    let body = concat!(
        r#"{"messages":[{"role":"assistant","content":[{"type":"tool_use","id":"tu_1","name":"get_status","input":"#,
        r#"{"access_token_expires_at":"2026-08-30T11:44:31.312645+08:00","access_token":"a1b2c3d4e5f6g7h8"}}]}]}"#,
    );
    let (out, n) = mask_json(&masker(), body, &secrets());
    if n != 1 || !out.contains("2026-08-30T11:44:31") || unmarked(&out).contains("a1b2c3d4e5f6g7h8")
    {
        errorf!(
            t,
            "the tool call's expiry masked with its token: {n}: {out}"
        );
    }
}

// A value that only starts as a timestamp still masks.
#[test]
fn timestamp_head_and_more_is_a_secret() {
    let mut t = Errors::default();
    one_masked(
        &mut t,
        &[
            (
                r#""token": "2026-08-30T11x9k2abc:def""#,
                "2026-08-30T11x9k2abc",
            ),
            ("token=a1b2c3d4-2026-08-30T11:44", "a1b2c3d4-2026-08-30T11"),
            ("token=2026-08-30T11-abc:def", "2026-08-30T11-abc"),
        ],
        &secrets(),
    );
}

// A secret of the same shape as a line range or a name still masks.
#[test]
fn secrets_beside_lines_and_names() {
    let mut t = Errors::default();
    one_masked(
        &mut t,
        &[
            ("password=hunter2pass9", "hunter2pass9"),
            ("API_KEY=AbC123dEf456GhI789", "AbC123dEf456GhI789"),
            ("token=t0k3nValue2", "t0k3nValue2"),
            ("password: Passw0rd123", "Passw0rd123"),
            ("DB_PASSWORD=password123", "password123"),
            ("PASSWORD=SuperSecret2024", "SuperSecret2024"),
            ("token: Lk8x9QmP2vR4", "Lk8x9QmP2vR4"),
            ("secret.txt:hunter2pass9", "hunter2pass9"),
            ("x-api-key:k9x2mp4qr7st1vw3", "k9x2mp4qr7st1vw3"),
            ("x-api-key:ab12cd34ef56", "ab12cd34ef56"),
            ("Server=db;Password=Abc12345;User Id=sa", "Abc12345"),
            ("api_token = 'tok-8f3a9c2e7b1d'", "tok-8f3a9c2e7b1d"),
            ("密钥为12231241", "12231241"),
            ("私钥 ab12cd34ef56", "ab12cd34ef56"),
            (
                "SECRET_KEY=django-insecure-a1b2c3d4e5f6",
                "django-insecure-a1b2c3d4e5f6",
            ),
        ],
        &secrets(),
    );
}

// nextfield_test.go

// A field set to nothing takes nothing from the next one: an address, a
// port, a path or a flag after an empty field goes as written, on any
// category.
#[test]
fn empty_field_takes_not_the_next() {
    let mut t = Errors::default();
    none_masked(
        &mut t,
        &[
            "env -i PATH HOME UI_USER= UI_PASSWORD= UI_ADDR=127.0.0.1:39117 DATA_DIR=/tmp/x ./syncer serve",
            "UI_PASSWORD= UI_ADDR=0.0.0.0:39117 ./syncer serve",
            "UI_TOKEN= LISTEN=[::1]:39117 ./syncer",
            "API_KEY= ADDR=localhost:39117 ./syncer",
            "DB_PASS= DB_HOST=127.0.0.1 DB_PORT=5432 psql",
            "SERVICE_KEY=\tBIND_ADDR=127.0.0.1:8080",
            "level=info password= addr=127.0.0.1:39117 data_dir=/tmp/x1",
            "client_secret: listen_addr=127.0.0.1:8080",
            "./syncer --password= --listen=127.0.0.1:39117",
            "secret= --data-dir=/var/lib/syncer2",
            "key= UPSTREAM_URL2=http://127.0.0.1:39117/v1",
            "密码= UI_ADDR=127.0.0.1:39117",
            "密钥： UI_ADDR=127.0.0.1:39117 ./syncer",
            "token 是 UI_ADDR=127.0.0.1:39117",
        ],
        &everything(),
    );
}

// A secret still masks beside an empty field: its own, after its own name,
// and a value after a space that is no assignment.
#[test]
fn secret_beside_empty_field_masks() {
    let mut t = Errors::default();
    for (s, secret, kept) in [
        (
            "env -i UI_USER= UI_PASSWORD=hunter2pass9 UI_ADDR=127.0.0.1:39117 ./syncer serve",
            "hunter2pass9",
            "UI_ADDR=127.0.0.1:39117",
        ),
        (
            "UI_PASSWORD= UI_TOKEN=t0k3nValue2 UI_ADDR=127.0.0.1:39117",
            "t0k3nValue2",
            "UI_ADDR=127.0.0.1:39117",
        ),
        (
            "UI_PASSWORD= SERVICE_KEY=AbC123dEf456GhI789 ./syncer",
            "AbC123dEf456GhI789",
            "SERVICE_KEY=",
        ),
        (
            "password= api_key=AbC123dEf456GhI789",
            "AbC123dEf456GhI789",
            "api_key=",
        ),
        ("password = hunter2pass9", "hunter2pass9", "password = "),
        (
            "password: dGVzdDEyMzQ1Ng==",
            "dGVzdDEyMzQ1Ng==",
            "password: ",
        ),
        ("UI_PASSWORD= hunter2pass9", "hunter2pass9", "UI_PASSWORD= "),
        ("密码= UI_TOKEN=t0k3nValue2", "t0k3nValue2", "UI_TOKEN="),
    ] {
        let (out, n) = mask(&masker(), s, &everything());
        if n == 0 || out.contains(secret) || !out.contains(kept) {
            errorf!(
                t,
                "{s:?}: {n} masked: {out:?}, want {secret:?} masked and {kept:?} kept"
            );
        }
    }
}

// slug_test.go

// An address of an article about SK hynix is no sk- key. A key beside one
// still is, and so are keys in lower case.
#[test]
fn slug_is_no_key() {
    let mut t = Errors::default();
    none_masked(
        &mut t,
        &[
            "https://www.cnbc.com/2026/10/28/sk-hynix-third-quarter-profit.html",
            "https://finance.yahoo.com/markets/stocks/articles/sk-hynix-announces-3q26-financial-234500011.html",
            "https://www.techpowerup.com/354203/sk-hynix-announces-3q26-financial-results",
            "https://au.investing.com/news/stock-market-news/sk-hynix-q3-2026-slides-revenue-tops-100t-80-margin-on-ai-boom",
        ],
        &secrets(),
    );
    for key in [
        "sk-proj-abcdEFGH1234ijklMNOP5678qrst".to_owned(),
        format!("sk-{}", "0123456789abcdef".repeat(2)),
        format!("sk-or-v1-{}", "0123456789abcdef".repeat(4)),
        "sk-ant-api03-Ab3dE-fGh1_jKl2mN-oPq3rSt4".to_owned(),
        // a UUID in lower case: no capital, no run of 16
        format!("sk-{}", "1f0e3dad-9990-4b5c-8a1b-2c3d4e5f6a7b"),
    ] {
        let s = format!("url sk-hynix-third-quarter-profit key {key}");
        let (out, n) = mask(&masker(), &s, &secrets());
        if n != 1 || unmarked(&out).contains(&key) || !out.contains("sk-hynix-third-quarter-profit")
        {
            errorf!(t, "{key:?}: {n} masked: {out:?}");
        }
    }
}

// hexliteral_test.go

// A number written in hex, 0x and up to 16 hex digits, goes as written: no
// rule takes one. The cases where the local model labels one are left out.
#[test]
fn hex_number_goes_as_written() {
    let mut t = Errors::default();
    none_masked(
        &mut t,
        &[
            "| `sub_4EA5E0` | `0x004EA5E0` | 解密函数入口 |",
            "DecryptKey: 0x004EA5E0",
            "the token is 0X7FFE0300",
            "mask = 0xFFFFFFFFFFFFFFFF;",
            "base 0x00007ff6a1b20000 in x64dbg",
        ],
        &everything(),
    );
}

// digest_test.go

// A hash's hex digits short of 32 are an id: under a bare "key", a cache's
// key, a field named for a hash or as a JSON key of data keyed by hashes
// they go as written. Under a credential's name, or after a key's name in
// a sentence, the same digits are a key still. Go ran these through
// Preview, which remembers nothing; a masker of each case's own does the
// same. The model's cases are left out.
#[test]
fn short_digests_go_as_written() {
    let o = secrets_and_personal();
    let mut t = Errors::default();
    none_masked(
        &mut t,
        &[
            r#"{"key": "5d41402abc4b2a76"}"#,
            "key: 1a2b3c4d5e6f7a8b9c0d",
            r#"{"key":"0123456789abcdef0123"}"#,
            r#"{"token_hash": "0123456789abcdef01234567"}"#,
            r#"{"cache_key": "a1b2c3d4e5f60718293a"}"#,
            r#"cacheKey = "a1b2c3d4e5f60718293b""#,
            r#"{"object_key": "A1B2C3D4E5F60718293C"}"#,
            r#"{"hash": "9f13800138000abc"}"#,
            r#"{"keys": {"5d41402abc4b2a76": {"hits": 3}, "7c4a8d09ca3762af": {"hits": 1}}}"#,
            "git checkout 5d41402abc4b2a76e",
        ],
        &o,
    );
    for (s, v) in [
        ("password=deadbeef12345678", "deadbeef12345678"),
        ("api_key: 9f2c4e6a8b0d1f3e5a7c", "9f2c4e6a8b0d1f3e5a7c"),
        (
            r#"{"access_token": "0123456789abcdef0123"}"#,
            "0123456789abcdef0123",
        ),
        ("DB_PASSWORD=0123456789ABCDEF01", "0123456789ABCDEF01"),
        ("密钥为 5d41402abc4b2a76", "5d41402abc4b2a76"),
        ("apikey 为 5d41402abc4b2a77", "5d41402abc4b2a77"),
        // not a digest: mixed case, or letters past f
        (
            r#"{"key": "AbC123dEf456GhI789xyz"}"#,
            "AbC123dEf456GhI789xyz",
        ),
    ] {
        let (out, _) = mask(&masker(), s, &o);
        if out.contains(v) {
            errorf!(t, "{s:?}: key not masked: {out:?}");
        }
    }
}

// code_test.go

// Code that names a secret is no secret: a TypeScript type, a field read, a
// call. What a secret looks like written down stays masked: a value
// quoted, one after =, one a line ends.
#[test]
fn code_is_no_secret() {
    let mut t = Errors::default();
    none_masked(
        &mut t,
        &[
            "...: string;   readonly signingKey: { readonly secretKey: Uint8Array; readonly publicKey: Uint8Array };   /** The host's cu...",
            "...     host_key: details.1,\n      private_key: details.2,              });         }     /// `#send` ...",
            "...     url,     hostKey: pair.hostKey,     privateKey: toBase64Url(device.privateKey),     account: { serverOrigin: clien...",
            r#"...ataDir, "relay-key");   try {     const secretKey = fromBase64Url((await readFile(path, "utf8")).trim());     if (secr..."#,
            "type Keys = { apiKey: Base64Str | null }",
            "f({ token: opts.token2 })",
            "const password = sha256Hex (input)",
        ],
        &secrets(),
    );
    one_masked(
        &mut t,
        &[
            (r#"secretKey: "Uint8Array9x","#, "Uint8Array9x"),
            ("TOKEN=abc123def456", "abc123def456"),
            ("api_key=abc123def456, user=bob", "abc123def456"),
            ("password: Passw0rd123\nuser: bob", "Passw0rd123"),
            ("password: Passw0rd123", "Passw0rd123"),
            (r#"{"secretKey": "abcd1234efgh"}"#, "abcd1234efgh"),
            (r#"{\"private_key\": \"abcd1234efgh\"}"#, "abcd1234efgh"),
        ],
        &secrets(),
    );
}

// rules_test.go: the rules built in

// A gateway's key, made up.
const OC_KEY: &str = "oc_sk_AAAABBBBccccDDDD1234eeeeFFFF5678";

// A gateway's key is an API key wherever it is, and key alone takes a value
// that is no API key as a secret; key as code says it stays.
#[test]
fn mask_gateway_key() {
    let cases: Vec<(String, Option<&str>)> = vec![
        (format!(r#""key": "{OC_KEY}""#), Some("API_KEY")),
        (format!(r#"\"key\": \"{OC_KEY}\""#), Some("API_KEY")),
        (format!(r#""apiKey": "{OC_KEY}""#), Some("API_KEY")),
        (format!(r#""token": "{OC_KEY}""#), Some("API_KEY")),
        (format!("bare {OC_KEY} value"), Some("API_KEY")),
        ("or_sk_0123456789abcdefghijKLMNOP".into(), Some("API_KEY")),
        // key alone, a value that is no API key
        (
            r#"{"id": "relay", "key": "Zq81nVx0pL2mWe7RtY4u"}"#.into(),
            Some("SECRET"),
        ),
        (
            r#"{\"key\": \"Zq81nVx0pL2mWe7RtY4u\"}"#.into(),
            Some("SECRET"),
        ),
        ("key: Zq81nVx0pL2mWe7RtY4u".into(), Some("SECRET")),
        (
            r#""api-key": "Zq81nVx0pL2mWe7RtY4u""#.into(),
            Some("SECRET"),
        ),
        (r#""apikey": "Zq81nVx0pL2mWe7RtY4u""#.into(), Some("SECRET")),
        (
            r#"\"apiKey\": \"Zq81nVx0pL2mWe7RtY4u\""#.into(),
            Some("SECRET"),
        ),
        // and key as code says it
        (r#"{"key": "value", "sort_key": "xyz"}"#.into(), None),
        (r#"key = "someIdentifier""#.into(), None),
        (r#"key = "someVeryLongIdentifierName""#.into(), None),
        ("sort_key: created_at_2024_rows".into(), None),
        ("monkey: banana1234567890abc".into(), None),
        (r#"obj.key = "abc123""#.into(), None),
        (r#""key": "row-12""#.into(), None),
        // no digits
        ("the task_sk_something_long_name_here module".into(), None),
        // five letters is a word
        ("abcde_sk_0123456789abcdefghijKL".into(), None),
        (r#""key": "${OPENCODE_API_KEY_0123456789}""#.into(), None),
    ];
    let mut t = Errors::default();
    for (s, kind) in &cases {
        let m = masker();
        let (out, n) = mask(&m, s, &secrets());
        let Some(kind) = kind else {
            if n != 0 || out != *s {
                errorf!(t, "{s:?} masked: {out:?}");
            }
            continue;
        };
        if n != 1 || !out.contains(&format!("{{{{{kind}_")) {
            errorf!(t, "{s:?}: {n} masked, want one {kind}: {out:?}");
        }
        let back = restore(&m, &out);
        if back != *s {
            errorf!(t, "round trip: {s:?} -> {out:?} -> {back:?}");
        }
    }
    // the same key, the same placeholder, whatever field it is in
    let m = masker();
    let (a, _) = mask(&m, &format!(r#""key": "{OC_KEY}""#), &secrets());
    let (b, _) = mask(&m, &format!("OC_API_KEY={OC_KEY}"), &secrets());
    let (c, _) = mask(&m, OC_KEY, &secrets());
    let (pa, pb, pc) = (
        first_placeholder(&a),
        first_placeholder(&b),
        first_placeholder(&c),
    );
    if pa.is_empty() || pa != pb || pa != pc {
        errorf!(t, "placeholders differ: {a:?} {b:?} {c:?}");
    }
}

// A key with an ark- prefix is a key on its own, under any name and in a
// request body; a word that only has ark- in it is none.
#[test]
fn mask_ark_key() {
    let in_env_name = "ark-c8b7e5a9-4f3d-4e2a-9b1c-7d6e5f4a3b2c";
    let under_ark = "ark-6f2a9d47-1c3b-8e5f-a7d9-0b4c2e6f8a1d";
    let in_prose = "ark-93be1f6c-7a2d-4b8e-9c0f-5d3a7e1b9c2a";
    let in_body = "ark-2e7c4a19-8d3f-6b5a-1e9c-7f4b2d8a3c6e";
    let no_key = "the dark-4a1c8f2e-9b3d-7e6a-0c5f-1d8b4a2e9f7b code";
    let mut t = Errors::default();
    for (s, json, gone) in [
        (format!("ARK_API_KEY={in_env_name}"), false, true),
        (format!("ARK_KEY={under_ark}"), false, true),
        (format!("the ark key is {in_prose}"), false, true),
        (
            format!(r#"{{"notes":"relay key {in_body} for the plan"}}"#),
            true,
            true,
        ),
        (no_key.to_owned(), false, false),
        ("ark-key".to_owned(), false, false),
    ] {
        let m = masker();
        let (out, n) = if json {
            mask_json(&m, &s, &secrets())
        } else {
            mask(&m, &s, &secrets())
        };
        if !gone {
            if n != 0 || out != s {
                errorf!(t, "{s:?} masked: {out:?}");
            }
            continue;
        }
        if n != 1 || out.contains("ark-") {
            errorf!(t, "{s:?}: {n} masked: {out:?}");
            continue;
        }
        let back = if json {
            restore_json(&m, &out)
        } else {
            restore(&m, &out)
        };
        if back != s {
            errorf!(t, "round trip: {s:?} -> {out:?} -> {back:?}");
        }
    }
}

// An env var's name says what it holds in the upper case: a SERVICE_X_KEY,
// a DB_PASS. In the lower case a name that only ends in key holds no
// secret.
#[test]
fn mask_upper_env_names() {
    let mut t = Errors::default();
    for (s, gone) in [
        ("SERVICE_X_KEY=Vt57wQx2mLp9kNr4Zc8b", true),
        ("FOO_KEY=Jw83hT6yQv2nRs9Xd5ke", true),
        (r#"DB_PASS="Mr4tQw9xZb7n""#, true),
        ("DB_PASS=Kp8vHs3wTn6j", true),
        ("sort_key: created_at_2024_rows", false),
        ("cache_key: lru_items_01", false),
        ("public_key = 04a1b2c3d4e5f6a7b8", false),
        ("db_pass=zz52Qx9vwR3n", false),
        ("SORT_KEY=created_at_2024", false),
        ("PUBLIC_KEY=04a1b2c3d4e5f6a7b8", false),
        ("PROMPT_CACHE_KEY=conv_12345678", false),
        ("SERVICE_X_KEY: Uint8Array;", false),
        ("DEBUG_PASS: options.pass8,", false),
    ] {
        let m = masker();
        let (out, n) = mask(&m, s, &secrets());
        if !gone {
            if n != 0 || out != s {
                errorf!(t, "{s:?} masked: {out:?}");
            }
            continue;
        }
        if n != 1
            || ["Vt57wQx2", "Jw83hT6y", "Mr4tQw9x", "Kp8vHs3w"]
                .iter()
                .any(|v| out.contains(v))
        {
            errorf!(t, "{s:?}: {n} masked: {out:?}");
            continue;
        }
        let back = restore(&m, &out);
        if back != s {
            errorf!(t, "round trip: {s:?} -> {out:?} -> {back:?}");
        }
    }
}

// An Authorization or Cookie header's value is a secret whatever it looks
// like. Prose about them, and a header of another name, are not.
#[test]
fn mask_auth_headers() {
    let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.c2lnbmF0dXJlc2lnbmF0dXJl";
    let bearer_jwt = format!("Authorization: Bearer {jwt}");
    let values = [
        "mfa_9x2kQ7wLr4Tz",
        "8f3kz9a1x2c4",
        "Zm9vYmFyYmF6",
        "deadbeef42",
        "tt5Za8wQ3nR7kLmx",
        "pq93Xm2vB8n4Rk",
    ];
    let mut t = Errors::default();
    for (s, want) in [
        ("Authorization: Bearer mfa_9x2kQ7wLr4Tz", 1),
        ("authorization: Basic Zm9vYmFyYmF6", 1),
        ("Cookie: sid=8f3kz9a1x2c4", 1),
        ("cookie: session=deadbeef42", 1),
        (
            "POST /v1 HTTP/1.1\nAuthorization: Bearer tt5Za8wQ3nR7kLmx\nCookie: sid=pq93Xm2vB8n4Rk\n",
            2,
        ),
        (bearer_jwt.as_str(), 1),
        ("the authorization is required", 0),
        ("cookies: 3", 0),
        ("Authorization: the user must sign in first", 0),
        ("Authorization: required", 0),
        ("Cookie: see the docs for how they are kept", 0),
        ("Authorization: Bearer ${API_TOKEN}", 0),
        ("Cookie: sid=Rk82mQ4xVb; theme=dark", 1),
    ] {
        let m = masker();
        let (out, n) = mask(&m, s, &secrets());
        if n != want {
            errorf!(t, "{s:?}: {n} masked, want {want}: {out:?}");
            continue;
        }
        if values.iter().any(|v| out.contains(v)) {
            errorf!(t, "{s:?}: a value went out: {out:?}");
            continue;
        }
        let back = restore(&m, &out);
        if back != s {
            errorf!(t, "round trip: {s:?} -> {out:?} -> {back:?}");
        }
    }
}

// ids_test.go

const IDS: &[&str] = &[
    "3f2b8c1e-9a4d-4e7b-b2c1-7d6e5f4a3b21",
    "3F2B8C1E-9A4D-4E7B-B2C1-7D6E5F4A3B21",
    // a phone number in its last group
    "3f2b8c1e-9a4d-4e7b-b2c1-a13800138000",
    // a bank card number across its groups
    "abcd4111-1111-1111-9007-abcdef123456",
    // no hyphens
    "3f2b8c1e9a4d4e7bb2c1a13800138000",
    // a long hash with both
    "9f8e7d6cabcd4111111111111111900713800138000a",
    "12345678-1234-1234-1234-123456789012",
];

// An id goes as written wherever it is and whatever is around it. It is
// masked whole only in a field named for a credential. The model's cases
// are left out.
#[test]
fn ids_go_as_written() {
    let o = everything();
    let as_written = [
        "%s",
        "id %s",
        r#"{"id":"%s"}"#,
        "request_id: %s",
        "/users/%s/profile",
        "urn:uuid:%s",
        "{%s}",
        "next_page_token: %s",
        r#""nextPageToken": "%s""#,
        r#"\"csrf_token\": \"%s\""#,
        r#""token_id": "%s""#,
        "secret_id: %s",
        r#""api_key_id": "%s""#,
        "access_key_id = %s",
        r#""key": "%s""#,
        "Idempotency-Key: %s",
        "X-Request-Id: %s",
        "trace_token=%s",
        "secret_hash: %s",
        "sn: %s",
        "Serial Number: %s",
        "tel: %s",
    ];
    let mut t = Errors::default();
    for id in IDS {
        for c in as_written {
            let s = c.replace("%s", id);
            let (out, n) = mask(&masker(), &s, &o);
            if out != s || n != 0 {
                errorf!(t, "{s:?} went as {out:?}");
            }
        }
    }
    let credentials = [
        "password=%s",
        "api_key=%s",
        "X-Api-Key: %s",
        "token: %s",
        r#""refresh_token": "%s""#,
        "session_token=%s",
        "postgres://app:%s@db:5432/x",
    ];
    for id in &IDS[..2] {
        for c in credentials {
            let s = c.replace("%s", id);
            let (out, n) = mask(&masker(), &s, &o);
            if out.contains(id) || n != 1 {
                errorf!(t, "a credential went out: {s:?} as {out:?}");
            }
        }
    }

    // a phone number masked before is not masked inside an id after
    let m = masker();
    mask(&m, "call 13800138000", &o);
    let s = format!("order {}", IDS[2]);
    let (out, _) = mask(&m, &s, &o);
    if out != s {
        errorf!(
            t,
            "a known phone number was masked inside {:?}: {out:?}",
            IDS[2]
        );
    }

    // a phone number and a card number of their own are still masked
    for s in [
        "call 13800138000",
        "card 4111-1111-1111-1111",
        "call +8613800138000",
    ] {
        let (out, n) = mask(&masker(), s, &o);
        if n != 1 || out == s {
            errorf!(t, "{s:?} went as {out:?}");
        }
    }

    // what the user chose stays theirs
    let uo = Options {
        words: vec![IDS[0].into()],
        ..o.clone()
    };
    let (out, n) = mask(&masker(), &format!("order {}", IDS[0]), &uo);
    if out.contains(IDS[0]) || n != 1 {
        errorf!(t, "the user's word went out: {out:?}");
    }
}

// cua-driver's element tokens go as written, in the window states it hands
// the model and in the clicks the model asks for. In a field named for a
// credential, or not shaped as one, a value is masked as before; the
// user's own words still mask one. The model's cases are left out.
#[test]
fn element_tokens_go_as_written() {
    let o = everything();
    let tokens = [
        "s00000001",
        "s00000003:74",
        "s0000002a:14",
        "s00007a3f:42",
        "s00010001:3",
    ];
    let as_written = [
        "\"element_index\": 0,\n\"element_token\": \"%s\",\n\"enabled\": true,",
        r#""depth":2,"element_index":74,"element_token":"%s","enabled":true"#,
        r#""element_index":74,"element_token": "%s","enabled":true"#,
        r#"cua-driver click '{"pid":844,"window_id":10725,"element_token": "%s","session":"run-1"}'"#,
        "element_token=%s",
        r#"\"element_token\":\"%s\""#,
        r#"{"elementToken": "%s"}"#,
    ];
    let mut t = Errors::default();
    for tok in tokens {
        for c in as_written {
            let s = c.replace("%s", tok);
            let (out, n) = mask(&masker(), &s, &o);
            if out != s || n != 0 {
                errorf!(t, "{s:?} went as {out:?}");
            }
        }
    }

    // a window's state and a click in one request, a password beside them
    let body = concat!(
        r#"{"messages":[{"role":"tool","tool_call_id":"c1","content":"{\"elements\":[{\"element_index\":14,\"element_token\":\"s0000002a:14\"}],\"env\":\"DB_PASSWORD=hunter2abc9\"}"},"#,
        r#"{"role":"assistant","tool_calls":[{"id":"c2","type":"function","function":{"name":"click","arguments":"{\"pid\":844,\"element_token\":\"s0000002a:14\"}"}}]}]}"#,
    );
    let (got, n) = mask_json(&masker(), body, &o);
    if got.matches("s0000002a:14").count() != 2 || got.contains("hunter2abc9") || n != 1 {
        errorf!(t, "{n} masked: {got}");
    }

    // a field named for a credential: masked
    for v in ["s00000001", "s00000003:74"] {
        for c in [
            "password=%s",
            "token: %s",
            r#""refresh_token": "%s""#,
            r#""api_key": "%s""#,
        ] {
            let s = c.replace("%s", v);
            let (out, n) = mask(&masker(), &s, &o);
            if out.contains(&v[..9]) || n != 1 {
                errorf!(t, "a credential went out: {s:?} as {out:?}");
            }
        }
    }
    // a value of another shape in an element's field: masked
    for v in [
        "abc123def456",
        "s0000000g1",
        "S0000002A",
        "s000000012",
        "s00000001x",
        "s00000001:x9",
    ] {
        let s = format!(r#""element_token": "{v}""#);
        let (out, n) = mask(&masker(), &s, &o);
        if out == s || n != 1 {
            errorf!(t, "{s:?} went as {out:?}");
        }
    }
    // a field with "element" in its name that is no element_token is a
    // credential's: a UUID or the token's shape in it is masked
    for c in [
        r#""element_api_key": "%s""#,
        r#""elementSecret": "%s""#,
        r#""element_secret_token": "%s""#,
    ] {
        for v in ["3f2504e0-4f89-11d3-9a0c-0305e82c3301", "s00000001"] {
            let s = c.replace("%s", v);
            let (out, n) = mask(&masker(), &s, &o);
            if out.contains(v) || n != 1 {
                errorf!(t, "a credential went out: {s:?} as {out:?}");
            }
        }
    }

    // the user's own words
    let uo = Options {
        words: vec!["s0000002a".into()],
        ..o.clone()
    };
    let (out, n) = mask(&masker(), r#""element_token": "s0000002a:14""#, &uo);
    if out.contains("s0000002a") || n != 1 {
        errorf!(t, "the user's word went out: {out:?}");
    }
}

// crypto_test.go
//
// Every wallet key here is made from a seed of the test's own, checksum and
// all, rather than written out: Go's file wrote out some published test
// vectors, and a key in the source is a key in every clone.

const B58: &str = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
const BASE32: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
const BECH32: &str = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";
const LOWER36: &str = "0123456789abcdefghijklmnopqrstuvwxyz";
const BIP39_LIST: &str = include_str!("../src/bip39english.txt");

/// `s` repeated to `n` characters.
fn rep(s: &str, n: usize) -> String {
    s.repeat(n / s.len() + 1)[..n].to_owned()
}

/// A run of `n` characters of alphabet `a`, from its `i`th.
fn pad(a: &str, n: usize, i: usize) -> String {
    let a = a.as_bytes();
    (0..n).map(|k| a[(i + k) % a.len()] as char).collect()
}

/// 32 bytes made from `name`.
fn test_seed(name: &str) -> [u8; 32] {
    Sha256::digest(format!("pansy crypto test {name}")).into()
}

/// `n` letters and digits that read as random, made from `name`.
fn random_alnum(name: &str, n: usize) -> String {
    let a = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    let mut out = String::new();
    let mut h = Sha512::digest(name.as_bytes());
    while out.len() < n {
        out.extend(
            h.iter()
                .map(|b| a[*b as usize % a.len()] as char)
                .take(n - out.len()),
        );
        h = Sha512::digest(h);
    }
    out
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn base64_with(b: &[u8], alphabet: &[u8], padded: bool) -> String {
    let mut out = String::new();
    for chunk in b.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &x)| n | (x as u32) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(alphabet[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
        if padded {
            out.push_str(&"=".repeat(3 - chunk.len()));
        }
    }
    out
}

fn base64_std(b: &[u8]) -> String {
    base64_with(
        b,
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/",
        true,
    )
}

fn base64_url_raw(b: &[u8]) -> String {
    base64_with(
        b,
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_",
        false,
    )
}

/// `b` in RFC 4648 base32, without padding.
fn base32_raw(b: &[u8]) -> String {
    let (mut out, mut acc, mut bits) = (String::new(), 0u32, 0);
    for &x in b {
        acc = acc << 8 | x as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(BASE32.as_bytes()[(acc >> bits & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(BASE32.as_bytes()[(acc << (5 - bits) & 31) as usize] as char);
    }
    out
}

/// `b` in base58, a 1 for each zero byte it starts with.
fn enc58(b: &[u8]) -> String {
    let mut num = b.to_vec();
    let mut start = num.iter().position(|&x| x != 0).unwrap_or(num.len());
    let mut out = Vec::new();
    while start < num.len() {
        let mut rem = 0u32;
        for x in &mut num[start..] {
            let acc = rem * 256 + *x as u32;
            *x = (acc / 58) as u8;
            rem = acc % 58;
        }
        out.push(B58.as_bytes()[rem as usize]);
        while start < num.len() && num[start] == 0 {
            start += 1;
        }
    }
    out.extend(b.iter().take_while(|&&x| x == 0).map(|_| b'1'));
    out.reverse();
    String::from_utf8(out).unwrap()
}

/// `p` with its double-sha256 checksum, in base58.
fn enc58check(p: &[u8]) -> String {
    let h = Sha256::digest(Sha256::digest(p));
    enc58(&[p, &h[..4]].concat())
}

/// The 64-byte ed25519 secret key of `name`'s seed, as Solana keeps it: the
/// seed, then its public key.
fn solana_secret(name: &str) -> Vec<u8> {
    let seed = test_seed(name);
    [&seed[..], &ed25519::public_key(&seed)].concat()
}

fn solana_public(name: &str) -> [u8; 32] {
    ed25519::public_key(&test_seed(name))
}

fn byte_list(b: &[u8]) -> String {
    format!(
        "[{}]",
        b.iter()
            .map(|x| x.to_string())
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn crc16_xmodem(b: &[u8]) -> u16 {
    let mut c: u16 = 0;
    for &x in b {
        c ^= (x as u16) << 8;
        for _ in 0..8 {
            c = if c & 0x8000 != 0 {
                c << 1 ^ 0x1021
            } else {
                c << 1
            };
        }
    }
    c
}

/// A Stellar StrKey: the version byte, the body, a CRC16 little-endian.
fn stellar(version: u8, body: &[u8]) -> String {
    let mut p = [&[version][..], body].concat();
    let c = crc16_xmodem(&p);
    p.extend([c as u8, (c >> 8) as u8]);
    base32_raw(&p)
}

/// `data` in bech32 under `hrp`, checksum and all (BIP-0173).
fn bech32(hrp: &str, data: &[u8]) -> String {
    fn polymod(values: &[u8]) -> u32 {
        const GEN: [u32; 5] = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3];
        let mut chk = 1u32;
        for &v in values {
            let top = chk >> 25;
            chk = (chk & 0x1ffffff) << 5 ^ v as u32;
            for (i, g) in GEN.iter().enumerate() {
                if top >> i & 1 == 1 {
                    chk ^= g;
                }
            }
        }
        chk
    }
    let (mut five, mut acc, mut bits) = (Vec::new(), 0u32, 0);
    for &x in data {
        acc = acc << 8 | x as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            five.push((acc >> bits & 31) as u8);
        }
    }
    if bits > 0 {
        five.push((acc << (5 - bits) & 31) as u8);
    }
    let mut values: Vec<u8> = hrp.bytes().map(|c| c >> 5).collect();
    values.push(0);
    values.extend(hrp.bytes().map(|c| c & 31));
    values.extend(&five);
    values.extend([0; 6]);
    let m = polymod(&values) ^ 1;
    five.extend((0..6).map(|i| (m >> (5 * (5 - i)) & 31) as u8));
    format!(
        "{hrp}1{}",
        five.iter()
            .map(|&d| BECH32.as_bytes()[d as usize] as char)
            .collect::<String>()
    )
}

/// A BIP-0032 extended key at the root: its version, a chain code and a
/// key of 33 bytes, with its checksum.
fn bip32(version: [u8; 4], key: &[u8]) -> String {
    let mut p = version.to_vec();
    p.extend([0; 9]); // depth, parent's fingerprint, child number
    p.extend(test_seed("chain code"));
    p.extend(key);
    enc58check(&p)
}

fn xprv() -> String {
    bip32(
        [0x04, 0x88, 0xad, 0xe4],
        &[&[0][..], &test_seed("bip32")].concat(),
    )
}

fn xpub() -> String {
    bip32(
        [0x04, 0x88, 0xb2, 0x1e],
        &[&[2][..], &test_seed("bip32 public")].concat(),
    )
}

/// An EVM wallet's private key in hex, without its 0x.
fn wallet_hex() -> String {
    hex(&test_seed("wallet"))
}

/// `n` words of BIP-0039's list, picked by `step`, as a phrase.
fn list_phrase(n: usize, step: usize) -> String {
    let words: Vec<&str> = BIP39_LIST.split_whitespace().collect();
    (0..n)
        .map(|i| words[(i * step + 11) % words.len()])
        .collect::<Vec<_>>()
        .join(" ")
}

fn crypto_values() -> Vec<(&'static str, String)> {
    vec![
        ("WALLET_KEY", format!("0x{}", wallet_hex())),
        (
            "WIF_KEY",
            enc58check(&[&[0x80][..], &test_seed("wif uncompressed")].concat()),
        ),
        ("BIP32_KEY", xprv()),
        ("SOLANA_KEY", enc58(&solana_secret("sol"))),
        ("SOLANA_KEYPAIR", byte_list(&solana_secret("keypair"))),
        (
            "NEAR_KEY",
            format!("ed25519:{}", enc58(&solana_secret("near"))),
        ),
        ("SUI_KEY", format!("suiprivkey1{}", pad(LOWER36, 58, 4))),
        (
            "APTOS_KEY",
            format!("ed25519-priv-0x{}", rep("9f2c4b6d8e0a", 64)),
        ),
        ("STELLAR_KEY", stellar(18 << 3, &test_seed("stellar"))),
        ("NOSTR_KEY", bech32("nsec", &test_seed("nostr"))),
        (
            "AGE_KEY",
            format!(
                "AGE-SECRET-KEY-1{}",
                pad("QPZRY9X8GF2TVDW0S3JN54KHCE6MUA7L", 58, 6)
            ),
        ),
        (
            "ZCASH_KEY",
            format!("secret-extended-key-main1{}", pad(LOWER36, 160, 9)),
        ),
        ("SEED_PHRASE", list_phrase(12, 389)),
    ]
}

// The keys these tests make have the shapes and the checks of real ones.
#[test]
fn crypto_test_values_are_well_formed() {
    let v: std::collections::HashMap<_, _> = crypto_values().into_iter().collect();
    assert_eq!(v["WIF_KEY"].len(), 51);
    assert!(v["WIF_KEY"].starts_with('5'));
    assert_eq!(xprv().len(), 111);
    assert!(xprv().starts_with("xprv") && xpub().starts_with("xpub"));
    assert_eq!(v["NOSTR_KEY"].len(), 63);
    assert_eq!(v["STELLAR_KEY"].len(), 56);
    assert!(v["STELLAR_KEY"].starts_with('S'));
    assert_eq!(bech32("a", &[]), "a12uel5l"); // BIP-0173's first valid string
    assert_eq!(base64_std(b"foob"), "Zm9vYg==");
    assert_eq!(base32_raw(b"foob"), "MZXW6YQ");
    assert_eq!(enc58(&[0, 0, 0x28, 0x7f, 0xb4, 0xcd]), "11233QC4");
    assert_eq!(crc16_xmodem(b"123456789"), 0x31c3);
}

// Every shape goes as its own kind with the secrets on, and comes back as
// it was. Personal data alone leaves them as written.
#[test]
fn mask_crypto_keys() {
    let mut t = Errors::default();
    for (kind, v) in crypto_values() {
        let s = match kind {
            "WALLET_KEY" => format!("PRIVATE_KEY {v} ok"),
            "SEED_PHRASE" => format!("seed phrase: {v} ok"),
            _ => format!("the key is {v} ok"),
        };
        let m = masker();
        let (out, n) = mask(&m, &s, &secrets());
        if n != 1 || !out.contains(&format!("{{{{{kind}_")) || out.contains(&v) {
            errorf!(t, "{kind}: {n} masked: {out}");
            continue;
        }
        let back = restore(&m, &out);
        if back != s {
            errorf!(t, "{kind}: restored {back:?}, want {s:?}");
        }
        let (off, n) = mask(
            &masker(),
            &s,
            &Options {
                personal: true,
                ..Options::default()
            },
        );
        if n != 0 || off != s {
            errorf!(t, "{kind}: masked with the secrets off ({n}): {off}");
        }
    }
}

// The shapes with no prefix are taken only when the key proves itself:
// compressed WIF, other chains' version bytes, an 87-character Solana key,
// a keyfile across lines. A tprv with an xprv's checksum is no key.
#[test]
fn mask_crypto_checked() {
    let compressed = enc58check(&[&[0x80][..], &test_seed("wif"), &[1]].concat());
    let litecoin = enc58check(&[&[0xb0][..], &test_seed("ltc")].concat());
    let testnet = enc58check(&[&[0xef][..], &test_seed("tbtc"), &[1]].concat());
    let sol87 = (0..)
        .map(|i| enc58(&solana_secret(&format!("short{i}"))))
        .find(|s| s.len() == 87)
        .unwrap();
    let keyfile = byte_list(&solana_secret("lines")).replace(',', ",\n  ");
    let mut t = Errors::default();
    for (kind, v) in [
        ("WIF_KEY", compressed),
        ("WIF_KEY", litecoin),
        ("WIF_KEY", testnet),
        ("SOLANA_KEY", sol87),
        ("SOLANA_KEYPAIR", keyfile),
        ("BIP32_KEY", format!("tprv{}", &xprv()[4..])),
    ] {
        let (out, n) = mask(&masker(), &format!("value {v} here"), &secrets());
        let want = !v.starts_with("tprv");
        let got = n == 1 && out.contains(&format!("{{{{{kind}_"));
        if got != want {
            errorf!(t, "{kind} {v:?}: masked {got}, want {want}: {out}");
        }
    }
}

// 0x and 64 hex digits is a key after a key's name, in a config, a flag,
// code and prose. Elsewhere it is a transaction hash, a block hash, a
// storage slot or a sha256 sum, and goes as written.
#[test]
fn mask_crypto_wallet_key_needs_its_name() {
    let w = wallet_hex();
    let k = format!("0x{w}");
    let mut t = Errors::default();
    for s in [
        format!("PRIVATE_KEY={k}"),
        format!("export DEPLOYER_PRIVATE_KEY={k}"),
        format!("DEPLOYER_PK={k}"),
        format!("forge script Deploy.s.sol --broadcast --private-key {k}"),
        format!("cast send --private-key {w} 0xabc"),
        format!(r#"const signer = new ethers.Wallet("{k}", provider)"#),
        format!(r#"account = Account.from_key(priv_key) # priv_key = "{k}""#),
        format!("privKey: '{k}',"),
        format!("私钥 {k}"),
        format!("我的私钥是{k}"),
        format!("signingKey={k}"),
        format!("STARKNET_PRIVATE_KEY=0x{}", &w[..60]),
    ] {
        let (out, _) = mask(&masker(), &s, &secrets());
        if out.contains(&w[..56]) {
            errorf!(t, "{s:?}: key left in {out:?}");
        }
    }
    let h = rep("4c0883a69102937d", 64);
    let slot = "360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc";
    let sum = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    let kept = [
        format!("see 0x{h} on chain"),
        format!(r#"{{"transactionHash":"0x{h}","blockHash":"0x{slot}"}}"#),
        format!("bytes32 internal constant IMPLEMENTATION_SLOT = 0x{slot};"),
        format!("wallet tx hash 0x{h}"),
        format!("the wallet's address 0x{h}"),
        format!("privateKeyToPublic(0x{h})"),
        format!("wallet_id: 0x{h}"),
        format!("{sum}  secrets.env"),
        format!("secrets.env sha256 {sum}"),
        format!("sha256:{sum}"),
        format!("pkg-lock {sum}"),
        // "key" alone names anything in code
        format!("key 0x{h}"),
        format!("0x{}", rep("4c0883a6", 40)),
    ];
    for s in &kept {
        let (out, n) = mask(&masker(), s, &secrets());
        if n != 0 {
            errorf!(t, "{s:?} masked: {out}");
        }
    }
}

// Values that share a piece of a key's shape and are not keys go as
// written, so a vendor can still read them.
#[test]
fn mask_crypto_lookalikes() {
    let sig = Sha512::digest(b"a transaction");
    let kept = [
        format!("0x{}", rep("4c0883a6", 40)), // an address, 20 bytes
        rep("a1b2c3d4", 40),                  // a git sha
        "commit 9fceb02d0ae598e95dc970b74767f19372d61af8".to_owned(), // a git sha
        rep("e3b0c44298fc1c14", 64),          // a bare sha256
        format!("sha256:{}", rep("e3b0c44298fc1c14", 64)), // a digest, named
        "550e8400-e29b-41d4-a716-446655440000".to_owned(), // a uuid
        xpub(),                               // BIP32's public key, checksum and all
        format!("ypub{}", pad(B58, 107, 3)),  // more public keys
        format!("xprv{}", pad(B58, 107, 3)),  // an xprv's shape with no checksum
        enc58(&solana_public("pub")),         // a Solana public key
        enc58(&sig),                          // a transaction signature, 64 bytes
        pad(B58, 88, 11),                     // 88 base58 characters
        pad(B58, 87, 13),                     // 87 of them
        format!("5{}", pad(B58, 50, 7)),      // WIF's shape, no checksum
        format!("K{}", pad(B58, 51, 7)),      // compressed WIF's shape
        "1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2".to_owned(), // a Bitcoin address
        "QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG".to_owned(), // an IPFS CID
        "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq".to_owned(), // a bech32 address
        format!("npub1{}", pad(BECH32, 58, 1)), // NIP-19's public key
        format!("ed25519-pub-0x{}", rep("9f2c4b6d8e0a", 64)), // AIP-80's public key
        format!("ed25519:{}", enc58(&solana_public("near"))), // NEAR's public key
        format!("suiprivkey{}", pad(LOWER36, 58, 4)), // no trailing 1
        format!(
            "age-secret-key-1{}",
            pad("QPZRY9X8GF2TVDW0S3JN54KHCE6MUA7L", 58, 6)
        ), // age is capitals
        format!("S{}", pad(BASE32, 55, 2)),   // a seed's shape, no CRC
        stellar(6 << 3, &test_seed("stellar")), // a Stellar account id, G…
        "SOMEVERYLONGCONSTANTNAMEWRITTENINCAPITALSWITHOUTANYBREAKS".to_owned(), // 56 capitals from S
        byte_list(&sig),     // a sha512 as 64 numbers
        byte_list(&[0; 64]), // 64 zeros
    ];
    let mut t = Errors::default();
    for v in &kept {
        let s = format!("value {v} here");
        let (out, n) = mask(&masker(), &s, &secrets());
        if n != 0 || out != s {
            errorf!(t, "masked {v:?} ({n}): {out}");
        }
    }
}

// A lockfile's integrity hashes and base64 blobs, a thousand of each, go as
// written.
#[test]
fn mask_crypto_lockfiles_and_blobs() {
    let (mut lock, mut blobs) = (String::new(), String::new());
    for i in 0..1000 {
        let h = Sha512::digest(format!("package {i}"));
        lock += &format!("    \"integrity\": \"sha512-{}\",\n", base64_std(&h));
        let s = Sha256::digest(format!("module {i}"));
        lock += &format!("example.com/m{i} v1.0.{i} h1:{}\n", base64_std(&s));
        lock += &format!(
            "  resolved \"https://registry.example/-/p-{i}.tgz#{}\"\n",
            hex(&s)
        );
        let mut blob: Vec<u8> = Vec::new();
        while blob.len() < 96 {
            let h = Sha512::digest([format!("blob {i}").as_bytes(), &blob].concat());
            blob.extend(h);
        }
        blobs += &format!("{}\n{}\n", base64_std(&blob), base64_url_raw(&blob));
    }
    let mut t = Errors::default();
    for (name, s) in [("lockfile", &lock), ("base64", &blobs)] {
        let (out, n) = mask(&masker(), s, &secrets());
        if n != 0 {
            let first = out.lines().find(|l| l.contains("{{")).unwrap_or("");
            errorf!(t, "{name}: masked {n}, first: {first}");
        }
    }
}

// A key that arrives as a value of its own in a body's strings, with no
// field name beside it, is masked.
#[test]
fn mask_crypto_json() {
    let m = masker();
    let sol = enc58(&solana_secret("sol"));
    let body = format!(r#"{{"tool":"wallet","input":{{"key":"{sol}"}}}}"#);
    let (out, n) = mask_json(&m, &body, &secrets());
    assert!(n == 1 && !out.contains(&sol), "{n} masked: {out}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(
        v["input"]["key"]
            .as_str()
            .unwrap()
            .starts_with("{{SOLANA_KEY_"),
        "{out}"
    );
    assert_eq!(restore_json(&m, &out), body);
}

// A key masked in one turn is masked again in the next, with the same
// placeholder.
#[test]
fn mask_crypto_known() {
    let m = masker();
    let sol = enc58(&solana_secret("sol"));
    let (first, n) = mask(&m, &format!("my key {sol}"), &secrets());
    assert_eq!(n, 1, "{first}");
    let ph = first_placeholder(&first);
    assert!(!ph.is_empty(), "no placeholder: {first}");
    let (second, n) = mask(&m, &format!("again {sol}"), &secrets());
    assert!(
        n == 1 && second.contains(ph),
        "second turn {n}: {second}, want {ph}"
    );
}

// A prefixed key is the whole run: one character more or less is another
// value, and stays as written.
#[test]
fn mask_crypto_lengths() {
    let mut t = Errors::default();
    for (kind, len, ok) in [
        ("NOSTR_KEY", 63, true),
        ("NOSTR_KEY", 62, false),
        ("NOSTR_KEY", 64, false),
        ("AGE_KEY", 74, true),
        ("AGE_KEY", 73, false),
        ("AGE_KEY", 75, false),
        ("SUI_KEY", 69, true),
        ("SUI_KEY", 68, false),
        ("SUI_KEY", 70, false),
        ("NEAR_KEY", 96, true),
        ("NEAR_KEY", 95, true),
        ("NEAR_KEY", 93, false),
        ("NEAR_KEY", 97, false),
    ] {
        let v = match kind {
            "NOSTR_KEY" => format!("nsec1{}", pad(BECH32, len - 5, 1)),
            "AGE_KEY" => format!(
                "AGE-SECRET-KEY-1{}",
                pad("QPZRY9X8GF2TVDW0S3JN54KHCE6MUA7L", len - 16, 6)
            ),
            "SUI_KEY" => format!("suiprivkey1{}", pad(LOWER36, len - 11, 4)),
            _ => format!("ed25519:{}", pad(B58, len - 8, 5)),
        };
        let (out, n) = mask(&masker(), &format!("key {v} end"), &secrets());
        if ok && (n != 1 || !out.contains(&format!("{{{{{kind}_"))) {
            errorf!(t, "{kind} at {len}: {n} masked: {out}");
        }
        if !ok && n != 0 {
            errorf!(t, "{kind} at {len} masked, want as written: {out}");
        }
    }
}

// 12 to 25 BIP-0039 words after a cue are a phrase. Fewer, words outside
// the list, a run with no cue, and a sentence after "recovery" or "seed"
// are not. "wallet" and "secret" are no cues.
#[test]
fn mask_crypto_seed_phrase() {
    let words: Vec<String> = list_phrase(12, 131).split(' ').map(str::to_owned).collect();
    let phrase = |n: usize| {
        (0..n)
            .map(|i| words[i % words.len()].as_str())
            .collect::<Vec<_>>()
            .join(" ")
    };
    let v1 = list_phrase(12, 997);
    let v2 = list_phrase(12, 389);
    let v2_capital = format!("{}{}", v2[..1].to_uppercase(), &v2[1..]);
    let mut t = Errors::default();
    for (s, v) in [
        (format!("seed phrase: {v1}"), v1.clone()),
        (format!("mnemonic = \"{v2}\""), v2.clone()),
        (format!("MNEMONIC={v2_capital}"), v2_capital.clone()),
        (format!("Recovery phrase:\n  {}", phrase(24)), phrase(24)),
        (format!("seed: {}. Keep it safe", phrase(15)), phrase(15)),
        (
            format!("seed phrase: {} and then more words", phrase(12)),
            phrase(12),
        ),
        (format!("seed phrase: {}", phrase(25)), phrase(25)),
        (
            format!("Seed phrase: my words are {}", phrase(12)),
            phrase(12),
        ),
        (format!("助记词：{}", phrase(12)), phrase(12)),
        (format!("シード: {}", phrase(18)), phrase(18)),
    ] {
        let m = masker();
        let (out, n) = mask(&m, &s, &secrets());
        if n != 1 || !out.contains("{{SEED_PHRASE_") || out.contains(&v) {
            errorf!(t, "{s:?}: {n} masked: {out}");
        }
        let back = restore(&m, &out);
        if back != s {
            errorf!(t, "{s:?}: restored {back:?}");
        }
    }
    // a 13th list word after a phrase is not the phrase's
    let (out, _) = mask(
        &masker(),
        &format!("seed phrase: {} able", phrase(12)),
        &secrets(),
    );
    if !out.ends_with("}} able") {
        errorf!(t, "13th word taken: {out}");
    }
    for s in [
        format!("seed phrase: {}", phrase(11)),
        // no cue at all
        phrase(12),
        format!("the wallet app lists {}", phrase(12)),
        format!("the secret is {}", phrase(12)),
        "recovery mode lets the system restore state after crash without losing user data".to_owned(),
        "seed the database with sample users before running each integration suite again today".to_owned(),
        "the phrase appears inside several long comments where people explain their design choices".to_owned(),
        "Seeded random numbers produce stable output across machines when tests compare golden files".to_owned(),
        format!("mnemonic: {} zebras {}", phrase(6), phrase(6)),
    ] {
        let (out, n) = mask(&masker(), &s, &secrets());
        if n != 0 || out != s {
            errorf!(t, "masked {s:?} ({n}): {out}");
        }
    }
}

// The embedded list is BIP-0039's: 2,048 words of 3 to 8 letters, from
// abandon to zoo, byte for byte.
#[test]
fn bip39_list() {
    let words: std::collections::HashSet<&str> = BIP39_LIST.split_whitespace().collect();
    assert!(
        words.len() == 2048
            && words.contains("abandon")
            && words.contains("zoo")
            && !words.contains("the")
    );
    assert_eq!(
        hex(&Sha256::digest(BIP39_LIST)),
        "2f5eed53a4727b4bf8880d8f3f199efc90e58503646d9ff8eff3a2ed3b24dbda"
    );
}

// A megabyte of base58, of hex after a cue and of list words after a cue is
// read through once: none of the patterns backtracks.
#[test]
fn mask_crypto_long_input() {
    let m = masker();
    for s in [
        rep(B58, 1 << 20),
        format!("PRIVATE_KEY={}", rep("0123456789abcdef", 1 << 20)),
        format!("seed phrase: {}", rep("abandon ", 1 << 20)),
        "[1,2,3,".repeat(1 << 17),
    ] {
        mask(&m, &s, &secrets());
    }
}

// An exchange's API key, secret and passphrase go by their fields' names,
// as ccxt configs, .env files and request headers have them.
#[test]
fn mask_exchange_keys() {
    let key = random_alnum("exchange key", 64);
    let secret = random_alnum("exchange secret", 64);
    let pass = "Okx9Pass2024";
    let mut t = Errors::default();
    for (s, v) in [
        (format!("BINANCE_API_KEY={key}"), key.as_str()),
        (format!("BINANCE_SECRET_KEY={secret}"), secret.as_str()),
        (
            format!(
                r#"exchange = ccxt.okx({{"apiKey": "{key}", "secret": "{secret}", "password": "{pass}"}})"#
            ),
            secret.as_str(),
        ),
        (format!("OKX_PASSPHRASE={pass}"), pass),
        (format!(r#"{{"passphrase": "{pass}"}}"#), pass),
        (format!("OK-ACCESS-PASSPHRASE: {pass}"), pass),
        (format!(r#"privKey = "{secret}""#), secret.as_str()),
        (format!("priv_key: {secret}"), secret.as_str()),
    ] {
        let (out, _) = mask(&masker(), &s, &secrets());
        if out.contains(v) {
            errorf!(t, "{s:?}: left in {out:?}");
        }
    }
}
