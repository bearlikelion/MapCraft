//! A map's own asset folders, such as the one BSPSource writes beside a decompiled map, and the
//! window that asks for the textures and models a map uses that no folder holds.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use egui::RichText;
use mapcraft_format::pack;
use serde_json::Value;

use crate::app::MapCraftApp;

pub(crate) fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path).and_then(|file| file.take(limit.saturating_add(1)).read_to_end(&mut bytes)).map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.len() as u64 > limit {
        return Err(format!("{} exceeds {limit} bytes", path.display()));
    }
    Ok(bytes)
}

/// Folders searched after the SDK's for this map, kept in its editor state.
const FOLDERS: &str = "asset_folders";
/// Missing materials and models the mapper chose not to look for again.
const SKIPPED: &str = "skipped_assets";
/// Names the window lists before it says how many more there are.
const SHOWN: usize = 12;

/// What a map uses that was not found.
pub(crate) struct MissingAssets {
    materials: Vec<String>,
    models: Vec<String>,
}

fn list(view: &Value, key: &str) -> Vec<String> {
    view.get(key).and_then(Value::as_array).into_iter().flatten().filter_map(|v| v.as_str().map(str::to_string)).collect()
}

fn add(view: &mut Value, key: &str, items: impl IntoIterator<Item = String>) {
    let mut all: BTreeSet<String> = list(view, key).into_iter().collect();
    all.extend(items);
    if !view.is_object() {
        *view = Value::Object(serde_json::Map::new());
    }
    if let Some(o) = view.as_object_mut() {
        o.insert(key.into(), all.into_iter().map(Value::String).collect());
    }
}

/// Whether `dir` holds a `materials` or `models` folder, in any case.
fn holds_assets(dir: &Path) -> bool {
    std::fs::read_dir(dir).into_iter().flatten().flatten().any(|e| {
        let name = e.file_name();
        e.path().is_dir() && (name.eq_ignore_ascii_case("materials") || name.eq_ignore_ascii_case("models"))
    })
}

/// The map's asset folders, searched after the SDK's own.
pub(crate) fn folders(view: &Value) -> Vec<PathBuf> {
    list(view, FOLDERS).into_iter().map(PathBuf::from).collect()
}

impl MapCraftApp {
    /// Adds the folder a decompiler wrote beside `map` with its packed files, such as BSPSource's
    /// `surf_x_d/` beside `surf_x_d.vmf`; true when it did.
    pub(crate) fn add_companion_folder(&mut self, map: &Path) -> bool {
        let dir = map.with_extension("");
        if !holds_assets(&dir) || folders(&self.session.view).contains(&dir) {
            return false;
        }
        self.output.push(format!("Using {} for the textures and models packed with {}", dir.display(), map.display()));
        self.add_asset_folder(dir);
        true
    }

    /// Searches `dir` for this map's textures and models from now on, and reloads them.
    pub(crate) fn add_asset_folder(&mut self, dir: PathBuf) {
        add(&mut self.session.view, FOLDERS, [dir.to_string_lossy().into_owned()]);
        self.session.mark_unsaved();
        self.load_project(self.session.sdk.clone());
        self.assets_checked = false;
    }

    /// Once an import's materials and models have loaded, notes the ones the map uses that were not found.
    pub(crate) fn check_missing_assets(&mut self) {
        if self.assets_checked || self.material_list.is_some() || self.project.models.busy() || self.scene_dirty || self.gpu.is_none() {
            return;
        }
        self.assets_checked = true;
        let skipped: BTreeSet<String> = list(&self.session.view, SKIPPED).into_iter().collect();
        let map = self.view_map();
        let faces = map.nodes().filter_map(|(_, n)| n.kind.brush()).flat_map(|b| &b.faces);
        let names: BTreeSet<&str> = faces.flat_map(|f| [Some(f.material.as_str()), f.blend_layer()]).flatten().collect();
        let materials: Vec<String> =
            names.into_iter().filter(|n| !n.is_empty() && self.project.find_material(n).is_none() && !skipped.contains(*n)).map(str::to_string).collect();
        let paths: BTreeSet<String> = map.nodes().filter_map(|(_, n)| self.project.model_path(n.kind.entity()?)).collect();
        let models: Vec<String> = paths.into_iter().filter(|p| self.project.models.failed(p) && !skipped.contains(p)).collect();
        if !materials.is_empty() || !models.is_empty() {
            self.missing_assets = Some(MissingAssets { materials, models });
        }
    }

