//! Paths and curve groups: picked faces swept along a path or spun about an axis, and copies
//! placed along a path, rebuilt in the same undo step whenever a source or a setting changes.

use std::collections::BTreeSet;

use mapcraft_format::Fragment;
use mapcraft_geom::{DMat4, DVec3};
use mapcraft_map::curve::{self, Profile};
use mapcraft_map::{Changes, CurveOp, Edit, EditError, FaceId, Group, Map, NodeId, NodeKind, Path, PathPoint};

use crate::ops::insert_fragments;
use crate::{EngineError, Session};

/// Most nodes an array along a path may add.
const MAX_ARRAY_NODES: usize = 20_000;

pub fn curve_of(kind: &NodeKind) -> Option<&CurveOp> {
    match kind {
        NodeKind::Group(Group { curve: Some(op), .. }) => Some(op),
        _ => None,
    }
}

/// Whether `id` is inside a curve group, whose contents get new ids each rebuild.
fn in_curve_group(map: &Map, id: NodeId) -> bool {
    map.ancestors(id).iter().any(|a| map.get(*a).is_some_and(|n| curve_of(&n.kind).is_some()))
}

fn fragment(e: &Edit, id: NodeId, depth: usize) -> Option<Fragment> {
    let n = e.node(id).filter(|_| depth < 64)?;
    Some(Fragment { kind: n.kind.clone(), children: n.children.iter().filter_map(|c| fragment(e, *c, depth + 1)).collect() })
}

fn profiles(e: &Edit, faces: &[(NodeId, FaceId)]) -> Result<Vec<Profile>, EngineError> {
    faces
        .iter()
        .map(|(id, f)| e.node(*id).and_then(|n| n.kind.brush()).and_then(|b| Profile::of(b, *f)))
        .collect::<Option<Vec<_>>>()
        .ok_or(EngineError::Refused("a picked face is gone; pick the faces again and make a new one"))
}

fn path(e: &Edit, id: NodeId) -> Result<Path, EngineError> {
    match e.node(id).map(|n| &n.kind) {
        Some(NodeKind::Path(p)) => Ok(p.clone()),
        _ => Err(EngineError::Refused("its path is gone")),
    }
}

/// The middle of the box around `frags`, point entities and prefabs counted by their origins.
fn center(frags: &[Fragment]) -> DVec3 {
    fn points(f: &Fragment, out: &mut Vec<DVec3>) {
        match &f.kind {
            NodeKind::Brush(b) => out.extend(b.bounds().ok().map(|b| [b.min, b.max]).into_iter().flatten()),
            NodeKind::Entity(e) if f.children.is_empty() => out.extend(e.origin()),
            NodeKind::Prefab(p) => out.push(p.origin),
            NodeKind::Path(p) => out.extend(p.bounds().map(|b| [b.min, b.max]).into_iter().flatten()),
            _ => {}
        }
        f.children.iter().for_each(|c| points(c, out));
    }
    let mut pts = Vec::new();
    frags.iter().for_each(|f| points(f, &mut pts));
    mapcraft_geom::Aabb::from_points(pts).map_or(DVec3::ZERO, |b| b.center())
}

/// The path a sweep or array follows and the middle of what it carries along.
fn anchor(e: &Edit, op: &CurveOp) -> Result<Option<(Path, DVec3)>, EngineError> {
    Ok(match op {
        CurveOp::Sweep { path: p, faces, .. } => Some((path(e, *p)?, curve::centroid(&profiles(e, faces)?))),
        CurveOp::Array { path: p, nodes, .. } => Some((path(e, *p)?, center(&nodes.iter().filter_map(|n| fragment(e, *n, 0)).collect::<Vec<_>>()))),
        CurveOp::Spin { .. } => None,
    })
}

