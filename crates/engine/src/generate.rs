use std::collections::BTreeSet;

use mapcraft_format::Fragment;
use mapcraft_geom::{DMat4, DVec3};
use mapcraft_map::{Changes, Generator, Group, NodeId, NodeKind};

use crate::ops::insert_fragments;
use crate::{EngineError, Session};

/// Where a generator's output goes.
pub fn generator_transform(g: &Generator) -> DMat4 {
    crate::prefab::placement(g.origin, g.angles) * g.stretch.as_deref().map_or(DMat4::IDENTITY, |s| DMat4::from_mat3(*s))
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
        // An imported `worldspawn` point entity carries the world's keys; the open map's own keys win.
        let (worlds, frags): (Vec<Fragment>, Vec<Fragment>) = frags.into_iter().partition(|f| {
            f.children.is_empty() && f.kind.entity().is_some_and(|x| matches!(f.kind, NodeKind::Entity(_)) && x.classname.eq_ignore_ascii_case("worldspawn"))
        });
        if let Some(NodeKind::World(mut world)) = e.node(mapcraft_map::ROOT).map(|n| n.kind.clone()) {
            let before = world.properties.len();
            for (key, value) in worlds.iter().filter_map(|f| f.kind.entity()).flat_map(|x| x.properties.iter()) {
                if key != "classname" && key != "origin" && world.get(key).is_none() {
                    world.set(key, value);
                }
            }
            if world.properties.len() != before {
                e.set_kind(mapcraft_map::ROOT, NodeKind::World(world))?;
            }
        }
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
        let group = e.insert(self.active_parent, NodeKind::Group(Group { name: name.into(), generator: Some(generator), ..Group::default() }))?;
        insert_fragments(&mut e, group, frags, &m, true)?;
        let c = e.finish();
        self.apply(&format!("Create {name}"), c);
        self.selection = BTreeSet::from([group]);
        Ok(group)
    }

    /// Replaces a generator group's contents with new output for `generator`'s parameters.
    pub fn regenerate(&mut self, group: NodeId, generator: Generator, frags: Vec<Fragment>, coalesce: Option<&str>) -> Result<(), EngineError> {
        let c = self.regeneration(group, generator, frags)?;
        self.apply_coalesced("Edit generator", c, coalesce);
        Ok(())
    }

    /// Like [`Session::regenerate`] for a group in [`Session::stretched`], as part of the edit that stretched it.
    pub fn refit(&mut self, group: NodeId, generator: Generator, frags: Vec<Fragment>) -> Result<(), EngineError> {
        let c = self.regeneration(group, generator, frags)?;
        self.amend("Edit generator", c);
        Ok(())
    }

    fn regeneration(&self, group: NodeId, generator: Generator, frags: Vec<Fragment>) -> Result<Changes, EngineError> {
        let Some(node) = self.map.get(group) else { return Err(mapcraft_map::EditError::Missing(group).into()) };
        let mut e = self.map.edit();
        let NodeKind::Group(mut g) = node.kind.clone() else { return Ok(e.finish()) };
        let m = generator_transform(&generator);
        for child in &node.children {
            e.remove(*child)?;
        }
        g.generator = Some(generator);
        e.set_kind(group, NodeKind::Group(g))?;
        insert_fragments(&mut e, group, frags, &m, true)?;
        Ok(e.finish())
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

    fn generator(s: &Session, group: NodeId) -> Generator {
        let NodeKind::Group(Group { generator: Some(g), .. }) = s.map.get(group).unwrap().kind.clone() else { panic!() };
        g
    }

    fn near(a: Aabb, min: DVec3, max: DVec3) -> bool {
        a.min.abs_diff_eq(min, 1e-9) && a.max.abs_diff_eq(max, 1e-9)
    }

    /// A resize the parameters do not hold stays on the group, so editing them keeps it; folding it into them is part of the resize.
    #[test]
    fn resizes_survive_parameter_edits() {
        use mapcraft_geom::DMat3;
        let mut s = Session::default();
        let g = Generator {
            plugin: "surfsup".into(),
            command: "bhop".into(),
            params: json!({"size": [32, 32, 32]}),
            origin: DVec3::ZERO,
            angles: DVec3::new(0.0, 90.0, 0.0),
            stretch: None,
        };
        let group = s.create_generator("Bhop", g, block(32.0)).unwrap();
        let wide = DMat4::from_scale(DVec3::new(2.0, 1.0, 1.0));
        s.transform_selection(&wide, "Scale", None).unwrap();
        assert_eq!(s.stretched, BTreeSet::from([group]));
        let stretched = generator(&s, group);
        assert!(stretched.stretch.as_deref().unwrap().abs_diff_eq(DMat3::from_diagonal(DVec3::new(1.0, 2.0, 1.0)), 1e-9), "world X is the turned output's Y");

        let tall = Brush::cuboid(Aabb::new(DVec3::ZERO, DVec3::new(32.0, 64.0, 32.0)), &TexDefaults::default());
        let fitted = Generator { params: json!({"size": [32, 64, 32]}), stretch: None, ..stretched };
        s.refit(group, fitted, vec![Fragment { kind: NodeKind::Brush(tall.clone()), children: Vec::new() }]).unwrap();
        assert!(near(bounds(&s, group), DVec3::new(-64.0, 0.0, 0.0), DVec3::new(0.0, 32.0, 32.0)));
        s.undo();
        assert!(near(bounds(&s, group), DVec3::new(-32.0, 0.0, 0.0), DVec3::new(0.0, 32.0, 32.0)), "one undo takes back the resize and its refit");
        assert_eq!(generator(&s, group).params, json!({"size": [32, 32, 32]}));
        s.redo();

        s.transform_selection(&wide, "Scale", None).unwrap();
        s.regenerate(group, generator(&s, group), vec![Fragment { kind: NodeKind::Brush(tall.clone()), children: Vec::new() }], None).unwrap();
        assert!(near(bounds(&s, group), DVec3::new(-128.0, 0.0, 0.0), DVec3::new(0.0, 32.0, 32.0)), "an edit keeps the resize");

        s.rotate_selection(DVec3::X, 90.0).unwrap();
        let turned = bounds(&s, group);
        s.regenerate(group, generator(&s, group), vec![Fragment { kind: NodeKind::Brush(tall), children: Vec::new() }], None).unwrap();
        assert!(near(bounds(&s, group), turned.min, turned.max), "and so does a tilt");
    }

    /// An import's `worldspawn` fills in the world's keys without replacing the open map's own.
    #[test]
    fn imports_give_the_world_their_keys() {
        let mut s = Session::default();
        let mut world = s.map.get(mapcraft_map::ROOT).unwrap().kind.entity().unwrap().clone();
        world.set("message", "Mine");
        let mut e = s.map.edit();
        e.set_kind(mapcraft_map::ROOT, NodeKind::World(world)).unwrap();
        s.map.apply(e.finish());
        let mut imported = mapcraft_map::Entity::new("worldspawn");
        imported.set("message", "Theirs");
        imported.set("_sunlight", "300");
        let mut frags = block(32.0);
        frags.push(Fragment { kind: NodeKind::Entity(imported), children: Vec::new() });
        let layer = s.import_layer("map", frags).unwrap();
        let world = s.map.get(mapcraft_map::ROOT).unwrap().kind.entity().unwrap();
        assert_eq!((world.get("message"), world.get("_sunlight")), (Some("Mine"), Some("300")));
        assert_eq!(s.map.get(layer).unwrap().children.len(), 1, "the world's keys are not placed as an entity");
        s.undo();
        assert!(s.map.get(mapcraft_map::ROOT).unwrap().kind.entity().unwrap().get("_sunlight").is_none(), "one undo takes them back");
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
            stretch: None,
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
