//! The BSP file: its lumps, materials and node tree, as in SourceGD's `bsp`.

use std::borrow::Cow;

use glam::DVec3;
use valve::u16_at;

use crate::byte_reader::{c_str, f32_at, i16_at, i32_at, index, unlzma, vec_at};

pub(crate) const ENTITIES: usize = 0;
pub(crate) const PLANES: usize = 1;
pub(crate) const TEXDATA: usize = 2;
pub(crate) const VISIBILITY: usize = 4;
pub(crate) const NODES: usize = 5;
pub(crate) const TEXINFO: usize = 6;
pub(crate) const LEAFS: usize = 10;
pub(crate) const MODELS: usize = 14;
pub(crate) const LEAFBRUSHES: usize = 17;
pub(crate) const BRUSHES: usize = 18;
pub(crate) const BRUSHSIDES: usize = 19;
pub(crate) const DISPINFO: usize = 26;
pub(crate) const GAME_LUMP: usize = 35;
pub(crate) const PAKFILE: usize = 40;
pub(crate) const TEXDATA_STRING_DATA: usize = 43;
pub(crate) const TEXDATA_STRING_TABLE: usize = 44;
pub(crate) const HEADER: usize = 8 + 64 * 16 + 4;
pub(crate) const STATIC_PROPS: u32 = u32::from_be_bytes(*b"sprp");
pub(crate) const NODRAW: &str = "tools/toolsnodraw";

pub(crate) struct Bsp<'a> {
    pub(crate) file: &'a [u8],
    /// Offset, length, version and uncompressed length (0 when stored plain) of each lump.
    lumps: Vec<[i32; 4]>,
}

impl<'a> Bsp<'a> {
    pub(crate) fn open(file: &'a [u8]) -> Result<Self, String> {
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

    pub(crate) fn lump(&self, id: usize) -> Result<Cow<'a, [u8]>, String> {
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

    pub(crate) fn lump_version(&self, id: usize) -> i32 {
        self.lumps.get(id).map_or(0, |l| l[2])
    }

    /// The lump's size once uncompressed, read from the header alone.
    pub(crate) fn lump_size(&self, id: usize) -> usize {
        self.lumps.get(id).and_then(|l| usize::try_from(if l[3] > 0 { l[3] } else { l[1] }).ok()).unwrap_or(0)
    }
}

/// Lowercase with forward slashes and no `materials/` prefix, as SourceGD names materials.
pub(crate) fn raw_material_name(raw: &str) -> String {
    let name = raw.to_ascii_lowercase().replace('\\', "/");
    let name = name.trim_start_matches('/');
    name.strip_prefix("materials/").unwrap_or(name).to_string()
}

#[cfg(test)]
pub(crate) fn material_name(raw: &str) -> String {
    unpatch(&raw_material_name(raw)).to_string()
}

/// The material a VBSP patch copies: `maps/<map>/<material>_<x>_<y>_<z>` for cubemaps, `..._wvt_patch` for blends.
pub(crate) fn unpatch(name: &str) -> &str {
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

pub(crate) struct Texinfo {
    pub(crate) material: String,
    pub(crate) s: (DVec3, f64),
    pub(crate) t: (DVec3, f64),
}

/// A texdata's material, the name the map stores it under, and its reflectivity (linear color).
pub(crate) type Texdata = (String, String, DVec3);

pub(crate) fn texinfos(bsp: &Bsp) -> Result<(Vec<Texinfo>, Vec<Texdata>), String> {
    let data = bsp.lump(TEXDATA_STRING_DATA)?;
    let table: Vec<usize> = bsp.lump(TEXDATA_STRING_TABLE)?.as_chunks::<4>().0.iter().map(|r| index(i32_at(r, 0))).collect();
    let texdata: Vec<Texdata> = bsp
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

/// One leaf of the node tree: its vis cluster (negative when solid), area, bounds and brush list.
pub(crate) struct Leaf {
    pub(crate) cluster: i16,
    pub(crate) area: u16,
    pub(crate) mins: DVec3,
    pub(crate) maxs: DVec3,
    pub(crate) brushes: (usize, usize),
}

/// The node tree of every model, with each node's plane and children (a negative child is `-1 - leaf`).
pub(crate) struct Tree {
    pub(crate) nodes: Vec<(usize, [i32; 2])>,
    pub(crate) leafs: Vec<Leaf>,
    pub(crate) leafbrushes: Vec<usize>,
    pub(crate) heads: Vec<i32>,
}

impl Tree {
    pub(crate) fn read(bsp: &Bsp) -> Result<Self, String> {
        let nodes = bsp.lump(NODES)?.as_chunks::<32>().0.iter().map(|r| (index(i32_at(r, 0)), [i32_at(r, 4), i32_at(r, 8)])).collect();
        let stride = if bsp.lump_version(LEAFS) == 0 { 56 } else { 32 };
        let corner = |r: &[u8], at: usize| DVec3::new(f64::from(i16_at(r, at)), f64::from(i16_at(r, at + 2)), f64::from(i16_at(r, at + 4)));
        let leafs = bsp
            .lump(LEAFS)?
            .chunks_exact(stride)
            .map(|r| Leaf {
                cluster: i16_at(r, 4),
                area: u16_at(r, 6) & 0x1ff,
                mins: corner(r, 8),
                maxs: corner(r, 14),
                brushes: (usize::from(u16_at(r, 24)), usize::from(u16_at(r, 26))),
            })
            .collect();
        let leafbrushes = bsp.lump(LEAFBRUSHES)?.as_chunks::<2>().0.iter().map(|r| usize::from(u16_at(r, 0))).collect();
        let heads = bsp.lump(MODELS)?.as_chunks::<48>().0.iter().map(|r| i32_at(r, 36)).collect();
        Ok(Tree { nodes, leafs, leafbrushes, heads })
    }

    /// The leaf of the world model holding `p`, walking from the head node by plane sides.
    pub(crate) fn point_leaf(&self, planes: &[(DVec3, f64)], p: DVec3) -> Option<usize> {
        let mut node = *self.heads.first()?;
        for _ in 0..self.nodes.len() + 1 {
            if node < 0 {
                return Some(index(-(node + 1)));
            }
            let &(plane, [front, back]) = self.nodes.get(index(node))?;
            let &(normal, dist) = planes.get(plane)?;
            node = if normal.dot(p) - dist >= 0.0 { front } else { back };
        }
        None
    }
}

/// The clusters `cluster` can see, one bit each; empty when the map has no vis.
pub(crate) fn cluster_pvs(bsp: &Bsp, cluster: i16) -> Vec<u8> {
    let Ok(vis) = bsp.lump(VISIBILITY) else { return Vec::new() };
    let clusters = index(i32_at(&vis, 0));
    let Some(start) = usize::try_from(cluster).ok().filter(|&c| c < clusters).and_then(|c| vis.get(4 + c * 8..)).map(|r| index(i32_at(r, 0))) else {
        return Vec::new();
    };
    let mut out = vec![0u8; clusters.div_ceil(8)];
    let (mut at, mut o) = (start, 0);
    while o < out.len() {
        let Some(&b) = vis.get(at) else { break };
        at += 1;
        if b != 0 {
            if let Some(slot) = out.get_mut(o) {
                *slot = b;
            }
            o += 1;
        } else {
            let Some(&run) = vis.get(at) else { break };
            at += 1;
            o += usize::from(run);
        }
    }
    out
}
