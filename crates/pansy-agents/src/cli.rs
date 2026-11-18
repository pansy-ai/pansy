//! An agent's program: where it is, and its version. The version is read
//! where the install says it (the npm package's `package.json`, a folder
//! named for it) and only else asked of the program (`--version`). A
//! version manager's shim (mise's, asdf's) is looked through and never run:
//! running one can install the tool it stands for.
//!
//! And its update: where the install tells how it was made (npm's, bun's or
//! pnpm's global packages, mise, the program's own updater), the version
//! that is out is asked of the npm registry or of mise, and the command
//! that updates it is run when the user says so, never before.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

/// An agent's program as the Agents page shows it.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cli {
    pub version: Option<String>,
    /// What installed it, when its install tells: `npm`, `bun`, `pnpm`,
    /// `mise`, or `own` for its own updater.
    pub via: Option<&'static str>,
    /// The command that updates it, as a terminal would take it.
    pub command: Option<String>,
    /// The version that is out, once asked.
    pub latest: Option<String>,
    /// A newer one is out.
    pub update: bool,
}

/// How a program was installed: what updates it, and where the version
/// that is out is asked.
#[derive(Debug, Clone, PartialEq)]
pub struct How {
    pub via: &'static str,
    /// The command that updates it: a program and its arguments.
    pub command: Vec<String>,
    pub latest: Latest,
}

/// Where the version that is out is asked.
#[derive(Debug, Clone, PartialEq)]
pub enum Latest {
    /// The npm registry, of this package.
    Npm(String),
    /// mise, of this tool.
    Mise(String),
}

/// The package `real` is a file of, and the folder its `node_modules` is in.
fn npm_package(real: &Path) -> Option<(String, PathBuf)> {
    let parts: Vec<String> = real
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let at = parts.iter().rposition(|p| p == "node_modules")?;
    let first = parts.get(at + 1)?;
    let package = if first.starts_with('@') {
        format!("{first}/{}", parts.get(at + 2)?)
    } else {
        first.clone()
    };
    Some((package, real.components().take(at).collect()))
}

/// How `program`, the file at `real` (links followed), was installed, when
/// its place tells. `own`: the arguments of its own updater and its npm
/// package, for one whose installer keeps a folder named for each version
/// (Claude Code's `versions/2.1.3`).
pub fn how(real: &Path, program: &str, own: Option<(&[&str], &str)>) -> Option<How> {
    let text = real.to_string_lossy().replace('\\', "/");
    let argv = |items: &[&str]| items.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    // mise's installs/<tool>/<version>/…
    if let Some(rest) = text.split("/mise/installs/").nth(1)
        && let Some(tool) = rest.split('/').next().filter(|t| !t.is_empty())
    {
        return Some(How {
            via: "mise",
            command: argv(&["mise", "upgrade", tool]),
            latest: Latest::Mise(tool.to_owned()),
        });
    }
    if let Some((package, above)) = npm_package(real) {
        let latest = format!("{package}@latest");
        let (via, command) = if text.contains("/.bun/") {
            ("bun", argv(&["bun", "add", "-g", &latest]))
        } else if text.contains("/pnpm/") {
            ("pnpm", argv(&["pnpm", "add", "-g", &latest]))
        } else {
            // The npm beside the Node it was installed under (nvm keeps
            // one for each), else the one on PATH.
            let beside = above.parent().map(|prefix| prefix.join("bin").join("npm"));
            let npm = beside
                .filter(|f| f.is_file())
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_else(|| "npm".to_owned());
            ("npm", argv(&[&npm, "install", "-g", &latest]))
        };
        return Some(How {
            via,
            command,
            latest: Latest::Npm(package),
        });
    }
    let (args, package) = own?;
    if !text.contains("/versions/") || path_version(real).is_none() {
        return None;
    }
    // Its updater is asked of the program as PATH finds it.
    let mut command = vec![program.to_owned()];
    command.extend(args.iter().map(|a| (*a).to_owned()));
    Some(How {
        via: "own",
        command,
        latest: Latest::Npm(package.to_owned()),
    })
}

