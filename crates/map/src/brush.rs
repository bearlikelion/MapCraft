use std::collections::BTreeMap;

use mapcraft_geom::{Aabb, DAffine2, DMat3, DMat4, DVec2, DVec3, EPSILON, Plane, Side, Solid, SolidError, convex_hull};
use serde::{Deserialize, Serialize};

use crate::disp::{Displacement, ray_triangle};
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disp: Option<Displacement>,
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
        Self {
            id: FaceId(0),
            points,
            material: tex.material.clone(),
            u: axis(u),
            v: axis(v),
            rotation: 0.0,
            attrs: BTreeMap::new(),
            extra: Extra::new(),
            disp: None,
        }
    }

    pub fn plane(&self) -> Option<Plane> {
        Plane::from_points(self.points[0], self.points[1], self.points[2])
    }

    /// Texture coordinates of a point in texels.
    pub fn texel(&self, p: DVec3) -> DVec2 {
        let texel = |a: &TexAxis| if a.scale != 0.0 { p.dot(a.axis) / a.scale + a.offset } else { a.offset };
        DVec2::new(texel(&self.u), texel(&self.v))
    }

    /// Normalized texture coordinates of a point for a texture of `size` texels.
    pub fn uv(&self, p: DVec3, size: DVec2) -> DVec2 {
        self.texel(p) / size.max(DVec2::ONE)
    }

    /// Remaps the texture so every point's texel coordinates become `m` of what they were; false,
    /// leaving the face alone, when that would flatten an axis.
    pub fn transform_texels(&mut self, m: DAffine2) -> bool {
        let grad = |a: &TexAxis| if a.scale != 0.0 { a.axis / a.scale } else { DVec3::ZERO };
        let (gu, gv, ou, ov) = (grad(&self.u), grad(&self.v), self.u.offset, self.v.offset);
        let axis = |row: DVec2, shift: f64| {
            let w = gu * row.x + gv * row.y;
            let len = w.length();
            (len > 1e-12 && len.is_finite()).then(|| TexAxis { axis: w / len, offset: row.x * ou + row.y * ov + shift, scale: 1.0 / len })
        };
        let rows = m.matrix2.transpose();
        let (Some(u), Some(v)) = (axis(rows.x_axis, m.translation.x), axis(rows.y_axis, m.translation.y)) else { return false };
        // The rotation field follows rotate_texture, which turns texel space the other way when u x v points out of the face.
        let flipped = self.plane().is_some_and(|p| self.u.axis.cross(self.v.axis).dot(p.normal) > 0.0);
        let turn = m.matrix2.x_axis.y.atan2(m.matrix2.x_axis.x).to_degrees();
        self.rotation = (self.rotation + if flipped { -turn } else { turn }).rem_euclid(360.0);
        (self.u, self.v) = (u, v);
        true
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
        if let Some(d) = &mut self.disp {
            d.start = m.transform_point3(d.start);
            if mirror {
                *d = d.transposed();
            }
        }
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
    #[error("cannot bevel: {0}")]
    Bevel(&'static str),
}

/// Blender's bevel: the faces beside an edge are cut back `width` and `segments` faces span the
/// gap along a curve set by `profile` (0.5 round, 0.25 or less a flat chamfer, toward 1 boxier).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bevel {
    pub width: f64,
    pub segments: usize,
    pub profile: f64,
}

impl Default for Bevel {
    fn default() -> Self {
        Self { width: 16.0, segments: 1, profile: 0.5 }
    }
}

