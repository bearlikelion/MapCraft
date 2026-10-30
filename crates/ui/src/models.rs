//! glTF models for point entities: loaded on a worker thread for the views, and the Models tab.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};

use egui::{ColorImage, RichText, TextureHandle};
use mapcraft_map::{Entity, Map};
use mapcraft_render::{MAX_MODEL_BYTES, Model, Renderer, load_gltf};
use mapcraft_sdk::Vfs;

use crate::app::MapCraftApp;
use crate::viewport::Tool;

const EXTENSIONS: [&str; 2] = ["glb", "gltf"];
/// Pixels on a side of a model's picture, twice its tile so it stays sharp on high DPI screens.
const PREVIEW_SIDE: u32 = crate::thumbs::SIZE * 2;
/// Pictures read from the cache or rendered per frame, so scrolling stays smooth.
const PREVIEWS_PER_FRAME: usize = 4;
/// Models loading at once only to be pictured.
const PREVIEW_LOADS: usize = 4;

type Loaded = (String, Result<Model, String>);

/// The SDK's models: settings from its `[models]` table, loaded models by path, and the browser's state.
pub(crate) struct Models {
    pub vfs: Vfs,
    pub units_per_meter: f64,
    /// The point entity class the Models tab places.
    pub class: Option<String>,
    pub key: String,
    /// Models classes show as their own, which the Models tab does not offer as props.
    fixed: BTreeSet<String>,
    /// `None` for a model that failed to load.
    loaded: HashMap<String, Option<Arc<Model>>>,
    pending: HashSet<String>,
    worker: Option<(mpsc::Sender<String>, mpsc::Receiver<Loaded>)>,
    /// Pending models a plugin reads, which load on the editor's thread where plugins live.
    plugin_queue: Vec<String>,
    files: Option<Vec<String>>,
    pub filter: String,
    /// The model the Entity tool places.
    pub chosen: Option<String>,
    /// The Models tab's pictures by path; `None` for a model that cannot be pictured.
    previews: HashMap<String, Option<TextureHandle>>,
    /// Models loading only to be pictured, which leave memory once they are.
    previewing: HashSet<String>,
}

impl Default for Models {
    fn default() -> Self {
        Self {
            vfs: Vfs::default(),
            units_per_meter: 39.37,
            class: None,
            key: "model".into(),
            fixed: BTreeSet::new(),
            loaded: HashMap::new(),
            pending: HashSet::new(),
            worker: None,
            plugin_queue: Vec::new(),
            files: None,
            filter: String::new(),
            chosen: None,
            previews: HashMap::new(),
            previewing: HashSet::new(),
        }
    }
}

/// A model key value as a path in the model folders, when it names a glTF file; Godot's `res://` is dropped.
pub(crate) fn model_file(raw: &str) -> Option<String> {
    let path = raw.trim().replace('\\', "/");
    let path = path.strip_prefix("res://").unwrap_or(&path).trim_start_matches('/');
    let ext = path.rsplit_once('.')?.1;
    EXTENSIONS.iter().any(|e| e.eq_ignore_ascii_case(ext)).then(|| path.to_string())
}

/// `uri` taken from the folder `path` is in, with `..` resolved; `None` when it climbs out.
pub(crate) fn beside(path: &str, uri: &str) -> Option<String> {
    let mut parts: Vec<&str> = path.split('/').collect();
    parts.pop();
    for c in uri.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            c => parts.push(c),
        }
    }
    Some(parts.join("/"))
}

pub(crate) fn read_capped(vfs: &Vfs, rel: &str) -> Option<Vec<u8>> {
    let file = vfs.find(rel)?;
    (std::fs::metadata(&file).ok()?.len() <= MAX_MODEL_BYTES).then(|| std::fs::read(&file).ok())?
}

fn load(vfs: &Vfs, path: &str, units_per_meter: f64) -> Result<Model, String> {
    let bytes = read_capped(vfs, path).ok_or("missing from the SDK's model folders, or too big")?;
    load_gltf(path, &bytes, &|uri| read_capped(vfs, &beside(path, uri)?), units_per_meter)
}

