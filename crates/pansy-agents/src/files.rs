//! Reading and setting one key of an agent's config file, in its own
//! format, leaving the rest of the file as the user wrote it.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;

use crate::jsonedit;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// JSON, or JSONC (comments, trailing commas).
    Json,
    Toml,
    /// `KEY=value` lines (`.env`).
    Env,
    /// YAML, edited by top-level and nested keys of plain scalars.
    Yaml,
}

pub fn read_text(path: &Path) -> Result<String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// The value at `path` in `text`, `None` when it is not there.
pub fn get(format: Format, text: &str, path: &[&str]) -> Result<Option<Value>> {
    let text = text.strip_prefix(BOM).unwrap_or(text);
    match format {
        Format::Json => {
            let root = jsonedit::to_value(text).map_err(|e| anyhow!(e))?;
            Ok(jsonedit::walk(&root, path).cloned())
        }
        Format::Toml => {
            if text.trim().is_empty() {
                return Ok(None);
            }
            let doc: toml_edit::DocumentMut = text.parse()?;
            if path.iter().any(|step| jsonedit::selector(step).is_some()) {
                let root = toml_to_json(doc.as_item()).unwrap_or_default();
                return Ok(jsonedit::walk(&root, path).cloned());
            }
            let mut item = doc.as_item();
            for key in path {
                match item.get(key) {
                    Some(next) => item = next,
                    None => return Ok(None),
                }
            }
            Ok(toml_to_json(item))
        }
        _ if picks_item(path) => Ok(None),
        Format::Env => {
            let key = path.last().copied().unwrap_or_default();
            Ok(env_lines(text)
                .find(|(k, _)| *k == key)
                .map(|(_, v)| Value::String(v)))
        }
        Format::Yaml => Ok(yaml_get(text, path).map(Value::String)),
    }
}

/// `text` with `path` set to `value`, or removed when `value` is `None`.
pub fn set(format: Format, text: &str, path: &[&str], value: Option<&Value>) -> Result<String> {
    // A file that starts with a byte order mark keeps it.
    match text.strip_prefix(BOM) {
        Some(rest) => Ok(format!("{BOM}{}", set_plain(format, rest, path, value)?)),
        None => set_plain(format, text, path, value),
    }
}

const BOM: char = '\u{feff}';

fn set_plain(format: Format, text: &str, path: &[&str], value: Option<&Value>) -> Result<String> {
    match format {
        Format::Json => match value {
            Some(v) => jsonedit::set(text, path, v),
            None => jsonedit::remove(text, path),
        }
        .map_err(|e| anyhow!(e)),
        Format::Toml => {
            let mut doc: toml_edit::DocumentMut = if text.trim().is_empty() {
                Default::default()
            } else {
                text.parse()?
            };
            let (last, parents) = path.split_last().ok_or_else(|| anyhow!("an empty key"))?;
            if parents
                .iter()
                .any(|step| jsonedit::selector(step).is_some())
            {
                bail!("in TOML a list's item is set whole: {}", path.join("."));
            }
            if let Some((by, is)) = jsonedit::selector(last) {
                toml_item(&mut doc, parents, by, is, value)?;
                return Ok(doc.to_string());
            }
            let mut table = doc.as_table_mut();
            for key in parents {
                if value.is_none() && !table.contains_key(key) {
                    return Ok(doc.to_string());
                }
                let entry = table.entry(key).or_insert_with(|| {
                    // `[a.b]` alone, without an empty `[a]` above it.
                    let mut t = toml_edit::Table::new();
                    t.set_implicit(true);
                    toml_edit::Item::Table(t)
                });
                table = entry
                    .as_table_mut()
                    .ok_or_else(|| anyhow!("{key} is not a table"))?;
            }
            match value {
                Some(v) => {
                    let new = json_to_toml(v).ok_or_else(|| anyhow!("no TOML for {v}"))?;
                    match table.get_mut(last) {
                        // Keep the comment and spacing around the old value.
                        Some(toml_edit::Item::Value(old)) => {
                            let decor = old.decor().clone();
                            *old = new;
                            *old.decor_mut() = decor;
                        }
                        _ => {
                            table.insert(last, toml_edit::Item::Value(new));
                        }
                    }
                }
                None => {
                    // A comment above the key is the user's: it stays, above
                    // the key that follows.
                    let above = table
                        .key(last)
                        .and_then(|k| k.leaf_decor().prefix()?.as_str().map(str::to_owned))
                        .filter(|p| p.contains('#'));
                    let next = table
                        .iter()
                        .skip_while(|(k, _)| k != last)
                        .nth(1)
                        .filter(|(_, v)| v.is_value())
                        .map(|(k, _)| k.to_owned());
                    table.remove(last);
                    if let (Some(above), Some(next)) = (above, next)
                        && let Some(mut key) = table.key_mut(&next)
                    {
                        let own = key
                            .leaf_decor()
                            .prefix()
                            .and_then(|p| p.as_str())
                            .unwrap_or_default()
                            .to_owned();
                        key.leaf_decor_mut().set_prefix(format!("{above}{own}"));
                    }
                    // Tables left empty go too, deepest first.
                    prune(doc.as_table_mut(), parents);
                }
            }
            Ok(doc.to_string())
        }
        _ if picks_item(path) => bail!(
            "a list's item ({}) is set in JSON and TOML alone",
            path.join(".")
        ),
        Format::Env => {
            let key = path.last().copied().unwrap_or_default();
            Ok(env_set(text, key, value.and_then(Value::as_str)))
        }
        Format::Yaml => Ok(yaml_set(
            text,
            path,
            value
                .and_then(|v| {
                    v.as_str()
                        .map(str::to_owned)
                        .or_else(|| Some(v.to_string()))
                })
                .as_deref(),
        )),
    }
}

