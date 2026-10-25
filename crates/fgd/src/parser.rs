use crate::lexer::{Tok, Token, lex};
use crate::{AutoVisGroup, Choice, ClassDef, ClassKind, Diagnostic, FgdFile, Helper, Io, Item, Member, Property};

type PResult<T> = Result<T, String>;

struct Parser<'a> {
    src: &'a str,
    toks: Vec<Token>,
    pos: usize,
    diagnostics: Vec<Diagnostic>,
}

/// Parses FGD source; problems become diagnostics and the parser resumes at the next `@`.
pub fn parse(src: &str) -> FgdFile {
    let mut p = Parser { src, toks: lex(src), pos: 0, diagnostics: Vec::new() };
    let mut items = Vec::new();
    while p.pos < p.toks.len() {
        let start = p.pos;
        match p.item() {
            Ok(Some(item)) => items.push(item),
            Ok(None) => {}
            Err(message) => {
                p.diagnostic(message);
                p.pos = p.pos.max(start + 1);
                p.skip_to_at();
            }
        }
    }
    FgdFile { items, diagnostics: p.diagnostics }
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|t| &t.tok)
    }

    fn peek_at(&self, n: usize) -> Option<&Tok> {
        self.toks.get(self.pos + n).map(|t| &t.tok)
    }

    fn diagnostic(&mut self, message: String) {
        let (line, col) = self.toks.get(self.pos).or(self.toks.last()).map_or((0, 0), |t| (t.line, t.col));
        self.diagnostics.push(Diagnostic { line, col, message });
    }

    fn skip_to_at(&mut self) {
        while self.peek().is_some_and(|t| *t != Tok::At) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, want: Tok, what: &str) -> PResult<()> {
        match self.peek() {
            Some(t) if *t == want => {
                self.pos += 1;
                Ok(())
            }
            other => Err(format!("expected {what}, found {}", describe(other))),
        }
    }

    fn word(&mut self, what: &str) -> PResult<String> {
        match self.peek() {
            Some(Tok::Word(w)) => {
                let w = w.clone();
                self.pos += 1;
                Ok(w)
            }
            other => Err(format!("expected {what}, found {}", describe(other))),
        }
    }

    /// A quoted string, joining `"a" + "b"`; a bare word is accepted where FGDs are sloppy.
    fn text(&mut self, what: &str) -> PResult<String> {
        let mut out = match self.peek() {
            Some(Tok::Str(s) | Tok::Word(s)) => s.clone(),
            other => return Err(format!("expected {what}, found {}", describe(other))),
        };
        self.pos += 1;
        while self.peek() == Some(&Tok::Plus) {
            self.pos += 1;
            match self.peek() {
                Some(Tok::Str(s)) => out.push_str(s),
                other => return Err(format!("expected a string after '+', found {}", describe(other))),
            }
            self.pos += 1;
        }
        Ok(out)
    }

    fn item(&mut self) -> PResult<Option<Item>> {
        self.expect(Tok::At, "'@'")?;
        let line = self.toks.get(self.pos).map_or(0, |t| t.line);
        let section = self.word("a section name after '@'")?;
        let item = match section.to_ascii_lowercase().as_str() {
            "include" => Item::Include(self.text("an include path")?),
            "mapsize" => {
                let n = self.number_list()?;
                match n.as_slice() {
                    [a, b] => Item::MapSize(*a as i64, *b as i64),
                    _ => return Err("@mapsize needs two numbers".into()),
                }
            }
            "gridnav" => Item::GridNav(self.number_list()?),
            "materialexclusion" => {
                self.expect(Tok::LBracket, "'['")?;
                let mut dirs = Vec::new();
                while self.peek().is_some_and(|t| *t != Tok::RBracket) {
                    dirs.push(self.text("a material folder")?);
                }
                self.expect(Tok::RBracket, "']'")?;
                Item::MaterialExclusion(dirs)
            }
            "autovisgroup" => Item::AutoVisGroup(self.auto_visgroup()?),
            _ => match ClassKind::from_section(&section) {
                Some(kind) => Item::Class(self.class(kind, line)?),
                None => {
                    self.diagnostic(format!("unknown section @{section} skipped"));
                    self.skip_to_at();
                    return Ok(None);
                }
            },
        };
        Ok(Some(item))
    }

    fn number_list(&mut self) -> PResult<Vec<f64>> {
        self.expect(Tok::LParen, "'('")?;
        let mut out = Vec::new();
        while self.peek().is_some_and(|t| *t != Tok::RParen) {
            if self.peek() == Some(&Tok::Comma) {
                self.pos += 1;
                continue;
            }
            let w = self.word("a number")?;
            out.push(w.parse::<f64>().ok().filter(|v| v.is_finite()).ok_or_else(|| format!("`{w}` is not a number"))?);
        }
        self.expect(Tok::RParen, "')'")?;
        Ok(out)
    }

    fn auto_visgroup(&mut self) -> PResult<AutoVisGroup> {
        self.expect(Tok::Eq, "'='")?;
        let name = self.text("a visgroup name")?;
        self.expect(Tok::LBracket, "'['")?;
        let mut children = Vec::new();
        while self.peek().is_some_and(|t| *t != Tok::RBracket) {
            let child = self.text("a visgroup name")?;
            self.expect(Tok::LBracket, "'['")?;
            let mut classes = Vec::new();
            while self.peek().is_some_and(|t| *t != Tok::RBracket) {
                classes.push(self.text("a class name")?);
            }
            self.expect(Tok::RBracket, "']'")?;
            children.push((child, classes));
        }
        self.expect(Tok::RBracket, "']'")?;
        Ok(AutoVisGroup { name, children })
    }

    fn helper_raw(&mut self) -> PResult<String> {
        let open = self.toks.get(self.pos).map(|t| t.end).ok_or("expected '('")?;
        self.pos += 1;
        let mut depth = 1usize;
        while let Some(t) = self.toks.get(self.pos) {
            self.pos += 1;
            match t.tok {
                Tok::LParen => depth += 1,
                Tok::RParen => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(self.src.get(open..t.start).unwrap_or_default().trim().to_string());
                    }
                }
                _ => {}
            }
        }
        Err("unclosed '(' in class header".into())
    }

    fn class(&mut self, kind: ClassKind, line: u32) -> PResult<ClassDef> {
        let mut helpers = Vec::new();
        loop {
            match self.peek() {
                Some(Tok::Eq) => break,
                Some(Tok::Word(_)) => {
                    let name = self.word("a helper")?;
                    let raw = if self.peek() == Some(&Tok::LParen) { self.helper_raw()? } else { String::new() };
                    helpers.push(Helper { name, raw });
                }
                other => return Err(format!("expected a helper or '=', found {}", describe(other))),
            }
        }
        self.pos += 1;
        let name = self.word("a class name")?;
        let description = if self.peek() == Some(&Tok::Colon) {
            self.pos += 1;
            self.text("a class description")?
        } else {
            String::new()
        };
        let members = if self.peek() == Some(&Tok::LBracket) { self.members()? } else { Vec::new() };
        Ok(ClassDef { kind, name, description, helpers, members, line })
    }

    fn members(&mut self) -> PResult<Vec<Member>> {
        self.expect(Tok::LBracket, "'['")?;
        let mut out = Vec::new();
        loop {
            match self.peek() {
                Some(Tok::RBracket) => break,
                None => return Err("unterminated class body".into()),
                _ => {}
            }
            let word = self.word("a property, input or output")?;
            let lower = word.to_ascii_lowercase();
            if (lower == "input" || lower == "output") && matches!(self.peek(), Some(Tok::Word(_))) {
                let name = self.word("an input or output name")?;
                let ty = self.paren_type()?;
                let description = self.colon_text()?.unwrap_or_default();
                let io = Io { name, ty, description };
                out.push(if lower == "input" { Member::Input(io) } else { Member::Output(io) });
                continue;
            }
            out.push(Member::Property(self.property(word)?));
        }
        self.pos += 1;
        Ok(out)
    }

    fn paren_type(&mut self) -> PResult<String> {
        self.expect(Tok::LParen, "'('")?;
        let ty = if let Some(Tok::Word(w)) = self.peek() {
            let w = w.to_ascii_lowercase();
            self.pos += 1;
            w
        } else {
            String::new()
        };
        self.expect(Tok::RParen, "')'")?;
        Ok(ty)
    }

    fn colon_text(&mut self) -> PResult<Option<String>> {
        if self.peek() != Some(&Tok::Colon) {
            return Ok(None);
        }
        self.pos += 1;
        match self.peek() {
            Some(Tok::Str(_)) => self.text("text").map(Some),
            _ => Ok(Some(String::new())),
        }
    }

    fn property(&mut self, name: String) -> PResult<Property> {
        let ty = self.paren_type()?;
        let mut p = Property { name, ty, ..Property::default() };
        while let Some(Tok::Word(w)) = self.peek() {
            match w.to_ascii_lowercase().as_str() {
                "readonly" => p.readonly = true,
                "report" => p.report = true,
                "*" => {}
                _ => break,
            }
            self.pos += 1;
        }
        if let Some(display) = self.colon_text()? {
            p.display = display;
            if self.peek() == Some(&Tok::Colon) {
                self.pos += 1;
                p.default = match self.peek() {
                    Some(Tok::Str(_)) => Some(self.text("a default")?),
                    // A word followed by '(' is the next property, not a default.
                    Some(Tok::Word(_)) if self.peek_at(1) != Some(&Tok::LParen) => Some(self.word("a default")?),
                    _ => None,
                };
                if let Some(description) = self.colon_text()? {
                    p.description = description;
                }
            }
        }
        if self.peek() == Some(&Tok::Eq) {
            self.pos += 1;
            p.choices = self.choices()?;
        }
        Ok(p)
    }

    fn choices(&mut self) -> PResult<Vec<Choice>> {
        self.expect(Tok::LBracket, "'['")?;
        let mut out = Vec::new();
        while self.peek().is_some_and(|t| *t != Tok::RBracket) {
            let value = self.text("a choice value")?;
            self.expect(Tok::Colon, "':' after a choice value")?;
            let label = self.text("a choice label")?;
            let mut default_on = None;
            if self.peek() == Some(&Tok::Colon) {
                self.pos += 1;
                match self.peek() {
                    Some(Tok::Word(w)) => {
                        default_on = Some(w.trim() != "0");
                        self.pos += 1;
                        self.colon_text()?;
                    }
                    Some(Tok::Str(_)) => {
                        self.text("a choice description")?;
                    }
                    _ => {}
                }
            }
            out.push(Choice { value, label, default_on });
        }
        self.expect(Tok::RBracket, "']'")?;
        Ok(out)
    }
}

