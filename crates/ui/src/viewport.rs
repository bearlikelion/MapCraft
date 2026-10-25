use std::collections::BTreeSet;

use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};
use mapcraft_engine::pick_target;
use mapcraft_geom::{Aabb, DMat4, DVec2, DVec3, snap};
use mapcraft_map::{FaceId, NodeId, NodeKind};
use mapcraft_render::{LineVertex, SELECTED_EDGE, box_lines};

use crate::app::MapCraftApp;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    #[default]
    Select,
    /// Select, with move arrows in the 3D view.
    Move,
    /// Drag brush faces along their normal.
    Extrude,
    Rotate,
    /// Stretch the selection with handles.
    Scale,
    Block,
    Entity,
    Clip,
    /// Pick and texture individual faces.
    Face,
    /// Raise, lower and smooth displacements.
    Sculpt,
    /// Drag brush corners.
    Vertex,
}

#[derive(Clone, Debug, Default)]
pub enum Drag {
    #[default]
    None,
    Marquee {
        view: usize,
        start: Pos2,
    },
    /// `anchor` is the selection's corner nearest the grab point; it is what snaps.
    Move {
        view: usize,
        start: DVec3,
        anchor: DVec3,
        applied: DVec3,
    },
    Resize {
        view: usize,
        handle: (i8, i8),
        bounds: Aabb,
        applied: Aabb,
    },
    Block {
        view: usize,
        start: DVec3,
        end: DVec3,
    },
    /// Drawing the clip line; its ends are `clip_points`.
    Clip {
        view: usize,
    },
    /// A sculpt stroke; flatten levels toward `level`, where it began.
    Sculpt {
        view: usize,
        level: DVec3,
        last: std::time::Instant,
    },
    Vertex {
        view: usize,
        originals: Vec<DVec3>,
        start: DVec3,
        anchor: DVec3,
        applied: DVec3,
    },
    /// Dragging faces of `faces` (the brushes as they were) along `normal`; `created` are the
    /// brushes a split or stamp added so far.
    Extrude {
        view: usize,
        faces: Vec<(NodeId, mapcraft_map::Brush, FaceId)>,
        normal: DVec3,
        center: DVec3,
        start: f64,
        mode: mapcraft_engine::ExtrudeMode,
        created: Vec<NodeId>,
        distance: f64,
    },
    /// Turning the selection about `axis` through `center`; `turned` is the unwrapped angle in
    /// radians and `applied` the degrees already applied.
    Rotate {
        view: usize,
        axis: DVec3,
        center: DVec3,
        last: f64,
        turned: f64,
        applied: f64,
    },
    /// Stretching the selection's bounds along one axis in the 3D view.
    Scale {
        view: usize,
        axis: usize,
        bounds: Aabb,
        start: f64,
        applied: Aabb,
    },
    /// Drawing a block on a surface in the 3D view: the base lies on the plane where `axis` is
    /// `offset`, and the block rises `height` toward `sign` along that axis.
    Block3d {
        view: usize,
        axis: usize,
        sign: f64,
        offset: f64,
        start: DVec3,
        end: DVec3,
        height: f64,
    },
    /// Moving the selection along one gizmo arrow in the 3D view.
    Axis {
        view: usize,
        axis: DVec3,
        center: DVec3,
        start: f64,
        /// The selection's low edge along the axis, which snaps to the grid.
        base: f64,
        applied: f64,
    },
}

/// What a vertex tool handle drags.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HandleKind {
    Vertex,
    Edge,
    Face,
}

pub(crate) const AXES: [(DVec3, Color32); 3] =
    [(DVec3::X, Color32::from_rgb(230, 60, 60)), (DVec3::Y, Color32::from_rgb(70, 200, 70)), (DVec3::Z, Color32::from_rgb(70, 120, 240))];

