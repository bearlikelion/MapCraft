//! Splits a triangle mesh, such as a Godot `ConcavePolygonShape3D`, into convex pieces for hull brushes.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use glam::{DVec2, DVec3};

/// How far a piece's corner may sit in front of one of its triangles, in map units.
const TOLERANCE: f64 = 0.5;
/// A quad split into two triangles may be this far from flat and still be one face.
const MAX_TWIST: f64 = 2.0;
/// How thick one sided surfaces are made behind their face.
pub const DEPTH: f64 = 16.0;
/// Corners closer than this are the same corner.
const WELD: f64 = 0.05;
const MIN_AREA: f64 = 1e-3;
const MAX_PIECE: usize = 128;

pub struct Piece {
    pub points: Vec<DVec3>,
    /// The source triangles on the piece's surface, with their outward normals.
    pub faces: Vec<(usize, DVec3)>,
}

pub struct Split {
    pub pieces: Vec<Piece>,
    /// Source triangles in no piece: those without area, and those past the work budget.
    pub left: usize,
}

struct Facet {
    normal: DVec3,
    offset: f64,
    tol: f64,
    /// Edges where the flat area ends.
    hard: Vec<[usize; 2]>,
}

struct Mesh {
    pts: Vec<DVec3>,
    tris: Vec<[usize; 3]>,
    src: Vec<usize>,
    normal: Vec<DVec3>,
    offset: Vec<f64>,
    area: Vec<f64>,
    tol: Vec<f64>,
    closed: Vec<bool>,
    near: Vec<Vec<usize>>,
    at: Vec<Vec<usize>>,
    facet: Vec<usize>,
    facets: Vec<Facet>,
}

/// Splits map space triangles into convex pieces, spending at most `budget` units of work.
pub fn split(triangles: &[[DVec3; 3]], budget: &mut usize) -> Split {
    let m = mesh(triangles);
    let mut left = triangles.len() - m.tris.len();
    let mut pieces = Vec::new();
    let mut used = vec![false; m.tris.len()];
    let mut grower = Grower { m: &m, member: vec![0; m.tris.len()], vert: vec![0; m.pts.len()], stamp: 0 };
    for seed in 0..m.tris.len() {
        if used.get(seed) != Some(&false) {
            continue;
        }
        if *budget == 0 {
            left += used.iter().filter(|u| !**u).count();
            break;
        }
        let (region, verts) = grower.grow(seed, &mut used, budget);
        pieces.extend(m.pieces(&region, &verts));
    }
    Split { pieces, left }
}

fn edge(a: usize, b: usize) -> (usize, usize) {
    (a.min(b), a.max(b))
}

fn mesh(triangles: &[[DVec3; 3]]) -> Mesh {
    let mut ids: BTreeMap<[i64; 3], usize> = BTreeMap::new();
    let mut pts: Vec<DVec3> = Vec::new();
    let mut tris = Vec::new();
    let mut src = Vec::new();
    for (i, t) in triangles.iter().enumerate() {
        if !t.iter().all(|p| p.is_finite()) {
            continue;
        }
        let tri = t.map(|p| {
            *ids.entry((p / WELD).round().as_i64vec3().to_array()).or_insert_with(|| {
                pts.push(p);
                pts.len() - 1
            })
        });
        let [a, b, c] = tri.map(|i| pts.get(i).copied().unwrap_or_default());
        if (b - a).cross(c - a).length() / 2.0 > MIN_AREA {
            tris.push(tri);
            src.push(i);
        }
    }
    let mut edges: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
    for (t, tri) in tris.iter().enumerate() {
        for k in 0..3 {
            edges.entry(edge(tri[k], tri[(k + 1) % 3])).or_default().push(t);
        }
    }
    let closed = orient(&pts, &mut tris, &edges);
    let corner = |t: &[usize; 3]| t.map(|i| pts.get(i).copied().unwrap_or_default());
    let mut normal = Vec::with_capacity(tris.len());
    let mut offset = Vec::with_capacity(tris.len());
    let mut area = Vec::with_capacity(tris.len());
    for t in &tris {
        let [a, b, c] = corner(t);
        let cross = (b - a).cross(c - a);
        normal.push(cross.normalize_or_zero());
        offset.push(cross.normalize_or_zero().dot(a));
        area.push(cross.length() / 2.0);
    }
    let mut near = vec![Vec::new(); tris.len()];
    let mut at = vec![Vec::new(); pts.len()];
    for (t, tri) in tris.iter().enumerate() {
        for k in 0..3 {
            for &u in edges.get(&edge(tri[k], tri[(k + 1) % 3])).map(Vec::as_slice).unwrap_or_default() {
                if u != t && !near[t].contains(&u) {
                    near[t].push(u);
                }
            }
            if let Some(list) = at.get_mut(tri[k]) {
                list.push(t);
            }
        }
    }
    let mut m = Mesh { pts, tris, src, normal, offset, area, tol: Vec::new(), closed, near, at, facet: Vec::new(), facets: Vec::new() };
    m.tol = m.tolerances(&edges);
    m.find_facets(&edges);
    m
}

