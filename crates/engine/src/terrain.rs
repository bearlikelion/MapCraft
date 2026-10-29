use std::collections::{BTreeMap, BTreeSet};

use mapcraft_geom::DVec3;
use mapcraft_geom::noise::seeded_noise;
use mapcraft_map::{BlendRole, Brush, Displacement, FaceId, NodeId, NodeKind, blend_layer, is_tool_material};

use crate::noise::{HeightNoise, NOISE_PIXELS};
use crate::{EngineError, Session};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SculptMode {
    #[default]
    Raise,
    Lower,
    Smooth,
    Flatten,
    Noise,
    /// Paints the current material's share of the blend.
    Paint,
    /// Softens blend edges.
    Blur,
}

/// How the brush's effect fades from its center to its radius.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Falloff {
    #[default]
    Smooth,
    Linear,
    Sharp,
    Constant,
}

impl Falloff {
    /// 1 at the center easing to 0 at radius `r`.
    pub fn weight(self, d: f64, r: f64) -> f64 {
        let x = d / r;
        if r <= 0.0 || x.is_nan() || x >= 1.0 {
            return 0.0;
        }
        match self {
            Self::Smooth => (1.0 - x * x).powi(2),
            Self::Linear => 1.0 - x,
            Self::Sharp => (1.0 - x).powi(2),
            Self::Constant => 1.0,
        }
    }
}

/// One dab of the sculpt brush.
#[derive(Clone, Copy, Debug, Default)]
pub struct Sculpt {
    pub mode: SculptMode,
    pub center: DVec3,
    pub radius: f64,
    /// Height units at the center for raise, lower and noise; 0 to 1 toward the target for the other modes.
    pub amount: f64,
    /// The point flatten levels toward.
    pub level: DVec3,
    pub falloff: Falloff,
    /// Paint: the most one stroke moves a weight, 0 to 1.
    pub opacity: f64,
    /// Paint toward the other layer; raise and lower are still swapped by the caller.
    pub invert: bool,
}

/// A fill of the targeted displacements' blend weights.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BlendFill {
    Value(f64),
    Invert,
    /// Degrees from flat, measured on each vertex's normal against +Z.
    Slope {
        from: f64,
        to: f64,
    },
    /// World Z of each displaced vertex.
    Height {
        from: f64,
        to: f64,
    },
    Noise {
        size: f64,
        seed: u32,
        from: f64,
        to: f64,
    },
}

/// A displaced face being edited: its brush, face index and id, material, flat grid, normal and displacement.
struct Target {
    brush: NodeId,
    face: usize,
    face_id: FaceId,
    material: String,
    base: Vec<DVec3>,
    normal: DVec3,
    disp: Displacement,
}

impl Target {
    fn layer(&self) -> Option<&str> {
        blend_layer(&self.material, self.disp.blend_material.as_deref())
    }

    /// The displaced grid, row by row.
    fn points(&self) -> Vec<DVec3> {
        self.base.iter().enumerate().map(|(k, p)| *p + self.normal * self.disp.heights.get(k).copied().unwrap_or(0.0)).collect()
    }

    /// Blend weights from `fill`, one per vertex.
    fn filled(&self, fill: BlendFill) -> Vec<f64> {
        let points = self.points();
        let normals = self.disp.normals(&points, self.normal);
        let weight = |k: usize, p: &DVec3, n: &DVec3| match fill {
            BlendFill::Value(v) => v,
            BlendFill::Invert => 1.0 - self.disp.blend_at(k),
            BlendFill::Slope { from, to } => ramp(from, to, n.z.clamp(-1.0, 1.0).acos().to_degrees()),
            BlendFill::Height { from, to } => ramp(from, to, p.z),
            BlendFill::Noise { size, seed, from, to } => ramp(from, to, 0.65 * seeded_noise(*p / size, seed) + 0.35 * seeded_noise(*p / (size * 0.37), seed)),
        };
        points.iter().zip(&normals).enumerate().map(|(k, (p, n))| weight(k, p, n)).collect()
    }
}

const ANOTHER_LAYER: &str = "those displacements blend another material; set their Blend material first";
const OWN_MATERIAL: &str = "pick a second material to paint; a face's own material paints back only where a blend exists";
const NOT_A_LAYER: &str = "that material cannot be a blend layer; pick a plain texture";
const OWN_BLEND: &str = "those faces blend by their own material; Alt+click one to paint it";

