use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use mapcraft_fgd::GameData;
use mapcraft_geom::DVec3;
use mapcraft_plugin_host::{Host, Plugin};
use mapcraft_render::{EntityStyle, Renderer};
use mapcraft_sdk::{Sdk, Vfs};

/// A material in the browser: an image to show and texture with, or else a flat sRGB color.
#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub name: String,
    pub image: Option<PathBuf>,
    pub color: Option<[u8; 3]>,
}

/// Size of the texture made for a color-only material.
const SWATCH: u32 = 64;

/// A flat color texture with darker edges, so faces show their texture grid.
fn swatch(c: [u8; 3]) -> Vec<u8> {
    let mut out = Vec::with_capacity((SWATCH * SWATCH * 4) as usize);
    for y in 0..SWATCH {
        for x in 0..SWATCH {
            let edge = x == 0 || y == 0;
            out.extend(c.map(|v| if edge { (u16::from(v) * 3 / 4) as u8 } else { v }));
            out.push(255);
        }
    }
    out
}

/// The SDK a map is edited with, and everything loaded from it.
#[derive(Default)]
pub struct Project {
    pub sdk: Option<Sdk>,
    pub vars: BTreeMap<String, String>,
    pub game: GameData,
    pub vfs: Vfs,
    /// Materials available in the SDK's texture folders, sorted by name.
    pub materials: Vec<Material>,
    pub plugins: Vec<Plugin>,
    pub problems: Vec<String>,
}

impl Project {
    /// Loads the SDK and its plugins, plus every plugin in `user_plugins`.
    pub fn load(sdk: Option<Sdk>, vars: BTreeMap<String, String>, host: Option<&Host>, user_plugins: Option<&Path>) -> Self {
        let mut p = Self { sdk, vars, ..Self::default() };
        let Some(sdk) = &p.sdk else {
            if let Some(host) = host {
                p.load_plugins(host, user_plugins);
            }
            return p;
        };
        match sdk.game_data(&p.vars) {
            Ok(gd) => {
                p.problems.extend(gd.diagnostics.iter().map(|(f, d)| format!("{}:{d}", f.display())));
                p.game = gd;
            }
            Err(e) => p.problems.push(e.to_string()),
        }
        match sdk.vfs(&p.vars) {
            Ok(vfs) => {
                p.materials =
                    vfs.list(&sdk.manifest.textures.extensions).into_iter().map(|(name, path)| Material { name, image: Some(path), color: None }).collect();
                p.vfs = vfs;
            }
            Err(e) => p.problems.push(e.to_string()),
        }
        if let Some(host) = host {
            p.load_plugins(host, user_plugins);
        }
        p
    }

