//! API keys in the system's own store instead of `providers.json`: the
//! login keyring on Linux (through `secret-tool`), the Keychain on macOS
//! (through `security`). Off unless the user asks (`keyStore`: `system`).
//!
//! With it on, `providers.json` holds where a key is (`keystore:<provider>/
//! <key id>`), never the key. Keys are read back when providers load and
//! kept in memory for as long as the program runs. One that can't be read
//! (the keyring locked, the entry gone) stays a reference, and the account
//! is passed over until it can be.
//!
//! `PANSY_KEYSTORE=dir:<folder>` keeps them as files in a folder instead:
//! for tests, which never touch the user's keys.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;

use anyhow::{Context, Result, anyhow, bail};

use crate::config;
use crate::provider::Provider;

/// What a key in the store is written as in `providers.json`.
pub const REF: &str = "keystore:";

/// The name every entry is filed under.
const SERVICE: &str = "pansy";

/// Whether `key` says where a key is, not what it is.
pub fn is_ref(key: &str) -> bool {
    key.starts_with(REF)
}

enum Store {
    /// libsecret's `secret-tool`, to the login keyring.
    SecretTool,
    /// macOS's `security`, to the login Keychain.
    Security,
    /// Files in a folder, for tests.
    Dir(PathBuf),
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
}

fn store() -> Result<Store> {
    if let Ok(test) = std::env::var("PANSY_KEYSTORE")
        && let Some(dir) = test.strip_prefix("dir:")
    {
        return Ok(Store::Dir(PathBuf::from(dir)));
    }
    if cfg!(target_os = "macos") {
        return Ok(Store::Security);
    }
    if cfg!(windows) {
        bail!("keeping keys in the system's store is not there on Windows yet");
    }
    if on_path("secret-tool") {
        return Ok(Store::SecretTool);
    }
    bail!("the system's key store needs secret-tool (libsecret-tools, libsecret) on PATH")
}

/// Whether keys can be kept in the system's store here, or why not.
pub fn available() -> Result<()> {
    store().map(|_| ())
}

fn cache() -> std::sync::MutexGuard<'static, HashMap<String, String>> {
    static KEPT: std::sync::LazyLock<Mutex<HashMap<String, String>>> =
        std::sync::LazyLock::new(Default::default);
    KEPT.lock().unwrap_or_else(|e| e.into_inner())
}

fn file_of(dir: &std::path::Path, name: &str) -> PathBuf {
    dir.join(name.replace(['/', '\\'], "__"))
}

/// Runs `command`, with `input` on its standard input; what it printed when
/// it succeeded.
fn run(mut command: Command, input: Option<&str>) -> Result<Option<String>> {
    let program = command.get_program().to_string_lossy().into_owned();
    let mut child = command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("starting {program}"))?;
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        stdin.write_all(input.as_bytes())?;
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        let said = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        if said.is_empty() {
            return Ok(None);
        }
        bail!("{program}: {said}");
    }
    Ok(Some(String::from_utf8_lossy(&out.stdout).into_owned()))
}