/// What a paint dab does to one face.
enum Goal {
    Weight(f64),
    /// Weight 1 once the current material becomes the face's second layer.
    Assign,
    Skip(&'static str),
}

/// Repeatable noise in -0.5..0.5 for a point, so faces sharing an edge agree.
fn noise(p: DVec3) -> f64 {
    let q = (p * 8.0).round() / 8.0;
    (q.dot(DVec3::new(12.9898, 78.233, 37.719)).sin() * 43_758.545_3).fract().abs() - 0.5
}

/// Average of a grid value's up to four neighbors.
fn neighbor_average(values: &[f64], n: usize, k: usize) -> f64 {
    let (i, j) = (k % n, k / n);
    let around = [(i > 0).then(|| k - 1), (i + 1 < n).then(|| k + 1), (j > 0).then(|| k - n), (j + 1 < n).then(|| k + n)];
    let vals: Vec<f64> = around.into_iter().flatten().filter_map(|q| values.get(q).copied()).collect();
    vals.iter().sum::<f64>() / vals.len().max(1) as f64
}

/// Smoothstep from `a` to `b`, a hard step at `a` when they meet, falling instead when `a > b`.
fn ramp(a: f64, b: f64, x: f64) -> f64 {
    if (b - a).abs() < 1e-9 {
        return if x >= a { 1.0 } else { 0.0 };
    }
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Session {
    /// Whether some picked face is displaced.
    pub fn has_picked_displacement(&self) -> bool {
        self.face_selection
            .iter()
            .any(|(id, face)| self.map.get(*id).and_then(|n| n.kind.brush()).is_some_and(|b| b.faces.iter().any(|f| f.id == *face && f.disp.is_some())))
    }

    /// How `material` takes part in blending: tool materials never do, then the project's roles, else as a layer.
    pub fn blend_role(&self, material: &str) -> BlendRole {
        if is_tool_material(material) {
            return BlendRole::Unblendable;
        }
        let prefix = |k: &str| k.strip_suffix('*').is_some_and(|p| material.get(..p.len()).is_some_and(|h| h.eq_ignore_ascii_case(p)));
        self.blend_roles.get(material).or_else(|| self.blend_roles.iter().find(|(k, _)| prefix(k)).map(|(_, r)| r)).copied().unwrap_or_default()
    }

    /// The displaced faces sculpting and heightmaps change: the picked ones, else every visible one.
    fn displacement_targets(&self) -> Vec<Target> {
        // Picks with no displacement among them would leave nothing to sculpt, so they limit nothing.
        let picked: BTreeSet<_> = if self.has_picked_displacement() { self.face_selection.iter().copied().collect() } else { BTreeSet::new() };
        let editable = self.map.editable_nodes();
        let mut out = Vec::new();
        for (id, node) in self.map.nodes() {
            let Some(b) = node.kind.brush() else { continue };
            if !editable.contains(&id) {
                continue;
            }
            let Ok(solid) = b.solid() else { continue };
            for (i, (f, w)) in b.faces.iter().zip(&solid.faces).enumerate() {
                if !picked.is_empty() && !picked.contains(&(id, f.id)) {
                    continue;
                }
                let (Some(disp), Some(w), Some(plane)) = (&f.disp, w, f.plane()) else { continue };
                let Some(base) = disp.base(&w.0) else { continue };
                out.push(Target { brush: id, face: i, face_id: f.id, material: f.material.clone(), base, normal: plane.normal, disp: disp.clone() });
            }
        }
        out
    }

    /// Writes edited displacements back as one undo step.
    fn store_targets(&mut self, targets: Vec<Target>, label: &str, coalesce: Option<&str>) -> Result<(), EngineError> {
        let mut brushes: BTreeMap<NodeId, Brush> = BTreeMap::new();
        for t in targets {
            if !brushes.contains_key(&t.brush)
                && let Some(b) = self.map.get(t.brush).and_then(|n| n.kind.brush())
            {
                brushes.insert(t.brush, b.clone());
            }
            if let Some(f) = brushes.get_mut(&t.brush).and_then(|b| b.faces.get_mut(t.face)) {
                f.disp = Some(t.disp);
            }
        }
        let mut e = self.map.edit();
        for (id, b) in brushes {
            e.set_kind(id, NodeKind::Brush(b))?;
        }
        let c = e.finish();
        self.apply_coalesced(label, c, coalesce);
        Ok(())
    }

    /// Gives the targets `weights` and stores those that differ from `original` as one undo step; returns how many.
    fn store_blend(
        &mut self,
        targets: Vec<Target>,
        original: &[Displacement],
        weights: Vec<Vec<f64>>,
        label: &str,
        coalesce: Option<&str>,
    ) -> Result<usize, EngineError> {
        let changed: Vec<Target> = targets
            .into_iter()
            .zip(weights)
            .zip(original)
            .filter_map(|((mut t, w), o)| {
                t.disp.set_blend(w);
                (t.disp != *o).then_some(t)
            })
            .collect();
        let n = changed.len();
        if n > 0 {
            self.store_targets(changed, label, coalesce)?;
        }
        Ok(n)
    }

    /// Turns the picked four-cornered faces into displacements of `power`, or changes the power of
    /// those that already are; returns how many faces changed.
    pub fn displace_faces(&mut self, power: u32) -> Result<usize, EngineError> {
        if self.face_selection.is_empty() {
            return Err(EngineError::NothingSelected);
        }
        let mut e = self.map.edit();
        let (mut changed, mut same) = (0, 0);
        let brushes: BTreeSet<NodeId> = self.face_selection.iter().map(|(id, _)| *id).collect();
        for id in brushes {
            let Some(mut b) = self.map.get(id).and_then(|n| n.kind.brush()).cloned() else { continue };
            let Ok(solid) = b.solid() else { continue };
            for (f, w) in b.faces.iter_mut().zip(&solid.faces) {
                let Some(w) = w.as_ref().filter(|w| w.0.len() == 4 && self.face_selection.contains(&(id, f.id))) else { continue };
                f.disp = Some(match &f.disp {
                    Some(d) if d.power == power => {
                        same += 1;
                        continue;
                    }
                    Some(d) => d.resampled(power),
                    None => Displacement::new(power, w.0[0]),
                });
                changed += 1;
            }
            e.set_kind(id, NodeKind::Brush(b))?;
        }
        if changed == 0 && same > 0 {
            return Err(EngineError::Refused("the picked faces already have that power"));
        }
        if changed == 0 {
            return Err(EngineError::Refused("only faces with four corners can be displaced"));
        }
        let c = e.finish();
        self.apply("Make displacement", c);
        Ok(changed)
    }

    /// Flattens the picked faces back into plain faces.
    pub fn remove_displacements(&mut self) -> Result<(), EngineError> {
        if self.face_selection.is_empty() {
            return Err(EngineError::NothingSelected);
        }
        let mut e = self.map.edit();
        let brushes: BTreeSet<NodeId> = self.face_selection.iter().map(|(id, _)| *id).collect();
        for id in brushes {
            let Some(mut b) = self.map.get(id).and_then(|n| n.kind.brush()).cloned() else { continue };
            for f in b.faces.iter_mut().filter(|f| self.face_selection.contains(&(id, f.id))) {
                f.disp = None;
            }
            e.set_kind(id, NodeKind::Brush(b))?;
        }
        let c = e.finish();
        self.apply("Remove displacement", c);
        Ok(())
    }

    /// Applies one dab of the sculpt brush; returns whether any vertex or weight changed.
    pub fn sculpt(&mut self, dab: &Sculpt, coalesce: Option<&str>) -> Result<bool, EngineError> {
        if matches!(dab.mode, SculptMode::Paint | SculptMode::Blur) {
            return self.paint_blend(dab, &[dab.center], coalesce);
        }
        let mut targets = self.displacement_targets();
        let original: Vec<Vec<f64>> = targets.iter().map(|t| t.disp.heights.clone()).collect();
        let mut touched = false;
        for t in &mut targets {
            let n = t.disp.size();
            let before = t.disp.heights.clone();
            for k in 0..n * n {
                let in_plane = (t.base[k] - dab.center) - t.normal * (t.base[k] - dab.center).dot(t.normal);
                let w = dab.falloff.weight(in_plane.length(), dab.radius);
                if w <= 0.0 {
                    continue;
                }
                let h = before[k];
                t.disp.heights[k] = match dab.mode {
                    SculptMode::Raise => h + dab.amount * w,
                    SculptMode::Lower => h - dab.amount * w,
                    SculptMode::Noise => h + dab.amount * w * noise(t.base[k]) * 2.0,
                    SculptMode::Smooth => h + (neighbor_average(&before, n, k) - h) * (dab.amount * w).min(1.0),
                    SculptMode::Flatten => h + ((dab.level - t.base[k]).dot(t.normal) - h) * (dab.amount * w).min(1.0),
                    SculptMode::Paint | SculptMode::Blur => h,
                };
                touched |= t.disp.heights[k] != h;
            }
        }
        if !touched {
            return Ok(false);
        }
        sew_heights(&mut targets);
        let changed = targets.into_iter().zip(original).filter(|(t, h)| t.disp.heights != *h).map(|(t, _)| t).collect();
        self.store_targets(changed, "Sculpt", coalesce)?;
        Ok(true)
    }

    /// What painting the current material does to a face.
    fn paint_goal(&self, t: &Target, current: &str, invert: bool) -> Goal {
        let same = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
        let goal = match t.layer() {
            Some(b) if same(current, b) => 1.0,
            Some(_) if same(current, &t.material) => 0.0,
            Some(_) => return Goal::Skip(ANOTHER_LAYER),
            None if current.is_empty() => 1.0,
            None => match (self.blend_role(&t.material), self.blend_role(current)) {
                (BlendRole::Own, _) if same(current, &t.material) => 1.0,
                (BlendRole::Own, _) => return Goal::Skip(OWN_BLEND),
                (BlendRole::Layer, _) if same(current, &t.material) || invert => return Goal::Skip(OWN_MATERIAL),
                (BlendRole::Layer, BlendRole::Layer) => return Goal::Assign,
                _ => return Goal::Skip(NOT_A_LAYER),
            },
        };
        Goal::Weight(if invert { 1.0 - goal } else { goal })
    }

    /// Paints or blurs weights with a dab at each center as one step; never moves a vertex.
    pub fn paint_blend(&mut self, dab: &Sculpt, centers: &[DVec3], coalesce: Option<&str>) -> Result<bool, EngineError> {
        let blur = dab.mode == SculptMode::Blur;
        let current = self.tex.material.clone();
        if !blur && !current.is_empty() && self.blend_role(&current) == BlendRole::Unblendable {
            return Err(EngineError::Refused(NOT_A_LAYER));
        }
        let mut targets = self.displacement_targets();
        let original: Vec<Displacement> = targets.iter().map(|t| t.disp.clone()).collect();
        let before: Vec<Vec<f64>> = original.iter().map(Displacement::blend_values).collect();
        let points: Vec<Vec<DVec3>> = targets.iter().map(Target::points).collect();
        let mut weights = before.clone();
        let mut assigned = vec![false; targets.len()];
        let mut stroke = match (coalesce, self.paint_stroke.take()) {
            (Some(key), Some((k, s))) if k == key => s,
            _ => BTreeMap::new(),
        };
        let mut skipped = None;
        for center in centers {
            for (ti, t) in targets.iter_mut().enumerate() {
                let (Some(pts), Some(w)) = (points.get(ti), weights.get_mut(ti)) else { continue };
                let fall: Vec<f64> = pts.iter().map(|p| dab.falloff.weight(p.distance(*center), dab.radius)).collect();
                if fall.iter().all(|f| *f <= 0.0) {
                    continue;
                }
                if blur {
                    let (n, old) = (t.disp.size(), w.clone());
                    for (k, (w, f)) in w.iter_mut().zip(&fall).enumerate() {
                        *w += (neighbor_average(&old, n, k) - *w) * (dab.amount * f).min(1.0);
                    }
                    continue;
                }
                let goal = match self.paint_goal(t, &current, dab.invert) {
                    Goal::Skip(why) => {
                        skipped.get_or_insert(why);
                        continue;
                    }
                    Goal::Weight(g) => g,
                    Goal::Assign => {
                        if dab.amount * dab.opacity > 0.0 {
                            t.disp.blend_material = Some(current.clone());
                            *w = vec![0.0; w.len()];
                            stroke.remove(&(t.brush, t.face_id));
                            if let Some(a) = assigned.get_mut(ti) {
                                *a = true;
                            }
                        }
                        1.0
                    }
                };
                if coalesce.is_none() {
                    for (w, f) in w.iter_mut().zip(&fall) {
                        *w += (goal - *w) * dab.opacity * dab.amount * f;
                    }
                    continue;
                }
                let (w0, cover) = stroke.entry((t.brush, t.face_id)).or_insert_with(|| (w.clone(), vec![0.0; w.len()]));
                // The power changed mid-stroke, so the stroke starts over from here.
                if w0.len() != w.len() || cover.len() != w.len() {
                    (*w0, *cover) = (w.clone(), vec![0.0; w.len()]);
                }
                for (((w, w0), c), f) in w.iter_mut().zip(w0.iter()).zip(cover.iter_mut()).zip(&fall) {
                    if *f > 0.0 {
                        *c += (1.0 - *c) * dab.amount * f;
                        *w = w0 + (goal - w0) * dab.opacity * *c;
                    }
                }
            }
        }
        let touched = weights != before || assigned.contains(&true);
        let result = if touched {
            sew_blend(&targets, &mut weights);
            self.store_blend(targets, &original, weights, if blur { "Blur blend" } else { "Paint blend" }, coalesce).map(|_| true)
        } else {
            skipped.map_or(Ok(false), |why| Err(EngineError::Refused(why)))
        };
        self.paint_stroke = coalesce.map(|k| (k.to_string(), stroke));
        result
    }

    /// Writes the targeted displacements' blend weights from `fill` as one undo step; returns how many faces changed.
    pub fn fill_blend(&mut self, fill: BlendFill) -> Result<usize, EngineError> {
        self.blend(None, Some(fill))
    }

    /// Sets the targets' second layer, or removes it with `None`, keeping their weights; returns how many faces changed.
    pub fn set_blend_material(&mut self, material: Option<&str>) -> Result<usize, EngineError> {
        self.blend(Some(material), None)
    }

    /// Sets the targets' second layer (`Some(None)` removes it), then their weights from `fill`, as one undo step.
    pub fn blend(&mut self, material: Option<Option<&str>>, fill: Option<BlendFill>) -> Result<usize, EngineError> {
        if matches!(fill, Some(BlendFill::Noise { size, .. }) if size.is_nan() || size < 1.0) {
            return Err(EngineError::Refused("the noise size must be at least 1 unit"));
        }
        if material.flatten().is_some_and(|m| self.blend_role(m) != BlendRole::Layer) {
            return Err(EngineError::Refused(NOT_A_LAYER));
        }
        let mut targets = self.displacement_targets();
        if targets.is_empty() {
            return Err(EngineError::Refused("pick displaced faces"));
        }
        let original: Vec<Displacement> = targets.iter().map(|t| t.disp.clone()).collect();
        if let Some(material) = material {
            let mut skipped = 0;
            for t in &mut targets {
                match material {
                    Some(m) if blend_layer(&t.material, Some(m)).is_none() => skipped += 1,
                    m => t.disp.blend_material = m.map(String::from),
                }
            }
            if skipped == targets.len() {
                return Err(EngineError::Refused("pick displaced faces whose own material is not that one"));
            }
        }
        let mut weights: Vec<Vec<f64>> = targets.iter().map(|t| fill.map_or_else(|| t.disp.blend_values(), |f| t.filled(f))).collect();
        if fill.is_some() {
            sew_blend(&targets, &mut weights);
        }
        let label = match (fill, material) {
            (Some(BlendFill::Value(v)), _) if v <= 0.0 => "Clear blend",
            (Some(BlendFill::Invert), _) => "Invert blend",
            (Some(_), _) => "Fill blend",
            (None, Some(Some(_))) => "Set blend material",
            (None, _) => "Remove blend material",
        };
        self.store_blend(targets, &original, weights, label, None)
    }

    /// Sets the targeted displacements from a grayscale image stretched over all of them together,
    /// 0 at `low` and 1 at `high`; the image's top is the faces' far side (+Y on a floor).
    pub fn apply_heightmap(&mut self, gray: &[f32], width: usize, height: usize, low: f64, high: f64) -> Result<(), EngineError> {
        let targets = self.displacement_targets();
        let samples = image_samples(&targets, gray, width, height)?;
        self.store_heightmap(targets, samples, false, low, high, None)
    }

    /// Sets the targeted displacements from `noise` laid over all of them together, its lowest point at `low` and highest at `high`.
    pub fn apply_noise(&mut self, noise: &HeightNoise, low: f64, high: f64) -> Result<(), EngineError> {
        let targets = self.displacement_targets();
        let samples = noise_samples(&targets, noise)?;
        self.store_heightmap(targets, samples, true, low, high, None)
    }

    /// Applies the last heightmap or noise again between `low` and `high`, regenerating noise from `noise`;
    /// false once another edit, an undo or a new pick has replaced it.
    pub fn retune_heightmap(&mut self, noise: &HeightNoise, low: f64, high: f64, coalesce: Option<&str>) -> Result<bool, EngineError> {
        let targets = self.displacement_targets();
        let Some((_, samples, from_noise)) = self.heightmap.take_if(|(faces, ..)| targets.iter().map(|t| (t.brush, t.face_id)).eq(faces.iter().copied()))
        else {
            return Ok(false);
        };
        let samples = if from_noise { noise_samples(&targets, noise)? } else { samples };
        self.store_heightmap(targets, samples, from_noise, low, high, coalesce)?;
        Ok(true)
    }

    /// Gives the targets `low` to `high` by their 0..1 samples as one undo step and remembers them for [`Session::retune_heightmap`].
    fn store_heightmap(
        &mut self,
        targets: Vec<Target>,
        samples: Vec<Vec<f64>>,
        from_noise: bool,
        low: f64,
        high: f64,
        coalesce: Option<&str>,
    ) -> Result<(), EngineError> {
        let faces = targets.iter().map(|t| (t.brush, t.face_id)).collect();
        let mut changed = Vec::new();
        for (mut t, g) in targets.into_iter().zip(&samples) {
            let before = t.disp.heights.clone();
            for (h, g) in t.disp.heights.iter_mut().zip(g) {
                *h = low + g * (high - low);
            }
            if t.disp.heights != before {
                changed.push(t);
            }
        }
        if !changed.is_empty() {
            self.store_targets(changed, if from_noise { "Apply noise" } else { "Apply heightmap" }, coalesce)?;
        }
        self.heightmap = Some((faces, samples, from_noise));
        Ok(())
    }

    /// Sets the targeted displacements' blend weights from a grayscale image, placed as [`Session::apply_heightmap`] places it.
    pub fn apply_blend_mask(&mut self, gray: &[f32], width: usize, height: usize) -> Result<(), EngineError> {
        let targets = self.displacement_targets();
        let mut weights = image_samples(&targets, gray, width, height)?;
        let original: Vec<Displacement> = targets.iter().map(|t| t.disp.clone()).collect();
        sew_blend(&targets, &mut weights);
        self.store_blend(targets, &original, weights, "Apply blend mask", None)?;
        Ok(())
    }
}

/// The two in-plane axes of the targets' first face, and the low corner and size of their flat grids' bounds.
fn layout(targets: &[Target]) -> Option<(usize, usize, DVec3, DVec3)> {
    let axis = targets.first()?.normal.abs().max_position();
    let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
    let (a, b) = if a < b { (a, b) } else { (b, a) };
    let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
    for p in targets.iter().flat_map(|t| t.base.iter()) {
        (lo, hi) = (lo.min(*p), hi.max(*p));
    }
    Some((a, b, lo, (hi - lo).max(DVec3::splat(1e-9))))
}

/// Each target's flat grid sampled from a gray image stretched over them all, its top on their far side.
fn image_samples(targets: &[Target], gray: &[f32], width: usize, height: usize) -> Result<Vec<Vec<f64>>, EngineError> {
    let Some((a, b, lo, span)) = layout(targets).filter(|_| width >= 2 && height >= 2 && gray.len() >= width * height) else {
        return Err(EngineError::Refused("pick displaced faces and a heightmap image"));
    };
    let at = |x: usize, y: usize| f64::from(gray.get(y.min(height - 1) * width + x.min(width - 1)).copied().unwrap_or(0.0));
    let sample = |p: &DVec3| {
        let u = (p[a] - lo[a]) / span[a] * (width - 1) as f64;
        let v = (1.0 - (p[b] - lo[b]) / span[b]) * (height - 1) as f64;
        let (x, y) = (u.floor().max(0.0) as usize, v.floor().max(0.0) as usize);
        let (fx, fy) = (u - x as f64, v - y as f64);
        (at(x, y) * (1.0 - fx) + at(x + 1, y) * fx) * (1.0 - fy) + (at(x, y + 1) * (1.0 - fx) + at(x + 1, y + 1) * fx) * fy
    };
    Ok(targets.iter().map(|t| t.base.iter().map(sample).collect()).collect())
}

/// Each target's flat grid sampled from `noise` laid over them all with its top on their far side, scaled to 0..1 as
/// NoiseTexture2D normalizes, then terraced and eased down at the edges.
fn noise_samples(targets: &[Target], noise: &HeightNoise) -> Result<Vec<Vec<f64>>, EngineError> {
    let (a, b, lo, span) = layout(targets).ok_or(EngineError::Refused("pick displaced faces"))?;
    let at = noise.sampler();
    let px = NOISE_PIXELS / span[a].max(span[b]);
    let raw: Vec<Vec<f64>> = targets.iter().map(|t| t.base.iter().map(|p| at((p[a] - lo[a]) * px, (lo[b] + span[b] - p[b]) * px)).collect()).collect();
    let (min, max) = raw.iter().flatten().fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), v| (l.min(*v), h.max(*v)));
    let steps = f64::from(noise.terraces);
    let falloff = if noise.edge_falloff.is_finite() { noise.edge_falloff.clamp(0.0, 0.5) } else { 0.0 };
    let edge = |x: f64| ramp(0.0, falloff, x.min(1.0 - x));
    let shape = |p: &DVec3, v: f64| {
        let mut g = if max > min { (v - min) / (max - min) } else { 0.0 };
        if noise.terraces >= 2 {
            g = ((g * steps).floor() + ramp(0.5, 1.0, (g * steps).fract())) / steps;
        }
        g * edge((p[a] - lo[a]) / span[a]) * edge((p[b] - lo[b]) / span[b])
    };
    Ok(targets.iter().zip(raw).map(|(t, r)| t.base.iter().zip(r).map(|(p, v)| shape(p, v)).collect()).collect())
}

