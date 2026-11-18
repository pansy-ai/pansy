//! Everything the gateway routes by, read from pansy's files. The window,
//! the command line and a separate `pansy serve` all write the same files;
//! the gateway reads them again when they change.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::fsutil::{read_json, write_json};
use crate::group::Group;
use crate::paths;
use crate::provider::Provider;
use crate::settings::Settings;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Providers {
    #[serde(default)]
    pub providers: Vec<Provider>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Groups {
    #[serde(default)]
    pub groups: Vec<Group>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Config {
    pub providers: Vec<Provider>,
    pub groups: Vec<Group>,
    pub settings: Settings,
}

impl Config {
    pub fn load() -> Result<Config> {
        Ok(Config {
            providers: load_providers()?,
            groups: load_groups()?,
            settings: load_settings()?,
        })
    }

    pub fn provider(&self, id: &str) -> Option<&Provider> {
        self.providers.iter().find(|p| p.id == id)
    }

    pub fn group(&self, id: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id)
    }
}

/// The providers, with the subscriptions' sign-ins as they are now: each
/// subscription signed in to is a provider whose accounts are its sign-ins.
pub fn load_providers() -> Result<Vec<Provider>> {
    let mut providers = read_json::<Providers>(&paths::providers())?.providers;
    // Keys kept in the system's store are read back; the file says where.
    if !crate::keystore::refs(&providers).is_empty() {
        crate::keystore::open(&mut providers);
    }
    crate::subscription::merge(&mut providers, &crate::subscription::found());
    crate::plugin::merge(&mut providers, &crate::plugin::Listing::load());
    Ok(providers)
}

pub fn save_providers(providers: &[Provider]) -> Result<()> {
    let mut ids: Vec<&str> = providers.iter().map(|p| p.id.as_str()).collect();
    ids.sort_unstable();
    if let Some(w) = ids.windows(2).find(|w| w[0] == w[1]) {
        bail!("two providers are called {}", w[0]);
    }
    let mut providers = providers.to_vec();
    let before = read_json::<Providers>(&paths::providers())
        .map(|kept| crate::keystore::refs(&kept.providers))
        .unwrap_or_default();
    if load_settings().is_ok_and(|s| s.key_store == "system") {
        crate::keystore::seal(&mut providers)?;
    }
    write_json(&paths::providers(), &Providers { providers }, true)?;
    // What the file no longer refers to leaves the store.
    if !before.is_empty() {
        let after = read_json::<Providers>(&paths::providers())
            .map(|kept| crate::keystore::refs(&kept.providers))
            .unwrap_or_default();
        for gone in before.iter().filter(|name| !after.contains(name)) {
            let _ = crate::keystore::remove(gone);
        }
    }
    Ok(())
}

pub fn load_groups() -> Result<Vec<Group>> {
    Ok(read_json::<Groups>(&paths::groups())?.groups)
}

pub fn save_groups(groups: &[Group]) -> Result<()> {
    write_json(
        &paths::groups(),
        &Groups {
            groups: groups.to_vec(),
        },
        false,
    )
}

pub fn load_settings() -> Result<Settings> {
    read_json(&paths::settings())
}

pub fn save_settings(settings: &Settings) -> Result<()> {
    write_json(&paths::settings(), settings, true)
}

/// Runs `f` holding the lock of `file` (`.providers.json.lock` beside it),
/// so two changes of it at once (the window's and the command line's, or
/// two commands') are made one after the other and both land.
pub(crate) fn locked<R>(file: &std::path::Path, f: impl FnOnce() -> Result<R>) -> Result<R> {
    let dir = file.parent().unwrap_or(std::path::Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(format!(".{name}.lock")))?;
    lock.lock()?;
    let out = f();
    let _ = lock.unlock();
    out
}

/// Changes the providers on disk: read, change, write.
pub fn update_providers<R>(change: impl FnOnce(&mut Vec<Provider>) -> Result<R>) -> Result<R> {
    locked(&paths::providers(), || {
        let mut providers = load_providers()?;
        let out = change(&mut providers)?;
        save_providers(&providers)?;
        Ok(out)
    })
}

pub fn update_groups<R>(change: impl FnOnce(&mut Vec<Group>) -> Result<R>) -> Result<R> {
    locked(&paths::groups(), || {
        let mut groups = load_groups()?;
        let out = change(&mut groups)?;
        save_groups(&groups)?;
        Ok(out)
    })
}

pub fn update_settings<R>(change: impl FnOnce(&mut Settings) -> Result<R>) -> Result<R> {
    locked(&paths::settings(), || {
        let mut settings = load_settings()?;
        let out = change(&mut settings)?;
        save_settings(&settings)?;
        Ok(out)
    })
}

/// The config as last read, read again when one of its files changed.
#[derive(Debug, Default)]
pub struct Live {
    state: Mutex<(Vec<Option<SystemTime>>, Arc<Config>)>,
}

impl Live {
    /// pansy's files, and those the subscriptions' sign-ins are found in.
    fn files() -> Vec<PathBuf> {
        let mut files = vec![
            paths::providers(),
            paths::groups(),
            paths::settings(),
            paths::plugin_providers(),
        ];
        files.extend(crate::subscription::watched());
        files
    }

    fn stamps() -> Vec<Option<SystemTime>> {
        Self::files()
            .iter()
            .map(|f| std::fs::metadata(f).and_then(|m| m.modified()).ok())
            .collect()
    }

    /// The current config. A file that cannot be read keeps the last good one.
    pub fn get(&self) -> Arc<Config> {
        let stamps = Self::stamps();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.0 != stamps || state.0.is_empty() {
            match Config::load() {
                Ok(config) => *state = (stamps, Arc::new(config)),
                Err(error) => tracing::warn!("keeping the last config: {error:#}"),
            }
        }
        Arc::clone(&state.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_at_once_all_land() {
        let home = tempfile::tempdir().unwrap();
        let _env = crate::subscription::tests::scratch_home(home.path());
        let threads: Vec<_> = (0..12)
            .map(|i| {
                std::thread::spawn(move || {
                    update_groups(|groups| {
                        groups.push(Group {
                            id: format!("g{i}"),
                            name: format!("G{i}"),
                            members: Vec::new(),
                            routing: Default::default(),
                            strategy: None,
                        });
                        Ok(())
                    })
                    .unwrap()
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(load_groups().unwrap().len(), 12);
    }
}
