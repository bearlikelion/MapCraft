use std::collections::{BTreeMap, BTreeSet};

use mapcraft_geom::DVec3;
use mapcraft_map::{Brush, Displacement, NodeId, NodeKind};

use crate::{EngineError, Session};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SculptMode {
    Raise,
    Lower,
    Smooth,
    Flatten,
    Noise,
}

/// One dab of the sculpt brush.
#[derive(Clone, Copy, Debug)]
pub struct Sculpt {
    pub mode: SculptMode,
    pub center: DVec3,
    pub radius: f64,
    /// Units of height at the center for raise, lower and noise; how far toward the target, from
    /// 0 to 1, for smooth and flatten.
    pub amount: f64,
    /// The point flatten levels toward.
    pub level: DVec3,
}

/// A displaced face being edited: its brush, face index, flat grid, normal and displacement.
struct Target {
    brush: NodeId,
    face: usize,
    base: Vec<DVec3>,
    normal: DVec3,
    disp: Displacement,
}

/// Falloff of the brush: 1 at the center, easing to 0 at the radius.
fn falloff(d: f64, r: f64) -> f64 {
    if d >= r { 0.0 } else { (1.0 - (d / r).powi(2)).powi(2) }
}

/// Repeatable noise in -0.5..0.5 for a point, so faces sharing an edge agree.
fn noise(p: DVec3) -> f64 {
    let q = (p * 8.0).round() / 8.0;
    (q.dot(DVec3::new(12.9898, 78.233, 37.719)).sin() * 43_758.545_3).fract().abs() - 0.5
}

