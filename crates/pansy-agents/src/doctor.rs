//! Diagnostics: whether the agents can reach their models through the
//! gateway. Each finding is a code and its values; the window (Settings ›
//! Diagnostics) and `pansy doctor` put it in words of their own.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use pansy_core::config::Config;
use serde::Serialize;

use crate::state::State;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Ok,
    /// Worth knowing, nothing wrong: an agent put back for now.
    Note,
    /// Something works less well than it could.
    Warn,
    /// Agents' requests fail.
    Fail,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Finding {
    /// What it is about: `gateway`, `providers`, `models`, `files`, or an
    /// agent's id.
    pub about: String,
    pub level: Level,
    /// What was found (`gateway-stopped`).
    pub code: &'static str,
    /// Its values, in the order its words take them: an agent's name first.
    pub args: Vec<String>,
}

fn finding(about: &str, level: Level, code: &'static str, args: &[&str]) -> Finding {
    Finding {
        about: about.to_owned(),
        level,
        code,
        args: args.iter().map(|a| (*a).to_owned()).collect(),
    }
}

/// What answers at the gateway's address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    Pansy,
    /// Something else has the port.
    Other,
    Nothing,
}

/// Asks `url` (`http://127.0.0.1:7267`) for its `/health`.
pub fn probe(url: &str) -> Probe {
    let Some(addr) = url.strip_prefix("http://").and_then(|r| r.parse().ok()) else {
        return Probe::Nothing;
    };
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(400)) else {
        return Probe::Nothing;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(800)));
    let ask = format!("GET /health HTTP/1.0\r\nHost: {addr}\r\n\r\n");
    if stream.write_all(ask.as_bytes()).is_err() {
        return Probe::Other;
    }
    let mut answer = String::new();
    let _ = stream.read_to_string(&mut answer);
    if answer.contains("\"pansy\"") {
        Probe::Pansy
    } else {
        Probe::Other
    }
}

/// A test request's outcome.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Sent {
    pub model: String,
    pub ok: bool,
    /// The gateway's HTTP status; 0: nothing answered at `url` (`said`
    /// empty), or nothing came back in time.
    pub status: u16,
    pub url: String,
    pub ms: u64,
    /// The model's answer, or what went wrong; cut short.
    pub said: String,
}

/// Sends one short message through the gateway, as an agent's request goes
/// (OpenAI Chat, translated where the model's vendor speaks another): to
/// `model`, else the first one an agent can pick. The Routing page shows
/// it as pansy's.
pub fn send(model: Option<&str>) -> anyhow::Result<Sent> {
    let model = match model.map(str::trim).filter(|m| !m.is_empty()) {
        Some(m) => m.to_owned(),
        None => pansy_core::manage::choices()?
            .into_iter()
            .next()
            .map(|c| c.id)
            .ok_or_else(|| anyhow::anyhow!("no model to try yet: add a provider first"))?,
    };
    let url = pansy_core::config::load_settings()?.base_url();
    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": "Reply with the single word OK."}],
        "stream": false,
    })
    .to_string();
    let started = std::time::Instant::now();
    let answered = post(&url, "/v1/chat/completions", &body);
    let ms = started.elapsed().as_millis() as u64;
    let (status, answer) = match answered {
        None => (0, String::new()),
        Some((0, _)) => (0, "no answer within two minutes".to_owned()),
        Some(answered) => answered,
    };
    let json: serde_json::Value = serde_json::from_str(&answer).unwrap_or_default();
    let said = if status == 200 {
        json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .to_owned()
    } else {
        json["error"]["message"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| answer.trim().to_owned())
    };
    Ok(Sent {
        model,
        ok: status == 200,
        status,
        url,
        ms,
        said: said.chars().take(300).collect(),
    })
}

/// POSTs `body` (JSON) to `path` at `url` over HTTP/1.0, which the
/// gateway answers whole and then closes: its status (0 when nothing came
/// back in time) and body; `None` when nothing listens.
fn post(url: &str, path: &str, body: &str) -> Option<(u16, String)> {
    let addr: std::net::SocketAddr = url.strip_prefix("http://")?.parse().ok()?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).ok()?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(120)));
    let ask = format!(
        "POST {path} HTTP/1.0\r\nHost: {addr}\r\nContent-Type: application/json\r\nAuthorization: Bearer {}\r\nx-pansy-agent: pansy\r\nContent-Length: {}\r\n\r\n{body}",
        crate::AGENT_KEY,
        body.len()
    );
    stream.write_all(ask.as_bytes()).ok()?;
    let mut answer = Vec::new();
    let _ = stream.read_to_end(&mut answer);
    let answer = String::from_utf8_lossy(&answer);
    let (head, body) = answer.split_once("\r\n\r\n").unwrap_or((&answer, ""));
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    Some((status, body.to_owned()))
}

