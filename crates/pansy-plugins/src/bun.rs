//! The Bun the plugins run on: `PANSY_BUN` when it is set, else the one
//! pansy downloaded, else downloaded now from Bun's releases on GitHub and
//! checked against the release's `SHASUMS256.txt`. One version, the one
//! magpie runs its plugins on, so a community plugin behaves as it does
//! there.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use sha2::{Digest, Sha256};

/// The Bun pansy downloads.
pub const VERSION: &str = "1.4.2";

/// This computer's build of Bun, as its releases name it; `None` where
/// there is none.
pub fn asset() -> Option<&'static str> {
    let musl = cfg!(target_os = "linux")
        && (Path::new("/lib/ld-musl-x86_64.so.1").exists()
            || Path::new("/lib/ld-musl-aarch64.so.1").exists());
    #[cfg(target_arch = "x86_64")]
    let modern = std::arch::is_x86_feature_detected!("avx2");
    #[cfg(not(target_arch = "x86_64"))]
    let modern = true;
    Some(
        match (std::env::consts::OS, std::env::consts::ARCH, musl, modern) {
            ("linux", "x86_64", false, true) => "bun-linux-x64",
            ("linux", "x86_64", false, false) => "bun-linux-x64-baseline",
            ("linux", "x86_64", true, true) => "bun-linux-x64-musl",
            ("linux", "x86_64", true, false) => "bun-linux-x64-musl-baseline",
            ("linux", "aarch64", false, _) => "bun-linux-aarch64",
            ("linux", "aarch64", true, _) => "bun-linux-aarch64-musl",
            ("macos", "aarch64", _, _) => "bun-darwin-aarch64",
            ("macos", "x86_64", _, true) => "bun-darwin-x64",
            ("macos", "x86_64", _, false) => "bun-darwin-x64-baseline",
            ("windows", "x86_64", _, true) => "bun-windows-x64",
            ("windows", "x86_64", _, false) => "bun-windows-x64-baseline",
            _ => return None,
        },
    )
}

fn exe() -> &'static str {
    if cfg!(windows) { "bun.exe" } else { "bun" }
}

/// Where the downloaded Bun is.
fn downloaded() -> Option<PathBuf> {
    let path = pansy_core::paths::bun()
        .join(VERSION)
        .join(asset()?)
        .join(exe());
    path.is_file().then_some(path)
}

/// The Bun to run plugins with, if one is at hand without a download.
pub fn find() -> Option<PathBuf> {
    if let Some(own) = std::env::var_os("PANSY_BUN").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(own));
    }
    downloaded()
}

/// The Bun to run plugins with, downloaded first when there is none.
pub async fn ensure() -> Result<PathBuf> {
    if let Some(bun) = find() {
        return Ok(bun);
    }
    crate::on_rt(download()).await
}

async fn download() -> Result<PathBuf> {
    if let Some(bun) = downloaded() {
        return Ok(bun);
    }
    let asset = asset().ok_or_else(|| {
        anyhow!(
            "Bun has no build for {} {}: set PANSY_BUN to a Bun of your own",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;
    let base = format!("https://github.com/oven-sh/bun/releases/download/bun-v{VERSION}");
    let settings = pansy_core::config::load_settings().unwrap_or_default();
    let builder = reqwest::Client::builder()
        .user_agent(format!("pansy/{}", pansy_core::VERSION))
        .connect_timeout(std::time::Duration::from_secs(20));
    let http = pansy_core::settings::use_proxy(builder, &settings.proxy)
        .map_err(|e| anyhow!(e))?
        .build()?;
    tracing::info!("downloading Bun {VERSION} ({asset}) to run plugins");
    // npmmirror's copy first with the 「国内镜像」 switch on (crate::mirror)
    let sums = crate::mirror::get(&http, &format!("{base}/SHASUMS256.txt"), |_| true)
        .await
        .context("Bun's checksums")?;
    let sums = String::from_utf8_lossy(&sums);
    let want = sums
        .lines()
        .find_map(|l| {
            let (sum, name) = l.split_once(char::is_whitespace)?;
            (name.trim() == format!("{asset}.zip")).then(|| sum.trim().to_ascii_lowercase())
        })
        .ok_or_else(|| anyhow!("Bun {VERSION}'s checksums don't list {asset}.zip"))?;
    let zip = crate::mirror::get(&http, &format!("{base}/{asset}.zip"), |_| true)
        .await
        .with_context(|| format!("downloading {asset}.zip"))?;
    let got: String = Sha256::digest(&zip)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if got != want {
        bail!("{asset}.zip isn't what Bun released (its checksum is {got}, not {want})");
    }
    crate::private_dir(&pansy_core::paths::bun())?;
    let root = pansy_core::paths::bun().join(VERSION);
    tokio::task::spawn_blocking(move || unpack(&zip, &root)).await??;
    downloaded().ok_or_else(|| anyhow!("{asset}.zip had no {}", exe()))
}

/// Unpacks Bun's zip into `root`, through a folder beside it renamed into
/// place, so a download cut short leaves nothing half made.
fn unpack(zip: &[u8], root: &Path) -> Result<()> {
    std::fs::create_dir_all(root)?;
    let tmp = tempfile_dir(root)?;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip))?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let Some(name) = file.enclosed_name() else {
            continue;
        };
        let out = tmp.join(name);
        if file.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut w = std::fs::File::create(&out)?;
        std::io::copy(&mut file, &mut w)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    for entry in std::fs::read_dir(&tmp)? {
        let entry = entry?;
        let to = root.join(entry.file_name());
        if to.exists() {
            let _ = std::fs::remove_dir_all(&to);
        }
        std::fs::rename(entry.path(), to)?;
    }
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(())
}

fn tempfile_dir(root: &Path) -> Result<PathBuf> {
    let dir = root.join(format!(".unpack-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_this_computers_build() {
        if let Some(asset) = asset() {
            assert!(asset.starts_with("bun-"));
        }
    }

    #[test]
    fn unpacks_a_zip_whole() {
        let dir = tempfile::tempdir().unwrap();
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let o: zip::write::SimpleFileOptions = Default::default();
            w.start_file("bun-test/bun", o).unwrap();
            std::io::Write::write_all(&mut w, b"#!/bin/sh\n").unwrap();
            w.finish().unwrap();
        }
        unpack(&buf, dir.path()).unwrap();
        assert!(dir.path().join("bun-test/bun").is_file());
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            1,
            "no unpacking folder left"
        );
    }
}
