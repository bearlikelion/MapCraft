use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::{AutoVisGroup, ClassDef, ClassKind, Diagnostic, Helper, Io, Item, Member, Property, parse};

const MAX_INCLUDE_DEPTH: usize = 32;
const MAX_BASE_DEPTH: usize = 64;

/// Every class from an FGD and its includes; later definitions replace earlier ones.
#[derive(Clone, Debug, Default)]
pub struct GameData {
    classes: BTreeMap<String, ClassDef>,
    order: Vec<String>,
    pub map_size: Option<(i64, i64)>,
    pub material_exclusions: Vec<String>,
    pub auto_visgroups: Vec<AutoVisGroup>,
    pub diagnostics: Vec<(PathBuf, Diagnostic)>,
}

/// A class with its bases merged in.
#[derive(Clone, Debug, PartialEq)]
pub struct EntityClass {
    pub name: String,
    pub kind: ClassKind,
    pub description: String,
    pub properties: Vec<Property>,
    pub inputs: Vec<Io>,
    pub outputs: Vec<Io>,
    /// Header helpers other than `base`, bases first.
    pub helpers: Vec<Helper>,
}

impl EntityClass {
    pub fn property(&self, name: &str) -> Option<&Property> {
        self.properties.iter().find(|p| p.name.eq_ignore_ascii_case(name))
    }

    fn helper(&self, names: &[&str]) -> Option<&Helper> {
        self.helpers.iter().rev().find(|h| names.iter().any(|n| h.name.eq_ignore_ascii_case(n)))
    }

    /// Editor box as (mins, maxs); a single size is centered on the origin.
    pub fn size(&self) -> Option<([f64; 3], [f64; 3])> {
        let args = self.helper(&["size"])?.args();
        let v: Vec<Vec<f64>> = args.iter().map(|a| Helper::numbers(a)).collect::<Option<_>>()?;
        match v.as_slice() {
            [a, b] if a.len() == 3 && b.len() == 3 => Some(([a[0], a[1], a[2]], [b[0], b[1], b[2]])),
            [a] if a.len() == 3 => Some(([-a[0] / 2.0, -a[1] / 2.0, -a[2] / 2.0], [a[0] / 2.0, a[1] / 2.0, a[2] / 2.0])),
            _ => None,
        }
    }

    pub fn color(&self) -> Option<[u8; 3]> {
        let args = self.helper(&["color"])?.args();
        match Helper::numbers(args.first()?)?.as_slice() {
            [r, g, b] => Some([r.clamp(0.0, 255.0) as u8, g.clamp(0.0, 255.0) as u8, b.clamp(0.0, 255.0) as u8]),
            _ => None,
        }
    }

    /// The preview model or sprite path when the class names a fixed one, as `studio("x")` or
    /// TrenchBroom's `model({ "path": "x" })`.
    pub fn model(&self) -> Option<String> {
        let h = self.helper(&["studio", "studioprop", "model", "sprite", "iconsprite"])?;
        let raw = h.raw.trim();
        let path = match raw.strip_prefix('{') {
            Some(obj) => obj.split_once("\"path\"")?.1.trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?.split_once('"')?.0,
            None => raw.strip_prefix('"')?.strip_suffix('"')?,
        };
        (!path.is_empty() && !path.contains('"')).then(|| path.to_string())
    }
}

impl GameData {
    pub fn parse_str(src: &str) -> Self {
        let mut gd = Self::default();
        gd.add_source(Path::new(""), src, Path::new(""), 0, &mut BTreeSet::new(), &mut |_| Err(std::io::ErrorKind::NotFound.into()));
        gd
    }

    /// Loads `path` and its includes through `read`, resolving an include next to the including
    /// file first and then next to `path`. Files are decoded as UTF-8 with invalid bytes replaced.
    pub fn load(path: &Path, read: &mut dyn FnMut(&Path) -> std::io::Result<Vec<u8>>) -> Self {
        let mut gd = Self::default();
        match read(path) {
            Ok(bytes) => {
                let root = path.parent().unwrap_or(Path::new("")).to_path_buf();
                gd.add_source(path, &String::from_utf8_lossy(&bytes), &root, 0, &mut BTreeSet::new(), read);
            }
            Err(e) => gd.diagnostics.push((path.to_path_buf(), Diagnostic { line: 0, col: 0, message: format!("cannot read: {e}") })),
        }
        gd
    }

