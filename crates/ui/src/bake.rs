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
    native: Option<Native>,
    /// [`mapcraft_light::fingerprint`] of the map the last native bake started from.
    print: Option<u64>,
    /// [`mapcraft_light::fingerprint`] of the map the Lit view's bake was made from.
    shown_print: Option<u64>,
    /// The map has changed since the shown bake, so its lighting is out of date.
    lighting_stale: bool,
    /// A compile profile waiting for the full bake to finish before it packs the map.
    pub(crate) then_compile: Option<usize>,
    /// A full bake asked for while a draft ran; it starts once the draft stops.
    queued: bool,
    /// What the last finished native bake did, until the `bake_lighting` tool reports it.
    last: Option<String>,
    /// Each material's albedo and emission, read once per map.
    surfaces: std::collections::HashMap<String, mapcraft_light::Surface>,
}

/// A bake by MapCraft's own baker, running on its own thread.
struct Native {
    job: std::thread::JoinHandle<Result<(mapcraft_light::Lightmap, Vec<String>, Bake), mapcraft_light::BakeError>>,
    progress: Arc<mapcraft_light::Progress>,
    manual: bool,
    brushes: u64,
    print: u64,
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
    /// Whether the SDK bakes with MapCraft's own baker.
    pub(crate) fn native_lightmaps(&self) -> bool {
        self.project.sdk.as_ref().is_some_and(|s| s.manifest.lightmaps.is_some())
    }

    fn lightmap_settings(&self, map: &Map) -> mapcraft_light::Settings {
        let mut settings = mapcraft_light::Settings::default();
        if let Some(l) = self.project.sdk.as_ref().and_then(|s| s.manifest.lightmaps.as_ref()) {
            settings.luxel = l.luxel.filter(|v| v.is_finite() && *v >= 1.0).unwrap_or(settings.luxel);
            settings.bounces = l.bounces.map_or(settings.bounces, |b| b.min(16));
            settings.rays = l.rays.map_or(settings.rays, |r| r.clamp(1, 4096));
        }
        settings.for_map(map)
    }

    /// The settings a full bake of `map` uses, leaving cores for the editor, which keeps drawing while it runs.
    fn full_settings(&self, map: &Map) -> mapcraft_light::Settings {
        let mut settings = self.lightmap_settings(map);
        settings.threads = std::thread::available_parallelism().map_or(2, |n| n.get().saturating_sub(2).max(1));
        settings
    }

    /// How a material takes and gives light: its picture's average color, and its emission.
    fn surface(&mut self, material: &str) -> mapcraft_light::Surface {
        if let Some(s) = self.bake.surfaces.get(material) {
            return *s;
        }
        let linear = |v: u8| (f32::from(v) / 255.0).powf(2.2);
        let average = |p: &mapcraft_render::Picture| {
            let px: Vec<&[u8; 4]> = p.rgba.as_chunks::<4>().0.iter().step_by((p.rgba.len() / 4 / 4096).max(1)).collect();
            let n = px.len().max(1) as f32;
            [0, 1, 2].map(|k| px.iter().map(|c| linear(c[k])).sum::<f32>() / n)
        };
        let mut surface = mapcraft_light::Surface::default();
        if let Some(p) = self.project.material_picture(material) {
            surface.albedo = average(&p).map(|v| v.clamp(0.0, 0.95));
        }
        if let Some(m) = self.project.find_material(material)
            && let Some(d) = &m.descriptor
        {
            let glow = m.emission.as_ref().and_then(|path| image::open(path).ok()).map(|i| {
                let i = i.to_rgba8();
                average(&mapcraft_render::Picture { width: i.width(), height: i.height(), rgba: i.into_raw() })
            });
            let base = glow.unwrap_or([0.0; 3]);
            surface.emission = [0, 1, 2].map(|k| (base[k] + d.emission[k]) * d.emission_energy);
        }
        self.bake.surfaces.insert(material.to_string(), surface);
        surface
    }

