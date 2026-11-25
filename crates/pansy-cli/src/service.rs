//! `pansy service`: the gateway as a service of the user's, started when
//! they log in and kept running without a window (roadmap iteration 2, which
//! the phone needs first: an agent it drives outlives the window).
//!
//! A systemd user unit on Linux (`~/.config/systemd/user/pansy.service`,
//! lingering so it runs after logging out), a launchd agent on macOS
//! (`~/Library/LaunchAgents/dev.pansy.serve.plist`) and a task at logon on
//! Windows. Each runs `<this program> serve` with the `PATH` it was installed
//! from, so the agents it starts are found as in the user's terminal, and
//! with `PANSY_HOME` when that is set. The window leaves the port to it
//! (`desktop/native`'s `start_gateway`).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use pansy_core::paths;

/// launchd's label, and Windows' task name.
const LABEL: &str = "dev.pansy.serve";
#[cfg(windows)]
const TASK: &str = "pansy serve";

/// What `pansy service install` puts down, before it is put down.
pub struct Plan {
    pub program: PathBuf,
    pub path: String,
    pub pansy_home: Option<String>,
}

impl Plan {
    /// This program, the `PATH` and `PANSY_HOME` it runs with now.
    pub fn here() -> Result<Plan> {
        Ok(Plan {
            program: std::env::current_exe().context("finding this program")?,
            path: std::env::var("PATH").unwrap_or_default(),
            pansy_home: std::env::var("PANSY_HOME").ok().filter(|h| !h.is_empty()),
        })
    }

    /// The systemd user unit.
    pub fn systemd_unit(&self) -> String {
        let mut env = format!("Environment=\"PATH={}\"\n", systemd_escape(&self.path));
        if let Some(home) = &self.pansy_home {
            env += &format!("Environment=\"PANSY_HOME={}\"\n", systemd_escape(home));
        }
        format!(
            "# Written by `pansy service install`; `pansy service uninstall` removes it.\n\
             [Unit]\n\
             Description=pansy, the gateway every agent's model goes through\n\
             After=network-online.target\n\
             \n\
             [Service]\n\
             ExecStart=\"{}\" serve\n\
             {env}\
             Restart=on-failure\n\
             RestartSec=3\n\
             \n\
             [Install]\n\
             WantedBy=default.target\n",
            systemd_escape(&self.program.to_string_lossy()),
        )
    }

    /// The launchd agent's property list.
    pub fn launchd_plist(&self, log: &Path) -> String {
        let mut env = format!(
            "    <key>PATH</key><string>{}</string>\n",
            xml_escape(&self.path)
        );
        if let Some(home) = &self.pansy_home {
            env += &format!(
                "    <key>PANSY_HOME</key><string>{}</string>\n",
                xml_escape(home)
            );
        }
        let log = xml_escape(&log.to_string_lossy());
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <!-- Written by `pansy service install`; `pansy service uninstall` removes it. -->\n\
             <plist version=\"1.0\">\n\
             <dict>\n\
             \x20 <key>Label</key><string>{LABEL}</string>\n\
             \x20 <key>ProgramArguments</key>\n\
             \x20 <array><string>{}</string><string>serve</string></array>\n\
             \x20 <key>EnvironmentVariables</key>\n\
             \x20 <dict>\n{env}  </dict>\n\
             \x20 <key>RunAtLoad</key><true/>\n\
             \x20 <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>\n\
             \x20 <key>StandardOutPath</key><string>{log}</string>\n\
             \x20 <key>StandardErrorPath</key><string>{log}</string>\n\
             </dict>\n\
             </plist>\n",
            xml_escape(&self.program.to_string_lossy()),
        )
    }
}

/// Where the service's file is on this system (none on Windows, whose task
/// lives in the Task Scheduler).
pub fn file() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        Some(
            paths::user_home()
                .join("Library/LaunchAgents")
                .join(format!("{LABEL}.plist")),
        )
    } else if cfg!(windows) {
        None
    } else {
        Some(paths::user_home().join(".config/systemd/user/pansy.service"))
    }
}

/// Whether it is installed, and whether a pansy answers on the gateway's
/// port now (the service's, or a window's).
pub struct Status {
    pub installed: bool,
    pub file: Option<PathBuf>,
    pub answering: bool,
    pub url: String,
}

pub fn status() -> Result<Status> {
    let url = pansy_core::config::load_settings()?.base_url();
    let file = file();
    let installed = match &file {
        Some(f) => f.exists(),
        None => windows_task_exists(),
    };
    let answering = pansy_agents::doctor::probe(&url) == pansy_agents::doctor::Probe::Pansy;
    Ok(Status {
        installed,
        file,
        answering,
        url,
    })
}

/// Writes the service and starts it. What it ran is returned, a line each.
pub fn install() -> Result<Vec<String>> {
    let plan = Plan::here()?;
    let mut said = Vec::new();
    if cfg!(target_os = "macos") {
        let file = file().expect("macOS keeps a file");
        let log = paths::logs().join("serve.log");
        std::fs::create_dir_all(paths::logs())?;
        write(&file, &plan.launchd_plist(&log))?;
        let domain = format!("gui/{}", uid());
        // Loaded already: unload first, so the new file is read.
        let _ = Command::new("launchctl")
            .args(["bootout", &domain, &file.to_string_lossy()])
            .output();
        run(
            "launchctl",
            &["bootstrap", &domain, &file.to_string_lossy()],
        )?;
        said.push(format!("wrote {}", file.display()));
        said.push(format!(
            "started it (launchctl bootstrap {domain}); its log is {}",
            log.display()
        ));
    } else if cfg!(windows) {
        let command = format!("\"{}\" serve", plan.program.display());
        windows_install(&command)?;
        said.push(format!("added the task \"{}\" at logon", windows_task()));
        said.push("started it".into());
    } else {
        let file = file().expect("Linux keeps a file");
        write(&file, &plan.systemd_unit())?;
        said.push(format!("wrote {}", file.display()));
        run("systemctl", &["--user", "daemon-reload"])?;
        run("systemctl", &["--user", "enable", "--now", "pansy.service"])?;
        said.push("started it (systemctl --user enable --now pansy.service)".into());
        // Without lingering a user's services stop when they log out.
        let user = std::env::var("USER").unwrap_or_default();
        match Command::new("loginctl").args(["enable-linger", &user]).output() {
            Ok(o) if o.status.success() => said.push("it keeps running after you log out (loginctl enable-linger)".into()),
            _ => said.push(format!(
                "it stops when you log out: `loginctl enable-linger {user}` (it may ask for your password) keeps it running"
            )),
        }
    }
    Ok(said)
}

