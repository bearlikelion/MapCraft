use std::collections::BTreeSet;

use mapcraft_geom::{Aabb, DMat4, DQuat, DVec3};
use mapcraft_map::{Brush, Edit, Entity, Face, FaceId, Group, NodeId, NodeKind, ROOT, TexDefaults};

use crate::{EngineError, Session};

/// Brush shapes the block tool can draw.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Shape {
    #[default]
    Block,
    Wedge,
    Cylinder(usize),
    Spike(usize),
    Sphere(usize),
    /// Degrees of `arc` starting `start` degrees off +X, with walls `thickness` deep.
    Arch {
        sides: usize,
        arc: f64,
        start: f64,
        thickness: f64,
    },
}

/// What dragging a face does (TrenchBroom's extrude tool).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtrudeMode {
    /// Move the face, resizing its brush.
    Resize,
    /// Leave the brush and add a new one in the dragged volume, or split it when dragged inward.
    Split,
    /// Add a new brush between the face and a moved copy of it.
    Stamp,
}

/// Which side of a clip plane to keep.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipKeep {
    Front,
    Back,
    Both,
}

fn fmt_num(v: f64) -> String {
    let r = (v * 1e4).round() / 1e4;
    if r == r.trunc() { format!("{r:.0}") } else { format!("{r}") }
}

fn fmt_vec(v: DVec3) -> String {
    format!("{} {} {}", fmt_num(v.x), fmt_num(v.y), fmt_num(v.z))
}

fn is_point_entity(edit: &Edit, id: NodeId) -> bool {
    edit.node(id).is_some_and(|n| matches!(n.kind, NodeKind::Entity(_)) && n.children.is_empty())
}

/// An origin and angles after `m`; turns about Z (the usual case) carry over to the yaw.
pub(crate) fn moved_placement(m: &DMat4, origin: DVec3, mut angles: DVec3) -> (DVec3, DVec3) {
    let x = m.x_axis.truncate();
    if m.z_axis.truncate().normalize_or_zero().abs_diff_eq(DVec3::Z, 1e-9) {
        angles.y = (angles.y + x.y.atan2(x.x).to_degrees()).rem_euclid(360.0);
    }
    (m.transform_point3(origin), angles)
}

/// A group's generator placement and mirror plane moved by `m`, so they stay with its children.
fn moved_group(mut g: Group, m: &DMat4) -> Group {
    if let Some(generator) = &mut g.generator {
        (generator.origin, generator.angles) = moved_placement(m, generator.origin, generator.angles);
    }
    g.mirror = g.mirror.map(|mirror| mirror.transformed(m));
    g
}

/// A brush, point entity, prefab or group placement moved by `m`; `None` for kinds placed by their children.
pub(crate) fn transform_kind(kind: NodeKind, m: &DMat4, lock: bool, point: bool) -> Result<Option<NodeKind>, EngineError> {
    Ok(Some(match kind {
        NodeKind::Brush(b) => NodeKind::Brush(b.transformed(m, lock)?),
        NodeKind::Group(g) => NodeKind::Group(moved_group(g, m)),
        NodeKind::Entity(mut ent) if point => {
            let origin = ent.origin().unwrap_or_default();
            ent.set("origin", fmt_vec(m.transform_point3(origin)));
            NodeKind::Entity(ent)
        }
        NodeKind::Prefab(mut p) => {
            (p.origin, p.angles) = moved_placement(m, p.origin, p.angles);
            NodeKind::Prefab(p)
        }
        _ => return Ok(None),
    }))
}

/// Inserts copied or generated nodes under `parent`, moved by `m`, with fresh face ids; returns
/// the new top-level nodes.
pub(crate) fn insert_fragments(e: &mut Edit, parent: NodeId, frags: Vec<mapcraft_format::Fragment>, m: &DMat4, lock: bool) -> Result<Vec<NodeId>, EngineError> {
    let mut roots = Vec::new();
    let mut stack: Vec<(mapcraft_format::Fragment, NodeId, bool)> = frags.into_iter().rev().map(|f| (f, parent, true)).collect();
    while let Some((frag, parent, root)) = stack.pop() {
        if matches!(frag.kind, NodeKind::World(_)) {
            continue;
        }
        let point = matches!(frag.kind, NodeKind::Entity(_)) && frag.children.is_empty();
        let kind = match transform_kind(frag.kind.clone(), m, lock, point)? {
            Some(NodeKind::Brush(mut b)) => {
                b.faces.iter_mut().for_each(|f| f.id = FaceId(0));
                NodeKind::Brush(b)
            }
            Some(k) => k,
            None => frag.kind,
        };
        let id = e.insert(parent, kind)?;
        if root {
            roots.push(id);
        }
        stack.extend(frag.children.into_iter().rev().map(|c| (c, id, false)));
    }
    Ok(roots)
}

impl Session {
    pub(crate) fn require_selection(&self) -> Result<Vec<NodeId>, EngineError> {
        if self.selection.is_empty() { Err(EngineError::NothingSelected) } else { Ok(self.selection.iter().copied().collect()) }
    }

    /// Brushes, point entities and prefabs inside the selection.
    fn selected_leaves(&self) -> Vec<NodeId> {
        self.leaves_of(self.selection.iter().copied())
    }

    /// Brushes, point entities and prefabs inside `roots`.
    fn leaves_of(&self, roots: impl IntoIterator<Item = NodeId>) -> Vec<NodeId> {
        let mut out = BTreeSet::new();
        for id in roots {
            for n in self.map.subtree(id) {
                if let Some(node) = self.map.get(n) {
                    match &node.kind {
                        NodeKind::Brush(_) | NodeKind::Prefab(_) => {
                            out.insert(n);
                        }
                        NodeKind::Entity(_) if node.children.is_empty() => {
                            out.insert(n);
                        }
                        _ => {}
                    }
                }
            }
        }
        out.into_iter().collect()
    }