/// Whether `latest` is a later version than `current`: by their numbers;
/// the same numbers, when only `current` is a pre-release.
pub fn newer(latest: &str, current: &str) -> bool {
    fn parts(v: &str) -> (Vec<u64>, bool) {
        let v = v.trim().trim_start_matches('v');
        let (numbers, pre) = match v.find(['-', '+']) {
            Some(at) => (&v[..at], v[at..].starts_with('-')),
            None => (v, false),
        };
        (
            numbers.split('.').map(|n| n.parse().unwrap_or(0)).collect(),
            pre,
        )
    }
    let (a, a_pre) = parts(latest);
    let (b, b_pre) = parts(current);
    let n = a.len().max(b.len());
    let at = |v: &[u64], i: usize| v.get(i).copied().unwrap_or(0);
    for i in 0..n {
        if at(&a, i) != at(&b, i) {
            return at(&a, i) > at(&b, i);
        }
    }
    b_pre && !a_pre
}

/// The npm registry: `PANSY_NPM_REGISTRY`, else npm's own.
fn registry() -> String {
    std::env::var("PANSY_NPM_REGISTRY")
        .ok()
        .filter(|r| !r.trim().is_empty())
        .unwrap_or_else(|| "https://registry.npmjs.org".to_owned())
        .trim_end_matches('/')
        .to_owned()
}

/// The version that is out, as last asked: kept half an hour, a failure
/// five minutes.
pub fn latest(from: &Latest) -> Option<String> {
    type Asked = HashMap<String, (Instant, Option<String>)>;
    static ASKED: Mutex<Option<Asked>> = Mutex::new(None);
    let key = match from {
        Latest::Npm(package) => format!("npm:{}/{package}", registry()),
        Latest::Mise(tool) => format!("mise:{tool}"),
    };
    if let Some((at, said)) = ASKED
        .lock()
        .ok()
        .and_then(|m| m.as_ref()?.get(&key).cloned())
        && at.elapsed() < Duration::from_secs(if said.is_some() { 1800 } else { 300 })
    {
        return said;
    }
    let said = match from {
        Latest::Npm(package) => {
            let url = format!("{}/{package}/latest", registry());
            let proxy = pansy_core::config::load_settings()
                .map(|s| s.proxy)
                .unwrap_or_default();
            pansy_plugins::block(async move {
                let builder = reqwest::Client::builder()
                    .user_agent(concat!("pansy/", env!("CARGO_PKG_VERSION")))
                    .timeout(Duration::from_secs(8));
                let client = pansy_core::settings::use_proxy(builder, &proxy)
                    .ok()?
                    .build()
                    .ok()?;
                let json: serde_json::Value = client
                    .get(url)
                    .send()
                    .await
                    .ok()?
                    .error_for_status()
                    .ok()?
                    .json()
                    .await
                    .ok()?;
                parse_version(json.get("version")?.as_str()?)
            })
        }
        Latest::Mise(tool) => which("mise").and_then(|mise| {
            let mut cmd = Command::new(mise);
            cmd.args(["latest", tool]);
            let (ok, out) = run(cmd, Duration::from_secs(20))?;
            ok.then(|| parse_version(&out))?
        }),
    };
    if let Ok(mut m) = ASKED.lock() {
        m.get_or_insert_with(HashMap::new)
            .insert(key, (Instant::now(), said.clone()));
    }
    said
}

/// The program of the first of `programs` on `PATH`, links and a version
/// manager's shim followed.
pub fn real_of(programs: &[&str]) -> Option<PathBuf> {
    let (program, found) = programs.iter().find_map(|p| Some((*p, which(p)?)))?;
    real_program(program, &found)
}

/// A program's version and how it is updated, the version that is out not
/// asked yet.
pub fn installed(programs: &[&str], own: Option<(&[&str], &str)>) -> (Cli, Option<How>) {
    let found = programs.iter().find_map(|p| Some((*p, which(p)?)));
    let Some((program, real)) = found.and_then(|(p, found)| Some((p, real_program(p, &found)?)))
    else {
        return (Cli::default(), None);
    };
    let how = how(&real, program, own);
    let cli = Cli {
        version: version(&real),
        via: how.as_ref().map(|h| h.via),
        command: how.as_ref().map(|h| h.command.join(" ")),
        ..Cli::default()
    };
    (cli, how)
}

/// `cli` with the version that is out, asked now if it wasn't lately.
pub fn with_latest(mut cli: Cli, how: &How) -> Cli {
    cli.latest = latest(&how.latest);
    cli.update = match (&cli.latest, &cli.version) {
        (Some(latest), Some(version)) => newer(latest, version),
        _ => false,
    };
    cli
}

