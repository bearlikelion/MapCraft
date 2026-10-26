use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mapcraft_geom::{DMat3, DMat4, DVec2, DVec3};
use mapcraft_map::{Entity, Face, FaceId, Map, NodeId, NodeKind, TexAxis};

use crate::model::{Model, entity_transform, placed_bounds};

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MeshVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub tint: [f32; 4],
    /// Where the texture's u grows, with w the sign that makes `cross(normal, tangent) * w` point up the picture.
    pub tangent: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LineVertex {
    pub pos: [f32; 3],
    pub color: [f32; 4],
}

/// A light for the Lit render mode, laid out as the shader reads it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Light {
    /// Position with w 1, or for a sun the direction toward it with w 0.
    pub pos: [f32; 4],
    /// Linear color times brightness; w is the falloff, numbered like Quake's `delay` key.
    pub color: [f32; 4],
    /// Spot direction, zero for an omni light; w is the falloff distance.
    pub spot: [f32; 4],
    /// Cosines of the spot's outer and inner half angles.
    pub cone: [f32; 4],
}

impl LineVertex {
    pub fn new(p: DVec3, color: [f32; 4]) -> Self {
        Self { pos: p.as_vec3().to_array(), color }
    }
}

/// Appends the 12 edges of a box.
pub fn box_lines(out: &mut Vec<LineVertex>, min: DVec3, max: DVec3, color: [f32; 4]) {
    let c = |x: bool, y: bool, z: bool| DVec3::new(if x { max.x } else { min.x }, if y { max.y } else { min.y }, if z { max.z } else { min.z });
    for (a, b) in [
        ((0, 0, 0), (1, 0, 0)),
        ((0, 1, 0), (1, 1, 0)),
        ((0, 0, 1), (1, 0, 1)),
        ((0, 1, 1), (1, 1, 1)),
        ((0, 0, 0), (0, 1, 0)),
        ((1, 0, 0), (1, 1, 0)),
        ((0, 0, 1), (0, 1, 1)),
        ((1, 0, 1), (1, 1, 1)),
        ((0, 0, 0), (0, 0, 1)),
        ((1, 0, 0), (1, 0, 1)),
        ((0, 1, 0), (0, 1, 1)),
        ((1, 1, 0), (1, 1, 1)),
    ] {
        let p = |t: (i32, i32, i32)| c(t.0 == 1, t.1 == 1, t.2 == 1);
        out.push(LineVertex::new(p(a), color));
        out.push(LineVertex::new(p(b), color));
    }
}

/// How entities look; the editor fills it from the SDK's FGDs.
pub trait EntityStyle {
    fn color(&self, classname: &str) -> Option<[u8; 3]>;
    /// Editor box of a point entity relative to its origin.
    fn size(&self, classname: &str) -> Option<(DVec3, DVec3)>;
    /// The loaded model a point entity shows in place of its box.
    fn model(&self, _: &Entity) -> Option<Arc<Model>> {
        None
    }
}

/// A point entity's world box: its placed model's bounds, else its editor box.
pub fn entity_bounds(style: &dyn EntityStyle, e: &Entity) -> (DVec3, DVec3) {
    if let Some(m) = style.model(e) {
        return placed_bounds(&m, &entity_transform(e));
    }
    let origin = e.origin().unwrap_or_default();
    let (min, max) = style.size(&e.classname).unwrap_or((DVec3::splat(-8.0), DVec3::splat(8.0)));
    (origin + min, origin + max)
}

pub struct NoStyle;

impl EntityStyle for NoStyle {
    fn color(&self, _: &str) -> Option<[u8; 3]> {
        None
    }

    fn size(&self, _: &str) -> Option<(DVec3, DVec3)> {
        None
    }
}

/// CPU-side geometry for the whole map, grouped by material.
#[derive(Clone, Debug, Default)]
pub struct SceneData {
    /// Material name to triangles; `""` is untextured.
    pub batches: BTreeMap<String, Vec<MeshVertex>>,
    pub edges: Vec<LineVertex>,
    /// Drawn over everything.
    pub selected_edges: Vec<LineVertex>,
    /// Faces with tool materials (triggers, clips), drawn see-through after everything else.
    pub translucent: Vec<MeshVertex>,
    pub lights: Vec<Light>,
    /// Linear ambient color a light entity's `_ambient` asks for.
    pub ambient: [f32; 3],
}

