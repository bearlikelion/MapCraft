//! Reads and writes Valve Hammer `.vmf` maps: world brushes, entities with their brushes, and visgroups as groups.

use std::collections::HashMap;

use valve::Token;

use crate::mapcraft::plugin::types::{Brush, Entity, Face, Group, MapData, MapNodeKind, Output, Solid, TexAxis, Vec3};

/// Deeper nesting than this is a broken file, not a mapper's work.
const MAX_DEPTH: usize = 64;
const NODRAW: &str = "tools/toolsnodraw";

/// A named block of key values and child blocks.
pub struct Block<'a> {
    pub name: &'a str,
    pub keys: Vec<(&'a str, &'a str)>,
    pub children: Vec<Block<'a>>,
}

impl<'a> Block<'a> {
    fn key(&self, key: &str) -> Option<&'a str> {
        self.keys.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| *v)
    }

    fn blocks<'s>(&'s self, name: &'s str) -> impl Iterator<Item = &'s Block<'a>> {
        self.children.iter().filter(move |c| c.name.eq_ignore_ascii_case(name))
    }
}

fn block<'a>(name: &'a str, tokens: &mut impl Iterator<Item = Token<'a>>, depth: usize) -> Result<Block<'a>, String> {
    if depth > MAX_DEPTH {
        return Err(format!("blocks nest deeper than {MAX_DEPTH}"));
    }
    let mut b = Block { name, keys: Vec::new(), children: Vec::new() };
    loop {
        match tokens.next() {
            Some(Token::Close) => return Ok(b),
            Some(Token::Text(k)) => match tokens.next() {
                Some(Token::Text(v)) => b.keys.push((k, v)),
                Some(Token::Open) => b.children.push(block(k, tokens, depth + 1)?),
                Some(Token::Close) => return Err(format!("`{k}` in a `{name}` block has no value")),
                None => break,
            },
            Some(Token::Open) => return Err(format!("a `{name}` block holds a block with no name")),
            None => break,
        }
    }
    Err(format!("the file ends inside a `{name}` block"))
}

/// The file's top-level blocks.
pub fn parse(text: &str) -> Result<Vec<Block<'_>>, String> {
    let mut tokens = valve::tokens(text.trim_start_matches('\u{feff}')).into_iter();
    let mut out = Vec::new();
    while let Some(token) = tokens.next() {
        match (token, tokens.next()) {
            (Token::Text(name), Some(Token::Open)) => out.push(block(name, &mut tokens, 1)?),
            _ => return Err("not a .vmf file: expected a named block at the top level".into()),
        }
    }
    Ok(out)
}

/// What an import read past or left out, for the log.
#[derive(Default)]
struct Skipped {
    displacements: usize,
    connections: usize,
    cameras: usize,
    cordons: usize,
    hammer_groups: usize,
    bad_sides: usize,
    bad_brushes: usize,
    unnamed: usize,
}

pub struct Import {
    pub output: Output,
    pub log: Vec<String>,
}

/// Exactly `N` finite numbers, ignoring the brackets and parentheses around them.
fn numbers<const N: usize>(text: &str) -> Option<[f64; N]> {
    let parts = text.split(|c: char| c.is_whitespace() || "()[]".contains(c)).filter(|s| !s.is_empty());
    let parsed: Vec<f64> = parts.map(|s| s.parse().ok().filter(|v: &f64| v.is_finite())).collect::<Option<_>>()?;
    parsed.try_into().ok()
}

fn face(side: &Block) -> Option<Face> {
    let [a, b, c, d, e, f, g, h, i] = numbers(side.key("plane")?)?;
    let material = side.key("material").map(|m| m.trim().replace('\\', "/").to_ascii_lowercase()).filter(|m| !m.is_empty());
    let axis = |key| {
        let [x, y, z, offset, scale] = numbers(side.key(key)?)?;
        (scale != 0.0).then_some(TexAxis { axis: Vec3 { x, y, z }, offset, scale })
    };
    Some(Face {
        points: (Vec3 { x: a, y: b, z: c }, Vec3 { x: d, y: e, z: f }, Vec3 { x: g, y: h, z: i }),
        material: material.unwrap_or_else(|| NODRAW.into()),
        uv: axis("uaxis").zip(axis("vaxis")),
    })
}

