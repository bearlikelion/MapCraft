//! The inspector's Shader Parameters: a Godot-style editor for the uniforms of a node's embedded shader.

use egui::Ui;
use egui::ecolor::{gamma_from_linear, linear_from_gamma};
use mapcraft_map::NodeId;
use mapcraft_render::{TextureSource, Uniform};
use serde_json::{Value, json};

use crate::app::MapCraftApp;
use crate::inspect::{self, Prop, Section, Slot, Val};

/// Uniforms under their group and subgroup, in source order.
type Groups = Vec<(String, Vec<(String, Vec<Uniform>)>)>;

fn grouped(uniforms: Vec<Uniform>) -> Groups {
    let mut out: Groups = Vec::new();
    for u in uniforms {
        let (top, sub) = u.group.split_once('.').map_or((u.group.clone(), String::new()), |(t, s)| (t.to_string(), s.to_string()));
        if out.last().is_none_or(|(t, _)| *t != top) {
            out.push((top, Vec::new()));
        }
        let Some((_, subs)) = out.last_mut() else { continue };
        if subs.last().is_none_or(|(s, _)| *s != sub) {
            subs.push((sub, Vec::new()));
        }
        if let Some((_, list)) = subs.last_mut() {
            list.push(u);
        }
    }
    out
}

