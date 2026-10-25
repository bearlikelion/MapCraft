use mapcraft_geom::{Aabb, DVec3};
use serde::{Deserialize, Serialize};

use crate::{Brush, Entity, Extra, NodeId, TagId};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub kind: NodeKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<TagId>,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NodeKind {
    /// The root, holding worldspawn's key/values.
    World(Entity),
    Layer(Layer),
    Group(Group),
    Entity(Entity),
    Brush(Brush),
    Prefab(Prefab),
}

impl NodeKind {
    pub fn can_contain(&self, child: &NodeKind) -> bool {
        use NodeKind as K;
        match self {
            K::World(_) => !matches!(child, K::World(_)),
            K::Layer(_) | K::Group(_) => !matches!(child, K::World(_) | K::Layer(_)),
            K::Entity(_) => matches!(child, K::Brush(_)),
            K::Brush(_) | K::Prefab(_) => false,
        }
    }

    pub fn entity(&self) -> Option<&Entity> {
        match self {
            NodeKind::World(e) | NodeKind::Entity(e) => Some(e),
            _ => None,
        }
    }

    pub fn brush(&self) -> Option<&Brush> {
        match self {
            NodeKind::Brush(b) => Some(b),
            _ => None,
        }
    }

    pub fn label(&self) -> String {
        match self {
            NodeKind::World(_) => "World".into(),
            NodeKind::Layer(l) => l.name.clone(),
            NodeKind::Group(g) => g.name.clone(),
            NodeKind::Entity(e) => match e.get("targetname") {
                Some(name) if !name.is_empty() => format!("{name} ({})", e.classname),
                _ => e.classname.clone(),
            },
            NodeKind::Brush(_) => "Brush".into(),
            NodeKind::Prefab(p) => p.path.clone(),
        }
    }
}

/// An exclusive container directly under the world (TrenchBroom layers).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[u8; 3]>,
    #[serde(default)]
    pub omit_from_export: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub name: String,
    /// When set, the children are produced by a plugin command and rebuilt when the params change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator: Option<Generator>,
    /// When set, the children are copied across this plane; see [`crate::mirror`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirror: Option<crate::Mirror>,
}

/// A plugin command whose output fills a group, placed by `origin` and `angles` (pitch, yaw,
/// roll in degrees) so running it again puts the new output where the group now is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Generator {
    pub plugin: String,
    pub command: String,
    pub params: serde_json::Value,
    #[serde(default)]
    pub origin: DVec3,
    #[serde(default)]
    pub angles: DVec3,
}

/// Another map instanced into this one.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Prefab {
    /// Relative to the map that contains the instance.
    pub path: String,
    #[serde(default)]
    pub origin: DVec3,
    /// Pitch, yaw, roll in degrees.
    #[serde(default)]
    pub angles: DVec3,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overrides: Vec<PropertyOverride>,
}

/// A key/value change applied to an entity inside a prefab instance; `None` removes the key.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PropertyOverride {
    pub node: NodeId,
    pub key: String,
    pub value: Option<String>,
}

/// A many-to-many label (Hammer visgroups, Godot groups); tags can nest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tag {
    pub id: TagId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[u8; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<TagId>,
    /// Hides every node carrying the tag (Hammer's visgroup checkbox).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
}

/// A box that limits what is drawn and compiled while active (Hammer's cordon).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cordon {
    pub name: String,
    pub bounds: Aabb,
    #[serde(default)]
    pub active: bool,
}
