//! The inspector's header, filter, foldable sections and label/value rows.

use std::hash::Hash;
use std::ops::RangeInclusive;

use egui::emath::Numeric;
use egui::response::Flags;
use egui::{Align, Color32, Id, Label, Layout, Rect, Response, RichText, Sense, TextureId, Ui, Vec2, pos2, vec2};
use mapcraft_map::{Entity, NodeId, NodeKind};
use mapcraft_plugin_host::{ParamKind, ParamSpec};
use serde_json::{Value, json};

use crate::app::MapCraftApp;
use crate::chrome::{AXIS, BG, ERROR, HEADER, HOVER, LINE, TEXT, TEXT_WEAK, WARN, icon_button};
use crate::icons;

const ROW_H: f32 = 22.0;
pub(crate) const ROW_GAP: f32 = 2.0;
const SECTION_H: f32 = 24.0;
const SECTION_GAP: f32 = 6.0;
const LABEL_SHARE: f32 = 0.4;
const LABEL_MIN: f32 = 64.0;
const LABEL_MAX: f32 = 140.0;
const LABEL_PAD: f32 = 4.0;
const SLOT: f32 = 18.0;
const HEADER_ICON: f32 = 20.0;
const THUMB: f32 = 18.0;
pub(crate) const LIMIT: f64 = 1e6;
pub(crate) const ANGLE_LIMIT: f64 = 3600.0;

pub(crate) const XYZ: [(&str, Color32); 3] = [("X", AXIS[0]), ("Y", AXIS[1]), ("Z", AXIS[2])];
pub(crate) const PYR: [(&str, Color32); 3] = [("P", AXIS[1]), ("Y", AXIS[2]), ("R", AXIS[0])];
const YPR: [(&str, Color32); 3] = [("Y", AXIS[2]), ("P", AXIS[1]), ("R", AXIS[0])];
pub(crate) const UV: [(&str, Color32); 2] = [("U", AXIS[0]), ("V", AXIS[1])];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Val {
    #[default]
    Set,
    Default,
    Mixed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Slot {
    #[default]
    None,
    Revert,
    Remove,
}

#[derive(Default)]
pub(crate) struct Prop<'a> {
    pub label: &'a str,
    pub key: &'a str,
    pub tip: &'a str,
    pub icon: Option<(&'a str, Color32)>,
    pub val: Val,
    pub slot: Slot,
    pub readonly: Option<&'a str>,
    pub warn: bool,
    pub wide: bool,
}

#[derive(Default)]
pub(crate) struct RowOut {
    pub changed: bool,
    pub slot: bool,
}

pub(crate) enum Note {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum Change {
    #[default]
    None,
    Set(String),
    Axis(usize, f64),
    Bit(u64, bool),
}

pub(crate) enum Title<'a> {
    Fixed(&'a str),
    Edit(&'a mut String, &'a str),
}

pub(crate) struct HeaderButton<'a> {
    pub icon: &'a str,
    pub label: &'a str,
    pub tip: &'a str,
    pub on: bool,
}

#[derive(Default)]
pub(crate) struct HeaderOut {
    pub renamed: bool,
    pub clicked: Option<usize>,
}

#[derive(Default)]
pub(crate) struct Section<'a> {
    pub key: &'a str,
    pub title: &'a str,
    pub badge: String,
    pub tip: &'a str,
    pub icon: Option<(&'a str, Color32)>,
    pub closed: bool,
    /// An icon button at the bar's right end.
    pub action: Option<HeaderButton<'a>>,
}

pub(crate) struct Action<'a> {
    pub icon: &'a str,
    pub label: &'a str,
    pub tip: &'a str,
    pub enabled: bool,
}

impl Change {
    /// The value `old` becomes; `None` when nothing changes or `old` cannot take the change.
    pub(crate) fn apply(&self, old: &str) -> Option<String> {
        match self {
            Change::None => None,
            Change::Set(s) => Some(s.clone()),
            Change::Axis(i, v) => {
                if *i >= 3 || !v.is_finite() {
                    return None;
                }
                let mut n = numbers(old)?;
                if n.len() < 3 {
                    n.resize(3, 0.0);
                }
                *n.get_mut(*i)? = *v;
                Some(n.iter().map(|x| format!("{}", x + 0.0)).collect::<Vec<_>>().join(" "))
            }
            Change::Bit(b, on) => {
                let t = old.trim();
                let bits: u64 = if t.is_empty() { 0 } else { t.parse().ok()? };
                Some((if *on { bits | b } else { bits & !b }).to_string())
            }
        }
    }
}

/// Whitespace separated finite numbers; `None` when any token is not one.
fn numbers(s: &str) -> Option<Vec<f64>> {
    s.split_whitespace().map(|t| t.parse::<f64>().ok().filter(|x| x.is_finite())).collect()
}

/// Equal strings, or the same count of equal numbers ("1" and "1.0").
pub(crate) fn same(a: &str, b: &str) -> bool {
    a == b || matches!((numbers(a), numbers(b)), (Some(x), Some(y)) if x == y)
}

/// The entity's own spelling of `key` and its value, ignoring ASCII case.
pub(crate) fn find_key<'e>(e: &'e Entity, key: &str) -> Option<(&'e str, &'e str)> {
    e.properties.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(k, v)| (k.as_str(), v.as_str()))
}

pub(crate) fn material_thumb(project: &crate::project::Project, thumbs: &mut crate::thumbs::Thumbs, ctx: &egui::Context, name: &str) -> Option<TextureId> {
    let m = project.find_material(name)?;
    thumbs.get(ctx, &m).map(|t| t.id())
}

/// A node's icon tinted with its FGD class color, as the scene tree and the inspector header show it.
pub(crate) fn node_icon_for(project: &crate::project::Project, kind: &NodeKind) -> (&'static str, Color32) {
    let entity_color = kind.entity().and_then(|e| mapcraft_render::EntityStyle::color(project, &e.classname)).map(|c| Color32::from_rgb(c[0], c[1], c[2]));
    icons::node_icon(kind, entity_color)
}

fn label_width(w: f32) -> f32 {
    (w * LABEL_SHARE).clamp(LABEL_MIN, LABEL_MAX)
}

fn matches(filter: &str, label: &str, key: &str) -> bool {
    filter.is_empty() || label.to_lowercase().contains(filter) || key.to_lowercase().contains(filter)
}

fn filter_id() -> Id {
    Id::new("inspector-filter")
}

fn filter_text(ui: &Ui) -> String {
    ui.data(|d| d.get_temp::<String>(filter_id())).unwrap_or_default()
}

pub(crate) fn filtering(ui: &Ui) -> bool {
    !filter_text(ui).is_empty()
}

