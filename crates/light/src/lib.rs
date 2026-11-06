//! Lightmap baking: one lightmap chart per drawn brush face, lit by the map's light entities, sun and sky,
//! with bounce light, stored as the L1 spherical harmonics Godot's directional lightmaps use.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod bake;
mod bvh;
pub mod charts;
mod file;
pub mod lights;
pub mod rgbe;

use std::sync::atomic::{AtomicBool, AtomicU64};

use mapcraft_map::{Map, NodeId, NodeKind};

pub use bake::{bake, godot_axes, shade};
pub use file::{fingerprint_of, read, write};

#[derive(Debug, thiserror::Error)]
pub enum BakeError {
    #[error("the bake was cancelled")]
    Cancelled,
    #[error("a bake thread failed")]
    Worker,
    #[error("not a MapCraft lightmap file")]
    NotLightmap,
    #[error("lightmap file version {0} is not the version this MapCraft bakes; bake the lighting again")]
    Version(u32),
    #[error("the lightmap file is damaged: {0}")]
    Damaged(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// World units per luxel unless a face's `lightmapscale` says otherwise.
    pub luxel: f64,
    /// Atlas slices are this many luxels square.
    pub slice: u32,
    /// Light bounces after the direct pass; 0 still gathers the sky and emissive faces once.
    pub bounces: u32,
    /// Rays each luxel gathers per bounce.
    pub rays: u32,
    /// Shadow rays toward the sun per luxel, spread over its disk.
    pub sun_samples: u32,
    /// Worker threads; 0 uses every core.
    pub threads: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self { luxel: 32.0, slice: 2048, bounces: 1, rays: 32, sun_samples: 8, threads: 0 }
    }
}

impl Settings {
    /// These settings with worldspawn's `_lightmap_scale` and `_bounce` applied.
    pub fn for_map(mut self, map: &Map) -> Self {
        let world = map.world();
        let num = |k: &str| world.get(k).and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite());
        if let Some(v) = num("_lightmap_scale").filter(|v| *v >= 1.0) {
            self.luxel = v.min(4096.0);
        }
        if let Some(v) = num("_bounce").filter(|v| *v >= 0.0) {
            self.bounces = v.min(16.0) as u32;
        }
        self
    }
}

/// How a material takes and gives light, in linear color.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Surface {
    pub albedo: [f32; 3],
    pub emission: [f32; 3],
}

impl Default for Surface {
    fn default() -> Self {
        Self { albedo: [0.5; 3], emission: [0.0; 3] }
    }
}

/// Where one face's luxels sit.
#[derive(Clone, Debug, PartialEq)]
pub struct FaceMap {
    pub node: NodeId,
    /// The face's index in its brush.
    pub face: u32,
    pub slice: u32,
    /// Rows taking a world point (x, y, z, 1) to its UV in the slice, 0 to 1.
    pub uv: [[f32; 4]; 2],
}

impl FaceMap {
    pub fn uv_at(&self, p: [f64; 3]) -> [f32; 2] {
        self.uv.map(|r| (f64::from(r[0]) * p[0] + f64::from(r[1]) * p[1] + f64::from(r[2]) * p[2] + f64::from(r[3])) as f32)
    }
}