    pub fn selected_brushes(&self) -> Vec<NodeId> {
        self.selected_leaves().into_iter().filter(|id| self.map.get(*id).is_some_and(|n| n.kind.brush().is_some())).collect()
    }

    pub fn create_brush(&mut self, bounds: Aabb) -> Result<NodeId, EngineError> {
        let brush = Brush::cuboid(bounds, &self.tex);
        brush.solid()?;
        let mut e = self.map.edit();
        let id = e.insert(self.active_parent, NodeKind::Brush(brush))?;
        let c = e.finish();
        self.apply("Create brush", c);
        self.selection = BTreeSet::from([id]);
        Ok(id)
    }

    /// Creates the brushes of `shape` filling `bounds` and selects them.
    pub fn create_shape(&mut self, shape: Shape, bounds: Aabb) -> Result<Vec<NodeId>, EngineError> {
        let brushes = match shape {
            Shape::Block => vec![Brush::cuboid(bounds, &self.tex)],
            Shape::Wedge => vec![Brush::wedge(bounds, &self.tex)?],
            Shape::Cylinder(n) => vec![Brush::cylinder(bounds, n, &self.tex)?],
            Shape::Spike(n) => vec![Brush::spike(bounds, n, &self.tex)?],
            Shape::Sphere(n) => vec![Brush::sphere(bounds, n, &self.tex)?],
            Shape::Arch { sides, arc, start, thickness } => Brush::arch(bounds, sides, arc, start, thickness, &self.tex)?,
        };
        let mut e = self.map.edit();
        let mut ids = Vec::new();
        for b in brushes {
            b.solid()?;
            ids.push(e.insert(self.active_parent, NodeKind::Brush(b))?);
        }
        let c = e.finish();
        self.apply("Create brush", c);
        self.selection = ids.iter().copied().collect();
        Ok(ids)
    }

    /// Center of the selection's bounds, with point entities counted as their origin.
    pub fn selection_center(&self) -> Option<DVec3> {
        self.bounds_of(self.selection.iter().copied()).map(|b| b.center())
    }

    /// Bounds of `roots` and everything inside them, with point entities counted as their origin.
    pub(crate) fn bounds_of(&self, roots: impl IntoIterator<Item = NodeId>) -> Option<Aabb> {
        let mut pts = Vec::new();
        for id in self.leaves_of(roots) {
            match self.map.get(id).map(|n| &n.kind) {
                Some(NodeKind::Brush(b)) => {
                    if let Ok(bb) = b.bounds() {
                        pts.extend([bb.min, bb.max]);
                    }
                }
                Some(NodeKind::Entity(e)) => pts.extend(e.origin()),
                Some(NodeKind::Prefab(p)) => pts.push(p.origin),
                _ => {}
            }
        }
        Aabb::from_points(pts)
    }

    /// Rotates the selection about an axis through its center.
    pub fn rotate_selection(&mut self, axis: DVec3, degrees: f64) -> Result<(), EngineError> {
        let c = self.selection_center().ok_or(EngineError::NothingSelected)?;
        let m = DMat4::from_translation(c)
            * DMat4::from_quat(DQuat::from_axis_angle(axis.normalize_or(DVec3::Z), degrees.to_radians()))
            * DMat4::from_translation(-c);
        self.transform_selection(&m, "Rotate", None)
    }

    /// Mirrors the selection along a world axis (0, 1 or 2) through its center.
    pub fn flip_selection(&mut self, axis: usize) -> Result<(), EngineError> {
        let c = self.selection_center().ok_or(EngineError::NothingSelected)?;
        let mut s = DVec3::ONE;
        if let Some(v) = s.as_mut().get_mut(axis) {
            *v = -1.0;
        }
        let m = DMat4::from_translation(c) * DMat4::from_scale(s) * DMat4::from_translation(-c);
        self.transform_selection(&m, "Flip", None)
    }

    /// Replaces the selected brushes with the convex hull around them.
    pub fn merge_selection(&mut self) -> Result<NodeId, EngineError> {
        let ids = self.selected_brushes();
        let brushes: Vec<Brush> = ids.iter().filter_map(|id| self.map.get(*id)?.kind.brush().cloned()).collect();
        if brushes.len() < 2 {
            return Err(EngineError::NothingSelected);
        }
        let merged = Brush::merge(&brushes)?;
        let parent = ids.first().and_then(|id| self.map.get(*id)?.parent).unwrap_or(ROOT);
        let mut e = self.map.edit();
        for id in &ids {
            e.remove(*id)?;
        }
        let id = e.insert(parent, NodeKind::Brush(merged))?;
        let c = e.finish();
        self.apply("Merge", c);
        self.selection = BTreeSet::from([id]);
        Ok(id)
    }

    /// Moves brush corners: every selected brush corner at `from` goes to `to`, and each brush is
    /// rebuilt as the convex hull of its corners.
    pub fn move_vertices(&mut self, moves: &[(DVec3, DVec3)], coalesce: Option<&str>) -> Result<(), EngineError> {
        let ids = self.selected_brushes();
        let mut e = self.map.edit();
        for id in ids {
            let Some(b) = e.node(id).and_then(|n| n.kind.brush()).cloned() else { continue };
            let mut verts = b.vertices()?;
            let mut changed = false;
            for v in &mut verts {
                if let Some((_, to)) = moves.iter().find(|(from, _)| from.distance_squared(*v) < 1e-4) {
                    *v = *to;
                    changed = true;
                }
            }
            if changed {
                e.set_kind(id, NodeKind::Brush(Brush::from_vertices(&verts, &b.faces, &self.tex)?))?;
            }
        }
        let c = e.finish();
        self.apply_coalesced("Move vertices", c, coalesce);
        Ok(())
    }

