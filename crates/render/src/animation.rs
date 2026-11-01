//! glTF node and skin animation, posed on the CPU for the selected entities that play one.

use std::sync::Arc;

use gltf_json as json;
use json::animation::{Interpolation, Property};
use json::validation::Checked;
use mapcraft_geom::{DMat3, DMat4, DQuat, DVec3, DVec4};
use mapcraft_map::{Entity, NodeId};

use crate::model::{Doc, MAX_DEPTH, MAX_NODES, Model, flat_normals, to_map};

/// Models with more vertices than this stay in their rest pose.
pub const MAX_ANIMATED_VERTICES: usize = 1 << 18;
const MAX_KEYS: usize = 1 << 22;
const MAX_JOINTS: usize = 1024;
const MAX_CLIPS: usize = 1024;

pub const ANIMATION_KEY: &str = "animation";
pub const ANIMATION_LOOP_KEY: &str = "animation_loop";

/// How an animation repeats, from the entity's `animation_loop` key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Repeat {
    #[default]
    Loop,
    PingPong,
    Once,
}

impl Repeat {
    pub fn of(e: &Entity) -> Self {
        match e.get(ANIMATION_LOOP_KEY).map(str::trim) {
            Some(v) if v.eq_ignore_ascii_case("pingpong") => Self::PingPong,
            Some(v) if v.eq_ignore_ascii_case("none") => Self::Once,
            _ => Self::Loop,
        }
    }

    /// The clip time `t` seconds into playing a clip `duration` seconds long.
    pub fn clip_time(self, t: f64, duration: f64) -> f64 {
        if !(duration > 0.0 && duration.is_finite() && t.is_finite()) {
            return 0.0;
        }
        let t = t.max(0.0);
        match self {
            Self::Once => t.min(duration),
            Self::Loop => t % duration,
            Self::PingPong => {
                let p = t % (2.0 * duration);
                if p > duration { 2.0 * duration - p } else { p }
            }
        }
    }

