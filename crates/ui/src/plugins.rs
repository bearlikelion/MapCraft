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

/// A plugin generator picked as the block tool's shape: previewed under the pointer, placed with a click.
pub(crate) struct PluginShape {
    pub plugin: String,
    pub command: CommandSpec,
    pub params: Value,
    pub yaw: f64,
    /// Edges of the output for the parameters it was made from, around the generator's origin.
    ghost: Option<(Value, Vec<[DVec3; 2]>)>,
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

    /// File > Import: one entry per importer of the loaded plugins.
    pub(crate) fn import_menu(&mut self, ui: &mut egui::Ui) {
        let importers: Vec<(String, mapcraft_plugin_host::ImporterSpec)> =
            self.project.plugins.iter().flat_map(|p| p.manifest.importers.iter().map(|i| (p.manifest.id.clone(), i.clone()))).collect();
        ui.add_enabled_ui(!importers.is_empty(), |ui| {
            ui.menu_button("Import", |ui| {
                for (plugin, importer) in importers {
                    if ui.button(format!("{}…", importer.label)).clicked() {
                        self.import_file(&plugin, &importer);
                    }
                }
            })
        })
        .response
        .on_disabled_hover_text("This SDK's plugins import no file formats");
    }

    /// File > Export: one entry per exporter of the loaded plugins.
    pub(crate) fn export_menu(&mut self, ui: &mut egui::Ui) {
        let exporters: Vec<mapcraft_plugin_host::ImporterSpec> = self.project.plugins.iter().flat_map(|p| p.manifest.exporters.clone()).collect();
        ui.add_enabled_ui(!exporters.is_empty(), |ui| {
            ui.menu_button("Export", |ui| {
                for exporter in exporters {
                    if ui.button(format!("{}…", exporter.label)).clicked() {
                        self.export_dialog(&exporter);
                    }
                }
            })
        })
        .response
        .on_disabled_hover_text("This SDK's plugins export no file formats");
    }

    fn export_dialog(&mut self, exporter: &mapcraft_plugin_host::ImporterSpec) {
        let stem = self.session.path.as_ref().and_then(|p| p.file_stem()).map_or("untitled".into(), |s| s.to_string_lossy().into_owned());
        let ext = exporter.extensions.first().cloned().unwrap_or_default();
        let mut dialog = rfd::FileDialog::new().add_filter(&exporter.label, &exporter.extensions).set_file_name(format!("{stem}.{ext}"));
        if let Some(dir) = self.session.path.as_ref().and_then(|p| p.parent()) {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.save_file() else { return };
        match self.export_to(&exporter.id, &path) {
            Ok(()) => self.status = format!("Exported {}", path.display()),
            Err(e) => self.status = e,
        }
    }

    /// Writes the map, with prefabs and mirrors resolved, through the exporter `id` into `path`.
    pub(crate) fn export_to(&mut self, id: &str, path: &std::path::Path) -> Result<(), String> {
        let ctx = self.plugin_context();
        let name = self.session.path.as_ref().and_then(|p| p.file_stem()).map_or("untitled".into(), |s| s.to_string_lossy().into_owned());
        let map = self.view_map().clone();
        let displaced = map.nodes().filter_map(|(_, n)| n.kind.brush()).filter(|b| b.has_displacement()).count();
        let connections: usize = map.nodes().filter_map(|(_, n)| n.kind.entity()).map(|e| e.connections.len()).sum();
        if displaced + connections > 0 {
            self.output.push(format!("Exporters do not get displacements or entity outputs yet: {displaced} displaced brushes are written flat and {connections} outputs are left out"));
        }
        let plugin = self
            .project
            .plugins
            .iter_mut()
            .find(|p| p.manifest.exporters.iter().any(|e| e.id == id))
            .ok_or_else(|| format!("no loaded plugin exports `{id}`"))?;
        let result = plugin.export(id, &map, &name, &ctx).map_err(|e| e.to_string());
        self.output.extend(plugin.take_log());
        let bytes = result?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        }
        // Exports can land on files made by other tools, such as a game's curated zone files.
        if let Ok(old) = std::fs::read(path)
            && old != bytes
        {
            let mut backup = path.as_os_str().to_owned();
            backup.push(".bak");
            std::fs::write(&backup, old).map_err(|e| format!("cannot keep a backup of {}: {e}", path.display()))?;
            self.output.push(format!("Kept the previous {} as {}", path.display(), std::path::Path::new(&backup).display()));
        }
        std::fs::write(path, bytes).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        self.output.push(format!("Exported {}", path.display()));
        Ok(())
    }