/// Whether a step of `path` picks an item of a list.
fn picks_item(path: &[&str]) -> bool {
    path.iter().any(|step| jsonedit::selector(step).is_some())
}

/// Sets, adds or removes the item of the list at `list` that `by` and `is`
/// pick: a `[[table]]` of its own for a list of tables, else an item of an
/// array. The other items stay as they are written.
fn toml_item(
    doc: &mut toml_edit::DocumentMut,
    list: &[&str],
    by: &str,
    is: &str,
    value: Option<&Value>,
) -> Result<()> {
    let (last, parents) = list
        .split_last()
        .ok_or_else(|| anyhow!("a TOML file is a table, not a list"))?;
    let mut table = doc.as_table_mut();
    for key in parents {
        if value.is_none() && !table.contains_key(key) {
            return Ok(());
        }
        let entry = table.entry(key).or_insert_with(|| {
            let mut t = toml_edit::Table::new();
            t.set_implicit(true);
            toml_edit::Item::Table(t)
        });
        table = entry
            .as_table_mut()
            .ok_or_else(|| anyhow!("{key} is not a table"))?;
    }
    let new_table = |v: &Value| -> Option<toml_edit::Table> {
        let mut t = toml_edit::Table::new();
        for (k, v) in v.as_object()? {
            t.insert(k, toml_edit::Item::Value(json_to_toml(v)?));
        }
        Some(t)
    };
    match table.get_mut(last) {
        Some(toml_edit::Item::ArrayOfTables(tables)) => {
            let at = tables
                .iter()
                .position(|t| !by.is_empty() && t.get(by).and_then(|v| v.as_str()) == Some(is));
            match (value, at) {
                (Some(v), at) => {
                    let new = new_table(v).ok_or_else(|| anyhow!("no TOML table for {v}"))?;
                    match at.and_then(|i| tables.get_mut(i)) {
                        Some(old) => {
                            let decor = old.decor().clone();
                            *old = new;
                            *old.decor_mut() = decor;
                        }
                        None => tables.push(new),
                    }
                }
                (None, Some(i)) => tables.remove(i),
                (None, None) => {}
            }
        }
        Some(toml_edit::Item::Value(toml_edit::Value::Array(items))) => {
            let at = items
                .iter()
                .position(|v| toml_value_to_json(v).is_some_and(|v| jsonedit::picks(&v, by, is)));
            match (value, at) {
                (Some(v), at) => {
                    let new = json_to_toml(v).ok_or_else(|| anyhow!("no TOML for {v}"))?;
                    match at {
                        Some(i) => {
                            items.replace(i, new);
                        }
                        None => items.push(new),
                    }
                }
                (None, Some(i)) => {
                    items.remove(i);
                }
                (None, None) => {}
            }
        }
        Some(_) => bail!("{last} is not a list"),
        None => {
            if let Some(v) = value {
                let item = match new_table(v) {
                    Some(t) => {
                        let mut tables = toml_edit::ArrayOfTables::new();
                        tables.push(t);
                        toml_edit::Item::ArrayOfTables(tables)
                    }
                    None => {
                        let mut items = toml_edit::Array::new();
                        items.push(json_to_toml(v).ok_or_else(|| anyhow!("no TOML for {v}"))?);
                        toml_edit::Item::Value(toml_edit::Value::Array(items))
                    }
                };
                table.insert(last, item);
            }
        }
    }
    Ok(())
}