    /// Bakes the shown map with MapCraft's own baker on a background thread.
    pub(crate) fn start_native_bake(&mut self, manual: bool) {
        if let Some(n) = &self.bake.native {
            // A full bake replaces a running draft; a second request for one is already in hand.
            if manual && !n.manual {
                n.progress.cancel.store(true, Ordering::Relaxed);
                self.bake.queued = true;
            }
            return;
        }
        let map = self.view_map().clone();
        let brushes = brush_print(&map);
        let print = mapcraft_light::fingerprint(&map);
        self.bake.print = Some(print);
        let names: std::collections::BTreeSet<String> =
            map.nodes().filter_map(|(_, n)| n.kind.brush()).flat_map(|b| b.faces.iter().map(|f| f.material.clone())).collect();
        let sizes = self.texture_sizes(names.iter().cloned());
        let surfaces: std::collections::HashMap<String, mapcraft_light::Surface> = names.into_iter().map(|n| (n.clone(), self.surface(&n))).collect();
        let see_through = self.session.see_through.clone();
        let mut settings = self.full_settings(&map);
        if !manual {
            // Automatic bakes are drafts, quick enough to follow edits; Bake Lighting makes the full one.
            settings.luxel = (settings.luxel * 2.0).max(48.0);
            settings.rays = settings.rays.min(8);
            settings.bounces = settings.bounces.min(1);
            settings.sun_samples = settings.sun_samples.min(2);
        }

        let progress = Arc::new(mapcraft_light::Progress::default());
        let watch = progress.clone();
        // Marks the map as tried, so a failed automatic bake waits for the next edit.
        self.bake.baked = Some(Vec::new());
        let job = std::thread::spawn(move || {
            let skip = mapcraft_map::covered_faces(&map, None, &|m| see_through.contains(m));
            let surface = |m: &str| surfaces.get(m).copied().unwrap_or_default();
            let (lightmap, warnings) = mapcraft_light::bake(&map, &surface, &skip, &settings, &watch)?;
            let bake = native_parts(&map, &lightmap, &sizes);
            Ok((lightmap, warnings, bake))
        });
        self.bake.native = Some(Native { job, progress, manual, brushes, print });
    }

    /// Shows a finished native bake; true while one is still running.
    fn poll_native(&mut self, ctx: &egui::Context) -> bool {
        let Some(native) = &self.bake.native else { return false };
        if !native.job.is_finished() {
            ctx.request_repaint_after(Duration::from_millis(250));
            return true;
        }
        let Some(native) = self.bake.native.take() else { return false };
        if std::mem::take(&mut self.bake.queued) {
            self.start_native_bake(true);
            return true;
        }
        let (lightmap, warnings, bake) = match native.job.join() {
            Ok(Ok(r)) => r,
            // A draft stopped because the map changed again; the next one follows the edit.
            Ok(Err(mapcraft_light::BakeError::Cancelled)) if !native.manual => return false,
            Ok(Err(e)) => {
                self.bake.last = Some(format!("Lighting bake failed: {e}"));
                self.bake_failed(&[], &e.to_string(), native.manual);
                self.stop_waiting_compile(&e.to_string());
                return false;
            }
            Err(_) => {
                self.bake_failed(&[], "the bake thread failed", native.manual);
                self.stop_waiting_compile("the bake thread failed");
                return false;
            }
        };
        self.output.extend(warnings.into_iter().map(|w| format!("Lighting: {w}")));
        self.bake.shown_print = Some(native.print);
        self.bake.lighting_stale = self.bake.shown_print != Some(mapcraft_light::fingerprint(self.view_map()));
        self.bake.shown = Some(native.brushes);
        self.show_bake(Some(&bake));
        if native.manual {
            self.status = match self.session.path.as_ref().map(|p| p.with_extension("mclight")) {
                Some(path) => match std::fs::write(&path, mapcraft_light::write(&lightmap)) {
                    Ok(()) => format!("Baked lighting into {}", path.display()),
                    Err(e) => format!("Baked lighting, but cannot write {}: {e}", path.display()),
                },
                None => "Baked lighting; save the map to keep it".into(),
            };
            self.bake.last = Some(self.status.clone());
            if let Some(index) = self.bake.then_compile.take() {
                match self.session.path.clone() {
                    Some(path) if self.saved_bake_current(&path) => self.start_compile(index),
                    _ => self.output.push("== stopped: the lighting bake was not saved beside the map ==".into()),
                }
            }
        }
        false
    }

