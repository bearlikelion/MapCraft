//! The pak lump of a compiled Source map: a zip of files the game reads before its own, such as custom materials.

const HEADER: usize = 8 + 64 * 16 + 4;
const PAKFILE: usize = 40;

fn u16_at(b: &[u8], at: usize) -> usize {
    usize::from(valve::u16_at(b, at))
}

fn u32_at(b: &[u8], at: usize) -> usize {
    valve::u32_at(b, at) as usize
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// One file of the new zip: its local header and data, and its central directory record without the offset.
struct Entry {
    name: String,
    local: Vec<u8>,
    central: Vec<u8>,
}

fn stored(name: &str, data: &[u8]) -> Entry {
    let crc = crc32(data);
    let size = data.len() as u32;
    let n = name.len() as u16;
    let mut local = Vec::with_capacity(30 + name.len() + data.len());
    local.extend_from_slice(b"PK\x03\x04");
    local.extend_from_slice(&[10, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    local.extend_from_slice(&crc.to_le_bytes());
    local.extend_from_slice(&size.to_le_bytes());
    local.extend_from_slice(&size.to_le_bytes());
    local.extend_from_slice(&n.to_le_bytes());
    local.extend_from_slice(&0u16.to_le_bytes());
    local.extend_from_slice(name.as_bytes());
    local.extend_from_slice(data);
    let mut central = Vec::with_capacity(46 + name.len());
    central.extend_from_slice(b"PK\x01\x02");
    central.extend_from_slice(&[20, 0, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    central.extend_from_slice(&crc.to_le_bytes());
    central.extend_from_slice(&size.to_le_bytes());
    central.extend_from_slice(&size.to_le_bytes());
    central.extend_from_slice(&n.to_le_bytes());
    central.extend_from_slice(&[0; 12]);
    central.extend_from_slice(&[0; 4]);
    central.extend_from_slice(name.as_bytes());
    Entry { name: name.to_string(), local, central }
}

/// The zip's files as they are, keeping their compression; a file the zip cannot account for ends the list.
fn existing(zip: &[u8]) -> Vec<Entry> {
    let mut out = Vec::new();
    let tail = zip.len().saturating_sub(22 + 65535);
    let Some(end) = (tail..zip.len().saturating_sub(21)).rev().find(|&at| zip.get(at..at + 4) == Some(b"PK\x05\x06")) else { return out };
    let (count, mut at) = (u16_at(zip, end + 10), u32_at(zip, end + 16));
    for _ in 0..count {
        if zip.get(at..at + 4) != Some(b"PK\x01\x02") {
            break;
        }
        let (packed, name_len, extra, comment, local) =
            (u32_at(zip, at + 20), u16_at(zip, at + 28), u16_at(zip, at + 30), u16_at(zip, at + 32), u32_at(zip, at + 42));
        let record = at + 46 + name_len + extra + comment;
        let (Some(name), Some(central)) = (zip.get(at + 46..at + 46 + name_len), zip.get(at..record)) else { break };
        if zip.get(local..local + 4) != Some(b"PK\x03\x04") {
            break;
        }
        let end = local + 30 + u16_at(zip, local + 26) + u16_at(zip, local + 28) + packed;
        let Some(local) = zip.get(local..end) else { break };
        out.push(Entry { name: String::from_utf8_lossy(name).to_ascii_lowercase().replace('\\', "/"), local: local.to_vec(), central: central.to_vec() });
        at = record;
    }
    out
}

/// The compiled map `bsp` with `files` in its pak lump, replacing files of the same path; the new lump goes at the end.
pub fn embed(bsp: &[u8], files: &[(String, Vec<u8>)]) -> Result<Vec<u8>, String> {
    if bsp.len() < HEADER || !bsp.starts_with(b"VBSP") {
        return Err("not a compiled Source map".into());
    }
    let lump = 8 + PAKFILE * 16;
    let (offset, length) = (u32_at(bsp, lump), u32_at(bsp, lump + 4));
    let old = if length == 0 { &[][..] } else { bsp.get(offset..offset.saturating_add(length)).ok_or("the map's pak lump lies outside the file")? };
    let names: Vec<String> = files.iter().map(|(n, _)| n.to_ascii_lowercase()).collect();
    let mut entries: Vec<Entry> = existing(old).into_iter().filter(|e| !names.contains(&e.name)).collect();
    entries.extend(files.iter().zip(&names).map(|((_, data), name)| stored(name, data)));
    let mut zip = Vec::new();
    let mut directory = Vec::new();
    for e in &entries {
        let mut central = e.central.clone();
        central.splice(42..46, (zip.len() as u32).to_le_bytes());
        directory.extend_from_slice(&central);
        zip.extend_from_slice(&e.local);
    }
    let directory_at = zip.len() as u32;
    zip.extend_from_slice(&directory);
    zip.extend_from_slice(b"PK\x05\x06");
    zip.extend_from_slice(&[0; 4]);
    zip.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    zip.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    zip.extend_from_slice(&(directory.len() as u32).to_le_bytes());
    zip.extend_from_slice(&directory_at.to_le_bytes());
    zip.extend_from_slice(&0u16.to_le_bytes());

    let mut out = bsp.to_vec();
    // The old lump stays as unread bytes when it is not at the end already.
    if offset + length == out.len() && length > 0 {
        out.truncate(offset);
    }
    out.resize(out.len().next_multiple_of(4), 0);
    let at = u32::try_from(out.len()).map_err(|_| "the map is too big for a Source map")?;
    out.extend_from_slice(&zip);
    out.splice(lump..lump + 16, [at.to_le_bytes(), (zip.len() as u32).to_le_bytes(), [0; 4], [0; 4]].concat());
    Ok(out)
}

/// A material that draws `name` as its base texture, see-through when the picture has clear texels.
pub fn vmt(name: &str, alpha: Alpha) -> String {
    let extra = match alpha {
        Alpha::Opaque => "",
        Alpha::Cutout => "\t\"$alphatest\" \"1\"\n",
        Alpha::Blended => "\t\"$translucent\" \"1\"\n",
    };
    format!("\"LightmappedGeneric\"\n{{\n\t\"$basetexture\" \"{name}\"\n{extra}}}\n")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alpha {
    Opaque,
    Cutout,
    Blended,
}

pub fn alpha_of(rgba: &[u8]) -> Alpha {
    let mut out = Alpha::Opaque;
    for a in rgba.as_chunks::<4>().0.iter().map(|p| p[3]) {
        match a {
            255 => {}
            0 => out = Alpha::Cutout,
            _ => return Alpha::Blended,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoFiles;

    impl valve::Files for NoFiles {
        fn read(&self, _path: &str) -> Option<Vec<u8>> {
            None
        }

        fn read_range(&self, _path: &str, _offset: u64, _length: u64) -> Option<Vec<u8>> {
            None
        }
    }

    fn pak(bsp: &[u8]) -> Vec<u8> {
        let lump = 8 + PAKFILE * 16;
        let (at, len) = (u32_at(bsp, lump), u32_at(bsp, lump + 4));
        bsp[at..at + len].to_vec()
    }

    #[test]
    fn adds_and_replaces_files_in_the_pak_lump() {
        let mut bsp = vec![0u8; HEADER + 100];
        bsp[..4].copy_from_slice(b"VBSP");
        bsp[4] = 20;
        let first = embed(&bsp, &[("materials/A.vmt".into(), b"one".to_vec()), ("materials/keep.vmt".into(), b"kept".to_vec())]).unwrap();
        assert_eq!(&first[HEADER..HEADER + 100], &bsp[HEADER..HEADER + 100], "the rest of the map is untouched");
        let second = embed(&first, &[("materials/a.vmt".into(), b"two".to_vec())]).unwrap();
        let zip = pak(&second);
        let files = NoFiles;
        let library = valve::Library::new(&zip, &files);
        assert_eq!(library.read("materials/a.vmt").as_deref(), Some(&b"two"[..]), "a file of the same path is replaced");
        assert_eq!(library.read("materials/keep.vmt").as_deref(), Some(&b"kept"[..]));
        assert!(second.len() < first.len() + 200, "the old lump at the end is replaced, not kept");
        assert!(embed(b"IBSP", &[]).is_err());
    }

    #[test]
    fn crc_matches_zip() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn materials_say_how_they_are_seen_through() {
        assert_eq!(alpha_of(&[1, 2, 3, 255, 4, 5, 6, 0]), Alpha::Cutout);
        assert_eq!(alpha_of(&[1, 2, 3, 128]), Alpha::Blended);
        assert!(vmt("ramps-orange1", Alpha::Cutout).contains("\"$basetexture\" \"ramps-orange1\"\n\t\"$alphatest\" \"1\"\n}"));
    }
}
