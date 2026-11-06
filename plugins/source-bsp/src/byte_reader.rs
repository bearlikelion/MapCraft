//! Little-endian readers, C strings and Valve LZMA blocks, as in SourceGD's `core/byte_reader`.

use std::borrow::Cow;

use glam::DVec3;
use valve::{bytes, lzma, u32_at};

pub(crate) fn i16_at(b: &[u8], at: usize) -> i16 {
    i16::from_le_bytes(bytes(b, at))
}

pub(crate) fn i32_at(b: &[u8], at: usize) -> i32 {
    i32::from_le_bytes(bytes(b, at))
}

pub(crate) fn f32_at(b: &[u8], at: usize) -> f64 {
    f64::from(f32::from_le_bytes(bytes(b, at)))
}

pub(crate) fn vec_at(b: &[u8], at: usize) -> DVec3 {
    DVec3::new(f32_at(b, at), f32_at(b, at + 4), f32_at(b, at + 8))
}

pub(crate) fn index(v: i32) -> usize {
    usize::try_from(v).unwrap_or(usize::MAX)
}

/// The NUL-terminated string at the start of `b`.
pub(crate) fn c_str(b: &[u8]) -> Cow<'_, str> {
    String::from_utf8_lossy(b.split(|&c| c == 0).next().unwrap_or_default())
}

/// Valve's LZMA block: `LZMA`, uncompressed size, compressed size, 5 property bytes, then the stream.
pub(crate) fn unlzma(raw: &[u8]) -> Result<Vec<u8>, String> {
    lzma(raw.get(12..17).ok_or("truncated LZMA header")?, raw.get(17..).unwrap_or_default(), u32_at(raw, 4) as usize, usize::MAX)
}
