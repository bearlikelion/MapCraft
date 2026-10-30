use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use mapcraft_geom::DVec3;
use mapcraft_map::{BlendRole, Changes, FaceId, Map, NodeId, NodeKind, ROOT, TexDefaults};
use serde_json::Value;

use crate::EngineError;

/// What a click selects (Hammer's selection modes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelectMode {
    /// The outermost group around the hit, else its entity, else the brush.
    #[default]
    Groups,
    /// The brush's entity, else the brush; groups are ignored.
    Objects,
    /// The brush itself.
    Solids,
}

/// The node a click on `hit` selects in `mode`.
pub fn pick_target(map: &Map, hit: NodeId, mode: SelectMode) -> NodeId {
    let ancestors = map.ancestors(hit);
    let entity = ancestors.first().copied().filter(|p| map.get(*p).is_some_and(|n| matches!(n.kind, NodeKind::Entity(_))));
    match mode {
        SelectMode::Solids => hit,
        SelectMode::Objects => entity.unwrap_or(hit),
        SelectMode::Groups => {
            let group = ancestors.iter().rev().find(|a| map.get(**a).is_some_and(|n| matches!(n.kind, NodeKind::Group(_))));
            group.copied().or(entity).unwrap_or(hit)
        }
    }
}

struct Transaction {
    label: String,
    inverse: Changes,
    serial: u64,
    /// Selection to restore when this entry is applied.
    selection: BTreeSet<NodeId>,
    coalesce: Option<String>,
}

pub struct Session {
    pub map: Map,
    pub path: Option<PathBuf>,
    pub sdk: Option<String>,
    pub view: Value,
    pub selection: BTreeSet<NodeId>,
    /// Faces picked for texturing (Hammer's face edit mode).
    pub face_selection: BTreeSet<(NodeId, FaceId)>,
    /// The face picked last, which Unwrap lays the others out from.
    pub active_face: Option<(NodeId, FaceId)>,
    /// Where the 3D view looks, for Project from View.
    pub view_dir: DVec3,
    pub select_mode: SelectMode,
    /// Material and scale given to new faces (Hammer's current texture).
    pub tex: TexDefaults,
    pub texture_lock: bool,
    pub grid: f64,
    /// Where new nodes go: the world, a layer or a group.
    pub active_parent: NodeId,
    pub default_solid_entity: String,
    /// Trigger classes, as the SDK's `triggers` lists them.
    pub triggers: Vec<String>,
    /// How the project's materials blend; a key ending in `*` covers every material starting with the rest.
    pub blend_roles: BTreeMap<String, BlendRole>,
    /// A paint stroke's coalesce key and, per face, the weights it started from and its coverage per vertex.
    pub(crate) paint_stroke: Option<(String, BTreeMap<(NodeId, FaceId), (Vec<f64>, Vec<f64>)>)>,
    /// The faces the last heightmap covered, its 0..1 samples on each and whether noise made them.
    pub(crate) heightmap: Option<(Vec<(NodeId, FaceId)>, Vec<Vec<f64>>, bool)>,
    /// Something the last edit wants the user to know, such as displacements it flattened.
    pub notice: Option<String>,
    /// Stretched generator groups the last transform touched, for the editor to fold into their parameters with [`Session::refit`].
    pub stretched: BTreeSet<NodeId>,
    undo: Vec<Transaction>,
    redo: Vec<Transaction>,
    next_serial: u64,
    saved_serial: u64,
}

impl Default for Session {
    fn default() -> Self {
        Self::new(Map::new())
    }
}

impl Session {
    pub fn new(map: Map) -> Self {
        Self {
            map,
            path: None,
            sdk: None,
            view: Value::Null,
            selection: BTreeSet::new(),
            face_selection: BTreeSet::new(),
            active_face: None,
            view_dir: DVec3::new(1.0, 1.0, -0.5),
            select_mode: SelectMode::default(),
            tex: TexDefaults::default(),
            texture_lock: true,
            grid: 16.0,
            active_parent: ROOT,
            default_solid_entity: "func_detail".into(),
            triggers: vec!["trigger*".into()],
            blend_roles: BTreeMap::new(),
            paint_stroke: None,
            heightmap: None,
            notice: None,
            stretched: BTreeSet::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            next_serial: 1,
            saved_serial: 0,
        }
    }

