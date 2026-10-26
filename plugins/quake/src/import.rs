//! Turns a `.map` into the plugin's output: world brushes, entities, TrenchBroom layers and groups as named groups, and the textures it uses.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::map::{self, MapEntity};
use crate::mapcraft::plugin::types::{Brush, Entity, Face, Group, Image, MaterialInfo, Output};
use crate::wad::{Files, Library, Picture};

/// Deeper nesting than this is a broken file, not a mapper's work.
const MAX_DEPTH: usize = 64;

pub struct Import {
    pub output: Output,
    pub log: Vec<String>,
}

pub fn material(name: String, picture: Picture) -> MaterialInfo {
    let color = Some(picture.average());
    MaterialInfo {
        name,
        image: None,
        color,
        size: Some(picture.full),
        pixels: Some(Image { width: picture.width, height: picture.height, rgba: picture.rgba }),
        pbr: None,
    }
}

fn brushes(entity: &mut MapEntity) -> Vec<Brush> {
    std::mem::take(&mut entity.brushes).into_iter().filter(|faces| !faces.is_empty()).map(Brush::Faces).collect()
}

fn faces(brush: &Brush) -> &[Face] {
    match brush {
        Brush::Faces(f) => f,
        Brush::Hull(_) => &[],
    }
}

/// A TrenchBroom layer or group: a `func_group` with `_tb_type`.
struct Container {
    layer: bool,
    id: String,
    name: String,
    /// The `_tb_group` or `_tb_layer` it sits in, as (is a layer, id).
    parent: Option<(bool, String)>,
    group: Group,
}

/// Where TrenchBroom put an entity or group: its `_tb_group`, else its `_tb_layer`.
fn parent_of(e: &MapEntity) -> Option<(bool, String)> {
    e.key("_tb_group").map(|g| (false, g.to_string())).or_else(|| e.key("_tb_layer").map(|l| (true, l.to_string())))
}

/// "Layer / Group / Subgroup": the container's name after those of the layer and groups around it.
fn path(containers: &[Container], index: &HashMap<(bool, &str), usize>, at: usize) -> String {
    let (mut names, mut seen, mut next) = (Vec::new(), HashSet::new(), Some(at));
    while let Some(c) = next.filter(|&i| names.len() < MAX_DEPTH && seen.insert(i)).and_then(|i| containers.get(i)) {
        names.push(c.name.as_str());
        next = c.parent.as_ref().and_then(|(layer, id)| index.get(&(*layer, id.as_str())).copied());
    }
    names.reverse();
    names.join(" / ")
}

