use mapcraft_geom::DVec3;
use mapcraft_map::{Generator, NodeId};
use mapcraft_plugin_host::{CommandSpec, Context, ParamKind, ParamSpec};
use serde_json::{Value, json};

use crate::app::MapCraftApp;

/// A plugin command waiting for its parameters before it runs.
pub(crate) struct PluginDialog {
    plugin: String,
    command: CommandSpec,
    params: Value,
}

/// Parameter widgets generated from the command's specs; true when a value changed.
fn param_widgets(ui: &mut egui::Ui, salt: impl std::hash::Hash + std::fmt::Debug, specs: &[ParamSpec], values: &mut Value, current_material: &str) -> bool {
    if !values.is_object() {
        *values = json!({});
    }
    let mut changed = false;
    egui::Grid::new(("params", salt)).num_columns(2).striped(true).show(ui, |ui| {
        for p in specs {
            let r = ui.label(&p.label);
            if !p.help.is_empty() {
                r.on_hover_text(&p.help);
            }
            let v = values.get(&p.id).cloned().unwrap_or_else(|| p.initial.clone());
            let new = match &p.kind {
                ParamKind::Number { min, max, step } => {
                    let mut n = v.as_f64().unwrap_or(*min);
                    ui.add(egui::DragValue::new(&mut n).range(*min..=*max).speed(*step)).changed().then(|| json!(n))
                }
                ParamKind::Integer { min, max } => {
                    let mut n = v.as_i64().or_else(|| v.as_f64().map(|f| f.round() as i64)).unwrap_or(*min);
                    ui.add(egui::DragValue::new(&mut n).range(*min..=*max)).changed().then(|| json!(n))
                }
                ParamKind::Toggle => {
                    let mut on = v.as_bool().unwrap_or(false);
                    ui.checkbox(&mut on, "").changed().then(|| json!(on))
                }
                ParamKind::Choice(options) => {
                    let mut s = v.as_str().unwrap_or_default().to_string();
                    egui::ComboBox::from_id_salt(("choice", &p.id)).selected_text(&s).show_ui(ui, |ui| {
                        for o in options {
                            ui.selectable_value(&mut s, o.clone(), o);
                        }
                    });
                    (Some(s.as_str()) != v.as_str()).then(|| json!(s))
                }
                ParamKind::Text => {
                    let mut s = v.as_str().unwrap_or_default().to_string();
                    ui.text_edit_singleline(&mut s).changed().then(|| json!(s))
                }
                ParamKind::Material => {
                    let mut s = v.as_str().unwrap_or_default().to_string();
                    ui.horizontal(|ui| {
                        let typed = ui.add(egui::TextEdit::singleline(&mut s).hint_text("current material").desired_width(140.0)).changed();
                        let picked = ui.small_button("Use current").on_hover_text("The material picked in the browser").clicked();
                        if picked {
                            s = current_material.to_string();
                        }
                        (typed || picked).then(|| json!(s))
                    })
                    .inner
                }
            };
            if let (Some(n), Some(obj)) = (new, values.as_object_mut()) {
                obj.insert(p.id.clone(), n);
                changed = true;
            }
            ui.end_row();
        }
    });
    changed
}

impl MapCraftApp {
    fn plugin_context(&self) -> Context {
        Context { grid: self.session.grid, material: self.session.tex.material.clone(), scale: self.session.tex.scale }
    }

    /// Runs a command of a loaded plugin, passing its log lines on to the Output panel.
    fn run_plugin(&mut self, plugin: &str, command: &str, params: &Value) -> Result<Vec<mapcraft_format::Fragment>, String> {
        let ctx = self.plugin_context();
        let p = self.project.plugin(plugin).ok_or_else(|| format!("plugin `{plugin}` is not loaded; is it in this SDK's plugins folder?"))?;
        let result = p.run(command, params, &ctx).map_err(|e| e.to_string());
        let log = p.take_log();
        self.output.extend(log);
        result
    }