    fn add_source(
        &mut self,
        path: &Path,
        src: &str,
        root: &Path,
        depth: usize,
        seen: &mut BTreeSet<PathBuf>,
        read: &mut dyn FnMut(&Path) -> std::io::Result<Vec<u8>>,
    ) {
        seen.insert(path.to_path_buf());
        let file = parse(src);
        self.diagnostics.extend(file.diagnostics.into_iter().map(|d| (path.to_path_buf(), d)));
        for item in file.items {
            match item {
                Item::Class(c) => {
                    let key = c.name.to_ascii_lowercase();
                    if !self.classes.contains_key(&key) {
                        self.order.push(key.clone());
                    }
                    self.classes.insert(key, c);
                }
                Item::Include(inc) => self.include(path, &inc, root, depth, seen, read),
                Item::MapSize(a, b) => self.map_size = Some((a, b)),
                Item::MaterialExclusion(dirs) => self.material_exclusions.extend(dirs),
                Item::AutoVisGroup(g) => self.auto_visgroups.push(g),
                Item::GridNav(_) => {}
            }
        }
    }

    fn include(
        &mut self,
        from: &Path,
        inc: &str,
        root: &Path,
        depth: usize,
        seen: &mut BTreeSet<PathBuf>,
        read: &mut dyn FnMut(&Path) -> std::io::Result<Vec<u8>>,
    ) {
        let diag = |message: String| (from.to_path_buf(), Diagnostic { line: 0, col: 0, message });
        if depth >= MAX_INCLUDE_DEPTH {
            self.diagnostics.push(diag(format!("@include \"{inc}\" nests deeper than {MAX_INCLUDE_DEPTH} files")));
            return;
        }
        let here = from.parent().unwrap_or(Path::new("")).join(inc);
        for candidate in [here, root.join(inc)] {
            if seen.contains(&candidate) {
                return;
            }
            if let Ok(bytes) = read(&candidate) {
                self.add_source(&candidate, &String::from_utf8_lossy(&bytes), root, depth + 1, seen, read);
                return;
            }
        }
        self.diagnostics.push(diag(format!("cannot find @include \"{inc}\"")));
    }

    /// Adds another file's classes and settings; its classes replace same-named ones.
    pub fn extend(&mut self, other: GameData) {
        for key in other.order {
            if let Some(c) = other.classes.get(&key) {
                if !self.classes.contains_key(&key) {
                    self.order.push(key.clone());
                }
                self.classes.insert(key, c.clone());
            }
        }
        self.map_size = other.map_size.or(self.map_size);
        self.material_exclusions.extend(other.material_exclusions);
        self.auto_visgroups.extend(other.auto_visgroups);
        self.diagnostics.extend(other.diagnostics);
    }

    /// Classes that can be placed in a map (everything except `@BaseClass`), in definition order.
    pub fn class_names(&self) -> impl Iterator<Item = &str> {
        self.order.iter().filter_map(|k| self.classes.get(k)).filter(|c| c.kind != ClassKind::Base).map(|c| c.name.as_str())
    }

    pub fn class(&self, name: &str) -> Option<EntityClass> {
        let def = self.classes.get(&name.to_ascii_lowercase())?;
        let mut out = EntityClass {
            name: def.name.clone(),
            kind: def.kind.clone(),
            description: def.description.clone(),
            properties: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            helpers: Vec::new(),
        };
        self.merge(def, &mut out, &mut BTreeSet::new(), 0);
        for p in out.properties.iter_mut().filter(|p| p.ty == "flags" && p.default.is_none()) {
            let bits = p.choices.iter().filter(|c| c.default_on == Some(true)).filter_map(|c| c.value.trim().parse::<u64>().ok()).fold(0, |a, b| a | b);
            if bits != 0 {
                p.default = Some(bits.to_string());
            }
        }
        Some(out)
    }

