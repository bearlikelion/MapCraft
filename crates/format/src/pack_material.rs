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
    /// Version 2: a height map for parallax, white high unless `height_flip`.
    Height,
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
    /// Version 2: multiplies the emission color plus the emission texture.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub emission_energy: f32,
    /// Version 2: how strongly the normal map bends the light.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub normal_strength: f32,
    /// Version 2: parallax depth of the height map.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub height_scale: f32,
    /// Version 2: the height map is a depth map, white low.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub height_flip: bool,
}

fn one() -> f32 {
    1.0
}

fn is_one(v: &f32) -> bool {
    *v == 1.0
}

impl Default for Material {
    fn default() -> Self {
        Self {
            version: 1,
            albedo: [1.0; 4],
            roughness: 1.0,
            metallic: 0.0,
            emission: [0.0; 3],
            textures: BTreeMap::new(),
            emission_energy: 1.0,
            normal_strength: 1.0,
            height_scale: 1.0,
            height_flip: false,
        }
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

    /// Version 1 unless it uses a version 2 field, so older Godot addons still read plain descriptors.
    pub fn needed_version(&self) -> u32 {
        let d = Self::default();
        let plain = (self.emission_energy, self.normal_strength, self.height_scale, self.height_flip)
            == (d.emission_energy, d.normal_strength, d.height_scale, d.height_flip);
        if plain && !self.textures.contains_key(&TextureSlot::Height) { 1 } else { 2 }
    }

    /// Pretty JSON at the version it needs.
    pub fn to_json(&self) -> Result<Vec<u8>, String> {
        let material = Self { version: self.needed_version(), ..self.clone() };
        material.validate()?;
        serde_json::to_vec_pretty(&material).map_err(|e| e.to_string())
    }

    pub fn validate(&self) -> Result<(), String> {
        if !(1..=2).contains(&self.version) {
            return Err(format!("unsupported material version {}", self.version));
        }
        if self.version < self.needed_version() {
            return Err("emission energy, normal strength, height maps and their settings need material version 2".into());
        }
        let ranged = |v: f32, lo: f32, hi: f32| v.is_finite() && (lo..=hi).contains(&v);
        if !ranged(self.emission_energy, 0.0, 64.0) || !ranged(self.normal_strength, 0.0, 16.0) || !ranged(self.height_scale, -16.0, 16.0) {
            return Err("emission energy must be 0 to 64, normal strength 0 to 16 and height scale -16 to 16".into());
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

    #[test]
    fn version_two_only_when_needed() {
        let mut material = Material::default();
        material.textures.insert(TextureSlot::Emission, "textures/glow.png".into());
        let plain: serde_json::Value = serde_json::from_slice(&material.to_json().unwrap()).unwrap();
        assert_eq!((plain["version"].as_u64(), plain.as_object().map(|o| o.len())), (Some(1), Some(6)), "plain descriptors keep the six version 1 fields");
        material.emission_energy = 4.0;
        material.normal_strength = 0.25;
        material.height_flip = true;
        let bytes = material.to_json().unwrap();
        let read = Material::read(&bytes).unwrap();
        assert_eq!((read.version, read.emission_energy, read.normal_strength, read.height_flip), (2, 4.0, 0.25, true));
        assert!(Material { version: 1, ..read.clone() }.validate().unwrap_err().contains("version 2"));
        let mut height = Material::default();
        height.textures.insert(TextureSlot::Height, "textures/h.png".into());
        assert_eq!(height.needed_version(), 2);
        assert!(Material { height_scale: 17.0, ..read.clone() }.validate().is_err());
        assert!(Material { normal_strength: f32::NAN, ..read }.validate().is_err());
    }
}
