use mapcraft_format::Fragment;
use mapcraft_geom::DVec3;
use mapcraft_map::{Brush, Entity, Face, Group, NodeKind, TexAxis, TexDefaults};
use serde_json::{Value as Json, json};

use crate::bindings::mapcraft::plugin::types as t;

/// Most nodes one call may return; more are dropped.
const MAX_NODES: usize = 100_000;
/// cos(8 degrees): how close a hull face's normal must be to a surface hint.
const SURFACE_COS: f64 = 0.990_268;

#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub commands: Vec<CommandSpec>,
    pub materials: bool,
    pub importers: Vec<ImporterSpec>,
    pub exporters: Vec<ImporterSpec>,
    pub validates: bool,
    /// Lowercase model file extensions the plugin reads.
    pub models: Vec<String>,
}

/// A file format the plugin imports or exports.
#[derive(Clone, Debug, PartialEq)]
pub struct ImporterSpec {
    pub id: String,
    pub label: String,
    pub extensions: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CommandSpec {
    pub id: String,
    pub label: String,
    pub menu: String,
    pub params: Vec<ParamSpec>,
    pub generator: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParamSpec {
    pub id: String,
    pub label: String,
    pub help: String,
    pub kind: ParamKind,
    pub initial: Json,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ParamKind {
    Number { min: f64, max: f64, step: f64 },
    Integer { min: i64, max: i64 },
    Toggle,
    Choice(Vec<String>),
    Text,
    Material,
}

/// Editor state passed to commands; `scale` is the texture scale given to new faces.
#[derive(Clone, Debug, PartialEq)]
pub struct Context {
    pub grid: f64,
    pub material: String,
    pub scale: f64,
}

impl CommandSpec {
    /// The parameters' starting values as a JSON object, the form generator groups store.
    pub fn defaults(&self) -> Json {
        Json::Object(self.params.iter().map(|p| (p.id.clone(), p.initial.clone())).collect())
    }
}

fn value_json(v: t::Value) -> Json {
    match v {
        t::Value::Number(n) if n.is_finite() => json!(n),
        t::Value::Number(_) => json!(0.0),
        t::Value::Integer(i) => json!(i),
        t::Value::Toggle(b) => json!(b),
        t::Value::Text(s) => json!(s),
    }
}

/// Ranges from an untrusted plugin, made safe to clamp with.
fn range(a: f64, b: f64) -> (f64, f64) {
    let a = if a.is_finite() { a } else { f64::MIN };
    let b = if b.is_finite() { b } else { f64::MAX };
    (a.min(b), a.max(b))
}

pub(crate) fn manifest(m: t::Manifest) -> Manifest {
    let commands = m
        .commands
        .into_iter()
        .map(|c| CommandSpec {
            id: c.id,
            label: c.label,
            menu: c.menu,
            generator: c.generator,
            params: c
                .params
                .into_iter()
                .map(|p| ParamSpec {
                    id: p.id,
                    label: p.label,
                    help: p.help,
                    initial: value_json(p.initial),
                    kind: match p.kind {
                        t::ParamKind::Number(r) => {
                            let (min, max) = range(r.min, r.max);
                            ParamKind::Number { min, max, step: if r.step.is_finite() && r.step > 0.0 { r.step } else { 1.0 } }
                        }
                        t::ParamKind::Integer(r) => ParamKind::Integer { min: r.min.min(r.max), max: r.min.max(r.max) },
                        t::ParamKind::Toggle => ParamKind::Toggle,
                        t::ParamKind::Choice(c) => ParamKind::Choice(c),
                        t::ParamKind::Text => ParamKind::Text,
                        t::ParamKind::Material => ParamKind::Material,
                    },
                })
                .collect(),
        })
        .collect();
    let importers = m.importers.into_iter().map(|i| ImporterSpec { id: i.id, label: i.label, extensions: i.extensions }).collect();
    let exporters = m.exporters.into_iter().map(|i| ImporterSpec { id: i.id, label: i.label, extensions: i.extensions }).collect();
    let models = m.models.into_iter().map(|e| e.trim_start_matches('.').to_ascii_lowercase()).filter(|e| !e.is_empty()).collect();
    Manifest { id: m.id, name: m.name, version: m.version, commands, materials: m.materials, importers, exporters, validates: m.validates, models }
}

/// Stored parameter values as the plugin expects them; missing or wrong values fall back to the
/// spec's starting value and numbers are clamped to their range.
pub(crate) fn params(spec: &CommandSpec, values: &Json) -> Vec<t::Param> {
    spec.params
        .iter()
        .map(|p| {
            let v = values.get(&p.id).unwrap_or(&p.initial);
            let value = match &p.kind {
                ParamKind::Number { min, max, .. } => {
                    t::Value::Number(v.as_f64().or(p.initial.as_f64()).filter(|n| n.is_finite()).unwrap_or(*min).clamp(*min, *max))
                }
                ParamKind::Integer { min, max } => {
                    let n = v.as_i64().or_else(|| v.as_f64().filter(|f| f.is_finite()).map(|f| f.round() as i64)).or(p.initial.as_i64());
                    t::Value::Integer(n.unwrap_or(*min).clamp(*min, *max))
                }
                ParamKind::Toggle => t::Value::Toggle(v.as_bool().or(p.initial.as_bool()).unwrap_or(false)),
                ParamKind::Choice(choices) => {
                    let s = v.as_str().filter(|s| choices.iter().any(|c| c == s)).or(p.initial.as_str()).or(choices.first().map(String::as_str));
                    t::Value::Text(s.unwrap_or_default().to_string())
                }
                ParamKind::Text | ParamKind::Material => t::Value::Text(v.as_str().or(p.initial.as_str()).unwrap_or_default().to_string()),
            };
            t::Param { id: p.id.clone(), value }
        })
        .collect()
}

fn vec3(p: &t::Vec3) -> DVec3 {
    DVec3::new(p.x, p.y, p.z)
}

fn brush(b: t::Brush, ctx: &Context) -> Option<Brush> {
    let tex = |m: &str| TexDefaults { material: if m.is_empty() { ctx.material.clone() } else { m.to_string() }, scale: ctx.scale };
    match b {
        t::Brush::Faces(faces) => {
            let faces: Vec<Face> = faces
                .iter()
                .map(|f| {
                    let mut face = Face::new([vec3(&f.points.0), vec3(&f.points.1), vec3(&f.points.2)], &tex(&f.material));
                    if let Some((u, v)) = &f.uv
                        && let (Some(u), Some(v)) = (tex_axis(u), tex_axis(v))
                    {
                        (face.u, face.v) = (u, v);
                    }
                    face
                })
                .collect();
            Brush::pruned(faces).ok()
        }
        t::Brush::Hull(h) => {
            let pts: Vec<DVec3> = h.points.iter().map(vec3).collect();
            let mut b = Brush::from_vertices(&pts, &[], &tex(&h.material)).ok()?;
            let hints: Vec<(DVec3, &str)> = h.surfaces.iter().map(|s| (vec3(&s.normal).normalize_or_zero(), s.material.as_str())).collect();
            for f in &mut b.faces {
                let Some(n) = f.plane().map(|p| p.normal) else { continue };
                let best = hints.iter().map(|(h, m)| (h.dot(n), *m)).filter(|(d, _)| *d > SURFACE_COS).max_by(|a, b| a.0.total_cmp(&b.0));
                if let Some((_, m)) = best {
                    f.material = tex(m).material;
                }
                f.align_to_face();
            }
            Some(b)
        }
    }
}

fn wit_vec3(p: DVec3) -> t::Vec3 {
    t::Vec3 { x: p.x, y: p.y, z: p.z }
}

fn solid(b: &Brush) -> t::Solid {
    let axis = |a: &TexAxis| t::TexAxis { axis: wit_vec3(a.axis), offset: a.offset, scale: a.scale };
    let faces = b
        .faces
        .iter()
        .map(|f| t::Face {
            points: (wit_vec3(f.points[0]), wit_vec3(f.points[1]), wit_vec3(f.points[2])),
            material: f.material.clone(),
            uv: Some((axis(&f.u), axis(&f.v))),
        })
        .collect();
    t::Solid { faces, vertices: b.vertices().unwrap_or_default().into_iter().map(wit_vec3).collect() }
}

/// The map as an exporter sees it, parents first: layers left out of exports, and anything outside
/// every active cordon, are dropped. `map` should already have prefabs and mirrors resolved.
pub(crate) fn map_data(map: &mapcraft_map::Map, name: &str) -> t::MapData {
    map_data_with_ids(map, name).0
}

/// [`map_data`] and the node id behind each entry of its `nodes`.
pub(crate) fn map_data_with_ids(map: &mapcraft_map::Map, name: &str) -> (t::MapData, Vec<mapcraft_map::NodeId>) {
    use mapcraft_geom::Aabb;
    let world = map.get(mapcraft_map::ROOT).and_then(|n| n.kind.entity()).map(|e| e.properties.clone()).unwrap_or_default();
    let cordons: Vec<Aabb> = map.cordons.iter().filter(|c| c.active).map(|c| c.bounds).collect();
    let inside = |b: &Aabb| cordons.is_empty() || cordons.iter().any(|c| c.intersects(b));
    let brush_inside = |id: &mapcraft_map::NodeId| map.get(*id).and_then(|n| n.kind.brush()).and_then(|b| b.bounds().ok()).is_some_and(|b| inside(&b));
    let roots = map.get(mapcraft_map::ROOT).map(|n| n.children.clone()).unwrap_or_default();
    let mut stack: Vec<(mapcraft_map::NodeId, Option<u32>)> = roots.into_iter().rev().map(|c| (c, None)).collect();
    let mut nodes = Vec::new();
    let mut ids = Vec::new();
    while let Some((id, parent)) = stack.pop() {
        let Some(n) = map.get(id) else { continue };
        let kind = match &n.kind {
            NodeKind::Layer(l) if l.omit_from_export => continue,
            NodeKind::Layer(l) => t::MapNodeKind::Layer(l.name.clone()),
            NodeKind::Group(g) => t::MapNodeKind::Group(g.name.clone()),
            NodeKind::Entity(e) => {
                let kept = if n.children.is_empty() { e.origin().is_none_or(|o| inside(&Aabb::new(o, o))) } else { n.children.iter().any(brush_inside) };
                if !kept {
                    continue;
                }
                t::MapNodeKind::Entity(t::MapEntity { classname: e.classname.clone(), properties: e.properties.clone() })
            }
            NodeKind::Brush(b) if brush_inside(&id) => t::MapNodeKind::Brush(solid(b)),
            _ => continue,
        };
        let index = u32::try_from(nodes.len()).unwrap_or(u32::MAX);
        nodes.push(t::MapNode { parent, kind });
        ids.push(id);
        stack.extend(n.children.iter().rev().map(|c| (*c, Some(index))));
    }
    (t::MapData { name: name.to_string(), world, nodes }, ids)
}

/// A texture axis, or `None` when it would divide by zero or is not a number.
fn tex_axis(a: &t::TexAxis) -> Option<TexAxis> {
    let axis = vec3(&a.axis);
    (axis.is_finite() && axis.length_squared() > 1e-12 && a.scale.is_finite() && a.scale.abs() > 1e-9 && a.offset.is_finite()).then_some(TexAxis {
        axis,
        offset: a.offset,
        scale: a.scale,
    })
}

fn contents(brushes: Vec<t::Brush>, entities: Vec<t::Entity>, ctx: &Context, budget: &mut usize) -> Vec<Fragment> {
    let leaf = |kind| Fragment { kind, children: Vec::new() };
    let take = |n: &mut usize| {
        *n > 0 && {
            *n -= 1;
            true
        }
    };
    let mut frags: Vec<Fragment> = Vec::new();
    let (mut hulls, mut faced) = (Vec::new(), Vec::new());
    for b in brushes {
        if !take(budget) {
            break;
        }
        let hull = matches!(b, t::Brush::Hull(_));
        if let Some(b) = brush(b, ctx) {
            if hull { hulls.push(b) } else { faced.push(b) }
        }
    }
    // Hulls carry no UVs, so a plugin's pieces get one texture running across them.
    mapcraft_map::wrap_runs(&mut hulls);
    frags.extend(faced.into_iter().chain(hulls).map(|b| leaf(NodeKind::Brush(b))));
    for e in entities {
        if !take(budget) {
            break;
        }
        let mut ent = Entity::new(e.classname);
        ent.properties = e.properties;
        let children = e.brushes.into_iter().take(MAX_NODES).filter_map(|b| brush(b, ctx)).map(|b| leaf(NodeKind::Brush(b))).collect();
        frags.push(Fragment { kind: NodeKind::Entity(ent), children });
    }
    frags
}

pub(crate) fn output(out: t::Output, ctx: &Context) -> Vec<Fragment> {
    let mut budget = MAX_NODES;
    let mut frags = contents(out.brushes, out.entities, ctx, &mut budget);
    for g in out.groups {
        let children = contents(g.brushes, g.entities, ctx, &mut budget);
        if !children.is_empty() {
            frags.push(Fragment { kind: NodeKind::Group(Group { name: g.name, ..Group::default() }), children });
        }
    }
    frags
}

/// A plugin's model with parts that do not add up dropped: indices past their positions, a UV
/// count that differs, or a texture that is not there.
pub(crate) fn model(m: t::Model) -> crate::PluginModel {
    const MAX_TRIANGLES: usize = 1 << 20;
    let textures: Vec<Option<crate::Picture>> = m.textures.into_iter().take(256).map(picture).collect();
    let mut triangles = 0usize;
    let parts = m
        .parts
        .into_iter()
        .filter(|p| p.uvs.len() == p.positions.len() && p.indices.len() % 3 == 0 && p.indices.iter().all(|i| (*i as usize) < p.positions.len()))
        .filter(|p| p.positions.iter().all(|v| v.x.is_finite() && v.y.is_finite() && v.z.is_finite()))
        .filter(|p| p.texture.is_none_or(|t| textures.get(t as usize).is_some_and(Option::is_some)))
        .take_while(|p| {
            triangles = triangles.saturating_add(p.indices.len() / 3);
            triangles <= MAX_TRIANGLES
        })
        .map(|p| crate::PluginModelPart {
            texture: p.texture.map(|t| t as usize),
            positions: p.positions.iter().map(vec3).collect(),
            uvs: p.uvs.into_iter().map(|(u, v)| [if u.is_finite() { u } else { 0.0 }, if v.is_finite() { v } else { 0.0 }]).collect(),
            indices: p.indices,
        })
        .collect();
    let textures = textures.into_iter().map(|t| t.unwrap_or(crate::Picture { width: 1, height: 1, rgba: vec![255; 4] })).collect();
    crate::PluginModel { parts, textures }
}

/// A picture whose size and pixel count agree, up to 8192 on a side.
pub(crate) fn picture(p: t::Image) -> Option<crate::Picture> {
    let ok = (1..=8192).contains(&p.width) && (1..=8192).contains(&p.height) && p.rgba.len() == p.width as usize * p.height as usize * 4;
    ok.then_some(crate::Picture { width: p.width, height: p.height, rgba: p.rgba })
}

/// A plugin's maps with values outside 0 to 1 clamped and non-numbers replaced by Godot's defaults.
pub(crate) fn pbr(p: t::Pbr) -> crate::Pbr {
    let unit = |v: f32, default: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { default };
    let map = |m: Option<t::ChannelMap>| {
        m.map(|m| crate::ChannelMap {
            path: m.path,
            channel: match m.channel {
                t::Channel::Red => crate::Channel::Red,
                t::Channel::Green => crate::Channel::Green,
                t::Channel::Blue => crate::Channel::Blue,
                t::Channel::Alpha => crate::Channel::Alpha,
                t::Channel::Gray => crate::Channel::Gray,
            },
        })
    };
    crate::Pbr {
        normal: p.normal,
        normal_dx: p.normal_dx,
        roughness: map(p.roughness),
        roughness_value: unit(p.roughness_value, 1.0),
        metallic: map(p.metallic),
        metallic_value: unit(p.metallic_value, 0.0),
        ao: map(p.ao),
    }
}

/// The pictures among `materials` whose size and pixel count agree, up to 8192 on a side, and
/// flat swatches for those that only give a color.
pub(crate) fn images(materials: Vec<t::MaterialInfo>) -> Vec<crate::ImportedMaterial> {
    materials
        .into_iter()
        .filter_map(|m| {
            let size = m.size.map(|(w, h)| [w, h]);
            let Some(p) = m.pixels else {
                let (r, g, b) = m.color?;
                return Some(crate::ImportedMaterial { name: m.name, width: 16, height: 16, rgba: [r, g, b, 255].repeat(16 * 16), size });
            };
            let p = picture(p)?;
            Some(crate::ImportedMaterial { name: m.name, width: p.width, height: p.height, rgba: p.rgba, size })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use mapcraft_geom::{Aabb, DVec3};
    use mapcraft_map::{Cordon, Entity, Layer, Map, ROOT};

    use super::*;

    #[test]
    fn exported_maps_drop_omitted_layers_and_what_cordons_leave_out() {
        let mut map = Map::new();
        let mut e = map.edit();
        let tex = TexDefaults { material: "wall".into(), scale: 1.0 };
        let kept = e.insert(ROOT, NodeKind::Layer(Layer { name: "Course".into(), ..Default::default() })).unwrap();
        let skipped = e.insert(ROOT, NodeKind::Layer(Layer { name: "Reference".into(), omit_from_export: true, ..Default::default() })).unwrap();
        e.insert(kept, NodeKind::Brush(Brush::cuboid(Aabb::new(DVec3::ZERO, DVec3::splat(64.0)), &tex))).unwrap();
        e.insert(kept, NodeKind::Brush(Brush::cuboid(Aabb::new(DVec3::splat(4096.0), DVec3::splat(4160.0)), &tex))).unwrap();
        e.insert(skipped, NodeKind::Brush(Brush::cuboid(Aabb::new(DVec3::ZERO, DVec3::splat(64.0)), &tex))).unwrap();
        let zone = e.insert(kept, NodeKind::Entity(Entity::new("zone_start"))).unwrap();
        e.insert(zone, NodeKind::Brush(Brush::cuboid(Aabb::new(DVec3::ZERO, DVec3::splat(32.0)), &tex))).unwrap();
        map.apply(e.finish());
        map.cordons = vec![Cordon { name: "start".into(), bounds: Aabb::new(DVec3::splat(-128.0), DVec3::splat(128.0)), active: true }];

        let data = map_data(&map, "surf_test");
        let kinds: Vec<&str> = data
            .nodes
            .iter()
            .map(|n| match &n.kind {
                t::MapNodeKind::Layer(_) => "layer",
                t::MapNodeKind::Group(_) => "group",
                t::MapNodeKind::Entity(_) => "entity",
                t::MapNodeKind::Brush(_) => "brush",
            })
            .collect();
        assert_eq!(kinds, ["layer", "brush", "entity", "brush"], "the far brush and the omitted layer are gone");
        assert!(data.nodes.iter().enumerate().all(|(i, n)| n.parent.is_none_or(|p| (p as usize) < i)), "parents come first");
        assert_eq!(data.nodes[3].parent, Some(2), "the zone's brush belongs to the zone");
        let t::MapNodeKind::Brush(b) = &data.nodes[1].kind else { panic!("a brush") };
        assert_eq!((b.faces.len(), b.vertices.len()), (6, 8));
        assert!(b.faces.iter().all(|f| f.uv.is_some()));
    }
}
