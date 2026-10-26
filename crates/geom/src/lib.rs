//! Math and the brush kernel: planes, windings and convex solids built from planes.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod aabb;
pub mod curve;
mod hull;
mod plane;
mod solid;
mod winding;

pub use aabb::Aabb;
pub use glam::{DAffine2, DMat2, DMat3, DMat4, DQuat, DVec2, DVec3, DVec4};
pub use hull::convex_hull;
pub use plane::{Plane, Side};
pub use solid::{Solid, SolidError, ray_hit};
pub use winding::Winding;

/// Half the side of the cube every brush is clipped from; vertices this far out mean an unbounded brush.
pub const WORLD_EXTENT: f64 = 131_072.0;

/// Distance under which a point counts as lying on a plane.
pub const EPSILON: f64 = 1e-3;

pub fn snap(v: DVec3, grid: f64) -> DVec3 {
    if grid > 0.0 && grid.is_finite() { (v / grid).round() * grid } else { v }
}
