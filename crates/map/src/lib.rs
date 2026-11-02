//! The MapCraft document model: a tree of layers, groups, entities, brushes and prefab instances.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod boolean;
mod brush;
mod cover;
pub mod curve;
pub mod disp;
mod entity;
pub mod link;
mod map;
pub mod mesh_edit;
pub mod mirror;
mod node;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub use boolean::Operation;
pub use brush::{
    ANTI_TILE, AntiTile, Bevel, Brush, BrushError, CONTENT_FLAGS, Face, Justify, TexAxis, TexDefaults, blend_layer, is_tool_material, share_edge, wrap_runs,
};
pub use cover::covered_faces;
pub use curve::{CurveOp, Path, PathPoint};
pub use disp::{BlendRole, Displacement};
pub use entity::{Connection, Entity, parse_vec3};
pub use link::Link;
pub use map::{Changes, Edit, EditError, Map, ROOT};
pub use mirror::{Axis, Mirror};
pub use node::{Cordon, Generator, Group, Layer, Node, NodeKind, Prefab, PropertyOverride, Tag};

/// The longest embedded shader source an edit accepts, in bytes.
pub const MAX_SHADER_BYTES: usize = 256 * 1024;

/// Uniform values by name: numbers, bools, arrays of numbers for vectors, and material names for textures.
pub type ShaderParams = BTreeMap<String, serde_json::Value>;

pub const MAX_SHADER_PARAMS: usize = 256;

/// Whether `name` and `value` fit the bounds a shader parameter must keep; anything else is dropped on load and refused on edit.
pub fn valid_shader_param(name: &str, value: &serde_json::Value) -> bool {
    use serde_json::Value;
    let finite = |v: &Value| v.as_f64().is_some_and(f64::is_finite);
    !name.is_empty()
        && name.len() <= 256
        && match value {
            Value::Bool(_) => true,
            Value::Number(_) => finite(value),
            Value::String(s) => s.len() <= 1024,
            Value::Array(a) => !a.is_empty() && a.len() <= 16 && a.iter().all(finite),
            Value::Null | Value::Object(_) => false,
        }
}

/// Data a plugin needs to round-trip but the core does not model.
pub type Extra = BTreeMap<String, serde_json::Value>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(pub u64);

/// Stable face id; 0 means "not assigned yet" and is replaced when the face enters a map.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FaceId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TagId(pub u32);
