//! The editor's frame around the viewports: theme, tool palette, top bar, viewport headers and
//! the status bar.

use std::time::{Duration, Instant};

use craft_ui::buttons::{ButtonVisuals, IconButton, IconButtonStyle};
use craft_ui::theme::WidgetPalette;
use egui::{Align2, Color32, CornerRadius, FontId, Painter, Pos2, Rect, RichText, Shape, Stroke, Vec2, pos2, vec2};
use mapcraft_geom::DVec3;
use mapcraft_render::{Camera, RenderMode, View};

use crate::app::MapCraftApp;
use crate::viewport::{Drag, Tool};

pub const BG: Color32 = Color32::from_rgb(18, 19, 22);
pub const PANEL: Color32 = Color32::from_rgb(29, 31, 35);
pub const HEADER: Color32 = Color32::from_rgb(36, 39, 44);
pub const WIDGET: Color32 = Color32::from_rgb(45, 49, 56);
pub const HOVER: Color32 = Color32::from_rgb(56, 61, 70);
pub const PRESSED: Color32 = Color32::from_rgb(68, 74, 84);
pub const LINE: Color32 = Color32::from_rgb(50, 54, 61);
pub const TEXT: Color32 = Color32::from_rgb(214, 218, 224);
pub const TEXT_WEAK: Color32 = Color32::from_rgb(132, 139, 150);
pub const ACCENT: Color32 = Color32::from_rgb(242, 140, 40);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(110, 66, 24);
pub const MIRROR: Color32 = Color32::from_rgb(90, 220, 200);
pub const AXIS: [Color32; 3] = [Color32::from_rgb(232, 76, 76), Color32::from_rgb(110, 200, 80), Color32::from_rgb(80, 140, 240)];

const ROTATE_HINT: &str = "it snaps to 15 degree steps while Snap is on";

/// How long a status message stays before the bar shows hints again.
const STATUS_TIME: Duration = Duration::from_secs(8);

pub fn apply_theme(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = PANEL;
    v.window_fill = PANEL;
    v.extreme_bg_color = BG;
    v.faint_bg_color = Color32::from_rgb(33, 35, 40);
    v.code_bg_color = BG;
    v.window_stroke = Stroke::new(1.0, LINE);
    v.window_corner_radius = CornerRadius::same(6);
    v.menu_corner_radius = CornerRadius::same(6);
    v.selection.bg_fill = ACCENT_DIM;
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.hyperlink_color = ACCENT;
    let palette = WidgetPalette { noninteractive: PANEL, inactive: WIDGET, hovered: HOVER, active: PRESSED, open: HOVER };
    palette.apply_to(&mut v.widgets, CornerRadius::same(4), Stroke::new(1.0, TEXT));
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(80, 86, 96));
    ctx.set_visuals_of(egui::Theme::Dark, v);
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = vec2(6.0, 4.0);
        s.spacing.button_padding = vec2(7.0, 3.0);
        s.spacing.interact_size.y = 22.0;
    });
}

fn tool_style() -> IconButtonStyle {
    let flat = |fill, icon| ButtonVisuals { fill, stroke: Stroke::NONE, icon };
    IconButtonStyle {
        normal: flat(Color32::TRANSPARENT, TEXT_WEAK),
        hovered: flat(HOVER, TEXT),
        pressed: Some(flat(PRESSED, TEXT)),
        selected: ButtonVisuals { fill: ACCENT_DIM, stroke: Stroke::new(1.0, ACCENT), icon: Color32::WHITE },
        corner_radius: CornerRadius::same(5),
        focus_stroke: Stroke::new(1.0, ACCENT),
    }
}

/// Painted icons, so they stay crisp at any scale and need no font or image assets.
pub mod icons {
    use super::*;

    fn line(p: &Painter, pts: &[Pos2], c: Color32) {
        p.add(Shape::line(pts.to_vec(), Stroke::new(1.6, c)));
    }

    fn at(r: Rect, x: f32, y: f32) -> Pos2 {
        // Icons are drawn on a 16 unit grid centered in the button.
        let s = r.width().min(r.height()) / 26.0;
        r.center() + vec2((x - 8.0) * s, (y - 8.0) * s)
    }

    pub fn select(p: &Painter, r: Rect, c: Color32) {
        let pts: Vec<Pos2> =
            [(3.0, 1.0), (3.0, 14.0), (6.5, 10.5), (9.0, 15.5), (11.0, 14.5), (8.5, 9.5), (13.0, 9.5)].iter().map(|(x, y)| at(r, *x, *y)).collect();
        p.add(Shape::convex_polygon(pts.clone(), c.gamma_multiply(0.25), Stroke::NONE));
        p.add(Shape::closed_line(pts, Stroke::new(1.4, c)));
    }

    fn arrow(p: &Painter, from: Pos2, to: Pos2, c: Color32) {
        let d = (to - from).normalized();
        let side = vec2(-d.y, d.x);
        let s = (to - from).length() * 0.3;
        line(p, &[from, to], c);
        line(p, &[to - d * s + side * s * 0.7, to, to - d * s - side * s * 0.7], c);
    }

