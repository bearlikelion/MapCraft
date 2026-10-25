use std::collections::BTreeSet;

use mapcraft_map::{Cordon, Face, FaceId, Map, NodeId, NodeKind, Tag, TagId};

use crate::{EngineError, Session};

/// Entities whose classname or any value contains `query`, or with `key=value`, whose `key`
/// value contains `value`; case does not matter.
pub fn find_entities(map: &Map, query: &str) -> Vec<NodeId> {
    let q = query.trim().to_lowercase();
    let (key, value) = match q.split_once('=') {
        Some((k, v)) => (Some(k.trim().to_string()), v.trim().to_string()),
        None => (None, q),
    };
    map.nodes()
        .filter(|(_, n)| matches!(n.kind, NodeKind::Entity(_)))
        .filter(|(_, n)| {
            let Some(e) = n.kind.entity() else { return false };
            match &key {
                Some(k) => e.properties.iter().any(|(pk, pv)| pk.to_lowercase() == *k && pv.to_lowercase().contains(&value)),
                None => e.classname.to_lowercase().contains(&value) || e.properties.iter().any(|(_, v)| v.to_lowercase().contains(&value)),
            }
        })
        .map(|(id, _)| id)
        .collect()
}

impl Session {
    /// The nodes an edit with `selected_only` reaches: the selected subtrees, or the whole map.
    fn scope(&self, selected_only: bool) -> BTreeSet<NodeId> {
        if selected_only { self.selection.iter().flat_map(|id| self.map.subtree(*id)).collect() } else { self.map.nodes().map(|(id, _)| id).collect() }
    }

    /// Creates a tag holding the selection (a Hammer visgroup).
    pub fn create_tag(&mut self, name: &str) -> Result<TagId, EngineError> {
        let id = TagId(self.map.tags.iter().map(|t| t.id.0).max().unwrap_or(0).saturating_add(1));
        let mut tags = self.map.tags.clone();
        tags.push(Tag { id, name: name.into(), color: None, parent: None, hidden: false });
        let mut e = self.map.edit();
        e.set_tags(tags);
        for n in &self.selection {
            e.update(*n, |_, _, t, _| t.push(id))?;
        }
        let c = e.finish();
        self.apply("Create tag", c);
        Ok(id)
    }

    /// Adds the selection to `tag`, or takes it out.
    pub fn tag_selection(&mut self, tag: TagId, add: bool) -> Result<(), EngineError> {
        let ids = self.require_selection()?;
        let mut e = self.map.edit();
        for id in ids {
            e.update(id, |_, _, t, _| {
                t.retain(|x| *x != tag);
                if add {
                    t.push(tag);
                }
            })?;
        }
        let c = e.finish();
        self.apply(if add { "Add to tag" } else { "Remove from tag" }, c);
        Ok(())
    }

    /// Replaces the tag list (rename, hide, delete); nodes drop tags that no longer exist.
    pub fn edit_tags(&mut self, tags: Vec<Tag>, label: &str) -> Result<(), EngineError> {
        let keep: BTreeSet<TagId> = tags.iter().map(|t| t.id).collect();
        let mut e = self.map.edit();
        for (id, n) in self.map.nodes() {
            if n.tags.iter().any(|t| !keep.contains(t)) {
                e.update(id, |_, _, t, _| t.retain(|x| keep.contains(x)))?;
            }
        }
        e.set_tags(tags);
        let c = e.finish();
        self.apply(label, c);
        Ok(())
    }

    pub fn select_tag(&mut self, tag: TagId) {
        self.selection = self.map.nodes().filter(|(_, n)| n.tags.contains(&tag) && !n.locked).map(|(id, _)| id).collect();
    }

    pub fn edit_cordons(&mut self, cordons: Vec<Cordon>, label: &str) {
        if cordons == self.map.cordons {
            return;
        }
        let mut e = self.map.edit();
        e.set_cordons(cordons);
        let c = e.finish();
        self.apply(label, c);
    }

    /// Renames materials on faces: whole names compared without case, or with `partial`, every
    /// occurrence of `find` inside a name. Returns how many faces changed.
    pub fn replace_materials(&mut self, find: &str, replace: &str, partial: bool, selected_only: bool) -> Result<usize, EngineError> {
        if find.is_empty() {
            return Ok(0);
        }
        let scope = self.scope(selected_only);
        let mut e = self.map.edit();
        let mut count = 0;
        for (id, n) in self.map.nodes().filter(|(id, _)| scope.contains(id)) {
            let Some(mut b) = n.kind.brush().cloned() else { continue };
            let mut changed = false;
            for f in &mut b.faces {
                let new = if partial && f.material.contains(find) {
                    f.material.replace(find, replace)
                } else if !partial && f.material.eq_ignore_ascii_case(find) {
                    replace.to_string()
                } else {
                    continue;
                };
                f.material = new;
                changed = true;
                count += 1;
            }
            if changed {
                e.set_kind(id, NodeKind::Brush(b))?;
            }
        }
        let c = e.finish();
        self.apply("Replace materials", c);
        Ok(count)
    }

