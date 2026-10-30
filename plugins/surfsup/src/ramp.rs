//! SURGE surf ramps as brushes: a cross-section swept along a path of frames, one convex brush
//! per path segment and cross-section part. Ported from SurfsUpSDK's `surf_ramp_math.gd`; lengths
//! are map units, the path starts at the origin heading -X with Z up.

use glam::{DAffine3, DVec2, DVec3};

const EPSILON: f64 = 1e-6;
/// How far a face's fourth corner may sit off the plane of the other three before it is split.
const TWIST: f64 = 1e-3;
const BANK_EASE_FRACTION: f64 = 0.2;
pub const SURF_MIN_ANGLE: f64 = 45.573;
const CURVED_SEGMENTS_MIN: usize = 3;
/// Degrees of turn per curve segment above which seams start to catch players.
pub const MAX_SEGMENT_DEGREES: f64 = 5.0;

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
    /// A trough: two half ramps facing each other across a floor `ridge` wide.
    Inside,
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
    /// Height lost over the ramp's length.
    pub drop: f64,
    /// Width of a flat top on the peak, or of the floor of an inside ramp.
    pub ridge: f64,
    /// Space left between consecutive pieces.
    pub gap: f64,
    /// How far a straight ramp's base reaches past its top at each end; negative undercuts.
    pub end_slope: f64,
    /// Straight runs before and after a curve.
    pub lead_in: f64,
    pub lead_out: f64,
    /// Trim bands along each surf face from its top and from its bottom edge, measured down the face.
    pub trim_top: f64,
    pub trim_bottom: f64,
    /// How far trim stands off the surf face.
    pub trim_depth: f64,
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
            drop: 0.0,
            ridge: 0.0,
            gap: 0.0,
            end_slope: 0.0,
            lead_in: 0.0,
            lead_out: 0.0,
            trim_top: 0.0,
            trim_bottom: 0.0,
            trim_depth: 1.0,
        }
    }
}

/// A texture axis in map units, `p . axis + offset`, before the texture scale divides it.
pub type UvAxis = (DVec3, f64);

/// A brush face: three points clockwise seen from outside, what part of the ramp it is, and its texture.
pub struct Face {
    pub points: [DVec3; 3],
    pub slot: Slot,
    pub uv: Option<(UvAxis, UvAxis)>,
}

impl Face {
    pub fn normal(&self) -> DVec3 {
        let [a, b, c] = self.points;
        (a - b).cross(c - b).normalize_or_zero()
    }
}

/// One brush, as the faces that bound it.
pub struct Piece {
    pub faces: Vec<Face>,
}

/// The surfable line across the cross-section, including any flat ridge or trough floor.
pub fn slope_line(surf: Surf, width: f64, height: f64, tip: f64, ridge: f64) -> Vec<DVec2> {
    let (half, g) = (width * 0.5, ridge.max(0.0) * 0.5);
    let mut line = match surf {
        Surf::Both => vec![DVec2::new(-width - g, tip), DVec2::new(-g, height), DVec2::new(g, height), DVec2::new(width + g, tip)],
        Surf::Left => vec![DVec2::new(half, tip), DVec2::new(-half, height), DVec2::new(-half - 2.0 * g, height)],
        Surf::Right => vec![DVec2::new(-half, tip), DVec2::new(half, height), DVec2::new(half + 2.0 * g, height)],
        Surf::Inside => vec![DVec2::new(-width - g, height), DVec2::new(-g, tip), DVec2::new(g, tip), DVec2::new(width + g, height)],
    };
    line.dedup_by(|a, b| same(*a, *b));
    line
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
    let line = slope_line(r.surf, r.width, r.height, tip, r.ridge);
    if r.style == Style::Thin {
        if r.thickness <= EPSILON {
            return Vec::new();
        }
        let offset = offset_polyline(&line, r.thickness);
        // A thin ramp bends at the peak, so each slope gets its own brush.
        return (0..line.len() - 1).map(|i| close(vec![line[i], line[i + 1], offset[i + 1], offset[i]])).collect();
    }
    // A wedge is the area under the slope line down to the ground, split where it stops being convex.
    let down = |p: DVec2| DVec2::new(p.x, 0.0);
    if r.surf == Surf::Inside {
        return line.windows(2).map(|w| close(vec![down(w[0]), down(w[1]), w[1], w[0]])).collect();
    }
    let (Some(first), Some(last)) = (line.first(), line.last()) else { return Vec::new() };
    let mut pts = vec![down(*first)];
    pts.extend(line.iter().copied());
    pts.push(down(*last));
    vec![close(pts)]
}

