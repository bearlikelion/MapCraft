//! The Path tool, editing path points and handles in every view, drawing paths, and the inspector
//! for paths and curve groups (sweep, spin and array along a path).

use std::collections::BTreeSet;

use egui::{Color32, Pos2, Rect, RichText, Stroke, Vec2};
use mapcraft_geom::DVec3;
use mapcraft_geom::curve::Measured;
use mapcraft_map::{CurveOp, NodeId, NodeKind, Path, PathPoint};
use mapcraft_render::{LineVertex, SELECTED_EDGE};

use crate::app::MapCraftApp;
use crate::chrome::TEXT_WEAK;
use crate::measure::units;
use crate::viewport::{Drag, Tool, closest_on_axis, dvec, segment_distance};

pub(crate) const PATH_COLOR: Color32 = Color32::from_rgb(235, 120, 235);

/// How close on screen, in points, the pointer must be to a path or one of its handles.
const REACH: f32 = 6.0;

/// Which part of a path point a drag holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Part {
    Point,
    In,
    Out,
}

/// A path point or handle being dragged.
#[derive(Clone, Debug)]
pub(crate) struct PathDrag {
    path: NodeId,
    index: usize,
    part: Part,
    /// Pull both handles out of the point, mirrored, instead of moving it.
    pull: bool,
    /// Move one handle without turning the other.
    free: bool,
    original: Path,
    /// Where the held part was, and in a 2D view the pointer's plane point, when the drag began.
    from: DVec3,
    start: DVec3,
}

fn plural(n: usize, what: &str) -> String {
    if n == 1 { format!("1 {what}") } else { format!("{n} {what}s") }
}

fn path_of(kind: &NodeKind) -> Option<&Path> {
    match kind {
        NodeKind::Path(p) => Some(p),
        _ => None,
    }
}

impl MapCraftApp {
    fn shows_handles(&self) -> bool {
        matches!(self.tool, Tool::Select | Tool::Move | Tool::Path)
    }

    /// The selected paths' points and handles: path, point index, part and where it is.
    fn path_handles(&self) -> Vec<(NodeId, usize, Part, DVec3)> {
        let mut out = Vec::new();
        for id in &self.session.selection {
            let Some(p) = self.session.map.get(*id).and_then(|n| path_of(&n.kind)) else { continue };
            for (i, q) in p.points.iter().enumerate() {
                out.push((*id, i, Part::Point, q.pos));
                for (part, at) in [(Part::In, q.handle_in), (Part::Out, q.handle_out)] {
                    if at.distance_squared(q.pos) > 1e-9 {
                        out.push((*id, i, part, at));
                    }
                }
            }
        }
        out
    }

    fn screen(&self, index: usize, rect: Rect, p: DVec3) -> Option<Pos2> {
        let cam = self.panes.get(index)?.camera;
        cam.project(p, dvec(rect.size())).map(|q| rect.min + Vec2::new(q.x as f32, q.y as f32))
    }

    /// The selected path's point or handle under the pointer; points win over handles on them.
    fn path_handle_at(&self, index: usize, rect: Rect, pos: Pos2) -> Option<(NodeId, usize, Part, DVec3)> {
        if !self.shows_handles() {
            return None;
        }
        self.path_handles()
            .into_iter()
            .filter_map(|h| {
                let d = self.screen(index, rect, h.3)?.distance(pos) - if h.2 == Part::Point { 0.5 } else { 0.0 };
                (d < REACH).then_some((d, h))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, h)| h)
    }

    /// The visible path whose line passes under the pointer, as a node of the real map.
    pub(crate) fn pick_path(&self, index: usize, rect: Rect, pos: Pos2) -> Option<NodeId> {
        let map = self.view_map();
        let visible = map.visible_nodes();
        let mut best: Option<(f32, NodeId)> = None;
        for (id, n) in map.nodes() {
            let (Some(p), true, false) = (path_of(&n.kind), visible.contains(&id), n.locked) else { continue };
            let pts: Vec<Option<Pos2>> = p.polyline(24).into_iter().map(|q| self.screen(index, rect, q)).collect();
            for w in pts.windows(2) {
                if let [Some(a), Some(b)] = w {
                    let d = segment_distance(pos, *a, *b);
                    if d < REACH && best.is_none_or(|(bd, _)| d < bd) {
                        best = Some((d, self.owner(id)));
                    }
                }
            }
        }
        best.map(|(_, id)| id)
    }

