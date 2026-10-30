use std::collections::BTreeMap;

use egui::{Key, KeyboardShortcut, Modifiers};
use serde::{Deserialize, Serialize};

/// Parses craft-keymap style chord text such as `Ctrl+Shift+Z`, `Delete` or `[`.
pub fn parse(text: &str) -> Option<KeyboardShortcut> {
    let mut mods = Modifiers::NONE;
    let mut key = None;
    for part in text.split('+').map(str::trim) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "cmd" => mods = mods.plus(Modifiers::COMMAND),
            "shift" => mods = mods.plus(Modifiers::SHIFT),
            "alt" => mods = mods.plus(Modifiers::ALT),
            _ => {
                key = match part {
                    "[" => Some(Key::OpenBracket),
                    "]" => Some(Key::CloseBracket),
                    other => Key::from_name(other),
                }
            }
        }
    }
    Some(KeyboardShortcut::new(mods, key?))
}

/// Starting shortcut sets for people coming from other editors.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Preset {
    #[default]
    MapCraft,
    Hammer,
    TrenchBroom,
}

impl Preset {
    pub const ALL: [Preset; 3] = [Preset::MapCraft, Preset::Hammer, Preset::TrenchBroom];

    pub fn label(self) -> &'static str {
        match self {
            Preset::MapCraft => "MapCraft",
            Preset::Hammer => "Hammer",
            Preset::TrenchBroom => "TrenchBroom",
        }
    }

    /// Chords that differ from the defaults (Godot-style transform keys on Q W E R T); an empty
    /// chord unbinds.
    fn overrides(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Preset::MapCraft => &[],
            Preset::Hammer => &[
                ("tool.select", "Shift+S"),
                ("tool.block", "Shift+B"),
                ("tool.entity", "Shift+E"),
                ("tool.clip", "Shift+X"),
                ("tool.vertex", "Shift+V"),
                ("tool.face", "Shift+A"),
            ],
            Preset::TrenchBroom => &[
                ("tool.entity", ""),
                ("tools.carve", "Ctrl+K"),
                ("tools.hollow", "Ctrl+Shift+K"),
                ("tools.ungroup", "Ctrl+Shift+G"),
                ("tools.flip_y", ""),
                ("tools.tie_to_entity", ""),
                ("view.isolate", "Ctrl+I"),
                ("view.hide_selected", "Ctrl+Alt+I"),
                ("view.show_all", "Ctrl+Shift+I"),
                ("view.grid_smaller", "Minus"),
                ("view.grid_larger", "Plus"),
                ("tools.roll_cw", "Alt+Up"),
                ("tools.roll_ccw", "Alt+Down"),
                ("tools.pitch_cw", "Alt+PageUp"),
                ("tools.pitch_ccw", "Alt+PageDown"),
                ("tools.yaw_cw", "Alt+Left"),
                ("tools.yaw_ccw", "Alt+Right"),
            ],
        }
    }
}

