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
    /// Rays start this far off the face along its normal, in map units, so they never hit the face they leave.
    pub bias: f64,
    /// Samples along each side of a luxel, averaged: 1, 2 or 4.
    pub extra: u32,
    /// Luxels on every side each baked luxel is averaged with, at most 8.
    pub soft: u32,
    /// How much of a material's color tints the light it bounces, 0 to 1.
    pub bounce_color: f64,
    pub dirt: Option<Dirt>,
    /// Worker threads; 0 uses every core.
    pub threads: usize,
    /// Significant mantissa bits kept for the L0 layer, 4 to 9; 9 keeps every bit a texel holds.
    /// Fewer costs a little shading detail and makes the atlas deflate far smaller, since a bake's own noise is finer than that.
    pub l0_bits: u32,
    /// Significant mantissa bits kept for each of the three L1 bands, 4 to 9. The bands are a third of the atlas each and the least
    /// visible of the four, so they take the coarser cut by default.
    pub band_bits: u32,
    /// Bytes of baked lighting to aim at: a map whose faces hold more luxels than this gets a coarser `luxel` scale.
    /// The default is what one entry of a map's package can carry; 0 leaves a bake uncapped.
    pub budget: u64,
    /// Set when `budget` coarsened `luxel` past what the map asked for, which is what `asked_luxel` was.
    pub asked_luxel: Option<f64>,
}

/// What one entry of a `.mcpck` can carry, the size a bake aims at: `mapcraft_format::pack::MAX_ENTRY_SIZE`.
pub const DEFAULT_BUDGET: u64 = 64 << 20;

/// A baked luxel's four texels deflate to around this many bytes on a sunlit map, and to a good deal less on a dark or flat one.
const BUDGET_BYTES_PER_LUXEL: u64 = 8;

impl Default for Settings {
    fn default() -> Self {
        Self {
            luxel: 32.0,
            slice: 2048,
            bounces: 1,
            rays: 32,
            sun_samples: 8,
            bias: 0.5,
            extra: 1,
            soft: 0,
            bounce_color: 1.0,
            dirt: None,
            threads: 0,
            l0_bits: 8,
            band_bits: 7,
            budget: DEFAULT_BUDGET,
            asked_luxel: None,
        }
    }
}

/// Dirt: ambient occlusion that darkens corners and crevices.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dirt {
    /// How far a luxel looks for what crowds it, in map units.
    pub depth: f64,
    pub scale: f64,
    /// The exponent on the occlusion; below 1 darkens and reaches further from corners.
    pub gain: f64,
    /// Degrees from the face's normal the occlusion rays spread over.
    pub angle: f64,
}

impl Default for Dirt {
    fn default() -> Self {
        Self { depth: 128.0, scale: 1.0, gain: 1.0, angle: 88.0 }
    }
}

