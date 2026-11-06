//! Converting the map's entities and materials to another SDK's by that SDK's rules.

use std::collections::BTreeSet;

use mapcraft_map::{NodeId, NodeKind};
use mapcraft_sdk::Conversion;

use crate::{EngineError, Session};

/// What a conversion changed, for the status line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Converted {
    pub entities: usize,
    pub to_world: usize,
    pub dropped: usize,
    pub faces: usize,
}

impl Session {
    /// Applies `rules` to every entity and face under `within` (the whole map from `ROOT`) as one undo step labelled `label`.
    pub fn convert(&mut self, rules: &Conversion, label: &str, within: NodeId) -> Result<Converted, EngineError> {
        let mut e = self.map.edit();
        let mut done = Converted::default();
        let mut singles: BTreeSet<String> = BTreeSet::new();
        let ids: Vec<NodeId> = self.map.subtree(within).into_iter().collect();
        for id in ids {
            let Some(node) = e.node(id).cloned() else { continue };
            match node.kind {
                NodeKind::Entity(mut ent) => {
                    let Some(rule) = rules.classes.get(&ent.classname).or_else(|| rules.classes.get(&ent.classname.to_ascii_lowercase())) else {
                        continue;
                    };
                    let class = rule.to.clone().unwrap_or_else(|| ent.classname.clone());
                    if rule.drop || (rule.single && !singles.insert(class.clone())) {
                        e.remove(id)?;
                        done.dropped += 1;
                        continue;
                    }
                    if rule.world {
                        let Some(parent) = node.parent else { continue };
                        for child in &node.children {
                            e.reparent(*child, parent, None)?;
                        }
                        e.remove(id)?;
                        done.to_world += 1;
                        continue;
                    }
                    ent.classname = class;
                    for (from, to) in &rule.keys {
                        if let Some(value) = ent.remove(from) {
                            ent.set(to.clone(), value);
                        }
                    }
                    for key in &rule.remove {
                        ent.remove(key);
                    }
                    for (key, value) in &rule.set {
                        ent.set(key.clone(), value.clone());
                    }
                    e.set_kind(id, NodeKind::Entity(ent))?;
                    done.entities += 1;
                }
                NodeKind::Brush(mut b) if !rules.materials.is_empty() => {
                    let mut changed = false;
                    for face in &mut b.faces {
                        if let Some(to) = rules.materials.get(&face.material).or_else(|| rules.materials.get(&face.material.to_ascii_lowercase())) {
                            face.material = to.clone();
                            changed = true;
                            done.faces += 1;
                        }
                    }
                    if changed {
                        e.set_kind(id, NodeKind::Brush(b))?;
                    }
                }
                _ => {}
            }
        }
        let c = e.finish();
        self.apply(label, c);
        self.selection.clear();
        Ok(done)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use mapcraft_geom::{Aabb, DVec3};
    use mapcraft_map::{Entity, NodeKind, ROOT};
    use mapcraft_sdk::Conversion;

    use crate::Session;

    fn entity(s: &mut Session, class: &str, keys: &[(&str, &str)]) -> mapcraft_map::NodeId {
        let mut ent = Entity::new(class);
        for (k, v) in keys {
            ent.set(*k, *v);
        }
        let mut e = s.map.edit();
        let id = e.insert(ROOT, NodeKind::Entity(ent)).unwrap();
        let c = e.finish();
        s.apply("add", c);
        id
    }

    #[test]
    fn renames_merges_drops_and_keeps_one_spawn() {
        let rules = Conversion::parse(
            "[classes.trigger_teleport]\nto = \"teleport_zone\"\nkeys = { target = \"next_stage\" }\nset = { kill_velocity = \"1\" }\nremove = [\"spawnflags\"]\n\n\
             [classes.func_wall]\nworld = true\n\n[classes.func_buyzone]\ndrop = true\n\n\
             [classes.info_player_terrorist]\nto = \"info_player_start\"\nsingle = true\n\n\
             [classes.info_player_counterterrorist]\nto = \"info_player_start\"\nsingle = true\n\n\
             [materials]\n\"tools/toolsclip\" = \"tools/clip\"\n",
            Path::new("x.toml"),
        )
        .unwrap();
        let mut s = Session::default();
        let tp = entity(&mut s, "trigger_teleport", &[("target", "dest"), ("spawnflags", "1")]);
        let wall = entity(&mut s, "func_wall", &[]);
        let brush = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
        let mut e = s.map.edit();
        e.reparent(brush, wall, None).unwrap();
        if let Some(NodeKind::Brush(mut b)) = e.node(brush).map(|n| n.kind.clone()) {
            b.faces[0].material = "TOOLS/TOOLSCLIP".into();
            e.set_kind(brush, NodeKind::Brush(b)).unwrap();
        }
        let c = e.finish();
        s.apply("setup", c);
        entity(&mut s, "func_buyzone", &[]);
        let first = entity(&mut s, "info_player_terrorist", &[("origin", "1 2 3")]);
        entity(&mut s, "info_player_counterterrorist", &[]);
        entity(&mut s, "info_player_terrorist", &[]);

        let done = s.convert(&rules, "Convert to SurfsUp", ROOT).unwrap();
        assert_eq!((done.entities, done.to_world, done.dropped, done.faces), (2, 1, 3, 1));
        let NodeKind::Entity(t) = &s.map.get(tp).unwrap().kind else { panic!() };
        assert_eq!(
            (t.classname.as_str(), t.get("next_stage"), t.get("target"), t.get("kill_velocity"), t.get("spawnflags")),
            ("teleport_zone", Some("dest"), None, Some("1"), None)
        );
        assert!(s.map.get(wall).is_none() && s.map.get(brush).is_some_and(|n| n.parent == Some(ROOT)), "func_wall's brush joins the world");
        let NodeKind::Brush(b) = &s.map.get(brush).unwrap().kind else { panic!() };
        assert_eq!(b.faces[0].material, "tools/clip");
        let spawns: Vec<_> = s.map.nodes().filter(|(_, n)| n.kind.entity().is_some_and(|e| e.classname == "info_player_start")).map(|(id, _)| id).collect();
        assert_eq!(spawns, [first], "one spawn point, the first");
        assert!(s.map.nodes().all(|(_, n)| n.kind.entity().is_none_or(|e| e.classname != "func_buyzone")));
        s.undo();
        assert!(
            s.map.get(wall).is_some() && s.map.get(tp).is_some_and(|n| n.kind.entity().is_some_and(|e| e.classname == "trigger_teleport")),
            "one undo step"
        );
    }
}
