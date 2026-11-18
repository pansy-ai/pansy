//! What the gateway carries between protocols, as a table: `docs/compat.md`
//! is written by this test, and the test fails when the file says anything
//! the code doesn't do.
//!
//! Each row is one thing an agent may ask or a vendor may answer, written
//! by hand in all four protocols as agents and vendors send it. A cell
//! takes the agent's request through the gateway's own readers and writers
//! to the vendor's protocol, looks at what the vendor would get, reads it
//! back and checks the thing is still there; answers go the other way,
//! whole and streamed. After a change that is meant:
//!
//! ```sh
//! PANSY_BLESS=1 cargo test -p pansy-gateway --test compat
//! ```

mod compat_data;

use compat_data::{ANSWERS, Answer, Ask, REQUESTS};
use pansy_gateway::ir::{Collector, Request, Response};
use pansy_gateway::proto::{self, Protocol, Quirks};
use pansy_gateway::sse::Decoder;
use serde_json::Value;

const ORDER: [Protocol; 4] = [
    Protocol::Anthropic,
    Protocol::Chat,
    Protocol::Responses,
    Protocol::Gemini,
];

fn name(p: Protocol) -> &'static str {
    match p {
        Protocol::Anthropic => "Anthropic",
        Protocol::Chat => "Chat",
        Protocol::Responses => "Responses",
        Protocol::Gemini => "Gemini",
    }
}

/// An OpenAI-kind Chat vendor: the reasoning effort and
/// `reasoning_content` go only to providers marked so.
fn quirks() -> Quirks {
    Quirks {
        max_completion_tokens: true,
        reasoning_content: true,
    }
}

/// Every `(pointer, needle)` holds in `body`: the JSON at the pointer, as
/// text, has the needle; a needle that begins with `!` must be absent.
fn wire(body: &Value, checks: &[(&str, &str)]) -> Result<(), String> {
    for (pointer, needle) in checks {
        let at = body.pointer(pointer).map(Value::to_string);
        match needle.strip_prefix('!') {
            Some(absent) => {
                if at.as_deref().is_some_and(|text| text.contains(absent)) {
                    return Err(format!("{absent} reached the vendor"));
                }
            }
            None => match at {
                Some(text) if text.contains(needle) => {}
                Some(_) => return Err(format!("no {needle} at {pointer}")),
                None => return Err(format!("nothing at {pointer}")),
            },
        }
    }
    Ok(())
}

fn json(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("a sample is not JSON ({e}): {text}"))
}

/// An agent's request in `from`, as a vendor of `to` gets it.
fn ask(row: &Ask, from: usize, to: usize) -> Result<(), String> {
    let sample = row.samples[from].ok_or("not asked")?;
    let read: Request = proto::parse_request(ORDER[from], &json(sample))?;
    if from == to {
        // The same protocol passes through untouched; the sample itself
        // must show the thing, or the row proves nothing.
        return (row.check)(&read);
    }
    let body = proto::build_request(ORDER[to], &read, "m", &quirks());
    wire(&body, row.wire[to])?;
    if row.wire_only {
        return Ok(());
    }
    (row.check)(&proto::parse_request(ORDER[to], &body)?)
}

/// A vendor's whole answer in `from`, as an agent of `to` gets it.
fn answer(row: &Answer, from: usize, to: usize) -> Result<(), String> {
    let read: Response = proto::parse_response(ORDER[from], &json(row.whole[from]))?;
    if from == to {
        return (row.check)(&read);
    }
    let body = proto::build_response(ORDER[to], &read, &Request::default());
    wire(&body, row.wire[to])?;
    (row.check)(&proto::parse_response(ORDER[to], &body)?)
}

/// A stream in `protocol`, read into a whole answer and the events it was.
fn read_stream(
    protocol: Protocol,
    text: &str,
) -> Result<(Response, Vec<pansy_gateway::ir::Event>), String> {
    let mut decoder = Decoder::default();
    let mut frames = decoder.push(text.as_bytes());
    frames.extend(decoder.finish());
    let mut reader = proto::reader(protocol);
    let mut collector = Collector::default();
    let mut events = Vec::new();
    for frame in &frames {
        for event in reader.frame(frame)? {
            events.push(event.clone());
            collector.push(event);
        }
    }
    Ok((collector.finish(), events))
}

/// A vendor's streamed answer in `from`, as an agent of `to` gets it.
fn stream(row: &Answer, from: usize, to: usize) -> Result<(), String> {
    let (whole, events) = read_stream(ORDER[from], row.stream[from])?;
    if from == to {
        return (row.check)(&whole);
    }
    let mut writer = proto::writer(ORDER[to], &Request::default(), "m", false);
    let mut out = writer.begin();
    for event in events {
        out.push_str(&writer.event(event));
    }
    out.push_str(&writer.finish());
    (row.check)(&read_stream(ORDER[to], &out)?.0)
}

