//! Which provider, account and model answer a request: the model it names
//! (`provider/model`, `group/<id>`, or a bare model id) opened into the
//! attempts the gateway makes in order, keys resting after a failure last.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use pansy_core::config::Config;
use pansy_core::group::{self, Routing};
use pansy_core::plugin::PluginRef;
use pansy_core::provider::{Account, AuthHeader, Endpoint, Protocol, Provider, Quirks};
use pansy_core::subscription::Kind;

/// One way to answer: a provider's endpoint, one of its keys, a model.
#[derive(Debug, Clone)]
pub struct Attempt {
    pub provider: String,
    pub preset: Option<String>,
    /// `None` for a local server that takes no key.
    pub account: Option<Account>,
    pub endpoint: Endpoint,
    pub model: String,
    pub headers: BTreeMap<String, String>,
    pub proxy: Option<String>,
    pub quirks: Quirks,
    /// The one header the key goes in on the Anthropic API; none for both.
    pub auth: Option<AuthHeader>,
    /// Azure OpenAI, which takes the key in `api-key`.
    pub azure: bool,
    /// The subscription whose sign-in the account is: its request is signed
    /// with the account's tokens (`pansy_core::subscription::sign`).
    pub subscription: Option<Kind>,
    /// The plugin that serves the provider: its request goes through the
    /// plugin's host (`pansy_plugins::host::fetch`).
    pub plugin: Option<PluginRef>,
    /// The model's id where it is asked, when that isn't `model`.
    pub api_id: Option<String>,
}

impl Attempt {
    /// The model as the vendor's request names it.
    pub fn wire_model(&self) -> &str {
        self.api_id.as_deref().unwrap_or(&self.model)
    }

    pub fn key_id(&self) -> String {
        format!(
            "{}:{}",
            self.provider,
            self.account.as_ref().map(|a| a.id.as_str()).unwrap_or("-")
        )
    }
}

/// What the gateway remembers between requests: keys resting after a
/// failure, and where rotating groups are.
#[derive(Debug, Default)]
pub struct Health {
    rests: Mutex<HashMap<String, Instant>>,
    turns: Mutex<HashMap<String, usize>>,
    used: Mutex<HashMap<String, u64>>,
}

impl Health {
    /// Rests a key: it is tried after the others until `rest` has passed.
    pub fn rest(&self, key_id: &str, rest: Duration) {
        let until = Instant::now() + rest;
        self.rests.lock().unwrap().insert(key_id.to_owned(), until);
    }

    /// A key answered: it rests no more.
    pub fn healthy(&self, key_id: &str) {
        self.rests.lock().unwrap().remove(key_id);
    }

    pub fn resting(&self, key_id: &str) -> bool {
        self.rest_left(key_id) > 0
    }

    /// The seconds `key_id` has left to rest, rounded up; 0 when it is ready.
    pub fn rest_left(&self, key_id: &str) -> u64 {
        let now = Instant::now();
        self.rests
            .lock()
            .unwrap()
            .get(key_id)
            .filter(|until| **until > now)
            .map_or(0, |until| (*until - now).as_millis().div_ceil(1000) as u64)
    }

    /// Keys resting now, with the seconds left.
    pub fn resting_keys(&self) -> Vec<(String, u64)> {
        let now = Instant::now();
        let mut out: Vec<(String, u64)> = self
            .rests
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, until)| **until > now)
            .map(|(k, until)| (k.clone(), (*until - now).as_secs()))
            .collect();
        out.sort();
        out
    }

    /// A member answered a request.
    pub fn count(&self, member: &str) {
        *self
            .used
            .lock()
            .unwrap()
            .entry(member.to_owned())
            .or_default() += 1;
    }

    fn next_turn(&self, group: &str, len: usize) -> usize {
        let mut turns = self.turns.lock().unwrap();
        let turn = turns.entry(group.to_owned()).or_default();
        let at = *turn % len.max(1);
        *turn = turn.wrapping_add(1);
        at
    }

    /// How many requests `member` (`provider/model`) has answered.
    pub fn used(&self, member: &str) -> u64 {
        self.used.lock().unwrap().get(member).copied().unwrap_or(0)
    }
}

/// Why no attempt could be planned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoRoute(pub String);

/// `model` without what some agents add after it (`[1m]` for a long context).
fn bare(model: &str) -> &str {
    let model = model.trim();
    match model.find('[') {
        Some(i) if model.ends_with(']') => &model[..i],
        _ => model,
    }
}

