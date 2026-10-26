use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Instant;

use craft_settings::{Location, SettingsStore};
use eframe::egui_wgpu;
use egui::{Pos2, Rect, Vec2};
use mapcraft_engine::{Issue, SelectMode, Session};
use mapcraft_geom::{Aabb, DVec3};
use mapcraft_map::{Map, NodeId, NodeKind};
use mapcraft_render::{Camera, RenderMode, Renderer, Target, View, build_scene};
use mapcraft_sdk::Sdk;

use crate::dialogs::Dialogs;
use crate::prefs::Prefs;
use crate::project::Project;
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
    /// Set while the pane shows the UV editor instead of its camera.
    pub uv: Option<crate::uv::UvCamera>,
}

pub(crate) struct Gpu {
    pub renderer: Renderer,
    pub state: egui_wgpu::RenderState,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum LeftTab {
    Scene,
    Tags,
    Materials,
    Entities,
    Models,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum BottomTab {
    Output,
    Issues,
    Find,
}

enum Pending {
    New(Option<String>),
    Open(PathBuf),
    Quit,
}

pub(crate) struct CompileJob {
    rx: mpsc::Receiver<String>,
    cancel: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
}

pub struct MapCraftApp {
    pub(crate) session: Session,
    pub(crate) prefs: SettingsStore<Prefs>,
    pub(crate) sdks: Vec<mapcraft_sdk_manager::Installed>,
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
    /// Scene tree rows being dragged, empty when none are.
    pub(crate) tree_drag: Vec<mapcraft_map::NodeId>,
    /// The selection the scene tree last unfolded and scrolled to.
    pub(crate) tree_revealed: BTreeSet<mapcraft_map::NodeId>,
    /// The scene tree's scroll offset and visible height last frame.
    pub(crate) tree_view: (f32, f32),
    pub(crate) last_box: Aabb,
    pub(crate) clip_points: Vec<DVec3>,
    pub(crate) clip_keep: mapcraft_engine::ClipKeep,
    /// Face drags keep the face's shape and push it along its normal, instead of stretching its neighbors.
    pub(crate) extrude_along_normal: bool,
    pub(crate) clip_view_axis: Option<DVec3>,
    pub(crate) cursor_world: Option<DVec3>,
    /// The map with prefab instances and mirror halves expanded, and the real node each added node belongs to.
    pub(crate) display: Option<(Map, BTreeMap<NodeId, NodeId>)>,
    pub(crate) prefab_cache: HashMap<String, Option<Map>>,
    pub(crate) dialogs: Dialogs,
    /// Where autosaves, the crash marker and user-installed SDKs live.
    pub(crate) data_dir: Option<PathBuf>,
    /// The view and spot of the last click, so clicking it again drills to the next object.
    pub(crate) last_click: Option<(usize, Pos2)>,
    /// Face whose material and alignment Alt+Shift+right click applies.
    pub(crate) picked_face: Option<mapcraft_map::Face>,
    pub(crate) thumbs: crate::thumbs::Thumbs,
    /// Larger pictures of materials for UV views.
    pub(crate) uv_images: crate::thumbs::Thumbs,
    pub(crate) uv_modal: Option<crate::uv::UvModal>,
    /// A UV transform ran as this frame began, so views ignore the clicks that end it.
    pub(crate) uv_busy: bool,
    pub(crate) find: crate::panels::FindState,
    pub(crate) tag_name: String,
    pub(crate) show_thumbs: bool,
    /// Compiles plugins; `None` when the WebAssembly engine could not start.
    plugin_host: Option<mapcraft_plugin_host::Host>,
    pub(crate) plugin_dialog: Option<crate::plugins::PluginDialog>,
    /// A plugin generator picked as the block tool's shape.
    pub(crate) plugin_shape: Option<crate::plugins::PluginShape>,
    /// Parameters last used for each generator shape, by `plugin/command`.
    pub(crate) plugin_params: BTreeMap<String, serde_json::Value>,
    /// Where a click would place the plugin shape, and in which view.
    pub(crate) ghost_at: Option<(usize, DVec3)>,
    /// The sculpt brush: mode, radius, and height per second at full strength.
    pub(crate) sculpt: (mapcraft_engine::SculptMode, f64, f64),
    /// The displaced surface under the sculpt brush: view, point and normal.
    pub(crate) sculpt_at: Option<(usize, DVec3, DVec3)>,
    /// Power of new displacements, and the heights a heightmap's black and white map to.
    pub(crate) disp_power: u32,
    pub(crate) heightmap_range: (f64, f64),
    /// The status message and when it first showed, so it can give way to tool hints.
    pub(crate) status_shown: (String, Instant),
    pub(crate) inspector_tab: crate::panels::InspectorTab,
    /// Corners and entity origins drags snap to, rebuilt after the map changes.
    pub(crate) snap_points: Option<Vec<(NodeId, DVec3)>>,
    /// The view and point a drag or a placing tool would snap to, and whether a corner caught it.
    pub(crate) snap_mark: Option<(usize, DVec3, bool)>,
    /// The viewport under the pointer, which Space maximizes.
    pub(crate) hovered_pane: Option<usize>,
    /// The face an extrude drag would take, shown highlighted.
    pub(crate) extrude_hover: Option<crate::transform::ExtrudeHover>,
    pub(crate) vertex: crate::vertex::VertexState,
    /// The compile profile Run Last Profile repeats.
    last_compile: Option<usize>,
    /// Quit was asked for from a menu or shortcut.
    quit_requested: bool,
    /// SDK paths were edited since the SDK last loaded.
    sdk_paths_changed: bool,
    /// FGD-driven property widgets; off shows raw key values (Hammer's SmartEdit).
    pub(crate) smart_edit: bool,
    pub(crate) show_new_dialog: bool,
    pub(crate) new_sdk: Option<String>,
    pub(crate) show_sdk_dialog: bool,
    /// The SDK the SDK Settings window edits, when not the map's own.
    pub(crate) sdk_settings_for: Option<Sdk>,
    pub(crate) sdk_manager: crate::sdk_manager::SdkManager,
    pending: Option<Pending>,
    pub(crate) confirm_tools: Option<usize>,
    pub(crate) compile: Option<CompileJob>,
    /// Plugin materials still being listed on a worker thread.
    pub(crate) material_list: Option<mpsc::Receiver<crate::project::Listed>>,
    last_autosave: Instant,
    screenshot: Option<PathBuf>,
    sdk_var_overrides: Vec<(String, String)>,
    frame_pending: bool,
    frame: u64,
    quit_confirmed: bool,
    /// Calls from agents through `mapcraft-cli mcp`.
    pub(crate) control: Option<crate::control::Channel>,
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
        crate::chrome::apply_theme(&cc.egui_ctx);
        let grid = prefs.get().view.grid;
        let data_dir = match &options.settings_dir {
            Some(dir) => Some(dir.join("data")),
            None => craft_settings::paths::data_dir("mapcraft"),
        };
        let recovery = data_dir.as_deref().map(crate::dialogs::start_session).unwrap_or_default();
        let cached = data_dir.as_deref().map(|d| mapcraft_plugin_host::Host::with_cache(&d.join("plugin-cache")));
        let plugin_host = match cached {
            Some(Ok(host)) => Ok(host),
            _ => mapcraft_plugin_host::Host::new(),
        }
        .map_err(|e| log::error!("plugins disabled: {e}"))
        .ok();
        let gpu = cc.wgpu_render_state.as_ref().map(|state| Gpu { renderer: Renderer::new(&state.device, &state.queue), state: state.clone() });
        let pane = |view| Pane { camera: Camera::new(view), target: None, texture: None, rect: Rect::NOTHING, uv: None };
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
            tree_drag: Vec::new(),
            tree_revealed: BTreeSet::new(),
            tree_view: (0.0, 0.0),
            last_box: Aabb::new(DVec3::ZERO, DVec3::splat(64.0)),
            clip_points: Vec::new(),
            clip_keep: mapcraft_engine::ClipKeep::Both,
            extrude_along_normal: true,
            clip_view_axis: None,
            cursor_world: None,
            display: None,
            prefab_cache: HashMap::new(),
            dialogs: Dialogs { recovery, ..Dialogs::default() },
            data_dir,
            last_click: None,
            picked_face: None,
            thumbs: crate::thumbs::Thumbs::new(),
            uv_images: crate::thumbs::Thumbs::sized(1024),
            uv_modal: None,
            uv_busy: false,
            find: crate::panels::FindState::default(),
            tag_name: String::new(),
            show_thumbs: true,
            plugin_host,
            plugin_dialog: None,
            plugin_shape: None,
            plugin_params: BTreeMap::new(),
            ghost_at: None,
            sculpt: (mapcraft_engine::SculptMode::Raise, 128.0, 256.0),
            sculpt_at: None,
            disp_power: 3,
            heightmap_range: (0.0, 512.0),
            status_shown: (String::new(), Instant::now()),
            inspector_tab: crate::panels::InspectorTab::default(),
            snap_points: None,
            snap_mark: None,
            hovered_pane: None,
            extrude_hover: None,
            vertex: crate::vertex::VertexState::default(),
            last_compile: None,
            quit_requested: false,
            sdk_paths_changed: false,
            smart_edit: true,
            show_new_dialog: false,
            new_sdk: None,
            show_sdk_dialog: false,
            sdk_settings_for: None,
            sdk_manager: crate::sdk_manager::SdkManager::default(),
            pending: None,
            confirm_tools: None,
            compile: None,
            material_list: None,
            last_autosave: Instant::now(),
            screenshot: options.screenshot,
            sdk_var_overrides: options.sdk_vars,
            frame_pending: false,
            frame: 0,
            quit_confirmed: false,
            control: None,
        };
        app.session.grid = grid;
        app.rescan_sdks();
        if app.screenshot.is_none() {
            app.control = app.data_dir.as_deref().and_then(|d| crate::control::start(d, &cc.egui_ctx));
        }
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
        let dirs = mapcraft_sdk_manager::search_dirs(self.data_dir.as_deref(), &self.prefs.get().sdk_dirs);
        self.sdks = mapcraft_sdk_manager::installed(&dirs);
    }

