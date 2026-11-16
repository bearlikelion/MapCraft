//! Grows convex polygons from neighboring triangles that share a plane, a material and a texture mapping, so a zone needs fewer brushes.

use std::collections::HashMap;

use glam::{DVec2, DVec3};

use crate::mapcraft::plugin::types::TexAxis;

/// Neighbors this far off the plane, in units, or this far off the mapping, in texels, stay apart.
const PLANE: f64 = 0.01;
const TEXEL: f64 = 1.0;
const NORMAL: f64 = 0.99999;
/// Corners within this of each other are the same corner.
const WELD: f64 = 1.0 / 64.0;
/// Turns sharper than this count as corners; flatter ones are dropped from the polygon.
const STRAIGHT: f64 = 1e-6;
/// Sides of one polygon, so a brush stays a reasonable size to edit.
const MAX_SIDES: usize = 16;

/// A triangle counter-clockwise about its normal, with its picture's texel coordinates.
pub struct Tri<K> {
    pub points: [DVec3; 3],
    pub normal: DVec3,
    pub material: String,
    pub kind: K,
    pub uv: Option<(TexAxis, TexAxis)>,
    pub texels: [DVec2; 3],
    pub size: DVec2,
}

/// A convex polygon counter-clockwise about `normal`, drawn with `seed`'s material and mapping.
pub struct Polygon {
    pub points: Vec<DVec3>,
    pub normal: DVec3,
    pub seed: usize,
}

type Key = [i64; 3];

fn key(p: DVec3) -> Key {
    (p / WELD).round().as_i64vec3().to_array()
}

fn texel(uv: &(TexAxis, TexAxis), p: DVec3) -> DVec2 {
    let at = |a: &TexAxis| p.dot(DVec3::new(a.axis.x, a.axis.y, a.axis.z)) / a.scale + a.offset;
    DVec2::new(at(&uv.0), at(&uv.1))
}

/// Whether `t` draws exactly as it would with `seed`'s plane, material and mapping, allowing whole repeats of the picture.
fn fits<K: PartialEq>(seed: &Tri<K>, t: &Tri<K>) -> bool {
    if seed.material != t.material || seed.kind != t.kind || seed.normal.dot(t.normal) < NORMAL {
        return false;
    }
    if t.points.iter().any(|p| (*p - seed.points[0]).dot(seed.normal).abs() > PLANE) {
        return false;
    }
    match (&seed.uv, &t.uv) {
        (None, None) => true,
        (Some(uv), Some(_)) => {
            let shift = texel(uv, t.points[0]) - t.texels[0];
            let whole = shift / seed.size.max(DVec2::ONE);
            let repeats = (whole - whole.round()) * seed.size;
            repeats.abs().max_element() < TEXEL && t.points.iter().zip(&t.texels).all(|(p, uv0)| (texel(uv, *p) - uv0 - shift).abs().max_element() < TEXEL)
        }
        _ => false,
    }
}

/// `points` without corners that do not turn, or `None` when it is not convex.
fn convex(points: &[DVec3], normal: DVec3) -> Option<Vec<DVec3>> {
    let n = points.len();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let (a, b, c) = (points[(i + n - 1) % n], points[i], points[(i + 1) % n]);
        let turn = (b - a).cross(c - b).dot(normal);
        if turn < -STRAIGHT * (b - a).length() * (c - b).length() {
            return None;
        }
        if turn > STRAIGHT * (b - a).length() * (c - b).length() {
            out.push(b);
        }
    }
    (out.len() >= 3).then_some(out)
}

pub fn merge<K: PartialEq>(tris: &[Tri<K>]) -> Vec<Polygon> {
    let mut edges: HashMap<(Key, Key), Vec<usize>> = HashMap::new();
    for (i, t) in tris.iter().enumerate() {
        for k in 0..3 {
            edges.entry((key(t.points[k]), key(t.points[(k + 1) % 3]))).or_default().push(i);
        }
    }
    let mut used = vec![false; tris.len()];
    let mut out = Vec::new();
    for (seed, s) in tris.iter().enumerate() {
        if used[seed] {
            continue;
        }
        used[seed] = true;
        // Keeps straight corners too, since neighbors attach along the triangles' own edges.
        let mut ring: Vec<DVec3> = s.points.to_vec();
        let mut grew = true;
        while grew && ring.len() < MAX_SIDES * 2 {
            grew = false;
            for e in 0..ring.len() {
                let (u, v) = (ring[e], ring[(e + 1) % ring.len()]);
                let Some(list) = edges.get(&(key(v), key(u))) else { continue };
                let found = list.iter().copied().find_map(|t| {
                    let tri = tris.get(t).filter(|tri| !used[t] && fits(s, tri))?;
                    let w = tri.points.iter().copied().find(|p| key(*p) != key(u) && key(*p) != key(v))?;
                    let mut next = ring.clone();
                    next.insert(e + 1, w);
                    convex(&next, s.normal).filter(|c| c.len() <= MAX_SIDES).map(|_| (t, next))
                });
                if let Some((t, next)) = found {
                    used[t] = true;
                    ring = next;
                    grew = true;
                    break;
                }
            }
        }
        let points = convex(&ring, s.normal).unwrap_or_else(|| s.points.to_vec());
        out.push(Polygon { points, normal: s.normal, seed });
    }
    out
}
