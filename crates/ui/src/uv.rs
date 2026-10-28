//! The UV editor: picked faces laid over their texture, with Blender's grab, rotate and scale.

use std::collections::{BTreeMap, BTreeSet};

use egui::{Align2, Color32, CursorIcon, FontId, PointerButton, Pos2, Rect, Sense, Shape, Stroke, Vec2};
use mapcraft_engine::{FacePick, face_polygons, uv_islands};
use mapcraft_geom::{DAffine2, DVec2};
use mapcraft_render::Renderer;

use crate::app::MapCraftApp;
use crate::chrome::{ACCENT, TEXT_WEAK};
use crate::viewport::{Drag, dvec};

/// Pan and zoom of a UV view: the point at its center, in texels of the texture it shows, and
/// pixels per texel.
#[derive(Clone, Copy, Debug)]
pub(crate) struct UvCamera {
    pub center: DVec2,
    pub zoom: f64,
    /// Fits the picked faces into the view on its next frame.
    pub frame: bool,
    /// Where the view was last drawn; the inspector's is `Rect::NOTHING` while it is hidden.
    pub rect: Rect,
    /// A hash of the picks it last fitted; picks changed outside UV views fit it again.
    pub picks: u64,
}

impl Default for UvCamera {
    fn default() -> Self {
        Self { center: DVec2::ZERO, zoom: 1.0, frame: true, rect: Rect::NOTHING, picks: 0 }
    }
}

/// A UV view: one filling a pane, or the one atop the Face tool's inspector.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum UvView {
    Pane(usize),
    Inspector,
}

impl UvCamera {
    fn screen(&self, rect: Rect, p: DVec2) -> Pos2 {
        let d = (p - self.center) * self.zoom;
        rect.center() + Vec2::new(d.x as f32, d.y as f32)
    }

