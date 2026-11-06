//! The materials a map uses, with their pictures from the map or the game, as in SourceGD's `material`.

use std::collections::BTreeSet;

use glam::DVec3;
use valve::{Library, Origin};

use crate::bsp::Texdata;
use crate::mapcraft::plugin::types::{BlendRole, Image, MaterialInfo};

/// RGBA bytes a map's pictures may take at most; maps with many materials get smaller mips.
const PICTURE_BUDGET: usize = 128 << 20;

fn srgb(linear: DVec3) -> (u8, u8, u8) {
    let [r, g, b] = linear.to_array().map(|c| (c.clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0).round() as u8);
    (r, g, b)
}

/// Each used material with its picture, and how many came from the map, from the game's files, or from neither.
pub(crate) fn materials(used: BTreeSet<&str>, texdata: &[Texdata], library: &Library) -> (Vec<MaterialInfo>, (usize, usize, usize)) {
    let side = [1024, 512, 256, 128].into_iter().find(|s| used.len().saturating_mul(s * s * 4) <= PICTURE_BUDGET).unwrap_or(64);
    let mut textures = (0, 0, 0);
    let list = used
        .into_iter()
        .map(|name| {
            // A map may embed only VBSP's patched copy of a material, so the copies are the fallback.
            let copies = texdata.iter().filter(|t| t.0 == name && t.1 != name);
            let picture = library.picture(name, side).or_else(|| copies.into_iter().find_map(|t| library.picture(&t.1, side)));
            match picture.as_ref().map(|p| p.1) {
                Some(Origin::Map) => textures.0 += 1,
                Some(Origin::Game) => textures.1 += 1,
                None => textures.2 += 1,
            }
            MaterialInfo {
                name: name.to_string(),
                folder: None,
                image: None,
                color: texdata.iter().find(|t| t.0 == name).map(|t| srgb(t.2)),
                size: picture.as_ref().map(|p| p.2),
                pixels: picture.map(|((width, height, rgba), ..)| Image { width, height, rgba }),
                pbr: None,
                uv_scale: None,
                pixelated: false,
                blend: BlendRole::Layer,
            }
        })
        .collect();
    (list, textures)
}