/// Actions the editor shell runs itself: id, label, default chord.
pub const UI_ACTIONS: &[(&str, &str, &str)] = &[
    ("file.new", "New Map", "Ctrl+N"),
    ("file.open", "Open", "Ctrl+O"),
    ("file.save", "Save", "Ctrl+S"),
    ("file.save_as", "Save As", "Ctrl+Shift+S"),
    ("file.revert", "Revert", ""),
    ("file.import_gltf", "Import glTF", ""),
    ("file.export_gltf", "Export glTF", ""),
    ("file.reload_assets", "Reload Assets", "F5"),
    ("edit.search", "Search Commands", "F3"),
    ("edit.preferences", "Preferences", "Ctrl+Comma"),
    ("tools.transform", "Transform", "Ctrl+M"),
    ("tools.array", "Array", "Ctrl+Shift+D"),
    ("tools.randomize", "Randomize Transform", ""),
    ("tools.replace_materials", "Replace Materials", ""),
    ("tools.roll_cw", "Roll 90° Clockwise", ""),
    ("tools.roll_ccw", "Roll 90° Counter-Clockwise", ""),
    ("tools.pitch_cw", "Pitch 90° Clockwise", ""),
    ("tools.pitch_ccw", "Pitch 90° Counter-Clockwise", ""),
    ("tools.yaw_cw", "Yaw 90° Clockwise", ""),
    ("tools.yaw_ccw", "Yaw 90° Counter-Clockwise", ""),
    ("file.sdk_settings", "SDK Settings", "Ctrl+Shift+Comma"),
    ("file.switch_sdk", "Switch SDK", ""),
    ("file.sdk_manager", "SDK Manager", ""),
    ("file.quit", "Quit", "Ctrl+Q"),
    ("help.shortcuts", "Keyboard Shortcuts", "F1"),
    ("help.about", "About MapCraft", ""),
    ("tools.check_problems", "Check for Problems", "Alt+P"),
    ("prefab.save", "Save Selection as Prefab", "Ctrl+Shift+P"),
    ("prefab.insert", "Insert Prefab", "Ctrl+Alt+P"),
    ("prefab.make_local", "Make Prefab Local", "Ctrl+Shift+U"),
    ("prefab.reload", "Reload Prefabs", "Ctrl+Alt+R"),
    ("layer.new", "New Layer", "Ctrl+Shift+N"),
    ("view.maximize", "Maximize View", "Space"),
    ("view.link_zoom", "Link 2D Zoom", "Shift+Z"),
    ("view.render_cycle", "Next Render Mode", "Z"),
    ("view.render_textured", "Textured", "Ctrl+Alt+1"),
    ("view.render_flat", "Flat Shaded", "Ctrl+Alt+2"),
    ("view.render_wireframe", "Wireframe", "Ctrl+Alt+3"),
    ("view.render_lit", "Lit", "Ctrl+Alt+4"),
    ("view.sky", "Show Sky", ""),
    ("view.colorize", "Color Brushes by Object", "Ctrl+Alt+C"),
    ("view.slope", "Color by Slope", "Ctrl+Alt+5"),
    ("view.frame_all", "Frame All", "Home"),
    ("view.origin", "Go to Origin", "Shift+Home"),
    ("view.single", "One View Layout", "Shift+Space"),
    ("view.frame_selection", "Focus Selection", "F"),
    ("view.find_entities", "Find Entities", "Ctrl+F"),
    ("panel.scene", "Scene Panel", "Ctrl+1"),
    ("panel.scene_search", "Search Scene", "Ctrl+Shift+F"),
    ("panel.tags", "Tags Panel", "Ctrl+2"),
    ("panel.materials", "Materials Panel", "Ctrl+3"),
    ("panel.entities", "Entities Panel", "Ctrl+4"),
    ("panel.models", "Models Panel", "Ctrl+6"),
    ("panel.output", "Output Panel", "Ctrl+5"),
    ("inspector.smart_edit", "Smart Edit", "Ctrl+Alt+E"),
    ("shape.next", "Next Block Shape", "Alt+B"),
    ("clip.next_side", "Next Clip Side", "Ctrl+Enter"),
    ("clip.remove_point", "Remove Last Clip Point", "Delete"),
    ("run.again", "Run Last Profile", "F9"),
    ("run.cancel", "Cancel Run", "Shift+F9"),
    ("run.load_pointfile", "Load Leak Trail", ""),
    ("run.unload_pointfile", "Unload Leak Trail", ""),
    ("run.point_next", "Next Leak Point", "Period"),
    ("run.point_prev", "Previous Leak Point", "Comma"),
    ("view.snap_grid", "Snap to Grid", "Shift+W"),
    ("view.snap_vertices", "Snap to Corners", "Shift+Q"),
    ("tool.select", "Select Tool", "Q"),
    ("tool.move", "Move Tool", "W"),
    ("tool.extrude", "Extrude Tool", "E"),
    ("tool.rotate", "Rotate Tool", "R"),
    ("tool.scale", "Scale Tool", "T"),
    ("tool.block", "Block Tool", "B"),
    ("tool.entity", "Entity Tool", "N"),
    ("tool.clip", "Clip Tool", "C"),
    ("tool.vertex", "Vertex Tool", "V"),
    ("tool.face", "Face Tool", "Shift+F"),
    ("tool.sculpt", "Sculpt Tool", "D"),
    ("tool.path", "Path Tool", "P"),
    ("sculpt.raise", "Sculpt: Raise", "1"),
    ("sculpt.lower", "Sculpt: Lower", "2"),
    ("sculpt.smooth", "Sculpt: Smooth", "3"),
    ("sculpt.flatten", "Sculpt: Flatten", "4"),
    ("sculpt.noise", "Sculpt: Noise", "5"),
    ("sculpt.paint", "Sculpt: Paint", "6"),
    ("sculpt.blur", "Sculpt: Blur", "7"),
    ("vertex.mode_vertex", "Vertex Select Mode", "1"),
    ("vertex.mode_edge", "Edge Select Mode", "2"),
    ("vertex.mode_face", "Face Select Mode", "3"),
    ("vertex.merge", "Merge Vertices", "M"),
    ("vertex.bevel", "Bevel Edges", "Ctrl+B"),
    ("vertex.delete", "Delete Vertices or Dissolve Faces", "Delete"),
    ("vertex.origin_to_corners", "Set Origin to Selected Corners", ""),
    ("uv.grab", "UV Grab", "G"),
    ("uv.rotate", "UV Rotate", "R"),
    ("uv.scale", "UV Scale", "S"),
    ("uv.fit", "Fit UV View", "F"),
];