/// Makes winding agree, out of closed solids and up for open surfaces; returns which triangles are closed.
fn orient(pts: &[DVec3], tris: &mut [[usize; 3]], edges: &BTreeMap<(usize, usize), Vec<usize>>) -> Vec<bool> {
    let mut seen = vec![false; tris.len()];
    let mut closed = vec![false; tris.len()];
    let origin = pts.first().copied().unwrap_or_default();
    let p = |i: usize| pts.get(i).copied().unwrap_or_default() - origin;
    for seed in 0..tris.len() {
        if seen[seed] {
            continue;
        }
        seen[seed] = true;
        let (mut part, mut stack, mut shut) = (vec![seed], vec![seed], true);
        while let Some(t) = stack.pop() {
            let tri = tris[t];
            for k in 0..3 {
                let (a, b) = (tri[k], tri[(k + 1) % 3]);
                let list = edges.get(&edge(a, b)).map(Vec::as_slice).unwrap_or_default();
                let &[x, y] = list else {
                    shut = false;
                    continue;
                };
                let u = if x == t { y } else { x };
                if seen[u] {
                    continue;
                }
                let o = tris[u];
                if (0..3).any(|j| o[j] == a && o[(j + 1) % 3] == b) {
                    tris[u].swap(1, 2);
                }
                seen[u] = true;
                part.push(u);
                stack.push(u);
            }
        }
        let volume: f64 = part.iter().map(|&t| p(tris[t][0]).dot(p(tris[t][1]).cross(p(tris[t][2])))).sum();
        let up: f64 = part.iter().map(|&t| (p(tris[t][1]) - p(tris[t][0])).cross(p(tris[t][2]) - p(tris[t][0])).z).sum();
        if (shut && volume < 0.0) || (!shut && up < 0.0) {
            part.iter().for_each(|&t| tris[t].swap(1, 2));
        }
        part.iter().for_each(|&t| closed[t] = shut);
    }
    closed
}

impl Mesh {
    fn p(&self, i: usize) -> DVec3 {
        self.pts.get(i).copied().unwrap_or_default()
    }

    /// How far each triangle's plane may miss a corner: more for the halves of a slightly twisted quad.
    fn tolerances(&self, edges: &BTreeMap<(usize, usize), Vec<usize>>) -> Vec<f64> {
        let mut tol = vec![TOLERANCE; self.tris.len()];
        for (&(a, b), list) in edges {
            let &[t, u] = list.as_slice() else { continue };
            if self.normal[t].dot(self.normal[u]) < 5f64.to_radians().cos() {
                continue;
            }
            let other = |x: usize| self.tris[x].into_iter().find(|v| *v != a && *v != b).unwrap_or(a);
            let twist = (self.normal[t].dot(self.p(other(u))) - self.offset[t]).abs().max((self.normal[u].dot(self.p(other(t))) - self.offset[u]).abs());
            if twist <= MAX_TWIST {
                tol[t] = tol[t].max(TOLERANCE + twist);
                tol[u] = tol[u].max(TOLERANCE + twist);
            }
        }
        tol
    }

