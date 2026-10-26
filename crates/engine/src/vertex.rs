//! Vertex tool edits on brush components: moving and merging corners, beveling edges.

use mapcraft_geom::DVec3;
use mapcraft_map::{Bevel, Brush, NodeId, NodeKind, TexDefaults};

use crate::{EngineError, Session};

/// Corners within this squared distance count as the same corner.
const SAME: f64 = 1e-4;

impl Session {
    /// Moves corners of each brush `(from, to)`; refused as a whole when any brush would not be a solid.
    pub fn move_corners(&mut self, moves: &[(NodeId, Vec<(DVec3, DVec3)>)], coalesce: Option<&str>) -> Result<(), EngineError> {
        self.rebuild_brushes("Move vertices", moves, coalesce, |b, m, tex| {
            let mut verts = b.vertices()?;
            let mut changed = false;
            for v in &mut verts {
                if let Some((_, to)) = m.iter().find(|(from, _)| from.distance_squared(*v) < SAME) {
                    *v = *to;
                    changed = true;
                }
            }
            Ok(if changed { Some(Brush::from_vertices(&verts, &b.faces, tex)?) } else { None })
        })
    }

    /// Collapses the given corners of each brush to their center (Blender's merge at center).
    pub fn merge_corners(&mut self, corners: &[(NodeId, Vec<DVec3>)]) -> Result<(), EngineError> {
        self.rebuild_brushes("Merge vertices", corners, None, |b, c, tex| {
            if c.len() < 2 {
                return Ok(None);
            }
            let center = c.iter().sum::<DVec3>() / c.len() as f64;
            let mut verts: Vec<DVec3> = b.vertices()?.into_iter().filter(|v| !c.iter().any(|p| p.distance_squared(*v) < SAME)).collect();
            verts.push(center);
            Ok(Some(Brush::from_vertices(&verts, &b.faces, tex)?))
        })
    }

    /// Replaces each brush with its `original` beveled along `edges`, so re-running it with new
    /// settings starts over rather than beveling the bevel.
    pub fn bevel_edges(&mut self, originals: &[(NodeId, (Brush, Vec<(DVec3, DVec3)>))], bevel: &Bevel, coalesce: Option<&str>) -> Result<(), EngineError> {
        self.rebuild_brushes("Bevel", originals, coalesce, |_, (original, edges), _| Ok(Some(original.bevel(edges, bevel)?)))
    }

    fn rebuild_brushes<T>(
        &mut self,
        label: &str,
        items: &[(NodeId, T)],
        coalesce: Option<&str>,
        rebuild: impl Fn(&Brush, &T, &TexDefaults) -> Result<Option<Brush>, EngineError>,
    ) -> Result<(), EngineError> {
        let mut e = self.map.edit();
        for (id, item) in items {
            let Some(b) = e.node(*id).and_then(|n| n.kind.brush()) else { continue };
            if let Some(new) = rebuild(b, item, &self.tex)? {
                e.set_kind(*id, NodeKind::Brush(new))?;
            }
        }
        let c = e.finish();
        self.apply_coalesced(label, c, coalesce);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use mapcraft_geom::Aabb;

    use super::*;

    #[test]
    fn merge_and_bevel_are_one_step_each() {
        let mut s = Session::default();
        let id = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
        let original = s.map.get(id).unwrap().kind.brush().unwrap().clone();

        let top = vec![DVec3::new(0.0, 64.0, 64.0), DVec3::new(64.0, 64.0, 64.0)];
        s.merge_corners(&[(id, top)]).unwrap();
        let merged = s.map.get(id).unwrap().kind.brush().unwrap();
        assert_eq!(merged.vertices().unwrap().len(), 7);
        assert!(merged.vertices().unwrap().iter().any(|v| v.distance(DVec3::new(32.0, 64.0, 64.0)) < 1e-6));
        let flat: Vec<DVec3> = original.vertices().unwrap().into_iter().filter(|v| v.z == 64.0).collect();
        s.undo();
        assert!(s.merge_corners(&[(id, flat.into_iter().chain([DVec3::ZERO, DVec3::X * 64.0]).collect())]).is_err(), "a flattened brush is refused");
        assert_eq!(s.map.get(id).unwrap().kind.brush().unwrap(), &original);

        let edges = vec![(DVec3::new(0.0, 64.0, 64.0), DVec3::new(64.0, 64.0, 64.0))];
        let job = [(id, (original.clone(), edges))];
        for width in [8.0, 16.0, 24.0] {
            s.bevel_edges(&job, &Bevel { width, segments: 3, profile: 0.5 }, Some("bevel")).unwrap();
        }
        s.end_coalesce();
        assert_eq!(s.map.get(id).unwrap().kind.brush().unwrap().faces.len(), 9);
        assert!(s.bevel_edges(&job, &Bevel { width: 100.0, segments: 3, profile: 0.5 }, None).is_err());
        s.undo();
        assert_eq!(s.map.get(id).unwrap().kind.brush().unwrap(), &original);

        s.move_corners(&[(id, vec![(DVec3::splat(64.0), DVec3::new(64.0, 64.0, 96.0))])], None).unwrap();
        assert_eq!(s.map.get(id).unwrap().kind.brush().unwrap().bounds().unwrap().max.z, 96.0);
        assert!(s.move_corners(&[(id, original.vertices().unwrap().into_iter().map(|v| (v, DVec3::ZERO)).collect())], None).is_err());
    }
}
