//! The community plugins as magpie's market shows them: one list of every
//! plugin, whoever wrote it, from pansy's registry at hub.pansyai.dev
//! (pansy-ai/pansy-registry), which reads npm and GitHub for it. Its
//! curated list (begun from magpie-community's; fetched at most once a day,
//! with a copy shipped for when the hub can't be reached:
//! `data/registry.json`, the registry of magpie-community/plugins, MIT),
//! and OpenCode's auth plugins and pi's provider packages that others
//! published on npm, found by their keywords; the hub says how often each
//! is downloaded, its version and who published it. [`search`] asks it for
//! the rest by a word, as magpie's search asks npm.

use std::time::{Duration, SystemTime};

use pansy_core::paths;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const HUB: &str = "https://hub.pansyai.dev/api/v1";
const SHIPPED: &str = include_str!("../data/registry.json");

/// A community plugin.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Listed {
    pub package: String,
    pub name: String,
    /// `magpie` (magpie-community's registry), `opencode` or `pi` (another
    /// author's, on npm).
    pub source: String,
    /// Written for magpie by its community.
    #[serde(skip_deserializing)]
    pub community: bool,
    /// Who published it on npm.
    pub publisher: String,
    /// Its newest version on npm.
    pub version: String,
    /// Its downloads last week, as npm counts them; 0 when not known.
    pub weekly: u64,
    /// `provider` (the default), `middleware`, `agent`, `tools` or `page`.
    pub kind: String,
    /// The market's shelf it is on: `opencode` (a provider plugin),
    /// `pi`, `middleware`, `agent`, `tools` or `page`.
    #[serde(skip_deserializing)]
    pub shelf: String,
    /// What it does, by language (`en`, `zh`).
    pub summary: std::collections::BTreeMap<String, String>,
    /// The providers it signs in to.
    pub providers: Vec<String>,
    pub icon: String,
    /// pansy runs its kind (`tools` and `page` it doesn't yet).
    #[serde(skip_deserializing)]
    pub runs: bool,
}

impl Listed {
    fn place(&mut self) {
        if self.kind.is_empty() {
            self.kind = "provider".into();
        }
        self.runs = matches!(self.kind.as_str(), "provider" | "middleware" | "agent");
        self.community = self.source == "magpie";
        self.shelf = match (self.source.as_str(), self.kind.as_str()) {
            ("pi", _) => "pi".into(),
            (_, "provider") => "opencode".into(),
            (_, kind) => kind.into(),
        };
    }
}

fn parse(text: &str) -> Option<Vec<Listed>> {
    let v: Value = serde_json::from_str(text).ok()?;
    let mut out: Vec<Listed> = serde_json::from_value(v.get("plugins")?.clone()).ok()?;
    for l in &mut out {
        l.source = "magpie".into();
        l.place();
    }
    Some(out)
}

fn cached(max_age: Option<Duration>) -> Option<Vec<Listed>> {
    parse(&fresh_enough(&paths::plugin_registry(), max_age)?)
}

/// A cached file's text, when it is younger than `max_age` (any age: `None`).
fn fresh_enough(path: &std::path::Path, max_age: Option<Duration>) -> Option<String> {
    if let Some(max) = max_age {
        let at = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
        if SystemTime::now().duration_since(at).unwrap_or_default() > max {
            return None;
        }
    }
    std::fs::read_to_string(path).ok()
}

const DAY: Duration = Duration::from_secs(24 * 3600);

/// The hub's answer at `path` (under `/api/v1`) with `query`. The hub is
/// on Cloudflare under pansy's own domain, reached from China as it is, so
/// no mirror is asked.
async fn hub(
    client: &reqwest::Client,
    path: &str,
    query: &[(&str, &str)],
) -> anyhow::Result<Value> {
    let url = reqwest::Url::parse_with_params(&format!("{HUB}/{path}"), query)?;
    let response = client.get(url).send().await?.error_for_status()?;
    Ok(response.json().await?)
}

fn client() -> Option<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .ok()
}