    pub fn move_arrows(p: &Painter, r: Rect, c: Color32) {
        let m = at(r, 8.0, 8.0);
        for (x, y) in [(8.0, 1.5), (14.5, 8.0), (8.0, 14.5), (1.5, 8.0)] {
            arrow(p, m, at(r, x, y), c);
        }
    }

    pub fn extrude(p: &Painter, r: Rect, c: Color32) {
        let base = [at(r, 2.0, 11.0), at(r, 10.0, 11.0), at(r, 14.0, 14.5), at(r, 6.0, 14.5)];
        p.add(Shape::convex_polygon(base.to_vec(), c.gamma_multiply(0.35), Stroke::new(1.4, c)));
        arrow(p, at(r, 8.0, 12.5), at(r, 8.0, 1.5), ACCENT);
    }

    pub fn rotate(p: &Painter, r: Rect, c: Color32) {
        let pts: Vec<Pos2> = (0..=16)
            .map(|i| {
                let a = std::f32::consts::PI * (0.3 + 1.5 * i as f32 / 16.0);
                at(r, 8.0 + 6.0 * a.cos(), 8.0 - 6.0 * a.sin())
            })
            .collect();
        let tip = pts[pts.len() - 1];
        let before = pts[pts.len() - 3];
        line(p, &pts, c);
        arrow(p, before, tip, c);
    }

    pub fn scale(p: &Painter, r: Rect, c: Color32) {
        line(p, &[at(r, 2.0, 6.0), at(r, 2.0, 14.0), at(r, 10.0, 14.0), at(r, 10.0, 6.0), at(r, 2.0, 6.0)], c.gamma_multiply(0.7));
        arrow(p, at(r, 6.0, 10.0), at(r, 14.5, 1.5), c);
    }

    pub fn block(p: &Painter, r: Rect, c: Color32) {
        let (a, b, d, e) = (at(r, 8.0, 1.5), at(r, 14.0, 4.5), at(r, 8.0, 7.5), at(r, 2.0, 4.5));
        let (bb, db, eb) = (at(r, 14.0, 11.5), at(r, 8.0, 14.5), at(r, 2.0, 11.5));
        p.add(Shape::convex_polygon(vec![a, b, d, e], c.gamma_multiply(0.3), Stroke::NONE));
        line(p, &[a, b, d, e, a], c);
        line(p, &[e, eb, db, bb, b], c);
        line(p, &[d, db], c);
    }

    pub fn entity(p: &Painter, r: Rect, c: Color32) {
        let center = at(r, 8.0, 6.0);
        let s = r.width().min(r.height()) / 26.0;
        p.circle_stroke(center, 4.0 * s, Stroke::new(1.6, c));
        p.circle_filled(center, 1.6 * s, c);
        line(p, &[at(r, 5.2, 9.0), at(r, 8.0, 15.0), at(r, 10.8, 9.0)], c);
    }

    pub fn clip(p: &Painter, r: Rect, c: Color32) {
        line(p, &[at(r, 2.0, 2.0), at(r, 14.0, 2.0), at(r, 14.0, 14.0), at(r, 2.0, 14.0), at(r, 2.0, 2.0)], c.gamma_multiply(0.7));
        p.add(Shape::line_segment([at(r, 0.0, 15.0), at(r, 16.0, 1.0)], Stroke::new(2.0, ACCENT)));
    }

    pub fn vertex(p: &Painter, r: Rect, c: Color32) {
        let corners = [at(r, 3.0, 3.0), at(r, 13.0, 3.0), at(r, 13.0, 13.0), at(r, 3.0, 13.0)];
        line(p, &[corners[0], corners[1], corners[2], corners[3], corners[0]], c.gamma_multiply(0.6));
        let s = r.width().min(r.height()) / 26.0;
        for k in corners {
            p.rect_filled(Rect::from_center_size(k, Vec2::splat(4.0 * s)), 0.0, c);
        }
    }

    pub fn sculpt(p: &Painter, r: Rect, c: Color32) {
        let hill: Vec<Pos2> = (0..=12).map(|i| i as f32 / 12.0).map(|t| at(r, 1.0 + t * 14.0, 13.0 - 7.0 * (std::f32::consts::PI * t).sin().powi(2))).collect();
        line(p, &hill, c);
        p.circle_stroke(at(r, 8.0, 3.0), r.width().min(r.height()) / 26.0 * 2.2, Stroke::new(1.4, c));
    }

    pub fn face(p: &Painter, r: Rect, c: Color32) {
        let q = |x: f32, y: f32| at(r, 2.0 + x * 12.0 - y * 2.0, 3.0 + y * 10.0);
        for i in 0..3 {
            for j in 0..3 {
                if (i + j) % 2 == 0 {
                    let (x0, y0, x1, y1) = (i as f32 / 3.0, j as f32 / 3.0, (i + 1) as f32 / 3.0, (j + 1) as f32 / 3.0);
                    p.add(Shape::convex_polygon(vec![q(x0, y0), q(x1, y0), q(x1, y1), q(x0, y1)], c.gamma_multiply(0.55), Stroke::NONE));
                }
            }
        }
        line(p, &[q(0.0, 0.0), q(1.0, 0.0), q(1.0, 1.0), q(0.0, 1.0), q(0.0, 0.0)], c);
    }

