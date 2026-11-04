use glam::DVec3;

const NEAR: f64 = 1e-4;

/// Where edge `a`-`b` starts in polygon `w`, in either direction.
pub fn edge_index(w: &[DVec3], a: DVec3, b: DVec3) -> Option<usize> {
    let n = w.len();
    let near = |p: DVec3, q: DVec3| p.distance_squared(q) < NEAR;
    (0..n).find(|&i| {
        let (p, q) = (w[i], w[(i + 1) % n]);
        (near(p, a) && near(q, b)) || (near(p, b) && near(q, a))
    })
}

/// Blender's edge ring on a convex solid's `faces`: from edge `a`-`b` through each quad beside it
/// to the quad's opposite edge, on until a face is not a quad or the ring closes. Returns the
/// edges, starting with `a`-`b`, and the indices of the quads crossed.
pub fn edge_ring(faces: &[Vec<DVec3>], a: DVec3, b: DVec3) -> (Vec<(DVec3, DVec3)>, Vec<usize>) {
    let beside = |p: DVec3, q: DVec3| faces.iter().enumerate().filter(move |(_, w)| edge_index(w, p, q).is_some()).map(|(i, _)| i);
    let mut edges = vec![(a, b)];
    let mut quads: Vec<usize> = Vec::new();
    for start in beside(a, b).collect::<Vec<_>>() {
        let (mut f, mut e) = (start, (a, b));
        while let Some(w) = faces.get(f).filter(|w| w.len() == 4 && !quads.contains(&f))
            && let Some(i) = edge_index(w, e.0, e.1)
        {
            quads.push(f);
            e = (w[(i + 2) % 4], w[(i + 3) % 4]);
            if edge_index(&[a, b], e.0, e.1).is_some() {
                return (edges, quads);
            }
            edges.push(e);
            let Some(next) = beside(e.0, e.1).find(|j| *j != f) else { break };
            f = next;
        }
    }
    (edges, quads)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Plane, Solid};

    fn faces(planes: &[Plane]) -> Vec<Vec<DVec3>> {
        Solid::from_planes(planes).unwrap().faces.into_iter().flatten().map(|w| w.0).collect()
    }

    fn box_planes() -> Vec<Plane> {
        let p = |normal: DVec3, dist: f64| Plane { normal, dist };
        vec![p(DVec3::NEG_X, 0.0), p(DVec3::X, 64.0), p(DVec3::NEG_Y, 0.0), p(DVec3::Y, 32.0), p(DVec3::NEG_Z, 0.0), p(DVec3::Z, 16.0)]
    }

    #[test]
    fn box_ring_closes_through_four_faces() {
        let f = faces(&box_planes());
        let (edges, quads) = edge_ring(&f, DVec3::new(0.0, 0.0, 16.0), DVec3::new(64.0, 0.0, 16.0));
        assert_eq!(edges.len(), 4);
        assert!(edges.iter().all(|(p, q)| ((p.x - q.x).abs() - 64.0).abs() < 1e-3));
        assert_eq!(quads.len(), 4);
    }

    #[test]
    fn ring_stops_at_a_face_that_is_not_a_quad() {
        let mut planes = box_planes();
        // Cutting one top corner makes the top a pentagon and adds a triangle.
        planes.push(Plane { normal: DVec3::ONE.normalize(), dist: 104.0 / 3f64.sqrt() });
        let f = faces(&planes);
        let (edges, quads) = edge_ring(&f, DVec3::new(0.0, 0.0, 0.0), DVec3::new(0.0, 0.0, 16.0));
        assert_eq!(quads.len(), 2);
        assert_eq!(edges.len(), 3);
    }
}
