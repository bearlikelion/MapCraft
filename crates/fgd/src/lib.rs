//! FGD entity definitions: a lenient parser for the Valve and TrenchBroom dialects and a resolver
//! for `@include` and `base(...)`.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod gamedata;
mod lexer;
mod parser;

pub use gamedata::{EntityClass, GameData};
pub use parser::parse;

/// One parsed file, in source order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FgdFile {
    pub items: Vec<Item>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Class(ClassDef),
    Include(String),
    MapSize(i64, i64),
    MaterialExclusion(Vec<String>),
    AutoVisGroup(AutoVisGroup),
    GridNav(Vec<f64>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClassKind {
    Base,
    Point,
    Solid,
    Npc,
    Filter,
    Move,
    KeyFrame,
    Other(String),
}

impl ClassKind {
    fn from_section(s: &str) -> Option<Self> {
        let lower = s.to_ascii_lowercase();
        let kind = match lower.as_str() {
            "baseclass" => Self::Base,
            "pointclass" => Self::Point,
            "solidclass" => Self::Solid,
            "npcclass" => Self::Npc,
            "filterclass" => Self::Filter,
            "moveclass" => Self::Move,
            "keyframeclass" => Self::KeyFrame,
            _ if lower.ends_with("class") => Self::Other(s.to_string()),
            _ => return None,
        };
        Some(kind)
    }

    /// Point-like classes are placed at an origin; solid classes own brushes.
    pub fn is_solid(&self) -> bool {
        *self == Self::Solid
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClassDef {
    pub kind: ClassKind,
    pub name: String,
    pub description: String,
    pub helpers: Vec<Helper>,
    pub members: Vec<Member>,
    pub line: u32,
}

/// `name(args)` in a class header; `raw` is the exact text between the parentheses.
#[derive(Clone, Debug, PartialEq)]
pub struct Helper {
    pub name: String,
    pub raw: String,
}

impl Helper {
    /// Arguments split on top-level commas, trimmed, with surrounding quotes removed.
    pub fn args(&self) -> Vec<String> {
        let (mut out, mut cur, mut depth, mut quoted) = (Vec::new(), String::new(), 0i32, false);
        for c in self.raw.chars() {
            match c {
                '"' => quoted = !quoted,
                '(' | '{' | '[' if !quoted => depth += 1,
                ')' | '}' | ']' if !quoted => depth -= 1,
                ',' if !quoted && depth == 0 => {
                    out.push(unquote(&cur));
                    cur.clear();
                    continue;
                }
                _ => {}
            }
            cur.push(c);
        }
        if !cur.trim().is_empty() || !out.is_empty() {
            out.push(unquote(&cur));
        }
        out
    }

    /// Whitespace-separated numbers of one argument, e.g. `255 128 0` or `-8 -8 -8`.
    pub fn numbers(arg: &str) -> Option<Vec<f64>> {
        arg.split_whitespace().map(|t| t.parse::<f64>().ok().filter(|v| v.is_finite())).collect()
    }
}

fn unquote(s: &str) -> String {
    let t = s.trim();
    t.strip_prefix('"').and_then(|t| t.strip_suffix('"')).unwrap_or(t).to_string()
}

#[derive(Clone, Debug, PartialEq)]
pub enum Member {
    Property(Property),
    Input(Io),
    Output(Io),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Property {
    pub name: String,
    /// Lowercase type such as `string`, `choices`, `flags` or `target_destination`.
    pub ty: String,
    pub readonly: bool,
    pub report: bool,
    pub display: String,
    pub default: Option<String>,
    pub description: String,
    pub choices: Vec<Choice>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub value: String,
    pub label: String,
    /// Flags only: whether the bit is set by default.
    pub default_on: Option<bool>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Io {
    pub name: String,
    pub ty: String,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AutoVisGroup {
    pub name: String,
    pub children: Vec<(String, Vec<String>)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    pub line: u32,
    pub col: u32,
    pub message: String,
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.col, self.message)
    }
}
