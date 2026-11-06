//! Rules for converting a map made for another SDK into this one, read from `convert/<source sdk>.toml`.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::{Sdk, SdkError, valid_id};

/// How the entities and materials of one SDK's maps become this SDK's.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Conversion {
    /// Rules by source classname; classes not listed are kept as they are.
    #[serde(default)]
    pub classes: BTreeMap<String, ClassRule>,
    /// Source material names and what they become.
    #[serde(default)]
    pub materials: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClassRule {
    /// The classname it becomes; unset keeps the class.
    #[serde(default)]
    pub to: Option<String>,
    /// Keys to rename, old name to new name.
    #[serde(default)]
    pub keys: BTreeMap<String, String>,
    /// Keys to set, added or overwritten after renaming.
    #[serde(default)]
    pub set: BTreeMap<String, String>,
    /// Keys to remove.
    #[serde(default)]
    pub remove: Vec<String>,
    /// Its brushes join the world and the entity goes.
    #[serde(default)]
    pub world: bool,
    /// The entity goes, brushes and all.
    #[serde(default)]
    pub drop: bool,
    /// Only the first entity converted to its class is kept, as for a game with one spawn point.
    #[serde(default)]
    pub single: bool,
}

impl Conversion {
    pub fn parse(text: &str, path: &Path) -> Result<Self, SdkError> {
        toml::from_str(text).map_err(|source| SdkError::Toml { path: path.to_path_buf(), source })
    }
}

impl Sdk {
    /// The rules for maps made for the SDK `source`, when this SDK ships them.
    pub fn conversion_from(&self, source: &str) -> Option<Result<Conversion, SdkError>> {
        if !valid_id(source) || source == self.manifest.id {
            return None;
        }
        let path = self.dir.join("convert").join(format!("{source}.toml"));
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(source) => return Some(Err(SdkError::Io { path, source })),
        };
        Some(Conversion::parse(&text, &path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_rules_and_refuses_unknown_keys() {
        let text = "[classes.trigger_teleport]\nto = \"teleport_zone\"\nkeys = { target = \"next_stage\" }\n\n[classes.func_wall]\nworld = true\n\n[materials]\n\"tools/toolsclip\" = \"tools/clip\"\n";
        let c = Conversion::parse(text, Path::new("x.toml")).unwrap();
        let tp = &c.classes["trigger_teleport"];
        assert_eq!((tp.to.as_deref(), tp.keys.get("target").map(String::as_str)), (Some("teleport_zone"), Some("next_stage")));
        assert!(c.classes["func_wall"].world);
        assert_eq!(c.materials.get("tools/toolsclip").map(String::as_str), Some("tools/clip"));
        assert!(Conversion::parse("[classes.a]\nrename = \"b\"\n", Path::new("x.toml")).is_err(), "a typo is an error, not a silent no-op");
    }

    #[test]
    fn finds_rules_beside_the_manifest() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("sdk.toml"), "id = \"surfsup\"\nname = \"SurfsUp\"\n").unwrap();
        std::fs::create_dir(dir.path().join("convert")).unwrap();
        std::fs::write(dir.path().join("convert/cstrike.toml"), "[classes.func_wall]\nworld = true\n").unwrap();
        let sdk = Sdk::load(dir.path()).unwrap();
        assert!(sdk.conversion_from("cstrike").is_some_and(|c| c.is_ok_and(|c| c.classes.contains_key("func_wall"))));
        assert!(sdk.conversion_from("quake").is_none());
        assert!(sdk.conversion_from("surfsup").is_none(), "no rules from an SDK to itself");
        assert!(sdk.conversion_from("../etc").is_none());
    }
}
