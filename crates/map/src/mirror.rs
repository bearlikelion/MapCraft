//! Mirror groups: a group whose children are copied across a plane, kept in step as they change.

use std::collections::BTreeMap;

use mapcraft_geom::{DMat4, DVec3, EPSILON, Plane, Side};
use serde::{Deserialize, Serialize};

use crate::entity::parse_vec3;
use crate::map::MAX_DEPTH;
use crate::{Brush, Edit, EditError, Entity, FaceId, Group, Map, NodeId, NodeKind, Prefab, ROOT};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Axis {
    #[default]
    X,
    Y,
    Z,
}

impl Axis {
    pub const ALL: [Axis; 3] = [Axis::X, Axis::Y, Axis::Z];

    pub fn index(self) -> usize {
        self as usize
    }
}

/// The plane where coordinate `axis` equals `offset`, across which a group's children are copied.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mirror {
    pub axis: Axis,
    pub offset: f64,
    /// Copies carry their textures across the plane; off, they keep the originals' texture axes.
    #[serde(default = "yes")]
    pub texture_lock: bool,
}

fn yes() -> bool {
    true
}

/// A node of a mirror group's derived half and the node it copies.
#[derive(Clone, Debug, PartialEq)]
pub struct Mirrored {
    pub source: NodeId,
    pub kind: NodeKind,
    pub children: Vec<Mirrored>,
}

fn fmt_vec(v: DVec3) -> String {
    let v = v + 0.0;
    format!("{} {} {}", v.x, v.y, v.z)
}

impl Mirror {
    fn normal(&self) -> DVec3 {
        DVec3::AXES[self.axis.index()]
    }

    pub fn plane(&self) -> Plane {
        Plane { normal: self.normal(), dist: self.offset }
    }

    pub fn matrix(&self) -> DMat4 {
        DMat4::from_translation(self.normal() * 2.0 * self.offset) * DMat4::from_scale(DVec3::ONE - 2.0 * self.normal())
    }

    pub fn point(&self, mut p: DVec3) -> DVec3 {
        let i = self.axis.index();
        p[i] = 2.0 * self.offset - p[i];
        p
    }

    fn on_plane(&self, p: DVec3) -> bool {
        (p[self.axis.index()] - self.offset).abs() <= EPSILON
    }

    /// Pitch, yaw and roll in degrees, turned as the mirror image faces; a Z mirror keeps entities upright.
    pub fn angles(&self, a: DVec3) -> DVec3 {
        let r = match self.axis {
            Axis::X => DVec3::new(a.x, 180.0 - a.y, -a.z),
            Axis::Y => DVec3::new(a.x, -a.y, -a.z),
            Axis::Z => DVec3::new(-a.x, a.y, a.z),
        };
        DVec3::new(r.x, r.y.rem_euclid(360.0), r.z) + 0.0
    }

    /// The plane after `m` moves its group; unchanged when `m` turns it off the world axes.
    pub fn transformed(&self, m: &DMat4) -> Self {
        let n = m.transform_vector3(self.normal()).normalize_or_zero();
        let i = n.abs().max_position();
        if (n[i].abs() - 1.0).abs() > 1e-9 {
            return *self;
        }
        let o = m.transform_point3(self.normal() * self.offset)[i];
        let axis = Axis::ALL.get(i).copied().unwrap_or(self.axis);
        Self { axis, offset: if (o - o.round()).abs() < 1e-6 { o.round() } else { o }, ..*self }
    }

    /// The mirror image of `b` less what `b` already fills, so a brush crossing the plane adds only its missing side.
    pub fn brush(&self, b: &Brush) -> Vec<Brush> {
        let Ok(image) = b.transformed(&self.matrix(), self.texture_lock) else { return Vec::new() };
        let image = image.with_new_ids();
        if b.solid().is_ok_and(|s| s.side_of(&self.plane()) == Side::Spanning) { image.subtract(b).unwrap_or_default() } else { vec![image] }
    }