/// A group's models in the order to try them: its own routing over its
/// members (a group in it counts as one), and each group in it opened in the
/// order its own routing gives. Each model comes once; a group inside itself
/// stops there.
fn group_members(
    config: &Config,
    id: &str,
    health: &Health,
    orders: &HashMap<String, Vec<usize>>,
    seen: &mut Vec<String>,
    out: &mut Vec<String>,
) {
    if seen.iter().any(|s| s == id) {
        return;
    }
    seen.push(id.to_owned());
    let Some(group) = config.group(id) else {
        return;
    };
    let mut direct: Vec<&String> = group.members.iter().collect();
    // A middleware's routing orders them when it served one (a permutation
    // of their indexes); one that serves none leaves the listed order.
    if let Some(strategy) = &group.strategy {
        if let Some(order) = orders.get(id).filter(|o| o.len() == direct.len()) {
            direct = order.iter().map(|&i| &group.members[i]).collect();
        }
        tracing::debug!(group = %id, %strategy, "ordered by a middleware's routing");
    } else {
        match group.routing {
            Routing::Order => {}
            Routing::Rotate => {
                if !direct.is_empty() {
                    let at = health.next_turn(id, direct.len());
                    direct.rotate_left(at);
                }
            }
            Routing::Usage => direct.sort_by_key(|m| match m.strip_prefix("group/") {
                Some(inner) => group::flatten(&config.groups, inner)
                    .iter()
                    .map(|m| health.used(m))
                    .sum(),
                None => health.used(m),
            }),
        }
    }
    for member in direct {
        match member.strip_prefix("group/") {
            Some(inner) => group_members(config, inner, health, orders, seen, out),
            None => {
                if !out.contains(member) {
                    out.push(member.clone());
                }
            }
        }
    }
}

/// The members a requested model stands for, as `provider/model`, in the
/// order to try them.
pub fn members(config: &Config, requested: &str, health: &Health) -> Result<Vec<String>, NoRoute> {
    members_with(config, requested, health, &HashMap::new())
}

/// The groups a requested model opens into whose members a middleware's
/// routing orders: each one's id, routing and direct members.
pub fn strategy_pools(config: &Config, requested: &str) -> Vec<(String, String, Vec<String>)> {
    fn walk(
        config: &Config,
        id: &str,
        seen: &mut Vec<String>,
        out: &mut Vec<(String, String, Vec<String>)>,
    ) {
        if seen.iter().any(|s| s == id) {
            return;
        }
        seen.push(id.to_owned());
        let Some(group) = config.group(id) else {
            return;
        };
        if let Some(strategy) = &group.strategy {
            out.push((id.to_owned(), strategy.clone(), group.members.clone()));
        }
        for m in &group.members {
            if let Some(inner) = m.strip_prefix("group/") {
                walk(config, inner, seen, out);
            }
        }
    }
    let mut out = Vec::new();
    if let Some(id) = bare(requested).strip_prefix("group/") {
        walk(config, id, &mut Vec::new(), &mut out);
    }
    out
}

/// [`members`], each group a middleware's routing orders in `orders`'
/// order (by group id: a permutation of its members' indexes).
pub fn members_with(
    config: &Config,
    requested: &str,
    health: &Health,
    orders: &HashMap<String, Vec<usize>>,
) -> Result<Vec<String>, NoRoute> {
    let requested = bare(requested);
    if requested.is_empty() {
        return Err(NoRoute("the request names no model".into()));
    }
    if let Some(id) = requested.strip_prefix("group/") {
        let group = config
            .group(id)
            .ok_or_else(|| NoRoute(format!("there is no routing group named {id}")))?;
        let mut members = Vec::new();
        group_members(config, id, health, orders, &mut Vec::new(), &mut members);
        if members.is_empty() {
            return Err(NoRoute(format!("the group {} has no members", group.name)));
        }
        return Ok(members);
    }
    if let Some((provider, model)) = requested.split_once('/')
        && config.provider(provider).is_some()
    {
        return Ok(vec![format!("{provider}/{model}")]);
    }
    // A bare model id: every provider that lists it, in the user's order.
    let offering: Vec<String> = config
        .providers
        .iter()
        .filter(|p| p.enabled && p.has_model(requested))
        .map(|p| format!("{}/{requested}", p.id))
        .collect();
    if offering.is_empty() {
        return Err(NoRoute(format!(
            "no provider offers {requested}: name it as provider/model, or add it to a provider"
        )));
    }
    Ok(offering)
}

