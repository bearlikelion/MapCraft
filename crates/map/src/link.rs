//! Linked copies: groups drawn as another group's contents, so editing the original changes every copy.

use std::collections::{BTreeMap, BTreeSet};

use mapcraft_geom::{DMat4, DVec3};
use serde::{Deserialize, Serialize};

use crate::entity::parse_vec3;
use crate::map::MAX_DEPTH;
use crate::mirror::{Mirrored, insert};
use crate::{Group, Map, NodeId, NodeKind};

/// A group's part in linked copies: with a `source` a copy of its contents, else an original whose frame moves with it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Link {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<NodeId>,
    pub frame: DMat4,
}

impl Link {
    pub fn transformed(&self, m: &DMat4) -> Self {
        Self { frame: *m * self.frame, ..*self }
    }
}

fn fmt_vec(v: DVec3) -> String {
    let v = v + 0.0;
    format!("{} {} {}", v.x, v.y, v.z)
}

/// Pitch, yaw and roll after `m`; only turns about Z carry over, as for prefabs and generators.
fn turned(m: &DMat4, mut angles: DVec3) -> DVec3 {
    let x = m.x_axis.truncate();
    if m.z_axis.truncate().normalize_or_zero().abs_diff_eq(DVec3::Z, 1e-9) {
        angles.y = (angles.y + x.y.atan2(x.x).to_degrees()).rem_euclid(360.0);
    }
    angles
}

/// `kind` moved by `m`, textures with it; `None` for what cannot be moved, such as a flattened brush.
pub fn transform(kind: &NodeKind, m: &DMat4, point: bool) -> Option<NodeKind> {
    Some(match kind {
        NodeKind::Brush(b) => NodeKind::Brush(b.transformed(m, true).ok()?),
        NodeKind::Entity(e) if point => {
            let mut e = e.clone();
            if let Some(o) = e.origin() {
                e.set("origin", fmt_vec(m.transform_point3(o)));
            }
            if let Some(a) = e.get("angles").and_then(parse_vec3) {
                e.set("angles", fmt_vec(turned(m, a)));
            }
            NodeKind::Entity(e)
        }
        NodeKind::Entity(e) => NodeKind::Entity(e.clone()),
        NodeKind::Prefab(p) => {
            let mut p = p.clone();
            (p.origin, p.angles) = (m.transform_point3(p.origin), turned(m, p.angles));
            NodeKind::Prefab(p)
        }
        NodeKind::Group(g) => {
            let mut g = g.clone();
            if let Some(generator) = &mut g.generator {
                (generator.origin, generator.angles) = (m.transform_point3(generator.origin), turned(m, generator.angles));
            }
            g.mirror = g.mirror.map(|mirror| mirror.transformed(m));
            g.link = g.link.map(|l| Box::new(l.transformed(m)));
            NodeKind::Group(g)
        }
        NodeKind::Path(p) => NodeKind::Path(p.transformed(m)),
        NodeKind::Layer(_) | NodeKind::World(_) => return None,
    })
}

fn copied(map: &Map, id: NodeId, m: &DMat4, depth: usize) -> Option<Mirrored> {
    let n = map.get(id).filter(|_| depth <= MAX_DEPTH)?;
    let point = matches!(n.kind, NodeKind::Entity(_)) && n.children.is_empty();
    let kind = transform(&n.kind, m, point)?;
    let children = n.children.iter().filter_map(|c| copied(map, *c, m, depth + 1)).collect();
    Some(Mirrored { source: id, kind, children })
}

/// The copy's link: its source and frame, when `id` is a linked copy.
pub fn copy_of(map: &Map, id: NodeId) -> Option<(NodeId, DMat4)> {
    match map.get(id).map(|n| &n.kind) {
        Some(NodeKind::Group(Group { link: Some(l), .. })) => l.source.map(|s| (s, l.frame)),
        _ => None,
    }
}

