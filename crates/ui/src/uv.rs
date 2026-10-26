//! The UV editor: picked faces laid over their texture, with Blender's grab, rotate and scale.

use std::collections::{BTreeMap, BTreeSet};

use egui::{Align2, Color32, FontId, PointerButton, Pos2, Rect, Sense, Shape, Stroke, Vec2};
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
}

impl Default for UvCamera {
    fn default() -> Self {
        Self { center: DVec2::ZERO, zoom: 1.0, frame: true }
    }
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
    pane: usize,
    /// The pointer as the transform began, taken on its first frame.
    start: Option<Pos2>,
    pivot: DVec2,
    /// Locks the transform to U (0) or V (1).
    axis: Option<usize>,
    applied: DAffine2,
    changed: bool,
}

impl UvModal {
    /// The whole transform for the pointer at `pointer`, with `snap` in whole texels, 15 degrees or
    /// tenths.
    fn transform(&self, cam: &UvCamera, rect: Rect, start: Pos2, pointer: Pos2, snap: bool) -> DAffine2 {
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
                let from = dvec(start - pivot).length();
                let mut f = if from > 1.0 { dvec(pointer - pivot).length() / from } else { 1.0 };
                if snap {
                    f = ((f * 10.0).round() / 10.0).max(0.1);
                }
                let f = f.max(0.01);
                around(DAffine2::from_scale(match self.axis {
                    Some(0) => DVec2::new(f, 1.0),
                    Some(_) => DVec2::new(1.0, f),
                    None => DVec2::splat(f),
                }))
            }
        }
    }

    pub(crate) fn hint(&self) -> String {
        format!("{}; X or Y locks an axis, Ctrl snaps, click or Enter applies, Esc or right click cancels", self.done())
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
        let is_uv = |i: &usize| self.panes.get(*i).is_some_and(|p| p.uv.is_some());
        let Some(pane) = self.hovered_pane.filter(is_uv).or_else(|| (0..self.panes.len()).find(is_uv)) else {
            self.status = "Turn a view into the UV Editor from its header first".into();
            return;
        };
        let Some((_, size)) = self.uv_reference() else {
            self.status = format!("Pick faces with the Face tool ({}) first", self.chord("tool.face"));
            return;
        };
        let faces = self.uv_faces(size);
        let Some((lo, hi)) = bounds(faces.iter().filter(|f| f.picked).flat_map(|f| f.points.iter().copied())) else { return };
        self.session.end_coalesce();
        self.uv_modal = Some(UvModal { mode, pane, start: None, pivot: (lo + hi) / 2.0, axis: None, applied: DAffine2::IDENTITY, changed: false });
    }

    /// Drives a running UV transform from the pointer and keys; every other shortcut waits for it.
    pub(crate) fn uv_modal_input(&mut self, ctx: &egui::Context) {
        let Some(mut m) = self.uv_modal else { return };
        let view = self.panes.get(m.pane).and_then(|p| Some((p.uv?, p.rect)));
        let (Some((cam, rect)), Some((_, size))) = (view, self.uv_reference()) else {
            self.uv_modal = None;
            return;
        };
        use egui::Key;
        let (pointer, snap, x, y, enter, escape, confirm, cancel) = ctx.input(|i| {
            (
                i.pointer.latest_pos(),
                i.modifiers.command,
                i.key_pressed(Key::X),
                i.key_pressed(Key::Y),
                i.key_pressed(Key::Enter),
                i.key_pressed(Key::Escape),
                i.pointer.primary_released(),
                i.pointer.button_released(PointerButton::Secondary),
            )
        });
        for (pressed, axis) in [(x, 0), (y, 1)] {
            if pressed {
                m.axis = if m.axis == Some(axis) { None } else { Some(axis) };
            }
        }
        if let Some(pointer) = pointer {
            let start = *m.start.get_or_insert(pointer);
            let total = m.transform(&cam, rect, start, pointer, snap);
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
                        (m.applied, m.changed) = (total, true);
                        self.mark_changed();
                    }
                    Err(e) => self.status = e.to_string(),
                }
            }
        }
        self.uv_modal = Some(m);
        if enter || confirm {
            self.session.end_coalesce();
            self.uv_modal = None;
        } else if escape || cancel {
            if m.changed {
                self.session.undo();
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
        self.mark_changed();
    }

    pub(crate) fn uv_view(&mut self, ui: &mut egui::Ui, index: usize, rect: Rect) {
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, Color32::from_gray(28));
        let Some(pane) = self.panes.get_mut(index) else { return };
        pane.rect = rect;
        let Some((material, size)) = self.uv_reference() else {
            let text = format!("Pick faces with the Face tool ({}) to see their UVs here", self.chord("tool.face"));
            painter.text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(14.0), TEXT_WEAK);
            return;
        };
        let faces = self.uv_faces(size);
        let response = ui.interact(rect, ui.id().with(("uv-view", index)), Sense::click_and_drag());
        if !self.uv_busy {
            self.uv_input(ui, index, rect, &response, &faces);
        }
        let Some(mut cam) = self.panes.get(index).and_then(|p| p.uv) else { return };
        if cam.frame {
            if let Some((lo, hi)) = bounds(faces.iter().filter(|f| f.picked).flat_map(|f| f.points.iter().copied())) {
                cam.center = (lo + hi) / 2.0;
                cam.zoom = ((dvec(rect.size()) / (hi - lo).max(DVec2::ONE)).min_element() / 1.3).clamp(1.0 / 64.0, 64.0);
            }
            cam.frame = false;
            if let Some(p) = self.panes.get_mut(index) {
                p.uv = Some(cam);
            }
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
        if let Some(m) = self.uv_modal.filter(|m| m.pane == index) {
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
        if let Drag::Marquee { view, start } = &self.drag
            && *view == index
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
    fn uv_input(&mut self, ui: &egui::Ui, index: usize, rect: Rect, response: &egui::Response, faces: &[UvFace]) {
        let ctx = ui.ctx().clone();
        let Some(mut cam) = self.panes.get(index).and_then(|p| p.uv) else { return };
        if let Some(h) = response.hover_pos() {
            let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
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
        if let Some(p) = self.panes.get_mut(index) {
            p.uv = Some(cam);
        }
        let additive = ctx.input(|i| i.modifiers.command || i.modifiers.shift);
        if response.clicked_by(PointerButton::Primary)
            && let Some(p) = response.interact_pointer_pos()
        {
            let at = cam.uv(rect, p);
            let hit = faces.iter().rev().find(|f| contains(&f.points, at)).map(|f| f.pick);
            self.pick_uv_faces(hit.into_iter().collect(), additive, true);
        }
        if response.drag_started_by(PointerButton::Primary)
            && let Some(p) = ctx.input(|i| i.pointer.press_origin())
        {
            self.drag = Drag::Marquee { view: index, start: p };
        }
        if response.drag_stopped()
            && let Drag::Marquee { view, start } = self.drag
            && view == index
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
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modal_transforms_follow_the_pointer() {
        let cam = UvCamera { center: DVec2::ZERO, zoom: 2.0, frame: false };
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::splat(200.0));
        let mut m = UvModal { mode: UvMode::Grab, pane: 0, start: None, pivot: DVec2::new(10.0, 0.0), axis: None, applied: DAffine2::IDENTITY, changed: false };
        let at = |x: f32, y: f32| rect.center() + Vec2::new(x, y);
        let moved = m.transform(&cam, rect, at(0.0, 0.0), at(7.0, -3.0), true);
        assert_eq!(moved.translation, DVec2::new(4.0, -2.0), "half texels round away under Ctrl");
        m.axis = Some(1);
        assert_eq!(m.transform(&cam, rect, at(0.0, 0.0), at(7.0, -3.0), false).translation, DVec2::new(0.0, -1.5));
        m.mode = UvMode::Rotate;
        m.axis = None;
        let turned = m.transform(&cam, rect, at(40.0, 0.0), at(41.0, 19.0), true);
        assert!(turned.transform_point2(m.pivot).abs_diff_eq(m.pivot, 1e-9), "rotation keeps the pivot");
        assert!((turned.matrix2.x_axis.y.atan2(turned.matrix2.x_axis.x).to_degrees() - 45.0).abs() < 1e-9);
        m.mode = UvMode::Scale;
        m.axis = Some(0);
        let scaled = m.transform(&cam, rect, at(40.0, 0.0), at(60.0, 0.0), true);
        assert!((scaled.matrix2.x_axis.x - 2.0).abs() < 1e-9 && scaled.matrix2.y_axis.y == 1.0, "{scaled:?}");
        assert!(scaled.transform_point2(m.pivot).abs_diff_eq(m.pivot, 1e-9));
        assert!(contains(&[DVec2::ZERO, DVec2::new(4.0, 0.0), DVec2::new(0.0, 4.0)], DVec2::ONE));
        assert!(!contains(&[DVec2::ZERO, DVec2::new(4.0, 0.0), DVec2::new(0.0, 4.0)], DVec2::splat(3.0)));
    }
}
