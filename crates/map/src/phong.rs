//! Phong shading: the brushes of an entity with `_phong 1` share normals where their faces meet within `_phong_angle` degrees.

use std::collections::HashMap;

use mapcraft_geom::DVec3;

use crate::{Map, NodeId, NodeKind};

/// func_godot's default `_phong_angle`.
const DEFAULT_ANGLE: f64 = 89.0;

#[derive(Default)]
struct Scope {
    cos: f64,
    at: HashMap<[i64; 3], Vec<DVec3>>,
}

/// The smoothed normals of every phong shaded brush in a map.
#[derive(Default)]
pub struct Phong {
    brushes: HashMap<NodeId, usize>,
    scopes: Vec<Scope>,
}

fn key(p: DVec3) -> [i64; 3] {
    (p * 16.0).round().to_array().map(|c| c as i64)
}

impl Phong {
    /// Worldspawn's keys cover the brushes outside entities; an entity's cover its own.
    pub fn of(map: &Map) -> Self {
        let mut out = Self::default();
        for (_, node) in map.nodes() {
            let Some(e) = node.kind.entity().filter(|e| e.get("_phong").map(str::trim) == Some("1")) else { continue };
            let angle = e.get("_phong_angle").and_then(|v| v.trim().parse::<f64>().ok()).filter(|a| a.is_finite()).unwrap_or(DEFAULT_ANGLE);
            let mut scope = Scope { cos: angle.clamp(0.0, 180.0).to_radians().cos() - 1e-9, at: HashMap::new() };
            let mut stack = node.children.clone();
            while let Some(id) = stack.pop() {
                let Some(n) = map.get(id) else { continue };
                match &n.kind {
                    NodeKind::Brush(b) => {
                        let Ok(solid) = b.solid() else { continue };
                        for (face, winding) in b.faces.iter().zip(&solid.faces) {
                            let (Some(w), Some(plane)) = (winding, face.plane()) else { continue };
                            for p in &w.0 {
                                let seen = scope.at.entry(key(*p)).or_default();
                                if !seen.iter().any(|s| s.dot(plane.normal) > 0.999_999) {
                                    seen.push(plane.normal);
                                }
                            }
                        }
                        out.brushes.insert(id, out.scopes.len());
                    }
                    NodeKind::Entity(_) => {}
                    _ => stack.extend(&n.children),
                }
            }
            out.scopes.push(scope);
        }
        out
    }

    pub fn covers(&self, brush: NodeId) -> bool {
        self.brushes.contains_key(&brush)
    }

    /// The normal at corner `p` of a face of `brush` whose plane faces `flat`.
    pub fn normal(&self, brush: NodeId, p: DVec3, flat: DVec3) -> DVec3 {
        let Some(s) = self.brushes.get(&brush).and_then(|i| self.scopes.get(*i)) else { return flat };
        s.at.get(&key(p)).into_iter().flatten().filter(|n| n.dot(flat) >= s.cos).sum::<DVec3>().try_normalize().unwrap_or(flat)
    }
}

/// The normal at `p` on a face with a normal at each corner, fanned from its first corner as its triangles are drawn.
pub fn fan_normal(corners: &[(DVec3, DVec3)], p: DVec3, flat: DVec3) -> DVec3 {
    let Some(((a, na), rest)) = corners.split_first() else { return flat };
    let mut best = (f64::NEG_INFINITY, flat);
    for pair in rest.windows(2) {
        let [(b, nb), (c, nc)] = pair else { continue };
        let (v0, v1, v2) = (*b - *a, *c - *a, p - *a);
        let (d00, d01, d11, d20, d21) = (v0.dot(v0), v0.dot(v1), v1.dot(v1), v2.dot(v0), v2.dot(v1));
        let den = d00 * d11 - d01 * d01;
        if den.abs() < 1e-12 {
            continue;
        }
        let (v, w) = ((d11 * d20 - d01 * d21) / den, (d00 * d21 - d01 * d20) / den);
        let u = 1.0 - v - w;
        let inside = u.min(v).min(w);
        if inside > best.0 {
            best = (inside, (*na * u.max(0.0) + *nb * v.max(0.0) + *nc * w.max(0.0)).normalize_or(flat));
        }
    }
    best.1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Brush, Entity, ROOT, TexDefaults};
    use mapcraft_geom::Aabb;

    #[test]
    fn only_phong_entities_smooth_and_only_within_their_angle() {
        let mut map = Map::new();
        let mut e = map.edit();
        let tex = TexDefaults { material: "a".into(), scale: 1.0 };
        let cube = || NodeKind::Brush(Brush::cuboid(Aabb::new(DVec3::ZERO, DVec3::splat(64.0)), &tex));
        let plain = e.insert(ROOT, cube()).unwrap();
        let mut entity = Entity::new("func_detail");
        entity.set("_phong", "1");
        entity.set("_phong_angle", "95");
        let round = e.insert(ROOT, NodeKind::Entity(entity)).unwrap();
        let brush = e.insert(round, cube()).unwrap();
        map.apply(e.finish());
        let phong = Phong::of(&map);
        assert!(!phong.covers(plain) && phong.covers(brush));
        assert_eq!(phong.normal(plain, DVec3::ZERO, DVec3::NEG_Z), DVec3::NEG_Z);
        let corner = phong.normal(brush, DVec3::ZERO, DVec3::NEG_Z);
        assert!((corner - DVec3::splat(-1.0).normalize()).length() < 1e-9, "a cube's faces meet at 90 degrees, inside 95");
        let corners = [(DVec3::ZERO, DVec3::X), (DVec3::X, DVec3::Y), (DVec3::new(1.0, 1.0, 0.0), DVec3::Y), (DVec3::Y, DVec3::Z)];
        assert_eq!(fan_normal(&corners, DVec3::X, DVec3::Z), DVec3::Y);
        assert_eq!(fan_normal(&corners, DVec3::Y, DVec3::Z), DVec3::Z);
    }
}