pub(crate) fn filter_box(ui: &mut Ui, text: &mut String) {
    let r = ui
        .scope(|ui| {
            ui.spacing_mut().text_edit_width = ui.available_width();
            crate::panels::search_field(ui, text, "Filter properties")
        })
        .inner;
    if !text.is_empty() {
        let slot = Rect::from_center_size(r.rect.right_center() - vec2(11.0, 0.0), Vec2::splat(18.0));
        ui.painter().rect_filled(slot.shrink(1.0), 2.0, BG);
        let clear = ui.put(slot, |ui: &mut Ui| icon_button(ui, "cross_in_square", "Clear filter", SLOT, false));
        if clear.on_hover_text("Clear filter").clicked() {
            text.clear();
        }
    }
    if (r.has_focus() || r.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        text.clear();
    }
    ui.add_space(4.0);
    let (line, _) = ui.allocate_exact_size(vec2(ui.available_width().max(0.0), 1.0), Sense::hover());
    ui.painter().rect_filled(line, 0.0, LINE);
    let f = text.trim().to_lowercase();
    ui.data_mut(|d| d.insert_temp(filter_id(), f));
}

pub(crate) fn end_filter(ui: &mut Ui) {
    ui.data_mut(|d| d.remove::<String>(filter_id()));
}

/// Weak text that truncates, for header subtitles and value cells.
pub(crate) fn weak(ui: &mut Ui, text: &str) -> Response {
    ui.add(Label::new(RichText::new(text).color(TEXT_WEAK)).truncate())
}

pub(crate) fn header(ui: &mut Ui, icon: (&str, Color32), title: Title, tip: &str, buttons: &[HeaderButton], subtitle: impl FnOnce(&mut Ui)) -> HeaderOut {
    let (renamed, clicked) = egui::Sides::new().shrink_left().height(26.0).show(
        ui,
        |ui| {
            let (r, _) = ui.allocate_exact_size(vec2(HEADER_ICON, 26.0), Sense::hover());
            icons::paint(ui, icon.0, r, HEADER_ICON, icon.1);
            match title {
                Title::Fixed(t) => {
                    let l = ui.add(Label::new(RichText::new(t).size(15.0).color(TEXT).strong()).truncate().show_tooltip_when_elided(false));
                    if !tip.is_empty() {
                        l.on_hover_text(tip);
                    }
                    false
                }
                Title::Edit(text, hint) => {
                    let edit = egui::TextEdit::singleline(text).hint_text(hint).font(egui::FontId::proportional(15.0)).desired_width(ui.available_width());
                    ui.add(edit).changed()
                }
            }
        },
        |ui| {
            ui.spacing_mut().item_spacing.x = 1.0;
            let mut clicked = None;
            for (i, b) in buttons.iter().enumerate().rev() {
                if icon_button(ui, b.icon, b.label, 22.0, b.on).on_hover_text(b.tip).clicked() {
                    clicked = Some(i);
                }
            }
            clicked
        },
    );
    ui.horizontal(|ui| {
        ui.add_space(HEADER_ICON + ui.spacing().item_spacing.x);
        subtitle(ui);
    });
    HeaderOut { renamed, clicked }
}

impl MapCraftApp {
    /// A foldable section bar; returns whether its rows show. While filtering nothing is drawn.
    pub(crate) fn section(&mut self, ui: &mut Ui, s: &Section) -> bool {
        self.section_with(ui, s).0
    }

    /// A section bar that also returns whether its action button was clicked.
    pub(crate) fn section_with(&mut self, ui: &mut Ui, s: &Section) -> (bool, bool) {
        if filtering(ui) {
            return (true, false);
        }
        ui.add_space(SECTION_GAP);
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(0.0), SECTION_H), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::CollapsingHeader, true, s.title));
        let fold = format!("{}:{}", s.key, s.closed);
        let open = self.prefs.get().inspector_folds.get(&fold).copied().unwrap_or(!s.closed);
        let painter = ui.painter();
        painter.rect_filled(rect, 3.0, if resp.hovered() { HOVER } else { HEADER });
        let chevron = Rect::from_min_size(rect.min + vec2(2.0, 0.0), vec2(16.0, SECTION_H));
        crate::chrome::chevron(painter, chevron, TEXT_WEAK, open);
        let mut x = chevron.right() + 2.0;
        if let Some((name, tint)) = s.icon {
            icons::paint(ui, name, Rect::from_min_size(pos2(x, rect.top()), vec2(16.0, SECTION_H)), 16.0, tint);
            x += 20.0;
        }
        let font = egui::TextStyle::Body.resolve(ui.style());
        let mut right = rect.right() - 8.0;
        let mut acted = false;
        if let Some(b) = &s.action {
            let r = Rect::from_center_size(pos2(rect.right() - 13.0, rect.center().y), Vec2::splat(20.0));
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r));
            acted = icon_button(&mut child, b.icon, b.label, 20.0, b.on).on_hover_text(b.tip).clicked();
            right = r.left() - 6.0;
        }
        if !s.badge.is_empty() {
            let badge = ui.painter().text(pos2(right, rect.center().y), egui::Align2::RIGHT_CENTER, &s.badge, font, TEXT_WEAK);
            right = badge.left() - 8.0;
        }
        let title = egui::WidgetText::from(RichText::new(s.title).strong()).into_galley(
            ui,
            Some(egui::TextWrapMode::Truncate),
            (right - x).max(0.0),
            egui::TextStyle::Body,
        );
        ui.painter().galley(pos2(x, rect.center().y - title.size().y / 2.0), title, TEXT);
        let resp = if s.tip.is_empty() { resp } else { resp.on_hover_text(s.tip) };
        if resp.clicked() {
            self.prefs.update(|p| {
                p.inspector_folds.insert(fold, !open);
            });
            return (!open, acted);
        }
        (open, acted)
    }

    pub(crate) fn node_icon_of(&self, kind: &NodeKind) -> (&'static str, Color32) {
        node_icon_for(&self.project, kind)
    }

    /// Edits every entity in `ids` with `f` as one undo step.
    pub(crate) fn edit_entities(&mut self, ids: &[NodeId], label: &str, coalesce: Option<&str>, f: impl Fn(&mut Entity)) {
        let mut e = self.session.map.edit();
        for id in ids {
            let Some(node) = e.node(*id) else { continue };
            let world = matches!(node.kind, NodeKind::World(_));
            let Some(mut ent) = node.kind.entity().cloned() else { continue };
            f(&mut ent);
            let kind = if world { NodeKind::World(ent) } else { NodeKind::Entity(ent) };
            if let Err(err) = e.set_kind(*id, kind) {
                self.status = err.to_string();
                return;
            }
        }
        let c = e.finish();
        self.session.apply_coalesced(label, c, coalesce);
        self.mark_changed();
    }

    /// `(targetname, classname)` of every named entity, sorted, one per name.
    pub(crate) fn target_names(&self) -> Vec<(String, String)> {
        let map = &self.session.map;
        let names = mapcraft_engine::target_names(map);
        let mut out: Vec<(String, String)> = map
            .nodes()
            .filter_map(|(_, n)| {
                let e = n.kind.entity()?;
                let t = e.get("targetname")?;
                names.contains(t).then(|| (t.to_string(), e.classname.clone()))
            })
            .collect();
        out.sort();
        out.dedup_by(|a, b| a.0 == b.0);
        out
    }

    /// `(name, description)` of the FGD inputs of the classes of the entities named `target`.
    pub(crate) fn inputs_for(&self, target: &str) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        if target.is_empty() {
            return out;
        }
        let classes: std::collections::BTreeSet<String> = self
            .session
            .map
            .nodes()
            .filter_map(|(_, n)| n.kind.entity())
            .filter(|e| e.get("targetname") == Some(target))
            .map(|e| e.classname.to_ascii_lowercase())
            .collect();
        for c in classes.iter().filter_map(|c| self.project.game.class(c)) {
            for io in c.inputs {
                if !out.iter().any(|(n, _)| n.eq_ignore_ascii_case(&io.name)) {
                    out.push((io.name, io.description));
                }
            }
        }
        out
    }
}