/// The first visgroup the object is in.
fn visgroup<'a>(b: &Block<'a>) -> Option<&'a str> {
    b.blocks("editor").find_map(|e| e.key("visgroupid"))
}

/// The brushes of a world or entity block, hidden ones too, with the visgroup each is in.
fn solids<'a>(b: &Block<'a>, skipped: &mut Skipped) -> Vec<(Option<&'a str>, Brush)> {
    let hidden = b.blocks("hidden").flat_map(|h| h.blocks("solid"));
    let mut out = Vec::new();
    for solid in b.blocks("solid").chain(hidden) {
        let mut faces = Vec::new();
        for side in solid.blocks("side") {
            skipped.displacements += side.blocks("dispinfo").count();
            match face(side) {
                Some(f) => faces.push(f),
                None => skipped.bad_sides += 1,
            }
        }
        if faces.len() < 4 {
            skipped.bad_brushes += 1;
            continue;
        }
        out.push((visgroup(solid), Brush::Faces(faces)));
    }
    out
}

fn entity<'a>(b: &Block<'a>, skipped: &mut Skipped) -> Option<(Option<&'a str>, Entity)> {
    skipped.connections += b.blocks("connections").map(|c| c.keys.len()).sum::<usize>();
    let Some(classname) = b.key("classname").filter(|c| !c.is_empty()) else {
        skipped.unnamed += 1;
        return None;
    };
    let brushes = solids(b, skipped).into_iter().map(|(_, brush)| brush).collect();
    let keys = b.keys.iter().filter(|(k, _)| !k.eq_ignore_ascii_case("id") && !k.eq_ignore_ascii_case("classname"));
    let properties = keys.map(|(k, v)| (k.to_string(), v.to_string())).collect();
    Some((visgroup(b), Entity { classname: classname.into(), properties, brushes }))
}

/// Every visgroup's id and its name after those of the visgroups around it, as "Parent / Child".
fn visgroup_names<'a>(b: &Block<'a>, parent: &str, out: &mut Vec<(&'a str, String)>) {
    for v in b.blocks("visgroup") {
        let name = v.key("name").unwrap_or("Visgroup");
        let path = if parent.is_empty() { name.to_string() } else { format!("{parent} / {name}") };
        if let Some(id) = v.key("visgroupid") {
            out.push((id, path.clone()));
        }
        visgroup_names(v, &path, out);
    }
}

