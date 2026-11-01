use mapcraft_map::{NodeId, NodeKind, ROOT};

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

impl Session {
    /// Embeds `source` in a node, or removes its shader with `None`; edits to one node's source merge into one undo step.
    pub fn set_shader(&mut self, id: NodeId, source: Option<String>) -> Result<(), EngineError> {
        let old = self.map.get(id).ok_or(mapcraft_map::EditError::Missing(id))?.shader.as_deref();
        if old == source.as_deref() {
            return Ok(());
        }
        let (label, key) = match (old, &source) {
            (None, _) => ("Add shader", None),
            (_, None) => ("Remove shader", None),
            _ => ("Edit shader", Some(format!("shader {}", id.0))),
        };
        let mut e = self.map.edit();
        e.set_shader(id, source)?;
        let c = e.finish();
        self.apply_coalesced(label, c, key.as_deref());
        Ok(())
    }

    /// Sets one uniform of a node's shader, or returns it to the shader's default with `None`; changes to one parameter merge into one undo step.
    pub fn set_shader_param(&mut self, id: NodeId, name: &str, value: Option<serde_json::Value>) -> Result<(), EngineError> {
        let node = self.map.get(id).ok_or(mapcraft_map::EditError::Missing(id))?;
        if node.shader_params.get(name) == value.as_ref() {
            return Ok(());
        }
        let mut e = self.map.edit();
        e.set_shader_param(id, name, value)?;
        let c = e.finish();
        self.apply_coalesced("Edit shader parameter", c, Some(&format!("shader param {} {name}", id.0)));
        Ok(())
    }

    /// What Add Shader reaches: the selected brushes, else the world.
    fn shader_targets(&self) -> Vec<NodeId> {
        let brushes = if self.selection.contains(&ROOT) { Vec::new() } else { self.selected_brushes() };
        if brushes.is_empty() { vec![ROOT] } else { brushes }
    }

    /// Gives the selected brushes a spatial shader, or the world a sky shader with no brush selected; existing shaders are kept.
    pub fn add_shader(&mut self) -> Result<(), EngineError> {
        let mut e = self.map.edit();
        for id in self.shader_targets() {
            let Some(node) = self.map.get(id).filter(|n| n.shader.is_none()) else { continue };
            let template = if matches!(node.kind, NodeKind::World(_)) { SKY_TEMPLATE } else { SPATIAL_TEMPLATE };
            e.set_shader(id, Some(template.into()))?;
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
        s.set_shader(b, Some("shader_type spatial;".into())).unwrap();
        s.set_shader(b, Some("shader_type spatial;\n".into())).unwrap();
        s.set_shader(b, Some("shader_type spatial;\n//".into())).unwrap();
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
        assert!(s.set_shader(b, Some("x".repeat(mapcraft_map::MAX_SHADER_BYTES + 1))).is_err());
        s.remove_shader().unwrap();
        assert_eq!(s.map.get(ROOT).unwrap().shader, None);
        assert!(s.undo());
        assert!(s.map.get(ROOT).unwrap().shader.is_some());

        for v in [0.1, 0.2, 0.3] {
            s.set_shader_param(ROOT, "density", Some(serde_json::json!(v))).unwrap();
        }
        assert_eq!(s.undo_label(), Some("Edit shader parameter"));
        assert!(s.set_shader_param(ROOT, "density", Some(serde_json::json!({}))).is_err());
        assert!(s.undo());
        assert!(s.map.get(ROOT).unwrap().shader_params.is_empty(), "a drag undoes as one step");
    }
}