/// Where the picture of the model file `model` is cached in `dir`, named by the file's path, size
/// and modified time so an edited model is pictured again.
fn preview_file(dir: &Path, model: &Path) -> Option<PathBuf> {
    let meta = std::fs::metadata(model).ok()?;
    let modified = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos();
    // DefaultHasher may change between Rust releases, which only pictures every model again.
    let mut hash = std::hash::DefaultHasher::new();
    (model, meta.len(), modified).hash(&mut hash);
    Some(dir.join(format!("{:016x}.png", hash.finish())))
}

/// A cached picture; `Some(None)` for a model that failed to load and `None` when there is no usable one.
fn read_preview(file: &Path) -> Option<Option<ColorImage>> {
    let bytes = std::fs::read(file).ok()?;
    if bytes.is_empty() {
        return Some(None);
    }
    let img = std::panic::catch_unwind(|| image::load_from_memory(&bytes)).ok()?.ok()?.to_rgba8();
    Some(Some(ColorImage::from_rgba_premultiplied([img.width() as usize, img.height() as usize], img.as_raw())))
}

/// Caches a picture, or an empty file for a model that failed to load; a failed write only means picturing it again.
fn write_preview(file: &Path, rgba: Option<&[u8]>) {
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = match rgba {
        Some(px) => image::save_buffer(file, px, PREVIEW_SIDE, PREVIEW_SIDE, image::ExtendedColorType::Rgba8).map_err(|e| e.to_string()),
        None => std::fs::write(file, []).map_err(|e| e.to_string()),
    };
}

impl Models {
    pub fn new(vfs: Vfs, sdk: &mapcraft_sdk::Manifest, game: &mapcraft_fgd::GameData) -> Self {
        let fixed = game.class_names().filter_map(|c| model_file(&game.class(c)?.model()?)).collect();
        Self { vfs, units_per_meter: sdk.units_per_meter, class: sdk.models.class.clone(), key: sdk.models.key.clone(), fixed, ..Self::default() }
    }

    pub fn get(&self, path: &str) -> Option<Arc<Model>> {
        self.loaded.get(path)?.clone()
    }

