//! Cubic Bezier paths: points along them by distance, and the frames that carry a profile along.

use glam::{DMat4, DQuat, DVec3};

/// One cubic span: its start, the start's out handle, the end's in handle, its end.
pub type Span = [DVec3; 4];

/// Steps per span in the table that turns distances into curve parameters.
const FINE: usize = 64;

pub fn bezier(s: &Span, t: f64) -> DVec3 {
    let u = 1.0 - t;
    s[0] * (u * u * u) + s[1] * (3.0 * u * u * t) + s[2] * (3.0 * u * t * t) + s[3] * (t * t * t)
}

/// The direction of travel at `t`; where a handle sits on its point a short chord stands in.
pub fn tangent(s: &Span, t: f64) -> DVec3 {
    let u = 1.0 - t;
    let d = (s[1] - s[0]) * (3.0 * u * u) + (s[2] - s[1]) * (6.0 * u * t) + (s[3] - s[2]) * (3.0 * t * t);
    if d.length_squared() > 1e-18 {
        return d.normalize();
    }
    (bezier(s, (t + 1e-4).min(1.0)) - bezier(s, (t - 1e-4).max(0.0))).normalize_or_zero()
}

/// Whether the span runs straight from its start to its end.
pub fn is_straight(s: &Span) -> bool {
    let d = s[3] - s[0];
    let len = d.length();
    len > 1e-9 && [s[1], s[2]].iter().all(|p| (*p - s[0]).cross(d).length() / len < 1e-6 && (*p - s[0]).dot(d) >= -1e-9 && (*p - s[3]).dot(d) <= 1e-9)
}

/// A point on a path, the direction of travel there and its distance along the path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub at: DVec3,
    pub tangent: DVec3,
    pub dist: f64,
}

/// Distances along a run of spans, so points can be found by how far along they are.
pub struct Measured<'a> {
    spans: &'a [Span],
    /// Distance at each table step of each span, `FINE + 1` per span.
    table: Vec<f64>,
}

impl<'a> Measured<'a> {
    pub fn new(spans: &'a [Span]) -> Self {
        let mut table = Vec::with_capacity(spans.len() * (FINE + 1));
        let mut total = 0.0;
        for s in spans {
            let mut last = s[0];
            for i in 0..=FINE {
                let p = bezier(s, i as f64 / FINE as f64);
                total += p.distance(last);
                last = p;
                table.push(total);
            }
        }
        Self { spans, table }
    }

    pub fn length(&self) -> f64 {
        self.table.last().copied().filter(|l| l.is_finite()).unwrap_or(0.0)
    }

    /// Where span `i` starts and ends along the path.
    pub fn span_range(&self, i: usize) -> (f64, f64) {
        let at = |k: usize| self.table.get(k).copied().unwrap_or(0.0);
        (at(i * (FINE + 1)), at(i * (FINE + 1) + FINE))
    }

    /// The point `dist` along the path, clamped to its ends.
    pub fn at(&self, dist: f64) -> Option<Sample> {
        let k = self.table.partition_point(|d| *d < dist).min(self.table.len().checked_sub(1)?);
        let (span, step) = (k / (FINE + 1), k % (FINE + 1));
        let s = self.spans.get(span)?;
        let t = if step == 0 {
            0.0
        } else {
            let (d0, d1) = (self.table.get(k - 1).copied().unwrap_or(0.0), self.table.get(k).copied().unwrap_or(0.0));
            let f = if d1 - d0 > 1e-12 { ((dist - d0) / (d1 - d0)).clamp(0.0, 1.0) } else { 1.0 };
            (step as f64 - 1.0 + f) / FINE as f64
        };
        Some(Sample { at: bezier(s, t), tangent: tangent(s, t), dist: dist.clamp(0.0, self.length()) })
    }