/// What moves an original's contents into a copy with `frame`.
pub fn copy_matrix(map: &Map, source: NodeId, frame: &DMat4) -> Option<DMat4> {
    let from = match map.get(source).map(|n| &n.kind) {
        Some(NodeKind::Group(Group { link: Some(l), .. })) => l.frame,
        _ => DMat4::IDENTITY,
    };
    (from.determinant().abs() > 1e-12).then(|| *frame * from.inverse())
}

/// A copy's contents as real nodes: its original's own children moved into the copy's frame.
pub fn contents(map: &Map, copy: NodeId) -> Vec<Mirrored> {
    let Some((source, frame)) = copy_of(map, copy) else { return Vec::new() };
    let Some(m) = copy_matrix(map, source, &frame) else { return Vec::new() };
    map.get(source).map(|n| n.children.clone()).unwrap_or_default().iter().filter_map(|c| copied(map, *c, &m, 0)).collect()
}

/// Fills each linked copy in `out` with its source's contents, sources first, each added node owned by its copy.
pub fn expand(out: &mut Map, owner: &mut BTreeMap<NodeId, NodeId>) {
    let mut pending: Vec<NodeId> = out.nodes().filter(|(id, _)| copy_of(out, *id).is_some()).map(|(id, _)| id).collect();
    let mut patient = true;
    for _ in 0..=MAX_DEPTH * 2 {
        let waiting: BTreeSet<NodeId> = if patient { pending.iter().copied().collect() } else { BTreeSet::new() };
        let (ready, rest): (Vec<NodeId>, Vec<NodeId>) = pending.into_iter().partition(|id| {
            copy_of(out, *id).is_some_and(|(source, _)| out.get(source).is_some() && out.subtree(source).iter().all(|n| n != id && !waiting.contains(n)))
        });
        pending = rest;
        // Copies stuck behind a cycle fill with what their originals hold once the rest are done.
        match (ready.is_empty(), patient) {
            (true, true) => {
                patient = false;
                continue;
            }
            (true, false) => break,
            _ => {}
        }
        for id in ready {
            let Some((source, frame)) = copy_of(out, id) else { continue };
            let Some(m) = copy_matrix(out, source, &frame) else { continue };
            let items: Vec<Mirrored> = out.get(source).map(|n| n.children.clone()).unwrap_or_default().iter().filter_map(|c| copied(out, *c, &m, 0)).collect();
            let mut e = out.edit();
            let Ok(added) = insert(&mut e, out, id, items) else { continue };
            let belongs = owner.get(&id).copied().unwrap_or(id);
            for (node, _) in added {
                owner.insert(node, belongs);
            }
            let c = e.finish();
            out.apply(c);
        }
    }
}

#[cfg(test)]
mod tests {
    use mapcraft_geom::Aabb;

    use super::*;
    use crate::{Axis, Brush, Mirror, ROOT, TexDefaults};

    fn cuboid(min: DVec3, max: DVec3) -> NodeKind {
        NodeKind::Brush(Brush::cuboid(Aabb::new(min, max), &TexDefaults::default()))
    }

    fn group(link: Option<Link>) -> NodeKind {
        NodeKind::Group(Group { name: "Lamp".into(), link: link.map(Box::new), ..Group::default() })
    }

    fn centers(map: &Map, id: NodeId) -> Vec<DVec3> {
        map.subtree(id).into_iter().filter_map(|n| Some(map.get(n)?.kind.brush()?.bounds().ok()?.center())).collect()
    }

