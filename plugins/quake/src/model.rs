//! Quake's models for the views: `.mdl` meshes (first frame, first skin) and the `.bsp` brush
//! models items use, read from the user's PAK files or loose files.

use crate::wad::{self, Files, Picture};

/// Models are read whole; Quake's largest are well under this.
const MAX_FILE: u64 = 16 << 20;
const MAX_VERTS: usize = 1 << 16;
const MAX_TRIS: usize = 1 << 17;
const MAX_SKIN: usize = 1 << 22;

/// Lightmap pages are this many luxels on a side.
const PAGE: usize = 1024;
const LUXEL: f64 = 16.0;

/// One texture's triangles: positions, UVs (0 to 1, V down) and three indexes per triangle.
#[derive(Default)]
pub struct Part {
    pub texture: usize,
    pub positions: Vec<[f64; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    /// The texture's name in a `.bsp`, which the editor matches to the SDK's material.
    pub material: Option<String>,
    /// Index into the model's textures of a lightmap page, with a coordinate in it per position.
    pub lightmap: Option<usize>,
    pub lightmap_uvs: Vec<[f32; 2]>,
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

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// A named lump from a `.bsp`'s BSPX directory, which follows the standard lumps at a 4 byte boundary.
fn bspx<'a>(b: &'a [u8], name: &[u8]) -> Option<&'a [u8]> {
    let end = (0..15).filter_map(|i| usize::try_from(i32_at(b, 4 + i * 8)).ok()?.checked_add(usize::try_from(i32_at(b, 8 + i * 8)).ok()?)).max()?;
    let dir = b.get(end.checked_add(3)? & !3..)?.strip_prefix(b"BSPX")?;
    let n = usize::try_from(i32_at(dir, 0)).ok()?;
    let entries = dir.get(4..n.checked_mul(32)?.checked_add(4)?)?;
    let entry = entries.as_chunks::<32>().0.iter().find(|e| e[..24].split(|c| *c == 0).next() == Some(name))?;
    let (at, len) = (usize::try_from(i32_at(entry, 24)).ok()?, usize::try_from(i32_at(entry, 28)).ok()?);
    b.get(at..at.checked_add(len)?)
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
        Some(&[29, 0, 0, 0]) => {
            let lit = path.rsplit_once('.').and_then(|(stem, _)| find(files, &format!("{stem}.lit")));
            bsp(&data, &palette, lit.as_deref())
        }
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
    let mut part = Part { positions: Vec::with_capacity(ntris * 3), uvs: Vec::with_capacity(ntris * 3), ..Part::default() };
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

/// Lightmap pages filled shelf by shelf; the first starts with a black luxel for faces the light tool left dark.
struct Pages {
    pages: Vec<Vec<u8>>,
    x: usize,
    y: usize,
    row: usize,
}

impl Pages {
    fn new() -> Self {
        let mut pages = Pages { pages: Vec::new(), x: 0, y: 0, row: 0 };
        pages.place(1, 1);
        pages
    }

    /// The page and corner of a free `w` by `h` block.
    fn place(&mut self, w: usize, h: usize) -> Option<(usize, usize, usize)> {
        if w == 0 || h == 0 || w > PAGE || h > PAGE {
            return None;
        }
        if self.x + w > PAGE {
            (self.x, self.y, self.row) = (0, self.y + self.row, 0);
        }
        if self.pages.is_empty() || self.y + h > PAGE {
            self.pages.push([0, 0, 0, 255].repeat(PAGE * PAGE));
            (self.x, self.y, self.row) = (0, 0, 0);
        }
        let at = (self.pages.len() - 1, self.x, self.y);
        self.x += w;
        self.row = self.row.max(h);
        Some(at)
    }

    fn set(&mut self, page: usize, x: usize, y: usize, rgb: [u8; 3]) {
        let at = (y * PAGE + x) * 4;
        if let Some(px) = self.pages.get_mut(page).and_then(|p| p.get_mut(at..at + 3)) {
            px.copy_from_slice(&rgb);
        }
    }

