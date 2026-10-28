//! Reads Godot 4's binary resource format (`.scn`, `.res`), plain (`RSRC`) or zstd compressed
//! (`RSCC`), into the same sections its text form (`.tscn`, `.tres`) parses to.

use std::io::Read;

use crate::tscn::{MAX_BYTES, Section, Value};

/// Nested values deeper than this are refused, so hostile files cannot exhaust the stack.
const MAX_DEPTH: usize = 64;
/// A compressed resource may unpack to at most this many bytes.
const MAX_UNPACKED: usize = 256 << 20;
const ZSTD: u32 = 2;
const NAMED_IDS: u32 = 1;
const UIDS: u32 = 2;
const REAL_IS_DOUBLE: u32 = 4;
const SCRIPT_CLASS: u32 = 8;
const RESERVED_FIELDS: usize = 11;
const NO_PARENT: i64 = 0x7FFF_FFFF;
const ID_IS_PATH: i64 = 1 << 30;
const INDEX_MASK: i64 = (1 << 24) - 1;
const NAME_MASK: i64 = (1 << 18) - 1;
const PROP_NAME_MASK: i64 = (1 << 30) - 1;
const TYPE_INSTANTIATED: i64 = 0x7FFF_FFFF;
const PLACEHOLDER: i64 = 1 << 30;

pub fn is_binary(data: &[u8]) -> bool {
    data.starts_with(b"RSRC") || data.starts_with(b"RSCC")
}

pub fn parse(data: &[u8]) -> Result<Vec<Section>, String> {
    let unpacked;
    let (s, start) = if let Some(rest) = data.strip_prefix(b"RSCC") {
        unpacked = unpack(rest)?;
        (unpacked.as_slice(), 0)
    } else if data.starts_with(b"RSRC") {
        (data, 4)
    } else {
        return Err("not a Godot binary resource".into());
    };
    let mut r = Reader { s, i: start, big: false, double: false, keep_bytes: false, strings: Vec::new(), ext: Vec::new(), ids: Vec::new() };
    r.big = r.u32()? != 0;
    r.u32()?;
    let (major, _minor, format) = (r.u32()?, r.u32()?, r.u32()?);
    if major != 4 || !(4..=6).contains(&format) {
        return Err(format!("binary resource format {format} from Godot {major} is not a Godot 4 one"));
    }
    let class = r.string()?;
    r.u64()?;
    let flags = r.u32()?;
    r.double = flags & REAL_IS_DOUBLE != 0;
    r.u64()?;
    if flags & SCRIPT_CLASS != 0 {
        r.string()?;
    }
    r.bytes(RESERVED_FIELDS * 4)?;
    for _ in 0..r.count(4)? {
        let text = r.string()?;
        r.strings.push(text);
    }
    for _ in 0..r.count(8)? {
        let kind = r.string()?;
        let path = r.string()?;
        if flags & UIDS != 0 {
            r.u64()?;
        }
        r.ext.push((kind, path));
    }
    let mut internal = Vec::new();
    for k in 0..r.count(12)? {
        let path = r.string()?;
        let offset = r.u64()?;
        let id = path.strip_prefix("local://").filter(|_| flags & NAMED_IDS != 0).map_or_else(|| k.to_string(), str::to_string);
        r.ids.push(id);
        internal.push(offset);
    }
    let mut resources = Vec::new();
    let mut end = r.i;
    for offset in internal {
        r.i = usize::try_from(offset).ok().filter(|o| *o >= end && *o <= s.len()).ok_or_else(|| "a resource offset is out of order".to_string())?;
        let kind = r.string()?;
        r.keep_bytes = kind == "ArrayMesh";
        let mut props = Vec::new();
        for _ in 0..r.count(8)? {
            let name = r.name()?;
            // A value of a type this reader does not know ends its resource, since its size is unknown.
            let Some(value) = r.variant(0)? else { break };
            props.push((name, value));
        }
        end = r.i;
        resources.push((kind, props));
    }
    let Some((main_kind, main_props)) = resources.pop() else { return Err("the resource is empty".into()) };
    let header = if class == "PackedScene" { "gd_scene" } else { "gd_resource" };
    let mut out = vec![Section { tag: header.into(), attrs: vec![("type".into(), Value::Str(class))], props: Vec::new() }];
    for (i, (kind, path)) in r.ext.iter().enumerate() {
        let attrs = vec![("type".into(), Value::Str(kind.clone())), ("path".into(), Value::Str(path.clone())), ("id".into(), Value::Str(i.to_string()))];
        out.push(Section { tag: "ext_resource".into(), attrs, props: Vec::new() });
    }
    for ((kind, props), id) in resources.into_iter().zip(&r.ids) {
        out.push(Section { tag: "sub_resource".into(), attrs: vec![("type".into(), Value::Str(kind)), ("id".into(), Value::Str(id.clone()))], props });
    }
    if main_kind == "PackedScene" {
        let bundled = main_props.iter().find(|(k, _)| k == "_bundled").map(|(_, v)| v).ok_or("the scene has no nodes")?;
        out.extend(nodes(bundled, s.len())?);
    } else {
        out.push(Section { tag: "resource".into(), attrs: Vec::new(), props: main_props });
    }
    Ok(out)
}