/// Actions whose keys work only over a UV view, where they win over other actions' keys.
pub fn uv_only(id: &str) -> bool {
    id.starts_with("uv.")
}

/// Actions whose keys work only in the Vertex tool, where they win over other actions' keys.
pub fn vertex_only(id: &str) -> bool {
    id.starts_with("vertex.")
}

/// Actions whose keys work only in the Clip tool, where they win over other actions' keys.
pub fn clip_only(id: &str) -> bool {
    id == "clip.remove_point"
}

/// The chord bound to `id`: the user's own, else the preset's, else the default.
pub fn chord(id: &str, preset: Preset, custom: &BTreeMap<String, String>) -> Option<String> {
    let text = custom
        .get(id)
        .map(String::as_str)
        .or_else(|| preset.overrides().iter().find(|(i, _)| *i == id).map(|(_, c)| *c))
        .or_else(|| UI_ACTIONS.iter().find(|a| a.0 == id).map(|a| a.2))
        .or_else(|| mapcraft_engine::find(id).and_then(|c| c.shortcut))?;
    (!text.is_empty()).then(|| text.to_string())
}

/// Every bindable action as (id, label).
pub fn actions() -> impl Iterator<Item = (&'static str, &'static str)> {
    UI_ACTIONS.iter().map(|a| (a.0, a.1)).chain(mapcraft_engine::commands().iter().map(|c| (c.id, c.label)))
}

/// Parsed bindings, chords with more modifiers first so Ctrl+Shift+S is not taken by Ctrl+S, and
/// UV view, Vertex tool and Clip tool actions before others on the same chord.
pub fn bindings(preset: Preset, custom: &BTreeMap<String, String>) -> Vec<(KeyboardShortcut, &'static str)> {
    let mut out: Vec<_> = actions().filter_map(|(id, _)| Some((parse(&chord(id, preset, custom)?)?, id))).collect();
    out.sort_by_key(|(sc, id)| {
        (std::cmp::Reverse(sc.modifiers.shift as u8 + sc.modifiers.command as u8 + sc.modifiers.alt as u8), !(uv_only(id) || vertex_only(id) || clip_only(id)))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords() {
        assert_eq!(parse("Ctrl+Shift+Z"), Some(KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z)));
        assert_eq!(parse("Delete"), Some(KeyboardShortcut::new(Modifiers::NONE, Key::Delete)));
        assert_eq!(parse("]"), Some(KeyboardShortcut::new(Modifiers::NONE, Key::CloseBracket)));
        assert_eq!(parse("Ctrl+"), None);
        let none = BTreeMap::new();
        for preset in Preset::ALL {
            for (id, _) in actions() {
                if let Some(c) = chord(id, preset, &none) {
                    assert!(parse(&c).is_some(), "{id} has an unparsable chord {c} in {preset:?}");
                }
            }
        }
        assert_eq!(chord("tools.carve", Preset::TrenchBroom, &none).as_deref(), Some("Ctrl+K"));
        assert_eq!(chord("tools.yaw_cw", Preset::TrenchBroom, &none).as_deref(), Some("Alt+Left"));
        assert_eq!(chord("tools.yaw_cw", Preset::MapCraft, &none), None);
        for preset in Preset::ALL {
            let mut seen: Vec<(KeyboardShortcut, &str)> = Vec::new();
            for (sc, id) in bindings(preset, &none) {
                let scope = |a: &str| (uv_only(a), vertex_only(a), clip_only(a));
                if let Some((_, other)) = seen.iter().find(|(s, other)| *s == sc && scope(other) == scope(id)) {
                    panic!("{preset:?}: {id} and {other} share {sc:?}");
                }
                seen.push((sc, id));
            }
        }
        assert_eq!(chord("tool.entity", Preset::TrenchBroom, &none), None);
        let custom = BTreeMap::from([("tools.carve".to_string(), "Ctrl+Alt+C".to_string())]);
        assert_eq!(chord("tools.carve", Preset::TrenchBroom, &custom).as_deref(), Some("Ctrl+Alt+C"));
    }
}
