//! `.mcmap`: MapCraft's native map file, a JSON tree described in `docs/mcmap-format.md`.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::BTreeMap;
use std::path::Path;

use mapcraft_geom::DVec3;
use mapcraft_map::{Cordon, Extra, Map, Node, NodeId, NodeKind, ROOT, Tag, TagId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const FORMAT: &str = "mapcraft.map";
pub const SCHEMA_VERSION: u32 = 1;
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    children: Vec<FileNode>,
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

fn node_value(map: &Map, id: NodeId) -> Result<Value, FormatError> {
    let node = map.get(id).ok_or(mapcraft_map::EditError::Missing(id))?;
    let mut v = serde_json::to_value(FileNodeRef { id, kind: &node.kind, tags: &node.tags, hidden: node.hidden, locked: node.locked, extra: &node.extra })?;
    if let (NodeKind::Brush(b), Some(faces)) = (&node.kind, v.get_mut("faces").and_then(Value::as_array_mut)) {
        // Engine importers read these polygons so they need no brush kernel; invalid brushes get none.
        if let Ok(solid) = b.solid() {
            let displaced: std::collections::BTreeMap<usize, Vec<DVec3>> = b.displaced(&solid).into_iter().collect();
            for (i, (face, winding)) in faces.iter_mut().zip(&solid.faces).enumerate() {
                if let (Some(obj), Some(w)) = (face.as_object_mut(), winding) {
                    obj.insert("polygon".into(), json!(w.0.iter().map(|p| tidy(*p)).collect::<Vec<_>>()));
                    if let Some(grid) = displaced.get(&i) {
                        obj.insert("disp_vertices".into(), json!(grid.iter().map(|p| tidy(*p)).collect::<Vec<_>>()));
                    }
                }
            }
        }
    }
    let children = node.children.iter().map(|c| node_value(map, *c)).collect::<Result<Vec<_>, _>>()?;
    if let (false, Some(obj)) = (children.is_empty(), v.as_object_mut()) {
        obj.insert("children".into(), Value::Array(children));
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
    let mut doc_map = json!({ "root": node_value(map, ROOT)? });
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
    let mut doc = json!({
        "format": FORMAT,
        "schema_version": SCHEMA_VERSION,
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
        let children: Vec<NodeId> = f.children.iter().map(|c| c.id).collect();
        let node = Node { parent, children, kind: f.kind, tags: f.tags, hidden: f.hidden, locked: f.locked, extra: f.extra };
        if nodes.insert(id, node).is_some() {
            return Err(FormatError::DuplicateId(id.0));
        }
        stack.extend(f.children.into_iter().map(|c| (c, Some(id))));
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
    let nodes = roots.iter().map(|id| node_value(map, *id)).collect::<Result<Vec<_>, _>>()?;
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
        Fragment { kind: f.kind, children: f.children.into_iter().map(convert).collect() }
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
    use mapcraft_geom::Aabb;
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
        e.insert(ROOT, NodeKind::Group(Group { name: "Pillars".into(), generator: None })).unwrap();
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
    fn save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.mcmap");
        let s = sample();
        save(&path, &s.map, s.sdk.as_deref(), &s.view).unwrap();
        assert_eq!(load(&path).unwrap(), sample());
    }
}
