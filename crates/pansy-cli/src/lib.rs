//! The `pansy` command: providers, agents, groups and profiles from the
//! terminal, and the gateway without a window (`pansy serve`).
//!
//! ```text
//! pansy provider add deepseek sk-…            a preset needs only the key
//! pansy claude deepseek/deepseek-chat         Claude Code on DeepSeek
//! pansy codex moonshot/kimi-k2-0905-preview   Codex on Kimi
//! pansy group add "Daily" models=a/x,b/y      a routing group
//! pansy login codex                           a ChatGPT subscription's models
//! pansy save work && pansy use work           profiles
//! pansy serve                                 the gateway, in the foreground
//! ```

use std::process::ExitCode;

use anyhow::{Result, anyhow, bail};
use pansy_agents::profile;
use pansy_core::config::{self, Config};
use pansy_core::group::Routing;
use pansy_core::manage;
use pansy_core::presets::{Kind, PRESETS};
use pansy_core::provider::{AuthHeader, Protocol, mask_key};
use pansy_core::subscription;
use pansy_core::usage;
use serde_json::json;

mod plugins;

const HELP: &str = "pansy — every agent's model, one place

Usage: pansy <command> [arguments]

Agents
  agents                              the agents on this computer and their models
  <agent> <provider/model>            point an agent at the gateway with a model
                                      (claude, codex, gemini, opencode, qwen, crush)
  <agent> --small <provider/model>    the agent's quick-task model (Claude Code, OpenCode, Crush)
  <agent> off                         put the agent's config back as it was
  connect <agent> <provider/model> | disconnect <agent>
                                      the same, by the agent's name
  models                              every model an agent can be given
  pause <agent>|all                   put an agent's config back for now, so it works
                                      without pansy; its model is remembered
  resume <agent>|all                  connect it again with that model
  doctor                              whether the gateway, providers and agents work
  test [provider/model]               send one short message through the gateway, as an
                                      agent would (the first model without one)

Providers
  provider presets                    the vendors pansy knows, with their plans and regions
  provider add <preset> [key] [label=…] [region=…] [address=…]
                                      add a key (a local server needs none); a plan or
                                      region, a server's address or Azure's endpoint
  provider region <id> <region>       move a provider to another of its plans or regions
  provider address <id> <address>     move a server of your own, or Azure's endpoint
  provider key-header <id> both|x-api-key|bearer
                                      the header its key goes in on the Anthropic API
  provider move <id> <place>          the order providers are tried in for a bare model id
  provider add-custom <name> url=<base URL> [protocol=chat|anthropic|responses|gemini]
                                      [key=…] [models=a,b]
  provider list | providers           providers, their keys and models
  provider remove <id>
  provider on|off <id>
  provider key add <id> <key> [label=…]
  provider key remove <id> <key id>
  provider key on|off <id> <key id>
  provider models <id> [--refresh]    a provider's models, fetched again with --refresh
  provider model add|remove <id> <model>

Import (the providers magpie, CC Switch, Alma, Claude Code, Codex and OpenCode have set
up, read only)
  import                              what each app has, and whether pansy has it
  import <app> [ref…] [as=add|key|replace]
                                      bring an app's providers over (without refs, the ones
                                      it offers); as: a provider of its own, one more key of
                                      the one at its address, or in place of the one at its id
  import [-y] '<link>'                add the provider a pansy://import?… (or magpie://)
                                      link describes, after showing it

Subscriptions (ChatGPT as Codex uses it, GitHub Copilot, Gemini Code Assist)
  accounts                            the subscriptions' accounts: Codex's, Copilot's and
                                      Gemini CLI's own sign-ins, read where they keep them,
                                      and pansy's
  login codex [--code]                sign in to a ChatGPT account in the browser, or by a
                                      code typed on ChatGPT's page
  login copilot                       sign in to a GitHub account by a code
  login gemini                        sign in to a Google account in the browser
  limits                              what each account has used of its allowance, and
                                      when each window renews
  logout <subscription> <account id>  sign out an account signed in from pansy
  accounts project gemini-cli <account id> [project]
                                      the Google Cloud project a Code Assist Standard or
                                      Enterprise account bills; none: the one Google gives
                                      (provider key on|off codex <id> switches one off)

Plugins (magpie's community plugins: OpenCode and pi providers, middleware, agents; on Bun)
  plugin [list] | plugins             the plugins added, what each is, and the providers
                                      they sign in to
  plugin discover                     the community plugins: OpenCode plugins, pi
                                      packages, middleware, agents
  plugin search <words>               OpenCode plugins and pi packages on npm
  plugin add <name|npm spec|github:owner/repo|path>
                                      add one (a community plugin by its name: workbuddy)
  plugin rm|on|off <plugin>           remove one, or switch it on or off
  plugin update [plugin]              update one, or every one
  plugin options <plugin> [json]      a plugin's options; set them with a JSON object
  plugin use <plugin> <provider>      the plugin that serves a provider two sign in to
  plugin refresh                      ask the plugins for their providers and models again
  plugin login <provider> [method]    sign in to a plugin's provider (a page, a code or a
                                      key, as the plugin asks)
  plugin logout <provider> [account]
  plugin import magpie                magpie's plugins, installed again here, and their
                                      sign-ins

Routing groups
  group list | groups
  group add <name> models=<a/x,b/y> [routing=order|rotate|usage|<plugin>/<name>]
  group set <id> [name=…] [models=…] [routing=…]
  group remove <id>