    /// Whether the pose still changes `t` seconds in.
    pub fn moving(self, t: f64, duration: f64) -> bool {
        duration > 0.0 && (self != Self::Once || t < duration)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Path {
    Translation,
    Rotation,
    Scale,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Curve {
    Linear,
    Step,
    Cubic,
}

#[derive(Clone, Debug, PartialEq)]
struct Channel {
    node: usize,
    path: Path,
    curve: Curve,
    /// Non-decreasing, at least one.
    times: Vec<f64>,
    /// One per time, or in-tangent, value and out-tangent per time for `Cubic`.
    values: Vec<DVec4>,
}

fn quat(v: DVec4) -> DQuat {
    let q = DQuat::from_vec4(v);
    if q.is_finite() && q.length_squared() > 0.0 { q.normalize() } else { DQuat::IDENTITY }
}

impl Channel {
    fn sample(&self, t: f64) -> DVec4 {
        let cubic = self.curve == Curve::Cubic;
        let value = |k: usize| self.values.get(if cubic { 3 * k + 1 } else { k }).copied().unwrap_or_default();
        let next = self.times.partition_point(|x| *x <= t);
        let (Some(k), Some(t0), Some(t1)) = (next.checked_sub(1), next.checked_sub(1).and_then(|k| self.times.get(k)), self.times.get(next)) else {
            return value(next.min(self.times.len().saturating_sub(1)));
        };
        let dt = t1 - t0;
        let s = if dt > 0.0 { (t - t0) / dt } else { 0.0 };
        let (v0, v1) = (value(k), value(next));
        match (self.curve, self.path) {
            (Curve::Step, _) => v0,
            (Curve::Linear, Path::Rotation) => quat(v0).slerp(quat(v1), s).into(),
            (Curve::Linear, _) => v0.lerp(v1, s),
            (Curve::Cubic, _) => {
                let out0 = self.values.get(3 * k + 2).copied().unwrap_or_default() * dt;
                let in1 = self.values.get(3 * next).copied().unwrap_or_default() * dt;
                let (s2, s3) = (s * s, s * s * s);
                let v = v0 * (2.0 * s3 - 3.0 * s2 + 1.0) + out0 * (s3 - 2.0 * s2 + s) + v1 * (3.0 * s2 - 2.0 * s3) + in1 * (s3 - s2);
                if self.path == Path::Rotation { quat(v).into() } else { v }
            }
        }
    }
}

/// A named animation.
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub name: String,
    /// Seconds to its last keyframe.
    pub duration: f64,
    channels: Vec<Channel>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Skin {
    joints: Vec<usize>,
    inverse_bind: Vec<DMat4>,
}

/// A model part's vertices in its mesh's own space, as the file has them.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Source {
    node: usize,
    skin: Option<usize>,
    positions: Vec<DVec3>,
    normals: Vec<DVec3>,
    joints: Vec<[usize; 4]>,
    /// Summing to 1, or all 0 for a vertex that stays put.
    weights: Vec<[f64; 4]>,
}

impl Source {
    pub fn read(doc: &Doc, prim: &json::mesh::Primitive, node: usize, positions: &[DVec3], tris: &[[u32; 3]]) -> Self {
        let n = positions.len();
        let attr =
            |s: json::mesh::Semantic, width: usize| prim.attributes.get(&Checked::Valid(s)).and_then(|a| doc.read(*a, width)).filter(|v| v.len() == width * n);
        let normals = match attr(json::mesh::Semantic::Normals, 3) {
            Some(v) => v.as_chunks::<3>().0.iter().map(|c| DVec3::from_array(*c)).collect(),
            None => flat_normals(positions, tris).iter().map(|c| DVec3::from_array(c.map(f64::from))).collect(),
        };
        let skin = doc.root.nodes.get(node).and_then(|n| n.skin).map(|s| s.value());
        let joints = attr(json::mesh::Semantic::Joints(0), 4).filter(|_| skin.is_some());
        let weights = attr(json::mesh::Semantic::Weights(0), 4).filter(|_| skin.is_some());
        let (joints, weights) = match (joints, weights) {
            (Some(j), Some(w)) => (
                j.as_chunks::<4>().0.iter().map(|c| c.map(|x| x as usize)).collect(),
                w.as_chunks::<4>()
                    .0
                    .iter()
                    .map(|c| {
                        let c = c.map(|x| x.max(0.0));
                        let sum: f64 = c.iter().sum();
                        if sum > 0.0 { c.map(|x| x / sum) } else { [0.0; 4] }
                    })
                    .collect(),
            ),
            _ => (Vec::new(), Vec::new()),
        };
        Self { node, skin, positions: positions.to_vec(), normals, joints, weights }
    }
}

/// A part's posed positions and normals, relative to the entity like `ModelPart`'s.
pub type Pose = (Vec<[f32; 3]>, Vec<[f32; 3]>);

/// A model's node tree, skins and clips, which pose its parts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rig {
    /// Each node's rest translation, rotation and scale.
    rest: Vec<(DVec3, DQuat, DVec3)>,
    /// The scene's nodes with their parents, each parent before its children.
    order: Vec<(usize, Option<usize>)>,
    to_map: DMat4,
    skins: Vec<Skin>,
    /// One per `Model::parts`.
    sources: Vec<Source>,
    pub clips: Vec<Clip>,
}

impl Rig {
    /// `None` when the model is too big to pose.
    pub(crate) fn build(doc: &Doc, units_per_meter: f64, sources: Vec<Source>) -> Option<Self> {
        let root = &doc.root;
        let count = root.nodes.len();
        if count > MAX_NODES || sources.iter().map(|s| s.positions.len()).sum::<usize>() > MAX_ANIMATED_VERTICES {
            return None;
        }
        let rest = root
            .nodes
            .iter()
            .map(|n| match n.matrix {
                Some(m) => {
                    let (s, r, t) = DMat4::from_cols_array(&m.map(f64::from)).to_scale_rotation_translation();
                    (t, r, s)
                }
                None => {
                    let v = |a: Option<[f32; 3]>, d: f64| a.map_or(DVec3::splat(d), |a| DVec3::from_array(a.map(f64::from)));
                    let r = n.rotation.map_or(DVec4::W, |q| DVec4::from_array(q.0.map(f64::from)));
                    (v(n.translation, 0.0), quat(r), v(n.scale, 1.0))
                }
            })
            .collect();
        let mut order = Vec::new();
        let mut stack: Vec<(usize, Option<usize>, usize)> = doc.scene()?.nodes.iter().take(MAX_NODES).map(|n| (n.value(), None, 0)).collect();
        while let Some((n, parent, depth)) = stack.pop() {
            let Some(node) = root.nodes.get(n).filter(|_| depth < MAX_DEPTH && order.len() < MAX_NODES) else { continue };
            order.push((n, parent));
            let room = MAX_NODES.saturating_sub(stack.len());
            stack.extend(node.children.iter().flatten().take(room).map(|c| (c.value(), Some(n), depth + 1)));
        }
        let skins = root
            .skins
            .iter()
            .map(|s| {
                let joints: Vec<usize> = s.joints.iter().map(|j| j.value()).collect();
                let inverse_bind = match s.inverse_bind_matrices {
                    Some(a) => doc.read(a, 16).unwrap_or_default().as_chunks::<16>().0.iter().map(DMat4::from_cols_array).collect(),
                    None => vec![DMat4::IDENTITY; joints.len()],
                };
                let usable = joints.len() <= MAX_JOINTS && inverse_bind.len() == joints.len() && joints.iter().all(|j| *j < count);
                if usable { Skin { joints, inverse_bind } } else { Skin::default() }
            })
            .collect();
        let mut keys = 0usize;
        let mut clips = Vec::new();
        for anim in root.animations.iter().take(MAX_CLIPS) {
            let Some(name) = anim.name.clone() else { continue };
            let mut channels = Vec::new();
            for ch in &anim.channels {
                let Some(channel) = read_channel(doc, anim, ch, count) else { continue };
                keys = keys.saturating_add(channel.values.len());
                if keys > MAX_KEYS {
                    return None;
                }
                channels.push(channel);
            }
            let duration = channels.iter().filter_map(|c| c.times.last()).fold(0.0, |a: f64, b| a.max(*b));
            if !channels.is_empty() {
                clips.push(Clip { name, duration, channels });
            }
        }
        Some(Self { rest, order, to_map: to_map(units_per_meter), skins, sources, clips })
    }

