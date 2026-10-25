//! Quake's textures: WAD2 texture lumps (and Half-Life's WAD3), the palette from the user's PAK files, and decoding to RGBA.

use std::collections::{BTreeMap, BTreeSet};

/// Files the plugin may read: the host in the editor, the disk in tests.
pub trait Files {
    fn read_range(&self, path: &str, offset: u64, length: u64) -> Option<Vec<u8>>;
    /// Files with one of the extensions, as `/`-separated relative paths.
    fn list(&self, extensions: &[&str]) -> Vec<String>;
}

const MAX_LUMPS: u64 = 1 << 16;
const MAX_LUMP: u64 = 64 << 20;
const MAX_PAK_DIR: u64 = 64 * (1 << 16);
const MAX_SIDE: u32 = 8192;
/// RGBA bytes one call may decode; past it every texture comes from a smaller mip.
const PICTURE_BUDGET: u64 = 128 << 20;
const MIPTEX_WAD2: u8 = 0x44;
const MIPTEX_WAD3: u8 = 0x43;

fn u16_at(b: &[u8], at: usize) -> u16 {
    b.get(at..at.saturating_add(2)).and_then(|s| s.try_into().ok()).map_or(0, u16::from_le_bytes)
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    b.get(at..at.saturating_add(4)).and_then(|s| s.try_into().ok()).map_or(0, u32::from_le_bytes)
}

/// A fixed-size name field up to its first zero, lowercased.
fn name_at(b: &[u8], at: usize, len: usize) -> String {
    let field = b.get(at..at + len).unwrap_or_default();
    let end = field.iter().position(|&c| c == 0).unwrap_or(field.len());
    String::from_utf8_lossy(field.get(..end).unwrap_or_default()).to_ascii_lowercase()
}

fn has_extension(path: &str, ext: &str) -> bool {
    path.rsplit_once('.').is_some_and(|(_, e)| e.eq_ignore_ascii_case(ext))
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Width, height and RGBA8 pixels, rows top to bottom.
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Picture {
    /// The average color of the opaque pixels.
    pub fn average(&self) -> (u8, u8, u8) {
        let (mut sum, mut n) = ([0u64; 3], 0u64);
        for p in self.rgba.as_chunks::<4>().0.iter().filter(|p| p[3] != 0) {
            for (s, &c) in sum.iter_mut().zip(p) {
                *s += u64::from(c);
            }
            n += 1;
        }
        let [r, g, b] = sum.map(|s| (s / n.max(1)) as u8);
        (r, g, b)
    }
}

/// A texture lump: its lowercase name, offset and size.
pub struct Lump {
    pub name: String,
    offset: u64,
    size: u64,
}

pub struct Wad {
    pub path: String,
    half_life: bool,
    pub lumps: Vec<Lump>,
}

impl Wad {
    pub fn open(files: &dyn Files, path: &str) -> Result<Self, String> {
        let header = files.read_range(path, 0, 12).ok_or("cannot be read")?;
        let half_life = match header.get(..4) {
            Some(b"WAD2") => false,
            Some(b"WAD3") => true,
            _ => return Err("is not a WAD2 or WAD3 file".into()),
        };
        let count = u64::from(u32_at(&header, 4)).min(MAX_LUMPS);
        let table =
            files.read_range(path, u64::from(u32_at(&header, 8)), count * 32).filter(|t| t.len() as u64 == count * 32).ok_or("has a truncated lump table")?;
        let kind = if half_life { MIPTEX_WAD3 } else { MIPTEX_WAD2 };
        let lumps = table
            .as_chunks::<32>()
            .0
            .iter()
            .filter(|e| e[12] == kind && e[13] == 0)
            .map(|e| Lump { name: name_at(e, 16, 16), offset: u64::from(u32_at(e, 0)), size: u64::from(u32_at(e, 4)) })
            .collect();
        Ok(Wad { path: path.into(), half_life, lumps })
    }

    /// The lump's picture at `mip` (0 is full size); Quake textures need Quake's palette, Half-Life's carry their own.
    pub fn picture(&self, files: &dyn Files, lump: &Lump, palette: Option<&[u8]>, mip: u32) -> Result<Picture, String> {
        let data = files.read_range(&self.path, lump.offset, lump.size.min(MAX_LUMP)).ok_or("cannot be read")?;
        let palette = if self.half_life { embedded_palette(&data).ok_or("has no palette")? } else { palette.ok_or("needs Quake's palette")? };
        decode(&data, palette, mip, !self.half_life && lump.name.starts_with("sky"), lump.name.starts_with('{'))
    }
}

/// The palette a WAD3 texture stores after its smallest mip.
fn embedded_palette(miptex: &[u8]) -> Option<&[u8]> {
    let (width, height) = (u32_at(miptex, 16) as usize, u32_at(miptex, 20) as usize);
    let at = (u32_at(miptex, 36) as usize).checked_add((width / 8).checked_mul(height / 8)?)?;
    let count = usize::from(u16_at(miptex, at));
    miptex.get(at.checked_add(2)?..at.checked_add(2 + count * 3)?)
}

/// Decodes one mip of a MIPTEX. A sky shows its left half, its front layer; `{` textures make index 255 clear.
pub fn decode(miptex: &[u8], palette: &[u8], mip: u32, sky: bool, fence: bool) -> Result<Picture, String> {
    let (width, height) = (u32_at(miptex, 16), u32_at(miptex, 20));
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Err(format!("has a bad size, {width} by {height}"));
    }
    let mip = mip.min(3);
    let (w, h) = ((width >> mip).max(1) as usize, (height >> mip).max(1) as usize);
    let at = u32_at(miptex, 24 + 4 * mip as usize) as usize;
    let indexes = miptex.get(at..at.saturating_add(w * h)).ok_or("is truncated")?;
    let shown = if sky { (w / 2).max(1) } else { w };
    let mut rgba = Vec::with_capacity(shown * h * 4);
    for row in indexes.chunks_exact(w) {
        for &i in row.get(..shown).unwrap_or_default() {
            let at = usize::from(i) * 3;
            let [r, g, b]: [u8; 3] = palette.get(at..at + 3).and_then(|c| c.try_into().ok()).unwrap_or_default();
            rgba.extend(if fence && i == 255 { [0, 0, 0, 0] } else { [r, g, b, 255] });
        }
    }
    Ok(Picture { width: shown as u32, height: h as u32, rgba })
}