/// `op` with its bind set so what it builds starts where its sources now are.
fn bound(e: &Edit, mut op: CurveOp) -> Result<CurveOp, EngineError> {
    if let Some((p, c)) = anchor(e, &op)? {
        let b = curve::start_frame(&p).map_err(EngineError::Refused)?.inverse() * DMat4::from_translation(c);
        if let CurveOp::Sweep { bind, .. } | CurveOp::Array { bind, .. } = &mut op {
            *bind = b;
        }
    }
    Ok(op)
}

/// What `op` builds from the sources as `e` sees them: nodes and the move that places them.
fn build(e: &Edit, op: &CurveOp) -> Result<Vec<(DMat4, Vec<Fragment>)>, EngineError> {
    let brushes =
        |b: Vec<mapcraft_map::Brush>| vec![(DMat4::IDENTITY, b.into_iter().map(|b| Fragment { kind: NodeKind::Brush(b), children: Vec::new() }).collect())];
    // The bound sources go to the path's start as it is now, so the result follows the path.
    let place = |p: &Path, bind: &DMat4, c: DVec3| -> Result<DMat4, EngineError> {
        Ok(curve::start_frame(p).map_err(EngineError::Refused)? * *bind * DMat4::from_translation(-c))
    };
    Ok(match op {
        CurveOp::Sweep { path: p, faces, segments, twist, scale, bind } => {
            let (p, profiles) = (path(e, *p)?, profiles(e, faces)?);
            let at = place(&p, bind, curve::centroid(&profiles))?;
            brushes(curve::sweep(&p, &profiles, *segments, *twist, *scale, &at).map_err(EngineError::Refused)?)
        }
        CurveOp::Spin { faces, axis, pivot, angle, steps } => {
            brushes(curve::spin(&profiles(e, faces)?, *axis, *pivot, *angle, *steps).map_err(EngineError::Refused)?)
        }
        CurveOp::Array { path: p, nodes, count, spacing, yaw_only, bind } => {
            let src: Vec<Fragment> = nodes.iter().filter_map(|n| fragment(e, *n, 0)).collect();
            let p = path(e, *p)?;
            let at = place(&p, bind, center(&src))?;
            let places = curve::placements(&p, *count, *spacing, *yaw_only).map_err(EngineError::Refused)?;
            fn size(f: &[Fragment]) -> usize {
                f.iter().map(|f| 1 + size(&f.children)).sum()
            }
            if size(&src).saturating_mul(places.len()) > MAX_ARRAY_NODES {
                return Err(EngineError::Refused("that would make too many copies; use fewer"));
            }
            places.into_iter().map(|m| (m * at, src.clone())).collect()
        }
    })
}

/// Replaces `group`'s children with what `op` builds and stores `op` on it.
fn rebuild(e: &mut Edit, group: NodeId, op: &CurveOp) -> Result<(), EngineError> {
    let built = build(e, op)?;
    let node = e.node(group).cloned().ok_or(EditError::Missing(group))?;
    let NodeKind::Group(g) = node.kind else { return Err(EngineError::Refused("only a group can hold a curve")) };
    // A source moved inside the group survives its rebuilds.
    let mut keep = BTreeSet::new();
    for s in op.sources() {
        let mut at = Some(s);
        while let Some(id) = at.filter(|id| keep.len() < 100_000 && keep.insert(*id)) {
            at = e.node(id).and_then(|n| n.parent);
        }
    }
    for child in node.children.iter().filter(|c| !keep.contains(*c)) {
        e.remove(*child)?;
    }
    e.set_kind(group, NodeKind::Group(Group { curve: Some(Box::new(op.clone())), ..g }))?;
    for (m, frags) in built {
        insert_fragments(e, group, frags, &m, true)?;
    }
    Ok(())
}