/// `water_color` as Godot's inspector shows it: `Water Color`.
fn title(name: &str) -> String {
    name.split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect::<String>()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The numbers a parameter holds, else the shader's default, padded to `width`.
fn numbers(u: &Uniform, stored: Option<&Value>, width: usize) -> Vec<f64> {
    let from_stored = stored.filter(|v| u.literal(v).is_some()).map(|v| match v {
        Value::Array(a) => a.iter().filter_map(Value::as_f64).collect(),
        Value::Bool(b) => vec![if *b { 1.0 } else { 0.0 }],
        v => v.as_f64().into_iter().collect(),
    });
    let mut v: Vec<f64> = from_stored.unwrap_or_else(|| u.default.clone());
    v.resize(width, if u.color { 1.0 } else { 0.0 });
    v
}

/// A JSON number that stays an integer for integer uniforms.
fn number(scalar: &str, n: f64) -> Value {
    if scalar == "float" { json!(n) } else { json!(n.round() as i64) }
}

/// The edit a row asks for: a new value, or `None` to return to the default.
type RowEdit = Option<Option<Value>>;

fn uniform_row(ui: &mut Ui, app: &mut MapCraftApp, u: &Uniform, stored: Option<&Value>) -> RowEdit {
    let label = title(&u.name);
    let prop = Prop {
        label: &label,
        key: &u.name,
        tip: &u.ty,
        val: if stored.is_some() { Val::Set } else { Val::Default },
        slot: if stored.is_some() { Slot::Revert } else { Slot::None },
        ..Default::default()
    };
    let mut edit: RowEdit = None;
    let out = match (&u.texture, u.shape()) {
        (Some(TextureSource::Screen), _) => inspect::row(ui, &Prop { readonly: Some("Godot fills it with the frame drawn so far"), ..prop }, |ui| {
            inspect::weak(ui, "Screen texture, filled by the renderer")
        }),
        (Some(fallback), _) => {
            let current = stored.and_then(Value::as_str).unwrap_or_default().to_string();
            let mut shown = if current.is_empty() {
                format!(
                    "Default ({})",
                    match fallback {
                        TextureSource::Black => "black",
                        TextureSource::Transparent => "transparent",
                        TextureSource::Normal => "flat normal",
                        _ => "white",
                    }
                )
            } else {
                current.clone()
            };
            let ctx = ui.ctx().clone();
            let thumb = (!current.is_empty()).then(|| inspect::material_thumb(&app.project, &mut app.thumbs, &ctx, &current)).flatten();
            let (mut browse, mut bucket) = (false, false);
            let out = inspect::row(ui, &prop, |ui| {
                let (r, b) = inspect::material(ui, thumb, &mut shown, &label, "", false, true);
                (browse, bucket) = (r.clicked(), b);
                r
            });
            if browse {
                app.run_action("panel.materials");
            }
            if bucket && !app.session.tex.material.is_empty() {
                edit = Some(Some(json!(app.session.tex.material)));
            }
            out
        }
        (None, Some(("bool", 1))) => {
            let mut on = numbers(u, stored, 1).first().is_some_and(|v| *v != 0.0);
            let out = inspect::row(ui, &prop, |ui| inspect::toggle(ui, &mut on, false, &label));
            if out.changed {
                edit = Some(Some(json!(on)));
            }
            out
        }
        (None, Some((scalar, 1))) => {
            let mut v = numbers(u, stored, 1).first().copied().unwrap_or_default();
            let out = inspect::row(ui, &prop, |ui| match u.range {
                Some([min, max, step]) if max > min => {
                    let decimals = if scalar == "float" { (2.0 - ((max - min) / 100.0).log10()).clamp(0.0, 6.0) as usize } else { 0 };
                    ui.spacing_mut().slider_width = (ui.available_width() - 64.0).max(40.0);
                    let mut s = egui::Slider::new(&mut v, min..=max).max_decimals(decimals).clamping(egui::SliderClamping::Never);
                    if step > 0.0 || scalar != "float" {
                        s = s.step_by(if step > 0.0 { step } else { 1.0 });
                    }
                    ui.add(s)
                }
                _ => inspect::num(ui, &mut v, f64::MIN..=f64::MAX, if scalar == "float" { 0.01 } else { 1.0 }, "", None),
            });
            if out.changed && v.is_finite() {
                edit = Some(Some(number(scalar, v)));
            }
            out
        }
        (None, Some(("float", w @ (3 | 4)))) if u.color => {
            let v = numbers(u, stored, w);
            let mut rgba = [0.0f32, 0.0, 0.0, 1.0];
            for (i, c) in rgba.iter_mut().enumerate().take(w) {
                let x = v.get(i).copied().unwrap_or(1.0) as f32;
                *c = if i < 3 { linear_from_gamma(x) } else { x };
            }
            let out = inspect::row(ui, &prop, |ui| {
                let w_full = ui.available_width();
                ui.spacing_mut().interact_size.x = w_full;
                if w == 4 {
                    let mut c = egui::Rgba::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], rgba[3]);
                    let r = egui::widgets::color_picker::color_edit_button_rgba(ui, &mut c, egui::widgets::color_picker::Alpha::OnlyBlend);
                    rgba = c.to_rgba_unmultiplied();
                    r
                } else {
                    let mut rgb = [rgba[0], rgba[1], rgba[2]];
                    let r = ui.color_edit_button_rgb(&mut rgb);
                    rgba[..3].copy_from_slice(&rgb);
                    r
                }
            });
            if out.changed {
                let srgb: Vec<f64> = rgba.iter().take(w).enumerate().map(|(i, c)| f64::from(if i < 3 { gamma_from_linear(*c) } else { *c })).collect();
                edit = Some(Some(json!(srgb)));
            }
            out
        }
        (None, Some((scalar, w))) => {
            let v = numbers(u, stored, w);
            let store = |values: &[f64]| Some(Some(Value::Array(values.iter().map(|n| number(scalar, *n)).collect())));
            let speed = if scalar == "float" { 0.001 } else { 1.0 };
            let axes = [("X", inspect::XYZ[0].1), ("Y", inspect::XYZ[1].1), ("Z", inspect::XYZ[2].1), ("W", crate::chrome::TEXT_WEAK)];
            inspect::row(ui, &prop, |ui| match w {
                2 => {
                    let mut a = [v.first().copied().unwrap_or_default(), v.get(1).copied().unwrap_or_default()];
                    let (r, axis) = inspect::vec(ui, &mut a, [axes[0], axes[1]], f64::MIN..=f64::MAX, speed, "", [false; 2]);
                    if axis.is_some() {
                        edit = store(&a);
                    }
                    r
                }
                3 => {
                    let mut a = [0.0; 3];
                    a.iter_mut().zip(&v).for_each(|(x, y)| *x = *y);
                    let (r, axis) = inspect::vec(ui, &mut a, [axes[0], axes[1], axes[2]], f64::MIN..=f64::MAX, speed, "", [false; 3]);
                    if axis.is_some() {
                        edit = store(&a);
                    }
                    r
                }
                _ => {
                    let mut a = [0.0; 4];
                    a.iter_mut().zip(&v).for_each(|(x, y)| *x = *y);
                    let (r, axis) = inspect::vec(ui, &mut a, axes, f64::MIN..=f64::MAX, speed, "", [false; 4]);
                    if axis.is_some() {
                        edit = store(&a);
                    }
                    r
                }
            })
        }
        (None, None) => inspect::row(ui, &Prop { readonly: Some("This type cannot be edited here yet"), ..prop }, |ui| inspect::weak(ui, &u.ty)),
    };
    if out.slot { Some(None) } else { edit }
}

