//! Lightmap charts: one rectangle of luxels per drawn brush face, packed into atlas slices.

use std::collections::BTreeSet;

use mapcraft_geom::{DVec2, DVec3};
use mapcraft_map::{FaceId, Map, NodeId, NodeKind};

/// A chart side never exceeds this many luxels; larger faces get coarser luxels.
pub const MAX_CHART: u32 = 512;

/// The luxels of one face, in the face's plane.
#[derive(Clone, Debug, PartialEq)]
pub struct Chart {
    pub node: NodeId,
    /// The face's index in its brush.
    pub face: usize,
    pub material: String,
    pub normal: DVec3,
    /// The world point at chart coordinate (0, 0); chart coordinates count luxels.
    pub origin: DVec3,
    /// Unit tangents along the chart's x and y.
    pub bu: DVec3,
    pub bv: DVec3,
    /// World units per luxel.
    pub scale: f64,
    pub w: u32,
    pub h: u32,
    /// The face polygon in chart coordinates.
    pub polygon: Vec<DVec2>,
    /// Where the chart sits: atlas slice and corner.
    pub slice: u32,
    pub x: u32,
    pub y: u32,
}

impl Chart {
    pub fn to_chart(&self, p: DVec3) -> DVec2 {
        let d = p - self.origin;
        DVec2::new(d.dot(self.bu), d.dot(self.bv)) / self.scale
    }

    pub fn to_world(&self, c: DVec2) -> DVec3 {
        self.origin + (self.bu * c.x + self.bv * c.y) * self.scale
    }

    /// The luxel holding chart point `c`, clamped into the chart.
    pub fn luxel(&self, c: DVec2) -> (u32, u32) {
        let clamp = |v: f64, n: u32| (v.floor().max(0.0) as u32).min(n.saturating_sub(1));
        (clamp(c.x, self.w), clamp(c.y, self.h))
    }

    /// Where luxel (i, j) samples the light: its center, pulled onto the face when the center lies off it.
    pub fn sample_point(&self, i: u32, j: u32) -> DVec3 {
        let c = DVec2::new(f64::from(i) + 0.5, f64::from(j) + 0.5);
        self.to_world(inside(&self.polygon, c))
    }

    /// The 2 x 4 matrix taking a world point to its atlas UV in a slice of `size` luxels.
    pub fn uv_matrix(&self, size: u32) -> [[f32; 4]; 2] {
        let s = f64::from(size.max(1));
        let row = |axis: DVec3, at: u32| {
            let a = axis / (self.scale * s);
            let offset = (f64::from(at) - self.origin.dot(axis) / self.scale) / s;
            [a.x as f32, a.y as f32, a.z as f32, offset as f32]
        };
        [row(self.bu, self.x), row(self.bv, self.y)]
    }
}

/// `c` when it lies inside `polygon`, else the nearest point a little way inside it.
fn inside(polygon: &[DVec2], c: DVec2) -> DVec2 {
    let n = polygon.len();
    if n < 3 {
        return c;
    }
    let area: f64 = (0..n).map(|i| polygon[i].perp_dot(polygon[(i + 1) % n])).sum();
    let sign = area.signum();
    if (0..n).all(|i| (polygon[(i + 1) % n] - polygon[i]).perp_dot(c - polygon[i]) * sign >= 0.0) {
        return c;
    }
    let center = polygon.iter().copied().sum::<DVec2>() / n as f64;
    let nearest = (0..n)
        .map(|i| {
            let (a, b) = (polygon[i], polygon[(i + 1) % n]);
            let t = ((c - a).dot(b - a) / (b - a).length_squared().max(1e-12)).clamp(0.0, 1.0);
            a + (b - a) * t
        })
        .min_by(|p, q| p.distance_squared(c).total_cmp(&q.distance_squared(c)))
        .unwrap_or(center);
    let toward = center - nearest;
    nearest + toward.normalize_or_zero() * toward.length().min(0.05)
}

