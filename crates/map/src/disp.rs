use mapcraft_geom::DVec3;
use serde::{Deserialize, Serialize};

/// The largest power: 17 by 17 vertices, as in Source.
pub const MAX_POWER: u32 = 4;

/// A Hammer-style displacement: a four-cornered face becomes a grid of `(2^power + 1)^2`
/// vertices, each raised along the face normal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Displacement {
    pub power: u32,
    /// The face corner nearest this point is where the grid starts; it moves with the brush.
    pub start: DVec3,
    /// Height of each vertex along the face normal, row by row from the start corner.
    pub heights: Vec<f64>,
    /// Share of the second layer at each vertex from 0 to 1, in the order of `heights`; empty is 0 everywhere.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blend: Vec<f64>,
    /// The second layer; `Face::blend_layer` decides whether it counts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blend_material: Option<String>,
}

/// How a material takes part in displacement blending.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BlendRole {
    #[default]
    Layer,
    /// Holds two textures and mixes them by vertex weight itself.
    Own,
    Unblendable,
}

impl Displacement {
    pub fn new(power: u32, start: DVec3) -> Self {
        let n = side(power);
        Self { power: power.clamp(1, MAX_POWER), start, heights: vec![0.0; n * n], blend: Vec::new(), blend_material: None }
    }

    /// Vertices along one side of the grid.
    pub fn size(&self) -> usize {
        side(self.power)
    }

    /// The second layer's share at vertex `k`, 0 where the file has no usable value.
    pub fn blend_at(&self, k: usize) -> f64 {
        self.blend.get(k).copied().filter(|w| w.is_finite()).unwrap_or(0.0).clamp(0.0, 1.0)
    }

    /// Every vertex's share of the second layer, in grid order.
    pub fn blend_values(&self) -> Vec<f64> {
        (0..self.size() * self.size()).map(|k| self.blend_at(k)).collect()
    }

    /// Stores weights clamped to 0..=1, or nothing when they are all 0, so cleared faces equal never-painted ones.
    pub fn set_blend(&mut self, values: Vec<f64>) {
        let values: Vec<f64> = values.into_iter().map(|w| if w.is_finite() { w.clamp(0.0, 1.0) } else { 0.0 }).collect();
        self.blend = if values.iter().all(|w| *w < 1e-6) { Vec::new() } else { values };
    }

    pub fn blend_fits(&self) -> bool {
        self.blend.is_empty() || self.blend.len() == self.size() * self.size()
    }

    /// Index of the point of `vertices` (from [`Displacement::vertices`]) closest to `p`.
    pub fn nearest(vertices: &[DVec3], p: DVec3) -> Option<usize> {
        (0..vertices.len()).min_by(|a, b| vertices[*a].distance_squared(p).total_cmp(&vertices[*b].distance_squared(p)))
    }

    /// The same surface and paint at another power, sampled bilinearly.
    pub fn resampled(&self, power: u32) -> Self {
        let (n, m) = (self.size(), side(power));
        let mut out = Self::new(power, self.start);
        out.blend_material.clone_from(&self.blend_material);
        let blend = (!self.blend.is_empty()).then(|| self.blend_values());
        let mut weights = vec![0.0; m * m];
        for j in 0..m {
            for i in 0..m {
                let (x, y) = (i as f64 / (m - 1) as f64 * (n - 1) as f64, j as f64 / (m - 1) as f64 * (n - 1) as f64);
                out.heights[j * m + i] = bilinear(&self.heights, n, x, y);
                if let Some(b) = &blend {
                    weights[j * m + i] = bilinear(b, n, x, y);
                }
            }
        }
        if blend.is_some() {
            out.set_blend(weights);
        }
        out
    }

    /// The grid swapped across its diagonal, which keeps a mirrored brush's surface in place.
    pub fn transposed(&self) -> Self {
        let n = self.size();
        let flip = |at: &dyn Fn(usize) -> f64| (0..n * n).map(|k| at((k % n) * n + k / n)).collect::<Vec<f64>>();
        let heights = flip(&|k| self.heights.get(k).copied().unwrap_or(0.0));
        let blend = if self.blend.is_empty() { Vec::new() } else { flip(&|k| self.blend_at(k)) };
        Self { power: self.power, start: self.start, heights, blend, blend_material: self.blend_material.clone() }
    }

