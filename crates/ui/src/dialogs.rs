use std::path::{Path, PathBuf};

use mapcraft_geom::{DMat4, DQuat, DVec3};

use crate::app::MapCraftApp;
use crate::shortcuts::Preset;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum TransformMode {
    #[default]
    Move,
    Rotate,
    Scale,
}

#[derive(Default)]
pub(crate) struct TransformDialog {
    pub open: bool,
    mode: TransformMode,
    values: [f64; 3],
}

#[derive(Default)]
pub(crate) struct ReplaceDialog {
    pub open: bool,
    find: String,
    replace: String,
    partial: bool,
    selected_only: bool,
}

#[derive(Default)]
pub(crate) struct Dialogs {
    pub preferences: bool,
    pub shortcuts: bool,
    pub shortcut_filter: String,
    /// A registry URL or path being typed in Preferences.
    pub new_registry: String,
    pub transform: TransformDialog,
    pub replace: ReplaceDialog,
    /// Autosaves left behind by a session that did not close cleanly.
    pub recovery: Vec<PathBuf>,
}

/// Marks a running editor; finding it at startup means the last session did not close cleanly.
const RUNNING: &str = "running";

/// Autosaves to offer back when the previous session crashed, and marks this one as running.
pub(crate) fn start_session(data: &Path) -> Vec<PathBuf> {
    let marker = data.join(RUNNING);
    let crashed = marker.exists();
    if let Err(e) = std::fs::create_dir_all(data).and_then(|()| std::fs::write(&marker, std::process::id().to_string())) {
        log::warn!("{}: {e}", marker.display());
    }
    if !crashed {
        return Vec::new();
    }
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(data.join("autosave"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == mapcraft_format::EXTENSION))
        .filter_map(|p| Some((p.metadata().ok()?.modified().ok()?, p)))
        .collect();
    found.sort_by_key(|f| std::cmp::Reverse(f.0));
    found.into_iter().map(|(_, p)| p).collect()
}

pub(crate) fn end_session(data: &Path) {
    let _ = std::fs::remove_file(data.join(RUNNING));
}

impl MapCraftApp {
    pub(crate) fn open_replace_materials(&mut self) {
        let d = &mut self.dialogs.replace;
        d.open = true;
        d.selected_only = !self.session.selection.is_empty();
        d.replace = self.session.tex.material.clone();
        if let Some(m) =
            self.session.selected_brushes().first().and_then(|id| self.session.map.get(*id)?.kind.brush()?.faces.first().map(|f| f.material.clone()))
        {
            d.find = m;
        }
    }

    pub(crate) fn extra_dialogs(&mut self, ctx: &egui::Context) {
        self.transform_dialog(ctx);
        self.replace_dialog(ctx);
        self.preferences_dialog(ctx);
        self.shortcuts_dialog(ctx);
        self.recovery_dialog(ctx);
    }