    /// Asks for a folder holding what the map is missing, or to go on without it.
    pub(crate) fn missing_assets_dialog(&mut self, ctx: &egui::Context) {
        let Some(missing) = &self.missing_assets else { return };
        let (mut locate, mut skip, mut open) = (false, false, true);
        let window = egui::Window::new("Missing Textures and Models").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]);
        window.open(&mut open).show(ctx, |ui| {
            let count = |n: usize, one: &str, many: &str| (n > 0).then(|| format!("{n} {}", if n == 1 { one } else { many }));
            let parts: Vec<String> =
                [count(missing.materials.len(), "material", "materials"), count(missing.models.len(), "model", "models")].into_iter().flatten().collect();
            ui.label(format!("This map uses {} that were not found.", parts.join(" and ")));
            let names: Vec<&String> = missing.materials.iter().chain(&missing.models).collect();
            egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                for name in names.iter().take(SHOWN) {
                    ui.label(RichText::new(name.as_str()).monospace().small());
                }
                if names.len() > SHOWN {
                    ui.label(RichText::new(format!("and {} more", names.len() - SHOWN)).color(crate::chrome::TEXT_WEAK));
                }
            });
            ui.label(
                RichText::new("Locate a folder holding them in materials and models folders, such as the one a decompiler writes beside the map.")
                    .color(crate::chrome::TEXT_WEAK),
            );
            ui.horizontal(|ui| {
                locate = ui.add(crate::icons::button(Some("folder_open"), "Locate Folder…")).clicked();
                skip = ui.button("Skip").on_hover_text("Leave them missing and stop asking about them for this map").clicked();
            });
        });
        if locate {
            if let Some(dir) = rfd::FileDialog::new().set_title("Folder holding the materials and models folders").pick_folder() {
                let named = |n: &str| dir.file_name().is_some_and(|f| f.eq_ignore_ascii_case(n));
                let dir = if named("materials") || named("models") { dir.parent().map_or(dir.clone(), Path::to_path_buf) } else { dir };
                self.missing_assets = None;
                self.add_asset_folder(dir);
            }
        } else if skip {
            let names: Vec<String> = missing.materials.iter().chain(&missing.models).cloned().collect();
            self.output.push(format!("Left {} missing textures and models as they are", names.len()));
            add(&mut self.session.view, SKIPPED, names);
            self.session.mark_unsaved();
            self.missing_assets = None;
        } else if !open {
            self.missing_assets = None;
        }
    }
}

impl MapCraftApp {
    /// The pictures and models the map takes from the player's own folders; the SDK's own ship with the game.
    pub(crate) fn custom_asset_entries(&self) -> Result<Vec<(String, Vec<u8>)>, String> {
        let Some(sdk) = &self.project.sdk else { return Ok(Vec::new()) };
        let own = |p: &Path| sdk.user_textures.iter().any(|u| p.starts_with(u));
        let mut remaining = pack::MAX_PACK_SIZE;
        let mut read = |p: &Path| {
            let bytes = read_limited(p, remaining.min(pack::MAX_ENTRY_SIZE))?;
            remaining = remaining.saturating_sub(bytes.len() as u64);
            Ok::<_, String>(bytes)
        };
        let map = self.view_map();
        let faces = map.nodes().filter_map(|(_, n)| n.kind.brush()).flat_map(|b| &b.faces);
        let shaded = map.nodes().flat_map(|(_, n)| n.shader_materials());
        let names: BTreeSet<&str> = faces.flat_map(|f| [Some(f.material.as_str()), f.blend_layer()]).flatten().chain(shaded).collect();
        let mut roots = sdk.user_textures.clone();
        roots.extend(folders(&self.session.view));
        let custom = mapcraft_sdk::Vfs::new(roots);
        let mut entries = Vec::new();
        for name in names {
            if let Some(descriptor) = custom.find(&format!("materials/{name}.json")).or_else(|| custom.find(&format!("{name}.json"))) {
                let bytes = read_limited(&descriptor, mapcraft_format::pack_material::MAX_DESCRIPTOR_SIZE as u64)?;
                let material = mapcraft_format::pack_material::Material::read(&bytes)?;
                for texture in material.textures.values() {
                    let path = custom.find(texture).ok_or_else(|| format!("{name}: missing texture {texture}"))?;
                    if !entries.iter().any(|(entry, _)| entry == texture) {
                        entries.push((texture.clone(), read(&path)?));
                    }
                }
                entries.push((format!("materials/{name}.json"), bytes));
                continue;
            }
            let Some(path) = self.project.find_material(name).and_then(|m| m.image).filter(|p| own(p)) else { continue };
            let bytes = read(&path)?;
            let entry = pack::material_entry(name, &bytes).ok_or_else(|| format!("{name}: SurfsUp maps carry only PNG and JPEG pictures"))?;
            entries.push((entry, bytes));
        }
        let models: BTreeSet<String> = map.nodes().filter_map(|(_, n)| self.project.model_path(n.kind.entity()?)).collect();
        for model in models {
            let Some(path) = self.project.models.vfs.find(&model).filter(|p| own(p)) else { continue };
            let entry = pack::model_entry(&model).ok_or_else(|| format!("{model}: maps carry only .glb and .gltf models"))?;
            if entries.iter().any(|(e, _)| *e == entry) {
                continue;
            }
            let vfs = &self.project.models.vfs;
            let glb = mapcraft_render::to_glb(read(&path)?, &|uri| crate::models::read_capped(vfs, &crate::models::beside(&model, uri)?))
                .map_err(|e| format!("{model}: {e}"))?;
            entries.push((entry, pack::without_morphs(glb)));
        }
        Ok(entries)
    }