Sessions (Claude Code's, Codex's and OpenCode's, read from their files)
  sessions [agent]                    the sessions on this computer, the latest first
  sessions show <id> [sub=<id>]       what was said in one (or in its subagent); an id's
                                      first characters are enough
  sessions find <words>               the sessions whose prompts or replies say them
  sessions delete <id>                move a session's files into pansy's trash
  sessions trash                      the deleted sessions
  sessions restore <key> | sessions purge <key> | sessions trash empty

Profiles
  save <name>                         every agent's model under one name
  use <name>                          put the agents back on a profile
  profiles                            the saved profiles
  profile remove <name>

Gateway
  serve                               run the gateway in the foreground
  status                              the gateway, providers and agents at a glance
  usage [days=7]                      requests and tokens through the gateway
  settings                            every setting
  settings get <key> | set <key> <value>

Masking
  mask <text>                         the text as a vendor would get it (standard input
                                      without one); kept nowhere
  masking [days=7]                    what masking kept from vendors, by kind and request
  masking clear                       remove masking's records

  version | help

Lists print as text; add --json for JSON.";

/// Short names for the agents on the command line.
const AGENT_ALIASES: &[(&str, &str)] = &[
    ("claude", "claude-code"),
    ("claude-code", "claude-code"),
    ("codex", "codex"),
    ("gemini", "gemini-cli"),
    ("gemini-cli", "gemini-cli"),
    ("opencode", "opencode"),
    ("qwen", "qwen-code"),
    ("qwen-code", "qwen-code"),
    ("crush", "crush"),
];

/// Runs the command `args` names; its exit code.
pub fn run(args: Vec<String>) -> ExitCode {
    // `pansy models | head` ends quietly, as other commands do, when the
    // reader goes away.
    #[cfg(unix)]
    // SAFETY: resets one signal's disposition to the default; nothing
    // else runs yet.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let json = args.iter().any(|a| a == "--json");
    let args: Vec<String> = args.into_iter().filter(|a| a != "--json").collect();
    match dispatch(&args, json) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("pansy: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// The options commands take as `name=value`. Any other word with an `=`
/// in it is a word of its own: a key ending in `==`.
const OPTIONS: &[&str] = &[
    "label", "url", "protocol", "key", "models", "routing", "name", "days", "region", "plan",
    "address", "endpoint", "sub",
];

/// `name=value` arguments, and the rest in order.
fn split_options(args: &[String]) -> (Vec<String>, std::collections::BTreeMap<String, String>) {
    let mut plain = Vec::new();
    let mut options = std::collections::BTreeMap::new();
    for arg in args {
        match arg.split_once('=') {
            Some((k, v)) if OPTIONS.contains(&k) => {
                options.insert(k.to_owned(), v.to_owned());
            }
            _ => plain.push(arg.clone()),
        }
    }
    (plain, options)
}

fn list_of(text: &str) -> Vec<String> {
    text.split([',', ' '])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

fn runtime() -> Result<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?)
}

pub(crate) fn print_json(value: &impl serde::Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn dispatch(args: &[String], json: bool) -> Result<()> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        [] | ["help"] | ["-h"] | ["--help"] => {
            println!("{HELP}");
            Ok(())
        }
        ["version"] | ["-V"] | ["--version"] => {
            println!("pansy {}", pansy_core::VERSION);
            Ok(())
        }
        ["serve", ..] => serve(),
        ["status"] => status(json),
        ["agents"] => agents(json),
        ["models"] => models(json),
        ["provider", rest @ ..] => provider(rest, &args[1..], json),
        ["providers"] => provider(&["list"], &[], json),
        ["group", rest @ ..] => group(rest, &args[1..], json),
        ["groups"] => group(&["list"], &[], json),
        ["save", name] => {
            profile::save(name)?;
            println!("Saved the agents' models as {name}.");
            Ok(())
        }
        ["use", name] => {
            let problems = profile::apply(name)?;
            if problems.is_empty() {
                println!("The agents are on {name}.");
            } else {
                println!("The agents are on {name}, except:");
                for p in problems {
                    println!("  {p}");
                }
            }
            Ok(())
        }
        ["profiles"] => profiles(json),
        ["accounts"] => accounts(json),
        ["limits"] | ["allowances"] => limits(json),
        ["accounts", "project", kind, account, rest @ ..] if rest.len() <= 1 => {
            let kind = subscription_kind(kind)?;
            let project = rest.first().copied().unwrap_or_default();
            subscription::set_project(kind, account, project)?;
            if project.is_empty() {
                println!(
                    "{account} of {} bills the project Google gives it.",
                    kind.name()
                );
            } else {
                println!("{account} of {} bills {project}.", kind.name());
            }
            Ok(())
        }
        ["login", kind] => login(kind, false, json),
        ["login", kind, "--code"] | ["login", "--code", kind] => login(kind, true, json),
        ["logout", kind, account] => {
            let kind = subscription_kind(kind)?;
            subscription::sign_out(kind, account)?;
            println!("Signed {account} out of {}.", kind.name());
            Ok(())
        }
        ["sessions", ..] => sessions(&args[1..], json),
        ["import", ..] => import(&args[1..], json),
        ["plugin", rest @ ..] => plugins::command(rest, json),
        ["plugins"] => plugins::command(&["list"], json),
        ["profile", "remove", name] => {
            profile::remove(name)?;
            println!("Removed the profile {name}.");
            Ok(())
        }
        ["connect", agent, model] => connect(agent, model),
        ["disconnect", agent] => disconnect(agent),
        ["pause", "all"] => {
            match pansy_agents::pause_all()? {
                0 => println!("No agent is connected to pansy."),
                n => println!(
                    "{n} agent(s) use their own configs for now; `pansy resume all` connects them again."
                ),
            }
            Ok(())
        }
        ["pause", agent] => {
            let id = agent_id(agent);
            pansy_agents::pause(id)?;
            let name = pansy_agents::agent::find(id).map(|a| a.name).unwrap_or(id);
            println!(
                "{name} uses its own config for now; `pansy resume {agent}` connects it again."
            );
            Ok(())
        }
        ["resume", "all"] => {
            match pansy_agents::resume_all()? {
                0 => println!("No agent was put back for now."),
                n => println!("{n} agent(s) back on pansy."),
            }
            Ok(())
        }
        ["resume", agent] => {
            let id = agent_id(agent);
            pansy_agents::resume(id)?;
            let name = pansy_agents::agent::find(id).map(|a| a.name).unwrap_or(id);
            println!("{name} is back on pansy.");
            Ok(())
        }
        ["doctor"] => doctor(json),
        ["test"] => send_test(None, json),
        ["test", model] => send_test(Some(model), json),
        ["usage", ..] => usage_report(&args[1..], json),
        ["mask", ..] => mask_try(&args[1..], json),
        ["masking", ..] => masking_report(&args[1..], json),
        ["settings"] => {
            let mut settings = config::load_settings()?;
            // Shown on its own only: `pansy settings get gatewayKey`.
            if !settings.gateway_key.is_empty() {
                settings.gateway_key = mask_key(&settings.gateway_key);
            }
            if json {
                return print_json(&settings);
            }
            for (key, what) in pansy_core::settings::Settings::KEYS {
                let mut value = settings.get(key).unwrap_or_default();
                if value.chars().count() > 28 {
                    value = value.chars().take(27).collect::<String>() + "…";
                }
                println!("{key:22} {value:28} {what}");
            }
            Ok(())
        }
        ["settings", "get", key] => {
            let settings = config::load_settings()?;
            println!(
                "{}",
                settings
                    .get(key)
                    .ok_or_else(|| anyhow!("no setting named {key}"))?
            );
            Ok(())
        }
        ["settings", "set", key, value] => {
            config::update_settings(|s| s.set(key, value).map_err(|e| anyhow!(e)))?;
            if *key == "port" {
                let problems = pansy_agents::reconnect_all()?;
                for p in problems {
                    eprintln!("pansy: {p}");
                }
                println!(
                    "Set {key} to {value}; the connected agents use the new port. Restart the gateway to listen on it."
                );
            } else if *key == "lan" {
                println!(
                    "Set {key} to {value}. Restart the gateway (or the window) for it to take effect."
                );
            } else if *key == "gatewayKey" {
                println!("Set the gateway key.");
            } else {
                println!("Set {key} to {value}.");
            }
            Ok(())
        }
        [agent, rest @ ..] => match AGENT_ALIASES.iter().find(|(alias, _)| alias == agent) {
            Some((_, id)) => match rest {
                [] => {
                    let v = pansy_agents::list()
                        .into_iter()
                        .find(|v| v.id == *id)
                        .unwrap();
                    println!(
                        "{}: {}",
                        v.name,
                        v.model.as_deref().unwrap_or("not connected to pansy")
                    );
                    Ok(())
                }
                ["off"] => disconnect(id),
                ["--small", model] => {
                    let agent =
                        pansy_agents::agent::find(id).ok_or_else(|| anyhow!("no agent {id}"))?;
                    if !agent.small_model {
                        bail!("{} has no quick-task model", agent.name);
                    }
                    let model = if *model == "off" { None } else { Some(*model) };
                    pansy_agents::set_small_model(id, model)?;
                    println!("Set {}'s quick-task model.", agent.name);
                    Ok(())
                }
                [model] => connect(id, model),
                _ => {
                    bail!("usage: pansy {agent} <provider/model> | off | --small <provider/model>")
                }
            },
            None => bail!("no command {agent}; `pansy help` lists them"),
        },
    }
}

fn agent_id(name: &str) -> &str {
    AGENT_ALIASES
        .iter()
        .find(|(alias, _)| *alias == name)
        .map(|(_, id)| *id)
        .unwrap_or(name)
}

fn connect(agent: &str, model: &str) -> Result<()> {
    let id = agent_id(agent);
    let config = Config::load()?;
    let known = model.starts_with("group/") && config.group(&model[6..]).is_some()
        || model
            .split_once('/')
            .is_some_and(|(p, _)| config.provider(p).is_some());
    if !known {
        eprintln!(
            "pansy: note: no provider or group answers {model} yet; `pansy models` lists what does"
        );
    }
    pansy_agents::connect(id, model)?;
    let name = pansy_agents::agent::find(id).map(|a| a.name).unwrap_or(id);
    println!("{name} now uses {model}. Start a new session for it to take effect.");
    Ok(())
}

fn disconnect(agent: &str) -> Result<()> {
    let id = agent_id(agent);
    pansy_agents::disconnect(id)?;
    let name = pansy_agents::agent::find(id).map(|a| a.name).unwrap_or(id);
    println!("{name}'s config is back as it was.");
    Ok(())
}

fn serve() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("PANSY_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .init();
    let gateway = pansy_gateway::Gateway::new();
    runtime()?.block_on(pansy_gateway::serve(gateway, async {
        let _ = tokio::signal::ctrl_c().await;
    }))
}

/// Every check, a line each; fails when one does.
fn doctor(json: bool) -> Result<()> {
    use pansy_agents::doctor::{self, Level};
    let found = doctor::run();
    if json {
        print_json(&found)?;
    } else {
        for f in &found {
            let mark = match f.level {
                Level::Ok => "ok  ",
                Level::Note => "note",
                Level::Warn => "warn",
                Level::Fail => "FAIL",
            };
            println!("{mark}  {}", doctor::say(f));
        }
    }
    let failed = found.iter().filter(|f| f.level == Level::Fail).count();
    if failed > 0 {
        bail!("{failed} of the checks failed");
    }
    Ok(())
}

/// One short message through the gateway; fails when it isn't answered.
fn send_test(model: Option<&str>, json: bool) -> Result<()> {
    let sent = pansy_agents::doctor::send(model)?;
    if json {
        print_json(&sent)?;
    }
    let secs = sent.ms as f64 / 1000.0;
    match (sent.ok, sent.status) {
        (true, _) => {
            if !json {
                println!("{} answered in {secs:.1} s: {}", sent.model, sent.said);
            }
            Ok(())
        }
        (false, 0) if sent.said.is_empty() => bail!(
            "nothing answers at {}: open pansy or run pansy serve",
            sent.url
        ),
        (false, 0) => bail!("{}: {}", sent.model, sent.said),
        (false, status) => bail!(
            "{} failed ({status}) in {secs:.1} s: {}",
            sent.model,
            sent.said
        ),
    }
}

fn status(json: bool) -> Result<()> {
    let config = Config::load()?;
    let agents = pansy_agents::list();
    if json {
        return print_json(&json!({
            "gateway": config.settings.base_url(),
            "providers": config.providers.iter().map(|p| json!({"id": p.id, "keys": p.accounts.len(), "models": p.models.len(), "enabled": p.enabled})).collect::<Vec<_>>(),
            "groups": config.groups.iter().map(|g| &g.id).collect::<Vec<_>>(),
            "agents": agents,
        }));
    }
    println!("Gateway   {}", config.settings.base_url());
    println!("Files     {}", pansy_core::paths::home().display());
    println!();
    println!("Providers");
    if config.providers.is_empty() {
        println!("  none yet: pansy provider add <preset> <key>");
    }
    for p in &config.providers {
        let state = if p.enabled { "" } else { "  (off)" };
        println!(
            "  {:16} {}, {}{state}",
            p.id,
            count(p.accounts.len(), "key", "keys"),
            count(p.models.len(), "model", "models")
        );
    }
    if !config.groups.is_empty() {
        println!();
        println!("Groups");
        let width = config
            .groups
            .iter()
            .map(|g| g.id.len())
            .max()
            .unwrap_or(0)
            .max(10);
        for g in &config.groups {
            println!(
                "  group/{:width$} {} ({})",
                g.id,
                g.members.join(", "),
                g.routing.as_str()
            );
        }
    }
    println!();
    println!("Agents");
    for a in agents.iter().filter(|a| a.installed || a.connected) {
        println!("  {:14} {}", a.name, a.model.as_deref().unwrap_or("-"));
    }
    Ok(())
}

fn agents(json: bool) -> Result<()> {
    let agents = pansy_agents::list();
    if json {
        return print_json(&agents);
    }
    println!("{:14} {:10} {:18} MODEL", "AGENT", "INSTALLED", "PROTOCOL");
    for a in agents {
        println!(
            "{:14} {:10} {:18} {}",
            a.name,
            if a.installed { "yes" } else { "no" },
            a.protocol.label(),
            a.model.as_deref().unwrap_or("-")
        );
    }
    Ok(())
}

fn models(json: bool) -> Result<()> {
    let choices = manage::choices()?;
    if json {
        return print_json(&choices);
    }
    if choices.is_empty() {
        println!("No models yet: add a provider (pansy provider add <preset> <key>).");
    }
    for c in choices {
        if c.group {
            println!("{:44} group: {}", c.id, c.name);
        } else {
            println!("{:44} {}", c.id, c.provider);
        }
    }
    Ok(())
}

fn provider(words: &[&str], args: &[String], json: bool) -> Result<()> {
    let (plain, options) = split_options(args);
    let plain: Vec<&str> = plain.iter().map(String::as_str).collect();
    match words {
        ["presets"] => {
            if json {
                let presets: Vec<_> = PRESETS
                    .iter()
                    .map(|p| {
                        json!({
                            "id": p.id, "name": p.name, "kind": p.kind.as_str(), "note": p.note,
                            "protocols": p.urls_at(None).iter().map(|u| u.0).collect::<Vec<_>>(),
                            "regions": p.regions.iter().map(|r| r.id).collect::<Vec<_>>(),
                            "keyUrl": p.key_url, "website": p.website,
                        })
                    })
                    .collect();
                return print_json(&presets);
            }
            let id_width = PRESETS.iter().map(|p| p.id.len()).max().unwrap_or(0);
            let name_width = PRESETS
                .iter()
                .map(|p| p.name.chars().count())
                .max()
                .unwrap_or(0);
            let mut kind = None;
            for p in PRESETS {
                if kind != Some(p.kind) {
                    kind = Some(p.kind);
                    println!(
                        "{}",
                        match p.kind {
                            Kind::Vendor => "Vendors — the makers' own APIs",
                            Kind::Relay => "\nRelays — one key, many vendors",
                            Kind::Local => "\nOn this machine",
                        }
                    );
                }
                let protocols: Vec<&str> = p
                    .urls_at(None)
                    .iter()
                    .map(|(protocol, _)| protocol.as_str())
                    .collect();
                // Chinese and the like are two columns wide.
                let wide = p.name.chars().filter(|c| !c.is_ascii()).count();
                let pad = name_width.saturating_sub(wide);
                let mut line = format!(
                    "  {:id_width$} {:pad$} {}",
                    p.id,
                    p.name,
                    protocols.join(", ")
                );
                if !p.note.is_empty() {
                    line.push_str(&format!(" · {}", p.note));
                }
                if !p.regions.is_empty() {
                    let ids: Vec<&str> = p.regions.iter().map(|r| r.id).collect();
                    line.push_str(&format!(" · {}=", p.region_label.to_lowercase()));
                    line.push_str(&ids.join("|"));
                }
                println!("{line}");
            }
            Ok(())
        }
        ["region", id, region] | ["plan", id, region] => {
            manage::set_region(id, region)?;
            let p = Config::load()?
                .provider(id)
                .cloned()
                .ok_or_else(|| anyhow!("there is no provider {id}"))?;
            println!(
                "{id} is at {}.",
                p.endpoints.first().map_or("", |e| e.base_url.as_str())
            );
            if p.lists_models() {
                match runtime()?.block_on(manage::refresh_models(id)) {
                    Ok(n) => println!("{id} has {}.", count(n, "model", "models")),
                    Err(e) => println!("Could not fetch its models ({e:#})."),
                }
            }
            Ok(())
        }
        ["address", id, address] | ["endpoint", id, address] => {
            manage::set_address(id, address)?;
            println!("{id} is at {address}.");
            Ok(())
        }
        ["key-header", id, header] => {
            let header = match *header {
                "both" => None,
                h => Some(
                    AuthHeader::parse(h)
                        .ok_or_else(|| anyhow!("the key header is both, x-api-key or bearer"))?,
                ),
            };
            manage::set_auth_header(id, header)?;
            println!(
                "{id}'s Anthropic API gets its key in {}.",
                header.map_or("both x-api-key and Authorization", |h| match h {
                    AuthHeader::XApiKey => "x-api-key",
                    AuthHeader::Bearer => "Authorization: Bearer",
                })
            );
            Ok(())
        }
        ["move", id, to] => {
            let to: usize = to
                .parse()
                .ok()
                .filter(|n| *n > 0)
                .ok_or_else(|| anyhow!("the place is a number from 1"))?;
            manage::move_provider(id, to - 1)?;
            println!(
                "{id} is number {to}: a model several providers serve goes to the higher one first."
            );
            Ok(())
        }
        ["list"] => {
            let providers = config::load_providers()?;
            if json {
                let masked: Vec<_> = providers
                    .iter()
                    .map(|p| {
                        json!({
                            "id": p.id, "name": p.name, "enabled": p.enabled, "endpoints": p.endpoints,
                            "subscription": p.subscription,
                            "keys": p.accounts.iter().map(|a| json!({"id": a.id, "label": a.label, "key": mask_key(&a.key), "enabled": a.enabled, "signIn": a.sign_in})).collect::<Vec<_>>(),
                            "models": p.models.iter().map(|m| &m.id).collect::<Vec<_>>(),
                        })
                    })
                    .collect();
                return print_json(&masked);
            }
            if providers.is_empty() {
                println!(
                    "No providers yet: pansy provider add <preset> <key> (pansy provider presets lists them)."
                );
            }
            for p in providers {
                println!(
                    "{} ({}){}",
                    p.name,
                    p.id,
                    if p.enabled { "" } else { " — off" }
                );
                for e in &p.endpoints {
                    println!("  {:18} {}", e.protocol.label(), e.base_url);
                }
                for a in &p.accounts {
                    let off = if a.enabled { "" } else { " (off)" };
                    match &a.sign_in {
                        Some(s) => println!(
                            "  account {:4} {}{}{off}",
                            a.id,
                            a.display(),
                            if s.plan.is_empty() {
                                String::new()
                            } else {
                                format!(" · {}", s.plan)
                            }
                        ),
                        None => println!(
                            "  key {:4} {:14} {}{off}",
                            a.id,
                            a.display(),
                            mask_key(&a.key)
                        ),
                    }
                }
                println!("  {} models", p.models.len());
            }
            Ok(())
        }
        ["add", ..] => {
            let preset = plain
                .get(1)
                .ok_or_else(|| anyhow!("usage: pansy provider add <preset> [key]"))?;
            let key = plain
                .get(2)
                .copied()
                .or(options.get("key").map(String::as_str));
            let had = config::load_providers()?
                .into_iter()
                .find(|p| p.preset.as_deref() == Some(*preset));
            if let (Some(had), Some(key)) = (&had, key.map(str::trim))
                && had.accounts.iter().any(|a| a.key == key)
            {
                println!("{} has that key already.", had.id);
                return Ok(());
            }
            let id = manage::add_preset(
                preset,
                key,
                options.get("label").map(String::as_str).unwrap_or(""),
                manage::At {
                    region: options
                        .get("region")
                        .or(options.get("plan"))
                        .map(String::as_str),
                    address: options
                        .get("address")
                        .or(options.get("endpoint"))
                        .map(String::as_str),
                },
            )?;
            if had.is_some() {
                println!("Added the key to {id}.");
                return Ok(());
            }
            println!("Added {id}. Fetching its models…");
            match runtime()?.block_on(manage::refresh_models(&id)) {
                Ok(n) => println!(
                    "{id} has {}: pansy provider models {id}",
                    count(n, "model", "models")
                ),
                Err(e) => {
                    let n = Config::load()?.provider(&id).map_or(0, |p| p.models.len());
                    if n > 0 {
                        println!("Could not fetch its models ({e:#}); the preset's {n} are there.");
                    } else {
                        println!(
                            "Could not fetch its models ({e:#}). Add one: pansy provider model add {id} <model>"
                        );
                    }
                }
            }
            Ok(())
        }
        ["add-custom", ..] => {
            let name = plain.get(1).ok_or_else(|| anyhow!("usage: pansy provider add-custom <name> url=<base URL> [protocol=chat] [key=…] [models=a,b]"))?;
            let url = options
                .get("url")
                .ok_or_else(|| anyhow!("url=<base URL> is needed"))?;
            let protocol = options
                .get("protocol")
                .map(String::as_str)
                .unwrap_or("chat");
            let protocol = Protocol::parse(protocol)
                .ok_or_else(|| anyhow!("protocol is chat, anthropic, responses or gemini"))?;
            let models = options
                .get("models")
                .map(|m| list_of(m))
                .unwrap_or_default();
            let id = manage::add_custom(
                name,
                protocol,
                url,
                options.get("key").map(String::as_str),
                &models,
            )?;
            println!("Added {id}.");
            Ok(())
        }
        ["remove", id] => {
            manage::remove_provider(id)?;
            println!("Removed {id}.");
            Ok(())
        }
        ["on", id] | ["enable", id] => {
            manage::set_provider_enabled(id, true)?;
            println!("{id} is on.");
            Ok(())
        }
        ["off", id] | ["disable", id] => {
            manage::set_provider_enabled(id, false)?;
            println!("{id} is off: its models are not served.");
            Ok(())
        }
        ["key", "add", ..] => {
            let id = plain
                .get(2)
                .ok_or_else(|| anyhow!("usage: pansy provider key add <id> <key>"))?;
            let key = plain
                .get(3)
                .ok_or_else(|| anyhow!("usage: pansy provider key add <id> <key>"))?;
            let account = manage::add_key(
                id,
                key,
                options.get("label").map(String::as_str).unwrap_or(""),
            )?;
            println!("Added key {account} to {id}.");
            Ok(())
        }
        ["key", "remove", id, account] => {
            manage::remove_key(id, account)?;
            println!("Removed key {account} from {id}.");
            Ok(())
        }
        ["key", "on", id, account] => {
            manage::set_key_enabled(id, account, true)?;
            println!("Key {account} of {id} is on.");
            Ok(())
        }
        ["key", "off", id, account] => {
            manage::set_key_enabled(id, account, false)?;
            println!("Key {account} of {id} is off.");
            Ok(())
        }
        ["models", id, rest @ ..] => {
            if rest.contains(&"--refresh") {
                let n = runtime()?.block_on(manage::refresh_models(id))?;
                eprintln!("{id} has {n} models.");
            }
            let config = Config::load()?;
            let p = config
                .provider(id)
                .ok_or_else(|| anyhow!("there is no provider {id}"))?;
            if json {
                return print_json(&p.models);
            }
            for m in &p.models {
                let hidden = if p.hidden_models.contains(&m.id) {
                    "  (hidden)"
                } else {
                    ""
                };
                println!("{}/{}{hidden}", p.id, m.id);
            }
            Ok(())
        }
        ["model", "add", id, model] => {
            manage::add_model(id, model)?;
            println!("Added {id}/{model}.");
            Ok(())
        }
        ["model", "remove", id, model] => {
            manage::remove_model(id, model)?;
            println!("Removed {id}/{model}.");
            Ok(())
        }
        _ => bail!("no such provider command; `pansy help` lists them"),
    }
}

fn group(words: &[&str], args: &[String], json: bool) -> Result<()> {
    let (plain, options) = split_options(args);
    // A middleware's routing (`<plugin>/<name>`) is kept apart from the
    // group's own.
    let strategy = |options: &std::collections::BTreeMap<String, String>| -> Option<String> {
        options.get("routing").filter(|r| r.contains('/')).cloned()
    };
    let routing =
        |options: &std::collections::BTreeMap<String, String>| -> Result<Option<Routing>> {
            options
                .get("routing")
                .filter(|r| !r.contains('/'))
                .map(|r| {
                    Routing::parse(r).ok_or_else(|| {
                        anyhow!("routing is order, rotate, usage or a middleware's <plugin>/<name>")
                    })
                })
                .transpose()
        };
    match words {
        ["list"] | [] => {
            let groups = config::load_groups()?;
            if json {
                return print_json(&groups);
            }
            if groups.is_empty() {
                println!("No groups yet: pansy group add <name> models=<a/x,b/y>");
            }
            for g in groups {
                let how = g
                    .strategy
                    .clone()
                    .unwrap_or_else(|| g.routing.as_str().to_owned());
                println!("group/{} — {} ({how})", g.id, g.name);
                for (i, m) in g.members.iter().enumerate() {
                    println!("  {}. {m}", i + 1);
                }
            }
            Ok(())
        }
        ["add", ..] => {
            let name = plain.get(1).ok_or_else(|| {
                anyhow!("usage: pansy group add <name> models=<a/x,b/y> [routing=order]")
            })?;
            let members = options
                .get("models")
                .map(|m| list_of(m))
                .unwrap_or_default();
            let id =
                manage::save_group(None, name, &members, routing(&options)?.unwrap_or_default())?;
            if let Some(s) = strategy(&options) {
                manage::set_group_strategy(&id, Some(&s))?;
            }
            println!("Added group/{id}: pansy claude group/{id}");
            Ok(())
        }
        ["set", id, ..] => {
            let groups = config::load_groups()?;
            let g = groups
                .iter()
                .find(|g| g.id == *id)
                .ok_or_else(|| anyhow!("there is no group {id}"))?;
            let name = options
                .get("name")
                .cloned()
                .unwrap_or_else(|| g.name.clone());
            let members = options
                .get("models")
                .map(|m| list_of(m))
                .unwrap_or_else(|| g.members.clone());
            manage::save_group(
                Some(id),
                &name,
                &members,
                routing(&options)?.unwrap_or(g.routing),
            )?;
            if options.contains_key("routing") {
                manage::set_group_strategy(id, strategy(&options).as_deref())?;
            }
            println!("Saved group/{id}.");
            Ok(())
        }
        ["remove", id] => {
            manage::remove_group(id)?;
            println!("Removed group/{id}.");
            Ok(())
        }
        _ => bail!("no such group command; `pansy help` lists them"),
    }
}

fn subscription_kind(name: &str) -> Result<subscription::Kind> {
    subscription::Kind::parse(name).ok_or_else(|| {
        anyhow!(
            "no subscription {name}: codex (ChatGPT), copilot or gemini-cli (Gemini Code Assist)"
        )
    })
}

/// The subscriptions' accounts, by provider, in the order they are tried.
fn accounts(json: bool) -> Result<()> {
    let providers: Vec<_> = config::load_providers()?
        .into_iter()
        .filter(|p| p.subscription.is_some())
        .collect();
    if json {
        let out: Vec<_> = providers
            .iter()
            .map(|p| {
                json!({
                    "provider": p.id, "name": p.name, "enabled": p.enabled,
                    "accounts": p.accounts.iter().map(|a| json!({"id": a.id, "enabled": a.enabled, "signIn": a.sign_in})).collect::<Vec<_>>(),
                })
            })
            .collect();
        return print_json(&out);
    }
    if providers.is_empty() {
        println!(
            "No subscription is signed in to: sign in in Codex, Copilot or Gemini CLI, or `pansy login codex|copilot|gemini`."
        );
        return Ok(());
    }
    for p in providers {
        println!(
            "{} ({}){}",
            p.name,
            p.id,
            if p.enabled { "" } else { " — off" }
        );
        for a in &p.accounts {
            let Some(s) = &a.sign_in else { continue };
            let kind = s.kind;
            let place = match s.source {
                subscription::Source::Agent => format!("{}'s own sign-in", kind.agent()),
                subscription::Source::Pansy => "signed in from pansy".to_owned(),
            };
            println!(
                "  {:4} {}{}  {place}{}",
                a.id,
                s.user,
                if s.plan.is_empty() {
                    String::new()
                } else {
                    format!(" · {}", s.plan)
                },
                if a.enabled { "" } else { " (off)" }
            );
        }
        println!("  {} models", p.models.len());
    }
    Ok(())
}

/// `2h 5m`, `3d 4h`: how long until `at`.
fn renews_in(at: chrono::DateTime<chrono::Utc>) -> String {
    let secs = (at - chrono::Utc::now()).num_seconds().max(0);
    let (d, h, m) = (secs / 86400, secs % 86400 / 3600, secs % 3600 / 60);
    match (d, h) {
        (0, 0) => format!("{}m", m.max(1)),
        (0, _) => format!("{h}h {m}m"),
        _ => format!("{d}d {h}h"),
    }
}

/// What each subscription account has used of its allowance.
fn limits(json: bool) -> Result<()> {
    let all = runtime()?.block_on(subscription::allowance::allowances(true));
    if json {
        return print_json(&all);
    }
    if all.is_empty() {
        println!("No subscription is signed in to: `pansy login codex|copilot|gemini`.");
        return Ok(());
    }
    let names = config::load_providers()?;
    for a in &all {
        let name = names
            .iter()
            .find(|p| p.id == a.provider)
            .map_or(a.provider.as_str(), |p| p.name.as_str());
        let plan = if a.plan.is_empty() {
            String::new()
        } else {
            format!(" · {}", a.plan)
        };
        let off = if a.enabled { "" } else { " (off)" };
        println!("{name} · {}{plan}{off}", a.user);
        if !a.error.is_empty() {
            println!("  couldn't be read: {}", a.error);
            continue;
        }
        for w in &a.windows {
            if w.unlimited {
                println!("  {:24} unlimited", w.name);
                continue;
            }
            let filled = (w.used / 10.0).round() as usize;
            let bar = "█".repeat(filled.min(10)) + &"░".repeat(10 - filled.min(10));
            let count = if w.display.is_empty() {
                String::new()
            } else {
                format!("  {}", w.display)
            };
            let renews = w
                .resets_at
                .map(|t| format!("  renews in {}", renews_in(t)))
                .unwrap_or_default();
            println!("  {:24} {bar} {:>3.0}%{count}{renews}", w.name, w.used);
        }
        if !a.note.is_empty() {
            println!("  {}", a.note);
        }
        if let Some(until) = a.spent_until {
            println!(
                "  held by its vendor for {}: the next account answers",
                renews_in(until)
            );
        }
    }
    Ok(())
}

/// Signs in to another account of a subscription, then lists its models.
fn login(name: &str, by_code: bool, json: bool) -> Result<()> {
    let kind = subscription_kind(name)?;
    let by_code = by_code || !kind.signs_in_by_browser();
    let started = subscription::flow::start(kind, by_code)?;
    if by_code {
        eprintln!("Open {} and type the code {}", started.url, started.code);
    } else {
        eprintln!(
            "Open this page to sign in to {}:\n\n  {}\n",
            kind.vendor(),
            started.url
        );
    }
    eprintln!("Waiting for the sign-in… (Ctrl-C to stop)");
    let ended = subscription::flow::wait(&started.id);
    if json {
        return print_json(&ended);
    }
    match ended.state.as_str() {
        "done" => {
            let plan = if ended.plan.is_empty() {
                String::new()
            } else {
                format!(" ({})", ended.plan)
            };
            println!("Signed in to {}{plan}.", ended.user);
            match runtime()?.block_on(manage::refresh_models(kind.id())) {
                Ok(n) => println!("{} has {n} models: {}/<model>.", kind.name(), kind.id()),
                Err(e) => eprintln!("pansy: its models couldn't be listed yet: {e:#}"),
            }
            Ok(())
        }
        "canceled" => bail!("the sign-in was stopped"),
        _ => bail!("{}", ended.error),
    }
}

fn profiles(json: bool) -> Result<()> {
    let profiles = profile::list()?;
    if json {
        return print_json(&profiles);
    }
    if profiles.is_empty() {
        println!("No profiles yet: connect agents, then pansy save <name>.");
    }
    for p in profiles {
        println!(
            "{}{}",
            p.profile.name,
            if p.active { "  (in use)" } else { "" }
        );
        for (agent, choice) in &p.profile.agents {
            println!("  {agent:14} {}", choice.model);
        }
    }
    Ok(())
}

fn usage_report(args: &[String], json: bool) -> Result<()> {
    let (_, options) = split_options(args);
    let days = days_of(&options)?;
    let summary = usage::summary(days);
    if json {
        return print_json(&summary);
    }
    let t = &summary.total;
    println!(
        "Last {}: {}, {}",
        count(days as usize, "day", "days"),
        count(t.requests as usize, "request", "requests"),
        count(t.errors as usize, "error", "errors")
    );
    println!(
        "Tokens: {} in ({} from the cache), {} out ({} reasoning)",
        grouped(t.input),
        grouped(t.cache_read),
        grouped(t.output),
        grouped(t.reasoning)
    );
    let table = |title: &str, rows: Vec<(&String, &usage::Totals)>| {
        if rows.is_empty() {
            return;
        }
        println!();
        println!(
            "{title:44} {:>9} {:>12} {:>12}",
            "REQUESTS", "INPUT", "OUTPUT"
        );
        for (name, t) in rows {
            println!(
                "{name:44} {:>9} {:>12} {:>12}",
                grouped(t.requests),
                grouped(t.input),
                grouped(t.output)
            );
        }
    };
    if days > 1 {
        table(
            "DAY",
            summary
                .by_day
                .iter()
                .filter(|(_, t)| t.requests > 0)
                .collect(),
        );
    }
    let mut providers: Vec<_> = summary.by_provider.iter().collect();
    providers.sort_by_key(|(_, t)| std::cmp::Reverse(t.input + t.output));
    table("PROVIDER", providers);
    if !summary.by_model.is_empty() {
        println!();
        println!(
            "{:44} {:>9} {:>12} {:>12}",
            "MODEL", "REQUESTS", "INPUT", "OUTPUT"
        );
        let mut rows: Vec<_> = summary.by_model.iter().collect();
        rows.sort_by_key(|(_, t)| std::cmp::Reverse(t.input + t.output));
        for (model, t) in rows {
            println!(
                "{model:44} {:>9} {:>12} {:>12}",
                grouped(t.requests),
                grouped(t.input),
                grouped(t.output)
            );
        }
    }
    if !summary.by_agent.is_empty() {
        println!();
        println!(
            "{:44} {:>9} {:>12} {:>12}",
            "AGENT", "REQUESTS", "INPUT", "OUTPUT"
        );
        for (agent, t) in &summary.by_agent {
            println!(
                "{agent:44} {:>9} {:>12} {:>12}",
                grouped(t.requests),
                grouped(t.input),
                grouped(t.output)
            );
        }
    }
    Ok(())
}

/// `days=` of a report: 1 to 90, 7 when not given.
fn days_of(options: &std::collections::BTreeMap<String, String>) -> Result<u32> {
    match options.get("days") {
        None => Ok(7),
        Some(d) => match d.parse::<u32>() {
            Ok(n @ 1..=90) => Ok(n),
            _ => bail!("days is a whole number from 1 to 90, not {d}"),
        },
    }
}

/// "1 key", "3 keys".
fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// 1234567 → "1,234,567".
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `pansy mask <text>`: the text masked as a request would be, with the
/// settings as they are; kept nowhere. With no text, standard input.
fn mask_try(words: &[String], json: bool) -> Result<()> {
    let text = if words.is_empty() {
        let mut text = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)?;
        text
    } else {
        words.join(" ")
    };
    let options = config::load_settings()?.masking;
    let masker = pansy_mask::Masker::new(&pansy_core::masking::key()?);
    let masked = masker.mask_text(&text, &options);
    if json {
        let hits: Vec<_> = masked
            .hits
            .iter()
            .map(|h| json!({"placeholder": h.placeholder, "kind": h.kind, "action": h.action}))
            .collect();
        return print_json(&json!({"text": masked.text, "hits": hits}));
    }
    println!("{}", masked.text.trim_end_matches('\n'));
    if !options.wanted() {
        eprintln!(
            "Masking is off: pansy settings set masking '{{\"secrets\": true}}' turns it on."
        );
    } else if masked.hits.is_empty() {
        eprintln!("Nothing to mask in it.");
    } else {
        eprintln!();
        for h in &masked.hits {
            eprintln!("  {:24} {}", h.placeholder, h.kind);
        }
    }
    Ok(())
}