    /// Groups neighboring triangles that lie in one plane, and finds the edges where each group ends.
    fn find_facets(&mut self, edges: &BTreeMap<(usize, usize), Vec<usize>>) {
        let n = self.tris.len();
        self.facet = vec![usize::MAX; n];
        for seed in 0..n {
            if self.facet[seed] != usize::MAX {
                continue;
            }
            let f = self.facets.len();
            let (normal, offset, tol) = (self.normal[seed], self.offset[seed], self.tol[seed]);
            self.facet[seed] = f;
            let (mut members, mut stack) = (vec![seed], vec![seed]);
            while let Some(t) = stack.pop() {
                for i in 0..self.near[t].len() {
                    let u = self.near[t][i];
                    let flat = self.tris[u].iter().all(|v| (normal.dot(self.p(*v)) - offset).abs() <= tol);
                    if self.facet[u] == usize::MAX && self.normal[u].dot(normal) > 0.99 && flat {
                        self.facet[u] = f;
                        members.push(u);
                        stack.push(u);
                    }
                }
            }
            let mut hard = Vec::new();
            for &t in &members {
                let tri = self.tris[t];
                for k in 0..3 {
                    let (a, b) = (tri[k], tri[(k + 1) % 3]);
                    let list = edges.get(&edge(a, b)).map(Vec::as_slice).unwrap_or_default();
                    if !list.iter().any(|u| *u != t && self.facet[*u] == f) {
                        hard.push([a, b]);
                    }
                }
            }
            self.facets.push(Facet { normal, offset, tol, hard });
        }
    }

    /// A region's brushes: its corners, else its surface pushed back by `DEPTH`, else one per triangle.
    fn pieces(&self, region: &[usize], verts: &[usize]) -> Vec<Piece> {
        let faces: Vec<(usize, DVec3)> = region.iter().map(|&t| (self.src[t], self.normal[t])).collect();
        let points: Vec<DVec3> = verts.iter().map(|&v| self.p(v)).collect();
        let depth = region.iter().map(|&t| points.iter().map(|p| self.offset[t] - self.normal[t].dot(*p)).fold(0.0, f64::max)).fold(f64::INFINITY, f64::min);
        let closed = region.iter().all(|&t| self.closed[t]);
        if depth >= DEPTH || (closed && depth > 1.0) {
            return vec![Piece { points, faces }];
        }
        let sum: DVec3 = region.iter().map(|&t| self.normal[t] * self.area[t]).sum();
        let total: f64 = region.iter().map(|&t| self.area[t]).sum();
        let back = sum.normalize_or_zero();
        if !closed && sum.length() > 0.5 * total && region.iter().all(|&t| self.normal[t].dot(back) > 0.05) {
            let mut points = points;
            points.extend(verts.iter().map(|&v| self.p(v) - back * DEPTH));
            return vec![Piece { points, faces }];
        }
        region
            .iter()
            .map(|&t| {
                let corners = self.tris[t].map(|v| self.p(v));
                let points = corners.iter().copied().chain(corners.iter().map(|p| *p - self.normal[t] * DEPTH)).collect();
                Piece { points, faces: vec![(self.src[t], self.normal[t])] }
            })
            .collect()
    }
}

/// Grows one region at a time; `member` and `vert` hold the current region's stamp.
struct Grower<'m> {
    m: &'m Mesh,
    member: Vec<u32>,
    vert: Vec<u32>,
    stamp: u32,
}

