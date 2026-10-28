use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use egui::RichText;
use mapcraft_geom::{DMat4, DQuat, DVec3};
use mapcraft_map::NodeId;

use crate::app::MapCraftApp;
use crate::chrome::TEXT_WEAK;
use crate::icons;
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

/// Copies of the selection, each one step on: an offset, then a turn about a pivot.
pub(crate) struct ArrayDialog {
    pub open: bool,
    copies: usize,
    offset: [f64; 3],
    rotation: [f64; 3],
    pivot: [f64; 3],
}

impl Default for ArrayDialog {
    fn default() -> Self {
        Self { open: false, copies: 4, offset: [128.0, 0.0, 0.0], rotation: [0.0; 3], pivot: [0.0; 3] }
    }
}

#[derive(Default)]
pub(crate) struct ReplaceDialog {
    pub open: bool,
    find: String,
    replace: String,
    partial: bool,
    selected_only: bool,
}

/// The F3 popup that runs any action by name.
pub(crate) struct CommandSearch {
    query: String,
    cursor: usize,
    /// Set the frame it opens, so the click that opened it does not close it.
    fresh: bool,
}

impl Default for CommandSearch {
    fn default() -> Self {
        Self { query: String::new(), cursor: 0, fresh: true }
    }
}

/// Lower is better: label prefix, inside the label, inside the menu path, then the query's letters in order; `None` for no match.
fn search_rank(query: &str, label: &str, path: &str) -> Option<u8> {
    let (q, l) = (query.trim().to_lowercase(), label.to_lowercase());
    let mut letters = l.chars();
    Some(if l.starts_with(&q) {
        0
    } else if l.contains(&q) {
        1
    } else if path.to_lowercase().contains(&q) {
        2
    } else if q.chars().filter(|c| !c.is_whitespace()).all(|c| letters.any(|x| x == c)) {
        3
    } else {
        return None;
    })
}

#[derive(Default)]
pub(crate) struct RandomizeDialog {
    pub open: bool,
    ranges: mapcraft_engine::Randomize,
    /// What the dialog last applied and to which selection; changing a range redoes that step.
    applied: Option<(mapcraft_engine::Randomize, BTreeSet<NodeId>)>,
}

#[derive(Default)]
pub(crate) struct Dialogs {
    pub preferences: bool,
    pub shortcuts: bool,
    pub shortcut_filter: String,
    /// A registry URL or path being typed in Preferences.
    pub new_registry: String,
    /// A texture folder being typed in SDK Settings.
    pub new_texture_dir: String,
    pub transform: TransformDialog,
    pub array: ArrayDialog,
    pub replace: ReplaceDialog,
    /// Autosaves left behind by a session that did not close cleanly.
    pub recovery: Vec<PathBuf>,
    /// This editor's running marker, locked while it stays open.
    pub running: Option<std::fs::File>,
    pub switch_sdk: bool,
    pub search: Option<CommandSearch>,
    /// Actions run from the search, newest first, listed first.
    pub recent_commands: Vec<String>,
    pub randomize: RandomizeDialog,
    /// Asking before File > Revert throws away unsaved changes.
    pub revert: bool,
}

/// Prefix of each editor's `running-<pid>` marker, locked while it runs; finding one unlocked means that session did not close cleanly.
const RUNNING: &str = "running";

/// The process an autosave named by `autosave_name` belongs to.
pub(crate) fn autosave_owner(name: &str) -> Option<&str> {
    name.strip_suffix(".autosave.mcmap")?.rsplit_once('-').map(|(_, pid)| pid)
}

/// Autosaves to offer back when an earlier session crashed, leaving out those of editors still running, and this session's marker to keep open.
pub(crate) fn start_session(data: &Path) -> (Vec<PathBuf>, Option<std::fs::File>) {
    let mut live = BTreeSet::new();
    let mut crashed = false;
    for entry in std::fs::read_dir(data).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(pid) = name.strip_prefix(RUNNING) else { continue };
        let held = std::fs::File::open(entry.path()).is_ok_and(|f| matches!(f.try_lock(), Err(std::fs::TryLockError::WouldBlock)));
        if held {
            live.insert(pid.trim_start_matches('-').to_string());
        } else {
            crashed = true;
            let _ = std::fs::remove_file(entry.path());
        }
    }
    let marker = data.join(format!("{RUNNING}-{}", std::process::id()));
    let running = match std::fs::create_dir_all(data).and_then(|()| std::fs::File::options().create(true).append(true).open(&marker)) {
        Ok(f) => {
            if let Err(e) = f.try_lock() {
                log::warn!("{}: {e}", marker.display());
            }
            Some(f)
        }
        Err(e) => {
            log::warn!("{}: {e}", marker.display());
            None
        }
    };
    if !crashed {
        return (Vec::new(), running);
    }
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(data.join("autosave"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == mapcraft_format::EXTENSION))
        .filter(|p| !p.file_name().and_then(|n| n.to_str()).and_then(autosave_owner).is_some_and(|pid| live.contains(pid)))
        .filter_map(|p| Some((p.metadata().ok()?.modified().ok()?, p)))
        .collect();
    found.sort_by_key(|f| std::cmp::Reverse(f.0));
    (found.into_iter().map(|(_, p)| p).collect(), running)
}