/// The `RSCC` container: zstd blocks that each unpack to the block size, the last to the rest.
fn unpack(data: &[u8]) -> Result<Vec<u8>, String> {
    let bad = || "the compressed resource is damaged".to_string();
    let u32_at = |at: usize| data.get(at..at.checked_add(4)?).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes);
    let (mode, block, total) = (u32_at(0).ok_or_else(bad)?, u32_at(4).ok_or_else(bad)?, u32_at(8).ok_or_else(bad)?);
    if mode != ZSTD {
        return Err(format!("compression mode {mode} is not zstd"));
    }
    let (block, total) = (usize::try_from(block).map_err(|_| bad())?, usize::try_from(total).map_err(|_| bad())?);
    if block == 0 || total > MAX_UNPACKED {
        return Err(bad());
    }
    let count = total / block + 1;
    let mut at = count.checked_mul(4).and_then(|n| n.checked_add(12)).filter(|n| *n <= data.len()).ok_or_else(bad)?;
    let mut out = Vec::new();
    for k in 0..count {
        let size = usize::try_from(u32_at(12 + k * 4).ok_or_else(bad)?).map_err(|_| bad())?;
        let chunk = data.get(at..at.checked_add(size).ok_or_else(bad)?).ok_or_else(bad)?;
        at += size;
        let want = block.min(total - out.len());
        if want == 0 {
            continue;
        }
        let decoder = ruzstd::decoding::StreamingDecoder::new(chunk).map_err(|_| bad())?;
        if decoder.take(want as u64).read_to_end(&mut out).map_err(|_| bad())? != want {
            return Err(bad());
        }
    }
    Ok(out)
}

