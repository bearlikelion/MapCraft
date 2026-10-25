//! Reads Valve BSP files (versions 19 to 21) into entities and brushes rebuilt from their planes.

use std::borrow::Cow;
use std::io::Read;

use std::collections::BTreeMap;

use glam::{DMat3, DVec3};
use lzma_rs::decompress::{Options, UnpackedSize};

use crate::mapcraft::plugin::types::{Brush, Entity, Face, Group, Image, MaterialInfo, Output, TexAxis, Vec3};
use crate::materials::{Files, Library, Origin};

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
const PAKFILE: usize = 40;
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
/// Source coordinates stay within 16384, so a face reaching this far belongs to an open brush.
const OPEN: f64 = 32768.0;

/// Source's steepest standable slope: a floor normal's Z is at least this.
const STANDABLE: f64 = 0.7;
/// Faces closer to vertical than this (about 84 degrees) are walls, not ramps.
const WALL: f64 = 0.1;
/// Surf faces smaller than this (square units) are trim, not ramps.
const MIN_RAMP_AREA: f64 = 64.0 * 64.0;
/// RGBA bytes a map's pictures may take at most; maps with many materials get smaller mips.
const PICTURE_BUDGET: usize = 128 << 20;
const GROUPS: [&str; 9] = ["Ramps", "World", "Triggers", "Spawns", "Teleport destinations", "Lights", "Props", "Brush entities", "Other"];

/// What an import produced, with counts for the log.
pub struct Import {
    pub output: Output,
    pub world_brushes: usize,
    pub ramps: usize,
    pub entities: usize,
    pub brush_entities: usize,
    pub static_props: usize,
    pub displacements: usize,
    pub dropped_brushes: usize,
    /// Materials whose texture came from the map's pakfile, from the game's files, or from neither.
    pub textures: (usize, usize, usize),
    pub game_files: bool,
    pub warnings: Vec<String>,
}

fn bytes<const N: usize>(b: &[u8], at: usize) -> [u8; N] {
    at.checked_add(N).and_then(|end| b.get(at..end)).and_then(|s| s.try_into().ok()).unwrap_or([0; N])
}

pub(crate) fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(bytes(b, at))
}

fn i16_at(b: &[u8], at: usize) -> i16 {
    i16::from_le_bytes(bytes(b, at))
}

fn i32_at(b: &[u8], at: usize) -> i32 {
    i32::from_le_bytes(bytes(b, at))
}

