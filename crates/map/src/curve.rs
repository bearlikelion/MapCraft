//! Bezier paths and what curve groups build from them: faces swept along a path or spun about an
//! axis into runs of convex brushes, and copies placed along a path.

use std::collections::BTreeSet;

use mapcraft_geom::curve::{self, Measured, Sample, Span};
use mapcraft_geom::{Aabb, DMat3, DMat4, DQuat, DVec2, DVec3, EPSILON, Plane, convex_hull};
use serde::{Deserialize, Serialize};

use crate::{Brush, Face, FaceId, NodeId, TexAxis, TexDefaults};

/// More brushes than this from one curve group are refused, so a careless setting cannot hang the editor.
pub const MAX_BRUSHES: usize = 4096;
pub const MAX_COPIES: usize = 256;

/// A path point and its two handles, all in map space.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PathPoint {
    pub pos: DVec3,
    #[serde(rename = "in")]
    pub handle_in: DVec3,
    #[serde(rename = "out")]
    pub handle_out: DVec3,
}

impl PathPoint {
    /// A sharp point: both handles sit on it.
    pub fn corner(pos: DVec3) -> Self {
        Self { pos, handle_in: pos, handle_out: pos }
    }
}

/// A curve of cubic Bezier spans through its points (Blender's Bezier curve); it builds nothing itself.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Path {
    pub points: Vec<PathPoint>,
    #[serde(default)]
    pub closed: bool,
}

impl Path {
    pub fn spans(&self) -> Vec<Span> {
        let n = self.points.len();
        let count = if self.closed && n > 1 { n } else { n.saturating_sub(1) };
        (0..count)
            .filter_map(|i| {
                let (a, b) = (self.points.get(i)?, self.points.get((i + 1) % n)?);
                Some([a.pos, a.handle_out, b.handle_in, b.pos])
            })
            .collect()
    }

    /// The curve as a line through `per_span` points per span.
    pub fn polyline(&self, per_span: usize) -> Vec<DVec3> {
        let spans = self.spans();
        let per_span = per_span.max(1);
        let mut out: Vec<DVec3> = spans.iter().flat_map(|s| (0..per_span).map(move |k| curve::bezier(s, k as f64 / per_span as f64))).collect();
        out.extend(spans.last().map(|s| s[3]).or_else(|| self.points.first().map(|p| p.pos)));
        out
    }

    pub fn bounds(&self) -> Option<Aabb> {
        Aabb::from_points(self.polyline(16))
    }

    pub fn transformed(&self, m: &DMat4) -> Self {
        let t = |p: DVec3| m.transform_point3(p);
        let points = self.points.iter().map(|p| PathPoint { pos: t(p.pos), handle_in: t(p.handle_in), handle_out: t(p.handle_out) }).collect();
        Self { points, closed: self.closed }
    }

    fn measured_samples(&self, pieces: usize) -> Result<(Vec<Sample>, f64), &'static str> {
        let spans = self.spans();
        let m = Measured::new(&spans);
        let len = m.length();
        let samples = m.samples(pieces, self.closed);
        if !len.is_finite() || len <= EPSILON || samples.len() < 2 || samples.iter().any(|s| s.tangent == DVec3::ZERO) {
            return Err("the path needs at least two points apart");
        }
        Ok((samples, len))
    }
}

/// What a curve group builds from other nodes; its children are rebuilt when they or these settings change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum CurveOp {
    /// Each face swept along `path`, `segments` brushes per curved span, turning `twist` degrees
    /// and scaling its cross-section to `scale` by the end; `bind` places the faces' center in the
    /// frame of the path's start.
    Sweep {
        path: NodeId,
        faces: Vec<(NodeId, FaceId)>,
        segments: usize,
        #[serde(default)]
        twist: f64,
        #[serde(default = "one")]
        scale: f64,
        bind: DMat4,
    },
    /// Each face turned `angle` degrees about `axis` through `pivot`, one brush per step.
    Spin { faces: Vec<(NodeId, FaceId)>, axis: DVec3, pivot: DVec3, angle: f64, steps: usize },
    /// Copies of `nodes` along `path`: `count` of them, or one every `spacing` units when that is
    /// above zero, each turned with the path (about Z only with `yaw_only`); `bind` is as for a sweep.
    Array {
        path: NodeId,
        nodes: Vec<NodeId>,
        count: usize,
        #[serde(default)]
        spacing: f64,
        #[serde(default)]
        yaw_only: bool,
        bind: DMat4,
    },
}