    pub fn entity(&self, e: &Entity) -> Entity {
        let mut e = e.clone();
        if let Some(o) = e.origin() {
            e.set("origin", fmt_vec(self.point(o)));
        }
        if let Some(a) = e.get("angles").and_then(parse_vec3) {
            e.set("angles", fmt_vec(self.angles(a)));
        }
        if let Some(yaw) = e.get("angle").and_then(|s| s.trim().parse::<f64>().ok()).filter(|y| y.is_finite()) {
            // -1 and -2 mean straight up and straight down.
            let yaw = match (yaw < 0.0, self.axis) {
                (true, Axis::Z) => -3.0 - yaw,
                (true, _) => yaw,
                (false, _) => self.angles(DVec3::new(0.0, yaw, 0.0)).y,
            };
            e.set("angle", format!("{}", yaw + 0.0));
        }
        e
    }

    fn node(&self, map: &Map, id: NodeId, depth: usize) -> Vec<Mirrored> {
        let Some(n) = map.get(id).filter(|_| depth <= MAX_DEPTH) else { return Vec::new() };
        let kind = match &n.kind {
            NodeKind::Brush(b) => return self.brush(b).into_iter().map(|b| Mirrored { source: id, kind: NodeKind::Brush(b), children: Vec::new() }).collect(),
            NodeKind::Entity(e) if n.children.is_empty() && e.origin().is_some_and(|o| self.on_plane(o)) => return Vec::new(),
            NodeKind::Entity(e) => NodeKind::Entity(self.entity(e)),
            NodeKind::Prefab(p) if self.on_plane(p.origin) => return Vec::new(),
            NodeKind::Prefab(p) => NodeKind::Prefab(Prefab { origin: self.point(p.origin), angles: self.angles(p.angles), ..p.clone() }),
            // A nested mirror group's copy stays live, so making the outer half real keeps it whole.
            NodeKind::Group(g) => NodeKind::Group(Group {
                name: g.name.clone(),
                generator: None,
                mirror: g.mirror.map(|m| m.transformed(&self.matrix())),
                link: g.link.as_ref().map(|l| Box::new(l.transformed(&self.matrix()))),
                curve: None,
                boolean: g.boolean,
            }),
            NodeKind::Path(p) => NodeKind::Path(p.transformed(&self.matrix())),
            NodeKind::Layer(_) | NodeKind::World(_) => return Vec::new(),
        };
        let children: Vec<Mirrored> = n.children.iter().flat_map(|c| self.node(map, *c, depth + 1)).collect();
        let link_copy = matches!(&n.kind, NodeKind::Group(g) if g.link.as_ref().is_some_and(|l| l.source.is_some()));
        if children.is_empty() && !link_copy && (matches!(n.kind, NodeKind::Group(_)) || !n.children.is_empty()) {
            return Vec::new();
        }
        vec![Mirrored { source: id, kind, children }]
    }
}

pub fn is_mirror_group(map: &Map, id: NodeId) -> bool {
    matches!(map.get(id).map(|n| &n.kind), Some(NodeKind::Group(Group { mirror: Some(_), .. })))
}

/// The derived half of `group`: its children mirrored, or nothing when it is not a mirror group.
pub fn mirrored(map: &Map, group: NodeId) -> Vec<Mirrored> {
    let Some((Some(m), children)) = map.get(group).and_then(|n| match &n.kind {
        NodeKind::Group(g) => Some((g.mirror, &n.children)),
        _ => None,
    }) else {
        return Vec::new();
    };
    children.iter().flat_map(|c| m.node(map, *c, 0)).collect()
}