/// Hammer's convention: `tools/...` materials mark invisible brushes such as triggers and clips.
pub fn is_tool_material(name: &str) -> bool {
    name.get(..6).is_some_and(|p| p.eq_ignore_ascii_case("tools/"))
}

const TOOL_ALPHA: f32 = 0.3;

pub const WORLD_EDGE: [f32; 4] = [0.85, 0.85, 0.85, 1.0];
pub const SELECTED_EDGE: [f32; 4] = [1.0, 0.1, 0.1, 1.0];
const SELECTED_TINT: [f32; 4] = [1.0, 0.62, 0.55, 1.0];
const PICKED_FACE_EDGE: [f32; 4] = [0.0, 0.8, 1.0, 1.0];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const SOLID_ENTITY: [u8; 3] = [220, 30, 220];
const POINT_ENTITY: [u8; 3] = [80, 220, 80];

fn linear(c: [u8; 3]) -> [f32; 4] {
    let f = |v: u8| (f32::from(v) / 255.0).powf(2.2);
    [f(c[0]), f(c[1]), f(c[2]), 1.0]
}

/// A bright color of its own for each brush, so neighbors stand apart in 2D views (Hammer's
/// random solid colors), stable across rebuilds.
fn object_color(id: NodeId) -> [f32; 4] {
    let hue = (id.0 as f64 * 0.618_033_988_75).fract() * 6.0;
    let x = (1.0 - (hue % 2.0 - 1.0).abs()) as f32;
    let (r, g, b) = match hue as u32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        4 => (x, 0.0, 1.0),
        _ => (1.0, 0.0, x),
    };
    let mix = |c: f32| ((0.35 + 0.65 * c) * 255.0) as u8;
    linear([mix(r), mix(g), mix(b)])
}

/// The face's tangent for normal mapping, from its texture axes.
fn tangent(face: &Face, normal: DVec3) -> [f32; 4] {
    let flat = |a: &TexAxis| {
        let g = if a.scale != 0.0 { a.axis / a.scale } else { DVec3::ZERO };
        g - normal * g.dot(normal)
    };
    let (gu, gv) = (flat(&face.u), flat(&face.v));
    // Along the face, u grows where v stays put.
    let t = gv.cross(normal);
    let t = if t.dot(gu) < 0.0 { -t } else { t };
    let t = t.try_normalize().or(gu.try_normalize()).unwrap_or_else(|| normal.any_orthonormal_vector());
    let w = if normal.cross(t).dot(gv) > 0.0 { -1.0 } else { 1.0 };
    t.as_vec3().extend(w).to_array()
}

