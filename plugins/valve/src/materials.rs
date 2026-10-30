//! Finds the pictures of a map's materials in its embedded pakfile and in the game's VPK packages.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};

use crate::{lzma, u16_at, u32_at, vtf};

/// Game files the plugin may read: the host in the editor, the disk in tests.
pub trait Files {
    fn read(&self, path: &str) -> Option<Vec<u8>>;
    fn read_range(&self, path: &str, offset: u64, length: u64) -> Option<Vec<u8>>;
}

/// The packages CS:S looks in after the map's own pakfile, in order.
const PACKAGES: &[&str] = &["cstrike_pak_dir.vpk", "hl2_textures_dir.vpk", "hl2_misc_dir.vpk"];
const MAX_INCLUDES: usize = 8;
/// Enough of a VTF for its header and resource list.
const HEADER: usize = 4096;

/// Where a file was found.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Origin {
    Map,
    Game,
}

fn normalize(path: &str) -> String {
    path.trim().to_ascii_lowercase().replace('\\', "/").trim_start_matches('/').to_string()
}

/// The stored or LZMA-compressed files of a zip archive, by lowercase path.
fn zip_entries(zip: &[u8]) -> HashMap<String, (u16, usize, usize, usize)> {
    let mut out = HashMap::new();
    let tail = zip.len().saturating_sub(22 + 65535);
    let Some(end) = (tail..zip.len().saturating_sub(21)).rev().find(|&at| zip.get(at..at + 4) == Some(b"PK\x05\x06")) else { return out };
    let (count, mut at) = (u16_at(zip, end + 10), u32_at(zip, end + 16) as usize);
    for _ in 0..count {
        if zip.get(at..at + 4) != Some(b"PK\x01\x02") {
            break;
        }
        let (method, packed, size) = (u16_at(zip, at + 10), u32_at(zip, at + 20) as usize, u32_at(zip, at + 24) as usize);
        let (name, extra, comment) = (usize::from(u16_at(zip, at + 28)), usize::from(u16_at(zip, at + 30)), usize::from(u16_at(zip, at + 32)));
        let local = u32_at(zip, at + 42) as usize;
        if let Some(name) = zip.get(at + 46..at + 46 + name) {
            out.insert(normalize(&String::from_utf8_lossy(name)), (method, local, packed, size));
        }
        at += 46 + name + extra + comment;
    }
    out
}

/// The entry's bytes; an LZMA-compressed entry stops after `limit` bytes.
fn zip_read<'a>(zip: &'a [u8], &(method, local, packed, size): &(u16, usize, usize, usize), limit: usize) -> Option<Cow<'a, [u8]>> {
    if zip.get(local..local.checked_add(4)?) != Some(b"PK\x03\x04") {
        return None;
    }
    let start = local + 30 + usize::from(u16_at(zip, local + 26)) + usize::from(u16_at(zip, local + 28));
    let data = zip.get(start..start.checked_add(packed)?)?;
    match method {
        0 => Some(Cow::Borrowed(data)),
        // Zip's LZMA: a 2-byte version, a 2-byte property size, the properties, then the stream.
        14 => lzma(data.get(4..9)?, data.get(9..)?, size, limit).ok().map(Cow::Owned),
        _ => None,
    }
}

struct VpkEntry {
    preload: (usize, usize),
    archive: u16,
    offset: u32,
    length: u32,
}

/// A VPK directory: the materials, textures and models it holds and the archives holding their bytes.
struct Vpk {
    name: &'static str,
    dir: Vec<u8>,
    data_start: usize,
    entries: HashMap<String, VpkEntry>,
}

fn c_str(b: &[u8], at: &mut usize) -> Option<String> {
    let rest = b.get(*at..)?;
    let len = rest.iter().position(|&c| c == 0)?;
    *at += len + 1;
    Some(String::from_utf8_lossy(rest.get(..len)?).to_ascii_lowercase())
}

