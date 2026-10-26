//! Decodes one picture from a Valve texture (VTF): frame 0, face 0, the largest mip that fits.

const RGBA8888: u32 = 0;
const ABGR8888: u32 = 1;
const RGB888: u32 = 2;
const BGR888: u32 = 3;
const I8: u32 = 5;
const IA88: u32 = 6;
const ARGB8888: u32 = 11;
const BGRA8888: u32 = 12;
const DXT1: u32 = 13;
const DXT3: u32 = 14;
const DXT5: u32 = 15;
const BGRX8888: u32 = 16;
const DXT1_ONEBITALPHA: u32 = 20;
const ENVMAP: u32 = 0x4000;

/// Width, height and RGBA8 pixels, rows top to bottom.
pub type Picture = (u32, u32, Vec<u8>);

fn u8_at(b: &[u8], at: usize) -> usize {
    usize::from(b.get(at).copied().unwrap_or(0))
}

fn u16_at(b: &[u8], at: usize) -> usize {
    u8_at(b, at) | u8_at(b, at + 1) << 8
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    (u16_at(b, at) | u16_at(b, at + 2) << 16) as u32
}

/// Bytes of one `w` by `h` image, or `None` for a format this reader doesn't know.
fn image_size(format: u32, w: usize, h: usize) -> Option<usize> {
    let blocks = w.div_ceil(4).max(1).checked_mul(h.div_ceil(4).max(1))?;
    let bytes = match format {
        DXT1 | DXT1_ONEBITALPHA => return blocks.checked_mul(8),
        DXT3 | DXT5 => return blocks.checked_mul(16),
        I8 | 7 | 8 => 1,
        4 | IA88 | 17 | 18 | 19 | 21 | 22 => 2,
        RGB888 | BGR888 | 9 | 10 => 3,
        RGBA8888 | ABGR8888 | ARGB8888 | BGRA8888 | BGRX8888 | 23 | 26 => 4,
        24 | 25 => 8,
        _ => return None,
    };
    w.checked_mul(h)?.checked_mul(bytes)
}

fn rgb565(c: usize) -> [u8; 3] {
    let (r, g, b) = ((c >> 11) & 31, (c >> 5) & 63, c & 31);
    [(r << 3 | r >> 2) as u8, (g << 2 | g >> 4) as u8, (b << 3 | b >> 2) as u8]
}

/// The 16 pixels of a DXT color block; `alpha_mode` allows DXT1's transparent fourth color.
fn color_block(block: &[u8], alpha_mode: bool) -> [[u8; 4]; 16] {
    let (c0, c1) = (u16_at(block, 0), u16_at(block, 2));
    let (a, b) = (rgb565(c0).map(u32::from), rgb565(c1).map(u32::from));
    let mix = |wa: u32, wb: u32| {
        let [r, g, bl] = [0, 1, 2].map(|i| ((a[i] * wa + b[i] * wb) / (wa + wb)) as u8);
        [r, g, bl, 255]
    };
    let palette = if c0 > c1 || !alpha_mode { [mix(1, 0), mix(0, 1), mix(2, 1), mix(1, 2)] } else { [mix(1, 0), mix(0, 1), mix(1, 1), [0, 0, 0, 0]] };
    let bits = u32_at(block, 4);
    std::array::from_fn(|i| palette[((bits >> (2 * i)) & 3) as usize])
}

fn dxt5_alpha(block: &[u8]) -> [u8; 16] {
    let (a0, a1) = (u8_at(block, 0), u8_at(block, 1));
    let mut table = [a0, a1, 0, 0, 0, 0, 0, 255];
    if a0 > a1 {
        for (k, slot) in table.iter_mut().enumerate().skip(2) {
            *slot = ((8 - k) * a0 + (k - 1) * a1) / 7;
        }
    } else {
        for (k, slot) in table.iter_mut().enumerate().take(6).skip(2) {
            *slot = ((6 - k) * a0 + (k - 1) * a1) / 5;
        }
    }
    let bits = (2..8).rev().fold(0u64, |acc, i| acc << 8 | u8_at(block, i) as u64);
    std::array::from_fn(|i| table[((bits >> (3 * i)) & 7) as usize] as u8)
}