/// `pansy masking [days=7]`: what masking kept from vendors.
fn masking_report(args: &[String], json: bool) -> Result<()> {
    if args.first().map(String::as_str) == Some("clear") {
        pansy_core::masking::clear()?;
        println!("Cleared masking's records.");
        return Ok(());
    }
    let (_, options) = split_options(args);
    let days = days_of(&options)?;
    let summary = pansy_core::masking::summary(days);
    if json {
        return print_json(&summary);
    }
    let on = config::load_settings()?.masking.wanted();
    println!(
        "Last {}: {} masked ({} different) in {}{}",
        count(summary.days as usize, "day", "days"),
        count(summary.values, "value", "values"),
        summary.distinct,
        count(summary.requests, "request", "requests"),
        if on { "" } else { "; masking is off" }
    );
    if !summary.by_kind.is_empty() {
        println!();
        let mut kinds: Vec<_> = summary.by_kind.iter().collect();
        kinds.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
        for (kind, n) in kinds {
            println!("  {kind:24} {n:>6}");
        }
    }
    if !summary.records.is_empty() {
        println!();
        println!("{:20} {:14} {:28} {:>6}", "WHEN", "AGENT", "TO", "VALUES");
        for r in summary.records.iter().take(20) {
            let when =
                r.t.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M:%S");
            let to = format!("{}/{}", r.provider, r.model);
            let held = if r.blocked { "  held back" } else { "" };
            println!("{when:20} {:14} {to:28} {:>6}{held}", r.agent, r.n);
        }
    }
    Ok(())
}

