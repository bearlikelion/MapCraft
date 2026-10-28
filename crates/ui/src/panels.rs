use std::collections::{BTreeMap, BTreeSet};

use egui::{Align, Color32, Layout, Rect, RichText};
use mapcraft_engine::{BlendFill, Severity};
use mapcraft_fgd::{EntityClass, Property};
use mapcraft_geom::DVec2;
use mapcraft_map::{
    AntiTile, Axis, Brush, Connection, Cordon, CurveOp, Entity, Face, Group, Justify, Layer, Mirror, NodeId, NodeKind, Path, Prefab, PropertyOverride, ROOT,
};

use crate::app::{BottomTab, LeftTab, MapCraftApp};
use crate::chrome::{LINE, MIRROR, TEXT_WEAK, icon_button};
use crate::icons;
use crate::inspect::{self, Action, Change, HeaderButton, Note, Prop, Section, Slot, Title, Val, find_key};
use crate::viewport::Tool;

const ROW: f32 = 18.0;

/// What the inspector shows, picked once per frame; the first match wins.
enum InspectorView {
    Shape,
    Faces,
    World,
    Entities(Vec<NodeId>),
    Brushes(Vec<NodeId>),
    Layer(NodeId, Layer),
    Group(NodeId, Group),
    Prefab(NodeId, Prefab),
    Path(NodeId, Path),
    Many(Vec<NodeId>),
}

impl InspectorView {
    /// Each node keeps its own scroll position.
    fn salt(&self) -> u64 {
        match self {
            Self::Shape => u64::MAX,
            Self::Faces => u64::MAX - 1,
            Self::World => u64::MAX - 2,
            Self::Entities(ids) | Self::Brushes(ids) | Self::Many(ids) => ids.first().map_or(0, |id| id.0),
            Self::Layer(id, _) | Self::Group(id, _) | Self::Prefab(id, _) | Self::Path(id, _) => id.0,
        }
    }
}

/// Property edits gathered while the rows draw, applied once they are done.
#[derive(Default)]
struct EntityEdits {
    changes: Vec<(String, Change)>,
    remove: Vec<String>,
    add: Option<String>,
}

impl EntityEdits {
    fn is_empty(&self) -> bool {
        self.changes.is_empty() && self.remove.is_empty() && self.add.is_none()
    }

    /// Applies each change to `e`'s own current-or-default value, under `e`'s own spelling of the key.
    fn apply(&self, e: &mut Entity, class: Option<&EntityClass>) {
        for (key, change) in &self.changes {
            let (spelling, old) = match find_key(e, key) {
                Some((k, v)) => (k.to_string(), v.to_string()),
                None => (key.clone(), class.and_then(|c| c.property(key)).and_then(|p| p.default.clone()).unwrap_or_default()),
            };
            if let Some(new) = change.apply(&old) {
                e.set(spelling, new);
            }
        }
        for key in &self.remove {
            if let Some(k) = find_key(e, key).map(|(k, _)| k.to_string()) {
                e.remove(&k);
            }
        }
        if let Some(key) = &self.add
            && find_key(e, key).is_none()
        {
            e.set(key.clone(), "");
        }
    }
}

/// The Displacement section's Blend fill choice and the numbers each kind takes.
pub(crate) struct BlendFillRows {
    kind: String,
    amount: f64,
    slope: [f64; 2],
    /// The picked faces' lowest and highest Z, filled in when Height is first shown.
    height: Option<[f64; 2]>,
    noise_size: f64,
    seed: u32,
    noise: [f64; 2],
}

impl Default for BlendFillRows {
    fn default() -> Self {
        Self { kind: "value".into(), amount: 100.0, slope: [30.0, 45.0], height: None, noise_size: 256.0, seed: 0, noise: [0.45, 0.55] }
    }
}

impl BlendFillRows {
    fn fill(&self) -> BlendFill {
        let ([sf, st], [nf, nt]) = (self.slope, self.noise);
        match (self.kind.as_str(), self.height) {
            ("slope", _) => BlendFill::Slope { from: sf, to: st },
            ("height", Some([from, to])) => BlendFill::Height { from, to },
            ("noise", _) => BlendFill::Noise { size: self.noise_size, seed: self.seed, from: nf, to: nt },
            _ => BlendFill::Value(self.amount / 100.0),
        }
    }
}

const ANTI_TILE_TIP: &str =
    "Hides repeating textures in the views and in Godot; hex tiling suits rock, sand and grass, not bricks or tiles; Source exports these faces plain";
const ANTI_TILE_OPTIONS: [(&str, &str); 3] = [("", "Off"), ("macro", "Variation"), ("hex", "Hex tiling")];

fn anti_tile_value(mode: AntiTile) -> &'static str {
    match mode {
        AntiTile::Off => "",
        AntiTile::Macro => "macro",
        AntiTile::Hex => "hex",
    }
}

fn anti_tile_of(value: &str) -> AntiTile {
    match value {
        "macro" => AntiTile::Macro,
        "hex" => AntiTile::Hex,
        _ => AntiTile::Off,
    }
}

/// The Anti-tiling row for these faces; returns the mode picked.
fn anti_tile_row<'f>(ui: &mut egui::Ui, id: &str, faces: impl Iterator<Item = &'f Face>, pixelated: bool) -> Option<AntiTile> {
    let modes: BTreeSet<AntiTile> = faces.map(Face::anti_tile).collect();
    let mixed = modes.len() > 1;
    let mut value = anti_tile_value(modes.first().copied().unwrap_or_default()).to_string();
    let prop = Prop { label: "Anti-tiling", key: "anti_tile", tip: ANTI_TILE_TIP, val: mixed_val(mixed), ..Default::default() };
    let changed = inspect::row(ui, &prop, |ui| inspect::choice(ui, egui::Id::new(id), &mut value, &ANTI_TILE_OPTIONS, mixed)).changed;
    if pixelated && !mixed && value == "hex" {
        inspect::note(ui, Note::Info, "Pixel textures use Variation instead.");
    }
    changed.then(|| anti_tile_of(&value))
}

#[derive(Default)]
pub(crate) struct FindState {
    query: String,
    replace: String,
    selected_only: bool,
}

#[derive(Default)]
pub(crate) struct TreeSearch {
    text: String,
    /// Set by the shortcut; the field takes focus when next drawn.
    pub(crate) focus: bool,
    /// How many times Enter has stepped through the current matches.
    next: usize,
}

