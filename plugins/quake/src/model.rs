//! Quake's models for the views: `.mdl` meshes (first frame, first skin) and the `.bsp` brush
//! models items use, read from the user's PAK files or loose files.

use crate::wad::{self, Files, Picture};

/// Models are read whole; Quake's largest are well under this.
const MAX_FILE: u64 = 16 << 20;
const MAX_VERTS: usize = 1 << 16;
const MAX_TRIS: usize = 1 << 17;
const MAX_SKIN: usize = 1 << 22;

/// One texture's triangles: positions, UVs (0 to 1, V down) and three indexes per triangle.
pub struct Part {
    pub texture: usize,
    pub positions: Vec<[f64; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

pub struct Model {
    pub parts: Vec<Part>,
    pub textures: Vec<Picture>,
}

fn i32_at(b: &[u8], at: usize) -> i32 {
    b.get(at..at.saturating_add(4)).and_then(|s| s.try_into().ok()).map_or(0, i32::from_le_bytes)
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    b.get(at..at.saturating_add(2)).and_then(|s| s.try_into().ok()).map_or(0, u16::from_le_bytes)
}

fn f32_at(b: &[u8], at: usize) -> f32 {
    b.get(at..at.saturating_add(4)).and_then(|s| s.try_into().ok()).map_or(0.0, f32::from_le_bytes)
}

fn vec_at(b: &[u8], at: usize) -> [f32; 3] {
    [f32_at(b, at), f32_at(b, at + 4), f32_at(b, at + 8)]
}

fn count(b: &[u8], at: usize, max: usize) -> Result<usize, String> {
    usize::try_from(i32_at(b, at)).ok().filter(|n| *n <= max).ok_or_else(|| "has an impossible count".to_string())
}

/// The file at `path` (such as `progs/ogre.mdl`), loose in a game folder or inside a PAK, id1's first.
pub fn find(files: &dyn Files, path: &str) -> Option<Vec<u8>> {
    let path = path.trim().replace('\\', "/").to_ascii_lowercase();
    let loose = [format!("id1/{path}"), path.clone()].into_iter().filter_map(|p| files.read_range(&p, 0, MAX_FILE)).find(|b| !b.is_empty());
    loose.or_else(|| {
        let mut paks: Vec<String> = files.list(&["pak"]);
        paks.sort_by_key(|p| {
            let p = p.to_ascii_lowercase();
            (!p.starts_with("id1/"), p)
        });
        paks.iter().find_map(|pak| wad::pak_file(files, pak, &path))
    })
}

pub fn load(files: &dyn Files, path: &str) -> Result<Model, String> {
    let data = find(files, path).ok_or_else(|| format!("{path} is not in the Quake folder or its PAK files; set the SDK's Quake folder"))?;
    let palette = wad::palette(files, &files.list(&["pak"])).ok_or("Quake's palette was not found; set the SDK's Quake folder")?;
    match data.get(..4) {
        Some(b"IDPO") => mdl(&data, &palette),
        Some(&[29, 0, 0, 0]) => bsp(&data, &palette),
        _ => Err(format!("{path} is not a Quake model")),
    }
}

/// The first frame and first skin of a Quake `.mdl` (IDPO version 6).
pub fn mdl(b: &[u8], palette: &[u8]) -> Result<Model, String> {
    if i32_at(b, 4) != 6 {
        return Err("is not a version 6 model".into());
    }
    let (scale, origin) = (vec_at(b, 8), vec_at(b, 20));
    let skins = count(b, 48, 256)?;
    let (sw, sh) = (count(b, 52, 4096)?, count(b, 56, 4096)?);
    let (nverts, ntris, nframes) = (count(b, 60, MAX_VERTS)?, count(b, 64, MAX_TRIS)?, count(b, 68, 1 << 16)?);
    let size = sw.checked_mul(sh).filter(|s| *s > 0 && *s <= MAX_SKIN).ok_or("has a bad skin size")?;
    if skins == 0 || nverts == 0 || nframes == 0 {
        return Err("has no skin, vertices or frames".into());
    }
    let mut at = 84;
    let mut skin: Option<&[u8]> = None;
    for _ in 0..skins {
        let group = i32_at(b, at);
        at += 4;
        let pictures = if group == 0 {
            1
        } else {
            let n = count(b, at, 256)?;
            at += 4 + n * 4;
            n
        };
        skin = skin.or_else(|| b.get(at..at + size));
        at = at.checked_add(size.checked_mul(pictures).ok_or("has too many skins")?).ok_or("is truncated")?;
    }
    let skin = skin.ok_or("is truncated in its skins")?;
    let coords = at;
    let tris = coords + nverts * 12;
    let frames = tris + ntris * 16;
    // A frame group starts with its count, bounds and times; its first frame follows.
    let frame = if i32_at(b, frames) == 0 { frames + 4 } else { frames + 4 + 4 + 8 + count(b, frames + 4, 1 << 16)? * 4 };
    let verts = frame + 8 + 16;
    b.get(verts..verts + nverts * 4).ok_or("is truncated in its frames")?;
    let position = |v: usize| {
        let at = verts + v * 4;
        std::array::from_fn(|k| f64::from(b.get(at + k).copied().unwrap_or(0)) * f64::from(scale[k]) + f64::from(origin[k]))
    };
    let mut part = Part { texture: 0, positions: Vec::with_capacity(ntris * 3), uvs: Vec::with_capacity(ntris * 3), indices: Vec::new() };
    for t in 0..ntris {
        let at = tris + t * 16;
        let front = i32_at(b, at) != 0;
        for k in 0..3 {
            let v = usize::try_from(i32_at(b, at + 4 + k * 4)).ok().filter(|v| *v < nverts).ok_or("has a triangle past its vertices")?;
            let c = coords + v * 12;
            let seam = i32_at(b, c) != 0;
            // A back-facing triangle on the seam takes the right half of the skin.
            let s = i32_at(b, c + 4) as f32 + if seam && !front { sw as f32 / 2.0 } else { 0.0 };
            let t = i32_at(b, c + 8) as f32;
            part.indices.push(part.positions.len() as u32);
            part.positions.push(position(v));
            part.uvs.push([(s + 0.5) / sw as f32, (t + 0.5) / sh as f32]);
        }
    }
    let mut rgba = Vec::with_capacity(size * 4);
    for &i in skin {
        let at = usize::from(i) * 3;
        let [r, g, bl]: [u8; 3] = palette.get(at..at + 3).and_then(|c| c.try_into().ok()).unwrap_or_default();
        rgba.extend([r, g, bl, 255]);
    }
    let picture = Picture { width: sw as u32, height: sh as u32, rgba, full: (sw as u32, sh as u32) };
    Ok(Model { parts: vec![part], textures: vec![picture] })
}

/// The world model of a Quake `.bsp` (version 29), as the health and ammo boxes are, with its textures.
pub fn bsp(b: &[u8], palette: &[u8]) -> Result<Model, String> {
    if b.len() < 4 + 15 * 8 {
        return Err("is truncated".into());
    }
    let lump = |i: usize| -> Result<&[u8], String> {
        let (at, len) = (i32_at(b, 4 + i * 8), i32_at(b, 8 + i * 8));
        let (at, len) = (usize::try_from(at).map_err(|_| "is damaged")?, usize::try_from(len).map_err(|_| "is damaged")?);
        b.get(at..at.checked_add(len).ok_or("is damaged")?).ok_or_else(|| "is truncated".to_string())
    };
    let (miptex, verts, texinfo, faces, edges, surfedges, models) = (lump(2)?, lump(3)?, lump(6)?, lump(7)?, lump(12)?, lump(13)?, lump(14)?);
    if models.len() < 64 {
        return Err("has no models".into());
    }
    let first = usize::try_from(i32_at(models, 56)).map_err(|_| "is damaged")?;
    let n = usize::try_from(i32_at(models, 60)).ok().filter(|n| *n <= 1 << 16).ok_or("is damaged")?;
    let vertex = |i: usize| vec_at(verts, i * 12).map(f64::from);
    let textures_count = usize::try_from(i32_at(miptex, 0)).ok().filter(|n| *n <= 1024).unwrap_or(0);
    let mut textures: Vec<Picture> = Vec::new();
    let mut index_of: Vec<Option<usize>> = vec![None; textures_count];
    let mut parts: Vec<Part> = Vec::new();
    for f in first..first.saturating_add(n) {
        let face = faces.get(f * 20..f * 20 + 20).ok_or("has a face past its faces")?;
        let (first_edge, edge_count, info) = (i32_at(face, 4), usize::from(u16_at(face, 8)), usize::from(u16_at(face, 10)));
        let tex = texinfo.get(info * 40..info * 40 + 40).ok_or("has a face without texture placement")?;
        let axis = |k: usize| (vec_at(tex, k * 16).map(f64::from), f64::from(f32_at(tex, k * 16 + 12)));
        let mip = usize::try_from(i32_at(tex, 32)).ok().filter(|m| *m < textures_count).ok_or("names a texture it lacks")?;
        let texture = match index_of.get(mip).copied().flatten() {
            Some(t) => t,
            None => {
                let at = usize::try_from(i32_at(miptex, 4 + mip * 4)).map_err(|_| "is damaged")?;
                let picture = wad::decode(miptex.get(at..).unwrap_or_default(), palette, 0, false, false)?;
                textures.push(picture);
                if let Some(slot) = index_of.get_mut(mip) {
                    *slot = Some(textures.len() - 1);
                }
                textures.len() - 1
            }
        };
        let (w, h) = textures.get(texture).map_or((1.0, 1.0), |p| (f64::from(p.full.0), f64::from(p.full.1)));
        let i = match parts.iter().position(|p| p.texture == texture) {
            Some(i) => i,
            None => {
                parts.push(Part { texture, positions: Vec::new(), uvs: Vec::new(), indices: Vec::new() });
                parts.len() - 1
            }
        };
        let part = parts.get_mut(i).ok_or("lost a part")?;
        let base = part.positions.len() as u32;
        for k in 0..edge_count {
            let se = i32_at(surfedges, usize::try_from(first_edge).map_err(|_| "is damaged")?.saturating_add(k) * 4);
            let e = se.unsigned_abs() as usize;
            let v = usize::from(u16_at(edges, e * 4 + if se < 0 { 2 } else { 0 }));
            let p = vertex(v);
            let (s, t) = (axis(0), axis(1));
            let dot = |a: [f64; 3]| a[0] * p[0] + a[1] * p[1] + a[2] * p[2];
            part.positions.push(p);
            part.uvs.push([((dot(s.0) + s.1) / w) as f32, ((dot(t.0) + t.1) / h) as f32]);
        }
        for k in 1..edge_count.saturating_sub(1) as u32 {
            part.indices.extend([base, base + k, base + k + 1]);
        }
    }
    Ok(Model { parts, textures })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette() -> Vec<u8> {
        (0..=255u8).flat_map(|i| [i, i, i]).collect()
    }

    /// A two-triangle model: a 2 by 2 skin, three vertices on the seam, one frame.
    fn tiny_mdl() -> Vec<u8> {
        let mut b = b"IDPO".to_vec();
        let i = |b: &mut Vec<u8>, v: i32| b.extend(v.to_le_bytes());
        let f = |b: &mut Vec<u8>, v: f32| b.extend(v.to_le_bytes());
        i(&mut b, 6);
        for v in [2.0, 2.0, 2.0, -8.0, -8.0, 0.0, 16.0, 0.0, 0.0, 0.0] {
            f(&mut b, v);
        }
        for v in [1, 2, 2, 3, 2, 1, 0, 0] {
            i(&mut b, v);
        }
        f(&mut b, 1.0);
        i(&mut b, 0);
        b.extend([1, 2, 3, 4]);
        for (seam, s, t) in [(1, 0, 0), (0, 1, 0), (1, 0, 1)] {
            for v in [seam, s, t] {
                i(&mut b, v);
            }
        }
        for (front, a, c, d) in [(1, 0, 1, 2), (0, 2, 1, 0)] {
            for v in [front, a, c, d] {
                i(&mut b, v);
            }
        }
        i(&mut b, 0);
        b.extend([0u8; 8]);
        b.extend([0u8; 16]);
        b.extend([0, 0, 0, 0, 8, 0, 0, 0, 0, 8, 4, 0]);
        b
    }

    #[test]
    fn reads_an_mdl() {
        let m = mdl(&tiny_mdl(), &palette()).unwrap();
        let part = &m.parts[0];
        assert_eq!(part.indices.len(), 6);
        assert_eq!(part.positions[1], [8.0, -8.0, 0.0], "positions are scale times the byte plus the origin");
        assert_eq!(part.positions[2], [-8.0, 8.0, 8.0]);
        assert_eq!(m.textures[0].rgba, [1, 1, 1, 255, 2, 2, 2, 255, 3, 3, 3, 255, 4, 4, 4, 255]);
        assert_eq!(part.uvs[0], [0.25, 0.25]);
        assert_eq!(part.uvs[5], [0.75, 0.25], "a back-facing triangle on the seam reads the right half");
    }

    #[test]
    fn damaged_models_are_errors() {
        let good = tiny_mdl();
        for cut in [10, 90, 120, good.len() - 2] {
            assert!(mdl(&good[..cut], &palette()).is_err(), "cut at {cut}");
        }
        let mut huge = good.clone();
        huge[64..68].copy_from_slice(&i32::MAX.to_le_bytes());
        assert!(mdl(&huge, &palette()).is_err());
        assert!(bsp(&[29, 0, 0, 0], &palette()).is_err());
    }
}
