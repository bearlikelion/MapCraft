//! Data-only PBR material descriptors for version 2 asset packs.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const MAX_DESCRIPTOR_SIZE: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextureSlot {
    Albedo,
    Normal,
    Roughness,
    Metallic,
    Emission,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Material {
    pub version: u32,
    pub albedo: [f32; 4],
    pub roughness: f32,
    pub metallic: f32,
    pub emission: [f32; 3],
    pub textures: BTreeMap<TextureSlot, String>,
}

impl Default for Material {
    fn default() -> Self {
        Self { version: 1, albedo: [1.0; 4], roughness: 1.0, metallic: 0.0, emission: [0.0; 3], textures: BTreeMap::new() }
    }
}

impl Material {
    pub fn read(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_DESCRIPTOR_SIZE {
            return Err("material descriptor exceeds 16 KiB".into());
        }
        let material: Self = serde_json::from_slice(bytes).map_err(|e| format!("invalid material descriptor: {e}"))?;
        material.validate()?;
        Ok(material)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err(format!("unsupported material version {}", self.version));
        }
        let unit = |v: &f32| v.is_finite() && (0.0..=1.0).contains(v);
        if !self.albedo.iter().all(unit) || !unit(&self.roughness) || !unit(&self.metallic) {
            return Err("albedo, roughness and metallic must be finite values from 0 to 1".into());
        }
        if !self.emission.iter().all(|v| v.is_finite() && (0.0..=64.0).contains(v)) {
            return Err("emission must be finite values from 0 to 64".into());
        }
        for name in self.textures.values() {
            if !name.starts_with("textures/") || !(name.ends_with(".png") || name.ends_with(".jpg")) {
                return Err("material textures must name PNG or JPEG entries under textures/".into());
            }
            crate::pack::validate_name(name)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_round_trip_and_hostile_values() {
        let mut material = Material::default();
        material.textures.insert(TextureSlot::Normal, "textures/stone_normal.png".into());
        assert_eq!(Material::read(&serde_json::to_vec(&material).unwrap()).unwrap(), material);
        material.roughness = f32::NAN;
        assert!(material.validate().is_err());
        material.roughness = 1.0;
        material.textures.insert(TextureSlot::Albedo, "textures/../outside.png".into());
        assert!(material.validate().is_err());
        assert!(Material::read(&vec![b' '; MAX_DESCRIPTOR_SIZE + 1]).is_err());
        assert!(Material::read(br#"{"version":1,"script":"res://bad.gd"}"#).is_err());
    }
}
