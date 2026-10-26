use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use mapcraft_geom::{DMat4, DQuat, DVec3};
use mapcraft_map::{Edit, FaceId, Group, Map, NodeId, NodeKind, Prefab, ROOT};

use crate::{EngineError, Session};

const MAX_DEPTH: usize = 16;

/// Angles (pitch, yaw, roll in degrees, Source order) and an origin as a transform.
pub fn placement(origin: DVec3, angles: DVec3) -> DMat4 {
    let r = DQuat::from_rotation_z(angles.y.to_radians()) * DQuat::from_rotation_y(angles.x.to_radians()) * DQuat::from_rotation_x(angles.z.to_radians());
    DMat4::from_rotation_translation(r, origin)
}

pub fn prefab_transform(p: &Prefab) -> DMat4 {
    placement(p.origin, p.angles)
}

fn fmt_vec(v: DVec3) -> String {
    let f = |x: f64| {
        let r = (x * 1e4).round() / 1e4;
        if r == r.trunc() { format!("{r:.0}") } else { format!("{r}") }
    };
    format!("{} {} {}", f(v.x), f(v.y), f(v.z))
}

/// Replaces the prefab instance `id` with a group holding the prefab's contents, moved into place.
/// Nested instances are expanded too, up to a fixed depth; a prefab that contains itself stops
/// with an error. Returns the new node ids.
pub fn expand_instance(e: &mut Edit, id: NodeId, load: &mut dyn FnMut(&str) -> Option<Map>, chain: &[String]) -> Result<Vec<NodeId>, EngineError> {
    let Some(NodeKind::Prefab(p)) = e.node(id).map(|n| n.kind.clone()) else { return Ok(Vec::new()) };
    if chain.len() >= MAX_DEPTH || chain.contains(&p.path) {
        return Err(EngineError::Prefab(format!("{} contains itself", p.path)));
    }
    let src = load(&p.path).ok_or_else(|| EngineError::Prefab(format!("cannot load {}", p.path)))?;
    // Mirror groups come in with their halves made real, since the instance's angles can turn the plane off the axes.
    let (src, _) = mapcraft_map::mirror::expand(&src);
    e.set_kind(id, NodeKind::Group(Group { name: p.path.clone(), ..Group::default() }))?;
    let m = prefab_transform(&p);
    let mut created = Vec::new();
    let mut ids: BTreeMap<NodeId, NodeId> = BTreeMap::new();
    let roots = src.get(ROOT).map(|n| n.children.clone()).unwrap_or_default();
    let mut stack: Vec<(NodeId, NodeId)> = roots.into_iter().rev().map(|c| (c, id)).collect();
    let mut nested = Vec::new();
    while let Some((src_id, parent)) = stack.pop() {
        let Some(node) = src.get(src_id) else { continue };
        let kind = match node.kind.clone() {
            NodeKind::Brush(b) => {
                let mut b = b.transformed(&m, true)?;
                b.faces.iter_mut().for_each(|f| f.id = FaceId(0));
                NodeKind::Brush(b)
            }
            NodeKind::Entity(mut ent) if node.children.is_empty() => {
                if let Some(o) = ent.origin() {
                    ent.set("origin", fmt_vec(m.transform_point3(o)));
                }
                NodeKind::Entity(ent)
            }
            NodeKind::Prefab(mut inner) => {
                inner.origin = m.transform_point3(inner.origin);
                inner.angles += p.angles;
                NodeKind::Prefab(inner)
            }
            NodeKind::Layer(l) => NodeKind::Group(Group { name: l.name, ..Group::default() }),
            NodeKind::Group(g) => NodeKind::Group(Group { mirror: None, curve: None, ..g }),
            NodeKind::Path(path) => NodeKind::Path(path.transformed(&m)),
            NodeKind::World(_) => continue,
            other => other,
        };
        let is_prefab = matches!(kind, NodeKind::Prefab(_));
        let new = e.insert(parent, kind)?;
        ids.insert(src_id, new);
        created.push(new);
        if is_prefab {
            nested.push(new);
        }
        stack.extend(node.children.iter().rev().map(|c| (*c, new)));
    }
    for o in &p.overrides {
        let Some(target) = ids.get(&o.node) else { continue };
        let Some(NodeKind::Entity(mut ent)) = e.node(*target).map(|n| n.kind.clone()) else { continue };
        match &o.value {
            Some(v) => ent.set(o.key.clone(), v.clone()),
            None => {
                ent.remove(&o.key);
            }
        }
        e.set_kind(*target, NodeKind::Entity(ent))?;
    }
    let mut chain = chain.to_vec();
    chain.push(p.path.clone());
    for n in nested {
        created.extend(expand_instance(e, n, load, &chain)?);
    }
    Ok(created)
}

/// `map` with mirror halves and prefab instances expanded, each added node's owner in `map` (its instance or mirrored original), and the instances that failed.
pub fn flatten(map: &Map, load: &mut dyn FnMut(&str) -> Option<Map>) -> (Map, BTreeMap<NodeId, NodeId>, Vec<String>) {
    let (mut out, mut owner) = mapcraft_map::mirror::expand(map);
    let instances: Vec<NodeId> = out.nodes().filter(|(_, n)| matches!(n.kind, NodeKind::Prefab(_))).map(|(id, _)| id).collect();
    let mut problems = Vec::new();
    for id in instances {
        let mut e = out.edit();
        match expand_instance(&mut e, id, load, &[]) {
            Ok(created) => {
                let instance = owner.get(&id).copied().unwrap_or(id);
                owner.extend(created.into_iter().map(|c| (c, instance)));
                let c = e.finish();
                out.apply(c);
            }
            Err(err) => problems.push(err.to_string()),
        }
    }
    (out, owner, problems)
}

