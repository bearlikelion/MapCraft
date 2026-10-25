//! Reads Valve BSP files (versions 19 to 21) into MapCraft brushes and entities.
//! Brushes are rebuilt from their planes, since a BSP keeps no editable geometry.

use std::borrow::Cow;
use std::io::Read;

use glam::{DMat3, DVec3};
use lzma_rs::decompress::{Options, UnpackedSize};

use crate::mapcraft::plugin::types::{Brush, Entity, Face, Output, Vec3};

const ENTITIES: usize = 0;
const PLANES: usize = 1;
const TEXDATA: usize = 2;
const NODES: usize = 5;
const TEXINFO: usize = 6;
const LEAFS: usize = 10;
const MODELS: usize = 14;
const LEAFBRUSHES: usize = 17;
const BRUSHES: usize = 18;
const BRUSHSIDES: usize = 19;
const DISPINFO: usize = 26;
const GAME_LUMP: usize = 35;
const TEXDATA_STRING_DATA: usize = 43;
const TEXDATA_STRING_TABLE: usize = 44;
const HEADER: usize = 8 + 64 * 16 + 4;
const STATIC_PROPS: u32 = u32::from_be_bytes(*b"sprp");
const NODRAW: &str = "tools/toolsnodraw";
const MAX_LUMP: usize = 256 << 20;
/// Bevels take curved brushes to about 500 sides; far more means a corrupt file, and clipping is quadratic.
const MAX_SIDES: usize = 2048;
/// Faces thinner than this come from planes that only touch the brush at an edge or corner.
const MIN_WIDTH: f64 = 0.01;
const CLIP_EPSILON: f64 = 1e-6;

/// What an import produced, with counts for the log.
pub struct Import {
    pub output: Output,
    pub brush_entities: usize,
    pub static_props: usize,
    pub displacements: usize,
    pub dropped_brushes: usize,
    pub warnings: Vec<String>,
}

fn bytes<const N: usize>(b: &[u8], at: usize) -> [u8; N] {
    at.checked_add(N).and_then(|end| b.get(at..end)).and_then(|s| s.try_into().ok()).unwrap_or([0; N])
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(bytes(b, at))
}

fn i16_at(b: &[u8], at: usize) -> i16 {
    i16::from_le_bytes(bytes(b, at))
}

fn i32_at(b: &[u8], at: usize) -> i32 {
    i32::from_le_bytes(bytes(b, at))
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes(b, at))
}

fn f32_at(b: &[u8], at: usize) -> f64 {
    f64::from(f32::from_le_bytes(bytes(b, at)))
}

fn vec_at(b: &[u8], at: usize) -> DVec3 {
    DVec3::new(f32_at(b, at), f32_at(b, at + 4), f32_at(b, at + 8))
}

fn index(v: i32) -> usize {
    usize::try_from(v).unwrap_or(usize::MAX)
}

/// The NUL-terminated string at the start of `b`.
fn c_str(b: &[u8]) -> Cow<'_, str> {
    String::from_utf8_lossy(b.split(|&c| c == 0).next().unwrap_or_default())
}

/// Valve's LZMA block: `LZMA`, uncompressed size, compressed size, 5 property bytes, then the stream.
fn unlzma(raw: &[u8]) -> Result<Vec<u8>, String> {
    let size = u32_at(raw, 4) as usize;
    if size > MAX_LUMP {
        return Err(format!("{size} bytes uncompressed is more than MapCraft reads"));
    }
    let props = raw.get(12..17).ok_or("truncated LZMA header")?;
    let stream = raw.get(17..).unwrap_or_default();
    let options = Options { unpacked_size: UnpackedSize::UseProvided(Some(size as u64)), memlimit: Some(MAX_LUMP), allow_incomplete: false };
    let mut out = Vec::with_capacity(size);
    lzma_rs::lzma_decompress_with_options(&mut props.chain(stream), &mut out, &options).map_err(|e| format!("LZMA: {e}"))?;
    Ok(out)
}

