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
}

impl Displacement {
    pub fn new(power: u32, start: DVec3) -> Self {
        let n = side(power);
        Self { power: power.clamp(1, MAX_POWER), start, heights: vec![0.0; n * n] }
    }

    /// Vertices along one side of the grid.
    pub fn size(&self) -> usize {
        side(self.power)
    }

    /// The same surface at another power, sampled bilinearly.
    pub fn resampled(&self, power: u32) -> Self {
        let (n, m) = (self.size(), side(power));
        let mut out = Self::new(power, self.start);
        for j in 0..m {
            for i in 0..m {
                let (u, v) = (i as f64 / (m - 1) as f64, j as f64 / (m - 1) as f64);
                out.heights[j * m + i] = self.sample(u * (n - 1) as f64, v * (n - 1) as f64);
            }
        }
        out
    }

    /// Height at fractional grid coordinates.
    fn sample(&self, x: f64, y: f64) -> f64 {
        let n = self.size();
        let at = |i: usize, j: usize| self.heights.get(j.min(n - 1) * n + i.min(n - 1)).copied().unwrap_or(0.0);
        let (i, j) = (x.floor().max(0.0) as usize, y.floor().max(0.0) as usize);
        let (fx, fy) = (x - i as f64, y - j as f64);
        let top = at(i, j) * (1.0 - fx) + at(i + 1, j) * fx;
        let bottom = at(i, j + 1) * (1.0 - fx) + at(i + 1, j + 1) * fx;
        top * (1.0 - fy) + bottom * fy
    }

    /// The grid swapped across its diagonal, which keeps a mirrored brush's surface in place.
    pub fn transposed(&self) -> Self {
        let n = self.size();
        let mut out = self.clone();
        for j in 0..n {
            for i in 0..n {
                out.heights[j * n + i] = self.heights.get(i * n + j).copied().unwrap_or(0.0);
            }
        }
        out
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
    fn rays_hit_triangles() {
        let tri = [DVec3::ZERO, DVec3::new(10.0, 0.0, 0.0), DVec3::new(0.0, 10.0, 0.0)];
        assert_eq!(ray_triangle(DVec3::new(1.0, 1.0, 5.0), -DVec3::Z, tri), Some(5.0));
        assert_eq!(ray_triangle(DVec3::new(9.0, 9.0, 5.0), -DVec3::Z, tri), None);
    }
}
