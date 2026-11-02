//! The baker: direct light with shadow rays, then gather passes for sky, emissive surfaces and bounce,
//! accumulated per luxel as the L1 spherical harmonics Godot's directional lightmaps hold.

use std::collections::BTreeSet;
use std::sync::atomic::Ordering;

use mapcraft_geom::{DVec2, DVec3};
use mapcraft_map::{FaceId, Map, NodeId, NodeKind};

use crate::bvh::{Bvh, Triangle};
use crate::charts::{self, Chart};
use crate::lights::{self, Kind, Light, Lights};
use crate::{BakeError, FaceMap, Lightmap, Progress, Settings, Slice, Surface, rgbe};

/// Rays start this far off the face, in map units, so they never hit the face they leave.
const BIAS: f64 = 0.5;

/// Light dimmer than this adds nothing worth a shadow ray.
const DIM: f64 = 1.0 / 1024.0;

/// Triangles of faces that cast shadows but hold no chart.
const NO_CHART: u32 = u32::MAX;

type Sh = [DVec3; 4];

/// A chart's luxels during the bake.
#[derive(Clone)]
struct Luxels {
    sh: Vec<Sh>,
    /// Light leaving each luxel toward others, for the next gather.
    exitant: Vec<DVec3>,
}

/// What the workers share.
struct Scene<'a> {
    charts: &'a [Chart],
    surfaces: Vec<Surface>,
    bvh: Bvh,
    lights: Lights,
    settings: &'a Settings,
    progress: &'a Progress,
}

/// Whether light from `material` faces is blocked: triggers, clips and other volumes let it pass.
fn casts_shadow(material: &str) -> bool {
    let m = material.to_ascii_lowercase();
    // An untextured face belongs to a zone or trigger drawn in its entity's color.
    if m.is_empty() {
        return false;
    }
    !["tools/trigger", "tools/clip", "tools/skip", "tools/hint", "tools/origin", "tools/areaportal", "tools/invisible", "tools/player"]
        .iter()
        .any(|t| m.starts_with(t))
}

/// Triangles of every face that blocks light, each owned by its chart when it has one.
fn triangles(map: &Map, charts: &[Chart]) -> Vec<Triangle> {
    let owner: std::collections::HashMap<(NodeId, usize), u32> = charts.iter().enumerate().map(|(i, c)| ((c.node, c.face), i as u32)).collect();
    let mut tris = Vec::new();
    for (id, node) in map.nodes() {
        let NodeKind::Brush(b) = &node.kind else { continue };
        if mapcraft_map::boolean::source_op(map, id).is_some() || b.is_clip() {
            continue;
        }
        let Ok(solid) = b.solid() else { continue };
        for (index, (face, winding)) in b.faces.iter().zip(&solid.faces).enumerate() {
            let Some(w) = winding else { continue };
            if !casts_shadow(&face.material) {
                continue;
            }
            let owner = owner.get(&(id, index)).copied().unwrap_or(NO_CHART);
            for k in 1..w.0.len().saturating_sub(1) {
                tris.push(Triangle { a: w.0[0], b: w.0[k], c: w.0[k + 1], owner });
            }
        }
    }
    tris
}

fn radical_inverse(mut i: u32) -> f64 {
    i = i.rotate_right(16);
    i = ((i & 0x5555_5555) << 1) | ((i & 0xAAAA_AAAA) >> 1);
    i = ((i & 0x3333_3333) << 2) | ((i & 0xCCCC_CCCC) >> 2);
    i = ((i & 0x0F0F_0F0F) << 4) | ((i & 0xF0F0_F0F0) >> 4);
    i = ((i & 0x00FF_00FF) << 8) | ((i & 0xFF00_FF00) >> 8);
    f64::from(i) / 4_294_967_296.0
}

/// Point `i` of `n` on the unit square, shifted by `seed` so neighboring luxels sample differently.
fn hammersley(i: u32, n: u32, seed: DVec2) -> DVec2 {
    let p = DVec2::new((f64::from(i) + 0.5) / f64::from(n.max(1)), radical_inverse(i)) + seed;
    p - p.floor()
}

