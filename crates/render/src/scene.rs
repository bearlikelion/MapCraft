use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mapcraft_geom::{DMat3, DMat4, DVec2, DVec3};
pub use mapcraft_map::is_tool_material;
use mapcraft_map::{AntiTile, Entity, Face, FaceId, Map, NodeId, NodeKind, Operation, TexAxis};

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
    /// Texture coordinates of a blended displacement's second layer.
    pub blend_uv: [f32; 2],
    /// Second layer share at a displaced vertex; 0 elsewhere.
    pub blend: f32,
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
    /// Linear color times brightness, or for a Quake light its lightmap brightness; w is the falloff, numbered like Quake's `delay` key.
    pub color: [f32; 4],
    /// Spot direction, zero for an omni light; w is the falloff distance.
    pub spot: [f32; 4],
    /// Cosines of the spot's outer and inner half angles; w is 1 for a Quake light, with z its angle scale.
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

/// What a batch of triangles is drawn with.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct BatchKey {
    /// `""` is untextured.
    pub material: String,
    /// The second layer of blended displacements; empty for one layer.
    pub blend: String,
    pub anti_tile: AntiTile,
}

impl From<&str> for BatchKey {
    fn from(material: &str) -> Self {
        Self { material: material.to_string(), ..Self::default() }
    }
}

/// CPU-side geometry for the whole map, grouped by what it is drawn with.
#[derive(Clone, Debug, Default)]
pub struct SceneData {
    pub batches: BTreeMap<BatchKey, Vec<MeshVertex>>,
    /// Point entity boxes and models by material, which a baked map leaves in place.
    pub points: BTreeMap<BatchKey, Vec<MeshVertex>>,
    pub edges: Vec<LineVertex>,
    /// Drawn over everything.
    pub selected_edges: Vec<LineVertex>,
    /// Faces with tool materials (triggers, clips) and untextured brush entity faces, drawn see-through after everything else.
    pub translucent: Vec<MeshVertex>,
    pub lights: Vec<Light>,
    /// Linear ambient color a light entity's `_ambient` asks for.
    pub ambient: [f32; 3],
    /// Quake's minimum lightmap brightness from worldspawn.
    pub minlight: [f32; 3],
    pub sky: Option<Sky>,
}

/// Godot's procedural sky for the Lit mode, in linear color, as the MapCraft addon builds it from a `light_environment`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sky {
    pub top: [f32; 3],
    pub horizon: [f32; 3],
    pub ground: [f32; 3],
    /// Scales the light the sky casts on surfaces.
    pub ambient: f32,
}

const TOOL_ALPHA: f32 = 0.3;

pub const WORLD_EDGE: [f32; 4] = [0.85, 0.85, 0.85, 1.0];
pub const SELECTED_EDGE: [f32; 4] = [1.0, 0.1, 0.1, 1.0];
pub const SUBTRACT_EDGE: [f32; 4] = [1.0, 0.6, 0.15, 1.0];
const SELECTED_TINT: [f32; 4] = [1.0, 0.62, 0.55, 1.0];
const PICKED_FACE_EDGE: [f32; 4] = [0.0, 0.8, 1.0, 1.0];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const SOLID_ENTITY: [u8; 3] = [220, 30, 220];
const POINT_ENTITY: [u8; 3] = [80, 220, 80];
/// Half the size of the cross marking a brush entity's origin.
const ORIGIN_MARK: f64 = 8.0;

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

/// Limits in degrees from flat for coloring faces by slope: walkable up to `walkable`, surfable up to `surfable`, then wall, then ceiling.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slope {
    pub walkable: f64,
    pub surfable: f64,
}

impl Slope {
    pub const WALKABLE: [f32; 4] = [0.2, 0.75, 0.25, 1.0];
    pub const SURFABLE: [f32; 4] = [0.15, 0.45, 1.0, 1.0];
    pub const WALL: [f32; 4] = [0.9, 0.5, 0.15, 1.0];
    pub const CEILING: [f32; 4] = [0.55, 0.25, 0.65, 1.0];

