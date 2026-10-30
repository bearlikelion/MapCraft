//! `.mcpack`: the pictures and models a map ships beside itself, described in `docs/mcpack-format.md`.
//!
//! Packs come from strangers, so every number is checked before it is used and nothing is decoded here.
//! [`write`] runs the same checks as [`read`] on what it wrote, so MapCraft never writes a pack the Godot addon refuses.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::{Map as Object, Value};

pub const EXTENSION: &str = "mcpack";
pub const VERSION: u32 = 1;
const MAGIC: &[u8; 4] = b"MCPK";
const HEADER_LEN: u64 = 12;
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

// The limits; godot/src/mapcraft_pack.cpp repeats them and docs/mcpack-format.md lists them.
pub const MAX_PACK_SIZE: u64 = 256 << 20;
pub const MAX_ENTRIES: u32 = 1024;
pub const MAX_NAME_LEN: usize = 255;
pub const MAX_ENTRY_SIZE: u64 = 64 << 20;
pub const MAX_IMAGE_SIZE: u32 = 4096;
pub const MAX_TOTAL_PIXELS: u64 = 64 << 20;
pub const MAX_JPEG_SCANS: u32 = 64;
pub const MAX_GLB_JSON: u64 = 2 << 20;
pub const MAX_JSON_DEPTH: usize = 64;
pub const MAX_GLTF_NODES: usize = 4096;
pub const MAX_GLTF_NODE_DEPTH: usize = 64;
pub const MAX_GLTF_MESHES: usize = 1024;
pub const MAX_GLTF_PRIMITIVES: usize = 4096;
pub const MAX_GLTF_ACCESSORS: usize = 16384;
pub const MAX_GLTF_BUFFER_VIEWS: usize = 16384;
pub const MAX_GLTF_MATERIALS: usize = 1024;
pub const MAX_GLTF_TEXTURES: usize = 1024;
pub const MAX_GLTF_IMAGES: usize = 256;
pub const MAX_ACCESSOR_ELEMENTS: u64 = 1 << 22;
pub const MAX_MESH_COMPONENTS: u64 = 1 << 24;
pub const GLTF_EXTENSIONS: &[&str] = &["KHR_materials_emissive_strength", "KHR_materials_unlit", "KHR_mesh_quantization", "KHR_texture_transform"];

