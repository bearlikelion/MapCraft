//! The Vertex tool's component selection (Blender's vertex, edge and face modes), Merge and Bevel.

use egui::{Align2, Color32, Pos2, Rect, RichText, Stroke, Vec2};
use mapcraft_geom::DVec3;
use mapcraft_map::{Bevel, Brush, NodeId};

use crate::app::MapCraftApp;
use crate::chrome::{ACCENT, TEXT_WEAK};
use crate::measure::{label, units};
use crate::viewport::{Drag, Tool, dvec};

/// Which brush components the Vertex tool shows and selects.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum HandleKind {
    #[default]
    Vertex,
    Edge,
    Face,
}

/// A corner, edge or face of a selected brush: where its handle sits and the corners it holds.
#[derive(Clone, Debug)]
pub(crate) struct Component {
    pub brush: NodeId,
    pub kind: HandleKind,
    pub at: DVec3,
    pub corners: Vec<DVec3>,
}

impl Component {
    fn same(&self, o: &Component) -> bool {
        self.brush == o.brush
            && self.kind == o.kind
            && self.corners.len() == o.corners.len()
            && self.corners.iter().all(|p| o.corners.iter().any(|q| q.distance_squared(*p) < 1e-4))
    }
}

/// Each brush as it was before a bevel, with the edges to bevel.
type BevelJob = Vec<(NodeId, (Brush, Vec<(DVec3, DVec3)>))>;

/// A bevel being sized with the pointer (Blender's modal bevel).
pub(crate) struct BevelModal {
    view: usize,
    center: DVec3,
    job: BevelJob,
    want: Bevel,
    tried: Option<Bevel>,
    /// The settings now applied to the map, if any worked yet.
    shown: Option<Bevel>,
}

impl BevelModal {
    pub(crate) fn hint(&self) -> String {
        let b = self.shown.unwrap_or(self.want);
        format!(
            "Bevel width {}, {} segments; move to size, scroll for segments, click or Enter applies, Esc or right click cancels",
            units(b.width),
            b.segments
        )
    }
}

/// The bevel applied last, which the Adjust Last Bevel window runs again with new settings.
#[derive(Clone)]
pub(crate) struct BevelRun {
    view: usize,
    job: BevelJob,
    bevel: Bevel,
    /// The brushes it made; the window closes once anything changes them.
    results: Vec<(NodeId, Brush)>,
}

#[derive(Default)]
pub(crate) struct VertexState {
    pub mode: HandleKind,
    pub selected: Vec<Component>,
    pub bevel: Option<BevelModal>,
    pub last: Option<BevelRun>,
    /// Width, segments and profile the next bevel starts from.
    pub settings: Bevel,
}

const SELECTED: Color32 = ACCENT;

impl MapCraftApp {
    /// Corners, edges and faces of the selected brushes, each brush's own.
    pub(crate) fn components(&self) -> Vec<Component> {
        let mut out: Vec<Component> = Vec::new();
        for brush in self.session.selected_brushes() {
            let Some(Ok(solid)) = self.session.map.get(brush).and_then(|n| n.kind.brush()).map(|b| b.solid()) else { continue };
            let start = out.len();
            let mut add = |kind, at: DVec3, corners: Vec<DVec3>| {
                if !out.iter().skip(start).any(|c| c.kind == kind && c.at.distance_squared(at) < 1e-6) {
                    out.push(Component { brush, kind, at, corners });
                }
            };
            for w in solid.faces.iter().flatten() {
                for (a, b) in w.0.iter().zip(w.0.iter().cycle().skip(1)) {
                    add(HandleKind::Vertex, *a, vec![*a]);
                    add(HandleKind::Edge, (*a + *b) / 2.0, vec![*a, *b]);
                }
                add(HandleKind::Face, w.center(), w.0.clone());
            }
        }
        out
    }

    /// Handles of the current mode under the pointer: all stacked there in a 2D view, the nearest
    /// in the 3D view along with the same corner of other brushes.
    fn components_at(&self, all: &[Component], index: usize, rect: Rect, pos: Pos2) -> Vec<Component> {
        let Some(cam) = self.panes.get(index).map(|p| p.camera) else { return Vec::new() };
        let press = dvec(pos - rect.min);
        let mut hits: Vec<&Component> =
            all.iter().filter(|c| c.kind == self.vertex.mode && cam.project(c.at, dvec(rect.size())).is_some_and(|p| (p - press).length() < 6.0)).collect();
        if !cam.is_2d()
            && let Some(near) = hits.iter().map(|c| c.at).min_by(|a, b| cam.position.distance(*a).total_cmp(&cam.position.distance(*b)))
        {
            hits.retain(|c| c.at.distance_squared(near) < 1e-4);
        }
        hits.into_iter().cloned().collect()
    }

