//! Brushes rebuilt from their planes, the model that owns each, and the 3D skybox, as in SourceGD's `world`.

use glam::{DMat3, DVec3};

use crate::bsp::{Bsp, Leaf, NODRAW, Texinfo, Tree, cluster_pvs};
use crate::byte_reader::index;
use crate::mapcraft::plugin::types::{Brush, Face, TexAxis, Vec3};

/// Bevels take curved brushes to about 500 sides; far more means a corrupt file, and clipping is quadratic.
pub(crate) const MAX_SIDES: usize = 2048;
/// Faces thinner than this come from planes that only touch the brush at an edge or corner.
pub(crate) const MIN_WIDTH: f64 = 0.01;
pub(crate) const CLIP_EPSILON: f64 = 1e-6;
/// Source coordinates stay within 16384, so a face reaching this far belongs to an open brush.
pub(crate) const OPEN: f64 = 32768.0;

/// Source's steepest standable slope: a floor normal's Z is at least this.
pub(crate) const STANDABLE: f64 = 0.7;
/// Faces closer to vertical than this (about 84 degrees) are walls, not ramps.
pub(crate) const WALL: f64 = 0.1;
/// Surf faces smaller than this (square units) are trim, not ramps.
pub(crate) const MIN_RAMP_AREA: f64 = 64.0 * 64.0;
/// Thickest wall around the 3D skybox's leaves that still counts as part of it.
const SKYBOX_WALL: f64 = 256.0;
/// Brushes with these contents are ladders, which SourceGD keeps in their own collision body.
pub(crate) const CONTENTS_LADDER: i32 = 0x2000_0000;

pub(crate) fn v3(p: DVec3) -> Vec3 {
    Vec3 { x: p.x, y: p.y, z: p.z }
}

/// A unit vector on the plane, as VBSP's BaseWindingForPlane picks it.
pub(crate) fn tangent(normal: DVec3) -> DVec3 {
    let a = normal.abs();
    let up = if a.z > a.x && a.z > a.y { DVec3::X } else { DVec3::Z };
    (up - normal * up.dot(normal)).normalize_or_zero()
}

/// A huge square on the plane, clockwise seen from the front.
pub(crate) fn base_winding(normal: DVec3, dist: f64) -> Vec<DVec3> {
    let up = tangent(normal) * 65536.0;
    let right = up.cross(normal);
    let origin = normal * dist;
    vec![origin - right + up, origin + right + up, origin + right - up, origin - right - up]
}

/// The part of the winding behind the plane.
pub(crate) fn chop(winding: &[DVec3], normal: DVec3, dist: f64) -> Vec<DVec3> {
    let mut out = Vec::with_capacity(winding.len() + 1);
    for (&a, &b) in winding.iter().zip(winding.iter().cycle().skip(1)) {
        let (da, db) = (normal.dot(a) - dist, normal.dot(b) - dist);
        if da <= CLIP_EPSILON {
            out.push(a);
        }
        if (da > CLIP_EPSILON && db < -CLIP_EPSILON) || (da < -CLIP_EPSILON && db > CLIP_EPSILON) {
            out.push(a + (b - a) * (da / (da - db)));
        }
    }
    out
}

pub(crate) fn area(winding: &[DVec3]) -> f64 {
    let Some(&first) = winding.first() else { return 0.0 };
    winding.iter().zip(winding.iter().cycle().skip(1)).map(|(&a, &b)| (a - first).cross(b - first)).sum::<DVec3>().length() / 2.0
}

/// Twice the area over the perimeter: about the width of a long sliver.
pub(crate) fn width(winding: &[DVec3]) -> f64 {
    let perimeter: f64 = winding.iter().zip(winding.iter().cycle().skip(1)).map(|(&a, &b)| a.distance(b)).sum();
    if perimeter > 0.0 { 2.0 * area(winding) / perimeter } else { 0.0 }
}

/// A texture axis in map space; `v` holds texels per unit and `w` the offset in texels.
pub(crate) fn tex_axis(rotation: DMat3, origin: DVec3, (v, w): (DVec3, f64)) -> Option<TexAxis> {
    let a = rotation * v;
    let len = a.length();
    (len > 1e-9).then(|| TexAxis { axis: v3(a / len), offset: w - origin.dot(a), scale: 1.0 / len })
}

/// The convex polygon each plane contributes to the brush they bound; empty when it clips away.
pub(crate) fn windings(planes: &[(DVec3, f64)]) -> Vec<Vec<DVec3>> {
    planes
        .iter()
        .enumerate()
        .map(|(i, &(normal, dist))| {
            let mut w = base_winding(normal, dist);
            for (j, &(n, d)) in planes.iter().enumerate() {
                if j != i && w.len() >= 3 {
                    w = chop(&w, n, d);
                }
            }
            if w.len() >= 3 && width(&w) >= MIN_WIDTH { w } else { Vec::new() }
        })
        .collect()
}

pub(crate) struct Map {
    pub(crate) planes: Vec<(DVec3, f64)>,
    pub(crate) texinfos: Vec<Texinfo>,
    /// First side and side count of each brush.
    pub(crate) brushes: Vec<(usize, usize)>,
    /// Plane, texinfo and bevel flag of each brush side.
    pub(crate) sides: Vec<(usize, i16, bool)>,
}