/// The protocol to speak to `provider` for a client speaking `client`: its
/// own if the provider has it (the request passes through), else the
/// nearest one.
pub fn pick_endpoint(provider: &Provider, client: Protocol) -> Option<&Endpoint> {
    let order: [Protocol; 4] = match client {
        Protocol::Anthropic => [
            Protocol::Anthropic,
            Protocol::Chat,
            Protocol::Responses,
            Protocol::Gemini,
        ],
        Protocol::Chat => [
            Protocol::Chat,
            Protocol::Responses,
            Protocol::Anthropic,
            Protocol::Gemini,
        ],
        Protocol::Responses => [
            Protocol::Responses,
            Protocol::Chat,
            Protocol::Anthropic,
            Protocol::Gemini,
        ],
        Protocol::Gemini => [
            Protocol::Gemini,
            Protocol::Chat,
            Protocol::Anthropic,
            Protocol::Responses,
        ],
    };
    order.iter().find_map(|p| provider.endpoint(*p))
}

/// The protocol to speak to `provider` for `model`: among the APIs the
/// model is served on, when its list says (Copilot's), the nearest to the
/// client's.
pub fn pick_endpoint_for<'a>(
    provider: &'a Provider,
    model: &str,
    client: Protocol,
) -> Option<&'a Endpoint> {
    let apis = provider
        .models
        .iter()
        .find(|m| m.id == model)
        .map(|m| m.apis.as_slice())
        .unwrap_or_default();
    if apis.is_empty() {
        return pick_endpoint(provider, client);
    }
    std::iter::once(client)
        .chain(Protocol::ALL)
        .filter(|p| apis.contains(p))
        .find_map(|p| provider.endpoint(p))
        .or_else(|| pick_endpoint(provider, client))
}

/// The attempts for `requested`: each member's keys in order, those resting
/// after the rest. A request makes the first `max_attempts` (Settings).
pub fn plan(
    config: &Config,
    requested: &str,
    client: Protocol,
    health: &Health,
) -> Result<Vec<Attempt>, NoRoute> {
    plan_with(config, requested, client, health, &HashMap::new())
}

