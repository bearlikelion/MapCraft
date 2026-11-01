//! The Material inspector: a PBR descriptor's settings, shown live in the views and written back to its file.

use std::path::PathBuf;

use mapcraft_format::pack_material::{Material, TextureSlot};

use crate::inspect::{HeaderButton, Section, Title};

pub(crate) struct MaterialInspector {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    /// The descriptor as it was opened, which Revert goes back to.
    opened: Material,
    pub(crate) material: Material,
    /// The last values written or refused, so a failed write is not retried every frame.
    tried: Material,
    pub(crate) status: String,
}

/// What drawing the inspector did.
#[derive(Default)]
pub(crate) struct Edit {
    /// The values changed and the views should show them.
    pub changed: bool,
    /// The descriptor file was written.
    pub saved: bool,
}

impl MaterialInspector {
    pub(crate) fn new(name: &str, path: PathBuf, material: Material) -> Self {
        Self { name: name.into(), path, opened: material.clone(), tried: material.clone(), material, status: String::new() }
    }

    /// Changed since it was opened.
    pub(crate) fn changed(&self) -> bool {
        self.material != self.opened
    }

    /// Validates, then writes the descriptor atomically, so a failure leaves the old file whole.
    pub(crate) fn save(&mut self) -> bool {
        self.tried = self.material.clone();
        let written = self.material.to_json().and_then(|bytes| craft_atomic::atomic_write(&self.path, &bytes).map_err(|e| e.to_string()));
        self.status = match &written {
            Ok(()) => format!("Saved {}", self.path.display()),
            Err(e) => e.clone(),
        };
        written.is_ok()
    }

    pub(crate) fn revert(&mut self) -> bool {
        self.material = self.opened.clone();
        self.save()
    }

    /// The descriptor's rows; edits save once the pointer is released.
    pub(crate) fn ui(&mut self, ui: &mut egui::Ui) -> Edit {
        let before = self.material.clone();
        let m = &mut self.material;
        egui::Grid::new("material-inspector").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            ui.label("Albedo tint").on_hover_text("Multiplies the albedo texture, or colors the surface without one");
            ui.color_edit_button_rgba_unmultiplied(&mut m.albedo);
            ui.end_row();
            ui.label("Roughness");
            ui.add(egui::Slider::new(&mut m.roughness, 0.0..=1.0));
            ui.end_row();
            ui.label("Metallic");
            ui.add(egui::Slider::new(&mut m.metallic, 0.0..=1.0));
            ui.end_row();
            ui.label("Emission color").on_hover_text("Added to the emission texture before the energy multiplies both");
            ui.color_edit_button_rgb(&mut m.emission);
            ui.end_row();
            crate::shader_editor::strength_rows(ui, m);
            for (label, slot) in [
                ("Albedo texture", TextureSlot::Albedo),
                ("Normal texture", TextureSlot::Normal),
                ("Roughness texture", TextureSlot::Roughness),
                ("Metallic texture", TextureSlot::Metallic),
                ("Emission texture", TextureSlot::Emission),
                ("Height texture", TextureSlot::Height),
            ] {
                ui.label(label);
                let name = m.textures.get(&slot).map_or("None", |t| t.rsplit('/').next().unwrap_or(t));
                crate::inspect::weak(ui, name).on_hover_text(m.textures.get(&slot).map_or("", String::as_str));
                ui.end_row();
            }
        });
        m.version = m.needed_version();
        let mut out = Edit { changed: self.material != before, saved: false };
        if self.material != self.tried && !ui.input(|i| i.pointer.any_down()) {
            out.saved = self.save();
        }
        if !self.status.is_empty() {
            let color = if out.saved || self.status.starts_with("Saved") { crate::chrome::TEXT_WEAK } else { egui::Color32::LIGHT_RED };
            ui.colored_label(color, &self.status);
        }
        out
    }
}

impl crate::app::MapCraftApp {
    /// Opens the Material inspector on `name`, a PBR descriptor in a custom asset folder.
    pub(crate) fn inspect_material(&mut self, name: &str) {
        if self.material_inspector.as_ref().is_some_and(|i| i.name == name) {
            return;
        }
        let found = self.project.descriptor_file(name).zip(self.project.find_material(name).and_then(|m| m.descriptor));
        match found {
            Some((path, material)) => self.material_inspector = Some(MaterialInspector::new(name, path, material)),
            None if name.is_empty() => self.status = "Pick a material in the Materials panel first".into(),
            None => self.status = format!("{name} is a plain picture, not a PBR material; File > Add Material makes one from its pictures"),
        }
    }

