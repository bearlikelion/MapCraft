//! `.mcmap`: MapCraft's native map file, a JSON tree described in `docs/mcmap-format.md`.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use mapcraft_geom::DVec3;
use mapcraft_map::{Cordon, Extra, FaceId, Map, Node, NodeId, NodeKind, ROOT, Tag, TagId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const FORMAT: &str = "mapcraft.map";
pub const SCHEMA_VERSION: u32 = 3;
pub const EXTENSION: &str = "mcmap";

#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    #[error("not a MapCraft map: {0}")]
    NotMcmap(String),
    #[error("made by a newer MapCraft (schema {0}, this version reads up to {SCHEMA_VERSION})")]
    Newer(u32),
    #[error("invalid map file: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid map: {0}")]
    Map(#[from] mapcraft_map::EditError),
    #[error("node {0} appears more than once")]
    DuplicateId(u64),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// A loaded map plus the file data that lives outside the document.
#[derive(Clone, Debug, PartialEq)]
pub struct MapFile {
    pub map: Map,
    /// Id of the SDK the map was made with.
    pub sdk: Option<String>,
    /// Editor state (cameras, view layout); not part of undo.
    pub view: Value,
}

impl MapFile {
    pub fn to_string(&self) -> Result<String, FormatError> {
        to_string(&self.map, self.sdk.as_deref(), &self.view)
    }
}

#[derive(Serialize, Deserialize)]
struct FileNode {
    id: NodeId,
    #[serde(flatten)]
    kind: NodeKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tags: Vec<TagId>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    hidden: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    locked: bool,
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    extra: Extra,
    /// A mirror group's copy, written for importers and rebuilt by MapCraft instead of read.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    derived: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    children: Vec<FileNode>,
    /// A boolean group's own brushes, kept out of `children` so importers get only its result.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    sources: Vec<FileNode>,
}

#[derive(Deserialize)]
struct Envelope {
    format: String,
    schema_version: u32,
    #[serde(default)]
    sdk: Option<String>,
    map: FileMap,
    #[serde(default)]
    view: Value,
}

#[derive(Deserialize)]
struct FileMap {
    #[serde(default)]
    tags: Vec<Tag>,
    #[serde(default)]
    cordons: Vec<Cordon>,
    #[serde(default)]
    extra: Extra,
    root: FileNode,
}

/// Removes float noise such as `63.99999999997` from derived coordinates.
fn tidy(v: DVec3) -> [f64; 3] {
    v.to_array().map(|c| {
        let r = c.round();
        if (c - r).abs() < 1e-6 { r } else { (c * 1e6).round() / 1e6 }
    })
}

/// Nodes written as `derived`, with ids numbered after the map's own so saves are stable.
#[derive(Default)]
struct Derived {
    nodes: BTreeSet<NodeId>,
    covered: BTreeSet<(NodeId, FaceId)>,
    next_node: u64,
    next_face: u64,
}

/// `id` of the expanded `map` as JSON; nodes it shares with `own`, the map as edited, keep their own kind and brushes.
fn node_value(map: &Map, own: &Map, id: NodeId, derived: &mut Derived) -> Result<Value, FormatError> {
    let node = map.get(id).ok_or(mapcraft_map::EditError::Missing(id))?;
    let is_derived = derived.nodes.contains(&id);
    let mine = own.get(id).filter(|_| !is_derived);
    let mut kind = std::borrow::Cow::Borrowed(mine.map_or(&node.kind, |n| &n.kind));
    let mut file_id = id;
    if is_derived {
        file_id = NodeId(derived.next_node);
        derived.next_node = derived.next_node.saturating_add(1);
        if let NodeKind::Brush(b) = kind.to_mut() {
            for f in &mut b.faces {
                f.id = FaceId(derived.next_face);
                derived.next_face = derived.next_face.saturating_add(1);
            }
        }
    }
    let mut v = serde_json::to_value(FileNodeRef { id: file_id, kind: &kind, tags: &node.tags, hidden: node.hidden, locked: node.locked, extra: &node.extra })?;
    if let (true, Some(obj)) = (is_derived, v.as_object_mut()) {
        obj.insert("derived".into(), Value::Bool(true));
    }
    if let (NodeKind::Brush(b), Some(faces)) = (&node.kind, v.get_mut("faces").and_then(Value::as_array_mut)) {
        // Engine importers read these polygons so they need no brush kernel; invalid brushes get none.
        if let Ok(solid) = b.solid() {
            let displaced: std::collections::BTreeMap<usize, Vec<DVec3>> = b.displaced(&solid).into_iter().collect();
            for (i, ((face, winding), f)) in faces.iter_mut().zip(&solid.faces).zip(&b.faces).enumerate() {
                if let (Some(obj), Some(w)) = (face.as_object_mut(), winding) {
                    obj.insert("polygon".into(), json!(w.0.iter().map(|p| tidy(*p)).collect::<Vec<_>>()));
                    if derived.covered.contains(&(id, f.id)) {
                        obj.insert("covered".into(), Value::Bool(true));
                    }
                    if let Some(grid) = displaced.get(&i) {
                        obj.insert("disp_vertices".into(), json!(grid.iter().map(|p| tidy(*p)).collect::<Vec<_>>()));
                    }
                }
            }
        }
    }
    let children = node.children.iter().map(|c| node_value(map, own, *c, derived)).collect::<Result<Vec<_>, _>>()?;
    if let (false, Some(obj)) = (children.is_empty(), v.as_object_mut()) {
        obj.insert("children".into(), Value::Array(children));
    }
    let dropped: Vec<NodeId> = mine.map(|n| n.children.iter().copied().filter(|c| map.get(*c).is_none()).collect()).unwrap_or_default();
    let sources = dropped.into_iter().map(|c| node_value(own, own, c, derived)).collect::<Result<Vec<_>, _>>()?;
    if let (false, Some(obj)) = (sources.is_empty(), v.as_object_mut()) {
        obj.insert("sources".into(), Value::Array(sources));
    }
    Ok(v)
}

#[derive(Serialize)]
struct FileNodeRef<'a> {
    id: NodeId,
    #[serde(flatten)]
    kind: &'a NodeKind,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tags: &'a Vec<TagId>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    hidden: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    locked: bool,
    #[serde(skip_serializing_if = "Extra::is_empty")]
    extra: &'a Extra,
}

pub fn to_string(map: &Map, sdk: Option<&str>, view: &Value) -> Result<String, FormatError> {
    let (full, owner) = mapcraft_map::mirror::expand(map);
    let mut derived = Derived {
        nodes: owner.into_keys().collect(),
        covered: mapcraft_map::covered_faces(&full, None),
        next_node: map.nodes().map(|(id, _)| id.0).max().unwrap_or(0).saturating_add(1),
        next_face: map.nodes().filter_map(|(_, n)| n.kind.brush()).flat_map(|b| &b.faces).map(|f| f.id.0).max().unwrap_or(0).saturating_add(1),
    };
    let mut doc_map = json!({ "root": node_value(&full, map, ROOT, &mut derived)? });
    if let Some(obj) = doc_map.as_object_mut() {
        if !map.tags.is_empty() {
            obj.insert("tags".into(), serde_json::to_value(&map.tags)?);
        }
        if !map.cordons.is_empty() {
            obj.insert("cordons".into(), serde_json::to_value(&map.cordons)?);
        }
        if !map.extra.is_empty() {
            obj.insert("extra".into(), serde_json::to_value(&map.extra)?);
        }
    }
    // Each map is written with the lowest schema that holds it, so older builds refuse only maps they would lose data from.
    let boolean = map.nodes().any(|(_, n)| matches!(&n.kind, NodeKind::Group(g) if g.boolean));
    let blended = map
        .nodes()
        .filter_map(|(_, n)| n.kind.brush())
        .flat_map(|b| &b.faces)
        .any(|f| f.blend_layer().is_some() || f.disp.as_ref().is_some_and(|d| !d.blend.is_empty()));
    let mut doc = json!({
        "format": FORMAT,
        "schema_version": if boolean { 3 } else if blended { 2 } else { 1 },
        "generator": concat!("MapCraft ", env!("CARGO_PKG_VERSION")),
    });
    if let Some(obj) = doc.as_object_mut() {
        if let Some(sdk) = sdk {
            obj.insert("sdk".into(), json!(sdk));
        }
        obj.insert("map".into(), doc_map);
        if !view.is_null() {
            obj.insert("view".into(), view.clone());
        }
    }
    let mut out = String::new();
    write_value(&doc, 0, &mut out);
    out.push('\n');
    Ok(out)
}

/// Pretty JSON with short arrays of numbers kept on one line, so diffs stay readable.
fn write_value(v: &Value, indent: usize, out: &mut String) {
    let pad = |n: usize, out: &mut String| out.extend(std::iter::repeat_n("  ", n));
    match v {
        Value::Number(n) => match n.as_f64() {
            // Integral floats print without ".0" and -0 prints as 0.
            Some(f) if f.fract() == 0.0 && f.abs() < 1e15 && !n.is_i64() && !n.is_u64() => out.push_str(&format!("{}", f as i64)),
            _ => out.push_str(&n.to_string()),
        },
        Value::Array(items) if items.is_empty() => out.push_str("[]"),
        Value::Array(items) if is_inline(v) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_value(item, indent, out);
            }
            out.push(']');
        }
        Value::Array(items) => {
            out.push_str("[\n");
            for (i, item) in items.iter().enumerate() {
                pad(indent + 1, out);
                write_value(item, indent + 1, out);
                out.push_str(if i + 1 < items.len() { ",\n" } else { "\n" });
            }
            pad(indent, out);
            out.push(']');
        }
        Value::Object(map) if map.is_empty() => out.push_str("{}"),
        Value::Object(map) if is_inline(v) => {
            out.push('{');
            for (i, (k, item)) in map.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&Value::String(k.clone()).to_string());
                out.push_str(": ");
                write_value(item, indent, out);
            }
            out.push('}');
        }
        Value::Object(map) => {
            out.push_str("{\n");
            for (i, (k, item)) in map.iter().enumerate() {
                pad(indent + 1, out);
                out.push_str(&Value::String(k.clone()).to_string());
                out.push_str(": ");
                write_value(item, indent + 1, out);
                out.push_str(if i + 1 < map.len() { ",\n" } else { "\n" });
            }
            pad(indent, out);
            out.push('}');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// Points, short lists of points, key/value pairs and texture axes stay on one line.
fn is_inline(v: &Value) -> bool {
    let numbers = |a: &Value| a.as_array().is_some_and(|xs| xs.len() <= 4 && xs.iter().all(Value::is_number));
    match v {
        Value::Array(items) => items.iter().all(|i| i.is_number() || i.is_string() && items.len() <= 4) || (items.len() <= 3 && items.iter().all(numbers)),
        Value::Object(map) => map.len() <= 3 && map.values().all(|x| x.is_number() || numbers(x)),
        _ => true,
    }
}

pub fn from_str(text: &str) -> Result<MapFile, FormatError> {
    let head: Value = serde_json::from_str(text)?;
    match head.get("format").and_then(Value::as_str) {
        Some(FORMAT) => {}
        other => return Err(FormatError::NotMcmap(format!("format is {other:?}"))),
    }
    let version = head.get("schema_version").and_then(Value::as_u64).unwrap_or(0);
    if version > u64::from(SCHEMA_VERSION) {
        return Err(FormatError::Newer(u32::try_from(version).unwrap_or(u32::MAX)));
    }
    let env: Envelope = serde_json::from_value(head)?;
    if env.format != FORMAT || env.schema_version == 0 {
        return Err(FormatError::NotMcmap("missing schema version".into()));
    }
    let mut nodes = BTreeMap::new();
    let mut stack = vec![(env.map.root, None)];
    while let Some((f, parent)) = stack.pop() {
        let id = f.id;
        let kept: Vec<FileNode> = f.sources.into_iter().chain(f.children.into_iter().filter(|c| !c.derived)).collect();
        let children: Vec<NodeId> = kept.iter().map(|c| c.id).collect();
        let node = Node { parent, children, kind: f.kind, tags: f.tags, hidden: f.hidden, locked: f.locked, extra: f.extra };
        if nodes.insert(id, node).is_some() {
            return Err(FormatError::DuplicateId(id.0));
        }
        stack.extend(kept.into_iter().map(|c| (c, Some(id))));
    }
    let mut map = Map::from_nodes(nodes, env.map.tags, env.map.extra)?;
    map.cordons = env.map.cordons;
    Ok(MapFile { map, sdk: env.sdk, view: env.view })
}

pub const CLIPBOARD_FORMAT: &str = "mapcraft.clipboard";

/// Copied nodes, without ids; brush faces get fresh ids when pasted.
#[derive(Clone, Debug, PartialEq)]
pub struct Fragment {
    pub kind: NodeKind,
    pub children: Vec<Fragment>,
}

/// The subtrees under `roots` as clipboard text.
pub fn fragment_to_string(map: &Map, roots: &[NodeId]) -> Result<String, FormatError> {
    let nodes = roots.iter().map(|id| node_value(map, map, *id, &mut Derived::default())).collect::<Result<Vec<_>, _>>()?;
    let mut out = String::new();
    write_value(&json!({ "format": CLIPBOARD_FORMAT, "nodes": nodes }), 0, &mut out);
    Ok(out)
}

pub fn fragment_from_str(text: &str) -> Result<Vec<Fragment>, FormatError> {
    #[derive(Deserialize)]
    struct Clip {
        format: String,
        nodes: Vec<FileNode>,
    }
    fn convert(f: FileNode) -> Fragment {
        Fragment { kind: f.kind, children: f.sources.into_iter().chain(f.children.into_iter().filter(|c| !c.derived)).map(convert).collect() }
    }
    let clip: Clip = serde_json::from_str(text)?;
    if clip.format != CLIPBOARD_FORMAT {
        return Err(FormatError::NotMcmap(format!("clipboard format is {:?}", clip.format)));
    }
    Ok(clip.nodes.into_iter().map(convert).collect())
}

pub fn load(path: &Path) -> Result<MapFile, FormatError> {
    from_str(&std::fs::read_to_string(path)?)
}

/// Writes atomically: a failed save leaves the previous file intact.
pub fn save(path: &Path, map: &Map, sdk: Option<&str>, view: &Value) -> Result<(), FormatError> {
    craft_atomic::atomic_write(path, to_string(map, sdk, view)?.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use mapcraft_geom::{Aabb, DMat4};
    use mapcraft_map::{Brush, Connection, Entity, Group, Layer, Prefab, TexDefaults};

    use super::*;

    fn sample() -> MapFile {
        let mut map = Map::new();
        let mut e = map.edit();
        let tex = TexDefaults { material: "ramps-orange1".into(), scale: 1.0 };
        let layer = e.insert(ROOT, NodeKind::Layer(Layer { name: "Ramps".into(), color: Some([255, 128, 0]), omit_from_export: false })).unwrap();
        e.insert(layer, NodeKind::Brush(Brush::cuboid(Aabb::new(DVec3::ZERO, DVec3::new(64.0, 32.0, 16.0)), &tex))).unwrap();
        let mut zone = Entity::new("zone_start");
        zone.set("course_id", "0");
        zone.connections.push(Connection {
            output: "OnStartTouch".into(),
            target: "timer".into(),
            input: "Start".into(),
            param: String::new(),
            delay: 0.5,
            times_to_fire: -1,
        });
        let z = e.insert(ROOT, NodeKind::Entity(zone)).unwrap();
        e.insert(z, NodeKind::Brush(Brush::cuboid(Aabb::new(DVec3::splat(100.0), DVec3::splat(164.0)), &tex))).unwrap();
        e.insert(ROOT, NodeKind::Group(Group { name: "Pillars".into(), ..Group::default() })).unwrap();
        e.insert(ROOT, NodeKind::Prefab(Prefab { path: "prefabs/start.mcmap".into(), origin: DVec3::new(0.0, 0.0, 128.0), ..Prefab::default() })).unwrap();
        map.apply(e.finish());
        map.cordons.push(Cordon { name: "Start".into(), bounds: Aabb::new(DVec3::ZERO, DVec3::splat(512.0)), active: true });
        MapFile { map, sdk: Some("surfsup".into()), view: json!({ "camera": [0, 0, 64] }) }
    }

    #[test]
    fn round_trip_is_stable() {
        let file = sample();
        let text = file.to_string().unwrap();
        let back = from_str(&text).unwrap();
        assert_eq!(back, file);
        assert_eq!(back.to_string().unwrap(), text);
        assert!(text.contains("\"polygon\""));
        assert!(text.contains("\"type\": \"zone_start\"") || text.contains("\"classname\": \"zone_start\""));
    }

    #[test]
    fn linked_copies_are_written_filled_and_read_back_as_links() {
        use mapcraft_map::Link;
        let mut map = Map::new();
        let mut e = map.edit();
        let tex = TexDefaults { material: "a".into(), scale: 1.0 };
        let lamp = e
            .insert(
                ROOT,
                NodeKind::Group(Group { name: "Lamp".into(), link: Some(Box::new(Link { source: None, frame: DMat4::IDENTITY })), ..Group::default() }),
            )
            .unwrap();
        e.insert(lamp, NodeKind::Brush(Brush::cuboid(Aabb::new(DVec3::ZERO, DVec3::splat(16.0)), &tex))).unwrap();
        let frame = DMat4::from_translation(DVec3::new(256.0, 0.0, 0.0));
        e.insert(ROOT, NodeKind::Group(Group { name: "Lamp".into(), link: Some(Box::new(Link { source: Some(lamp), frame })), ..Group::default() })).unwrap();
        map.apply(e.finish());
        let file = MapFile { map, sdk: None, view: Value::Null };
        let text = file.to_string().unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        let copy = &v["map"]["root"]["children"][1];
        assert_eq!(copy["children"][0]["derived"], true, "importers get the copy's brushes");
        assert_eq!(from_str(&text).unwrap(), file, "and the editor reads the link back");
    }

    #[test]
    fn faces_between_brushes_are_marked_covered() {
        let mut map = Map::new();
        let mut e = map.edit();
        let tex = TexDefaults { material: "a".into(), scale: 1.0 };
        for x in [0.0, 64.0] {
            e.insert(ROOT, NodeKind::Brush(Brush::cuboid(Aabb::new(DVec3::new(x, 0.0, 0.0), DVec3::new(x + 64.0, 64.0, 64.0)), &tex))).unwrap();
        }
        map.apply(e.finish());
        let file = MapFile { map, sdk: None, view: Value::Null };
        let text = file.to_string().unwrap();
        assert_eq!(text.matches("\"covered\": true").count(), 2);
        assert_eq!(from_str(&text).unwrap(), file, "covered is derived and ignored on load");
    }

    #[test]
    fn polygons_are_exact() {
        let text = sample().to_string().unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        let brush = &v["map"]["root"]["children"][0]["children"][0];
        assert_eq!(brush["type"], "brush");
        let poly = brush["faces"][0]["polygon"].as_array().unwrap();
        assert_eq!(poly.len(), 4);
        assert!(poly.iter().flat_map(|p| p.as_array().unwrap()).all(|c| c.as_f64().unwrap().fract() == 0.0));
    }

    #[test]
    fn rejects_bad_files() {
        assert!(matches!(from_str("{}"), Err(FormatError::NotMcmap(_))));
        assert!(matches!(from_str(r#"{"format":"mapcraft.map","schema_version":99,"map":{}}"#), Err(FormatError::Newer(99))));
        assert!(from_str("not json").is_err());
        let dup = r#"{"format":"mapcraft.map","schema_version":1,"map":{"root":{"id":1,"type":"world","classname":"worldspawn","children":[{"id":1,"type":"group","name":"x"}]}}}"#;
        assert!(from_str(dup).is_err());
        let bad_parent = r#"{"format":"mapcraft.map","schema_version":1,"map":{"root":{"id":1,"type":"world","classname":"worldspawn","children":[{"id":2,"type":"brush","faces":[],"children":[{"id":3,"type":"group","name":"x"}]}]}}}"#;
        assert!(from_str(bad_parent).is_err());
        let deep = format!(r#"{{"format":"mapcraft.map","schema_version":1,"map":{{"root":{}}}}}"#, "{\"id\":1,\"children\":[".repeat(5000));
        assert!(from_str(&deep).is_err());
    }

    /// `file` with its first brush's first face displaced at power 2 and changed by `paint`.
    fn displaced(mut file: MapFile, paint: impl Fn(&mut mapcraft_map::Displacement)) -> MapFile {
        let (id, mut b) = file.map.nodes().find_map(|(id, n)| Some((id, n.kind.brush()?.clone()))).unwrap();
        let start = b.solid().unwrap().faces[0].as_ref().unwrap().0[0];
        let mut d = mapcraft_map::Displacement::new(2, start);
        paint(&mut d);
        b.faces[0].disp = Some(d);
        let mut e = file.map.edit();
        e.set_kind(id, NodeKind::Brush(b)).unwrap();
        file.map.apply(e.finish());
        file
    }

    #[test]
    fn blend_data_raises_the_schema_version() {
        let version = |f: &MapFile| serde_json::from_str::<Value>(&f.to_string().unwrap()).unwrap()["schema_version"].clone();
        let painted = displaced(sample(), |d| {
            d.set_blend((0..25).map(|k| k as f64 / 24.0).collect());
            d.blend_material = Some("grass".into());
        });
        let text = painted.to_string().unwrap();
        let back = from_str(&text).unwrap();
        assert_eq!(back, painted);
        assert_eq!(back.to_string().unwrap(), text, "the round trip is stable");
        assert_eq!(version(&painted), 2);
        assert_eq!(version(&sample()), 1, "unpainted maps stay readable by older builds");
        assert_eq!(version(&displaced(sample(), |d| d.blend_material = Some(String::new()))), 1, "an empty layer name is no blend");
        assert_eq!(version(&displaced(sample(), |d| d.blend_material = Some("grass".into()))), 2, "a layer counts before any paint");

        let old = from_str(&displaced(sample(), |_| {}).to_string().unwrap()).unwrap();
        let disps = || old.map.nodes().filter_map(|(_, n)| n.kind.brush()).flat_map(|b| &b.faces).filter_map(|f| f.disp.as_ref()).collect::<Vec<_>>();
        assert!(disps().len() == 1 && disps()[0].blend.is_empty(), "a schema 1 grid loads unpainted");

        let mut short: Value = serde_json::from_str(&text).unwrap();
        let disp = &mut short["map"]["root"]["children"][0]["children"][0]["faces"][0]["disp"];
        assert_eq!(disp["blend"].as_array().map(Vec::len), Some(25));
        disp["blend"] = json!([0.5, 7, -3]);
        let loaded = from_str(&short.to_string()).unwrap();
        let d = loaded.map.nodes().find_map(|(_, n)| n.kind.brush()?.faces[0].disp.clone()).unwrap();
        assert_eq!((d.blend_at(1), d.blend_at(2), d.blend_at(3)), (1.0, 0.0, 0.0), "a short blend loads and reads tamed");
        short["schema_version"] = json!(SCHEMA_VERSION + 1);
        assert!(matches!(from_str(&short.to_string()), Err(FormatError::Newer(v)) if v == SCHEMA_VERSION + 1));
    }

    #[test]
    fn clipboard_round_trip() {
        let file = sample();
        let roots: Vec<NodeId> = file.map.get(ROOT).unwrap().children.clone();
        let text = fragment_to_string(&file.map, &roots).unwrap();
        let frags = fragment_from_str(&text).unwrap();
        assert_eq!(frags.len(), roots.len());
        assert!(matches!(frags[0].kind, NodeKind::Layer(_)));
        assert_eq!(frags[0].children.len(), 1);
        assert!(fragment_from_str("{\"format\":\"other\",\"nodes\":[]}").is_err());
        assert!(fragment_from_str("garbage").is_err());
    }

    #[test]
    fn mirror_halves_are_written_but_not_read() {
        let mut file = sample();
        let mut e = file.map.edit();
        let mirror = mapcraft_map::Mirror { axis: mapcraft_map::Axis::X, offset: 0.0, texture_lock: true };
        let g = e.insert(ROOT, NodeKind::Group(Group { name: "Half".into(), mirror: Some(mirror), ..Group::default() })).unwrap();
        e.insert(g, NodeKind::Brush(Brush::cuboid(Aabb::new(DVec3::new(-64.0, 0.0, 0.0), DVec3::new(0.0, 64.0, 64.0)), &TexDefaults::default()))).unwrap();
        file.map.apply(e.finish());
        let text = file.to_string().unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        let half = v["map"]["root"]["children"].as_array().unwrap().last().unwrap();
        let kids = half["children"].as_array().unwrap();
        assert_eq!(kids.len(), 2);
        assert_eq!(kids[1]["derived"], true);
        let xs: Vec<f64> =
            kids[1]["faces"].as_array().unwrap().iter().flat_map(|f| f["polygon"].as_array().unwrap().iter().map(|p| p[0].as_f64().unwrap())).collect();
        assert_eq!(
            (xs.iter().copied().fold(f64::MAX, f64::min), xs.iter().copied().fold(f64::MIN, f64::max)),
            (0.0, 64.0),
            "importers get the mirrored polygons"
        );
        let back = from_str(&text).unwrap();
        assert_eq!(back, file, "the copy is dropped on read");
        assert_eq!(back.to_string().unwrap(), text);
    }

    #[test]
    fn save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.mcmap");
        let s = sample();
        save(&path, &s.map, s.sdk.as_deref(), &s.view).unwrap();
        assert_eq!(load(&path).unwrap(), sample());
    }

    #[test]
    fn paths_and_curve_groups_round_trip() {
        use mapcraft_map::{CurveOp, Path, PathPoint};
        let mut file = sample();
        let mut e = file.map.edit();
        let pt = |x: f64| PathPoint { pos: DVec3::new(x, 0.0, 0.0), handle_in: DVec3::new(x - 32.0, 0.0, 0.0), handle_out: DVec3::new(x + 32.0, 16.0, 0.0) };
        let path = e.insert(ROOT, NodeKind::Path(Path { points: vec![pt(0.0), pt(256.0)], closed: false })).unwrap();
        let op = CurveOp::Array { path, nodes: vec![path], count: 3, spacing: 0.0, yaw_only: true, bind: mapcraft_geom::DMat4::IDENTITY };
        e.insert(ROOT, NodeKind::Group(Group { name: "Posts".into(), curve: Some(Box::new(op)), ..Group::default() })).unwrap();
        file.map.apply(e.finish());
        let text = file.to_string().unwrap();
        assert!(text.contains("\"type\": \"path\"") && text.contains("\"op\": \"array\"") && text.contains("\"in\": [-32, 0, 0]"));
        assert_eq!(from_str(&text).unwrap(), file);
    }
}
