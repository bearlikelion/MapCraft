use std::collections::{BTreeMap, BTreeSet, VecDeque};

use mapcraft_geom::{Aabb, DAffine2, DVec2, DVec3, EPSILON};
use mapcraft_map::{CONTENT_FLAGS, Face, FaceId, Map, NodeId, TexAxis, TexDefaults, share_edge};

use crate::{EngineError, Session};

/// A face of a brush, as the Face tool picks it.
pub type FacePick = (NodeId, FaceId);

/// What a face transfer (TrenchBroom's Alt+click) carries from one face to another.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaceTransfer {
    /// Material, attributes and the UV axes as they are, which stretches on faces turned far away.
    Projected,
    /// Material, attributes and the UV axes turned onto the target about the line the planes share.
    Rotated,
    Material,
}

impl FaceTransfer {
    pub fn label(self) -> &'static str {
        match self {
            Self::Projected => "Transfer material and alignment (projected)",
            Self::Rotated => "Transfer material and alignment (rotated)",
            Self::Material => "Transfer material",
        }
    }

    fn apply(self, f: &mut Face, src: &Face) {
        match self {
            Self::Material => {
                f.material = src.material.clone();
                return;
            }
            Self::Projected => (f.material, f.u, f.v, f.rotation) = (src.material.clone(), src.u, src.v, src.rotation),
            Self::Rotated => f.wrap_from(src),
        }
        let contents = f.attrs.remove(CONTENT_FLAGS);
        f.attrs = src.attrs.clone();
        f.attrs.remove(CONTENT_FLAGS);
        f.attrs.extend(contents.map(|c| (CONTENT_FLAGS.to_string(), c)));
    }
}

/// Every face of `brushes` with its polygon.
pub fn face_polygons<'a>(map: &'a Map, brushes: &BTreeSet<NodeId>) -> Vec<(FacePick, &'a Face, Vec<DVec3>)> {
    let mut out = Vec::new();
    for id in brushes {
        let Some(b) = map.get(*id).and_then(|n| n.kind.brush()) else { continue };
        let Ok(solid) = b.solid() else { continue };
        for (f, w) in b.faces.iter().zip(solid.faces) {
            if let Some(w) = w {
                out.push(((*id, f.id), f, w.0));
            }
        }
    }
    out
}

/// The picked faces in islands of faces that share an edge and a material. Each island starts at
/// the active face when it holds it and lists every other face after the neighbor it was reached from.
pub fn uv_islands(map: &Map, picks: &BTreeSet<FacePick>, active: Option<FacePick>) -> Vec<Vec<(FacePick, Option<FacePick>)>> {
    let brushes = picks.iter().map(|(id, _)| *id).collect();
    let faces: Vec<_> = face_polygons(map, &brushes).into_iter().filter(|(p, ..)| picks.contains(p)).collect();
    let boxes: Vec<Option<Aabb>> = faces.iter().map(|(_, _, w)| Aabb::from_points(w.iter().copied()).map(|b| b.expanded(EPSILON))).collect();
    let near = |i: usize, j: usize| boxes[i].zip(boxes[j]).is_some_and(|(a, b)| a.intersects(&b));
    let mut seen = vec![false; faces.len()];
    let seeds = faces.iter().position(|(p, ..)| Some(*p) == active).into_iter().chain(0..faces.len());
    let mut out = Vec::new();
    for seed in seeds {
        if std::mem::replace(&mut seen[seed], true) {
            continue;
        }
        let mut island = vec![(faces[seed].0, None)];
        let mut queue = VecDeque::from([seed]);
        while let Some(i) = queue.pop_front() {
            let (pick, face, poly) = &faces[i];
            for (j, (other, f, p)) in faces.iter().enumerate() {
                if !seen[j] && f.material == face.material && near(i, j) && share_edge(poly, p) {
                    seen[j] = true;
                    island.push((*other, Some(*pick)));
                    queue.push_back(j);
                }
            }
        }
        out.push(island);
    }
    out
}

impl Session {
    /// The active face while it is picked, else the first picked face.
    pub fn active_pick(&self) -> Option<FacePick> {
        self.active_face.filter(|a| self.face_selection.contains(a)).or_else(|| self.face_selection.first().copied())
    }