struct Bsp<'a> {
    file: &'a [u8],
    /// Offset, length, version and uncompressed length (0 when stored plain) of each lump.
    lumps: Vec<[i32; 4]>,
}

impl<'a> Bsp<'a> {
    fn open(file: &'a [u8]) -> Result<Self, String> {
        if file.len() < HEADER || !file.starts_with(b"VBSP") {
            return Err("not a Source engine BSP file".into());
        }
        let version = i32_at(file, 4);
        if !(19..=21).contains(&version) {
            return Err(format!("BSP version {version} is not supported; versions 19 to 21 are (Counter-Strike: Source, Half-Life 2, Garry's Mod)"));
        }
        let lumps = (0..64).map(|i| [0, 4, 8, 12].map(|k| i32_at(file, 8 + i * 16 + k))).collect();
        Ok(Bsp { file, lumps })
    }

    fn lump(&self, id: usize) -> Result<Cow<'a, [u8]>, String> {
        let Some(&[offset, length, _, uncompressed]) = self.lumps.get(id) else { return Ok(Cow::Borrowed(&[])) };
        if length <= 0 {
            return Ok(Cow::Borrowed(&[]));
        }
        let (offset, length) = (index(offset), index(length));
        let raw = offset.checked_add(length).and_then(|end| self.file.get(offset..end)).ok_or_else(|| format!("lump {id} lies outside the file"))?;
        if uncompressed > 0 && raw.starts_with(b"LZMA") {
            return unlzma(raw).map(Cow::Owned).map_err(|e| format!("lump {id}: {e}"));
        }
        Ok(Cow::Borrowed(raw))
    }

    fn lump_version(&self, id: usize) -> i32 {
        self.lumps.get(id).map_or(0, |l| l[2])
    }
}

/// Lowercase with forward slashes and no `materials/` prefix, as SourceGD names materials.
fn material_name(raw: &str) -> String {
    let name = raw.to_ascii_lowercase().replace('\\', "/");
    let name = name.trim_start_matches('/');
    name.strip_prefix("materials/").unwrap_or(name).to_string()
}

/// The keyvalues of each entity in the entity lump, in file order; quotes never occur inside values.
fn parse_entities(text: &str) -> Vec<Vec<(String, String)>> {
    let mut out = Vec::new();
    let mut entity: Option<Vec<(String, String)>> = None;
    let mut key: Option<&str> = None;
    for (i, part) in text.split('"').enumerate() {
        if i % 2 == 1 {
            if let Some(keys) = entity.as_mut() {
                match key.take() {
                    Some(k) => keys.push((k.to_string(), part.to_string())),
                    None => key = Some(part),
                }
            }
            continue;
        }
        for c in part.chars().filter(|c| matches!(c, '{' | '}')) {
            out.extend(entity.take().filter(|keys| !keys.is_empty()));
            key = None;
            if c == '{' {
                entity = Some(Vec::new());
            }
        }
    }
    out.extend(entity.filter(|keys| !keys.is_empty()));
    out
}

/// The last value of `name`, which is the one the engine keeps.
fn key<'k>(keys: &'k [(String, String)], name: &str) -> Option<&'k str> {
    keys.iter().rev().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
}

fn vector(text: Option<&str>) -> DVec3 {
    let mut parts = text.unwrap_or_default().split_whitespace().map(|s| s.parse::<f64>().ok().filter(|v| v.is_finite()).unwrap_or(0.0));
    DVec3::new(parts.next().unwrap_or(0.0), parts.next().unwrap_or(0.0), parts.next().unwrap_or(0.0))
}

/// Source's pitch, yaw, roll in degrees as a rotation matrix (AngleMatrix).
fn rotation(angles: DVec3) -> DMat3 {
    let [pitch, yaw, roll] = angles.to_array().map(f64::to_radians);
    DMat3::from_rotation_z(yaw) * DMat3::from_rotation_y(pitch) * DMat3::from_rotation_x(roll)
}

fn number(v: f64) -> String {
    format!("{}", v as f32 + 0.0)
}

fn v3(p: DVec3) -> Vec3 {
    Vec3 { x: p.x, y: p.y, z: p.z }
}

