//! Extrude, rotate and scale tools (TrenchBroom's extrude tool with Godot-style transform modes).

use egui::{Color32, Pos2, Rect, Stroke, Vec2};
use mapcraft_engine::ExtrudeMode;
use mapcraft_geom::{Aabb, DMat4, DQuat, DVec3};
use mapcraft_map::{FaceId, NodeId};
use mapcraft_render::Camera;

use crate::app::MapCraftApp;
use crate::viewport::{AXES, Drag, Tool, box_transform, closest_on_axis, dvec, segment_distance};

/// Rotation snaps to this many degrees while grid snapping is on.
const ROTATE_STEP: f64 = 15.0;
/// How close on screen, in points, the pointer must be to pick a face edge or a gizmo.
const REACH: f32 = 6.0;

/// The face (or coplanar faces of several brushes) an extrude would drag.
#[derive(Clone, Debug)]
pub(crate) struct ExtrudeHover {
    pub view: usize,
    pub faces: Vec<(NodeId, FaceId)>,
    pub polygons: Vec<Vec<DVec3>>,
    pub normal: DVec3,
    pub center: DVec3,
}

fn to_screen(cam: &Camera, rect: Rect, p: DVec3) -> Option<Pos2> {
    cam.project(p, dvec(rect.size())).map(|q| rect.min + Vec2::new(q.x as f32, q.y as f32))
}

/// The signed angle of `p` around `axis` through `center`, measured in a basis whose cross
/// product is `axis`, so a growing angle is a positive turn about it.
fn angle_about(axis: DVec3, center: DVec3, p: DVec3) -> f64 {
    let b1 = axis.any_orthonormal_vector();
    let b2 = axis.cross(b1);
    let d = p - center;
    d.dot(b2).atan2(d.dot(b1))
}

