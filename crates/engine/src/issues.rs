use std::collections::{BTreeMap, BTreeSet};

use mapcraft_fgd::GameData;
use mapcraft_geom::EPSILON;
use mapcraft_map::{FaceId, Map, NodeId, NodeKind};

use crate::{EngineError, Session};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Issue {
    pub node: NodeId,
    pub face: Option<FaceId>,
    pub severity: Severity,
    pub message: String,
    pub fix: Option<Fix>,
}

/// What Fix does to the issue's node.
#[derive(Clone, Debug, PartialEq)]
pub enum Fix {
    Remove,
    RemoveFace,
    ResetTextureAxes,
    ClearProperty(String),
    RemoveConnection(usize),
}

/// Keys that name another entity when no FGD says otherwise.
const LINK_KEYS: &[&str] = &["target", "parentname", "filtername", "landmark"];

/// Problems that make a map fail to compile or play, mirror halves included; FGD checks run when `game` is given.
pub fn validate(map: &Map, game: Option<&GameData>) -> Vec<Issue> {
    let (map, derived) = mapcraft_map::mirror::expand(map);
    let map = &map;
    let mut out = Vec::new();
    let mut push = |node, face, severity, message: String, fix| out.push(Issue { node, face, severity, message, fix });
    let names: BTreeSet<&str> = map.nodes().filter_map(|(_, n)| n.kind.entity()?.get("targetname")).filter(|s| !s.is_empty()).collect();
    for (id, node) in map.nodes() {
        match &node.kind {
            NodeKind::Brush(b) => {
                for (i, f) in b.faces.iter().enumerate() {
                    let Some(plane) = f.plane() else {
                        push(id, Some(f.id), Severity::Error, format!("face {i} has collinear points"), Some(Fix::RemoveFace));
                        continue;
                    };
                    if b.faces.iter().take(i).filter_map(|g| g.plane()).any(|g| g.approx_eq(&plane, EPSILON)) {
                        push(id, Some(f.id), Severity::Warning, format!("face {i} duplicates another face's plane"), Some(Fix::RemoveFace));
                    }
                    let bad_axis = |a: &mapcraft_map::TexAxis| a.scale == 0.0 || !a.scale.is_finite() || a.axis.length() < 1e-9;
                    let u_in_plane = f.u.axis.cross(f.v.axis).length() > 1e-6 && f.u.axis.cross(f.v.axis).normalize_or_zero().dot(plane.normal).abs() > 1e-3;
                    if bad_axis(&f.u) || bad_axis(&f.v) || !u_in_plane {
                        push(id, Some(f.id), Severity::Warning, format!("face {i} has invalid texture axes"), Some(Fix::ResetTextureAxes));
                    }
                }
                if let Err(e) = b.solid() {
                    push(id, None, Severity::Error, format!("invalid brush: {e}"), Some(Fix::Remove));
                }
            }
            NodeKind::Entity(e) => {
                if e.classname.is_empty() {
                    push(id, None, Severity::Error, "entity has no classname".into(), Some(Fix::Remove));
                    continue;
                }
                let class = game.and_then(|g| g.class(&e.classname));
                if let (Some(_), None) = (game, &class) {
                    push(id, None, Severity::Warning, format!("`{}` is not defined in the SDK's FGDs", e.classname), None);
                }
                if let Some(c) = &class {
                    if c.kind.is_solid() && node.children.is_empty() {
                        push(id, None, Severity::Error, format!("brush entity `{}` has no brushes", e.classname), Some(Fix::Remove));
                    }
                    if !c.kind.is_solid() && !node.children.is_empty() {
                        push(id, None, Severity::Error, format!("point entity `{}` contains brushes", e.classname), None);
                    }
                }
                for (key, value) in &e.properties {
                    let is_link = match &class {
                        Some(c) => c.property(key).is_some_and(|p| p.ty == "target_destination" || p.ty == "filterclass"),
                        None => LINK_KEYS.contains(&key.as_str()),
                    };
                    if is_link && !value.is_empty() && !value.contains('*') && !value.starts_with('!') && !names.contains(value.as_str()) {
                        push(
                            id,
                            None,
                            Severity::Warning,
                            format!("`{key}` points at `{value}`, which no entity is named"),
                            Some(Fix::ClearProperty(key.clone())),
                        );
                    }
                }
                for (i, c) in e.connections.iter().enumerate() {
                    if !c.target.starts_with('!') && !c.target.contains('*') && !names.contains(c.target.as_str()) {
                        let msg = format!("output {} targets `{}`, which no entity is named", c.output, c.target);
                        push(id, None, Severity::Warning, msg, Some(Fix::RemoveConnection(i)));
                    }
                }
            }
            NodeKind::Group(g) if node.children.is_empty() && g.generator.is_none() => {
                push(id, None, Severity::Warning, "empty group".into(), Some(Fix::Remove));
            }
            _ => {}
        }
    }
    // A derived node's problems are its original's, already reported.
    out.retain(|i| !derived.contains_key(&i.node));
    out.sort_by(|a, b| b.severity.cmp(&a.severity).then(a.node.cmp(&b.node)));
    out
}

