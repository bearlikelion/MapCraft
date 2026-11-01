//! The SDK manager's headless half: finding installed SDKs, registries, installing and removing,
//! packaging, new SDKs from the template or a TrenchBroom game, and checks.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod check;
mod install;
mod registry;
mod trenchbroom;
pub mod update;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use mapcraft_sdk::{Sdk, SdkError};

pub use check::{Item, Level, check, samples, worst};
pub use install::{create, extract, install, install_archive, package, sha256_hex, uninstall};
pub use registry::{DEFAULT_REGISTRY, Entry, FORMAT, Registry, VERSION, fetch, fetch_registry};
pub use trenchbroom::TrenchBroomImport;

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
    /// The source tree MapCraft was built from.
    Source,
    /// A folder from the preferences or `MAPCRAFT_SDK_PATH`.
    Folder,
    /// Shipped in `sdks` next to the executable, updated with it.
    Bundled,
}

impl Origin {
    pub fn label(self) -> &'static str {
        match self {
            Origin::User => "user",
            Origin::Source => "source tree",
            Origin::Folder => "folder",
            Origin::Bundled => "bundled",
        }
    }
}

/// The user's data folder, `~/.local/share/mapcraft` on Linux.
pub fn data_dir() -> Option<PathBuf> {
    craft_settings::paths::data_dir(APP_ID)
}

/// The settings and data folder name; a branded build sets `MAPCRAFT_APP_ID` to keep its own.
pub const APP_ID: &str = match option_env!("MAPCRAFT_APP_ID") {
    Some(id) => id,
    None => "mapcraft",
};

/// Where SDKs are installed and created inside a data folder.
pub fn user_dir(data: &Path) -> PathBuf {
    data.join("sdks")
}

/// Folders searched for SDKs, each once, so earlier copies shadow later ones: `sdks` next to the
/// executable, the data folder's `sdks`, the source tree this was built from, `MAPCRAFT_SDK_PATH`,
/// and `extra` folders from the preferences.
pub fn search_dirs(data: Option<&Path>, extra: &[String]) -> Vec<(PathBuf, Origin)> {
    let exe = std::env::current_exe().ok();
    dirs_with(exe.as_deref().and_then(Path::parent).map(|d| d.join("sdks")), data, extra)
}

fn dirs_with(bundled: Option<PathBuf>, data: Option<&Path>, extra: &[String]) -> Vec<(PathBuf, Origin)> {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sdks");
    let mut dirs: Vec<(PathBuf, Origin)> = bundled.map(|d| (d, Origin::Bundled)).into_iter().collect();
    dirs.extend(data.map(|d| (user_dir(d), Origin::User)));
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
    pub logo: Option<PathBuf>,
}

/// Every SDK folder in the search folders, in search order.
pub fn installed(dirs: &[(PathBuf, Origin)]) -> Vec<Installed> {
    let mut out = Vec::new();
    for (dir, origin) in dirs {
        let found = mapcraft_sdk::discover(std::slice::from_ref(dir));
        out.extend(found.into_iter().map(|(dir, sdk)| Installed { logo: logo(&dir), dir, origin: *origin, sdk: sdk.map_err(|e| e.to_string()) }));
    }
    out
}

/// The SDK folder's logo: `logo.png`, else `logo.jpg`.
pub fn logo(dir: &Path) -> Option<PathBuf> {
    ["logo.png", "logo.jpg"].into_iter().map(|n| dir.join(n)).find(|p| p.is_file())
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

    #[test]
    fn bundled_sdks_come_first() {
        let tmp = tempfile::tempdir().unwrap();
        for dir in ["app/sdks/demo", "data/sdks/demo"] {
            std::fs::create_dir_all(tmp.path().join(dir)).unwrap();
            std::fs::write(tmp.path().join(dir).join("sdk.toml"), "id = \"demo\"\nname = \"Demo\"\n").unwrap();
        }
        let found = installed(&dirs_with(Some(tmp.path().join("app/sdks")), Some(&tmp.path().join("data")), &[]));
        let origins: Vec<_> = found.iter().filter(|i| i.dir.ends_with("demo")).map(|i| i.origin).collect();
        assert_eq!(origins, [Origin::Bundled, Origin::User]);
    }
}
