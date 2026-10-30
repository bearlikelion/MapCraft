use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::{BlendFill, EngineError, SelectMode, Session};

/// A user-facing action that menus, shortcuts and the CLI dispatch by id.
pub struct CommandSpec {
    pub id: &'static str,
    pub label: &'static str,
    /// Menu path such as `Edit/Undo`.
    pub menu: &'static str,
    /// Default chord in craft-keymap text form.
    pub shortcut: Option<&'static str>,
    pub enabled: fn(&Session) -> bool,
    pub run: fn(&mut Session) -> Result<(), EngineError>,
}

fn always(_: &Session) -> bool {
    true
}

fn has_selection(s: &Session) -> bool {
    !s.selection.is_empty()
}

fn has_faces(s: &Session) -> bool {
    !s.face_selection.is_empty()
}

fn in_boolean(s: &Session) -> bool {
    s.selection.iter().any(|id| mapcraft_map::boolean::group_of(&s.map, *id).is_some())
}

fn has_boolean(s: &Session) -> bool {
    in_boolean(s) || s.selection.iter().any(|id| crate::boolean::is_flagged(&s.map, *id))
}

fn has_link_copy(s: &Session) -> bool {
    s.selection.iter().any(|id| mapcraft_map::link::copy_of(&s.map, *id).is_some())
}

fn one_link_copy(s: &Session) -> bool {
    s.selection.len() == 1 && has_link_copy(s)
}