impl Session {
    /// Applies the fixes of `issues` as one undo step; returns how many issues were fixed.
    pub fn fix_issues(&mut self, issues: &[Issue]) -> Result<usize, EngineError> {
        let mut by_node: BTreeMap<NodeId, Vec<&Issue>> = BTreeMap::new();
        for i in issues.iter().filter(|i| i.fix.is_some()) {
            by_node.entry(i.node).or_default().push(i);
        }
        let mut e = self.map.edit();
        let mut fixed = 0;
        let mut removals = Vec::new();
        for (id, list) in by_node {
            if list.iter().any(|i| i.fix == Some(Fix::Remove)) {
                removals.push(id);
                fixed += list.len();
                continue;
            }
            let Some(mut kind) = e.node(id).map(|n| n.kind.clone()) else { continue };
            let mut conns: Vec<usize> = Vec::new();
            for i in &list {
                match (&mut kind, &i.fix) {
                    (NodeKind::Brush(b), Some(Fix::RemoveFace)) => b.faces.retain(|f| Some(f.id) != i.face),
                    (NodeKind::Brush(b), Some(Fix::ResetTextureAxes)) => {
                        for f in b.faces.iter_mut().filter(|f| Some(f.id) == i.face) {
                            f.align_to_face();
                            for a in [&mut f.u, &mut f.v] {
                                if a.scale == 0.0 || !a.scale.is_finite() {
                                    a.scale = 1.0;
                                }
                            }
                        }
                    }
                    (NodeKind::Entity(ent), Some(Fix::ClearProperty(k))) => {
                        ent.remove(k);
                    }
                    (NodeKind::Entity(_), Some(Fix::RemoveConnection(c))) => conns.push(*c),
                    _ => continue,
                }
                fixed += 1;
            }
            if let NodeKind::Entity(ent) = &mut kind {
                conns.sort_unstable();
                conns.dedup();
                for c in conns.into_iter().rev() {
                    if c < ent.connections.len() {
                        ent.connections.remove(c);
                    }
                }
            }
            e.set_kind(id, kind)?;
        }
        for id in removals {
            if e.node(id).is_some() {
                e.remove(id)?;
            }
        }
        let c = e.finish();
        self.apply("Fix problems", c);
        Ok(fixed)
    }
}

#[cfg(test)]
mod tests {
    use mapcraft_geom::{Aabb, DVec3};
    use mapcraft_map::{Brush, Entity, ROOT, TexDefaults};

    use super::*;

    #[test]
    fn finds_problems() {
        let mut map = Map::new();
        let mut e = map.edit();
        let mut bad = Brush::cuboid(Aabb::new(DVec3::ZERO, DVec3::splat(64.0)), &TexDefaults::default());
        bad.faces[0].u.scale = 0.0;
        bad.faces.pop();
        let bad_id = e.insert(ROOT, NodeKind::Brush(bad)).unwrap();
        let mut tp = Entity::new("trigger_teleport");
        tp.set("target", "dest");
        let tp_id = e.insert(ROOT, NodeKind::Entity(tp)).unwrap();
        let mut start = Entity::new("info_player_start");
        start.set("targetname", "spawn");
        e.insert(ROOT, NodeKind::Entity(start)).unwrap();
        map.apply(e.finish());

        let issues = validate(&map, None);
        assert!(issues.iter().any(|i| i.node == bad_id && i.severity == Severity::Error));
        assert!(issues.iter().any(|i| i.node == bad_id && i.message.contains("texture axes")));
        assert!(issues.iter().any(|i| i.node == tp_id && i.message.contains("`dest`")));

        let mut s = Session::new(map.clone());
        assert!(s.fix_issues(&issues).unwrap() >= 3);
        assert!(s.map.get(bad_id).is_none());
        assert!(s.map.get(tp_id).unwrap().kind.entity().unwrap().get("target").is_none());
        assert_eq!(validate(&s.map, None), vec![]);

        let game = GameData::parse_str("@SolidClass = trigger_teleport [ target(target_destination) : \"Dest\" ]\n@PointClass = info_player_start []");
        let issues = validate(&map, Some(&game));
        assert!(issues.iter().any(|i| i.node == tp_id && i.message.contains("has no brushes")));
        assert!(!issues.iter().any(|i| i.message.contains("not defined")));
    }

    #[test]
    fn clean_cube_has_no_issues() {
        let mut map = Map::new();
        let mut e = map.edit();
        e.insert(ROOT, NodeKind::Brush(Brush::cuboid(Aabb::new(DVec3::ZERO, DVec3::splat(64.0)), &TexDefaults::default()))).unwrap();
        map.apply(e.finish());
        assert_eq!(validate(&map, None), vec![]);
    }
}
