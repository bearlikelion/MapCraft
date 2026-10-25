use std::collections::{BTreeMap, BTreeSet};

use mapcraft_geom::Aabb;

use crate::{Cordon, Entity, Extra, FaceId, Node, NodeId, NodeKind, Tag, TagId};

pub const ROOT: NodeId = NodeId(1);

/// Deeper trees than this are refused; it bounds every recursive walk.
const MAX_DEPTH: usize = 256;

#[derive(Clone, Debug, PartialEq)]
pub struct Map {
    nodes: BTreeMap<NodeId, Node>,
    pub tags: Vec<Tag>,
    pub cordons: Vec<Cordon>,
    pub extra: Extra,
    next_node: u64,
    next_face: u64,
}

/// A set of node writes; `None` removes the node. Applying one returns its inverse.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Changes {
    pub nodes: BTreeMap<NodeId, Option<Node>>,
    pub tags: Option<Vec<Tag>>,
    pub cordons: Option<Vec<Cordon>>,
    next_node: u64,
    next_face: u64,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.tags.is_none() && self.cordons.is_none()
    }

    /// Folds `later` into `self` so both apply as one step.
    pub fn merge(&mut self, later: Changes) {
        self.nodes.extend(later.nodes);
        if later.tags.is_some() {
            self.tags = later.tags;
        }
        if later.cordons.is_some() {
            self.cordons = later.cordons;
        }
        self.next_node = self.next_node.max(later.next_node);
        self.next_face = self.next_face.max(later.next_face);
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    #[error("node {0:?} does not exist")]
    Missing(NodeId),
    #[error("{child} cannot go inside {parent}")]
    BadParent { parent: String, child: String },
    #[error("the world node cannot be removed or moved")]
    Root,
    #[error("a node cannot move inside itself")]
    Cycle,
    #[error("the map tree is deeper than {MAX_DEPTH} levels")]
    TooDeep,
    #[error("invalid map: {0}")]
    Invalid(String),
}

impl Default for Map {
    fn default() -> Self {
        Self::new()
    }
}

impl Map {
    pub fn new() -> Self {
        let world = Node {
            parent: None,
            children: Vec::new(),
            kind: NodeKind::World(Entity::new("worldspawn")),
            tags: Vec::new(),
            hidden: false,
            locked: false,
            extra: Extra::new(),
        };
        Self { nodes: BTreeMap::from([(ROOT, world)]), tags: Vec::new(), cordons: Vec::new(), extra: Extra::new(), next_node: ROOT.0 + 1, next_face: 1 }
    }