    /// One step of a face drag. `faces` are the brushes as they were when the drag began, each
    /// with the face being dragged; `created` are the brushes the previous step added, which this
    /// step replaces. Returns the brushes this step added. Steps sharing `coalesce` undo together.
    pub fn extrude(
        &mut self,
        faces: &[(NodeId, Brush, FaceId)],
        distance: f64,
        mode: ExtrudeMode,
        created: &[NodeId],
        coalesce: Option<&str>,
    ) -> Result<Vec<NodeId>, EngineError> {
        let mut e = self.map.edit();
        for id in created {
            if e.node(*id).is_some() {
                e.remove(*id)?;
            }
        }
        let mut out = Vec::new();
        for (id, orig, face_id) in faces {
            let Some(face) = orig.faces.iter().find(|f| f.id == *face_id) else { continue };
            let Some(plane) = face.plane() else { continue };
            let parent = e.node(*id).and_then(|n| n.parent).unwrap_or(self.active_parent);
            let tex = mapcraft_map::TexDefaults { material: face.material.clone(), scale: face.u.scale };
            let new = match mode {
                ExtrudeMode::Resize => {
                    e.set_kind(*id, NodeKind::Brush(orig.extrude_face(*face_id, distance).ok_or(EngineError::Refused("that would remove a face"))?))?;
                    None
                }
                _ if distance.abs() < 1e-9 => {
                    e.set_kind(*id, NodeKind::Brush(orig.clone()))?;
                    None
                }
                ExtrudeMode::Split if distance > 0.0 => {
                    e.set_kind(*id, NodeKind::Brush(orig.clone()))?;
                    let grown = orig.extrude_face(*face_id, distance).ok_or(EngineError::Refused("that would remove a face"))?;
                    grown.split(face.points, &tex)?.0
                }
                ExtrudeMode::Split => {
                    let moved = face.points.map(|p| p + plane.normal * distance);
                    let (front, back) = orig.split(moved, &tex)?;
                    e.set_kind(*id, NodeKind::Brush(back.ok_or(EngineError::Refused("the split leaves nothing"))?))?;
                    front
                }
                ExtrudeMode::Stamp => Some(orig.stamp_face(*face_id, distance).ok_or(EngineError::Refused("the stamp has no volume"))?),
            };
            if let Some(b) = new {
                out.push(e.insert(parent, NodeKind::Brush(b.with_new_ids()))?);
            }
        }
        let c = e.finish();
        let label = match mode {
            ExtrudeMode::Resize => "Extrude",
            ExtrudeMode::Split => "Split",
            ExtrudeMode::Stamp => "Stamp",
        };
        self.apply_coalesced(label, c, coalesce);
        Ok(out)
    }

    /// Copies the selected subtrees as clipboard text.
    pub fn copy_selection(&self) -> Result<String, EngineError> {
        let sel = self.require_selection()?;
        let roots: Vec<NodeId> = sel.iter().copied().filter(|id| !self.map.ancestors(*id).iter().any(|a| self.selection.contains(a))).collect();
        Ok(mapcraft_format::fragment_to_string(&self.map, &roots)?)
    }

    /// Pastes clipboard text under the active parent, offset by `delta`, and selects the copies.
    pub fn paste(&mut self, text: &str, delta: DVec3) -> Result<(), EngineError> {
        let frags = mapcraft_format::fragment_from_str(text)?;
        let mut e = self.map.edit();
        let roots = insert_fragments(&mut e, self.active_parent, frags, &DMat4::from_translation(delta), self.texture_lock)?;
        let c = e.finish();
        self.apply("Paste", c);
        self.selection = roots.into_iter().collect();
        Ok(())
    }

    /// Edits every selected face in one undo step.
    pub fn edit_faces(&mut self, label: &str, coalesce: Option<&str>, f: impl Fn(&mut Face)) -> Result<(), EngineError> {
        if self.face_selection.is_empty() {
            return Err(EngineError::NothingSelected);
        }
        let mut e = self.map.edit();
        let brushes: BTreeSet<NodeId> = self.face_selection.iter().map(|(id, _)| *id).collect();
        for id in brushes {
            let Some(mut b) = e.node(id).and_then(|n| n.kind.brush()).cloned() else { continue };
            for face in b.faces.iter_mut().filter(|x| self.face_selection.contains(&(id, x.id))) {
                f(face);
            }
            e.set_kind(id, NodeKind::Brush(b))?;
        }
        let c = e.finish();
        self.apply_coalesced(label, c, coalesce);
        Ok(())
    }

    /// Gives one face a material (Hammer's apply-to-face click).
    pub fn apply_material_to_face(&mut self, id: NodeId, face: FaceId) -> Result<(), EngineError> {
        let Some(mut b) = self.map.get(id).and_then(|n| n.kind.brush()).cloned() else { return Ok(()) };
        for f in b.faces.iter_mut().filter(|f| f.id == face) {
            f.material = self.tex.material.clone();
        }
        self.set_kind(id, NodeKind::Brush(b), "Apply material", None)
    }

    pub fn create_point_entity(&mut self, classname: &str, origin: DVec3, properties: Vec<(String, String)>) -> Result<NodeId, EngineError> {
        let mut ent = Entity::new(classname);
        ent.properties = properties;
        ent.set("origin", fmt_vec(origin));
        let mut e = self.map.edit();
        let id = e.insert(self.active_parent, NodeKind::Entity(ent))?;
        let c = e.finish();
        self.apply("Create entity", c);
        self.selection = BTreeSet::from([id]);
        Ok(id)
    }

    pub fn delete_selection(&mut self) -> Result<(), EngineError> {
        let ids = self.require_selection()?;
        let mut e = self.map.edit();
        for id in ids {
            if e.node(id).is_some() {
                e.remove(id)?;
            }
        }
        let c = e.finish();
        self.apply("Delete", c);
        self.selection.clear();
        Ok(())
    }

    pub fn translate_selection(&mut self, delta: DVec3, coalesce: Option<&str>) -> Result<(), EngineError> {
        self.transform_selection(&DMat4::from_translation(delta), "Move", coalesce)
    }

