use mapcraft_map::mirror::{insert, is_mirror_group, mirrored};
use mapcraft_map::{Axis, Group, Mirror, NodeId, NodeKind};

use crate::{EngineError, Session};

impl Session {
    /// The default plane on `axis`: through the point of `roots` nearest the map origin, so a half built up to the middle mirrors across its inner edge.
    pub fn mirror_offset(&self, roots: &[NodeId], axis: Axis) -> Option<f64> {
        let b = self.bounds_of(roots.iter().copied())?;
        let i = axis.index();
        Some(0.0_f64.max(b.min[i]).min(b.max[i]))
    }

    /// Makes the selection a mirror group across X; a lone plain group takes the plane itself.
    pub fn make_mirror_group(&mut self) -> Result<NodeId, EngineError> {
        let ids = self.require_selection()?;
        let offset = self.mirror_offset(&ids, Axis::X).unwrap_or(0.0);
        let mirror = Some(Mirror { axis: Axis::X, offset, texture_lock: self.texture_lock });
        if let [id] = ids[..]
            && let Some(NodeKind::Group(g)) = self.map.get(id).map(|n| &n.kind)
            && g.mirror.is_none()
        {
            let kind = NodeKind::Group(Group { mirror, ..g.clone() });
            self.set_kind(id, kind, "Make mirror group", None)?;
            return Ok(id);
        }
        self.wrap_selection(Group { name: "Mirror".into(), generator: None, mirror }, "Make mirror group")
    }

    /// Turns the selected mirror groups' derived halves into ordinary nodes and stops mirroring.
    pub fn make_mirror_real(&mut self) -> Result<(), EngineError> {
        let groups: Vec<NodeId> = self.selection.iter().copied().filter(|id| is_mirror_group(&self.map, *id)).collect();
        if groups.is_empty() {
            return Err(EngineError::NothingSelected);
        }
        let mut e = self.map.edit();
        for g in groups {
            let Some(NodeKind::Group(group)) = e.node(g).map(|n| n.kind.clone()) else { continue };
            insert(&mut e, &self.map, g, mirrored(&self.map, g))?;
            e.set_kind(g, NodeKind::Group(Group { mirror: None, ..group }))?;
        }
        let c = e.finish();
        self.apply("Make mirror real", c);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use mapcraft_geom::{Aabb, DVec3};
    use mapcraft_map::{Map, ROOT};

    use super::*;

    fn boxes(map: &Map) -> Vec<Aabb> {
        let mut out: Vec<Aabb> = map.nodes().filter_map(|(_, n)| n.kind.brush()?.bounds().ok()).collect();
        out.sort_by(|a, b| a.min.x.total_cmp(&b.min.x).then(a.min.y.total_cmp(&b.min.y)));
        out
    }

    fn flat(s: &Session) -> Map {
        crate::flatten(&s.map, &mut |_| None).0
    }

    #[test]
    fn mirror_groups_follow_their_originals() {
        let mut s = Session::default();
        let a = s.create_brush(Aabb::new(DVec3::new(-256.0, 0.0, 0.0), DVec3::new(-64.0, 64.0, 64.0))).unwrap();
        let b = s.create_brush(Aabb::new(DVec3::new(-64.0, 0.0, 0.0), DVec3::new(0.0, 32.0, 16.0))).unwrap();
        s.selection = BTreeSet::from([a, b]);
        let g = s.make_mirror_group().unwrap();
        let Some(NodeKind::Group(Group { mirror: Some(m), .. })) = s.map.get(g).map(|n| n.kind.clone()) else { panic!() };
        assert_eq!((m.axis, m.offset), (Axis::X, 0.0), "the plane goes through the edge nearest the origin");
        assert_eq!(s.map.len(), 4, "the copies are not part of the map");
        let mirrored = |s: &Session| boxes(&flat(s)).into_iter().filter(|b| b.min.x >= 0.0).collect::<Vec<_>>();
        assert_eq!(
            mirrored(&s),
            vec![Aabb::new(DVec3::ZERO, DVec3::new(64.0, 32.0, 16.0)), Aabb::new(DVec3::new(64.0, 0.0, 0.0), DVec3::new(256.0, 64.0, 64.0))]
        );

        s.selection = BTreeSet::from([a]);
        s.translate_selection(DVec3::new(0.0, 0.0, 32.0), None).unwrap();
        assert_eq!(mirrored(&s)[1], Aabb::new(DVec3::new(64.0, 0.0, 32.0), DVec3::new(256.0, 64.0, 96.0)), "editing an original moves its copy");
        s.undo();
        assert_eq!(mirrored(&s)[1].min.z, 0.0);
        s.redo();
        assert_eq!(mirrored(&s)[1].min.z, 32.0);

        s.selection = BTreeSet::from([g]);
        s.translate_selection(DVec3::new(16.0, 0.0, 0.0), None).unwrap();
        assert_eq!(mirrored(&s)[0].min.x, 16.0, "moving the group carries its plane along");

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("half.mcmap");
        s.save_as(&path).unwrap();
        let back = Session::open(&path).unwrap();
        assert_eq!(back.map, s.map, "saving and loading keeps the originals only");
        assert_eq!(boxes(&flat(&back)), boxes(&flat(&s)));

        s.make_mirror_real().unwrap();
        assert_eq!(s.map.len(), 6);
        assert_eq!(boxes(&s.map), boxes(&flat(&s)), "making it real changes nothing in the views");
        assert!(!is_mirror_group(&s.map, g));
        s.undo();
        assert_eq!(s.map.len(), 4);
        assert!(is_mirror_group(&s.map, g));
    }

    #[test]
    fn a_lone_group_becomes_the_mirror_group() {
        let mut s = Session::default();
        let a = s.create_brush(Aabb::new(DVec3::new(64.0, 0.0, 0.0), DVec3::new(128.0, 64.0, 64.0))).unwrap();
        s.selection = BTreeSet::from([a]);
        let g = s.group_selection().unwrap();
        assert_eq!(s.make_mirror_group().unwrap(), g);
        let Some(NodeKind::Group(Group { mirror: Some(m), .. })) = s.map.get(g).map(|n| n.kind.clone()) else { panic!() };
        assert_eq!(m.offset, 64.0);
        assert_eq!(s.map.get(ROOT).unwrap().children, vec![g]);
        let outer = s.make_mirror_group().unwrap();
        assert_ne!(outer, g, "a mirror group is wrapped, so mirrors can nest");
    }
}