impl MapCraftApp {
    pub(crate) fn left_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (tab, label, id) in [
                (LeftTab::Scene, "Scene", "panel.scene"),
                (LeftTab::Tags, "Tags", "panel.tags"),
                (LeftTab::Materials, "Materials", "panel.materials"),
                (LeftTab::Entities, "Entities", "panel.entities"),
                (LeftTab::Models, "Models", "panel.models"),
            ] {
                let chord = self.chord(id);
                let tip = if chord.is_empty() { label.to_string() } else { format!("{label}  ({chord})") };
                if icon_button(ui, icons::action_icon(id).unwrap_or("help"), label, 26.0, self.left_tab == tab).on_hover_text(tip).clicked() {
                    self.run_action(id);
                }
            }
        });
        ui.separator();
        match self.left_tab {
            LeftTab::Scene => self.scene_tree(ui),
            LeftTab::Tags => self.tags_panel(ui),
            LeftTab::Materials => self.material_browser(ui),
            LeftTab::Entities => self.entity_browser(ui),
            LeftTab::Models => self.model_browser(ui),
        }
    }

    /// The tree's rows; with `shown`, only those nodes, all unfolded.
    fn scene_rows(&self, shown: Option<&BTreeSet<NodeId>>) -> Vec<(NodeId, usize)> {
        let mut rows = Vec::new();
        let mut stack = vec![(ROOT, 0usize)];
        while let Some((id, depth)) = stack.pop() {
            let Some(n) = self.session.map.get(id) else { continue };
            if shown.is_some_and(|s| !s.contains(&id)) {
                continue;
            }
            rows.push((id, depth));
            if (shown.is_some() || self.expanded.contains(&id)) && depth < 64 {
                stack.extend(n.children.iter().rev().map(|c| (*c, depth + 1)));
            }
        }
        rows
    }

    /// The selected nodes without those inside another selected node, in tree order.
    fn dragged_selection(&self, rows: &[(NodeId, usize)]) -> Vec<NodeId> {
        let sel = &self.session.selection;
        let mut ids: Vec<NodeId> = sel.iter().copied().filter(|id| *id != ROOT && !self.session.map.ancestors(*id).iter().any(|a| sel.contains(a))).collect();
        ids.sort_by_key(|id| rows.iter().position(|(r, _)| r == id).unwrap_or(usize::MAX));
        ids
    }

    /// Where the dragged rows land when dropped at `f` (0 top, 1 bottom) of `id`'s row: the
    /// parent, the index among its children, and whether they go inside `id`; `unfolded` when its children show below it.
    fn drop_target(&self, id: NodeId, f: f32, unfolded: bool) -> Option<(NodeId, usize, bool)> {
        let map = &self.session.map;
        let n = map.get(id)?;
        if self.tree_drag.contains(&id) || map.ancestors(id).iter().any(|a| self.tree_drag.contains(a)) {
            return None;
        }
        let fits = |parent: &mapcraft_map::Node| {
            !matches!(&parent.kind, NodeKind::Group(g) if g.generator.is_some() || g.curve.is_some())
                && self.tree_drag.iter().all(|d| map.get(*d).is_some_and(|dn| parent.kind.can_contain(&dn.kind)))
        };
        if fits(n) {
            // Below an unfolded row is above its first child.
            if unfolded && f >= 0.75 {
                return Some((id, 0, false));
            }
            if id == ROOT || (0.25..0.75).contains(&f) {
                return Some((id, n.children.len(), true));
            }
        }
        let parent = n.parent?;
        let siblings = map.get(parent)?;
        let at = siblings.children.iter().position(|c| *c == id)?;
        fits(siblings).then_some((parent, if f < 0.5 { at } else { at + 1 }, false))
    }

    fn scene_tree(&mut self, ui: &mut egui::Ui) {
        let mut enter = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if icon_button(ui, "plus", "New Layer", 22.0, false).on_hover_text(format!("New Layer  ({})", self.chord("layer.new"))).clicked() {
                self.run_action("layer.new");
            }
            let group = ui.add_enabled_ui(!self.session.selection.is_empty(), |ui| icon_button(ui, "Group", "Group", 22.0, false)).inner;
            if group.on_hover_text(format!("Group the selection  ({})", self.chord("tools.group"))).clicked() {
                self.run_command("tools.group");
            }
            ui.spacing_mut().text_edit_width = f32::INFINITY;
            let tip =
                format!("Name, class, targetname or material; Enter steps through the matches, Down leaves the box  ({})", self.chord("panel.scene_search"));
            let r = search_field(ui, &mut self.tree_search.text, "Search").on_hover_text(tip);
            if std::mem::take(&mut self.tree_search.focus) {
                let mut state = egui::text_edit::TextEditState::load(ui.ctx(), r.id).unwrap_or_default();
                let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(self.tree_search.text.chars().count()));
                state.cursor.set_char_range(Some(all));
                state.store(ui.ctx(), r.id);
                r.request_focus();
            }
            let input = |k| ui.input(|i| i.key_pressed(k));
            if (r.has_focus() || r.lost_focus()) && input(egui::Key::Escape) {
                self.tree_search.text.clear();
                self.tree_revealed.clear();
                r.surrender_focus();
            }
            if r.has_focus() && input(egui::Key::ArrowDown) {
                r.surrender_focus();
            }
            if r.lost_focus() && input(egui::Key::Enter) {
                enter = true;
                r.request_focus();
            }
            // Escape stays with the box instead of reaching the editor's own Escape.
            ui.memory_mut(|m| m.set_focus_lock_filter(r.id, egui::EventFilter { escape: true, horizontal_arrows: true, vertical_arrows: true, tab: false }));
            if r.changed() {
                self.tree_search.next = 0;
                if self.tree_search.text.trim().is_empty() {
                    self.tree_revealed.clear();
                }
            }
        });
        let query = self.tree_search.text.trim().to_lowercase();
        let found: BTreeMap<NodeId, String> = if query.is_empty() {
            BTreeMap::new()
        } else {
            self.session.map.nodes().filter_map(|(id, n)| Some((id, search_label(&n.kind, &query)?))).collect()
        };
        let shown: Option<BTreeSet<NodeId>> = (!query.is_empty()).then(|| {
            let mut shown = BTreeSet::new();
            for id in found.keys() {
                let mut at = Some(*id);
                while let Some(a) = at
                    && shown.insert(a)
                {
                    at = self.session.map.get(a).and_then(|n| n.parent);
                }
            }
            shown
        });
        let reveal = self.session.selection != self.tree_revealed;
        if reveal {
            for id in &self.session.selection {
                self.expanded.extend(self.session.map.ancestors(*id));
            }
            self.tree_revealed = self.session.selection.clone();
        }
        let rows = self.scene_rows(shown.as_ref());
        if shown.is_some() && rows.is_empty() {
            ui.label(RichText::new("Nothing matches").color(crate::chrome::TEXT_WEAK));
        }
        let mut action: Option<(NodeId, &'static str)> = None;
        if enter {
            let matches: Vec<NodeId> = rows.iter().map(|(id, _)| *id).filter(|id| found.contains_key(id)).collect();
            if let Some(i) = self.tree_search.next.checked_rem(matches.len())
                && let Some(id) = matches.get(i)
            {
                self.status = format!("Match {} of {}", i + 1, matches.len());
                self.tree_search.next += 1;
                action = Some((*id, "frame"));
            }
        }
        let mut drop: Option<(NodeId, usize)> = None;
        let pointer = ui.input(|i| i.pointer.latest_pos());
        let mut scroll = egui::ScrollArea::vertical().auto_shrink(false);
        let (offset, height) = self.tree_view;
        if reveal && let Some(i) = rows.iter().position(|(id, _)| self.session.selection.contains(id)) {
            let y = i as f32 * ROW;
            if y < offset || y + ROW > offset + height {
                scroll = scroll.vertical_scroll_offset((y - height / 3.0).max(0.0));
            }
        }
        ui.spacing_mut().item_spacing.y = 0.0;
        let out = scroll.show_rows(ui, ROW, rows.len(), |ui, range| {
            for &(id, depth) in rows.get(range).unwrap_or_default() {
                let Some(n) = self.session.map.get(id) else { continue };
                let hit = found.get(&id);
                let label = hit.cloned().unwrap_or_else(|| n.kind.label());
                let selected = self.session.selection.contains(&id);
                let (row, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW), egui::Sense::click_and_drag());
                resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, label.clone()));
                let painter = ui.painter_at(row);
                if selected {
                    painter.rect_filled(row, 3.0, crate::chrome::ACCENT_DIM);
                } else if resp.hovered() {
                    painter.rect_filled(row, 3.0, crate::chrome::WIDGET);
                }

                let chevron = Rect::from_min_size(egui::pos2(row.left() + 2.0 + depth as f32 * 14.0, row.top()), egui::vec2(16.0, ROW));
                let unfolded = match &shown {
                    Some(s) => n.children.iter().any(|c| s.contains(c)),
                    None => self.expanded.contains(&id) && !n.children.is_empty(),
                };
                if shown.is_some() && unfolded {
                    crate::chrome::chevron(&painter, chevron, crate::chrome::TEXT_WEAK, true);
                } else if shown.is_none() && !n.children.is_empty() {
                    let r = ui.interact(chevron, ui.id().with(("tree-chevron", id.0)), egui::Sense::click());
                    let c = if r.hovered() { crate::chrome::TEXT } else { crate::chrome::TEXT_WEAK };
                    crate::chrome::chevron(&painter, chevron, c, unfolded);
                    if r.clicked() {
                        action = Some((id, "toggle"));
                    }
                }
                let icon = Rect::from_min_size(egui::pos2(chevron.right(), row.top()), egui::vec2(16.0, ROW));
                let (name, tint) = inspect::node_icon_for(&self.project, &n.kind);
                icons::paint(ui, name, icon, icons::SIZE, if n.hidden { tint.gamma_multiply(0.5) } else { tint });

                let eye = Rect::from_min_size(egui::pos2(row.right() - 24.0, row.top()), egui::vec2(20.0, ROW));
                if id != ROOT {
                    let r = ui.interact(eye, ui.id().with(("tree-eye", id.0)), egui::Sense::click());
                    let c = if r.hovered() {
                        crate::chrome::TEXT
                    } else if n.hidden {
                        crate::chrome::TEXT_WEAK.gamma_multiply(0.6)
                    } else {
                        crate::chrome::TEXT_WEAK
                    };
                    icons::paint(ui, if n.hidden { "eye_closed" } else { "eye" }, eye, 14.0, c);
                    if r.on_hover_text(if n.hidden { "Show" } else { "Hide" }).clicked() {
                        action = Some((id, "visibility"));
                    }
                }

                let color = if self.session.active_parent == id {
                    crate::chrome::ACCENT
                } else if n.hidden {
                    crate::chrome::TEXT_WEAK.gamma_multiply(0.7)
                } else if shown.is_some() {
                    if hit.is_some() { crate::chrome::TEXT } else { crate::chrome::TEXT_WEAK }
                } else if matches!(n.kind, NodeKind::Brush(_)) {
                    crate::chrome::TEXT_WEAK
                } else {
                    crate::chrome::TEXT
                };
                let text_area = Rect::from_min_max(egui::pos2(icon.right() + 4.0, row.top()), egui::pos2(eye.left() - 2.0, row.bottom()));
                let font = egui::TextStyle::Body.resolve(ui.style());
                let job = highlight(&label, if hit.is_some() { &query } else { "" }, font, color);
                let galley = ui.fonts_mut(|f| f.layout_job(job));
                ui.painter_at(text_area).galley(egui::pos2(text_area.left(), text_area.center().y - galley.size().y / 2.0), galley, color);

                if resp.clicked() {
                    action = Some((id, if ui.input(|i| i.modifiers.command) { "toggle-select" } else { "select" }));
                }
                if resp.drag_started() && id != ROOT {
                    self.tree_drag = if selected { self.dragged_selection(&rows) } else { vec![id] };
                }
                if !self.tree_drag.is_empty()
                    && let Some(p) = pointer.filter(|p| row.contains(*p))
                    && let Some((target, index, into)) = self.drop_target(id, (p.y - row.top()) / ROW, unfolded)
                {
                    let x = row.left() + 18.0 + depth as f32 * 14.0;
                    let stroke = egui::Stroke::new(2.0, crate::chrome::ACCENT);
                    if into {
                        painter.rect_stroke(row.shrink(1.0), 3.0, stroke, egui::StrokeKind::Inside);
                    } else {
                        let y = if (p.y - row.top()) / ROW < 0.5 { row.top() } else { row.bottom() };
                        painter.line_segment([egui::pos2(x, y), egui::pos2(row.right(), y)], stroke);
                    }
                    drop = Some((target, index));
                }
                if resp.double_clicked() {
                    action = Some((id, "frame"));
                }
                resp.context_menu(|ui| {
                    if matches!(n.kind, NodeKind::World(_) | NodeKind::Layer(_) | NodeKind::Group(_))
                        && ui.add(icons::button(Some("thumbtack"), "New objects go here")).clicked()
                    {
                        action = Some((id, "activate"));
                    }
                    if id != ROOT && ui.add(icons::button(Some("trash_can"), "Delete")).clicked() {
                        action = Some((id, "delete"));
                    }
                });
            }
        });
        self.tree_view = (out.state.offset.y, out.inner_rect.height());
        if !self.tree_drag.is_empty() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            if !ui.input(|i| i.pointer.primary_down()) {
                let ids = std::mem::take(&mut self.tree_drag);
                if let Some((parent, index)) = drop {
                    if let Err(e) = self.session.move_nodes(&ids, parent, index) {
                        self.status = e.to_string();
                    }
                    self.expanded.insert(parent);
                    self.mark_changed();
                }
            }
        }
        let Some((id, what)) = action else { return };
        match what {
            "toggle" => {
                let open = !self.expanded.contains(&id);
                let ids = if ui.input(|i| i.modifiers.shift) { self.session.map.subtree(id) } else { vec![id] };
                for n in ids.into_iter().filter(|n| self.session.map.get(*n).is_some_and(|x| !x.children.is_empty())) {
                    if open {
                        self.expanded.insert(n);
                    } else {
                        self.expanded.remove(&n);
                    }
                }
            }
            "visibility" => {
                let hidden = self.session.map.get(id).is_some_and(|n| n.hidden);
                if let Err(e) = self.session.set_hidden(&[id], !hidden) {
                    self.status = e.to_string();
                }
            }
            "select" => self.session.selection = BTreeSet::from([id]),
            "toggle-select" => {
                if !self.session.selection.remove(&id) {
                    self.session.selection.insert(id);
                }
            }
            "frame" => {
                self.session.selection = BTreeSet::from([id]);
                self.frame_selection();
            }
            "activate" => self.session.active_parent = id,
            "delete" => {
                self.session.selection = BTreeSet::from([id]);
                self.run_command("edit.delete");
            }
            _ => {}
        }
        self.mark_changed();
    }

    /// The FGD's `@AutoVisGroup` sets, which follow entity classes and are not stored in the map.
    fn auto_tags(&mut self, ui: &mut egui::Ui) {
        let mut parents: Vec<(String, Vec<(String, Vec<String>)>)> = Vec::new();
        for g in &self.project.game.auto_visgroups {
            match parents.iter_mut().find(|(p, _)| *p == g.name) {
                Some((_, children)) => children.extend(g.children.iter().cloned()),
                None => parents.push((g.name.clone(), g.children.clone())),
            }
        }
        if parents.is_empty() {
            return;
        }
        ui.separator();
        ui.label(RichText::new("Auto tags").strong());
        let mut action = None;
        for (parent, children) in &parents {
            ui.collapsing(parent, |ui| {
                for (name, classes) in children {
                    let members = mapcraft_engine::entities_of(&self.session.map, classes);
                    if members.is_empty() {
                        continue;
                    }
                    let shown = !members.iter().all(|id| self.session.map.get(*id).is_some_and(|n| n.hidden));
                    ui.horizontal(|ui| {
                        if visibility_toggle(ui, shown).on_hover_text(if shown { "Hide them" } else { "Show them" }).clicked() {
                            action = Some((members.clone(), Some(shown)));
                        }
                        ui.label(name).on_hover_text(classes.join(", "));
                        ui.label(RichText::new(members.len().to_string()).weak());
                        if icon_button(ui, "cursor", "Select", 20.0, false).on_hover_text("Select them").clicked() {
                            action = Some((members.clone(), None));
                        }
                    });
                }
            });
        }
        match action {
            Some((ids, Some(hidden))) => {
                if let Err(e) = self.session.set_hidden(&ids, hidden) {
                    self.status = e.to_string();
                }
                self.mark_changed();
            }
            Some((ids, None)) => {
                self.session.selection = ids.into_iter().filter(|id| self.session.map.get(*id).is_some_and(|n| !n.locked && !n.hidden)).collect()
            }
            None => {}
        }
    }

    fn tags_panel(&mut self, ui: &mut egui::Ui) {
        let any = !self.session.selection.is_empty();
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.tag_name).hint_text("New tag").desired_width(120.0));
            if ui.add_enabled(any, icons::button(Some("plus"), "Tag selection")).clicked() {
                let name = if self.tag_name.trim().is_empty() { format!("Tag {}", self.session.map.tags.len() + 1) } else { self.tag_name.trim().to_string() };
                if let Err(e) = self.session.create_tag(&name) {
                    self.status = e.to_string();
                }
                self.tag_name.clear();
                self.mark_changed();
            }
        });
        let mut tags = self.session.map.tags.clone();
        let (mut edit, mut remove, mut select, mut membership) = (None, None, None, None);
        egui::Grid::new("tags").num_columns(2).striped(true).show(ui, |ui| {
            for (i, t) in tags.iter_mut().enumerate() {
                let count = self.session.map.nodes().filter(|(_, n)| n.tags.contains(&t.id)).count();
                if visibility_toggle(ui, !t.hidden).on_hover_text("Show or hide everything with this tag").clicked() {
                    t.hidden = !t.hidden;
                    edit = Some(if t.hidden { "Hide tag" } else { "Show tag" });
                }
                ui.horizontal(|ui| {
                    if ui.add(egui::TextEdit::singleline(&mut t.name).desired_width(110.0)).changed() {
                        edit = Some("Rename tag");
                    }
                    ui.label(RichText::new(count.to_string()).weak());
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if icon_button(ui, "cursor", "Select", 20.0, false).on_hover_text("Select everything with this tag").clicked() {
                        select = Some(t.id);
                    }
                    let add = ui.add_enabled_ui(any, |ui| icon_button(ui, "plus_sign_in_square", "Add the selection", 20.0, false)).inner;
                    if add.on_hover_text("Add the selection").clicked() {
                        membership = Some((t.id, true));
                    }
                    let take = ui.add_enabled_ui(any, |ui| icon_button(ui, "dash_in_square", "Take the selection out", 20.0, false)).inner;
                    if take.on_hover_text("Take the selection out").clicked() {
                        membership = Some((t.id, false));
                    }
                    if icon_button(ui, "trash_can", "Delete the tag", 20.0, false).on_hover_text("Delete the tag").clicked() {
                        remove = Some(i);
                    }
                });
                ui.end_row();
            }
        });
        if let Some(i) = remove {
            tags.remove(i);
            edit = Some("Delete tag");
        }
        let result = match (edit, select, membership) {
            (Some(label), _, _) => self.session.edit_tags(tags, label),
            (_, Some(t), _) => {
                self.session.select_tag(t);
                Ok(())
            }
            (_, _, Some((t, add))) => self.session.tag_selection(t, add),
            _ => Ok(()),
        };
        if edit.is_some() || select.is_some() || membership.is_some() {
            if let Err(e) = result {
                self.status = e.to_string();
            }
            self.mark_changed();
        }
        self.auto_tags(ui);

        ui.separator();
        ui.label(RichText::new("Cordons").strong());
        ui.label(RichText::new("While a cordon is on, only what touches it is shown and compiled.").weak().small());
        let mut cordons = self.session.map.cordons.clone();
        let bounds = self.selection_bounds();
        let mut label = None;
        if ui.add_enabled(bounds.is_some(), icons::button(Some("selection_square"), "New cordon around selection")).clicked()
            && let Some(b) = bounds
        {
            cordons.push(Cordon { name: format!("Cordon {}", cordons.len() + 1), bounds: b, active: true });
            label = Some("Add cordon");
        }
        let mut remove = None;
        for (i, c) in cordons.iter_mut().enumerate() {
            ui.push_id(("cordon", i), |ui| {
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut c.active, "").changed() {
                        label = Some(if c.active { "Turn cordon on" } else { "Turn cordon off" });
                    }
                    if ui.add(egui::TextEdit::singleline(&mut c.name).desired_width(110.0)).changed() {
                        label = Some("Rename cordon");
                    }
                    let fit = ui.add_enabled_ui(bounds.is_some(), |ui| icon_button(ui, "CenterView", "Fit", 20.0, false)).inner;
                    if fit.on_hover_text("Fit to the selection").clicked()
                        && let Some(b) = bounds
                    {
                        c.bounds = b;
                        label = Some("Fit cordon");
                    }
                    if icon_button(ui, "trash_can", "Delete the cordon", 20.0, false).on_hover_text("Delete the cordon").clicked() {
                        remove = Some(i);
                    }
                });
            });
        }
        if let Some(i) = remove {
            cordons.remove(i);
            label = Some("Delete cordon");
        }
        if let Some(l) = label {
            self.session.edit_cordons(cordons, l);
            self.mark_changed();
        }
    }

    fn material_browser(&mut self, ui: &mut egui::Ui) {
        if self.project.sdk.is_none() {
            ui.label("No SDK: pick one with File > New Map to browse materials.");
            return;
        }
        if self.material_list.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Loading the SDK's materials…");
            });
        }
        search_field(ui, &mut self.material_filter, "Search materials");
        let hidden = self.project.id().and_then(|id| self.prefs.get().hidden_textures.get(id).cloned()).unwrap_or_default();
        let folders = if hidden.is_empty() { "Folders".to_string() } else { format!("Folders ({} hidden)", hidden.len()) };
        egui::CollapsingHeader::new(folders).id_salt("material-folders").show(ui, |ui| self.texture_folder_toggles(ui));
        let any = !self.session.selection.is_empty() || !self.session.face_selection.is_empty();
        ui.horizontal(|ui| {
            if ui.add_enabled(any, icons::button(Some("bucket_fill"), "Apply to selection")).clicked() {
                self.run_command("tools.apply_material");
            }
            if ui
                .add_enabled(any, icons::button(Some("eraser"), "Clear material"))
                .on_hover_text("Take the material off the picked faces or the selected brushes")
                .clicked()
            {
                self.run_command("tools.clear_material");
            }
        });
        let filter = self.material_filter.to_lowercase();
        let names: Vec<crate::project::Material> = self
            .project
            .materials
            .iter()
            .filter(|m| !top_folder(&m.name).is_some_and(|f| hidden.contains(f)))
            .filter(|m| filter.is_empty() || m.name.to_lowercase().contains(&filter))
            .cloned()
            .collect();
        count_and_previews(ui, &format!("{} materials", names.len()), &mut self.show_thumbs);
        let mut chosen = None;
        let mut select = None;
        let menu = |r: &egui::Response, name: &str, select: &mut Option<(String, &'static str)>| {
            r.context_menu(|ui| {
                for (id, label) in [("edit.select_faces_by_material", "Select Faces Using It"), ("edit.select_brushes_by_material", "Select Brushes Using It")]
                {
                    if ui.add(icons::button(icons::action_icon(id), label)).clicked() {
                        *select = Some((name.to_string(), id));
                    }
                }
            });
        };
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        if self.show_thumbs {
            tile_grid(ui, &names, |ui, m| {
                let name = &m.name;
                let size = egui::Vec2::splat(crate::thumbs::SIZE as f32);
                let selected = self.session.tex.material == *name;
                let r = match self.thumbs.get(ui.ctx(), m) {
                    Some(t) => ui.add(egui::Button::image(egui::Image::new((t.id(), size))).selected(selected)),
                    None => ui.add_sized(size, egui::Button::new("…").selected(selected)),
                };
                let r = r.on_hover_text(name);
                menu(&r, name, &mut select);
                if r.clicked() {
                    chosen = Some((name.clone(), false));
                }
                if r.double_clicked() {
                    chosen = Some((name.clone(), true));
                }
                ui.add(egui::Label::new(RichText::new(name.rsplit('/').next().unwrap_or(name)).small()).truncate());
            });
        } else {
            egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, ROW, names.len(), |ui, range| {
                for m in names.get(range).unwrap_or_default() {
                    let name = &m.name;
                    let r = ui.selectable_label(self.session.tex.material == *name, name);
                    menu(&r, name, &mut select);
                    if r.clicked() {
                        chosen = Some((name.clone(), false));
                    }
                    if r.double_clicked() {
                        chosen = Some((name.clone(), true));
                    }
                }
            });
        }
        if let Some((name, id)) = select {
            self.session.tex.material = name;
            self.run_action(id);
        }
        if let Some((name, twice)) = chosen {
            let changed = self.session.tex.material != name;
            self.session.tex.material = name;
            // A double click's first click already applied it.
            let apply = if self.prefs.get().view.apply_on_click { any && (changed || !twice) } else { twice };
            if apply {
                self.run_command("tools.apply_material");
            }
        }
    }

    /// A checkbox per top-level material folder of the current SDK, saved per SDK.
    pub(crate) fn texture_folder_toggles(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.project.id().map(str::to_string) else { return };
        let folders: BTreeSet<&str> = self.project.materials.iter().filter_map(|m| top_folder(&m.name)).collect();
        if folders.is_empty() {
            ui.label(RichText::new("This SDK's materials are not in folders.").color(crate::chrome::TEXT_WEAK));
            return;
        }
        let mut hidden = self.prefs.get().hidden_textures.get(&id).cloned().unwrap_or_default();
        let before = hidden.clone();
        ui.horizontal(|ui| {
            if ui.add(icons::button(Some("eye"), "Show all").small()).clicked() {
                hidden.clear();
            }
            if ui.add(icons::button(Some("eye_closed"), "Hide all").small()).clicked() {
                hidden = folders.iter().map(|f| f.to_string()).collect();
            }
        });
        egui::ScrollArea::vertical().id_salt("texture-folder-toggles").max_height(240.0).show(ui, |ui| {
            for f in &folders {
                let mut shown = !hidden.contains(*f);
                if ui.checkbox(&mut shown, *f).changed() {
                    if shown {
                        hidden.remove(*f);
                    } else {
                        hidden.insert(f.to_string());
                    }
                }
            }
        });
        if hidden != before {
            self.prefs.update(|p| {
                p.hidden_textures.insert(id, hidden);
            });
        }
    }

    fn entity_browser(&mut self, ui: &mut egui::Ui) {
        search_field(ui, &mut self.entity_filter, "Search entities");
        let filter = self.entity_filter.to_lowercase();
        let classes: Vec<(String, bool, String, Option<Color32>)> = self
            .project
            .game
            .class_names()
            .filter(|n| filter.is_empty() || n.to_lowercase().contains(&filter))
            .filter_map(|n| self.project.game.class(n))
            .map(|c| {
                let color = c.color().map(|[r, g, b]| Color32::from_rgb(r, g, b));
                (c.name, c.kind.is_solid(), c.description, color)
            })
            .collect();
        if classes.is_empty() {
            ui.label("No entity classes. The SDK's FGD files define them.");
        }
        let mut pick = None;
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, ROW, classes.len(), |ui, range| {
            for (name, solid, desc, color) in classes.get(range).unwrap_or_default() {
                let (icon, tint) = icons::class_icon(*solid, *color);
                let mut r = ui.add(egui::Button::selectable(self.entity_class == *name, (icons::icon(icon).tint(tint), name.as_str())));
                if !desc.is_empty() {
                    r = r.on_hover_text(desc);
                }
                if r.clicked() {
                    pick = Some((name.clone(), *solid));
                }
            }
        });
        if let Some((name, solid)) = pick {
            if solid {
                let tie = self.chord("tools.tie_to_entity");
                self.status = format!(
                    "{name} picked: Tie to Entity ({tie}) turns the selected brushes into it, and Tools > Zone from Selection stands one on a platform"
                );
                self.session.default_solid_entity = name;
            } else {
                self.entity_class = name;
                self.tool = Tool::Entity;
            }
        }
    }

    pub(crate) fn bottom_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for (tab, label, icon) in
                [(BottomTab::Output, "Output", "window_terminal"), (BottomTab::Issues, "Issues", "danger"), (BottomTab::Find, "Find", "magnifying_glass")]
            {
                if ui.add(icons::selectable(self.bottom_tab == tab, Some(icon), label)).clicked() {
                    self.bottom_tab = tab;
                }
            }
            if icon_button(ui, "brush", "Clear", 22.0, false).on_hover_text("Clear the output and the problems").clicked() {
                self.output.clear();
                self.issues.clear();
            }
        });
        ui.separator();
        match self.bottom_tab {
            BottomTab::Output => {
                egui::ScrollArea::vertical().auto_shrink(false).stick_to_bottom(true).show_rows(ui, ROW, self.output.len(), |ui, range| {
                    for line in self.output.get(range).unwrap_or_default() {
                        ui.monospace(line);
                    }
                });
            }
            BottomTab::Issues => self.issues_panel(ui),
            BottomTab::Find => self.find_panel(ui),
        }
    }

    fn issues_panel(&mut self, ui: &mut egui::Ui) {
        let fixable = self.issues.iter().filter(|i| i.fix.is_some()).count();
        let mut fix: Option<Vec<mapcraft_engine::Issue>> = None;
        ui.horizontal(|ui| {
            if self.issues.is_empty() {
                ui.label("No problems found.");
            }
            if ui.add_enabled(fixable > 0, icons::button(Some("wrench"), format!("Fix all ({fixable})"))).clicked() {
                fix = Some(self.issues.clone());
            }
            if ui.add(icons::button(Some("arrows_clockwise"), "Check again")).clicked() {
                self.check_problems();
            }
        });
        let mut select = None;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            for (i, issue) in self.issues.iter().enumerate() {
                let (color, icon) = if issue.severity == Severity::Error { (Color32::LIGHT_RED, "no_entry") } else { (Color32::YELLOW, "danger") };
                ui.horizontal(|ui| {
                    ui.add(icons::icon(icon).tint(color));
                    if issue.fix.is_some() && icon_button(ui, "wrench", "Fix", 20.0, false).on_hover_text("Fix this problem").clicked() {
                        fix = self.issues.get(i).cloned().map(|x| vec![x]);
                    }
                    let label = self.session.map.get(issue.node).map_or_else(|| format!("node {}", issue.node.0), |n| n.kind.label());
                    if ui.selectable_label(false, RichText::new(format!("{label}: {}", issue.message)).color(color)).clicked() {
                        select = Some(issue.node);
                    }
                });
            }
        });
        if let Some(list) = fix {
            match self.session.fix_issues(&list) {
                Ok(n) => self.status = format!("Fixed {n} problem{}", if n == 1 { "" } else { "s" }),
                Err(e) => self.status = e.to_string(),
            }
            self.mark_changed();
            self.check_problems();
        } else if let Some(id) = select.filter(|id| self.session.map.get(*id).is_some()) {
            self.session.selection = BTreeSet::from([id]);
            self.frame_selection();
            self.mark_changed();
        }
    }

    /// Hammer's entity report and find and replace in one: list entities matching the query.
    fn find_panel(&mut self, ui: &mut egui::Ui) {
        let results = mapcraft_engine::find_entities(&self.session.map, &self.find.query);
        let mut action = None;
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.find.query).hint_text("classname, value, or key=value").desired_width(220.0));
            ui.label(format!("{} entities", results.len()));
            if ui.add_enabled(!results.is_empty(), icons::button(Some("cursor"), "Select all")).clicked() {
                action = Some("select");
            }
            ui.separator();
            ui.add(egui::TextEdit::singleline(&mut self.find.replace).hint_text("replace with").desired_width(140.0));
            ui.checkbox(&mut self.find.selected_only, "In selection");
            let can = !self.find.query.is_empty() && !self.find.query.contains('=');
            if ui
                .add_enabled(can, icons::button(Some("arrow_right_arrow_left"), "Replace in values"))
                .on_hover_text("Replaces the text in key values and output targets")
                .clicked()
            {
                action = Some("replace");
            }
        });
        let mut select = None;
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, ROW, results.len(), |ui, range| {
            for id in results.get(range).unwrap_or_default() {
                let Some(n) = self.session.map.get(*id) else { continue };
                if ui.selectable_label(self.session.selection.contains(id), n.kind.label()).clicked() {
                    select = Some(*id);
                }
            }
        });
        match action {
            Some("select") => {
                self.session.selection = results.into_iter().collect();
                self.mark_changed();
            }
            Some(_) => {
                let (find, replace, scope) = (self.find.query.clone(), self.find.replace.clone(), self.find.selected_only);
                self.status = match self.session.replace_values(&find, &replace, scope) {
                    Ok(n) => format!("Replaced {n} value{}", if n == 1 { "" } else { "s" }),
                    Err(e) => e.to_string(),
                };
                self.mark_changed();
            }
            None => {}
        }
        if let Some(id) = select {
            self.session.selection = BTreeSet::from([id]);
            self.frame_selection();
            self.mark_changed();
        }
    }

    pub(crate) fn inspector(&mut self, ui: &mut egui::Ui) {
        let focus = ui.memory(|m| m.focused());
        if ui.input(|i| i.pointer.any_pressed()) || focus != self.inspector_focus {
            self.session.end_coalesce();
        }
        self.inspector_focus = focus;
        let view = self.inspector_view();
        self.view_header(ui, &view);
        inspect::filter_box(ui, &mut self.inspector_filter);
        self.uv_panel.rect = Rect::NOTHING;
        egui::ScrollArea::vertical().id_salt(("inspector", view.salt())).auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = inspect::ROW_GAP;
            self.view_body(ui, &view);
        });
        inspect::end_filter(ui);
    }

    fn inspector_view(&self) -> InspectorView {
        if self.tool == Tool::Block && self.plugin_shape.is_some() {
            return InspectorView::Shape;
        }
        if self.tool == Tool::Face {
            return InspectorView::Faces;
        }
        let map = &self.session.map;
        let nodes: Vec<(NodeId, &NodeKind)> = self.session.selection.iter().filter_map(|id| Some((*id, &map.get(*id)?.kind))).collect();
        let ids: Vec<NodeId> = nodes.iter().map(|(id, _)| *id).collect();
        if ids.is_empty() || ids == [ROOT] {
            return InspectorView::World;
        }
        let class = |k: &NodeKind| match k {
            NodeKind::Entity(e) => Some(e.classname.to_ascii_lowercase()),
            _ => None,
        };
        let first = nodes.first().and_then(|(_, k)| class(k));
        if first.is_some() && nodes.iter().all(|(_, k)| class(k) == first) {
            return InspectorView::Entities(ids);
        }
        if nodes.iter().all(|(_, k)| matches!(k, NodeKind::Brush(_))) {
            return InspectorView::Brushes(ids);
        }
        if let [(id, kind)] = nodes.as_slice() {
            match kind {
                NodeKind::Layer(l) => return InspectorView::Layer(*id, l.clone()),
                NodeKind::Group(g) => return InspectorView::Group(*id, g.clone()),
                NodeKind::Prefab(p) => return InspectorView::Prefab(*id, p.clone()),
                NodeKind::Path(p) => return InspectorView::Path(*id, p.clone()),
                _ => {}
            }
        }
        InspectorView::Many(ids)
    }

    fn view_header(&mut self, ui: &mut egui::Ui, view: &InspectorView) {
        match view {
            InspectorView::Shape => self.shape_header(ui),
            InspectorView::Faces => self.faces_header(ui),
            InspectorView::World => self.entities_header(ui, &[ROOT]),
            InspectorView::Entities(ids) => self.entities_header(ui, ids),
            InspectorView::Brushes(ids) => self.brushes_header(ui, ids),
            InspectorView::Layer(id, l) => self.layer_header(ui, *id, l),
            InspectorView::Group(id, g) => self.group_header(ui, *id, g),
            InspectorView::Prefab(_, p) => self.prefab_header(ui, p),
            InspectorView::Path(_, p) => self.path_header(ui, p),
            InspectorView::Many(ids) => self.many_header(ui, ids),
        }
    }

    fn view_body(&mut self, ui: &mut egui::Ui, view: &InspectorView) {
        match view {
            InspectorView::Shape => self.shape_body(ui),
            InspectorView::Faces => self.faces_body(ui),
            InspectorView::World => self.entities_body(ui, &[ROOT]),
            InspectorView::Entities(ids) => self.entities_body(ui, ids),
            InspectorView::Brushes(_) => self.brushes_section(ui, false),
            InspectorView::Layer(id, _) => self.layer_body(ui, *id),
            InspectorView::Group(id, _) => self.group_body(ui, *id),
            InspectorView::Prefab(id, _) => self.prefab_body(ui, *id),
            InspectorView::Path(id, _) => self.path_body(ui, *id),
            InspectorView::Many(ids) => self.many_body(ui, ids),
        }
    }

    fn entities_header(&mut self, ui: &mut egui::Ui, ids: &[NodeId]) {
        let Some(&first) = ids.first() else { return };
        let Some(node) = self.session.map.get(first) else { return };
        let Some(ent) = node.kind.entity().cloned() else { return };
        let world = matches!(node.kind, NodeKind::World(_));
        let solid = !node.children.is_empty();
        let icon = self.node_icon_of(&node.kind);
        let class = self.project.game.class(&ent.classname);
        let description = class.as_ref().map(|c| c.description.clone()).unwrap_or_default();
        let single = ids.len() == 1;
        let name_key = if single && !world {
            find_key(&ent, "targetname").map(|(k, _)| k.to_string()).or_else(|| class.as_ref().and_then(|c| c.property("targetname")).map(|p| p.name.clone()))
        } else {
            None
        };
        let mut name = find_key(&ent, "targetname").map(|(_, v)| v.to_string()).unwrap_or_default();
        let classes: Vec<String> = if world {
            Vec::new()
        } else {
            let game = &self.project.game;
            game.class_names().filter(|n| game.class(n).is_some_and(|c| c.kind.is_solid() == solid)).map(String::from).collect()
        };
        let mut classname = ent.classname.clone();
        let fixed = if world {
            "World".to_string()
        } else if !single {
            format!("{} entities", ids.len())
        } else if solid {
            "Brush entity".to_string()
        } else {
            "Point entity".to_string()
        };
        let title = if name_key.is_some() { Title::Edit(&mut name, "unnamed") } else { Title::Fixed(&fixed) };
        let smart_tip = format!("Smart edit  ({})  Off shows the raw keys", self.chord("inspector.smart_edit"));
        let mut buttons = Vec::new();
        if !description.is_empty() {
            buttons.push(HeaderButton { icon: "help", label: "Class help", tip: "Class help", on: self.inspector_help });
        }
        if single {
            buttons.push(HeaderButton { icon: "sliders", label: "Smart edit", tip: &smart_tip, on: self.smart_edit });
        }
        let out = inspect::header(ui, icon, title, "", &buttons, |ui| {
            if world {
                inspect::weak(ui, "worldspawn");
            } else if classes.is_empty() {
                ui.add(egui::TextEdit::singleline(&mut classname).desired_width(f32::INFINITY));
            } else {
                let w = ui.available_width();
                egui::ComboBox::from_id_salt("inspector-class").selected_text(classname.clone()).width(w).height(400.0).truncate().show_ui(ui, |ui| {
                    for n in &classes {
                        ui.selectable_value(&mut classname, n.clone(), n);
                    }
                });
            }
        });
        let coalesce = if single { format!("edit:{}", first.0) } else { format!("edit-many:{}", first.0) };
        match out.clicked.and_then(|i| buttons.get(i)).map(|b| b.label) {
            Some("Class help") => self.inspector_help = !self.inspector_help,
            Some(_) => self.run_action("inspector.smart_edit"),
            None => {}
        }
        if out.renamed
            && let Some(key) = name_key
        {
            self.edit_entities(ids, "Edit entity", Some(&coalesce), move |e| e.set(key.clone(), name.clone()));
        }
        if classname != ent.classname {
            self.edit_entities(ids, "Change class", Some(&coalesce), move |e| e.classname = classname.clone());
        }
        if self.inspector_help && !description.is_empty() {
            inspect::note(ui, Note::Info, &description);
        }
    }

    fn entities_body(&mut self, ui: &mut egui::Ui, ids: &[NodeId]) {
        let ents: Vec<(NodeId, Entity)> = ids.iter().filter_map(|id| Some((*id, self.session.map.get(*id)?.kind.entity()?.clone()))).collect();
        let Some((first, original)) = ents.first().cloned() else { return };
        let Some(node) = self.session.map.get(first) else { return };
        let world = matches!(node.kind, NodeKind::World(_));
        let solid = !node.children.is_empty();
        let single = ents.len() == 1;
        let point = single && !world && !solid;
        let class = self.project.game.class(&original.classname);
        let named = single && !world && (find_key(&original, "targetname").is_some() || class.as_ref().is_some_and(|c| c.property("targetname").is_some()));
        let angles = point && class.as_ref().is_some_and(|c| c.property("angles").is_some());
        let hidden = |k: &str| {
            (k.eq_ignore_ascii_case("origin") && (solid || point))
                || (angles && k.eq_ignore_ascii_case("angles"))
                || (named && k.eq_ignore_ascii_case("targetname"))
        };
        let targets = self.target_names();
        let mut edits = EntityEdits::default();
        let mut e = original.clone();
        let mut dirty = false;

        if point && self.section(ui, &Section { key: "entity.transform", title: "Transform", ..Default::default() }) {
            let origin = class.as_ref().and_then(|c| c.property("origin")).cloned().unwrap_or_else(|| Property { name: "origin".into(), ..Default::default() });
            self.fgd_row(ui, &ents, &Property { ty: "origin".into(), ..origin }, Some("Origin"), &targets, &mut edits);
            if let Some(p) = class.as_ref().and_then(|c| c.property("angles")) {
                self.fgd_row(ui, &ents, &Property { ty: "angle".into(), ..p.clone() }, Some("Angles"), &targets, &mut edits);
            }
        }

        if self.section(ui, &Section { key: "entity.properties", title: "Properties", ..Default::default() }) {
            let mut items = Vec::new();
            if single && !self.smart_edit {
                let mut remove = None;
                for (i, (k, v)) in e.properties.iter_mut().enumerate() {
                    let out = ui.push_id(("raw", i), |ui| inspect::raw_row(ui, k, v)).inner;
                    dirty |= out.changed;
                    if out.slot {
                        remove = Some(i);
                    }
                }
                if let Some(i) = remove.filter(|i| *i < e.properties.len()) {
                    e.properties.remove(i);
                    dirty = true;
                }
            } else {
                if let Some(c) = &class {
                    for p in c.properties.iter().filter(|p| !hidden(&p.name)) {
                        self.fgd_row(ui, &ents, p, None, &targets, &mut edits);
                    }
                    items = c.properties.iter().filter(|p| find_key(&original, &p.name).is_none()).map(|p| (p.name.clone(), p.display.clone())).collect();
                }
                self.custom_rows(ui, &ents, class.as_ref(), &hidden, &targets, &mut edits);
            }
            let key_id = egui::Id::new(("inspector-new-key", first.0));
            let mut key: String = ui.data_mut(|d| d.get_temp(key_id).unwrap_or_default());
            if inspect::add_key_row(ui, &mut key, &items) {
                let k = key.trim().to_string();
                if ents.iter().all(|(_, ent)| find_key(ent, &k).is_some()) {
                    self.status = format!("{k} is already set");
                } else {
                    edits.add = Some(k);
                    key.clear();
                }
            }
            ui.data_mut(|d| d.insert_temp(key_id, key));
        }

        if single && !world {
            dirty |= self.outputs_section(ui, &mut e, class.as_ref(), &targets);
            self.inputs_section(ui, first);
        }

        if single {
            edits.apply(&mut e, class.as_ref());
            if (dirty || !edits.is_empty()) && e != original {
                let kind = if world { NodeKind::World(e) } else { NodeKind::Entity(e) };
                self.set_kind(first, kind, "Edit entity");
            }
        } else if !edits.is_empty() {
            let ids: Vec<NodeId> = ents.iter().map(|(id, _)| *id).collect();
            self.edit_entities(&ids, "Edit entities", Some(&format!("edit-many:{}", first.0)), move |ent| edits.apply(ent, class.as_ref()));
        }
        if solid && !world {
            self.brushes_section(ui, false);
        }
    }

    /// One row for FGD property `p` across `ents`; edits are gathered into `edits`.
    fn fgd_row(
        &mut self,
        ui: &mut egui::Ui,
        ents: &[(NodeId, Entity)],
        p: &Property,
        label: Option<&str>,
        targets: &[(String, String)],
        edits: &mut EntityEdits,
    ) {
        let label = label.unwrap_or(if p.display.is_empty() { p.name.as_str() } else { p.display.as_str() });
        let tip = if p.description.is_empty() { format!("{label} ({})", p.name) } else { format!("{label} ({})\n\n{}", p.name, p.description) };
        let values: Vec<String> =
            ents.iter().map(|(_, e)| find_key(e, &p.name).map(|(_, v)| v.to_string()).or_else(|| p.default.clone()).unwrap_or_default()).collect();
        let first = values.first().cloned().unwrap_or_default();
        let mixed = values.iter().any(|v| !inspect::same(v, &first));
        let default = ents.iter().all(|(_, e)| find_key(e, &p.name).is_none_or(|(_, v)| p.default.as_deref().is_some_and(|d| inspect::same(v, d))));
        let names: BTreeSet<&str> = targets.iter().map(|(n, _)| n.as_str()).collect();
        let warn = !mixed && mapcraft_engine::is_broken_link(Some(&self.project.game), Some(&p.ty), &names, &p.name, &first);
        let prop = Prop {
            label,
            key: &p.name,
            tip: &tip,
            val: if mixed {
                Val::Mixed
            } else if default {
                Val::Default
            } else {
                Val::Set
            },
            slot: if default || p.name.eq_ignore_ascii_case("origin") { Slot::None } else { Slot::Revert },
            readonly: p.readonly.then_some("Read only in the FGD."),
            warn,
            wide: inspect::fgd_wide(p, &values),
            icon: None,
        };
        let id = egui::Id::new(("fgd", ents.first().map_or(0, |(id, _)| id.0), &p.name));
        let mut change = Change::None;
        let out = if matches!(p.ty.as_str(), "material" | "decal") {
            let ctx = ui.ctx().clone();
            let thumb = inspect::material_thumb(&self.project, &mut self.thumbs, &ctx, &first);
            let current = self.session.tex.material.clone();
            let mut v = first.clone();
            inspect::row(ui, &prop, |ui| {
                let (r, bucket) = inspect::material(ui, thumb, &mut v, label, "material", mixed, false);
                if bucket {
                    change = Change::Set(current);
                } else if r.changed() {
                    change = Change::Set(v.clone());
                }
                r
            })
        } else {
            inspect::row(ui, &prop, |ui| {
                let (r, c) = inspect::fgd_value(ui, id, p, label, &values, targets);
                change = c;
                r
            })
        };
        if out.slot {
            match &p.default {
                Some(d) => edits.changes.push((p.name.clone(), Change::Set(d.clone()))),
                None => edits.remove.push(p.name.clone()),
            }
        } else if change != Change::None {
            edits.changes.push((p.name.clone(), change));
        }
    }

    /// Rows for keys the FGD does not define, present on any of `ents`.
    fn custom_rows(
        &mut self,
        ui: &mut egui::Ui,
        ents: &[(NodeId, Entity)],
        class: Option<&EntityClass>,
        hidden: &dyn Fn(&str) -> bool,
        targets: &[(String, String)],
        edits: &mut EntityEdits,
    ) {
        let mut keys: Vec<String> = Vec::new();
        for (_, ent) in ents {
            for (k, _) in &ent.properties {
                if class.is_none_or(|c| c.property(k).is_none()) && !hidden(k) && !keys.iter().any(|x| x.eq_ignore_ascii_case(k)) {
                    keys.push(k.clone());
                }
            }
        }
        let names: BTreeSet<&str> = targets.iter().map(|(n, _)| n.as_str()).collect();
        let first = ents.first().map_or(0, |(id, _)| id.0);
        for k in &keys {
            let values: Vec<String> = ents.iter().map(|(_, ent)| find_key(ent, k).map_or_else(String::new, |(_, v)| v.to_string())).collect();
            let mut v = values.first().cloned().unwrap_or_default();
            let mixed = values.iter().any(|x| *x != v);
            let link = mapcraft_engine::LINK_KEYS.iter().any(|l| l.eq_ignore_ascii_case(k));
            let prop = Prop {
                label: k,
                key: k,
                tip: "Not in the SDK's FGDs",
                val: if mixed { Val::Mixed } else { Val::Set },
                slot: Slot::Remove,
                warn: !mixed && mapcraft_engine::is_broken_link(None, None, &names, k, &v),
                ..Default::default()
            };
            let id = egui::Id::new(("inspector-key", first, k));
            let out =
                inspect::row(ui, &prop, |ui| if link { inspect::suggest(ui, id, &mut v, "", targets, mixed) } else { inspect::text(ui, &mut v, "", mixed) });
            if out.slot {
                edits.remove.push(k.clone());
            } else if out.changed {
                edits.changes.push((k.clone(), Change::Set(v)));
            }
        }
    }

    /// The entity's outputs, edited in place on `e`; true when one changed.
    fn outputs_section(&mut self, ui: &mut egui::Ui, e: &mut Entity, class: Option<&EntityClass>, targets: &[(String, String)]) -> bool {
        let count = e.connections.len();
        if !self.section(ui, &Section { key: "entity.outputs", title: "Outputs", badge: count.to_string(), closed: count == 0, ..Default::default() }) {
            return false;
        }
        let outputs: Vec<(String, String)> = class.map(|c| c.outputs.iter().map(|o| (o.name.clone(), o.description.clone())).collect()).unwrap_or_default();
        let filtering = inspect::filtering(ui);
        let mut dirty = false;
        let mut remove = None;
        for (i, c) in e.connections.iter_mut().enumerate() {
            let inputs = self.inputs_for(&c.target);
            let number = if filtering { format!(" {}", i + 1) } else { String::new() };
            let (changed, gone) = ui
                .push_id(("output", i), |ui| {
                    if filtering {
                        connection_rows(ui, c, &number, &outputs, targets, &inputs)
                    } else {
                        egui::Frame::new()
                            .fill(ui.visuals().faint_bg_color)
                            .stroke(egui::Stroke::new(1.0, LINE))
                            .corner_radius(4)
                            .inner_margin(4)
                            .show(ui, |ui| connection_rows(ui, c, &number, &outputs, targets, &inputs))
                            .inner
                    }
                })
                .inner;
            dirty |= changed;
            if gone {
                remove = Some(i);
            }
        }
        if let Some(i) = remove.filter(|i| *i < e.connections.len()) {
            e.connections.remove(i);
            dirty = true;
        }
        if inspect::actions(ui, &[Action { icon: "plus", label: "Add output", tip: "Wire one of this entity's outputs to another entity", enabled: true }])
            .is_some()
        {
            e.connections.push(Connection {
                output: outputs.first().map(|(n, _)| n.clone()).unwrap_or_default(),
                target: String::new(),
                input: String::new(),
                param: String::new(),
                delay: 0.0,
                times_to_fire: -1,
            });
            dirty = true;
        }
        dirty
    }

    fn inputs_section(&mut self, ui: &mut egui::Ui, id: NodeId) {
        let inputs = self.inputs_to(id);
        if !self.section(ui, &Section { key: "entity.inputs", title: "Inputs", badge: inputs.len().to_string(), closed: true, ..Default::default() }) {
            return;
        }
        if inputs.is_empty() {
            inspect::note(ui, Note::Info, "No outputs target this entity.");
        }
        let mut go = None;
        for (i, (src, c)) in inputs.iter().enumerate() {
            let Some(n) = self.session.map.get(*src) else { continue };
            let name = n.kind.label();
            let icon = self.node_icon_of(&n.kind);
            let param = if c.param.is_empty() { String::new() } else { format!(" ({})", c.param) };
            let detail = format!("{} to {}{param}", c.output, c.input);
            let select = format!("Select {name}");
            ui.push_id(("input", i), |ui| {
                inspect::row(ui, &Prop { label: &name, key: &detail, icon: Some(icon), ..Default::default() }, |ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if icon_button(ui, "cursor", &select, 22.0, false).on_hover_text(select.as_str()).clicked() {
                            go = Some(*src);
                        }
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| inspect::weak(ui, &detail)).inner
                    })
                    .inner
                });
            });
        }
        if let Some(src) = go {
            self.session.selection = BTreeSet::from([src]);
            self.mark_changed();
        }
    }

    fn brushes_header(&mut self, ui: &mut egui::Ui, ids: &[NodeId]) {
        let map = &self.session.map;
        let owners: BTreeSet<Option<String>> = ids.iter().map(|id| owner_class(map, *id)).collect();
        let subtitle = match owners.iter().next() {
            Some(owner) if owners.len() == 1 => owner.clone().unwrap_or_else(|| "world".to_string()),
            _ => "mixed owners".to_string(),
        };
        let title = if ids.len() == 1 { "Brush".to_string() } else { format!("{} brushes", ids.len()) };
        let icon = self.node_icon_of(&NodeKind::Brush(Brush::default()));
        inspect::header(ui, icon, Title::Fixed(&title), "", &[], |ui| {
            inspect::weak(ui, &subtitle);
        });
    }

    /// Type, material and texture rows for every selected brush, entity and group brushes included.
    fn brushes_section(&mut self, ui: &mut egui::Ui, closed: bool) {
        let ids = self.session.selected_brushes();
        if ids.is_empty() {
            return;
        }
        if !self.section(ui, &Section { key: "brushes", title: "Brushes", badge: ids.len().to_string(), closed, ..Default::default() }) {
            return;
        }
        let map = &self.session.map;
        let classes: Vec<&'static str> =
            ["func_detail", "func_illusionary", "func_wall"].into_iter().filter(|c| self.project.game.class(c).is_some_and(|d| d.kind.is_solid())).collect();
        let owners: BTreeSet<Option<String>> = ids.iter().map(|id| owner_class(map, *id)).collect();
        let current: Option<Option<&'static str>> = match owners.iter().next() {
            Some(None) if owners.len() == 1 => Some(None),
            Some(Some(c)) if owners.len() == 1 => classes.iter().find(|x| x.eq_ignore_ascii_case(c)).map(|x| Some(*x)),
            _ => None,
        };
        let tips: Vec<String> = classes.iter().map(|c| self.project.game.class(c).map(|d| d.description).unwrap_or_default()).collect();
        let mut options: Vec<(Option<Option<&'static str>>, &str, &str)> = vec![(Some(None), "World", "Plain world brushes")];
        for (c, tip) in classes.iter().zip(&tips) {
            options.push((Some(Some(*c)), c.strip_prefix("func_").unwrap_or(c), tip.as_str()));
        }
        let faces: Vec<&Face> = ids.iter().filter_map(|id| map.get(*id)?.kind.brush()).flat_map(|b| &b.faces).collect();
        let materials: BTreeSet<&str> = faces.iter().map(|f| f.material.as_str()).collect();
        let mixed_material = materials.len() != 1;
        let mut name = match materials.iter().next() {
            Some(m) if !mixed_material => m.to_string(),
            _ => format!("{} materials", materials.len()),
        };
        let first_scale = faces.first().map_or(1.0, |f| f.u.scale.abs());
        let mixed_scale = faces.iter().any(|f| f.u.scale.abs() != first_scale || f.v.scale.abs() != first_scale);
        let ctx = ui.ctx().clone();
        let thumb = if mixed_material { None } else { inspect::material_thumb(&self.project, &mut self.thumbs, &ctx, &name) };

        let mut pick = current;
        let mut retyped = false;
        inspect::row(ui, &Prop { label: "Type", key: "classname", wide: true, ..Default::default() }, |ui| {
            let mut r = ui.scope(|ui| retyped = Self::segmented(ui, &mut pick, &options)).response;
            if retyped {
                r.mark_changed();
            }
            r
        });
        let (mut browse, mut bucket) = (false, false);
        let material_prop = Prop { label: "Material", key: "material", val: if mixed_material { Val::Mixed } else { Val::Set }, ..Default::default() };
        inspect::row(ui, &material_prop, |ui| {
            let (r, b) = inspect::material(ui, thumb, &mut name, "Material", "", mixed_material, true);
            (browse, bucket) = (r.clicked(), b);
            r
        });
        let mut scale = first_scale;
        let scale_prop = Prop { label: "Texture scale", key: "scale", val: if mixed_scale { Val::Mixed } else { Val::Set }, ..Default::default() };
        let rescaled = inspect::row(ui, &scale_prop, |ui| inspect::num(ui, &mut scale, 0.01..=64.0, 0.01, "", mixed_scale.then_some("mixed"))).changed;
        let tip = "Shift the texture by 16 texels";
        let mut shift = None;
        inspect::row(ui, &Prop { label: "Shift", key: "shift", ..Default::default() }, |ui| {
            let items =
                [("arrow_left", "Shift left", tip), ("arrow_right", "Shift right", tip), ("arrow_up", "Shift up", tip), ("arrow_down", "Shift down", tip)];
            ui.scope(|ui| shift = inspect::icons(ui, &items)).response
        });
        let mut align = None;
        inspect::row(ui, &Prop { label: "Align", key: "align", ..Default::default() }, |ui| {
            ui.scope(|ui| align = inspect::icons(ui, &[("globe", "Align to world", "Align to world"), ("face", "Align to face", "Align to face")])).response
        });
        let anti_tile = anti_tile_row(ui, "brushes-anti-tile", faces.iter().copied(), faces.iter().any(|f| self.project.pixel_face(f)));

        if retyped && let Some(class) = pick {
            if let Err(e) = self.session.set_brush_class(class) {
                self.status = e.to_string();
            }
            self.mark_changed();
        }
        if browse {
            self.run_action("panel.materials");
        }
        if bucket {
            self.run_command("tools.apply_material");
        }
        if rescaled {
            self.edit_brushes("Texture scale", Some("brush-scale"), move |b| {
                for f in &mut b.faces {
                    f.u.scale = scale.copysign(f.u.scale);
                    f.v.scale = scale.copysign(f.v.scale);
                }
            });
        }
        if let Some((du, dv)) = shift.and_then(|i| [(-16.0, 0.0), (16.0, 0.0), (0.0, 16.0), (0.0, -16.0)].get(i).copied()) {
            self.edit_brushes("Shift texture", None, move |b| {
                for f in &mut b.faces {
                    f.u.offset += du;
                    f.v.offset += dv;
                }
            });
        }
        match align {
            Some(0) => self.edit_brushes("Align to world", None, |b| b.faces.iter_mut().for_each(|f| f.align_to_world())),
            Some(1) => self.edit_brushes("Align to face", None, |b| b.faces.iter_mut().for_each(|f| f.align_to_face())),
            _ => {}
        }
        if let Some(mode) = anti_tile {
            self.edit_brushes("Anti-tiling", None, move |b| b.faces.iter_mut().for_each(|f| f.set_anti_tile(mode)));
        }
    }

    fn many_header(&mut self, ui: &mut egui::Ui, ids: &[NodeId]) {
        let mut counts = [
            ("entity", "entities", 0),
            ("brush", "brushes", 0),
            ("group", "groups", 0),
            ("layer", "layers", 0),
            ("prefab", "prefabs", 0),
            ("path", "paths", 0),
        ];
        for id in ids {
            let i = match self.session.map.get(*id).map(|n| &n.kind) {
                Some(NodeKind::Entity(_)) => 0,
                Some(NodeKind::Brush(_)) => 1,
                Some(NodeKind::Group(_)) => 2,
                Some(NodeKind::Layer(_)) => 3,
                Some(NodeKind::Prefab(_)) => 4,
                Some(NodeKind::Path(_)) => 5,
                _ => continue,
            };
            if let Some(c) = counts.get_mut(i) {
                c.2 += 1;
            }
        }
        let subtitle = counts
            .iter()
            .filter(|c| c.2 > 0)
            .map(|(one, many, n)| if *n == 1 { format!("1 {one}") } else { format!("{n} {many}") })
            .collect::<Vec<_>>()
            .join(", ");
        inspect::header(ui, ("selection_square", TEXT_WEAK), Title::Fixed(&format!("{} selected", ids.len())), "", &[], |ui| {
            inspect::weak(ui, &subtitle);
        });
    }

    fn many_body(&mut self, ui: &mut egui::Ui, ids: &[NodeId]) {
        if ids.iter().any(|id| self.session.map.get(*id).is_some_and(|n| matches!(n.kind, NodeKind::Entity(_)))) {
            ui.add_space(4.0);
            inspect::note(ui, Note::Info, "Select entities of one class to edit their properties together.");
        }
        self.brushes_section(ui, false);
    }

    /// The thumbtack header button: puts new objects in `id`, or back in the world when it already takes them.
    fn active_toggle(&mut self, id: NodeId) {
        self.session.active_parent = if self.session.active_parent == id { ROOT } else { id };
    }

    fn active_button(&self, id: NodeId) -> HeaderButton<'static> {
        let on = self.session.active_parent == id;
        let tip = if on { "New objects go here; click to put them in the world again" } else { "Put new objects here" };
        HeaderButton { icon: "thumbtack", label: "Put new objects here", tip, on }
    }

    fn layer_header(&mut self, ui: &mut egui::Ui, id: NodeId, l: &Layer) {
        let mut name = l.name.clone();
        let icon = self.node_icon_of(&NodeKind::Layer(Layer::default()));
        let children = self.session.map.get(id).map_or(0, |n| n.children.len());
        let out = inspect::header(ui, icon, Title::Edit(&mut name, "Layer name"), "", &[self.active_button(id)], |ui| {
            inspect::weak(ui, &children_text(children));
        });
        if out.renamed {
            self.set_kind(id, NodeKind::Layer(Layer { name, ..l.clone() }), "Rename layer");
        }
        if out.clicked.is_some() {
            self.active_toggle(id);
        }
    }

    fn layer_body(&mut self, ui: &mut egui::Ui, id: NodeId) {
        let Some(NodeKind::Layer(mut l)) = self.session.map.get(id).map(|n| n.kind.clone()) else { return };
        if !self.section(ui, &Section { key: "layer", title: "Layer", ..Default::default() }) {
            return;
        }
        let tip = "Importers and compiles skip this layer: reference geometry such as an imported map you are zoning";
        let label = "Leave out of exports";
        let out = inspect::row(ui, &Prop { label, key: "omit_from_export", tip, ..Default::default() }, |ui| {
            inspect::toggle(ui, &mut l.omit_from_export, false, label)
        });
        if out.changed {
            self.set_kind(id, NodeKind::Layer(l), "Leave layer out of exports");
        }
    }

    fn group_header(&mut self, ui: &mut egui::Ui, id: NodeId, g: &Group) {
        let map = &self.session.map;
        let copy = g.link.as_ref().and_then(|l| l.source);
        let path_name = |p: &NodeId| map.get(*p).map_or("a deleted path".to_string(), |n| format!("{} {}", n.kind.label(), p.0));
        let subtitle = if let Some(made) = &g.generator {
            let spec =
                self.project.plugins.iter().find(|p| p.manifest.id == made.plugin).and_then(|p| p.manifest.commands.iter().find(|c| c.id == made.command));
            format!("{} generator", spec.map_or(made.command.as_str(), |c| c.label.as_str()))
        } else if let Some(src) = copy {
            format!("Linked copy of {}", map.get(src).map_or_else(|| "a missing group".to_string(), |n| n.kind.label()))
        } else if g.link.is_some() {
            let n = map.nodes().filter(|(n, _)| mapcraft_map::link::copy_of(map, *n).is_some_and(|(s, _)| s == id)).count();
            format!("Original of {n} linked copies")
        } else if let Some(op) = g.curve.as_deref() {
            match op {
                CurveOp::Sweep { path, .. } => format!("Sweep along {}", path_name(path)),
                CurveOp::Spin { .. } => "Spin".to_string(),
                CurveOp::Array { path, .. } => format!("Array along {}", path_name(path)),
            }
        } else if g.mirror.is_some() {
            "Mirror group".to_string()
        } else {
            children_text(map.get(id).map_or(0, |n| n.children.len()))
        };
        let icon = self.node_icon_of(&NodeKind::Group(Group { generator: g.generator.clone(), mirror: g.mirror, curve: g.curve.clone(), ..Group::default() }));
        let buttons = if g.curve.is_none() && copy.is_none() { vec![self.active_button(id)] } else { Vec::new() };
        let mut name = g.name.clone();
        let out = inspect::header(ui, icon, Title::Edit(&mut name, "Group name"), "", &buttons, |ui| {
            inspect::weak(ui, &subtitle);
        });
        if out.renamed {
            self.set_kind(id, NodeKind::Group(Group { name, ..g.clone() }), "Rename group");
        }
        if out.clicked.is_some() {
            self.active_toggle(id);
        }
    }

    fn group_body(&mut self, ui: &mut egui::Ui, id: NodeId) {
        let Some(NodeKind::Group(g)) = self.session.map.get(id).map(|n| n.kind.clone()) else { return };
        if let Some(generator) = g.generator.clone() {
            self.generator_section(ui, id, generator);
        }
        if let Some(m) = g.mirror {
            self.mirror_section(ui, id, &g, m);
        }
        if let Some(op) = g.curve.as_deref().cloned() {
            self.curve_section(ui, id, op);
        }
        let link = g.link.as_deref().copied();
        if let Some(link) = link {
            self.link_section(ui, id, link);
        }
        if link.is_none_or(|l| l.source.is_none()) {
            let others = g.generator.is_some() || g.mirror.is_some() || g.curve.is_some() || link.is_some();
            self.brushes_section(ui, others);
        }
    }

    fn link_section(&mut self, ui: &mut egui::Ui, id: NodeId, link: mapcraft_map::Link) {
        let map = &self.session.map;
        let copies: Vec<NodeId> = map.nodes().filter(|(n, _)| mapcraft_map::link::copy_of(map, *n).is_some_and(|(s, _)| s == id)).map(|(n, _)| n).collect();
        let original = link.source.map(|s| (s, map.get(s).map_or_else(|| "a missing group".to_string(), |n| n.kind.label())));
        let (title, tip) = match &original {
            Some((_, name)) => ("Linked copy", format!("Shows {name}; edit the original and every copy follows.")),
            None => ("Linked original", "Edits here show in every copy; moving this group leaves the copies where they are.".to_string()),
        };
        if !self.section(ui, &Section { key: "link", title, tip: &tip, icon: Some(("link", TEXT_WEAK)), ..Default::default() }) {
            return;
        }
        let mut select = None;
        match original {
            Some((source, name)) => {
                inspect::row(ui, &Prop { label: "Original", key: "original", ..Default::default() }, |ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if icon_button(ui, "cursor", "Select original", 22.0, false).on_hover_text("Select original").clicked() {
                            select = Some(vec![source]);
                        }
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| inspect::weak(ui, &name)).inner
                    })
                    .inner
                });
                let edit = format!("Edit the original in place  ({})", self.chord("tools.edit_link_here"));
                let real = format!("Turn this copy into ordinary objects  ({})", self.chord("tools.make_link_real"));
                let items = [
                    Action { icon: "pencil", label: "Edit here", tip: &edit, enabled: true },
                    Action { icon: "Unlinked", label: "Make real", tip: &real, enabled: true },
                ];
                match inspect::actions(ui, &items) {
                    Some(0) => self.run_action("tools.edit_link_here"),
                    Some(1) => self.run_action("tools.make_link_real"),
                    _ => {}
                }
            }
            None => {
                let count = copies.len().to_string();
                inspect::row(ui, &Prop { label: "Copies", key: "copies", ..Default::default() }, |ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let b = ui.add_enabled_ui(!copies.is_empty(), |ui| icon_button(ui, "cursor", "Select copies", 22.0, false)).inner;
                        if b.on_hover_text("Select copies").clicked() {
                            select = Some(copies.clone());
                        }
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| inspect::weak(ui, &count)).inner
                    })
                    .inner
                });
            }
        }
        if let Some(ids) = select {
            self.session.selection = ids.into_iter().collect();
            self.mark_changed();
        }
    }

    fn mirror_section(&mut self, ui: &mut egui::Ui, id: NodeId, g: &Group, m: Mirror) {
        let icon = if m.axis == Axis::Z { "mirror_vertically" } else { "mirror_horizontally" };
        let section = Section {
            key: "mirror",
            title: "Mirror",
            tip: "The children are copied across the plane and kept in step.",
            icon: Some((icon, MIRROR)),
            ..Default::default()
        };
        if !self.section(ui, &section) {
            return;
        }
        let mut edited = m;
        let mut turned = false;
        inspect::row(ui, &Prop { label: "Axis", key: "axis", ..Default::default() }, |ui| {
            let axes = [(Axis::X, "X", "Mirror left and right"), (Axis::Y, "Y", "Mirror front and back"), (Axis::Z, "Z", "Mirror up and down")];
            let mut r = ui.scope(|ui| turned = Self::segmented(ui, &mut edited.axis, &axes)).response;
            if turned {
                r.mark_changed();
            }
            r
        });
        if turned {
            let children = self.session.map.get(id).map(|n| n.children.clone()).unwrap_or_default();
            edited.offset = self.session.mirror_offset(&children, edited.axis).unwrap_or(edited.offset);
        }
        inspect::row(ui, &Prop { label: "Plane at", key: "offset", ..Default::default() }, |ui| {
            inspect::num(ui, &mut edited.offset, -inspect::LIMIT..=inspect::LIMIT, 1.0, "", None)
        });
        let label = "Carry textures";
        inspect::row(ui, &Prop { label, key: "texture_lock", tip: "Copies carry their textures across the plane", ..Default::default() }, |ui| {
            inspect::toggle(ui, &mut edited.texture_lock, false, label)
        });
        if edited != m {
            self.set_kind(id, NodeKind::Group(Group { mirror: Some(edited), ..g.clone() }), "Edit mirror");
        }
        let tip = format!("Turn the mirrored half into ordinary objects and stop mirroring  ({})", self.chord("tools.mirror_make_real"));
        if inspect::actions(ui, &[Action { icon: "Unlinked", label: "Make real", tip: &tip, enabled: true }]).is_some() {
            self.run_action("tools.mirror_make_real");
        }
    }

    fn prefab_header(&mut self, ui: &mut egui::Ui, p: &Prefab) {
        let stem = std::path::Path::new(&p.path).file_stem().map_or_else(|| p.path.clone(), |s| s.to_string_lossy().into_owned());
        let icon = self.node_icon_of(&NodeKind::Prefab(Prefab::default()));
        let local = format!("Make local: turn this instance into ordinary objects  ({})", self.chord("prefab.make_local"));
        let buttons = [
            HeaderButton { icon: "folder_open", label: "Open prefab", tip: "Open prefab: edit the prefab's own map", on: false },
            HeaderButton { icon: "Unlinked", label: "Make local", tip: &local, on: false },
        ];
        let out = inspect::header(ui, icon, Title::Fixed(&stem), "", &buttons, |ui| {
            inspect::weak(ui, &p.path).on_hover_text(p.path.as_str());
        });
        match out.clicked {
            Some(0) => {
                if let Ok(path) = craft_docpath::resolve_absolute(&p.path, self.session.path.as_deref()) {
                    self.open_dialog_path(path);
                }
            }
            Some(1) => self.run_action("prefab.make_local"),
            _ => {}
        }
    }

    fn prefab_body(&mut self, ui: &mut egui::Ui, id: NodeId) {
        let Some(NodeKind::Prefab(p)) = self.session.map.get(id).map(|n| n.kind.clone()) else { return };
        let mut edited = p.clone();
        if self.section(ui, &Section { key: "prefab.transform", title: "Transform", ..Default::default() }) {
            inspect::row(ui, &Prop { label: "Origin", key: "origin", wide: true, ..Default::default() }, |ui| {
                inspect::vec(ui, edited.origin.as_mut(), inspect::XYZ, -inspect::LIMIT..=inspect::LIMIT, 1.0, "", [false; 3]).0
            });
            inspect::row(ui, &Prop { label: "Angles", key: "angles", wide: true, ..Default::default() }, |ui| {
                inspect::vec(ui, edited.angles.as_mut(), inspect::PYR, -inspect::ANGLE_LIMIT..=inspect::ANGLE_LIMIT, 1.0, "°", [false; 3]).0
            });
        }
        let doc = self.session.path.clone();
        let Some(src) = crate::app::prefab_loader(&mut self.prefab_cache, doc)(&p.path) else {
            inspect::note(ui, Note::Error, "The prefab file cannot be loaded.");
            self.commit_prefab(id, &p, edited);
            return;
        };
        let section = Section {
            key: "prefab.overrides",
            title: "Overrides",
            badge: p.overrides.len().to_string(),
            tip: "Changes here apply to this instance only.",
            ..Default::default()
        };
        if self.section(ui, &section) {
            let filtering = inspect::filtering(ui);
            for (eid, n) in src.nodes() {
                let Some(e) = n.kind.entity().filter(|_| matches!(n.kind, NodeKind::Entity(_))) else { continue };
                let entity_label = n.kind.label();
                inspect::subheading(ui, Some(self.node_icon_of(&n.kind)), &entity_label);
                let class = self.project.game.class(&e.classname);
                // Overrides apply after the instance is placed, so a moved origin would land in world space.
                for (k, original) in e.properties.iter().filter(|(k, _)| k != "origin") {
                    let pos = edited.overrides.iter().position(|o| o.node == eid && o.key == *k);
                    let current = pos.and_then(|i| edited.overrides.get(i)).and_then(|o| o.value.clone()).unwrap_or_else(|| original.clone());
                    let label = if filtering { format!("{entity_label}: {k}") } else { k.clone() };
                    let prop = Prop {
                        label: &label,
                        key: k,
                        val: if pos.is_some() { Val::Set } else { Val::Default },
                        slot: if pos.is_some() { Slot::Revert } else { Slot::None },
                        ..Default::default()
                    };
                    let fgd = class.as_ref().and_then(|c| c.property(k));
                    let values = [current.clone()];
                    let vid = egui::Id::new(("override", id.0, eid.0, k));
                    let mut change = Change::None;
                    let out = inspect::row(ui, &prop, |ui| match fgd {
                        Some(fp) => {
                            let (r, c) = inspect::fgd_value(ui, vid, fp, &label, &values, &[]);
                            change = c;
                            r
                        }
                        None => {
                            let mut v = current.clone();
                            let r = inspect::text(ui, &mut v, "", false);
                            if r.changed() {
                                change = Change::Set(v);
                            }
                            r
                        }
                    });
                    if out.slot {
                        if let Some(i) = pos.filter(|i| *i < edited.overrides.len()) {
                            edited.overrides.remove(i);
                        }
                    } else if let Some(v) = change.apply(&current) {
                        let o = PropertyOverride { node: eid, key: k.clone(), value: Some(v) };
                        match pos.and_then(|i| edited.overrides.get_mut(i)) {
                            Some(slot) => *slot = o,
                            None => edited.overrides.push(o),
                        }
                    }
                }
            }
        }
        self.commit_prefab(id, &p, edited);
    }

    fn commit_prefab(&mut self, id: NodeId, old: &Prefab, new: Prefab) {
        if *old != new {
            self.set_kind(id, NodeKind::Prefab(new), "Edit prefab");
        }
    }

    fn picked_faces(&self) -> Vec<(NodeId, Face)> {
        let map = &self.session.map;
        self.session.face_selection.iter().filter_map(|(id, f)| Some((*id, map.get(*id)?.kind.brush()?.faces.iter().find(|x| x.id == *f)?.clone()))).collect()
    }

    fn faces_header(&mut self, ui: &mut egui::Ui) {
        let faces = self.picked_faces();
        let brushes: BTreeSet<NodeId> = faces.iter().map(|(id, _)| *id).collect();
        let title = match faces.len() {
            0 => "No faces".to_string(),
            1 => "1 face".to_string(),
            n => format!("{n} faces"),
        };
        let subtitle = match brushes.len() {
            0 => "Face tool".to_string(),
            1 => "on 1 brush".to_string(),
            n => format!("on {n} brushes"),
        };
        inspect::header(ui, ("face", TEXT_WEAK), Title::Fixed(&title), "", &[], |ui| {
            inspect::weak(ui, &subtitle);
        });
    }

    fn faces_body(&mut self, ui: &mut egui::Ui) {
        let faces = self.picked_faces();
        let Some((_, first)) = faces.first().cloned() else {
            ui.add_space(4.0);
            inspect::note(ui, Note::Info, "Click faces in the 3D view; Ctrl+click adds more.");
            inspect::note(ui, Note::Info, "Alt+click picks a face's material, Alt+right click applies the current one.");
            return;
        };

        self.uv_section(ui);
        if self.section(ui, &Section { key: "face.material", title: "Surface", ..Default::default() }) {
            let materials: BTreeSet<&str> = faces.iter().map(|(_, f)| f.material.as_str()).collect();
            let mixed = materials.len() != 1;
            let mut name = if mixed { format!("{} materials", materials.len()) } else { first.material.clone() };
            let ctx = ui.ctx().clone();
            let thumb = if mixed { None } else { inspect::material_thumb(&self.project, &mut self.thumbs, &ctx, &name) };
            let (mut browse, mut bucket) = (false, false);
            inspect::row(ui, &Prop { label: "Material", key: "material", val: if mixed { Val::Mixed } else { Val::Set }, ..Default::default() }, |ui| {
                let (r, b) = inspect::material(ui, thumb, &mut name, "Material", "", mixed, true);
                (browse, bucket) = (r.clicked(), b);
                r
            });
            if browse {
                self.run_action("panel.materials");
            }
            if bucket {
                let m = self.session.tex.material.clone();
                self.face_edit("Apply material", None, move |f| f.material = m.clone());
            }
            let tip = "Material, alignment and attributes of the face last copied (Ctrl+C) or Alt+clicked, wrapped across shared edges (Ctrl+V)";
            if inspect::actions(ui, &[Action { icon: "clipboard", label: "Paste face", tip, enabled: self.picked_face.is_some() }]).is_some()
                && let Some(src) = self.picked_face.clone()
            {
                let r = self.session.paste_face(&src);
                self.after_edit(r);
            }
        }

        if self.section(ui, &Section { key: "face.align", title: "Alignment", ..Default::default() }) {
            let differs = |f: &dyn Fn(&Face) -> f64| faces.iter().any(|(_, x)| f(x) != f(&first));
            let shown_offset = [first.u.offset, first.v.offset];
            let shown_scale = [first.u.scale, first.v.scale];
            let (mut offset, mut scale) = (shown_offset, shown_scale);
            let offset_mixed = [differs(&|f| f.u.offset), differs(&|f| f.v.offset)];
            let scale_mixed = [differs(&|f| f.u.scale), differs(&|f| f.v.scale)];
            let rot_mixed = differs(&|f| f.rotation);
            let (mut moved, mut moved_drag) = (None, false);
            inspect::row(ui, &Prop { label: "Offset", key: "offset", val: mixed_val(offset_mixed.contains(&true)), ..Default::default() }, |ui| {
                let (r, axis) = inspect::vec(ui, &mut offset, inspect::UV, -inspect::LIMIT..=inspect::LIMIT, 1.0, "", offset_mixed);
                (moved, moved_drag) = (axis, r.dragged());
                r
            });
            let (mut scaled, mut scaled_drag) = (None, false);
            inspect::row(ui, &Prop { label: "Texture scale", key: "scale", val: mixed_val(scale_mixed.contains(&true)), ..Default::default() }, |ui| {
                let (r, axis) = inspect::vec(ui, &mut scale, inspect::UV, -inspect::LIMIT..=inspect::LIMIT, 0.01, "", scale_mixed);
                (scaled, scaled_drag) = (axis, r.dragged());
                r
            });
            let mut rot = first.rotation;
            let rotated = inspect::row(ui, &Prop { label: "Rotation", key: "rotation", val: mixed_val(rot_mixed), ..Default::default() }, |ui| {
                inspect::num(ui, &mut rot, -inspect::ANGLE_LIMIT..=inspect::ANGLE_LIMIT, 1.0, "°", rot_mixed.then_some("mixed"))
            })
            .changed;
            let mut justify = None;
            inspect::row(ui, &Prop { label: "Justify", key: "justify", ..Default::default() }, |ui| {
                let items = [
                    ("expand", "Fit", "Fit the texture to the face"),
                    ("arrow_left_to_line", "Justify left", "Justify left"),
                    ("arrow_right_to_line", "Justify right", "Justify right"),
                    ("arrow_up_to_line", "Justify top", "Justify top"),
                    ("arrow_down_to_line", "Justify bottom", "Justify bottom"),
                    ("reticle", "Center", "Center the texture on the face"),
                ];
                ui.scope(|ui| justify = inspect::icons(ui, &items)).response
            });
            let mut align = None;
            inspect::row(ui, &Prop { label: "Align", key: "align", ..Default::default() }, |ui| {
                ui.scope(|ui| align = inspect::icons(ui, &[("globe", "Align to world", "Align to world"), ("face", "Align to face", "Align to face")])).response
            });
            let lightmaps: Vec<Option<&String>> = faces.iter().map(|(_, f)| f.attrs.get("lightmapscale")).collect();
            let any = lightmaps.iter().any(Option::is_some);
            let lm_mixed = lightmaps.iter().any(|v| *v != lightmaps.first().copied().flatten());
            let mut lightmap = lightmaps.iter().flatten().next().and_then(|s| s.trim().parse::<i64>().ok()).unwrap_or(1);
            let shown = if lm_mixed {
                Some("mixed")
            } else if !any {
                Some("default")
            } else {
                None
            };
            let lm_prop = Prop {
                label: "Lightmap scale",
                key: "lightmapscale",
                tip: "Lightmap texels per world unit on this face; the game's default applies when unset",
                val: if lm_mixed {
                    Val::Mixed
                } else if any {
                    Val::Set
                } else {
                    Val::Default
                },
                slot: if any { Slot::Revert } else { Slot::None },
                ..Default::default()
            };
            let lm = inspect::row(ui, &lm_prop, |ui| inspect::num(ui, &mut lightmap, 1..=65536, 1.0, "", shown));
            let anti_tile = anti_tile_row(ui, "face-anti-tile", faces.iter().map(|(_, f)| f), faces.iter().any(|(_, f)| self.project.pixel_face(f)));

            for (axis, dragged, new, old, key, is_scale) in [
                (moved, moved_drag, offset, shown_offset, ["face-offset-u", "face-offset-v"], false),
                (scaled, scaled_drag, scale, shown_scale, ["face-scale-u", "face-scale-v"], true),
            ] {
                let Some((i, (new, old), key)) = axis.and_then(|i| Some((i, (*new.get(i)?, *old.get(i)?), *key.get(i)?))) else { continue };
                let delta = new - old;
                self.face_edit("Edit texture", Some(key), move |f| {
                    let a = if i == 0 { &mut f.u } else { &mut f.v };
                    let (current, slot) = if is_scale { (a.scale, &mut a.scale) } else { (a.offset, &mut a.offset) };
                    let v = if dragged { current + delta } else { new };
                    // A delta on mixed faces can land near zero, which would collapse the texture.
                    if v.is_finite() && (!is_scale || v.abs() >= 1e-3) {
                        *slot = v;
                    }
                });
            }
            if rotated {
                let delta = rot - first.rotation;
                self.face_edit("Rotate texture", Some("face-rot"), move |f| f.rotate_texture(delta));
            }
            let modes = [Justify::Fit, Justify::Left, Justify::Right, Justify::Top, Justify::Bottom, Justify::Center];
            if let Some(mode) = justify.and_then(|i| modes.get(i).copied()) {
                self.justify_faces(mode);
            }
            match align {
                Some(0) => self.face_edit("Align to world", None, |f| f.align_to_world()),
                Some(1) => self.face_edit("Align to face", None, |f| f.align_to_face()),
                _ => {}
            }
            if lm.slot {
                self.face_edit("Lightmap scale", None, |f| {
                    f.attrs.remove("lightmapscale");
                });
            } else if lm.changed {
                let v = lightmap.to_string();
                self.face_edit("Lightmap scale", Some("face-lm"), move |f| {
                    f.attrs.insert("lightmapscale".into(), v.clone());
                });
            }
            if let Some(mode) = anti_tile {
                self.face_edit("Anti-tiling", None, move |f| f.set_anti_tile(mode));
            }
        }

        self.displacement_section(ui, faces.iter().any(|(_, f)| f.disp.is_some()));
    }

    /// The picked faces' UV editor, at the top of the face panel.
    fn uv_section(&mut self, ui: &mut egui::Ui) {
        let badge = self.uv_reference().map(|(_, s)| format!("{}x{}", s.x, s.y)).unwrap_or_default();
        let tip = format!("Fit the picked faces in the view ({} over it, or double click)", self.chord("uv.fit"));
        let fit = HeaderButton { icon: crate::icons::action_icon("uv.fit").unwrap_or("help"), label: "Fit UV View", tip: &tip, on: false };
        let section = Section { key: "face.uv", title: "UV", badge, icon: Some(("unwrap", TEXT_WEAK)), action: Some(fit), ..Default::default() };
        let (open, fit) = self.section_with(ui, &section);
        if fit {
            // uv.fit fits only the view under the pointer, and the bar belongs to this one.
            self.uv_hovered = Some(crate::uv::UvView::Inspector);
            self.run_action("uv.fit");
        }
        if open && !inspect::filtering(ui) {
            let w = ui.available_width();
            let (_, rect) = ui.allocate_space(egui::vec2(w, w.clamp(160.0, 360.0)));
            self.uv_view(ui, crate::uv::UvView::Inspector, rect);
        }
    }

    /// Making, removing and heightmapping displacements on the picked faces.
    fn displacement_section(&mut self, ui: &mut egui::Ui, any: bool) {
        if !self.section(ui, &Section { key: "face.disp", title: "Displacement", icon: Some(("mountains", TEXT_WEAK)), closed: !any, ..Default::default() }) {
            return;
        }
        inspect::row(ui, &Prop { label: "Power", key: "power", tip: "Rows and columns of the grid a displacement gets", ..Default::default() }, |ui| {
            let before = self.disp_power;
            let w = ui.available_width();
            let mut r = egui::ComboBox::from_id_salt("disp-power")
                .selected_text(format!("Power {}", self.disp_power))
                .width(w)
                .show_ui(ui, |ui| {
                    for p in 2..=mapcraft_map::disp::MAX_POWER {
                        let n = (1 << p) + 1;
                        ui.selectable_value(&mut self.disp_power, p, format!("Power {p} ({n} by {n})"));
                    }
                })
                .response;
            if self.disp_power != before {
                r.mark_changed();
            }
            r
        });
        let items = [
            Action {
                icon: "grid_fine",
                label: if any { "Set power" } else { "Make" },
                tip: "Turns four-cornered faces into a grid you can sculpt",
                enabled: true,
            },
            Action { icon: "trash_can", label: "Remove displacement", tip: "Turns the picked displacements back into flat faces", enabled: any },
        ];
        match inspect::actions(ui, &items) {
            Some(0) => {
                let r = self.session.displace_faces(self.disp_power).map(|_| ());
                self.after_edit(r);
            }
            Some(1) => {
                let r = self.session.remove_displacements();
                self.after_edit(r);
            }
            _ => {}
        }
        let readonly = (!any).then_some("Make a displacement first.");
        let tip = "The height a heightmap's black maps to";
        inspect::row(ui, &Prop { label: "Black height", key: "black", tip, readonly, ..Default::default() }, |ui| {
            inspect::num(ui, &mut self.heightmap_range.0, -inspect::LIMIT..=inspect::LIMIT, 4.0, "", None)
        });
        let tip = "The height a heightmap's white maps to";
        inspect::row(ui, &Prop { label: "White height", key: "white", tip, readonly, ..Default::default() }, |ui| {
            inspect::num(ui, &mut self.heightmap_range.1, -inspect::LIMIT..=inspect::LIMIT, 4.0, "", None)
        });
        let load =
            [Action { icon: "folder_open", label: "Load heightmap…", tip: "A grayscale image stretched over all the picked displacements", enabled: any }];
        if inspect::actions(ui, &load).is_some() {
            self.load_heightmap();
        }
        inspect::note(ui, Note::Info, &format!("Sculpt with the Sculpt tool ({}).", self.chord("tool.sculpt")));
        self.blend_rows(ui, any);
    }

    /// The second material of the picked displacements and the ways to fill its shares.
    fn blend_rows(&mut self, ui: &mut egui::Ui, any: bool) {
        let displaced: Vec<(NodeId, Face)> = self.picked_faces().into_iter().filter(|(_, f)| f.disp.is_some()).collect();
        let layers: BTreeSet<Option<&str>> = displaced.iter().map(|(_, f)| f.blend_layer()).collect();
        let (mixed, has) = (layers.len() > 1, layers.iter().any(Option::is_some));
        let mut name = match layers.first() {
            _ if mixed => "mixed".to_string(),
            Some(Some(m)) => m.to_string(),
            _ => "none".to_string(),
        };
        let ctx = ui.ctx().clone();
        let thumb = if mixed || !has { None } else { inspect::material_thumb(&self.project, &mut self.thumbs, &ctx, &name) };
        let readonly = (!any).then_some("Make a displacement first.");
        let prop = Prop {
            label: "Blend material",
            key: "blend_material",
            tip: "The second material painted into these displacements",
            val: if mixed {
                Val::Mixed
            } else if has {
                Val::Set
            } else {
                Val::Default
            },
            slot: if has { Slot::Remove } else { Slot::None },
            readonly,
            ..Default::default()
        };
        let (mut browse, mut bucket) = (false, false);
        let out = inspect::row(ui, &prop, |ui| {
            let (r, b) = inspect::material(ui, thumb, &mut name, "Blend material", "", mixed || !has, true);
            (browse, bucket) = (r.clicked(), b);
            r
        });
        let kinds = [("value", "Amount"), ("slope", "Slope"), ("height", "Height"), ("noise", "Noise")];
        inspect::row(
            ui,
            &Prop { label: "Blend fill", key: "blend_fill", tip: "What Fill blend sets each point's share by", readonly, ..Default::default() },
            |ui| inspect::choice(ui, egui::Id::new("blend-fill"), &mut self.blend_fill.kind, &kinds, false),
        );
        if self.blend_fill.kind == "height" && self.blend_fill.height.is_none() {
            self.blend_fill.height = Some(self.displaced_z_range(&displaced));
        }
        let fill = &mut self.blend_fill;
        let pair = |ui: &mut egui::Ui, v: &mut [f64; 2], keys: [&str; 2], tips: [&str; 2], range: std::ops::RangeInclusive<f64>, speed: f64, suffix: &str| {
            for ((label, key), (tip, x)) in ["From", "To"].into_iter().zip(keys).zip(tips.into_iter().zip(v.iter_mut())) {
                inspect::row(ui, &Prop { label, key, tip, readonly, ..Default::default() }, |ui| inspect::num(ui, x, range.clone(), speed, suffix, None));
            }
        };
        match fill.kind.as_str() {
            "slope" => {
                let tips = ["Steepness where the second material starts to show", "Steepness where only the second material shows"];
                pair(ui, &mut fill.slope, ["blend_slope_from", "blend_slope_to"], tips, 0.0..=90.0, 0.5, "°");
            }
            "height" => {
                let tips = ["Height where the second material starts to show", "Height where only the second material shows"];
                let h = fill.height.get_or_insert([0.0, 0.0]);
                pair(ui, h, ["blend_height_from", "blend_height_to"], tips, -inspect::LIMIT..=inspect::LIMIT, 4.0, "");
            }
            "noise" => {
                inspect::row(
                    ui,
                    &Prop { label: "Size", key: "blend_noise_size", tip: "Width of the noise's patches in units", readonly, ..Default::default() },
                    |ui| inspect::num(ui, &mut fill.noise_size, 1.0..=inspect::LIMIT, 4.0, "", None),
                );
                inspect::row(ui, &Prop { label: "Seed", key: "blend_seed", tip: "Another seed gives other patches", readonly, ..Default::default() }, |ui| {
                    inspect::num(ui, &mut fill.seed, 0..=u32::MAX, 1.0, "", None)
                });
                let tips = ["Noise value where the second material starts to show", "Noise value where only the second material shows"];
                pair(ui, &mut fill.noise, ["blend_noise_from", "blend_noise_to"], tips, 0.0..=1.0, 0.01, "");
            }
            _ => {
                inspect::row(
                    ui,
                    &Prop { label: "Amount", key: "blend_amount", tip: "The share Fill blend gives every point", readonly, ..Default::default() },
                    |ui| inspect::num(ui, &mut fill.amount, 0.0..=100.0, 1.0, "%", None),
                );
            }
        }
        let items = [
            Action { icon: "bucket_fill", label: "Fill blend", tip: "Sets the picked displacements' shares by the Blend fill row", enabled: any },
            Action { icon: "arrow_right_arrow_left", label: "Invert blend", tip: "Swaps the two materials' shares", enabled: any },
            Action { icon: "cross_in_square", label: "Clear blend", tip: "Shows only each face's own material again", enabled: any },
            Action {
                icon: "folder_open",
                label: "Load blend mask…",
                tip: "A grayscale image stretched over all the picked displacements; white is the second material",
                enabled: any,
            },
        ];
        let action = inspect::actions(ui, &items);
        inspect::note(ui, Note::Info, &format!("Paint it with the Sculpt tool's Paint mode ({}).", self.chord("sculpt.paint")));

        if browse {
            self.run_action("panel.materials");
        }
        if bucket {
            self.run_command("disp.blend_use_current");
        }
        if out.slot {
            self.run_command("disp.blend_remove");
        }
        let fill = match action {
            Some(0) => Some(self.blend_fill.fill()),
            Some(1) => Some(BlendFill::Invert),
            Some(2) => Some(BlendFill::Value(0.0)),
            Some(3) => {
                self.load_blend_mask();
                None
            }
            _ => None,
        };
        if let Some(fill) = fill {
            let r = self.session.fill_blend(fill).map(|_| ());
            self.after_edit(r);
        }
    }

    /// The lowest and highest Z of these displaced faces' points.
    fn displaced_z_range(&self, faces: &[(NodeId, Face)]) -> [f64; 2] {
        let mut range = [f64::INFINITY, f64::NEG_INFINITY];
        for (id, face) in faces {
            let Some(b) = self.session.map.get(*id).and_then(|n| n.kind.brush()) else { continue };
            let (Ok(solid), Some(i)) = (b.solid(), b.faces.iter().position(|f| f.id == face.id)) else { continue };
            for p in b.displaced(&solid).into_iter().filter(|(k, _)| *k == i).flat_map(|(_, pts)| pts) {
                range = [range[0].min(p.z), range[1].max(p.z)];
            }
        }
        if range[0] <= range[1] { range } else { [0.0, 512.0] }
    }

    fn after_edit(&mut self, result: Result<(), mapcraft_engine::EngineError>) {
        if let Err(e) = result {
            self.status = e.to_string();
        }
        self.mark_changed();
    }

    /// Asks for a grayscale image; `None` when cancelled or unreadable.
    fn pick_gray_image(&mut self) -> Option<image::ImageBuffer<image::Luma<f32>, Vec<f32>>> {
        let path = rfd::FileDialog::new().add_filter("Image", &["png", "jpg", "jpeg", "tga", "bmp", "tif", "tiff", "exr"]).pick_file()?;
        match image::open(&path) {
            Ok(i) => Some(i.to_luma32f()),
            Err(e) => {
                self.status = format!("cannot read {}: {e}", path.display());
                None
            }
        }
    }

    fn load_heightmap(&mut self) {
        let Some(img) = self.pick_gray_image() else { return };
        let (w, h) = (img.width() as usize, img.height() as usize);
        let (low, high) = self.heightmap_range;
        let r = self.session.apply_heightmap(img.as_raw(), w, h, low, high);
        self.after_edit(r);
    }

    fn load_blend_mask(&mut self) {
        let Some(img) = self.pick_gray_image() else { return };
        let r = self.session.apply_blend_mask(img.as_raw(), img.width() as usize, img.height() as usize);
        self.after_edit(r);
    }

    fn face_edit(&mut self, label: &str, coalesce: Option<&str>, f: impl Fn(&mut Face)) {
        if let Err(e) = self.session.edit_faces(label, coalesce, f) {
            self.status = e.to_string();
        }
        self.mark_changed();
    }

    fn justify_faces(&mut self, mode: Justify) {
        let mut polys = BTreeMap::new();
        for (id, face) in &self.session.face_selection {
            let Some(b) = self.session.map.get(*id).and_then(|n| n.kind.brush()) else { continue };
            let (Ok(solid), Some(i)) = (b.solid(), b.faces.iter().position(|f| f.id == *face)) else { continue };
            let Some(Some(w)) = solid.faces.get(i) else { continue };
            let size = b.faces.get(i).map_or(DVec2::splat(64.0), |f| crate::uv::texture_size(self.gpu.as_ref().map(|g| &g.renderer), &f.material));
            polys.insert(*face, (w.0.clone(), size));
        }
        self.face_edit("Justify texture", None, move |f| {
            if let Some((poly, size)) = polys.get(&f.id) {
                f.justify(poly, *size, mode);
            }
        });
    }

    fn set_kind(&mut self, id: NodeId, kind: NodeKind, label: &str) {
        if let Err(e) = self.session.set_kind(id, kind, label, Some(&format!("edit:{}", id.0))) {
            self.status = e.to_string();
        }
        self.mark_changed();
    }

    /// Outputs of other entities that target this one by name.
    fn inputs_to(&self, id: NodeId) -> Vec<(NodeId, Connection)> {
        let Some(name) = self.session.map.get(id).and_then(|n| n.kind.entity()).and_then(|e| e.get("targetname")).filter(|n| !n.is_empty()) else {
            return Vec::new();
        };
        self.session
            .map
            .nodes()
            .filter_map(|(src, n)| Some((src, n.kind.entity()?)))
            .flat_map(|(src, e)| e.connections.iter().filter(|c| c.target == name).map(move |c| (src, c.clone())))
            .collect()
    }

    fn edit_brushes(&mut self, label: &str, coalesce: Option<&str>, f: impl Fn(&mut Brush)) {
        let ids = self.session.selected_brushes();
        let mut e = self.session.map.edit();
        for id in ids {
            let Some(mut b) = e.node(id).and_then(|n| n.kind.brush()).cloned() else { continue };
            f(&mut b);
            if let Err(err) = e.set_kind(id, NodeKind::Brush(b)) {
                self.status = err.to_string();
                return;
            }
        }
        let c = e.finish();
        self.session.apply_coalesced(label, c, coalesce);
        self.mark_changed();
    }
}

