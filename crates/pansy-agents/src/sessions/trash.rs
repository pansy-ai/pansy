//! Deleted sessions. Delete moves a session's files and folders into
//! pansy's trash, `trash/sessions/<agent>/<id>-<when>/files/`, beside a note
//! of where each came from (`session.json`); Restore moves them back, and
//! only Delete forever or Empty the trash erases them. A session written to
//! in the last minute may still be running, and is not deleted.
//!
//! The agents' own indexes are left as they are: Codex's
//! `session_index.jsonl` and Claude Code's `history.jsonl` are written by
//! the agent while it runs, and a name or a prompt kept for a session that
//! is gone does nothing.

use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use pansy_core::{fsutil, paths};
use serde::{Deserialize, Serialize};

/// A session in the trash.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Trashed {
    /// Its folder in the trash: `<agent>/<name>`.
    pub key: String,
    pub agent: String,
    pub id: String,
    pub title: String,
    pub cwd: String,
    pub last: Option<DateTime<Utc>>,
    pub deleted: DateTime<Utc>,
    pub size: u64,
    pub items: Vec<Moved>,
}

/// A file or folder of a session in the trash: where it was, and its name
/// in the trash's `files/`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Moved {
    pub from: String,
    pub name: String,
}

/// The note beside a session's files in the trash.
const NOTE: &str = "session.json";

/// Where deleted sessions are kept.
pub fn dir() -> PathBuf {
    paths::home().join("trash").join("sessions")
}

/// Moves the session `id` of `agent` into the trash.
pub fn delete(agent: &str, id: &str) -> Result<Trashed> {
    let session = super::find(agent, id).ok_or_else(|| anyhow!("no {agent} session {id}"))?;
    if !session.deletable {
        let name = crate::agent::find(agent).map_or(agent, |a| a.name);
        bail!(
            "{name} keeps its sessions in its own database, which it keeps open: pansy lists them, and doesn't delete them"
        );
    }
    if session.active {
        bail!(
            "“{}” was written to in the last minute; it may still be running",
            session.title
        );
    }
    let deleted = Utc::now();
    let name = format!("{id}-{}", deleted.timestamp_millis());
    let folder = dir().join(agent).join(&name);
    let files = folder.join("files");
    fs::create_dir_all(&files).with_context(|| format!("making {}", files.display()))?;
    private(&paths::home().join("trash"));
    let items: Vec<Moved> = session
        .files
        .iter()
        .enumerate()
        .map(|(i, from)| Moved {
            from: from.display().to_string(),
            name: format!(
                "{i}-{}",
                from.file_name().unwrap_or_default().to_string_lossy()
            ),
        })
        .collect();
    let trashed = Trashed {
        key: format!("{agent}/{name}"),
        agent: agent.to_owned(),
        id: id.to_owned(),
        title: session.title,
        cwd: session.cwd,
        last: session.last,
        deleted,
        size: session.size,
        items,
    };
    // the note first: files in the trash are never without one
    if let Err(e) = fsutil::write_json(&folder.join(NOTE), &trashed, true) {
        let _ = fs::remove_dir_all(&folder);
        return Err(e);
    }
    for (i, item) in trashed.items.iter().enumerate() {
        if let Err(e) = move_path(Path::new(&item.from), &files.join(&item.name)) {
            for back in &trashed.items[..i] {
                let _ = move_path(&files.join(&back.name), Path::new(&back.from));
            }
            let _ = fs::remove_dir_all(&folder);
            return Err(e.context(format!("moving {} to the trash", item.from)));
        }
    }
    Ok(trashed)
}

/// The sessions in the trash, the latest deleted first.
pub fn list() -> Vec<Trashed> {
    let mut out: Vec<Trashed> = fs::read_dir(dir())
        .into_iter()
        .flatten()
        .flatten()
        .flat_map(|agent| fs::read_dir(agent.path()).into_iter().flatten().flatten())
        .filter_map(|folder| {
            let bytes = fs::read(folder.path().join(NOTE)).ok()?;
            serde_json::from_slice::<Trashed>(&bytes).ok()
        })
        .collect();
    out.sort_by_key(|t| std::cmp::Reverse(t.deleted));
    out
}

/// Moves a session in the trash back where its files were. Nothing is
/// moved when any of them is there again.
pub fn restore(key: &str) -> Result<Trashed> {
    let folder = folder_of(key)?;
    let bytes = fs::read(folder.join(NOTE))?;
    let trashed: Trashed = serde_json::from_slice(&bytes)?;
    for item in &trashed.items {
        if !plain(&item.name) {
            bail!("the trash's note names {} oddly", item.name);
        }
        if fs::symlink_metadata(&item.from).is_ok() {
            bail!("{} is there again; move it away first", item.from);
        }
    }
    for item in &trashed.items {
        let to = Path::new(&item.from);
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        move_path(&folder.join("files").join(&item.name), to)
            .with_context(|| format!("putting back {}", item.from))?;
    }
    fs::remove_dir_all(&folder)?;
    Ok(trashed)
}

/// Erases a session in the trash for good.
pub fn purge(key: &str) -> Result<()> {
    let folder = folder_of(key)?;
    fs::remove_dir_all(&folder).with_context(|| format!("erasing {}", folder.display()))
}

/// Erases every session in the trash; how many there were.
pub fn empty() -> Result<usize> {
    let all = list();
    for t in &all {
        purge(&t.key)?;
    }
    Ok(all.len())
}

/// A name of one part, with no way out of the folder it is in.
fn plain(name: &str) -> bool {
    let mut parts = Path::new(name).components();
    matches!(parts.next(), Some(Component::Normal(_)))
        && parts.next().is_none()
        && !name.contains(['/', '\\'])
}

/// The folder of a session in the trash, by its key: refused for a key
/// that isn't one, or a folder that is a link (it could lead out of the
/// trash).
fn folder_of(key: &str) -> Result<PathBuf> {
    let no = || anyhow!("nothing in the trash is called {key}");
    let (agent, name) = key.split_once('/').ok_or_else(no)?;
    if !plain(agent) || !plain(name) {
        return Err(no());
    }
    let agent_dir = dir().join(agent);
    let folder = agent_dir.join(name);
    for d in [&agent_dir, &folder] {
        match fs::symlink_metadata(d) {
            Ok(meta) if meta.is_dir() => {}
            _ => return Err(no()),
        }
    }
    match fs::symlink_metadata(folder.join(NOTE)) {
        Ok(meta) if meta.is_file() => Ok(folder),
        _ => Err(no()),
    }
}

/// Moves a file or folder; across disks, a copy and a removal.
fn move_path(from: &Path, to: &Path) -> Result<()> {
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::CrossesDevices => {
            copy_all(from, to)?;
            if fs::symlink_metadata(from)?.is_dir() {
                fs::remove_dir_all(from)?;
            } else {
                fs::remove_file(from)?;
            }
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

fn copy_all(from: &Path, to: &Path) -> Result<()> {
    if fs::symlink_metadata(from)?.is_dir() {
        fs::create_dir_all(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            copy_all(&entry.path(), &to.join(entry.file_name()))?;
        }
    } else {
        fs::copy(from, to)?;
    }
    Ok(())
}

/// Only the user may look into the trash: it holds their conversations.
fn private(dir: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    }
    #[cfg(not(unix))]
    let _ = dir;
}
