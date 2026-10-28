//! Baked lighting for the Lit view: an SDK's bake profile compiles the map in the background, and the
//! plugin that reads the result gives back the compiled faces with their lightmaps.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use mapcraft_map::Map;
use mapcraft_render::{Bake, BakePart, Picture, RenderMode};

use crate::app::MapCraftApp;

/// How long edits pause before the Lit view bakes again.
const SETTLE: Duration = Duration::from_millis(600);

#[derive(Default)]
pub(crate) struct BakeState {
    /// The last change to the scene, which a bake waits to settle.
    changed: Option<Instant>,
    job: Option<Job>,
    /// The export the shown bake, or the last one that failed, was made from.
    baked: Option<Vec<u8>>,
    /// The map file the shown bake belongs to.
    map: Option<PathBuf>,
    /// [`brush_print`] of the brushes on screen, and of those the shown bake was made from.
    brushes: u64,
    shown: Option<u64>,
}

impl BakeState {
    pub(crate) fn changed(&mut self) {
        self.changed = Some(Instant::now());
    }

    /// Notes the brushes on screen; true while they differ from those the shown bake was made from.
    pub(crate) fn stale(&mut self, map: &Map) -> bool {
        self.brushes = brush_print(map);
        self.shown.is_some_and(|b| b != self.brushes)
    }
}

/// A fingerprint of every brush face's corners, material and alignment.
fn brush_print(map: &Map) -> u64 {
    let mut h = DefaultHasher::new();
    for (id, n) in map.nodes() {
        let Some(b) = n.kind.brush() else { continue };
        id.0.hash(&mut h);
        for f in &b.faces {
            let numbers =
                f.points.iter().chain([&f.u.axis, &f.v.axis]).flat_map(|p| p.to_array()).chain([f.u.offset, f.u.scale, f.v.offset, f.v.scale, f.rotation]);
            numbers.for_each(|x| x.to_bits().hash(&mut h));
            f.material.hash(&mut h);
        }
    }
    h.finish()
}

struct Job {
    rx: mpsc::Receiver<String>,
    done: Arc<AtomicBool>,
    ok: Arc<AtomicBool>,
    export: Vec<u8>,
    /// The bake's output, as a path the SDK's plugins can read.
    file: String,
    log: Vec<String>,
    manual: bool,
    brushes: u64,
}

impl MapCraftApp {
    fn bake_profile(&self) -> Option<usize> {
        self.project.sdk.as_ref()?.manifest.compile.iter().position(|p| p.bake.is_some())
    }

