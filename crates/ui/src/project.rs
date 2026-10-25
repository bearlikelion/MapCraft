use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use mapcraft_fgd::GameData;
use mapcraft_geom::DVec3;
use mapcraft_render::{EntityStyle, Renderer};
use mapcraft_sdk::{Sdk, Vfs};

/// The SDK a map is edited with, and everything loaded from it.
#[derive(Default)]
pub struct Project {
    pub sdk: Option<Sdk>,
    pub vars: BTreeMap<String, String>,
    pub game: GameData,
    pub vfs: Vfs,
    /// Material names available in the SDK's texture folders, with their files.
    pub materials: Vec<(String, PathBuf)>,
    pub problems: Vec<String>,
}

impl Project {
    pub fn load(sdk: Option<Sdk>, vars: BTreeMap<String, String>) -> Self {
        let mut p = Self { sdk, vars, ..Self::default() };
        let Some(sdk) = &p.sdk else { return p };
        match sdk.game_data(&p.vars) {
            Ok(gd) => {
                p.problems.extend(gd.diagnostics.iter().map(|(f, d)| format!("{}:{d}", f.display())));
                p.game = gd;
            }
            Err(e) => p.problems.push(e.to_string()),
        }
        match sdk.vfs(&p.vars) {
            Ok(vfs) => {
                p.materials = vfs.list(&sdk.manifest.textures.extensions);
                p.vfs = vfs;
            }
            Err(e) => p.problems.push(e.to_string()),
        }
        p
    }

    pub fn id(&self) -> Option<&str> {
        self.sdk.as_ref().map(|s| s.manifest.id.as_str())
    }

    /// Paths the SDK asks for that the user has not set yet.
    pub fn missing_paths(&self) -> Vec<String> {
        let Some(sdk) = &self.sdk else { return Vec::new() };
        sdk.manifest.paths.keys().filter(|k| self.vars.get(*k).is_none_or(|v| v.is_empty())).cloned().collect()
    }

    pub fn material_file(&self, name: &str) -> Option<&Path> {
        self.materials.binary_search_by(|(n, _)| n.as_str().cmp(name)).ok().and_then(|i| self.materials.get(i)).map(|(_, p)| p.as_path())
    }

    /// Decodes and uploads every material in `names` the renderer does not have yet; returns how many failed.
    pub fn upload_materials<'a>(&self, renderer: &mut Renderer, names: impl Iterator<Item = &'a str>) -> usize {
        let mut failed = 0;
        for name in names {
            if name.is_empty() || renderer.has_material(name) {
                continue;
            }
            let Some(path) = self.material_file(name) else { continue };
            match image::open(path) {
                Ok(img) => {
                    let rgba = img.to_rgba8();
                    if !renderer.upload_material(name, rgba.width(), rgba.height(), rgba.as_raw()) {
                        failed += 1;
                    }
                }
                Err(e) => {
                    log::warn!("{}: {e}", path.display());
                    failed += 1;
                }
            }
        }
        failed
    }
}

impl EntityStyle for Project {
    fn color(&self, classname: &str) -> Option<[u8; 3]> {
        self.game.class(classname)?.color()
    }

    fn size(&self, classname: &str) -> Option<(DVec3, DVec3)> {
        let (a, b) = self.game.class(classname)?.size()?;
        Some((DVec3::from_array(a), DVec3::from_array(b)))
    }
}

/// Folders searched for SDKs: next to the executable, the user's data folder, the source tree in
/// development builds, and folders from the preferences.
pub fn sdk_search_dirs(extra: &[String]) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(exe_dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        dirs.push(exe_dir.join("sdks"));
        dirs.push(exe_dir.join("../share/mapcraft/sdks"));
    }
    if cfg!(debug_assertions) {
        dirs.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sdks"));
    }
    if let Some(data) = craft_settings::paths::data_dir("mapcraft") {
        dirs.push(data.join("sdks"));
    }
    dirs.extend(extra.iter().map(PathBuf::from));
    dirs
}
