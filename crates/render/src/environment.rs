//! Bounded environment controls supported by the viewport's forward renderer.

use mapcraft_map::Entity;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvironmentEffects {
    pub tonemap: u32,
    pub exposure: f32,
    pub white: f32,
    pub brightness: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub fog_color: [f32; 3],
    pub fog_density: f32,
    pub fog_energy: f32,
    pub fog_sun_scatter: f32,
    pub fog_sky_affect: f32,
    /// Godot's fog mode: 0 exponential, 1 depth.
    pub fog_mode: u32,
    /// Depth fog's curve and its start and end in meters; an end of 0 reaches the far plane.
    pub fog_depth: [f32; 3],
    /// Height fog's height in meters and its density per meter; a density of 0 turns it off.
    pub fog_height: [f32; 2],
    pub fog_aerial_perspective: f32,
    pub sky_curve: f32,
    pub ground_curve: f32,
    pub sky_energy: f32,
    pub ground_energy: f32,
}

impl Default for EnvironmentEffects {
    fn default() -> Self {
        Self {
            tonemap: 2,
            exposure: 1.0,
            white: 1.0,
            brightness: 1.0,
            contrast: 1.0,
            saturation: 1.0,
            fog_color: [0.0; 3],
            fog_density: 0.0,
            fog_energy: 1.0,
            fog_sun_scatter: 0.0,
            fog_sky_affect: 1.0,
            fog_mode: 0,
            fog_depth: [1.0, 10.0, 100.0],
            fog_height: [0.0, 0.0],
            fog_aerial_perspective: 0.0,
            sky_curve: 0.15,
            ground_curve: 0.02,
            sky_energy: 1.0,
            ground_energy: 1.0,
        }
    }
}

pub(crate) fn bounded(e: &Entity, key: &str, default: f32, min: f32, max: f32, warnings: &mut Vec<String>) -> f32 {
    let Some(raw) = e.get(key) else { return default };
    match raw.trim().parse::<f32>() {
        Ok(value) if value.is_finite() && (min..=max).contains(&value) => value,
        Ok(value) if value.is_finite() => {
            let clamped = value.clamp(min, max);
            warnings.push(format!("light_environment: {key} {value} is outside {min} to {max}; using {clamped}"));
            clamped
        }
        _ => {
            warnings.push(format!("light_environment: {key} must be finite and between {min} and {max}; using {default}"));
            default
        }
    }
}

fn enabled(e: &Entity, key: &str, default: bool) -> bool {
    let Some(raw) = e.get(key) else { return default };
    let raw = raw.trim().to_ascii_lowercase();
    match raw.as_str() {
        "-1" | "auto" => default,
        "0" | "false" | "off" | "" => false,
        _ => raw.parse::<f64>().map_or(true, |v| if v.is_finite() { v != 0.0 } else { default }),
    }
}