struct Reader<'a> {
    s: &'a [u8],
    i: usize,
    big: bool,
    double: bool,
    /// Whether byte arrays are kept: only meshes need theirs, for their vertices.
    keep_bytes: bool,
    strings: Vec<String>,
    /// External resources as type and path.
    ext: Vec<(String, String)>,
    /// Internal resource ids, in file order.
    ids: Vec<String>,
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], String> {
        let b = self.i.checked_add(n).and_then(|end| self.s.get(self.i..end)).ok_or_else(|| format!("damaged at byte {}", self.i))?;
        self.i += n;
        Ok(b)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], String> {
        let at = self.i;
        self.bytes(N)?.try_into().map_err(|_| format!("damaged at byte {at}"))
    }

    fn u16(&mut self) -> Result<u16, String> {
        let b = self.array()?;
        Ok(if self.big { u16::from_be_bytes(b) } else { u16::from_le_bytes(b) })
    }

    fn u32(&mut self) -> Result<u32, String> {
        let b = self.array()?;
        Ok(if self.big { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) })
    }

    fn u64(&mut self) -> Result<u64, String> {
        let b = self.array()?;
        Ok(if self.big { u64::from_be_bytes(b) } else { u64::from_le_bytes(b) })
    }

    fn int(&mut self) -> Result<f64, String> {
        Ok(f64::from(self.u32()? as i32))
    }

    fn float(&mut self) -> Result<f64, String> {
        Ok(f64::from(f32::from_bits(self.u32()?)))
    }

    fn double(&mut self) -> Result<f64, String> {
        Ok(f64::from_bits(self.u64()?))
    }

    fn real(&mut self) -> Result<f64, String> {
        if self.double { self.double() } else { self.float() }
    }

    fn real_size(&self) -> usize {
        if self.double { 8 } else { 4 }
    }

    /// A count of items at least `size` bytes each, refused when the rest of the file cannot hold them.
    fn count(&mut self, size: usize) -> Result<usize, String> {
        let at = self.i;
        let n = usize::try_from(self.u32()? & 0x7FFF_FFFF).unwrap_or(usize::MAX);
        if n.saturating_mul(size) > self.s.len().saturating_sub(self.i) {
            return Err(format!("damaged at byte {at}: a count runs past the end"));
        }
        Ok(n)
    }

    fn text(&mut self, len: usize) -> Result<String, String> {
        Ok(String::from_utf8_lossy(self.bytes(len)?).trim_end_matches('\0').to_string())
    }

    fn string(&mut self) -> Result<String, String> {
        let n = self.count(1)?;
        self.text(n)
    }

    /// A name from the string table, or inline when its top bit is set.
    fn name(&mut self) -> Result<String, String> {
        let at = self.i;
        let id = self.u32()?;
        if id & 0x8000_0000 != 0 {
            return self.text(usize::try_from(id & 0x7FFF_FFFF).unwrap_or(usize::MAX));
        }
        usize::try_from(id).ok().and_then(|i| self.strings.get(i)).cloned().ok_or_else(|| format!("damaged at byte {at}: no string {id}"))
    }

    fn call(name: &str, items: Vec<f64>) -> Value {
        Value::Call(name.into(), items.into_iter().map(Value::Num).collect())
    }

    fn reals(&mut self, name: &str, n: usize) -> Result<Value, String> {
        Ok(Self::call(name, (0..n).map(|_| self.real()).collect::<Result<_, _>>()?))
    }

    fn ints(&mut self, name: &str, n: usize) -> Result<Value, String> {
        Ok(Self::call(name, (0..n).map(|_| self.int()).collect::<Result<_, _>>()?))
    }

    /// A packed array of `per` numbers an item, each `size` bytes.
    fn packed(&mut self, name: &str, per: usize, size: usize, read: fn(&mut Self) -> Result<f64, String>) -> Result<Value, String> {
        let n = self.count(per * size)?;
        Ok(Self::call(name, (0..n * per).map(|_| read(self)).collect::<Result<_, _>>()?))
    }

    /// One value, or none for a type this reader does not know.
    fn variant(&mut self, depth: usize) -> Result<Option<Value>, String> {
        if depth > MAX_DEPTH {
            return Err("values nest too deeply".into());
        }
        let real = self.real_size();
        Ok(Some(match self.u32()? {
            1 | 42 | 43 => Value::Nil,
            2 => Value::Bool(self.u32()? != 0),
            3 => Value::Num(self.int()?),
            4 => Value::Num(self.real()?),
            5 | 44 => Value::Str(self.string()?),
            10 => self.reals("Vector2", 2)?,
            11 => self.reals("Rect2", 4)?,
            12 => self.reals("Vector3", 3)?,
            13 => self.reals("Plane", 4)?,
            14 => self.reals("Quaternion", 4)?,
            15 => self.reals("AABB", 6)?,
            16 => self.reals("Basis", 9)?,
            17 => self.reals("Transform3D", 12)?,
            18 => self.reals("Transform2D", 6)?,
            20 => Self::call("Color", (0..4).map(|_| self.float()).collect::<Result<_, _>>()?),
            22 => self.node_path()?,
            23 => Value::Num(f64::from(self.u32()?)),
            24 => self.object()?,
            26 => {
                let mut entries = Vec::new();
                for _ in 0..self.count(8)? {
                    let (Some(k), Some(v)) = (self.variant(depth + 1)?, self.variant(depth + 1)?) else { return Ok(None) };
                    entries.push((k, v));
                }
                Value::Dict(entries)
            }
            30 => {
                let mut items = Vec::new();
                for _ in 0..self.count(4)? {
                    let Some(v) = self.variant(depth + 1)? else { return Ok(None) };
                    items.push(v);
                }
                Value::List(items)
            }
            31 => {
                let n = self.count(1)?;
                let data = self.bytes(n.next_multiple_of(4))?;
                Value::Bytes(data.get(..n).filter(|_| self.keep_bytes && n <= MAX_BYTES).map(<[u8]>::to_vec).unwrap_or_default())
            }
            32 => self.packed("PackedInt32Array", 1, 4, Self::int)?,
            33 => self.packed("PackedFloat32Array", 1, 4, Self::float)?,
            34 => {
                let mut items = Vec::new();
                for _ in 0..self.count(4)? {
                    items.push(Value::Str(self.string()?));
                }
                Value::Call("PackedStringArray".into(), items)
            }
            35 => self.packed("PackedVector3Array", 3, real, Self::real)?,
            36 => self.packed("PackedColorArray", 4, 4, Self::float)?,
            37 => self.packed("PackedVector2Array", 2, real, Self::real)?,
            40 => Value::Num(self.u64()? as i64 as f64),
            41 => Value::Num(self.double()?),
            45 => self.ints("Vector2i", 2)?,
            46 => self.ints("Rect2i", 4)?,
            47 => self.ints("Vector3i", 3)?,
            48 => self.packed("PackedInt64Array", 1, 8, |r| Ok(r.u64()? as i64 as f64))?,
            49 => self.packed("PackedFloat64Array", 1, 8, Self::double)?,
            50 => self.reals("Vector4", 4)?,
            51 => self.ints("Vector4i", 4)?,
            52 => self.reals("Projection", 16)?,
            53 => self.packed("PackedVector4Array", 4, real, Self::real)?,
            _ => return Ok(None),
        }))
    }

    fn node_path(&mut self) -> Result<Value, String> {
        let names = self.u16()?;
        let subs = self.u16()?;
        let mut text = if subs & 0x8000 != 0 { "/".to_string() } else { String::new() };
        for k in 0..names {
            if k > 0 {
                text.push('/');
            }
            text += &self.name()?;
        }
        for _ in 0..subs & 0x7FFF {
            text.push(':');
            text += &self.name()?;
        }
        Ok(Value::Call("NodePath".into(), vec![Value::Str(text)]))
    }

    /// A resource reference, as the text form's `SubResource("id")` or `ExtResource("id")`.
    fn object(&mut self) -> Result<Value, String> {
        let at = self.i;
        let index = |r: &mut Self| r.u32().map(|n| usize::try_from(n).unwrap_or(usize::MAX));
        Ok(match self.u32()? {
            0 => Value::Nil,
            1 => {
                let kind = self.string()?;
                let path = self.string()?;
                self.ext.push((kind, path));
                Value::Call("ExtResource".into(), vec![Value::Str((self.ext.len() - 1).to_string())])
            }
            2 => {
                let id = index(self)?;
                let id = self.ids.get(id).ok_or_else(|| format!("damaged at byte {at}: no resource {id}"))?;
                Value::Call("SubResource".into(), vec![Value::Str(id.clone())])
            }
            3 => {
                let id = index(self)?;
                if id < self.ext.len() { Value::Call("ExtResource".into(), vec![Value::Str(id.to_string())]) } else { Value::Nil }
            }
            other => return Err(format!("damaged at byte {at}: unknown resource reference {other}")),
        })
    }
}

