//! Faces hidden between touching brushes, which engines need not draw.

use std::collections::{BTreeSet, HashMap};

use mapcraft_geom::{Aabb, DVec3, Plane, Winding};

use crate::{FaceId, Map, NodeId, NodeKind};

/// Faces closer than this to an opposite face count as touching it.
const TOUCH: f64 = 0.01;

/// A face's plane rounded so touching opposite faces share a key; a near miss only leaves a face uncovered.
fn key(normal: DVec3, dist: f64) -> [i64; 4] {
    let q = |v: f64, s: f64| (v * s).round() as i64;
    [q(normal.x, 1e4), q(normal.y, 1e4), q(normal.z, 1e4), q(dist, 1.0 / TOUCH)]
}

fn is_tool(material: &str) -> bool {
    material.get(..6).is_some_and(|p| p.eq_ignore_ascii_case("tools/"))
}

/// The part of `face` inside `other`, a convex polygon on the same plane facing the other way.
fn overlap(face: &Winding, other: &Winding, normal: DVec3) -> Option<Winding> {
    let n = other.0.len();
    (0..n).try_fold(face.clone(), |w, i| {
        let (a, b) = (other.0[i], other.0[(i + 1) % n]);
        let out = (b - a).cross(-normal).normalize_or_zero();
        w.clip_back(&Plane { normal: out, dist: out.dot(a) })
    })
}

fn inside(p: DVec3, w: &Winding, normal: DVec3) -> bool {
    let n = w.0.len();
    (0..n).all(|i| (w.0[(i + 1) % n] - w.0[i]).cross(-normal).dot(p - w.0[i]) <= TOUCH)
}

/// Faces of world brushes in `only` (all with `None`) wholly covered by touching faces of other world brushes.
pub fn covered_faces(map: &Map, only: Option<&BTreeSet<NodeId>>) -> BTreeSet<(NodeId, FaceId)> {
    let mut faces: Vec<(NodeId, FaceId, Winding, Plane, Aabb)> = Vec::new();
    for (id, node) in map.nodes() {
        let NodeKind::Brush(b) = &node.kind else { continue };
        // Brush entities may move or be invisible in the game, so they neither cover nor get covered.
        if b.has_displacement() || map.ancestors(id).iter().any(|a| map.get(*a).is_some_and(|n| matches!(n.kind, NodeKind::Entity(_)))) {
            continue;
        }
        let Ok(solid) = b.solid() else { continue };
        for (f, w) in b.faces.iter().zip(solid.faces) {
            if let (Some(w), Some(p), false) = (w, f.plane(), is_tool(&f.material))
                && let Some(bounds) = w.bounds()
            {
                faces.push((id, f.id, w, p, bounds));
            }
        }
    }
    let mut by_plane: HashMap<[i64; 4], Vec<usize>> = HashMap::new();
    for (i, f) in faces.iter().enumerate() {
        by_plane.entry(key(f.3.normal, f.3.dist)).or_default().push(i);
    }
    let mut out = BTreeSet::new();
    for (id, face, w, p, bounds) in faces.iter().filter(|f| only.is_none_or(|o| o.contains(&f.0))) {
        let Some(others) = by_plane.get(&key(-p.normal, -p.dist)) else { continue };
        let near = Aabb::new(bounds.min - TOUCH, bounds.max + TOUCH);
        let covers: Vec<&Winding> = others.iter().filter_map(|&j| faces.get(j)).filter(|o| o.0 != *id && o.4.intersects(&near)).map(|o| &o.2).collect();
        let area: f64 = covers.iter().filter_map(|o| overlap(w, o, p.normal)).map(|o| o.area()).sum();
        if area < w.area() - TOUCH {
            continue;
        }
        // Overlapping covers can add up to the area and still leave a gap, so spot check points too.
        let c = w.center();
        let n = w.0.len();
        let samples = (0..n).flat_map(|i| [w.0[i], (w.0[i] + w.0[(i + 1) % n]) / 2.0]).map(|q| q + (c - q) * 0.01).chain([c]);
        if samples.into_iter().all(|q| covers.iter().any(|o| inside(q, o, p.normal))) {
            out.insert((*id, *face));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use mapcraft_geom::Aabb;

    use super::*;
    use crate::{Brush, ROOT, TexDefaults};

    fn map(boxes: &[(DVec3, DVec3, &str)]) -> (Map, Vec<NodeId>) {
        let mut m = Map::new();
        let mut e = m.edit();
        let ids = boxes
            .iter()
            .map(|(lo, hi, mat)| {
                e.insert(ROOT, NodeKind::Brush(Brush::cuboid(Aabb::new(*lo, *hi), &TexDefaults { material: (*mat).into(), scale: 1.0 }))).unwrap()
            })
            .collect();
        m.apply(e.finish());
        (m, ids)
    }

    fn facing(m: &Map, id: NodeId, normal: DVec3) -> FaceId {
        m.get(id).unwrap().kind.brush().unwrap().faces.iter().find(|f| f.plane().unwrap().normal.distance(normal) < 1e-9).unwrap().id
    }

    #[test]
    fn touching_faces_hide_each_other() {
        let (m, ids) = map(&[(DVec3::ZERO, DVec3::splat(64.0), "a"), (DVec3::new(64.0, 0.0, 0.0), DVec3::new(128.0, 64.0, 64.0), "b")]);
        let covered = covered_faces(&m, None);
        assert_eq!(covered, BTreeSet::from([(ids[0], facing(&m, ids[0], DVec3::X)), (ids[1], facing(&m, ids[1], -DVec3::X))]));
        assert_eq!(covered_faces(&m, Some(&BTreeSet::from([ids[0]]))).len(), 1);
    }

    #[test]
    fn brush_entities_take_no_part() {
        let (mut m, ids) = map(&[(DVec3::ZERO, DVec3::splat(64.0), "a"), (DVec3::new(64.0, 0.0, 0.0), DVec3::new(128.0, 64.0, 64.0), "b")]);
        let mut e = m.edit();
        let zone = e.insert(ROOT, NodeKind::Entity(crate::Entity::new("zone_start"))).unwrap();
        e.reparent(ids[1], zone, None).unwrap();
        m.apply(e.finish());
        assert!(covered_faces(&m, None).is_empty());
    }

    #[test]
    fn partly_covered_tool_and_split_covers() {
        // The small box covers only part of the big one's side; the big one covers all of the small one's.
        let (m, ids) = map(&[(DVec3::ZERO, DVec3::splat(64.0), "a"), (DVec3::new(64.0, 0.0, 0.0), DVec3::new(96.0, 32.0, 32.0), "b")]);
        assert_eq!(covered_faces(&m, None), BTreeSet::from([(ids[1], facing(&m, ids[1], -DVec3::X))]));
        let (m, _) = map(&[(DVec3::ZERO, DVec3::splat(64.0), "a"), (DVec3::new(64.0, 0.0, 0.0), DVec3::new(128.0, 64.0, 64.0), "tools/toolstrigger")]);
        assert!(covered_faces(&m, None).is_empty(), "a trigger hides nothing");
        // Two halves together cover the big side.
        let (m, ids) = map(&[
            (DVec3::ZERO, DVec3::splat(64.0), "a"),
            (DVec3::new(64.0, 0.0, 0.0), DVec3::new(96.0, 32.0, 64.0), "b"),
            (DVec3::new(64.0, 32.0, 0.0), DVec3::new(96.0, 64.0, 64.0), "b"),
        ]);
        assert!(covered_faces(&m, None).contains(&(ids[0], facing(&m, ids[0], DVec3::X))));
    }
}