/// Removes the tables along `path` that are empty, deepest first.
fn prune(table: &mut toml_edit::Table, path: &[&str]) {
    let Some((first, rest)) = path.split_first() else {
        return;
    };
    let empty = match table.get_mut(first).and_then(|i| i.as_table_mut()) {
        Some(child) => {
            prune(child, rest);
            child.is_empty()
        }
        None => false,
    };
    if empty {
        table.remove(first);
    }
}

fn toml_value_to_json(v: &toml_edit::Value) -> Option<Value> {
    match v {
        toml_edit::Value::String(s) => Some(Value::String(s.value().clone())),
        toml_edit::Value::Integer(i) => Some(Value::from(*i.value())),
        toml_edit::Value::Float(f) => serde_json::Number::from_f64(*f.value()).map(Value::Number),
        toml_edit::Value::Boolean(b) => Some(Value::Bool(*b.value())),
        // A list and a table written in one line are what they hold, so
        // putting one back writes a list or a table again, not its text.
        toml_edit::Value::Array(items) => Some(Value::Array(
            items.iter().filter_map(toml_value_to_json).collect(),
        )),
        toml_edit::Value::InlineTable(t) => Some(Value::Object(
            t.iter()
                .filter_map(|(k, v)| Some((k.to_owned(), toml_value_to_json(v)?)))
                .collect(),
        )),
        other => Some(Value::String(other.to_string().trim().to_owned())),
    }
}

fn toml_to_json(item: &toml_edit::Item) -> Option<Value> {
    match item {
        toml_edit::Item::Value(v) => toml_value_to_json(v),
        toml_edit::Item::Table(t) => {
            let mut map = serde_json::Map::new();
            for (k, v) in t.iter() {
                if let Some(v) = toml_to_json(v) {
                    map.insert(k.to_owned(), v);
                }
            }
            Some(Value::Object(map))
        }
        toml_edit::Item::ArrayOfTables(tables) => Some(Value::Array(
            tables
                .iter()
                .filter_map(|t| toml_to_json(&toml_edit::Item::Table(t.clone())))
                .collect(),
        )),
        toml_edit::Item::None => None,
    }
}

fn json_to_toml(value: &Value) -> Option<toml_edit::Value> {
    Some(match value {
        Value::String(s) => s.as_str().into(),
        Value::Bool(b) => (*b).into(),
        Value::Number(n) => match n.as_i64() {
            Some(i) => i.into(),
            None => n.as_f64()?.into(),
        },
        Value::Array(items) => {
            let mut array = toml_edit::Array::new();
            for item in items {
                array.push(json_to_toml(item)?);
            }
            toml_edit::Value::Array(array)
        }
        Value::Object(map) => {
            let mut table = toml_edit::InlineTable::new();
            for (k, v) in map {
                table.insert(k, json_to_toml(v)?);
            }
            toml_edit::Value::InlineTable(table)
        }
        Value::Null => return None,
    })
}

/// What follows a line's `=`: the value, the quote it was written in, and
/// what comes after it (spaces and a comment), kept as it is.
fn env_value(raw: &str) -> (String, Option<char>, &str) {
    let raw = raw.trim_start();
    if let Some(rest) = raw.strip_prefix('"') {
        let mut value = String::new();
        let mut chars = rest.char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                '\\' => match chars.next() {
                    Some((_, e @ ('"' | '\\'))) => value.push(e),
                    Some((_, e)) => {
                        value.push('\\');
                        value.push(e);
                    }
                    None => value.push('\\'),
                },
                '"' => return (value, Some('"'), &rest[i + 1..]),
                c => value.push(c),
            }
        }
    } else if let Some(rest) = raw.strip_prefix('\'')
        && let Some(end) = rest.find('\'')
    {
        return (rest[..end].to_owned(), Some('\''), &rest[end + 1..]);
    }
    // Unquoted: a comment starts at a `#` after a space.
    let end = raw
        .char_indices()
        .find(|&(i, c)| c == '#' && i > 0 && raw[..i].ends_with([' ', '\t']))
        .map_or(raw.len(), |(i, _)| i);
    let value = raw[..end].trim_end();
    (value.to_owned(), None, &raw[value.len()..])
}

