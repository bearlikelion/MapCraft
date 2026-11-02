//! Lights read from a map's entities: Source-style `light` and `light_spot`, and the Godot-style
//! `light_environment` sun and sky the MapCraft Godot addon builds.

use mapcraft_geom::DVec3;
use mapcraft_map::{Entity, Map, NodeKind};

/// Light dimmer than this, in linear units, counts as none when finding a light's reach.
const CUTOFF: f64 = 1.0 / 512.0;

/// The farthest any point light reaches, so a light with no falloff still has a bound.
const MAX_RANGE: f64 = 65_536.0;

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Point,
    /// Cosines of the outer and inner half angles, and the falloff exponent between them.
    Spot {
        dir: DVec3,
        cos_outer: f64,
        cos_inner: f64,
        exponent: f64,
    },
    /// `toward` points at the sun; `spread` is the sun's angular radius in radians.
    Sun {
        toward: DVec3,
        spread: f64,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Falloff {
    /// `1 / (c + l d + q d^2)`, Source's attenuation keys.
    Quadratic { c: f64, l: f64, q: f64 },
    /// `1 / (c + l d + q d^2)` fit through the 50% distance, ending at `end` with a fade from `fade` when hard.
    Distances { c: f64, l: f64, q: f64, fade: f64, end: f64, hard: bool },
    /// Full brightness at the light, easing to nothing at `end`, as a light with only a range reads.
    Range { end: f64 },
}

impl Falloff {
    /// The light's fraction left at distance `d`.
    pub fn at(&self, d: f64) -> f64 {
        let d = d.max(1.0);
        match *self {
            Falloff::Quadratic { c, l, q } => 1.0 / (c + l * d + q * d * d).max(1e-9),
            Falloff::Range { end } => {
                let t = (d / end.max(1.0)).min(1.0);
                (1.0 - t * t).powi(2)
            }
            Falloff::Distances { c, l, q, fade, end, hard } => {
                if d >= end {
                    return 0.0;
                }
                let f = (1.0 / (c + l * d + q * d * d).max(1e-9)).min(1.0);
                if hard && d > fade {
                    let t = ((d - fade) / (end - fade).max(1e-9)).clamp(0.0, 1.0);
                    f * (1.0 - t * t * t * (t * (t * 6.0 - 15.0) + 10.0))
                } else {
                    f
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Light {
    pub kind: Kind,
    pub pos: DVec3,
    /// Linear color times brightness, already scaled so `falloff` alone dims it.
    pub color: DVec3,
    pub falloff: Falloff,
    /// Beyond this distance the light adds nothing visible.
    pub range: f64,
    /// A `range` the mapper set: the light fades out to nothing there, as Godot's lights do.
    pub cutoff: Option<f64>,
    /// The light's diameter, which softens its shadows; 0 casts hard ones.
    pub size: f64,
}

impl Light {
    /// The light's fraction left at distance `d`, after its range.
    pub fn attenuation(&self, d: f64) -> f64 {
        let window = self.cutoff.map_or(1.0, |r| {
            let t = (d / r.max(1e-9)).powi(4);
            ((1.0 - t).max(0.0)).powi(2)
        });
        self.falloff.at(d) * window
    }
}

/// Godot's procedural sky as `light_environment` describes it, in linear color.
#[derive(Clone, Debug, PartialEq)]
pub struct Sky {
    pub top: DVec3,
    pub horizon: DVec3,
    pub ground: DVec3,
    pub energy: f64,
}

impl Sky {
    /// The light the sky sends along `-dir`, seen looking along `dir`.
    pub fn radiance(&self, dir: DVec3) -> DVec3 {
        let z = dir.z.clamp(-1.0, 1.0);
        let c = if z >= 0.0 { self.horizon.lerp(self.top, z.sqrt()) } else { self.horizon.lerp(self.ground, (-z).sqrt()) };
        c * self.energy
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lights {
    pub lights: Vec<Light>,
    pub sky: Option<Sky>,
    /// Worldspawn `_minlight`: light no luxel falls below, linear.
    pub minlight: DVec3,
}

fn numbers(s: &str) -> Vec<f64> {
    s.split_whitespace().take(8).map_while(|t| t.parse::<f64>().ok().filter(|v| v.is_finite())).collect()
}

fn number(e: &Entity, key: &str) -> Option<f64> {
    numbers(e.get(key)?).first().copied()
}

/// "r g b" in 0 to 255 as linear color.
fn srgb(v: &[f64]) -> Option<DVec3> {
    let &[r, g, b, ..] = v else { return None };
    Some(DVec3::new(r, g, b).clamp(DVec3::ZERO, DVec3::splat(255.0)) / 255.0).map(|c| c.powf(2.2))
}

/// Unit vector for a yaw and pitch in degrees; pitch -90 points straight down.
fn aim(yaw: f64, pitch: f64) -> DVec3 {
    let (y, p) = (yaw.to_radians(), pitch.to_radians());
    DVec3::new(p.cos() * y.cos(), p.cos() * y.sin(), p.sin())
}

/// Reads every light in `map`, with warnings for keys that were ignored.
pub fn read(map: &Map) -> (Lights, Vec<String>) {
    let mut out = Lights::default();
    let mut warnings = Vec::new();
    for (_, node) in map.nodes() {
        let NodeKind::Entity(e) = &node.kind else { continue };
        if e.classname == "light_environment" {
            let (sun, sky) = environment(e, &mut warnings);
            out.lights.extend(sun);
            out.sky = Some(sky);
        } else if let Some(l) = entity(e, &mut warnings) {
            out.lights.push(l);
        }
    }
    let world = map.world();
    if let Some(min) = world.get("_minlight").map(numbers) {
        out.minlight = match min.as_slice() {
            [v] => DVec3::splat(v / 255.0),
            v => srgb(v).unwrap_or_default() * v.get(3).map_or(1.0, |b| b / 255.0),
        };
    }
    (out, warnings)
}

/// The sun and sky of a `light_environment`, read with the Godot addon's keys and defaults.
fn environment(e: &Entity, warnings: &mut Vec<String>) -> (Option<Light>, Sky) {
    let color = |key: &str, default: [f64; 3]| e.get(key).and_then(|s| srgb(&numbers(s))).unwrap_or(DVec3::from_array(default).powf(2.2));
    let bounded = |key: &str, default: f64, max: f64, warnings: &mut Vec<String>| match e.get(key).map(|s| numbers(s).first().copied()) {
        None => default,
        Some(Some(v)) if (0.0..=max).contains(&v) => v,
        Some(_) => {
            warnings.push(format!("light_environment: {key} must be a number from 0 to {max}; using {default}"));
            default
        }
    };
    let angles = numbers(e.get("angles").unwrap_or("50 135 0"));
    let (pitch, yaw) = match angles.as_slice() {
        [p, y, ..] => (*p, *y),
        _ => (50.0, 135.0),
    };
    let energy = bounded("sun_energy", 1.0, 64.0, warnings);
    let spread = (bounded("sun_spread", 0.5, 90.0, warnings) / 2.0).to_radians();
    let sun = Light {
        kind: Kind::Sun { toward: -aim(yaw, -pitch), spread },
        pos: DVec3::ZERO,
        color: color("sun_color", [1.0, 0.96, 0.9]) * energy,
        falloff: Falloff::Quadratic { c: 1.0, l: 0.0, q: 0.0 },
        range: f64::INFINITY,
        cutoff: None,
        size: 0.0,
    };
    let sky = Sky {
        top: color("sky_top_color", [0.3, 0.48, 0.75]),
        horizon: color("sky_horizon_color", [0.72, 0.8, 0.88]),
        ground: color("ground_color", [0.2, 0.21, 0.23]),
        energy: bounded("ambient_energy", 1.0, 64.0, warnings),
    };
    (Some(sun).filter(|s| s.color.max_element() > 0.0), sky)
}

/// A Source-style point or spot light: `_light "r g b brightness"`, brightness meaning the light at 100 units,
/// with Godot's `range` and `size`.
pub fn entity(e: &Entity, warnings: &mut Vec<String>) -> Option<Light> {
    let name = e.classname.as_str();
    if name != "light" && name != "light_spot" {
        return None;
    }
    let Some(pos) = e.origin() else {
        warnings.push(format!("{name} has no origin and was skipped"));
        return None;
    };
    let raw = numbers(e.get("_light").unwrap_or("255 255 255 200"));
    let (color, brightness) = match raw.as_slice() {
        [v] => (DVec3::ONE, *v),
        [r, g, b] => (srgb(&[*r, *g, *b])?, 200.0),
        [r, g, b, v, ..] => (srgb(&[*r, *g, *b])?, *v),
        _ => {
            warnings.push(format!("{name} at {pos}: _light must be \"r g b brightness\"; skipped"));
            return None;
        }
    };
    let color = color * brightness.max(0.0) / 255.0;
    let fifty = number(e, "_fifty_percent_distance").filter(|d| *d > 0.0);
    let range = number(e, "range").filter(|r| *r > 0.0).unwrap_or(0.0);
    let (falloff, color) = match fifty {
        Some(d50) => {
            let end = number(e, "_zero_percent_distance").filter(|d| *d > d50).unwrap_or(d50 * 4.0);
            // 1 / falloff runs through 1 at 0, 2 at d50 and 256 at the end, so half the light is left at d50.
            let q = (255.0 * d50 - end) / (end * d50 * (end - d50));
            let l = (1.0 - q * d50 * d50) / d50;
            let hard = number(e, "_hardfalloff").is_some_and(|v| v != 0.0);
            (Falloff::Distances { c: 1.0, l, q, fade: end * 0.75 + d50 * 0.25, end, hard }, color)
        }
        None if range > 0.0 && !["_constant_attn", "_linear_attn"].iter().any(|k| number(e, k).is_some_and(|v| v > 0.0)) => {
            (Falloff::Range { end: range }, color)
        }
        None => {
            let (c, l, q) = (number(e, "_constant_attn").unwrap_or(0.0), number(e, "_linear_attn").unwrap_or(0.0), number(e, "_quadratic_attn").unwrap_or(0.0));
            let (c, l, q) = if c + l + q <= 0.0 { (0.0, 0.0, 1.0) } else { (c.max(0.0), l.max(0.0), q.max(0.0)) };
            // The brightness is what the light gives at 100 units.
            (Falloff::Quadratic { c, l, q }, color * (c + 100.0 * l + 10_000.0 * q))
        }
    };
    let kind = if name == "light_spot" {
        let angles = numbers(e.get("angles").unwrap_or_default());
        let pitch = number(e, "pitch").or(angles.first().copied()).unwrap_or(-90.0);
        let yaw = angles.get(1).copied().unwrap_or(0.0);
        let outer = number(e, "_cone").unwrap_or(45.0).clamp(0.0, 90.0);
        let inner = number(e, "_inner_cone").unwrap_or(30.0).clamp(0.0, outer);
        Kind::Spot {
            dir: aim(yaw, pitch),
            cos_outer: outer.to_radians().cos(),
            cos_inner: inner.to_radians().cos(),
            exponent: number(e, "_exponent").unwrap_or(1.0).max(0.0),
        }
    } else {
        Kind::Point
    };
    // A range-only light already fades out at its range, so it needs no second fade.
    let cutoff = Some(range).filter(|r| *r > 0.0 && !matches!(falloff, Falloff::Range { .. }));
    let range = if range > 0.0 { range } else { reach(&falloff, color.max_element()) };
    let size = number(e, "size").unwrap_or(0.0).clamp(0.0, 4096.0);
    Some(Light { kind, pos, color, falloff, range, cutoff, size })
}

/// The distance past which a light of peak `brightness` gives less than [`CUTOFF`].
fn reach(falloff: &Falloff, brightness: f64) -> f64 {
    if let Falloff::Distances { end, .. } | Falloff::Range { end } = falloff {
        return *end;
    }
    let (mut lo, mut hi) = (1.0, MAX_RANGE);
    if brightness * falloff.at(hi) >= CUTOFF {
        return hi;
    }
    for _ in 0..48 {
        let mid = (lo + hi) / 2.0;
        if brightness * falloff.at(mid) >= CUTOFF { lo = mid } else { hi = mid }
    }
    hi
}

#[cfg(test)]
mod tests {
    use super::*;

    fn light(keys: &[(&str, &str)]) -> Light {
        let mut e = Entity::new("light");
        e.set("origin", "0 0 0");
        for (k, v) in keys {
            e.set(*k, *v);
        }
        entity(&e, &mut Vec::new()).unwrap()
    }

    #[test]
    fn brightness_is_the_light_at_100_units() {
        let l = light(&[("_light", "255 255 255 255")]);
        assert!((l.color.x * l.falloff.at(100.0) - 1.0).abs() < 1e-9);
        assert!((l.color.x * l.falloff.at(200.0) - 0.25).abs() < 1e-9);
    }

    #[test]
    fn fifty_percent_distance_halves_the_light() {
        let l = light(&[("_light", "255 255 255 255"), ("_fifty_percent_distance", "300"), ("_zero_percent_distance", "1200")]);
        assert!((l.falloff.at(1.0) - 1.0).abs() < 0.01);
        assert!((l.falloff.at(300.0) - 0.5).abs() < 1e-6);
        assert_eq!(l.falloff.at(1200.0), 0.0);
        assert_eq!(l.range, 1200.0);
    }

    #[test]
    fn reach_ends_where_the_light_fades() {
        let l = light(&[("_light", "255 255 255 255")]);
        assert!(l.range > 1000.0 && l.range < 3000.0, "{}", l.range);
        assert!(l.color.x * l.falloff.at(l.range) < CUTOFF * 1.01);
    }

    #[test]
    fn range_ends_the_light_smoothly() {
        let l = light(&[("_light", "255 255 255 255"), ("range", "2000")]);
        assert_eq!(l.range, 2000.0);
        assert!((l.attenuation(1.0) - 1.0).abs() < 1e-3, "full brightness at the light");
        assert!((l.attenuation(1000.0) - 0.5625).abs() < 1e-9, "halfway keeps over half");
        assert_eq!(l.attenuation(2000.0), 0.0);
    }

    #[test]
    fn a_fifty_percent_distance_with_a_range_keeps_its_curve_and_ends_at_the_range() {
        let l = light(&[("_light", "255 255 255 255"), ("_fifty_percent_distance", "300"), ("range", "500")]);
        assert!((l.falloff.at(300.0) - 0.5).abs() < 1e-6);
        assert_eq!(l.attenuation(500.0), 0.0);
    }

    #[test]
    fn sun_points_up_toward_the_sky() {
        let mut e = Entity::new("light_environment");
        e.set("angles", "90 0 0");
        let (sun, _) = environment(&e, &mut Vec::new());
        let Kind::Sun { toward, .. } = sun.unwrap().kind else { panic!() };
        assert!(toward.z > 0.99, "{toward}");
    }
}
