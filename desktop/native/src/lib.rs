//! What the pansy window (`App.tsx`) imports from `pansy/native`: the
//! providers, agents, groups, profiles, usage and settings of pansy-core
//! and pansy-agents, and the gateway that runs beside the window. Their
//! TypeScript declarations are generated into `.nsh/types`.
//!
//! Lists come back as JSON (the shapes are in `ui/model.ts`); a change
//! resolves when it is on disk, or rejects with what went wrong.

mod panel;
#[cfg(windows)]
pub mod win;

use std::sync::{Mutex, OnceLock};

use nsh_core::prelude::*;
use pansy_core::config;
use pansy_core::group::Routing;
use pansy_core::manage;
use pansy_core::provider::{AuthHeader, Protocol, mask_key};
use pansy_core::subscription;
use serde::Serialize;
use serde_json::{Value, json};

#[native_module("pansy/native")]
fn native(registry: &mut Registry) {
    registry
        .module::<Gateway>()
        .module::<Providers>()
        .module::<Subscriptions>()
        .module::<Plugins>()
        .module::<Agents>()
        .module::<Diagnostics>()
        .module::<Groups>()
        .module::<Profiles>()
        .module::<Sessions>()
        .module::<Usage>()
        .module::<Masking>()
        .module::<Settings>()
        .module::<Prefs>()
        .module::<Panel>();
}

fn err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

fn to_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_owned())
}

/// Runs `f` off the engine thread.
async fn off<R: Send + 'static>(
    f: impl FnOnce() -> anyhow::Result<R> + Send + 'static,
) -> Result<R, String> {
    blocking(move || f().map_err(err))
        .await
        .map_err(|e| e.to_string())?
}

// --- The gateway --------------------------------------------------------------

/// The gateway the window runs: its runtime, and how to stop it.
struct Running {
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
    error: Option<String>,
    gateway: std::sync::Arc<pansy_gateway::Gateway>,
}

fn running() -> &'static Mutex<Option<Running>> {
    static RUNNING: OnceLock<Mutex<Option<Running>>> = OnceLock::new();
    RUNNING.get_or_init(|| Mutex::new(None))
}

/// Starts the gateway on a thread of its own. When another pansy (`pansy
/// serve`) has the port already, that one serves the agents and this one
/// stays off.
pub fn start_gateway() {
    let mut slot = running().lock().unwrap_or_else(|e| e.into_inner());
    if slot
        .as_ref()
        .is_some_and(|r| r.thread.as_ref().is_some_and(|t| !t.is_finished()))
    {
        return;
    }
    let gateway = pansy_gateway::Gateway::new();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let serving = std::sync::Arc::clone(&gateway);
    let thread = std::thread::Builder::new()
        .name("pansy-gateway".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(e) => return record_error(format!("{e}")),
            };
            let result = runtime.block_on(pansy_gateway::serve(serving, async {
                let _ = stopped.await;
            }));
            if let Err(e) = result {
                record_error(format!("{e:#}"));
            }
        })
        .ok();
    *slot = Some(Running {
        stop: Some(stop),
        thread,
        error: None,
        gateway,
    });
}

fn record_error(error: String) {
    tracing::warn!("the gateway stopped: {error}");
    if let Some(r) = running().lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        r.error = Some(error);
    }
}