    /// Builds a map from loaded nodes, checking the tree and giving fresh ids to unassigned or duplicate faces.
    pub fn from_nodes(mut nodes: BTreeMap<NodeId, Node>, tags: Vec<Tag>, extra: Extra) -> Result<Self, EditError> {
        match nodes.get(&ROOT) {
            Some(n) if matches!(n.kind, NodeKind::World(_)) && n.parent.is_none() => {}
            _ => return Err(EditError::Invalid("the root node must be the world".into())),
        }
        let mut seen = BTreeSet::new();
        let mut stack = vec![(ROOT, 0usize)];
        while let Some((id, depth)) = stack.pop() {
            if depth > MAX_DEPTH {
                return Err(EditError::TooDeep);
            }
            if !seen.insert(id) {
                return Err(EditError::Invalid(format!("node {} appears twice in the tree", id.0)));
            }
            let node = nodes.get(&id).ok_or(EditError::Missing(id))?;
            for c in &node.children {
                let child = nodes.get(c).ok_or(EditError::Missing(*c))?;
                if child.parent != Some(id) {
                    return Err(EditError::Invalid(format!("node {} has the wrong parent", c.0)));
                }
                if !node.kind.can_contain(&child.kind) {
                    return Err(bad_parent(&node.kind, &child.kind));
                }
                stack.push((*c, depth + 1));
            }
        }
        if seen.len() != nodes.len() {
            return Err(EditError::Invalid("some nodes are not reachable from the world".into()));
        }
        let next_node = nodes.keys().last().map_or(ROOT.0, |id| id.0).saturating_add(1);
        let mut faces = BTreeSet::new();
        let mut next_face = nodes.values().filter_map(|n| n.kind.brush()).flat_map(|b| &b.faces).map(|f| f.id.0).max().unwrap_or(0).saturating_add(1);
        for node in nodes.values_mut() {
            if let NodeKind::Brush(b) = &mut node.kind {
                for f in &mut b.faces {
                    if f.id.0 == 0 || !faces.insert(f.id) {
                        f.id = FaceId(next_face);
                        faces.insert(f.id);
                        next_face = next_face.saturating_add(1);
                    }
                }
            }
        }
        Ok(Self { nodes, tags, cordons: Vec::new(), extra, next_node, next_face })
    }

    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(&id)
    }

    pub fn nodes(&self) -> impl Iterator<Item = (NodeId, &Node)> {
        self.nodes.iter().map(|(id, n)| (*id, n))
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.len() <= 1
    }

    pub fn world(&self) -> &Entity {
        static EMPTY: std::sync::OnceLock<Entity> = std::sync::OnceLock::new();
        self.nodes.get(&ROOT).and_then(|n| n.kind.entity()).unwrap_or_else(|| EMPTY.get_or_init(Entity::default))
    }

    /// `id` and everything below it, parents before children.
    pub fn subtree(&self, id: NodeId) -> Vec<NodeId> {
        subtree(|i| self.nodes.get(&i), id)
    }

    pub fn ancestors(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut cur = self.nodes.get(&id).and_then(|n| n.parent);
        while let Some(p) = cur {
            if out.len() > MAX_DEPTH || out.contains(&p) {
                break;
            }
            out.push(p);
            cur = self.nodes.get(&p).and_then(|n| n.parent);
        }
        out
    }

    /// Nodes that are drawn and picked: not hidden, carrying no hidden tag, under no hidden
    /// parent, and while a cordon is active, touching one.
    pub fn visible_nodes(&self) -> BTreeSet<NodeId> {
        let hidden_tags: BTreeSet<TagId> = self.tags.iter().filter(|t| t.hidden).map(|t| t.id).collect();
        let cordons: Vec<Aabb> = self.cordons.iter().filter(|c| c.active).map(|c| c.bounds).collect();
        let outside = |n: &Node| {
            let inside = |b: &Aabb| cordons.iter().any(|c| c.intersects(b));
            match &n.kind {
                NodeKind::Brush(b) => b.bounds().is_ok_and(|b| !inside(&b)),
                NodeKind::Entity(e) if n.children.is_empty() => e.origin().is_some_and(|o| !inside(&Aabb::new(o, o))),
                NodeKind::Prefab(p) => !inside(&Aabb::new(p.origin, p.origin)),
                _ => false,
            }
        };
        let mut out = BTreeSet::new();
        let mut stack = vec![ROOT];
        while let Some(id) = stack.pop() {
            let Some(n) = self.nodes.get(&id) else { continue };
            if n.hidden || n.tags.iter().any(|t| hidden_tags.contains(t)) || (!cordons.is_empty() && outside(n)) || !out.insert(id) {
                continue;
            }
            stack.extend(n.children.iter().copied());
        }
        out
    }

    pub fn edit(&self) -> Edit<'_> {
        Edit { map: self, changes: Changes { next_node: self.next_node, next_face: self.next_face, ..Changes::default() } }
    }

    pub fn apply(&mut self, changes: Changes) -> Changes {
        let mut inverse = Changes::default();
        for (id, value) in changes.nodes {
            let old = match value {
                Some(n) => self.nodes.insert(id, n),
                None => self.nodes.remove(&id),
            };
            inverse.nodes.insert(id, old);
        }
        if let Some(tags) = changes.tags {
            inverse.tags = Some(std::mem::replace(&mut self.tags, tags));
        }
        if let Some(cordons) = changes.cordons {
            inverse.cordons = Some(std::mem::replace(&mut self.cordons, cordons));
        }
        // Ids only ever grow so undo and redo never hand out an id twice.
        self.next_node = self.next_node.max(changes.next_node);
        self.next_face = self.next_face.max(changes.next_face);
        inverse.next_node = self.next_node;
        inverse.next_face = self.next_face;
        inverse
    }
}