    /// The band color of a surface facing `normal`.
    pub fn tint(&self, normal: DVec3) -> [f32; 4] {
        let angle = normal.normalize_or(DVec3::Z).z.clamp(-1.0, 1.0).acos().to_degrees();
        if angle > 90.0 + 1e-6 {
            Self::CEILING
        } else if angle <= self.walkable {
            Self::WALKABLE
        } else if angle <= self.surfable {
            Self::SURFABLE
        } else {
            Self::WALL
        }
    }
}

/// Builds the scene. `texture_size` gives the texel size used to normalize UVs; with `colorize`
/// each world brush's edges get their own color, and with `slope` faces are tinted by steepness.
pub fn build_scene(
    map: &Map,
    selection: &BTreeSet<NodeId>,
    faces: &BTreeSet<(NodeId, FaceId)>,
    texture_size: &dyn Fn(&str) -> Option<[u32; 2]>,
    style: &dyn EntityStyle,
    colorize: bool,
    slope: Option<Slope>,
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
                // A boolean group's own brushes are outlines only; its result shows their faces.
                if let Some(op) = mapcraft_map::boolean::source_op(map, id) {
                    let color = match (selected, op) {
                        (true, _) => SELECTED_EDGE,
                        (false, Operation::Subtract) => SUBTRACT_EDGE,
                        (false, Operation::Add) => continue,
                    };
                    for w in solid.faces.iter().flatten() {
                        for (a, b) in w.0.iter().zip(w.0.iter().cycle().skip(1)) {
                            scene.selected_edges.extend([LineVertex::new(*a, color), LineVertex::new(*b, color)]);
                        }
                    }
                    continue;
                }
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
                    let size_of = |m: &str| texture_size(m).map_or(DVec2::splat(64.0), |s| DVec2::new(f64::from(s[0]), f64::from(s[1])));
                    let size = size_of(&face.material);
                    let face_selected = faces.contains(&(id, face.id));
                    // An untextured brush entity face, such as a trigger's, shows the entity color.
                    let tool = is_tool_material(&face.material) || (face.material.is_empty() && entity_color.is_some());
                    // Picked faces keep their true color so materials can be judged; the outline marks them.
                    let band = |s: Slope, n: DVec3| {
                        let t = s.tint(n);
                        if selected && !face_selected { std::array::from_fn(|i| t[i] * SELECTED_TINT[i]) } else { t }
                    };
                    let tint = match (selected || face_selected, tool, slope) {
                        (_, false, Some(s)) => band(s, plane_normal),
                        (true, false, None) if selected && !face_selected => SELECTED_TINT,
                        (_, false, None) => WHITE,
                        (true, true, _) => [SELECTED_EDGE[0], SELECTED_EDGE[1], SELECTED_EDGE[2], TOOL_ALPHA + 0.15],
                        (false, true, _) => {
                            let c = entity_color.unwrap_or([0.8, 0.5, 0.1, 1.0]);
                            [c[0], c[1], c[2], TOOL_ALPHA]
                        }
                    };
                    let layer = face.blend_layer().filter(|_| displaced.contains_key(&index));
                    let key = BatchKey { blend: layer.unwrap_or_default().to_string(), anti_tile: face.anti_tile(), ..BatchKey::from(face.material.as_str()) };
                    let verts = if tool { &mut scene.translucent } else { scene.batches.entry(key).or_default() };
                    let edges = if selected || face_selected { &mut scene.selected_edges } else { &mut scene.edges };
                    let edge = if face_selected { PICKED_FACE_EDGE } else { edge };
                    if let (Some(grid), Some(d)) = (displaced.get(&index), &face.disp) {
                        let normals = d.normals(grid, plane_normal);
                        let size_b = layer.map(size_of);
                        let v = |k: usize| MeshVertex {
                            pos: grid[k].as_vec3().to_array(),
                            normal: normals[k].as_vec3().to_array(),
                            uv: face.uv(grid[k], size).as_vec2().to_array(),
                            tint,
                            tangent,
                            blend_uv: size_b.map_or([0.0; 2], |s| face.uv(grid[k], s).as_vec2().to_array()),
                            blend: if tool { 0.0 } else { d.blend_at(k) as f32 },
                        };
                        let slope_tint = |[a, b, c]: [usize; 3]| {
                            let n = (grid[b] - grid[a]).cross(grid[c] - grid[a]);
                            slope.filter(|_| !tool).map(|s| band(s, if n.dot(plane_normal) < 0.0 { -n } else { n }))
                        };
                        verts.extend(d.triangles().into_iter().flat_map(|t| {
                            let tinted = |k| MeshVertex { tint: slope_tint(t).unwrap_or(tint), ..v(k) };
                            [tinted(t[0]), tinted(t[1]), tinted(t[2])]
                        }));
                        let n = d.size();
                        for k in 0..n * n {
                            for next in [(k % n + 1 < n).then_some(k + 1), (k + n < n * n).then_some(k + n)].into_iter().flatten() {
                                edges.push(LineVertex::new(grid[k], edge));
                                edges.push(LineVertex::new(grid[next], edge));
                            }
                        }
                        continue;
                    }
                    let v = |p: DVec3| MeshVertex {
                        pos: p.as_vec3().to_array(),
                        normal,
                        uv: face.uv(p, size).as_vec2().to_array(),
                        tint,
                        tangent,
                        blend_uv: [0.0; 2],
                        blend: 0.0,
                    };
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
                    push_model(&mut scene.points, &mut scene.edges, &model, &entity_transform(e), selected, edge);
                    if selected {
                        box_lines(&mut scene.selected_edges, min, max, SELECTED_EDGE);
                    }
                } else {
                    push_box(scene.points.entry(BatchKey::default()).or_default(), min, max, if selected { SELECTED_TINT } else { color });
                    let out = if selected { &mut scene.selected_edges } else { &mut scene.edges };
                    box_lines(out, min, max, edge);
                    if ["angles", "angle"].iter().any(|k| !numbers(e.get(k).unwrap_or_default()).is_empty()) {
                        facing_arrow(out, &entity_transform(e), min, max, edge);
                    }
                }
                if let Some((sun, sky)) = read_environment(e) {
                    scene.lights.push(sun);
                    scene.sky = Some(sky);
                } else {
                    scene.lights.extend(read_light(e, origin));
                }
                if let &[r, g, b, v, ..] = numbers(e.get("_ambient").unwrap_or_default()).as_slice() {
                    scene.ambient = (light_color(&[r, g, b]).powf(2.2) * v / 255.0).as_vec3().to_array();
                }
            }
            NodeKind::Entity(e) => {
                let color = linear(style.color(&e.classname).unwrap_or(SOLID_ENTITY));
                child_color = Some(color);
                if let Some(o) = e.origin() {
                    let (out, edge) = if selected { (&mut scene.selected_edges, SELECTED_EDGE) } else { (&mut scene.edges, color) };
                    for axis in [DVec3::X, DVec3::Y, DVec3::Z] {
                        out.push(LineVertex::new(o - axis * ORIGIN_MARK, edge));
                        out.push(LineVertex::new(o + axis * ORIGIN_MARK, edge));
                    }
                }
            }
            _ => {}
        }
        stack.extend(node.children.iter().map(|c| (*c, selected, child_color)));
    }
    read_quake_world(map.world(), &mut scene);
    scene
}