#[derive(Debug, thiserror::Error)]
pub enum PackError {
    #[error("invalid pack: {0}")]
    Invalid(String),
    #[error("{0} {1}")]
    Entry(String, String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

fn invalid(reason: impl Into<String>) -> PackError {
    PackError::Invalid(reason.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Png,
    Jpeg,
    Glb,
}

/// The pack that goes beside a map: `<map>.mcpack`.
pub fn path_for(map: &Path) -> PathBuf {
    map.with_extension(EXTENSION)
}

/// The entry name for a material's picture, `.png` or `.jpg` by its content, or `None` when it is neither.
pub fn material_entry(material: &str, picture: &[u8]) -> Option<String> {
    let ext = match image_header(picture).ok()?.0 {
        Kind::Png => "png",
        _ => "jpg",
    };
    Some(format!("materials/{material}.{ext}"))
}

/// The entry name the Godot addon looks up for a point entity's `model` value, or `None` when it is not a `.glb`.
pub fn model_entry(model: &str) -> Option<String> {
    let path = model.trim().replace('\\', "/");
    let path = path.strip_prefix("res://").unwrap_or_else(|| path.strip_prefix('/').unwrap_or(&path));
    let stem = path.len().checked_sub(4).filter(|&n| path.get(n..).is_some_and(|e| e.eq_ignore_ascii_case(".glb"))).and_then(|n| path.get(..n))?;
    Some(format!("models/{stem}.glb"))
}

/// Builds a pack from `(name, bytes)` entries, sorted by name, refusing anything [`read`] would refuse.
pub fn write(entries: &[(String, Vec<u8>)]) -> Result<Vec<u8>, PackError> {
    if entries.len() > MAX_ENTRIES as usize {
        return Err(invalid(format!("{} entries, at most {MAX_ENTRIES}", entries.len())));
    }
    let mut sorted: Vec<&(String, Vec<u8>)> = entries.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut toc_end = HEADER_LEN;
    let mut size = HEADER_LEN;
    for (name, data) in &sorted {
        kind_of(name).map_err(|r| PackError::Entry(name.clone(), r))?;
        let len = data.len() as u64;
        if !(1..=MAX_ENTRY_SIZE).contains(&len) {
            return Err(PackError::Entry(name.clone(), format!("is empty or larger than {} MiB", MAX_ENTRY_SIZE >> 20)));
        }
        toc_end += 18 + name.len() as u64;
        size += 18 + name.len() as u64 + len;
        if size > MAX_PACK_SIZE {
            return Err(invalid(format!("larger than {} MiB", MAX_PACK_SIZE >> 20)));
        }
    }
    let mut out = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(sorted.len() as u32).to_le_bytes());
    let mut offset = toc_end;
    for (name, data) in &sorted {
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(&(data.len() as u64).to_le_bytes());
        offset += data.len() as u64;
    }
    for (_, data) in &sorted {
        out.extend_from_slice(data);
    }
    validate(&out)?;
    Ok(out)
}

/// Writes atomically: a failed save leaves the previous pack intact.
pub fn save(path: &Path, entries: &[(String, Vec<u8>)]) -> Result<(), PackError> {
    craft_atomic::atomic_write(path, &write(entries)?)?;
    Ok(())
}

/// The entries of a pack, in file order, after every check passes.
pub fn read(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, PackError> {
    Ok(validate(bytes)?.into_iter().map(|(name, data)| (name, data.to_vec())).collect())
}

pub fn load(path: &Path) -> Result<Vec<(String, Vec<u8>)>, PackError> {
    if std::fs::metadata(path)?.len() > MAX_PACK_SIZE {
        return Err(invalid(format!("larger than {} MiB", MAX_PACK_SIZE >> 20)));
    }
    read(&std::fs::read(path)?)
}

fn validate(bytes: &[u8]) -> Result<Vec<(String, &[u8])>, PackError> {
    let entries = entries(bytes)?;
    let mut pixels = 0;
    for (name, data) in &entries {
        check(name, data, &mut pixels).map_err(|r| PackError::Entry(name.clone(), r))?;
    }
    Ok(entries)
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], PackError> {
        let end = self.at.checked_add(n).ok_or_else(|| invalid("table of contents is cut short"))?;
        let out = self.bytes.get(self.at..end).ok_or_else(|| invalid("table of contents is cut short"))?;
        self.at = end;
        Ok(out)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], PackError> {
        self.take(N)?.try_into().map_err(|_| invalid("table of contents is cut short"))
    }
}

/// The table of contents: everything about a pack except what its entries hold.
fn entries(bytes: &[u8]) -> Result<Vec<(String, &[u8])>, PackError> {
    let total = bytes.len() as u64;
    if total > MAX_PACK_SIZE {
        return Err(invalid(format!("larger than {} MiB", MAX_PACK_SIZE >> 20)));
    }
    if bytes.get(..4) != Some(MAGIC.as_slice()) {
        return Err(invalid("not a MapCraft pack"));
    }
    let mut c = Cursor { bytes, at: 4 };
    let version = u32::from_le_bytes(c.array()?);
    if version != VERSION {
        return Err(invalid(format!("version {version}, this MapCraft reads version {VERSION}")));
    }
    let count = u32::from_le_bytes(c.array()?);
    if count > MAX_ENTRIES {
        return Err(invalid(format!("{count} entries, at most {MAX_ENTRIES}")));
    }
    let mut toc = Vec::with_capacity(count as usize);
    for i in 0..count {
        let len = usize::from(u16::from_le_bytes(c.array()?));
        let name = std::str::from_utf8(c.take(len)?).map_err(|_| invalid(format!("entry {i} has a name that is not text")))?;
        kind_of(name).map_err(|r| invalid(format!("entry {i}: {r}")))?;
        toc.push((name, u64::from_le_bytes(c.array()?), u64::from_le_bytes(c.array()?)));
    }
    let mut at = c.at as u64;
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(toc.len());
    for (name, offset, length) in toc {
        let bad = |reason: &str| PackError::Entry(name.to_string(), reason.to_string());
        if !seen.insert(stem_key(name)) {
            return Err(bad("is in the pack twice (names ignore case and extension)"));
        }
        if !(1..=MAX_ENTRY_SIZE).contains(&length) {
            return Err(bad(&format!("is empty or larger than {} MiB", MAX_ENTRY_SIZE >> 20)));
        }
        if offset != at {
            return Err(bad("does not start where the entry before it ends"));
        }
        let end = offset.checked_add(length).filter(|&end| end <= total).ok_or_else(|| bad("runs past the end of the pack"))?;
        let data = usize::try_from(offset)
            .ok()
            .zip(usize::try_from(end).ok())
            .and_then(|(start, end)| bytes.get(start..end))
            .ok_or_else(|| bad("runs past the end of the pack"))?;
        out.push((name.to_string(), data));
        at = end;
    }
    if at != total {
        return Err(invalid("data after the last entry"));
    }
    Ok(out)
}

/// What a name holds, or why it is not allowed.
fn kind_of(name: &str) -> Result<Kind, String> {
    if name.is_empty() || name.len() > MAX_NAME_LEN {
        return Err(format!("names are 1 to {MAX_NAME_LEN} bytes"));
    }
    if let Some(c) = name.chars().find(|&c| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/'))) {
        return Err(format!("{c:?} is not allowed in a name"));
    }
    if name.split('/').any(|s| s.is_empty() || s == "." || s == "..") {
        return Err("names cannot have empty, . or .. path segments".into());
    }
    let (stem, kind, folder) = if let Some(stem) = name.strip_suffix(".png") {
        (stem, Kind::Png, "materials/")
    } else if let Some(stem) = name.strip_suffix(".jpg") {
        (stem, Kind::Jpeg, "materials/")
    } else if let Some(stem) = name.strip_suffix(".glb") {
        (stem, Kind::Glb, "models/")
    } else {
        return Err("only .png, .jpg and .glb entries are allowed".into());
    };
    match stem.strip_prefix(folder) {
        Some(rest) if !rest.is_empty() && !rest.ends_with('/') => Ok(kind),
        _ => Err(format!("this kind of entry belongs under {folder}")),
    }
}

/// Names are unique ignoring ASCII case and the extension, so `a.png` and `A.jpg` collide.
fn stem_key(name: &str) -> String {
    name.get(..name.len().saturating_sub(4)).unwrap_or(name).to_ascii_lowercase()
}

fn check(name: &str, data: &[u8], pixels: &mut u64) -> Result<(), String> {
    match kind_of(name)? {
        Kind::Glb => glb(data, pixels),
        kind => image(data, kind, pixels),
    }
}

/// Checks a picture's header against the caps and charges its pixels to the pack.
fn image(data: &[u8], expected: Kind, pixels: &mut u64) -> Result<(), String> {
    let (kind, width, height) = image_header(data)?;
    if kind != expected {
        return Err("holds a different kind of picture than its name says".into());
    }
    if !(1..=MAX_IMAGE_SIZE).contains(&width) || !(1..=MAX_IMAGE_SIZE).contains(&height) {
        return Err(format!("is {width}x{height}, pictures are 1x1 to {MAX_IMAGE_SIZE}x{MAX_IMAGE_SIZE}"));
    }
    *pixels = pixels.saturating_add(u64::from(width) * u64::from(height));
    if *pixels > MAX_TOTAL_PIXELS {
        return Err(format!("takes the pack's pictures past {} million pixels", MAX_TOTAL_PIXELS >> 20));
    }
    Ok(())
}

fn image_header(data: &[u8]) -> Result<(Kind, u32, u32), String> {
    let be32 = |at: usize| data.get(at..at + 4).and_then(|b| b.try_into().ok()).map(u32::from_be_bytes);
    if data.starts_with(&PNG_SIGNATURE) {
        // IHDR must come first: length 13, then width and height.
        if data.get(8..16) != Some(&[0, 0, 0, 13, b'I', b'H', b'D', b'R'][..]) {
            return Err("is a PNG without its header".into());
        }
        let (Some(width), Some(height)) = (be32(16), be32(20)) else { return Err("is a cut-short PNG".into()) };
        return Ok((Kind::Png, width, height));
    }
    if data.starts_with(&[0xFF, 0xD8]) {
        let (width, height) = jpeg_size(data)?;
        return Ok((Kind::Jpeg, width, height));
    }
    Err("is neither a PNG nor a JPEG".into())
}

/// Walks a JPEG's markers to its frame size, counting scans, since each progressive scan makes the decoder pass over the whole picture.
fn jpeg_size(data: &[u8]) -> Result<(u32, u32), String> {
    let be16 = |at: usize| data.get(at..at + 2).and_then(|b| b.try_into().ok()).map(|b| usize::from(u16::from_be_bytes(b)));
    let mut size = None;
    let mut scans = 0;
    let mut at = 2;
    while let Some(&byte) = data.get(at) {
        if byte != 0xFF {
            return Err("is a JPEG with a broken marker".into());
        }
        while data.get(at) == Some(&0xFF) {
            at += 1;
        }
        let Some(&marker) = data.get(at) else { break };
        at += 1;
        if marker == 0xD9 {
            break;
        }
        if matches!(marker, 0x00 | 0x01 | 0xD0..=0xD8) {
            return Err("is a JPEG with a misplaced marker".into());
        }
        if matches!(marker, 0xC3 | 0xC5..=0xCB | 0xCD..=0xCF) {
            return Err("is a JPEG of a kind the game cannot decode".into());
        }
        let len = be16(at).filter(|&len| len >= 2).ok_or("is a cut-short JPEG")?;
        let end = at + len;
        if end > data.len() {
            return Err("is a cut-short JPEG".into());
        }
        if matches!(marker, 0xC0..=0xC2) {
            if size.is_some() {
                return Err("is a JPEG with more than one frame".into());
            }
            let (Some(height), Some(width)) = (be16(at + 3).filter(|_| len >= 8), be16(at + 5)) else { return Err("is a cut-short JPEG".into()) };
            size = Some((width as u32, height as u32));
        }
        at = end;
        if marker == 0xDA {
            if size.is_none() {
                return Err("is a JPEG that starts a scan before its frame".into());
            }
            scans += 1;
            if scans > MAX_JPEG_SCANS {
                return Err(format!("is a JPEG with over {MAX_JPEG_SCANS} scans"));
            }
            // Scan data ends at the first 0xFF that is not a stuffed zero or a restart marker.
            while let Some(&b) = data.get(at) {
                if b == 0xFF && !matches!(data.get(at + 1), Some(0x00 | 0xD0..=0xD7)) {
                    break;
                }
                at += if b == 0xFF { 2 } else { 1 };
            }
        }
    }
    size.ok_or_else(|| "is a JPEG without a frame header".into())
}

/// Checks a binary glTF before Godot sees it: the container, then everything in its JSON that could reach outside the file or run away with memory.
fn glb(data: &[u8], pixels: &mut u64) -> Result<(), String> {
    let le32 = |at: usize| data.get(at..at + 4).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes);
    if data.get(..4) != Some(b"glTF".as_slice()) {
        return Err("is not a GLB model".into());
    }
    if le32(4) != Some(2) {
        return Err("is not glTF 2.0".into());
    }
    if le32(8).map(u64::from) != Some(data.len() as u64) {
        return Err("has the wrong length in its header".into());
    }
    let json_len = u64::from(le32(12).ok_or("is a cut-short GLB")?);
    if le32(16) != Some(0x4E4F_534A) {
        return Err("does not start with its JSON chunk".into());
    }
    if json_len > MAX_GLB_JSON {
        return Err(format!("has a JSON chunk over {} MiB", MAX_GLB_JSON >> 20));
    }
    let json_end = 20 + json_len as usize;
    let json = data.get(20..json_end).ok_or("has a JSON chunk past its end")?;
    let bin = if json_end == data.len() {
        &[][..]
    } else {
        let bin_len = u64::from(le32(json_end).ok_or("has a cut-short BIN chunk")?);
        if le32(json_end + 4) != Some(0x004E_4942) {
            return Err("has a second chunk that is not BIN".into());
        }
        if json_end as u64 + 8 + bin_len != data.len() as u64 {
            return Err("has a BIN chunk that does not end with the file".into());
        }
        data.get(json_end + 8..).ok_or("has a cut-short BIN chunk")?
    };
    let doc: Value = serde_json::from_slice(json).map_err(|e| format!("has broken JSON: {e}"))?;
    gltf(&doc, bin, pixels)
}

/// A JSON number that is a whole number from 0 to `max`; Godot reads every JSON number as a float.
fn whole(value: Option<&Value>, max: u64) -> Option<u64> {
    let value = value?;
    if let Some(n) = value.as_u64() {
        return (n <= max).then_some(n);
    }
    let f = value.as_f64()?;
    ((0.0..=max as f64).contains(&f) && f.fract() == 0.0).then_some(f as u64)
}

fn optional(object: &Object<String, Value>, key: &str, default: u64, max: u64) -> Option<u64> {
    match object.get(key) {
        None => Some(default),
        value => whole(value, max),
    }
}

fn index<'a, T>(value: Option<&Value>, items: &'a [T]) -> Option<&'a T> {
    items.get(usize::try_from(whole(value, u64::MAX)?).ok()?)
}