    /// Where new plugin output goes: the selection's center, else the last point under the cursor.
    fn placement_point(&self) -> DVec3 {
        let p = self.session.selection_center().or(self.cursor_world).unwrap_or_default();
        mapcraft_geom::snap(p, self.session.grid)
    }

    pub(crate) fn plugins_menu(&mut self, ui: &mut egui::Ui) {
        let commands: Vec<(String, CommandSpec)> =
            self.project.plugins.iter().flat_map(|p| p.manifest.commands.iter().map(|c| (p.manifest.id.clone(), c.clone()))).collect();
        ui.menu_button("Plugins", |ui| {
            if commands.is_empty() {
                ui.label("This SDK's plugins have no commands");
            }
            for (plugin, c) in commands {
                let label = c.menu.rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or(&c.label).to_string();
                if ui.button(format!("{label}…")).clicked() {
                    let params = c.defaults();
                    self.plugin_dialog = Some(PluginDialog { plugin, command: c, params });
                }
            }
            ui.separator();
            for p in &self.project.plugins {
                ui.label(egui::RichText::new(format!("{} {}", p.manifest.name, p.manifest.version)).weak());
            }
        });
    }

    pub(crate) fn plugin_window(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.plugin_dialog.take() else { return };
        let mut open = true;
        let mut create = false;
        let material = self.session.tex.material.clone();
        egui::Window::new(&d.command.label).open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
            egui::ScrollArea::vertical().max_height(480.0).show(ui, |ui| {
                param_widgets(ui, ("dialog", &d.command.id), &d.command.params, &mut d.params, &material);
            });
            if d.command.generator {
                ui.label(egui::RichText::new("Makes a group you can change later from the inspector.").weak().small());
            }
            create = ui.button("Create").clicked();
        });
        if create {
            let origin = self.placement_point();
            match self.run_plugin(&d.plugin, &d.command.id, &d.params) {
                Ok(frags) if d.command.generator => {
                    let g = Generator { plugin: d.plugin.clone(), command: d.command.id.clone(), params: d.params.clone(), origin, angles: DVec3::ZERO };
                    if let Err(e) = self.session.create_generator(&d.command.label, g, frags) {
                        self.status = e.to_string();
                    }
                    open = false;
                }
                Ok(frags) => {
                    if let Err(e) = self.session.insert_output(&d.command.label, frags, origin) {
                        self.status = e.to_string();
                    }
                    open = false;
                }
                Err(e) => self.status = e,
            }
            self.mark_changed();
        }
        if open {
            self.plugin_dialog = Some(d);
        }
    }

    /// Parameters of a generator group; editing one runs the plugin again.
    pub(crate) fn generator_inspector(&mut self, ui: &mut egui::Ui, id: NodeId, generator: Generator) {
        let spec = self.project.plugin(&generator.plugin).and_then(|p| p.manifest.commands.iter().find(|c| c.id == generator.command).cloned());
        let Some(spec) = spec else {
            ui.colored_label(egui::Color32::YELLOW, format!("Made by {} / {}, which this SDK does not load.", generator.plugin, generator.command));
            ui.label("Its brushes stay as they are and can still be edited by hand.");
            return;
        };
        ui.label(egui::RichText::new(format!("{} parameters", spec.label)).strong());
        let mut params = generator.params.clone();
        let material = self.session.tex.material.clone();
        if !param_widgets(ui, ("gen", id.0), &spec.params, &mut params, &material) {
            return;
        }
        match self.run_plugin(&generator.plugin, &generator.command, &params) {
            Ok(frags) => {
                let g = Generator { params, ..generator };
                if let Err(e) = self.session.regenerate(id, g, frags, Some(&format!("gen:{}", id.0))) {
                    self.status = e.to_string();
                }
            }
            Err(e) => self.status = e,
        }
        self.mark_changed();
    }
}