/// `pansy import …`: the providers other apps have set up, read only, and
/// brought over as the window's Import does.
fn import(args: &[String], json: bool) -> Result<()> {
    use pansy_agents::import::{self, Mode, Pick, Status};
    // a link's query has `=` in it: it is no option
    if let Some(link) = args.iter().find(|a| import::link::is_link(a)) {
        let yes = args.iter().any(|a| a == "-y" || a == "--yes");
        return import_link(link, yes, json);
    }
    let (plain, options) = split_options(args);
    let sources = import::sources(true);
    let Some((app, refs)) = plain.split_first() else {
        if json {
            return print_json(&import::view(&sources));
        }
        for s in &sources {
            let state = match (&s.error, s.found) {
                (Some(e), _) => format!(" · {e}"),
                (None, false) => " · not found".into(),
                (None, true) => String::new(),
            };
            println!("{} ({}) · {}{state}", s.name, s.id, s.path.display());
            for it in &s.items {
                let p = it.provider.as_ref();
                let host = p
                    .and_then(|p| p.endpoints.first())
                    .map(|e| e.base_url.as_str())
                    .unwrap_or_default();
                let key = p
                    .and_then(|p| p.accounts.first())
                    .map(|a| mask_key(&a.key))
                    .unwrap_or_default();
                let said = if !it.skip.is_empty() {
                    it.skip.clone()
                } else if !it.off.is_empty() {
                    it.off.clone()
                } else {
                    match it.status {
                        Status::Same => format!("already added as {}", it.existing),
                        Status::Taken if !it.key_of.is_empty() => {
                            format!("same vendor as {}: as=key adds it as a key", it.key_of)
                        }
                        Status::Taken => format!("{} is in use", it.existing),
                        Status::New => "new".into(),
                    }
                };
                println!("  {}  {}  {host}  {key}  · {said}", it.reference, it.name);
            }
        }
        return Ok(());
    };
    let s = sources
        .iter()
        .find(|s| s.id == app.as_str() || s.name.eq_ignore_ascii_case(app))
        .ok_or_else(|| {
            let ids: Vec<&str> = sources.iter().map(|s| s.id).collect();
            anyhow!("no app {app}: one of {}", ids.join(", "))
        })?;
    let mode = match options.get("as").map(String::as_str) {
        None => None,
        Some("add") => Some(Mode::Add),
        Some("key") => Some(Mode::Key),
        Some("replace") => Some(Mode::Replace),
        Some(other) => bail!("as={other}: add, key or replace"),
    };
    let mut picks = Vec::new();
    for it in &s.items {
        let asked = refs.contains(&it.reference);
        if !(asked || (refs.is_empty() && import::picked(it))) {
            continue;
        }
        if !it.skip.is_empty() {
            bail!("{} can't be imported: {}", it.reference, it.skip);
        }
        let mode = mode.unwrap_or(if it.key_of.is_empty() {
            Mode::Add
        } else {
            Mode::Key
        });
        picks.push(Pick {
            source: s.id.to_string(),
            reference: it.reference.clone(),
            mode,
            name: String::new(),
        });
    }
    if let Some(missing) = refs
        .iter()
        .find(|r| !s.items.iter().any(|it| it.reference == **r))
    {
        bail!("{} has no {missing}", s.name);
    }
    let added = import::import(&picks)?;
    if json {
        return print_json(&json!({"names": added.names, "ids": added.ids}));
    }
    if added.names.is_empty() {
        println!("Nothing new to import from {}.", s.name);
    } else {
        println!("Imported from {}: {}.", s.name, added.names.join(", "));
        for id in &added.ids {
            println!("  pansy provider models {id} --refresh   its models from the vendor");
        }
    }
    Ok(())
}