/// Rebuilds the curve groups whose sources are among `touched`, as changes to apply after them;
/// a group with a source missing keeps what it has.
pub(crate) fn follow(map: &Map, touched: &BTreeSet<NodeId>) -> Option<Changes> {
    let groups: Vec<(NodeId, CurveOp)> = map.nodes().filter_map(|(id, n)| Some((id, curve_of(&n.kind)?.clone()))).collect();
    if groups.is_empty() {
        return None;
    }
    let mut e = map.edit();
    let mut any = false;
    for (g, op) in groups {
        let sources = op.sources();
        if touched.contains(&g) || sources.iter().any(|s| map.get(*s).is_none()) {
            continue;
        }
        if sources.iter().any(|s| map.subtree(*s).iter().any(|n| touched.contains(n))) {
            any |= rebuild(&mut e, g, &op).is_ok();
        }
    }
    any.then(|| e.finish())
}

impl Session {
    /// Makes a path through `points` under the active parent and selects it.
    pub fn create_path(&mut self, points: Vec<PathPoint>) -> Result<NodeId, EngineError> {
        let mut e = self.map.edit();
        let id = e.insert(self.active_parent, NodeKind::Path(Path { points, closed: false }))?;
        let c = e.finish();
        self.apply("Create path", c);
        self.selection = BTreeSet::from([id]);
        Ok(id)
    }

    /// The path curve tools use: the selected one, else the newest.
    pub fn curve_path(&self) -> Option<NodeId> {
        let is_path = |id: &NodeId| self.map.get(*id).is_some_and(|n| matches!(n.kind, NodeKind::Path(_)));
        self.selection.iter().copied().find(is_path).or_else(|| self.map.nodes().map(|(id, _)| id).filter(is_path).last())
    }

    fn picked_profiles(&self) -> Result<Vec<(NodeId, FaceId)>, EngineError> {
        if self.face_selection.is_empty() {
            return Err(EngineError::Refused("pick faces with the Face tool first"));
        }
        if self.face_selection.iter().any(|(id, _)| in_curve_group(&self.map, *id)) {
            return Err(EngineError::Refused("a curve group's own brushes are rebuilt; make it real before using its faces"));
        }
        Ok(self.face_selection.iter().copied().collect())
    }

    /// Sweeps the picked faces along the selected or newest path.
    pub fn sweep_faces(&mut self) -> Result<NodeId, EngineError> {
        let faces = self.picked_profiles()?;
        let path = self.curve_path().ok_or(EngineError::Refused("draw a path first"))?;
        self.create_curve(CurveOp::Sweep { path, faces, segments: 8, twist: 0.0, scale: 1.0, bind: DMat4::IDENTITY })
    }

    /// Spins the picked faces a full turn through `pivot`: about Z, or for a level face about the
    /// level axis that swings it up over the pivot.
    pub fn spin_faces(&mut self, pivot: DVec3) -> Result<NodeId, EngineError> {
        let faces = self.picked_profiles()?;
        let e = self.map.edit();
        let p = profiles(&e, &faces)?;
        let n = p.first().and_then(|p| p.face.plane()).map_or(DVec3::Z, |pl| pl.normal);
        let out = (curve::centroid(&p) - pivot).with_z(0.0);
        let axis = if n.z.abs() < 0.99 { DVec3::Z } else { n.cross(out).try_normalize().unwrap_or(DVec3::X) };
        self.create_curve(CurveOp::Spin { faces, axis, pivot, angle: 360.0, steps: 16 })
    }

    /// Copies the selection, less any path in it, along the selected or newest path.
    pub fn array_along_path(&mut self) -> Result<NodeId, EngineError> {
        let path = self.curve_path().ok_or(EngineError::Refused("draw a path first"))?;
        let nodes: Vec<NodeId> =
            self.selection.iter().copied().filter(|id| *id != path && !self.map.ancestors(*id).iter().any(|a| self.selection.contains(a))).collect();
        if nodes.is_empty() {
            return Err(EngineError::Refused("select what to copy along the path"));
        }
        if nodes.iter().any(|n| in_curve_group(&self.map, *n)) {
            return Err(EngineError::Refused("a curve group's own brushes are rebuilt; make it real before copying them"));
        }
        self.create_curve(CurveOp::Array { path, nodes, count: 8, spacing: 0.0, yaw_only: false, bind: DMat4::IDENTITY })
    }