/// A file inside a PAK archive, by its path there, ignoring case.
pub fn pak_file(files: &dyn Files, pak: &str, wanted: &str) -> Option<Vec<u8>> {
    let header = files.read_range(pak, 0, 12)?;
    if header.get(..4) != Some(b"PACK") {
        return None;
    }
    let dir = files.read_range(pak, u64::from(u32_at(&header, 4)), u64::from(u32_at(&header, 8)).min(MAX_PAK_DIR))?;
    let entry = dir.as_chunks::<64>().0.iter().find(|e| name_at(&e[..], 0, 56) == wanted)?;
    files.read_range(pak, u64::from(u32_at(entry, 56)), u64::from(u32_at(entry, 60)).min(MAX_LUMP))
}

/// Quake's palette from the user's own files: `gfx/palette.lmp` in id1's `pak0.pak` first, as the engine finds it.
pub fn palette(files: &dyn Files, listed: &[String]) -> Option<Vec<u8>> {
    let mut paks: Vec<&String> = listed.iter().filter(|p| has_extension(p, "pak")).collect();
    paks.sort_by_key(|p| {
        let p = p.to_ascii_lowercase();
        (p != "id1/pak0.pak", file_name(&p) != "pak0.pak")
    });
    let packed = paks.into_iter().filter_map(|p| pak_file(files, p, "gfx/palette.lmp"));
    let loose = ["id1/gfx/palette.lmp", "gfx/palette.lmp"].into_iter().filter_map(|p| files.read_range(p, 0, 768));
    packed.chain(loose).find(|p| p.len() >= 768)
}

/// Every WAD the plugin can read, in the order textures are looked up, and the palette.
pub struct Library {
    pub wads: Vec<Wad>,
    pub palette: Option<Vec<u8>>,
    pub problems: Vec<String>,
}

