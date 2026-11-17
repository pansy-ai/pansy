//! What the tests share: a scratch home, and Bun when there is one.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

static ENV: Mutex<()> = Mutex::new(());

/// Points pansy's home and the user's at `dir` for as long as the guard is
/// held: tests that read the environment, or start a host, take turns.
pub fn scratch(dir: &Path) -> MutexGuard<'static, ()> {
    let guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: every test of this crate that reads these holds the guard.
    unsafe {
        std::env::set_var("HOME", dir);
        std::env::set_var("PANSY_HOME", dir.join(".config/pansy"));
        std::env::remove_var("XDG_CONFIG_HOME");
    }
    crate::host::forget();
    crate::middleware::forget();
    guard
}

/// The Bun tests run plugins on: `PANSY_BUN`, else one on `PATH`. A test
/// that needs it says so and passes without one.
pub fn bun() -> Option<PathBuf> {
    if let Some(b) = std::env::var_os("PANSY_BUN").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(b));
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(if cfg!(windows) { "bun.exe" } else { "bun" }))
        .find(|p| p.is_file())
        .inspect(|p| {
            // SAFETY: the tests that start hosts hold the ENV guard.
            unsafe { std::env::set_var("PANSY_BUN", p) };
        })
}

/// A fixture plugin's folder (`tests/fixtures/<name>`).
pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