/// [`plan`], with the groups middleware routings order in `orders`.
pub fn plan_with(
    config: &Config,
    requested: &str,
    client: Protocol,
    health: &Health,
    orders: &HashMap<String, Vec<usize>>,
) -> Result<Vec<Attempt>, NoRoute> {
    let members = members_with(config, requested, health, orders)?;
    let mut fresh = Vec::new();
    let mut resting = Vec::new();
    let mut problems = Vec::new();
    for member in &members {
        let (provider_id, model) = member.split_once('/').unwrap_or((member, ""));
        let Some(provider) = config.provider(provider_id) else {
            problems.push(format!("there is no provider {provider_id}"));
            continue;
        };
        if !provider.enabled {
            problems.push(format!("{} is turned off", provider.name));
            continue;
        }
        let Some(endpoint) = pick_endpoint_for(provider, model, client) else {
            problems.push(format!("{} has no endpoint", provider.name));
            continue;
        };
        let base = Attempt {
            provider: provider.id.clone(),
            preset: provider.preset.clone(),
            account: None,
            endpoint: endpoint.clone(),
            model: model.to_owned(),
            headers: provider.headers.clone(),
            proxy: provider.proxy.clone(),
            quirks: provider.quirks.clone(),
            auth: provider.anthropic_auth(),
            azure: provider.is_azure(),
            subscription: provider.subscription,
            plugin: provider.plugin.clone(),
            api_id: provider
                .models
                .iter()
                .find(|m| m.id == model)
                .and_then(|m| m.api_id.clone()),
        };
        let accounts: Vec<&Account> = provider.enabled_accounts().collect();
        if accounts.is_empty() {
            if provider.keyless() {
                fresh.push(base);
            } else if provider.subscription.is_some() || provider.plugin.is_some() {
                problems.push(format!("{} has no account switched on", provider.name));
            } else {
                problems.push(format!("{} has no API key", provider.name));
            }
            continue;
        }
        for account in accounts {
            let mut attempt = base.clone();
            if account.proxy.is_some() {
                attempt.proxy = account.proxy.clone();
            }
            attempt.account = Some(account.clone());
            if health.resting(&attempt.key_id()) {
                resting.push(attempt);
            } else {
                fresh.push(attempt);
            }
        }
    }
    fresh.extend(resting);
    if fresh.is_empty() {
        return Err(NoRoute(problems.join("; ")));
    }
    Ok(fresh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pansy_core::group::Group;
    use pansy_core::presets;

    fn config() -> Config {
        let mut deepseek = Provider::from_preset(presets::find("deepseek").unwrap(), Some("sk-1"));
        deepseek.add_account("sk-2", "");
        for m in ["deepseek-chat", "deepseek-reasoner"] {
            deepseek.models.push(pansy_core::provider::Model::new(m));
        }
        let mut moonshot = Provider::from_preset(presets::find("moonshot").unwrap(), Some("sk-m"));
        moonshot
            .models
            .push(pansy_core::provider::Model::new("kimi-k2-0905-preview"));
        Config {
            providers: vec![deepseek, moonshot],
            groups: vec![Group {
                id: "daily".into(),
                name: "Daily".into(),
                members: vec![
                    "deepseek/deepseek-chat".into(),
                    "moonshot/kimi-k2-0905-preview".into(),
                ],
                routing: Routing::Rotate,
                strategy: None,
            }],
            settings: Default::default(),
        }
    }

    #[test]
    fn plans_each_key_then_the_resting_ones() {
        let config = config();
        let health = Health::default();
        health.rest("deepseek:k1", Duration::from_secs(60));
        let plan = plan(
            &config,
            "deepseek/deepseek-chat",
            Protocol::Anthropic,
            &health,
        )
        .unwrap();
        let keys: Vec<String> = plan.iter().map(Attempt::key_id).collect();
        assert_eq!(keys, ["deepseek:k2", "deepseek:k1"]);
        // DeepSeek answers Anthropic itself: the request passes through.
        assert_eq!(plan[0].endpoint.protocol, Protocol::Anthropic);
        assert_eq!(plan[0].model, "deepseek-chat");
    }

    #[test]
    fn rotates_a_group_and_finds_bare_ids() {
        let config = config();
        let health = Health::default();
        let first = members(&config, "group/daily", &health).unwrap();
        let second = members(&config, "group/daily", &health).unwrap();
        assert_eq!(first[0], "deepseek/deepseek-chat");
        assert_eq!(second[0], "moonshot/kimi-k2-0905-preview");
        assert_eq!(
            members(&config, "deepseek-chat[1m]", &health).unwrap(),
            ["deepseek/deepseek-chat"]
        );
        assert!(members(&config, "nope", &health).is_err());
        assert!(members(&config, "group/none", &health).is_err());
    }

    #[test]
    fn a_model_goes_to_an_api_it_is_served_on() {
        let mut copilot = pansy_core::subscription::provider(Kind::Copilot);
        copilot.models = vec![
            pansy_core::provider::Model {
                apis: vec![Protocol::Responses],
                ..pansy_core::provider::Model::new("gpt-5.5")
            },
            pansy_core::provider::Model::new("gpt-4.1"),
        ];
        let at =
            |model| pick_endpoint_for(&copilot, model, Protocol::Anthropic).map(|e| e.protocol);
        assert_eq!(at("gpt-5.5"), Some(Protocol::Responses));
        assert_eq!(at("gpt-4.1"), Some(Protocol::Anthropic));
        // A subscription with no account on says so.
        let config = Config {
            providers: vec![copilot],
            groups: Vec::new(),
            settings: Default::default(),
        };
        let no = plan(
            &config,
            "copilot/gpt-5.5",
            Protocol::Chat,
            &Health::default(),
        );
        assert_eq!(no.unwrap_err().0, "Copilot has no account switched on");
    }

    #[test]
    fn a_middlewares_routing_orders_its_group() {
        let mut config = config();
        config.groups[0].strategy = Some("guard/reverse".into());
        let pools = strategy_pools(&config, "group/daily");
        assert_eq!(pools.len(), 1);
        assert_eq!(pools[0].1, "guard/reverse");
        let health = Health::default();
        let orders = HashMap::from([("daily".to_owned(), vec![1, 0])]);
        let members = members_with(&config, "group/daily", &health, &orders).unwrap();
        assert_eq!(members[0], "moonshot/kimi-k2-0905-preview");
        // No plugin serves it: the listed order, not the group's rotation.
        for _ in 0..2 {
            assert_eq!(
                members_with(&config, "group/daily", &health, &HashMap::new()).unwrap()[0],
                "deepseek/deepseek-chat"
            );
        }
    }

    #[test]
    fn a_group_inside_another_routes_its_own_way() {
        let mut config = config();
        config.groups.push(Group {
            id: "wrapper".into(),
            name: "Wrapper".into(),
            members: vec!["group/daily".into(), "deepseek/deepseek-reasoner".into()],
            routing: Routing::Order,
            strategy: None,
        });
        let health = Health::default();
        let firsts: Vec<String> = (0..4)
            .map(|_| members(&config, "group/wrapper", &health).unwrap()[0].clone())
            .collect();
        assert_eq!(
            firsts,
            [
                "deepseek/deepseek-chat",
                "moonshot/kimi-k2-0905-preview",
                "deepseek/deepseek-chat",
                "moonshot/kimi-k2-0905-preview"
            ]
        );
        let all = members(&config, "group/wrapper", &health).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[2], "deepseek/deepseek-reasoner");
    }
}