    fn create_curve(&mut self, op: CurveOp) -> Result<NodeId, EngineError> {
        let mut e = self.map.edit();
        let op = bound(&e, op)?;
        let group = e.insert(self.active_parent, NodeKind::Group(Group { name: op.name().into(), ..Group::default() }))?;
        rebuild(&mut e, group, &op)?;
        let c = e.finish();
        self.apply(op.name(), c);
        self.selection = BTreeSet::from([group]);
        Ok(group)
    }

    /// Gives a curve group new settings and rebuilds it.
    pub fn set_curve(&mut self, group: NodeId, op: CurveOp, coalesce: Option<&str>) -> Result<(), EngineError> {
        let mut e = self.map.edit();
        rebuild(&mut e, group, &op)?;
        let c = e.finish();
        self.apply_coalesced(&format!("Edit {}", op.name().to_lowercase()), c, coalesce);
        Ok(())
    }

    /// Turns the selected curve groups into plain groups that keep their brushes.
    pub fn make_curve_real(&mut self) -> Result<(), EngineError> {
        let groups: Vec<NodeId> = self.selection.iter().copied().filter(|id| self.map.get(*id).is_some_and(|n| curve_of(&n.kind).is_some())).collect();
        if groups.is_empty() {
            return Err(EngineError::NothingSelected);
        }
        let mut e = self.map.edit();
        for g in groups {
            if let Some(NodeKind::Group(group)) = e.node(g).map(|n| n.kind.clone()) {
                e.set_kind(g, NodeKind::Group(Group { curve: None, ..group }))?;
            }
        }
        let c = e.finish();
        self.apply("Make curve real", c);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use mapcraft_geom::Aabb;
    use mapcraft_map::ROOT;

    use super::*;

    fn op(s: &Session, g: NodeId) -> CurveOp {
        curve_of(&s.map.get(g).unwrap().kind).unwrap().clone()
    }

    fn kids(s: &Session, g: NodeId) -> Vec<Aabb> {
        s.map.get(g).unwrap().children.iter().filter_map(|c| s.map.get(*c)?.kind.brush()?.bounds().ok()).collect()
    }

    fn setup() -> (Session, NodeId, NodeId) {
        let mut s = Session::default();
        let b = s.create_brush(Aabb::new(DVec3::new(-16.0, -32.0, 0.0), DVec3::new(0.0, 32.0, 16.0))).unwrap();
        let p = |pos: DVec3, d: DVec3| PathPoint { pos, handle_in: pos - d, handle_out: pos + d };
        let path = s.create_path(vec![p(DVec3::ZERO, DVec3::X * 128.0), p(DVec3::new(512.0, 256.0, 0.0), DVec3::X * 128.0)]).unwrap();
        let face = s.map.get(b).unwrap().kind.brush().unwrap().faces.iter().find(|f| f.plane().unwrap().normal.x > 0.99).unwrap().id;
        s.face_selection = BTreeSet::from([(b, face)]);
        (s, b, path)
    }

    #[test]
    fn sweeps_follow_their_path_and_settings_in_one_step_each() {
        let (mut s, b, path) = setup();
        let g = s.sweep_faces().unwrap();
        assert_eq!(s.selection, BTreeSet::from([g]));
        assert_eq!(kids(&s, g).len(), 8);
        let original = s.map.get(b).cloned();

        for n in [4, 12] {
            let mut edited = op(&s, g);
            if let CurveOp::Sweep { segments, .. } = &mut edited {
                *segments = n;
            }
            s.set_curve(g, edited, Some("edit")).unwrap();
        }
        assert_eq!(kids(&s, g).len(), 12);
        s.undo();
        assert_eq!(kids(&s, g).len(), 8, "a run of setting changes undoes in one step");

        let NodeKind::Path(mut moved) = s.map.get(path).unwrap().kind.clone() else { panic!() };
        moved.points[1].pos.z += 128.0;
        moved.points[1].handle_in.z += 128.0;
        moved.points[1].handle_out.z += 128.0;
        s.set_kind(path, NodeKind::Path(moved), "Edit path", None).unwrap();
        let top = kids(&s, g).iter().map(|b| b.max.z).fold(0.0, f64::max);
        assert!(top > 128.0, "editing the path rebuilds the sweep");
        assert_eq!(s.map.get(b).cloned(), original, "the profile brush is untouched");
        s.undo();
        assert!(kids(&s, g).iter().all(|b| b.max.z < 16.0 + 1e-6), "the path edit and the rebuild undo together");

        let low = |s: &Session| kids(s, g).iter().map(|b| b.min.x).fold(f64::MAX, f64::min);
        let x = low(&s);
        s.selection = BTreeSet::from([path]);
        s.translate_selection(DVec3::X * 100.0, None).unwrap();
        assert!((low(&s) - x - 100.0).abs() < 1e-6, "moving the path carries the sweep");
        s.selection = BTreeSet::from([b]);
        s.translate_selection(DVec3::Y * 50.0, None).unwrap();
        assert!((low(&s) - x - 100.0).abs() < 1e-6, "moving the profile alone leaves it on the path");
        s.selection = BTreeSet::from([b, path]);
        s.translate_selection(DVec3::X * 10.0, None).unwrap();
        assert!((low(&s) - x - 110.0).abs() < 1e-6, "moving both moves it once");
        s.selection = BTreeSet::from([b]);
        s.delete_selection().unwrap();
        assert_eq!(kids(&s, g).len(), 8, "deleting a profile keeps the result");
        let mut e = s.map.edit();
        e.remove(path).unwrap();
        let c = e.finish();
        s.apply("Delete path", c);
        assert_eq!(kids(&s, g).len(), 8);

        s.selection = BTreeSet::from([g]);
        s.make_curve_real().unwrap();
        assert!(matches!(&s.map.get(g).unwrap().kind, NodeKind::Group(Group { curve: None, .. })));
    }

    #[test]
    fn spins_and_arrays() {
        let (mut s, b, path) = setup();
        let ring = s.spin_faces(DVec3::new(-64.0, 0.0, 0.0)).unwrap();
        assert_eq!(kids(&s, ring).len(), 16);
        assert!(
            s.set_curve(
                ring,
                CurveOp::Spin { faces: s.face_selection.iter().copied().collect(), axis: DVec3::Y, pivot: DVec3::new(0.0, 0.0, -64.0), angle: 180.0, steps: 6 },
                None
            )
            .is_ok()
        );
        assert_eq!(kids(&s, ring).len(), 6);

        s.selection = BTreeSet::from([b, path]);
        let arr = s.array_along_path().unwrap();
        let copies = s.map.get(arr).unwrap().children.len();
        assert_eq!(copies, 8);
        let ends = |s: &Session| kids(s, arr).last().unwrap().center();
        assert!(ends(&s).distance(DVec3::new(512.0, 256.0, 8.0)) < 40.0, "the last copy sits at the path's end");
        s.selection = BTreeSet::from([path]);
        s.translate_selection(DVec3::new(0.0, 0.0, 64.0), None).unwrap();
        assert!((ends(&s).z - 72.0).abs() < 1e-6, "copies follow the path");
        s.selection = BTreeSet::from([arr]);
        s.duplicate_selection(DVec3::X * 32.0, None).unwrap();
        let dup = *s.selection.first().unwrap();
        assert!(curve_of(&s.map.get(dup).unwrap().kind).is_none(), "a duplicate does not follow");
        assert!(s.map.get(ROOT).unwrap().children.contains(&dup));
    }
}