    pub fn busy(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Queues `path` for loading unless it is loaded or queued already.
    pub fn request(&mut self, path: &str) {
        self.previewing.remove(path);
        if self.loaded.contains_key(path) || self.pending.contains(path) {
            return;
        }
        if model_file(path).is_none() {
            self.pending.insert(path.to_string());
            self.plugin_queue.push(path.to_string());
            return;
        }
        let (vfs, units) = (self.vfs.clone(), self.units_per_meter);
        let (requests, _) = self.worker.get_or_insert_with(|| {
            let (requests, rx) = mpsc::channel::<String>();
            let (tx, results) = mpsc::channel();
            // Ends when the sender is dropped with the project.
            std::thread::spawn(move || {
                for path in rx {
                    let model = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| load(&vfs, &path, units)))
                        .unwrap_or_else(|_| Err("the loader crashed".into()));
                    if tx.send((path, model)).is_err() {
                        break;
                    }
                }
            });
            (requests, results)
        });
        if requests.send(path.to_string()).is_ok() {
            self.pending.insert(path.to_string());
        }
    }

    /// Takes finished loads, reporting failures of the ones the views wait for; true when any of those arrived.
    pub fn poll(&mut self, problems: &mut Vec<String>) -> bool {
        let Some((_, results)) = &self.worker else { return false };
        let done: Vec<Loaded> = results.try_iter().collect();
        let mut any = false;
        for (path, result) in done {
            self.pending.remove(&path);
            let preview = self.previewing.contains(&path);
            if let (Err(e), false) = (&result, preview) {
                problems.push(format!("{path}: {e}; showing a box instead"));
            }
            self.loaded.insert(path, result.ok().map(Arc::new));
            any |= !preview;
        }
        any
    }

    /// Loads `path` only to picture it, so it leaves memory once it is.
    fn request_preview(&mut self, path: &str) {
        self.request(path);
        if self.pending.contains(path) {
            self.previewing.insert(path.to_string());
        }
    }

    /// Pictures models that finished loading, then reads the `visible` ones still without a picture
    /// from the cache in `cache` or starts loading them, a few per frame.
    fn make_previews(&mut self, ctx: &egui::Context, renderer: Option<&Renderer>, cache: Option<&Path>, visible: &[String]) {
        let mut budget = PREVIEWS_PER_FRAME;
        let ready: Vec<String> = self.previewing.iter().chain(visible).filter(|p| self.loaded.contains_key(*p)).cloned().collect();
        for path in ready {
            if budget == 0 {
                return;
            }
            if self.previews.contains_key(&path) {
                continue;
            }
            budget -= 1;
            let model = if self.previewing.remove(&path) { self.loaded.remove(&path).flatten() } else { self.get(&path) };
            let side = PREVIEW_SIDE as usize;
            let rgba = model.as_ref().zip(renderer).and_then(|(m, r)| r.model_preview(m, PREVIEW_SIDE)).filter(|px| px.len() == side * side * 4);
            let file = cache.zip(self.vfs.find(&path)).and_then(|(dir, f)| preview_file(dir, &f));
            if let Some(file) = file.filter(|_| model.is_none() || rgba.is_some()) {
                write_preview(&file, rgba.as_deref());
            }
            self.show_preview(ctx, path, rgba.map(|px| ColorImage::from_rgba_premultiplied([side, side], &px)));
        }
        for path in visible {
            if budget == 0 {
                return;
            }
            if self.previews.contains_key(path) || self.loaded.contains_key(path) || self.pending.contains(path) {
                continue;
            }
            let file = cache.zip(self.vfs.find(path)).and_then(|(dir, f)| preview_file(dir, &f));
            match file.as_deref().and_then(read_preview) {
                Some(image) => {
                    budget -= 1;
                    self.show_preview(ctx, path.clone(), image);
                }
                None if self.previewing.len() < PREVIEW_LOADS => self.request_preview(path),
                None => {}
            }
        }
    }

    fn show_preview(&mut self, ctx: &egui::Context, path: String, image: Option<ColorImage>) {
        let texture = image.map(|i| ctx.load_texture(format!("model:{path}"), i, egui::TextureOptions::LINEAR));
        self.previews.insert(path, texture);
    }

    /// Every glTF file in the model folders and every model `plugins` list, but the classes' own,
    /// listed the first time it is asked for.
    fn files(&mut self, plugins: &mut [mapcraft_plugin_host::Plugin], problems: &mut Vec<String>) -> &[String] {
        let (vfs, fixed) = (&self.vfs, &self.fixed);
        self.files.get_or_insert_with(|| {
            let exts = EXTENSIONS.map(String::from);
            let names = vfs.list(&exts).into_iter().filter_map(|(name, path)| Some(format!("{name}.{}", path.extension()?.to_string_lossy())));
            let mut names: Vec<String> = names.collect();
            for plugin in plugins {
                match plugin.list_models() {
                    Ok(list) => names.extend(list),
                    Err(e) => problems.push(e.to_string()),
                }
            }
            names.into_iter().filter(|n| !fixed.contains(n)).collect()
        })
    }
}

impl crate::project::Project {
    /// The model a point entity shows: its class's fixed model, else its model key; a glTF file,
    /// or a file a loaded plugin reads.
    pub fn model_path(&self, e: &Entity) -> Option<String> {
        let fixed = self.game.class(&e.classname).and_then(|c| c.model());
        let raw = fixed.or_else(|| e.get(&self.models.key).map(str::to_string))?;
        model_file(&raw).or_else(|| {
            let path = raw.trim().replace('\\', "/");
            self.plugins.iter().any(|p| p.reads_model(&path)).then_some(path)
        })
    }