    pub fn undo(p: &Painter, r: Rect, c: Color32) {
        let pts: Vec<Pos2> = (0..=10)
            .map(|i| {
                let a = std::f32::consts::PI * (0.15 + 1.1 * i as f32 / 10.0);
                at(r, 8.5 + 5.0 * a.cos(), 9.0 - 5.0 * a.sin())
            })
            .collect();
        let tip = pts[pts.len() - 1];
        line(p, &pts, c);
        let s = r.width().min(r.height()) / 26.0;
        line(p, &[tip + vec2(-2.5, -2.5) * s, tip, tip + vec2(2.5, -2.0) * s], c);
    }

    pub fn redo(p: &Painter, r: Rect, c: Color32) {
        let pts: Vec<Pos2> = (0..=10)
            .map(|i| {
                let a = std::f32::consts::PI * (0.85 - 1.1 * i as f32 / 10.0);
                at(r, 7.5 + 5.0 * a.cos(), 9.0 - 5.0 * a.sin())
            })
            .collect();
        let tip = pts[pts.len() - 1];
        line(p, &pts, c);
        let s = r.width().min(r.height()) / 26.0;
        line(p, &[tip + vec2(2.5, -2.5) * s, tip, tip + vec2(-2.5, -2.0) * s], c);
    }

    /// A flat disclosure triangle, pointing down when open.
    pub fn chevron(p: &Painter, r: Rect, c: Color32, open: bool) {
        let m = r.center();
        let pts = if open {
            vec![m + vec2(-4.0, -2.0), m + vec2(4.0, -2.0), m + vec2(0.0, 3.0)]
        } else {
            vec![m + vec2(-2.0, -4.0), m + vec2(3.0, 0.0), m + vec2(-2.0, 4.0)]
        };
        p.add(Shape::convex_polygon(pts, c, Stroke::NONE));
    }

    /// An open eye, or a closed one for hidden nodes.
    pub fn eye(p: &Painter, r: Rect, c: Color32, open: bool) {
        let m = r.center();
        let lid: Vec<Pos2> = (0..=12)
            .map(|i| {
                let t = i as f32 / 12.0 * std::f32::consts::PI;
                m + vec2(-6.0 * t.cos(), -3.5 * t.sin())
            })
            .collect();
        if open {
            let low: Vec<Pos2> = lid.iter().map(|q| pos2(q.x, 2.0 * m.y - q.y)).collect();
            p.add(Shape::line(lid, Stroke::new(1.2, c)));
            p.add(Shape::line(low, Stroke::new(1.2, c)));
            p.circle_filled(m, 1.8, c);
        } else {
            let low: Vec<Pos2> = lid.iter().map(|q| pos2(q.x, 2.0 * m.y - q.y + 1.0)).collect();
            p.add(Shape::line(low, Stroke::new(1.2, c)));
        }
    }

    /// What kind of node a scene tree row is.
    pub fn node_kind(p: &Painter, r: Rect, kind: &mapcraft_map::NodeKind, entity_color: Option<Color32>) {
        use mapcraft_map::NodeKind as K;
        let m = r.center();
        let stroke = |c| Stroke::new(1.2, c);
        match kind {
            K::World(_) => {
                p.circle_stroke(m, 5.0, stroke(TEXT_WEAK));
                p.line_segment([m + vec2(-5.0, 0.0), m + vec2(5.0, 0.0)], stroke(TEXT_WEAK));
            }
            K::Layer(_) => {
                for dy in [-3.0, 0.0, 3.0] {
                    p.line_segment([m + vec2(-5.0, dy), m + vec2(5.0, dy)], stroke(Color32::from_rgb(150, 170, 200)));
                }
            }
            K::Group(g) => {
                let c = if g.generator.is_some() { ACCENT } else { Color32::from_rgb(200, 180, 120) };
                p.rect_stroke(Rect::from_center_size(m + vec2(0.0, 1.0), vec2(10.0, 7.0)), 1.0, stroke(c), egui::StrokeKind::Middle);
                p.line_segment([m + vec2(-5.0, -3.5), m + vec2(-1.0, -3.5)], stroke(c));
                if g.mirror.is_some() {
                    p.line_segment([m + vec2(0.0, -4.5), m + vec2(0.0, 6.5)], stroke(MIRROR));
                }
            }
            K::Entity(_) => {
                let c = entity_color.unwrap_or(Color32::from_rgb(110, 210, 110));
                let pts = vec![m + vec2(0.0, -5.0), m + vec2(5.0, 0.0), m + vec2(0.0, 5.0), m + vec2(-5.0, 0.0)];
                p.add(Shape::convex_polygon(pts, c.gamma_multiply(0.35), stroke(c)));
            }
            K::Brush(_) => {
                p.rect_stroke(Rect::from_center_size(m, vec2(9.0, 9.0)), 1.0, stroke(TEXT_WEAK), egui::StrokeKind::Middle);
            }
            K::Prefab(_) => {
                p.circle_stroke(m + vec2(-2.0, 0.0), 3.0, stroke(Color32::from_rgb(120, 200, 230)));
                p.circle_stroke(m + vec2(2.5, 0.0), 3.0, stroke(Color32::from_rgb(120, 200, 230)));
            }
        }
    }

