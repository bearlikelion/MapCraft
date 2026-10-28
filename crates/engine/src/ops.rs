use std::collections::BTreeSet;

use mapcraft_geom::{Aabb, DMat4, DQuat, DVec3, Solid, Winding};
use mapcraft_map::{Brush, BrushError, Edit, Entity, Face, FaceId, Group, NodeId, NodeKind, ROOT, TexDefaults};

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

impl Shape {
    /// The brushes this shape makes filling `bounds`, without adding them to a map.
    pub fn brushes(self, bounds: Aabb, tex: &TexDefaults) -> Result<Vec<Brush>, BrushError> {
        Ok(match self {
            Shape::Block => vec![Brush::cuboid(bounds, tex)],
            Shape::Wedge => vec![Brush::wedge(bounds, tex)?],
            Shape::Cylinder(n) => vec![Brush::cylinder(bounds, n, tex)?],
            Shape::Spike(n) => vec![Brush::spike(bounds, n, tex)?],
            Shape::Sphere(n) => vec![Brush::sphere(bounds, n, tex)?],
            Shape::Arch { sides, arc, start, thickness } => Brush::arch(bounds, sides, arc, start, thickness, tex)?,
        })
    }
}

/// Whether the brushes fill `hull`; overlaps count once per pair, so triple overlaps can only refuse a fine merge.
fn fills(brushes: &[Brush], hull: &Brush) -> Result<bool, BrushError> {
    let volume = |b: &Brush| b.solid().map(|s| s.volume());
    let mut covered = 0.0;
    for (i, a) in brushes.iter().enumerate() {
        covered += volume(a)?;
        for b in &brushes[i + 1..] {
            covered -= a.intersect(b).map_or(Ok(0.0), |both| volume(&both))?;
        }
    }
    let total = volume(hull)?;
    Ok(total - covered <= total * 1e-6 + 1.0)
}

fn displaced_faces(brushes: &[Brush]) -> usize {
    brushes.iter().flat_map(|b| &b.faces).filter(|f| f.disp.is_some()).count()
}

fn flattened_notice(n: usize) -> Option<String> {
    match n {
        0 => None,
        1 => Some("1 displacement changed shape and came out flat".into()),
        n => Some(format!("{n} displacements changed shape and came out flat")),
    }
}

/// What dragging a face does (TrenchBroom's extrude tool).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtrudeMode {
    /// Move the face's plane, stretching the faces around it.
    Resize,
    /// Push the face along its normal keeping its shape, adding sides square to it.
    AlongNormal,
    /// Leave the brush and add a new one in the dragged volume, or split it when dragged inward.
    Split,
    /// Add a new brush between the face and a moved copy of it.
    Stamp,
    /// Move the face's corners so it keeps its shape; the faces around it tilt to follow.
    Slide,
}

/// How far a face drag moves its faces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Push {
    /// Each face this far along its own normal.
    Distance(f64),
    /// Every face by one world offset, so faces across a seam move together; all but a slide use only its part along each face's normal.
    Offset(DVec3),
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
    g.link = g.link.map(|l| Box::new(l.transformed(m)));
    g
}