fn mixed_val(mixed: bool) -> Val {
    if mixed { Val::Mixed } else { Val::Set }
}

fn children_text(n: usize) -> String {
    if n == 1 { "1 child".to_string() } else { format!("{n} children") }
}

/// The classname of the entity that owns brush `id`, or `None` for a world brush.
fn owner_class(map: &mapcraft_map::Map, id: NodeId) -> Option<String> {
    let parent = map.get(map.get(id)?.parent?)?;
    match &parent.kind {
        NodeKind::Entity(e) => Some(e.classname.clone()),
        _ => None,
    }
}

/// The six rows of one output; returns whether it changed and whether its remove slot was clicked.
fn connection_rows(
    ui: &mut egui::Ui,
    c: &mut Connection,
    number: &str,
    outputs: &[(String, String)],
    targets: &[(String, String)],
    inputs: &[(String, String)],
) -> (bool, bool) {
    let names: BTreeSet<&str> = targets.iter().map(|(n, _)| n.as_str()).collect();
    let label = |s: &str| format!("{s}{number}");
    let mut changed = false;
    let l = label("Output name");
    let id = ui.id();
    let mut gone = false;
    changed |= inspect::row(ui, &Prop { label: &l, key: "output", tip: "The event of this entity that fires", ..Default::default() }, |ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let remove = format!("Remove {l}");
            gone = icon_button(ui, "trash_can", &remove, 18.0, false).on_hover_text(remove.as_str()).clicked();
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| inspect::suggest(ui, id.with("output"), &mut c.output, "output", outputs, false)).inner
        })
        .inner
    })
    .changed;
    let l = label("Target");
    let warn = mapcraft_engine::is_unresolved(&names, &c.target);
    changed |= inspect::row(ui, &Prop { label: &l, key: "target", tip: "The name of the entity it reaches", warn, ..Default::default() }, |ui| {
        inspect::suggest(ui, id.with("target"), &mut c.target, "target", targets, false)
    })
    .changed;
    let l = label("Input");
    changed |= inspect::row(ui, &Prop { label: &l, key: "input", tip: "What the target does", ..Default::default() }, |ui| {
        inspect::suggest(ui, id.with("input"), &mut c.input, "input", inputs, false)
    })
    .changed;
    let l = label("Parameter");
    changed |= inspect::row(ui, &Prop { label: &l, key: "parameter", ..Default::default() }, |ui| inspect::text(ui, &mut c.param, "", false)).changed;
    let l = label("Delay");
    changed |= inspect::row(ui, &Prop { label: &l, key: "delay", tip: "Seconds before the input fires", ..Default::default() }, |ui| {
        inspect::num(ui, &mut c.delay, 0.0..=3600.0, 0.05, " s", None)
    })
    .changed;
    let l = label("Once");
    let mut once = c.times_to_fire == 1;
    let fired = inspect::row(ui, &Prop { label: &l, key: "once", tip: "Fire only the first time", ..Default::default() }, |ui| {
        inspect::toggle(ui, &mut once, false, &l)
    })
    .changed;
    if fired {
        c.times_to_fire = if once { 1 } else { -1 };
        changed = true;
    }
    (changed, gone)
}