    /// Points at each span's ends, with curved spans cut into `pieces` of even length between;
    /// where spans meet at an angle the tangent splits it, so the joint is mitered.
    pub fn samples(&self, pieces: usize, closed: bool) -> Vec<Sample> {
        let (Some(first), Some(last)) = (self.spans.first(), self.spans.last()) else { return Vec::new() };
        let joint = |prev: Option<&Span>, next: &Span| match prev {
            Some(p) => (tangent(p, 1.0) + tangent(next, 0.0)).normalize_or(tangent(next, 0.0)),
            None => tangent(next, 0.0),
        };
        let mut out = Vec::new();
        for (i, s) in self.spans.iter().enumerate() {
            let (a, b) = self.span_range(i);
            if b - a < 1e-6 {
                continue;
            }
            let prev = if i > 0 { self.spans.get(i - 1) } else { closed.then_some(last) };
            out.push(Sample { at: s[0], tangent: joint(prev, s), dist: a });
            let n = if is_straight(s) { 1 } else { pieces.max(1) };
            out.extend((1..n).filter_map(|k| self.at(a + (b - a) * k as f64 / n as f64)));
        }
        let end = if closed { joint(Some(last), first) } else { tangent(last, 1.0) };
        out.push(Sample { at: last[3], tangent: end, dist: self.length() });
        out
    }
}

/// A frame per sample (X along the tangent, origin at the point): upright while the path never
/// runs vertical, else rotation minimizing, which never flips; a closed path's last meets its first.
pub fn frames(samples: &[Sample], closed: bool) -> Vec<DMat4> {
    let frame = |s: &Sample, n: DVec3| DMat4::from_cols(s.tangent.extend(0.0), n.extend(0.0), s.tangent.cross(n).extend(0.0), s.at.extend(1.0));
    let upright = |t: DVec3| DVec3::Z.cross(t).normalize_or_zero();
    if samples.iter().all(|s| s.tangent.z.abs() < 0.99) {
        return samples.iter().map(|s| frame(s, upright(s.tangent))).collect();
    }
    let Some(first) = samples.first() else { return Vec::new() };
    let mut n = upright(first.tangent);
    if n == DVec3::ZERO {
        n = first.tangent.any_orthonormal_vector();
    }
    // Double reflection (Wang, Juttler, Zheng and Liu 2008).
    let mut normals = vec![n];
    for w in samples.windows(2) {
        let (a, b) = (w[0], w[1]);
        let v1 = b.at - a.at;
        let c1 = v1.length_squared();
        let (rl, tl) = if c1 > 1e-18 { (n - v1 * (2.0 / c1 * v1.dot(n)), a.tangent - v1 * (2.0 / c1 * v1.dot(a.tangent))) } else { (n, a.tangent) };
        let v2 = b.tangent - tl;
        let c2 = v2.length_squared();
        let r = if c2 > 1e-18 { rl - v2 * (2.0 / c2 * v2.dot(rl)) } else { rl };
        n = (r - b.tangent * r.dot(b.tangent)).normalize_or(n);
        normals.push(n);
    }
    if closed && let (Some(last), Some(n0), Some(s0)) = (normals.last().copied(), normals.first().copied(), samples.first()) {
        let gap = last.cross(n0).dot(s0.tangent).atan2(last.dot(n0));
        let total = samples.last().map_or(0.0, |s| s.dist).max(1e-9);
        for (n, s) in normals.iter_mut().zip(samples) {
            *n = DQuat::from_axis_angle(s.tangent, gap * s.dist / total) * *n;
        }
    }
    samples.iter().zip(normals).map(|(s, n)| frame(s, n)).collect()
}

/// `f` turned `degrees` about its tangent and its cross-section scaled by `scale`.
pub fn twisted(f: &DMat4, degrees: f64, scale: f64) -> DMat4 {
    *f * DMat4::from_quat(DQuat::from_rotation_x(degrees.to_radians())) * DMat4::from_scale(DVec3::new(1.0, scale, scale))
}

