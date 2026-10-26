//! Imports SurfsUp SDK maps: a Godot pack (`.pck`) holding the level scene, or the text scene
//! itself. CSG shapes become brushes, zones become SurfsUp entities, and the sky and sun become a
//! `light_environment`.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use glam::{DAffine3, DMat3, DVec2, DVec3};

use crate::mapcraft::plugin::types::{Brush, Entity, Group, Hull, Image, MaterialInfo, Output, Vec3};
use crate::tscn::{self, Section, Value};

/// SurfsUp's scale: map units (inches) per Godot meter.
const UNITS_PER_METER: f64 = 39.37;
/// MapCraft's world reaches this far from the origin on every axis.
const WORLD: f64 = 131_072.0;
/// Path points within this many map units of the line through their neighbors are dropped.
const PATH_TOLERANCE: f64 = 1.0;
/// Path points are also kept where the path turns more than this, so tall profiles stay true.
const PATH_TURN_DEGREES: f64 = 0.5;
/// Instanced scenes nest at most this deep.
const MAX_NESTING: usize = 8;
const MAX_NODES: usize = 200_000;
const MAX_BRUSHES: usize = 200_000;
const MAX_PATH_POINTS: usize = 100_000;
const MAX_POLYGON_POINTS: usize = 512;
/// How far a world boundary kill zone reaches past the map's brushes, and how thick it is.
const BOUNDARY_MARGIN: f64 = 4096.0;
const BOUNDARY_THICKNESS: f64 = 512.0;
const CLIP: &str = "tools/clip";
const TRIGGER: &str = "tools/trigger";

/// What an import made, and lines for the Output panel.
pub struct Imported {
    pub output: Output,
    pub log: Vec<String>,
}

pub fn import(name: &str, data: &[u8]) -> Result<Imported, String> {
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    let mut doc = Doc { data, files: BTreeMap::new(), scenes: Vec::new(), by_path: BTreeMap::new() };
    let main = if data.starts_with(b"GDPC") {
        doc.files = pack_files(data)?;
        let path = level_scene(&doc.files)?;
        doc.load(&path, 0).ok_or_else(|| format!("{path} is not a readable text scene"))?
    } else if data.starts_with(b"[gd_scene") {
        let text = std::str::from_utf8(data).map_err(|_| "the scene is not UTF-8 text".to_string())?;
        doc.add(name.to_string(), tscn::parse(text)?, 0)
    } else {
        return Err("not a Godot pack (.pck) or text scene (.tscn)".into());
    };
    let nodes = doc.nodes(main)?;
    let mut b = Builder::new(&doc, &nodes, stem);
    b.run()?;
    let scene = doc.scenes.get(main).map_or(name, |s| s.path.as_str());
    let mut log = vec![format!(
        "{name}: {scene}: {} brushes, {} groups, {} zones, {} other entities, {} materials",
        b.brush_count,
        b.groups.len(),
        b.zones,
        b.entities.len() - b.zones,
        b.materials.infos.len()
    )];
    if !b.skipped.is_empty() {
        let list: Vec<String> = b.skipped.iter().map(|(what, n)| format!("{n} {what}")).collect();
        log.push(format!("{name}: left out {}", list.join(", ")));
    }
    if b.clamped > 0 {
        log.push(format!("{name}: {} brushes reached past MapCraft's world and were cut to fit", b.clamped));
    }
    let output = Output { brushes: Vec::new(), entities: b.entities, groups: b.groups, materials: b.materials.infos };
    Ok(Imported { output, log })
}

/// Files in a Godot pack by path without `res://`, as byte ranges of the pack.
fn pack_files(data: &[u8]) -> Result<BTreeMap<String, Range<usize>>, String> {
    let u32_at = |at: usize| data.get(at..at.checked_add(4)?).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes);
    let u64_at = |at: usize| data.get(at..at.checked_add(8)?).and_then(|b| b.try_into().ok()).map(u64::from_le_bytes);
    let bad = || "the pack's directory is damaged".to_string();
    let format = u32_at(4).ok_or_else(bad)?;
    if !(2..=4).contains(&format) {
        return Err(format!("pack format {format} is not a Godot 4 pack"));
    }
    let flags = u32_at(20).ok_or_else(bad)?;
    let base = u64_at(24).ok_or_else(bad)?;
    if flags & 1 != 0 {
        return Err("the pack is encrypted".into());
    }
    // Godot 4.4 moved the directory to the end of the pack and points at it from the header.
    let mut at = if format >= 3 { usize::try_from(u64_at(32).ok_or_else(bad)?).map_err(|_| bad())? } else { 96 };
    let count = u32_at(at).ok_or_else(bad)?;
    at += 4;
    let mut files = BTreeMap::new();
    for _ in 0..count.min(1_000_000) {
        let len = usize::try_from(u32_at(at).ok_or_else(bad)?).map_err(|_| bad())?;
        at = at.checked_add(4).ok_or_else(bad)?;
        let raw = data.get(at..at.checked_add(len).ok_or_else(bad)?).ok_or_else(bad)?;
        at += len;
        let offset = u64_at(at).ok_or_else(bad)?;
        let size = u64_at(at + 8).ok_or_else(bad)?;
        let file_flags = u32_at(at + 32).ok_or_else(bad)?;
        at += 36;
        let start = if flags & 2 != 0 { offset.checked_add(base) } else { Some(offset) };
        let range = start
            .zip(start.and_then(|s| s.checked_add(size)))
            .and_then(|(s, e)| Some(usize::try_from(s).ok()?..usize::try_from(e).ok()?))
            .filter(|r| r.end <= data.len())
            .ok_or_else(bad)?;
        let path = String::from_utf8_lossy(raw).trim_end_matches('\0').trim_start_matches("res://").to_string();
        if file_flags & 1 == 0 {
            files.insert(path, range);
        }
    }
    Ok(files)
}

/// The pack's level: a text scene under `Levels/` other than the game's utilities, the largest if
/// there are several.
fn level_scene(files: &BTreeMap<String, Range<usize>>) -> Result<String, String> {
    let scenes = || files.iter().filter(|(p, _)| p.ends_with(".tscn"));
    let level = scenes().filter(|(p, _)| p.starts_with("Levels/") && !p.contains("/Utilities/")).max_by_key(|(_, r)| r.len());
    if let Some((path, _)) = level.or_else(|| scenes().max_by_key(|(_, r)| r.len())) {
        return Ok(path.clone());
    }
    if files.keys().any(|p| p.ends_with(".scn.remap") || p.ends_with(".scn")) {
        return Err("the pack holds its level as a binary scene (.scn), which MapCraft cannot read; export it with text scenes".into());
    }
    Err("the pack holds no scene".into())
}

struct Scene {
    path: String,
    sections: Vec<Section>,
    /// `ext_resource` ids to paths without `res://`.
    ext: BTreeMap<String, String>,
    /// `sub_resource` ids to section indices.
    sub: BTreeMap<String, usize>,
}

struct Doc<'a> {
    data: &'a [u8],
    files: BTreeMap<String, Range<usize>>,
    scenes: Vec<Scene>,
    by_path: BTreeMap<String, usize>,
}

/// A node with its instanced scene merged in; each property remembers the scene its resources live in.
struct Node {
    path: String,
    name: String,
    kind: String,
    props: Vec<(String, Value, usize)>,
    instance: Option<String>,
    global: DAffine3,
    hidden: bool,
}

impl Node {
    fn prop(&self, key: &str) -> Option<(&Value, usize)> {
        self.props.iter().rev().find(|(k, _, _)| k == key).map(|(_, v, s)| (v, *s))
    }

    fn value(&self, key: &str) -> Option<&Value> {
        self.prop(key).map(|(v, _)| v)
    }

    fn num(&self, key: &str, default: f64) -> f64 {
        self.value(key).and_then(Value::num).filter(|n| n.is_finite()).unwrap_or(default)
    }

    fn meta(&self, key: &str) -> Option<&Value> {
        self.value(&format!("metadata/{key}"))
    }

    fn parent_path(&self) -> &str {
        self.path.rsplit_once('/').map_or("", |(p, _)| p)
    }
}