fn convex(p: &[DVec2]) -> bool {
    let n = p.len();
    n >= 3 && (0..n).all(|i| (p[(i + 1) % n] - p[i]).perp_dot(p[(i + 2) % n] - p[(i + 1) % n]) > EPSILON)
}

/// Slabs standing `trim_depth` off the surf line within `trim_top` of its highest points and `trim_bottom` of its lowest, one convex part per line segment.
fn trim_parts(r: &Ramp) -> Vec<Vec<DVec2>> {
    if r.trim_depth <= EPSILON || r.width <= EPSILON || r.height <= EPSILON {
        return Vec::new();
    }
    let tip = if r.style == Style::Thin { 0.0 } else { r.tip.clamp(0.0, r.height - EPSILON) };
    let line = slope_line(r.surf, r.width, r.height, tip, r.ridge);
    let Some(last) = line.len().checked_sub(1).filter(|n| *n > 0) else { return Vec::new() };
    let at: Vec<f64> = std::iter::once(0.0)
        .chain(line.windows(2).scan(0.0, |s, w| {
            *s += w[0].distance(w[1]);
            Some(*s)
        }))
        .collect();
    let (lo, hi) = line.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| (lo.min(p.y), hi.max(p.y)));
    let mut spans = Vec::new();
    for (band, level) in [(r.trim_top, hi), (r.trim_bottom, lo)] {
        let on = |i: usize| line.get(i).is_some_and(|p| (p.y - level).abs() <= EPSILON);
        if band > EPSILON {
            // A flat ridge or trough floor at that level is trimmed whole.
            spans.extend((0..line.len()).filter(|i| on(*i)).map(|i| (at[i] - band, at[if on(i + 1) { i + 1 } else { i }] + band)));
        }
    }
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (a, b) in spans {
        match merged.last_mut() {
            Some(m) if a <= m.1 => m.1 = m.1.max(b),
            _ => merged.push((a, b)),
        }
    }
    // Ends within a unit of a corner move onto it, so no sliver is left past the corner.
    let snap = |t: f64| at.iter().copied().find(|v| (v - t).abs() < 1.0).unwrap_or(t).clamp(0.0, at[last]);
    let point = |t: f64| {
        let k = at.partition_point(|v| *v <= t).clamp(1, last);
        let span = at[k] - at[k - 1];
        if span <= EPSILON { line[k] } else { line[k - 1].lerp(line[k], (t - at[k - 1]) / span) }
    };
    let mut parts = Vec::new();
    for (a, b) in merged.into_iter().map(|(a, b)| (snap(a), snap(b))) {
        let mut band = vec![point(a)];
        band.extend(line.iter().zip(&at).filter(|(_, s)| **s > a + EPSILON && **s < b - EPSILON).map(|(p, _)| *p));
        band.push(point(b));
        band.dedup_by(|x, y| same(*x, *y));
        let outer = offset_polyline(&band, -r.trim_depth);
        parts.extend(band.windows(2).zip(outer.windows(2)).map(|(i, o)| close(vec![i[0], i[1], o[1], o[0]])).filter(|p| convex(p)));
    }
    parts
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