    /// Applies an affine transform to everything selected, honoring texture lock.
    pub fn transform_selection(&mut self, m: &DMat4, label: &str, coalesce: Option<&str>) -> Result<(), EngineError> {
        let roots = self.require_selection()?;
        let mut e = self.map.edit();
        self.transform_nodes(&mut e, &roots, m)?;
        let c = e.finish();
        self.apply_coalesced(label, c, coalesce);
        Ok(())
    }

    /// Applies an affine transform to `roots` and everything inside them, honoring texture lock.
    fn transform_nodes(&self, e: &mut Edit, roots: &[NodeId], m: &DMat4) -> Result<(), EngineError> {
        for id in self.leaves_of(roots.iter().copied()) {
            let Some(kind) = e.node(id).map(|n| n.kind.clone()) else { continue };
            if let Some(new) = transform_kind(kind, m, self.texture_lock, is_point_entity(e, id))? {
                e.set_kind(id, new)?;
            }
        }
        let groups: BTreeSet<NodeId> = roots.iter().flat_map(|id| self.map.subtree(*id)).collect();
        for id in groups {
            let Some(NodeKind::Group(g)) = e.node(id).map(|n| n.kind.clone()) else { continue };
            let moved = moved_group(g.clone(), m);
            if moved != g {
                e.set_kind(id, NodeKind::Group(moved))?;
            }
        }
        Ok(())
    }

    /// Moves each selected object onto the grid without reshaping it: its brushes' lowest corner,
    /// or a point entity's or prefab's origin, lands on the nearest grid point.
    pub fn snap_selection_to_grid(&mut self) -> Result<(), EngineError> {
        self.require_selection()?;
        let roots: Vec<NodeId> = self.selection.iter().copied().filter(|id| !self.map.ancestors(*id).iter().any(|a| self.selection.contains(a))).collect();
        let mut e = self.map.edit();
        for id in roots {
            let leaves = self.leaves_of([id]);
            let corner = leaves.iter().filter_map(|l| self.map.get(*l)?.kind.brush()?.bounds().ok()).reduce(|a, b| a.union(&b)).map(|b| b.min);
            let origin = || {
                leaves.iter().find_map(|l| match &self.map.get(*l)?.kind {
                    NodeKind::Entity(ent) => ent.origin(),
                    NodeKind::Prefab(p) => Some(p.origin),
                    _ => None,
                })
            };
            let Some(anchor) = corner.or_else(origin) else { continue };
            let delta = mapcraft_geom::snap(anchor, self.grid) - anchor;
            if delta.length_squared() > 1e-12 {
                self.transform_nodes(&mut e, &[id], &DMat4::from_translation(delta))?;
            }
        }
        let c = e.finish();
        if !c.is_empty() {
            self.apply("Snap to grid", c);
        }
        Ok(())
    }

    /// Copies the selection, offset by `delta`, and selects the copies.
    pub fn duplicate_selection(&mut self, delta: DVec3) -> Result<(), EngineError> {
        let roots = self.require_selection()?;
        let mut e = self.map.edit();
        let mut copies = BTreeSet::new();
        for root in roots {
            let Some(parent) = e.node(root).and_then(|n| n.parent) else { continue };
            let mut stack = vec![(root, parent)];
            let mut first = true;
            while let Some((src, dst_parent)) = stack.pop() {
                let Some(node) = e.node(src).cloned() else { continue };
                let kind = match node.kind {
                    NodeKind::Brush(b) => {
                        let mut b = b.translated(delta, self.texture_lock);
                        b.faces.iter_mut().for_each(|f| f.id = Default::default());
                        NodeKind::Brush(b)
                    }
                    NodeKind::Entity(mut ent) if node.children.is_empty() => {
                        ent.set("origin", fmt_vec(ent.origin().unwrap_or_default() + delta));
                        NodeKind::Entity(ent)
                    }
                    NodeKind::Prefab(mut p) => {
                        p.origin += delta;
                        NodeKind::Prefab(p)
                    }
                    NodeKind::Group(g) => NodeKind::Group(moved_group(g, &DMat4::from_translation(delta))),
                    other => other,
                };
                let id = e.insert(dst_parent, kind)?;
                if std::mem::take(&mut first) {
                    copies.insert(id);
                }
                stack.extend(node.children.iter().rev().map(|c| (*c, id)));
            }
        }
        let c = e.finish();
        self.apply("Duplicate", c);
        self.selection = copies;
        Ok(())
    }

    fn replace_brush(e: &mut Edit, id: NodeId, parts: Vec<Brush>) -> Result<Vec<NodeId>, EngineError> {
        let parent = e.node(id).and_then(|n| n.parent).ok_or(mapcraft_map::EditError::Missing(id))?;
        let index = e.node(parent).and_then(|p| p.children.iter().position(|c| *c == id)).unwrap_or(0);
        e.remove(id)?;
        let mut out = Vec::new();
        for (i, part) in parts.into_iter().enumerate() {
            out.push(e.insert_at(parent, index + i, NodeKind::Brush(part))?);
        }
        Ok(out)
    }

    /// Subtracts the selected brushes from every other visible brush they touch (Hammer's carve).
    pub fn carve(&mut self) -> Result<(), EngineError> {
        let cutters: Vec<Brush> = self.selected_brushes().iter().filter_map(|id| self.map.get(*id)?.kind.brush().cloned()).collect();
        if cutters.is_empty() {
            return Err(EngineError::NothingSelected);
        }
        let cutter_ids: BTreeSet<NodeId> = self.selected_brushes().into_iter().collect();
        let mut e = self.map.edit();
        let targets: Vec<(NodeId, Brush)> = self
            .map
            .nodes()
            .filter(|(id, n)| !cutter_ids.contains(id) && !n.hidden && !n.locked)
            .filter_map(|(id, n)| Some((id, n.kind.brush()?.clone())))
            .collect();
        for (id, brush) in targets {
            let mut parts = vec![brush.clone()];
            for cutter in &cutters {
                parts = parts.iter().map(|p| p.subtract(cutter)).collect::<Result<Vec<_>, _>>()?.into_iter().flatten().collect();
            }
            if parts != [brush] {
                Self::replace_brush(&mut e, id, parts)?;
            }
        }
        let c = e.finish();
        self.apply("Carve", c);
        Ok(())
    }

