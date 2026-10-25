use glam::DVec3;

use crate::{Aabb, EPSILON, Plane, WORLD_EXTENT};

/// A convex polygon, counter-clockwise seen from the front of its plane.
#[derive(Clone, Debug, PartialEq)]
pub struct Winding(pub Vec<DVec3>);

impl Winding {
    /// A square on the plane larger than the world, the start of every face polygon.
    pub fn base(plane: &Plane) -> Self {
        let (u, v) = plane.basis();
        let o = plane.normal * plane.dist;
        let s = WORLD_EXTENT * 2.0;
        Self(vec![o - u * s - v * s, o + u * s - v * s, o + u * s + v * s, o - u * s + v * s])
    }

    /// Splits into the parts in front of and behind the plane; points on the plane go to both.
    pub fn split(&self, plane: &Plane) -> (Option<Self>, Option<Self>) {
        let n = self.0.len();
        let dists: Vec<f64> = self.0.iter().map(|p| plane.distance(*p)).collect();
        let (mut front, mut back) = (Vec::with_capacity(n + 1), Vec::with_capacity(n + 1));
        for i in 0..n {
            let (p, q) = (self.0[i], self.0[(i + 1) % n]);
            let (dp, dq) = (dists[i], dists[(i + 1) % n]);
            if dp >= -EPSILON {
                front.push(p);
            }
            if dp <= EPSILON {
                back.push(p);
            }
            if (dp > EPSILON && dq < -EPSILON) || (dp < -EPSILON && dq > EPSILON) {
                let mut mid = p + (q - p) * (dp / (dp - dq));
                // Exact coordinates on axial planes keep integer brushes integer.
                for axis in 0..3 {
                    if plane.normal[axis] == 1.0 {
                        mid[axis] = plane.dist;
                    } else if plane.normal[axis] == -1.0 {
                        mid[axis] = -plane.dist;
                    }
                }
                front.push(mid);
                back.push(mid);
            }
        }
        (Self::cleaned(front), Self::cleaned(back))
    }

    pub fn clip_back(&self, plane: &Plane) -> Option<Self> {
        self.split(plane).1
    }

    fn cleaned(mut points: Vec<DVec3>) -> Option<Self> {
        points.dedup_by(|a, b| a.distance_squared(*b) < EPSILON * EPSILON);
        while points.len() > 1 && points.first().zip(points.last()).is_some_and(|(a, b)| a.distance_squared(*b) < EPSILON * EPSILON) {
            points.pop();
        }
        let w = Self(points);
        (w.0.len() >= 3 && w.area() > EPSILON).then_some(w)
    }

    /// Area-weighted normal (Newell's method); its length is twice the area.
    pub fn area_vector(&self) -> DVec3 {
        let n = self.0.len();
        (0..n).fold(DVec3::ZERO, |acc, i| acc + self.0[i].cross(self.0[(i + 1) % n]))
    }

    pub fn area(&self) -> f64 {
        self.area_vector().length() * 0.5
    }

    pub fn center(&self) -> DVec3 {
        if self.0.is_empty() { DVec3::ZERO } else { self.0.iter().sum::<DVec3>() / self.0.len() as f64 }
    }

    pub fn bounds(&self) -> Option<Aabb> {
        Aabb::from_points(self.0.iter().copied())
    }
}