fn one() -> f64 {
    1.0
}

impl CurveOp {
    pub fn sources(&self) -> Vec<NodeId> {
        match self {
            Self::Sweep { path, faces, .. } => std::iter::once(*path).chain(faces.iter().map(|f| f.0)).collect(),
            Self::Spin { faces, .. } => faces.iter().map(|f| f.0).collect(),
            Self::Array { path, nodes, .. } => std::iter::once(*path).chain(nodes.iter().copied()).collect(),
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Sweep { .. } => "Sweep",
            Self::Spin { .. } => "Spin",
            Self::Array { .. } => "Path Array",
        }
    }
}

/// A face to sweep or spin: its polygon, the face, and the face beside each edge from `polygon[j]`
/// to `polygon[j + 1]`, whose material and texture the side faces carry on.
#[derive(Clone, Debug)]
pub struct Profile {
    pub polygon: Vec<DVec3>,
    pub face: Face,
    pub sides: Vec<Option<Face>>,
}

impl Profile {
    pub fn of(brush: &Brush, id: FaceId) -> Option<Self> {
        let solid = brush.solid().ok()?;
        let i = brush.faces.iter().position(|f| f.id == id)?;
        let polygon = solid.faces.get(i)?.as_ref()?.0.clone();
        let near = |a: &DVec3, b: DVec3| a.distance_squared(b) < 1e-6;
        let n = polygon.len();
        let sides = (0..n)
            .map(|j| {
                let (a, b) = (polygon[j], polygon[(j + 1) % n]);
                brush
                    .faces
                    .iter()
                    .zip(&solid.faces)
                    .enumerate()
                    .find(|(k, (_, w))| *k != i && w.as_ref().is_some_and(|w| w.0.iter().any(|p| near(p, a)) && w.0.iter().any(|p| near(p, b))))
                    .map(|(_, (f, _))| f.clone())
            })
            .collect();
        Some(Self { polygon, face: brush.faces.get(i)?.clone(), sides })
    }
}

/// The frame at the start of `path`, as [`curve::frames`] makes it.
pub fn start_frame(path: &Path) -> Result<DMat4, &'static str> {
    let (samples, _) = path.measured_samples(1)?;
    curve::frames(&samples, path.closed).first().copied().ok_or("the path needs at least two points apart")
}

/// Where the profiles' corners sit on average.
pub fn centroid(profiles: &[Profile]) -> DVec3 {
    let pts: Vec<DVec3> = profiles.iter().flat_map(|p| p.polygon.iter().copied()).collect();
    pts.iter().sum::<DVec3>() / pts.len().max(1) as f64
}

/// Each profile, moved by `place` to the start of `path`, swept along it.
pub fn sweep(path: &Path, profiles: &[Profile], segments: usize, twist: f64, scale: f64, place: &DMat4) -> Result<Vec<Brush>, &'static str> {
    let (samples, len) = path.measured_samples(segments.clamp(1, 64))?;
    if samples.len().saturating_sub(1).saturating_mul(profiles.len()) > MAX_BRUSHES {
        return Err("that would make too many brushes; use fewer segments");
    }
    // A closed path's ends must meet, so it turns in whole turns and keeps its size.
    let (twist, scale) = match (path.closed, twist.is_finite(), scale.is_finite()) {
        (true, true, _) => ((twist / 360.0).round() * 360.0, 1.0),
        (true, false, _) => (0.0, 1.0),
        (false, t, s) => (if t { twist } else { 0.0 }, if s { scale.clamp(0.01, 100.0) } else { 1.0 }),
    };
    let frames = curve::frames(&samples, path.closed);
    let shaped: Vec<DMat4> = frames.iter().zip(&samples).map(|(f, s)| curve::twisted(f, twist * s.dist / len, 1.0 + (scale - 1.0) * s.dist / len)).collect();
    let start = shaped.first().ok_or("the path needs at least two points apart")?.inverse();
    let rings: Vec<DMat4> = shaped.iter().map(|f| *f * start * *place).collect();
    // Side textures unroll from the faces where they sit, so the start direction is taken back there.
    let t0 = place.inverse().transform_vector3(samples.first().map_or(DVec3::X, |s| s.tangent));
    loft_all(profiles, &rings, &|_| t0)
}

