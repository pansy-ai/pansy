//! `pansy remote`, `pansy pair` and `pansy devices`: the remote interface
//! the phone connects to (docs/plans/phone.md), from the terminal.

use std::sync::Arc;

use anyhow::{Result, bail};
use pansy_remote::auth::{Auth, Scope};
use pansy_remote::config;
use serde_json::json;

use crate::print_json;

pub(crate) fn command(words: &[&str], json: bool) -> Result<()> {
    match words {
        [] | ["status"] => status(json),
        ["on"] | ["off"] => {
            let mut s = config::load()?;
            s.enabled = words[0] == "on";
            config::save(&s)?;
            if s.enabled {
                println!(
                    "Other devices may control this computer once paired (port {}{}).",
                    s.port,
                    if s.lan {
                        ", this computer, its private networks and tailnets"
                    } else {
                        ", this computer only"
                    }
                );
                println!(
                    "A paired device can do anything here: run agents, commit and push, open a terminal."
                );
                println!(
                    "It starts with the gateway: restart `pansy serve` or the service, then `pansy pair`."
                );
            } else {
                println!(
                    "No other device may control this computer; restart `pansy serve` or the service."
                );
            }
            Ok(())
        }
        ["lan", on @ ("on" | "off")] => {
            let mut s = config::load()?;
            s.lan = *on == "on";
            config::save(&s)?;
            println!(
                "Listening {} from the next start.",
                if s.lan {
                    "on private networks and tailnets too"
                } else {
                    "on this computer only"
                }
            );
            Ok(())
        }
        ["webhook"] => {
            let s = config::load()?;
            println!(
                "{}",
                if s.webhook.is_empty() {
                    "(none)"
                } else {
                    &s.webhook
                }
            );
            Ok(())
        }
        ["webhook", url] => {
            let mut s = config::load()?;
            s.webhook = if *url == "off" {
                String::new()
            } else {
                (*url).to_owned()
            };
            config::save(&s)?;
            println!(
                "{}",
                if s.webhook.is_empty() {
                    "No webhook.".to_owned()
                } else {
                    format!("Threads that wait or end are told to {}.", s.webhook)
                }
            );
            Ok(())
        }
        ["writer", model] => {
            let mut s = config::load()?;
            s.writer_model = if *model == "default" {
                String::new()
            } else {
                (*model).to_owned()
            };
            config::save(&s)?;
            println!(
                "Commit messages and pull requests are written by {}.",
                if s.writer_model.is_empty() {
                    "each thread's own model"
                } else {
                    &s.writer_model
                }
            );
            Ok(())
        }
        _ => bail!(
            "usage: pansy remote [status|on|off|lan on|off|webhook <url>|off|writer <provider/model>|default]"
        ),
    }
}

fn status(json: bool) -> Result<()> {
    let s = config::load()?;
    let auth = Auth::load()?;
    let devices = auth.devices();
    if json {
        return print_json(
            &json!({"settings": s, "machine": auth.machine_id(), "devices": devices}),
        );
    }
    println!(
        "remote: {}",
        if s.enabled {
            "on"
        } else {
            "off (pansy remote on)"
        }
    );
    println!(
        "port: {}{}",
        s.port,
        if s.lan {
            ", private networks and tailnets"
        } else {
            ", this computer only"
        }
    );
    println!("devices: {}", devices.len());
    if !s.webhook.is_empty() {
        println!("webhook: {}", s.webhook);
    }
    Ok(())
}

/// `pansy pair [read]`: a one-time link a device pairs with.
pub(crate) fn pair(words: &[&str]) -> Result<()> {
    let s = config::load()?;
    if !s.enabled {
        bail!("the remote interface is off: pansy remote on");
    }
    let scopes: Option<Vec<Scope>> = match words {
        [] => None,
        ["read"] => Some(vec![
            Scope::AccessRead,
            Scope::OrchestrationRead,
            Scope::FilesystemRead,
            Scope::TerminalRead,
        ]),
        _ => bail!("usage: pansy pair [read]"),
    };
    let auth = Auth::load()?;
    let (secret, expires) =
        auth.create_pairing(scopes.as_deref(), pansy_remote::auth::PAIRING_TTL)?;
    let hosts = pansy_remote::server::pair_hosts(s.port);
    println!(
        "Open on the phone (valid until {}):",
        expires.with_timezone(&chrono::Local).format("%H:%M")
    );
    for host in &hosts {
        println!("  http://{host}/#pair={secret}");
    }
    if hosts.is_empty() {
        println!("  http://127.0.0.1:{}/#pair={secret}", s.port);
    }
    println!("or enter the address and this code in the app: {secret}");
    Ok(())
}

pub(crate) fn devices(words: &[&str], json: bool) -> Result<()> {
    let auth = Auth::load()?;
    match words {
        [] => {
            let devices = auth.devices();
            if json {
                return print_json(&devices);
            }
            if devices.is_empty() {
                println!("No device is paired (pansy pair).");
            }
            for d in devices {
                let pending = if d.approval == pansy_remote::auth::Approval::Pending {
                    format!(
                        "  waits: pansy devices approve {} (code {})",
                        d.id,
                        auth.code_for(&d.id).unwrap_or_default()
                    )
                } else {
                    String::new()
                };
                println!("{}  {} ({}){pending}", d.id, d.name, d.platform);
            }
            Ok(())
        }
        ["revoke", id] => {
            auth.revoke(id)?;
            println!("{id} can't control this computer any more.");
            Ok(())
        }
        ["approve", id] => {
            auth.approve(id)?;
            println!("{id} may control this computer.");
            Ok(())
        }
        ["approval", on @ ("on" | "off")] => {
            auth.set_require_approval(*on == "on")?;
            println!(
                "New devices {} approved here first.",
                if *on == "on" { "are" } else { "aren't" }
            );
            Ok(())
        }
        _ => bail!("usage: pansy devices [revoke <id>|approve <id>|approval on|off]"),
    }
}

/// Runs the remote interface beside the gateway when it is on.
pub(crate) async fn serve(shutdown: impl Future<Output = ()> + Send + 'static) -> Result<()> {
    let settings = config::load()?;
    if !settings.enabled {
        shutdown.await;
        return Ok(());
    }
    let app = Arc::new(pansy_remote::server::App {
        hub: pansy_remote::hub::Hub::load()?,
        auth: Arc::new(Auth::load()?),
        web: pansy_remote::server::web_dir(),
    });
    pansy_remote::server::serve(app, settings, shutdown).await
}