fn decode_dxt(data: &[u8], format: u32, w: usize, h: usize) -> Vec<u8> {
    let stride = if matches!(format, DXT3 | DXT5) { 16 } else { 8 };
    let mut out = vec![0u8; w * h * 4];
    for (n, block) in data.chunks_exact(stride).take(w.div_ceil(4) * h.div_ceil(4)).enumerate() {
        let (bx, by) = (n % w.div_ceil(4) * 4, n / w.div_ceil(4) * 4);
        let mut pixels = color_block(&block[stride - 8..], !matches!(format, DXT3 | DXT5));
        if format == DXT3 {
            for (i, p) in pixels.iter_mut().enumerate() {
                p[3] = ((u8_at(block, i / 2) >> (4 * (i % 2))) & 15) as u8 * 17;
            }
        } else if format == DXT5 {
            for (p, a) in pixels.iter_mut().zip(dxt5_alpha(block)) {
                p[3] = a;
            }
        }
        for (i, p) in pixels.iter().enumerate() {
            let (x, y) = (bx + i % 4, by + i / 4);
            if x < w && y < h {
                let at = (y * w + x) * 4;
                if let Some(dst) = out.get_mut(at..at + 4) {
                    dst.copy_from_slice(p);
                }
            }
        }
    }
    out
}

type Swizzle = fn(&[u8]) -> [u8; 4];

fn decode_plain(data: &[u8], format: u32, w: usize, h: usize) -> Option<Vec<u8>> {
    let (bytes, order): (usize, Swizzle) = match format {
        RGBA8888 => (4, |p| [p[0], p[1], p[2], p[3]]),
        ABGR8888 => (4, |p| [p[3], p[2], p[1], p[0]]),
        ARGB8888 => (4, |p| [p[1], p[2], p[3], p[0]]),
        BGRA8888 => (4, |p| [p[2], p[1], p[0], p[3]]),
        BGRX8888 => (4, |p| [p[2], p[1], p[0], 255]),
        RGB888 => (3, |p| [p[0], p[1], p[2], 255]),
        BGR888 => (3, |p| [p[2], p[1], p[0], 255]),
        I8 => (1, |p| [p[0], p[0], p[0], 255]),
        IA88 => (2, |p| [p[0], p[0], p[0], p[1]]),
        _ => return None,
    };
    Some(data.chunks_exact(bytes).take(w * h).flat_map(order).collect())
}

/// Every `step`th pixel of every `step`th row.
fn shrink((w, h, rgba): Picture, step: usize) -> Picture {
    let (w, h) = (w as usize, h as usize);
    let (nw, nh) = ((w / step).max(1), (h / step).max(1));
    let mut out = Vec::with_capacity(nw * nh * 4);
    for y in 0..nh {
        for x in 0..nw {
            let at = (y * step * w + x * step) * 4;
            out.extend_from_slice(rgba.get(at..at + 4).unwrap_or(&[0; 4]));
        }
    }
    (nw as u32, nh as u32, out)
}

/// Where the picture's bytes are and what they hold.
struct Layout {
    at: usize,
    len: usize,
    w: usize,
    h: usize,
    format: u32,
}

/// Frame 0, face 0 of the largest mip no wider or taller than `max`, else of the smallest mip.
fn layout(vtf: &[u8], max: usize) -> Result<Layout, String> {
    if !vtf.starts_with(b"VTF\0") {
        return Err("not a VTF file".into());
    }
    let minor = u32_at(vtf, 8);
    let (width, height, flags) = (u16_at(vtf, 16), u16_at(vtf, 18), u32_at(vtf, 20));
    let (frames, first_frame) = (u16_at(vtf, 24).max(1), u16_at(vtf, 26));
    let (format, mips) = (u32_at(vtf, 52), u8_at(vtf, 56).clamp(1, 16));
    let depth = if minor >= 2 { u16_at(vtf, 63).max(1) } else { 1 };
    let faces = match (flags & ENVMAP != 0, minor < 5 && first_frame != 0xffff) {
        (false, _) => 1,
        (true, true) => 7,
        (true, false) => 6,
    };
    let start = if minor >= 3 {
        let count = (u32_at(vtf, 68) as usize).min(32);
        (0..count).map(|i| 80 + i * 8).find(|&at| vtf.get(at..at + 3) == Some(&[0x30, 0, 0])).map(|at| u32_at(vtf, at + 4) as usize)
    } else {
        let low = u32_at(vtf, 57);
        let low_size = if low == u32::MAX { Some(0) } else { image_size(low, u8_at(vtf, 61), u8_at(vtf, 62)) };
        low_size.and_then(|s| (u32_at(vtf, 12) as usize).checked_add(s))
    }
    .ok_or("the VTF has no image data")?;
    let mip = (0..mips).find(|&m| (width >> m).max(1) <= max && (height >> m).max(1) <= max).unwrap_or(mips - 1);
    let (w, h) = ((width >> mip).max(1), (height >> mip).max(1));
    if w.saturating_mul(h) > 4096 * 4096 {
        return Err(format!("{w}x{h} is too large"));
    }
    let size = |m: usize| image_size(format, (width >> m).max(1), (height >> m).max(1));
    let len = size(mip).ok_or_else(|| format!("VTF image format {format} is not supported"))?;
    let copies = frames.checked_mul(faces * depth).ok_or("bad VTF frame count")?;
    let skip = (mip + 1..mips).try_fold(0usize, |sum, m| size(m)?.checked_mul(copies)?.checked_add(sum)).ok_or("bad VTF mip sizes")?;
    let at = start.checked_add(skip).ok_or("bad VTF offset")?;
    Ok(Layout { at, len, w, h, format })
}

