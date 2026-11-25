//! This computer stays awake while a turn runs: one asleep stops answering
//! the phone and stops the agent mid-turn. While any [`Awake`] is held,
//! idle sleep is blocked. Closing the lid still sleeps (on Linux as long as
//! logind's `LidSwitchIgnoreInhibited` is on, its default), and on Linux a
//! Suspend chosen by hand says what blocks it first.
//!
//! On macOS and Linux the block is a process of the system's own
//! (`caffeinate`, `systemd-inhibit`) running `cat` on a pipe from pansy:
//! letting it go closes the pipe, and so does pansy ending, however it
//! ends, so nothing stays awake after it. On Windows it is the execution
//! state of a thread of pansy's own.

use std::sync::OnceLock;
use std::sync::mpsc::{Sender, channel};

/// Held while a turn runs: the first one blocks sleep, the last one's drop
/// lets it go.
pub struct Awake(());

impl Awake {
    pub fn hold() -> Awake {
        tell(true);
        Awake(())
    }
}

impl Drop for Awake {
    fn drop(&mut self) {
        tell(false);
    }
}

/// Counted on a thread of its own, which may wait on the system's commands.
fn tell(up: bool) {
    static KEEPER: OnceLock<Sender<bool>> = OnceLock::new();
    let keeper = KEEPER.get_or_init(|| {
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new()
            .name("pansy-awake".into())
            .spawn(move || keep(rx.into_iter()));
        if let Err(e) = spawned {
            tracing::warn!("keeping this computer awake: {e}");
        }
        tx
    });
    let _ = keeper.send(up);
}

fn keep(ups: impl Iterator<Item = bool>) {
    let mut count = 0usize;
    let mut held: Option<sys::Block> = None;
    let mut warned = false;
    for up in ups {
        count = if up {
            count + 1
        } else {
            count.saturating_sub(1)
        };
        if count > 0 && held.is_none() {
            held = sys::block();
            if held.is_none() && !warned {
                tracing::warn!("this computer may sleep while a turn runs: nothing here blocks it");
                warned = true;
            }
        } else if count == 0 {
            held = None;
        }
    }
}

/// The commands that block sleep while they run, strongest first: one
/// refused (by polkit, say) gives way to the next.
#[cfg_attr(windows, allow(dead_code))]
fn commands(os: &str, has: impl Fn(&str) -> bool) -> Vec<Vec<String>> {
    let words = |w: &[&str]| w.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    match os {
        // -i: idle sleep alone; the display may still sleep
        "macos" => vec![words(&["/usr/bin/caffeinate", "-i", "/bin/cat"])],
        "linux" => {
            let Some(inhibit) = ["/usr/bin/systemd-inhibit", "/bin/systemd-inhibit"]
                .into_iter()
                .find(|p| has(p))
            else {
                return Vec::new();
            };
            ["sleep:idle", "idle"]
                .into_iter()
                .map(|what| {
                    words(&[
                        inhibit,
                        &format!("--what={what}"),
                        "--who=pansy",
                        "--why=An agent is working for another device",
                        "--mode=block",
                        "cat",
                    ])
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

#[cfg(not(windows))]
mod sys {
    use std::process::{Child, Command, Stdio};
    use std::time::Duration;

    /// The command blocking sleep; dropped, it ends.
    pub struct Block(Child);

    impl Drop for Block {
        fn drop(&mut self) {
            // `cat` ends on the pipe's end; the blocker goes with it, or now
            drop(self.0.stdin.take());
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    pub fn block() -> Option<Block> {
        let has = |p: &str| std::path::Path::new(p).exists();
        first_running(super::commands(std::env::consts::OS, has))
    }

    pub(super) fn first_running(cmds: Vec<Vec<String>>) -> Option<Block> {
        for cmd in cmds {
            let Ok(child) = Command::new(&cmd[0])
                .args(&cmd[1..])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            else {
                continue;
            };
            let mut block = Block(child);
            // a refused block ends at once
            std::thread::sleep(Duration::from_millis(200));
            if matches!(block.0.try_wait(), Ok(None)) {
                return Some(block);
            }
        }
        None
    }

    #[cfg(test)]
    pub(super) fn id(block: &Block) -> u32 {
        block.0.id()
    }
}

#[cfg(windows)]
mod sys {
    use windows_sys::Win32::System::Power::{
        ES_CONTINUOUS, ES_SYSTEM_REQUIRED, SetThreadExecutionState,
    };

    /// The keeper thread's execution state; dropped (on that thread), back
    /// to what the system decides.
    pub struct Block(());

    impl Drop for Block {
        fn drop(&mut self) {
            unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
        }
    }

    pub fn block() -> Option<Block> {
        let ok = unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) } != 0;
        ok.then_some(Block(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_idle_sleep_with_the_systems_own_commands() {
        let mac = commands("macos", |_| false);
        assert_eq!(mac, vec![vec!["/usr/bin/caffeinate", "-i", "/bin/cat"]]);

        let linux = commands("linux", |p| p == "/usr/bin/systemd-inhibit");
        assert_eq!(linux.len(), 2);
        assert_eq!(linux[0][0], "/usr/bin/systemd-inhibit");
        assert_eq!(linux[0][1], "--what=sleep:idle");
        assert_eq!(linux[1][1], "--what=idle");
        assert!(linux.iter().all(|c| c.last().unwrap() == "cat"));

        assert!(commands("linux", |_| false).is_empty());
        assert!(commands("freebsd", |_| true).is_empty());
    }

    /// A refused blocker gives way to the next; the one held ends when it
    /// is let go.
    #[cfg(unix)]
    #[test]
    fn holds_the_first_blocker_that_runs_and_ends_it_when_let_go() {
        let words = |w: &[&str]| w.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        let held = sys::first_running(vec![
            words(&["/nonexistent/blocker"]),
            words(&["sh", "-c", "exit 1"]),
            words(&["cat"]),
        ])
        .expect("cat runs until its pipe ends");
        let pid = sys::id(&held).to_string();
        let alive = || {
            std::process::Command::new("kill")
                .args(["-0", &pid])
                .status()
                .is_ok_and(|s| s.success())
        };
        assert!(alive());
        drop(held);
        assert!(!alive());

        assert!(sys::first_running(vec![words(&["sh", "-c", "exit 1"])]).is_none());
    }
}