    fn shortcuts_dialog(&mut self, ctx: &egui::Context) {
        let mut open = self.dialogs.shortcuts;
        let prefs = self.prefs.get().clone();
        let mut edit = false;
        egui::Window::new("Keyboard Shortcuts").open(&mut open).default_width(420.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.dialogs.shortcut_filter).hint_text("Search").desired_width(220.0));
                edit = ui.button("Edit in Preferences").clicked();
            });
            let needle = self.dialogs.shortcut_filter.to_lowercase();
            const FIXED: [(&str, &str); 11] = [
                ("Copy", "Ctrl+C"),
                ("Cut", "Ctrl+X"),
                ("Paste", "Ctrl+V"),
                ("Cancel the current drag or tool step", "Escape"),
                ("Apply the clip", "Enter"),
                ("Extrude a face", "Shift+Drag"),
                ("Extrude into a new brush", "Ctrl+Shift+Drag"),
                ("Stamp a copy along a face", "Ctrl+Alt+Shift+Drag"),
                ("Pick a face's material", "Alt+Click"),
                ("Apply or wrap the picked material", "Alt+Right Click, Alt+Shift+Right Click"),
                ("Fly the 3D camera", "Right Drag + WASD"),
            ];
            let bound = crate::shortcuts::actions().filter_map(|(id, label)| Some((label, crate::shortcuts::chord(id, prefs.keymap, &prefs.shortcuts)?)));
            let rows: Vec<(&str, String)> = bound
                .chain(FIXED.iter().map(|(l, c)| (*l, c.to_string())))
                .filter(|(l, c)| needle.is_empty() || l.to_lowercase().contains(&needle) || c.to_lowercase().contains(&needle))
                .collect();
            egui::ScrollArea::vertical().max_height(480.0).show(ui, |ui| {
                egui::Grid::new("shortcut-list").num_columns(2).striped(true).show(ui, |ui| {
                    for (label, chord) in rows {
                        ui.label(label);
                        ui.monospace(chord);
                        ui.end_row();
                    }
                });
            });
        });
        self.dialogs.shortcuts = open && !edit;
        self.dialogs.preferences |= edit;
    }

    fn transform_dialog(&mut self, ctx: &egui::Context) {
        let mut open = self.dialogs.transform.open;
        let mut apply = false;
        egui::Window::new("Transform").open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
            let d = &mut self.dialogs.transform;
            ui.horizontal(|ui| {
                for (m, label) in [(TransformMode::Move, "Move"), (TransformMode::Rotate, "Rotate"), (TransformMode::Scale, "Scale")] {
                    if ui.selectable_label(d.mode == m, label).clicked() && d.mode != m {
                        d.mode = m;
                        d.values = if m == TransformMode::Scale { [1.0; 3] } else { [0.0; 3] };
                    }
                }
            });
            ui.horizontal(|ui| {
                for (v, axis) in d.values.iter_mut().zip(["X", "Y", "Z"]) {
                    ui.label(axis);
                    ui.add(egui::DragValue::new(v).speed(if d.mode == TransformMode::Scale { 0.01 } else { 1.0 }));
                }
            });
            ui.label(match d.mode {
                TransformMode::Move => "Units along each axis.",
                TransformMode::Rotate => "Degrees about each axis, through the selection's center.",
                TransformMode::Scale => "Factors along each axis, from the selection's center.",
            });
            apply = ui.add_enabled(!self.session.selection.is_empty(), egui::Button::new("Apply")).clicked();
        });
        self.dialogs.transform.open = open;
        if !apply {
            return;
        }
        let d = &self.dialogs.transform;
        let v = DVec3::from_array(d.values);
        let c = self.session.selection_center().unwrap_or_default();
        let about = |m: DMat4| DMat4::from_translation(c) * m * DMat4::from_translation(-c);
        let (m, label) = match d.mode {
            TransformMode::Move => (DMat4::from_translation(v), "Move"),
            TransformMode::Rotate => {
                let r = DQuat::from_rotation_z(v.z.to_radians()) * DQuat::from_rotation_y(v.y.to_radians()) * DQuat::from_rotation_x(v.x.to_radians());
                (about(DMat4::from_quat(r)), "Rotate")
            }
            TransformMode::Scale => (about(DMat4::from_scale(v)), "Scale"),
        };
        if let Err(e) = self.session.transform_selection(&m, label, None) {
            self.status = e.to_string();
        }
        self.mark_changed();
    }

    fn replace_dialog(&mut self, ctx: &egui::Context) {
        let mut open = self.dialogs.replace.open;
        let mut run = false;
        egui::Window::new("Replace Materials").open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
            let d = &mut self.dialogs.replace;
            egui::Grid::new("replace-grid").num_columns(2).show(ui, |ui| {
                ui.label("Find");
                ui.text_edit_singleline(&mut d.find);
                ui.end_row();
                ui.label("Replace with");
                ui.text_edit_singleline(&mut d.replace);
                ui.end_row();
            });
            ui.checkbox(&mut d.partial, "Replace text inside names");
            ui.horizontal(|ui| {
                ui.radio_value(&mut d.selected_only, true, "Selection");
                ui.radio_value(&mut d.selected_only, false, "Whole map");
            });
            run = ui.add_enabled(!d.find.is_empty(), egui::Button::new("Replace")).clicked();
        });
        self.dialogs.replace.open = open;
        if run {
            let d = &self.dialogs.replace;
            self.status = match self.session.replace_materials(&d.find, &d.replace, d.partial, d.selected_only) {
                Ok(n) => format!("Replaced the material on {n} faces"),
                Err(e) => e.to_string(),
            };
            self.mark_changed();
        }
    }

    fn preferences_dialog(&mut self, ctx: &egui::Context) {
        let mut open = self.dialogs.preferences;
        let mut prefs = self.prefs.get().clone();
        let mut rescan = false;
        egui::Window::new("Preferences").open(&mut open).default_width(460.0).show(ctx, |ui| {
            egui::Grid::new("prefs-grid").num_columns(2).show(ui, |ui| {
                ui.label("Grid size");
                ui.add(egui::DragValue::new(&mut prefs.view.grid).range(0.125..=4096.0));
                ui.end_row();
                ui.label("Autosave every");
                ui.add(egui::DragValue::new(&mut prefs.view.autosave_minutes).range(0..=120).suffix(" min"));
                ui.end_row();
                ui.label("Fly speed");
                ui.add(egui::DragValue::new(&mut prefs.view.fly_speed).range(16.0..=8192.0).suffix(" units/s"));
                ui.end_row();
                ui.label("Shortcuts");
                egui::ComboBox::from_id_salt("keymap").selected_text(prefs.keymap.label()).show_ui(ui, |ui| {
                    for p in Preset::ALL {
                        ui.selectable_value(&mut prefs.keymap, p, p.label());
                    }
                });
                ui.end_row();
            });
            ui.separator();
            ui.label("Extra SDK folders");
            let mut remove = None;
            for (i, d) in prefs.sdk_dirs.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.monospace(d);
                    if ui.small_button("Remove").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                prefs.sdk_dirs.remove(i);
                rescan = true;
            }
            if ui.button("Add folder…").clicked()
                && let Some(p) = rfd::FileDialog::new().pick_folder()
            {
                prefs.sdk_dirs.push(p.to_string_lossy().into_owned());
                rescan = true;
            }
            ui.separator();
            ui.label("SDK registries");
            let mut remove = None;
            for (i, r) in prefs.registries.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.monospace(r);
                    if ui.small_button("Remove").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                prefs.registries.remove(i);
            }
            ui.horizontal(|ui| {
                let typed = &mut self.dialogs.new_registry;
                ui.add(egui::TextEdit::singleline(typed).hint_text("https://… or a path to an index.json").desired_width(300.0));
                if ui.add_enabled(!typed.trim().is_empty(), egui::Button::new("Add")).clicked() {
                    prefs.registries.push(typed.trim().to_string());
                    typed.clear();
                }
                let default = mapcraft_sdk_manager::DEFAULT_REGISTRY;
                if !prefs.registries.iter().any(|r| r == default) && ui.button("Add the default").clicked() {
                    prefs.registries.insert(0, default.to_string());
                }
            });
            ui.separator();
            egui::CollapsingHeader::new("Key bindings").show(ui, |ui| {
                ui.label("Type a chord such as Ctrl+Shift+K; clear it to unbind.");
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    egui::Grid::new("keys").num_columns(3).striped(true).show(ui, |ui| {
                        for (id, label) in crate::shortcuts::actions() {
                            ui.label(label);
                            let mut text = crate::shortcuts::chord(id, prefs.keymap, &prefs.shortcuts).unwrap_or_default();
                            let valid = text.is_empty() || crate::shortcuts::parse(&text).is_some();
                            let edit = egui::TextEdit::singleline(&mut text).desired_width(140.0);
                            let r = ui.add(if valid { edit } else { edit.text_color(egui::Color32::LIGHT_RED) });
                            if r.changed() {
                                prefs.shortcuts.insert(id.to_string(), text);
                            }
                            if ui.add_enabled(prefs.shortcuts.contains_key(id), egui::Button::new("Reset").small()).clicked() {
                                prefs.shortcuts.remove(id);
                            }
                            ui.end_row();
                        }
                    });
                });
            });
        });
        self.dialogs.preferences = open;
        if &prefs != self.prefs.get() {
            if prefs.registries != self.prefs.get().registries {
                self.sdk_manager.registries_changed();
            }
            self.session.grid = prefs.view.grid;
            self.prefs.update(|p| *p = prefs);
        }
        if rescan {
            self.rescan_sdks();
        }
    }

    fn recovery_dialog(&mut self, ctx: &egui::Context) {
        if self.dialogs.recovery.is_empty() {
            return;
        }
        let mut open = None;
        let mut discard = false;
        let mut later = false;
        egui::Modal::new(egui::Id::new("recover")).show(ctx, |ui| {
            ui.heading("Recover unsaved work?");
            ui.label("MapCraft did not close cleanly last time. These autosaves were left behind:");
            for p in &self.dialogs.recovery {
                ui.horizontal(|ui| {
                    ui.monospace(p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                    if ui.button("Open").clicked() {
                        open = Some(p.clone());
                    }
                });
            }
            ui.horizontal(|ui| {
                discard = ui.button("Delete autosaves").clicked();
                later = ui.button("Not now").clicked();
            });
        });
        if let Some(p) = open {
            self.open_now(&p);
            // A recovered map is unsaved work, not the autosave file itself.
            self.session.path = None;
            self.session.mark_unsaved();
            self.dialogs.recovery.clear();
        } else if discard {
            for p in self.dialogs.recovery.drain(..) {
                let _ = std::fs::remove_file(p);
            }
        } else if later {
            self.dialogs.recovery.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crash_leaves_autosaves_to_recover() {
        let dir = tempfile::tempdir().unwrap();
        assert!(start_session(dir.path()).is_empty());
        std::fs::create_dir_all(dir.path().join("autosave")).unwrap();
        std::fs::write(dir.path().join("autosave/level.autosave.mcmap"), "{}").unwrap();
        assert_eq!(start_session(dir.path()).len(), 1, "the marker survived, so the last run crashed");
        end_session(dir.path());
        assert!(start_session(dir.path()).is_empty());
    }
}