/// How many values a value holds, to bound copies of shared ones.
fn weight(v: &Value) -> usize {
    match v {
        Value::Call(_, items) | Value::List(items) => 1 + items.iter().map(weight).sum::<usize>(),
        Value::Dict(entries) => 1 + entries.iter().map(|(k, v)| weight(k) + weight(v)).sum::<usize>(),
        Value::Bytes(b) => 1 + b.len() / 8,
        _ => 1,
    }
}

/// The `node` sections of a packed scene's `_bundled` data, as the text form writes them.
fn nodes(bundled: &Value, file_len: usize) -> Result<Vec<Section>, String> {
    let bad = || "the scene's node data is damaged".to_string();
    let list = |key: &str| match bundled.get(key) {
        Some(Value::List(items) | Value::Call(_, items)) => items.as_slice(),
        _ => &[],
    };
    let names: Vec<&str> = list("names").iter().map(|n| n.text().unwrap_or_default()).collect();
    let (variants, paths) = (list("variants"), list("node_paths"));
    let ints: Vec<i64> = list("nodes").iter().map(|n| n.num().unwrap_or(-1.0) as i64).collect();
    let count = bundled.get("node_count").and_then(Value::num).unwrap_or(0.0).max(0.0) as usize;
    let mut ints = ints.into_iter();
    let mut next = || ints.next().ok_or_else(bad);
    let name = |i: i64| usize::try_from(i).ok().and_then(|i| names.get(i)).map(|s| s.to_string()).ok_or_else(bad);
    let mut budget = file_len.saturating_mul(2).saturating_add(1_000_000);
    let mut value = |i: i64| -> Result<Value, String> {
        let v = usize::try_from(i).ok().and_then(|i| variants.get(i)).ok_or_else(bad)?;
        budget = budget.checked_sub(weight(v)).ok_or("the scene repeats too many values")?;
        Ok(v.clone())
    };
    let mut node_paths: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for i in 0..count {
        let parent = next()?;
        next()?;
        let kind = next()?;
        let node_name = name(next()? & NAME_MASK)?;
        let instance = next()?;
        let mut props = Vec::new();
        for _ in 0..next()? {
            let key = name(next()? & PROP_NAME_MASK)?;
            props.push((key, value(next()?)?));
        }
        for _ in 0..next()? {
            next()?;
        }
        let parent_path = if parent < 0 || parent == NO_PARENT {
            None
        } else if parent & ID_IS_PATH != 0 {
            let text = usize::try_from(parent & INDEX_MASK).ok().and_then(|p| paths.get(p)).and_then(Value::text).ok_or_else(bad)?;
            Some(text.split('/').filter(|p| !p.is_empty() && *p != ".").collect::<Vec<_>>().join("/"))
        } else {
            Some(usize::try_from(parent & INDEX_MASK).ok().filter(|p| *p < i).and_then(|p| node_paths.get(p)).cloned().ok_or_else(bad)?)
        };
        let mut attrs = vec![("name".to_string(), Value::Str(node_name.clone()))];
        if kind != TYPE_INSTANTIATED {
            attrs.push(("type".into(), Value::Str(name(kind)?)));
        }
        node_paths.push(match parent_path.as_deref() {
            None => String::new(),
            Some("") => node_name,
            Some(p) => format!("{p}/{node_name}"),
        });
        if let Some(p) = parent_path {
            attrs.push(("parent".into(), Value::Str(if p.is_empty() { ".".into() } else { p })));
        }
        if instance >= 0 && instance & PLACEHOLDER == 0 {
            attrs.push(("instance".into(), value(instance & INDEX_MASK)?));
        }
        out.push(Section { tag: "node".into(), attrs, props });
    }
    Ok(out)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::tscn;

    /// Writes text form sections as a binary resource, the way Godot's exporter would.
    pub fn encode(sections: &[Section], big: bool, double: bool) -> Vec<u8> {
        write(sections, big, double, 0)
    }

    /// What an `RSCC` file compresses: the resource without its magic, offsets counted from there.
    pub fn encode_body(sections: &[Section], big: bool, double: bool) -> Vec<u8> {
        write(sections, big, double, 4).split_off(4)
    }

    fn write(sections: &[Section], big: bool, double: bool, shift: u64) -> Vec<u8> {
        let of = |tag: &str| sections.iter().filter(|s| s.tag == tag).collect::<Vec<_>>();
        let (ext, sub, nodes) = (of("ext_resource"), of("sub_resource"), of("node"));
        let ext_ids: Vec<&str> = ext.iter().map(|s| s.attr_text("id").unwrap()).collect();
        let sub_ids: Vec<&str> = sub.iter().map(|s| s.attr_text("id").unwrap()).collect();
        let mut w = W { out: b"RSRC".to_vec(), big, double, ext: ext_ids, sub: sub_ids, strings: Vec::new() };
        let main_props =
            if nodes.is_empty() { of("resource").first().map(|s| s.props.clone()).unwrap_or_default() } else { vec![("_bundled".into(), bundle(&nodes))] };
        let class = sections[0].attr_text("type").unwrap_or("PackedScene").to_string();
        for s in sub.iter().map(|s| &s.props).chain([&main_props]) {
            for (k, _) in s {
                if !w.strings.contains(k) {
                    w.strings.push(k.clone());
                }
            }
        }
        w.out.extend(u32::from(big).to_le_bytes());
        w.u32s(&[0, 4, 5, 6]);
        w.str(&class);
        w.u64(0);
        w.u32(NAMED_IDS | UIDS | if double { REAL_IS_DOUBLE } else { 0 });
        w.u64(u64::MAX);
        (0..RESERVED_FIELDS).for_each(|_| w.u32(0));
        w.u32(w.strings.len() as u32);
        for s in w.strings.clone() {
            w.str(&s);
        }
        w.u32(ext.len() as u32);
        for e in &ext {
            w.str(e.attr_text("type").unwrap());
            w.str(e.attr_text("path").unwrap());
            w.u64(u64::MAX);
        }
        let resources: Vec<(String, &str, &[(String, Value)])> = sub
            .iter()
            .map(|s| (format!("local://{}", s.attr_text("id").unwrap()), s.attr_text("type").unwrap(), s.props.as_slice()))
            .chain([("res://main".to_string(), if nodes.is_empty() { class.as_str() } else { "PackedScene" }, main_props.as_slice())])
            .collect();
        w.u32(resources.len() as u32);
        let mut slots = Vec::new();
        for (path, _, _) in &resources {
            w.str(path);
            slots.push(w.out.len());
            w.u64(0);
        }
        for ((_, kind, props), slot) in resources.iter().zip(slots) {
            let at = w.out.len() as u64 - shift;
            let bytes = if big { at.to_be_bytes() } else { at.to_le_bytes() };
            w.out[slot..slot + 8].copy_from_slice(&bytes);
            w.str(kind);
            w.u32(props.len() as u32);
            for (k, v) in *props {
                let i = w.strings.iter().position(|s| s == k).unwrap();
                w.u32(i as u32);
                w.variant(v);
            }
        }
        w.out.extend(b"RSRC");
        w.out
    }

    /// The `_bundled` data of the node sections.
    fn bundle(nodes: &[&Section]) -> Value {
        let mut names: Vec<String> = Vec::new();
        let mut name = |n: &str| -> f64 {
            if let Some(i) = names.iter().position(|x| x == n) {
                return i as f64;
            }
            names.push(n.to_string());
            (names.len() - 1) as f64
        };
        let mut variants = Vec::new();
        let mut ints: Vec<f64> = Vec::new();
        let mut paths: Vec<String> = Vec::new();
        for n in nodes {
            let node_name = n.attr_text("name").unwrap();
            let parent = n.attr_text("parent");
            let parent_index = parent.map_or(-1.0, |p| {
                let p = if p == "." { "" } else { p };
                paths.iter().position(|x| x == p).unwrap() as f64
            });
            paths.push(match parent {
                None => String::new(),
                Some(".") => node_name.to_string(),
                Some(p) => format!("{p}/{node_name}"),
            });
            ints.extend([parent_index, 0.0, n.attr_text("type").map_or(TYPE_INSTANTIATED as f64, &mut name), name(node_name)]);
            ints.push(n.attr("instance").map_or(-1.0, |v| {
                variants.push(v.clone());
                (variants.len() - 1) as f64
            }));
            ints.push(n.props.len() as f64);
            for (k, v) in &n.props {
                ints.push(name(k));
                variants.push(v.clone());
                ints.push((variants.len() - 1) as f64);
            }
            ints.push(0.0);
        }
        let nums = |name: &str, v: Vec<f64>| Value::Call(name.into(), v.into_iter().map(Value::Num).collect());
        let entries = vec![
            ("names", Value::Call("PackedStringArray".into(), names.into_iter().map(Value::Str).collect())),
            ("variants", Value::List(variants)),
            ("node_count", Value::Num(nodes.len() as f64)),
            ("nodes", nums("PackedInt32Array", ints)),
            ("conn_count", Value::Num(0.0)),
            ("conns", nums("PackedInt32Array", Vec::new())),
            ("node_paths", Value::List(Vec::new())),
            ("editable_instances", Value::List(Vec::new())),
            ("version", Value::Num(3.0)),
        ];
        Value::Dict(entries.into_iter().map(|(k, v)| (Value::Str(k.into()), v)).collect())
    }

    struct W<'a> {
        out: Vec<u8>,
        big: bool,
        double: bool,
        ext: Vec<&'a str>,
        sub: Vec<&'a str>,
        strings: Vec<String>,
    }

    impl W<'_> {
        fn u32(&mut self, n: u32) {
            self.out.extend(if self.big { n.to_be_bytes() } else { n.to_le_bytes() });
        }

        fn u32s(&mut self, ns: &[u32]) {
            ns.iter().for_each(|n| self.u32(*n));
        }

        fn u64(&mut self, n: u64) {
            self.out.extend(if self.big { n.to_be_bytes() } else { n.to_le_bytes() });
        }

        fn real(&mut self, v: f64) {
            if self.double { self.u64(v.to_bits()) } else { self.u32((v as f32).to_bits()) }
        }

        fn str(&mut self, s: &str) {
            self.u32(s.len() as u32 + 1);
            self.out.extend(s.as_bytes());
            self.out.push(0);
        }

        fn variant(&mut self, v: &Value) {
            let nums = |args: &[Value]| args.iter().map(|a| a.num().unwrap()).collect::<Vec<_>>();
            match v {
                Value::Nil => self.u32(1),
                Value::Bool(b) => self.u32s(&[2, u32::from(*b)]),
                Value::Num(n) if n.fract() == 0.0 && n.abs() < 1e9 => self.u32s(&[3, *n as i32 as u32]),
                Value::Num(n) if n.fract() == 0.0 && n.abs() < 1e18 => {
                    self.u32(40);
                    self.u64(*n as i64 as u64);
                }
                Value::Num(n) => {
                    self.u32(4);
                    self.real(*n);
                }
                Value::Str(s) => {
                    self.u32(5);
                    self.str(s);
                }
                Value::List(items) => {
                    self.u32(30);
                    self.u32(items.len() as u32);
                    items.iter().for_each(|i| self.variant(i));
                }
                Value::Bytes(b) => {
                    self.u32(31);
                    self.u32(b.len() as u32);
                    self.out.extend(b);
                    self.out.extend(vec![0; b.len().next_multiple_of(4) - b.len()]);
                }
                Value::Dict(entries) => {
                    self.u32(26);
                    self.u32(entries.len() as u32);
                    for (k, v) in entries {
                        self.variant(k);
                        self.variant(v);
                    }
                }
                Value::Call(name, args) => match name.as_str() {
                    "SubResource" | "ExtResource" => {
                        let id = args[0].text().unwrap();
                        let (kind, list) = if name == "SubResource" { (2, &self.sub) } else { (3, &self.ext) };
                        let i = list.iter().position(|x| *x == id).unwrap() as u32;
                        self.u32s(&[24, kind, i]);
                    }
                    "NodePath" => {
                        self.u32(22);
                        let text = args[0].text().unwrap();
                        let parts: Vec<&str> = text.split('/').collect();
                        self.out.extend(if self.big { (parts.len() as u16).to_be_bytes() } else { (parts.len() as u16).to_le_bytes() });
                        self.out.extend([0, 0]);
                        for p in parts {
                            self.u32((p.len() as u32) | 0x8000_0000);
                            self.out.extend(p.as_bytes());
                        }
                    }
                    "Color" => {
                        self.u32(20);
                        nums(args).into_iter().for_each(|n| self.u32((n as f32).to_bits()));
                    }
                    "PackedStringArray" => {
                        self.u32(34);
                        self.u32(args.len() as u32);
                        args.iter().for_each(|a| self.str(a.text().unwrap()));
                    }
                    "PackedInt32Array" => {
                        self.u32(32);
                        self.u32(args.len() as u32);
                        nums(args).into_iter().for_each(|n| self.u32(n as i32 as u32));
                    }
                    "PackedFloat32Array" => {
                        self.u32(33);
                        self.u32(args.len() as u32);
                        nums(args).into_iter().for_each(|n| self.u32((n as f32).to_bits()));
                    }
                    _ => {
                        let (tag, per) = match name.as_str() {
                            "Vector2" => (10, 0),
                            "Vector3" => (12, 0),
                            "AABB" => (15, 0),
                            "Transform3D" => (17, 0),
                            "PackedVector2Array" => (37, 2),
                            "PackedVector3Array" => (35, 3),
                            other => panic!("the test encoder has no {other}"),
                        };
                        self.u32(tag);
                        if per > 0 {
                            self.u32((args.len() / per) as u32);
                        }
                        nums(args).into_iter().for_each(|n| self.real(n));
                    }
                },
            }
        }
    }

    /// Wraps a resource body the way `RSCC` files are: zstd blocks of `block` bytes.
    pub fn compress(body: &[u8], block: usize) -> Vec<u8> {
        let blocks: Vec<Vec<u8>> = body.chunks(block).map(|c| ruzstd::encoding::compress_to_vec(c, ruzstd::encoding::CompressionLevel::Fastest)).collect();
        let mut out = b"RSCC".to_vec();
        for n in [ZSTD, block as u32, body.len() as u32] {
            out.extend(n.to_le_bytes());
        }
        let count = body.len() / block + 1;
        for k in 0..count {
            out.extend((blocks.get(k).map_or(0, Vec::len) as u32).to_le_bytes());
        }
        blocks.iter().for_each(|b| out.extend(b));
        out
    }

    const SCENE: &str = r#"[gd_scene format=3]