/// One atlas slice: Godot's four layers (L0, then the L1 bands for y, z and x), RGBE9995 texels row by row.
#[derive(Clone, Debug, PartialEq)]
pub struct Slice {
    pub layers: [Vec<u32>; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Lightmap {
    /// [`fingerprint`] of the map it was baked from.
    pub fingerprint: u64,
    /// Slices are this many texels square.
    pub size: u32,
    pub slices: Vec<Slice>,
    pub faces: Vec<FaceMap>,
}

impl Lightmap {
    pub fn face(&self, node: NodeId, face: usize) -> Option<&FaceMap> {
        self.faces.iter().find(|f| f.node == node && f.face as usize == face)
    }
}

/// What Godot shows on each lightmapped face with no normal map: per slice, RGBE9995 texels of the
/// spherical harmonics evaluated at the face's normal.
pub fn irradiance(lm: &Lightmap, map: &Map) -> Vec<Vec<u32>> {
    let size = lm.size as usize;
    let mut out = vec![vec![0u32; size * size]; lm.slices.len()];
    let mut solid: Option<(NodeId, mapcraft_geom::Solid)> = None;
    for f in &lm.faces {
        let Some(brush) = map.get(f.node).and_then(|n| n.kind.brush()) else { continue };
        if solid.as_ref().is_none_or(|(id, _)| *id != f.node) {
            solid = brush.solid().ok().map(|s| (f.node, s));
        }
        let Some((_, s)) = &solid else { continue };
        let (Some(face), Some(Some(w))) = (brush.faces.get(f.face as usize), s.faces.get(f.face as usize)) else { continue };
        let Some(normal) = face.plane().map(|p| p.normal) else { continue };
        let (Some(slice), Some(dest)) = (lm.slices.get(f.slice as usize), out.get_mut(f.slice as usize)) else { continue };
        let texel = |p: &mapcraft_geom::DVec3| f.uv_at(p.to_array()).map(|v| f64::from(v) * size as f64);
        let (lo, hi) =
            w.0.iter()
                .map(texel)
                .fold(([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]), |(lo, hi), t| ([lo[0].min(t[0]), lo[1].min(t[1])], [hi[0].max(t[0]), hi[1].max(t[1])]));
        // A face whose light is the same everywhere owns only the one texel its mapping points at.
        let pad = if f.uv.iter().all(|r| r[..3] == [0.0; 3]) { 0.0 } else { 1.0 };
        let range = |a: f64, b: f64| {
            let first = (a.floor() - pad).clamp(0.0, size as f64) as usize;
            first..((b.ceil() + pad).clamp(0.0, size as f64) as usize).max(first)
        };
        for y in range(lo[1], hi[1]) {
            for x in range(lo[0], hi[0]) {
                let at = y * size + x;
                let texels = std::array::from_fn(|k| rgbe::decode(slice.layers[k].get(at).copied().unwrap_or_default()));
                if let Some(d) = dest.get_mut(at) {
                    *d = rgbe::encode(shade(&texels, normal));
                }
            }
        }
    }
    out
}

/// A bake's progress, shared with the thread watching it.
#[derive(Debug, Default)]
pub struct Progress {
    pub done: AtomicU64,
    pub total: AtomicU64,
    pub cancel: AtomicBool,
}

impl Progress {
    /// From 0 to 1.
    pub fn fraction(&self) -> f32 {
        use std::sync::atomic::Ordering::Relaxed;
        let total = self.total.load(Relaxed);
        if total == 0 { 0.0 } else { (self.done.load(Relaxed) as f64 / total as f64).min(1.0) as f32 }
    }
}

/// FNV-1a, stable across builds and platforms so files can compare it.
struct Fnv(u64);

impl Fnv {
    fn bytes(&mut self, b: &[u8]) {
        for x in b {
            self.0 = (self.0 ^ u64::from(*x)).wrapping_mul(0x0100_0000_01B3);
        }
    }

    fn f64(&mut self, v: f64) {
        self.bytes(&v.to_bits().to_le_bytes());
    }

    fn str(&mut self, s: &str) {
        self.bytes(s.as_bytes());
        self.bytes(&[0xFF]);
    }
}

/// What a bake's file records: [`fingerprint`] of the map with the settings it was baked at, so a change to either
/// marks the saved bake out of date.
pub fn bake_fingerprint(map: &Map, settings: &Settings) -> u64 {
    let mut h = Fnv(fingerprint(map));
    h.bytes(b"bands on Godot's axes");
    h.bytes(b"displacements lightmapped");
    h.f64(settings.luxel);
    h.bytes(&settings.slice.to_le_bytes());
    h.bytes(&settings.bounces.to_le_bytes());
    h.bytes(&settings.rays.to_le_bytes());
    h.bytes(&settings.sun_samples.to_le_bytes());
    h.0
}

/// A fingerprint of everything a bake reads: brush faces and their materials, light entities and worldspawn.
pub fn fingerprint(map: &Map) -> u64 {
    let mut h = Fnv(0xCBF2_9CE4_8422_2325);
    for (id, node) in map.nodes() {
        match &node.kind {
            NodeKind::Brush(b) => {
                h.bytes(&id.0.to_le_bytes());
                for f in &b.faces {
                    f.points.iter().flat_map(|p| p.to_array()).for_each(|v| h.f64(v));
                    h.str(&f.material);
                    h.str(f.attrs.get("lightmapscale").map_or("", String::as_str));
                }
            }
            NodeKind::Entity(e) | NodeKind::World(e) if e.classname.starts_with("light") || matches!(node.kind, NodeKind::World(_)) => {
                h.str(&e.classname);
                for (k, v) in e.properties.iter() {
                    h.str(k);
                    h.str(v);
                }
            }
            _ => {}
        }
    }
    h.0
}