impl Library {
    /// The WADs in the readable folders, those the map's `wad` key names first.
    pub fn open(files: &dyn Files, wad_key: &str) -> Self {
        let listed = files.list(&["wad", "pak"]);
        let wads: Vec<&str> = listed.iter().map(String::as_str).filter(|p| has_extension(p, "wad")).collect();
        let mut order: Vec<&str> = Vec::new();
        let mut problems = Vec::new();
        for entry in wad_key.split(';').map(|e| e.trim().replace('\\', "/").to_ascii_lowercase()).filter(|e| !e.is_empty()) {
            // The key often holds paths on the mapper's own disk, so a WAD is matched by its file name, the closest path first.
            let mut found: Vec<&str> = wads.iter().copied().filter(|p| file_name(p).eq_ignore_ascii_case(file_name(&entry))).collect();
            found.sort_by_key(|p| !entry.ends_with(&p.to_ascii_lowercase()));
            match found.first() {
                Some(p) if !order.contains(p) => order.push(p),
                Some(_) => {}
                None => problems.push(format!("{entry}, which the map names, is not in the SDK's folders")),
            }
        }
        // Nested readable folders list one WAD under two paths.
        for p in wads {
            if !order.iter().any(|o| file_name(o).eq_ignore_ascii_case(file_name(p))) {
                order.push(p);
            }
        }
        let wads = order.into_iter().filter_map(|p| Wad::open(files, p).map_err(|e| problems.push(format!("{p} {e}"))).ok()).collect();
        Library { wads, palette: palette(files, &listed), problems }
    }

    /// The file names of the first WADs holding these textures, in lookup order, as a `wad` key.
    pub fn wad_key(&self, names: &BTreeSet<&str>) -> String {
        let needed: BTreeSet<usize> = names.iter().filter_map(|n| self.wads.iter().position(|w| w.lumps.iter().any(|l| l.name == *n))).collect();
        needed.into_iter().filter_map(|i| self.wads.get(i)).map(|w| file_name(&w.path)).collect::<Vec<_>>().join(";")
    }