    /// Bakes the map with compile profile `index` without saving it; an automatic bake skips a map that exports as it did last time.
    pub(crate) fn start_bake(&mut self, index: usize, manual: bool) {
        if self.bake.job.is_some() {
            if manual {
                self.status = "Lighting is already baking".into();
            }
            return;
        }
        let Some(sdk) = self.project.sdk.clone() else { return };
        let Some(profile) = sdk.manifest.compile.get(index).cloned() else { return };
        let Some(file) = profile.bake.clone() else { return };
        let Some(dir) = self.project.vars.get("bake_dir").map(PathBuf::from) else {
            self.status = "Lighting cannot be baked without a settings folder".into();
            return;
        };
        let stem = self.session.path.as_ref().and_then(|p| p.file_stem()).map_or("untitled".into(), |s| s.to_string_lossy().into_owned());
        let mut vars = self.project.vars.clone();
        vars.extend(mapcraft_sdk::builtin_vars(&dir.join(format!("{stem}.mcmap")), &sdk.dir));
        let (mut export, mut log) = (Vec::new(), Vec::new());
        let mut written = mapcraft_sdk::expand(&file, &vars).map_err(|e| e.to_string());
        for step in &profile.steps {
            let mapcraft_sdk::Step::Export { export: id, to } = step else { continue };
            written = written.and_then(|file| {
                let to = mapcraft_sdk::expand(to, &vars).map_err(|e| e.to_string())?;
                let bytes = self.export_bytes(id, &mut log)?;
                std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&to, &bytes)).map_err(|e| format!("cannot write {to}: {e}"))?;
                export.extend(bytes);
                Ok(file)
            });
        }
        let file = match written {
            Ok(file) => file,
            Err(e) => {
                self.bake.baked = Some(export);
                self.bake_failed(&log, &e, manual);
                return;
            }
        };
        if !manual && self.bake.baked.as_ref() == Some(&export) {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let (done, ok) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
        let (d, o) = (done.clone(), ok.clone());
        std::thread::spawn(move || {
            let result = mapcraft_sdk::run_profile(
                &profile,
                &vars,
                &mut |line| {
                    let _ = tx.send(line.to_string());
                },
                &AtomicBool::new(false),
            );
            match result {
                Ok(()) => o.store(true, Ordering::Relaxed),
                Err(e) => {
                    let _ = tx.send(format!("== failed: {e} =="));
                }
            }
            d.store(true, Ordering::Relaxed);
        });
        if manual {
            self.status = "Baking lighting".into();
        }
        self.bake.job = Some(Job { rx, done, ok, export, file, log, manual, brushes: self.bake.brushes });
    }

    /// Shows a finished bake, and starts the next once edits settle while a view is Lit.
    pub(crate) fn poll_bake(&mut self, ctx: &egui::Context) {
        if self.bake.map != self.session.path {
            self.bake.map = self.session.path.clone();
            self.bake.baked = None;
            self.show_bake(None);
        }
        if let Some(job) = &self.bake.job {
            if !job.done.load(Ordering::Relaxed) {
                ctx.request_repaint_after(Duration::from_millis(100));
                return;
            }
            let Some(mut job) = self.bake.job.take() else { return };
            job.log.extend(job.rx.try_iter());
            self.bake.baked = Some(std::mem::take(&mut job.export));
            if !job.ok.load(Ordering::Relaxed) {
                self.bake_failed(&job.log, "a compile step failed", job.manual);
                return;
            }
            match self.read_bake(&job.file) {
                Ok(bake) => {
                    self.bake.shown = Some(job.brushes);
                    self.show_bake(Some(&bake));
                    if job.manual {
                        self.output.extend(job.log);
                        self.status = "Baked lighting".into();
                    }
                }
                Err(e) => self.bake_failed(&job.log, &e, job.manual),
            }
            return;
        }
        let Some(index) = self.bake_profile().filter(|_| self.render_mode == RenderMode::Lit) else { return };
        let approved = self.project.id().is_some_and(|id| self.prefs.get().approved_tools.contains(id));
        let wanted = self.bake.changed.is_some() || self.bake.baked.is_none();
        match self.bake.changed {
            Some(t) if t.elapsed() < SETTLE => ctx.request_repaint_after(SETTLE.saturating_sub(t.elapsed())),
            _ if approved && wanted => {
                self.bake.changed = None;
                self.start_bake(index, false);
            }
            _ => {}
        }
    }

    fn bake_failed(&mut self, log: &[String], error: &str, manual: bool) {
        self.show_bake(None);
        self.output.extend(log.iter().cloned());
        self.output.push(format!("Lighting bake failed: {error}; the Lit view shows live lights until the next bake"));
        if manual {
            self.status = format!("Lighting bake failed: {error}");
        }
    }

    fn show_bake(&mut self, bake: Option<&Bake>) {
        let Some(gpu) = &mut self.gpu else { return };
        if let Some(b) = bake {
            self.project.upload_materials(&mut gpu.renderer, b.parts.iter().map(|p| p.material.as_str()));
        }
        gpu.renderer.set_bake(bake);
        gpu.renderer.set_bake_stale(bake.is_some() && self.bake.shown != Some(self.bake.brushes));
    }

    /// The compiled faces a plugin reads from `file`, keeping only the pictures they use as lightmaps.
    fn read_bake(&mut self, file: &str) -> Result<Bake, String> {
        let plugin = self.project.plugins.iter_mut().find(|p| p.reads_model(file)).ok_or_else(|| format!("no plugin reads {file}"))?;
        let model = plugin.load_model(file).map_err(|e| e.to_string());
        self.output.extend(plugin.take_log());
        let model = model?;
        let mut lightmaps: Vec<Picture> = Vec::new();
        let mut index_of = vec![None; model.textures.len()];
        let mut parts = Vec::new();
        for p in model.parts {
            let Some(material) = p.material else { continue };
            let lightmap = p.lightmap.and_then(|l| {
                let slot = index_of.get_mut(l)?;
                if slot.is_none() {
                    let t = model.textures.get(l)?;
                    lightmaps.push(Picture { width: t.width, height: t.height, rgba: t.rgba.clone() });
                    *slot = Some(lightmaps.len() - 1);
                }
                *slot
            });
            let positions = p.positions.iter().map(|v| v.as_vec3().to_array()).collect();
            parts.push(BakePart { material, lightmap, positions, uvs: p.uvs, lightmap_uvs: p.lightmap_uvs, indices: p.indices });
        }
        if parts.is_empty() {
            return Err(format!("{file} has no faces"));
        }
        Ok(Bake { parts, lightmaps })
    }
}