    /// Queues the models `map` uses and uploads the textures of loaded ones the renderer lacks.
    /// Models plugins read load right away, since a plugin cannot leave the editor's thread.
    pub fn prepare_models(&mut self, map: &Map, renderer: &mut Renderer, problems: &mut Vec<String>) {
        let paths: BTreeSet<String> = map.nodes().filter(|(_, n)| n.children.is_empty()).filter_map(|(_, n)| self.model_path(n.kind.entity()?)).collect();
        for p in paths {
            if model_file(&p).is_some() {
                self.models.request(&p);
            } else if !self.models.loaded.contains_key(&p) {
                let model = self.plugin_model(&p, problems).map_err(|e| problems.push(format!("{p}: {e}; showing a box instead"))).ok();
                self.models.loaded.insert(p, model.map(Arc::new));
            }
        }
        for m in self.models.loaded.values().flatten() {
            for (i, t) in m.textures.iter().enumerate() {
                let name = m.texture_name(i);
                if !renderer.has_material(&name) {
                    renderer.upload_material(&name, t.width, t.height, &t.rgba);
                }
            }
        }
    }

    /// Loads a few of the models plugins read that the Models tab asked for; true when any loaded.
    pub fn load_queued_models(&mut self, problems: &mut Vec<String>) -> bool {
        let take = self.models.plugin_queue.len().min(PREVIEWS_PER_FRAME);
        let paths: Vec<String> = self.models.plugin_queue.drain(..take).collect();
        for path in &paths {
            let model = self.plugin_model(path, problems);
            self.models.pending.remove(path);
            if let (Err(e), false) = (&model, self.models.previewing.contains(path)) {
                problems.push(format!("{path}: {e}; showing a box instead"));
            }
            self.models.loaded.insert(path.clone(), model.ok().map(Arc::new));
        }
        !paths.is_empty()
    }

    fn plugin_model(&mut self, path: &str, problems: &mut Vec<String>) -> Result<Model, String> {
        let plugin = self.plugins.iter_mut().find(|p| p.reads_model(path)).ok_or("no plugin reads it")?;
        let m = plugin.load_model(path).map_err(|e| e.to_string());
        problems.extend(plugin.take_log());
        let m = m?;
        let textures = m.textures.into_iter().map(|t| mapcraft_render::Picture { width: t.width, height: t.height, rgba: t.rgba }).collect();
        let parts =
            m.parts.into_iter().map(|p| mapcraft_render::MeshPart { texture: p.texture, positions: p.positions, uvs: p.uvs, indices: p.indices }).collect();
        mapcraft_render::mesh_model(path, parts, textures)
    }

    /// Keys for a new point entity, with the model picked in the Models tab when `class` is the SDK's prop class.
    pub fn with_model(&self, class: &str, mut keys: Vec<(String, String)>) -> Vec<(String, String)> {
        let m = &self.models;
        if let (Some(path), Some(c)) = (&m.chosen, &m.class)
            && c.eq_ignore_ascii_case(class)
        {
            keys.retain(|(k, _)| *k != m.key);
            keys.push((m.key.clone(), path.clone()));
        }
        keys
    }
}

impl MapCraftApp {
    /// Each model the Models tab pictured, and whether it got a picture, for tests.
    pub fn model_previews_for_test(&self) -> Vec<(String, bool)> {
        let mut out: Vec<(String, bool)> = self.project.models.previews.iter().map(|(p, t)| (p.clone(), t.is_some())).collect();
        out.sort();
        out
    }

