//! The `.mclight` file: a header, the face table, then every slice's layers zlib-compressed.
//! See `docs/mclight-format.md`.

use mapcraft_map::NodeId;

use crate::{BakeError, FaceMap, Lightmap, Slice};

const MAGIC: &[u8; 4] = b"MCLM";
/// Version 2 stores the L1 bands along Godot's axes; version 1 had the map's, which Godot misread.
const VERSION: u32 = 2;
const MAX_SIZE: u32 = 8192;
const MAX_SLICES: u32 = 64;
const MAX_FACES: u32 = 4_000_000;
/// Uncompressed texels may not pass this, matching what Godot's decompress takes.
const MAX_BYTES: u64 = 2 << 30;

pub fn write(map: &Lightmap) -> Vec<u8> {
    let mut out = Vec::with_capacity(32 + map.faces.len() * 48);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&map.fingerprint.to_le_bytes());
    out.extend_from_slice(&map.size.to_le_bytes());
    out.extend_from_slice(&(map.slices.len() as u32).to_le_bytes());
    out.extend_from_slice(&(map.faces.len() as u32).to_le_bytes());
    for f in &map.faces {
        out.extend_from_slice(&f.node.0.to_le_bytes());
        out.extend_from_slice(&f.face.to_le_bytes());
        out.extend_from_slice(&f.slice.to_le_bytes());
        for v in f.uv.iter().flatten() {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    let mut texels = Vec::with_capacity(map.slices.len() * 4 * (map.size as usize).pow(2) * 4);
    for v in map.slices.iter().flat_map(|s| s.layers.iter().flatten()) {
        texels.extend_from_slice(&v.to_le_bytes());
    }
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&texels, 6);
    out.extend_from_slice(&(packed.len() as u64).to_le_bytes());
    out.extend_from_slice(&packed);
    out
}

/// The fingerprint in a lightmap file's header, read without decoding the rest.
pub fn fingerprint_of(bytes: &[u8]) -> Option<u64> {
    let header = bytes.get(..16)?;
    if &header[..4] != MAGIC {
        return None;
    }
    header.get(8..16).and_then(|b| b.try_into().ok()).map(u64::from_le_bytes)
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], BakeError> {
        let (head, rest) = self.0.split_at_checked(n).ok_or_else(|| BakeError::Damaged("it ends early".into()))?;
        self.0 = rest;
        Ok(head)
    }

    fn u32(&mut self) -> Result<u32, BakeError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Result<u64, BakeError> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }

    fn f32(&mut self) -> Result<f32, BakeError> {
        Ok(f32::from_bits(self.u32()?))
    }
}

pub fn read(bytes: &[u8]) -> Result<Lightmap, BakeError> {
    let mut r = Reader(bytes);
    if r.take(4).map_err(|_| BakeError::NotLightmap)? != MAGIC {
        return Err(BakeError::NotLightmap);
    }
    let version = r.u32()?;
    if version != VERSION {
        return Err(BakeError::Version(version));
    }
    let fingerprint = r.u64()?;
    let size = r.u32()?;
    let count = r.u32()?;
    let faces = r.u32()?;
    if size == 0 || size > MAX_SIZE || count > MAX_SLICES || faces > MAX_FACES {
        return Err(BakeError::Damaged(format!("{count} slices of {size} and {faces} faces is out of range")));
    }
    let mut table = Vec::with_capacity(faces as usize);
    for _ in 0..faces {
        let node = NodeId(r.u64()?);
        let face = r.u32()?;
        let slice = r.u32()?;
        let mut uv = [[0f32; 4]; 2];
        for v in uv.iter_mut().flatten() {
            *v = r.f32()?;
        }
        if slice >= count || uv.iter().flatten().any(|v| !v.is_finite()) {
            return Err(BakeError::Damaged(format!("face {face} of node {} has a bad slice or mapping", node.0)));
        }
        table.push(FaceMap { node, face, slice, uv });
    }
    let len = r.u64()?;
    let packed = r.take(usize::try_from(len).map_err(|_| BakeError::Damaged("texel data too large".into()))?)?;
    let layer = (size as usize) * (size as usize);
    let expected = u64::from(count) * 4 * layer as u64 * 4;
    if expected > MAX_BYTES {
        return Err(BakeError::Damaged("texel data too large".into()));
    }
    let texels = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(packed, expected as usize)
        .map_err(|e| BakeError::Damaged(format!("texels do not decompress: {e:?}")))?;
    if texels.len() as u64 != expected {
        return Err(BakeError::Damaged(format!("{} texel bytes where {expected} were expected", texels.len())));
    }
    let words: Vec<u32> = texels.as_chunks::<4>().0.iter().map(|b| u32::from_le_bytes(*b)).collect();
    let slices = words
        .chunks_exact(layer * 4)
        .map(|s| Slice { layers: std::array::from_fn(|k| s.get(k * layer..(k + 1) * layer).map(<[u32]>::to_vec).unwrap_or_default()) })
        .collect();
    Ok(Lightmap { fingerprint, size, slices, faces: table })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Lightmap {
        Lightmap {
            fingerprint: 0xDEAD_BEEF_0123,
            size: 4,
            slices: vec![Slice { layers: std::array::from_fn(|k| (0..16).map(|i| i * 4 + k as u32).collect()) }],
            faces: vec![FaceMap { node: NodeId(7), face: 2, slice: 0, uv: [[0.1, 0.0, 0.0, 0.5], [0.0, 0.2, 0.0, 0.25]] }],
        }
    }

    #[test]
    fn a_lightmap_survives_a_round_trip() {
        assert_eq!(read(&write(&sample())).unwrap(), sample());
    }

    #[test]
    fn the_header_fingerprint_reads_alone() {
        assert_eq!(fingerprint_of(&write(&sample())), Some(0xDEAD_BEEF_0123));
        assert_eq!(fingerprint_of(b"MCLM"), None);
    }

    #[test]
    fn damaged_files_are_errors() {
        let bytes = write(&sample());
        assert!(matches!(read(b"nope"), Err(BakeError::NotLightmap)));
        for cut in [8, 20, 40, bytes.len() - 3] {
            assert!(read(&bytes[..cut]).is_err(), "cut at {cut}");
        }
        let mut huge = bytes.clone();
        huge[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(read(&huge).is_err());
        let mut newer = bytes;
        newer[4..8].copy_from_slice(&9u32.to_le_bytes());
        assert!(matches!(read(&newer), Err(BakeError::Version(9))));
    }
}