impl Grower<'_> {
    fn grow(&mut self, seed: usize, used: &mut [bool], budget: &mut usize) -> (Vec<usize>, Vec<usize>) {
        let m = self.m;
        self.stamp += 1;
        let stamp = self.stamp;
        let mut region = vec![seed];
        let mut verts: Vec<usize> = m.tris[seed].to_vec();
        self.mark(seed, &verts, used);
        let mut queue: VecDeque<usize> = m.near[seed].iter().copied().collect();
        let (mut rejected, mut progress) = (Vec::new(), false);
        while region.len() < MAX_PIECE && *budget > 0 {
            let Some(t) = queue.pop_front() else {
                if rejected.is_empty() || !progress {
                    break;
                }
                queue.extend(rejected.drain(..));
                progress = false;
                continue;
            };
            if used.get(t) != Some(&false) {
                continue;
            }
            let fresh: Vec<usize> = m.tris[t].into_iter().filter(|v| self.vert[*v] != stamp).collect();
            let has = |v: usize, s: &Self| s.vert[v] == stamp || fresh.contains(&v);
            let mut added = vec![t];
            for &v in &fresh {
                for &u in &m.at[v] {
                    if u != t && !used[u] && !added.contains(&u) && m.tris[u].iter().all(|w| has(*w, self)) {
                        added.push(u);
                    }
                }
            }
            let hard = m.facets.get(m.facet[t]).map_or(0, |f| f.hard.len());
            *budget = budget.saturating_sub((region.len() + added.len()) * (verts.len() + fresh.len()) + hard * 16);
            if !self.fits(&region, &verts, &added, &fresh) {
                rejected.push(t);
                continue;
            }
            progress = true;
            verts.extend(&fresh);
            for &u in &added {
                self.mark(u, &fresh, used);
                queue.extend(&m.near[u]);
            }
            region.extend(added);
        }
        (region, verts)
    }

    fn mark(&mut self, t: usize, verts: &[usize], used: &mut [bool]) {
        used[t] = true;
        self.member[t] = self.stamp;
        verts.iter().for_each(|v| self.vert[*v] = self.stamp);
    }

    /// Whether the region stays convex with `added` triangles and `fresh` corners.
    fn fits(&self, region: &[usize], verts: &[usize], added: &[usize], fresh: &[usize]) -> bool {
        let m = self.m;
        let all: Vec<usize> = verts.iter().chain(fresh).copied().collect();
        let behind = |t: usize, vs: &[usize]| vs.iter().all(|v| m.normal[t].dot(m.p(*v)) - m.offset[t] <= m.tol[t]);
        if !added.iter().all(|&t| behind(t, &all)) || !region.iter().all(|&t| behind(t, fresh)) {
            return false;
        }
        let facets: BTreeSet<usize> = region.iter().chain(added).map(|&t| m.facet[t]).collect();
        for &i in &facets {
            let Some(f) = m.facets.get(i) else { continue };
            let touched = added.iter().any(|t| m.facet[*t] == i) || fresh.iter().any(|v| (f.normal.dot(m.p(*v)) - f.offset).abs() <= f.tol);
            if touched && !self.flat_fits(f, &all) {
                return false;
            }
        }
        self.bridges_nothing(region, added, &all, &facets)
    }

    /// A flat side's corners must not reach past the edges where that flat area of the mesh ends.
    fn flat_fits(&self, f: &Facet, all: &[usize]) -> bool {
        let m = self.m;
        let x = f.normal.any_orthonormal_vector();
        let y = f.normal.cross(x);
        let flat = |p: DVec3| DVec2::new(p.dot(x), p.dot(y));
        let corners: Vec<DVec2> = all.iter().filter(|v| (f.normal.dot(m.p(**v)) - f.offset).abs() <= f.tol).map(|v| flat(m.p(*v))).collect();
        let hull = hull2(corners);
        hull.len() < 3 || !f.hard.iter().any(|[a, b]| crosses(&hull, flat(m.p(*a)), flat(m.p(*b)), f.tol))
    }

    /// No neighbor's corner may sit in the region's box behind all its planes, as when it wraps a hollow.
    fn bridges_nothing(&self, region: &[usize], added: &[usize], all: &[usize], facets: &BTreeSet<usize>) -> bool {
        let m = self.m;
        let stamp = self.stamp;
        let pts: Vec<DVec3> = all.iter().map(|v| m.p(*v)).collect();
        let Some(lo) = pts.iter().copied().reduce(DVec3::min) else { return true };
        let hi = pts.iter().copied().fold(lo, DVec3::max);
        let planes: Vec<&Facet> = facets.iter().filter_map(|f| m.facets.get(*f)).collect();
        let inside = |p: DVec3| p.cmpge(lo - TOLERANCE).all() && p.cmple(hi + TOLERANCE).all() && planes.iter().all(|f| f.normal.dot(p) - f.offset < -f.tol);
        region
            .iter()
            .chain(added)
            .flat_map(|t| &m.near[*t])
            .filter(|u| self.member[**u] != stamp && !added.contains(u))
            .all(|u| m.tris[*u].iter().all(|v| self.vert[*v] == stamp || all.contains(v) || !inside(m.p(*v))))
    }
}

