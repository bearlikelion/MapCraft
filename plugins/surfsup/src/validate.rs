//! SurfsUp's own map checks, listed by Check for Problems.

use std::collections::BTreeMap;

use glam::DVec3;

use crate::mapcraft::plugin::types::{Issue, MapData, MapNodeKind, Severity};

/// Zones a player must touch to time a run.
const TIMED_ZONES: [&str; 4] = ["zone_start", "zone_finish", "zone_checkpoint", "zone_split"];
/// Shorter zones can be jumped over.
const MIN_ZONE_HEIGHT: f64 = 256.0;
/// Thinner zones can be crossed between two physics ticks at surf speeds.
const MIN_ZONE_THICKNESS: f64 = 64.0;

struct Ent {
    node: u32,
    class: String,
    properties: Vec<(String, String)>,
    has_brushes: bool,
    bounds: Option<(DVec3, DVec3)>,
}

impl Ent {
    fn property(&self, key: &str) -> Option<&str> {
        self.properties.iter().rev().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.trim()).filter(|v| !v.is_empty())
    }

    fn course(&self) -> i64 {
        self.property("course_id").and_then(|v| v.parse::<f64>().ok()).filter(|v| v.is_finite()).map_or(0, |v| v.round() as i64)
    }

    fn origin(&self) -> Option<DVec3> {
        let v: Vec<f64> = self.property("origin")?.split_whitespace().filter_map(|n| n.parse().ok()).collect();
        let p = DVec3::new(*v.first()?, *v.get(1)?, *v.get(2)?);
        p.is_finite().then_some(p)
    }
}

fn entities(map: &MapData) -> Vec<Ent> {
    let mut out: Vec<Ent> = Vec::new();
    let mut at: BTreeMap<usize, usize> = BTreeMap::new();
    for (i, node) in map.nodes.iter().enumerate() {
        match &node.kind {
            MapNodeKind::Entity(e) => {
                at.insert(i, out.len());
                let node = u32::try_from(i).unwrap_or(u32::MAX);
                out.push(Ent { node, class: e.classname.to_ascii_lowercase(), properties: e.properties.clone(), has_brushes: false, bounds: None });
            }
            MapNodeKind::Brush(solid) => {
                let Some(owner) = node.parent.and_then(|p| at.get(&(p as usize))).and_then(|&j| out.get_mut(j)) else { continue };
                owner.has_brushes = true;
                for v in &solid.vertices {
                    let p = DVec3::new(v.x, v.y, v.z);
                    if p.is_finite() {
                        owner.bounds = Some(owner.bounds.map_or((p, p), |(lo, hi)| (lo.min(p), hi.max(p))));
                    }
                }
            }
            MapNodeKind::Layer(_) | MapNodeKind::Group(_) => {}
        }
    }
    out
}

fn issue(severity: Severity, message: String, node: Option<u32>) -> Issue {
    Issue { severity, message, node }
}

