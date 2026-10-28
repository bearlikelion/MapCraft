use glam::DVec3;

use crate::{Aabb, EPSILON, Plane, Side, WORLD_EXTENT, Winding};

/// The polygons of a convex solid, one slot per input plane; `None` marks a plane that cuts nothing away.
#[derive(Clone, Debug, PartialEq)]
pub struct Solid {
    pub faces: Vec<Option<Winding>>,
    pub bounds: Aabb,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SolidError {
    #[error("a brush needs at least four faces that bound a volume")]
    TooFewFaces,
    #[error("the brush faces do not enclose a finite volume")]
    Unbounded,
    #[error("the brush has no volume")]
    NoVolume,
}

impl Solid {
    pub fn from_planes(planes: &[Plane]) -> Result<Self, SolidError> {
        let mut faces = Vec::with_capacity(planes.len());
        for (i, plane) in planes.iter().enumerate() {
            let duplicate = planes.iter().take(i).any(|q| q.approx_eq(plane, EPSILON));
            let winding =
                if duplicate { None } else { planes.iter().enumerate().filter(|(j, _)| *j != i).try_fold(Winding::base(plane), |w, (_, q)| w.clip_back(q)) };
            faces.push(winding);
        }
        if faces.iter().flatten().count() < 4 {
            return Err(SolidError::TooFewFaces);
        }
        let bounds = Aabb::from_points(faces.iter().flatten().flat_map(|w| w.0.iter().copied())).ok_or(SolidError::TooFewFaces)?;
        if bounds.min.min_element() <= -WORLD_EXTENT * 0.99 || bounds.max.max_element() >= WORLD_EXTENT * 0.99 {
            return Err(SolidError::Unbounded);
        }
        let solid = Self { faces, bounds };
        if solid.volume() < EPSILON {
            return Err(SolidError::NoVolume);
        }
        Ok(solid)
    }

    pub fn volume(&self) -> f64 {
        self.faces.iter().flatten().map(|w| w.center().dot(w.area_vector())).sum::<f64>() / 6.0
    }

    pub fn vertices(&self) -> impl Iterator<Item = DVec3> + '_ {
        self.faces.iter().flatten().flat_map(|w| w.0.iter().copied())
    }

    pub fn side_of(&self, plane: &Plane) -> Side {
        plane.side_of(self.vertices(), EPSILON)
    }
}