    /// The selected corners, by brush.
    fn selected_corners(&self) -> Vec<(NodeId, Vec<DVec3>)> {
        let mut out: Vec<(NodeId, Vec<DVec3>)> = Vec::new();
        for c in &self.vertex.selected {
            let i = out.iter().position(|(b, _)| *b == c.brush).unwrap_or_else(|| {
                out.push((c.brush, Vec::new()));
                out.len() - 1
            });
            if let Some((_, corners)) = out.get_mut(i) {
                for p in &c.corners {
                    if !corners.iter().any(|q| q.distance_squared(*p) < 1e-4) {
                        corners.push(*p);
                    }
                }
            }
        }
        out
    }

    /// Selects a component under a click; false when there is none, so the click picks brushes.
    pub(crate) fn vertex_click(&mut self, index: usize, rect: Rect, pos: Pos2, additive: bool) -> bool {
        let hits = self.components_at(&self.components(), index, rect, pos);
        let sel = &mut self.vertex.selected;
        if !additive {
            sel.clear();
        }
        if hits.is_empty() {
            return false;
        }
        if additive && hits.iter().all(|h| sel.iter().any(|s| s.same(h))) {
            sel.retain(|s| !hits.iter().any(|h| h.same(s)));
        } else {
            for h in hits {
                if !sel.iter().any(|s| s.same(&h)) {
                    sel.push(h);
                }
            }
        }
        true
    }

    /// Starts dragging the selection from a handle (picking it first if it was not selected), or a
    /// selection box from empty space.
    pub(crate) fn begin_vertex_drag(&mut self, index: usize, rect: Rect, pos: Pos2, mods: egui::Modifiers) {
        let Some(cam) = self.panes.get(index).map(|p| p.camera) else { return };
        let hits = self.components_at(&self.components(), index, rect, pos);
        let Some(anchor) = hits.first().map(|c| c.at) else {
            self.drag = Drag::ComponentBox { view: index, start: pos, add: mods.shift, remove: mods.command };
            return;
        };
        if !hits.iter().any(|h| self.vertex.selected.iter().any(|s| s.same(h))) {
            if !(mods.shift || mods.command) {
                self.vertex.selected.clear();
            }
            self.vertex.selected.extend(hits);
        }
        let originals = self.selected_corners();
        if cam.is_2d() {
            let start = cam.plane_point(dvec(pos - rect.min), dvec(rect.size()));
            let (_, _, d) = cam.view.axes();
            let anchor = anchor - d * (anchor - start).dot(d);
            self.drag = Drag::Vertex { view: index, originals, start, anchor, applied: DVec3::ZERO };
        } else {
            self.drag = Drag::Vertex3d { view: index, originals, anchor, applied: DVec3::ZERO };
        }
    }

    /// Moves the dragged corners from `applied` to `total` off where they began; false when refused.
    pub(crate) fn drag_corners(&mut self, originals: &[(NodeId, Vec<DVec3>)], applied: DVec3, total: DVec3) -> bool {
        self.shift_corners(originals, applied, total, "drag-vertex")
    }

    /// Moves the picked corners, edges or faces by `d`, as the arrow keys do.
    pub(crate) fn nudge_components(&mut self, d: DVec3) {
        let corners = self.selected_corners();
        self.shift_corners(&corners, DVec3::ZERO, d, "nudge-vertex");
    }

    fn shift_corners(&mut self, originals: &[(NodeId, Vec<DVec3>)], applied: DVec3, total: DVec3, coalesce: &str) -> bool {
        let moves: Vec<(NodeId, Vec<(DVec3, DVec3)>)> =
            originals.iter().map(|(id, corners)| (*id, corners.iter().map(|o| (*o + applied, *o + total)).collect())).collect();
        self.mark_changed();
        match self.session.move_corners(&moves, Some(coalesce)) {
            Ok(()) => {
                for c in &mut self.vertex.selected {
                    c.at += total - applied;
                    c.corners.iter_mut().for_each(|p| *p += total - applied);
                }
                true
            }
            Err(e) => {
                self.status = format!("Cannot move there: {e}");
                false
            }
        }
    }

