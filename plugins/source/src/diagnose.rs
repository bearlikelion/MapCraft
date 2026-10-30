//! Problems in vbsp, vvis and vrad output, traced to the brushes and entities of the map the VMF exporter wrote.

use std::collections::BTreeSet;

use crate::mapcraft::plugin::types::{Issue, MapData, MapNodeKind, Severity};
use crate::vmf::node_of;

/// Brushes one missing material can flag before the rest are left out.
const MAX_PER_MATERIAL: usize = 100;
/// How far an entity may be from a position a message gives and still be the one it means.
const NEAR: f64 = 64.0;
/// Words that make a line a problem even when it names no node.
const PROBLEMS: [&str; 6] = ["error", "warning", "leaked", "not found", "outside", "in solid"];

fn normalize(path: &str) -> String {
    path.trim().replace('\\', "/").to_ascii_lowercase()
}

/// The number right after `prefix` in `line`.
fn number_after(line: &str, prefix: &str) -> Option<u64> {
    let rest = &line[line.find(prefix)? + prefix.len()..];
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// The text between the first pair of `quote`s after `prefix`.
fn quoted_after<'a>(line: &'a str, prefix: &str, quote: char) -> Option<&'a str> {
    let rest = &line[line.find(prefix)? + prefix.len()..];
    let rest = &rest[rest.find(quote)? + 1..];
    Some(&rest[..rest.find(quote)?])
}

/// The last three numbers in a row in `line`, such as `(1.0 2.0 3.0)` or `at 1, 2, 3`.
fn position(line: &str) -> Option<[f64; 3]> {
    let numbers: Vec<f64> = line
        .split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | ','))
        .filter(|t| !t.is_empty())
        .map(str::parse::<f64>)
        .collect::<Vec<_>>()
        .split(Result::is_err)
        .rfind(|run| run.len() >= 3)?
        .iter()
        .rev()
        .take(3)
        .filter_map(|n| n.as_ref().ok().copied())
        .collect();
    let [z, y, x] = numbers[..] else { return None };
    [x, y, z].iter().all(|v| v.is_finite()).then_some([x, y, z])
}

struct Map<'a> {
    data: &'a MapData,
    /// The `map-data.nodes` place of each entity vbsp numbers, the world first as `None`.
    entities: Vec<Option<usize>>,
    /// Each of those entities' brushes, in the order the exporter writes them.
    brushes: Vec<Vec<usize>>,
}

impl<'a> Map<'a> {
    fn new(data: &'a MapData) -> Self {
        let parent = |i: usize| data.nodes.get(i).and_then(|n| n.parent).and_then(|p| usize::try_from(p).ok());
        let is_entity = |i: usize| data.nodes.get(i).is_some_and(|n| matches!(n.kind, MapNodeKind::Entity(_)));
        let solid = |i: usize| data.nodes.get(i).is_some_and(|n| matches!(&n.kind, MapNodeKind::Brush(s) if s.faces.len() >= 4));
        let mut entities = vec![None];
        let mut brushes = vec![(0..data.nodes.len()).filter(|&i| solid(i) && !parent(i).is_some_and(is_entity)).collect()];
        for i in (0..data.nodes.len()).filter(|&i| is_entity(i)) {
            entities.push(Some(i));
            brushes.push((0..data.nodes.len()).filter(|&b| solid(b) && parent(b) == Some(i)).collect());
        }
        Map { data, entities, brushes }
    }

    fn entity(&self, i: usize) -> Option<(&'a str, &'a [(String, String)])> {
        match &self.data.nodes.get(i)?.kind {
            MapNodeKind::Entity(e) => Some((&e.classname, &e.properties)),
            _ => None,
        }
    }

    fn key(&self, i: usize, key: &str) -> Option<&'a str> {
        self.entity(i)?.1.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str())
    }

    /// The entity of `class`, or any class, whose origin is nearest `at`, if it is near enough.
    fn nearest(&self, at: [f64; 3], class: Option<&str>) -> Option<usize> {
        self.entities
            .iter()
            .flatten()
            .filter(|&&i| class.is_none_or(|c| self.entity(i).is_some_and(|(name, _)| name.eq_ignore_ascii_case(c))))
            .filter_map(|&i| {
                let o: Vec<f64> = self.key(i, "origin")?.split_whitespace().filter_map(|v| v.parse().ok()).collect();
                let [x, y, z] = o[..] else { return None };
                let d = ((x - at[0]).powi(2) + (y - at[1]).powi(2) + (z - at[2]).powi(2)).sqrt();
                (d <= NEAR).then_some((d, i))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, i)| i)
    }

    /// The nodes a line of compiler output is about, empty for a problem with the whole map, and
    /// `None` for a line that reports no problem.
    fn nodes(&self, line: &str) -> Option<Vec<usize>> {
        let brush = |i: &usize| matches!(self.data.nodes.get(*i).map(|n| &n.kind), Some(MapNodeKind::Brush(_)));
        if let Some(id) = number_after(line, "Brush ").filter(|_| line.contains(':')) {
            return Some(node_of(id).filter(brush).into_iter().collect());
        }
        if let (Some(e), Some(b)) = (number_after(line, "(entity "), number_after(line, ", brush ")) {
            let found = usize::try_from(e).ok().and_then(|e| self.brushes.get(e)).and_then(|list| usize::try_from(b).ok().and_then(|b| list.get(b)));
            return Some(found.copied().into_iter().collect());
        }
        if let Some(name) = line.split_once("Material not found!:").map(|(_, n)| normalize(n)) {
            let uses = |i: &usize| matches!(self.data.nodes.get(*i).map(|n| &n.kind), Some(MapNodeKind::Brush(s)) if s.faces.iter().any(|f| normalize(&f.material) == name));
            return Some((0..self.data.nodes.len()).filter(uses).take(MAX_PER_MATERIAL).collect());
        }
        let model = quoted_after(line, "model", '"').or_else(|| quoted_after(line, "Static Prop", '\''));
        if let Some(model) = model.map(normalize) {
            return Some(self.entities.iter().flatten().copied().filter(|&i| self.key(i, "model").is_some_and(|m| normalize(m) == model)).collect());
        }
        let lower = line.to_ascii_lowercase();
        if !PROBLEMS.iter().any(|w| lower.contains(w)) {
            return None;
        }
        let class = line.strip_prefix("Entity ").filter(|_| line.contains("leaked")).and_then(|r| r.split_whitespace().next());
        let class = class.or_else(|| line.contains("Static prop").then_some("prop_static"));
        Some(position(line).and_then(|at| self.nearest(at, class)).into_iter().collect())
    }
}