/// Builds the scene. `texture_size` gives the texel size used to normalize UVs; with `colorize`
/// each world brush's edges get their own color.
pub fn build_scene(
    map: &Map,
    selection: &BTreeSet<NodeId>,
    faces: &BTreeSet<(NodeId, FaceId)>,
    texture_size: &dyn Fn(&str) -> Option<[u32; 2]>,
    style: &dyn EntityStyle,
    colorize: bool,
) -> SceneData {
    let mut scene = SceneData::default();
    let visible = map.visible_nodes();
    let mut stack = vec![(mapcraft_map::ROOT, false, None::<[f32; 4]>)];
    while let Some((id, parent_selected, entity_color)) = stack.pop() {
        let Some(node) = map.get(id).filter(|_| visible.contains(&id)) else { continue };
        let selected = parent_selected || selection.contains(&id);
        let mut child_color = entity_color;
        match &node.kind {
            NodeKind::Brush(b) => {
                let Ok(solid) = b.solid() else { continue };
                let world = if colorize { object_color(id) } else { WORLD_EDGE };
                let edge = if selected { SELECTED_EDGE } else { entity_color.unwrap_or(world) };
                let displaced: BTreeMap<usize, Vec<DVec3>> = b.displaced(&solid).into_iter().collect();
                for (index, (face, winding)) in b.faces.iter().zip(&solid.faces).enumerate() {
                    let Some(w) = winding else { continue };
                    // A displaced brush shows only its displaced faces, as Source compiles it.
                    if b.has_displacement() && !displaced.contains_key(&index) {
                        continue;
                    }
                    let plane_normal = face.plane().map_or(DVec3::Z, |p| p.normal);
                    let normal = plane_normal.as_vec3().to_array();
                    let tangent = tangent(face, plane_normal);
                    let size = texture_size(&face.material).map_or(DVec2::splat(64.0), |s| DVec2::new(f64::from(s[0]), f64::from(s[1])));
                    let face_selected = faces.contains(&(id, face.id));
                    let tool = is_tool_material(&face.material);
                    // Picked faces keep their true color so materials can be judged; the outline marks them.
                    let tint = match (selected || face_selected, tool) {
                        (true, false) if selected && !face_selected => SELECTED_TINT,
                        (_, false) => WHITE,
                        (true, true) => [SELECTED_EDGE[0], SELECTED_EDGE[1], SELECTED_EDGE[2], TOOL_ALPHA + 0.15],
                        (false, true) => {
                            let c = entity_color.unwrap_or([0.8, 0.5, 0.1, 1.0]);
                            [c[0], c[1], c[2], TOOL_ALPHA]
                        }
                    };
                    let verts = if tool { &mut scene.translucent } else { scene.batches.entry(face.material.clone()).or_default() };
                    let edges = if selected || face_selected { &mut scene.selected_edges } else { &mut scene.edges };
                    let edge = if face_selected { PICKED_FACE_EDGE } else { edge };
                    if let (Some(grid), Some(d)) = (displaced.get(&index), &face.disp) {
                        let normals = d.normals(grid, plane_normal);
                        let v = |k: usize| MeshVertex {
                            pos: grid[k].as_vec3().to_array(),
                            normal: normals[k].as_vec3().to_array(),
                            uv: face.uv(grid[k], size).as_vec2().to_array(),
                            tint,
                            tangent,
                        };
                        verts.extend(d.triangles().into_iter().flat_map(|[a, b, c]| [v(a), v(b), v(c)]));
                        let n = d.size();
                        for k in 0..n * n {
                            for next in [(k % n + 1 < n).then_some(k + 1), (k + n < n * n).then_some(k + n)].into_iter().flatten() {
                                edges.push(LineVertex::new(grid[k], edge));
                                edges.push(LineVertex::new(grid[next], edge));
                            }
                        }
                        continue;
                    }
                    let v = |p: DVec3| MeshVertex { pos: p.as_vec3().to_array(), normal, uv: face.uv(p, size).as_vec2().to_array(), tint, tangent };
                    for i in 1..w.0.len().saturating_sub(1) {
                        verts.extend([v(w.0[0]), v(w.0[i]), v(w.0[i + 1])]);
                    }
                    for i in 0..w.0.len() {
                        edges.push(LineVertex::new(w.0[i], edge));
                        edges.push(LineVertex::new(w.0[(i + 1) % w.0.len()], edge));
                    }
                }
            }
            NodeKind::Entity(e) if node.children.is_empty() => {
                let origin = e.origin().unwrap_or_default();
                let color = linear(style.color(&e.classname).unwrap_or(POINT_ENTITY));
                let (min, max) = entity_bounds(style, e);
                let edge = if selected { SELECTED_EDGE } else { color };
                if let Some(model) = style.model(e) {
                    push_model(&mut scene.batches, &mut scene.edges, &model, &entity_transform(e), selected, edge);
                    if selected {
                        box_lines(&mut scene.selected_edges, min, max, SELECTED_EDGE);
                    }
                } else {
                    push_box(scene.batches.entry(String::new()).or_default(), min, max, if selected { SELECTED_TINT } else { color });
                    box_lines(if selected { &mut scene.selected_edges } else { &mut scene.edges }, min, max, edge);
                }
                scene.lights.extend(read_light(e, origin));
                if let &[r, g, b, v, ..] = numbers(e.get("_ambient").unwrap_or_default()).as_slice() {
                    scene.ambient = (DVec3::from_array(light_color(&[r, g, b])) * v / 255.0).as_vec3().to_array();
                }
            }
            NodeKind::Entity(e) => {
                child_color = Some(linear(style.color(&e.classname).unwrap_or(SOLID_ENTITY)));
            }
            _ => {}
        }
        stack.extend(node.children.iter().map(|c| (*c, selected, child_color)));
    }
    scene
}

fn numbers(s: &str) -> Vec<f64> {
    s.split_whitespace().take(4).map(|t| t.parse::<f64>().ok().filter(|v| v.is_finite())).collect::<Option<Vec<_>>>().unwrap_or_default()
}

/// Linear color from "r g b" in 0 to 255, or in 0 to 1 as some Quake tools write it.
fn light_color(v: &[f64]) -> [f64; 3] {
    let &[r, g, b, ..] = v else { return [1.0; 3] };
    let scale = if r.max(g).max(b) > 1.0 { 255.0 } else { 1.0 };
    [r, g, b].map(|c| (c / scale).clamp(0.0, 1.0).powf(2.2))
}

