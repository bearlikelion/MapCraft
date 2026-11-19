use std::collections::BTreeSet;

use mapcraft_map::{Edit, NodeId, NodeKind, ROOT, ShaderParams};

use crate::{EngineError, Session};

pub const SPATIAL_TEMPLATE: &str = "shader_type spatial;

uniform vec4 albedo : source_color = vec4(1.0);

void fragment() {
	ALBEDO = albedo.rgb;
}
";

pub const SKY_TEMPLATE: &str = "shader_type sky;

uniform vec3 sky_top : source_color = vec3(0.25, 0.45, 0.8);
uniform vec3 horizon : source_color = vec3(0.75, 0.85, 0.95);

void sky() {
	COLOR = mix(horizon, sky_top, clamp(EYEDIR.y, 0.0, 1.0));
}
";

/// Embeds `shader`, a source and its parameters, in `id`.
pub(crate) fn put_shader(e: &mut Edit, id: NodeId, shader: Option<(String, ShaderParams)>) -> Result<(), EngineError> {
    let Some((source, params)) = shader else { return Ok(()) };
    e.set_shader(id, Some(source))?;
    for (name, value) in params {
        e.set_shader_param(id, &name, Some(value))?;
    }
    Ok(())
}

impl Session {
    /// Embeds `source` in every node of `ids`, or removes their shaders with `None`; edits to the same nodes' source merge into one undo step.
    pub fn set_shader(&mut self, ids: &[NodeId], source: Option<String>) -> Result<(), EngineError> {
        let Some(first) = ids.first() else { return Ok(()) };
        let old = |id: &NodeId| self.map.get(*id).and_then(|n| n.shader.as_deref());
        if ids.iter().all(|id| self.map.get(*id).is_some() && old(id) == source.as_deref()) {
            return Ok(());
        }
        let (label, key) = match (old(first), &source) {
            (None, _) => ("Add shader", None),
            (_, None) => ("Remove shader", None),
            _ => ("Edit shader", Some(format!("shader {}", first.0))),
        };
        let mut e = self.map.edit();
        for id in ids {
            e.set_shader(*id, source.clone())?;
        }
        let c = e.finish();
        self.apply_coalesced(label, c, key.as_deref());
        Ok(())
    }

    /// Sets one uniform of the shaders of `ids`, or returns it to the shader's default with `None`; changes to one parameter merge into one undo step.
    pub fn set_shader_param(&mut self, ids: &[NodeId], name: &str, value: Option<serde_json::Value>) -> Result<(), EngineError> {
        let Some(first) = ids.first() else { return Ok(()) };
        if ids.iter().all(|id| self.map.get(*id).is_some_and(|n| n.shader_params.get(name) == value.as_ref())) {
            return Ok(());
        }
        let mut e = self.map.edit();
        for id in ids {
            e.set_shader_param(*id, name, value.clone())?;
        }
        let c = e.finish();
        self.apply_coalesced("Edit shader parameter", c, Some(&format!("shader param {} {name}", first.0)));
        Ok(())
    }

    /// What an edit of `id`'s shader reaches: `id`, and when it is selected, the selected brushes carrying the same source.
    pub fn shader_peers(&self, id: NodeId) -> Vec<NodeId> {
        let source = |n: &NodeId| self.map.get(*n).and_then(|n| n.shader.as_deref());
        let brushes = self.selected_brushes();
        if source(&id).is_none() || !brushes.contains(&id) {
            return vec![id];
        }
        brushes.into_iter().filter(|b| source(b) == source(&id)).collect()
    }

    /// Selects every brush a click can reach whose shader matches a selected brush's.
    pub fn select_same_shader(&mut self) -> Result<(), EngineError> {
        let sources: BTreeSet<String> = self.selected_brushes().iter().filter_map(|id| self.map.get(*id)?.shader.clone()).collect();
        let editable = self.map.editable_nodes();
        let same = |n: &mapcraft_map::Node| n.kind.brush().is_some() && n.shader.as_ref().is_some_and(|s| sources.contains(s));
        let brushes: BTreeSet<NodeId> = self.map.nodes().filter(|(id, n)| editable.contains(id) && same(n)).map(|(id, _)| id).collect();
        if brushes.is_empty() {
            return Err(EngineError::Refused("select a brush with a shader first"));
        }
        self.selection = brushes;
        self.face_selection.clear();
        Ok(())
    }