/// The convex hull of points in a plane, counterclockwise.
fn hull2(mut pts: Vec<DVec2>) -> Vec<DVec2> {
    pts.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    pts.dedup_by(|a, b| a.distance_squared(*b) < 1e-12);
    if pts.len() < 3 {
        return pts;
    }
    let half = |pts: &mut dyn Iterator<Item = DVec2>| {
        let mut out: Vec<DVec2> = Vec::new();
        for q in pts {
            while let [.., a, b] = *out.as_slice()
                && (b - a).perp_dot(q - a) <= 1e-9
            {
                out.pop();
            }
            out.push(q);
        }
        out.pop();
        out
    };
    let mut lower = half(&mut pts.iter().copied());
    lower.extend(half(&mut pts.iter().rev().copied()));
    lower
}

/// Whether the segment `a`-`b` passes more than `tol` inside a counterclockwise convex polygon.
fn crosses(hull: &[DVec2], a: DVec2, b: DVec2, tol: f64) -> bool {
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    let d = b - a;
    for (i, p) in hull.iter().enumerate() {
        let q = hull.get((i + 1) % hull.len()).copied().unwrap_or(*p);
        let out = DVec2::new(q.y - p.y, p.x - q.x).normalize_or_zero();
        if out == DVec2::ZERO {
            continue;
        }
        let (start, along) = ((a - *p).dot(out) + tol, d.dot(out));
        if along.abs() < 1e-12 {
            if start > 0.0 {
                return false;
            }
            continue;
        }
        let r = -start / along;
        if along > 0.0 {
            t1 = t1.min(r);
        } else {
            t0 = t0.max(r);
        }
        if t0 > t1 {
            return false;
        }
    }
    t1 - t0 > 1e-9
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quads(grid: &[Vec<DVec3>], wrap: bool) -> Vec<[DVec3; 3]> {
        let mut out = Vec::new();
        for pair in grid.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            let n = a.len();
            for j in 0..n - usize::from(!wrap) {
                let k = (j + 1) % n;
                out.push([a[j], a[k], b[k]]);
                out.push([a[j], b[k], b[j]]);
            }
        }
        out
    }

    /// A closed sweep of a triangle profile along a path in the XY plane.
    fn sweep(path: &[DVec2]) -> Vec<[DVec3; 3]> {
        let profile = [(-300.0, 0.0), (300.0, 0.0), (0.0, 400.0)];
        let rings: Vec<Vec<DVec3>> = (0..path.len())
            .map(|k| {
                let dir = (path[(k + 1).min(path.len() - 1)] - path[k.saturating_sub(1)]).normalize();
                profile.iter().map(|(x, z)| (path[k] + dir.perp() * -*x).extend(*z)).collect()
            })
            .collect();
        let mut out = quads(&rings, true);
        let (first, last) = (&rings[0], &rings[rings.len() - 1]);
        out.push([first[0], first[2], first[1]]);
        out.push([last[0], last[1], last[2]]);
        out
    }

    fn arc(radius: f64, degrees: f64, steps: usize) -> Vec<DVec2> {
        (0..=steps).map(|i| DVec2::from_angle((degrees * i as f64 / steps as f64).to_radians()) * radius).collect()
    }

    /// Whether every source triangle lies on its piece's hull: all the piece's corners are behind it.
    fn on_faces(tris: &[[DVec3; 3]], split: &Split) -> usize {
        split
            .pieces
            .iter()
            .flat_map(|p| p.faces.iter().map(move |f| (p, f)))
            .filter(|(p, (i, n))| {
                p.points.iter().all(|q| n.dot(*q - tris[*i][0]) <= 1.0) && tris[*i].iter().all(|c| p.points.iter().any(|q| q.distance(*c) < 0.1))
            })
            .count()
    }

    fn run(tris: &[[DVec3; 3]]) -> Split {
        split(tris, &mut 10_000_000)
    }

    #[test]
    fn a_closed_prism_is_one_brush() {
        let tris = sweep(&[DVec2::ZERO, DVec2::new(0.0, 1000.0)]);
        let out = run(&tris);
        assert_eq!((out.pieces.len(), out.left), (1, 0));
        assert_eq!(out.pieces[0].faces.len(), 8);
        assert_eq!(on_faces(&tris, &out), 8);
    }

    #[test]
    fn a_curved_ramp_splits_by_segment_and_keeps_every_face() {
        let mut tris = sweep(&arc(3000.0, 90.0, 24));
        tris.iter_mut().step_by(3).for_each(|t| t.swap(1, 2));
        let out = run(&tris);
        assert_eq!(out.pieces.len(), 24, "one brush a segment, though some triangles were wound backwards");
        assert_eq!(on_faces(&tris, &out), tris.len());
    }

    #[test]
    fn a_curved_strip_becomes_convex_brushes_covering_every_triangle() {
        let row = |y: f64| (0..=8).map(|i| DVec3::new(i as f64 * 100.0 - 400.0, y, (i as f64 - 4.0).powi(2) * 10.0)).collect::<Vec<_>>();
        let tris = quads(&[row(0.0), row(600.0)], false);
        let out = run(&tris);
        assert!(out.pieces.len() > 1, "a valley cannot be one convex brush");
        assert_eq!(on_faces(&tris, &out), tris.len());
        for p in &out.pieces {
            assert!(p.faces.iter().all(|(i, n)| p.points.iter().any(|q| n.dot(*q - tris[*i][0]) < -10.0)), "each piece has thickness under the surface");
        }
    }

    #[test]
    fn an_open_ramp_gets_thickness() {
        let (a, b, c, d) = (DVec3::ZERO, DVec3::new(500.0, 0.0, 0.0), DVec3::new(500.0, 500.0, 300.0), DVec3::new(0.0, 500.0, 300.0));
        let tris = [[a, c, b], [a, d, c]];
        let out = run(&tris);
        assert_eq!(out.pieces.len(), 1);
        let p = &out.pieces[0];
        assert_eq!(p.points.len(), 8);
        let n = p.faces[0].1;
        assert!(n.z > 0.5, "the face points up, whichever way it was wound: {n}");
        assert!(p.points.iter().any(|q| (n.dot(*q - a) + DEPTH).abs() < 1e-6), "the back sits {DEPTH} units behind the face");
        assert_eq!(on_faces(&tris, &out), 2);
    }

    #[test]
    fn a_notched_solid_does_not_fill_its_notch() {
        let l = [(0.0, 0.0), (400.0, 0.0), (400.0, 100.0), (100.0, 100.0), (100.0, 300.0), (0.0, 300.0)];
        let ring = |y: f64| l.iter().map(|(x, z)| DVec3::new(*x, y, *z)).collect::<Vec<_>>();
        let (front, back) = (ring(0.0), ring(500.0));
        let mut tris = quads(&[front.clone(), back.clone()], true);
        for k in 1..5 {
            tris.push([front[0], front[k + 1], front[k]]);
            tris.push([back[0], back[k], back[k + 1]]);
        }
        let out = run(&tris);
        assert_eq!(on_faces(&tris, &out), tris.len());
        for p in &out.pieces {
            let (low, narrow) = (p.points.iter().all(|q| q.z <= 100.0 + 1e-6), p.points.iter().all(|q| q.x <= 100.0 + 1e-6));
            assert!(low || narrow, "no piece reaches across the notch");
        }
    }

    #[test]
    fn a_flat_ring_is_cut_where_it_bends() {
        let ring = |r: f64| arc(r, 180.0, 18).into_iter().map(|p| p.extend(0.0)).collect::<Vec<_>>();
        let tris = quads(&[ring(1000.0), ring(1500.0)], false);
        let out = run(&tris);
        assert_eq!(on_faces(&tris, &out), tris.len());
        for p in &out.pieces {
            let reach = p.points.iter().map(|q| q.truncate().length()).fold(f64::INFINITY, f64::min);
            assert!(reach > 990.0, "no piece reaches into the ring's hole: {reach}");
        }
    }

    #[test]
    fn degenerate_meshes_fail_safely() {
        let line = [[DVec3::ZERO, DVec3::X, DVec3::X * 2.0], [DVec3::ZERO; 3], [DVec3::NAN, DVec3::ONE, DVec3::Y]];
        let out = run(&line);
        assert_eq!((out.pieces.len(), out.left), (0, 3));
        assert_eq!(run(&[]).pieces.len(), 0);
        let tris = sweep(&arc(3000.0, 90.0, 24));
        let out = split(&tris, &mut 1000);
        assert!(out.left > 0 && out.pieces.len() < 24, "work stops at the budget");
    }
}