fn numbers(s: &str) -> Vec<f64> {
    s.split_whitespace().take(4).map(|t| t.parse::<f64>().ok().filter(|v| v.is_finite())).collect::<Option<Vec<_>>>().unwrap_or_default()
}

/// Gamma encoded color from "r g b" in 0 to 255, or in 0 to 1 as some Quake tools write it.
fn light_color(v: &[f64]) -> DVec3 {
    let &[r, g, b, ..] = v else { return DVec3::ONE };
    let scale = if r.max(g).max(b) > 1.0 { 255.0 } else { 1.0 };
    (DVec3::new(r, g, b) / scale).clamp(DVec3::ZERO, DVec3::ONE)
}

/// Unit vector for a yaw and pitch in degrees; lights point straight down at pitch -90.
fn aim(yaw: f64, pitch: f64) -> DVec3 {
    let (y, p) = (yaw.to_radians(), pitch.to_radians());
    DVec3::new(p.cos() * y.cos(), p.cos() * y.sin(), p.sin())
}

/// A `light_environment` without Source's `_light`, read with the Godot addon's keys and defaults.
fn read_environment(e: &Entity) -> Option<(Light, Sky)> {
    if e.classname != "light_environment" || e.get("_light").is_some() {
        return None;
    }
    let number = |k: &str, default: f64| e.get(k).and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite()).unwrap_or(default);
    let color = |k: &str, default: [f64; 3]| {
        let v = numbers(e.get(k).unwrap_or_default());
        let c = if let &[r, g, b, ..] = v.as_slice() { [r, g, b].map(|c| c / 255.0) } else { default };
        c.map(|c| c.clamp(0.0, 1.0).powf(2.2) as f32)
    };
    let (pitch, yaw) = match numbers(e.get("angles").unwrap_or_default()).as_slice() {
        &[p, y, ..] => (p, y),
        _ => (50.0, 135.0),
    };
    let sun = DVec3::from_array(color("sun_color", [1.0, 0.96, 0.9]).map(f64::from)) * number("sun_energy", 1.0);
    // Positive pitch tips the sunlight below the horizon, as it does a marker's facing.
    let light = Light { pos: (-aim(yaw, -pitch)).extend(0.0).as_vec4().to_array(), color: sun.extend(3.0).as_vec4().to_array(), ..Light::default() };
    let sky = Sky {
        top: color("sky_top_color", [0.3, 0.48, 0.75]),
        horizon: color("sky_horizon_color", [0.72, 0.8, 0.88]),
        ground: color("ground_color", [0.2, 0.21, 0.23]),
        ambient: number("ambient_energy", 1.0) as f32,
    };
    Some((light, sky))
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
    // The light compiler stores half a Quake light's value, and engines show a lightmap of 128 as the texture itself.
    let color = if quake { light_color(&num("_color")) } else { light_color(&source).powf(2.2) } * brightness / 255.0;
    let wait = first("wait").filter(|w| *w > 0.0).unwrap_or(1.0);
    let falloff = if quake { first("delay").map(f64::floor).filter(|d| [1.0, 2.0, 3.0, 5.0].contains(d)).unwrap_or(0.0) } else { 2.0 };
    let angle_scale = if quake { first("_anglescale").or_else(|| first("_anglesense")).unwrap_or(0.5).clamp(0.0, 1.0) } else { 0.0 };
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
        cone: [cos(outer) as f32, cos(inner).max(cos(outer) + 1e-3) as f32, angle_scale as f32, if quake { 1.0 } else { 0.0 }],
    })
}