pub fn import(text: &str) -> Result<Import, String> {
    let blocks = parse(text)?;
    let mut skipped = Skipped::default();
    let mut names = Vec::new();
    let (mut world, mut entities, mut world_keys) = (Vec::new(), Vec::new(), Vec::new());
    for b in &blocks {
        match b.name.to_ascii_lowercase().as_str() {
            "visgroups" => visgroup_names(b, "", &mut names),
            "world" => {
                let keys = b.keys.iter().map(|(k, _)| *k).filter(|k| !["id", "classname", "mapversion"].iter().any(|s| k.eq_ignore_ascii_case(s)));
                world_keys.extend(keys);
                world.extend(solids(b, &mut skipped));
                skipped.hammer_groups += b.blocks("group").count();
            }
            "entity" => entities.extend(entity(b, &mut skipped)),
            "hidden" => entities.extend(b.blocks("entity").filter_map(|e| entity(e, &mut skipped))),
            "cameras" => skipped.cameras += b.blocks("camera").count(),
            "cordon" => skipped.cordons += 1,
            "cordons" => skipped.cordons += b.blocks("cordon").count(),
            _ => {}
        }
    }

    let mut groups: Vec<Group> = names.iter().map(|(_, name)| Group { name: name.clone(), brushes: Vec::new(), entities: Vec::new() }).collect();
    let index: HashMap<&str, usize> = names.iter().enumerate().rev().map(|(i, (id, _))| (*id, i)).collect();
    let (mut brushes, mut top) = (Vec::new(), Vec::new());
    let (world_count, entity_count) = (world.len(), entities.len());
    let brush_entities = entities.iter().filter(|(_, e)| !e.brushes.is_empty()).count();
    for (v, brush) in world {
        match v.and_then(|v| index.get(v)).and_then(|&i| groups.get_mut(i)) {
            Some(g) => g.brushes.push(brush),
            None => brushes.push(brush),
        }
    }
    for (v, entity) in entities {
        match v.and_then(|v| index.get(v)).and_then(|&i| groups.get_mut(i)) {
            Some(g) => g.entities.push(entity),
            None => top.push(entity),
        }
    }
    groups.retain(|g| !g.brushes.is_empty() || !g.entities.is_empty());

    let mut log = vec![format!("{world_count} world brushes, {entity_count} entities ({brush_entities} with brushes), {} visgroups as groups", groups.len())];
    let s = &skipped;
    let mut lost: Vec<String> = [
        (s.displacements, "displacements (their brushes come in flat)"),
        (s.connections, "entity outputs"),
        (s.cameras, "cameras"),
        (s.cordons, "cordons"),
        (s.hammer_groups, "Hammer groups (their contents come in ungrouped)"),
        (s.bad_brushes, "brushes with fewer than four readable sides"),
        (s.bad_sides, "unreadable sides"),
        (s.unnamed, "entities with no classname"),
    ]
    .into_iter()
    .filter(|(n, _)| *n > 0)
    .map(|(n, what)| format!("{n} {what}"))
    .collect();
    if !world_keys.is_empty() {
        lost.push(format!("worldspawn keys {}", world_keys.join(", ")));
    }
    if !lost.is_empty() {
        log.push(format!("skipped: {}", lost.join("; ")));
    }
    Ok(Import { output: Output { brushes, entities: top, groups, materials: Vec::new() }, log })
}

fn line(out: &mut String, depth: usize, text: &str) {
    out.extend(std::iter::repeat_n('\t', depth));
    out.push_str(text);
    out.push('\n');
}

fn pair(out: &mut String, depth: usize, key: &str, value: &str) -> Result<(), String> {
    if key.contains(['"', '\n', '\r']) || value.contains(['"', '\n', '\r']) {
        return Err(format!("`{key}` `{value}` has a quote or line break, which a .vmf cannot hold"));
    }
    line(out, depth, &format!("\"{key}\" \"{value}\""));
    Ok(())
}

fn open(out: &mut String, depth: usize, name: &str) {
    line(out, depth, name);
    line(out, depth, "{");
}

/// The shortest text that reads back as exactly this number.
fn number(v: f64) -> Result<String, String> {
    if v.is_finite() { Ok(format!("{}", v + 0.0)) } else { Err("a brush has a coordinate or texture value that is not a number".into()) }
}

fn point(p: &Vec3) -> Result<String, String> {
    Ok(format!("({} {} {})", number(p.x)?, number(p.y)?, number(p.z)?))
}

fn axis(a: &TexAxis) -> Result<String, String> {
    Ok(format!("[{} {} {} {}] {}", number(a.axis.x)?, number(a.axis.y)?, number(a.axis.z)?, number(a.offset)?, number(a.scale)?))
}

fn editor(out: &mut String, depth: usize, visgroup: Option<usize>) -> Result<(), String> {
    let Some(v) = visgroup else { return Ok(()) };
    open(out, depth, "editor");
    pair(out, depth + 1, "color", "220 220 220")?;
    pair(out, depth + 1, "visgroupid", &v.to_string())?;
    pair(out, depth + 1, "visgroupshown", "1")?;
    pair(out, depth + 1, "visgroupautoshown", "1")?;
    line(out, depth, "}");
    Ok(())
}

/// Ids count up through solids, sides and entities in the order they are written, so the same map gets the same ids.
struct Writer {
    out: String,
    id: u64,
    skipped: usize,
}

