use glam::DVec3;

use crate::{EPSILON, Plane};

/// Planes of the convex hull of `points`, each with three of its vertices in the map file order
/// (clockwise seen from outside). `None` when the points do not span a volume.
pub fn convex_hull(points: &[DVec3]) -> Option<Vec<[DVec3; 3]>> {
    let mut pts: Vec<DVec3> = Vec::with_capacity(points.len());
    for p in points {
        if p.is_finite() && !pts.iter().any(|q| q.distance_squared(*p) < EPSILON * EPSILON) {
            pts.push(*p);
        }
    }
    let first = *pts.first()?;
    let b = (1..pts.len()).max_by(|&i, &j| pts[i].distance_squared(first).total_cmp(&pts[j].distance_squared(first)))?;
    let line = pts[b] - first;
    let c = (1..pts.len()).filter(|&i| i != b).max_by(|&i, &j| {
        let d = |k: usize| (pts[k] - first).cross(line).length_squared();
        d(i).total_cmp(&d(j))
    })?;
    let normal = line.cross(pts[c] - first);
    if normal.length() < EPSILON {
        return None;
    }
    let d =
        (1..pts.len()).filter(|&i| i != b && i != c).max_by(|&i, &j| ((pts[i] - first).dot(normal)).abs().total_cmp(&((pts[j] - first).dot(normal)).abs()))?;
    if ((pts[d] - first).dot(normal)).abs() / normal.length() < EPSILON {
        return None;
    }

    // Triangles counter-clockwise seen from outside.
    let plane = |f: &[usize; 3], pts: &[DVec3]| {
        let n = (pts[f[1]] - pts[f[0]]).cross(pts[f[2]] - pts[f[0]]).normalize_or_zero();
        (n, n.dot(pts[f[0]]))
    };
    let mut faces: Vec<[usize; 3]> = vec![[0, b, c], [0, c, d], [0, d, b], [b, d, c]];
    let centroid = (pts[0] + pts[b] + pts[c] + pts[d]) / 4.0;
    for f in &mut faces {
        let (n, dist) = plane(f, &pts);
        if n.dot(centroid) - dist > 0.0 {
            f.swap(1, 2);
        }
    }
    for i in 0..pts.len() {
        if [0, b, c, d].contains(&i) {
            continue;
        }
        let p = pts[i];
        let visible: Vec<bool> = faces
            .iter()
            .map(|f| {
                let (n, dist) = plane(f, &pts);
                n.dot(p) - dist > EPSILON
            })
            .collect();
        if !visible.iter().any(|v| *v) {
            continue;
        }
        let mut horizon = Vec::new();
        for (f, _) in faces.iter().zip(&visible).filter(|(_, v)| **v) {
            for k in 0..3 {
                let (a, e) = (f[k], f[(k + 1) % 3]);
                let shared = faces.iter().zip(&visible).any(|(g, v)| *v && (0..3).any(|m| g[m] == e && g[(m + 1) % 3] == a));
                if !shared {
                    horizon.push((a, e));
                }
            }
        }
        faces = faces.into_iter().zip(visible).filter(|(_, v)| !v).map(|(f, _)| f).collect();
        faces.extend(horizon.into_iter().map(|(a, e)| [a, e, i]));
    }

    // Merge coplanar triangles into one plane each.
    let mut out: Vec<(Plane, [DVec3; 3])> = Vec::new();
    for f in &faces {
        let (n, dist) = plane(f, &pts);
        if n == DVec3::ZERO {
            continue;
        }
        let p = Plane { normal: n, dist };
        if out.iter().any(|(q, _)| q.approx_eq(&p, EPSILON)) {
            continue;
        }
        // Reversed so the points run clockwise seen from outside.
        out.push((p, [pts[f[2]], pts[f[1]], pts[f[0]]]));
    }
    (out.len() >= 4).then(|| out.into_iter().map(|(_, pts)| pts).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Solid;

    fn cube_points() -> Vec<DVec3> {
        let mut v = Vec::new();
        for x in [0.0, 64.0] {
            for y in [0.0, 64.0] {
                for z in [0.0, 64.0] {
                    v.push(DVec3::new(x, y, z));
                }
            }
        }
        v
    }

    fn volume(planes: &[[DVec3; 3]]) -> f64 {
        let planes: Vec<Plane> = planes.iter().map(|p| Plane::from_points(p[0], p[1], p[2]).unwrap()).collect();
        Solid::from_planes(&planes).unwrap().volume()
    }

    #[test]
    fn cube_hull() {
        let mut pts = cube_points();
        pts.push(DVec3::splat(32.0));
        pts.push(DVec3::new(0.0, 0.0, 0.0));
        let hull = convex_hull(&pts).unwrap();
        assert_eq!(hull.len(), 6);
        assert!((volume(&hull) - 64.0f64.powi(3)).abs() < 1e-6);
    }

    #[test]
    fn moved_vertex() {
        let mut pts = cube_points();
        let last = pts.len() - 1;
        pts[last] = DVec3::new(96.0, 96.0, 96.0);
        let hull = convex_hull(&pts).unwrap();
        assert!(volume(&hull) > 64.0f64.powi(3));
        let wedge: Vec<DVec3> = cube_points().into_iter().filter(|p| !(p.x == 64.0 && p.z == 64.0)).collect();
        let hull = convex_hull(&wedge).unwrap();
        assert_eq!(hull.len(), 5);
        assert!((volume(&hull) - 64.0f64.powi(3) / 2.0).abs() < 1e-6);
    }

    #[test]
    fn degenerate() {
        assert!(convex_hull(&[]).is_none());
        assert!(convex_hull(&[DVec3::ZERO, DVec3::X, DVec3::Y, DVec3::new(1.0, 1.0, 0.0)]).is_none());
        assert!(convex_hull(&[DVec3::ZERO, DVec3::X, DVec3::X * 2.0]).is_none());
    }
}