    /// The path the Path tool is drawing: the one selected path.
    fn drawing_path(&self) -> Option<(NodeId, Path)> {
        match self.session.selection.iter().collect::<Vec<_>>()[..] {
            [id] => Some((*id, path_of(&self.session.map.get(*id)?.kind)?.clone())),
            _ => None,
        }
    }

    /// Adds a point at `at` to the path being drawn, or starts a new path; returns the path and the point's index.
    fn add_path_point(&mut self, at: DVec3) -> Option<(NodeId, usize)> {
        self.session.end_coalesce();
        let result = match self.drawing_path() {
            Some((id, mut p)) if !p.closed => {
                p.points.push(PathPoint::corner(at));
                let index = p.points.len() - 1;
                self.session.set_kind(id, NodeKind::Path(p), "Add path point", Some("drag-path")).map(|()| (id, index))
            }
            _ => self.session.create_path(vec![PathPoint::corner(at)]).map(|id| (id, 0)),
        };
        self.mark_changed();
        result.map_err(|e| self.status = e.to_string()).ok()
    }

    /// The Path tool's click and selecting a path by clicking its line; true when it used the click.
    pub(crate) fn path_click(&mut self, index: usize, rect: Rect, pos: Pos2, additive: bool) -> bool {
        let handle = self.path_handle_at(index, rect, pos);
        if self.tool == Tool::Path {
            match (handle, self.drawing_path()) {
                (Some((id, 0, Part::Point, _)), Some((drawing, mut p))) if id == drawing && p.points.len() >= 3 && !p.closed => {
                    p.closed = true;
                    if let Err(e) = self.session.set_kind(id, NodeKind::Path(p), "Close path", None) {
                        self.status = e.to_string();
                    }
                    self.mark_changed();
                }
                (Some(_), _) => {}
                (None, _) => {
                    if let Some(at) = self.placement_at(index, rect, pos) {
                        self.add_path_point(at);
                    }
                }
            }
            return true;
        }
        if handle.is_some() {
            return true;
        }
        if !matches!(self.tool, Tool::Select | Tool::Move | Tool::Rotate | Tool::Scale | Tool::Face) {
            return false;
        }
        let Some(id) = self.pick_path(index, rect, pos) else { return false };
        if self.tool == Tool::Face {
            self.session.selection = BTreeSet::from([id]);
        } else {
            self.session.click(Some(id), additive);
        }
        self.mark_changed();
        true
    }

    /// Starts dragging a selected path's point or handle, a new point's handles in the Path tool,
    /// or a path line in a 2D view to move it; true when it took the drag.
    pub(crate) fn begin_path_drag(&mut self, index: usize, rect: Rect, pos: Pos2, mods: egui::Modifiers) -> bool {
        let Some(cam) = self.panes.get(index).map(|p| p.camera) else { return false };
        let press = dvec(pos - rect.min);
        let start = cam.plane_point(press, dvec(rect.size()));
        let grab = |path: NodeId, index: usize, part: Part, pull: bool, from: DVec3, original: Path| PathDrag {
            path,
            index,
            part,
            pull,
            free: mods.alt,
            original,
            from,
            start,
        };
        if let Some((id, i, part, at)) = self.path_handle_at(index, rect, pos) {
            let Some(p) = self.session.map.get(id).and_then(|n| path_of(&n.kind)).cloned() else { return false };
            self.session.end_coalesce();
            let pull = part == Part::Point && mods.alt;
            self.drag = Drag::Path { view: index, grab: Box::new(grab(id, i, part, pull, at, p)) };
            return true;
        }
        if self.tool == Tool::Path {
            let Some(at) = self.placement_at(index, rect, pos) else { return true };
            if let Some((id, i)) = self.add_path_point(at)
                && let Some(p) = self.session.map.get(id).and_then(|n| path_of(&n.kind)).cloned()
            {
                self.drag = Drag::Path { view: index, grab: Box::new(grab(id, i, Part::Point, true, at, p)) };
            }
            return true;
        }
        if !cam.is_2d() || !matches!(self.tool, Tool::Select | Tool::Move) {
            return false;
        }
        let Some(id) = self.pick_path(index, rect, pos) else { return false };
        if !self.session.selection.contains(&id) {
            self.session.click(Some(id), false);
            self.mark_changed();
        }
        let points: Vec<DVec3> = self.session.map.get(id).and_then(|n| path_of(&n.kind)).map(|p| p.points.iter().map(|q| q.pos).collect()).unwrap_or_default();
        let nearest = points.into_iter().min_by(|a, b| {
            let d = |p: &DVec3| self.screen(index, rect, *p).map_or(f32::MAX, |s| s.distance(pos));
            d(a).total_cmp(&d(b))
        });
        let (_, _, d) = cam.view.axes();
        let anchor = nearest.unwrap_or(start);
        self.drag = Drag::Move { view: index, start, anchor: anchor - d * (anchor - start).dot(d), applied: DVec3::ZERO };
        true
    }

