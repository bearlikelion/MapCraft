//! A map's own asset folders, such as the one BSPSource writes beside a decompiled map, and the
//! window that asks for the textures and models a map uses that no folder holds.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use egui::RichText;
use serde_json::Value;

use crate::app::MapCraftApp;

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
    fn add_asset_folder(&mut self, dir: PathBuf) {
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
    fn finds_a_decompilers_folder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("surf_x_d/Materials")).unwrap();
        assert!(holds_assets(&dir.path().join("surf_x_d")));
        assert!(!holds_assets(dir.path()));
    }
}