/// A unit vector on the plane, as VBSP's BaseWindingForPlane picks it.
fn tangent(normal: DVec3) -> DVec3 {
    let a = normal.abs();
    let up = if a.z > a.x && a.z > a.y { DVec3::X } else { DVec3::Z };
    (up - normal * up.dot(normal)).normalize_or_zero()
}

/// A huge square on the plane, clockwise seen from the front.
fn base_winding(normal: DVec3, dist: f64) -> Vec<DVec3> {
    let up = tangent(normal) * 65536.0;
    let right = up.cross(normal);
    let origin = normal * dist;
    vec![origin - right + up, origin + right + up, origin + right - up, origin - right - up]
}

/// The part of the winding behind the plane.
fn chop(winding: &[DVec3], normal: DVec3, dist: f64) -> Vec<DVec3> {
    let mut out = Vec::with_capacity(winding.len() + 1);
    for (&a, &b) in winding.iter().zip(winding.iter().cycle().skip(1)) {
        let (da, db) = (normal.dot(a) - dist, normal.dot(b) - dist);
        if da <= CLIP_EPSILON {
            out.push(a);
        }
        if (da > CLIP_EPSILON && db < -CLIP_EPSILON) || (da < -CLIP_EPSILON && db > CLIP_EPSILON) {
            out.push(a + (b - a) * (da / (da - db)));
        }
    }
    out
}

/// Twice the area over the perimeter: about the width of a long sliver.
fn width(winding: &[DVec3]) -> f64 {
    let Some(&first) = winding.first() else { return 0.0 };
    let pairs = || winding.iter().zip(winding.iter().cycle().skip(1));
    let area: DVec3 = pairs().map(|(&a, &b)| (a - first).cross(b - first)).sum();
    let perimeter: f64 = pairs().map(|(&a, &b)| a.distance(b)).sum();
    if perimeter > 0.0 { area.length() / perimeter } else { 0.0 }
}

/// The convex polygon each plane contributes to the brush they bound; empty when it clips away.
pub fn windings(planes: &[(DVec3, f64)]) -> Vec<Vec<DVec3>> {
    planes
        .iter()
        .enumerate()
        .map(|(i, &(normal, dist))| {
            let mut w = base_winding(normal, dist);
            for (j, &(n, d)) in planes.iter().enumerate() {
                if j != i && w.len() >= 3 {
                    w = chop(&w, n, d);
                }
            }
            if w.len() >= 3 && width(&w) >= MIN_WIDTH { w } else { Vec::new() }
        })
        .collect()
}

struct Map {
    planes: Vec<(DVec3, f64)>,
    /// Material of each texinfo.
    materials: Vec<String>,
    /// First side and side count of each brush.
    brushes: Vec<(usize, usize)>,
    /// Plane, texinfo and bevel flag of each brush side.
    sides: Vec<(usize, i16, bool)>,
}

impl Map {
    /// The brush in map space; `None` when it has fewer than four faces left.
    fn brush(&self, index: usize, rotation: DMat3, origin: DVec3) -> Option<Brush> {
        let &(first, count) = self.brushes.get(index)?;
        if count > MAX_SIDES {
            return None;
        }
        let mut planes = Vec::with_capacity(count);
        let mut sides: Vec<(usize, i16)> = Vec::with_capacity(count);
        for &(plane, texinfo, bevel) in self.sides.get(first..first.checked_add(count)?)? {
            if bevel || sides.iter().any(|s| s.0 == plane) {
                continue;
            }
            let Some(&p) = self.planes.get(plane) else { continue };
            planes.push(p);
            sides.push((plane, texinfo));
        }
        let faces: Vec<Face> = windings(&planes)
            .iter()
            .zip(&planes)
            .zip(&sides)
            .filter(|((w, _), _)| !w.is_empty())
            .map(|((w, &(normal, dist)), &(_, texinfo))| {
                let center = w.iter().sum::<DVec3>() / w.len() as f64;
                let center = center - normal * (normal.dot(center) - dist);
                let up = tangent(normal);
                let right = up.cross(normal);
                let place = |p: DVec3| v3(rotation * p + origin);
                let material = usize::try_from(texinfo).ok().and_then(|t| self.materials.get(t)).map_or(NODRAW, String::as_str);
                Face { points: (place(center + right * 64.0), place(center), place(center + up * 64.0)), material: material.to_string() }
            })
            .collect();
        (faces.len() >= 4).then_some(Brush::Faces(faces))
    }
}