/// Issues for the problems `log` reports, each on the node it names when it names one.
pub fn issues(data: &MapData, log: &[String]) -> Vec<Issue> {
    let map = Map::new(data);
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for line in log.iter().map(|l| l.trim()) {
        let Some(nodes) = map.nodes(line) else { continue };
        let lower = line.to_ascii_lowercase();
        let severity = if ["error", "leaked", "not found"].iter().any(|w| lower.contains(w)) { Severity::Error } else { Severity::Warning };
        let nodes: Vec<Option<u32>> = if nodes.is_empty() { vec![None] } else { nodes.into_iter().map(|n| u32::try_from(n).ok()).collect() };
        for node in nodes {
            if seen.insert((line.to_string(), node)) {
                out.push(Issue { severity, message: line.to_string(), node });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapcraft::plugin::types::{Face, MapEntity, MapNode, Solid, Vec3};

    fn brush(material: &str) -> MapNodeKind {
        let p = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
        let face = Face { points: (p, p, p), material: material.into(), uv: None, attrs: Vec::new(), disp: None };
        MapNodeKind::Brush(Solid { faces: vec![face; 6], vertices: Vec::new() })
    }

    fn entity(class: &str, keys: &[(&str, &str)]) -> MapNodeKind {
        let properties = keys.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        MapNodeKind::Entity(MapEntity { classname: class.into(), properties, connections: Vec::new() })
    }

    fn map() -> MapData {
        let node = |parent: Option<u32>, kind| MapNode { parent, kind };
        MapData {
            name: "m".into(),
            world: Vec::new(),
            nodes: vec![
                node(None, MapNodeKind::Layer("a".into())),
                node(Some(0), brush("dev/dev_measuregeneric01b")),
                node(Some(0), brush("Custom\\Missing")),
                node(None, entity("trigger_multiple", &[])),
                node(Some(3), brush("tools/toolstrigger")),
                node(None, entity("prop_static", &[("model", "models/props/crate.mdl"), ("origin", "100 0 0")])),
                node(None, entity("info_player_terrorist", &[("origin", "0 0 0")])),
            ],
        }
    }

    fn found(lines: &[&str]) -> Vec<(String, Option<u32>)> {
        let log: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        issues(&map(), &log).into_iter().map(|i| (i.message, i.node)).collect()
    }

    #[test]
    fn traces_messages_to_nodes() {
        assert_eq!(found(&["Brush 3: mixed face contents"]), [("Brush 3: mixed face contents".into(), Some(1))]);
        assert_eq!(found(&["Material not found!: custom/missing"]), [("Material not found!: custom/missing".into(), Some(2))]);
        assert_eq!(found(&["Trying to create a non-quad displacement! (entity 1, brush 0)"])[0].1, Some(4));
        assert_eq!(found(&["Error loading studio model \"models/props/crate.mdl\"!"])[0].1, Some(5));
        assert_eq!(found(&["Error! prop_static using model \"models/props/crate.mdl\", which must be used on a dynamic entity. Deleted."])[0].1, Some(5));
        assert_eq!(found(&["Static prop models/props/crate.mdl outside the map (101.00, 2.00, 0.00)"])[0].1, Some(5));
        assert_eq!(found(&["Entity info_player_terrorist (1.00 2.00 3.00) leaked!"])[0].1, Some(6));
        assert_eq!(found(&["**** leaked ****"]), [("**** leaked ****".into(), None)]);
        assert!(found(&["Building Faces...done (0)", "12 threads", "> vbsp -game x"]).is_empty());
        assert_eq!(found(&["Brush 999: bounds out of range"]), [("Brush 999: bounds out of range".into(), None)], "an id the map lacks stays map-wide");
    }
}
