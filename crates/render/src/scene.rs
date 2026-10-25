use std::collections::{BTreeMap, BTreeSet};

use mapcraft_geom::{DVec2, DVec3};
use mapcraft_map::{FaceId, Map, NodeId, NodeKind};

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MeshVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub tint: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LineVertex {
    pub pos: [f32; 3],
    pub color: [f32; 4],
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
}

/// Hammer's convention: `tools/...` materials mark invisible brushes such as triggers and clips.
pub fn is_tool_material(name: &str) -> bool {
    name.get(..6).is_some_and(|p| p.eq_ignore_ascii_case("tools/"))
}

const TOOL_ALPHA: f32 = 0.3;

pub const WORLD_EDGE: [f32; 4] = [0.85, 0.85, 0.85, 1.0];
pub const SELECTED_EDGE: [f32; 4] = [1.0, 0.1, 0.1, 1.0];
const SELECTED_TINT: [f32; 4] = [1.0, 0.62, 0.55, 1.0];
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
                for (face, winding) in b.faces.iter().zip(&solid.faces) {
                    let Some(w) = winding else { continue };
                    let normal = face.plane().map_or(DVec3::Z, |p| p.normal).as_vec3().to_array();
                    let size = texture_size(&face.material).map_or(DVec2::splat(64.0), |s| DVec2::new(f64::from(s[0]), f64::from(s[1])));
                    let face_selected = faces.contains(&(id, face.id));
                    let tool = is_tool_material(&face.material);
                    let tint = match (selected || face_selected, tool) {
                        (true, false) => SELECTED_TINT,
                        (false, false) => WHITE,
                        (true, true) => [SELECTED_EDGE[0], SELECTED_EDGE[1], SELECTED_EDGE[2], TOOL_ALPHA + 0.15],
                        (false, true) => {
                            let c = entity_color.unwrap_or([0.8, 0.5, 0.1, 1.0]);
                            [c[0], c[1], c[2], TOOL_ALPHA]
                        }
                    };
                    let verts = if tool { &mut scene.translucent } else { scene.batches.entry(face.material.clone()).or_default() };
                    let v = |p: DVec3| MeshVertex { pos: p.as_vec3().to_array(), normal, uv: face.uv(p, size).as_vec2().to_array(), tint };
                    for i in 1..w.0.len().saturating_sub(1) {
                        verts.extend([v(w.0[0]), v(w.0[i]), v(w.0[i + 1])]);
                    }
                    let edges = if selected || face_selected { &mut scene.selected_edges } else { &mut scene.edges };
                    let edge = if face_selected { SELECTED_EDGE } else { edge };
                    for i in 0..w.0.len() {
                        edges.push(LineVertex::new(w.0[i], edge));
                        edges.push(LineVertex::new(w.0[(i + 1) % w.0.len()], edge));
                    }
                }
            }
            NodeKind::Entity(e) if node.children.is_empty() => {
                let origin = e.origin().unwrap_or_default();
                let (min, max) = style.size(&e.classname).unwrap_or((DVec3::splat(-8.0), DVec3::splat(8.0)));
                let color = linear(style.color(&e.classname).unwrap_or(POINT_ENTITY));
                let (min, max) = (origin + min, origin + max);
                let tint = if selected { SELECTED_TINT } else { color };
                let verts = scene.batches.entry(String::new()).or_default();
                push_box(verts, min, max, tint);
                let edges = if selected { &mut scene.selected_edges } else { &mut scene.edges };
                box_lines(edges, min, max, if selected { SELECTED_EDGE } else { color });
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
        let v = |p: DVec3| MeshVertex { pos: p.as_vec3().to_array(), normal: n.as_vec3().to_array(), uv: [0.0, 0.0], tint };
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
        let uv_max = scene.batches["a"].iter().map(|v| v.uv[0].abs().max(v.uv[1].abs())).fold(0.0f32, f32::max);
        assert!((uv_max - 1.0).abs() < 1e-6);

        let scene = build_scene(&map, &BTreeSet::from([b]), &BTreeSet::new(), &|_| None, &NoStyle, true);
        assert_eq!(scene.selected_edges.len(), 48);
    }
}