    /// Whether the `.mclight` beside the map at `path` was baked from the map as it is now.
    pub(crate) fn saved_bake_current(&self, path: &std::path::Path) -> bool {
        let mut header = [0u8; 16];
        let read = std::fs::File::open(path.with_extension("mclight")).and_then(|mut f| std::io::Read::read_exact(&mut f, &mut header));
        let full = self.full_settings(self.view_map());
        read.is_ok() && mapcraft_light::fingerprint_of(&header) == Some(mapcraft_light::bake_fingerprint(self.view_map(), &full))
    }

    /// Drops a compile that was waiting on a bake that did not finish.
    fn stop_waiting_compile(&mut self, why: &str) {
        if self.bake.then_compile.take().is_some() {
            self.output.push(format!("== stopped: the lighting bake failed: {why} =="));
            self.show_bottom = true;
        }
    }

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
            self.bake.surfaces.clear();
            self.bake.print = None;
            self.bake.shown_print = None;
            self.show_bake(None);
            self.load_saved_bake();
        }
        // A draft of a map that has changed since is wasted work, and slows the editor while it runs.
        if self.bake.changed.is_some()
            && let Some(n) = self.bake.native.as_ref().filter(|n| !n.manual && !n.progress.cancel.load(Ordering::Relaxed))
            && self.bake.print != Some(mapcraft_light::fingerprint(self.view_map()))
        {
            n.progress.cancel.store(true, Ordering::Relaxed);
        }
        if self.poll_native(ctx) {
            return;
        }
        if self.native_lightmaps() {
            if !self.prefs.get().view.auto_bake {
                // Without automatic bakes, edits only mark the lighting out of date.
                if self.bake.changed.take().is_some() || self.bake.shown_print.is_none() {
                    self.bake.lighting_stale = self.bake.shown_print != Some(mapcraft_light::fingerprint(self.view_map()));
                }
                return;
            }
            if self.render_mode != RenderMode::Lit {
                return;
            }
            let wanted = self.bake.changed.is_some() || self.bake.baked.is_none();
            if !matches!(self.drag, crate::viewport::Drag::None) {
                return;
            }
            match self.bake.changed {
                Some(t) if t.elapsed() < SETTLE => ctx.request_repaint_after(SETTLE.saturating_sub(t.elapsed())),
                _ if wanted => {
                    self.bake.changed = None;
                    // Selecting and other edits that leave what the bake reads alone need no new bake.
                    if self.bake.print != Some(mapcraft_light::fingerprint(self.view_map())) {
                        self.start_native_bake(false);
                    }
                }
                _ => {}
            }
            return;
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

    /// How far a running native bake has come, from 0 to 1.
    pub(crate) fn bake_progress(&self) -> Option<f32> {
        self.bake.native.as_ref().map(|n| n.progress.fraction())
    }

    /// Shows the `.mclight` saved beside the map when it was baked from the map as it is now.
    fn load_saved_bake(&mut self) {
        if !self.native_lightmaps() {
            return;
        }
        let Some(path) = self.session.path.as_ref().map(|p| p.with_extension("mclight")) else { return };
        let Ok(bytes) = std::fs::read(&path) else { return };
        let lightmap = match mapcraft_light::read(&bytes) {
            Ok(l) => l,
            Err(e) => {
                self.output.push(format!("Lighting: cannot read {}: {e}", path.display()));
                return;
            }
        };
        let print = mapcraft_light::fingerprint(self.view_map());
        if lightmap.fingerprint != mapcraft_light::bake_fingerprint(self.view_map(), &self.full_settings(self.view_map())) {
            return;
        }
        let names: Vec<String> = self.view_map().nodes().filter_map(|(_, n)| n.kind.brush()).flat_map(|b| b.faces.iter().map(|f| f.material.clone())).collect();
        let sizes = self.texture_sizes(names.into_iter());
        let bake = native_parts(self.view_map(), &lightmap, &sizes);
        (self.bake.print, self.bake.shown_print, self.bake.lighting_stale) = (Some(print), Some(print), false);
        self.bake.shown = Some(brush_print(self.view_map()));
        self.bake.baked = Some(Vec::new());
        self.show_bake(Some(&bake));
    }

    /// Texel sizes of `names`, uploading them first so a bake made before the scene loads them is not mapped at 64.
    fn texture_sizes(&mut self, names: impl Iterator<Item = String>) -> std::collections::HashMap<String, mapcraft_geom::DVec2> {
        let names: std::collections::BTreeSet<String> = names.collect();
        if let Some(gpu) = &mut self.gpu {
            self.project.upload_materials(&mut gpu.renderer, names.iter().map(String::as_str));
        }
        let renderer = self.gpu.as_ref().map(|g| &g.renderer);
        names
            .into_iter()
            .map(|n| {
                let size = crate::uv::texture_size(renderer, &n);
                (n, size)
            })
            .collect()
    }

    /// Whether to warn that the lighting is out of date: automatic bakes are off and nothing is baking.
    pub(crate) fn lighting_stale(&self) -> bool {
        self.native_lightmaps() && !self.prefs.get().view.auto_bake && self.bake.native.is_none() && self.bake.lighting_stale
    }

    /// Whether MapCraft's own baker is running.
    pub(crate) fn baking(&self) -> bool {
        self.bake.native.is_some()
    }

    /// The editor's `bake_lighting` tool: starts a bake, or reports the running one or the last result.
    pub(crate) fn bake_tool(&mut self) -> Result<serde_json::Value, String> {
        if !self.native_lightmaps() {
            return Err("this map's SDK does not bake with MapCraft's baker".into());
        }
        if let Some(n) = &self.bake.native {
            return Ok(serde_json::json!({"baking": true, "progress": n.progress.fraction()}));
        }
        if let Some(done) = self.bake.last.take() {
            return Ok(serde_json::json!({"baking": false, "result": done}));
        }
        self.start_native_bake(true);
        Ok(serde_json::json!({"baking": true, "progress": 0.0}))
    }

    /// Cancels a running native bake.
    pub(crate) fn cancel_native_bake(&mut self) {
        if let Some(n) = &self.bake.native {
            n.progress.cancel.store(true, Ordering::Relaxed);
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
        Ok(Bake { parts, lightmaps, linear: false })
    }
}

/// The Lit view's picture of a native bake: every drawn face with its lightmap, and faces without one unlit.
fn native_parts(map: &Map, lm: &mapcraft_light::Lightmap, sizes: &std::collections::HashMap<String, mapcraft_geom::DVec2>) -> Bake {
    let size = |m: &str| sizes.get(m).copied().unwrap_or(mapcraft_geom::DVec2::splat(64.0));
    // Each part draws from one lightmap slice, so parts are kept per material and slice.
    let mut parts: std::collections::BTreeMap<(String, Option<u32>), BakePart> = std::collections::BTreeMap::new();
    for (id, node) in map.nodes() {
        let Some(b) = node.kind.brush() else { continue };
        if b.has_displacement() || mapcraft_map::boolean::source_op(map, id).is_some() {
            continue;
        }
        let Ok(solid) = b.solid() else { continue };
        for (index, (face, winding)) in b.faces.iter().zip(&solid.faces).enumerate() {
            let Some(w) = winding.as_ref().filter(|_| b.draws(face) && !face.material.is_empty()) else { continue };
            let entry = lm.face(id, index);
            let slice = entry.map(|e| e.slice);
            let part = parts.entry((face.material.clone(), slice)).or_insert_with(|| BakePart {
                material: face.material.clone(),
                lightmap: slice.map(|s| s as usize),
                ..BakePart::default()
            });
            let texture = size(&face.material);
            let first = part.positions.len() as u32;
            for p in &w.0 {
                part.positions.push(p.as_vec3().to_array());
                part.uvs.push(face.uv(*p, texture).as_vec2().to_array());
                part.lightmap_uvs.push(entry.map_or([0.0; 2], |e| e.uv_at(p.to_array())));
            }
            for k in 1..w.0.len().saturating_sub(1) as u32 {
                part.indices.extend([first, first + k, first + k + 1]);
            }
        }
    }
    let lightmaps = mapcraft_light::irradiance(lm, map)
        .into_iter()
        .map(|texels| Picture { width: lm.size, height: lm.size, rgba: texels.iter().flat_map(|t| t.to_le_bytes()).collect() })
        .collect();
    Bake { parts: parts.into_values().collect(), lightmaps, linear: true }
}
