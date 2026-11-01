//! Shader workspaces: Godot shaders embedded in map nodes, shader graphs and material descriptors.

use std::collections::BTreeMap;
use std::path::PathBuf;

use mapcraft_engine::Session;
use mapcraft_map::{Map, NodeId, NodeKind};

fn bounded_text(path: &std::path::Path, limit: u64) -> Result<String, std::io::Error> {
    use std::io::Read;
    let mut text = String::new();
    std::fs::File::open(path)?.take(limit.saturating_add(1)).read_to_string(&mut text)?;
    if text.len() as u64 > limit {
        return Err(std::io::Error::other("Shader document exceeds 256 KiB"));
    }
    Ok(text)
}

/// The Shaders list's name for a node's shader.
pub(crate) fn shader_name(map: &Map, id: NodeId) -> String {
    let entity = map.get(id).and_then(|n| n.parent).and_then(|p| map.get(p)).filter(|p| matches!(p.kind, NodeKind::Entity(_)));
    match (map.get(id).map(|n| &n.kind), entity) {
        (Some(NodeKind::World(_)), _) => "World (sky)".into(),
        (Some(NodeKind::Brush(_)), Some(e)) => format!("Brush #{} ({})", id.0, e.kind.label()),
        (Some(k), _) => format!("{} #{}", k.label(), id.0),
        (None, _) => format!("#{}", id.0),
    }
}

/// What a shader workspace asks of the editor after drawing.
#[derive(Default)]
pub(crate) struct Outcome {
    /// The map or selection changed.
    pub changed: bool,
    /// An action to run, such as Undo.
    pub run: Option<&'static str>,
}

#[derive(Default)]
pub(crate) struct ShaderEditor {
    pub(crate) open: bool,
    viewport: bool,
    cursor_position: [usize; 2],
    /// The node whose embedded shader the Script tab edits.
    pub(crate) target: Option<NodeId>,
    filter: String,
    /// Where the last shader was imported from or exported to.
    path: Option<PathBuf>,
    graph_path: Option<PathBuf>,
    material_path: Option<PathBuf>,
    /// The target's source as the text editor holds it.
    source: String,
    /// Graph output waiting for a shader to go into.
    generated: Option<String>,
    status: String,
    graph: mapcraft_format::shader_graph::Graph,
    graph_mode: bool,
    material_mode: bool,
    material: mapcraft_format::pack_material::Material,
    saved_graph: String,
    canvas: crate::shader_canvas::Canvas,
    graph_history: Vec<mapcraft_format::shader_graph::Graph>,
    graph_future: Vec<mapcraft_format::shader_graph::Graph>,
    graph_pending: Option<mapcraft_format::shader_graph::Graph>,
    graph_skip_history: bool,
    pub(crate) reload_assets: bool,
    saved_material: Option<mapcraft_format::pack_material::Material>,
}

impl ShaderEditor {
    fn graph_dirty(&self) -> bool {
        if self.saved_graph.is_empty() {
            self.graph != mapcraft_format::shader_graph::Graph::default()
        } else {
            serde_json::to_string_pretty(&self.graph).map_or(true, |text| text != self.saved_graph)
        }
    }

    fn material_dirty(&self) -> bool {
        self.saved_material.as_ref().map_or_else(|| self.material != Default::default(), |saved| &self.material != saved)
    }

    /// Graph or material documents not saved; embedded shaders are the map's to save.
    pub(crate) fn has_unsaved(&self) -> bool {
        self.graph_dirty() || self.material_dirty()
    }

    pub(crate) fn save_all(&mut self) -> bool {
        let modes = (self.graph_mode, self.material_mode);
        if self.graph_dirty() {
            self.graph_mode = true;
            self.material_mode = false;
            self.save_active(false);
            if self.graph_dirty() {
                self.graph_mode = modes.0;
                self.material_mode = modes.1;
                return false;
            }
        }
        if self.material_dirty() {
            self.material_mode = true;
            self.save_active(false);
        }
        self.graph_mode = modes.0;
        self.material_mode = modes.1;
        !self.has_unsaved()
    }