    fn sdk_by_id(&self, id: &str) -> Option<Sdk> {
        self.sdks.iter().find_map(|i| i.sdk.as_ref().ok().filter(|s| s.manifest.id == id).cloned())
    }

    /// Path settings for SDK `id`: the saved ones, then command line overrides, then the manifest's defaults.
    pub(crate) fn sdk_vars(&self, id: &str, sdk: Option<&Sdk>) -> BTreeMap<String, String> {
        let mut vars = self.prefs.get().sdk_paths.get(id).cloned().unwrap_or_default();
        vars.extend(self.sdk_var_overrides.iter().cloned());
        for (k, setting) in sdk.iter().flat_map(|s| &s.manifest.paths) {
            if let (None, Some(d)) = (vars.get(k), &setting.default) {
                vars.insert(k.clone(), d.clone());
            }
        }
        vars
    }

    pub(crate) fn load_project(&mut self, sdk_id: Option<String>) {
        let sdk = sdk_id.as_deref().and_then(|id| self.sdk_by_id(id));
        if let (Some(id), None) = (&sdk_id, &sdk) {
            self.status = format!("SDK `{id}` is not installed; entities and materials are unavailable");
        }
        let vars = sdk_id.as_deref().map(|id| self.sdk_vars(id, sdk.as_ref())).unwrap_or_default();
        if let Some(s) = &sdk {
            self.session.tex.scale = s.manifest.textures.scale;
            if let Some(d) = &s.manifest.textures.default {
                self.session.tex.material = d.clone();
            }
        }
        let user_plugins = self.data_dir.as_ref().map(|d| d.join("plugins"));
        self.project = Project::load(sdk, vars, self.plugin_host.as_ref(), user_plugins.as_deref());
        self.material_list = self.plugin_host.as_ref().and_then(|h| self.project.list_materials_later(h));
        if let Some(dir) = self.material_cache() {
            self.project.add_cached_materials(&dir);
        }
        self.thumbs.clear();
        let pixelated = self.project.sdk.as_ref().is_some_and(|s| s.manifest.textures.pixelated);
        if let Some(gpu) = &mut self.gpu {
            gpu.renderer.forget_materials();
            gpu.renderer.set_pixelated(pixelated);
        }
        for p in &self.project.problems {
            self.output.push(p.clone());
        }
        let missing = self.project.missing_for_editing();
        if !missing.is_empty() {
            // Materials cannot load until these are set, so ask right away.
            self.show_sdk_dialog = true;
            self.status = format!("Set {} in SDK Settings to load this SDK's materials", missing.join(", "));
        }
        self.scene_dirty = true;
    }

    /// Gives the open map another SDK and reloads the project for it; not an undo step.
    pub(crate) fn switch_sdk(&mut self, sdk: Option<String>) {
        if sdk == self.session.sdk {
            return;
        }
        self.session.sdk = sdk.clone();
        self.session.mark_unsaved();
        self.prefs.update(|p| p.last_sdk = sdk.clone());
        self.status.clear();
        self.load_project(sdk);
        let name = self.project.sdk.as_ref().map_or("no SDK".to_string(), |s| s.manifest.name.clone());
        let entities = self.session.map.nodes().filter_map(|(_, n)| n.kind.entity()).map(|e| e.classname.as_str());
        let unknown: BTreeSet<&str> = entities.filter(|c| self.project.sdk.is_some() && !c.is_empty() && self.project.game.class(c).is_none()).collect();
        let switched = if unknown.is_empty() {
            format!("Switched the map to {name}")
        } else {
            format!("Switched the map to {name}, which does not define {}; Check for Problems lists them", Vec::from_iter(unknown).join(", "))
        };
        // load_project's own warning, such as folders left to set, matters more.
        if self.status.is_empty() {
            self.status = switched;
        } else {
            self.output.push(switched);
        }
    }

