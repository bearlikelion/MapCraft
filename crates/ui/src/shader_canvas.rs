//! Shader node placement and explicit, validated input connections.

use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};
use mapcraft_format::shader_graph::{Graph, Node};

const WIDTH: f32 = 190.0;
const HEIGHT: f32 = 112.0;
const ACCENT: Color32 = Color32::from_rgb(225, 174, 101);

#[derive(Clone, Copy, Debug)]
enum Input {
    Node(usize, usize),
    Albedo,
    Roughness,
    Metallic,
    Emission,
}

#[derive(Default)]
pub(crate) struct Canvas {
    from: Option<usize>,
    dragging_link: bool,
}

fn position(graph: &Graph, id: usize) -> Pos2 {
    graph.positions.get(id).map_or_else(
        || {
            if id < 3 {
                Pos2::new(30.0, 30.0 + id as f32 * 150.0)
            } else {
                Pos2::new(280.0 + ((id - 3) / 4) as f32 * 240.0, 30.0 + ((id - 3) % 4) as f32 * 150.0)
            }
        },
        |p| Pos2::new(p[0], p[1]),
    )
}

fn wire(ui: &egui::Ui, from: Pos2, to: Pos2, color: Color32) {
    let bend = ((to.x - from.x).abs() * 0.5).max(40.0);
    ui.painter().add(egui::epaint::CubicBezierShape::from_points_stroke(
        [from, from + Vec2::new(bend, 0.0), to - Vec2::new(bend, 0.0), to],
        false,
        Color32::TRANSPARENT,
        Stroke::new(2.0, color),
    ));
}

fn connect(graph: &mut Graph, source: usize, input: Input) -> Result<(), String> {
    if graph.nodes.get(source).is_none() {
        return Err("Choose an existing output node".into());
    }
    let mut next = graph.clone();
    match input {
        Input::Node(id, port) => match next.nodes.get_mut(id) {
            Some(Node::Multiply { a, b }) => {
                if port == 0 {
                    *a = source;
                } else {
                    *b = source;
                }
            }
            Some(Node::Sine { input }) => *input = source,
            _ => return Err("This node has no input port".into()),
        },
        Input::Albedo => next.albedo = source,
        Input::Roughness => next.roughness = source,
        Input::Metallic => next.metallic = source,
        Input::Emission => next.emission = Some(source),
    }
    next.source().map_err(|e| e.to_string())?;
    *graph = next;
    Ok(())
}