fn subtree<'a>(get: impl Fn(NodeId) -> Option<&'a Node>, id: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack = vec![(id, 0usize)];
    while let Some((i, depth)) = stack.pop() {
        let Some(n) = get(i) else { continue };
        if depth > MAX_DEPTH || out.contains(&i) {
            continue;
        }
        out.push(i);
        stack.extend(n.children.iter().rev().map(|c| (*c, depth + 1)));
    }
    out
}

fn bad_parent(parent: &NodeKind, child: &NodeKind) -> EditError {
    EditError::BadParent { parent: kind_name(parent).into(), child: kind_name(child).into() }
}

fn kind_name(k: &NodeKind) -> &'static str {
    match k {
        NodeKind::World(_) => "the world",
        NodeKind::Layer(_) => "a layer",
        NodeKind::Group(_) => "a group",
        NodeKind::Entity(_) => "an entity",
        NodeKind::Brush(_) => "a brush",
        NodeKind::Prefab(_) => "a prefab",
    }
}

/// Builds [`Changes`] against a map, reading its own pending writes first.
pub struct Edit<'a> {
    map: &'a Map,
    changes: Changes,
}

impl Edit<'_> {
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        match self.changes.nodes.get(&id) {
            Some(pending) => pending.as_ref(),
            None => self.map.nodes.get(&id),
        }
    }

    fn node_mut(&mut self, id: NodeId) -> Result<&mut Node, EditError> {
        if !self.changes.nodes.contains_key(&id) {
            let n = self.map.nodes.get(&id).ok_or(EditError::Missing(id))?.clone();
            self.changes.nodes.insert(id, Some(n));
        }
        self.changes.nodes.get_mut(&id).and_then(Option::as_mut).ok_or(EditError::Missing(id))
    }

    fn assign_face_ids(&mut self, kind: &mut NodeKind) {
        if let NodeKind::Brush(b) = kind {
            for f in b.faces.iter_mut().filter(|f| f.id.0 == 0) {
                f.id = FaceId(self.changes.next_face);
                self.changes.next_face = self.changes.next_face.saturating_add(1);
            }
        }
    }

    pub fn insert(&mut self, parent: NodeId, kind: NodeKind) -> Result<NodeId, EditError> {
        let len = self.node(parent).ok_or(EditError::Missing(parent))?.children.len();
        self.insert_at(parent, len, kind)
    }

    pub fn insert_at(&mut self, parent: NodeId, index: usize, mut kind: NodeKind) -> Result<NodeId, EditError> {
        let p = self.node(parent).ok_or(EditError::Missing(parent))?;
        if !p.kind.can_contain(&kind) {
            return Err(bad_parent(&p.kind, &kind));
        }
        self.assign_face_ids(&mut kind);
        let id = NodeId(self.changes.next_node);
        self.changes.next_node = self.changes.next_node.saturating_add(1);
        let node = Node { parent: Some(parent), children: Vec::new(), kind, tags: Vec::new(), hidden: false, locked: false, extra: Extra::new() };
        self.changes.nodes.insert(id, Some(node));
        let p = self.node_mut(parent)?;
        let index = index.min(p.children.len());
        p.children.insert(index, id);
        Ok(id)
    }

    pub fn remove(&mut self, id: NodeId) -> Result<(), EditError> {
        if id == ROOT {
            return Err(EditError::Root);
        }
        let parent = self.node(id).ok_or(EditError::Missing(id))?.parent.ok_or(EditError::Root)?;
        for n in subtree(|i| self.node(i), id) {
            self.changes.nodes.insert(n, None);
        }
        self.node_mut(parent)?.children.retain(|c| *c != id);
        Ok(())
    }

    /// Replaces a node's content, keeping its place in the tree.
    pub fn set_kind(&mut self, id: NodeId, mut kind: NodeKind) -> Result<(), EditError> {
        let node = self.node(id).ok_or(EditError::Missing(id))?;
        if let Some(p) = node.parent.and_then(|p| self.node(p))
            && !p.kind.can_contain(&kind)
        {
            return Err(bad_parent(&p.kind, &kind));
        }
        if matches!(node.kind, NodeKind::World(_)) != matches!(kind, NodeKind::World(_)) {
            return Err(EditError::Root);
        }
        for c in &node.children {
            let child = self.node(*c).ok_or(EditError::Missing(*c))?;
            if !kind.can_contain(&child.kind) {
                return Err(bad_parent(&kind, &child.kind));
            }
        }
        self.assign_face_ids(&mut kind);
        self.node_mut(id)?.kind = kind;
        Ok(())
    }

    /// Changes flags, tags or extra data; the tree position and content are left alone.
    pub fn update(&mut self, id: NodeId, f: impl FnOnce(&mut bool, &mut bool, &mut Vec<crate::TagId>, &mut Extra)) -> Result<(), EditError> {
        let n = self.node_mut(id)?;
        f(&mut n.hidden, &mut n.locked, &mut n.tags, &mut n.extra);
        Ok(())
    }

    pub fn reparent(&mut self, id: NodeId, new_parent: NodeId, index: Option<usize>) -> Result<(), EditError> {
        if id == ROOT {
            return Err(EditError::Root);
        }
        if subtree(|i| self.node(i), id).contains(&new_parent) {
            return Err(EditError::Cycle);
        }
        let node = self.node(id).ok_or(EditError::Missing(id))?;
        let old_parent = node.parent.ok_or(EditError::Root)?;
        let target = self.node(new_parent).ok_or(EditError::Missing(new_parent))?;
        if !target.kind.can_contain(&node.kind) {
            return Err(bad_parent(&target.kind, &node.kind));
        }
        self.node_mut(old_parent)?.children.retain(|c| *c != id);
        let p = self.node_mut(new_parent)?;
        let index = index.unwrap_or(p.children.len()).min(p.children.len());
        p.children.insert(index, id);
        self.node_mut(id)?.parent = Some(new_parent);
        Ok(())
    }

    pub fn set_tags(&mut self, tags: Vec<Tag>) {
        self.changes.tags = Some(tags);
    }

    pub fn set_cordons(&mut self, cordons: Vec<Cordon>) {
        self.changes.cordons = Some(cordons);
    }

    pub fn finish(self) -> Changes {
        self.changes
    }
}