    /// Picks a file for `importer` and puts what it reads in a new layer.
    fn import_file(&mut self, plugin: &str, importer: &mapcraft_plugin_host::ImporterSpec) {
        let Some(path) = rfd::FileDialog::new().add_filter(&importer.label, &importer.extensions).pick_file() else { return };
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let data = match std::fs::read(&path) {
            Ok(d) => d,
            Err(e) => {
                self.status = format!("cannot read {}: {e}", path.display());
                return;
            }
        };
        let ctx = self.plugin_context();
        let Some(p) = self.project.plugin(plugin) else { return };
        let result = p.import(&importer.id, &name, &data, &ctx).map_err(|e| e.to_string());
        self.output.extend(p.take_log());
        let imported = match result {
            Ok(i) => i,
            Err(e) => {
                self.status = e;
                return;
            }
        };
        if let Some(dir) = self.material_cache() {
            let saved = self.project.cache_materials(&dir, &imported.materials);
            if saved > 0 {
                self.output.push(format!("Kept {saved} material pictures from {name} in {}", dir.display()));
            }
        }
        match self.session.import_layer(&name, imported.nodes) {
            Ok(_) => {
                self.status = format!("Imported {name}");
                self.mark_changed();
                self.frame_all();
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    /// Generator commands of the loaded plugins, offered as block tool shapes.
    pub(crate) fn generator_shapes(&self) -> Vec<(String, CommandSpec)> {
        self.project.plugins.iter().flat_map(|p| p.manifest.commands.iter().filter(|c| c.generator).map(|c| (p.manifest.id.clone(), c.clone()))).collect()
    }

    /// Picks a generator as the block shape, or a built-in shape with `None`, keeping each generator's parameters.
    pub(crate) fn pick_plugin_shape(&mut self, pick: Option<(String, CommandSpec)>) {
        if let Some(old) = self.plugin_shape.take() {
            self.plugin_params.insert(format!("{}/{}", old.plugin, old.command.id), old.params);
        }
        self.plugin_shape = pick.map(|(plugin, command)| {
            let params = self.plugin_params.get(&format!("{plugin}/{}", command.id)).cloned().unwrap_or_else(|| command.defaults());
            PluginShape { plugin, command, params, yaw: 0.0, ghost: None }
        });
    }

    /// The picked generator's parameters, shown in the inspector while the block tool is placing it.
    pub(crate) fn plugin_shape_inspector(&mut self, ui: &mut egui::Ui) {
        let material = self.session.tex.material.clone();
        let Some(shape) = &mut self.plugin_shape else { return };
        ui.label(egui::RichText::new(&shape.command.label).heading());
        ui.label(egui::RichText::new("Click in a view to place it; it stays editable from the inspector.").weak().small());
        ui.horizontal(|ui| {
            ui.label("Turn");
            ui.add(egui::DragValue::new(&mut shape.yaw).range(-360.0..=360.0).speed(15.0).suffix("°"));
            if ui.small_button("90°").on_hover_text("Turn a quarter turn counter-clockwise").clicked() {
                shape.yaw = (shape.yaw + 90.0) % 360.0;
            }
        });
        ui.separator();
        param_widgets(ui, ("shape", &shape.command.id), &shape.command.params, &mut shape.params, &material);
        self.refresh_ghost();
        if self.ghost_at.is_none() {
            self.ghost_at = Some((0, self.placement_point()));
        }
    }

    /// Runs the picked generator again when its parameters changed since the ghost was made.
    pub(crate) fn refresh_ghost(&mut self) {
        let Some(shape) = &self.plugin_shape else { return };
        if shape.ghost.as_ref().is_some_and(|(p, _)| *p == shape.params) {
            return;
        }
        let (plugin, command, params) = (shape.plugin.clone(), shape.command.id.clone(), shape.params.clone());
        let edges = match self.run_plugin(&plugin, &command, &params) {
            Ok(frags) => fragment_edges(&frags),
            Err(e) => {
                self.status = e;
                Vec::new()
            }
        };
        if let Some(shape) = &mut self.plugin_shape {
            shape.ghost = Some((params, edges));
        }
    }

    /// The ghost's edges placed at `at`, for the overlay.
    pub(crate) fn ghost_lines(&self, at: DVec3) -> Vec<[DVec3; 2]> {
        let Some(shape) = &self.plugin_shape else { return Vec::new() };
        let m = mapcraft_engine::generator_transform(&Self::shape_generator(shape, at));
        shape.ghost.iter().flat_map(|(_, e)| e).map(|[a, b]| [m.transform_point3(*a), m.transform_point3(*b)]).collect()
    }

    fn shape_generator(shape: &PluginShape, origin: DVec3) -> Generator {
        Generator {
            plugin: shape.plugin.clone(),
            command: shape.command.id.clone(),
            params: shape.params.clone(),
            origin,
            angles: DVec3::new(0.0, shape.yaw, 0.0),
        }
    }

    /// Places the picked generator at `at` as a group that stays editable.
    pub(crate) fn place_plugin_shape(&mut self, at: DVec3) {
        let Some(shape) = &self.plugin_shape else { return };
        let (g, label) = (Self::shape_generator(shape, at), shape.command.label.clone());
        match self.run_plugin(&g.plugin, &g.command, &g.params) {
            Ok(frags) => {
                if let Err(e) = self.session.create_generator(&label, g, frags) {
                    self.status = e.to_string();
                }
            }
            Err(e) => self.status = e,
        }
        self.mark_changed();
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

/// Every edge of the brushes in `frags`, for drawing them as wireframe.
fn fragment_edges(frags: &[mapcraft_format::Fragment]) -> Vec<[DVec3; 2]> {
    let mut out = Vec::new();
    let mut stack: Vec<&mapcraft_format::Fragment> = frags.iter().collect();
    while let Some(f) = stack.pop() {
        stack.extend(&f.children);
        let Some(solid) = f.kind.brush().and_then(|b| b.solid().ok()) else { continue };
        for w in solid.faces.iter().flatten() {
            out.extend(w.0.iter().zip(w.0.iter().cycle().skip(1)).map(|(a, b)| [*a, *b]));
        }
    }
    out
}
