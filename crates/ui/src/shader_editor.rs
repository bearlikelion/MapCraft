//! Source editing for external Godot shader files.

use std::path::PathBuf;

#[derive(Default)]
pub(crate) struct ShaderEditor {
    pub(crate) open: bool,
    path: Option<PathBuf>,
    source: String,
    saved: String,
    status: String,
}

impl ShaderEditor {
    pub(crate) fn show(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        let mut open = true;
        egui::Window::new("Shader Editor").open(&mut open).default_size([720.0, 540.0]).show(ctx, |ui| {
            let dirty = self.source != self.saved;
            ui.horizontal(|ui| {
                if ui.add_enabled(!dirty, egui::Button::new("Open .gdshader…")).clicked() {
                    if let Some(path) = rfd::FileDialog::new().add_filter("Godot shader", &["gdshader"]).pick_file() {
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
                }
                if ui.button("Save As…").clicked() {
                    if let Some(path) = rfd::FileDialog::new().add_filter("Godot shader", &["gdshader"]).set_file_name("material.gdshader").save_file() {
                        match craft_atomic::atomic_write(&path, self.source.as_bytes()) {
                            Ok(()) => {
                                self.saved = self.source.clone();
                                self.path = Some(path);
                                self.status = "Saved shader source".into();
                            }
                            Err(e) => self.status = e.to_string(),
                        }
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
}