pub(crate) fn segment_distance(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = if ab.length_sq() > 0.0 { ((p - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0) } else { 0.0 };
    p.distance(a + ab * t)
}

/// Parameter along the line `center + axis * t` closest to the ray.
pub(crate) fn closest_on_axis(center: DVec3, axis: DVec3, origin: DVec3, dir: DVec3) -> Option<f64> {
    let w = center - origin;
    let (a, b, c, d, e) = (axis.dot(axis), axis.dot(dir), dir.dot(dir), axis.dot(w), dir.dot(w));
    let denom = a * c - b * b;
    (denom.abs() > 1e-9).then(|| (b * e - c * d) / denom)
}

const PREVIEW: [f32; 4] = [1.0, 1.0, 0.2, 1.0];

/// How close on screen, in points, a corner must be to catch a dragged point.
const SNAP_REACH: f64 = 8.0;

pub(crate) fn dvec(v: Vec2) -> DVec2 {
    DVec2::new(f64::from(v.x), f64::from(v.y))
}

impl MapCraftApp {
    fn point_box(&self, e: &mapcraft_map::Entity) -> Aabb {
        let o = e.origin().unwrap_or_default();
        let (min, max) = mapcraft_render::EntityStyle::size(&self.project, &e.classname).unwrap_or((DVec3::splat(-8.0), DVec3::splat(8.0)));
        Aabb::new(o + min, o + max)
    }

    /// Bounds of a brush or point entity.
    pub(crate) fn leaf_bounds(&self, id: NodeId) -> Option<Aabb> {
        let n = self.view_map().get(id)?;
        match &n.kind {
            NodeKind::Brush(b) => b.bounds().ok(),
            NodeKind::Entity(e) if n.children.is_empty() => Some(self.point_box(e)),
            _ => None,
        }
    }

    pub(crate) fn node_bounds(&self, id: NodeId) -> Option<Aabb> {
        self.view_map().subtree(id).into_iter().filter_map(|n| self.leaf_bounds(n)).reduce(|a, b| a.union(&b))
    }

    pub(crate) fn selection_bounds(&self) -> Option<Aabb> {
        self.session.selection.iter().filter_map(|id| self.node_bounds(*id)).reduce(|a, b| a.union(&b))
    }

    /// Visible brushes and point entities on the ray, nearest first, as the nodes of the real map
    /// they belong to.
    pub(crate) fn pick_all(&self, origin: DVec3, dir: DVec3) -> Vec<(NodeId, f64)> {
        let visible = self.view_map().visible_nodes();
        let mut hits: Vec<(NodeId, f64)> = Vec::new();
        for (id, n) in self.view_map().nodes() {
            if !visible.contains(&id) || n.locked {
                continue;
            }
            let t = match &n.kind {
                NodeKind::Brush(b) => b.ray_hit(origin, dir).map(|(t, _)| t),
                NodeKind::Entity(e) if n.children.is_empty() => self.point_box(e).ray_hit(origin, dir),
                _ => None,
            };
            if let Some(t) = t {
                hits.push((self.owner(id), t));
            }
        }
        hits.sort_by(|a, b| a.1.total_cmp(&b.1));
        let mut seen = BTreeSet::new();
        hits.retain(|(id, _)| seen.insert(*id));
        hits
    }

    /// Nearest visible brush or point entity on the ray, with the hit distance.
    pub(crate) fn pick(&self, origin: DVec3, dir: DVec3) -> Option<(NodeId, f64)> {
        self.pick_all(origin, dir).into_iter().next()
    }

    /// Nearest visible brush face on the ray.
    pub(crate) fn pick_face(&self, origin: DVec3, dir: DVec3) -> Option<(NodeId, FaceId, f64)> {
        let visible = self.session.map.visible_nodes();
        let mut best: Option<(NodeId, FaceId, f64)> = None;
        for (id, n) in self.session.map.nodes() {
            let (true, false, Some(b)) = (visible.contains(&id), n.locked, n.kind.brush()) else { continue };
            let Some((t, i)) = b.ray_hit(origin, dir) else { continue };
            if best.is_none_or(|(_, _, bt)| t < bt)
                && let Some(f) = b.faces.get(i)
            {
                best = Some((id, f.id, t));
            }
        }
        best
    }

    /// Vertex tool handles of the selected brushes: corners, edge midpoints and face centers,
    /// each with the corners it drags.
    fn vertex_handles(&self) -> Vec<(HandleKind, DVec3, Vec<DVec3>)> {
        let mut out: Vec<(HandleKind, DVec3, Vec<DVec3>)> = Vec::new();
        let mut add = |kind, at: DVec3, verts: Vec<DVec3>| {
            if !out.iter().any(|(k, p, _)| *k == kind && p.distance_squared(at) < 1e-6) {
                out.push((kind, at, verts));
            }
        };
        for id in self.session.selected_brushes() {
            let Some(Ok(solid)) = self.session.map.get(id).and_then(|n| n.kind.brush()).map(|b| b.solid()) else { continue };
            for w in solid.faces.iter().flatten() {
                for (i, a) in w.0.iter().enumerate() {
                    let b = w.0[(i + 1) % w.0.len()];
                    add(HandleKind::Vertex, *a, vec![*a]);
                    add(HandleKind::Edge, (*a + b) / 2.0, vec![*a, b]);
                }
                add(HandleKind::Face, w.center(), w.0.clone());
            }
        }
        out
    }

    /// The displaced surface under the pointer in the 3D view, and its face's normal.
    fn sculpt_hit(&self, index: usize, rect: Rect, pos: Pos2) -> Option<(DVec3, DVec3)> {
        let cam = self.panes.get(index)?.camera;
        if cam.is_2d() {
            return None;
        }
        let (origin, dir) = cam.ray(dvec(pos - rect.min), dvec(rect.size()));
        let (id, face, t) = self.pick_face(origin, dir)?;
        let b = self.session.map.get(id)?.kind.brush()?;
        let normal = b.faces.iter().find(|f| f.id == face && f.disp.is_some())?.plane()?.normal;
        Some((origin + dir * t, normal))
    }

    /// Continues a sculpt stroke under the pointer.
    fn sculpt_step(&mut self, index: usize, rect: Rect, pos: Pos2, invert: bool) {
        let Drag::Sculpt { view, level, last } = self.drag.clone() else { return };
        let Some((p, n)) = self.sculpt_hit(index, rect, pos) else { return };
        self.sculpt_at = Some((index, p, n));
        self.drag = Drag::Sculpt { view, level, last: std::time::Instant::now() };
        self.dab(p, level, last.elapsed().as_secs_f64().min(0.1), invert);
    }

    /// One dab of the sculpt brush at `center` worth `dt` seconds; `invert` swaps raise and lower.
    fn dab(&mut self, center: DVec3, level: DVec3, dt: f64, invert: bool) {
        use mapcraft_engine::SculptMode as M;
        let (mode, radius, strength) = self.sculpt;
        let mode = match (mode, invert) {
            (M::Raise, true) => M::Lower,
            (M::Lower, true) => M::Raise,
            (m, _) => m,
        };
        let amount = match mode {
            M::Smooth | M::Flatten => (strength / 64.0 * dt).min(1.0),
            _ => strength * dt,
        };
        match self.session.sculpt(&mapcraft_engine::Sculpt { mode, center, radius, amount, level }, Some("sculpt")) {
            Ok(true) => self.mark_changed(),
            Ok(false) => {}
            Err(e) => self.status = e.to_string(),
        }
    }

    /// A clip point under the pointer and the direction the clip plane runs through it: the view
    /// axis in a 2D view, the normal of the face under the pointer in the 3D view.
    fn clip_point(&mut self, index: usize, rect: Rect, pos: Pos2) -> Option<(DVec3, DVec3)> {
        let cam = self.panes.get(index)?.camera;
        if cam.is_2d() {
            return Some((self.snapped_plane_point(index, pos, rect)?, cam.view.axes().2));
        }
        let (origin, dir) = cam.ray(dvec(pos - rect.min), dvec(rect.size()));
        let (id, face, t) = self.pick_face(origin, dir)?;
        let plane = self.session.map.get(id)?.kind.brush()?.faces.iter().find(|f| f.id == face)?.plane()?;
        let p = self.grid_on_plane(plane, origin + dir * t);
        self.snap_mark = Some((index, p, false));
        Some((p, plane.normal))
    }

    /// The far end of a clip line being dragged: on the view plane in 2D, on the first point's face in 3D.
    fn clip_drag_point(&mut self, index: usize, rect: Rect, pos: Pos2) -> Option<DVec3> {
        let cam = self.panes.get(index)?.camera;
        if cam.is_2d() {
            return self.snapped_plane_point(index, pos, rect);
        }
        let (a, normal) = (*self.clip_points.first()?, self.clip_view_axis?);
        let (origin, dir) = cam.ray(dvec(pos - rect.min), dvec(rect.size()));
        let den = dir.dot(normal);
        let t = (a - origin).dot(normal) / den;
        if den.abs() < 1e-9 || t <= 0.0 {
            return None;
        }
        let p = self.grid_on_plane(mapcraft_geom::Plane { normal, dist: normal.dot(a) }, origin + dir * t);
        self.snap_mark = Some((index, p, false));
        Some(p)
    }

    /// `p` with its in-plane coordinates on the grid, kept on `plane`.
    fn grid_on_plane(&self, plane: mapcraft_geom::Plane, p: DVec3) -> DVec3 {
        let axis = plane.normal.abs().max_position();
        let mut q = self.snap_in_plane(axis, p);
        let rest: f64 = (0..3).filter(|k| *k != axis).map(|k| plane.normal[k] * q[k]).sum();
        q[axis] = (plane.dist - rest) / plane.normal[axis];
        q
    }

    pub(crate) fn apply_clip(&mut self) {
        let (Some(a), Some(b)) = (self.clip_points.first().copied(), self.clip_points.get(1).copied()) else { return };
        let view = self.panes.iter().find(|p| p.camera.is_2d()).map_or(DVec3::Z, |p| p.camera.view.axes().2);
        let toward = self.clip_view_axis.unwrap_or(view);
        if let Err(e) = self.session.clip([a, b, b + toward * 64.0], self.clip_keep) {
            self.status = e.to_string();
        }
        self.clip_points.clear();
        self.mark_changed();
    }

    fn overlay_lines(&self) -> Vec<LineVertex> {
        let mut out = Vec::new();
        if let Drag::Block { start, end, .. } = &self.drag {
            let b = self.block_box(*start, *end);
            box_lines(&mut out, b.min, b.max, PREVIEW);
        }
        if let Some(b) = self.block_3d_box() {
            box_lines(&mut out, b.min, b.max, PREVIEW);
        }
        if self.tool == Tool::Clip
            && let (Some(a), Some(b), Some(t)) = (self.clip_points.first(), self.clip_points.get(1), self.clip_view_axis)
        {
            let dir = (*b - *a).normalize_or_zero();
            let (a, b) = (*a - dir * 100_000.0, *b + dir * 100_000.0);
            for d in [-1.0, 1.0] {
                out.push(LineVertex::new(a + t * d * 100_000.0, PREVIEW));
                out.push(LineVertex::new(b + t * d * 100_000.0, PREVIEW));
            }
            out.push(LineVertex::new(a, PREVIEW));
            out.push(LineVertex::new(b, PREVIEW));
        }
        if self.tool == Tool::Clip
            && let (&[a, b], Some(t)) = (self.clip_points.as_slice(), self.clip_view_axis)
            && a.distance(b) > 1e-6
        {
            use mapcraft_engine::ClipKeep as K;
            for id in self.session.selected_brushes().into_iter().take(64) {
                let Some(brush) = self.session.map.get(id).and_then(|n| n.kind.brush()) else { continue };
                let Ok((front, back)) = brush.split([a, b, b + t * 64.0], &self.session.tex) else { continue };
                for (part, kept) in [(front, self.clip_keep != K::Back), (back, self.clip_keep != K::Front)] {
                    let Some(solid) = part.and_then(|p| p.solid().ok()) else { continue };
                    let color = if kept { PREVIEW } else { [0.45, 0.05, 0.05, 1.0] };
                    for w in solid.faces.iter().flatten() {
                        for (i, p) in w.0.iter().enumerate() {
                            out.push(LineVertex::new(*p, color));
                            out.push(LineVertex::new(w.0[(i + 1) % w.0.len()], color));
                        }
                    }
                }
            }
        }
        if self.tool == Tool::Sculpt
            && let Some((_, p, n)) = self.sculpt_at
        {
            let (u, v) = (n.any_orthonormal_vector(), n.cross(n.any_orthonormal_vector()));
            let ring = |k: usize| {
                let a = k as f64 / 48.0 * std::f64::consts::TAU;
                p + n + (u * a.cos() + v * a.sin()) * self.sculpt.1
            };
            out.extend((0..48).flat_map(|k| [LineVertex::new(ring(k), PREVIEW), LineVertex::new(ring(k + 1), PREVIEW)]));
        }
        if self.tool == Tool::Block
            && let Some((_, at)) = self.ghost_at
        {
            out.extend(self.ghost_lines(at).into_iter().flat_map(|[a, b]| [LineVertex::new(a, PREVIEW), LineVertex::new(b, PREVIEW)]));
        }
        self.link_lines(&mut out);
        for c in self.session.map.cordons.iter().filter(|c| c.active) {
            box_lines(&mut out, c.bounds.min, c.bounds.max, [0.3, 0.6, 1.0, 1.0]);
        }
        if let Some(h) = &self.extrude_hover {
            for poly in &h.polygons {
                for (i, p) in poly.iter().enumerate() {
                    out.push(LineVertex::new(*p, PREVIEW));
                    out.push(LineVertex::new(poly[(i + 1) % poly.len()], PREVIEW));
                }
            }
        }
        if let Some(b) = self.selection_bounds().filter(|_| matches!(self.tool, Tool::Select | Tool::Move | Tool::Rotate | Tool::Scale)) {
            box_lines(&mut out, b.min, b.max, [SELECTED_EDGE[0], SELECTED_EDGE[1], SELECTED_EDGE[2], 0.35]);
        }
        out
    }

    /// Lines from selected entities to the entities their keys and outputs name.
    fn link_lines(&self, out: &mut Vec<LineVertex>) {
        let map = &self.session.map;
        let center = |id: NodeId| self.node_bounds(id).map(|b| b.center());
        for sel in &self.session.selection {
            let Some(e) = map.get(*sel).and_then(|n| n.kind.entity()) else { continue };
            let class = self.project.game.class(&e.classname);
            let is_link = |k: &str| match &class {
                Some(c) => c.property(k).is_some_and(|p| p.ty == "target_destination"),
                None => ["target", "parentname", "respawn", "landmark"].contains(&k),
            };
            let names: Vec<&str> = e
                .properties
                .iter()
                .filter(|(k, _)| is_link(k))
                .map(|(_, v)| v.as_str())
                .chain(e.connections.iter().map(|c| c.target.as_str()))
                .filter(|n| !n.is_empty())
                .collect();
            if names.is_empty() {
                continue;
            }
            let Some(from) = center(*sel) else { continue };
            for (id, n) in map.nodes() {
                let Some(t) = n.kind.entity().and_then(|t| t.get("targetname")) else { continue };
                if names.contains(&t)
                    && let Some(to) = center(id)
                {
                    out.push(LineVertex::new(from, [1.0, 0.85, 0.2, 1.0]));
                    out.push(LineVertex::new(to, [1.0, 0.85, 0.2, 1.0]));
                }
            }
        }
    }

    /// The box a block drag makes: the dragged rectangle, with the depth of the last box made.
    fn block_box(&self, start: DVec3, end: DVec3) -> Aabb {
        let mut b = Aabb::new(start, end);
        for axis in 0..3 {
            if (b.max[axis] - b.min[axis]).abs() < 1e-9 {
                b.min[axis] = self.last_box.min[axis];
                b.max[axis] = self.last_box.max[axis];
            }
        }
        b
    }

    /// Brush corners and point entity origins that drags snap to, with the node each belongs to.
    fn snap_candidates(&mut self) -> &[(NodeId, DVec3)] {
        if self.snap_points.is_none() {
            let map = self.view_map();
            let visible = map.visible_nodes();
            let mut pts = Vec::new();
            for (id, n) in map.nodes().filter(|(id, _)| visible.contains(id)) {
                match &n.kind {
                    NodeKind::Brush(b) => pts.extend(b.vertices().unwrap_or_default().into_iter().map(|v| (self.owner(id), v))),
                    NodeKind::Entity(e) if n.children.is_empty() => pts.extend(e.origin().map(|o| (self.owner(id), o))),
                    _ => {}
                }
            }
            self.snap_points = Some(pts);
        }
        self.snap_points.as_deref().unwrap_or_default()
    }

    /// The selected nodes and everything inside them, which a move drags along.
    fn moving_nodes(&self) -> BTreeSet<NodeId> {
        self.session.selection.iter().flat_map(|id| self.session.map.subtree(*id)).collect()
    }

    /// Where a point moved in a 2D view lands: on a corner or entity origin within reach of it on
    /// screen, else on the grid, keeping the view's depth. Points of `moving` nodes and points at
    /// `skip` cannot catch it. The flag says whether a corner caught it.
    pub(crate) fn snap_point(&mut self, index: usize, rect: Rect, p: DVec3, moving: &BTreeSet<NodeId>, skip: &[DVec3]) -> (DVec3, bool) {
        let Some(cam) = self.panes.get(index).map(|pane| pane.camera) else { return (p, false) };
        let (r, u, d) = cam.view.axes();
        let size = dvec(rect.size());
        let view = self.prefs.get().view.clone();
        let g = self.view_grid(index);
        let on_grid = |x: f64| (x / g).round() * g;
        let grid_point = view.snap_grid.then(|| r * on_grid(p.dot(r)) + u * on_grid(p.dot(u)) + d * p.dot(d));
        if view.snap_vertices
            && let Some(at) = cam.project(p, size)
        {
            // A corner wins only over a farther grid point, so aiming at a grid line lands on it.
            let reach = grid_point.and_then(|q| cam.project(q, size)).map_or(SNAP_REACH, |q| (q - at).length().min(SNAP_REACH));
            let best = self
                .snap_candidates()
                .iter()
                .filter(|(id, v)| !moving.contains(id) && !skip.iter().any(|s| s.distance_squared(*v) < 1e-6))
                .filter_map(|(_, v)| Some(((cam.project(*v, size)? - at).length(), *v)))
                .filter(|(dist, _)| *dist < reach)
                .min_by(|a, b| a.0.total_cmp(&b.0));
            if let Some((_, v)) = best {
                return (r * v.dot(r) + u * v.dot(u) + d * p.dot(d), true);
            }
        }
        (grid_point.unwrap_or(p), false)
    }

    /// The grid a view draws and snaps to: coarser than the grid size in a zoomed out 2D view.
    pub(crate) fn view_grid(&self, index: usize) -> f64 {
        match self.panes.get(index).map(|p| p.camera) {
            Some(cam) if cam.is_2d() => mapcraft_render::grid_step(self.session.grid, cam.zoom),
            _ => self.session.grid,
        }
    }

    /// The snapped point under the cursor, remembered so the view can mark it.
    fn snapped_plane_point(&mut self, index: usize, pos: Pos2, rect: Rect) -> Option<DVec3> {
        let cam = self.panes.get(index)?.camera;
        let p = cam.plane_point(dvec(pos - rect.min), dvec(rect.size()));
        let (snapped, caught) = self.snap_point(index, rect, p, &BTreeSet::new(), &[]);
        self.snap_mark = Some((index, snapped, caught));
        Some(snapped)
    }

    fn draw_snap_mark(&self, ui: &egui::Ui, index: usize, rect: Rect) {
        let (Some((view, p, caught)), Some(cam)) = (self.snap_mark, self.panes.get(index).map(|pane| pane.camera)) else { return };
        if view != index {
            return;
        }
        let Some(q) = cam.project(p, dvec(rect.size())) else { return };
        let c = rect.min + Vec2::new(q.x as f32, q.y as f32);
        let painter = ui.painter_at(rect);
        if caught {
            painter.circle_stroke(c, 6.0, Stroke::new(2.0, Color32::from_rgb(255, 220, 60)));
            painter.circle_filled(c, 2.0, Color32::from_rgb(255, 220, 60));
        } else {
            let s = Stroke::new(1.5, crate::chrome::ACCENT);
            painter.line_segment([c - Vec2::new(5.0, 0.0), c + Vec2::new(5.0, 0.0)], s);
            painter.line_segment([c - Vec2::new(0.0, 5.0), c + Vec2::new(0.0, 5.0)], s);
        }
    }

    pub(crate) fn viewport(&mut self, ui: &mut egui::Ui, index: usize, rect: Rect) {
        if rect.width() < 2.0 || rect.height() < 2.0 {
            return;
        }
        let ppp = ui.ctx().pixels_per_point();
        let px = [(rect.width() * ppp).round() as u32, (rect.height() * ppp).round() as u32];
        let overlay = self.overlay_lines();
        let grid = self.view_grid(index);
        let mode = self.render_mode;
        if let (Some(gpu), Some(pane)) = (&mut self.gpu, self.panes.get_mut(index)) {
            pane.rect = rect;
            if pane.target.as_ref().is_none_or(|t| t.size != px) {
                let target = gpu.renderer.target(px);
                let mut egui_renderer = gpu.state.renderer.write();
                pane.texture = Some(match pane.texture {
                    Some(id) => {
                        egui_renderer.update_egui_texture_from_wgpu_texture(&gpu.state.device, &target.display, wgpu_filter(), id);
                        id
                    }
                    None => egui_renderer.register_native_texture(&gpu.state.device, &target.display, wgpu_filter()),
                });
                pane.target = Some(target);
            }
            if let Some(t) = &pane.target {
                // The target is in physical pixels, the camera's zoom in points.
                let camera = mapcraft_render::Camera { zoom: pane.camera.zoom * f64::from(ppp), ..pane.camera };
                gpu.renderer.render(t, &camera, mode, Some(grid), &overlay);
            }
            if let Some(tex) = pane.texture {
                ui.painter().image(tex, rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            }
        } else {
            ui.painter().rect_filled(rect, 0.0, Color32::BLACK);
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "No GPU renderer", egui::FontId::proportional(14.0), Color32::GRAY);
        }
        let response = ui.interact(rect, ui.id().with(("viewport", index)), Sense::click_and_drag());
        self.viewport_input(ui, index, rect, &response);
        if let Drag::Marquee { view, start } = &self.drag
            && *view == index
            && let Some(p) = response.interact_pointer_pos()
        {
            ui.painter_at(rect).rect_stroke(Rect::from_two_pos(*start, p), 0.0, Stroke::new(1.0, Color32::YELLOW), egui::StrokeKind::Inside);
        }
        if matches!(self.tool, Tool::Select | Tool::Move | Tool::Scale) {
            self.draw_handles(ui, index, rect);
        }
        if self.tool == Tool::Vertex
            && let Some(cam) = self.panes.get(index).map(|p| p.camera).filter(|c| c.is_2d())
        {
            for (kind, at, _) in self.vertex_handles().into_iter().rev() {
                let Some(p) = cam.project(at, dvec(rect.size())) else { continue };
                let c = rect.min + Vec2::new(p.x as f32, p.y as f32);
                if !rect.contains(c) {
                    continue;
                }
                match kind {
                    HandleKind::Vertex => ui.painter_at(rect).rect_filled(Rect::from_center_size(c, Vec2::splat(6.0)), 0.0, Color32::from_rgb(255, 220, 60)),
                    HandleKind::Edge => ui.painter_at(rect).rect_filled(Rect::from_center_size(c, Vec2::splat(5.0)), 0.0, Color32::from_rgb(80, 220, 240)),
                    HandleKind::Face => ui.painter_at(rect).circle_filled(c, 3.0, Color32::from_rgb(120, 240, 120)),
                };
            }
        }
        for (_, a, b, color) in self.gizmo(index, rect) {
            ui.painter_at(rect).line_segment([a, b], Stroke::new(3.0, color));
            ui.painter_at(rect).circle_filled(b, 5.0, color);
        }
        self.draw_transform_gizmos(ui, index, rect);
        self.draw_snap_mark(ui, index, rect);
    }

    /// The move arrows drawn at the selection's center in the 3D view: axis, screen start and end.
    fn gizmo(&self, index: usize, rect: Rect) -> Vec<(DVec3, Pos2, Pos2, Color32)> {
        let (Some(cam), Some(b)) = (self.panes.get(index).map(|p| p.camera), self.selection_bounds()) else { return Vec::new() };
        if cam.is_2d() || self.tool != Tool::Move {
            return Vec::new();
        }
        let size = dvec(rect.size());
        let c = b.center();
        let len = cam.position.distance(c) * 0.12;
        let to_screen = |p: DVec3| cam.project(p, size).map(|q| rect.min + Vec2::new(q.x as f32, q.y as f32));
        let Some(start) = to_screen(c) else { return Vec::new() };
        AXES.iter().filter_map(|(axis, color)| Some((*axis, start, to_screen(c + *axis * len)?, *color))).collect()
    }

    fn handle_rects(&self, index: usize, rect: Rect) -> Vec<((i8, i8), Rect)> {
        let (Some(pane), Some(b)) = (self.panes.get(index), self.selection_bounds()) else { return Vec::new() };
        if !pane.camera.is_2d() {
            return Vec::new();
        }
        let size = dvec(rect.size());
        let (Some(a), Some(c)) = (pane.camera.project(b.min, size), pane.camera.project(b.max, size)) else { return Vec::new() };
        let (lo, hi) = (a.min(c), a.max(c));
        let mut out = Vec::new();
        for hx in [-1i8, 0, 1] {
            for hy in [-1i8, 0, 1] {
                if hx == 0 && hy == 0 {
                    continue;
                }
                let x = match hx {
                    -1 => lo.x - 6.0,
                    1 => hi.x + 6.0,
                    _ => (lo.x + hi.x) / 2.0,
                };
                // Screen y grows down while the view's up axis grows up.
                let y = match hy {
                    1 => lo.y - 6.0,
                    -1 => hi.y + 6.0,
                    _ => (lo.y + hi.y) / 2.0,
                };
                out.push(((hx, hy), Rect::from_center_size(rect.min + Vec2::new(x as f32, y as f32), Vec2::splat(7.0))));
            }
        }
        out
    }

    fn draw_handles(&self, ui: &egui::Ui, index: usize, rect: Rect) {
        for (_, r) in self.handle_rects(index, rect) {
            ui.painter_at(rect).rect_filled(r, 0.0, Color32::WHITE);
            ui.painter_at(rect).rect_stroke(r, 0.0, Stroke::new(1.0, Color32::BLACK), egui::StrokeKind::Inside);
        }
    }

    fn viewport_input(&mut self, ui: &egui::Ui, index: usize, rect: Rect, response: &egui::Response) {
        let Some(pane) = self.panes.get(index) else { return };
        let cam = pane.camera;
        let size = dvec(rect.size());
        let ctx = ui.ctx();
        let hover = response.hover_pos();
        if let Some(h) = hover {
            self.cursor_world = cam.is_2d().then(|| cam.plane_point(dvec(h - rect.min), size));
        }
        if matches!(self.drag, Drag::None) {
            let shift = ctx.input(|i| i.modifiers.shift);
            match hover {
                Some(h) if self.extrude_wanted(shift) => self.extrude_hover = self.extrude_hit(index, rect, h),
                _ if self.extrude_hover.as_ref().is_some_and(|x| x.view == index) => self.extrude_hover = None,
                _ => {}
            }
        }
        // Tools that place things show where a click would land.
        if matches!(self.drag, Drag::None) {
            if self.tool == Tool::Sculpt {
                match hover {
                    Some(h) => self.sculpt_at = self.sculpt_hit(index, rect, h).map(|(p, n)| (index, p, n)),
                    None if self.sculpt_at.is_some_and(|(v, ..)| v == index) => self.sculpt_at = None,
                    None => {}
                }
            }
            // The ghost stays where the pointer left it, so inspector edits show on it.
            if self.tool == Tool::Block
                && self.plugin_shape.is_some()
                && let Some(h) = hover
            {
                self.ghost_at = self.placement_at(index, rect, h).map(|p| (index, p)).or(self.ghost_at);
            }
            match hover {
                Some(h) if cam.is_2d() && matches!(self.tool, Tool::Block | Tool::Clip | Tool::Entity) => {
                    self.snapped_plane_point(index, h, rect);
                }
                Some(h) if self.tool == Tool::Block => {
                    self.snap_mark = self.block_base(index, rect, h).map(|(_, _, p)| (index, p, false));
                }
                _ if self.snap_mark.is_some_and(|(v, ..)| v == index) => self.snap_mark = None,
                _ => {}
            }
        }

        if cam.is_2d() {
            if let Some(h) = hover {
                let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
                if scroll != 0.0
                    && let Some(p) = self.panes.get_mut(index)
                {
                    p.camera.zoom_at(dvec(h - rect.min), size, (f64::from(scroll) / 200.0).exp());
                    let zoom = p.camera.zoom;
                    if self.prefs.get().view.link_zoom {
                        for other in self.panes.iter_mut().filter(|o| o.camera.is_2d()) {
                            other.camera.zoom = zoom;
                        }
                    }
                }
            }
            if response.dragged_by(egui::PointerButton::Middle) || response.dragged_by(egui::PointerButton::Secondary) {
                let d = response.drag_delta();
                if let Some(p) = self.panes.get_mut(index) {
                    p.camera.center -= DVec2::new(f64::from(d.x), -f64::from(d.y)) / p.camera.zoom;
                }
            }
            if hover.is_some() && !ctx.egui_wants_keyboard_input() {
                self.nudge(ui, index);
            }
        } else {
            self.fly(ui, index, response);
        }

        let primary_pos = response.interact_pointer_pos();
        // Drags start where the button went down, not where the drag threshold was crossed.
        if response.drag_started_by(egui::PointerButton::Primary)
            && let Some(p) = ctx.input(|i| i.pointer.press_origin()).or(primary_pos)
        {
            let mods = ctx.input(|i| i.modifiers);
            self.begin_drag(index, rect, p, mods);
        }
        if response.dragged_by(egui::PointerButton::Primary)
            && let Some(p) = primary_pos
        {
            let mods = ctx.input(|i| i.modifiers);
            if matches!(self.drag, Drag::Sculpt { view, .. } if view == index) {
                self.sculpt_step(index, rect, p, mods.command);
                // Holding still keeps sculpting.
                ctx.request_repaint();
            } else {
                self.update_drag(index, rect, p, mods.shift);
            }
        }
        if response.drag_stopped()
            && let Some(p) = primary_pos.or(hover)
        {
            self.end_drag(index, rect, p);
        }
        let alt = ctx.input(|i| i.modifiers.alt);
        if alt && !cam.is_2d() && (response.clicked_by(egui::PointerButton::Primary) || response.clicked_by(egui::PointerButton::Secondary)) {
            if let Some(p) = primary_pos.or(hover) {
                let (origin, dir) = cam.ray(dvec(p - rect.min), size);
                if let Some((id, face, _)) = self.pick_face(origin, dir) {
                    if response.clicked_by(egui::PointerButton::Primary) {
                        let material = self.session.map.get(id).and_then(|n| n.kind.brush()).and_then(|b| b.faces.iter().find(|f| f.id == face)).cloned();
                        if let Some(f) = material {
                            self.status = format!("Picked {}", f.material);
                            self.session.tex.material = f.material.clone();
                            self.picked_face = Some(f);
                        }
                    } else {
                        let result = match (&self.picked_face, ctx.input(|i| i.modifiers.shift)) {
                            (Some(src), true) => self.session.wrap_face(id, face, &src.clone()),
                            _ => self.session.apply_material_to_face(id, face),
                        };
                        if let Err(e) = result {
                            self.status = e.to_string();
                        }
                    }
                    self.mark_changed();
                }
            }
        } else if response.clicked_by(egui::PointerButton::Primary)
            && let Some(p) = primary_pos
        {
            self.click(index, rect, p, ctx.input(|i| i.modifiers.command || i.modifiers.shift));
        }
        if response.double_clicked() && cam.is_2d() {
            self.frame_selection();
        }
    }

    pub(crate) fn frame_selection(&mut self) {
        if let Some(b) = self.selection_bounds() {
            self.frame_bounds(b);
        }
    }

    /// Where a click places something: on the view plane at the last brush's depth in 2D, on the
    /// ground or the face under the pointer in 3D.
    fn placement_at(&mut self, index: usize, rect: Rect, pos: Pos2) -> Option<DVec3> {
        let cam = self.panes.get(index)?.camera;
        if !cam.is_2d() {
            return self.block_base(index, rect, pos).map(|(_, _, p)| p);
        }
        let p = self.snapped_plane_point(index, pos, rect)?;
        let depth = cam.view.axes().2.abs();
        Some(p + depth * self.last_box.min.dot(depth))
    }

    fn click(&mut self, index: usize, rect: Rect, pos: Pos2, additive: bool) {
        let Some(cam) = self.panes.get(index).map(|p| p.camera) else { return };
        if self.tool == Tool::Block && self.plugin_shape.is_some() {
            if let Some(p) = self.placement_at(index, rect, pos) {
                self.place_plugin_shape(p);
            }
            return;
        }
        let size = dvec(rect.size());
        let (origin, dir) = cam.ray(dvec(pos - rect.min), size);
        match self.tool {
            Tool::Select | Tool::Move | Tool::Extrude | Tool::Rotate | Tool::Scale | Tool::Block | Tool::Vertex => {
                // Clicking the same spot again steps to the next object behind (drill select).
                let mut targets = BTreeSet::new();
                let hits: Vec<NodeId> = self
                    .pick_all(origin, dir)
                    .into_iter()
                    .map(|(id, _)| id)
                    .filter(|id| targets.insert(pick_target(&self.session.map, *id, self.session.select_mode)))
                    .collect();
                let again = self.last_click.is_some_and(|(v, p)| v == index && p.distance(pos) < 4.0);
                let current = hits.iter().position(|h| {
                    let t = pick_target(&self.session.map, *h, self.session.select_mode);
                    self.session.selection.len() == 1 && self.session.selection.contains(&t)
                });
                let i = match current {
                    Some(c) if again && !additive && !hits.is_empty() => (c + 1) % hits.len(),
                    _ => 0,
                };
                self.last_click = Some((index, pos));
                self.session.click(hits.get(i).copied(), additive);
                self.mark_changed();
            }
            Tool::Sculpt => {
                if let Some((p, _)) = self.sculpt_hit(index, rect, pos) {
                    self.session.end_coalesce();
                    self.dab(p, p, 0.05, additive);
                    self.session.end_coalesce();
                }
            }
            Tool::Face => {
                let hit = self.pick_face(origin, dir).map(|(id, f, _)| (id, f));
                if !additive {
                    self.session.face_selection.clear();
                }
                if let Some(h) = hit
                    && !self.session.face_selection.remove(&h)
                {
                    self.session.face_selection.insert(h);
                }
                self.mark_changed();
            }
            Tool::Entity => {
                let point = if cam.is_2d() {
                    let p = self.snapped_plane_point(index, pos, rect).unwrap_or_default();
                    let depth = cam.view.axes().2.abs();
                    p + depth * self.last_box.min.dot(depth)
                } else {
                    match self.pick(origin, dir) {
                        Some((_, t)) => snap(origin + dir * t, self.session.grid),
                        None => return,
                    }
                };
                let class = self.entity_class.clone();
                let defaults = self
                    .project
                    .game
                    .class(&class)
                    .map(|c| c.properties.iter().filter(|p| p.name != "origin").filter_map(|p| Some((p.name.clone(), p.default.clone()?))).collect())
                    .unwrap_or_default();
                if let Err(e) = self.session.create_point_entity(&class, point, defaults) {
                    self.status = e.to_string();
                }
                self.mark_changed();
            }
            Tool::Clip => {
                if self.clip_points.len() >= 2 {
                    self.clip_points.clear();
                }
                if let Some((p, t)) = self.clip_point(index, rect, pos) {
                    if self.clip_points.is_empty() {
                        self.clip_view_axis = Some(t);
                    }
                    self.clip_points.push(p);
                }
            }
        }
    }

    fn begin_drag(&mut self, index: usize, rect: Rect, pos: Pos2, mods: egui::Modifiers) {
        let Some(cam) = self.panes.get(index).map(|p| p.camera) else { return };
        let size = dvec(rect.size());
        if self.extrude_wanted(mods.shift) && self.begin_extrude(index, rect, pos, mods) {
            return;
        }
        if self.tool == Tool::Rotate {
            self.begin_rotate(index, rect, pos);
            return;
        }
        if self.tool == Tool::Block && self.plugin_shape.is_some() {
            return;
        }
        if self.tool == Tool::Clip {
            if let Some((p, t)) = self.clip_point(index, rect, pos) {
                self.clip_points = vec![p, p];
                self.clip_view_axis = Some(t);
                self.drag = Drag::Clip { view: index };
            }
            return;
        }
        if !cam.is_2d() {
            if self.tool == Tool::Scale && self.begin_scale(index, rect, pos) {
                return;
            }
            if self.tool == Tool::Block {
                self.begin_block_3d(index, rect, pos);
                return;
            }
            if self.tool == Tool::Sculpt {
                if let Some((p, _)) = self.sculpt_hit(index, rect, pos) {
                    self.session.end_coalesce();
                    self.drag = Drag::Sculpt { view: index, level: p, last: std::time::Instant::now() };
                    self.dab(p, p, 1.0 / 60.0, mods.command);
                }
                return;
            }
            let arrow = self.gizmo(index, rect).into_iter().find(|(_, a, b, _)| segment_distance(pos, *a, *b) < 6.0);
            if let (Some((axis, ..)), Some(b)) = (arrow, self.selection_bounds()) {
                let (origin, dir) = cam.ray(dvec(pos - rect.min), size);
                if let Some(start) = closest_on_axis(b.center(), axis, origin, dir) {
                    self.drag = Drag::Axis { view: index, axis, center: b.center(), start, base: b.min.dot(axis), applied: 0.0 };
                }
            }
            return;
        }
        match self.tool {
            Tool::Select | Tool::Move | Tool::Scale => {
                if let Some((handle, _)) = self.handle_rects(index, rect).into_iter().find(|(_, r)| r.expand(3.0).contains(pos))
                    && let Some(b) = self.selection_bounds()
                {
                    self.drag = Drag::Resize { view: index, handle, bounds: b, applied: b };
                    return;
                }
                let (origin, dir) = cam.ray(dvec(pos - rect.min), size);
                let hit = self.pick(origin, dir).map(|(id, _)| pick_target(&self.session.map, id, self.session.select_mode));
                match hit {
                    Some(target) => {
                        if !self.session.selection.contains(&target) {
                            self.session.selection = BTreeSet::from([target]);
                            self.mark_changed();
                        }
                        let start = cam.plane_point(dvec(pos - rect.min), size);
                        let moving = self.moving_nodes();
                        let press = dvec(pos - rect.min);
                        let nearest = self
                            .snap_candidates()
                            .iter()
                            .filter(|(id, _)| moving.contains(id))
                            .filter_map(|(_, v)| Some(((cam.project(*v, size)? - press).length(), *v)))
                            .min_by(|a, b| a.0.total_cmp(&b.0))
                            .map(|(_, v)| v);
                        let anchor = nearest.or(self.selection_bounds().map(|b| b.min)).unwrap_or(start);
                        // Only the in-view part of the anchor moves with the cursor.
                        let (_, _, d) = cam.view.axes();
                        let anchor = anchor - d * (anchor - start).dot(d);
                        self.drag = Drag::Move { view: index, start, anchor, applied: DVec3::ZERO };
                    }
                    None => self.drag = Drag::Marquee { view: index, start: pos },
                }
            }
            Tool::Block => {
                if let Some(p) = self.snapped_plane_point(index, pos, rect) {
                    self.drag = Drag::Block { view: index, start: p, end: p };
                }
            }
            Tool::Vertex => {
                // Everything under the cursor moves together, so in a 2D view a corner takes the
                // corners stacked behind it and an edge midpoint can take a whole side.
                let handles = self.vertex_handles();
                let under = |kind| -> Vec<(DVec3, &Vec<DVec3>)> {
                    handles
                        .iter()
                        .filter(|(k, at, _)| *k == kind && cam.project(*at, size).is_some_and(|p| (p - dvec(pos - rect.min)).length() < 6.0))
                        .map(|(_, at, v)| (*at, v))
                        .collect()
                };
                let mut hit = under(HandleKind::Vertex);
                if hit.is_empty() {
                    hit = under(HandleKind::Edge);
                }
                if hit.is_empty() {
                    hit = under(HandleKind::Face);
                }
                let mut near: Vec<DVec3> = hit.iter().flat_map(|(_, v)| v.iter().copied()).collect();
                near.dedup_by(|a, b| a.distance_squared(*b) < 1e-6);
                if let Some((anchor, _)) = hit.first() {
                    let start = cam.plane_point(dvec(pos - rect.min), size);
                    let (_, _, d) = cam.view.axes();
                    let anchor = *anchor - d * (*anchor - start).dot(d);
                    self.drag = Drag::Vertex { view: index, originals: near, start, anchor, applied: DVec3::ZERO };
                }
            }
            Tool::Entity | Tool::Clip | Tool::Face | Tool::Extrude | Tool::Rotate | Tool::Sculpt => {}
        }
    }

    fn update_drag(&mut self, index: usize, rect: Rect, pos: Pos2, shift: bool) {
        let Some(cam) = self.panes.get(index).map(|p| p.camera) else { return };
        let size = dvec(rect.size());
        let g = self.view_grid(index);
        let (r, u, _) = cam.view.axes();
        let cursor = cam.plane_point(dvec(pos - rect.min), size);
        match self.drag.clone() {
            Drag::Extrude { view, .. } if view == index => self.update_extrude(index, rect, pos),
            Drag::Block3d { view, .. } if view == index => self.update_block_3d(index, rect, pos, shift),
            Drag::Clip { view } if view == index => {
                if let Some(p) = self.clip_drag_point(index, rect, pos)
                    && let Some(end) = self.clip_points.get_mut(1)
                {
                    *end = p;
                }
            }
            Drag::Rotate { view, .. } if view == index => self.update_rotate(index, rect, pos),
            Drag::Scale { view, .. } if view == index => self.update_scale(index, rect, pos),
            Drag::Move { view, start, anchor, applied } if view == index => {
                let moving = self.moving_nodes();
                let (snapped, caught) = self.snap_point(index, rect, anchor + (cursor - start), &moving, &[]);
                self.snap_mark = Some((index, snapped, caught));
                let total = snapped - anchor;
                let step = total - applied;
                if step.length_squared() > 1e-12 {
                    if let Err(e) = self.session.translate_selection(step, Some("drag-move")) {
                        self.status = e.to_string();
                    }
                    self.drag = Drag::Move { view, start, anchor, applied: total };
                    self.mark_changed();
                }
            }
            Drag::Resize { view, handle, bounds, applied } if view == index => {
                let moving = self.moving_nodes();
                let (p, caught) = self.snap_point(index, rect, cursor, &moving, &[]);
                self.snap_mark = Some((index, p, caught));
                let mut target = bounds;
                for (axis, side) in [(r, handle.0), (u, handle.1)] {
                    let i = axis.abs().max_position();
                    let v = p.dot(axis.abs());
                    match side {
                        1 => target.max[i] = v.max(target.min[i] + g),
                        -1 => target.min[i] = v.min(target.max[i] - g),
                        _ => {}
                    }
                }
                if target != applied {
                    let m = box_transform(&applied, &target);
                    if let Err(e) = self.session.transform_selection(&m, "Resize", Some("drag-resize")) {
                        self.status = e.to_string();
                    }
                    self.drag = Drag::Resize { view, handle, bounds, applied: target };
                    self.mark_changed();
                }
            }
            Drag::Vertex { view, originals, start, anchor, applied } if view == index => {
                let dragged: Vec<DVec3> = originals.iter().map(|o| *o + applied).collect();
                let (snapped, caught) = self.snap_point(index, rect, anchor + (cursor - start), &BTreeSet::new(), &dragged);
                self.snap_mark = Some((index, snapped, caught));
                let total = snapped - anchor;
                if (total - applied).length_squared() > 1e-12 {
                    let moves: Vec<(DVec3, DVec3)> = originals.iter().map(|o| (*o + applied, *o + total)).collect();
                    match self.session.move_vertices(&moves, Some("drag-vertex")) {
                        Ok(()) => self.drag = Drag::Vertex { view, originals, start, anchor, applied: total },
                        Err(e) => self.status = format!("Cannot move there: {e}"),
                    }
                    self.mark_changed();
                }
            }
            Drag::Axis { view, axis, center, start, base, applied } if view == index => {
                let (origin, dir) = cam.ray(dvec(pos - rect.min), size);
                let Some(t) = closest_on_axis(center, axis, origin, dir) else { return };
                let total = if self.prefs.get().view.snap_grid { ((base + t - start) / g).round() * g - base } else { t - start };
                if total != applied {
                    if let Err(e) = self.session.translate_selection(axis * (total - applied), Some("drag-move")) {
                        self.status = e.to_string();
                    }
                    self.drag = Drag::Axis { view, axis, center, start, base, applied: total };
                    self.mark_changed();
                }
            }
            Drag::Block { view, start, .. } if view == index => {
                if let Some(end) = self.snapped_plane_point(index, pos, rect) {
                    self.drag = Drag::Block { view, start, end };
                }
            }
            _ => {}
        }
    }

    fn end_drag(&mut self, index: usize, rect: Rect, pos: Pos2) {
        let size = dvec(rect.size());
        self.snap_mark = None;
        let dragged = self.drag.clone();
        match std::mem::take(&mut self.drag) {
            Drag::Marquee { view, start } if view == index => {
                let Some(cam) = self.panes.get(index).map(|p| p.camera) else { return };
                let (a, b) = (cam.plane_point(dvec(start - rect.min), size), cam.plane_point(dvec(pos - rect.min), size));
                let (r, u, _) = cam.view.axes();
                let (h0, h1) = (a.dot(r).min(b.dot(r)), a.dot(r).max(b.dot(r)));
                let (v0, v1) = (a.dot(u).min(b.dot(u)), a.dot(u).max(b.dot(u)));
                let visible = self.view_map().visible_nodes();
                let mut picked = BTreeSet::new();
                for (id, n) in self.view_map().nodes() {
                    if !visible.contains(&id) || n.locked {
                        continue;
                    }
                    let Some(bb) = self.leaf_bounds(id) else { continue };
                    let (bh0, bh1) = (bb.min.dot(r).min(bb.max.dot(r)), bb.min.dot(r).max(bb.max.dot(r)));
                    let (bv0, bv1) = (bb.min.dot(u).min(bb.max.dot(u)), bb.min.dot(u).max(bb.max.dot(u)));
                    if bh0 >= h0 && bh1 <= h1 && bv0 >= v0 && bv1 <= v1 {
                        picked.insert(pick_target(&self.session.map, self.owner(id), self.session.select_mode));
                    }
                }
                self.session.selection = picked;
                self.mark_changed();
            }
            Drag::Move { .. } | Drag::Resize { .. } | Drag::Vertex { .. } | Drag::Axis { .. } | Drag::Rotate { .. } | Drag::Scale { .. } => {
                self.session.end_coalesce()
            }
            Drag::Extrude { created, .. } => {
                self.session.end_coalesce();
                // A split or stamp selects what it made, ready for the next drag.
                if !created.is_empty() {
                    self.session.selection = created.into_iter().collect();
                    self.mark_changed();
                }
            }
            Drag::Block3d { .. } => {
                if let Some(b) = self.block_3d_box_of(&dragged) {
                    match self.session.create_shape(self.shape, b) {
                        Ok(_) => self.last_box = b,
                        Err(e) => self.status = e.to_string(),
                    }
                    self.mark_changed();
                }
            }
            Drag::Sculpt { .. } => self.session.end_coalesce(),
            Drag::Clip { .. } => {
                if let [a, b] = self.clip_points[..]
                    && a.distance(b) < 1e-6
                {
                    self.clip_points.truncate(1);
                }
            }
            Drag::Block { start, end, .. } => {
                let b = self.block_box(start, end);
                if b.size().min_element() > 0.0 {
                    match self.session.create_shape(self.shape, b) {
                        Ok(_) => self.last_box = b,
                        Err(e) => self.status = e.to_string(),
                    }
                    self.mark_changed();
                }
            }
            _ => {}
        }
    }

    fn nudge(&mut self, ui: &egui::Ui, index: usize) {
        let Some(cam) = self.panes.get(index).map(|p| p.camera) else { return };
        let (r, u, _) = cam.view.axes();
        let g = self.session.grid;
        let mut d = DVec3::ZERO;
        ui.input(|i| {
            if i.key_pressed(egui::Key::ArrowLeft) {
                d -= r * g;
            }
            if i.key_pressed(egui::Key::ArrowRight) {
                d += r * g;
            }
            if i.key_pressed(egui::Key::ArrowUp) {
                d += u * g;
            }
            if i.key_pressed(egui::Key::ArrowDown) {
                d -= u * g;
            }
        });
        if d != DVec3::ZERO && !self.session.selection.is_empty() {
            if let Err(e) = self.session.translate_selection(d, Some("nudge")) {
                self.status = e.to_string();
            }
            self.mark_changed();
        }
    }

    /// Hold the right mouse button to look around; WASD moves, Q/E go down/up, Shift is faster.
    fn fly(&mut self, ui: &egui::Ui, index: usize, response: &egui::Response) {
        let speed = self.prefs.get().view.fly_speed;
        let Some(pane) = self.panes.get_mut(index) else { return };
        let cam = &mut pane.camera;
        let looking = response.dragged_by(egui::PointerButton::Secondary);
        if looking {
            let d = response.drag_delta();
            cam.yaw -= f64::from(d.x) * 0.25;
            cam.pitch = (cam.pitch - f64::from(d.y) * 0.25).clamp(-89.0, 89.0);
            let dt = f64::from(ui.input(|i| i.stable_dt)).min(0.1);
            let fast = if ui.input(|i| i.modifiers.shift) { 3.0 } else { 1.0 };
            let (f, r) = (cam.forward(), cam.right());
            let mut v = DVec3::ZERO;
            ui.input(|i| {
                for (key, dir) in
                    [(egui::Key::W, f), (egui::Key::S, -f), (egui::Key::D, r), (egui::Key::A, -r), (egui::Key::E, DVec3::Z), (egui::Key::Q, -DVec3::Z)]
                {
                    if i.key_down(key) {
                        v += dir;
                    }
                }
            });
            cam.position += v * speed * fast * dt;
            ui.ctx().request_repaint();
        }
        if response.dragged_by(egui::PointerButton::Middle) {
            let d = response.drag_delta();
            let (r, up) = (cam.right(), cam.right().cross(cam.forward()));
            cam.position += (-r * f64::from(d.x) + up * f64::from(d.y)) * 2.0;
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                cam.position += cam.forward() * f64::from(scroll) * 2.0;
            }
        }
    }
}

/// Maps box `from` onto box `to`.
pub(crate) fn box_transform(from: &Aabb, to: &Aabb) -> DMat4 {
    let s = from.size();
    let scale = DVec3::new(
        if s.x.abs() > 1e-9 { to.size().x / s.x } else { 1.0 },
        if s.y.abs() > 1e-9 { to.size().y / s.y } else { 1.0 },
        if s.z.abs() > 1e-9 { to.size().z / s.z } else { 1.0 },
    );
    DMat4::from_translation(to.min) * DMat4::from_scale(scale) * DMat4::from_translation(-from.min)
}

fn wgpu_filter() -> eframe::wgpu::FilterMode {
    eframe::wgpu::FilterMode::Linear
}