/// The label cell: inset label text, an optional icon, and the revert or remove slot at its right end.
fn label_cell(ui: &mut Ui, p: &Prop, width: f32) -> (Id, bool) {
    ui.allocate_ui_with_layout(vec2(width, ROW_H), Layout::right_to_left(Align::Center), |ui| {
        ui.set_min_size(vec2(width, ROW_H));
        let slot = match p.slot {
            Slot::None => false,
            Slot::Revert => icon_button(ui, "recover", &format!("Revert {}", p.label), SLOT, false).on_hover_text("Back to the default").clicked(),
            Slot::Remove => {
                let label = format!("Remove {}", p.label);
                icon_button(ui, "trash_can", &label, SLOT, false).on_hover_text(label).clicked()
            }
        };
        let id = ui
            .with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add_space(LABEL_PAD);
                if let Some((name, tint)) = p.icon {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                    icons::paint(ui, name, r, 14.0, tint);
                }
                let color = if p.warn {
                    WARN
                } else if p.val == Val::Default {
                    TEXT_WEAK
                } else {
                    TEXT
                };
                let mut rich = RichText::new(p.label).color(color);
                // Custom keys, whose label is their key, show italic.
                if p.val == Val::Mixed || (p.slot == Slot::Remove && p.label == p.key) {
                    rich = rich.italics();
                }
                let full =
                    egui::WidgetText::from(rich.clone()).into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Body).size().x;
                let elided = full > ui.available_width();
                let l = ui.add(Label::new(rich).truncate().show_tooltip_when_elided(false).sense(Sense::hover()));
                let mut tip = if elided && !p.tip.starts_with(p.label) { join(p.label, p.tip) } else { p.tip.to_string() };
                if p.val == Val::Mixed {
                    tip = join(&tip, "Values differ; editing sets them all.");
                }
                if let Some(reason) = p.readonly {
                    tip = join(&tip, reason);
                }
                if tip.is_empty() { l.id } else { l.on_hover_text(tip).id }
            })
            .inner;
        (id, slot)
    })
    .inner
}

fn join(a: &str, b: &str) -> String {
    match (a.is_empty(), b.is_empty()) {
        (_, true) => a.to_string(),
        (true, false) => b.to_string(),
        (false, false) => format!("{a}\n\n{b}"),
    }
}

/// One property: the label column then the value cell, or the value on its own line when wide.
pub(crate) fn row(ui: &mut Ui, p: &Prop, value: impl FnOnce(&mut Ui) -> Response) -> RowOut {
    if !matches(&filter_text(ui), p.label, p.key) {
        return RowOut::default();
    }
    let w = ui.available_width().max(0.0);
    let enabled = p.readonly.is_none();
    let (label, slot, r) = if p.wide {
        let (label, slot) = label_cell(ui, p, w);
        let r = ui.allocate_ui_with_layout(vec2(w, ROW_H), Layout::left_to_right(Align::Center), |ui| ui.add_enabled_ui(enabled, value).inner).inner;
        (label, slot, r)
    } else {
        ui.horizontal(|ui| {
            let (label, slot) = label_cell(ui, p, label_width(w));
            let rest = ui.available_width().max(0.0);
            let r = ui.allocate_ui_with_layout(vec2(rest, ROW_H), Layout::left_to_right(Align::Center), |ui| ui.add_enabled_ui(enabled, value).inner).inner;
            (label, slot, r)
        })
        .inner
    };
    let r = r.labelled_by(label);
    RowOut { changed: r.changed(), slot }
}

/// A raw key and value pair, both editable, with a remove slot after the value.
pub(crate) fn raw_row(ui: &mut Ui, key: &mut String, value: &mut String) -> RowOut {
    if !matches(&filter_text(ui), key, value) {
        return RowOut::default();
    }
    let w = ui.available_width().max(0.0);
    ui.horizontal(|ui| {
        let k = ui
            .allocate_ui_with_layout(vec2(label_width(w), ROW_H), Layout::left_to_right(Align::Center), |ui| {
                ui.add(egui::TextEdit::singleline(key).desired_width(f32::INFINITY))
            })
            .inner;
        let label = format!("Remove {key}");
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let slot = icon_button(ui, "trash_can", &label, SLOT, false).on_hover_text(label.as_str()).clicked();
            let v = ui.with_layout(Layout::left_to_right(Align::Center), |ui| ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY))).inner;
            RowOut { changed: k.changed() || v.changed(), slot }
        })
        .inner
    })
    .inner
}

/// A field for a new key's name and an Add key button; true when the key should be added.
pub(crate) fn add_key_row(ui: &mut Ui, key: &mut String, items: &[(String, String)]) -> bool {
    if filtering(ui) {
        return false;
    }
    let w = ui.available_width().max(0.0);
    ui.horizontal(|ui| {
        let typed = ui
            .allocate_ui_with_layout(vec2(label_width(w), ROW_H), Layout::left_to_right(Align::Center), |ui| {
                let r = suggest(ui, Id::new("inspector-new-key"), key, "new key", items, false);
                r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
            })
            .inner;
        let add = icon_button(ui, "plus", "Add key", 22.0, false).on_hover_text("Add key").clicked();
        (typed || add) && !key.trim().is_empty()
    })
    .inner
}

