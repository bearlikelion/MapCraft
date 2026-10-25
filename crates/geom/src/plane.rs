use glam::DVec3;

/// A plane `normal . p = dist`; the solid side is behind it (`normal . p <= dist`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plane {
    pub normal: DVec3,
    pub dist: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Front,
    Back,
    On,
    Spanning,
}

impl Plane {
    /// The map file convention: points run clockwise seen from outside, normal = (a - b) x (c - b).
    pub fn from_points(a: DVec3, b: DVec3, c: DVec3) -> Option<Self> {
        let n = (a - b).cross(c - b);
        let len = n.length();
        if !len.is_finite() || len <= 1e-9 {
            return None;
        }
        let normal = n / len;
        Some(Self { normal, dist: normal.dot(b) })
    }

    /// Three points that reproduce this plane through [`Plane::from_points`].
    pub fn three_points(&self) -> [DVec3; 3] {
        let (u, v) = self.basis();
        let o = self.normal * self.dist;
        [o + u * 64.0, o, o + v * 64.0]
    }

    /// Orthonormal tangents with `u x v = normal`.
    pub fn basis(&self) -> (DVec3, DVec3) {
        let n = self.normal;
        let helper = if n.z.abs() < 0.9 { DVec3::Z } else { DVec3::X };
        let u = helper.cross(n).normalize_or_zero();
        (u, n.cross(u))
    }

    pub fn distance(&self, p: DVec3) -> f64 {
        self.normal.dot(p) - self.dist
    }

    pub fn flipped(&self) -> Self {
        Self { normal: -self.normal, dist: -self.dist }
    }

    pub fn approx_eq(&self, o: &Self, eps: f64) -> bool {
        (self.dist - o.dist).abs() <= eps && self.normal.abs_diff_eq(o.normal, 1e-6)
    }

    pub fn side_of(&self, points: impl IntoIterator<Item = DVec3>, eps: f64) -> Side {
        let (mut front, mut back) = (false, false);
        for p in points {
            let d = self.distance(p);
            front |= d > eps;
            back |= d < -eps;
        }
        match (front, back) {
            (true, true) => Side::Spanning,
            (true, false) => Side::Front,
            (false, true) => Side::Back,
            (false, false) => Side::On,
        }
    }
}
