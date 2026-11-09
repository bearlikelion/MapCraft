//! Reads KTX2 pictures: the largest level of BC1, BC3, BC5 and BC7 textures, without supercompression.

use image::{Rgba, RgbaImage};

const MAGIC: &[u8] = b"\xABKTX 20\xBB\r\n\x1A\n";
const CUT: &str = "the file is cut short";

/// Decodes one block into 16 RGBA texels, four to a row.
type Block = fn(&[u8], &mut [u8]);

/// BC5 holds a normal's X and Y; Z is rebuilt so the picture reads as a normal map.
fn normal_block(block: &[u8], out: &mut [u8]) {
    let mut xy = [[0; 2]; 16];
    bcdec_rs::bc5(block, xy.as_flattened_mut(), 8, false);
    for (texel, [x, y]) in out.as_chunks_mut::<4>().0.iter_mut().zip(xy) {
        let [nx, ny] = [x, y].map(|v| f32::from(v) / 127.5 - 1.0);
        let z = (1.0 - nx * nx - ny * ny).max(0.0).sqrt();
        *texel = [x, y, (z * 127.5 + 127.5) as u8, 255];
    }
}

/// The first picture of a KTX2 file at full size.
pub fn decode(bytes: &[u8]) -> Result<RgbaImage, String> {
    if !bytes.starts_with(MAGIC) {
        return Err("not a KTX2 file".into());
    }
    let word = |at: usize| bytes.get(at..at + 4).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes).ok_or(CUT);
    let long = |at: usize| bytes.get(at..at + 8).and_then(|b| b.try_into().ok()).map(u64::from_le_bytes).ok_or(CUT);
    // A height of 0 marks a one-dimensional texture.
    let (format, width, height) = (word(12)?, word(20)?, word(24)?.max(1));
    if word(44)? != 0 {
        return Err("supercompressed KTX2 files are not supported".into());
    }
    // VkFormat numbers.
    let (size, block): (usize, Block) = match format {
        131..=134 => (8, |b, out| bcdec_rs::bc1(b, out, 16)),
        137 | 138 => (16, |b, out| bcdec_rs::bc3(b, out, 16)),
        141 => (16, normal_block),
        145 | 146 => (16, |b, out| bcdec_rs::bc7(b, out, 16)),
        other => return Err(format!("KTX2 pixel format {other} is not supported")),
    };
    // The level index starts with the largest level: its offset and length.
    let level = usize::try_from(long(80)?).ok().zip(usize::try_from(long(88)?).ok());
    let data = level.and_then(|(start, len)| bytes.get(start..start.checked_add(len)?)).ok_or(CUT)?;
    let across = width.div_ceil(4) as usize;
    // Checked against the data before the picture is allocated, so a header cannot ask for more than the file holds.
    let blocks = across.checked_mul(height.div_ceil(4) as usize).filter(|n| *n > 0 && data.len() / size >= *n).ok_or(CUT)?;
    let mut image = RgbaImage::new(width, height);
    let mut texels = [[0; 4]; 16];
    for (i, chunk) in data.chunks_exact(size).take(blocks).enumerate() {
        block(chunk, texels.as_flattened_mut());
        let (left, top) = ((i % across) as u32 * 4, (i / across) as u32 * 4);
        for (n, texel) in texels.iter().enumerate() {
            if let Some(pixel) = image.get_pixel_mut_checked(left + n as u32 % 4, top + n as u32 / 4) {
                *pixel = Rgba(*texel);
            }
        }
    }
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 6 by 5 BC7 picture of opaque white: mode 6 blocks with every endpoint bit set.
    fn white() -> Vec<u8> {
        let mut file = MAGIC.to_vec();
        for word in [146u32, 1, 6, 5, 0, 0, 1, 1, 0] {
            file.extend(word.to_le_bytes());
        }
        file.resize(80, 0);
        for long in [104u64, 64, 64] {
            file.extend(long.to_le_bytes());
        }
        for _ in 0..4 {
            file.extend([0xC0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01, 0, 0, 0, 0, 0, 0, 0]);
        }
        file
    }

    #[test]
    fn decodes_bc7_and_crops_the_last_blocks() {
        let image = decode(&white()).unwrap();
        assert_eq!(image.dimensions(), (6, 5));
        assert!(image.pixels().all(|p| p.0 == [255; 4]));
    }

    #[test]
    fn cut_or_lying_files_are_errors() {
        let file = white();
        for n in 0..file.len() {
            assert!(decode(&file[..n]).is_err(), "cut at {n}");
        }
        let mut huge = file.clone();
        huge[20..28].fill(0xFF);
        assert!(decode(&huge).is_err());
        let mut far = file;
        far[80..88].fill(0xFF);
        assert!(decode(&far).is_err());
    }
}