    /// The face's corners in grid order (start, then counter-clockwise from outside), or `None`
    /// unless the face has exactly four corners and the heights fit the power.
    pub fn corners(&self, polygon: &[DVec3]) -> Option<[DVec3; 4]> {
        if polygon.len() != 4 || self.heights.len() != self.size() * self.size() {
            return None;
        }
        let s = (0..4).min_by(|a, b| polygon[*a].distance_squared(self.start).total_cmp(&polygon[*b].distance_squared(self.start)))?;
        Some([polygon[s], polygon[(s + 1) % 4], polygon[(s + 2) % 4], polygon[(s + 3) % 4]])
    }

    /// Grid points on the flat face, row by row: rows run from the start corner toward the
    /// fourth corner, columns toward the second.
    pub fn base(&self, polygon: &[DVec3]) -> Option<Vec<DVec3>> {
        let [c0, c1, c2, c3] = self.corners(polygon)?;
        let n = self.size();
        let step = (n - 1) as f64;
        Some(
            (0..n)
                .flat_map(|j| {
                    (0..n).map(move |i| {
                        let (u, v) = (i as f64 / step, j as f64 / step);
                        c0.lerp(c1, u).lerp(c3.lerp(c2, u), v)
                    })
                })
                .collect(),
        )
    }

    /// The displaced grid, row by row.
    pub fn vertices(&self, polygon: &[DVec3], normal: DVec3) -> Option<Vec<DVec3>> {
        Some(self.base(polygon)?.into_iter().zip(&self.heights).map(|(p, h)| p + normal * *h).collect())
    }

    /// Triangles over the grid as vertex indices, counter-clockwise from outside, with the
    /// diagonals alternating so the surface has no grain.
    pub fn triangles(&self) -> Vec<[usize; 3]> {
        let n = self.size();
        let mut out = Vec::with_capacity((n - 1) * (n - 1) * 2);
        for j in 0..n - 1 {
            for i in 0..n - 1 {
                let (a, b, c, d) = (j * n + i, j * n + i + 1, (j + 1) * n + i + 1, (j + 1) * n + i);
                if (i + j) % 2 == 0 {
                    out.extend([[a, b, c], [a, c, d]]);
                } else {
                    out.extend([[a, b, d], [b, c, d]]);
                }
            }
        }
        out
    }

    /// Per-vertex normals of the displaced grid.
    pub fn normals(&self, vertices: &[DVec3], face_normal: DVec3) -> Vec<DVec3> {
        let n = self.size();
        let at = |i: usize, j: usize| vertices.get(j * n + i).copied().unwrap_or_default();
        (0..n * n)
            .map(|k| {
                let (i, j) = (k % n, k / n);
                let du = at((i + 1).min(n - 1), j) - at(i.saturating_sub(1), j);
                let dv = at(i, (j + 1).min(n - 1)) - at(i, j.saturating_sub(1));
                let nrm = du.cross(dv).normalize_or_zero();
                if nrm.dot(face_normal) < 0.0 { -nrm } else { nrm }
            })
            .map(|v| if v == DVec3::ZERO { face_normal } else { v })
            .collect()
    }
}

fn side(power: u32) -> usize {
    (1usize << power.clamp(1, MAX_POWER)) + 1
}

/// `values` of an `n` by `n` grid at fractional grid coordinates; missing values count as 0.
fn bilinear(values: &[f64], n: usize, x: f64, y: f64) -> f64 {
    let at = |i: usize, j: usize| values.get(j.min(n - 1) * n + i.min(n - 1)).copied().unwrap_or(0.0);
    let (i, j) = (x.floor().max(0.0) as usize, y.floor().max(0.0) as usize);
    let (fx, fy) = (x - i as f64, y - j as f64);
    let top = at(i, j) * (1.0 - fx) + at(i + 1, j) * fx;
    let bottom = at(i, j + 1) * (1.0 - fx) + at(i + 1, j + 1) * fx;
    top * (1.0 - fy) + bottom * fy
}

