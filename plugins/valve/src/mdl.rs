//! Source studio models: an MDL with its VVD vertices and DirectX 9 VTX triangles, in bind pose.

use std::collections::HashMap;

use crate::materials::Library;
use crate::vtf::Picture;

/// Bytes of one VVD vertex: bone weights, position, normal and texture coordinate.
const VERTEX: usize = 48;
const TEXTURE: usize = 64;
const BODY_PART: usize = 16;
const MESH: usize = 116;
const VTX_MESH: usize = 9;
const STRIP_GROUP: usize = 25;
const STRIP: usize = 27;
const VTX_VERTEX: usize = 9;
const STRIP_IS_TRILIST: u8 = 1;
const STRIP_IS_TRISTRIP: u8 = 2;
/// More of anything than a real model has; a count past it means a damaged file.
const MAX_COUNT: usize = 1 << 20;

/// A mesh of the model's first body of each part: its MDL texture name and its triangles.
pub struct Mesh {
    pub texture: String,
    pub positions: Vec<[f64; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// Counter-clockwise seen from the front.
    pub indices: Vec<u32>,
}

/// What [`parse`] reads from the three files.
pub struct Studio {
    /// Folders under `materials/` the textures are looked up in, in order, each ending in `/`.
    pub cdmaterials: Vec<String>,
    pub meshes: Vec<Mesh>,
}

/// A part of a loaded model: its material, an index into the model's pictures, and its triangles.
pub struct Part {
    pub material: String,
    pub picture: Option<usize>,
    pub mesh: Mesh,
}

pub struct Model {
    pub parts: Vec<Part>,
    pub pictures: Vec<Picture>,
}

fn get<const N: usize>(b: &[u8], at: usize) -> Option<[u8; N]> {
    b.get(at..at.checked_add(N)?)?.try_into().ok()
}

fn i32_at(b: &[u8], at: usize) -> Option<i32> {
    get(b, at).map(i32::from_le_bytes)
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    get(b, at).map(u16::from_le_bytes)
}

fn f32_at(b: &[u8], at: usize) -> Option<f32> {
    get(b, at).map(f32::from_le_bytes).filter(|v| v.is_finite())
}

fn count(b: &[u8], at: usize) -> Option<usize> {
    pos(b, at).filter(|&n| n <= MAX_COUNT)
}

/// A place in the file stored at `at`, which `get` later checks against the file's length.
fn pos(b: &[u8], at: usize) -> Option<usize> {
    usize::try_from(i32_at(b, at)?).ok()
}

/// `base` moved by the signed offset stored at `at`.
fn offset(b: &[u8], base: usize, at: usize) -> Option<usize> {
    base.checked_add_signed(isize::try_from(i32_at(b, at)?).ok()?)
}

fn c_str(b: &[u8], at: usize) -> Option<String> {
    let rest = b.get(at..)?;
    let len = rest.iter().position(|&c| c == 0)?;
    Some(String::from_utf8_lossy(rest.get(..len)?).to_string())
}

fn normalize(path: &str) -> String {
    path.trim().replace('\\', "/").trim_start_matches('/').to_ascii_lowercase()
}

/// The VVD's level-of-detail 0 vertices as offsets of their position in the file.
fn vvd_vertices(vvd: &[u8]) -> Option<Vec<usize>> {
    if vvd.get(..4)? != b"IDSV" {
        return None;
    }
    let (lod0, fixups, fixup_start, data_start) = (count(vvd, 16)?, count(vvd, 48)?, pos(vvd, 52)?, pos(vvd, 56)?);
    let ids: Vec<usize> = if fixups == 0 {
        (0..lod0).collect()
    } else {
        let mut ids = Vec::new();
        for f in 0..fixups {
            let at = fixup_start.checked_add(f.checked_mul(12)?)?;
            if i32_at(vvd, at)? >= 0 {
                let (source, n) = (count(vvd, at + 4)?, count(vvd, at + 8)?);
                ids.extend(source..source.checked_add(n)?);
            }
            if ids.len() > MAX_COUNT {
                return None;
            }
        }
        ids
    };
    ids.into_iter().map(|i| data_start.checked_add(i.checked_mul(VERTEX)?).filter(|at| at + VERTEX <= vvd.len())).collect()
}

/// Corners of the triangles in a VTX strip group, as indices into its vertices.
fn strip_triangles(vtx: &[u8], group: usize) -> Option<Vec<usize>> {
    let (num_indices, index_at) = (count(vtx, group + 8)?, offset(vtx, group, group + 12)?);
    let (strips, strip_at) = (count(vtx, group + 16)?, offset(vtx, group, group + 20)?);
    let index = |i: usize| -> Option<usize> {
        (i < num_indices).then_some(())?;
        u16_at(vtx, index_at.checked_add(i.checked_mul(2)?)?).map(usize::from)
    };
    let mut out = Vec::new();
    for s in 0..strips {
        let at = strip_at.checked_add(s.checked_mul(STRIP)?)?;
        let (n, first, flags) = (count(vtx, at)?, count(vtx, at + 4)?, *vtx.get(at + 18)?);
        if flags & STRIP_IS_TRILIST != 0 {
            for t in (0..n / 3 * 3).step_by(3) {
                out.extend([index(first + t)?, index(first + t + 1)?, index(first + t + 2)?]);
            }
        } else if flags & STRIP_IS_TRISTRIP != 0 {
            for t in 0..n.saturating_sub(2) {
                let (a, b, c) = (index(first + t)?, index(first + t + 1)?, index(first + t + 2)?);
                if a != b && b != c && a != c {
                    out.extend(if t % 2 == 0 { [a, b, c] } else { [b, a, c] });
                }
            }
        }
        if out.len() > MAX_COUNT {
            return None;
        }
    }
    Some(out)
}

/// Reads the first body of every body part, as Hammer shows a model.
pub fn parse(mdl: &[u8], vvd: &[u8], vtx: &[u8]) -> Result<Studio, String> {
    let bad = |what: &str| format!("damaged or unsupported model: {what}");
    if mdl.get(..4) != Some(b"IDST") {
        return Err(bad("not an MDL"));
    }
    let version = i32_at(mdl, 4).ok_or_else(|| bad("header"))?;
    if !(44..=49).contains(&version) {
        return Err(format!("MDL version {version} is not one Source 2013 games use"));
    }
    if i32_at(vtx, 0) != Some(7) {
        return Err(bad("VTX version"));
    }
    let header = || -> Option<_> {
        let textures: Vec<String> = (0..count(mdl, 204)?)
            .map(|i| {
                let at = pos(mdl, 208)?.checked_add(i.checked_mul(TEXTURE)?)?;
                c_str(mdl, offset(mdl, at, at)?)
            })
            .collect::<Option<_>>()?;
        let cdmaterials: Vec<String> = (0..count(mdl, 212)?)
            .map(|i| {
                let dir = normalize(&c_str(mdl, pos(mdl, pos(mdl, 216)?.checked_add(i.checked_mul(4)?)?)?)?);
                Some(if dir.is_empty() || dir.ends_with('/') { dir } else { format!("{dir}/") })
            })
            .collect::<Option<_>>()?;
        let (refs, families, skins) = (count(mdl, 220)?, count(mdl, 224)?, pos(mdl, 228)?);
        let skin = move |r: usize| -> Option<usize> {
            if families == 0 || r >= refs {
                return Some(r);
            }
            usize::try_from(i16::from_le_bytes(get(mdl, skins.checked_add(r.checked_mul(2)?)?)?)).ok()
        };
        Some((textures, cdmaterials, skin, count(mdl, 232)?, pos(mdl, 236)?))
    };
    let (textures, cdmaterials, skin, parts, part_at) = header().ok_or_else(|| bad("textures"))?;
    let vertices = vvd_vertices(vvd).ok_or_else(|| bad("VVD"))?;
    let vtx_parts = (count(vtx, 28), pos(vtx, 32));
    let mut meshes = Vec::new();
    for p in 0..parts {
        let read = || -> Option<Vec<Mesh>> {
            let (Some(vtx_count), Some(vtx_at)) = vtx_parts else { return None };
            if p >= vtx_count {
                return None;
            }
            let part = part_at.checked_add(p.checked_mul(BODY_PART)?)?;
            if count(mdl, part + 4)? == 0 {
                return Some(Vec::new());
            }
            let model = offset(mdl, part, part + 12)?;
            let vpart = vtx_at.checked_add(p.checked_mul(8)?)?;
            let vmodel = offset(vtx, vpart, vpart + 4)?;
            let lod = offset(vtx, vmodel, vmodel + 4)?;
            let (num_meshes, mesh_at, base) = (count(mdl, model + 72)?, offset(mdl, model, model + 76)?, pos(mdl, model + 84)? / VERTEX);
            let vmesh_at = offset(vtx, lod, lod + 4)?;
            if count(vtx, lod)? < num_meshes {
                return None;
            }
            let mut out = Vec::new();
            for m in 0..num_meshes {
                let mesh = mesh_at.checked_add(m.checked_mul(MESH)?)?;
                let first = base.checked_add(count(mdl, mesh + 12)?)?;
                let texture = textures.get(skin(count(mdl, mesh)?)?).cloned().unwrap_or_default();
                let vmesh = vmesh_at.checked_add(m.checked_mul(VTX_MESH)?)?;
                let (groups, group_at) = (count(vtx, vmesh)?, offset(vtx, vmesh, vmesh + 4)?);
                let mut out_mesh = Mesh { texture, positions: Vec::new(), uvs: Vec::new(), indices: Vec::new() };
                let mut local: HashMap<usize, u32> = HashMap::new();
                for g in 0..groups {
                    let group = group_at.checked_add(g.checked_mul(STRIP_GROUP)?)?;
                    let (num_verts, vert_at) = (count(vtx, group)?, offset(vtx, group, group + 4)?);
                    let corners = strip_triangles(vtx, group)?;
                    for tri in corners.as_chunks::<3>().0 {
                        let mut ids = [0u32; 3];
                        for (slot, &v) in ids.iter_mut().zip(tri) {
                            if v >= num_verts {
                                return None;
                            }
                            let orig = usize::from(u16_at(vtx, vert_at.checked_add(v.checked_mul(VTX_VERTEX)?)?.checked_add(4)?)?);
                            let global = first.checked_add(orig)?;
                            *slot = match local.get(&global) {
                                Some(&i) => i,
                                None => {
                                    let at = *vertices.get(global)?;
                                    let i = u32::try_from(out_mesh.positions.len()).ok()?;
                                    out_mesh.positions.push([16, 20, 24].map(|o| f32_at(vvd, at + o).map_or(0.0, f64::from)));
                                    out_mesh.uvs.push([f32_at(vvd, at + 40).unwrap_or(0.0), f32_at(vvd, at + 44).unwrap_or(0.0)]);
                                    local.insert(global, i);
                                    i
                                }
                            };
                        }
                        // Source draws clockwise triangles as their front.
                        out_mesh.indices.extend([ids[0], ids[2], ids[1]]);
                    }
                }
                if !out_mesh.indices.is_empty() {
                    out.push(out_mesh);
                }
            }
            Some(out)
        };
        meshes.extend(read().ok_or_else(|| bad("body part"))?);
    }
    Ok(Studio { cdmaterials, meshes })
}

/// The model at `path`, such as `models/props/de_dust/du_crate_64x64.mdl`, with its pictures no
/// wider or taller than `max`.
pub fn load(library: &Library, path: &str, max: usize) -> Result<Model, String> {
    let path = normalize(path);
    let stem = path.strip_suffix(".mdl").ok_or_else(|| format!("{path} is not an .mdl"))?;
    let read = |p: &str| library.read(p).ok_or_else(|| format!("{p} is not in the game's files"));
    let (mdl, vvd) = (read(&path)?, read(&format!("{stem}.vvd"))?);
    let vtx = ["dx90.vtx", "dx80.vtx", "sw.vtx"]
        .iter()
        .find_map(|ext| library.read(&format!("{stem}.{ext}")))
        .ok_or_else(|| format!("{stem}.dx90.vtx is not in the game's files"))?;
    let studio = parse(&mdl, &vvd, &vtx)?;
    let mut pictures = Vec::new();
    let mut by_material: HashMap<String, Option<usize>> = HashMap::new();
    let parts = studio
        .meshes
        .into_iter()
        .map(|mesh| {
            let texture = normalize(&mesh.texture);
            let candidates: Vec<String> = studio.cdmaterials.iter().map(|dir| format!("{dir}{texture}")).chain([texture.clone()]).collect();
            let material = candidates.iter().find(|c| library.has(&format!("materials/{c}.vmt"))).or(candidates.first()).cloned().unwrap_or_default();
            let picture = *by_material.entry(material.clone()).or_insert_with(|| {
                library.picture(&material, max).map(|(p, ..)| {
                    pictures.push(p);
                    pictures.len() - 1
                })
            });
            Part { material, picture, mesh }
        })
        .collect();
    Ok(Model { parts, pictures })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODEL: usize = 148;

    fn put(b: &mut Vec<u8>, at: usize, v: &[u8]) {
        if b.len() < at + v.len() {
            b.resize(at + v.len(), 0);
        }
        b[at..at + v.len()].copy_from_slice(v);
    }

    fn int(b: &mut Vec<u8>, at: usize, v: i32) {
        put(b, at, &v.to_le_bytes());
    }

    /// One triangle with one texture, laid out as studiomdl writes the three files.
    fn triangle() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let mut mdl = Vec::new();
        put(&mut mdl, 0, b"IDST");
        int(&mut mdl, 4, 48);
        let (tex, cd, skins, part) = (240, 320, 360, 400);
        int(&mut mdl, 204, 1);
        int(&mut mdl, 208, tex);
        int(&mut mdl, tex as usize, TEXTURE as i32);
        put(&mut mdl, tex as usize + TEXTURE, b"Crate\0");
        int(&mut mdl, 212, 1);
        int(&mut mdl, 216, cd);
        int(&mut mdl, cd as usize, cd + 8);
        put(&mut mdl, cd as usize + 8, b"models\\props\\\0");
        int(&mut mdl, 220, 1);
        int(&mut mdl, 224, 1);
        int(&mut mdl, 228, skins);
        int(&mut mdl, 232, 1);
        int(&mut mdl, 236, part);
        let part = part as usize;
        int(&mut mdl, part + 4, 1);
        int(&mut mdl, part + 12, BODY_PART as i32);
        let model = part + BODY_PART;
        int(&mut mdl, model + 72, 1);
        int(&mut mdl, model + 76, MODEL as i32);
        int(&mut mdl, model + 80, 3);
        int(&mut mdl, model + MODEL + 8, 3);
        mdl.resize(model + MODEL + MESH, 0);

        let mut vvd = Vec::new();
        put(&mut vvd, 0, b"IDSV");
        int(&mut vvd, 4, 4);
        int(&mut vvd, 12, 1);
        int(&mut vvd, 16, 3);
        int(&mut vvd, 52, 64);
        int(&mut vvd, 56, 64);
        for (i, (p, uv)) in [([0.0f32, 0.0, 0.0], [0.0f32, 1.0]), ([10.0, 0.0, 0.0], [1.0, 1.0]), ([0.0, 0.0, 10.0], [0.0, 0.0])].iter().enumerate() {
            let at = 64 + i * VERTEX;
            for (k, v) in p.iter().chain(uv).enumerate() {
                put(&mut vvd, at + if k < 3 { 16 + k * 4 } else { 40 + (k - 3) * 4 }, &v.to_le_bytes());
            }
        }

        let mut vtx = Vec::new();
        int(&mut vtx, 0, 7);
        int(&mut vtx, 28, 1);
        int(&mut vtx, 32, 36);
        int(&mut vtx, 36, 1);
        int(&mut vtx, 40, 8);
        int(&mut vtx, 44, 1);
        int(&mut vtx, 48, 8);
        int(&mut vtx, 52, 1);
        int(&mut vtx, 56, 12);
        int(&mut vtx, 64, 1);
        int(&mut vtx, 68, VTX_MESH as i32);
        let group = 64 + VTX_MESH;
        int(&mut vtx, group, 3);
        int(&mut vtx, group + 4, STRIP_GROUP as i32);
        int(&mut vtx, group + 8, 3);
        int(&mut vtx, group + 12, (STRIP_GROUP + 3 * VTX_VERTEX) as i32);
        int(&mut vtx, group + 16, 1);
        int(&mut vtx, group + 20, (STRIP_GROUP + 3 * VTX_VERTEX + 6) as i32);
        for v in 0..3u16 {
            put(&mut vtx, group + STRIP_GROUP + usize::from(v) * VTX_VERTEX + 4, &v.to_le_bytes());
            put(&mut vtx, group + STRIP_GROUP + 3 * VTX_VERTEX + usize::from(v) * 2, &v.to_le_bytes());
        }
        let strip = group + STRIP_GROUP + 3 * VTX_VERTEX + 6;
        int(&mut vtx, strip, 3);
        int(&mut vtx, strip + 8, 3);
        put(&mut vtx, strip + 18, &[STRIP_IS_TRILIST]);
        put(&mut vtx, strip + STRIP - 1, &[0]);
        (mdl, vvd, vtx)
    }

