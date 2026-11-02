//! A bounding volume hierarchy over triangles for shadow and bounce rays.

use mapcraft_geom::DVec3;

const LEAF: usize = 4;
const BINS: usize = 12;
const STACK: usize = 96;

#[derive(Clone, Copy, Debug)]
pub struct Triangle {
    pub a: DVec3,
    pub b: DVec3,
    pub c: DVec3,
    /// What the triangle belongs to, such as a chart index.
    pub owner: u32,
}

impl Triangle {
    fn min(&self) -> DVec3 {
        self.a.min(self.b).min(self.c)
    }

    fn max(&self) -> DVec3 {
        self.a.max(self.b).max(self.c)
    }

    fn center(&self) -> DVec3 {
        (self.a + self.b + self.c) / 3.0
    }

    /// Distance along `dir` to the triangle, hit from either side.
    fn hit(&self, origin: DVec3, dir: DVec3, t_max: f64) -> Option<f64> {
        let (e1, e2) = (self.b - self.a, self.c - self.a);
        let p = dir.cross(e2);
        let det = e1.dot(p);
        if det.abs() < 1e-12 {
            return None;
        }
        let inv = 1.0 / det;
        let s = origin - self.a;
        let u = s.dot(p) * inv;
        if !(0.0..=1.0).contains(&u) {
            return None;
        }
        let q = s.cross(e1);
        let v = dir.dot(q) * inv;
        if v < 0.0 || u + v > 1.0 {
            return None;
        }
        let t = e2.dot(q) * inv;
        (t > 1e-6 && t < t_max).then_some(t)
    }
}

#[derive(Clone, Copy, Debug)]
struct Node {
    min: DVec3,
    max: DVec3,
    /// A leaf's first triangle, or an inner node's first child (the second follows it).
    index: u32,
    /// Triangles in a leaf; 0 for an inner node.
    count: u32,
}

pub struct Bvh {
    nodes: Vec<Node>,
    tris: Vec<Triangle>,
}

fn empty() -> (DVec3, DVec3) {
    (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY))
}

fn slab(origin: DVec3, inv: DVec3, min: DVec3, max: DVec3, t_max: f64) -> bool {
    let (t0, t1) = ((min - origin) * inv, (max - origin) * inv);
    let near = t0.min(t1).max_element().max(0.0);
    let far = t0.max(t1).min_element().min(t_max);
    near <= far
}

fn area(min: DVec3, max: DVec3) -> f64 {
    let d = (max - min).max(DVec3::ZERO);
    d.x * d.y + d.y * d.z + d.z * d.x
}

impl Bvh {
    pub fn new(tris: Vec<Triangle>) -> Self {
        let mut bvh = Self { nodes: Vec::with_capacity(tris.len() * 2 + 1), tris };
        if bvh.tris.is_empty() {
            return bvh;
        }
        bvh.nodes.push(Node { min: DVec3::ZERO, max: DVec3::ZERO, index: 0, count: 0 });
        let mut work = vec![(0usize, 0usize, bvh.tris.len())];
        while let Some((node, start, end)) = work.pop() {
            let (min, max) = bvh.tris[start..end].iter().fold(empty(), |(lo, hi), t| (lo.min(t.min()), hi.max(t.max())));
            let mid = if end - start > LEAF { bvh.split(start, end) } else { None };
            let entry = match mid {
                Some(mid) => {
                    let first = bvh.nodes.len();
                    bvh.nodes.extend([Node { min, max, index: 0, count: 0 }; 2]);
                    work.push((first, start, mid));
                    work.push((first + 1, mid, end));
                    Node { min, max, index: first as u32, count: 0 }
                }
                None => Node { min, max, index: start as u32, count: (end - start) as u32 },
            };
            if let Some(n) = bvh.nodes.get_mut(node) {
                *n = entry;
            }
        }
        bvh
    }