    /// Loads the SDK's plugins from its `plugins` folder; a plugin that lists materials replaces
    /// the plain image listing.
    fn load_plugins(&mut self, host: &Host, user_plugins: Option<&Path>) {
        let mut paths: Vec<PathBuf> =
            self.sdk.iter().flat_map(|sdk| sdk.manifest.plugins.iter().map(|id| sdk.dir.join("plugins").join(format!("{id}.wasm")))).collect();
        if let Some(dir) = user_plugins {
            let mut user: Vec<PathBuf> =
                std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "wasm")).collect();
            user.sort();
            paths.extend(user);
        }
        for path in paths {
            match host.load_file(&path, self.vfs.clone()) {
                Ok(plugin) if self.plugins.iter().any(|p| p.manifest.id == plugin.manifest.id) => {
                    self.problems.push(format!("{}: plugin `{}` is already loaded", path.display(), plugin.manifest.id));
                }
                Ok(plugin) => self.plugins.push(plugin),
                Err(e) => self.problems.push(format!("{}: {e}", path.display())),
            }
        }
        let mut listed = Vec::new();
        for plugin in self.plugins.iter_mut().filter(|p| p.manifest.materials) {
            match plugin.materials() {
                Ok(list) => {
                    let found = list.into_iter().map(|m| Material { image: m.image.and_then(|i| self.vfs.find(&i)), name: m.name, color: m.color });
                    listed.extend(found.filter(|m| m.image.is_some() || m.color.is_some()));
                }
                Err(e) => self.problems.push(e.to_string()),
            }
            self.problems.extend(plugin.take_log());
        }
        if !listed.is_empty() {
            listed.sort_by(|a, b| a.name.cmp(&b.name));
            listed.dedup_by(|b, a| a.name == b.name);
            self.materials = listed;
        }
    }

    /// The loaded plugin with this id.
    pub fn plugin(&mut self, id: &str) -> Option<&mut Plugin> {
        self.plugins.iter_mut().find(|p| p.manifest.id == id)
    }

    pub fn id(&self) -> Option<&str> {
        self.sdk.as_ref().map(|s| s.manifest.id.as_str())
    }

    /// Paths the SDK asks for that the user has not set yet.
    pub fn missing_paths(&self) -> Vec<String> {
        let Some(sdk) = &self.sdk else { return Vec::new() };
        sdk.manifest.paths.keys().filter(|k| self.vars.get(*k).is_none_or(|v| v.is_empty())).cloned().collect()
    }

    /// Unset paths that materials or entity definitions come from, so editing needs them; the
    /// rest only matter when compiling.
    pub fn missing_for_editing(&self) -> Vec<String> {
        let Some(sdk) = &self.sdk else { return Vec::new() };
        let m = &sdk.manifest;
        let used: Vec<&String> = m.textures.roots.iter().chain(&m.fgd).collect();
        self.missing_paths().into_iter().filter(|k| used.iter().any(|s| s.contains(&format!("${{{k}}}")))).collect()
    }

    pub fn material(&self, name: &str) -> Option<&Material> {
        self.materials.binary_search_by(|m| m.name.as_str().cmp(name)).ok().and_then(|i| self.materials.get(i))
    }

    /// Decodes and uploads every material in `names` the renderer does not have yet; returns how many failed.
    pub fn upload_materials<'a>(&self, renderer: &mut Renderer, names: impl Iterator<Item = &'a str>) -> usize {
        let mut failed = 0;
        for name in names {
            if name.is_empty() || renderer.has_material(name) {
                continue;
            }
            // Images a material uses are not listed, but faces may still name them.
            let loose = self.sdk.as_ref().and_then(|sdk| self.vfs.find_material(name, &sdk.manifest.textures.extensions));
            let fallback = loose.map(|p| Material { name: name.to_string(), image: Some(p), color: None });
            let Some(m) = self.material(name).or(fallback.as_ref()) else { continue };
            let Some(path) = &m.image else {
                if let Some(c) = m.color
                    && !renderer.upload_material(name, SWATCH, SWATCH, &swatch(c))
                {
                    failed += 1;
                }
                continue;
            };
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

/// Folders searched for SDKs, each once: an `sdks` folder beside the executable or up to four
/// folders above it (a build in `target/release` finds the source tree's), the installed share
/// folder, the source tree this was built from, `MAPCRAFT_SDK_PATH`, the user's data folder, and
/// folders from the preferences.
pub fn sdk_search_dirs(extra: &[String]) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(exe_dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        dirs.extend(exe_dir.ancestors().take(5).map(|d| d.join("sdks")));
        dirs.push(exe_dir.join("../share/mapcraft/sdks"));
    }
    dirs.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sdks"));
    if let Some(paths) = std::env::var_os("MAPCRAFT_SDK_PATH") {
        dirs.extend(std::env::split_paths(&paths));
    }
    if let Some(data) = craft_settings::paths::data_dir("mapcraft") {
        dirs.push(data.join("sdks"));
    }
    dirs.extend(extra.iter().map(PathBuf::from));
    let mut seen = std::collections::BTreeSet::new();
    dirs.retain(|d| d.canonicalize().is_ok_and(|c| seen.insert(c)));
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_source_tree_sdks_once() {
        let dirs = sdk_search_dirs(&[String::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../sdks"))]);
        let found = mapcraft_sdk::discover(&dirs);
        assert_eq!(found.iter().filter(|(_, sdk)| sdk.as_ref().is_ok_and(|s| s.manifest.id == "surfsup")).count(), 1);
    }
}