    /// None and every installed SDK, as selectable rows; clicking one switches the map to it.
    pub(crate) fn sdk_choices(&mut self, ui: &mut egui::Ui) -> bool {
        let current = self.session.sdk.clone();
        let mut pick = ui.selectable_label(current.is_none(), "None").clicked().then_some(None);
        for s in self.sdks.iter().filter_map(|i| i.sdk.as_ref().ok()) {
            let on = current.as_deref() == Some(s.manifest.id.as_str());
            if ui.selectable_label(on, &s.manifest.name).on_hover_text(&s.manifest.description).clicked() {
                pick = Some(Some(s.manifest.id.clone()));
            }
        }
        let Some(sdk) = pick else { return false };
        self.switch_sdk(sdk);
        true
    }

    pub(crate) fn new_now(&mut self, sdk: Option<String>) {
        self.session = Session::default();
        self.session.grid = self.prefs.get().view.grid;
        self.session.sdk = sdk.clone();
        self.prefs.update(|p| p.last_sdk = sdk.clone());
        self.status = "New map".into();
        self.load_project(sdk);
    }

    pub(crate) fn open_now(&mut self, path: &Path) {
        if let Err(e) = self.open_file(path) {
            self.status = format!("Cannot open {}: {e}", path.display());
        }
    }

    pub(crate) fn open_file(&mut self, path: &Path) -> Result<(), mapcraft_engine::EngineError> {
        let mut s = Session::open(path)?;
        s.grid = self.prefs.get().view.grid;
        let sdk = s.sdk.clone();
        // Layers are the map's outline, so the tree opens with them unfolded.
        self.expanded = s.map.nodes().filter(|(_, n)| matches!(n.kind, NodeKind::Layer(_))).map(|(id, _)| id).chain([mapcraft_map::ROOT]).collect();
        self.session = s;
        self.prefs.update(|p| p.push_recent(&path.to_string_lossy()));
        self.status = format!("Opened {}", path.display());
        self.load_project(sdk);
        self.frame_pending = true;
        Ok(())
    }

    pub(crate) fn save(&mut self, save_as: bool) -> bool {
        let path = match (&self.session.path, save_as) {
            (Some(p), false) => p.clone(),
            _ => match rfd::FileDialog::new().add_filter("MapCraft map", &["mcmap"]).set_file_name("untitled.mcmap").save_file() {
                Some(p) => p.with_extension(mapcraft_format::EXTENSION),
                None => return false,
            },
        };
        match self.save_to(&path) {
            Ok(()) => true,
            Err(e) => {
                self.status = format!("Save failed: {e}");
                false
            }
        }
    }

    /// Saves to `path`, which becomes the map's path, and drops the old autosave.
    pub(crate) fn save_to(&mut self, path: &Path) -> Result<(), mapcraft_engine::EngineError> {
        let old_autosave = self.autosave_path();
        self.session.save_as(path)?;
        if let Some(a) = old_autosave {
            let _ = std::fs::remove_file(a);
        }
        self.prefs.update(|p| p.push_recent(&path.to_string_lossy()));
        self.status = format!("Saved {}", path.display());
        Ok(())
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
        self.frame_bounds(self.map_bounds().unwrap_or(Aabb::new(DVec3::splat(-256.0), DVec3::splat(256.0))));
    }

    pub(crate) fn map_bounds(&self) -> Option<Aabb> {
        self.view_map().nodes().filter_map(|(id, _)| self.leaf_bounds(id)).reduce(|a, b| a.union(&b))
    }

    pub(crate) fn frame_bounds(&mut self, b: Aabb) {
        for pane in &mut self.panes {
            let size = pane.rect.size();
            frame_camera(&mut pane.camera, &b, f64::from(size.x.min(size.y)));
        }
    }

    pub(crate) fn mark_changed(&mut self) {
        self.scene_dirty = true;
        self.snap_points = None;
    }

    pub(crate) fn reload_prefabs(&mut self) {
        self.prefab_cache.clear();
        self.mark_changed();
    }

    fn update_display(&mut self) {
        let derived = |n: &mapcraft_map::Node| {
            matches!(
                &n.kind,
                NodeKind::Prefab(_) | NodeKind::Group(mapcraft_map::Group { mirror: Some(_), .. }) | NodeKind::Group(mapcraft_map::Group { link: Some(_), .. })
            )
        };
        if !self.session.map.nodes().any(|(_, n)| derived(n)) {
            self.display = None;
            return;
        }
        let doc = self.session.path.clone();
        let (flat, owner, problems) = mapcraft_engine::flatten(&self.session.map, &mut prefab_loader(&mut self.prefab_cache, doc));
        if let Some(p) = problems.first() {
            self.status = p.clone();
        }
        self.display = Some((flat, owner));
    }

    /// The map as drawn and picked: prefab instances and mirror halves expanded.
    pub(crate) fn view_map(&self) -> &Map {
        self.display.as_ref().map_or(&self.session.map, |(m, _)| m)
    }

    /// The node in the real map a drawn node belongs to.
    pub(crate) fn owner(&self, id: NodeId) -> NodeId {
        self.display.as_ref().and_then(|(_, o)| o.get(&id).copied()).unwrap_or(id)
    }

    pub(crate) fn rebuild_scene(&mut self) {
        self.update_display();
        let Some(gpu) = &mut self.gpu else { return };
        let map = self.display.as_ref().map_or(&self.session.map, |(m, _)| m);
        let names: BTreeSet<String> = map.nodes().filter_map(|(_, n)| n.kind.brush()).flat_map(|b| b.faces.iter().map(|f| f.material.clone())).collect();
        self.project.upload_materials(&mut gpu.renderer, names.iter().map(String::as_str));
        self.project.prepare_models(map, &mut gpu.renderer, &mut self.output);
        let renderer = &gpu.renderer;
        let colorize = self.prefs.get().view.colorize;
        let scene = build_scene(map, &self.session.selection, &self.session.face_selection, &|m| renderer.material_size(m), &self.project, colorize);
        gpu.renderer.set_scene(&scene);
        self.scene_dirty = false;
    }

    /// Number of nodes in the map, for tests.
    pub fn node_count_for_test(&self) -> usize {
        self.session.map.len()
    }

    /// The editing session, for tests.
    pub fn session_for_test(&mut self) -> &mut Session {
        self.mark_changed();
        &mut self.session
    }

    /// Each view's 2D zoom, for tests.
    pub fn zooms_for_test(&self) -> Vec<f64> {
        self.panes.iter().map(|p| p.camera.zoom).collect()
    }

    /// Whether the SDK Settings window is open, for tests.
    pub fn sdk_settings_open_for_test(&self) -> bool {
        self.show_sdk_dialog
    }

    /// Results of the SDK Manager's last check of SDK `id`, for tests.
    pub fn sdk_check_for_test(&self, id: &str) -> Option<Vec<mapcraft_sdk_manager::Item>> {
        let dir = &self.sdks.iter().find(|i| i.sdk.as_ref().is_ok_and(|s| s.manifest.id == id))?.dir;
        self.sdk_manager.check_results(dir).cloned()
    }

    /// The status bar message, for tests.
    pub fn status_for_test(&self) -> &str {
        &self.status
    }