    fn uv(&self, rect: Rect, p: Pos2) -> DVec2 {
        self.center + dvec(p - rect.center()) / self.zoom
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UvMode {
    Grab,
    Rotate,
    Scale,
}

/// A grab, rotate or scale of the picked faces' textures that follows the pointer until a click
/// or Enter applies it (Blender's modal transforms).
#[derive(Clone, Copy, Debug)]
pub(crate) struct UvModal {
    mode: UvMode,
    view: UvView,
    /// The pointer as the transform began, taken on its first frame.
    start: Option<Pos2>,
    pivot: DVec2,
    /// Locks the transform to U (0) or V (1).
    axis: Option<usize>,
    /// Scales U and V apart, as a corner handle does, unless Shift keeps the aspect.
    free: bool,
    /// A drag of a face or handle, which applies on release, and its cursor.
    drag: Option<CursorIcon>,
    applied: DAffine2,
}

impl UvModal {
    fn new(mode: UvMode, view: UvView, pivot: DVec2) -> Self {
        Self { mode, view, start: None, pivot, axis: None, free: false, drag: None, applied: DAffine2::IDENTITY }
    }

    /// The whole transform for the pointer at `pointer`, with `snap` in whole texels, 15 degrees or
    /// tenths.
    fn transform(&self, cam: &UvCamera, rect: Rect, start: Pos2, pointer: Pos2, snap: bool, keep_aspect: bool) -> DAffine2 {
        let around = |m: DAffine2| DAffine2::from_translation(self.pivot) * m * DAffine2::from_translation(-self.pivot);
        let pivot = cam.screen(rect, self.pivot);
        match self.mode {
            UvMode::Grab => {
                let mut d = dvec(pointer - start) / cam.zoom;
                if snap {
                    d = d.round();
                }
                DAffine2::from_translation(match self.axis {
                    Some(0) => DVec2::new(d.x, 0.0),
                    Some(_) => DVec2::new(0.0, d.y),
                    None => d,
                })
            }
            UvMode::Rotate => {
                let (a, b) = (dvec(start - pivot), dvec(pointer - pivot));
                let mut turn = b.y.atan2(b.x) - a.y.atan2(a.x);
                if snap {
                    let step = 15f64.to_radians();
                    turn = (turn / step).round() * step;
                }
                around(DAffine2::from_angle(turn))
            }
            UvMode::Scale => {
                let ratio = |to: f64, from: f64| if from.abs() > 1.0 { to / from } else { 1.0 };
                let (a, b) = (dvec(start - pivot), dvec(pointer - pivot));
                let mut f = if self.free && !keep_aspect { DVec2::new(ratio(b.x, a.x), ratio(b.y, a.y)) } else { DVec2::splat(ratio(b.length(), a.length())) };
                if snap {
                    f = ((f * 10.0).round() / 10.0).max(DVec2::splat(0.1));
                }
                let f = f.max(DVec2::splat(0.01));
                around(DAffine2::from_scale(match self.axis {
                    Some(0) => DVec2::new(f.x, 1.0),
                    Some(_) => DVec2::new(1.0, f.y),
                    None => f,
                }))
            }
        }
    }

    pub(crate) fn hint(&self) -> String {
        match self.drag {
            Some(_) if self.free => format!("{}; Shift keeps the aspect, Ctrl snaps, Esc or right click cancels", self.done()),
            Some(_) => format!("{}; Ctrl snaps, Esc or right click cancels", self.done()),
            None => format!("{}; X or Y locks an axis, Ctrl snaps, click or Enter applies, Esc or right click cancels", self.done()),
        }
    }

    /// How far the transform has gone, as the view shows it.
    fn done(&self) -> String {
        let m = self.applied;
        let lock = match self.axis {
            Some(0) => " along U",
            Some(_) => " along V",
            None => "",
        };
        match self.mode {
            UvMode::Grab => format!("Moved {:.1}, {:.1} texels{lock}", m.translation.x, m.translation.y),
            UvMode::Rotate => format!("Turned {:.1} degrees", m.matrix2.x_axis.y.atan2(m.matrix2.x_axis.x).to_degrees()),
            UvMode::Scale => format!("Scaled {:.2}, {:.2}{lock}", m.matrix2.x_axis.x, m.matrix2.y_axis.y),
        }
    }
}

/// A face as a UV view draws it.
struct UvFace {
    pick: FacePick,
    points: Vec<DVec2>,
    picked: bool,
    island: usize,
}

/// Above this many picked faces islands share one color, as finding them costs the square of it.
const ISLAND_LIMIT: usize = 256;

/// Texel size of a material's texture as faces are mapped with it; 64 until it loads.
pub(crate) fn texture_size(renderer: Option<&Renderer>, material: &str) -> DVec2 {
    renderer.and_then(|r| r.material_size(material)).map_or(DVec2::splat(64.0), |s| DVec2::new(f64::from(s[0]), f64::from(s[1])))
}

fn bounds(points: impl Iterator<Item = DVec2>) -> Option<(DVec2, DVec2)> {
    points.fold(None, |b, p| Some(b.map_or((p, p), |(lo, hi): (DVec2, DVec2)| (lo.min(p), hi.max(p)))))
}

fn picked_bounds(faces: &[UvFace]) -> Option<(DVec2, DVec2)> {
    bounds(faces.iter().filter(|f| f.picked).flat_map(|f| f.points.iter().copied()))
}

/// Screen pixels from a handle that still grab it.
const HANDLE_REACH: f32 = 7.0;
/// Screen pixels the rotate handle sits above the box.
const KNOB: f32 = 20.0;

/// The handles of the box around the picked faces' UVs, where each draws and the drag it starts:
/// corners and edges scale about the opposite side, the knob above turns about the middle.
fn uv_handles(cam: &UvCamera, rect: Rect, view: UvView, (lo, hi): (DVec2, DVec2)) -> Vec<(Pos2, UvModal)> {
    let (mid, half) = ((lo + hi) / 2.0, (hi - lo) / 2.0);
    let mut out: Vec<(Pos2, UvModal)> = [(-1i8, -1i8), (1, -1), (1, 1), (-1, 1), (0, -1), (1, 0), (0, 1), (-1, 0)]
        .into_iter()
        .map(|(i, j)| {
            let side = DVec2::new(f64::from(i), f64::from(j)) * half;
            let (axis, cursor) = match (i, j) {
                (0, _) => (Some(1), CursorIcon::ResizeVertical),
                (_, 0) => (Some(0), CursorIcon::ResizeHorizontal),
                _ if i == j => (None, CursorIcon::ResizeNwSe),
                _ => (None, CursorIcon::ResizeNeSw),
            };
            (cam.screen(rect, mid + side), UvModal { axis, free: true, drag: Some(cursor), ..UvModal::new(UvMode::Scale, view, mid - side) })
        })
        .collect();
    let knob = cam.screen(rect, DVec2::new(mid.x, lo.y)) - Vec2::new(0.0, KNOB);
    out.push((knob, UvModal { drag: Some(CursorIcon::Grab), ..UvModal::new(UvMode::Rotate, view, mid) }));
    out
}

/// Whether `p` lies inside `poly`, by the even-odd rule.
fn contains(poly: &[DVec2], p: DVec2) -> bool {
    let mut inside = false;
    for (a, b) in poly.iter().zip(poly.iter().cycle().skip(1)) {
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) * (b.x - a.x) / (b.y - a.y) {
            inside = !inside;
        }
    }
    inside
}

fn island_color(island: usize) -> Color32 {
    egui::ecolor::Hsva::new((island as f32 * 0.618_034 + 0.52).fract(), 0.65, 1.0, 1.0).into()
}

impl MapCraftApp {
    fn uv_camera(&self, view: UvView) -> Option<UvCamera> {
        match view {
            UvView::Pane(i) => self.panes.get(i)?.uv,
            UvView::Inspector => Some(self.uv_panel),
        }
    }