/// Each profile turned `angle` degrees about `axis` through `pivot` in `steps` brushes.
pub fn spin(profiles: &[Profile], axis: DVec3, pivot: DVec3, angle: f64, steps: usize) -> Result<Vec<Brush>, &'static str> {
    let axis = axis.normalize_or_zero();
    if axis == DVec3::ZERO || !pivot.is_finite() {
        return Err("the spin axis needs a direction");
    }
    let angle = if angle.is_finite() { angle.clamp(-360.0, 360.0) } else { 360.0 };
    if angle.abs() < 1e-6 {
        return Err("the spin angle must not be zero");
    }
    let steps = steps.clamp(1, 256);
    if steps.saturating_mul(profiles.len()) > MAX_BRUSHES {
        return Err("that would make too many brushes; use fewer steps");
    }
    let turn = |k: usize| {
        // The last step of a full turn lands exactly on the first.
        let a = if k == steps && angle.abs() == 360.0 { 0.0 } else { angle * k as f64 / steps as f64 };
        DMat4::from_translation(pivot) * DMat4::from_quat(DQuat::from_axis_angle(axis, a.to_radians())) * DMat4::from_translation(-pivot)
    };
    let motion = |c: DVec3| axis.cross(c - pivot) * angle.signum();
    loft_all(profiles, &(0..=steps).map(turn).collect::<Vec<_>>(), &motion)
}

/// The moves from the start of `path` to each copy's spot along it: `count` copies at even steps,
/// or one every `spacing` units when that is above zero.
pub fn placements(path: &Path, count: usize, spacing: f64, yaw_only: bool) -> Result<Vec<DMat4>, &'static str> {
    let (samples, len) = path.measured_samples(16)?;
    let spans = path.spans();
    let m = Measured::new(&spans);
    let dists: Vec<f64> = if spacing.is_finite() && spacing > EPSILON {
        let n = ((len / spacing + 1e-9).floor().min(MAX_COPIES as f64)) as usize;
        (1..=n).map(|i| i as f64 * spacing).filter(|d| !path.closed || *d < len - 1e-6).collect()
    } else {
        let count = count.clamp(1, MAX_COPIES);
        let step = len / if path.closed { count + 1 } else { count } as f64;
        (1..=count).map(|i| i as f64 * step).collect()
    };
    if dists.is_empty() {
        return Err("the spacing is longer than the path");
    }
    // The copies' spots join the path's own samples, so rotation-minimizing frames see every bend.
    let mut all: Vec<(Sample, bool)> = samples.into_iter().map(|s| (s, false)).chain(dists.iter().filter_map(|d| Some((m.at(*d)?, true)))).collect();
    all.sort_by(|a, b| a.0.dist.total_cmp(&b.0.dist));
    let only: Vec<Sample> = all.iter().map(|a| a.0).collect();
    let frames = curve::frames(&only, path.closed);
    let (Some(f0), Some(s0)) = (frames.first(), only.first()) else { return Err("the path needs at least two points apart") };
    let start = f0.inverse();
    let yaw0 = curve::yaw(s0.tangent).unwrap_or(0.0);
    Ok(all
        .iter()
        .zip(&frames)
        .filter(|((_, copy), _)| *copy)
        .map(|((s, _), f)| {
            if yaw_only {
                let turn = curve::yaw(s.tangent).map_or(0.0, |y| y - yaw0);
                DMat4::from_translation(s.at) * DMat4::from_rotation_z(turn.to_radians()) * DMat4::from_translation(-s0.at)
            } else {
                *f * start
            }
        })
        .collect())
}

/// `motion` gives the direction a point sets off in, which side textures unroll along.
fn loft_all(profiles: &[Profile], rings: &[DMat4], motion: &dyn Fn(DVec3) -> DVec3) -> Result<Vec<Brush>, &'static str> {
    let out: Vec<Brush> = profiles.iter().flat_map(|p| loft(p, rings, motion)).collect();
    if out.is_empty() {
        return Err("nothing to build: each face must cross its path, not lie along it");
    }
    Ok(out)
}

/// How side faces along one profile edge are textured: like the face beside that edge on the
/// profile's brush, unrolled along the way the profile travels.
struct SideTex {
    src: Face,
    /// Texel coordinates at the edge's two ends.
    base: [DVec2; 2],
    /// Texels per unit travelled.
    rate: DVec2,
}

