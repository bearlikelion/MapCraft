//! Value noise on a hashed integer lattice, as the editor's shaders and the Godot addon compute it.

use glam::DVec3;

// Shared with crates/render/src/shaders.wgsl and the Godot addon's mapcraft_surface.gdshaderinc; change all three together.
pub const MACRO_CELLS: [f64; 2] = [512.0, 197.0];
pub const MACRO_AMOUNT: f64 = 0.15;

pub fn mix32(x: u32) -> u32 {
    let mut h = x ^ (x >> 16);
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^ (h >> 16)
}

/// A value in 0..1 for the lattice cell at whole-number coordinates `c`.
pub fn lattice(c: DVec3) -> f64 {
    seeded_lattice(c, 0)
}

/// [`lattice`] with `seed` xored into the first hash.
fn seeded_lattice(c: DVec3, seed: u32) -> f64 {
    let u = |v: f64| (v as i32).wrapping_add(1_048_576) as u32;
    f64::from(mix32(u(c.x) ^ mix32(u(c.y) ^ mix32(u(c.z) ^ seed))) >> 8) / 16_777_216.0
}

pub fn value_noise(p: DVec3) -> f64 {
    seeded_noise(p, 0)
}

/// Value noise in 0..1 whose pattern each `seed` changes.
pub fn seeded_noise(p: DVec3, seed: u32) -> f64 {
    let i = p.floor();
    let f = p - i;
    let s = f * f * (3.0 - 2.0 * f);
    let at = |x: f64, y: f64, z: f64| seeded_lattice(i + DVec3::new(x, y, z), seed);
    let mix = |a: f64, b: f64, t: f64| a + (b - a) * t;
    let x00 = mix(at(0.0, 0.0, 0.0), at(1.0, 0.0, 0.0), s.x);
    let x10 = mix(at(0.0, 1.0, 0.0), at(1.0, 1.0, 0.0), s.x);
    let x01 = mix(at(0.0, 0.0, 1.0), at(1.0, 0.0, 1.0), s.x);
    let x11 = mix(at(0.0, 1.0, 1.0), at(1.0, 1.0, 1.0), s.x);
    mix(mix(x00, x10, s.y), mix(x01, x11, s.y), s.z)
}

/// The factor anti-tiling multiplies albedo by at a map point.
pub fn macro_variation(world: DVec3) -> f64 {
    let n = 0.6 * value_noise(world / MACRO_CELLS[0]) + 0.4 * value_noise(world / MACRO_CELLS[1]);
    1.0 + MACRO_AMOUNT * (2.0 * n - 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_shaders() {
        assert_eq!([mix32(0), mix32(1), mix32(0xffff_ffff)], [0, 1_753_845_952, 1_734_902_346]);
        for (c, v) in [
            ([0.0, 0.0, 0.0], 0.3226417303085327),
            ([1.0, 2.0, 3.0], 0.27086812257766724),
            ([-1.0, -1.0, -1.0], 0.48611706495285034),
            ([-5.0, 7.0, -11.0], 0.2217172384262085),
            ([100.0, -200.0, 300.0], 0.9877477884292603),
        ] {
            assert_eq!(lattice(DVec3::from_array(c)), v, "{c:?}");
        }
        assert!((macro_variation(DVec3::new(-600.0, 0.0, 0.0)) - 0.8630210879367924).abs() < 1e-12);
        for p in [DVec3::ZERO, DVec3::splat(-1e9), DVec3::new(123.4, -5.6, 7.8)] {
            let v = macro_variation(p);
            assert!((1.0 - MACRO_AMOUNT..=1.0 + MACRO_AMOUNT).contains(&v), "{p} {v}");
        }
    }
}