    /// Writes `to` as one `.mcpck` holding `map` and its custom assets, or copies `map` there as a `.mcmap` when it has
    /// none, deleting the file of the other kind so a game cannot load a stale copy; returns what it wrote.
    pub(crate) fn pack_map(&self, map: &Path, to: &Path) -> Result<PathBuf, String> {
        let entries = self.custom_asset_entries()?;
        pack_or_copy(map, to, entries)
    }

    pub(crate) fn export_asset_pack(&mut self) {
        let entries = match self.custom_asset_entries() {
            Ok(entries) => entries,
            Err(e) => {
                self.status = e;
                return;
            }
        };
        let name = self.session.path.as_ref().map(|p| p.with_extension("mcpck"));
        let mut dialog = rfd::FileDialog::new().add_filter("MapCraft asset pack", &["mcpck"]);
        if let Some(path) = name {
            dialog = dialog.set_file_name(path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned()));
            if let Some(parent) = path.parent() {
                dialog = dialog.set_directory(parent);
            }
        }
        let Some(path) = dialog.save_file() else { return };
        match pack::save(&path, &entries) {
            Ok(()) => self.status = format!("Exported {} custom assets to {}", entries.len(), path.display()),
            Err(e) => self.status = e.to_string(),
        }
    }
}

fn pack_or_copy(map: &Path, to: &Path, mut entries: Vec<(String, Vec<u8>)>) -> Result<PathBuf, String> {
    let packed = to.with_extension(pack::EXTENSION);
    let plain = to.with_extension(mapcraft_format::EXTENSION);
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let (written, stale) = if entries.is_empty() {
        std::fs::copy(map, &plain).map_err(|e| format!("copy {} to {}: {e}", map.display(), plain.display()))?;
        (plain, packed)
    } else {
        entries.push((pack::MAP_ENTRY.into(), read_limited(map, pack::MAX_ENTRY_SIZE)?));
        pack::save(&packed, &entries).map_err(|e| format!("{}: {e}", packed.display()))?;
        (packed, plain)
    };
    if stale.exists() {
        std::fs::remove_file(&stale).map_err(|e| format!("cannot remove the old {}: {e}", stale.display()))?;
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_lists_merge_and_keep_other_state() {
        let mut view = serde_json::json!({ "cameras": [1], "asset_folders": ["/b"] });
        add(&mut view, FOLDERS, ["/a".to_string(), "/b".to_string()]);
        assert_eq!(folders(&view), [PathBuf::from("/a"), PathBuf::from("/b")]);
        assert_eq!(view["cameras"], serde_json::json!([1]));
        let mut empty = Value::Null;
        add(&mut empty, SKIPPED, ["tools/x".to_string()]);
        assert_eq!(list(&empty, SKIPPED), ["tools/x"]);
    }

    #[test]
    fn play_packs_a_map_with_assets_and_copies_one_without() {
        let dir = tempfile::tempdir().unwrap();
        let map = dir.path().join("surf_a.mcmap");
        let text = br#"{"format":"mapcraft.map","schema_version":1}"#;
        std::fs::write(&map, text).unwrap();
        let to = dir.path().join("Maps/surf_a.mcpck");
        let mut picture = Vec::new();
        image::RgbaImage::new(2, 2).write_to(&mut std::io::Cursor::new(&mut picture), image::ImageFormat::Png).unwrap();

        assert_eq!(pack_or_copy(&map, &to, Vec::new()).unwrap(), dir.path().join("Maps/surf_a.mcmap"));
        let written = pack_or_copy(&map, &to, vec![("materials/a.png".into(), picture)]).unwrap();
        assert_eq!(written, to);
        assert!(!dir.path().join("Maps/surf_a.mcmap").exists(), "the stale plain map goes");
        let entries = pack::load(&to).unwrap();
        assert!(entries.contains(&(pack::MAP_ENTRY.to_string(), text.to_vec())));

        pack_or_copy(&map, &to, Vec::new()).unwrap();
        assert!(!to.exists(), "the stale pack goes");
    }

    #[test]
    fn finds_a_decompilers_folder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("surf_x_d/Materials")).unwrap();
        assert!(holds_assets(&dir.path().join("surf_x_d")));
        assert!(!holds_assets(dir.path()));
    }
}
