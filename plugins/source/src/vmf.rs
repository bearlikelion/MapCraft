//! Reads and writes Valve Hammer `.vmf` maps: world brushes and keys, entities with their brushes
//! and outputs, and visgroups and Hammer groups as groups.

use std::collections::HashMap;

use valve::Token;

use crate::mapcraft::plugin::types::{Brush, Connection, Displacement, Entity, Face, Group, MapData, MapNodeKind, Output, Solid, TexAxis, Vec3};

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
    bad_displacements: usize,
    sideways: usize,
    bad_connections: usize,
    cameras: usize,
    cordons: usize,
    bad_sides: usize,
    bad_brushes: usize,
    unnamed: usize,
}

/// The visgroup and Hammer group an object is in, from its `editor` block.
type Member<'a> = (Option<&'a str>, Option<&'a str>);

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

type V = [f64; 3];

fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: V, b: V) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn scaled(a: V, s: f64) -> V {
    a.map(|v| v * s)
}

/// The outward unit normal of a side's plane points, or `None` when they are in a line.
fn plane_normal(p0: V, p1: V, p2: V) -> Option<V> {
    let (a, b) = (sub(p0, p1), sub(p2, p1));
    let n = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let len = dot(n, n).sqrt();
    (len.is_finite() && len > 1e-9).then(|| scaled(n, 1.0 / len))
}

/// The `n` rows of `width * n` numbers in a `dispinfo` block, all in one list; a missing block is `default`.
fn rows(info: &Block, name: &str, n: usize, width: usize, default: Option<f64>) -> Option<Vec<f64>> {
    let Some(block) = info.blocks(name).next() else { return default.map(|d| vec![d; n * n * width]) };
    let mut out = Vec::with_capacity(n * n * width);
    for r in 0..n {
        let row: Vec<f64> = block.key(&format!("row{r}"))?.split_whitespace().map(|s| s.parse().ok().filter(|v: &f64| v.is_finite())).collect::<Option<_>>()?;
        if row.len() != n * width {
            return None;
        }
        out.extend(row);
    }
    Some(out)
}

/// A side's `dispinfo` as heights along the face normal, and whether any vertex was offset sideways.
fn dispinfo(info: &Block, normal: V) -> Option<(Displacement, bool)> {
    let power: u32 = info.key("power")?.trim().parse().ok().filter(|p| (2..=4).contains(p))?;
    let n = (1usize << power) + 1;
    let [x, y, z] = numbers(info.key("startposition")?)?;
    let elevation = match info.key("elevation") {
        Some(e) => e.trim().parse().ok().filter(|e: &f64| e.is_finite())?,
        None => 0.0,
    };
    let normals = rows(info, "normals", n, 3, None)?;
    let distances = rows(info, "distances", n, 1, None)?;
    let offsets = rows(info, "offsets", n, 3, Some(0.0))?;
    let alphas = rows(info, "alphas", n, 1, Some(0.0))?;
    let mut sideways = false;
    let heights = normals
        .as_chunks::<3>()
        .0
        .iter()
        .zip(&distances)
        .zip(offsets.as_chunks::<3>().0)
        .map(|((dir, d), off)| {
            let moved = [dir[0] * d + off[0], dir[1] * d + off[1], dir[2] * d + off[2]];
            let along = dot(moved, normal);
            let side = sub(moved, scaled(normal, along));
            sideways |= dot(side, side) > 0.01 * 0.01;
            along + elevation
        })
        .collect();
    let start = Vec3 { x, y, z };
    let blend = alphas.iter().map(|a| a / 255.0).collect();
    Some((Displacement { power, corners: (start, start, start, start), heights, blend, blend_material: None }, sideways))
}