/// Unit vector for a yaw and pitch in degrees; lights point straight down at pitch -90.
fn aim(yaw: f64, pitch: f64) -> DVec3 {
    let (y, p) = (yaw.to_radians(), pitch.to_radians());
    DVec3::new(p.cos() * y.cos(), p.cos() * y.sin(), p.sin())
}

/// Reads a light from Source or Quake style key values.
fn read_light(e: &Entity, origin: DVec3) -> Option<Light> {
    let name = e.classname.as_str();
    if name != "light" && !name.starts_with("light_") && e.get("_light").is_none() {
        return None;
    }
    let num = |k: &str| e.get(k).map(numbers).unwrap_or_default();
    let first = |k: &str| num(k).first().copied();
    let source = num("_light");
    // Source writes "r g b brightness" there; Quake tools take a lone brightness.
    let quake = source.len() < 3;
    let brightness = if quake { source.first().copied().or_else(|| first("light")).unwrap_or(300.0) } else { source.get(3).copied().unwrap_or(200.0) };
    let color = DVec3::from_array(light_color(&if quake { num("_color") } else { source })) * brightness / 255.0;
    let wait = first("wait").filter(|w| *w > 0.0).unwrap_or(1.0);
    let falloff = if quake { first("delay").filter(|d| (0.0..=3.0).contains(d)).map_or(0.0, f64::floor) } else { 2.0 };
    // Quake's linear falloff ends at `light / wait`; its inverse falloffs reach 1 at 128 units, Source's at 100.
    let distance = if !quake {
        100.0
    } else if falloff == 0.0 {
        brightness.abs() / wait
    } else {
        128.0 / wait
    };
    let angles = num("angles");
    let pitch = first("pitch").or(angles.first().copied()).unwrap_or(-90.0);
    let (dir, outer, inner) = match (name, num("mangle").as_slice()) {
        ("light_spot" | "light_environment", _) => {
            (aim(angles.get(1).copied().unwrap_or(0.0), pitch), first("_cone").unwrap_or(45.0), first("_inner_cone").unwrap_or(30.0))
        }
        (_, &[yaw, pitch, ..]) => (aim(yaw, pitch), first("angle").unwrap_or(40.0) / 2.0, first("_softangle").unwrap_or(0.0) / 2.0),
        _ => (DVec3::ZERO, 0.0, 0.0),
    };
    if name == "light_environment" {
        return Some(Light { pos: (-dir).extend(0.0).as_vec4().to_array(), color: color.extend(3.0).as_vec4().to_array(), ..Light::default() });
    }
    let cos = |deg: f64| deg.to_radians().cos();
    Some(Light {
        pos: origin.extend(1.0).as_vec4().to_array(),
        color: color.extend(falloff).as_vec4().to_array(),
        spot: dir.extend(distance.max(1.0)).as_vec4().to_array(),
        // Kept apart so the shader's smoothstep never divides by zero.
        cone: [cos(outer) as f32, cos(inner).max(cos(outer) + 1e-3) as f32, 0.0, 0.0],
    })
}

/// Adds a placed model's triangles to the batches of its textures, and its wireframe.
fn push_model(batches: &mut BTreeMap<String, Vec<MeshVertex>>, edges: &mut Vec<LineVertex>, model: &Model, at: &DMat4, selected: bool, edge: [f32; 4]) {
    let turn = DMat3::from_mat4(*at);
    for part in &model.parts {
        let tint = if selected { std::array::from_fn(|i| part.color[i] * SELECTED_TINT[i]) } else { part.color };
        let world: Vec<DVec3> = part.positions.iter().map(|p| at.transform_point3(DVec3::from_array(p.map(f64::from)))).collect();
        let verts = batches.entry(model.texture_name(part.texture)).or_default();
        verts.extend(part.indices.iter().filter_map(|i| {
            let i = *i as usize;
            let n = (turn * DVec3::from_array(part.normals.get(i)?.map(f64::from))).normalize_or(DVec3::Z);
            Some(MeshVertex {
                pos: world.get(i)?.as_vec3().to_array(),
                normal: n.as_vec3().to_array(),
                uv: *part.uvs.get(i)?,
                tint,
                // Any tangent across the normal does, since model textures carry no normal maps.
                tangent: n.any_orthonormal_vector().as_vec3().extend(1.0).to_array(),
            })
        }));
        for [a, b] in &part.edges {
            if let (Some(a), Some(b)) = (world.get(*a as usize), world.get(*b as usize)) {
                edges.push(LineVertex::new(*a, edge));
                edges.push(LineVertex::new(*b, edge));
            }
        }
    }
}