    pub fn hollow(&mut self, thickness: f64) -> Result<(), EngineError> {
        let brushes = self.selected_brushes();
        if brushes.is_empty() {
            return Err(EngineError::NothingSelected);
        }
        let mut e = self.map.edit();
        let mut walls = BTreeSet::new();
        for id in brushes {
            let Some(b) = e.node(id).and_then(|n| n.kind.brush()).cloned() else { continue };
            walls.extend(Self::replace_brush(&mut e, id, b.hollow(thickness)?)?);
        }
        let c = e.finish();
        self.apply("Hollow", c);
        self.selection = walls;
        Ok(())
    }

    /// Splits the selected brushes along the plane through three points (clockwise seen from the front).
    pub fn clip(&mut self, points: [DVec3; 3], keep: ClipKeep) -> Result<(), EngineError> {
        let brushes = self.selected_brushes();
        if brushes.is_empty() {
            return Err(EngineError::NothingSelected);
        }
        let mut e = self.map.edit();
        let mut kept = BTreeSet::new();
        for id in brushes {
            let Some(b) = e.node(id).and_then(|n| n.kind.brush()).cloned() else { continue };
            let (front, back) = b.split(points, &self.tex)?;
            let parts: Vec<Brush> = match keep {
                ClipKeep::Front => front.into_iter().collect(),
                ClipKeep::Back => back.into_iter().collect(),
                ClipKeep::Both => front.into_iter().chain(back).collect(),
            };
            kept.extend(Self::replace_brush(&mut e, id, parts)?);
        }
        let c = e.finish();
        self.apply("Clip", c);
        self.selection = kept;
        Ok(())
    }

    pub fn group_selection(&mut self) -> Result<NodeId, EngineError> {
        self.wrap_selection(Group { name: "Group".into(), ..Group::default() }, "Group")
    }

    /// Moves the selection into a new group and selects it.
    pub(crate) fn wrap_selection(&mut self, group: Group, label: &str) -> Result<NodeId, EngineError> {
        let ids = self.require_selection()?;
        let parent = ids.first().and_then(|id| self.map.get(*id)?.parent).ok_or(EngineError::NothingSelected)?;
        let mut e = self.map.edit();
        let group = e.insert(parent, NodeKind::Group(group))?;
        for id in ids {
            e.reparent(id, group, None)?;
        }
        let c = e.finish();
        self.apply(label, c);
        self.selection = BTreeSet::from([group]);
        Ok(group)
    }

    pub fn ungroup_selection(&mut self) -> Result<(), EngineError> {
        let ids = self.require_selection()?;
        let mut e = self.map.edit();
        let mut freed = BTreeSet::new();
        for id in ids {
            let Some(node) = e.node(id).cloned() else { continue };
            let (NodeKind::Group(_), Some(parent)) = (&node.kind, node.parent) else { continue };
            let index = e.node(parent).and_then(|p| p.children.iter().position(|c| *c == id)).unwrap_or(0);
            for (i, child) in node.children.iter().enumerate() {
                e.reparent(*child, parent, Some(index + i))?;
                freed.insert(*child);
            }
            e.remove(id)?;
        }
        let c = e.finish();
        self.apply("Ungroup", c);
        self.selection = freed;
        Ok(())
    }

    /// Moves the selected world brushes into a new brush entity.
    pub fn tie_to_entity(&mut self, classname: &str) -> Result<NodeId, EngineError> {
        let brushes: Vec<NodeId> = self
            .selected_brushes()
            .into_iter()
            .filter(|id| self.map.ancestors(*id).first().and_then(|p| self.map.get(*p)).is_some_and(|p| !matches!(p.kind, NodeKind::Entity(_))))
            .collect();
        let parent = brushes.first().and_then(|id| self.map.get(*id)?.parent).ok_or(EngineError::NothingSelected)?;
        let mut e = self.map.edit();
        let ent = e.insert(parent, NodeKind::Entity(Entity::new(classname)))?;
        for id in brushes {
            e.reparent(id, ent, None)?;
        }
        let c = e.finish();
        self.apply("Tie to entity", c);
        self.selection = BTreeSet::from([ent]);
        Ok(ent)
    }

    /// A `tools/trigger` brush entity of `classname` standing on the selected brushes, `height`
    /// tall: a zone fitted to a platform.
    pub fn zone_from_selection(&mut self, classname: &str, height: f64) -> Result<NodeId, EngineError> {
        if classname == "func_detail" {
            return Err(EngineError::Refused("pick a trigger class such as zone_start in the Entities tab first"));
        }
        let b = self
            .selected_brushes()
            .iter()
            .filter_map(|id| self.map.get(*id)?.kind.brush()?.bounds().ok())
            .reduce(|a, b| a.union(&b))
            .ok_or(EngineError::NothingSelected)?;
        let zone = Aabb::new(DVec3::new(b.min.x, b.min.y, b.max.z), DVec3::new(b.max.x, b.max.y, b.max.z + height));
        let tex = TexDefaults { material: "tools/trigger".into(), scale: self.tex.scale };
        let mut e = self.map.edit();
        let ent = e.insert(self.active_parent, NodeKind::Entity(Entity::new(classname)))?;
        e.insert(ent, NodeKind::Brush(Brush::cuboid(zone, &tex)))?;
        let c = e.finish();
        self.apply(&format!("Zone {classname}"), c);
        self.selection = BTreeSet::from([ent]);
        Ok(ent)
    }

