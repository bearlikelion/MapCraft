//! WLD files: the fragments of a zone, its objects and lights that S3D archives hold.

use crate::read::Reader;

const MAGIC: u32 = 0x5450_3d02;
const NEW_FORMAT: u32 = 0x1000_c800;
const KEY: [u8; 8] = [0x95, 0x3a, 0xc5, 0x2a, 0x95, 0x7a, 0x95, 0x6a];
const MAX_FRAGMENTS: usize = 1 << 20;
/// Zone meshes stay far below this; it caps what one corrupt count can allocate.
const MAX_COUNT: usize = 65_536;

pub const BITMAP_NAME: u32 = 0x03;
pub const BITMAP_INFO: u32 = 0x04;
pub const BITMAP_INFO_REF: u32 = 0x05;
pub const ACTOR_DEF: u32 = 0x14;
pub const ACTOR: u32 = 0x15;
pub const LIGHT_SOURCE: u32 = 0x1b;
pub const LIGHT_SOURCE_REF: u32 = 0x1c;
pub const LIGHT: u32 = 0x28;
pub const MESH_REF: u32 = 0x2d;
pub const MATERIAL: u32 = 0x30;
pub const MATERIAL_LIST: u32 = 0x31;
pub const MESH: u32 = 0x36;

/// Decodes a hashed string: names in the string table and bitmap file names.
fn unhash(bytes: &[u8]) -> String {
    unhash_all(bytes).split('\0').next().unwrap_or_default().to_string()
}

pub struct Fragment<'a> {
    pub kind: u32,
    pub name: String,
    /// What follows the name reference.
    pub body: &'a [u8],
}

pub struct Wld<'a> {
    pub new_format: bool,
    strings: String,
    pub fragments: Vec<Fragment<'a>>,
}

