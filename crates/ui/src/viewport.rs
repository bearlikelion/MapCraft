use std::collections::{BTreeMap, BTreeSet};

use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};
use mapcraft_engine::pick_target;
use mapcraft_geom::{Aabb, DMat4, DQuat, DVec2, DVec3, snap};
use mapcraft_map::{FaceId, Group, Mirror, NodeId, NodeKind};
use mapcraft_render::{LineVertex, SELECTED_EDGE, box_lines};

use crate::app::MapCraftApp;
use crate::vertex::HandleKind;

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
    /// Click to add path points, drag to pull their handles.
    Path,
}

#[derive(Clone, Debug, Default)]
pub enum Drag {
    #[default]
    None,
    Marquee {
        view: usize,
        start: Pos2,
    },
    UvMarquee {
        view: crate::uv::UvView,
        start: Pos2,
    },
    /// `anchor` is the selection's corner nearest the grab point; it is what snaps. With `clone` the next step makes a copy to move.
    Move {
        view: usize,
        start: DVec3,
        anchor: DVec3,
        applied: DVec3,
        clone: bool,
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
    /// Moving one of `clip_points`.
    ClipPoint {
        view: usize,
        point: usize,
    },
    /// A sculpt stroke; flatten levels toward `level`, where it began, paint spaces dabs from `from`, unset off every displacement, keeping Ctrl's `invert` from the press.
    Sculpt {
        view: usize,
        level: DVec3,
        last: std::time::Instant,
        from: Option<DVec3>,
        refused: bool,
        invert: bool,
    },
    /// Dragging the Vertex tool's selection; `originals` are each brush's corners as they began.
    Vertex {
        view: usize,
        originals: Vec<(NodeId, Vec<DVec3>)>,
        start: DVec3,
        anchor: DVec3,
        applied: DVec3,
    },
    /// Dragging the Vertex tool's selection in the 3D view across the grabbed handle's level
    /// plane, or up and down with Shift.
    Vertex3d {
        view: usize,
        originals: Vec<(NodeId, Vec<DVec3>)>,
        anchor: DVec3,
        applied: DVec3,
    },
    /// A box selecting Vertex tool handles; Shift adds to the selection and Ctrl takes away.
    ComponentBox {
        view: usize,
        start: Pos2,
        add: bool,
        remove: bool,
    },
    /// Dragging faces of `faces` (the brushes as they were) from the pointer's `start`; `paths` snap their corners, `created` is what a split or stamp added.
    Extrude {
        view: usize,
        faces: Vec<(NodeId, mapcraft_map::Brush, FaceId)>,
        normal: DVec3,
        anchor: DVec3,
        reference: Option<DVec3>,
        start: DVec3,
        paths: Vec<(DVec3, DVec3)>,
        mode: mapcraft_engine::ExtrudeMode,
        created: Vec<NodeId>,
        offset: DVec3,
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
    /// Stretching the selection along one axis in the 3D view, from its low side when `low` is set.
    Scale {
        view: usize,
        axis: usize,
        low: bool,
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
    /// Dragging a path point or handle.
    Path {
        view: usize,
        grab: Box<crate::curves::PathDrag>,
    },
    /// Dragging a brush entity's origin handle in a 2D view.
    Origin {
        view: usize,
        entity: NodeId,
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
        clone: bool,
    },
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
const LEAK: [f32; 4] = [1.0, 0.1, 0.1, 1.0];

/// How close on screen, in points, a corner must be to catch a dragged point.
const SNAP_REACH: f64 = 8.0;

pub(crate) fn dvec(v: Vec2) -> DVec2 {
    DVec2::new(f64::from(v.x), f64::from(v.y))
}

/// Nearest visible, unlocked brush face of `map` on the ray: its brush, face index and distance.
fn face_on_ray(map: &mapcraft_map::Map, origin: DVec3, dir: DVec3) -> Option<(NodeId, usize, f64)> {
    let visible = map.visible_nodes();
    map.nodes()
        .filter(|(id, n)| visible.contains(id) && !n.locked)
        .filter_map(|(id, n)| n.kind.brush()?.ray_hit(origin, dir).map(|(t, i)| (id, i, t)))
        .min_by(|a, b| a.2.total_cmp(&b.2))
}

impl MapCraftApp {
    fn point_box(&self, e: &mapcraft_map::Entity) -> Aabb {
        let (min, max) = mapcraft_render::entity_bounds(&self.project, e);
        Aabb::new(min, max)
    }

    /// Bounds of a brush or point entity.
    pub(crate) fn leaf_bounds(&self, id: NodeId) -> Option<Aabb> {
        let n = self.view_map().get(id)?;
        match &n.kind {
            NodeKind::Brush(b) => b.bounds().ok(),
            NodeKind::Entity(e) if n.children.is_empty() => Some(self.point_box(e)),
            NodeKind::Path(p) => p.bounds(),
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
    /// they belong to; locked ones count only as a `surface` to place on or aim at.
    pub(crate) fn pick_all(&self, origin: DVec3, dir: DVec3, surface: bool) -> Vec<(NodeId, f64)> {
        let map = self.view_map();
        let reach = if surface { map.visible_nodes() } else { map.editable_nodes() };
        let mut hits: Vec<(NodeId, f64)> = Vec::new();
        for (id, n) in map.nodes() {
            if !reach.contains(&id) {
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
    pub(crate) fn pick(&self, origin: DVec3, dir: DVec3, surface: bool) -> Option<(NodeId, f64)> {
        self.pick_all(origin, dir, surface).into_iter().next()
    }

    /// Nearest visible brush face on the ray; locked ones count only as a `surface`.
    pub(crate) fn pick_face(&self, origin: DVec3, dir: DVec3, surface: bool) -> Option<(NodeId, FaceId, f64)> {
        let map = &self.session.map;
        let reach = if surface { map.visible_nodes() } else { map.editable_nodes() };
        let mut best: Option<(NodeId, FaceId, f64)> = None;
        for (id, n) in map.nodes() {
            let (true, Some(b)) = (reach.contains(&id), n.kind.brush()) else { continue };
            let Some((t, i)) = b.ray_hit(origin, dir) else { continue };
            if best.is_none_or(|(_, _, bt)| t < bt)
                && let Some(f) = b.faces.get(i)
            {
                best = Some((id, f.id, t));
            }
        }
        best
    }

    /// The displaced surface under the pointer in the 3D view, and its face's normal.
    fn sculpt_hit(&self, index: usize, rect: Rect, pos: Pos2) -> Option<(DVec3, DVec3)> {
        self.sculpt_face(index, rect, pos).map(|(p, n, ..)| (p, n))
    }

    /// The displaced surface under the pointer, its face's normal, and the face.
    fn sculpt_face(&self, index: usize, rect: Rect, pos: Pos2) -> Option<(DVec3, DVec3, NodeId, FaceId)> {
        let cam = self.panes.get(index)?.camera;
        let (origin, dir) = cam.ray(dvec(pos - rect.min), dvec(rect.size()));
        let (id, face, t) = self.pick_face(origin, dir, false)?;
        let b = self.session.map.get(id)?.kind.brush()?;
        let normal = b.faces.iter().find(|f| f.id == face && f.disp.is_some())?.plane()?.normal;
        Some((origin + dir * t, normal, id, face))
    }

    /// Moves the sculpt ring to the pointer and, while painting, reads the blend there.
    fn hover_sculpt(&mut self, index: usize, rect: Rect, pos: Pos2) -> Option<(DVec3, DVec3)> {
        let hit = self.sculpt_face(index, rect, pos);
        self.sculpt_at = hit.map(|(p, n, ..)| (index, p, n));
        let painting = matches!(self.sculpt.0, mapcraft_engine::SculptMode::Paint | mapcraft_engine::SculptMode::Blur);
        self.blend_readout = hit.filter(|_| painting).and_then(|(p, _, id, face)| {
            let (f, w) = self.blend_under(id, face, p)?;
            let short = |m: &str| m.rsplit('/').next().unwrap_or(m).to_string();
            Some(format!("{} + {}: {:.0}%", short(&f.material), f.blend_layer().map_or_else(|| "weights".into(), short), w * 100.0))
        });
        hit.map(|(p, n, ..)| (p, n))
    }

    /// A displaced face and its second layer's share at the grid point nearest `p`.
    fn blend_under(&self, id: NodeId, face: FaceId, p: DVec3) -> Option<(&mapcraft_map::Face, f64)> {
        let b = self.session.map.get(id)?.kind.brush()?;
        let solid = b.solid().ok()?;
        let (i, grid) = b.displaced(&solid).into_iter().find(|(i, _)| b.faces.get(*i).is_some_and(|f| f.id == face))?;
        let f = b.faces.get(i)?;
        Some((f, f.disp.as_ref()?.blend_at(mapcraft_map::Displacement::nearest(&grid, p)?)))
    }

    /// The layer that shows most at `p` on a blended displacement: its second material from half up, else its own.
    fn dominant_layer(&self, id: NodeId, face: FaceId, p: DVec3) -> Option<String> {
        let (f, w) = self.blend_under(id, face, p)?;
        let b = f.blend_layer()?;
        Some(if w >= 0.5 { b } else { &f.material }.to_string())
    }

    /// Why a sculpt click found nothing to change.
    fn nothing_to_sculpt(&self) -> String {
        let any = self.session.map.nodes().any(|(_, n)| n.kind.brush().is_some_and(|b| b.has_displacement()));
        let face = self.chord("tool.face");
        if any {
            "Point at a displacement to sculpt it".into()
        } else {
            format!("Nothing to sculpt yet: pick a floor's top face with the Face tool ({face}) and click Make under Displacement in the inspector")
        }
    }

    /// Continues a sculpt stroke under the pointer.
    fn sculpt_step(&mut self, index: usize, rect: Rect, pos: Pos2, ctrl: bool) {
        let Drag::Sculpt { view, level, last, from, refused, invert } = self.drag.clone() else { return };
        let Some((p, _)) = self.hover_sculpt(index, rect, pos) else {
            self.drag = Drag::Sculpt { view, level, last, from: None, refused, invert };
            return;
        };
        let paint = self.sculpt.0 == mapcraft_engine::SculptMode::Paint;
        let centers = match from {
            Some(from) if paint => dabs_along(from, p, (self.paint.spacing * self.sculpt.1).max(1.0)),
            _ => vec![p],
        };
        let from = centers.last().copied().or(from);
        let mut refused = refused;
        if !centers.is_empty()
            && let Err(e) = self.dab(&centers, level, last.elapsed().as_secs_f64().min(0.1), if paint { invert } else { ctrl })
            && !refused
        {
            self.refuse(e);
            refused = true;
        }
        self.drag = Drag::Sculpt { view, level, last: std::time::Instant::now(), from, refused, invert };
    }

    /// Shows why a stroke changed nothing, again even when the last stroke said the same.
    fn refuse(&mut self, why: String) {
        self.status = why;
        self.status_shown.0.clear();
    }

    /// Dabs of the sculpt brush at `centers` worth `dt` seconds as one step; `invert` swaps raise and lower, or paints back.
    fn dab(&mut self, centers: &[DVec3], level: DVec3, dt: f64, invert: bool) -> Result<(), String> {
        use mapcraft_engine::SculptMode as M;
        let (mode, radius, strength) = self.sculpt;
        let mode = match (mode, invert) {
            (M::Raise, true) => M::Lower,
            (M::Lower, true) => M::Raise,
            (m, _) => m,
        };
        let amount = match mode {
            M::Paint => self.paint.flow,
            M::Blur => (self.paint.flow * 8.0 * dt).min(1.0),
            M::Smooth | M::Flatten => (strength / 64.0 * dt).min(1.0),
            _ => strength * dt,
        };
        let Some(&center) = centers.first() else { return Ok(()) };
        let dab = mapcraft_engine::Sculpt { mode, center, radius, amount, level, falloff: self.sculpt_falloff, opacity: self.paint.opacity, invert };
        let result = match mode {
            M::Paint | M::Blur => self.session.paint_blend(&dab, centers, Some("sculpt")),
            _ => self.session.sculpt(&dab, Some("sculpt")),
        };
        match result {
            Ok(true) => self.mark_changed(),
            Ok(false) => {}
            Err(e) => return Err(e.to_string()),
        }
        Ok(())
    }

    /// `p` from a 2D view moved along the view's depth to the middle of the selection, so points
    /// placed in two views make a tilted cut through it.
    fn clip_depth(&self, index: usize, mut p: DVec3) -> DVec3 {
        if let (Some(cam), Some(b)) = (self.panes.get(index).map(|pane| pane.camera), self.selection_bounds()) {
            let k = cam.view.axes().2.abs().max_position();
            p[k] = b.center()[k];
        }
        p
    }

    /// A clip point under the pointer and the direction the clip plane runs through it: the view
    /// axis in a 2D view, the normal of the face under the pointer in the 3D view.
    fn clip_point(&mut self, index: usize, rect: Rect, pos: Pos2) -> Option<(DVec3, DVec3)> {
        let cam = self.panes.get(index)?.camera;
        if cam.is_2d() {
            let p = self.snapped_plane_point(index, pos, rect)?;
            return Some((self.clip_depth(index, p), cam.view.axes().2));
        }
        let (origin, dir) = cam.ray(dvec(pos - rect.min), dvec(rect.size()));
        let (id, face, t) = self.pick_face(origin, dir, true)?;
        let plane = self.session.map.get(id)?.kind.brush()?.faces.iter().find(|f| f.id == face)?.plane()?;
        let p = self.grid_on_plane(plane, origin + dir * t);
        self.snap_mark = Some((index, p, false));
        Some((p, plane.normal))
    }

    /// The far end of a clip line being dragged: on the view plane in 2D, on the first point's face in 3D.
    fn clip_drag_point(&mut self, index: usize, rect: Rect, pos: Pos2) -> Option<DVec3> {
        let cam = self.panes.get(index)?.camera;
        if cam.is_2d() {
            let p = self.snapped_plane_point(index, pos, rect)?;
            return Some(self.clip_depth(index, p));
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

    /// The clip plane: the line plus the direction of the view or face it was drawn on, or three clicked points.
    fn clip_plane(&self) -> Option<[DVec3; 3]> {
        match self.clip_points[..] {
            [a, b, c] if (b - a).cross(c - a).length() > 1e-6 => Some([a, b, c]),
            [a, b, ..] if a.distance(b) > 1e-6 => {
                let view = self.panes.iter().find(|p| p.camera.is_2d()).map_or(DVec3::Z, |p| p.camera.view.axes().2);
                Some([a, b, b + self.clip_view_axis.unwrap_or(view) * 64.0])
            }
            _ => None,
        }
    }

    /// Keeps the bigger part of the selected brushes, since a fresh cut usually trims a corner off.
    fn choose_clip_side(&mut self) {
        use mapcraft_engine::ClipKeep as K;
        let Some(plane) = self.clip_plane() else { return };
        let volume = |b: Option<mapcraft_map::Brush>| b.and_then(|b| b.solid().ok()).map_or(0.0, |s| s.volume());
        let (mut front, mut back) = (0.0, 0.0);
        for id in self.session.selected_brushes().into_iter().take(64) {
            let Some(b) = self.session.map.get(id).and_then(|n| n.kind.brush()) else { continue };
            let Ok((f, k)) = b.split(plane, &self.session.tex) else { continue };
            front += volume(f);
            back += volume(k);
        }
        self.clip_keep = if front >= back { K::Front } else { K::Back };
    }

    /// The clip point drawn under the pointer in view `index`, if any.
    fn clip_handle_at(&self, index: usize, rect: Rect, pos: Pos2) -> Option<usize> {
        let cam = self.panes.get(index)?.camera;
        let size = dvec(rect.size());
        let screen = |p: DVec3| cam.project(p, size).map(|q| rect.min + Vec2::new(q.x as f32, q.y as f32));
        self.clip_points
            .iter()
            .enumerate()
            .filter_map(|(i, p)| Some((i, screen(*p)?.distance(pos))))
            .filter(|(_, d)| *d <= 8.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    /// The clip points as square handles, with their coordinates in the 2D views.
    fn draw_clip_points(&self, ui: &egui::Ui, index: usize, rect: Rect) {
        let Some(cam) = self.panes.get(index).map(|p| p.camera).filter(|_| self.tool == Tool::Clip) else { return };
        let painter = ui.painter_at(rect);
        let (r, u, _) = cam.view.axes();
        let (ri, ui_) = (r.abs().max_position(), u.abs().max_position());
        for p in &self.clip_points {
            let Some(q) = cam.project(*p, dvec(rect.size())) else { continue };
            let c = rect.min + Vec2::new(q.x as f32, q.y as f32);
            painter.rect(
                Rect::from_center_size(c, Vec2::splat(8.0)),
                1.0,
                Color32::from_rgb(255, 255, 51),
                Stroke::new(1.0, Color32::BLACK),
                egui::StrokeKind::Middle,
            );
            if cam.is_2d() {
                let text = format!("{} {}", crate::measure::units(p[ri]), crate::measure::units(p[ui_]));
                crate::measure::label(&painter, c + Vec2::new(8.0, -8.0), egui::Align2::LEFT_BOTTOM, text);
            }
        }
    }

    pub(crate) fn apply_clip(&mut self) {
        let Some(points) = self.clip_plane() else { return };
        if let Err(e) = self.session.clip(points, self.clip_keep) {
            self.status = e.to_string();
        }
        self.clip_points.clear();
        self.mark_changed();
    }

    /// Takes back the last clip point placed.
    pub(crate) fn pop_clip_point(&mut self) {
        if self.clip_points.pop().is_none() {
            self.status = "No clip points to remove".into();
        } else if self.clip_points.is_empty() {
            self.clip_view_axis = None;
        } else {
            self.choose_clip_side();
        }
    }

    /// Puts the clip plane on the plane of the face under the pointer in 3D, so the cut meets it without a gap.
    fn clip_to_face(&mut self, index: usize, rect: Rect, pos: Pos2) {
        let Some(cam) = self.panes.get(index).map(|p| p.camera) else { return };
        let (origin, dir) = cam.ray(dvec(pos - rect.min), dvec(rect.size()));
        let face =
            self.pick_face(origin, dir, true).and_then(|(id, face, _)| self.session.map.get(id)?.kind.brush()?.faces.iter().find(|f| f.id == face).cloned());
        if let Some(f) = face {
            self.clip_points = f.points.to_vec();
            self.clip_view_axis = f.plane().map(|p| p.normal);
            self.clip_view = Some(index);
            self.choose_clip_side();
        }
    }

    /// The block being dragged out, drawn as the shape it will make inside a faint box.
    fn shape_ghost(&self, out: &mut Vec<LineVertex>, b: Aabb) {
        if self.shape == mapcraft_engine::Shape::Block {
            box_lines(out, b.min, b.max, PREVIEW);
            return;
        }
        box_lines(out, b.min, b.max, [PREVIEW[0], PREVIEW[1], PREVIEW[2], 0.3]);
        for brush in self.shape.brushes(b, &self.session.tex).unwrap_or_default() {
            let Ok(solid) = brush.solid() else { continue };
            for w in solid.faces.iter().flatten() {
                for (i, p) in w.0.iter().enumerate() {
                    out.push(LineVertex::new(*p, PREVIEW));
                    out.push(LineVertex::new(w.0[(i + 1) % w.0.len()], PREVIEW));
                }
            }
        }
    }

    fn overlay_lines(&self, flat: bool) -> Vec<LineVertex> {
        let mut out = Vec::new();
        if let Drag::Block { start, end, .. } = &self.drag {
            self.shape_ghost(&mut out, self.block_box(*start, *end));
        }
        if let Some(b) = self.block_3d_box() {
            self.shape_ghost(&mut out, b);
        }
        if let (Tool::Clip, [a, b, c]) = (self.tool, &self.clip_points[..]) {
            out.extend([a, b, b, c, c, a].map(|p| LineVertex::new(*p, PREVIEW)));
        } else if self.tool == Tool::Clip
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
            && let Some(plane) = self.clip_plane()
        {
            use mapcraft_engine::ClipKeep as K;
            for id in self.session.selected_brushes().into_iter().take(64) {
                let Some(brush) = self.session.map.get(id).and_then(|n| n.kind.brush()) else { continue };
                let Ok((front, back)) = brush.split(plane, &self.session.tex) else { continue };
                for (part, kept) in [(front, self.clip_keep != K::Back), (back, self.clip_keep != K::Front)] {
                    let Some(solid) = part.and_then(|p| p.solid().ok()) else { continue };
                    let color = if kept { PREVIEW } else { [0.95, 0.22, 0.18, 1.0] };
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
        if self.tool == Tool::Entity
            && let Some((_, at)) = self.ghost_at
        {
            let b = self.new_entity_box(at);
            box_lines(&mut out, b.min, b.max, PREVIEW);
        }
        if let Some(trail) = &self.pointfile {
            out.extend(trail.points.windows(2).flat_map(|w| w.iter().map(|p| LineVertex::new(*p, LEAK))));
        }
        self.link_lines(&mut out);
        self.mirror_lines(&mut out);
        self.path_lines(&mut out);
        for c in self.session.map.cordons.iter().filter(|c| c.active) {
            box_lines(&mut out, c.bounds.min, c.bounds.max, [0.3, 0.6, 1.0, 1.0]);
        }
        if let Some((low, high)) = self.project.game.map_size.filter(|(low, high)| flat && low < high) {
            box_lines(&mut out, DVec3::splat(low as f64), DVec3::splat(high as f64), [0.7, 0.25, 0.25, 1.0]);
        }
        if matches!(self.drag, Drag::None | Drag::Extrude { .. }) {
            for poly in self.manipulated_faces() {
                out.extend(poly.iter().zip(poly.iter().cycle().skip(1)).flat_map(|(a, b)| [LineVertex::new(*a, PREVIEW), LineVertex::new(*b, PREVIEW)]));
            }
        }
        let guide = match self.drag {
            Drag::Extrude { .. } => self.measured_box(),
            _ => self.selection_bounds().filter(|_| matches!(self.tool, Tool::Select | Tool::Move | Tool::Rotate | Tool::Scale)),
        };
        if let Some(b) = guide {
            box_lines(&mut out, b.min, b.max, [SELECTED_EDGE[0], SELECTED_EDGE[1], SELECTED_EDGE[2], 0.35]);
        }
        out
    }

    /// The plane of each mirror group that is selected or holds the selection, outlined across the group.
    fn mirror_lines(&self, out: &mut Vec<LineVertex>) {
        let map = &self.session.map;
        let mut mirrors: BTreeMap<NodeId, Mirror> = BTreeMap::new();
        for id in &self.session.selection {
            for a in std::iter::once(*id).chain(map.ancestors(*id)) {
                if let Some(NodeKind::Group(Group { mirror: Some(m), .. })) = map.get(a).map(|n| &n.kind) {
                    mirrors.insert(a, *m);
                }
            }
        }
        let color = egui::Rgba::from(crate::chrome::MIRROR).to_array();
        for (g, m) in mirrors {
            let Some(b) = self.node_bounds(g).map(|b| b.expanded(32.0)) else { continue };
            let (i, j, k) = (m.axis.index(), (m.axis.index() + 1) % 3, (m.axis.index() + 2) % 3);
            let corner = |u: f64, v: f64| {
                let mut p = DVec3::ZERO;
                (p[i], p[j], p[k]) = (m.offset, u, v);
                p
            };
            let c = [corner(b.min[j], b.min[k]), corner(b.max[j], b.min[k]), corner(b.max[j], b.max[k]), corner(b.min[j], b.max[k])];
            for (a, b) in c.iter().zip(c.iter().cycle().skip(1)) {
                out.push(LineVertex::new(*a, color));
                out.push(LineVertex::new(*b, color));
            }
        }
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
    pub(crate) fn block_box(&self, start: DVec3, end: DVec3) -> Aabb {
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
                    NodeKind::Path(p) => pts.extend(p.points.iter().map(|q| (self.owner(id), q.pos))),
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
        let overlay = self.overlay_lines(self.panes.get(index).is_some_and(|p| p.camera.is_2d()));
        let grid = self.view_grid(index);
        let mode = self.render_mode;
        let sky = self.prefs.get().view.sky;
        let weights = self.tool == Tool::Sculpt
            && matches!(self.sculpt.0, mapcraft_engine::SculptMode::Paint | mapcraft_engine::SculptMode::Blur)
            && self.paint.show_weights;
        if let (Some(gpu), Some(pane)) = (&mut self.gpu, self.panes.get_mut(index)) {
            pane.rect = rect;
            gpu.renderer.show_sky = sky;
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
                gpu.renderer.set_weight_overlay(if weights { 0.5 } else { 0.0 });
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
        if self.tool == Tool::Vertex {
            self.draw_components(ui, index, rect);
        }
        self.draw_path_handles(ui, index, rect);
        for (_, a, b, color) in self.gizmo(index, rect) {
            ui.painter_at(rect).line_segment([a, b], Stroke::new(3.0, color));
            ui.painter_at(rect).circle_filled(b, 5.0, color);
        }
        self.draw_transform_gizmos(ui, index, rect);
        self.draw_measurements(ui, index, rect);
        self.draw_clip_points(ui, index, rect);
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
        for (_, c) in self.origin_handles(index, rect) {
            let painter = ui.painter_at(rect);
            painter.circle(c, 5.0, Color32::WHITE, Stroke::new(1.0, Color32::BLACK));
            painter.line_segment([c - Vec2::new(3.0, 0.0), c + Vec2::new(3.0, 0.0)], Stroke::new(1.0, Color32::BLACK));
            painter.line_segment([c - Vec2::new(0.0, 3.0), c + Vec2::new(0.0, 3.0)], Stroke::new(1.0, Color32::BLACK));
        }
    }

    /// Where the origins of the selected brush entities show as handles in a 2D view.
    fn origin_handles(&self, index: usize, rect: Rect) -> Vec<(NodeId, Pos2)> {
        let Some(cam) = self.panes.get(index).map(|p| p.camera).filter(|c| c.is_2d()) else { return Vec::new() };
        let size = dvec(rect.size());
        self.session
            .selected_brush_entities()
            .into_iter()
            .filter_map(|id| {
                let q = cam.project(self.session.map.get(id)?.kind.entity()?.origin()?, size)?;
                Some((id, rect.min + Vec2::new(q.x as f32, q.y as f32)))
            })
            .collect()
    }

    fn origin_handle_at(&self, index: usize, rect: Rect, pos: Pos2) -> Option<NodeId> {
        self.origin_handles(index, rect).into_iter().find(|(_, c)| c.distance(pos) <= 7.0).map(|(id, _)| id)
    }

    fn viewport_input(&mut self, ui: &egui::Ui, index: usize, rect: Rect, response: &egui::Response) {
        let Some(pane) = self.panes.get(index).filter(|_| !self.uv_busy) else { return };
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
                    Some(h) => {
                        self.hover_sculpt(index, rect, h);
                    }
                    None if self.sculpt_at.is_some_and(|(v, ..)| v == index) => (self.sculpt_at, self.blend_readout) = (None, None),
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
            if self.tool == Tool::Entity {
                match hover {
                    Some(h) => self.ghost_at = self.entity_spot(index, rect, h).map(|p| (index, p)),
                    None if self.ghost_at.is_some_and(|(v, _)| v == index) => self.ghost_at = None,
                    None => {}
                }
            }
            match hover {
                Some(h) if cam.is_2d() && matches!(self.tool, Tool::Block | Tool::Clip | Tool::Entity | Tool::Path) => {
                    self.snapped_plane_point(index, h, rect);
                }
                Some(h) if matches!(self.tool, Tool::Block | Tool::Path) => {
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
                self.update_drag(index, rect, p, mods);
            }
        }
        if response.drag_stopped()
            && let Some(p) = primary_pos.or(hover)
        {
            self.end_drag(index, rect, p);
        }
        let alt = ctx.input(|i| i.modifiers.alt);
        let painting = self.tool == Tool::Sculpt && matches!(self.sculpt.0, mapcraft_engine::SculptMode::Paint | mapcraft_engine::SculptMode::Blur);
        let primary = response.clicked_by(egui::PointerButton::Primary);
        if alt && (!cam.is_2d() || (painting && primary)) && (primary || response.clicked_by(egui::PointerButton::Secondary)) {
            if let Some(p) = primary_pos.or(hover) {
                let (origin, dir) = cam.ray(dvec(p - rect.min), size);
                if let Some((id, face, t)) = self.pick_face(origin, dir, primary) {
                    if primary {
                        let material = self.session.map.get(id).and_then(|n| n.kind.brush()).and_then(|b| b.faces.iter().find(|f| f.id == face)).cloned();
                        if let Some(layer) = self.dominant_layer(id, face, origin + dir * t).filter(|_| painting) {
                            self.status = format!("Picked {layer}");
                            self.session.tex.material = layer;
                        } else if let Some(f) = material {
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
        } else if primary && let Some(p) = primary_pos {
            let mods = ctx.input(|i| i.modifiers);
            if mods.shift && !cam.is_2d() && matches!(self.tool, Tool::Select | Tool::Move) {
                self.pick_face_at(index, rect, p);
            } else {
                self.click(index, rect, p, mods.command || (mods.shift && self.tool != Tool::Sculpt));
            }
        }
        if response.double_clicked() && cam.is_2d() {
            self.frame_selection();
        } else if response.double_clicked()
            && self.tool == Tool::Clip
            && let Some(p) = primary_pos
        {
            self.clip_to_face(index, rect, p);
        }
        // Alt+right click applies materials and right click cancels a bevel being sized.
        if !alt && self.vertex.bevel.is_none() {
            response.context_menu(|ui| self.viewport_menu(ui, index));
        }
    }

    pub(crate) fn frame_selection(&mut self) {
        if let Some(b) = self.selection_bounds() {
            self.frame_bounds(b);
        }
    }

    /// Where a click places something: on the view plane at the last brush's depth in 2D, on the
    /// ground or the face under the pointer in 3D.
    pub(crate) fn placement_at(&mut self, index: usize, rect: Rect, pos: Pos2) -> Option<DVec3> {
        let cam = self.panes.get(index)?.camera;
        if !cam.is_2d() {
            return self.block_base(index, rect, pos).map(|(_, _, p)| p);
        }
        let p = self.snapped_plane_point(index, pos, rect)?;
        let depth = cam.view.axes().2.abs();
        Some(p + depth * self.last_box.min.dot(depth))
    }

    /// The keys a new entity of the picked class starts with: its class defaults and model.
    fn new_entity_keys(&self) -> Vec<(String, String)> {
        let class = &self.entity_class;
        let defaults = self
            .project
            .game
            .class(class)
            .map(|c| c.properties.iter().filter(|p| p.name != "origin").filter_map(|p| Some((p.name.clone(), p.default.clone()?))).collect())
            .unwrap_or_default();
        self.project.with_model(class, defaults)
    }

    /// The box a new entity of the picked class fills with its origin at `at`.
    fn new_entity_box(&self, at: DVec3) -> Aabb {
        let mut e = mapcraft_map::Entity::new(self.entity_class.clone());
        e.properties = self.new_entity_keys();
        e.set("origin", format!("{} {} {}", at.x, at.y, at.z));
        self.point_box(&e)
    }

    /// Where the Entity tool puts a new entity's origin: its box resting on the face or ground
    /// under the pointer in the 3D view, at the last brush's depth in 2D.
    pub(crate) fn entity_spot(&mut self, index: usize, rect: Rect, pos: Pos2) -> Option<DVec3> {
        let cam = self.panes.get(index)?.camera;
        if cam.is_2d() {
            return self.placement_at(index, rect, pos);
        }
        let (origin, dir) = cam.ray(dvec(pos - rect.min), dvec(rect.size()));
        let map = self.view_map();
        let plane = match face_on_ray(map, origin, dir) {
            Some((id, i, _)) => map.get(id)?.kind.brush()?.faces.get(i)?.plane()?,
            None if dir.z.abs() > 1e-9 && (0.0..=4096.0).contains(&(-origin.z / dir.z)) => {
                mapcraft_geom::Plane { normal: DVec3::Z * origin.z.signum(), dist: 0.0 }
            }
            None => return Some(if self.prefs.get().view.snap_grid { snap(origin + dir * 256.0, self.session.grid) } else { origin + dir * 256.0 }),
        };
        let den = dir.dot(plane.normal);
        if den.abs() < 1e-9 {
            return None;
        }
        let hit = self.grid_on_plane(plane, origin + dir * ((plane.dist - origin.dot(plane.normal)) / den));
        let b = self.new_entity_box(DVec3::ZERO);
        let n = plane.normal;
        let lowest = DVec3::select(n.cmpge(DVec3::ZERO), b.min, b.max);
        Some(hit - n * lowest.dot(n))
    }

    /// Picks the face under the pointer in the Face tool, as Shift+click does from Select or Move.
    fn pick_face_at(&mut self, index: usize, rect: Rect, pos: Pos2) {
        let Some(cam) = self.panes.get(index).map(|p| p.camera) else { return };
        let (origin, dir) = cam.ray(dvec(pos - rect.min), dvec(rect.size()));
        let Some((id, face, _)) = self.pick_face(origin, dir, false) else { return };
        self.session.selection.clear();
        self.run_action("tool.face");
        self.session.face_selection = BTreeSet::from([(id, face)]);
        self.session.active_face = Some((id, face));
        self.mark_changed();
    }

    fn click(&mut self, index: usize, rect: Rect, pos: Pos2, additive: bool) {
        let Some(cam) = self.panes.get(index).map(|p| p.camera) else { return };
        if self.tool == Tool::Block && self.plugin_shape.is_some() {
            if let Some(p) = self.placement_at(index, rect, pos) {
                self.place_plugin_shape(p);
            }
            return;
        }
        if self.path_click(index, rect, pos, additive) {
            return;
        }
        if self.tool == Tool::Vertex && self.vertex_click(index, rect, pos, additive) {
            self.mark_changed();
            return;
        }
        let size = dvec(rect.size());
        let (origin, dir) = cam.ray(dvec(pos - rect.min), size);
        match self.tool {
            Tool::Select | Tool::Move | Tool::Extrude | Tool::Rotate | Tool::Scale | Tool::Block | Tool::Vertex => {
                // Clicking the same spot again steps to the next object behind (drill select).
                let mut targets = BTreeSet::new();
                let hits: Vec<NodeId> = self
                    .pick_all(origin, dir, false)
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
            Tool::Sculpt => match self.sculpt_hit(index, rect, pos) {
                Some((p, _)) => {
                    self.session.end_coalesce();
                    if let Err(e) = self.dab(&[p], p, 0.05, additive) {
                        self.refuse(e);
                    }
                    self.session.end_coalesce();
                }
                None => self.status = self.nothing_to_sculpt(),
            },
            Tool::Face => {
                let hit = self.pick_face(origin, dir, false).map(|(id, f, _)| (id, f));
                if !additive {
                    self.session.face_selection.clear();
                }
                if let Some(h) = hit
                    && !self.session.face_selection.remove(&h)
                {
                    self.session.face_selection.insert(h);
                    self.session.active_face = Some(h);
                }
                self.mark_changed();
            }
            Tool::Entity => {
                let Some(point) = self.entity_spot(index, rect, pos) else { return };
                let class = self.entity_class.clone();
                if let Err(e) = self.session.create_point_entity(&class, point, self.new_entity_keys()) {
                    self.status = e.to_string();
                }
                self.mark_changed();
            }
            Tool::Clip => {
                // Clicking again in the 2D view that drew the line starts a new line; a third point elsewhere tilts the cut.
                if self.clip_points.len() >= 3 || (self.clip_points.len() == 2 && cam.is_2d() && self.clip_view == Some(index)) {
                    self.clip_points.clear();
                }
                if let Some((p, t)) = self.clip_point(index, rect, pos) {
                    if let [a, b] = self.clip_points[..]
                        && (b - a).cross(p - a).length() < 1e-6
                    {
                        self.status = "That point lines up with the other two; put the third off their line to tilt the cut".into();
                        return;
                    }
                    if self.clip_points.is_empty() {
                        self.clip_view_axis = Some(t);
                        self.clip_view = Some(index);
                    }
                    self.clip_points.push(p);
                    self.choose_clip_side();
                }
            }
            Tool::Path => {}
        }
    }

    fn begin_drag(&mut self, index: usize, rect: Rect, pos: Pos2, mods: egui::Modifiers) {
        let Some(cam) = self.panes.get(index).map(|p| p.camera) else { return };
        let size = dvec(rect.size());
        if self.begin_path_drag(index, rect, pos, mods) {
            return;
        }
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
            if let Some(point) = self.clip_handle_at(index, rect, pos) {
                self.drag = Drag::ClipPoint { view: index, point };
                return;
            }
            if let Some((p, t)) = self.clip_point(index, rect, pos) {
                self.clip_points = vec![p, p];
                self.clip_view_axis = Some(t);
                self.clip_view = Some(index);
                self.drag = Drag::Clip { view: index };
            }
            return;
        }
        if self.tool == Tool::Sculpt {
            match self.sculpt_hit(index, rect, pos) {
                Some((p, _)) => {
                    self.session.end_coalesce();
                    let result = self.dab(&[p], p, 1.0 / 60.0, mods.command);
                    if let Err(e) = &result {
                        self.refuse(e.clone());
                    }
                    self.drag =
                        Drag::Sculpt { view: index, level: p, last: std::time::Instant::now(), from: Some(p), refused: result.is_err(), invert: mods.command };
                }
                None => self.status = self.nothing_to_sculpt(),
            }
            return;
        }
        if self.tool == Tool::Vertex {
            self.begin_vertex_drag(index, rect, pos, mods);
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
            let arrow = self.gizmo(index, rect).into_iter().find(|(_, a, b, _)| segment_distance(pos, *a, *b) < 6.0);
            if let (Some((axis, ..)), Some(b)) = (arrow, self.selection_bounds()) {
                let (origin, dir) = cam.ray(dvec(pos - rect.min), size);
                if let Some(start) = closest_on_axis(b.center(), axis, origin, dir) {
                    self.drag = Drag::Axis { view: index, axis, center: b.center(), start, base: b.min.dot(axis), applied: 0.0, clone: mods.command };
                }
            }
            return;
        }
        match self.tool {
            Tool::Select | Tool::Move | Tool::Scale => {
                if let Some(entity) = self.origin_handle_at(index, rect, pos) {
                    self.drag = Drag::Origin { view: index, entity };
                    return;
                }
                if let Some((handle, _)) = self.handle_rects(index, rect).into_iter().find(|(_, r)| r.expand(3.0).contains(pos))
                    && let Some(b) = self.selection_bounds()
                {
                    self.drag = Drag::Resize { view: index, handle, bounds: b, applied: b };
                    return;
                }
                let (origin, dir) = cam.ray(dvec(pos - rect.min), size);
                let hit = self.pick(origin, dir, false).map(|(id, _)| pick_target(&self.session.map, id, self.session.select_mode));
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
                        self.drag = Drag::Move { view: index, start, anchor, applied: DVec3::ZERO, clone: mods.command };
                    }
                    None => self.drag = Drag::Marquee { view: index, start: pos },
                }
            }
            Tool::Block => {
                if let Some(p) = self.snapped_plane_point(index, pos, rect) {
                    self.drag = Drag::Block { view: index, start: p, end: p };
                }
            }
            Tool::Entity | Tool::Clip | Tool::Face | Tool::Extrude | Tool::Rotate | Tool::Sculpt | Tool::Vertex | Tool::Path => {}
        }
    }

    /// Follows the pointer in a drag; Shift, or Alt where it means nothing else, moves 3D view handles up and down.
    fn update_drag(&mut self, index: usize, rect: Rect, pos: Pos2, mods: egui::Modifiers) {
        let vertical = mods.shift || mods.alt;
        let Some(cam) = self.panes.get(index).map(|p| p.camera) else { return };
        let size = dvec(rect.size());
        let g = self.view_grid(index);
        let (r, u, _) = cam.view.axes();
        let cursor = cam.plane_point(dvec(pos - rect.min), size);
        let on_grid = self.prefs.get().view.snap_grid;
        match self.drag.clone() {
            Drag::Extrude { view, .. } if view == index => self.update_extrude(index, rect, pos),
            Drag::Block3d { view, .. } if view == index => self.update_block_3d(index, rect, pos, vertical),
            Drag::Clip { view } if view == index => {
                if let Some(p) = self.clip_drag_point(index, rect, pos)
                    && let Some(end) = self.clip_points.get_mut(1)
                {
                    *end = p;
                    self.choose_clip_side();
                }
            }
            Drag::ClipPoint { view, point } if view == index => {
                let old = self.clip_points.get(point).copied();
                let moved = if cam.is_2d() {
                    // Points keep their depth, so a point placed in another view stays put along this one.
                    let depth = cam.view.axes().2.abs().max_position();
                    self.snapped_plane_point(index, pos, rect).zip(old).map(|(mut p, old)| {
                        p[depth] = old[depth];
                        p
                    })
                } else {
                    self.clip_point(index, rect, pos).map(|(p, _)| p)
                };
                if let (Some(p), Some(slot)) = (moved, self.clip_points.get_mut(point)) {
                    *slot = p;
                    self.choose_clip_side();
                }
            }
            Drag::Rotate { view, .. } if view == index => self.update_rotate(index, rect, pos),
            // Alt on a path point pulls out its handles.
            Drag::Path { view, .. } if view == index => self.update_path_drag(index, rect, pos, mods.shift),
            Drag::Origin { view, entity } if view == index => {
                let Some(old) = self.session.map.get(entity).and_then(|n| n.kind.entity()).and_then(|e| e.origin()) else { return };
                let d = cam.view.axes().2;
                let (p, caught) = self.snap_point(index, rect, cursor + d * old.dot(d), &BTreeSet::new(), &[]);
                self.snap_mark = Some((index, p, caught));
                if let Err(e) = self.session.set_origins(&[entity], Some(p), Some("drag-origin")) {
                    self.status = e.to_string();
                }
                self.mark_changed();
            }
            Drag::Scale { view, .. } if view == index => self.update_scale(index, rect, pos),
            Drag::Move { view, start, anchor, applied, clone } if view == index => {
                let moving = self.moving_nodes();
                let (snapped, caught) = self.snap_point(index, rect, anchor + (cursor - start), &moving, &[]);
                self.snap_mark = Some((index, snapped, caught));
                let total = snapped - anchor;
                let step = total - applied;
                if step.length_squared() > 1e-12 {
                    self.clone_once(clone);
                    if let Err(e) = self.session.translate_selection(step, Some("drag-move")) {
                        self.status = e.to_string();
                    }
                    self.drag = Drag::Move { view, start, anchor, applied: total, clone: false };
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
                let (snapped, caught) = if self.vertex.mode == HandleKind::Vertex {
                    let dragged: Vec<DVec3> = originals.iter().flat_map(|(_, c)| c.iter().map(|o| *o + applied)).collect();
                    self.snap_point(index, rect, anchor + (cursor - start), &BTreeSet::new(), &dragged)
                } else {
                    // Edges and faces snap how far they travel, so corners that start on the grid stay on it.
                    let step = |x: f64| if on_grid { (x / g).round() * g } else { x };
                    let d = cursor - start;
                    (anchor + r * step(d.dot(r)) + u * step(d.dot(u)), false)
                };
                self.snap_mark = Some((index, snapped, caught));
                let total = snapped - anchor;
                if (total - applied).length_squared() > 1e-12 && self.drag_corners(&originals, applied, total) {
                    self.drag = Drag::Vertex { view, originals, start, anchor, applied: total };
                }
            }
            Drag::Vertex3d { view, originals, anchor, applied } if view == index => {
                let (origin, dir) = cam.ray(dvec(pos - rect.min), size);
                let relative = self.vertex.mode != HandleKind::Vertex;
                let snap_to = |x: f64, from: f64| match (on_grid, relative) {
                    (false, _) => x,
                    (true, false) => (x / g).round() * g,
                    (true, true) => from + ((x - from) / g).round() * g,
                };
                let at = anchor + applied;
                let target = if vertical {
                    let Some(t) = closest_on_axis(at, DVec3::Z, origin, dir) else { return };
                    DVec3::new(at.x, at.y, snap_to(at.z + t, anchor.z))
                } else {
                    let t = (at.z - origin.z) / dir.z;
                    if dir.z.abs() < 1e-9 || t <= 0.0 {
                        return;
                    }
                    let p = origin + dir * t;
                    DVec3::new(snap_to(p.x, anchor.x), snap_to(p.y, anchor.y), at.z)
                };
                self.snap_mark = Some((index, target, false));
                let total = target - anchor;
                if (total - applied).length_squared() > 1e-12 && self.drag_corners(&originals, applied, total) {
                    self.drag = Drag::Vertex3d { view, originals, anchor, applied: total };
                }
            }
            Drag::Axis { view, axis, center, start, base, applied, clone } if view == index => {
                let (origin, dir) = cam.ray(dvec(pos - rect.min), size);
                let Some(t) = closest_on_axis(center, axis, origin, dir) else { return };
                let total = if self.prefs.get().view.snap_grid { ((base + t - start) / g).round() * g - base } else { t - start };
                if total != applied {
                    self.clone_once(clone);
                    if let Err(e) = self.session.translate_selection(axis * (total - applied), Some("drag-move")) {
                        self.status = e.to_string();
                    }
                    self.drag = Drag::Axis { view, axis, center, start, base, applied: total, clone: false };
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

    /// Copies the selection in place as the first step of a Ctrl drag, so undo takes the copy and its move together.
    fn clone_once(&mut self, now: bool) {
        if now && let Err(e) = self.session.duplicate_selection(DVec3::ZERO, Some("drag-move")) {
            self.status = e.to_string();
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
                let editable = self.view_map().editable_nodes();
                let mut picked = BTreeSet::new();
                for (id, _) in self.view_map().nodes() {
                    if !editable.contains(&id) {
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
            Drag::ComponentBox { view, start, add, remove } if view == index => self.box_select_components(index, rect, start, pos, add, remove),
            Drag::Move { .. }
            | Drag::Resize { .. }
            | Drag::Vertex { .. }
            | Drag::Vertex3d { .. }
            | Drag::Axis { .. }
            | Drag::Rotate { .. }
            | Drag::Scale { .. }
            | Drag::Path { .. }
            | Drag::Origin { .. } => self.session.end_coalesce(),
            d @ Drag::Extrude { .. } => self.end_extrude(d),
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

    /// Arrow keys move the selection a grid step; Shift moves it one unit and Ctrl moves a copy.
    fn nudge(&mut self, ui: &egui::Ui, index: usize) {
        let Some(cam) = self.panes.get(index).map(|p| p.camera) else { return };
        let (r, u, _) = cam.view.axes();
        let mods = ui.input(|i| i.modifiers);
        let g = if mods.shift { 1.0 } else { self.session.grid };
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
        if d != DVec3::ZERO && self.tool == Tool::Vertex && !self.vertex.selected.is_empty() {
            self.nudge_components(d);
        } else if d != DVec3::ZERO && !self.session.selection.is_empty() {
            let moved = if mods.command {
                let copied = self.session.duplicate_selection(DVec3::ZERO, Some("clone-nudge"));
                let moved = copied.and_then(|()| self.session.translate_selection(d, Some("clone-nudge")));
                self.session.end_coalesce();
                moved
            } else {
                self.session.translate_selection(d, Some("nudge"))
            };
            if let Err(e) = moved {
                self.status = e.to_string();
            }
            self.mark_changed();
        }
    }

    /// Hold the right mouse button to look around (WASD moves, Q/E go down/up, Shift is faster), or Alt and it to orbit.
    fn fly(&mut self, ui: &egui::Ui, index: usize, response: &egui::Response) {
        let speed = self.prefs.get().view.fly_speed;
        if response.drag_started_by(egui::PointerButton::Secondary) && ui.input(|i| i.modifiers.alt) {
            self.orbit = self.orbit_pivot(ui, index, response).map(|p| (index, p));
        }
        if !response.dragged_by(egui::PointerButton::Secondary) && self.orbit.is_some_and(|(v, _)| v == index) {
            self.orbit = None;
        }
        let pivot = self.orbit.filter(|(v, _)| *v == index).map(|(_, p)| p);
        let Some(pane) = self.panes.get_mut(index) else { return };
        let cam = &mut pane.camera;
        if let Some(pivot) = pivot {
            let d = response.drag_delta();
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            let yaw = -f64::from(d.x) * 0.25;
            let pitch = (cam.pitch - f64::from(d.y) * 0.25).clamp(-89.0, 89.0) - cam.pitch;
            // Turning the camera and its offset together keeps the pivot under the pointer.
            let turn = DQuat::from_rotation_z(yaw.to_radians()) * DQuat::from_axis_angle(cam.right(), pitch.to_radians());
            let off = turn * (cam.position - pivot);
            let dist = (off.length() * (-f64::from(scroll) / 250.0).exp()).max(32.0);
            cam.position = pivot + off.normalize_or_zero() * dist;
            cam.yaw += yaw;
            cam.pitch += pitch;
            return;
        }
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
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        // Shift turns the wheel into a sideways scroll, and Shift is held for flying fast.
        let fly_scroll = ui.input(|i| if i.smooth_scroll_delta.y != 0.0 { i.smooth_scroll_delta.y } else { i.smooth_scroll_delta.x });
        if fly_scroll != 0.0 && looking {
            let speed = fly_speed_scrolled(speed, fly_scroll);
            self.prefs.update(|p| p.view.fly_speed = speed);
            self.status = format!("Fly speed {speed:.0} units/s");
        } else if scroll != 0.0 && response.hovered() {
            cam.position += cam.forward() * f64::from(scroll) * 2.0;
        }
    }

    /// What an orbit in view `index` turns about: the surface under the press, else a point ahead.
    fn orbit_pivot(&self, ui: &egui::Ui, index: usize, response: &egui::Response) -> Option<DVec3> {
        let cam = self.panes.get(index)?.camera;
        let press = ui.input(|i| i.pointer.press_origin()).unwrap_or(response.rect.center());
        let (origin, dir) = cam.ray(dvec(press - response.rect.min), dvec(response.rect.size()));
        Some(origin + dir * self.pick(origin, dir, true).map_or(1024.0, |(_, t)| t))
    }
}

/// The fly speed after scrolling while flying, as Godot's freelook does; the range matches Preferences.
fn fly_speed_scrolled(speed: f64, scroll: f32) -> f64 {
    (speed * (f64::from(scroll) / 250.0).exp()).clamp(16.0, 8192.0)
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

/// Paint dab centers every `step` units from `from` toward `to`, past `from` and at most 64.
fn dabs_along(from: DVec3, to: DVec3, step: f64) -> Vec<DVec3> {
    let d = to - from;
    let count = (d.length() / step).floor().clamp(0.0, 64.0) as usize;
    let dir = d.normalize_or_zero();
    (1..=count).map(|i| from + dir * (step * i as f64)).collect()
}

fn wgpu_filter() -> eframe::wgpu::FilterMode {
    eframe::wgpu::FilterMode::Linear
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrolling_while_flying_scales_the_speed_within_range() {
        assert!(fly_speed_scrolled(512.0, 50.0) > 512.0);
        assert!(fly_speed_scrolled(512.0, -50.0) < 512.0);
        assert_eq!(fly_speed_scrolled(8000.0, 5000.0), 8192.0);
        assert_eq!(fly_speed_scrolled(20.0, -5000.0), 16.0);
    }

    #[test]
    fn paint_dabs_are_spaced_and_capped() {
        let dabs = dabs_along(DVec3::ZERO, DVec3::new(100.0, 0.0, 0.0), 32.0);
        assert_eq!(dabs, [DVec3::new(32.0, 0.0, 0.0), DVec3::new(64.0, 0.0, 0.0), DVec3::new(96.0, 0.0, 0.0)]);
        assert!(dabs_along(DVec3::ZERO, DVec3::new(10.0, 0.0, 0.0), 32.0).is_empty(), "holding still adds nothing");
        assert_eq!(dabs_along(DVec3::ZERO, DVec3::new(1e6, 0.0, 0.0), 1.0).len(), 64);
        assert!(dabs_along(DVec3::ZERO, DVec3::splat(f64::NAN), 1.0).is_empty());
    }
}