/// Length of the ramp's center line, over which `drop` and `gap` are spread.
pub fn path_length(r: &Ramp) -> f64 {
    let a = r.angle.to_radians();
    match r.shape {
        Shape::Straight => r.size,
        Shape::Up | Shape::Dip => (r.size + r.height) * a,
        _ => r.size * a,
    }
    .abs()
}

/// The ramp stretched by `s` along its own axes, keeping its angles; a curve stays round, following whichever axis of its plane was stretched more.
pub fn scaled(r: &Ramp, s: DVec3) -> Ramp {
    let more = |a: f64, b: f64| if a.ln().abs() >= b.ln().abs() { a } else { b };
    let (along, across, up) = match r.shape {
        Shape::Straight => (s.x, s.y, s.z),
        Shape::Up | Shape::Down | Shape::Arc | Shape::Dip => (more(s.x, s.z), s.y, more(s.x, s.z)),
        _ => (more(s.x, s.y), more(s.x, s.y), s.z),
    };
    let tip = if r.style == Style::Thin { 0.0 } else { r.tip };
    let slope = DVec2::new(r.width, r.height - tip);
    let stretched = DVec2::new(across * slope.x, up * slope.y).length();
    // A slab's depth across its slope scales with the cross-section's area over the slope's length.
    let depth = across * up * slope.length() / stretched.max(EPSILON);
    let down_slope = stretched / slope.length().max(EPSILON);
    // Settings this shape and style ignore stay as they are.
    let used = |yes: bool, k: f64| if yes { k } else { 1.0 };
    Ramp {
        size: r.size * along,
        gap: r.gap * along,
        end_slope: r.end_slope * used(r.shape == Shape::Straight, along),
        lead_in: r.lead_in * along,
        lead_out: r.lead_out * along,
        width: r.width * across,
        ridge: r.ridge * across,
        height: r.height * up,
        tip: r.tip * used(r.style == Style::Wedge, up),
        thickness: r.thickness * used(r.style == Style::Thin, depth),
        drop: r.drop * s.z,
        rise: r.rise * used(r.shape == Shape::Spiral, s.z),
        trim_top: r.trim_top * down_slope,
        trim_bottom: r.trim_bottom * down_slope,
        trim_depth: r.trim_depth * depth,
        ..r.clone()
    }
}

