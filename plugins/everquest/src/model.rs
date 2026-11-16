//! Static object models, named `<archive>.s3d/<actor>.actordef`, read from the archive's actor definitions.

use std::collections::BTreeMap;

use crate::image::decode;
use crate::mapcraft::plugin::types::{Image, Model, ModelPart, Vec3};
use crate::pfs::Archive;
use crate::wld::{ACTOR_DEF, Wld};

/// The archive and actor a model path names.
pub fn split(path: &str) -> Option<(&str, &str)> {
    let (archive, actor) = path.rsplit_once('/')?;
    let actor = actor.strip_suffix(".actordef")?;
    archive.to_ascii_lowercase().ends_with(".s3d").then_some((archive, actor))
}

/// The actor's mesh from `data`, the archive's bytes, with each material's picture.
pub fn load(archive_name: &str, data: &[u8], actor: &str) -> Result<Model, String> {
    let archive = Archive::open(data)?;
    let stem = archive_name.rsplit('/').next().unwrap_or(archive_name);
    let stem = stem.rsplit_once('.').map_or(stem, |(s, _)| s);
    let wld_data = archive.read(&format!("{stem}.wld"))?;
    let wld = Wld::parse(&wld_data)?;
    let def = format!("{actor}_actordef");
    let f = wld.of_kind(ACTOR_DEF).find(|f| f.name.eq_ignore_ascii_case(&def)).ok_or_else(|| format!("{archive_name} has no object {actor}"))?;
    let mesh = wld.actor_mesh(f).ok_or_else(|| format!("{actor} is not a static object"))?;
    let list = wld.material_list(mesh.materials);
    let mut textures = Vec::new();
    let mut by_bitmap: BTreeMap<String, Option<u32>> = BTreeMap::new();
    let mut parts: BTreeMap<Option<u32>, ModelPart> = BTreeMap::new();
    for t in &mesh.triangles {
        let material = list.get(t.material).and_then(Option::as_ref);
        if material.is_none_or(|m| m.invisible()) {
            continue;
        }
        let texture = material.and_then(|m| {
            let bitmap = m.bitmaps.first()?;
            *by_bitmap.entry(bitmap.clone()).or_insert_with(|| {
                let p = archive.read(bitmap).and_then(|d| decode(&d, m.masked())).ok()?;
                textures.push(Image { width: p.width, height: p.height, rgba: p.rgba });
                u32::try_from(textures.len() - 1).ok()
            })
        });
        let part = parts.entry(texture).or_insert_with(|| ModelPart {
            texture,
            positions: Vec::new(),
            uvs: Vec::new(),
            indices: Vec::new(),
            material: None,
            lightmap: None,
            lightmap_uvs: Vec::new(),
        });
        // Clockwise in EverQuest, counter-clockwise here.
        for &i in &[t.corners[0], t.corners[2], t.corners[1]] {
            let Some(&[x, y, z]) = mesh.positions.get(i) else { continue };
            let [u, v] = mesh.uvs.get(i).copied().unwrap_or_default();
            let Ok(at) = u32::try_from(part.positions.len()) else { continue };
            part.positions.push(Vec3 { x: x.into(), y: y.into(), z: z.into() });
            part.uvs.push((u, v));
            part.indices.push(at);
        }
    }
    Ok(Model { parts: parts.into_values().filter(|p| p.indices.len() % 3 == 0 && !p.indices.is_empty()).collect(), textures })
}