/// The model each brush belongs to, found by walking every model's node tree down to its leaves.
fn owners(bsp: &Bsp, brushes: usize) -> Result<(Vec<Option<usize>>, usize), String> {
    let nodes: Vec<[i32; 2]> = bsp.lump(NODES)?.chunks_exact(32).map(|r| [i32_at(r, 4), i32_at(r, 8)]).collect();
    let stride = if bsp.lump_version(LEAFS) == 0 { 56 } else { 32 };
    let leafs: Vec<(usize, usize)> = bsp.lump(LEAFS)?.chunks_exact(stride).map(|r| (usize::from(u16_at(r, 24)), usize::from(u16_at(r, 26)))).collect();
    let leafbrushes: Vec<usize> = bsp.lump(LEAFBRUSHES)?.chunks_exact(2).map(|r| usize::from(u16_at(r, 0))).collect();
    let heads: Vec<i32> = bsp.lump(MODELS)?.chunks_exact(48).map(|r| i32_at(r, 36)).collect();
    let mut owner = vec![None; brushes];
    let mut visited = vec![usize::MAX; nodes.len()];
    let mut stack = Vec::new();
    for (model, &head) in heads.iter().enumerate() {
        stack.push(head);
        while let Some(node) = stack.pop() {
            if node < 0 {
                let leaf = index(-(node + 1));
                let Some(&(first, count)) = leafs.get(leaf) else { continue };
                for &b in leafbrushes.get(first..first + count).unwrap_or_default() {
                    if let Some(o) = owner.get_mut(b) {
                        *o = Some(model);
                    }
                }
            } else if let Some(seen) = visited.get_mut(index(node)).filter(|seen| **seen != model) {
                *seen = model;
                stack.extend(nodes.get(index(node)).into_iter().flatten());
            }
        }
    }
    Ok((owner, heads.len()))
}

fn materials(bsp: &Bsp) -> Result<Vec<String>, String> {
    let data = bsp.lump(TEXDATA_STRING_DATA)?;
    let table: Vec<usize> = bsp.lump(TEXDATA_STRING_TABLE)?.chunks_exact(4).map(|r| index(i32_at(r, 0))).collect();
    let names: Vec<String> = bsp
        .lump(TEXDATA)?
        .chunks_exact(32)
        .map(|r| table.get(index(i32_at(r, 12))).and_then(|&at| data.get(at..)).map_or_else(|| NODRAW.to_string(), |s| material_name(&c_str(s))))
        .collect();
    Ok(bsp.lump(TEXINFO)?.chunks_exact(72).map(|r| names.get(index(i32_at(r, 68))).map_or_else(|| NODRAW.to_string(), Clone::clone)).collect())
}