    /// Turns selected brush entities back into world brushes.
    pub fn move_to_world(&mut self) -> Result<(), EngineError> {
        let ids = self.require_selection()?;
        let mut e = self.map.edit();
        let mut freed = BTreeSet::new();
        for id in ids {
            let Some(node) = e.node(id).cloned() else { continue };
            let (NodeKind::Entity(_), Some(parent), false) = (&node.kind, node.parent, node.children.is_empty()) else { continue };
            for child in &node.children {
                e.reparent(*child, parent, None)?;
                freed.insert(*child);
            }
            e.remove(id)?;
        }
        let c = e.finish();
        self.apply("Move to world", c);
        self.selection = freed;
        Ok(())
    }

    pub fn set_property(&mut self, id: NodeId, key: &str, value: &str) -> Result<(), EngineError> {
        let mut e = self.map.edit();
        let kind = e.node(id).map(|n| n.kind.clone()).ok_or(mapcraft_map::EditError::Missing(id))?;
        let kind = match kind {
            NodeKind::Entity(mut ent) => {
                ent.set(key, value);
                NodeKind::Entity(ent)
            }
            NodeKind::World(mut ent) => {
                ent.set(key, value);
                NodeKind::World(ent)
            }
            _ => return Ok(()),
        };
        e.set_kind(id, kind)?;
        let c = e.finish();
        self.apply(&format!("Set {key}"), c);
        Ok(())
    }

    /// Replaces a node's content, e.g. an edited entity or a renamed group; edits with the same
    /// `coalesce` key merge into one undo step, so typing into a field is undone in one go.
    pub fn set_kind(&mut self, id: NodeId, kind: NodeKind, label: &str, coalesce: Option<&str>) -> Result<(), EngineError> {
        if self.map.get(id).is_some_and(|n| n.kind == kind) {
            return Ok(());
        }
        let mut e = self.map.edit();
        e.set_kind(id, kind)?;
        let c = e.finish();
        self.apply_coalesced(label, c, coalesce);
        Ok(())
    }

    pub fn set_hidden(&mut self, ids: &[NodeId], hidden: bool) -> Result<(), EngineError> {
        let mut e = self.map.edit();
        for id in ids {
            e.update(*id, |h, _, _, _| *h = hidden)?;
        }
        let c = e.finish();
        self.apply(if hidden { "Hide" } else { "Show" }, c);
        if hidden {
            let hidden_set: BTreeSet<NodeId> = ids.iter().flat_map(|id| self.map.subtree(*id)).collect();
            self.selection.retain(|id| !hidden_set.contains(id));
        }
        Ok(())
    }

    /// Hides every brush, entity and prefab outside the selection (Hammer's isolate).
    pub fn isolate_selection(&mut self) -> Result<(), EngineError> {
        self.require_selection()?;
        let keep: BTreeSet<NodeId> = self.selection.iter().flat_map(|id| self.map.subtree(*id).into_iter().chain(self.map.ancestors(*id))).collect();
        let hide: Vec<NodeId> = self
            .map
            .nodes()
            .filter(|(id, n)| !keep.contains(id) && !n.hidden && matches!(n.kind, NodeKind::Brush(_) | NodeKind::Entity(_) | NodeKind::Prefab(_)))
            .map(|(id, _)| id)
            .collect();
        self.set_hidden(&hide, true)
    }

    pub fn show_all(&mut self) -> Result<(), EngineError> {
        let hidden: Vec<NodeId> = self.map.nodes().filter(|(_, n)| n.hidden).map(|(id, _)| id).collect();
        self.set_hidden(&hidden, false)
    }

    pub fn create_layer(&mut self, name: &str) -> Result<NodeId, EngineError> {
        let mut e = self.map.edit();
        let id = e.insert(mapcraft_map::ROOT, NodeKind::Layer(mapcraft_map::Layer { name: name.into(), ..Default::default() }))?;
        let c = e.finish();
        self.apply("Create layer", c);
        self.active_parent = id;
        Ok(id)
    }