/// A line of weak text naming a group of rows inside a section.
pub(crate) fn subheading(ui: &mut Ui, icon: Option<(&str, Color32)>, text: &str) {
    if filtering(ui) {
        return;
    }
    ui.allocate_ui_with_layout(vec2(ui.available_width().max(0.0), 20.0), Layout::left_to_right(Align::Center), |ui| {
        ui.add_space(LABEL_PAD);
        if let Some((name, tint)) = icon {
            let (r, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
            icons::paint(ui, name, r, 14.0, tint);
        }
        weak(ui, text);
    });
}

/// A wrapping row of icon and text buttons; returns the clicked index.
pub(crate) fn actions(ui: &mut Ui, items: &[Action]) -> Option<usize> {
    if filtering(ui) {
        return None;
    }
    ui.horizontal_wrapped(|ui| {
        let mut clicked = None;
        for (i, a) in items.iter().enumerate() {
            let r = ui.add_enabled(a.enabled, icons::button(Some(a.icon), a.label));
            let r = if a.tip.is_empty() { r } else { r.on_hover_text(a.tip) };
            if r.clicked() {
                clicked = Some(i);
            }
        }
        clicked
    })
    .inner
}

/// A row of icon buttons `(icon, label, tip)` for a value cell; returns the clicked index.
pub(crate) fn icons(ui: &mut Ui, items: &[(&str, &str, &str)]) -> Option<usize> {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = 1.0;
        let mut clicked = None;
        for (i, (icon, label, tip)) in items.iter().enumerate() {
            if icon_button(ui, icon, label, 22.0, false).on_hover_text(*tip).clicked() {
                clicked = Some(i);
            }
        }
        clicked
    })
    .inner
}

pub(crate) fn note(ui: &mut Ui, kind: Note, text: &str) {
    let (color, icon) = match kind {
        Note::Info if filtering(ui) => return,
        Note::Info => (TEXT_WEAK, None),
        Note::Warn => (WARN, Some("danger")),
        Note::Error => (ERROR, Some("no_entry")),
    };
    let r = egui::Frame::new().fill(BG).corner_radius(3).inner_margin(egui::Margin { left: 9, right: 6, top: 4, bottom: 4 }).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_top(|ui| {
            if let Some(name) = icon {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                icons::paint(ui, name, r, 14.0, color);
            }
            ui.add(Label::new(RichText::new(text).color(color)).wrap());
        });
    });
    let bar = Rect::from_min_size(r.response.rect.min, vec2(3.0, r.response.rect.height()));
    ui.painter().rect_filled(bar, egui::CornerRadius { nw: 3, sw: 3, ne: 0, se: 0 }, color);
}

fn parse_finite(text: &str) -> Option<f64> {
    let text: String = text.chars().filter(|c| !c.is_whitespace()).map(|c| if c == '\u{2212}' { '-' } else { c }).collect();
    text.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Every inspector DragValue: keeps stored values outside the range, refuses non-finite text, and applies typing on Enter or focus loss.
fn drag<N: Numeric>(v: &mut N, range: RangeInclusive<N>, speed: f64) -> egui::DragValue<'_> {
    egui::DragValue::new(v).range(range).speed(speed).clamp_existing_to_range(false).custom_parser(parse_finite).update_while_editing(false)
}

/// Adds a number field that drops typed or pasted letters while it is being typed into.
pub(crate) fn number(ui: &mut Ui, d: egui::DragValue<'_>) -> Response {
    // A DragValue takes the next auto id, so this is the id its text box focuses.
    let id = ui.next_auto_id();
    if ui.memory(|m| m.has_focus(id)) {
        ui.input_mut(|i| {
            i.events.retain_mut(|e| match e {
                egui::Event::Text(t) | egui::Event::Paste(t) => {
                    t.retain(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | '\u{2212}'));
                    !t.is_empty()
                }
                _ => true,
            })
        });
    }
    ui.add(d)
}

fn unchange(r: &mut Response) {
    r.flags.set(Flags::CHANGED, false);
}

pub(crate) fn num<N: Numeric>(ui: &mut Ui, v: &mut N, range: RangeInclusive<N>, speed: f64, suffix: &str, shown: Option<&str>) -> Response {
    let old = *v;
    let mut r = ui
        .scope(|ui| {
            ui.spacing_mut().interact_size.x = ui.available_width().max(0.0);
            let mut d = drag(v, range, speed).suffix(suffix);
            if let Some(s) = shown {
                d = d.custom_formatter(move |_, _| s.to_string());
            }
            number(ui, d)
        })
        .inner;
    if r.changed() && !v.to_f64().is_finite() {
        *v = old;
        unchange(&mut r);
    }
    r
}

pub(crate) fn vec<const K: usize>(
    ui: &mut Ui,
    v: &mut [f64; K],
    axes: [(&str, Color32); K],
    range: RangeInclusive<f64>,
    speed: f64,
    suffix: &str,
    mixed: [bool; K],
) -> (Response, Option<usize>) {
    let w = ui.available_width().max(0.0);
    let field = ((w - (K.saturating_sub(1)) as f32) / K.max(1) as f32).max(0.0);
    ui.allocate_ui_with_layout(vec2(w, ROW_H), Layout::left_to_right(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 1.0;
        ui.spacing_mut().interact_size.x = field;
        let mut union: Option<Response> = None;
        let mut axis = None;
        for (i, ((letter, color), x)) in axes.iter().zip(v.iter_mut()).enumerate() {
            let old = *x;
            let mut d = drag(x, range.clone(), speed).prefix(RichText::new(format!("{letter} ")).color(*color)).suffix(suffix);
            if mixed.get(i).copied().unwrap_or(false) {
                d = d.custom_formatter(|_, _| "mixed".to_string());
            }
            let mut r = number(ui, d);
            if r.changed() {
                if x.is_finite() {
                    axis = Some(i);
                } else {
                    *x = old;
                    unchange(&mut r);
                }
            }
            union = Some(match union {
                Some(u) => u | r,
                None => r,
            });
        }
        (union.unwrap_or_else(|| ui.allocate_response(Vec2::ZERO, Sense::hover())), axis)
    })
    .inner
}

pub(crate) fn toggle(ui: &mut Ui, on: &mut bool, mixed: bool, label: &str) -> Response {
    let mut value = !mixed && *on;
    let w = ui.available_width().max(0.0);
    let r = ui.add_sized([w, ROW_H], egui::Checkbox::new(&mut value, ("On", egui::Atom::grow())).indeterminate(mixed));
    if r.changed() {
        *on = value;
    }
    let enabled = ui.is_enabled();
    r.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, value, label));
    r
}