    /// Each part in the nodes' rest pose, one per `Model::parts`.
    pub fn rest_pose(&self) -> Option<Vec<Pose>> {
        self.posed(&[], 0.0)
    }

    /// Each part posed by `clip` at `time` seconds into it, one per `Model::parts`.
    pub fn pose(&self, clip: usize, time: f64) -> Option<Vec<Pose>> {
        self.posed(&self.clips.get(clip)?.channels, time)
    }

    fn posed(&self, channels: &[Channel], time: f64) -> Option<Vec<Pose>> {
        let mut local = self.rest.clone();
        for ch in channels {
            let Some(l) = local.get_mut(ch.node) else { continue };
            let v = ch.sample(time);
            match ch.path {
                Path::Translation => l.0 = v.truncate(),
                Path::Rotation => l.1 = quat(v),
                Path::Scale => l.2 = v.truncate(),
            }
        }
        let mut global = vec![self.to_map; local.len()];
        for &(n, parent) in &self.order {
            let above = parent.and_then(|p| global.get(p)).copied().unwrap_or(self.to_map);
            if let (Some(&(t, r, s)), Some(g)) = (local.get(n), global.get_mut(n)) {
                *g = above * DMat4::from_scale_rotation_translation(s, r, t);
            }
        }
        let at = |n: usize| global.get(n).copied().unwrap_or(self.to_map);
        let palettes: Vec<Vec<DMat4>> = self.skins.iter().map(|s| s.joints.iter().zip(&s.inverse_bind).map(|(j, b)| at(*j) * *b).collect()).collect();
        let posed = self.sources.iter().map(|src| {
            let rigid = at(src.node);
            let rigid_normals = DMat3::from_mat4(rigid).inverse().transpose();
            let palette = src.skin.and_then(|s| palettes.get(s));
            let mut positions = Vec::with_capacity(src.positions.len());
            let mut normals = Vec::with_capacity(src.positions.len());
            for (i, p) in src.positions.iter().enumerate() {
                let n = src.normals.get(i).copied().unwrap_or(DVec3::Z);
                let (p, n) = match palette {
                    Some(palette) => {
                        let m = skin_matrix(palette, src.joints.get(i), src.weights.get(i)).unwrap_or(self.to_map);
                        (m.transform_point3(*p), DMat3::from_mat4(m) * n)
                    }
                    None => (rigid.transform_point3(*p), rigid_normals * n),
                };
                positions.push(p.as_vec3().to_array());
                normals.push(n.normalize_or(DVec3::Z).as_vec3().to_array());
            }
            (positions, normals)
        });
        Some(posed.collect())
    }
}

fn skin_matrix(palette: &[DMat4], joints: Option<&[usize; 4]>, weights: Option<&[f64; 4]>) -> Option<DMat4> {
    let (joints, weights) = (joints?, weights?);
    let mut m = DMat4::ZERO;
    let mut used = 0.0;
    for (j, w) in joints.iter().zip(weights).filter(|(_, w)| **w > 0.0) {
        if let Some(joint) = palette.get(*j) {
            m += *joint * *w;
            used += w;
        }
    }
    (used > 0.0).then(|| m * (1.0 / used))
}

fn read_channel(doc: &Doc, anim: &json::Animation, ch: &json::animation::Channel, nodes: usize) -> Option<Channel> {
    let node = ch.target.node.value();
    let path = match ch.target.path {
        Checked::Valid(Property::Translation) => Path::Translation,
        Checked::Valid(Property::Rotation) => Path::Rotation,
        Checked::Valid(Property::Scale) => Path::Scale,
        _ => return None,
    };
    let sampler = anim.samplers.get(ch.sampler.value()).filter(|_| node < nodes)?;
    let curve = match sampler.interpolation {
        Checked::Valid(Interpolation::Step) => Curve::Step,
        Checked::Valid(Interpolation::CubicSpline) => Curve::Cubic,
        _ => Curve::Linear,
    };
    let times = doc.read(sampler.input, 1)?;
    if times.is_empty() || !times.windows(2).all(|w| w[0] <= w[1]) {
        return None;
    }
    let width = if path == Path::Rotation { 4 } else { 3 };
    let raw = doc.read(sampler.output, width)?;
    let values: Vec<DVec4> = raw.chunks_exact(width).map(|c| DVec4::new(c[0], c[1], c[2], c.get(3).copied().unwrap_or(0.0))).collect();
    let per = if curve == Curve::Cubic { 3 } else { 1 };
    (values.len() == times.len().checked_mul(per)?).then_some(Channel { node, path, curve, times, values })
}

/// The clip of `model` that `e`'s `animation` key names, when the model can play it.
pub fn playing_clip(model: &Model, e: &Entity) -> Option<usize> {
    let name = e.get(ANIMATION_KEY)?.trim();
    model.rig.as_ref()?.clips.iter().position(|c| c.name == name)
}

/// A selected entity playing its model's animation in the 3D views.
#[derive(Clone, Debug)]
pub struct Playing {
    pub id: NodeId,
    pub model: Arc<Model>,
    pub at: DMat4,
    pub clip: usize,
    pub repeat: Repeat,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(path: Path, curve: Curve, times: &[f64], values: &[[f64; 4]]) -> Channel {
        Channel { node: 0, path, curve, times: times.to_vec(), values: values.iter().map(|v| DVec4::from_array(*v)).collect() }
    }

