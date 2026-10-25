use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::{EngineError, SelectMode, Session};

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
            s.duplicate_selection(mapcraft_geom::DVec3::new(g, g, 0.0))
        },
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
        id: "tools.move_to_world",
        label: "Move to World",
        menu: "Tools/Move to World",
        shortcut: Some("Ctrl+Shift+W"),
        enabled: has_selection,
        run: Session::move_to_world,
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
}