pub fn import(text: &str, files: &dyn Files) -> Result<Import, String> {
    let parsed = map::parse(text)?;
    let mut log = Vec::new();
    let mut world = Vec::new();
    let mut world_keys: Vec<(String, String)> = Vec::new();
    let mut wad_key = String::new();
    let mut containers: Vec<Container> = Vec::new();
    let mut entities: Vec<(Option<(bool, String)>, Entity)> = Vec::new();
    let mut unnamed = 0;
    for mut e in parsed.entities {
        let classname = e.key("classname").unwrap_or_default().to_string();
        let tb_type = e.key("_tb_type").filter(|_| classname == "func_group").map(str::to_string);
        if classname == "worldspawn" {
            world.extend(brushes(&mut e));
            wad_key = e.key("wad").unwrap_or_default().to_string();
            // The export writes its own `wad` and `mapversion`, and the `wad` paths are the author's own disk.
            world_keys.extend(e.properties.into_iter().filter(|(k, _)| !matches!(k.as_str(), "classname" | "wad" | "mapversion") && !k.starts_with("_tb_")));
        } else if let Some(kind) = tb_type.filter(|t| t == "_tb_layer" || t == "_tb_group") {
            let layer = kind == "_tb_layer";
            let id = e.key("_tb_id").unwrap_or_default().to_string();
            let name = e.key("_tb_name").map_or_else(|| format!("{} {id}", if layer { "Layer" } else { "Group" }), str::to_string);
            let parent = if layer { None } else { parent_of(&e) };
            let group = Group { name: String::new(), brushes: brushes(&mut e), entities: Vec::new() };
            containers.push(Container { layer, id, name, parent, group });
        } else if classname.is_empty() {
            unnamed += 1;
        } else {
            let parent = parent_of(&e);
            let brushes = brushes(&mut e);
            let properties = e.properties.into_iter().filter(|(k, _)| k != "classname" && !k.starts_with("_tb_")).collect();
            entities.push((parent, Entity { classname, properties, brushes }));
        }
    }

    let index: HashMap<(bool, &str), usize> = containers.iter().enumerate().map(|(i, c)| ((c.layer, c.id.as_str()), i)).collect();
    let names: Vec<String> = (0..containers.len()).map(|i| path(&containers, &index, i)).collect();
    let (mut top, mut placed) = (Vec::new(), vec![Vec::new(); containers.len()]);
    for (parent, entity) in entities {
        match parent.as_ref().and_then(|(layer, id)| index.get(&(*layer, id.as_str()))).and_then(|&i| placed.get_mut(i)) {
            Some(list) => list.push(entity),
            None => top.push(entity),
        }
    }
    let groups: Vec<Group> = containers
        .into_iter()
        .zip(names)
        .zip(placed)
        .map(|((c, name), entities)| Group { name, entities, ..c.group })
        .filter(|g| !g.brushes.is_empty() || !g.entities.is_empty())
        .collect();

    let all_entities = || top.iter().chain(groups.iter().flat_map(|g| &g.entities));
    let all_brushes = world.iter().chain(groups.iter().flat_map(|g| &g.brushes)).chain(all_entities().flat_map(|e| &e.brushes));
    let used: BTreeSet<String> = all_brushes.flat_map(faces).map(|f| f.material.clone()).collect();
    let entity_count = all_entities().count();
    let brush_entities = all_entities().filter(|e| !e.brushes.is_empty()).count();
    log.push(format!(
        "{} world brushes, {entity_count} entities ({brush_entities} with brushes), {} TrenchBroom layers and groups",
        world.len() + groups.iter().map(|g| g.brushes.len()).sum::<usize>(),
        groups.len()
    ));
    if parsed.degenerate > 0 {
        log.push(format!("left out {} faces whose points are on one line", parsed.degenerate));
    }
    if unnamed > 0 {
        log.push(format!("left out {unnamed} entities with no classname"));
    }

    let library = Library::open(files, &wad_key);
    log.extend(library.problems.iter().cloned());
    let folders = library.missing_folders(&wad_key);
    if !folders.is_empty() {
        let folders: Vec<&str> = folders.into_iter().collect();
        log.push(format!("to load those textures, set the SDK's WAD folder (File > SDK Settings) to {}", folders.join(" or ")));
    }
    let (pictures, failed) = library.pictures(files, Some(&used));
    let missing: Vec<&str> = used.iter().filter(|n| !library.wads.iter().any(|w| w.lumps.iter().any(|l| &l.name == *n))).map(String::as_str).collect();
    log.push(format!("textures: {} of {} found in {} WADs", pictures.len(), used.len(), library.wads.len()));
    if library.wads.is_empty() {
        log.push("no WAD files found: set the SDK's WAD folder or Quake folder".into());
    }
    if library.palette.is_none() && !failed.is_empty() {
        log.push("Quake's palette was not found: set the SDK's Quake folder (the one holding id1)".into());
    }
    if !missing.is_empty() {
        log.push(format!("not in any WAD: {}", missing.join(", ")));
    }
    log.extend(failed.into_iter().take(10).map(|f| format!("texture {f}")));
    let materials = pictures.into_iter().map(|(name, p)| material(name, p)).collect();
    if !world_keys.is_empty() {
        top.push(Entity { classname: "worldspawn".into(), properties: world_keys, brushes: Vec::new() });
    }
    Ok(Import { output: Output { brushes: world, entities: top, groups, materials }, log })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::time::Instant;

    use super::*;
    use crate::map::tests::assert_solid;
    use crate::wad::tests::{Disk, Mem, miptex, pak, test_palette, wad};

    fn cube(x: i32, texture: &str) -> String {
        let (a, b) = (x, x + 32);
        format!(
            "{{\n( {a} 32 32 ) ( {a} 0 32 ) ( {a} 0 0 ) {texture} 0 0 0 1 1\n( {b} 0 32 ) ( {b} 32 32 ) ( {b} 32 0 ) {texture} 0 0 0 1 1\n\
             ( {a} 0 32 ) ( {b} 0 32 ) ( {b} 0 0 ) {texture} 0 0 0 1 1\n( {b} 32 32 ) ( {a} 32 32 ) ( {a} 32 0 ) {texture} 0 0 0 1 1\n\
             ( {a} 32 0 ) ( {a} 0 0 ) ( {b} 0 0 ) {texture} 0 0 0 1 1\n( {a} 0 32 ) ( {a} 32 32 ) ( {b} 32 32 ) {texture} 0 0 0 1 1\n}}\n"
        )
    }

    fn entity(keys: &str, brushes: &[String]) -> String {
        format!("{{\n{keys}\n{}}}\n", brushes.concat())
    }

    fn no_files() -> Mem {
        Mem(HashMap::new())
    }

    #[test]
    fn trenchbroom_layers_and_groups_become_named_groups() {
        let text = [
            entity("\"classname\" \"worldspawn\"\n\"_tb_textures\" \"textures\"\n\"message\" \"Hi\"", &[cube(0, "a")]),
            entity(
                "\"classname\" \"func_group\"\n\"_tb_type\" \"_tb_layer\"\n\"_tb_name\" \"Detail\"\n\"_tb_id\" \"3\"\n\"_tb_layer_sort_index\" \"0\"",
                &[cube(100, "b")],
            ),
            entity(
                "\"classname\" \"func_group\"\n\"_tb_type\" \"_tb_group\"\n\"_tb_name\" \"Stairs\"\n\"_tb_id\" \"5\"\n\"_tb_layer\" \"3\"",
                &[cube(200, "c")],
            ),
            entity(
                "\"classname\" \"func_group\"\n\"_tb_type\" \"_tb_group\"\n\"_tb_name\" \"Rail\"\n\"_tb_id\" \"6\"\n\"_tb_group\" \"5\"",
                &[cube(300, "c"), cube(400, "c")],
            ),
            entity("\"classname\" \"light\"\n\"origin\" \"0 0 64\"\n\"_tb_group\" \"6\"", &[]),
            entity("\"classname\" \"info_player_start\"\n\"origin\" \"8 8 8\"\n\"_tb_layer\" \"3\"", &[]),
            entity("\"classname\" \"func_group\"\n\"_tb_type\" \"_tb_group\"\n\"_tb_name\" \"Pillars\"\n\"_tb_id\" \"7\"", &[cube(500, "d")]),
            entity("\"classname\" \"func_door\"\n\"_tb_group\" \"7\"\n\"speed\" \"50\"", &[cube(600, "e")]),
            entity("\"classname\" \"info_player_deathmatch\"", &[]),
            entity("\"classname\" \"light\"\n\"_tb_group\" \"99\"", &[]),
            entity("\"classname\" \"func_group\"\n\"_tb_type\" \"_tb_group\"\n\"_tb_id\" \"8\"\n\"_tb_group\" \"8\"", &[cube(700, "f")]),
            entity("\"classname\" \"func_group\"", &[cube(800, "g")]),
            entity("\"origin\" \"0 0 0\"", &[]),
        ]
        .concat();
        let import = import(&text, &no_files()).unwrap();
        let out = &import.output;
        assert_eq!(out.brushes.len(), 1);
        let top: Vec<&str> = out.entities.iter().map(|e| e.classname.as_str()).collect();
        assert_eq!(top, ["info_player_deathmatch", "light", "func_group", "worldspawn"]);
        let groups: Vec<(&str, usize, Vec<&str>)> =
            out.groups.iter().map(|g| (g.name.as_str(), g.brushes.len(), g.entities.iter().map(|e| e.classname.as_str()).collect())).collect();
        assert_eq!(
            groups,
            [
                ("Detail", 1, vec!["info_player_start"]),
                ("Detail / Stairs", 1, vec![]),
                ("Detail / Stairs / Rail", 2, vec!["light"]),
                ("Pillars", 1, vec!["func_door"]),
                ("Group 8", 1, vec![]),
            ]
        );
        let door = &out.groups[3].entities[0];
        assert_eq!(door.properties, [("speed".to_string(), "50".to_string())]);
        assert_eq!(door.brushes.len(), 1);
        let world = import.output.entities.iter().find(|e| e.classname == "worldspawn").unwrap();
        assert_eq!(world.properties, [("message".to_string(), "Hi".to_string())], "the world's keys come along, without TrenchBroom's own");
        assert!(import.log.iter().any(|l| l.contains("1 entities with no classname")));
    }

    #[test]
    fn finds_textures_through_the_wad_key() {
        let palette = test_palette();
        let mut fence = miptex("{fence", 16, 16, None);
        fence[40] = 255;
        let files = Mem(HashMap::from([
            ("wads/first.wad".to_string(), wad(b"WAD2", &[("brick", 0x44, miptex("brick", 16, 16, None)), ("sky1", 0x44, miptex("sky1", 32, 16, None))])),
            ("second.wad".to_string(), wad(b"WAD2", &[("BRICK", 0x44, miptex("BRICK", 32, 32, None)), ("{fence", 0x44, fence)])),
            ("id1/pak0.pak".to_string(), pak(&[("gfx/palette.lmp", palette)])),
        ]));
        let world = "\"classname\" \"worldspawn\"\n\"wad\" \"C:\\Quake\\second.wad;gone.wad\"";
        let text = entity(world, &[cube(0, "Brick"), cube(64, "SKY1"), cube(128, "{fence"), cube(192, "missing")]);
        let import = import(&text, &files).unwrap();
        let sizes: Vec<(&str, u32, u32)> =
            import.output.materials.iter().map(|m| (m.name.as_str(), m.pixels.as_ref().unwrap().width, m.pixels.as_ref().unwrap().height)).collect();
        assert_eq!(sizes, [("brick", 32, 32), ("sky1", 32, 16), ("{fence", 16, 16)]);
        let fence = import.output.materials[2].pixels.as_ref().unwrap();
        assert_eq!((fence.rgba[3], fence.rgba[7]), (0, 255));
        assert!(import.log.iter().any(|l| l == "not in any WAD: missing"), "{:?}", import.log);
        assert!(import.log.iter().any(|l| l.contains("gone.wad")), "{:?}", import.log);
        let no_palette = Mem(files.0.into_iter().filter(|(k, _)| !k.ends_with(".pak")).collect());
        let import = super::import(&text, &no_palette).unwrap();
        assert!(import.output.materials.is_empty());
        assert!(import.log.iter().any(|l| l.contains("palette was not found")), "{:?}", import.log);
    }

    /// Mark's Quake folder: id1 with a loose palette, and his WADs in its WAD folder.
    const QUAKE: &str = "/mnt/Storage/Quake";
    const MAP: &str = "/mnt/Storage/Quake/Maps/mc_q30th_bearlikelion.map";

    #[test]
    fn imports_marks_map_with_its_textures() {
        let (Ok(text), true) = (std::fs::read_to_string(MAP), PathBuf::from(QUAKE).join("WAD").is_dir()) else {
            eprintln!("skipped: {MAP} or its WADs are missing");
            return;
        };
        for roots in [vec![format!("{QUAKE}/WAD"), "/home/mark/.quakeforge".to_string()], vec![QUAKE.to_string()]] {
            let files = Disk(roots.iter().map(PathBuf::from).collect());
            let start = Instant::now();
            let import = import(&text, &files).unwrap();
            let elapsed = start.elapsed();
            let out = &import.output;
            for brush in out.brushes.iter().chain(out.entities.iter().flat_map(|e| &e.brushes)) {
                let Brush::Faces(faces) = brush else { panic!("hull") };
                assert_solid(faces);
            }
            let sizes: Vec<(&str, u32, u32)> =
                out.materials.iter().map(|m| (m.name.as_str(), m.pixels.as_ref().unwrap().width, m.pixels.as_ref().unwrap().height)).collect();
            eprintln!("{roots:?}: {elapsed:?}, {} brushes, {} entities, textures {sizes:?}", out.brushes.len(), out.entities.len());
            eprintln!("{:#?}", import.log);
            assert_eq!((out.brushes.len(), out.entities.len()), (67, 8), "seven entities and the world's keys");
            assert_eq!(sizes.len(), 6, "{:?}", import.log);
            assert!(sizes.contains(&("skyik2", 256, 128)), "{sizes:?}");
        }
    }
}