pub(crate) fn choice(ui: &mut Ui, id: Id, value: &mut String, options: &[(&str, &str)], mixed: bool) -> Response {
    let shown = if mixed {
        RichText::new("mixed").italics().color(TEXT_WEAK)
    } else {
        match options.iter().find(|(v, _)| *v == value.as_str()) {
            Some((_, l)) => RichText::new(*l),
            None => RichText::new(value.as_str()).italics().color(TEXT_WEAK),
        }
    };
    let mut picked: Option<String> = None;
    let w = ui.available_width().max(0.0);
    let mut r = egui::ComboBox::from_id_salt(id)
        .selected_text(shown)
        .width(w)
        .truncate()
        .show_ui(ui, |ui| {
            for (v, l) in options {
                if ui.selectable_label(!mixed && *v == value.as_str(), *l).clicked() {
                    picked = Some(v.to_string());
                }
            }
        })
        .response;
    if let Some(p) = picked
        && (mixed || p != *value)
    {
        *value = p;
        r.mark_changed();
    }
    r
}

pub(crate) fn flags(ui: &mut Ui, bits: &mut u64, mixed: u64, options: &[(u64, &str)]) -> (Response, Option<(u64, bool)>) {
    let w = ui.available_width().max(0.0);
    let cols = if w >= 240.0 { 2 } else { 1 };
    let cell = ((w - 4.0 * (cols - 1) as f32) / cols as f32).max(0.0);
    let mut toggled = None;
    let mut r = ui
        .vertical(|ui| {
            ui.spacing_mut().item_spacing.y = ROW_GAP;
            for chunk in options.chunks(cols) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for (bit, label) in chunk {
                        let mix = mixed & bit != 0;
                        let mut on = !mix && *bits & bit != 0;
                        let r = ui
                            .allocate_ui_with_layout(vec2(cell, ROW_H), Layout::left_to_right(Align::Center), |ui| {
                                ui.set_width(cell);
                                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                                ui.add(egui::Checkbox::new(&mut on, *label).indeterminate(mix))
                            })
                            .inner;
                        if r.changed() {
                            *bits = if on { *bits | bit } else { *bits & !bit };
                            toggled = Some((*bit, on));
                        }
                    }
                });
            }
            let named = options.iter().fold(0, |a, (b, _)| a | b);
            let other = *bits & !named;
            if other != 0 {
                weak(ui, &format!("Other bits: {other}"));
            }
        })
        .response;
    if toggled.is_some() {
        r.mark_changed();
    }
    (r, toggled)
}

pub(crate) fn color(ui: &mut Ui, value: &mut String, unit: bool, mixed: bool) -> Response {
    let parts: Vec<f32> = value.split_whitespace().filter_map(|t| t.parse().ok()).collect();
    let scale = if unit { 255.0 } else { 1.0 };
    let mut rgb = [0u8; 3];
    for (i, c) in rgb.iter_mut().enumerate() {
        *c = parts.get(i).map_or(255.0, |v| v * scale).clamp(0.0, 255.0) as u8;
    }
    let swatch = ui
        .scope(|ui| {
            ui.spacing_mut().interact_size = Vec2::splat(ROW_H);
            ui.color_edit_button_srgb(&mut rgb)
        })
        .inner;
    if swatch.changed() {
        let rest = parts.iter().skip(3).map(|v| v.to_string());
        let out: Vec<String> = rgb.iter().map(|c| if unit { format!("{:.3}", f32::from(*c) / 255.0) } else { c.to_string() }).chain(rest).collect();
        *value = out.join(" ");
    }
    let field = text(ui, value, "", mixed && !swatch.changed());
    swatch | field
}

pub(crate) fn text(ui: &mut Ui, value: &mut String, hint: &str, mixed: bool) -> Response {
    if mixed {
        let mut buf = String::new();
        let r = ui.add(egui::TextEdit::singleline(&mut buf).hint_text("mixed").desired_width(f32::INFINITY));
        if r.changed() {
            *value = buf;
        }
        r
    } else {
        ui.add(egui::TextEdit::singleline(value).hint_text(hint).desired_width(f32::INFINITY))
    }
}

/// A text field that offers `(value, detail)` items containing what is typed; free text stays allowed.
pub(crate) fn suggest(ui: &mut Ui, id: Id, value: &mut String, hint: &str, items: &[(String, String)], mixed: bool) -> Response {
    let mut r = text(ui, value, hint, mixed);
    if r.gained_focus() {
        egui::Popup::open_id(ui.ctx(), id);
    }
    // Focus leaves on a press when an item is clicked; Enter, Tab and Esc close the list.
    if r.lost_focus() && !ui.input(|i| i.pointer.any_down()) {
        egui::Popup::close_id(ui.ctx(), id);
    }
    let typed = if mixed { String::new() } else { value.to_lowercase() };
    let shown: Vec<&(String, String)> = items.iter().filter(|(v, _)| v.to_lowercase().contains(&typed)).collect();
    if shown.is_empty() {
        return r;
    }
    let mut pick = None;
    egui::Popup::from_response(&r)
        .id(id)
        .open_memory(None::<egui::containers::SetOpenCommand>)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .width(r.rect.width())
        .show(|ui| {
            egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                for (v, detail) in shown {
                    let b = egui::Button::selectable(false, v.as_str()).right_text(RichText::new(detail.as_str()).color(TEXT_WEAK)).truncate();
                    if ui.add(b).clicked() {
                        pick = Some(v.clone());
                    }
                }
            });
        });
    if let Some(v) = pick {
        *value = v;
        egui::Popup::close_id(ui.ctx(), id);
        r.mark_changed();
    }
    r
}

/// A material field: a browse button or text field, a thumbnail and a bucket; returns the field's response and the bucket's click.
pub(crate) fn material(ui: &mut Ui, thumb: Option<TextureId>, name: &mut String, label: &str, hint: &str, mixed: bool, browse: bool) -> (Response, bool) {
    let bucket = if browse { "Apply current material" } else { "Use current material" };
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        let clicked = icon_button(ui, "bucket_fill", bucket, 22.0, false).on_hover_text(bucket).clicked();
        let (r, _) = ui.allocate_exact_size(Vec2::splat(THUMB), Sense::hover());
        match thumb {
            Some(t) => {
                ui.painter().image(t, r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            None => icons::paint(ui, "checkerboard", r, 14.0, TEXT_WEAK),
        }
        let middle = ui
            .with_layout(Layout::left_to_right(Align::Center), |ui| {
                if browse {
                    let short = name.rsplit('/').next().unwrap_or(name.as_str());
                    let rich = if mixed { RichText::new(short).color(TEXT_WEAK) } else { RichText::new(short) };
                    let w = ui.available_width().max(0.0);
                    let r = ui.add(egui::Button::new((rich, egui::Atom::grow())).truncate().min_size(vec2(w, ROW_H)));
                    let enabled = ui.is_enabled();
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
                    r.on_hover_text(name.as_str())
                } else {
                    text(ui, name, hint, mixed)
                }
            })
            .inner;
        (middle, clicked)
    })
    .inner
}

