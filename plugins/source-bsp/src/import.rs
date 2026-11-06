//! A BSP read into SourceGD's layout: World, Skybox3D, BrushCollision and LadderCollision, SourceEntities, Props and SourceEnvironment, with one spawn point.

use std::borrow::Cow;
use std::collections::BTreeSet;

use glam::{DMat3, DVec3};
use valve::{Files, Library, u16_at};

use crate::bsp::{BRUSHES, BRUSHSIDES, Bsp, DISPINFO, ENTITIES, PAKFILE, PLANES, Tree, texinfos};
use crate::byte_reader::{c_str, f32_at, i16_at, i32_at, index, vec_at};
use crate::entity::{Section, is_spawn, section, spawn_of, spawn_point};
use crate::key_values::{key, parse_entities, rotation, vector};
use crate::mapcraft::plugin::types::{Brush, Entity, Group, Output};
use crate::material::materials;
use crate::model::static_props;
use crate::world::{Bounds, CONTENTS_LADDER, Map, owners, skybox_bounds, within};

/// Groups in SourceGD's order, and whether each starts hidden like its collision bodies.
const GROUPS: [(&str, bool); 7] = [
    ("World", false),
    ("Skybox3D", false),
    ("BrushCollision", true),
    ("LadderCollision", true),
    ("SourceEntities", false),
    ("Props", false),
    ("SourceEnvironment", false),
];

/// What an import produced, with counts for the log.
pub struct Import {
    pub output: Output,
    pub world_brushes: usize,
    pub ramps: usize,
    pub entities: usize,
    pub brush_entities: usize,
    pub static_props: usize,
    pub displacements: usize,
    pub dropped_brushes: usize,
    /// Materials whose texture came from the map's pakfile, from the game's files, or from neither.
    pub textures: (usize, usize, usize),
    pub game_files: bool,
    pub warnings: Vec<String>,
}

fn is_tool(brush: &Brush) -> bool {
    let Brush::Faces(faces) = brush else { return false };
    faces.iter().all(|f| f.material.get(..6).is_some_and(|p| p.eq_ignore_ascii_case("tools/")))
}

