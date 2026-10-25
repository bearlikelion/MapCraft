//! The SDK manager's headless half: finding installed SDKs, registries, installing and removing,
//! packaging, new SDKs from the template, and checks.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod check;
mod install;
mod registry;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use mapcraft_sdk::{Sdk, SdkError};

pub use check::{Item, Level, check, samples, worst};
pub use install::{create, extract, install, install_archive, package, sha256_hex, uninstall};
pub use registry::{DEFAULT_REGISTRY, Entry, FORMAT, Registry, VERSION, fetch, fetch_registry};

/// This MapCraft's version, compared with an entry's `min_mapcraft`.
pub const MAPCRAFT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{}: {source}", path.display())]
    Io { path: PathBuf, source: std::io::Error },
    #[error("{url}: {message}")]
    Download { url: String, message: String },
    #[error("registry: {0}")]
    Registry(String),
    #[error("the download does not match its checksum (expected {expected}, got {actual})")]
    Checksum { expected: String, actual: String },
    #[error("the archive holds an unsafe path: {0}")]
    UnsafePath(String),
    #[error("{0}")]
    TooLarge(String),
    #[error("archive: {0}")]
    Zip(String),
    #[error(transparent)]
    Sdk(#[from] SdkError),
    #[error("{0}")]
    Invalid(String),
    #[error("cancelled")]
    Cancelled,
}

pub(crate) fn io_err(path: &Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
    move |source| Error::Io { path: path.to_path_buf(), source }
}

/// Where an SDK folder was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Installed or created by the user; the only kind that may be removed.
    User,
    /// Shipped next to the MapCraft executable.
    Bundled,
    /// The source tree MapCraft was built from.
    Source,
    /// A folder from the preferences or `MAPCRAFT_SDK_PATH`.
    Folder,
}

impl Origin {
    pub fn label(self) -> &'static str {
        match self {
            Origin::User => "user",
            Origin::Bundled => "bundled",
            Origin::Source => "source tree",
            Origin::Folder => "folder",
        }
    }
}

/// The user's data folder, `~/.local/share/mapcraft` on Linux.
pub fn data_dir() -> Option<PathBuf> {
    craft_settings::paths::data_dir("mapcraft")
}

/// Where SDKs are installed and created inside a data folder.
pub fn user_dir(data: &Path) -> PathBuf {
    data.join("sdks")
}

/// Folders searched for SDKs, each once and user installs first so they shadow other copies: the
/// data folder's `sdks`, an `sdks` folder beside the executable or up to four folders above it (a
/// build in `target/release` finds the source tree's), the installed share folder, the source
/// tree this was built from, `MAPCRAFT_SDK_PATH`, and `extra` folders from the preferences.
pub fn search_dirs(data: Option<&Path>, extra: &[String]) -> Vec<(PathBuf, Origin)> {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sdks");
    let mut dirs: Vec<(PathBuf, Origin)> = data.map(|d| (user_dir(d), Origin::User)).into_iter().collect();
    if let Some(exe_dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        dirs.extend(exe_dir.ancestors().take(5).map(|d| (d.join("sdks"), Origin::Bundled)));
        dirs.push((exe_dir.join("../share/mapcraft/sdks"), Origin::Bundled));
    }
    dirs.push((source.clone(), Origin::Source));
    if let Some(paths) = std::env::var_os("MAPCRAFT_SDK_PATH") {
        dirs.extend(std::env::split_paths(&paths).map(|p| (p, Origin::Folder)));
    }
    dirs.extend(extra.iter().map(|d| (PathBuf::from(d), Origin::Folder)));
    let source = source.canonicalize().ok();
    let mut seen = BTreeSet::new();
    dirs.retain_mut(|(dir, origin)| {
        let Ok(c) = dir.canonicalize() else { return false };
        if Some(&c) == source.as_ref() {
            *origin = Origin::Source;
        }
        seen.insert(c)
    });
    dirs
}

/// An SDK folder found in a search folder, loaded or with the reason it did not load.
#[derive(Clone, Debug)]
pub struct Installed {
    pub dir: PathBuf,
    pub origin: Origin,
    pub sdk: Result<Sdk, String>,
}

/// Every SDK folder in the search folders, in search order.
pub fn installed(dirs: &[(PathBuf, Origin)]) -> Vec<Installed> {
    let mut out = Vec::new();
    for (dir, origin) in dirs {
        let found = mapcraft_sdk::discover(std::slice::from_ref(dir));
        out.extend(found.into_iter().map(|(dir, sdk)| Installed { dir, origin: *origin, sdk: sdk.map_err(|e| e.to_string()) }));
    }
    out
}

/// Whether version `a` is newer than `b`: numerically for dotted numbers, else when they differ.
pub fn newer(a: &str, b: &str) -> bool {
    let parts = |s: &str| {
        let mut v = s.trim().split('.').map(|p| p.parse::<u64>().ok()).collect::<Option<Vec<_>>>()?;
        while v.last() == Some(&0) {
            v.pop();
        }
        Some(v)
    };
    match (parts(a), parts(b)) {
        (Some(x), Some(y)) => x > y,
        _ => a.trim() != b.trim(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert!(newer("1.10", "1.9"));
        assert!(!newer("1.0.0", "1.0"));
        assert!(!newer("0.1.0", MAPCRAFT_VERSION));
        assert!(newer("3-beta", "3"));
        assert!(!newer("", ""));
    }

    #[test]
    fn finds_the_source_tree_sdks_once() {
        let source = concat!(env!("CARGO_MANIFEST_DIR"), "/../../sdks").to_string();
        let found = installed(&search_dirs(None, &[source]));
        let surfsup: Vec<_> = found.iter().filter(|i| i.sdk.as_ref().is_ok_and(|s| s.manifest.id == "surfsup")).collect();
        assert_eq!(surfsup.len(), 1);
        assert_eq!(surfsup[0].origin, Origin::Source);
    }
}
