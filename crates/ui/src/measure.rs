//! Size labels on boxes being drawn or resized and on the selection, and highlights on the faces a drag moves.

use egui::{Align2, Color32, FontId, Pos2, Rect, Stroke, Vec2};
use mapcraft_geom::{Aabb, DVec3, Plane};

use crate::app::MapCraftApp;
use crate::viewport::{Drag, Tool, dvec};

const AXIS_NAMES: [&str; 3] = ["X", "Y", "Z"];
/// The renderer's selection red, `SELECTED_EDGE`.
const SELECTED: Color32 = Color32::from_rgb(255, 26, 26);

/// A length in map units, without decimals when it is whole.
pub(crate) fn units(v: f64) -> String {
    if (v - v.round()).abs() < 1e-6 { format!("{}", v.round()) } else { format!("{v:.2}").trim_end_matches('0').to_string() }
}

/// The face of `b` on the high or low side of `axis`.
fn box_face(b: &Aabb, axis: usize, high: bool) -> Vec<DVec3> {
    let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
    let at = if high { b.max[axis] } else { b.min[axis] };
    [(b.min[u], b.min[v]), (b.max[u], b.min[v]), (b.max[u], b.max[v]), (b.min[u], b.max[v])]
        .iter()
        .map(|(x, y)| {
            let mut p = DVec3::ZERO;
            p[axis] = at;
            p[u] = *x;
            p[v] = *y;
            p
        })
        .collect()
}

pub(crate) fn label(painter: &egui::Painter, at: Pos2, align: Align2, text: String) {
    let galley = painter.layout_no_wrap(text, FontId::proportional(12.0), Color32::WHITE);
    let rect = align.anchor_size(at, galley.size()).expand2(Vec2::new(4.0, 2.0));
    painter.rect_filled(rect, 2.0, Color32::from_black_alpha(200));
    painter.galley(rect.min + Vec2::new(4.0, 2.0), galley, Color32::WHITE);
}

impl MapCraftApp {
    /// The box whose size is shown: a block being drawn, the selection being resized or extruded, or the selection the select tools outline.
    pub(crate) fn measured_box(&self) -> Option<Aabb> {
        match &self.drag {
            Drag::Block { start, end, .. } => Some(self.block_box(*start, *end)),
            Drag::Resize { applied, .. } | Drag::Scale { applied, .. } => Some(*applied),
            Drag::Extrude { created, .. } => created.iter().filter_map(|id| self.node_bounds(*id)).chain(self.selection_bounds()).reduce(|a, b| a.union(&b)),
            _ => {
                self.block_3d_box().or_else(|| self.selection_bounds().filter(|_| matches!(self.tool, Tool::Select | Tool::Move | Tool::Rotate | Tool::Scale)))
            }
        }
    }