impl<'a> Wld<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self, String> {
        let mut r = Reader::new(data);
        if r.u32()? != MAGIC {
            return Err("not a WLD file".into());
        }
        let new_format = r.u32()? == NEW_FORMAT;
        let count = r.u32()? as usize;
        r.skip(8)?;
        let hash_size = r.u32()? as usize;
        r.skip(4)?;
        let strings = unhash_all(r.bytes(hash_size)?);
        let mut wld = Self { new_format, strings, fragments: Vec::with_capacity(count.min(MAX_FRAGMENTS)) };
        for _ in 0..count.min(MAX_FRAGMENTS) {
            let size = r.u32()? as usize;
            let kind = r.u32()?;
            let frag = r.bytes(size)?;
            let mut f = Reader::new(frag);
            let name = wld.string(f.i32()?);
            wld.fragments.push(Fragment { kind, name, body: frag.get(4..).unwrap_or_default() });
        }
        Ok(wld)
    }

    /// The string a negative name reference points at.
    pub fn string(&self, reference: i32) -> String {
        let Some(at) = reference.checked_neg().and_then(|r| usize::try_from(r).ok()) else { return String::new() };
        let rest = self.strings.get(at..).unwrap_or_default();
        rest.split('\0').next().unwrap_or_default().to_string()
    }

    pub fn get(&self, index: Option<usize>, kind: u32) -> Option<&Fragment<'a>> {
        self.fragments.get(index?).filter(|f| f.kind == kind)
    }

    pub fn of_kind(&self, kind: u32) -> impl Iterator<Item = &Fragment<'a>> {
        self.fragments.iter().filter(move |f| f.kind == kind)
    }

    /// The bitmap files a material draws, first frame first; empty for an invisible one.
    pub fn material(&self, index: Option<usize>) -> Option<Material> {
        let f = self.get(index, MATERIAL)?;
        let mut r = Reader::new(f.body);
        let (_flags, params) = (r.u32().ok()?, r.u32().ok()?);
        r.skip(12).ok()?;
        let info_ref = r.frag().ok()?;
        let info = self.get(info_ref, BITMAP_INFO_REF).and_then(|f| Reader::new(f.body).frag().ok()).and_then(|i| self.get(i, BITMAP_INFO));
        let mut bitmaps = Vec::new();
        if let Some(info) = info {
            let mut r = Reader::new(info.body);
            let (flags, count) = (r.u32().ok()?, r.u32().ok()? as usize);
            if flags & 0x20 != 0 {
                r.skip(4).ok()?;
            }
            if flags & 0x08 != 0 && flags & 0x10 != 0 {
                r.skip(4).ok()?;
            }
            for _ in 0..count.min(MAX_COUNT) {
                let Some(name) = self.get(r.frag().ok()?, BITMAP_NAME) else { continue };
                let mut n = Reader::new(name.body);
                n.skip(4).ok()?;
                let len = usize::from(n.u16().ok()?);
                bitmaps.push(unhash(n.bytes(len).ok()?).to_ascii_lowercase());
            }
        }
        Some(Material { kind: params & !0x8000_0000, bitmaps })
    }

    /// The materials of a material list, in order.
    pub fn material_list(&self, index: Option<usize>) -> Vec<Option<Material>> {
        let Some(f) = self.get(index, MATERIAL_LIST) else { return Vec::new() };
        let mut r = Reader::new(f.body);
        let count = r.skip(4).and_then(|()| r.u32()).unwrap_or(0) as usize;
        (0..count.min(MAX_COUNT)).map_while(|_| r.frag().ok()).map(|i| self.material(i)).collect()
    }

    pub fn mesh(&self, f: &Fragment) -> Result<Mesh, String> {
        let mut r = Reader::new(f.body);
        r.skip(4)?;
        let materials = r.frag()?;
        r.skip(12)?;
        let center = r.vec3()?;
        r.skip(40)?;
        let mut count = || r.u16().map(usize::from);
        let (verts, uvs, normals, colors, polys, pieces, groups) = (count()?, count()?, count()?, count()?, count()?, count()?, count()?);
        r.skip(4)?;
        let scale = 1.0 / f32::powi(2.0, i32::from(r.i16()?.clamp(-30, 30)));
        let mut mesh = Mesh { materials, ..Mesh::default() };
        for _ in 0..verts {
            let [x, y, z] = [r.i16()?, r.i16()?, r.i16()?].map(|v| f32::from(v) * scale);
            mesh.positions.push([x + center[0], y + center[1], z + center[2]]);
        }
        for _ in 0..uvs {
            mesh.uvs.push(if self.new_format { [r.f32()?, r.f32()?] } else { [f32::from(r.i16()?) / 256.0, f32::from(r.i16()?) / 256.0] });
        }
        for _ in 0..normals {
            mesh.normals.push([r.i8()?, r.i8()?, r.i8()?].map(|n| f32::from(n) / 128.0));
        }
        r.skip(colors.checked_mul(4).ok_or("bad color count")?)?;
        for _ in 0..polys {
            let solid = r.u16()? == 0;
            mesh.triangles.push(Triangle { corners: [r.u16()?, r.u16()?, r.u16()?].map(usize::from), solid, material: 0 });
        }
        r.skip(pieces.checked_mul(4).ok_or("bad piece count")?)?;
        let mut at = 0usize;
        for _ in 0..groups {
            let (n, material) = (usize::from(r.u16()?), usize::from(r.u16()?));
            for t in mesh.triangles.iter_mut().skip(at).take(n) {
                t.material = material;
            }
            at = at.saturating_add(n);
        }
        Ok(mesh)
    }

    /// The mesh an actor definition draws, for static objects.
    pub fn actor_mesh(&self, def: &Fragment) -> Option<Mesh> {
        let mut r = Reader::new(def.body);
        let flags = r.u32().ok()?;
        r.skip(4).ok()?;
        let (actions, refs) = (r.u32().ok()? as usize, r.u32().ok()? as usize);
        r.skip(4).ok()?;
        if flags & 0x01 != 0 {
            r.skip(4).ok()?;
        }
        if flags & 0x02 != 0 {
            r.skip(28).ok()?;
        }
        for _ in 0..actions.min(MAX_COUNT) {
            let lods = r.u32().ok()? as usize;
            r.skip(4).ok()?;
            r.skip(lods.checked_mul(4)?).ok()?;
        }
        (0..refs.min(MAX_COUNT))
            .map_while(|_| r.frag().ok())
            .find_map(|i| self.get(i, MESH_REF).and_then(|m| Reader::new(m.body).frag().ok()).and_then(|m| self.get(m, MESH)))
            .and_then(|m| self.mesh(m).ok())
    }

    /// Placed objects: actor name, position, heading and tilt in EverQuest's 512ths of a turn, and scale.
    pub fn actors(&self) -> Vec<Actor> {
        self.of_kind(ACTOR).filter_map(|f| self.actor(f)).collect()
    }

    fn actor(&self, f: &Fragment) -> Option<Actor> {
        let mut r = Reader::new(f.body);
        let name = self.string(r.i32().ok()?);
        let flags = r.u32().ok()?;
        r.skip(4).ok()?;
        if flags & 0x01 != 0 {
            r.skip(4).ok()?;
        }
        if flags & 0x02 == 0 {
            return None;
        }
        let position = r.vec3().ok()?;
        let rotation = r.vec3().ok()?;
        r.skip(4).ok()?;
        if flags & 0x04 != 0 {
            r.skip(4).ok()?;
        }
        let scale = if flags & 0x08 != 0 { r.f32().ok()? } else { 1.0 };
        Some(Actor { name, position, rotation, scale })
    }

    /// Point lights: position, radius and color from 0 to 1.
    pub fn lights(&self) -> Vec<Light> {
        self.of_kind(LIGHT)
            .filter_map(|f| {
                let mut r = Reader::new(f.body);
                let source = r.frag().ok()?;
                r.skip(4).ok()?;
                let (position, radius) = (r.vec3().ok()?, r.f32().ok()?);
                let source = self.get(source, LIGHT_SOURCE_REF).and_then(|s| Reader::new(s.body).frag().ok()).and_then(|s| self.get(s, LIGHT_SOURCE));
                Some(Light { position, radius, color: source.and_then(light_color).unwrap_or([1.0; 3]) })
            })
            .collect()
    }
}