    /// Pictures of the named textures, or of every texture with `None`, and why some could not be decoded.
    pub fn pictures(&self, files: &dyn Files, names: Option<&BTreeSet<String>>) -> (BTreeMap<String, Picture>, Vec<String>) {
        let mut chosen: BTreeMap<&str, (&Wad, &Lump)> = BTreeMap::new();
        for wad in &self.wads {
            for lump in wad.lumps.iter().filter(|l| names.is_none_or(|n| n.contains(&l.name))) {
                chosen.entry(&lump.name).or_insert((wad, lump));
            }
        }
        // A MIPTEX is about a third larger than its full-size indexes, which become four bytes each.
        let estimate = chosen.values().fold(0u64, |sum, (_, l)| sum.saturating_add(l.size.saturating_mul(3)));
        let mip = (0..4).find(|m| estimate >> (2 * m) <= PICTURE_BUDGET).unwrap_or(3);
        let (mut pictures, mut failed) = (BTreeMap::new(), Vec::new());
        for (name, (wad, lump)) in chosen {
            match wad.picture(files, lump, self.palette.as_deref(), mip) {
                Ok(p) => {
                    pictures.insert(name.to_string(), p);
                }
                Err(e) => failed.push(format!("{name} in {} {e}", wad.path)),
            }
        }
        (pictures, failed)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::HashMap;
    use std::io::{Read, Seek, SeekFrom};
    use std::path::PathBuf;

    use super::*;

    /// Files held in memory.
    pub struct Mem(pub HashMap<String, Vec<u8>>);

    impl Files for Mem {
        fn read_range(&self, path: &str, offset: u64, length: u64) -> Option<Vec<u8>> {
            let data = self.0.get(path)?;
            let start = (offset as usize).min(data.len());
            Some(data[start..(start + length as usize).min(data.len())].to_vec())
        }

        fn list(&self, extensions: &[&str]) -> Vec<String> {
            let mut out: Vec<String> = self.0.keys().filter(|p| extensions.iter().any(|e| has_extension(p, e))).cloned().collect();
            out.sort();
            out
        }
    }

    /// Files under folders on disk, like the SDK's readable roots.
    pub struct Disk(pub Vec<PathBuf>);

    impl Files for Disk {
        fn read_range(&self, path: &str, offset: u64, length: u64) -> Option<Vec<u8>> {
            self.0.iter().find_map(|root| {
                let mut file = std::fs::File::open(root.join(path)).ok()?;
                file.seek(SeekFrom::Start(offset)).ok()?;
                let mut out = Vec::new();
                file.take(length).read_to_end(&mut out).ok()?;
                Some(out)
            })
        }

        fn list(&self, extensions: &[&str]) -> Vec<String> {
            fn walk(root: &PathBuf, dir: PathBuf, extensions: &[&str], out: &mut Vec<String>) {
                for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        walk(root, path, extensions, out);
                    } else if let Ok(rel) = path.strip_prefix(root) {
                        let rel = rel.to_string_lossy().replace('\\', "/");
                        if extensions.iter().any(|e| has_extension(&rel, e)) {
                            out.push(rel);
                        }
                    }
                }
            }
            let mut out = Vec::new();
            for root in &self.0 {
                walk(root, root.clone(), extensions, &mut out);
            }
            out.sort();
            out.dedup();
            out
        }
    }

    /// A made-up palette, nothing like Quake's: index `i` is `(i, 255 - i, i / 2)`.
    pub fn test_palette() -> Vec<u8> {
        (0..=255u8).flat_map(|i| [i, 255 - i, i / 2]).collect()
    }

    /// A MIPTEX whose texel at (x, y) of mip `m` is `x + y * 16 + m`, with a WAD3 palette when given.
    pub fn miptex(name: &str, width: u32, height: u32, palette: Option<&[u8]>) -> Vec<u8> {
        let mut out = vec![0u8; 40];
        out[..name.len()].copy_from_slice(name.as_bytes());
        out[16..20].copy_from_slice(&width.to_le_bytes());
        out[20..24].copy_from_slice(&height.to_le_bytes());
        for m in 0..4 {
            let at = out.len() as u32;
            out[24 + 4 * m..28 + 4 * m].copy_from_slice(&at.to_le_bytes());
            let (w, h) = (width >> m, height >> m);
            out.extend((0..h).flat_map(|y| (0..w).map(move |x| (x + y * 16 + m as u32) as u8)));
        }
        if let Some(p) = palette {
            out.extend(((p.len() / 3) as u16).to_le_bytes());
            out.extend(p);
        }
        out
    }

    pub fn wad(magic: &[u8; 4], lumps: &[(&str, u8, Vec<u8>)]) -> Vec<u8> {
        let mut out = magic.to_vec();
        out.extend((lumps.len() as u32).to_le_bytes());
        out.extend([0; 4]);
        let mut table = Vec::new();
        for (name, kind, data) in lumps {
            table.extend((out.len() as u32).to_le_bytes());
            table.extend((data.len() as u32).to_le_bytes());
            table.extend((data.len() as u32).to_le_bytes());
            table.extend([*kind, 0, 0, 0]);
            let mut field = [0u8; 16];
            field[..name.len()].copy_from_slice(name.as_bytes());
            table.extend(field);
            out.extend(data);
        }
        let at = out.len() as u32;
        out[8..12].copy_from_slice(&at.to_le_bytes());
        out.extend(table);
        out
    }

    pub fn pak(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut out = b"PACK".to_vec();
        out.extend([0; 8]);
        let mut dir = Vec::new();
        for (name, data) in files {
            let mut field = [0u8; 56];
            field[..name.len()].copy_from_slice(name.as_bytes());
            dir.extend(field);
            dir.extend((out.len() as u32).to_le_bytes());
            dir.extend((data.len() as u32).to_le_bytes());
            out.extend(data);
        }
        let at = out.len() as u32;
        out[4..8].copy_from_slice(&at.to_le_bytes());
        out[8..12].copy_from_slice(&(dir.len() as u32).to_le_bytes());
        out.extend(dir);
        out
    }

    fn mem(files: Vec<(&str, Vec<u8>)>) -> Mem {
        Mem(files.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    #[test]
    fn decodes_mips_skies_and_fences() {
        let palette = test_palette();
        let tex = miptex("brick", 32, 16, None);
        let full = decode(&tex, &palette, 0, false, false).unwrap();
        assert_eq!((full.width, full.height), (32, 16));
        // Texel (3, 1) is index 19.
        assert_eq!(&full.rgba[(32 + 3) * 4..(32 + 4) * 4], &[19, 236, 9, 255]);
        let small = decode(&tex, &palette, 2, false, false).unwrap();
        assert_eq!((small.width, small.height, &small.rgba[..4]), (8, 4, &[2, 253, 1, 255][..]));
        let sky = decode(&tex, &palette, 0, true, false).unwrap();
        assert_eq!((sky.width, sky.height), (16, 16));
        assert_eq!(&sky.rgba[16 * 4..17 * 4], &full.rgba[32 * 4..33 * 4]);
        let mut fence = miptex("{grate", 16, 16, None);
        fence[40] = 255;
        let p = decode(&fence, &palette, 0, false, true).unwrap();
        assert_eq!((&p.rgba[..4], &p.rgba[4..8]), (&[0, 0, 0, 0][..], &[1, 254, 0, 255][..]));
        assert!(decode(&tex[..200], &palette, 0, false, false).is_err());
        assert!(decode(&tex[..10], &palette, 0, false, false).is_err());
        assert_eq!(Picture { width: 1, height: 2, rgba: vec![10, 20, 30, 255, 0, 0, 0, 0] }.average(), (10, 20, 30));
    }

    #[test]
    fn reads_wad2_and_wad3() {
        let wad2 =
            wad(b"WAD2", &[("Brick", 0x44, miptex("Brick", 16, 16, None)), ("palette", 0x40, test_palette()), ("sky1", 0x44, miptex("sky1", 32, 16, None))]);
        let hl_palette: Vec<u8> = (0..=255u8).flat_map(|i| [255 - i, i, 7]).collect();
        let wad3 = wad(b"WAD3", &[("{Fence", 0x43, miptex("{Fence", 16, 16, Some(&hl_palette)))]);
        let files = mem(vec![("a.wad", wad2.clone()), ("nested/A.WAD", wad2), ("hl/b.WAD", wad3), ("junk.wad", b"WAD2 not really".to_vec())]);
        let a = Wad::open(&files, "a.wad").unwrap();
        assert_eq!(a.lumps.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["brick", "sky1"]);
        let palette = test_palette();
        assert!(a.picture(&files, &a.lumps[0], None, 0).is_err());
        assert_eq!(a.picture(&files, &a.lumps[1], Some(&palette), 0).unwrap().width, 16);
        let b = Wad::open(&files, "hl/b.WAD").unwrap();
        let fence = b.picture(&files, &b.lumps[0], None, 0).unwrap();
        assert_eq!((&fence.rgba[..4], &fence.rgba[4..8]), (&[255, 0, 7, 255][..], &[254, 1, 7, 255][..]));
        let library = Library::open(&files, "");
        assert_eq!(library.wads.len(), 2);
        assert_eq!(library.problems.len(), 1, "{:?}", library.problems);
        assert!(Wad::open(&files, "missing.wad").is_err());
    }

    #[test]
    fn finds_the_palette_in_pak0() {
        let palette = test_palette();
        let mut other = palette.clone();
        other[0] = 99;
        let files = mem(vec![
            ("mod/pak0.pak", pak(&[("gfx/palette.lmp", other)])),
            ("id1/pak1.pak", pak(&[("maps/e1m1.bsp", vec![1, 2, 3])])),
            ("id1/pak0.pak", pak(&[("progs/player.mdl", vec![0; 10]), ("GFX/Palette.lmp", palette.clone())])),
            ("id1/gfx/palette.lmp", vec![1; 768]),
        ]);
        let listed = files.list(&["pak"]);
        assert_eq!(super::palette(&files, &listed), Some(palette));
        assert_eq!(pak_file(&files, "id1/pak1.pak", "maps/e1m1.bsp"), Some(vec![1, 2, 3]));
        assert_eq!(pak_file(&files, "id1/pak1.pak", "maps/e1m2.bsp"), None);
        let loose = mem(vec![("id1/gfx/palette.lmp", vec![1; 768]), ("id1/pak0.pak", b"PACK".to_vec())]);
        assert_eq!(super::palette(&loose, &loose.list(&["pak"])), Some(vec![1; 768]));
    }

    #[test]
    fn garbage_never_panics() {
        let palette = test_palette();
        let good = wad(b"WAD2", &[("brick", 0x44, miptex("brick", 16, 16, None))]);
        let hl = wad(b"WAD3", &[("brick", 0x43, miptex("brick", 16, 16, Some(&palette)))]);
        let archive = pak(&[("gfx/palette.lmp", palette.clone())]);
        let mut seed = 0x2545_f491_u32;
        let mut noise = |n: usize| -> Vec<u8> {
            (0..n)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    seed as u8
                })
                .collect()
        };
        let mut inputs: Vec<Vec<u8>> = Vec::new();
        for file in [&good, &hl, &archive] {
            inputs.extend((0..file.len()).step_by(7).map(|n| file[..n].to_vec()));
            for at in (0..file.len()).step_by(3) {
                let mut bent = file.clone();
                bent[at] ^= 0xa5;
                inputs.push(bent);
            }
        }
        inputs.extend((0..200).map(|n| noise(n * 5)));
        for (n, data) in inputs.into_iter().enumerate() {
            for magic in [&b"WAD2"[..], b"WAD3", b"PACK"] {
                let mut data = data.clone();
                if n % 2 == 0 && data.len() >= 4 {
                    data[..4].copy_from_slice(magic);
                }
                let files = mem(vec![("x.wad", data.clone()), ("id1/pak0.pak", data.clone())]);
                let library = Library::open(&files, "x.wad;nope.wad");
                let _ = library.pictures(&files, None);
                let _ = pak_file(&files, "id1/pak0.pak", "gfx/palette.lmp");
                let _ = decode(&data, &palette, 0, true, true);
                let _ = embedded_palette(&data);
            }
        }
    }

    const WAD_DIRS: &[&str] = &["/mnt/Storage/Quake/WAD", "/home/mark/Source/quake-cli-tools/prototype_all_wads_1_3", "/home/mark/Downloads/dll_cbble2013_2"];
    const QUAKE: &str = "/home/mark/.quakeforge";

    #[test]
    fn decodes_real_wads() {
        let roots: Vec<PathBuf> = WAD_DIRS.iter().chain([&QUAKE]).map(PathBuf::from).filter(|p| p.is_dir()).collect();
        if roots.is_empty() {
            eprintln!("skipped: no WAD folders");
            return;
        }
        let files = Disk(roots);
        let library = Library::open(&files, "");
        assert!(library.problems.is_empty(), "{:?}", library.problems);
        let Some(palette) = library.palette.as_deref() else {
            eprintln!("skipped: no pak0.pak with a palette");
            return;
        };
        let mut checked = 0;
        for wad in &library.wads {
            let (mut fences, mut skies) = (0, 0);
            for lump in &wad.lumps {
                let data = files.read_range(&wad.path, lump.offset, lump.size).unwrap();
                let (width, height) = (u32_at(&data, 16), u32_at(&data, 20));
                let p = wad.picture(&files, lump, Some(palette), 0).unwrap_or_else(|e| panic!("{} in {}: {e}", lump.name, wad.path));
                let sky = !wad.half_life && lump.name.starts_with("sky");
                assert_eq!((p.width, p.height), (if sky { width / 2 } else { width }, height), "{}", lump.name);
                assert_eq!(p.rgba.len(), (p.width * p.height * 4) as usize);
                if lump.name.starts_with('{') {
                    assert!(p.rgba.as_chunks::<4>().0.iter().any(|c| c[3] == 0), "{} has no clear pixels", lump.name);
                    fences += 1;
                }
                skies += usize::from(sky);
                checked += 1;
            }
            eprintln!("{}: {} textures, {fences} fences, {skies} skies", wad.path, wad.lumps.len());
        }
        assert!(checked > 0);
    }
}
