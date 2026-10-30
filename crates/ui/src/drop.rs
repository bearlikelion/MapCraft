//! Files dropped on the window: maps open, glTF files and files a plugin imports are added to the map.

use std::path::{Path, PathBuf};

use egui::{Align2, Color32, FontId, Id, LayerId, Order, Stroke, StrokeKind};

use crate::app::MapCraftApp;

impl MapCraftApp {
    /// What dropping `path` does, in words.
    fn drop_verb(&self, path: &Path) -> String {
        let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        if is_map(path) {
            format!("Open {name}")
        } else if is_gltf(path) {
            format!("Import {name} as brushes")
        } else if let Some((_, importer)) = self.importer_for(path) {
            format!("Import {name} as {} into a new layer", importer.label)
        } else {
            format!("MapCraft cannot open {name}: it is not a .mcmap, .glb or .gltf and no loaded plugin imports it")
        }
    }

    /// Opens or imports files dropped this frame and dims the window while files are dragged over it.
    pub(crate) fn handle_drops(&mut self, ctx: &egui::Context) {
        let (hovered, dropped): (Vec<Option<PathBuf>>, Vec<PathBuf>) =
            ctx.input(|i| (i.raw.hovered_files.iter().map(|f| f.path.clone()).collect(), i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect()));
        for path in dropped {
            self.drop_file(path);
        }
        if hovered.is_empty() {
            return;
        }
        let text = hovered.iter().map(|p| p.as_deref().map_or("Drop a map to open it".into(), |p| self.drop_verb(p))).collect::<Vec<_>>().join("\n");
        let painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("file-drop")));
        let rect = ctx.content_rect();
        painter.rect_filled(rect, 0.0, Color32::from_black_alpha(160));
        painter.rect_stroke(rect.shrink(8.0), 8.0, Stroke::new(2.0, crate::chrome::ACCENT), StrokeKind::Inside);
        painter.text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(20.0), crate::chrome::TEXT);
    }

    /// Opens or imports `path` as if it were dropped on the window.
    pub fn drop_file_for_test(&mut self, path: PathBuf) {
        self.drop_file(path);
    }

    fn drop_file(&mut self, path: PathBuf) {
        if is_map(&path) {
            self.open_dialog_path(path);
        } else if is_gltf(&path) {
            self.import_gltf(&path);
        } else if let Some((plugin, importer)) = self.importer_for(&path) {
            self.import_path(&plugin, &importer, &path);
        } else {
            self.status = self.drop_verb(&path);
        }
    }
}

fn is_map(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case(mapcraft_format::EXTENSION))
}

fn is_gltf(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("glb") || e.eq_ignore_ascii_case("gltf"))
}