    /// Selects the handles of the current mode inside the box from `a` to `b`.
    pub(crate) fn box_select_components(&mut self, index: usize, rect: Rect, a: Pos2, b: Pos2, add: bool, remove: bool) {
        let Some(cam) = self.panes.get(index).map(|p| p.camera) else { return };
        let area = Rect::from_two_pos(a, b);
        let inside: Vec<Component> = self
            .components()
            .into_iter()
            .filter(|c| {
                c.kind == self.vertex.mode && cam.project(c.at, dvec(rect.size())).is_some_and(|p| area.contains(rect.min + Vec2::new(p.x as f32, p.y as f32)))
            })
            .collect();
        let sel = &mut self.vertex.selected;
        if remove {
            sel.retain(|s| !inside.iter().any(|c| c.same(s)));
            return;
        }
        if !add {
            sel.clear();
        }
        for c in inside {
            if !sel.iter().any(|s| s.same(&c)) {
                sel.push(c);
            }
        }
    }

    /// Switches mode, keeping the components whose corners were all selected (Blender's rule).
    fn set_component_mode(&mut self, mode: HandleKind) {
        let corners = self.selected_corners();
        let held = |c: &Component| corners.iter().any(|(b, set)| *b == c.brush && c.corners.iter().all(|p| set.iter().any(|q| q.distance_squared(*p) < 1e-4)));
        self.vertex.selected = self.components().into_iter().filter(|c| c.kind == mode && held(c)).collect();
        self.vertex.mode = mode;
    }

    pub(crate) fn vertex_action(&mut self, id: &str) {
        if self.tool != Tool::Vertex {
            self.run_action("tool.vertex");
        }
        match id {
            "vertex.mode_vertex" => self.set_component_mode(HandleKind::Vertex),
            "vertex.mode_edge" => self.set_component_mode(HandleKind::Edge),
            "vertex.mode_face" => self.set_component_mode(HandleKind::Face),
            "vertex.merge" => self.merge_selected(),
            "vertex.bevel" => self.begin_bevel(),
            _ => {}
        }
    }

    fn merge_selected(&mut self) {
        let corners: Vec<(NodeId, Vec<DVec3>)> = self.selected_corners().into_iter().filter(|(_, c)| c.len() >= 2).collect();
        if corners.is_empty() {
            self.status = "Select two or more corners of a brush to merge".into();
            return;
        }
        match self.session.merge_corners(&corners) {
            Ok(()) => {
                self.vertex.mode = HandleKind::Vertex;
                self.vertex.selected = corners
                    .iter()
                    .map(|(brush, c)| {
                        let at = c.iter().sum::<DVec3>() / c.len() as f64;
                        Component { brush: *brush, kind: HandleKind::Vertex, at, corners: vec![at] }
                    })
                    .collect();
            }
            Err(e) => self.status = format!("Cannot merge: {e}"),
        }
        self.mark_changed();
    }

    /// The selected edges of each brush; in vertex and face mode, the edges between selected corners.
    fn bevel_job(&self) -> BevelJob {
        let all = self.components();
        let corners = self.selected_corners();
        let mut job: BevelJob = Vec::new();
        for (brush, set) in corners {
            let Some(b) = self.session.map.get(brush).and_then(|n| n.kind.brush()) else { continue };
            let held = |p: &DVec3| set.iter().any(|q| q.distance_squared(*p) < 1e-4);
            let edges: Vec<(DVec3, DVec3)> = all
                .iter()
                .filter(|c| c.brush == brush && c.kind == HandleKind::Edge)
                .filter(|c| match self.vertex.mode {
                    HandleKind::Edge => self.vertex.selected.iter().any(|s| s.same(c)),
                    _ => c.corners.iter().all(held),
                })
                .filter_map(|c| Some((*c.corners.first()?, *c.corners.get(1)?)))
                .collect();
            if !edges.is_empty() {
                job.push((brush, (b.clone(), edges)));
            }
        }
        job
    }

    fn begin_bevel(&mut self) {
        let job = self.bevel_job();
        let mids: Vec<DVec3> = job.iter().flat_map(|(_, (_, e))| e.iter().map(|(a, b)| (*a + *b) / 2.0)).collect();
        if mids.is_empty() {
            self.status = format!("Select edges to bevel first; {} switches to edge select", self.chord("vertex.mode_edge"));
            return;
        }
        let is_view = |i: &usize| self.panes.get(*i).is_some_and(|p| p.uv.is_none());
        let view = self.hovered_pane.filter(is_view).or_else(|| (0..self.panes.len()).find(is_view)).unwrap_or(0);
        self.session.end_coalesce();
        let center = mids.iter().sum::<DVec3>() / mids.len() as f64;
        let modal = BevelModal { view, center, job, want: self.vertex.settings, tried: None, shown: None };
        self.status = modal.hint();
        self.vertex.bevel = Some(modal);
    }