    /// Replaces text inside entity values and output targets and parameters. Returns how many
    /// values changed.
    pub fn replace_values(&mut self, find: &str, replace: &str, selected_only: bool) -> Result<usize, EngineError> {
        if find.is_empty() {
            return Ok(0);
        }
        let scope = self.scope(selected_only);
        let mut e = self.map.edit();
        let mut count = 0;
        for (id, n) in self.map.nodes().filter(|(id, _)| scope.contains(id)) {
            let NodeKind::Entity(mut ent) = n.kind.clone() else { continue };
            let values = ent.properties.iter_mut().map(|(_, v)| v);
            let conns = ent.connections.iter_mut().flat_map(|c| [&mut c.target, &mut c.param]);
            let mut changed = 0;
            for v in values.chain(conns).filter(|v| v.contains(find)) {
                *v = v.replace(find, replace);
                changed += 1;
            }
            if changed > 0 {
                e.set_kind(id, NodeKind::Entity(ent))?;
                count += changed;
            }
        }
        let c = e.finish();
        self.apply("Replace values", c);
        Ok(count)
    }

    /// Gives a face `src`'s material and alignment, wrapped across their shared edge.
    pub fn wrap_face(&mut self, id: NodeId, face: FaceId, src: &Face) -> Result<(), EngineError> {
        let Some(mut b) = self.map.get(id).and_then(|n| n.kind.brush()).cloned() else { return Ok(()) };
        for f in b.faces.iter_mut().filter(|f| f.id == face) {
            f.wrap_from(src);
        }
        self.set_kind(id, NodeKind::Brush(b), "Apply material and alignment", None)
    }
}

#[cfg(test)]
mod tests {
    use mapcraft_geom::{Aabb, DVec3};
    use mapcraft_map::Connection;

    use super::*;

    #[test]
    fn tags_cordons_and_replace() {
        let mut s = Session::default();
        s.tex.material = "dev/grid".into();
        let a = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
        let t = s.create_tag("Detail").unwrap();
        assert_eq!(s.map.get(a).unwrap().tags, vec![t]);
        let mut tags = s.map.tags.clone();
        tags[0].hidden = true;
        s.edit_tags(tags, "Hide tag").unwrap();
        assert!(!s.map.visible_nodes().contains(&a));
        s.edit_tags(vec![], "Delete tag").unwrap();
        assert!(s.map.get(a).unwrap().tags.is_empty());
        s.undo();
        s.select_tag(t);
        assert_eq!(s.selection, BTreeSet::from([a]));

        s.edit_cordons(vec![Cordon { name: "c".into(), bounds: Aabb::new(DVec3::splat(500.0), DVec3::splat(600.0)), active: true }], "Cordon");
        assert_eq!(s.map.visible_nodes().len(), 1);
        s.undo();
        assert!(s.map.cordons.is_empty());

        assert_eq!(s.replace_materials("DEV/GRID", "ramps/orange", false, false).unwrap(), 6);
        assert_eq!(s.replace_materials("orange", "blue", true, false).unwrap(), 6);
        assert!(s.map.get(a).unwrap().kind.brush().unwrap().faces.iter().all(|f| f.material == "ramps/blue"));

        let ent = s.create_point_entity("trigger_teleport", DVec3::ZERO, vec![("target".into(), "dest_1".into())]).unwrap();
        let mut e = s.map.get(ent).unwrap().kind.entity().cloned().unwrap();
        e.connections.push(Connection {
            output: "OnTrigger".into(),
            target: "dest_1".into(),
            input: "Kill".into(),
            param: String::new(),
            delay: 0.0,
            times_to_fire: -1,
        });
        s.set_kind(ent, NodeKind::Entity(e), "Edit", None).unwrap();
        assert_eq!(find_entities(&s.map, "target=DEST"), vec![ent]);
        assert_eq!(find_entities(&s.map, "teleport"), vec![ent]);
        assert!(find_entities(&s.map, "origin=5").is_empty());
        assert_eq!(s.replace_values("dest_1", "dest_2", false).unwrap(), 2);
        assert_eq!(s.map.get(ent).unwrap().kind.entity().unwrap().connections[0].target, "dest_2");
    }
}
