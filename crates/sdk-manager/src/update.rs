//! MapCraft's own updates: the newest GitHub release, and swapping it in for the running copy.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use serde::Deserialize;

use crate::registry::MAX_DOWNLOAD;
use crate::{Error, MAPCRAFT_VERSION, io_err, sha256_hex};

pub const LATEST_RELEASE: &str = "https://api.github.com/repos/bearlikelion/MapCraft/releases/latest";
const MAX_INFO: u64 = 1 << 20;

/// This platform's release asset and, for a zip, the path inside it that replaces the running copy.
const ASSET: Option<(&str, Option<&str>)> = if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
    Some(("MapCraft-x86_64.AppImage", None))
} else if cfg!(windows) {
    Some(("MapCraft-windows-x86_64.zip", Some("MapCraft/MapCraft.exe")))
} else if cfg!(target_os = "macos") {
    Some(("MapCraft-macos.zip", Some("MapCraft.app")))
} else {
    None
};

#[derive(Clone, Debug, Deserialize)]
pub struct Release {
    pub tag_name: String,
    pub html_url: String,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
    /// `sha256:<hex>`, which GitHub computes for each upload.
    #[serde(default)]
    pub digest: Option<String>,
}

impl Release {
    pub fn version(&self) -> &str {
        self.tag_name.trim_start_matches('v')
    }

    pub fn is_newer(&self) -> bool {
        crate::newer(self.version(), MAPCRAFT_VERSION)
    }
}

/// Reads the release info from the GitHub API URL or a local JSON file.
pub fn latest_release(source: &str) -> Result<Release, Error> {
    let bytes = crate::fetch(source, MAX_INFO, &mut |_, _| {}, &AtomicBool::new(false))?;
    serde_json::from_slice(&bytes).map_err(|e| Error::Invalid(format!("{source}: {e}")))
}

/// What an update replaces: the AppImage, the macOS app bundle or the packaged Windows executable,
/// or `None` for builds that did not come from a release, such as `cargo run` or a distro package.
pub fn target() -> Option<PathBuf> {
    if cfg!(target_os = "linux") {
        return std::env::var_os("APPIMAGE").map(PathBuf::from);
    }
    let exe = std::env::current_exe().ok()?;
    if cfg!(target_os = "macos") {
        let app = exe.parent()?.parent()?.parent()?;
        return app.extension().is_some_and(|e| e == "app").then(|| app.to_path_buf());
    }
    (cfg!(windows) && exe.file_name().is_some_and(|n| n == "MapCraft.exe")).then_some(exe)
}

/// The program to start for `target`.
pub fn executable(target: &Path) -> PathBuf {
    if cfg!(target_os = "macos") { target.join("Contents/MacOS/mapcraft") } else { target.to_path_buf() }
}

fn sibling(target: &Path, suffix: &str) -> PathBuf {
    let mut name = target.as_os_str().to_owned();
    name.push(format!(".{suffix}"));
    PathBuf::from(name)
}

fn remove(path: &Path) {
    let _ = if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
}

/// Deletes the copy the last update moved aside, which Windows keeps while it runs.
pub fn clean(target: &Path) {
    remove(&sibling(target, "old"));
}

/// Downloads this platform's build of `release` and swaps it in for `target`; it runs from the next start.
pub fn install(release: &Release, target: &Path, progress: &mut dyn FnMut(u64, Option<u64>), cancel: &AtomicBool) -> Result<(), Error> {
    let (name, inner) = ASSET.ok_or_else(|| Error::Invalid("there is no MapCraft release for this platform".into()))?;
    let asset = release.assets.iter().find(|a| a.name == name).ok_or_else(|| Error::Invalid(format!("MapCraft {} has no {name}", release.version())))?;
    let bytes = crate::fetch(&asset.browser_download_url, MAX_DOWNLOAD, progress, cancel)?;
    if let Some(expected) = asset.digest.as_deref().and_then(|d| d.strip_prefix("sha256:")) {
        let actual = sha256_hex(&bytes);
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(Error::Checksum { expected: expected.to_string(), actual });
        }
    }
    let (new, old, stage) = (sibling(target, "new"), sibling(target, "old"), sibling(target, "update"));
    for p in [&new, &old, &stage] {
        remove(p);
    }
    match inner {
        Some(inner) => {
            crate::extract(&bytes, &stage)?;
            let from = stage.join(inner);
            std::fs::rename(&from, &new).map_err(io_err(&from))?;
            remove(&stage);
        }
        None => std::fs::write(&new, &bytes).map_err(io_err(&new))?,
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let exe = executable(&new);
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).map_err(io_err(&exe))?;
    }
    std::fs::rename(target, &old).map_err(io_err(target))?;
    if let Err(e) = std::fs::rename(&new, target) {
        let _ = std::fs::rename(&old, target);
        return Err(Error::Io { path: target.to_path_buf(), source: e });
    }
    clean(target);
    Ok(())
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
mod tests {
    use super::*;

    fn release(dir: &Path, digest: &str) -> Result<Release, Error> {
        let build = dir.join("build.AppImage");
        let _ = std::fs::write(&build, b"new build");
        let json = serde_json::json!({
            "tag_name": "v999.0.0",
            "html_url": "https://example.com",
            "assets": [{ "name": "MapCraft-x86_64.AppImage", "browser_download_url": build, "digest": digest }],
        });
        let info = dir.join("release.json");
        let _ = std::fs::write(&info, json.to_string());
        latest_release(&info.to_string_lossy())
    }

    #[test]
    fn install_swaps_the_appimage_and_checks_the_digest() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("MapCraft.AppImage");
        let _ = std::fs::write(&target, b"old build");
        let cancel = AtomicBool::new(false);

        let bad = release(dir.path(), "sha256:00").unwrap();
        assert!(bad.is_newer());
        assert!(matches!(install(&bad, &target, &mut |_, _| {}, &cancel), Err(Error::Checksum { .. })));
        assert_eq!(std::fs::read(&target).ok().as_deref(), Some(&b"old build"[..]));

        let good = release(dir.path(), &format!("sha256:{}", sha256_hex(b"new build"))).unwrap();
        install(&good, &target, &mut |_, _| {}, &cancel).unwrap();
        assert_eq!(std::fs::read(&target).ok().as_deref(), Some(&b"new build"[..]));
        assert!(!sibling(&target, "old").exists() && !sibling(&target, "new").exists());
    }
}
