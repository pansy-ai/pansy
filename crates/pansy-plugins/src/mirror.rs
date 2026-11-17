//! magpie's 「国内镜像」 switch (`chinaMirror` in Settings): Bun and plugins'
//! packages asked of a mirror in China first, the official address after
//! when the mirror can't answer. Only mirrors that serve the very files:
//! npmmirror for npm's registry and Bun's releases (whose zips are checked
//! against Bun's checksums all the same). Never a proxy that could hand
//! back anything. The plugin list needs none: pansy's hub is reached from
//! China as it is (`market.rs`).

/// npm's registry in China.
pub const NPM_MIRROR: &str = "https://registry.npmmirror.com";
const BUN_MIRROR: &str = "https://registry.npmmirror.com/-/binary/bun/";

/// Whether the switch is on.
pub fn china() -> bool {
    pansy_core::config::load_settings()
        .map(|s| s.china_mirror)
        .unwrap_or(false)
}

/// The mirror in China for `raw`, if one serves it.
pub fn china_first(raw: &str) -> Option<String> {
    let rest = raw.strip_prefix("https://")?;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    match host.to_ascii_lowercase().as_str() {
        "registry.npmjs.org" => Some(format!("{NPM_MIRROR}/{path}")),
        "github.com" => path
            .strip_prefix("oven-sh/bun/releases/download/")
            .filter(|p| !p.is_empty())
            .map(|p| format!("{BUN_MIRROR}{p}")),
        _ => None,
    }
}

/// Where to ask for `raw`, in order: its mirror first with the switch on,
/// then `raw`.
pub fn urls(raw: &str) -> Vec<String> {
    urls_with(raw, china())
}

fn urls_with(raw: &str, on: bool) -> Vec<String> {
    let mirror = if on { china_first(raw) } else { None };
    mirror.into_iter().chain([raw.to_owned()]).collect()
}

/// `raw`'s body, asked of its addresses in turn (`urls`): a mirror's that
/// fails, or that `keep` turns down (a search it has no answer to), is
/// asked of the official address.
pub async fn get(
    client: &reqwest::Client,
    raw: &str,
    keep: impl Fn(&[u8]) -> bool,
) -> anyhow::Result<Vec<u8>> {
    let urls = urls(raw);
    let mut last = None;
    for (i, url) in urls.iter().enumerate() {
        let official = i + 1 == urls.len();
        let body = async {
            let response = client.get(url).send().await?.error_for_status()?;
            anyhow::Ok(response.bytes().await?.to_vec())
        };
        match body.await {
            Ok(body) if official || keep(&body) => return Ok(body),
            Ok(_) => tracing::debug!("{url} had no answer; asking {raw}"),
            Err(e) => {
                if !official {
                    tracing::debug!("{url}: {e}; asking {raw}");
                }
                last = Some(e);
            }
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("{raw} couldn't be asked")))
}

/// The registry `bun install` is told to use: npmmirror with the switch
/// on, unless the user named one of their own (bun's variables, a
/// `.bunfig.toml`, an `.npmrc`), which is never put behind it.
pub fn bun_registry() -> Option<&'static str> {
    (china() && !own_registry()).then_some(NPM_MIRROR)
}

fn own_registry() -> bool {
    let named = |v: Option<String>| v.is_some_and(|v| !v.trim().is_empty());
    if [
        "BUN_CONFIG_REGISTRY",
        "NPM_CONFIG_REGISTRY",
        "npm_config_registry",
    ]
    .iter()
    .any(|k| named(std::env::var(k).ok()))
    {
        return true;
    }
    let home = pansy_core::paths::user_home();
    let plugins = pansy_core::paths::plugins_dir();
    let files = [
        (plugins.join("bunfig.toml"), true),
        (plugins.join(".npmrc"), false),
        (home.join(".bunfig.toml"), true),
        (home.join(".npmrc"), false),
    ];
    files.iter().any(|(path, bunfig)| {
        std::fs::read_to_string(path).is_ok_and(|text| {
            text.lines().any(|l| {
                let l = l.trim();
                if *bunfig {
                    l.starts_with("registry") && l.contains('=')
                } else {
                    l.strip_prefix("registry")
                        .is_some_and(|r| r.trim_start().starts_with('='))
                }
            })
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirrors_only_what_a_mirror_serves() {
        assert_eq!(
            china_first("https://registry.npmjs.org/-/v1/search?text=x").as_deref(),
            Some("https://registry.npmmirror.com/-/v1/search?text=x")
        );
        assert_eq!(
            china_first(
                "https://github.com/oven-sh/bun/releases/download/bun-v1.2.3/bun-linux-x64.zip"
            )
            .as_deref(),
            Some("https://registry.npmmirror.com/-/binary/bun/bun-v1.2.3/bun-linux-x64.zip")
        );
        assert_eq!(
            china_first("https://github.com/someone/else/releases/download/x"),
            None
        );
        assert_eq!(
            china_first(
                "https://raw.githubusercontent.com/magpie-community/plugins/main/registry.json"
            ),
            None
        );
        assert_eq!(china_first("http://registry.npmjs.org/x"), None);
        assert_eq!(china_first("https://models.dev/api.json"), None);
    }

    #[test]
    fn asks_the_mirror_first_only_when_on() {
        let raw = "https://registry.npmjs.org/x";
        assert_eq!(urls_with(raw, false), vec![raw.to_owned()]);
        assert_eq!(
            urls_with(raw, true),
            vec![
                "https://registry.npmmirror.com/x".to_owned(),
                raw.to_owned()
            ]
        );
        let other = "https://example.com/x";
        assert_eq!(urls_with(other, true), vec![other.to_owned()]);
    }
}
