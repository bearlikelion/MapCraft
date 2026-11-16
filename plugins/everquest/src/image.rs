//! The pictures S3D archives hold: Windows bitmaps, and DDS files that keep a `.bmp` name.

use crate::read::Reader;

const MAX_SIDE: usize = 4096;

/// Width, height and RGBA8 pixels, rows top to bottom.
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Picture {
    pub fn average(&self) -> (u8, u8, u8) {
        let n = (self.rgba.len() / 4).max(1) as u64;
        let sum = self.rgba.as_chunks::<4>().0.iter().fold([0u64; 3], |s, p| [s[0] + u64::from(p[0]), s[1] + u64::from(p[1]), s[2] + u64::from(p[2])]);
        let [r, g, b] = sum.map(|c| (c / n) as u8);
        (r, g, b)
    }
}

/// Decodes a bitmap or DDS; `masked` makes a bitmap's first palette color clear, as EverQuest's masked materials draw it.
pub fn decode(data: &[u8], masked: bool) -> Result<Picture, String> {
    match data.get(..4) {
        Some(b"DDS ") => dds(data),
        Some([b'B', b'M', ..]) => bmp(data, masked),
        _ => Err("not a BMP or DDS picture".into()),
    }
}

fn side(v: i64) -> Result<usize, String> {
    usize::try_from(v).ok().filter(|v| (1..=MAX_SIDE).contains(v)).ok_or_else(|| format!("a picture {v} pixels across is not supported"))
}

fn dds(data: &[u8]) -> Result<Picture, String> {
    let mut r = Reader::new(data);
    r.seek(12)?;
    let (h, w) = (side(r.u32()?.into())?, side(r.u32()?.into())?);
    r.seek(84)?;
    let kind = match r.bytes(4)? {
        b"DXT1" => 1,
        b"DXT3" => 3,
        b"DXT5" => 5,
        other => return Err(format!("DDS format {} is not supported", String::from_utf8_lossy(other))),
    };
    let blocks = data.get(128..).unwrap_or_default();
    let rgba = valve::vtf::dxt(blocks, kind, w, h).ok_or("bad DDS format")?;
    Ok(Picture { width: w as u32, height: h as u32, rgba })
}

fn bmp(data: &[u8], masked: bool) -> Result<Picture, String> {
    let mut r = Reader::new(data);
    r.seek(10)?;
    let pixels = r.u32()? as usize;
    let header = r.u32()? as usize;
    let (w, h) = (r.i32()?, r.i32()?);
    r.skip(2)?;
    let bits = r.u16()?;
    if r.u32()? != 0 {
        return Err("compressed bitmaps are not supported".into());
    }
    r.skip(12)?;
    let used = r.u32()? as usize;
    let (width, height) = (side(w.into())?, side(i64::from(h).abs())?);
    let palette_len = if used == 0 && bits <= 8 { 1 << bits } else { used.min(256) };
    let palette: Vec<[u8; 4]> = data
        .get(header.saturating_add(14)..)
        .unwrap_or_default()
        .as_chunks::<4>()
        .0
        .iter()
        .take(palette_len)
        .enumerate()
        .map(|(i, c)| [c[2], c[1], c[0], if masked && i == 0 { 0 } else { 255 }])
        .collect();
    let stride = (width * usize::from(bits)).div_ceil(32) * 4;
    let mut rgba = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        let row = if h > 0 { height - 1 - y } else { y };
        let line = data.get(pixels.saturating_add(row * stride)..).unwrap_or_default();
        for x in 0..width {
            let px = match bits {
                8 => palette.get(usize::from(line.get(x).copied().unwrap_or(0))).copied().unwrap_or([0, 0, 0, 255]),
                4 => {
                    let b = line.get(x / 2).copied().unwrap_or(0);
                    let i = if x % 2 == 0 { b >> 4 } else { b & 15 };
                    palette.get(usize::from(i)).copied().unwrap_or([0, 0, 0, 255])
                }
                16 => {
                    let v = u16::from_le_bytes([line.get(x * 2).copied().unwrap_or(0), line.get(x * 2 + 1).copied().unwrap_or(0)]);
                    let c = |s: u16| ((v >> s & 31) * 255 / 31) as u8;
                    [c(10), c(5), c(0), 255]
                }
                24 | 32 => {
                    let n = usize::from(bits / 8);
                    let p = line.get(x * n..x * n + 3).unwrap_or(&[0, 0, 0]);
                    [p[2], p[1], p[0], 255]
                }
                _ => return Err(format!("{bits} bit bitmaps are not supported")),
            };
            rgba.extend_from_slice(&px);
        }
    }
    Ok(Picture { width: width as u32, height: height as u32, rgba })
}
