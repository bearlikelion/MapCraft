use glam::DVec3;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Aabb {
    pub min: DVec3,
    pub max: DVec3,
}

impl Aabb {
    pub fn new(a: DVec3, b: DVec3) -> Self {
        Self { min: a.min(b), max: a.max(b) }
    }

    pub fn from_points(points: impl IntoIterator<Item = DVec3>) -> Option<Self> {
        let mut it = points.into_iter();
        let first = it.next()?;
        Some(it.fold(Self { min: first, max: first }, |b, p| Self { min: b.min.min(p), max: b.max.max(p) }))
    }

    pub fn union(&self, o: &Self) -> Self {
        Self { min: self.min.min(o.min), max: self.max.max(o.max) }
    }

    pub fn intersects(&self, o: &Self) -> bool {
        self.min.cmple(o.max).all() && o.min.cmple(self.max).all()
    }

    pub fn contains(&self, p: DVec3) -> bool {
        self.min.cmple(p).all() && p.cmple(self.max).all()
    }

    pub fn center(&self) -> DVec3 {
        (self.min + self.max) * 0.5
    }

    pub fn size(&self) -> DVec3 {
        self.max - self.min
    }

    /// Distance along `dir` at which a ray enters the box (0 when it starts inside).
    pub fn ray_hit(&self, origin: DVec3, dir: DVec3) -> Option<f64> {
        let (mut enter, mut exit) = (0.0f64, f64::INFINITY);
        for i in 0..3 {
            if dir[i].abs() < 1e-12 {
                if origin[i] < self.min[i] || origin[i] > self.max[i] {
                    return None;
                }
                continue;
            }
            let (a, b) = ((self.min[i] - origin[i]) / dir[i], (self.max[i] - origin[i]) / dir[i]);
            enter = enter.max(a.min(b));
            exit = exit.min(a.max(b));
        }
        (enter <= exit).then_some(enter)
    }

    pub fn expanded(&self, by: f64) -> Self {
        Self { min: self.min - DVec3::splat(by), max: self.max + DVec3::splat(by) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ray_hits() {
        let b = Aabb::new(DVec3::ZERO, DVec3::splat(10.0));
        assert_eq!(b.ray_hit(DVec3::new(5.0, 5.0, 20.0), -DVec3::Z), Some(10.0));
        assert_eq!(b.ray_hit(DVec3::new(5.0, 5.0, 5.0), DVec3::X), Some(0.0));
        assert_eq!(b.ray_hit(DVec3::new(15.0, 5.0, 20.0), -DVec3::Z), None);
        assert_eq!(b.ray_hit(DVec3::new(5.0, 5.0, 20.0), DVec3::Z), None);
    }
}