/// `pansy import <link>`: what the link would add, then, asked on a
/// terminal or with `-y`, the provider added.
fn import_link(link: &str, yes: bool, json: bool) -> Result<()> {
    use pansy_agents::import::link;
    let v = link::view(link);
    if let Some(e) = v.get("error").and_then(|e| e.as_str()) {
        bail!("{e}");
    }
    let s = |k: &str| v[k].as_str().unwrap_or_default().to_owned();
    if !json {
        println!("{} ({})", s("name"), s("id"));
        for e in v["endpoints"].as_array().into_iter().flatten() {
            let protocol: Protocol = serde_json::from_value(e["protocol"].clone())?;
            println!(
                "  {:<18} {}",
                protocol.label(),
                e["url"].as_str().unwrap_or_default()
            );
        }
        let key = s("key");
        println!(
            "  key {}",
            if key.is_empty() {
                "none in the link"
            } else {
                &key
            }
        );
        let models: Vec<&str> = v["models"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| m.as_str())
            .collect();
        if !models.is_empty() {
            println!("  models {}", models.join(", "));
        }
        let hosts: Vec<&str> = v["hosts"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|h| h.as_str())
            .collect();
        println!(
            "Your prompts and this key go to {}: add it only if you trust who sent the link.",
            hosts.join(", ")
        );
        if !s("replaces").is_empty() {
            println!("! It replaces your {}, key and all.", s("replaces"));
        }
    }
    if !yes {
        if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            bail!("not a terminal: add -y to import without asking");
        }
        print!("Add it? [y/N] ");
        std::io::Write::flush(&mut std::io::stdout())?;
        let mut answer = String::new();
        std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut answer)?;
        if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            bail!("nothing added");
        }
    }
    let added = link::add(link, "", "")?;
    if json {
        return print_json(&added);
    }
    println!("Added {}.", added.names.join(", "));
    for id in &added.ids {
        println!("  pansy provider models {id} --refresh   its models from the vendor");
    }
    Ok(())
}