    /// Moves the held point or handle to the pointer, snapped like other drags.
    pub(crate) fn update_path_drag(&mut self, index: usize, rect: Rect, pos: Pos2, shift: bool) {
        let Drag::Path { grab, .. } = &self.drag else { return };
        let g = grab.as_ref().clone();
        let Some(cam) = self.panes.get(index).map(|p| p.camera) else { return };
        let size = dvec(rect.size());
        let target = if cam.is_2d() {
            let cursor = cam.plane_point(dvec(pos - rect.min), size);
            let (snapped, caught) = self.snap_point(index, rect, g.from + (cursor - g.start), &BTreeSet::from([g.path]), &[]);
            self.snap_mark = Some((index, snapped, caught));
            snapped
        } else {
            let (origin, dir) = cam.ray(dvec(pos - rect.min), size);
            let step = self.session.grid;
            let snap = |x: f64| if self.prefs.get().view.snap_grid && step > 0.0 { (x / step).round() * step } else { x };
            let t = if shift {
                closest_on_axis(g.from, DVec3::Z, origin, dir)
            } else {
                Some((g.from.z - origin.z) / dir.z).filter(|t| dir.z.abs() > 1e-9 && *t > 0.0)
            };
            let Some(t) = t else { return };
            let p = if shift {
                DVec3::new(g.from.x, g.from.y, snap(g.from.z + t))
            } else {
                (origin + dir * t).with_x(snap((origin + dir * t).x)).with_y(snap((origin + dir * t).y))
            };
            self.snap_mark = Some((index, p, false));
            p
        };
        let mut p = g.original.clone();
        let Some(q) = p.points.get_mut(g.index) else { return };
        let other = |handle: DVec3, pos: DVec3, toward: DVec3| {
            let len = handle.distance(pos);
            let dir = (pos - toward).normalize_or_zero();
            if len > 1e-9 && !g.free {
                pos + dir * len
            } else if g.free {
                handle
            } else {
                pos * 2.0 - toward
            }
        };
        match (g.part, g.pull) {
            (Part::Point, true) => {
                (q.handle_out, q.handle_in) = (target, q.pos * 2.0 - target);
            }
            (Part::Point, false) => {
                let d = target - q.pos;
                (q.pos, q.handle_in, q.handle_out) = (q.pos + d, q.handle_in + d, q.handle_out + d);
            }
            (Part::Out, _) => (q.handle_out, q.handle_in) = (target, other(q.handle_in, q.pos, target)),
            (Part::In, _) => (q.handle_in, q.handle_out) = (target, other(q.handle_out, q.pos, target)),
        }
        if let Err(e) = self.session.set_kind(g.path, NodeKind::Path(p), "Edit path", Some("drag-path")) {
            self.status = e.to_string();
        }
        self.mark_changed();
    }