/// The hub's curated list, each with its downloads, version and publisher:
/// the cache when it is less than a day old (or `fresh` is false and the
/// hub can't be reached), else fetched again; the shipped list when there
/// is neither.
pub async fn list(fresh: bool) -> Vec<Listed> {
    if !fresh && let Some(list) = cached(Some(DAY)) {
        return list;
    }
    let fetched = async {
        let body = hub(&client()?, "registry.json", &[("curated", "1")])
            .await
            .ok()?;
        let text = body.to_string();
        let list = parse(&text)?;
        let _ = pansy_core::fsutil::write_atomic(&paths::plugin_registry(), text.as_bytes(), false);
        Some(list)
    };
    match fetched.await {
        Some(list) => list,
        None => cached(None).unwrap_or_else(shipped),
    }
}

/// The list pansy ships.
pub fn shipped() -> Vec<Listed> {
    parse(SHIPPED).unwrap_or_default()
}

// --- the rest, on npm -----------------------------------------------------------

/// The hub's lists the rest of the market is made of, each the most
/// downloaded first: OpenCode's plugins (`opencode-plugin`) and pi's
/// packages (`pi-package`) that say they sign in or serve a provider.
const SEARCHES: [(&str, &str, &str); 4] = [
    ("opencode", "opencode", "auth"),
    ("opencode", "opencode", "provider"),
    ("pi", "pi", "provider"),
    ("pi", "pi", "auth"),
];

/// The most of each list asked for.
const LIMIT: &str = "2000";

/// A package the hub found on npm: its name, keywords (lowercased),
/// description, version, publisher and downloads last week.
#[derive(Debug, Clone, Default)]
struct Hit {
    package: String,
    keywords: Vec<String>,
    description: String,
    version: String,
    publisher: String,
    weekly: u64,
}

fn hits(page: &Value) -> Vec<Hit> {
    let mut out = Vec::new();
    for p in page["plugins"].as_array().into_iter().flatten() {
        let Some(package) = p["package"].as_str() else {
            continue;
        };
        out.push(Hit {
            package: package.to_owned(),
            keywords: p["keywords"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|k| k.as_str())
                .map(str::to_ascii_lowercase)
                .collect(),
            description: p["description"].as_str().unwrap_or("").trim().to_owned(),
            version: p["version"].as_str().unwrap_or("").to_owned(),
            publisher: p["publisher"].as_str().unwrap_or("").to_owned(),
            weekly: p["weekly"].as_u64().unwrap_or(0),
        });
    }
    out
}

impl Hit {
    fn has(&self, k: &str) -> bool {
        self.keywords.iter().any(|x| x == k)
    }

    fn text(&self) -> String {
        format!(
            "{} {} {}",
            self.package,
            self.description,
            self.keywords.join(" ")
        )
        .to_ascii_lowercase()
    }

    /// An OpenCode plugin that signs in to a provider or serves one (V2's,
    /// a different API, left out).
    fn opencode(&self) -> bool {
        let text = self.text();
        self.has("opencode-plugin")
            && (text.contains("auth") || text.contains("provider"))
            && !self.has("opencode-v2")
            && !text.contains("opencode v2")
            && !text.contains("v2 plugin")
    }

    /// A pi package that adds a provider; one that only adds pi a command
    /// or a tool has nothing for pansy.
    fn pi(&self) -> bool {
        let about = format!("{} {}", self.package, self.description).to_ascii_lowercase();
        (self.has("pi-package") || self.has("pi-provider"))
            && (self.has("pi-provider")
                || self.package.contains("provider")
                || [
                    "provider extension",
                    "provider package",
                    "provider for pi",
                    "model provider",
                    "custom provider",
                    "as a provider",
                    "auth",
                ]
                .iter()
                .any(|w| about.contains(w)))
    }

    fn listed(&self, source: &str) -> Listed {
        // a badge's markdown is no summary
        let summary = if self.description.starts_with(['[', '!']) {
            String::new()
        } else {
            self.description.clone()
        };
        let mut l = Listed {
            package: self.package.clone(),
            name: self
                .package
                .rsplit('/')
                .next()
                .unwrap_or(&self.package)
                .to_owned(),
            source: source.into(),
            publisher: self.publisher.clone(),
            version: self.version.clone(),
            weekly: self.weekly,
            summary: [("en".to_owned(), summary)].into_iter().collect(),
            ..Listed::default()
        };
        l.place();
        l
    }
}

