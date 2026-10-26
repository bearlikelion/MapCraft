//! Valve formats the Source plugins share: VPK packages, VMT materials, VTF textures, KeyValues text and LZMA.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod materials;
pub mod vtf;

use std::io::Read;

pub use materials::{Files, Library, Origin, Token, tokens, value};

const MAX_UNPACKED: usize = 256 << 20;

pub fn bytes<const N: usize>(b: &[u8], at: usize) -> [u8; N] {
    at.checked_add(N).and_then(|end| b.get(at..end)).and_then(|s| s.try_into().ok()).unwrap_or([0; N])
}

pub fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(bytes(b, at))
}

pub fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes(b, at))
}

/// The first `limit` bytes of a raw LZMA stream that unpacks to `size` bytes, given its 5 property bytes.
pub fn lzma(props: &[u8], stream: &[u8], size: usize, limit: usize) -> Result<Vec<u8>, String> {
    let len = size.min(limit);
    if len > MAX_UNPACKED {
        return Err(format!("{size} bytes uncompressed is more than MapCraft reads"));
    }
    let (&lc_lp_pb, dict) = props.split_first().ok_or("truncated LZMA header")?;
    let dict = u32::from_le_bytes(dict.try_into().map_err(|_| "truncated LZMA header")?);
    let mut reader = lzma_rust2::LzmaReader::new_with_props(stream, size as u64, lc_lp_pb, dict, None).map_err(|e| format!("LZMA: {e}"))?;
    let mut out = vec![0; len];
    reader.read_exact(&mut out).map_err(|e| format!("LZMA: {e}"))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lzma_prefix_matches_the_whole() {
        use std::io::Write;
        let data: Vec<u8> = (0..200_000u32).map(|i| ((i % 251) ^ ((i / 997) % 7)) as u8).collect();
        let mut writer = lzma_rust2::LzmaWriter::new_use_header(Vec::new(), &lzma_rust2::LzmaOptions::default(), Some(data.len() as u64)).unwrap();
        writer.write_all(&data).unwrap();
        let packed = writer.finish().unwrap();
        let (props, stream) = (&packed[..5], &packed[13..]);
        for limit in [1, 4095, 4096, 77_777, 199_999, 200_000, usize::MAX] {
            assert_eq!(lzma(props, stream, data.len(), limit).unwrap(), data[..limit.min(data.len())], "{limit}");
        }
        assert!(lzma(props, &stream[..stream.len() / 2], data.len(), usize::MAX).is_err());
    }
}