    /// The id of the SDK the editor loaded for the map, for tests.
    pub fn project_sdk_for_test(&self) -> Option<&str> {
        self.project.id()
    }

    /// Ids of the SDKs found, in search order, for tests.
    pub fn sdk_ids_for_test(&self) -> Vec<String> {
        self.sdks.iter().filter_map(|i| Some(i.sdk.as_ref().ok()?.manifest.id.clone())).collect()
    }

    /// Replaces the registries the SDK Manager lists, for tests and snapshots.
    pub fn set_registries_for_test(&mut self, registries: Vec<String>) {
        self.prefs.update(|p| p.registries = registries);
    }

    /// Where a world point shows in view `index`, for tests that drag things.
    pub fn screen_for_test(&self, index: usize, p: DVec3) -> Pos2 {
        let Some(pane) = self.panes.get(index) else { return Pos2::ZERO };
        let q = pane.camera.project(p, mapcraft_geom::DVec2::new(f64::from(pane.rect.width()), f64::from(pane.rect.height()))).unwrap_or_default();
        pane.rect.min + Vec2::new(q.x as f32, q.y as f32)
    }

    /// The maximized view, for tests.
    pub fn maximized_for_test(&self) -> Option<usize> {
        self.maximized
    }

    /// Where a point dragged in view `index` would snap, and whether a corner caught it.
    pub fn snap_for_test(&mut self, index: usize, p: DVec3) -> (DVec3, bool) {
        let rect = self.panes.get(index).map_or(Rect::NOTHING, |pane| pane.rect);
        self.snap_point(index, rect, p, &BTreeSet::new(), &[])
    }

    /// Runs an action or engine command by id, as a menu item would.
    pub fn fly_speed_for_test(&self) -> f64 {
        self.prefs.get().view.fly_speed
    }

    pub fn clip_points_for_test(&self) -> usize {
        self.clip_points.len()
    }

    pub fn run_command_for_test(&mut self, id: &str) {
        self.run_action(id);
    }

