use std::collections::BTreeMap;

use mapcraft_geom::{Aabb, DMat3, DMat4, DVec2, DVec3, Plane, Side, Solid, SolidError, convex_hull};
use serde::{Deserialize, Serialize};

use crate::{Extra, FaceId};

/// One texture axis in Valve220 form: `u = (p . axis) / scale + offset`, in texels.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TexAxis {
    pub axis: DVec3,
    pub offset: f64,
    pub scale: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Face {
    #[serde(default)]
    pub id: FaceId,
    /// Clockwise seen from outside; see [`Plane::from_points`].
    pub points: [DVec3; 3],
    pub material: String,
    pub u: TexAxis,
    pub v: TexAxis,
    #[serde(default)]
    pub rotation: f64,
    /// Per-face game attributes such as `lightmapscale` or `smoothing_groups`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attrs: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

/// Where to anchor a texture on its face.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Justify {
    Fit,
    Left,
    Right,
    Top,
    Bottom,
    Center,
}

/// Material and texture scale for newly created faces.
#[derive(Clone, Debug, PartialEq)]
pub struct TexDefaults {
    pub material: String,
    pub scale: f64,
}

impl Default for TexDefaults {
    fn default() -> Self {
        Self { material: String::new(), scale: 1.0 }
    }
}

fn world_axes(normal: DVec3) -> (DVec3, DVec3) {
    let a = normal.abs();
    if a.z >= a.x && a.z >= a.y {
        (DVec3::X, -DVec3::Y)
    } else if a.x >= a.y {
        (DVec3::Y, -DVec3::Z)
    } else {
        (DVec3::X, -DVec3::Z)
    }
}

impl Face {
    pub fn new(points: [DVec3; 3], tex: &TexDefaults) -> Self {
        let axis = |axis| TexAxis { axis, offset: 0.0, scale: tex.scale };
        let (u, v) = Plane::from_points(points[0], points[1], points[2]).map(|p| world_axes(p.normal)).unwrap_or((DVec3::X, -DVec3::Y));
        Self { id: FaceId(0), points, material: tex.material.clone(), u: axis(u), v: axis(v), rotation: 0.0, attrs: BTreeMap::new(), extra: Extra::new() }
    }

    pub fn plane(&self) -> Option<Plane> {
        Plane::from_points(self.points[0], self.points[1], self.points[2])
    }

    /// Normalized texture coordinates of a point for a texture of `size` texels.
    pub fn uv(&self, p: DVec3, size: DVec2) -> DVec2 {
        let texel = |a: &TexAxis| if a.scale != 0.0 { p.dot(a.axis) / a.scale + a.offset } else { a.offset };
        let size = size.max(DVec2::ONE);
        DVec2::new(texel(&self.u) / size.x, texel(&self.v) / size.y)
    }

    pub fn align_to_world(&mut self) {
        if let Some(p) = self.plane() {
            (self.u.axis, self.v.axis) = world_axes(p.normal);
            self.rotation = 0.0;
        }
    }

    /// Axes in the face plane, with v pointing as far down as the plane allows.
    pub fn align_to_face(&mut self) {
        let Some(p) = self.plane() else { return };
        let down = -DVec3::Z;
        let v = (down - p.normal * down.dot(p.normal)).normalize_or_zero();
        if v == DVec3::ZERO || p.normal.z.abs() > 0.999 {
            self.align_to_world();
            return;
        }
        self.u.axis = p.normal.cross(v);
        self.v.axis = v;
        self.rotation = 0.0;
    }

    /// Rotates the texture on the face by `degrees` around the face normal.
    pub fn rotate_texture(&mut self, degrees: f64) {
        let Some(p) = self.plane() else { return };
        let q = mapcraft_geom::DQuat::from_axis_angle(p.normal, degrees.to_radians());
        self.u.axis = q * self.u.axis;
        self.v.axis = q * self.v.axis;
        self.rotation = (self.rotation + degrees).rem_euclid(360.0);
    }