fn int_of(s: &str) -> Option<i64> {
    let t = s.trim();
    if t.is_empty() {
        return Some(0);
    }
    t.parse::<i64>().ok().or_else(|| t.parse::<f64>().ok().filter(|f| f.is_finite()).map(|f| f.round() as i64))
}

fn float_of(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() { Some(0.0) } else { t.parse::<f64>().ok().filter(|f| f.is_finite()) }
}

fn bits_of(s: &str) -> Option<u64> {
    let t = s.trim();
    if t.is_empty() { Some(0) } else { t.parse().ok() }
}

/// An X/Y/Z style editor when every value holds three numbers; changes one axis at a time.
fn vec3_value(
    ui: &mut Ui,
    values: &[String],
    axes: [(&str, Color32); 3],
    range: RangeInclusive<f64>,
    speed: f64,
    suffix: &str,
    empty_zero: bool,
) -> Option<(Response, Change)> {
    let parsed: Vec<[f64; 3]> = values.iter().map(|v| vec3_of(v, empty_zero)).collect::<Option<_>>()?;
    let mut v = parsed.first().copied().unwrap_or_default();
    let mix: [bool; 3] = std::array::from_fn(|i| parsed.iter().any(|p| p.get(i) != v.get(i)));
    let (r, axis) = vec(ui, &mut v, axes, range, speed, suffix, mix);
    let c = axis.and_then(|i| Some(Change::Axis(i, *v.get(i)?))).unwrap_or_default();
    Some((r, c))
}

/// Three numbers, with an empty value as zeros when `empty_zero`.
fn vec3_of(s: &str, empty_zero: bool) -> Option<[f64; 3]> {
    if empty_zero && s.trim().is_empty() {
        return Some([0.0; 3]);
    }
    match numbers(s)?.as_slice() {
        [a, b, c] => Some([*a, *b, *c]),
        _ => None,
    }
}

/// Whether an FGD row with these values puts its value on its own full width line.
pub(crate) fn fgd_wide(p: &mapcraft_fgd::Property, values: &[String]) -> bool {
    let all = |f: &dyn Fn(&str) -> bool| !values.is_empty() && values.iter().all(|v| f(v));
    match p.ty.as_str() {
        "flags" => all(&|v| bits_of(v).is_some()),
        "angle" => all(&|v| vec3_of(v, false).is_some()),
        "origin" | "vector" => all(&|v| vec3_of(v, true).is_some()),
        "string" | "" => {
            (p.default.as_deref().and_then(|d| vec3_of(d, false)).is_some() || values.iter().any(|v| vec3_of(v, false).is_some()))
                && all(&|v| vec3_of(v, true).is_some())
        }
        _ => false,
    }
}

/// The FGD types whose value names another entity.
pub(crate) fn is_target_type(ty: &str) -> bool {
    matches!(ty, "target_destination" | "target_name_or_class" | "filterclass")
}

/// The value widget for FGD property `p`; `values` holds each edited entity's current-or-default value.
pub(crate) fn fgd_value(ui: &mut Ui, id: Id, p: &mapcraft_fgd::Property, label: &str, values: &[String], targets: &[(String, String)]) -> (Response, Change) {
    let first = values.first().cloned().unwrap_or_default();
    let mixed = values.iter().any(|v| !same(v, &first));
    let fallback = |ui: &mut Ui, hint: &str| {
        let mut v = first.clone();
        let r = text(ui, &mut v, hint, mixed);
        let c = if r.changed() { Change::Set(v) } else { Change::None };
        (r, c)
    };
    match p.ty.as_str() {
        "integer" => {
            let parsed: Option<Vec<i64>> = values.iter().map(|v| int_of(v)).collect();
            let Some(mut n) = parsed.and_then(|v| v.first().copied()) else { return fallback(ui, "") };
            let r = num(ui, &mut n, i64::from(i32::MIN)..=i64::from(i32::MAX), 1.0, "", mixed.then_some("mixed"));
            let c = if r.changed() { Change::Set(n.to_string()) } else { Change::None };
            (r, c)
        }
        "float" => {
            let parsed: Option<Vec<f64>> = values.iter().map(|v| float_of(v)).collect();
            let Some(mut n) = parsed.and_then(|v| v.first().copied()) else { return fallback(ui, "") };
            let r = num(ui, &mut n, -LIMIT..=LIMIT, 0.1, "", mixed.then_some("mixed"));
            let c = if r.changed() { Change::Set(format!("{n}")) } else { Change::None };
            (r, c)
        }
        "boolean" => {
            let mut on = first.trim() == "1";
            let r = toggle(ui, &mut on, mixed, label);
            let c = if r.changed() { Change::Set(if on { "1" } else { "0" }.into()) } else { Change::None };
            (r, c)
        }
        "choices" if !p.choices.is_empty() => {
            let options: Vec<(&str, &str)> = p.choices.iter().map(|c| (c.value.as_str(), c.label.as_str())).collect();
            let mut v = first.clone();
            let r = choice(ui, id, &mut v, &options, mixed);
            let c = if r.changed() { Change::Set(v) } else { Change::None };
            (r, c)
        }
        "flags" => {
            let parsed: Option<Vec<u64>> = values.iter().map(|v| bits_of(v)).collect();
            let Some(parsed) = parsed else { return fallback(ui, "") };
            let mut bits = parsed.first().copied().unwrap_or(0);
            let mixed_bits = parsed.iter().fold(0, |m, b| m | (b ^ bits));
            let options: Vec<(u64, &str)> =
                p.choices.iter().filter_map(|c| Some((c.value.trim().parse::<u64>().ok().filter(|b| *b != 0)?, c.label.as_str()))).collect();
            let (r, toggled) = flags(ui, &mut bits, mixed_bits, &options);
            (r, toggled.map_or(Change::None, |(b, on)| Change::Bit(b, on)))
        }
        "color255" | "color1" => {
            let mut v = first.clone();
            let r = color(ui, &mut v, p.ty == "color1", mixed);
            let c = if r.changed() { Change::Set(v) } else { Change::None };
            (r, c)
        }
        "angle" | "origin" | "vector" => {
            let angle = p.ty == "angle";
            let (axes, range, speed, suffix) = match p.ty.as_str() {
                "angle" => (PYR, -ANGLE_LIMIT..=ANGLE_LIMIT, 1.0, "°"),
                "vector" => (XYZ, -LIMIT..=LIMIT, 0.01, ""),
                _ => (XYZ, -LIMIT..=LIMIT, 1.0, ""),
            };
            if let Some(out) = vec3_value(ui, values, axes, range, speed, suffix, !angle || p.name.eq_ignore_ascii_case("angles")) {
                return out;
            }
            let single: Option<Vec<f64>> = values.iter().map(|v| v.trim().parse::<f64>().ok().filter(|f| f.is_finite())).collect();
            match single.and_then(|v| v.first().copied()) {
                Some(mut n) if angle => {
                    let r = num(ui, &mut n, -ANGLE_LIMIT..=ANGLE_LIMIT, 1.0, "°", mixed.then_some("mixed"));
                    let c = if r.changed() { Change::Set(format!("{}", n + 0.0)) } else { Change::None };
                    (r, c)
                }
                _ => fallback(ui, ""),
            }
        }
        "string" | "" if p.default.as_deref().and_then(|d| vec3_of(d, false)).is_some() || values.iter().any(|v| vec3_of(v, false).is_some()) => {
            let text = format!("{} {}", p.display, p.description).to_lowercase();
            let (axes, angles) = match (text.find("yaw"), text.find("pitch")) {
                (Some(y), Some(pi)) if y < pi => (YPR, true),
                (Some(_), Some(_)) => (PYR, true),
                _ => (XYZ, false),
            };
            let small = values.iter().map(String::as_str).chain(p.default.as_deref()).filter_map(|v| vec3_of(v, false)).flatten().all(|n| n.abs() <= 1.0);
            let (range, speed, suffix) = match (angles, small) {
                (true, _) => (-ANGLE_LIMIT..=ANGLE_LIMIT, 1.0, "°"),
                (false, true) => (-LIMIT..=LIMIT, 0.01, ""),
                (false, false) => (-LIMIT..=LIMIT, 1.0, ""),
            };
            vec3_value(ui, values, axes, range, speed, suffix, true).unwrap_or_else(|| fallback(ui, ""))
        }
        t if is_target_type(t) => {
            let mut v = first.clone();
            let r = suggest(ui, id, &mut v, "", targets, mixed);
            let c = if r.changed() { Change::Set(v) } else { Change::None };
            (r, c)
        }
        "target_source" => fallback(ui, "name"),
        "studio" => fallback(ui, "model path"),
        "sound" => fallback(ui, "sound"),
        "sprite" => fallback(ui, "sprite"),
        _ => fallback(ui, ""),
    }
}

