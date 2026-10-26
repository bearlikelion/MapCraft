//! glTF models for point entities: loaded on a worker thread for the views, and the Models tab.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, mpsc};

use egui::RichText;
use mapcraft_map::{Entity, Map};
use mapcraft_render::{MAX_MODEL_BYTES, Model, Renderer, load_gltf};
use mapcraft_sdk::Vfs;

use crate::app::MapCraftApp;
use crate::viewport::Tool;

const EXTENSIONS: [&str; 2] = ["glb", "gltf"];

type Loaded = (String, Result<Model, String>);

/// The SDK's models: settings from its `[models]` table, loaded models by path, and the browser's state.
pub(crate) struct Models {
    pub vfs: Vfs,
    pub units_per_meter: f64,
    /// The point entity class the Models tab places.
    pub class: Option<String>,
    pub key: String,
    /// `None` for a model that failed to load.
    loaded: HashMap<String, Option<Arc<Model>>>,
    pending: HashSet<String>,
    worker: Option<(mpsc::Sender<String>, mpsc::Receiver<Loaded>)>,
    files: Option<Vec<String>>,
    pub filter: String,
    /// The model the Entity tool places.
    pub chosen: Option<String>,
}

impl Default for Models {
    fn default() -> Self {
        Self {
            vfs: Vfs::default(),
            units_per_meter: 39.37,
            class: None,
            key: "model".into(),
            loaded: HashMap::new(),
            pending: HashSet::new(),
            worker: None,
            files: None,
            filter: String::new(),
            chosen: None,
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
fn beside(path: &str, uri: &str) -> Option<String> {
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

fn read_capped(vfs: &Vfs, rel: &str) -> Option<Vec<u8>> {
    let file = vfs.find(rel)?;
    (std::fs::metadata(&file).ok()?.len() <= MAX_MODEL_BYTES).then(|| std::fs::read(&file).ok())?
}

fn load(vfs: &Vfs, path: &str, units_per_meter: f64) -> Result<Model, String> {
    let bytes = read_capped(vfs, path).ok_or("missing from the SDK's model folders, or too big")?;
    load_gltf(path, &bytes, &|uri| read_capped(vfs, &beside(path, uri)?), units_per_meter)
}

impl Models {
    pub fn new(vfs: Vfs, sdk: &mapcraft_sdk::Manifest) -> Self {
        Self { vfs, units_per_meter: sdk.units_per_meter, class: sdk.models.class.clone(), key: sdk.models.key.clone(), ..Self::default() }
    }

    pub fn get(&self, path: &str) -> Option<Arc<Model>> {
        self.loaded.get(path)?.clone()
    }

    pub fn busy(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Queues `path` for loading unless it is loaded or queued already.
    pub fn request(&mut self, path: &str) {
        if self.loaded.contains_key(path) || self.pending.contains(path) {
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

    /// Takes finished loads, reporting failures; true when any arrived.
    pub fn poll(&mut self, problems: &mut Vec<String>) -> bool {
        let Some((_, results)) = &self.worker else { return false };
        let done: Vec<Loaded> = results.try_iter().collect();
        let any = !done.is_empty();
        for (path, result) in done {
            self.pending.remove(&path);
            let model = result.map_err(|e| problems.push(format!("{path}: {e}; showing a box instead"))).ok();
            self.loaded.insert(path, model.map(Arc::new));
        }
        any
    }

    /// Every glTF file in the model folders, listed the first time it is asked for.
    fn files(&mut self) -> &[String] {
        let vfs = &self.vfs;
        self.files.get_or_insert_with(|| {
            let exts = EXTENSIONS.map(String::from);
            vfs.list(&exts).into_iter().filter_map(|(name, path)| Some(format!("{name}.{}", path.extension()?.to_string_lossy()))).collect()
        })
    }
}

impl crate::project::Project {
    /// The glTF model a point entity shows: its class's fixed model, else its model key.
    pub fn model_path(&self, e: &Entity) -> Option<String> {
        let fixed = self.game.class(&e.classname).and_then(|c| c.model());
        model_file(&fixed.or_else(|| e.get(&self.models.key).map(str::to_string))?)
    }

    /// Queues the models `map` uses and uploads the textures of loaded ones the renderer lacks.
    pub fn prepare_models(&mut self, map: &Map, renderer: &mut Renderer) {
        let paths: BTreeSet<String> = map.nodes().filter(|(_, n)| n.children.is_empty()).filter_map(|(_, n)| self.model_path(n.kind.entity()?)).collect();
        for p in paths {
            self.models.request(&p);
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
    pub(crate) fn poll_models(&mut self, ctx: &egui::Context) {
        if self.project.models.poll(&mut self.output) {
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
        let models = &mut self.project.models;
        ui.add(egui::TextEdit::singleline(&mut models.filter).hint_text("Search models"));
        let filter = models.filter.to_lowercase();
        let names: Vec<String> = models.files().iter().filter(|n| filter.is_empty() || n.to_lowercase().contains(&filter)).cloned().collect();
        ui.label(format!("{} models", names.len()));
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
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, 18.0, names.len(), |ui, range| {
            for name in names.get(range).unwrap_or_default() {
                if ui.selectable_label(models.chosen.as_ref() == Some(name), name).clicked() {
                    pick = Some(name.clone());
                }
            }
        });
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
