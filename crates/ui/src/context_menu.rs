//! The right click menu in the views: the tools that fit what is selected, as in Blender.

use egui::RichText;

use crate::app::MapCraftApp;
use crate::chrome::TEXT_WEAK;
use crate::icons;
use crate::viewport::Tool;

impl MapCraftApp {
    /// A menu entry for an action; engine commands are greyed out when they cannot run.
    fn menu_item(&mut self, ui: &mut egui::Ui, id: &str, label: &str) {
        let enabled = mapcraft_engine::find(id).is_none_or(|c| (c.enabled)(&self.session));
        self.action_button(ui, id, label, enabled);
    }

    /// Runs a UV command, first picking the selected brushes' faces when none are picked.
    fn uv_action(&mut self, id: &str) {
        if self.session.face_selection.is_empty() && !self.session.selection.is_empty() {
            self.run_action("tool.face");
        }
        self.run_action(id);
    }

    fn uv_menu(&mut self, ui: &mut egui::Ui, index: usize) {
        let can = !self.session.face_selection.is_empty() || !self.session.selected_brushes().is_empty();
        ui.menu_button((icons::slot(Some("unwrap")), "UV"), |ui| {
            for (id, label) in
                [("uv.unwrap", "Unwrap"), ("uv.project_from_view", "Project from View"), ("uv.cube_projection", "Cube Projection"), ("uv.reset", "Reset")]
            {
                let button = icons::button(icons::action_icon(id), label).shortcut_text(self.chord(id));
                if ui.add_enabled(can, button).clicked() {
                    self.uv_action(id);
                }
            }
            ui.separator();
            if ui.add_enabled(can, icons::button(None, "Edit UVs in This View")).clicked() {
                if self.session.face_selection.is_empty() {
                    self.run_action("tool.face");
                }
                if let Some(p) = self.panes.get_mut(index) {
                    p.uv = Some(crate::uv::UvCamera::default());
                }
            }
        });
    }

    fn face_menu(&mut self, ui: &mut egui::Ui, index: usize) {
        ui.label(RichText::new(format!("{} faces", self.session.face_selection.len())).color(TEXT_WEAK));
        self.menu_item(ui, "tools.apply_material", "Apply Current Material");
        self.menu_item(ui, "tools.clear_material", "Clear Material");
        if ui.add(icons::button(Some("ActionCopy"), "Copy Face").shortcut_text("Ctrl+C")).clicked() {
            self.copy_face();
        }
        if ui.add_enabled(self.picked_face.is_some(), icons::button(Some("clipboard"), "Paste Face").shortcut_text("Ctrl+V")).clicked() {
            self.paste_face();
        }
        self.uv_menu(ui, index);
        ui.separator();
        self.menu_item(ui, "tools.sweep", "Sweep Along Path");
        self.menu_item(ui, "tools.spin", "Spin");
    }

