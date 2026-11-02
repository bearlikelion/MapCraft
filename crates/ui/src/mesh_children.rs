//! Editable Children: a prop's glTF meshes listed under it, picked, moved and hidden one at a time.

use std::sync::Arc;

use mapcraft_engine::MeshFrame;
use mapcraft_geom::{Aabb, DMat4, DVec3};
use mapcraft_map::mesh_edit::{MeshEdit, mesh_edits};
use mapcraft_map::{Entity, NodeId, NodeKind};
use mapcraft_render::{Model, edited_model, entity_transform, mesh_matrices};
use serde_json::Value;

use crate::app::MapCraftApp;

/// The props whose meshes the scene tree lists, kept in the map's editor state.
const EDITABLE: &str = "editable_children";

/// Every node at or under `node` in `model`, which moving or hiding `node` carries along.
fn subtree(model: &Model, node: usize) -> Vec<bool> {
    let mut inside = vec![false; model.nodes.len()];
    for (i, n) in model.nodes.iter().enumerate() {
        let below = n.parent.and_then(|p| inside.get(p).copied()).unwrap_or(false);
        if let Some(slot) = inside.get_mut(i) {
            *slot = i == node || below;
        }
    }
    inside
}

impl MapCraftApp {
    /// A prop's entity and its model, when the model has meshes it can edit one by one.
    pub(crate) fn prop_model(&self, id: NodeId) -> Option<(Entity, Arc<Model>)> {
        let n = self.session.map.get(id).filter(|n| n.children.is_empty())?;
        let NodeKind::Entity(e) = &n.kind else { return None };
        let model = self.project.models.get(&self.project.model_path(e)?)?;
        (!model.nodes.is_empty() && model.rig.is_none()).then(|| (e.clone(), model))
    }

    pub(crate) fn editable_children(&self, id: NodeId) -> bool {
        self.session.view.get(EDITABLE).and_then(Value::as_array).is_some_and(|a| a.iter().any(|v| v.as_u64() == Some(id.0)))
    }