    /// Drives a running bevel from the pointer and keys; every other shortcut waits for it.
    pub(crate) fn bevel_modal_input(&mut self, ctx: &egui::Context) {
        let Some(mut m) = self.vertex.bevel.take() else { return };
        let Some((cam, rect)) = self.panes.get(m.view).map(|p| (p.camera, p.rect)) else { return };
        use egui::{Key, PointerButton};
        let (pointer, scroll, confirm, cancel) = ctx.input(|i| {
            (
                i.pointer.latest_pos(),
                i.events.iter().map(|e| if let egui::Event::MouseWheel { delta, .. } = e { delta.y } else { 0.0 }).sum::<f32>(),
                i.key_pressed(Key::Enter) || i.pointer.primary_released(),
                i.key_pressed(Key::Escape) || i.pointer.button_released(PointerButton::Secondary),
            )
        });
        if let Some(p) = pointer {
            // The width is how far the pointer is from the middle of the edges.
            let (origin, dir) = cam.ray(dvec(p - rect.min), dvec(rect.size()));
            let to = m.center - origin;
            let mut w = (to - dir * to.dot(dir)).length();
            let g = self.session.grid;
            if self.prefs.get().view.snap_grid && g > 0.0 {
                w = (w / g).round() * g;
            }
            if w > 0.0 {
                m.want.width = w;
            }
        }
        if scroll != 0.0 {
            m.want.segments = if scroll > 0.0 { m.want.segments.saturating_add(1).min(64) } else { m.want.segments.saturating_sub(1).max(1) };
        }
        if m.tried != Some(m.want) {
            m.tried = Some(m.want);
            match self.session.bevel_edges(&m.job, &m.want, Some("bevel")) {
                Ok(()) => {
                    m.shown = Some(m.want);
                    self.status = m.hint();
                }
                Err(e) => self.status = e.to_string(),
            }
            self.mark_changed();
        }
        if confirm {
            self.session.end_coalesce();
            if let Some(bevel) = m.shown {
                let results = m.job.iter().filter_map(|(id, _)| Some((*id, self.session.map.get(*id)?.kind.brush()?.clone()))).collect();
                self.vertex.settings = bevel;
                self.vertex.last = Some(BevelRun { view: m.view, job: m.job, bevel, results });
                self.status = "Beveled; Adjust Last Bevel changes it until the next edit".into();
            }
        } else if cancel {
            if m.shown.is_some() {
                self.session.undo();
                self.mark_changed();
            }
            self.status = "Bevel cancelled".into();
        } else {
            self.vertex.bevel = Some(m);
        }
    }

    /// Blender's Adjust Last Operation panel for the last bevel: changing a setting re-runs it in
    /// place of the old one, as one undo step.
    pub(crate) fn bevel_window(&mut self, ctx: &egui::Context) {
        let Some(mut run) = self.vertex.last.take() else { return };
        let unchanged =
            self.session.undo_label() == Some("Bevel") && run.results.iter().all(|(id, b)| self.session.map.get(*id).and_then(|n| n.kind.brush()) == Some(b));
        if !unchanged || self.vertex.bevel.is_some() {
            return;
        }
        let corner = self.panes.get(self.maximized.unwrap_or(run.view)).map_or(Pos2::new(320.0, 900.0), |p| p.rect.left_bottom() + Vec2::new(8.0, -8.0));
        let mut b = run.bevel;
        egui::Window::new("Adjust Last Bevel").pivot(Align2::LEFT_BOTTOM).fixed_pos(corner).resizable(false).show(ctx, |ui| {
            ui.add(egui::DragValue::new(&mut b.width).range(0.125..=4096.0).speed(0.5).prefix("Width "));
            ui.add(egui::DragValue::new(&mut b.segments).range(1..=64).prefix("Segments "));
            ui.add(egui::Slider::new(&mut b.profile, 0.0..=1.0).text("Profile"));
            ui.label(RichText::new("0.5 is round, 0.25 or less flat, 1 boxy").color(TEXT_WEAK).small());
        });
        if b != run.bevel {
            self.session.undo();
            match self.session.bevel_edges(&run.job, &b, None) {
                Ok(()) => {
                    run.bevel = b;
                    run.results = run.job.iter().filter_map(|(id, _)| Some((*id, self.session.map.get(*id)?.kind.brush()?.clone()))).collect();
                    self.vertex.settings = b;
                }
                Err(e) => {
                    self.session.redo();
                    self.status = e.to_string();
                }
            }
            self.mark_changed();
        }
        self.vertex.last = Some(run);
    }