fn push_box(out: &mut Vec<MeshVertex>, min: DVec3, max: DVec3, tint: [f32; 4]) {
    let faces: [(DVec3, [DVec3; 4]); 6] = [
        (DVec3::X, [DVec3::new(max.x, min.y, min.z), DVec3::new(max.x, max.y, min.z), DVec3::new(max.x, max.y, max.z), DVec3::new(max.x, min.y, max.z)]),
        (-DVec3::X, [DVec3::new(min.x, max.y, min.z), DVec3::new(min.x, min.y, min.z), DVec3::new(min.x, min.y, max.z), DVec3::new(min.x, max.y, max.z)]),
        (DVec3::Y, [DVec3::new(max.x, max.y, min.z), DVec3::new(min.x, max.y, min.z), DVec3::new(min.x, max.y, max.z), DVec3::new(max.x, max.y, max.z)]),
        (-DVec3::Y, [DVec3::new(min.x, min.y, min.z), DVec3::new(max.x, min.y, min.z), DVec3::new(max.x, min.y, max.z), DVec3::new(min.x, min.y, max.z)]),
        (DVec3::Z, [DVec3::new(min.x, min.y, max.z), DVec3::new(max.x, min.y, max.z), DVec3::new(max.x, max.y, max.z), DVec3::new(min.x, max.y, max.z)]),
        (-DVec3::Z, [DVec3::new(min.x, max.y, min.z), DVec3::new(max.x, max.y, min.z), DVec3::new(max.x, min.y, min.z), DVec3::new(min.x, min.y, min.z)]),
    ];
    for (n, q) in faces {
        let v = |p: DVec3| MeshVertex { pos: p.as_vec3().to_array(), normal: n.as_vec3().to_array(), uv: [0.0, 0.0], tint, tangent: [1.0, 0.0, 0.0, 1.0] };
        out.extend([v(q[0]), v(q[1]), v(q[2]), v(q[0]), v(q[2]), v(q[3])]);
    }
}

#[cfg(test)]
mod tests {
    use mapcraft_geom::Aabb;
    use mapcraft_map::{Brush, Entity, ROOT, TexDefaults};

    use super::*;

    #[test]
    fn builds_batches_and_edges() {
        let mut map = Map::new();
        let mut e = map.edit();
        let tex = TexDefaults { material: "a".into(), scale: 1.0 };
        let b = e.insert(ROOT, NodeKind::Brush(Brush::cuboid(Aabb::new(DVec3::ZERO, DVec3::splat(64.0)), &tex))).unwrap();
        let mut ent = Entity::new("info_player_start");
        ent.set("origin", "0 0 100");
        e.insert(ROOT, NodeKind::Entity(ent)).unwrap();
        map.apply(e.finish());

        let scene = build_scene(&map, &BTreeSet::new(), &BTreeSet::new(), &|_| Some([64, 64]), &NoStyle, false);
        assert_eq!(scene.batches["a"].len(), 6 * 2 * 3);
        assert_eq!(scene.batches[""].len(), 36);
        assert_eq!(scene.edges.len(), 6 * 4 * 2 + 24);
        assert!(scene.selected_edges.is_empty());
        assert!(scene.lights.is_empty());
        let uv_max = scene.batches["a"].iter().map(|v| v.uv[0].abs().max(v.uv[1].abs())).fold(0.0f32, f32::max);
        assert!((uv_max - 1.0).abs() < 1e-6);
        let frame = |n: [f32; 3]| scene.batches["a"].iter().find(|v| v.normal == n).unwrap().tangent;
        assert_eq!(frame([0.0, 0.0, 1.0]), [1.0, 0.0, 0.0, 1.0], "on a floor u runs along +X and the picture's top faces +Y");
        assert_eq!(frame([0.0, 1.0, 0.0]), [1.0, 0.0, 0.0, -1.0], "world aligned textures mirror on the far wall");
        assert_eq!(frame([0.0, -1.0, 0.0]), [1.0, 0.0, 0.0, 1.0]);

        let scene = build_scene(&map, &BTreeSet::from([b]), &BTreeSet::new(), &|_| None, &NoStyle, true);
        assert_eq!(scene.selected_edges.len(), 48);

        let face = map.get(b).unwrap().kind.brush().unwrap().faces[0].id;
        let scene = build_scene(&map, &BTreeSet::new(), &BTreeSet::from([(b, face)]), &|_| None, &NoStyle, true);
        assert_eq!(scene.selected_edges.len(), 8, "a picked face is outlined");
        assert!(scene.batches["a"].iter().all(|v| v.tint == WHITE), "and keeps its true color");
    }