    /// Gives the current material to the picked faces, else to every face of the selected brushes.
    pub fn apply_material(&mut self) -> Result<(), EngineError> {
        if !self.face_selection.is_empty() {
            let m = self.tex.material.clone();
            return self.edit_faces("Apply material", None, |f| f.material = m.clone());
        }
        let brushes = self.selected_brushes();
        if brushes.is_empty() {
            return Err(EngineError::NothingSelected);
        }
        let mut e = self.map.edit();
        for id in brushes {
            let Some(mut b) = e.node(id).and_then(|n| n.kind.brush()).cloned() else { continue };
            b.faces.iter_mut().for_each(|f| f.material = self.tex.material.clone());
            e.set_kind(id, NodeKind::Brush(b))?;
        }
        let c = e.finish();
        self.apply("Apply material", c);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use mapcraft_map::{ROOT, TexDefaults};

    use super::*;

    fn volume(s: &Session) -> f64 {
        s.map.nodes().filter_map(|(_, n)| n.kind.brush()).map(|b| b.solid().unwrap().volume()).sum()
    }

    fn session() -> Session {
        let mut s = Session::default();
        s.tex = TexDefaults { material: "dev/grid".into(), scale: 0.25 };
        s
    }

    #[test]
    fn apply_material_prefers_picked_faces() {
        let mut s = session();
        let b = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
        let face = s.map.get(b).unwrap().kind.brush().unwrap().faces[0].id;
        s.selection.clear();
        s.face_selection = BTreeSet::from([(b, face)]);
        s.tex.material = "ramps/orange".into();
        s.apply_material().unwrap();
        let mats: Vec<String> = s.map.get(b).unwrap().kind.brush().unwrap().faces.iter().map(|f| f.material.clone()).collect();
        assert_eq!(mats.iter().filter(|m| *m == "ramps/orange").count(), 1, "only the picked face changes: {mats:?}");
    }

    #[test]
    fn zones_stand_on_the_selected_platform() {
        let mut s = session();
        let platform = s.create_brush(Aabb::new(DVec3::new(0.0, 0.0, -32.0), DVec3::new(512.0, 256.0, 0.0))).unwrap();
        s.selection = BTreeSet::from([platform]);
        assert!(s.zone_from_selection("func_detail", 256.0).is_err(), "the default class is not a zone");
        let zone = s.zone_from_selection("zone_start", 256.0).unwrap();
        let node = s.map.get(zone).unwrap();
        assert_eq!(node.kind.entity().unwrap().classname, "zone_start");
        let b = s.map.get(node.children[0]).unwrap().kind.brush().unwrap();
        assert_eq!(b.bounds().unwrap(), Aabb::new(DVec3::ZERO, DVec3::new(512.0, 256.0, 256.0)));
        assert!(b.faces.iter().all(|f| f.material == "tools/trigger"));
    }

    #[test]
    fn snap_selection_to_grid_moves_each_object() {
        let mut s = session();
        s.grid = 16.0;
        let loose = s.create_brush(Aabb::new(DVec3::new(3.0, 5.0, 0.0), DVec3::new(67.0, 69.0, 64.0))).unwrap();
        let ent = s.create_point_entity("info_player_start", DVec3::new(10.0, 30.0, 7.0), vec![]).unwrap();
        let inner = s.create_brush(Aabb::new(DVec3::new(100.0, 100.0, 0.0), DVec3::new(140.0, 140.0, 40.0))).unwrap();
        s.selection = BTreeSet::from([inner]);
        let group = s.group_selection().unwrap();
        s.select_all();
        assert_eq!(s.selection, BTreeSet::from([loose, ent, group]));
        s.snap_selection_to_grid().unwrap();
        let bounds = |s: &Session, id| s.map.get(id).unwrap().kind.brush().unwrap().bounds().unwrap();
        assert_eq!((bounds(&s, loose).min, bounds(&s, loose).max), (DVec3::ZERO, DVec3::splat(64.0)), "moved, not reshaped");
        assert_eq!(s.map.get(ent).unwrap().kind.entity().unwrap().origin(), Some(DVec3::new(16.0, 32.0, 0.0)));
        assert_eq!(bounds(&s, inner).min, DVec3::new(96.0, 96.0, 0.0), "a group moves as one by its lowest corner");
        let undo = s.undo_label().map(str::to_string);
        s.snap_selection_to_grid().unwrap();
        assert_eq!(s.undo_label().map(str::to_string), undo, "snapping what is on the grid adds no undo step");
    }

    #[test]
    fn create_move_duplicate_delete() {
        let mut s = session();
        let b = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
        let ent = s.create_point_entity("info_player_start", DVec3::new(32.0, 32.0, 0.0), vec![]).unwrap();
        s.selection = BTreeSet::from([b, ent]);
        s.translate_selection(DVec3::new(16.0, 0.0, 0.0), None).unwrap();
        assert_eq!(s.map.get(ent).unwrap().kind.entity().unwrap().get("origin"), Some("48 32 0"));
        assert_eq!(s.map.get(b).unwrap().kind.brush().unwrap().bounds().unwrap().min.x, 16.0);
        s.duplicate_selection(DVec3::new(0.0, 128.0, 0.0)).unwrap();
        assert_eq!(s.selection.len(), 2);
        assert!(!s.selection.contains(&b));
        assert_eq!(s.map.len(), 5);
        s.delete_selection().unwrap();
        assert_eq!(s.map.len(), 3);
        s.undo();
        s.undo();
        assert_eq!(s.map.len(), 3);
        assert!(s.selection.contains(&b));
    }

    #[test]
    fn carve_hollow_clip() {
        let mut s = session();
        s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
        s.create_brush(Aabb::new(DVec3::splat(48.0), DVec3::splat(80.0))).unwrap();
        s.carve().unwrap();
        let expected = 64.0f64.powi(3) - 16.0f64.powi(3) + 32.0f64.powi(3);
        assert!((volume(&s) - expected).abs() < 1e-6);
        s.undo();
        assert!((volume(&s) - (64.0f64.powi(3) + 32.0f64.powi(3))).abs() < 1e-6);

        s.hollow(8.0).unwrap();
        assert_eq!(s.selection.len(), 6);

        let mut s = session();
        s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
        let pts = [DVec3::new(32.0, 1.0, 0.0), DVec3::new(32.0, 0.0, 0.0), DVec3::new(32.0, 0.0, 1.0)];
        s.clip(pts, ClipKeep::Both).unwrap();
        assert_eq!(s.selection.len(), 2);
        s.clip(pts, ClipKeep::Back).unwrap();
        assert_eq!(s.selection.len(), 1);
        assert!((volume(&s) - 64.0f64.powi(3) / 2.0).abs() < 1e-6);
    }

    #[test]
    fn groups_and_entities() {
        let mut s = session();
        let a = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
        let b = s.create_brush(Aabb::new(DVec3::splat(100.0), DVec3::splat(164.0))).unwrap();
        s.selection = BTreeSet::from([a, b]);
        let g = s.group_selection().unwrap();
        assert_eq!(s.map.get(g).unwrap().children, vec![a, b]);
        s.ungroup_selection().unwrap();
        assert!(s.map.get(g).is_none());
        assert_eq!(s.map.get(a).unwrap().parent, Some(ROOT));

        s.selection = BTreeSet::from([a]);
        let ent = s.tie_to_entity("zone_start").unwrap();
        assert_eq!(s.map.get(a).unwrap().parent, Some(ent));
        s.set_property(ent, "course_id", "2").unwrap();
        assert_eq!(s.map.get(ent).unwrap().kind.entity().unwrap().get("course_id"), Some("2"));
        s.move_to_world().unwrap();
        assert!(s.map.get(ent).is_none());

        s.tex.material = "ramps/orange".into();
        s.selection = BTreeSet::from([a]);
        s.apply_material().unwrap();
        assert!(s.map.get(a).unwrap().kind.brush().unwrap().faces.iter().all(|f| f.material == "ramps/orange"));
        assert!(matches!(Session::default().carve(), Err(EngineError::NothingSelected)));
    }

    #[test]
    fn clipboard_rotate_merge_vertices() {
        let mut s = session();
        let a = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::new(64.0, 32.0, 16.0))).unwrap();
        let text = s.copy_selection().unwrap();
        s.paste(&text, DVec3::new(0.0, 0.0, 64.0)).unwrap();
        assert_eq!(s.map.len(), 3);
        let copy = *s.selection.iter().next().unwrap();
        assert_ne!(copy, a);
        assert_eq!(s.map.get(copy).unwrap().kind.brush().unwrap().bounds().unwrap().min.z, 64.0);

