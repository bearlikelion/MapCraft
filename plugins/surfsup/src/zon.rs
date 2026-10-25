//! Writes SurfsUp v3's `.zon` zones, SurfTimer KeyValues read by `Scenes/BSP/bsp_zone_data.gd`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use glam::DVec3;

use crate::mapcraft::plugin::types::{MapData, MapNodeKind};

/// v3 stops reading stages and checkpoints past this number.
const MAX_STAGE: usize = 80;

/// What a zone does, which picks its block name and SurfTimer effect flags.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Start,
    End,
    Stage,
    Checkpoint,
}

struct Zone {
    name: String,
    kind: Kind,
    bonus: usize,
    number: usize,
    min: DVec3,
    max: DVec3,
}

/// The file and the lines to log about it.
pub struct Written {
    pub text: String,
    pub log: Vec<String>,
}

/// A zone entity found in the map, in document order.
struct Found {
    classname: String,
    properties: Vec<(String, String)>,
    bounds: Option<(DVec3, DVec3)>,
}

fn property<'a>(f: &'a Found, key: &str) -> Option<&'a str> {
    f.properties.iter().rev().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str())
}

fn integer(f: &Found, key: &str) -> Option<i64> {
    property(f, key)?.trim().parse::<f64>().ok().filter(|v| v.is_finite()).map(|v| v.round() as i64)
}

