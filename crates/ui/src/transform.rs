//! Extrude, rotate and scale tools (TrenchBroom's extrude tool with Godot-style transform modes).

use egui::{Color32, Pos2, Rect, Stroke, Vec2};
use mapcraft_engine::{ExtrudeMode, Push};
use mapcraft_geom::{Aabb, DMat4, DQuat, DVec3, Solid, snap_travel};
use mapcraft_map::{Brush, FaceId, NodeId, share_edge};
use mapcraft_render::Camera;

use crate::app::MapCraftApp;
use crate::viewport::{AXES, Drag, Tool, box_transform, closest_on_axis, dvec, segment_distance};

/// Rotation snaps to this many degrees while grid snapping is on.
const ROTATE_STEP: f64 = 15.0;
/// How close on screen, in points, the pointer must be to pick a face edge or a gizmo.
const REACH: f32 = 6.0;
/// How far past a brush, in points, Shift+drag in the select tools reaches for a silhouette edge in the 3D view.
const EDGE_REACH: f32 = 40.0;

/// The faces of one or more brushes an extrude would drag.
#[derive(Clone, Debug)]
pub(crate) struct ExtrudeHover {
    pub view: usize,
    pub faces: Vec<(NodeId, FaceId)>,
    pub polygons: Vec<Vec<DVec3>>,
    pub normal: DVec3,
    /// Where a 3D drag takes hold: the hit point or a point on the grabbed edge.
    pub anchor: DVec3,
    /// The visible face's normal when an edge was grabbed; the pointer then moves on its plane through `anchor`.
    pub reference: Option<DVec3>,
}

/// An edge of a selected brush between a face toward the eye and one edge-on or away from it.
struct Silhouette {
    screen: f32,
    id: NodeId,
    hidden: usize,
    visible: DVec3,
    ends: [DVec3; 2],
    /// The point of the edge nearest the pointer's ray.
    point: DVec3,
}