impl MapCraftApp {
    /// Godot-style rows for the uniforms of `id`'s embedded shader, grouped by `group_uniforms`.
    pub(crate) fn shader_params_section(&mut self, ui: &mut Ui, id: NodeId) {
        let Some(source) = self.session.map.get(id).and_then(|n| n.shader.clone()) else { return };
        let uniforms = mapcraft_render::uniforms(&source);
        let section = Section { key: "shader.params", title: "Shader Parameters", badge: uniforms.len().to_string(), ..Default::default() };
        if !self.section(ui, &section) {
            return;
        }
        if uniforms.is_empty() {
            inspect::weak(ui, "The shader has no uniforms");
            return;
        }
        let mut edits: Vec<(String, Option<Value>)> = Vec::new();
        for (top, subs) in grouped(uniforms) {
            let mut rows = |ui: &mut Ui, app: &mut Self, list: &[Uniform]| {
                for u in list {
                    let stored = app.session.map.get(id).and_then(|n| n.shader_params.get(&u.name)).cloned();
                    if let Some(edit) = ui.push_id(("shader-param", &u.name), |ui| uniform_row(ui, app, u, stored.as_ref())).inner {
                        edits.push((u.name.clone(), edit));
                    }
                }
            };
            let mut body = |ui: &mut Ui, app: &mut Self| {
                for (sub, list) in &subs {
                    if sub.is_empty() {
                        rows(ui, app, list);
                    } else {
                        egui::CollapsingHeader::new(title(sub)).id_salt(("shader-sub", id.0, &top, sub)).default_open(true).show(ui, |ui| rows(ui, app, list));
                    }
                }
            };
            if top.is_empty() {
                body(ui, self);
            } else {
                egui::CollapsingHeader::new(egui::RichText::new(title(&top)).strong())
                    .id_salt(("shader-group", id.0, &top))
                    .default_open(true)
                    .show(ui, |ui| body(ui, self));
            }
        }
        for (name, value) in edits {
            if let Err(e) = self.session.set_shader_param(id, &name, value) {
                self.status = e.to_string();
            }
            self.mark_changed();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniforms_group_and_read_stored_or_default_values() {
        let source = "shader_type spatial;\ngroup_uniforms water;\nuniform vec4 water_color : source_color = vec4(0.1, 0.3, 0.2, 0.75);\ngroup_uniforms ripples;\nuniform vec2 ripple_speed = vec2(0.02, 0.01);\nsubgroup_uniforms fine;\nuniform int steps = 4;\ngroup_uniforms;\nuniform float a;\n";
        let u = mapcraft_render::uniforms(source);
        let g = grouped(u.clone());
        let shape: Vec<(&str, Vec<&str>)> = g.iter().map(|(t, s)| (t.as_str(), s.iter().map(|(n, _)| n.as_str()).collect())).collect();
        assert_eq!(shape, [("water", vec![""]), ("ripples", vec!["", "fine"]), ("", vec![""])]);
        assert_eq!(numbers(&u[0], None, 4), [0.1, 0.3, 0.2, 0.75]);
        assert_eq!(numbers(&u[1], Some(&json!([1, 2])), 2), [1.0, 2.0]);
        assert_eq!(numbers(&u[1], Some(&json!("bad")), 2), [0.02, 0.01], "a wrong type shows the default");
        assert_eq!(number("int", 3.6), json!(4));
        assert_eq!(title("ripple_strength"), "Ripple Strength");
    }
}