fn list<'a>(object: &'a Object<String, Value>, key: &str, max: usize) -> Result<&'a [Value], String> {
    match object.get(key) {
        None => Ok(&[]),
        Some(Value::Array(items)) if items.len() <= max => Ok(items),
        Some(_) => Err(format!("has {key} that is not a list of at most {max}")),
    }
}

fn object<'a>(value: &'a Value, what: &str) -> Result<&'a Object<String, Value>, String> {
    value.as_object().ok_or_else(|| format!("has {what} that is not an object"))
}

/// Every `extensions` object anywhere may only name allowed extensions, since Godot acts on some without checking `extensionsUsed`.
fn extensions_allowed(value: &Value, depth: usize) -> Result<(), String> {
    if depth > MAX_JSON_DEPTH {
        return Err(format!("nests its JSON over {MAX_JSON_DEPTH} deep"));
    }
    match value {
        Value::Array(items) => items.iter().try_for_each(|v| extensions_allowed(v, depth + 1)),
        Value::Object(fields) => {
            match fields.get("extensions") {
                None | Some(Value::Null) => {}
                Some(Value::Object(used)) => {
                    if let Some(name) = used.keys().find(|k| !GLTF_EXTENSIONS.contains(&k.as_str())) {
                        return Err(format!("uses the glTF extension {name:?}, which packed models cannot"));
                    }
                }
                Some(_) => return Err("has extensions that are not an object".into()),
            }
            fields.values().try_for_each(|v| extensions_allowed(v, depth + 1))
        }
        _ => Ok(()),
    }
}

struct View {
    offset: u64,
    length: u64,
    stride: Option<u64>,
}