    fn uv_cameras(&mut self) -> impl Iterator<Item = (UvView, &mut UvCamera)> {
        let panes = self.panes.iter_mut().enumerate().filter_map(|(i, p)| Some((UvView::Pane(i), p.uv.as_mut()?)));
        panes.chain([(UvView::Inspector, &mut self.uv_panel)])
    }

    fn uv_picks(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::hash::DefaultHasher::new();
        self.session.face_selection.hash(&mut h);
        h.finish()
    }

    /// Fits the picked faces in the UV view under the pointer, else in every UV view.
    pub(crate) fn fit_uv(&mut self) {
        let hovered = self.uv_hovered;
        for (view, cam) in self.uv_cameras() {
            cam.frame |= hovered.is_none_or(|h| h == view);
        }
    }

    fn set_uv_camera(&mut self, view: UvView, cam: UvCamera) {
        let slot = match view {
            UvView::Pane(i) => self.panes.get_mut(i).and_then(|p| p.uv.as_mut()),
            UvView::Inspector => Some(&mut self.uv_panel),
        };
        if let Some(c) = slot {
            *c = cam;
        }
    }

    /// The material UV views show, the active face's, and its texture size.
    pub(crate) fn uv_reference(&self) -> Option<(String, DVec2)> {
        let (id, face) = self.session.active_pick()?;
        let f = self.session.map.get(id)?.kind.brush()?.faces.iter().find(|f| f.id == face)?;
        Some((f.material.clone(), texture_size(self.gpu.as_ref().map(|g| &g.renderer), &f.material)))
    }

    /// Every face of the brushes with picked faces, in texels of a texture of `reference` size;
    /// picked faces come last so they draw on top.
    fn uv_faces(&self, reference: DVec2) -> Vec<UvFace> {
        let picks = &self.session.face_selection;
        let islands: BTreeMap<FacePick, usize> = if picks.len() <= ISLAND_LIMIT {
            let found = uv_islands(&self.session.map, picks, None);
            found.into_iter().enumerate().flat_map(|(i, island)| island.into_iter().map(move |(p, _)| (p, i))).collect()
        } else {
            BTreeMap::new()
        };
        let brushes: BTreeSet<_> = picks.iter().map(|(id, _)| *id).collect();
        let renderer = self.gpu.as_ref().map(|g| &g.renderer);
        let mut out: Vec<UvFace> = face_polygons(&self.session.map, &brushes)
            .into_iter()
            .map(|(pick, f, w)| {
                let size = texture_size(renderer, &f.material);
                let points = w.iter().map(|p| f.uv(*p, size) * reference).collect();
                UvFace { pick, points, picked: picks.contains(&pick), island: islands.get(&pick).copied().unwrap_or(0) }
            })
            .collect();
        out.sort_by_key(|f| f.picked);
        out
    }

