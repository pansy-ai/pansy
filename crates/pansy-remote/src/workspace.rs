//! Where a new thread works: the project as it is, a new branch of it, or a
//! worktree of its own; and the worktree's branch named after the thread
//! once its first turn is done.

use std::path::Path;

use anyhow::Result;

use crate::git;
use crate::store::Thread;

/// The workdir, branch, whether it is a worktree, and its base, for
/// `kind`: `current`, `branch` or `worktree`.
pub fn prepare(
    folder: &Path,
    kind: &str,
    base: Option<&str>,
) -> Result<(String, Option<String>, bool, Option<String>)> {
    let here = || folder.to_string_lossy().into_owned();
    match kind {
        "" | "current" => {
            let branch = git::is_repo(folder)
                .then(|| git::status(folder).ok().map(|s| s.branch))
                .flatten();
            Ok((here(), branch, false, None))
        }
        "branch" => {
            let branch = git::new_branch(folder, base)?;
            Ok((here(), Some(branch), false, base.map(str::to_owned)))
        }
        "worktree" => {
            let repo = git::toplevel(folder)?;
            let base = match base {
                Some(b) if !b.is_empty() => b.to_owned(),
                _ => git::default_branch(&repo)?,
            };
            let w = git::create_worktree(&repo, &base)?;
            Ok((
                w.path.to_string_lossy().into_owned(),
                Some(w.branch),
                true,
                Some(w.base),
            ))
        }
        other => Err(anyhow::Error::new(crate::hub::Coded(
            "bad_request",
            format!("no workspace {other}: current, branch or worktree"),
        ))),
    }
}

/// The settings' writer model, else the thread's.
pub fn writer(t: &Thread) -> String {
    crate::config::load()
        .ok()
        .map(|s| s.writer_model)
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| t.model.clone())
}

/// The gateway's address and the key that counts requests under the
/// thread's project.
pub fn gateway(t: &Thread) -> (String, String) {
    let base = pansy_core::config::load_settings()
        .map(|s| s.base_url())
        .unwrap_or_else(|_| "http://127.0.0.1:7267".into());
    (base, pansy_core::project::key_for(Path::new(&t.folder)))
}

/// Renames a worktree thread's `pansy/<hex>` branch after its title.
pub async fn branch_name(t: &Thread) -> Option<String> {
    let old = t.branch.clone()?;
    let (base, key) = gateway(t);
    let (slug, _) = git::generate(git::Writing::Branch, "", &t.title, &base, &key, &writer(t))
        .await
        .ok()?;
    let dir = t.workdir.clone();
    tokio::task::spawn_blocking(move || git::rename_branch(Path::new(&dir), &old, &slug))
        .await
        .ok()?
        .ok()
}