impl<'a> Doc<'a> {
    fn add(&mut self, path: String, sections: Vec<Section>, depth: usize) -> usize {
        let mut ext = BTreeMap::new();
        let mut sub = BTreeMap::new();
        for (i, s) in sections.iter().enumerate() {
            let id = s.attr_text("id").unwrap_or_default().to_string();
            match s.tag.as_str() {
                "ext_resource" => {
                    ext.insert(id, s.attr_text("path").unwrap_or_default().trim_start_matches("res://").to_string());
                }
                "sub_resource" => {
                    sub.insert(id, i);
                }
                _ => {}
            }
        }
        let index = self.scenes.len();
        self.by_path.insert(path.clone(), index);
        self.scenes.push(Scene { path, sections, ext: ext.clone(), sub });
        for res in ext.into_values().filter(|p| p.ends_with(".tscn") || p.ends_with(".tres")) {
            self.load(&res, depth + 1);
        }
        index
    }

    /// Reads a text scene or resource from the pack once, with the resources it uses.
    fn load(&mut self, path: &str, depth: usize) -> Option<usize> {
        if let Some(i) = self.by_path.get(path) {
            return Some(*i);
        }
        if depth > MAX_NESTING {
            return None;
        }
        let bytes = self.files.get(path).and_then(|r| self.data.get(r.clone()))?;
        let sections = tscn::parse(std::str::from_utf8(bytes).ok()?).ok()?;
        Some(self.add(path.to_string(), sections, depth))
    }

    /// The `[sub_resource]` or loaded `[resource]` a `SubResource` or `ExtResource` value names.
    fn resolve(&self, scene: usize, v: &Value) -> Option<(usize, &Section)> {
        let s = self.scenes.get(scene)?;
        if let Some(args) = v.call("SubResource") {
            let i = *s.sub.get(args.first()?.text()?)?;
            return Some((scene, s.sections.get(i)?));
        }
        let at = *self.by_path.get(self.ext_path(scene, v)?)?;
        let section = self.scenes.get(at)?.sections.iter().find(|x| x.tag == "resource")?;
        Some((at, section))
    }

    /// A resource's class: a `[sub_resource]`'s own, or the `[gd_resource]` header's for a file.
    fn kind<'s>(&'s self, scene: usize, section: &'s Section) -> &'s str {
        let header = if section.tag == "resource" { self.scenes.get(scene).and_then(|s| s.sections.first()) } else { Some(section) };
        header.and_then(|h| h.attr_text("type")).unwrap_or_default()
    }

    fn ext_path(&self, scene: usize, v: &Value) -> Option<&str> {
        let id = v.call("ExtResource")?.first()?.text()?;
        self.scenes.get(scene)?.ext.get(id).map(String::as_str)
    }

    /// Every node of `scene` in order, with instanced scenes merged in and global transforms.
    fn nodes(&self, scene: usize) -> Result<Vec<Node>, String> {
        let mut out = Vec::new();
        let mut index = BTreeMap::new();
        self.add_nodes(scene, None, 0, &mut out, &mut index)?;
        Ok(out)
    }

    fn add_nodes(&self, scene: usize, root: Option<Node>, depth: usize, out: &mut Vec<Node>, index: &mut BTreeMap<String, usize>) -> Result<(), String> {
        let Some(s) = self.scenes.get(scene) else { return Ok(()) };
        let root_path = root.as_ref().map(|r| r.path.clone()).unwrap_or_default();
        let mut root = root;
        for section in s.sections.iter().filter(|x| x.tag == "node") {
            if out.len() >= MAX_NODES {
                return Err("the scene has too many nodes".into());
            }
            let props: Vec<(String, Value, usize)> = section.props.iter().map(|(k, v)| (k.clone(), v.clone(), scene)).collect();
            let name = section.attr_text("name").unwrap_or_default().to_string();
            let kind = section.attr_text("type").unwrap_or_default().to_string();
            let Some(parent) = section.attr_text("parent") else {
                // The scene's root, merged into the node that instances it.
                let mut node = root.take().unwrap_or(Node {
                    path: String::new(),
                    name: name.clone(),
                    kind: String::new(),
                    props: Vec::new(),
                    instance: None,
                    global: DAffine3::IDENTITY,
                    hidden: false,
                });
                let overrides = std::mem::take(&mut node.props);
                node.props = props;
                node.props.extend(overrides);
                if node.kind.is_empty() {
                    node.kind = kind;
                }
                node.global *= local(&node);
                node.hidden |= node.value("visible") == Some(&Value::Bool(false));
                index.insert(node.path.clone(), out.len());
                out.push(node);
                continue;
            };
            let parent_path = join(&root_path, parent);
            let (parent_global, parent_hidden) =
                index.get(&parent_path).and_then(|i| out.get(*i)).map_or((DAffine3::IDENTITY, false), |p| (p.global, p.hidden));
            let path = join(&parent_path, &name);
            let instance = section.attr("instance").and_then(|v| self.ext_path(scene, v)).map(str::to_string);
            let node = Node { path, name, kind, props, instance: instance.clone(), global: parent_global, hidden: parent_hidden };
            match instance.as_deref().filter(|p| !point_utility(p)).and_then(|p| self.by_path.get(p)) {
                Some(sub) if depth < MAX_NESTING => self.add_nodes(*sub, Some(node), depth + 1, out, index)?,
                _ => {
                    let mut node = node;
                    node.global *= local(&node);
                    node.hidden |= node.value("visible") == Some(&Value::Bool(false));
                    index.insert(node.path.clone(), out.len());
                    out.push(node);
                }
            }
        }
        Ok(())
    }
}

fn join(parent: &str, child: &str) -> String {
    match (parent, child) {
        (p, ".") => p.to_string(),
        ("", c) => c.to_string(),
        (p, c) => format!("{p}/{c}"),
    }
}

/// Resolves a node path such as `../Path3D` against the node at `from`.
fn relative(from: &str, path: &str) -> String {
    let mut parts: Vec<&str> = if path.starts_with('/') { Vec::new() } else { from.split('/').filter(|p| !p.is_empty()).collect() };
    for part in path.split('/').filter(|p| !p.is_empty()) {
        match part {
            "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

fn local(node: &Node) -> DAffine3 {
    if let Some(t) = node.value("transform").and_then(affine) {
        return t;
    }
    DAffine3::from_translation(node.value("position").and_then(vec3).unwrap_or_default())
}

fn affine(v: &Value) -> Option<DAffine3> {
    let n = v.call("Transform3D")?.iter().map(Value::num).collect::<Option<Vec<f64>>>()?;
    let &[a, b, c, d, e, f, g, h, i, x, y, z] = n.as_slice() else { return None };
    // Godot writes the basis row by row.
    let t = DAffine3 { matrix3: DMat3::from_cols(DVec3::new(a, d, g), DVec3::new(b, e, h), DVec3::new(c, f, i)), translation: DVec3::new(x, y, z) };
    t.is_finite().then_some(t)
}

fn vec3(v: &Value) -> Option<DVec3> {
    let n = v.call("Vector3")?.iter().map(Value::num).collect::<Option<Vec<f64>>>()?;
    let &[x, y, z] = n.as_slice() else { return None };
    Some(DVec3::new(x, y, z)).filter(|p| p.is_finite())
}

fn vec2(v: &Value) -> Option<DVec2> {
    let n = v.call("Vector2")?.iter().map(Value::num).collect::<Option<Vec<f64>>>()?;
    let &[x, y] = n.as_slice() else { return None };
    Some(DVec2::new(x, y)).filter(|p| p.is_finite())
}

/// An sRGB color from `Color(r, g, b, a)`, 0 to 1.
fn color(v: &Value) -> Option<DVec3> {
    let n = v.call("Color")?.iter().map(Value::num).collect::<Option<Vec<f64>>>()?;
    match n.as_slice() {
        &[r, g, b, ..] => Some(DVec3::new(r, g, b).clamp(DVec3::ZERO, DVec3::ONE)),
        _ => None,
    }
}

/// Godot's Y up meters to map space: Z up, in inches.
fn to_map(p: DVec3) -> DVec3 {
    DVec3::new(p.x, -p.z, p.y) * UNITS_PER_METER
}

fn to_map_dir(d: DVec3) -> DVec3 {
    DVec3::new(d.x, -d.z, d.y)
}

/// Coordinates and angles, to a thousandth.
fn fmt(v: f64) -> String {
    trimmed((v * 1000.0).round() / 1000.0)
}

/// Settings such as fog density, which can be tiny.
fn fmt_setting(v: f64) -> String {
    trimmed((v * 1e6).round() / 1e6)
}

fn trimmed(r: f64) -> String {
    if r == 0.0 {
        "0".into()
    } else if r.fract() == 0.0 && r.abs() < 1e15 {
        format!("{}", r as i64)
    } else {
        format!("{r}")
    }
}

fn fmt_vec(v: DVec3) -> String {
    format!("{} {} {}", fmt(v.x), fmt(v.y), fmt(v.z))
}

fn fmt_color(c: DVec3) -> String {
    let c = (c * 255.0).round();
    format!("{} {} {}", c.x as u8, c.y as u8, c.z as u8)
}

/// Map `angles` (pitch below the horizon, yaw, roll) for a node facing down its -Z like Godot nodes do.
fn angles(t: &DAffine3) -> String {
    let f = to_map_dir(-t.matrix3.z_axis).normalize_or_zero();
    let yaw = if f.x.hypot(f.y) < 1e-9 { 0.0 } else { f.y.atan2(f.x).to_degrees() };
    let pitch = (-f.z).atan2(f.x.hypot(f.y)).to_degrees();
    format!("{} {} 0", fmt(pitch), fmt(yaw))
}

/// Utility scenes that SurfsUp's FGD places as point entities; the game instances them itself.
fn point_utility(path: &str) -> bool {
    utility_class(path).is_some_and(|(_, point)| point)
}

/// The FGD class for one of SurfsUp's `Levels/Utilities` scenes, and whether it is a point entity.
fn utility_class(path: &str) -> Option<(&'static str, bool)> {
    match path.rsplit('/').next()? {
        "kill_box.tscn" => Some(("kill_box", false)),
        "bhop_zone.tscn" => Some(("bhop_zone", false)),
        "starting_platform.tscn" => Some(("starting_platform", true)),
        "landing_platform.tscn" => Some(("finish_line", true)),
        _ => None,
    }
}

/// The FGD class of an SDK zone kind (`sdk_zone` metadata).
fn zone_class(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "start" => "zone_start",
        "finish" => "zone_finish",
        "kill" => "kill_box",
        "split" => "zone_split",
        "checkpoint" => "zone_checkpoint",
        "teleport" => "teleport_zone",
        "booster" => "zone_booster",
        "respawn" => "zone_respawn",
        "point_tp" => "zone_point_tp",
        "marker" => "teleport_destination",
        _ => return None,
    })
}

/// SurfsUp's default zone node names, which older maps rely on instead of metadata.
fn named_zone(name: &str) -> Option<&'static str> {
    Some(match name {
        "StartZone" => "zone_start",
        "FinishZone" => "zone_finish",
        "KillBox" => "kill_box",
        "SplitZone" => "zone_split",
        "CheckpointArea" => "zone_checkpoint",
        "TeleportZone" => "teleport_zone",
        "Booster" => "zone_booster",
        "SetRespawn" => "zone_respawn",
        "PointTP" => "zone_point_tp",
        _ => return None,
    })
}

