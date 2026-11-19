//! Edits JSON (and JSONC: comments, trailing commas) in place. An agent's
//! config is the user's file: setting one key leaves every other byte,
//! comments, order and indentation included, as it was.

use std::ops::Range;

use serde_json::Value;

#[derive(Debug)]
enum Node {
    Object {
        range: Range<usize>,
        members: Vec<Member>,
    },
    Array {
        range: Range<usize>,
        items: Vec<Node>,
    },
    Scalar {
        range: Range<usize>,
    },
}

impl Node {
    fn range(&self) -> Range<usize> {
        match self {
            Node::Object { range, .. } | Node::Array { range, .. } | Node::Scalar { range } => {
                range.clone()
            }
        }
    }
}

#[derive(Debug)]
struct Member {
    key: String,
    /// From the key's opening quote.
    start: usize,
    value: Node,
}

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn skip(&mut self) {
        loop {
            while self.pos < self.src.len() && self.src[self.pos].is_ascii_whitespace() {
                self.pos += 1;
            }
            if self.src[self.pos..].starts_with(b"//") {
                while self.pos < self.src.len() && self.src[self.pos] != b'\n' {
                    self.pos += 1;
                }
            } else if self.src[self.pos..].starts_with(b"/*") {
                match find(&self.src[self.pos + 2..], b"*/") {
                    Some(end) => self.pos += end + 4,
                    None => self.pos = self.src.len(),
                }
            } else {
                return;
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn value(&mut self) -> Result<Node, String> {
        self.skip();
        let start = self.pos;
        match self.peek().ok_or("unexpected end")? {
            b'{' => {
                self.pos += 1;
                let mut members = Vec::new();
                loop {
                    self.skip();
                    match self.peek().ok_or("unclosed object")? {
                        b'}' => {
                            self.pos += 1;
                            return Ok(Node::Object {
                                range: start..self.pos,
                                members,
                            });
                        }
                        b',' => self.pos += 1,
                        b'"' => {
                            let key_start = self.pos;
                            let key = self.string()?;
                            self.skip();
                            if self.peek() != Some(b':') {
                                return Err(format!("expected ':' at byte {}", self.pos));
                            }
                            self.pos += 1;
                            let value = self.value()?;
                            members.push(Member {
                                key,
                                start: key_start,
                                value,
                            });
                        }
                        c => {
                            return Err(format!("unexpected '{}' at byte {}", c as char, self.pos));
                        }
                    }
                }
            }
            b'[' => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    self.skip();
                    match self.peek().ok_or("unclosed array")? {
                        b']' => {
                            self.pos += 1;
                            return Ok(Node::Array {
                                range: start..self.pos,
                                items,
                            });
                        }
                        b',' => self.pos += 1,
                        _ => items.push(self.value()?),
                    }
                }
            }
            b'"' => {
                self.string()?;
                Ok(Node::Scalar {
                    range: start..self.pos,
                })
            }
            _ => {
                while self.pos < self.src.len() && !b",}] \t\r\n/".contains(&self.src[self.pos]) {
                    self.pos += 1;
                }
                if self.pos == start {
                    return Err(format!("unexpected character at byte {start}"));
                }
                Ok(Node::Scalar {
                    range: start..self.pos,
                })
            }
        }
    }

    fn string(&mut self) -> Result<String, String> {
        let start = self.pos;
        self.pos += 1;
        while self.pos < self.src.len() {
            match self.src[self.pos] {
                b'\\' => self.pos += 2,
                b'"' => {
                    self.pos += 1;
                    let raw = std::str::from_utf8(&self.src[start..self.pos])
                        .map_err(|e| e.to_string())?;
                    return serde_json::from_str(raw).map_err(|e| e.to_string());
                }
                _ => self.pos += 1,
            }
        }
        Err("unclosed string".into())
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn parse(text: &str) -> Result<Node, String> {
    let mut p = Parser {
        src: text.as_bytes(),
        pos: 0,
    };
    let node = p.value()?;
    Ok(node)
}

/// Reads JSONC as a value: comments and trailing commas dropped.
pub fn to_value(text: &str) -> Result<Value, String> {
    if text.trim().is_empty() {
        return Ok(Value::Object(Default::default()));
    }
    serde_json::from_str(&strip(text)).map_err(|e| e.to_string())
}

/// JSONC without its comments and trailing commas.
fn strip(text: &str) -> String {
    let src = text.as_bytes();
    let mut out = Vec::with_capacity(src.len());
    let mut i = 0;
    while i < src.len() {
        match src[i] {
            b'"' => {
                let start = i;
                i += 1;
                while i < src.len() && src[i] != b'"' {
                    i += if src[i] == b'\\' { 2 } else { 1 };
                }
                i = (i + 1).min(src.len());
                out.extend_from_slice(&src[start..i]);
            }
            b'/' if src.get(i + 1) == Some(&b'/') => {
                while i < src.len() && src[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if src.get(i + 1) == Some(&b'*') => {
                i = find(&src[i + 2..], b"*/")
                    .map(|e| i + e + 4)
                    .unwrap_or(src.len());
            }
            b',' => {
                let mut j = i + 1;
                loop {
                    while j < src.len() && src[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if src[j..].starts_with(b"//") {
                        while j < src.len() && src[j] != b'\n' {
                            j += 1;
                        }
                    } else if src[j..].starts_with(b"/*") {
                        j = find(&src[j + 2..], b"*/")
                            .map(|e| j + e + 4)
                            .unwrap_or(src.len());
                    } else {
                        break;
                    }
                }
                if !matches!(src.get(j), Some(b'}') | Some(b']')) {
                    out.push(b',');
                }
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_default()
}

/// The indentation of the line `pos` is on.
fn indent_at(text: &str, pos: usize) -> String {
    let line_start = text[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
    text[line_start..]
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect()
}

/// The unit of indentation the file uses (two spaces when it shows none).
fn indent_unit(text: &str) -> String {
    for line in text.lines() {
        let lead: String = line
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        if !lead.is_empty() && line.len() > lead.len() {
            return lead;
        }
    }
    "  ".to_owned()
}

/// `value` as pretty JSON whose lines after the first start with `indent`.
fn render(value: &Value, indent: &str, unit: &str) -> String {
    let mut buf = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(unit.as_bytes());
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);
    serde::Serialize::serialize(value, &mut ser).expect("a JSON value serializes");
    let text = String::from_utf8(buf).expect("JSON is UTF-8");
    text.replace('\n', &format!("\n{indent}"))
}

/// A step of a path that picks an item of a list: `[id=pansy]`, the item
/// whose `id` is `pansy`; `[=pansy]`, the item that is the text `pansy`.
pub fn selector(step: &str) -> Option<(&str, &str)> {
    step.strip_prefix('[')?.strip_suffix(']')?.split_once('=')
}

/// Whether `item` is the one the selector picks.
pub fn picks(item: &Value, key: &str, value: &str) -> bool {
    match key {
        "" => item.as_str() == Some(value),
        _ => item.get(key).and_then(Value::as_str) == Some(value),
    }
}

/// The value at `path` in `root`.
pub fn walk<'a>(root: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(root, |v, step| match selector(step) {
        Some((key, value)) => v.as_array()?.iter().find(|item| picks(item, key, value)),
        None => v.get(*step),
    })
}

/// `value` under `path`: an object for each key, a list of one for each
/// step that picks an item.
fn nest(path: &[&str], value: &Value) -> Value {
    path.iter().rev().fold(value.clone(), |rest, step| {
        if selector(step).is_some() {
            Value::Array(vec![rest])
        } else {
            let mut map = serde_json::Map::new();
            map.insert(step.to_string(), rest);
            Value::Object(map)
        }
    })
}

/// The item of `items` the selector picks.
fn picked(text: &str, items: &[Node], key: &str, value: &str) -> Option<usize> {
    items
        .iter()
        .position(|item| to_value(&text[item.range()]).is_ok_and(|v| picks(&v, key, value)))
}

/// `text` with `value` added after the list's last item.
fn append(text: &str, list: Range<usize>, items: &[Node], value: &Value, unit: &str) -> String {
    match items.last() {
        Some(last) => {
            let end = last.range().end;
            let (rendered, lead) = if text[list.clone()].contains('\n') {
                let indent = indent_at(text, last.range().start);
                (render(value, &indent, unit), format!(",\n{indent}"))
            } else {
                let indent = indent_at(text, list.start);
                (render(value, &indent, unit), ", ".to_owned())
            };
            // A trailing comma after the last item serves the new one.
            let after = &text[end..list.end - 1];
            match after.find(|c: char| !c.is_whitespace()) {
                Some(i) if after.as_bytes()[i] == b',' => {
                    let at = end + i + 1;
                    format!("{}{}{rendered},{}", &text[..at], &lead[1..], &text[at..])
                }
                _ => format!("{}{lead}{rendered}{}", &text[..end], &text[end..]),
            }
        }
        None => {
            let indent = indent_at(text, list.start);
            let inner = format!("{indent}{unit}");
            let rendered = render(value, &inner, unit);
            format!(
                "{}[\n{inner}{rendered}\n{indent}]{}",
                &text[..list.start],
                &text[list.end..]
            )
        }
    }
}

/// Sets `path` to `value` in `text`, creating the objects on the way. A
/// step that picks an item of a list ([`selector`]) sets that item, or adds
/// it after the others.
pub fn set(text: &str, path: &[&str], value: &Value) -> Result<String, String> {
    if text.trim().is_empty() {
        return Ok(format!("{}\n", render(&nest(path, value), "", "  ")));
    }
    let root = parse(text)?;
    let unit = indent_unit(text);
    let mut node = &root;
    for (depth, key) in path.iter().enumerate() {
        if let Some((by, is)) = selector(key) {
            let Node::Array { range, items } = node else {
                return Err(format!("{} is not a list", path[..depth].join(".")));
            };
            match picked(text, items, by, is) {
                Some(i) => node = &items[i],
                None => {
                    let rest = nest(&path[depth + 1..], value);
                    return Ok(append(text, range.clone(), items, &rest, &unit));
                }
            }
            continue;
        }
        let Node::Object { range, members } = node else {
            return Err(format!("{} is not an object", path[..depth].join(".")));
        };
        match members.iter().find(|m| m.key == *key) {
            Some(member) => node = &member.value,
            None => {
                // Build what is missing below this object.
                let rest = nest(&path[depth + 1..], value);
                return Ok(insert(text, range.clone(), members, key, &rest, &unit));
            }
        }
    }
    let range = node.range();
    let indent = indent_at(text, range.start);
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..range.start]);
    out.push_str(&render(value, &indent, &unit));
    out.push_str(&text[range.end..]);
    Ok(out)
}

fn insert(
    text: &str,
    object: Range<usize>,
    members: &[Member],
    key: &str,
    value: &Value,
    unit: &str,
) -> String {
    let key_json = serde_json::to_string(key).unwrap();
    match members.last() {
        Some(last) => {
            let indent = indent_at(text, last.start);
            let end = last.value.range().end;
            let close = object.end - 1;
            let after = &text[end..close];
            // A trailing comma after the last member stays where it is.
            let comma = after
                .find(|c: char| !c.is_whitespace())
                .filter(|&i| after.as_bytes()[i] == b',')
                .map(|i| end + i);
            let rendered = render(value, &indent, unit);
            if !text[object.clone()].contains('\n') {
                let at = comma.map(|c| c + 1).unwrap_or(end);
                let lead = if comma.is_some() { "" } else { "," };
                return format!(
                    "{}{lead} {key_json}: {rendered}{}",
                    &text[..at],
                    &text[at..]
                );
            }
            // On a line of its own, after a comment that ends the line before.
            let mut at = comma.map(|c| c + 1).unwrap_or(end);
            let line_end = text[at..]
                .find('\n')
                .map(|i| at + i)
                .unwrap_or(close)
                .min(close);
            let rest = text[at..line_end].trim();
            if rest.is_empty()
                || rest.starts_with("//")
                || (rest.starts_with("/*") && rest.ends_with("*/"))
            {
                at = line_end;
            }
            let mut out = String::with_capacity(text.len() + rendered.len() + 16);
            out.push_str(&text[..end]);
            if comma.is_none() {
                out.push(',');
            }
            out.push_str(&text[end..at]);
            out.push_str(&format!("\n{indent}{key_json}: {rendered}"));
            out.push_str(&text[at..]);
            out
        }
        None => {
            let indent = indent_at(text, object.start);
            let inner = format!("{indent}{unit}");
            let rendered = render(value, &inner, unit);
            format!(
                "{}{{\n{inner}{key_json}: {rendered}\n{indent}}}{}",
                &text[..object.start],
                &text[object.end..]
            )
        }
    }
}

/// Removes `path` from `text`; nothing happens when it is not there.
pub fn remove(text: &str, path: &[&str]) -> Result<String, String> {
    if text.trim().is_empty() || path.is_empty() {
        return Ok(text.to_owned());
    }
    let root = parse(text)?;
    let mut node = &root;
    for (depth, key) in path.iter().enumerate() {
        if let Some((by, is)) = selector(key) {
            let Node::Array { range, items } = node else {
                return Ok(text.to_owned());
            };
            let Some(i) = picked(text, items, by, is) else {
                return Ok(text.to_owned());
            };
            if depth + 1 < path.len() {
                node = &items[i];
                continue;
            }
            // The item with what parted it from the next; the last, with
            // what parted it from the one before; the only one, with all
            // the list held.
            let cut = if let Some(next) = items.get(i + 1) {
                items[i].range().start..next.range().start
            } else if i > 0 {
                items[i - 1].range().end..items[i].range().end
            } else {
                range.start + 1..range.end - 1
            };
            return Ok(format!("{}{}", &text[..cut.start], &text[cut.end..]));
        }
        let Node::Object { range, members } = node else {
            return Ok(text.to_owned());
        };
        let Some(i) = members.iter().position(|m| m.key == *key) else {
            return Ok(text.to_owned());
        };
        if depth + 1 < path.len() {
            node = &members[i].value;
            continue;
        }
        let member = &members[i];
        let end = member.value.range().end;
        let bytes = text.as_bytes();
        // Take the member with the comma after it, or the one before it
        // when it is the last; and the whitespace that led up to it.
        let mut cut_end = end;
        let mut j = end;
        while j < range.end && bytes[j].is_ascii_whitespace() && bytes[j] != b'\n' {
            j += 1;
        }
        let had_comma = bytes.get(j) == Some(&b',');
        if had_comma {
            cut_end = j + 1;
        }
        let mut cut_start = member.start;
        let line_start = text[..cut_start].rfind('\n').map(|n| n + 1).unwrap_or(0);
        let own_line = text[line_start..cut_start].trim().is_empty();
        if own_line {
            cut_start = line_start;
            // The newline that ends the member's line goes too.
            let rest = &text[cut_end..];
            let trailing = rest.len() - rest.trim_start_matches([' ', '\t']).len();
            if rest[trailing..].starts_with('\n') {
                cut_end += trailing + 1;
            } else if rest[trailing..].starts_with("\r\n") {
                cut_end += trailing + 2;
            }
        } else if had_comma {
            // `"a": 1, "b": 2` loses `"a": 1, ` whole.
            let rest = &text[cut_end..];
            cut_end += rest.len() - rest.trim_start_matches([' ', '\t']).len();
        }
        if !had_comma && i > 0 {
            // The last member: the comma before it goes, and on one line the
            // space after that comma.
            let prev_end = members[i - 1].value.range().end;
            if let Some(comma) = text[prev_end..cut_start].find(',') {
                let comma = prev_end + comma;
                let between = &text[comma + 1..cut_start];
                let keep = if !own_line && between.trim().is_empty() {
                    ""
                } else {
                    between
                };
                return Ok(format!("{}{keep}{}", &text[..comma], &text[cut_end..]));
            }
        }
        return Ok(format!("{}{}", &text[..cut_start], &text[cut_end..]));
    }
    Ok(text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const CONFIG: &str = r#"{
  // the user's own note
  "theme": "dark",
  "env": {
    "FOO": "1", /* keep */
  },
}
"#;

    #[test]
    fn replaces_a_value_and_keeps_the_rest() {
        let out = set(CONFIG, &["theme"], &json!("light")).unwrap();
        assert_eq!(out, CONFIG.replace("\"dark\"", "\"light\""));
    }

    #[test]
    fn adds_keys_after_a_trailing_comma() {
        let out = set(
            CONFIG,
            &["env", "ANTHROPIC_BASE_URL"],
            &json!("http://127.0.0.1:7267"),
        )
        .unwrap();
        assert!(out.contains(
            "\"FOO\": \"1\", /* keep */\n    \"ANTHROPIC_BASE_URL\": \"http://127.0.0.1:7267\""
        ));
        assert_eq!(
            to_value(&out).unwrap()["env"]["ANTHROPIC_BASE_URL"],
            "http://127.0.0.1:7267"
        );
        assert!(out.contains("// the user's own note"));
    }

    #[test]
    fn creates_the_objects_on_the_way() {
        let out = set(
            "{\n  \"a\": 1\n}\n",
            &["provider", "pansy", "name"],
            &json!("Pansy"),
        )
        .unwrap();
        assert_eq!(
            out,
            "{\n  \"a\": 1,\n  \"provider\": {\n    \"pansy\": {\n      \"name\": \"Pansy\"\n    }\n  }\n}\n"
        );
        let empty = set("{}", &["x"], &json!(true)).unwrap();
        assert_eq!(empty, "{\n  \"x\": true\n}");
        assert_eq!(
            set("", &["x", "y"], &json!(1)).unwrap(),
            "{\n  \"x\": {\n    \"y\": 1\n  }\n}\n"
        );
    }

    #[test]
    fn sets_adds_and_removes_an_item_of_a_list() {
        let mine = "{\n  // mine\n  \"providers\": [\n    { \"id\": \"own\", \"url\": \"x\" } // kept\n  ]\n}\n";
        let item = json!({"id": "pansy", "url": "u"});
        let added = set(mine, &["providers", "[id=pansy]"], &item).unwrap();
        assert_eq!(
            added,
            "{\n  // mine\n  \"providers\": [\n    { \"id\": \"own\", \"url\": \"x\" },\n    {\n      \"id\": \"pansy\",\n      \"url\": \"u\"\n    } // kept\n  ]\n}\n"
        );
        assert_eq!(
            walk(
                &to_value(&added).unwrap(),
                &["providers", "[id=pansy]", "url"]
            ),
            Some(&json!("u"))
        );
        // again: the item is replaced, not added twice
        let again = set(
            &added,
            &["providers", "[id=pansy]"],
            &json!({"id": "pansy", "url": "v"}),
        )
        .unwrap();
        assert_eq!(
            to_value(&again).unwrap()["providers"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(to_value(&again).unwrap()["providers"][1]["url"], "v");
        let inside = set(&added, &["providers", "[id=pansy]", "url"], &json!("w")).unwrap();
        assert_eq!(to_value(&inside).unwrap()["providers"][1]["url"], "w");
        let gone = remove(&added, &["providers", "[id=pansy]"]).unwrap();
        assert_eq!(to_value(&gone).unwrap(), to_value(mine).unwrap());
        assert!(gone.contains("// mine") && gone.contains("\"own\""));

        // a list of texts, on one line, and one there is none of yet
        let one = "{\"on\": [\"a\", \"b\"]}";
        let more = set(one, &["on", "[=c]"], &json!("c")).unwrap();
        assert_eq!(more, "{\"on\": [\"a\", \"b\", \"c\"]}");
        assert_eq!(set(&more, &["on", "[=c]"], &json!("c")).unwrap(), more);
        assert_eq!(remove(&more, &["on", "[=c]"]).unwrap(), one);
        assert_eq!(
            remove(&more, &["on", "[=a]"]).unwrap(),
            "{\"on\": [\"b\", \"c\"]}"
        );
        let new = set("{}", &["list", "[id=p]"], &json!({"id": "p"})).unwrap();
        assert_eq!(to_value(&new).unwrap(), json!({"list": [{"id": "p"}]}));
        assert_eq!(
            to_value(&remove(&new, &["list", "[id=p]"]).unwrap()).unwrap(),
            json!({"list": []})
        );
        // the file is the list
        let root = set("", &["[id=p]"], &json!({"id": "p"})).unwrap();
        assert_eq!(to_value(&root).unwrap(), json!([{"id": "p"}]));
        let root = set(
            "[\n  {\"id\": \"own\"},\n]\n",
            &["[id=p]"],
            &json!({"id": "p"}),
        )
        .unwrap();
        assert_eq!(
            to_value(&root).unwrap(),
            json!([{"id": "own"}, {"id": "p"}])
        );
    }

    #[test]
    fn removes_members() {
        let text = "{\n  \"a\": 1,\n  \"b\": 2,\n  \"c\": 3\n}\n";
        assert_eq!(
            remove(text, &["b"]).unwrap(),
            "{\n  \"a\": 1,\n  \"c\": 3\n}\n"
        );
        assert_eq!(
            remove(text, &["c"]).unwrap(),
            "{\n  \"a\": 1,\n  \"b\": 2\n}\n"
        );
        assert_eq!(
            remove(text, &["a"]).unwrap(),
            "{\n  \"b\": 2,\n  \"c\": 3\n}\n"
        );
        assert_eq!(remove(text, &["zz"]).unwrap(), text);
        let removed = remove(CONFIG, &["env", "FOO"]).unwrap();
        assert!(
            to_value(&removed).unwrap()["env"]
                .as_object()
                .unwrap()
                .is_empty()
        );
        let inline = "{\"env\": {\"A\": 1, \"B\": 2, \"C\": 3}}";
        assert_eq!(
            remove(inline, &["env", "A"]).unwrap(),
            "{\"env\": {\"B\": 2, \"C\": 3}}"
        );
        assert_eq!(
            remove(inline, &["env", "C"]).unwrap(),
            "{\"env\": {\"A\": 1, \"B\": 2}}"
        );
        assert_eq!(
            remove(inline, &["env", "B"]).unwrap(),
            "{\"env\": {\"A\": 1, \"C\": 3}}"
        );
    }
}