impl Session {
    /// Writes the selection to `path` as its own map, centered on the grid point nearest its
    /// middle, and replaces it with an instance of that file (Godot's Save Branch as Scene).
    pub fn save_selection_as_prefab(&mut self, path: &Path) -> Result<NodeId, EngineError> {
        let center = mapcraft_geom::snap(self.selection_center().ok_or(EngineError::NothingSelected)?, self.grid);
        let text = self.copy_selection()?;
        let mut prefab = Session::new(Map::new());
        prefab.sdk = self.sdk.clone();
        prefab.texture_lock = self.texture_lock;
        prefab.paste(&text, -center)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(mapcraft_format::FormatError::Io)?;
        }
        prefab.save_as(path)?;

        let stored = self.relative_path(path);
        let roots: Vec<NodeId> = self.selection.iter().copied().filter(|id| !self.map.ancestors(*id).iter().any(|a| self.selection.contains(a))).collect();
        let parent = roots.first().and_then(|id| self.map.get(*id)?.parent).unwrap_or(ROOT);
        let mut e = self.map.edit();
        for id in roots {
            e.remove(id)?;
        }
        let id = e.insert(parent, NodeKind::Prefab(Prefab { path: stored, origin: center, ..Prefab::default() }))?;
        let c = e.finish();
        self.apply("Save as prefab", c);
        self.selection = BTreeSet::from([id]);
        Ok(id)
    }

    /// A path to store in this map: relative to the map's folder when the map has been saved.
    pub fn relative_path(&self, path: &Path) -> String {
        let rel = self.path.as_ref().and_then(|m| m.parent()).and_then(|dir| path.strip_prefix(dir).ok());
        rel.unwrap_or(path).to_string_lossy().replace('\\', "/")
    }

    pub fn insert_prefab(&mut self, path: &str, origin: DVec3) -> Result<NodeId, EngineError> {
        let mut e = self.map.edit();
        let id = e.insert(self.active_parent, NodeKind::Prefab(Prefab { path: path.into(), origin, ..Prefab::default() }))?;
        let c = e.finish();
        self.apply("Insert prefab", c);
        self.selection = BTreeSet::from([id]);
        Ok(id)
    }

    /// Turns the selected prefab instances into ordinary nodes (Godot's Make Local).
    pub fn make_local(&mut self, load: &mut dyn FnMut(&str) -> Option<Map>) -> Result<(), EngineError> {
        let ids: Vec<NodeId> = self.selection.iter().copied().filter(|id| self.map.get(*id).is_some_and(|n| matches!(n.kind, NodeKind::Prefab(_)))).collect();
        if ids.is_empty() {
            return Err(EngineError::NothingSelected);
        }
        let mut e = self.map.edit();
        for id in &ids {
            expand_instance(&mut e, *id, load, &[])?;
        }
        let c = e.finish();
        self.apply("Make local", c);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use mapcraft_geom::Aabb;

    use super::*;

    #[test]
    fn save_instance_flatten_make_local() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Session::default();
        s.path = Some(dir.path().join("level.mcmap"));
        let b = s.create_brush(Aabb::new(DVec3::new(64.0, 0.0, 0.0), DVec3::new(128.0, 64.0, 64.0))).unwrap();
        let ent = s.create_point_entity("info_target", DVec3::new(96.0, 32.0, 0.0), vec![("targetname".into(), "t".into())]).unwrap();
        s.selection = BTreeSet::from([b, ent]);
        let inst = s.save_selection_as_prefab(&dir.path().join("prefabs/pillar.mcmap")).unwrap();
        let Some(NodeKind::Prefab(p)) = s.map.get(inst).map(|n| n.kind.clone()) else { panic!() };
        assert_eq!(p.path, "prefabs/pillar.mcmap");
        assert_eq!(p.origin, DVec3::new(96.0, 32.0, 32.0));

        let dir_path = dir.path().to_path_buf();
        let mut load = |rel: &str| mapcraft_format::load(&dir_path.join(rel)).ok().map(|f| f.map);
        let (flat, owner, problems) = flatten(&s.map, &mut load);
        assert!(problems.is_empty());
        let brush_bounds: Vec<Aabb> = flat.nodes().filter_map(|(_, n)| n.kind.brush()?.bounds().ok()).collect();
        assert_eq!(brush_bounds, vec![Aabb::new(DVec3::new(64.0, 0.0, 0.0), DVec3::new(128.0, 64.0, 64.0))]);
        assert!(owner.values().all(|o| *o == inst));
        assert_eq!(owner.len(), 2);

        s.make_local(&mut load).unwrap();
        assert!(s.map.nodes().all(|(_, n)| !matches!(n.kind, NodeKind::Prefab(_))));
        assert_eq!(s.map.nodes().filter(|(_, n)| n.kind.brush().is_some()).count(), 1);
    }

    #[test]
    fn cycles_are_refused() {
        let mut inner = Map::new();
        let mut e = inner.edit();
        e.insert(ROOT, NodeKind::Prefab(Prefab { path: "self.mcmap".into(), ..Prefab::default() })).unwrap();
        inner.apply(e.finish());
        let files = HashMap::from([("self.mcmap".to_string(), inner.clone())]);
        let (_, _, problems) = flatten(&inner, &mut |p| files.get(p).cloned());
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("contains itself"));
    }
}