/// The market: the curated list, then the OpenCode plugins and pi
/// packages others published.
fn merge(registry: Vec<Listed>, pages: &[(String, Value)]) -> Vec<Listed> {
    let found: Vec<(&str, Hit)> = pages
        .iter()
        .flat_map(|(source, page)| hits(page).into_iter().map(move |h| (source.as_str(), h)))
        .collect();
    let mut out = registry;
    for (source, h) in &found {
        let fits = match *source {
            "opencode" => h.opencode(),
            "pi" => h.pi(),
            _ => false,
        };
        if fits && !out.iter().any(|l| l.package == h.package) {
            out.push(h.listed(source));
        }
    }
    out
}

/// Where the hub's lists are kept, a day.
fn pages_cache() -> std::path::PathBuf {
    paths::plugin_registry().with_file_name("plugin-hub.json")
}

async fn pages(fresh: bool) -> Vec<(String, Value)> {
    let read = |max| -> Option<Vec<(String, Value)>> {
        serde_json::from_str(&fresh_enough(&pages_cache(), max)?).ok()
    };
    if !fresh && let Some(pages) = read(Some(DAY)) {
        return pages;
    }
    let fetched = async {
        let client = client()?;
        let asks = SEARCHES.iter().map(|(source, ecosystem, q)| {
            let client = client.clone();
            async move {
                let query = [
                    ("curated", "0"),
                    ("ecosystem", ecosystem),
                    ("q", q),
                    ("limit", LIMIT),
                ];
                let page = hub(&client, "registry.json", &query).await.ok()?;
                Some((source.to_string(), page))
            }
        });
        let pages: Vec<(String, Value)> = futures::future::join_all(asks)
            .await
            .into_iter()
            .flatten()
            .collect();
        if pages.is_empty() {
            return None;
        }
        let _ = pansy_core::fsutil::write_atomic(
            &pages_cache(),
            serde_json::to_string(&pages).ok()?.as_bytes(),
            false,
        );
        Some(pages)
    };
    match fetched.await {
        Some(pages) => pages,
        None => read(None).unwrap_or_default(),
    }
}

/// Every community plugin: the curated and the OpenCode plugins and pi
/// packages on npm, each with its downloads last week where npm has them.
/// Read again once a day (`fresh`: now); what was read last, or the
/// shipped list, when the hub can't be reached.
pub async fn all(fresh: bool) -> Vec<Listed> {
    let (registry, pages) = futures::join!(list(fresh), pages(fresh));
    merge(registry, &pages)
}

/// The hub's plugins for `q`, as magpie's search asks npm: OpenCode
/// plugins (its name or keywords say so, and that it signs in, serves a
/// provider or is a plugin) and pi packages that sign in or bring a
/// provider.
pub async fn search(q: &str) -> anyhow::Result<Vec<Listed>> {
    let q = q.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let client = client().ok_or_else(|| anyhow::anyhow!("no HTTP client"))?;
    let ask = |ecosystem: &'static str| {
        let client = client.clone();
        async move {
            let query = [("q", q), ("ecosystem", ecosystem), ("limit", "30")];
            anyhow::Ok(hits(&hub(&client, "plugins", &query).await?))
        }
    };
    let (oc, pi) = futures::join!(ask("opencode"), ask("pi"));
    if let (Err(e), Err(_)) = (&oc, &pi) {
        anyhow::bail!("the plugin hub couldn't be asked: {e}");
    }
    let mut out: Vec<Listed> = Vec::new();
    for h in oc.unwrap_or_default() {
        let text = format!("{} {}", h.package, h.keywords.join(" ")).to_ascii_lowercase();
        if text.contains("opencode")
            && ["auth", "plugin", "provider"]
                .iter()
                .any(|w| text.contains(w))
            && !out.iter().any(|l| l.package == h.package)
        {
            out.push(h.listed("opencode"));
        }
    }
    for h in pi.unwrap_or_default() {
        let text = format!("{} {}", h.package, h.keywords.join(" ")).to_ascii_lowercase();
        if h.has("pi-package")
            && (text.contains("auth") || text.contains("provider"))
            && !out.iter().any(|l| l.package == h.package)
        {
            out.push(h.listed("pi"));
        }
    }
    Ok(out)
}