    #[test]
    fn reads_source_and_quake_lights() {
        let mut map = Map::new();
        let mut e = map.edit();
        for kv in [
            &[("classname", "light"), ("light", "200"), ("_color", "255 0 0"), ("wait", "2")][..],
            &[("classname", "light_spot"), ("_light", "0 255 0 400"), ("pitch", "-90"), ("_cone", "60")],
            &[("classname", "light_environment"), ("_light", "255 255 255 100"), ("angles", "0 0 0"), ("pitch", "-90"), ("_ambient", "255 255 255 51")],
            &[("classname", "light"), ("mangle", "90 0 0"), ("delay", "2"), ("light", "junk")],
        ] {
            let mut ent = Entity::default();
            for (k, v) in kv {
                ent.set(*k, *v);
            }
            e.insert(ROOT, NodeKind::Entity(ent)).unwrap();
        }
        map.apply(e.finish());
        let scene = build_scene(&map, &BTreeSet::new(), &BTreeSet::new(), &|_| None, &NoStyle, false);
        let find = |w: f32, falloff: f32| *scene.lights.iter().find(|l| l.pos[3] == w && l.color[3] == falloff).unwrap();
        let close = |a: f32, b: f32| (a - b).abs() < 1e-4;

        let quake = find(1.0, 0.0);
        assert!(close(quake.color[0], 200.0 / 255.0) && quake.color[1] == 0.0);
        assert!(close(quake.spot[3], 100.0), "linear falloff ends at light / wait");
        assert_eq!(&quake.spot[..3], &[0.0; 3], "an omni light");

        let spot = *scene.lights.iter().find(|l| l.pos[3] == 1.0 && l.spot[2] < -0.99).unwrap();
        assert!(close(spot.color[1], 400.0 / 255.0) && spot.color[3] == 2.0);
        assert!(close(spot.cone[0], 0.5));

        let sun = find(0.0, 3.0);
        assert!(close(sun.pos[2], 1.0), "points toward the sun");
        assert!(close(scene.ambient[0], 0.2));

        let aimed = *scene.lights.iter().find(|l| l.spot[1] > 0.99).unwrap();
        assert!(close(aimed.color[0], 300.0 / 255.0) && aimed.color[3] == 2.0, "bad brightness falls back to 300");
        assert_eq!(scene.lights.len(), 4);
    }

    struct Props(Arc<Model>);

    impl EntityStyle for Props {
        fn color(&self, _: &str) -> Option<[u8; 3]> {
            None
        }

        fn size(&self, _: &str) -> Option<(DVec3, DVec3)> {
            None
        }

        fn model(&self, e: &Entity) -> Option<Arc<Model>> {
            (e.get("model") == Some("quad.glb")).then(|| self.0.clone())
        }
    }

    #[test]
    fn draws_models_in_place_of_boxes() {
        let quad = crate::model::load_gltf("quad.glb", &crate::model::tests::quad_glb(), &|_| None, 10.0).unwrap();
        let mut map = Map::new();
        let mut e = map.edit();
        let mut prop = Entity::new("prop_static");
        prop.set("model", "quad.glb");
        prop.set("origin", "100 0 0");
        let id = e.insert(ROOT, NodeKind::Entity(prop.clone())).unwrap();
        map.apply(e.finish());
        let style = Props(Arc::new(quad));
        let (min, max) = entity_bounds(&style, &prop);
        assert!(min.abs_diff_eq(DVec3::new(100.0, 20.0, 0.0), 1e-4) && max.abs_diff_eq(DVec3::new(110.0, 20.0, 10.0), 1e-4), "{min} {max}");

        let scene = build_scene(&map, &BTreeSet::new(), &BTreeSet::new(), &|_| None, &style, false);
        assert_eq!(scene.batches["quad.glb#0"].len(), 6, "two triangles under the model's texture");
        assert!(!scene.batches.contains_key(""), "no box");
        assert_eq!(scene.edges.len(), 10, "five wireframe edges");

        let scene = build_scene(&map, &BTreeSet::from([id]), &BTreeSet::new(), &|_| None, &style, false);
        assert_eq!(scene.selected_edges.len(), 24, "a selected model is boxed");
    }
}