    pub fn open(path: &Path) -> Result<Self, EngineError> {
        let file = mapcraft_format::load(path)?;
        let mut s = Self::new(file.map);
        s.path = Some(path.to_path_buf());
        s.sdk = file.sdk;
        s.view = file.view;
        Ok(s)
    }

    pub fn save(&mut self) -> Result<(), EngineError> {
        let path = self.path.clone().ok_or(EngineError::NoPath)?;
        self.save_as(&path)
    }

    pub fn save_as(&mut self, path: &Path) -> Result<(), EngineError> {
        mapcraft_format::save(path, &self.map, self.sdk.as_deref(), &self.view)?;
        self.path = Some(path.to_path_buf());
        self.saved_serial = self.current_serial();
        Ok(())
    }

    /// The last undo step's serial, which no other step shares; 0 with none.
    pub fn current_serial(&self) -> u64 {
        self.undo.last().map_or(0, |t| t.serial)
    }

    pub fn is_dirty(&self) -> bool {
        self.current_serial() != self.saved_serial
    }

    /// Treats the map as changed since it was saved, e.g. one recovered from an autosave.
    pub fn mark_unsaved(&mut self) {
        self.saved_serial = u64::MAX;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|t| t.label.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|t| t.label.as_str())
    }

    /// Applies an edit as one undo step.
    pub fn apply(&mut self, label: &str, changes: Changes) {
        self.apply_coalesced(label, changes, None);
    }

    /// Like [`Session::apply`], but merges into the previous step when it used the same key (a drag or a run of nudges).
    pub fn apply_coalesced(&mut self, label: &str, changes: Changes, key: Option<&str>) {
        if changes.is_empty() {
            return;
        }
        // Any edit makes a paint stroke's starting weights and the last heightmap stale; their own edits restore them after storing.
        self.paint_stroke = None;
        self.heightmap = None;
        self.stretched.clear();
        let before = self.selection.clone();
        let mut inverse = self.apply_following(changes);
        self.redo.clear();
        if let (Some(key), Some(top)) = (key, self.undo.last_mut())
            && top.coalesce.as_deref() == Some(key)
            && top.serial != self.saved_serial
        {
            // The older inverse wins so undo returns to the state before the whole run.
            inverse.merge(std::mem::take(&mut top.inverse));
            top.inverse = inverse;
            self.prune_selection();
            return;
        }
        let serial = self.next_serial;
        self.next_serial += 1;
        self.undo.push(Transaction { label: label.into(), inverse, serial, selection: before, coalesce: key.map(String::from) });
        self.prune_selection();
    }

    /// Applies `changes` and lets curve groups follow their sources; returns the inverse of both.
    fn apply_following(&mut self, changes: Changes) -> Changes {
        let mut touched: BTreeSet<NodeId> = changes.nodes.keys().copied().collect();
        let mut inverse = self.map.apply(changes);
        // A few rounds let one curve feed another.
        for _ in 0..4 {
            let Some(rebuilt) = crate::curves::follow(&self.map, &touched) else { break };
            touched = rebuilt.nodes.keys().copied().collect();
            let mut both = self.map.apply(rebuilt);
            both.merge(inverse);
            inverse = both;
        }
        inverse
    }

    /// Folds `changes` into the last undo step, the edit that made them necessary; a step of its own once that one is saved.
    pub(crate) fn amend(&mut self, label: &str, changes: Changes) {
        if self.undo.last().is_none_or(|t| t.serial == self.saved_serial) {
            return self.apply(label, changes);
        }
        let mut inverse = self.apply_following(changes);
        if let Some(top) = self.undo.last_mut() {
            inverse.merge(std::mem::take(&mut top.inverse));
            top.inverse = inverse;
        }
        self.prune_selection();
    }

    /// Ends a coalescing run so the next edit starts a new undo step.
    pub fn end_coalesce(&mut self) {
        self.paint_stroke = None;
        if let Some(top) = self.undo.last_mut() {
            top.coalesce = None;
        }
    }

    /// Reverts the run of edits under `key` without a redo step, as when a drag is cancelled; false when the last edit is not under it.
    pub fn cancel_coalesce(&mut self, key: &str) -> bool {
        self.paint_stroke = None;
        self.stretched.clear();
        let mut cancelled = false;
        while self.undo.last().is_some_and(|t| t.coalesce.as_deref() == Some(key)) {
            let Some(t) = self.undo.pop() else { break };
            self.map.apply(t.inverse);
            self.selection = t.selection;
            self.heightmap = None;
            cancelled = true;
        }
        self.prune_selection();
        cancelled
    }

    pub fn undo(&mut self) -> bool {
        self.paint_stroke = None;
        self.stretched.clear();
        self.heightmap = None;
        let Some(t) = self.undo.pop() else { return false };
        let inverse = self.map.apply(t.inverse);
        let after = std::mem::replace(&mut self.selection, t.selection);
        self.redo.push(Transaction { label: t.label, inverse, serial: t.serial, selection: after, coalesce: None });
        self.prune_selection();
        true
    }

    pub fn redo(&mut self) -> bool {
        self.paint_stroke = None;
        self.stretched.clear();
        self.heightmap = None;
        let Some(t) = self.redo.pop() else { return false };
        let inverse = self.map.apply(t.inverse);
        let before = std::mem::replace(&mut self.selection, t.selection);
        self.undo.push(Transaction { label: t.label, inverse, serial: t.serial, selection: before, coalesce: None });
        self.prune_selection();
        true
    }

    fn prune_selection(&mut self) {
        let map = &self.map;
        self.selection.retain(|id| map.get(*id).is_some());
        self.face_selection.retain(|(id, f)| map.get(*id).and_then(|n| n.kind.brush()).is_some_and(|b| b.faces.iter().any(|x| x.id == *f)));
        if map.get(self.active_parent).is_none_or(|n| !matches!(n.kind, NodeKind::World(_) | NodeKind::Layer(_) | NodeKind::Group(_))) {
            self.active_parent = ROOT;
        }
    }

    /// Selects the nodes a click on `hit` reaches in the current mode; `additive` toggles instead of replacing.
    pub fn click(&mut self, hit: Option<NodeId>, additive: bool) {
        let target = hit.filter(|h| self.map.editable_nodes().contains(h)).map(|h| pick_target(&self.map, h, self.select_mode));
        if !additive {
            self.selection.clear();
        }
        if let Some(t) = target
            && !self.selection.remove(&t)
        {
            self.selection.insert(t);
        }
    }

    pub fn select_all(&mut self) {
        let map = &self.map;
        let roots = map.get(ROOT).map(|n| n.children.clone()).unwrap_or_default();
        let mut out = BTreeSet::new();
        for id in roots {
            match map.get(id).map(|n| &n.kind) {
                Some(NodeKind::Layer(_)) => out.extend(map.get(id).map(|n| n.children.clone()).unwrap_or_default()),
                Some(_) => {
                    out.insert(id);
                }
                None => {}
            }
        }
        let editable = map.editable_nodes();
        out.retain(|id| editable.contains(id));
        self.selection = out;
    }

    pub fn select_none(&mut self) {
        self.selection.clear();
    }
}