fn env_lines(text: &str) -> impl Iterator<Item = (&str, String)> {
    text.lines().filter_map(|line| {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line);
        if line.starts_with('#') {
            return None;
        }
        let (k, v) = line.split_once('=')?;
        Some((k.trim(), env_value(v).0))
    })
}

/// `text` with `key` set to `value` (or its line removed): a line that has
/// it keeps its quotes and its comment, and the file its line endings.
fn env_set(text: &str, key: &str, value: Option<&str>) -> String {
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut out = Vec::new();
    let mut done = false;
    let mut added = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        let bare = trimmed.strip_prefix("export ").unwrap_or(trimmed);
        let existing = (!trimmed.starts_with('#'))
            .then(|| bare.split_once('='))
            .flatten()
            .filter(|(k, _)| k.trim() == key);
        if let Some((_, raw)) = existing {
            if let (Some(v), false) = (value, done) {
                let export = if trimmed.starts_with("export ") {
                    "export "
                } else {
                    ""
                };
                let (_, quote, after) = env_value(raw);
                out.push(format!("{export}{key}={}{after}", quote_env_as(v, quote)));
                done = true;
            }
            continue;
        }
        out.push(line.to_owned());
    }
    if let (Some(v), false) = (value, done) {
        out.push(format!("{key}={}", quote_env(v)));
        added = true;
    }
    let mut out = out.join(newline);
    if !out.is_empty() && (added || text.ends_with('\n')) {
        out.push_str(newline);
    }
    out
}

/// `value` in the quotes the line had it in, where it can be.
fn quote_env_as(value: &str, quote: Option<char>) -> String {
    match quote {
        Some('"') => format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"")),
        Some('\'') if !value.contains('\'') => format!("'{value}'"),
        _ => quote_env(value),
    }
}

