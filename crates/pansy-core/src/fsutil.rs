//! Files written whole or not at all, readable only by the user when they
//! hold secrets.

use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Writes `bytes` to `path` through a temporary file beside it and a rename,
/// so a reader never sees half a file. `private`: mode 0600 on Unix; else
/// the mode of the file replaced.
///
/// A symlink (a config kept in a dotfiles folder) stays one: the file it
/// points at is the one written.
pub fn write_atomic(path: &Path, bytes: &[u8], private: bool) -> Result<()> {
    let target;
    let path = match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            target = fs::canonicalize(path).or_else(|_| {
                // A link to a file not made yet.
                fs::read_link(path).map(|to| match path.parent() {
                    Some(dir) if to.is_relative() => dir.join(to),
                    _ => to,
                })
            })?;
            target.as_path()
        }
        _ => path,
    };
    let dir = path.parent().context("a file needs a directory")?;
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let mut tmp = tempfile::Builder::new()
        .prefix(".pansy-")
        .tempfile_in(dir)
        .with_context(|| format!("writing in {}", dir.display()))?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if private {
            0o600
        } else {
            // Keep the mode of the file being replaced (an agent's config).
            fs::metadata(path)
                .map(|m| m.permissions().mode() & 0o777)
                .unwrap_or(0o644)
        };
        fs::set_permissions(tmp.path(), fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    let _ = private;
    tmp.persist(path)
        .with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

/// A JSON file's value, or the default when there is none yet.
pub fn read_json<T: DeserializeOwned + Default>(path: &Path) -> Result<T> {
    match fs::read(path) {
        Ok(bytes) if bytes.iter().all(u8::is_ascii_whitespace) => Ok(T::default()),
        Ok(bytes) => {
            serde_json::from_slice(&bytes).with_context(|| format!("reading {}", path.display()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// Writes `value` as indented JSON.
pub fn write_json<T: Serialize>(path: &Path, value: &T, private: bool) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    write_atomic(path, &bytes, private)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn writes_through_a_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("dotfiles/settings.json");
        fs::create_dir_all(real.parent().unwrap()).unwrap();
        fs::write(&real, "{}").unwrap();
        let link = dir.path().join("settings.json");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        write_atomic(&link, b"{\"a\": 1}", false).unwrap();
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_to_string(&real).unwrap(), "{\"a\": 1}");
    }

    #[test]
    fn writes_privately_and_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a/b.json");
        write_json(&path, &vec![1, 2], true).unwrap();
        let back: Vec<i32> = read_json(&path).unwrap();
        assert_eq!(back, [1, 2]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let missing: Vec<i32> = read_json(&dir.path().join("none.json")).unwrap();
        assert!(missing.is_empty());
    }
}
