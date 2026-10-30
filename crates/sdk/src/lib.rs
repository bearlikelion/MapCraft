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
    /// The lowest and highest coordinate a map may use on every axis, such as Quake's `[-4096, 4096]`;
    /// unset, an FGD's `@mapsize` gives it, else maps are unbounded.
    #[serde(default)]
    pub map_size: Option<[i64; 2]>,
    /// Entity definition files, relative to the SDK folder; `${var}` paths are expanded.
    #[serde(default)]
    pub fgd: Vec<String>,
    #[serde(default)]
    pub textures: Textures,
    #[serde(default)]
    pub models: Models,
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
    /// Textures are magnified without smoothing, as the game draws them.
    #[serde(default)]
    pub pixelated: bool,
    /// Material name prefixes that are skies in Quake's two-layer layout.
    #[serde(default)]
    pub sky: Vec<String>,
}

impl Default for Textures {
    fn default() -> Self {
        Self { roots: Vec::new(), extensions: default_extensions(), default: None, scale: default_scale(), pixelated: false, sky: Vec::new() }
    }
}

fn default_extensions() -> Vec<String> {
    ["png", "jpg", "jpeg", "tga", "bmp"].map(String::from).to_vec()
}

fn default_scale() -> f64 {
    1.0
}

/// glTF models: where they live and the point entity class the Models tab places them with.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Models {
    /// Folders model paths are relative to; empty uses the texture folders.
    #[serde(default)]
    pub roots: Vec<String>,
    #[serde(default)]
    pub class: Option<String>,
    /// The key that holds the model path.
    #[serde(default = "default_model_key")]
    pub key: String,
}

impl Default for Models {
    fn default() -> Self {
        Self { roots: Vec::new(), class: None, key: default_model_key() }
    }
}

fn default_model_key() -> String {
    "model".into()
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
    /// Texture folders and single files the user added in SDK Settings, searched after the manifest's.
    pub user_textures: Vec<PathBuf>,
}

impl Sdk {
    pub fn load(dir: &Path) -> Result<Self, SdkError> {
        let path = dir.join(MANIFEST);
        let text = std::fs::read_to_string(&path).map_err(|source| SdkError::Io { path: path.clone(), source })?;
        let mut manifest: Manifest = toml::from_str(&text).map_err(|source| SdkError::Toml { path, source })?;
        manifest.compile.retain(|p| p.os.as_deref().is_none_or(|os| os == std::env::consts::OS));
        if !valid_id(&manifest.id) {
            return Err(SdkError::Invalid(format!("SDK id `{}` must use lowercase letters, digits, '_' or '-'", manifest.id)));
        }
        if !manifest.units_per_meter.is_finite() || manifest.units_per_meter <= 0.0 {
            return Err(SdkError::Invalid("units_per_meter must be a positive number".into()));
        }
        if !(manifest.textures.scale != 0.0 && manifest.textures.scale.is_finite()) {
            return Err(SdkError::Invalid("textures.scale must be a non-zero number".into()));
        }
        Ok(Self { dir: dir.to_path_buf(), manifest, user_textures: Vec::new() })
    }

    /// Fills paths the user left unset with the manifest's defaults, which may name other paths,
    /// such as `${cstrike}/../bin/cstrike.fgd`; a default naming an unset path stays unset.
    pub fn apply_path_defaults(&self, vars: &mut BTreeMap<String, String>) {
        for (k, setting) in &self.manifest.paths {
            if let (None, Some(d)) = (vars.get(k), &setting.default)
                && let Ok(value) = expand(d, vars)
            {
                vars.insert(k.clone(), value);
            }
        }
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
        if let Some([low, high]) = self.manifest.map_size {
            gd.map_size = Some((low, high));
        }
        Ok(gd)
    }

    /// The texture folders: the manifest's whose paths are set, then the user's.
    pub fn vfs(&self, vars: &BTreeMap<String, String>) -> Vfs {
        let roots = self.manifest.textures.roots.iter().filter_map(|r| self.resolve(r, vars).ok());
        Vfs::new(roots.chain(self.user_textures.iter().cloned()).collect())
    }

    /// Where model paths resolve: the `[models]` folders whose paths are set, else the texture folders.
    pub fn model_vfs(&self, vars: &BTreeMap<String, String>) -> Vfs {
        let models = &self.manifest.models.roots;
        let roots = if models.is_empty() { &self.manifest.textures.roots } else { models };
        Vfs::new(roots.iter().filter_map(|r| self.resolve(r, vars).ok()).collect())
    }

