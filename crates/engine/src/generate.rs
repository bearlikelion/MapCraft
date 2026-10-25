use std::collections::BTreeSet;

use mapcraft_format::Fragment;
use mapcraft_geom::{DMat4, DVec3};
use mapcraft_map::{Generator, Group, NodeId, NodeKind};

use crate::ops::insert_fragments;
use crate::{EngineError, Session};

/// Where a generator's output goes.
pub fn generator_transform(g: &Generator) -> DMat4 {
    crate::prefab::placement(g.origin, g.angles)
}

impl Session {
    /// Inserts a plugin command's output at `origin` and selects it.
    pub fn insert_output(&mut self, label: &str, frags: Vec<Fragment>, origin: DVec3) -> Result<Vec<NodeId>, EngineError> {
        let mut e = self.map.edit();
        let roots = insert_fragments(&mut e, self.active_parent, frags, &DMat4::from_translation(origin), true)?;
        let c = e.finish();
        self.apply(label, c);
        self.selection = roots.iter().copied().collect();
        Ok(roots)
    }

    /// Puts imported nodes, already in map space, in a new layer named `name`.
    pub fn import_layer(&mut self, name: &str, frags: Vec<Fragment>) -> Result<NodeId, EngineError> {
        let mut e = self.map.edit();
        let layer = e.insert(mapcraft_map::ROOT, NodeKind::Layer(mapcraft_map::Layer { name: name.into(), ..Default::default() }))?;
        insert_fragments(&mut e, layer, frags, &DMat4::IDENTITY, true)?;
        let c = e.finish();
        self.apply(&format!("Import {name}"), c);
        self.selection.clear();
        Ok(layer)
    }

    /// Creates a group that holds a generator's output and runs it again when edited.
    pub fn create_generator(&mut self, name: &str, generator: Generator, frags: Vec<Fragment>) -> Result<NodeId, EngineError> {
        let m = generator_transform(&generator);
        let mut e = self.map.edit();
        let group = e.insert(self.active_parent, NodeKind::Group(Group { name: name.into(), generator: Some(generator), mirror: None }))?;
        insert_fragments(&mut e, group, frags, &m, true)?;
        let c = e.finish();
        self.apply(&format!("Create {name}"), c);
        self.selection = BTreeSet::from([group]);
        Ok(group)
    }

    /// Replaces a generator group's contents with new output for `generator`'s parameters.
    pub fn regenerate(&mut self, group: NodeId, generator: Generator, frags: Vec<Fragment>, coalesce: Option<&str>) -> Result<(), EngineError> {
        let Some(node) = self.map.get(group) else { return Err(mapcraft_map::EditError::Missing(group).into()) };
        let NodeKind::Group(mut g) = node.kind.clone() else { return Ok(()) };
        let m = generator_transform(&generator);
        let mut e = self.map.edit();
        for child in &node.children {
            e.remove(*child)?;
        }
        g.generator = Some(generator);
        e.set_kind(group, NodeKind::Group(g))?;
        insert_fragments(&mut e, group, frags, &m, true)?;
        let c = e.finish();
        self.apply_coalesced("Edit generator", c, coalesce);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use mapcraft_geom::Aabb;
    use mapcraft_map::{Brush, TexDefaults};
    use serde_json::json;

    use super::*;

    fn block(size: f64) -> Vec<Fragment> {
        let b = Brush::cuboid(Aabb::new(DVec3::ZERO, DVec3::splat(size)), &TexDefaults::default());
        vec![Fragment { kind: NodeKind::Brush(b), children: Vec::new() }]
    }

    fn bounds(s: &Session, group: NodeId) -> Aabb {
        let child = s.map.get(group).unwrap().children[0];
        s.map.get(child).unwrap().kind.brush().unwrap().bounds().unwrap()
    }

    #[test]
    fn generators_follow_their_group() {
        let mut s = Session::default();
        let g = Generator {
            plugin: "surfsup".into(),
            command: "bhop".into(),
            params: json!({"size": 32}),
            origin: DVec3::new(100.0, 0.0, 0.0),
            angles: DVec3::ZERO,
        };
        let group = s.create_generator("Bhop", g.clone(), block(32.0)).unwrap();
        assert_eq!(bounds(&s, group).min, DVec3::new(100.0, 0.0, 0.0));

        s.translate_selection(DVec3::new(0.0, 64.0, 0.0), None).unwrap();
        let moved = s.map.get(group).unwrap().kind.clone();
        let NodeKind::Group(Group { generator: Some(stored), .. }) = moved else { panic!() };
        assert_eq!(stored.origin, DVec3::new(100.0, 64.0, 0.0));

        let edited = Generator { params: json!({"size": 64}), ..stored };
        for _ in 0..3 {
            s.regenerate(group, edited.clone(), block(64.0), Some("gen")).unwrap();
        }
        assert_eq!(bounds(&s, group), Aabb::new(DVec3::new(100.0, 64.0, 0.0), DVec3::new(164.0, 128.0, 64.0)));
        assert_eq!(s.map.get(group).unwrap().children.len(), 1);
        s.undo();
        assert_eq!(bounds(&s, group).size(), DVec3::splat(32.0), "a run of edits undoes in one step");

        let ids = s.insert_output("Run", block(16.0), DVec3::splat(8.0)).unwrap();
        assert_eq!(s.map.get(ids[0]).unwrap().kind.brush().unwrap().bounds().unwrap().min, DVec3::splat(8.0));
    }
}
