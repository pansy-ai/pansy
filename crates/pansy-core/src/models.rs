//! A provider's model list, as its vendor tells it: `GET /models` in the
//! OpenAI, Anthropic or Gemini shape.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::provider::{AuthHeader, Model, Protocol, Provider};

/// Asks the vendor for its models, with the provider's first enabled key (a
/// subscription, with its first account's sign-in).
/// A vendor (or a plan) whose preset says it has no list is not asked; a
/// plan sold on a key that buys much else keeps its own models alone.
pub async fn fetch(provider: &Provider) -> Result<Vec<Model>> {
    if provider.plugin.is_some() {
        bail!(
            "{}'s models are its plugin's: `pansy plugin refresh` lists them again",
            provider.name
        );
    }
    if provider.subscription.is_some() {
        return crate::subscription::models(provider).await;
    }
    if !provider.lists_models() {
        bail!(
            "{} does not list its models: its usual ones are there, and others are added by hand",
            provider.name
        );
    }
    let only = provider.preset().map_or("", |p| p.only);
    let mut models = ask(provider).await?;
    models.retain(|m| m.id.starts_with(only));
    if models.is_empty() {
        bail!("{} listed none of its plan's models", provider.name);
    }
    Ok(models)
}

async fn ask(provider: &Provider) -> Result<Vec<Model>> {
    let key = provider
        .enabled_accounts()
        .next()
        .map(|a| a.key.clone())
        .unwrap_or_default();
    // The account's proxy, else the provider's, else pansy's.
    let proxy = provider
        .enabled_accounts()
        .next()
        .and_then(|a| a.proxy.clone())
        .or_else(|| provider.proxy.clone())
        .filter(|p| !p.is_empty())
        .or_else(|| crate::config::load_settings().ok().map(|s| s.proxy))
        .unwrap_or_default();
    let builder = reqwest::Client::builder()
        .user_agent(concat!("pansy/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(20));
    let client = crate::settings::use_proxy(builder, &proxy)
        .map_err(anyhow::Error::msg)?
        .build()?;
    // The protocols in the order their lists are most complete.
    let order = [
        Protocol::Chat,
        Protocol::Responses,
        Protocol::Anthropic,
        Protocol::Gemini,
    ];
    let mut last = None;
    for protocol in order {
        let Some(endpoint) = provider.endpoint(protocol) else {
            continue;
        };
        let base = endpoint.base_url.trim_end_matches('/');
        let request = match protocol {
            // Azure's v1 API lists the resource's deployments, with the
            // key in api-key.
            Protocol::Chat | Protocol::Responses if provider.is_azure() => {
                client.get(format!("{base}/models")).header("api-key", &key)
            }
            Protocol::Chat | Protocol::Responses => {
                client.get(format!("{base}/models")).bearer_auth(&key)
            }
            Protocol::Anthropic => {
                let url = if base.ends_with("/v1") {
                    format!("{base}/models")
                } else {
                    format!("{base}/v1/models")
                };
                let r = client.get(url).header("anthropic-version", "2023-06-01");
                match provider.anthropic_auth() {
                    Some(AuthHeader::XApiKey) => r.header("x-api-key", &key),
                    Some(AuthHeader::Bearer) => r.bearer_auth(&key),
                    None => r.header("x-api-key", &key).bearer_auth(&key),
                }
            }
            Protocol::Gemini => {
                let root = if base.ends_with("/v1beta") {
                    base.to_owned()
                } else {
                    format!("{base}/v1beta")
                };
                client
                    .get(format!("{root}/models?pageSize=1000"))
                    .header("x-goog-api-key", &key)
            }
        };
        // The provider's own headers, as every request to it has them.
        let request = provider
            .headers
            .iter()
            .fold(request, |r, (name, value)| r.header(name, value));
        match request.send().await {
            Ok(response) if response.status().is_success() => {
                let body: Value = response
                    .json()
                    .await
                    .context("the model list is not JSON")?;
                let models = parse(&body);
                if !models.is_empty() {
                    return Ok(models);
                }
                if last.is_none() {
                    last = Some(anyhow::anyhow!("{} listed no models", provider.name));
                }
            }
            Ok(response) => {
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                // The first endpoint's answer says most: the others are
                // the vendor's other protocols, often with no list at all.
                if last.is_none() {
                    last = Some(anyhow::anyhow!(
                        "{status}: {}",
                        text.chars().take(300).collect::<String>()
                    ));
                }
            }
            Err(e) => {
                if last.is_none() {
                    last = Some(e.into());
                }
            }
        }
    }
    match last {
        Some(e) => Err(e.context(format!("listing {}'s models", provider.name))),
        None => bail!("{} has no endpoint that lists models", provider.name),
    }
}

/// Models of a list in any of the vendors' shapes.
pub fn parse(body: &Value) -> Vec<Model> {
    let items = body
        .get("data")
        .or_else(|| body.get("models"))
        .and_then(Value::as_array)
        .cloned()
        .or_else(|| body.as_array().cloned())
        .unwrap_or_default();
    let mut models: Vec<Model> = items
        .iter()
        .filter_map(|item| {
            // Gemini's OpenAI-compatible list says `models/gemini-2.5-pro`.
            let id = item
                .get("id")
                .and_then(Value::as_str)
                .map(|n| n.strip_prefix("models/").unwrap_or(n))
                .or_else(|| {
                    item.get("name")
                        .and_then(Value::as_str)
                        .map(|n| n.strip_prefix("models/").unwrap_or(n))
                })
                .or_else(|| item.get("model").and_then(Value::as_str))?;
            // Gemini lists embedding and image models too.
            if let Some(methods) = item
                .get("supportedGenerationMethods")
                .and_then(Value::as_array)
                && !methods.iter().any(|m| m == "generateContent")
            {
                return None;
            }
            let name = item
                .get("display_name")
                .or_else(|| item.get("displayName"))
                .or_else(|| item.get("name").filter(|n| n.as_str() != Some(id)))
                .and_then(Value::as_str)
                .filter(|n| !n.starts_with("models/"))
                .unwrap_or_default();
            let context = item
                .get("context_length")
                .or_else(|| item.get("context_window"))
                .or_else(|| item.get("inputTokenLimit"))
                .or_else(|| item.get("max_context_length"))
                .and_then(Value::as_u64);
            let max_output = item
                .get("outputTokenLimit")
                .or_else(|| item.get("max_output_tokens"))
                .or_else(|| item.pointer("/top_provider/max_completion_tokens"))
                .and_then(Value::as_u64);
            Some(Model {
                id: id.to_owned(),
                name: name.to_owned(),
                context,
                max_output,
                reasoning: item
                    .get("thinking")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                vision: false,
                apis: Vec::new(),
                api_id: None,
            })
        })
        .collect();
    models.sort_by(|a, b| a.id.cmp(&b.id));
    models.dedup_by(|a, b| a.id == b.id);
    models
}

/// Keeps what the user set by hand on models the vendor lists again, and
/// models the user added that the vendor does not list.
pub fn merge(old: &[Model], fetched: Vec<Model>, manual: &[String]) -> Vec<Model> {
    let mut out: Vec<Model> = fetched
        .into_iter()
        .map(|mut m| {
            if let Some(prev) = old.iter().find(|o| o.id == m.id) {
                if m.name.is_empty() {
                    m.name = prev.name.clone();
                }
                m.context = m.context.or(prev.context);
                m.max_output = m.max_output.or(prev.max_output);
                m.reasoning |= prev.reasoning;
                m.vision |= prev.vision;
            }
            m
        })
        .collect();
    for id in manual {
        if !out.iter().any(|m| &m.id == id) {
            if let Some(prev) = old.iter().find(|o| &o.id == id) {
                out.push(prev.clone());
            } else {
                out.push(Model::new(id));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_each_vendors_list() {
        let openai =
            json!({"object": "list", "data": [{"id": "b"}, {"id": "a", "context_length": 128000}]});
        let ids: Vec<String> = parse(&openai).into_iter().map(|m| m.id).collect();
        assert_eq!(ids, ["a", "b"]);
        let anthropic =
            json!({"data": [{"type": "model", "id": "claude-x", "display_name": "Claude X"}]});
        assert_eq!(parse(&anthropic)[0].name, "Claude X");
        let gemini = json!({"models": [
            {"name": "models/gemini-2.5-pro", "displayName": "Gemini 2.5 Pro", "inputTokenLimit": 1048576, "supportedGenerationMethods": ["generateContent"]},
            {"name": "models/embedding-001", "supportedGenerationMethods": ["embedContent"]}
        ]});
        let g = parse(&gemini);
        assert_eq!(g.len(), 1);
        assert_eq!(
            (g[0].id.as_str(), g[0].context),
            ("gemini-2.5-pro", Some(1048576))
        );
    }

    #[test]
    fn keeps_models_added_by_hand() {
        let old = vec![Model {
            id: "mine".into(),
            name: "Mine".into(),
            ..Model::default()
        }];
        let merged = merge(&old, vec![Model::new("theirs")], &["mine".to_owned()]);
        assert_eq!(
            merged.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["theirs", "mine"]
        );
        assert_eq!(merged[1].name, "Mine");
    }
}