/// `pansy sessions …`: Claude Code's and Codex's sessions, read from their
/// files; a delete moves one into pansy's trash.
fn sessions(args: &[String], json: bool) -> Result<()> {
    use pansy_agents::sessions::{self, trash};
    let (plain, options) = split_options(args);
    let plain: Vec<&str> = plain.iter().map(String::as_str).collect();
    match plain.as_slice() {
        [] => session_list(None, json),
        ["show", id] => {
            let s = session_by(id)?;
            let sub = match options.get("sub") {
                None => String::new(),
                Some(sub) => s
                    .subagents
                    .iter()
                    .find(|a| a.id == *sub)
                    .or_else(|| s.subagents.iter().find(|a| a.id.starts_with(sub.as_str())))
                    .map(|a| a.id.clone())
                    .ok_or_else(|| anyhow!("no subagent {sub} in {}", s.id))?,
            };
            let turns = sessions::transcript(s.agent, &s.id, &sub)?;
            if json {
                return print_json(&turns);
            }
            let agent = agent_name(s.agent);
            println!("{} · {agent}", s.title);
            if !s.cwd.is_empty() {
                println!("{}", s.cwd);
            }
            if !s.resume.is_empty() && sub.is_empty() {
                println!("Resume: {}", s.resume);
            }
            for t in &turns {
                let when = t
                    .at
                    .map(|at| format!(" · {}", at.with_timezone(&chrono::Local).format("%H:%M")))
                    .unwrap_or_default();
                match t.role {
                    "user" => println!("\nYou{when}\n{}", indent(&t.text)),
                    "assistant" => println!("\n{agent}{when}\n{}", indent(&t.text)),
                    "tool" => println!("  → {} {}", t.name, t.text),
                    "compact" => println!("\n— the conversation was compacted here —"),
                    // what the model thought: the window folds it, the list leaves it out
                    _ => {}
                }
            }
            if sub.is_empty() && !s.subagents.is_empty() {
                println!(
                    "\nSubagents (`pansy sessions show {} sub=<id>`):",
                    short_id(&s.id)
                );
                for a in &s.subagents {
                    println!("  {}  {}", short_id(&a.id), a.title);
                }
            }
            Ok(())
        }
        ["find", words @ ..] if !words.is_empty() => {
            let hits = sessions::search(&words.join(" "), 50);
            if json {
                return print_json(&hits);
            }
            if hits.is_empty() {
                println!("No session says that.");
                return Ok(());
            }
            let all = sessions::list();
            for h in &hits {
                let title = all
                    .iter()
                    .find(|s| s.agent == h.agent && s.id == h.id)
                    .map_or("", |s| s.title.as_str());
                println!("{}  {:11}  {title}", short_id(&h.id), agent_name(h.agent));
                println!("          {}", h.text);
            }
            Ok(())
        }
        ["delete", id] => {
            let s = session_by(id)?;
            let t = trash::delete(s.agent, &s.id)?;
            println!(
                "Moved “{}” into pansy's trash; `pansy sessions restore {}` puts it back.",
                t.title, t.key
            );
            Ok(())
        }
        ["trash"] => {
            let all = trash::list();
            if json {
                return print_json(&all);
            }
            if all.is_empty() {
                println!("The trash is empty.");
                return Ok(());
            }
            println!(
                "{:16}  {:11}  TITLE, and the key that restores or purges it",
                "DELETED", "AGENT"
            );
            for t in &all {
                let when = t
                    .deleted
                    .with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M");
                println!("{when:16}  {:11}  {}", agent_name(&t.agent), t.title);
                println!("{:31}{}", "", t.key);
            }
            Ok(())
        }
        ["trash", "empty"] => {
            let n = trash::empty()?;
            println!("Erased {} for good.", count(n, "session", "sessions"));
            Ok(())
        }
        ["restore", key] => {
            let t = trash::restore(key)?;
            println!("Put “{}” back.", t.title);
            Ok(())
        }
        ["purge", key] => {
            trash::purge(key)?;
            println!("Erased {key} for good.");
            Ok(())
        }
        [agent] => match agent_id(agent) {
            id @ ("claude-code" | "codex" | "opencode") => session_list(Some(id), json),
            _ => bail!(
                "pansy reads Claude Code's, Codex's and OpenCode's sessions; `pansy help` lists the commands"
            ),
        },
        _ => bail!("`pansy help` lists the sessions commands"),
    }
}

