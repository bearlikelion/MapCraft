//! Reads Godot's text resource format (`.tscn`, `.tres`): `[section]` headers with attributes,
//! each followed by `key = value` lines.

/// Nested values deeper than this are refused, so hostile files cannot exhaust the stack.
const MAX_DEPTH: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Nil,
    Bool(bool),
    Num(f64),
    /// Strings, `&"names"` and `^"paths"`.
    Str(String),
    /// A constructor such as `Vector3(1, 2, 3)` or `SubResource("id")`.
    Call(String, Vec<Value>),
    List(Vec<Value>),
    Dict(Vec<(Value, Value)>),
}

impl Value {
    pub fn num(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            Value::Bool(b) => Some(f64::from(u8::from(*b))),
            _ => None,
        }
    }

    pub fn text(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            Value::Call(_, args) => args.first().and_then(Value::text),
            _ => None,
        }
    }

    /// The arguments of a constructor named `name`.
    pub fn call(&self, name: &str) -> Option<&[Value]> {
        match self {
            Value::Call(n, args) if n == name => Some(args),
            _ => None,
        }
    }

    /// Every number inside a constructor or list, in order, such as a packed array's contents.
    pub fn numbers(&self) -> Vec<f64> {
        let mut out = Vec::new();
        self.collect_numbers(&mut out, 0);
        out
    }

    fn collect_numbers(&self, out: &mut Vec<f64>, depth: usize) {
        match self {
            Value::Num(n) => out.push(*n),
            Value::Call(_, items) | Value::List(items) if depth < MAX_DEPTH => items.iter().for_each(|v| v.collect_numbers(out, depth + 1)),
            _ => {}
        }
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Dict(entries) => entries.iter().find(|(k, _)| k.text() == Some(key)).map(|(_, v)| v),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Section {
    pub tag: String,
    pub attrs: Vec<(String, Value)>,
    pub props: Vec<(String, Value)>,
}

impl Section {
    pub fn attr(&self, key: &str) -> Option<&Value> {
        self.attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn attr_text(&self, key: &str) -> Option<&str> {
        self.attr(key).and_then(Value::text)
    }

    pub fn prop(&self, key: &str) -> Option<&Value> {
        self.props.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
}

pub fn parse(text: &str) -> Result<Vec<Section>, String> {
    let mut p = Parser { s: text.as_bytes(), i: 0 };
    let mut sections: Vec<Section> = Vec::new();
    loop {
        p.skip_space();
        let Some(c) = p.peek() else { break };
        if c == b'[' {
            p.i += 1;
            sections.push(p.header()?);
        } else {
            let key = p.key()?;
            let value = p.value(0)?;
            match sections.last_mut() {
                Some(s) => s.props.push((key, value)),
                None => return Err(format!("{} before any section", key)),
            }
        }
    }
    Ok(sections)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn line(&self) -> usize {
        self.s.get(..self.i).map_or(0, |b| b.iter().filter(|c| **c == b'\n').count()) + 1
    }

    fn error(&self, what: &str) -> String {
        format!("line {}: {what}", self.line())
    }

    /// Skips white space and `;` comments.
    fn skip_space(&mut self) {
        while let Some(c) = self.peek() {
            if c == b';' {
                while self.peek().is_some_and(|c| c != b'\n') {
                    self.i += 1;
                }
            } else if c.is_ascii_whitespace() {
                self.i += 1;
            } else {
                break;
            }
        }
    }

    fn expect(&mut self, c: u8) -> Result<(), String> {
        self.skip_space();
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            Err(self.error(&format!("expected `{}`", c as char)))
        }
    }

    fn ident(&mut self) -> String {
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_') {
            self.i += 1;
        }
        String::from_utf8_lossy(self.s.get(start..self.i).unwrap_or_default()).into_owned()
    }

    fn header(&mut self) -> Result<Section, String> {
        self.skip_space();
        let tag = self.ident();
        let mut attrs = Vec::new();
        loop {
            self.skip_space();
            match self.peek() {
                Some(b']') => {
                    self.i += 1;
                    return Ok(Section { tag, attrs, props: Vec::new() });
                }
                Some(_) => {
                    let key = self.ident();
                    if key.is_empty() {
                        return Err(self.error("bad section header"));
                    }
                    self.expect(b'=')?;
                    attrs.push((key, self.value(0)?));
                }
                None => return Err(self.error("unclosed section header")),
            }
        }
    }

    /// A property name, which may hold `/` and other punctuation, up to its `=`.
    fn key(&mut self) -> Result<String, String> {
        let start = self.i;
        while self.peek().is_some_and(|c| c != b'=' && c != b'\n') {
            self.i += 1;
        }
        if self.peek() != Some(b'=') {
            return Err(self.error("expected `key = value`"));
        }
        let key = String::from_utf8_lossy(self.s.get(start..self.i).unwrap_or_default()).trim().trim_matches('"').to_string();
        self.i += 1;
        Ok(key)
    }

    fn value(&mut self, depth: usize) -> Result<Value, String> {
        if depth > MAX_DEPTH {
            return Err(self.error("values nest too deeply"));
        }
        self.skip_space();
        match self.peek() {
            Some(b'"') => Ok(Value::Str(self.string()?)),
            Some(b'&' | b'^') => {
                self.i += 1;
                Ok(Value::Str(self.string()?))
            }
            Some(b'[') => {
                self.i += 1;
                Ok(Value::List(self.items(b']', depth)?))
            }
            Some(b'{') => {
                self.i += 1;
                let mut entries = Vec::new();
                loop {
                    self.skip_space();
                    if self.peek() == Some(b'}') {
                        self.i += 1;
                        return Ok(Value::Dict(entries));
                    }
                    let key = self.value(depth + 1)?;
                    self.expect(b':')?;
                    entries.push((key, self.value(depth + 1)?));
                    self.skip_space();
                    if self.peek() == Some(b',') {
                        self.i += 1;
                    }
                }
            }
            Some(c) if c == b'-' || c == b'+' || c == b'.' || c.is_ascii_digit() => self.number(),
            Some(c) if c.is_ascii_alphabetic() || c == b'_' => {
                let name = self.ident();
                match name.as_str() {
                    "true" => return Ok(Value::Bool(true)),
                    "false" => return Ok(Value::Bool(false)),
                    "null" => return Ok(Value::Nil),
                    "inf" => return Ok(Value::Num(f64::INFINITY)),
                    "nan" => return Ok(Value::Num(f64::NAN)),
                    _ => {}
                }
                self.skip_space();
                // Typed arrays and dictionaries name their element types in brackets first.
                if self.peek() == Some(b'[') {
                    self.skip_brackets()?;
                    self.skip_space();
                }
                if self.peek() != Some(b'(') {
                    return Err(self.error(&format!("unknown value `{name}`")));
                }
                self.i += 1;
                Ok(Value::Call(name, self.items(b')', depth)?))
            }
            _ => Err(self.error("expected a value")),
        }
    }

    fn items(&mut self, close: u8, depth: usize) -> Result<Vec<Value>, String> {
        let mut items = Vec::new();
        loop {
            self.skip_space();
            if self.peek() == Some(close) {
                self.i += 1;
                return Ok(items);
            }
            items.push(self.value(depth + 1)?);
            self.skip_space();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(c) if c == close => {}
                _ => return Err(self.error(&format!("expected `,` or `{}`", close as char))),
            }
        }
    }

    fn skip_brackets(&mut self) -> Result<(), String> {
        let mut level = 0usize;
        while let Some(c) = self.peek() {
            self.i += 1;
            match c {
                b'[' => level += 1,
                b']' => {
                    level = level.saturating_sub(1);
                    if level == 0 {
                        return Ok(());
                    }
                }
                _ => {}
            }
        }
        Err(self.error("unclosed `[`"))
    }

    fn number(&mut self) -> Result<Value, String> {
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'+' | b'.')) {
            self.i += 1;
        }
        let text = std::str::from_utf8(self.s.get(start..self.i).unwrap_or_default()).unwrap_or_default();
        match text {
            "-inf" => Ok(Value::Num(f64::NEG_INFINITY)),
            _ => text.parse::<f64>().map(Value::Num).map_err(|_| self.error(&format!("bad number `{text}`"))),
        }
    }

    fn string(&mut self) -> Result<String, String> {
        if self.peek() != Some(b'"') {
            return Err(self.error("expected a string"));
        }
        self.i += 1;
        let mut out = Vec::new();
        while let Some(c) = self.peek() {
            self.i += 1;
            match c {
                b'"' => return Ok(String::from_utf8_lossy(&out).into_owned()),
                b'\\' => {
                    let e = self.peek().unwrap_or(b'\\');
                    self.i += 1;
                    out.push(match e {
                        b'n' => b'\n',
                        b't' => b'\t',
                        b'r' => b'\r',
                        other => other,
                    });
                }
                _ => out.push(c),
            }
        }
        Err(self.error("unclosed string"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_sections_and_values() {
        let text = r#"[gd_scene format=3 uid="uid://x"]

[ext_resource type="PackedScene" path="res://a.tscn" id="kill"]

[sub_resource type="Shader" id="sh"]
code = "shader_type spatial;
void f() { ALBEDO = vec3(\"q\"); }
"

[sub_resource type="Curve3D" id="c"]
_data = {
"points": PackedVector3Array(0, 0, 0, 0, 0, 0, 1, -2.5e-1, 3),
"tilts": PackedFloat32Array(0)
}
point_count = 1

[node name="Root" type="Node3D"]
transform = Transform3D(1, 0, 0, 0, 1, 0, 0, 0, 1, 4, 5, 6)
metadata/sdk_zone = "start"
metadata/tags = Array[StringName]([&"a", &"b"])
bus = &"Music"
path_node = NodePath("../P")

[node name="Kill" parent="." instance=ExtResource("kill")]
"#;
        let s = parse(text).unwrap();
        assert_eq!(s.len(), 6);
        assert_eq!(s[1].attr_text("path"), Some("res://a.tscn"));
        assert!(s[2].prop("code").unwrap().text().unwrap().contains("vec3(\"q\")"), "strings keep escaped quotes and new lines");
        let points = s[3].prop("_data").unwrap().get("points").unwrap().numbers();
        assert_eq!(points, [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, -0.25, 3.0]);
        let root = &s[4];
        assert_eq!(root.prop("transform").unwrap().numbers().len(), 12);
        assert_eq!(root.prop("metadata/sdk_zone").unwrap().text(), Some("start"));
        assert_eq!(root.prop("bus").unwrap().text(), Some("Music"));
        assert_eq!(root.prop("path_node").unwrap().text(), Some("../P"));
        assert_eq!(s[5].attr("instance").unwrap().call("ExtResource").unwrap()[0].text(), Some("kill"));
    }

    #[test]
    fn hostile_input_is_an_error() {
        assert!(parse("[node name=\"a\"").is_err());
        assert!(parse("key = 1").is_err(), "a property needs a section");
        assert!(parse(&format!("[a]\nk = {}", "[".repeat(500))).is_err(), "deep nesting is refused");
        assert!(parse("[a]\nk = \"open").is_err());
    }
}
