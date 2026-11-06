//! Imports of real maps, when they are on this machine, and the brush and entity readers.

use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::Instant;

use glam::{DMat3, DVec3};
use valve::Files;

use crate::bsp::{HEADER, NODRAW, Texinfo, material_name};
use crate::import::{Import, import};
use crate::key_values::{key, parse_entities, rotation, vector};
use crate::mapcraft::plugin::types::{Brush, Entity, Face, Group, Vec3};
use crate::world::{Map, OPEN, width, windings};

const SURF: &str = "/mnt/Storage/Gamedev/SurfsUp/SurfMaps/BSPs/surf_utopia_njv.bsp";
const LZMA: &str = "/mnt/Storage/Gamedev/SurfsUp/SurfMaps/BSPs/surf_fiellu_ksf.bsp";
const EMERALD: &str = "/mnt/Storage/Gamedev/SurfsUp/SurfMaps/BSPs/surf_emerald.bsp";
const LARGEST: &str = "/mnt/Storage/Gamedev/SurfsUp/BhopMaps/BSPs/kz_bhop_badg3s.bsp";
const CSS: &str = "/mnt/Storage/Gamedev/SurfsUp/CSS-Windows";

/// Game files under the given folders, like the SDK's `cstrike` and `hl2` roots.
struct Disk(Vec<PathBuf>);

impl Files for Disk {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.0.iter().find_map(|root| std::fs::read(root.join(path)).ok())
    }

    fn read_range(&self, path: &str, offset: u64, length: u64) -> Option<Vec<u8>> {
        self.0.iter().find_map(|root| {
            let mut file = std::fs::File::open(root.join(path)).ok()?;
            file.seek(SeekFrom::Start(offset)).ok()?;
            let mut out = vec![0; length as usize];
            file.read_exact(&mut out).ok()?;
            Some(out)
        })
    }
}

const NO_FILES: Disk = Disk(Vec::new());

fn read(path: &str) -> Option<Vec<u8>> {
    std::fs::read(path).map_err(|_| eprintln!("skipped: {path} is missing")).ok()
}

fn dv(v: &Vec3) -> DVec3 {
    DVec3::new(v.x, v.y, v.z)
}

fn group<'a>(import: &'a Import, name: &str) -> Option<&'a Group> {
    import.output.groups.iter().find(|g| g.name == name)
}

fn world(import: &Import) -> Vec<&Brush> {
    ["World", "Skybox3D", "BrushCollision", "LadderCollision"].iter().filter_map(|n| group(import, n)).flat_map(|g| &g.brushes).collect()
}

fn entities(import: &Import) -> Vec<&Entity> {
    import.output.groups.iter().flat_map(|g| &g.entities).collect()
}

fn planes_of(brush: &Brush) -> Vec<(DVec3, f64)> {
    let Brush::Faces(faces) = brush else { panic!("hull brush") };
    faces
        .iter()
        .map(|f| {
            let (p0, p1, p2) = (dv(&f.points.0), dv(&f.points.1), dv(&f.points.2));
            let n = (p0 - p1).cross(p2 - p1).normalize();
            (n, n.dot(p1))
        })
        .collect()
}

/// At least four faces survive clipping by the others, they close up, and every normal points away from the middle.
fn assert_closed(brush: &Brush) {
    let planes = planes_of(brush);
    let ws: Vec<Vec<DVec3>> = windings(&planes).into_iter().filter(|w| !w.is_empty()).collect();
    assert!(ws.len() >= 4 && ws.iter().flatten().all(|p| p.abs().max_element() < OPEN), "open brush");
    let area: DVec3 = ws.iter().flat_map(|w| w.iter().zip(w.iter().cycle().skip(1)).map(|(a, b)| a.cross(*b))).sum();
    let total: f64 = ws.iter().map(|w| width(w) * w.iter().zip(w.iter().cycle().skip(1)).map(|(a, b)| a.distance(*b)).sum::<f64>()).sum();
    assert!(area.length() <= total * 1e-6, "open brush: {area} over {total}");
    let points: Vec<DVec3> = ws.into_iter().flatten().collect();
    let middle = points.iter().sum::<DVec3>() / points.len() as f64;
    assert!(planes.iter().all(|(n, d)| n.dot(middle) < *d), "a face points inward");
}