/// A brush, entity, prefab or group placement moved by `m`; `None` for kinds placed by their children.
pub(crate) fn transform_kind(kind: NodeKind, m: &DMat4, lock: bool, point: bool) -> Result<Option<NodeKind>, EngineError> {
    Ok(Some(match kind {
        NodeKind::Brush(b) => NodeKind::Brush(b.transformed(m, lock)?),
        // Copies of a curve group keep its brushes but stop following its sources.
        NodeKind::Group(g) => NodeKind::Group(Group { curve: None, ..moved_group(g, m) }),
        NodeKind::Entity(mut ent) if point => {
            let origin = ent.origin().unwrap_or_default();
            ent.set("origin", fmt_vec(m.transform_point3(origin)));
            let turn = m.x_axis.y.atan2(m.x_axis.x).to_degrees();
            if turn.abs() > 1e-9 && m.determinant() > 0.0 {
                if let Some(a) = ent.get("angles").and_then(mapcraft_map::parse_vec3) {
                    ent.set("angles", fmt_vec(moved_placement(m, origin, a).1));
                }
                // -1 and -2 mean straight up and straight down, which a turn about Z leaves alone.
                if let Some(yaw) = ent.get("angle").and_then(|s| s.trim().parse::<f64>().ok()).filter(|y| y.is_finite() && *y >= 0.0) {
                    ent.set("angle", fmt_num(moved_placement(m, origin, DVec3::new(0.0, yaw, 0.0)).1.y));
                }
            }
            NodeKind::Entity(ent)
        }
        // A brush entity's origin is the hinge its brushes turn about, so it goes where they go.
        NodeKind::Entity(mut ent) => {
            let Some(origin) = ent.origin() else { return Ok(None) };
            ent.set("origin", fmt_vec(m.transform_point3(origin)));
            NodeKind::Entity(ent)
        }
        NodeKind::Prefab(mut p) => {
            (p.origin, p.angles) = moved_placement(m, p.origin, p.angles);
            NodeKind::Prefab(p)
        }
        NodeKind::Path(p) => NodeKind::Path(p.transformed(m)),
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
                        NodeKind::Brush(_) | NodeKind::Prefab(_) | NodeKind::Path(_) => {
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
        let brushes = shape.brushes(bounds, &self.tex)?;
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
        let roots: Vec<NodeId> = roots.into_iter().collect();
        let linked = roots.iter().any(|r| self.map.subtree(*r).iter().any(|n| mapcraft_map::link::copy_of(&self.map, *n).is_some()));
        // Linked copies hold nothing real, so their bounds come from the expanded map.
        let full = linked.then(|| mapcraft_map::mirror::expand(&self.map).0);
        let map = full.as_ref().unwrap_or(&self.map);
        let mut pts = Vec::new();
        let leaves: BTreeSet<NodeId> = roots.iter().flat_map(|r| map.subtree(*r)).collect();
        for id in leaves {
            match map.get(id).map(|n| &n.kind) {
                Some(NodeKind::Brush(b)) => {
                    if let Ok(bb) = b.bounds() {
                        pts.extend([bb.min, bb.max]);
                    }
                }
                Some(NodeKind::Entity(e)) if map.get(id).is_some_and(|n| n.children.is_empty()) => pts.extend(e.origin()),
                Some(NodeKind::Prefab(p)) => pts.push(p.origin),
                Some(NodeKind::Path(p)) => pts.extend(p.bounds().map(|b| [b.min, b.max]).into_iter().flatten()),
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
        let flattened = displaced_faces(&brushes).saturating_sub(displaced_faces(std::slice::from_ref(&merged)));
        if !fills(&brushes, &merged)? {
            return Err(EngineError::Refused("those brushes don't make one convex shape, so merging would fill in the gaps; group them instead"));
        }
        let parent = ids.first().and_then(|id| self.map.get(*id)?.parent).unwrap_or(ROOT);
        let mut e = self.map.edit();
        for id in &ids {
            e.remove(*id)?;
        }
        let id = e.insert(parent, NodeKind::Brush(merged))?;
        let c = e.finish();
        self.apply("Merge", c);
        self.notice = flattened_notice(flattened);
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

    /// The selected faces a drag on `face` moves, with their polygons: those in its plane and opposite ones overlapping it across a seam.
    pub fn extrude_group(&self, id: NodeId, face: FaceId) -> Vec<(NodeId, FaceId, Winding)> {
        let solids: Vec<(NodeId, &Brush, Solid)> = self
            .selected_brushes()
            .into_iter()
            .filter_map(|b| {
                let brush = self.map.get(b)?.kind.brush()?;
                Some((b, brush, brush.solid().ok()?))
            })
            .collect();
        let picked = solids.iter().find(|(b, ..)| *b == id).and_then(|(_, brush, solid)| {
            let i = brush.faces.iter().position(|f| f.id == face)?;
            Some((brush.faces.get(i)?.plane()?, solid.faces.get(i)?.clone()?))
        });
        let Some((plane, outline)) = picked else { return Vec::new() };
        let mut out = Vec::new();
        for (b, brush, solid) in &solids {
            for (f, w) in brush.faces.iter().zip(&solid.faces) {
                let (Some(w), Some(p)) = (w, f.plane()) else { continue };
                if p.approx_eq(&plane, 1e-3) || (p.approx_eq(&plane.flipped(), 1e-3) && w.overlaps(&outline)) {
                    out.push((*b, f.id, w.clone()));
                }
            }
        }
        out
    }

    /// One step of a face drag. `faces` are the brushes as they were when the drag began, each
    /// with the face being dragged; `created` are the brushes the previous step added, which this
    /// step replaces. Returns the brushes this step added. Steps sharing `coalesce` undo together.
    pub fn extrude(
        &mut self,
        faces: &[(NodeId, Brush, FaceId)],
        push: Push,
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
        let lock = self.texture_lock;
        let planes: Vec<_> = faces.iter().filter_map(|(_, b, f)| b.faces.iter().find(|x| x.id == *f)?.plane()).collect();
        if mode == ExtrudeMode::Stamp && planes.iter().any(|p| planes.iter().any(|q| p.approx_eq(&q.flipped(), 1e-3))) {
            return Err(EngineError::Refused("stamps cannot cross a seam"));
        }
        let mut out = Vec::new();
        for (id, orig, face_id) in faces {
            let Some(face) = orig.faces.iter().find(|f| f.id == *face_id) else { continue };
            let Some(plane) = face.plane() else { continue };
            let shift = match push {
                Push::Distance(d) => plane.normal * d,
                Push::Offset(v) => v,
            };
            let distance = shift.dot(plane.normal);
            let still = if mode == ExtrudeMode::Slide { shift.length() } else { distance.abs() } < 1e-9;
            let parent = e.node(*id).and_then(|n| n.parent).unwrap_or(self.active_parent);
            let tex = mapcraft_map::TexDefaults { material: face.material.clone(), scale: face.u.scale };
            let new = match mode {
                ExtrudeMode::Stamp if distance < 1e-9 => return Err(EngineError::Refused("stamps go outward")),
                _ if still => {
                    e.set_kind(*id, NodeKind::Brush(orig.clone()))?;
                    None
                }
                ExtrudeMode::Resize => {
                    e.set_kind(*id, NodeKind::Brush(orig.extrude_face(*face_id, distance, lock).ok_or(EngineError::Refused("that would leave no volume"))?))?;
                    None
                }
                ExtrudeMode::AlongNormal => {
                    e.set_kind(*id, NodeKind::Brush(orig.sweep_face(*face_id, distance, lock).ok_or(EngineError::Refused("that would leave no volume"))?))?;
                    None
                }
                ExtrudeMode::Slide => {
                    let slid = orig.slide_face(*face_id, shift, lock).ok_or(EngineError::Refused("the face would lose its shape there"))?;
                    e.set_kind(*id, NodeKind::Brush(slid))?;
                    None
                }
                ExtrudeMode::Split if distance > 0.0 => {
                    e.set_kind(*id, NodeKind::Brush(orig.clone()))?;
                    let grown = orig.extrude_face(*face_id, distance, lock).ok_or(EngineError::Refused("that would leave no volume"))?;
                    grown.split(face.points, &tex)?.0
                }
                ExtrudeMode::Split => {
                    let moved = face.points.map(|p| p + plane.normal * distance);
                    let (front, back) = orig.split(moved, &tex)?;
                    // A cut past the whole brush leaves it as it was.
                    let Some(mut kept) = back else {
                        e.set_kind(*id, NodeKind::Brush(orig.clone()))?;
                        continue;
                    };
                    kept.adopt_face(face, plane.normal * distance, lock);
                    e.set_kind(*id, NodeKind::Brush(kept))?;
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
            ExtrudeMode::Resize | ExtrudeMode::AlongNormal => "Extrude",
            ExtrudeMode::Split => "Split",
            ExtrudeMode::Stamp => "Stamp",
            ExtrudeMode::Slide => "Slide",
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
            if e.node(id).and_then(|n| n.kind.brush()) != Some(&b) {
                e.set_kind(id, NodeKind::Brush(b))?;
            }
        }
        let c = e.finish();
        self.apply_coalesced(label, c, coalesce);
        Ok(())
    }

    /// Gives the picked faces `src`'s material, alignment (wrapped across shared edges) and face attributes.
    pub fn paste_face(&mut self, src: &Face) -> Result<(), EngineError> {
        self.edit_faces("Paste face", None, |f| {
            f.wrap_from(src);
            f.attrs = src.attrs.clone();
        })
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
        let inner: BTreeSet<NodeId> = roots.iter().flat_map(|id| self.map.subtree(*id)).collect();
        for id in inner {
            let Some(node) = e.node(id) else { continue };
            let moved = match node.kind.clone() {
                NodeKind::Group(g) => Some(NodeKind::Group(moved_group(g, m))).filter(|k| *k != node.kind),
                kind @ NodeKind::Entity(_) if !node.children.is_empty() => transform_kind(kind, m, self.texture_lock, false)?,
                _ => None,
            };
            if let Some(kind) = moved {
                e.set_kind(id, kind)?;
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
                    NodeKind::Path(p) => p.points.first().map(|q| q.pos),
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
    pub fn duplicate_selection(&mut self, delta: DVec3, coalesce: Option<&str>) -> Result<(), EngineError> {
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
                    NodeKind::Entity(mut ent) => {
                        if let Some(origin) = ent.origin().or(node.children.is_empty().then_some(DVec3::ZERO)) {
                            ent.set("origin", fmt_vec(origin + delta));
                        }
                        NodeKind::Entity(ent)
                    }
                    NodeKind::Prefab(mut p) => {
                        p.origin += delta;
                        NodeKind::Prefab(p)
                    }
                    NodeKind::Group(g) => NodeKind::Group(Group { curve: None, ..moved_group(g, &DMat4::from_translation(delta)) }),
                    NodeKind::Path(p) => NodeKind::Path(p.transformed(&DMat4::from_translation(delta))),
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
        self.apply_coalesced("Duplicate", c, coalesce);
        self.selection = copies;
        Ok(())
    }

    /// Copies the selection `count` times, each copy one `step` on from the last (Hammer's Paste Special).
    pub fn array_selection(&mut self, count: usize, step: &DMat4) -> Result<(), EngineError> {
        let roots = self.require_selection()?;
        if !(1..=256).contains(&count) {
            return Err(EngineError::Refused("make between 1 and 256 copies"));
        }
        let mut e = self.map.edit();
        let mut copies = BTreeSet::new();
        let mut m = DMat4::IDENTITY;
        for _ in 0..count {
            m = *step * m;
            for root in &roots {
                let Some(parent) = e.node(*root).and_then(|n| n.parent) else { continue };
                let mut stack = vec![(*root, parent, true)];
                while let Some((src, dst, top)) = stack.pop() {
                    let Some(node) = e.node(src).cloned() else { continue };
                    let point = matches!(node.kind, NodeKind::Entity(_)) && node.children.is_empty();
                    let mut kind = transform_kind(node.kind.clone(), &m, self.texture_lock, point)?.unwrap_or(node.kind);
                    if let NodeKind::Brush(b) = &mut kind {
                        b.faces.iter_mut().for_each(|f| f.id = Default::default());
                    }
                    let id = e.insert(dst, kind)?;
                    if top {
                        copies.insert(id);
                    }
                    stack.extend(node.children.iter().rev().map(|c| (*c, id, false)));
                }
            }
        }
        let c = e.finish();
        self.apply("Array", c);
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
        let editable = self.map.editable_nodes();
        let mut e = self.map.edit();
        let mut flattened = 0;
        let targets: Vec<(NodeId, Brush)> = self
            .map
            .nodes()
            .filter(|(id, _)| !cutter_ids.contains(id) && editable.contains(id))
            .filter_map(|(id, n)| Some((id, n.kind.brush()?.clone())))
            .collect();
        for (id, brush) in targets {
            let mut parts = vec![brush.clone()];
            for cutter in &cutters {
                parts = parts.iter().map(|p| p.subtract(cutter)).collect::<Result<Vec<_>, _>>()?.into_iter().flatten().collect();
            }
            if parts != std::slice::from_ref(&brush) {
                // A brush carved away whole is gone, not flattened.
                if !parts.is_empty() {
                    flattened += displaced_faces(std::slice::from_ref(&brush)).saturating_sub(displaced_faces(&parts));
                }
                Self::replace_brush(&mut e, id, parts)?;
            }
        }
        let c = e.finish();
        self.apply("Carve", c);
        self.notice = flattened_notice(flattened);
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
        let mut flattened = 0;
        for id in brushes {
            let Some(b) = e.node(id).and_then(|n| n.kind.brush()).cloned() else { continue };
            let (front, back) = b.split(points, &self.tex)?;
            let halves: Vec<Brush> = front.iter().chain(&back).cloned().collect();
            flattened += displaced_faces(std::slice::from_ref(&b)).saturating_sub(displaced_faces(&halves));
            let parts: Vec<Brush> = match keep {
                ClipKeep::Front => front.into_iter().collect(),
                ClipKeep::Back => back.into_iter().collect(),
                ClipKeep::Both => front.into_iter().chain(back).collect(),
            };
            kept.extend(Self::replace_brush(&mut e, id, parts)?);
        }
        let c = e.finish();
        self.apply("Clip", c);
        self.notice = flattened_notice(flattened);
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

    /// Brush entities that are selected, inside the selection or hold a selected brush.
    pub fn selected_brush_entities(&self) -> Vec<NodeId> {
        let is_brush_entity = |id: &NodeId| self.map.get(*id).is_some_and(|n| matches!(n.kind, NodeKind::Entity(_)) && !n.children.is_empty());
        let mut out = BTreeSet::new();
        for id in &self.selection {
            out.extend(self.map.subtree(*id).into_iter().filter(is_brush_entity));
            out.extend(self.map.ancestors(*id).into_iter().find(is_brush_entity));
        }
        out.into_iter().collect()
    }

    /// Sets the origin of brush entities `ids` to `at`, or to the center of each one's brushes.
    pub fn set_origins(&mut self, ids: &[NodeId], at: Option<DVec3>, coalesce: Option<&str>) -> Result<(), EngineError> {
        if ids.is_empty() {
            return Err(EngineError::Refused("select a brush entity, such as a door, first"));
        }
        let mut e = self.map.edit();
        for id in ids {
            let Some(origin) = at.or_else(|| self.bounds_of([*id]).map(|b| b.center())) else { continue };
            let Some(NodeKind::Entity(mut ent)) = e.node(*id).map(|n| n.kind.clone()) else { continue };
            ent.set("origin", fmt_vec(origin));
            e.set_kind(*id, NodeKind::Entity(ent))?;
        }
        let c = e.finish();
        if !c.is_empty() {
            self.apply_coalesced("Set origin", c, coalesce);
        }
        Ok(())
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
    /// Takes the selected brushes out of their entities and, given a class, ties them to one new entity of it.
    pub fn set_brush_class(&mut self, classname: Option<&str>) -> Result<(), EngineError> {
        let brushes = self.selected_brushes();
        if brushes.is_empty() {
            return Err(EngineError::NothingSelected);
        }
        let mut e = self.map.edit();
        let mut parents = Vec::new();
        for id in &brushes {
            let Some(parent) = e.node(*id).and_then(|n| n.parent) else { continue };
            let owner = e.node(parent).filter(|p| matches!(p.kind, NodeKind::Entity(_))).map(|p| (parent, p.parent));
            match owner {
                Some((entity, Some(above))) => {
                    e.reparent(*id, above, None)?;
                    if e.node(entity).is_some_and(|n| n.children.is_empty()) {
                        e.remove(entity)?;
                    }
                    parents.push(above);
                }
                _ => parents.push(parent),
            }
        }
        let mut selection: BTreeSet<NodeId> = brushes.iter().copied().collect();
        if let (Some(class), Some(parent)) = (classname, parents.first()) {
            let ent = e.insert(*parent, NodeKind::Entity(Entity::new(class)))?;
            for id in &brushes {
                e.reparent(*id, ent, None)?;
            }
            selection = BTreeSet::from([ent]);
        }
        let c = e.finish();
        self.apply(classname.map_or("Move to world".to_string(), |c| format!("Make {c}")).as_str(), c);
        self.selection = selection;
        Ok(())
    }

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
            NodeKind::Group(mut g) if key == "name" => {
                g.name = value.to_string();
                NodeKind::Group(g)
            }
            NodeKind::Layer(mut l) if key == "name" => {
                l.name = value.to_string();
                NodeKind::Layer(l)
            }
            _ => return Err(EngineError::Argument(format!("node {} takes no key `{key}`; groups and layers only take `name`", id.0))),
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

    /// Locks or unlocks nodes; locked ones cannot be picked in the views or changed by tools.
    pub fn set_locked(&mut self, ids: &[NodeId], locked: bool) -> Result<(), EngineError> {
        let mut e = self.map.edit();
        for id in ids {
            e.update(*id, |_, l, _, _| *l = locked)?;
        }
        let c = e.finish();
        self.apply(if locked { "Lock" } else { "Unlock" }, c);
        if locked {
            let locked_set: BTreeSet<NodeId> = ids.iter().flat_map(|id| self.map.subtree(*id)).collect();
            self.selection.retain(|id| !locked_set.contains(id));
        }
        Ok(())
    }

    pub fn unlock_all(&mut self) -> Result<(), EngineError> {
        let ids: Vec<NodeId> = self.map.nodes().filter(|(_, n)| n.locked).map(|(id, _)| id).collect();
        if ids.is_empty() {
            return Ok(());
        }
        self.set_locked(&ids, false)
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
        let m = self.tex.material.clone();
        self.set_material(&m, "Apply material")
    }

    /// Takes the material off the picked faces, else off every face of the selected brushes.
    pub fn clear_material(&mut self) -> Result<(), EngineError> {
        self.set_material("", "Clear material")
    }

    fn set_material(&mut self, material: &str, label: &str) -> Result<(), EngineError> {
        if !self.face_selection.is_empty() {
            return self.edit_faces(label, None, |f| f.material = material.to_string());
        }
        let brushes = self.selected_brushes();
        if brushes.is_empty() {
            return Err(EngineError::NothingSelected);
        }
        let mut e = self.map.edit();
        for id in brushes {
            let Some(mut b) = e.node(id).and_then(|n| n.kind.brush()).cloned() else { continue };
            b.faces.iter_mut().for_each(|f| f.material = material.to_string());
            e.set_kind(id, NodeKind::Brush(b))?;
        }
        let c = e.finish();
        self.apply(label, c);
        Ok(())
    }

    /// Selected nodes not inside another selected node.
    fn selection_roots(&self) -> Vec<NodeId> {
        self.selection.iter().copied().filter(|id| !self.map.ancestors(*id).iter().any(|a| self.selection.contains(a))).collect()
    }

    /// Moves each selected object down onto the first visible solid brush below it, with `size` giving point entity boxes; returns how many found nothing.
    pub fn drop_to_floor(&mut self, size: &dyn Fn(&Entity) -> Option<Aabb>) -> Result<usize, EngineError> {
        self.require_selection()?;
        let roots = self.selection_roots();
        let moving: BTreeSet<NodeId> = roots.iter().flat_map(|r| self.map.subtree(*r)).collect();
        let visible = self.map.visible_nodes();
        let floors: Vec<&Brush> = self
            .map
            .nodes()
            .filter(|(id, _)| visible.contains(id) && !moving.contains(id))
            .filter_map(|(_, n)| n.kind.brush())
            .filter(|b| !b.faces.iter().all(|f| mapcraft_map::is_tool_material(&f.material)))
            .collect();
        let mut e = self.map.edit();
        let mut stuck = 0;
        for &root in &roots {
            let bounds = self
                .leaves_of([root])
                .into_iter()
                .filter_map(|l| match &self.map.get(l)?.kind {
                    NodeKind::Brush(b) => b.bounds().ok(),
                    NodeKind::Entity(ent) => size(ent).or_else(|| ent.origin().map(|o| Aabb::new(o, o))),
                    NodeKind::Prefab(p) => Some(Aabb::new(p.origin, p.origin)),
                    NodeKind::Path(p) => p.bounds(),
                    _ => None,
                })
                .reduce(|a, b| a.union(&b))
                .or_else(|| self.bounds_of([root]));
            let Some(b) = bounds else {
                stuck += 1;
                continue;
            };
            let (c, h) = (b.center(), b.size() * 0.49);
            // Rays start a unit above mid height so an object sunk into its floor rises onto it.
            let top = c.z + 1.0;
            let probes = [(0.0, 0.0), (1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)].map(|(x, y)| DVec3::new(c.x + h.x * x, c.y + h.y * y, top));
            let hit = probes.iter().flat_map(|p| floors.iter().filter_map(|f| f.ray_hit(*p, -DVec3::Z).map(|(t, _)| t))).min_by(f64::total_cmp);
            let Some(t) = hit else {
                stuck += 1;
                continue;
            };
            let drop = t - (top - b.min.z);
            if drop.abs() > 1e-9 {
                self.transform_nodes(&mut e, &[root], &DMat4::from_translation(-DVec3::Z * drop))?;
            }
        }
        if stuck == roots.len() {
            return Err(EngineError::Refused("nothing solid is below the selection"));
        }
        let c = e.finish();
        self.apply("Drop to floor", c);
        Ok(stuck)
    }

    /// Turns and scales each selected object about its own center within `r`, in one undo step; a seed repeats its result.
    pub fn randomize_selection(&mut self, r: &Randomize, coalesce: Option<&str>) -> Result<(), EngineError> {
        self.require_selection()?;
        let mut state = r.seed;
        let mut next = move || {
            state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            (z ^ (z >> 31)) as f64 / u64::MAX as f64 * 2.0 - 1.0
        };
        let mut e = self.map.edit();
        for root in self.selection_roots() {
            let (yaw, pitch, roll, grow) = (next() * r.yaw, next() * r.tilt, next() * r.tilt, 1.0 + next() * r.scale.clamp(0.0, 0.9));
            let point = is_point_entity(&e, root);
            if point && let Some(NodeKind::Entity(mut ent)) = e.node(root).map(|n| n.kind.clone()) {
                // Entities face by their angles key; turning about the origin would leave tilt unrecorded.
                match ent.get("angles").and_then(mapcraft_map::parse_vec3) {
                    None if ent.get("angle").is_some() => {
                        let a = ent.get("angle").and_then(|s| s.trim().parse::<f64>().ok()).unwrap_or(0.0);
                        ent.set("angle", fmt_num((a + yaw).rem_euclid(360.0)));
                    }
                    a => {
                        let a = a.unwrap_or_default();
                        ent.set("angles", fmt_vec(DVec3::new(a.x + pitch, (a.y + yaw).rem_euclid(360.0), a.z + roll)));
                    }
                }
                e.set_kind(root, NodeKind::Entity(ent))?;
                continue;
            }
            let Some(c) = self.bounds_of([root]).map(|b| b.center()) else { continue };
            let turn = DQuat::from_rotation_z(yaw.to_radians()) * DQuat::from_rotation_y(pitch.to_radians()) * DQuat::from_rotation_x(roll.to_radians());
            let m = DMat4::from_translation(c) * DMat4::from_quat(turn) * DMat4::from_scale(DVec3::splat(grow)) * DMat4::from_translation(-c);
            self.transform_nodes(&mut e, &[root], &m)?;
        }
        let c = e.finish();
        self.apply_coalesced("Randomize", c, coalesce);
        Ok(())
    }
}

/// Degrees of turn about Z and tilt about X and Y either way, and `scale` as a fraction (0.1 is 10%), for [`Session::randomize_selection`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Randomize {
    pub yaw: f64,
    pub tilt: f64,
    pub scale: f64,
    pub seed: u64,
}

impl Default for Randomize {
    fn default() -> Self {
        Self { yaw: 180.0, tilt: 0.0, scale: 0.0, seed: 1 }
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

    fn brush(s: &Session, id: NodeId) -> Brush {
        s.map.get(id).unwrap().kind.brush().unwrap().clone()
    }

    fn face_toward(s: &Session, id: NodeId, n: DVec3) -> (NodeId, Brush, FaceId) {
        let b = brush(s, id);
        let f = b.faces.iter().find(|f| f.plane().unwrap().normal.dot(n) > 0.99).unwrap().id;
        (id, b, f)
    }

    #[test]
    fn face_drags_slide_stamp_and_seams() {
        let mut s = session();
        let w = s.create_shape(Shape::Wedge, Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap()[0];
        let n = DVec3::new(1.0, 0.0, 1.0).normalize();
        let slope = vec![face_toward(&s, w, n)];
        s.extrude(&slope, Push::Offset(n * 16.0), ExtrudeMode::Slide, &[], Some("slide")).unwrap();
        assert_eq!(brush(&s, w).vertices().unwrap().len(), 6, "a slide tilts the neighbors instead of adding faces");
        assert!(s.extrude(&slope, Push::Offset(n * -200.0), ExtrudeMode::Slide, &[], Some("slide")).is_err());
        s.extrude(&slope, Push::Offset(DVec3::ZERO), ExtrudeMode::Slide, &[], Some("slide")).unwrap();
        assert_eq!(brush(&s, w), slope[0].1, "back at zero the wedge is as it was");
        assert_eq!(s.undo_label(), Some("Slide"));
        assert!(s.cancel_coalesce("slide"));

        let block = s.create_brush(Aabb::new(DVec3::new(100.0, 0.0, 0.0), DVec3::new(164.0, 64.0, 64.0))).unwrap();
        let top = vec![face_toward(&s, block, DVec3::Z)];
        assert!(s.extrude(&top, Push::Distance(-8.0), ExtrudeMode::Stamp, &[], None).is_err(), "stamps go outward");
        assert!(s.extrude(&top, Push::Distance(0.0), ExtrudeMode::Stamp, &[], None).is_err());
        let made = s.extrude(&top, Push::Distance(-100.0), ExtrudeMode::Split, &[], None).unwrap();
        assert!(made.is_empty(), "a cut past the whole brush adds nothing");
        assert_eq!(brush(&s, block), top[0].1);

        let left = s.create_brush(Aabb::new(DVec3::new(0.0, 100.0, 0.0), DVec3::new(64.0, 164.0, 64.0))).unwrap();
        let right = s.create_brush(Aabb::new(DVec3::new(64.0, 100.0, 0.0), DVec3::new(128.0, 164.0, 64.0))).unwrap();
        let far = s.create_brush(Aabb::new(DVec3::new(64.0, 300.0, 0.0), DVec3::new(128.0, 364.0, 64.0))).unwrap();
        s.selection = BTreeSet::from([left, right, far]);
        let seam = face_toward(&s, left, DVec3::X);
        let group = s.extrude_group(left, seam.2);
        assert_eq!(group.iter().map(|g| g.0).collect::<Vec<_>>(), vec![left, right], "the touching face joins, the far one does not");
        let faces: Vec<_> = group.iter().map(|(id, f, _)| (*id, brush(&s, *id), *f)).collect();
        assert!(matches!(
            s.extrude(&faces, Push::Offset(DVec3::X * 16.0), ExtrudeMode::Stamp, &[], None),
            Err(EngineError::Refused("stamps cannot cross a seam"))
        ));
        s.extrude(&faces, Push::Offset(DVec3::X * 16.0), ExtrudeMode::Resize, &[], None).unwrap();
        assert_eq!(brush(&s, left).bounds().unwrap().max.x, 80.0);
        assert_eq!(brush(&s, right).bounds().unwrap().min.x, 80.0, "the seam slides");
    }

    #[test]
    fn merge_refuses_shapes_that_are_not_convex() {
        let mut s = session();
        let a = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::new(64.0, 32.0, 32.0))).unwrap();
        let b = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::new(32.0, 64.0, 32.0))).unwrap();
        s.selection = BTreeSet::from([a, b]);
        assert!(matches!(s.merge_selection(), Err(EngineError::Refused(_))), "an L shape is not one brush");
        assert_eq!(s.map.len(), 3);
        let c = s.create_brush(Aabb::new(DVec3::new(16.0, 0.0, 0.0), DVec3::new(48.0, 32.0, 32.0))).unwrap();
        s.selection = BTreeSet::from([a, c]);
        s.merge_selection().unwrap();
        assert!((volume(&s) - (64.0 * 32.0 * 32.0 + 32.0 * 64.0 * 32.0)).abs() < 1e-6, "overlapping boxes inside one box merge");
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
    fn clear_material_takes_picked_faces_else_whole_brushes() {
        let mut s = session();
        let b = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
        let mats = |s: &Session| -> Vec<String> { s.map.get(b).unwrap().kind.brush().unwrap().faces.iter().map(|f| f.material.clone()).collect() };
        let face = s.map.get(b).unwrap().kind.brush().unwrap().faces[0].id;
        s.selection.clear();
        s.face_selection = BTreeSet::from([(b, face)]);
        s.clear_material().unwrap();
        assert_eq!(mats(&s).iter().filter(|m| m.is_empty()).count(), 1, "only the picked face is cleared");
        s.face_selection.clear();
        s.selection = BTreeSet::from([b]);
        s.clear_material().unwrap();
        assert!(mats(&s).iter().all(String::is_empty), "then every face of the selected brush");
        assert_eq!(s.undo_label(), Some("Clear material"));
        s.selection.clear();
        assert!(s.clear_material().is_err(), "nothing selected is refused");
    }

    #[test]
    fn arrays_step_each_copy_from_the_last() {
        let mut s = session();
        let a = s.create_brush(Aabb::new(DVec3::new(64.0, -8.0, 0.0), DVec3::new(80.0, 8.0, 16.0))).unwrap();
        s.selection = BTreeSet::from([a]);
        let quarter = DMat4::from_quat(DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2));
        s.array_selection(3, &quarter).unwrap();
        let centers: Vec<DVec3> = s.selection.iter().map(|id| s.map.get(*id).unwrap().kind.brush().unwrap().bounds().unwrap().center()).collect();
        for want in [DVec3::new(0.0, 72.0, 8.0), DVec3::new(-72.0, 0.0, 8.0), DVec3::new(0.0, -72.0, 8.0)] {
            assert!(centers.iter().any(|c| c.distance(want) < 1e-6), "{want} in {centers:?}");
        }
        assert!(s.array_selection(0, &quarter).is_err());
        assert!(s.undo() && s.map.nodes().filter(|(_, n)| n.kind.brush().is_some()).count() == 1, "one undo step for the array");
    }

    #[test]
    fn brush_class_switches_in_one_step() {
        let mut s = session();
        let a = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
        let b = s.create_brush(Aabb::new(DVec3::splat(64.0), DVec3::splat(128.0))).unwrap();
        s.selection = BTreeSet::from([a, b]);
        s.set_brush_class(Some("func_detail")).unwrap();
        let detail = *s.selection.first().unwrap();
        assert_eq!(s.map.get(detail).unwrap().kind.entity().unwrap().classname, "func_detail");
        s.set_brush_class(Some("func_illusionary")).unwrap();
        let ill = *s.selection.first().unwrap();
        assert!(s.map.get(detail).is_none(), "the emptied entity goes");
        assert_eq!(s.map.get(ill).unwrap().children.len(), 2);
        s.set_brush_class(None).unwrap();
        assert!(s.map.get(ill).is_none() && s.map.get(a).unwrap().parent == Some(ROOT), "back in the world");
        assert!(s.undo() && s.map.get(ill).is_some(), "each switch is one undo step");
    }

    #[test]
    fn pasted_faces_take_material_alignment_and_attributes() {
        let mut s = session();
        let b = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
        let faces = s.map.get(b).unwrap().kind.brush().unwrap().faces.clone();
        let mut src = faces[0].clone();
        src.material = "ramps/orange".into();
        src.u.offset = 7.0;
        src.attrs.insert("lightmapscale".into(), "8".into());
        s.face_selection = faces.iter().skip(1).map(|f| (b, f.id)).collect();
        let before = s.undo_label().map(str::to_string);
        s.paste_face(&src).unwrap();
        let after = s.map.get(b).unwrap().kind.brush().unwrap().faces.clone();
        assert!(after.iter().skip(1).all(|f| f.material == "ramps/orange" && f.attrs.get("lightmapscale").is_some_and(|v| v == "8")));
        assert_eq!(after[0].material, faces[0].material, "faces not picked keep theirs");
        assert!(s.undo() && s.undo_label().map(str::to_string) == before, "one undo step");
    }

    #[test]
    fn drop_to_floor_lands_each_object_on_what_is_below() {
        let mut s = session();
        s.create_brush(Aabb::new(DVec3::new(-512.0, -512.0, -64.0), DVec3::new(512.0, 512.0, 0.0))).unwrap();
        let shelf = s.create_brush(Aabb::new(DVec3::new(200.0, 200.0, 0.0), DVec3::new(300.0, 300.0, 32.0))).unwrap();
        let crate_ = s.create_brush(Aabb::new(DVec3::new(0.0, 0.0, 100.0), DVec3::new(32.0, 32.0, 132.0))).unwrap();
        let ent = s.create_point_entity("info_player_start", DVec3::new(250.0, 250.0, 300.0), vec![]).unwrap();
        let lost = s.create_point_entity("info_null", DVec3::new(5000.0, 0.0, 300.0), vec![]).unwrap();
        s.selection = BTreeSet::from([crate_, ent, lost]);
        let before = s.undo_label().map(str::to_string);
        assert_eq!(s.drop_to_floor(&|_| None).unwrap(), 1, "nothing is under the far entity");
        assert_eq!(brush(&s, crate_).bounds().unwrap().min.z, 0.0);
        assert_eq!(s.map.get(ent).unwrap().kind.entity().unwrap().origin(), Some(DVec3::new(250.0, 250.0, 32.0)), "lands on the shelf");
        assert_eq!(s.map.get(lost).unwrap().kind.entity().unwrap().origin(), Some(DVec3::new(5000.0, 0.0, 300.0)));
        assert!(s.undo() && s.undo_label().map(str::to_string) == before, "one undo step");
        s.selection = BTreeSet::from([ent]);
        let feet = |e: &Entity| e.origin().map(|o| Aabb::new(o - DVec3::new(16.0, 16.0, 24.0), o + DVec3::new(16.0, 16.0, 32.0)));
        s.drop_to_floor(&feet).unwrap();
        assert_eq!(s.map.get(ent).unwrap().kind.entity().unwrap().origin(), Some(DVec3::new(250.0, 250.0, 56.0)), "its box's bottom lands");
        assert!(s.map.get(shelf).is_some());
        s.selection = BTreeSet::from([lost]);
        assert!(matches!(s.drop_to_floor(&|_| None), Err(EngineError::Refused(_))), "nothing below anything selected");
    }

    #[test]
    fn drop_to_floor_passes_triggers_and_rises_out_of_its_floor() {
        let mut s = session();
        s.create_brush(Aabb::new(DVec3::new(-512.0, -512.0, -256.0), DVec3::new(512.0, 512.0, -128.0))).unwrap();
        s.create_brush(Aabb::new(DVec3::new(-256.0, -256.0, -16.0), DVec3::new(256.0, 256.0, 0.0))).unwrap();
        s.tex.material = "tools/trigger".into();
        s.create_brush(Aabb::new(DVec3::new(-64.0, -64.0, 0.0), DVec3::new(64.0, 64.0, 128.0))).unwrap();
        let ent = s.create_point_entity("info_player_start", DVec3::new(0.0, 0.0, 512.0), vec![]).unwrap();
        let sunk = s.create_point_entity("info_target", DVec3::new(128.0, 128.0, 0.0), vec![]).unwrap();
        s.selection = BTreeSet::from([ent, sunk]);
        let centered = |e: &Entity| e.origin().map(|o| Aabb::new(o - DVec3::splat(8.0), o + DVec3::splat(8.0)));
        assert_eq!(s.drop_to_floor(&centered).unwrap(), 0);
        let origin = |id| s.map.get(id).unwrap().kind.entity().unwrap().origin();
        assert_eq!(origin(ent), Some(DVec3::new(0.0, 0.0, 8.0)), "a zone trigger is not a floor");
        assert_eq!(origin(sunk), Some(DVec3::new(128.0, 128.0, 8.0)), "a box half in its floor rises onto it");
    }

    #[test]
    fn randomize_turns_each_object_about_itself_repeatably() {
        let mut s = session();
        let a = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::new(128.0, 32.0, 32.0))).unwrap();
        let b = s.create_brush(Aabb::new(DVec3::splat(512.0), DVec3::new(640.0, 544.0, 544.0))).unwrap();
        let ent = s.create_point_entity("prop_static", DVec3::ZERO, vec![("angles".into(), "0 90 0".into())]).unwrap();
        s.selection = BTreeSet::from([a, b, ent]);
        let centers = |s: &Session| [a, b].map(|id| brush(s, id).bounds().unwrap().center());
        let before = centers(&s);
        let shape = brush(&s, a);
        let r = Randomize { yaw: 90.0, tilt: 0.0, scale: 0.0, seed: 7 };
        s.randomize_selection(&r, Some("randomize")).unwrap();
        let once = s.map.clone();
        for (c, was) in centers(&s).iter().zip(before) {
            assert!(c.distance(was) < 1e-6, "each object turns about its own center");
        }
        assert_ne!(brush(&s, a), shape, "it turned");
        let angles = s.map.get(ent).unwrap().kind.entity().unwrap().get("angles").unwrap().to_string();
        assert_ne!(angles, "0 90 0");
        assert!(s.cancel_coalesce("randomize"));
        s.randomize_selection(&r, Some("randomize")).unwrap();
        assert_eq!(s.map, once, "the same seed gives the same result");
    }

    #[test]
    fn carve_spares_what_is_hidden_and_locks_hold() {
        let mut s = session();
        let wall = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::new(256.0, 32.0, 128.0))).unwrap();
        let layer = s.create_layer("Hidden").unwrap();
        let hidden = s.create_brush(Aabb::new(DVec3::new(0.0, 64.0, 0.0), DVec3::new(256.0, 96.0, 128.0))).unwrap();
        s.set_hidden(&[layer], true).unwrap();
        s.active_parent = ROOT;
        let cutter = s.create_brush(Aabb::new(DVec3::new(96.0, -32.0, 0.0), DVec3::new(160.0, 128.0, 64.0))).unwrap();
        s.selection = BTreeSet::from([cutter]);
        s.carve().unwrap();
        assert!(s.map.get(wall).is_none(), "the visible wall is cut");
        assert!(s.map.get(hidden).is_some(), "a brush in a hidden layer is left alone");

        s.selection = BTreeSet::from([cutter]);
        s.set_locked(&[cutter], true).unwrap();
        assert!(s.selection.is_empty() && s.map.get(cutter).unwrap().locked);
        s.unlock_all().unwrap();
        assert!(!s.map.get(cutter).unwrap().locked);
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
    fn rotating_a_point_entity_turns_its_facing() {
        let mut s = session();
        let props = |a: &str, b: &str| vec![(a.to_string(), b.to_string())];
        let spawn = s.create_point_entity("info_player_start", DVec3::new(32.0, 32.0, 0.0), props("angles", "0 30 0")).unwrap();
        let quake = s.create_point_entity("info_player_start", DVec3::new(32.0, 32.0, 0.0), props("angle", "90")).unwrap();
        let key = |s: &Session, id, k| s.map.get(id).unwrap().kind.entity().unwrap().get(k).map(str::to_string);
        s.selection = BTreeSet::from([spawn]);
        s.rotate_selection(DVec3::Z, 90.0).unwrap();
        assert_eq!(key(&s, spawn, "angles").as_deref(), Some("0 120 0"));
        assert_eq!(key(&s, spawn, "origin").as_deref(), Some("32 32 0"));
        s.selection = BTreeSet::from([quake]);
        s.rotate_selection(DVec3::Z, -135.0).unwrap();
        assert_eq!(key(&s, quake, "angle").as_deref(), Some("315"));
        s.translate_selection(DVec3::new(16.0, 0.0, 0.0), None).unwrap();
        assert_eq!(key(&s, quake, "angle").as_deref(), Some("315"), "moving leaves the facing alone");
    }

    #[test]
    fn brush_entity_origins_follow_their_brushes() {
        let mut s = session();
        let b = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::new(64.0, 8.0, 128.0))).unwrap();
        let door = s.tie_to_entity("func_door_rotating").unwrap();
        let key = |s: &Session, id| s.map.get(id).unwrap().kind.entity().unwrap().get("origin").map(str::to_string);
        assert_eq!(s.selected_brush_entities(), [door]);
        s.selection = BTreeSet::from([b]);
        assert_eq!(s.selected_brush_entities(), [door], "a selected brush counts its entity");
        s.set_origins(&[door], Some(DVec3::new(0.0, 4.0, 0.0)), None).unwrap();
        s.selection = BTreeSet::from([door]);
        s.translate_selection(DVec3::new(32.0, 0.0, 0.0), None).unwrap();
        assert_eq!(key(&s, door).as_deref(), Some("32 4 0"), "the hinge moves with the door");
        s.rotate_selection(DVec3::Z, 90.0).unwrap();
        let hinge = s.map.get(door).unwrap().kind.entity().unwrap().origin().unwrap();
        let corner = s.bounds_of([door]).unwrap();
        assert!((hinge.x - corner.max.x + 4.0).abs() < 1e-6 && (hinge.y - corner.min.y).abs() < 1e-6, "and turns with it: {hinge:?} in {corner:?}");
        s.duplicate_selection(DVec3::new(0.0, 0.0, 256.0), None).unwrap();
        let copy = *s.selection.iter().next().unwrap();
        assert_eq!(s.map.get(copy).unwrap().kind.entity().unwrap().origin(), Some(hinge + DVec3::Z * 256.0), "and copies keep it in place");

        s.selection = BTreeSet::from([door]);
        crate::execute(&mut s, "tools.center_origins").unwrap();
        assert_eq!(s.map.get(door).unwrap().kind.entity().unwrap().origin(), Some(s.bounds_of([door]).unwrap().center()));
        assert_eq!(s.undo_label(), Some("Set origin"));
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
        s.duplicate_selection(DVec3::new(0.0, 128.0, 0.0), None).unwrap();
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
        assert!(s.extrude(&faces, Push::Distance(16.0), ExtrudeMode::Resize, &[], Some("x")).unwrap().is_empty());
        s.extrude(&faces, Push::Distance(32.0), ExtrudeMode::Resize, &[], Some("x")).unwrap();
        assert_eq!(s.map.get(block).unwrap().kind.brush().unwrap().bounds().unwrap().max.z, 96.0);
        assert!(s.extrude(&faces, Push::Distance(-64.0), ExtrudeMode::Resize, &[], Some("x")).is_err());
        s.end_coalesce();
        s.undo();
        let made = s.extrude(&faces, Push::Distance(32.0), ExtrudeMode::Split, &[], Some("y")).unwrap();
        let made = s.extrude(&faces, Push::Distance(48.0), ExtrudeMode::Split, &made, Some("y")).unwrap();
        assert_eq!(made.len(), 1);
        assert_eq!(s.map.get(made[0]).unwrap().kind.brush().unwrap().bounds().unwrap().min.z, 64.0);
        assert_eq!(s.map.get(block).unwrap().kind.brush().unwrap().bounds().unwrap().max.z, 64.0);
        let inward = s.extrude(&faces, Push::Distance(-16.0), ExtrudeMode::Split, &made, Some("y")).unwrap();
        assert_eq!(s.map.get(block).unwrap().kind.brush().unwrap().bounds().unwrap().max.z, 48.0);
        assert_eq!(s.map.get(inward[0]).unwrap().kind.brush().unwrap().bounds().unwrap().min.z, 48.0);
        let dragged = b.faces.iter().find(|f| f.id == top).unwrap();
        let cut = s.map.get(block).unwrap().kind.brush().unwrap().faces.iter().find(|f| f.id == top).unwrap().clone();
        assert_eq!(Face { points: dragged.points, ..cut }, *dragged, "the cut face takes the dragged face's id and texture");
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