[ext_resource type="PackedScene" path="res://Levels/Utilities/kill_box.tscn" id="kill"]

[sub_resource type="StandardMaterial3D" id="pink"]
albedo_color = Color(1, 0, 0.5, 1)

[sub_resource type="BoxShape3D" id="box"]
size = Vector3(4, 2, 4)

[node name="Map" type="Node3D"]

[node name="Deck" type="CSGBox3D" parent="."]
transform = Transform3D(1, 0, 0, 0, 1, 0, 0, 0, 1, 1.5, -2, 3)
use_collision = true
size = Vector3(2, 1, 4)
material = SubResource("pink")

[node name="StartZone" type="Area3D" parent="."]
metadata/sdk_zone = "start"

[node name="Shape" type="CollisionShape3D" parent="StartZone"]
shape = SubResource("box")
path = NodePath("../Deck")

[node name="KillBox" parent="." instance=ExtResource("kill")]
"#;

    fn sections() -> Vec<Section> {
        tscn::parse(SCENE).unwrap()
    }

    fn same(a: &[Section], b: &[Section]) {
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b).skip(1) {
            assert_eq!(x.tag, y.tag);
            for (k, v) in &x.attrs {
                match k.as_str() {
                    "id" if x.tag == "ext_resource" => {}
                    "instance" => assert!(y.attr(k).and_then(|v| v.call("ExtResource")).is_some()),
                    _ => assert_eq!(y.attr(k), Some(v), "{} {k}", x.tag),
                }
            }
            assert_eq!(x.props, y.props, "{}", x.tag);
        }
    }

    #[test]
    fn reads_a_scene_in_every_byte_order_and_precision() {
        let text = sections();
        for (big, double) in [(false, false), (true, false), (false, true), (true, true)] {
            let back = parse(&encode(&text, big, double)).unwrap();
            assert_eq!(back[0].tag, "gd_scene");
            same(&text, &back);
        }
    }

    #[test]
    fn reads_compressed_resources() {
        let body = encode_body(&sections(), true, false);
        let back = parse(&compress(&body, 64)).unwrap();
        same(&sections(), &back);
        let mut exact = body.clone();
        exact.resize(body.len().next_multiple_of(64), 0);
        same(&sections(), &parse(&compress(&exact, 64)).unwrap());
    }

    #[test]
    fn reads_a_material_resource() {
        let tres = tscn::parse("[gd_resource type=\"StandardMaterial3D\" format=3]\n\n[resource]\nalbedo_color = Color(0, 1, 0, 1)\n").unwrap();
        let back = parse(&encode(&tres, false, false)).unwrap();
        assert_eq!((back[0].tag.as_str(), back[0].attr_text("type")), ("gd_resource", Some("StandardMaterial3D")));
        assert_eq!(back[1].tag, "resource");
        assert_eq!(back[1].prop("albedo_color").map(Value::numbers), Some(vec![0.0, 1.0, 0.0, 1.0]));
    }

    #[test]
    fn hostile_input_is_an_error() {
        let rsrc = encode(&sections(), false, false);
        for n in 0..rsrc.len() - 4 {
            assert!(parse(&rsrc[..n]).is_err(), "cut at {n}");
        }
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        for round in 0..4000 {
            let mut bad = rsrc.clone();
            for _ in 0..4 {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let at = (seed % bad.len() as u64) as usize;
                bad[at] = (seed >> 32) as u8;
            }
            let _ = parse(&bad);
            if round % 16 == 0 {
                let _ = parse(&compress(&bad[4..], 64));
            }
        }
        let mut huge = rsrc.clone();
        huge[0x68..0x6c].copy_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
        assert!(parse(&huge).is_err(), "a string count past the end");
        let mut newer = rsrc.clone();
        newer[20..24].copy_from_slice(&7u32.to_le_bytes());
        assert!(parse(&newer).unwrap_err().contains("format 7"));
        let mut packed = compress(&rsrc[4..], 64);
        packed[4] = 1;
        assert!(parse(&packed).unwrap_err().contains("not zstd"));
    }

    #[test]
    fn an_unknown_value_ends_only_its_resource() {
        let mut text = sections();
        text[2].props.push(("later".into(), Value::Num(1.0)));
        let mut rsrc = encode(&text, false, false);
        let color = rsrc.windows(8).position(|w| w == [20, 0, 0, 0, 0, 0, 0x80, 0x3f]).unwrap();
        rsrc[color..color + 4].copy_from_slice(&99u32.to_le_bytes());
        let back = parse(&rsrc).unwrap();
        let pink = back.iter().find(|s| s.attr_text("id") == Some("pink")).unwrap();
        assert!(pink.props.is_empty(), "the material stops at the unknown value");
        assert!(back.iter().any(|s| s.attr_text("name") == Some("Deck")), "the scene still reads");
    }
}