/// Nearest entry of a ray into the convex solid bounded by `planes`: (distance along `dir`, plane index).
pub fn ray_hit(planes: &[Plane], origin: DVec3, dir: DVec3) -> Option<(f64, usize)> {
    let (mut enter, mut exit, mut face) = (f64::NEG_INFINITY, f64::INFINITY, None);
    for (i, p) in planes.iter().enumerate() {
        let denom = p.normal.dot(dir);
        let dist = p.distance(origin);
        if denom.abs() < 1e-12 {
            if dist > 0.0 {
                return None;
            }
            continue;
        }
        let t = -dist / denom;
        if denom < 0.0 {
            if t > enter {
                enter = t;
                face = Some(i);
            }
        } else {
            exit = exit.min(t);
        }
    }
    (enter <= exit && enter >= 0.0).then_some(()).and(face.map(|f| (enter, f)))
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn box_planes(min: DVec3, max: DVec3) -> Vec<Plane> {
        vec![
            Plane { normal: DVec3::X, dist: max.x },
            Plane { normal: -DVec3::X, dist: -min.x },
            Plane { normal: DVec3::Y, dist: max.y },
            Plane { normal: -DVec3::Y, dist: -min.y },
            Plane { normal: DVec3::Z, dist: max.z },
            Plane { normal: -DVec3::Z, dist: -min.z },
        ]
    }

    #[test]
    fn cube() {
        let s = Solid::from_planes(&box_planes(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
        assert_eq!(s.faces.iter().flatten().count(), 6);
        assert!(s.faces.iter().flatten().all(|w| w.0.len() == 4));
        assert!((s.volume() - 64.0f64.powi(3)).abs() < 1e-6);
        assert_eq!(s.bounds, Aabb::new(DVec3::ZERO, DVec3::splat(64.0)));
        // Axial clipping must land exactly on integers.
        assert!(s.vertices().all(|v| v == v.round()));
    }

    #[test]
    fn outward_normals() {
        let s = Solid::from_planes(&box_planes(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
        for w in s.faces.iter().flatten() {
            let out = w.center() - s.bounds.center();
            assert!(w.area_vector().dot(out) > 0.0);
        }
    }

    #[test]
    fn wedge() {
        let mut planes = box_planes(DVec3::ZERO, DVec3::splat(64.0));
        planes.push(Plane::from_points(DVec3::new(0.0, 0.0, 64.0), DVec3::new(0.0, 64.0, 64.0), DVec3::new(64.0, 64.0, 0.0)).unwrap());
        let s = Solid::from_planes(&planes).unwrap();
        assert!((s.volume() - 64.0f64.powi(3) / 2.0).abs() < 1e-6);
        // The top plane now cuts nothing away.
        assert!(s.faces[4].is_none());
    }

    #[test]
    fn redundant_and_duplicate_planes() {
        let mut planes = box_planes(DVec3::ZERO, DVec3::splat(64.0));
        planes.push(Plane { normal: DVec3::X, dist: 128.0 });
        planes.push(Plane { normal: DVec3::Y, dist: 64.0 });
        let s = Solid::from_planes(&planes).unwrap();
        assert!(s.faces[6].is_none());
        assert!(s.faces[7].is_none());
        assert!(s.faces[2].is_some());
    }

    #[test]
    fn invalid_brushes() {
        let mut open = box_planes(DVec3::ZERO, DVec3::splat(64.0));
        open.pop();
        assert_eq!(Solid::from_planes(&open), Err(SolidError::Unbounded));

        let flat = box_planes(DVec3::ZERO, DVec3::new(64.0, 64.0, 0.0));
        assert!(Solid::from_planes(&flat).is_err());

        let disjoint = vec![Plane { normal: DVec3::X, dist: 0.0 }, Plane { normal: -DVec3::X, dist: -10.0 }];
        assert!(Solid::from_planes(&disjoint).is_err());
        assert!(Solid::from_planes(&[]).is_err());
    }

    #[test]
    fn three_points_round_trip() {
        let p = Plane::from_points(DVec3::new(1.0, 2.0, 3.0), DVec3::new(5.0, -2.0, 7.0), DVec3::new(-3.0, 4.0, 1.0)).unwrap();
        let [a, b, c] = p.three_points();
        assert!(Plane::from_points(a, b, c).unwrap().approx_eq(&p, 1e-9));
        assert!(Plane::from_points(DVec3::ZERO, DVec3::ZERO, DVec3::X).is_none());
        assert!(Plane::from_points(DVec3::NAN, DVec3::ZERO, DVec3::X).is_none());
    }

    #[test]
    fn rays() {
        let planes = box_planes(DVec3::ZERO, DVec3::splat(64.0));
        let (t, face) = ray_hit(&planes, DVec3::new(32.0, 32.0, 200.0), -DVec3::Z).unwrap();
        assert!((t - 136.0).abs() < 1e-9);
        assert_eq!(face, 4);
        assert!(ray_hit(&planes, DVec3::new(100.0, 32.0, 200.0), -DVec3::Z).is_none());
        assert!(ray_hit(&planes, DVec3::new(32.0, 32.0, 200.0), DVec3::Z).is_none());
    }

    #[test]
    fn split_winding() {
        let w = Winding::base(&Plane { normal: DVec3::Z, dist: 0.0 });
        let (f, b) = w.split(&Plane { normal: DVec3::X, dist: 0.0 });
        let (f, b) = (f.unwrap(), b.unwrap());
        assert!((f.area() - b.area()).abs() < 1e-3 * f.area());
        assert!(w.split(&Plane { normal: DVec3::Z, dist: 0.0 }).0.is_some());
    }

    #[test]
    fn overlapping_windings() {
        let square = |x: f64| Winding(vec![DVec3::new(x, 0.0, 0.0), DVec3::new(x + 64.0, 0.0, 0.0), DVec3::new(x + 64.0, 64.0, 0.0), DVec3::new(x, 64.0, 0.0)]);
        let flipped = |w: Winding| Winding(w.0.into_iter().rev().collect());
        assert!(square(0.0).overlaps(&flipped(square(32.0))));
        assert!(!square(0.0).overlaps(&square(64.0)), "touching along an edge is no overlap");
        assert!(!flipped(square(0.0)).overlaps(&square(100.0)));
    }

    #[test]
    fn travel_snaps_corners_to_the_grid() {
        let up = [(DVec3::new(0.0, 0.0, 17.0), DVec3::Z)];
        assert_eq!(crate::snap_travel(&up, 13.0, 16.0), 15.0, "an off-grid top lands on the next grid plane");
        assert_eq!(crate::snap_travel(&[], 13.0, 16.0), 13.0);
        assert_eq!(crate::snap_travel(&up, 13.0, 0.0), 13.0);
    }
}
