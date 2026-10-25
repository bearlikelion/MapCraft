//! The MapCraft document model: a tree of layers, groups, entities, brushes and prefab instances.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod brush;
pub mod disp;
mod entity;
mod map;
pub mod mirror;
mod node;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub use brush::{Brush, BrushError, Face, Justify, TexAxis, TexDefaults};
pub use disp::Displacement;
pub use entity::{Connection, Entity};
pub use map::{Changes, Edit, EditError, Map, ROOT};
pub use mirror::{Axis, Mirror};
pub use node::{Cordon, Generator, Group, Layer, Node, NodeKind, Prefab, PropertyOverride, Tag};

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