fn quote_env(value: &str) -> String {
    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_./:@".contains(c))
    {
        value.to_owned()
    } else {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// The scalar at `path` in a block-style YAML file.
fn yaml_get(text: &str, path: &[&str]) -> Option<String> {
    let (line, _) = yaml_find(text, path)?;
    let raw = text.lines().nth(line)?;
    let (_, value) = raw.split_once(':')?;
    let value = value.split(" #").next().unwrap_or("").trim();
    let value = value.trim_matches(|c| c == '"' || c == '\'');
    if value.is_empty() {
        None
    } else {
        Some(value.to_owned())
    }
}

/// The line of `path`'s key and its indent, in a block-style YAML file.
fn yaml_find(text: &str, path: &[&str]) -> Option<(usize, usize)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut start = 0;
    let mut end = lines.len();
    let mut indent = 0;
    let mut found = None;
    for (depth, key) in path.iter().enumerate() {
        let hit = (start..end).find(|&i| {
            let l = lines[i];
            let lead = l.len() - l.trim_start().len();
            (if depth == 0 {
                lead == 0
            } else {
                lead >= indent
            }) && l
                .trim_start()
                .strip_prefix(key)
                .is_some_and(|rest| rest.starts_with(':'))
        })?;
        let lead = lines[hit].len() - lines[hit].trim_start().len();
        found = Some((hit, lead));
        start = hit + 1;
        end = (start..lines.len())
            .find(|&i| {
                let l = lines[i];
                !l.trim().is_empty()
                    && !l.trim_start().starts_with('#')
                    && l.len() - l.trim_start().len() <= lead
            })
            .unwrap_or(lines.len());
        indent = lead + 1;
    }
    found
}

fn yaml_set(text: &str, path: &[&str], value: Option<&str>) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let quoted = |v: &str| {
        if v.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c))
        {
            v.to_owned()
        } else {
            format!("\"{}\"", v.replace('"', "\\\""))
        }
    };
    match (yaml_find(text, path), value) {
        (Some((line, lead)), Some(v)) => {
            let key = path.last().unwrap();
            lines[line] = format!("{}{key}: {}", " ".repeat(lead), quoted(v));
        }
        (Some((line, _)), None) => {
            lines.remove(line);
        }
        (None, Some(v)) => {
            // Add what is missing under the deepest key that is there.
            let mut have = 0;
            let mut at = (lines.len(), 0usize);
            for depth in (1..=path.len()).rev() {
                if let Some((line, lead)) = yaml_find(text, &path[..depth]) {
                    have = depth;
                    let end = (line + 1..lines.len())
                        .find(|&i| {
                            let l = &lines[i];
                            !l.trim().is_empty() && l.len() - l.trim_start().len() <= lead
                        })
                        .unwrap_or(lines.len());
                    at = (end, lead + 2);
                    break;
                }
            }
            let mut insert = Vec::new();
            for (i, key) in path[have..].iter().enumerate() {
                let pad = " ".repeat(at.1 + i * 2);
                if have + i + 1 == path.len() {
                    insert.push(format!("{pad}{key}: {}", quoted(v)));
                } else {
                    insert.push(format!("{pad}{key}:"));
                }
            }
            if have == 0 {
                for (i, l) in insert.iter_mut().enumerate() {
                    *l = format!("{}{}", " ".repeat(i * 2), l.trim_start());
                }
            }
            for (i, l) in insert.into_iter().enumerate() {
                lines.insert(at.0 + i, l);
            }
        }
        (None, None) => {}
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn edits_toml_and_keeps_comments() {
        let text = "# my codex\nmodel = \"gpt-5\" # the default\n\n[projects.\"/x\"]\ntrust_level = \"trusted\"\n";
        let out = set(
            Format::Toml,
            text,
            &["model"],
            Some(&json!("deepseek/deepseek-chat")),
        )
        .unwrap();
        assert!(out.starts_with("# my codex\nmodel = \"deepseek/deepseek-chat\" # the default\n"));
        let out = set(
            Format::Toml,
            &out,
            &["model_providers", "pansy", "base_url"],
            Some(&json!("http://127.0.0.1:7267/v1")),
        )
        .unwrap();
        assert_eq!(
            get(
                Format::Toml,
                &out,
                &["model_providers", "pansy", "base_url"]
            )
            .unwrap(),
            Some(json!("http://127.0.0.1:7267/v1"))
        );
        assert!(out.contains("trust_level = \"trusted\""));
        let back = set(
            Format::Toml,
            &out,
            &["model_providers", "pansy", "base_url"],
            None,
        )
        .unwrap();
        assert_eq!(
            get(
                Format::Toml,
                &back,
                &["model_providers", "pansy", "base_url"]
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn sets_adds_and_removes_an_item_of_a_toml_list() {
        let mine = "# mine\nactive = \"own\"\nenabled = [\"a\"]\n\n[[providers]]\nname = \"own\" # kept\nurl = \"x\"\n";
        let item = json!({"name": "pansy", "url": "u"});
        let path = ["providers", "[name=pansy]"];
        let added = set(Format::Toml, mine, &path, Some(&item)).unwrap();
        assert!(added.starts_with(mine), "{added}");
        assert!(
            added.contains("[[providers]]\nname = \"pansy\"\nurl = \"u\"\n"),
            "{added}"
        );
        assert_eq!(
            get(Format::Toml, &added, &path).unwrap(),
            Some(item.clone())
        );
        assert_eq!(
            get(Format::Toml, &added, &["providers", "[name=pansy]", "url"]).unwrap(),
            Some(json!("u"))
        );
        assert_eq!(get(Format::Toml, mine, &path).unwrap(), None);
        let again = set(
            Format::Toml,
            &added,
            &path,
            Some(&json!({"name": "pansy", "url": "v"})),
        )
        .unwrap();
        assert_eq!(again.matches("[[providers]]").count(), 2);
        assert!(again.contains("url = \"v\""));
        assert_eq!(set(Format::Toml, &added, &path, None).unwrap(), mine);

        let on = ["enabled", "[=b]"];
        let more = set(Format::Toml, mine, &on, Some(&json!("b"))).unwrap();
        assert!(more.contains("enabled = [\"a\", \"b\"]"), "{more}");
        assert_eq!(set(Format::Toml, &more, &on, None).unwrap(), mine);
        // a byte order mark stays where it is
        let marked = set(Format::Json, "\u{feff}{\"a\": 1}", &["b"], Some(&json!(2))).unwrap();
        assert_eq!(marked, "\u{feff}{\"a\": 1, \"b\": 2}");
        assert_eq!(get(Format::Json, &marked, &["b"]).unwrap(), Some(json!(2)));
        // a key taken out leaves the comment above it
        let less = set(Format::Toml, mine, &["active"], None).unwrap();
        assert!(less.starts_with("# mine\nenabled = [\"a\"]\n"), "{less}");

        // a list there is none of: tables of their own
        let new = set(
            Format::Toml,
            "",
            &["models", "[id=m]"],
            Some(&json!({"id": "m"})),
        )
        .unwrap();
        assert_eq!(new, "[[models]]\nid = \"m\"\n");
        assert_eq!(
            set(Format::Toml, &new, &["models", "[id=m]"], None).unwrap(),
            ""
        );
        assert!(
            set(
                Format::Yaml,
                "",
                &["a", "[id=m]"],
                Some(&json!({"id": "m"}))
            )
            .is_err()
        );
    }

    #[test]
    fn keeps_an_env_lines_quotes_and_comment() {
        let text = "A=1\r\nGEMINI_API_KEY=\"AIzaSyRealKey\" # my key\r\nB='x y' # b\r\nC=plain # c";
        assert_eq!(
            get(Format::Env, text, &["GEMINI_API_KEY"]).unwrap(),
            Some(json!("AIzaSyRealKey"))
        );
        assert_eq!(get(Format::Env, text, &["B"]).unwrap(), Some(json!("x y")));
        assert_eq!(
            get(Format::Env, text, &["C"]).unwrap(),
            Some(json!("plain"))
        );
        let on = set(
            Format::Env,
            text,
            &["GEMINI_API_KEY"],
            Some(&json!("pansy")),
        )
        .unwrap();
        assert_eq!(
            on,
            "A=1\r\nGEMINI_API_KEY=\"pansy\" # my key\r\nB='x y' # b\r\nC=plain # c"
        );
        let back = set(
            Format::Env,
            &on,
            &["GEMINI_API_KEY"],
            Some(&json!("AIzaSyRealKey")),
        )
        .unwrap();
        assert_eq!(back, text, "put back as it was");
        let c = set(Format::Env, text, &["C"], Some(&json!("new"))).unwrap();
        assert!(c.ends_with("C=new # c"), "{c}");
    }

    #[test]
    fn edits_env_files() {
        let text = "# keys\nexport GEMINI_API_KEY=old\nOTHER=1\n";
        let out = set(
            Format::Env,
            text,
            &["GEMINI_API_KEY"],
            Some(&json!("pansy")),
        )
        .unwrap();
        assert_eq!(out, "# keys\nexport GEMINI_API_KEY=pansy\nOTHER=1\n");
        let out = set(
            Format::Env,
            &out,
            &["GOOGLE_GEMINI_BASE_URL"],
            Some(&json!("http://127.0.0.1:7267")),
        )
        .unwrap();
        assert_eq!(
            get(Format::Env, &out, &["GOOGLE_GEMINI_BASE_URL"]).unwrap(),
            Some(json!("http://127.0.0.1:7267"))
        );
        let out = set(Format::Env, &out, &["OTHER"], None).unwrap();
        assert!(!out.contains("OTHER"));
    }

    #[test]
    fn edits_yaml_scalars() {
        let text = "provider: openai\nmodel:\n  name: gpt-4o\n  temp: 1\nother: x\n";
        assert_eq!(
            get(Format::Yaml, text, &["model", "name"]).unwrap(),
            Some(json!("gpt-4o"))
        );
        let out = set(
            Format::Yaml,
            text,
            &["model", "name"],
            Some(&json!("deepseek/deepseek-chat")),
        )
        .unwrap();
        assert!(out.contains("  name: deepseek/deepseek-chat\n"));
        let out = set(
            Format::Yaml,
            &out,
            &["model", "base_url"],
            Some(&json!("http://127.0.0.1:7267/v1")),
        )
        .unwrap();
        assert_eq!(
            get(Format::Yaml, &out, &["model", "base_url"]).unwrap(),
            Some(json!("http://127.0.0.1:7267/v1"))
        );
        assert!(out.ends_with("other: x\n"));
        let out = set(
            Format::Yaml,
            "",
            &["GOOSE_PROVIDER"],
            Some(&json!("openai")),
        )
        .unwrap();
        assert_eq!(out, "GOOSE_PROVIDER: openai\n");
    }
}