/// A hash of a luxel's place, as an offset on the unit square.
fn seed(chart: usize, index: usize) -> DVec2 {
    let mut h = (chart as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (index as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 31;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 29;
    DVec2::new((h & 0xFFFF_FFFF) as f64 / 4_294_967_296.0, (h >> 32) as f64 / 4_294_967_296.0)
}

fn frame(n: DVec3) -> (DVec3, DVec3) {
    let t = n.any_orthonormal_vector();
    (t, n.cross(t))
}

/// A cosine-weighted direction around `n` for unit-square point `u`.
fn cosine(n: DVec3, u: DVec2) -> DVec3 {
    let (t, b) = frame(n);
    let r = u.x.sqrt();
    let phi = std::f64::consts::TAU * u.y;
    (t * (r * phi.cos()) + b * (r * phi.sin()) + n * (1.0 - u.x).max(0.0).sqrt()).normalize_or(n)
}

/// A direction within `spread` radians of `axis` for unit-square point `u`.
fn cone(axis: DVec3, spread: f64, u: DVec2) -> DVec3 {
    if spread <= 0.0 {
        return axis;
    }
    let (t, b) = frame(axis);
    let cos_max = spread.cos();
    let z = 1.0 - u.x * (1.0 - cos_max);
    let r = (1.0 - z * z).max(0.0).sqrt();
    let phi = std::f64::consts::TAU * u.y;
    (t * (r * phi.cos()) + b * (r * phi.sin()) + axis * z).normalize_or(axis)
}

/// A map direction (Z up) in Godot's axes (Y up), the ones its lightmap shader reads the L1 bands along.
pub fn godot_axes(v: DVec3) -> DVec3 {
    DVec3::new(v.x, v.z, -v.y)
}

/// Adds light arriving from `dir` (toward the light) as Godot bakes a direct light; `light` already carries N.L.
fn add_direct(sh: &mut Sh, light: DVec3, dir: DVec3, ndl: f64) {
    let c0 = 1.0 / (1.0 + 4.0 * ndl);
    let c1 = (1.0 - c0) / ndl;
    sh[0] += light * c0;
    let g = godot_axes(dir);
    sh[1] += light * (c1 * g.y);
    sh[2] += light * (c1 * g.z);
    sh[3] += light * (c1 * g.x);
}

impl Scene<'_> {
    fn surface(&self, chart: usize) -> Surface {
        self.surfaces.get(chart).copied().unwrap_or_default()
    }

    /// Light from point, spot and sun lights reaching `p` on a face facing `n`.
    fn direct(&self, p: DVec3, n: DVec3, near: &[&Light], seed: DVec2) -> (Sh, DVec3) {
        let mut sh = [DVec3::ZERO; 4];
        let mut total = DVec3::ZERO;
        let origin = p + n * BIAS;
        for l in near {
            match &l.kind {
                Kind::Sun { toward, spread } => {
                    let samples = self.settings.sun_samples.max(1);
                    for s in 0..samples {
                        let dir = cone(*toward, *spread, hammersley(s, samples, seed));
                        let ndl = n.dot(dir);
                        if ndl <= 1e-4 || self.bvh.occluded(origin, dir, 1e9) {
                            continue;
                        }
                        let light = l.color * (ndl / f64::from(samples));
                        add_direct(&mut sh, light, dir, ndl);
                        total += light;
                    }
                }
                Kind::Point | Kind::Spot { .. } if l.size > 0.0 => {
                    // A light with a size casts soft shadows: shadow rays spread over its sphere.
                    let to = l.pos - origin;
                    let d = to.length();
                    if d < 1e-6 || d > l.range {
                        continue;
                    }
                    let Some((light, dir, ndl)) = self.arrival(l, n, to, d) else { continue };
                    let samples = self.settings.sun_samples.max(1);
                    let spread = (l.size / 2.0 / d).min(1.0).asin();
                    let lit =
                        (0..samples).filter(|s| !self.bvh.occluded(origin, cone(dir, spread, hammersley(*s, samples, seed)), d - l.size / 2.0 - BIAS)).count();
                    if lit == 0 {
                        continue;
                    }
                    let light = light * (lit as f64 / f64::from(samples));
                    add_direct(&mut sh, light, dir, ndl);
                    total += light;
                }
                Kind::Point | Kind::Spot { .. } => {
                    let to = l.pos - origin;
                    let d = to.length();
                    if d < 1e-6 || d > l.range {
                        continue;
                    }
                    let Some((light, dir, ndl)) = self.arrival(l, n, to, d) else { continue };
                    if self.bvh.occluded(origin, dir, d - BIAS) {
                        continue;
                    }
                    add_direct(&mut sh, light, dir, ndl);
                    total += light;
                }
            }
        }
        (sh, total)
    }

    /// Light from point or spot `l` arriving along `to`, `d` long, at a face facing `n`, before shadows:
    /// the light times N.L, its direction and N.L; none when it is too dim or faces away.
    fn arrival(&self, l: &Light, n: DVec3, to: DVec3, d: f64) -> Option<(DVec3, DVec3, f64)> {
        let dir = to / d;
        let ndl = n.dot(dir);
        if ndl <= 1e-4 {
            return None;
        }
        let spot = match &l.kind {
            Kind::Spot { dir: aim, cos_outer, cos_inner, exponent } => {
                let c = aim.dot(-dir);
                if c <= *cos_outer {
                    return None;
                }
                if c >= *cos_inner { 1.0 } else { ((c - cos_outer) / (cos_inner - cos_outer).max(1e-6)).powf(*exponent) }
            }
            _ => 1.0,
        };
        let light = l.color * (l.attenuation(d) * spot * ndl);
        (light.max_element() >= DIM).then_some((light, dir, ndl))
    }

    /// Light gathered over the hemisphere at `p`: what the previous pass left on faces the rays hit, plus the sky on the first pass.
    fn gather(&self, p: DVec3, n: DVec3, prev: &[Luxels], sky: bool, seed: DVec2) -> (Sh, DVec3) {
        let rays = self.settings.rays.max(1);
        let mut sh = [DVec3::ZERO; 4];
        let mut total = DVec3::ZERO;
        let origin = p + n * BIAS;
        for r in 0..rays {
            let dir = cosine(n, hammersley(r, rays, seed));
            let light = match self.bvh.nearest(origin, dir, 1e9) {
                Some((t, owner)) if owner != NO_CHART => {
                    let (Some(c), Some(lux)) = (self.charts.get(owner as usize), prev.get(owner as usize)) else { continue };
                    if c.normal.dot(dir) >= 0.0 {
                        continue;
                    }
                    let (i, j) = c.luxel(c.to_chart(origin + dir * t));
                    lux.exitant.get((j * c.w + i) as usize).copied().unwrap_or_default()
                }
                Some(_) => continue,
                None if sky => self.lights.sky.as_ref().map_or(DVec3::ZERO, |s| s.radiance(dir)),
                None => continue,
            };
            sh[0] += light * 0.25;
            let g = godot_axes(dir);
            sh[1] += light * g.y;
            sh[2] += light * g.z;
            sh[3] += light * g.x;
            total += light;
        }
        let k = 1.0 / f64::from(rays);
        (sh.map(|c| c * k), total * k)
    }

    /// Runs `work` over every chart on all threads, in chart order.
    fn each_chart<F>(&self, work: F) -> Result<Vec<Luxels>, BakeError>
    where
        F: Fn(usize, &Chart) -> Luxels + Sync,
    {
        let next = std::sync::atomic::AtomicUsize::new(0);
        let threads = if self.settings.threads > 0 { self.settings.threads } else { std::thread::available_parallelism().map_or(4, |n| n.get()) };
        let mut parts: Vec<(usize, Luxels)> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..threads)
                .map(|_| {
                    s.spawn(|| {
                        let mut out = Vec::new();
                        loop {
                            let i = next.fetch_add(1, Ordering::Relaxed);
                            let Some(c) = self.charts.get(i) else { break };
                            if self.progress.cancel.load(Ordering::Relaxed) {
                                break;
                            }
                            out.push((i, work(i, c)));
                            self.progress.done.fetch_add(u64::from(c.w) * u64::from(c.h), Ordering::Relaxed);
                        }
                        out
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().map_err(|_| BakeError::Worker)).collect::<Result<Vec<_>, _>>().map(|v| v.into_iter().flatten().collect())
        })?;
        if self.progress.cancel.load(Ordering::Relaxed) {
            return Err(BakeError::Cancelled);
        }
        parts.sort_by_key(|p| p.0);
        Ok(parts.into_iter().map(|p| p.1).collect())
    }
}

/// Bakes `map`. `surface` gives each material's albedo and emission; faces in `skip` get no lightmap.
/// Returns the lightmap and warnings about lights that were skipped.
pub fn bake(
    map: &Map,
    surface: &(dyn Fn(&str) -> Surface + Sync),
    skip: &BTreeSet<(NodeId, FaceId)>,
    settings: &Settings,
    progress: &Progress,
) -> Result<(Lightmap, Vec<String>), BakeError> {
    let mut charts = charts::build(map, skip, settings.luxel);
    let (lights, warnings) = lights::read(map);
    let surfaces = charts.iter().map(|c| surface(&c.material)).collect::<Vec<_>>();
    let emissive = surfaces.iter().any(|s| s.emission.iter().any(|v| *v > 0.0));
    let gathers = if settings.bounces > 0 { settings.bounces } else { u32::from(lights.sky.is_some() || emissive) };
    let luxels: u64 = charts.iter().map(|c| u64::from(c.w) * u64::from(c.h)).sum();
    progress.total.store(luxels * (1 + u64::from(gathers)), Ordering::Relaxed);
    progress.done.store(0, Ordering::Relaxed);
    let scene = Scene { charts: &charts, surfaces, bvh: Bvh::new(triangles(map, &charts)), lights, settings, progress };
    let fingerprint = crate::bake_fingerprint(map, settings);

    let mut totals = scene.each_chart(|ci, c| {
        let near: Vec<&Light> = scene.lights.lights.iter().filter(|l| l.range.is_infinite() || chart_distance(c, l.pos) <= l.range).collect();
        let s = scene.surface(ci);
        let (albedo, emission) = (DVec3::from_array(s.albedo.map(f64::from)), DVec3::from_array(s.emission.map(f64::from)));
        let reflect = if settings.bounces > 0 { albedo } else { DVec3::ZERO };
        let n = (c.w * c.h) as usize;
        let mut out = Luxels { sh: Vec::with_capacity(n), exitant: Vec::with_capacity(n) };
        for j in 0..c.h {
            for i in 0..c.w {
                let (sh, total) = scene.direct(c.sample_point(i, j), c.normal, &near, seed(ci, (j * c.w + i) as usize));
                out.sh.push(sh);
                out.exitant.push(total * reflect + emission);
            }
        }
        out
    })?;

    let mut prev = totals.clone();
    for pass in 0..gathers {
        let next = scene.each_chart(|ci, c| {
            let s = scene.surface(ci);
            let albedo = DVec3::from_array(s.albedo.map(f64::from));
            let n = (c.w * c.h) as usize;
            let mut out = Luxels { sh: Vec::with_capacity(n), exitant: Vec::with_capacity(n) };
            for j in 0..c.h {
                for i in 0..c.w {
                    let index = (j * c.w + i) as usize;
                    let (sh, total) = scene.gather(c.sample_point(i, j), c.normal, &prev, pass == 0, seed(ci, index + 7919 * (pass as usize + 1)));
                    out.sh.push(sh);
                    out.exitant.push(total * albedo);
                }
            }
            out
        })?;
        for (t, g) in totals.iter_mut().zip(&next) {
            for (a, b) in t.sh.iter_mut().zip(&g.sh) {
                for k in 0..4 {
                    a[k] += b[k];
                }
            }
        }
        prev = next;
    }

    let min = scene.lights.minlight;
    for t in &mut totals {
        for sh in &mut t.sh {
            sh[0] = sh[0].max(min);
        }
    }
    let collapsed: Vec<Option<Sh>> = totals.iter().map(|t| uniform(&t.sh)).collect();
    for (c, flat) in charts.iter_mut().zip(&collapsed) {
        if flat.is_some() {
            (c.w, c.h) = (1, 1);
        }
    }
    let size = settings.slice.max(64);
    let count = charts::pack(&mut charts, size);
    let texels = (size as usize) * (size as usize);
    let mut slices: Vec<Slice> = (0..count).map(|_| Slice { layers: std::array::from_fn(|_| vec![0u32; texels]) }).collect();
    let mut faces = Vec::with_capacity(charts.len());
    for ((c, t), flat) in charts.iter().zip(&totals).zip(&collapsed) {
        let Some(slice) = slices.get_mut(c.slice as usize) else { continue };
        let mut put = |x: u32, y: u32, sh: &Sh| {
            let at = (y as usize) * (size as usize) + x as usize;
            for (k, v) in pack_sh(sh).iter().enumerate() {
                if let Some(texel) = slice.layers[k].get_mut(at) {
                    *texel = rgbe::encode(*v);
                }
            }
        };
        let uv = match flat {
            Some(sh) => {
                put(c.x, c.y, sh);
                let s = size as f32;
                [[0.0, 0.0, 0.0, (c.x as f32 + 0.5) / s], [0.0, 0.0, 0.0, (c.y as f32 + 0.5) / s]]
            }
            None => {
                for j in 0..c.h {
                    for i in 0..c.w {
                        if let Some(sh) = t.sh.get((j * c.w + i) as usize) {
                            put(c.x + i, c.y + j, sh);
                        }
                    }
                }
                c.uv_matrix(size)
            }
        };
        faces.push(FaceMap { node: c.node, face: c.face as u32, slice: c.slice, uv });
    }
    Ok((Lightmap { fingerprint, size, slices, faces }, warnings))
}

/// The closest distance from `p` to any point of the chart's rectangle.
fn chart_distance(c: &Chart, p: DVec3) -> f64 {
    let local = c.to_chart(p);
    let clamped = local.clamp(DVec2::ZERO, DVec2::new(f64::from(c.w), f64::from(c.h)));
    let plane = (p - c.origin).dot(c.normal);
    (((local - clamped) * c.scale).length_squared() + plane * plane).sqrt()
}

/// The one value every luxel shares, when they all match closely enough to need a single luxel.
fn uniform(sh: &[Sh]) -> Option<Sh> {
    let first = *sh.first()?;
    let close = |a: DVec3, b: DVec3| (a - b).abs().max_element() <= 1e-3 + 0.01 * a.abs().max(b.abs()).max_element();
    sh.iter().all(|s| (0..4).all(|k| close(s[k], first[k]))).then_some(first)
}

/// The four texels Godot stores: L0 as is, and each L1 band divided by 8 L0 and shifted into 0 to 1.
fn pack_sh(sh: &Sh) -> [DVec3; 4] {
    let l0 = sh[0].max(DVec3::ZERO);
    let band = |l1: DVec3| (l1 / (l0 * 8.0 + DVec3::splat(1e-6)) + DVec3::splat(0.5)).clamp(DVec3::ZERO, DVec3::ONE);
    [l0, band(sh[1]), band(sh[2]), band(sh[3])]
}

/// What Godot's shader shows for packed texels on a face with normal `n`.
pub fn shade(texels: &[DVec3; 4], n: DVec3) -> DVec3 {
    let n = godot_axes(n);
    let l0 = texels[0];
    let l1 = |t: DVec3| (t - DVec3::splat(0.5)) * 2.0;
    l0 + l1(texels[1]) * n.y * (l0 * 4.0) + l1(texels[2]) * n.z * (l0 * 4.0) + l1(texels[3]) * n.x * (l0 * 4.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_direct_light_shades_back_to_its_brightness() {
        for (dir, n) in [(DVec3::Z, DVec3::Z), (DVec3::new(0.6, 0.0, 0.8), DVec3::Z), (DVec3::new(0.0, 0.8, 0.6), DVec3::new(0.0, 0.6, 0.8))] {
            let ndl = n.dot(dir);
            let mut sh = [DVec3::ZERO; 4];
            add_direct(&mut sh, DVec3::ONE * ndl, dir, ndl);
            let seen = shade(&pack_sh(&sh), n);
            assert!((seen.x - ndl).abs() < 0.01, "{dir} on {n}: {seen} vs {ndl}");
        }
    }

    #[test]
    fn light_from_one_side_points_the_l1_band_that_way() {
        let mut sh = [DVec3::ZERO; 4];
        let dir = DVec3::new(0.7, 0.0, 0.714).normalize();
        add_direct(&mut sh, DVec3::ONE * dir.z, dir, dir.z);
        let packed = pack_sh(&sh);
        assert!(packed[3].x > 0.5, "Godot's x band, the map's x: {}", packed[3]);
        assert!(packed[1].x > 0.5, "Godot's y band is the map's up: {}", packed[1]);
        assert!((packed[2].x - 0.5).abs() < 1e-9, "Godot's z band, the map's -y: {}", packed[2]);
    }

    #[test]
    fn uniform_charts_collapse_and_varied_ones_do_not() {
        let a = [DVec3::splat(0.2), DVec3::ZERO, DVec3::ZERO, DVec3::ZERO];
        let mut b = a;
        b[0] = DVec3::splat(0.4);
        assert!(uniform(&[a, a, a]).is_some());
        assert!(uniform(&[a, b]).is_none());
    }
}
