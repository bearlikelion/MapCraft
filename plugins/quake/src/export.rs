//! Writes a map as a Valve220 `.map`, with layers and groups as TrenchBroom `func_group` entities.

use std::collections::BTreeSet;

use glam::DVec3;

use crate::map::standard_axes;
use crate::mapcraft::plugin::types::{Face, MapData, MapNodeKind, TexAxis, Vec3};
use crate::wad::{Files, Library};

/// Quake stores texture names in 16 bytes with a closing zero.
const MAX_TEXTURE_NAME: usize = 15;

#[derive(Clone, Copy, PartialEq)]
enum Owner {
    Layer,
    Group,
    Entity,
    Brush,
}

fn quoted(text: &str) -> Result<String, String> {
    if text.contains(['"', '\n', '\r']) {
        return Err(format!("`{text}` has a quote or line break, which a .map cannot hold"));
    }
    Ok(format!("\"{text}\""))
}

fn pair(out: &mut String, key: &str, value: &str) -> Result<(), String> {
    out.push_str(&format!("{} {}\n", quoted(key)?, quoted(value)?));
    Ok(())
}

/// The shortest text that reads back as exactly this number.
fn number(v: f64) -> Result<String, String> {
    if v.is_finite() { Ok(format!("{}", v + 0.0)) } else { Err("a brush has a coordinate or texture value that is not a number".into()) }
}

fn point(p: &Vec3) -> Result<String, String> {
    Ok(format!("( {} {} {} )", number(p.x)?, number(p.y)?, number(p.z)?))
}

fn axis(a: &TexAxis) -> Result<String, String> {
    Ok(format!("[ {} {} {} {} ]", number(a.axis.x)?, number(a.axis.y)?, number(a.axis.z)?, number(a.offset)?))
}

fn dv(p: &Vec3) -> DVec3 {
    DVec3::new(p.x, p.y, p.z)
}

fn face(out: &mut String, f: &Face, long_names: &mut BTreeSet<String>) -> Result<(), String> {
    let name = &f.material;
    if name.is_empty() || name.contains(|c: char| c.is_whitespace() || c == '"') {
        return Err(format!("the material `{name}` cannot be written to a .map"));
    }
    if name.len() > MAX_TEXTURE_NAME {
        long_names.insert(name.clone());
    }
    let (p0, p1, p2) = &f.points;
    // Faces without axes get Quake's own world alignment.
    let normal = (dv(p0) - dv(p1)).cross(dv(p2) - dv(p1)).normalize_or_zero();
    let (u, v) = f.uv.unwrap_or_else(|| standard_axes(normal, [0.0, 0.0, 0.0, 1.0, 1.0]));
    let line = [point(p0)?, point(p1)?, point(p2)?, name.clone(), axis(&u)?, axis(&v)?, "0".into(), number(u.scale)?, number(v.scale)?];
    out.push_str(&line.join(" "));
    out.push('\n');
    Ok(())
}

/// A `wad` key for a world that has none: the file names of the WADs holding the map's textures, for qbsp's `-wadpath`.
pub fn default_wad(map: &MapData, files: &dyn Files) -> Option<String> {
    if map.world.iter().any(|(k, _)| k == "wad") {
        return None;
    }
    let used: BTreeSet<&str> = map
        .nodes
        .iter()
        .filter_map(|n| match &n.kind {
            MapNodeKind::Brush(s) => Some(s.faces.iter().map(|f| f.material.as_str())),
            _ => None,
        })
        .flatten()
        .collect();
    Some(Library::open(files, "").wad_key(&used)).filter(|key| !key.is_empty())
}

pub struct Export {
    pub text: String,
    pub warnings: Vec<String>,
}