/// Zone settings that name another node, which then becomes a `teleport_destination`.
const LINKS: [&str; 3] = ["next_stage", "respawn", "target"];

fn is_csg(kind: &str) -> bool {
    kind.starts_with("CSG")
}

struct Materials {
    names: BTreeMap<(usize, String), String>,
    infos: Vec<MaterialInfo>,
}

struct Builder<'a> {
    doc: &'a Doc<'a>,
    nodes: &'a [Node],
    by_path: BTreeMap<&'a str, usize>,
    map: &'a str,
    groups: Vec<Group>,
    group_index: BTreeMap<String, usize>,
    entities: Vec<Entity>,
    materials: Materials,
    skipped: BTreeMap<String, usize>,
    brush_count: usize,
    clamped: usize,
    zones: usize,
    /// Kill zones bounded by a world boundary plane, at this map height, built once the map's extent is known.
    boundaries: Vec<(usize, f64)>,
    /// Extent of everything but zones, in map space.
    bounds: Option<(DVec3, DVec3)>,
}

impl<'a> Builder<'a> {
    fn new(doc: &'a Doc<'a>, nodes: &'a [Node], map: &'a str) -> Self {
        let by_path = nodes.iter().enumerate().map(|(i, n)| (n.path.as_str(), i)).collect();
        Builder {
            doc,
            nodes,
            by_path,
            map,
            groups: Vec::new(),
            group_index: BTreeMap::new(),
            entities: Vec::new(),
            materials: Materials { names: BTreeMap::new(), infos: Vec::new() },
            skipped: BTreeMap::new(),
            brush_count: 0,
            clamped: 0,
            zones: 0,
            boundaries: Vec::new(),
            bounds: None,
        }
    }

    fn skip(&mut self, what: &str) {
        *self.skipped.entry(what.to_string()).or_default() += 1;
    }