fn describe(t: Option<&Tok>) -> String {
    match t {
        None => "the end of the file".into(),
        Some(Tok::Str(s)) => format!("\"{s}\""),
        Some(Tok::Word(w)) => format!("`{w}`"),
        Some(t) => format!("{t:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
// A comment
@mapsize(-16384, 16384)
@include "base.fgd"
@BaseClass = Targetname
[
	targetname(target_source) : "Name" : : "The name that other entities refer to this entity by."
	input Kill(void) : "Removes this entity from the world."
	output OnUser1(void) : "Fired in response to " +
		"FireUser1 input."
]
@PointClass base(Targetname) studio("models/editor/playerstart.mdl") size(-16 -16 0, 16 16 72) color(0 255 0) halfgridsnap
	= info_player_start : "Player spawn"
[
	spawnflags(flags) =
	[
		1 : "Master" : 0
		2 : "Other" : 1 : "Flag with a description"
	]
	skin(choices) readonly : "Skin" : 0 : "Skin to use" =
	[
		0 : "Default"
		1 : "Red"
	]
	speed(float) : "Speed" : "1.5"
	empty(string) :
	next(integer) : "Next"
]
@SolidClass = zone_start : "Start zone" []
@AutoVisGroup = "Brushes"
[
	"Triggers" [ "trigger_once" "trigger_multiple" ]
]
@MaterialExclusion [ "debug" "editor" ]
@PointClass model({ "path": "models/a.mdl", "skin": skin }) = tb_model []
"#;

    #[test]
    fn sample() {
        let f = parse(SAMPLE);
        assert!(f.diagnostics.is_empty(), "{:?}", f.diagnostics);
        assert_eq!(f.items.len(), 8);
        assert_eq!(f.items[0], Item::MapSize(-16384, 16384));
        assert_eq!(f.items[1], Item::Include("base.fgd".into()));
        let Item::Class(base) = &f.items[2] else { panic!() };
        assert_eq!(base.kind, ClassKind::Base);
        assert_eq!(base.members.len(), 3);
        let Member::Output(o) = &base.members[2] else { panic!() };
        assert_eq!(o.description, "Fired in response to FireUser1 input.");
        let Member::Property(t) = &base.members[0] else { panic!() };
        assert_eq!((t.ty.as_str(), t.default.as_deref()), ("target_source", None));

        let Item::Class(start) = &f.items[3] else { panic!() };
        assert_eq!(start.name, "info_player_start");
        let names: Vec<&str> = start.helpers.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(names, ["base", "studio", "size", "color", "halfgridsnap"]);
        assert_eq!(start.helpers[2].args(), ["-16 -16 0", "16 16 72"]);
        assert_eq!(start.helpers[1].args(), ["models/editor/playerstart.mdl"]);
        let Member::Property(flags) = &start.members[0] else { panic!() };
        assert_eq!(flags.choices[1].default_on, Some(true));
        let Member::Property(skin) = &start.members[1] else { panic!() };
        assert!(skin.readonly);
        assert_eq!(skin.choices.len(), 2);
        assert_eq!(skin.default.as_deref(), Some("0"));
        let Member::Property(empty) = &start.members[3] else { panic!() };
        assert_eq!(empty.default, None);
        assert_eq!(start.members.len(), 5);

        let Item::Class(tb) = &f.items[7] else { panic!() };
        assert_eq!(tb.helpers[0].raw, r#"{ "path": "models/a.mdl", "skin": skin }"#);
    }

    #[test]
    fn recovers_from_errors() {
        let f = parse("@PointClass = broken [ key( : ]\n@Bogus stuff\n@PointClass = fine []\n\"stray\"");
        assert_eq!(f.items.len(), 1);
        assert!(f.diagnostics.len() >= 2);
        let Item::Class(c) = &f.items[0] else { panic!() };
        assert_eq!(c.name, "fine");
        // Truncated input must not hang or panic.
        for cut in 0..SAMPLE.len() {
            if SAMPLE.is_char_boundary(cut) {
                let _ = parse(&SAMPLE[..cut]);
            }
        }
    }
}
