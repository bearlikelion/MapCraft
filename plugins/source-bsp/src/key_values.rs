//! Entity lump keyvalues and the vectors and angles they hold, as in SourceGD's `core/key_values`.

use glam::{DMat3, DVec3};

/// The keyvalues of each entity in the entity lump, in file order; quotes never occur inside values.
pub(crate) fn parse_entities(text: &str) -> Vec<Vec<(String, String)>> {
    let mut out = Vec::new();
    let mut entity: Option<Vec<(String, String)>> = None;
    let mut key: Option<&str> = None;
    for (i, part) in text.split('"').enumerate() {
        if i % 2 == 1 {
            if let Some(keys) = entity.as_mut() {
                match key.take() {
                    Some(k) => keys.push((k.to_string(), part.to_string())),
                    None => key = Some(part),
                }
            }
            continue;
        }
        for c in part.chars().filter(|c| matches!(c, '{' | '}')) {
            out.extend(entity.take().filter(|keys| !keys.is_empty()));
            key = None;
            if c == '{' {
                entity = Some(Vec::new());
            }
        }
    }
    out.extend(entity.filter(|keys| !keys.is_empty()));
    out
}

/// The last value of `name`, which is the one the engine keeps.
pub(crate) fn key<'k>(keys: &'k [(String, String)], name: &str) -> Option<&'k str> {
    keys.iter().rev().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
}

pub(crate) fn vector(text: Option<&str>) -> DVec3 {
    let mut parts = text.unwrap_or_default().split_whitespace().map(|s| s.parse::<f64>().ok().filter(|v| v.is_finite()).unwrap_or(0.0));
    DVec3::new(parts.next().unwrap_or(0.0), parts.next().unwrap_or(0.0), parts.next().unwrap_or(0.0))
}

/// Source's pitch, yaw, roll in degrees as a rotation matrix (AngleMatrix).
pub(crate) fn rotation(angles: DVec3) -> DMat3 {
    let [pitch, yaw, roll] = angles.to_array().map(f64::to_radians);
    DMat3::from_rotation_z(yaw) * DMat3::from_rotation_y(pitch) * DMat3::from_rotation_x(roll)
}

pub(crate) fn number(v: f64) -> String {
    format!("{}", v as f32 + 0.0)
}
