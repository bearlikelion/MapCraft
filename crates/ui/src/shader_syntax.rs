//! Godot shader coloring; this lexer does not claim to compile shaders.

use egui::{Color32, FontId, TextFormat, text::LayoutJob};

pub(crate) fn highlight(source: &str, font: FontId, text_color: Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    let mut at = 0;
    while let Some(rest) = source.get(at..).filter(|rest| !rest.is_empty()) {
        let Some(first) = rest.chars().next() else { break };
        let (length, color) = if rest.starts_with("//") {
            (rest.find('\n').unwrap_or(rest.len()), Color32::from_rgb(128, 154, 130))
        } else if rest.starts_with("/*") {
            (rest.get(2..).and_then(|s| s.find("*/")).map_or(rest.len(), |end| end + 4), Color32::from_rgb(128, 154, 130))
        } else if first == '"' {
            let mut end = first.len_utf8();
            let mut escaped = false;
            for (index, ch) in rest.char_indices().skip(1) {
                end = index + ch.len_utf8();
                if ch == '"' && !escaped {
                    break;
                }
                escaped = ch == '\\' && !escaped;
            }
            (end, Color32::from_rgb(220, 184, 130))
        } else if first.is_ascii_alphabetic() || first == '_' {
            let length =
                rest.char_indices().take_while(|(_, ch)| ch.is_ascii_alphanumeric() || *ch == '_').last().map_or(first.len_utf8(), |(i, ch)| i + ch.len_utf8());
            let word = rest.get(..length).unwrap_or_default();
            let color = if matches!(
                word,
                "shader_type"
                    | "render_mode"
                    | "uniform"
                    | "varying"
                    | "const"
                    | "void"
                    | "if"
                    | "else"
                    | "for"
                    | "while"
                    | "return"
                    | "discard"
                    | "group_uniforms"
                    | "true"
                    | "false"
            ) {
                Color32::from_rgb(214, 162, 230)
            } else if matches!(
                word,
                "float"
                    | "int"
                    | "uint"
                    | "bool"
                    | "vec2"
                    | "vec3"
                    | "vec4"
                    | "ivec2"
                    | "ivec3"
                    | "ivec4"
                    | "mat2"
                    | "mat3"
                    | "mat4"
                    | "sampler2D"
                    | "samplerCube"
            ) {
                Color32::from_rgb(125, 199, 218)
            } else if word.chars().all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit()) {
                Color32::from_rgb(235, 193, 125)
            } else {
                text_color
            };
            (length, color)
        } else if first.is_ascii_digit() {
            let length =
                rest.char_indices().take_while(|(_, ch)| ch.is_ascii_alphanumeric() || *ch == '.').last().map_or(first.len_utf8(), |(i, ch)| i + ch.len_utf8());
            (length, Color32::from_rgb(151, 201, 163))
        } else {
            (first.len_utf8(), text_color)
        };
        let Some(token) = rest.get(..length) else { break };
        job.append(token, 0.0, TextFormat::simple(font.clone(), color));
        at = at.saturating_add(length);
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coloring_preserves_unicode_and_unfinished_edits() {
        for source in ["// café 🌊\nALBEDO = vec3(1.0);", "/* unfinished λ", "uniform sampler2D image;\n#include \"shader.gdshaderinc\"", "\"unfinished 🌊", ""]
        {
            let job = highlight(source, FontId::monospace(14.0), Color32::WHITE);
            assert_eq!(job.text, source);
            for section in &job.sections {
                assert!(source.is_char_boundary(section.byte_range.start.into()));
                assert!(source.is_char_boundary(section.byte_range.end.into()));
            }
        }
    }
}
