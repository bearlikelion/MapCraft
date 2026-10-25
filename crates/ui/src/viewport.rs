use std::collections::BTreeSet;

use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};
use mapcraft_engine::pick_target;
use mapcraft_geom::{Aabb, DMat4, DVec2, DVec3, snap};
use mapcraft_map::{FaceId, Map, NodeId, NodeKind, ROOT};
use mapcraft_render::{LineVertex, SELECTED_EDGE, box_lines};

use crate::app::MapCraftApp;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    #[default]
    Select,
    Block,
    Entity,
    Clip,
    /// Pick and texture individual faces.
    Face,
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
    Move {
        view: usize,
        start: DVec3,
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
    Vertex {
        view: usize,
        originals: Vec<DVec3>,
        start: DVec3,
        applied: DVec3,
    },
}

const PREVIEW: [f32; 4] = [1.0, 1.0, 0.2, 1.0];

fn dvec(v: Vec2) -> DVec2 {
    DVec2::new(f64::from(v.x), f64::from(v.y))
}

/// Nodes whose node and ancestors are all visible.
pub fn visible_nodes(map: &Map) -> BTreeSet<NodeId> {
    let mut out = BTreeSet::new();
    let mut stack = vec![ROOT];
    while let Some(id) = stack.pop() {
        let Some(n) = map.get(id) else { continue };
        if n.hidden || !out.insert(id) {
            continue;
        }
        stack.extend(n.children.iter().copied());
    }
    out
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

    /// Nearest visible brush or point entity on the ray, with the hit distance.
    pub(crate) fn pick(&self, origin: DVec3, dir: DVec3) -> Option<(NodeId, f64)> {
        let visible = visible_nodes(self.view_map());
        let mut best: Option<(NodeId, f64)> = None;
        for (id, n) in self.view_map().nodes() {
            if !visible.contains(&id) || n.locked {
                continue;
            }
            let t = match &n.kind {
                NodeKind::Brush(b) => b.planes().ok().and_then(|p| mapcraft_geom::ray_hit(&p, origin, dir)).map(|(t, _)| t),
                NodeKind::Entity(e) if n.children.is_empty() => self.point_box(e).ray_hit(origin, dir),
                _ => None,
            };
            if let Some(t) = t
                && best.is_none_or(|(_, b)| t < b)
            {
                best = Some((id, t));
            }
        }
        best.map(|(id, t)| (self.owner(id), t))
    }

    /// Nearest visible brush face on the ray.
    pub(crate) fn pick_face(&self, origin: DVec3, dir: DVec3) -> Option<(NodeId, FaceId, f64)> {
        let visible = visible_nodes(&self.session.map);
        let mut best: Option<(NodeId, FaceId, f64)> = None;
        for (id, n) in self.session.map.nodes() {
            let (true, false, Some(b)) = (visible.contains(&id), n.locked, n.kind.brush()) else { continue };
            let Some((t, i)) = b.planes().ok().and_then(|p| mapcraft_geom::ray_hit(&p, origin, dir)) else { continue };
            if best.is_none_or(|(_, _, bt)| t < bt)
                && let Some(f) = b.faces.get(i)
            {
                best = Some((id, f.id, t));
            }
        }
        best
    }

    /// Corners of the selected brushes.
    fn selected_vertices(&self) -> Vec<DVec3> {
        let mut out: Vec<DVec3> = Vec::new();
        for id in self.session.selected_brushes() {
            if let Some(Ok(vs)) = self.session.map.get(id).and_then(|n| n.kind.brush()).map(|b| b.vertices()) {
                for v in vs {
                    if !out.iter().any(|o| o.distance_squared(v) < 1e-6) {
                        out.push(v);
                    }
                }
            }
        }
        out
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
        self.link_lines(&mut out);
        if let Some(b) = self.selection_bounds().filter(|_| self.tool == Tool::Select) {
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
            let names: Vec<&str> = ["target", "parentname", "respawn", "landmark"]
                .iter()
                .filter_map(|k| e.get(k))
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

    fn snapped_plane_point(&self, index: usize, pos: Pos2, rect: Rect) -> Option<DVec3> {
        let pane = self.panes.get(index)?;
        let p = pane.camera.plane_point(dvec(pos - rect.min), dvec(rect.size()));
        let (r, u, _) = pane.camera.view.axes();
        let g = self.session.grid;
        Some(r * snap(DVec3::splat(p.dot(r)), g).x + u * snap(DVec3::splat(p.dot(u)), g).x)
    }

    pub(crate) fn viewport(&mut self, ui: &mut egui::Ui, index: usize, rect: Rect) {
        if rect.width() < 2.0 || rect.height() < 2.0 {
            return;
        }
        let ppp = ui.ctx().pixels_per_point();
        let px = [(rect.width() * ppp).round() as u32, (rect.height() * ppp).round() as u32];
        let overlay = self.overlay_lines();
        let grid = self.session.grid;
        let mode = self.render_mode;
        if let (Some(gpu), Some(pane)) = (&mut self.gpu, self.panes.get_mut(index)) {
            pane.rect = rect;
            if pane.target.as_ref().is_none_or(|t| t.size != px) {
                let target = gpu.renderer.target(px);
                let mut egui_renderer = gpu.state.renderer.write();
                pane.texture = Some(match pane.texture {
                    Some(id) => {
                        egui_renderer.update_egui_texture_from_wgpu_texture(&gpu.state.device, &target.view, wgpu_filter(), id);
                        id
                    }
                    None => egui_renderer.register_native_texture(&gpu.state.device, &target.view, wgpu_filter()),
                });
                pane.target = Some(target);
            }
            if let Some(t) = &pane.target {
                gpu.renderer.render(t, &pane.camera, mode, Some(grid), &overlay);
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
            ui.painter().rect_stroke(Rect::from_two_pos(*start, p), 0.0, Stroke::new(1.0, Color32::YELLOW), egui::StrokeKind::Inside);
        }
        if self.tool == Tool::Select {
            self.draw_handles(ui, index, rect);
        }
        if self.tool == Tool::Vertex
            && let Some(cam) = self.panes.get(index).map(|p| p.camera)
        {
            for v in self.selected_vertices() {
                if let Some(p) = cam.project(v, dvec(rect.size())) {
                    let r = Rect::from_center_size(rect.min + Vec2::new(p.x as f32, p.y as f32), Vec2::splat(6.0));
                    if rect.contains(r.center()) {
                        ui.painter().rect_filled(r, 0.0, Color32::from_rgb(255, 220, 60));
                    }
                }
            }
        }
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
            ui.painter().rect_filled(r, 0.0, Color32::WHITE);
            ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, Color32::BLACK), egui::StrokeKind::Inside);
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

        if cam.is_2d() {
            if let Some(h) = hover {
                let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
                if scroll != 0.0
                    && let Some(p) = self.panes.get_mut(index)
                {
                    p.camera.zoom_at(dvec(h - rect.min), size, (f64::from(scroll) / 200.0).exp());
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
            self.begin_drag(index, rect, p);
        }
        if response.dragged_by(egui::PointerButton::Primary)
            && let Some(p) = primary_pos
        {
            self.update_drag(index, rect, p);
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
                        let material = self
                            .session
                            .map
                            .get(id)
                            .and_then(|n| n.kind.brush())
                            .and_then(|b| b.faces.iter().find(|f| f.id == face))
                            .map(|f| f.material.clone());
                        if let Some(m) = material {
                            self.status = format!("Picked {m}");
                            self.session.tex.material = m;
                        }
                    } else if let Err(e) = self.session.apply_material_to_face(id, face) {
                        self.status = e.to_string();
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

    fn click(&mut self, index: usize, rect: Rect, pos: Pos2, additive: bool) {
        let Some(cam) = self.panes.get(index).map(|p| p.camera) else { return };
        let size = dvec(rect.size());
        let (origin, dir) = cam.ray(dvec(pos - rect.min), size);
        match self.tool {
            Tool::Select | Tool::Block | Tool::Vertex => {
                let hit = self.pick(origin, dir).map(|(id, _)| id);
                self.session.click(hit, additive);
                self.mark_changed();
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
                if !cam.is_2d() {
                    return;
                }
                if self.clip_points.len() >= 2 {
                    self.clip_points.clear();
                }
                if let Some(p) = self.snapped_plane_point(index, pos, rect) {
                    let (_, _, t) = cam.view.axes();
                    self.clip_points.push(p);
                    self.clip_view_axis = Some(t);
                }
            }
        }
    }

    fn begin_drag(&mut self, index: usize, rect: Rect, pos: Pos2) {
        let Some(cam) = self.panes.get(index).map(|p| p.camera) else { return };
        if !cam.is_2d() {
            return;
        }
        let size = dvec(rect.size());
        match self.tool {
            Tool::Select => {
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
                        self.drag = Drag::Move { view: index, start, applied: DVec3::ZERO };
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
                let near: Vec<DVec3> =
                    self.selected_vertices().into_iter().filter(|v| cam.project(*v, size).is_some_and(|p| (p - dvec(pos - rect.min)).length() < 6.0)).collect();
                if !near.is_empty() {
                    let start = cam.plane_point(dvec(pos - rect.min), size);
                    self.drag = Drag::Vertex { view: index, originals: near, start, applied: DVec3::ZERO };
                }
            }
            Tool::Entity | Tool::Clip | Tool::Face => {}
        }
    }

    fn update_drag(&mut self, index: usize, rect: Rect, pos: Pos2) {
        let Some(cam) = self.panes.get(index).map(|p| p.camera) else { return };
        let size = dvec(rect.size());
        let g = self.session.grid;
        let (r, u, _) = cam.view.axes();
        let snapped = |d: DVec3| r * (d.dot(r) / g).round() * g + u * (d.dot(u) / g).round() * g;
        match self.drag.clone() {
            Drag::Move { view, start, applied } if view == index => {
                let total = snapped(cam.plane_point(dvec(pos - rect.min), size) - start);
                let step = total - applied;
                if step != DVec3::ZERO {
                    if let Err(e) = self.session.translate_selection(step, Some("drag-move")) {
                        self.status = e.to_string();
                    }
                    self.drag = Drag::Move { view, start, applied: total };
                    self.mark_changed();
                }
            }
            Drag::Resize { view, handle, bounds, applied } if view == index => {
                let p = cam.plane_point(dvec(pos - rect.min), size);
                let mut target = bounds;
                for (axis, side) in [(r, handle.0), (u, handle.1)] {
                    let i = axis.abs().max_position();
                    let v = (p.dot(axis.abs()) / g).round() * g;
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
            Drag::Vertex { view, originals, start, applied } if view == index => {
                let total = snapped(cam.plane_point(dvec(pos - rect.min), size) - start);
                if total != applied {
                    let moves: Vec<(DVec3, DVec3)> = originals.iter().map(|o| (*o + applied, *o + total)).collect();
                    match self.session.move_vertices(&moves, Some("drag-vertex")) {
                        Ok(()) => self.drag = Drag::Vertex { view, originals, start, applied: total },
                        Err(e) => self.status = format!("Cannot move there: {e}"),
                    }
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
        match std::mem::take(&mut self.drag) {
            Drag::Marquee { view, start } if view == index => {
                let Some(cam) = self.panes.get(index).map(|p| p.camera) else { return };
                let (a, b) = (cam.plane_point(dvec(start - rect.min), size), cam.plane_point(dvec(pos - rect.min), size));
                let (r, u, _) = cam.view.axes();
                let (h0, h1) = (a.dot(r).min(b.dot(r)), a.dot(r).max(b.dot(r)));
                let (v0, v1) = (a.dot(u).min(b.dot(u)), a.dot(u).max(b.dot(u)));
                let visible = visible_nodes(self.view_map());
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
            Drag::Move { .. } | Drag::Resize { .. } | Drag::Vertex { .. } => self.session.end_coalesce(),
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
fn box_transform(from: &Aabb, to: &Aabb) -> DMat4 {
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