/// Inserts `items` under `parent` with fresh face ids and their sources' flags and tags from `src`; returns (new node, source) pairs.
pub fn insert(e: &mut Edit, src: &Map, parent: NodeId, items: Vec<Mirrored>) -> Result<Vec<(NodeId, NodeId)>, EditError> {
    let mut out = Vec::new();
    let mut stack: Vec<(Mirrored, NodeId)> = items.into_iter().rev().map(|m| (m, parent)).collect();
    while let Some((m, parent)) = stack.pop() {
        let mut kind = m.kind;
        if let NodeKind::Brush(b) = &mut kind {
            b.faces.iter_mut().for_each(|f| f.id = FaceId(0));
        }
        let id = e.insert(parent, kind)?;
        if let Some(s) = src.get(m.source) {
            let (hidden, locked, tags) = (s.hidden, s.locked, s.tags.clone());
            e.update(id, |h, l, t, _| (*h, *l, *t) = (hidden, locked, tags))?;
        }
        out.push((id, m.source));
        stack.extend(m.children.into_iter().rev().map(|c| (c, id)));
    }
    Ok(out)
}

/// `map` as games get it, mirror halves and linked copies added and boolean groups' own brushes replaced by their result, with each added node's original.
pub fn expand(map: &Map) -> (Map, BTreeMap<NodeId, NodeId>) {
    expand_with(map, true)
}

/// Like [`expand`], but boolean groups keep their own brushes beside their result, as the editor shows them.
pub fn expand_live(map: &Map) -> (Map, BTreeMap<NodeId, NodeId>) {
    expand_with(map, false)
}

fn expand_with(map: &Map, game: bool) -> (Map, BTreeMap<NodeId, NodeId>) {
    let groups: Vec<NodeId> = map.subtree(ROOT).into_iter().rev().filter(|id| is_mirror_group(map, *id)).collect();
    let mut out = map.clone();
    let mut owner = BTreeMap::new();
    for g in groups {
        let mut e = out.edit();
        let Ok(added) = insert(&mut e, &out, g, mirrored(&out, g)) else { continue };
        for (id, source) in added {
            owner.insert(id, owner.get(&source).copied().unwrap_or(source));
        }
        let c = e.finish();
        out.apply(c);
    }
    crate::link::expand(&mut out, &mut owner);
    crate::boolean::expand(&mut out, &mut owner, game);
    (out, owner)
}

#[cfg(test)]
mod tests {
    use mapcraft_geom::Aabb;

    use super::*;
    use crate::TexDefaults;

    fn cuboid(min: DVec3, max: DVec3) -> NodeKind {
        NodeKind::Brush(Brush::cuboid(Aabb::new(min, max), &TexDefaults::default()))
    }

    fn mirror_group(axis: Axis, offset: f64) -> NodeKind {
        NodeKind::Group(Group { name: "Mirror".into(), mirror: Some(Mirror { axis, offset, texture_lock: true }), ..Group::default() })
    }

    fn brush_bounds(map: &Map, ids: impl IntoIterator<Item = NodeId>) -> Vec<Aabb> {
        ids.into_iter().filter_map(|id| map.get(id)?.kind.brush()?.bounds().ok()).collect()
    }