/// A plugin number range made safe: finite, ordered bounds.
fn bounds(min: f64, max: f64) -> (f64, f64) {
    let lo = if min.is_finite() { min } else { -LIMIT };
    let hi = if max.is_finite() { max } else { LIMIT };
    (lo.min(hi), lo.max(hi))
}

/// Rows for a plugin command's parameters, materials last; true when a value changed.
pub(crate) fn param_rows(
    ui: &mut Ui,
    salt: impl Hash + std::fmt::Debug,
    specs: &[ParamSpec],
    values: &mut Value,
    current_material: &str,
    thumb: &mut dyn FnMut(&str) -> Option<TextureId>,
) -> bool {
    if !values.is_object() {
        *values = json!({});
    }
    let base = Id::new(salt);
    let (plain, materials): (Vec<&ParamSpec>, Vec<&ParamSpec>) = specs.iter().partition(|p| p.kind != ParamKind::Material);
    let mut changed = false;
    for p in plain {
        changed |= param_row(ui, base, p, values, current_material, thumb);
    }
    if !materials.is_empty() {
        subheading(ui, None, "Materials");
        for p in materials {
            changed |= param_row(ui, base, p, values, current_material, thumb);
        }
    }
    changed
}

fn param_row(ui: &mut Ui, base: Id, p: &ParamSpec, values: &mut Value, current_material: &str, thumb: &mut dyn FnMut(&str) -> Option<TextureId>) -> bool {
    let v = values.get(&p.id).cloned().unwrap_or_else(|| p.initial.clone());
    let differs = match p.kind {
        ParamKind::Number { .. } | ParamKind::Integer { .. } => v.as_f64() != p.initial.as_f64(),
        _ => v != p.initial,
    };
    let prop = Prop {
        label: &p.label,
        key: &p.id,
        tip: &p.help,
        val: if differs { Val::Set } else { Val::Default },
        slot: if differs { Slot::Revert } else { Slot::None },
        ..Default::default()
    };
    let id = base.with(&p.id);
    let mut new: Option<Value> = None;
    let out = ui
        .push_id(&p.id, |ui| {
            row(ui, &prop, |ui| match &p.kind {
                ParamKind::Number { min, max, step } => {
                    let (lo, hi) = bounds(*min, *max);
                    let step = if step.is_finite() && *step > 0.0 { *step } else { 0.1 };
                    let mut n = v.as_f64().filter(|n| n.is_finite()).unwrap_or(lo);
                    let r = num(ui, &mut n, lo..=hi, step, "", None);
                    if r.changed() {
                        new = Some(json!(n));
                    }
                    r
                }
                ParamKind::Integer { min, max } => {
                    let (lo, hi) = ((*min).min(*max), (*min).max(*max));
                    let mut n = v.as_i64().or_else(|| v.as_f64().filter(|f| f.is_finite()).map(|f| f.round() as i64)).unwrap_or(lo);
                    let r = num(ui, &mut n, lo..=hi, 1.0, "", None);
                    if r.changed() {
                        new = Some(json!(n));
                    }
                    r
                }
                ParamKind::Toggle => {
                    let mut on = v.as_bool().unwrap_or(false);
                    let r = toggle(ui, &mut on, false, &p.label);
                    if r.changed() {
                        new = Some(json!(on));
                    }
                    r
                }
                ParamKind::Choice(options) => {
                    let pairs: Vec<(&str, &str)> = options.iter().map(|o| (o.as_str(), o.as_str())).collect();
                    let mut s = v.as_str().unwrap_or_default().to_string();
                    let r = choice(ui, id, &mut s, &pairs, false);
                    if r.changed() {
                        new = Some(json!(s));
                    }
                    r
                }
                ParamKind::Text => {
                    let mut s = v.as_str().unwrap_or_default().to_string();
                    let r = text(ui, &mut s, "", false);
                    if r.changed() {
                        new = Some(json!(s));
                    }
                    r
                }
                ParamKind::Material => {
                    let mut s = v.as_str().unwrap_or_default().to_string();
                    let t = thumb(if s.is_empty() { current_material } else { &s });
                    let (r, bucket) = material(ui, t, &mut s, &p.label, "current material", false, false);
                    if bucket {
                        new = Some(json!(current_material));
                    } else if r.changed() {
                        new = Some(json!(s));
                    }
                    r
                }
            })
        })
        .inner;
    if out.slot {
        new = Some(p.initial.clone());
    }
    match (new, values.as_object_mut()) {
        (Some(n), Some(obj)) => {
            obj.insert(p.id.clone(), n);
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_apply_to_old_values() {
        assert_eq!(Change::None.apply("1"), None);
        assert_eq!(Change::Set("x".into()).apply("1").as_deref(), Some("x"));
        assert_eq!(Change::Axis(1, 5.0).apply("1 2 3").as_deref(), Some("1 5 3"));
        assert_eq!(Change::Axis(1, 5.0).apply("").as_deref(), Some("0 5 0"));
        assert_eq!(Change::Axis(0, -0.0).apply("1 2 3").as_deref(), Some("0 2 3"));
        assert_eq!(Change::Axis(1, 5.0).apply("a b"), None);
        assert_eq!(Change::Axis(5, 5.0).apply("1 2 3"), None);
        assert_eq!(Change::Bit(4, true).apply("3").as_deref(), Some("7"));
        assert_eq!(Change::Bit(1, false).apply("3").as_deref(), Some("2"));
        assert_eq!(Change::Bit(2, true).apply("").as_deref(), Some("2"));
        assert_eq!(Change::Bit(2, false).apply("").as_deref(), Some("0"));
        for bad in ["x", "-1", "8.0"] {
            assert_eq!(Change::Bit(1, true).apply(bad), None, "{bad}");
        }
    }

    #[test]
    fn same_compares_numbers_by_value() {
        assert!(same("1", "1.0"));
        assert!(same("0 0 0", "0.0 0 0"));
        assert!(!same("1", "1 0"));
        assert!(!same("a", "b"));
    }

    #[test]
    fn label_column_width() {
        assert_eq!(label_width(224.0), 89.6);
        assert_eq!(label_width(284.0), 113.6);
        assert_eq!(label_width(600.0), LABEL_MAX);
        assert_eq!(label_width(100.0), LABEL_MIN);
    }

    #[test]
    fn filter_matches_label_or_key_ignoring_case() {
        assert!(matches("", "Origin", "origin"));
        assert!(matches("orig", "Origin", "x"));
        assert!(matches("spawn", "Flags", "SpawnFlags"));
        assert!(!matches("zzz", "Origin", "origin"));
    }

    fn once(f: impl FnMut(&mut Ui)) {
        let ctx = egui::Context::default();
        ctx.run_ui(Default::default(), f).textures_delta.clear();
    }

    #[test]
    fn showing_a_number_outside_its_range_changes_nothing() {
        let mut scale = -1.0;
        let mut changed = true;
        once(|ui| changed = num(ui, &mut scale, 0.01..=64.0, 0.01, "", None).changed());
        assert!(!changed);
        assert_eq!(scale, -1.0);
    }

    #[test]
    fn number_fields_drop_typed_and_pasted_letters() {
        let ctx = egui::Context::default();
        let mut v = 0.0;
        let frame = |events: Vec<egui::Event>, v: &mut f64| {
            let input = egui::RawInput { events, ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| {
                let id = ui.next_auto_id();
                ui.memory_mut(|m| m.request_focus(id));
                number(ui, egui::DragValue::new(v));
            });
            out.textures_delta.clear();
        };
        frame(vec![], &mut v);
        frame(vec![egui::Event::Text("4x2".into()), egui::Event::Paste("e1".into())], &mut v);
        frame(vec![], &mut v);
        assert_eq!(v, 421.0);
    }

    #[test]
    fn string_keys_holding_three_numbers_edit_per_axis() {
        let sun = mapcraft_fgd::Property {
            name: "_sun_mangle".into(),
            ty: "string".into(),
            display: "Sun direction".into(),
            description: "Yaw and pitch the sunlight travels along".into(),
            default: Some("0 -90 0".into()),
            ..Default::default()
        };
        assert!(fgd_wide(&sun, &["0 -90 0".to_string()]), "a three-number string gets the full-width axis editor");
        assert!(fgd_wide(&sun, &[String::new()]), "an unset key with a three-number default does too");
        let message = mapcraft_fgd::Property { name: "message".into(), ty: "string".into(), ..Default::default() };
        assert!(!fgd_wide(&message, &["hello there".to_string()]), "plain text stays a text field");
    }

    #[test]
    fn showing_fgd_values_changes_nothing() {
        let prop = |ty: &str, choices: Vec<mapcraft_fgd::Choice>| mapcraft_fgd::Property { name: "k".into(), ty: ty.into(), choices, ..Default::default() };
        let flag = |v: &str| mapcraft_fgd::Choice { value: v.into(), label: v.into(), default_on: None };
        let cases = [
            (prop("string", vec![]), vec!["hello".to_string()]),
            (prop("integer", vec![]), vec!["99999999999".into()]),
            (prop("integer", vec![]), vec!["4".into(), "5".into()]),
            (prop("float", vec![]), vec!["2.5".into()]),
            (prop("boolean", vec![]), vec!["1".into()]),
            (prop("choices", vec![flag("0"), flag("1")]), vec!["7".into()]),
            (prop("flags", vec![flag("1"), flag("2")]), vec![String::new()]),
            (prop("flags", vec![flag("1")]), vec!["x".into()]),
            (prop("color255", vec![]), vec!["255 0 0 200".into()]),
            (prop("color1", vec![]), vec!["1 0.5 0".into()]),
            (prop("angle", vec![]), vec!["0 90 0".into()]),
            (prop("angle", vec![]), vec!["90".into()]),
            (prop("origin", vec![]), vec![String::new()]),
            (prop("origin", vec![]), vec!["1 2".into()]),
            (prop("vector", vec![]), vec!["0 0 1".into(), "1 0 0".into()]),
            (prop("target_destination", vec![]), vec!["door".into()]),
            (prop("target_source", vec![]), vec!["name".into()]),
            (prop("studio", vec![]), vec!["m.mdl".into()]),
            (prop("sound", vec![]), vec!["s.wav".into()]),
            (prop("sprite", vec![]), vec!["s.spr".into()]),
            (prop("string", vec![]), vec!["0 -90 0".into()]),
            (
                mapcraft_fgd::Property { name: "_sun_mangle".into(), ty: "string".into(), default: Some("0 -90 0".into()), ..Default::default() },
                vec![String::new()],
            ),
        ];
        for (p, values) in cases {
            let mut change = Change::Set("unset".into());
            once(|ui| change = fgd_value(ui, Id::new("t"), &p, "K", &values, &[]).1);
            assert_eq!(change, Change::None, "{} {values:?}", p.ty);
        }
    }
}