    pub(crate) fn is_graph(&self) -> bool {
        self.is_active() && self.graph_mode && !self.material_mode
    }

    /// Runs `id` for the active graph or material; Script leaves saving and undo to the map, which holds its shaders.
    pub(crate) fn action(&mut self, id: &str) -> bool {
        if !self.is_active() || (!self.graph_mode && !self.material_mode) {
            return false;
        }
        match id {
            "file.save" | "file.save_as" => self.save_active(id == "file.save_as"),
            "edit.undo" if self.is_graph() => {
                if let Some(previous) = self.graph_history.pop() {
                    self.graph_future.push(std::mem::replace(&mut self.graph, previous));
                    self.graph_skip_history = true;
                    self.canvas = Default::default();
                }
            }
            "edit.redo" if self.is_graph() => {
                if let Some(next) = self.graph_future.pop() {
                    self.graph_history.push(std::mem::replace(&mut self.graph, next));
                    self.graph_skip_history = true;
                    self.canvas = Default::default();
                }
            }
            "edit.undo" | "edit.redo" => self.status = "Material edits have no undo yet.".into(),
            _ => return false,
        }
        true
    }

    fn save_active(&mut self, save_as: bool) {
        let (extension, current, bytes) = if self.material_mode {
            ("json", self.material_path.clone(), self.material.validate().and_then(|()| serde_json::to_vec_pretty(&self.material).map_err(|e| e.to_string())))
        } else {
            (
                "mcshader",
                self.graph_path.clone(),
                self.graph.source().map_err(|e| e.to_string()).and_then(|_| serde_json::to_vec_pretty(&self.graph).map_err(|e| e.to_string())),
            )
        };
        let bytes = match bytes {
            Ok(bytes) if bytes.len() <= 256 * 1024 => bytes,
            Ok(_) => {
                self.status = "Shader document exceeds 256 KiB".into();
                return;
            }
            Err(e) => {
                self.status = e;
                return;
            }
        };
        let path = if !save_as && current.is_some() { current } else { save_dialog(extension, current.as_ref()) };
        let Some(path) = path else { return };
        if let Err(e) = craft_atomic::atomic_write(&path, &bytes) {
            self.status = e.to_string();
            return;
        }
        self.status = format!("Saved {}", path.display());
        if self.material_mode {
            self.material_path = Some(path);
            self.saved_material = Some(self.material.clone());
            self.reload_assets = true;
        } else {
            self.graph_path = Some(path);
            self.saved_graph = String::from_utf8_lossy(&bytes).into_owned();
        }
    }

    pub(crate) fn open_graph(&mut self) {
        self.open = true;
        self.viewport = false;
        self.graph_mode = true;
        self.material_mode = false;
    }

    pub(crate) fn open_script(&mut self) {
        self.open_graph();
        self.graph_mode = false;
    }

    /// Opens the Script tab on `id`'s embedded shader.
    pub(crate) fn edit(&mut self, id: NodeId) {
        self.target = Some(id);
        self.open_script();
    }

    pub(crate) fn is_active(&self) -> bool {
        self.open && !self.viewport
    }

    /// Returns whether an editor occupies the central workspace.
    pub(crate) fn workspace_tabs(&mut self, ui: &mut egui::Ui) -> bool {
        ui.horizontal_wrapped(|ui| {
            if ui.selectable_label(!self.open || self.viewport, "Viewport").clicked() {
                self.viewport = true;
            }
            if self.open {
                if ui.selectable_label(!self.viewport && self.material_mode, "Material").clicked() {
                    self.viewport = false;
                    self.material_mode = true;
                }
                if ui.selectable_label(!self.viewport && !self.material_mode && self.graph_mode, "Shader Graph").clicked() {
                    self.open_graph();
                }
                if ui.selectable_label(!self.viewport && !self.material_mode && !self.graph_mode, "Script").clicked() {
                    self.open_script();
                }
            }
        });
        self.is_active()
    }