    /// Starts a grab, rotate or scale of the picked faces in the UV view under the pointer, else
    /// the first one.
    pub(crate) fn begin_uv_modal(&mut self, mode: UvMode) {
        if !matches!(self.drag, Drag::None) {
            return;
        }
        let shown = |v: &UvView| self.uv_camera(*v).is_some_and(|c| c.rect.is_positive());
        let mut views = (0..self.panes.len()).map(UvView::Pane).chain([UvView::Inspector]);
        let Some(view) = self.uv_hovered.filter(shown).or_else(|| views.find(shown)) else {
            self.status = "Open UV in the Face tool's inspector, or turn a view into the UV Editor from its header".into();
            return;
        };
        let Some((_, size)) = self.uv_reference() else {
            self.status = format!("Pick faces with the Face tool ({}) first", self.chord("tool.face"));
            return;
        };
        let Some((lo, hi)) = picked_bounds(&self.uv_faces(size)) else { return };
        self.session.end_coalesce();
        self.uv_modal = Some(UvModal::new(mode, view, (lo + hi) / 2.0));
    }

    /// Drives a running UV transform from the pointer and keys; every other shortcut waits for it.
    pub(crate) fn uv_modal_input(&mut self, ctx: &egui::Context) {
        let Some(mut m) = self.uv_modal else { return };
        let (Some(cam), Some((_, size))) = (self.uv_camera(m.view).filter(|c| c.rect.is_positive()), self.uv_reference()) else {
            self.uv_modal = None;
            return;
        };
        let rect = cam.rect;
        use egui::Key;
        let (pointer, snap, keep_aspect, x, y, enter, escape, confirm, cancel) = ctx.input(|i| {
            (
                i.pointer.latest_pos(),
                i.modifiers.command,
                i.modifiers.shift,
                i.key_pressed(Key::X),
                i.key_pressed(Key::Y),
                i.key_pressed(Key::Enter),
                i.key_pressed(Key::Escape),
                i.pointer.primary_released(),
                i.pointer.button_released(PointerButton::Secondary),
            )
        });
        for (pressed, axis) in [(x, 0), (y, 1)] {
            if pressed && m.drag.is_none() {
                m.axis = if m.axis == Some(axis) { None } else { Some(axis) };
            }
        }
        if let Some(pointer) = pointer {
            let start = *m.start.get_or_insert(pointer);
            let total = m.transform(&cam, rect, start, pointer, snap, keep_aspect);
            if !total.abs_diff_eq(m.applied, 1e-12) {
                let label = match m.mode {
                    UvMode::Grab => "Move UVs",
                    UvMode::Rotate => "Rotate UVs",
                    UvMode::Scale => "Scale UVs",
                };
                let renderer = self.gpu.as_ref().map(|g| &g.renderer);
                let step = total * m.applied.inverse();
                match self.session.transform_uvs(label, step, size, &|name| texture_size(renderer, name), Some("uv-transform")) {
                    Ok(()) => {
                        m.applied = total;
                        self.mark_changed();
                    }
                    Err(e) => self.status = e.to_string(),
                }
            }
        }
        self.uv_modal = Some(m);
        let back = m.applied.abs_diff_eq(DAffine2::IDENTITY, 1e-12);
        if (enter || confirm) && !back {
            self.session.end_coalesce();
            self.uv_modal = None;
        } else if enter || confirm || escape || cancel {
            if self.session.cancel_coalesce("uv-transform") {
                self.mark_changed();
            }
            self.uv_modal = None;
        }
    }