/// An eye that shows whether something is visible; clicking it flips that.
fn visibility_toggle(ui: &mut egui::Ui, shown: bool) -> egui::Response {
    icon_button(ui, if shown { "eye" } else { "eye_closed" }, if shown { "Hide" } else { "Show" }, 20.0, false)
}

/// A search box with Godot's magnifying glass inside its right end.
/// A browser's count, with the Previews button on the right switching between pictures and a list of names.
pub(crate) fn count_and_previews(ui: &mut egui::Ui, count: &str, previews: &mut bool) {
    ui.horizontal(|ui| {
        ui.label(count);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (name, tip) =
                if *previews { ("tileset", "Showing previews; click for a list") } else { ("list_unordered", "Showing a list; click for previews") };
            if icon_button(ui, name, "Previews", 22.0, false).on_hover_text(tip).clicked() {
                *previews = !*previews;
            }
        });
    });
}

/// Picture tiles in rows as wide as the panel; only the rows on screen are drawn.
pub(crate) fn tile_grid<T>(ui: &mut egui::Ui, items: &[T], mut tile: impl FnMut(&mut egui::Ui, &T)) {
    let cell = crate::thumbs::SIZE as f32 + 8.0;
    let per_row = ((ui.available_width() / cell).floor() as usize).max(1);
    let rows = items.len().div_ceil(per_row);
    egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, cell + ROW, rows, |ui, range| {
        for row in range {
            ui.horizontal(|ui| {
                for item in items.iter().skip(row * per_row).take(per_row) {
                    ui.vertical(|ui| {
                        ui.set_width(cell - 4.0);
                        tile(ui, item);
                    });
                }
            });
        }
    });
}