    #[test]
    fn halves_meet_at_the_plane() {
        let mut map = Map::new();
        let mut e = map.edit();
        let g = e.insert(ROOT, mirror_group(Axis::X, 0.0)).unwrap();
        e.insert(g, cuboid(DVec3::new(-128.0, 0.0, 0.0), DVec3::new(0.0, 64.0, 64.0))).unwrap();
        e.insert(g, cuboid(DVec3::new(-32.0, 100.0, 0.0), DVec3::new(32.0, 132.0, 16.0))).unwrap();
        e.insert(g, cuboid(DVec3::new(-64.0, 200.0, 0.0), DVec3::new(16.0, 232.0, 16.0))).unwrap();
        let mut spawn = Entity::new("info_player_start");
        spawn.set("origin", "-96 32 0");
        spawn.set("angles", "0 30 0");
        e.insert(g, NodeKind::Entity(spawn)).unwrap();
        let mut light = Entity::new("light");
        light.set("origin", "0 32 64");
        e.insert(g, NodeKind::Entity(light)).unwrap();
        map.apply(e.finish());

        let (full, owner) = expand(&map);
        let added: Vec<NodeId> = full.get(g).unwrap().children.iter().copied().filter(|c| owner.contains_key(c)).collect();
        let boxes = brush_bounds(&full, added.clone());
        assert_eq!(
            boxes,
            vec![Aabb::new(DVec3::new(0.0, 0.0, 0.0), DVec3::new(128.0, 64.0, 64.0)), Aabb::new(DVec3::new(16.0, 200.0, 0.0), DVec3::new(64.0, 232.0, 16.0)),],
            "a brush on the plane meets its copy, one centered on it adds nothing, one crossing it adds only the missing side"
        );
        let spawn = added.iter().find_map(|id| full.get(*id)?.kind.entity().filter(|e| e.classname == "info_player_start")).unwrap();
        assert_eq!(spawn.get("origin"), Some("96 32 0"));
        assert_eq!(spawn.get("angles"), Some("0 150 0"));
        assert!(
            !added.iter().any(|id| full.get(*id).and_then(|n| n.kind.entity()).is_some_and(|e| e.classname == "light")),
            "a light on the plane is not doubled"
        );
        let ids: Vec<u64> = full.nodes().filter_map(|(_, n)| n.kind.brush()).flat_map(|b| b.faces.iter().map(|f| f.id.0)).collect();
        assert_eq!(ids.len(), ids.iter().collect::<std::collections::BTreeSet<_>>().len(), "copies get their own face ids");
    }

    #[test]
    fn copies_are_exact_mirror_images() {
        let b = Brush::wedge(Aabb::new(DVec3::new(-200.0, -40.0, 0.0), DVec3::new(-72.0, 24.0, 96.0)), &TexDefaults::default()).unwrap();
        for (axis, offset) in [(Axis::X, -8.0), (Axis::Y, 48.0), (Axis::Z, 160.0)] {
            let m = Mirror { axis, offset, texture_lock: true };
            let copy = m.brush(&b);
            assert_eq!(copy.len(), 1);
            let key = |v: DVec3| (v * 1000.0).round().as_i64vec3().to_array();
            let mut want: Vec<[i64; 3]> = b.vertices().unwrap().into_iter().map(|v| key(m.point(v))).collect();
            let mut got: Vec<[i64; 3]> = copy[0].vertices().unwrap().into_iter().map(key).collect();
            want.sort();
            got.sort();
            assert_eq!(got, want, "{axis:?}");
            let p = b.faces[2].points[1];
            assert!(
                (copy[0].faces[2].uv(m.point(p), mapcraft_geom::DVec2::ONE) - b.faces[2].uv(p, mapcraft_geom::DVec2::ONE)).length() < 1e-9,
                "texture lock carries the texture across"
            );
        }
    }

    #[test]
    fn nested_mirrors_make_four_quarters() {
        let mut map = Map::new();
        let mut e = map.edit();
        let outer = e.insert(ROOT, mirror_group(Axis::Y, 0.0)).unwrap();
        let inner = e.insert(outer, mirror_group(Axis::X, 0.0)).unwrap();
        let b = e.insert(inner, cuboid(DVec3::new(-64.0, -64.0, 0.0), DVec3::new(-32.0, -32.0, 32.0))).unwrap();
        map.apply(e.finish());
        let (full, owner) = expand(&map);
        let mut mins: Vec<[i64; 2]> = full.nodes().filter_map(|(_, n)| n.kind.brush()?.bounds().ok()).map(|b| [b.min.x as i64, b.min.y as i64]).collect();
        mins.sort();
        assert_eq!(mins, vec![[-64, -64], [-64, 32], [32, -64], [32, 32]]);
        assert!(owner.values().all(|o| *o == b || *o == inner), "every copy leads back to an original");

        let moved = Mirror { axis: Axis::X, offset: 16.0, texture_lock: true }.transformed(&DMat4::from_rotation_z(std::f64::consts::FRAC_PI_2));
        assert_eq!((moved.axis, moved.offset), (Axis::Y, 16.0));
    }
}