#[cfg(test)]
mod tests {
    use mapcraft_geom::{Aabb, DVec3};
    use mapcraft_map::{Brush, Entity, Group};

    use super::*;

    fn cube() -> NodeKind {
        NodeKind::Brush(Brush::cuboid(Aabb::new(DVec3::ZERO, DVec3::splat(64.0)), &TexDefaults::default()))
    }

    #[test]
    fn undo_redo_and_dirty() {
        let mut s = Session::default();
        assert!(!s.is_dirty());
        let mut e = s.map.edit();
        let b = e.insert(ROOT, cube()).unwrap();
        let c = e.finish();
        s.apply("Create brush", c);
        s.selection.insert(b);
        assert!(s.is_dirty());
        assert_eq!(s.undo_label(), Some("Create brush"));
        assert!(s.undo());
        assert!(s.map.get(b).is_none());
        assert!(s.selection.is_empty());
        assert!(!s.is_dirty());
        assert!(s.redo());
        assert!(s.map.get(b).is_some());
        assert!(!s.redo());
    }

    #[test]
    fn coalescing() {
        let mut s = Session::default();
        let mut e = s.map.edit();
        let b = e.insert(ROOT, cube()).unwrap();
        let c = e.finish();
        s.apply("Create", c);
        let original = s.map.clone();
        for _ in 0..3 {
            let NodeKind::Brush(br) = &s.map.get(b).unwrap().kind else { panic!() };
            let moved = br.translated(DVec3::X * 8.0, true);
            let mut e = s.map.edit();
            e.set_kind(b, NodeKind::Brush(moved)).unwrap();
            let c = e.finish();
            s.apply_coalesced("Move", c, Some("nudge"));
        }
        assert_eq!(s.undo.len(), 2);
        s.undo();
        assert_eq!(s.map, original);
    }