pub(crate) fn search_field(ui: &mut egui::Ui, text: &mut String, hint: &str) -> egui::Response {
    let edit = egui::TextEdit::singleline(text).hint_text(hint).margin(egui::Margin { left: 4, right: 22, top: 2, bottom: 2 });
    let r = ui.add(edit);
    let glass = Rect::from_center_size(r.rect.right_center() - egui::vec2(11.0, 0.0), egui::Vec2::splat(14.0));
    icons::paint(ui, "magnifying_glass", glass, 14.0, crate::chrome::TEXT_WEAK);
    r
}

/// The scene tree label of a node matching the lowercase `query`, naming the material a brush was found by.
fn search_label(kind: &NodeKind, query: &str) -> Option<String> {
    let label = kind.label();
    if find_ci(&label, query).is_some() {
        return Some(label);
    }
    let face = kind.brush()?.faces.iter().find(|f| find_ci(&f.material, query).is_some())?;
    Some(format!("{label} ({})", face.material))
}

/// Where the lowercase `query` starts in `text`, ignoring case; a byte offset into `text` when it is ASCII.
fn find_ci(text: &str, query: &str) -> Option<usize> {
    if query.is_empty() {
        return None;
    }
    if text.is_ascii() {
        return text.as_bytes().windows(query.len()).position(|w| w.eq_ignore_ascii_case(query.as_bytes()));
    }
    text.to_lowercase().find(query)
}

/// `text` in `color` with the first match of the lowercase `query` in the accent color.
fn highlight(text: &str, query: &str, font: egui::FontId, color: Color32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let format = |c| egui::TextFormat::simple(font.clone(), c);
    // Lowercasing can change byte lengths, so the offset only counts where the span really matches.
    let at = find_ci(text, query).filter(|&s| text.get(s..s + query.len()).is_some_and(|h| h.to_lowercase() == query));
    match at.and_then(|s| Some((text.get(..s)?, text.get(s..s + query.len())?, text.get(s + query.len()..)?))) {
        Some((before, hit, after)) => {
            job.append(before, 0.0, format(color));
            job.append(hit, 0.0, format(crate::chrome::ACCENT));
            job.append(after, 0.0, format(color));
        }
        None => job.append(text, 0.0, format(color)),
    }
    job
}

fn top_folder(material: &str) -> Option<&str> {
    material.split_once('/').map(|(f, _)| f)
}
