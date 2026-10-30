//! Boolean groups made from the selection, brushes released from them, and their result applied as plain brushes.

use std::collections::BTreeSet;

use mapcraft_map::boolean::{group_of, result};
use mapcraft_map::mirror::{Mirrored, insert};
use mapcraft_map::{Brush, Edit, Group, Map, NodeId, NodeKind, Operation};

use crate::{EngineError, Session};

fn set_op(e: &mut Edit, id: NodeId, op: Operation) -> Result<(), EngineError> {
    if let Some(NodeKind::Brush(b)) = e.node(id).map(|n| n.kind.clone())
        && b.operation != op
    {
        e.set_kind(id, NodeKind::Brush(Brush { operation: op, ..b }))?;
    }
    Ok(())
}

fn in_entity(map: &Map, id: NodeId) -> bool {
    map.get(id).and_then(|n| n.parent).and_then(|p| map.get(p)).is_some_and(|p| matches!(p.kind, NodeKind::Entity(_)))
}

/// A group carrying the boolean flag itself, not a linked copy of one.
pub(crate) fn is_flagged(map: &Map, id: NodeId) -> bool {
    matches!(map.get(id).map(|n| &n.kind), Some(NodeKind::Group(Group { boolean: true, .. })))
}

impl Session {
    /// Marks the selected brushes `op`; those outside a boolean group join the selected one, else one holding a brush they touch, else a new one, around the brushes they touch too when subtracting.
    pub fn make_boolean(&mut self, op: Operation) -> Result<NodeId, EngineError> {
        let map = &self.map;
        let chosen: Vec<NodeId> = self.selection.iter().copied().filter(|id| is_flagged(map, *id)).collect();
        let picked: Vec<NodeId> =
            self.selected_brushes().into_iter().filter(|id| !in_entity(map, *id) && group_of(map, *id).is_none_or(|g| !chosen.contains(&g))).collect();
        let (inside, loose): (Vec<NodeId>, Vec<NodeId>) = picked.into_iter().partition(|id| group_of(map, *id).is_some());
        let label = if op.is_add() { "Add" } else { "Subtract" };
        let mut e = map.edit();
        for id in &inside {
            set_op(&mut e, *id, op)?;
        }
        let group = match (loose.is_empty(), inside.first(), chosen.as_slice()) {
            (true, Some(id), _) => group_of(map, *id).ok_or(EngineError::NothingSelected)?,
            (true, None, _) => return Err(EngineError::Refused("select brushes that are not part of an entity")),
            (false, _, [g]) => *g,
            (false, _, [_, _, ..]) => return Err(EngineError::Refused("select one boolean group to put them in")),
            (false, _, []) => {
                let editable = map.editable_nodes();
                let cutters: Vec<&Brush> = loose.iter().filter_map(|id| map.get(*id)?.kind.brush()).collect();
                let touched: Vec<NodeId> = map
                    .nodes()
                    .filter(|(id, n)| {
                        editable.contains(id)
                            && !loose.contains(id)
                            && !in_entity(map, *id)
                            && mapcraft_map::boolean::source_op(map, *id) != Some(Operation::Subtract)
                            && n.kind.brush().is_some_and(|b| cutters.iter().any(|c| c.intersect(b).is_some()))
                    })
                    .map(|(id, _)| id)
                    .collect();
                match touched.iter().find_map(|id| group_of(map, *id)) {
                    Some(g) => g,
                    None => {
                        let adds: Vec<NodeId> = if op.is_add() { Vec::new() } else { touched };
                        if !op.is_add() && adds.is_empty() {
                            return Err(EngineError::Refused("the brushes cut nothing; move them into what they should cut first"));
                        }
                        let first = adds.first().or(loose.first()).copied().ok_or(EngineError::NothingSelected)?;
                        let parent = map.get(first).and_then(|n| n.parent).ok_or(EngineError::NothingSelected)?;
                        let index = map.get(parent).and_then(|p| p.children.iter().position(|c| *c == first)).unwrap_or(0);
                        let g = e.insert_at(parent, index, NodeKind::Group(Group { name: "Boolean".into(), boolean: true, ..Group::default() }))?;
                        for id in adds {
                            e.reparent(id, g, None)?;
                            set_op(&mut e, id, Operation::Add)?;
                        }
                        g
                    }
                }
            }
        };
        for id in &loose {
            e.reparent(*id, group, None)?;
            set_op(&mut e, *id, op)?;
        }
        let c = e.finish();
        self.apply(label, c);
        Ok(group)
    }

