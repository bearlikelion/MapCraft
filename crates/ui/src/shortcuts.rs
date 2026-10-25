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
                ("tools.move_to_world", "Ctrl+Shift+W"),
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
    ("edit.preferences", "Preferences", "Ctrl+Comma"),
    ("tools.transform", "Transform", "Ctrl+M"),
    ("tools.replace_materials", "Replace Materials", ""),
    ("file.sdk_settings", "SDK Settings", "Ctrl+Shift+Comma"),
    ("file.quit", "Quit", "Ctrl+Q"),
    ("help.shortcuts", "Keyboard Shortcuts", "F1"),
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
    ("view.colorize", "Color Brushes by Object", "Ctrl+Alt+C"),
    ("view.frame_all", "Frame All", "Home"),
    ("view.frame_selection", "Focus Selection", "F"),
    ("view.find_entities", "Find Entities", "Ctrl+F"),
    ("panel.scene", "Scene Panel", "Ctrl+1"),
    ("panel.tags", "Tags Panel", "Ctrl+2"),
    ("panel.materials", "Materials Panel", "Ctrl+3"),
    ("panel.entities", "Entities Panel", "Ctrl+4"),
    ("panel.output", "Output Panel", "Ctrl+5"),
    ("inspector.smart_edit", "Smart Edit", "Ctrl+Alt+E"),
    ("shape.next", "Next Block Shape", "Alt+B"),
    ("clip.next_side", "Next Clip Side", "Alt+C"),
    ("run.again", "Run Last Profile", "F9"),
    ("run.cancel", "Cancel Run", "Shift+F9"),
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
];

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

/// Parsed bindings, chords with more modifiers first so Ctrl+Shift+S is not taken by Ctrl+S.
pub fn bindings(preset: Preset, custom: &BTreeMap<String, String>) -> Vec<(KeyboardShortcut, &'static str)> {
    let mut out: Vec<_> = actions().filter_map(|(id, _)| Some((parse(&chord(id, preset, custom)?)?, id))).collect();
    out.sort_by_key(|(sc, _)| std::cmp::Reverse(sc.modifiers.shift as u8 + sc.modifiers.command as u8 + sc.modifiers.alt as u8));
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
        for preset in Preset::ALL {
            let mut seen: Vec<(KeyboardShortcut, &str)> = Vec::new();
            for (sc, id) in bindings(preset, &none) {
                if let Some((_, other)) = seen.iter().find(|(s, _)| *s == sc) {
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
