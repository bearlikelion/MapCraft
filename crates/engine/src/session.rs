use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use mapcraft_geom::{DMat4, DVec3};
use mapcraft_map::{BlendRole, Changes, FaceId, Map, Node, NodeId, NodeKind, ROOT, TexDefaults};
use serde_json::Value;

use crate::EngineError;

/// What a click selects (Hammer's selection modes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelectMode {
    /// The hit itself, a double click its outermost group, else its entity (TrenchBroom); [`Session::click_groups`] swaps the two.
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
        // A generator's brushes are rebuilt on every edit, so a click takes the generator instead.
        SelectMode::Groups => {
            ancestors.iter().rev().copied().find(|a| map.get(*a).is_some_and(|n| matches!(&n.kind, NodeKind::Group(g) if g.generator.is_some()))).unwrap_or(hit)
        }
    }
}

/// What a double click on `id` selects: its outermost group, else its entity.
pub fn whole_object(map: &Map, id: NodeId) -> Option<NodeId> {
    let ancestors = map.ancestors(id);
    let is = |a: &&NodeId, group: bool| {
        map.get(**a).is_some_and(|n| if group { matches!(n.kind, NodeKind::Group(_)) } else { matches!(n.kind, NodeKind::Entity(_)) })
    };
    ancestors.iter().rev().find(|a| is(a, true)).or(ancestors.first().filter(|a| is(a, false))).copied()
}

struct Transaction {
    label: String,
    inverse: Changes,
    serial: u64,
    /// Selection to restore when this entry is applied.
    selection: BTreeSet<NodeId>,
    coalesce: Option<String>,
    /// The serial of the step this one took back on its own.
    reverts: Option<u64>,
    /// It put back a step that was taken back on its own.
    redo: bool,
}

/// One thing a step changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Part {
    Node(NodeId),
    Tags,
    Cordons,
}

impl Transaction {
    fn parts(&self) -> impl Iterator<Item = Part> + '_ {
        let nodes = self.inverse.nodes.keys().map(|id| Part::Node(*id));
        nodes.chain(self.inverse.tags.is_some().then_some(Part::Tags)).chain(self.inverse.cordons.is_some().then_some(Part::Cordons))
    }
}

/// One edit in the history list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryStep {
    pub serial: u64,
    pub label: String,
    /// Some of it is in effect, so it can be undone on its own.
    pub can_undo: bool,
    /// Some of it was undone, so it can come back.
    pub can_redo: bool,
}