/// Runs the command that updates the program: what it printed last, when
/// it failed. It may take minutes (an npm install).
pub fn update(how: &How) -> Result<(), String> {
    let (program, args) = how.command.split_first().ok_or("no command")?;
    let found = which(program).unwrap_or_else(|| PathBuf::from(program));
    let mut cmd = Command::new(found);
    cmd.args(args).env("NO_COLOR", "1");
    let said = how.command.join(" ");
    match run(cmd, Duration::from_secs(600)) {
        None => Err(format!("{said} could not be run, or took over ten minutes")),
        Some((true, _)) => Ok(()),
        Some((false, out)) => {
            let last: Vec<&str> = out
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .rev()
                .take(3)
                .collect();
            let last: Vec<&str> = last.into_iter().rev().collect();
            Err(format!("{said} failed: {}", last.join(" · ")))
        }
    }
}

/// The version of the first of `programs` on `PATH`: `None` when none is,
/// it doesn't say, or it is a shim whose manager can't say what it runs.
/// Can take a while (a version manager asked, a program run): the Agents
/// page asks it after its rows.
pub fn version_of(programs: &[&str]) -> Option<String> {
    let (program, found) = programs.iter().find_map(|p| Some((*p, which(p)?)))?;
    version(&real_program(program, &found)?)
}

/// Where `PATH` finds `program`.
pub fn which(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT".into())
            .split(';')
            .map(|e| e.to_ascii_lowercase())
            .collect()
    } else {
        vec![String::new()]
    };
    std::env::split_paths(&path).find_map(|dir| {
        exts.iter()
            .map(|ext| dir.join(format!("{program}{ext}")))
            .find(|f| f.is_file())
    })
}

/// The version manager whose shim `found` is: mise's shims (and command
/// wrappers) are links to mise itself or files in its `shims` folder,
/// asdf's are scripts in `~/.asdf/shims`.
fn manager(found: &Path, real: &Path) -> Option<&'static str> {
    if stem(real) == "mise" {
        return Some("mise");
    }
    let dir = found.parent()?;
    if stem(dir) != "shims" {
        return None;
    }
    let above = dir.parent().map(stem).unwrap_or_default();
    match above.as_str() {
        "mise" => Some("mise"),
        ".asdf" | "asdf" => Some("asdf"),
        _ => None,
    }
}

/// What a version manager said each shim runs, and when.
type Asked = HashMap<PathBuf, (Instant, Option<PathBuf>)>;
/// Each program's version, by the file's size and time.
type Known = HashMap<PathBuf, ((u64, Option<SystemTime>), Option<String>)>;