fn session_list(agent: Option<&str>, json: bool) -> Result<()> {
    let all: Vec<_> = pansy_agents::sessions::list()
        .into_iter()
        .filter(|s| agent.is_none_or(|a| s.agent == a))
        .collect();
    if json {
        return print_json(&all);
    }
    if all.is_empty() {
        match agent {
            Some(id) => println!("No {} sessions yet.", agent_name(id)),
            None => println!(
                "No sessions yet: Claude Code's, Codex's and OpenCode's show up here once they have some."
            ),
        }
        return Ok(());
    }
    let home = pansy_core::paths::user_home().display().to_string();
    println!(
        "{:16} {:11} {:8} {:>8} {:>8}  TITLE",
        "LAST", "AGENT", "ID", "MESSAGES", "TOKENS"
    );
    for s in &all {
        let when = s
            .last
            .map(|t| {
                t.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M")
                    .to_string()
            })
            .unwrap_or_default();
        let folder = match s.cwd.strip_prefix(&home) {
            Some(rest) if rest.is_empty() || rest.starts_with(['/', '\\']) => format!("~{rest}"),
            _ => s.cwd.clone(),
        };
        let title: String = s.title.chars().take(60).collect();
        println!(
            "{when:16} {:11} {:8} {:>8} {:>8}  {title}{}",
            agent_name(s.agent),
            short_id(&s.id),
            s.messages,
            short(s.tokens.total()),
            if folder.is_empty() {
                String::new()
            } else {
                format!("  ({folder})")
            }
        );
    }
    println!("\n{}", count(all.len(), "session", "sessions"));
    Ok(())
}