/// Every problem SurfsUp would have with this map.
pub fn check(map: &MapData) -> Vec<Issue> {
    let ents = entities(map);
    let mut out = Vec::new();
    let of = |class: &'static str| ents.iter().filter(move |e| e.class == class);

    let spawns: Vec<&Ent> = of("info_player_start").collect();
    match spawns.as_slice() {
        [] => out.push(issue(Severity::Error, "There is no info_player_start, so players have nowhere to spawn.".into(), None)),
        [_, extra @ ..] => {
            for e in extra {
                out.push(issue(
                    Severity::Warning,
                    "There is more than one info_player_start; SurfsUp spawns players at only one of them.".into(),
                    Some(e.node),
                ));
            }
        }
    }

    let main_start = of("zone_start").find(|e| e.course() == 0);
    if main_start.is_none() {
        out.push(issue(Severity::Error, "There is no zone_start on course 0, so the run timer never starts.".into(), None));
    }
    if !of("zone_finish").any(|e| e.course() == 0) {
        out.push(issue(Severity::Error, "There is no zone_finish on course 0, so the run timer never stops.".into(), None));
    }

    for e in ents.iter().filter(|e| TIMED_ZONES.contains(&e.class.as_str())) {
        let Some((lo, hi)) = e.bounds.filter(|_| e.has_brushes) else {
            out.push(issue(Severity::Error, format!("This {} has no brush, so it covers nothing.", e.class), Some(e.node)));
            continue;
        };
        let size = hi - lo;
        if size.z < MIN_ZONE_HEIGHT {
            out.push(issue(
                Severity::Warning,
                format!("This {} is only {:.0} units tall; players can jump over zones under {MIN_ZONE_HEIGHT:.0}.", e.class, size.z),
                Some(e.node),
            ));
        }
        let thin = size.x.min(size.y);
        if thin < MIN_ZONE_THICKNESS {
            out.push(issue(
                Severity::Warning,
                format!("This {} is only {thin:.0} units thick; fast players can cross zones under {MIN_ZONE_THICKNESS:.0} between ticks.", e.class),
                Some(e.node),
            ));
        }
    }

    let mut destinations: BTreeMap<&str, usize> = BTreeMap::new();
    for e in of("teleport_destination") {
        if let Some(name) = e.property("targetname") {
            *destinations.entry(name).or_default() += 1;
        }
    }
    for e in of("teleport_destination") {
        if let Some(name) = e.property("targetname").filter(|n| destinations.get(n).is_some_and(|c| *c > 1)) {
            out.push(issue(Severity::Warning, format!("More than one teleport_destination is named {name}; teleports pick one of them."), Some(e.node)));
        }
    }
    for e in of("teleport_zone") {
        match e.property("next_stage") {
            None => out.push(issue(Severity::Error, "This teleport_zone has no Destination, so it sends players nowhere.".into(), Some(e.node))),
            Some(name) if !destinations.contains_key(name) => {
                out.push(issue(
                    Severity::Error,
                    format!("This teleport_zone sends players to {name}, but no teleport_destination has that name."),
                    Some(e.node),
                ));
            }
            Some(_) => {}
        }
    }

    if let (Some(spawn), Some((lo, hi))) = (spawns.first(), main_start.and_then(|s| s.bounds)) {
        let margin = DVec3::splat(1.0);
        if let Some(p) = spawn.origin().filter(|p| p.cmplt(lo - margin).any() || p.cmpgt(hi + margin).any()) {
            out.push(issue(Severity::Warning, format!("The player start at {:.0} {:.0} {:.0} is outside the start zone.", p.x, p.y, p.z), Some(spawn.node)));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapcraft::plugin::types::{MapEntity, MapNode, Solid, Vec3};

    #[derive(Default)]
    struct Builder(Vec<MapNode>);

    impl Builder {
        fn entity(&mut self, class: &str, props: &[(&str, &str)], bounds: Option<(DVec3, DVec3)>) {
            let at = self.0.len() as u32;
            let properties = props.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            self.0.push(MapNode { parent: None, kind: MapNodeKind::Entity(MapEntity { classname: class.into(), properties }) });
            if let Some((lo, hi)) = bounds {
                let vertices = [lo, hi].iter().map(|p| Vec3 { x: p.x, y: p.y, z: p.z }).collect();
                self.0.push(MapNode { parent: Some(at), kind: MapNodeKind::Brush(Solid { faces: Vec::new(), vertices }) });
            }
        }

        fn map(self) -> MapData {
            MapData { name: "surf_test".into(), world: Vec::new(), nodes: self.0 }
        }
    }

    fn b(x: f64, h: f64) -> Option<(DVec3, DVec3)> {
        Some((DVec3::new(x, 0.0, 0.0), DVec3::new(x + 512.0, 512.0, h)))
    }

    fn messages(map: &MapData) -> Vec<String> {
        check(map).into_iter().map(|i| i.message).collect()
    }

    #[test]
    fn a_complete_course_passes() {
        let mut m = Builder::default();
        m.entity("info_player_start", &[("origin", "256 256 16")], None);
        m.entity("zone_start", &[], b(0.0, 256.0));
        m.entity("zone_finish", &[], b(4096.0, 256.0));
        m.entity("teleport_zone", &[("next_stage", "stage2")], b(8192.0, 512.0));
        m.entity("teleport_destination", &[("targetname", "stage2"), ("origin", "0 0 0")], None);
        assert!(check(&m.map()).is_empty());
    }

    #[test]
    fn missing_and_broken_pieces_are_reported() {
        let mut m = Builder::default();
        m.entity("zone_finish", &[("course_id", "1")], b(4096.0, 128.0));
        m.entity("zone_checkpoint", &[], None);
        m.entity("teleport_zone", &[("next_stage", "nowhere")], b(8192.0, 512.0));
        m.entity("teleport_zone", &[], b(9000.0, 512.0));
        let map = m.map();
        let found = messages(&map);
        for expected in [
            "no info_player_start",
            "no zone_start on course 0",
            "no zone_finish on course 0",
            "has no brush",
            "only 128 units tall",
            "sends players to nowhere",
            "has no Destination",
        ] {
            assert!(found.iter().any(|f| f.contains(expected)), "missing {expected:?} in {found:?}");
        }
        let errors = check(&map).iter().filter(|i| matches!(i.severity, Severity::Error)).count();
        assert_eq!(errors, 6);
    }

    #[test]
    fn spawns_and_destinations_are_checked_against_each_other() {
        let mut m = Builder::default();
        m.entity("info_player_start", &[("origin", "9000 9000 0")], None);
        m.entity("info_player_start", &[("origin", "0 0 0")], None);
        m.entity("zone_start", &[], b(0.0, 256.0));
        m.entity("zone_finish", &[], b(4096.0, 256.0));
        m.entity("teleport_destination", &[("targetname", "a")], None);
        m.entity("teleport_destination", &[("targetname", "a")], None);
        let issues = check(&m.map());
        assert!(issues.iter().any(|i| i.message.contains("more than one info_player_start") && i.node == Some(1)));
        assert!(issues.iter().any(|i| i.message.contains("outside the start zone") && i.node == Some(0)));
        assert_eq!(issues.iter().filter(|i| i.message.contains("named a")).count(), 2);
    }
}
