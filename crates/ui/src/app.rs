use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Instant;

use craft_settings::{Location, SettingsStore};
use eframe::egui_wgpu;
use egui::{Color32, Pos2, Rect, Vec2};
use mapcraft_engine::{Issue, Session};
use mapcraft_geom::{Aabb, DVec3};
use mapcraft_map::{Map, NodeId, NodeKind};
use mapcraft_render::{Camera, RenderMode, Renderer, Target, View, build_scene};
use mapcraft_sdk::Sdk;

use crate::prefs::Prefs;
use crate::project::{Project, sdk_search_dirs};
use crate::viewport::{Drag, Tool};

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub open: Option<PathBuf>,
    /// Save a screenshot of the window after a few frames and quit (used to check the UI).
    pub screenshot: Option<PathBuf>,
    /// Keep settings here instead of the user's config folder (tests and snapshots).
    pub settings_dir: Option<PathBuf>,
    /// SDK path settings that override the saved ones, as `name=value`.
    pub sdk_vars: Vec<(String, String)>,
}

pub(crate) struct Pane {
    pub camera: Camera,
    pub target: Option<Target>,
    pub texture: Option<egui::TextureId>,
    pub rect: Rect,
}

pub(crate) struct Gpu {
    pub renderer: Renderer,
    pub state: egui_wgpu::RenderState,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum LeftTab {
    Scene,
    Materials,
    Entities,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum BottomTab {
    Output,
    Issues,
}

enum Pending {
    New(Option<String>),
    Open(PathBuf),
    Quit,
}

struct CompileJob {
    rx: mpsc::Receiver<String>,
    cancel: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
}

pub struct MapCraftApp {
    pub(crate) session: Session,
    pub(crate) prefs: SettingsStore<Prefs>,
    pub(crate) sdks: Vec<(PathBuf, Result<Sdk, String>)>,
    pub(crate) project: Project,
    pub(crate) gpu: Option<Gpu>,
    pub(crate) panes: [Pane; 4],
    pub(crate) split: Vec2,
    pub(crate) maximized: Option<usize>,
    pub(crate) tool: Tool,
    pub(crate) shape: mapcraft_engine::Shape,
    pub(crate) drag: Drag,
    pub(crate) render_mode: RenderMode,
    pub(crate) scene_dirty: bool,
    pub(crate) status: String,
    pub(crate) output: Vec<String>,
    pub(crate) issues: Vec<Issue>,
    pub(crate) left_tab: LeftTab,
    pub(crate) bottom_tab: BottomTab,
    pub(crate) show_bottom: bool,
    pub(crate) material_filter: String,
    pub(crate) entity_filter: String,
    pub(crate) entity_class: String,
    pub(crate) expanded: BTreeSet<mapcraft_map::NodeId>,
    pub(crate) last_box: Aabb,
    pub(crate) clip_points: Vec<DVec3>,
    pub(crate) clip_keep: mapcraft_engine::ClipKeep,
    pub(crate) clip_view_axis: Option<DVec3>,
    pub(crate) cursor_world: Option<DVec3>,
    /// The map with prefab instances expanded, and which instance each expanded node belongs to.
    pub(crate) display: Option<(Map, BTreeMap<NodeId, NodeId>)>,
    prefab_cache: HashMap<String, Option<Map>>,
    show_new_dialog: bool,
    new_sdk: Option<String>,
    show_sdk_dialog: bool,
    pending: Option<Pending>,
    confirm_tools: Option<usize>,
    compile: Option<CompileJob>,
    last_autosave: Instant,
    screenshot: Option<PathBuf>,
    sdk_var_overrides: Vec<(String, String)>,
    frame_pending: bool,
    frame: u64,
    quit_confirmed: bool,
}

impl MapCraftApp {
    pub fn new(cc: &eframe::CreationContext<'_>, options: Options) -> Self {
        let (prefs, report) = match &options.settings_dir {
            Some(dir) => SettingsStore::<Prefs>::open_at(dir),
            None => SettingsStore::<Prefs>::open(Location::Auto),
        };
        if report.has_problems() {
            log::warn!("settings: {report:?}");
        }
        let grid = prefs.get().view.grid;
        let gpu = cc.wgpu_render_state.as_ref().map(|state| Gpu { renderer: Renderer::new(&state.device, &state.queue), state: state.clone() });
        let pane = |view| Pane { camera: Camera::new(view), target: None, texture: None, rect: Rect::NOTHING };
        let mut app = Self {
            session: Session::default(),
            prefs,
            sdks: Vec::new(),
            project: Project::default(),
            gpu,
            panes: [pane(View::Perspective), pane(View::Top), pane(View::Front), pane(View::Side)],
            split: Vec2::splat(0.5),
            maximized: None,
            tool: Tool::Select,
            shape: mapcraft_engine::Shape::Block,
            drag: Drag::None,
            render_mode: RenderMode::Textured,
            scene_dirty: true,
            status: String::new(),
            output: Vec::new(),
            issues: Vec::new(),
            left_tab: LeftTab::Scene,
            bottom_tab: BottomTab::Output,
            show_bottom: false,
            material_filter: String::new(),
            entity_filter: String::new(),
            entity_class: "info_player_start".into(),
            expanded: BTreeSet::from([mapcraft_map::ROOT]),
            last_box: Aabb::new(DVec3::ZERO, DVec3::splat(64.0)),
            clip_points: Vec::new(),
            clip_keep: mapcraft_engine::ClipKeep::Both,
            clip_view_axis: None,
            cursor_world: None,
            display: None,
            prefab_cache: HashMap::new(),
            show_new_dialog: false,
            new_sdk: None,
            show_sdk_dialog: false,
            pending: None,
            confirm_tools: None,
            compile: None,
            last_autosave: Instant::now(),
            screenshot: options.screenshot,
            sdk_var_overrides: options.sdk_vars,
            frame_pending: false,
            frame: 0,
            quit_confirmed: false,
        };
        app.session.grid = grid;
        app.rescan_sdks();
        match options.open {
            Some(path) => app.open_now(&path),
            None => {
                let last = app.prefs.get().last_sdk.clone();
                app.new_now(last);
            }
        }
        app
    }

    pub(crate) fn rescan_sdks(&mut self) {
        let dirs = sdk_search_dirs(&self.prefs.get().sdk_dirs);
        self.sdks = mapcraft_sdk::discover(&dirs).into_iter().map(|(p, r)| (p, r.map_err(|e| e.to_string()))).collect();
    }

    fn sdk_by_id(&self, id: &str) -> Option<Sdk> {
        self.sdks.iter().find_map(|(_, r)| r.as_ref().ok().filter(|s| s.manifest.id == id).cloned())
    }

    pub(crate) fn load_project(&mut self, sdk_id: Option<String>) {
        let sdk = sdk_id.as_deref().and_then(|id| self.sdk_by_id(id));
        if let (Some(id), None) = (&sdk_id, &sdk) {
            self.status = format!("SDK `{id}` is not installed; entities and materials are unavailable");
        }
        let mut vars = sdk_id.as_ref().and_then(|id| self.prefs.get().sdk_paths.get(id).cloned()).unwrap_or_default();
        vars.extend(self.sdk_var_overrides.iter().cloned());
        if let Some(s) = &sdk {
            for (k, setting) in &s.manifest.paths {
                if let (None, Some(d)) = (vars.get(k), &setting.default) {
                    vars.insert(k.clone(), d.clone());
                }
            }
            self.session.tex.scale = s.manifest.textures.scale;
            if let Some(d) = &s.manifest.textures.default {
                self.session.tex.material = d.clone();
            }
        }
        self.project = Project::load(sdk, vars);
        for p in &self.project.problems {
            self.output.push(p.clone());
        }
        let missing = self.project.missing_paths();
        if !missing.is_empty() {
            self.status = format!("Set {} in File > SDK Settings", missing.join(", "));
        }
        self.scene_dirty = true;
    }

    fn new_now(&mut self, sdk: Option<String>) {
        self.session = Session::default();
        self.session.grid = self.prefs.get().view.grid;
        self.session.sdk = sdk.clone();
        self.prefs.update(|p| p.last_sdk = sdk.clone());
        self.load_project(sdk);
        self.status = "New map".into();
    }

    pub(crate) fn open_now(&mut self, path: &Path) {
        match Session::open(path) {
            Ok(mut s) => {
                s.grid = self.prefs.get().view.grid;
                let sdk = s.sdk.clone();
                self.session = s;
                self.prefs.update(|p| p.push_recent(&path.to_string_lossy()));
                self.load_project(sdk);
                self.frame_pending = true;
                self.status = format!("Opened {}", path.display());
            }
            Err(e) => self.status = format!("Cannot open {}: {e}", path.display()),
        }
    }

    pub(crate) fn save(&mut self, save_as: bool) -> bool {
        let path = match (&self.session.path, save_as) {
            (Some(p), false) => p.clone(),
            _ => match rfd::FileDialog::new().add_filter("MapCraft map", &["mcmap"]).set_file_name("untitled.mcmap").save_file() {
                Some(p) => p.with_extension(mapcraft_format::EXTENSION),
                None => return false,
            },
        };
        match self.session.save_as(&path) {
            Ok(()) => {
                self.prefs.update(|p| p.push_recent(&path.to_string_lossy()));
                self.status = format!("Saved {}", path.display());
                true
            }
            Err(e) => {
                self.status = format!("Save failed: {e}");
                false
            }
        }
    }

    fn request(&mut self, action: Pending) {
        if self.session.is_dirty() {
            self.pending = Some(action);
        } else {
            self.run_pending(action);
        }
    }

    fn run_pending(&mut self, action: Pending) {
        match action {
            Pending::New(sdk) => self.new_now(sdk),
            Pending::Open(p) => self.open_now(&p),
            Pending::Quit => self.quit_confirmed = true,
        }
    }

    /// Centers every view on the map's bounds.
    pub(crate) fn frame_all(&mut self) {
        let bounds = self.view_map().nodes().filter_map(|(id, _)| self.leaf_bounds(id)).reduce(|a, b| a.union(&b));
        let Some(b) = bounds else { return };
        self.frame_bounds(b);
    }

    pub(crate) fn frame_bounds(&mut self, b: Aabb) {
        let c = b.center();
        let extent = b.size().max_element().max(64.0);
        for pane in &mut self.panes {
            let cam = &mut pane.camera;
            if cam.is_2d() {
                let (r, u, _) = cam.view.axes();
                cam.center = mapcraft_geom::DVec2::new(c.dot(r), c.dot(u));
                let size = pane.rect.size();
                let px = f64::from(size.x.min(size.y).max(100.0));
                cam.zoom = (px / (extent * 1.3)).clamp(1.0 / 256.0, 64.0);
            } else {
                cam.position = c - cam.forward() * extent * 1.2;
            }
        }
    }

    pub(crate) fn mark_changed(&mut self) {
        self.scene_dirty = true;
    }

    pub(crate) fn reload_prefabs(&mut self) {
        self.prefab_cache.clear();
        self.mark_changed();
    }

    fn update_display(&mut self) {
        if !self.session.map.nodes().any(|(_, n)| matches!(n.kind, NodeKind::Prefab(_))) {
            self.display = None;
            return;
        }
        let doc = self.session.path.clone();
        let (flat, owner, problems) = mapcraft_engine::flatten_prefabs(&self.session.map, &mut prefab_loader(&mut self.prefab_cache, doc));
        if let Some(p) = problems.first() {
            self.status = p.clone();
        }
        self.display = Some((flat, owner));
    }

    /// The map as drawn and picked: prefab instances expanded.
    pub(crate) fn view_map(&self) -> &Map {
        self.display.as_ref().map_or(&self.session.map, |(m, _)| m)
    }

    /// The node in the real map a drawn node belongs to.
    pub(crate) fn owner(&self, id: NodeId) -> NodeId {
        self.display.as_ref().and_then(|(_, o)| o.get(&id).copied()).unwrap_or(id)
    }

    fn rebuild_scene(&mut self) {
        self.update_display();
        let Some(gpu) = &mut self.gpu else { return };
        let map = self.display.as_ref().map_or(&self.session.map, |(m, _)| m);
        let names: BTreeSet<String> = map.nodes().filter_map(|(_, n)| n.kind.brush()).flat_map(|b| b.faces.iter().map(|f| f.material.clone())).collect();
        self.project.upload_materials(&mut gpu.renderer, names.iter().map(String::as_str));
        let renderer = &gpu.renderer;
        let scene = build_scene(map, &self.session.selection, &self.session.face_selection, &|m| renderer.material_size(m), &self.project);
        gpu.renderer.set_scene(&scene);
        self.scene_dirty = false;
    }

    /// Number of nodes in the map, for tests.
    pub fn node_count_for_test(&self) -> usize {
        self.session.map.len()
    }

    /// Runs an engine command by id, as a menu item would.
    pub fn run_command_for_test(&mut self, id: &str) {
        self.run_command(id);
    }

    pub(crate) fn run_command(&mut self, id: &str) {
        if let Err(e) = mapcraft_engine::execute(&mut self.session, id) {
            self.status = e.to_string();
        }
        self.mark_changed();
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let file = [
            (crate::shortcuts::parse("Ctrl+S"), "save"),
            (crate::shortcuts::parse("Ctrl+Shift+S"), "save_as"),
            (crate::shortcuts::parse("Ctrl+O"), "open"),
            (crate::shortcuts::parse("Ctrl+N"), "new"),
        ];
        // Longer chords first so Ctrl+Shift+S is not taken by Ctrl+S.
        for (sc, action) in file.iter().rev() {
            if let Some(sc) = sc
                && ctx.input_mut(|i| i.consume_shortcut(sc))
            {
                match *action {
                    "save" => {
                        self.save(false);
                    }
                    "save_as" => {
                        self.save(true);
                    }
                    "open" => self.open_dialog(),
                    _ => self.show_new_dialog = true,
                }
            }
        }
        let mut cmds: Vec<_> = mapcraft_engine::commands().iter().filter_map(|c| Some((crate::shortcuts::parse(c.shortcut?)?, c.id))).collect();
        cmds.sort_by_key(|(sc, _)| std::cmp::Reverse(sc.modifiers.shift as u8 + sc.modifiers.command as u8 + sc.modifiers.alt as u8));
        for (sc, id) in cmds {
            if ctx.input_mut(|i| i.consume_shortcut(&sc)) {
                self.run_command(id);
            }
        }
        self.clipboard(ctx);
        let keys = [
            (egui::Key::Escape, Tool::Select),
            (egui::Key::B, Tool::Block),
            (egui::Key::E, Tool::Entity),
            (egui::Key::X, Tool::Clip),
            (egui::Key::F, Tool::Face),
            (egui::Key::V, Tool::Vertex),
        ];
        for (key, tool) in keys {
            if ctx.input(|i| i.key_pressed(key) && i.modifiers.is_none()) {
                if key == egui::Key::Escape {
                    self.clip_points.clear();
                    self.drag = Drag::None;
                }
                self.tool = tool;
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Enter)) && self.tool == Tool::Clip && self.clip_points.len() == 2 {
            self.apply_clip();
        }
    }

    fn clipboard(&mut self, ctx: &egui::Context) {
        let events: Vec<egui::Event> = ctx.input(|i| i.events.clone());
        for ev in events {
            match ev {
                egui::Event::Copy | egui::Event::Cut => match self.session.copy_selection() {
                    Ok(text) => {
                        ctx.copy_text(text);
                        if matches!(ev, egui::Event::Cut) {
                            self.run_command("edit.delete");
                        }
                    }
                    Err(e) => self.status = e.to_string(),
                },
                egui::Event::Paste(text) => {
                    let g = self.session.grid;
                    if let Err(e) = self.session.paste(&text, DVec3::new(g, g, 0.0)) {
                        self.status = format!("Cannot paste: {e}");
                    }
                    self.mark_changed();
                }
                _ => {}
            }
        }
    }

    pub(crate) fn open_dialog(&mut self) {
        if let Some(p) = rfd::FileDialog::new().add_filter("MapCraft map", &["mcmap"]).pick_file() {
            self.request(Pending::Open(p));
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("New Map…").clicked() {
                    self.new_sdk = self.project.id().map(String::from);
                    self.show_new_dialog = true;
                }
                if ui.button("Open…").clicked() {
                    self.open_dialog();
                }
                ui.menu_button("Open Recent", |ui| {
                    for p in self.prefs.get().recent.clone() {
                        if ui.button(&p).clicked() {
                            self.request(Pending::Open(PathBuf::from(p)));
                        }
                    }
                });
                ui.separator();
                if ui.button("Save").clicked() {
                    self.save(false);
                }
                if ui.button("Save As…").clicked() {
                    self.save(true);
                }
                ui.separator();
                if ui.button("SDK Settings…").clicked() {
                    self.show_sdk_dialog = true;
                }
                ui.separator();
                if ui.button("Quit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            for top in ["Edit", "Tools", "View"] {
                ui.menu_button(top, |ui| {
                    if top == "Tools" {
                        self.prefab_menu(ui);
                        ui.separator();
                    }
                    if top == "View" {
                        for (mode, label) in [(RenderMode::Textured, "Textured"), (RenderMode::Flat, "Flat"), (RenderMode::Wireframe, "Wireframe")] {
                            if ui.radio(self.render_mode == mode, label).clicked() {
                                self.render_mode = mode;
                            }
                        }
                        if ui.button("Frame All").clicked() {
                            self.frame_all();
                        }
                        ui.separator();
                    }
                    for c in mapcraft_engine::commands().iter().filter(|c| c.menu.split('/').next() == Some(top)) {
                        let label = c.menu.rsplit('/').next().unwrap_or(c.label);
                        let enabled = (c.enabled)(&self.session);
                        let button = egui::Button::new(label).shortcut_text(c.shortcut.unwrap_or_default());
                        if ui.add_enabled(enabled, button).clicked() {
                            self.run_command(c.id);
                        }
                    }
                });
            }
            ui.menu_button("Run", |ui| {
                let profiles: Vec<String> = self.project.sdk.iter().flat_map(|s| s.manifest.compile.iter().map(|p| p.name.clone())).collect();
                if profiles.is_empty() {
                    ui.label("This SDK has no compile profiles");
                }
                for (i, name) in profiles.iter().enumerate() {
                    if ui.add_enabled(self.compile.is_none(), egui::Button::new(name)).clicked() {
                        self.start_compile(i);
                    }
                }
                if let Some(job) = &self.compile
                    && ui.button("Cancel").clicked()
                {
                    job.cancel.store(true, Ordering::Relaxed);
                }
            });
            if ui.button("Check for Problems").clicked() {
                self.issues = mapcraft_engine::validate(&self.session.map, self.project.sdk.as_ref().map(|_| &self.project.game));
                self.bottom_tab = BottomTab::Issues;
                self.show_bottom = true;
            }
        });
    }

    fn prefab_menu(&mut self, ui: &mut egui::Ui) {
        if ui.add_enabled(!self.session.selection.is_empty(), egui::Button::new("Save Selection as Prefab…")).clicked() {
            let mut dialog = rfd::FileDialog::new().add_filter("MapCraft map", &["mcmap"]).set_file_name("prefab.mcmap");
            if let Some(dir) = self.session.path.as_ref().and_then(|p| p.parent()) {
                dialog = dialog.set_directory(dir);
            }
            if let Some(path) = dialog.save_file() {
                match self.session.save_selection_as_prefab(&path.with_extension(mapcraft_format::EXTENSION)) {
                    Ok(_) => self.reload_prefabs(),
                    Err(e) => self.status = e.to_string(),
                }
            }
        }
        if ui.button("Insert Prefab…").clicked()
            && let Some(path) = rfd::FileDialog::new().add_filter("MapCraft map", &["mcmap"]).pick_file()
        {
            let stored = self.session.relative_path(&path);
            let at = self.cursor_world.map_or(DVec3::ZERO, |p| mapcraft_geom::snap(p, self.session.grid));
            if let Err(e) = self.session.insert_prefab(&stored, at) {
                self.status = e.to_string();
            }
            self.mark_changed();
        }
        if ui.add_enabled(!self.session.selection.is_empty(), egui::Button::new("Make Prefab Local")).clicked() {
            let doc = self.session.path.clone();
            if let Err(e) = self.session.make_local(&mut prefab_loader(&mut self.prefab_cache, doc)) {
                self.status = e.to_string();
            }
            self.mark_changed();
        }
        if ui.button("Reload Prefabs").clicked() {
            self.reload_prefabs();
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for (tool, label, tip) in [
                (Tool::Select, "Select", "Select and move (Esc)"),
                (Tool::Block, "Block", "Drag in a 2D view to create a brush (B)"),
                (Tool::Entity, "Entity", "Click to place the entity class chosen in the Entities tab (E)"),
                (Tool::Clip, "Clip", "Click two points in a 2D view, Enter to clip (X)"),
                (Tool::Vertex, "Vertex", "Drag the corners of selected brushes in a 2D view (V)"),
                (Tool::Face, "Face", "Click faces in the 3D view to texture them; Alt+click picks a material, Alt+right click applies it (F)"),
            ] {
                if ui.selectable_label(self.tool == tool, label).on_hover_text(tip).clicked() {
                    self.tool = tool;
                    self.clip_points.clear();
                }
            }
            ui.separator();
            use mapcraft_engine::SelectMode as M;
            for (mode, label) in [(M::Groups, "Groups"), (M::Objects, "Objects"), (M::Solids, "Solids")] {
                if ui.selectable_label(self.session.select_mode == mode, label).on_hover_text("Selection mode").clicked() {
                    self.session.select_mode = mode;
                }
            }
            ui.separator();
            if self.tool == Tool::Block {
                use mapcraft_engine::Shape as S;
                for (shape, label) in [(S::Block, "Block"), (S::Wedge, "Wedge"), (S::Cylinder(8), "Cylinder"), (S::Spike(4), "Spike")] {
                    let same = std::mem::discriminant(&self.shape) == std::mem::discriminant(&shape);
                    if ui.selectable_label(same, label).clicked() {
                        self.shape = match (shape, self.shape) {
                            (S::Cylinder(_), S::Cylinder(n)) | (S::Cylinder(_), S::Spike(n)) => S::Cylinder(n),
                            (S::Spike(_), S::Spike(n)) | (S::Spike(_), S::Cylinder(n)) => S::Spike(n),
                            (s, _) => s,
                        };
                    }
                }
                if let S::Cylinder(n) | S::Spike(n) = &mut self.shape {
                    ui.add(egui::DragValue::new(n).range(3..=64).prefix("sides "));
                }
                ui.separator();
            }
            if self.tool == Tool::Clip {
                use mapcraft_engine::ClipKeep as K;
                for (k, label) in [(K::Both, "Keep both"), (K::Front, "Keep front"), (K::Back, "Keep back")] {
                    if ui.selectable_label(self.clip_keep == k, label).clicked() {
                        self.clip_keep = k;
                    }
                }
                ui.separator();
            }
            ui.label(format!("Material: {}", if self.session.tex.material.is_empty() { "(none)" } else { &self.session.tex.material }));
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(&self.status);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.selectable_label(self.show_bottom, "Output").clicked() {
                    self.show_bottom = !self.show_bottom;
                }
                ui.separator();
                ui.label(format!("Grid {}", self.session.grid));
                ui.separator();
                ui.label(if self.session.texture_lock { "Texture lock on" } else { "Texture lock off" });
                ui.separator();
                ui.label(format!("{} selected", self.session.selection.len()));
                if let Some(p) = self.cursor_world {
                    ui.separator();
                    ui.label(format!("{:.0} {:.0} {:.0}", p.x, p.y, p.z));
                }
                ui.separator();
                ui.label(self.project.sdk.as_ref().map_or("No SDK".to_string(), |s| s.manifest.name.clone()));
            });
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if self.show_new_dialog {
            let mut open = true;
            egui::Window::new("New Map").collapsible(false).resizable(false).open(&mut open).show(ctx, |ui| {
                ui.label("SDK");
                let ids: Vec<(String, String)> =
                    self.sdks.iter().filter_map(|(_, r)| r.as_ref().ok()).map(|s| (s.manifest.id.clone(), s.manifest.name.clone())).collect();
                let current = self.new_sdk.clone();
                egui::ComboBox::from_id_salt("new-sdk")
                    .selected_text(ids.iter().find(|(i, _)| Some(i) == current.as_ref()).map_or("None", |(_, n)| n))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.new_sdk, None, "None");
                        for (id, name) in &ids {
                            ui.selectable_value(&mut self.new_sdk, Some(id.clone()), name);
                        }
                    });
                if ui.button("Create").clicked() {
                    let sdk = self.new_sdk.clone();
                    self.show_new_dialog = false;
                    self.request(Pending::New(sdk));
                }
            });
            self.show_new_dialog &= open;
        }
        if self.show_sdk_dialog {
            self.sdk_dialog(ctx);
        }
        if let Some(action) = self.pending.take() {
            let name = self.session.path.as_ref().and_then(|p| p.file_name()).map_or("Untitled".to_string(), |n| n.to_string_lossy().into_owned());
            let mut choice = None;
            egui::Modal::new(egui::Id::new("save-changes")).show(ctx, |ui| {
                ui.heading(format!("Save changes to {name}?"));
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        choice = Some(1);
                    }
                    if ui.button("Don't Save").clicked() {
                        choice = Some(2);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(3);
                    }
                });
            });
            match choice {
                Some(1) if self.save(false) => self.run_pending(action),
                Some(2) => self.run_pending(action),
                Some(_) => {}
                None => self.pending = Some(action),
            }
        }
        if let Some(i) = self.confirm_tools {
            let mut choice = None;
            egui::Modal::new(egui::Id::new("confirm-tools")).show(ctx, |ui| {
                ui.heading("Run this SDK's programs?");
                ui.label("Compile profiles run programs named by the SDK. Only allow this for SDKs you trust.");
                ui.horizontal(|ui| {
                    if ui.button("Allow").clicked() {
                        choice = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(false);
                    }
                });
            });
            match choice {
                Some(true) => {
                    if let Some(id) = self.project.id().map(String::from) {
                        self.prefs.update(|p| {
                            p.approved_tools.insert(id);
                        });
                    }
                    self.confirm_tools = None;
                    self.start_compile(i);
                }
                Some(false) => self.confirm_tools = None,
                None => {}
            }
        }
    }

    fn sdk_dialog(&mut self, ctx: &egui::Context) {
        let mut open = true;
        let Some(sdk) = self.project.sdk.clone() else {
            egui::Window::new("SDK Settings").open(&mut open).show(ctx, |ui| {
                ui.label("This map has no SDK. Pick one with File > New Map.");
                ui.label("SDK folders searched:");
                for d in sdk_search_dirs(&self.prefs.get().sdk_dirs) {
                    ui.monospace(d.display().to_string());
                }
            });
            self.show_sdk_dialog &= open;
            return;
        };
        let mut changed = false;
        egui::Window::new(format!("{} SDK Settings", sdk.manifest.name)).open(&mut open).show(ctx, |ui| {
            egui::Grid::new("sdk-paths").num_columns(3).show(ui, |ui| {
                for (key, setting) in &sdk.manifest.paths {
                    ui.label(&setting.prompt);
                    let mut value = self.project.vars.get(key).cloned().unwrap_or_default();
                    if ui.add(egui::TextEdit::singleline(&mut value).desired_width(320.0)).changed() {
                        self.project.vars.insert(key.clone(), value.clone());
                        changed = true;
                    }
                    if ui.button("Browse…").clicked() {
                        let picked = if setting.kind == "file" { rfd::FileDialog::new().pick_file() } else { rfd::FileDialog::new().pick_folder() };
                        if let Some(p) = picked {
                            self.project.vars.insert(key.clone(), p.to_string_lossy().into_owned());
                            changed = true;
                        }
                    }
                    ui.end_row();
                }
            });
            if ui.button("Reload SDK").clicked() {
                let id = sdk.manifest.id.clone();
                self.load_project(Some(id));
            }
        });
        if changed {
            let (id, vars) = (sdk.manifest.id.clone(), self.project.vars.clone());
            self.prefs.update(|p| {
                p.sdk_paths.insert(id, vars);
            });
        }
        self.show_sdk_dialog &= open;
        if !self.show_sdk_dialog && changed {
            self.load_project(Some(sdk.manifest.id.clone()));
        }
    }

    fn start_compile(&mut self, index: usize) {
        let Some(sdk) = self.project.sdk.clone() else { return };
        let Some(profile) = sdk.manifest.compile.get(index).cloned() else { return };
        if !self.prefs.get().approved_tools.contains(&sdk.manifest.id) {
            self.confirm_tools = Some(index);
            return;
        }
        if !self.save(false) {
            return;
        }
        let Some(path) = self.session.path.clone() else { return };
        let mut vars = self.project.vars.clone();
        vars.extend(mapcraft_sdk::builtin_vars(&path, &sdk.dir));
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicBool::new(false));
        let (c, d) = (cancel.clone(), done.clone());
        self.output.push(format!("== {} ==", profile.name));
        std::thread::spawn(move || {
            let result = mapcraft_sdk::run_profile(
                &profile,
                &vars,
                &mut |line| {
                    let _ = tx.send(line.to_string());
                },
                &c,
            );
            let _ = tx.send(match result {
                Ok(()) => "== finished ==".to_string(),
                Err(e) => format!("== failed: {e} =="),
            });
            d.store(true, Ordering::Relaxed);
        });
        self.compile = Some(CompileJob { rx, cancel, done });
        self.show_bottom = true;
        self.bottom_tab = BottomTab::Output;
    }

    fn poll_compile(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.compile else { return };
        self.output.extend(job.rx.try_iter());
        if job.done.load(Ordering::Relaxed) {
            self.output.extend(job.rx.try_iter());
            self.compile = None;
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    fn autosave(&mut self) {
        let minutes = self.prefs.get().view.autosave_minutes;
        if minutes == 0 || !self.session.is_dirty() || self.last_autosave.elapsed().as_secs() < u64::from(minutes) * 60 {
            return;
        }
        self.last_autosave = Instant::now();
        let Some(dir) = craft_settings::paths::data_dir("mapcraft").map(|d| d.join("autosave")) else { return };
        let stem = self.session.path.as_ref().and_then(|p| p.file_stem()).map_or("untitled".to_string(), |s| s.to_string_lossy().into_owned());
        let path = dir.join(format!("{stem}.autosave.mcmap"));
        let result = std::fs::create_dir_all(&dir)
            .map_err(|e| e.to_string())
            .and_then(|()| mapcraft_format::save(&path, &self.session.map, self.session.sdk.as_deref(), &self.session.view).map_err(|e| e.to_string()));
        if let Err(e) = result {
            self.status = format!("Autosave failed: {e}");
        }
    }

    fn central(&mut self, ui: &mut egui::Ui) {
        let full = ui.available_rect_before_wrap();
        ui.painter().rect_filled(full, 0.0, Color32::from_gray(24));
        if let Some(m) = self.maximized {
            self.pane_ui(ui, m, full);
            return;
        }
        let gap = 4.0;
        let split = Pos2::new(full.left() + full.width() * self.split.x, full.top() + full.height() * self.split.y);
        let rects = [
            Rect::from_min_max(full.min, split - Vec2::splat(gap / 2.0)),
            Rect::from_min_max(Pos2::new(split.x + gap / 2.0, full.top()), Pos2::new(full.right(), split.y - gap / 2.0)),
            Rect::from_min_max(Pos2::new(full.left(), split.y + gap / 2.0), Pos2::new(split.x - gap / 2.0, full.bottom())),
            Rect::from_min_max(split + Vec2::splat(gap / 2.0), full.max),
        ];
        for (i, r) in rects.into_iter().enumerate() {
            self.pane_ui(ui, i, r);
        }
        let handle = Rect::from_center_size(split, Vec2::splat(gap * 3.0));
        let v = ui.interact(Rect::from_x_y_ranges(split.x - gap..=split.x + gap, full.y_range()), ui.id().with("split-x"), egui::Sense::drag());
        let h = ui.interact(Rect::from_x_y_ranges(full.x_range(), split.y - gap..=split.y + gap), ui.id().with("split-y"), egui::Sense::drag());
        let c = ui.interact(handle, ui.id().with("split-xy"), egui::Sense::drag());
        if let Some(p) = ui.ctx().pointer_interact_pos() {
            if v.dragged() || c.dragged() {
                self.split.x = ((p.x - full.left()) / full.width()).clamp(0.1, 0.9);
            }
            if h.dragged() || c.dragged() {
                self.split.y = ((p.y - full.top()) / full.height()).clamp(0.1, 0.9);
            }
        }
        if v.hovered() || v.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        } else if h.hovered() || h.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }
    }

    fn pane_ui(&mut self, ui: &mut egui::Ui, index: usize, rect: Rect) {
        let header = Rect::from_min_size(rect.min, Vec2::new(rect.width(), 20.0));
        let body = Rect::from_min_max(Pos2::new(rect.left(), header.bottom()), rect.max);
        let mut header_ui = ui.new_child(egui::UiBuilder::new().max_rect(header).layout(egui::Layout::left_to_right(egui::Align::Center)));
        let Some(pane) = self.panes.get_mut(index) else { return };
        egui::ComboBox::from_id_salt(("view", index)).selected_text(pane.camera.view.label()).width(110.0).show_ui(&mut header_ui, |ui| {
            for v in View::ALL {
                if ui.selectable_value(&mut pane.camera.view, v, v.label()).changed() {
                    pane.camera = Camera { view: v, ..pane.camera };
                }
            }
        });
        let label = if self.maximized.is_some() { "Restore" } else { "Maximize" };
        if header_ui.small_button(label).clicked() {
            self.maximized = if self.maximized.is_some() { None } else { Some(index) };
        }
        self.viewport(ui, index, body);
    }

    fn take_screenshot(&mut self, ctx: &egui::Context) {
        let Some(path) = self.screenshot.clone() else { return };
        self.frame += 1;
        if self.frame == 10 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        let shot = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = shot {
            let [w, h] = img.size;
            let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
            if let Some(buf) = image::RgbaImage::from_raw(w as u32, h as u32, bytes)
                && let Err(e) = buf.save(&path)
            {
                log::error!("screenshot: {e}");
            }
            self.screenshot = None;
            self.quit_confirmed = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint();
    }

    fn title(&self) -> String {
        let name = self.session.path.as_ref().and_then(|p| p.file_name()).map_or("Untitled".to_string(), |n| n.to_string_lossy().into_owned());
        format!("{}{name} - MapCraft", if self.session.is_dirty() { "*" } else { "" })
    }
}

