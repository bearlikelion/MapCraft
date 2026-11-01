//! Source editing for external Godot shader files.

use std::path::PathBuf;

fn bounded_text(path: &std::path::Path, limit: u64) -> Result<String, std::io::Error> {
    use std::io::Read;
    let mut text = String::new();
    std::fs::File::open(path)?.take(limit.saturating_add(1)).read_to_string(&mut text)?;
    if text.len() as u64 > limit {
        return Err(std::io::Error::other("Shader document exceeds 256 KiB"));
    }
    Ok(text)
}

#[derive(Default)]
pub(crate) struct ShaderEditor {
    pub(crate) open: bool,
    viewport: bool,
    cursor_position: [usize; 2],
    path: Option<PathBuf>,
    graph_path: Option<PathBuf>,
    material_path: Option<PathBuf>,
    source: String,
    saved: String,
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

    pub(crate) fn has_unsaved(&self) -> bool {
        self.source != self.saved
            || self.graph_dirty()
            || self.saved_material.as_ref().map_or_else(|| self.material != Default::default(), |saved| &self.material != saved)
    }

    pub(crate) fn save_all(&mut self) -> bool {
        let modes = (self.graph_mode, self.material_mode);
        if self.source != self.saved {
            self.graph_mode = false;
            self.material_mode = false;
            self.save_active(false);
            if self.source != self.saved {
                self.graph_mode = modes.0;
                self.material_mode = modes.1;
                return false;
            }
        }
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
        if self.saved_material.as_ref().map_or_else(|| self.material != Default::default(), |saved| &self.material != saved) {
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

    pub(crate) fn action(&mut self, id: &str) -> bool {
        if !self.is_active() {
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
            "edit.undo" | "edit.redo" => self.status = "Focus the Script editor to use text undo and redo.".into(),
            _ => return false,
        }
        true
    }

    fn save_active(&mut self, save_as: bool) {
        let (extension, current, bytes) = if self.material_mode {
            ("json", self.material_path.clone(), self.material.validate().and_then(|()| serde_json::to_vec_pretty(&self.material).map_err(|e| e.to_string())))
        } else if self.graph_mode {
            (
                "mcshader",
                self.graph_path.clone(),
                self.graph.source().map_err(|e| e.to_string()).and_then(|_| serde_json::to_vec_pretty(&self.graph).map_err(|e| e.to_string())),
            )
        } else {
            ("gdshader", self.path.clone(), Ok(self.source.as_bytes().to_vec()))
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
        let path = if !save_as && current.is_some() {
            current
        } else {
            let mut dialog = rfd::FileDialog::new().add_filter("Shader material document", &[extension]).set_file_name(format!("material.{extension}"));
            if let Some(path) = &current {
                if let Some(parent) = path.parent() {
                    dialog = dialog.set_directory(parent);
                }
                if let Some(file) = path.file_name() {
                    dialog = dialog.set_file_name(file.to_string_lossy());
                }
            }
            dialog.save_file()
        };
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
        } else if self.graph_mode {
            self.graph_path = Some(path);
            self.saved_graph = String::from_utf8_lossy(&bytes).into_owned();
        } else {
            self.path = Some(path);
            self.saved = self.source.clone();
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
        ui.separator();
        self.is_active()
    }

    pub(crate) fn content(&mut self, ui: &mut egui::Ui) {
        if self.material_mode {
            self.shader_material_menu(ui);
            egui::ScrollArea::vertical().id_salt("material-editor").show(ui, |ui| {
                egui::CollapsingHeader::new("Material").id_salt("pbr-material").default_open(true).show(ui, |ui| self.material_ui(ui));
            });
            return;
        }
        if self.graph_mode {
            self.graph_ui(ui);
            return;
        }
        let dirty = self.source != self.saved;
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(!dirty, egui::Button::new("Open .gdshader…")).clicked()
                && let Some(path) = rfd::FileDialog::new().add_filter("Godot shader", &["gdshader"]).pick_file()
            {
                match bounded_text(&path, 256 * 1024) {
                    Ok(source) => {
                        self.saved = source.clone();
                        self.source = source;
                        self.path = Some(path);
                        self.status.clear();
                    }
                    Err(e) => self.status = e.to_string(),
                }
            }
            if ui.button("Save").clicked() {
                self.save_active(false);
            }
            if ui.button("Save As…").clicked() {
                self.save_active(true);
            }
            if dirty {
                ui.label("Unsaved changes");
            }
        });
        if let Some(path) = &self.path {
            ui.label(path.display().to_string());
        }
        if self.source.len() > 256 * 1024 {
            ui.colored_label(egui::Color32::LIGHT_RED, "Source exceeds 256 KiB; shorten it before saving.");
        }
        ui.label("Source editing only. Shader compilation and viewport preview are not connected yet.");
        ui.label(&self.status);
        ui.label(format!("Ln {}, Col {}", self.cursor_position[0].max(1), self.cursor_position[1].max(1)));
        let lines = (1..=self.source.split('\n').count()).map(|line| line.to_string()).collect::<Vec<_>>().join("\n");
        egui::ScrollArea::both().id_salt("shader-script").show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.label(egui::RichText::new(lines).monospace().color(crate::chrome::TEXT_WEAK));
                let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, _width: f32| {
                    let font = egui::TextStyle::Monospace.resolve(ui.style());
                    let job = crate::shader_syntax::highlight(text.as_str(), font, ui.visuals().text_color());
                    ui.fonts_mut(|fonts| fonts.layout_job(job))
                };
                let output = egui::TextEdit::multiline(&mut self.source)
                    .id_salt("shader-source")
                    .code_editor()
                    .hint_text("Write Godot shader source…")
                    .char_limit(256 * 1024)
                    .desired_width(f32::INFINITY)
                    .desired_rows(40)
                    .layouter(&mut layouter)
                    .show(ui);
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
            if ui.add_enabled(self.source == self.saved, egui::Button::new("Generate source")).clicked() {
                match self.graph.source() {
                    Ok(source) => {
                        self.source = source;
                        self.path = None;
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

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    #[test]
    fn save_all_preserves_shader_work_and_reports_invalid_documents() {
        let dir = tempfile::tempdir().unwrap();
        let mut editor = ShaderEditor {
            path: Some(dir.path().join("source.gdshader")),
            graph_path: Some(dir.path().join("graph.mcshader")),
            material_path: Some(dir.path().join("material.json")),
            ..Default::default()
        };
        assert!(!editor.has_unsaved());
        editor.source = "shader_type spatial;".into();
        editor.graph.nodes.push(mapcraft_format::shader_graph::Node::Time);
        editor.material.roughness = 0.3;
        assert!(editor.has_unsaved());
        assert!(editor.save_all());
        assert!(!editor.has_unsaved());
        assert_eq!(editor.source, "shader_type spatial;");
        editor.graph.nodes.push(mapcraft_format::shader_graph::Node::Scalar { value: f32::NAN });
        assert!(!editor.save_all());
        assert!(editor.has_unsaved());
        assert_eq!(editor.source, "shader_type spatial;");
        assert!(std::fs::read_to_string(dir.path().join("source.gdshader")).unwrap().contains("shader_type"));
    }

    #[test]
    fn save_action_targets_the_active_shader_document_and_preserves_failed_saves() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("test.gdshader");
        let graph_path = dir.path().join("test.mcshader");
        let mut editor =
            ShaderEditor { path: Some(source_path.clone()), graph_path: Some(graph_path.clone()), source: "shader_type spatial;".into(), ..Default::default() };
        editor.open_script();
        assert!(editor.action("file.save"));
        assert_eq!(std::fs::read_to_string(&source_path).unwrap(), "shader_type spatial;");
        assert_eq!(editor.saved, editor.source);
        editor.source = "x".repeat(256 * 1024 + 1);
        editor.action("file.save");
        assert!(editor.status.contains("exceeds"));
        assert_eq!(std::fs::read_to_string(&source_path).unwrap(), "shader_type spatial;");
        editor.open_graph();
        editor.action("file.save");
        assert_eq!(serde_json::from_slice::<mapcraft_format::shader_graph::Graph>(&std::fs::read(&graph_path).unwrap()).unwrap(), editor.graph);
    }

    #[test]
    fn graph_ports_connect_and_undo_without_corrupting_invalid_links() {
        let mut editor = ShaderEditor::default();
        editor.open_graph();
        let mut harness = Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(
            |ui, state: &mut ShaderEditor| {
                if state.workspace_tabs(ui) {
                    state.content(ui);
                }
            },
            editor,
        );
        harness.get_by_label("Output of node 2").click();
        harness.run_steps(2);
        harness.get_by_label("Roughness input").click();
        harness.run_steps(2);
        assert_eq!(harness.state().graph.roughness, 2);
        harness.get_by_label("Undo").click();
        harness.run_steps(2);
        assert_eq!(harness.state().graph.roughness, 1);
        harness.get_by_label("Redo").click();
        harness.run_steps(2);
        assert_eq!(harness.state().graph.roughness, 2);
        let valid = harness.state().graph.clone();
        harness.get_by_label("Output of node 1").click();
        harness.run_steps(2);
        harness.get_by_label("Albedo input").click();
        harness.run_steps(2);
        assert_eq!(harness.state().graph, valid);
        assert!(harness.state().status.contains("incompatible"));
    }

    #[test]
    fn workspace_tabs_preserve_unsaved_documents() {
        let mut editor = ShaderEditor { source: "shader_type spatial;\n// unsaved".into(), ..Default::default() };
        editor.graph.nodes.push(mapcraft_format::shader_graph::Node::Time);
        let graph = editor.graph.clone();
        editor.open_graph();
        let mut harness = Harness::new_ui_state(
            |ui, state: &mut ShaderEditor| {
                if state.workspace_tabs(ui) {
                    state.content(ui);
                }
            },
            editor,
        );
        harness.get_by_label("Script").click();
        harness.run_steps(2);
        assert!(!harness.state().graph_mode);
        assert!(harness.state().source.contains("unsaved"));
        harness.get_by_label("Viewport").click();
        harness.run_steps(2);
        assert!(harness.state().viewport);
        harness.get_by_label("Shader Graph").click();
        harness.run_steps(2);
        assert_eq!(harness.state().graph, graph);
        assert!(!harness.state().viewport);
        assert!(harness.state().source.contains("unsaved"));
    }
}