impl SideTex {
    fn new(p: &Profile, j: usize, dir: DVec3) -> Self {
        let n = p.polygon.len();
        let src = p.sides.get(j).cloned().flatten().unwrap_or_else(|| p.face.clone());
        let grad = |a: &TexAxis| if a.scale != 0.0 { a.axis / a.scale } else { DVec3::ZERO };
        let mut rate = DVec2::new(grad(&src.u).dot(dir), grad(&src.v).dot(dir));
        if !rate.length().is_finite() || rate.length() <= 1e-9 {
            rate = DVec2::new(1.0 / if src.u.scale.abs() > 1e-9 { src.u.scale.abs() } else { 1.0 }, 0.0);
        }
        let at = |k: usize| p.polygon.get(k % n).map_or(DVec2::ZERO, |q| src.texel(*q));
        Self { base: [at(j), at(j + 1)], rate, src }
    }
}

/// One convex brush between each pair of neighboring rings, where ring `k` is the profile moved by `rings[k]`.
fn loft(p: &Profile, rings: &[DMat4], motion: &dyn Fn(DVec3) -> DVec3) -> Vec<Brush> {
    let n = p.polygon.len();
    if n < 3 {
        return Vec::new();
    }
    let pts: Vec<Vec<DVec3>> = rings.iter().map(|m| p.polygon.iter().map(|q| m.transform_point3(*q)).collect()).collect();
    // How far each corner has travelled by each ring, which side textures run along.
    let mut tracks: Vec<Vec<f64>> = vec![vec![0.0; n]];
    for w in pts.windows(2) {
        let last = tracks.last().cloned().unwrap_or_default();
        tracks.push(last.iter().zip(w[0].iter().zip(&w[1])).map(|(t, (a, b))| t + a.distance(*b)).collect());
    }
    let center = p.polygon.iter().sum::<DVec3>() / n as f64;
    let normal = p.face.plane().map_or(DVec3::Z, |pl| pl.normal);
    let dir = motion(center).try_normalize().unwrap_or(normal);
    let sides: Vec<SideTex> = (0..n).map(|j| SideTex::new(p, j, dir)).collect();
    let mut out = Vec::new();
    for k in 0..rings.len().saturating_sub(1) {
        let corners: Vec<(usize, usize, DVec3)> = (k..=k + 1).flat_map(|r| pts[r].iter().enumerate().map(move |(j, q)| (r, j, *q))).collect();
        let Some(hull) = convex_hull(&corners.iter().map(|c| c.2).collect::<Vec<_>>()) else { continue };
        let mut faces = Vec::with_capacity(hull.len());
        for tri in hull {
            let Some(plane) = Plane::from_points(tri[0], tri[1], tri[2]) else { continue };
            let on: Vec<(usize, usize, DVec3)> = corners.iter().copied().filter(|c| plane.distance(c.2).abs() < EPSILON).collect();
            let full = |r: usize| on.iter().filter(|c| c.0 == r).count() == n;
            let face = match (full(k), full(k + 1)) {
                (true, false) => cap(p, &rings[k], tri),
                (false, true) => cap(p, &rings[k + 1], tri),
                _ => side(&sides, n, &plane, tri, &on, &tracks),
            };
            faces.push(face.unwrap_or_else(|| {
                let mut f = Face::new(tri, &TexDefaults { material: p.face.material.clone(), scale: p.face.u.scale });
                f.align_to_face();
                f
            }));
        }
        if let Ok(b) = Brush::pruned(faces) {
            out.push(b);
        }
    }
    out
}

/// A face across the sweep, textured as the profile face would be if moved there.
fn cap(p: &Profile, m: &DMat4, tri: [DVec3; 3]) -> Option<Face> {
    let mut f = Brush { faces: vec![p.face.clone()] }.transformed(m, true).ok()?.faces.pop()?;
    (f.id, f.points, f.disp) = (FaceId(0), tri, None);
    Some(f)
}