impl Canvas {
    pub(crate) fn show(&mut self, ui: &mut egui::Ui, graph: &mut Graph, status: &mut String) {
        if self.from.is_some() {
            ui.label("Choose an input port to connect. Escape cancels.");
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.from = None;
            self.dragging_link = false;
        }
        let mut extent = Vec2::new(1100.0, 760.0);
        for id in 0..graph.nodes.len() {
            let p = position(graph, id);
            extent = extent.max(p.to_vec2() + Vec2::new(WIDTH + 40.0, HEIGHT + 40.0));
        }
        let output = graph.output_position.map_or(Pos2::new(730.0, 150.0), |p| Pos2::new(p[0], p[1]));
        extent = extent.max(output.to_vec2() + Vec2::new(260.0, 230.0));
        egui::ScrollArea::both().id_salt("shader-canvas").show(ui, |ui| {
            let (canvas, _) = ui.allocate_exact_size(extent, Sense::hover());
            let origin = canvas.min;
            ui.painter().rect_filled(canvas, 0.0, crate::chrome::BG);
            let output_pos = |id: usize| position(graph, id) + origin.to_vec2() + Vec2::new(WIDTH, 52.0);
            for (id, node) in graph.nodes.iter().enumerate() {
                let to = position(graph, id) + origin.to_vec2();
                let links: Vec<usize> = match node {
                    Node::Multiply { a, b } => vec![*a, *b],
                    Node::Sine { input } => vec![*input],
                    _ => Vec::new(),
                };
                for (port, source) in links.into_iter().enumerate() {
                    if graph.nodes.get(source).is_some() {
                        wire(ui, output_pos(source), to + Vec2::new(0.0, 60.0 + port as f32 * 26.0), ACCENT);
                    }
                }
            }
            let outputs =
                [(graph.albedo, Input::Albedo, "Albedo"), (graph.roughness, Input::Roughness, "Roughness"), (graph.metallic, Input::Metallic, "Metallic")];
            for (port, (source, _, _)) in outputs.iter().enumerate() {
                if graph.nodes.get(*source).is_some() {
                    wire(ui, output_pos(*source), output + origin.to_vec2() + Vec2::new(0.0, 60.0 + port as f32 * 26.0), ACCENT);
                }
            }
            if let Some(source) = graph.emission.filter(|id| graph.nodes.get(*id).is_some()) {
                wire(ui, output_pos(source), output + origin.to_vec2() + Vec2::new(0.0, 138.0), ACCENT);
            }
            if let Some(source) = self.from.filter(|id| graph.nodes.get(*id).is_some())
                && let Some(pointer) = ui.ctx().pointer_hover_pos()
            {
                wire(ui, output_pos(source), pointer, Color32::WHITE);
            }
            let mut chosen = None;
            let mut remove = None;
            for id in 0..graph.nodes.len() {
                let p = position(graph, id) + origin.to_vec2();
                let rect = Rect::from_min_size(p, Vec2::new(WIDTH, HEIGHT));
                ui.painter().rect_filled(rect, 4.0, ui.visuals().widgets.noninteractive.bg_fill);
                ui.painter().rect_stroke(rect, 4.0, Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color), egui::StrokeKind::Inside);
                let header = Rect::from_min_size(p, Vec2::new(WIDTH, 28.0));
                let drag = ui.interact(header, ui.id().with(("node-drag", id)), Sense::click_and_drag());
                drag.context_menu(|ui| {
                    if ui.button("Delete node").clicked() {
                        remove = Some(id);
                        ui.close();
                    }
                });
                if drag.dragged() {
                    let delta = ui.input(|i| i.pointer.delta());
                    while graph.positions.len() < graph.nodes.len() {
                        let next = position(graph, graph.positions.len());
                        graph.positions.push([next.x, next.y]);
                    }
                    if let Some(value) = graph.positions.get_mut(id) {
                        value[0] = (value[0] + delta.x).clamp(0.0, 16384.0);
                        value[1] = (value[1] + delta.y).clamp(0.0, 16384.0);
                    }
                }
                let Some(node) = graph.nodes.get_mut(id) else { continue };
                let title = match node {
                    Node::Scalar { .. } => "Scalar",
                    Node::Color { .. } => "Color",
                    Node::Time => "Time",
                    Node::Multiply { .. } => "Multiply",
                    Node::Sine { .. } => "Sine",
                };
                ui.painter().text(
                    p + Vec2::new(10.0, 14.0),
                    egui::Align2::LEFT_CENTER,
                    format!("{title}  #{id}"),
                    egui::FontId::proportional(14.0),
                    ui.visuals().text_color(),
                );
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .id_salt(("node-content", id))
                        .max_rect(Rect::from_min_size(p + Vec2::new(12.0, 38.0), Vec2::new(WIDTH - 24.0, 65.0))),
                    |ui| match node {
                        Node::Scalar { value } => {
                            ui.add(egui::DragValue::new(value).speed(0.01));
                        }
                        Node::Color { value } => {
                            ui.color_edit_button_rgb(value);
                        }
                        Node::Time => {
                            ui.label("Seconds");
                        }
                        Node::Multiply { a, b } => {
                            ui.label(format!("A  ← #{a}"));
                            ui.label(format!("B  ← #{b}"));
                        }
                        Node::Sine { input } => {
                            ui.label(format!("Value  ← #{input}"));
                        }
                    },
                );
                let count = match node {
                    Node::Multiply { .. } => 2,
                    Node::Sine { .. } => 1,
                    _ => 0,
                };
                for port in 0..count {
                    if self.input_port(ui, p + Vec2::new(0.0, 60.0 + port as f32 * 26.0), (id, port)) {
                        chosen = Some(Input::Node(id, port));
                    }
                }
                let port = p + Vec2::new(WIDTH, 52.0);
                let response = ui.interact(Rect::from_center_size(port, Vec2::splat(20.0)), ui.id().with(("node-output", id)), Sense::click_and_drag());
                ui.painter().circle_filled(port, 6.0, if self.from == Some(id) { Color32::WHITE } else { ACCENT });
                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Output of node {id}")));
                if response.clicked() || response.drag_started() {
                    self.from = Some(id);
                }
                if response.drag_started() {
                    self.dragging_link = true;
                }
            }
            let p = output + origin.to_vec2();
            let rect = Rect::from_min_size(p, Vec2::new(220.0, 190.0));
            ui.painter().rect_filled(rect, 4.0, ui.visuals().widgets.noninteractive.bg_fill);
            let header = ui.interact(Rect::from_min_size(p, Vec2::new(220.0, 28.0)), ui.id().with("material-output-drag"), Sense::drag());
            if header.dragged() {
                let moved = output + ui.input(|i| i.pointer.delta());
                graph.output_position = Some([moved.x.clamp(0.0, 16384.0), moved.y.clamp(0.0, 16384.0)]);
            }
            ui.painter().text(
                p + Vec2::new(12.0, 14.0),
                egui::Align2::LEFT_CENTER,
                "Spatial Material",
                egui::FontId::proportional(14.0),
                ui.visuals().text_color(),
            );
            for (port, (_, input, label)) in outputs.iter().enumerate() {
                let point = p + Vec2::new(0.0, 60.0 + port as f32 * 26.0);
                ui.painter().text(point + Vec2::new(12.0, 0.0), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(13.0), ui.visuals().text_color());
                if self.input_port(ui, point, (usize::MAX, port)) {
                    chosen = Some(*input);
                }
            }
            let point = p + Vec2::new(0.0, 138.0);
            ui.painter().text(point + Vec2::new(12.0, 0.0), egui::Align2::LEFT_CENTER, "Emission", egui::FontId::proportional(13.0), ui.visuals().text_color());
            if self.input_port(ui, point, (usize::MAX, 3)) {
                chosen = Some(Input::Emission);
            }
            if graph.emission.is_some() {
                ui.scope_builder(
                    egui::UiBuilder::new().id_salt("clear-emission").max_rect(Rect::from_min_size(p + Vec2::new(12.0, 162.0), Vec2::new(195.0, 22.0))),
                    |ui| {
                        if ui.button("Clear emission").clicked() {
                            graph.emission = None;
                        }
                    },
                );
            }
            if let (Some(source), Some(input)) = (self.from, chosen) {
                match connect(graph, source, input) {
                    Ok(()) => *status = "Connected nodes".into(),
                    Err(e) => *status = e,
                }
                self.from = None;
            }
            if let Some(id) = remove {
                match graph.remove_node(id) {
                    Ok(()) => {
                        *status = "Deleted node".into();
                        self.from = None;
                    }
                    Err(e) => *status = e.to_string(),
                }
            }
            if self.dragging_link && ui.input(|i| i.pointer.any_released()) {
                self.from = None;
                self.dragging_link = false;
            }
        });
    }

    fn input_port(&self, ui: &mut egui::Ui, point: Pos2, id: (usize, usize)) -> bool {
        let response = ui.interact(Rect::from_center_size(point, Vec2::splat(20.0)), ui.id().with(("node-input", id)), Sense::click());
        ui.painter().circle_filled(point, 6.0, if response.hovered() { Color32::WHITE } else { ACCENT });
        let label = if id.0 == usize::MAX {
            ["Albedo input", "Roughness input", "Metallic input", "Emission input"].get(id.1).copied().unwrap_or("Material input").to_string()
        } else {
            format!("Node {} input {}", id.0, id.1)
        };
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label.clone()));
        response.clicked() || (self.dragging_link && response.hovered() && ui.input(|i| i.pointer.any_released()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_connections_leave_the_graph_intact() {
        let mut graph = Graph::default();
        let original = graph.clone();
        assert!(connect(&mut graph, 1, Input::Albedo).is_err());
        assert_eq!(graph, original);
        graph.nodes.push(Node::Sine { input: 1 });
        let valid = graph.clone();
        assert!(connect(&mut graph, 3, Input::Node(3, 0)).is_err());
        assert_eq!(graph, valid);
        assert!(connect(&mut graph, 2, Input::Roughness).is_ok());
        assert_eq!(graph.roughness, 2);
        assert!(connect(&mut graph, usize::MAX, Input::Albedo).is_err());
    }
}