fn texinfo(material: &str) -> Texinfo {
    Texinfo { material: material.into(), s: (DVec3::X * 0.25, 8.0), t: (DVec3::NEG_Z * 0.5, -3.0) }
}

#[test]
fn cube_drops_a_plane_touching_an_edge() {
    let mut planes: Vec<(DVec3, f64)> = [DVec3::X, DVec3::NEG_X, DVec3::Y, DVec3::NEG_Y, DVec3::Z, DVec3::NEG_Z].map(|n| (n, 64.0)).to_vec();
    planes.push((DVec3::new(1.0, 1.0, 0.0).normalize(), 128.0 / 2f64.sqrt()));
    let ws = windings(&planes);
    assert_eq!(ws.iter().filter(|w| !w.is_empty()).count(), 6);
    assert!(ws[6].is_empty());
    let map = Map { planes, texinfos: vec![texinfo("dev/a")], brushes: vec![(0, 7)], sides: (0..7).map(|p| (p, 0, false)).collect() };
    let (rot, origin) = (rotation(DVec3::new(0.0, 90.0, 0.0)), DVec3::new(100.0, 0.0, 0.0));
    let (brush, surf, _) = map.brush(0, rot, origin).unwrap();
    assert!(!surf);
    assert_closed(&brush);
    let Brush::Faces(faces) = &brush else { panic!() };
    assert_eq!(faces.len(), 6);
    assert!(faces.iter().all(|f| f.material == "dev/a" && (dv(&f.points.1) - origin).abs().max_element() <= 64.0 + 1e-9));
    // A point keeps its texel coordinates once the brush is rotated and moved.
    let local = DVec3::new(64.0, 10.0, 20.0);
    let (u, v) = faces[0].uv.as_ref().unwrap();
    let placed = rot * local + origin;
    assert!((placed.dot(dv(&u.axis)) / u.scale + u.offset - (local.dot(DVec3::X * 0.25) + 8.0)).abs() < 1e-9);
    assert!((placed.dot(dv(&v.axis)) / v.scale + v.offset - (local.dot(DVec3::NEG_Z * 0.5) - 3.0)).abs() < 1e-9);
}

#[test]
fn steep_slopes_are_ramps() {
    let ramp = |degrees: f64, size: f64| {
        let slope = DVec3::new(degrees.to_radians().sin(), 0.0, degrees.to_radians().cos());
        let planes = vec![(DVec3::NEG_X, size), (DVec3::Y, size), (DVec3::NEG_Y, size), (DVec3::NEG_Z, size), (slope, 0.0)];
        let map = Map { planes, texinfos: vec![texinfo("dev/a")], brushes: vec![(0, 5)], sides: (0..5).map(|p| (p, 0, false)).collect() };
        map.brush(0, DMat3::IDENTITY, DVec3::ZERO).unwrap().1
    };
    assert!(ramp(60.0, 512.0));
    assert!(!ramp(40.0, 512.0), "walkable");
    assert!(!ramp(88.0, 512.0), "a wall");
    assert!(!ramp(60.0, 16.0), "trim");
}

#[test]
fn bevel_closes_an_open_brush() {
    let planes = [DVec3::X, DVec3::NEG_X, DVec3::NEG_Y, DVec3::Z, DVec3::NEG_Z, DVec3::Y].map(|n| (n, 64.0)).to_vec();
    let map = Map { planes, texinfos: Vec::new(), brushes: vec![(0, 6)], sides: (0..6).map(|p| (p, -1, p == 5)).collect() };
    let (brush, ..) = map.brush(0, DMat3::IDENTITY, DVec3::ZERO).unwrap();
    assert_closed(&brush);
    let Brush::Faces(faces) = &brush else { panic!() };
    assert!(faces.len() == 6 && faces.iter().all(|f| f.material == NODRAW && f.uv.is_none()));
}