pub(crate) fn u32_at(b: &[u8], at: usize) -> u32 {
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

/// A raw LZMA stream of `size` bytes once decompressed, with its 5 property bytes.
pub(crate) fn lzma(props: &[u8], stream: &[u8], size: usize) -> Result<Vec<u8>, String> {
    if size > MAX_LUMP {
        return Err(format!("{size} bytes uncompressed is more than MapCraft reads"));
    }
    let options = Options { unpacked_size: UnpackedSize::UseProvided(Some(size as u64)), memlimit: Some(MAX_LUMP), allow_incomplete: false };
    let mut out = Vec::with_capacity(size);
    lzma_rs::lzma_decompress_with_options(&mut props.chain(stream), &mut out, &options).map_err(|e| format!("LZMA: {e}"))?;
    Ok(out)
}

/// Valve's LZMA block: `LZMA`, uncompressed size, compressed size, 5 property bytes, then the stream.
fn unlzma(raw: &[u8]) -> Result<Vec<u8>, String> {
    lzma(raw.get(12..17).ok_or("truncated LZMA header")?, raw.get(17..).unwrap_or_default(), u32_at(raw, 4) as usize)
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

    /// The lump's size once uncompressed, read from the header alone.
    fn lump_size(&self, id: usize) -> usize {
        self.lumps.get(id).and_then(|l| usize::try_from(if l[3] > 0 { l[3] } else { l[1] }).ok()).unwrap_or(0)
    }
}

/// Lowercase with forward slashes and no `materials/` prefix, as SourceGD names materials.
fn raw_material_name(raw: &str) -> String {
    let name = raw.to_ascii_lowercase().replace('\\', "/");
    let name = name.trim_start_matches('/');
    name.strip_prefix("materials/").unwrap_or(name).to_string()
}

#[cfg(test)]
fn material_name(raw: &str) -> String {
    unpatch(&raw_material_name(raw)).to_string()
}

/// The material a VBSP patch copies: `maps/<map>/<material>_<x>_<y>_<z>` for cubemaps, `..._wvt_patch` for blends.
fn unpatch(name: &str) -> &str {
    let Some((_, inner)) = name.strip_prefix("maps/").and_then(|rest| rest.split_once('/')) else { return name };
    if let Some(base) = inner.strip_suffix("_wvt_patch") {
        return base;
    }
    let mut parts = inner.rsplitn(4, '_');
    let numbers = parts.by_ref().take(3).filter(|p| p.parse::<i32>().is_ok()).count();
    match parts.next() {
        Some(base) if numbers == 3 => base,
        _ => name,
    }
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

fn area(winding: &[DVec3]) -> f64 {
    let Some(&first) = winding.first() else { return 0.0 };
    winding.iter().zip(winding.iter().cycle().skip(1)).map(|(&a, &b)| (a - first).cross(b - first)).sum::<DVec3>().length() / 2.0
}

/// Twice the area over the perimeter: about the width of a long sliver.
fn width(winding: &[DVec3]) -> f64 {
    let perimeter: f64 = winding.iter().zip(winding.iter().cycle().skip(1)).map(|(&a, &b)| a.distance(b)).sum();
    if perimeter > 0.0 { 2.0 * area(winding) / perimeter } else { 0.0 }
}

/// A texture axis in map space; `v` holds texels per unit and `w` the offset in texels.
fn tex_axis(rotation: DMat3, origin: DVec3, (v, w): (DVec3, f64)) -> Option<TexAxis> {
    let a = rotation * v;
    let len = a.length();
    (len > 1e-9).then(|| TexAxis { axis: v3(a / len), offset: w - origin.dot(a), scale: 1.0 / len })
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

struct Texinfo {
    material: String,
    s: (DVec3, f64),
    t: (DVec3, f64),
}

struct Map {
    planes: Vec<(DVec3, f64)>,
    texinfos: Vec<Texinfo>,
    /// First side and side count of each brush.
    brushes: Vec<(usize, usize)>,
    /// Plane, texinfo and bevel flag of each brush side.
    sides: Vec<(usize, i16, bool)>,
}

impl Map {
    /// The brush in map space and whether it has a surfable face; `None` when it has fewer than four faces left or is open.
    fn brush(&self, index: usize, rotation: DMat3, origin: DVec3) -> Option<(Brush, bool)> {
        let &(first, count) = self.brushes.get(index)?;
        if count > MAX_SIDES {
            return None;
        }
        let all = self.sides.get(first..first.checked_add(count)?)?;
        // A bevel now and then stands in for a real face, so bevels come back when the brush is open without them.
        let (planes, sides, polygons) = [false, true]
            .into_iter()
            .map(|bevels| {
                let mut planes = Vec::with_capacity(count);
                let mut sides: Vec<(usize, i16)> = Vec::with_capacity(count);
                for &(plane, texinfo, bevel) in all {
                    if let Some(&p) = self.planes.get(plane).filter(|_| (bevels || !bevel) && !sides.iter().any(|s| s.0 == plane)) {
                        planes.push(p);
                        sides.push((plane, texinfo));
                    }
                }
                let w = windings(&planes);
                (planes, sides, w)
            })
            .find(|(_, _, w)| w.iter().flatten().all(|p| p.abs().max_element() < OPEN))?;
        let mut surf = false;
        let faces: Vec<Face> = polygons
            .iter()
            .zip(&planes)
            .zip(&sides)
            .filter(|((w, _), _)| !w.is_empty())
            .map(|((w, &(normal, dist)), &(_, texinfo))| {
                let z = (rotation * normal).z;
                surf |= z > WALL && z < STANDABLE && area(w) >= MIN_RAMP_AREA;
                let center = w.iter().sum::<DVec3>() / w.len() as f64;
                let center = center - normal * (normal.dot(center) - dist);
                let up = tangent(normal);
                let right = up.cross(normal);
                let place = |p: DVec3| v3(rotation * p + origin);
                let info = usize::try_from(texinfo).ok().and_then(|t| self.texinfos.get(t));
                Face {
                    points: (place(center + right * 64.0), place(center), place(center + up * 64.0)),
                    material: info.map_or(NODRAW, |i| i.material.as_str()).to_string(),
                    uv: info.and_then(|i| tex_axis(rotation, origin, i.s).zip(tex_axis(rotation, origin, i.t))),
                }
            })
            .collect();
        (faces.len() >= 4).then_some((Brush::Faces(faces), surf))
    }
}

/// The model each brush belongs to, found by walking every model's node tree down to its leaves.
fn owners(bsp: &Bsp, brushes: usize) -> Result<(Vec<Option<usize>>, usize), String> {
    let nodes: Vec<[i32; 2]> = bsp.lump(NODES)?.as_chunks::<32>().0.iter().map(|r| [i32_at(r, 4), i32_at(r, 8)]).collect();
    let stride = if bsp.lump_version(LEAFS) == 0 { 56 } else { 32 };
    let leafs: Vec<(usize, usize)> = bsp.lump(LEAFS)?.chunks_exact(stride).map(|r| (usize::from(u16_at(r, 24)), usize::from(u16_at(r, 26)))).collect();
    let leafbrushes: Vec<usize> = bsp.lump(LEAFBRUSHES)?.as_chunks::<2>().0.iter().map(|r| usize::from(u16_at(r, 0))).collect();
    let heads: Vec<i32> = bsp.lump(MODELS)?.as_chunks::<48>().0.iter().map(|r| i32_at(r, 36)).collect();
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

/// Every texinfo, and each texdata's material, the name the map stores it under and its reflectivity (linear color).
fn texinfos(bsp: &Bsp) -> Result<(Vec<Texinfo>, Vec<(String, String, DVec3)>), String> {
    let data = bsp.lump(TEXDATA_STRING_DATA)?;
    let table: Vec<usize> = bsp.lump(TEXDATA_STRING_TABLE)?.as_chunks::<4>().0.iter().map(|r| index(i32_at(r, 0))).collect();
    let texdata: Vec<(String, String, DVec3)> = bsp
        .lump(TEXDATA)?
        .as_chunks::<32>()
        .0
        .iter()
        .map(|r| {
            let raw = table.get(index(i32_at(r, 12))).and_then(|&at| data.get(at..)).map_or_else(|| NODRAW.to_string(), |s| raw_material_name(&c_str(s)));
            (unpatch(&raw).to_string(), raw, vec_at(r, 0))
        })
        .collect();
    let infos = bsp
        .lump(TEXINFO)?
        .as_chunks::<72>()
        .0
        .iter()
        .map(|r| Texinfo {
            material: texdata.get(index(i32_at(r, 68))).map_or_else(|| NODRAW.to_string(), |t| t.0.clone()),
            s: (vec_at(r, 0), f32_at(r, 12)),
            t: (vec_at(r, 16), f32_at(r, 28)),
        })
        .collect();
    Ok((infos, texdata))
}

fn srgb(linear: DVec3) -> (u8, u8, u8) {
    let [r, g, b] = linear.to_array().map(|c| (c.clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0).round() as u8);
    (r, g, b)
}

fn group_of(classname: &str) -> &'static str {
    let c = classname.to_ascii_lowercase();
    match () {
        _ if c.starts_with("trigger_") => "Triggers",
        _ if c.starts_with("info_player_") => "Spawns",
        _ if c == "info_teleport_destination" => "Teleport destinations",
        _ if c.starts_with("light") || c == "env_sun" => "Lights",
        _ if c.starts_with("prop_") => "Props",
        _ if c.starts_with("func_") => "Brush entities",
        _ => "Other",
    }
}

/// `prop_static` entities from the `sprp` game lump; every version keeps the fields read here at the same offsets.
fn static_props(bsp: &Bsp) -> Result<Vec<Entity>, String> {
    let dir = bsp.lump(GAME_LUMP)?;
    let count = index(i32_at(&dir, 0));
    let Some(entry) = dir.get(4..).unwrap_or_default().as_chunks::<16>().0.iter().take(count).find(|e| u32_at(*e, 0) == STATIC_PROPS) else {
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
    let dict: Vec<String> =
        body.get(4..).unwrap_or_default().as_chunks::<128>().0.iter().take(names).map(|n| c_str(n).to_ascii_lowercase().replace('\\', "/")).collect();
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

pub fn import(file: &[u8], files: &dyn Files) -> Result<Import, String> {
    let bsp = Bsp::open(file)?;
    let (texinfos, texdata) = texinfos(&bsp)?;
    let map = Map {
        planes: bsp.lump(PLANES)?.as_chunks::<20>().0.iter().map(|r| (vec_at(r, 0), f32_at(r, 12))).collect(),
        texinfos,
        brushes: bsp.lump(BRUSHES)?.as_chunks::<12>().0.iter().map(|r| (index(i32_at(r, 0)), index(i32_at(r, 4)))).collect(),
        sides: bsp.lump(BRUSHSIDES)?.as_chunks::<8>().0.iter().map(|r| (usize::from(u16_at(r, 0)), i16_at(r, 2), r.get(6).is_some_and(|&b| b != 0))).collect(),
    };
    let (owner, models) = owners(&bsp, map.brushes.len())?;
    let mut model_brushes = vec![Vec::new(); models];
    for (brush, model) in owner.iter().enumerate() {
        if let Some(list) = model.and_then(|m| model_brushes.get_mut(m)) {
            list.push(brush);
        }
    }
    let mut dropped = 0;
    let mut build = |model: usize, rotation: DMat3, origin: DVec3| -> Vec<(Brush, bool)> {
        let list = model_brushes.get(model).map_or(&[][..], Vec::as_slice);
        let built: Vec<(Brush, bool)> = list.iter().filter_map(|&b| map.brush(b, rotation, origin)).collect();
        dropped += list.len() - built.len();
        built
    };
    let world = build(0, DMat3::IDENTITY, DVec3::ZERO);

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
            entity_brushes = build(model, rotation(vector(key(&keys, "angles"))), vector(key(&keys, "origin"))).into_iter().map(|(b, _)| b).collect();
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

    let used: BTreeMap<&str, ()> = world
        .iter()
        .map(|(b, _)| b)
        .chain(entities.iter().flat_map(|e| &e.brushes))
        .flat_map(|b| match b {
            Brush::Faces(faces) => faces.as_slice(),
            Brush::Hull(_) => &[],
        })
        .map(|f| (f.material.as_str(), ()))
        .collect();
    let pak = bsp.lump(PAKFILE).unwrap_or_else(|e| {
        warnings.push(format!("embedded files: {e}"));
        Cow::Borrowed(&[])
    });
    let library = Library::new(&pak, files);
    let side = [1024, 512, 256, 128].into_iter().find(|s| used.len() * s * s * 4 <= PICTURE_BUDGET).unwrap_or(64);
    let mut textures = (0, 0, 0);
    let materials = used
        .into_keys()
        .map(|name| {
            // A map may embed only VBSP's patched copy of a material, so the copies are the fallback.
            let copies = texdata.iter().filter(|t| t.0 == name && t.1 != name);
            let picture = library.picture(name, side).or_else(|| copies.into_iter().find_map(|t| library.picture(&t.1, side)));
            match picture.as_ref().map(|p| p.1) {
                Some(Origin::Map) => textures.0 += 1,
                Some(Origin::Game) => textures.1 += 1,
                None => textures.2 += 1,
            }
            MaterialInfo {
                name: name.to_string(),
                image: None,
                color: texdata.iter().find(|t| t.0 == name).map(|t| srgb(t.2)),
                pixels: picture.map(|((width, height, rgba), _)| Image { width, height, rgba }),
            }
        })
        .collect();

    let (world_brushes, ramps) = (world.len(), world.iter().filter(|b| b.1).count());
    let entity_count = entities.len();
    let mut groups: Vec<Group> = GROUPS.iter().map(|&name| Group { name: name.into(), brushes: Vec::new(), entities: Vec::new() }).collect();
    let group = |name: &str| GROUPS.iter().position(|&g| g == name).unwrap_or(GROUPS.len() - 1);
    let (ramps_at, world_at) = (group("Ramps"), group("World"));
    let placed: Vec<(usize, Entity)> = entities.into_iter().map(|e| (group(group_of(&e.classname)), e)).collect();
    for (brush, surf) in world {
        if let Some(g) = groups.get_mut(if surf { ramps_at } else { world_at }) {
            g.brushes.push(brush);
        }
    }
    for (at, entity) in placed {
        if let Some(g) = groups.get_mut(at) {
            g.entities.push(entity);
        }
    }
    groups.retain(|g| !g.brushes.is_empty() || !g.entities.is_empty());
    Ok(Import {
        output: Output { brushes: Vec::new(), entities: Vec::new(), groups, materials },
        world_brushes,
        ramps,
        entities: entity_count,
        brush_entities,
        static_props,
        displacements: bsp.lump_size(DISPINFO) / 176,
        dropped_brushes: dropped,
        textures,
        game_files: library.has_game_files(),
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Seek, SeekFrom};
    use std::path::PathBuf;
    use std::time::Instant;

    use super::*;

    const SURF: &str = "/mnt/Storage/Gamedev/SurfsUp/SurfMaps/BSPs/surf_utopia_njv.bsp";
    const LZMA: &str = "/mnt/Storage/Gamedev/SurfsUp/SurfMaps/BSPs/surf_fiellu_ksf.bsp";
    const EMERALD: &str = "/mnt/Storage/Gamedev/SurfsUp/SurfMaps/BSPs/surf_emerald.bsp";
    const LARGEST: &str = "/mnt/Storage/Gamedev/SurfsUp/BhopMaps/BSPs/kz_bhop_badg3s.bsp";
    const CSS: &str = "/mnt/Storage/Gamedev/SurfsUp/CSS-Windows";

    /// Game files under the given folders, like the SDK's `cstrike` and `hl2` roots.
    struct Disk(Vec<PathBuf>);

    impl Files for Disk {
        fn read(&self, path: &str) -> Option<Vec<u8>> {
            self.0.iter().find_map(|root| std::fs::read(root.join(path)).ok())
        }

        fn read_range(&self, path: &str, offset: u64, length: u64) -> Option<Vec<u8>> {
            self.0.iter().find_map(|root| {
                let mut file = std::fs::File::open(root.join(path)).ok()?;
                file.seek(SeekFrom::Start(offset)).ok()?;
                let mut out = vec![0; length as usize];
                file.read_exact(&mut out).ok()?;
                Some(out)
            })
        }
    }

    const NO_FILES: Disk = Disk(Vec::new());

    fn read(path: &str) -> Option<Vec<u8>> {
        std::fs::read(path).map_err(|_| eprintln!("skipped: {path} is missing")).ok()
    }

    fn dv(v: &Vec3) -> DVec3 {
        DVec3::new(v.x, v.y, v.z)
    }

    fn group<'a>(import: &'a Import, name: &str) -> Option<&'a Group> {
        import.output.groups.iter().find(|g| g.name == name)
    }

    fn world(import: &Import) -> Vec<&Brush> {
        ["Ramps", "World"].iter().filter_map(|n| group(import, n)).flat_map(|g| &g.brushes).collect()
    }

    fn entities(import: &Import) -> Vec<&Entity> {
        import.output.groups.iter().flat_map(|g| &g.entities).collect()
    }

    fn planes_of(brush: &Brush) -> Vec<(DVec3, f64)> {
        let Brush::Faces(faces) = brush else { panic!("hull brush") };
        faces
            .iter()
            .map(|f| {
                let (p0, p1, p2) = (dv(&f.points.0), dv(&f.points.1), dv(&f.points.2));
                let n = (p0 - p1).cross(p2 - p1).normalize();
                (n, n.dot(p1))
            })
            .collect()
    }

    /// At least four faces survive clipping by the others, they close up, and every normal points away from the middle.
    fn assert_closed(brush: &Brush) {
        let planes = planes_of(brush);
        let ws: Vec<Vec<DVec3>> = windings(&planes).into_iter().filter(|w| !w.is_empty()).collect();
        assert!(ws.len() >= 4 && ws.iter().flatten().all(|p| p.abs().max_element() < OPEN), "open brush");
        let area: DVec3 = ws.iter().flat_map(|w| w.iter().zip(w.iter().cycle().skip(1)).map(|(a, b)| a.cross(*b))).sum();
        let total: f64 = ws.iter().map(|w| width(w) * w.iter().zip(w.iter().cycle().skip(1)).map(|(a, b)| a.distance(*b)).sum::<f64>()).sum();
        assert!(area.length() <= total * 1e-6, "open brush: {area} over {total}");
        let points: Vec<DVec3> = ws.into_iter().flatten().collect();
        let middle = points.iter().sum::<DVec3>() / points.len() as f64;
        assert!(planes.iter().all(|(n, d)| n.dot(middle) < *d), "a face points inward");
    }

    fn texinfo(material: &str) -> Texinfo {
        Texinfo { material: material.into(), s: (DVec3::X * 0.25, 8.0), t: (DVec3::NEG_Z * 0.5, -3.0) }
    }

    #[test]
    fn cube_drops_a_plane_touching_an_edge() {
        let mut planes: Vec<(DVec3, f64)> = [DVec3::X, DVec3::NEG_X, DVec3::Y, DVec3::NEG_Y, DVec3::Z, DVec3::NEG_Z].map(|n| (n, 64.0)).to_vec();
        planes.push((DVec3::new(1.0, 1.0, 0.0).normalize(), 128.0 / 2f64.sqrt()));
        let ws = windings(&planes);
        assert_eq!(ws.iter().filter(|w| !w.is_empty()).count(), 6);
        assert!(ws[6].is_empty());
        let map = Map { planes, texinfos: vec![texinfo("dev/a")], brushes: vec![(0, 7)], sides: (0..7).map(|p| (p, 0, false)).collect() };
        let (rot, origin) = (rotation(DVec3::new(0.0, 90.0, 0.0)), DVec3::new(100.0, 0.0, 0.0));
        let (brush, surf) = map.brush(0, rot, origin).unwrap();
        assert!(!surf);
        assert_closed(&brush);
        let Brush::Faces(faces) = &brush else { panic!() };
        assert_eq!(faces.len(), 6);
        assert!(faces.iter().all(|f| f.material == "dev/a" && (dv(&f.points.1) - origin).abs().max_element() <= 64.0 + 1e-9));
        // A point keeps its texel coordinates once the brush is rotated and moved.
        let local = DVec3::new(64.0, 10.0, 20.0);
        let (u, v) = faces[0].uv.as_ref().unwrap();
        let placed = rot * local + origin;
        assert!((placed.dot(dv(&u.axis)) / u.scale + u.offset - (local.dot(DVec3::X * 0.25) + 8.0)).abs() < 1e-9);
        assert!((placed.dot(dv(&v.axis)) / v.scale + v.offset - (local.dot(DVec3::NEG_Z * 0.5) - 3.0)).abs() < 1e-9);
    }

    #[test]
    fn steep_slopes_are_ramps() {
        let ramp = |degrees: f64, size: f64| {
            let slope = DVec3::new(degrees.to_radians().sin(), 0.0, degrees.to_radians().cos());
            let planes = vec![(DVec3::NEG_X, size), (DVec3::Y, size), (DVec3::NEG_Y, size), (DVec3::NEG_Z, size), (slope, 0.0)];
            let map = Map { planes, texinfos: vec![texinfo("dev/a")], brushes: vec![(0, 5)], sides: (0..5).map(|p| (p, 0, false)).collect() };
            map.brush(0, DMat3::IDENTITY, DVec3::ZERO).unwrap().1
        };
        assert!(ramp(60.0, 512.0));
        assert!(!ramp(40.0, 512.0), "walkable");
        assert!(!ramp(88.0, 512.0), "a wall");
        assert!(!ramp(60.0, 16.0), "trim");
    }

    #[test]
    fn bevel_closes_an_open_brush() {
        let planes = [DVec3::X, DVec3::NEG_X, DVec3::NEG_Y, DVec3::Z, DVec3::NEG_Z, DVec3::Y].map(|n| (n, 64.0)).to_vec();
        let map = Map { planes, texinfos: Vec::new(), brushes: vec![(0, 6)], sides: (0..6).map(|p| (p, -1, p == 5)).collect() };
        let (brush, _) = map.brush(0, DMat3::IDENTITY, DVec3::ZERO).unwrap();
        assert_closed(&brush);
        let Brush::Faces(faces) = &brush else { panic!() };
        assert!(faces.len() == 6 && faces.iter().all(|f| f.material == NODRAW && f.uv.is_none()));
    }

    #[test]
    fn rotation_matches_source_angles() {
        let forward = |p, y| rotation(DVec3::new(p, y, 0.0)) * DVec3::X;
        assert!(forward(0.0, 90.0).abs_diff_eq(DVec3::Y, 1e-12));
        assert!(forward(90.0, 0.0).abs_diff_eq(DVec3::NEG_Z, 1e-12));
        assert!((rotation(DVec3::new(0.0, 0.0, 90.0)) * DVec3::Y).abs_diff_eq(DVec3::Z, 1e-12));
    }

    #[test]
    fn parses_entity_text() {
        let text = "{\n\"classname\" \"worldspawn\"\n}\n{\n\"classname\" \"logic_relay\"\n\"OnTrigger\" \"a,Kill,,0,-1\"\n\"OnTrigger\" \"b,Use,}{,0,-1\"\n}\n{ \"classname\" \"x\" \"dangling\" }\n{ \"classname\" \"cut";
        let ents = parse_entities(text);
        assert_eq!(ents.len(), 4);
        assert_eq!(ents[1].len(), 3);
        assert_eq!(ents[1][2], ("OnTrigger".to_string(), "b,Use,}{,0,-1".to_string()));
        assert_eq!(key(&ents[2], "CLASSNAME"), Some("x"));
        assert_eq!(key(&ents[3], "classname"), Some("cut"));
    }

    #[test]
    fn unpatches_material_names() {
        assert_eq!(material_name("MATERIALS\\Maps\\surf_x\\Glass\\GlassWindow001a_-14096_0_-6152"), "glass/glasswindow001a");
        assert_eq!(material_name("maps/surf_x/nature/blendrockdirt001a_wvt_patch"), "nature/blendrockdirt001a");
        assert_eq!(material_name("maps/surf_x/custom/ramp_1_2"), "maps/surf_x/custom/ramp_1_2");
        assert_eq!(material_name("TOOLS/TOOLSPLAYERCLIP"), "tools/toolsplayerclip");
    }

    #[test]
    fn rejects_bad_files_without_panicking() {
        assert!(import(b"", &NO_FILES).is_err());
        assert!(import(b"IBSP\x26\0\0\0", &NO_FILES).is_err());
        let mut file = vec![0u8; HEADER];
        file[..4].copy_from_slice(b"VBSP");
        file[4] = 25;
        assert!(import(&file, &NO_FILES).err().unwrap_or_default().contains("version 25"));
        file[4] = 20;
        file[8..12].copy_from_slice(&5000i32.to_le_bytes());
        file[12..16].copy_from_slice(&10i32.to_le_bytes());
        assert!(import(&file, &NO_FILES).err().unwrap_or_default().contains("outside"));
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        for round in 0..300 {
            let mut file: Vec<u8> = (0..HEADER + 4096)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    seed as u8
                })
                .collect();
            file[..8].copy_from_slice(b"VBSP\x14\0\0\0");
            for lump in 0..64 {
                let at = 8 + lump * 16;
                let offset = (HEADER + (lump * 61 + round) % 4096) as i32;
                file[at..at + 4].copy_from_slice(&offset.to_le_bytes());
                file[at + 4..at + 8].copy_from_slice(&((lump as i32 * 37 + round as i32) % 600).to_le_bytes());
                if round % 3 == 0 && lump % 5 == 0 {
                    let o = offset as usize;
                    file[o..o + 4].copy_from_slice(b"LZMA");
                    file[at + 12..at + 16].copy_from_slice(&1000i32.to_le_bytes());
                }
            }
            let _ = import(&file, &NO_FILES);
        }
    }

    #[test]
    fn imports_a_surf_map() {
        let Some(data) = read(SURF) else { return };
        let map = import(&data, &NO_FILES).unwrap();
        let names: Vec<&str> = map.output.groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["Ramps", "World", "Triggers", "Spawns", "Teleport destinations", "Lights", "Brush entities", "Other"]);
        assert!(map.output.brushes.is_empty() && map.output.entities.is_empty());
        let (brushes, ents) = (world(&map), entities(&map));
        assert!(brushes.len() == map.world_brushes && brushes.len() > 3000, "{} world brushes", brushes.len());
        assert!(map.ramps > 100 && map.ramps < brushes.len() / 2, "{} ramps", map.ramps);
        assert_eq!((ents.len(), map.brush_entities, map.dropped_brushes), (153, 99, 0));
        brushes.iter().copied().for_each(assert_closed);
        assert_eq!(group(&map, "Spawns").map(|g| g.entities.len()), Some(48));
        let teleports = &group(&map, "Triggers").unwrap().entities;
        assert_eq!(teleports.iter().filter(|e| e.classname == "trigger_teleport").count(), 76);
        assert!(teleports.iter().all(|e| !e.brushes.is_empty() && key(&e.properties, "model").is_none()));
        teleports.iter().flat_map(|e| &e.brushes).for_each(assert_closed);
        let faces: Vec<&Face> = brushes
            .iter()
            .flat_map(|b| match b {
                Brush::Faces(f) => f.as_slice(),
                Brush::Hull(_) => &[],
            })
            .collect();
        assert!(faces.iter().any(|f| f.material == "tools/toolsplayerclip"));
        assert!(faces.iter().all(|f| !f.material.starts_with("maps/") && f.uv.is_some()));
        assert!(faces.iter().all(|f| map.output.materials.iter().any(|m| m.name == f.material && m.color.is_some())));
        let illusionary = ents.iter().find(|e| e.classname == "func_illusionary").unwrap();
        let origin = vector(key(&illusionary.properties, "origin"));
        let points: Vec<DVec3> = illusionary.brushes.iter().flat_map(|b| windings(&planes_of(b))).flatten().collect();
        let middle = points.iter().fold(DVec3::MAX, |a, p| a.min(*p)).midpoint(points.iter().fold(DVec3::MIN, |a, p| a.max(*p)));
        assert!(middle.distance(origin) < 1.0, "brush entity at {middle}, origin {origin}");
    }

    #[test]
    fn imports_lzma_lumps_and_static_props() {
        let Some(data) = read(LZMA) else { return };
        let map = import(&data, &NO_FILES).unwrap();
        assert!(map.warnings.is_empty(), "{:?}", map.warnings);
        assert!(map.world_brushes > 0 && map.static_props > 0);
        world(&map).into_iter().for_each(assert_closed);
        let prop = group(&map, "Props").unwrap().entities.iter().find(|e| e.classname == "prop_static").unwrap();
        assert!(key(&prop.properties, "model").is_some_and(|m| m.starts_with("models/") && m.ends_with(".mdl")));
    }

    #[test]
    fn finds_textures_in_the_map_and_the_game() {
        let game = Disk(vec![PathBuf::from(CSS).join("cstrike"), PathBuf::from(CSS).join("hl2")]);
        if let Some(data) = read(EMERALD) {
            let map = import(&data, &NO_FILES).unwrap();
            assert_eq!(map.textures, (19, 0, 0), "surf_emerald embeds all its textures");
            for m in &map.output.materials {
                let p = m.pixels.as_ref().unwrap();
                assert!(p.width <= 1024 && p.height <= 1024 && p.width > 1 && p.rgba.len() == (p.width * p.height * 4) as usize, "{}", m.name);
            }
        }
        let Some(data) = read(SURF) else { return };
        let alone = import(&data, &NO_FILES).unwrap();
        let start = Instant::now();
        let full = import(&data, &game).unwrap();
        eprintln!("{SURF}: alone {:?}, with the game {:?} in {:?}", alone.textures, full.textures, start.elapsed());
        assert_eq!(alone.textures.1, 0);
        if full.game_files {
            assert!(full.textures.1 > 0 && full.textures.2 < alone.textures.2);
        }
    }

    #[test]
    fn times_the_largest_map() {
        let Some(data) = read(LARGEST) else { return };
        let start = Instant::now();
        let map = import(&data, &NO_FILES).unwrap();
        eprintln!("{LARGEST}: {} MB in {:?}, {} world brushes, {} entities", data.len() / 1_000_000, start.elapsed(), map.world_brushes, map.entities);
        world(&map).into_iter().for_each(assert_closed);
    }
}

#[cfg(test)]
mod survey {
    use super::*;

    struct Disk(Vec<std::path::PathBuf>);
    impl Files for Disk {
        fn read(&self, path: &str) -> Option<Vec<u8>> {
            self.0.iter().find_map(|root| std::fs::read(root.join(path)).ok())
        }
        fn read_range(&self, path: &str, offset: u64, length: u64) -> Option<Vec<u8>> {
            use std::io::{Read, Seek, SeekFrom};
            self.0.iter().find_map(|root| {
                let mut file = std::fs::File::open(root.join(path)).ok()?;
                file.seek(SeekFrom::Start(offset)).ok()?;
                let mut out = vec![0; length as usize];
                file.read_exact(&mut out).ok()?;
                Some(out)
            })
        }
    }

    #[test]
    fn survey() {
        let Ok(list) = std::fs::read_to_string(std::env::var("BSPS").unwrap_or_default()) else { return };
        let css = std::path::PathBuf::from("/mnt/Storage/Gamedev/SurfsUp/CSS-Windows");
        let game = Disk(vec![css.join("cstrike"), css.join("hl2")]);
        let none = Disk(Vec::new());
        let (mut a, mut g) = ((0, 0, 0), (0, 0, 0));
        let mut missing = std::collections::BTreeMap::new();
        let (mut maps, mut pixels, mut worst, mut slowest) = (0, 0usize, (0usize, String::new()), (std::time::Duration::ZERO, String::new()));
        let (mut ramps, mut world) = (0, 0);
        for path in list.lines() {
            let Ok(data) = std::fs::read(path) else { continue };
            let Ok(alone) = import(&data, &none) else { continue };
            let t = std::time::Instant::now();
            let Ok(full) = import(&data, &game) else { continue };
            let el = t.elapsed();
            if el > slowest.0 { slowest = (el, path.to_string()); }
            maps += 1;
            ramps += full.ramps; world += full.world_brushes;
            a = (a.0 + alone.textures.0, a.1 + alone.textures.1, a.2 + alone.textures.2);
            g = (g.0 + full.textures.0, g.1 + full.textures.1, g.2 + full.textures.2);
            let px: usize = full.output.materials.iter().filter_map(|m| m.pixels.as_ref()).map(|p| p.rgba.len()).sum();
            pixels += px;
            if px > worst.0 { worst = (px, path.to_string()); }
            for m in full.output.materials.iter().filter(|m| m.pixels.is_none()) {
                let k = m.name.split('/').next().unwrap_or("").to_string();
                *missing.entry(k).or_insert(0) += 1;
            }
        }
        eprintln!("maps {maps}: alone {a:?} with game {g:?}; ramps {ramps} of {world} world brushes; avg pixel bytes {} MB, worst {:.1} MB {}; slowest {:?}", pixels / maps.max(1) / 1_000_000, worst.0 as f64 / 1e6, worst.1, slowest);
        let mut v: Vec<_> = missing.into_iter().collect();
        v.sort_by_key(|e| std::cmp::Reverse(e.1));
        eprintln!("missing by folder: {:?}", &v[..v.len().min(25)]);
    }
}
