//! RGBE9995: three 9-bit mantissas sharing a 5-bit exponent in one u32 (red in the low bits, the exponent on top),
//! the shared-exponent format GPUs sample as float and Godot calls `FORMAT_RGBE9995`.

use mapcraft_geom::DVec3;

const BIAS: i32 = 15;
const MANTISSA: i32 = 9;
const MAX_EXP: i32 = 31;

/// The largest value the format holds.
pub const MAX: f64 = 511.0 / 512.0 * 65_536.0;

/// The mantissa bits a texel holds per channel.
pub const BITS: u32 = 9;

/// `c` kept to `bits` significant bits a channel, which leaves the low mantissa bits of its texel zero.
/// A bake's own noise is far finer than the 6 or 7 bits worth asking for, and the atlas deflates much smaller for it.
pub fn rounded(c: DVec3, bits: u32) -> DVec3 {
    if bits >= BITS {
        return c;
    }
    DVec3::new(kept(c.x, bits), kept(c.y, bits), kept(c.z, bits))
}

/// `v` rounded to the nearest multiple of a step half the size of its own leading bit, `bits` steps down.
fn kept(v: f64, bits: u32) -> f64 {
    if !v.is_finite() || v == 0.0 {
        return 0.0;
    }
    let step = 2f64.powf(v.abs().log2().floor() - f64::from(bits) + 1.0);
    (v / step).round() * step
}

/// The texel of `c` with `bits` significant bits kept a channel.
pub fn encode_rounded(c: DVec3, bits: u32) -> u32 {
    encode(rounded(c, bits))
}

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

    #[test]
    fn keeping_nine_bits_changes_nothing() {
        for c in [DVec3::new(0.3, 1.7, 0.0), DVec3::new(1e-7, 5.0, 900.0), DVec3::ZERO] {
            assert_eq!(rounded(c, 9), c);
            assert_eq!(encode_rounded(c, 9), encode(c));
        }
    }

    #[test]
    fn kept_bits_stay_inside_the_step_and_keep_the_small_ones() {
        let c = DVec3::new(1.234, 0.000_123_4, 0.0);
        for bits in 4..9 {
            let r = rounded(c, bits);
            let step = 1.234f64 / 2f64.powi(bits as i32);
            assert!((r.x - c.x).abs() <= step, "{bits} bits: {} vs {}", r.x, c.x);
            assert!(r.y > 0.0, "{bits} bits leaves the dim channel lit: {r}");
            assert!((r.y - c.y).abs() <= c.y / 2f64.powi(bits as i32 - 1), "{bits} bits: {} vs {}", r.y, c.y);
        }
        assert_eq!(rounded(DVec3::new(f64::NAN, f64::INFINITY, 0.0), 6), DVec3::ZERO);
    }

    #[test]
    fn kept_bits_zero_the_low_bits_of_a_texel_mantissa() {
        // The brightest channel sets the shared exponent, so its kept bits show up as zero low bits.
        let c = DVec3::new(1.234_567, 0.6, 0.123_456);
        for (bits, zeros) in [(7, 2), (6, 3), (5, 4)] {
            let t = encode_rounded(c, bits);
            let m = t & 0x1FF;
            assert_eq!(m & ((1 << zeros) - 1), 0, "{bits} bits leaves the brightest mantissa's low {zeros} bits zero: {m:b}");
        }
        assert_ne!(encode_rounded(DVec3::new(1.0, 0.999, 0.998), 7), encode(DVec3::new(1.0, 0.999, 0.998)), "and that is another texel than all nine bits");
    }
}