#[test]
fn rotation_matches_source_angles() {
    let forward = |p, y| rotation(DVec3::new(p, y, 0.0)) * DVec3::X;
    assert!(forward(0.0, 90.0).abs_diff_eq(DVec3::Y, 1e-12));
    assert!(forward(90.0, 0.0).abs_diff_eq(DVec3::NEG_Z, 1e-12));
    assert!((rotation(DVec3::new(0.0, 0.0, 90.0)) * DVec3::Y).abs_diff_eq(DVec3::Z, 1e-12));
}

#[test]
fn parses_entity_text() {
    let text = "{\n\"classname\" \"worldspawn\"\n}\n{\n\"classname\" \"logic_relay\"\n\"OnTrigger\" \"a,Kill,,0,-1\"\n\"OnTrigger\" \"b,Use,}{,0,-1\"\n}\n{ \"classname\" \"x\" \"dangling\" }\n{ \"classname\" \"cut";
    let ents = parse_entities(text);
    assert_eq!(ents.len(), 4);
    assert_eq!(ents[1].len(), 3);
    assert_eq!(ents[1][2], ("OnTrigger".to_string(), "b,Use,}{,0,-1".to_string()));
    assert_eq!(key(&ents[2], "CLASSNAME"), Some("x"));
    assert_eq!(key(&ents[3], "classname"), Some("cut"));
}

#[test]
fn unpatches_material_names() {
    assert_eq!(material_name("MATERIALS\\Maps\\surf_x\\Glass\\GlassWindow001a_-14096_0_-6152"), "glass/glasswindow001a");
    assert_eq!(material_name("maps/surf_x/nature/blendrockdirt001a_wvt_patch"), "nature/blendrockdirt001a");
    assert_eq!(material_name("maps/surf_x/custom/ramp_1_2"), "maps/surf_x/custom/ramp_1_2");
    assert_eq!(material_name("TOOLS/TOOLSPLAYERCLIP"), "tools/toolsplayerclip");
}

#[test]
fn rejects_bad_files_without_panicking() {
    assert!(import(b"", &NO_FILES).is_err());
    assert!(import(b"IBSP\x26\0\0\0", &NO_FILES).is_err());
    let mut file = vec![0u8; HEADER];
    file[..4].copy_from_slice(b"VBSP");
    file[4] = 25;
    assert!(import(&file, &NO_FILES).err().unwrap_or_default().contains("version 25"));
    file[4] = 20;
    file[8..12].copy_from_slice(&5000i32.to_le_bytes());
    file[12..16].copy_from_slice(&10i32.to_le_bytes());
    assert!(import(&file, &NO_FILES).err().unwrap_or_default().contains("outside"));
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    for round in 0..300 {
        let mut file: Vec<u8> = (0..HEADER + 4096)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                seed as u8
            })
            .collect();
        file[..8].copy_from_slice(b"VBSP\x14\0\0\0");
        for lump in 0..64 {
            let at = 8 + lump * 16;
            let offset = (HEADER + (lump * 61 + round) % 4096) as i32;
            file[at..at + 4].copy_from_slice(&offset.to_le_bytes());
            file[at + 4..at + 8].copy_from_slice(&((lump as i32 * 37 + round as i32) % 600).to_le_bytes());
            if round % 3 == 0 && lump % 5 == 0 {
                let o = offset as usize;
                file[o..o + 4].copy_from_slice(b"LZMA");
                file[at + 12..at + 16].copy_from_slice(&1000i32.to_le_bytes());
            }
        }
        let _ = import(&file, &NO_FILES);
    }
}