    /// Moves the selected brushes out of their boolean groups, just after them, as ordinary brushes.
    pub fn release_from_boolean(&mut self) -> Result<(), EngineError> {
        let ids: Vec<NodeId> = self.selection.iter().copied().filter(|id| group_of(&self.map, *id).is_some()).collect();
        if ids.is_empty() {
            return Err(EngineError::Refused("select brushes inside a boolean group"));
        }
        let mut e = self.map.edit();
        for id in ids {
            let Some(group) = e.node(id).and_then(|n| n.parent) else { continue };
            let Some(parent) = e.node(group).and_then(|n| n.parent) else { continue };
            let index = e.node(parent).and_then(|p| p.children.iter().position(|c| *c == group)).map(|i| i + 1);
            e.reparent(id, parent, index)?;
            set_op(&mut e, id, Operation::Add)?;
        }
        let c = e.finish();
        self.apply("Release from boolean", c);
        Ok(())
    }

    /// Replaces the own brushes of the selected boolean groups, or of those holding selected brushes, with their result, leaving plain groups.
    pub fn apply_boolean(&mut self) -> Result<(), EngineError> {
        let map = &self.map;
        let groups: BTreeSet<NodeId> =
            self.selection.iter().filter_map(|id| if is_flagged(map, *id) { Some(*id) } else { group_of(map, *id) }).filter(|g| is_flagged(map, *g)).collect();
        if groups.is_empty() {
            return Err(EngineError::Refused("select a boolean group"));
        }
        let mut e = map.edit();
        for g in &groups {
            let items = result(map, *g).into_iter().map(|(source, b)| Mirrored { source, kind: NodeKind::Brush(b), children: Vec::new() }).collect();
            for c in map.get(*g).map(|n| n.children.clone()).unwrap_or_default() {
                if map.get(c).is_some_and(|n| n.kind.brush().is_some()) {
                    e.remove(c)?;
                }
            }
            insert(&mut e, map, *g, items)?;
            if let Some(NodeKind::Group(group)) = e.node(*g).map(|n| n.kind.clone()) {
                e.set_kind(*g, NodeKind::Group(Group { boolean: false, ..group }))?;
            }
        }
        let c = e.finish();
        self.apply("Apply boolean", c);
        self.selection = groups;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use mapcraft_geom::{Aabb, DVec3};
    use mapcraft_map::ROOT;
    use mapcraft_map::boolean::is_boolean_group;

    use super::*;
    use crate::ops::Shape;

    /// Volume of what games get from `s`: every brush once mirrors, links and booleans are expanded.
    fn game_volume(s: &Session) -> f64 {
        let (game, _) = mapcraft_map::mirror::expand(&s.map);
        game.nodes().filter_map(|(_, n)| n.kind.brush()?.solid().ok()).map(|s| s.volume()).sum()
    }

    fn game_brushes(s: &Session) -> Vec<Brush> {
        let (game, _) = mapcraft_map::mirror::expand(&s.map);
        game.nodes().filter_map(|(_, n)| n.kind.brush().cloned()).collect()
    }

    /// Whether `p` is inside some brush games get.
    fn solid_at(s: &Session, p: DVec3) -> bool {
        game_brushes(s).iter().any(|b| b.planes().is_ok_and(|planes| planes.iter().all(|pl| pl.distance(p) < -1e-6)))
    }

    /// A 256 x 16 x 128 wall and a 16-sided upright cylinder through it, 64 across, centered on `at`.
    fn wall_and_cutter(s: &mut Session, at: DVec3) -> (NodeId, NodeId) {
        s.tex.material = "wall".into();
        let wall = s.create_brush(Aabb::new(DVec3::new(-128.0, -8.0, 0.0), DVec3::new(128.0, 8.0, 128.0))).unwrap();
        s.tex.material = "trim".into();
        let tube = Aabb::new(at - DVec3::new(32.0, 32.0, 96.0), at + DVec3::new(32.0, 32.0, 96.0));
        let cutter = s.create_shape(Shape::Cylinder(16), tube).unwrap()[0];
        // Turned to run through the wall along Y.
        s.rotate_selection(DVec3::X, 90.0).unwrap();
        (wall, cutter)
    }

    #[test]
    fn a_block_minus_a_cylinder_is_a_live_hole_in_convex_pieces() {
        let mut s = Session::default();
        let (wall, cutter) = wall_and_cutter(&mut s, DVec3::new(0.0, 0.0, 64.0));
        let whole = game_volume(&s);
        let tube = s.map.get(cutter).unwrap().kind.brush().unwrap().clone();
        let plug = tube.intersect(s.map.get(wall).unwrap().kind.brush().unwrap()).unwrap().solid().unwrap().volume();

        let g = s.make_boolean(Operation::Subtract).unwrap();
        assert_eq!(s.map.get(g).unwrap().children, vec![wall, cutter], "the wall it touches joins as Add, the cutter as Subtract");
        assert_eq!(s.selection, BTreeSet::from([cutter]), "the cutter stays selected, ready to move");
        let wall_volume = 256.0 * 16.0 * 128.0;
        assert!((game_volume(&s) - (wall_volume - plug)).abs() < 1e-3, "the pieces fill the wall less the hole, with no overlap");
        assert!(whole > wall_volume, "before, the cutter was solid too");
        assert!(!solid_at(&s, DVec3::new(0.0, 0.0, 64.0)) && solid_at(&s, DVec3::new(100.0, 0.0, 64.0)));
        let pieces = game_brushes(&s);
        assert!(pieces.len() > 1 && pieces.iter().all(|b| b.solid().is_ok() && b.operation.is_add()), "convex pieces, none a cutter");
        assert!(pieces.iter().flat_map(|b| &b.faces).any(|f| f.material == "trim"), "the hole is lined with the cutter's material");
        let (live, _, _) = crate::flatten(&s.map, &mut |_| None);
        let shown = live.nodes().filter(|(id, n)| n.kind.brush().is_some() && mapcraft_map::boolean::source_op(&live, *id).is_none()).count();
        assert_eq!(shown, pieces.len(), "the editor shows the pieces as ordinary brushes beside the group's own");

        s.translate_selection(DVec3::new(64.0, 0.0, 0.0), None).unwrap();
        assert!(solid_at(&s, DVec3::new(0.0, 0.0, 64.0)) && !solid_at(&s, DVec3::new(64.0, 0.0, 64.0)), "moving the cutter moves the hole");
        s.undo();
        assert!(!solid_at(&s, DVec3::new(0.0, 0.0, 64.0)) && solid_at(&s, DVec3::new(64.0, 0.0, 30.0)));
        s.undo();
        assert!(solid_at(&s, DVec3::new(0.0, 0.0, 64.0)) && s.map.get(g).is_none(), "undo takes the group away again");
        assert!((game_volume(&s) - whole).abs() < 1e-6);
        s.redo();

        s.selection = BTreeSet::from([g]);
        let before = game_volume(&s);
        s.apply_boolean().unwrap();
        assert!(!is_boolean_group(&s.map, g));
        assert!(s.map.get(wall).is_none() && s.map.get(cutter).is_none(), "the sources are gone");
        assert!((game_volume(&s) - before).abs() < 1e-6, "applying changes nothing games get");
        assert!(s.map.get(g).unwrap().children.iter().all(|c| s.map.get(*c).unwrap().kind.brush().is_some_and(|b| b.operation.is_add())));
        s.undo();
        assert!(is_boolean_group(&s.map, g) && s.map.get(cutter).is_some());
    }

    #[test]
    fn switching_releasing_and_more_cutters() {
        let mut s = Session::default();
        let (wall, _) = wall_and_cutter(&mut s, DVec3::new(-64.0, 0.0, 64.0));
        let g = s.make_boolean(Operation::Subtract).unwrap();
        let second = s.create_brush(Aabb::new(DVec3::new(32.0, -32.0, 32.0), DVec3::new(64.0, 32.0, 64.0))).unwrap();
        assert_eq!(s.make_boolean(Operation::Subtract).unwrap(), g, "a cutter touching the wall joins its group");
        assert_eq!(s.map.get(second).unwrap().parent, Some(g));
        assert!(!solid_at(&s, DVec3::new(48.0, 0.0, 48.0)));

        s.make_boolean(Operation::Add).unwrap();
        assert!(solid_at(&s, DVec3::new(48.0, 0.0, 48.0)) && solid_at(&s, DVec3::new(48.0, 20.0, 48.0)), "switched to Add it adds");
        s.release_from_boolean().unwrap();
        assert_eq!(s.map.get(ROOT).unwrap().children, vec![g, second], "released just after its group");
        assert_eq!(mapcraft_map::boolean::group_of(&s.map, second), None);

        s.selection = BTreeSet::from([wall]);
        s.make_boolean(Operation::Subtract).unwrap();
        assert!(game_brushes(&s).len() == 1, "with nothing to add the group gives nothing; only the released brush is left");
        s.undo();

        s.selection = BTreeSet::from([g, second]);
        s.make_boolean(Operation::Subtract).unwrap();
        assert_eq!(s.map.get(second).unwrap().parent, Some(g), "a selected boolean group takes the brushes");
        let lonely = s.create_brush(Aabb::new(DVec3::splat(1000.0), DVec3::splat(1064.0))).unwrap();
        s.selection = BTreeSet::from([lonely]);
        assert!(s.make_boolean(Operation::Subtract).is_err(), "a cutter touching nothing is refused");
        assert_ne!(s.make_boolean(Operation::Add).unwrap(), g, "Add on a loose brush makes a new group");
    }

    #[test]
    fn the_file_and_exports_hold_only_the_result() {
        let mut s = Session::default();
        wall_and_cutter(&mut s, DVec3::new(0.0, 0.0, 64.0));
        let g = s.make_boolean(Operation::Subtract).unwrap();
        let text = mapcraft_format::to_string(&s.map, None, &serde_json::Value::Null).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let group = &v["map"]["root"]["children"][0];
        assert_eq!(group["boolean"], true);
        assert_eq!(v["schema_version"], 3, "older builds refuse the map instead of opening an empty group");
        let kids = group["children"].as_array().unwrap();
        assert!(kids.len() > 1 && kids.iter().all(|c| c["derived"] == true && c.get("operation").is_none()), "importers get the pieces");
        assert_eq!(group["sources"].as_array().unwrap().len(), 2, "the wall and cutter sit where importers do not look");
        assert!(group["sources"][1]["operation"] == "subtract");
        let back = mapcraft_format::from_str(&text).unwrap();
        assert_eq!(back.map, s.map, "the editor reads the sources back and rebuilds the result");

        let (game, _) = mapcraft_map::mirror::expand(&s.map);
        let fragment = mapcraft_format::fragment_to_string(&s.map, &[g]).unwrap();
        assert!(fragment.contains("subtract"), "copies keep the cutter live");
        assert!(game.nodes().all(|(_, n)| !matches!(&n.kind, NodeKind::Brush(b) if !b.operation.is_add())), "games never see a cutter");
    }
}
