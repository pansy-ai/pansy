//! magpie's community plugins, run as magpie runs them: on Bun, in two
//! hosts of their own (docs/plans/plugin-host.md).
//!
//! - Host A ([`host`]) runs magpie's `host.js`: OpenCode provider plugins
//!   and pi packages, which sign in, list models and send requests, and the
//!   asynchronous request hooks (`magpie.hooks`).
//! - Host B ([`middleware`], [`agents`]) runs pansy's `mw-host.js`: the
//!   gateway middleware (`magpie.middleware`) on every request and streamed
//!   event, and the agents plugins add (`magpie.agent`).
//!
//! Which plugins are installed, in their order, on or off, is
//! `plugins.json` ([`store`]); Bun is downloaded the first time a plugin
//! needs it ([`bun`]). Every file is under `pansy_core::paths`.
//!
//! The async functions may be awaited on any Tokio runtime: the hosts and
//! their I/O live on this crate's own ([`rt`]), so a host started from a
//! command's runtime outlives it. Sync callers use [`block`].

pub mod agents;
pub mod bun;
pub mod host;
pub mod import;
pub mod kind;
pub mod market;
pub mod middleware;
pub mod mirror;
mod rpc;
pub mod signin;
pub mod store;

use std::future::Future;
use std::sync::OnceLock;

/// The runtime the hosts run on.
pub fn rt() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("pansy-plugins")
            .enable_all()
            .build()
            .expect("the plugins' runtime")
    })
}

/// Runs `fut` on [`rt`] and waits for it: for callers that aren't async
/// (the agents' wiring, the window's native calls). Safe on any thread,
/// a runtime's included, which it blocks meanwhile.
pub fn block<F>(fut: F) -> F::Output
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let task = rt().spawn(fut);
    let (tx, rx) = std::sync::mpsc::channel();
    rt().spawn(async move {
        let _ = tx.send(task.await);
    });
    match rx.recv().expect("the plugins' runtime went away") {
        Ok(out) => out,
        // A panic in it is the caller's.
        Err(e) if e.is_panic() => std::panic::resume_unwind(e.into_panic()),
        Err(e) => panic!("a plugins' task was cancelled: {e}"),
    }
}

/// Runs `fut` on [`rt`], from an async caller on any runtime.
pub(crate) async fn on_rt<F>(fut: F) -> F::Output
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    rt().spawn(fut).await.expect("a plugins' task panicked")
}

/// Makes `dir` (and what leads to it) only the user may look into: the
/// plugins' code and Bun run with their permissions.
pub(crate) fn private_dir(dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// The scripts embedded, written where Bun can run them.
pub(crate) mod scripts {
    use std::path::PathBuf;

    use anyhow::Result;
    use sha2::{Digest, Sha256};

    pub const HOST: &str = include_str!("../host/host.js");
    pub const PI: &str = include_str!("../host/pi.js");
    pub const MW: &str = include_str!("../host/mw-host.js");

    /// `js` written under `plugins/`, named by what it holds.
    pub fn write(name: &str, js: &str) -> Result<PathBuf> {
        let sum = Sha256::digest(js.as_bytes());
        let hex: String = sum[..6].iter().map(|b| format!("{b:02x}")).collect();
        let dir = pansy_core::paths::plugins_dir();
        let path = dir.join(format!("{name}-{hex}.js"));
        if std::fs::read_to_string(&path).is_ok_and(|s| s == js) {
            return Ok(path);
        }
        crate::private_dir(&dir)?;
        pansy_core::fsutil::write_atomic(&path, js.as_bytes(), false)?;
        Ok(path)
    }
}

#[cfg(test)]
pub(crate) mod testing;

#[cfg(test)]
mod tests;