    pub(crate) fn can_select_same_shader(&self) -> bool {
        self.selected_brushes().iter().any(|id| self.map.get(*id).is_some_and(|n| n.shader.is_some()))
    }

    /// What Add Shader reaches: the selected brushes, else the world.
    fn shader_targets(&self) -> Vec<NodeId> {
        let brushes = if self.selection.contains(&ROOT) { Vec::new() } else { self.selected_brushes() };
        if brushes.is_empty() { vec![ROOT] } else { brushes }
    }

    /// Gives the selected brushes with no shader the one the rest of them share, else a starter, or the world a sky shader with no brush selected.
    pub fn add_shader(&mut self) -> Result<(), EngineError> {
        let targets = self.shader_targets();
        let mut shaded = targets.iter().filter_map(|id| self.map.get(*id)).filter(|n| n.shader.is_some());
        let first = shaded.next();
        let shared = first.filter(|f| shaded.all(|n| n.shader == f.shader)).and_then(|n| Some((n.shader.clone()?, n.shader_params.clone())));
        let mut e = self.map.edit();
        for id in targets {
            let Some(node) = self.map.get(id).filter(|n| n.shader.is_none()) else { continue };
            let template = if matches!(node.kind, NodeKind::World(_)) { SKY_TEMPLATE } else { SPATIAL_TEMPLATE };
            put_shader(&mut e, id, Some(shared.clone().unwrap_or_else(|| (template.into(), ShaderParams::new()))))?;
        }
        let c = e.finish();
        self.apply("Add shader", c);
        Ok(())
    }

    pub fn remove_shader(&mut self) -> Result<(), EngineError> {
        let mut e = self.map.edit();
        for id in self.shader_targets() {
            if self.map.get(id).is_some_and(|n| n.shader.is_some()) {
                e.set_shader(id, None)?;
            }
        }
        let c = e.finish();
        self.apply("Remove shader", c);
        Ok(())
    }

    pub(crate) fn can_add_shader(&self) -> bool {
        self.shader_targets().iter().any(|id| self.map.get(*id).is_some_and(|n| n.shader.is_none()))
    }

    pub(crate) fn can_remove_shader(&self) -> bool {
        self.shader_targets().iter().any(|id| self.map.get(*id).is_some_and(|n| n.shader.is_some()))
    }
}

#[cfg(test)]
mod tests {
    use mapcraft_geom::{Aabb, DVec3};

    use super::*;

    #[test]
    fn shader_edits_undo_and_typing_merges() {
        let mut s = Session::default();
        let b = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
        s.selection = [b].into();
        s.add_shader().unwrap();
        assert_eq!(s.map.get(b).unwrap().shader.as_deref(), Some(SPATIAL_TEMPLATE));
        assert_eq!(s.undo_label(), Some("Add shader"));
        s.set_shader(&[b], Some("shader_type spatial;".into())).unwrap();
        s.set_shader(&[b], Some("shader_type spatial;\n".into())).unwrap();
        s.set_shader(&[b], Some("shader_type spatial;\n//".into())).unwrap();
        assert_eq!(s.undo_label(), Some("Edit shader"));
        assert!(s.undo());
        assert_eq!(s.map.get(b).unwrap().shader.as_deref(), Some(SPATIAL_TEMPLATE), "typing undoes as one step");
        s.selection.clear();
        s.add_shader().unwrap();
        assert_eq!(s.map.get(ROOT).unwrap().shader.as_deref(), Some(SKY_TEMPLATE), "no brush selected gives the world a sky");
        s.selection = [ROOT].into();
        s.remove_shader().unwrap();
        assert!(s.map.get(b).unwrap().shader.is_some(), "the selected world is not every brush in it");
        assert!(s.undo());
        assert!(s.set_shader(&[b], Some("x".repeat(mapcraft_map::MAX_SHADER_BYTES + 1))).is_err());
        s.remove_shader().unwrap();
        assert_eq!(s.map.get(ROOT).unwrap().shader, None);
        assert!(s.undo());
        assert!(s.map.get(ROOT).unwrap().shader.is_some());

        for v in [0.1, 0.2, 0.3] {
            s.set_shader_param(&[ROOT], "density", Some(serde_json::json!(v))).unwrap();
        }
        assert_eq!(s.undo_label(), Some("Edit shader parameter"));
        assert!(s.set_shader_param(&[ROOT], "density", Some(serde_json::json!({}))).is_err());
        assert!(s.undo());
        assert!(s.map.get(ROOT).unwrap().shader_params.is_empty(), "a drag undoes as one step");
    }