impl Settings {
    /// These settings with worldspawn's bake keys applied.
    pub fn for_map(mut self, map: &Map) -> Self {
        let world = map.world();
        let num = |k: &str| world.get(k).and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite());
        if let Some(v) = num("_lightmap_scale").filter(|v| *v >= 1.0) {
            self.luxel = v.min(4096.0);
        }
        if let Some(v) = num("_bounce").filter(|v| *v >= 0.0) {
            self.bounces = v.min(16.0) as u32;
        }
        if let Some(v) = num("_lightmap_bias").filter(|v| *v > 0.0) {
            self.bias = v.min(64.0);
        }
        if let Some(v) = num("_lightmap_extra") {
            self.extra = v.clamp(1.0, 4.0) as u32;
        }
        if let Some(v) = num("_lightmap_soft").filter(|v| *v >= 0.0) {
            self.soft = v.min(8.0) as u32;
        }
        if let Some(v) = num("_bouncecolorscale") {
            self.bounce_color = v.clamp(0.0, 1.0);
        }
        if let Some(on) = num("_dirt") {
            let d = Dirt::default();
            self.dirt = (on > 0.0).then(|| Dirt {
                depth: num("_dirtdepth").filter(|v| *v > 0.0).map_or(d.depth, |v| v.clamp(1.0, 4096.0)),
                scale: num("_dirtscale").filter(|v| *v >= 0.0).map_or(d.scale, |v| v.min(16.0)),
                gain: num("_dirtgain").filter(|v| *v > 0.0).map_or(d.gain, |v| v.clamp(0.01, 16.0)),
                angle: num("_dirtangle").filter(|v| *v > 0.0).map_or(d.angle, |v| v.clamp(1.0, 90.0)),
            });
        }
        if let Some(v) = world.get("_lightmap_bits") {
            self.keep_bits(v);
        }
        if let Some(v) = num("_lightmap_budget") {
            self.set_budget(v);
        }
        self.fit_budget(map);
        self
    }

    /// `_lightmap_bits` as a map writes it, or `--bits` as the command line takes it: one number keeps both layers at it,
    /// two give the L0 layer's and the bands'. Pulled inside the 4 to 9 bits a texel holds.
    pub fn keep_bits(&mut self, numbers: &str) {
        let keep = |v: f64| v.round().clamp(4.0, f64::from(rgbe::BITS)) as u32;
        let bits: Vec<f64> = numbers.split_whitespace().filter_map(|t| t.parse::<f64>().ok()).collect();
        if let Some(first) = bits.first() {
            let (l0, band) = (keep(*first), bits.get(1).map_or(keep(*first), |v| keep(*v)));
            (self.l0_bits, self.band_bits) = (l0, band);
        }
    }

    /// `_lightmap_budget` as a map writes it, or `--budget` as the command line takes it, in MiB: 0 leaves a bake uncapped.
    pub fn set_budget(&mut self, mib: f64) {
        self.budget = if mib <= 0.0 { 0 } else { (mib.clamp(1.0, 1024.0) as u64) << 20 };
    }

    /// Coarsens [`Settings::luxel`] until `map`'s faces hold no more luxels than [`Settings::budget`] carries, so a
    /// bake comes in under what a map's package can carry rather than outrunning it and being left behind.
    /// A face with its own `lightmapscale` keeps it, so a bake can still pass what it was aiming at; it says so.
    /// [`Settings::for_map`] calls this after reading a map's keys; call it again after changing the scale or the budget.
    pub fn fit_budget(&mut self, map: &Map) {
        self.asked_luxel = None;
        if self.budget == 0 {
            return;
        }
        let (asked, room) = (self.luxel, (self.budget / BUDGET_BYTES_PER_LUXEL).max(1));
        // The step coarsens by the square root of how far over it is, which is where the count lands in one go
        // unless wide faces are already capped at MAX_CHART or a face sets its own scale; then it stops trying.
        for _ in 0..6 {
            let held = charts::luxels(map, self.luxel);
            if held <= room || self.luxel >= 4096.0 {
                break;
            }
            self.luxel = (self.luxel * (held as f64 / room as f64).sqrt()).ceil().max(self.luxel + 1.0).min(4096.0);
        }
        if self.luxel > asked {
            self.asked_luxel = Some(asked);
        }
    }

    /// What to do about a bake whose file came to `bytes` of lighting, as a warning; nothing when it fits [`Settings::budget`].
    pub fn over_budget(&self, bytes: u64) -> Option<String> {
        (self.budget > 0 && bytes > self.budget).then(|| {
            format!(
                "the baked lighting came to {} MB, past the {} MB one entry of a map's package carries: raise the lightmap \
                 scale above {}, or keep fewer than {} and {} mantissa bits, and bake again",
                bytes >> 20,
                self.budget >> 20,
                self.luxel,
                self.l0_bits,
                self.band_bits
            )
        })
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
    let phong = mapcraft_map::Phong::of(map);
    for f in &lm.faces {
        let Some(brush) = map.get(f.node).and_then(|n| n.kind.brush()) else { continue };
        if solid.as_ref().is_none_or(|(id, _)| *id != f.node) {
            solid = brush.solid().ok().map(|s| (f.node, s));
        }
        let Some((_, s)) = &solid else { continue };
        let (Some(face), Some(Some(w))) = (brush.faces.get(f.face as usize), s.faces.get(f.face as usize)) else { continue };
        let Some(plane) = face.plane() else { continue };
        let normal = plane.normal;
        let corners: Vec<_> = if phong.covers(f.node) { w.0.iter().map(|p| (*p, phong.normal(f.node, *p, normal))).collect() } else { Vec::new() };
        // The point of the face under a texel, from the face's mapping turned around.
        let (bu, bv) = plane.basis();
        let row = |k: usize, v: mapcraft_geom::DVec3| f.uv[k][..3].iter().zip(v.to_array()).map(|(m, c)| f64::from(*m) * c).sum::<f64>();
        let (m, o) = ([[row(0, bu), row(0, bv)], [row(1, bu), row(1, bv)]], normal * plane.dist);
        let det = m[0][0] * m[1][1] - m[0][1] * m[1][0];
        let facing = |x: usize, y: usize| {
            if corners.is_empty() || det.abs() < 1e-12 {
                return normal;
            }
            let [u, v] = [(x as f64 + 0.5) / size as f64, (y as f64 + 0.5) / size as f64];
            let (du, dv) = (u - row(0, o) - f64::from(f.uv[0][3]), v - row(1, o) - f64::from(f.uv[1][3]));
            let p = o + bu * ((du * m[1][1] - dv * m[0][1]) / det) + bv * ((dv * m[0][0] - du * m[1][0]) / det);
            mapcraft_map::phong::fan_normal(&corners, p, normal)
        };
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
                    *d = rgbe::encode(shade(&texels, facing(x, y)));
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
pub fn bake_fingerprint(map: &Map, settings: &Settings, surface: &dyn Fn(&str) -> Surface) -> u64 {
    let mut h = Fnv(fingerprint(map, surface));
    h.bytes(b"bands on Godot's axes");
    h.bytes(b"displacements lightmapped");
    h.bytes(b"buried luxels mended");
    h.f64(settings.bias);
    h.f64(settings.luxel);
    h.bytes(&settings.slice.to_le_bytes());
    h.bytes(&settings.bounces.to_le_bytes());
    h.bytes(&settings.rays.to_le_bytes());
    h.bytes(&settings.sun_samples.to_le_bytes());
    h.bytes(&settings.extra.to_le_bytes());
    h.bytes(&settings.soft.to_le_bytes());
    h.bytes(&settings.l0_bits.to_le_bytes());
    h.bytes(&settings.band_bits.to_le_bytes());
    h.bytes(&settings.budget.to_le_bytes());
    h.f64(settings.bounce_color);
    if let Some(d) = settings.dirt {
        h.bytes(b"dirt");
        [d.depth, d.scale, d.gain, d.angle].into_iter().for_each(|v| h.f64(v));
    }
    h.0
}

/// A fingerprint of everything a bake reads: brush faces, their materials and the light `surface` says those give off, light entities and worldspawn.
pub fn fingerprint(map: &Map, surface: &dyn Fn(&str) -> Surface) -> u64 {
    let mut h = Fnv(0xCBF2_9CE4_8422_2325);
    for (id, node) in map.nodes() {
        match &node.kind {
            NodeKind::Brush(b) => {
                h.bytes(&id.0.to_le_bytes());
                for f in &b.faces {
                    f.points.iter().flat_map(|p| p.to_array()).for_each(|v| h.f64(v));
                    h.str(&f.material);
                    // Albedo is left out: it follows pictures that can load late, so it differs between sessions.
                    surface(&f.material).emission.iter().for_each(|v| h.bytes(&v.to_bits().to_le_bytes()));
                    h.str(f.attrs.get("lightmapscale").map_or("", String::as_str));
                }
            }
            NodeKind::Entity(e) | NodeKind::World(e)
                if e.classname.starts_with("light") || e.get("_phong").is_some() || matches!(node.kind, NodeKind::World(_)) =>
            {
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