/// Stops the gateway and waits for it.
pub fn stop_gateway() {
    let taken = running().lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(mut r) = taken {
        if let Some(stop) = r.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = r.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Whether something answers as pansy at `url`.
fn answers(url: &str) -> bool {
    pansy_agents::doctor::probe(url) == pansy_agents::doctor::Probe::Pansy
}

/// Why the window's gateway stopped, when it did.
fn gateway_error() -> Option<String> {
    let slot = running().lock().unwrap_or_else(|e| e.into_inner());
    slot.as_ref().and_then(|r| r.error.clone())
}

/// The local gateway.
#[module]
pub struct Gateway;

#[module]
impl Gateway {
    /// `{running, own, url, port, error, resting: [{key, secs}]}` (JSON).
    pub async fn status() -> Result<String, String> {
        off(|| {
            let settings = config::load_settings()?;
            let url = settings.base_url();
            let (own, error, resting) = {
                let slot = running().lock().unwrap_or_else(|e| e.into_inner());
                match slot.as_ref() {
                    Some(r) => {
                        let alive = r.thread.as_ref().is_some_and(|t| !t.is_finished());
                        let resting: Vec<Value> = r
                            .gateway
                            .health()
                            .resting_keys()
                            .into_iter()
                            .map(|(key, secs)| json!({"key": key, "secs": secs}))
                            .collect();
                        (alive, r.error.clone(), resting)
                    }
                    None => (false, None, Vec::new()),
                }
            };
            let running = own || answers(&url);
            Ok(to_json(&json!({
                "running": running, "own": own, "url": url, "port": settings.port,
                "error": if running { None } else { error }, "resting": resting,
            })))
        })
        .await
    }

    /// The requests the window's gateway routed lately, when they changed
    /// since `known`, the version it gave last (0 at first, with none):
    /// `{version, own, routes?}` (JSON, see `pansy_gateway::trace::Route`).
    /// `own` is false while another pansy serves the port.
    pub fn trace(known: u32) -> String {
        let slot = running().lock().unwrap_or_else(|e| e.into_inner());
        let Some(r) = slot.as_ref() else {
            return to_json(&json!({"version": 0, "own": false}));
        };
        let own = r.thread.as_ref().is_some_and(|t| !t.is_finished());
        let trace = r.gateway.trace();
        let version = trace.version() as u32;
        if known == version {
            return to_json(&json!({"version": version, "own": own}));
        }
        to_json(&json!({"version": version, "own": own, "routes": trace.routes()}))
    }

    /// Stops the window's gateway and starts it again: after the port or
    /// sharing changed.
    pub async fn restart() -> Result<(), String> {
        off(|| {
            stop_gateway();
            start_gateway();
            Ok(())
        })
        .await
    }
}

// --- Providers ----------------------------------------------------------------

fn provider_views() -> anyhow::Result<Value> {
    let providers = config::load_providers()?;
    let views: Vec<Value> = providers
        .iter()
        .map(|p| {
            let preset = p.preset();
            json!({
                "id": p.id,
                "name": p.name,
                // The preset's id now, for one saved under an id it had before.
                "preset": preset.map(|pr| pr.id),
                "icon": p.icon(),
                "kind": preset.map(|pr| pr.kind.as_str()),
                "note": preset.map_or("", |pr| pr.note),
                "enabled": p.enabled,
                "keyless": p.keyless(),
                "keyUrl": p.key_url(),
                "website": p.website(),
                "region": p.region().map(|r| r.id),
                "address": p.address(),
                "authHeader": p.auth_header.map(|h| h.as_str()),
                "anthropicAuth": p.anthropic_auth().map(|h| h.as_str()),
                "headers": p.headers,
                "endpoints": p.endpoints.iter().map(|e| json!({"protocol": e.protocol, "baseUrl": e.base_url})).collect::<Vec<_>>(),
                "subscription": p.subscription,
                "accounts": p.accounts.iter().map(|a| json!({
                    "id": a.id, "label": a.label, "display": a.display(), "masked": mask_key(&a.key), "enabled": a.enabled,
                    "proxy": a.proxy.clone().unwrap_or_default(),
                    "signIn": a.sign_in,
                })).collect::<Vec<_>>(),
                "models": p.models.iter().map(|m| json!({
                    "id": m.id, "name": m.name, "context": m.context, "hidden": p.hidden_models.contains(&m.id),
                    "manual": p.manual_models.contains(&m.id), "apis": m.apis,
                })).collect::<Vec<_>>(),
                "proxy": p.proxy.clone().unwrap_or_default(),
            })
        })
        .collect();
    Ok(Value::Array(views))
}

/// Providers, their keys and models.
#[module]
pub struct Providers;

#[module]
impl Providers {
    /// Every provider with its keys masked (JSON).
    pub async fn list() -> Result<String, String> {
        off(|| Ok(to_json(&provider_views()?))).await
    }

    /// The presets to add from, as the add sheet lists them: `[{id, name,
    /// short, icon, kind, note, host, endpoints, website, keyUrl, keyHint,
    /// keyless, models, regions, regionLabel, headerHints, endpoint,
    /// endpointHint}]` (JSON).
    pub fn presets() -> String {
        let endpoints = |urls: &[(Protocol, &str)]| -> Vec<Value> {
            urls.iter()
                .map(|(protocol, url)| json!({"protocol": protocol, "baseUrl": url}))
                .collect()
        };
        let presets: Vec<Value> = pansy_core::presets::PRESETS
            .iter()
            .map(|p| {
                json!({
                    "id": p.id, "name": p.name, "short": p.short, "icon": p.icon, "kind": p.kind.as_str(),
                    "note": p.note, "host": p.host(), "endpoints": endpoints(p.urls_at(None)),
                    "website": p.website, "keyUrl": p.key_url, "keyHint": p.key_hint, "keyless": p.keyless,
                    "models": p.models_at(None), "regionLabel": p.region_label, "headerHints": p.header_hints,
                    "endpoint": p.endpoint, "endpointHint": p.endpoint_hint,
                    "regions": p.regions.iter().map(|r| json!({
                        "id": r.id, "name": r.name, "endpoints": endpoints(r.urls),
                        "keyUrl": p.key_url_at(Some(r)), "website": p.website_at(Some(r)),
                        "lists": p.lists_at(Some(r)),
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();
        to_json(&presets)
    }

    /// Adds a key for a preset, at one of its plans or regions (`region`)
    /// and, for a server of the user's own or an Azure resource, at
    /// `address`; the provider's id.
    pub async fn add(
        preset: String,
        key: String,
        label: String,
        region: String,
        address: String,
    ) -> Result<String, String> {
        off(move || {
            manage::add_preset(
                &preset,
                Some(&key).filter(|k| !k.is_empty()).map(String::as_str),
                &label,
                manage::At {
                    region: Some(region.as_str()).filter(|r| !r.is_empty()),
                    address: Some(address.as_str()).filter(|a| !a.is_empty()),
                },
            )
        })
        .await
    }

    /// Moves a provider to another of its preset's plans or regions.
    pub async fn set_region(id: String, region: String) -> Result<(), String> {
        off(move || manage::set_region(&id, &region)).await
    }

    /// Moves a server of the user's own, or an Azure resource, to `address`.
    pub async fn set_address(id: String, address: String) -> Result<(), String> {
        off(move || manage::set_address(&id, &address)).await
    }

    /// The header its key goes in on the Anthropic API: `x-api-key`,
    /// `bearer`, or empty for both.
    pub async fn set_auth_header(id: String, header: String) -> Result<(), String> {
        off(move || manage::set_auth_header(&id, AuthHeader::parse(&header))).await
    }

    /// The headers sent with every request to it: `{name: value}` (JSON).
    pub async fn set_headers(id: String, headers: String) -> Result<(), String> {
        off(move || {
            let headers: std::collections::BTreeMap<String, String> =
                serde_json::from_str(&headers)?;
            let headers = headers
                .into_iter()
                .map(|(k, v)| (k.trim().to_owned(), v))
                .filter(|(k, _)| !k.is_empty())
                .collect();
            manage::set_headers(&id, headers)
        })
        .await
    }

    /// Puts a provider at `to` (from 0) in the order they are tried in.
    pub async fn move_to(id: String, to: u32) -> Result<(), String> {
        off(move || manage::move_provider(&id, to as usize)).await
    }

    /// Adds a provider of the user's own; `models` one per line or comma.
    pub async fn add_custom(
        name: String,
        protocol: String,
        base_url: String,
        key: String,
        models: String,
    ) -> Result<String, String> {
        off(move || {
            let protocol = Protocol::parse(&protocol)
                .ok_or_else(|| anyhow::anyhow!("unknown protocol {protocol}"))?;
            let models: Vec<String> = models
                .split([',', '\n'])
                .map(str::trim)
                .filter(|m| !m.is_empty())
                .map(str::to_owned)
                .collect();
            manage::add_custom(&name, protocol, &base_url, Some(key.as_str()), &models)
        })
        .await
    }

    /// Removes a provider and its keys.
    pub async fn remove(id: String) -> Result<(), String> {
        off(move || manage::remove_provider(&id)).await
    }

    /// Turns a provider on or off: off, the gateway leaves it out.
    pub async fn set_enabled(id: String, on: bool) -> Result<(), String> {
        off(move || manage::set_provider_enabled(&id, on)).await
    }

    /// Renames a provider (its id stays).
    pub async fn rename(id: String, name: String) -> Result<(), String> {
        off(move || manage::rename_provider(&id, &name)).await
    }

    /// Sets the base URL of one protocol; empty removes it.
    pub async fn set_endpoint(
        id: String,
        protocol: String,
        base_url: String,
    ) -> Result<(), String> {
        off(move || {
            let protocol = Protocol::parse(&protocol)
                .ok_or_else(|| anyhow::anyhow!("unknown protocol {protocol}"))?;
            manage::set_endpoint(&id, protocol, &base_url)
        })
        .await
    }

    /// The provider's own proxy; empty uses the global one.
    pub async fn set_proxy(id: String, proxy: String) -> Result<(), String> {
        off(move || manage::set_proxy(&id, &proxy)).await
    }

    /// One key's own proxy, over the provider's; empty for none of its own.
    pub async fn set_key_proxy(id: String, account: String, proxy: String) -> Result<(), String> {
        off(move || manage::set_key_proxy(&id, &account, &proxy)).await
    }

    /// Adds a key; its id.
    pub async fn add_key(id: String, key: String, label: String) -> Result<String, String> {
        off(move || manage::add_key(&id, &key, &label)).await
    }

    /// Removes a key.
    pub async fn remove_key(id: String, account: String) -> Result<(), String> {
        off(move || manage::remove_key(&id, &account)).await
    }

    /// Turns a key on or off.
    pub async fn set_key_enabled(id: String, account: String, on: bool) -> Result<(), String> {
        off(move || manage::set_key_enabled(&id, &account, on)).await
    }

    /// Moves a key up (`-1`) or down (`1`) the order it is tried in.
    pub async fn move_key(id: String, account: String, by: i32) -> Result<(), String> {
        off(move || manage::move_key(&id, &account, by)).await
    }

    /// Adds a model by its id; it stays when the list is fetched again.
    pub async fn add_model(id: String, model: String) -> Result<(), String> {
        off(move || manage::add_model(&id, &model)).await
    }

    /// Removes a model added by hand.
    pub async fn remove_model(id: String, model: String) -> Result<(), String> {
        off(move || manage::remove_model(&id, &model)).await
    }

    /// Hides a model from pickers, or shows it again.
    pub async fn set_model_hidden(id: String, model: String, hidden: bool) -> Result<(), String> {
        off(move || manage::set_model_hidden(&id, &model, hidden)).await
    }

    /// Fetches the vendor's model list; how many models the provider has.
    pub async fn refresh_models(id: String) -> Result<u32, String> {
        blocking(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| e.to_string())?;
            runtime
                .block_on(manage::refresh_models(&id))
                .map(|n| n as u32)
                .map_err(err)
        })
        .await
        .map_err(|e| e.to_string())?
    }

    /// The other apps' providers to bring over, keys masked: `[{id, name,
    /// path, found, error, items: [{ref, from, name, icon, host, key, keys,
    /// models, status, existing, keyOf, reserved, off, skip, picked,
    /// fingerprint}]}]` (JSON).
    pub async fn import_sources() -> Result<String, String> {
        off(|| {
            Ok(to_json(&pansy_agents::import::view(
                &pansy_agents::import::sources(true),
            )))
        })
        .await
    }

    /// What the Providers page offers to import by itself, unless ignored:
    /// `[{fingerprint, source}]` (JSON).
    pub async fn import_found() -> Result<String, String> {
        off(|| Ok(to_json(&pansy_agents::import::found()?))).await
    }

    /// Stops offering these entries (fingerprints, JSON) until they change.
    pub async fn import_ignore(fingerprints: String) -> Result<(), String> {
        off(move || {
            pansy_agents::import::ignore(&serde_json::from_str::<Vec<String>>(&fingerprints)?)
        })
        .await
    }

    /// Imports the picks (`[{source, ref, mode, name}]`, JSON): `{names,
    /// ids}` (JSON), the new providers' ids to fetch the models of.
    pub async fn import(picks: String) -> Result<String, String> {
        off(move || {
            let picks: Vec<pansy_agents::import::Pick> = serde_json::from_str(&picks)?;
            Ok(to_json(&pansy_agents::import::import(&picks)?))
        })
        .await
    }

    /// What a pasted text is read as (JSON): `{kind: "link", link}` for an
    /// import link (`link` as `import::link::view` shows it, the key
    /// masked), `{kind: "curl"|"env", bases, key, placeholder, model, name,
    /// urlHole}` for a curl command or environment lines, `{kind: ""}` for
    /// neither.
    pub async fn read_paste(text: String) -> Result<String, String> {
        off(move || {
            use pansy_agents::import::{link, paste};
            Ok(if link::is_link(&text) {
                to_json(&serde_json::json!({ "kind": "link", "link": link::view(&text) }))
            } else {
                match paste::read(&text) {
                    Some(p) => to_json(&p),
                    None => to_json(&serde_json::json!({ "kind": "" })),
                }
            })
        })
        .await
    }

    /// Adds the provider an import link describes, read from the link
    /// again, with the name and key given in the window when there are:
    /// `{names, ids}` (JSON).
    pub async fn add_link(link: String, name: String, key: String) -> Result<String, String> {
        off(move || {
            Ok(to_json(&pansy_agents::import::link::add(
                &link, &name, &key,
            )?))
        })
        .await
    }

    /// Every model an agent can be given, groups last:
    /// `[{id, provider, name, group}]` (JSON).
    pub async fn choices() -> Result<String, String> {
        off(|| Ok(to_json(&manage::choices()?))).await
    }
}

// --- Subscriptions --------------------------------------------------------------

/// The subscriptions' accounts signed in from the window, and signed out.
#[module]
pub struct Subscriptions;

fn kind_of(name: &str) -> anyhow::Result<subscription::Kind> {
    subscription::Kind::parse(name).ok_or_else(|| anyhow::anyhow!("no subscription {name}"))
}

#[module]
impl Subscriptions {
    /// Begins signing in to another account of `kind` (`codex`, `copilot`):
    /// in the browser, or by a code typed on the vendor's page (`byCode`).
    /// Resolves once the page is known: `{id, kind, how, url, code, state,
    /// user, plan, account, error}` (JSON).
    pub async fn sign_in(kind: String, by_code: bool) -> Result<String, String> {
        off(move || {
            Ok(to_json(&subscription::flow::start(
                kind_of(&kind)?,
                by_code,
            )?))
        })
        .await
    }

    /// Where a sign-in stands (JSON, as `signIn` gives it); `null` when it is gone.
    pub fn status(id: String) -> String {
        to_json(&subscription::flow::status(&id))
    }

    /// Stops a sign-in that is still waiting.
    pub fn cancel(id: String) -> Result<(), String> {
        subscription::flow::cancel(&id);
        Ok(())
    }

    /// Names the Google Cloud project a Gemini Code Assist account bills;
    /// empty for the one Google gives it.
    pub async fn set_project(kind: String, account: String, project: String) -> Result<(), String> {
        off(move || subscription::set_project(kind_of(&kind)?, &account, &project)).await
    }

    /// What each subscription account has used of its allowance, read
    /// within the last minute or asked again (`fresh`): `[{provider,
    /// account, kind, user, plan, enabled, windows: [{name, used, resetsAt,
    /// spanSecs, display, unlimited}], spentUntil, note, error, readAt}]`
    /// (JSON).
    pub async fn allowances(fresh: bool) -> Result<String, String> {
        blocking(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| e.to_string())?;
            Ok(to_json(
                &runtime.block_on(subscription::allowance::allowances(fresh)),
            ))
        })
        .await
        .map_err(|e| e.to_string())?
    }

    /// What is left on each API key whose balance pansy can read, beside
    /// the allowances: a vendor's own query, or a relay's it finds; read
    /// within the last minute or asked again (`fresh`): `[{provider,
    /// account, name, user, amount, windows, error, readAt}]` (JSON).
    pub async fn balances(fresh: bool) -> Result<String, String> {
        blocking(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| e.to_string())?;
            Ok(to_json(
                &runtime.block_on(pansy_core::balance::balances(fresh)),
            ))
        })
        .await
        .map_err(|e| e.to_string())?
    }

    /// Signs out an account signed in from pansy: its tokens are deleted.
    pub async fn sign_out(kind: String, account: String) -> Result<(), String> {
        off(move || subscription::sign_out(kind_of(&kind)?, &account)).await
    }
}

// --- Plugins --------------------------------------------------------------------

/// magpie's community plugins (`pansy-plugins`): added, switched, signed in
/// to, and brought over from magpie.
#[module]
pub struct Plugins;

/// Runs `fut` on the plugins' runtime, off the engine thread.
async fn plugins_off<R: Send + 'static>(
    fut: impl std::future::Future<Output = anyhow::Result<R>> + Send + 'static,
) -> Result<R, String> {
    blocking(move || pansy_plugins::block(fut).map_err(err))
        .await
        .map_err(|e| e.to_string())?
}

/// Each plugin added, as the Plugins page shows it.
fn plugin_rows() -> anyhow::Result<Value> {
    let listing = pansy_core::plugin::Listing::load();
    let agents = pansy_plugins::agents::cached();
    let rows: Vec<Value> = pansy_plugins::store::installed()?
        .iter()
        .map(|i| {
            let loaded = listing.loaded.iter().find(|l| l.spec == i.entry.spec);
            json!({
                "spec": i.entry.spec,
                "package": i.entry.package(),
                "name": pansy_plugins::store::short_name(&i.entry.package()),
                "version": i.kinds.version,
                "description": i.kinds.description,
                "icon": i.kinds.icon.as_deref().unwrap_or(""),
                "off": i.entry.off,
                "kinds": i.kinds.words(),
                "path": pansy_plugins::store::is_path(&i.entry.spec),
                "options": i.entry.options_value(),
                "example": i.kinds.options,
                "error": loaded.and_then(|l| l.error.clone())
                    .or_else(|| agents.errors.iter().find(|(s, _)| *s == i.entry.spec).map(|(_, e)| e.clone())),
                "middleware": i.kinds.middleware.is_some().then(|| pansy_plugins::middleware::name_of(i)),
                "providers": listing.providers.iter().filter(|p| p.spec == i.entry.spec).map(|p| json!({
                    "id": p.id,
                    "name": p.name,
                    "methods": p.methods,
                    "models": p.models.len(),
                    "listError": p.list_error,
                    "accounts": p.accounts.iter().map(|a| json!({
                        "key": a.key,
                        "kind": a.kind,
                        "user": if a.account_id.is_empty() { a.hint.clone() } else { a.account_id.clone() },
                    })).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
                "agents": agents.agents.iter().filter(|a| a.spec == i.entry.spec).map(|a| json!({"id": a.id, "name": a.name})).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(Value::Array(rows))
}

#[module]
impl Plugins {
    /// The plugins added, in their order: `[{spec, package, name, version,
    /// description, off, kinds, path, options, example, error, middleware,
    /// providers: [{id, name, methods, models, listError, accounts: [{key,
    /// kind, user}]}], agents: [{id, name}]}]` (JSON).
    pub async fn list() -> Result<String, String> {
        off(|| Ok(plugin_rows()?.to_string())).await
    }

    /// Each middleware's calls, failures, time a call and last error, by
    /// its name (JSON); `{}` when none runs.
    pub async fn stats() -> Result<String, String> {
        plugins_off(async { Ok(pansy_plugins::middleware::states().await?.to_string()) }).await
    }

    /// The community plugins, one market as magpie's: `[{package, name,
    /// source, community, publisher, version, weekly, kind, shelf, summary:
    /// {en, zh}, providers, icon, runs}]` (JSON), read again when `fresh`.
    pub async fn discover(fresh: bool) -> Result<String, String> {
        plugins_off(async move { Ok(to_json(&pansy_plugins::market::all(fresh).await)) }).await
    }

    /// npm's OpenCode plugins and pi packages for `query`, as `discover`'s.
    pub async fn search(query: String) -> Result<String, String> {
        plugins_off(async move { Ok(to_json(&pansy_plugins::market::search(&query).await?)) }).await
    }

    /// Adds a plugin: a community plugin's name, an npm spec, a repository
    /// or a path. Its providers are listed again.
    pub async fn add(spec: String) -> Result<String, String> {
        plugins_off(async move {
            let added = pansy_plugins::store::add(&spec).await?;
            if added.kinds.in_host() {
                let _ = pansy_plugins::host::list_now().await;
            }
            if added.kinds.agent.is_some() {
                let _ = pansy_plugins::agents::reload().await;
            }
            Ok(added.entry.package())
        })
        .await
    }

    /// Removes a plugin and uninstalls it.
    pub async fn remove(word: String) -> Result<(), String> {
        plugins_off(async move {
            pansy_plugins::store::remove(&word).await?;
            let _ = pansy_plugins::host::list_now().await;
            let _ = pansy_plugins::agents::reload().await;
            Ok(())
        })
        .await
    }

    /// Switches a plugin on or off.
    pub async fn set_on(word: String, on: bool) -> Result<(), String> {
        plugins_off(async move {
            pansy_plugins::store::set_off(&word, !on)?;
            let _ = pansy_plugins::host::list_now().await;
            let _ = pansy_plugins::agents::reload().await;
            Ok(())
        })
        .await
    }

    /// Updates a plugin (empty: every one); the packages updated (JSON).
    pub async fn update(word: String) -> Result<String, String> {
        plugins_off(async move {
            let done =
                pansy_plugins::store::update_plugins(Some(word.as_str()).filter(|w| !w.is_empty()))
                    .await?;
            let _ = pansy_plugins::host::list_now().await;
            Ok(to_json(&done))
        })
        .await
    }

    /// Sets a plugin's options: a JSON object.
    pub async fn set_options(word: String, options: String) -> Result<(), String> {
        off(move || {
            let value: Value = serde_json::from_str(&options)?;
            pansy_plugins::store::set_options(&word, value).map(|_| ())
        })
        .await
    }

    /// Asks the plugins for their providers and models again.
    pub async fn refresh() -> Result<(), String> {
        plugins_off(async {
            pansy_plugins::host::list_now().await?;
            pansy_plugins::agents::reload().await?;
            Ok(())
        })
        .await
    }

    /// Begins signing in to a plugin's provider by its `method`th way: where
    /// it stands (JSON, `pansy_plugins::signin::Flow`).
    pub async fn sign_in(provider: String, method: u32) -> Result<String, String> {
        plugins_off(async move {
            Ok(to_json(
                &pansy_plugins::signin::start(&provider, method as usize).await?,
            ))
        })
        .await
    }

    /// Answers what a sign-in waits for: a question, a key or a code.
    pub async fn answer(id: String, value: String) -> Result<String, String> {
        plugins_off(async move { Ok(to_json(&pansy_plugins::signin::answer(&id, &value).await?)) })
            .await
    }

    /// Where a sign-in stands (JSON); `null` when it is gone.
    pub fn status(id: String) -> String {
        to_json(&pansy_plugins::signin::status(&id))
    }

    /// Gives a sign-in up.
    pub fn cancel(id: String) -> Result<(), String> {
        pansy_plugins::signin::cancel(&id);
        Ok(())
    }

    /// Signs an account out (empty: every account of the provider).
    pub async fn sign_out(provider: String, account: String) -> Result<(), String> {
        plugins_off(async move {
            pansy_plugins::host::sign_out(
                &provider,
                Some(account.as_str()).filter(|a| !a.is_empty()),
            )
            .await
        })
        .await
    }

    /// The routings middleware give, `<plugin>/<name>` (JSON array).
    pub async fn routings() -> Result<String, String> {
        plugins_off(async { Ok(to_json(&pansy_plugins::middleware::routings().await)) }).await
    }

    /// What magpie has to bring over: `{dir, plugins, accounts}`, or `null`.
    pub async fn magpie() -> Result<String, String> {
        off(|| Ok(to_json(&pansy_plugins::import::found()))).await
    }

    /// Installs magpie's plugins here and copies their sign-ins: `{added,
    /// kept, failed, accounts}` (JSON).
    pub async fn import_magpie() -> Result<String, String> {
        plugins_off(async { Ok(to_json(&pansy_plugins::import::from_magpie().await?)) }).await
    }
}

// --- Agents -------------------------------------------------------------------

/// The coding agents on this computer.
#[module]
pub struct Agents;

#[module]
impl Agents {
    /// Every agent pansy knows, installed or not (JSON, see `pansy_agents::AgentView`).
    pub async fn list() -> Result<String, String> {
        off(|| Ok(to_json(&pansy_agents::list()))).await
    }

    /// The agents' programs as far as they are known within three seconds:
    /// `{clis: {id: {version, via, command, latest, update}}, pending}`
    /// (`pansy_agents::cli::Cli`); pending, some are still being asked (a
    /// version, or over the network the one that is out), and are ready the
    /// next time (JSON).
    pub async fn versions() -> Result<String, String> {
        off(|| {
            let (clis, pending) = pansy_agents::clis(std::time::Duration::from_secs(3));
            Ok(json!({ "clis": clis, "pending": pending }).to_string())
        })
        .await
    }

    /// Updates the agent's program with the command its install tells,
    /// which may take minutes: what it is now (JSON, as in `versions`).
    pub async fn update_cli(id: String) -> Result<String, String> {
        off(move || Ok(to_json(&pansy_agents::update_cli(&id)?))).await
    }

    /// Gives the agent's field `key` a value (a tier's model, an effort);
    /// empty takes it back.
    pub async fn set_field(id: String, key: String, value: String) -> Result<(), String> {
        off(move || {
            pansy_agents::set_field(&id, &key, Some(value.as_str()).filter(|v| !v.is_empty()))
        })
        .await
    }

    /// Picks which of pansy's models the agent lists (JSON, see
    /// `pansy_core::settings::AgentModels`); a connected one's list is
    /// written again. The settings as they are then (JSON).
    pub async fn set_listed(id: String, models: String) -> Result<String, String> {
        off(move || {
            pansy_agents::set_listed(&id, serde_json::from_str(&models)?)?;
            Ok(to_json(&config::load_settings()?))
        })
        .await
    }

    /// Whether the agent's list names a model's provider before the model:
    /// `on`, `off`, or empty to follow Settings again. The settings as they
    /// are then (JSON).
    pub async fn set_names(id: String, mode: String) -> Result<String, String> {
        off(move || {
            let with_provider = match mode.as_str() {
                "" => None,
                "on" => Some(true),
                "off" => Some(false),
                other => anyhow::bail!("{other}: on, off or nothing"),
            };
            pansy_agents::set_names(&id, with_provider)?;
            Ok(to_json(&config::load_settings()?))
        })
        .await
    }

    /// Points the agent at the gateway with `model`.
    pub async fn connect(id: String, model: String) -> Result<(), String> {
        off(move || pansy_agents::connect(&id, &model)).await
    }

    /// Sets the model for the agent's quick tasks; empty follows the main one.
    pub async fn set_small_model(id: String, model: String) -> Result<(), String> {
        off(move || {
            pansy_agents::set_small_model(&id, Some(model.as_str()).filter(|m| !m.is_empty()))
        })
        .await
    }

    /// Sets the model the agent's subagents start on when they name none;
    /// empty leaves that to the agent.
    pub async fn set_subagent_model(id: String, model: String) -> Result<(), String> {
        off(move || {
            pansy_agents::set_subagent_model(&id, Some(model.as_str()).filter(|m| !m.is_empty()))
        })
        .await
    }

    /// Puts back what pansy changed in the agent's config.
    pub async fn disconnect(id: String) -> Result<(), String> {
        off(move || pansy_agents::disconnect(&id)).await
    }

    /// Puts back the agent's config for now, remembering its model.
    pub async fn pause(id: String) -> Result<(), String> {
        off(move || pansy_agents::pause(&id)).await
    }

    /// Connects an agent put back for now again, with its model.
    pub async fn resume(id: String) -> Result<(), String> {
        off(move || pansy_agents::resume(&id)).await
    }

    /// Puts back every connected agent for now: how many.
    pub async fn pause_all() -> Result<u32, String> {
        off(|| Ok(pansy_agents::pause_all()? as u32)).await
    }

    /// Connects every agent put back for now again: how many.
    pub async fn resume_all() -> Result<u32, String> {
        off(|| Ok(pansy_agents::resume_all()? as u32)).await
    }

    /// What connecting the agent with `model` would write, file by file,
    /// keys masked, and nothing written (JSON, see `pansy_agents::FileChange`).
    pub async fn preview(id: String, model: String) -> Result<String, String> {
        off(move || Ok(to_json(&pansy_agents::preview::preview(&id, &model)?))).await
    }

    /// What pansy changed in the agent's config, file by file, keys masked
    /// (JSON, see `pansy_agents::FileChange`).
    pub async fn changes(id: String) -> Result<String, String> {
        off(move || Ok(to_json(&pansy_agents::changes(&id)?))).await
    }

    /// Saves the Agents page's order and the agents hidden on it (JSON
    /// arrays of ids); the settings as they are then (JSON).
    pub async fn arrange(order: String, hidden: String) -> Result<String, String> {
        off(move || {
            let order: Vec<String> = serde_json::from_str(&order)?;
            let hidden: Vec<String> = serde_json::from_str(&hidden)?;
            let settings = config::update_settings(|s| {
                s.agent_order = order;
                s.agents_hidden = hidden;
                s.check().map_err(|e| anyhow::anyhow!(e))?;
                Ok(s.clone())
            })?;
            Ok(to_json(&settings))
        })
        .await
    }
}

// --- Diagnostics --------------------------------------------------------------

/// Settings › Diagnostics: whether the agents can reach their models.
#[module]
pub struct Diagnostics;

#[module]
impl Diagnostics {
    /// Every check (JSON, see `pansy_agents::doctor::Finding`); the
    /// window's gateway's own error where nothing answers.
    pub async fn run() -> Result<String, String> {
        off(|| {
            let mut found = pansy_agents::doctor::run();
            if let Some(error) = gateway_error()
                && let Some(f) = found.iter_mut().find(|f| f.code == "gateway-stopped")
            {
                f.code = "gateway-failed";
                f.args = vec![error];
            }
            Ok(to_json(&found))
        })
        .await
    }

    /// Sends one short message through the gateway to `model` (empty: the
    /// first one an agent can pick), as an agent's request goes (JSON, see
    /// `pansy_agents::doctor::Sent`).
    pub async fn send(model: String) -> Result<String, String> {
        off(move || Ok(to_json(&pansy_agents::doctor::send(Some(&model))?))).await
    }
}

// --- Groups -------------------------------------------------------------------

/// Routing groups.
#[module]
pub struct Groups;

#[module]
impl Groups {
    /// `[{id, name, members, routing}]` (JSON).
    pub async fn list() -> Result<String, String> {
        off(|| Ok(to_json(&config::load_groups()?))).await
    }

    /// Saves a group (new when `id` is empty); `members` is a JSON array. Its id.
    pub async fn save(
        id: String,
        name: String,
        members: String,
        routing: String,
    ) -> Result<String, String> {
        off(move || {
            let members: Vec<String> = serde_json::from_str(&members)?;
            // A middleware's routing (`<plugin>/<name>`) is kept beside the
            // group's own.
            let strategy = routing.contains('/').then(|| routing.clone());
            let routing = Routing::parse(&routing).unwrap_or_default();
            let id = manage::save_group(
                Some(id.as_str()).filter(|i| !i.is_empty()),
                &name,
                &members,
                routing,
            )?;
            manage::set_group_strategy(&id, strategy.as_deref())?;
            Ok(id)
        })
        .await
    }

    /// Deletes a group; the groups that held it lose it.
    pub async fn remove(id: String) -> Result<(), String> {
        off(move || manage::remove_group(&id)).await
    }
}

// --- Profiles -----------------------------------------------------------------

/// Profiles: every agent's model under one name.
#[module]
pub struct Profiles;

#[module]
impl Profiles {
    /// `[{name, agents: {id: {model, smallModel, subagentModel}}, active}]` (JSON).
    pub async fn list() -> Result<String, String> {
        off(|| Ok(to_json(&pansy_agents::profile::list()?))).await
    }

    /// Saves the agents' models as they are now under `name`.
    pub async fn save(name: String) -> Result<(), String> {
        off(move || pansy_agents::profile::save(&name)).await
    }

    /// Puts every agent of the profile on its model; what could not be done (JSON array).
    pub async fn apply(name: String) -> Result<String, String> {
        off(move || Ok(to_json(&pansy_agents::profile::apply(&name)?))).await
    }

    /// Deletes a profile; the agents keep their models.
    pub async fn remove(name: String) -> Result<(), String> {
        off(move || pansy_agents::profile::remove(&name)).await
    }

    /// Renames a profile.
    pub async fn rename(name: String, to: String) -> Result<(), String> {
        off(move || pansy_agents::profile::rename(&name, &to)).await
    }
}

// --- Sessions -----------------------------------------------------------------

/// The agents' sessions on this computer, read from their files.
#[module]
pub struct Sessions;

#[module]
impl Sessions {
    /// `{home, sessions, trash}`: every session, the latest first, and how
    /// many are in the trash (JSON, see `pansy_agents::sessions::Session`).
    pub async fn list() -> Result<String, String> {
        off(|| {
            Ok(to_json(&json!({
                "home": pansy_core::paths::user_home().display().to_string(),
                "sessions": pansy_agents::sessions::list(),
                "trash": pansy_agents::sessions::trash::list().len(),
            })))
        })
        .await
    }

    /// What was said in a session, or in its subagent `sub` (JSON, see
    /// `pansy_agents::sessions::Turn`).
    pub async fn transcript(agent: String, id: String, sub: String) -> Result<String, String> {
        off(move || {
            Ok(to_json(&pansy_agents::sessions::transcript(
                &agent, &id, &sub,
            )?))
        })
        .await
    }

    /// Saves a session's whole conversation as Markdown in the Downloads
    /// folder: the file's path.
    pub async fn export(agent: String, id: String) -> Result<String, String> {
        off(move || {
            let path = pansy_agents::sessions::markdown::export(&agent, &id)?;
            Ok(path.display().to_string())
        })
        .await
    }

    /// The sessions whose prompts or replies say `query`: `[{agent, id, text}]` (JSON).
    pub async fn search(query: String) -> Result<String, String> {
        off(move || Ok(to_json(&pansy_agents::sessions::search(&query, 200)))).await
    }

    /// Moves a session's files into pansy's trash.
    pub async fn remove(agent: String, id: String) -> Result<(), String> {
        off(move || pansy_agents::sessions::trash::delete(&agent, &id).map(|_| ())).await
    }

    /// The sessions in the trash, the latest deleted first (JSON, see
    /// `pansy_agents::sessions::trash::Trashed`).
    pub async fn trash() -> Result<String, String> {
        off(|| Ok(to_json(&pansy_agents::sessions::trash::list()))).await
    }

    /// Puts a session in the trash back where it was.
    pub async fn restore(key: String) -> Result<(), String> {
        off(move || pansy_agents::sessions::trash::restore(&key).map(|_| ())).await
    }

    /// Erases a session in the trash for good.
    pub async fn purge(key: String) -> Result<(), String> {
        off(move || pansy_agents::sessions::trash::purge(&key)).await
    }

    /// Erases everything in the trash; how many sessions were in it.
    pub async fn empty_trash() -> Result<u32, String> {
        off(|| Ok(pansy_agents::sessions::trash::empty()? as u32)).await
    }
}

// --- Usage --------------------------------------------------------------------

/// What went through the gateway.
#[module]
pub struct Usage;

#[module]
impl Usage {
    /// The last `days` days (JSON, see `pansy_core::usage::Summary`).
    pub async fn summary(days: u32) -> Result<String, String> {
        off(move || Ok(to_json(&pansy_core::usage::summary(days.clamp(1, 90))))).await
    }

    /// The last week's days that had requests, newest first: `[{day,
    /// requests}]` (JSON).
    pub async fn days() -> Result<String, String> {
        off(|| {
            let days: Vec<Value> = pansy_core::usage::busy_days(7)
                .into_iter()
                .map(|(day, n)| json!({"day": day, "requests": n}))
                .collect();
            Ok(to_json(&days))
        })
        .await
    }

    /// The requests of `day` (`2026-11-12`) as the Routing page tells them,
    /// oldest first (JSON, see `pansy_gateway::trace::Route`).
    pub async fn day(day: String) -> Result<String, String> {
        off(move || {
            let config = config::Config::load()?;
            let records = pansy_core::usage::day(&day);
            Ok(to_json(&pansy_gateway::trace::from_records(
                &records, &config,
            )))
        })
        .await
    }

    /// Today's tries by account and model: Routing's Accounts and keys
    /// (JSON, see `pansy_gateway::trace::Tally`).
    pub async fn tally() -> Result<String, String> {
        off(|| {
            Ok(to_json(&pansy_gateway::trace::tally(
                &pansy_core::usage::recent(1),
            )))
        })
        .await
    }
}

// --- Masking ------------------------------------------------------------------

/// What masking kept from vendors, and masking tried on the user's text.
#[module]
pub struct Masking;

#[module]
impl Masking {
    /// The last `days` days (JSON, see `pansy_core::masking::Summary`).
    pub async fn summary(days: u32) -> Result<String, String> {
        off(move || Ok(to_json(&pansy_core::masking::summary(days)))).await
    }

    /// What the request `key` said where masking changed it (JSON, a list
    /// of `{path, before, after}`), or `null` once it is no longer in memory.
    pub async fn changes(key: String) -> Result<String, String> {
        let key: u64 = key.parse().map_err(|_| format!("no request {key}"))?;
        off(move || Ok(to_json(&pansy_core::masking::changes(key)))).await
    }

    /// `text` masked as a request would be with the settings as they are,
    /// kept nowhere (JSON: `{text, hits: [{placeholder, kind, action}]}`).
    pub async fn try_it(text: String) -> Result<String, String> {
        off(move || {
            let options = config::load_settings()?.masking;
            let masker = pansy_mask::Masker::new(&pansy_core::masking::key()?);
            let masked = masker.mask_text(&text, &options);
            let hits: Vec<Value> = masked
                .hits
                .iter()
                .map(|h| json!({"placeholder": h.placeholder, "kind": h.kind, "action": h.action}))
                .collect();
            Ok(json!({"text": masked.text, "hits": hits}).to_string())
        })
        .await
    }

    /// The kinds masked for the first time since the last call (JSON, a
    /// list of `[kind, places]`), for the masking alert.
    pub fn firsts() -> String {
        to_json(&pansy_core::masking::take_firsts())
    }

    /// Removes every record.
    pub async fn clear() -> Result<(), String> {
        off(pansy_core::masking::clear).await
    }
}

// --- Settings -----------------------------------------------------------------

/// pansy's settings.
#[module]
pub struct Settings;

#[module]
impl Settings {
    /// Every setting (JSON).
    pub async fn get() -> Result<String, String> {
        off(|| Ok(to_json(&config::load_settings()?))).await
    }

    /// Sets one setting from text (`"8080"`, `"true"`). A new port goes
    /// into the connected agents' configs too.
    pub async fn set(key: String, value: String) -> Result<(), String> {
        off(move || {
            // Where the keys are kept: the keys move with the setting.
            if key == "keyStore" {
                pansy_core::keystore::move_to(&value)?;
                return Ok(());
            }
            config::update_settings(|s| s.set(&key, &value).map_err(|e| anyhow::anyhow!(e)))?;
            // A new port, and the names models are listed by, go into the
            // connected agents' files.
            if key == "port" || key == "providerInNames" {
                let problems = pansy_agents::reconnect_all()?;
                if !problems.is_empty() {
                    anyhow::bail!("{}", problems.join("; "));
                }
            }
            Ok(())
        })
        .await
    }

    /// pansy's directory, where its files are.
    pub fn home() -> String {
        pansy_core::paths::home().display().to_string()
    }

    /// Shows pansy's directory in the system's file manager.
    pub async fn open_home() -> Result<(), String> {
        off(|| {
            let home = pansy_core::paths::home();
            std::fs::create_dir_all(&home)?;
            open::that_detached(&home)?;
            Ok(())
        })
        .await
    }

    /// pansy's version.
    pub fn version() -> String {
        pansy_core::VERSION.to_owned()
    }
}

// --- Language -----------------------------------------------------------------

/// The window's own choices, read before its first paint: its language,
/// whether the sidebar shows, and whether it welcomes first.
#[module]
pub struct Prefs;

#[module]
impl Prefs {
    /// The saved choice: `system`, `en` or `zh-Hans`.
    pub fn language() -> String {
        config::load_settings()
            .map(|s| s.language)
            .unwrap_or_else(|_| "system".to_owned())
    }

    /// Saves the choice.
    pub fn set_language(language: String) -> Result<(), String> {
        config::update_settings(|s| s.set("language", &language).map_err(|e| anyhow::anyhow!(e)))
            .map_err(err)
    }

    /// Whether the sidebar shows (`sidebar`); hidden, the pages are tabs in
    /// the title bar.
    pub fn sidebar() -> bool {
        config::load_settings().map(|s| s.sidebar).unwrap_or(true)
    }

    /// Saves it.
    pub fn set_sidebar(on: bool) -> Result<(), String> {
        config::update_settings(|s| {
            s.sidebar = on;
            Ok(())
        })
        .map_err(err)
    }

    /// Whether pansy has no provider yet: the welcome is shown to a pansy
    /// new to this computer, never to one already in use.
    pub fn fresh() -> bool {
        config::load_providers()
            .map(|p| p.is_empty())
            .unwrap_or(false)
    }

    /// This computer's name as its owner gave it (`tsui's MacBook Air`),
    /// else its host name; empty when neither is known.
    pub fn computer_name() -> String {
        computer_name().unwrap_or_default()
    }

    /// The locale the window uses now: `en` or `zh-Hans`.
    pub fn locale() -> String {
        let choice = Prefs::language();
        if choice != "system" {
            return choice;
        }
        let system = [
            "PANSY_LANGUAGE",
            "LC_ALL",
            "LC_MESSAGES",
            "LANG",
            "LANGUAGE",
        ]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| !v.is_empty())
        .unwrap_or_default();
        if system.to_ascii_lowercase().starts_with("zh") {
            "zh-Hans".to_owned()
        } else {
            "en".to_owned()
        }
    }
}

/// The Mac's Sharing name, systemd's pretty host name, Windows' computer
/// name; else the host name.
fn computer_name() -> Option<String> {
    let given = |s: String| Some(s.trim().trim_matches('"').to_owned()).filter(|s| !s.is_empty());
    #[cfg(target_os = "macos")]
    if let Some(name) = std::process::Command::new("scutil")
        .args(["--get", "ComputerName"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| given(String::from_utf8_lossy(&o.stdout).into_owned()))
    {
        return Some(name);
    }
    #[cfg(windows)]
    if let Some(name) = std::env::var("COMPUTERNAME").ok().and_then(given) {
        return Some(name);
    }
    if let Some(name) = std::fs::read_to_string("/etc/machine-info")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("PRETTY_HOSTNAME=").map(str::to_owned))
        })
        .and_then(given)
    {
        return Some(name);
    }
    ["/proc/sys/kernel/hostname", "/etc/hostname"]
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok().and_then(given))
        .or_else(|| std::env::var("HOSTNAME").ok().and_then(given))
}

// --- The quick panel ------------------------------------------------------------

/// Where the quick panel by the tray icon opens, and when a click outside
/// closes it (Hyprland; elsewhere the system places it and losing the focus
/// closes it).
#[module]
pub struct Panel;

#[module]
impl Panel {
    /// Readies the panel titled `title`, `width` points wide, to open under
    /// the bar at the pointer; resolves with the room it has down to the
    /// screen's foot, 0 where the system places it.
    pub async fn place(title: String, width: u32) -> Result<u32, String> {
        let width = i32::try_from(width).unwrap_or(i32::MAX);
        off(move || panel::place(&title, width).map(|room| room.max(0).unsigned_abs())).await
    }

    /// The panel closed: clicks are no longer announced, and `outsideClick`
    /// resolves `false`.
    pub async fn closed() -> Result<(), String> {
        off(panel::closed).await
    }

    /// Resolves `true` at the first click outside the open panel titled
    /// `title` (below the bar), `false` once it has closed.
    pub async fn outside_click(title: String) -> Result<bool, String> {
        off(move || Ok(panel::wait_outside(&title))).await
    }
}