    fn brush(s: &mut Session, x: f64) -> NodeId {
        s.create_brush(Aabb::new(DVec3::new(x, 0.0, 0.0), DVec3::new(x + 64.0, 64.0, 64.0))).unwrap()
    }

    #[test]
    fn one_shader_goes_onto_many_brushes_and_edits_reach_the_selected_ones() {
        let mut s = Session::default();
        let (a, b, c, d) = (brush(&mut s, 0.0), brush(&mut s, 100.0), brush(&mut s, 200.0), brush(&mut s, 300.0));
        let source = |s: &Session, id: NodeId| s.map.get(id).unwrap().shader.clone();
        s.selection = [a].into();
        s.add_shader().unwrap();
        s.set_shader(&[a], Some("shader_type spatial; // water".into())).unwrap();
        s.set_shader_param(&[a], "depth", Some(serde_json::json!(2.5))).unwrap();

        s.selection = [a, b, c].into();
        s.add_shader().unwrap();
        assert_eq!(source(&s, b), source(&s, a), "the others take the shader already in the selection");
        assert_eq!(s.map.get(c).unwrap().shader_params.get("depth"), Some(&serde_json::json!(2.5)), "with its parameters");

        s.selection = [d].into();
        s.add_shader().unwrap();
        assert_eq!(source(&s, d).as_deref(), Some(SPATIAL_TEMPLATE));

        s.selection = [a, b, d].into();
        assert_eq!(s.shader_peers(a), vec![a, b], "selected brushes with the same source");
        assert_eq!(s.shader_peers(c), vec![c], "a brush outside the selection is edited alone");
        s.set_shader(&s.shader_peers(a), Some("shader_type spatial; // lava".into())).unwrap();
        assert_eq!(source(&s, b).as_deref(), Some("shader_type spatial; // lava"));
        assert_eq!(source(&s, c).as_deref(), Some("shader_type spatial; // water"), "unselected brushes keep theirs");
        assert_eq!(source(&s, d).as_deref(), Some(SPATIAL_TEMPLATE), "and so do selected brushes with another shader");
        assert!(s.undo());
        assert_eq!(source(&s, b).as_deref(), Some("shader_type spatial; // water"), "one undo step for all of them");

        s.set_shader_param(&[a, b], "depth", Some(serde_json::json!(4.0))).unwrap();
        assert_eq!(s.map.get(b).unwrap().shader_params.get("depth"), Some(&serde_json::json!(4.0)));

        s.selection = [c].into();
        s.select_same_shader().unwrap();
        assert_eq!(s.selection, [a, b, c].into());
        s.selection.clear();
        assert!(s.select_same_shader().is_err());
    }

    #[test]
    fn copies_keep_the_shader_and_its_parameters() {
        let mut s = Session::default();
        let a = brush(&mut s, 0.0);
        s.selection = [a].into();
        s.add_shader().unwrap();
        s.set_shader_param(&[a], "depth", Some(serde_json::json!(2.5))).unwrap();
        let shaded = |s: &Session| {
            s.selection.iter().all(|id| {
                let n = s.map.get(*id).unwrap();
                n.shader.as_deref() == Some(SPATIAL_TEMPLATE) && n.shader_params.get("depth") == Some(&serde_json::json!(2.5))
            })
        };

        s.duplicate_selection(DVec3::new(0.0, 100.0, 0.0), None).unwrap();
        assert!(!s.selection.contains(&a) && shaded(&s), "Duplicate");

        s.selection = [a].into();
        s.array_selection(2, &mapcraft_geom::DMat4::from_translation(DVec3::new(0.0, 0.0, 100.0))).unwrap();
        assert!(s.selection.len() == 2 && shaded(&s), "Paste Special");

        let text = mapcraft_format::fragment_to_string(&s.map, &[a]).unwrap();
        let frags = mapcraft_format::fragment_from_str(&text).unwrap();
        let mut e = s.map.edit();
        let pasted = crate::ops::insert_fragments(&mut e, ROOT, frags, &mapcraft_geom::DMat4::IDENTITY, false).unwrap();
        let c = e.finish();
        s.apply("Paste", c);
        s.selection = pasted.into_iter().collect();
        assert!(s.selection.len() == 1 && shaded(&s), "copy and paste");
    }
}