/// Every check, the gateway's first.
pub fn run() -> Vec<Finding> {
    let mut out = Vec::new();
    let config = match Config::load() {
        Ok(config) => config,
        Err(e) => {
            out.push(finding(
                "providers",
                Level::Fail,
                "config-unreadable",
                &[&format!("{e:#}")],
            ));
            return out;
        }
    };

    let url = config.settings.base_url();
    out.push(match probe(&url) {
        Probe::Pansy => finding("gateway", Level::Ok, "gateway-running", &[&url]),
        Probe::Other => finding(
            "gateway",
            Level::Fail,
            "gateway-port-taken",
            &[&config.settings.port.to_string()],
        ),
        Probe::Nothing => finding("gateway", Level::Fail, "gateway-stopped", &[&url]),
    });

    let on = config.providers.iter().filter(|p| p.enabled).count();
    let choices: Vec<String> = pansy_core::manage::choices()
        .unwrap_or_default()
        .into_iter()
        .map(|c| c.id)
        .collect();
    if config.providers.is_empty() {
        out.push(finding("providers", Level::Fail, "providers-none", &[]));
    } else if on == 0 {
        out.push(finding("providers", Level::Fail, "providers-off", &[]));
    } else {
        out.push(finding(
            "providers",
            Level::Ok,
            "providers-on",
            &[&on.to_string()],
        ));
        out.push(if choices.is_empty() {
            finding("models", Level::Fail, "models-none", &[])
        } else {
            finding("models", Level::Ok, "models", &[&choices.len().to_string()])
        });
    }

    let served = |model: &str| {
        choices.iter().any(|c| c == model)
            || config.settings.bare_model_ids
                && choices
                    .iter()
                    .any(|c| c.split_once('/').is_some_and(|(_, id)| id == model))
    };
    let state = State::load().unwrap_or_default();
    let mut any = false;
    for agent in crate::agent::all() {
        let v = crate::view(agent, &state);
        let saved = state.agents.get(agent.id);
        if v.connected {
            any = true;
            let model = v.model.clone().unwrap_or_default();
            out.push(if served(&model) {
                finding(
                    agent.id,
                    Level::Ok,
                    "agent-connected",
                    &[agent.name, &model],
                )
            } else {
                finding(
                    agent.id,
                    Level::Warn,
                    "agent-model-gone",
                    &[agent.name, &model],
                )
            });
            if v.cli_missing {
                out.push(finding(
                    agent.id,
                    Level::Warn,
                    "agent-cli-missing",
                    &[agent.name],
                ));
            }
        } else if let Some(model) = &v.paused {
            any = true;
            out.push(finding(
                agent.id,
                Level::Note,
                "agent-paused",
                &[agent.name, model],
            ));
        } else if let Some(model) = saved
            .filter(|s| !s.touched.is_empty())
            .and_then(|s| s.model.as_deref())
        {
            // pansy wrote its keys, and its config points elsewhere now.
            any = true;
            out.push(finding(
                agent.id,
                Level::Warn,
                "agent-drifted",
                &[agent.name, model],
            ));
        }
    }
    if !any {
        out.push(finding("agents", Level::Note, "agents-none", &[]));
    }

    let open = open_files();
    if !open.is_empty() {
        out.push(finding(
            "files",
            Level::Warn,
            "files-open",
            &[&open.join(", ")],
        ));
    }
    out
}

/// pansy's private files others on this computer can read.
fn open_files() -> Vec<String> {
    #[cfg(unix)]
    {
        use pansy_core::paths;
        use std::os::unix::fs::PermissionsExt;
        [
            paths::providers(),
            paths::settings(),
            paths::logins(),
            paths::agents(),
        ]
        .into_iter()
        .filter(|p| std::fs::metadata(p).is_ok_and(|m| m.permissions().mode() & 0o077 != 0))
        .filter_map(|p| Some(p.file_name()?.to_string_lossy().into_owned()))
        .collect()
    }
    #[cfg(not(unix))]
    {
        Vec::new()
    }
}

/// A finding in English: `pansy doctor`'s words.
pub fn say(f: &Finding) -> String {
    let a = |i: usize| f.args.get(i).map(String::as_str).unwrap_or_default();
    match f.code {
        "config-unreadable" => format!("pansy's files can't be read: {}", a(0)),
        "gateway-running" => format!("The gateway answers at {}", a(0)),
        "gateway-port-taken" => format!(
            "Another program has port {}: change pansy's port (pansy settings set port <port>)",
            a(0)
        ),
        "gateway-stopped" => format!(
            "Nothing answers at {}: open pansy or run pansy serve, or put the agents back for now with pansy pause all",
            a(0)
        ),
        "gateway-failed" => format!("The gateway could not start: {}", a(0)),
        "providers-none" => "No provider yet: pansy provider add <preset> <key>".to_owned(),
        "providers-off" => "Every provider is switched off".to_owned(),
        "providers-on" => format!("{} providers on", a(0)),
        "models-none" => "The providers serve no model yet".to_owned(),
        "models" => format!("{} models to pick from", a(0)),
        "agent-connected" => format!("{} is on {}", a(0), a(1)),
        "agent-model-gone" => format!(
            "{} is on {}, which no provider or group serves now: pick another",
            a(0),
            a(1)
        ),
        "agent-cli-missing" => format!("{}'s settings are here, its program isn't", a(0)),
        "agent-paused" => format!(
            "{} uses its own config for now; pansy resume {} connects it again with {}",
            a(0),
            f.about,
            a(1)
        ),
        "agent-drifted" => format!(
            "{}'s config no longer points at this gateway: connect it again with {}, or disconnect it",
            a(0),
            a(1)
        ),
        "agents-none" => {
            "No agent is connected to pansy: pansy test [provider/model] sends a request as one would"
                .to_owned()
        }
        "files-open" => format!("Others on this computer can read {}: chmod 600 them", a(0)),
        other => other.to_owned(),
    }
}