    /// Lines for every visible path, the selected ones' handles, and the axis of a selected spin.
    pub(crate) fn path_lines(&self, out: &mut Vec<LineVertex>) {
        let map = self.view_map();
        let visible = map.visible_nodes();
        let color = egui::Rgba::from(PATH_COLOR).to_array();
        for (id, n) in map.nodes() {
            let (Some(p), true) = (path_of(&n.kind), visible.contains(&id)) else { continue };
            let c = if self.session.selection.contains(&self.owner(id)) { SELECTED_EDGE } else { color };
            for w in p.polyline(24).windows(2) {
                out.extend([LineVertex::new(w[0], c), LineVertex::new(w[1], c)]);
            }
        }
        if self.shows_handles() {
            let grey = [0.6, 0.6, 0.6, 1.0];
            for (id, i, part, at) in self.path_handles() {
                let pos = self.session.map.get(id).and_then(|n| path_of(&n.kind)).and_then(|p| p.points.get(i)).map(|q| q.pos);
                if let (Some(pos), true) = (pos, part != Part::Point) {
                    out.extend([LineVertex::new(pos, grey), LineVertex::new(at, grey)]);
                }
            }
        }
        let mirror = egui::Rgba::from(crate::chrome::MIRROR).to_array();
        for id in &self.session.selection {
            if let Some(NodeKind::Group(g)) = self.session.map.get(*id).map(|n| &n.kind)
                && let Some(CurveOp::Spin { axis, pivot, .. }) = g.curve.as_deref()
            {
                let a = axis.normalize_or(DVec3::Z) * 4096.0;
                out.extend([LineVertex::new(*pivot - a, mirror), LineVertex::new(*pivot + a, mirror)]);
            }
        }
    }