    /// Mode buttons, Merge and Bevel on the top bar.
    pub(crate) fn vertex_options(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        let tip = |what: &str, id: &str| format!("{what}  ({})", self.chord(id));
        let tips = [
            tip("Select corners", "vertex.mode_vertex"),
            tip("Select edges", "vertex.mode_edge"),
            tip("Select faces", "vertex.mode_face"),
            tip("Collapse the selected corners of each brush to their center", "vertex.merge"),
            tip("Round off the selected edges", "vertex.bevel"),
        ];
        let modes =
            [(HandleKind::Vertex, "Vertex", tips[0].as_str()), (HandleKind::Edge, "Edge", tips[1].as_str()), (HandleKind::Face, "Face", tips[2].as_str())];
        let mut mode = self.vertex.mode;
        if Self::segmented(ui, &mut mode, &modes) {
            self.set_component_mode(mode);
        }
        if ui.button("Merge").on_hover_text(&tips[3]).clicked() {
            self.run_action("vertex.merge");
        }
        if ui.button("Bevel").on_hover_text(&tips[4]).clicked() {
            self.run_action("vertex.bevel");
        }
    }

    /// The Vertex tool's handles in a view, selected ones highlighted, plus a selection box and
    /// the size of a bevel being made.
    pub(crate) fn draw_components(&mut self, ui: &egui::Ui, index: usize, rect: Rect) {
        let Some(cam) = self.panes.get(index).map(|p| p.camera) else { return };
        let all = self.components();
        // Selections follow their brushes through rebuilds and drop what no longer exists.
        let mut kept = Vec::new();
        for s in &self.vertex.selected {
            if let Some(c) = all.iter().find(|c| c.same(s)) {
                kept.push(c.clone());
            }
        }
        self.vertex.selected = kept;
        let painter = ui.painter_at(rect);
        let screen = |p: DVec3| cam.project(p, dvec(rect.size())).map(|q| rect.min + Vec2::new(q.x as f32, q.y as f32));
        for c in self.vertex.selected.iter().filter(|c| c.kind != HandleKind::Vertex) {
            let Some(pts) = c.corners.iter().map(|p| screen(*p)).collect::<Option<Vec<Pos2>>>() else { continue };
            if c.kind == HandleKind::Face {
                painter.add(egui::Shape::convex_polygon(pts, SELECTED.gamma_multiply(0.3), Stroke::new(2.0, SELECTED)));
            } else if let [a, b] = pts[..] {
                painter.line_segment([a, b], Stroke::new(3.0, SELECTED));
            }
        }
        for c in all.iter().rev().filter(|c| c.kind == self.vertex.mode) {
            let Some(p) = screen(c.at).filter(|p| rect.contains(*p)) else { continue };
            let on = self.vertex.selected.iter().any(|s| s.same(c));
            match (c.kind, on) {
                (HandleKind::Vertex, false) => {
                    painter.rect_filled(Rect::from_center_size(p, Vec2::splat(6.0)), 0.0, Color32::from_rgb(255, 220, 60));
                }
                (HandleKind::Edge, false) => {
                    painter.rect_filled(Rect::from_center_size(p, Vec2::splat(5.0)), 0.0, Color32::from_rgb(80, 220, 240));
                }
                (HandleKind::Face, false) => {
                    painter.circle_filled(p, 3.0, Color32::from_rgb(120, 240, 120));
                }
                (HandleKind::Face, true) => {
                    painter.circle_filled(p, 4.0, SELECTED);
                }
                (_, true) => {
                    let r = Rect::from_center_size(p, Vec2::splat(8.0));
                    painter.rect_filled(r, 0.0, SELECTED);
                    painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::WHITE), egui::StrokeKind::Inside);
                }
            }
        }
        if let Drag::ComponentBox { view, start, .. } = self.drag
            && view == index
            && let Some(p) = ui.ctx().pointer_latest_pos()
        {
            painter.rect_stroke(Rect::from_two_pos(start, p), 0.0, Stroke::new(1.0, Color32::YELLOW), egui::StrokeKind::Inside);
        }
        if let Some(m) = &self.vertex.bevel
            && let Some(p) = screen(m.center)
        {
            let b = m.shown.unwrap_or(m.want);
            label(&painter, p - Vec2::new(0.0, 14.0), Align2::CENTER_BOTTOM, format!("Width {}  Segments {}", units(b.width), b.segments));
        }
    }
}