    #[test]
    fn copies_follow_their_original_but_not_its_moves() {
        let mut map = Map::new();
        let mut e = map.edit();
        let lamp = e.insert(ROOT, group(Some(Link { source: None, frame: DMat4::IDENTITY }))).unwrap();
        e.insert(lamp, cuboid(DVec3::ZERO, DVec3::splat(16.0))).unwrap();
        let turn = DMat4::from_translation(DVec3::new(256.0, 0.0, 0.0)) * DMat4::from_rotation_z(std::f64::consts::FRAC_PI_2);
        let copy = e.insert(ROOT, group(Some(Link { source: Some(lamp), frame: turn }))).unwrap();
        map.apply(e.finish());
        let (out, owner) = crate::mirror::expand(&map);
        let got = centers(&out, copy);
        assert_eq!(got.len(), 1);
        assert!(got[0].distance(DVec3::new(248.0, 8.0, 8.0)) < 1e-9, "{got:?}");
        assert!(out.subtree(copy).iter().skip(1).all(|n| owner.get(n) == Some(&copy)), "clicks on the copy select the copy");

        // Moving the original moves its frame too, so the copy stays where it is.
        let shift = DMat4::from_translation(DVec3::new(0.0, 512.0, 0.0));
        let mut e = map.edit();
        let Some(NodeKind::Group(mut g)) = map.get(lamp).map(|n| n.kind.clone()) else { panic!() };
        g.link = g.link.map(|l| Box::new(l.transformed(&shift)));
        e.set_kind(lamp, NodeKind::Group(g)).unwrap();
        let child = map.get(lamp).unwrap().children[0];
        let Some(NodeKind::Brush(b)) = map.get(child).map(|n| n.kind.clone()) else { panic!() };
        e.set_kind(child, NodeKind::Brush(b.transformed(&shift, true).unwrap())).unwrap();
        map.apply(e.finish());
        let (out, _) = crate::mirror::expand(&map);
        assert!(centers(&out, copy)[0].distance(DVec3::new(248.0, 8.0, 8.0)) < 1e-9);
    }

    #[test]
    fn nested_mirrored_and_cyclic_copies() {
        let mut map = Map::new();
        let mut e = map.edit();
        let lamp = e.insert(ROOT, group(Some(Link { source: None, frame: DMat4::IDENTITY }))).unwrap();
        e.insert(lamp, cuboid(DVec3::ZERO, DVec3::splat(16.0))).unwrap();
        // A copy of a group that itself holds a copy.
        let pair = e.insert(ROOT, group(Some(Link { source: None, frame: DMat4::IDENTITY }))).unwrap();
        e.insert(pair, group(Some(Link { source: Some(lamp), frame: DMat4::from_translation(DVec3::new(0.0, 64.0, 0.0)) }))).unwrap();
        let pair_copy = e.insert(ROOT, group(Some(Link { source: Some(pair), frame: DMat4::from_translation(DVec3::new(0.0, 0.0, 128.0)) }))).unwrap();
        // A copy inside a mirror group is mirrored too.
        let mirror = e
            .insert(
                ROOT,
                NodeKind::Group(Group { name: "M".into(), mirror: Some(Mirror { axis: Axis::X, offset: -64.0, texture_lock: true }), ..Group::default() }),
            )
            .unwrap();
        let mirrored = e.insert(mirror, group(Some(Link { source: Some(lamp), frame: DMat4::from_translation(DVec3::new(64.0, 0.0, 0.0)) }))).unwrap();
        // A copy of its own parent never fills.
        let selfish = e.insert(lamp, group(Some(Link { source: Some(lamp), frame: DMat4::IDENTITY }))).unwrap();
        map.apply(e.finish());
        let (out, owner) = crate::mirror::expand(&map);
        assert!(centers(&out, pair_copy).iter().any(|c| c.distance(DVec3::new(8.0, 72.0, 136.0)) < 1e-9), "{:?}", centers(&out, pair_copy));
        assert!(centers(&out, mirrored).iter().any(|c| c.distance(DVec3::new(72.0, 8.0, 8.0)) < 1e-9));
        assert!(centers(&out, mirror).iter().any(|c| c.distance(DVec3::new(-200.0, 8.0, 8.0)) < 1e-9), "{:?}", centers(&out, mirror));
        assert!(out.get(selfish).is_some_and(|n| n.children.is_empty()));
        assert!(owner.values().all(|o| map.get(*o).is_some()), "every drawn copy belongs to a real node");
    }
}