/// One mesh of a prop's model, which its `mesh.<name>.` keys place.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshFrame {
    pub prop: NodeId,
    pub name: String,
    /// From the model space the mesh's edit applies in to the world: the prop's placement times its parent meshes' changes.
    pub frame: DMat4,
    /// The mesh's own origin in model space, which it turns and scales about.
    pub pivot: DVec3,
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
    /// In Groups mode a click picks the whole group or entity and a double click the part.
    pub click_groups: bool,
    /// Material and scale given to new faces (Hammer's current texture).
    pub tex: TexDefaults,
    pub texture_lock: bool,
    pub extrude_top_level: bool,
    pub grid: f64,
    /// Where new nodes go: the world, a layer or a group.
    pub active_parent: NodeId,
    pub default_solid_entity: String,
    /// Materials with clear pixels, such as glass and grates, whose faces hide nothing behind them when saved.
    pub see_through: BTreeSet<String>,
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
    /// The mesh of the one selected prop that transforms move instead of the prop, which the editor sets from the prop's model.
    pub mesh: Option<MeshFrame>,
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
            click_groups: false,
            tex: TexDefaults::default(),
            texture_lock: true,
            extrude_top_level: false,
            grid: 16.0,
            active_parent: ROOT,
            default_solid_entity: "func_detail".into(),
            see_through: BTreeSet::new(),
            triggers: vec!["trigger*".into()],
            blend_roles: BTreeMap::new(),
            paint_stroke: None,
            heightmap: None,
            notice: None,
            stretched: BTreeSet::new(),
            mesh: None,
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
        mapcraft_format::save_with(path, &self.map, self.sdk.as_deref(), &self.view, &self.see_through)?;
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
        self.undo.push(Transaction { label: label.into(), inverse, serial, selection: before, coalesce: key.map(String::from), reverts: None, redo: false });
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
        self.redo.push(Transaction { inverse, selection: after, coalesce: None, ..t });
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
        self.undo.push(Transaction { inverse, selection: before, coalesce: None, ..t });
        self.prune_selection();
        true
    }

    /// Per step serial, the parts of it that a later step took back on its own.
    fn taken_back(&self) -> BTreeMap<u64, BTreeSet<Part>> {
        let mut out: BTreeMap<u64, BTreeSet<Part>> = BTreeMap::new();
        for t in self.undo.iter().rev() {
            let Some(target) = t.reverts else { continue };
            let live: Vec<Part> = t.parts().filter(|p| out.get(&t.serial).is_none_or(|gone| !gone.contains(p))).collect();
            out.entry(target).or_default().extend(live);
        }
        out
    }

    fn live_parts(&self, index: usize, gone: &BTreeMap<u64, BTreeSet<Part>>) -> BTreeSet<Part> {
        let Some(t) = self.undo.get(index) else { return BTreeSet::new() };
        t.parts().filter(|p| gone.get(&t.serial).is_none_or(|g| !g.contains(p))).collect()
    }

    /// Node `id` as step `index` left it.
    fn after(&self, index: usize, id: NodeId) -> Option<&Node> {
        let later = self.undo.get(index.saturating_add(1)..).unwrap_or_default();
        later.iter().find_map(|t| t.inverse.nodes.get(&id)).map_or(self.map.get(id), Option::as_ref)
    }

    /// Whether step `index` added, removed or moved any of `parts` in the tree.
    fn restructures(&self, index: usize, parts: &BTreeSet<Part>) -> bool {
        let Some(step) = self.undo.get(index) else { return false };
        parts.iter().any(|p| {
            let Part::Node(id) = *p else { return false };
            match (step.inverse.nodes.get(&id).and_then(Option::as_ref), self.after(index, id)) {
                (Some(b), Some(a)) => b.parent != a.parent || b.children != a.children,
                (None, None) => false,
                _ => true,
            }
        })
    }

    /// Changes that take `parts` of step `index` back and keep later edits to everything else.
    fn take_back(&self, index: usize, parts: &BTreeSet<Part>) -> Result<Changes, EngineError> {
        let Some(step) = self.undo.get(index) else { return Err(EngineError::Refused("that step is no longer in the history")) };
        let later = self.undo.get(index.saturating_add(1)..).unwrap_or_default();
        let clash = |touches: &dyn Fn(&Transaction) -> bool| EngineError::Conflict {
            step: step.label.clone(),
            later: later.iter().rev().find(|t| touches(t)).map_or_else(|| "a later edit".to_string(), |t| t.label.clone()),
        };
        let mut out = self.map.edit().finish();
        for part in parts {
            match *part {
                Part::Tags => {
                    let Some(before) = &step.inverse.tags else { continue };
                    if later.iter().find_map(|t| t.inverse.tags.as_ref()).is_some_and(|after| *after != self.map.tags) {
                        return Err(clash(&|t| t.inverse.tags.is_some()));
                    }
                    out.tags = Some(before.clone());
                }
                Part::Cordons => {
                    let Some(before) = &step.inverse.cordons else { continue };
                    if later.iter().find_map(|t| t.inverse.cordons.as_ref()).is_some_and(|after| *after != self.map.cordons) {
                        return Err(clash(&|t| t.inverse.cordons.is_some()));
                    }
                    out.cordons = Some(before.clone());
                }
                Part::Node(id) => {
                    let Some(before) = step.inverse.nodes.get(&id) else { continue };
                    let current = self.map.get(id);
                    let touched = |t: &Transaction| t.inverse.nodes.contains_key(&id);
                    let node = match (before, self.after(index, id), current) {
                        (None, None, _) => continue,
                        (None, after, _) if after == current => None,
                        (Some(b), None, None) => Some(b.clone()),
                        (Some(b), Some(a), Some(cur)) => Some(restored(b, a, cur).ok_or_else(|| clash(&touched))?),
                        _ => return Err(clash(&touched)),
                    };
                    out.nodes.insert(id, node);
                }
            }
        }
        if !tree_holds(&self.map, &out) {
            return Err(EngineError::Refused("later edits rearranged the same objects; undo those first"));
        }
        Ok(out)
    }

    /// Takes `parts` of step `index` back as a new step.
    fn revert(&mut self, index: usize, parts: &BTreeSet<Part>) -> Result<(), EngineError> {
        let changes = self.take_back(index, parts)?;
        let Some(step) = self.undo.get(index) else { return Ok(()) };
        if changes.is_empty() {
            return Err(EngineError::Refused("nothing of that step is left to undo"));
        }
        let (serial, redo) = (step.serial, step.reverts.is_some());
        let label = match step.label.strip_prefix("Undo ") {
            Some(what) if redo => format!("Redo {what}"),
            _ => format!("Undo {}", step.label),
        };
        self.apply(&label, changes);
        if let Some(top) = self.undo.last_mut() {
            top.reverts = Some(serial);
            top.redo = redo;
        }
        Ok(())
    }

    fn step_index(&self, serial: u64) -> Result<usize, EngineError> {
        self.undo.iter().position(|t| t.serial == serial).ok_or(EngineError::Refused("that step is no longer in the history"))
    }

    /// The newest `limit` edits, newest first: those Undo took back, then the rest with the ones undone on their own left in place.
    pub fn history(&self, limit: usize) -> Vec<HistoryStep> {
        let gone = self.taken_back();
        let undone = self.redo.iter().map(|t| (t, false, true));
        let done = self.undo.iter().rev().map(|t| {
            let back = gone.get(&t.serial);
            (t, t.parts().any(|p| back.is_none_or(|b| !b.contains(&p))), back.is_some_and(|b| !b.is_empty()))
        });
        let steps = undone.chain(done).filter(|(t, ..)| t.reverts.is_none()).take(limit);
        steps.map(|(t, can_undo, can_redo)| HistoryStep { serial: t.serial, label: t.label.clone(), can_undo, can_redo }).collect()
    }

    /// Undoes step `serial` on its own as a new step, keeping later edits to other objects.
    pub fn undo_step(&mut self, serial: u64) -> Result<(), EngineError> {
        let index = self.step_index(serial)?;
        let parts = self.live_parts(index, &self.taken_back());
        self.revert(index, &parts)
    }

    /// Puts step `serial` back: one Undo took back, with those after it, or one undone on its own.
    pub fn redo_step(&mut self, serial: u64) -> Result<(), EngineError> {
        if self.redo.iter().any(|t| t.serial == serial) {
            while self.undo.last().is_none_or(|t| t.serial != serial) && self.redo() {}
            return Ok(());
        }
        let gone = self.taken_back();
        let live = |t: &&Transaction| t.parts().any(|p| gone.get(&t.serial).is_none_or(|g| !g.contains(&p)));
        let undos: Vec<u64> = self.undo.iter().rev().filter(|t| t.reverts == Some(serial)).filter(live).map(|t| t.serial).collect();
        if undos.is_empty() {
            return Err(EngineError::Refused("that step is not undone"));
        }
        for undo in undos {
            let index = self.step_index(undo)?;
            let parts = self.live_parts(index, &self.taken_back());
            self.revert(index, &parts)?;
        }
        Ok(())
    }

    /// The selected nodes and everything inside them.
    fn selection_scope(&self) -> BTreeSet<NodeId> {
        self.selection.iter().flat_map(|id| self.map.subtree(*id)).collect()
    }

    /// Undoes the last edit to the selection and what is inside it, leaving the rest of that edit; an edit that added, removed or moved them in the tree is undone whole.
    pub fn undo_selection(&mut self) -> Result<(), EngineError> {
        if self.selection.is_empty() {
            return Err(EngineError::NothingSelected);
        }
        let scope = self.selection_scope();
        let gone = self.taken_back();
        // ponytail: walks every step's nodes; index steps by node if deep histories make this slow
        for (index, t) in self.undo.iter().enumerate().rev().filter(|(_, t)| t.reverts.is_none()) {
            let back = gone.get(&t.serial);
            let mine: BTreeSet<Part> =
                t.inverse.nodes.keys().filter(|id| scope.contains(id)).map(|id| Part::Node(*id)).filter(|p| back.is_none_or(|b| !b.contains(p))).collect();
            if mine.is_empty() {
                continue;
            }
            let parts = if self.restructures(index, &mine) { self.live_parts(index, &gone) } else { mine };
            return self.revert(index, &parts);
        }
        Err(EngineError::Refused("no earlier edit to the selection"))
    }

    /// Puts back the last edit [`Session::undo_selection`] took from the selection, until another edit changes it; with nothing selected, the one just taken, with its selection.
    pub fn redo_selection(&mut self) -> Result<(), EngineError> {
        let scope = self.selection_scope();
        let gone = self.taken_back();
        let found = if scope.is_empty() {
            self.undo.iter().enumerate().next_back()
        } else {
            let live = |t: &Transaction, id: &NodeId| gone.get(&t.serial).is_none_or(|b| !b.contains(&Part::Node(*id)));
            self.undo.iter().enumerate().rev().find(|(_, t)| !t.redo && t.inverse.nodes.keys().any(|id| scope.contains(id) && live(t, id)))
        };
        let Some((index, undo)) = found.filter(|(_, t)| t.reverts.is_some() && !t.redo) else {
            return Err(EngineError::Refused("nothing to redo in the selection"));
        };
        let selected = undo.selection.clone();
        let parts = self.live_parts(index, &gone);
        self.revert(index, &parts)?;
        if self.selection.is_empty() {
            self.selection = selected;
            self.prune_selection();
        }
        Ok(())
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
    /// What a click on `hit` selects.
    pub fn pick(&self, hit: NodeId) -> NodeId {
        match self.select_mode {
            SelectMode::Groups if self.click_groups => whole_object(&self.map, hit).unwrap_or(hit),
            mode => pick_target(&self.map, hit, mode),
        }
    }

    pub fn click(&mut self, hit: Option<NodeId>, additive: bool) {
        let target = hit.filter(|h| self.map.editable_nodes().contains(h)).map(|h| self.pick(h));
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

/// `cur` with what changed from `before` to `after` put back; `None` when a later edit changed the same field.
fn restored(before: &Node, after: &Node, cur: &Node) -> Option<Node> {
    let mut n = cur.clone();
    macro_rules! put_back {
        ($($field:ident),*) => {$(
            if before.$field != after.$field {
                if cur.$field != after.$field {
                    return None;
                }
                n.$field.clone_from(&before.$field);
            }
        )*};
    }
    put_back!(parent, kind, tags, hidden, locked, extra, shader, shader_params);
    if cur.children == after.children {
        n.children = before.children.clone();
    } else if before.children != after.children {
        let (was, now): (BTreeSet<NodeId>, BTreeSet<NodeId>) = (before.children.iter().copied().collect(), after.children.iter().copied().collect());
        n.children.retain(|c| was.contains(c) || !now.contains(c));
        let mut have: BTreeSet<NodeId> = n.children.iter().copied().collect();
        for (i, c) in before.children.iter().enumerate() {
            if !now.contains(c) && have.insert(*c) {
                n.children.insert(i.min(n.children.len()), *c);
            }
        }
    }
    Some(n)
}

/// Whether every node `changes` writes still agrees with its parent and children and hangs from the root.
fn tree_holds(map: &Map, changes: &Changes) -> bool {
    let get = |id: NodeId| match changes.nodes.get(&id) {
        Some(n) => n.as_ref(),
        None => map.get(id),
    };
    let rooted = |id: NodeId| {
        let mut at = id;
        for _ in 0..map.len().saturating_add(changes.nodes.len()) {
            if at == ROOT {
                return true;
            }
            let Some(parent) = get(at).and_then(|n| n.parent) else { return false };
            at = parent;
        }
        false
    };
    changes.nodes.iter().all(|(id, n)| match n {
        Some(n) => {
            let listed = match n.parent {
                None => *id == ROOT,
                Some(p) => *id != ROOT && get(p).is_some_and(|p| p.kind.can_contain(&n.kind) && p.children.iter().filter(|c| *c == id).count() == 1),
            };
            listed && rooted(*id) && n.children.iter().all(|c| get(*c).is_some_and(|x| x.parent == Some(*id)))
        }
        None => map.get(*id).is_none_or(|old| {
            let unlisted = old.parent.and_then(get).is_none_or(|p| !p.children.contains(id));
            unlisted && old.children.iter().all(|c| get(*c).is_none_or(|x| x.parent != Some(*id)))
        }),
    })
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
        assert_eq!(pick_target(&s.map, b, SelectMode::Groups), b);
        assert_eq!(pick_target(&s.map, b, SelectMode::Objects), ent);
        assert_eq!(pick_target(&s.map, b, SelectMode::Solids), b);
        assert_eq!(whole_object(&s.map, b), Some(g));
        assert_eq!(whole_object(&s.map, ent), Some(g));
        assert_eq!(whole_object(&s.map, g), None);
        s.click(Some(b), false);
        assert_eq!(s.selection, BTreeSet::from([b]));
        s.click(Some(b), true);
        assert!(s.selection.is_empty());
        s.click_groups = true;
        s.click(Some(b), false);
        assert_eq!(s.selection, BTreeSet::from([g]));
        s.click_groups = false;
        s.select_all();
        assert_eq!(s.selection, BTreeSet::from([g]));
        let mut e = s.map.edit();
        let generator = mapcraft_map::Generator {
            plugin: "surfsup".into(),
            command: "ramp".into(),
            params: Value::Null,
            origin: DVec3::ZERO,
            angles: DVec3::ZERO,
            stretch: None,
        };
        let made = e.insert(ROOT, NodeKind::Group(Group { generator: Some(generator), ..Group::default() })).unwrap();
        let part = e.insert(made, cube()).unwrap();
        let c = e.finish();
        s.apply("Generate", c);
        assert_eq!(pick_target(&s.map, part, SelectMode::Groups), made, "a generator's parts are picked as the generator");
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

    fn brush_at(s: &Session, id: NodeId) -> f64 {
        s.map.get(id).unwrap().kind.brush().unwrap().bounds().unwrap().min.x
    }

    fn slide(s: &mut Session, id: NodeId, by: f64) {
        s.selection = BTreeSet::from([id]);
        s.translate_selection(DVec3::X * by, None).unwrap();
    }

    fn serial_of(s: &Session, nth_newest: usize) -> u64 {
        s.history(usize::MAX)[nth_newest].serial
    }

    #[test]
    fn one_step_undoes_on_its_own_and_comes_back() {
        let mut s = Session::default();
        let a = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
        let b = s.create_brush(Aabb::new(DVec3::splat(128.0), DVec3::splat(192.0))).unwrap();
        slide(&mut s, a, 8.0);
        slide(&mut s, b, 8.0);
        let moved_a = serial_of(&s, 1);
        s.undo_step(moved_a).unwrap();
        assert_eq!((brush_at(&s, a), brush_at(&s, b)), (0.0, 136.0), "the later move of the other brush stays");
        let rows = s.history(usize::MAX);
        assert_eq!(rows.len(), 4, "the step that took it back is not a row");
        assert_eq!((rows[1].serial, rows[1].can_undo, rows[1].can_redo), (moved_a, false, true));
        assert!(matches!(s.undo_step(moved_a), Err(EngineError::Refused(_))), "nothing of it is left");
        assert!(s.undo(), "taking it back is itself a step");
        assert_eq!(brush_at(&s, a), 8.0);
        assert!(s.redo());
        s.redo_step(moved_a).unwrap();
        assert_eq!(brush_at(&s, a), 8.0);
        assert!(s.history(usize::MAX)[1].can_undo && !s.history(usize::MAX)[1].can_redo);
        s.undo_step(moved_a).unwrap();
        assert_eq!(brush_at(&s, a), 0.0, "and it can go again");
    }

    #[test]
    fn a_step_later_edits_built_on_is_refused() {
        let mut s = Session::default();
        let a = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
        slide(&mut s, a, 8.0);
        let before = s.map.clone();
        let created = serial_of(&s, 1);
        let Err(EngineError::Conflict { later, .. }) = s.undo_step(created) else { panic!("moving it came after") };
        assert_eq!(Some(later.as_str()), s.undo_label());
        assert_eq!(s.map, before);
        s.undo_step(serial_of(&s, 0)).unwrap();
        s.undo_step(created).unwrap();
        assert!(s.map.get(a).is_none(), "newest first works");
    }

    #[test]
    fn selection_undo_walks_one_object_back() {
        let mut s = Session::default();
        let a = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
        let b = s.create_brush(Aabb::new(DVec3::splat(128.0), DVec3::splat(192.0))).unwrap();
        slide(&mut s, a, 8.0);
        slide(&mut s, a, 8.0);
        s.selection = BTreeSet::from([a, b]);
        s.translate_selection(DVec3::X * 32.0, None).unwrap();
        s.selection = BTreeSet::from([a]);
        s.undo_selection().unwrap();
        assert_eq!((brush_at(&s, a), brush_at(&s, b)), (16.0, 160.0), "only the selected brush leaves the shared move");
        s.undo_selection().unwrap();
        s.undo_selection().unwrap();
        assert_eq!(brush_at(&s, a), 0.0);
        s.redo_selection().unwrap();
        s.redo_selection().unwrap();
        assert_eq!(brush_at(&s, a), 16.0);
        s.undo_selection().unwrap();
        assert_eq!(brush_at(&s, a), 8.0);
        slide(&mut s, a, 100.0);
        assert!(s.redo_selection().is_err(), "a new edit ends the redo run");
        for _ in 0..3 {
            s.selection = BTreeSet::from([a]);
            s.undo_selection().unwrap();
        }
        assert!(s.map.get(a).is_none(), "the last step back is its creation");
        s.redo_selection().unwrap();
        assert_eq!(s.selection, BTreeSet::from([a]), "with nothing left selected, redo brings back what was just taken");
        s.undo_selection().unwrap();
        assert_eq!(s.map.get(ROOT).unwrap().children, vec![b]);
        assert_eq!(brush_at(&s, b), 160.0);
        s.selection = BTreeSet::from([b]);
        s.undo_selection().unwrap();
        assert_eq!(brush_at(&s, b), 128.0, "the other half of the shared move is still there to undo");
    }

    #[test]
    fn tree_changes_undo_whole_around_later_edits() {
        let mut s = Session::default();
        let a = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
        let b = s.create_brush(Aabb::new(DVec3::splat(128.0), DVec3::splat(192.0))).unwrap();
        s.selection = BTreeSet::from([a]);
        s.delete_selection().unwrap();
        let c = s.create_brush(Aabb::new(DVec3::splat(256.0), DVec3::splat(320.0))).unwrap();
        s.undo_step(serial_of(&s, 1)).unwrap();
        assert_eq!(s.map.get(ROOT).unwrap().children, vec![a, b, c], "the deleted brush is back in its place");
        s.selection = BTreeSet::from([a, b]);
        let g = s.group_selection().unwrap();
        let d = s.create_brush(Aabb::new(DVec3::splat(512.0), DVec3::splat(576.0))).unwrap();
        slide(&mut s, d, 8.0);
        s.selection = BTreeSet::from([a]);
        s.undo_selection().unwrap();
        assert!(s.map.get(g).is_none(), "grouping moved the brush in the tree, so all of it goes");
        assert_eq!(s.map.get(b).unwrap().parent, Some(ROOT));
        assert!(s.map.get(ROOT).unwrap().children.contains(&d));
        let saved = mapcraft_format::to_string(&s.map, None, &Value::Null).unwrap();
        assert!(mapcraft_format::from_str(&saved).is_ok(), "the tree still loads");
    }

    #[test]
    fn history_lists_newest_first_and_redoes_in_order() {
        let mut s = Session::default();
        let a = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
        slide(&mut s, a, 8.0);
        s.edit_cordons(vec![mapcraft_map::Cordon { name: "c".into(), bounds: Aabb::new(DVec3::ZERO, DVec3::splat(64.0)), active: false }], "Add cordon");
        slide(&mut s, a, 8.0);
        let labels = |s: &Session| s.history(3).into_iter().map(|h| (h.label, h.can_undo, h.can_redo)).collect::<Vec<_>>();
        assert_eq!(labels(&s).len(), 3, "the limit holds");
        let cordon = serial_of(&s, 1);
        s.undo_step(cordon).unwrap();
        assert!(s.map.cordons.is_empty() && brush_at(&s, a) == 16.0);
        assert_eq!(labels(&s)[1], ("Add cordon".to_string(), false, true));
        s.redo_step(cordon).unwrap();
        assert_eq!(s.map.cordons.len(), 1);
        while s.undo() {}
        let rows = s.history(usize::MAX);
        assert!(rows.iter().all(|r| r.can_redo && !r.can_undo) && rows.len() == 4);
        s.redo_step(rows[2].serial).unwrap();
        assert_eq!(brush_at(&s, a), 8.0, "redoing a row redoes the ones before it");
        assert_eq!(s.history(usize::MAX).iter().filter(|r| r.can_undo).count(), 2);
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
