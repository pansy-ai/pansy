//! The project a request is for. `pansy run` starts an agent with a key
//! that names the folder it runs in (`pansy-p-…`); the gateway, which takes
//! any key from this computer, reads the folder back and counts the request
//! under it. The key is no secret and opens nothing.

use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// What a project's key begins with.
pub const KEY_PREFIX: &str = "pansy-p-";

/// The longest folder a key may name.
const MAX: usize = 1024;

/// The key an agent run in `folder` sends.
pub fn key_for(folder: &Path) -> String {
    format!(
        "{KEY_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(folder.to_string_lossy().as_bytes())
    )
}

/// The folder a key names, when it is a project's: text without control
/// characters, of a sane length.
pub fn of_key(key: &str) -> Option<String> {
    let folder = URL_SAFE_NO_PAD
        .decode(key.trim().strip_prefix(KEY_PREFIX)?)
        .ok()?;
    let folder = String::from_utf8(folder).ok()?;
    let sane = !folder.is_empty() && folder.len() <= MAX && !folder.chars().any(char::is_control);
    sane.then_some(folder)
}

/// A folder's last name, to show it short (`/home/me/code/shop` → `shop`).
pub fn short(folder: &str) -> &str {
    let trimmed = folder.trim_end_matches(['/', '\\']);
    trimmed
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(folder)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_names_its_folder() {
        let key = key_for(Path::new("/home/me/代码/shop"));
        assert!(key.starts_with("pansy-p-"));
        assert!(key.is_ascii());
        assert_eq!(of_key(&key).as_deref(), Some("/home/me/代码/shop"));
        assert_eq!(short("/home/me/代码/shop/"), "shop");
        assert_eq!(short("C:\\code\\shop"), "shop");
    }

    #[test]
    fn other_keys_name_none() {
        assert_eq!(of_key("pansy"), None);
        assert_eq!(of_key("sk-ant-123"), None);
        assert_eq!(of_key("pansy-p-"), None);
        assert_eq!(of_key("pansy-p-!!!"), None);
        let control = format!("{KEY_PREFIX}{}", URL_SAFE_NO_PAD.encode("a\nb"));
        assert_eq!(of_key(&control), None);
        let long = key_for(Path::new(&"x".repeat(MAX + 1)));
        assert_eq!(of_key(&long), None);
    }
}