fn gltf(doc: &Value, bin: &[u8], pixels: &mut u64) -> Result<(), String> {
    let root = object(doc, "JSON")?;
    extensions_allowed(doc, 0)?;
    for key in ["extensionsUsed", "extensionsRequired"] {
        for name in list(root, key, usize::MAX)? {
            if !name.as_str().is_some_and(|n| GLTF_EXTENSIONS.contains(&n)) {
                return Err(format!("lists the glTF extension {name}, which packed models cannot use"));
            }
        }
    }
    for key in ["animations", "skins", "cameras"] {
        if root.get(key).is_some_and(|v| v.as_array().is_none_or(|a| !a.is_empty())) {
            return Err(format!("has {key}, which packed models cannot"));
        }
    }

    let buffers = list(root, "buffers", 1)?;
    let mut buffer_len = 0;
    if let Some(buffer) = buffers.first() {
        let buffer = object(buffer, "a buffer")?;
        if buffer.contains_key("uri") {
            return Err("has a buffer outside the file (uri)".into());
        }
        buffer_len = whole(buffer.get("byteLength"), bin.len() as u64).ok_or("has a buffer longer than its BIN chunk")?;
    }
    let mut views = Vec::new();
    for view in list(root, "bufferViews", MAX_GLTF_BUFFER_VIEWS)? {
        let view = object(view, "a buffer view")?;
        let outside = "has a buffer view outside its buffer";
        if buffers.is_empty() || whole(view.get("buffer"), 0).is_none() {
            return Err(outside.into());
        }
        let offset = optional(view, "byteOffset", 0, buffer_len).ok_or(outside)?;
        let length = whole(view.get("byteLength"), buffer_len - offset).filter(|&l| l > 0).ok_or(outside)?;
        let stride = match view.get("byteStride") {
            None => None,
            stride => Some(whole(stride, 252).filter(|&s| s >= 4).ok_or("has a buffer view with a bad byteStride")?),
        };
        views.push(View { offset, length, stride });
    }

    let mut components = Vec::new();
    for accessor in list(root, "accessors", MAX_GLTF_ACCESSORS)? {
        let accessor = object(accessor, "an accessor")?;
        if accessor.contains_key("sparse") {
            return Err("has a sparse accessor, which packed models cannot".into());
        }
        let view = index(accessor.get("bufferView"), &views).ok_or("has an accessor without a buffer view")?;
        let size = match whole(accessor.get("componentType"), u64::MAX) {
            Some(5120 | 5121) => 1,
            Some(5122 | 5123) => 2,
            Some(5125 | 5126) => 4,
            _ => return Err("has an accessor of an unknown component type".into()),
        };
        let n = match accessor.get("type").and_then(Value::as_str) {
            Some("SCALAR") => 1,
            Some("VEC2") => 2,
            Some("VEC3") => 3,
            Some("VEC4") => 4,
            _ => return Err("has an accessor of a type packed models cannot use".into()),
        };
        let count = whole(accessor.get("count"), MAX_ACCESSOR_ELEMENTS).filter(|&c| c > 0).ok_or("has an accessor with no or too many elements")?;
        let offset = optional(accessor, "byteOffset", 0, view.length).ok_or("has an accessor that reads past its buffer view")?;
        let element = size * n;
        let end = view.stride.unwrap_or(element).checked_mul(count - 1).and_then(|s| s.checked_add(offset)).and_then(|s| s.checked_add(element));
        if end.is_none_or(|end| end > view.length) {
            return Err("has an accessor that reads past its buffer view".into());
        }
        components.push(count * n);
    }

    for picture in list(root, "images", MAX_GLTF_IMAGES)? {
        let picture = object(picture, "an image")?;
        if picture.contains_key("uri") {
            return Err("has a picture outside the file (uri)".into());
        }
        let view = index(picture.get("bufferView"), &views).ok_or("has a picture without a buffer view")?;
        let kind = match picture.get("mimeType").and_then(Value::as_str) {
            Some("image/png") => Kind::Png,
            Some("image/jpeg") => Kind::Jpeg,
            _ => return Err("has a picture that is not image/png or image/jpeg".into()),
        };
        let bytes = usize::try_from(view.offset)
            .ok()
            .zip(usize::try_from(view.offset + view.length).ok())
            .and_then(|(start, end)| bin.get(start..end))
            .ok_or("has a picture outside its BIN chunk")?;
        image(bytes, kind, pixels).map_err(|e| format!("has a picture that {e}"))?;
    }

    let mut primitives = 0;
    let mut total: u64 = 0;
    for mesh in list(root, "meshes", MAX_GLTF_MESHES)? {
        for primitive in list(object(mesh, "a mesh")?, "primitives", MAX_GLTF_PRIMITIVES)? {
            primitives += 1;
            if primitives > MAX_GLTF_PRIMITIVES {
                return Err(format!("has over {MAX_GLTF_PRIMITIVES} mesh primitives"));
            }
            let primitive = object(primitive, "a mesh primitive")?;
            if primitive.contains_key("targets") {
                return Err("has morph targets, which packed models cannot".into());
            }
            let attributes = primitive.get("attributes").and_then(Value::as_object).ok_or("has a mesh primitive without attributes")?;
            for accessor in attributes.values().chain(primitive.get("indices")) {
                total = total.saturating_add(*index(Some(accessor), &components).ok_or("has a mesh primitive with a missing accessor")?);
            }
            if total > MAX_MESH_COMPONENTS {
                return Err(format!("has meshes over {} million values", MAX_MESH_COMPONENTS >> 20));
            }
        }
    }

    let nodes = list(root, "nodes", MAX_GLTF_NODES)?;
    let mut parent: Vec<Option<usize>> = vec![None; nodes.len()];
    for (i, node) in nodes.iter().enumerate() {
        for child in list(object(node, "a node")?, "children", MAX_GLTF_NODES)? {
            let slot = whole(Some(child), u64::MAX).and_then(|c| parent.get_mut(usize::try_from(c).ok()?)).ok_or("has a node with a missing child")?;
            if slot.replace(i).is_some() {
                return Err("has a node with two parents".into());
            }
        }
    }
    // Godot walks parent chains without a limit, so a loop would hang it.
    for start in 0..nodes.len() {
        let mut at = start;
        let mut depth = 0;
        while let Some(Some(up)) = parent.get(at) {
            depth += 1;
            if depth > MAX_GLTF_NODE_DEPTH {
                return Err(format!("has nodes in a loop or nested over {MAX_GLTF_NODE_DEPTH} deep"));
            }
            at = *up;
        }
    }

    list(root, "materials", MAX_GLTF_MATERIALS)?;
    list(root, "textures", MAX_GLTF_TEXTURES)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut out = PNG_SIGNATURE.to_vec();
        out.extend([0, 0, 0, 13]);
        out.extend(b"IHDR");
        out.extend(width.to_be_bytes());
        out.extend(height.to_be_bytes());
        out.extend([8, 6, 0, 0, 0, 0, 0, 0, 0]);
        out
    }

    fn segment(marker: u8, body: &[u8]) -> Vec<u8> {
        let mut out = vec![0xFF, marker];
        out.extend(((body.len() + 2) as u16).to_be_bytes());
        out.extend(body);
        out
    }

    fn jpeg_with(frame: u8, width: u16, height: u16, scans: usize) -> Vec<u8> {
        let mut out = vec![0xFF, 0xD8];
        out.extend(segment(0xE0, b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0"));
        let mut sof = vec![8];
        sof.extend(height.to_be_bytes());
        sof.extend(width.to_be_bytes());
        sof.extend([1, 1, 0x11, 0]);
        out.extend(segment(frame, &sof));
        for _ in 0..scans {
            out.extend(segment(0xDA, &[1, 1, 0, 0, 63, 0]));
            out.extend([0x12, 0xFF, 0x00, 0x34, 0xFF, 0xD0, 0x56]);
        }
        out.extend([0xFF, 0xD9]);
        out
    }

    fn jpeg(width: u16, height: u16) -> Vec<u8> {
        jpeg_with(0xC0, width, height, 1)
    }

    fn glb_from(json: &[u8], bin: &[u8]) -> Vec<u8> {
        let mut out = b"glTF".to_vec();
        out.extend(2u32.to_le_bytes());
        out.extend((20 + json.len() as u32 + if bin.is_empty() { 0 } else { 8 + bin.len() as u32 }).to_le_bytes());
        out.extend((json.len() as u32).to_le_bytes());
        out.extend(b"JSON");
        out.extend(json);
        if !bin.is_empty() {
            out.extend((bin.len() as u32).to_le_bytes());
            out.extend(b"BIN\0");
            out.extend(bin);
        }
        out
    }

    fn glb(json: &Value, bin: &[u8]) -> Vec<u8> {
        glb_from(&serde_json::to_vec(json).unwrap(), bin)
    }

    /// A triangle with an embedded PNG: the shape every model test edits.
    fn model() -> (Value, Vec<u8>) {
        let picture = png(2, 2);
        let mut bin = vec![0u8; 42];
        bin.extend(&picture);
        let json = json!({
            "asset": {"version": "2.0"},
            "extensionsUsed": ["KHR_materials_unlit"],
            "scene": 0,
            "scenes": [{"nodes": [0]}],
            "nodes": [{"name": "Crate", "mesh": 0, "children": [1]}, {"name": "Lid", "extras": {"note": "kept"}}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1, "material": 0}]}],
            "materials": [{"pbrMetallicRoughness": {"baseColorTexture": {"index": 0}}, "extensions": {"KHR_materials_unlit": {}}}],
            "textures": [{"source": 0}],
            "images": [{"bufferView": 2, "mimeType": "image/png"}],
            "accessors": [
                {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"},
                {"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"}
            ],
            "bufferViews": [
                {"buffer": 0, "byteLength": 36},
                {"buffer": 0, "byteOffset": 36, "byteLength": 6},
                {"buffer": 0, "byteOffset": 42, "byteLength": picture.len()}
            ],
            "buffers": [{"byteLength": bin.len()}]
        });
        (json, bin)
    }

    fn entry(name: &str, data: Vec<u8>) -> (String, Vec<u8>) {
        (name.to_string(), data)
    }

    fn good() -> Vec<(String, Vec<u8>)> {
        let (json, bin) = model();
        vec![entry("models/props/crate.glb", glb(&json, &bin)), entry("materials/dev/grid.png", png(64, 64)), entry("materials/Rock.jpg", jpeg(32, 16))]
    }

    /// A pack laid out by hand, so the reader sees what no writer would make.
    fn raw(entries: &[(&[u8], u64, u64)], count: u32, data: &[u8]) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        out.extend(VERSION.to_le_bytes());
        out.extend(count.to_le_bytes());
        for (name, offset, length) in entries {
            out.extend((name.len() as u16).to_le_bytes());
            out.extend(*name);
            out.extend(offset.to_le_bytes());
            out.extend(length.to_le_bytes());
        }
        out.extend(data);
        out
    }

    fn toc_len(names: &[&[u8]]) -> u64 {
        HEADER_LEN + names.iter().map(|n| 18 + n.len() as u64).sum::<u64>()
    }

    fn reason(result: Result<impl std::fmt::Debug, PackError>) -> String {
        result.unwrap_err().to_string()
    }

    fn write_err(name: &str, data: Vec<u8>) -> String {
        reason(write(&[entry(name, data)]))
    }

    #[test]
    fn round_trips_sorted_and_stable() {
        let entries = good();
        let bytes = write(&entries).unwrap();
        let back = read(&bytes).unwrap();
        let mut sorted = entries.clone();
        sorted.sort();
        assert_eq!(back, sorted);
        let mut shuffled = entries;
        shuffled.reverse();
        assert_eq!(write(&shuffled).unwrap(), bytes);
        assert_eq!(&bytes[..4], b"MCPK");
    }

    #[test]
    fn saves_and_loads_beside_the_map() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_for(&dir.path().join("surf_test.mcmap"));
        assert_eq!(path.file_name().unwrap(), "surf_test.mcpack");
        save(&path, &good()).unwrap();
        assert_eq!(load(&path).unwrap().len(), 3);
    }

    #[test]
    fn names_entries_as_the_addon_looks_them_up() {
        assert_eq!(material_entry("dev/grid", &png(4, 4)).as_deref(), Some("materials/dev/grid.png"));
        assert_eq!(material_entry("rock", &jpeg(4, 4)).as_deref(), Some("materials/rock.jpg"));
        assert_eq!(material_entry("rock", b"GIF89a"), None);
        assert_eq!(model_entry(" Props\\Crate.GLB ").as_deref(), Some("models/Props/Crate.glb"));
        assert_eq!(model_entry("res://props/crate.glb").as_deref(), Some("models/props/crate.glb"));
        assert_eq!(model_entry("/props/crate.glb").as_deref(), Some("models/props/crate.glb"));
        assert_eq!(model_entry("props/crate.gltf"), None);
    }

    #[test]
    fn the_godot_test_box_passes() {
        let bytes = include_bytes!("../../../godot/test/Assets/Models/box.glb").to_vec();
        write(&[entry("models/Assets/Models/box.glb", bytes)]).unwrap();
    }

    /// The Godot addon's test loads this file, so it proves the addon accepts what the writer makes.
    #[test]
    fn the_godot_test_pack_matches_the_writer() {
        let stripes = include_bytes!("../../../godot/test/Assets/Textures/stripes.png").to_vec();
        let crate_model = include_bytes!("../../../godot/test/Assets/Models/box.glb").to_vec();
        let bytes = write(&[entry("materials/ramps-orange1.png", stripes), entry("models/props/crate.glb", crate_model)]).unwrap();
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../godot/test/pack/course.mcpack");
        if std::env::var_os("MAPCRAFT_BLESS").is_some() {
            std::fs::write(&path, &bytes).unwrap();
        }
        assert!(std::fs::read(&path).unwrap() == bytes, "run with MAPCRAFT_BLESS=1 to rewrite {}", path.display());
    }

    #[test]
    fn every_truncation_is_refused() {
        let bytes = write(&good()).unwrap();
        for len in 0..bytes.len() {
            assert!(read(&bytes[..len]).is_err(), "a pack cut to {len} bytes was read");
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(reason(read(&longer)).contains("after the last entry"));
    }

    #[test]
    fn bad_headers_are_refused() {
        assert!(reason(read(b"PK\x03\x04 zip file")).contains("not a MapCraft pack"));
        let mut newer = write(&good()).unwrap();
        newer[4] = 2;
        assert!(reason(read(&newer)).contains("version 2"));
        assert!(reason(read(&raw(&[], u32::MAX, &[]))).contains("at most 1024"));
        assert!(reason(read(&raw(&[], MAX_ENTRIES + 1, &[]))).contains("at most 1024"));
    }

    #[test]
    fn offsets_and_lengths_are_checked() {
        let a: &[u8] = b"materials/a.png";
        let b: &[u8] = b"materials/b.png";
        let start = toc_len(&[a, b]);
        let picture = png(1, 1);
        let n = picture.len() as u64;
        let data = [picture.clone(), picture].concat();
        let layout = |a_at: u64, a_len: u64, b_at: u64, b_len: u64| reason(read(&raw(&[(a, a_at, a_len), (b, b_at, b_len)], 2, &data)));
        assert!(layout(start, n, start + n, n + 1).contains("past the end"));
        assert!(layout(start, n, start, n).contains("does not start where"));
        assert!(layout(start, n, start + n - 4, n).contains("does not start where"));
        assert!(layout(start + 1, n, start + n, n).contains("does not start where"));
        assert!(layout(start, n, start + n, u64::MAX - start - n + 1).contains("larger than 64 MiB"));
        assert!(layout(start, 0, start, n).contains("empty"));
        let single = toc_len(&[a]);
        assert!(reason(read(&raw(&[(a, single, u64::MAX)], 1, &[]))).contains("larger than 64 MiB"));
        let huge = MAX_ENTRY_SIZE;
        assert!(reason(read(&raw(&[(a, u64::MAX - 4, huge)], 1, &[]))).contains("does not start where"));
        assert!(reason(read(&raw(&[(a, single, huge)], 1, &[]))).contains("past the end"));
    }

    #[test]
    fn hostile_names_are_refused() {
        let picture = png(1, 1);
        for name in [
            "../materials/a.png",
            "materials/../a.png",
            "materials/./a.png",
            "materials//a.png",
            "/materials/a.png",
            "materials/a.png/",
            "materials\\a.png",
            "C:/materials/a.png",
            "materials/C:a.png",
            "materials/a\0.png",
            "materials/a b.png",
            "materials/é.png",
            "materials/.png",
            "materials/a/.png",
            "materials/a.PNG",
            "models/a.png",
            "materials/a.glb",
            "a.png",
            "",
        ] {
            assert!(write(&[entry(name, picture.clone())]).is_err(), "{name:?} was written");
            let bytes = name.as_bytes();
            assert!(reason(read(&raw(&[(bytes, toc_len(&[bytes]), picture.len() as u64)], 1, &picture))).contains("entry 0"), "{name:?} was read");
        }
        assert!(write_err(&format!("materials/{}.png", "a".repeat(250)), picture.clone()).contains("1 to 255"));
        let not_text: &[u8] = b"materials/\xFF.png";
        assert!(reason(read(&raw(&[(not_text, toc_len(&[not_text]), picture.len() as u64)], 1, &picture))).contains("not text"));
    }

    #[test]
    fn only_pictures_and_glb_models_are_allowed() {
        for ext in ["tres", "res", "scn", "tscn", "pck", "gd", "cs", "gdshader", "svg", "so", "dll", "dylib", "webp", "gltf", "bin", "jpeg"] {
            assert!(write_err(&format!("materials/a.{ext}"), b"x".to_vec()).contains("only .png, .jpg and .glb"), "{ext}");
            assert!(write_err(&format!("models/a.{ext}"), b"x".to_vec()).contains("only .png, .jpg and .glb"), "{ext}");
        }
    }

    #[test]
    fn duplicates_are_refused_ignoring_case_and_extension() {
        for (a, b) in [("materials/a.png", "materials/a.png"), ("materials/Grid.png", "materials/grid.png"), ("materials/a.png", "materials/A.jpg")] {
            let data = if b.ends_with(".jpg") { jpeg(1, 1) } else { png(1, 1) };
            assert!(reason(write(&[entry(a, png(1, 1)), entry(b, data)])).contains("twice"), "{a} {b}");
        }
    }

    #[test]
    fn content_must_match_the_extension() {
        let (json, bin) = model();
        assert!(write_err("materials/a.jpg", png(1, 1)).contains("different kind"));
        assert!(write_err("materials/a.png", jpeg(1, 1)).contains("different kind"));
        assert!(write_err("materials/a.png", glb(&json, &bin)).contains("neither a PNG nor a JPEG"));
        assert!(write_err("materials/a.png", b"<svg xmlns='http://www.w3.org/2000/svg'/>".to_vec()).contains("neither"));
        assert!(write_err("models/a.glb", png(1, 1)).contains("not a GLB"));
        assert!(write_err("models/a.glb", b"{\"asset\":{\"version\":\"2.0\"}}".to_vec()).contains("not a GLB"));
    }

    #[test]
    fn picture_sizes_are_capped_before_decoding() {
        assert!(write_err("materials/a.png", png(100_000, 100_000)).contains("100000x100000"));
        assert!(write_err("materials/a.png", png(4097, 1)).contains("4097x1"));
        assert!(write_err("materials/a.png", png(0, 16)).contains("0x16"));
        assert!(write_err("materials/a.jpg", jpeg(65535, 65535)).contains("65535x65535"));
        assert!(write_err("materials/a.jpg", jpeg(16, 0)).contains("16x0"));
        write(&[entry("materials/a.png", png(4096, 4096)), entry("materials/b.jpg", jpeg(4096, 4096))]).unwrap();
        let many: Vec<_> = (0..5).map(|i| entry(&format!("materials/{i}.png"), png(4096, 4096))).collect();
        assert!(reason(write(&many)).contains("64 million pixels"));
        assert!(write_err("materials/a.png", PNG_SIGNATURE.to_vec()).contains("without its header"));
        assert!(write_err("materials/a.png", png(1, 1)[..20].to_vec()).contains("cut-short"));
    }

    #[test]
    fn jpeg_markers_are_walked_strictly() {
        write(&[entry("materials/a.jpg", jpeg_with(0xC2, 64, 64, MAX_JPEG_SCANS as usize))]).unwrap();
        assert!(write_err("materials/a.jpg", jpeg_with(0xC2, 64, 64, MAX_JPEG_SCANS as usize + 1)).contains("scans"));
        assert!(write_err("materials/a.jpg", jpeg_with(0xC9, 64, 64, 1)).contains("cannot decode"));
        assert!(write_err("materials/a.jpg", jpeg_with(0xC3, 64, 64, 1)).contains("cannot decode"));
        let mut scan_first = vec![0xFF, 0xD8];
        scan_first.extend(segment(0xDA, &[1, 1, 0, 0, 63, 0]));
        scan_first.extend(jpeg(8, 8).get(2..).unwrap());
        assert!(write_err("materials/a.jpg", scan_first).contains("before its frame"));
        let mut two_frames = jpeg(8, 8);
        two_frames.truncate(two_frames.len() - 2);
        two_frames.extend(segment(0xC0, &[8, 0x40, 0, 0x40, 0, 1, 1, 0x11, 0]));
        assert!(write_err("materials/a.jpg", two_frames).contains("more than one frame"));
        let mut long_segment = vec![0xFF, 0xD8, 0xFF, 0xE0, 0xFF, 0xF0];
        long_segment.extend([0; 8]);
        assert!(write_err("materials/a.jpg", long_segment).contains("cut-short"));
        assert!(write_err("materials/a.jpg", vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x01]).contains("cut-short"));
        assert!(write_err("materials/a.jpg", vec![0xFF, 0xD8, 0x12]).contains("broken marker"));
        assert!(write_err("materials/a.jpg", vec![0xFF, 0xD8, 0xFF, 0xD8]).contains("misplaced"));
        assert!(write_err("materials/a.jpg", vec![0xFF, 0xD8, 0xFF, 0xD9]).contains("without a frame"));
    }

    fn model_err(edit: impl FnOnce(&mut Value)) -> String {
        let (mut json, bin) = model();
        edit(&mut json);
        write_err("models/a.glb", glb(&json, &bin))
    }

    #[test]
    fn a_good_model_passes() {
        let (json, bin) = model();
        write(&[entry("models/a.glb", glb(&json, &bin))]).unwrap();
        write(&[entry("models/a.glb", glb(&json!({"asset": {"version": "2.0"}}), &[]))]).unwrap();
    }

    #[test]
    fn models_cannot_reach_outside_the_file() {
        for uri in [
            "data:application/octet-stream;base64,AAAA",
            "file:///etc/passwd",
            "/etc/passwd",
            "C:\\Windows\\win.ini",
            "res://project.godot",
            "user://x.bin",
            "../x.bin",
        ] {
            assert!(model_err(|j| j["buffers"][0]["uri"] = json!(uri)).contains("buffer outside the file"), "{uri}");
            assert!(model_err(|j| j["images"][0]["uri"] = json!(uri)).contains("picture outside the file"), "{uri}");
        }
        assert!(model_err(|j| j["images"][0] = json!({"uri": "evil.tres"})).contains("picture outside the file"));
        assert!(model_err(|j| j["buffers"] = json!([{"byteLength": 8}, {"byteLength": 8, "uri": "x.bin"}])).contains("buffers"));
    }

    #[test]
    fn model_extensions_are_allowlisted() {
        assert!(model_err(|j| j["extensionsRequired"] = json!(["KHR_draco_mesh_compression"])).contains("KHR_draco_mesh_compression"));
        assert!(model_err(|j| j["extensionsRequired"] = json!(["EXT_unknown"])).contains("EXT_unknown"));
        assert!(model_err(|j| j["extensionsUsed"] = json!(["OMI_physics_body"])).contains("OMI_physics_body"));
        assert!(model_err(|j| j["extensions"] = json!({"KHR_lights_punctual": {"lights": []}})).contains("KHR_lights_punctual"));
        assert!(model_err(|j| j["nodes"][1]["extensions"] = json!({"KHR_node_visibility": {"visible": false}})).contains("KHR_node_visibility"));
        assert!(model_err(|j| j["materials"][0]["extensions"] = json!({"KHR_materials_variants": {}})).contains("KHR_materials_variants"));
        assert!(model_err(|j| j["materials"][0]["extensions"] = json!([])).contains("not an object"));
        let (mut json, bin) = model();
        json["extensionsRequired"] = json!(["KHR_texture_transform"]);
        write(&[entry("models/a.glb", glb(&json, &bin))]).unwrap();
    }

    #[test]
    fn models_are_static() {
        assert!(model_err(|j| j["animations"] = json!([{"channels": [], "samplers": []}])).contains("animations"));
        assert!(model_err(|j| j["animations"] = json!({})).contains("animations"));
        assert!(model_err(|j| j["skins"] = json!([{"joints": [0]}])).contains("skins"));
        assert!(model_err(|j| j["cameras"] = json!([{"type": "perspective"}])).contains("cameras"));
        assert!(model_err(|j| j["meshes"][0]["primitives"][0]["targets"] = json!([{"POSITION": 0}])).contains("morph targets"));
        let (mut json, bin) = model();
        json["animations"] = json!([]);
        write(&[entry("models/a.glb", glb(&json, &bin))]).unwrap();
    }

    #[test]
    fn model_containers_are_checked() {
        let (json, bin) = model();
        let good = glb(&json, &bin);
        let mut wrong_length = good.clone();
        wrong_length[8] ^= 1;
        assert!(write_err("models/a.glb", wrong_length).contains("wrong length"));
        let mut version = good.clone();
        version[4] = 1;
        assert!(write_err("models/a.glb", version).contains("glTF 2.0"));
        let mut huge_json = good.clone();
        huge_json[12..16].copy_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
        assert!(write_err("models/a.glb", huge_json).contains("over 2 MiB"));
        let mut past_end = good.clone();
        let len = past_end.len() as u32;
        past_end[12..16].copy_from_slice(&(len - 19).to_le_bytes());
        assert!(write_err("models/a.glb", past_end).contains("past its end"));
        let mut not_bin = good.clone();
        let at = 20 + u32::from_le_bytes(good[12..16].try_into().unwrap()) as usize + 4;
        not_bin[at] = b'X';
        assert!(write_err("models/a.glb", not_bin).contains("not BIN"));
        let mut short_bin = good.clone();
        short_bin[at - 4] ^= 1;
        assert!(write_err("models/a.glb", short_bin).contains("does not end with the file"));
        let mut padded = json.to_string().into_bytes();
        padded.resize(padded.len() + (MAX_GLB_JSON as usize), b' ');
        assert!(write_err("models/a.glb", glb_from(&padded, &bin)).contains("over 2 MiB"));
        assert!(write_err("models/a.glb", glb_from(b"{\"asset\":", &bin)).contains("broken JSON"));
        assert!(write_err("models/a.glb", glb_from(b"[1, 2]", &bin)).contains("not an object"));
        let deep = format!("{{\"asset\":{{\"version\":\"2.0\"}},\"extras\":{}{}}}", "[".repeat(100), "]".repeat(100));
        assert!(write_err("models/a.glb", glb_from(deep.as_bytes(), &bin)).contains("nests its JSON"));
    }

    #[test]
    fn model_buffers_are_bounds_checked() {
        assert!(model_err(|j| j["buffers"][0]["byteLength"] = json!(1 << 30)).contains("longer than its BIN"));
        assert!(model_err(|j| j["bufferViews"][2]["byteLength"] = json!(1000)).contains("outside its buffer"));
        assert!(model_err(|j| j["bufferViews"][2]["byteOffset"] = json!(u64::MAX)).contains("outside its buffer"));
        assert!(model_err(|j| j["bufferViews"][0]["buffer"] = json!(1)).contains("outside its buffer"));
        assert!(model_err(|j| j["bufferViews"][0]["byteLength"] = json!(-1)).contains("outside its buffer"));
        assert!(model_err(|j| j["bufferViews"][0]["byteLength"] = json!(1.5)).contains("outside its buffer"));
        assert!(model_err(|j| j["bufferViews"][0]["byteStride"] = json!(2)).contains("byteStride"));
        assert!(model_err(|j| j["accessors"][0]["count"] = json!(4)).contains("reads past"));
        assert!(model_err(|j| j["accessors"][0]["byteOffset"] = json!(4)).contains("reads past"));
        assert!(model_err(|j| j["accessors"][0]["count"] = json!(MAX_ACCESSOR_ELEMENTS + 1)).contains("too many elements"));
        assert!(model_err(|j| j["accessors"][0]["count"] = json!(0)).contains("too many elements"));
        assert!(model_err(|j| j["accessors"][0].as_object_mut().unwrap().remove("bufferView").map(|_| ()).unwrap()).contains("without a buffer view"));
        assert!(model_err(|j| j["accessors"][0]["bufferView"] = json!(9)).contains("without a buffer view"));
        assert!(model_err(|j| j["accessors"][0]["sparse"] = json!({"count": 1 << 30})).contains("sparse"));
        assert!(model_err(|j| j["accessors"][0]["type"] = json!("MAT4")).contains("type"));
        assert!(model_err(|j| j["accessors"][0]["componentType"] = json!(5130)).contains("component type"));
        let (mut json, bin) = model();
        json["bufferViews"][0]["byteStride"] = json!(252);
        json["bufferViews"][0]["byteLength"] = json!(36);
        json["accessors"][0]["count"] = json!(MAX_ACCESSOR_ELEMENTS);
        assert!(write_err("models/a.glb", glb(&json, &bin)).contains("reads past"));
    }

    #[test]
    fn model_pictures_are_checked_like_materials() {
        let picture = png(5000, 5000);
        let (mut json, mut bin) = model();
        bin.truncate(42);
        bin.extend(&picture);
        json["bufferViews"][2]["byteLength"] = json!(picture.len());
        json["buffers"][0]["byteLength"] = json!(bin.len());
        assert!(write_err("models/a.glb", glb(&json, &bin)).contains("picture that is 5000x5000"));
        assert!(model_err(|j| j["images"][0]["mimeType"] = json!("image/jpeg")).contains("different kind"));
        assert!(model_err(|j| j["images"][0]["mimeType"] = json!("image/webp")).contains("image/png or image/jpeg"));
        assert!(model_err(|j| j["images"][0].as_object_mut().unwrap().remove("mimeType").map(|_| ()).unwrap()).contains("image/png or image/jpeg"));
        assert!(model_err(|j| j["images"][0]["bufferView"] = json!(0)).contains("neither a PNG nor a JPEG"));
        let (json, bin) = model();
        let images = json!((0..=MAX_GLTF_IMAGES).map(|_| json["images"][0].clone()).collect::<Vec<_>>());
        assert!(model_err(|j| j["images"] = images).contains("images"));
        let big: Vec<_> = (0..4).map(|i| entry(&format!("materials/{i}.png"), png(4096, 4096))).collect();
        let mut with_model = big.clone();
        with_model.push(entry("models/a.glb", glb(&json, &bin)));
        write(&big).unwrap();
        assert!(reason(write(&with_model)).contains("million pixels"));
    }

    #[test]
    fn model_meshes_have_a_budget() {
        let primitive = json!({"attributes": {"POSITION": 0}, "indices": 1});
        let one = json!([{"primitives": [primitive.clone()]}]);
        assert!(model_err(|j| j["meshes"][0]["primitives"][0]["attributes"]["NORMAL"] = json!(7)).contains("missing accessor"));
        assert!(
            model_err(|j| j["meshes"][0]["primitives"][0].as_object_mut().unwrap().remove("attributes").map(|_| ()).unwrap()).contains("without attributes")
        );
        let too_many = json!((0..=MAX_GLTF_PRIMITIVES).map(|_| primitive.clone()).collect::<Vec<_>>());
        assert!(model_err(|j| j["meshes"][0]["primitives"] = too_many).contains("primitives"));
        let meshes = json!((0..=MAX_GLTF_MESHES).map(|_| one[0].clone()).collect::<Vec<_>>());
        assert!(model_err(|j| j["meshes"] = meshes).contains("meshes"));
        // One big accessor shared by many primitives still costs memory each time Godot decodes it.
        let (mut json, _) = model();
        let bin = vec![0u8; 12 * 65536];
        json["bufferViews"] = json!([{"buffer": 0, "byteLength": bin.len()}]);
        json["buffers"] = json!([{"byteLength": bin.len()}]);
        json["accessors"] = json!([{"bufferView": 0, "componentType": 5126, "count": 65536, "type": "VEC3"}]);
        json["images"] = json!([]);
        json["textures"] = json!([]);
        json["meshes"] = json!([{"primitives": (0..85).map(|_| json!({"attributes": {"POSITION": 0}})).collect::<Vec<_>>()}]);
        write(&[entry("models/a.glb", glb(&json, &bin))]).unwrap();
        json["meshes"][0]["primitives"] = json!((0..86).map(|_| json!({"attributes": {"POSITION": 0}})).collect::<Vec<_>>());
        assert!(write_err("models/a.glb", glb(&json, &bin)).contains("16 million values"));
    }

    #[test]
    fn model_node_trees_cannot_loop() {
        assert!(model_err(|j| j["nodes"][1]["children"] = json!([0])).contains("loop"));
        assert!(model_err(|j| j["nodes"][0]["children"] = json!([0])).contains("loop"));
        assert!(model_err(|j| j["nodes"][0]["children"] = json!([1, 1])).contains("two parents"));
        assert!(model_err(|j| j["nodes"][0]["children"] = json!([5])).contains("missing child"));
        assert!(model_err(|j| j["nodes"][0]["children"] = json!([-1])).contains("missing child"));
        let deep: Vec<Value> = (0..=MAX_GLTF_NODE_DEPTH + 1).map(|i| if i <= MAX_GLTF_NODE_DEPTH { json!({"children": [i + 1]}) } else { json!({}) }).collect();
        assert!(model_err(|j| j["nodes"] = json!(deep)).contains("nested over 64"));
        let nodes = json!((0..=MAX_GLTF_NODES).map(|_| json!({})).collect::<Vec<_>>());
        assert!(model_err(|j| j["nodes"] = nodes).contains("nodes"));
        let ok: Vec<Value> = (0..=MAX_GLTF_NODE_DEPTH).map(|i| if i < MAX_GLTF_NODE_DEPTH { json!({"children": [i + 1]}) } else { json!({}) }).collect();
        let (mut json, bin) = model();
        json["nodes"] = json!(ok);
        write(&[entry("models/a.glb", glb(&json, &bin))]).unwrap();
    }

    #[test]
    fn oversized_packs_are_refused_before_building() {
        let big = vec![0u8; (MAX_ENTRY_SIZE + 1) as usize];
        assert!(write_err("materials/a.png", big).contains("larger than 64 MiB"));
        let many: Vec<_> = (0..=MAX_ENTRIES).map(|i| entry(&format!("materials/{i}.png"), png(1, 1))).collect();
        assert!(reason(write(&many)).contains("at most 1024"));
    }
}