fn unhash_all(bytes: &[u8]) -> String {
    let plain: Vec<u8> = bytes.iter().zip(KEY.iter().cycle()).map(|(b, k)| b ^ k).collect();
    String::from_utf8_lossy(&plain).into_owned()
}

fn light_color(f: &Fragment) -> Option<[f32; 3]> {
    let mut r = Reader::new(f.body);
    let (flags, frames) = (r.u32().ok()?, r.u32().ok()? as usize);
    if flags & 0x10 == 0 || frames == 0 {
        return None;
    }
    let skip = usize::from(flags & 0x01 != 0) + usize::from(flags & 0x02 != 0);
    r.skip(skip * 4).ok()?;
    if flags & 0x04 != 0 {
        r.skip(frames.checked_mul(4)?).ok()?;
    }
    let c = r.vec3().ok()?;
    Some(c.map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 1.0 }))
}

pub struct Material {
    /// The render method, without its user defined bit.
    pub kind: u32,
    pub bitmaps: Vec<String>,
}

impl Material {
    /// Drawn with palette index 0 as a hole.
    pub fn masked(&self) -> bool {
        matches!(self.kind, 0x13 | 0x19)
    }

    /// Collision only: zone walls and other faces the game never draws.
    pub fn invisible(&self) -> bool {
        matches!(self.kind, 0x00 | 0x03 | 0x4b | 0x53) || self.bitmaps.is_empty()
    }
}

pub struct Triangle {
    pub corners: [usize; 3],
    /// Players collide with it.
    pub solid: bool,
    /// Index into the mesh's material list.
    pub material: usize,
}

#[derive(Default)]
pub struct Mesh {
    pub materials: Option<usize>,
    /// EverQuest's own space: Z up.
    pub positions: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub normals: Vec<[f32; 3]>,
    pub triangles: Vec<Triangle>,
}

pub struct Actor {
    pub name: String,
    pub position: [f32; 3],
    pub rotation: [f32; 3],
    pub scale: f32,
}

pub struct Light {
    pub position: [f32; 3],
    pub radius: f32,
    pub color: [f32; 3],
}