/// Quake's worldspawn lighting, as ericw-tools reads it: the minimum light level and the sun.
fn read_quake_world(world: &Entity, scene: &mut SceneData) {
    let num = |k: &str| world.get(k).map(numbers).unwrap_or_default();
    let either = |a: &str, b: &str| Some(num(a)).filter(|v| !v.is_empty()).unwrap_or_else(|| num(b));
    if let Some(&min) = either("_minlight", "light").first() {
        scene.minlight = (light_color(&either("_minlight_color", "_mincolor")) * min / 255.0).as_vec3().to_array();
    }
    let sun = num("_sunlight").first().copied().unwrap_or(0.0);
    if sun != 0.0 {
        let (yaw, pitch) = match either("_sunlight_mangle", "_sun_mangle").as_slice() {
            &[yaw, pitch, ..] => (yaw, pitch),
            _ => (0.0, -90.0),
        };
        let angle_scale = either("_anglescale", "_anglesense").first().copied().unwrap_or(0.5).clamp(0.0, 1.0);
        scene.lights.push(Light {
            pos: (-aim(yaw, pitch)).extend(0.0).as_vec4().to_array(),
            color: (light_color(&num("_sunlight_color")) * sun / 255.0).extend(3.0).as_vec4().to_array(),
            cone: [0.0, 0.0, angle_scale as f32, 1.0],
            ..Light::default()
        });
    }
}