/// Distance along the ray to the nearest triangle it hits from either side.
pub fn ray_triangle(origin: DVec3, dir: DVec3, [a, b, c]: [DVec3; 3]) -> Option<f64> {
    let (e1, e2) = (b - a, c - a);
    let p = dir.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let s = origin - a;
    let u = s.dot(p) / det;
    let q = s.cross(e1);
    let v = dir.dot(q) / det;
    let t = e2.dot(q) / det;
    (u >= 0.0 && v >= 0.0 && u + v <= 1.0 && t > 0.0).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Vec<DVec3> {
        vec![DVec3::new(0.0, 0.0, 0.0), DVec3::new(64.0, 0.0, 0.0), DVec3::new(64.0, 64.0, 0.0), DVec3::new(0.0, 64.0, 0.0)]
    }

    #[test]
    fn grid_starts_at_the_nearest_corner() {
        let mut d = Displacement::new(2, DVec3::new(60.0, 1.0, 0.0));
        d.heights[0] = 8.0;
        let v = d.vertices(&square(), DVec3::Z).unwrap();
        assert_eq!(v.len(), 25);
        assert_eq!(v[0], DVec3::new(64.0, 0.0, 8.0), "the first vertex sits on the start corner, raised");
        assert_eq!(v[4], DVec3::new(64.0, 64.0, 0.0), "the first row runs to the next corner");
        assert_eq!(d.triangles().len(), 32);
        assert!(d.vertices(&square()[..3], DVec3::Z).is_none(), "three corners cannot hold a grid");
    }

    #[test]
    fn resampling_keeps_the_shape() {
        let mut d = Displacement::new(2, DVec3::ZERO);
        for (k, h) in d.heights.iter_mut().enumerate() {
            *h = (k % 5) as f64 * 4.0;
        }
        let fine = d.resampled(3);
        assert_eq!(fine.size(), 9);
        assert_eq!(fine.heights[8], 16.0);
        assert_eq!(fine.heights[1], 2.0, "new vertices interpolate their neighbors");
        assert_eq!(fine.resampled(2).heights, d.heights);
    }

    #[test]
    fn paint_resamples_and_transposes_with_the_heights() {
        let mut d = Displacement::new(2, DVec3::ZERO);
        d.set_blend((0..25).map(|k| ((k % 5 + k / 5) % 2) as f64).collect());
        d.blend_material = Some("grass".into());
        let fine = d.resampled(3);
        assert_eq!(fine.blend.len(), 81);
        assert_eq!(fine.blend_at(1), 0.5, "new midpoints mix their neighbors");
        assert_eq!(fine.blend_at(2), 1.0);
        assert_eq!(fine.blend_material.as_deref(), Some("grass"));
        assert!(Displacement::new(2, DVec3::ZERO).resampled(3).blend.is_empty(), "unpainted stays unpainted");

        d.heights[1] = 5.0;
        d.blend = vec![0.0; 25];
        d.blend[1] = 0.25;
        let t = d.transposed();
        assert_eq!((t.heights[5], t.blend[5], t.blend[1]), (5.0, 0.25, 0.0), "blend swaps like heights");
        assert_eq!(t.transposed(), d);
    }

    #[test]
    fn hostile_blend_values_are_tamed() {
        let mut d = Displacement::new(1, DVec3::ZERO);
        d.blend = vec![7.0, -3.0, f64::NAN];
        assert_eq!(d.blend_values(), vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], "short vectors read as 0 past their end");
        assert!(!d.blend_fits());
        d.blend = vec![0.5; 40];
        assert_eq!(d.blend_values().len(), 9, "long vectors are cut to the grid");
        d.set_blend(vec![0.0; 9]);
        assert!(d.blend.is_empty() && d.blend_fits(), "all zeros store nothing");
        d.set_blend(vec![2.0, f64::INFINITY]);
        assert_eq!(d.blend, vec![1.0, 0.0]);
        d.heights.truncate(1);
        assert_eq!(d.transposed().heights.len(), 9, "a short grid comes back full length");
        let grid = d.base(&square());
        assert!(grid.is_none());
        assert_eq!(Displacement::nearest(&square(), DVec3::new(60.0, 50.0, 3.0)), Some(2));
    }

    #[test]
    fn rays_hit_triangles() {
        let tri = [DVec3::ZERO, DVec3::new(10.0, 0.0, 0.0), DVec3::new(0.0, 10.0, 0.0)];
        assert_eq!(ray_triangle(DVec3::new(1.0, 1.0, 5.0), -DVec3::Z, tri), Some(5.0));
        assert_eq!(ray_triangle(DVec3::new(9.0, 9.0, 5.0), -DVec3::Z, tri), None);
    }
}