/// The session whose id is `id`, or the only one whose id starts with it.
fn session_by(id: &str) -> Result<pansy_agents::sessions::Session> {
    let all = pansy_agents::sessions::list();
    if let Some(s) = all.iter().find(|s| s.id == id) {
        return Ok(s.clone());
    }
    let mut found: Vec<_> = all.into_iter().filter(|s| s.id.starts_with(id)).collect();
    match found.len() {
        0 => bail!("no session {id}; `pansy sessions` lists them"),
        1 => Ok(found.remove(0)),
        n => bail!("{n} sessions' ids start with {id}; give more of it"),
    }
}

fn agent_name(id: &str) -> &str {
    pansy_agents::agent::find(id).map_or(id, |a| a.name)
}

/// An id's first eight characters, as lists show it.
fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

/// 1234567 → "1.2M", as the window counts.
fn short(n: u64) -> String {
    match n {
        1_000_000_000.. => format!("{:.1}B", n as f64 / 1e9),
        1_000_000.. => format!("{:.1}M", n as f64 / 1e6),
        10_000.. => format!("{}K", (n as f64 / 1e3).round()),
        1_000.. => format!("{:.1}K", n as f64 / 1e3),
        _ => n.to_string(),
    }
}

fn indent(text: &str) -> String {
    text.lines()
        .map(|l| format!("  {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_options_from_words() {
        let args: Vec<String> = [
            "add",
            "Daily coding",
            "models=a/x,b/y",
            "routing=rotate",
            "--json",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let (plain, options) = split_options(&args);
        assert_eq!(plain, ["add", "Daily coding", "--json"]);
        assert_eq!(options["models"], "a/x,b/y");
        assert_eq!(list_of(&options["models"]), ["a/x", "b/y"]);
        // A key that ends in `=` is a word, not an option.
        let args = vec![
            "add".to_owned(),
            "moonshot".to_owned(),
            "c2stYWJj==".to_owned(),
        ];
        let (plain, options) = split_options(&args);
        assert_eq!(plain[2], "c2stYWJj==");
        assert!(options.is_empty());
    }

    #[test]
    fn counts_and_groups_digits() {
        assert_eq!(count(1, "key", "keys"), "1 key");
        assert_eq!(count(3, "key", "keys"), "3 keys");
        assert_eq!(grouped(4941271), "4,941,271");
        assert_eq!(grouped(12), "12");
    }
}