    /// The pages as pictures, the last cut to the rows it uses.
    fn pictures(self) -> Vec<Picture> {
        let (last, used) = (self.pages.len().saturating_sub(1), self.y + self.row.max(1));
        let pages = self.pages.into_iter().enumerate();
        pages
            .map(|(i, mut rgba)| {
                let h = if i == last { used } else { PAGE };
                rgba.truncate(h * PAGE * 4);
                Picture { width: PAGE as u32, height: h as u32, rgba, full: (PAGE as u32, h as u32) }
            })
            .collect()
    }
}

/// The submodels Quake draws: the world and every brush entity but triggers, which its progs hide.
fn drawn(entities: &[u8], models: usize) -> Vec<usize> {
    let text = String::from_utf8_lossy(entities);
    let mut out = vec![0];
    for e in crate::map::parse(text.trim_end_matches('\0')).map(|m| m.entities).unwrap_or_default() {
        let shown = e.key("classname").is_some_and(|c| !c.starts_with("trigger_"));
        let n = e.key("model").and_then(|m| m.strip_prefix('*')).and_then(|n| n.parse::<usize>().ok());
        out.extend(n.filter(|n| shown && *n > 0 && *n < models));
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// A Quake `.bsp` (version 29): the world and the brush entities the game draws, with lightmap pages after the textures.
pub fn bsp(b: &[u8], palette: &[u8], lit: Option<&[u8]>) -> Result<Model, String> {
    if b.len() < 4 + 15 * 8 {
        return Err("is truncated".into());
    }
    let lump = |i: usize| -> Result<&[u8], String> {
        let (at, len) = (i32_at(b, 4 + i * 8), i32_at(b, 8 + i * 8));
        let (at, len) = (usize::try_from(at).map_err(|_| "is damaged")?, usize::try_from(len).map_err(|_| "is damaged")?);
        b.get(at..at.checked_add(len).ok_or("is damaged")?).ok_or_else(|| "is truncated".to_string())
    };
    let (entities, miptex, verts, texinfo, faces, lighting) = (lump(0)?, lump(2)?, lump(3)?, lump(6)?, lump(7)?, lump(8)?);
    let (edges, surfedges, models) = (lump(12)?, lump(13)?, lump(14)?);
    if models.len() < 64 {
        return Err("has no models".into());
    }
    let rgb = bspx(b, b"RGBLIGHTING")
        .filter(|l| l.len() % 3 == 0 && (lighting.is_empty() || l.len() == lighting.len() * 3))
        .or_else(|| lit.filter(|l| l.get(..8) == Some(b"QLIT\x01\0\0\0".as_slice()) && l.len() == 8 + lighting.len() * 3)?.get(8..));
    let samples = rgb.map_or(lighting.len(), |c| c.len() / 3);
    let (lmshift, lmoffset, lmstyle, decoupled) = (bspx(b, b"LMSHIFT"), bspx(b, b"LMOFFSET"), bspx(b, b"LMSTYLE"), bspx(b, b"DECOUPLED_LM"));
    // Quakespasm-family engines weigh a style at 'm' as 264/256 and clamp the sum to a byte.
    let luxel = |at: usize, styles: usize, size: usize| -> [u8; 3] {
        let mut sum = [0u32; 3];
        for k in 0..styles {
            let o = at + k * size;
            let rgb = match rgb {
                Some(c) => c.get(o * 3..o * 3 + 3).and_then(|c| c.try_into().ok()).unwrap_or_default(),
                None => [lighting.get(o).copied().unwrap_or(0); 3],
            };
            for (s, c) in sum.iter_mut().zip(rgb) {
                *s += u32::from(c) * 264;
            }
        }
        sum.map(|s| (s >> 8).min(255) as u8)
    };
    let vertex = |i: usize| vec_at(verts, i * 12).map(f64::from);
    let textures_count = usize::try_from(i32_at(miptex, 0)).ok().filter(|n| *n <= 1024).unwrap_or(0);
    let (mut textures, mut names): (Vec<Picture>, Vec<String>) = (Vec::new(), Vec::new());
    let mut index_of: Vec<Option<usize>> = vec![None; textures_count];
    let mut parts: Vec<Part> = Vec::new();
    let mut pages = Pages::new();
    for m in drawn(entities, models.len() / 64) {
        let model = models.get(m * 64..m * 64 + 64).ok_or("has a model past its models")?;
        let first = usize::try_from(i32_at(model, 56)).map_err(|_| "is damaged")?;
        let n = usize::try_from(i32_at(model, 60)).ok().filter(|n| *n <= 1 << 16).ok_or("is damaged")?;
        for f in first..first.saturating_add(n) {
            let face = faces.get(f * 20..f * 20 + 20).ok_or("has a face past its faces")?;
            let (first_edge, edge_count, info) = (i32_at(face, 4), usize::from(u16_at(face, 8)), usize::from(u16_at(face, 10)));
            let decoupled = decoupled.and_then(|d| d.get(f * 40..f * 40 + 40));
            let scale = lmshift.and_then(|s| s.get(f)).map(|s| 2f64.powi(i32::from(*s)));
            // qbsp can write LMSHIFT for the world's faces alone; the others keep their own light.
            let scaled = decoupled.is_some() || scale.is_some();
            let style_bytes = lmstyle.filter(|_| scaled).and_then(|s| s.get(f * 4..f * 4 + 4)).or(face.get(12..16)).unwrap_or_default();
            let styles = style_bytes.iter().take_while(|s| **s != 255).count();
            let offset = lmoffset.filter(|_| scaled).and_then(|o| o.get(f * 4..f * 4 + 4));
            let light_at = match (decoupled, offset) {
                (Some(d), _) => i32_at(d, 4),
                (None, Some(o)) => i32_at(o, 0),
                (None, None) => i32_at(face, 16),
            };
            let light_at = usize::try_from(light_at).ok();
            let tex = texinfo.get(info * 40..info * 40 + 40).ok_or("has a face without texture placement")?;
            let axis = |k: usize| (vec_at(tex, k * 16).map(f64::from), f64::from(f32_at(tex, k * 16 + 12)));
            let mip = usize::try_from(i32_at(tex, 32)).ok().filter(|m| *m < textures_count).ok_or("names a texture it lacks")?;
            let texture = match index_of.get(mip).copied().flatten() {
                Some(t) => t,
                None => {
                    // qbsp leaves a texture it could not find without pixels, so it shows as a blank.
                    let data = usize::try_from(i32_at(miptex, 4 + mip * 4)).ok().and_then(|at| miptex.get(at..)).unwrap_or_default();
                    let blank = || Picture { width: 1, height: 1, rgba: vec![255; 4], full: (64, 64) };
                    textures.push(wad::decode(data, palette, 0, false, false).unwrap_or_else(|_| blank()));
                    names.push(wad::name_at(data, 0, 16));
                    if let Some(slot) = index_of.get_mut(mip) {
                        *slot = Some(textures.len() - 1);
                    }
                    textures.len() - 1
                }
            };
            let (w, h) = textures.get(texture).map_or((1.0, 1.0), |p| (f64::from(p.full.0), f64::from(p.full.1)));
            let name = names.get(texture).cloned().unwrap_or_default();
            let (s_axis, t_axis) = (axis(0), axis(1));
            let mut points = Vec::with_capacity(edge_count);
            for k in 0..edge_count {
                let se = i32_at(surfedges, usize::try_from(first_edge).map_err(|_| "is damaged")?.saturating_add(k) * 4);
                let e = se.unsigned_abs() as usize;
                let p = vertex(usize::from(u16_at(edges, e * 4 + if se < 0 { 2 } else { 0 })));
                // Light tools keep texture coordinates as floats, so lightmap extents must round as theirs do.
                let st = |a: ([f64; 3], f64)| f64::from((dot(a.0, p) + a.1) as f32);
                points.push((p, st(s_axis), st(t_axis)));
            }
            // Skies and liquids the light tool skipped draw unlit; any other face without light is black.
            let unlit = name.starts_with("sky") || (name.starts_with('*') && light_at.is_none());
            let lightmap = (!unlit).then(|| {
                // Each point's luxel coordinate, whole at a luxel's center.
                let (size, coords): (Option<(usize, usize)>, Vec<[f64; 2]>) = match decoupled {
                    Some(d) => {
                        let axis = |k: usize| (vec_at(d, 8 + k * 16).map(f64::from), f64::from(f32_at(d, 20 + k * 16)));
                        let (u, v) = (axis(0), axis(1));
                        let coords = points.iter().map(|(p, _, _)| [dot(u.0, *p) + u.1, dot(v.0, *p) + v.1]).collect();
                        (Some((usize::from(u16_at(d, 0)), usize::from(u16_at(d, 2)))), coords)
                    }
                    None => {
                        let scale = scale.unwrap_or(LUXEL);
                        let lo = points.iter().fold([f64::INFINITY; 2], |m, (_, s, t)| [m[0].min(*s), m[1].min(*t)]);
                        let hi = points.iter().fold([f64::NEG_INFINITY; 2], |m, (_, s, t)| [m[0].max(*s), m[1].max(*t)]);
                        let (s0, t0) = ((lo[0] / scale).floor(), (lo[1] / scale).floor());
                        let (lw, lh) = ((hi[0] / scale).ceil() - s0 + 1.0, (hi[1] / scale).ceil() - t0 + 1.0);
                        let size = (lw.is_finite() && lh.is_finite()).then_some((lw as usize, lh as usize));
                        (size, points.iter().map(|(_, s, t)| [s / scale - s0, t / scale - t0]).collect())
                    }
                };
                let block = size.zip(light_at).filter(|((lw, lh), at)| *lw <= PAGE && *lh <= PAGE && styles > 0 && at + styles * lw * lh <= samples);
                match block.and_then(|((lw, lh), at)| Some((pages.place(lw, lh)?, lw, lh, at))) {
                    Some(((page, x, y), lw, lh, at)) => {
                        for j in 0..lh {
                            for i in 0..lw {
                                pages.set(page, x + i, y + j, luxel(at + j * lw + i, styles, lw * lh));
                            }
                        }
                        let uvs = coords.iter().map(|[u, v]| [(x as f64 + u + 0.5) as f32, (y as f64 + v + 0.5) as f32]);
                        (page, uvs.collect())
                    }
                    None => (0, vec![[0.5f32, 0.5]; points.len()]),
                }
            });
            let page = lightmap.as_ref().map(|(p, _)| *p);
            let i = match parts.iter().position(|p| p.texture == texture && p.lightmap == page) {
                Some(i) => i,
                None => {
                    let material = Some(name).filter(|n| !n.is_empty());
                    parts.push(Part { texture, material, lightmap: page, ..Part::default() });
                    parts.len() - 1
                }
            };
            let part = parts.get_mut(i).ok_or("lost a part")?;
            let base = part.positions.len() as u32;
            for (p, s, t) in &points {
                part.positions.push(*p);
                part.uvs.push([(s / w) as f32, (t / h) as f32]);
            }
            part.lightmap_uvs.extend(lightmap.map(|(_, uvs)| uvs).unwrap_or_default());
            for k in 1..edge_count.saturating_sub(1) as u32 {
                part.indices.extend([base, base + k, base + k + 1]);
            }
        }
    }
    if parts.iter().any(|p| p.lightmap.is_some()) {
        let pictures = pages.pictures();
        for part in parts.iter_mut() {
            let Some(page) = part.lightmap else { continue };
            let height = pictures.get(page).map_or(1.0, |p| p.height as f32);
            part.lightmap_uvs.iter_mut().for_each(|[u, v]| (*u, *v) = (*u / PAGE as f32, *v / height));
            part.lightmap = Some(textures.len() + page);
        }
        textures.extend(pictures);
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

    /// A 32 unit square at z 0 three times: in the world, a door and a trigger, each with a 3 by 3 lightmap.
    fn tiny_bsp() -> Vec<u8> {
        let i = |v: i32| v.to_le_bytes().to_vec();
        let f = |v: f32| v.to_le_bytes().to_vec();
        let entities = b"{\n\"classname\" \"worldspawn\"\n}\n{\n\"classname\" \"func_door\"\n\"model\" \"*1\"\n}\n\
            {\n\"classname\" \"trigger_once\"\n\"model\" \"*2\"\n}\n\0"
            .to_vec();
        let mut miptex = [i(1), i(8)].concat();
        miptex.extend(crate::wad::tests::miptex("Brick", 32, 32, None));
        let verts: Vec<u8> = [[0.0, 0.0], [32.0, 0.0], [32.0, 32.0], [0.0, 32.0]].iter().flat_map(|[x, y]| [f(*x), f(*y), f(0.0)].concat()).collect();
        let texinfo = [[1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0].map(f).concat(), i(0), i(0)].concat();
        let faces: Vec<u8> = (0..3).flat_map(|_| [vec![0, 0, 0, 0], i(0), vec![4, 0, 0, 0], vec![0, 255, 255, 255], i(0)].concat()).collect();
        let lighting = [200, 100, 100, 100, 100, 100, 100, 100, 100].to_vec();
        let edges: Vec<u8> = [0u16, 0, 0, 1, 1, 2, 2, 3, 3, 0].iter().flat_map(|v| v.to_le_bytes()).collect();
        let surfedges: Vec<u8> = (1..=4).flat_map(i).collect();
        let models: Vec<u8> = (0..3).flat_map(|m| [vec![0u8; 56], i(m), i(1)].concat()).collect();
        let lumps = [entities, vec![], miptex, verts, vec![], vec![], texinfo, faces, lighting, vec![], vec![], vec![], edges, surfedges, models];
        let mut b = i(29);
        let mut at = 4 + 15 * 8;
        for l in &lumps {
            b.extend([i(at as i32), i(l.len() as i32)].concat());
            at += l.len();
        }
        b.extend(lumps.concat());
        b
    }

    #[test]
    fn reads_a_bsps_lightmaps_into_pages() {
        let m = bsp(&tiny_bsp(), &palette(), None).unwrap();
        assert_eq!(m.parts.len(), 1);
        let part = &m.parts[0];
        assert_eq!(part.positions.len(), 8, "the world and the door, not the trigger");
        assert_eq!(part.material.as_deref(), Some("brick"));
        assert_eq!(part.lightmap, Some(1));
        let page = &m.textures[1];
        assert_eq!((page.width, page.height), (1024, 3), "a black luxel, then a 3 by 3 block per face, cut to the rows used");
        assert_eq!(page.rgba[4..12], [206, 206, 206, 255, 103, 103, 103, 255], "each style weighs 264/256");
        assert_eq!(page.rgba[..4], [0, 0, 0, 255]);
        let [u, v] = part.lightmap_uvs[0];
        assert!((u * 1024.0 - 1.5).abs() < 1e-4 && (v * 3.0 - 0.5).abs() < 1e-4, "luxel centers: {u} {v}");
        assert!((part.lightmap_uvs[4][0] * 1024.0 - 4.5).abs() < 1e-4, "the door's block follows the world's");
        let lit = [b"QLIT\x01\0\0\0".to_vec(), [255u8, 0, 10].repeat(9)].concat();
        let colored = bsp(&tiny_bsp(), &palette(), Some(lit.as_slice())).unwrap();
        assert_eq!(colored.textures[1].rgba[4..8], [255, 0, 10, 255], "a .lit colors it, clamped to a byte");
        assert_eq!(
            bsp(&tiny_bsp(), &palette(), Some(b"QLIT".as_slice())).unwrap().textures[1].rgba[4..8],
            [206, 206, 206, 255],
            "a .lit that does not fit is ignored"
        );
    }

    /// `b` with a BSPX directory of `lumps` after it.
    fn with_bspx(mut b: Vec<u8>, lumps: &[(&str, Vec<u8>)]) -> Vec<u8> {
        b.resize(b.len().next_multiple_of(4), 0);
        let mut at = b.len() + 8 + lumps.len() * 32;
        b.extend(b"BSPX");
        b.extend((lumps.len() as i32).to_le_bytes());
        for (name, data) in lumps {
            let mut field = [0u8; 24];
            field[..name.len()].copy_from_slice(name.as_bytes());
            b.extend([field.to_vec(), (at as i32).to_le_bytes().to_vec(), (data.len() as i32).to_le_bytes().to_vec()].concat());
            at += data.len();
        }
        b.extend(lumps.iter().flat_map(|(_, d)| d.clone()));
        b
    }

    fn ints(v: &[i32]) -> Vec<u8> {
        v.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    #[test]
    fn bspx_rgblighting_colors_the_lightmap() {
        let b = with_bspx(tiny_bsp(), &[("RGBLIGHTING", [100u8, 50, 200].repeat(9))]);
        assert_eq!(bsp(&b, &palette(), None).unwrap().textures[1].rgba[4..8], [103, 51, 206, 255], "no .lit needed");
        let lit = [b"QLIT\x01\0\0\0".to_vec(), [255u8, 0, 10].repeat(9)].concat();
        assert_eq!(bsp(&b, &palette(), Some(lit.as_slice())).unwrap().textures[1].rgba[4..8], [103, 51, 206, 255], "the lump wins over a .lit");
    }

    #[test]
    fn bspx_lightmap_scale_sizes_and_places_lightmaps() {
        let lumps = [("LMSHIFT", vec![5]), ("LMOFFSET", ints(&[1, 0, 0])), ("LMSTYLE", [0, 0, 255, 255].repeat(3))];
        let m = bsp(&with_bspx(tiny_bsp(), &lumps), &palette(), None).unwrap();
        let (page, uvs) = (&m.textures[1], &m.parts[0].lightmap_uvs);
        assert_eq!(page.height, 3, "the world's 2 by 2 block at 32 units a luxel, then the door's 3 by 3");
        assert_eq!(page.rgba[4..8], [206, 206, 206, 255], "two styles from sample 1 add samples 1 and 5");
        assert!((uvs[1][0] * 1024.0 - 2.5).abs() < 1e-4, "the world's block is two luxels wide: {}", uvs[1][0] * 1024.0);
        assert!((uvs[4][0] * 1024.0 - 3.5).abs() < 1e-4, "the door has no LMSHIFT byte, so it keeps 16 units a luxel");

        let mut decoupled = [2u16.to_le_bytes(), 2u16.to_le_bytes()].concat();
        decoupled.extend(ints(&[0]));
        decoupled.extend([1.0f32 / 32.0, 0.0, 0.0, 0.0, 0.0, 1.0 / 32.0, 0.0, 0.0].iter().flat_map(|v| v.to_le_bytes()));
        let m = bsp(&with_bspx(tiny_bsp(), &[("DECOUPLED_LM", decoupled)]), &palette(), None).unwrap();
        let uvs = &m.parts[0].lightmap_uvs;
        assert!((uvs[2][0] * 1024.0 - 2.5).abs() < 1e-4 && (uvs[2][1] * 3.0 - 1.5).abs() < 1e-4, "DECOUPLED_LM gives the size and luxel coordinates");
        assert!((uvs[4][0] * 1024.0 - 3.5).abs() < 1e-4);
    }

    #[test]
    fn a_damaged_bspx_directory_is_ignored() {
        let good = with_bspx(tiny_bsp(), &[("RGBLIGHTING", [100u8, 50, 200].repeat(9))]);
        let dir = tiny_bsp().len().next_multiple_of(4);
        let mut magic = good.clone();
        magic[dir] = b'Q';
        let mut count = good.clone();
        count[dir + 4..dir + 8].copy_from_slice(&i32::MAX.to_le_bytes());
        let mut past = good.clone();
        past[dir + 32..dir + 36].copy_from_slice(&(good.len() as i32 - 3).to_le_bytes());
        let cut = good[..dir + 20].to_vec();
        let short = with_bspx(tiny_bsp(), &[("RGBLIGHTING", [100u8, 50, 200].repeat(8))]);
        let cases = [
            (magic, "bad magic"),
            (count, "a count past the file"),
            (past, "a lump past the end"),
            (cut, "a cut directory"),
            (short, "a lump unlike LIGHTING"),
        ];
        for (b, why) in cases {
            assert_eq!(bsp(&b, &palette(), None).unwrap().textures[1].rgba[4..8], [206, 206, 206, 255], "{why}");
        }
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
        assert!(bsp(&[29, 0, 0, 0], &palette(), None).is_err());
    }
}
