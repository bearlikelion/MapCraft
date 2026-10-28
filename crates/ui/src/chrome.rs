//! The editor's frame around the viewports: theme, tool palette, top bar, viewport headers and
//! the status bar.

use std::time::{Duration, Instant};

use craft_ui::buttons::{ButtonVisuals, IconButton, IconButtonStyle};
use craft_ui::theme::WidgetPalette;
use egui::{Align2, Color32, CornerRadius, FontId, Painter, Pos2, Rect, RichText, Shape, Stroke, Vec2, pos2, vec2};
use mapcraft_geom::DVec3;
use mapcraft_render::{Camera, RenderMode, View};

use crate::app::MapCraftApp;
use crate::icons;
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
pub const WARN: Color32 = Color32::from_rgb(240, 190, 80);
pub const ERROR: Color32 = Color32::from_rgb(240, 100, 100);
pub const AXIS: [Color32; 3] = [Color32::from_rgb(232, 76, 76), Color32::from_rgb(110, 200, 80), Color32::from_rgb(80, 140, 240)];

const ROTATE_HINT: &str = "it snaps to 15 degree steps while Snap is on";

/// How long a status message stays before the bar shows hints again.
const STATUS_TIME: Duration = Duration::from_secs(8);

pub fn apply_theme(ctx: &egui::Context) {
    egui_extras::install_image_loaders(ctx);
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
        selected: flat(Color32::from_rgb(72, 53, 36), ACCENT),
        corner_radius: CornerRadius::same(5),
        focus_stroke: Stroke::new(1.0, ACCENT),
    }
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

/// A flat icon-only button; `label` stays its accessible name, so tests and the snapshot tool find it.
pub(crate) fn icon_button(ui: &mut egui::Ui, name: &str, label: &str, size: f32, selected: bool) -> egui::Response {
    let icon = if size < 22.0 { 14.0 } else { icons::SIZE };
    IconButton::new(label, Vec2::splat(size), &tool_style()).selected(selected).show(ui, |ui, r, c| icons::paint(ui, name, r, icon, c))
}

/// A row of icon toggles of which one is on, like Godot's toolbar modes.
pub(crate) fn icon_toggles<T: PartialEq + Copy>(ui: &mut egui::Ui, current: &mut T, options: &[(T, &str, &str, &str)]) -> bool {
    let mut changed = false;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = 1.0;
        for (value, icon, label, tip) in options {
            let on = *current == *value;
            if icon_button(ui, icon, label, 24.0, on).on_hover_text(*tip).clicked() && !on {
                *current = *value;
                changed = true;
            }
        }
    });
    changed
}

