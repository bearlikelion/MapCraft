use std::collections::{BTreeMap, BTreeSet};

use egui::{Color32, RichText};
use mapcraft_engine::Severity;
use mapcraft_geom::DVec2;
use mapcraft_map::{Brush, Connection, Face, Justify, NodeId, NodeKind, ROOT};

use crate::app::{BottomTab, LeftTab, MapCraftApp};
use crate::viewport::Tool;

const ROW: f32 = 18.0;

impl MapCraftApp {
    pub(crate) fn left_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for (tab, label) in [(LeftTab::Scene, "Scene"), (LeftTab::Materials, "Materials"), (LeftTab::Entities, "Entities")] {
                if ui.selectable_label(self.left_tab == tab, label).clicked() {
                    self.left_tab = tab;
                }
            }
        });
        ui.separator();
        match self.left_tab {
            LeftTab::Scene => self.scene_tree(ui),
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

    fn scene_tree(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("New Layer").clicked() {
                let n = self.session.map.nodes().filter(|(_, n)| matches!(n.kind, NodeKind::Layer(_))).count() + 1;
                if let Err(e) = self.session.create_layer(&format!("Layer {n}")) {
                    self.status = e.to_string();
                }
                self.mark_changed();
            }
            if ui.add_enabled(!self.session.selection.is_empty(), egui::Button::new("Group")).clicked() {
                self.run_command("tools.group");
            }
        });
        let rows = self.scene_rows();
        let mut action: Option<(NodeId, &'static str)> = None;
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, ROW, rows.len(), |ui, range| {
            for &(id, depth) in rows.get(range).unwrap_or_default() {
                let Some(n) = self.session.map.get(id) else { continue };
                ui.horizontal(|ui| {
                    ui.add_space(depth as f32 * 12.0);
                    if n.children.is_empty() {
                        ui.add_space(18.0);
                    } else {
                        let open = self.expanded.contains(&id);
                        if ui.small_button(if open { "⏷" } else { "⏵" }).clicked() {
                            action = Some((id, "toggle"));
                        }
                    }
                    if id != ROOT && ui.small_button(if n.hidden { "○" } else { "👁" }).on_hover_text("Show or hide").clicked() {
                        action = Some((id, "visibility"));
                    }
                    let active = self.session.active_parent == id;
                    let mut text = RichText::new(n.kind.label());
                    if matches!(n.kind, NodeKind::Layer(_) | NodeKind::Group(_)) {
                        text = text.strong();
                    }
                    if active {
                        text = text.color(Color32::LIGHT_BLUE);
                    }
                    if n.hidden {
                        text = text.weak();
                    }
                    let r = ui.selectable_label(self.session.selection.contains(&id), text);
                    if r.clicked() {
                        action = Some((id, if ui.input(|i| i.modifiers.command) { "toggle-select" } else { "select" }));
                    }
                    if r.double_clicked() {
                        action = Some((id, "frame"));
                    }
                    r.context_menu(|ui| {
                        if matches!(n.kind, NodeKind::World(_) | NodeKind::Layer(_) | NodeKind::Group(_)) && ui.button("New objects go here").clicked() {
                            action = Some((id, "activate"));
                        }
                        if id != ROOT && ui.button("Delete").clicked() {
                            action = Some((id, "delete"));
                        }
                    });
                });
            }
        });
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
        let names: Vec<&str> =
            self.project.materials.iter().map(|(n, _)| n.as_str()).filter(|n| filter.is_empty() || n.to_lowercase().contains(&filter)).collect();
        ui.label(format!("{} materials", names.len()));
        let mut chosen = None;
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, ROW, names.len(), |ui, range| {
            for name in names.get(range).unwrap_or_default() {
                let r = ui.selectable_label(self.session.tex.material == *name, *name);
                if r.clicked() {
                    chosen = Some((name.to_string(), false));
                }
                if r.double_clicked() {
                    chosen = Some((name.to_string(), true));
                }
            }
        });
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
                    self.status = format!("Select brushes, then click {name} to tie them to it");
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
            for (tab, label) in [(BottomTab::Output, "Output"), (BottomTab::Issues, "Issues")] {
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
            BottomTab::Issues => {
                if self.issues.is_empty() {
                    ui.label("No problems found.");
                }
                let mut select = None;
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    for i in &self.issues {
                        let color = if i.severity == Severity::Error { Color32::LIGHT_RED } else { Color32::YELLOW };
                        if ui.selectable_label(false, RichText::new(format!("node {}: {}", i.node.0, i.message)).color(color)).clicked() {
                            select = Some(i.node);
                        }
                    }
                });
                if let Some(id) = select {
                    self.session.selection = BTreeSet::from([id]);
                    self.frame_selection();
                    self.mark_changed();
                }
            }
        }
    }

    pub(crate) fn inspector(&mut self, ui: &mut egui::Ui) {
        if self.tool == Tool::Face {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| self.face_editor(ui));
            return;
        }
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            let sel: Vec<NodeId> = self.session.selection.iter().copied().collect();
            match sel.as_slice() {
                [] => {
                    ui.heading("World");
                    self.entity_editor(ui, ROOT);
                }
                [id] => {
                    let kind = self.session.map.get(*id).map(|n| n.kind.clone());
                    match kind {
                        Some(NodeKind::World(_) | NodeKind::Entity(_)) => {
                            ui.heading("Entity");
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
                            if let Some(gen_) = &g.generator {
                                ui.label(format!("Generated by {} / {}", gen_.plugin, gen_.command));
                            }
                            self.active_button(ui, *id);
                            ui.separator();
                            self.brush_editor(ui);
                        }
                        Some(NodeKind::Prefab(p)) => {
                            ui.heading("Prefab");
                            ui.label(&p.path);
                            ui.label(format!("Origin {:?}", p.origin.to_array()));
                        }
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
        if ui.button("Apply current material").clicked() {
            let m = self.session.tex.material.clone();
            self.face_edit("Apply material", None, move |f| f.material = m.clone());
        }
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
            ui.label("Class");
            if is_world {
                ui.label(&e.classname);
                return;
            }
            let names: Vec<String> =
                self.project.game.class_names().filter(|n| self.project.game.class(n).is_some_and(|c| c.kind.is_solid() == solid)).map(String::from).collect();
            if names.is_empty() {
                ui.text_edit_singleline(&mut e.classname);
            } else {
                egui::ComboBox::from_id_salt(("class", id.0)).selected_text(&e.classname).height(400.0).show_ui(ui, |ui| {
                    for n in names {
                        ui.selectable_value(&mut e.classname, n.clone(), n);
                    }
                });
            }
        });
        if let Some(c) = &class
            && !c.description.is_empty()
        {
            ui.label(RichText::new(&c.description).weak().small());
        }
        ui.separator();
        egui::Grid::new(("props", id.0)).num_columns(2).striped(true).show(ui, |ui| {
            if let Some(c) = &class {
                for p in c.properties.iter().filter(|p| p.name != "origin" || !solid) {
                    let label = if p.display.is_empty() { p.name.clone() } else { p.display.clone() };
                    let r = ui.label(label);
                    if !p.description.is_empty() {
                        r.on_hover_text(&p.description);
                    }
                    let current = e.get(&p.name).map(String::from).or_else(|| p.default.clone()).unwrap_or_default();
                    let mut value = current.clone();
                    property_widget(ui, (id.0, &p.name), p, &mut value);
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
                ui.label(RichText::new(k.as_str()).italics());
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
        ui.horizontal(|ui| {
            let key_id = egui::Id::new(("new-key", id.0));
            let mut key: String = ui.data_mut(|d| d.get_temp(key_id).unwrap_or_default());
            ui.add(egui::TextEdit::singleline(&mut key).hint_text("key").desired_width(110.0));
            if ui.add_enabled(!key.trim().is_empty(), egui::Button::new("Add key")).clicked() {
                e.set(key.trim(), "");
                key.clear();
            }
            ui.data_mut(|d| d.insert_temp(key_id, key));
        });
        if !is_world {
            ui.separator();
            ui.label(RichText::new("Outputs").strong());
            let outputs: Vec<String> = class.iter().flat_map(|c| c.outputs.iter().map(|o| o.name.clone())).collect();
            connections_editor(ui, id, &mut e.connections, &outputs);
        }
        if e != ent {
            let kind = if is_world { NodeKind::World(e) } else { NodeKind::Entity(e) };
            self.set_kind(id, kind, "Edit entity");
        }
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