/// `prop_static` entities from the `sprp` game lump; every version keeps the fields read here at the same offsets.
fn static_props(bsp: &Bsp) -> Result<Vec<Entity>, String> {
    let dir = bsp.lump(GAME_LUMP)?;
    let count = index(i32_at(&dir, 0));
    let Some(entry) = dir.get(4..).unwrap_or_default().chunks_exact(16).take(count).find(|e| u32_at(e, 0) == STATIC_PROPS) else {
        return Ok(Vec::new());
    };
    let (flags, offset, length) = (u16_at(entry, 4), index(i32_at(entry, 8)), index(i32_at(entry, 12)));
    let raw = bsp.file.get(offset..).unwrap_or_default();
    let body: Cow<[u8]> = if flags & 1 != 0 && raw.starts_with(b"LZMA") {
        Cow::Owned(unlzma(raw).map_err(|e| format!("static props: {e}"))?)
    } else {
        Cow::Borrowed(raw.get(..length).ok_or("the static prop lump lies outside the file")?)
    };
    let names = index(i32_at(&body, 0));
    let dict: Vec<String> = body.get(4..).unwrap_or_default().chunks_exact(128).take(names).map(|n| c_str(n).to_ascii_lowercase().replace('\\', "/")).collect();
    let mut at = 4 + dict.len() * 128;
    at = at.saturating_add(4).saturating_add(index(i32_at(&body, at)).saturating_mul(2));
    let props = index(i32_at(&body, at));
    if props == 0 {
        return Ok(Vec::new());
    }
    let rest = body.get(at.saturating_add(4)..).unwrap_or_default();
    let stride = rest.len().checked_div(props).unwrap_or(0);
    if dict.len() != names || stride < 56 {
        return Err("the static prop lump is malformed".into());
    }
    let format = |v: DVec3| format!("{} {} {}", number(v.x), number(v.y), number(v.z));
    Ok(rest
        .chunks_exact(stride)
        .take(props)
        .map(|r| Entity {
            classname: "prop_static".into(),
            properties: vec![
                ("model".into(), dict.get(usize::from(u16_at(r, 24))).cloned().unwrap_or_default()),
                ("origin".into(), format(vec_at(r, 0))),
                ("angles".into(), format(vec_at(r, 12))),
                ("skin".into(), i32_at(r, 32).to_string()),
            ],
            brushes: Vec::new(),
        })
        .collect())
}

pub fn import(file: &[u8]) -> Result<Import, String> {
    let bsp = Bsp::open(file)?;
    let map = Map {
        planes: bsp.lump(PLANES)?.chunks_exact(20).map(|r| (vec_at(r, 0), f32_at(r, 12))).collect(),
        materials: materials(&bsp)?,
        brushes: bsp.lump(BRUSHES)?.chunks_exact(12).map(|r| (index(i32_at(r, 0)), index(i32_at(r, 4)))).collect(),
        sides: bsp.lump(BRUSHSIDES)?.chunks_exact(8).map(|r| (usize::from(u16_at(r, 0)), i16_at(r, 2), r.get(6).is_some_and(|&b| b != 0))).collect(),
    };
    let (owner, models) = owners(&bsp, map.brushes.len())?;
    let mut model_brushes = vec![Vec::new(); models];
    for (brush, model) in owner.iter().enumerate() {
        if let Some(list) = model.and_then(|m| model_brushes.get_mut(m)) {
            list.push(brush);
        }
    }
    let mut dropped = 0;
    let mut build = |model: usize, rotation: DMat3, origin: DVec3| -> Vec<Brush> {
        let list = model_brushes.get(model).map_or(&[][..], Vec::as_slice);
        let built: Vec<Brush> = list.iter().filter_map(|&b| map.brush(b, rotation, origin)).collect();
        dropped += list.len() - built.len();
        built
    };
    let brushes = build(0, DMat3::IDENTITY, DVec3::ZERO);

    let text = bsp.lump(ENTITIES)?;
    let mut entities = Vec::new();
    let mut brush_entities = 0;
    for mut keys in parse_entities(&c_str(&text)) {
        let Some(classname) = key(&keys, "classname").map(str::to_string) else { continue };
        if classname.eq_ignore_ascii_case("worldspawn") {
            continue;
        }
        let model = key(&keys, "model").and_then(|m| m.strip_prefix('*')).and_then(|m| m.parse::<usize>().ok()).filter(|&m| m > 0 && m < models);
        let mut entity_brushes = Vec::new();
        if let Some(model) = model {
            entity_brushes = build(model, rotation(vector(key(&keys, "angles"))), vector(key(&keys, "origin")));
            keys.retain(|(k, _)| !k.eq_ignore_ascii_case("model"));
            brush_entities += 1;
        }
        keys.retain(|(k, _)| !k.eq_ignore_ascii_case("classname"));
        entities.push(Entity { classname, properties: keys, brushes: entity_brushes });
    }
    let mut warnings = Vec::new();
    let props = static_props(&bsp).unwrap_or_else(|e| {
        warnings.push(e);
        Vec::new()
    });
    let static_props = props.len();
    entities.extend(props);
    Ok(Import {
        output: Output { brushes, entities },
        brush_entities,
        static_props,
        displacements: bsp.lump(DISPINFO)?.len() / 176,
        dropped_brushes: dropped,
        warnings,
    })
}