fn face(side: &Block, skipped: &mut Skipped) -> Option<Face> {
    let [a, b, c, d, e, f, g, h, i] = numbers(side.key("plane")?)?;
    let material = side.key("material").map(|m| m.trim().replace('\\', "/").to_ascii_lowercase()).filter(|m| !m.is_empty());
    let axis = |key| {
        let [x, y, z, offset, scale] = numbers(side.key(key)?)?;
        (scale != 0.0).then_some(TexAxis { axis: Vec3 { x, y, z }, offset, scale })
    };
    let disp = side.blocks("dispinfo").next().and_then(|info| {
        let read = plane_normal([a, b, c], [d, e, f], [g, h, i]).and_then(|normal| dispinfo(info, normal));
        skipped.bad_displacements += usize::from(read.is_none());
        skipped.sideways += usize::from(read.as_ref().is_some_and(|r| r.1));
        read.map(|r| r.0)
    });
    let attrs = [("lightmapscale", "16"), ("smoothing_groups", "0")]
        .into_iter()
        .filter_map(|(k, default)| Some((k.to_string(), side.key(k)?.trim().to_string())).filter(|(_, v)| v != default))
        .collect();
    Some(Face {
        points: (Vec3 { x: a, y: b, z: c }, Vec3 { x: d, y: e, z: f }, Vec3 { x: g, y: h, z: i }),
        material: material.unwrap_or_else(|| NODRAW.into()),
        uv: axis("uaxis").zip(axis("vaxis")),
        attrs,
        disp,
    })
}

/// The first visgroup the object is in, and its Hammer group.
fn member<'a>(b: &Block<'a>) -> Member<'a> {
    (b.blocks("editor").find_map(|e| e.key("visgroupid")), b.blocks("editor").find_map(|e| e.key("groupid")))
}

/// An output's `target,input,param,delay,times`, which newer Hammers separate with escape characters instead.
fn connection(output: &str, value: &str) -> Option<Connection> {
    let sep = if value.contains('\u{1b}') { "\u{1b}" } else { "," };
    let parts: Vec<&str> = value.split(sep).collect();
    let [target, input, param @ .., delay, times] = parts.as_slice() else { return None };
    Some(Connection {
        output: output.into(),
        target: target.to_string(),
        input: input.to_string(),
        param: param.join(sep),
        delay: delay.trim().parse().ok().filter(|d: &f64| d.is_finite())?,
        times: times.trim().parse().ok()?,
    })
}

/// Each Hammer group's id with the id of the outermost group around it, so nested groups stay together.
fn hammer_groups<'a>(world: &Block<'a>) -> Vec<(&'a str, &'a str)> {
    let parent: HashMap<&str, &str> = world.blocks("group").filter_map(|g| Some((g.key("id")?, member(g).1?))).collect();
    world
        .blocks("group")
        .filter_map(|g| g.key("id"))
        .map(|id| {
            let mut top = id;
            for _ in 0..MAX_DEPTH {
                match parent.get(top) {
                    Some(&p) if p != top => top = p,
                    _ => break,
                }
            }
            (id, top)
        })
        .collect()
}

/// The brushes of a world or entity block, hidden ones too, with the groups each is in.
fn solids<'a>(b: &Block<'a>, skipped: &mut Skipped) -> Vec<(Member<'a>, Brush)> {
    let hidden = b.blocks("hidden").flat_map(|h| h.blocks("solid"));
    let mut out = Vec::new();
    for solid in b.blocks("solid").chain(hidden) {
        let mut faces = Vec::new();
        for side in solid.blocks("side") {
            match face(side, skipped) {
                Some(f) => faces.push(f),
                None => skipped.bad_sides += 1,
            }
        }
        if faces.len() < 4 {
            skipped.bad_brushes += 1;
            continue;
        }
        out.push((member(solid), Brush::Faces(faces)));
    }
    out
}