#[cfg(test)]
mod tests {
    use mapcraft_geom::{Aabb, DVec3};

    use super::*;
    use crate::{Brush, Group, Layer, TexDefaults};

    fn brush() -> NodeKind {
        NodeKind::Brush(Brush::cuboid(Aabb::new(DVec3::ZERO, DVec3::splat(64.0)), &TexDefaults::default()))
    }

    #[test]
    fn insert_apply_undo_redo() {
        let mut map = Map::new();
        let original = map.clone();
        let mut e = map.edit();
        let layer = e.insert(ROOT, NodeKind::Layer(Layer { name: "Detail".into(), ..Layer::default() })).unwrap();
        let b = e.insert(layer, brush()).unwrap();
        let changes = e.finish();
        let undo = map.apply(changes);
        assert_eq!(map.get(ROOT).unwrap().children, vec![layer]);
        let faces: Vec<u64> = map.get(b).unwrap().kind.brush().unwrap().faces.iter().map(|f| f.id.0).collect();
        assert_eq!(faces, vec![1, 2, 3, 4, 5, 6]);

        let redo = map.apply(undo);
        assert_eq!(map.nodes, original.nodes);
        map.apply(redo);
        assert!(map.get(b).is_some());

        // Fresh ids keep growing after undo.
        let mut e = map.edit();
        let next = e.insert(ROOT, brush()).unwrap();
        assert!(next.0 > b.0);
    }

