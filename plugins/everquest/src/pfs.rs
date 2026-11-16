//! PFS archives (`.s3d`, `.eqg`): zlib compressed files named by a directory entry of their own.

use std::collections::BTreeMap;

use crate::read::Reader;

/// The CRC EverQuest gives the entry holding the file names.
const NAMES_CRC: u32 = 0x6158_0ac9;
const MAX_ENTRIES: usize = 65_536;
/// No file in a real archive comes near this; it stops a corrupt size from filling memory.
const MAX_FILE: usize = 256 << 20;

/// Each file's compressed blocks, by lowercase name.
pub struct Archive<'a> {
    data: &'a [u8],
    files: BTreeMap<String, (usize, usize)>,
}

impl<'a> Archive<'a> {
    pub fn open(data: &'a [u8]) -> Result<Self, String> {
        let mut r = Reader::new(data);
        let dir = r.u32()? as usize;
        if data.get(4..8) != Some(b"PFS ") {
            return Err("not a PFS archive (.s3d or .eqg)".into());
        }
        r.seek(dir)?;
        let count = r.u32()? as usize;
        if count > MAX_ENTRIES {
            return Err(format!("{count} files is too many for a PFS archive"));
        }
        let mut entries = Vec::with_capacity(count);
        let mut names = None;
        for _ in 0..count {
            let (crc, offset, size) = (r.u32()?, r.u32()? as usize, r.u32()? as usize);
            if crc == NAMES_CRC {
                names = Some((offset, size));
            } else {
                entries.push((offset, size));
            }
        }
        let (offset, size) = names.ok_or("the archive has no file names")?;
        let list = inflate(data, offset, size)?;
        let mut r = Reader::new(&list);
        let mut names = Vec::new();
        for _ in 0..r.u32()?.min(MAX_ENTRIES as u32) {
            let len = r.u32()? as usize;
            let bytes = r.bytes(len)?;
            let name = bytes.split(|&b| b == 0).next().unwrap_or_default();
            names.push(String::from_utf8_lossy(name).to_ascii_lowercase());
        }
        // Names are listed in the order the files are stored.
        entries.sort_unstable();
        let files = names.into_iter().zip(entries).collect();
        Ok(Self { data, files })
    }

    /// The file `name`, any case.
    pub fn read(&self, name: &str) -> Result<Vec<u8>, String> {
        let &(offset, size) = self.files.get(&name.to_ascii_lowercase()).ok_or_else(|| format!("{name} is not in the archive"))?;
        inflate(self.data, offset, size)
    }
}

/// Blocks of (compressed length, inflated length, zlib data) from `offset` until `size` bytes are out.
fn inflate(data: &[u8], offset: usize, size: usize) -> Result<Vec<u8>, String> {
    if size > MAX_FILE {
        return Err(format!("a file of {size} bytes is too large"));
    }
    let mut out = Vec::with_capacity(size);
    let mut r = Reader::new(data);
    r.seek(offset)?;
    while out.len() < size {
        let (packed, unpacked) = (r.u32()? as usize, r.u32()? as usize);
        let room = size - out.len();
        let block =
            miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(r.bytes(packed)?, room).map_err(|e| format!("a compressed block is broken: {e}"))?;
        if block.len() != unpacked || block.is_empty() {
            return Err("a compressed block has the wrong size".into());
        }
        out.extend_from_slice(&block);
    }
    Ok(out)
}