impl EnvironmentEffects {
    pub(crate) fn read(e: &Entity, horizon: [f32; 3]) -> (Self, Vec<String>) {
        let mut warnings = Vec::new();
        let mut value = Self::default();
        let mut scalar = |key, default, min, max| bounded(e, key, default, min, max, &mut warnings);
        value.exposure = scalar("tonemap_exposure", 1.0, 0.0, 64.0);
        value.white = scalar("tonemap_white", 1.0, 0.001, 64.0);
        value.sky_curve = scalar("sky_curve", 0.15, 0.001, 1.0);
        value.ground_curve = scalar("ground_curve", 0.02, 0.001, 1.0);
        value.sky_energy = scalar("sky_energy_multiplier", 1.0, 0.0, 64.0);
        value.ground_energy = scalar("ground_energy_multiplier", 1.0, 0.0, 64.0);
        if enabled(e, "adjustment_enabled", false) {
            value.brightness = scalar("adjustment_brightness", 1.0, 0.0, 8.0);
            value.contrast = scalar("adjustment_contrast", 1.0, 0.0, 8.0);
            value.saturation = scalar("adjustment_saturation", 1.0, 0.0, 8.0);
        }
        value.fog_density = scalar("fog_density", 0.0, 0.0, 1.0);
        value.fog_height = [scalar("fog_height", 0.0, -1024.0, 1024.0), scalar("fog_height_density", 0.0, -16.0, 16.0)];
        if !enabled(e, "fog_enabled", value.fog_density > 0.0 || value.fog_height[1] != 0.0) {
            value.fog_density = 0.0;
            value.fog_height[1] = 0.0;
        }
        value.fog_depth =
            [scalar("fog_depth_curve", 1.0, 0.00001, 16.0), scalar("fog_depth_begin", 10.0, 0.0, 4000.0), scalar("fog_depth_end", 100.0, 0.0, 4000.0)];
        value.fog_aerial_perspective = scalar("fog_aerial_perspective", 0.0, 0.0, 1.0);
        value.fog_energy = scalar("fog_light_energy", 1.0, 0.0, 64.0);
        value.fog_sun_scatter = scalar("fog_sun_scatter", 0.0, 0.0, 1.0);
        value.fog_sky_affect = scalar("fog_sky_affect", 1.0, 0.0, 1.0);
        value.fog_color = horizon;
        if let Some(raw) = e.get("fog_color") {
            let parts: Vec<_> = raw.split_whitespace().take(4).map(str::parse::<f32>).collect();
            if let [Ok(r), Ok(g), Ok(b)] = parts.as_slice()
                && [r, g, b].into_iter().all(|v| v.is_finite() && (0.0..=255.0).contains(v))
            {
                value.fog_color = [*r, *g, *b].map(|v| (v / 255.0).powf(2.2));
            } else {
                warnings.push("light_environment: fog_color must be three components from 0 to 255; using horizon color".into());
            }
        }
        let mode = e.get("fog_mode").map(|m| m.trim().to_ascii_lowercase());
        value.fog_mode = match mode.as_deref() {
            None | Some("exponential") => 0,
            Some("depth") => 1,
            Some(n) if n.parse::<f64>() == Ok(0.0) => 0,
            Some(n) if n.parse::<f64>() == Ok(1.0) => 1,
            Some(other) => {
                warnings.push(format!("light_environment: fog_mode {other:?} is unsupported; using exponential"));
                0
            }
        };
        value.tonemap = match e.get("tonemap_mode").map(str::trim) {
            None | Some("2" | "filmic") => 2,
            Some("0" | "linear") => 0,
            Some("1" | "reinhardt") => 1,
            Some("3" | "aces") => 3,
            Some(other) => {
                warnings.push(format!("light_environment: tonemap_mode {other:?} is unsupported in the viewport; using filmic"));
                2
            }
        };
        for (key, reason) in [
            ("ssao_enabled", "requires a depth/normal occlusion pass"),
            ("ssil_enabled", "requires screen-space indirect lighting and temporal history"),
            ("ssr_enabled", "requires screen-space reflection tracing"),
            ("sdfgi_enabled", "requires a signed-distance-field GI volume"),
            ("volumetric_fog_enabled", "requires a volumetric lighting grid"),
            ("adjustment_color_correction", "requires a color-grading LUT"),
            ("auto_exposure_enabled", "requires luminance history"),
        ] {
            if enabled(e, key, false) {
                warnings.push(format!("light_environment: {key} is not rendered in the viewport ({reason})"));
            }
        }
        (value, warnings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_controls_are_bounded_and_reported() {
        let mut e = Entity::default();
        for (key, value) in [
            ("tonemap_exposure", "NaN"),
            ("tonemap_white", "0"),
            ("fog_density", "inf"),
            ("sky_curve", "-1"),
            ("fog_color", "1 NaN 3"),
            ("ssao_enabled", "1"),
            ("tonemap_mode", "agx"),
        ] {
            e.set(key, value);
        }
        let (effects, warnings) = EnvironmentEffects::read(&e, [0.5; 3]);
        assert_eq!(effects.exposure, 1.0);
        assert_eq!(effects.fog_density, 0.0);
        assert_eq!(effects.fog_color, [0.5; 3]);
        assert_eq!(warnings.len(), 7);
    }

    #[test]
    fn out_of_range_values_clamp_with_a_warning() {
        let mut e = Entity::default();
        e.set("fog_density", "100");
        e.set("fog_sky_affect", "-2");
        let (effects, warnings) = EnvironmentEffects::read(&e, [0.5; 3]);
        assert_eq!(effects.fog_density, 1.0);
        assert_eq!(effects.fog_sky_affect, 0.0);
        assert_eq!(warnings.len(), 2);
    }

    #[test]
    fn depth_height_and_aerial_fog_match_godot_keys() {
        let mut e = Entity::default();
        e.set("fog_mode", "depth");
        e.set("fog_depth_curve", "2");
        e.set("fog_depth_begin", "5");
        e.set("fog_depth_end", "50");
        e.set("fog_height", "-3");
        e.set("fog_height_density", "0.5");
        e.set("fog_aerial_perspective", "0.25");
        let (effects, warnings) = EnvironmentEffects::read(&e, [0.5; 3]);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!((effects.fog_mode, effects.fog_depth, effects.fog_height, effects.fog_aerial_perspective), (1, [2.0, 5.0, 50.0], [-3.0, 0.5], 0.25));
        e.set("fog_enabled", "0");
        assert_eq!(EnvironmentEffects::read(&e, [0.5; 3]).0.fog_height[1], 0.0, "turning fog off turns height fog off too");
        e.set("fog_mode", "volumetric");
        assert_eq!(EnvironmentEffects::read(&e, [0.5; 3]).0.fog_mode, 0);
    }

    #[test]
    fn controls_and_disabled_effects_preserve_defaults() {
        let mut e = Entity::default();
        e.set("fog_density", "0.02");
        e.set("fog_enabled", "0");
        e.set("tonemap_mode", "reinhardt");
        e.set("fog_height_density", "0.0");
        e.set("fog_aerial_perspective", "0.00");
        e.set("fog_mode", "0.0");
        e.set("adjustment_enabled", "1");
        e.set("adjustment_saturation", "0");
        let (effects, warnings) = EnvironmentEffects::read(&e, [0.5; 3]);
        assert_eq!(effects.tonemap, 1);
        assert_eq!(effects.fog_density, 0.0);
        assert_eq!(effects.saturation, 0.0);
        assert!(warnings.is_empty());
    }
}
