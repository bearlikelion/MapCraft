//! Game SDKs: the `sdk.toml` manifest, the asset file system and compile profiles.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod compile;
mod vfs;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use mapcraft_fgd::GameData;
use serde::Deserialize;

pub use compile::{CompileError, CompileProfile, Step, builtin_vars, expand, run_profile};
pub use vfs::Vfs;

pub const MANIFEST: &str = "sdk.toml";

#[derive(Debug, thiserror::Error)]
pub enum SdkError {
    #[error("{}: {source}", path.display())]
    Io { path: PathBuf, source: std::io::Error },
    #[error("{}: {source}", path.display())]
    Toml { path: PathBuf, source: toml::de::Error },
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Compile(#[from] CompileError),
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    /// SPDX expression, such as `MIT`.
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub homepage: String,
    /// Ids of the plugins this SDK needs.
    #[serde(default)]
    pub plugins: Vec<String>,
    #[serde(default = "default_units_per_meter")]
    pub units_per_meter: f64,
    /// Entity definition files, relative to the SDK folder; `${var}` paths are expanded.
    #[serde(default)]
    pub fgd: Vec<String>,
    #[serde(default)]
    pub textures: Textures,
    /// More folders plugins may read, such as a game install; ones whose paths are unset are skipped.
    #[serde(default)]
    pub files: Files,
    /// Folders and programs the user points the SDK at, available as `${name}`.
    #[serde(default)]
    pub paths: BTreeMap<String, PathSetting>,
    #[serde(default)]
    pub compile: Vec<CompileProfile>,
}

fn default_units_per_meter() -> f64 {
    39.37
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Files {
    #[serde(default)]
    pub roots: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Textures {
    #[serde(default)]
    pub roots: Vec<String>,
    #[serde(default = "default_extensions")]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default = "default_scale")]
    pub scale: f64,
}

impl Default for Textures {
    fn default() -> Self {
        Self { roots: Vec::new(), extensions: default_extensions(), default: None, scale: default_scale() }
    }
}

fn default_extensions() -> Vec<String> {
    ["png", "jpg", "jpeg", "tga", "bmp"].map(String::from).to_vec()
}

fn default_scale() -> f64 {
    1.0
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathSetting {
    pub prompt: String,
    #[serde(default)]
    pub default: Option<String>,
    /// `dir` or `file`.
    #[serde(default = "default_kind")]
    pub kind: String,
}

fn default_kind() -> String {
    "dir".into()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sdk {
    pub dir: PathBuf,
    pub manifest: Manifest,
}

impl Sdk {
    pub fn load(dir: &Path) -> Result<Self, SdkError> {
        let path = dir.join(MANIFEST);
        let text = std::fs::read_to_string(&path).map_err(|source| SdkError::Io { path: path.clone(), source })?;
        let manifest: Manifest = toml::from_str(&text).map_err(|source| SdkError::Toml { path, source })?;
        if !valid_id(&manifest.id) {
            return Err(SdkError::Invalid(format!("SDK id `{}` must use lowercase letters, digits, '_' or '-'", manifest.id)));
        }
        if !manifest.units_per_meter.is_finite() || manifest.units_per_meter <= 0.0 {
            return Err(SdkError::Invalid("units_per_meter must be a positive number".into()));
        }
        if !(manifest.textures.scale != 0.0 && manifest.textures.scale.is_finite()) {
            return Err(SdkError::Invalid("textures.scale must be a non-zero number".into()));
        }
        Ok(Self { dir: dir.to_path_buf(), manifest })
    }

    /// Resolves a manifest path: `${var}`s expanded, relative paths taken from the SDK folder.
    pub fn resolve(&self, raw: &str, vars: &BTreeMap<String, String>) -> Result<PathBuf, SdkError> {
        let mut vars = vars.clone();
        vars.entry("sdk_dir".into()).or_insert_with(|| self.dir.to_string_lossy().into_owned());
        Ok(self.dir.join(expand(raw, &vars)?))
    }

    /// All FGDs merged in manifest order.
    pub fn game_data(&self, vars: &BTreeMap<String, String>) -> Result<GameData, SdkError> {
        let mut gd = GameData::default();
        for raw in &self.manifest.fgd {
            let path = self.resolve(raw, vars)?;
            gd.extend(GameData::load(&path, &mut |p| std::fs::read(p)));
        }
        Ok(gd)
    }

    pub fn vfs(&self, vars: &BTreeMap<String, String>) -> Result<Vfs, SdkError> {
        let roots = self.manifest.textures.roots.iter().map(|r| self.resolve(r, vars)).collect::<Result<Vec<_>, _>>()?;
        Ok(Vfs::new(roots))
    }

    /// What plugins may read: the texture folders, then the extra file folders that are set.
    pub fn plugin_vfs(&self, vars: &BTreeMap<String, String>) -> Vfs {
        let textures = self.manifest.textures.roots.iter();
        let roots = textures.chain(&self.manifest.files.roots).filter_map(|r| self.resolve(r, vars).ok()).filter(|p| p.is_dir()).collect();
        Vfs::new(roots)
    }
}

/// SDK and plugin ids: lowercase letters, digits, `_` and `-`, so they are safe as folder names.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// Every SDK folder (one holding `sdk.toml`) directly inside the given directories.
pub fn discover(dirs: &[PathBuf]) -> Vec<(PathBuf, Result<Sdk, SdkError>)> {
    let mut out = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        let mut found: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.join(MANIFEST).is_file()).collect();
        found.sort();
        out.extend(found.into_iter().map(|p| {
            let sdk = Sdk::load(&p);
            (p, sdk)
        }));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SURFSUP: &str = r#"
id = "surfsup"
name = "SurfsUp"
plugins = ["godot", "surfsup"]
fgd = ["fgd/surfsup.fgd"]

[textures]
roots = ["${project}/Assets/Textures"]
default = "ramps-orange1"

[paths.project]
prompt = "SurfsUp Godot project folder"

[[compile]]
name = "Build and play"
steps = [
    { run = "${godot}", args = ["--path", "${project}", "+map", "${map_name}"] },
    { copy = "${map}", to = "${project}/maps/${map_name}.mcmap" },
]
"#;

    #[test]
    fn manifest_and_discovery() {
        let root = tempfile::tempdir().unwrap();
        let sdk_dir = root.path().join("surfsup");
        std::fs::create_dir_all(sdk_dir.join("fgd")).unwrap();
        std::fs::write(sdk_dir.join(MANIFEST), SURFSUP).unwrap();
        std::fs::write(sdk_dir.join("fgd/surfsup.fgd"), "@SolidClass = zone_start : \"Start\" [ course_id(integer) : \"Course\" : 0 ]").unwrap();
        let bad = root.path().join("bad");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join(MANIFEST), "id = \"Bad Id\"\nname = \"x\"").unwrap();

        let found = discover(&[root.path().to_path_buf()]);
        assert_eq!(found.len(), 2);
        assert!(found[0].1.is_err());
        let sdk = found[1].1.as_ref().unwrap();
        assert_eq!(sdk.manifest.units_per_meter, 39.37);
        assert_eq!(sdk.manifest.compile[0].steps.len(), 2);

        let vars = BTreeMap::from([("project".to_string(), "/games/surfsup".to_string())]);
        let gd = sdk.game_data(&vars).unwrap();
        assert!(gd.diagnostics.is_empty());
        assert!(gd.class("zone_start").is_some());
        assert_eq!(sdk.vfs(&vars).unwrap().roots(), [PathBuf::from("/games/surfsup/Assets/Textures")]);
        assert!(sdk.vfs(&BTreeMap::new()).is_err());
    }

    #[test]
    fn rejects_unknown_keys() {
        assert!(toml::from_str::<Manifest>("id = \"a\"\nname = \"A\"\ntypo = 1").is_err());
    }
}
