//! A thread's repository, as the phone works it (phone-protocol.md, Git):
//! its status and diffs, the worktrees and branches pansy makes, the
//! checkpoints after each turn, commit, push and the pull request; and the
//! messages for them, written by a model through the gateway.
//!
//! Everything goes through the `git` program, in the thread's folder. The
//! user's own checkout is read: a diff of what is not committed is taken
//! from a snapshot made with an index of pansy's, never the user's.
//! Checkpoints are commits no branch points to, under `refs/pansy/`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use pansy_core::paths;
use serde::Serialize;
use serde_json::{Value, json};

/// Git's tree with nothing in it: what an unborn branch is diffed from.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
/// A diff longer than this many lines is cut short (`truncated`).
const MAX_LINES: usize = 20_000;
/// A file whose diff is longer is listed without its lines.
const MAX_FILE_LINES: usize = 4_000;

/// `GitStatus` of the protocol (`pr` is the thread's, added by the server).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub branch: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub default_branch: String,
    pub on_default: bool,
    /// The folder is a linked worktree.
    pub worktree: bool,
    pub files: Vec<StatusFile>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusFile {
    pub path: String,
    /// `added`, `modified`, `deleted`, `renamed` or `untracked`.
    pub status: String,
    pub add: u32,
    pub del: u32,
}

/// `Diff` of the protocol.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Diff {
    pub files: Vec<File>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct File {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    /// `added`, `modified`, `deleted` or `renamed`.
    pub status: String,
    pub add: u32,
    pub del: u32,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub binary: bool,
    pub hunks: Vec<Hunk>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Hunk {
    pub header: String,
    pub lines: Vec<Line>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Line {
    /// `+`, `-` or a space.
    pub kind: char,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new: Option<u32>,
    pub text: String,
}

/// What a diff is of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// The folder as it is (untracked files too) against `HEAD`.
    Uncommitted,
    /// The folder as it is against where it left this branch.
    Base(String),
    /// Between two commits (checkpoints).
    Range { from: String, to: String },
}

/// A worktree pansy made.
#[derive(Debug, Clone)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: String,
    pub base: String,
}

/// A pull request on GitHub.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pr {
    pub number: u64,
    pub url: String,
    /// `open`, `closed` or `merged`.
    pub state: String,
}

/// What a model writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Writing {
    Commit,
    PullRequest,
    Branch,
}

// --- Running git ----------------------------------------------------------------

