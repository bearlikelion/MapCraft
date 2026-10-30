//! Vertex tool edits on brush components: moving, merging and deleting corners, dissolving faces, beveling edges.

use mapcraft_geom::DVec3;
use mapcraft_map::{Bevel, Brush, Face, NodeId, NodeKind, TexDefaults};

use crate::{EngineError, Session};

/// Corners within this squared distance count as the same corner.
const SAME: f64 = 1e-4;

fn near(a: DVec3, b: DVec3) -> bool {
    a.distance_squared(b) < SAME
}

/// Whether `winding` has exactly the points of `corners`.
fn same_corners(winding: &[DVec3], corners: &[DVec3]) -> bool {
    corners.iter().all(|c| winding.iter().any(|p| near(*p, *c))) && winding.iter().all(|p| corners.iter().any(|c| near(*p, *c)))
}

/// Whether two `corners` are still an edge of `b`, or three or more still a face.
fn has_component(b: &Brush, corners: &[DVec3]) -> bool {
    let Ok(solid) = b.solid() else { return false };
    solid.faces.iter().flatten().any(|w| match corners {
        [a, c] => w.0.iter().zip(w.0.iter().cycle().skip(1)).any(|(p, q)| (near(*p, *a) && near(*q, *c)) || (near(*p, *c) && near(*q, *a))),
        _ => same_corners(&w.0, corners),
    })
}

impl Session {
    /// Moves corners of each brush `(from, to)`; refused as a whole when any brush would not be a
    /// solid, or would lose an edge or face of `keep`, given by its corners where they end up.
    pub fn move_corners(&mut self, moves: &[(NodeId, Vec<(DVec3, DVec3)>)], keep: &[(NodeId, Vec<DVec3>)], coalesce: Option<&str>) -> Result<(), EngineError> {
        let items: Vec<(NodeId, (&[(DVec3, DVec3)], Vec<&[DVec3]>))> =
            moves.iter().map(|(id, m)| (*id, (m.as_slice(), keep.iter().filter(|(k, _)| k == id).map(|(_, c)| c.as_slice()).collect()))).collect();
        self.rebuild_brushes("Move vertices", &items, coalesce, |b, (m, keep), tex| {
            let mut verts = b.vertices()?;
            let mut changed = false;
            for v in &mut verts {
                if let Some((_, to)) = m.iter().find(|(from, _)| near(*from, *v)) {
                    *v = *to;
                    changed = true;
                }
            }
            if !changed {
                return Ok(None);
            }
            let moved = Brush::from_vertices(&verts, &b.faces, tex)?;
            if !keep.iter().all(|c| has_component(&moved, c)) {
                return Err(EngineError::Refused("the moved edge or face would merge into the faces around it"));
            }
            Ok(Some(moved))
        })
    }

    /// Removes the given corners of each brush, which closes up around the corners left.
    pub fn delete_corners(&mut self, corners: &[(NodeId, Vec<DVec3>)]) -> Result<(), EngineError> {
        self.rebuild_brushes("Delete vertices", corners, None, |b, c, tex| {
            let verts: Vec<DVec3> = b.vertices()?.into_iter().filter(|v| !c.iter().any(|p| near(*p, *v))).collect();
            Ok(Some(Brush::from_vertices(&verts, &b.faces, tex)?))
        })
    }