impl Map {
    /// The brush in map space, whether it has a surfable face, and its bounds; `None` when it has fewer than four faces left or is open.
    pub(crate) fn brush(&self, index: usize, rotation: DMat3, origin: DVec3) -> Option<(Brush, bool, Bounds)> {
        let &(first, count) = self.brushes.get(index)?;
        if count > MAX_SIDES {
            return None;
        }
        let all = self.sides.get(first..first.checked_add(count)?)?;
        // A bevel now and then stands in for a real face, so bevels come back when the brush is open without them.
        let (planes, sides, polygons) = [false, true]
            .into_iter()
            .map(|bevels| {
                let mut planes = Vec::with_capacity(count);
                let mut sides: Vec<(usize, i16)> = Vec::with_capacity(count);
                for &(plane, texinfo, bevel) in all {
                    if let Some(&p) = self.planes.get(plane).filter(|_| (bevels || !bevel) && !sides.iter().any(|s| s.0 == plane)) {
                        planes.push(p);
                        sides.push((plane, texinfo));
                    }
                }
                let w = windings(&planes);
                (planes, sides, w)
            })
            .find(|(_, _, w)| w.iter().flatten().all(|p| p.abs().max_element() < OPEN))?;
        let mut surf = false;
        let corners = polygons.iter().flatten().map(|&p| rotation * p + origin);
        let bounds = corners.fold((DVec3::MAX, DVec3::MIN), |(lo, hi), p| (lo.min(p), hi.max(p)));
        let faces: Vec<Face> = polygons
            .iter()
            .zip(&planes)
            .zip(&sides)
            .filter(|((w, _), _)| !w.is_empty())
            .map(|((w, &(normal, dist)), &(_, texinfo))| {
                let z = (rotation * normal).z;
                surf |= z > WALL && z < STANDABLE && area(w) >= MIN_RAMP_AREA;
                let center = w.iter().sum::<DVec3>() / w.len() as f64;
                let center = center - normal * (normal.dot(center) - dist);
                let up = tangent(normal);
                let right = up.cross(normal);
                let place = |p: DVec3| v3(rotation * p + origin);
                let info = usize::try_from(texinfo).ok().and_then(|t| self.texinfos.get(t));
                Face {
                    points: (place(center + right * 64.0), place(center), place(center + up * 64.0)),
                    material: info.map_or(NODRAW, |i| i.material.as_str()).to_string(),
                    uv: info.and_then(|i| tex_axis(rotation, origin, i.s).zip(tex_axis(rotation, origin, i.t))),
                    attrs: Vec::new(),
                    disp: None,
                }
            })
            .collect();
        (faces.len() >= 4).then_some((Brush::Faces(faces), surf, bounds))
    }
}

/// Lowest and highest corner.
pub(crate) type Bounds = (DVec3, DVec3);

/// The model each brush belongs to, found by walking every model's node tree down to its leaves.
pub(crate) fn owners(tree: &Tree, brushes: usize) -> Vec<Option<usize>> {
    let mut owner = vec![None; brushes];
    let mut visited = vec![usize::MAX; tree.nodes.len()];
    let mut stack = Vec::new();
    for (model, &head) in tree.heads.iter().enumerate() {
        stack.push(head);
        while let Some(node) = stack.pop() {
            if node < 0 {
                let Some(leaf) = tree.leafs.get(index(-(node + 1))) else { continue };
                let (first, count) = leaf.brushes;
                for &b in tree.leafbrushes.get(first..first + count).unwrap_or_default() {
                    if let Some(o) = owner.get_mut(b) {
                        *o = Some(model);
                    }
                }
            } else if let Some(seen) = visited.get_mut(index(node)).filter(|seen| **seen != model) {
                *seen = model;
                stack.extend(tree.nodes.get(index(node)).into_iter().flat_map(|n| n.1));
            }
        }
    }
    owner
}

/// Bounds of the leaves the 3D skybox camera at `camera` sees, as SourceGD's `set_sky_camera` picks them: its PVS, or its area on a map without vis.
pub(crate) fn skybox_bounds(bsp: &Bsp, tree: &Tree, planes: &[(DVec3, f64)], camera: DVec3) -> Option<Bounds> {
    let leaf = tree.leafs.get(tree.point_leaf(planes, camera)?)?;
    let pvs = cluster_pvs(bsp, leaf.cluster);
    let seen = |l: &Leaf| match usize::try_from(l.cluster) {
        _ if pvs.is_empty() => leaf.area > 0 && l.area == leaf.area,
        Ok(c) => pvs.get(c / 8).is_some_and(|b| b & (1 << (c % 8)) != 0),
        Err(_) => false,
    };
    tree.leafs.iter().filter(|l| seen(l)).map(|l| (l.mins, l.maxs)).reduce(|(lo, hi), (a, b)| (lo.min(a), hi.max(b)))
}

/// Whether `inner` lies within `outer`, allowing for the walls that seal it.
pub(crate) fn within(outer: Bounds, inner: Bounds) -> bool {
    let (lo, hi) = (outer.0 - SKYBOX_WALL, outer.1 + SKYBOX_WALL);
    inner.0.cmpge(lo).all() && inner.1.cmple(hi).all()
}