    #[test]
    fn tree_rules() {
        let mut map = Map::new();
        let mut e = map.edit();
        let ent = e.insert(ROOT, NodeKind::Entity(Entity::new("func_detail"))).unwrap();
        assert!(matches!(e.insert(ent, NodeKind::Group(Group::default())), Err(EditError::BadParent { .. })));
        let g = e.insert(ROOT, NodeKind::Group(Group::default())).unwrap();
        let inner = e.insert(g, NodeKind::Group(Group::default())).unwrap();
        assert_eq!(e.reparent(g, inner, None), Err(EditError::Cycle));
        assert_eq!(e.remove(ROOT), Err(EditError::Root));
        assert!(e.insert(g, NodeKind::Layer(Layer::default())).is_err());
        map.apply(e.finish());

        let mut e = map.edit();
        e.reparent(ent, g, Some(0)).unwrap();
        e.remove(g).unwrap();
        map.apply(e.finish());
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn visibility_tags_and_cordons() {
        let mut map = Map::new();
        let mut e = map.edit();
        let g = e.insert(ROOT, NodeKind::Group(Group::default())).unwrap();
        let a = e.insert(g, brush()).unwrap();
        let far = NodeKind::Brush(Brush::cuboid(Aabb::new(DVec3::splat(1000.0), DVec3::splat(1064.0)), &TexDefaults::default()));
        let b = e.insert(ROOT, far).unwrap();
        e.update(b, |_, _, tags, _| tags.push(TagId(1))).unwrap();
        e.set_tags(vec![Tag { id: TagId(1), name: "Far".into(), color: None, parent: None, hidden: true }]);
        map.apply(e.finish());
        let v = map.visible_nodes();
        assert!(v.contains(&a) && !v.contains(&b));

        let mut e = map.edit();
        e.update(g, |hidden, _, _, _| *hidden = true).unwrap();
        e.set_tags(vec![]);
        e.set_cordons(vec![Cordon { name: "Far".into(), bounds: Aabb::new(DVec3::splat(900.0), DVec3::splat(1100.0)), active: true }]);
        let undo = map.apply(e.finish());
        let v = map.visible_nodes();
        assert!(!v.contains(&a) && v.contains(&b));
        map.apply(undo);
        assert!(map.cordons.is_empty() && map.tags.len() == 1);
    }

    #[test]
    fn from_nodes_validates_and_repairs() {
        let mut map = Map::new();
        let mut e = map.edit();
        let a = e.insert(ROOT, brush()).unwrap();
        let b = e.insert(ROOT, brush()).unwrap();
        map.apply(e.finish());

        let mut nodes = map.nodes.clone();
        // Duplicate face ids get repaired.
        let copy = nodes[&a].kind.clone();
        nodes.get_mut(&b).unwrap().kind = copy;
        let fixed = Map::from_nodes(nodes.clone(), vec![], Extra::new()).unwrap();
        let mut ids: Vec<_> = fixed.nodes().filter_map(|(_, n)| n.kind.brush()).flat_map(|b| b.faces.iter().map(|f| f.id)).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 12);

        let mut broken = nodes.clone();
        broken.get_mut(&b).unwrap().parent = Some(a);
        assert!(Map::from_nodes(broken, vec![], Extra::new()).is_err());

        let mut cyclic = nodes.clone();
        cyclic.get_mut(&ROOT).unwrap().children.push(a);
        assert!(Map::from_nodes(cyclic, vec![], Extra::new()).is_err());

        let mut orphan = nodes;
        orphan.get_mut(&ROOT).unwrap().children.retain(|c| *c != b);
        assert!(Map::from_nodes(orphan, vec![], Extra::new()).is_err());
    }
}
