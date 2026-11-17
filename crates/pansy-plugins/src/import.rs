//! Bringing magpie's plugins over: its `plugins.json` (the same shape as
//! pansy's) installed again here, and its plugins' sign-ins copied from its
//! `plugin-auth.json` into pansy's. magpie's files are only read.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use pansy_core::paths;
use serde::Serialize;
use serde_json::Value;

use crate::store::{self, List};

/// magpie's folder: `$XDG_CONFIG_HOME/magpie`, else `~/.config/magpie`.
pub fn magpie_dir() -> PathBuf {
    match std::env::var_os("XDG_CONFIG_HOME").filter(|x| !x.is_empty()) {
        Some(x) => PathBuf::from(x).join("magpie"),
        None => paths::user_home().join(".config").join("magpie"),
    }
}

/// What magpie has to bring over.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    pub dir: String,
    pub plugins: Vec<String>,
    pub accounts: usize,
}

fn magpie_auth() -> serde_json::Map<String, Value> {
    std::fs::read(magpie_dir().join("plugin-auth.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

/// magpie's plugins and sign-ins, when it has any.
pub fn found() -> Option<Found> {
    let dir = magpie_dir();
    let list: List = pansy_core::fsutil::read_json(&dir.join("plugins.json")).ok()?;
    if list.plugins.is_empty() {
        return None;
    }
    Some(Found {
        dir: dir.display().to_string(),
        plugins: list.plugins.iter().map(|e| e.spec.clone()).collect(),
        accounts: magpie_auth().len(),
    })
}

/// What bringing them over did.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Imported {
    pub added: Vec<String>,
    /// Already here.
    pub kept: Vec<String>,
    pub failed: Vec<(String, String)>,
    pub accounts: usize,
}

/// Installs magpie's plugins here (each one pansy hasn't), in its order,
/// on or off and with its options as there, and copies the sign-ins pansy
/// doesn't have.
pub async fn from_magpie() -> Result<Imported> {
    let dir = magpie_dir();
    let theirs: List = pansy_core::fsutil::read_json(&dir.join("plugins.json"))
        .with_context(|| format!("reading magpie's plugins in {}", dir.display()))?;
    if theirs.plugins.is_empty() {
        bail!("magpie has no plugins in {}", dir.display());
    }
    let mut out = Imported::default();
    let mine = List::load()?;
    for e in &theirs.plugins {
        if mine.find(&e.package()).is_some() {
            out.kept.push(e.spec.clone());
            continue;
        }
        match store::add(&e.spec).await {
            Ok(_) => {
                let (off, options) = (e.off, e.options.clone());
                let package = e.package();
                store::update(|list| {
                    if let Some(at) = list.find(&package) {
                        list.plugins[at].off = off;
                        list.plugins[at].options = options;
                    }
                    Ok(())
                })?;
                out.added.push(e.spec.clone());
            }
            Err(err) => out.failed.push((e.spec.clone(), format!("{err:#}"))),
        }
    }
    store::update(|list| {
        for (k, v) in &theirs.prefer {
            list.prefer.entry(k.clone()).or_insert_with(|| v.clone());
        }
        for (k, v) in &theirs.config {
            list.config.entry(k.clone()).or_insert_with(|| v.clone());
        }
        Ok(())
    })?;
    let accounts = magpie_auth();
    if !accounts.is_empty() {
        let path = paths::plugin_auth();
        let mut auth = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        for (k, v) in accounts {
            if !auth.contains_key(&k) {
                auth.insert(k, v);
                out.accounts += 1;
            }
        }
        let bytes = serde_json::to_vec_pretty(&Value::Object(auth))?;
        pansy_core::fsutil::write_atomic(&path, &bytes, true)?;
    }
    if let Err(e) = crate::host::list_now().await {
        tracing::warn!("listing the plugins' providers: {e:#}");
    }
    Ok(out)
}