    #[test]
    fn cancelling_a_run_leaves_no_step() {
        let mut s = Session::default();
        let b = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
        let original = s.map.clone();
        for _ in 0..2 {
            let moved = s.map.get(b).unwrap().kind.brush().unwrap().translated(DVec3::X * 8.0, true);
            let mut e = s.map.edit();
            e.set_kind(b, NodeKind::Brush(moved)).unwrap();
            let c = e.finish();
            s.apply_coalesced("Move", c, Some("drag"));
        }
        assert!(!s.cancel_coalesce("other"));
        assert!(s.cancel_coalesce("drag"));
        assert_eq!(s.map, original);
        assert_eq!(s.undo_label(), Some("Create brush"));
        assert!(!s.can_redo());
        assert!(!s.cancel_coalesce("drag"), "the run is gone");
    }

    #[test]
    fn selection_modes() {
        let mut s = Session::default();
        let mut e = s.map.edit();
        let g = e.insert(ROOT, NodeKind::Group(Group::default())).unwrap();
        let ent = e.insert(g, NodeKind::Entity(Entity::new("func_detail"))).unwrap();
        let b = e.insert(ent, cube()).unwrap();
        let c = e.finish();
        s.apply("Build", c);
        assert_eq!(pick_target(&s.map, b, SelectMode::Groups), g);
        assert_eq!(pick_target(&s.map, b, SelectMode::Objects), ent);
        assert_eq!(pick_target(&s.map, b, SelectMode::Solids), b);
        s.click(Some(b), false);
        assert_eq!(s.selection, BTreeSet::from([g]));
        s.click(Some(b), true);
        assert!(s.selection.is_empty());
        s.select_all();
        assert_eq!(s.selection, BTreeSet::from([g]));
    }

    #[test]
    fn locked_and_hidden_layers_keep_their_contents_out_of_reach() {
        let mut s = Session::default();
        let layer = s.create_layer("Reference").unwrap();
        s.active_parent = layer;
        let b = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
        s.set_locked(&[layer], true).unwrap();
        s.click(Some(b), false);
        assert!(s.selection.is_empty(), "a brush in a locked layer cannot be clicked");
        s.select_all();
        assert!(s.selection.is_empty(), "nor selected with Select All");
        s.set_locked(&[layer], false).unwrap();
        s.set_hidden(&[layer], true).unwrap();
        s.select_all();
        assert!(s.selection.is_empty(), "Select All skips a hidden layer's children");
        s.set_hidden(&[layer], false).unwrap();
        s.select_all();
        assert_eq!(s.selection, BTreeSet::from([b]));
        s.edit_cordons(vec![mapcraft_map::Cordon { name: "c".into(), bounds: Aabb::new(DVec3::splat(500.0), DVec3::splat(600.0)), active: true }], "Cordon");
        s.select_all();
        assert!(s.selection.is_empty(), "Select All skips what a cordon leaves out");
    }

    #[test]
    fn save_marks_clean() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Session::default();
        let mut e = s.map.edit();
        e.insert(ROOT, cube()).unwrap();
        let c = e.finish();
        s.apply("Create", c);
        assert!(matches!(s.save(), Err(EngineError::NoPath)));
        let path = dir.path().join("a.mcmap");
        s.save_as(&path).unwrap();
        assert!(!s.is_dirty());
        let reopened = Session::open(&path).unwrap();
        assert_eq!(reopened.map, s.map);
    }
}