/// A 0 to 1 value edited as a percentage from `min` to 100.
fn percent(ui: &mut egui::Ui, value: &mut f64, min: f64, prefix: &str, tip: &str) {
    let mut p = *value * 100.0;
    if crate::inspect::number(ui, egui::DragValue::new(&mut p).range(min..=100.0).max_decimals(0).prefix(prefix).suffix("%")).on_hover_text(tip).changed() {
        *value = p / 100.0;
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
        if let Some(m) = &self.uv_modal {
            return m.hint();
        }
        if let Some(m) = &self.vertex.bevel {
            return m.hint();
        }
        let painting = self.tool == Tool::Sculpt && matches!(self.sculpt.0, mapcraft_engine::SculptMode::Paint | mapcraft_engine::SculptMode::Blur);
        if let Some(r) = self.blend_readout.as_ref().filter(|_| painting) {
            return r.clone();
        }
        match &self.drag {
            Drag::None => {}
            Drag::Extrude { offset, normal, mode, .. } => {
                use mapcraft_engine::ExtrudeMode as M;
                let (what, amount) = match mode {
                    M::Resize | M::AlongNormal => ("Extruding", offset.dot(*normal)),
                    M::Split => ("Splitting", offset.dot(*normal)),
                    M::Stamp => ("Stamping", offset.dot(*normal)),
                    M::Slide => ("Sliding", offset.length().copysign(offset.dot(*normal))),
                };
                return format!("{what} {} units; release to finish, Esc cancels", crate::measure::units(amount));
            }
            Drag::Rotate { applied, .. } => return format!("Turned {applied} degrees; release to finish"),
            _ => return "Release to finish; Ctrl+Z undoes the whole drag".into(),
        }
        match self.tool {
            Tool::Select => {
                "Click to select, click again to pick the object behind; drag to move, drag the white handles to resize; Shift+drag a face extrudes".into()
            }
            Tool::Move => "Drag in a 2D view or along the arrows in the 3D view to move the selection".into(),
            Tool::Extrude => {
                "Drag a highlighted face of a selected brush; Ctrl splits off a new brush, Ctrl+Alt stamps one, Alt slides the face keeping its shape".into()
            }
            Tool::Rotate => format!("Drag in a 2D view to turn the selection about its center, or drag a ring in the 3D view; {ROTATE_HINT}"),
            Tool::Scale => "Drag the white handles in a 2D view or the square handles in the 3D view to stretch the selection".into(),
            Tool::Block if self.plugin_shape.is_some() => "Click in a view to place it; its parameters are in the inspector".into(),
            Tool::Block => format!("Drag in a 2D view to draw a {}; its depth matches the last brush", shape_name(self.shape)),
            Tool::Entity => format!(
                "Click in a view to place {}; in the 3D view it rests on the face under the pointer; pick the class in the Entities tab",
                self.entity_class
            ),
            Tool::Clip if self.session.selected_brushes().is_empty() => "Select the brushes to clip first".into(),
            Tool::Clip if self.clip_points.len() == 2 => format!(
                "Enter clips, Esc or Delete takes back a point; yellow stays, red goes; {} picks the side to keep; drag a point to move it, or click a third in another view to tilt the cut",
                self.chord("clip.next_side")
            ),
            Tool::Clip if self.clip_points.len() == 3 => {
                format!(
                    "Enter clips through the three points, Esc or Delete takes back the last; {} picks the side to keep; drag a point to move it",
                    self.chord("clip.next_side")
                )
            }
            Tool::Clip => {
                "Click two points, or drag a line, in any view; in the 3D view the cut runs straight into the face; double-click a face to cut along it".into()
            }
            Tool::Vertex => format!(
                "Click or box select corners, edges or faces ({} {} {}), Shift adds; drag to move them, Shift or Alt drags up and down in the 3D view; {} merges, {} bevels, {} deletes",
                self.chord("vertex.mode_vertex"),
                self.chord("vertex.mode_edge"),
                self.chord("vertex.mode_face"),
                self.chord("vertex.merge"),
                self.chord("vertex.bevel"),
                self.chord("vertex.delete")
            ),
            Tool::Sculpt if painting => {
                "Drag on a displacement to paint the current material, or to soften blend edges with Blur; Ctrl paints the face's own material back, Alt+click picks the material under the pointer".into()
            }
            Tool::Sculpt => "Drag on a displacement in any view; Ctrl swaps raise and lower; displaced faces picked with the Face tool limit it".into(),
            Tool::Face => {
                "Click faces in the 3D view; Ctrl+C copies a face and Ctrl+V pastes it on the picked faces; Alt+click picks, Alt+right click applies".into()
            }
            Tool::Path => format!(
                "Click to add path points, drag to pull out handles; click the first point to close, Enter starts a new path; pick faces and {} sweeps them along it",
                self.chord("tools.sweep")
            ),
        }
    }

    pub(crate) fn tool_palette(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        let tools: [(Tool, &str, &str); 12] = [
            (Tool::Select, "Select", "tool.select"),
            (Tool::Move, "Move", "tool.move"),
            (Tool::Extrude, "Extrude", "tool.extrude"),
            (Tool::Rotate, "Rotate", "tool.rotate"),
            (Tool::Scale, "Scale", "tool.scale"),
            (Tool::Block, "Block", "tool.block"),
            (Tool::Entity, "Entity", "tool.entity"),
            (Tool::Clip, "Clip", "tool.clip"),
            (Tool::Vertex, "Vertex", "tool.vertex"),
            (Tool::Face, "Face", "tool.face"),
            (Tool::Sculpt, "Sculpt", "tool.sculpt"),
            (Tool::Path, "Path", "tool.path"),
        ];
        ui.vertical_centered(|ui| {
            for (tool, label, id) in tools {
                let chord = self.chord(id);
                let tip = if chord.is_empty() { label.to_string() } else { format!("{label}  ({chord})") };
                let block = tool == Tool::Block;
                let tip = if block { format!("{tip}; right click for shapes") } else { tip };
                let name = icons::action_icon(id).unwrap_or("help");
                let r = IconButton::new(label, Vec2::splat(32.0), &tool_style()).selected(self.tool == tool).show(ui, |ui, rect, c| {
                    icons::paint(ui, name, rect, icons::SIZE, c);
                    if block {
                        let corner = rect.right_bottom() - Vec2::splat(3.0);
                        ui.painter().add(Shape::convex_polygon(vec![corner, corner - Vec2::new(5.0, 0.0), corner - Vec2::new(0.0, 5.0)], c, Stroke::NONE));
                    }
                });
                let r = r.on_hover_text(tip);
                let mut pick = None;
                if block {
                    r.context_menu(|ui| pick = self.shape_menu(ui));
                }
                if let Some((shape, generator)) = pick {
                    self.pick_block_shape(shape, generator);
                } else if r.clicked() {
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

    /// The built-in block shapes and plugin generators as menu items; returns the one clicked.
    pub(crate) fn shape_menu(&mut self, ui: &mut egui::Ui) -> Option<BlockPick> {
        let mut pick = None;
        for (shape, label, icon) in block_shapes() {
            if ui.add(icons::button(Some(icon), label)).clicked() {
                pick = Some((Some(shape), None));
            }
        }
        let generators = self.generator_shapes();
        if !generators.is_empty() {
            ui.separator();
        }
        for (plugin, command) in generators {
            if ui.add(icons::button(Some("jigsaw_piece"), &command.label)).clicked() {
                pick = Some((None, Some((plugin, command))));
            }
        }
        pick
    }

    pub(crate) fn segmented<T: PartialEq + Copy>(ui: &mut egui::Ui, current: &mut T, options: &[(T, &str, &str)]) -> bool {
        let mut changed = false;
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.x = 1.0;
            for (value, label, tip) in options {
                let on = *current == *value;
                let text = if on { RichText::new(*label).color(Color32::WHITE) } else { RichText::new(*label) };
                let button = egui::Button::new(text).fill(if on { ACCENT_DIM } else { WIDGET }).corner_radius(CornerRadius::ZERO);
                let enabled = ui.is_enabled();
                let r = ui.add(button);
                r.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, on, *label));
                if r.on_hover_text(*tip).clicked() && !on {
                    *current = *value;
                    changed = true;
                }
            }
        });
        changed
    }

    pub(crate) fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for (id, label, can, what) in [
                ("edit.undo", "Undo", self.session.can_undo(), self.session.undo_label().map(str::to_string)),
                ("edit.redo", "Redo", self.session.can_redo(), self.session.redo_label().map(str::to_string)),
            ] {
                let name = icons::action_icon(id).unwrap_or("help");
                let r = ui.add_enabled_ui(can, |ui| icon_button(ui, name, label, 24.0, false)).inner;
                let tip = format!("{label} {}", what.unwrap_or_default());
                if r.on_hover_text(format!("{}  ({})", tip.trim(), self.chord(id))).clicked() {
                    self.run_action(id);
                }
            }
            ui.separator();

            use mapcraft_engine::SelectMode as M;
            let modes = [
                (M::Groups, icons::select_mode_icon(M::Groups), "Groups", "Clicks select whole groups"),
                (M::Objects, icons::select_mode_icon(M::Objects), "Objects", "Clicks select entities and loose brushes, ignoring groups"),
                (M::Solids, icons::select_mode_icon(M::Solids), "Solids", "Clicks select single brushes"),
            ];
            icon_toggles(ui, &mut self.session.select_mode, &modes);
            ui.separator();

            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let grid = self.session.grid;
                let mut step = |ui: &mut egui::Ui, id: &str, label: &str| {
                    if icon_button(ui, icons::action_icon(id).unwrap_or("help"), label, 24.0, false)
                        .on_hover_text(format!("{label}  ({})", self.chord(id)))
                        .clicked()
                    {
                        self.run_action(id);
                    }
                };
                step(ui, "view.grid_smaller", "Smaller grid");
                ui.label(RichText::new(grid.to_string()).strong().monospace()).on_hover_text("Grid size");
                step(ui, "view.grid_larger", "Larger grid");
            });
            let view = self.prefs.get().view.clone();
            let lock = self.session.texture_lock;
            for (on, label, id, tip) in [
                (view.snap_grid, "Snap", "view.snap_grid", "Snap to the grid"),
                (view.snap_vertices, "Corners", "view.snap_vertices", "Snap dragged points to nearby brush corners and entity origins"),
                (lock, "Texture lock", "tools.toggle_texture_lock", "Keep textures fixed to brushes as they move"),
            ] {
                let name = if id == "tools.toggle_texture_lock" && !on { "unlock" } else { icons::action_icon(id).unwrap_or("help") };
                let chord = self.chord(id);
                let tip = if chord.is_empty() { tip.to_string() } else { format!("{tip}  ({chord})") };
                if icon_button(ui, name, label, 24.0, on).on_hover_text(tip).clicked() {
                    self.run_action(id);
                }
            }
            ui.separator();
            self.material_chip(ui);
            self.tool_options(ui);

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.run_button(ui);
                let tip = format!("Check the map for problems  ({})", self.chord("tools.check_problems"));
                if icon_button(ui, "list_checkboxes", "Check", 24.0, false).on_hover_text(tip).clicked() {
                    self.run_action("tools.check_problems");
                }
            });
        });
    }

    fn material_chip(&mut self, ui: &mut egui::Ui) {
        let name = if self.session.tex.material.is_empty() { "No material".to_string() } else { self.session.tex.material.clone() };
        let thumb = crate::inspect::material_thumb(&self.project, &mut self.thumbs, ui.ctx(), &self.session.tex.material);
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
        let tip = if self.tool == Tool::Sculpt && self.sculpt.0 == mapcraft_engine::SculptMode::Paint {
            "Paints this material; Ctrl paints the face's own material back, and Alt+click on a face picks another".to_string()
        } else {
            format!("The material new faces get; click to browse  ({})", self.chord("panel.materials"))
        };
        if r.on_hover_text(tip).clicked() {
            self.run_action("panel.materials");
        }
    }

    fn tool_options(&mut self, ui: &mut egui::Ui) {
        if self.tool == Tool::Block {
            ui.separator();
            use mapcraft_engine::Shape as S;
            let shapes = block_shapes();
            ui.label(RichText::new(self.chord("shape.next")).color(TEXT_WEAK).small()).on_hover_text("Next shape");
            let generators = self.generator_shapes();
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.x = 1.0;
                for (shape, label, icon) in shapes {
                    let same = self.plugin_shape.is_none() && std::mem::discriminant(&self.shape) == std::mem::discriminant(&shape);
                    if icon_button(ui, icon, label, 24.0, same).on_hover_text(label).clicked() && !same {
                        self.shape = shape;
                        self.pick_plugin_shape(None);
                    }
                }
                ui.add_space(4.0);
                for (plugin, command) in generators {
                    let same = self.plugin_shape.as_ref().is_some_and(|s| s.plugin == plugin && s.command.id == command.id);
                    if ui.add(icons::selectable(same, Some("jigsaw_piece"), &command.label)).clicked() && !same {
                        self.pick_plugin_shape(Some((plugin, command)));
                    }
                }
            });
            if self.plugin_shape.is_some() {
                return;
            }
            match &mut self.shape {
                S::Cylinder(n) | S::Spike(n) => sides_stepper(ui, n, 3, 64),
                S::Sphere(n) => sides_stepper(ui, n, 4, 32),
                S::Arch { sides, arc, start, thickness } => {
                    sides_stepper(ui, sides, 1, 64);
                    crate::inspect::number(ui, egui::DragValue::new(arc).range(1.0..=360.0).prefix("arc ").suffix("°"));
                    crate::inspect::number(ui, egui::DragValue::new(start).range(0.0..=360.0).prefix("start ").suffix("°"));
                    crate::inspect::number(ui, egui::DragValue::new(thickness).range(1.0..=4096.0).prefix("wall "));
                }
                S::Block | S::Wedge => {}
            }
        }
        if self.tool == Tool::Sculpt {
            ui.separator();
            use mapcraft_engine::SculptMode as M;
            let tip = |what: &str, id: &str| format!("{what}  ({})", self.chord(id));
            let tips = [
                tip("Pull the surface up", "sculpt.raise"),
                tip("Push the surface down", "sculpt.lower"),
                tip("Even out bumps", "sculpt.smooth"),
                tip("Level toward where the stroke began", "sculpt.flatten"),
                tip("Roughen the surface", "sculpt.noise"),
                tip("Paint the current material onto the terrain", "sculpt.paint"),
                tip("Soften blend edges", "sculpt.blur"),
            ];
            let modes = [
                (M::Raise, "arrow_up", "Raise", tips[0].as_str()),
                (M::Lower, "arrow_down", "Lower", tips[1].as_str()),
                (M::Smooth, "wave", "Smooth", tips[2].as_str()),
                (M::Flatten, "arrow_down_to_line", "Flatten", tips[3].as_str()),
                (M::Noise, "dice", "Noise", tips[4].as_str()),
                (M::Paint, "brush", "Paint", tips[5].as_str()),
                (M::Blur, "circle_half_filled", "Blur", tips[6].as_str()),
            ];
            icon_toggles(ui, &mut self.sculpt.0, &modes);
            crate::inspect::number(ui, egui::DragValue::new(&mut self.sculpt.1).range(8.0..=4096.0).speed(4.0).prefix("radius "));
            use mapcraft_engine::Falloff as F;
            let falloffs = [(F::Smooth, "Smooth"), (F::Linear, "Linear"), (F::Sharp, "Sharp"), (F::Constant, "Constant")];
            let shown = falloffs.iter().find(|(f, _)| *f == self.sculpt_falloff).map_or("", |(_, l)| l);
            egui::ComboBox::from_id_salt("sculpt-falloff")
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    for (f, label) in falloffs {
                        ui.selectable_value(&mut self.sculpt_falloff, f, label);
                    }
                })
                .response
                .on_hover_text("How the brush fades from its center to its edge");
            if !matches!(self.sculpt.0, M::Paint | M::Blur) {
                crate::inspect::number(ui, egui::DragValue::new(&mut self.sculpt.2).range(1.0..=4096.0).speed(4.0).prefix("strength "));
            } else {
                percent(ui, &mut self.paint.flow, 1.0, "flow ", "How much one dab paints, or how fast Blur softens");
                if self.sculpt.0 == M::Paint {
                    percent(ui, &mut self.paint.opacity, 1.0, "opacity ", "The most one stroke changes the blend");
                    percent(ui, &mut self.paint.spacing, 5.0, "spacing ", "Distance between dabs along a stroke, as a share of the radius");
                }
                let on = self.paint.show_weights;
                if icon_button(ui, "eye", "Show weights", 24.0, on).on_hover_text("Show weights: tint the terrain cyan where the second material is").clicked()
                {
                    self.paint.show_weights = !on;
                }
            }
        }
        if self.tool == Tool::Vertex {
            self.vertex_options(ui);
        }
        if self.tool == Tool::Extrude {
            ui.separator();
            let modes = [
                (
                    false,
                    "arrow_double_vertical",
                    "Stretch",
                    "A drag moves the face's plane and the faces around it stretch to meet it; Alt+drag slides the face instead",
                ),
                (
                    true,
                    "arrow_up_from_line",
                    "Along normal",
                    "A drag pushes the face out keeping its shape, with new sides square to it; Alt+drag slides the face instead",
                ),
            ];
            let mut along = self.prefs.get().view.extrude_along_normal;
            if icon_toggles(ui, &mut along, &modes) {
                self.prefs.update(|p| p.view.extrude_along_normal = along);
            }
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
        let tip = if profiles.is_empty() {
            "This SDK has no compile profiles".to_string()
        } else if running {
            "Running; click for Cancel".to_string()
        } else {
            format!("Compile and play the map; {} runs the last profile again", self.chord("run.again"))
        };
        let flat = |fill| ButtonVisuals { fill, stroke: Stroke::NONE, icon: Color32::BLACK };
        let style = IconButtonStyle {
            normal: flat(ACCENT),
            hovered: flat(Color32::from_rgb(255, 165, 75)),
            pressed: Some(flat(Color32::from_rgb(210, 115, 25))),
            selected: flat(ACCENT),
            corner_radius: CornerRadius::same(5),
            focus_stroke: Stroke::new(1.0, Color32::WHITE),
        };
        let r = ui
            .add_enabled_ui(!profiles.is_empty(), |ui| {
                IconButton::new("Run", vec2(40.0, 24.0), &style)
                    .show(ui, |ui, r, c| icons::paint(ui, if running { "hourglass" } else { "play" }, r, icons::SIZE, c))
            })
            .inner
            .on_hover_text(&tip)
            .on_disabled_hover_text(&tip);
        egui::Popup::menu(&r).show(|ui| {
            for (i, name) in profiles.iter().enumerate() {
                if ui.add_enabled(!running, icons::button(Some("play"), name)).clicked() {
                    self.start_compile(i);
                }
            }
            if running && ui.add(icons::button(Some("square"), "Cancel")).clicked() {
                self.cancel_compile();
            }
        });
    }

    /// The strip above a viewport: view picker, axes and zoom, render mode, maximize.
    pub(crate) fn pane_header(&mut self, ui: &mut egui::Ui, index: usize, header: Rect, active: bool) {
        ui.painter().rect_filled(header, 0.0, HEADER);
        let mut header_ui =
            ui.new_child(egui::UiBuilder::new().max_rect(header.shrink2(vec2(4.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        let Some((cam, uv)) = self.panes.get(index).map(|p| (p.camera, p.uv)) else { return };
        let ui = &mut header_ui;
        let title = RichText::new(if uv.is_some() { "UV" } else { short_label(cam.view) }).color(if active { ACCENT } else { TEXT }).strong();
        ui.menu_button(title, |ui| {
            for v in View::ALL {
                if ui.add(icons::selectable(uv.is_none() && cam.view == v, None, v.label())).clicked()
                    && let Some(p) = self.panes.get_mut(index)
                {
                    p.camera = Camera { view: v, ..p.camera };
                    p.uv = None;
                }
            }
            if ui.add(icons::selectable(uv.is_some(), Some("unwrap"), "UV Editor")).on_hover_text("The picked faces laid out over their texture").clicked()
                && let Some(p) = self.panes.get_mut(index)
            {
                p.uv = Some(crate::uv::UvCamera::default());
            }
        });
        match (uv, view_axes(cam.view)) {
            (Some(c), _) => {
                if let Some((material, size)) = self.uv_reference() {
                    ui.label(RichText::new(format!("{material}  {}x{}", size.x, size.y)).color(TEXT_WEAK).monospace());
                }
                ui.label(RichText::new(format!("{:.2} px/texel", c.zoom)).color(TEXT_WEAK).monospace()).on_hover_text("Screen pixels per texel");
            }
            (None, Some((a, b))) => {
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
            (None, None) => {
                let mode = self.render_mode;
                let modes = [
                    (RenderMode::Textured, "Textured", "view.render_textured"),
                    (RenderMode::Flat, "Flat", "view.render_flat"),
                    (RenderMode::Wireframe, "Wireframe", "view.render_wireframe"),
                    (RenderMode::Lit, "Lit", "view.render_lit"),
                ];
                let (label, id) = modes.iter().find(|(m, ..)| *m == mode).map_or(("Textured", "view.render_textured"), |(_, l, id)| (*l, *id));
                let current = icons::icon(icons::action_icon(id).unwrap_or("help")).tint(TEXT_WEAK).alt_text(label);
                ui.menu_button(current, |ui| {
                    for (m, l, id) in modes {
                        if ui.add(icons::selectable(mode == m, icons::action_icon(id), l)).clicked() {
                            self.render_mode = m;
                        }
                    }
                })
                .response
                .on_hover_text(format!("Render mode: {label}"));
                let sky = self.prefs.get().view.sky;
                let chord = self.chord("view.sky");
                let tip = if sky { "Sky shown; click to hide it and draw sky faces flat" } else { "Sky hidden; click to show it" };
                let tip = if chord.is_empty() { tip.to_string() } else { format!("{tip}  ({chord})") };
                let drawn = matches!(mode, RenderMode::Textured | RenderMode::Lit);
                let r = ui.add_enabled_ui(drawn, |ui| icon_button(ui, icons::action_icon("view.sky").unwrap_or("help"), "Sky", 20.0, sky)).inner;
                if r.on_hover_text(tip).on_disabled_hover_text("Flat and Wireframe draw no sky").clicked() {
                    self.run_action("view.sky");
                }
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let maximized = self.maximized.is_some();
            let linked = self.prefs.get().view.link_zoom;
            let label = if maximized { "Restore" } else { "Maximize" };
            let r = icon_button(ui, if maximized { "windows" } else { "expand" }, label, 20.0, false);
            if r.on_hover_text(format!("{label}  ({})", self.chord("view.maximize"))).clicked() {
                self.maximized = if maximized { None } else { Some(index) };
                if maximized {
                    self.prefs.update(|p| p.view.single_view = false);
                }
            }
            if cam.is_2d() && uv.is_none() {
                let r = icon_button(ui, "link", "Link zoom", 20.0, linked);
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
        let dist = self.pick(cam.position, cam.forward(), true).map_or(1024.0, |(_, t)| t);
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
            let item = |ui: &mut egui::Ui, icon: &str, tint: Color32, tip: &str, value: &str| {
                let r = ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.add(icons::icon(icon).tint(tint));
                    if !value.is_empty() {
                        ui.label(RichText::new(value).color(TEXT));
                    }
                });
                r.response.on_hover_text(tip);
                ui.add_space(8.0);
            };
            ui.label(RichText::new(self.selection_summary()).color(TEXT).strong());
            ui.add_space(8.0);
            if let Some(p) = self.cursor_world {
                item(ui, "reticle", TEXT_WEAK, "Cursor", &format!("{:.0} {:.0} {:.0}", p.x, p.y, p.z));
            }
            item(ui, "grid_coarse", TEXT_WEAK, "Grid", &self.session.grid.to_string());
            let mode = format!("{:?}", self.session.select_mode).to_lowercase();
            item(ui, icons::select_mode_icon(self.session.select_mode), TEXT_WEAK, "Selection mode", &mode);
            let lock = self.session.texture_lock;
            let (icon, tint, tip) = if lock { ("lock", ACCENT, "Texture lock on") } else { ("unlock", TEXT_WEAK, "Texture lock off") };
            item(ui, icon, tint, tip, "");
            let name = self.project.sdk.as_ref().map(|s| s.manifest.name.clone()).or_else(|| self.session.sdk.clone()).unwrap_or("none".into());
            ui.menu_button((icons::icon("delivery_box").tint(TEXT_WEAK), RichText::new(name).color(TEXT)), |ui| {
                self.sdk_choices(ui);
            })
            .response
            .on_hover_text("Switch the map's SDK");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_button(ui, "window_terminal", "Output", 20.0, self.show_bottom)
                    .on_hover_text(format!("Output  ({})", self.chord("panel.output")))
                    .clicked()
                {
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

/// A shape's side count as a `- n +` stepper after a hexagon icon; the number still drags.
fn sides_stepper(ui: &mut egui::Ui, n: &mut usize, min: usize, max: usize) {
    ui.separator();
    let (r, hover) = ui.allocate_exact_size(Vec2::splat(icons::SIZE), egui::Sense::hover());
    icons::paint(ui, "hexagon", r, icons::SIZE, TEXT_WEAK);
    hover.on_hover_text("Sides");
    let fewer = ui.add_enabled_ui(*n > min, |ui| icon_button(ui, "dash_in_square", "Fewer sides", 20.0, false)).inner;
    if fewer.on_hover_text("Fewer sides").clicked() {
        *n -= 1;
    }
    crate::inspect::number(ui, egui::DragValue::new(n).range(min..=max).speed(0.1));
    let more = ui.add_enabled_ui(*n < max, |ui| icon_button(ui, "plus_sign_in_square", "More sides", 20.0, false)).inner;
    if more.on_hover_text("More sides").clicked() {
        *n += 1;
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

impl MapCraftApp {
    /// Switches to the Block tool drawing `shape`, or placing a plugin generator.
    pub(crate) fn pick_block_shape(&mut self, shape: Option<mapcraft_engine::Shape>, generator: Option<(String, mapcraft_plugin_host::CommandSpec)>) {
        if let Some(shape) = shape {
            self.shape = shape;
        }
        self.pick_plugin_shape(generator);
        if self.tool != Tool::Block {
            self.run_action("tool.block");
        }
    }
}

/// A block shape or a plugin generator picked from a shape menu.
pub(crate) type BlockPick = (Option<mapcraft_engine::Shape>, Option<(String, mapcraft_plugin_host::CommandSpec)>);

/// The Block tool's built-in shapes with their default settings, names and icons.
pub(crate) fn block_shapes() -> [(mapcraft_engine::Shape, &'static str, &'static str); 6] {
    use mapcraft_engine::Shape as S;
    let arch = S::Arch { sides: 8, arc: 180.0, start: 0.0, thickness: 16.0 };
    [
        (S::Block, "Box", "box"),
        (S::Wedge, "Wedge", "prism"),
        (S::Cylinder(8), "Cylinder", "cylinder"),
        (S::Spike(4), "Spike", "pyramid"),
        (S::Sphere(8), "Sphere", "sphere"),
        (arch, "Arch", "doorway"),
    ]
}