    fn parent(&self, node: &Node) -> Option<&'a Node> {
        self.by_path.get(node.parent_path()).and_then(|i| self.nodes.get(*i)).filter(|_| !node.path.is_empty())
    }

    fn descendants(&self, node: &'a Node) -> impl Iterator<Item = &'a Node> + use<'a> {
        let prefix = format!("{}/", node.path);
        let all = node.path.is_empty();
        let nodes: &'a [Node] = self.nodes;
        nodes.iter().filter(move |n| !n.path.is_empty() && (all || n.path.starts_with(&prefix)))
    }

    fn run(&mut self) -> Result<(), String> {
        let baked = self.baked_collision();
        let links: BTreeSet<String> = self
            .nodes
            .iter()
            .flat_map(|n| LINKS.iter().filter_map(|k| n.meta(k).and_then(Value::text)).filter(|t| !t.is_empty()).map(str::to_string))
            .collect();
        let mut consumed: Vec<String> = Vec::new();
        let mut named: BTreeSet<String> = BTreeSet::new();
        let mut environment = false;
        for node in self.nodes {
            if node.hidden || consumed.iter().any(|c| node.path.starts_with(&format!("{c}/"))) {
                continue;
            }
            if self.brush_count > MAX_BRUSHES {
                return Err("the map makes too many brushes".into());
            }
            let kind = node.kind.as_str();
            let instance_class = node.instance.as_deref().and_then(utility_class);
            let meta_class = node.meta("sdk_zone").and_then(Value::text).and_then(zone_class);
            if let Some((class, true)) = instance_class {
                self.point(class, node, Vec::new());
                consumed.push(node.path.clone());
            } else if meta_class == Some("teleport_destination") {
                self.point("teleport_destination", node, vec![("targetname".into(), node.name.clone())]);
                named.insert(node.name.clone());
            } else if let Some(class) = meta_class.or(instance_class.map(|c| c.0)).or_else(|| (kind == "Area3D").then(|| named_zone(&node.name)).flatten()) {
                self.zone(class, node);
                named.insert(node.name.clone());
                consumed.push(node.path.clone());
            } else if node.name == "SpawnPoint" && node.parent_path().is_empty() && !node.path.is_empty() {
                self.point("info_player_start", node, Vec::new());
            } else if is_csg(kind) && self.parent(node).is_none_or(|p| !is_csg(&p.kind)) {
                self.csg(node, &baked);
            } else if kind == "MeshInstance3D" {
                self.mesh(node);
            } else if kind == "StaticBody3D" {
                self.static_body(node);
            } else if kind == "WorldEnvironment" || kind == "DirectionalLight3D" {
                environment = true;
            } else if matches!(
                kind,
                "Label3D" | "OmniLight3D" | "SpotLight3D" | "GPUParticles3D" | "CPUParticles3D" | "Decal" | "Sprite3D" | "AnimatedSprite3D"
            ) {
                self.skip(kind);
            } else if kind.is_empty() && node.instance.is_some() {
                self.skip("scenes that are not in the pack");
            }
        }
        for name in links.difference(&named) {
            if let Some(node) = self.nodes.iter().find(|n| &n.name == name) {
                self.point("teleport_destination", node, vec![("targetname".into(), name.clone())]);
            }
        }
        if environment {
            self.environment();
        }
        self.finish_boundaries();
        Ok(())
    }

    /// Bounds of static collision meshes, in Godot space: CSG with collision off inside one of them
    /// collides in the game, since SDK maps bake ramp collision into those meshes.
    fn baked_collision(&mut self) -> Vec<(DVec3, DVec3)> {
        let mut out = Vec::new();
        for body in self.nodes.iter().filter(|n| n.kind == "StaticBody3D" && !n.hidden) {
            for shape in self.descendants(body).filter(|n| n.kind == "CollisionShape3D") {
                let Some((s, section)) = shape.prop("shape").and_then(|(v, s)| self.doc.resolve(s, v)) else { continue };
                if self.doc.kind(s, section) != "ConcavePolygonShape3D" {
                    continue;
                }
                let pts = section.prop("data").map(Value::numbers).unwrap_or_default();
                let world: Vec<DVec3> = pts.as_chunks::<3>().0.iter().map(|c| shape.global.transform_point3(DVec3::from_array(*c))).collect();
                if let Some(b) = aabb(&world) {
                    out.push(b);
                }
            }
        }
        out
    }

    /// The group a node's brushes go in: its parent's when the parent is a plain spatial node
    /// such as `FinishLine`, otherwise its own.
    fn group_name(&self, node: &Node) -> String {
        match self.parent(node) {
            Some(p) if !p.path.is_empty() && p.kind == "Node3D" => p.name.clone(),
            _ => node.name.clone(),
        }
    }

    fn group(&mut self, name: &str) -> Option<&mut Group> {
        let i = match self.group_index.get(name) {
            Some(i) => *i,
            None => {
                self.groups.push(Group { name: name.to_string(), brushes: Vec::new(), entities: Vec::new() });
                self.group_index.insert(name.to_string(), self.groups.len() - 1);
                self.groups.len() - 1
            }
        };
        self.groups.get_mut(i)
    }

    /// A convex brush around Godot space points, or none when they enclose no volume.
    fn hull(&mut self, points: &[DVec3], material: &str) -> Option<Brush> {
        let mut clamped = false;
        let pts: Vec<DVec3> = points
            .iter()
            .map(|p| {
                let m = to_map(*p);
                let c = m.clamp(DVec3::splat(-WORLD), DVec3::splat(WORLD));
                clamped |= c != m;
                c
            })
            .collect();
        if !pts.iter().all(|p| p.is_finite()) || !encloses(&pts) {
            return None;
        }
        self.clamped += usize::from(clamped);
        self.brush_count += 1;
        let (lo, hi) = aabb(&pts)?;
        if material != TRIGGER {
            self.bounds = Some(self.bounds.map_or((lo, hi), |(a, b)| (a.min(lo), b.max(hi))));
        }
        Some(Brush::Hull(Hull { points: pts.iter().map(|p| Vec3 { x: p.x, y: p.y, z: p.z }).collect(), material: material.to_string(), surfaces: Vec::new() }))
    }

    fn point(&mut self, class: &str, node: &Node, mut properties: Vec<(String, String)>) {
        properties.insert(0, ("origin".into(), fmt_vec(to_map(node.global.translation))));
        properties.insert(1, ("angles".into(), angles(&node.global)));
        self.entities.push(Entity { classname: class.into(), properties, brushes: Vec::new() });
    }

    fn zone(&mut self, class: &str, node: &'a Node) {
        let mut properties = vec![("targetname".to_string(), node.name.clone())];
        for (key, value, _) in &node.props {
            let Some(key) = key.strip_prefix("metadata/").filter(|k| *k != "sdk_zone") else { continue };
            if let Some(text) = key_text(value) {
                properties.retain(|(k, _)| k != key);
                properties.push((key.to_string(), text));
            }
        }
        let mut brushes = Vec::new();
        let shapes: Vec<&Node> = std::iter::once(node).chain(self.descendants(node)).filter(|n| n.kind == "CollisionShape3D" && !n.hidden).collect();
        for shape in shapes {
            if shape.value("disabled") == Some(&Value::Bool(true)) {
                continue;
            }
            match self.shape_points(shape) {
                Shape::Hulls(hulls) => brushes.extend(hulls.iter().filter_map(|h| self.hull(h, TRIGGER))),
                Shape::Boundary(height) => self.boundaries.push((self.entities.len(), height)),
                Shape::Skip(what) => self.skip(what),
            }
        }
        if brushes.is_empty() && !self.boundaries.iter().any(|(e, _)| *e == self.entities.len()) {
            self.skip("zones without a usable shape");
            return;
        }
        self.zones += 1;
        self.entities.push(Entity { classname: class.into(), properties, brushes });
    }

    fn shape_points(&self, shape: &Node) -> Shape {
        let Some((s, section)) = shape.prop("shape").and_then(|(v, s)| self.doc.resolve(s, v)) else { return Shape::Skip("collision shapes without a shape") };
        let t = shape.global;
        let num = |k: &str, d: f64| section.prop(k).and_then(Value::num).filter(|n| n.is_finite()).unwrap_or(d);
        let place = |pts: Vec<DVec3>| Shape::Hulls(vec![pts.into_iter().map(|p| t.transform_point3(p)).collect()]);
        match self.doc.kind(s, section) {
            "BoxShape3D" => place(box_points(section.prop("size").and_then(vec3).unwrap_or(DVec3::ONE))),
            "ConvexPolygonShape3D" => {
                let n = section.prop("points").map(Value::numbers).unwrap_or_default();
                place(n.as_chunks::<3>().0.iter().map(|c| DVec3::from_array(*c)).collect())
            }
            "CylinderShape3D" => place(cylinder_points(num("radius", 0.5), num("radius", 0.5), num("height", 2.0), 16)),
            "CapsuleShape3D" => place(cylinder_points(num("radius", 0.5), num("radius", 0.5), num("height", 2.0), 16)),
            "SphereShape3D" => place(sphere_points(num("radius", 0.5))),
            "WorldBoundaryShape3D" => {
                let plane = section.prop("plane").map(Value::numbers).unwrap_or_else(|| vec![0.0, 1.0, 0.0, 0.0]);
                let (normal, d) = match plane.as_slice() {
                    &[a, b, c, d] => (DVec3::new(a, b, c), d),
                    _ => (DVec3::Y, 0.0),
                };
                let up = t.matrix3 * normal;
                if up.normalize_or_zero().y < 0.99 {
                    return Shape::Skip("world boundaries that are not level");
                }
                Shape::Boundary(to_map(t.transform_point3(normal * d)).z)
            }
            "ConcavePolygonShape3D" => Shape::Skip("concave collision meshes in zones"),
            _ => Shape::Skip("collision shapes of other kinds"),
        }
    }

    /// Kill zones under a world boundary become slabs under the plane, as wide as the map plus a margin.
    fn finish_boundaries(&mut self) {
        let Some((lo, hi)) = self.bounds else { return };
        for (entity, top) in std::mem::take(&mut self.boundaries) {
            let (lo, hi) = (lo - DVec3::splat(BOUNDARY_MARGIN), hi + DVec3::splat(BOUNDARY_MARGIN));
            let corners: Vec<DVec3> = box_corners(DVec3::new(lo.x, lo.y, top - BOUNDARY_THICKNESS), DVec3::new(hi.x, hi.y, top))
                .into_iter()
                .map(|p| DVec3::new(p.x, p.z, -p.y) / UNITS_PER_METER)
                .collect();
            if let Some(brush) = self.hull(&corners, TRIGGER)
                && let Some(e) = self.entities.get_mut(entity)
            {
                e.brushes.push(brush);
            }
        }
    }

    /// A CSG shape and the union children merged into it.
    fn csg(&mut self, root: &'a Node, baked: &[(DVec3, DVec3)]) {
        let mut parts: Vec<(Vec<DVec3>, String)> = Vec::new();
        for node in std::iter::once(root).chain(self.descendants(root)) {
            if node.hidden || !is_csg(&node.kind) {
                continue;
            }
            if node.path != root.path && node.num("operation", 0.0) != 0.0 {
                self.skip("CSG subtractions and intersections");
                continue;
            }
            let material = self.material(node.prop("material"));
            match self.csg_hulls(node) {
                Some(hulls) => parts.extend(hulls.into_iter().map(|h| (h, material.clone()))),
                None if node.kind != "CSGCombiner3D" => self.skip(&node.kind),
                None => {}
            }
        }
        let all: Vec<DVec3> = parts.iter().flat_map(|(h, _)| h.iter().copied()).collect();
        let solid = root.value("use_collision") == Some(&Value::Bool(true))
            || aabb(&all).is_some_and(|(lo, hi)| baked.iter().any(|(a, b)| lo.cmpge(*a - DVec3::splat(0.5)).all() && hi.cmple(*b + DVec3::splat(0.5)).all()));
        let brushes: Vec<Brush> = parts.iter().filter_map(|(h, m)| self.hull(h, m)).collect();
        self.place(root, brushes, solid);
    }

    fn place(&mut self, node: &Node, brushes: Vec<Brush>, solid: bool) {
        if brushes.is_empty() {
            return;
        }
        let name = self.group_name(node);
        let Some(group) = self.group(&name) else { return };
        if solid {
            group.brushes.extend(brushes);
        } else if let Some(e) = group.entities.iter_mut().find(|e| e.classname == "func_illusionary") {
            e.brushes.extend(brushes);
        } else {
            group.entities.push(Entity { classname: "func_illusionary".into(), properties: Vec::new(), brushes });
        }
    }

    fn csg_hulls(&self, node: &Node) -> Option<Vec<Vec<DVec3>>> {
        let t = node.global;
        let place = |pts: Vec<DVec3>| vec![pts.into_iter().map(|p| t.transform_point3(p)).collect()];
        match node.kind.as_str() {
            "CSGBox3D" => Some(place(box_points(node.value("size").and_then(vec3).unwrap_or(DVec3::ONE)))),
            "CSGCylinder3D" => {
                let r = node.num("radius", 0.5);
                let top = if node.value("cone") == Some(&Value::Bool(true)) { 0.0 } else { r };
                Some(place(cylinder_points(top, r, node.num("height", 2.0), node.num("sides", 8.0).clamp(3.0, 64.0) as usize)))
            }
            "CSGPolygon3D" => self.polygon(node),
            _ => None,
        }
    }

    /// A `CSGPolygon3D`: each convex piece of its polygon swept from frame to frame.
    fn polygon(&self, node: &Node) -> Option<Vec<Vec<DVec3>>> {
        let flat = node.value("polygon").map(Value::numbers).unwrap_or_else(|| vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0]);
        let polygon: Vec<DVec2> = flat.as_chunks::<2>().0.iter().map(|c| DVec2::from_array(*c)).collect();
        if polygon.len() > MAX_POLYGON_POINTS {
            return None;
        }
        let pieces = convex_pieces(&polygon);
        let frames = match node.num("mode", 0.0) as i64 {
            0 => vec![DAffine3::IDENTITY, DAffine3::from_translation(DVec3::new(0.0, 0.0, -node.num("depth", 1.0)))],
            1 => {
                let sides = node.num("spin_sides", 8.0).clamp(3.0, 128.0) as usize;
                let degrees = node.num("spin_degrees", 360.0).clamp(1.0, 360.0);
                (0..=sides).map(|i| DAffine3::from_rotation_y((degrees * i as f64 / sides as f64).to_radians())).collect()
            }
            _ => self.path_frames(node)?,
        };
        let mut out = Vec::new();
        for pair in frames.windows(2) {
            let &[a, b] = pair else { continue };
            for piece in &pieces {
                let ring = |f: DAffine3| piece.iter().map(move |p| node.global.transform_point3(f.transform_point3(DVec3::new(p.x, p.y, 0.0))));
                out.push(ring(a).chain(ring(b)).collect());
            }
        }
        Some(out)
    }

    /// Frames along a path-mode polygon's `Path3D`, in the polygon's own space.
    fn path_frames(&self, node: &Node) -> Option<Vec<DAffine3>> {
        let target = relative(&node.path, node.value("path_node")?.text()?);
        let path = self.nodes.get(*self.by_path.get(target.as_str())?)?;
        let (_, curve) = path.prop("curve").and_then(|(v, s)| self.doc.resolve(s, v))?;
        let data = curve.prop("_data")?;
        let raw = data.get("points").map(Value::numbers).unwrap_or_default();
        let tilts = data.get("tilts").map(Value::numbers).unwrap_or_default();
        let controls: Vec<[DVec3; 3]> =
            raw.as_chunks::<9>().0.iter().map(|&[a, b, c, d, e, f, g, h, i]| [DVec3::new(a, b, c), DVec3::new(d, e, f), DVec3::new(g, h, i)]).collect();
        let interval = node.num("path_interval", 1.0).max(0.01);
        let by_count = node.num("path_interval_type", 0.0) == 1.0;
        let mut points: Vec<(DVec3, f64)> = Vec::new();
        for (i, pair) in controls.windows(2).enumerate() {
            let &[[_, out, p0], [inn, _, p1]] = pair else { continue };
            let (t0, t1) = (tilts.get(i).copied().unwrap_or(0.0), tilts.get(i + 1).copied().unwrap_or(0.0));
            let steps = if out == DVec3::ZERO && inn == DVec3::ZERO {
                1
            } else if by_count {
                interval.clamp(1.0, 64.0) as usize
            } else {
                ((p0.distance(p0 + out) + (p0 + out).distance(p1 + inn) + (p1 + inn).distance(p1)) / interval).ceil().clamp(1.0, 64.0) as usize
            };
            for k in 0..steps {
                let s = k as f64 / steps as f64;
                let u = 1.0 - s;
                let p = p0 * u * u * u + (p0 + out) * 3.0 * u * u * s + (p1 + inn) * 3.0 * u * s * s + p1 * s * s * s;
                points.push((p, t0 + (t1 - t0) * s));
            }
            if points.len() > MAX_PATH_POINTS {
                return None;
            }
        }
        if let Some(last) = controls.last() {
            points.push((last[2], tilts.get(controls.len() - 1).copied().unwrap_or(0.0)));
        }
        if node.value("path_joined") == Some(&Value::Bool(true))
            && let Some(first) = points.first().copied()
        {
            points.push(first);
        }
        let base = if node.value("path_local") == Some(&Value::Bool(true)) { local(path) } else { node.global.inverse() * path.global };
        let points: Vec<(DVec3, f64)> = points.into_iter().map(|(p, tilt)| (base.transform_point3(p), tilt)).collect();
        let points = simplify(&points);
        let rotation = node.num("path_rotation", 2.0) as i64;
        let mut up = DVec3::Y;
        let mut frames = Vec::with_capacity(points.len());
        for (i, (p, tilt)) in points.iter().enumerate() {
            let prev = points.get(i.wrapping_sub(1)).map_or(*p, |q| q.0);
            let next = points.get(i + 1).map_or(*p, |q| q.0);
            let dir = (next - prev).normalize_or_zero();
            let basis = match rotation {
                0 => DMat3::IDENTITY,
                1 => looking_at(dir, DVec3::Y),
                _ => {
                    let flat = (up - dir * up.dot(dir)).normalize_or_zero();
                    up = if flat == DVec3::ZERO { up } else { flat };
                    let tilted = if dir == DVec3::ZERO { up } else { DMat3::from_axis_angle(dir, *tilt) * up };
                    looking_at(dir, tilted)
                }
            };
            frames.push(DAffine3 { matrix3: basis, translation: *p });
        }
        Some(frames)
    }

    fn mesh(&mut self, node: &'a Node) {
        let Some((scene, section)) = node.prop("mesh").and_then(|(v, s)| self.doc.resolve(s, v)) else {
            self.skip("meshes that are not Godot primitives");
            return;
        };
        let num = |k: &str, d: f64| section.prop(k).and_then(Value::num).filter(|n| n.is_finite()).unwrap_or(d);
        // Planes become one unit thick slabs.
        let thin = 1.0 / UNITS_PER_METER;
        let mesh_kind = self.doc.kind(scene, section);
        let points = match mesh_kind {
            "BoxMesh" => box_points(section.prop("size").and_then(vec3).unwrap_or(DVec3::ONE)),
            "PlaneMesh" | "QuadMesh" => {
                let quad = mesh_kind == "QuadMesh";
                let size = section.prop("size").and_then(vec2).unwrap_or(if quad { DVec2::ONE } else { DVec2::splat(2.0) });
                let offset = section.prop("center_offset").and_then(vec3).unwrap_or_default();
                let facing = section.prop("orientation").and_then(Value::num).map_or(if quad { 2 } else { 1 }, |n| n as i64);
                let extent = match facing {
                    0 => DVec3::new(thin, size.y, size.x),
                    2 => DVec3::new(size.x, size.y, thin),
                    _ => DVec3::new(size.x, thin, size.y),
                };
                box_points(extent).into_iter().map(|p| p + offset).collect()
            }
            "CylinderMesh" => cylinder_points(num("top_radius", 0.5), num("bottom_radius", 0.5), num("height", 2.0), 16),
            "PrismMesh" => {
                let size = section.prop("size").and_then(vec3).unwrap_or(DVec3::ONE);
                let apex = (num("left_to_right", 0.5) - 0.5) * size.x;
                let h = size / 2.0;
                [-h.z, h.z].iter().flat_map(|z| [DVec3::new(-h.x, -h.y, *z), DVec3::new(h.x, -h.y, *z), DVec3::new(apex, h.y, *z)]).collect()
            }
            _ => {
                self.skip("meshes that are not Godot primitives");
                return;
            }
        };
        let material =
            node.prop("material_override").or_else(|| node.prop("surface_material_override/0")).or_else(|| section.prop("material").map(|v| (v, scene)));
        let material = self.material(material);
        let pts: Vec<DVec3> = points.into_iter().map(|p| node.global.transform_point3(p)).collect();
        let solid = self.ancestors(node).any(|a| a.kind == "StaticBody3D");
        let brushes: Vec<Brush> = self.hull(&pts, &material).into_iter().collect();
        self.place(node, brushes, solid);
    }

    fn ancestors(&self, node: &Node) -> impl Iterator<Item = &'a Node> + '_ {
        let mut path = node.path.clone();
        std::iter::from_fn(move || {
            if path.is_empty() {
                return None;
            }
            path = path.rsplit_once('/').map_or(String::new(), |(p, _)| p.to_string());
            self.by_path.get(path.as_str()).and_then(|i| self.nodes.get(*i))
        })
    }

    /// Collision without a mesh of its own becomes player clip; bodies with meshes collide through them.
    fn static_body(&mut self, body: &'a Node) {
        if self.descendants(body).any(|n| n.kind == "MeshInstance3D" || is_csg(&n.kind)) {
            return;
        }
        let shapes: Vec<&Node> = self.descendants(body).filter(|n| n.kind == "CollisionShape3D" && !n.hidden).collect();
        for shape in shapes {
            match self.shape_points(shape) {
                Shape::Hulls(hulls) => {
                    let brushes: Vec<Brush> = hulls.iter().filter_map(|h| self.hull(h, CLIP)).collect();
                    self.place(body, brushes, true);
                }
                Shape::Skip("concave collision meshes in zones") => self.skip("collision meshes, rebuilt from the CSG they were baked from"),
                Shape::Skip(what) => self.skip(what),
                Shape::Boundary(_) => self.skip("world boundaries outside zones"),
            }
        }
    }

    /// The sky and sun, as the MapCraft Godot addon's `light_environment` keys.
    fn environment(&mut self) {
        let mut properties: Vec<(String, String)> = Vec::new();
        let env = self.nodes.iter().find(|n| n.kind == "WorldEnvironment" && !n.hidden);
        let resolve = |v: Option<(&Value, usize)>| v.and_then(|(v, s)| self.doc.resolve(s, v));
        let environment = resolve(env.and_then(|n| n.prop("environment")));
        let sky = environment.and_then(|(s, e)| resolve(e.prop("sky").map(|v| (v, s))));
        let sky_material =
            sky.and_then(|(s, k)| resolve(k.prop("sky_material").map(|v| (v, s)))).filter(|(s, m)| self.doc.kind(*s, m) == "ProceduralSkyMaterial");
        let sky_color = |key: &str, default: DVec3| sky_material.and_then(|(_, m)| m.prop(key)).and_then(color).unwrap_or(default);
        let env_prop = |key: &str| environment.and_then(|(_, e)| e.prop(key));
        // Godot's own defaults, for keys the scene leaves unset.
        properties.push(("sky_top_color".into(), fmt_color(sky_color("sky_top_color", DVec3::new(0.385, 0.454, 0.55)))));
        properties.push(("sky_horizon_color".into(), fmt_color(sky_color("sky_horizon_color", DVec3::new(0.6463, 0.6558, 0.6708)))));
        properties.push(("ground_color".into(), fmt_color(sky_color("ground_bottom_color", DVec3::new(0.2, 0.169, 0.133)))));
        properties.push(("ambient_energy".into(), fmt_setting(env_prop("ambient_light_energy").and_then(Value::num).unwrap_or(1.0))));
        let fog = env_prop("fog_enabled") == Some(&Value::Bool(true));
        properties.push(("fog_density".into(), fmt_setting(if fog { env_prop("fog_density").and_then(Value::num).unwrap_or(0.01) } else { 0.0 })));
        properties.push(("fog_color".into(), fmt_color(env_prop("fog_light_color").and_then(color).unwrap_or(DVec3::new(0.518, 0.553, 0.608)))));
        let sun = self.nodes.iter().find(|n| n.kind == "DirectionalLight3D" && !n.hidden);
        let mut origin = DVec3::ZERO;
        if let Some(sun) = sun {
            origin = to_map(sun.global.translation);
            properties.push(("angles".into(), angles(&sun.global)));
            properties.push(("sun_color".into(), fmt_color(sun.value("light_color").and_then(color).unwrap_or(DVec3::ONE))));
            properties.push(("sun_energy".into(), fmt_setting(sun.num("light_energy", 1.0))));
            properties.push(("shadows".into(), if sun.value("shadow_enabled") == Some(&Value::Bool(true)) { "1" } else { "0" }.into()));
        }
        properties.insert(0, ("origin".into(), fmt_vec(origin)));
        self.entities.push(Entity { classname: "light_environment".into(), properties, brushes: Vec::new() });
    }

    /// The MapCraft material for a Godot material: an SDK texture when it uses one from
    /// `Assets/Textures`, otherwise a swatch of its color named after the map.
    fn material(&mut self, value: Option<(&Value, usize)>) -> String {
        let Some((value, scene)) = value else { return self.swatch("default".into(), DVec3::splat(0.8)) };
        let key = (scene, format!("{value:?}"));
        if let Some(name) = self.materials.names.get(&key) {
            return name.clone();
        }
        let name = self.material_name(value, scene);
        self.materials.names.insert(key, name.clone());
        name
    }

    fn material_name(&mut self, value: &Value, scene: usize) -> String {
        if let Some(path) = self.doc.ext_path(scene, value).and_then(sdk_texture) {
            return path;
        }
        let id = self.doc.ext_path(scene, value).map(|p| p.rsplit('/').next().unwrap_or(p).split('.').next().unwrap_or(p).to_string());
        let id = id.or_else(|| value.call("SubResource").and_then(|a| a.first()?.text().map(str::to_string))).unwrap_or_else(|| "material".into());
        let Some((s, section)) = self.doc.resolve(scene, value) else { return self.swatch(id, DVec3::splat(0.8)) };
        if let Some(path) = section.prop("albedo_texture").and_then(|t| self.doc.ext_path(s, t)).and_then(sdk_texture) {
            return path;
        }
        let rgb = match self.doc.kind(s, section) {
            "ShaderMaterial" => self.shader_color(s, section),
            _ => section.prop("albedo_color").and_then(color).unwrap_or(DVec3::ONE),
        };
        self.swatch(id, rgb)
    }

    /// A shader material's color: its first color parameter, else the brightest color its code
    /// writes to `ALBEDO`, which is linear.
    fn shader_color(&self, scene: usize, material: &Section) -> DVec3 {
        if let Some(c) = material.props.iter().filter(|(k, _)| k.starts_with("shader_parameter/")).find_map(|(_, v)| color(v)) {
            return c;
        }
        let code = material.prop("shader").and_then(|v| self.doc.resolve(scene, v)).and_then(|(_, s)| s.prop("code")).and_then(Value::text).unwrap_or_default();
        code.split("ALBEDO")
            .skip(1)
            .filter_map(|rest| {
                let args = rest.trim_start().strip_prefix('=')?.trim_start().strip_prefix("vec3(")?;
                let n: Vec<f64> = args.split(')').next()?.split(',').map(|x| x.trim().parse::<f64>().ok()).collect::<Option<_>>()?;
                let &[r, g, b] = n.as_slice() else { return None };
                Some(DVec3::new(r, g, b).clamp(DVec3::ZERO, DVec3::ONE).powf(1.0 / 2.2))
            })
            .max_by(|a, b| a.element_sum().total_cmp(&b.element_sum()))
            .unwrap_or(DVec3::splat(0.5))
    }

    fn swatch(&mut self, id: String, rgb: DVec3) -> String {
        let id: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
        let name = format!("{}/{id}", self.map);
        if !self.materials.infos.iter().any(|m| m.name == name) {
            let c = (rgb * 255.0).round();
            let px = [c.x as u8, c.y as u8, c.z as u8, 255];
            self.materials.infos.push(MaterialInfo {
                name: name.clone(),
                image: None,
                color: Some((px[0], px[1], px[2])),
                pixels: Some(Image { width: 4, height: 4, rgba: px.repeat(16) }),
                size: None,
                pbr: None,
            });
        }
        name
    }
}