/// The full width and height a VTF header gives, whatever mip is decoded.
pub fn full_size(header: &[u8]) -> (u32, u32) {
    (u16_at(header, 16) as u32, u16_at(header, 18) as u32)
}

/// How many leading bytes of the file `decode` reads, worked out from its header.
pub fn needed(header: &[u8], max: usize) -> Option<usize> {
    layout(header, max).ok().map(|l| l.at.saturating_add(l.len))
}

/// Frame 0, face 0 of the largest mip no wider or taller than `max`, shrunk to fit when every mip is larger.
pub fn decode(vtf: &[u8], max: usize) -> Result<Picture, String> {
    let Layout { at, len, w, h, format } = layout(vtf, max)?;
    let data = vtf.get(at..at.saturating_add(len)).ok_or("the VTF is truncated")?;
    let rgba = match format {
        DXT1 | DXT1_ONEBITALPHA | DXT3 | DXT5 => decode_dxt(data, format, w, h),
        _ => decode_plain(data, format, w, h).ok_or_else(|| format!("VTF image format {format} is not supported"))?,
    };
    Ok(shrink((w as u32, h as u32, rgba), w.max(h).div_ceil(max.max(1)).next_power_of_two()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vtf(minor: u32, width: u16, height: u16, mips: u8, format: u32, data: &[u8]) -> Vec<u8> {
        let mut v = vec![0u8; 80];
        v[..4].copy_from_slice(b"VTF\0");
        v[4..8].copy_from_slice(&7u32.to_le_bytes());
        v[8..12].copy_from_slice(&minor.to_le_bytes());
        v[12..16].copy_from_slice(&80u32.to_le_bytes());
        v[16..18].copy_from_slice(&width.to_le_bytes());
        v[18..20].copy_from_slice(&height.to_le_bytes());
        v[24..26].copy_from_slice(&1u16.to_le_bytes());
        v[52..56].copy_from_slice(&format.to_le_bytes());
        v[56] = mips;
        v[57..61].copy_from_slice(&u32::MAX.to_le_bytes());
        v[63..65].copy_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(data);
        v
    }

    #[test]
    fn picks_the_largest_mip_that_fits() {
        // A 4x2 BGRA texture: the 2x1 mip, then the 4x2 one.
        let mut data = vec![1, 2, 3, 4, 5, 6, 7, 8];
        data.extend((0..32).map(|i| i as u8));
        let file = vtf(2, 4, 2, 2, BGRA8888, &data);
        assert_eq!(decode(&file, 2).unwrap(), (2, 1, vec![3, 2, 1, 4, 7, 6, 5, 8]));
        let (w, h, rgba) = decode(&file, 1024).unwrap();
        assert_eq!((w, h, &rgba[..4]), (4, 2, &[2, 1, 0, 3][..]));
        assert_eq!(decode(&file, 1).unwrap(), (1, 1, vec![3, 2, 1, 4]));
        assert_eq!((needed(&file[..80], 2), needed(&file[..80], 1024)), (Some(88), Some(120)));
        assert!(decode(&file[..90], 1024).is_err());
        assert!(decode(&vtf(2, 4, 4, 1, 4242, &[]), 1024).is_err());
    }

    #[test]
    fn decodes_dxt_blocks() {
        // Pure red and pure blue endpoints; the first row walks the four palette entries.
        let mut block = vec![0x00, 0xf8, 0x1f, 0x00];
        block.extend([0b1110_0100, 0, 0, 0]);
        let (_, _, rgba) = decode(&vtf(2, 4, 4, 1, DXT1, &block), 1024).unwrap();
        assert_eq!(&rgba[..16], &[255, 0, 0, 255, 0, 0, 255, 255, 170, 0, 85, 255, 85, 0, 170, 255]);
        let mut dxt5 = vec![255, 0, 0b0000_1000, 0, 0, 0, 0, 0];
        dxt5.extend(&block);
        let (_, _, rgba) = decode(&vtf(2, 4, 4, 1, DXT5, &dxt5), 1024).unwrap();
        assert_eq!((rgba[3], rgba[7]), (255, 0));
    }
}