/// A face along the sweep, its texture fitted to the unrolled coordinates of its corners.
fn side(sides: &[SideTex], n: usize, plane: &Plane, tri: [DVec3; 3], on: &[(usize, usize, DVec3)], tracks: &[Vec<f64>]) -> Option<Face> {
    let idx: BTreeSet<usize> = on.iter().map(|c| c.1).collect();
    let j = (0..n).find(|j| idx.contains(j) && idx.contains(&((j + 1) % n)))?;
    let s = sides.get(j)?;
    let pts: Vec<(DVec3, DVec2)> = on
        .iter()
        .filter_map(|&(r, i, q)| {
            let end = if i == j {
                0
            } else if i == (j + 1) % n {
                1
            } else {
                return None;
            };
            Some((q, s.base[end] + s.rate * *tracks.get(r)?.get(i)?))
        })
        .collect();
    let (u, v) = fit(plane, &pts)?;
    Some(Face { id: FaceId(0), points: tri, u, v, rotation: 0.0, disp: None, ..s.src.clone() })
}

/// Texture axes on `plane` that give each point its texel coordinates, as nearly as one plane mapping can.
fn fit(plane: &Plane, pts: &[(DVec3, DVec2)]) -> Option<(TexAxis, TexAxis)> {
    let (e1, e2) = plane.basis();
    let mean = pts.iter().map(|p| p.0).sum::<DVec3>() / pts.len().max(1) as f64;
    let (mut m, mut ru, mut rv) = (DMat3::ZERO, DVec3::ZERO, DVec3::ZERO);
    for (p, t) in pts {
        let d = *p - mean;
        let r = DVec3::new(d.dot(e1), d.dot(e2), 1.0);
        m += DMat3::from_cols(r * r.x, r * r.y, r * r.z);
        ru += r * t.x;
        rv += r * t.y;
    }
    let det = m.determinant().abs();
    if !det.is_finite() || det <= 1e-9 {
        return None;
    }
    let inv = m.inverse();
    let axis = |c: DVec3| {
        let g = e1 * c.x + e2 * c.y;
        let len = g.length();
        (len > 1e-12 && len.is_finite()).then(|| TexAxis { axis: g / len, scale: 1.0 / len, offset: c.z - g.dot(mean) })
    };
    Some((axis(inv * ru)?, axis(inv * rv)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A box whose faces are numbered, with the side faces given materials.
    fn block(min: DVec3, max: DVec3) -> Brush {
        let mut b = Brush::cuboid(Aabb::new(min, max), &TexDefaults { material: "trim".into(), scale: 0.5 });
        for (i, f) in b.faces.iter_mut().enumerate() {
            f.id = FaceId(i as u64 + 1);
        }
        b
    }

    fn face_toward(b: &Brush, n: DVec3) -> FaceId {
        b.faces.iter().find(|f| f.plane().unwrap().normal.abs_diff_eq(n, 1e-9)).unwrap().id
    }

    fn smooth(points: &[(DVec3, DVec3)], closed: bool) -> Path {
        Path { points: points.iter().map(|(p, d)| PathPoint { pos: *p, handle_in: *p - *d, handle_out: *p + *d }).collect(), closed }
    }

    fn s_curve() -> Path {
        smooth(&[(DVec3::ZERO, DVec3::X * 256.0), (DVec3::new(512.0, 256.0, 0.0), DVec3::X * 200.0), (DVec3::new(1024.0, 0.0, 0.0), DVec3::X * 256.0)], false)
    }

    fn shared(a: &[DVec3], b: &[DVec3]) -> Vec<DVec3> {
        a.iter().filter(|p| b.iter().any(|q| q.distance(**p) < 1e-6)).copied().collect()
    }

    /// Every brush is a closed convex solid, and each meets the next across a whole ring.
    fn check_solids(brushes: &[Brush], ring: usize) {
        for b in brushes {
            let s = b.solid().unwrap();
            let closure: DVec3 = s.faces.iter().flatten().map(|w| w.area_vector()).sum();
            assert!(closure.length() < 1e-6, "closed");
            let planes = b.planes().unwrap();
            assert!(s.vertices().all(|p| planes.iter().all(|q| q.distance(p) < 1e-6)), "convex");
            assert!(s.volume() > 1.0);
        }
        for w in brushes.windows(2) {
            assert_eq!(shared(&w[0].vertices().unwrap(), &w[1].vertices().unwrap()).len(), ring, "neighbors share a ring with no gap");
        }
    }

    /// Where two brushes share an edge between faces of one material, both faces give it the same texels.
    fn check_seams(brushes: &[Brush]) -> usize {
        let mut seams = 0;
        for w in brushes.windows(2) {
            let (sa, sb) = (w[0].solid().unwrap(), w[1].solid().unwrap());
            for (f, x) in w[0].faces.iter().zip(&sa.faces) {
                for (g, y) in w[1].faces.iter().zip(&sb.faces) {
                    let (Some(x), Some(y)) = (x, y) else { continue };
                    let common = shared(&x.0, &y.0);
                    if common.len() < 2 || f.material != g.material || f.plane().unwrap().normal.dot(g.plane().unwrap().normal) < 0.5 {
                        continue;
                    }
                    // A flat quad where the curve is not a circle can match only three corners, leaving a hair of seam.
                    for p in common {
                        assert!(f.texel(p).abs_diff_eq(g.texel(p), 0.25), "{} parts at {p:?}: {:?} {:?}", f.material, f.texel(p), g.texel(p));
                    }
                    seams += 1;
                }
            }
        }
        seams
    }

    #[test]
    fn sweeps_follow_an_s_curve_with_textures_running_on() {
        let mut b = block(DVec3::new(-16.0, -32.0, 0.0), DVec3::new(0.0, 32.0, 16.0));
        let top = face_toward(&b, DVec3::Z);
        for f in b.faces.iter_mut().filter(|f| f.id == top) {
            f.material = "road".into();
            f.u.offset = 7.0;
        }
        let profile = Profile::of(&b, face_toward(&b, DVec3::X)).unwrap();
        let brushes = sweep(&s_curve(), std::slice::from_ref(&profile), 8, 0.0, 1.0, &DMat4::IDENTITY).unwrap();
        assert_eq!(brushes.len(), 16);
        check_solids(&brushes, 4);
        assert!(check_seams(&brushes) >= 15 * 4, "every side runs on across every join");
        let first = &brushes[0];
        let start = first.faces.iter().find(|f| f.plane().unwrap().normal.abs_diff_eq(-DVec3::X, 1e-9)).unwrap();
        let p = DVec3::new(0.0, 10.0, 5.0);
        assert!(start.texel(p).abs_diff_eq(profile.face.texel(p), 1e-6), "the start keeps the profile face's mapping");
        let road: Vec<&Face> = brushes.iter().flat_map(|b| &b.faces).filter(|f| f.material == "road").collect();
        assert_eq!(road.len(), 16, "the top keeps its own material");
        // u runs along the path: one texel per half unit travelled, from the profile's own offset.
        let (corner, end) = (DVec3::new(0.0, 32.0, 16.0), brushes.last().unwrap());
        let last = end.faces.iter().rfind(|f| f.material == "road").unwrap();
        let tip = end.vertices().unwrap().into_iter().filter(|v| v.x > 1000.0 && v.z > 15.0).max_by(|a, b| a.y.total_cmp(&b.y)).unwrap();
        let travelled = last.texel(tip).x - b.faces.iter().find(|f| f.id == top).unwrap().texel(corner).x;
        assert!(travelled > 1100.0 * 2.0 && travelled < 1400.0 * 2.0, "{travelled}");
        assert!(brushes.iter().all(|b| b.vertices().unwrap().iter().all(|v| v.z > -1e-6 && v.z < 16.0 + 1e-6)), "a level path never banks");
    }

    #[test]
    fn twist_scale_and_refusals() {
        let b = block(DVec3::new(-16.0, -32.0, -16.0), DVec3::new(0.0, 32.0, 16.0));
        let profile = Profile::of(&b, face_toward(&b, DVec3::X)).unwrap();
        let line = smooth(&[(DVec3::ZERO, DVec3::ZERO), (DVec3::X * 512.0, DVec3::ZERO)], false);
        let straight = sweep(&line, std::slice::from_ref(&profile), 8, 0.0, 0.5, &DMat4::IDENTITY).unwrap();
        assert_eq!(straight.len(), 1, "a straight span is one brush");
        let end = straight[0].bounds().unwrap();
        assert!((end.max.y - 32.0).abs() < 1e-6);
        let twisted = sweep(&s_curve(), std::slice::from_ref(&profile), 8, 90.0, 1.0, &DMat4::IDENTITY).unwrap();
        check_solids(&twisted, 4);
        check_seams(&twisted);
        let flat = Profile::of(&b, face_toward(&b, DVec3::Z)).unwrap();
        assert!(sweep(&line, &[flat], 8, 0.0, 1.0, &DMat4::IDENTITY).is_err(), "a face lying along the path makes nothing");
        assert!(sweep(&Path::default(), std::slice::from_ref(&profile), 8, 0.0, 1.0, &DMat4::IDENTITY).is_err());
        let many: Vec<Profile> = (0..400).map(|_| profile.clone()).collect();
        assert!(sweep(&s_curve(), &many, 64, 0.0, 1.0, &DMat4::IDENTITY).is_err(), "too many brushes are refused");
    }

    #[test]
    fn spins_close_at_360() {
        let b = block(DVec3::new(64.0, 0.0, 0.0), DVec3::new(128.0, 16.0, 32.0));
        let profile = Profile::of(&b, face_toward(&b, -DVec3::Y)).unwrap();
        let ring = spin(std::slice::from_ref(&profile), DVec3::Z, DVec3::ZERO, 360.0, 16).unwrap();
        assert_eq!(ring.len(), 16);
        check_solids(&ring, 4);
        assert!(check_seams(&ring) >= 15 * 3);
        assert_eq!(shared(&ring[0].vertices().unwrap(), &ring[15].vertices().unwrap()).len(), 4, "the last brush meets the first");
        let volume: f64 = ring.iter().map(|b| b.solid().unwrap().volume()).sum();
        let a = std::f64::consts::TAU / 16.0;
        let want = 16.0 * 0.5 * a.sin() * (128.0f64.powi(2) - 64.0f64.powi(2)) * 32.0;
        assert!((volume - want).abs() < 1e-3, "{volume} {want}");
        let floor = block(DVec3::new(64.0, 0.0, 0.0), DVec3::new(96.0, 32.0, 16.0));
        let arch = spin(&[Profile::of(&floor, face_toward(&floor, -DVec3::Z)).unwrap()], -DVec3::Y, DVec3::ZERO, 180.0, 8).unwrap();
        assert_eq!(arch.len(), 8);
        check_solids(&arch, 4);
        assert!(arch.iter().all(|b| b.bounds().unwrap().min.z > -1e-6), "the arch rises over the axis");
        let flat = Profile::of(&b, face_toward(&b, DVec3::Z)).unwrap();
        assert!(spin(&[flat], DVec3::Z, DVec3::ZERO, 90.0, 4).is_err(), "a face turning in its own plane makes nothing");
        assert!(spin(&[profile], DVec3::ZERO, DVec3::ZERO, 90.0, 4).is_err());
    }

    #[test]
    fn copies_spread_along_the_path() {
        let line = smooth(&[(DVec3::ZERO, DVec3::ZERO), (DVec3::X * 400.0, DVec3::ZERO)], false);
        let at = |m: &[DMat4]| m.iter().map(|m| m.transform_point3(DVec3::ZERO).x.round()).collect::<Vec<_>>();
        assert_eq!(at(&placements(&line, 4, 0.0, false).unwrap()), vec![100.0, 200.0, 300.0, 400.0]);
        assert_eq!(at(&placements(&line, 4, 150.0, false).unwrap()), vec![150.0, 300.0]);
        assert!(placements(&line, 4, 500.0, false).is_err());
        let r = 256.0;
        let k = 0.5523 * r;
        let circle = smooth(
            &[
                (DVec3::new(r, 0.0, 0.0), DVec3::Y * k),
                (DVec3::new(0.0, r, 0.0), -DVec3::X * k),
                (DVec3::new(-r, 0.0, 0.0), -DVec3::Y * k),
                (DVec3::new(0.0, -r, 0.0), DVec3::X * k),
            ],
            true,
        );
        let copies = placements(&circle, 3, 0.0, true).unwrap();
        assert_eq!(copies.len(), 3);
        let half = copies[1];
        assert!(half.transform_point3(DVec3::new(r, 0.0, 0.0)).distance(DVec3::new(-r, 0.0, 0.0)) < 1.0, "the middle copy is across the loop");
        assert!(half.transform_vector3(DVec3::Y).distance(-DVec3::Y) < 1e-3, "and turned to face along it");
        let full = placements(&circle, 3, 0.0, false).unwrap();
        assert!(full[0].transform_vector3(DVec3::Z).distance(DVec3::Z) < 1e-9, "a level loop keeps copies upright");
    }
}