    pub(crate) fn run_command(&mut self, id: &str) {
        if let Err(e) = mapcraft_engine::execute(&mut self.session, id) {
            self.status = e.to_string();
        }
        self.mark_changed();
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if self.uv_modal.is_some() {
            self.uv_modal_input(ctx);
            return;
        }
        if self.vertex.bevel.is_some() {
            self.bevel_modal_input(ctx);
            return;
        }
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        // WASD and friends fly the 3D camera while the right button is down.
        let flying = ctx.input(|i| i.pointer.secondary_down());
        let over_uv = self.hovered_pane.and_then(|i| self.panes.get(i)).is_some_and(|p| p.uv.is_some());
        let prefs = self.prefs.get();
        for (sc, id) in crate::shortcuts::bindings(prefs.keymap, &prefs.shortcuts) {
            if (flying && !sc.modifiers.command)
                || (!over_uv && crate::shortcuts::uv_only(id))
                || (self.tool != Tool::Vertex && crate::shortcuts::vertex_only(id))
            {
                continue;
            }
            if ctx.input_mut(|i| i.consume_shortcut(&sc)) {
                self.run_action(id);
            }
        }
        self.clipboard(ctx);
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.tool == Tool::Clip && !self.clip_points.is_empty() {
                self.clip_points.clear();
            } else {
                self.set_tool(Tool::Select);
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Enter)) && self.tool == Tool::Clip && self.clip_points.len() >= 2 {
            self.apply_clip();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Enter)) && self.tool == Tool::Path {
            self.session.select_none();
            self.mark_changed();
        }
    }

    /// Switches tools; the Face tool takes over the selected brushes as picked faces, as in Hammer.
    fn set_tool(&mut self, t: Tool) {
        self.clip_points.clear();
        self.drag = Drag::None;
        if t == Tool::Face && self.tool != Tool::Face {
            let map = &self.session.map;
            let brushes: BTreeSet<NodeId> = self.session.selected_brushes().into_iter().collect();
            let covered = mapcraft_map::covered_faces(map, Some(&brushes));
            let faces: BTreeSet<_> = brushes
                .iter()
                .filter_map(|id| Some(map.get(*id)?.kind.brush()?.faces.iter().map(move |f| (*id, f.id)).collect::<Vec<_>>()))
                .flatten()
                .filter(|f| !covered.contains(f))
                .collect();
            if !covered.is_empty() {
                self.status = format!("Picked {} faces; left out {} hidden between touching brushes", faces.len(), covered.len());
            }
            self.session.face_selection = faces;
            self.session.selection.clear();
        } else if !matches!(t, Tool::Face | Tool::Sculpt) {
            self.session.face_selection.clear();
        }
        self.tool = t;
        self.mark_changed();
    }

    /// Runs a bindable action: the shell's own, or an engine command.
    pub(crate) fn run_action(&mut self, id: &str) {
        if crate::shortcuts::vertex_only(id) {
            self.vertex_action(id);
            return;
        }
        let tool = match id {
            "tool.select" => Some(Tool::Select),
            "tool.move" => Some(Tool::Move),
            "tool.extrude" => Some(Tool::Extrude),
            "tool.rotate" => Some(Tool::Rotate),
            "tool.scale" => Some(Tool::Scale),
            "tool.block" => Some(Tool::Block),
            "tool.entity" => Some(Tool::Entity),
            "tool.clip" => Some(Tool::Clip),
            "tool.vertex" => Some(Tool::Vertex),
            "tool.face" => Some(Tool::Face),
            "tool.sculpt" => Some(Tool::Sculpt),
            "tool.path" => Some(Tool::Path),
            _ => None,
        };
        if let Some(t) = tool {
            self.set_tool(t);
            return;
        }
        use mapcraft_engine::SculptMode as M;
        let mode = match id {
            "sculpt.raise" => Some(M::Raise),
            "sculpt.lower" => Some(M::Lower),
            "sculpt.smooth" => Some(M::Smooth),
            "sculpt.flatten" => Some(M::Flatten),
            "sculpt.noise" => Some(M::Noise),
            _ => None,
        };
        if let Some(m) = mode {
            self.sculpt.0 = m;
            if self.tool != Tool::Sculpt {
                self.set_tool(Tool::Sculpt);
            }
            return;
        }
        match id {
            "file.new" => {
                self.new_sdk = self.project.id().map(String::from);
                self.show_new_dialog = true;
            }
            "file.open" => self.open_dialog(),
            "file.save" => {
                self.save(false);
            }
            "file.save_as" => {
                self.save(true);
            }
            "edit.preferences" => self.dialogs.preferences = true,
            "tools.transform" => self.dialogs.transform.open = true,
            "tools.array" => self.open_array(),
            "tools.sweep" | "tools.spin" => self.run_curve(id),
            "tools.replace_materials" => self.open_replace_materials(),
            "view.snap_grid" => {
                self.prefs.update(|p| p.view.snap_grid = !p.view.snap_grid);
            }
            "view.snap_vertices" => {
                self.prefs.update(|p| p.view.snap_vertices = !p.view.snap_vertices);
            }
            "view.link_zoom" => {
                self.prefs.update(|p| p.view.link_zoom = !p.view.link_zoom);
            }
            "view.maximize" if self.prefs.get().view.single_view => {
                let next = self.maximized.map_or(0, |m| (m + 1) % self.panes.len().max(1));
                self.maximized = Some(next);
                self.status = format!("Showing {}; Space shows the next view", self.panes.get(next).map_or("", |p| p.camera.view.label()));
            }
            "view.maximize" => {
                self.maximized = if self.maximized.is_some() { None } else { self.hovered_pane };
            }
            "view.single" => {
                let on = !self.prefs.get().view.single_view;
                self.prefs.update(|p| p.view.single_view = on);
                self.maximized = on.then(|| self.hovered_pane.unwrap_or(0));
            }
            "view.origin" => self.frame_bounds(Aabb::new(DVec3::splat(-256.0), DVec3::splat(256.0))),
            "prefab.make_local" => self.make_local(),
            "prefab.save" => self.save_prefab(),
            "prefab.insert" => self.insert_prefab(),
            "prefab.reload" => self.reload_prefabs(),
            "file.sdk_settings" => {
                self.sdk_settings_for = None;
                self.show_sdk_dialog = true;
            }
            "file.sdk_manager" => self.sdk_manager.open = true,
            "file.switch_sdk" => self.dialogs.switch_sdk = true,
            "file.quit" => self.quit_requested = true,
            "help.shortcuts" => self.dialogs.shortcuts = !self.dialogs.shortcuts,
            "tools.check_problems" => self.check_problems(),
            "layer.new" => {
                let n = self.session.map.nodes().filter(|(_, n)| matches!(n.kind, NodeKind::Layer(_))).count() + 1;
                if let Err(e) = self.session.create_layer(&format!("Layer {n}")) {
                    self.status = e.to_string();
                }
                self.mark_changed();
            }
            "view.render_cycle" => {
                self.render_mode = match self.render_mode {
                    RenderMode::Textured => RenderMode::Flat,
                    RenderMode::Flat => RenderMode::Wireframe,
                    RenderMode::Wireframe => RenderMode::Lit,
                    RenderMode::Lit => RenderMode::Textured,
                };
            }
            "view.render_textured" => self.render_mode = RenderMode::Textured,
            "view.render_flat" => self.render_mode = RenderMode::Flat,
            "view.render_wireframe" => self.render_mode = RenderMode::Wireframe,
            "view.render_lit" => self.render_mode = RenderMode::Lit,
            "view.colorize" => {
                self.prefs.update(|p| p.view.colorize = !p.view.colorize);
                self.mark_changed();
            }
            "view.frame_all" => self.frame_all(),
            "view.frame_selection" if self.session.selection.is_empty() => self.frame_all(),
            "view.frame_selection" => self.frame_selection(),
            "view.find_entities" => {
                self.bottom_tab = BottomTab::Find;
                self.show_bottom = true;
            }
            "panel.scene" => self.left_tab = LeftTab::Scene,
            "panel.tags" => self.left_tab = LeftTab::Tags,
            "panel.materials" => self.left_tab = LeftTab::Materials,
            "panel.entities" => self.left_tab = LeftTab::Entities,
            "panel.models" => self.left_tab = LeftTab::Models,
            "panel.output" => self.show_bottom = !self.show_bottom,
            "inspector.smart_edit" => self.smart_edit = !self.smart_edit,
            "shape.next" => {
                self.shape = crate::chrome::next_shape(self.shape);
                self.pick_plugin_shape(None);
            }
            "clip.next_side" => {
                use mapcraft_engine::ClipKeep as K;
                self.clip_keep = match self.clip_keep {
                    K::Both => K::Front,
                    K::Front => K::Back,
                    K::Back => K::Both,
                };
            }
            "run.again" => {
                let count = self.project.sdk.as_ref().map_or(0, |s| s.manifest.compile.len());
                match self.last_compile.filter(|i| *i < count).or((count > 0).then_some(0)) {
                    Some(i) if self.compile.is_none() => self.start_compile(i),
                    Some(_) => self.status = "A compile is already running".into(),
                    None => self.status = "This SDK has no compile profiles".into(),
                }
            }
            "run.cancel" => self.cancel_compile(),
            "uv.grab" => self.begin_uv_modal(crate::uv::UvMode::Grab),
            "uv.rotate" => self.begin_uv_modal(crate::uv::UvMode::Rotate),
            "uv.scale" => self.begin_uv_modal(crate::uv::UvMode::Scale),
            _ => self.run_command(id),
        }
    }

    fn save_prefab(&mut self) {
        if self.session.selection.is_empty() {
            self.status = "Select what the prefab should hold first".into();
            return;
        }
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

    fn insert_prefab(&mut self) {
        let Some(path) = rfd::FileDialog::new().add_filter("MapCraft map", &["mcmap"]).pick_file() else { return };
        let stored = self.session.relative_path(&path);
        let at = self.cursor_world.map_or(DVec3::ZERO, |p| mapcraft_geom::snap(p, self.session.grid));
        if let Err(e) = self.session.insert_prefab(&stored, at) {
            self.status = e.to_string();
        }
        self.mark_changed();
    }

    /// Where pictures of materials brought in by imports are kept between sessions.
    pub(crate) fn material_cache(&self) -> Option<PathBuf> {
        self.data_dir.as_ref().map(|d| d.join("materials"))
    }

    /// The chord shown next to an action in menus.
    pub(crate) fn chord(&self, id: &str) -> String {
        let p = self.prefs.get();
        crate::shortcuts::chord(id, p.keymap, &p.shortcuts).unwrap_or_default()
    }

    /// Remembers the active picked face's material and alignment for pasting.
    pub(crate) fn copy_face(&mut self) {
        let face = self.session.active_pick().and_then(|(id, f)| self.session.map.get(id)?.kind.brush()?.faces.iter().find(|x| x.id == f).cloned());
        match face {
            Some(f) => {
                self.status = format!("Copied the face's material and alignment ({}); Ctrl+V pastes them on picked faces", f.material);
                self.session.tex.material = f.material.clone();
                self.picked_face = Some(f);
            }
            None => self.status = "Pick a face to copy first".into(),
        }
    }

    pub(crate) fn paste_face(&mut self) {
        let result = match self.picked_face.clone() {
            Some(src) => self.session.paste_face(&src).map_err(|e| e.to_string()),
            None => Err("Copy a face first: Ctrl+C on a picked face, or Alt+click a face".into()),
        };
        if let Err(e) = result {
            self.status = e;
        }
        self.mark_changed();
    }

    fn clipboard(&mut self, ctx: &egui::Context) {
        let events: Vec<egui::Event> = ctx.input(|i| i.events.clone());
        for ev in events {
            match ev {
                egui::Event::Copy if self.tool == Tool::Face => self.copy_face(),
                egui::Event::Paste(_) if self.tool == Tool::Face => self.paste_face(),
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

    pub(crate) fn open_dialog_path(&mut self, path: PathBuf) {
        self.request(Pending::Open(path));
    }

    fn make_local(&mut self) {
        let doc = self.session.path.clone();
        if let Err(e) = self.session.make_local(&mut prefab_loader(&mut self.prefab_cache, doc)) {
            self.status = e.to_string();
        }
        self.mark_changed();
    }

    pub(crate) fn open_dialog(&mut self) {
        if let Some(p) = rfd::FileDialog::new().add_filter("MapCraft map", &["mcmap"]).pick_file() {
            self.request(Pending::Open(p));
        }
    }

    pub(crate) fn action_button(&mut self, ui: &mut egui::Ui, id: &str, label: &str, enabled: bool) {
        let button = egui::Button::new(label).shortcut_text(self.chord(id));
        if ui.add_enabled(enabled, button).clicked() {
            self.run_action(id);
        }
    }

    /// A menu item that shows `on` as selected.
    fn action_toggle(&mut self, ui: &mut egui::Ui, id: &str, label: &str, on: bool) {
        if ui.add(egui::Button::selectable(on, label).shortcut_text(self.chord(id))).clicked() {
            self.run_action(id);
        }
    }

    /// A menu item for an engine command; selection modes show which one is on.
    pub(crate) fn command_item(&mut self, ui: &mut egui::Ui, c: &mapcraft_engine::CommandSpec) {
        let label = c.menu.rsplit('/').next().unwrap_or(c.label);
        let mode = match c.id {
            "edit.select_groups" => Some(SelectMode::Groups),
            "edit.select_objects" => Some(SelectMode::Objects),
            "edit.select_solids" => Some(SelectMode::Solids),
            _ => None,
        };
        match mode {
            Some(m) => self.action_toggle(ui, c.id, label, self.session.select_mode == m),
            None => {
                let enabled = (c.enabled)(&self.session);
                self.action_button(ui, c.id, label, enabled);
            }
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                self.action_button(ui, "file.new", "New Map…", true);
                self.action_button(ui, "file.open", "Open…", true);
                ui.menu_button("Open Recent", |ui| {
                    for p in self.prefs.get().recent.clone() {
                        if ui.button(&p).clicked() {
                            self.request(Pending::Open(PathBuf::from(p)));
                        }
                    }
                });
                ui.separator();
                self.action_button(ui, "file.save", "Save", true);
                self.action_button(ui, "file.save_as", "Save As…", true);
                ui.separator();
                self.import_menu(ui);
                self.export_menu(ui);
                ui.separator();
                self.action_button(ui, "file.switch_sdk", "Switch SDK…", true);
                self.action_button(ui, "file.sdk_settings", "SDK Settings…", true);
                self.action_button(ui, "file.sdk_manager", "SDK Manager…", true);
                ui.separator();
                self.action_button(ui, "file.quit", "Quit", true);
            });
            for top in ["Edit", "Tools", "View"] {
                ui.menu_button(top, |ui| {
                    if top == "Tools" {
                        self.action_button(ui, "tools.check_problems", "Check for Problems", true);
                        let any = !self.session.selection.is_empty();
                        self.action_button(ui, "tools.transform", "Transform…", any);
                        self.action_button(ui, "tools.array", "Array…", any);
                        self.action_button(ui, "tools.replace_materials", "Replace Materials…", true);
                        ui.separator();
                        self.prefab_menu(ui);
                        ui.separator();
                    }
                    if top == "View" {
                        let mode = self.render_mode;
                        self.action_toggle(ui, "view.render_textured", "Textured", mode == RenderMode::Textured);
                        self.action_toggle(ui, "view.render_flat", "Flat Shaded", mode == RenderMode::Flat);
                        self.action_toggle(ui, "view.render_wireframe", "Wireframe", mode == RenderMode::Wireframe);
                        self.action_toggle(ui, "view.render_lit", "Lit", mode == RenderMode::Lit);
                        self.action_button(ui, "view.render_cycle", "Next Render Mode", true);
                        ui.separator();
                        let view = self.prefs.get().view.clone();
                        self.action_toggle(ui, "view.link_zoom", "Link 2D Zoom", view.link_zoom);
                        self.action_toggle(ui, "view.colorize", "Color Brushes by Object", view.colorize);
                        self.action_toggle(ui, "view.snap_grid", "Snap to Grid", view.snap_grid);
                        self.action_toggle(ui, "view.snap_vertices", "Snap to Corners", view.snap_vertices);
                        ui.separator();
                        self.action_button(ui, "view.frame_all", "Frame All", true);
                        self.action_button(ui, "view.frame_selection", "Focus Selection", true);
                        self.action_button(ui, "view.origin", "Go to Origin", true);
                        self.action_button(ui, "view.maximize", if view.single_view { "Next View" } else { "Maximize View Under Pointer" }, true);
                        self.action_toggle(ui, "view.single", "One View Layout", view.single_view);
                        self.action_button(ui, "view.find_entities", "Find Entities", true);
                        ui.menu_button("Panels", |ui| {
                            for (id, label) in [
                                ("panel.scene", "Scene"),
                                ("panel.tags", "Tags"),
                                ("panel.materials", "Materials"),
                                ("panel.entities", "Entities"),
                                ("panel.models", "Models"),
                                ("panel.output", "Output"),
                            ] {
                                self.action_button(ui, id, label, true);
                            }
                        });
                        ui.separator();
                    }
                    let in_menu = |c: &&mapcraft_engine::CommandSpec, menu: &str| c.menu.rsplit_once('/').is_some_and(|(m, _)| m == menu);
                    let mut submenus: Vec<&str> = Vec::new();
                    for c in mapcraft_engine::commands().iter().filter(|c| c.menu.starts_with(&format!("{top}/"))) {
                        match c.menu.split('/').nth(1).filter(|_| c.menu.matches('/').count() == 2) {
                            Some(sub) if !submenus.contains(&sub) => {
                                submenus.push(sub);
                                ui.menu_button(sub, |ui| {
                                    for c in mapcraft_engine::commands().iter().filter(|c| in_menu(c, &format!("{top}/{sub}"))) {
                                        self.command_item(ui, c);
                                    }
                                });
                            }
                            Some(_) => {}
                            None => self.command_item(ui, c),
                        }
                    }
                    if top == "Edit" {
                        ui.separator();
                        self.action_button(ui, "edit.preferences", "Preferences…", true);
                    }
                });
            }
            self.plugins_menu(ui);
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
                ui.separator();
                self.action_button(ui, "run.again", "Run Last Profile", !profiles.is_empty() && self.compile.is_none());
                self.action_button(ui, "run.cancel", "Cancel", self.compile.is_some());
            });
            ui.menu_button("Help", |ui| {
                self.action_button(ui, "help.shortcuts", "Keyboard Shortcuts", true);
            });
        });
    }

    pub(crate) fn check_problems(&mut self) {
        self.issues = mapcraft_engine::validate(&self.session.map, self.project.sdk.as_ref().map(|_| &self.project.game));
        let map = self.view_map().clone();
        let name = self.session.path.as_ref().and_then(|p| p.file_stem()).map_or("untitled".into(), |s| s.to_string_lossy().into_owned());
        let mut found = Vec::new();
        for plugin in self.project.plugins.iter_mut().filter(|p| p.manifest.validates) {
            match plugin.validate(&map, &name) {
                Ok(list) => found.extend(list),
                Err(e) => self.output.push(e.to_string()),
            }
            self.output.extend(plugin.take_log());
        }
        for i in found {
            let node = i.node.map_or(mapcraft_map::ROOT, |n| self.owner(n));
            let severity = if i.error { mapcraft_engine::Severity::Error } else { mapcraft_engine::Severity::Warning };
            self.issues.push(mapcraft_engine::Issue { node, face: None, severity, message: i.message, fix: None });
        }
        self.bottom_tab = BottomTab::Issues;
        self.show_bottom = true;
    }

    fn prefab_menu(&mut self, ui: &mut egui::Ui) {
        let any = !self.session.selection.is_empty();
        self.action_button(ui, "prefab.save", "Save Selection as Prefab…", any);
        self.action_button(ui, "prefab.insert", "Insert Prefab…", true);
        self.action_button(ui, "prefab.make_local", "Make Prefab Local", any);
        self.action_button(ui, "prefab.reload", "Reload Prefabs", true);
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if self.show_new_dialog {
            let mut open = true;
            egui::Window::new("New Map").collapsible(false).resizable(false).open(&mut open).show(ctx, |ui| {
                ui.label("SDK");
                let ids: Vec<(String, String)> =
                    self.sdks.iter().filter_map(|i| i.sdk.as_ref().ok()).map(|s| (s.manifest.id.clone(), s.manifest.name.clone())).collect();
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
        let Some(sdk) = self.sdk_settings_for.clone().or_else(|| self.project.sdk.clone()) else {
            egui::Window::new("SDK Settings").open(&mut open).show(ctx, |ui| {
                ui.label("This map has no SDK. Pick one with File > New Map.");
                ui.label("SDK folders searched:");
                for (d, _) in mapcraft_sdk_manager::search_dirs(self.data_dir.as_deref(), &self.prefs.get().sdk_dirs) {
                    ui.monospace(d.display().to_string());
                }
            });
            self.show_sdk_dialog &= open;
            return;
        };
        // Settings for an SDK the map does not use are saved as typed and apply when it loads.
        let current = self.project.id() == Some(sdk.manifest.id.as_str());
        let mut vars = if current { self.project.vars.clone() } else { self.sdk_vars(&sdk.manifest.id, Some(&sdk)) };
        let mut changed = false;
        let mut apply = false;
        let editing = if current { self.project.missing_for_editing() } else { Vec::new() };
        // Paths that materials and entities need come first; the rest only matter for Run.
        let mut paths: Vec<_> = sdk.manifest.paths.iter().collect();
        let needed = |k: &String| sdk.manifest.textures.roots.iter().chain(&sdk.manifest.fgd).any(|s| s.contains(&format!("${{{k}}}")));
        paths.sort_by_key(|(k, _)| !needed(k));
        egui::Window::new(format!("{} SDK Settings", sdk.manifest.name)).open(&mut open).min_width(680.0).show(ctx, |ui| {
            if paths.is_empty() {
                ui.label("This SDK has no folders or programs to set.");
            }
            egui::Grid::new("sdk-paths").num_columns(3).min_col_width(80.0).show(ui, |ui| {
                for (key, setting) in paths {
                    let label = ui.label(&setting.prompt);
                    let mut value = vars.get(key).cloned().unwrap_or_default();
                    let bad = !value.is_empty() && !std::path::Path::new(&value).exists();
                    let edit = egui::TextEdit::singleline(&mut value).min_size(egui::vec2(300.0, 0.0)).desired_width(300.0);
                    let edit = if bad { edit.text_color(egui::Color32::LIGHT_RED) } else { edit };
                    if ui.add(edit).on_hover_text(if bad { "This path does not exist" } else { "" }).changed() {
                        vars.insert(key.clone(), value.clone());
                        changed = true;
                    }
                    if ui.button("Browse…").clicked() {
                        let picked = if setting.kind == "file" { rfd::FileDialog::new().pick_file() } else { rfd::FileDialog::new().pick_folder() };
                        if let Some(p) = picked {
                            vars.insert(key.clone(), p.to_string_lossy().into_owned());
                            changed = true;
                        }
                    }
                    ui.end_row();
                    if editing.contains(key) {
                        label.on_hover_text("Needed to load materials and entities");
                    }
                }
            });
            if !editing.is_empty() {
                ui.colored_label(crate::chrome::ACCENT, format!("Materials load once {} is set.", editing.join(", ")));
            }
            if current {
                ui.horizontal(|ui| {
                    apply = ui.add_enabled(self.sdk_paths_changed || changed, egui::Button::new("Apply")).clicked();
                    ui.label(egui::RichText::new("Closing the window applies changes too.").color(crate::chrome::TEXT_WEAK));
                });
            } else {
                ui.label(egui::RichText::new("Saved as you type; used whenever a map with this SDK opens.").color(crate::chrome::TEXT_WEAK));
            }
        });
        if changed {
            let id = sdk.manifest.id.clone();
            if current {
                self.sdk_paths_changed = true;
                self.project.vars = vars.clone();
            }
            self.prefs.update(|p| {
                p.sdk_paths.insert(id, vars);
            });
        }
        self.show_sdk_dialog &= open;
        if !self.show_sdk_dialog {
            self.sdk_settings_for = None;
        }
        if current && (apply || !self.show_sdk_dialog) && self.sdk_paths_changed {
            self.sdk_paths_changed = false;
            self.load_project(Some(sdk.manifest.id.clone()));
            self.show_sdk_dialog = !open || apply;
            self.show_sdk_dialog &= open;
        }
    }

    pub(crate) fn start_compile(&mut self, index: usize) {
        self.last_compile = Some(index);
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
        // Games load one file, so prefabs are baked in first; the copy keeps the map's name.
        let has_prefabs = self.session.map.nodes().any(|(_, n)| matches!(n.kind, NodeKind::Prefab(_)));
        if has_prefabs
            && let (Some((flat, _)), Some(dir)) = (&self.display, self.data_dir.as_ref().map(|d| d.join("compile")))
            && let Some(file) = path.file_name()
        {
            let copy = dir.join(file);
            let written = std::fs::create_dir_all(&dir)
                .map_err(|e| e.to_string())
                .and_then(|()| mapcraft_format::save(&copy, flat, self.session.sdk.as_deref(), &serde_json::Value::Null).map_err(|e| e.to_string()));
            match written {
                Ok(()) => {
                    vars.insert("map".into(), copy.to_string_lossy().into_owned());
                }
                Err(e) => {
                    self.status = format!("cannot bake prefabs for the compile: {e}");
                    return;
                }
            }
        }
        self.output.push(format!("== {} ==", profile.name));
        // Exporters are plugins, which live on this thread, so export steps run before the tools do.
        for step in &profile.steps {
            let mapcraft_sdk::Step::Export { export, to } = step else { continue };
            let written = mapcraft_sdk::expand(to, &vars).map_err(|e| e.to_string()).and_then(|to| self.export_to(export, Path::new(&to)));
            if let Err(e) = written {
                self.output.push(format!("== failed: {e} =="));
                self.status = e;
                self.show_bottom = true;
                self.bottom_tab = BottomTab::Output;
                return;
            }
        }
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicBool::new(false));
        let (c, d) = (cancel.clone(), done.clone());
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

    fn poll_materials(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.material_list else { return };
        match rx.try_recv() {
            Ok(listed) => {
                self.material_list = None;
                let problems = self.project.take_listed(listed);
                self.output.extend(problems);
                if let Some(dir) = self.material_cache() {
                    self.project.add_cached_materials(&dir);
                }
                self.thumbs.clear();
                if let Some(gpu) = &mut self.gpu {
                    gpu.renderer.forget_materials();
                }
                self.scene_dirty = true;
            }
            Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
            Err(mpsc::TryRecvError::Disconnected) => self.material_list = None,
        }
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

    fn autosave_path(&self) -> Option<PathBuf> {
        let stem = self.session.path.as_ref().and_then(|p| p.file_stem()).map_or("untitled".to_string(), |s| s.to_string_lossy().into_owned());
        Some(self.data_dir.as_ref()?.join("autosave").join(format!("{stem}.autosave.mcmap")))
    }

    fn autosave(&mut self) {
        let minutes = self.prefs.get().view.autosave_minutes;
        if minutes == 0 || !self.session.is_dirty() || self.last_autosave.elapsed().as_secs() < u64::from(minutes) * 60 {
            return;
        }
        self.last_autosave = Instant::now();
        let Some(path) = self.autosave_path() else { return };
        let Some(dir) = path.parent() else { return };
        let result = std::fs::create_dir_all(dir)
            .map_err(|e| e.to_string())
            .and_then(|()| mapcraft_format::save(&path, &self.session.map, self.session.sdk.as_deref(), &self.session.view).map_err(|e| e.to_string()));
        if let Err(e) = result {
            self.status = format!("Autosave failed: {e}");
        }
    }

    fn central(&mut self, ui: &mut egui::Ui) {
        let full = ui.available_rect_before_wrap();
        self.hovered_pane = None;
        ui.painter().rect_filled(full, 0.0, crate::chrome::BG);
        if self.prefs.get().view.single_view && self.maximized.is_none() {
            self.maximized = Some(0);
        }
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
        let header = Rect::from_min_size(rect.min, Vec2::new(rect.width(), 22.0));
        let body = Rect::from_min_max(Pos2::new(rect.left(), header.bottom()), rect.max);
        let active = ui.ctx().pointer_hover_pos().is_some_and(|p| rect.contains(p));
        if active {
            self.hovered_pane = Some(index);
        }
        self.pane_header(ui, index, header, active);
        if self.panes.get(index).is_some_and(|p| p.uv.is_some()) {
            self.uv_view(ui, index, body);
        } else {
            self.viewport(ui, index, body);
            if self.panes.get(index).is_some_and(|p| !p.camera.is_2d()) {
                self.axis_widget(ui, index, body);
            }
        }
        if active {
            ui.painter().rect_stroke(rect, 0.0, egui::Stroke::new(1.0, crate::chrome::ACCENT.gamma_multiply(0.7)), egui::StrokeKind::Inside);
        }
    }

    pub(crate) fn cancel_compile(&self) {
        if let Some(job) = &self.compile {
            job.cancel.store(true, Ordering::Relaxed);
        }
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

/// Centers `cam` on `b`; a 2D view zooms so the box fills `px`, in the units of its zoom.
pub(crate) fn frame_camera(cam: &mut Camera, b: &Aabb, px: f64) {
    let c = b.center();
    if cam.is_2d() {
        let (r, u, _) = cam.view.axes();
        cam.center = mapcraft_geom::DVec2::new(c.dot(r), c.dot(u));
        cam.zoom = (px.max(100.0) / (b.size().max_element().max(64.0) * 1.3)).clamp(1.0 / 256.0, 64.0);
    } else {
        // Backs off until the box's bounding sphere fits the field of view.
        let radius = b.size().length().max(64.0) * 0.5;
        cam.position = c - cam.forward() * radius / (cam.fov_y.to_radians() * 0.5).sin().max(0.1);
    }
}

/// Loads prefabs by the path stored in an instance, relative to the map at `doc`, once each.
pub(crate) fn prefab_loader(cache: &mut HashMap<String, Option<Map>>, doc: Option<PathBuf>) -> impl FnMut(&str) -> Option<Map> + '_ {
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
        if self.quit_confirmed || std::mem::take(&mut self.quit_requested) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.prefs.tick(Instant::now());
        self.autosave();
        self.poll_compile(ctx);
        self.poll_materials(ctx);
        self.poll_models(ctx);
        self.poll_control();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.take_screenshot(&ctx);
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.title()));
        self.uv_busy = self.uv_modal.is_some() || self.vertex.bevel.is_some();
        let views = || self.panes.iter().filter(|p| p.uv.is_none());
        if let Some(p) = views().find(|p| !p.camera.is_2d()).or_else(|| views().next()) {
            self.session.view_dir = if p.camera.is_2d() { -p.camera.view.axes().2 } else { p.camera.forward() };
        }
        self.handle_shortcuts(&ctx);
        if self.scene_dirty {
            self.rebuild_scene();
        }
        egui::Panel::top("menu").show(ui, |ui| {
            self.menu_bar(ui);
            ui.add_space(2.0);
            self.top_bar(ui);
            ui.add_space(2.0);
        });
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        if self.show_bottom {
            egui::Panel::bottom("output").resizable(true).default_size(160.0).show(ui, |ui| self.bottom_panel(ui));
        }
        egui::Panel::left("left").resizable(true).default_size(260.0).show(ui, |ui| self.left_panel(ui));
        egui::Panel::left("tools").resizable(false).exact_size(44.0).show(ui, |ui| self.tool_palette(ui));
        egui::Panel::right("inspector").resizable(true).default_size(300.0).show(ui, |ui| self.inspector(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| self.central(ui));
        self.dialogs(&ctx);
        self.extra_dialogs(&ctx);
        self.sdk_manager_window(&ctx);
        self.plugin_window(&ctx);
        self.bevel_window(&ctx);
        self.handle_drops(&ctx);
        self.thumbs.poll(&ctx);
        self.uv_images.poll(&ctx);
        if self.frame_pending && self.panes.iter().all(|p| p.rect.is_positive()) {
            self.frame_pending = false;
            self.frame_all();
        }
        if self.scene_dirty {
            ctx.request_repaint();
        }
    }

    fn on_exit(&mut self) {
        if let Some(a) = self.autosave_path() {
            let _ = std::fs::remove_file(a);
        }
        if let Some(d) = &self.data_dir {
            crate::dialogs::end_session(d);
        }
        if let Some(c) = &self.control {
            c.stop();
        }
        let grid = self.session.grid;
        self.prefs.update(|p| p.view.grid = grid);
        if let Err(e) = self.prefs.flush() {
            log::error!("saving settings: {e}");
        }
    }
}