    #[test]
    fn samples_step_linear_and_cubic_curves() {
        let keys = [[0.0, 0.0, 0.0, 0.0], [10.0, 20.0, 0.0, 0.0]];
        let linear = channel(Path::Translation, Curve::Linear, &[1.0, 3.0], &keys);
        assert_eq!(linear.sample(0.0), DVec4::ZERO, "before the first key holds it");
        assert_eq!(linear.sample(2.0), DVec4::new(5.0, 10.0, 0.0, 0.0));
        assert_eq!(linear.sample(9.0), DVec4::new(10.0, 20.0, 0.0, 0.0), "past the last key holds it");
        let step = channel(Path::Translation, Curve::Step, &[1.0, 3.0], &keys);
        assert_eq!(step.sample(2.9), DVec4::ZERO);
        assert_eq!(step.sample(3.0).x, 10.0);
        // In-tangent, value, out-tangent per key; zero tangents ease in and out.
        let cubic = channel(Path::Translation, Curve::Cubic, &[0.0, 1.0], &[[0.0; 4], [0.0; 4], [0.0; 4], [0.0; 4], [1.0, 0.0, 0.0, 0.0], [0.0; 4]]);
        assert!((cubic.sample(0.5).x - 0.5).abs() < 1e-12);
        assert!((cubic.sample(0.25).x - 0.15625).abs() < 1e-12);
        let half = std::f64::consts::FRAC_1_SQRT_2;
        let turn = channel(Path::Rotation, Curve::Linear, &[0.0, 1.0], &[[0.0, 0.0, 0.0, 1.0], [0.0, 0.0, half, half]]);
        let q = quat(turn.sample(0.5));
        assert!(q.angle_between(DQuat::from_rotation_z(std::f64::consts::FRAC_PI_4)) < 1e-9, "rotations slerp: {q}");
    }

    #[test]
    fn loops_ping_pongs_and_holds() {
        assert_eq!(Repeat::Loop.clip_time(2.5, 2.0), 0.5);
        assert_eq!(Repeat::PingPong.clip_time(1.5, 2.0), 1.5);
        assert_eq!(Repeat::PingPong.clip_time(2.5, 2.0), 1.5, "comes back down");
        assert_eq!(Repeat::PingPong.clip_time(4.5, 2.0), 0.5, "and up again");
        assert_eq!(Repeat::Once.clip_time(7.0, 2.0), 2.0, "holds the last frame");
        assert!(!Repeat::Once.moving(7.0, 2.0) && Repeat::Loop.moving(7.0, 2.0));
        assert_eq!(Repeat::Loop.clip_time(f64::NAN, 2.0), 0.0);
        assert_eq!(Repeat::Loop.clip_time(3.0, 0.0), 0.0);
        let mut e = Entity::new("prop");
        assert_eq!(Repeat::of(&e), Repeat::Loop);
        e.set(ANIMATION_LOOP_KEY, "pingpong");
        assert_eq!(Repeat::of(&e), Repeat::PingPong);
        e.set(ANIMATION_LOOP_KEY, "none");
        assert_eq!(Repeat::of(&e), Repeat::Once);
    }