const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "edit.undo",
        label: "Undo",
        menu: "Edit/Undo",
        shortcut: Some("Ctrl+Z"),
        enabled: Session::can_undo,
        run: |s| {
            s.undo();
            Ok(())
        },
    },
    CommandSpec {
        id: "edit.redo",
        label: "Redo",
        menu: "Edit/Redo",
        shortcut: Some("Ctrl+Shift+Z"),
        enabled: Session::can_redo,
        run: |s| {
            s.redo();
            Ok(())
        },
    },
    CommandSpec { id: "edit.delete", label: "Delete", menu: "Edit/Delete", shortcut: Some("Delete"), enabled: has_selection, run: Session::delete_selection },
    CommandSpec {
        id: "edit.duplicate",
        label: "Duplicate",
        menu: "Edit/Duplicate",
        shortcut: Some("Ctrl+D"),
        enabled: has_selection,
        run: |s| {
            let g = s.grid;
            s.duplicate_selection(mapcraft_geom::DVec3::new(g, g, 0.0), None)
        },
    },
    CommandSpec {
        id: "edit.duplicate_linked",
        label: "Duplicate Linked",
        menu: "Edit/Duplicate Linked",
        shortcut: Some("Alt+D"),
        enabled: has_selection,
        run: |s| {
            let g = s.grid;
            s.duplicate_linked(mapcraft_geom::DVec3::new(g, g, 0.0)).map(|_| ())
        },
    },
    CommandSpec {
        id: "tools.edit_link_here",
        label: "Edit Linked Copy Here",
        menu: "Tools/Edit Linked Copy Here",
        shortcut: None,
        enabled: one_link_copy,
        run: |s| s.edit_link_here().map(|_| ()),
    },
    CommandSpec {
        id: "tools.make_link_real",
        label: "Make Linked Copy Real",
        menu: "Tools/Make Linked Copy Real",
        shortcut: None,
        enabled: has_link_copy,
        run: Session::make_link_real,
    },
    CommandSpec {
        id: "edit.select_all",
        label: "Select All",
        menu: "Edit/Select All",
        shortcut: Some("Ctrl+A"),
        enabled: always,
        run: |s| {
            s.select_all();
            Ok(())
        },
    },
    CommandSpec {
        id: "edit.select_none",
        label: "Select None",
        menu: "Edit/Select None",
        shortcut: Some("Ctrl+Shift+A"),
        enabled: has_selection,
        run: |s| {
            s.select_none();
            Ok(())
        },
    },
    CommandSpec {
        id: "edit.select_faces_by_material",
        label: "Select Faces by Material",
        menu: "Edit/Select by Material/Faces Using Current Material",
        shortcut: None,
        enabled: always,
        run: |s| {
            let m = s.tex.material.clone();
            match s.select_faces_with_material(&m) {
                0 => Err(EngineError::Refused("no face a click can reach uses the current material")),
                _ => Ok(()),
            }
        },
    },
    CommandSpec {
        id: "edit.select_brushes_by_material",
        label: "Select Brushes by Material",
        menu: "Edit/Select by Material/Brushes Using Current Material",
        shortcut: None,
        enabled: always,
        run: |s| {
            let m = s.tex.material.clone();
            match s.select_brushes_with_material(&m) {
                0 => Err(EngineError::Refused("no brush a click can reach uses the current material")),
                _ => Ok(()),
            }
        },
    },
    CommandSpec {
        id: "edit.select_groups",
        label: "Select Groups",
        menu: "Edit/Selection Mode/Groups",
        shortcut: Some("Alt+1"),
        enabled: always,
        run: |s| {
            s.select_mode = SelectMode::Groups;
            Ok(())
        },
    },
    CommandSpec {
        id: "edit.select_objects",
        label: "Select Objects",
        menu: "Edit/Selection Mode/Objects",
        shortcut: Some("Alt+2"),
        enabled: always,
        run: |s| {
            s.select_mode = SelectMode::Objects;
            Ok(())
        },
    },
    CommandSpec {
        id: "edit.select_solids",
        label: "Select Solids",
        menu: "Edit/Selection Mode/Solids",
        shortcut: Some("Alt+3"),
        enabled: always,
        run: |s| {
            s.select_mode = SelectMode::Solids;
            Ok(())
        },
    },
    CommandSpec { id: "tools.carve", label: "Carve", menu: "Tools/Carve", shortcut: Some("Ctrl+Shift+C"), enabled: has_selection, run: Session::carve },
    CommandSpec {
        id: "tools.boolean_subtract",
        label: "Subtract",
        menu: "Tools/Boolean/Subtract",
        shortcut: Some("Ctrl+Shift+B"),
        enabled: has_selection,
        run: |s| s.make_boolean(mapcraft_map::Operation::Subtract).map(|_| ()),
    },
    CommandSpec {
        id: "tools.boolean_add",
        label: "Add",
        menu: "Tools/Boolean/Add",
        shortcut: Some("Ctrl+Alt+B"),
        enabled: has_selection,
        run: |s| s.make_boolean(mapcraft_map::Operation::Add).map(|_| ()),
    },
    CommandSpec {
        id: "tools.boolean_release",
        label: "Release from Boolean",
        menu: "Tools/Boolean/Release",
        shortcut: None,
        enabled: in_boolean,
        run: Session::release_from_boolean,
    },
    CommandSpec {
        id: "tools.boolean_apply",
        label: "Apply Boolean",
        menu: "Tools/Boolean/Apply",
        shortcut: None,
        enabled: has_boolean,
        run: Session::apply_boolean,
    },
    CommandSpec {
        id: "tools.hollow",
        label: "Hollow",
        menu: "Tools/Hollow",
        shortcut: Some("Ctrl+H"),
        enabled: has_selection,
        run: |s| {
            let t = s.grid.max(1.0);
            s.hollow(t)
        },
    },
    CommandSpec {
        id: "tools.zone_from_selection",
        label: "Zone from Selection",
        menu: "Tools/Zone from Selection",
        shortcut: None,
        enabled: has_selection,
        run: |s| {
            let class = s.default_solid_entity.clone();
            s.zone_from_selection(&class, 256.0).map(|_| ())
        },
    },
    CommandSpec {
        id: "tools.snap_to_grid",
        label: "Snap Selection to Grid",
        menu: "Tools/Snap Selection to Grid",
        shortcut: Some("Ctrl+B"),
        enabled: has_selection,
        run: Session::snap_selection_to_grid,
    },
    CommandSpec {
        id: "tools.group",
        label: "Group",
        menu: "Tools/Group",
        shortcut: Some("Ctrl+G"),
        enabled: has_selection,
        run: |s| s.group_selection().map(|_| ()),
    },
    CommandSpec {
        id: "tools.ungroup",
        label: "Ungroup",
        menu: "Tools/Ungroup",
        shortcut: Some("Ctrl+U"),
        enabled: has_selection,
        run: Session::ungroup_selection,
    },
    CommandSpec {
        id: "tools.mirror",
        label: "Make Mirror Group",
        menu: "Tools/Make Mirror Group",
        shortcut: Some("Ctrl+Shift+M"),
        enabled: has_selection,
        run: |s| s.make_mirror_group().map(|_| ()),
    },
    CommandSpec {
        id: "tools.mirror_make_real",
        label: "Make Mirror Real",
        menu: "Tools/Make Mirror Real",
        shortcut: None,
        enabled: |s| s.selection.iter().any(|id| mapcraft_map::mirror::is_mirror_group(&s.map, *id)),
        run: Session::make_mirror_real,
    },
    CommandSpec {
        id: "tools.sweep",
        label: "Sweep Along Path",
        menu: "Tools/Curves/Sweep Along Path",
        shortcut: Some("Ctrl+Alt+S"),
        enabled: has_faces,
        run: |s| s.sweep_faces().map(|_| ()),
    },
    CommandSpec {
        id: "tools.spin",
        label: "Spin",
        menu: "Tools/Curves/Spin",
        shortcut: Some("Ctrl+Alt+N"),
        enabled: has_faces,
        run: |s| {
            let pivot = s.selection_center().unwrap_or_default();
            s.spin_faces(pivot).map(|_| ())
        },
    },
    CommandSpec {
        id: "tools.array_path",
        label: "Array Along Path",
        menu: "Tools/Curves/Array Along Path",
        shortcut: Some("Ctrl+Alt+D"),
        enabled: has_selection,
        run: |s| s.array_along_path().map(|_| ()),
    },
    CommandSpec {
        id: "tools.curve_make_real",
        label: "Make Curve Real",
        menu: "Tools/Curves/Make Real",
        shortcut: None,
        enabled: |s| s.selection.iter().any(|id| s.map.get(*id).is_some_and(|n| crate::curves::curve_of(&n.kind).is_some())),
        run: Session::make_curve_real,
    },
    CommandSpec {
        id: "tools.tie_to_entity",
        label: "Tie to Entity",
        menu: "Tools/Tie to Entity",
        shortcut: Some("Ctrl+T"),
        enabled: has_selection,
        run: |s| {
            let class = s.default_solid_entity.clone();
            s.tie_to_entity(&class).map(|_| ())
        },
    },
    CommandSpec {
        id: "tools.drop_to_floor",
        label: "Drop to Floor",
        menu: "Tools/Drop to Floor",
        shortcut: Some("End"),
        enabled: has_selection,
        run: |s| s.drop_to_floor(&|_| None).map(|_| ()),
    },
    CommandSpec {
        id: "tools.move_to_world",
        label: "Move to World",
        menu: "Tools/Move to World",
        shortcut: Some("Ctrl+Shift+W"),
        enabled: has_selection,
        run: Session::move_to_world,
    },
    CommandSpec {
        id: "tools.center_origins",
        label: "Center Origins",
        menu: "Tools/Center Origins",
        shortcut: None,
        enabled: |s| !s.selected_brush_entities().is_empty(),
        run: |s| {
            let ids = s.selected_brush_entities();
            s.set_origins(&ids, None, None)
        },
    },
    CommandSpec {
        id: "tools.rotate_cw",
        label: "Rotate 90° Clockwise",
        menu: "Tools/Rotate 90° Clockwise",
        shortcut: Some("Ctrl+R"),
        enabled: has_selection,
        run: |s| s.rotate_selection(mapcraft_geom::DVec3::Z, -90.0),
    },
    CommandSpec {
        id: "tools.rotate_ccw",
        label: "Rotate 90° Counter-Clockwise",
        menu: "Tools/Rotate 90° Counter-Clockwise",
        shortcut: Some("Ctrl+Shift+R"),
        enabled: has_selection,
        run: |s| s.rotate_selection(mapcraft_geom::DVec3::Z, 90.0),
    },
    CommandSpec { id: "tools.flip_x", label: "Flip X", menu: "Tools/Flip X", shortcut: Some("Ctrl+L"), enabled: has_selection, run: |s| s.flip_selection(0) },
    CommandSpec { id: "tools.flip_y", label: "Flip Y", menu: "Tools/Flip Y", shortcut: Some("Ctrl+I"), enabled: has_selection, run: |s| s.flip_selection(1) },
    CommandSpec {
        id: "tools.flip_z",
        label: "Flip Z",
        menu: "Tools/Flip Z",
        shortcut: Some("Ctrl+Alt+L"),
        enabled: has_selection,
        run: |s| s.flip_selection(2),
    },
    CommandSpec {
        id: "tools.merge",
        label: "Merge Brushes",
        menu: "Tools/Merge Brushes",
        shortcut: Some("Ctrl+J"),
        enabled: has_selection,
        run: |s| s.merge_selection().map(|_| ()),
    },
    CommandSpec {
        id: "tools.apply_material",
        label: "Apply Current Material",
        menu: "Tools/Apply Current Material",
        shortcut: Some("Shift+T"),
        enabled: |s| has_selection(s) || !s.face_selection.is_empty(),
        run: Session::apply_material,
    },
    CommandSpec {
        id: "tools.clear_material",
        label: "Clear Material",
        menu: "Tools/Clear Material",
        shortcut: None,
        enabled: |s| has_selection(s) || !s.face_selection.is_empty(),
        run: Session::clear_material,
    },
    CommandSpec {
        id: "disp.blend_use_current",
        label: "Blend Current Material",
        menu: "Tools/Displacement/Blend Current Material",
        shortcut: None,
        enabled: Session::has_picked_displacement,
        run: |s| {
            let current = s.tex.material.clone();
            if current.is_empty() {
                return Err(EngineError::Refused("pick a material to blend in the Materials panel first"));
            }
            s.set_blend_material(Some(&current)).map(|_| ())
        },
    },
    CommandSpec {
        id: "disp.blend_remove",
        label: "Remove Blend Material",
        menu: "Tools/Displacement/Remove Blend Material",
        shortcut: None,
        enabled: Session::has_picked_displacement,
        run: |s| s.set_blend_material(None).map(|_| ()),
    },
    CommandSpec {
        id: "disp.blend_fill",
        label: "Fill Blend",
        menu: "Tools/Displacement/Fill Blend",
        shortcut: None,
        enabled: Session::has_picked_displacement,
        run: |s| s.fill_blend(BlendFill::Value(1.0)).map(|_| ()),
    },
    CommandSpec {
        id: "disp.blend_invert",
        label: "Invert Blend",
        menu: "Tools/Displacement/Invert Blend",
        shortcut: None,
        enabled: Session::has_picked_displacement,
        run: |s| s.fill_blend(BlendFill::Invert).map(|_| ()),
    },
    CommandSpec {
        id: "disp.blend_clear",
        label: "Clear Blend",
        menu: "Tools/Displacement/Clear Blend",
        shortcut: None,
        enabled: Session::has_picked_displacement,
        run: |s| s.fill_blend(BlendFill::Value(0.0)).map(|_| ()),
    },
    CommandSpec { id: "uv.unwrap", label: "Unwrap", menu: "Tools/UV/Unwrap", shortcut: Some("U"), enabled: has_faces, run: Session::unwrap_faces },
    CommandSpec {
        id: "uv.project_from_view",
        label: "Project from View",
        menu: "Tools/UV/Project from View",
        shortcut: None,
        enabled: has_faces,
        run: |s| s.project_faces(s.view_dir),
    },
    CommandSpec {
        id: "uv.cube_projection",
        label: "Cube Projection",
        menu: "Tools/UV/Cube Projection",
        shortcut: None,
        enabled: has_faces,
        run: |s| s.edit_faces("Cube projection", None, mapcraft_map::Face::align_to_world),
    },
    CommandSpec { id: "uv.reset", label: "Reset UVs", menu: "Tools/UV/Reset", shortcut: None, enabled: has_faces, run: Session::reset_uvs },
    CommandSpec {
        id: "tools.toggle_texture_lock",
        label: "Texture Lock",
        menu: "Tools/Texture Lock",
        shortcut: Some("Shift+L"),
        enabled: always,
        run: |s| {
            s.texture_lock = !s.texture_lock;
            Ok(())
        },
    },
    CommandSpec {
        id: "view.lock_selected",
        label: "Lock Selected",
        menu: "View/Lock Selected",
        shortcut: Some("Ctrl+Alt+K"),
        enabled: has_selection,
        run: |s| {
            let ids: Vec<_> = s.selection.iter().copied().collect();
            s.set_locked(&ids, true)
        },
    },
    CommandSpec {
        id: "view.unlock_all",
        label: "Unlock All",
        menu: "View/Unlock All",
        shortcut: Some("Ctrl+Alt+Shift+K"),
        enabled: |_| true,
        run: Session::unlock_all,
    },
    CommandSpec {
        id: "view.hide_selected",
        label: "Hide Selected",
        menu: "View/Hide Selected",
        shortcut: Some("H"),
        enabled: has_selection,
        run: |s| {
            let ids: Vec<_> = s.selection.iter().copied().collect();
            s.set_hidden(&ids, true)
        },
    },
    CommandSpec {
        id: "view.isolate",
        label: "Isolate Selected",
        menu: "View/Isolate Selected",
        shortcut: Some("I"),
        enabled: has_selection,
        run: Session::isolate_selection,
    },
    CommandSpec { id: "view.show_all", label: "Show All", menu: "View/Show All", shortcut: Some("U"), enabled: always, run: Session::show_all },
    CommandSpec {
        id: "view.grid_smaller",
        label: "Smaller Grid",
        menu: "View/Smaller Grid",
        shortcut: Some("["),
        enabled: always,
        run: |s| {
            s.grid = (s.grid / 2.0).max(0.125);
            Ok(())
        },
    },
    CommandSpec {
        id: "view.grid_larger",
        label: "Larger Grid",
        menu: "View/Larger Grid",
        shortcut: Some("]"),
        enabled: always,
        run: |s| {
            s.grid = (s.grid * 2.0).min(4096.0);
            Ok(())
        },
    },
];

