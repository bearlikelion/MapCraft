use egui::{Key, KeyboardShortcut, Modifiers};

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords() {
        assert_eq!(parse("Ctrl+Shift+Z"), Some(KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z)));
        assert_eq!(parse("Delete"), Some(KeyboardShortcut::new(Modifiers::NONE, Key::Delete)));
        assert_eq!(parse("]"), Some(KeyboardShortcut::new(Modifiers::NONE, Key::CloseBracket)));
        assert_eq!(parse("Ctrl+"), None);
        for c in mapcraft_engine::commands() {
            if let Some(s) = c.shortcut {
                assert!(parse(s).is_some(), "{} has an unparsable shortcut {s}", c.id);
            }
        }
    }
}