impl Session {
    /// The displaced faces sculpting and heightmaps change: the picked ones, else every visible one.
    fn displacement_targets(&self) -> Vec<Target> {
        let mut picked: BTreeSet<_> = self.face_selection.iter().copied().collect();
        // Picks with no displacement among them would leave nothing to sculpt, so they limit nothing.
        let displaced = |(id, face): &(NodeId, mapcraft_map::FaceId)| {
            self.map.get(*id).and_then(|n| n.kind.brush()).is_some_and(|b| b.faces.iter().any(|f| f.id == *face && f.disp.is_some()))
        };
        if !picked.iter().any(displaced) {
            picked.clear();
        }
        let visible = self.map.visible_nodes();
        let mut out = Vec::new();
        for (id, node) in self.map.nodes() {
            let (Some(b), false) = (node.kind.brush(), node.locked) else { continue };
            if !visible.contains(&id) {
                continue;
            }
            let Ok(solid) = b.solid() else { continue };
            for (i, (f, w)) in b.faces.iter().zip(&solid.faces).enumerate() {
                if !picked.is_empty() && !picked.contains(&(id, f.id)) {
                    continue;
                }
                let (Some(disp), Some(w), Some(plane)) = (&f.disp, w, f.plane()) else { continue };
                let Some(base) = disp.base(&w.0) else { continue };
                out.push(Target { brush: id, face: i, base, normal: plane.normal, disp: disp.clone() });
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

    /// Applies one dab of the sculpt brush; returns whether any vertex moved.
    pub fn sculpt(&mut self, dab: &Sculpt, coalesce: Option<&str>) -> Result<bool, EngineError> {
        let mut targets = self.displacement_targets();
        let original: Vec<Vec<f64>> = targets.iter().map(|t| t.disp.heights.clone()).collect();
        let mut touched = false;
        for t in &mut targets {
            let n = t.disp.size();
            let before = t.disp.heights.clone();
            for k in 0..n * n {
                let in_plane = (t.base[k] - dab.center) - t.normal * (t.base[k] - dab.center).dot(t.normal);
                let w = falloff(in_plane.length(), dab.radius);
                if w <= 0.0 {
                    continue;
                }
                let h = before[k];
                let neighbors = || {
                    let (i, j) = (k % n, k / n);
                    let around = [(i > 0).then(|| k - 1), (i + 1 < n).then(|| k + 1), (j > 0).then(|| k - n), (j + 1 < n).then(|| k + n)];
                    let vals: Vec<f64> = around.into_iter().flatten().map(|q| before[q]).collect();
                    vals.iter().sum::<f64>() / vals.len().max(1) as f64
                };
                t.disp.heights[k] = match dab.mode {
                    SculptMode::Raise => h + dab.amount * w,
                    SculptMode::Lower => h - dab.amount * w,
                    SculptMode::Noise => h + dab.amount * w * noise(t.base[k]) * 2.0,
                    SculptMode::Smooth => h + (neighbors() - h) * (dab.amount * w).min(1.0),
                    SculptMode::Flatten => h + ((dab.level - t.base[k]).dot(t.normal) - h) * (dab.amount * w).min(1.0),
                };
                touched |= t.disp.heights[k] != h;
            }
        }
        if !touched {
            return Ok(false);
        }
        sew(&mut targets);
        let changed = targets.into_iter().zip(original).filter(|(t, h)| t.disp.heights != *h).map(|(t, _)| t).collect();
        self.store_targets(changed, "Sculpt", coalesce)?;
        Ok(true)
    }

    /// Sets the targeted displacements from a grayscale image stretched over all of them together,
    /// 0 at `low` and 1 at `high`; the image's top is the faces' far side (+Y on a floor).
    pub fn apply_heightmap(&mut self, gray: &[f32], width: usize, height: usize, low: f64, high: f64) -> Result<(), EngineError> {
        let mut targets = self.displacement_targets();
        if targets.is_empty() || width < 2 || height < 2 || gray.len() < width * height {
            return Err(EngineError::Refused("pick displaced faces and a heightmap image"));
        }
        let axis = targets[0].normal.abs().max_position();
        let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
        let (a, b) = if a < b { (a, b) } else { (b, a) };
        let pts = targets.iter().flat_map(|t| t.base.iter());
        let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
        for p in pts {
            (lo, hi) = (lo.min(*p), hi.max(*p));
        }
        let span = (hi - lo).max(DVec3::splat(1e-9));
        let at = |x: usize, y: usize| f64::from(gray.get(y.min(height - 1) * width + x.min(width - 1)).copied().unwrap_or(0.0));
        for t in &mut targets {
            for (k, p) in t.base.iter().enumerate() {
                let u = (p[a] - lo[a]) / span[a] * (width - 1) as f64;
                let v = (1.0 - (p[b] - lo[b]) / span[b]) * (height - 1) as f64;
                let (x, y) = (u.floor().max(0.0) as usize, v.floor().max(0.0) as usize);
                let (fx, fy) = (u - x as f64, v - y as f64);
                let g = (at(x, y) * (1.0 - fx) + at(x + 1, y) * fx) * (1.0 - fy) + (at(x, y + 1) * (1.0 - fx) + at(x + 1, y + 1) * fx) * fy;
                if let Some(h) = t.disp.heights.get_mut(k) {
                    *h = low + g * (high - low);
                }
            }
        }
        self.store_targets(targets, "Apply heightmap", None)
    }
}

/// Gives vertices that neighboring faces share one height, so their edges stay joined.
fn sew(targets: &mut [Target]) {
    let key = |p: DVec3, n: DVec3| ((p * 100.0).round().as_i64vec3().to_array(), (n * 1000.0).round().as_i64vec3().to_array());
    let mut shared: BTreeMap<_, Vec<(usize, usize)>> = BTreeMap::new();
    for (ti, t) in targets.iter().enumerate() {
        for (k, p) in t.base.iter().enumerate() {
            shared.entry(key(*p, t.normal)).or_default().push((ti, k));
        }
    }
    for list in shared.values().filter(|l| l.len() > 1) {
        let sum: f64 = list.iter().filter_map(|(ti, k)| targets.get(*ti)?.disp.heights.get(*k)).sum();
        let avg = sum / list.len() as f64;
        for (ti, k) in list {
            if let Some(h) = targets.get_mut(*ti).and_then(|t| t.disp.heights.get_mut(*k)) {
                *h = avg;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use mapcraft_geom::Aabb;

    use super::*;

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
        let dab = Sculpt { mode: SculptMode::Raise, center: DVec3::new(64.0, 32.0, 0.0), radius: 40.0, amount: 10.0, level: DVec3::ZERO };
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
        let dab = Sculpt { mode: SculptMode::Raise, center: DVec3::new(32.0, 32.0, 0.0), radius: 64.0, amount: 8.0, level: DVec3::ZERO };
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
}