pub(crate) fn end_session(data: &Path) {
    let _ = std::fs::remove_file(data.join(format!("{RUNNING}-{}", std::process::id())));
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
        self.array_dialog(ctx);
        self.replace_dialog(ctx);
        self.preferences_dialog(ctx);
        self.shortcuts_dialog(ctx);
        self.recovery_dialog(ctx);
        self.switch_sdk_dialog(ctx);
        self.randomize_dialog(ctx);
        self.revert_dialog(ctx);
        self.command_search(ctx);
    }

    fn command_search(&mut self, ctx: &egui::Context) {
        const ROWS: usize = 14;
        let Some(mut search) = self.dialogs.search.take() else { return };
        let prefs = self.prefs.get().clone();
        let recent = &self.dialogs.recent_commands;
        let mut rows: Vec<(u8, usize, &'static str, &'static str)> = crate::shortcuts::actions()
            .filter_map(|(id, label)| {
                let rank = search_rank(&search.query, label, mapcraft_engine::find(id).map_or("", |c| c.menu))?;
                Some((rank, recent.iter().position(|r| r == id).unwrap_or(usize::MAX), id, label))
            })
            .collect();
        rows.sort_by_key(|r| (r.0, r.1));
        rows.truncate(ROWS);
        let none = egui::Modifiers::NONE;
        let (down, up, enter, escape) = ctx.input_mut(|i| {
            (
                i.consume_key(none, egui::Key::ArrowDown),
                i.consume_key(none, egui::Key::ArrowUp),
                i.consume_key(none, egui::Key::Enter),
                i.consume_key(none, egui::Key::Escape),
            )
        });
        if down {
            search.cursor += 1;
        }
        if up {
            search.cursor = search.cursor.saturating_sub(1);
        }
        search.cursor = search.cursor.min(rows.len().saturating_sub(1));
        let mut run = rows.get(search.cursor).filter(|_| enter).map(|r| r.2);
        let area = egui::Area::new(egui::Id::new("command-search"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 72.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(460.0);
                    let edit = ui.add(egui::TextEdit::singleline(&mut search.query).hint_text("Search commands").desired_width(f32::INFINITY));
                    edit.request_focus();
                    if edit.changed() {
                        search.cursor = 0;
                    }
                    ui.separator();
                    if rows.is_empty() {
                        ui.label(RichText::new("No command matches").color(TEXT_WEAK));
                    }
                    ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                        for (i, (_, _, id, label)) in rows.iter().enumerate() {
                            let spec = mapcraft_engine::find(id);
                            let enabled = spec.is_none_or(|c| (c.enabled)(&self.session));
                            let chord = crate::shortcuts::chord(id, prefs.keymap, &prefs.shortcuts).unwrap_or_default();
                            let row = icons::selectable(i == search.cursor, icons::action_icon(id), *label).shortcut_text(chord);
                            let r = ui.add_enabled(enabled, row);
                            let r = match spec {
                                Some(c) => r.on_hover_text(c.menu.replace('/', " > ")),
                                None => r,
                            };
                            if r.clicked() {
                                run = Some(id);
                            }
                        }
                    });
                });
            });
        let close = escape || (!search.fresh && area.response.clicked_elsewhere());
        search.fresh = false;
        match run {
            Some(id) => {
                self.dialogs.recent_commands.retain(|r| r != id);
                self.dialogs.recent_commands.insert(0, id.to_string());
                self.dialogs.recent_commands.truncate(10);
                self.run_action(id);
            }
            None if !close => self.dialogs.search = Some(search),
            None => {}
        }
    }

    /// Ranges for Randomize Transform; changing one while the dialog is open redoes its last step.
    fn randomize_dialog(&mut self, ctx: &egui::Context) {
        let mut open = self.dialogs.randomize.open;
        if !open {
            return;
        }
        let any = !self.session.selection.is_empty();
        let mut roll = false;
        egui::Window::new("Randomize Transform").open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
            let r = &mut self.dialogs.randomize.ranges;
            egui::Grid::new("randomize-grid").num_columns(2).show(ui, |ui| {
                ui.label("Turn up to");
                crate::inspect::number(ui, egui::DragValue::new(&mut r.yaw).range(0.0..=180.0).suffix("°"));
                ui.end_row();
                ui.label("Tilt up to");
                crate::inspect::number(ui, egui::DragValue::new(&mut r.tilt).range(0.0..=90.0).suffix("°"));
                ui.end_row();
                ui.label("Scale up to");
                let mut percent = r.scale * 100.0;
                if crate::inspect::number(ui, egui::DragValue::new(&mut percent).range(0.0..=90.0).suffix("%")).changed() {
                    r.scale = percent / 100.0;
                }
                ui.end_row();
                ui.label("Seed");
                crate::inspect::number(ui, egui::DragValue::new(&mut r.seed));
                ui.end_row();
            });
            ui.label(RichText::new("Each object turns and scales about its own center by up to these amounts either way.").color(TEXT_WEAK));
            roll = ui.add_enabled(any, icons::button(Some("dice"), "Randomize")).on_hover_text("Click again for another roll").clicked();
        });
        let d = &mut self.dialogs.randomize;
        let moved_on = d.applied.as_ref().is_some_and(|(_, sel)| *sel != self.session.selection);
        if moved_on || !open {
            d.applied = None;
        }
        if roll && d.applied.is_some() {
            d.ranges.seed = d.ranges.seed.wrapping_add(1);
        }
        let redo = d.applied.as_ref().is_some_and(|(r, _)| *r != d.ranges);
        let (ranges, replace) = (d.ranges, d.applied.is_some());
        d.open = open;
        if moved_on || !open {
            self.session.end_coalesce();
        }
        if !(roll || redo) {
            return;
        }
        if replace {
            self.session.cancel_coalesce("randomize");
        }
        match self.session.randomize_selection(&ranges, Some("randomize")) {
            Ok(()) => self.dialogs.randomize.applied = Some((ranges, self.session.selection.clone())),
            Err(e) => self.status = e.to_string(),
        }
        self.mark_changed();
    }

    fn revert_dialog(&mut self, ctx: &egui::Context) {
        if !self.dialogs.revert {
            return;
        }
        let mut choice = None;
        egui::Modal::new(egui::Id::new("revert")).show(ctx, |ui| {
            ui.heading("Revert to the saved map?");
            ui.label("Unsaved changes and the undo history are lost.");
            ui.horizontal(|ui| {
                if ui.button("Revert").clicked() {
                    choice = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(false);
                }
            });
        });
        if let Some(revert) = choice {
            self.dialogs.revert = false;
            if let (true, Some(p)) = (revert, self.session.path.clone()) {
                self.open_now(&p);
            }
        }
    }

    fn switch_sdk_dialog(&mut self, ctx: &egui::Context) {
        let mut open = self.dialogs.switch_sdk;
        let mut picked = false;
        egui::Window::new("Switch SDK").open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
            ui.label(egui::RichText::new("The map keeps its brushes and entities and is saved with the SDK picked.").color(crate::chrome::TEXT_WEAK));
            picked = self.sdk_choices(ui);
        });
        self.dialogs.switch_sdk = open && !picked;
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
            const FIXED: [(&str, &str); 34] = [
                ("Copy, or copy the active face in the Face tool", "Ctrl+C"),
                ("Cut", "Ctrl+X"),
                ("Paste, or paste a face on the picked faces in the Face tool", "Ctrl+V"),
                ("Cancel the current drag or tool step, leave the tool, then deselect", "Escape"),
                ("Apply the clip", "Enter"),
                ("Extrude a face", "Shift+Drag"),
                ("Slide a face, keeping its shape", "Alt+Shift+Drag"),
                ("Extrude into a new brush", "Ctrl+Shift+Drag"),
                ("Stamp a copy along a face", "Ctrl+Alt+Shift+Drag"),
                ("Pick a face's material", "Alt+Click"),
                ("Apply or wrap the picked material", "Alt+Right Click, Alt+Shift+Right Click"),
                ("Fly the 3D camera", "Right Drag + WASD"),
                ("Change the fly speed", "Right Drag + Scroll"),
                ("Lock a UV grab or scale to U or V", "X, Y"),
                ("Snap a UV grab, rotate or scale", "Ctrl"),
                ("Apply a UV grab, rotate or scale", "Click, Enter"),
                ("Cancel a UV grab, rotate or scale", "Escape, Right Click"),
                ("Move the picked faces' UVs", "Drag a picked face in a UV editor"),
                ("Scale UVs; Shift keeps a corner's aspect", "Drag a square handle in a UV editor"),
                ("Rotate UVs", "Drag the round handle in a UV editor"),
                ("Change a bevel's segments while sizing it", "Scroll"),
                ("Apply a bevel being sized", "Click, Enter"),
                ("Cancel a bevel being sized", "Escape, Right Click"),
                ("Move a 3D vertex handle or block height up and down", "Shift+Drag, Alt+Drag"),
                ("Add a third clip point that tilts the cut", "Click in another view"),
                ("Clip along a face", "Double-click the face in the 3D view"),
                ("Move a clip point", "Drag the point"),
                ("Move a brush entity's origin", "Drag its handle in a 2D view"),
                ("Nudge the selection one grid step, or the picked handles in the Vertex tool", "Arrow keys"),
                ("Nudge the selection one unit", "Shift+Arrow keys"),
                ("Nudge a copy of the selection, leaving the original", "Ctrl+Arrow keys"),
                ("Move a copy of the selection, leaving the original", "Ctrl+Drag"),
                ("Pick a face from the Select or Move tool", "Shift+Click in the 3D view"),
                ("Orbit the 3D camera around the point under the pointer", "Alt+Right Drag"),
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
                for (m, label, icon) in
                    [(TransformMode::Move, "Move", "arrow_cross"), (TransformMode::Rotate, "Rotate", "rotate"), (TransformMode::Scale, "Scale", "resize")]
                {
                    if ui.add(crate::icons::selectable(d.mode == m, Some(icon), label)).clicked() && d.mode != m {
                        d.mode = m;
                        d.values = if m == TransformMode::Scale { [1.0; 3] } else { [0.0; 3] };
                    }
                }
            });
            ui.horizontal(|ui| {
                for (v, axis) in d.values.iter_mut().zip(["X", "Y", "Z"]) {
                    ui.label(axis);
                    crate::inspect::number(ui, egui::DragValue::new(v).speed(if d.mode == TransformMode::Scale { 0.01 } else { 1.0 }));
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

    pub(crate) fn open_array(&mut self) {
        self.dialogs.array.open = true;
        if let Some(c) = self.session.selection_center() {
            self.dialogs.array.pivot = c.to_array();
        }
    }

    fn array_dialog(&mut self, ctx: &egui::Context) {
        let mut open = self.dialogs.array.open;
        let mut apply = false;
        let center = self.session.selection_center();
        egui::Window::new("Array").open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
            let d = &mut self.dialogs.array;
            egui::Grid::new("array-grid").num_columns(4).show(ui, |ui| {
                ui.label("Copies");
                crate::inspect::number(ui, egui::DragValue::new(&mut d.copies).range(1..=256));
                ui.end_row();
                for (label, values, speed) in [("Offset", &mut d.offset, 1.0), ("Turn", &mut d.rotation, 1.0), ("Pivot", &mut d.pivot, 1.0)] {
                    ui.label(label);
                    for (v, axis) in values.iter_mut().zip(["X", "Y", "Z"]) {
                        crate::inspect::number(ui, egui::DragValue::new(v).speed(speed).prefix(format!("{axis} ")));
                    }
                    ui.end_row();
                }
            });
            ui.label("Each copy moves by the offset and turns by the degrees about the pivot, from the copy before it.");
            ui.horizontal(|ui| {
                if ui.add_enabled(center.is_some(), egui::Button::new("Pivot at selection")).clicked()
                    && let Some(c) = center
                {
                    d.pivot = c.to_array();
                }
                if ui.button("Pivot at origin").clicked() {
                    d.pivot = [0.0; 3];
                }
            });
            apply = ui.add_enabled(center.is_some(), egui::Button::new("Make copies")).clicked();
        });
        self.dialogs.array.open = open;
        if !apply {
            return;
        }
        let d = &self.dialogs.array;
        let (v, p) = (DVec3::from_array(d.rotation), DVec3::from_array(d.pivot));
        let turn = DQuat::from_rotation_z(v.z.to_radians()) * DQuat::from_rotation_y(v.y.to_radians()) * DQuat::from_rotation_x(v.x.to_radians());
        let step = DMat4::from_translation(DVec3::from_array(d.offset) + p) * DMat4::from_quat(turn) * DMat4::from_translation(-p);
        if let Err(e) = self.session.array_selection(d.copies, &step) {
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
                crate::inspect::number(ui, egui::DragValue::new(&mut prefs.view.grid).range(0.125..=4096.0));
                ui.end_row();
                ui.label("Autosave every");
                crate::inspect::number(ui, egui::DragValue::new(&mut prefs.view.autosave_minutes).range(0..=120).suffix(" min"));
                ui.end_row();
                ui.label("Keep backups");
                crate::inspect::number(ui, egui::DragValue::new(&mut prefs.view.backups).range(0..=20))
                    .on_hover_text("Earlier versions kept in a backups folder beside the map each time it is saved");
                ui.end_row();
                ui.label("Color by Slope");
                ui.horizontal(|ui| {
                    crate::inspect::number(
                        ui,
                        egui::DragValue::new(&mut prefs.view.slope_walkable).range(0.0..=90.0).speed(0.1).prefix("walkable to ").suffix("°"),
                    );
                    crate::inspect::number(
                        ui,
                        egui::DragValue::new(&mut prefs.view.slope_surfable).range(0.0..=90.0).speed(0.1).prefix("surfable to ").suffix("°"),
                    );
                });
                ui.end_row();
                ui.label("Fly speed");
                crate::inspect::number(ui, egui::DragValue::new(&mut prefs.view.fly_speed).range(16.0..=8192.0).suffix(" units/s"));
                ui.end_row();
                ui.label("Materials");
                ui.checkbox(&mut prefs.view.apply_on_click, "Clicking a material applies it to the selection");
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
                    if crate::chrome::icon_button(ui, "trash_can", "Remove", 20.0, false).on_hover_text("Remove this folder").clicked() {
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
                    if crate::chrome::icon_button(ui, "trash_can", "Remove", 20.0, false).on_hover_text("Remove this registry").clicked() {
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
                            let reset =
                                ui.add_enabled_ui(prefs.shortcuts.contains_key(id), |ui| crate::chrome::icon_button(ui, "recover", "Reset", 20.0, false)).inner;
                            if reset.on_hover_text("Back to the preset's chord").clicked() {
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
            self.mark_changed();
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
            self.recovered = Some(p);
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
    fn command_search_ranks_prefixes_then_words_then_paths_then_letters() {
        assert_eq!(search_rank("", "Undo", "Edit/Undo"), Some(0));
        assert_eq!(search_rank("dup", "Duplicate", "Edit/Duplicate"), Some(0));
        assert_eq!(search_rank("linked", "Duplicate Linked", ""), Some(1));
        assert_eq!(search_rank("curves", "Spin", "Tools/Curves/Spin"), Some(2));
        assert_eq!(search_rank("dtf", "Drop to Floor", ""), Some(3));
        assert_eq!(search_rank("zzz", "Drop to Floor", "Tools/Drop to Floor"), None);
    }

    #[test]
    fn crash_leaves_autosaves_to_recover() {
        let dir = tempfile::tempdir().unwrap();
        assert!(start_session(dir.path()).0.is_empty());
        std::fs::create_dir_all(dir.path().join("autosave")).unwrap();
        std::fs::write(dir.path().join("autosave/level.autosave.mcmap"), "{}").unwrap();
        assert_eq!(start_session(dir.path()).0.len(), 1, "the marker was left unlocked, so the last run crashed");
        end_session(dir.path());
        assert!(start_session(dir.path()).0.is_empty());
    }

    #[test]
    fn a_running_editor_keeps_its_autosaves() {
        let dir = tempfile::tempdir().unwrap();
        let (_, running) = start_session(dir.path());
        let autosave = dir.path().join("autosave");
        std::fs::create_dir_all(&autosave).unwrap();
        std::fs::write(autosave.join(format!("untitled-{}.autosave.mcmap", std::process::id())), "{}").unwrap();
        std::fs::write(dir.path().join("running-4000000000"), "").unwrap();
        std::fs::write(autosave.join("level-00ff-4000000000.autosave.mcmap"), "{}").unwrap();
        assert_eq!(start_session(dir.path()).0, vec![autosave.join("level-00ff-4000000000.autosave.mcmap")], "only the crashed editor's");
        drop(running);
    }
}