/// A value inside double quotes on `security -i`'s command line.
fn quoted(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Keeps `secret` under `name`.
pub fn put(name: &str, secret: &str) -> Result<()> {
    if cache().get(name).is_some_and(|kept| kept == secret) {
        return Ok(());
    }
    match store()? {
        Store::Dir(dir) => {
            std::fs::create_dir_all(&dir)?;
            crate::fsutil::write_atomic(&file_of(&dir, name), secret.as_bytes(), true)?;
        }
        Store::SecretTool => {
            let mut command = Command::new("secret-tool");
            command.args([
                "store",
                "--label",
                &format!("pansy: {name}"),
                "service",
                SERVICE,
                "account",
                name,
            ]);
            run(command, Some(secret))?.ok_or_else(|| {
                anyhow!("the login keyring didn't take the key (is it unlocked?)")
            })?;
        }
        Store::Security => {
            // On `security -i`'s input, not its arguments: other programs
            // can read those.
            let line = format!(
                "add-generic-password -U -s {} -a {} -w {}\n",
                quoted(SERVICE),
                quoted(name),
                quoted(secret)
            );
            let mut command = Command::new("security");
            command.arg("-i");
            run(command, Some(&line))?
                .ok_or_else(|| anyhow!("the Keychain didn't take the key"))?;
        }
    }
    cache().insert(name.to_owned(), secret.to_owned());
    Ok(())
}

/// The secret under `name`; `None` when the store has none.
pub fn get(name: &str) -> Result<Option<String>> {
    if let Some(kept) = cache().get(name) {
        return Ok(Some(kept.clone()));
    }
    let found = match store()? {
        Store::Dir(dir) => match std::fs::read_to_string(file_of(&dir, name)) {
            Ok(secret) => Some(secret),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        },
        Store::SecretTool => {
            let mut command = Command::new("secret-tool");
            command.args(["lookup", "service", SERVICE, "account", name]);
            run(command, None)?
        }
        Store::Security => {
            let mut command = Command::new("security");
            command.args(["find-generic-password", "-s", SERVICE, "-a", name, "-w"]);
            // It says "could not be found" for one that isn't there.
            run(command, None)
                .ok()
                .flatten()
                .map(|s| s.trim_end_matches('\n').to_owned())
        }
    }
    .filter(|s| !s.is_empty());
    if let Some(secret) = &found {
        cache().insert(name.to_owned(), secret.clone());
    }
    Ok(found)
}

/// Removes what is under `name`.
pub fn remove(name: &str) -> Result<()> {
    cache().remove(name);
    match store()? {
        Store::Dir(dir) => {
            let _ = std::fs::remove_file(file_of(&dir, name));
        }
        Store::SecretTool => {
            let mut command = Command::new("secret-tool");
            command.args(["clear", "service", SERVICE, "account", name]);
            run(command, None)?;
        }
        Store::Security => {
            let mut command = Command::new("security");
            command.args(["delete-generic-password", "-s", SERVICE, "-a", name]);
            let _ = run(command, None);
        }
    }
    Ok(())
}

fn name_of(provider: &str, account: &str) -> String {
    format!("{provider}/{account}")
}

/// After reading `providers.json`: each reference becomes its key. One that
/// can't be read stays a reference; how many did.
pub fn open(providers: &mut [Provider]) -> usize {
    let mut locked = 0;
    for provider in providers {
        for account in &mut provider.accounts {
            let Some(name) = account.key.strip_prefix(REF) else {
                continue;
            };
            match get(name) {
                Ok(Some(secret)) => account.key = secret,
                Ok(None) => {
                    locked += 1;
                    tracing::warn!("the system's key store has no key {name}");
                }
                Err(e) => {
                    locked += 1;
                    tracing::warn!("reading the key {name}: {e:#}");
                }
            }
        }
    }
    locked
}

/// Before writing `providers.json` with keys in the store: each key is
/// kept there and becomes its reference.
pub fn seal(providers: &mut [Provider]) -> Result<()> {
    for provider in providers {
        for account in &mut provider.accounts {
            if account.key.is_empty() || is_ref(&account.key) {
                continue;
            }
            let name = name_of(&provider.id, &account.id);
            put(&name, &account.key).with_context(|| {
                format!("keeping {}'s key in the system's store", provider.name)
            })?;
            account.key = format!("{REF}{name}");
        }
    }
    Ok(())
}

/// The names `providers` refer to.
pub fn refs(providers: &[Provider]) -> Vec<String> {
    providers
        .iter()
        .flat_map(|p| &p.accounts)
        .filter_map(|a| a.key.strip_prefix(REF).map(str::to_owned))
        .collect()
}

/// Moves every key into the system's store (`system`) or back into
/// `providers.json` (`file`), and sets `keyStore`; how many keys moved.
/// Nothing changes when it can't all be done.
pub fn move_to(mode: &str) -> Result<usize> {
    match mode {
        "system" => available()?,
        "file" => {}
        _ => bail!("keyStore is file or system"),
    }
    let was = config::load_settings()?.key_store;
    if was == mode {
        return Ok(0);
    }
    let count = |providers: &[Provider]| {
        providers
            .iter()
            .flat_map(|p| &p.accounts)
            .filter(|a| !a.key.is_empty())
            .count()
    };
    if mode == "file" {
        // Every key must be read back before the references are let go.
        let mut providers = config::load_providers()?;
        let locked = open(&mut providers);
        if locked > 0 {
            bail!(
                "{locked} key(s) can't be read from the system's store now (is it unlocked?); nothing was changed"
            );
        }
    }
    config::update_settings(|s| {
        s.key_store = mode.to_owned();
        Ok(())
    })?;
    match config::update_providers(|providers| Ok(count(providers))) {
        Ok(moved) => Ok(moved),
        Err(e) => {
            config::update_settings(|s| {
                s.key_store = was;
                Ok(())
            })?;
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manage;
    use crate::provider::Protocol;

    #[test]
    fn keys_move_into_the_store_and_back() {
        let home = tempfile::tempdir().unwrap();
        let _env = crate::subscription::tests::scratch_home(home.path());
        let store = home.path().join("keys");
        // SAFETY: the guard is held; the variable is taken away again below.
        unsafe { std::env::set_var("PANSY_KEYSTORE", format!("dir:{}", store.display())) };

        let id = manage::add_custom("Acme", Protocol::Chat, "https://acme.example/v1", None, &[])
            .unwrap();
        let first = manage::add_key(&id, "sk-first-secret", "").unwrap();
        let file = || std::fs::read_to_string(crate::paths::providers()).unwrap();
        assert!(file().contains("sk-first-secret"));

        assert_eq!(move_to("system").unwrap(), 1);
        assert!(!file().contains("sk-first-secret"), "{}", file());
        assert!(file().contains(&format!("keystore:{id}/{first}")));
        // In memory the key is the key.
        let key_of = |account: &str| {
            let providers = config::load_providers().unwrap();
            let p = providers.iter().find(|p| p.id == id).unwrap();
            p.accounts
                .iter()
                .find(|a| a.id == account)
                .map(|a| a.key.clone())
        };
        assert_eq!(key_of(&first).as_deref(), Some("sk-first-secret"));

        // A key added now goes to the store; one removed leaves it.
        let second = manage::add_key(&id, "sk-second-secret", "").unwrap();
        assert!(!file().contains("sk-second-secret"));
        assert_eq!(std::fs::read_dir(&store).unwrap().count(), 2);
        manage::remove_key(&id, &first).unwrap();
        assert_eq!(std::fs::read_dir(&store).unwrap().count(), 1);

        // An entry gone from the store: the account is passed over, not sent
        // out as a reference, and the others' keys are not touched by a save.
        let third = manage::add_key(&id, "sk-third-secret", "").unwrap();
        std::fs::remove_file(file_of(&store, &name_of(&id, &third))).unwrap();
        cache().remove(&name_of(&id, &third));
        assert!(is_ref(&key_of(&third).unwrap()));
        let providers = config::load_providers().unwrap();
        let usable: Vec<&str> = providers
            .iter()
            .find(|p| p.id == id)
            .unwrap()
            .enabled_accounts()
            .map(|a| a.id.as_str())
            .collect();
        assert_eq!(usable, [second.as_str()]);
        let error = move_to("file").unwrap_err().to_string();
        assert!(error.contains("can't be read"), "{error}");
        assert!(!file().contains("sk-second-secret"), "nothing was changed");

        // Back into the file, once every key can be read.
        manage::remove_key(&id, &third).unwrap();
        assert_eq!(move_to("file").unwrap(), 1);
        assert!(file().contains("sk-second-secret"));
        assert!(!file().contains("keystore:"));
        assert_eq!(std::fs::read_dir(&store).unwrap().count(), 0);
        assert!(move_to("vault").is_err());

        // SAFETY: as above.
        unsafe { std::env::remove_var("PANSY_KEYSTORE") };
    }
}