        s.selection = BTreeSet::from([a]);
        s.rotate_selection(DVec3::Z, 90.0).unwrap();
        let b = s.map.get(a).unwrap().kind.brush().unwrap().bounds().unwrap();
        assert!((b.size().x - 32.0).abs() < 1e-6 && (b.size().y - 64.0).abs() < 1e-6);
        s.flip_selection(0).unwrap();
        assert!((s.map.get(a).unwrap().kind.brush().unwrap().solid().unwrap().volume() - 64.0 * 32.0 * 16.0).abs() < 1e-6);

        let mut s = session();
        let a = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(32.0))).unwrap();
        let b = s.create_brush(Aabb::new(DVec3::new(32.0, 0.0, 0.0), DVec3::new(64.0, 32.0, 32.0))).unwrap();
        s.selection = BTreeSet::from([a, b]);
        let m = s.merge_selection().unwrap();
        assert_eq!(s.map.len(), 2);
        s.move_vertices(&[(DVec3::new(64.0, 32.0, 32.0), DVec3::new(64.0, 32.0, 64.0))], None).unwrap();
        assert!(s.map.get(m).unwrap().kind.brush().unwrap().bounds().unwrap().max.z == 64.0);

        let w = s.create_shape(Shape::Wedge, Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap()[0];
        let face = s.map.get(w).unwrap().kind.brush().unwrap().faces[0].id;
        s.face_selection = BTreeSet::from([(w, face)]);
        s.edit_faces("Scale", None, |f| f.u.scale = 2.0).unwrap();
        assert_eq!(s.map.get(w).unwrap().kind.brush().unwrap().faces[0].u.scale, 2.0);

        let block = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
        let b = s.map.get(block).unwrap().kind.brush().unwrap().clone();
        let top = b.faces.iter().find(|f| f.plane().unwrap().normal.z > 0.99).unwrap().id;
        let faces = vec![(block, b.clone(), top)];
        assert!(s.extrude(&faces, 16.0, ExtrudeMode::Resize, &[], Some("x")).unwrap().is_empty());
        s.extrude(&faces, 32.0, ExtrudeMode::Resize, &[], Some("x")).unwrap();
        assert_eq!(s.map.get(block).unwrap().kind.brush().unwrap().bounds().unwrap().max.z, 96.0);
        assert!(s.extrude(&faces, -64.0, ExtrudeMode::Resize, &[], Some("x")).is_err());
        s.end_coalesce();
        s.undo();
        let made = s.extrude(&faces, 32.0, ExtrudeMode::Split, &[], Some("y")).unwrap();
        let made = s.extrude(&faces, 48.0, ExtrudeMode::Split, &made, Some("y")).unwrap();
        assert_eq!(made.len(), 1);
        assert_eq!(s.map.get(made[0]).unwrap().kind.brush().unwrap().bounds().unwrap().min.z, 64.0);
        assert_eq!(s.map.get(block).unwrap().kind.brush().unwrap().bounds().unwrap().max.z, 64.0);
        let inward = s.extrude(&faces, -16.0, ExtrudeMode::Split, &made, Some("y")).unwrap();
        assert_eq!(s.map.get(block).unwrap().kind.brush().unwrap().bounds().unwrap().max.z, 48.0);
        assert_eq!(s.map.get(inward[0]).unwrap().kind.brush().unwrap().bounds().unwrap().min.z, 48.0);
        s.end_coalesce();
        s.undo();
        assert_eq!(s.map.get(block).unwrap().kind.brush().unwrap().bounds().unwrap().max.z, 64.0);
        assert!(s.map.get(inward[0]).is_none(), "a whole drag undoes in one step");

        let arch = Shape::Arch { sides: 6, arc: 180.0, start: 0.0, thickness: 16.0 };
        let ids = s.create_shape(arch, Aabb::new(DVec3::ZERO, DVec3::new(256.0, 256.0, 32.0))).unwrap();
        assert_eq!(ids.len(), 6);
        assert_eq!(s.selection.len(), 6);
    }

    #[test]
    fn hide_isolate_and_typing() {
        let mut s = session();
        let a = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
        let b = s.create_brush(Aabb::new(DVec3::splat(100.0), DVec3::splat(164.0))).unwrap();
        s.selection = BTreeSet::from([a]);
        s.isolate_selection().unwrap();
        assert!(s.map.get(b).unwrap().hidden && !s.map.get(a).unwrap().hidden);
        s.show_all().unwrap();
        assert!(!s.map.get(b).unwrap().hidden);

        let ent = s.create_point_entity("info_target", DVec3::ZERO, vec![]).unwrap();
        for name in ["s", "sp", "spa", "spawn"] {
            let NodeKind::Entity(mut e) = s.map.get(ent).unwrap().kind.clone() else { panic!() };
            e.set("targetname", name);
            s.set_kind(ent, NodeKind::Entity(e), "Edit targetname", Some("prop:targetname")).unwrap();
        }
        assert_eq!(s.map.get(ent).unwrap().kind.entity().unwrap().get("targetname"), Some("spawn"));
        s.undo();
        assert_eq!(s.map.get(ent).unwrap().kind.entity().unwrap().get("targetname"), None);
    }
}