    fn vertex_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button((icons::slot(None), "Select Mode"), |ui| {
            self.menu_item(ui, "vertex.mode_vertex", "Vertex");
            self.menu_item(ui, "vertex.mode_edge", "Edge");
            self.menu_item(ui, "vertex.mode_face", "Face");
        });
        self.menu_item(ui, "vertex.merge", "Merge at Center");
        self.menu_item(ui, "vertex.bevel", "Bevel Edges");
        let delete = if self.vertex.mode == crate::vertex::HandleKind::Face { "Dissolve Faces" } else { "Delete Vertices" };
        self.menu_item(ui, "vertex.delete", delete);
        if !self.session.selected_brush_entities().is_empty() {
            self.menu_item(ui, "vertex.origin_to_corners", "Set Origin to Selected Corners");
        }
    }

    fn brush_type_menu(&mut self, ui: &mut egui::Ui) {
        let classes: Vec<&str> =
            ["func_detail", "func_illusionary", "func_wall"].into_iter().filter(|c| self.project.game.class(c).is_some_and(|d| d.kind.is_solid())).collect();
        let any = !self.session.selected_brushes().is_empty();
        ui.menu_button((icons::slot(None), "Brush Type"), |ui| {
            let mut pick = None;
            if ui.add_enabled(any, icons::button(Some("globe"), "World")).clicked() {
                pick = Some(None);
            }
            for c in classes {
                if ui.add_enabled(any, icons::button(Some("cube"), c)).clicked() {
                    pick = Some(Some(c));
                }
            }
            if let Some(class) = pick {
                self.make_brush_class(class);
            }
        });
    }

    /// Makes the selected brushes one entity of `class` with its defaults, or world brushes when `None`.
    pub(crate) fn make_brush_class(&mut self, class: Option<&str>) {
        let keys = class.and_then(|c| self.project.game.class(c)).map(|c| mapcraft_engine::class_defaults(&c)).unwrap_or_default();
        if let Err(e) = self.session.set_brush_class(class, keys) {
            self.status = e.to_string();
        }
        self.mark_changed();
    }

    /// Every brush class, those sharing a prefix such as `func_` or `trigger_` in a submenu of their own, as TrenchBroom lists them.
    fn brush_entity_menu(&mut self, ui: &mut egui::Ui) {
        let mut groups: std::collections::BTreeMap<String, Vec<(String, Option<egui::Color32>)>> = std::collections::BTreeMap::new();
        for c in self
            .project
            .game
            .class_names()
            .filter_map(|n| self.project.game.class(n))
            .filter(|c| c.kind.is_solid() && !c.name.eq_ignore_ascii_case("worldspawn"))
        {
            let prefix = c.name.split_once('_').map_or(String::new(), |(p, _)| p.to_string());
            let color = c.color().map(|[r, g, b]| egui::Color32::from_rgb(r, g, b));
            groups.entry(prefix).or_default().push((c.name, color));
        }
        if groups.is_empty() {
            return;
        }
        let any = !self.session.selected_brushes().is_empty();
        let mut pick = None;
        let item = |ui: &mut egui::Ui, name: &str, color: Option<egui::Color32>, pick: &mut Option<String>| {
            let (icon, tint) = icons::class_icon(true, color);
            if ui.add_enabled(any, egui::Button::new((icons::icon(icon).tint(tint), name))).clicked() {
                *pick = Some(name.to_string());
            }
        };
        ui.menu_button((icons::slot(Some("cube")), "Create Brush Entity"), |ui| {
            let (nested, single): (Vec<_>, Vec<_>) = groups.iter().partition(|(prefix, classes)| classes.len() > 1 && !prefix.is_empty());
            for (prefix, classes) in nested {
                let mut title = prefix.clone();
                if let Some(first) = title.get_mut(..1) {
                    first.make_ascii_uppercase();
                }
                ui.menu_button(title, |ui| classes.iter().for_each(|(n, c)| item(ui, n, *c, &mut pick)));
            }
            single.into_iter().flat_map(|(_, classes)| classes).for_each(|(n, c)| item(ui, n, *c, &mut pick));
        });
        if let Some(class) = pick {
            self.session.default_solid_entity = class.clone();
            self.make_brush_class(Some(&class));
        }
    }

    fn selection_menu(&mut self, ui: &mut egui::Ui, index: usize) {
        ui.label(RichText::new(format!("{} selected", self.session.selection.len())).color(TEXT_WEAK));
        self.menu_item(ui, "edit.duplicate", "Duplicate");
        self.menu_item(ui, "edit.duplicate_linked", "Duplicate Linked");
        self.menu_item(ui, "edit.delete", "Delete");
        ui.separator();
        self.menu_item(ui, "tools.apply_material", "Apply Current Material");
        self.menu_item(ui, "tools.clear_material", "Clear Material");
        ui.separator();
        self.menu_item(ui, "tools.hollow", "Hollow");
        self.menu_item(ui, "tools.carve", "Subtract (Carve)");
        ui.menu_button((icons::slot(Some("box_wireframe")), "Boolean"), |ui| {
            self.menu_item(ui, "tools.boolean_subtract", "Subtract");
            self.menu_item(ui, "tools.boolean_add", "Add");
            self.menu_item(ui, "tools.boolean_release", "Release from Boolean");
            self.menu_item(ui, "tools.boolean_apply", "Apply Boolean");
        });
        self.menu_item(ui, "tools.merge", "Merge Brushes");
        self.menu_item(ui, "tool.clip", "Clip");
        self.brush_type_menu(ui);
        self.brush_entity_menu(ui);
        ui.menu_button((icons::slot(Some("gimbal")), "Transform"), |ui| {
            self.menu_item(ui, "tools.rotate_cw", "Rotate 90° Clockwise");
            self.menu_item(ui, "tools.rotate_ccw", "Rotate 90° Counter-Clockwise");
            self.menu_item(ui, "tools.flip_x", "Flip X");
            self.menu_item(ui, "tools.flip_y", "Flip Y");
            self.menu_item(ui, "tools.flip_z", "Flip Z");
            self.menu_item(ui, "tools.snap_to_grid", "Snap to Grid");
            self.menu_item(ui, "tools.drop_to_floor", "Drop to Floor");
            ui.separator();
            self.menu_item(ui, "tools.transform", "Transform…");
            self.menu_item(ui, "tools.array", "Array…");
            self.menu_item(ui, "tools.randomize", "Randomize…");
        });
        self.uv_menu(ui, index);
        ui.menu_button((icons::slot(Some("folder")), "Organize"), |ui| {
            self.menu_item(ui, "tools.group", "Group");
            self.menu_item(ui, "tools.ungroup", "Ungroup");
            self.menu_item(ui, "tools.tie_to_entity", "Tie to Entity");
            self.menu_item(ui, "tools.move_to_world", "Move to World");
            self.menu_item(ui, "tools.center_origins", "Center Origins");
            self.menu_item(ui, "tools.mirror", "Make Mirror Group");
            self.menu_item(ui, "tools.zone_from_selection", "Zone from Selection");
        });
        ui.menu_button((icons::slot(Some("itinerary")), "Curves"), |ui| {
            self.menu_item(ui, "tools.array_path", "Array Along Path");
            self.menu_item(ui, "tools.curve_make_real", "Make Curve Real");
        });
        let one_copy =
            self.session.selection.len() == 1 && self.session.selection.iter().all(|id| mapcraft_map::link::copy_of(&self.session.map, *id).is_some());
        if one_copy {
            ui.separator();
            self.menu_item(ui, "tools.edit_link_here", "Edit Linked Copy Here");
            self.menu_item(ui, "tools.make_link_real", "Make Linked Copy Real");
        }
        ui.separator();
        self.menu_item(ui, "view.frame_selection", "Focus Selection");
        self.menu_item(ui, "view.hide_selected", "Hide");
        self.menu_item(ui, "view.isolate", "Isolate");
        self.menu_item(ui, "view.lock_selected", "Lock");
    }

    fn empty_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button((icons::slot(Some("plus")), "Add"), |ui| {
            let pick = self.shape_menu(ui);
            ui.separator();
            self.menu_item(ui, "tool.entity", "Entity");
            self.menu_item(ui, "tool.path", "Path");
            if let Some((shape, generator)) = pick {
                self.pick_block_shape(shape, generator);
            }
        });
        ui.separator();
        self.menu_item(ui, "edit.select_all", "Select All");
        self.menu_item(ui, "view.show_all", "Show All");
        self.menu_item(ui, "view.unlock_all", "Unlock All");
        ui.separator();
        self.menu_item(ui, "view.frame_all", "Frame All");
        self.menu_item(ui, "view.origin", "Go to Origin");
    }

    /// The right click menu for view `index`.
    pub(crate) fn viewport_menu(&mut self, ui: &mut egui::Ui, index: usize) {
        ui.set_min_width(220.0);
        if self.tool == Tool::Face && !self.session.face_selection.is_empty() {
            self.face_menu(ui, index);
            return;
        }
        if self.tool == Tool::Vertex {
            self.vertex_menu(ui);
            ui.separator();
        }
        if self.session.selection.is_empty() {
            self.empty_menu(ui);
        } else {
            self.selection_menu(ui, index);
        }
    }
}