#[cfg(test)]
mod explore {
    use super::*;

    fn dv(v: &Vec3) -> DVec3 {
        DVec3::new(v.x, v.y, v.z)
    }

    fn planes_of(b: &Brush) -> Vec<(DVec3, f64)> {
        let Brush::Faces(faces) = b else { panic!() };
        faces.iter().map(|f| { let (p0, p1, p2) = (dv(&f.points.0), dv(&f.points.1), dv(&f.points.2)); let n = (p0 - p1).cross(p2 - p1).normalize(); (n, n.dot(p1)) }).collect()
    }

    fn bounds(b: &Brush) -> (DVec3, DVec3) {
        let pts: Vec<DVec3> = windings(&planes_of(b)).into_iter().flatten().collect();
        (pts.iter().fold(DVec3::MAX, |a, &p| a.min(p)), pts.iter().fold(DVec3::MIN, |a, &p| a.max(p)))
    }

    #[test]
    fn explore() {
        let path = std::env::var("BSP").unwrap();
        let data = std::fs::read(&path).unwrap();
        let bsp = Bsp::open(&data).unwrap();
        let planes: Vec<(DVec3, f64)> = bsp.lump(PLANES).unwrap().chunks_exact(20).map(|r| (vec_at(r, 0), f32_at(r, 12))).collect();
        let brushes: Vec<(usize, usize, u32)> = bsp.lump(BRUSHES).unwrap().chunks_exact(12).map(|r| (index(i32_at(r, 0)), index(i32_at(r, 4)), u32_at(r, 8))).collect();
        let sides: Vec<(usize, i16, bool)> = bsp.lump(BRUSHSIDES).unwrap().chunks_exact(8).map(|r| (usize::from(u16_at(r, 0)), i16_at(r, 2), r[6] != 0)).collect();
        let mats = materials(&bsp).unwrap();
        let (owner, _) = owners(&bsp, brushes.len()).unwrap();
        let map = Map { planes: planes.clone(), materials: mats.clone(), brushes: brushes.iter().map(|b| (b.0, b.1)).collect(), sides: sides.clone() };
        for bi in 0..brushes.len() { if owner[bi].is_some() && map.brush(bi, DMat3::IDENTITY, DVec3::ZERO).is_none() { eprintln!("NONE {bi} owner {:?} sides {}", owner[bi], brushes[bi].1); } }
        let mut shown = 0;
        for (bi, b) in brushes.iter().enumerate() {
            let mut seen = Vec::new();
            let ps: Vec<(DVec3, f64)> = sides[b.0..b.0 + b.1].iter().filter(|s| !s.2).filter(|s| if seen.contains(&s.0) { false } else { seen.push(s.0); true }).map(|s| planes[s.0]).collect();
            let ws = windings(&ps);
            let n = ws.iter().filter(|w| !w.is_empty()).count();
            if n < 4 && owner[bi].is_some() && shown < 6 {
                shown += 1;
                eprintln!("brush {bi} owner {:?} contents {:#x} sides {} nonbevel {} faces {n}", owner[bi], b.2, b.1, ps.len());
                for (k, s) in sides[b.0..b.0 + b.1].iter().enumerate() {
                    let p = planes[s.0];
                    let mut w = base_winding(p.0, p.1);
                    for (j, q) in ps.iter().enumerate() { if *q != p { w = chop(&w, q.0, q.1); } let _ = j; }
                    eprintln!("   side {k} plane {} n {:?} d {} bevel {} mat {:?} pts {} width {:.5}", s.0, p.0, p.1, s.2, mats.get(s.1 as usize), w.len(), if w.len() >= 3 { width(&w) } else { 0.0 });
                }
            }
        }
    }
}