    pub(crate) fn poll_models(&mut self, ctx: &egui::Context) {
        if self.project.models.poll(&mut self.output) | self.project.load_queued_models(&mut self.output) {
            self.scene_dirty = true;
        }
        if self.project.models.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    pub(crate) fn model_browser(&mut self, ui: &mut egui::Ui) {
        if self.project.sdk.is_none() {
            ui.label("No SDK: pick one with File > New Map to browse models.");
            return;
        }
        let cache = self.data_dir.as_ref().zip(self.project.id()).map(|(d, id)| d.join("model-previews").join(id));
        let renderer = self.gpu.as_ref().map(|g| &g.renderer);
        let sdk = self.project.id().map(str::to_string);
        let mut hidden = sdk.as_ref().and_then(|id| self.prefs.get().hidden_models.get(id).cloned()).unwrap_or_default();
        let (models, plugins) = (&mut self.project.models, &mut self.project.plugins);
        crate::panels::search_field(ui, &mut models.filter, "Search models");
        let filter = models.filter.to_lowercase();
        let folder = |n: &str| n.rsplit_once('/').map_or("", |(f, _)| f).to_string();
        let all = models.files(plugins, &mut self.output);
        let mut folders: BTreeMap<String, usize> = BTreeMap::new();
        for n in all {
            *folders.entry(folder(n)).or_default() += 1;
        }
        let names: Vec<String> = all
            .iter()
            .filter(|n| !crate::panels::in_hidden_folder(&folder(n), &hidden))
            .filter(|n| filter.is_empty() || n.to_lowercase().contains(&filter))
            .cloned()
            .collect();
        let label = if hidden.is_empty() { "Folders".to_string() } else { format!("Folders ({} hidden)", hidden.len()) };
        let tree = egui::CollapsingHeader::new(label)
            .id_salt("model-folders")
            .show(ui, |ui| crate::panels::folder_tree(ui, "model-folder-tree", &folders, &mut hidden, &mut models.filter));
        if let (Some(true), Some(id)) = (tree.body_returned, sdk) {
            self.prefs.update(|p| {
                p.hidden_models.insert(id, hidden);
            });
        }
        crate::panels::count_and_previews(ui, &format!("{} models", names.len()), &mut self.show_model_thumbs);
        if models.class.is_none() {
            ui.label(
                RichText::new("This SDK has no prop class, so models can be previewed but not placed. Set `class` under `[models]` in its sdk.toml.").weak(),
            );
        }
        if let Some(path) = &models.chosen {
            let about = match (models.loaded.get(path), models.pending.contains(path)) {
                (Some(Some(m)), _) => {
                    let size = m.max - m.min;
                    format!("{} triangles, {:.0} x {:.0} x {:.0} units", m.triangles(), size.x, size.y, size.z)
                }
                (Some(None), _) => "Cannot load; see the Output panel".into(),
                (None, true) => "Loading…".into(),
                (None, false) => String::new(),
            };
            ui.label(RichText::new(about).small());
        }
        ui.separator();
        let mut pick = None;
        let mut missing = Vec::new();
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        if self.show_model_thumbs {
            crate::panels::tile_grid(ui, &names, |ui, name| {
                let size = egui::Vec2::splat(crate::thumbs::SIZE as f32);
                let selected = models.chosen.as_ref() == Some(name);
                let r = match models.previews.get(name) {
                    Some(Some(t)) => ui.add(egui::Button::image(egui::Image::new((t.id(), size))).selected(selected)),
                    Some(None) => {
                        let icon = crate::icons::icon("cube").fit_to_exact_size(egui::Vec2::splat(32.0)).tint(crate::chrome::TEXT_WEAK);
                        ui.add_sized(size, egui::Button::image(icon).selected(selected))
                    }
                    None => {
                        missing.push(name.clone());
                        ui.add_sized(size, egui::Button::new("…").selected(selected))
                    }
                };
                if r.on_hover_text(name).clicked() {
                    pick = Some(name.clone());
                }
                ui.add(egui::Label::new(RichText::new(name.rsplit('/').next().unwrap_or(name)).small()).truncate());
            });
        } else {
            egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, 18.0, names.len(), |ui, range| {
                for name in names.get(range).unwrap_or_default() {
                    if ui.selectable_label(models.chosen.as_ref() == Some(name), name).clicked() {
                        pick = Some(name.clone());
                    }
                }
            });
        }
        models.make_previews(ui.ctx(), renderer, cache.as_deref(), &missing);
        if !missing.is_empty() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(30));
        }
        let Some(name) = pick else { return };
        models.request(&name);
        models.chosen = Some(name.clone());
        if let Some(class) = models.class.clone() {
            self.entity_class = class;
            self.tool = Tool::Entity;
            self.status = format!("Click in a view to place {name}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_paths() {
        assert_eq!(model_file(" res://Assets/Props/Crate.GLB ").as_deref(), Some("Assets/Props/Crate.GLB"));
        assert_eq!(model_file("props\\rock.gltf").as_deref(), Some("props/rock.gltf"));
        assert_eq!(model_file("models/props/crate.mdl"), None);
        assert_eq!(beside("props/rock.gltf", "../textures/rock%20a.png").as_deref(), Some("textures/rock%20a.png"));
        assert_eq!(beside("rock.gltf", "../x.bin"), None);

        let mut p = crate::project::Project::default();
        p.game = mapcraft_fgd::GameData::parse_str("@PointClass studio(\"fixed.glb\") = fixed []\n@PointClass studio() = prop []");
        let mut e = Entity::new("prop");
        assert_eq!(p.model_path(&e), None);
        e.set("model", "res://crate.glb");
        assert_eq!(p.model_path(&e).as_deref(), Some("crate.glb"));
        e.classname = "fixed".into();
        assert_eq!(p.model_path(&e).as_deref(), Some("fixed.glb"), "the class's own model wins");
        e.classname = "unknown".into();
        assert_eq!(p.model_path(&e).as_deref(), Some("crate.glb"), "any entity with a model key shows it");

        p.models.class = Some("prop".into());
        p.models.chosen = Some("rock.glb".into());
        let keys = p.with_model("prop", vec![("model".into(), String::new()), ("angles".into(), "0 0 0".into())]);
        assert_eq!(keys, [("angles".to_string(), "0 0 0".to_string()), ("model".to_string(), "rock.glb".to_string())]);
        assert!(p.with_model("light", Vec::new()).is_empty());
    }

    #[test]
    fn a_changed_model_gets_a_new_picture() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("crate.glb");
        std::fs::write(&model, b"glTF").unwrap();
        let file = std::fs::File::options().write(true).open(&model).unwrap();
        let then = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        file.set_modified(then).unwrap();
        let before = preview_file(dir.path(), &model).unwrap();
        assert_eq!(preview_file(dir.path(), &model).unwrap(), before, "the same file keeps its picture");
        file.set_modified(then + std::time::Duration::from_secs(1)).unwrap();
        assert_ne!(preview_file(dir.path(), &model).unwrap(), before, "a newer file is pictured again");
        assert_eq!(preview_file(dir.path(), &dir.path().join("missing.glb")), None);

        write_preview(&before, None);
        assert!(matches!(read_preview(&before), Some(None)), "a model that failed to load is remembered");
        write_preview(&before, Some(&[255; (PREVIEW_SIDE * PREVIEW_SIDE * 4) as usize]));
        assert_eq!(read_preview(&before).unwrap().unwrap().size, [PREVIEW_SIDE as usize; 2]);
        std::fs::write(&before, b"junk").unwrap();
        assert!(read_preview(&before).is_none(), "a broken cache file is pictured again");
    }

    #[test]
    fn loads_models_off_the_ui_thread() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("broken.glb"), b"glTF\x02\0\0\0junk").unwrap();
        let mut m = Models { vfs: Vfs::new(vec![dir.path().to_path_buf()]), ..Models::default() };
        m.request("broken.glb");
        m.request("missing.glb");
        let mut problems = Vec::new();
        let start = std::time::Instant::now();
        while m.busy() && start.elapsed().as_secs() < 10 {
            std::thread::sleep(std::time::Duration::from_millis(5));
            m.poll(&mut problems);
        }
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(m.get("broken.glb").is_none() && !m.busy());
    }
}