fn frame(r: &Ramp, t: f64) -> DAffine3 {
    let theta = r.angle.to_radians() * t;
    translation(0.0, 0.0, -r.drop * t)
        * match r.shape {
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
    // A straight ramp needs pieces only when gaps break it up.
    let segments = if r.shape == Shape::Straight && r.gap <= EPSILON { 1 } else { r.smoothness.clamp(CURVED_SEGMENTS_MIN, 256) };
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
    if !closed {
        let slope = if path_length(r) > EPSILON { r.drop / path_length(r) } else { 0.0 };
        if r.lead_in > EPSILON
            && let Some(first) = out.first().copied()
        {
            out.insert(0, translation(0.0, 0.0, slope * r.lead_in) * first * translation(r.lead_in, 0.0, 0.0));
        }
        if r.lead_out > EPSILON
            && let Some(last) = out.last().copied()
        {
            out.push(translation(0.0, 0.0, -slope * r.lead_out) * last * translation(-r.lead_out, 0.0, 0.0));
        }
    }
    out
}

fn local(p: DVec2) -> DVec3 {
    DVec3::new(0.0, p.x, p.y)
}

/// Distance down the outline from its highest corner to each corner, walking whichever way runs downhill.
fn down_from_top(part: &[DVec2]) -> Vec<f64> {
    let n = part.len();
    let top = (0..n).max_by(|i, j| part[*i].y.total_cmp(&part[*j].y)).unwrap_or(0);
    let mut out = vec![f64::INFINITY; n];
    out[top] = 0.0;
    for step in [1, n.saturating_sub(1)] {
        let (mut i, mut d) = (top, 0.0);
        for _ in 1..n {
            let k = (i + step) % n;
            if part[k].y > part[i].y + EPSILON {
                break;
            }
            d += part[i].distance(part[k]);
            out[k] = out[k].min(d);
            i = k;
        }
    }
    out.into_iter().map(|d| if d.is_finite() { d } else { 0.0 }).collect()
}

/// The face through `tri` turned to point along `out`; none when the points are in a line.
fn facing(tri: [DVec3; 3], out: DVec3, slot: Slot, uv: Option<(UvAxis, UvAxis)>) -> Option<Face> {
    let n = (tri[0] - tri[1]).cross(tri[2] - tri[1]);
    if n.length() <= EPSILON {
        return None;
    }
    Some(Face { points: if n.dot(out) < 0.0 { [tri[2], tri[1], tri[0]] } else { tri }, slot, uv })
}

/// Texture flat on a face with v running down it, as Align to Face does.
fn facing_uv(n: DVec3) -> Option<(UvAxis, UvAxis)> {
    let n = n.normalize_or_zero();
    let v = (n * n.z - DVec3::Z).normalize_or_zero();
    (v != DVec3::ZERO && n.z.abs() < 0.999).then(|| ((n.cross(v), 0.0), (v, 0.0)))
}

/// Texture like SURGE's Follow Active Quads: u runs `along` the center line, v down the outline from `below` its top.
fn strip_uv(a: DAffine3, b: DAffine3, along: (f64, f64), (p, q): (DVec2, DVec2), below: (f64, f64), out: DVec3) -> Option<(UvAxis, UvAxis)> {
    let (hi, lo, top) = if p.y >= q.y { (p, q, below.0) } else { (q, p, below.1) };
    let mid = local((p + q) * 0.5);
    let (ma, mb) = (a.transform_point3(mid), b.transform_point3(mid));
    let len = ma.distance(mb);
    if len <= EPSILON {
        return None;
    }
    let dir = (mb - ma) / len;
    let down = a.transform_vector3(local(lo - hi)) + b.transform_vector3(local(lo - hi));
    let v = (down - dir * down.dot(dir)).normalize_or_zero();
    if v == DVec3::ZERO {
        return None;
    }
    let sign = if out.cross(v).dot(dir) < 0.0 { -1.0 } else { 1.0 };
    let step = along.1 - along.0;
    // Inner and outer faces of a curve stretch to the center line's length so their textures line up.
    let u = dir * sign * if step > EPSILON { step / len } else { 1.0 };
    Some(((u, sign * along.0 - ma.dot(u)), (v, top - a.transform_point3(local(hi)).dot(v))))
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
    sweep(r, &parts, &slope_line(r.surf, r.width, r.height, tip, r.ridge))
}

/// The ramp's trim as brushes, empty when it has none.
pub fn trim(r: &Ramp) -> Result<Vec<Piece>, String> {
    let parts = trim_parts(r);
    if parts.is_empty() {
        return Ok(Vec::new());
    }
    sweep(r, &parts, &[])
}

/// Brushes for cross-section `parts` swept along the ramp's path; edges on `line` are surf faces.
fn sweep(r: &Ramp, parts: &[Vec<DVec2>], line: &[DVec2]) -> Result<Vec<Piece>, String> {
    let frames = frames(r);
    let closed = is_closed(r.shape, r.angle, r.rise);
    let steps = if closed { frames.len() } else { frames.len().saturating_sub(1) };
    let mut along = vec![0.0];
    for s in 0..steps {
        along.push(along[s] + frames[s].translation.distance(frames[(s + 1) % frames.len()].translation));
    }
    let mut out = Vec::new();
    for s in 0..steps {
        let (a, b) = (frames[s], frames[(s + 1) % frames.len()]);
        // Each piece gives up half the gap at every end it shares with another piece.
        let half_gap = if r.gap > EPSILON { r.gap * 0.5 / a.translation.distance(b.translation).max(EPSILON) } else { 0.0 };
        let u0 = if closed || s > 0 { half_gap } else { 0.0 };
        let u1 = if closed || s + 1 < steps { 1.0 - half_gap } else { 1.0 };
        if u0 >= u1 {
            return Err("The gap is longer than a piece of the ramp; use fewer segments or a smaller gap.".into());
        }
        for part in parts {
            let end = |u: f64| -> Vec<DVec3> { part.iter().map(|p| a.transform_point3(local(*p)).lerp(b.transform_point3(local(*p)), u)).collect() };
            let (mut pa, mut pb) = (end(u0), end(u1));
            // Sloped ends lean each point along the path by how far below the peak it sits.
            if r.shape == Shape::Straight && r.end_slope.abs() > EPSILON {
                let reach = |p: &DVec2| r.end_slope * (1.0 - p.y / r.height.max(EPSILON));
                if s == 0 {
                    pa.iter_mut().zip(part).for_each(|(q, p)| q.x += reach(p));
                }
                if s + 1 == steps {
                    pb.iter_mut().zip(part).for_each(|(q, p)| q.x -= reach(p));
                }
            }
            let mut faces = Vec::new();
            let area = |t: &[DVec3; 3]| (t[1] - t[0]).cross(t[2] - t[0]).length_squared();
            for (cap, outward) in [(&pa, a.transform_vector3(DVec3::X)), (&pb, b.transform_vector3(-DVec3::X))] {
                let tri = (1..cap.len().saturating_sub(1)).map(|i| [cap[0], cap[i], cap[i + 1]]).max_by(|x, y| area(x).total_cmp(&area(y)));
                if let Some(mut f) = tri.and_then(|t| facing(t, outward, Slot::Side, None)) {
                    f.uv = facing_uv(f.normal());
                    faces.push(f);
                }
            }
            let below = down_from_top(part);
            for j in 0..part.len() {
                let k = (j + 1) % part.len();
                let (p, q) = (part[j], part[k]);
                let slot = edge_slot(p, q, line);
                // Counter-clockwise, so outward is the edge direction turned clockwise.
                let d = q - p;
                let outward = local(DVec2::new(d.y, -d.x));
                let outward = a.transform_vector3(outward) + b.transform_vector3(outward);
                let uv = strip_uv(a, b, (along[s], along[s + 1]), (p, q), (below[j], below[k]), outward).or_else(|| facing_uv(outward));
                let quad = [pa[j], pa[k], pb[k], pb[j]];
                let first = (quad[1] - quad[0]).cross(quad[2] - quad[0]);
                let first = if first.dot(outward) < 0.0 { -first } else { first }.normalize_or_zero();
                let bend = first.dot(quad[3] - quad[0]);
                // A twisted face is two triangles, split along the diagonal that keeps the brush convex.
                let tris: &[[usize; 3]] = if first == DVec3::ZERO {
                    &[[0, 2, 3]]
                } else if bend.abs() <= TWIST {
                    &[[0, 1, 2]]
                } else if bend < 0.0 {
                    &[[0, 1, 2], [0, 2, 3]]
                } else {
                    &[[0, 1, 3], [1, 2, 3]]
                };
                faces.extend(tris.iter().filter_map(|t| facing(t.map(|i| quad[i]), outward, slot, uv)));
            }
            out.push(Piece { faces });
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
        assert_eq!(pieces[0].faces.len(), 6, "two ends, the surf face, the tip, the back and the bottom");
        let n = pieces[0].faces.iter().find(|f| f.slot == Slot::Surf).unwrap().normal();
        assert!(n.z > 0.0 && n.y > 0.0, "a left surf faces up and to +Y: {n:?}");
        let angle = slope_angle(r.width, r.height, r.tip);
        assert!((angle - 51.34).abs() < 0.01, "the default face is the 5:4 slope: {angle}");
        assert!(corners(&pieces[0]).any(|p| (p.x + r.size).abs() < 1e-9));
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

    fn corners(p: &Piece) -> impl Iterator<Item = DVec3> + '_ {
        p.faces.iter().flat_map(|f| f.points)
    }

    fn zs(pieces: &[Piece]) -> (f64, f64) {
        pieces.iter().flat_map(|p| corners(p).map(|q| q.z)).fold((f64::MAX, f64::MIN), |(lo, hi), z| (lo.min(z), hi.max(z)))
    }

    #[test]
    fn drop_and_leads_extend_the_path() {
        let r = Ramp { drop: 256.0, ..Ramp::default() };
        let piece = &build(&r).unwrap()[0];
        let far: Vec<f64> = corners(piece).filter(|p| (p.x + r.size).abs() < 1e-9).map(|p| p.z).collect();
        assert!(far.iter().all(|z| *z <= r.height - 256.0 + 1e-9), "the far end sits lower: {far:?}");
        let curve = Ramp { shape: Shape::Right, smoothness: 18, lead_in: 512.0, lead_out: 256.0, ..Ramp::default() };
        let f = frames(&curve);
        assert_eq!(f.len(), 21, "a lead adds a frame at each end");
        assert!((f[0].translation.x - 512.0).abs() < 1e-9, "the lead in starts behind the curve");
        assert_eq!(build(&curve).unwrap().len(), 20);
    }

    #[test]
    fn ridges_troughs_and_gaps() {
        let ridge = Ramp { surf: Surf::Both, ridge: 64.0, ..Ramp::default() };
        let top: Vec<f64> = profile_parts(&ridge)[0].iter().filter(|p| (p.y - ridge.height).abs() < 1e-9).map(|p| p.x).collect();
        assert_eq!(top.len(), 2, "a ridge flattens the peak");
        assert!((top[0] - top[1]).abs() - 64.0 < 1e-9);

        let trough = Ramp { surf: Surf::Inside, ridge: 128.0, ..Ramp::default() };
        let pieces = build(&trough).unwrap();
        assert_eq!(pieces.len(), 3, "two walls and the floor between them");
        let surf: Vec<DVec3> = pieces.iter().flat_map(|p| p.faces.iter().filter(|f| f.slot == Slot::Surf).map(Face::normal)).collect();
        assert!(surf.iter().any(|n| n.y > 0.1) && surf.iter().any(|n| n.y < -0.1), "the walls face each other");

        let broken = Ramp { gap: 64.0, smoothness: 4, ..Ramp::default() };
        let pieces = build(&broken).unwrap();
        assert_eq!(pieces.len(), 4);
        let ends: Vec<(f64, f64)> = pieces.iter().map(|p| corners(p).map(|q| q.x).fold((f64::MAX, f64::MIN), |(a, b), x| (a.min(x), b.max(x)))).collect();
        assert!((ends[0].0 - ends[1].1 - 64.0).abs() < 1e-9, "pieces stand a gap apart: {ends:?}");
        assert!(build(&Ramp { gap: 4096.0, smoothness: 4, ..Ramp::default() }).is_err());
    }

    #[test]
    fn sloped_ends_and_pyramids() {
        let hip = Ramp { surf: Surf::Both, end_slope: 256.0, ..Ramp::default() };
        let p = &build(&hip).unwrap()[0];
        let at = |z: f64| corners(p).filter(|q| (q.z - z).abs() < 1e-9).map(|q| q.x).fold(f64::MIN, f64::max);
        assert!((at(0.0) - 256.0).abs() < 1e-9, "the base reaches past the start");
        assert!(at(hip.height).abs() < 1e-9, "the peak stays put");
        let pyramid = Ramp { surf: Surf::Both, size: 0.0, end_slope: 512.0, ..Ramp::default() };
        let (lo, hi) = zs(&build(&pyramid).unwrap());
        assert_eq!((lo, hi), (0.0, pyramid.height));
    }

    #[test]
    fn stretched_settings_build_the_stretched_ramp() {
        let r = Ramp { surf: Surf::Both, ridge: 32.0, end_slope: 128.0, drop: 64.0, ..Ramp::default() };
        let s = DVec3::new(1.5, 2.0, 0.75);
        let want: Vec<DVec3> = build(&r).unwrap().iter().flat_map(|p| corners(p).map(|q| q * s).collect::<Vec<_>>()).collect();
        let got: Vec<DVec3> = build(&scaled(&r, s)).unwrap().iter().flat_map(|p| corners(p).collect::<Vec<_>>()).collect();
        assert_eq!(want.len(), got.len());
        assert!(want.iter().zip(&got).all(|(a, b)| a.distance(*b) < 1e-6), "{want:?} {got:?}");

        let curve = scaled(&Ramp { shape: Shape::Right, ..Ramp::default() }, DVec3::new(2.0, 1.2, 1.0));
        assert_eq!((curve.size, curve.width, curve.height, curve.angle), (2048.0, 768.0, 544.0, 90.0), "a curve stays round");

        let t = Ramp { trim_top: 64.0, trim_depth: 2.0, ..Ramp::default() };
        let low = |r: &Ramp| zs(&trim(r).unwrap()).0;
        assert!((low(&scaled(&t, s)) - low(&t) * s.z).abs() < 1e-6, "the band keeps its place down the stretched face");
    }

    #[test]
    fn trim_is_off_by_default_and_leaves_the_collision_alone() {
        assert!(trim(&Ramp::default()).unwrap().is_empty());
        let points = |r: &Ramp| build(r).unwrap().iter().flat_map(|p| corners(p).collect::<Vec<_>>()).collect::<Vec<_>>();
        for r in [Ramp::default(), Ramp { shape: Shape::Spiral, surf: Surf::Both, bank: 10.0, ..Ramp::default() }] {
            let trimmed = Ramp { trim_top: 64.0, trim_bottom: 32.0, trim_depth: 4.0, ..r.clone() };
            assert!(!trim(&trimmed).unwrap().is_empty());
            assert_eq!(points(&r), points(&trimmed), "the ramp's own brushes do not change");
        }
    }

    #[test]
    fn trim_bands_sit_on_the_surf_face() {
        let r = Ramp { trim_top: 64.0, trim_bottom: 32.0, trim_depth: 2.0, ..Ramp::default() };
        let face = build(&r).unwrap().remove(0).faces.into_iter().find(|f| f.slot == Slot::Surf).unwrap();
        let (n, o) = (face.normal(), face.points[0]);
        let pieces = trim(&r).unwrap();
        assert_eq!(pieces.len(), 2, "a band at the top and one at the bottom");
        let sin = (r.height - r.tip) / DVec2::new(r.width, r.height - r.tip).length();
        let mut bands: Vec<(f64, f64)> = pieces
            .iter()
            .map(|p| {
                assert!(corners(p).all(|q| (-1e-6..=2.0 + 1e-6).contains(&(q - o).dot(n))), "the band stands off the face, never into it");
                assert!(corners(p).any(|q| ((q - o).dot(n) - 2.0).abs() < 1e-6) && p.faces.iter().any(|f| f.normal().dot(n) > 1.0 - 1e-9));
                corners(p).filter(|q| (*q - o).dot(n).abs() < 1e-6).fold((f64::MAX, f64::MIN), |(lo, hi), q| (lo.min(q.z), hi.max(q.z)))
            })
            .collect();
        bands.sort_by(|a, b| a.0.total_cmp(&b.0));
        assert!((bands[0].0 - r.tip).abs() < 1e-6 && (bands[0].1 - r.tip - 32.0 * sin).abs() < 1e-6, "{bands:?}");
        assert!((bands[1].0 - r.height + 64.0 * sin).abs() < 1e-6 && (bands[1].1 - r.height).abs() < 1e-6, "{bands:?}");
    }

    #[test]
    fn trim_covers_both_sides_peaks_and_floors() {
        let both = Ramp { surf: Surf::Both, trim_top: 64.0, trim_bottom: 32.0, ..Ramp::default() };
        let parts = trim_parts(&both);
        assert_eq!(parts.len(), 4, "two halves of the cap and a band at each foot");
        assert_eq!(parts.iter().filter(|p| p.iter().map(|q| q.x).sum::<f64>() < 0.0).count(), 2, "as much on each side");
        let top = parts.iter().flatten().map(|p| p.y).fold(f64::MIN, f64::max);
        assert!(top > both.height + both.trim_depth, "the cap stands over the pointy peak: {top}");
        assert_eq!(trim_parts(&Ramp { ridge: 64.0, trim_bottom: 0.0, ..both }).len(), 3, "a flat ridge is capped whole");
        let trough = Ramp { surf: Surf::Inside, ridge: 128.0, trim_bottom: 32.0, ..Ramp::default() };
        assert_eq!(trim_parts(&trough).len(), 3, "the floor and the foot of each wall");
        assert!(trim_parts(&Ramp { trim_top: 64.0, trim_depth: 0.0, ..Ramp::default() }).is_empty());
    }

    #[test]
    fn trim_follows_curves_piece_by_piece() {
        let r = Ramp { shape: Shape::Right, smoothness: 16, trim_top: 48.0, trim_depth: 2.0, ..Ramp::default() };
        let (ramp, band) = (build(&r).unwrap(), trim(&r).unwrap());
        assert_eq!(band.len(), ramp.len());
        for (piece, strip) in ramp.iter().zip(&band) {
            let face = piece.faces.iter().find(|f| f.slot == Slot::Surf).unwrap();
            let (n, o) = (face.normal(), face.points[0]);
            assert!(strip.faces.iter().any(|f| f.normal().dot(n) > 1.0 - 1e-9), "the band's face is parallel to the surf face");
            assert!(corners(strip).all(|q| (-1e-6..=2.0 + 1e-6).contains(&(q - o).dot(n))));
        }
        for w in band.windows(2) {
            let mut shared: Vec<DVec3> = Vec::new();
            for p in corners(&w[0]).filter(|p| corners(&w[1]).any(|q| p.distance(q) < 1e-6)) {
                if !shared.iter().any(|s| s.distance(p) < 1e-6) {
                    shared.push(p);
                }
            }
            assert_eq!(shared.len(), 4, "neighboring pieces share the whole end of the band");
        }
    }

    #[test]
    fn curved_textures_follow_the_path_and_meet_at_seams() {
        let r = Ramp { shape: Shape::Right, smoothness: 16, ..Ramp::default() };
        let pieces = build(&r).unwrap();
        let surf: Vec<&Face> = pieces.iter().map(|p| p.faces.iter().find(|f| f.slot == Slot::Surf).unwrap()).collect();
        let tex = |f: &Face, p: DVec3| {
            let ((u, uo), (v, vo)) = f.uv.unwrap();
            DVec2::new(p.dot(u) + uo, p.dot(v) + vo)
        };
        let line = slope_line(r.surf, r.width, r.height, r.tip, r.ridge);
        let mid = local((line[0] + line[1]) * 0.5);
        let f = frames(&r);
        for (i, w) in surf.windows(2).enumerate() {
            let seam = f[i + 1].transform_point3(mid);
            assert!(tex(w[0], seam).distance(tex(w[1], seam)) < 1e-6, "segment {i} meets the next");
        }
        for face in &surf {
            let ((_, _), (v, _)) = face.uv.unwrap();
            let n = face.normal();
            assert!(v.dot((n * n.z - DVec3::Z).normalize()) > 0.999, "v runs straight down every face");
        }
        let (first, last) = (tex(surf[0], f[0].transform_point3(mid)), tex(surf[15], f[16].transform_point3(mid)));
        assert!(((last.x - first.x).abs() - path_length(&r)).abs() < 2.0, "u runs the length of the center line");
    }
}
