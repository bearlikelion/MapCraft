//! Boolean groups: brushes marked Add or Subtract whose result, each Add brush less every Subtract brush, is built whenever the map is expanded.

use std::collections::BTreeMap;

use mapcraft_geom::Plane;
use serde::{Deserialize, Serialize};

use crate::mirror::{Mirrored, insert};
use crate::{Brush, Face, FaceId, Group, Map, NodeId, NodeKind, TexDefaults};

/// One boolean group's result stops being cut past this many pieces, so a careless cutter cannot hang the editor.
pub const MAX_PIECES: usize = 4096;

/// What a brush does inside a boolean group; anywhere else every brush adds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Operation {
    #[default]
    Add,
    Subtract,
}

impl Operation {
    pub fn is_add(&self) -> bool {
        *self == Operation::Add
    }
}

/// Whether `id` is a boolean group, or a linked copy of one, which holds the same brushes.
pub fn is_boolean_group(map: &Map, id: NodeId) -> bool {
    let flagged = |id: NodeId| matches!(map.get(id).map(|n| &n.kind), Some(NodeKind::Group(Group { boolean: true, .. })));
    flagged(id) || crate::link::copy_of(map, id).is_some_and(|(source, _)| flagged(source))
}

/// The boolean group holding brush `id` as one of its own, the brushes its result is built from.
pub fn group_of(map: &Map, id: NodeId) -> Option<NodeId> {
    map.get(id).filter(|n| n.kind.brush().is_some())?.parent.filter(|p| is_boolean_group(map, *p))
}

/// The operation of brush `id` when it is one of a boolean group's own brushes.
pub fn source_op(map: &Map, id: NodeId) -> Option<Operation> {
    group_of(map, id)?;
    map.get(id)?.kind.brush().map(|b| b.operation)
}

fn brushes(map: &Map, group: NodeId) -> Vec<(NodeId, &Brush)> {
    map.get(group).map(|n| n.children.iter().filter_map(|c| Some((*c, map.get(*c)?.kind.brush()?))).collect()).unwrap_or_default()
}

/// Each Add brush of boolean group `group` less every Subtract brush, as convex pieces paired with the Add brush they came from.
pub fn result(map: &Map, group: NodeId) -> Vec<(NodeId, Brush)> {
    let all = brushes(map, group);
    let cutters: Vec<&Brush> = all.iter().filter(|(_, b)| b.operation == Operation::Subtract).map(|(_, b)| *b).collect();
    let mut out: Vec<(NodeId, Brush)> = Vec::new();
    for (id, b) in all.iter().filter(|(_, b)| b.operation.is_add()) {
        let mut parts = vec![(*b).clone()];
        for c in &cutters {
            let mut next = Vec::with_capacity(parts.len());
            for p in parts {
                if out.len() + next.len() < MAX_PIECES { next.extend(carve(p, c)) } else { next.push(p) }
            }
            parts = next;
        }
        out.extend(parts.into_iter().map(|p| (*id, p.with_new_ids())));
    }
    out
}

/// `p` less `cutter`, cut along the cutter's box first so the pieces away from it stay whole slabs instead of slivers.
fn carve(p: Brush, cutter: &Brush) -> Vec<Brush> {
    let (Ok(bounds), Some(face)) = (cutter.bounds(), cutter.faces.first()) else { return vec![p] };
    if p.bounds().is_ok_and(|b| !b.intersects(&bounds)) {
        return vec![p];
    }
    let block = Brush::cuboid(bounds, &TexDefaults { material: face.material.clone(), scale: face.u.scale });
    let (Some(inside), Ok(mut out)) = (p.intersect(&block), p.subtract(&block)) else { return vec![p] };
    match inside.subtract(cutter) {
        Ok(rest) => out.extend(rest),
        Err(_) => out.push(inside),
    }
    out
}

/// The face of `add`, or for a cut face of the Subtract brush that cut it, that `face` of a piece of `add`'s result lies on.
pub fn source_face(map: &Map, add: NodeId, face: &Face) -> Option<(NodeId, FaceId)> {
    let group = group_of(map, add)?;
    let plane = face.plane()?;
    let flipped = Plane { normal: -plane.normal, dist: -plane.dist };
    let on = |f: &Face, p: &Plane| f.plane().is_some_and(|q| q.approx_eq(p, 1e-3));
    let own = map.get(add)?.kind.brush()?;
    if let Some(f) = own.faces.iter().find(|f| on(f, &plane)) {
        return Some((add, f.id));
    }
    brushes(map, group)
        .into_iter()
        .filter(|(_, b)| b.operation == Operation::Subtract)
        .find_map(|(id, b)| b.faces.iter().find(|f| on(f, &flipped) || on(f, &plane)).map(|f| (id, f.id)))
}

/// Adds each boolean group's result to `out`, pieces owned by their Add brush: for a `game` in place of the group's own brushes, else in a plain group inside it.
pub fn expand(out: &mut Map, owner: &mut BTreeMap<NodeId, NodeId>, game: bool) {
    let groups: Vec<NodeId> = out.nodes().map(|(id, _)| id).filter(|id| is_boolean_group(out, *id)).collect();
    for g in groups {
        let items = result(out, g).into_iter().map(|(source, b)| Mirrored { source, kind: NodeKind::Brush(b), children: Vec::new() }).collect();
        let mut e = out.edit();
        let parent = match game {
            true => g,
            false => {
                let Ok(pieces) = e.insert(g, NodeKind::Group(Group { name: "Result".into(), ..Group::default() })) else { continue };
                owner.insert(pieces, owner.get(&g).copied().unwrap_or(g));
                pieces
            }
        };
        let Ok(added) = insert(&mut e, out, parent, items) else { continue };
        for (id, source) in added {
            owner.insert(id, owner.get(&source).copied().unwrap_or(source));
        }
        if game {
            for (id, _) in brushes(out, g) {
                let _ = e.remove(id);
            }
            if let Some(NodeKind::Group(group)) = e.node(g).map(|n| n.kind.clone()) {
                let _ = e.set_kind(g, NodeKind::Group(Group { boolean: false, ..group }));
            }
        }
        let c = e.finish();
        out.apply(c);
    }
}

#[cfg(test)]
mod tests {
    use mapcraft_geom::{Aabb, DVec3};

    use super::*;
    use crate::ROOT;

    #[test]
    fn a_cutter_left_outside_a_boolean_group_adds_and_link_copies_cut_like_their_source() {
        let mut map = Map::new();
        let mut e = map.edit();
        let g = e.insert(ROOT, NodeKind::Group(Group { boolean: true, ..Group::default() })).unwrap();
        let mut cutter = Brush::cuboid(Aabb::new(DVec3::ZERO, DVec3::splat(8.0)), &TexDefaults::default());
        cutter.operation = Operation::Subtract;
        let inside = e.insert(g, NodeKind::Brush(cutter.clone())).unwrap();
        let outside = e.insert(ROOT, NodeKind::Brush(cutter)).unwrap();
        let link = Some(Box::new(crate::Link { source: Some(g), frame: mapcraft_geom::DMat4::IDENTITY }));
        let copy = e.insert(ROOT, NodeKind::Group(Group { link, ..Group::default() })).unwrap();
        map.apply(e.finish());
        assert_eq!(source_op(&map, inside), Some(Operation::Subtract));
        assert_eq!(source_op(&map, outside), None);
        assert!(is_boolean_group(&map, copy));
    }
}