fn git(dir: &Path) -> Command {
    let mut c = Command::new("git");
    c.arg("-C")
        .arg(dir)
        .args(["-c", "core.quotePath=false", "-c", "color.ui=false"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C");
    c
}

fn output(mut c: Command) -> Result<String> {
    let out = c.output().context("running git")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err.trim();
        bail!(
            "{}",
            if err.is_empty() {
                format!("git exited with {}", out.status)
            } else {
                err.to_owned()
            }
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn run<I, S>(dir: &Path, args: I) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let mut c = git(dir);
    c.args(args);
    output(c)
}

fn ok<I, S>(dir: &Path, args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let mut c = git(dir);
    c.args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    c.status().is_ok_and(|s| s.success())
}

fn hex(n: usize) -> String {
    let mut bytes = vec![0u8; n.div_ceil(2)];
    let _ = getrandom::getrandom(&mut bytes);
    let s: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    s[..n].to_owned()
}

// --- The repository ---------------------------------------------------------------

pub fn is_repo(dir: &Path) -> bool {
    run(dir, ["rev-parse", "--is-inside-work-tree"]).is_ok_and(|o| o.trim() == "true")
}

/// The checkout's top folder.
pub fn toplevel(dir: &Path) -> Result<PathBuf> {
    Ok(PathBuf::from(
        run(dir, ["rev-parse", "--show-toplevel"])?.trim(),
    ))
}

fn has_head(dir: &Path) -> bool {
    ok(dir, ["rev-parse", "--verify", "--quiet", "HEAD"])
}

fn exists(dir: &Path, rev: &str) -> bool {
    ok(
        dir,
        [
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{rev}^{{commit}}"),
        ],
    )
}

/// The branch checked out, else `HEAD` (detached).
fn current_branch(dir: &Path) -> Result<String> {
    let b = run(dir, ["symbolic-ref", "--quiet", "--short", "HEAD"])
        .or_else(|_| run(dir, ["rev-parse", "--abbrev-ref", "HEAD"]))?;
    Ok(b.trim().to_owned())
}

/// The branch pull requests go to: `origin`'s, else `main` or `master`,
/// else the one checked out.
pub fn default_branch(dir: &Path) -> Result<String> {
    if let Ok(b) = run(
        dir,
        [
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
    ) {
        let b = b.trim();
        return Ok(b.strip_prefix("origin/").unwrap_or(b).to_owned());
    }
    for b in ["main", "master"] {
        if exists(dir, &format!("refs/heads/{b}"))
            || exists(dir, &format!("refs/remotes/origin/{b}"))
        {
            return Ok(b.to_owned());
        }
    }
    current_branch(dir)
}

/// Whether `dir` is a worktree linked to another checkout.
fn linked(dir: &Path) -> bool {
    let (Ok(a), Ok(b)) = (
        run(dir, ["rev-parse", "--absolute-git-dir"]),
        run(
            dir,
            ["rev-parse", "--path-format=absolute", "--git-common-dir"],
        ),
    ) else {
        return false;
    };
    Path::new(a.trim()) != Path::new(b.trim())
}

// --- Branches and worktrees -------------------------------------------------------

/// A branch of `base` (else of what is checked out), switched to in the
/// folder itself: `pansy/<hex>`, named after the thread later.
pub fn new_branch(dir: &Path, base: Option<&str>) -> Result<String> {
    let branch = format!("pansy/{}", hex(6));
    let mut args = vec!["switch".to_owned(), "-c".to_owned(), branch.clone()];
    if let Some(b) = base.filter(|b| !b.is_empty()) {
        args.push(start_point(dir, b));
    }
    run(dir, &args)?;
    Ok(branch)
}

/// `base` as git can start from it: the local branch, else `origin`'s.
fn start_point(dir: &Path, base: &str) -> String {
    if !exists(dir, base) && exists(dir, &format!("origin/{base}")) {
        format!("origin/{base}")
    } else {
        base.to_owned()
    }
}

/// Where pansy keeps the worktrees it makes.
pub fn worktrees() -> PathBuf {
    paths::home().join("worktrees")
}

/// A worktree of `repo` on a new branch `pansy/<hex>` of `base`, under
/// pansy's folder.
pub fn create_worktree(repo: &Path, base: &str) -> Result<Worktree> {
    create_worktree_in(repo, base, &worktrees())
}

fn create_worktree_in(repo: &Path, base: &str, root: &Path) -> Result<Worktree> {
    let id = hex(6);
    let name = repo
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".into());
    let path = root.join(&name).join(&id);
    std::fs::create_dir_all(path.parent().unwrap_or(root))?;
    let branch = format!("pansy/{id}");
    let mut args: Vec<String> = vec![
        "worktree".into(),
        "add".into(),
        "-b".into(),
        branch.clone(),
        path.to_string_lossy().into_owned(),
    ];
    if has_head(repo) {
        args.push(start_point(repo, base));
    }
    run(repo, &args)?;
    Ok(Worktree {
        path,
        branch,
        base: base.to_owned(),
    })
}

/// Removes a worktree pansy made, and its branch when nothing new is on it.
pub fn remove_worktree(repo: &Path, path: &Path, branch: &str) -> Result<()> {
    run(
        repo,
        [
            "worktree".as_ref(),
            "remove".as_ref(),
            "--force".as_ref(),
            path.as_os_str(),
        ],
    )?;
    let _ = run(repo, ["branch", "-d", branch]);
    Ok(())
}

/// A branch name of a model's words: lowercase, letters, digits and
/// hyphens, at most 48 characters.
pub fn slug(text: &str) -> String {
    let text = text.trim().trim_start_matches("pansy/");
    let mut out = String::new();
    for c in text.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= 48 {
            break;
        }
    }
    out.trim_end_matches('-').to_owned()
}

/// Renames `old` to `pansy/<slug>` (with `-2`, `-3`… when taken): the new
/// name.
pub fn rename_branch(dir: &Path, old: &str, slug_text: &str) -> Result<String> {
    let s = slug(slug_text);
    if s.is_empty() {
        bail!("no name for the branch in “{slug_text}”");
    }
    let mut name = format!("pansy/{s}");
    let mut n = 2;
    while exists(dir, &format!("refs/heads/{name}")) {
        name = format!("pansy/{s}-{n}");
        n += 1;
    }
    run(dir, ["branch", "-m", old, &name])?;
    Ok(name)
}

// --- Snapshots and checkpoints ----------------------------------------------------

/// A tree of the folder as it is, untracked files too (not those ignored),
/// written with an index of pansy's: the user's index is left as it was.
fn snapshot(dir: &Path) -> Result<String> {
    let git_dir = PathBuf::from(run(dir, ["rev-parse", "--absolute-git-dir"])?.trim());
    let index = git_dir.join(format!("pansy-index-{}", hex(8)));
    let real = git_dir.join("index");
    if real.exists() {
        std::fs::copy(&real, &index)?;
    }
    let with = |args: &[&str]| {
        let mut c = git(dir);
        c.env("GIT_INDEX_FILE", &index).args(args);
        output(c)
    };
    let tree = (|| {
        if !real.exists() && has_head(dir) {
            with(&["read-tree", "HEAD"])?;
        }
        with(&["add", "--all", "."])?;
        with(&["write-tree"])
    })();
    let _ = std::fs::remove_file(&index);
    Ok(tree?.trim().to_owned())
}

/// The ref of a thread's checkpoint `n`: 0 is its worktree as made, `n`
/// the folder after its `n`th turn.
pub fn checkpoint_ref(thread: &str, n: u32) -> String {
    format!("refs/pansy/checkpoints/{thread}/{n}")
}

/// Records the folder as it is as the thread's checkpoint `n`: a commit
/// no branch points to. Its id.
pub fn checkpoint(dir: &Path, thread: &str, n: u32) -> Result<String> {
    let tree = snapshot(dir)?;
    let mut c = git(dir);
    c.args(["commit-tree", &tree, "-m"])
        .arg(format!("pansy checkpoint {thread} {n}"));
    if has_head(dir) {
        c.args(["-p", "HEAD"]);
    }
    for (k, v) in [
        ("GIT_AUTHOR_NAME", "pansy"),
        ("GIT_AUTHOR_EMAIL", "pansy@localhost"),
        ("GIT_COMMITTER_NAME", "pansy"),
        ("GIT_COMMITTER_EMAIL", "pansy@localhost"),
    ] {
        c.env(k, v);
    }
    let commit = output(c)?.trim().to_owned();
    run(dir, ["update-ref", &checkpoint_ref(thread, n), &commit])?;
    Ok(commit)
}

// --- Diffs ------------------------------------------------------------------------

/// What changed in `scope`.
pub fn diff(dir: &Path, scope: &Scope) -> Result<Diff> {
    let head = || {
        if has_head(dir) {
            "HEAD".to_owned()
        } else {
            EMPTY_TREE.to_owned()
        }
    };
    let (from, to) = match scope {
        Scope::Uncommitted => (head(), snapshot(dir)?),
        Scope::Base(base) => {
            let start = start_point(dir, base);
            let from = run(dir, ["merge-base", &start, "HEAD"])
                .map(|s| s.trim().to_owned())
                .unwrap_or_else(|_| head());
            (from, snapshot(dir)?)
        }
        Scope::Range { from, to } => (from.clone(), to.clone()),
    };
    let patch = run(
        dir,
        [
            "diff",
            "--no-ext-diff",
            "--find-renames",
            "--unified=3",
            &from,
            &to,
        ],
    )?;
    Ok(parse_diff(&patch))
}

fn hunk_start(part: &str) -> u32 {
    part[1..]
        .split(',')
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

/// A unified diff, as the protocol's `Diff`.
pub fn parse_diff(patch: &str) -> Diff {
    let mut out = Diff::default();
    let mut total = 0usize;
    let (mut old_no, mut new_no) = (0u32, 0u32);
    for line in patch.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            if total > MAX_LINES {
                out.truncated = true;
                break;
            }
            // `a/x b/x`: the path from `+++`/`---` replaces it when there is one.
            let path = rest
                .split_once(" b/")
                .map(|(_, b)| b.to_owned())
                .unwrap_or_else(|| rest.to_owned());
            out.files.push(File {
                path,
                old_path: None,
                status: "modified".into(),
                add: 0,
                del: 0,
                binary: false,
                hunks: Vec::new(),
            });
            continue;
        }
        let Some(file) = out.files.last_mut() else {
            continue;
        };
        if line.starts_with("new file mode") {
            file.status = "added".into();
        } else if line.starts_with("deleted file mode") {
            file.status = "deleted".into();
        } else if let Some(p) = line.strip_prefix("rename from ") {
            file.status = "renamed".into();
            file.old_path = Some(p.to_owned());
        } else if let Some(p) = line.strip_prefix("rename to ") {
            file.path = p.to_owned();
        } else if line.starts_with("Binary files ") || line == "GIT binary patch" {
            file.binary = true;
        } else if let Some(p) = line.strip_prefix("+++ ") {
            if let Some(p) = p.strip_prefix("b/") {
                file.path = p.to_owned();
            }
        } else if let Some(p) = line.strip_prefix("--- ") {
            if file.status == "deleted"
                && let Some(p) = p.strip_prefix("a/")
            {
                file.path = p.to_owned();
            }
        } else if line.starts_with("@@ ") {
            let mut parts = line.split(' ');
            parts.next();
            old_no = parts.next().map(hunk_start).unwrap_or(0);
            new_no = parts.next().map(hunk_start).unwrap_or(0);
            file.hunks.push(Hunk {
                header: line.to_owned(),
                lines: Vec::new(),
            });
        } else if let Some(kind @ ('+' | '-' | ' ')) = line.chars().next() {
            let Some(hunk) = file.hunks.last_mut() else {
                continue;
            };
            let text = line[1..].to_owned();
            let (old, new) = match kind {
                '+' => {
                    file.add += 1;
                    new_no += 1;
                    (None, Some(new_no - 1))
                }
                '-' => {
                    file.del += 1;
                    old_no += 1;
                    (Some(old_no - 1), None)
                }
                _ => {
                    old_no += 1;
                    new_no += 1;
                    (Some(old_no - 1), Some(new_no - 1))
                }
            };
            total += 1;
            hunk.lines.push(Line {
                kind,
                old,
                new,
                text,
            });
        }
    }
    for f in &mut out.files {
        let lines: usize = f.hunks.iter().map(|h| h.lines.len()).sum();
        if lines > MAX_FILE_LINES {
            f.hunks.clear();
            out.truncated = true;
        }
    }
    out
}

// --- Status -----------------------------------------------------------------------

/// The branch, how far it is from its upstream, and the files changed.
pub fn status(dir: &Path) -> Result<Status> {
    let text = run(
        dir,
        [
            "status",
            "--porcelain=v2",
            "--branch",
            "--untracked-files=all",
        ],
    )?;
    let mut st = Status {
        branch: String::new(),
        upstream: None,
        ahead: 0,
        behind: 0,
        default_branch: String::new(),
        on_default: false,
        worktree: linked(dir),
        files: Vec::new(),
    };
    for line in text.lines() {
        if let Some(h) = line.strip_prefix("# branch.head ") {
            st.branch = if h == "(detached)" {
                "HEAD".into()
            } else {
                h.to_owned()
            };
        } else if let Some(u) = line.strip_prefix("# branch.upstream ") {
            st.upstream = Some(u.to_owned());
        } else if let Some(ab) = line.strip_prefix("# branch.ab ") {
            for part in ab.split(' ') {
                if let Some(n) = part.strip_prefix('+') {
                    st.ahead = n.parse().unwrap_or(0);
                } else if let Some(n) = part.strip_prefix('-') {
                    st.behind = n.parse().unwrap_or(0);
                }
            }
        } else if let Some(p) = line.strip_prefix("? ") {
            st.files.push(StatusFile {
                path: p.to_owned(),
                status: "untracked".into(),
                add: 0,
                del: 0,
            });
        } else if line.starts_with("1 ") || line.starts_with("2 ") || line.starts_with("u ") {
            let renamed = line.starts_with("2 ");
            let fields = if renamed {
                10
            } else if line.starts_with("u ") {
                11
            } else {
                9
            };
            let parts: Vec<&str> = line.splitn(fields, ' ').collect();
            let xy = parts.get(1).copied().unwrap_or("..");
            let path = parts.last().copied().unwrap_or_default();
            let path = path.split('\t').next().unwrap_or(path).to_owned();
            let status = if renamed {
                "renamed"
            } else if xy.contains('D') {
                "deleted"
            } else if xy.contains('A') {
                "added"
            } else {
                "modified"
            };
            st.files.push(StatusFile {
                path,
                status: status.into(),
                add: 0,
                del: 0,
            });
        }
    }
    // Lines added and removed, the index and the folder together.
    let base = if has_head(dir) { "HEAD" } else { EMPTY_TREE };
    let mut counts: HashMap<String, (u32, u32)> = HashMap::new();
    if let Ok(num) = run(dir, ["diff", "--numstat", "--find-renames", base]) {
        for line in num.lines() {
            let mut p = line.splitn(3, '\t');
            let (Some(a), Some(d), Some(path)) = (p.next(), p.next(), p.next()) else {
                continue;
            };
            // `old => new` and `dir/{old => new}` for a rename.
            let path = if let Some((head, tail)) = path.split_once('{') {
                let (inner, after) = tail.split_once('}').unwrap_or((tail, ""));
                let new = inner.split(" => ").last().unwrap_or(inner);
                format!("{head}{new}{after}").replace("//", "/")
            } else {
                path.split(" => ").last().unwrap_or(path).to_owned()
            };
            counts.insert(path, (a.parse().unwrap_or(0), d.parse().unwrap_or(0)));
        }
    }
    let top = toplevel(dir).unwrap_or_else(|_| dir.to_path_buf());
    for f in &mut st.files {
        if let Some((a, d)) = counts.get(&f.path) {
            (f.add, f.del) = (*a, *d);
        } else if f.status == "untracked" {
            f.add = std::fs::read(top.join(&f.path))
                .ok()
                .filter(|b| !b.contains(&0))
                .map(|b| bytecount_lines(&b))
                .unwrap_or(0);
        }
    }
    st.default_branch = default_branch(dir).unwrap_or_default();
    st.on_default = !st.branch.is_empty() && st.branch == st.default_branch;
    Ok(st)
}

fn bytecount_lines(b: &[u8]) -> u32 {
    let n = b.iter().filter(|c| **c == b'\n').count();
    (n + usize::from(!b.is_empty() && !b.ends_with(b"\n"))) as u32
}

// --- Commit and push --------------------------------------------------------------

/// Commits `files` (every change when none are given) with `message`: the
/// commit's id.
pub fn commit(dir: &Path, files: Option<&[String]>, message: &str) -> Result<String> {
    match files.filter(|f| !f.is_empty()) {
        Some(files) => {
            let mut args = vec!["add".to_owned(), "--all".to_owned(), "--".to_owned()];
            args.extend(files.iter().cloned());
            run(dir, &args)?;
        }
        None => {
            run(dir, ["add", "--all"])?;
        }
    }
    if ok(dir, ["diff", "--cached", "--quiet"]) && has_head(dir) {
        bail!("nothing to commit");
    }
    let mut c = git(dir);
    c.args(["commit", "--quiet", "-F", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = c.spawn().context("running git commit")?;
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin.write_all(message.as_bytes())?;
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let said = String::from_utf8_lossy(&out.stdout);
        bail!(
            "{}",
            if err.trim().is_empty() {
                said.trim().to_owned()
            } else {
                err.trim().to_owned()
            }
        );
    }
    Ok(run(dir, ["rev-parse", "HEAD"])?.trim().to_owned())
}

/// Pushes the branch, setting its upstream on `origin` (else the only
/// remote) the first time: what git said last.
pub fn push(dir: &Path) -> Result<String> {
    let has_upstream = ok(
        dir,
        ["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
    );
    let mut c = git(dir);
    if has_upstream {
        c.arg("push");
    } else {
        let remotes = run(dir, ["remote"])?;
        let remotes: Vec<&str> = remotes.lines().collect();
        let remote = if remotes.contains(&"origin") {
            "origin"
        } else {
            match remotes.as_slice() {
                [one] => one,
                [] => bail!("this repository has no remote to push to"),
                _ => bail!("no origin among the remotes: {}", remotes.join(", ")),
            }
        };
        c.args(["push", "--set-upstream", remote, "HEAD"]);
    }
    let out = c.output().context("running git push")?;
    let said = String::from_utf8_lossy(&out.stderr).trim().to_owned();
    if !out.status.success() {
        bail!("{said}");
    }
    Ok(said
        .lines()
        .rfind(|l| !l.trim().is_empty())
        .unwrap_or("pushed")
        .trim()
        .to_owned())
}

// --- GitHub -----------------------------------------------------------------------

/// The GitHub owner and repository `origin` is.
pub fn remote_repo(dir: &Path) -> Result<(String, String)> {
    let url = run(dir, ["remote", "get-url", "origin"])?;
    parse_github(url.trim()).ok_or_else(|| anyhow!("origin is not on GitHub: {}", url.trim()))
}

fn parse_github(url: &str) -> Option<(String, String)> {
    let rest = url
        .strip_prefix("git@github.com:")
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))
        .or_else(|| url.strip_prefix("https://github.com/"))
        .or_else(|| url.strip_prefix("http://github.com/"))
        .or_else(|| {
            // https://user@github.com/o/r
            url.strip_prefix("https://")
                .and_then(|u| u.split_once("@github.com/"))
                .map(|(_, r)| r)
        })?;
    let rest = rest.trim_end_matches('/').trim_end_matches(".git");
    let (owner, repo) = rest.split_once('/')?;
    (!owner.is_empty() && !repo.is_empty() && !repo.contains('/'))
        .then(|| (owner.to_owned(), repo.to_owned()))
}

/// A token for GitHub's API: `GH_TOKEN`, `GITHUB_TOKEN`, else what the
/// GitHub CLI is signed in with.
pub fn github_token() -> Option<String> {
    for k in ["GH_TOKEN", "GITHUB_TOKEN"] {
        if let Ok(t) = std::env::var(k)
            && !t.trim().is_empty()
        {
            return Some(t.trim().to_owned());
        }
    }
    let out = Command::new("gh").args(["auth", "token"]).output().ok()?;
    let t = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (out.status.success() && !t.is_empty()).then_some(t)
}

fn github() -> Result<(reqwest::Client, String)> {
    let token = github_token().ok_or_else(|| {
        anyhow!("no GitHub sign-in: run `gh auth login` on this computer, or set GH_TOKEN")
    })?;
    let client = reqwest::Client::builder()
        .user_agent("pansy")
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    Ok((client, token))
}

fn pr_of(v: &Value) -> Option<Pr> {
    let state = if v["merged_at"].is_string() || v["merged"].as_bool() == Some(true) {
        "merged".to_owned()
    } else {
        v["state"].as_str()?.to_owned()
    };
    Some(Pr {
        number: v["number"].as_u64()?,
        url: v["html_url"].as_str()?.to_owned(),
        state,
    })
}

/// Opens a pull request of the branch checked out into `base`; the one
/// already open for it when there is one.
pub async fn create_pr(dir: &Path, title: &str, body: &str, base: &str, api: &str) -> Result<Pr> {
    let (owner, repo) = remote_repo(dir)?;
    let branch = current_branch(dir)?;
    if branch == "HEAD" {
        bail!("no branch is checked out");
    }
    let (client, token) = github()?;
    let res = client
        .post(format!("{api}/repos/{owner}/{repo}/pulls"))
        .bearer_auth(&token)
        .header("Accept", "application/vnd.github+json")
        .json(&json!({"title": title, "body": body, "head": branch, "base": base}))
        .send()
        .await?;
    let status = res.status();
    let v: Value = res.json().await.unwrap_or(Value::Null);
    if status.is_success() {
        return pr_of(&v).ok_or_else(|| anyhow!("GitHub answered without a pull request"));
    }
    if status.as_u16() == 422
        && v.to_string().contains("already exists")
        && let Some(pr) = pr_for_branch(dir, api).await?
    {
        return Ok(pr);
    }
    let said = v["errors"][0]["message"]
        .as_str()
        .or(v["message"].as_str())
        .unwrap_or("no reason given");
    bail!("GitHub refused the pull request ({status}): {said}")
}

/// The newest pull request of the branch checked out, open or not.
pub async fn pr_for_branch(dir: &Path, api: &str) -> Result<Option<Pr>> {
    let (owner, repo) = remote_repo(dir)?;
    let branch = current_branch(dir)?;
    let (client, token) = github()?;
    let v: Value = client
        .get(format!("{api}/repos/{owner}/{repo}/pulls"))
        .query(&[
            ("head", format!("{owner}:{branch}")),
            ("state", "all".into()),
        ])
        .bearer_auth(&token)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(v.as_array().and_then(|a| a.first()).and_then(pr_of))
}

// --- Writing ----------------------------------------------------------------------

fn instructions(kind: Writing) -> &'static str {
    match kind {
        Writing::Commit => {
            "Write a git commit message for the changes below. The first line is a summary in the imperative mood, under 72 characters. Add a blank line and a short body only when the summary is not enough. Reply with the message alone, no code fences."
        }
        Writing::PullRequest => {
            "Write a pull request for the changes below. The first line is its title, under 72 characters, no prefix. Then a blank line and a description in Markdown: what changed and why, as a few short points. Reply with the pull request alone, no code fences."
        }
        Writing::Branch => {
            "Name a git branch for the task below: two to five lowercase English words joined by hyphens, no prefix. Reply with the name alone."
        }
    }
}

/// Splits a model's reply into a title and a body.
fn split_reply(kind: Writing, reply: &str) -> (String, String) {
    let text = reply.trim();
    let text = text
        .strip_prefix("```")
        .map(|t| t.split_once('\n').map_or("", |(_, rest)| rest))
        .map(|t| t.trim_end().trim_end_matches("```"))
        .unwrap_or(text)
        .trim();
    let (title, body) = text.split_once('\n').unwrap_or((text, ""));
    let title = title
        .trim()
        .trim_start_matches('#')
        .trim()
        .trim_matches('"');
    match kind {
        Writing::Branch => (slug(title), String::new()),
        _ => (title.to_owned(), body.trim().to_owned()),
    }
}

/// What a model writes of `kind` for the changes in `diff` and the task
/// `context` (the thread's title), through the gateway at `base_url`: a
/// title (a commit's summary, a branch's name) and a body.
pub async fn generate(
    kind: Writing,
    diff: &str,
    context: &str,
    base_url: &str,
    key: &str,
    model: &str,
) -> Result<(String, String)> {
    if model.trim().is_empty() {
        bail!("no model to write with: set one in Settings › Phone and other devices");
    }
    let mut user = format!("Task: {context}");
    if !diff.trim().is_empty() {
        user += "\n\nChanges:\n";
        user += diff;
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .no_proxy()
        .build()?;
    let res = client
        .post(format!(
            "{}/v1/chat/completions",
            base_url.trim_end_matches('/')
        ))
        .bearer_auth(key)
        .json(&json!({
            "model": model,
            "messages": [
                {"role": "system", "content": instructions(kind)},
                {"role": "user", "content": user},
            ],
            "stream": false,
        }))
        .send()
        .await?;
    let status = res.status();
    let v: Value = res.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let said = v["error"]["message"].as_str().unwrap_or("no reason given");
        bail!("{model} could not write it ({status}): {said}");
    }
    let reply = v["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or_default();
    let (title, body) = split_reply(kind, reply);
    if title.is_empty() {
        bail!("{model} wrote nothing");
    }
    Ok((title, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        run(d, ["init", "--quiet", "--initial-branch=main"]).unwrap();
        run(d, ["config", "user.name", "Test"]).unwrap();
        run(d, ["config", "user.email", "test@example.com"]).unwrap();
        run(d, ["config", "commit.gpgsign", "false"]).unwrap();
        std::fs::write(d.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        run(d, ["add", "."]).unwrap();
        run(d, ["commit", "--quiet", "-m", "first"]).unwrap();
        dir
    }

    #[test]
    fn diffs_what_is_not_committed_and_leaves_the_index_alone() {
        let dir = repo();
        let d = dir.path();
        std::fs::write(d.join("a.txt"), "one\n2\nthree\n").unwrap();
        std::fs::write(d.join("new.txt"), "hello\n").unwrap();
        let diff = diff(d, &Scope::Uncommitted).unwrap();
        let a = diff.files.iter().find(|f| f.path == "a.txt").unwrap();
        assert_eq!((a.status.as_str(), a.add, a.del), ("modified", 1, 1));
        let line = &a.hunks[0].lines.iter().find(|l| l.kind == '+').unwrap();
        assert_eq!((line.text.as_str(), line.new), ("2", Some(2)));
        let n = diff.files.iter().find(|f| f.path == "new.txt").unwrap();
        assert_eq!((n.status.as_str(), n.add), ("added", 1));
        // The untracked file is still untracked: the user's index is as it was.
        assert!(
            run(d, ["status", "--porcelain"])
                .unwrap()
                .contains("?? new.txt")
        );
    }

    #[test]
    fn says_the_status_with_counts() {
        let dir = repo();
        let d = dir.path();
        std::fs::write(d.join("a.txt"), "one\n").unwrap();
        std::fs::write(d.join("b.txt"), "x\ny\n").unwrap();
        let st = status(d).unwrap();
        assert_eq!(st.branch, "main");
        assert_eq!(st.default_branch, "main");
        assert!(st.on_default && !st.worktree);
        let a = st.files.iter().find(|f| f.path == "a.txt").unwrap();
        assert_eq!((a.status.as_str(), a.add, a.del), ("modified", 0, 2));
        let b = st.files.iter().find(|f| f.path == "b.txt").unwrap();
        assert_eq!((b.status.as_str(), b.add), ("untracked", 2));
    }

    #[test]
    fn makes_a_worktree_checkpoints_it_and_diffs_a_turn() {
        let dir = repo();
        let root = tempfile::tempdir().unwrap();
        let w = create_worktree_in(dir.path(), "main", root.path()).unwrap();
        assert!(w.branch.starts_with("pansy/"));
        assert!(status(&w.path).unwrap().worktree);
        checkpoint(&w.path, "t_1", 0).unwrap();
        std::fs::write(w.path.join("c.txt"), "made\n").unwrap();
        checkpoint(&w.path, "t_1", 1).unwrap();
        let d = diff(
            &w.path,
            &Scope::Range {
                from: checkpoint_ref("t_1", 0),
                to: checkpoint_ref("t_1", 1),
            },
        )
        .unwrap();
        assert_eq!(d.files.len(), 1);
        assert_eq!(d.files[0].path, "c.txt");
        // A checkpoint moves no branch.
        assert!(
            status(&w.path)
                .unwrap()
                .files
                .iter()
                .any(|f| f.path == "c.txt")
        );
        let named = rename_branch(&w.path, &w.branch, "Fix the login page!").unwrap();
        assert_eq!(named, "pansy/fix-the-login-page");
        assert_eq!(current_branch(&w.path).unwrap(), named);
        let sha = commit(&w.path, None, "Add c\n\nBecause.").unwrap();
        assert_eq!(
            run(&w.path, ["log", "-1", "--format=%H %s"])
                .unwrap()
                .trim(),
            format!("{sha} Add c")
        );
        assert!(commit(&w.path, None, "again").is_err());
        let st = status(&w.path).unwrap();
        assert!(st.files.is_empty());
        remove_worktree(dir.path(), &w.path, &named).unwrap();
        assert!(!w.path.exists());
    }

    #[test]
    fn diffs_from_where_the_branch_left_its_base() {
        let dir = repo();
        let d = dir.path();
        let b = new_branch(d, Some("main")).unwrap();
        assert_eq!(current_branch(d).unwrap(), b);
        std::fs::write(d.join("a.txt"), "one\ntwo\nthree\nfour\n").unwrap();
        commit(d, Some(&["a.txt".into()]), "four").unwrap();
        std::fs::write(d.join("z.txt"), "z\n").unwrap();
        let diff = diff(d, &Scope::Base("main".into())).unwrap();
        let paths: Vec<_> = diff.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["a.txt", "z.txt"]);
    }

    #[test]
    fn reads_github_remotes() {
        for url in [
            "git@github.com:pansy-ai/pansy.git",
            "https://github.com/pansy-ai/pansy",
            "https://github.com/pansy-ai/pansy.git/",
            "ssh://git@github.com/pansy-ai/pansy.git",
            "https://x-access-token@github.com/pansy-ai/pansy.git",
        ] {
            assert_eq!(
                parse_github(url),
                Some(("pansy-ai".into(), "pansy".into())),
                "{url}"
            );
        }
        assert_eq!(parse_github("https://gitlab.com/a/b"), None);
    }

    #[test]
    fn takes_a_title_and_body_from_a_reply() {
        assert_eq!(
            split_reply(Writing::Commit, "```\nFix login\n\nThe token expired.\n```"),
            ("Fix login".into(), "The token expired.".into())
        );
        assert_eq!(
            split_reply(Writing::Branch, "`fix-login-timeout`"),
            ("fix-login-timeout".into(), String::new())
        );
        assert_eq!(slug("pansy/Add OAuth — Google"), "add-oauth-google");
    }

    #[test]
    fn parses_renames_deletions_and_binaries() {
        let patch = "diff --git a/old.txt b/new.txt\nsimilarity index 90%\nrename from old.txt\nrename to new.txt\n--- a/old.txt\n+++ b/new.txt\n@@ -1,2 +1,2 @@\n keep\n-gone\n+here\ndiff --git a/x.bin b/x.bin\nnew file mode 100644\nBinary files /dev/null and b/x.bin differ\ndiff --git a/d.txt b/d.txt\ndeleted file mode 100644\n--- a/d.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-bye\n";
        let d = parse_diff(patch);
        assert_eq!(d.files.len(), 3);
        assert_eq!(
            (
                d.files[0].status.as_str(),
                d.files[0].old_path.as_deref(),
                d.files[0].path.as_str()
            ),
            ("renamed", Some("old.txt"), "new.txt")
        );
        assert_eq!((d.files[0].add, d.files[0].del), (1, 1));
        assert!(d.files[1].binary && d.files[1].status == "added");
        assert_eq!(
            (
                d.files[2].status.as_str(),
                d.files[2].path.as_str(),
                d.files[2].del
            ),
            ("deleted", "d.txt", 1)
        );
    }
}
