//! Source editing for external Godot shader files.

use std::path::PathBuf;

#[derive(Default)]
pub(crate) struct ShaderEditor {
    pub(crate) open: bool,
    path: Option<PathBuf>,
    source: String,
    saved: String,
    status: String,
    graph: mapcraft_format::shader_graph::Graph,
    graph_mode: bool,
    saved_graph: String,
}

impl ShaderEditor {
    pub(crate) fn show(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        let mut open = true;
        egui::Window::new("Shader Editor").open(&mut open).default_size([720.0, 540.0]).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.graph_mode, false, "Source");
                ui.selectable_value(&mut self.graph_mode, true, "Graph");
            });
            if self.graph_mode {
                self.graph_ui(ui);
                return;
            }
            let dirty = self.source != self.saved;
            ui.horizontal(|ui| {
                if ui.add_enabled(!dirty, egui::Button::new("Open .gdshader…")).clicked()
                    && let Some(path) = rfd::FileDialog::new().add_filter("Godot shader", &["gdshader"]).pick_file()
                {
                    match std::fs::metadata(&path).and_then(|m| {
                        if m.len() > 256 * 1024 {
                            return Err(std::io::Error::other("Shader exceeds 256 KiB"));
                        }
                        std::fs::read_to_string(&path)
                    }) {
                        Ok(source) => {
                            self.saved = source.clone();
                            self.source = source;
                            self.path = Some(path);
                            self.status.clear();
                        }
                        Err(e) => self.status = e.to_string(),
                    }
                }
                if ui.button("Save As…").clicked()
                    && let Some(path) = rfd::FileDialog::new().add_filter("Godot shader", &["gdshader"]).set_file_name("material.gdshader").save_file()
                {
                    match craft_atomic::atomic_write(&path, self.source.as_bytes()) {
                        Ok(()) => {
                            self.saved = self.source.clone();
                            self.path = Some(path);
                            self.status = "Saved shader source".into();
                        }
                        Err(e) => self.status = e.to_string(),
                    }
                }
                if dirty {
                    ui.label("Unsaved changes");
                }
            });
            if let Some(path) = &self.path {
                ui.label(path.display().to_string());
            }
            ui.label("Source editing only. Shader compilation and viewport preview are not connected yet.");
            ui.label(&self.status);
            egui::ScrollArea::both().show(ui, |ui| {
                ui.add(egui::TextEdit::multiline(&mut self.source).code_editor().desired_width(f32::INFINITY).desired_rows(24));
            });
        });
        self.open = open;
    }

    fn graph_ui(&mut self, ui: &mut egui::Ui) {
        use mapcraft_format::shader_graph::{Graph, MAX_NODES, Node};
        let serialized = serde_json::to_string_pretty(&self.graph);
        let dirty = serialized.as_ref().is_ok_and(|s| s != &self.saved_graph) && !(self.saved_graph.is_empty() && self.graph == Graph::default());
        ui.horizontal(|ui| {
            if ui.add_enabled(!dirty, egui::Button::new("Open graph…")).clicked()
                && let Some(path) = rfd::FileDialog::new().add_filter("MapCraft shader graph", &["mcshader"]).pick_file()
            {
                let loaded = std::fs::metadata(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|metadata| {
                        if metadata.len() > 256 * 1024 {
                            return Err("Graph exceeds 256 KiB".into());
                        }
                        std::fs::read_to_string(&path).map_err(|e| e.to_string())
                    })
                    .and_then(|text| serde_json::from_str::<Graph>(&text).map_err(|e| e.to_string()));
                match loaded.and_then(|graph| graph.source().map_err(|e| e.to_string()).map(|_| graph)) {
                    Ok(graph) => {
                        self.graph = graph;
                        self.saved_graph = serde_json::to_string_pretty(&self.graph).unwrap_or_default();
                        self.status = "Loaded graph".into();
                    }
                    Err(e) => self.status = e,
                }
            }
            if ui.button("Save graph…").clicked()
                && let Some(path) = rfd::FileDialog::new().add_filter("MapCraft shader graph", &["mcshader"]).set_file_name("material.mcshader").save_file()
            {
                match self
                    .graph
                    .source()
                    .map_err(|e| e.to_string())
                    .and_then(|_| serialized.as_ref().map_err(|e| e.to_string()))
                    .and_then(|text| craft_atomic::atomic_write(&path, text.as_bytes()).map_err(|e| e.to_string()).map(|_| text.clone()))
                {
                    Ok(text) => {
                        self.saved_graph = text;
                        self.status = "Saved graph".into();
                    }
                    Err(e) => self.status = e,
                }
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
        ui.label("Inputs reference earlier node numbers. Generate source, then Save As to export .gdshader.");
        ui.label(&self.status);
        ui.horizontal(|ui| {
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
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (id, node) in self.graph.nodes.iter_mut().enumerate() {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.label(format!("Node {id}"));
                    match node {
                        Node::Scalar { value } => {
                            ui.horizontal(|ui| {
                                ui.label("Scalar");
                                ui.add(egui::DragValue::new(value).speed(0.01));
                            });
                        }
                        Node::Color { value } => {
                            ui.horizontal(|ui| {
                                ui.label("Color");
                                ui.color_edit_button_rgb(value);
                            });
                        }
                        Node::Time => {
                            ui.label("Time (seconds)");
                        }
                        Node::Multiply { a, b } => {
                            ui.horizontal(|ui| {
                                ui.label("Multiply inputs");
                                ui.add(egui::DragValue::new(a));
                                ui.add(egui::DragValue::new(b));
                            });
                        }
                        Node::Sine { input } => {
                            ui.horizontal(|ui| {
                                ui.label("Sine input");
                                ui.add(egui::DragValue::new(input));
                            });
                        }
                    }
                });
            }
            ui.separator();
            ui.label("Spatial output connections");
            for (label, input) in
                [("Albedo (color)", &mut self.graph.albedo), ("Roughness (scalar)", &mut self.graph.roughness), ("Metallic (scalar)", &mut self.graph.metallic)]
            {
                ui.horizontal(|ui| {
                    ui.label(label);
                    ui.add(egui::DragValue::new(input));
                });
            }
            let mut emission = self.graph.emission.is_some();
            if ui.checkbox(&mut emission, "Emission").changed() {
                self.graph.emission = emission.then_some(0);
            }
            if let Some(input) = &mut self.graph.emission {
                ui.add(egui::DragValue::new(input));
            }
            match self.graph.source() {
                Ok(source) => {
                    ui.collapsing("Generated .gdshader", |ui| {
                        ui.monospace(source);
                    });
                }
                Err(e) => {
                    ui.colored_label(egui::Color32::LIGHT_RED, e.to_string());
                }
            }
        });
    }
}