pub(crate) fn import(file: &[u8], files: &dyn Files) -> Result<Import, String> {
    let bsp = Bsp::open(file)?;
    let (texinfos, texdata) = texinfos(&bsp)?;
    let raw_brushes = bsp.lump(BRUSHES)?;
    let raw_brushes = raw_brushes.as_chunks::<12>().0;
    let map = Map {
        planes: bsp.lump(PLANES)?.as_chunks::<20>().0.iter().map(|r| (vec_at(r, 0), f32_at(r, 12))).collect(),
        texinfos,
        brushes: raw_brushes.iter().map(|r| (index(i32_at(r, 0)), index(i32_at(r, 4)))).collect(),
        sides: bsp.lump(BRUSHSIDES)?.as_chunks::<8>().0.iter().map(|r| (usize::from(u16_at(r, 0)), i16_at(r, 2), r.get(6).is_some_and(|&b| b != 0))).collect(),
    };
    let contents: Vec<i32> = raw_brushes.iter().map(|r| i32_at(r, 8)).collect();
    let tree = Tree::read(&bsp)?;
    let owner = owners(&tree, map.brushes.len());
    let models = tree.heads.len();
    let mut model_brushes = vec![Vec::new(); models];
    for (brush, model) in owner.iter().enumerate() {
        if let Some(list) = model.and_then(|m| model_brushes.get_mut(m)) {
            list.push(brush);
        }
    }

    let text = bsp.lump(ENTITIES)?;
    let parsed = parse_entities(&c_str(&text));
    let camera = parsed.iter().find(|k| key(k, "classname").is_some_and(|c| c.eq_ignore_ascii_case("sky_camera"))).map(|k| vector(key(k, "origin")));
    let skybox = camera.and_then(|c| skybox_bounds(&bsp, &tree, &map.planes, c));
    let in_skybox = |b: Bounds| skybox.is_some_and(|s| within(s, b));

    let mut dropped = 0;
    let mut build = |model: usize, rotation: DMat3, origin: DVec3| -> Vec<(usize, Brush, bool, Bounds)> {
        let list = model_brushes.get(model).map_or(&[][..], Vec::as_slice);
        let built: Vec<_> = list.iter().filter_map(|&b| map.brush(b, rotation, origin).map(|(brush, surf, bounds)| (b, brush, surf, bounds))).collect();
        dropped += list.len() - built.len();
        built
    };
    let world = build(0, DMat3::IDENTITY, DVec3::ZERO);

    let mut entities: Vec<(Entity, Option<Bounds>)> = Vec::new();
    let mut brush_entities = 0;
    for mut keys in parsed {
        let Some(classname) = key(&keys, "classname").map(str::to_string) else { continue };
        if classname.eq_ignore_ascii_case("worldspawn") {
            continue;
        }
        let origin = vector(key(&keys, "origin"));
        let model = key(&keys, "model").and_then(|m| m.strip_prefix('*')).and_then(|m| m.parse::<usize>().ok()).filter(|&m| m > 0 && m < models);
        let mut entity_brushes = Vec::new();
        let mut bounds = Some((origin, origin));
        if let Some(model) = model {
            let built = build(model, rotation(vector(key(&keys, "angles"))), origin);
            bounds = built.iter().map(|b| b.3).reduce(|(lo, hi), (a, b)| (lo.min(a), hi.max(b)));
            entity_brushes = built.into_iter().map(|b| b.1).collect();
            keys.retain(|(k, _)| !k.eq_ignore_ascii_case("model"));
            brush_entities += 1;
        }
        keys.retain(|(k, _)| !k.eq_ignore_ascii_case("classname"));
        entities.push((Entity { classname, properties: keys, connections: Vec::new(), brushes: entity_brushes }, bounds));
    }
    let mut warnings = Vec::new();
    let props = static_props(&bsp).unwrap_or_else(|e| {
        warnings.push(e);
        Vec::new()
    });
    let static_props = props.len();
    entities.extend(props.into_iter().map(|p| {
        let at = vector(key(&p.properties, "origin"));
        (p, Some((at, at)))
    }));
    let entity_count = entities.len();
    let spawn = spawn_of(&entities.iter().map(|e| e.0.clone()).collect::<Vec<_>>()).map(spawn_point);
    entities.retain(|e| !is_spawn(&e.0.classname));

    let used: BTreeSet<&str> = world
        .iter()
        .map(|b| &b.1)
        .chain(entities.iter().flat_map(|e| &e.0.brushes))
        .flat_map(|b| match b {
            Brush::Faces(faces) => faces.as_slice(),
            Brush::Hull(_) => &[],
        })
        .map(|f| f.material.as_str())
        .collect();
    let pak = bsp.lump(PAKFILE).unwrap_or_else(|e| {
        warnings.push(format!("embedded files: {e}"));
        Cow::Borrowed(&[])
    });
    let library = Library::new(&pak, files);
    let (materials, textures) = materials(used, &texdata, &library);

    let (world_brushes, ramps) = (world.len(), world.iter().filter(|b| b.2).count());
    let mut groups: Vec<Group> = GROUPS.iter().map(|&(name, hidden)| Group { name: name.into(), hidden, brushes: Vec::new(), entities: Vec::new() }).collect();
    let at = |name: &str| GROUPS.iter().position(|g| g.0 == name).unwrap_or(0);
    for (index, brush, _, bounds) in world {
        let name = match () {
            _ if in_skybox(bounds) => "Skybox3D",
            _ if contents.get(index).is_some_and(|c| c & CONTENTS_LADDER != 0) => "LadderCollision",
            _ if is_tool(&brush) => "BrushCollision",
            _ => "World",
        };
        if let Some(g) = groups.get_mut(at(name)) {
            g.brushes.push(brush);
        }
    }
    for (entity, bounds) in entities {
        let name = match section(&entity.classname) {
            _ if bounds.is_some_and(in_skybox) => "Skybox3D",
            Section::Props => "Props",
            Section::Environment => "SourceEnvironment",
            Section::Entities => "SourceEntities",
        };
        if let Some(g) = groups.get_mut(at(name)) {
            g.entities.push(entity);
        }
    }
    groups.retain(|g| !g.brushes.is_empty() || !g.entities.is_empty());
    Ok(Import {
        output: Output { brushes: Vec::new(), entities: spawn.into_iter().collect(), groups, materials },
        world_brushes,
        ramps,
        entities: entity_count,
        brush_entities,
        static_props,
        displacements: bsp.lump_size(DISPINFO) / 176,
        dropped_brushes: dropped,
        textures,
        game_files: library.has_game_files(),
        warnings,
    })
}
