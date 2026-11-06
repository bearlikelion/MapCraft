//! Where each entity goes in the imported tree, and the one spawn point, as in SourceGD's `entity` and SurfsUp's BSP importer.

use crate::key_values::key;
use crate::mapcraft::plugin::types::Entity;

/// Counter-Strike's spawns, all of which give way to one spawn point.
const SPAWN_CLASSNAMES: [&str; 4] = ["info_player_terrorist", "info_player_counterterrorist", "info_player_start", "info_player_deathmatch"];

/// Teleport destination names that mark the map's start, most specific first.
const MAP_START_NAMES: [&str; 17] = [
    "td_mapstart",
    "td_start",
    "map_spawn",
    "mapspawn",
    "mapstart",
    "map_start",
    "mainstart",
    "main_start",
    "s1_start",
    "main dest",
    "main",
    "start",
    "spawn",
    "spawn_guy",
    "spawn_hub",
    "s1",
    "stage1",
];

/// Destination names holding these are bonus or stage starts, not the map's.
const NON_START_TOKENS: [&str; 16] = ["bonus", "stage", "end", "hub", "secret", "filter", "cp", "b1", "b2", "b3", "b4", "b5", "b6", "b7", "b8", "fade"];

/// Entities that set up the sky, sun, fog and lighting, which SourceGD builds into `SourceEnvironment`.
const ENVIRONMENT: [&str; 5] = ["env_sun", "env_fog_controller", "env_tonemap_controller", "shadow_control", "env_skypaint"];

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Section {
    Entities,
    Props,
    Environment,
}

pub(crate) fn section(classname: &str) -> Section {
    let c = classname.to_ascii_lowercase();
    if c.starts_with("prop_") {
        Section::Props
    } else if c.starts_with("light") || ENVIRONMENT.contains(&c.as_str()) {
        Section::Environment
    } else {
        Section::Entities
    }
}

pub(crate) fn is_spawn(classname: &str) -> bool {
    SPAWN_CLASSNAMES.iter().any(|s| s.eq_ignore_ascii_case(classname))
}

fn name(e: &Entity) -> String {
    key(&e.properties, "targetname").unwrap_or_default().to_ascii_lowercase()
}

/// The map's start: a teleport destination named like one, else the first Counter-Strike spawn.
pub(crate) fn spawn_of(entities: &[Entity]) -> Option<&Entity> {
    let destinations: Vec<&Entity> = entities.iter().filter(|e| e.classname.eq_ignore_ascii_case("info_teleport_destination")).collect();
    let named = MAP_START_NAMES.iter().find_map(|wanted| destinations.iter().find(|e| name(e) == *wanted));
    let fuzzy = || {
        ["start", "spawn"].iter().find_map(|token| {
            destinations.iter().find(|e| {
                let n = name(e);
                n.contains(token) && !NON_START_TOKENS.iter().any(|t| n.contains(t))
            })
        })
    };
    named.or_else(fuzzy).copied().or_else(|| entities.iter().find(|e| is_spawn(&e.classname)))
}

/// One `info_player_start` where `at` stands, facing its yaw.
pub(crate) fn spawn_point(at: &Entity) -> Entity {
    let yaw = key(&at.properties, "angles").and_then(|a| a.split_whitespace().nth(1)).unwrap_or("0");
    Entity {
        classname: "info_player_start".into(),
        properties: vec![("origin".into(), key(&at.properties, "origin").unwrap_or("0 0 0").into()), ("angles".into(), format!("0 {yaw} 0"))],
        connections: Vec::new(),
        brushes: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(classname: &str, keys: &[(&str, &str)]) -> Entity {
        Entity {
            classname: classname.into(),
            properties: keys.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            connections: Vec::new(),
            brushes: Vec::new(),
        }
    }

    #[test]
    fn picks_one_spawn() {
        let ct = entity("info_player_counterterrorist", &[("origin", "1 2 3"), ("angles", "0 90 0")]);
        let bonus = entity("info_teleport_destination", &[("targetname", "bonus_start"), ("origin", "9 9 9")]);
        let start = entity("info_teleport_destination", &[("targetname", "s1_start"), ("origin", "4 5 6")]);
        let mapstart = entity("info_teleport_destination", &[("targetname", "TD_MapStart"), ("origin", "7 8 9")]);
        let origin = |list: &[Entity]| spawn_of(list).and_then(|e| key(&e.properties, "origin")).map(str::to_string);
        assert_eq!(origin(&[ct.clone(), bonus.clone()]).as_deref(), Some("1 2 3"), "bonus starts are not the map's");
        assert_eq!(origin(&[ct.clone(), bonus, start.clone(), mapstart]).as_deref(), Some("7 8 9"));
        assert_eq!(origin(&[ct.clone(), start]).as_deref(), Some("4 5 6"));
        let point = spawn_point(&ct);
        assert_eq!(
            (point.classname.as_str(), key(&point.properties, "origin"), key(&point.properties, "angles")),
            ("info_player_start", Some("1 2 3"), Some("0 90 0"))
        );
    }
}