    /// Two chain links: the 2D views zoom together.
    pub fn link(p: &Painter, r: Rect, c: Color32) {
        let m = r.center();
        for dx in [-2.5, 2.5] {
            p.rect_stroke(Rect::from_center_size(m + vec2(dx, 0.0), vec2(7.0, 4.5)), 2.0, Stroke::new(1.3, c), egui::StrokeKind::Middle);
        }
    }

    pub fn maximize(p: &Painter, r: Rect, c: Color32) {
        line(p, &[at(r, 3.0, 3.0), at(r, 13.0, 3.0), at(r, 13.0, 13.0), at(r, 3.0, 13.0), at(r, 3.0, 3.0)], c);
    }

    pub fn restore(p: &Painter, r: Rect, c: Color32) {
        line(p, &[at(r, 5.0, 2.0), at(r, 14.0, 2.0), at(r, 14.0, 11.0)], c);
        line(p, &[at(r, 2.0, 5.0), at(r, 11.0, 5.0), at(r, 11.0, 14.0), at(r, 2.0, 14.0), at(r, 2.0, 5.0)], c);
    }
}

/// The world axes a 2D view shows across and up.
fn view_axes(view: View) -> Option<(usize, usize)> {
    match view {
        View::Top => Some((0, 1)),
        View::Front => Some((1, 2)),
        View::Side => Some((0, 2)),
        View::Perspective => None,
    }
}

impl MapCraftApp {
    pub(crate) fn tool_hint(&self) -> String {
        match &self.drag {
            Drag::None => {}
            Drag::Extrude { distance, mode, .. } => {
                let what = match mode {
                    mapcraft_engine::ExtrudeMode::Resize => "Extruding",
                    mapcraft_engine::ExtrudeMode::Split => "Splitting",
                    mapcraft_engine::ExtrudeMode::Stamp => "Stamping",
                };
                return format!("{what} {distance} units; release to finish");
            }
            Drag::Rotate { applied, .. } => return format!("Turned {applied} degrees; release to finish"),
            _ => return "Release to finish; Ctrl+Z undoes the whole drag".into(),
        }
        match self.tool {
            Tool::Select => {
                "Click to select, click again to pick the object behind; drag to move, drag the white handles to resize; Shift+drag a face extrudes".into()
            }
            Tool::Move => "Drag in a 2D view or along the arrows in the 3D view to move the selection".into(),
            Tool::Extrude => "Drag a highlighted face of a selected brush; Ctrl splits off a new brush, Ctrl+Alt stamps one".into(),
            Tool::Rotate => format!("Drag in a 2D view to turn the selection about its center, or drag a ring in the 3D view; {ROTATE_HINT}"),
            Tool::Scale => "Drag the white handles in a 2D view or the square handles in the 3D view to stretch the selection".into(),
            Tool::Block if self.plugin_shape.is_some() => "Click in a view to place it; its parameters are in the inspector".into(),
            Tool::Block => format!("Drag in a 2D view to draw a {}; its depth matches the last brush", shape_name(self.shape)),
            Tool::Entity => format!("Click in a view to place {}; pick the class in the Entities tab", self.entity_class),
            Tool::Clip if self.session.selected_brushes().is_empty() => "Select the brushes to clip first".into(),
            Tool::Clip if self.clip_points.len() == 2 => {
                format!("Enter clips, Esc cancels; yellow stays, red goes; {} picks which side to keep", self.chord("clip.next_side"))
            }
            Tool::Clip => "Drag a line, or click two points, in any view; in the 3D view the cut runs straight into the face".into(),
            Tool::Vertex => "Drag yellow corners, blue edge midpoints or green face centers in a 2D view".into(),
            Tool::Sculpt => "Drag on a displacement in the 3D view; Ctrl swaps raise and lower; faces picked with the Face tool limit it".into(),
            Tool::Face => "Click faces in the 3D view; Alt+click picks a material, Alt+right click applies it, with Shift it wraps the alignment".into(),
        }
    }

    pub(crate) fn tool_palette(&mut self, ui: &mut egui::Ui) {
        let style = tool_style();
        ui.add_space(4.0);
        type Icon = fn(&Painter, Rect, Color32);
        let tools: [(Tool, &str, &str, Icon); 11] = [
            (Tool::Select, "Select", "tool.select", icons::select),
            (Tool::Move, "Move", "tool.move", icons::move_arrows),
            (Tool::Extrude, "Extrude", "tool.extrude", icons::extrude),
            (Tool::Rotate, "Rotate", "tool.rotate", icons::rotate),
            (Tool::Scale, "Scale", "tool.scale", icons::scale),
            (Tool::Block, "Block", "tool.block", icons::block),
            (Tool::Entity, "Entity", "tool.entity", icons::entity),
            (Tool::Clip, "Clip", "tool.clip", icons::clip),
            (Tool::Vertex, "Vertex", "tool.vertex", icons::vertex),
            (Tool::Face, "Face", "tool.face", icons::face),
            (Tool::Sculpt, "Sculpt", "tool.sculpt", icons::sculpt),
        ];
        ui.vertical_centered(|ui| {
            for (tool, label, id, icon) in tools {
                let chord = self.chord(id);
                let tip = if chord.is_empty() { label.to_string() } else { format!("{label}  ({chord})") };
                let r = IconButton::new(label, Vec2::splat(32.0), &style).selected(self.tool == tool).show(ui, |ui, rect, c| icon(ui.painter(), rect, c));
                if r.on_hover_text(tip).clicked() {
                    self.run_action(id);
                }
                if tool == Tool::Scale {
                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(4.0);
                }
            }
        });
    }