    pub(crate) fn revert_material(&mut self) {
        let Some(inspector) = &mut self.material_inspector else {
            self.status = "No material is open in the inspector".into();
            return;
        };
        let saved = inspector.revert();
        let (name, material) = (inspector.name.clone(), inspector.material.clone());
        self.show_material(&name, &material, saved);
    }

    /// Draws `material` in the views without reloading the project.
    fn show_material(&mut self, name: &str, material: &Material, saved: bool) {
        self.project.set_descriptor_values(name, material);
        if let Some(gpu) = &mut self.gpu {
            gpu.renderer.forget_material(name);
        }
        if saved {
            self.thumbs.forget(name);
        }
        self.scene_dirty = true;
    }

    pub(crate) fn material_header(&mut self, ui: &mut egui::Ui) {
        let Some(inspector) = &self.material_inspector else { return };
        let title = inspector.name.rsplit('/').next().unwrap_or(&inspector.name).to_string();
        let buttons = [
            HeaderButton { icon: "recover", label: "Revert Material", tip: "Go back to the values it had when opened, and save them", on: false },
            HeaderButton { icon: "cross_in_square", label: "Close", tip: "Close the Material inspector", on: false },
        ];
        let path = inspector.path.display().to_string();
        let out = crate::inspect::header(ui, ("swatches", crate::chrome::TEXT), Title::Fixed(&title), &inspector.name, &buttons, |ui| {
            crate::inspect::weak(ui, &path).on_hover_text(&path);
        });
        match out.clicked {
            Some(0) => self.run_action("material.revert"),
            Some(1) => self.material_inspector = None,
            _ => {}
        }
    }

    pub(crate) fn material_body(&mut self, ui: &mut egui::Ui) {
        if !self.section(ui, &Section { key: "material.pbr", title: "PBR material", ..Default::default() }) {
            return;
        }
        let Some(inspector) = &mut self.material_inspector else { return };
        let edit = inspector.ui(ui);
        let changed = if inspector.changed() { "Changed since opened; Revert goes back" } else { "Edits save to the file when you let go" };
        ui.colored_label(crate::chrome::TEXT_WEAK, changed);
        ui.colored_label(crate::chrome::TEXT_WEAK, "Switch a view to Lit to see roughness, metallic, normals and emission.");
        let (name, material) = (inspector.name.clone(), inspector.material.clone());
        if edit.changed || edit.saved {
            self.show_material(&name, &material, edit.saved);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;

    fn opened(dir: &std::path::Path) -> MaterialInspector {
        let mut material = Material::default();
        material.textures.insert(TextureSlot::Normal, "textures/stone_normal.png".into());
        let path = dir.join("stone.json");
        std::fs::write(&path, material.to_json().unwrap()).unwrap();
        MaterialInspector::new("stone", path, material)
    }

    #[test]
    fn edits_save_on_release_and_revert_restores_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(500.0, 500.0))
            .build_ui_state(|ui, (inspector, edits): &mut (MaterialInspector, Vec<Edit>)| edits.push(inspector.ui(ui)), (opened(dir.path()), Vec::new()));
        harness.run();
        harness.state_mut().0.material.normal_strength = 0.25;
        harness.state_mut().0.material.emission_energy = 3.0;
        harness.run_steps(1);
        assert!(harness.state().1.iter().any(|e| e.saved));
        let path = harness.state().0.path.clone();
        let read = Material::read(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!((read.version, read.normal_strength, read.emission_energy), (2, 0.25, 3.0));
        assert_eq!(read.textures.get(&TextureSlot::Normal).map(String::as_str), Some("textures/stone_normal.png"));
        assert!(harness.state().0.changed());
        assert!(harness.state_mut().0.revert());
        assert!(!harness.state().0.changed());
        assert_eq!(Material::read(&std::fs::read(&path).unwrap()).unwrap().version, 1, "revert writes the plain descriptor again");
    }

    #[test]
    fn invalid_values_never_reach_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut inspector = opened(dir.path());
        let good = std::fs::read(&inspector.path).unwrap();
        inspector.material.roughness = f32::NAN;
        assert!(!inspector.save());
        assert!(!inspector.status.is_empty());
        assert_eq!(std::fs::read(&inspector.path).unwrap(), good);
        inspector.path = inspector.path.join("inside_a_file.json");
        inspector.material.roughness = 0.5;
        assert!(!inspector.save(), "a write that fails reports instead of crashing");
    }
}