/// A material name from a texture or material in SurfsUp's `Assets/Textures`, which the SDK lists by
/// that path without the extension.
fn sdk_texture(path: &str) -> Option<String> {
    let rest = path.strip_prefix("Assets/Textures/")?;
    Some(rest.rsplit_once('.').map_or(rest, |(p, _)| p).to_string())
}

/// A zone setting as an FGD key value; vectors turn from Godot's axes to the map's.
fn key_text(v: &Value) -> Option<String> {
    match v {
        Value::Bool(b) => Some(if *b { "1" } else { "0" }.into()),
        Value::Num(n) if n.is_finite() => Some(fmt_setting(*n)),
        Value::Str(s) => Some(s.clone()),
        Value::Call(name, _) if name == "Vector3" => vec3(v).map(|d| fmt_vec(to_map_dir(d))),
        Value::Call(name, args) if name == "NodePath" || name == "StringName" => args.first().and_then(Value::text).map(str::to_string),
        _ => None,
    }
}

enum Shape {
    Hulls(Vec<Vec<DVec3>>),
    /// A level world boundary at this map height.
    Boundary(f64),
    Skip(&'static str),
}

fn box_corners(lo: DVec3, hi: DVec3) -> Vec<DVec3> {
    (0..8).map(|i| DVec3::new(if i & 1 == 0 { lo.x } else { hi.x }, if i & 2 == 0 { lo.y } else { hi.y }, if i & 4 == 0 { lo.z } else { hi.z })).collect()
}

fn box_points(size: DVec3) -> Vec<DVec3> {
    box_corners(-size.abs() / 2.0, size.abs() / 2.0)
}

/// A Godot cylinder along Y, centered on its origin; a zero radius end is a single point.
fn cylinder_points(top: f64, bottom: f64, height: f64, sides: usize) -> Vec<DVec3> {
    let ring = |r: f64, y: f64| -> Vec<DVec3> {
        if r <= 0.0 {
            return vec![DVec3::new(0.0, y, 0.0)];
        }
        (0..sides).map(|i| (i as f64 / sides as f64) * std::f64::consts::TAU).map(|a| DVec3::new(r * a.cos(), y, r * a.sin())).collect()
    };
    let mut pts = ring(top.abs(), height / 2.0);
    pts.extend(ring(bottom.abs(), -height / 2.0));
    pts
}

fn sphere_points(r: f64) -> Vec<DVec3> {
    let mut pts = vec![DVec3::Y * r, -DVec3::Y * r];
    for ring in 1..8 {
        let a = std::f64::consts::PI * ring as f64 / 8.0;
        pts.extend(cylinder_points(r * a.sin(), 0.0, 0.0, 16).into_iter().take(16).map(|p| p + DVec3::Y * r * a.cos()));
    }
    pts
}

fn aabb(points: &[DVec3]) -> Option<(DVec3, DVec3)> {
    let first = *points.first()?;
    Some(points.iter().fold((first, first), |(lo, hi), p| (lo.min(*p), hi.max(*p))))
}

/// Whether the points span a volume of at least a unit in each direction.
fn encloses(points: &[DVec3]) -> bool {
    let Some(&a) = points.first() else { return false };
    let Some(&b) = points.iter().max_by(|p, q| p.distance_squared(a).total_cmp(&q.distance_squared(a))) else { return false };
    let line = (b - a).normalize_or_zero();
    let off_line = |p: &DVec3| (*p - a - line * (*p - a).dot(line)).length();
    let Some(&c) = points.iter().max_by(|p, q| off_line(p).total_cmp(&off_line(q))) else { return false };
    let normal = line.cross(c - a).normalize_or_zero();
    a.distance(b) > 0.5 && off_line(&c) > 0.5 && points.iter().any(|p| (*p - a).dot(normal).abs() > 0.5)
}

/// Godot's `looking_at`: -Z along `dir`, Y as close to `up` as it can be.
fn looking_at(dir: DVec3, up: DVec3) -> DMat3 {
    let z = -dir.normalize_or_zero();
    let mut x = up.cross(z).normalize_or_zero();
    if x == DVec3::ZERO {
        x = DVec3::X;
    }
    DMat3::from_cols(x, z.cross(x), z)
}

/// Drops path points that sit within tolerance of the line between the points kept around them.
fn simplify(points: &[(DVec3, f64)]) -> Vec<(DVec3, f64)> {
    let tolerance = PATH_TOLERANCE / UNITS_PER_METER;
    let turn = PATH_TURN_DEGREES.to_radians().cos();
    let (Some(first), Some(last)) = (points.first(), points.last()) else { return Vec::new() };
    let mut out = vec![*first];
    let mut anchor = 0;
    for end in 2..points.len() {
        let (Some(a), Some(b)) = (points.get(anchor), points.get(end)) else { break };
        let chord = (b.0 - a.0).normalize_or_zero();
        let fits = points.get(anchor + 1..end).unwrap_or_default().iter().enumerate().all(|(k, p)| {
            let off = p.0 - a.0;
            let step = points.get(anchor + k + 2).map_or(chord, |q| (q.0 - p.0).normalize_or_zero());
            (off - chord * off.dot(chord)).length() <= tolerance && step.dot(chord) >= turn && (p.1 - a.1).abs() < 1e-3
        });
        if !fits && let Some(kept) = points.get(end - 1) {
            out.push(*kept);
            anchor = end - 1;
        }
    }
    if points.len() > 1 {
        out.push(*last);
    }
    out.dedup_by(|a, b| a.0.distance(b.0) < 1e-6);
    out
}

/// A simple polygon as convex pieces: itself when convex, else triangles by ear clipping.
fn convex_pieces(polygon: &[DVec2]) -> Vec<Vec<DVec2>> {
    let n = polygon.len();
    let at = |i: usize| polygon.get(i % n.max(1)).copied().unwrap_or_default();
    if n < 3 {
        return Vec::new();
    }
    let sign = (0..n).map(|i| at(i).perp_dot(at(i + 1))).sum::<f64>();
    if sign.abs() < 1e-12 {
        return Vec::new();
    }
    let sign = sign.signum();
    if (0..n).all(|i| (at(i + 1) - at(i)).perp_dot(at(i + 2) - at(i + 1)) * sign >= -1e-12) {
        return vec![polygon.to_vec()];
    }
    let mut left: Vec<DVec2> = polygon.to_vec();
    let mut out = Vec::new();
    while left.len() > 3 {
        let m = left.len();
        let get = |i: usize| left.get(i % m).copied().unwrap_or_default();
        let inside = |p: DVec2, a: DVec2, b: DVec2, c: DVec2| {
            (b - a).perp_dot(p - a) * sign >= 0.0 && (c - b).perp_dot(p - b) * sign >= 0.0 && (a - c).perp_dot(p - c) * sign >= 0.0
        };
        let ear = (0..m).find(|&i| {
            let (a, b, c) = (get(i + m - 1), get(i), get(i + 1));
            (b - a).perp_dot(c - b) * sign > 1e-12 && (0..m).filter(|j| (j + 1) % m != i && *j != i && (i + 1) % m != *j).all(|j| !inside(get(j), a, b, c))
        });
        let Some(i) = ear else { break };
        out.push(vec![get(i + m - 1), get(i), get(i + 1)]);
        left.remove(i);
    }
    if left.len() == 3 {
        out.push(left);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCENE: &str = r#"[gd_scene format=3]

[ext_resource type="PackedScene" path="res://Levels/Utilities/kill_box.tscn" id="kill"]

[sub_resource type="StandardMaterial3D" id="pink"]
albedo_color = Color(1, 0, 0.5, 1)

[sub_resource type="BoxShape3D" id="box"]
size = Vector3(4, 2, 4)

[sub_resource type="Curve3D" id="path"]
_data = {
"points": PackedVector3Array(0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, -1, -10, 0, 0, 0, 0, 0, 0, 0, -2, -20, 0, 0, 0, 0, 0, 0, 0, -3, -30),
"tilts": PackedFloat32Array(0, 0, 0, 0)
}
point_count = 4

[sub_resource type="ConcavePolygonShape3D" id="baked"]
data = PackedVector3Array(-20, -10, 5, 20, 20, 5, 0, 0, -40)

[sub_resource type="ProceduralSkyMaterial" id="skymat"]
sky_top_color = Color(1, 0, 0, 1)

[sub_resource type="Sky" id="sky"]
sky_material = SubResource("skymat")

[sub_resource type="Environment" id="env"]
sky = SubResource("sky")
ambient_light_energy = 2.0

[node name="Map" type="Node3D"]

[node name="Sun" type="DirectionalLight3D" parent="."]
transform = Transform3D(1, 0, 0, 0, 0, 1, 0, -1, 0, 0, 10, 0)
light_energy = 1.5

[node name="WorldEnvironment" type="WorldEnvironment" parent="."]
environment = SubResource("env")

[node name="SpawnPoint" type="Marker3D" parent="."]
transform = Transform3D(1, 0, 0, 0, 1, 0, 0, 0, 1, 1, 2, 3)

[node name="Level" type="Node" parent="."]

[node name="Deck" type="CSGBox3D" parent="Level"]
use_collision = true
size = Vector3(2, 1, 4)
material = SubResource("pink")

[node name="StartZone" type="Area3D" parent="Level"]

[node name="Shape" type="CollisionShape3D" parent="Level/StartZone"]
transform = Transform3D(1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 2, 0)
shape = SubResource("box")

[node name="Teleport" type="Area3D" parent="Level"]
metadata/sdk_zone = "teleport"
metadata/next_stage = "Stage2"
metadata/kill_velocity = true

[node name="Shape" type="CollisionShape3D" parent="Level/Teleport"]
shape = SubResource("box")

[node name="Stage2" type="Marker3D" parent="Level"]
transform = Transform3D(1, 0, 0, 0, 1, 0, 0, 0, 1, 50, 0, 0)

[node name="RampBody" type="StaticBody3D" parent="Level"]

[node name="Col" type="CollisionShape3D" parent="Level/RampBody"]
shape = SubResource("baked")

[node name="RampPath" type="Path3D" parent="Level"]
curve = SubResource("path")

[node name="Ramp" type="CSGPolygon3D" parent="Level"]
polygon = PackedVector2Array(-2, 0, 2, 0, 0, 3)
mode = 2
path_node = NodePath("../RampPath")
path_rotation = 1
path_local = true

[node name="Arrow" type="Node3D" parent="Level"]
transform = Transform3D(1, 0, 0, 0, 1, 0, 0, 0, 1, 100, 0, 0)

[node name="Chevron" type="CSGPolygon3D" parent="Level/Arrow"]
polygon = PackedVector2Array(-14, 11, 0, 4.2, 14, 11, 14, 6.8, 0, 0, -14, 6.8)
depth = 3.0

[node name="Sign" type="Label3D" parent="Level"]

[node name="KillBox" parent="Level" instance=ExtResource("kill")]
transform = Transform3D(1, 0, 0, 0, 1, 0, 0, 0, 1, 0, -100, 0)
"#;

    const KILL_BOX: &str = r#"[gd_scene format=3]

[sub_resource type="WorldBoundaryShape3D" id="plane"]

[node name="KillBox" type="Area3D"]
collision_mask = 2

[node name="CollisionShape3D" type="CollisionShape3D" parent="."]
shape = SubResource("plane")
"#;

    /// A Godot 4.4 style pack: header, file data, then the directory.
    fn pack(files: &[(&str, &[u8])]) -> Vec<u8> {
        let base = 104u64;
        let mut body: Vec<u8> = Vec::new();
        let mut dir = (files.len() as u32).to_le_bytes().to_vec();
        for (path, bytes) in files {
            let mut name = format!("res://{path}").into_bytes();
            name.resize(name.len().div_ceil(4) * 4, 0);
            dir.extend((name.len() as u32).to_le_bytes());
            dir.extend(&name);
            dir.extend((body.len() as u64).to_le_bytes());
            dir.extend((bytes.len() as u64).to_le_bytes());
            dir.extend([0u8; 16]);
            dir.extend(0u32.to_le_bytes());
            body.extend(*bytes);
        }
        let mut out = b"GDPC".to_vec();
        for n in [3u32, 4, 7, 2, 2] {
            out.extend(n.to_le_bytes());
        }
        out.extend(base.to_le_bytes());
        out.extend((base + body.len() as u64).to_le_bytes());
        out.resize(base as usize, 0);
        out.extend(body);
        out.extend(dir);
        out
    }

    fn find<'a>(out: &'a Output, class: &str) -> &'a Entity {
        out.entities.iter().find(|e| e.classname == class).unwrap()
    }