    fn merge(&self, def: &ClassDef, out: &mut EntityClass, visited: &mut BTreeSet<String>, depth: usize) {
        if depth > MAX_BASE_DEPTH || !visited.insert(def.name.to_ascii_lowercase()) {
            return;
        }
        for h in def.helpers.iter().filter(|h| h.name.eq_ignore_ascii_case("base")) {
            for base in h.args() {
                if let Some(b) = self.classes.get(&base.to_ascii_lowercase()) {
                    self.merge(b, out, visited, depth + 1);
                }
            }
        }
        out.helpers.extend(def.helpers.iter().filter(|h| !h.name.eq_ignore_ascii_case("base")).cloned());
        for m in &def.members {
            match m {
                Member::Property(p) => upsert(&mut out.properties, p.clone(), |a, b| a.name.eq_ignore_ascii_case(&b.name)),
                Member::Input(io) => upsert(&mut out.inputs, io.clone(), |a, b| a.name.eq_ignore_ascii_case(&b.name)),
                Member::Output(io) => upsert(&mut out.outputs, io.clone(), |a, b| a.name.eq_ignore_ascii_case(&b.name)),
            }
        }
    }
}

fn upsert<T>(list: &mut Vec<T>, item: T, same: impl Fn(&T, &T) -> bool) {
    match list.iter_mut().find(|x| same(x, &item)) {
        Some(slot) => *slot = item,
        None => list.push(item),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn includes_and_bases() {
        let files: HashMap<PathBuf, &str> = HashMap::from([
            (
                PathBuf::from("fgd/game.fgd"),
                "@include \"base.fgd\"\n@PointClass base(Targetname, Angles) size(32 32 64) color(0 255 0) studio(\"models/x.mdl\") = thing : \"A thing\" [ angles(angle) : \"Pitch Yaw Roll\" : \"0 90 0\" ]",
            ),
            (
                PathBuf::from("fgd/base.fgd"),
                "@include \"game.fgd\"\n@BaseClass = Targetname [ targetname(target_source) : \"Name\" output OnUser1(void) : \"\" ]\n@BaseClass = Angles [ angles(angle) : \"Angles\" : \"0 0 0\" ]",
            ),
        ]);
        let gd = GameData::load(Path::new("fgd/game.fgd"), &mut |p| files.get(p).map(|s| s.as_bytes().to_vec()).ok_or(std::io::ErrorKind::NotFound.into()));
        assert!(gd.diagnostics.is_empty(), "{:?}", gd.diagnostics);
        assert_eq!(gd.class_names().collect::<Vec<_>>(), ["thing"]);
        let c = gd.class("THING").unwrap();
        let names: Vec<&str> = c.properties.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["targetname", "angles"]);
        assert_eq!(c.property("angles").unwrap().default.as_deref(), Some("0 90 0"));
        assert_eq!(c.outputs.len(), 1);
        assert_eq!(c.size(), Some(([-16.0, -16.0, -32.0], [16.0, 16.0, 32.0])));
        assert_eq!(c.color(), Some([0, 255, 0]));
        assert_eq!(c.model().as_deref(), Some("models/x.mdl"));
    }

    #[test]
    fn cyclic_bases_and_missing_includes() {
        let gd = GameData::parse_str(
            "@include \"nope.fgd\"\n@BaseClass base(B) = A [ a(string) ]\n@BaseClass base(A) = B [ b(string) ]\n@PointClass base(A) = c []",
        );
        assert_eq!(gd.diagnostics.len(), 1);
        let c = gd.class("c").unwrap();
        assert_eq!(c.properties.len(), 2);
        assert!(gd.class("missing").is_none());
    }

    #[test]
    fn flags_default_to_their_default_on_bits() {
        let gd = GameData::parse_str("@PointClass = lamp [ spawnflags(flags) = [ 1 : \"Off\" : 1 2 : \"Quiet\" : 0 4 : \"Loud\" : 1 ] ]");
        let c = gd.class("lamp").unwrap();
        assert_eq!(c.property("spawnflags").unwrap().default.as_deref(), Some("5"));
    }

    #[test]
    fn model_helpers() {
        let gd = GameData::parse_str(
            "@PointClass model({ \"path\": \"props/crate.glb\", \"skin\": skin }) = tb []\n@PointClass model({ \"path\": model }) = keyed []\n@PointClass studio() = any []",
        );
        let model = |c: &str| gd.class(c).unwrap().model();
        assert_eq!(model("tb").as_deref(), Some("props/crate.glb"));
        assert_eq!(model("keyed"), None, "a path taken from a key is not fixed");
        assert_eq!(model("any"), None);
    }
}