    fn picked_face(&self, (id, face): FacePick) -> Option<&Face> {
        self.map.get(id)?.kind.brush()?.faces.iter().find(|f| f.id == face)
    }

    /// Maps the texture of every picked face by `m`, an affine map of UV space measured in texels of
    /// a texture of `reference` size; `size` gives each material's texture size.
    pub fn transform_uvs(
        &mut self,
        label: &str,
        m: DAffine2,
        reference: DVec2,
        size: &dyn Fn(&str) -> DVec2,
        coalesce: Option<&str>,
    ) -> Result<(), EngineError> {
        let det = m.matrix2.determinant();
        if !m.is_finite() || det.abs() < 1e-9 {
            return Err(EngineError::Refused("that would squash the texture flat"));
        }
        let reference = reference.max(DVec2::ONE);
        self.edit_faces(label, coalesce, |f| {
            let k = size(&f.material).max(DVec2::ONE) / reference;
            f.transform_texels(DAffine2::from_scale(k) * m * DAffine2::from_scale(1.0 / k));
        })
    }

    /// Lays each island of picked faces out from its first face, the active one where it can, so
    /// the texture runs on across shared edges (Blender's Follow Active Quads).
    pub fn unwrap_faces(&mut self) -> Result<(), EngineError> {
        let islands = uv_islands(&self.map, &self.face_selection, self.active_pick());
        let mut faces: BTreeMap<FacePick, Face> = islands.iter().flatten().filter_map(|(p, _)| Some((*p, self.picked_face(*p)?.clone()))).collect();
        for (pick, from) in islands.iter().flatten() {
            let Some(src) = from.and_then(|p| faces.get(&p)).cloned() else { continue };
            if let Some(f) = faces.get_mut(pick) {
                f.wrap_from(&src);
            }
        }
        let by_id: BTreeMap<FaceId, Face> = faces.into_values().map(|f| (f.id, f)).collect();
        self.edit_faces("Unwrap", None, |f| {
            if let Some(n) = by_id.get(&f.id) {
                *f = n.clone();
            }
        })
    }

    /// Projects one texture onto every picked face along `dir`, upright as a camera looking that
    /// way sees it, at the active face's scale.
    pub fn project_faces(&mut self, dir: DVec3) -> Result<(), EngineError> {
        let dir = dir.normalize_or_zero();
        if dir == DVec3::ZERO {
            return Err(EngineError::Refused("the view has no direction"));
        }
        let right = dir.cross(DVec3::Z).normalize_or(DVec3::X);
        let active = self.active_pick().and_then(|p| self.picked_face(p)).ok_or(EngineError::NothingSelected)?;
        let scale = |s: f64| if s.abs() > 1e-9 { s } else { self.tex.scale };
        let u = TexAxis { axis: right, offset: 0.0, scale: scale(active.u.scale) };
        let v = TexAxis { axis: dir.cross(right), offset: 0.0, scale: scale(active.v.scale) };
        self.edit_faces("Project from view", None, |f| (f.u, f.v, f.rotation) = (u, v, 0.0))
    }

    /// Gives every picked face the mapping a new face gets: world aligned at the default scale.
    pub fn reset_uvs(&mut self) -> Result<(), EngineError> {
        let tex = TexDefaults { material: String::new(), scale: self.tex.scale };
        self.edit_faces("Reset UVs", None, |f| {
            let fresh = Face::new(f.points, &tex);
            (f.u, f.v, f.rotation) = (fresh.u, fresh.v, 0.0);
        })
    }