pub fn commands() -> &'static [CommandSpec] {
    COMMANDS
}

pub fn find(id: &str) -> Option<&'static CommandSpec> {
    COMMANDS.iter().find(|c| c.id == id)
}

/// Runs a command by id; a panic inside it becomes an error and the map is kept.
pub fn execute(session: &mut Session, id: &str) -> Result<(), EngineError> {
    let spec = find(id).ok_or_else(|| EngineError::UnknownCommand(id.to_string()))?;
    if !(spec.enabled)(session) {
        return Err(EngineError::Disabled(spec.id));
    }
    catch_unwind(AssertUnwindSafe(|| (spec.run)(session))).unwrap_or(Err(EngineError::Panic(spec.id)))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use mapcraft_geom::{Aabb, DVec3};

    use super::*;

    #[test]
    fn registry() {
        let mut ids: Vec<&str> = commands().iter().map(|c| c.id).collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n, "command ids must be unique");

        let mut s = Session::default();
        assert!(matches!(execute(&mut s, "edit.undo"), Err(EngineError::Disabled(_))));
        assert!(matches!(execute(&mut s, "nope"), Err(EngineError::UnknownCommand(_))));
        let b = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
        execute(&mut s, "edit.duplicate").unwrap();
        assert_eq!(s.map.len(), 3);
        execute(&mut s, "edit.undo").unwrap();
        execute(&mut s, "view.grid_larger").unwrap();
        assert_eq!(s.grid, 32.0);
        s.selection = BTreeSet::from([b]);
        execute(&mut s, "tools.tie_to_entity").unwrap();
        assert_eq!(s.map.get(s.map.get(b).unwrap().parent.unwrap()).unwrap().kind.entity().unwrap().classname, "func_detail");
    }

    #[test]
    fn select_by_material_and_drop_to_floor() {
        let mut s = Session::default();
        s.tex.material = "dev/floor".into();
        s.create_brush(Aabb::new(DVec3::new(-256.0, -256.0, -16.0), DVec3::new(256.0, 256.0, 0.0))).unwrap();
        s.tex.material = "dev/crate".into();
        let c = s.create_brush(Aabb::new(DVec3::new(0.0, 0.0, 64.0), DVec3::splat(96.0))).unwrap();
        execute(&mut s, "edit.select_brushes_by_material").unwrap();
        assert_eq!(s.selection, BTreeSet::from([c]));
        execute(&mut s, "tools.drop_to_floor").unwrap();
        assert_eq!(s.map.get(c).unwrap().kind.brush().unwrap().bounds().unwrap().min.z, 0.0);
        execute(&mut s, "edit.select_faces_by_material").unwrap();
        assert_eq!(s.face_selection.len(), 6);
        s.tex.material = "unused".into();
        assert!(matches!(execute(&mut s, "edit.select_faces_by_material"), Err(EngineError::Refused(_))));
    }
}