    pub(crate) fn toggle_editable_children(&mut self, id: NodeId) {
        let on = !self.editable_children(id);
        if !self.session.view.is_object() {
            self.session.view = Value::Object(serde_json::Map::new());
        }
        let mut ids: Vec<u64> = self.session.view.get(EDITABLE).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_u64).collect();
        ids.retain(|i| *i != id.0 && self.session.map.get(NodeId(*i)).is_some());
        if on {
            ids.push(id.0);
            self.expanded.insert(id);
        } else if self.picked_mesh.as_ref().is_some_and(|(p, _)| *p == id) {
            self.picked_mesh = None;
        }
        if let Some(o) = self.session.view.as_object_mut() {
            o.insert(EDITABLE.into(), ids.into_iter().map(Value::from).collect());
        }
        self.session.mark_unsaved();
    }

    /// Where the gizmo moves the picked mesh from; clears the pick once its prop is no longer the selection.
    pub(crate) fn sync_mesh_frame(&mut self) {
        let frame = self
            .picked_mesh
            .as_ref()
            .filter(|(id, _)| self.session.selection.len() == 1 && self.session.selection.contains(id) && self.editable_children(*id))
            .and_then(|(id, name)| self.mesh_frame(*id, name));
        if frame.is_none() {
            self.picked_mesh = None;
        }
        self.session.mesh = frame;
    }

    fn mesh_frame(&self, id: NodeId, name: &str) -> Option<MeshFrame> {
        let (e, model) = self.prop_model(id)?;
        let i = model.nodes.iter().position(|n| n.name == name)?;
        let node = model.nodes.get(i)?;
        let moved = mesh_matrices(&model, &mesh_edits(&e));
        let above = match node.parent {
            Some(p) => (*moved.get(p)?)?,
            None => DMat4::IDENTITY,
        };
        Some(MeshFrame { prop: id, name: name.to_string(), frame: entity_transform(&e) * above, pivot: node.origin })
    }

    /// The world points of the visible triangles of mesh `name` and the meshes under it, three per triangle.
    fn mesh_triangles(&self, id: NodeId, name: &str) -> Vec<DVec3> {
        let Some((e, model)) = self.prop_model(id) else { return Vec::new() };
        let Some(node) = model.nodes.iter().position(|n| n.name == name) else { return Vec::new() };
        let inside = subtree(&model, node);
        let at = entity_transform(&e);
        let edited = edited_model(&model, &e);
        let parts = edited.parts.iter().filter(|p| p.node.and_then(|n| inside.get(n).copied()).unwrap_or(false));
        let point = |p: &mapcraft_render::ModelPart, i: &u32| p.positions.get(*i as usize).map(|v| at.transform_point3(DVec3::from_array(v.map(f64::from))));
        parts.flat_map(|p| p.indices.iter().filter_map(move |i| point(p, i))).collect()
    }

    /// The world box of a mesh and the meshes under it; `None` when all of them are hidden.
    pub(crate) fn mesh_bounds(&self, id: NodeId, name: &str) -> Option<Aabb> {
        let points = self.mesh_triangles(id, name);
        let first = *points.first()?;
        let (lo, hi) = points.iter().fold((first, first), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
        Some(Aabb::new(lo, hi))
    }

    /// The mesh of an editable prop a ray hits first.
    pub(crate) fn pick_mesh(&self, id: NodeId, origin: DVec3, dir: DVec3) -> Option<String> {
        let (e, model) = self.prop_model(id)?;
        let at = entity_transform(&e);
        let edited = edited_model(&model, &e);
        let mut best: Option<(f64, usize)> = None;
        for part in &edited.parts {
            let Some(node) = part.node else { continue };
            let point = |i: &u32| part.positions.get(*i as usize).map(|v| at.transform_point3(DVec3::from_array(v.map(f64::from))));
            for [i, j, k] in part.indices.as_chunks::<3>().0 {
                let (Some(a), Some(b), Some(c)) = (point(i), point(j), point(k)) else { continue };
                if let Some(t) = mapcraft_map::disp::ray_triangle(origin, dir, [a, b, c])
                    && best.is_none_or(|(d, _)| t < d)
                {
                    best = Some((t, node));
                }
            }
        }
        model.nodes.get(best?.1).map(|n| n.name.clone())
    }

    /// Selects mesh `name` of prop `id`, turning on its Editable Children.
    pub(crate) fn select_mesh(&mut self, id: NodeId, name: &str) {
        if !self.editable_children(id) {
            self.toggle_editable_children(id);
        }
        self.session.selection = std::collections::BTreeSet::from([id]);
        self.picked_mesh = Some((id, name.to_string()));
        self.sync_mesh_frame();
    }

    pub(crate) fn mesh_edit(&self, id: NodeId, name: &str) -> MeshEdit {
        let Some(NodeKind::Entity(e)) = self.session.map.get(id).map(|n| &n.kind) else { return MeshEdit::default() };
        mesh_edits(e).remove(name).unwrap_or_default()
    }

    pub(crate) fn set_mesh_edit(&mut self, id: NodeId, name: &str, edit: &MeshEdit, coalesce: Option<&str>) {
        if let Err(e) = self.session.set_mesh_edit(id, name, edit, coalesce) {
            self.status = e.to_string();
        }
        self.mark_changed();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mapcraft_render::ModelNode;

    #[test]
    fn a_subtree_holds_the_node_and_everything_under_it() {
        let node = |parent: Option<usize>| ModelNode { parent, ..ModelNode::default() };
        let model = Model { nodes: vec![node(None), node(Some(0)), node(Some(1)), node(None)], ..Model::default() };
        assert_eq!(subtree(&model, 1), [false, true, true, false]);
        assert_eq!(subtree(&model, 0), [true, true, true, false]);
    }
}