    /// Gives the faces `to` what `mode` carries from face `from`; their content flags always stay.
    pub fn transfer_face(&mut self, from: FacePick, to: &BTreeSet<FacePick>, mode: FaceTransfer, coalesce: Option<&str>) -> Result<(), EngineError> {
        let src = self.picked_face(from).cloned().ok_or(EngineError::NothingSelected)?;
        self.edit_face_set(to, mode.label(), coalesce, |f| mode.apply(f, &src))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(s: &Session, p: FacePick) -> Face {
        s.picked_face(p).unwrap().clone()
    }

    /// The largest texel mismatch at corners each face shares with the face it was laid out from.
    fn seam(s: &Session) -> f64 {
        let brushes = s.face_selection.iter().map(|(id, _)| *id).collect();
        let polys: BTreeMap<FacePick, Vec<DVec3>> = face_polygons(&s.map, &brushes).into_iter().map(|(p, _, w)| (p, w)).collect();
        let mut worst = 0.0f64;
        for (pick, from) in uv_islands(&s.map, &s.face_selection, s.active_pick()).into_iter().flatten() {
            let Some(from) = from else { continue };
            let (a, b) = (face(s, pick), face(s, from));
            for p in polys[&pick].iter().filter(|p| polys[&from].iter().any(|q| q.distance(**p) < 1e-6)) {
                worst = worst.max((a.texel(*p) - b.texel(*p)).length());
            }
        }
        worst
    }

    fn picks(s: &Session, id: NodeId, keep: impl Fn(DVec3) -> bool) -> BTreeSet<FacePick> {
        s.map.get(id).unwrap().kind.brush().unwrap().faces.iter().filter(|f| keep(f.plane().unwrap().normal)).map(|f| (id, f.id)).collect()
    }

    #[test]
    fn islands_follow_shared_edges_and_materials() {
        let mut s = Session::default();
        s.tex.material = "dev".into();
        let a = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
        let b = s.create_brush(Aabb::new(DVec3::new(64.0, 0.0, 0.0), DVec3::new(192.0, 128.0, 64.0))).unwrap();
        s.create_brush(Aabb::new(DVec3::splat(500.0), DVec3::splat(564.0))).unwrap();
        let mut top: BTreeSet<FacePick> = picks(&s, a, |n| n.z > 0.5);
        top.extend(picks(&s, b, |n| n.z > 0.5));
        assert_eq!(uv_islands(&s.map, &top, None).len(), 1, "tops of side by side brushes meet along part of an edge");
        s.face_selection = picks(&s, b, |n| n.z > 0.5);
        s.tex.material = "other".into();
        s.apply_material().unwrap();
        assert_eq!(uv_islands(&s.map, &top, None).len(), 2, "a different material starts a new island");
    }

    #[test]
    fn unwrap_runs_the_texture_on_across_edges() {
        let mut s = Session::default();
        s.tex.material = "dev".into();
        assert!(matches!(crate::execute(&mut s, "uv.unwrap"), Err(EngineError::Disabled(_))));
        let a = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::new(64.0, 96.0, 128.0))).unwrap();
        s.face_selection = picks(&s, a, |_| true);
        s.edit_faces("Scramble", None, |f| {
            f.rotate_texture(17.0 * f.id.0 as f64);
            f.u.offset += 3.0 * f.id.0 as f64;
        })
        .unwrap();
        assert!(seam(&s) > 1.0, "scrambled faces do not line up");
        let active = *s.face_selection.iter().nth(2).unwrap();
        s.active_face = Some(active);
        let before = face(&s, active);
        crate::execute(&mut s, "uv.unwrap").unwrap();
        assert!(seam(&s) < 1e-9, "{}", seam(&s));
        assert_eq!(face(&s, active), before, "the active face keeps its mapping");
        assert_eq!(uv_islands(&s.map, &s.face_selection, Some(active))[0][0].0, active);
        s.undo();
        assert!(seam(&s) > 1.0);
    }

    #[test]
    fn transforms_round_trip_and_keep_islands_whole() {
        let mut s = Session::default();
        s.tex.material = "dev".into();
        let a = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
        s.tex.material = "big".into();
        let b = s.create_brush(Aabb::new(DVec3::splat(200.0), DVec3::splat(264.0))).unwrap();
        let mut picked = picks(&s, a, |n| n.z > 0.5 || n.x > 0.5);
        picked.extend(picks(&s, b, |n| n.z > 0.5));
        s.face_selection = picked.clone();
        s.unwrap_faces().unwrap();
        let size = |m: &str| if m == "big" { DVec2::splat(128.0) } else { DVec2::splat(64.0) };
        let reference = DVec2::splat(64.0);
        let brushes = picked.iter().map(|(id, _)| *id).collect();
        let shown = |s: &Session| -> Vec<Vec<DVec2>> {
            face_polygons(&s.map, &brushes)
                .into_iter()
                .filter(|(p, ..)| picked.contains(p))
                .map(|(_, f, w)| w.iter().map(|p| f.uv(*p, size(&f.material)) * reference).collect())
                .collect()
        };
        let before = shown(&s);
        let m = DAffine2::from_translation(DVec2::new(30.0, 20.0)) * DAffine2::from_scale_angle_translation(DVec2::new(1.5, 0.5), 0.4, DVec2::new(-3.0, 7.0));
        s.transform_uvs("Rotate UVs", m, reference, &size, Some("uv")).unwrap();
        for (after, before) in shown(&s).iter().zip(&before) {
            for (a, b) in after.iter().zip(before) {
                assert!(a.abs_diff_eq(m.transform_point2(*b), 1e-9), "{a:?} {:?}", m.transform_point2(*b));
            }
        }
        assert!(seam(&s) < 1e-9, "the island moved as one: {}", seam(&s));
        s.transform_uvs("Rotate UVs", m.inverse(), reference, &size, Some("uv")).unwrap();
        s.end_coalesce();
        s.undo();
        assert_eq!(shown(&s), before, "a coalesced run is one undo step");
        assert!(s.transform_uvs("Scale UVs", DAffine2::from_scale(DVec2::new(1.0, 0.0)), reference, &size, None).is_err());
    }

    #[test]
    fn displaced_faces_keep_their_grid() {
        let mut s = Session::default();
        let a = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
        s.face_selection = picks(&s, a, |n| n.z > 0.5);
        s.displace_faces(2).unwrap();
        s.face_selection = picks(&s, a, |_| true);
        s.unwrap_faces().unwrap();
        s.transform_uvs("Move UVs", DAffine2::from_translation(DVec2::X), DVec2::ONE, &|_| DVec2::ONE, None).unwrap();
        s.project_faces(DVec3::NEG_Z).unwrap();
        s.reset_uvs().unwrap();
        assert!(s.map.get(a).unwrap().kind.brush().unwrap().faces.iter().any(|f| f.disp.is_some()));
    }

    #[test]
    fn projections_and_reset() {
        let mut s = Session::default();
        s.tex = TexDefaults { material: "dev".into(), scale: 0.25 };
        let a = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
        s.face_selection = picks(&s, a, |_| true);
        let all = |s: &Session| s.face_selection.iter().map(|p| face(s, *p)).collect::<Vec<_>>();

        s.view_dir = DVec3::new(1.0, 2.0, -1.0);
        crate::execute(&mut s, "uv.project_from_view").unwrap();
        let (right, down) = (DVec3::new(2.0, -1.0, 0.0).normalize(), DVec3::new(-1.0, -2.0, -5.0).normalize());
        for f in all(&s) {
            assert!(f.u.axis.abs_diff_eq(right, 1e-9) && f.v.axis.abs_diff_eq(down, 1e-9), "{:?} {:?}", f.u.axis, f.v.axis);
        }

        crate::execute(&mut s, "uv.cube_projection").unwrap();
        for f in all(&s) {
            let mut w = f.clone();
            w.align_to_world();
            assert_eq!(f, w);
        }

        s.edit_faces("Shift", None, |f| f.u.offset = 9.0).unwrap();
        s.edit_faces("Shift again", None, |f| f.u.offset = 9.0).unwrap();
        assert_eq!(s.undo_label(), Some("Shift"), "an edit that changes nothing adds no undo step");
        crate::execute(&mut s, "uv.reset").unwrap();
        for f in all(&s) {
            let fresh = Face::new(f.points, &s.tex);
            assert_eq!((f.u, f.v, f.rotation), (fresh.u, fresh.v, 0.0));
        }
    }

    fn one(s: &Session, id: NodeId, keep: impl Fn(DVec3) -> bool) -> FacePick {
        *picks(s, id, keep).first().unwrap()
    }

    #[test]
    fn transfers_copy_the_alignment_or_only_the_material_and_keep_content_flags() {
        let mut s = Session::default();
        s.tex.material = "dev".into();
        let a = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
        let (top, side, bottom) = (one(&s, a, |n| n.z > 0.5), one(&s, a, |n| n.x > 0.5), one(&s, a, |n| n.z < -0.5));
        s.face_selection = BTreeSet::from([top]);
        s.edit_faces("Look", None, |f| {
            f.material = "ramps/orange".into();
            f.rotate_texture(30.0);
            f.u.offset = 5.0;
            f.v.scale = 0.5;
            f.attrs = BTreeMap::from([("lightmapscale".into(), "8".into()), (CONTENT_FLAGS.into(), "1".into())]);
        })
        .unwrap();
        s.face_selection = BTreeSet::from([side]);
        s.edit_faces("Flags", None, |f| {
            f.attrs.insert(CONTENT_FLAGS.into(), "32".into());
        })
        .unwrap();
        let (src, before) = (face(&s, top), face(&s, side));

        s.transfer_face(top, &BTreeSet::from([side]), FaceTransfer::Material, None).unwrap();
        let f = face(&s, side);
        assert_eq!(f.material, "ramps/orange");
        assert_eq!((f.u, f.v, f.rotation, &f.attrs), (before.u, before.v, before.rotation, &before.attrs), "material only keeps the alignment");

        s.transfer_face(top, &BTreeSet::from([side]), FaceTransfer::Projected, None).unwrap();
        let f = face(&s, side);
        assert_eq!((f.u, f.v, f.rotation), (src.u, src.v, src.rotation), "the axes are copied as they are");
        assert_eq!(f.attrs.get("lightmapscale").map(String::as_str), Some("8"));
        assert_eq!(f.attrs.get(CONTENT_FLAGS).map(String::as_str), Some("32"), "content flags stay");
        assert_eq!(f.points, before.points);
        assert_eq!(s.undo_label(), Some(FaceTransfer::Projected.label()));

        s.transfer_face(top, &BTreeSet::from([bottom]), FaceTransfer::Rotated, None).unwrap();
        assert!(!face(&s, bottom).attrs.contains_key(CONTENT_FLAGS), "nor do a target's missing ones arrive");
        assert!(s.transfer_face((a, FaceId(9999)), &BTreeSet::from([side]), FaceTransfer::Projected, None).is_err());
    }

    #[test]
    fn rotated_transfers_run_on_around_corners_and_chain_from_the_last_face() {
        let mut s = Session::default();
        s.tex.material = "dev".into();
        let a = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
        let (top, side, bottom) = (one(&s, a, |n| n.z > 0.5), one(&s, a, |n| n.x > 0.5), one(&s, a, |n| n.z < -0.5));
        s.face_selection = BTreeSet::from([top]);
        s.edit_faces("Look", None, |f| {
            f.rotate_texture(30.0);
            f.u.scale = 0.5;
            f.v.offset = 7.0;
        })
        .unwrap();
        let before = (face(&s, side), face(&s, bottom));

        // A drag: the bottom takes the texture from the side, the face transferred to before it.
        s.transfer_face(top, &BTreeSet::from([side]), FaceTransfer::Rotated, Some("transfer")).unwrap();
        s.transfer_face(side, &BTreeSet::from([bottom]), FaceTransfer::Rotated, Some("transfer")).unwrap();
        let (t, x, b) = (face(&s, top), face(&s, side), face(&s, bottom));
        for y in [0.0, 16.0, 64.0] {
            let (upper, lower) = (DVec3::new(64.0, y, 64.0), DVec3::new(64.0, y, 0.0));
            for d in [0.0, 8.0, 64.0] {
                assert!(x.texel(upper - DVec3::Z * d).abs_diff_eq(t.texel(upper + DVec3::X * d), 1e-9), "the side reads as the top unfolded, at its density");
                assert!(b.texel(lower - DVec3::X * d).abs_diff_eq(x.texel(lower - DVec3::Z * d), 1e-9), "the bottom as the side unfolded");
            }
        }
        s.undo();
        assert_eq!((face(&s, side), face(&s, bottom)), before, "the whole drag is one undo step");

        s.transfer_face(top, &BTreeSet::from([side]), FaceTransfer::Projected, None).unwrap();
        let p = face(&s, side);
        assert!(p.texel(DVec3::new(64.0, 8.0, 0.0)).abs_diff_eq(p.texel(DVec3::new(64.0, 8.0, 64.0)), 1e-9), "projected, the texture stretches down the side");
    }
}
