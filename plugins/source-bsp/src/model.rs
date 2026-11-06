//! Static props from the game lump, as in SourceGD's `model`.

use std::borrow::Cow;

use glam::DVec3;
use valve::{u16_at, u32_at};

use crate::bsp::{Bsp, GAME_LUMP, STATIC_PROPS};
use crate::byte_reader::{c_str, i32_at, index, unlzma, vec_at};
use crate::key_values::number;
use crate::mapcraft::plugin::types::Entity;

/// `prop_static` entities from the `sprp` game lump; every version keeps the fields read here at the same offsets.
pub(crate) fn static_props(bsp: &Bsp) -> Result<Vec<Entity>, String> {
    let dir = bsp.lump(GAME_LUMP)?;
    let count = index(i32_at(&dir, 0));
    let Some(entry) = dir.get(4..).unwrap_or_default().as_chunks::<16>().0.iter().take(count).find(|e| u32_at(*e, 0) == STATIC_PROPS) else {
        return Ok(Vec::new());
    };
    let (flags, offset, length) = (u16_at(entry, 4), index(i32_at(entry, 8)), index(i32_at(entry, 12)));
    let raw = bsp.file.get(offset..).unwrap_or_default();
    let body: Cow<[u8]> = if flags & 1 != 0 && raw.starts_with(b"LZMA") {
        Cow::Owned(unlzma(raw).map_err(|e| format!("static props: {e}"))?)
    } else {
        Cow::Borrowed(raw.get(..length).ok_or("the static prop lump lies outside the file")?)
    };
    let names = index(i32_at(&body, 0));
    let dict: Vec<String> =
        body.get(4..).unwrap_or_default().as_chunks::<128>().0.iter().take(names).map(|n| c_str(n).to_ascii_lowercase().replace('\\', "/")).collect();
    let mut at = 4 + dict.len() * 128;
    at = at.saturating_add(4).saturating_add(index(i32_at(&body, at)).saturating_mul(2));
    let props = index(i32_at(&body, at));
    if props == 0 {
        return Ok(Vec::new());
    }
    let rest = body.get(at.saturating_add(4)..).unwrap_or_default();
    let stride = rest.len().checked_div(props).unwrap_or(0);
    if dict.len() != names || stride < 56 {
        return Err("the static prop lump is malformed".into());
    }
    let format = |v: DVec3| format!("{} {} {}", number(v.x), number(v.y), number(v.z));
    Ok(rest
        .chunks_exact(stride)
        .take(props)
        .map(|r| Entity {
            classname: "prop_static".into(),
            properties: vec![
                ("model".into(), dict.get(usize::from(u16_at(r, 24))).cloned().unwrap_or_default()),
                ("origin".into(), format(vec_at(r, 0))),
                ("angles".into(), format(vec_at(r, 12))),
                ("skin".into(), i32_at(r, 32).to_string()),
            ],
            connections: Vec::new(),
            brushes: Vec::new(),
        })
        .collect())
}