fn wrap(a: f64) -> f64 {
    (a + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
}

impl MapCraftApp {
    /// Where a block drawn in the 3D view starts: on the face under the pointer, else on the
    /// ground plane. Returns the plane's axis, which way the block rises, and the snapped point.
    pub(crate) fn block_base(&self, index: usize, rect: Rect, pos: Pos2) -> Option<(usize, f64, DVec3)> {
        let cam = self.panes.get(index)?.camera;
        if cam.is_2d() {
            return None;
        }
        let (origin, dir) = cam.ray(dvec(pos - rect.min), dvec(rect.size()));
        let (normal, mut hit, dist) = match self.pick_face(origin, dir) {
            Some((id, face, t)) => {
                let plane = self.session.map.get(id)?.kind.brush()?.faces.iter().find(|f| f.id == face)?.plane()?;
                (plane.normal, origin + dir * t, plane.dist)
            }
            None if dir.z.abs() > 1e-9 && -origin.z / dir.z > 0.0 => (DVec3::Z * origin.z.signum(), origin + dir * (-origin.z / dir.z), 0.0),
            None => return None,
        };
        let axis = normal.abs().max_position();
        let sign = if normal[axis] >= 0.0 { 1.0 } else { -1.0 };
        // Ray hits drift off axis-aligned faces by rounding error.
        if (normal[axis].abs() - 1.0).abs() < 1e-9 {
            hit[axis] = dist * sign;
        }
        Some((axis, sign, self.snap_in_plane(axis, hit)))
    }

    /// `p` with the two coordinates across `axis` on the grid while grid snapping is on.
    pub(crate) fn snap_in_plane(&self, axis: usize, p: DVec3) -> DVec3 {
        if !self.prefs.get().view.snap_grid {
            return p;
        }
        let g = self.session.grid;
        let mut out = p;
        for k in (0..3).filter(|k| *k != axis) {
            out[k] = (p[k] / g).round() * g;
        }
        out
    }

    pub(crate) fn begin_block_3d(&mut self, index: usize, rect: Rect, pos: Pos2) {
        let Some((axis, sign, start)) = self.block_base(index, rect, pos) else { return };
        let depth = self.last_box.size()[axis];
        let height = if depth > 0.0 { depth } else { self.session.grid.max(1.0) * 4.0 };
        self.drag = Drag::Block3d { view: index, axis, sign, offset: start[axis], start, end: start, height };
    }

    /// Drags the far corner across the base plane, or with Shift held, the height.
    pub(crate) fn update_block_3d(&mut self, index: usize, rect: Rect, pos: Pos2, shift: bool) {
        let Drag::Block3d { view, axis, sign, offset, start, end, height } = self.drag.clone() else { return };
        let Some(cam) = self.panes.get(index).map(|p| p.camera) else { return };
        let (origin, dir) = cam.ray(dvec(pos - rect.min), dvec(rect.size()));
        let mut normal = DVec3::ZERO;
        normal[axis] = sign;
        if shift {
            let Some(t) = closest_on_axis(end, normal, origin, dir) else { return };
            let g = self.session.grid.max(1.0);
            let h = if self.prefs.get().view.snap_grid { (t / g).round() * g } else { t };
            self.drag = Drag::Block3d { view, axis, sign, offset, start, end, height: h.max(g) };
            return;
        }
        let den = dir[axis];
        if den.abs() < 1e-9 {
            return;
        }
        let t = (offset - origin[axis]) / den;
        if t <= 0.0 {
            return;
        }
        let end = self.snap_in_plane(axis, origin + dir * t);
        self.snap_mark = Some((index, end, false));
        self.drag = Drag::Block3d { view, axis, sign, offset, start, end, height };
    }

    pub(crate) fn block_3d_box(&self) -> Option<Aabb> {
        self.block_3d_box_of(&self.drag)
    }

    /// The block a 3D block drag would make, if its base has an area.
    pub(crate) fn block_3d_box_of(&self, drag: &Drag) -> Option<Aabb> {
        let Drag::Block3d { axis, sign, offset, start, end, height, .. } = drag else { return None };
        let mut top = *end;
        top[*axis] = offset + sign * height;
        let b = Aabb::new(*start, top);
        let size = b.size();
        (0..3).filter(|k| k != axis).all(|k| size[k] > 1e-9).then_some(b)
    }

    /// Extrude takes over from the select tools while Shift is held, as in TrenchBroom.
    pub(crate) fn extrude_wanted(&self, shift: bool) -> bool {
        self.tool == Tool::Extrude || (matches!(self.tool, Tool::Select | Tool::Move) && shift)
    }

    /// The face of a selected brush under the pointer: the face hit in the 3D view, or in a 2D
    /// view the face seen edge-on nearest the pointer. Coplanar faces of other selected brushes
    /// come along.
    pub(crate) fn extrude_hit(&self, index: usize, rect: Rect, pos: Pos2) -> Option<ExtrudeHover> {
        let cam = self.panes.get(index)?.camera;
        let size = dvec(rect.size());
        let brushes: Vec<(NodeId, mapcraft_map::Brush)> =
            self.session.selected_brushes().into_iter().filter_map(|id| Some((id, self.session.map.get(id)?.kind.brush()?.clone()))).collect();
        let mut best: Option<(f64, NodeId, usize)> = None;
        if cam.is_2d() {
            let depth = cam.view.axes().2;
            let cursor = cam.plane_point(dvec(pos - rect.min), size);
            for (id, b) in &brushes {
                let Ok(solid) = b.solid() else { continue };
                for (i, w) in solid.faces.iter().enumerate() {
                    let (Some(w), Some(n)) = (w, b.faces.get(i).and_then(|f| f.plane()).map(|p| p.normal)) else { continue };
                    if n.dot(depth).abs() > 1e-6 {
                        continue;
                    }
                    let pts: Vec<Pos2> = w.0.iter().filter_map(|p| to_screen(&cam, rect, *p)).collect();
                    let dist = (0..pts.len()).map(|k| segment_distance(pos, pts[k], pts[(k + 1) % pts.len()])).fold(f32::INFINITY, f32::min);
                    if dist >= REACH {
                        continue;
                    }
                    // Of two faces under the pointer, the one facing it wins.
                    let facing = (cursor - w.center()).dot(n) >= 0.0;
                    let score = f64::from(dist) - if facing { REACH as f64 } else { 0.0 };
                    if best.is_none_or(|(s, ..)| score < s) {
                        best = Some((score, *id, i));
                    }
                }
            }
        } else {
            let (origin, dir) = cam.ray(dvec(pos - rect.min), size);
            for (id, b) in &brushes {
                let Some((t, i)) = b.ray_hit(origin, dir) else { continue };
                if best.is_none_or(|(s, ..)| t < s) {
                    best = Some((t, *id, i));
                }
            }
        }
        let (_, id, i) = best?;
        let plane = brushes.iter().find(|(b, _)| *b == id)?.1.faces.get(i)?.plane()?;
        let mut hover = ExtrudeHover { view: index, faces: Vec::new(), polygons: Vec::new(), normal: plane.normal, center: DVec3::ZERO };
        for (bid, b) in &brushes {
            let Ok(solid) = b.solid() else { continue };
            for (f, w) in b.faces.iter().zip(&solid.faces) {
                if let (Some(w), Some(p)) = (w, f.plane())
                    && p.approx_eq(&plane, 1e-3)
                {
                    hover.faces.push((*bid, f.id));
                    hover.polygons.push(w.0.clone());
                }
            }
        }
        let pts: Vec<DVec3> = hover.polygons.iter().flatten().copied().collect();
        hover.center = pts.iter().copied().sum::<DVec3>() / pts.len().max(1) as f64;
        Some(hover)
    }

    /// Where along the face normal the pointer is, for an extrude drag.
    fn extrude_param(&self, index: usize, rect: Rect, pos: Pos2, normal: DVec3, center: DVec3) -> Option<f64> {
        let cam = self.panes.get(index)?.camera;
        let size = dvec(rect.size());
        if cam.is_2d() {
            Some((cam.plane_point(dvec(pos - rect.min), size) - center).dot(normal))
        } else {
            let (origin, dir) = cam.ray(dvec(pos - rect.min), size);
            closest_on_axis(center, normal, origin, dir)
        }
    }

    /// Starts dragging the face under the pointer; Ctrl splits off a new brush, Ctrl+Alt stamps one.
    pub(crate) fn begin_extrude(&mut self, index: usize, rect: Rect, pos: Pos2, mods: egui::Modifiers) -> bool {
        let Some(hover) = self.extrude_hit(index, rect, pos) else { return false };
        let Some(start) = self.extrude_param(index, rect, pos, hover.normal, hover.center) else { return false };
        let mode = match (mods.command, mods.alt) {
            (true, true) => ExtrudeMode::Stamp,
            (true, false) => ExtrudeMode::Split,
            _ => ExtrudeMode::Resize,
        };
        let faces = hover.faces.iter().filter_map(|(id, f)| Some((*id, self.session.map.get(*id)?.kind.brush()?.clone(), *f))).collect();
        self.extrude_hover = None;
        self.drag = Drag::Extrude { view: index, faces, normal: hover.normal, center: hover.center, start, mode, created: Vec::new(), distance: 0.0 };
        true
    }

    pub(crate) fn update_extrude(&mut self, index: usize, rect: Rect, pos: Pos2) {
        let Drag::Extrude { view, faces, normal, center, start, mode, created, distance } = self.drag.clone() else { return };
        let Some(t) = self.extrude_param(index, rect, pos, normal, center) else { return };
        let raw = t - start;
        let g = self.view_grid(index);
        let dist = if !self.prefs.get().view.snap_grid {
            raw
        } else if normal.abs().max_element() > 0.9999 {
            // Axis-aligned faces land on grid planes; others move in grid steps.
            let p0 = center.dot(normal);
            ((p0 + raw) / g).round() * g - p0
        } else {
            (raw / g).round() * g
        };
        if (dist - distance).abs() < 1e-9 {
            return;
        }
        match self.session.extrude(&faces, dist, mode, &created, Some("drag-extrude")) {
            Ok(made) => {
                self.drag = Drag::Extrude { view, faces, normal, center, start, mode, created: made, distance: dist };
                self.mark_changed();
            }
            Err(e) => self.status = format!("Cannot extrude there: {e}"),
        }
    }

    /// The rotation rings drawn around the selection in the 3D view: axis and screen polyline.
    fn rotate_rings(&self, index: usize, rect: Rect) -> Vec<(DVec3, Vec<Pos2>, Color32)> {
        let (Some(cam), Some(b)) = (self.panes.get(index).map(|p| p.camera), self.selection_bounds()) else { return Vec::new() };
        if cam.is_2d() || self.tool != Tool::Rotate {
            return Vec::new();
        }
        let c = b.center();
        let radius = cam.position.distance(c) * 0.12;
        AXES.iter()
            .map(|(axis, color)| {
                let b1 = axis.any_orthonormal_vector();
                let b2 = axis.cross(b1);
                let pts = (0..=48)
                    .filter_map(|k| {
                        let a = std::f64::consts::TAU * f64::from(k) / 48.0;
                        to_screen(&cam, rect, c + (b1 * a.cos() + b2 * a.sin()) * radius)
                    })
                    .collect();
                (*axis, pts, *color)
            })
            .collect()
    }

    /// Starts turning the selection: about a picked ring in the 3D view, or about the view's
    /// depth axis through the selection's center in a 2D view.
    pub(crate) fn begin_rotate(&mut self, index: usize, rect: Rect, pos: Pos2) {
        let (Some(cam), Some(b)) = (self.panes.get(index).map(|p| p.camera), self.selection_bounds()) else { return };
        let center = b.center();
        let size = dvec(rect.size());
        let axis = if cam.is_2d() {
            let (r, u, _) = cam.view.axes();
            r.cross(u)
        } else {
            let near = |pts: &Vec<Pos2>| (1..pts.len()).map(|k| segment_distance(pos, pts[k - 1], pts[k])).fold(f32::INFINITY, f32::min);
            let Some((axis, ..)) =
                self.rotate_rings(index, rect).into_iter().filter(|(_, pts, _)| near(pts) < REACH).min_by(|a, b| near(&a.1).total_cmp(&near(&b.1)))
            else {
                return;
            };
            axis
        };
        let Some(angle) = self.rotate_angle(&cam, size, pos - rect.min, axis, center) else { return };
        self.drag = Drag::Rotate { view: index, axis, center, last: angle, turned: 0.0, applied: 0.0 };
    }

    fn rotate_angle(&self, cam: &Camera, size: mapcraft_geom::DVec2, at: Vec2, axis: DVec3, center: DVec3) -> Option<f64> {
        let p = if cam.is_2d() {
            cam.plane_point(dvec(at), size)
        } else {
            let (origin, dir) = cam.ray(dvec(at), size);
            let den = dir.dot(axis);
            if den.abs() < 1e-9 {
                return None;
            }
            origin + dir * ((center - origin).dot(axis) / den)
        };
        Some(angle_about(axis, center, p))
    }

    pub(crate) fn update_rotate(&mut self, index: usize, rect: Rect, pos: Pos2) {
        let Drag::Rotate { view, axis, center, last, turned, applied } = self.drag.clone() else { return };
        let Some(cam) = self.panes.get(index).map(|p| p.camera) else { return };
        let Some(angle) = self.rotate_angle(&cam, dvec(rect.size()), pos - rect.min, axis, center) else { return };
        let turned = turned + wrap(angle - last);
        let mut degrees = turned.to_degrees();
        if self.prefs.get().view.snap_grid {
            degrees = (degrees / ROTATE_STEP).round() * ROTATE_STEP;
        }
        let mut applied_now = applied;
        if (degrees - applied).abs() > 1e-9 {
            let m = DMat4::from_translation(center)
                * DMat4::from_quat(DQuat::from_axis_angle(axis, (degrees - applied).to_radians()))
                * DMat4::from_translation(-center);
            match self.session.transform_selection(&m, "Rotate", Some("drag-rotate")) {
                Ok(()) => applied_now = degrees,
                Err(e) => self.status = e.to_string(),
            }
            self.mark_changed();
        }
        self.drag = Drag::Rotate { view, axis, center, last: angle, turned, applied: applied_now };
    }

    /// The scale handles in the 3D view: axis index, screen start and end.
    fn scale_handles(&self, index: usize, rect: Rect) -> Vec<(usize, Pos2, Pos2, Color32)> {
        let (Some(cam), Some(b)) = (self.panes.get(index).map(|p| p.camera), self.selection_bounds()) else { return Vec::new() };
        if cam.is_2d() || self.tool != Tool::Scale {
            return Vec::new();
        }
        let c = b.center();
        let len = cam.position.distance(c) * 0.12;
        let Some(start) = to_screen(&cam, rect, c) else { return Vec::new() };
        AXES.iter().enumerate().filter_map(|(i, (axis, color))| Some((i, start, to_screen(&cam, rect, c + *axis * len)?, *color))).collect()
    }

    pub(crate) fn begin_scale(&mut self, index: usize, rect: Rect, pos: Pos2) -> bool {
        let handle = self.scale_handles(index, rect).into_iter().find(|(_, a, b, _)| segment_distance(pos, *a, *b) < REACH);
        let (Some((axis, ..)), Some(b), Some(cam)) = (handle, self.selection_bounds(), self.panes.get(index).map(|p| p.camera)) else { return false };
        let (origin, dir) = cam.ray(dvec(pos - rect.min), dvec(rect.size()));
        let Some(start) = closest_on_axis(b.center(), AXES[axis].0, origin, dir) else { return false };
        self.drag = Drag::Scale { view: index, axis, bounds: b, start, applied: b };
        true
    }

    pub(crate) fn update_scale(&mut self, index: usize, rect: Rect, pos: Pos2) {
        let Drag::Scale { view, axis, bounds, start, applied } = self.drag.clone() else { return };
        let Some(cam) = self.panes.get(index).map(|p| p.camera) else { return };
        let (origin, dir) = cam.ray(dvec(pos - rect.min), dvec(rect.size()));
        let Some(t) = closest_on_axis(bounds.center(), AXES[axis].0, origin, dir) else { return };
        let g = self.view_grid(index);
        let mut edge = bounds.max[axis] + (t - start);
        if self.prefs.get().view.snap_grid {
            edge = (edge / g).round() * g;
        }
        let mut target: Aabb = bounds;
        target.max[axis] = edge.max(bounds.min[axis] + g.max(1.0));
        if target != applied {
            if let Err(e) = self.session.transform_selection(&box_transform(&applied, &target), "Scale", Some("drag-scale")) {
                self.status = e.to_string();
            }
            self.drag = Drag::Scale { view, axis, bounds, start, applied: target };
            self.mark_changed();
        }
    }

    /// Rotation rings or a 2D pivot mark, and scale handles, for the active tool.
    pub(crate) fn draw_transform_gizmos(&self, ui: &egui::Ui, index: usize, rect: Rect) {
        let painter = ui.painter_at(rect);
        for (_, pts, color) in self.rotate_rings(index, rect) {
            painter.add(egui::Shape::line(pts, Stroke::new(2.5, color)));
        }
        if self.tool == Tool::Rotate
            && let (Some(cam), Some(b)) = (self.panes.get(index).map(|p| p.camera).filter(|c| c.is_2d()), self.selection_bounds())
            && let Some(c) = to_screen(&cam, rect, b.center())
        {
            painter.circle_stroke(c, 48.0, Stroke::new(1.5, crate::chrome::ACCENT.gamma_multiply(0.8)));
            painter.line_segment([c - Vec2::new(5.0, 0.0), c + Vec2::new(5.0, 0.0)], Stroke::new(1.5, crate::chrome::ACCENT));
            painter.line_segment([c - Vec2::new(0.0, 5.0), c + Vec2::new(0.0, 5.0)], Stroke::new(1.5, crate::chrome::ACCENT));
        }
        for (_, a, b, color) in self.scale_handles(index, rect) {
            painter.line_segment([a, b], Stroke::new(3.0, color));
            painter.rect_filled(Rect::from_center_size(b, Vec2::splat(9.0)), 1.0, color);
        }
    }
}