impl Writer {
    fn id(&mut self, depth: usize) -> Result<(), String> {
        self.id += 1;
        pair(&mut self.out, depth, "id", &self.id.to_string())
    }

    fn solid(&mut self, s: &Solid, visgroup: Option<usize>) -> Result<(), String> {
        if s.faces.len() < 4 {
            self.skipped += 1;
            return Ok(());
        }
        open(&mut self.out, 1, "solid");
        self.id(2)?;
        for f in &s.faces {
            let (u, v) = f.uv.as_ref().ok_or("a face has no texture axes")?;
            open(&mut self.out, 2, "side");
            self.id(3)?;
            let (p0, p1, p2) = &f.points;
            pair(&mut self.out, 3, "plane", &format!("{} {} {}", point(p0)?, point(p1)?, point(p2)?))?;
            pair(&mut self.out, 3, "material", if f.material.is_empty() { NODRAW } else { &f.material })?;
            pair(&mut self.out, 3, "uaxis", &axis(u)?)?;
            pair(&mut self.out, 3, "vaxis", &axis(v)?)?;
            pair(&mut self.out, 3, "rotation", "0")?;
            pair(&mut self.out, 3, "lightmapscale", "16")?;
            pair(&mut self.out, 3, "smoothing_groups", "0")?;
            line(&mut self.out, 2, "}");
        }
        editor(&mut self.out, 2, visgroup)?;
        line(&mut self.out, 1, "}");
        Ok(())
    }
}

/// Layers and groups as nested visgroups.
fn visgroups(out: &mut String, map: &MapData, children: &[Vec<usize>], ids: &[Option<usize>], list: &[usize], depth: usize) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err(format!("layers and groups nest deeper than {MAX_DEPTH}"));
    }
    for &i in list {
        let (Some(MapNodeKind::Layer(name) | MapNodeKind::Group(name)), Some(Some(id))) = (map.nodes.get(i).map(|n| &n.kind), ids.get(i)) else { continue };
        open(out, depth, "visgroup");
        pair(out, depth + 1, "name", name)?;
        pair(out, depth + 1, "visgroupid", &id.to_string())?;
        pair(out, depth + 1, "color", "220 220 220")?;
        visgroups(out, map, children, ids, children.get(i).map_or(&[][..], Vec::as_slice), depth + 1)?;
        line(out, depth, "}");
    }
    Ok(())
}

pub struct Export {
    pub text: String,
    pub warnings: Vec<String>,
}