/// (target, vertex) lists of the vertices neighboring faces share.
fn shared_points(targets: &[Target]) -> Vec<Vec<(usize, usize)>> {
    let key = |p: DVec3, n: DVec3| ((p * 100.0).round().as_i64vec3().to_array(), (n * 1000.0).round().as_i64vec3().to_array());
    let mut shared: BTreeMap<_, Vec<(usize, usize)>> = BTreeMap::new();
    for (ti, t) in targets.iter().enumerate() {
        for (k, p) in t.base.iter().enumerate() {
            shared.entry(key(*p, t.normal)).or_default().push((ti, k));
        }
    }
    shared.into_values().filter(|l| l.len() > 1).collect()
}

/// Gives vertices that neighboring faces share one height, so their edges stay joined.
fn sew_heights(targets: &mut [Target]) {
    for list in shared_points(targets) {
        let sum: f64 = list.iter().filter_map(|(ti, k)| targets.get(*ti)?.disp.heights.get(*k)).sum();
        let avg = sum / list.len() as f64;
        for (ti, k) in list {
            if let Some(h) = targets.get_mut(ti).and_then(|t| t.disp.heights.get_mut(k)) {
                *h = avg;
            }
        }
    }
}

/// Averages shared vertices' weights among faces with the same material and second layer.
fn sew_blend(targets: &[Target], weights: &mut [Vec<f64>]) {
    for list in shared_points(targets) {
        let mut groups: BTreeMap<(String, String), Vec<(usize, usize)>> = BTreeMap::new();
        for (ti, k) in list {
            let Some(t) = targets.get(ti) else { continue };
            groups.entry((t.material.to_ascii_lowercase(), t.layer().unwrap_or_default().to_ascii_lowercase())).or_default().push((ti, k));
        }
        for group in groups.values().filter(|g| g.len() > 1) {
            let avg = group.iter().filter_map(|(ti, k)| weights.get(*ti)?.get(*k)).sum::<f64>() / group.len() as f64;
            for (ti, k) in group {
                if let Some(w) = weights.get_mut(*ti).and_then(|w| w.get_mut(*k)) {
                    *w = avg;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use mapcraft_geom::Aabb;

    use super::*;
    use crate::{FractalType, NoiseType};

    fn floor(s: &mut Session, x: f64) -> NodeId {
        s.create_brush(Aabb::new(DVec3::new(x, 0.0, -16.0), DVec3::new(x + 64.0, 64.0, 0.0))).unwrap()
    }

    fn grid(s: &Session, id: NodeId) -> Vec<DVec3> {
        let br = s.map.get(id).unwrap().kind.brush().unwrap();
        br.displaced(&br.solid().unwrap()).into_iter().next().unwrap().1
    }

    fn top(s: &Session, id: NodeId) -> mapcraft_map::FaceId {
        let b = s.map.get(id).unwrap().kind.brush().unwrap();
        b.faces.iter().find(|f| f.plane().unwrap().normal.z > 0.9).unwrap().id
    }

    #[test]
    fn sculpted_neighbors_stay_joined() {
        let mut s = Session::default();
        let (a, b) = (floor(&mut s, 0.0), floor(&mut s, 64.0));
        s.face_selection = BTreeSet::from([(a, top(&s, a)), (b, top(&s, b))]);
        assert_eq!(s.displace_faces(3).unwrap(), 2);
        let dab = Sculpt { mode: SculptMode::Raise, center: DVec3::new(64.0, 32.0, 0.0), radius: 40.0, amount: 10.0, level: DVec3::ZERO, ..Default::default() };
        assert!(s.sculpt(&dab, Some("stroke")).unwrap());
        let smooth = Sculpt { mode: SculptMode::Smooth, amount: 1.0, ..dab };
        s.sculpt(&smooth, Some("stroke")).unwrap();
        let (ga, gb) = (grid(&s, a), grid(&s, b));
        let seam = |g: &[DVec3]| {
            g.iter()
                .filter(|p| (p.x - 64.0).abs() < 1e-6)
                .map(|p| (p.y * 100.0).round() as i64)
                .zip(g.iter().filter(|p| (p.x - 64.0).abs() < 1e-6).map(|p| p.z))
                .collect::<BTreeMap<_, _>>()
        };
        assert_eq!(seam(&ga), seam(&gb), "the shared edge has one height on both faces");
        assert!(ga.iter().any(|p| p.z > 1.0), "the brush raised the surface");
        s.undo();
        assert!(grid(&s, a).iter().all(|p| p.z.abs() < 1e-9), "one undo takes back the whole stroke");
    }

    #[test]
    fn plain_picks_do_not_block_sculpting() {
        let mut s = Session::default();
        let a = floor(&mut s, 0.0);
        s.face_selection = BTreeSet::from([(a, top(&s, a))]);
        s.displace_faces(2).unwrap();
        let side = s.map.get(a).unwrap().kind.brush().unwrap().faces.iter().find(|f| f.disp.is_none()).unwrap().id;
        s.face_selection = BTreeSet::from([(a, side)]);
        let dab = Sculpt { mode: SculptMode::Raise, center: DVec3::new(32.0, 32.0, 0.0), radius: 64.0, amount: 8.0, level: DVec3::ZERO, ..Default::default() };
        assert!(s.sculpt(&dab, None).unwrap(), "a pick with no displacement in it limits nothing");
    }

    #[test]
    fn heightmaps_span_the_picked_faces() {
        let mut s = Session::default();
        let (a, b) = (floor(&mut s, 0.0), floor(&mut s, 64.0));
        s.face_selection = BTreeSet::from([(a, top(&s, a)), (b, top(&s, b))]);
        s.displace_faces(2).unwrap();
        // A ramp from black at the left edge to white at the right.
        let gray: Vec<f32> = (0..4).flat_map(|_| (0..5).map(|x| x as f32 / 4.0)).collect();
        s.apply_heightmap(&gray, 5, 4, 0.0, 100.0).unwrap();
        let right = grid(&s, b).iter().filter(|p| (p.x - 128.0).abs() < 1e-6).map(|p| p.z).fold(0.0, f64::max);
        assert!((right - 100.0).abs() < 1e-6, "the far edge is white: {right}");
        assert!(s.displace_faces(2).is_err(), "nothing changes when the power is the same");
    }

    #[test]
    fn heightmap_ranges_retune_live_in_one_step() {
        let (mut s, ids) = terrain(2);
        let gray: Vec<f32> = (0..4).flat_map(|_| (0..5).map(|x| x as f32 / 4.0)).collect();
        let noise = HeightNoise::default();
        assert!(!s.retune_heightmap(&noise, 0.0, 10.0, Some("heightmap")).unwrap(), "nothing applied yet");
        s.apply_heightmap(&gray, 5, 4, 0.0, 100.0).unwrap();
        let peak = |s: &Session| grid(s, ids[1]).iter().map(|p| p.z).fold(f64::MIN, f64::max);
        for high in [120.0, 150.0, 200.0] {
            assert!(s.retune_heightmap(&noise, 0.0, high, Some("heightmap")).unwrap());
        }
        assert!((peak(&s) - 200.0).abs() < 1e-6, "white follows the new height");
        s.undo();
        assert!((peak(&s) - 100.0).abs() < 1e-6, "one undo takes back the whole drag");
        assert!(!s.retune_heightmap(&noise, 0.0, 50.0, None).unwrap(), "an undo ends it");
        s.apply_heightmap(&gray, 5, 4, 0.0, 100.0).unwrap();
        let picked = s.face_selection.clone();
        s.face_selection = BTreeSet::from([(ids[0], top(&s, ids[0]))]);
        assert!(!s.retune_heightmap(&noise, 0.0, 50.0, None).unwrap(), "another pick ends it");
        s.face_selection = picked;
        let dab = Sculpt { mode: SculptMode::Raise, center: DVec3::new(32.0, 32.0, 0.0), radius: 16.0, amount: 4.0, ..Default::default() };
        s.sculpt(&dab, None).unwrap();
        assert!(!s.retune_heightmap(&noise, 0.0, 50.0, None).unwrap(), "another edit ends it");
    }

    #[test]
    fn noise_fills_the_range_and_survives_wild_settings() {
        let (mut s, ids) = terrain(2);
        let heights = |s: &Session| ids.iter().flat_map(|id| disp(s, *id).heights).collect::<Vec<f64>>();
        for (name, preset) in HeightNoise::presets() {
            s.apply_noise(&preset, -10.0, 50.0).unwrap();
            let (min, max) = heights(&s).iter().fold((f64::MAX, f64::MIN), |(l, h), v| (l.min(*v), h.max(*v)));
            assert!((min + 10.0).abs() < 1e-6 && (max - 50.0).abs() < 1e-6, "{name} spans the range: {min} {max}");
        }
        let hills = HeightNoise::presets()[0].1;
        s.apply_noise(&hills, 0.0, 64.0).unwrap();
        let first = heights(&s);
        assert!(s.retune_heightmap(&HeightNoise { seed: 7, ..hills }, 0.0, 64.0, Some("heightmap")).unwrap());
        assert_ne!(heights(&s), first, "a new seed reshapes it live");
        assert!(s.retune_heightmap(&hills, 0.0, 64.0, Some("heightmap")).unwrap());
        assert_eq!(heights(&s), first, "the same settings give the same surface");
        s.retune_heightmap(&HeightNoise { edge_falloff: 0.25, ..hills }, 0.0, 64.0, None).unwrap();
        let rim = ids.iter().flat_map(|id| grid(&s, *id)).filter(|p| p.x < 1e-6 || p.x > 128.0 - 1e-6 || p.y < 1e-6 || p.y > 64.0 - 1e-6);
        assert!(rim.into_iter().all(|p| p.z.abs() < 1e-9), "the edges ease down to black");
        let wild = HeightNoise {
            seed: u32::MAX,
            frequency: f64::NAN,
            offset: [f64::INFINITY, -1e300],
            octaves: u32::MAX,
            lacunarity: 1e300,
            gain: f64::NAN,
            weighted_strength: -5.0,
            edge_falloff: f64::NAN,
            terraces: u32::MAX,
            ..hills
        };
        let kinds = [NoiseType::OpenSimplex2, NoiseType::OpenSimplex2S, NoiseType::Cellular, NoiseType::Perlin, NoiseType::ValueCubic, NoiseType::Value];
        for kind in kinds {
            for fractal in [FractalType::None, FractalType::FBm, FractalType::Ridged, FractalType::PingPong] {
                for frequency in [f64::NAN, 0.0, 1e300] {
                    s.apply_noise(&HeightNoise { kind, fractal, frequency, ..wild }, 0.0, 64.0).unwrap();
                    assert!(heights(&s).iter().all(|h| (0.0..=64.0).contains(h)), "{kind:?} {fractal:?} {frequency}");
                }
            }
        }
    }

    /// A session with `n` displaced `rock` floors side by side along +X, all picked.
    fn terrain(n: usize) -> (Session, Vec<NodeId>) {
        let mut s = Session::default();
        s.tex.material = "rock".into();
        let ids: Vec<NodeId> = (0..n).map(|i| floor(&mut s, i as f64 * 64.0)).collect();
        s.face_selection = ids.iter().map(|id| (*id, top(&s, *id))).collect();
        s.displace_faces(2).unwrap();
        (s, ids)
    }

    fn disp(s: &Session, id: NodeId) -> Displacement {
        s.map.get(id).unwrap().kind.brush().unwrap().faces.iter().find_map(|f| f.disp.clone()).unwrap()
    }

    /// Changes a floor's displacement and material directly, as a file could hold them.
    fn edit(s: &mut Session, id: NodeId, change: impl Fn(&mut mapcraft_map::Face)) {
        let mut b = s.map.get(id).unwrap().kind.brush().unwrap().clone();
        b.faces.iter_mut().filter(|f| f.disp.is_some()).for_each(change);
        s.set_kind(id, NodeKind::Brush(b), "Edit", None).unwrap();
    }

    /// Weights by the displaced point they sit on.
    fn weights(s: &Session, id: NodeId) -> Vec<(DVec3, f64)> {
        grid(s, id).into_iter().zip(disp(s, id).blend_values()).collect()
    }

    fn paint(x: f64, y: f64, radius: f64) -> Sculpt {
        Sculpt { mode: SculptMode::Paint, center: DVec3::new(x, y, 0.0), radius, amount: 1.0, opacity: 1.0, ..Default::default() }
    }

    fn refusal(r: Result<bool, EngineError>) -> &'static str {
        match r {
            Err(EngineError::Refused(why)) => why,
            other => panic!("not refused: {other:?}"),
        }
    }

    #[test]
    fn painting_assigns_the_layer_and_never_moves_a_vertex() {
        let (mut s, ids) = terrain(2);
        // An unsewn seam: the second floor sits 3 units higher.
        edit(&mut s, ids[1], |f| f.disp.as_mut().unwrap().heights.iter_mut().for_each(|h| *h = 3.0));
        let heights: Vec<Vec<f64>> = ids.iter().map(|id| disp(&s, *id).heights).collect();
        s.tex.material = "grass".into();
        let dab = paint(40.0, 32.0, 32.0);
        assert!(s.paint_blend(&dab, &[dab.center], Some("stroke")).unwrap());
        assert_eq!(ids.iter().map(|id| disp(&s, *id).heights).collect::<Vec<_>>(), heights, "paint never moves a vertex");
        for id in &ids {
            assert_eq!(disp(&s, *id).blend_material.as_deref(), Some("grass"));
        }
        let w = weights(&s, ids[0]);
        assert!(w.iter().any(|(p, w)| p.distance(DVec3::new(32.0, 32.0, 0.0)) < 1e-9 && *w > 0.8), "{w:?}");
        assert!(w.iter().all(|(p, w)| p.distance(dab.center) < 32.0 || *w == 0.0), "nothing outside the ring");
        s.undo();
        assert!(ids.iter().all(|id| disp(&s, *id).blend.is_empty() && disp(&s, *id).blend_material.is_none()), "one undo takes back both");
    }

    #[test]
    fn a_new_layer_starts_from_nothing() {
        let (mut s, ids) = terrain(1);
        s.tex.material = "grass".into();
        let everywhere = Sculpt { falloff: Falloff::Constant, ..paint(32.0, 32.0, 200.0) };
        s.sculpt(&everywhere, None).unwrap();
        assert!(disp(&s, ids[0]).blend_values().iter().all(|w| *w == 1.0));
        assert_eq!(s.set_blend_material(None).unwrap(), 1);
        assert_eq!(disp(&s, ids[0]).blend.len(), 25, "removing the layer keeps the weights");
        s.tex.material = "sand".into();
        s.sculpt(&Sculpt { falloff: Falloff::Constant, ..paint(32.0, 32.0, 20.0) }, None).unwrap();
        assert_eq!(disp(&s, ids[0]).blend_material.as_deref(), Some("sand"));
        for (p, w) in weights(&s, ids[0]) {
            assert_eq!(w, if p.distance(DVec3::new(32.0, 32.0, 0.0)) < 20.0 { 1.0 } else { 0.0 }, "{p}");
        }
    }

    #[test]
    fn paint_refuses_what_cannot_blend() {
        let (mut s, ids) = terrain(1);
        let before = s.map.clone();
        let dab = Sculpt { falloff: Falloff::Constant, ..paint(32.0, 32.0, 32.0) };
        s.tex.material = "rock".into();
        assert_eq!(refusal(s.sculpt(&dab, None)), OWN_MATERIAL);
        s.tex.material = "grass".into();
        assert_eq!(refusal(s.sculpt(&Sculpt { invert: true, ..dab }, None)), OWN_MATERIAL, "Ctrl never adds a layer");
        s.blend_roles.insert("sky*".into(), BlendRole::Unblendable);
        for current in ["SKY_day", "tools/trigger"] {
            s.tex.material = current.into();
            assert_eq!(refusal(s.sculpt(&dab, None)), NOT_A_LAYER, "{current}");
        }
        assert_eq!(s.map, before, "refusals change nothing");

        edit(&mut s, ids[0], |f| f.disp.as_mut().unwrap().blend_material = Some("grass".into()));
        s.tex.material = "sand".into();
        assert_eq!(refusal(s.sculpt(&dab, None)), ANOTHER_LAYER);
        s.tex.material = "grass".into();
        s.sculpt(&dab, None).unwrap();
        s.sculpt(&Sculpt { invert: true, ..dab }, None).unwrap();
        assert!(disp(&s, ids[0]).blend_values().iter().all(|w| *w < 1e-9), "Ctrl paints the face's own material back");

        edit(&mut s, ids[0], |f| f.disp.as_mut().unwrap().blend_material = None);
        s.blend_roles.insert("rock".into(), BlendRole::Own);
        assert_eq!(refusal(s.sculpt(&dab, None)), OWN_BLEND);
        s.tex.material = "rock".into();
        assert!(s.sculpt(&dab, None).unwrap());
        assert!(disp(&s, ids[0]).blend_material.is_none() && disp(&s, ids[0]).blend_at(12) == 1.0, "a material that blends itself takes weights only");

        let (mut s, ids) = terrain(1);
        s.tex.material.clear();
        assert!(s.sculpt(&dab, None).unwrap());
        assert!(disp(&s, ids[0]).blend_material.is_none() && disp(&s, ids[0]).blend_at(12) == 1.0, "no current material paints weights only");
    }

    #[test]
    fn strokes_stop_at_their_opacity() {
        let (mut s, ids) = terrain(1);
        s.tex.material = "grass".into();
        let dab = Sculpt { amount: 0.5, opacity: 0.4, ..paint(32.0, 32.0, 24.0) };
        for _ in 0..30 {
            s.paint_blend(&dab, &[dab.center], Some("stroke")).unwrap();
        }
        let peak = disp(&s, ids[0]).blend_values().into_iter().fold(0.0, f64::max);
        assert!((0.39..=0.4).contains(&peak), "{peak}");

        let mut b = s.map.get(ids[0]).unwrap().kind.brush().unwrap().clone();
        b.faces.iter_mut().filter_map(|f| f.disp.as_mut()).for_each(|d| *d = d.resampled(3));
        let mut e = s.map.edit();
        e.set_kind(ids[0], NodeKind::Brush(b.clone())).unwrap();
        let c = e.finish();
        s.map.apply(c);
        s.paint_blend(&dab, &[dab.center], Some("stroke")).unwrap();
        assert_eq!(disp(&s, ids[0]).blend.len(), 81, "a power change mid-stroke restarts it");

        b.faces.iter_mut().filter_map(|f| f.disp.as_mut()).for_each(|d| d.blend = vec![1.0; 81]);
        s.set_kind(ids[0], NodeKind::Brush(b), "Fill", Some("stroke")).unwrap();
        s.paint_blend(&dab, &[dab.center], Some("stroke")).unwrap();
        assert!(disp(&s, ids[0]).blend_values().iter().all(|w| *w == 1.0), "an edit under the stroke's key restarts it");
    }

    #[test]
    fn one_call_paints_every_dab_in_one_step() {
        let (mut s, ids) = terrain(1);
        s.tex.material = "grass".into();
        let dab = paint(0.0, 0.0, 8.0);
        assert!(s.paint_blend(&dab, &[DVec3::new(0.0, 0.0, 0.0), DVec3::new(64.0, 64.0, 0.0)], None).unwrap());
        let painted: Vec<DVec3> = weights(&s, ids[0]).into_iter().filter(|(_, w)| *w > 0.0).map(|(p, _)| p).collect();
        assert!(painted.len() == 2 && painted.contains(&DVec3::ZERO) && painted.contains(&DVec3::new(64.0, 64.0, 0.0)), "{painted:?}");
        s.undo();
        assert_eq!(s.undo_label(), Some("Make displacement"), "both dabs were one step");
    }

    #[test]
    fn dabs_paint_only_the_floor_under_the_cursor() {
        let mut s = Session::default();
        let low = floor(&mut s, 0.0);
        let high = s.create_brush(Aabb::new(DVec3::new(0.0, 0.0, 112.0), DVec3::new(64.0, 64.0, 128.0))).unwrap();
        s.face_selection = BTreeSet::from([(low, top(&s, low)), (high, top(&s, high))]);
        s.displace_faces(2).unwrap();
        s.tex.material = "grass".into();
        assert!(s.sculpt(&paint(32.0, 32.0, 100.0), None).unwrap());
        assert!(disp(&s, low).blend_at(12) > 0.0);
        assert!(disp(&s, high).blend.is_empty() && disp(&s, high).blend_material.is_none());
    }

    #[test]
    fn blur_softens_a_spike() {
        let (mut s, ids) = terrain(1);
        edit(&mut s, ids[0], |f| {
            let d = f.disp.as_mut().unwrap();
            d.blend = vec![0.0; 25];
            d.blend[12] = 1.0;
        });
        let blur = Sculpt { mode: SculptMode::Blur, falloff: Falloff::Constant, ..paint(32.0, 32.0, 20.0) };
        assert!(s.sculpt(&blur, None).unwrap());
        let w = disp(&s, ids[0]).blend_values();
        assert_eq!(w[12], 0.0, "the spike takes its neighbors' average");
        assert!(w[7] > 0.2 && w[0] == 0.0, "{w:?}");
        assert_eq!(s.undo_label(), Some("Blur blend"));
    }

    #[test]
    fn seams_join_only_matching_blends() {
        let (mut s, ids) = terrain(2);
        for id in &ids {
            edit(&mut s, *id, |f| f.disp.as_mut().unwrap().blend_material = Some("grass".into()));
        }
        let seam = |targets: &[Target], w: &[Vec<f64>], ti: usize| -> Vec<f64> {
            targets[ti].base.iter().zip(&w[ti]).filter(|(p, _)| (p.x - 64.0).abs() < 1e-9).map(|(_, w)| *w).collect()
        };
        let mut targets = s.displacement_targets();
        let mut w = vec![vec![1.0; 25], vec![0.0; 25]];
        sew_blend(&targets, &mut w);
        assert_eq!((seam(&targets, &w, 0), seam(&targets, &w, 1)), (vec![0.5; 5], vec![0.5; 5]));
        assert_eq!(w[0].iter().filter(|w| **w == 1.0).count(), 20, "away from the seam nothing moves");
        targets[1].material = "sand".into();
        let mut w = vec![vec![1.0; 25], vec![0.0; 25]];
        sew_blend(&targets, &mut w);
        assert_eq!((seam(&targets, &w, 0), seam(&targets, &w, 1)), (vec![1.0; 5], vec![0.0; 5]), "another material keeps a hard seam");
    }

    #[test]
    fn fills_follow_slope_height_and_noise() {
        let (mut s, ids) = terrain(1);
        // Flat for the first half, then rising 64 units over the last 32.
        let gray: Vec<f32> = [0.0, 0.0, 0.0, 0.5, 1.0].repeat(2);
        s.apply_heightmap(&gray, 5, 2, 0.0, 64.0).unwrap();
        let check = |s: &mut Session, fill: BlendFill, flat: f64, steep: f64| {
            s.fill_blend(fill).unwrap();
            for (p, w) in weights(s, ids[0]) {
                if p.x < 20.0 {
                    assert_eq!(w, flat, "{fill:?} at {p}");
                } else if p.x > 60.0 {
                    assert_eq!(w, steep, "{fill:?} at {p}");
                }
            }
        };
        check(&mut s, BlendFill::Slope { from: 30.0, to: 45.0 }, 0.0, 1.0);
        check(&mut s, BlendFill::Slope { from: 45.0, to: 30.0 }, 1.0, 0.0);
        check(&mut s, BlendFill::Height { from: 0.0, to: 64.0 }, 0.0, 1.0);
        check(&mut s, BlendFill::Height { from: 64.0, to: 0.0 }, 1.0, 0.0);
        check(&mut s, BlendFill::Height { from: 32.0, to: 32.0 }, 0.0, 1.0);
        assert_eq!(s.undo_label(), Some("Fill blend"));
        let before = disp(&s, ids[0]).blend_values();
        s.fill_blend(BlendFill::Invert).unwrap();
        assert_eq!(disp(&s, ids[0]).blend_values(), before.iter().map(|w| 1.0 - w).collect::<Vec<_>>());
        assert_eq!(s.undo_label(), Some("Invert blend"));
        s.fill_blend(BlendFill::Value(0.0)).unwrap();
        assert!(disp(&s, ids[0]).blend.is_empty());
        assert_eq!(s.undo_label(), Some("Clear blend"));

        let (mut s, ids) = terrain(2);
        edit(&mut s, ids[1], |f| f.material = "sand".into());
        let noise = BlendFill::Noise { size: 40.0, seed: 7, from: 0.3, to: 0.7 };
        s.fill_blend(noise).unwrap();
        let first: Vec<_> = ids.iter().map(|id| weights(&s, *id)).collect();
        s.fill_blend(BlendFill::Value(0.0)).unwrap();
        s.fill_blend(noise).unwrap();
        assert_eq!(ids.iter().map(|id| weights(&s, *id)).collect::<Vec<_>>(), first, "noise repeats");
        let seam = |w: &[(DVec3, f64)]| w.iter().filter(|(p, _)| (p.x - 64.0).abs() < 1e-9).map(|(_, w)| *w).collect::<Vec<_>>();
        assert_eq!(seam(&first[0]), seam(&first[1]), "faces of different materials still agree at their shared edge");
        assert!(first[0].iter().any(|(_, w)| *w > 0.0 && *w < 1.0));
        assert!(matches!(s.fill_blend(BlendFill::Noise { size: 0.0, seed: 0, from: 0.0, to: 1.0 }), Err(EngineError::Refused(_))));
    }

    #[test]
    fn blend_masks_sample_like_heightmaps() {
        let (mut s, ids) = terrain(2);
        let gray: Vec<f32> = (0..12).map(|k| (k * 7 % 12) as f32 / 11.0).collect();
        s.apply_blend_mask(&gray, 4, 3).unwrap();
        assert_eq!(s.undo_label(), Some("Apply blend mask"));
        s.apply_heightmap(&gray, 4, 3, 0.0, 1.0).unwrap();
        for id in &ids {
            let d = disp(&s, *id);
            for (w, h) in d.blend_values().iter().zip(&d.heights) {
                assert!((w - h).abs() < 1e-9, "{w} {h}");
            }
        }
        assert!(s.apply_blend_mask(&gray, 1, 12).is_err());
    }

    #[test]
    fn blend_materials_skip_their_own_faces() {
        let (mut s, ids) = terrain(2);
        edit(&mut s, ids[1], |f| f.material = "sand".into());
        assert_eq!(s.set_blend_material(Some("SAND")).unwrap(), 1);
        assert_eq!(disp(&s, ids[0]).blend_material.as_deref(), Some("SAND"));
        assert!(disp(&s, ids[1]).blend_material.is_none(), "a face's own material is no second layer");
        s.face_selection = BTreeSet::from([(ids[1], top(&s, ids[1]))]);
        assert!(matches!(s.set_blend_material(Some("sand")), Err(EngineError::Refused(_))));
        assert!(matches!(s.set_blend_material(Some("tools/clip")), Err(EngineError::Refused(NOT_A_LAYER))));
        assert_eq!(s.undo_label(), Some("Set blend material"));
    }

    #[test]
    fn blend_commands_need_a_picked_displacement() {
        let mut s = Session::default();
        let a = floor(&mut s, 0.0);
        s.tex.material = "grass".into();
        assert!(matches!(s.set_blend_material(Some("grass")), Err(EngineError::Refused("pick displaced faces"))));
        assert!(matches!(crate::execute(&mut s, "disp.blend_fill"), Err(EngineError::Disabled(_))));
        s.face_selection = BTreeSet::from([(a, top(&s, a))]);
        assert!(matches!(crate::execute(&mut s, "disp.blend_use_current"), Err(EngineError::Disabled(_))), "a plain face is not enough");
        s.displace_faces(2).unwrap();
        for id in ["disp.blend_use_current", "disp.blend_fill", "disp.blend_invert", "disp.blend_clear", "disp.blend_remove"] {
            crate::execute(&mut s, id).unwrap();
        }
        assert!(disp(&s, a).blend.is_empty() && disp(&s, a).blend_material.is_none());
    }
}
