//! The editor's frame around the viewports: theme, tool palette, top bar, viewport headers and
//! the status bar.

use std::time::{Duration, Instant};

use craft_ui::buttons::{ButtonVisuals, IconButton, IconButtonStyle};
use craft_ui::theme::WidgetPalette;
use egui::{Align2, Color32, CornerRadius, FontId, Painter, Pos2, Rect, RichText, Shape, Stroke, Vec2, pos2, vec2};
use mapcraft_geom::DVec3;
use mapcraft_render::{Camera, RenderMode, View};

use crate::app::{LeftTab, MapCraftApp};
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
pub const AXIS: [Color32; 3] = [Color32::from_rgb(232, 76, 76), Color32::from_rgb(110, 200, 80), Color32::from_rgb(80, 140, 240)];

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
        if !matches!(self.drag, Drag::None) {
            return "Release to finish; Ctrl+Z undoes the whole drag".into();
        }
        match self.tool {
            Tool::Select => "Click to select, click again to pick the object behind; drag to move, drag the white handles to resize, Ctrl+click adds".into(),
            Tool::Block => format!("Drag in a 2D view to draw a {}; its depth matches the last brush", shape_name(self.shape)),
            Tool::Entity => format!("Click in a view to place {}; pick the class in the Entities tab", self.entity_class),
            Tool::Clip if self.clip_points.len() == 2 => "Enter clips, Esc cancels; the toolbar picks which side to keep".into(),
            Tool::Clip => "Click two points in a 2D view to draw the clip line".into(),
            Tool::Vertex => "Drag yellow corners, blue edge midpoints or green face centers in a 2D view".into(),
            Tool::Face => "Click faces in the 3D view; Alt+click picks a material, Alt+right click applies it, with Shift it wraps the alignment".into(),
        }
    }

    pub(crate) fn tool_palette(&mut self, ui: &mut egui::Ui) {
        let style = tool_style();
        ui.add_space(4.0);
        type Icon = fn(&Painter, Rect, Color32);
        let tools: [(Tool, &str, &str, Icon); 6] = [
            (Tool::Select, "Select", "tool.select", icons::select),
            (Tool::Block, "Block", "tool.block", icons::block),
            (Tool::Entity, "Entity", "tool.entity", icons::entity),
            (Tool::Clip, "Clip", "tool.clip", icons::clip),
            (Tool::Vertex, "Vertex", "tool.vertex", icons::vertex),
            (Tool::Face, "Face", "tool.face", icons::face),
        ];
        ui.vertical_centered(|ui| {
            for (tool, label, id, icon) in tools {
                let chord = self.chord(id);
                let tip = if chord.is_empty() { label.to_string() } else { format!("{label}  ({chord})") };
                let r = IconButton::new(label, Vec2::splat(32.0), &style).selected(self.tool == tool).show(ui, |ui, rect, c| icon(ui.painter(), rect, c));
                if r.on_hover_text(tip).clicked() {
                    self.run_action(id);
                }
            }
        });
    }

    fn segmented<T: PartialEq + Copy>(ui: &mut egui::Ui, current: &mut T, options: &[(T, &str, &str)]) -> bool {
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
                if ui.button("Check").on_hover_text("Check the map for problems").clicked() {
                    self.check_problems();
                }
            });
        });
    }

    fn material_chip(&mut self, ui: &mut egui::Ui) {
        let name = if self.session.tex.material.is_empty() { "No material".to_string() } else { self.session.tex.material.clone() };
        let thumb = self.project.material(&self.session.tex.material).cloned().and_then(|m| self.thumbs.get(ui.ctx(), &m).map(|t| t.id()));
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
        if r.on_hover_text("The material new faces get; click to browse").clicked() {
            self.left_tab = LeftTab::Materials;
        }
    }

    fn tool_options(&mut self, ui: &mut egui::Ui) {
        if self.tool == Tool::Block {
            ui.separator();
            use mapcraft_engine::Shape as S;
            let arch = S::Arch { sides: 8, arc: 180.0, start: 0.0, thickness: 16.0 };
            let shapes =
                [(S::Block, "Box"), (S::Wedge, "Wedge"), (S::Cylinder(8), "Cylinder"), (S::Spike(4), "Spike"), (S::Sphere(8), "Sphere"), (arch, "Arch")];
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.x = 1.0;
                for (shape, label) in shapes {
                    let same = std::mem::discriminant(&self.shape) == std::mem::discriminant(&shape);
                    let text = if same { RichText::new(label).color(Color32::WHITE) } else { RichText::new(label) };
                    let button = egui::Button::new(text).fill(if same { ACCENT_DIM } else { WIDGET }).corner_radius(CornerRadius::ZERO);
                    if ui.add(button).clicked() && !same {
                        self.shape = shape;
                    }
                }
            });
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
        if self.tool == Tool::Clip {
            ui.separator();
            use mapcraft_engine::ClipKeep as K;
            let keeps = [(K::Both, "Both", "Keep both halves"), (K::Front, "Front", "Keep the front half"), (K::Back, "Back", "Keep the back half")];
            Self::segmented(ui, &mut self.clip_keep, &keeps);
        }
    }

    fn run_button(&mut self, ui: &mut egui::Ui) {
        let profiles: Vec<String> = self.project.sdk.iter().flat_map(|s| s.manifest.compile.iter().map(|p| p.name.clone())).collect();
        let running = self.compile.is_some();
        let text = RichText::new(if running { "Running…" } else { "Run" }).color(Color32::BLACK).strong();
        let tip = if profiles.is_empty() { "This SDK has no compile profiles" } else { "Compile and play the map" };
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
            let label = if maximized { "Restore" } else { "Maximize" };
            let r = IconButton::new(label, Vec2::splat(18.0), &style)
                .show(ui, |ui, r, c| if maximized { icons::restore(ui.painter(), r, c) } else { icons::maximize(ui.painter(), r, c) });
            if r.on_hover_text(label).clicked() {
                self.maximized = if maximized { None } else { Some(index) };
            }
        });
    }

    /// Which way the world axes point, drawn in a 3D view's corner.
    pub(crate) fn axis_widget(&self, ui: &egui::Ui, cam: &Camera, rect: Rect) {
        let center = pos2(rect.left() + 34.0, rect.bottom() - 34.0);
        let (right, forward) = (cam.right(), cam.forward());
        let up = right.cross(forward);
        let mut axes: Vec<(f64, usize, Vec2)> =
            [DVec3::X, DVec3::Y, DVec3::Z].iter().enumerate().map(|(i, a)| (a.dot(forward), i, vec2(a.dot(right) as f32, -a.dot(up) as f32) * 22.0)).collect();
        // Axes pointing into the screen are drawn first so nearer ones overlap them.
        axes.sort_by(|a, b| b.0.total_cmp(&a.0));
        let p = ui.painter_at(rect);
        p.circle_filled(center, 30.0, Color32::from_black_alpha(90));
        for (depth, i, d) in axes {
            let c = if depth > 0.3 { AXIS[i].gamma_multiply(0.5) } else { AXIS[i] };
            p.line_segment([center, center + d], Stroke::new(2.0, c));
            p.circle_filled(center + d, 7.0, c);
            p.text(center + d, Align2::CENTER_CENTER, ["X", "Y", "Z"][i], FontId::proportional(10.0), Color32::BLACK);
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
                if ui.selectable_label(self.show_bottom, "Output").clicked() {
                    self.show_bottom = !self.show_bottom;
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