/// The yaw of a direction in degrees, `None` when it points straight up or down.
pub fn yaw(d: DVec3) -> Option<f64> {
    (d.x.abs() + d.y.abs() > 1e-9).then(|| d.y.atan2(d.x).to_degrees())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s_curve() -> Vec<Span> {
        let (a, b, c) = (DVec3::ZERO, DVec3::new(512.0, 256.0, 0.0), DVec3::new(1024.0, 0.0, 0.0));
        vec![[a, a + DVec3::X * 256.0, b - DVec3::X * 200.0, b], [b, b + DVec3::X * 200.0, c - DVec3::X * 256.0, c]]
    }

    #[test]
    fn distances_and_straight_spans() {
        let line: Vec<Span> = vec![[DVec3::ZERO, DVec3::X * 10.0, DVec3::X * 90.0, DVec3::X * 100.0]];
        let m = Measured::new(&line);
        assert!((m.length() - 100.0).abs() < 1e-6);
        assert_eq!(m.samples(8, false).len(), 2, "a straight span needs no cuts");
        assert!(m.at(25.0).unwrap().at.distance(DVec3::X * 25.0) < 0.05);
        let curve = s_curve();
        let m = Measured::new(&curve);
        let s = m.samples(8, false);
        assert_eq!(s.len(), 17);
        let steps: Vec<f64> = s.windows(2).map(|w| w[1].dist - w[0].dist).collect();
        assert!(steps[..8].iter().all(|d| (d - steps[0]).abs() < 1e-3), "pieces of a span are even: {steps:?}");
        assert!(s.iter().all(|x| (m.at(x.dist).unwrap().at - x.at).length() < 1e-9));
        assert!(m.at(-5.0).unwrap().at.distance(DVec3::ZERO) < 1e-9 && m.at(1e9).unwrap().at.distance(DVec3::new(1024.0, 0.0, 0.0)) < 1e-9);
        assert!(Measured::new(&[]).at(1.0).is_none());
    }

    fn check(frames: &[DMat4], samples: &[Sample]) {
        for (f, s) in frames.iter().zip(samples) {
            let (x, y, z) = (f.x_axis.truncate(), f.y_axis.truncate(), f.z_axis.truncate());
            assert!(x.abs_diff_eq(s.tangent, 1e-9));
            assert!((y.length() - 1.0).abs() < 1e-9 && x.dot(y).abs() < 1e-9 && z.abs_diff_eq(x.cross(y), 1e-9));
        }
        for w in frames.windows(2) {
            assert!(w[0].y_axis.dot(w[1].y_axis) > 0.5, "no flips between neighbors");
        }
    }

    #[test]
    fn frames_stay_upright_on_flat_curves() {
        let curve = s_curve();
        let samples = Measured::new(&curve).samples(16, false);
        let f = frames(&samples, false);
        check(&f, &samples);
        assert!(f.iter().all(|f| f.z_axis.truncate().abs_diff_eq(DVec3::Z, 1e-9)), "the cross-section never banks on a level path");
    }

    #[test]
    fn frames_through_a_vertical_loop_do_not_flip() {
        let r = 256.0;
        let k = 0.5523 * r;
        let (a, b, c, d) = (DVec3::new(0.0, 0.0, -r), DVec3::new(r, 0.0, 0.0), DVec3::new(0.0, 80.0, r), DVec3::new(-r, 0.0, 0.0));
        let loop_: Vec<Span> = vec![
            [a, a + DVec3::X * k, b - DVec3::Z * k, b],
            [b, b + DVec3::Z * k, c + DVec3::X * k, c],
            [c, c - DVec3::X * k, d + DVec3::Z * k, d],
            [d, d - DVec3::Z * k, a - DVec3::X * k, a],
        ];
        let samples = Measured::new(&loop_).samples(12, true);
        let f = frames(&samples, true);
        check(&f, &samples);
        let (first, last) = (f.first().unwrap(), f.last().unwrap());
        assert!(first.y_axis.abs_diff_eq(last.y_axis, 1e-6), "a closed path's frames meet");
    }
}
