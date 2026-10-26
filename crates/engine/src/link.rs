//! Linked copies: duplicates that follow their original, made real or turned into the original.

use std::collections::BTreeSet;

use mapcraft_geom::{DMat4, DVec3};
use mapcraft_map::link::{contents, copy_of};
use mapcraft_map::{Group, Link, NodeId, NodeKind};

use crate::{EngineError, Session};

fn group_of(kind: Option<&NodeKind>) -> Option<&Group> {
    match kind {
        Some(NodeKind::Group(g)) => Some(g),
        _ => None,
    }
}

impl Session {
    /// Adds a copy of the selection `delta` away that follows it; a selection that is not one group is grouped first.
    pub fn duplicate_linked(&mut self, delta: DVec3) -> Result<NodeId, EngineError> {
        let roots = self.require_selection()?;
        let step = DMat4::from_translation(delta);
        let mut e = self.map.edit();
        let first = roots.first().copied().ok_or(EngineError::NothingSelected)?;
        let parent = e.node(first).and_then(|n| n.parent).ok_or(EngineError::NothingSelected)?;
        let single = match roots.as_slice() {
            [id] => group_of(e.node(*id).map(|n| &n.kind)).cloned().map(|g| (*id, g)),
            _ => None,
        };
        let (source, frame, name) = match single {
            Some((id, g)) => match g.link.as_deref().copied() {
                Some(Link { source: Some(source), frame }) => (source, step * frame, g.name),
                Some(Link { source: None, frame }) => (id, step * frame, g.name),
                None => {
                    e.set_kind(id, NodeKind::Group(Group { link: Some(Box::new(Link { source: None, frame: DMat4::IDENTITY })), ..g.clone() }))?;
                    (id, step, g.name)
                }
            },
            None => {
                let index = e.node(parent).and_then(|p| p.children.iter().position(|c| *c == first)).unwrap_or(0);
                let link = Some(Box::new(Link { source: None, frame: DMat4::IDENTITY }));
                let group = e.insert_at(parent, index, NodeKind::Group(Group { name: "Linked".into(), link, ..Group::default() }))?;
                for id in &roots {
                    e.reparent(*id, group, None)?;
                }
                (group, step, "Linked".to_string())
            }
        };
        let copy = e.insert(parent, NodeKind::Group(Group { name, link: Some(Box::new(Link { source: Some(source), frame })), ..Group::default() }))?;
        let c = e.finish();
        self.apply("Duplicate linked", c);
        self.selection = BTreeSet::from([copy]);
        Ok(copy)
    }

    /// Turns the selected linked copies into ordinary groups holding real copies of their originals' contents.
    pub fn make_link_real(&mut self) -> Result<(), EngineError> {
        let copies: Vec<NodeId> = self.selection.iter().copied().filter(|id| copy_of(&self.map, *id).is_some()).collect();
        if copies.is_empty() {
            return Err(EngineError::Refused("select a linked copy"));
        }
        let mut e = self.map.edit();
        for id in copies {
            mapcraft_map::mirror::insert(&mut e, &self.map, id, contents(&self.map, id))?;
            let Some(g) = group_of(e.node(id).map(|n| &n.kind)).cloned() else { continue };
            e.set_kind(id, NodeKind::Group(Group { link: None, ..g }))?;
        }
        let c = e.finish();
        self.apply("Make linked copy real", c);
        Ok(())
    }