impl Vpk {
    fn open(name: &'static str, dir: Vec<u8>) -> Option<Self> {
        if u32_at(&dir, 0) != 0x55aa_1234 {
            return None;
        }
        let header = match u32_at(&dir, 4) {
            1 => 12,
            2 => 28,
            _ => return None,
        };
        let data_start = header + u32_at(&dir, 8) as usize;
        let mut entries = HashMap::new();
        let mut at = header;
        while let Some(ext) = c_str(&dir, &mut at).filter(|s| !s.is_empty()) {
            while let Some(path) = c_str(&dir, &mut at).filter(|s| !s.is_empty()) {
                while let Some(file) = c_str(&dir, &mut at).filter(|s| !s.is_empty()) {
                    let e = dir.get(at..at + 18)?;
                    let preload = usize::from(u16_at(e, 4));
                    let entry = VpkEntry { preload: (at + 18, preload), archive: u16_at(e, 6), offset: u32_at(e, 8), length: u32_at(e, 12) };
                    at += 18 + preload;
                    if ["vmt", "vtf", "mdl", "vvd", "vtx"].contains(&ext.as_str()) {
                        let full = if path == " " { format!("{file}.{ext}") } else { format!("{path}/{file}.{ext}") };
                        entries.insert(full, entry);
                    }
                }
            }
        }
        Some(Vpk { name, dir, data_start, entries })
    }

    /// The entry's bytes, stopping after `limit` of them.
    fn read(&self, path: &str, files: &dyn Files, limit: usize) -> Option<Vec<u8>> {
        let e = self.entries.get(path)?;
        let mut out = self.dir.get(e.preload.0..e.preload.0 + e.preload.1)?.to_vec();
        let length = (e.length as usize).min(limit.saturating_sub(out.len()));
        if length > 0 {
            let rest = if e.archive == 0x7fff {
                let at = self.data_start.checked_add(e.offset as usize)?;
                self.dir.get(at..at.checked_add(length)?)?.to_vec()
            } else {
                let archive = self.name.strip_suffix("dir.vpk")?;
                files.read_range(&format!("{archive}{:03}.vpk", e.archive), u64::from(e.offset), length as u64)?
            };
            out.extend(rest);
        }
        Some(out)
    }
}

/// Every place a material or texture can come from, searched in the engine's order.
pub struct Library<'a> {
    pak: &'a [u8],
    pak_entries: HashMap<String, (u16, usize, usize, usize)>,
    vpks: Vec<Vpk>,
    files: &'a dyn Files,
    /// Decoded textures by path, since VBSP's patched copies of a material share one.
    decoded: RefCell<HashMap<String, Option<Found>>>,
}

impl<'a> Library<'a> {
    pub fn new(pak: &'a [u8], files: &'a dyn Files) -> Self {
        let vpks = PACKAGES.iter().filter_map(|name| Vpk::open(name, files.read(name)?)).collect();
        Library { pak, pak_entries: zip_entries(pak), vpks, files, decoded: RefCell::default() }
    }

    pub fn has_game_files(&self) -> bool {
        !self.vpks.is_empty()
    }

    /// The materials in the game's packages, such as `concrete/concretefloor001a`.
    pub fn materials(&self) -> BTreeSet<&str> {
        self.vpks.iter().flat_map(|v| v.entries.keys()).filter_map(|p| p.strip_prefix("materials/")?.strip_suffix(".vmt")).collect()
    }

    /// The models in the game's packages, such as `models/props/de_dust/du_crate_64x64.mdl`.
    pub fn models(&self) -> BTreeSet<&str> {
        self.vpks.iter().flat_map(|v| v.entries.keys()).map(String::as_str).filter(|p| p.starts_with("models/") && p.ends_with(".mdl")).collect()
    }

    /// Whether one of the game's packages holds the file, without reading it.
    pub fn has_packed(&self, path: &str) -> bool {
        let path = normalize(path);
        self.vpks.iter().any(|v| v.entries.contains_key(&path))
    }