/// Stops the service and removes it.
pub fn uninstall() -> Result<Vec<String>> {
    let mut said = Vec::new();
    if cfg!(target_os = "macos") {
        let file = file().expect("macOS keeps a file");
        let _ = Command::new("launchctl")
            .args([
                "bootout",
                &format!("gui/{}", uid()),
                &file.to_string_lossy(),
            ])
            .output();
        if file.exists() {
            std::fs::remove_file(&file)?;
            said.push(format!("stopped it and removed {}", file.display()));
        }
    } else if cfg!(windows) {
        if windows_task_exists() {
            let _ = Command::new("schtasks")
                .args(["/End", "/TN", windows_task()])
                .output();
            run("schtasks", &["/Delete", "/TN", windows_task(), "/F"])?;
            said.push(format!(
                "stopped and removed the task \"{}\"",
                windows_task()
            ));
        }
    } else {
        let file = file().expect("Linux keeps a file");
        if file.exists() {
            let _ = Command::new("systemctl")
                .args(["--user", "disable", "--now", "pansy.service"])
                .output();
            std::fs::remove_file(&file)?;
            let _ = Command::new("systemctl")
                .args(["--user", "daemon-reload"])
                .output();
            said.push(format!("stopped it and removed {}", file.display()));
        }
    }
    if said.is_empty() {
        said.push("pansy wasn't installed as a service".into());
    }
    Ok(said)
}

fn write(file: &Path, text: &str) -> Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(file, text).with_context(|| format!("writing {}", file.display()))
}

fn run(program: &str, args: &[&str]) -> Result<()> {
    let out = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("running {program}"))?;
    if !out.status.success() {
        bail!(
            "{program} {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// systemd's quoted strings: a backslash, a quote and `%` (its specifiers)
/// written so they stay themselves.
fn systemd_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%")
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(unix)]
fn uid() -> u32 {
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

#[cfg(not(unix))]
fn uid() -> u32 {
    0
}

#[cfg(windows)]
fn windows_task() -> &'static str {
    TASK
}

#[cfg(not(windows))]
fn windows_task() -> &'static str {
    ""
}

#[cfg(windows)]
fn windows_task_exists() -> bool {
    Command::new("schtasks")
        .args(["/Query", "/TN", TASK])
        .output()
        .is_ok_and(|o| o.status.success())
}

#[cfg(not(windows))]
fn windows_task_exists() -> bool {
    false
}

#[cfg(windows)]
fn windows_install(command: &str) -> Result<()> {
    run(
        "schtasks",
        &[
            "/Create", "/TN", TASK, "/TR", command, "/SC", "ONLOGON", "/RL", "LIMITED", "/F",
        ],
    )?;
    run("schtasks", &["/Run", "/TN", TASK])
}

#[cfg(not(windows))]
fn windows_install(_command: &str) -> Result<()> {
    unreachable!("Windows only")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> Plan {
        Plan {
            program: PathBuf::from("/opt/pansy 1/pansy"),
            path: "/usr/bin:/home/a/.local/share/mise/shims:/odd%dir".into(),
            pansy_home: Some("/tmp/scratch \"home\"".into()),
        }
    }

    #[test]
    fn the_systemd_unit_runs_serve_with_the_path_it_was_installed_from() {
        let unit = plan().systemd_unit();
        assert!(
            unit.contains("ExecStart=\"/opt/pansy 1/pansy\" serve\n"),
            "{unit}"
        );
        assert!(
            unit.contains(
                "Environment=\"PATH=/usr/bin:/home/a/.local/share/mise/shims:/odd%%dir\"\n"
            ),
            "{unit}"
        );
        assert!(
            unit.contains("Environment=\"PANSY_HOME=/tmp/scratch \\\"home\\\"\"\n"),
            "{unit}"
        );
        assert!(unit.contains("WantedBy=default.target"));
        let without = Plan {
            pansy_home: None,
            ..plan()
        }
        .systemd_unit();
        assert!(!without.contains("PANSY_HOME"));
    }

    #[test]
    fn the_launchd_plist_is_escaped_and_logs_to_pansys_logs() {
        let plist = plan().launchd_plist(Path::new("/home/a/.config/pansy/logs/serve.log"));
        assert!(plist.contains("<string>dev.pansy.serve</string>"));
        assert!(
            plist.contains(
                "<array><string>/opt/pansy 1/pansy</string><string>serve</string></array>"
            )
        );
        assert!(
            plist.contains("<key>PANSY_HOME</key><string>/tmp/scratch &quot;home&quot;</string>")
        );
        assert!(plist.contains(
            "<key>StandardErrorPath</key><string>/home/a/.config/pansy/logs/serve.log</string>"
        ));
    }

    #[test]
    fn the_file_is_under_home() {
        let Some(file) = file() else { return };
        assert!(file.starts_with(paths::user_home()), "{}", file.display());
    }
}