    /// Drops the faces of each brush given by their corners, so the faces around them grow to
    /// close the gap (Blender's dissolve faces).
    pub fn dissolve_faces(&mut self, faces: &[(NodeId, Vec<Vec<DVec3>>)]) -> Result<(), EngineError> {
        self.rebuild_brushes("Dissolve faces", faces, None, |b, list, _| {
            let solid = b.solid()?;
            let kept: Vec<Face> = b
                .faces
                .iter()
                .zip(&solid.faces)
                .filter(|(_, w)| !w.as_ref().is_some_and(|w| list.iter().any(|c| same_corners(&w.0, c))))
                .map(|(f, _)| f.clone())
                .collect();
            Ok((kept.len() < b.faces.len()).then(|| Brush::pruned(kept)).transpose()?)
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
            if let Some(new) = rebuild(b, item, &self.tex)?.map(|n| n.like(b)) {
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

        s.move_corners(&[(id, vec![(DVec3::splat(64.0), DVec3::new(64.0, 64.0, 96.0))])], &[], None).unwrap();
        assert_eq!(s.map.get(id).unwrap().kind.brush().unwrap().bounds().unwrap().max.z, 96.0);
        assert!(s.move_corners(&[(id, original.vertices().unwrap().into_iter().map(|v| (v, DVec3::ZERO)).collect())], &[], None).is_err());
    }

    /// A 64 unit box with its top +X edge chamfered 16 units.
    fn chamfered(s: &mut Session) -> NodeId {
        let id = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
        let mut pts: Vec<DVec3> = [0.0, 64.0].into_iter().flat_map(|y| [DVec3::new(0.0, y, 0.0), DVec3::new(64.0, y, 0.0), DVec3::new(0.0, y, 64.0)]).collect();
        pts.extend([0.0, 64.0].into_iter().flat_map(|y| [DVec3::new(48.0, y, 64.0), DVec3::new(64.0, y, 48.0)]));
        let b = Brush::from_vertices(&pts, &[], &s.tex).unwrap();
        s.set_kind(id, NodeKind::Brush(b), "Shape", None).unwrap();
        id
    }

    #[test]
    fn face_moves_that_merge_the_face_away_are_refused() {
        let mut s = Session::default();
        let id = chamfered(&mut s);
        let before = s.map.get(id).unwrap().kind.brush().unwrap().clone();
        let top: Vec<DVec3> = [0.0, 64.0].into_iter().flat_map(|y| [DVec3::new(0.0, y, 64.0), DVec3::new(48.0, y, 64.0)]).collect();
        let down = |d: f64| -> Vec<(DVec3, DVec3)> { top.iter().map(|p| (*p, *p - DVec3::Z * d)).collect() };
        let keep = |m: &[(DVec3, DVec3)]| vec![(id, m.iter().map(|(_, to)| *to).collect::<Vec<_>>())];
        let (all, half) = (down(16.0), down(8.0));
        assert!(s.move_corners(&[(id, all.clone())], &keep(&all), None).is_err(), "the top face would merge with the chamfer's foot");
        assert_eq!(s.map.get(id).unwrap().kind.brush().unwrap(), &before);
        s.move_corners(&[(id, half.clone())], &keep(&half), None).unwrap();
        assert_eq!(s.map.get(id).unwrap().kind.brush().unwrap().bounds().unwrap().max.z, 56.0, "a move that keeps the face goes through");
    }

    #[test]
    fn deleting_corners_and_dissolving_faces() {
        let mut s = Session::default();
        let id = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
        s.delete_corners(&[(id, vec![DVec3::splat(64.0)])]).unwrap();
        let b = s.map.get(id).unwrap().kind.brush().unwrap().clone();
        assert_eq!((b.vertices().unwrap().len(), b.faces.len()), (7, 7), "the corner is shaved off");
        assert_eq!(s.undo_label(), Some("Delete vertices"));
        let top: Vec<DVec3> = b.vertices().unwrap().into_iter().filter(|v| v.z == 64.0).collect();
        assert!(s.delete_corners(&[(id, top.into_iter().chain([DVec3::ZERO, DVec3::new(64.0, 0.0, 0.0)]).collect())]).is_err(), "a flat brush is refused");

        let id = chamfered(&mut s);
        let slope: Vec<DVec3> = [0.0, 64.0].into_iter().flat_map(|y| [DVec3::new(48.0, y, 64.0), DVec3::new(64.0, y, 48.0)]).collect();
        s.dissolve_faces(&[(id, vec![slope])]).unwrap();
        let b = s.map.get(id).unwrap().kind.brush().unwrap();
        assert_eq!((b.faces.len(), b.vertices().unwrap().len()), (6, 8), "dissolving the chamfer brings back the box corner");
        let bottom: Vec<DVec3> = b.vertices().unwrap().into_iter().filter(|v| v.z == 0.0).collect();
        assert!(s.dissolve_faces(&[(id, vec![bottom])]).is_err(), "a brush left open is refused");
    }
}