    fn key<'a>(e: &'a Entity, k: &str) -> &'a str {
        e.properties.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str()).unwrap()
    }

    fn group<'a>(out: &'a Output, name: &str) -> &'a Group {
        out.groups.iter().find(|g| g.name == name).unwrap()
    }

    #[test]
    fn converts_a_sdk_scene() {
        let pck = pack(&[("Levels/Utilities/kill_box.tscn", KILL_BOX.as_bytes()), ("Levels/test.tscn", SCENE.as_bytes())]);
        let imported = import("test.pck", &pck).unwrap();
        let out = &imported.output;

        assert_eq!(key(find(out, "info_player_start"), "origin"), "39.37 -118.11 78.74", "Godot Y up meters become Z up inches");
        let start = find(out, "zone_start");
        assert_eq!(start.brushes.len(), 1, "a zone found by its SDK name gets its box");
        let teleport = find(out, "teleport_zone");
        assert_eq!((key(teleport, "next_stage"), key(teleport, "kill_velocity")), ("Stage2", "1"), "sdk_zone metadata carries the settings");
        let dest = find(out, "teleport_destination");
        assert_eq!((key(dest, "targetname"), key(dest, "origin")), ("Stage2", "1968.5 0 0"), "a node a teleport names becomes its destination");

        let kill = find(out, "kill_box");
        let Some(Brush::Hull(slab)) = kill.brushes.first() else { panic!("the kill box has no brush") };
        let top = slab.points.iter().map(|p| p.z).fold(f64::MIN, f64::max);
        assert!((top + 3937.0).abs() < 0.01, "the instanced scene's world boundary becomes a slab under its plane");

        let env = find(out, "light_environment");
        assert_eq!((key(env, "sky_top_color"), key(env, "ambient_energy")), ("255 0 0", "2"));
        assert_eq!((key(env, "angles"), key(env, "sun_energy")), ("90 0 0", "1.5"), "a sun shining straight down");

        let deck = group(out, "Deck");
        let Some(Brush::Hull(h)) = deck.brushes.first() else { panic!("the deck has no brush") };
        assert_eq!(h.material, "test/pink");
        assert!(out.materials.iter().any(|m| m.name == "test/pink" && m.color == Some((255, 0, 128))));
        assert_eq!(group(out, "Ramp").brushes.len(), 1, "a straight path is one brush, solid since its collision was baked");
        let arrow = group(out, "Arrow");
        assert!(arrow.brushes.is_empty());
        assert_eq!(
            arrow.entities.first().map(|e| (e.classname.as_str(), e.brushes.len())),
            Some(("func_illusionary", 4)),
            "a concave polygon becomes triangles"
        );
        assert!(imported.log.iter().any(|l| l.contains("1 Label3D")));
    }

    #[test]
    fn a_ramp_profile_stands_on_its_path() {
        let imported = import("test.tscn", SCENE.as_bytes()).unwrap();
        let Some(Brush::Hull(h)) = group(&imported.output, "Ramp").brushes.first() else { panic!("no ramp") };
        let top = h.points.iter().map(|p| p.z).fold(f64::MIN, f64::max);
        assert!(top > 2.0 * UNITS_PER_METER, "the polygon's +Y points up from the path");
        assert!(h.points.iter().all(|p| p.x.abs() <= 2.0 * UNITS_PER_METER + 0.01), "and its +X across it");
    }

    #[test]
    fn damaged_input_is_an_error() {
        assert!(import("x.pck", b"not a map").is_err());
        let mut pck = pack(&[("Levels/test.tscn", SCENE.as_bytes())]);
        pck.truncate(pck.len() - 10);
        assert!(import("x.pck", &pck).is_err(), "a cut off directory");
        let mut far = pack(&[("Levels/test.tscn", SCENE.as_bytes())]);
        far.splice(32..40, u64::MAX.to_le_bytes());
        assert!(import("x.pck", &far).is_err(), "a directory past the end");
        assert!(import("x.pck", &pack(&[("a.txt", b"hi")])).is_err(), "a pack without a scene");
    }

    #[test]
    fn simplifies_paths_and_splits_polygons() {
        let line: Vec<(DVec3, f64)> = (0..10).map(|i| (DVec3::new(0.0, 0.0, f64::from(i)), 0.0)).collect();
        assert_eq!(simplify(&line).len(), 2);
        let bend: Vec<(DVec3, f64)> = (0..10).map(|i| (DVec3::new(f64::from(i * i) * 0.1, 0.0, f64::from(i)), 0.0)).collect();
        assert!(simplify(&bend).len() > 2, "a curve keeps its bends");
        let square = [DVec2::ZERO, DVec2::X, DVec2::ONE, DVec2::Y];
        assert_eq!(convex_pieces(&square).len(), 1);
        assert!(convex_pieces(&[DVec2::ZERO, DVec2::X, DVec2::X * 2.0]).is_empty(), "a flat polygon has no pieces");
    }
}