    /// A game file's bytes from the map's pakfile, the game's packages or its folders.
    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.find(&normalize(path), usize::MAX).map(|(data, _)| data.into_owned())
    }

    /// Whether the game has the file, reading as little of it as it can.
    pub fn has(&self, path: &str) -> bool {
        self.find(&normalize(path), 1).is_some()
    }

    /// Lets go of the pictures [`Self::picture`] decoded, for a library kept between calls.
    pub fn forget_pictures(&self) {
        self.decoded.borrow_mut().clear();
    }

    /// The file's bytes, at least its first `limit` of them, and where it was found.
    fn find(&self, path: &str, limit: usize) -> Option<(Cow<'a, [u8]>, Origin)> {
        if let Some(data) = self.pak_entries.get(path).and_then(|e| zip_read(self.pak, e, limit)) {
            return Some((data, Origin::Map));
        }
        let packed = self.vpks.iter().find_map(|v| v.read(path, self.files, limit));
        packed.or_else(|| self.files.read(path)).or_else(|| self.files.read(&format!("download/{path}"))).map(|d| (Cow::Owned(d), Origin::Game))
    }

    /// The textures that picture the material, Hammer's `%tooltexture` first, following `include` through patch materials.
    fn textures(&self, material: &str) -> Vec<String> {
        let mut name = material.to_string();
        for _ in 0..MAX_INCLUDES {
            let Some((vmt, _)) = self.find(&format!("materials/{name}.vmt"), usize::MAX) else { break };
            let text = String::from_utf8_lossy(&vmt);
            let tokens = tokens(&text);
            let found: Vec<String> = ["%tooltexture", "$basetexture"]
                .iter()
                .filter_map(|key| value(&tokens, key))
                .map(|t| normalize(t).trim_start_matches("materials/").trim_end_matches(".vtf").to_string())
                .collect();
            match value(&tokens, "include") {
                Some(include) if found.is_empty() => name = normalize(include).trim_start_matches("materials/").trim_end_matches(".vmt").to_string(),
                _ => return found,
            }
        }
        Vec::new()
    }

    /// Whether the material mixes two textures by vertex alpha itself, as a WorldVertexTransition does.
    pub fn blends(&self, material: &str) -> bool {
        let mut name = material.to_string();
        for _ in 0..MAX_INCLUDES {
            let Some((vmt, _)) = self.find(&format!("materials/{name}.vmt"), usize::MAX) else { return false };
            let text = String::from_utf8_lossy(&vmt);
            let tokens = tokens(&text);
            match (tokens.first(), value(&tokens, "include")) {
                (Some(Token::Text(s)), Some(include)) if s.eq_ignore_ascii_case("patch") => {
                    name = normalize(include).trim_start_matches("materials/").trim_end_matches(".vmt").to_string();
                }
                (Some(Token::Text(s)), _) => return s.eq_ignore_ascii_case("worldvertextransition"),
                _ => return false,
            }
        }
        false
    }

    /// The material's picture, no wider or taller than `max`, where its texture was found, and
    /// the texture's full size.
    pub fn picture(&self, material: &str, max: usize) -> Option<Found> {
        self.textures(material).iter().find_map(|texture| {
            let path = format!("materials/{texture}.vtf");
            if let Some(known) = self.decoded.borrow().get(&path) {
                return known.clone();
            }
            // Mips run smallest first, so a compressed texture is only unpacked up to the one wanted.
            let picture = self.find(&path, HEADER).and_then(|(head, origin)| {
                let need = vtf::needed(&head, max)?;
                let data = if need <= head.len() { head } else { self.find(&path, need)?.0 };
                vtf::decode(&data, max).ok().map(|p| (p, origin, vtf::full_size(&data)))
            });
            self.decoded.borrow_mut().insert(path, picture.clone());
            picture
        })
    }
}

/// A material's picture, where its texture was found, and the texture's full size.
pub type Found = (vtf::Picture, Origin, (u32, u32));

/// A KeyValues token; a quoted brace is text.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Token<'a> {
    Text(&'a str),
    Open,
    Close,
}

/// KeyValues tokens: quoted or bare strings and braces, without `//` comments.
pub fn tokens(text: &str) -> Vec<Token<'_>> {
    let mut out = Vec::new();
    let mut rest = text;
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            return out;
        }
        if rest.starts_with("//") {
            rest = rest.split_once('\n').map_or("", |(_, r)| r);
        } else if let Some(after) = rest.strip_prefix('{') {
            out.push(Token::Open);
            rest = after;
        } else if let Some(after) = rest.strip_prefix('}') {
            out.push(Token::Close);
            rest = after;
        } else if let Some(quoted) = rest.strip_prefix('"') {
            let (token, after) = quoted.split_once('"').unwrap_or((quoted, ""));
            out.push(Token::Text(token));
            rest = after;
        } else {
            let end = rest.find(|c: char| c.is_whitespace() || c == '"' || c == '{' || c == '}').unwrap_or(rest.len());
            let (token, after) = rest.split_at_checked(end).unwrap_or((rest, ""));
            out.push(Token::Text(token));
            rest = after;
        }
    }
}