    /// Picks faces from a UV view the way the Face tool picks them in the 3D view.
    fn pick_uv_faces(&mut self, hits: Vec<FacePick>, additive: bool, toggle: bool) {
        if !additive {
            self.session.face_selection.clear();
        }
        for h in hits {
            if !(toggle && self.session.face_selection.remove(&h)) {
                self.session.face_selection.insert(h);
                self.session.active_face = Some(h);
            }
        }
        let picks = self.uv_picks();
        self.uv_cameras().for_each(|(_, c)| c.picks = picks);
        self.mark_changed();
    }

    pub(crate) fn uv_view(&mut self, ui: &mut egui::Ui, view: UvView, rect: Rect) {
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, Color32::from_gray(28));
        if let UvView::Pane(i) = view
            && let Some(pane) = self.panes.get_mut(i)
        {
            pane.rect = rect;
        }
        let Some(cam) = self.uv_camera(view) else { return };
        self.set_uv_camera(view, UvCamera { rect, ..cam });
        if ui.rect_contains_pointer(rect) {
            self.uv_hovered = Some(view);
        }
        let Some((material, size)) = self.uv_reference() else {
            let text = format!("Pick faces with the Face tool ({}) to see their UVs here", self.chord("tool.face"));
            painter.text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(14.0), TEXT_WEAK);
            return;
        };
        let faces = self.uv_faces(size);
        let response = ui.interact(rect, ui.id().with(("uv-view", view)), Sense::click_and_drag());
        if !self.uv_busy {
            self.uv_input(ui, view, rect, &response, &faces);
        }
        let Some(mut cam) = self.uv_camera(view) else { return };
        let picks = self.uv_picks();
        if cam.frame || cam.picks != picks {
            if let Some((lo, hi)) = picked_bounds(&faces) {
                cam.center = (lo + hi) / 2.0;
                cam.zoom = ((dvec(rect.size()) / (hi - lo).max(DVec2::ONE)).min_element() / 1.3).clamp(1.0 / 64.0, 64.0);
            }
            (cam.frame, cam.picks) = (false, picks);
            self.set_uv_camera(view, cam);
        }
        self.draw_uv_texture(ui, &painter, rect, &cam, &material, size);
        for f in &faces {
            let pts: Vec<Pos2> = f.points.iter().map(|p| cam.screen(rect, *p)).collect();
            if f.picked {
                let c = island_color(f.island);
                let active = self.session.active_pick() == Some(f.pick);
                let stroke = if active { Stroke::new(2.5, Color32::WHITE) } else { Stroke::new(1.5, c) };
                painter.add(Shape::convex_polygon(pts, c.gamma_multiply(0.22), stroke));
            } else {
                painter.add(Shape::closed_line(pts, Stroke::new(1.0, Color32::from_white_alpha(70))));
            }
        }
        if let Some((lo, hi)) = picked_bounds(&faces) {
            painter.rect_stroke(Rect::from_two_pos(cam.screen(rect, lo), cam.screen(rect, hi)), 0.0, Stroke::new(1.0, ACCENT), egui::StrokeKind::Middle);
            for (at, m) in uv_handles(&cam, rect, view, (lo, hi)) {
                if m.mode == UvMode::Rotate {
                    painter.line_segment([at, at + Vec2::new(0.0, KNOB)], Stroke::new(1.0, ACCENT));
                    painter.circle(at, 5.0, Color32::WHITE, Stroke::new(1.0, Color32::BLACK));
                } else {
                    painter.rect(Rect::from_center_size(at, Vec2::splat(7.0)), 0.0, Color32::WHITE, Stroke::new(1.0, Color32::BLACK), egui::StrokeKind::Inside);
                }
            }
        }
        if let Some(m) = self.uv_modal.filter(|m| m.view == view) {
            if let Some(cursor) = m.drag {
                ui.ctx().set_cursor_icon(cursor);
            }
            let c = cam.screen(rect, m.pivot);
            if let Some(p) = ui.ctx().pointer_latest_pos().filter(|_| m.mode != UvMode::Grab) {
                painter.circle_stroke(c, 5.0, Stroke::new(1.5, ACCENT));
                painter.add(Shape::dashed_line(&[c, p], Stroke::new(1.0, ACCENT), 4.0, 4.0));
            }
            let text = painter.layout_no_wrap(m.done(), FontId::proportional(13.0), Color32::WHITE);
            let at = rect.left_top() + Vec2::splat(8.0);
            painter.rect_filled(Rect::from_min_size(at, text.size()).expand(4.0), 3.0, Color32::from_black_alpha(170));
            painter.galley(at, text, Color32::WHITE);
        }
        if let Drag::UvMarquee { view: v, start } = &self.drag
            && *v == view
            && let Some(p) = response.interact_pointer_pos()
        {
            painter.rect_stroke(Rect::from_two_pos(*start, p), 0.0, Stroke::new(1.0, Color32::YELLOW), egui::StrokeKind::Inside);
        }
    }

    /// The texture tiled across the view, its first tile outlined, and a grid of texels.
    fn draw_uv_texture(&mut self, ui: &egui::Ui, painter: &egui::Painter, rect: Rect, cam: &UvCamera, material: &str, size: DVec2) {
        let (lo, hi) = (cam.uv(rect, rect.min), cam.uv(rect, rect.max));
        let (t0, t1) = ((lo / size).floor(), (hi / size).floor());
        let tiles = (t1 - t0 + DVec2::ONE).max(DVec2::ZERO);
        let image = self.project.find_material(material).and_then(|m| self.uv_images.get(ui.ctx(), &m).map(|t| t.id()));
        let tile_rect = |i: f64, j: f64| Rect::from_two_pos(cam.screen(rect, DVec2::new(i, j) * size), cam.screen(rect, DVec2::new(i + 1.0, j + 1.0) * size));
        if let Some(tex) = image.filter(|_| tiles.x * tiles.y <= 400.0) {
            for j in t0.y as i64..=t1.y as i64 {
                for i in t0.x as i64..=t1.x as i64 {
                    let tint = if i == 0 && j == 0 { Color32::WHITE } else { Color32::from_gray(150) };
                    painter.image(tex, tile_rect(i as f64, j as f64), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), tint);
                }
            }
        }
        let line = |painter: &egui::Painter, step: DVec2, color: Color32| {
            if (step * cam.zoom).min_element() < 4.0 {
                return;
            }
            let (a, b) = ((lo / step).ceil(), (hi / step).floor());
            for i in a.x as i64..=b.x as i64 {
                let x = cam.screen(rect, DVec2::new(i as f64 * step.x, 0.0)).x;
                painter.vline(x, rect.y_range(), Stroke::new(1.0, color));
            }
            for j in a.y as i64..=b.y as i64 {
                let y = cam.screen(rect, DVec2::new(0.0, j as f64 * step.y)).y;
                painter.hline(rect.x_range(), y, Stroke::new(1.0, color));
            }
        };
        line(painter, DVec2::splat(mapcraft_render::grid_step(1.0, cam.zoom)), Color32::from_white_alpha(14));
        line(painter, size, Color32::from_white_alpha(50));
        painter.rect_stroke(tile_rect(0.0, 0.0), 0.0, Stroke::new(1.5, ACCENT), egui::StrokeKind::Middle);
    }

    /// Pan, zoom, picking and the context menu of a UV view.
    fn uv_input(&mut self, ui: &egui::Ui, view: UvView, rect: Rect, response: &egui::Response, faces: &[UvFace]) {
        let ctx = ui.ctx().clone();
        let Some(mut cam) = self.uv_camera(view) else { return };
        if let Some(h) = response.hover_pos() {
            // Taken so the inspector around it does not scroll too.
            let scroll = ctx.input_mut(|i| std::mem::take(&mut i.smooth_scroll_delta)).y;
            if scroll != 0.0 {
                let at = cam.uv(rect, h);
                cam.zoom = (cam.zoom * (f64::from(scroll) / 200.0).exp()).clamp(1.0 / 64.0, 256.0);
                cam.center = at - dvec(h - rect.center()) / cam.zoom;
            }
        }
        if response.dragged_by(PointerButton::Middle) || response.dragged_by(PointerButton::Secondary) {
            cam.center -= dvec(response.drag_delta()) / cam.zoom;
        }
        if response.double_clicked() {
            cam.frame = true;
        }
        self.set_uv_camera(view, cam);
        let additive = ctx.input(|i| i.modifiers.command || i.modifiers.shift);
        let handles = picked_bounds(faces).map(|b| uv_handles(&cam, rect, view, b)).unwrap_or_default();
        let handle_at = |p: Pos2| handles.iter().find(|(at, _)| at.distance(p) <= HANDLE_REACH).map(|(_, m)| *m);
        let face_at = |p: Pos2| faces.iter().rev().find(|f| contains(&f.points, cam.uv(rect, p)));
        if let Some(h) = response.hover_pos().filter(|_| !response.dragged()) {
            let picked = face_at(h).is_some_and(|f| f.picked);
            if let Some(cursor) = handle_at(h).and_then(|m| m.drag).or(picked.then_some(CursorIcon::Move)) {
                ctx.set_cursor_icon(cursor);
            }
        }
        if response.clicked_by(PointerButton::Primary)
            && let Some(p) = response.interact_pointer_pos()
            && handle_at(p).is_none()
        {
            let hit = face_at(p).map(|f| f.pick);
            self.pick_uv_faces(hit.into_iter().collect(), additive, true);
        }
        if response.drag_started_by(PointerButton::Primary)
            && let Some(p) = ctx.input(|i| i.pointer.press_origin())
        {
            let modal = match (handle_at(p), face_at(p)) {
                (Some(m), _) => Some(m),
                (None, Some(f)) => {
                    if !f.picked {
                        self.pick_uv_faces(vec![f.pick], additive, false);
                    }
                    Some(UvModal { drag: Some(CursorIcon::Move), ..UvModal::new(UvMode::Grab, view, cam.uv(rect, p)) })
                }
                (None, None) => None,
            };
            if let Some(m) = modal {
                self.session.end_coalesce();
                self.uv_modal = Some(UvModal { start: Some(p), ..m });
            } else {
                self.drag = Drag::UvMarquee { view, start: p };
            }
        }
        if response.drag_stopped()
            && let Drag::UvMarquee { view: v, start } = self.drag
            && v == view
        {
            self.drag = Drag::None;
            if let Some(end) = response.interact_pointer_pos().or(response.hover_pos()) {
                let (a, b) = (cam.uv(rect, start), cam.uv(rect, end));
                let inside = |f: &&UvFace| {
                    bounds(f.points.iter().copied()).is_some_and(|(lo, hi)| {
                        let c = (lo + hi) / 2.0;
                        c.cmpge(a.min(b)).all() && c.cmple(a.max(b)).all()
                    })
                };
                self.pick_uv_faces(faces.iter().filter(inside).map(|f| f.pick).collect(), additive, false);
            }
        }
        response.context_menu(|ui| {
            for id in ["uv.unwrap", "uv.project_from_view", "uv.cube_projection", "uv.reset"] {
                if let Some(c) = mapcraft_engine::find(id) {
                    self.command_item(ui, c);
                }
            }
            ui.separator();
            let hovered = self.uv_hovered.replace(view);
            self.action_button(ui, "uv.fit", "Fit UV View", true);
            self.uv_hovered = hovered;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modal_transforms_follow_the_pointer() {
        let cam = UvCamera { center: DVec2::ZERO, zoom: 2.0, frame: false, ..UvCamera::default() };
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::splat(200.0));
        let mut m = UvModal::new(UvMode::Grab, UvView::Pane(0), DVec2::new(10.0, 0.0));
        let at = |x: f32, y: f32| rect.center() + Vec2::new(x, y);
        let moved = m.transform(&cam, rect, at(0.0, 0.0), at(7.0, -3.0), true, false);
        assert_eq!(moved.translation, DVec2::new(4.0, -2.0), "half texels round away under Ctrl");
        m.axis = Some(1);
        assert_eq!(m.transform(&cam, rect, at(0.0, 0.0), at(7.0, -3.0), false, false).translation, DVec2::new(0.0, -1.5));
        m.mode = UvMode::Rotate;
        m.axis = None;
        let turned = m.transform(&cam, rect, at(40.0, 0.0), at(41.0, 19.0), true, false);
        assert!(turned.transform_point2(m.pivot).abs_diff_eq(m.pivot, 1e-9), "rotation keeps the pivot");
        assert!((turned.matrix2.x_axis.y.atan2(turned.matrix2.x_axis.x).to_degrees() - 45.0).abs() < 1e-9);
        m.mode = UvMode::Scale;
        m.axis = Some(0);
        let scaled = m.transform(&cam, rect, at(40.0, 0.0), at(60.0, 0.0), true, false);
        assert!((scaled.matrix2.x_axis.x - 2.0).abs() < 1e-9 && scaled.matrix2.y_axis.y == 1.0, "{scaled:?}");
        assert!(scaled.transform_point2(m.pivot).abs_diff_eq(m.pivot, 1e-9));
        assert!(contains(&[DVec2::ZERO, DVec2::new(4.0, 0.0), DVec2::new(0.0, 4.0)], DVec2::ONE));
        assert!(!contains(&[DVec2::ZERO, DVec2::new(4.0, 0.0), DVec2::new(0.0, 4.0)], DVec2::splat(3.0)));
    }

    #[test]
    fn handles_scale_about_the_opposite_side_and_turn_about_the_middle() {
        let cam = UvCamera { center: DVec2::new(5.0, 10.0), zoom: 2.0, frame: false, ..UvCamera::default() };
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::splat(200.0));
        let (lo, hi) = (DVec2::ZERO, DVec2::new(10.0, 20.0));
        let handles = uv_handles(&cam, rect, UvView::Pane(0), (lo, hi));
        let handle = |at: DVec2| handles.iter().copied().find(|(p, _)| *p == cam.screen(rect, at)).unwrap();

        let (from, corner) = handle(hi);
        let to = from + Vec2::new(10.0, -20.0);
        let scaled = corner.transform(&cam, rect, from, to, false, false);
        assert!(scaled.transform_point2(lo).abs_diff_eq(lo, 1e-9), "the opposite corner stays put");
        assert!(scaled.transform_point2(hi).abs_diff_eq(cam.uv(rect, to), 1e-9), "the corner follows the pointer: {scaled:?}");
        let kept = corner.transform(&cam, rect, from, to, false, true);
        assert!((kept.matrix2.x_axis.x - kept.matrix2.y_axis.y).abs() < 1e-9, "Shift keeps the aspect: {kept:?}");
        assert!(kept.transform_point2(lo).abs_diff_eq(lo, 1e-9));

        let (from, edge) = handle(DVec2::new(10.0, 10.0));
        let stretched = edge.transform(&cam, rect, from, from + Vec2::new(10.0, 7.0), false, false);
        assert_eq!((stretched.matrix2.x_axis.x, stretched.matrix2.y_axis.y), (1.5, 1.0), "an edge scales only its own axis");
        assert!(stretched.transform_point2(DVec2::new(0.0, 10.0)).abs_diff_eq(DVec2::new(0.0, 10.0), 1e-9), "about the opposite edge");

        let (knob, turn) = handles.iter().copied().find(|(_, m)| m.mode == UvMode::Rotate).unwrap();
        assert!(turn.pivot == (lo + hi) / 2.0 && knob.y < cam.screen(rect, lo).y, "the knob sits above the box and turns about its middle");
        let angle = |m: DAffine2| m.matrix2.x_axis.y.atan2(m.matrix2.x_axis.x).to_degrees();
        let to = cam.screen(rect, turn.pivot) + Vec2::new(50.0, -60.0);
        assert!((angle(turn.transform(&cam, rect, knob, to, false, false)) - 39.805_571).abs() < 1e-5);
        assert!((angle(turn.transform(&cam, rect, knob, to, true, false)) - 45.0).abs() < 1e-9, "Ctrl snaps to 15 degrees");
    }
}