pub fn export(map: &MapData) -> Result<Export, String> {
    let count = map.nodes.len();
    let (mut owners, mut parents) = (Vec::with_capacity(count), Vec::with_capacity(count));
    // The brushes each node owns, by node index; brushes at the top belong to the world.
    let (mut brushes, mut world) = (vec![Vec::new(); count], Vec::new());
    for (i, node) in map.nodes.iter().enumerate() {
        let parent = node
            .parent
            .map(|p| usize::try_from(p).ok().filter(|&p| p < i).ok_or_else(|| format!("node {i} has a parent that does not come before it")))
            .transpose()?;
        let owner = match &node.kind {
            MapNodeKind::Layer(_) if parent.is_none() => Owner::Layer,
            MapNodeKind::Layer(_) | MapNodeKind::Group(_) => Owner::Group,
            MapNodeKind::Entity(_) => Owner::Entity,
            MapNodeKind::Brush(_) => Owner::Brush,
        };
        match (parent, parent.and_then(|p| owners.get(p).copied())) {
            (_, Some(Owner::Brush)) => return Err(format!("node {i} sits inside a brush")),
            (_, Some(Owner::Entity)) if owner != Owner::Brush => return Err(format!("node {i} sits inside an entity, where only brushes can go")),
            (Some(p), _) if owner == Owner::Brush => brushes.get_mut(p).into_iter().for_each(|list: &mut Vec<usize>| list.push(i)),
            (None, _) if owner == Owner::Brush => world.push(i),
            _ => {}
        }
        owners.push(owner);
        parents.push(parent);
    }

    let mut out = String::from("// Game: Quake\n// Format: Valve\n// entity 0\n{\n");
    let (mut long_names, mut skipped) = (BTreeSet::new(), 0);
    let mut write_brushes = |out: &mut String, list: &[usize]| -> Result<(), String> {
        for (n, solid) in list
            .iter()
            .filter_map(|&b| match map.nodes.get(b).map(|n| &n.kind) {
                Some(MapNodeKind::Brush(solid)) => Some(solid),
                _ => None,
            })
            .enumerate()
        {
            if solid.faces.len() < 4 {
                skipped += 1;
                continue;
            }
            out.push_str(&format!("// brush {n}\n{{\n"));
            for f in &solid.faces {
                face(out, f, &mut long_names)?;
            }
            out.push_str("}\n");
        }
        Ok(())
    };
    pair(&mut out, "classname", "worldspawn")?;
    pair(&mut out, "mapversion", "220")?;
    for (k, v) in map.world.iter().filter(|(k, _)| k != "classname" && k != "mapversion") {
        pair(&mut out, k, v)?;
    }
    write_brushes(&mut out, &world)?;
    out.push_str("}\n");

    let mut number = 0;
    for (i, node) in map.nodes.iter().enumerate() {
        let id = (i + 1).to_string();
        match &node.kind {
            MapNodeKind::Brush(_) => continue,
            MapNodeKind::Layer(name) | MapNodeKind::Group(name) => {
                number += 1;
                out.push_str(&format!("// entity {number}\n{{\n"));
                pair(&mut out, "classname", "func_group")?;
                pair(&mut out, "_tb_type", if owners.get(i) == Some(&Owner::Layer) { "_tb_layer" } else { "_tb_group" })?;
                pair(&mut out, "_tb_name", name)?;
                pair(&mut out, "_tb_id", &id)?;
            }
            MapNodeKind::Entity(e) => {
                if e.classname.is_empty() || e.classname == "worldspawn" {
                    return Err(format!("node {i} is an entity named `{}`, which a .map cannot hold besides the world", e.classname));
                }
                number += 1;
                out.push_str(&format!("// entity {number}\n{{\n"));
                pair(&mut out, "classname", &e.classname)?;
                for (k, v) in e.properties.iter().filter(|(k, _)| k != "classname" && !k.starts_with("_tb_")) {
                    pair(&mut out, k, v)?;
                }
            }
        }
        if let Some(p) = parents.get(i).copied().flatten() {
            match owners.get(p) {
                Some(Owner::Layer) => pair(&mut out, "_tb_layer", &(p + 1).to_string())?,
                Some(Owner::Group) => pair(&mut out, "_tb_group", &(p + 1).to_string())?,
                _ => {}
            }
        }
        write_brushes(&mut out, brushes.get(i).map_or(&[][..], Vec::as_slice))?;
        out.push_str("}\n");
    }

    let mut warnings = Vec::new();
    if !long_names.is_empty() {
        let names: Vec<String> = long_names.into_iter().collect();
        warnings.push(format!("texture names longer than {MAX_TEXTURE_NAME} characters do not fit Quake's WADs and BSPs: {}", names.join(", ")));
    }
    if skipped > 0 {
        warnings.push(format!("left out {skipped} brushes with fewer than four faces"));
    }
    Ok(Export { text: out, warnings })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::time::Instant;

    use super::*;
    use crate::import::import;
    use crate::map::tests::{MAPS, assert_solid, bsp_texinfo, marks_maps};
    use crate::map::{MapEntity, parse};
    use crate::mapcraft::plugin::types::{Brush, MapEntity as Ent, MapNode, Output, Solid};
    use crate::wad::tests::{Mem, miptex, wad};

    fn no_files() -> Mem {
        Mem(HashMap::new())
    }

    fn node(parent: Option<usize>, kind: MapNodeKind) -> MapNode {
        MapNode { parent: parent.map(|p| p as u32), kind }
    }

    fn solid(brush: &Brush) -> MapNodeKind {
        let Brush::Faces(faces) = brush else { panic!("hull") };
        MapNodeKind::Brush(Solid { faces: faces.clone(), vertices: Vec::new() })
    }

    fn entity(nodes: &mut Vec<MapNode>, parent: Option<usize>, e: &crate::mapcraft::plugin::types::Entity) {
        nodes.push(node(parent, MapNodeKind::Entity(Ent { classname: e.classname.clone(), properties: e.properties.clone() })));
        let at = nodes.len() - 1;
        nodes.extend(e.brushes.iter().map(|b| node(Some(at), solid(b))));
    }

    /// What the editor holds after importing `output`, optionally inside the layer an import makes.
    fn editor_map(name: &str, world: Vec<(String, String)>, output: &Output, layer: Option<&str>) -> MapData {
        let mut nodes = Vec::new();
        let top = layer.map(|l| {
            nodes.push(node(None, MapNodeKind::Layer(l.into())));
            0
        });
        nodes.extend(output.brushes.iter().map(|b| node(top, solid(b))));
        // The editor turns an imported `worldspawn` into the world's keys rather than an entity.
        for e in output.entities.iter().filter(|e| e.classname != "worldspawn") {
            entity(&mut nodes, top, e);
        }
        for g in &output.groups {
            nodes.push(node(top, MapNodeKind::Group(g.name.clone())));
            let at = nodes.len() - 1;
            nodes.extend(g.brushes.iter().map(|b| node(Some(at), solid(b))));
            for e in &g.entities {
                entity(&mut nodes, Some(at), e);
            }
        }
        MapData { name: name.into(), world, nodes }
    }

    /// The map as TrenchBroom holds it: layers and groups nested as the file's `_tb_*` keys say.
    fn trenchbroom_map_data(parsed: Vec<MapEntity>) -> MapData {
        let mut nodes = Vec::new();
        let mut world = Vec::new();
        let mut ids: HashMap<(bool, String), usize> = HashMap::new();
        let parent = |e: &MapEntity, ids: &HashMap<(bool, String), usize>| {
            e.key("_tb_group").map(|g| ids[&(false, g.to_string())]).or_else(|| e.key("_tb_layer").map(|l| ids[&(true, l.to_string())]))
        };
        for e in parsed {
            let brushes: Vec<MapNodeKind> = e.brushes.iter().map(|f| MapNodeKind::Brush(Solid { faces: f.clone(), vertices: Vec::new() })).collect();
            let at = nodes.len();
            match (e.key("classname"), e.key("_tb_type")) {
                (Some("worldspawn"), _) => {
                    world = e.properties.iter().filter(|(k, _)| k != "classname").cloned().collect();
                    nodes.extend(brushes.into_iter().map(|b| node(None, b)));
                    continue;
                }
                (_, Some(kind @ ("_tb_layer" | "_tb_group"))) => {
                    let name = e.key("_tb_name").unwrap().to_string();
                    let layer = kind == "_tb_layer";
                    nodes.push(node(parent(&e, &ids), if layer { MapNodeKind::Layer(name) } else { MapNodeKind::Group(name) }));
                    ids.insert((layer, e.key("_tb_id").unwrap().to_string()), at);
                }
                (Some(classname), _) => {
                    let properties = e.properties.iter().filter(|(k, _)| k != "classname").cloned().collect();
                    nodes.push(node(parent(&e, &ids), MapNodeKind::Entity(Ent { classname: classname.into(), properties })));
                }
                _ => panic!("entity with no classname"),
            }
            nodes.extend(brushes.into_iter().map(|b| node(Some(at), b)));
        }
        MapData { name: "test".into(), world, nodes }
    }

    /// The entities an import places; its `worldspawn` holds the world's keys, checked on their own.
    fn placed(entities: &[crate::mapcraft::plugin::types::Entity]) -> Vec<&crate::mapcraft::plugin::types::Entity> {
        entities.iter().filter(|e| e.classname != "worldspawn").collect()
    }

    fn fingerprint(output: &Output) -> String {
        format!("{:?}\n{:?}\n{:?}", output.brushes, placed(&output.entities), output.groups)
    }

    fn worldspawn(text: &str) -> Vec<(String, String)> {
        parse(text).unwrap().entities.into_iter().find(|e| e.key("classname") == Some("worldspawn")).unwrap().properties
    }

    fn square(x: f64, material: &str) -> Vec<Face> {
        let text = format!(
            "{{ \"classname\" \"worldspawn\" {{\n( {x} 32 32 ) ( {x} 0 32 ) ( {x} 0 0 ) {material} 0 0 0 1 1\n( {b} 0 32 ) ( {b} 32 32 ) ( {b} 32 0 ) {material} 0 0 0 1 1\n\
             ( {x} 0 32 ) ( {b} 0 32 ) ( {b} 0 0 ) {material} 0 0 0 1 1\n( {b} 32 32 ) ( {x} 32 32 ) ( {x} 32 0 ) {material} 0 0 0 1 1\n\
             ( {x} 32 0 ) ( {x} 0 0 ) ( {b} 0 0 ) {material} 0 0 0 1 1\n( {x} 0 32 ) ( {x} 32 32 ) ( {b} 32 32 ) {material} 0.5 -3 0 1 1\n}} }}",
            b = x + 32.0
        );
        parse(&text).unwrap().entities.swap_remove(0).brushes.swap_remove(0)
    }

    fn brush(x: f64, material: &str) -> MapNodeKind {
        MapNodeKind::Brush(Solid { faces: square(x, material), vertices: Vec::new() })
    }

    fn ent(classname: &str, properties: &[(&str, &str)]) -> MapNodeKind {
        MapNodeKind::Entity(Ent { classname: classname.into(), properties: properties.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() })
    }

    #[test]
    fn writes_valve220_with_trenchbroom_layers_and_groups() {
        let mut map = MapData {
            name: "demo".into(),
            world: vec![("mapversion".into(), "100".into()), ("wad".into(), "a.wad;b.wad".into()), ("message".into(), "Demo".into())],
            nodes: vec![
                node(None, brush(0.0, "ewall1")),
                node(None, MapNodeKind::Layer("Detail".into())),
                node(Some(1), brush(64.0, "a_texture_name_too_long")),
                node(Some(1), MapNodeKind::Group("Stairs".into())),
                node(Some(3), MapNodeKind::Group("Rail".into())),
                node(Some(4), ent("light", &[("origin", "8 8 8"), ("_tb_group", "99")])),
                node(Some(4), ent("func_door", &[("speed", "50")])),
                node(Some(6), brush(128.0, "*water1")),
                node(Some(4), brush(192.0, "+0button")),
                node(None, ent("info_player_start", &[("origin", "0 0 40")])),
                node(Some(1), MapNodeKind::Layer("Nested layer".into())),
            ],
        };
        // A face without axes gets Quake's world alignment.
        if let MapNodeKind::Brush(s) = &mut map.nodes[0].kind {
            s.faces[5].uv = None;
        }
        let export = export(&map).unwrap();
        let text = &export.text;
        assert!(text.starts_with("// Game: Quake\n// Format: Valve\n// entity 0\n{\n\"classname\" \"worldspawn\"\n\"mapversion\" \"220\"\n\"wad\" \"a.wad;b.wad\"\n\"message\" \"Demo\"\n"), "{text}");
        assert!(text.contains("( 0 32 32 ) ( 0 0 32 ) ( 0 0 0 ) ewall1 [ 0 1 0 0 ] [ 0 0 -1 0 ] 0 1 1\n"), "{text}");
        assert!(text.contains("( 0 0 32 ) ( 0 32 32 ) ( 32 32 32 ) ewall1 [ 1 0 0 0 ] [ 0 -1 0 0 ] 0 1 1\n"), "{text}");
        assert!(text.contains("\"classname\" \"func_group\"\n\"_tb_type\" \"_tb_layer\"\n\"_tb_name\" \"Detail\"\n\"_tb_id\" \"2\"\n"), "{text}");
        assert!(text.contains("\"_tb_type\" \"_tb_group\"\n\"_tb_name\" \"Rail\"\n\"_tb_id\" \"5\"\n\"_tb_group\" \"4\"\n"), "{text}");
        assert!(text.contains("\"classname\" \"light\"\n\"origin\" \"8 8 8\"\n\"_tb_group\" \"5\"\n}"), "{text}");
        assert_eq!(export.warnings, ["texture names longer than 15 characters do not fit Quake's WADs and BSPs: a_texture_name_too_long"]);

        let back = import(text, &no_files()).unwrap().output;
        assert_eq!(back.brushes.len(), 1);
        assert_eq!(back.entities.iter().map(|e| e.classname.as_str()).collect::<Vec<_>>(), ["info_player_start", "worldspawn"]);
        assert_eq!(back.entities[1].properties, [("message".to_string(), "Demo".to_string())], "the world's keys survive the round trip");
        let groups: Vec<(&str, usize, Vec<&str>)> =
            back.groups.iter().map(|g| (g.name.as_str(), g.brushes.len(), g.entities.iter().map(|e| e.classname.as_str()).collect())).collect();
        assert_eq!(groups, [("Detail", 1, vec![]), ("Detail / Stairs / Rail", 1, vec!["light", "func_door"])]);
        let door = &back.groups[1].entities[1];
        assert_eq!((door.properties.clone(), door.brushes.len()), (vec![("speed".to_string(), "50".to_string())], 1));
        let world = worldspawn(text);
        assert_eq!(world.iter().filter(|(k, _)| k == "mapversion").count(), 1);
    }

    fn with_face(change: impl Fn(&mut Face)) -> MapNodeKind {
        let mut faces = square(0.0, "a");
        change(&mut faces[2]);
        MapNodeKind::Brush(Solid { faces, vertices: Vec::new() })
    }

    #[test]
    fn names_the_wads_a_world_without_a_wad_key_needs() {
        let files = Mem(HashMap::from([
            ("a.wad".to_string(), wad(b"WAD2", &[("ewall1", 0x44, miptex("ewall1", 16, 16, None))])),
            ("sub/B.WAD".to_string(), wad(b"WAD2", &[("ewall1", 0x44, miptex("ewall1", 16, 16, None)), ("*water1", 0x44, miptex("*water1", 16, 16, None))])),
            ("c.wad".to_string(), wad(b"WAD2", &[("unused", 0x44, miptex("unused", 16, 16, None))])),
        ]));
        let mut map = MapData { name: "x".into(), world: Vec::new(), nodes: vec![node(None, brush(0.0, "ewall1")), node(None, brush(64.0, "*water1"))] };
        assert_eq!(default_wad(&map, &files).as_deref(), Some("a.wad;B.WAD"));
        map.nodes.truncate(1);
        assert_eq!(default_wad(&map, &files).as_deref(), Some("a.wad"));
        map.world.push(("wad".into(), "mine.wad".into()));
        assert_eq!(default_wad(&map, &files), None);
        assert_eq!(default_wad(&MapData { name: "x".into(), world: Vec::new(), nodes: Vec::new() }, &files), None);
    }

    #[test]
    fn refuses_what_a_map_cannot_hold() {
        let world = |nodes: Vec<MapNode>| MapData { name: "x".into(), world: Vec::new(), nodes };
        let bad = [
            world(vec![node(Some(0), brush(0.0, "a"))]),
            world(vec![node(Some(5), brush(0.0, "a"))]),
            world(vec![node(None, brush(0.0, "a")), node(Some(0), brush(64.0, "a"))]),
            world(vec![node(None, ent("func_wall", &[])), node(Some(0), MapNodeKind::Group("g".into()))]),
            world(vec![node(None, ent("light", &[("message", "say \"hi\"")]))]),
            world(vec![node(None, ent("light", &[("message", "two\nlines")]))]),
            world(vec![node(None, ent("", &[]))]),
            world(vec![node(None, ent("worldspawn", &[]))]),
            world(vec![node(None, with_face(|f| f.material = "has space".into()))]),
            world(vec![node(None, with_face(|f| f.material.clear()))]),
            world(vec![node(None, with_face(|f| f.points.1.y = f64::NAN))]),
            world(vec![node(None, with_face(|f| f.uv.as_mut().unwrap().0.scale = f64::INFINITY))]),
            MapData { name: "x".into(), world: vec![("wad".into(), "\"".into())], nodes: Vec::new() },
        ];
        for map in &bad {
            assert!(export(map).is_err());
        }
        let mut thin = square(0.0, "a");
        thin.truncate(3);
        let export = export(&world(vec![node(None, MapNodeKind::Brush(Solid { faces: thin, vertices: Vec::new() }))])).unwrap();
        assert_eq!(export.warnings, ["left out 1 brushes with fewer than four faces"]);
        assert!(!export.text.contains("// brush"));
    }

    #[test]
    fn round_trips_marks_maps() {
        let maps = marks_maps();
        if maps.is_empty() {
            eprintln!("skipped: no maps in {MAPS}");
            return;
        }
        for path in maps {
            let text = std::fs::read_to_string(&path).unwrap();
            let first = import(&text, &no_files()).unwrap().output;
            let mut world = worldspawn(&text);
            world.retain(|(k, _)| k != "classname");
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            let exported = export(&editor_map(&name, world.clone(), &first, None)).unwrap();
            let again = import(&exported.text, &no_files()).unwrap().output;
            assert_eq!(fingerprint(&again), fingerprint(&first), "{}", path.display());
            let mut expected: Vec<(String, String)> = vec![("classname".into(), "worldspawn".into()), ("mapversion".into(), "220".into())];
            expected.extend(world.into_iter().filter(|(k, _)| k != "mapversion"));
            assert_eq!(worldspawn(&exported.text), expected);
            // Inside the layer an import makes, everything comes back in that layer.
            let layered = import(&export(&editor_map(&name, Vec::new(), &first, Some("Imported"))).unwrap().text, &no_files()).unwrap().output;
            assert!(layered.brushes.is_empty() && layered.entities.is_empty());
            assert_eq!(layered.groups[0].name, "Imported");
            assert_eq!(format!("{:?}{:?}", layered.groups[0].brushes, layered.groups[0].entities), format!("{:?}{:?}", first.brushes, placed(&first.entities)));
            eprintln!("{}: {} brushes and {} entities round trip", path.display(), first.brushes.len(), first.entities.len());
        }
    }

    /// A brush 32 units wide with Valve220 axes that vary with `n`; every fifth has a sloped top.
    fn generated_brush(x: i32, y: i32, z: i32, texture: &str, n: usize) -> String {
        let (a, b, c, d, e, f) = (x, x + 32, y, y + 32, z, z + 32);
        let top = if n.is_multiple_of(5) {
            format!("( {a} {c} {f} ) ( {a} {d} {f} ) ( {b} {d} {} )", e + 16)
        } else {
            format!("( {a} {c} {f} ) ( {a} {d} {f} ) ( {b} {d} {f} )")
        };
        let planes = [
            format!("( {a} {d} {f} ) ( {a} {c} {f} ) ( {a} {c} {e} )"),
            format!("( {b} {c} {f} ) ( {b} {d} {f} ) ( {b} {d} {e} )"),
            format!("( {a} {c} {f} ) ( {b} {c} {f} ) ( {b} {c} {e} )"),
            format!("( {b} {d} {f} ) ( {a} {d} {f} ) ( {a} {d} {e} )"),
            format!("( {a} {d} {e} ) ( {a} {c} {e} ) ( {b} {c} {e} )"),
            top,
        ];
        let (s, co) = (n as f64 * 15.0).to_radians().sin_cos();
        let scale = [1.0, 0.5, -2.0, 0.25][n % 4];
        let (u_offset, v_offset) = ((n % 13) as f64 * 0.37, 0.0 - (n % 7) as f64);
        let faces: String = planes
            .iter()
            .map(|p| format!("{p} {texture} [ {co} {s} 0 {u_offset} ] [ {} {} -0.7071 {v_offset} ] {} {scale} 0.75\n", -s * 0.5, co * 0.5, n % 360))
            .collect();
        format!("{{\n{faces}}}\n")
    }

    /// TrenchBroom's layout at the size of a big map: 11,515 brushes in three layers with nested groups, and 330 entities.
    fn big_trenchbroom_map() -> String {
        const TEXTURES: [&str; 8] = ["kstone2", "ewall1", "ground1_1", "skyik2", "*water1", "+0button", "clip", "trigger"];
        let mut out = String::from(
            "// Game: Quake\n// Format: Valve\n{\n\"mapversion\" \"220\"\n\"classname\" \"worldspawn\"\n\"wad\" \"/mnt/Storage/Quake/WAD/mc_anniv_jam.wad\"\n",
        );
        let mut n = 0;
        let mut add = |out: &mut String, x: usize, y: usize, z: i32, texture: &str| {
            n += 1;
            out.push_str(&generated_brush(x as i32 * 40, y as i32 * 40, z, texture, n));
        };
        for i in 0..4000 {
            add(&mut out, i % 80, i / 80, 0, TEXTURES[i % 8]);
        }
        out.push_str("}\n");
        for layer in 0..3 {
            let lid = 100 + layer;
            out.push_str(&format!("{{\n\"classname\" \"func_group\"\n\"_tb_type\" \"_tb_layer\"\n\"_tb_name\" \"Layer {layer}\"\n\"_tb_id\" \"{lid}\"\n\"_tb_layer_sort_index\" \"{layer}\"\n"));
            for i in 0..1000 {
                add(&mut out, i % 40, i / 40, 100 + layer as i32 * 100, "kstone4archs");
            }
            out.push_str("}\n");
            for group in 0..5 {
                let gid = 1000 + layer * 10 + group;
                out.push_str(&format!("{{\n\"classname\" \"func_group\"\n\"_tb_type\" \"_tb_group\"\n\"_tb_name\" \"Group {group}\"\n\"_tb_id\" \"{gid}\"\n\"_tb_layer\" \"{lid}\"\n"));
                for i in 0..200 {
                    add(&mut out, i % 20, i / 20, 1000 + (layer * 500 + group * 40) as i32, "jf_rustfloor1");
                }
                out.push_str("}\n");
                let sub = gid * 10;
                out.push_str(&format!(
                    "{{\n\"classname\" \"func_group\"\n\"_tb_type\" \"_tb_group\"\n\"_tb_name\" \"Sub\"\n\"_tb_id\" \"{sub}\"\n\"_tb_group\" \"{gid}\"\n"
                ));
                for i in 0..100 {
                    add(&mut out, i % 20, i / 20, -1000 - (layer * 500 + group * 40) as i32, "ewall1");
                }
                out.push_str("}\n");
                out.push_str(&format!("{{\n\"classname\" \"func_door\"\n\"_tb_group\" \"{sub}\"\n\"speed\" \"100\"\n"));
                add(&mut out, 125 + group, layer, 0, "+0button");
                out.push_str("}\n");
                out.push_str(&format!("{{\n\"classname\" \"light\"\n\"origin\" \"{} {} 50\"\n\"_tb_group\" \"{gid}\"\n}}\n", group * 10, layer * 10));
            }
        }
        for i in 0..300 {
            out.push_str(&format!("{{\n\"classname\" \"monster_army\"\n\"origin\" \"{} -500 0\"\n\"angle\" \"90\"\n}}\n", i * 10));
        }
        out
    }

    #[test]
    fn round_trips_a_big_trenchbroom_map() {
        let text = big_trenchbroom_map();
        let start = Instant::now();
        let first = import(&text, &no_files()).unwrap().output;
        let imported = start.elapsed();
        let parsed = parse(&text).unwrap();
        for brush in parsed.entities.iter().flat_map(|e| &e.brushes) {
            assert_solid(brush);
        }
        let start = Instant::now();
        let exported = export(&trenchbroom_map_data(parsed.entities)).unwrap();
        let elapsed = start.elapsed();
        let again = import(&exported.text, &no_files()).unwrap().output;
        let count =
            |o: &Output| o.brushes.len() + o.groups.iter().map(|g| g.brushes.len() + g.entities.iter().map(|e| e.brushes.len()).sum::<usize>()).sum::<usize>();
        assert_eq!((count(&first), first.groups.len()), (11515, 33));
        for (a, b) in fingerprint(&again).lines().zip(fingerprint(&first).lines()) {
            if a != b {
                let at = a.bytes().zip(b.bytes()).position(|(x, y)| x != y).unwrap_or(0);
                panic!(
                    "round trip differs at {at}: {} vs {}",
                    &a[at.saturating_sub(300)..(at + 200).min(a.len())],
                    &b[at.saturating_sub(300)..(at + 200).min(b.len())]
                );
            }
        }
        let names: Vec<&str> = again.groups.iter().map(|g| g.name.as_str()).take(3).collect();
        assert_eq!(names, ["Layer 0", "Layer 0 / Group 0", "Layer 0 / Group 0 / Sub"]);
        // The flat groups an import makes come back under the same names.
        let flat = export(&editor_map("big", Vec::new(), &first, None)).unwrap();
        assert_eq!(fingerprint(&import(&flat.text, &no_files()).unwrap().output), fingerprint(&first));
        eprintln!("{} bytes in, {} out; import {imported:?}, export {elapsed:?} (debug build)", text.len(), exported.text.len());
    }

    const QBSP: &str = "/mnt/Storage/Quake/ericw-tools-v0.18.1-Linux/bin/qbsp";

    fn lump(bsp: &[u8], index: usize) -> &[u8] {
        let at = |i: usize| u32::from_le_bytes(bsp[i..i + 4].try_into().unwrap()) as usize;
        let (offset, length) = (at(4 + index * 8), at(8 + index * 8));
        &bsp[offset..offset + length]
    }

    /// Texture vectors rounded to 1e-4, with their texture's name and flags.
    type Texinfo = (Vec<i64>, String, u32);

    /// What qbsp compiled: its texinfo sorted, then the face and model counts.
    fn compiled(bsp: &[u8]) -> (Vec<Texinfo>, usize, usize) {
        let textures = lump(bsp, 2);
        let count = u32::from_le_bytes(textures[..4].try_into().unwrap()) as usize;
        let names: Vec<String> = (0..count)
            .map(|i| {
                let at = u32::from_le_bytes(textures[4 + i * 4..8 + i * 4].try_into().unwrap()) as usize;
                String::from_utf8_lossy(&textures[at..at + 16]).trim_end_matches('\0').to_string()
            })
            .collect();
        let mut infos: Vec<Texinfo> = lump(bsp, 6)
            .as_chunks::<40>()
            .0
            .iter()
            .zip(bsp_texinfo(bsp))
            .map(|(raw, vecs)| {
                let word = |i: usize| u32::from_le_bytes(raw[i..i + 4].try_into().unwrap());
                (vecs.iter().map(|v| (v * 1e4).round() as i64).collect(), names[word(32) as usize].clone(), word(36))
            })
            .collect();
        infos.sort();
        (infos, lump(bsp, 7).len() / 20, lump(bsp, 14).len() / 64)
    }

    fn qbsp(dir: &std::path::Path, name: &str, text: &str) -> Vec<u8> {
        let map = dir.join(format!("{name}.map"));
        std::fs::write(&map, text).unwrap();
        let run = std::process::Command::new(QBSP).arg(&map).current_dir(dir).output().unwrap();
        let log = String::from_utf8_lossy(&run.stdout);
        assert!(run.status.success(), "{log}");
        eprintln!("{name}: {}", log.lines().filter(|l| l.contains("texinfo") || l.contains("ERROR")).map(str::trim).collect::<Vec<_>>().join(", "));
        std::fs::read(dir.join(format!("{name}.bsp"))).unwrap()
    }

    #[test]
    fn qbsp_compiles_an_export_like_its_source() {
        let source = format!("{MAPS}/mc_q30th_bearlikelion.map");
        let (Ok(text), true) = (std::fs::read_to_string(&source), PathBuf::from(QBSP).is_file()) else {
            eprintln!("skipped: Mark's map or ericw-tools is missing");
            return;
        };
        let first = import(&text, &no_files()).unwrap().output;
        let mut world = worldspawn(&text);
        world.retain(|(k, _)| k != "classname");
        let exported = export(&editor_map("exported", world, &first, Some("mc_q30th_bearlikelion.map"))).unwrap();
        let dir = std::env::temp_dir().join(format!("mapcraft-quake-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (original, ours) = (compiled(&qbsp(&dir, "original", &text)), compiled(&qbsp(&dir, "exported", &exported.text)));
        let _ = std::fs::remove_dir_all(&dir);
        eprintln!("texinfo {}, faces {}, models {}", ours.0.len(), ours.1, ours.2);
        // The import layer's func_group merges into the world, so both have the world model alone.
        assert_eq!(ours, original);
    }
}