fn entity<'a>(b: &Block<'a>, skipped: &mut Skipped) -> Option<(Member<'a>, Entity)> {
    let Some(classname) = b.key("classname").filter(|c| !c.is_empty()) else {
        skipped.unnamed += 1;
        return None;
    };
    let mut connections = Vec::new();
    for (output, value) in b.blocks("connections").flat_map(|c| c.keys.iter()) {
        match connection(output, value) {
            Some(c) => connections.push(c),
            None => skipped.bad_connections += 1,
        }
    }
    let brushes = solids(b, skipped).into_iter().map(|(_, brush)| brush).collect();
    let keys = b.keys.iter().filter(|(k, _)| !k.eq_ignore_ascii_case("id") && !k.eq_ignore_ascii_case("classname"));
    let properties = keys.map(|(k, v)| (k.to_string(), v.to_string())).collect();
    Some((member(b), Entity { classname: classname.into(), properties, connections, brushes }))
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
    let (mut world, mut entities, mut world_keys, mut nested) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for b in &blocks {
        match b.name.to_ascii_lowercase().as_str() {
            "visgroups" => visgroup_names(b, "", &mut names),
            "world" => {
                let keys = b.keys.iter().filter(|(k, _)| !["id", "classname", "mapversion"].iter().any(|s| k.eq_ignore_ascii_case(s)));
                world_keys.extend(keys.map(|(k, v)| (k.to_string(), v.to_string())));
                world.extend(solids(b, &mut skipped));
                nested.extend(hammer_groups(b));
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
    let tops: HashMap<&str, &str> = nested.iter().copied().collect();
    let mut hammer: HashMap<&str, usize> = HashMap::new();
    for (_, top) in &nested {
        hammer.entry(*top).or_insert_with(|| {
            groups.push(Group { name: format!("Group {top}"), brushes: Vec::new(), entities: Vec::new() });
            groups.len() - 1
        });
    }
    // A Hammer group keeps its objects together, so it wins over their visgroup.
    let slot = |(v, g): Member<'_>| g.and_then(|g| tops.get(g)).and_then(|t| hammer.get(t)).or_else(|| v.and_then(|v| index.get(v))).copied();
    let (mut brushes, mut top) = (Vec::new(), Vec::new());
    let (world_count, entity_count) = (world.len(), entities.len());
    let brush_entities = entities.iter().filter(|(_, e)| !e.brushes.is_empty()).count();
    let outputs: usize = entities.iter().map(|(_, e)| e.connections.len()).sum();
    for (m, brush) in world {
        match slot(m).and_then(|i| groups.get_mut(i)) {
            Some(g) => g.brushes.push(brush),
            None => brushes.push(brush),
        }
    }
    for (m, entity) in entities {
        match slot(m).and_then(|i| groups.get_mut(i)) {
            Some(g) => g.entities.push(entity),
            None => top.push(entity),
        }
    }
    groups.retain(|g| !g.brushes.is_empty() || !g.entities.is_empty());
    if !world_keys.is_empty() {
        top.push(Entity { classname: "worldspawn".into(), properties: world_keys, connections: Vec::new(), brushes: Vec::new() });
    }

    let mut log =
        vec![format!("{world_count} world brushes, {entity_count} entities ({brush_entities} with brushes) with {outputs} outputs, {} groups", groups.len())];
    let s = &skipped;
    let lost: Vec<String> = [
        (s.bad_connections, "unreadable entity outputs"),
        (s.cameras, "cameras"),
        (s.cordons, "cordons"),
        (s.bad_brushes, "brushes with fewer than four readable sides"),
        (s.bad_sides, "unreadable sides"),
        (s.unnamed, "entities with no classname"),
    ]
    .into_iter()
    .filter(|(n, _)| *n > 0)
    .map(|(n, what)| format!("{n} {what}"))
    .collect();
    if !lost.is_empty() {
        log.push(format!("skipped: {}", lost.join("; ")));
    }
    if s.bad_displacements > 0 {
        log.push(format!("{} unreadable displacements came in flat", s.bad_displacements));
    }
    if s.sideways > 0 {
        log.push(format!("{} displacements had sideways offsets, which MapCraft drops", s.sideways));
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

/// An output as Hammer writes it, with escape characters between the parts when a part holds a comma.
fn connection_text(c: &Connection) -> Result<String, String> {
    if !c.delay.is_finite() {
        return Err(format!("output `{}` has a delay that is not a number", c.output));
    }
    let sep = if [&c.target, &c.input, &c.param].iter().any(|p| p.contains(',')) { "\u{1b}" } else { "," };
    Ok([c.target.clone(), c.input.clone(), c.param.clone(), format!("{}", c.delay + 0.0), c.times.to_string()].join(sep))
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
#[derive(Default)]
struct Writer {
    out: String,
    id: u64,
    skipped: usize,
    power_one: usize,
    on_entities: usize,
    blended: usize,
    first_blend: Option<String>,
    anti_tiled: usize,
}

/// The face attribute `key` as an integer, else `default`.
fn int_attr(f: &Face, key: &str, default: i64) -> String {
    f.attrs.iter().find(|(k, _)| k == key).and_then(|(_, v)| v.trim().parse().ok()).unwrap_or(default).to_string()
}

/// A `dispinfo` block with a row of text per grid row, `value` giving each vertex's part.
fn disp_rows(out: &mut String, name: &str, n: usize, value: impl Fn(usize) -> Result<String, String>) -> Result<(), String> {
    open(out, 4, name);
    for r in 0..n {
        let row: Vec<String> = (0..n).map(|c| value(r * n + c)).collect::<Result<_, _>>()?;
        pair(out, 5, &format!("row{r}"), &row.join(" "))?;
    }
    line(out, 4, "}");
    Ok(())
}

impl Writer {
    fn id(&mut self, depth: usize) -> Result<(), String> {
        self.id += 1;
        pair(&mut self.out, depth, "id", &self.id.to_string())
    }

    /// Writes the side's displacement, or counts why it stays flat.
    fn dispinfo(&mut self, f: &Face, d: &Displacement, world: bool) -> Result<(), String> {
        let n = (1usize << d.power.min(4)) + 1;
        if !world {
            self.on_entities += 1;
            return Ok(());
        }
        if !(2..=4).contains(&d.power) || d.heights.len() != n * n {
            self.power_one += usize::from(d.power == 1);
            return Ok(());
        }
        let (p0, p1, p2) = &f.points;
        let v = |p: &Vec3| [p.x, p.y, p.z];
        let normal = plane_normal(v(p0), v(p1), v(p2)).ok_or("a displaced face has no plane")?;
        if let Some(m) = &d.blend_material {
            self.blended += 1;
            self.first_blend.get_or_insert_with(|| m.clone());
        }
        let vector = |a: V| -> Result<String, String> { Ok(format!("{} {} {}", number(a[0])?, number(a[1])?, number(a[2])?)) };
        let height = |k: usize| d.heights.get(k).copied().unwrap_or(0.0);
        let out = &mut self.out;
        open(out, 3, "dispinfo");
        pair(out, 4, "power", &d.power.to_string())?;
        let s = &d.corners.0;
        pair(out, 4, "startposition", &format!("[{} {} {}]", number(s.x)?, number(s.y)?, number(s.z)?))?;
        pair(out, 4, "flags", "0")?;
        pair(out, 4, "elevation", "0")?;
        pair(out, 4, "subdiv", "0")?;
        disp_rows(out, "normals", n, |k| vector(if height(k) < 0.0 { scaled(normal, -1.0) } else { normal }))?;
        disp_rows(out, "distances", n, |k| number(height(k).abs()))?;
        disp_rows(out, "offsets", n, |_| Ok("0 0 0".into()))?;
        disp_rows(out, "offset_normals", n, |_| vector(normal))?;
        disp_rows(out, "alphas", n, |k| {
            let w = d.blend.get(k).copied().filter(|w| w.is_finite()).unwrap_or(0.0).clamp(0.0, 1.0);
            Ok(format!("{}", (w * 255.0).round() as u8))
        })?;
        open(out, 4, "triangle_tags");
        for r in 0..n - 1 {
            pair(out, 5, &format!("row{r}"), &vec!["0"; 2 * (n - 1)].join(" "))?;
        }
        line(out, 4, "}");
        open(out, 4, "allowed_verts");
        pair(out, 5, "10", &["-1"; 10].join(" "))?;
        line(out, 4, "}");
        line(out, 3, "}");
        Ok(())
    }

    fn solid(&mut self, s: &Solid, visgroup: Option<usize>, world: bool) -> Result<(), String> {
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
            pair(&mut self.out, 3, "lightmapscale", &int_attr(f, "lightmapscale", 16))?;
            pair(&mut self.out, 3, "smoothing_groups", &int_attr(f, "smoothing_groups", 0))?;
            let anti_tile = f.attrs.iter().find(|(k, _)| k == "anti_tile").map(|(_, v)| v.trim().to_ascii_lowercase());
            self.anti_tiled += usize::from(anti_tile.is_some_and(|m| m == "macro" || m == "hex"));
            if let Some(d) = &f.disp {
                self.dispinfo(f, d, world)?;
            }
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

    let mut w = Writer { id: 1, ..Writer::default() };
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
            w.solid(s, visgroup_of(i), true)?;
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
        if !e.connections.is_empty() {
            open(&mut w.out, 1, "connections");
            for c in &e.connections {
                pair(&mut w.out, 2, &c.output, &connection_text(c)?)?;
            }
            line(&mut w.out, 1, "}");
        }
        for &b in children.get(i).map_or(&[][..], Vec::as_slice) {
            if let Some(MapNodeKind::Brush(s)) = map.nodes.get(b).map(|n| &n.kind) {
                w.solid(s, None, false)?;
            }
        }
        editor(&mut w.out, 1, visgroup_of(i))?;
        line(&mut w.out, 0, "}");
    }
    open(&mut w.out, 0, "cameras");
    pair(&mut w.out, 1, "activecamera", "-1")?;
    line(&mut w.out, 0, "}");

    let blend = w.first_blend.as_deref().unwrap_or_default();
    let warnings = [
        (w.skipped, format!("left out {} brushes with fewer than four faces", w.skipped)),
        (w.power_one, format!("{} displacements were power 1, which Source does not allow, and were written flat", w.power_one)),
        (w.on_entities, format!("{} displacements on brush entities were written flat; Source allows them only on world brushes", w.on_entities)),
        (
            w.blended,
            format!(
                "{} displaced faces blend {blend} through MapCraft's second layer, which a VMF cannot hold; their alphas are kept, so give them a WorldVertexTransition material that names both textures",
                w.blended
            ),
        ),
        (w.anti_tiled, format!("{} faces use anti-tiling, which Source cannot show; they export plain", w.anti_tiled)),
    ]
    .into_iter()
    .filter(|(n, _)| *n > 0)
    .map(|(_, text)| text)
    .collect();
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

    /// What the editor holds after importing `output` into a layer, as an exporter gets it: a
    /// `worldspawn` entity's keys go to the world.
    fn editor_map(output: &Output) -> MapData {
        let mut nodes = vec![MapNode { parent: None, kind: MapNodeKind::Layer("import".into()) }];
        let solid = |b: &Brush| {
            let Brush::Faces(faces) = b else { panic!("hull") };
            MapNodeKind::Brush(Solid { faces: faces.clone(), vertices: Vec::new() })
        };
        let add = |nodes: &mut Vec<MapNode>, parent: u32, brushes: &[Brush], entities: &[Entity]| {
            nodes.extend(brushes.iter().map(|b| MapNode { parent: Some(parent), kind: solid(b) }));
            for e in entities.iter().filter(|e| e.classname != "worldspawn") {
                let at = nodes.len() as u32;
                let (classname, properties, connections) = (e.classname.clone(), e.properties.clone(), e.connections.clone());
                nodes.push(MapNode { parent: Some(parent), kind: MapNodeKind::Entity(MapEntity { classname, properties, connections }) });
                nodes.extend(e.brushes.iter().map(|b| MapNode { parent: Some(at), kind: solid(b) }));
            }
        };
        add(&mut nodes, 0, &output.brushes, &output.entities);
        for g in &output.groups {
            let at = nodes.len() as u32;
            nodes.push(MapNode { parent: Some(0), kind: MapNodeKind::Group(g.name.clone()) });
            add(&mut nodes, at, &g.brushes, &g.entities);
        }
        let world = output.entities.iter().filter(|e| e.classname == "worldspawn").flat_map(|e| e.properties.clone()).collect();
        MapData { name: "test".into(), world, nodes }
    }

    /// Every entity but the world.
    fn all_entities(out: &Output) -> Vec<&Entity> {
        out.entities.iter().chain(out.groups.iter().flat_map(|g| &g.entities)).filter(|e| e.classname != "worldspawn").collect()
    }

    /// World brushes, entities, brush entity brushes and group names.
    fn counts(out: &Output) -> (usize, usize, usize, Vec<String>) {
        let brushes = out.brushes.len() + out.groups.iter().map(|g| g.brushes.len()).sum::<usize>();
        let entities = all_entities(out);
        (brushes, entities.len(), entities.iter().map(|e| e.brushes.len()).sum(), out.groups.iter().map(|g| g.name.clone()).collect())
    }

    #[test]
    fn reads_world_entities_and_visgroups() {
        let import = import(&vmf()).unwrap();
        let out = &import.output;
        assert_eq!(out.brushes.len(), 1, "the hidden brush is in no visgroup");
        let groups: Vec<(&str, usize, usize)> = out.groups.iter().map(|g| (g.name.as_str(), g.brushes.len(), g.entities.len())).collect();
        assert_eq!(groups, [("Course", 0, 1), ("Course / Ramps", 1, 0)], "the empty Hammer group is left out");
        let trigger = &out.groups[0].entities[0];
        assert_eq!(trigger.properties, [("message".to_string(), "{".to_string())]);
        assert_eq!(trigger.brushes.len(), 1);
        let wire = &trigger.connections[0];
        assert_eq!(
            (wire.output.as_str(), wire.target.as_str(), wire.input.as_str(), wire.param.as_str(), wire.delay, wire.times),
            ("OnStartTouch", "a", "b", "", 0.0, -1)
        );
        assert_eq!(out.entities[0].classname, "info_player_counterterrorist");
        assert_eq!(out.entities[1].classname, "worldspawn", "the world's keys come in on their own entity");
        assert_eq!(out.entities[1].properties, [("skyname".to_string(), "sky_day01_01".to_string())]);
        let Brush::Faces(faces) = &out.brushes[0] else { panic!("hull") };
        let (u, v) = faces[0].uv.as_ref().unwrap();
        assert_eq!(faces[0].material, "dev/dev_measuregeneric01b");
        assert_eq!((u.axis.x, u.offset, u.scale, v.axis.y, v.offset, v.scale), (1.0, 16.0, 0.25, -1.0, -8.0, 0.5));
        assert_eq!(import.log[0], "2 world brushes, 2 entities (1 with brushes) with 1 outputs, 2 groups");
        assert_eq!(import.log[1], "skipped: 1 cameras");
    }

    #[test]
    fn round_trips_through_export() {
        let first = import(&vmf()).unwrap().output;
        let export = export(&editor_map(&first)).unwrap();
        assert!(export.text.contains("\"skyname\" \"sky_day01_01\"") && export.text.contains("\"uaxis\" \"[1 0 0 16] 0.25\""), "{}", export.text);
        assert!(export.text.contains("\tconnections\n\t{\n\t\t\"OnStartTouch\" \"a,b,,0,-1\"\n\t}\n"), "{}", export.text);
        let again = import(&export.text).unwrap();
        let names = vec!["import".to_string(), "import / Course".into(), "import / Course / Ramps".into()];
        assert_eq!(counts(&again.output), (2, 2, 1, names));
        let wires = |out: &Output| -> Vec<(String, String, String, String, f64, i32)> {
            let all = all_entities(out).into_iter().flat_map(|e| &e.connections);
            all.map(|c| (c.output.clone(), c.target.clone(), c.input.clone(), c.param.clone(), c.delay, c.times)).collect()
        };
        assert_eq!(wires(&again.output), wires(&first), "outputs survive the trip");
        let ids: Vec<&str> = export.text.lines().filter_map(|l| l.trim().strip_prefix("\"id\" ")).collect();
        let unique: std::collections::BTreeSet<&&str> = ids.iter().collect();
        assert_eq!(ids.len(), unique.len(), "ids are unique");
        assert_eq!(export.text, super::export(&editor_map(&first)).unwrap().text, "ids are stable");
    }

    #[test]
    fn hammer_groups_and_outputs_with_commas() {
        let text = "world\n{\n\t\"classname\" \"worldspawn\"\n\t\"skyname\" \"sky_day01_01\"\n\tSOLID\n\tSOLID\n\
            \tgroup\n\t{\n\t\t\"id\" \"3\"\n\t}\n\tgroup\n\t{\n\t\t\"id\" \"8\"\n\t\teditor\n\t\t{\n\t\t\t\"groupid\" \"3\"\n\t\t}\n\t}\n}\n\
            entity\n{\n\t\"classname\" \"logic_relay\"\n\tconnections\n\t{\n\t\t\"OnTrigger\" \"door\u{1b}AddOutput\u{1b}speed 5, 6\u{1b}1.5\u{1b}1\"\n\t\t\"OnSpawn\" \"x\"\n\t}\n\
            \teditor\n\t{\n\t\t\"groupid\" \"3\"\n\t}\n}\n";
        let solid = |group: &str| format!("solid\n{{\n{}\neditor\n{{\n\"groupid\" \"{group}\"\n}}\n}}", cube(0.0));
        let text = text.replacen("SOLID", &solid("3"), 1).replacen("SOLID", &solid("8"), 1);
        let import = import(&text).unwrap();
        let out = &import.output;
        let groups: Vec<(&str, usize, usize)> = out.groups.iter().map(|g| (g.name.as_str(), g.brushes.len(), g.entities.len())).collect();
        assert_eq!(groups, [("Group 3", 2, 1)], "a nested group's brush joins its outermost group");
        let relay = &out.groups[0].entities[0];
        assert_eq!(relay.connections.len(), 1);
        assert_eq!((relay.connections[0].param.as_str(), relay.connections[0].delay), ("speed 5, 6", 1.5));
        assert_eq!(import.log[1], "skipped: 1 unreadable entity outputs");
        let export = export(&editor_map(out)).unwrap();
        assert!(
            export.text.contains("\"OnTrigger\" \"door\u{1b}AddOutput\u{1b}speed 5, 6\u{1b}1.5\u{1b}1\""),
            "a param with a comma keeps the escape separators"
        );
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

    /// A world cube whose top face, at z 64, has a power 2 displacement.
    fn displaced_cube() -> Vec<Face> {
        let out = import(&format!("world\n{{\nsolid\n{{\n{}}}\n}}\n", cube(0.0))).unwrap().output;
        let Brush::Faces(mut faces) = out.brushes[0].clone() else { panic!("hull") };
        let p = |x, y| Vec3 { x, y, z: 64.0 };
        let mut heights = vec![0.0; 25];
        (heights[1], heights[2], heights[24]) = (8.0, -4.0, 2.5);
        let mut blend = vec![0.0; 25];
        (blend[4], blend[12]) = (1.0, 0.25);
        let corners = (p(0.0, 0.0), p(64.0, 0.0), p(64.0, 64.0), p(0.0, 64.0));
        faces[0].disp = Some(Displacement { power: 2, corners, heights, blend, blend_material: None });
        faces
    }

    fn one_brush(faces: Vec<Face>, in_entity: bool) -> MapData {
        let solid = MapNode { parent: in_entity.then_some(0), kind: MapNodeKind::Brush(Solid { faces, vertices: Vec::new() }) };
        let entity =
            MapNode { parent: None, kind: MapNodeKind::Entity(MapEntity { classname: "func_detail".into(), properties: Vec::new(), connections: Vec::new() }) };
        MapData { name: "disp".into(), world: Vec::new(), nodes: if in_entity { vec![entity, solid] } else { vec![solid] } }
    }

    #[test]
    fn writes_and_reads_displacements() {
        let faces = displaced_cube();
        let export = export(&one_brush(faces.clone(), false)).unwrap();
        let text = &export.text;
        assert!(export.warnings.is_empty(), "{:?}", export.warnings);
        assert!(text.contains("\t\t\tdispinfo\n\t\t\t{\n\t\t\t\t\"power\" \"2\"\n\t\t\t\t\"startposition\" \"[0 0 64]\"\n"), "{text}");
        assert!(text.contains("\t\t\t\tnormals\n\t\t\t\t{\n\t\t\t\t\t\"row0\" \"0 0 1 0 0 1 0 0 -1 0 0 1 0 0 1\"\n"), "a lowered vertex points down: {text}");
        assert!(text.contains("\t\t\t\t\t\"row0\" \"0 8 4 0 0\"\n"), "{text}");
        assert!(
            text.contains(
                "\t\t\t\talphas\n\t\t\t\t{\n\t\t\t\t\t\"row0\" \"0 0 0 0 255\"\n\t\t\t\t\t\"row1\" \"0 0 0 0 0\"\n\t\t\t\t\t\"row2\" \"0 0 64 0 0\"\n"
            ),
            "{text}"
        );
        assert!(text.contains("\t\t\t\ttriangle_tags\n\t\t\t\t{\n\t\t\t\t\t\"row0\" \"0 0 0 0 0 0 0 0\"\n"), "{text}");
        assert_eq!(text.matches("dispinfo").count(), 1);

        let back = import(text).unwrap();
        let Brush::Faces(read) = &back.output.brushes[0] else { panic!("hull") };
        let (want, got) = (faces[0].disp.as_ref().unwrap(), read[0].disp.as_ref().unwrap());
        assert_eq!((got.power, got.corners.0.x, got.corners.0.z, &got.heights), (2, 0.0, 64.0, &want.heights));
        assert!(got.blend.iter().zip(&want.blend).all(|(a, b)| (a - b).abs() <= 0.5 / 255.0), "{:?}", got.blend);
        assert!(read[1..].iter().all(|f| f.disp.is_none()));
        assert_eq!(back.log.len(), 1, "{:?}", back.log);
    }

    #[test]
    fn displacements_stay_flat_where_source_has_none() {
        let entity = export(&one_brush(displaced_cube(), true)).unwrap();
        assert!(!entity.text.contains("dispinfo"));
        assert_eq!(entity.warnings, ["1 displacements on brush entities were written flat; Source allows them only on world brushes"]);

        let mut faces = displaced_cube();
        let d = faces[0].disp.as_mut().unwrap();
        (d.power, d.heights, d.blend) = (1, vec![0.0; 9], Vec::new());
        let low = export(&one_brush(faces, false)).unwrap();
        assert!(!low.text.contains("dispinfo"));
        assert_eq!(low.warnings, ["1 displacements were power 1, which Source does not allow, and were written flat"]);

        let mut faces = displaced_cube();
        faces[0].disp.as_mut().unwrap().blend_material = Some("grass".into());
        faces[1].attrs = vec![("anti_tile".into(), "macro".into()), ("lightmapscale".into(), "32".into())];
        let blended = export(&one_brush(faces, false)).unwrap();
        assert!(blended.text.contains("dispinfo") && blended.text.contains("\"lightmapscale\" \"32\""));
        assert_eq!(blended.warnings.len(), 2);
        assert!(blended.warnings[0].starts_with("1 displaced faces blend grass through MapCraft's second layer"), "{:?}", blended.warnings);
        assert_eq!(blended.warnings[1], "1 faces use anti-tiling, which Source cannot show; they export plain");
        let Brush::Faces(read) = &import(&blended.text).unwrap().output.brushes[0] else { panic!("hull") };
        assert_eq!(read[1].attrs, [("lightmapscale".to_string(), "32".to_string())], "attributes Source knows come back");
    }

    #[test]
    fn reads_sparse_and_broken_dispinfo() {
        let grid = |name: &str, row: &str| format!("{name}\n{{\n{}}}\n", (0..5).map(|r| format!("\"row{r}\" \"{row}\"\n")).collect::<String>());
        let side = |info: &str| {
            let cube = cube(0.0);
            let first = cube.find("}\n").unwrap();
            format!("world\n{{\nsolid\n{{\n{}{info}{}}}\n}}\n", &cube[..first], &cube[first..])
        };
        let sparse = format!(
            "dispinfo\n{{\n\"power\" \"2\"\n\"startposition\" \"[0 0 64]\"\n{}{}}}\n",
            grid("normals", &"0 0 1 ".repeat(5)),
            grid("distances", "1 2 3 4 5")
        );
        let read = import(&side(&sparse)).unwrap();
        let Brush::Faces(faces) = &read.output.brushes[0] else { panic!("hull") };
        let d = faces[0].disp.as_ref().unwrap();
        assert_eq!((&d.heights[..5], d.blend.iter().all(|w| *w == 0.0)), (&[1.0, 2.0, 3.0, 4.0, 5.0][..], true), "missing offsets and alphas are zeros");

        let short = format!(
            "dispinfo\n{{\n\"power\" \"2\"\n\"startposition\" \"[0 0 64]\"\n{}{}}}\n",
            grid("normals", &"0 0 1 ".repeat(5)),
            grid("distances", "1 2 3 4")
        );
        let read = import(&side(&short)).unwrap();
        let Brush::Faces(faces) = &read.output.brushes[0] else { panic!("hull") };
        assert!(faces[0].disp.is_none());
        assert!(read.log.iter().any(|l| l == "1 unreadable displacements came in flat"), "{:?}", read.log);

        let slid = format!(
            "dispinfo\n{{\n\"power\" \"2\"\n\"startposition\" \"[0 0 64]\"\n{}{}{}}}\n",
            grid("normals", &"0 0 1 ".repeat(5)),
            grid("distances", "1 1 1 1 1"),
            grid("offsets", &"3 0 0 ".repeat(5))
        );
        let read = import(&side(&slid)).unwrap();
        assert!(read.log.iter().any(|l| l == "1 displacements had sideways offsets, which MapCraft drops"), "{:?}", read.log);
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
        let export = export(&editor_map(&first.output)).unwrap();
        let again = import(&export.text).unwrap();
        let (a, b) = (counts(&first.output), counts(&again.output));
        assert_eq!((a.0, a.1, a.2), (b.0, b.1, b.2));
        assert_eq!((a.0, a.1), (3373, 4411));
    }
}
