use std::collections::{BTreeMap, BTreeSet};

use egui::{Color32, Rect, RichText};
use mapcraft_engine::Severity;
use mapcraft_geom::DVec2;
use mapcraft_map::{Axis, Brush, Connection, Cordon, Face, Group, Justify, Mirror, NodeId, NodeKind, Prefab, PropertyOverride, ROOT};

use crate::app::{BottomTab, LeftTab, MapCraftApp};
use crate::viewport::Tool;

const ROW: f32 = 18.0;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum InspectorTab {
    #[default]
    Keyvalues,
    Outputs,
    Inputs,
}

#[derive(Default)]
pub(crate) struct FindState {
    query: String,
    replace: String,
    selected_only: bool,
}

impl MapCraftApp {
    pub(crate) fn left_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for (tab, label, id) in [
                (LeftTab::Scene, "Scene", "panel.scene"),
                (LeftTab::Tags, "Tags", "panel.tags"),
                (LeftTab::Materials, "Materials", "panel.materials"),
                (LeftTab::Entities, "Entities", "panel.entities"),
            ] {
                if ui.selectable_label(self.left_tab == tab, label).on_hover_text(self.chord(id)).clicked() {
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
        }
    }

    fn scene_rows(&self) -> Vec<(NodeId, usize)> {
        let mut rows = Vec::new();
        let mut stack = vec![(ROOT, 0usize)];
        while let Some((id, depth)) = stack.pop() {
            let Some(n) = self.session.map.get(id) else { continue };
            rows.push((id, depth));
            if self.expanded.contains(&id) && depth < 64 {
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
    /// parent, the index among its children, and whether they go inside `id`.
    fn drop_target(&self, id: NodeId, f: f32) -> Option<(NodeId, usize, bool)> {
        let map = &self.session.map;
        let n = map.get(id)?;
        if self.tree_drag.contains(&id) || map.ancestors(id).iter().any(|a| self.tree_drag.contains(a)) {
            return None;
        }
        let fits = |parent: &mapcraft_map::Node| {
            !matches!(&parent.kind, NodeKind::Group(g) if g.generator.is_some())
                && self.tree_drag.iter().all(|d| map.get(*d).is_some_and(|dn| parent.kind.can_contain(&dn.kind)))
        };
        if fits(n) {
            // Below an unfolded row is above its first child.
            if self.expanded.contains(&id) && !n.children.is_empty() && f >= 0.75 {
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
        ui.horizontal(|ui| {
            if ui.button("New Layer").on_hover_text(self.chord("layer.new")).clicked() {
                self.run_action("layer.new");
            }
            if ui.add_enabled(!self.session.selection.is_empty(), egui::Button::new("Group")).clicked() {
                self.run_command("tools.group");
            }
        });
        let reveal = self.session.selection != self.tree_revealed;
        if reveal {
            for id in &self.session.selection {
                self.expanded.extend(self.session.map.ancestors(*id));
            }
            self.tree_revealed = self.session.selection.clone();
        }
        let rows = self.scene_rows();
        let mut action: Option<(NodeId, &'static str)> = None;
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
        let out = scroll.show_rows(ui, ROW, rows.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for &(id, depth) in rows.get(range).unwrap_or_default() {
                let Some(n) = self.session.map.get(id) else { continue };
                let label = n.kind.label();
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
                if !n.children.is_empty() {
                    let r = ui.interact(chevron, ui.id().with(("tree-chevron", id.0)), egui::Sense::click());
                    let c = if r.hovered() { crate::chrome::TEXT } else { crate::chrome::TEXT_WEAK };
                    crate::chrome::icons::chevron(&painter, chevron, c, self.expanded.contains(&id));
                    if r.clicked() {
                        action = Some((id, "toggle"));
                    }
                }
                let icon = Rect::from_min_size(egui::pos2(chevron.right(), row.top()), egui::vec2(16.0, ROW));
                let entity_color =
                    n.kind.entity().and_then(|e| mapcraft_render::EntityStyle::color(&self.project, &e.classname)).map(|c| Color32::from_rgb(c[0], c[1], c[2]));
                crate::chrome::icons::node_kind(&painter, icon, &n.kind, entity_color);

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
                    crate::chrome::icons::eye(&painter, eye, c, !n.hidden);
                    if r.on_hover_text(if n.hidden { "Show" } else { "Hide" }).clicked() {
                        action = Some((id, "visibility"));
                    }
                }

                let color = if self.session.active_parent == id {
                    crate::chrome::ACCENT
                } else if n.hidden {
                    crate::chrome::TEXT_WEAK.gamma_multiply(0.7)
                } else if matches!(n.kind, NodeKind::Brush(_)) {
                    crate::chrome::TEXT_WEAK
                } else {
                    crate::chrome::TEXT
                };
                let text_area = Rect::from_min_max(egui::pos2(icon.right() + 4.0, row.top()), egui::pos2(eye.left() - 2.0, row.bottom()));
                ui.painter_at(text_area).text(text_area.left_center(), egui::Align2::LEFT_CENTER, &label, egui::TextStyle::Body.resolve(ui.style()), color);

                if resp.clicked() {
                    action = Some((id, if ui.input(|i| i.modifiers.command) { "toggle-select" } else { "select" }));
                }
                if resp.drag_started() && id != ROOT {
                    self.tree_drag = if selected { self.dragged_selection(&rows) } else { vec![id] };
                }
                if !self.tree_drag.is_empty()
                    && let Some(p) = pointer.filter(|p| row.contains(*p))
                    && let Some((target, index, into)) = self.drop_target(id, (p.y - row.top()) / ROW)
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
                    if matches!(n.kind, NodeKind::World(_) | NodeKind::Layer(_) | NodeKind::Group(_)) && ui.button("New objects go here").clicked() {
                        action = Some((id, "activate"));
                    }
                    if id != ROOT && ui.button("Delete").clicked() {
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
                if !self.expanded.remove(&id) {
                    self.expanded.insert(id);
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

    fn tags_panel(&mut self, ui: &mut egui::Ui) {
        let any = !self.session.selection.is_empty();
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.tag_name).hint_text("New tag").desired_width(120.0));
            if ui.add_enabled(any, egui::Button::new("Tag selection")).clicked() {
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
                let mut shown = !t.hidden;
                if ui.checkbox(&mut shown, "").on_hover_text("Show or hide everything with this tag").changed() {
                    t.hidden = !shown;
                    edit = Some(if shown { "Show tag" } else { "Hide tag" });
                }
                ui.horizontal(|ui| {
                    if ui.add(egui::TextEdit::singleline(&mut t.name).desired_width(110.0)).changed() {
                        edit = Some("Rename tag");
                    }
                    ui.label(RichText::new(count.to_string()).weak());
                    if ui.small_button("Select").clicked() {
                        select = Some(t.id);
                    }
                    if ui.add_enabled(any, egui::Button::new("+").small()).on_hover_text("Add the selection").clicked() {
                        membership = Some((t.id, true));
                    }
                    if ui.add_enabled(any, egui::Button::new("−").small()).on_hover_text("Take the selection out").clicked() {
                        membership = Some((t.id, false));
                    }
                    if ui.small_button("🗙").on_hover_text("Delete the tag").clicked() {
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

        ui.separator();
        ui.label(RichText::new("Cordons").strong());
        ui.label(RichText::new("While a cordon is on, only what touches it is shown and compiled.").weak().small());
        let mut cordons = self.session.map.cordons.clone();
        let bounds = self.selection_bounds();
        let mut label = None;
        if ui.add_enabled(bounds.is_some(), egui::Button::new("New cordon around selection")).clicked()
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
                    if ui.add_enabled(bounds.is_some(), egui::Button::new("Fit").small()).on_hover_text("Fit to the selection").clicked()
                        && let Some(b) = bounds
                    {
                        c.bounds = b;
                        label = Some("Fit cordon");
                    }
                    if ui.small_button("🗙").clicked() {
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
        ui.add(egui::TextEdit::singleline(&mut self.material_filter).hint_text("Search materials"));
        if ui.add_enabled(!self.session.selection.is_empty(), egui::Button::new("Apply to selection")).clicked() {
            self.run_command("tools.apply_material");
        }
        let filter = self.material_filter.to_lowercase();
        let names: Vec<crate::project::Material> =
            self.project.materials.iter().filter(|m| filter.is_empty() || m.name.to_lowercase().contains(&filter)).cloned().collect();
        ui.horizontal(|ui| {
            ui.label(format!("{} materials", names.len()));
            ui.checkbox(&mut self.show_thumbs, "Previews");
        });
        let mut chosen = None;
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        if self.show_thumbs {
            let cell = crate::thumbs::SIZE as f32 + 8.0;
            let per_row = ((ui.available_width() / cell).floor() as usize).max(1);
            let rows = names.len().div_ceil(per_row);
            egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, cell + ROW, rows, |ui, range| {
                for row in range {
                    ui.horizontal(|ui| {
                        for m in names.iter().skip(row * per_row).take(per_row) {
                            let name = &m.name;
                            ui.vertical(|ui| {
                                ui.set_width(cell - 4.0);
                                let size = egui::Vec2::splat(crate::thumbs::SIZE as f32);
                                let selected = self.session.tex.material == *name;
                                let r = match self.thumbs.get(ui.ctx(), m) {
                                    Some(t) => ui.add(egui::Button::image(egui::Image::new((t.id(), size))).selected(selected)),
                                    None => ui.add_sized(size, egui::Button::new("…").selected(selected)),
                                };
                                let r = r.on_hover_text(name);
                                if r.clicked() {
                                    chosen = Some((name.clone(), false));
                                }
                                if r.double_clicked() {
                                    chosen = Some((name.clone(), true));
                                }
                                ui.add(egui::Label::new(RichText::new(name.rsplit('/').next().unwrap_or(name)).small()).truncate());
                            });
                        }
                    });
                }
            });
        } else {
            egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, ROW, names.len(), |ui, range| {
                for m in names.get(range).unwrap_or_default() {
                    let name = &m.name;
                    let r = ui.selectable_label(self.session.tex.material == *name, name);
                    if r.clicked() {
                        chosen = Some((name.clone(), false));
                    }
                    if r.double_clicked() {
                        chosen = Some((name.clone(), true));
                    }
                }
            });
        }
        if let Some((name, apply)) = chosen {
            self.session.tex.material = name;
            if apply {
                self.run_command("tools.apply_material");
            }
        }
    }

    fn entity_browser(&mut self, ui: &mut egui::Ui) {
        ui.add(egui::TextEdit::singleline(&mut self.entity_filter).hint_text("Search entities"));
        let filter = self.entity_filter.to_lowercase();
        let classes: Vec<(String, bool, String)> = self
            .project
            .game
            .class_names()
            .filter(|n| filter.is_empty() || n.to_lowercase().contains(&filter))
            .filter_map(|n| self.project.game.class(n).map(|c| (c.name, c.kind.is_solid(), c.description)))
            .collect();
        if classes.is_empty() {
            ui.label("No entity classes. The SDK's FGD files define them.");
        }
        let mut pick = None;
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, ROW, classes.len(), |ui, range| {
            for (name, solid, desc) in classes.get(range).unwrap_or_default() {
                let label = format!("{} {name}", if *solid { "■" } else { "○" });
                let mut r = ui.selectable_label(self.entity_class == *name, label);
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
                if !self.session.selection.is_empty() {
                    if let Err(e) = self.session.tie_to_entity(&name) {
                        self.status = e.to_string();
                    }
                    self.mark_changed();
                } else {
                    self.status = format!("Select brushes and click {name} to tie them to it, or select a platform and use Tools > Zone from Selection");
                }
                self.session.default_solid_entity = name;
            } else {
                self.entity_class = name;
                self.tool = Tool::Entity;
            }
        }
    }

    pub(crate) fn bottom_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for (tab, label) in [(BottomTab::Output, "Output"), (BottomTab::Issues, "Issues"), (BottomTab::Find, "Find")] {
                if ui.selectable_label(self.bottom_tab == tab, label).clicked() {
                    self.bottom_tab = tab;
                }
            }
            if ui.small_button("Clear").clicked() {
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
            if ui.add_enabled(fixable > 0, egui::Button::new(format!("Fix all ({fixable})"))).clicked() {
                fix = Some(self.issues.clone());
            }
            if ui.button("Check again").clicked() {
                self.check_problems();
            }
        });
        let mut select = None;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            for (i, issue) in self.issues.iter().enumerate() {
                let color = if issue.severity == Severity::Error { Color32::LIGHT_RED } else { Color32::YELLOW };
                ui.horizontal(|ui| {
                    if issue.fix.is_some() && ui.small_button("Fix").clicked() {
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
            if ui.add_enabled(!results.is_empty(), egui::Button::new("Select all")).clicked() {
                action = Some("select");
            }
            ui.separator();
            ui.add(egui::TextEdit::singleline(&mut self.find.replace).hint_text("replace with").desired_width(140.0));
            ui.checkbox(&mut self.find.selected_only, "In selection");
            let can = !self.find.query.is_empty() && !self.find.query.contains('=');
            if ui.add_enabled(can, egui::Button::new("Replace in values")).on_hover_text("Replaces the text in key values and output targets").clicked() {
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
        if self.tool == Tool::Block && self.plugin_shape.is_some() {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| self.plugin_shape_inspector(ui));
            return;
        }
        if self.tool == Tool::Face {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| self.face_editor(ui));
            return;
        }
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            let sel: Vec<NodeId> = self.session.selection.iter().copied().collect();
            match sel.as_slice() {
                [] => {
                    ui.label(RichText::new("World").heading());
                    self.entity_editor(ui, ROOT);
                }
                [id] => {
                    let kind = self.session.map.get(*id).map(|n| n.kind.clone());
                    let solid_entity = self.session.map.get(*id).is_some_and(|n| !n.children.is_empty());
                    match kind {
                        Some(NodeKind::World(_) | NodeKind::Entity(_)) => {
                            ui.label(RichText::new(if solid_entity { "Brush entity" } else { "Entity" }).heading());
                            self.entity_editor(ui, *id);
                            if self.session.map.get(*id).is_some_and(|n| !n.children.is_empty()) {
                                ui.separator();
                                self.brush_editor(ui);
                            }
                        }
                        Some(NodeKind::Brush(_)) => self.brush_editor(ui),
                        Some(NodeKind::Layer(mut l)) => {
                            ui.heading("Layer");
                            if ui.text_edit_singleline(&mut l.name).changed() {
                                self.set_kind(*id, NodeKind::Layer(l), "Rename layer");
                            }
                            self.active_button(ui, *id);
                        }
                        Some(NodeKind::Group(mut g)) => {
                            ui.heading("Group");
                            if ui.text_edit_singleline(&mut g.name).changed() {
                                self.set_kind(*id, NodeKind::Group(g.clone()), "Rename group");
                            }
                            if let Some(generator) = g.generator.clone() {
                                ui.separator();
                                self.generator_inspector(ui, *id, generator);
                            }
                            if let Some(m) = g.mirror {
                                ui.separator();
                                self.mirror_inspector(ui, *id, g.clone(), m);
                            }
                            self.active_button(ui, *id);
                            ui.separator();
                            self.brush_editor(ui);
                        }
                        Some(NodeKind::Prefab(p)) => self.prefab_inspector(ui, *id, p),
                        None => {}
                    }
                }
                many => {
                    ui.heading(format!("{} selected", many.len()));
                    self.brush_editor(ui);
                }
            }
        });
    }

    fn mirror_inspector(&mut self, ui: &mut egui::Ui, id: NodeId, g: Group, m: Mirror) {
        ui.label(RichText::new("Mirror").strong());
        ui.label(RichText::new("The children are copied across the plane and kept in step.").weak().small());
        let mut edited = m;
        ui.horizontal(|ui| {
            ui.label("Axis");
            let axes = [(Axis::X, "X", "Mirror left and right"), (Axis::Y, "Y", "Mirror front and back"), (Axis::Z, "Z", "Mirror up and down")];
            if Self::segmented(ui, &mut edited.axis, &axes) {
                let children = self.session.map.get(id).map(|n| n.children.clone()).unwrap_or_default();
                edited.offset = self.session.mirror_offset(&children, edited.axis).unwrap_or(edited.offset);
            }
        });
        ui.horizontal(|ui| {
            ui.label("Plane at");
            ui.add(egui::DragValue::new(&mut edited.offset).speed(1.0));
        });
        ui.checkbox(&mut edited.texture_lock, "Texture lock").on_hover_text("Copies carry their textures across the plane");
        if edited != m {
            self.set_kind(id, NodeKind::Group(Group { mirror: Some(edited), ..g }), "Edit mirror");
        }
        if ui.button("Make real").on_hover_text("Turn the mirrored half into ordinary objects and stop mirroring").clicked() {
            self.run_action("tools.mirror_make_real");
        }
    }

    fn prefab_inspector(&mut self, ui: &mut egui::Ui, id: NodeId, p: Prefab) {
        ui.heading("Prefab");
        ui.label(&p.path);
        let mut edited = p.clone();
        egui::Grid::new(("prefab", id.0)).num_columns(4).show(ui, |ui| {
            ui.label("Origin");
            for v in edited.origin.as_mut() {
                ui.add(egui::DragValue::new(v).speed(1.0));
            }
            ui.end_row();
            ui.label("Pitch Yaw Roll");
            for v in edited.angles.as_mut() {
                ui.add(egui::DragValue::new(v).speed(1.0).suffix("°"));
            }
            ui.end_row();
        });
        let doc = self.session.path.clone();
        let file = crate::app::prefab_loader(&mut self.prefab_cache, doc.clone())(&p.path);
        ui.horizontal(|ui| {
            if ui.button("Open prefab").on_hover_text("Edit the prefab's own map").clicked()
                && let Ok(path) = craft_docpath::resolve_absolute(&p.path, doc.as_deref())
            {
                self.open_dialog_path(path);
            }
            if ui.button("Make local").clicked() {
                self.run_action("prefab.make_local");
            }
        });
        let Some(src) = file else {
            ui.colored_label(Color32::LIGHT_RED, "The prefab file cannot be loaded.");
            self.commit_prefab(id, &p, edited);
            return;
        };
        ui.separator();
        ui.label(RichText::new("Overrides").strong());
        ui.label(RichText::new("Changes here apply to this instance only.").weak().small());
        for (eid, n) in src.nodes() {
            let Some(e) = n.kind.entity().filter(|_| matches!(n.kind, NodeKind::Entity(_))) else { continue };
            egui::CollapsingHeader::new(n.kind.label()).id_salt(("ov", id.0, eid.0)).show(ui, |ui| {
                egui::Grid::new(("ov-grid", id.0, eid.0)).num_columns(3).show(ui, |ui| {
                    // Overrides apply after the instance is placed, so a moved origin would land in world space.
                    for (k, original) in e.properties.iter().filter(|(k, _)| k != "origin") {
                        let pos = edited.overrides.iter().position(|o| o.node == eid && o.key == *k);
                        let mut value = pos.and_then(|i| edited.overrides.get(i)).and_then(|o| o.value.clone()).unwrap_or_else(|| original.clone());
                        ui.label(if pos.is_some() { RichText::new(k).strong() } else { RichText::new(k) });
                        if ui.text_edit_singleline(&mut value).changed() {
                            let o = PropertyOverride { node: eid, key: k.clone(), value: Some(value) };
                            match pos {
                                Some(i) => edited.overrides[i] = o,
                                None => edited.overrides.push(o),
                            }
                        }
                        if ui.add_enabled(pos.is_some(), egui::Button::new("↺").small()).on_hover_text("Use the prefab's value").clicked()
                            && let Some(i) = pos
                        {
                            edited.overrides.remove(i);
                        }
                        ui.end_row();
                    }
                });
            });
        }
        self.commit_prefab(id, &p, edited);
    }

    fn commit_prefab(&mut self, id: NodeId, old: &Prefab, new: Prefab) {
        if *old != new {
            self.set_kind(id, NodeKind::Prefab(new), "Edit prefab");
        }
    }

    fn face_editor(&mut self, ui: &mut egui::Ui) {
        let faces: Vec<(NodeId, Face)> = self
            .session
            .face_selection
            .iter()
            .filter_map(|(id, f)| Some((*id, self.session.map.get(*id)?.kind.brush()?.faces.iter().find(|x| x.id == *f)?.clone())))
            .collect();
        ui.heading(format!("{} face{}", faces.len(), if faces.len() == 1 { "" } else { "s" }));
        let Some((_, first)) = faces.first().cloned() else {
            ui.label("Click faces in the 3D view; Ctrl+click adds more.");
            ui.label("Alt+click picks a face's material, Alt+right click applies the current one.");
            return;
        };
        ui.label(format!("Material: {}", first.material));
        ui.horizontal(|ui| {
            if ui.button("Apply current material").clicked() {
                let m = self.session.tex.material.clone();
                self.face_edit("Apply material", None, move |f| f.material = m.clone());
            }
            let picked = self.picked_face.clone();
            let tip = "Material and alignment of the face last picked with Alt+click, wrapped across shared edges";
            if ui.add_enabled(picked.is_some(), egui::Button::new("Paste alignment")).on_hover_text(tip).clicked()
                && let Some(src) = picked
            {
                self.face_edit("Apply material and alignment", None, move |f| f.wrap_from(&src));
            }
        });
        let (mut uo, mut vo, mut us, mut vs, mut rot) = (first.u.offset, first.v.offset, first.u.scale, first.v.scale, first.rotation);
        let mut changed = false;
        egui::Grid::new("face-tex").num_columns(3).show(ui, |ui| {
            ui.label("");
            ui.label("U");
            ui.label("V");
            ui.end_row();
            ui.label("Offset");
            changed |= ui.add(egui::DragValue::new(&mut uo).speed(1.0)).changed();
            changed |= ui.add(egui::DragValue::new(&mut vo).speed(1.0)).changed();
            ui.end_row();
            ui.label("Scale");
            changed |= ui.add(egui::DragValue::new(&mut us).speed(0.01)).changed();
            changed |= ui.add(egui::DragValue::new(&mut vs).speed(0.01)).changed();
            ui.end_row();
        });
        let rot_changed = ui.horizontal(|ui| {
            ui.label("Rotation");
            ui.add(egui::DragValue::new(&mut rot).speed(1.0).suffix("°")).changed()
        });
        if changed {
            self.face_edit("Edit texture", Some("face-tex"), move |f| {
                (f.u.offset, f.v.offset) = (uo, vo);
                if us != 0.0 {
                    f.u.scale = us;
                }
                if vs != 0.0 {
                    f.v.scale = vs;
                }
            });
        }
        if rot_changed.inner {
            let delta = rot - first.rotation;
            self.face_edit("Rotate texture", Some("face-rot"), move |f| f.rotate_texture(delta));
        }
        ui.label("Justify");
        ui.horizontal(|ui| {
            for (label, mode) in
                [("Fit", Justify::Fit), ("L", Justify::Left), ("R", Justify::Right), ("T", Justify::Top), ("B", Justify::Bottom), ("C", Justify::Center)]
            {
                if ui.button(label).clicked() {
                    self.justify_faces(mode);
                }
            }
        });
        ui.horizontal(|ui| {
            if ui.button("Align to world").clicked() {
                self.face_edit("Align to world", None, |f| f.align_to_world());
            }
            if ui.button("Align to face").clicked() {
                self.face_edit("Align to face", None, |f| f.align_to_face());
            }
        });
        let mut lightmap = first.attrs.get("lightmapscale").cloned().unwrap_or_default();
        ui.horizontal(|ui| {
            ui.label("Lightmap scale");
            if ui.add(egui::TextEdit::singleline(&mut lightmap).desired_width(60.0)).changed() {
                let v = lightmap.clone();
                self.face_edit("Lightmap scale", Some("face-lm"), move |f| {
                    if v.is_empty() {
                        f.attrs.remove("lightmapscale");
                    } else {
                        f.attrs.insert("lightmapscale".into(), v.clone());
                    }
                });
            }
        });
        self.displacement_editor(ui, faces.iter().any(|(_, f)| f.disp.is_some()));
    }

    /// Making, removing and heightmapping displacements on the picked faces.
    fn displacement_editor(&mut self, ui: &mut egui::Ui, any: bool) {
        ui.separator();
        ui.label(RichText::new("Displacement").strong());
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("disp-power").selected_text(format!("Power {}", self.disp_power)).show_ui(ui, |ui| {
                for p in 2..=mapcraft_map::disp::MAX_POWER {
                    let n = (1 << p) + 1;
                    ui.selectable_value(&mut self.disp_power, p, format!("Power {p} ({n} by {n})"));
                }
            });
            let label = if any { "Set power" } else { "Make" };
            if ui.button(label).on_hover_text("Turns four-cornered faces into a grid you can sculpt").clicked() {
                let r = self.session.displace_faces(self.disp_power).map(|_| ());
                self.after_edit(r);
            }
            if ui.add_enabled(any, egui::Button::new("Remove")).clicked() {
                let r = self.session.remove_displacements();
                self.after_edit(r);
            }
        });
        ui.add_enabled_ui(any, |ui| {
            ui.horizontal(|ui| {
                ui.label("Heightmap");
                ui.add(egui::DragValue::new(&mut self.heightmap_range.0).speed(4.0).prefix("black "));
                ui.add(egui::DragValue::new(&mut self.heightmap_range.1).speed(4.0).prefix("white "));
                if ui.button("Load…").on_hover_text("A grayscale image stretched over all the picked displacements").clicked() {
                    self.load_heightmap();
                }
            });
        });
        ui.label(RichText::new(format!("Sculpt them with the Sculpt tool ({}).", self.chord("tool.sculpt"))).weak().small());
    }

    fn after_edit(&mut self, result: Result<(), mapcraft_engine::EngineError>) {
        if let Err(e) = result {
            self.status = e.to_string();
        }
        self.mark_changed();
    }

    fn load_heightmap(&mut self) {
        let Some(path) = rfd::FileDialog::new().add_filter("Image", &["png", "jpg", "jpeg", "tga", "bmp", "tif", "tiff", "exr"]).pick_file() else { return };
        let img = match image::open(&path) {
            Ok(i) => i.to_luma32f(),
            Err(e) => {
                self.status = format!("cannot read {}: {e}", path.display());
                return;
            }
        };
        let (w, h) = (img.width() as usize, img.height() as usize);
        let (low, high) = self.heightmap_range;
        let r = self.session.apply_heightmap(img.as_raw(), w, h, low, high);
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
            let size = b
                .faces
                .get(i)
                .and_then(|f| self.gpu.as_ref()?.renderer.material_size(&f.material))
                .map_or(DVec2::splat(64.0), |s| DVec2::new(f64::from(s[0]), f64::from(s[1])));
            polys.insert(*face, (w.0.clone(), size));
        }
        self.face_edit("Justify texture", None, move |f| {
            if let Some((poly, size)) = polys.get(&f.id) {
                f.justify(poly, *size, mode);
            }
        });
    }

    fn active_button(&mut self, ui: &mut egui::Ui, id: NodeId) {
        let active = self.session.active_parent == id;
        if ui.add_enabled(!active, egui::Button::new(if active { "New objects go here" } else { "Put new objects here" })).clicked() {
            self.session.active_parent = id;
        }
    }

    fn set_kind(&mut self, id: NodeId, kind: NodeKind, label: &str) {
        if let Err(e) = self.session.set_kind(id, kind, label, Some(&format!("edit:{}", id.0))) {
            self.status = e.to_string();
        }
        self.mark_changed();
    }

    fn entity_editor(&mut self, ui: &mut egui::Ui, id: NodeId) {
        let Some(node) = self.session.map.get(id) else { return };
        let Some(ent) = node.kind.entity().cloned() else { return };
        let is_world = matches!(node.kind, NodeKind::World(_));
        let solid = !node.children.is_empty();
        let mut e = ent.clone();
        let class = self.project.game.class(&e.classname);

        ui.horizontal(|ui| {
            if is_world {
                ui.label(RichText::new(&e.classname).strong());
            } else {
                let names: Vec<String> = self
                    .project
                    .game
                    .class_names()
                    .filter(|n| self.project.game.class(n).is_some_and(|c| c.kind.is_solid() == solid))
                    .map(String::from)
                    .collect();
                if names.is_empty() {
                    ui.add(egui::TextEdit::singleline(&mut e.classname).desired_width(170.0));
                } else {
                    egui::ComboBox::from_id_salt(("class", id.0)).selected_text(&e.classname).width(170.0).height(400.0).show_ui(ui, |ui| {
                        for n in names {
                            ui.selectable_value(&mut e.classname, n.clone(), n);
                        }
                    });
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let tip = format!("Off shows the raw key values  ({})", self.chord("inspector.smart_edit"));
                ui.toggle_value(&mut self.smart_edit, "Smart edit").on_hover_text(tip);
            });
        });

        let inputs = self.inputs_to(id);
        if !is_world {
            ui.horizontal(|ui| {
                let tabs = [
                    (InspectorTab::Keyvalues, "Keyvalues".to_string()),
                    (InspectorTab::Outputs, format!("Outputs {}", e.connections.len())),
                    (InspectorTab::Inputs, format!("Inputs {}", inputs.len())),
                ];
                for (tab, label) in tabs {
                    if ui.selectable_label(self.inspector_tab == tab, label).clicked() {
                        self.inspector_tab = tab;
                    }
                }
            });
        }
        ui.separator();
        let tab = if is_world { InspectorTab::Keyvalues } else { self.inspector_tab };
        let mut help: Option<(String, String)> = None;
        match tab {
            InspectorTab::Keyvalues if self.smart_edit => {
                egui::Grid::new(("props", id.0)).num_columns(2).striped(true).spacing([8.0, 4.0]).show(ui, |ui| {
                    if let Some(c) = &class {
                        for p in c.properties.iter().filter(|p| p.name != "origin" || !solid) {
                            let label = if p.display.is_empty() { p.name.clone() } else { p.display.clone() };
                            let set = e.get(&p.name).is_some();
                            let name =
                                ui.label(if set { RichText::new(label.as_str()) } else { RichText::new(label.as_str()).color(crate::chrome::TEXT_WEAK) });
                            let current = e.get(&p.name).map(String::from).or_else(|| p.default.clone()).unwrap_or_default();
                            let mut value = current.clone();
                            let cell = ui.scope(|ui| property_widget(ui, (id.0, &p.name), p, &mut value)).response;
                            if name.hovered() || cell.contains_pointer() {
                                help = Some((format!("{label} ({})", p.name), p.description.clone()));
                            }
                            if value != current {
                                e.set(p.name.clone(), value);
                            }
                            ui.end_row();
                        }
                    }
                    let known: BTreeSet<String> = class.iter().flat_map(|c| c.properties.iter().map(|p| p.name.to_lowercase())).collect();
                    let mut remove = None;
                    for (i, (k, v)) in e.properties.iter_mut().enumerate() {
                        if known.contains(&k.to_lowercase()) {
                            continue;
                        }
                        ui.label(RichText::new(k.as_str()).italics()).on_hover_text("Not in the SDK's FGDs");
                        ui.horizontal(|ui| {
                            ui.text_edit_singleline(v);
                            if ui.small_button("🗙").clicked() {
                                remove = Some(i);
                            }
                        });
                        ui.end_row();
                    }
                    if let Some(i) = remove {
                        e.properties.remove(i);
                    }
                });
                add_key_row(ui, id, &mut e);
            }
            InspectorTab::Keyvalues => {
                egui::Grid::new(("raw", id.0)).num_columns(2).striped(true).show(ui, |ui| {
                    ui.label(RichText::new("Key").color(crate::chrome::TEXT_WEAK));
                    ui.label(RichText::new("Value").color(crate::chrome::TEXT_WEAK));
                    ui.end_row();
                    let mut remove = None;
                    for (i, (k, v)) in e.properties.iter_mut().enumerate() {
                        ui.add(egui::TextEdit::singleline(k).desired_width(110.0));
                        ui.horizontal(|ui| {
                            ui.text_edit_singleline(v);
                            if ui.small_button("🗙").clicked() {
                                remove = Some(i);
                            }
                        });
                        ui.end_row();
                    }
                    if let Some(i) = remove {
                        e.properties.remove(i);
                    }
                });
                add_key_row(ui, id, &mut e);
            }
            InspectorTab::Outputs => {
                let outputs: Vec<String> = class.iter().flat_map(|c| c.outputs.iter().map(|o| o.name.clone())).collect();
                connections_editor(ui, id, &mut e.connections, &outputs);
            }
            InspectorTab::Inputs => {
                if inputs.is_empty() {
                    ui.label(RichText::new("No outputs target this entity.").color(crate::chrome::TEXT_WEAK));
                }
                let mut go = None;
                for (source, c) in &inputs {
                    let name = self.session.map.get(*source).map_or_else(String::new, |n| n.kind.label());
                    let text =
                        format!("{name}  {} \u{2192} {}{}", c.output, c.input, if c.param.is_empty() { String::new() } else { format!(" ({})", c.param) });
                    if ui.selectable_label(false, text).on_hover_text("Select the entity that sends it").clicked() {
                        go = Some(*source);
                    }
                }
                if let Some(source) = go {
                    self.session.selection = BTreeSet::from([source]);
                    self.mark_changed();
                }
            }
        }

        let (title, text) = help.unwrap_or_else(|| (e.classname.clone(), class.as_ref().map(|c| c.description.clone()).unwrap_or_default()));
        if !text.is_empty() {
            ui.add_space(6.0);
            egui::Frame::new().fill(crate::chrome::BG).corner_radius(4.0).inner_margin(6.0).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(title).strong());
                ui.label(RichText::new(text).color(crate::chrome::TEXT_WEAK));
            });
        }
        if e != ent {
            let kind = if is_world { NodeKind::World(e) } else { NodeKind::Entity(e) };
            self.set_kind(id, kind, "Edit entity");
        }
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

    fn selected_brushes(&self) -> Vec<NodeId> {
        let mut out = BTreeSet::new();
        for id in &self.session.selection {
            out.extend(self.session.map.subtree(*id).into_iter().filter(|n| self.session.map.get(*n).is_some_and(|n| matches!(n.kind, NodeKind::Brush(_)))));
        }
        out.into_iter().collect()
    }

    fn edit_brushes(&mut self, label: &str, f: impl Fn(&mut Brush)) {
        let ids = self.selected_brushes();
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
        self.session.apply(label, c);
        self.mark_changed();
    }

    fn brush_editor(&mut self, ui: &mut egui::Ui) {
        let ids = self.selected_brushes();
        if ids.is_empty() {
            return;
        }
        ui.heading(format!("{} brush{}", ids.len(), if ids.len() == 1 { "" } else { "es" }));
        let materials: BTreeSet<String> =
            ids.iter().filter_map(|id| self.session.map.get(*id)?.kind.brush().cloned()).flat_map(|b| b.faces.into_iter().map(|f| f.material)).collect();
        ui.label(match materials.len() {
            0 => "No faces".to_string(),
            1 => format!("Material: {}", materials.iter().next().map_or("", String::as_str)),
            n => format!("{n} materials"),
        });
        ui.horizontal(|ui| {
            if ui.button("Apply current material").clicked() {
                self.run_command("tools.apply_material");
            }
        });
        ui.horizontal(|ui| {
            if ui.button("Align to world").clicked() {
                self.edit_brushes("Align to world", |b| b.faces.iter_mut().for_each(|f| f.align_to_world()));
            }
            if ui.button("Align to face").clicked() {
                self.edit_brushes("Align to face", |b| b.faces.iter_mut().for_each(|f| f.align_to_face()));
            }
        });
        let scale_id = egui::Id::new("texture-scale");
        let mut scale: f64 = ui.data_mut(|d| d.get_temp(scale_id).unwrap_or(self.session.tex.scale));
        ui.horizontal(|ui| {
            ui.label("Texture scale");
            ui.add(egui::DragValue::new(&mut scale).speed(0.01).range(0.01..=64.0));
            if ui.button("Set").clicked() {
                self.edit_brushes("Texture scale", |b| {
                    for f in &mut b.faces {
                        f.u.scale = scale;
                        f.v.scale = scale;
                    }
                });
            }
        });
        ui.data_mut(|d| d.insert_temp(scale_id, scale));
        ui.horizontal(|ui| {
            for (label, du, dv) in [("⬅", -16.0, 0.0), ("➡", 16.0, 0.0), ("⬆", 0.0, 16.0), ("⬇", 0.0, -16.0)] {
                if ui.button(label).on_hover_text("Shift the texture by 16 texels").clicked() {
                    self.edit_brushes("Shift texture", |b| {
                        for f in &mut b.faces {
                            f.u.offset += du;
                            f.v.offset += dv;
                        }
                    });
                }
            }
        });
        ui.horizontal(|ui| {
            for (label, id) in [("Carve", "tools.carve"), ("Hollow", "tools.hollow"), ("Tie to entity", "tools.tie_to_entity")] {
                if ui.button(label).clicked() {
                    self.run_command(id);
                }
            }
        });
    }
}

fn add_key_row(ui: &mut egui::Ui, id: NodeId, e: &mut mapcraft_map::Entity) {
    ui.horizontal(|ui| {
        let key_id = egui::Id::new(("new-key", id.0));
        let mut key: String = ui.data_mut(|d| d.get_temp(key_id).unwrap_or_default());
        ui.add(egui::TextEdit::singleline(&mut key).hint_text("new key").desired_width(110.0));
        if ui.add_enabled(!key.trim().is_empty(), egui::Button::new("Add")).clicked() {
            e.set(key.trim(), "");
            key.clear();
        }
        ui.data_mut(|d| d.insert_temp(key_id, key));
    });
}

fn property_widget(ui: &mut egui::Ui, id: (u64, &str), p: &mapcraft_fgd::Property, value: &mut String) {
    match p.ty.as_str() {
        "choices" if !p.choices.is_empty() => {
            let selected = p.choices.iter().find(|c| c.value == *value).map_or(value.clone(), |c| c.label.clone());
            egui::ComboBox::from_id_salt(id).selected_text(selected).show_ui(ui, |ui| {
                for c in &p.choices {
                    ui.selectable_value(value, c.value.clone(), &c.label);
                }
            });
        }
        "flags" => {
            let mut bits: u64 = value.trim().parse().unwrap_or(0);
            ui.vertical(|ui| {
                for c in &p.choices {
                    let Ok(bit) = c.value.trim().parse::<u64>() else { continue };
                    let mut on = bits & bit != 0;
                    if ui.checkbox(&mut on, &c.label).changed() {
                        bits = if on { bits | bit } else { bits & !bit };
                    }
                }
            });
            *value = bits.to_string();
        }
        "boolean" => {
            let mut on = value.trim() == "1";
            if ui.checkbox(&mut on, "").changed() {
                *value = if on { "1" } else { "0" }.into();
            }
        }
        "color255" | "color1" => {
            ui.horizontal(|ui| {
                let parts: Vec<f32> = value.split_whitespace().filter_map(|t| t.parse().ok()).collect();
                let scale = if p.ty == "color1" { 255.0 } else { 1.0 };
                let mut rgb = [0u8; 3];
                for (i, c) in rgb.iter_mut().enumerate() {
                    *c = parts.get(i).map_or(255.0, |v| v * scale).clamp(0.0, 255.0) as u8;
                }
                if ui.color_edit_button_srgb(&mut rgb).changed() {
                    let rest: Vec<String> = parts.iter().skip(3).map(|v| v.to_string()).collect();
                    let mut out: Vec<String> =
                        rgb.iter().map(|c| if p.ty == "color1" { format!("{:.3}", f32::from(*c) / 255.0) } else { c.to_string() }).collect();
                    out.extend(rest);
                    *value = out.join(" ");
                }
                ui.text_edit_singleline(value);
            });
        }
        _ => {
            ui.text_edit_singleline(value);
        }
    }
}

fn connections_editor(ui: &mut egui::Ui, id: NodeId, conns: &mut Vec<Connection>, outputs: &[String]) {
    let mut remove = None;
    for (i, c) in conns.iter_mut().enumerate() {
        ui.push_id((id.0, i), |ui| {
            ui.horizontal(|ui| {
                if outputs.is_empty() {
                    ui.add(egui::TextEdit::singleline(&mut c.output).hint_text("output").desired_width(90.0));
                } else {
                    egui::ComboBox::from_id_salt("out").selected_text(&c.output).width(90.0).show_ui(ui, |ui| {
                        for o in outputs {
                            ui.selectable_value(&mut c.output, o.clone(), o);
                        }
                    });
                }
                ui.add(egui::TextEdit::singleline(&mut c.target).hint_text("target").desired_width(80.0));
                ui.add(egui::TextEdit::singleline(&mut c.input).hint_text("input").desired_width(80.0));
                if ui.small_button("🗙").clicked() {
                    remove = Some(i);
                }
            });
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut c.param).hint_text("parameter").desired_width(90.0));
                ui.label("delay");
                ui.add(egui::DragValue::new(&mut c.delay).speed(0.05).range(0.0..=3600.0));
                let mut once = c.times_to_fire == 1;
                if ui.checkbox(&mut once, "once").changed() {
                    c.times_to_fire = if once { 1 } else { -1 };
                }
            });
        });
    }
    if let Some(i) = remove {
        conns.remove(i);
    }
    if ui.button("Add output").clicked() {
        conns.push(Connection {
            output: outputs.first().cloned().unwrap_or_default(),
            target: String::new(),
            input: String::new(),
            param: String::new(),
            delay: 0.0,
            times_to_fire: -1,
        });
    }
}