/// A file's name without its extension, in lower case (`mise.exe` is `mise`).
fn stem(p: &Path) -> String {
    p.file_stem()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

/// The program `found` runs: itself, links followed, or what the version
/// manager whose shim it is says it runs (none when it has none installed
/// and active, or can't say). Its answer is kept ten minutes, a failure one.
fn real_program(program: &str, found: &Path) -> Option<PathBuf> {
    let real = fs::canonicalize(found).unwrap_or_else(|_| found.to_path_buf());
    let Some(manager) = manager(found, &real) else {
        return Some(real);
    };
    static ASKED: Mutex<Option<Asked>> = Mutex::new(None);
    if let Some((at, said)) = ASKED
        .lock()
        .ok()
        .and_then(|m| m.as_ref()?.get(found).cloned())
        && at.elapsed() < Duration::from_secs(if said.is_some() { 600 } else { 60 })
    {
        return said;
    }
    // Its manager on PATH, else the one its link names; never the shim.
    let bin = which(manager).or_else(|| (stem(&real) == manager).then(|| real.clone()))?;
    let mut cmd = Command::new(bin);
    cmd.args(["which", program])
        // which never installs; these say so to every mise there is
        .env("MISE_AUTO_INSTALL", "0")
        .env("MISE_EXEC_AUTO_INSTALL", "0")
        .env("MISE_NOT_FOUND_AUTO_INSTALL", "0");
    // from the home folder, whose config a terminal opened there has
    if let Some(home) = std::env::var_os("HOME").filter(|h| Path::new(h).is_dir()) {
        cmd.current_dir(home);
    }
    // mise looks up a `latest` it hasn't kept over the network
    let said = run(cmd, Duration::from_secs(15))
        .filter(|(ok, _)| *ok)
        .and_then(|(_, out)| {
            let line = out.lines().map(str::trim).find(|l| !l.is_empty())?;
            let path = PathBuf::from(line);
            path.is_file()
                .then(|| fs::canonicalize(&path).unwrap_or(path))
        });
    if let Ok(mut m) = ASKED.lock() {
        m.get_or_insert_with(HashMap::new)
            .insert(found.to_path_buf(), (Instant::now(), said.clone()));
    }
    said
}

static KNOWN: Mutex<Option<Known>> = Mutex::new(None);

/// Forgets the version kept for the program at `real`: an update may
/// leave the file itself as it was.
pub fn forget(real: &Path) {
    if let Ok(mut m) = KNOWN.lock()
        && let Some(known) = m.as_mut()
    {
        known.remove(real);
    }
}

/// The version of the program at `real`, kept while the file stays the
/// same: a run that failed isn't made again until it changes.
pub fn version(real: &Path) -> Option<String> {
    let meta = fs::metadata(real).ok()?;
    let stamp = (meta.len(), meta.modified().ok());
    if let Some((was, v)) = KNOWN
        .lock()
        .ok()
        .and_then(|m| m.as_ref()?.get(real).cloned())
        && was == stamp
    {
        return v;
    }
    let v = npm_version(real)
        .or_else(|| shim_version(real))
        .or_else(|| path_version(real))
        .or_else(|| asked_version(real));
    if let Ok(mut m) = KNOWN.lock() {
        m.get_or_insert_with(HashMap::new)
            .insert(real.to_path_buf(), (stamp, v.clone()));
    }
    v
}

/// The version of the npm package `real` is a file of
/// (`…/node_modules/@openai/codex/bin/codex.js` is in `@openai/codex`).
pub fn npm_version(real: &Path) -> Option<String> {
    real.ancestors().skip(1).find_map(|dir| {
        let up = dir.parent()?;
        let up_name = up.file_name()?.to_string_lossy();
        let in_modules = up_name == "node_modules"
            || (up_name.starts_with('@')
                && up
                    .parent()?
                    .file_name()
                    .is_some_and(|n| n == "node_modules"));
        in_modules.then(|| package_version(&dir.join("package.json")))?
    })
}

/// The version of the npm package a small script beside `node_modules`
/// starts (npm's `.cmd` shims on Windows, pnpm's scripts).
pub fn shim_version(real: &Path) -> Option<String> {
    let meta = fs::metadata(real).ok()?;
    if meta.len() > 64 << 10 {
        return None;
    }
    let text = fs::read_to_string(real).ok()?.replace('\\', "/");
    let rest = &text[text.find("/node_modules/")? + "/node_modules/".len()..];
    let mut parts = rest.split('/');
    let first = parts.next()?;
    let pkg = if first.starts_with('@') {
        format!("{first}/{}", parts.next()?)
    } else {
        first.to_owned()
    };
    if pkg.is_empty() || pkg.contains(|c: char| c.is_whitespace() || "\"'%$".contains(c)) {
        return None;
    }
    package_version(
        &real
            .parent()?
            .join("node_modules")
            .join(pkg)
            .join("package.json"),
    )
}

fn package_version(file: &Path) -> Option<String> {
    let json: serde_json::Value = serde_json::from_str(&fs::read_to_string(file).ok()?).ok()?;
    let v = json.get("version")?.as_str()?.trim();
    (!v.is_empty()).then(|| v.to_owned())
}

/// A version the install is named for: Claude Code's own installer keeps
/// `versions/2.1.3`, Homebrew `Cellar/gemini-cli/0.6.0/bin/gemini`, mise
/// `installs/codex/0.9.0/bin/codex`.
pub fn path_version(real: &Path) -> Option<String> {
    real.ancestors().take(4).find_map(|p| {
        // nvm's versions/node/v22.1.0 is Node's version, not the program's
        if p.parent().is_some_and(|up| stem(up) == "node") {
            return None;
        }
        let name = p.file_name()?.to_string_lossy();
        let name = name.strip_prefix('v').unwrap_or(&name);
        (parse_version(name).as_deref() == Some(name)).then(|| name.to_owned())
    })
}

/// What `real --version` says, its version alone.
fn asked_version(real: &Path) -> Option<String> {
    let mut cmd = Command::new(real);
    cmd.arg("--version").env("NO_COLOR", "1");
    let (_, out) = run(cmd, Duration::from_secs(10))?;
    parse_version(&out)
}

/// The first version in `text`: `1.2`, `0.168.0`, `2.0.0-beta.3`.
pub fn parse_version(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // a number that doesn't go on another word's digits
        if bytes[i].is_ascii_digit()
            && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'v')
        {
            let start = i;
            let mut dots = 0;
            let mut end = i;
            while end < bytes.len() {
                if bytes[end].is_ascii_digit() {
                    end += 1;
                } else if bytes[end] == b'.'
                    && end + 1 < bytes.len()
                    && bytes[end + 1].is_ascii_digit()
                {
                    dots += 1;
                    end += 1;
                } else {
                    break;
                }
            }
            if dots >= 1 {
                // a pre-release or build after it
                if end + 1 < bytes.len()
                    && (bytes[end] == b'-' || bytes[end] == b'+')
                    && bytes[end + 1].is_ascii_alphanumeric()
                {
                    end += 1;
                    while end < bytes.len()
                        && (bytes[end].is_ascii_alphanumeric()
                            || bytes[end] == b'.'
                            || bytes[end] == b'-')
                    {
                        end += 1;
                    }
                    while bytes[end - 1] == b'.' || bytes[end - 1] == b'-' {
                        end -= 1;
                    }
                }
                return Some(text[start..end].to_owned());
            }
            i = end.max(i + 1);
            continue;
        }
        i += 1;
    }
    None
}

