//! RGBE9995: three 9-bit mantissas sharing a 5-bit exponent in one u32 (red in the low bits, the exponent on top),
//! the shared-exponent format GPUs sample as float and Godot calls `FORMAT_RGBE9995`.

use mapcraft_geom::DVec3;

const BIAS: i32 = 15;
const MANTISSA: i32 = 9;
const MAX_EXP: i32 = 31;

/// The largest value the format holds.
pub const MAX: f64 = 511.0 / 512.0 * 65_536.0;

pub fn encode(c: DVec3) -> u32 {
    let c = DVec3::new(clean(c.x), clean(c.y), clean(c.z));
    let max = c.max_element();
    if max <= 0.0 {
        return 0;
    }
    let mut exp = (max.log2().floor() as i32).max(-BIAS - 1) + 1 + BIAS;
    let mut scale = 2f64.powi(exp - BIAS - MANTISSA);
    if (max / scale).round() >= 512.0 {
        exp += 1;
        scale *= 2.0;
    }
    if exp > MAX_EXP {
        return encode(DVec3::splat(MAX).min(c) * 0.999);
    }
    let m = |v: f64| ((v / scale).round() as u32).min(511);
    m(c.x) | (m(c.y) << 9) | (m(c.z) << 18) | ((exp.clamp(0, MAX_EXP) as u32) << 27)
}

fn clean(v: f64) -> f64 {
    if v.is_finite() { v.clamp(0.0, MAX) } else { 0.0 }
}

pub fn decode(v: u32) -> DVec3 {
    let scale = 2f64.powi((v >> 27) as i32 - BIAS - MANTISSA);
    DVec3::new(f64::from(v & 0x1FF), f64::from((v >> 9) & 0x1FF), f64::from((v >> 18) & 0x1FF)) * scale
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_survive_a_round_trip() {
        for c in [DVec3::new(1.0, 0.5, 0.25), DVec3::new(0.001, 0.002, 0.0), DVec3::new(37.0, 2.0, 0.5), DVec3::splat(0.999)] {
            let back = decode(encode(c));
            let tolerance = c.max_element() / 256.0;
            assert!((back - c).abs().max_element() <= tolerance, "{c} came back {back}");
        }
        assert_eq!(decode(encode(DVec3::ZERO)), DVec3::ZERO);
        assert_eq!(encode(DVec3::new(f64::NAN, -1.0, 0.0)), 0);
        assert!(decode(encode(DVec3::splat(1e9))).x > 60_000.0);
    }

    #[test]
    fn one_matches_godots_bit_layout() {
        // 1.0 is mantissa 256 with exponent 16: 2^(16 - 24) * 256.
        assert_eq!(encode(DVec3::new(1.0, 0.0, 0.0)), 256 | (16 << 27));
    }
}