    /// The target when it still carries a shader, else the first selected node that does.
    fn live_target(&mut self, session: &Session) -> Option<NodeId> {
        let shaded = |id: &NodeId| session.map.get(*id).is_some_and(|n| n.shader.is_some());
        if !self.target.as_ref().is_some_and(shaded) {
            self.target = session.selection.iter().copied().find(|id| shaded(id));
        }
        self.target
    }

    /// The Shaders list: every node with an embedded shader, filtered, with errors in red.
    pub(crate) fn shader_list(&mut self, ui: &mut egui::Ui, session: &mut Session, errors: &BTreeMap<NodeId, String>) -> Outcome {
        const ROW: f32 = 20.0;
        let mut out = Outcome::default();
        let target = self.live_target(session);
        ui.horizontal(|ui| {
            ui.strong("Shaders");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let tip = "Add a surface shader to the selected brushes, or a sky shader to the World with no brush selected";
                if ui.add(crate::icons::button(Some("plus"), "Add shader")).on_hover_text(tip).clicked() {
                    out.run = Some("shader.add");
                }
            });
        });
        crate::panels::search_field(ui, &mut self.filter, "Filter shaders");
        let query = self.filter.trim().to_lowercase();
        let entries: Vec<(NodeId, String)> = session
            .map
            .nodes()
            .filter(|(_, n)| n.shader.is_some())
            .map(|(id, _)| (id, shader_name(&session.map, id)))
            .filter(|(_, name)| query.is_empty() || name.to_lowercase().contains(&query))
            .collect();
        if entries.is_empty() {
            ui.colored_label(
                crate::chrome::TEXT_WEAK,
                if query.is_empty() { "No shaders yet. Select brushes and add one, or add a sky shader with nothing selected." } else { "No shader matches" },
            );
        }
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::vertical().id_salt("shader-list").auto_shrink(false).show(ui, |ui| {
            for (id, name) in entries {
                let error = errors.get(&id);
                let active = target == Some(id);
                let (row, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW), egui::Sense::click());
                resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, active, name.clone()));
                let painter = ui.painter_at(row);
                if active {
                    painter.rect_filled(row, 3.0, crate::chrome::ACCENT_DIM);
                } else if resp.hovered() {
                    painter.rect_filled(row, 3.0, crate::chrome::WIDGET);
                }
                let icon_rect = egui::Rect::from_min_size(egui::pos2(row.left() + 4.0, row.top()), egui::vec2(16.0, ROW));
                let (icon, tint) = match (error, id == mapcraft_map::ROOT) {
                    (Some(_), _) => ("danger", egui::Color32::LIGHT_RED),
                    (None, true) => ("sky_dome", crate::chrome::TEXT),
                    (None, false) => ("wave", crate::chrome::TEXT),
                };
                crate::icons::paint(ui, icon, icon_rect, crate::icons::SIZE, tint);
                let color = if error.is_some() { egui::Color32::LIGHT_RED } else { crate::chrome::TEXT };
                let font = egui::TextStyle::Body.resolve(ui.style());
                painter.text(egui::pos2(icon_rect.right() + 6.0, row.center().y), egui::Align2::LEFT_CENTER, &name, font, color);
                let resp = match error {
                    Some(e) => resp.on_hover_text(e),
                    None => resp,
                };
                if resp.clicked() {
                    session.selection = [id].into();
                    self.edit(id);
                    out.changed = true;
                }
            }
        });
        out
    }

    pub(crate) fn content(&mut self, ui: &mut egui::Ui, session: &mut Session, errors: &BTreeMap<NodeId, String>) -> Outcome {
        if self.material_mode {
            self.shader_material_menu(ui);
            egui::ScrollArea::vertical().id_salt("material-editor").show(ui, |ui| self.material_ui(ui));
            return Outcome::default();
        }
        if self.graph_mode {
            self.graph_ui(ui);
            return Outcome::default();
        }
        self.script_ui(ui, session, errors)
    }

    fn script_ui(&mut self, ui: &mut egui::Ui, session: &mut Session, errors: &BTreeMap<NodeId, String>) -> Outcome {
        let mut out = Outcome::default();
        let Some(id) = self.live_target(session) else {
            ui.add_space(8.0);
            ui.label("Pick a shader in the Shaders list, or add one: select brushes for a surface shader, or nothing for a sky shader on the World.");
            if self.generated.is_some() {
                ui.colored_label(crate::chrome::TEXT_WEAK, "The generated graph source goes into the next shader you pick.");
            }
            if ui.add(crate::icons::button(Some("plus"), "Add shader")).clicked() {
                out.run = Some("shader.add");
            }
            return out;
        };
        let write = |session: &mut Session, source: Option<String>, step: bool, status: &mut String| {
            if step {
                session.end_coalesce();
            }
            match session.set_shader(id, source) {
                Ok(()) => status.clear(),
                Err(e) => *status = e.to_string(),
            }
            if step {
                session.end_coalesce();
            }
        };
        if let Some(source) = self.generated.take() {
            write(session, Some(source), true, &mut self.status);
            out.changed = true;
        }
        let stored = session.map.get(id).and_then(|n| n.shader.as_deref()).unwrap_or_default();
        if self.source != stored {
            stored.clone_into(&mut self.source);
        }
        ui.horizontal(|ui| {
            if ui.add_enabled(session.can_undo(), egui::Button::new("Undo")).on_hover_text(session.undo_label().unwrap_or_default()).clicked() {
                out.run = Some("edit.undo");
            }
            if ui.add_enabled(session.can_redo(), egui::Button::new("Redo")).on_hover_text(session.redo_label().unwrap_or_default()).clicked() {
                out.run = Some("edit.redo");
            }
            if ui.button("Import .gdshader…").on_hover_text("Replace this shader with a .gdshader file's source").clicked()
                && let Some(path) = rfd::FileDialog::new().add_filter("Godot shader", &["gdshader"]).pick_file()
            {
                match bounded_text(&path, mapcraft_map::MAX_SHADER_BYTES as u64) {
                    Ok(source) => {
                        write(session, Some(source), true, &mut self.status);
                        self.path = Some(path);
                        out.changed = true;
                    }
                    Err(e) => self.status = e.to_string(),
                }
            }
            if ui.button("Export .gdshader…").clicked()
                && let Some(path) = save_dialog("gdshader", self.path.as_ref())
            {
                self.status = match craft_atomic::atomic_write(&path, self.source.as_bytes()) {
                    Ok(()) => format!("Exported {}", path.display()),
                    Err(e) => e.to_string(),
                };
                self.path = Some(path);
            }
            if ui.button("Remove shader").clicked() {
                write(session, None, false, &mut self.status);
                out.changed = true;
            }
            ui.colored_label(crate::chrome::TEXT_WEAK, shader_name(&session.map, id));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.colored_label(crate::chrome::TEXT_WEAK, format!("Ln {}, Col {}", self.cursor_position[0].max(1), self.cursor_position[1].max(1)));
            });
        });
        ui.horizontal_wrapped(|ui| {
            match errors.get(&id) {
                Some(e) => ui.colored_label(egui::Color32::LIGHT_RED, e),
                None if id == mapcraft_map::ROOT => ui.colored_label(crate::chrome::TEXT_WEAK, "Drawn as the sky in the 3D views"),
                None => ui.colored_label(crate::chrome::TEXT_WEAK, "Drawn on this brush's faces in the 3D views"),
            };
            if !self.status.is_empty() {
                ui.separator();
                ui.colored_label(crate::chrome::TEXT_WEAK, &self.status);
            }
        });
        ui.separator();
        let lines = (1..=self.source.split('\n').count()).map(|line| line.to_string()).collect::<Vec<_>>().join("\n");
        ui.painter().rect_filled(ui.available_rect_before_wrap(), 0.0, crate::chrome::BG);
        egui::ScrollArea::both().id_salt(("shader-script", id.0)).show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.label(egui::RichText::new(lines).monospace().color(crate::chrome::TEXT_WEAK));
                let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, _width: f32| {
                    let font = egui::TextStyle::Monospace.resolve(ui.style());
                    let job = crate::shader_syntax::highlight(text.as_str(), font, ui.visuals().text_color());
                    ui.fonts_mut(|fonts| fonts.layout_job(job))
                };
                let output = egui::TextEdit::multiline(&mut self.source)
                    .id(egui::Id::new(("shader-source", id.0)))
                    .code_editor()
                    .hint_text("Write Godot shader source…")
                    .char_limit(mapcraft_map::MAX_SHADER_BYTES)
                    .desired_width(f32::INFINITY)
                    .desired_rows(40)
                    .layouter(&mut layouter)
                    .show(ui);
                if output.response.changed() {
                    write(session, Some(self.source.clone()), false, &mut self.status);
                    out.changed = true;
                }
                if output.response.lost_focus() {
                    session.end_coalesce();
                }
                if let Some(cursor) = output.cursor_range {
                    let mut line = 1;
                    let mut column = 1;
                    for ch in self.source.chars().take(cursor.primary.index.into()) {
                        if ch == '\n' {
                            line += 1;
                            column = 1;
                        } else {
                            column += 1;
                        }
                    }
                    self.cursor_position = [line, column];
                }
            });
        });
        out
    }

    fn shader_material_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Shader Material", |ui| {
            if ui.button("Shader Graph").clicked() {
                self.open_graph();
                ui.close();
            }
            if ui.button("Script").clicked() {
                self.open_script();
                ui.close();
            }
        });
    }

    fn graph_ui(&mut self, ui: &mut egui::Ui) {
        let before = self.graph.clone();
        self.graph_controls(ui);
        if std::mem::take(&mut self.graph_skip_history) {
            self.graph_pending = None;
            return;
        }
        if self.graph != before && self.graph_pending.is_none() {
            self.graph_pending = Some(before);
            self.graph_future.clear();
        }
        if !ui.input(|i| i.pointer.any_down())
            && let Some(before) = self.graph_pending.take()
            && before != self.graph
        {
            if self.graph_history.len() >= 64 {
                self.graph_history.remove(0);
            }
            self.graph_history.push(before);
        }
    }

    fn graph_controls(&mut self, ui: &mut egui::Ui) {
        use mapcraft_format::shader_graph::{Graph, MAX_NODES, Node};
        let dirty = self.graph_dirty();
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(!self.graph_history.is_empty(), egui::Button::new("Undo")).clicked() {
                self.action("edit.undo");
            }
            if ui.add_enabled(!self.graph_future.is_empty(), egui::Button::new("Redo")).clicked() {
                self.action("edit.redo");
            }
            if ui.add_enabled(!dirty, egui::Button::new("Open graph…")).clicked()
                && let Some(path) = rfd::FileDialog::new().add_filter("MapCraft shader graph", &["mcshader"]).pick_file()
            {
                let loaded =
                    bounded_text(&path, 256 * 1024).map_err(|e| e.to_string()).and_then(|text| serde_json::from_str::<Graph>(&text).map_err(|e| e.to_string()));
                match loaded.and_then(|graph| graph.source().map_err(|e| e.to_string()).map(|_| graph)) {
                    Ok(graph) => {
                        self.graph = graph;
                        self.graph_path = Some(path);
                        self.canvas = Default::default();
                        self.graph_history.clear();
                        self.graph_future.clear();
                        self.graph_skip_history = true;
                        self.saved_graph = serde_json::to_string_pretty(&self.graph).unwrap_or_default();
                        self.status = "Loaded graph".into();
                    }
                    Err(e) => self.status = e,
                }
            }
            if ui.button("Save graph").clicked() {
                self.save_active(false);
            }
            if ui.button("Generate source").on_hover_text("Replace the picked shader's source with the graph's").clicked() {
                match self.graph.source() {
                    Ok(source) => {
                        self.generated = Some(source);
                        self.graph_mode = false;
                    }
                    Err(e) => self.status = e.to_string(),
                }
            }
            if dirty {
                ui.label("Unsaved graph changes");
            }
        });
        ui.label("Drag node headers to arrange. Click or drag an output port to an input; invalid connections are refused.");
        ui.label(&self.status);
        ui.horizontal_wrapped(|ui| {
            let room = self.graph.nodes.len() < MAX_NODES;
            for (label, node) in [
                ("Add scalar", Node::Scalar { value: 1.0 }),
                ("Add color", Node::Color { value: [1.0; 3] }),
                ("Add time", Node::Time),
                ("Add multiply", Node::Multiply { a: 0, b: 1 }),
                ("Add sine", Node::Sine { input: 1 }),
            ] {
                if ui.add_enabled(room, egui::Button::new(label)).clicked() {
                    self.graph.nodes.push(node);
                }
            }
        });
        if let Err(e) = self.graph.source() {
            ui.colored_label(egui::Color32::LIGHT_RED, e.to_string());
        }
        self.canvas.show(ui, &mut self.graph, &mut self.status);
    }

    fn material_ui(&mut self, ui: &mut egui::Ui) {
        use mapcraft_format::pack_material::TextureSlot;
        ui.label("Save under materials/<name>.json in a custom asset folder. Texture names are relative to that folder.");
        let dirty = self.material != self.saved_material.clone().unwrap_or_default();
        if ui.add_enabled(!dirty, egui::Button::new("Open material descriptor…")).clicked()
            && let Some(path) = rfd::FileDialog::new().add_filter("PBR material descriptor", &["json"]).pick_file()
        {
            use std::io::Read;
            let mut bytes = Vec::new();
            let loaded = std::fs::File::open(&path)
                .and_then(|file| file.take((mapcraft_format::pack_material::MAX_DESCRIPTOR_SIZE + 1) as u64).read_to_end(&mut bytes))
                .map_err(|e| e.to_string())
                .and_then(|_| mapcraft_format::pack_material::Material::read(&bytes));
            match loaded {
                Ok(material) => {
                    self.saved_material = Some(material.clone());
                    self.material_path = Some(path.clone());
                    self.material = material;
                    self.status = format!("Opened {}", path.display());
                }
                Err(e) => self.status = e,
            }
        }
        if dirty {
            ui.label("Unsaved material changes");
        }
        ui.horizontal_wrapped(|ui| {
            ui.label("Albedo");
            ui.color_edit_button_rgba_unmultiplied(&mut self.material.albedo);
        });
        ui.add(egui::Slider::new(&mut self.material.roughness, 0.0..=1.0).text("Roughness"));
        ui.add(egui::Slider::new(&mut self.material.metallic, 0.0..=1.0).text("Metallic"));
        ui.horizontal_wrapped(|ui| {
            ui.label("Emission RGB");
            for value in &mut self.material.emission {
                ui.add(egui::DragValue::new(value).range(0.0..=64.0).speed(0.01));
            }
        });
        for (label, slot) in [
            ("Albedo texture", TextureSlot::Albedo),
            ("Normal texture", TextureSlot::Normal),
            ("Roughness texture", TextureSlot::Roughness),
            ("Metallic texture", TextureSlot::Metallic),
            ("Emission texture", TextureSlot::Emission),
        ] {
            ui.horizontal_wrapped(|ui| {
                ui.label(label);
                let mut path = self.material.textures.get(&slot).cloned().unwrap_or_default();
                if ui.text_edit_singleline(&mut path).changed() {
                    if path.is_empty() {
                        self.material.textures.remove(&slot);
                    } else {
                        self.material.textures.insert(slot, path);
                    }
                }
            });
        }
        if let Err(error) = self.material.validate() {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        if ui.button("Save material descriptor…").clicked() {
            self.save_active(false);
        }
        ui.label(&self.status);
        ui.label("Saved materials reload in the map viewport. Use Lit mode for PBR lighting. Shader bindings are not connected yet.");
    }
}