fn found(map: &MapData) -> Vec<Found> {
    let mut out: Vec<Found> = Vec::new();
    let mut entity_at: BTreeMap<usize, usize> = BTreeMap::new();
    for (i, node) in map.nodes.iter().enumerate() {
        match &node.kind {
            MapNodeKind::Entity(e) => {
                entity_at.insert(i, out.len());
                out.push(Found { classname: e.classname.to_ascii_lowercase(), properties: e.properties.clone(), bounds: None });
            }
            MapNodeKind::Brush(solid) => {
                let Some(owner) = node.parent.and_then(|p| entity_at.get(&(p as usize))).and_then(|&at| out.get_mut(at)) else { continue };
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

fn reason(classname: &str) -> Option<&'static str> {
    Some(match classname {
        "zone_respawn" => "v3's .zon has no respawn-only zone; a zone_checkpoint stage moves the respawn",
        "kill_box" => "v3's .zon has no kill zones",
        "teleport_zone" | "zone_point_tp" => "v3's .zon has no teleport zones; it keeps the BSP's own trigger_teleport",
        "zone_booster" => "v3's .zon has no boosters",
        "bhop_zone" => "v3's .zon has no bunny hop areas",
        "starting_platform" | "finish_line" => "a platform is a point entity with no zone box; place a zone_start or zone_finish",
        _ => return None,
    })
}

fn number(v: f64) -> String {
    format!("{:.6}", v + 0.0)
}

fn corner(p: DVec3) -> String {
    format!("{} {} {}", number(p.x), number(p.y), number(p.z))
}

fn block(out: &mut String, z: &Zone) {
    let color = if z.kind == Kind::Start { "0 255 255 255" } else { "255 0 255 255" };
    let flag = |k: Kind| if z.kind == k { "1" } else { "0" };
    let count = |k: Kind| if z.kind == k { z.number.to_string() } else { "0".into() };
    let effects = [
        ("start", flag(Kind::Start).to_string()),
        ("end", flag(Kind::End).to_string()),
        ("prespeed", "350".into()),
        ("TeleToStart", "0".into()),
        ("Validator", "0".into()),
        ("bonus", z.bonus.to_string()),
        ("Checkpoint", count(Kind::Checkpoint)),
        ("Stop", "0".into()),
        ("Misc", "0".into()),
        ("Checker", "0".into()),
        ("Stage", count(Kind::Stage)),
        ("Speed", "0".into()),
    ];
    // v3 reads `start` and `end` up to the first `}` or `"effects"`, so they come first.
    let _ = write!(
        out,
        "\t\"{}\"\n\t{{\n\t\t\"start\"\t\t\"{}\"\n\t\t\"end\"\t\t\"{}\"\n\t\t\"type\"\t\t\"Box\"\n\t\t\"color\"\t\t\"{color}\"\n\t\t\"display\"\t\t\"Bottom\"\n\t\t\"origin\"\t\t\"0.000000 0.000000 0.000000\"\n\t\t\"radius\"\t\t\"10.000000\"\n\t\t\"effects\"\n\t\t{{\n\t\t\t\"fuckTimer\"\n\t\t\t{{\n",
        z.name,
        corner(z.min),
        corner(z.max)
    );
    for (key, value) in effects {
        let _ = writeln!(out, "\t\t\t\t\"{key}\"\t\t\"{value}\"");
    }
    out.push_str("\t\t\t}\n\t\t}\n\t}\n");
}

/// The `.zon` for the map's zone entities; an error when it has no main start or no main finish.
pub fn write(map: &MapData) -> Result<Written, String> {
    let mut log = Vec::new();
    let mut starts: BTreeMap<i64, Vec<(DVec3, DVec3)>> = BTreeMap::new();
    let mut ends: BTreeMap<i64, Vec<(DVec3, DVec3)>> = BTreeMap::new();
    // Stages and checkpoints share v3's numbering; each carries its split_index, if any, and box.
    let mut line: Vec<(Kind, Option<i64>, (DVec3, DVec3))> = Vec::new();
    for f in found(map) {
        let class = f.classname.as_str();
        if let Some(why) = reason(class) {
            log.push(format!("skipped a {class}: {why}"));
            continue;
        }
        if !matches!(class, "zone_start" | "zone_finish" | "zone_checkpoint" | "zone_split") {
            continue;
        }
        // A volume also keeps both corners from reading 0 0 0, which v3 takes for a missing zone.
        let Some(bounds) = f.bounds.filter(|(lo, hi)| (*hi - *lo).min_element() >= 1e-3) else {
            log.push(format!("skipped a {class} with no brush volume"));
            continue;
        };
        let course = integer(&f, "course_id").unwrap_or(0);
        match class {
            "zone_start" | "zone_finish" if course < 0 => log.push(format!("skipped a {class} with course {course}")),
            "zone_start" => starts.entry(course).or_default().push(bounds),
            "zone_finish" => ends.entry(course).or_default().push(bounds),
            "zone_split" if course != 0 => log.push(format!("skipped a zone_split on course {course}: v3's .zon only has main course splits")),
            "zone_split" => line.push((Kind::Checkpoint, integer(&f, "split_index").filter(|&i| i >= 0), bounds)),
            _ => line.push((Kind::Stage, None, bounds)),
        }
    }

    let mut zones = Vec::new();
    let mut course = |name: &str, bonus: usize, starts: &[(DVec3, DVec3)], ends: &[(DVec3, DVec3)], log: &mut Vec<String>| {
        if starts.len() > 1 {
            log.push(format!("{name} has {} start zones; v3 reads one, so only the first is written", starts.len()));
        }
        if let Some(&(min, max)) = starts.first() {
            zones.push(Zone { name: format!("{name}_start"), kind: Kind::Start, bonus, number: 0, min, max });
        }
        for (i, &(min, max)) in ends.iter().enumerate() {
            let suffix = if i == 0 { String::new() } else { (i + 1).to_string() };
            zones.push(Zone { name: format!("{name}_end{suffix}"), kind: Kind::End, bonus, number: 0, min, max });
        }
    };
    let main = (starts.remove(&0).unwrap_or_default(), ends.remove(&0).unwrap_or_default());
    if main.0.is_empty() || main.1.is_empty() {
        return Err("SurfsUp needs a zone_start and a zone_finish on course 0 for its timer to work".into());
    }
    course("main0", 0, &main.0, &main.1, &mut log);

    let courses: BTreeSet<i64> = starts.keys().chain(ends.keys()).copied().collect();
    let mut bonus = 0;
    for id in courses {
        let (s, e) = (starts.remove(&id).unwrap_or_default(), ends.remove(&id).unwrap_or_default());
        if s.is_empty() || e.is_empty() {
            log.push(format!("skipped course {id}: it needs both a zone_start and a zone_finish"));
            continue;
        }
        bonus += 1;
        if bonus != id as usize {
            log.push(format!("course {id} is written as bonus{bonus}, since v3 numbers bonuses from 1 without gaps"));
        }
        course(&format!("bonus{bonus}"), bonus, &s, &e, &mut log);
    }

    // Splits with a split_index take the split slots in that order; everything else keeps document order.
    let mut splits: Vec<_> = line.iter().filter(|z| z.0 == Kind::Checkpoint).cloned().collect();
    splits.sort_by_key(|z| z.1.unwrap_or(i64::MAX));
    let mut splits = splits.into_iter();
    for (n, slot) in line.iter().enumerate() {
        let (kind, _, (min, max)) = if slot.0 == Kind::Checkpoint { splits.next().unwrap_or(*slot) } else { *slot };
        let number = n + 2;
        if number > MAX_STAGE {
            log.push(format!("skipped stages and checkpoints past {MAX_STAGE}, where v3 stops reading"));
            break;
        }
        let name = if kind == Kind::Stage { format!("stage{number}") } else { format!("checkpoint{number}") };
        zones.push(Zone { name, kind, bonus: 0, number, min, max });
    }

    let mut text = String::from("\"zones\"\n{\n");
    for z in &zones {
        block(&mut text, z);
    }
    text.push_str("}\n");
    let count = |k: Kind| zones.iter().filter(|z| z.kind == k).count();
    log.push(format!(
        "wrote {} zones: {} starts, {} finishes, {bonus} bonuses, {} stages, {} checkpoints",
        zones.len(),
        count(Kind::Start),
        count(Kind::End),
        count(Kind::Stage),
        count(Kind::Checkpoint)
    ));
    Ok(Written { text, log })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapcraft::plugin::types::{MapEntity, MapNode, Solid, Vec3};

    type Box3 = (DVec3, DVec3);

    fn find_from(text: &str, what: &str, from: usize) -> Option<usize> {
        text.get(from..)?.find(what).map(|i| i + from)
    }

    /// v3's `_string_value`: the quoted string after the first `"key"` in the body.
    fn string_value(body: &str, key: &str) -> String {
        let Some(at) = body.find(&format!("\"{key}\"")) else { return String::new() };
        let Some(q1) = find_from(body, "\"", at + key.len() + 2) else { return String::new() };
        let Some(q2) = find_from(body, "\"", q1 + 1) else { return String::new() };
        body[q1 + 1..q2].to_string()
    }

    fn vec3(s: &str) -> DVec3 {
        let parts: Vec<f64> = s.split(' ').filter(|p| !p.is_empty()).map(|p| p.parse().unwrap_or(0.0)).collect();
        if parts.len() < 3 { DVec3::ZERO } else { DVec3::new(parts[0], parts[1], parts[2]) }
    }

    /// v3's `_parse_zone`: the first `"name"` anywhere, its body up to the first `}` or `"effects"`.
    fn parse_zone(text: &str, name: &str) -> Option<Box3> {
        let key = text.find(&format!("\"{name}\""))?;
        let open = find_from(text, "{", key)?;
        let mut end = find_from(text, "}", open)?;
        if let Some(effects) = find_from(text, "\"effects\"", open).filter(|&e| e > 0 && e < end) {
            end = effects;
        }
        let body = &text[open..end];
        let (a, b) = (vec3(&string_value(body, "start")), vec3(&string_value(body, "end")));
        (a != DVec3::ZERO || b != DVec3::ZERO).then(|| (a.min(b), a.max(b)))
    }

    /// Every zone v3's `load_zones` picks up, by block name.
    fn v3_load(text: &str) -> BTreeMap<String, Box3> {
        let mut out = BTreeMap::new();
        let mut take = |name: String| parse_zone(text, &name).map(|z| out.insert(name, z)).is_some();
        let extras = |take: &mut dyn FnMut(String) -> bool, end: &str| (2..).take_while(|n| take(format!("{end}{n}"))).count();
        take("main0_start".into());
        if take("main0_end".into()) {
            extras(&mut take, "main0_end");
        }
        for n in 1.. {
            let (s, e) = (take(format!("bonus{n}_start")), take(format!("bonus{n}_end")));
            if !s && !e {
                break;
            }
            extras(&mut take, &format!("bonus{n}_end"));
        }
        let (mut index, mut missing) = (2, 0);
        while missing <= 2 && index <= 80 {
            let stage = parse_zone(text, &format!("stage{index}"));
            let split = parse_zone(text, &format!("checkpoint{index}"));
            match (stage, split) {
                (None, None) => missing += 1,
                (Some(z), _) => {
                    missing = 0;
                    out.insert(format!("stage{index}"), z);
                }
                (None, Some(z)) => {
                    missing = 0;
                    out.insert(format!("checkpoint{index}"), z);
                }
            }
            index += 1;
        }
        out
    }

    fn b(x: f64, y: f64, z: f64, s: f64) -> Box3 {
        (DVec3::new(x, y, z), DVec3::new(x + s, y + s / 2.0, z + 128.0))
    }

    struct Map(Vec<MapNode>);

    impl Map {
        fn node(&mut self, parent: Option<usize>, kind: MapNodeKind) -> usize {
            self.0.push(MapNode { parent: parent.map(|p| p as u32), kind });
            self.0.len() - 1
        }

        fn zone(&mut self, parent: Option<usize>, classname: &str, props: &[(&str, &str)], bounds: Option<Box3>) {
            let properties = props.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            let at = self.node(parent, MapNodeKind::Entity(MapEntity { classname: classname.into(), properties }));
            if let Some((lo, hi)) = bounds {
                let vertices = (0..8)
                    .map(|i| Vec3 { x: if i & 1 == 0 { lo.x } else { hi.x }, y: if i & 2 == 0 { lo.y } else { hi.y }, z: if i & 4 == 0 { lo.z } else { hi.z } })
                    .collect();
                self.node(Some(at), MapNodeKind::Brush(Solid { faces: Vec::new(), vertices }));
            }
        }

        fn data(self) -> MapData {
            MapData { name: "surf_test".into(), world: Vec::new(), nodes: self.0 }
        }
    }

    #[test]
    fn v3_finds_every_zone_written() {
        let mut m = Map(Vec::new());
        let layer = m.node(None, MapNodeKind::Layer("Zones".into()));
        let group = m.node(Some(layer), MapNodeKind::Group("Course".into()));
        m.zone(Some(group), "zone_start", &[], Some(b(0.0, 0.0, 0.0, 512.0)));
        m.zone(None, "zone_checkpoint", &[], Some(b(1000.0, 0.0, -500.0, 512.0)));
        m.zone(None, "zone_split", &[("split_index", "1")], Some(b(2000.0, 0.0, -1000.0, 256.0)));
        m.zone(None, "zone_checkpoint", &[], Some(b(3000.0, 0.0, -1500.0, 512.0)));
        m.zone(None, "zone_split", &[("split_index", "0")], Some(b(4000.0, 0.0, -2000.0, 256.0)));
        m.zone(Some(group), "zone_finish", &[("course_id", "0")], Some(b(-5000.0, -2000.0, -3000.0, 640.0)));
        m.zone(None, "ZONE_FINISH", &[], Some(b(-6000.0, -2000.0, -3000.0, 640.0)));
        m.zone(None, "zone_start", &[("course_id", "2")], Some(b(8000.0, 8000.0, 0.0, 256.0)));
        m.zone(None, "zone_finish", &[("course_id", "2.0")], Some(b(9000.0, 8000.0, 0.0, 256.0)));
        m.zone(None, "zone_start", &[("course_id", "3")], Some(b(10000.0, 8000.0, 0.0, 256.0)));
        m.zone(None, "zone_split", &[("course_id", "2")], Some(b(11000.0, 0.0, 0.0, 64.0)));
        m.zone(None, "zone_start", &[], Some(b(12000.0, 0.0, 0.0, 64.0)));
        for class in ["kill_box", "teleport_zone", "zone_respawn", "zone_booster", "zone_point_tp", "bhop_zone"] {
            m.zone(None, class, &[], Some(b(0.0, 0.0, -9000.0, 64.0)));
        }
        m.zone(None, "starting_platform", &[], None);
        m.zone(None, "zone_checkpoint", &[], None);
        m.zone(None, "trigger_teleport", &[], Some(b(0.0, 0.0, 0.0, 64.0)));
        let written = write(&m.data()).unwrap();
        let expected: BTreeMap<String, Box3> = [
            ("main0_start", b(0.0, 0.0, 0.0, 512.0)),
            ("main0_end", b(-5000.0, -2000.0, -3000.0, 640.0)),
            ("main0_end2", b(-6000.0, -2000.0, -3000.0, 640.0)),
            ("bonus1_start", b(8000.0, 8000.0, 0.0, 256.0)),
            ("bonus1_end", b(9000.0, 8000.0, 0.0, 256.0)),
            ("stage2", b(1000.0, 0.0, -500.0, 512.0)),
            ("checkpoint3", b(4000.0, 0.0, -2000.0, 256.0)),
            ("stage4", b(3000.0, 0.0, -1500.0, 512.0)),
            ("checkpoint5", b(2000.0, 0.0, -1000.0, 256.0)),
        ]
        .into_iter()
        .map(|(n, z)| (n.to_string(), z))
        .collect();
        assert_eq!(v3_load(&written.text), expected, "{}", written.text);
        assert!(!written.text.contains("origin_name") && written.text.starts_with("\"zones\"\n{\n"));
        let log = written.log.join("\n");
        for needle in [
            "course 2 is written as bonus1",
            "skipped course 3",
            "on course 2",
            "only the first",
            "kill_box",
            "teleport_zone",
            "zone_respawn",
            "zone_booster",
            "zone_point_tp",
            "bhop_zone",
            "starting_platform",
            "zone_checkpoint with no brush",
            "wrote 9 zones",
        ] {
            assert!(log.contains(needle), "{needle} missing from:\n{log}");
        }
        assert!(!log.contains("skipped a trigger_teleport"));
    }

    #[test]
    fn needs_a_main_start_and_finish() {
        assert!(write(&Map(Vec::new()).data()).is_err());
        let mut m = Map(Vec::new());
        m.zone(None, "zone_start", &[], Some(b(0.0, 0.0, 0.0, 64.0)));
        m.zone(None, "zone_finish", &[("course_id", "1")], Some(b(0.0, 0.0, 0.0, 64.0)));
        assert!(write(&m.data()).err().unwrap_or_default().contains("zone_finish"));
    }

    #[test]
    fn survives_hostile_maps() {
        let mut m = Map(Vec::new());
        m.zone(None, "zone_start", &[("course_id", "nan")], Some(b(0.0, 0.0, 0.0, 64.0)));
        m.zone(None, "zone_finish", &[("course_id", "-4")], Some(b(0.0, 0.0, 0.0, 64.0)));
        m.zone(None, "zone_finish", &[], Some((DVec3::ZERO, DVec3::ZERO)));
        m.0.push(MapNode {
            parent: Some(u32::MAX),
            kind: MapNodeKind::Brush(Solid { faces: Vec::new(), vertices: vec![Vec3 { x: f64::NAN, y: 0.0, z: 0.0 }] }),
        });
        m.zone(None, "zone_finish", &[], Some((DVec3::splat(-1e-7), DVec3::splat(1e-7))));
        m.zone(None, "zone_finish", &[], Some(b(100.0, 0.0, 0.0, 64.0)));
        for i in 0..100 {
            m.zone(None, "zone_checkpoint", &[], Some(b(f64::from(i) * 100.0, 0.0, 0.0, 64.0)));
        }
        let written = write(&m.data()).unwrap();
        let found = v3_load(&written.text);
        assert_eq!(found.keys().filter(|k| k.starts_with("stage")).count(), 79);
        assert!(found.contains_key("main0_start") && found.contains_key("main0_end") && !found.contains_key("main0_end2"));
        assert!(written.log.iter().any(|l| l.contains("past 80")));
    }
}