/// Runs `cmd` with nothing on its stdin, for at most `limit`: whether it
/// exited well, and what it printed (stdout, then stderr). `None` when it
/// couldn't start or took too long (it is stopped then).
fn run(mut cmd: Command, limit: Duration) -> Option<(bool, String)> {
    let mut child: Child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(15)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let out = child.wait_with_output().ok()?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Some((out.status.success(), text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_version_out_of_what_a_program_says() {
        assert_eq!(
            parse_version("2.1.323 (Claude Code)").as_deref(),
            Some("2.1.323")
        );
        assert_eq!(
            parse_version("codex-cli 0.168.0").as_deref(),
            Some("0.168.0")
        );
        assert_eq!(parse_version("opencode v2.2.7\n").as_deref(), Some("2.2.7"));
        assert_eq!(
            parse_version("crush version v0.10.0-beta.2.").as_deref(),
            Some("0.10.0-beta.2")
        );
        assert_eq!(parse_version("x86_64 build 7, 1.2").as_deref(), Some("1.2"));
        assert_eq!(parse_version("no version here 7"), None);
    }

    #[test]
    fn reads_a_version_where_the_install_says_it() {
        let dir = tempfile::tempdir().unwrap();
        // an npm package, its program in bin/
        let pkg = dir.path().join("lib/node_modules/@openai/codex");
        fs::create_dir_all(pkg.join("bin")).unwrap();
        fs::write(
            pkg.join("package.json"),
            r#"{"name":"@openai/codex","version":"0.168.0"}"#,
        )
        .unwrap();
        fs::write(pkg.join("bin/codex.js"), "").unwrap();
        assert_eq!(
            npm_version(&pkg.join("bin/codex.js")).as_deref(),
            Some("0.168.0")
        );
        assert_eq!(
            version(&pkg.join("bin/codex.js")).as_deref(),
            Some("0.168.0")
        );

        // npm's .cmd beside node_modules
        let bin = dir.path().join("npm");
        fs::create_dir_all(bin.join("node_modules/cline")).unwrap();
        fs::write(
            bin.join("node_modules/cline/package.json"),
            r#"{"version":"3.1.0"}"#,
        )
        .unwrap();
        fs::write(
            bin.join("cline.cmd"),
            "@\"%dp0%\\node_modules\\cline\\bin\\cline\" %*",
        )
        .unwrap();
        assert_eq!(
            shim_version(&bin.join("cline.cmd")).as_deref(),
            Some("3.1.0")
        );

        // a folder named for it
        let native = dir.path().join("claude/versions/2.1.3");
        fs::create_dir_all(native.parent().unwrap()).unwrap();
        fs::write(&native, "").unwrap();
        assert_eq!(path_version(&native).as_deref(), Some("2.1.3"));
        let brew = dir.path().join("Cellar/gemini-cli/0.6.0/bin/gemini");
        assert_eq!(path_version(&brew).as_deref(), Some("0.6.0"));
        assert_eq!(path_version(&dir.path().join("tools-1.2/bin/x")), None);
    }

    #[test]
    fn knows_a_version_managers_shims() {
        let real = Path::new("/usr/bin/mise");
        assert_eq!(
            manager(
                Path::new("/home/u/.local/share/mise/command-wrappers/bin/claude"),
                real
            ),
            Some("mise")
        );
        let shim = Path::new("/home/u/.local/share/mise/shims/qwen");
        assert_eq!(manager(shim, shim), Some("mise"));
        let asdf = Path::new("/home/u/.asdf/shims/codex");
        assert_eq!(manager(asdf, asdf), Some("asdf"));
        let plain = Path::new("/usr/local/bin/codex");
        assert_eq!(manager(plain, plain), None);
    }
}