    #[test]
    fn reads_a_triangle() {
        let (mdl, vvd, vtx) = triangle();
        let studio = parse(&mdl, &vvd, &vtx).unwrap();
        assert_eq!(studio.cdmaterials, ["models/props/"]);
        assert_eq!(studio.meshes.len(), 1);
        let mesh = &studio.meshes[0];
        assert_eq!(mesh.texture, "Crate");
        assert_eq!(mesh.positions, [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 0.0, 10.0]]);
        assert_eq!(mesh.uvs[1], [1.0, 1.0]);
        assert_eq!(mesh.indices, [0, 2, 1], "Source's clockwise front turned counter-clockwise");
    }

    #[test]
    fn damaged_files_are_errors() {
        let (mdl, vvd, vtx) = triangle();
        // The header, the texture name, and the first mesh's vertex offset.
        for cut in [3, 100, 239, 306, 578] {
            assert!(parse(&mdl[..cut], &vvd, &vtx).is_err(), "MDL cut at {cut}");
        }
        for cut in [10, 60, vvd.len() - 1] {
            assert!(parse(&mdl, &vvd[..cut], &vtx).is_err(), "VVD cut at {cut}");
        }
        for cut in [2, 40, 130] {
            assert!(parse(&mdl, &vvd, &vtx[..cut]).is_err(), "VTX cut at {cut}");
        }
        let mut huge = vtx.clone();
        int(&mut huge, 64 + VTX_MESH + 8, i32::MAX);
        assert!(parse(&mdl, &vvd, &huge).is_err());
    }
}
