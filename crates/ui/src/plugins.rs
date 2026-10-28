use egui::{Align, Layout};
use mapcraft_geom::DVec3;
use mapcraft_map::{Generator, NodeId};
use mapcraft_plugin_host::{CommandSpec, Context};
use serde_json::Value;

use crate::app::MapCraftApp;
use crate::chrome::icon_button;
use crate::inspect::{self, Note, Prop, Section, Title};

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

impl MapCraftApp {
    fn plugin_context(&self) -> Context {
        Context {
            grid: self.session.grid,
            material: self.session.tex.material.clone(),
            scale: self.session.tex.scale,
            game: self.project.sdk.as_ref().map(|s| s.manifest.name.clone()).unwrap_or_default(),
        }
    }

    /// Runs a command of a loaded plugin, passing its log lines on to the Output panel.
    pub(crate) fn run_plugin(&mut self, plugin: &str, command: &str, params: &Value) -> Result<Vec<mapcraft_format::Fragment>, String> {
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
                if ui.add(crate::icons::button(Some("jigsaw_piece"), format!("{label}…"))).clicked() {
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
        let (project, thumbs) = (&self.project, &mut self.thumbs);
        let mut thumb = |name: &str| inspect::material_thumb(project, thumbs, ctx, name);
        egui::Window::new(&d.command.label).open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
            ui.set_width(320.0);
            egui::ScrollArea::vertical().max_height(480.0).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = inspect::ROW_GAP;
                inspect::param_rows(ui, ("dialog", &d.command.id), &d.command.params, &mut d.params, &material, &mut thumb);
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
            ui.menu_button((crate::icons::slot(Some("file_arrow_down")), "Import"), |ui| {
                for (plugin, importer) in importers {
                    if ui.add(crate::icons::button(None, format!("{}…", importer.label))).clicked() {
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
            ui.menu_button((crate::icons::slot(Some("file_arrow_up")), "Export"), |ui| {
                for exporter in exporters {
                    if ui.add(crate::icons::button(None, format!("{}…", exporter.label))).clicked() {
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

    /// The map, with prefabs and mirrors resolved, as the exporter `id` writes it; what the plugin says goes in `log`.
    pub(crate) fn export_bytes(&mut self, id: &str, log: &mut Vec<String>) -> Result<Vec<u8>, String> {
        let ctx = self.plugin_context();
        let name = self.session.path.as_ref().and_then(|p| p.file_stem()).map_or("untitled".into(), |s| s.to_string_lossy().into_owned());
        let map = self.view_map().clone();
        let plugin = self
            .project
            .plugins
            .iter_mut()
            .find(|p| p.manifest.exporters.iter().any(|e| e.id == id))
            .ok_or_else(|| format!("no loaded plugin exports `{id}`"))?;
        let result = plugin.export(id, &map, &name, &ctx).map_err(|e| e.to_string());
        log.extend(plugin.take_log());
        result
    }

    /// Writes the map, with prefabs and mirrors resolved, through the exporter `id` into `path`.
    pub(crate) fn export_to(&mut self, id: &str, path: &std::path::Path) -> Result<(), String> {
        let mut log = Vec::new();
        let result = self.export_bytes(id, &mut log);
        self.output.extend(log);
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
        if let Some(path) = rfd::FileDialog::new().add_filter(&importer.label, &importer.extensions).pick_file() {
            self.import_path(plugin, importer, &path);
        }
    }

    /// The loaded plugin and importer that read files with `path`'s extension.
    pub(crate) fn importer_for(&self, path: &std::path::Path) -> Option<(String, mapcraft_plugin_host::ImporterSpec)> {
        let ext = path.extension()?.to_str()?;
        self.project.plugins.iter().find_map(|p| {
            let reads = |i: &&mapcraft_plugin_host::ImporterSpec| i.extensions.iter().any(|e| e.trim_start_matches('.').eq_ignore_ascii_case(ext));
            Some((p.manifest.id.clone(), p.manifest.importers.iter().find(reads)?.clone()))
        })
    }

    /// Reads `path` with `importer` into a new layer.
    pub(crate) fn import_path(&mut self, plugin: &str, importer: &mapcraft_plugin_host::ImporterSpec, path: &std::path::Path) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let data = match std::fs::read(path) {
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
                self.output.push(format!("Cannot import {name}: {e}"));
                self.show_bottom = true;
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

    pub(crate) fn shape_header(&mut self, ui: &mut egui::Ui) {
        let Some(shape) = &self.plugin_shape else { return };
        let label = shape.command.label.clone();
        inspect::header(ui, ("magic_wand", crate::icons::GENERATOR), Title::Fixed(&label), "", &[], |ui| {
            inspect::weak(ui, "Click in a view to place it");
        });
    }

    /// The picked generator's placement and parameters while the block tool is placing it.
    pub(crate) fn shape_body(&mut self, ui: &mut egui::Ui) {
        let Some(mut shape) = self.plugin_shape.take() else { return };
        if self.section(ui, &Section { key: "shape.placement", title: "Placement", ..Default::default() }) {
            inspect::row(ui, &Prop { label: "Facing", key: "yaw", tip: "Degrees the shape turns about Z", ..Default::default() }, |ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if icon_button(ui, "arrow_counterclockwise", "Turn 90°", 22.0, false).on_hover_text("Turn a quarter turn counter-clockwise").clicked() {
                        shape.yaw = (shape.yaw + 90.0) % 360.0;
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| inspect::num(ui, &mut shape.yaw, -360.0..=360.0, 15.0, "°", None)).inner
                })
                .inner
            });
        }
        if self.section(ui, &Section { key: "shape.params", title: "Parameters", ..Default::default() }) {
            let material = self.session.tex.material.clone();
            let ctx = ui.ctx().clone();
            let (project, thumbs) = (&self.project, &mut self.thumbs);
            let mut thumb = |name: &str| inspect::material_thumb(project, thumbs, &ctx, name);
            inspect::param_rows(ui, ("shape", &shape.command.id), &shape.command.params, &mut shape.params, &material, &mut thumb);
        }
        self.plugin_shape = Some(shape);
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
    pub(crate) fn generator_section(&mut self, ui: &mut egui::Ui, id: NodeId, generator: Generator) {
        let spec = self.project.plugin(&generator.plugin).and_then(|p| p.manifest.commands.iter().find(|c| c.id == generator.command).cloned());
        let Some(spec) = spec else {
            let text = format!("Made by {} / {}, which this SDK does not load. Its brushes stay editable by hand.", generator.plugin, generator.command);
            inspect::note(ui, Note::Warn, &text);
            return;
        };
        let tip = format!("Made by {}; changing a value runs it again", generator.plugin);
        let section = Section { key: "generator", title: &spec.label, tip: &tip, icon: Some(("magic_wand", crate::icons::GENERATOR)), ..Default::default() };
        if !self.section(ui, &section) {
            return;
        }
        let mut params = generator.params.clone();
        let material = self.session.tex.material.clone();
        let ctx = ui.ctx().clone();
        let (project, thumbs) = (&self.project, &mut self.thumbs);
        let mut thumb = |name: &str| inspect::material_thumb(project, thumbs, &ctx, name);
        if !inspect::param_rows(ui, ("gen", id.0), &spec.params, &mut params, &material, &mut thumb) {
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