/// The package a word people use for a plugin stands for: a community
/// plugin's name or short name (`workbuddy`, `think-tags`); anything else
/// (an npm spec, a repository, a path) as it is.
pub async fn resolve(word: &str) -> String {
    if word.contains(['/', '@', '\\', ':']) || word.starts_with('.') || word.starts_with('~') {
        return word.to_owned();
    }
    let lower = word.to_ascii_lowercase();
    let known = cached(None).unwrap_or_else(shipped);
    known
        .iter()
        .find(|l| {
            l.name.to_ascii_lowercase() == lower
                || crate::store::short_name(&l.package) == lower
                || l.package.to_ascii_lowercase() == lower
        })
        .map(|l| l.package.clone())
        .unwrap_or_else(|| word.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ships_a_list() {
        let list = shipped();
        assert!(list.len() > 100);
        let wb = list.iter().find(|l| l.name == "WorkBuddy").unwrap();
        assert_eq!(wb.kind, "provider");
        assert_eq!((wb.shelf.as_str(), wb.community), ("opencode", true));
        assert!(wb.runs);
        assert!(list.iter().any(|l| l.kind == "page" && !l.runs));
    }

    #[test]
    fn makes_one_market_of_the_registry_and_npm() {
        let page = serde_json::json!({"version": 1, "plugins": [
            {"package": "opencode-poe-auth", "description": "Poe OAuth authentication plugin for OpenCode",
                "keywords": ["opencode", "opencode-plugin", "auth"], "weekly": 50,
                "version": "0.3.0", "publisher": "poe"},
            {"package": "@joe/opencode-mem", "description": "Memory for OpenCode agents",
                "keywords": ["opencode", "opencode-plugin", "memory"], "weekly": 900},
            {"package": "@t/opencode-provider", "description": "OpenCode V2 plugin for a provider.",
                "keywords": ["opencode-plugin"]},
            {"package": "pi-fireworks", "description": "Fireworks AI provider extension package for pi",
                "keywords": ["pi-package"], "weekly": 30},
            {"package": "pi-subagents", "description": "Subagents for pi",
                "keywords": ["pi-package", "pi"], "weekly": 5000},
            {"package": "@magpie-community/opencode-workbuddy-auth",
                "keywords": ["opencode-plugin", "auth"], "weekly": 300},
        ]});
        let pages = vec![
            ("opencode".to_owned(), page.clone()),
            ("pi".to_owned(), page),
        ];
        let all = merge(shipped(), &pages);
        assert_eq!(all.iter().filter(|l| l.name == "WorkBuddy").count(), 1);
        let from_npm: Vec<(&str, &str, &str, u64)> = all
            .iter()
            .filter(|l| !l.community)
            .map(|l| {
                (
                    l.package.as_str(),
                    l.shelf.as_str(),
                    l.publisher.as_str(),
                    l.weekly,
                )
            })
            .collect();
        assert_eq!(
            from_npm,
            [
                ("opencode-poe-auth", "opencode", "poe", 50),
                ("pi-fireworks", "pi", "", 30)
            ]
        );
    }

    #[test]
    fn reads_the_hubs_curated_list() {
        let text = serde_json::json!({"version": 1, "plugins": [
            {"package": "@magpie-community/opencode-workbuddy-auth", "name": "WorkBuddy",
                "curated": true, "kind": "provider", "summary": {"en": "WorkBuddy"},
                "providers": ["workbuddy"], "version": "0.2.36", "publisher": "yetone",
                "weekly": 294000, "stars": 283, "ecosystems": ["opencode"]},
        ]})
        .to_string();
        let list = parse(&text).unwrap();
        let wb = &list[0];
        assert_eq!(
            (wb.name.as_str(), wb.shelf.as_str(), wb.community, wb.weekly),
            ("WorkBuddy", "opencode", true, 294000)
        );
        assert_eq!(
            (wb.version.as_str(), wb.publisher.as_str()),
            ("0.2.36", "yetone")
        );
    }

    #[test]
    fn resolves_a_community_name() {
        let home = tempfile::tempdir().unwrap();
        let _env = crate::testing::scratch(home.path());
        let r = crate::block(resolve("workbuddy"));
        assert_eq!(r, "@magpie-community/opencode-workbuddy-auth");
        assert_eq!(
            crate::block(resolve("Word Guard")),
            "@magpie-community/middleware-word-guard"
        );
        assert_eq!(
            crate::block(resolve("word-guard")),
            "@magpie-community/middleware-word-guard"
        );
        assert_eq!(crate::block(resolve("./mine")), "./mine");
    }
}