    /// Squares on the selected paths' points (the first one green) and dots on their handles.
    pub(crate) fn draw_path_handles(&self, ui: &egui::Ui, index: usize, rect: Rect) {
        if !self.shows_handles() {
            return;
        }
        let painter = ui.painter_at(rect);
        for (_, i, part, at) in self.path_handles() {
            let Some(p) = self.screen(index, rect, at).filter(|p| rect.contains(*p)) else { continue };
            if part == Part::Point {
                let fill = if i == 0 { Color32::from_rgb(90, 220, 90) } else { Color32::WHITE };
                let r = Rect::from_center_size(p, Vec2::splat(7.0));
                painter.rect_filled(r, 0.0, fill);
                painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::BLACK), egui::StrokeKind::Inside);
            } else {
                painter.circle_filled(p, 3.5, PATH_COLOR);
                painter.circle_stroke(p, 3.5, Stroke::new(1.0, Color32::BLACK));
            }
        }
    }

    /// Runs Sweep or Spin, spinning about the point under the pointer in a 2D view, then shows
    /// the result's settings.
    pub(crate) fn run_curve(&mut self, id: &str) {
        let hover = self.hovered_pane.and_then(|i| self.panes.get(i)).filter(|p| p.camera.is_2d()).and(self.cursor_world);
        let result = match id {
            "tools.spin" => {
                let pivot = hover.map(|p| mapcraft_geom::snap(p, self.session.grid)).or_else(|| self.session.selection_center()).unwrap_or_default();
                self.session.spin_faces(pivot)
            }
            _ => self.session.sweep_faces(),
        };
        match result {
            Ok(_) => {
                self.run_action("tool.select");
                self.status = "Its settings are in the inspector; edit the path or faces and it follows".into();
            }
            Err(e) => self.status = e.to_string(),
        }
        self.mark_changed();
    }

    pub(crate) fn path_inspector(&mut self, ui: &mut egui::Ui, id: NodeId, p: Path) {
        ui.heading("Path");
        let len = Measured::new(&p.spans()).length();
        ui.label(format!("{} points, {} units long", p.points.len(), units(len)));
        let mut closed = p.closed;
        if ui.checkbox(&mut closed, "Closed").on_hover_text("Join the last point back to the first").changed() {
            if let Err(e) = self.session.set_kind(id, NodeKind::Path(Path { closed, ..p }), "Close path", None) {
                self.status = e.to_string();
            }
            self.mark_changed();
        }
        let hint = format!(
            "Drag points and handles in any view; Alt+drag a point pulls out its handles, Alt+drag a handle turns it alone. Pick faces with the Face tool and Sweep ({}), or select objects with the path and Array Along Path ({}).",
            self.chord("tools.sweep"),
            self.chord("tools.array_path")
        );
        ui.label(RichText::new(hint).color(TEXT_WEAK).small());
    }

    /// Blender's adjust last operation for a curve group: changing a setting rebuilds it in place.
    pub(crate) fn curve_inspector(&mut self, ui: &mut egui::Ui, id: NodeId, op: CurveOp) {
        let mut edited = op.clone();
        let label = |p: &NodeId| self.session.map.get(*p).map_or("a deleted path".to_string(), |n| format!("{} {}", n.kind.label(), p.0));
        egui::Grid::new(("curve", id.0)).num_columns(2).show(ui, |ui| match &mut edited {
            CurveOp::Sweep { path, faces, segments, twist, scale, .. } => {
                ui.label(RichText::new(format!("Sweeps {} along {}", plural(faces.len(), "face"), label(path))).strong());
                ui.end_row();
                ui.label("Segments");
                ui.add(egui::DragValue::new(segments).range(1..=64)).on_hover_text("Brushes per curved span; straight spans take one");
                ui.end_row();
                ui.label("Twist");
                ui.add(egui::DragValue::new(twist).speed(1.0).suffix("°"))
                    .on_hover_text("Turns the cross-section by the end; a closed path turns in whole turns");
                ui.end_row();
                ui.label("End scale");
                ui.add(egui::DragValue::new(scale).range(0.01..=100.0).speed(0.01)).on_hover_text("Size of the cross-section at the end of an open path");
                ui.end_row();
            }
            CurveOp::Spin { faces, axis, pivot, angle, steps } => {
                ui.label(RichText::new(format!("Spins {}", plural(faces.len(), "face"))).strong());
                ui.end_row();
                ui.label("Angle");
                ui.add(egui::DragValue::new(angle).range(-360.0..=360.0).speed(1.0).suffix("°"));
                ui.end_row();
                ui.label("Steps");
                ui.add(egui::DragValue::new(steps).range(1..=256));
                ui.end_row();
                ui.label("Axis");
                let mut pick = (0..3).find(|i| axis.abs_diff_eq(DVec3::AXES[*i], 1e-9)).unwrap_or(3);
                if Self::segmented(ui, &mut pick, &[(0, "X", "Turn about X"), (1, "Y", "Turn about Y"), (2, "Z", "Turn about Z")]) {
                    *axis = DVec3::AXES.get(pick).copied().unwrap_or(DVec3::Z);
                }
                ui.end_row();
                ui.label("Pivot");
                ui.horizontal(|ui| {
                    for v in pivot.as_mut() {
                        ui.add(egui::DragValue::new(v).speed(1.0));
                    }
                });
                ui.end_row();
            }
            CurveOp::Array { path, nodes, count, spacing, yaw_only, .. } => {
                ui.label(RichText::new(format!("Copies {} along {}", plural(nodes.len(), "object"), label(path))).strong());
                ui.end_row();
                ui.label("Count");
                ui.add_enabled(*spacing <= 0.0, egui::DragValue::new(count).range(1..=256));
                ui.end_row();
                ui.label("Spacing");
                ui.add(egui::DragValue::new(spacing).range(0.0..=65536.0).speed(1.0)).on_hover_text("Units between copies; 0 spreads Count evenly");
                ui.end_row();
                ui.label("Turn");
                Self::segmented(ui, yaw_only, &[(false, "Follow", "Turn with the path"), (true, "Yaw only", "Turn about Z only, staying upright")]);
                ui.end_row();
            }
        });
        let gone = op.sources().iter().filter(|s| self.session.map.get(**s).is_none()).count();
        if gone > 0 {
            ui.colored_label(Color32::LIGHT_RED, format!("{gone} of its sources are gone, so it no longer follows them."));
        }
        if edited != op {
            if let Err(e) = self.session.set_curve(id, edited, Some(&format!("curve:{}", id.0))) {
                self.status = e.to_string();
            }
            self.mark_changed();
        }
        if ui.button("Make real").on_hover_text("Keep the brushes and stop following the path and faces").clicked() {
            self.run_action("tools.curve_make_real");
        }
    }
}