/// Each edge of `solid` with the indices of the two faces meeting there.
fn solid_edges(solid: &Solid) -> Vec<(DVec3, DVec3, usize, usize)> {
    let on = |w: &mapcraft_geom::Winding, p: DVec3| w.0.iter().any(|q| q.distance_squared(p) < 1e-6);
    let mut out = Vec::new();
    for (i, w) in solid.faces.iter().enumerate() {
        let Some(w) = w else { continue };
        for (a, b) in w.0.iter().zip(w.0.iter().cycle().skip(1)) {
            if let Some((j, _)) = solid.faces.iter().enumerate().skip(i + 1).find(|(_, o)| o.as_ref().is_some_and(|o| on(o, *a) && on(o, *b))) {
                out.push((*a, *b, i, j));
            }
        }
    }
    out
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
        let (normal, mut hit, dist) = match self.pick_face(origin, dir, true) {
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

    /// The faces a drag would move: the face hit or the hidden face at an outline edge in 3D, the nearest edge-on face in 2D.
    pub(crate) fn extrude_hit(&self, index: usize, rect: Rect, pos: Pos2) -> Option<ExtrudeHover> {
        let cam = self.panes.get(index)?.camera;
        let brushes: Vec<(NodeId, Brush, Solid)> = self
            .session
            .selected_brushes()
            .into_iter()
            .filter_map(|id| {
                let b = self.session.map.get(id)?.kind.brush()?.clone();
                let solid = b.solid().ok()?;
                Some((id, b, solid))
            })
            .collect();
        let (id, i, anchor, reference) = if cam.is_2d() {
            self.edge_on_face(&cam, rect, pos, &brushes).map(|(id, i)| (id, i, DVec3::ZERO, None))?
        } else {
            self.face_3d(&cam, rect, pos, &brushes)?
        };
        let face = brushes.iter().find(|(b, ..)| *b == id)?.1.faces.get(i)?;
        let group = self.session.extrude_group(id, face.id);
        let polygons: Vec<Vec<DVec3>> = group.iter().map(|(.., w)| w.0.clone()).collect();
        Some(ExtrudeHover { view: index, faces: group.iter().map(|(b, f, _)| (*b, *f)).collect(), polygons, normal: face.plane()?.normal, anchor, reference })
    }

    /// The edge-on face nearest the pointer in a 2D view; off the selection the Extrude tool reaches it from any distance.
    fn edge_on_face(&self, cam: &Camera, rect: Rect, pos: Pos2, brushes: &[(NodeId, Brush, Solid)]) -> Option<(NodeId, usize)> {
        let (r, u, depth) = cam.view.axes();
        let cursor = cam.plane_point(dvec(pos - rect.min), dvec(rect.size()));
        let (mut near, mut any): (Option<(f64, NodeId, usize)>, Option<(f64, NodeId, usize)>) = (None, None);
        for (id, b, solid) in brushes {
            for (i, (f, w)) in b.faces.iter().zip(&solid.faces).enumerate() {
                let (Some(w), Some(n)) = (w, f.plane().map(|p| p.normal)) else { continue };
                if n.dot(depth).abs() > 1e-6 {
                    continue;
                }
                let pts: Vec<Pos2> = w.0.iter().filter_map(|p| to_screen(cam, rect, *p)).collect();
                let dist = pts.iter().zip(pts.iter().cycle().skip(1)).map(|(a, b)| segment_distance(pos, *a, *b)).fold(f32::INFINITY, f32::min);
                // Of two faces under the pointer, the one facing it wins.
                let facing = (cursor - w.center()).dot(n) >= 0.0;
                let score = f64::from(dist) - if facing { f64::from(REACH) } else { 0.0 };
                if dist < REACH && near.is_none_or(|(s, ..)| score < s) {
                    near = Some((score, *id, i));
                }
                if any.is_none_or(|(s, ..)| score < s) {
                    any = Some((score, *id, i));
                }
            }
        }
        let within = |s: &Solid, a: DVec3| (s.bounds.min.dot(a)..=s.bounds.max.dot(a)).contains(&cursor.dot(a));
        let inside = brushes.iter().any(|(.., s)| within(s, r) && within(s, u));
        let far = any.filter(|_| self.tool == Tool::Extrude && !inside);
        near.or(far).map(|(_, id, i)| (id, i))
    }

    /// The face to drag in the 3D view, the grabbed point, and for a grabbed edge the normal of the visible face the pointer moves on.
    fn face_3d(&self, cam: &Camera, rect: Rect, pos: Pos2, brushes: &[(NodeId, Brush, Solid)]) -> Option<(NodeId, usize, DVec3, Option<DVec3>)> {
        let (origin, dir) = cam.ray(dvec(pos - rect.min), dvec(rect.size()));
        let hit = brushes.iter().filter_map(|(id, b, _)| b.ray_hit(origin, dir).map(|(t, i)| (t, *id, i))).min_by(|a, b| a.0.total_cmp(&b.0));
        let mut edges: Vec<Silhouette> = Vec::new();
        for (id, b, solid) in brushes {
            let toward = |k: usize| b.faces.get(k).and_then(|f| f.plane()).is_some_and(|p| p.distance(cam.position) > 1e-6);
            for (a, q, i, j) in solid_edges(solid) {
                let (hidden, shown) = match (toward(i), toward(j)) {
                    (true, false) => (j, i),
                    (false, true) => (i, j),
                    _ => continue,
                };
                let (Some(sa), Some(sq), Some(normal)) = (to_screen(cam, rect, a), to_screen(cam, rect, q), b.faces.get(shown).and_then(|f| f.plane())) else {
                    continue;
                };
                let t = closest_on_axis(a, q - a, origin, dir).unwrap_or(0.5).clamp(0.0, 1.0);
                edges.push(Silhouette { screen: segment_distance(pos, sa, sq), id: *id, hidden, visible: normal.normal, ends: [a, q], point: a + (q - a) * t });
            }
        }
        let nearest = |reach: f32, seam: bool| {
            edges
                .iter()
                .filter(|e| {
                    e.screen < reach && (!seam || brushes.iter().any(|(o, _, s)| *o != e.id && s.faces.iter().flatten().any(|w| share_edge(&e.ends, &w.0))))
                })
                .min_by(|a, b| a.screen.total_cmp(&b.screen))
        };
        let grab = |e: &Silhouette| (e.id, e.hidden, e.point, Some(e.visible));
        let Some((t, id, i)) = hit else {
            let reach = if self.tool == Tool::Extrude { f32::INFINITY } else { EDGE_REACH };
            return nearest(reach, false).map(grab);
        };
        // A seam with another selected brush near the pointer wins, unless a face away from it is in front.
        let seam = nearest(REACH, true).filter(|e| {
            let on_edge = brushes.iter().find(|(b, ..)| *b == id).and_then(|(.., s)| s.faces.get(i)?.as_ref()).is_some_and(|w| share_edge(&e.ends, &w.0));
            on_edge || t >= (e.point - origin).dot(dir)
        });
        Some(seam.map_or((id, i, origin + dir * t, None), grab))
    }

    /// The point under the pointer a face drag measures from: on the view plane in 2D, else on the normal through `anchor` or on the `reference` plane.
    fn extrude_point(&self, index: usize, rect: Rect, pos: Pos2, normal: DVec3, anchor: DVec3, reference: Option<DVec3>) -> Option<DVec3> {
        let cam = self.panes.get(index)?.camera;
        let size = dvec(rect.size());
        if cam.is_2d() {
            return Some(cam.plane_point(dvec(pos - rect.min), size));
        }
        let (origin, dir) = cam.ray(dvec(pos - rect.min), size);
        match reference {
            Some(r) => {
                let den = dir.dot(r);
                let t = (anchor - origin).dot(r) / den;
                (den.abs() > 1e-9 && t > 0.0).then(|| origin + dir * t)
            }
            None => closest_on_axis(anchor, normal, origin, dir).map(|t| anchor + normal * t),
        }
    }

    /// Starts dragging the face under the pointer; Ctrl splits off a new brush, Ctrl+Alt stamps one and Alt slides the face.
    pub(crate) fn begin_extrude(&mut self, index: usize, rect: Rect, pos: Pos2, mods: egui::Modifiers) -> bool {
        let Some(h) = self.extrude_hit(index, rect, pos) else { return false };
        let Some(start) = self.extrude_point(index, rect, pos, h.normal, h.anchor, h.reference) else { return false };
        let faces: Vec<(NodeId, Brush, FaceId)> =
            h.faces.iter().filter_map(|(id, f)| Some((*id, self.session.map.get(*id)?.kind.brush()?.clone(), *f))).collect();
        let seam = faces.iter().any(|(_, b, f)| b.faces.iter().find(|x| x.id == *f).and_then(|x| x.plane()).is_some_and(|p| p.normal.dot(h.normal) < 0.0));
        let mode = match (mods.command, mods.alt) {
            (true, true) => ExtrudeMode::Stamp,
            // Faces across a seam point both ways, so there is no single side to split off.
            (true, false) if !seam => ExtrudeMode::Split,
            (false, true) => ExtrudeMode::Slide,
            _ if self.prefs.get().view.extrude_along_normal => ExtrudeMode::AlongNormal,
            _ => ExtrudeMode::Resize,
        };
        let paths = match mode {
            ExtrudeMode::Resize | ExtrudeMode::Split => faces.iter().flat_map(|(_, b, f)| b.corner_paths(*f, h.normal)).collect(),
            ExtrudeMode::AlongNormal | ExtrudeMode::Stamp => h.polygons.iter().flatten().map(|c| (*c, h.normal)).collect(),
            ExtrudeMode::Slide => Vec::new(),
        };
        self.session.end_coalesce();
        self.extrude_hover = None;
        self.drag = Drag::Extrude {
            view: index,
            faces,
            normal: h.normal,
            anchor: h.anchor,
            reference: h.reference,
            start,
            paths,
            mode,
            created: Vec::new(),
            offset: DVec3::ZERO,
        };
        true
    }

    pub(crate) fn update_extrude(&mut self, index: usize, rect: Rect, pos: Pos2) {
        let Drag::Extrude { view, faces, normal, anchor, reference, start, paths, mode, created, offset } = self.drag.clone() else { return };
        let Some(p) = self.extrude_point(index, rect, pos, normal, anchor, reference) else { return };
        let snap = self.prefs.get().view.snap_grid;
        let g = self.view_grid(index);
        let raw = p - start;
        let moved = if mode == ExtrudeMode::Slide {
            // In a 2D view a slide moves freely in the view plane.
            let free = self.panes.get(index).is_some_and(|pane| pane.camera.is_2d());
            let v = if free { raw } else { normal * raw.dot(normal) };
            if snap { mapcraft_geom::snap(v, g) } else { v }
        } else {
            let d = raw.dot(normal);
            normal * if snap { snap_travel(&paths, d, g) } else { d }
        };
        if (moved - offset).length() < 1e-9 {
            return;
        }
        match self.session.extrude(&faces, Push::Offset(moved), mode, &created, Some("drag-extrude")) {
            Ok(made) => {
                self.drag = Drag::Extrude { view, faces, normal, anchor, reference, start, paths, mode, created: made, offset: moved };
                self.mark_changed();
            }
            Err(e) => self.status = format!("Cannot extrude there: {e}"),
        }
    }

    /// Ends a face drag as one undo step, or none if it changed nothing; splits and stamps select what they made, inward splits add it.
    pub(crate) fn end_extrude(&mut self, drag: Drag) {
        let Drag::Extrude { normal, mode, created, offset, .. } = drag else { return };
        if created.is_empty() && (offset.length() < 1e-9 || matches!(mode, ExtrudeMode::Split | ExtrudeMode::Stamp)) {
            self.cancel_extrude();
            return;
        }
        self.session.end_coalesce();
        if !created.is_empty() {
            if mode == ExtrudeMode::Split && offset.dot(normal) < 0.0 {
                self.session.selection.extend(created);
            } else {
                self.session.selection = created.into_iter().collect();
            }
            self.mark_changed();
        }
    }

    /// Puts back what the face drag changed and removes the brushes it made.
    pub(crate) fn cancel_extrude(&mut self) {
        self.drag = Drag::None;
        if self.session.cancel_coalesce("drag-extrude") {
            self.mark_changed();
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

    /// 3D scale handles on both sides of each axis: axis, low side, screen start and end, color.
    fn scale_handles(&self, index: usize, rect: Rect) -> Vec<(usize, bool, Pos2, Pos2, Color32)> {
        let (Some(cam), Some(b)) = (self.panes.get(index).map(|p| p.camera), self.selection_bounds()) else { return Vec::new() };
        if cam.is_2d() || self.tool != Tool::Scale {
            return Vec::new();
        }
        let c = b.center();
        let len = cam.position.distance(c) * 0.12;
        let Some(start) = to_screen(&cam, rect, c) else { return Vec::new() };
        let mut out = Vec::new();
        for (i, (axis, color)) in AXES.iter().enumerate() {
            for low in [false, true] {
                let (dir, color) = if low { (-*axis, color.gamma_multiply(0.6)) } else { (*axis, *color) };
                if let Some(end) = to_screen(&cam, rect, c + dir * len) {
                    out.push((i, low, start, end, color));
                }
            }
        }
        out
    }

    pub(crate) fn begin_scale(&mut self, index: usize, rect: Rect, pos: Pos2) -> bool {
        let handle = self
            .scale_handles(index, rect)
            .into_iter()
            .filter(|(_, _, a, b, _)| segment_distance(pos, *a, *b) < REACH)
            .min_by(|x, y| x.3.distance(pos).total_cmp(&y.3.distance(pos)));
        let (Some((axis, low, ..)), Some(b), Some(cam)) = (handle, self.selection_bounds(), self.panes.get(index).map(|p| p.camera)) else { return false };
        let (origin, dir) = cam.ray(dvec(pos - rect.min), dvec(rect.size()));
        let Some(start) = closest_on_axis(b.center(), AXES[axis].0, origin, dir) else { return false };
        self.drag = Drag::Scale { view: index, axis, low, bounds: b, start, applied: b };
        true
    }

    pub(crate) fn update_scale(&mut self, index: usize, rect: Rect, pos: Pos2) {
        let Drag::Scale { view, axis, low, bounds, start, applied } = self.drag.clone() else { return };
        let Some(cam) = self.panes.get(index).map(|p| p.camera) else { return };
        let (origin, dir) = cam.ray(dvec(pos - rect.min), dvec(rect.size()));
        let Some(t) = closest_on_axis(bounds.center(), AXES[axis].0, origin, dir) else { return };
        let g = self.view_grid(index);
        let mut edge = if low { bounds.min[axis] } else { bounds.max[axis] } + (t - start);
        if self.prefs.get().view.snap_grid {
            edge = (edge / g).round() * g;
        }
        let mut target: Aabb = bounds;
        if low {
            target.min[axis] = edge.min(bounds.max[axis] - g.max(1.0));
        } else {
            target.max[axis] = edge.max(bounds.min[axis] + g.max(1.0));
        }
        if target != applied {
            if let Err(e) = self.session.transform_selection(&box_transform(&applied, &target), "Scale", Some("drag-scale")) {
                self.status = e.to_string();
            }
            self.drag = Drag::Scale { view, axis, low, bounds, start, applied: target };
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
        for (_, _, a, b, color) in self.scale_handles(index, rect) {
            painter.line_segment([a, b], Stroke::new(3.0, color));
            painter.rect_filled(Rect::from_center_size(b, Vec2::splat(9.0)), 1.0, color);
        }
    }
}