/// A bevel strip face: `src`'s material and attributes, its texture turned on from `prev`.
fn strip_face(src: &Face, prev: &Face, points: [DVec3; 3]) -> Face {
    let mut f = Face { id: FaceId(0), points, disp: None, ..src.clone() };
    f.wrap_from(prev);
    f
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

    /// The brush with face `id` moved `distance` along its outward normal, or `None` when that
    /// would add or remove faces or leave no volume (TrenchBroom's extrude rules).
    pub fn extrude_face(&self, id: FaceId, distance: f64) -> Option<Self> {
        let mut b = self.clone();
        let face = b.faces.iter_mut().find(|f| f.id == id)?;
        let n = face.plane()?.normal;
        face.points = face.points.map(|p| p + n * distance);
        let solid = b.solid().ok()?;
        (solid.faces.iter().all(Option::is_some) && solid.volume() > EPSILON).then_some(b)
    }

    /// Face `id` pushed `distance` along its normal keeping its shape: outward the brush gains the
    /// volume the face sweeps through, with new sides square to it; inward it is cut at the moved face.
    pub fn sweep_face(&self, id: FaceId, distance: f64) -> Option<Self> {
        if distance <= 0.0 {
            return self.extrude_face(id, distance);
        }
        let solid = self.solid().ok()?;
        let i = self.faces.iter().position(|f| f.id == id)?;
        let face = self.faces.get(i)?;
        let n = face.plane()?.normal;
        let mut pts: Vec<DVec3> = solid.vertices().collect();
        pts.extend(solid.faces.get(i)?.as_ref()?.0.iter().map(|p| *p + n * distance));
        let tex = TexDefaults { material: face.material.clone(), scale: face.u.scale };
        let mut b = Self::from_vertices(&pts, &self.faces, &tex).ok()?;
        let moved = b.faces.iter_mut().find(|f| f.plane().is_some_and(|p| p.normal.dot(n) > 1.0 - 1e-9))?;
        *moved = Face { points: moved.points, ..face.clone() };
        let solid = b.solid().ok()?;
        (solid.volume() > EPSILON).then_some(b)
    }

    /// A new brush between face `id` and a copy of it moved `distance` along its normal
    /// (TrenchBroom's stamp); faces keep the textures of the planes they share with this brush.
    pub fn stamp_face(&self, id: FaceId, distance: f64) -> Option<Self> {
        let solid = self.solid().ok()?;
        let i = self.faces.iter().position(|f| f.id == id)?;
        let face = self.faces.get(i)?;
        let n = face.plane()?.normal;
        let poly = solid.faces.get(i)?.as_ref()?;
        let pts: Vec<DVec3> = poly.0.iter().flat_map(|p| [*p, *p + n * distance]).collect();
        let tex = TexDefaults { material: face.material.clone(), scale: face.u.scale };
        Self::from_vertices(&pts, &self.faces, &tex).ok().map(Self::with_new_ids)
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

    /// The box around the brush, including any displaced surface raised out of it.
    pub fn bounds(&self) -> Result<Aabb, BrushError> {
        let solid = self.solid()?;
        let raised = self.displaced(&solid).into_iter().flat_map(|(_, v)| v);
        Ok(Aabb::from_points(raised).map_or(solid.bounds, |b| b.union(&solid.bounds)))
    }

    /// Whether a face is displaced; the brush then counts only as its displaced surfaces.
    pub fn has_displacement(&self) -> bool {
        self.faces.iter().any(|f| f.disp.is_some())
    }

    /// The displaced grid of each face whose displacement fits it, by face index.
    pub fn displaced(&self, solid: &Solid) -> Vec<(usize, Vec<DVec3>)> {
        self.faces
            .iter()
            .zip(&solid.faces)
            .enumerate()
            .filter_map(|(i, (f, w))| Some((i, f.disp.as_ref()?.vertices(&w.as_ref()?.0, f.plane()?.normal)?)))
            .collect()
    }

    /// The nearest face the ray hits and the distance to it; a displaced brush is hit on its
    /// displaced surfaces, or on its volume when the ray misses them, as an edge-on view does.
    pub fn ray_hit(&self, origin: DVec3, dir: DVec3) -> Option<(f64, usize)> {
        let volume = || mapcraft_geom::ray_hit(&self.planes().ok()?, origin, dir);
        if !self.has_displacement() {
            return volume();
        }
        let solid = self.solid().ok()?;
        let mut best: Option<(f64, usize)> = None;
        for (i, v) in self.displaced(&solid) {
            let Some(d) = self.faces.get(i).and_then(|f| f.disp.as_ref()) else { continue };
            for [a, b, c] in d.triangles() {
                if let Some(t) = ray_triangle(origin, dir, [v[a], v[b], v[c]])
                    && best.is_none_or(|(bt, _)| t < bt)
                {
                    best = Some((t, i));
                }
            }
        }
        best.or_else(volume)
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
            if let Some(d) = &mut f.disp {
                d.start += t;
            }
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

    pub fn with_new_ids(mut self) -> Self {
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

    /// The brush with each edge `(a, b)` beveled; refused when one is not an edge between two
    /// faces, the cut is wider than a face beside it, or two edges' bevels overlap.
    pub fn bevel(&self, edges: &[(DVec3, DVec3)], bevel: &Bevel) -> Result<Self, BrushError> {
        let solid = self.solid()?;
        let w = bevel.width;
        if !(w.is_finite() && w > EPSILON) {
            return Err(BrushError::Bevel("the width must be above zero"));
        }
        let n = bevel.segments.clamp(1, 64);
        // Superellipse exponent: 2 is a circle; under 1 would dent the brush, so it stops at a flat chamfer.
        let r = 2f64.powf(4.0 * bevel.profile.clamp(0.0, 1.0) - 1.0).max(1.0);
        let near = |p: DVec3, q: DVec3| p.distance_squared(q) < 1e-6;
        let mut faces = self.faces.clone();
        let mut chamfers: Vec<Plane> = Vec::new();
        let mut done: Vec<(DVec3, DVec3)> = Vec::new();
        for &(a, b) in edges {
            if done.iter().any(|&(p, q)| (near(p, a) && near(q, b)) || (near(p, b) && near(q, a))) {
                continue;
            }
            done.push((a, b));
            let sides: Vec<usize> = (0..self.faces.len())
                .filter(|i| solid.faces.get(*i).and_then(Option::as_ref).is_some_and(|w| w.0.iter().any(|p| near(*p, a)) && w.0.iter().any(|p| near(*p, b))))
                .collect();
            let not_edge = BrushError::Bevel("pick edges between two faces of the brush");
            let &[i, j] = sides.as_slice() else { return Err(not_edge) };
            let (Some(fa), Some(fb), Some(Some(wa)), Some(Some(wb))) = (self.faces.get(i), self.faces.get(j), solid.faces.get(i), solid.faces.get(j)) else {
                return Err(not_edge);
            };
            let (Some(na), Some(nb)) = (fa.plane().map(|p| p.normal), fb.plane().map(|p| p.normal)) else { return Err(not_edge) };
            let e = (b - a).normalize_or_zero();
            // Along each face, away from the edge.
            let into = |n: DVec3, other: DVec3| {
                let d = e.cross(n);
                if d.dot(other) > 0.0 { -d } else { d }
            };
            let (da, db) = (into(na, nb), into(nb, na));
            let reach = |w: &mapcraft_geom::Winding, d: DVec3| w.0.iter().map(|p| (*p - a).dot(d)).fold(0.0, f64::max);
            if w > reach(wa, da) + EPSILON || w > reach(wb, db) + EPSILON {
                return Err(BrushError::Bevel("the width is more than a face beside the edge"));
            }
            let profile: Vec<DVec3> = (0..=n)
                .map(|k| {
                    let t = std::f64::consts::FRAC_PI_2 * k as f64 / n as f64;
                    let (x, y) = (t.cos().max(0.0).powf(2.0 / r), t.sin().max(0.0).powf(2.0 / r));
                    a + da * w * (1.0 - y) + db * w * (1.0 - x)
                })
                .collect();
            let outward = |p: [DVec3; 3]| match Plane::from_points(p[0], p[1], p[2]) {
                Some(plane) if plane.normal.dot(na + nb) < 0.0 => [p[2], p[1], p[0]],
                _ => p,
            };
            let strip: Vec<Option<[DVec3; 3]>> =
                profile.windows(2).map(|q| (q[0].distance(q[1]) > EPSILON).then(|| outward([q[0] + (b - a), q[0], q[1]]))).collect();
            let (Some(&pa), Some(&pb)) = (profile.first(), profile.last()) else { return Err(not_edge) };
            let chamfer = outward([pa + (b - a), pa, pb]);
            chamfers.extend(Plane::from_points(chamfer[0], chamfer[1], chamfer[2]));
            // Each half's texture runs on from the face it starts at.
            let (first, second) = strip.split_at(n.div_ceil(2).min(strip.len()));
            let from_a: Vec<[DVec3; 3]> = first.iter().flatten().copied().collect();
            let from_b: Vec<[DVec3; 3]> = second.iter().rev().flatten().copied().collect();
            for (src, part) in [(fa, from_a), (fb, from_b)] {
                let mut prev = src.clone();
                for p in part {
                    let f = strip_face(src, &prev, p);
                    faces.push(f.clone());
                    prev = f;
                }
            }
        }
        let planes = self.planes()?;
        for (k, c) in chamfers.iter().enumerate() {
            for d in chamfers.iter().skip(k + 1) {
                let mut both = planes.clone();
                both.extend([c.flipped(), d.flipped()]);
                if Solid::from_planes(&both).is_ok() {
                    return Err(BrushError::Bevel("the bevels of two edges overlap"));
                }
            }
        }
        Self::pruned(faces)
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
    fn extrude_and_stamp() {
        let mut b = cube(0.0, 64.0);
        for (i, f) in b.faces.iter_mut().enumerate() {
            f.id = FaceId(i as u64 + 1);
        }
        let top = b.faces.iter().find(|f| f.plane().unwrap().normal.z > 0.99).unwrap().id;
        let taller = b.extrude_face(top, 32.0).unwrap();
        assert_eq!(taller.bounds().unwrap().max.z, 96.0);
        assert!(b.extrude_face(top, -64.0).is_none(), "pushing a face through the brush is refused");
        let mut w = Brush::wedge(Aabb::new(DVec3::ZERO, DVec3::splat(64.0)), &TexDefaults::default()).unwrap();
        for (i, f) in w.faces.iter_mut().enumerate() {
            f.id = FaceId(i as u64 + 1);
        }
        let bottom = w.faces.iter().find(|f| f.plane().unwrap().normal.z < -0.99).unwrap().id;
        assert!(w.extrude_face(bottom, 16.0).is_some());
        let stamp = b.stamp_face(top, 16.0).unwrap();
        assert_eq!(stamp.bounds().unwrap(), Aabb::new(DVec3::new(0.0, 0.0, 64.0), DVec3::new(64.0, 64.0, 80.0)));
    }

    /// TrenchBroom's face drag: the sloped face of a wedge moves out keeping its shape, and the
    /// wedge grows by the slab it sweeps instead of stretching its other faces.
    #[test]
    fn sweep_keeps_the_face_shape() {
        let mut w = Brush::wedge(Aabb::new(DVec3::ZERO, DVec3::splat(64.0)), &TexDefaults::default()).unwrap();
        for (i, f) in w.faces.iter_mut().enumerate() {
            f.id = FaceId(i as u64 + 1);
            f.u.offset = 7.0;
        }
        let slope = w.faces.iter().find(|f| f.plane().is_some_and(|p| p.normal.x > 0.1 && p.normal.z > 0.1)).unwrap().clone();
        let area = 64.0 * 64.0 * 2f64.sqrt();
        let swept = w.sweep_face(slope.id, 16.0).unwrap();
        assert_eq!(swept.vertices().unwrap().len(), 10, "a wedge with a slab on its slope has five sides");
        let gain = swept.solid().unwrap().volume() - w.solid().unwrap().volume();
        assert!((gain - area * 16.0).abs() < 1e-6, "it grows by the face's area times the distance");
        let moved = swept.faces.iter().find(|f| f.id == slope.id).unwrap();
        assert_eq!((moved.u.offset, moved.material.as_str()), (7.0, slope.material.as_str()), "the moved face keeps its texture");
        assert_eq!(w.sweep_face(slope.id, -8.0), w.extrude_face(slope.id, -8.0), "inward it cuts like a plain face drag");
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

    fn numbered(mut b: Brush) -> Brush {
        for (i, f) in b.faces.iter_mut().enumerate() {
            f.id = FaceId(i as u64 + 1);
        }
        b
    }

    const TOP_FRONT: (DVec3, DVec3) = (DVec3::new(0.0, 64.0, 64.0), DVec3::new(64.0, 64.0, 64.0));

    #[test]
    fn bevel_chamfer_round_and_profiles() {
        let b = numbered(cube(0.0, 64.0));
        let full = 64.0f64.powi(3);
        let chamfer = b.bevel(&[TOP_FRONT], &Bevel { width: 16.0, segments: 1, profile: 0.5 }).unwrap();
        assert_eq!(chamfer.faces.len(), 7);
        assert!((volume(std::slice::from_ref(&chamfer)) - (full - 0.5 * 16.0 * 16.0 * 64.0)).abs() < 1e-6);

        let round = b.bevel(&[TOP_FRONT], &Bevel { width: 16.0, segments: 4, profile: 0.5 }).unwrap();
        assert_eq!(round.faces.len(), 10);
        assert_eq!(round.faces.iter().filter(|f| f.id != FaceId(0)).count(), 6, "the cut back faces keep their ids");
        let s = round.solid().unwrap();
        for v in s.vertices().filter(|v| v.y > 48.0 + 1e-6 && v.z > 48.0 + 1e-6) {
            assert!(((v.y - 48.0).hypot(v.z - 48.0) - 16.0).abs() < 1e-6, "{v:?} is off the round");
        }
        let closure: DVec3 = s.faces.iter().flatten().map(|w| w.area_vector()).sum();
        assert!(closure.length() < 1e-6);
        let planes = round.planes().unwrap();
        assert!(s.vertices().all(|p| planes.iter().all(|q| q.distance(p) < 1e-6)), "convex");
        let rounded = volume(std::slice::from_ref(&round));
        assert!(rounded > volume(&[chamfer]) && rounded < full);

        let flat = b.bevel(&[TOP_FRONT], &Bevel { width: 16.0, segments: 4, profile: 0.0 }).unwrap();
        assert_eq!(flat.faces.len(), 7, "a profile at 0 collapses to a chamfer");
        let boxy = b.bevel(&[TOP_FRONT], &Bevel { width: 16.0, segments: 4, profile: 1.0 }).unwrap();
        assert!(volume(&[boxy]) > rounded);
    }

    #[test]
    fn bevel_pairs_and_refusals() {
        let b = cube(0.0, 64.0);
        let bev = Bevel { width: 16.0, segments: 2, profile: 0.5 };
        let top_back = (DVec3::new(0.0, 0.0, 64.0), DVec3::new(64.0, 0.0, 64.0));
        let top_side = (DVec3::new(64.0, 0.0, 64.0), DVec3::new(64.0, 64.0, 64.0));
        assert!(b.bevel(&[top_back, TOP_FRONT], &bev).is_ok());
        let meet = b.bevel(&[top_back, TOP_FRONT], &Bevel { width: 32.0, ..bev }).unwrap();
        assert!(meet.faces.iter().all(|f| f.plane().unwrap().normal.z < 0.99), "the top is rounded away");
        assert!(matches!(b.bevel(&[TOP_FRONT, top_side], &bev), Err(BrushError::Bevel(_))), "bevels sharing a corner overlap");
        assert!(b.bevel(&[TOP_FRONT], &Bevel { width: 80.0, ..bev }).is_err());
        assert!(b.bevel(&[TOP_FRONT], &Bevel { width: 0.0, ..bev }).is_err());
        assert!(b.bevel(&[(DVec3::new(0.0, 0.0, 64.0), DVec3::splat(64.0))], &bev).is_err(), "a diagonal is not an edge");
    }

    #[test]
    fn bevel_textures_run_on() {
        let mut b = numbered(cube(0.0, 64.0));
        b.faces[4].material = "top".into();
        b.faces[4].rotate_texture(30.0);
        b.faces[4].u.offset = 5.0;
        b.faces[2].material = "side".into();
        let round = b.bevel(&[TOP_FRONT], &Bevel { width: 16.0, segments: 4, profile: 0.5 }).unwrap();
        assert!(round.faces.iter().filter(|f| f.id == FaceId(0)).all(|f| f.material == "top" || f.material == "side"));
        let s = round.solid().unwrap();
        let size = DVec2::splat(64.0);
        let mut seams = 0;
        for (i, (f, w)) in round.faces.iter().zip(&s.faces).enumerate() {
            for (g, x) in round.faces.iter().zip(&s.faces).skip(i + 1) {
                let (Some(w), Some(x)) = (w, x) else { continue };
                let shared: Vec<DVec3> = w.0.iter().filter(|p| x.0.iter().any(|q| q.distance(**p) < 1e-6)).copied().collect();
                if shared.len() < 2 || f.material != g.material || (f.id != FaceId(0) && g.id != FaceId(0)) {
                    continue;
                }
                for p in &shared {
                    assert!(f.uv(*p, size).abs_diff_eq(g.uv(*p, size), 1e-9), "{} and {} part at {p:?}", f.material, g.material);
                }
                seams += 1;
            }
        }
        assert_eq!(seams, 4);
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
    fn texel_transforms_round_trip() {
        let b = cube(0.0, 64.0);
        let solid = b.solid().unwrap();
        let m = DAffine2::from_cols_array(&[1.3, 0.4, -0.2, 0.7, 9.0, -4.5]);
        for i in [1, 4] {
            let poly = solid.faces[i].as_ref().unwrap().0.clone();
            let mut f = b.faces[i].clone();
            (f.u.offset, f.v.offset, f.v.scale) = (5.0, -3.0, 0.5);
            f.rotate_texture(20.0);
            let before: Vec<DVec2> = poly.iter().map(|p| f.texel(*p)).collect();
            let mut g = f.clone();
            assert!(g.transform_texels(m));
            for (p, t) in poly.iter().zip(&before) {
                assert!(g.texel(*p).abs_diff_eq(m.transform_point2(*t), 1e-9), "{:?} {:?}", g.texel(*p), m.transform_point2(*t));
            }
            assert!(!g.transform_texels(DAffine2::from_scale(DVec2::new(1.0, 0.0))), "flattening an axis is refused");

            let mut turned = b.faces[i].clone();
            (turned.u.offset, turned.v.offset) = (5.0, -3.0);
            let mut same = turned.clone();
            turned.rotate_texture(30.0);
            let o = DVec2::new(5.0, -3.0);
            let n = b.faces[i].plane().unwrap().normal;
            let sign = if b.faces[i].u.axis.cross(b.faces[i].v.axis).dot(n) > 0.0 { -1.0 } else { 1.0 };
            same.transform_texels(DAffine2::from_translation(o) * DAffine2::from_angle(sign * 30f64.to_radians()) * DAffine2::from_translation(-o));
            for p in &poly {
                assert!(same.texel(*p).abs_diff_eq(turned.texel(*p), 1e-9));
            }
            assert!((same.rotation - turned.rotation).abs() < 1e-9, "face {i}: {} {}", same.rotation, turned.rotation);
        }
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