    pub(crate) fn segmented<T: PartialEq + Copy>(ui: &mut egui::Ui, current: &mut T, options: &[(T, &str, &str)]) -> bool {
        let mut changed = false;
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.x = 1.0;
            for (value, label, tip) in options {
                let on = *current == *value;
                let text = if on { RichText::new(*label).color(Color32::WHITE) } else { RichText::new(*label) };
                let button = egui::Button::new(text).fill(if on { ACCENT_DIM } else { WIDGET }).corner_radius(CornerRadius::ZERO);
                if ui.add(button).on_hover_text(*tip).clicked() && !on {
                    *current = *value;
                    changed = true;
                }
            }
        });
        changed
    }

    pub(crate) fn top_bar(&mut self, ui: &mut egui::Ui) {
        let style = tool_style();
        ui.horizontal(|ui| {
            let undo = format!("Undo {}", self.session.undo_label().unwrap_or_default());
            let r = ui
                .add_enabled_ui(self.session.can_undo(), |ui| {
                    IconButton::new("Undo", Vec2::splat(24.0), &style).show(ui, |ui, r, c| icons::undo(ui.painter(), r, c))
                })
                .inner;
            if r.on_hover_text(format!("{}  ({})", undo.trim(), self.chord("edit.undo"))).clicked() {
                self.run_action("edit.undo");
            }
            let redo = format!("Redo {}", self.session.redo_label().unwrap_or_default());
            let r = ui
                .add_enabled_ui(self.session.can_redo(), |ui| {
                    IconButton::new("Redo", Vec2::splat(24.0), &style).show(ui, |ui, r, c| icons::redo(ui.painter(), r, c))
                })
                .inner;
            if r.on_hover_text(format!("{}  ({})", redo.trim(), self.chord("edit.redo"))).clicked() {
                self.run_action("edit.redo");
            }
            ui.separator();

            use mapcraft_engine::SelectMode as M;
            let modes = [
                (M::Groups, "Groups", "Clicks select whole groups"),
                (M::Objects, "Objects", "Clicks select entities and loose brushes, ignoring groups"),
                (M::Solids, "Solids", "Clicks select single brushes"),
            ];
            Self::segmented(ui, &mut self.session.select_mode, &modes);
            ui.separator();

            ui.label(RichText::new("Grid").color(TEXT_WEAK));
            if ui.small_button("-").on_hover_text(format!("Smaller grid  ({})", self.chord("view.grid_smaller"))).clicked() {
                self.run_action("view.grid_smaller");
            }
            ui.label(RichText::new(format!("{}", self.session.grid)).strong().monospace());
            if ui.small_button("+").on_hover_text(format!("Larger grid  ({})", self.chord("view.grid_larger"))).clicked() {
                self.run_action("view.grid_larger");
            }
            let view = self.prefs.get().view.clone();
            for (on, label, id, tip) in [
                (view.snap_grid, "Snap", "view.snap_grid", "Snap to the grid"),
                (view.snap_vertices, "Corners", "view.snap_vertices", "Snap dragged points to nearby brush corners and entity origins"),
            ] {
                let text = RichText::new(label).color(if on { Color32::WHITE } else { TEXT_WEAK });
                if ui.add(egui::Button::new(text).selected(on)).on_hover_text(format!("{tip}  ({})", self.chord(id))).clicked() {
                    self.run_action(id);
                }
            }
            let lock = self.session.texture_lock;
            let lock_text = RichText::new("Texture lock").color(if lock { Color32::WHITE } else { TEXT_WEAK });
            let r = ui.add(egui::Button::new(lock_text).selected(lock)).on_hover_text("Keep textures fixed to brushes as they move");
            if r.clicked() {
                self.run_action("tools.toggle_texture_lock");
            }
            ui.separator();
            self.material_chip(ui);
            self.tool_options(ui);

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.run_button(ui);
                if ui.button("Check").on_hover_text(format!("Check the map for problems  ({})", self.chord("tools.check_problems"))).clicked() {
                    self.run_action("tools.check_problems");
                }
            });
        });
    }

    fn material_chip(&mut self, ui: &mut egui::Ui) {
        let name = if self.session.tex.material.is_empty() { "No material".to_string() } else { self.session.tex.material.clone() };
        let thumb = self.project.material(&self.session.tex.material).cloned().and_then(|m| self.thumbs.get(ui.ctx(), &m).map(|t| t.id()));
        ui.label(RichText::new("Material").color(TEXT_WEAK));
        let r = ui
            .horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(20.0), egui::Sense::hover());
                match thumb {
                    Some(id) => {
                        ui.painter().image(id, rect, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
                    }
                    None => {
                        ui.painter().rect_filled(rect, 3.0, WIDGET);
                    }
                }
                ui.painter().rect_stroke(rect, 3.0, Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
                ui.add(egui::Label::new(RichText::new(name).color(TEXT)).truncate().sense(egui::Sense::click()))
            })
            .inner;
        if r.on_hover_text(format!("The material new faces get; click to browse  ({})", self.chord("panel.materials"))).clicked() {
            self.run_action("panel.materials");
        }
    }

    fn tool_options(&mut self, ui: &mut egui::Ui) {
        if self.tool == Tool::Block {
            ui.separator();
            use mapcraft_engine::Shape as S;
            let arch = S::Arch { sides: 8, arc: 180.0, start: 0.0, thickness: 16.0 };
            let shapes =
                [(S::Block, "Box"), (S::Wedge, "Wedge"), (S::Cylinder(8), "Cylinder"), (S::Spike(4), "Spike"), (S::Sphere(8), "Sphere"), (arch, "Arch")];
            ui.label(RichText::new(self.chord("shape.next")).color(TEXT_WEAK).small()).on_hover_text("Next shape");
            let generators = self.generator_shapes();
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.x = 1.0;
                let picked = |label: &str, on: bool, ui: &mut egui::Ui| {
                    let text = if on { RichText::new(label).color(Color32::WHITE) } else { RichText::new(label) };
                    ui.add(egui::Button::new(text).fill(if on { ACCENT_DIM } else { WIDGET }).corner_radius(CornerRadius::ZERO)).clicked() && !on
                };
                for (shape, label) in shapes {
                    let same = self.plugin_shape.is_none() && std::mem::discriminant(&self.shape) == std::mem::discriminant(&shape);
                    if picked(label, same, ui) {
                        self.shape = shape;
                        self.pick_plugin_shape(None);
                    }
                }
                for (plugin, command) in generators {
                    let same = self.plugin_shape.as_ref().is_some_and(|s| s.plugin == plugin && s.command.id == command.id);
                    if picked(&command.label, same, ui) {
                        self.pick_plugin_shape(Some((plugin, command)));
                    }
                }
            });
            if self.plugin_shape.is_some() {
                return;
            }
            match &mut self.shape {
                S::Cylinder(n) | S::Spike(n) => {
                    ui.add(egui::DragValue::new(n).range(3..=64).prefix("sides "));
                }
                S::Sphere(n) => {
                    ui.add(egui::DragValue::new(n).range(4..=32).prefix("sides "));
                }
                S::Arch { sides, arc, start, thickness } => {
                    ui.add(egui::DragValue::new(sides).range(1..=64).prefix("sides "));
                    ui.add(egui::DragValue::new(arc).range(1.0..=360.0).prefix("arc ").suffix("°"));
                    ui.add(egui::DragValue::new(start).range(0.0..=360.0).prefix("start ").suffix("°"));
                    ui.add(egui::DragValue::new(thickness).range(1.0..=4096.0).prefix("wall "));
                }
                S::Block | S::Wedge => {}
            }
        }
        if self.tool == Tool::Sculpt {
            ui.separator();
            use mapcraft_engine::SculptMode as M;
            let modes = [
                (M::Raise, "Raise", "Pull the surface up"),
                (M::Lower, "Lower", "Push the surface down"),
                (M::Smooth, "Smooth", "Even out bumps"),
                (M::Flatten, "Flatten", "Level toward where the stroke began"),
                (M::Noise, "Noise", "Roughen the surface"),
            ];
            Self::segmented(ui, &mut self.sculpt.0, &modes);
            ui.add(egui::DragValue::new(&mut self.sculpt.1).range(8.0..=4096.0).speed(4.0).prefix("radius "));
            ui.add(egui::DragValue::new(&mut self.sculpt.2).range(1.0..=4096.0).speed(4.0).prefix("strength "));
        }
        if self.tool == Tool::Clip {
            ui.separator();
            use mapcraft_engine::ClipKeep as K;
            ui.label(RichText::new(self.chord("clip.next_side")).color(TEXT_WEAK).small()).on_hover_text("Next side to keep");
            let keeps = [(K::Both, "Both", "Keep both halves"), (K::Front, "Front", "Keep the front half"), (K::Back, "Back", "Keep the back half")];
            Self::segmented(ui, &mut self.clip_keep, &keeps);
        }
    }

    fn run_button(&mut self, ui: &mut egui::Ui) {
        let profiles: Vec<String> = self.project.sdk.iter().flat_map(|s| s.manifest.compile.iter().map(|p| p.name.clone())).collect();
        let running = self.compile.is_some();
        let text = RichText::new(if running { "Running…" } else { "Run" }).color(Color32::BLACK).strong();
        let tip = if profiles.is_empty() {
            "This SDK has no compile profiles".to_string()
        } else {
            format!("Compile and play the map; {} runs the last profile again", self.chord("run.again"))
        };
        let r = ui.add_enabled(!profiles.is_empty(), egui::Button::new(text).fill(ACCENT).min_size(vec2(64.0, 22.0))).on_hover_text(tip);
        egui::Popup::menu(&r).show(|ui| {
            for (i, name) in profiles.iter().enumerate() {
                if ui.add_enabled(!running, egui::Button::new(name)).clicked() {
                    self.start_compile(i);
                }
            }
            if running && ui.button("Cancel").clicked() {
                self.cancel_compile();
            }
        });
    }

    /// The strip above a viewport: view picker, axes and zoom, render mode, maximize.
    pub(crate) fn pane_header(&mut self, ui: &mut egui::Ui, index: usize, header: Rect, active: bool) {
        ui.painter().rect_filled(header, 0.0, HEADER);
        let mut header_ui =
            ui.new_child(egui::UiBuilder::new().max_rect(header.shrink2(vec2(4.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        let style = tool_style();
        let Some(cam) = self.panes.get(index).map(|p| p.camera) else { return };
        let ui = &mut header_ui;
        let title = RichText::new(short_label(cam.view)).color(if active { ACCENT } else { TEXT }).strong();
        ui.menu_button(title, |ui| {
            for v in View::ALL {
                if ui.selectable_label(cam.view == v, v.label()).clicked()
                    && let Some(p) = self.panes.get_mut(index)
                {
                    p.camera = Camera { view: v, ..p.camera };
                }
            }
        });
        match view_axes(cam.view) {
            Some((a, b)) => {
                for axis in [a, b] {
                    ui.label(RichText::new(["x", "y", "z"][axis]).color(AXIS[axis]).monospace());
                }
                ui.label(RichText::new(format!("1:{:.2}", 1.0 / cam.zoom.max(1e-6))).color(TEXT_WEAK).monospace()).on_hover_text("Map units per pixel");
                let step = self.view_grid(index);
                if step > self.session.grid {
                    let tip = format!("Zoomed out, this view draws and snaps to a {step} grid; zoom in for {}", self.session.grid);
                    ui.label(RichText::new(format!("grid {step}")).color(ACCENT).monospace()).on_hover_text(tip);
                }
            }
            None => {
                let mode = self.render_mode;
                let label = match mode {
                    RenderMode::Textured => "Textured",
                    RenderMode::Flat => "Flat",
                    RenderMode::Wireframe => "Wireframe",
                };
                ui.menu_button(RichText::new(label).color(TEXT_WEAK), |ui| {
                    for (m, l) in [(RenderMode::Textured, "Textured"), (RenderMode::Flat, "Flat"), (RenderMode::Wireframe, "Wireframe")] {
                        if ui.selectable_label(mode == m, l).clicked() {
                            self.render_mode = m;
                        }
                    }
                });
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let maximized = self.maximized.is_some();
            let linked = self.prefs.get().view.link_zoom;
            let label = if maximized { "Restore" } else { "Maximize" };
            let r = IconButton::new(label, Vec2::splat(18.0), &style)
                .show(ui, |ui, r, c| if maximized { icons::restore(ui.painter(), r, c) } else { icons::maximize(ui.painter(), r, c) });
            if r.on_hover_text(format!("{label}  ({})", self.chord("view.maximize"))).clicked() {
                self.maximized = if maximized { None } else { Some(index) };
            }
            if cam.is_2d() {
                let r = IconButton::new("Link zoom", Vec2::splat(18.0), &style).selected(linked).show(ui, |ui, r, c| icons::link(ui.painter(), r, c));
                let tip = if linked { "2D views zoom together; click to zoom each on its own" } else { "Click to zoom all 2D views together" };
                if r.on_hover_text(tip).clicked() {
                    self.run_action("view.link_zoom");
                }
            }
        });
    }

    /// Which way the world axes point, drawn in a 3D view's corner; clicking an axis looks down it.
    pub(crate) fn axis_widget(&mut self, ui: &egui::Ui, index: usize, rect: Rect) {
        let Some(cam) = self.panes.get(index).map(|p| p.camera) else { return };
        let center = pos2(rect.left() + 34.0, rect.bottom() - 34.0);
        let (right, forward) = (cam.right(), cam.forward());
        let up = right.cross(forward);
        let mut axes: Vec<(f64, usize, Vec2)> =
            [DVec3::X, DVec3::Y, DVec3::Z].iter().enumerate().map(|(i, a)| (a.dot(forward), i, vec2(a.dot(right) as f32, -a.dot(up) as f32) * 22.0)).collect();
        // Axes pointing into the screen are drawn first so nearer ones overlap them.
        axes.sort_by(|a, b| b.0.total_cmp(&a.0));
        let p = ui.painter_at(rect);
        p.circle_filled(center, 30.0, Color32::from_black_alpha(90));
        let mut clicked = None;
        for (depth, i, d) in axes {
            let r = ui.interact(Rect::from_center_size(center + d, vec2(16.0, 16.0)), ui.id().with(("axis-widget", index, i)), egui::Sense::click());
            let c = if depth > 0.3 { AXIS[i].gamma_multiply(0.5) } else { AXIS[i] };
            p.line_segment([center, center + d], Stroke::new(2.0, c));
            p.circle_filled(center + d, 7.0, c);
            if r.hovered() {
                p.circle_stroke(center + d, 8.5, Stroke::new(1.5, Color32::WHITE));
            }
            p.text(center + d, Align2::CENTER_CENTER, ["X", "Y", "Z"][i], FontId::proportional(10.0), Color32::BLACK);
            if r.on_hover_text("Look down this axis; click again to look from the other side").clicked() {
                clicked = Some(i);
            }
        }
        if let Some(axis) = clicked {
            self.look_along_axis(index, axis);
        }
    }

    /// Turns view `index` to look down world `axis` at what it was looking at, or back up the axis
    /// when it already looks down it.
    fn look_along_axis(&mut self, index: usize, axis: usize) {
        let Some(cam) = self.panes.get(index).map(|p| p.camera) else { return };
        let dist = self.pick(cam.position, cam.forward()).map_or(1024.0, |(_, t)| t);
        let pivot = cam.position + cam.forward() * dist;
        let down = cam.forward().dot(-[DVec3::X, DVec3::Y, DVec3::Z][axis.min(2)]) > 0.999;
        // Pitch stops short of 90 degrees, where the view's up direction is undefined.
        let (yaw, pitch) = match (axis, down) {
            (0, false) => (180.0, 0.0),
            (0, true) => (0.0, 0.0),
            (1, false) => (-90.0, 0.0),
            (1, true) => (90.0, 0.0),
            (_, false) => (90.0, -89.9),
            (_, true) => (90.0, 89.9),
        };
        if let Some(p) = self.panes.get_mut(index) {
            p.camera.yaw = yaw;
            p.camera.pitch = pitch;
            p.camera.position = pivot - p.camera.forward() * dist;
        }
    }

    fn selection_summary(&self) -> String {
        let sel = &self.session.selection;
        match sel.len() {
            0 => "No selection".into(),
            1 => sel.iter().next().and_then(|id| self.session.map.get(*id)).map_or_else(String::new, |n| n.kind.label()),
            n => format!("{n} objects"),
        }
    }

    pub(crate) fn status_bar(&mut self, ui: &mut egui::Ui) {
        if self.status != self.status_shown.0 {
            self.status_shown = (self.status.clone(), Instant::now());
        }
        let fresh = !self.status.is_empty() && self.status_shown.1.elapsed() < STATUS_TIME;
        if fresh {
            ui.ctx().request_repaint_after(STATUS_TIME);
        }
        ui.horizontal(|ui| {
            let item = |ui: &mut egui::Ui, name: &str, value: String| {
                ui.label(RichText::new(name).color(TEXT_WEAK));
                ui.label(RichText::new(value).color(TEXT));
                ui.add_space(8.0);
            };
            ui.label(RichText::new(self.selection_summary()).color(TEXT).strong());
            ui.add_space(8.0);
            if let Some(p) = self.cursor_world {
                item(ui, "Cursor", format!("{:.0} {:.0} {:.0}", p.x, p.y, p.z));
            }
            item(ui, "Grid", format!("{}", self.session.grid));
            item(ui, "Mode", format!("{:?}", self.session.select_mode).to_lowercase());
            item(ui, "Texture lock", if self.session.texture_lock { "on".into() } else { "off".into() });
            item(ui, "SDK", self.project.sdk.as_ref().map_or("none".to_string(), |s| s.manifest.name.clone()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.selectable_label(self.show_bottom, "Output").on_hover_text(self.chord("panel.output")).clicked() {
                    self.run_action("panel.output");
                }
                ui.separator();
                let (text, color) = if fresh { (self.status.clone(), TEXT) } else { (self.tool_hint(), TEXT_WEAK) };
                ui.add(egui::Label::new(RichText::new(text).color(color)).truncate());
            });
        });
    }
}

fn short_label(view: View) -> &'static str {
    match view {
        View::Perspective => "3D",
        View::Top => "Top",
        View::Front => "Front",
        View::Side => "Side",
    }
}

/// The block shape after `shape` in the toolbar's order.
pub(crate) fn next_shape(shape: mapcraft_engine::Shape) -> mapcraft_engine::Shape {
    use mapcraft_engine::Shape as S;
    match shape {
        S::Block => S::Wedge,
        S::Wedge => S::Cylinder(8),
        S::Cylinder(_) => S::Spike(4),
        S::Spike(_) => S::Sphere(8),
        S::Sphere(_) => S::Arch { sides: 8, arc: 180.0, start: 0.0, thickness: 16.0 },
        S::Arch { .. } => S::Block,
    }
}

fn shape_name(shape: mapcraft_engine::Shape) -> &'static str {
    use mapcraft_engine::Shape as S;
    match shape {
        S::Block => "box",
        S::Wedge => "wedge",
        S::Cylinder(_) => "cylinder",
        S::Spike(_) => "spike",
        S::Sphere(_) => "sphere",
        S::Arch { .. } => "arch",
    }
}