#[test]
fn imports_a_surf_map() {
    let Some(data) = read(SURF) else { return };
    let map = import(&data, &NO_FILES).unwrap();
    let names: Vec<(&str, bool)> = map.output.groups.iter().map(|g| (g.name.as_str(), g.hidden)).collect();
    assert_eq!(names, [("World", false), ("BrushCollision", true), ("SourceEntities", false), ("SourceEnvironment", false)]);
    assert_eq!(map.output.entities.len(), 1, "one spawn point");
    assert_eq!(map.output.entities[0].classname, "info_player_start");
    let (brushes, ents) = (world(&map), entities(&map));
    assert!(brushes.len() == map.world_brushes && brushes.len() > 3000, "{} world brushes", brushes.len());
    assert!(map.ramps > 100 && map.ramps < brushes.len() / 2, "{} ramps", map.ramps);
    assert_eq!((map.entities, ents.len(), map.brush_entities, map.dropped_brushes), (153, 105, 99, 0));
    assert!(ents.iter().all(|e| !e.classname.starts_with("info_player_")));
    brushes.iter().copied().for_each(assert_closed);
    let teleports: Vec<&Entity> = group(&map, "SourceEntities").unwrap().entities.iter().filter(|e| e.classname == "trigger_teleport").collect();
    assert_eq!(teleports.len(), 76);
    assert!(teleports.iter().all(|e| !e.brushes.is_empty() && key(&e.properties, "model").is_none()));
    teleports.iter().flat_map(|e| &e.brushes).for_each(assert_closed);
    let faces: Vec<&Face> = brushes
        .iter()
        .flat_map(|b| match b {
            Brush::Faces(f) => f.as_slice(),
            Brush::Hull(_) => &[],
        })
        .collect();
    assert!(faces.iter().any(|f| f.material == "tools/toolsplayerclip"));
    assert!(faces.iter().all(|f| !f.material.starts_with("maps/") && f.uv.is_some()));
    assert!(faces.iter().all(|f| map.output.materials.iter().any(|m| m.name == f.material && m.color.is_some())));
    let illusionary = ents.iter().find(|e| e.classname == "func_illusionary").unwrap();
    let origin = vector(key(&illusionary.properties, "origin"));
    let points: Vec<DVec3> = illusionary.brushes.iter().flat_map(|b| windings(&planes_of(b))).flatten().collect();
    let middle = points.iter().fold(DVec3::MAX, |a, p| a.min(*p)).midpoint(points.iter().fold(DVec3::MIN, |a, p| a.max(*p)));
    assert!(middle.distance(origin) < 1.0, "brush entity at {middle}, origin {origin}");
}

#[test]
fn imports_lzma_lumps_and_static_props() {
    let Some(data) = read(LZMA) else { return };
    let map = import(&data, &NO_FILES).unwrap();
    assert!(map.warnings.is_empty(), "{:?}", map.warnings);
    assert!(map.world_brushes > 0 && map.static_props > 0);
    world(&map).into_iter().for_each(assert_closed);
    let prop = group(&map, "Props").unwrap().entities.iter().find(|e| e.classname == "prop_static").unwrap();
    assert!(key(&prop.properties, "model").is_some_and(|m| m.starts_with("models/") && m.ends_with(".mdl")));
}

#[test]
fn finds_textures_in_the_map_and_the_game() {
    let game = Disk(vec![PathBuf::from(CSS).join("cstrike"), PathBuf::from(CSS).join("hl2")]);
    if let Some(data) = read(EMERALD) {
        let map = import(&data, &NO_FILES).unwrap();
        assert_eq!(map.textures, (19, 0, 0), "surf_emerald embeds all its textures");
        for m in &map.output.materials {
            let p = m.pixels.as_ref().unwrap();
            assert!(p.width <= 1024 && p.height <= 1024 && p.width > 1 && p.rgba.len() == (p.width * p.height * 4) as usize, "{}", m.name);
        }
    }
    let Some(data) = read(SURF) else { return };
    let alone = import(&data, &NO_FILES).unwrap();
    let start = Instant::now();
    let full = import(&data, &game).unwrap();
    eprintln!("{SURF}: alone {:?}, with the game {:?} in {:?}", alone.textures, full.textures, start.elapsed());
    assert_eq!(alone.textures.1, 0);
    if full.game_files {
        assert!(full.textures.1 > 0 && full.textures.2 < alone.textures.2);
    }
}

#[test]
fn times_the_largest_map() {
    let Some(data) = read(LARGEST) else { return };
    let start = Instant::now();
    let map = import(&data, &NO_FILES).unwrap();
    eprintln!("{LARGEST}: {} MB in {:?}, {} world brushes, {} entities", data.len() / 1_000_000, start.elapsed(), map.world_brushes, map.entities);
    world(&map).into_iter().for_each(assert_closed);
}