/// Adds a placed model's triangles to the batches of its textures, and its wireframe.
pub(crate) fn push_model(
    batches: &mut BTreeMap<BatchKey, Vec<MeshVertex>>,
    edges: &mut Vec<LineVertex>,
    model: &Model,
    at: &DMat4,
    selected: bool,
    edge: [f32; 4],
) {
    let turn = DMat3::from_mat4(*at);
    for part in &model.parts {
        let tint = if selected { std::array::from_fn(|i| part.color[i] * SELECTED_TINT[i]) } else { part.color };
        let world: Vec<DVec3> = part.positions.iter().map(|p| at.transform_point3(DVec3::from_array(p.map(f64::from)))).collect();
        let verts = batches.entry(BatchKey::from(model.texture_name(part.texture).as_str())).or_default();
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
                blend_uv: [0.0; 2],
                blend: 0.0,
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

/// A point entity's facing, as an arrow from the middle of its box's top out past its side.
fn facing_arrow(out: &mut Vec<LineVertex>, at: &DMat4, min: DVec3, max: DVec3, color: [f32; 4]) {
    let turn = DMat3::from_mat4(*at);
    let (dir, side) = ((turn * DVec3::X).normalize_or(DVec3::X), (turn * DVec3::Y).normalize_or(DVec3::Y));
    let reach = (max - min).truncate().max_element().max(16.0);
    let from = ((min + max) / 2.0).with_z(max.z);
    let tip = from + dir * reach;
    let head = reach * 0.3;
    for (a, b) in [(from, tip), (tip, tip - (dir + side * 0.5) * head), (tip, tip - (dir - side * 0.5) * head)] {
        out.push(LineVertex::new(a, color));
        out.push(LineVertex::new(b, color));
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
        let v = |p: DVec3| MeshVertex {
            pos: p.as_vec3().to_array(),
            normal: n.as_vec3().to_array(),
            uv: [0.0, 0.0],
            tint,
            tangent: [1.0, 0.0, 0.0, 1.0],
            blend_uv: [0.0; 2],
            blend: 0.0,
        };
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

        let scene = build_scene(&map, &BTreeSet::new(), &BTreeSet::new(), &|_| Some([64, 64]), &NoStyle, false, None);
        assert_eq!(scene.batches[&BatchKey::from("a")].len(), 6 * 2 * 3);
        assert_eq!(scene.points[&BatchKey::default()].len(), 36);
        assert_eq!(scene.edges.len(), 6 * 4 * 2 + 24);
        assert!(scene.selected_edges.is_empty());
        assert!(scene.lights.is_empty());
        let uv_max = scene.batches[&BatchKey::from("a")].iter().map(|v| v.uv[0].abs().max(v.uv[1].abs())).fold(0.0f32, f32::max);
        assert!((uv_max - 1.0).abs() < 1e-6);
        let frame = |n: [f32; 3]| scene.batches[&BatchKey::from("a")].iter().find(|v| v.normal == n).unwrap().tangent;
        assert_eq!(frame([0.0, 0.0, 1.0]), [1.0, 0.0, 0.0, 1.0], "on a floor u runs along +X and the picture's top faces +Y");
        assert_eq!(frame([0.0, 1.0, 0.0]), [1.0, 0.0, 0.0, -1.0], "world aligned textures mirror on the far wall");
        assert_eq!(frame([0.0, -1.0, 0.0]), [1.0, 0.0, 0.0, 1.0]);

        let scene = build_scene(&map, &BTreeSet::from([b]), &BTreeSet::new(), &|_| None, &NoStyle, true, None);
        assert_eq!(scene.selected_edges.len(), 48);

        let face = map.get(b).unwrap().kind.brush().unwrap().faces[0].id;
        let scene = build_scene(&map, &BTreeSet::new(), &BTreeSet::from([(b, face)]), &|_| None, &NoStyle, true, None);
        assert_eq!(scene.selected_edges.len(), 8, "a picked face is outlined");
        assert!(scene.batches[&BatchKey::from("a")].iter().all(|v| v.tint == WHITE), "and keeps its true color");

        let slope = Slope { walkable: 45.573, surfable: 72.0 };
        let scene = build_scene(&map, &BTreeSet::new(), &BTreeSet::new(), &|_| None, &NoStyle, false, Some(slope));
        let tint = |n: [f32; 3]| scene.batches[&BatchKey::from("a")].iter().find(|v| v.normal == n).unwrap().tint;
        assert_eq!(tint([0.0, 0.0, 1.0]), Slope::WALKABLE);
        assert_eq!(tint([1.0, 0.0, 0.0]), Slope::WALL);
        assert_eq!(tint([0.0, 0.0, -1.0]), Slope::CEILING);
        assert_eq!(slope.tint(DVec3::new(0.0, 0.8, 0.6)), Slope::SURFABLE, "53 degrees surfs");
        assert_eq!(slope.tint(DVec3::new(0.0, 0.7, 0.71)), Slope::WALKABLE, "normal z 0.7 and up stands");
        assert_eq!(slope.tint(DVec3::new(0.0, 0.97, 0.24)), Slope::WALL, "76 degrees is too steep");
    }

    #[test]
    fn brush_entities_mark_their_origin() {
        let mut map = Map::new();
        let mut e = map.edit();
        let tex = TexDefaults { material: "a".into(), scale: 1.0 };
        let mut door = Entity::new("func_door_rotating");
        door.set("origin", "0 0 0");
        let door = e.insert(ROOT, NodeKind::Entity(door)).unwrap();
        e.insert(door, NodeKind::Brush(Brush::cuboid(Aabb::new(DVec3::ZERO, DVec3::splat(64.0)), &tex))).unwrap();
        map.apply(e.finish());
        let scene = build_scene(&map, &BTreeSet::new(), &BTreeSet::new(), &|_| None, &NoStyle, false, None);
        assert_eq!(scene.edges.len(), 6 * 4 * 2 + 6, "the brush's edges and a cross at the origin");
        let scene = build_scene(&map, &BTreeSet::from([door]), &BTreeSet::new(), &|_| None, &NoStyle, false, None);
        assert_eq!(scene.selected_edges.len(), 6 * 4 * 2 + 6);
    }

    #[test]
    fn blended_displacements_get_a_two_layer_batch() {
        let mut map = Map::new();
        let mut e = map.edit();
        for (x, material, layer) in [(0.0, "a", "b"), (128.0, "a", "A"), (256.0, "tools/trigger", "b")] {
            let tex = TexDefaults { material: material.into(), scale: 1.0 };
            let mut b = Brush::cuboid(Aabb::new(DVec3::new(x, 0.0, -16.0), DVec3::new(x + 64.0, 64.0, 0.0)), &tex);
            let solid = b.solid().unwrap();
            let (top, w) = b.faces.iter_mut().zip(&solid.faces).find(|(f, _)| f.plane().unwrap().normal.z > 0.9).unwrap();
            let mut d = mapcraft_map::Displacement::new(1, w.as_ref().unwrap().0[0]);
            d.set_blend((0..9).map(|k| k as f64 / 8.0).collect());
            d.blend_material = Some(layer.into());
            top.disp = Some(d);
            e.insert(ROOT, NodeKind::Brush(b)).unwrap();
        }
        map.apply(e.finish());
        let size = |m: &str| Some(if m == "b" { [32, 16] } else { [64, 64] });
        let scene = build_scene(&map, &BTreeSet::new(), &BTreeSet::new(), &size, &NoStyle, false, None);

        let blended = &scene.batches[&BatchKey { blend: "b".into(), ..BatchKey::from("a") }];
        assert_eq!(blended.len(), 8 * 3);
        let b = map.nodes().find_map(|(_, n)| n.kind.brush()).unwrap();
        let face = b.faces.iter().find(|f| f.disp.is_some()).unwrap();
        for v in blended {
            let texel = face.texel(DVec3::from_array(v.pos.map(f64::from)));
            assert!((DVec2::from_array(v.blend_uv.map(f64::from)) - texel / DVec2::new(32.0, 16.0)).length() < 1e-4, "layer b tiles by its own size");
        }
        assert!(blended.iter().any(|v| v.blend == 1.0) && blended.iter().any(|v| v.blend == 0.0));
        let own = &scene.batches[&BatchKey::from("a")];
        assert!(own.iter().any(|v| v.blend > 0.0) && own.iter().all(|v| v.blend_uv == [0.0; 2]), "its own material in other case stays one layer");
        assert_eq!(scene.batches.len(), 2, "tool faces ignore both");
        assert!(scene.translucent.iter().all(|v| v.blend == 0.0));
    }

    #[test]
    fn anti_tiled_faces_get_their_own_batch() {
        let mut map = Map::new();
        let mut e = map.edit();
        for material in ["a", "tools/trigger"] {
            let mut b = Brush::cuboid(Aabb::new(DVec3::ZERO, DVec3::splat(64.0)), &TexDefaults { material: material.into(), scale: 1.0 });
            b.faces[0].set_anti_tile(AntiTile::Hex);
            e.insert(ROOT, NodeKind::Brush(b)).unwrap();
        }
        map.apply(e.finish());
        let scene = build_scene(&map, &BTreeSet::new(), &BTreeSet::new(), &|_| None, &NoStyle, false, None);
        let hex = BatchKey { anti_tile: AntiTile::Hex, ..BatchKey::from("a") };
        assert_eq!(scene.batches[&hex].len(), 6);
        assert_eq!(scene.batches[&BatchKey::from("a")].len(), 30);
        assert_eq!(scene.batches.len(), 2, "tool faces stay see-through whatever they ask for");
        assert_eq!(scene.translucent.len(), 36);
    }

    #[test]
    fn untextured_brush_entity_faces_show_the_entity_color() {
        struct Green;
        impl EntityStyle for Green {
            fn color(&self, _: &str) -> Option<[u8; 3]> {
                Some([0, 255, 0])
            }

            fn size(&self, _: &str) -> Option<(DVec3, DVec3)> {
                None
            }
        }
        let mut map = Map::new();
        let mut e = map.edit();
        let tex = TexDefaults { material: String::new(), scale: 1.0 };
        let trigger = e.insert(ROOT, NodeKind::Entity(Entity::new("trigger_multiple"))).unwrap();
        e.insert(trigger, NodeKind::Brush(Brush::cuboid(Aabb::new(DVec3::ZERO, DVec3::splat(64.0)), &tex))).unwrap();
        e.insert(ROOT, NodeKind::Brush(Brush::cuboid(Aabb::new(DVec3::splat(128.0), DVec3::splat(192.0)), &tex))).unwrap();
        map.apply(e.finish());
        let scene = build_scene(&map, &BTreeSet::new(), &BTreeSet::new(), &|_| None, &Green, false, None);
        assert_eq!(scene.translucent.len(), 36);
        assert!(scene.translucent.iter().all(|v| v.tint == [0.0, 1.0, 0.0, TOOL_ALPHA]));
        assert_eq!(scene.batches[&BatchKey::default()].len(), 36, "an untextured world brush stays solid");
    }

    #[test]
    fn point_entities_show_their_facing() {
        let mut map = Map::new();
        let mut e = map.edit();
        let mut ent = Entity::new("info_player_start");
        ent.set("angles", "0 90 0");
        e.insert(ROOT, NodeKind::Entity(ent)).unwrap();
        e.insert(ROOT, NodeKind::Entity(Entity::new("info_target"))).unwrap();
        map.apply(e.finish());
        let scene = build_scene(&map, &BTreeSet::new(), &BTreeSet::new(), &|_| None, &NoStyle, false, None);
        assert_eq!(scene.edges.len(), 24 * 2 + 6, "only the entity with a facing gets an arrow");
        assert!(
            scene.edges.iter().any(|v| DVec3::from(v.pos.map(f64::from)).abs_diff_eq(DVec3::new(0.0, 16.0, 8.0), 1e-4)),
            "the arrow runs along +Y from the top of the box"
        );
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
        let scene = build_scene(&map, &BTreeSet::new(), &BTreeSet::new(), &|_| None, &NoStyle, false, None);
        let find = |w: f32, falloff: f32| *scene.lights.iter().find(|l| l.pos[3] == w && l.color[3] == falloff).unwrap();
        let close = |a: f32, b: f32| (a - b).abs() < 1e-4;

        let quake = find(1.0, 0.0);
        assert!(close(quake.color[0], 200.0 / 255.0) && quake.color[1] == 0.0);
        assert!(close(quake.spot[3], 100.0), "linear falloff ends at light / wait");
        assert_eq!(&quake.spot[..3], &[0.0; 3], "an omni light");
        assert_eq!(quake.cone[2..], [0.5, 1.0], "shades like a lightmap, with ericw-tools' default angle scale");

        let spot = *scene.lights.iter().find(|l| l.pos[3] == 1.0 && l.spot[2] < -0.99).unwrap();
        assert!(close(spot.color[1], 400.0 / 255.0) && spot.color[3] == 2.0);
        assert!(close(spot.cone[0], 0.5));
        assert_eq!(spot.cone[3], 0.0, "a Source light shades physically");

        let sun = find(0.0, 3.0);
        assert!(close(sun.pos[2], 1.0), "points toward the sun");
        assert!(close(scene.ambient[0], 0.2));

        let aimed = *scene.lights.iter().find(|l| l.spot[1] > 0.99).unwrap();
        assert!(close(aimed.color[0], 300.0 / 255.0) && aimed.color[3] == 2.0, "bad brightness falls back to 300");
        assert_eq!(scene.lights.len(), 4);
        assert!(scene.sky.is_none(), "a Source sun brings no sky");
    }

    #[test]
    fn reads_quake_lightmap_keys() {
        let mut map = Map::new();
        let mut e = map.edit();
        let mut world = Entity::new("worldspawn");
        for (k, v) in [("light", "64"), ("_minlight_color", "255 128 0"), ("_sunlight", "150"), ("_sun_mangle", "90 -45 0"), ("_sunlight_color", "0.5 0.5 1")] {
            world.set(k, v);
        }
        e.set_kind(ROOT, NodeKind::World(world)).unwrap();
        let mut torch = Entity::new("light_torch_small_walltorch");
        for (k, v) in [("_color", "128 128 128"), ("delay", "5"), ("wait", "2"), ("_anglescale", "3")] {
            torch.set(k, v);
        }
        e.insert(ROOT, NodeKind::Entity(torch)).unwrap();
        map.apply(e.finish());
        let scene = build_scene(&map, &BTreeSet::new(), &BTreeSet::new(), &|_| None, &NoStyle, false, None);
        let close = |a: f32, b: f32| (a - b).abs() < 1e-4;

        let torch = scene.lights.iter().find(|l| l.pos[3] == 1.0).unwrap();
        assert!(close(torch.color[0], 128.0 / 255.0 * 300.0 / 255.0), "Quake colors stay gamma encoded, as the lightmap is");
        assert!(torch.color[3] == 5.0 && close(torch.spot[3], 64.0), "delay 5 falls off over 128 units / wait");
        assert_eq!(torch.cone[2], 1.0, "the angle scale is at most 1");

        assert!(close(scene.minlight[0], 64.0 / 255.0) && close(scene.minlight[1], 128.0 / 255.0 * 64.0 / 255.0) && scene.minlight[2] == 0.0);
        let sun = scene.lights.iter().find(|l| l.pos[3] == 0.0).unwrap();
        assert!(close(sun.pos[1], -0.70710677) && close(sun.pos[2], 0.70710677), "points back up along the sunlight");
        assert!(close(sun.color[0], 0.5 * 150.0 / 255.0) && close(sun.color[2], 150.0 / 255.0) && sun.color[3] == 3.0);
        assert_eq!(sun.cone[2..], [0.5, 1.0]);
    }

    #[test]
    fn reads_the_godot_environment() {
        let mut map = Map::new();
        let mut e = map.edit();
        let mut ent = Entity::default();
        ent.set("classname", "light_environment");
        ent.set("angles", "90 0 0");
        ent.set("sun_energy", "2");
        ent.set("sky_top_color", "255 0 0");
        e.insert(ROOT, NodeKind::Entity(ent)).unwrap();
        map.apply(e.finish());
        let scene = build_scene(&map, &BTreeSet::new(), &BTreeSet::new(), &|_| None, &NoStyle, false, None);
        let sun = scene.lights[0];
        assert!(sun.pos[3] == 0.0 && (sun.pos[2] - 1.0).abs() < 1e-4, "pitch 90 shines straight down");
        assert!((sun.color[0] - 2.0).abs() < 1e-4, "the default white sun at energy 2");
        let sky = scene.sky.unwrap();
        assert_eq!(sky.top, [1.0, 0.0, 0.0]);
        assert!((sky.horizon[0] - 0.72f32.powf(2.2)).abs() < 1e-4 && sky.ambient == 1.0, "unset keys take the addon's defaults");
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

        let scene = build_scene(&map, &BTreeSet::new(), &BTreeSet::new(), &|_| None, &style, false, None);
        assert_eq!(scene.points[&BatchKey::from("quad.glb#0")].len(), 6, "two triangles under the model's texture");
        assert!(!scene.points.contains_key(&BatchKey::default()), "no box");
        assert_eq!(scene.edges.len(), 10, "five wireframe edges");

        let scene = build_scene(&map, &BTreeSet::from([id]), &BTreeSet::new(), &|_| None, &style, false, None);
        assert_eq!(scene.selected_edges.len(), 24, "a selected model is boxed");
    }
}