    /// Makes the selected copy the original, so the old original and every other copy follow edits made here.
    pub fn edit_link_here(&mut self) -> Result<NodeId, EngineError> {
        let ids: Vec<NodeId> = self.selection.iter().copied().collect();
        let [id] = ids.as_slice() else { return Err(EngineError::Refused("select one linked copy")) };
        let id = *id;
        let (source, frame) = copy_of(&self.map, id).ok_or(EngineError::Refused("select one linked copy"))?;
        if self.map.subtree(source).contains(&id) {
            return Err(EngineError::Refused("this copy sits inside its own original"));
        }
        let old = self.map.get(source).map(|n| n.kind.clone());
        let old = group_of(old.as_ref()).cloned().ok_or(EngineError::Refused("the copy's original is gone"))?;
        let old_frame = old.link.as_ref().map_or(DMat4::IDENTITY, |l| l.frame);
        let others: Vec<NodeId> =
            self.map.nodes().filter(|(n, _)| *n != id && copy_of(&self.map, *n).is_some_and(|(s, _)| s == source)).map(|(n, _)| n).collect();
        let mut e = self.map.edit();
        mapcraft_map::mirror::insert(&mut e, &self.map, id, contents(&self.map, id))?;
        let here = group_of(e.node(id).map(|n| &n.kind)).cloned().unwrap_or_default();
        e.set_kind(id, NodeKind::Group(Group { link: Some(Box::new(Link { source: None, frame })), ..here }))?;
        for child in self.map.get(source).map(|n| n.children.clone()).unwrap_or_default() {
            e.remove(child)?;
        }
        e.set_kind(source, NodeKind::Group(Group { link: Some(Box::new(Link { source: Some(id), frame: old_frame })), ..old }))?;
        for n in others {
            let Some(g) = group_of(e.node(n).map(|x| &x.kind)).cloned() else { continue };
            let link = g.link.as_deref().map(|l| Box::new(Link { source: Some(id), ..*l }));
            e.set_kind(n, NodeKind::Group(Group { link, ..g }))?;
        }
        let c = e.finish();
        self.apply("Edit linked copy here", c);
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use mapcraft_geom::Aabb;
    use mapcraft_map::ROOT;

    use super::*;

    fn centers(s: &Session, id: NodeId) -> Vec<DVec3> {
        let (full, _) = mapcraft_map::mirror::expand(&s.map);
        full.subtree(id).into_iter().filter_map(|n| Some(full.get(n)?.kind.brush()?.bounds().ok()?.center())).collect()
    }

    #[test]
    fn linked_copies_follow_moves_and_swap_originals() {
        let mut s = Session::default();
        let a = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(16.0))).unwrap();
        s.selection = BTreeSet::from([a]);
        let copy = s.duplicate_linked(DVec3::new(128.0, 0.0, 0.0)).unwrap();
        let original = s.map.get(a).unwrap().parent.unwrap();
        assert_eq!(centers(&s, copy), vec![DVec3::new(136.0, 8.0, 8.0)]);
        assert!(s.undo() && s.map.get(a).unwrap().parent == Some(ROOT), "grouping and linking is one undo step");
        assert!(s.redo());

        // A second copy, then the original's brush grows: both copies grow too.
        s.selection = BTreeSet::from([copy]);
        let copy2 = s.duplicate_linked(DVec3::new(0.0, 128.0, 0.0)).unwrap();
        s.selection = BTreeSet::from([a]);
        s.transform_selection(&DMat4::from_scale(DVec3::new(2.0, 1.0, 1.0)), "Scale", None).unwrap();
        assert_eq!(centers(&s, copy), vec![DVec3::new(144.0, 8.0, 8.0)]);
        assert_eq!(centers(&s, copy2), vec![DVec3::new(144.0, 136.0, 8.0)]);

        // Moving the original group leaves its copies where they are.
        s.selection = BTreeSet::from([original]);
        s.transform_selection(&DMat4::from_translation(DVec3::new(0.0, 0.0, 64.0)), "Move", None).unwrap();
        assert_eq!(centers(&s, copy), vec![DVec3::new(144.0, 8.0, 8.0)]);

        // Edit here: the copy becomes the original, and the rest follow it.
        s.selection = BTreeSet::from([copy]);
        s.edit_link_here().unwrap();
        let here = s.map.get(copy).unwrap().children[0];
        s.selection = BTreeSet::from([here]);
        s.transform_selection(&DMat4::from_translation(DVec3::new(0.0, 0.0, 32.0)), "Move", None).unwrap();
        assert_eq!(centers(&s, original), vec![DVec3::new(16.0, 8.0, 104.0)], "the old original follows");
        assert_eq!(centers(&s, copy2), vec![DVec3::new(144.0, 136.0, 40.0)]);

        s.selection = BTreeSet::from([copy2]);
        s.make_link_real().unwrap();
        assert!(copy_of(&s.map, copy2).is_none() && s.map.get(copy2).unwrap().children.len() == 1);
    }
}
