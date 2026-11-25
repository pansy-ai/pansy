//! The remote interface's own settings: `remote-settings.json` in pansy's
//! home. Off until the user turns it on.

use anyhow::Result;
use pansy_core::fsutil::{read_json, write_json};
use pansy_core::paths;
use serde::{Deserialize, Serialize};

/// The port the remote interface listens on: never the gateway's.
pub const PORT: u16 = 7268;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Other devices may control this computer.
    pub enabled: bool,
    /// Listen beyond this computer (private networks and tailnets only).
    pub lan: bool,
    pub port: u16,
    /// Where a line goes when a thread waits or ends (ntfy, Bark, a bot):
    /// the thread's title and what happened, never what was said.
    pub webhook: String,
    /// The model that writes commit messages, pull requests and branch
    /// names; empty: the thread's own.
    pub writer_model: String,
    /// A name for this computer on the phone; empty: the system's.
    pub name: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            enabled: false,
            lan: true,
            port: PORT,
            webhook: String::new(),
            writer_model: String::new(),
            name: String::new(),
        }
    }
}

pub fn path() -> std::path::PathBuf {
    paths::home().join("remote-settings.json")
}

pub fn load() -> Result<Settings> {
    read_json(&path())
}

pub fn save(s: &Settings) -> Result<()> {
    write_json(&path(), s, false)
}

/// This computer's name, as the phone shows it.
pub fn machine_name(s: &Settings) -> String {
    if !s.name.trim().is_empty() {
        return s.name.trim().to_owned();
    }
    let host = std::process::Command::new("hostname")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|h| !h.is_empty());
    host.unwrap_or_else(|| "pansy".into())
}