    /// The faces the current drag moves or transfers onto, or an extrude would move, as world polygons.
    pub(crate) fn manipulated_faces(&self) -> Vec<Vec<DVec3>> {
        match &self.drag {
            Drag::Resize { view, handle, applied, .. } => {
                let Some(cam) = self.panes.get(*view).map(|p| p.camera) else { return Vec::new() };
                let (r, u, _) = cam.view.axes();
                [(r, handle.0), (u, handle.1)]
                    .iter()
                    .filter(|(_, side)| *side != 0)
                    .map(|(axis, side)| box_face(applied, axis.abs().max_position(), *side > 0))
                    .collect()
            }
            Drag::Scale { axis, low, applied, .. } => vec![box_face(applied, *axis, !*low)],
            Drag::Extrude { faces, created, offset, .. } => {
                // Where each face is now, on its own brush or on the one a split or stamp made.
                let moved: Vec<Plane> = faces
                    .iter()
                    .filter_map(|(_, b, face)| b.faces.iter().find(|f| f.id == *face)?.plane())
                    .map(|p| Plane { normal: p.normal, dist: p.dist + offset.dot(p.normal) })
                    .collect();
                faces
                    .iter()
                    .map(|(id, ..)| *id)
                    .chain(created.iter().copied())
                    .filter_map(|id| {
                        let brush = self.session.map.get(id)?.kind.brush()?;
                        let solid = brush.solid().ok()?;
                        brush
                            .faces
                            .iter()
                            .zip(solid.faces)
                            .find_map(|(f, w)| w.filter(|_| f.plane().is_some_and(|p| moved.iter().any(|m| p.approx_eq(m, 1e-3)))))
                    })
                    .map(|w| w.0)
                    .collect()
            }
            Drag::Transfer { done, .. } => {
                let brushes = done.iter().map(|(id, _)| *id).collect();
                mapcraft_engine::face_polygons(&self.session.map, &brushes).into_iter().filter(|(p, ..)| done.contains(p)).map(|(.., w)| w).collect()
            }
            Drag::None => self.extrude_hover.as_ref().map(|h| h.polygons.clone()).unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    pub(crate) fn draw_measurements(&self, ui: &egui::Ui, index: usize, rect: Rect) {
        let Some(cam) = self.panes.get(index).map(|p| p.camera) else { return };
        let size = dvec(rect.size());
        let painter = ui.painter_at(rect);
        let screen = |p: DVec3| cam.project(p, size).map(|q| rect.min + Vec2::new(q.x as f32, q.y as f32));
        for poly in self.manipulated_faces() {
            let Some(pts) = poly.iter().map(|p| screen(*p)).collect::<Option<Vec<Pos2>>>() else { continue };
            let width = if cam.is_2d() { 3.0 } else { 2.0 };
            painter.add(egui::Shape::convex_polygon(pts, SELECTED.gamma_multiply(0.35), Stroke::new(width, SELECTED)));
        }
        let Some(b) = self.measured_box() else { return };
        let extent = b.max - b.min;
        if cam.is_2d() {
            let (r, u, _) = cam.view.axes();
            let (Some(a), Some(c)) = (screen(b.min), screen(b.max)) else { return };
            let (lo, hi) = (a.min(c), a.max(c));
            let (ri, ui_) = (r.abs().max_position(), u.abs().max_position());
            label(&painter, Pos2::new((lo.x + hi.x) / 2.0, hi.y + 14.0), Align2::CENTER_TOP, format!("{}: {}", AXIS_NAMES[ri], units(extent[ri])));
            label(&painter, Pos2::new(lo.x - 14.0, (lo.y + hi.y) / 2.0), Align2::RIGHT_CENTER, format!("{}: {}", AXIS_NAMES[ui_], units(extent[ui_])));
            return;
        }
        let Some(c) = screen(b.center()) else { return };
        let text = |axis: usize| format!("{}: {}", AXIS_NAMES[axis], units(extent[axis]));
        // X and Y label the bottom edges meeting at the corner nearest the camera.
        let near = |axis: usize| if cam.position[axis] > b.center()[axis] { b.max[axis] } else { b.min[axis] };
        let corner = DVec3::new(near(0), near(1), b.min.z);
        for axis in 0..2 {
            let mut mid = corner;
            mid[axis] = b.center()[axis];
            if let Some(p) = screen(mid) {
                label(&painter, p + (p - c).normalized() * 14.0, Align2::CENTER_CENTER, text(axis));
            }
        }
        // Z labels the vertical edge on the far side of the outline from that corner.
        let Some(near_x) = screen(corner).map(|p| p.x) else { return };
        let edges = [(b.min.x, b.min.y), (b.min.x, b.max.y), (b.max.x, b.min.y), (b.max.x, b.max.y)]
            .into_iter()
            .filter(|(x, y)| (*x, *y) != (corner.x, corner.y))
            .filter_map(|(x, y)| screen(DVec3::new(x, y, b.center().z)));
        let side = if near_x < c.x { edges.max_by(|a, b| a.x.total_cmp(&b.x)) } else { edges.min_by(|a, b| a.x.total_cmp(&b.x)) };
        if let Some(p) = side {
            let (at, align) = if p.x >= c.x { (p + Vec2::new(10.0, 0.0), Align2::LEFT_CENTER) } else { (p - Vec2::new(10.0, 0.0), Align2::RIGHT_CENTER) };
            label(&painter, at, align, text(2));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_lengths_have_no_decimals() {
        assert_eq!(units(128.0), "128");
        assert_eq!(units(12.5), "12.5");
        assert_eq!(units(0.25), "0.25");
    }

    #[test]
    fn faces_lie_on_their_side() {
        let b = Aabb::new(DVec3::ZERO, DVec3::new(1.0, 2.0, 3.0));
        assert!(box_face(&b, 1, true).iter().all(|p| p.y == 2.0));
        assert!(box_face(&b, 2, false).iter().all(|p| p.z == 0.0));
    }
}