    /// What plugins may read: the texture folders, the user's folders and files, then the extra file folders that are set.
    pub fn plugin_vfs(&self, vars: &BTreeMap<String, String>) -> Vfs {
        let resolve = |r: &String| self.resolve(r, vars).ok();
        let textures = self.manifest.textures.roots.iter().filter_map(resolve).chain(self.user_textures.iter().cloned());
        Vfs::new(textures.chain(self.manifest.files.roots.iter().filter_map(resolve)).filter(|p| p.exists()).collect())
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

[[compile]]
name = "Elsewhere"
os = "plan9"
steps = []
"#;

    #[test]
    fn manifest_and_discovery() {
        let root = tempfile::tempdir().unwrap();
        let sdk_dir = root.path().join("surfsup");
        std::fs::create_dir_all(sdk_dir.join("fgd")).unwrap();
        std::fs::write(sdk_dir.join(MANIFEST), SURFSUP).unwrap();
        std::fs::write(
            sdk_dir.join("fgd/surfsup.fgd"),
            "@mapsize(-16384, 16384)\n@SolidClass = zone_start : \"Start\" [ course_id(integer) : \"Course\" : 0 ]",
        )
        .unwrap();
        let bad = root.path().join("bad");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join(MANIFEST), "id = \"Bad Id\"\nname = \"x\"").unwrap();

        let found = discover(&[root.path().to_path_buf()]);
        assert_eq!(found.len(), 2);
        assert!(found[0].1.is_err());
        let sdk = found[1].1.as_ref().unwrap();
        assert_eq!(sdk.manifest.units_per_meter, 39.37);
        assert_eq!(sdk.manifest.compile[0].steps.len(), 2);
        assert_eq!(sdk.manifest.compile.len(), 1, "profiles for another OS are dropped");

        let project = root.path().join("games/surfsup");
        let vars = BTreeMap::from([("project".to_string(), project.to_string_lossy().into_owned())]);
        let gd = sdk.game_data(&vars).unwrap();
        assert!(gd.diagnostics.is_empty());
        assert!(gd.class("zone_start").is_some());
        assert_eq!(gd.map_size, Some((-16384, 16384)), "the FGD's @mapsize, with none in the manifest");
        let mut quake = sdk.clone();
        quake.manifest.map_size = Some([-4096, 4096]);
        assert_eq!(quake.game_data(&vars).unwrap().map_size, Some((-4096, 4096)), "the manifest wins");
        assert_eq!(sdk.vfs(&vars).roots(), [project.join("Assets/Textures")]);
        assert_eq!(sdk.model_vfs(&vars).roots(), sdk.vfs(&vars).roots(), "models default to the texture folders");
        assert_eq!((sdk.manifest.models.class.as_deref(), sdk.manifest.models.key.as_str()), (None, "model"));
        let mut props = sdk.clone();
        props.manifest.models = toml::from_str("roots = [\"${project}\"]\nclass = \"prop_static\"").unwrap();
        assert_eq!(props.model_vfs(&vars).roots(), std::slice::from_ref(&project));
        assert_eq!(props.manifest.models.key, "model");
        assert!(sdk.vfs(&BTreeMap::new()).roots().is_empty(), "folders whose paths are unset are left out");
    }

    #[test]
    fn user_texture_folders_follow_the_sdks_own() {
        let root = tempfile::tempdir().unwrap();
        let (own, user, game) = (root.path().join("p/Assets/Textures"), root.path().join("mine"), root.path().join("game"));
        let wad = root.path().join("jams/jam.wad");
        for file in [own.join("wall.png"), user.join("wall.png"), user.join("metal/plate.png"), game.join("x.png"), wad.clone()] {
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, b"").unwrap();
        }
        let user_textures = vec![user.clone(), root.path().join("gone"), wad.clone()];
        let mut sdk = Sdk { dir: root.path().into(), manifest: toml::from_str(SURFSUP).unwrap(), user_textures };
        sdk.manifest.files.roots = vec!["${game}".into()];
        let s = |p: &Path| p.to_string_lossy().into_owned();
        let vars = BTreeMap::from([("project".to_string(), s(&root.path().join("p"))), ("game".to_string(), s(&game))]);
        let vfs = sdk.vfs(&vars);
        assert_eq!(vfs.roots(), [own.clone(), user.clone(), root.path().join("gone"), wad.clone()]);
        let plugins = sdk.plugin_vfs(&vars);
        assert_eq!(plugins.roots(), [own.clone(), user.clone(), wad, game], "missing folders are left out of what plugins read, single files are not");
        assert_eq!(plugins.list(&["wad".to_string()]).len(), 1);
        let listed = vfs.list(&["png".to_string()]);
        assert_eq!(
            listed,
            [("metal/plate".to_string(), user.join("metal/plate.png")), ("wall".to_string(), own.join("wall.png"))],
            "the SDK's own folders win a clash"
        );
    }

    #[test]
    fn path_defaults_expand_other_paths() {
        let manifest = "id = \"a\"\nname = \"A\"\n[paths.game]\nprompt = \"g\"\n[paths.fgd]\nprompt = \"f\"\ndefault = \"${game}/bin/a.fgd\"";
        let sdk = Sdk { dir: PathBuf::new(), manifest: toml::from_str(manifest).unwrap(), user_textures: Vec::new() };
        let mut vars = BTreeMap::new();
        sdk.apply_path_defaults(&mut vars);
        assert!(vars.is_empty(), "a default naming an unset path stays unset");
        vars.insert("game".to_string(), "/g".to_string());
        sdk.apply_path_defaults(&mut vars);
        assert_eq!(vars["fgd"], "/g/bin/a.fgd");
    }

    #[test]
    fn rejects_unknown_keys() {
        assert!(toml::from_str::<Manifest>("id = \"a\"\nname = \"A\"\ntypo = 1").is_err());
    }
}