    /// Aligns the texture to the face's polygon (Hammer's justify buttons).
    pub fn justify(&mut self, polygon: &[DVec3], size: DVec2, mode: Justify) {
        let range = |a: &TexAxis| polygon.iter().map(|p| p.dot(a.axis)).fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), x| (lo.min(x), hi.max(x)));
        let ((u0, u1), (v0, v1)) = (range(&self.u), range(&self.v));
        if !(u0.is_finite() && v0.is_finite()) || size.x <= 0.0 || size.y <= 0.0 {
            return;
        }
        if mode == Justify::Fit {
            if u1 - u0 > 1e-9 {
                self.u.scale = (u1 - u0) / size.x;
            }
            if v1 - v0 > 1e-9 {
                self.v.scale = (v1 - v0) / size.y;
            }
        }
        let (us, vs) = (self.u.scale, self.v.scale);
        if us == 0.0 || vs == 0.0 {
            return;
        }
        match mode {
            Justify::Fit => {
                self.u.offset = -u0 / us;
                self.v.offset = -v0 / vs;
            }
            Justify::Left => self.u.offset = -u0 / us,
            Justify::Right => self.u.offset = size.x - u1 / us,
            Justify::Top => self.v.offset = -v0 / vs,
            Justify::Bottom => self.v.offset = size.y - v1 / vs,
            Justify::Center => {
                self.u.offset = size.x / 2.0 - (u0 + u1) / 2.0 / us;
                self.v.offset = size.y / 2.0 - (v0 + v1) / 2.0 / vs;
            }
        }
        self.u.offset = self.u.offset.rem_euclid(size.x);
        self.v.offset = self.v.offset.rem_euclid(size.y);
    }

    /// Takes `src`'s material and alignment, turned about the line where the two planes meet so
    /// the texture runs on across a shared edge (Hammer's apply with alignment).
    pub fn wrap_from(&mut self, src: &Face) {
        self.material = src.material.clone();
        let (Some(a), Some(b)) = (src.plane(), self.plane()) else { return };
        let line = a.normal.cross(b.normal);
        let (q, pivot) = if line.length_squared() < 1e-12 {
            (mapcraft_geom::DQuat::IDENTITY, DVec3::ZERO)
        } else {
            let pivot = (a.dist * b.normal.cross(line) + b.dist * line.cross(a.normal)) / line.length_squared();
            (mapcraft_geom::DQuat::from_rotation_arc(a.normal, b.normal), pivot)
        };
        for (dst, s) in [(&mut self.u, &src.u), (&mut self.v, &src.v)] {
            let axis = q * s.axis;
            let shift = if s.scale != 0.0 { (s.axis.dot(pivot) - axis.dot(pivot)) / s.scale } else { 0.0 };
            *dst = TexAxis { axis, offset: s.offset + shift, scale: s.scale };
        }
        self.rotation = src.rotation;
    }

    fn reversed(&self) -> Self {
        let mut f = self.clone();
        f.points = [self.points[2], self.points[1], self.points[0]];
        f.id = FaceId(0);
        f
    }

    fn transform(&mut self, m: &DMat4, inv_t: &DMat3, mirror: bool, lock: bool) {
        let p = self.points.map(|p| m.transform_point3(p));
        self.points = if mirror { [p[2], p[1], p[0]] } else { p };
        if !lock {
            return;
        }
        let t = m.w_axis.truncate();
        for a in [&mut self.u, &mut self.v] {
            // The inverse transpose keeps p . axis unchanged under the linear part.
            let axis = *inv_t * a.axis;
            let len = axis.length();
            if len > 1e-12 && a.scale != 0.0 && len.is_finite() {
                a.offset -= t.dot(axis) / a.scale;
                a.axis = axis / len;
                a.scale /= len;
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum BrushError {
    #[error("face {0} has collinear points")]
    DegenerateFace(usize),
    #[error(transparent)]
    Solid(#[from] SolidError),
    #[error("the transform cannot be inverted")]
    SingularTransform,
}

/// A convex solid stored as its faces.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Brush {
    pub faces: Vec<Face>,
}

impl Brush {
    pub fn cuboid(b: Aabb, tex: &TexDefaults) -> Self {
        let (n, x) = (b.min, b.max);
        let pts = [
            [DVec3::new(x.x, x.y, n.z), DVec3::new(x.x, n.y, n.z), DVec3::new(x.x, n.y, x.z)],
            [DVec3::new(n.x, n.y, x.z), DVec3::new(n.x, n.y, n.z), DVec3::new(n.x, x.y, n.z)],
            [DVec3::new(n.x, x.y, x.z), DVec3::new(n.x, x.y, n.z), DVec3::new(x.x, x.y, n.z)],
            [DVec3::new(x.x, n.y, n.z), DVec3::new(n.x, n.y, n.z), DVec3::new(n.x, n.y, x.z)],
            [DVec3::new(x.x, n.y, x.z), DVec3::new(n.x, n.y, x.z), DVec3::new(n.x, x.y, x.z)],
            [DVec3::new(n.x, x.y, n.z), DVec3::new(n.x, n.y, n.z), DVec3::new(x.x, n.y, n.z)],
        ];
        Self { faces: pts.into_iter().map(|p| Face::new(p, tex)).collect() }
    }

    /// The convex hull of `points`; each face takes its texture from the face of `like` it matches
    /// best, and keeps that face's id when the plane did not move.
    pub fn from_vertices(points: &[DVec3], like: &[Face], tex: &TexDefaults) -> Result<Self, BrushError> {
        let hull = convex_hull(points).ok_or(BrushError::Solid(SolidError::NoVolume))?;
        let old: Vec<(Plane, &Face)> = like.iter().filter_map(|f| Some((f.plane()?, f))).collect();
        let mut used = std::collections::BTreeSet::new();
        let mut faces = Vec::with_capacity(hull.len());
        for pts in hull {
            let Some(plane) = Plane::from_points(pts[0], pts[1], pts[2]) else { continue };
            let best = old.iter().max_by(|a, b| a.0.normal.dot(plane.normal).total_cmp(&b.0.normal.dot(plane.normal)));
            let mut face = match best {
                Some((p, f)) if p.approx_eq(&plane, 1e-3) && (f.id == FaceId(0) || used.insert(f.id)) => {
                    let mut f = (*f).clone();
                    f.points = pts;
                    f
                }
                Some((_, f)) => {
                    let mut n = Face::new(pts, &TexDefaults { material: f.material.clone(), scale: f.u.scale });
                    n.attrs = f.attrs.clone();
                    n
                }
                None => Face::new(pts, tex),
            };
            face.points = pts;
            faces.push(face);
        }
        Self::pruned(faces)
    }

    /// Distinct corner points.
    pub fn vertices(&self) -> Result<Vec<DVec3>, BrushError> {
        let mut out: Vec<DVec3> = Vec::new();
        for p in self.solid()?.vertices() {
            if !out.iter().any(|q| q.distance_squared(p) < 1e-6) {
                out.push(p);
            }
        }
        Ok(out)
    }

    /// A wedge filling `b`: full height at min X sloping down to max X.
    pub fn wedge(b: Aabb, tex: &TexDefaults) -> Result<Self, BrushError> {
        let (n, x) = (b.min, b.max);
        let pts = [
            DVec3::new(n.x, n.y, n.z),
            DVec3::new(x.x, n.y, n.z),
            DVec3::new(x.x, x.y, n.z),
            DVec3::new(n.x, x.y, n.z),
            DVec3::new(n.x, n.y, x.z),
            DVec3::new(n.x, x.y, x.z),
        ];
        Self::from_vertices(&pts, &[], tex)
    }

    fn ring(b: &Aabb, sides: usize, z: f64) -> impl Iterator<Item = DVec3> + '_ {
        let (c, r) = (b.center(), b.size() / 2.0);
        (0..sides).map(move |i| {
            let a = std::f64::consts::TAU * (i as f64 + 0.5) / sides as f64;
            DVec3::new(c.x + r.x * a.cos(), c.y + r.y * a.sin(), z)
        })
    }

    /// An upright prism with `sides` sides inscribed in `b`.
    pub fn cylinder(b: Aabb, sides: usize, tex: &TexDefaults) -> Result<Self, BrushError> {
        let sides = sides.clamp(3, 64);
        let pts: Vec<DVec3> = Self::ring(&b, sides, b.min.z).chain(Self::ring(&b, sides, b.max.z)).collect();
        Self::from_vertices(&pts, &[], tex)
    }

    /// A pyramid or cone with `sides` sides inscribed in `b`, apex at the top.
    pub fn spike(b: Aabb, sides: usize, tex: &TexDefaults) -> Result<Self, BrushError> {
        let sides = sides.clamp(3, 64);
        let apex = DVec3::new(b.center().x, b.center().y, b.max.z);
        let pts: Vec<DVec3> = Self::ring(&b, sides, b.min.z).chain(std::iter::once(apex)).collect();
        Self::from_vertices(&pts, &[], tex)
    }

    /// A sphere (an ellipsoid for uneven boxes) inscribed in `b`, `sides` faces around.
    pub fn sphere(b: Aabb, sides: usize, tex: &TexDefaults) -> Result<Self, BrushError> {
        let sides = sides.clamp(4, 32);
        let rings = sides / 2;
        let (c, r) = (b.center(), b.size() / 2.0);
        let mut pts = vec![c - DVec3::Z * r.z, c + DVec3::Z * r.z];
        for i in 1..rings {
            let lat = std::f64::consts::PI * i as f64 / rings as f64 - std::f64::consts::FRAC_PI_2;
            for j in 0..sides {
                let lon = std::f64::consts::TAU * (j as f64 + 0.5) / sides as f64;
                pts.push(c + DVec3::new(r.x * lat.cos() * lon.cos(), r.y * lat.cos() * lon.sin(), r.z * lat.sin()));
            }
        }
        Self::from_vertices(&pts, &[], tex)
    }

    /// The blocks of an arch inscribed in `b` around Z: `sides` blocks spanning `arc` degrees
    /// counter-clockwise from `start` degrees off +X, each `thickness` deep (Hammer's arch).
    pub fn arch(b: Aabb, sides: usize, arc: f64, start: f64, thickness: f64, tex: &TexDefaults) -> Result<Vec<Self>, BrushError> {
        let sides = sides.clamp(1, 64);
        let arc = arc.clamp(1.0, 360.0);
        let (c, r) = (b.center(), b.size() / 2.0);
        let (ix, iy) = ((r.x - thickness).max(0.0), (r.y - thickness).max(0.0));
        let at = |deg: f64, rx: f64, ry: f64, z: f64| {
            let a = deg.to_radians();
            DVec3::new(c.x + rx * a.cos(), c.y + ry * a.sin(), z)
        };
        (0..sides)
            .map(|i| {
                let (a0, a1) = (start + arc * i as f64 / sides as f64, start + arc * (i + 1) as f64 / sides as f64);
                let mut pts = Vec::with_capacity(8);
                for z in [b.min.z, b.max.z] {
                    for a in [a0, a1] {
                        pts.push(at(a, r.x, r.y, z));
                        pts.push(at(a, ix, iy, z));
                    }
                }
                Self::from_vertices(&pts, &[], tex)
            })
            .collect()
    }

    /// One brush around all of `brushes` (Hammer and TrenchBroom's convex merge).
    pub fn merge(brushes: &[Self]) -> Result<Self, BrushError> {
        let mut pts = Vec::new();
        let mut faces = Vec::new();
        for b in brushes {
            pts.extend(b.vertices()?);
            faces.extend(b.faces.iter().cloned());
        }
        Self::from_vertices(&pts, &faces, &TexDefaults::default()).map(Self::with_new_ids)
    }

    pub fn planes(&self) -> Result<Vec<Plane>, BrushError> {
        self.faces.iter().enumerate().map(|(i, f)| f.plane().ok_or(BrushError::DegenerateFace(i))).collect()
    }

    pub fn solid(&self) -> Result<Solid, BrushError> {
        Ok(Solid::from_planes(&self.planes()?)?)
    }

    pub fn bounds(&self) -> Result<Aabb, BrushError> {
        Ok(self.solid()?.bounds)
    }

    /// The brush with faces that cut nothing away removed.
    pub fn pruned(faces: Vec<Face>) -> Result<Self, BrushError> {
        let b = Self { faces };
        let solid = b.solid()?;
        Ok(Self { faces: b.faces.into_iter().zip(solid.faces).filter_map(|(f, w)| w.map(|_| f)).collect() })
    }

    /// Applies an affine transform; with `lock` the texture stays fixed to the surface.
    pub fn transformed(&self, m: &DMat4, lock: bool) -> Result<Self, BrushError> {
        let linear = DMat3::from_mat4(*m);
        let det = linear.determinant();
        if !det.is_finite() || det.abs() <= 1e-12 {
            return Err(BrushError::SingularTransform);
        }
        let inv_t = linear.inverse().transpose();
        let mut b = self.clone();
        for f in &mut b.faces {
            f.transform(m, &inv_t, det < 0.0, lock);
        }
        Ok(b)
    }

    pub fn translated(&self, t: DVec3, lock: bool) -> Self {
        let mut b = self.clone();
        for f in &mut b.faces {
            f.points = f.points.map(|p| p + t);
            if lock {
                for a in [&mut f.u, &mut f.v] {
                    if a.scale != 0.0 {
                        a.offset -= t.dot(a.axis) / a.scale;
                    }
                }
            }
        }
        b
    }

    fn with_new_ids(mut self) -> Self {
        for f in &mut self.faces {
            f.id = FaceId(0);
        }
        self
    }

    /// Splits along the plane through `points` into (front, back); the back part keeps the face ids.
    pub fn split(&self, points: [DVec3; 3], tex: &TexDefaults) -> Result<(Option<Self>, Option<Self>), BrushError> {
        let plane = Plane::from_points(points[0], points[1], points[2]).ok_or(BrushError::DegenerateFace(self.faces.len()))?;
        match self.solid()?.side_of(&plane) {
            Side::Front => Ok((Some(self.clone()), None)),
            Side::Back | Side::On => Ok((None, Some(self.clone()))),
            Side::Spanning => {
                let cut = Face::new(points, tex);
                let mut back = self.faces.clone();
                back.push(cut.clone());
                let mut front = self.clone().with_new_ids().faces;
                front.push(cut.reversed());
                Ok((Self::pruned(front).ok(), Self::pruned(back).ok()))
            }
        }
    }

    /// The parts of this brush outside `cutter` (Hammer's carve).
    pub fn subtract(&self, cutter: &Self) -> Result<Vec<Self>, BrushError> {
        let solid = self.solid()?;
        let cutter_solid = cutter.solid()?;
        if !solid.bounds.intersects(&cutter_solid.bounds) {
            return Ok(vec![self.clone()]);
        }
        let (mut rest, mut rest_solid, mut out) = (self.clone(), solid, Vec::new());
        for (face, winding) in cutter.faces.iter().zip(&cutter_solid.faces) {
            let (Some(_), Some(plane)) = (winding, face.plane()) else { continue };
            match rest_solid.side_of(&plane) {
                Side::Front => return Ok(vec![self.clone()]),
                Side::Back | Side::On => {}
                Side::Spanning => {
                    let mut outside = rest.faces.clone();
                    outside.push(face.reversed());
                    let mut inside = rest.faces.clone();
                    inside.push(face.clone());
                    if let Ok(b) = Self::pruned(outside) {
                        out.push(b.with_new_ids());
                    }
                    let Ok(b) = Self::pruned(inside) else { return Ok(out) };
                    rest_solid = b.solid()?;
                    rest = b;
                }
            }
        }
        Ok(out)
    }

    pub fn intersect(&self, other: &Self) -> Option<Self> {
        let mut faces = self.faces.clone();
        faces.extend(other.faces.iter().cloned());
        Self::pruned(faces).ok().map(Self::with_new_ids)
    }

    /// Walls of the given thickness around a hollow interior.
    pub fn hollow(&self, thickness: f64) -> Result<Vec<Self>, BrushError> {
        let mut inner = self.clone();
        for (i, f) in inner.faces.iter_mut().enumerate() {
            let n = f.plane().ok_or(BrushError::DegenerateFace(i))?.normal;
            f.points = f.points.map(|p| p - n * thickness);
        }
        inner.solid()?;
        self.subtract(&inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(min: f64, max: f64) -> Brush {
        Brush::cuboid(Aabb::new(DVec3::splat(min), DVec3::splat(max)), &TexDefaults { material: "dev".into(), scale: 0.25 })
    }

    fn volume(brushes: &[Brush]) -> f64 {
        brushes.iter().map(|b| b.solid().unwrap().volume()).sum()
    }

    #[test]
    fn cuboid_faces_point_out() {
        let b = cube(0.0, 64.0);
        let s = b.solid().unwrap();
        assert_eq!(s.faces.iter().flatten().count(), 6);
        for (f, w) in b.faces.iter().zip(&s.faces) {
            let w = w.as_ref().unwrap();
            assert!(f.plane().unwrap().normal.dot(w.center() - DVec3::splat(32.0)) > 0.0);
        }
    }

    #[test]
    fn texture_lock_keeps_uvs() {
        let b = cube(0.0, 64.0);
        let size = DVec2::new(128.0, 128.0);
        let m = DMat4::from_rotation_translation(mapcraft_geom::DQuat::from_rotation_z(0.7), DVec3::new(10.0, -3.0, 5.0));
        let moved = b.transformed(&m, true).unwrap();
        let shifted = b.translated(DVec3::new(16.0, 8.0, 0.0), true);
        let p = DVec3::new(64.0, 20.0, 30.0);
        let before = b.faces[0].uv(p, size);
        assert!(moved.faces[0].uv(m.transform_point3(p), size).abs_diff_eq(before, 1e-9));
        assert!(shifted.faces[0].uv(p + DVec3::new(16.0, 8.0, 0.0), size).abs_diff_eq(before, 1e-9));
        assert!((moved.solid().unwrap().volume() - 64.0f64.powi(3)).abs() < 1e-6);
    }

    #[test]
    fn mirror_keeps_brush_valid() {
        let m = DMat4::from_scale(DVec3::new(-1.0, 1.0, 1.0));
        let b = cube(0.0, 64.0).transformed(&m, true).unwrap();
        assert!((b.solid().unwrap().volume() - 64.0f64.powi(3)).abs() < 1e-6);
        assert!(cube(0.0, 64.0).transformed(&DMat4::from_scale(DVec3::ZERO), true).is_err());
    }

    #[test]
    fn split() {
        let b = cube(0.0, 64.0);
        let pts = [DVec3::new(32.0, 1.0, 0.0), DVec3::new(32.0, 0.0, 0.0), DVec3::new(32.0, 0.0, 1.0)];
        let (front, back) = b.split(pts, &TexDefaults::default()).unwrap();
        let (front, back) = (front.unwrap(), back.unwrap());
        assert!((volume(std::slice::from_ref(&front)) - volume(std::slice::from_ref(&back))).abs() < 1e-6);
        assert!(back.faces.iter().any(|f| f.id == FaceId(0)));
        assert!(front.solid().unwrap().bounds.min.x >= 32.0 - 1e-9);
        let miss = [DVec3::new(100.0, 1.0, 0.0), DVec3::new(100.0, 0.0, 0.0), DVec3::new(100.0, 0.0, 1.0)];
        assert!(b.split(miss, &TexDefaults::default()).unwrap().0.is_none());
    }

    #[test]
    fn carve() {
        let a = cube(0.0, 64.0);
        let cutter = cube(48.0, 80.0);
        let parts = a.subtract(&cutter).unwrap();
        assert!(parts.len() >= 3);
        assert!((volume(&parts) - (64.0f64.powi(3) - 16.0f64.powi(3))).abs() < 1e-6);
        assert_eq!(a.subtract(&cube(100.0, 120.0)).unwrap(), vec![a.clone()]);
        assert!(a.subtract(&cube(-10.0, 100.0)).unwrap().is_empty());
        assert!((volume(&[a.intersect(&cutter).unwrap()]) - 16.0f64.powi(3)).abs() < 1e-6);
    }

    #[test]
    fn hollow() {
        let walls = cube(0.0, 64.0).hollow(8.0).unwrap();
        assert_eq!(walls.len(), 6);
        assert!((volume(&walls) - (64.0f64.powi(3) - 48.0f64.powi(3))).abs() < 1e-6);
        assert!(cube(0.0, 64.0).hollow(40.0).is_err());
    }

    #[test]
    fn primitives_and_vertex_edits() {
        let tex = TexDefaults { material: "dev".into(), scale: 1.0 };
        let b = Aabb::new(DVec3::ZERO, DVec3::splat(64.0));
        assert!((volume(&[Brush::wedge(b, &tex).unwrap()]) - 64.0f64.powi(3) / 2.0).abs() < 1e-6);
        let cyl = Brush::cylinder(b, 8, &tex).unwrap();
        assert_eq!(cyl.faces.len(), 10);
        assert_eq!(Brush::spike(b, 4, &tex).unwrap().faces.len(), 5);

        let mut c = cube(0.0, 64.0);
        for (i, f) in c.faces.iter_mut().enumerate() {
            f.id = FaceId(i as u64 + 1);
        }
        let mut verts = c.vertices().unwrap();
        assert_eq!(verts.len(), 8);
        let top = verts.iter().position(|v| *v == DVec3::splat(64.0)).unwrap();
        verts[top] = DVec3::new(64.0, 64.0, 128.0);
        let edited = Brush::from_vertices(&verts, &c.faces, &tex).unwrap();
        // Untouched faces keep their ids and textures.
        let kept = edited.faces.iter().filter(|f| c.faces.iter().any(|o| o.id == f.id && o.material == f.material && f.id != FaceId(0))).count();
        // Moving the corner up keeps the four side planes and the bottom.
        assert_eq!(kept, 5);
        assert!(edited.faces.iter().all(|f| f.material == "dev"));

        let merged = Brush::merge(&[cube(0.0, 32.0), Brush::cuboid(Aabb::new(DVec3::new(32.0, 0.0, 0.0), DVec3::new(64.0, 32.0, 32.0)), &tex)]).unwrap();
        assert!((volume(&[merged]) - 64.0 * 32.0 * 32.0).abs() < 1e-6);
    }

    #[test]
    fn sphere_and_arch() {
        let tex = TexDefaults::default();
        let s = Brush::sphere(Aabb::new(DVec3::ZERO, DVec3::splat(64.0)), 8, &tex).unwrap();
        assert_eq!(s.faces.len(), 32);
        let v = volume(std::slice::from_ref(&s));
        assert!(v > 0.4 * 64.0f64.powi(3) && v < 0.53 * 64.0f64.powi(3), "{v}");
        let arch = Brush::arch(Aabb::new(DVec3::ZERO, DVec3::new(128.0, 128.0, 32.0)), 8, 180.0, 0.0, 16.0, &tex).unwrap();
        assert_eq!(arch.len(), 8);
        assert!(arch.iter().all(|b| b.bounds().unwrap().min.y >= 64.0 - 1e-9));
        let ring = Brush::arch(Aabb::new(DVec3::ZERO, DVec3::new(128.0, 128.0, 32.0)), 4, 360.0, 0.0, 200.0, &tex).unwrap();
        assert!((volume(&ring) - 64.0 * 64.0 * 2.0 * 32.0).abs() < 1e-6);
    }

    #[test]
    fn wrap_keeps_shared_edge_continuous() {
        let b = cube(0.0, 64.0);
        let mut top = b.faces[4].clone();
        top.u.offset = 13.0;
        top.v.offset = 7.0;
        top.rotate_texture(30.0);
        let mut side = b.faces[0].clone();
        side.wrap_from(&top);
        let size = DVec2::splat(64.0);
        for p in [DVec3::new(64.0, 0.0, 64.0), DVec3::new(64.0, 50.0, 64.0)] {
            assert!(side.uv(p, size).abs_diff_eq(top.uv(p, size), 1e-9));
        }
        let n = side.plane().unwrap().normal;
        assert!(side.u.axis.dot(n).abs() < 1e-9 && side.v.axis.dot(n).abs() < 1e-9);
    }

    #[test]
    fn justify_and_rotate() {
        let b = cube(0.0, 64.0);
        let solid = b.solid().unwrap();
        let poly = solid.faces[4].as_ref().unwrap().0.clone();
        let mut f = b.faces[4].clone();
        let size = DVec2::new(128.0, 128.0);
        f.justify(&poly, size, Justify::Fit);
        let uvs: Vec<DVec2> = poly.iter().map(|p| f.uv(*p, size)).collect();
        let lo = uvs.iter().fold(DVec2::splat(f64::INFINITY), |a, b| a.min(*b));
        let hi = uvs.iter().fold(DVec2::splat(f64::NEG_INFINITY), |a, b| a.max(*b));
        assert!((hi - lo).abs_diff_eq(DVec2::ONE, 1e-9), "{lo:?} {hi:?}");
        let n = f.plane().unwrap().normal;
        f.rotate_texture(90.0);
        assert!(f.u.axis.dot(n).abs() < 1e-9 && (f.rotation - 90.0).abs() < 1e-9);
    }

    #[test]
    fn face_alignment() {
        let mut f = cube(0.0, 64.0).faces[0].clone();
        f.align_to_face();
        let n = f.plane().unwrap().normal;
        assert!(f.u.axis.dot(n).abs() < 1e-9 && f.v.axis.dot(n).abs() < 1e-9);
        assert!(f.u.axis.cross(f.v.axis).abs_diff_eq(-n, 1e-9));
    }
}