    /// Two joints in a row along glTF x; the second bends 90 degrees about z over one second.
    fn arm_glb(times: &str) -> Vec<u8> {
        let mut bin: Vec<u8> = Vec::new();
        let mut f32s = |v: &[f32]| v.iter().for_each(|x| bin.extend(x.to_le_bytes()));
        // Positions: a triangle at the root joint, and one vertex at x 2 on the second joint.
        f32s(&[0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 2.0, 0.0, 0.0]);
        f32s(&[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]);
        // Inverse bind matrices: identity, then a move back by x 1.
        f32s(&[1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
        f32s(&[1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, -1.0, 0.0, 0.0, 1.0]);
        let half = std::f32::consts::FRAC_1_SQRT_2;
        f32s(&[0.0, 1.0]);
        f32s(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, half, half]);
        bin.extend([0u8, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0]);
        let json = r#"{
            "asset": {"version": "2.0"},
            "scene": 0,
            "scenes": [{"nodes": [0, 1]}],
            "nodes": [
                {"mesh": 0, "skin": 0},
                {"children": [2]},
                {"translation": [1, 0, 0]}
            ],
            "skins": [{"joints": [1, 2], "inverseBindMatrices": 2}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "WEIGHTS_0": 1, "JOINTS_0": 5}}]}],
            "animations": [{"name": "Bend", "samplers": [{"input": 3, "output": 4}], "channels": [{"sampler": 0, "target": {"node": 2, "path": "rotation"}}]}],
            "buffers": [{"byteLength": 264}],
            "bufferViews": [
                {"buffer": 0, "byteOffset": 0, "byteLength": 36},
                {"buffer": 0, "byteOffset": 36, "byteLength": 48},
                {"buffer": 0, "byteOffset": 84, "byteLength": 128},
                {"buffer": 0, "byteOffset": 212, "byteLength": 8},
                {"buffer": 0, "byteOffset": 220, "byteLength": 32},
                {"buffer": 0, "byteOffset": 252, "byteLength": 12}
            ],
            "accessors": [
                {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"},
                {"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC4"},
                {"bufferView": 2, "componentType": 5126, "count": 2, "type": "MAT4"},
                {"bufferView": 3, "componentType": 5126, "count": TIMES, "type": "SCALAR"},
                {"bufferView": 4, "componentType": 5126, "count": 2, "type": "VEC4"},
                {"bufferView": 5, "componentType": 5121, "count": 3, "type": "VEC4"}
            ]
        }"#;
        crate::model::tests::glb(&json.replace("TIMES", times), &bin)
    }

    #[test]
    fn skins_a_bending_arm() {
        let model = crate::model::load_gltf("arm.glb", &arm_glb("2"), &|_| None, 1.0).unwrap();
        let rig = model.rig.as_ref().expect("the arm can play");
        assert_eq!(rig.clips[0].name, "Bend");
        assert_eq!(rig.clips[0].duration, 1.0);
        let tip = |t: f64| DVec3::from_array(rig.pose(0, t).unwrap()[0].0[2].map(f64::from));
        let rest = DVec3::from_array(model.parts[0].positions[2].map(f64::from));
        assert!((tip(0.0) - rest).length() < 1e-6, "the first frame matches the rest pose");
        // glTF (1, 1, 0) is map (1, 0, 1).
        assert!((tip(1.0) - DVec3::new(1.0, 0.0, 1.0)).length() < 1e-5, "the tip swings up: {}", tip(1.0));
        let root = DVec3::from_array(rig.pose(0, 1.0).unwrap()[0].0[1].map(f64::from));
        assert!((root - DVec3::new(0.0, 0.0, 1.0)).length() < 1e-6, "the root joint's vertex stays");
        assert!(rig.pose(1, 0.0).is_none());
    }

    #[test]
    fn broken_animation_data_stays_in_rest_pose() {
        let model = crate::model::load_gltf("arm.glb", &arm_glb("9"), &|_| None, 1.0).unwrap();
        assert!(model.rig.is_none(), "times past their buffer leave nothing to play");
        assert!(!model.parts.is_empty());
    }
}