pub fn export(map: &MapData) -> Result<Export, String> {
    let count = map.nodes.len();
    let (mut parents, mut children, mut ids) = (Vec::with_capacity(count), vec![Vec::new(); count], vec![None; count]);
    let mut top = Vec::new();
    let mut next = 0;
    for (i, node) in map.nodes.iter().enumerate() {
        let parent = node
            .parent
            .map(|p| usize::try_from(p).ok().filter(|&p| p < i).ok_or_else(|| format!("node {i} has a parent that does not come before it")))
            .transpose()?;
        match (parent.and_then(|p| map.nodes.get(p)).map(|n| &n.kind), &node.kind) {
            (Some(MapNodeKind::Brush(_)), _) => return Err(format!("node {i} sits inside a brush")),
            (Some(MapNodeKind::Entity(_)), MapNodeKind::Brush(_)) => {}
            (Some(MapNodeKind::Entity(_)), _) => return Err(format!("node {i} sits inside an entity, where only brushes can go")),
            (_, MapNodeKind::Layer(_) | MapNodeKind::Group(_)) => {
                next += 1;
                ids[i] = Some(next);
            }
            _ => {}
        }
        match parent.and_then(|p| children.get_mut(p)) {
            Some(list) => list.push(i),
            None => top.push(i),
        }
        parents.push(parent);
    }
    let visgroup_of = |i: usize| parents.get(i).copied().flatten().and_then(|p| ids.get(p).copied().flatten());

    let mut w = Writer { out: String::new(), id: 1, skipped: 0 };
    let out = &mut w.out;
    open(out, 0, "versioninfo");
    pair(out, 1, "editorversion", "400")?;
    let version = map.world.iter().find(|(k, _)| k == "mapversion").map_or("1", |(_, v)| v.as_str());
    pair(out, 1, "mapversion", version)?;
    pair(out, 1, "formatversion", "100")?;
    pair(out, 1, "prefab", "0")?;
    line(out, 0, "}");
    open(out, 0, "visgroups");
    visgroups(out, map, &children, &ids, &top, 1)?;
    line(out, 0, "}");
    open(out, 0, "world");
    pair(out, 1, "id", "1")?;
    pair(out, 1, "mapversion", version)?;
    pair(out, 1, "classname", "worldspawn")?;
    for (k, v) in map.world.iter().filter(|(k, _)| !["id", "classname", "mapversion"].contains(&k.as_str())) {
        pair(out, 1, k, v)?;
    }
    let in_entity = |i: usize| parents.get(i).copied().flatten().and_then(|p| map.nodes.get(p)).is_some_and(|n| matches!(n.kind, MapNodeKind::Entity(_)));
    for (i, node) in map.nodes.iter().enumerate() {
        if let MapNodeKind::Brush(s) = &node.kind
            && !in_entity(i)
        {
            w.solid(s, visgroup_of(i))?;
        }
    }
    line(&mut w.out, 0, "}");

    for (i, node) in map.nodes.iter().enumerate() {
        let MapNodeKind::Entity(e) = &node.kind else { continue };
        if e.classname.is_empty() || e.classname.eq_ignore_ascii_case("worldspawn") {
            return Err(format!("node {i} is an entity named `{}`, which a .vmf cannot hold besides the world", e.classname));
        }
        open(&mut w.out, 0, "entity");
        w.id(1)?;
        pair(&mut w.out, 1, "classname", &e.classname)?;
        for (k, v) in e.properties.iter().filter(|(k, _)| !k.eq_ignore_ascii_case("id") && !k.eq_ignore_ascii_case("classname")) {
            pair(&mut w.out, 1, k, v)?;
        }
        for &b in children.get(i).map_or(&[][..], Vec::as_slice) {
            if let Some(MapNodeKind::Brush(s)) = map.nodes.get(b).map(|n| &n.kind) {
                w.solid(s, None)?;
            }
        }
        editor(&mut w.out, 1, visgroup_of(i))?;
        line(&mut w.out, 0, "}");
    }
    open(&mut w.out, 0, "cameras");
    pair(&mut w.out, 1, "activecamera", "-1")?;
    line(&mut w.out, 0, "}");

    let warnings = if w.skipped > 0 { vec![format!("left out {} brushes with fewer than four faces", w.skipped)] } else { Vec::new() };
    Ok(Export { text: w.out, warnings })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapcraft::plugin::types::{MapEntity, MapNode};

    const VMF: &str = "versioninfo\n{\n\t\"editorversion\" \"400\"\n}\nvisgroups\n{\n\tvisgroup\n\t{\n\t\t\"name\" \"Course\"\n\t\t\"visgroupid\" \"4\"\n\
        \t\tvisgroup\n\t\t{\n\t\t\t\"name\" \"Ramps\"\n\t\t\t\"visgroupid\" \"9\"\n\t\t}\n\t}\n}\nworld\n{\n\t\"id\" \"1\"\n\t\"classname\" \"worldspawn\"\n\
        \t\"skyname\" \"sky_day01_01\"\n\tSOLID\n\t{\n\t\t\"id\" \"2\"\nSIDES\n\t\teditor\n\t\t{\n\t\t\t\"visgroupid\" \"9\"\n\t\t}\n\t}\n\
        \thidden\n\t{\n\t\tsolid\n\t\t{\nSIDES\n\t\t}\n\t}\n\tgroup\n\t{\n\t\t\"id\" \"7\"\n\t}\n}\n\
        entity\n{\n\t\"id\" \"20\"\n\t\"classname\" \"trigger_multiple\"\n\t\"message\" \"{\"\n\tconnections\n\t{\n\t\t\"OnStartTouch\" \"a,b,,0,-1\"\n\t}\n\
        \tsolid\n\t{\nSIDES\n\t}\n\teditor\n\t{\n\t\t\"visgroupid\" \"4\"\n\t}\n}\n\
        hidden\n{\n\tentity\n\t{\n\t\t\"classname\" \"info_player_counterterrorist\"\n\t\t\"origin\" \"0 0 64\"\n\t}\n}\n\
        cameras\n{\n\t\"activecamera\" \"0\"\n\tcamera\n\t{\n\t\t\"position\" \"[0 0 0]\"\n\t}\n}\n";

    fn cube(x: f64) -> String {
        let b = x + 64.0;
        [
            format!("({x} 0 64) ({x} 64 64) ({b} 64 64)"),
            format!("({x} 64 0) ({x} 0 0) ({b} 0 0)"),
            format!("({x} 0 64) ({x} 0 0) ({x} 64 0)"),
            format!("({b} 64 64) ({b} 64 0) ({b} 0 0)"),
            format!("({x} 64 64) ({x} 64 0) ({b} 64 0)"),
            format!("({b} 0 64) ({b} 0 0) ({x} 0 0)"),
        ]
        .iter()
        .map(|plane| {
            format!("side\n{{\n\"plane\" \"{plane}\"\n\"material\" \"DEV\\DEV_MeasureGeneric01B\"\n\"uaxis\" \"[1 0 0 16] 0.25\"\n\"vaxis\" \"[0 -1 0 -8] 0.5\"\n}}\n")
        })
        .collect()
    }

    fn vmf() -> String {
        let mut text = VMF.to_string();
        for x in [0.0, 128.0, 256.0] {
            text = text.replacen("SIDES", &cube(x), 1);
        }
        text
    }

    /// What the editor holds after importing `output` into a layer, as an exporter gets it.
    fn editor_map(output: &Output, world: Vec<(String, String)>) -> MapData {
        let mut nodes = vec![MapNode { parent: None, kind: MapNodeKind::Layer("import".into()) }];
        let solid = |b: &Brush| {
            let Brush::Faces(faces) = b else { panic!("hull") };
            MapNodeKind::Brush(Solid { faces: faces.clone(), vertices: Vec::new() })
        };
        let add = |nodes: &mut Vec<MapNode>, parent: u32, brushes: &[Brush], entities: &[Entity]| {
            nodes.extend(brushes.iter().map(|b| MapNode { parent: Some(parent), kind: solid(b) }));
            for e in entities {
                let at = nodes.len() as u32;
                nodes.push(MapNode {
                    parent: Some(parent),
                    kind: MapNodeKind::Entity(MapEntity { classname: e.classname.clone(), properties: e.properties.clone() }),
                });
                nodes.extend(e.brushes.iter().map(|b| MapNode { parent: Some(at), kind: solid(b) }));
            }
        };
        add(&mut nodes, 0, &output.brushes, &output.entities);
        for g in &output.groups {
            let at = nodes.len() as u32;
            nodes.push(MapNode { parent: Some(0), kind: MapNodeKind::Group(g.name.clone()) });
            add(&mut nodes, at, &g.brushes, &g.entities);
        }
        MapData { name: "test".into(), world, nodes }
    }

    /// World brushes, entities, brush entity brushes and group names.
    fn counts(out: &Output) -> (usize, usize, usize, Vec<String>) {
        let entities = || out.entities.iter().chain(out.groups.iter().flat_map(|g| &g.entities));
        let brushes = out.brushes.len() + out.groups.iter().map(|g| g.brushes.len()).sum::<usize>();
        (brushes, entities().count(), entities().map(|e| e.brushes.len()).sum(), out.groups.iter().map(|g| g.name.clone()).collect())
    }

    #[test]
    fn reads_world_entities_and_visgroups() {
        let import = import(&vmf()).unwrap();
        let out = &import.output;
        assert_eq!(out.brushes.len(), 1, "the hidden brush is in no visgroup");
        let groups: Vec<(&str, usize, usize)> = out.groups.iter().map(|g| (g.name.as_str(), g.brushes.len(), g.entities.len())).collect();
        assert_eq!(groups, [("Course", 0, 1), ("Course / Ramps", 1, 0)]);
        let trigger = &out.groups[0].entities[0];
        assert_eq!(trigger.properties, [("message".to_string(), "{".to_string())]);
        assert_eq!(trigger.brushes.len(), 1);
        assert_eq!(out.entities[0].classname, "info_player_counterterrorist");
        let Brush::Faces(faces) = &out.brushes[0] else { panic!("hull") };
        let (u, v) = faces[0].uv.as_ref().unwrap();
        assert_eq!(faces[0].material, "dev/dev_measuregeneric01b");
        assert_eq!((u.axis.x, u.offset, u.scale, v.axis.y, v.offset, v.scale), (1.0, 16.0, 0.25, -1.0, -8.0, 0.5));
        assert_eq!(import.log[0], "2 world brushes, 2 entities (1 with brushes), 2 visgroups as groups");
        assert_eq!(import.log[1], "skipped: 1 entity outputs; 1 cameras; 1 Hammer groups (their contents come in ungrouped); worldspawn keys skyname");
    }

    #[test]
    fn round_trips_through_export() {
        let first = import(&vmf()).unwrap().output;
        let export = export(&editor_map(&first, vec![("skyname".into(), "sky_day01_01".into())])).unwrap();
        assert!(export.text.contains("\"skyname\" \"sky_day01_01\"") && export.text.contains("\"uaxis\" \"[1 0 0 16] 0.25\""), "{}", export.text);
        let again = import(&export.text).unwrap();
        let names = vec!["import".to_string(), "import / Course".into(), "import / Course / Ramps".into()];
        assert_eq!(counts(&again.output), (2, 2, 1, names));
        let ids: Vec<&str> = export.text.lines().filter_map(|l| l.trim().strip_prefix("\"id\" ")).collect();
        let unique: std::collections::BTreeSet<&&str> = ids.iter().collect();
        assert_eq!(ids.len(), unique.len(), "ids are unique");
        assert_eq!(export.text, super::export(&editor_map(&first, vec![("skyname".into(), "sky_day01_01".into())])).unwrap().text, "ids are stable");
    }

    #[test]
    fn broken_files_are_errors() {
        for text in ["world\n{\n\"id\" \"1\"\n", "world\n{\n\"id\"\n}\n", "\"key\" \"value\"", "world {\n{\n}\n}", &"a {\n".repeat(100)] {
            assert!(import(text).is_err(), "{text}");
        }
        let empty = import("").unwrap();
        assert!(empty.output.brushes.is_empty() && empty.log.len() == 1);
        let bad = MapData { name: "x".into(), world: Vec::new(), nodes: vec![MapNode { parent: Some(3), kind: MapNodeKind::Group("g".into()) }] };
        assert!(export(&bad).is_err());
        let quote = MapData { name: "x".into(), world: vec![("message".into(), "say \"hi\"".into())], nodes: Vec::new() };
        assert!(export(&quote).is_err());
    }

    const SURF: &str = "/mnt/Storage/Gamedev/SurfsUp/SurfMaps/Decomp/surf_activation_d.vmf";

    #[test]
    fn surf_activation_round_trips() {
        let Ok(text) = std::fs::read_to_string(SURF) else {
            eprintln!("skipped: {SURF} is missing");
            return;
        };
        let start = std::time::Instant::now();
        let first = import(&text).unwrap();
        eprintln!("imported in {:?}: {:?}", start.elapsed(), first.log);
        let export = export(&editor_map(&first.output, Vec::new())).unwrap();
        let again = import(&export.text).unwrap();
        let (a, b) = (counts(&first.output), counts(&again.output));
        assert_eq!((a.0, a.1, a.2), (b.0, b.1, b.2));
        assert_eq!((a.0, a.1), (3373, 4411));
    }
}
