//! What connecting an agent would write, before anything is written: each
//! file, each key, what is there now and what would be. Nothing is changed
//! and nothing is recorded.

use std::path::Path;

use anyhow::{Context, Result};

use crate::{AGENT_KEY, FileChange, KeyChange, agent, files, find, gateway_url, shown};

/// The keys connecting `id` with `model` would set, file by file. A key
/// that already holds the value it would get is left out: connecting again
/// changes only what differs.
pub fn preview(id: &str, model: &str) -> Result<Vec<FileChange>> {
    let agent = find(id)?;
    let saved = crate::state::State::load()
        .ok()
        .and_then(|s| s.agents.get(id).cloned());
    let base = gateway_url();
    let choices: Vec<String> = pansy_core::manage::choices()
        .unwrap_or_default()
        .into_iter()
        .map(|c| c.id)
        .collect();
    let edits = agent.edits(&agent::Wiring {
        base_url: &base,
        key: AGENT_KEY,
        model: model.trim(),
        small_model: saved.as_ref().and_then(|s| s.small_model.as_deref()),
        choices: &choices,
    });
    let mut out: Vec<FileChange> = Vec::new();
    for e in edits {
        let file = e.file.display().to_string();
        let text = files::read_text(&e.file)?;
        let key: Vec<&str> = e.path.iter().map(String::as_str).collect();
        let now = files::get(e.format, &text, &key).with_context(|| format!("reading {file}"))?;
        if now.as_ref() == Some(&e.value) {
            continue;
        }
        let name = e.path.last().map(String::as_str).unwrap_or_default();
        let change = KeyChange {
            key: e.path.join("."),
            before: now.as_ref().map(|v| shown(name, v)),
            after: Some(shown(name, &e.value)),
        };
        match out.iter_mut().find(|f| f.path == file) {
            Some(f) => f.keys.push(change),
            None => out.push(FileChange {
                created: !Path::new(&file).exists(),
                path: file,
                keys: vec![change],
            }),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::tests::scratch;

    #[test]
    fn says_what_connecting_would_write_and_writes_nothing() {
        let (home, _lock) = scratch();
        let claude = home.path().join(".claude/settings.json");
        fs::create_dir_all(claude.parent().unwrap()).unwrap();
        let mine = "{\n  \"env\": {\"ANTHROPIC_MODEL\": \"opus\", \"ANTHROPIC_AUTH_TOKEN\": \"sk-ant-my-own-secret-key\"}\n}\n";
        fs::write(&claude, mine).unwrap();

        let changes = preview("claude-code", "deepseek/deepseek-chat").unwrap();
        assert_eq!(changes.len(), 1);
        let file = &changes[0];
        assert_eq!(file.path, claude.display().to_string());
        assert!(!file.created);
        let key = |name: &str| file.keys.iter().find(|k| k.key == name).unwrap();
        let model = key("env.ANTHROPIC_MODEL");
        assert_eq!(model.before.as_deref(), Some("opus"));
        assert_eq!(model.after.as_deref(), Some("deepseek/deepseek-chat"));
        let url = key("env.ANTHROPIC_BASE_URL");
        assert_eq!(
            (url.before.as_deref(), url.after.as_deref()),
            (None, Some("http://127.0.0.1:7267"))
        );
        // The user's own key is shown masked; pansy's is no secret.
        let token = key("env.ANTHROPIC_AUTH_TOKEN");
        assert!(!token.before.as_deref().unwrap().contains("my-own-secret"));
        assert_eq!(token.after.as_deref(), Some("pansy"));

        // Nothing was written or recorded.
        assert_eq!(fs::read_to_string(&claude).unwrap(), mine);
        assert!(!home.path().join(".config/pansy/agents.json").exists());

        // A file pansy would make.
        let gemini = preview("gemini-cli", "deepseek/deepseek-chat").unwrap();
        assert!(gemini.iter().all(|f| f.created));
        assert!(!home.path().join(".gemini").exists());

        // Connected: the same model again changes nothing, another only its keys.
        crate::connect("claude-code", "deepseek/deepseek-chat").unwrap();
        assert!(
            preview("claude-code", "deepseek/deepseek-chat")
                .unwrap()
                .is_empty()
        );
        let other = preview("claude-code", "moonshot/kimi-k2").unwrap();
        assert!(
            other[0].keys.iter().all(|k| k.key.contains("MODEL")),
            "{other:?}"
        );
    }
}