    /// Reorders triangles `start..end` and returns where to split them, by the binned surface area heuristic.
    fn split(&mut self, start: usize, end: usize) -> Option<usize> {
        let tris = &mut self.tris[start..end];
        let (cmin, cmax) = tris.iter().fold(empty(), |(lo, hi), t| (lo.min(t.center()), hi.max(t.center())));
        let extent = cmax - cmin;
        let axis = if extent.x >= extent.y && extent.x >= extent.z {
            0
        } else if extent.y >= extent.z {
            1
        } else {
            2
        };
        let (lo, span) = (cmin[axis], extent[axis]);
        let median = |tris: &mut [Triangle]| {
            let mid = tris.len() / 2;
            tris.select_nth_unstable_by(mid, |a, b| a.center()[axis].total_cmp(&b.center()[axis]));
            Some(start + mid)
        };
        if span <= 1e-9 {
            return median(tris);
        }
        let bin = |t: &Triangle| (((t.center()[axis] - lo) / span * BINS as f64) as usize).min(BINS - 1);
        let mut boxes = [(empty().0, empty().1, 0usize); BINS];
        for t in tris.iter() {
            let b = &mut boxes[bin(t)];
            *b = (b.0.min(t.min()), b.1.max(t.max()), b.2 + 1);
        }
        let side = |r: &[(DVec3, DVec3, usize)]| r.iter().fold((empty().0, empty().1, 0), |a, b| (a.0.min(b.0), a.1.max(b.1), a.2 + b.2));
        let mut best = (f64::INFINITY, 0);
        for cut in 1..BINS {
            let (l, r) = (side(&boxes[..cut]), side(&boxes[cut..]));
            if l.2 == 0 || r.2 == 0 {
                continue;
            }
            let cost = area(l.0, l.1) * l.2 as f64 + area(r.0, r.1) * r.2 as f64;
            if cost < best.0 {
                best = (cost, cut);
            }
        }
        if best.1 == 0 {
            return median(tris);
        }
        let mut i = 0;
        for j in 0..tris.len() {
            if bin(&tris[j]) < best.1 {
                tris.swap(i, j);
                i += 1;
            }
        }
        Some(start + i)
    }

    fn walk(&self, origin: DVec3, dir: DVec3, mut t_max: f64, any: bool) -> Option<(f64, u32)> {
        if self.nodes.is_empty() {
            return None;
        }
        let inv = dir.recip();
        let mut stack = [0u32; STACK];
        let mut top = 1usize;
        let mut best = None;
        while top > 0 {
            top -= 1;
            let Some(node) = self.nodes.get(stack[top] as usize) else { continue };
            if !slab(origin, inv, node.min, node.max, t_max) {
                continue;
            }
            if node.count > 0 {
                let start = node.index as usize;
                for t in self.tris.get(start..start + node.count as usize).unwrap_or_default() {
                    if let Some(d) = t.hit(origin, dir, t_max) {
                        if any {
                            return Some((d, t.owner));
                        }
                        t_max = d;
                        best = Some((d, t.owner));
                    }
                }
            } else if top + 2 <= STACK {
                stack[top] = node.index;
                stack[top + 1] = node.index + 1;
                top += 2;
            }
        }
        best
    }

    /// Whether anything lies along the ray closer than `t_max`.
    pub fn occluded(&self, origin: DVec3, dir: DVec3, t_max: f64) -> bool {
        self.walk(origin, dir, t_max, true).is_some()
    }

    /// The nearest triangle along the ray: its distance and owner.
    pub fn nearest(&self, origin: DVec3, dir: DVec3, t_max: f64) -> Option<(f64, u32)> {
        self.walk(origin, dir, t_max, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(z: f64, owner: u32, out: &mut Vec<Triangle>) {
        let (a, b, c, d) = (DVec3::new(-1.0, -1.0, z), DVec3::new(1.0, -1.0, z), DVec3::new(1.0, 1.0, z), DVec3::new(-1.0, 1.0, z));
        out.push(Triangle { a, b, c, owner });
        out.push(Triangle { a, b: c, c: d, owner });
    }

    #[test]
    fn nearest_finds_the_closest_of_many() {
        let mut tris = Vec::new();
        for i in 0..200u32 {
            quad(f64::from(i) * 2.0 + 1.0, i, &mut tris);
        }
        let bvh = Bvh::new(tris);
        assert_eq!(bvh.nearest(DVec3::new(0.1, 0.2, 0.0), DVec3::Z, 1e9).map(|h| h.1), Some(0));
        assert_eq!(bvh.nearest(DVec3::new(0.1, 0.2, 50.0), DVec3::Z, 1e9).map(|h| h.1), Some(25));
        assert_eq!(bvh.nearest(DVec3::new(0.1, 0.2, 50.0), -DVec3::Z, 1e9).map(|h| h.1), Some(24));
        assert!(bvh.occluded(DVec3::new(0.0, 0.0, 0.0), DVec3::Z, 1.5));
        assert!(!bvh.occluded(DVec3::new(0.0, 0.0, 0.0), DVec3::Z, 0.5));
        assert!(!bvh.occluded(DVec3::new(5.0, 0.0, 0.0), DVec3::Z, 1e9));
    }
}
