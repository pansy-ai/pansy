//! Where pansy keeps its files: `PANSY_HOME`, else `~/.config/pansy` on
//! every system (agents keep theirs under the home directory too, and one
//! place is easier to find, back up and sync).

use std::path::PathBuf;

/// pansy's directory.
pub fn home() -> PathBuf {
    if let Some(dir) = std::env::var_os("PANSY_HOME").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    user_home().join(".config").join("pansy")
}

/// The user's home directory: `HOME` (tests point it elsewhere), else the
/// system's.
pub fn user_home() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME").filter(|h| !h.is_empty()) {
        return PathBuf::from(home);
    }
    #[cfg(windows)]
    if let Some(home) = std::env::var_os("USERPROFILE").filter(|h| !h.is_empty()) {
        return PathBuf::from(home);
    }
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// Where what the user exports goes: their Downloads folder, as the system
/// keeps it (`XDG_DOWNLOAD_DIR` on Linux), else `Downloads` in the home
/// directory. Under `HOME` when it is set, as tests set it (Windows' own
/// folder is asked of the system, which knows nothing of `HOME`).
pub fn downloads() -> PathBuf {
    #[cfg(windows)]
    if std::env::var_os("HOME").is_some_and(|h| !h.is_empty()) {
        return user_home().join("Downloads");
    }
    dirs::download_dir().unwrap_or_else(|| user_home().join("Downloads"))
}

/// Providers, their accounts and keys (private).
pub fn providers() -> PathBuf {
    home().join("providers.json")
}

/// The subscriptions' accounts signed in from pansy, with their tokens
/// (private).
pub fn logins() -> PathBuf {
    home().join("logins.json")
}

pub fn groups() -> PathBuf {
    home().join("groups.json")
}

pub fn profiles() -> PathBuf {
    home().join("profiles.json")
}

pub fn settings() -> PathBuf {
    home().join("settings.json")
}

/// What pansy wrote into each agent's config, and what was there before.
pub fn agents() -> PathBuf {
    home().join("agents.json")
}

/// Copies of agents' configs as they were before pansy first changed them.
pub fn backups() -> PathBuf {
    home().join("backups")
}

/// Model lists fetched from providers.
pub fn models_cache() -> PathBuf {
    home().join("cache").join("models.json")
}

pub fn logs() -> PathBuf {
    home().join("logs")
}

// --- Plugins (pansy-plugins) -----------------------------------------------------

/// The plugins installed, in their order, each on or off with its options:
/// magpie's `plugins.json`, in its shape (private: options may hold keys).
pub fn plugins() -> PathBuf {
    home().join("plugins.json")
}

/// Where npm plugins are installed (`package.json`, `node_modules`) and the
/// plugin hosts' scripts are written.
pub fn plugins_dir() -> PathBuf {
    home().join("plugins")
}

/// The plugins' sign-ins, in OpenCode's `auth.json` shape (private).
pub fn plugin_auth() -> PathBuf {
    home().join("plugin-auth.json")
}

/// The providers the plugins served when they last listed them, so the
/// gateway starts without waiting for Bun.
pub fn plugin_providers() -> PathBuf {
    home().join("plugin-providers.json")
}

/// The agents the plugins add, as they last loaded.
pub fn agent_plugins() -> PathBuf {
    home().join("agent-plugins.json")
}

/// The key a middleware's account refs are made with (private).
pub fn account_ref_key() -> PathBuf {
    home().join("account-ref.key")
}

/// pi's folder for the pi packages pansy runs (`PI_CODING_AGENT_DIR`),
/// never the user's own `~/.pi`.
pub fn pi() -> PathBuf {
    home().join("pi")
}

/// The Bun pansy downloaded to run plugins.
pub fn bun() -> PathBuf {
    home().join("bun")
}

/// models.dev's catalog, which a plugin's provider is filled in from.
pub fn models_dev() -> PathBuf {
    home().join("cache").join("models-dev.json")
}

/// The community plugins' list, as last fetched.
pub fn plugin_registry() -> PathBuf {
    home().join("cache").join("plugin-registry.json")
}