/// One cell: every other protocol's side of it, into (or out of) `to`.
fn cell(
    to: usize,
    skip: impl Fn(usize) -> bool,
    run: impl Fn(usize, usize) -> Result<(), String>,
) -> String {
    let mut problems = Vec::new();
    for (from, protocol) in ORDER.iter().enumerate() {
        if skip(from) {
            continue;
        }
        if let Err(e) = run(from, to) {
            let what = if from == to {
                format!("the {} sample: {e}", name(*protocol))
            } else {
                format!("from {}: {e}", name(*protocol))
            };
            problems.push(what);
        }
    }
    if problems.is_empty() {
        "✓".to_owned()
    } else {
        format!("✗ {}", problems.join("; "))
    }
}

fn table(out: &mut String, rows: Vec<(String, [String; 4])>) {
    out.push_str("| | Anthropic | Chat | Responses | Gemini |\n| :-- | :-- | :-- | :-- | :-- |\n");
    for (title, cells) in rows {
        out.push_str(&format!("| {title} | {} |\n", cells.join(" | ")));
    }
    out.push('\n');
}

fn render() -> String {
    let mut out = String::from(
        "# What the gateway carries between protocols\n\n\
         Written by `cargo test -p pansy-gateway --test compat`, which fails when this page\n\
         and the code disagree. A request in the vendor's own protocol passes through\n\
         untouched and is not in these tables.\n\n\
         A column is the protocol the **vendor** speaks. ✓: the thing arrives from each of\n\
         the other three protocols. —: the protocol has no such thing, and why. ✗: it is\n\
         lost, from where, and how.\n\n\
         ## Requests\n\n",
    );
    let mut rows = Vec::new();
    for row in REQUESTS {
        let cells: [String; 4] = std::array::from_fn(|to| match row.no_vendor[to] {
            Some(why) => format!("— {why}"),
            None => cell(
                to,
                |from| row.samples[from].is_none(),
                |f, t| ask(row, f, t),
            ),
        });
        let from: Vec<&str> = (0..4)
            .filter(|i| row.samples[*i].is_some())
            .map(|i| name(ORDER[i]))
            .collect();
        let title = if from.len() == 4 {
            row.title.to_owned()
        } else {
            format!("{} (asked in {})", row.title, from.join(", "))
        };
        rows.push((title, cells));
    }
    table(&mut out, rows);
    out.push_str(
        "The Chat column is a vendor marked as OpenAI's kind: only those are sent\n\
         `reasoning_effort`, `max_completion_tokens` and reasoning back as `reasoning_content`.\n\n\
         ## Answers\n\n\
         A column is the protocol the **agent** speaks; the answer comes from a vendor of each\n\
         of the other three.\n\n### Whole\n\n",
    );
    let whole = ANSWERS
        .iter()
        .map(|row| {
            (
                row.title.to_owned(),
                std::array::from_fn(|to| cell(to, |_| false, |f, t| answer(row, f, t))),
            )
        })
        .collect();
    table(&mut out, whole);
    out.push_str("### Streamed\n\n");
    let streamed = ANSWERS
        .iter()
        .map(|row| {
            (
                row.title.to_owned(),
                std::array::from_fn(|to| cell(to, |_| false, |f, t| stream(row, f, t))),
            )
        })
        .collect();
    table(&mut out, streamed);
    out
}

#[test]
fn the_page_says_what_the_code_does() {
    let page = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/compat.md");
    let now = render();
    if std::env::var_os("PANSY_BLESS").is_some() {
        std::fs::write(&page, &now).unwrap();
    }
    let kept = std::fs::read_to_string(&page).unwrap_or_default();
    assert!(
        now == kept,
        "docs/compat.md is not what the code does; see this test's comment.\n\n{now}"
    );
}

/// A sample that doesn't show its own thing would make its row say nothing.
#[test]
fn every_sample_shows_its_own_thing() {
    for row in REQUESTS {
        for i in (0..4).filter(|i| row.samples[*i].is_some()) {
            if let Err(e) = ask(row, i, i) {
                panic!("{}: the {} sample: {e}", row.title, name(ORDER[i]));
            }
        }
    }
    for row in ANSWERS {
        for (i, protocol) in ORDER.iter().enumerate() {
            if let Err(e) = answer(row, i, i) {
                panic!("{} (whole): the {} sample: {e}", row.title, name(*protocol));
            }
            if let Err(e) = stream(row, i, i) {
                panic!(
                    "{} (streamed): the {} sample: {e}",
                    row.title,
                    name(*protocol)
                );
            }
        }
    }
}