/// The first value of `key` at any depth; a string followed by a brace names a block instead.
pub fn value<'t>(tokens: &[Token<'t>], key: &str) -> Option<&'t str> {
    let mut i = 0;
    while let (Some(k), Some(v)) = (tokens.get(i), tokens.get(i + 1)) {
        match (k, v) {
            (Token::Text(k), Token::Text(v)) if k.eq_ignore_ascii_case(key) => return Some(v),
            (Token::Text(_), Token::Text(_)) => i += 2,
            _ => i += 1,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_vmt_values() {
        let text = "\"LightmappedGeneric\"\n{\n// \"$basetexture\" \"no\"\n\t$basetexture \"Concrete\\Floor01\"\n\t\"%keywords\" \"a b\"\n\t\"Proxies\" { \"x\" { } }\n}";
        let t = tokens(text);
        assert_eq!(value(&t, "$BaseTexture"), Some("Concrete\\Floor01"));
        assert_eq!(value(&t, "%keywords"), Some("a b"));
        assert_eq!(value(&t, "proxies"), None);
        let patch = tokens("patch { include \"materials/a/b.vmt\" insert { \"$envmap\" \"c\" } }");
        assert_eq!(value(&patch, "include"), Some("materials/a/b.vmt"));
        assert_eq!(value(&patch, "$basetexture"), None);
        assert_eq!(tokens("{\"unterminated"), [Token::Open, Token::Text("unterminated")]);
        assert_eq!(tokens("k \"{\" a}"), [Token::Text("k"), Token::Text("{"), Token::Text("a"), Token::Close]);
    }

    struct Loose(HashMap<String, Vec<u8>>);

    impl Files for Loose {
        fn read(&self, path: &str) -> Option<Vec<u8>> {
            self.0.get(path).cloned()
        }

        fn read_range(&self, _path: &str, _offset: u64, _length: u64) -> Option<Vec<u8>> {
            None
        }
    }

    #[test]
    fn knows_materials_that_blend_themselves() {
        let files = Loose(HashMap::from([
            ("materials/nature/blend.vmt".to_string(), b"\"WorldVertexTransition\" { \"$basetexture\" \"a\" \"$basetexture2\" \"b\" }".to_vec()),
            ("materials/nature/patched.vmt".to_string(), b"patch { include \"materials/nature/blend.vmt\" insert { } }".to_vec()),
            ("materials/brick/wall.vmt".to_string(), b"LightmappedGeneric { $basetexture brick/wall }".to_vec()),
        ]));
        let library = Library::new(&[], &files);
        assert!(library.blends("nature/blend") && library.blends("nature/patched"));
        assert!(!library.blends("brick/wall") && !library.blends("missing"));
    }

    #[test]
    fn reads_stored_zip_entries() {
        // A one-file zip as bspzip writes it: stored, no compression.
        let (name, body) = (b"Materials/A.vmt", b"x{}");
        let mut zip = b"PK\x03\x04".to_vec();
        zip.extend([0u8; 22]);
        zip.extend((name.len() as u16).to_le_bytes());
        zip.extend([0, 0]);
        zip.extend(name);
        zip.extend(body);
        let central = zip.len();
        zip.extend(b"PK\x01\x02");
        zip.extend([0u8; 16]);
        zip.extend((body.len() as u32).to_le_bytes());
        zip.extend((body.len() as u32).to_le_bytes());
        zip.extend((name.len() as u16).to_le_bytes());
        zip.extend([0u8; 12]);
        zip.extend(0u32.to_le_bytes());
        zip.extend(name);
        let size = zip.len() - central;
        zip.extend(b"PK\x05\x06");
        zip.extend([0u8; 6]);
        zip.extend(1u16.to_le_bytes());
        zip.extend((size as u32).to_le_bytes());
        zip.extend((central as u32).to_le_bytes());
        zip.extend([0, 0]);
        let entries = zip_entries(&zip);
        let entry = entries.get("materials/a.vmt").unwrap();
        assert_eq!(zip_read(&zip, entry, usize::MAX).as_deref(), Some(&body[..]));
        assert!(zip_entries(&zip[..zip.len() - 3]).is_empty());
    }
}