/// Charts for every drawn face of every brush, skipping faces in `skip` and the brushes of boolean groups.
/// Luxel size comes from a face's `lightmapscale`, else `luxel`.
pub fn build(map: &Map, skip: &BTreeSet<(NodeId, FaceId)>, luxel: f64) -> Vec<Chart> {
    let mut charts = Vec::new();
    for (id, node) in map.nodes() {
        let NodeKind::Brush(b) = &node.kind else { continue };
        if mapcraft_map::boolean::source_op(map, id).is_some() || b.has_displacement() {
            continue;
        }
        let Ok(solid) = b.solid() else { continue };
        for (index, (face, winding)) in b.faces.iter().zip(&solid.faces).enumerate() {
            let (Some(w), Some(plane)) = (winding, face.plane()) else { continue };
            if !b.draws(face) || face.material.is_empty() || skip.contains(&(id, face.id)) || w.0.len() < 3 {
                continue;
            }
            let wanted =
                face.attrs.get("lightmapscale").and_then(|s| s.trim().parse::<f64>().ok()).filter(|v| v.is_finite() && *v >= 1.0).unwrap_or(luxel).max(1.0);
            let (bu, bv) = plane.basis();
            let flat: Vec<DVec2> = w.0.iter().map(|p| DVec2::new(p.dot(bu), p.dot(bv))).collect();
            let (min, max) = flat.iter().fold((DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY)), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
            let extent = max - min;
            if !extent.is_finite() {
                continue;
            }
            // One luxel of border on every side, so filtering at the edge reads lit luxels.
            let longest = extent.max_element();
            let scale = wanted.max(longest / f64::from(MAX_CHART - 2));
            let size = |e: f64| ((e / scale).ceil() as u32).saturating_add(2).clamp(3, MAX_CHART);
            let corner = min - DVec2::splat(scale);
            let origin = bu * corner.x + bv * corner.y + plane.normal * plane.dist;
            let polygon = flat.iter().map(|p| (*p - corner) / scale).collect();
            charts.push(Chart {
                node: id,
                face: index,
                material: face.material.clone(),
                normal: plane.normal,
                origin,
                bu,
                bv,
                scale,
                w: size(extent.x),
                h: size(extent.y),
                polygon,
                slice: 0,
                x: 0,
                y: 0,
            });
        }
    }
    charts
}

/// Places every chart in square slices of `size` luxels with shelf packing; returns the slice count.
pub fn pack(charts: &mut [Chart], size: u32) -> u32 {
    let mut order: Vec<usize> = (0..charts.len()).collect();
    order.sort_by_key(|&i| (std::cmp::Reverse(charts[i].h), std::cmp::Reverse(charts[i].w)));
    let (mut slice, mut x, mut y, mut shelf) = (0u32, 0u32, 0u32, 0u32);
    let mut used = false;
    for i in order {
        let c = &mut charts[i];
        c.w = c.w.min(size);
        c.h = c.h.min(size);
        if x + c.w > size {
            x = 0;
            y += shelf;
            shelf = 0;
        }
        if y + c.h > size {
            slice += 1;
            (x, y, shelf) = (0, 0, 0);
        }
        (c.slice, c.x, c.y) = (slice, x, y);
        x += c.w;
        shelf = shelf.max(c.h);
        used = true;
    }
    if used { slice + 1 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chart(w: u32, h: u32) -> Chart {
        Chart {
            node: NodeId(1),
            face: 0,
            material: String::new(),
            normal: DVec3::Z,
            origin: DVec3::ZERO,
            bu: DVec3::X,
            bv: DVec3::Y,
            scale: 1.0,
            w,
            h,
            polygon: Vec::new(),
            slice: 0,
            x: 0,
            y: 0,
        }
    }

    #[test]
    fn packed_charts_never_overlap() {
        let mut charts: Vec<Chart> = (0..300u32).map(|i| chart(3 + (i * 37) % 120, 3 + (i * 53) % 90)).collect();
        let slices = pack(&mut charts, 512);
        assert!(slices >= 1);
        for (i, a) in charts.iter().enumerate() {
            assert!(a.x + a.w <= 512 && a.y + a.h <= 512);
            for b in &charts[i + 1..] {
                let apart = a.slice != b.slice || a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y;
                assert!(apart, "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn uv_matrix_lands_inside_the_chart() {
        let mut c = chart(10, 10);
        (c.scale, c.x, c.y, c.origin) = (16.0, 100, 200, DVec3::new(-32.0, 64.0, 0.0));
        let m = c.uv_matrix(1024);
        let p = c.to_world(DVec2::new(2.5, 7.5));
        let uv = |r: [f32; 4]| f64::from(r[0]) * p.x + f64::from(r[1]) * p.y + f64::from(r[2]) * p.z + f64::from(r[3]);
        assert!((uv(m[0]) * 1024.0 - 102.5).abs() < 1e-3);
        assert!((uv(m[1]) * 1024.0 - 207.5).abs() < 1e-3);
    }

    #[test]
    fn samples_off_the_face_move_onto_it() {
        let square = vec![DVec2::new(1.0, 1.0), DVec2::new(4.0, 1.0), DVec2::new(4.0, 4.0), DVec2::new(1.0, 4.0)];
        assert_eq!(inside(&square, DVec2::new(2.0, 2.0)), DVec2::new(2.0, 2.0));
        let p = inside(&square, DVec2::new(0.5, 2.0));
        assert!(p.x > 1.0 && p.x < 1.1 && (p.y - 2.0).abs() < 0.05, "{p}");
    }
}
