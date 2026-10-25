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
}

/// A file format the plugin imports.
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
    Manifest { id: m.id, name: m.name, version: m.version, commands, materials: m.materials, importers }
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
            }
            Some(b)
        }
    }
}

/// A texture axis, or `None` when it would divide by zero or is not a number.
fn tex_axis(a: &t::TexAxis) -> Option<TexAxis> {
    let axis = vec3(&a.axis);
    (axis.is_finite() && axis.length_squared() > 1e-12 && a.scale.is_finite() && a.scale.abs() > 1e-9 && a.offset.is_finite())
        .then_some(TexAxis { axis, offset: a.offset, scale: a.scale })
}

fn contents(brushes: Vec<t::Brush>, entities: Vec<t::Entity>, ctx: &Context, budget: &mut usize) -> Vec<Fragment> {
    let leaf = |kind| Fragment { kind, children: Vec::new() };
    let take = |n: &mut usize| *n > 0 && {
        *n -= 1;
        true
    };
    let mut frags: Vec<Fragment> = Vec::new();
    for b in brushes {
        if !take(budget) {
            break;
        }
        frags.extend(brush(b, ctx).map(|b| leaf(NodeKind::Brush(b))));
    }
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

/// The pictures among `materials` whose size and pixel count agree, up to 8192 on a side.
pub(crate) fn images(materials: Vec<t::MaterialInfo>) -> Vec<crate::ImportedMaterial> {
    materials
        .into_iter()
        .filter_map(|m| {
            let p = m.pixels?;
            let ok = (1..=8192).contains(&p.width) && (1..=8192).contains(&p.height) && p.rgba.len() == p.width as usize * p.height as usize * 4;
            ok.then_some(crate::ImportedMaterial { name: m.name, width: p.width, height: p.height, rgba: p.rgba })
        })
        .collect()
}
