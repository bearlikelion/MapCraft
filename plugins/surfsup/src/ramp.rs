//! SURGE surf ramps as brushes: a cross-section swept along a path of frames, one convex brush
//! per path segment and cross-section part. Ported from SurfsUpSDK's `surf_ramp_math.gd`; lengths
//! are map units, the path starts at the origin heading -X with Z up.

use glam::{DAffine3, DVec2, DVec3};

const EPSILON: f64 = 1e-6;
const BANK_EASE_FRACTION: f64 = 0.2;
pub const SURF_MIN_ANGLE: f64 = 45.573;
const CURVED_SEGMENTS_MIN: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Wedge,
    Thin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surf {
    Left,
    Right,
    Both,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Straight,
    Left,
    Right,
    Up,
    Down,
    Dip,
    Arc,
    Spiral,
    SCurve,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Turn {
    Right,
    Left,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Surf,
    Side,
    Bottom,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Ramp {
    pub shape: Shape,
    pub turn: Turn,
    /// Straight length, or the turn radius at the ramp's center line.
    pub size: f64,
    pub angle: f64,
    pub rise: f64,
    pub bank: f64,
    pub smoothness: usize,
    pub style: Style,
    pub surf: Surf,
    pub width: f64,
    pub height: f64,
    pub tip: f64,
    pub thickness: f64,
}

impl Default for Ramp {
    fn default() -> Self {
        Self {
            shape: Shape::Straight,
            turn: Turn::Right,
            size: 1024.0,
            angle: 90.0,
            rise: 512.0,
            bank: 0.0,
            smoothness: 16,
            style: Style::Wedge,
            surf: Surf::Left,
            width: 384.0,
            height: 544.0,
            tip: 64.0,
            thickness: 64.0,
        }
    }
}

/// One brush: the points it is the hull of, and the outward normals of its surf and bottom faces.
pub struct Piece {
    pub points: Vec<DVec3>,
    pub faces: Vec<(DVec3, Slot)>,
}

pub fn slope_line(surf: Surf, width: f64, height: f64, tip: f64) -> Vec<DVec2> {
    match surf {
        Surf::Both => vec![DVec2::new(-width, tip), DVec2::new(0.0, height), DVec2::new(width, tip)],
        Surf::Left => vec![DVec2::new(width * 0.5, tip), DVec2::new(-width * 0.5, height)],
        Surf::Right => vec![DVec2::new(-width * 0.5, tip), DVec2::new(width * 0.5, height)],
    }
}

pub fn slope_angle(width: f64, height: f64, tip: f64) -> f64 {
    if width <= EPSILON {
        return 90.0;
    }
    (height - tip).max(0.0).atan2(width).to_degrees()
}

fn same(a: DVec2, b: DVec2) -> bool {
    (a.x - b.x).abs() <= EPSILON && (a.y - b.y).abs() <= EPSILON
}

fn signed_area(points: &[DVec2]) -> f64 {
    (0..points.len()).map(|i| points[i].perp_dot(points[(i + 1) % points.len()])).sum::<f64>() * 0.5
}

/// Drops repeated points and makes the polygon counter-clockwise.
fn close(points: Vec<DVec2>) -> Vec<DVec2> {
    let mut out: Vec<DVec2> = Vec::new();
    for p in points {
        if out.last().is_none_or(|l| !same(p, *l)) {
            out.push(p);
        }
    }
    while out.len() > 1 && out.first().zip(out.last()).is_some_and(|(a, b)| same(*a, *b)) {
        out.pop();
    }
    if signed_area(&out) < 0.0 {
        out.reverse();
    }
    out
}

fn down_normal(a: DVec2, b: DVec2) -> DVec2 {
    let d = (b - a).normalize_or_zero();
    let n = DVec2::new(-d.y, d.x);
    if n.y < 0.0 { n } else { -n }
}

fn offset_polyline(points: &[DVec2], thickness: f64) -> Vec<DVec2> {
    let normals: Vec<DVec2> = points.windows(2).map(|w| down_normal(w[0], w[1])).collect();
    let (Some(first), Some(last)) = (normals.first(), normals.last()) else { return Vec::new() };
    let mut out = vec![points[0] + *first * thickness];
    for i in 1..points.len() - 1 {
        let (a0, da) = (points[i - 1] + normals[i - 1] * thickness, points[i] - points[i - 1]);
        let (b0, db) = (points[i] + normals[i] * thickness, points[i + 1] - points[i]);
        let den = da.perp_dot(db);
        out.push(if den.abs() < EPSILON { b0 } else { a0 + da * ((b0 - a0).perp_dot(db) / den) });
    }
    out.push(points[points.len() - 1] + *last * thickness);
    out
}

/// The convex parts of the cross-section, each counter-clockwise; empty when the dimensions
/// cannot make a ramp.
pub fn profile_parts(r: &Ramp) -> Vec<Vec<DVec2>> {
    if r.width <= EPSILON || r.height <= EPSILON {
        return Vec::new();
    }
    let tip = if r.style == Style::Thin { 0.0 } else { r.tip.clamp(0.0, r.height - EPSILON) };
    if r.style == Style::Thin {
        if r.thickness <= EPSILON {
            return Vec::new();
        }
        let line = slope_line(r.surf, r.width, r.height, tip);
        let offset = offset_polyline(&line, r.thickness);
        // A thin ramp bends at the peak, so each slope gets its own brush.
        return (0..line.len() - 1).map(|i| close(vec![line[i], line[i + 1], offset[i + 1], offset[i]])).collect();
    }
    let half = if r.surf == Surf::Both { r.width } else { r.width * 0.5 };
    let pts = match r.surf {
        Surf::Both => vec![DVec2::new(-half, 0.0), DVec2::new(half, 0.0), DVec2::new(half, tip), DVec2::new(0.0, r.height), DVec2::new(-half, tip)],
        Surf::Left => vec![DVec2::new(-half, 0.0), DVec2::new(half, 0.0), DVec2::new(half, tip), DVec2::new(-half, r.height)],
        Surf::Right => vec![DVec2::new(-half, 0.0), DVec2::new(half, 0.0), DVec2::new(half, r.height), DVec2::new(-half, tip)],
    };
    vec![close(pts)]
}

fn on_line(a: DVec2, b: DVec2, line: &[DVec2]) -> bool {
    line.windows(2).any(|w| (same(a, w[0]) && same(b, w[1])) || (same(a, w[1]) && same(b, w[0])))
}

fn edge_slot(a: DVec2, b: DVec2, line: &[DVec2]) -> Slot {
    if on_line(a, b, line) {
        Slot::Surf
    } else if (b - a).normalize_or_zero().x > 0.5 {
        Slot::Bottom
    } else {
        Slot::Side
    }
}

pub fn is_closed(shape: Shape, angle: f64, rise: f64) -> bool {
    if matches!(shape, Shape::Straight | Shape::SCurve) || (angle - 360.0).abs() > EPSILON {
        return false;
    }
    shape != Shape::Spiral || rise.abs() < EPSILON
}

fn translation(x: f64, y: f64, z: f64) -> DAffine3 {
    DAffine3::from_translation(DVec3::new(x, y, z))
}

fn turn_about(theta: f64, radius: f64, sign: f64) -> DAffine3 {
    let offset = translation(0.0, sign * radius, 0.0);
    offset * DAffine3::from_axis_angle(DVec3::Z, -sign * theta) * offset.inverse()
}

fn pitch(theta: f64, radius: f64, sign: f64) -> DAffine3 {
    let offset = translation(0.0, 0.0, sign * radius);
    offset * DAffine3::from_axis_angle(DVec3::Y, sign * theta) * offset.inverse()
}

fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn bank_weight(t: f64, closed: bool) -> f64 {
    if closed { 1.0 } else { smoothstep(t / BANK_EASE_FRACTION).min(smoothstep((1.0 - t) / BANK_EASE_FRACTION)) }
}

fn turn_sign(turn: Turn) -> f64 {
    if turn == Turn::Right { 1.0 } else { -1.0 }
}

fn turn_sign_at(shape: Shape, t: f64, turn: Turn) -> f64 {
    match shape {
        Shape::Left => -1.0,
        Shape::Right => 1.0,
        Shape::Spiral => turn_sign(turn),
        Shape::SCurve if t <= 0.5 => turn_sign(turn),
        Shape::SCurve => -turn_sign(turn),
        _ => 0.0,
    }
}

fn frame(r: &Ramp, t: f64) -> DAffine3 {
    let theta = r.angle.to_radians() * t;
    match r.shape {
        Shape::Straight => translation(-r.size * t, 0.0, 0.0),
        Shape::Left => turn_about(theta, r.size, -1.0),
        Shape::Right => turn_about(theta, r.size, 1.0),
        Shape::Spiral => translation(0.0, 0.0, r.rise * t) * turn_about(theta, r.size, turn_sign(r.turn)),
        Shape::SCurve => {
            let sign = turn_sign(r.turn);
            let half = r.angle.to_radians() * 0.5;
            if theta <= half { turn_about(theta, r.size, sign) } else { turn_about(half, r.size, sign) * turn_about(theta - half, r.size, -sign) }
        }
        Shape::Up | Shape::Dip => pitch(theta, r.size + r.height, 1.0),
        Shape::Down | Shape::Arc => pitch(theta, r.size, -1.0),
    }
}

pub fn frames(r: &Ramp) -> Vec<DAffine3> {
    let closed = is_closed(r.shape, r.angle, r.rise);
    let segments = if r.shape == Shape::Straight { 1 } else { r.smoothness.clamp(CURVED_SEGMENTS_MIN, 256) };
    let count = if closed { segments } else { segments + 1 };
    let mut out: Vec<DAffine3> = (0..count)
        .map(|i| {
            let t = i as f64 / segments as f64;
            let mut f = frame(r, t);
            if r.bank.abs() > EPSILON {
                let roll = -turn_sign_at(r.shape, t, r.turn) * r.bank.to_radians() * bank_weight(t, closed);
                f *= DAffine3::from_axis_angle(DVec3::X, roll);
            }
            f
        })
        .collect();
    if matches!(r.shape, Shape::Arc | Shape::Dip) {
        let middle = frame(r, 0.5).inverse();
        out.iter_mut().for_each(|f| *f = middle * *f);
    }
    out
}

fn local(p: DVec2) -> DVec3 {
    DVec3::new(0.0, p.x, p.y)
}

/// The brushes of a ramp, or why it cannot be built.
pub fn build(r: &Ramp) -> Result<Vec<Piece>, String> {
    let parts = profile_parts(r);
    if parts.is_empty() {
        return Err("These dimensions cannot make a ramp.".into());
    }
    if r.shape != Shape::Straight && r.angle <= 0.0 {
        return Err("A curved ramp needs an angle above zero.".into());
    }
    let tip = if r.style == Style::Thin { 0.0 } else { r.tip };
    let line = slope_line(r.surf, r.width, r.height, tip);
    let frames = frames(r);
    let closed = is_closed(r.shape, r.angle, r.rise);
    let steps = if closed { frames.len() } else { frames.len().saturating_sub(1) };
    let mut out = Vec::new();
    for s in 0..steps {
        let (a, b) = (frames[s], frames[(s + 1) % frames.len()]);
        for part in &parts {
            let points = part.iter().flat_map(|p| [a.transform_point3(local(*p)), b.transform_point3(local(*p))]).collect();
            let mut faces = Vec::new();
            for j in 0..part.len() {
                let (p, q) = (part[j], part[(j + 1) % part.len()]);
                let slot = edge_slot(p, q, &line);
                if slot == Slot::Side {
                    continue;
                }
                // Counter-clockwise, so outward is the edge direction turned clockwise.
                let d = q - p;
                let out = local(DVec2::new(d.y, -d.x));
                let out = a.transform_vector3(out) + b.transform_vector3(out);
                let [p0, p1] = [p, q].map(|v| a.transform_point3(local(v)));
                let [q0, q1] = [p, q].map(|v| b.transform_point3(local(v)));
                // A twisted face becomes two triangles in the brush; either diagonal may be used.
                for (x, y, z) in [(p0, p1, q1), (p0, q1, q0), (p0, p1, q0), (p1, q1, q0)] {
                    let n = (y - x).cross(z - x).normalize_or_zero();
                    if n != DVec3::ZERO {
                        faces.push((if n.dot(out) < 0.0 { -n } else { n }, slot));
                    }
                }
            }
            out.push(Piece { points, faces });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn straight_wedge_is_one_brush_with_a_surfable_face() {
        let r = Ramp::default();
        let pieces = build(&r).unwrap();
        assert_eq!(pieces.len(), 1);
        assert_eq!(pieces[0].points.len(), 8);
        let (n, slot) = pieces[0].faces.iter().find(|(_, s)| *s == Slot::Surf).copied().unwrap();
        assert_eq!(slot, Slot::Surf);
        assert!(n.z > 0.0 && n.y > 0.0, "a left surf faces up and to +Y: {n:?}");
        assert!(slope_angle(r.width * 0.5, r.height, r.tip) > SURF_MIN_ANGLE);
        assert!(pieces[0].points.iter().any(|p| (p.x + r.size).abs() < 1e-9));
    }

    #[test]
    fn curves_close_and_thin_ramps_split_at_the_peak() {
        let ring = Ramp { shape: Shape::Right, angle: 360.0, smoothness: 12, ..Ramp::default() };
        assert!(is_closed(ring.shape, ring.angle, ring.rise));
        assert_eq!(build(&ring).unwrap().len(), 12);
        let f = frames(&ring);
        assert_eq!(f.len(), 12);

        let thin = Ramp { style: Style::Thin, surf: Surf::Both, ..Ramp::default() };
        assert_eq!(build(&thin).unwrap().len(), 2);
        let arc = Ramp { shape: Shape::Arc, smoothness: 8, ..Ramp::default() };
        let f = frames(&arc);
        assert!(f[4].translation.length() < 1e-9, "an arc is centered on its middle frame");
        assert!(build(&Ramp { width: 0.0, ..Ramp::default() }).is_err());
    }
}