/// Loads prefabs by the path stored in an instance, relative to the map at `doc`, once each.
fn prefab_loader(cache: &mut HashMap<String, Option<Map>>, doc: Option<PathBuf>) -> impl FnMut(&str) -> Option<Map> + '_ {
    move |stored| {
        cache
            .entry(stored.to_string())
            .or_insert_with(|| craft_docpath::resolve_absolute(stored, doc.as_deref()).ok().and_then(|p| mapcraft_format::load(&p).ok()).map(|f| f.map))
            .clone()
    }
}

impl eframe::App for MapCraftApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx.input(|i| i.viewport().close_requested()) && !self.quit_confirmed && self.session.is_dirty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending = Some(Pending::Quit);
        }
        if self.quit_confirmed {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.prefs.tick(Instant::now());
        self.autosave();
        self.poll_compile(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.take_screenshot(&ctx);
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.title()));
        self.handle_shortcuts(&ctx);
        if self.scene_dirty {
            self.rebuild_scene();
        }
        egui::Panel::top("menu").show(ui, |ui| {
            self.menu_bar(ui);
            self.toolbar(ui);
        });
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        if self.show_bottom {
            egui::Panel::bottom("output").resizable(true).default_size(160.0).show(ui, |ui| self.bottom_panel(ui));
        }
        egui::Panel::left("left").resizable(true).default_size(260.0).show(ui, |ui| self.left_panel(ui));
        egui::Panel::right("inspector").resizable(true).default_size(300.0).show(ui, |ui| self.inspector(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| self.central(ui));
        self.dialogs(&ctx);
        if self.frame_pending && self.panes.iter().all(|p| p.rect.is_positive()) {
            self.frame_pending = false;
            self.frame_all();
        }
        if self.scene_dirty {
            ctx.request_repaint();
        }
    }

    fn on_exit(&mut self) {
        let grid = self.session.grid;
        self.prefs.update(|p| p.view.grid = grid);
        if let Err(e) = self.prefs.flush() {
            log::error!("saving settings: {e}");
        }
    }
}