/// A save dialog for a `.extension` file, starting beside `current`.
fn save_dialog(extension: &str, current: Option<&PathBuf>) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new().add_filter("Shader document", &[extension]).set_file_name(format!("material.{extension}"));
    if let Some(path) = current {
        if let Some(parent) = path.parent() {
            dialog = dialog.set_directory(parent);
        }
        if let Some(file) = path.file_name() {
            dialog = dialog.set_file_name(file.to_string_lossy());
        }
    }
    dialog.save_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};
    use mapcraft_map::ROOT;

    type State = (ShaderEditor, Session, BTreeMap<NodeId, String>);

    fn workspace(state: State) -> Harness<'static, State> {
        Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(
            |ui, (editor, session, errors): &mut State| {
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(240.0);
                        editor.shader_list(ui, session, errors);
                    });
                    ui.vertical(|ui| {
                        if editor.workspace_tabs(ui) {
                            editor.content(ui, session, errors);
                        }
                    });
                });
            },
            state,
        )
    }

    #[test]
    fn save_all_preserves_shader_work_and_reports_invalid_documents() {
        let dir = tempfile::tempdir().unwrap();
        let mut editor =
            ShaderEditor { graph_path: Some(dir.path().join("graph.mcshader")), material_path: Some(dir.path().join("material.json")), ..Default::default() };
        assert!(!editor.has_unsaved());
        editor.source = "shader_type spatial;".into();
        assert!(!editor.has_unsaved(), "embedded shaders are saved with the map");
        editor.graph.nodes.push(mapcraft_format::shader_graph::Node::Time);
        editor.material.roughness = 0.3;
        assert!(editor.has_unsaved());
        assert!(editor.save_all());
        assert!(!editor.has_unsaved());
        editor.graph.nodes.push(mapcraft_format::shader_graph::Node::Scalar { value: f32::NAN });
        assert!(!editor.save_all());
        assert!(editor.has_unsaved());
        assert!(std::fs::read_to_string(dir.path().join("graph.mcshader")).unwrap().contains("nodes"));
    }

    #[test]
    fn save_and_undo_in_script_belong_to_the_map() {
        let dir = tempfile::tempdir().unwrap();
        let graph_path = dir.path().join("test.mcshader");
        let mut editor = ShaderEditor { graph_path: Some(graph_path.clone()), ..Default::default() };
        editor.open_script();
        assert!(!editor.action("file.save") && !editor.action("edit.undo"));
        editor.open_graph();
        assert!(editor.action("file.save"));
        assert_eq!(serde_json::from_slice::<mapcraft_format::shader_graph::Graph>(&std::fs::read(&graph_path).unwrap()).unwrap(), editor.graph);
    }

    #[test]
    fn graph_ports_connect_and_undo_without_corrupting_invalid_links() {
        let mut editor = ShaderEditor::default();
        editor.open_graph();
        let mut harness = workspace((editor, Session::default(), BTreeMap::new()));
        harness.get_by_label("Output of node 2").click();
        harness.run_steps(2);
        harness.get_by_label("Roughness input").click();
        harness.run_steps(2);
        assert_eq!(harness.state().0.graph.roughness, 2);
        harness.get_by_label("Undo").click();
        harness.run_steps(2);
        assert_eq!(harness.state().0.graph.roughness, 1);
        harness.get_by_label("Redo").click();
        harness.run_steps(2);
        assert_eq!(harness.state().0.graph.roughness, 2);
        let valid = harness.state().0.graph.clone();
        harness.get_by_label("Output of node 1").click();
        harness.run_steps(2);
        harness.get_by_label("Albedo input").click();
        harness.run_steps(2);
        assert_eq!(harness.state().0.graph, valid);
        assert!(harness.state().0.status.contains("incompatible"));
    }

    #[test]
    fn the_list_picks_a_node_and_the_script_writes_its_shader_undoably() {
        let mut session = Session::default();
        let brush = session.create_brush(mapcraft_geom::Aabb::new(mapcraft_geom::DVec3::ZERO, mapcraft_geom::DVec3::splat(64.0))).unwrap();
        session.selection = [brush].into();
        session.add_shader().unwrap();
        session.selection.clear();
        session.add_shader().unwrap();
        let errors = BTreeMap::from([(brush, "line 3: broken".to_string())]);
        let mut editor = ShaderEditor::default();
        editor.open_script();
        let mut harness = workspace((editor, session, errors));
        harness.run_steps(2);
        harness.get_by_label("World (sky)");
        harness.get_by_label(&format!("Brush #{}", brush.0)).click();
        harness.run_steps(2);
        assert_eq!(harness.state().0.target, Some(brush));
        assert!(harness.state().1.selection.contains(&brush));
        harness.get_by_label("line 3: broken");
        harness.get_by_role(egui::accesskit::Role::MultilineTextInput).click();
        harness.run_steps(2);
        harness.get_by_role(egui::accesskit::Role::MultilineTextInput).type_text("//a");
        harness.run_steps(2);
        harness.get_by_role(egui::accesskit::Role::MultilineTextInput).type_text("b");
        harness.run_steps(2);
        let edited = harness.state().1.map.get(brush).unwrap().shader.clone().unwrap();
        assert!(edited.contains("//ab") && edited != mapcraft_engine::SPATIAL_TEMPLATE);
        assert_eq!(harness.state().1.undo_label(), Some("Edit shader"));
        assert!(harness.state().1.is_dirty());
        assert!(harness.state_mut().1.undo(), "typing undoes as one step");
        harness.run_steps(2);
        assert_eq!(harness.state().1.map.get(brush).unwrap().shader.as_deref(), Some(mapcraft_engine::SPATIAL_TEMPLATE));
        assert_eq!(harness.state().0.source, mapcraft_engine::SPATIAL_TEMPLATE, "the editor follows the map's undo");
        harness.get_by_role(egui::accesskit::Role::TextInput).click();
        harness.run_steps(2);
        harness.get_by_role(egui::accesskit::Role::TextInput).type_text("world");
        harness.run_steps(2);
        assert_eq!(harness.query_all_by_label(&format!("Brush #{}", brush.0)).count(), 1, "only the toolbar still names the brush");
        harness.get_by_label("World (sky)").click();
        harness.run_steps(2);
        assert_eq!(harness.state().0.target, Some(ROOT));
        assert_eq!(harness.state().0.source, mapcraft_engine::SKY_TEMPLATE);
    }

    #[test]
    fn workspace_tabs_preserve_unsaved_documents() {
        let mut editor = ShaderEditor::default();
        editor.graph.nodes.push(mapcraft_format::shader_graph::Node::Time);
        let graph = editor.graph.clone();
        editor.open_graph();
        let mut harness = workspace((editor, Session::default(), BTreeMap::new()));
        harness.get_by_label("Script").click();
        harness.run_steps(2);
        assert!(!harness.state().0.graph_mode);
        harness.get_by_label("Viewport").click();
        harness.run_steps(2);
        assert!(harness.state().0.viewport);
        harness.get_by_label("Shader Graph").click();
        harness.run_steps(2);
        assert_eq!(harness.state().0.graph, graph);
        assert!(!harness.state().0.viewport);
    }
}
