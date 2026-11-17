//! Profiles: every agent's model saved under one name ("Budget",
//! "Focus") and put back in one move. Saved in `profiles.json`.

use std::collections::BTreeMap;

use anyhow::{Result, anyhow, bail};
use pansy_core::fsutil::{read_json, write_json};
use pansy_core::paths;
use serde::{Deserialize, Serialize};

use crate::state::State;
use crate::view;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Choice {
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub small_model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    /// Agent id → its models.
    pub agents: BTreeMap<String, Choice>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    profiles: Vec<Profile>,
}

/// A profile and whether the agents are on it now.
#[derive(Debug, Clone, Serialize)]
pub struct Listed {
    #[serde(flatten)]
    pub profile: Profile,
    pub active: bool,
}

fn load() -> Result<Vec<Profile>> {
    Ok(read_json::<File>(&paths::profiles())?.profiles)
}

fn store(profiles: Vec<Profile>) -> Result<()> {
    write_json(&paths::profiles(), &File { profiles }, false)
}

/// The agents connected now, with their models.
fn current() -> BTreeMap<String, Choice> {
    let state = State::load().unwrap_or_default();
    crate::agent::all()
        .into_iter()
        .map(|a| view(a, &state))
        .filter_map(|v| {
            Some((
                v.id.to_owned(),
                Choice {
                    model: v.model.clone()?,
                    small_model: v.small_model.clone(),
                },
            ))
        })
        .collect()
}

pub fn list() -> Result<Vec<Listed>> {
    let now = current();
    Ok(load()?
        .into_iter()
        .map(|p| {
            let active = !p.agents.is_empty()
                && p.agents
                    .iter()
                    .all(|(id, c)| now.get(id).is_some_and(|n| n.model == c.model));
            Listed { profile: p, active }
        })
        .collect())
}

/// Saves the agents' models as they are now under `name` (replacing a
/// profile of that name).
pub fn save(name: &str) -> Result<()> {
    let name = name.trim();
    if name.is_empty() {
        bail!("a profile needs a name");
    }
    let agents = current();
    if agents.is_empty() {
        bail!("no agent is connected to pansy: connect some, then save them");
    }
    let mut profiles = load()?;
    profiles.retain(|p| p.name != name);
    profiles.push(Profile {
        name: name.to_owned(),
        agents,
    });
    store(profiles)
}

/// Puts each agent of the profile on its models; what could not be done.
pub fn apply(name: &str) -> Result<Vec<String>> {
    let profile = load()?
        .into_iter()
        .find(|p| p.name == name)
        .ok_or_else(|| anyhow!("there is no profile {name}"))?;
    let mut problems = Vec::new();
    for (id, choice) in &profile.agents {
        let result = crate::connect(id, &choice.model).and_then(|_| match &choice.small_model {
            Some(small) => crate::set_small_model(id, Some(small)),
            None => Ok(()),
        });
        if let Err(e) = result {
            problems.push(format!("{id}: {e:#}"));
        }
    }
    Ok(problems)
}

/// Renames a profile; what it holds stays.
pub fn rename(name: &str, to: &str) -> Result<()> {
    let to = to.trim();
    if to.is_empty() {
        bail!("a profile needs a name");
    }
    let mut profiles = load()?;
    if to != name && profiles.iter().any(|p| p.name == to) {
        bail!("there is a profile {to} already");
    }
    let profile = profiles
        .iter_mut()
        .find(|p| p.name == name)
        .ok_or_else(|| anyhow!("there is no profile {name}"))?;
    profile.name = to.to_owned();
    store(profiles)
}

pub fn remove(name: &str) -> Result<()> {
    let mut profiles = load()?;
    let before = profiles.len();
    profiles.retain(|p| p.name != name);
    if profiles.len() == before {
        bail!("there is no profile {name}");
    }
    store(profiles)
}
