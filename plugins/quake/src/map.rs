//! Reads Quake `.map` files: entities with their keyvalues in file order, and brushes in the Standard or Valve220 format.

use glam::DVec3;

use crate::mapcraft::plugin::types::{Face, TexAxis, Vec3};

/// Quake's texture projection: a face takes the u and v axes of the first entry whose normal is closest to its own.
const BASE_AXES: [(DVec3, DVec3, DVec3); 6] = [
    (DVec3::Z, DVec3::X, DVec3::NEG_Y),
    (DVec3::NEG_Z, DVec3::X, DVec3::NEG_Y),
    (DVec3::X, DVec3::Y, DVec3::NEG_Z),
    (DVec3::NEG_X, DVec3::Y, DVec3::NEG_Z),
    (DVec3::Y, DVec3::X, DVec3::NEG_Z),
    (DVec3::NEG_Y, DVec3::X, DVec3::NEG_Z),
];

#[derive(Default)]
pub struct MapEntity {
    pub properties: Vec<(String, String)>,
    pub brushes: Vec<Vec<Face>>,
}

impl MapEntity {
    /// The last value of `name`, which is the one Quake keeps.
    pub fn key(&self, name: &str) -> Option<&str> {
        self.properties.iter().rev().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

pub struct ParsedMap {
    pub entities: Vec<MapEntity>,
    /// Faces whose three points lie on one line and so define no plane; they are left out.
    pub degenerate: usize,
}

struct Lexer<'a> {
    text: &'a str,
    at: usize,
}

impl<'a> Lexer<'a> {
    fn error(&self, what: &str) -> String {
        let line = self.text.as_bytes().get(..self.at).map_or(0, |b| b.iter().filter(|&&c| c == b'\n').count()) + 1;
        format!("line {line}: {what}")
    }

    /// The next byte after whitespace and `//` comments.
    fn peek(&mut self) -> Option<u8> {
        let bytes = self.text.as_bytes();
        while let Some(&b) = bytes.get(self.at) {
            if b.is_ascii_whitespace() {
                self.at += 1;
            } else if b == b'/' && bytes.get(self.at + 1) == Some(&b'/') {
                self.at = bytes.get(self.at..).and_then(|rest| rest.iter().position(|&c| c == b'\n')).map_or(bytes.len(), |n| self.at + n);
            } else {
                return Some(b);
            }
        }
        None
    }

    fn eat(&mut self, c: u8) -> bool {
        let found = self.peek() == Some(c);
        self.at += usize::from(found);
        found
    }

    fn expect(&mut self, c: u8) -> Result<(), String> {
        if self.eat(c) { Ok(()) } else { Err(self.error(&format!("expected `{}`", char::from(c)))) }
    }

    /// Characters up to whitespace or, with `delimited`, a bracket, brace or quote.
    fn token(&mut self, delimited: bool) -> &'a str {
        self.peek();
        let start = self.at;
        let bytes = self.text.as_bytes();
        while let Some(&b) = bytes.get(self.at) {
            if b.is_ascii_whitespace() || (delimited && b"()[]{}\"".contains(&b)) {
                break;
            }
            self.at += 1;
        }
        self.text.get(start..self.at).unwrap_or_default()
    }

    /// A quoted string; Quake has no escapes, so it ends at the next quote.
    fn quoted(&mut self) -> Result<&'a str, String> {
        self.expect(b'"')?;
        let start = self.at;
        let len =
            self.text.as_bytes().get(start..).and_then(|rest| rest.iter().position(|&c| c == b'"')).ok_or_else(|| self.error("a quoted string never ends"))?;
        self.at = start + len + 1;
        Ok(self.text.get(start..start + len).unwrap_or_default())
    }

    fn number(&mut self) -> Result<f64, String> {
        let token = self.token(true);
        token.parse::<f64>().ok().filter(|n| n.is_finite()).ok_or_else(|| self.error(&format!("expected a number, found `{token}`")))
    }

    fn texture(&mut self) -> Result<String, String> {
        let name = if self.peek() == Some(b'"') { self.quoted()? } else { self.token(false) };
        if name.is_empty() {
            return Err(self.error("expected a texture name"));
        }
        Ok(name.to_ascii_lowercase())
    }
}

fn v3(p: DVec3) -> Vec3 {
    Vec3 { x: p.x, y: p.y, z: p.z }
}

/// Quake reads a zero scale as 1.
fn scale(s: f64) -> f64 {
    if s == 0.0 { 1.0 } else { s }
}

/// The texture axes a Standard format face gets from its normal, offsets, rotation in degrees and scales.
pub fn standard_axes(normal: DVec3, [x_offset, y_offset, rotation, x_scale, y_scale]: [f64; 5]) -> (TexAxis, TexAxis) {
    let first = BASE_AXES[0];
    let (_, u, v) = BASE_AXES.into_iter().fold(first, |best, e| if e.0.dot(normal) > best.0.dot(normal) { e } else { best });
    let r = rotation.rem_euclid(360.0);
    let (sin, cos) = if r % 90.0 == 0.0 { [(0.0, 1.0), (1.0, 0.0), (0.0, -1.0), (-1.0, 0.0)][(r / 90.0) as usize % 4] } else { r.to_radians().sin_cos() };
    // The rotation turns the plane of the two world axes the base u and v lie along, whichever way the face points.
    let along = |w: DVec3| {
        if w.x != 0.0 {
            0
        } else if w.y != 0.0 {
            1
        } else {
            2
        }
    };
    let (a, b) = (along(u), along(v));
    let turn = |w: DVec3| {
        let mut out = w.to_array();
        let (x, y) = (w[a], w[b]);
        out[a] = cos * x - sin * y;
        out[b] = sin * x + cos * y;
        DVec3::from_array(out)
    };
    (TexAxis { axis: v3(turn(u)), offset: x_offset, scale: scale(x_scale) }, TexAxis { axis: v3(turn(v)), offset: y_offset, scale: scale(y_scale) })
}

/// `x y z offset ]` of a Valve220 axis, after its `[`.
fn valve_axis(lx: &mut Lexer) -> Result<(DVec3, f64), String> {
    let axis = DVec3::new(lx.number()?, lx.number()?, lx.number()?);
    let offset = lx.number()?;
    lx.expect(b']')?;
    Ok((axis, offset))
}

/// One face line; `None` when its points define no plane.
fn face(lx: &mut Lexer) -> Result<Option<Face>, String> {
    let mut points = [DVec3::ZERO; 3];
    for p in &mut points {
        lx.expect(b'(')?;
        *p = DVec3::new(lx.number()?, lx.number()?, lx.number()?);
        lx.expect(b')')?;
    }
    let [p0, p1, p2] = points;
    let material = lx.texture()?;
    let cross = (p0 - p1).cross(p2 - p1);
    let uv = if lx.eat(b'[') {
        let (u, u_offset) = valve_axis(lx)?;
        lx.expect(b'[')?;
        let (v, v_offset) = valve_axis(lx)?;
        let [_rotation, u_scale, v_scale] = [lx.number()?, lx.number()?, lx.number()?];
        (TexAxis { axis: v3(u), offset: u_offset, scale: scale(u_scale) }, TexAxis { axis: v3(v), offset: v_offset, scale: scale(v_scale) })
    } else {
        let values = [lx.number()?, lx.number()?, lx.number()?, lx.number()?, lx.number()?];
        standard_axes(cross.normalize_or_zero(), values)
    };
    // Quake 2, Hexen 2 and others add surface flags and values after the texture.
    while matches!(lx.peek(), Some(b'0'..=b'9' | b'-' | b'+' | b'.')) {
        lx.number()?;
    }
    let length = cross.length();
    Ok((length.is_finite() && length > 1e-9).then(|| Face { points: (v3(p0), v3(p1), v3(p2)), material, uv: Some(uv), attrs: Vec::new(), disp: None }))
}

fn brush(lx: &mut Lexer, degenerate: &mut usize) -> Result<Vec<Face>, String> {
    let mut faces = Vec::new();
    loop {
        match lx.peek() {
            Some(b'(') => match face(lx)? {
                Some(f) => faces.push(f),
                None => *degenerate += 1,
            },
            Some(b'}') => {
                lx.at += 1;
                return Ok(faces);
            }
            Some(_) => return Err(lx.error("expected a face or `}` (Quake 3 brush primitives and patches are not supported)")),
            None => return Err(lx.error("the file ends inside a brush")),
        }
    }
}

pub fn parse(text: &str) -> Result<ParsedMap, String> {
    let mut lx = Lexer { text, at: 0 };
    let mut map = ParsedMap { entities: Vec::new(), degenerate: 0 };
    while lx.peek().is_some() {
        lx.expect(b'{')?;
        let mut entity = MapEntity::default();
        loop {
            match lx.peek() {
                Some(b'"') => {
                    let key = lx.quoted()?.to_string();
                    entity.properties.push((key, lx.quoted()?.to_string()));
                }
                Some(b'{') => {
                    lx.at += 1;
                    entity.brushes.push(brush(&mut lx, &mut map.degenerate)?);
                }
                Some(b'}') => {
                    lx.at += 1;
                    break;
                }
                Some(_) => return Err(lx.error("expected a key, a brush or `}`")),
                None => return Err(lx.error("the file ends inside an entity")),
            }
        }
        map.entities.push(entity);
    }
    Ok(map)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn dv(v: &Vec3) -> DVec3 {
        DVec3::new(v.x, v.y, v.z)
    }

    fn texel(a: &TexAxis, p: DVec3) -> f64 {
        p.dot(dv(&a.axis)) / a.scale + a.offset
    }

    /// The part of a polygon behind a plane.
    fn clip(polygon: &[DVec3], normal: DVec3, dist: f64) -> Vec<DVec3> {
        let mut out = Vec::new();
        for (&a, &b) in polygon.iter().zip(polygon.iter().cycle().skip(1)) {
            let (da, db) = (normal.dot(a) - dist, normal.dot(b) - dist);
            if da <= 1e-6 {
                out.push(a);
            }
            if (da > 1e-6 && db < -1e-6) || (da < -1e-6 && db > 1e-6) {
                out.push(a + (b - a) * (da / (da - db)));
            }
        }
        out
    }

    /// Builds the brush from its face planes and checks it is a bounded, closed solid whose face normals all point out.
    pub fn assert_solid(faces: &[Face]) {
        let planes: Vec<(DVec3, f64)> = faces
            .iter()
            .map(|f| {
                let (a, b, c) = (dv(&f.points.0), dv(&f.points.1), dv(&f.points.2));
                let n = (a - b).cross(c - b).normalize();
                (n, n.dot(b))
            })
            .collect();
        let polygons: Vec<Vec<DVec3>> = planes
            .iter()
            .enumerate()
            .map(|(i, &(n, d))| {
                let side = n.cross(if n.z.abs() < 0.9 { DVec3::Z } else { DVec3::X }).normalize() * 1e5;
                let up = n.cross(side);
                let c = n * d;
                let square = vec![c + side + up, c - side + up, c - side - up, c + side - up];
                planes.iter().enumerate().filter(|&(j, _)| j != i).fold(square, |w, (_, &(m, e))| clip(&w, m, e))
            })
            .filter(|w| w.len() >= 3)
            .collect();
        assert!(polygons.len() >= 4, "only {} faces survive", polygons.len());
        let points: Vec<DVec3> = polygons.iter().flatten().copied().collect();
        assert!(points.iter().all(|p| p.abs().max_element() < 5e4), "the brush is open");
        let area: DVec3 = polygons.iter().flat_map(|w| w.iter().zip(w.iter().cycle().skip(1)).map(|(a, b)| a.cross(*b))).sum();
        let total: f64 = polygons.iter().map(|w| w.iter().zip(w.iter().cycle().skip(1)).map(|(a, b)| a.cross(*b).length()).sum::<f64>()).sum();
        assert!(area.length() <= total * 1e-6, "the brush does not close");
        let middle = points.iter().sum::<DVec3>() / points.len() as f64;
        assert!(planes.iter().all(|&(n, d)| n.dot(middle) < d), "a face points inward");
    }

    const CUBE: &str = r#"// Game: Quake
// Format: Standard
{
"classname" "worldspawn"
"message" "Test"
"message" "Kept twice"
{
( 0 64 64 ) ( 0 0 64 ) ( 0 0 0 ) Ewall1 0 0 0 1 1
( 64 0 64 ) ( 64 64 64 ) ( 64 64 0 ) *Water1 16 8 30 0.5 2
( 0 0 64 ) ( 64 0 64 ) ( 64 0 0 ) +0Button 0 0 0 1 1
( 64 64 64 ) ( 0 64 64 ) ( 0 64 0 ) {Grate 0 0 0 1 1 0 0 0
( 0 64 0 ) ( 0 0 0 ) ( 64 0 0 ) "sky space" 0 0 90 1 -1
(0 0 64) (0 64 64) (64 64 64) floor -4 3 -15 2 2
}
}
{
"classname" "func_door"
"angle" "-1"
{
( 0 0 0 ) ( 0 1 0 ) ( 0 2 0 ) degenerate 0 0 0 1 1
( 128 64 32 ) ( 128 0 32 ) ( 128 0 0 ) base [ 0 -1 0 8 ] [ 0 0 -1 0 ] 0 1 1
( 192 0 32 ) ( 192 64 32 ) ( 192 64 0 ) base [ 0 1 0 0 ] [ 0 0 -1 0 ] 0 1 1
( 128 0 32 ) ( 192 0 32 ) ( 192 0 0 ) base [ 1 0 0 0 ] [ 0 0 -1 0 ] 0 1 1
( 192 64 32 ) ( 128 64 32 ) ( 128 64 0 ) base [ -1 0 0 0 ] [ 0 0 -1 0 ] 0 1 1
( 128 64 0 ) ( 128 0 0 ) ( 192 0 0 ) base [ -1 0 0 0 ] [ 0 -1 0 0 ] 0 1 1
( 128 0 32 ) ( 128 64 32 ) ( 192 64 32 ) base [ 0.7071 0.7071 0 0 ] [ 0.7071 -0.7071 0 0 ] 45 0.25 0
}
}
{ "classname" "light" "origin" "32 32 48" "light" "300" }
"#;

    #[test]
    fn parses_standard_and_valve220() {
        let map = parse(CUBE).unwrap();
        assert_eq!(map.entities.len(), 3);
        assert_eq!(map.degenerate, 1);
        let world = &map.entities[0];
        assert_eq!(world.properties[1..], [("message".into(), "Test".into()), ("message".into(), "Kept twice".into())]);
        assert_eq!(world.key("message"), Some("Kept twice"));
        let names: Vec<&str> = world.brushes[0].iter().map(|f| f.material.as_str()).collect();
        assert_eq!(names, ["ewall1", "*water1", "+0button", "{grate", "sky space", "floor"]);
        let door = &map.entities[1].brushes[0];
        assert_eq!(door.len(), 6);
        let (u, v) = door[0].uv.as_ref().unwrap();
        assert_eq!((dv(&u.axis), u.offset, u.scale, dv(&v.axis)), (DVec3::NEG_Y, 8.0, 1.0, DVec3::NEG_Z));
        let (u, v) = door[5].uv.as_ref().unwrap();
        assert_eq!((u.scale, v.scale), (0.25, 1.0));
        assert!(map.entities[2].brushes.is_empty());
        assert_eq!(map.entities[2].key("light"), Some("300"));
        for brush in map.entities.iter().flat_map(|e| &e.brushes) {
            assert_solid(brush);
        }
    }

    #[test]
    fn inverted_faces_are_not_a_solid() {
        let mut brush = parse(CUBE).unwrap().entities.swap_remove(0).brushes.swap_remove(0);
        for f in &mut brush {
            f.points = (f.points.2, f.points.1, f.points.0);
        }
        assert!(std::panic::catch_unwind(|| assert_solid(&brush)).is_err());
    }

    fn axes(normal: DVec3, values: [f64; 5]) -> (DVec3, DVec3) {
        let (u, v) = standard_axes(normal.normalize(), values);
        (dv(&u.axis), dv(&v.axis))
    }

    #[test]
    fn standard_axes_follow_quake() {
        let near = |(a, b): (DVec3, DVec3), (c, d): (DVec3, DVec3)| assert!(a.abs_diff_eq(c, 1e-12) && b.abs_diff_eq(d, 1e-12), "{a} {b} vs {c} {d}");
        near(axes(DVec3::Z, [0.0, 0.0, 0.0, 1.0, 1.0]), (DVec3::X, DVec3::NEG_Y));
        near(axes(DVec3::NEG_Z, [0.0, 0.0, 90.0, 1.0, 1.0]), (DVec3::Y, DVec3::X));
        near(axes(DVec3::X, [0.0, 0.0, 90.0, 1.0, 1.0]), (DVec3::Z, DVec3::Y));
        near(axes(DVec3::NEG_Y, [0.0, 0.0, -90.0, 1.0, 1.0]), (DVec3::NEG_Z, DVec3::NEG_X));
        near(axes(DVec3::Y, [0.0, 0.0, 180.0, 1.0, 1.0]), (DVec3::NEG_X, DVec3::Z));
        // A 45 degree slope is a tie that the floor wins; past it the wall does.
        near(axes(DVec3::new(1.0, 0.0, 1.0), [0.0; 5]), (DVec3::X, DVec3::NEG_Y));
        near(axes(DVec3::new(1.0, 0.0, 0.99), [0.0; 5]), (DVec3::Y, DVec3::NEG_Z));
        near(axes(DVec3::new(-1.0, -1.0, 0.0), [0.0; 5]), (DVec3::Y, DVec3::NEG_Z));
        let (s, c) = 30f64.to_radians().sin_cos();
        near(axes(DVec3::Z, [0.0, 0.0, 30.0, 1.0, 1.0]), (DVec3::new(c, s, 0.0), DVec3::new(s, -c, 0.0)));
        let (u, v) = standard_axes(DVec3::Z, [3.0, -4.0, 0.0, 0.0, -2.0]);
        assert_eq!((u.offset, u.scale, v.offset, v.scale), (3.0, 1.0, -4.0, -2.0));
    }

    #[test]
    fn standard_and_valve220_agree() {
        // The same faces written both ways: Valve220 states the rotated base axes Standard derives.
        let (s, c) = 30f64.to_radians().sin_cos();
        let text = format!(
            "{{ \"classname\" \"worldspawn\"\n{{\n\
             ( 0 0 64 ) ( 0 64 64 ) ( 64 64 64 ) a 7 -3 30 0.5 -2\n\
             ( 0 0 64 ) ( 0 64 64 ) ( 64 64 64 ) a [ {c} {s} 0 7 ] [ {s} {n} 0 -3 ] 30 0.5 -2\n\
             ( 64 0 64 ) ( 64 64 64 ) ( 64 64 0 ) a 0 5 120 1 4\n\
             ( 64 0 64 ) ( 64 64 64 ) ( 64 64 0 ) a [ 0 -0.5 0.8660254037844387 0 ] [ 0 0.8660254037844387 0.5 5 ] 120 1 4\n\
             }}\n}}",
            n = -c
        );
        let map = parse(&text).unwrap();
        let faces = &map.entities[0].brushes[0];
        for [standard, valve] in faces.as_chunks::<2>().0 {
            let ((su, sv), (vu, vv)) = (standard.uv.as_ref().unwrap(), valve.uv.as_ref().unwrap());
            for p in [DVec3::ZERO, DVec3::new(64.0, 0.0, 64.0), DVec3::new(13.0, -200.0, 7.5)] {
                assert!((texel(su, p) - texel(vu, p)).abs() < 1e-9 && (texel(sv, p) - texel(vv, p)).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn rejects_garbage_without_panicking() {
        for bad in [
            "{",
            "{ \"classname\" ",
            "{ \"a\" \"b\" { ( 0 0 0 ) ( 1 0 0 ) ( 0 1 ) x 0 0 0 1 1 } }",
            "}",
            "{ { ( 0 0 0 ) ( 1 0 0 ) ( 0 1 0 ) x [ 1 0 0 0 ] 0 0 0 } }",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        assert!(parse("{ { brushDef { } } }").is_err_and(|e| e.contains("Quake 3")));
        assert!(parse("{ { ( 0 0 0 ) ( 1 0 0 ) ( 0 1 0 ) x nan 0 0 1 1 } }").is_err());
        assert_eq!(parse("// only a comment").unwrap().entities.len(), 0);
        for n in 0..=CUBE.len() {
            if let Some(cut) = CUBE.get(..n) {
                let _ = parse(cut);
            }
        }
        let mut seed = 0x9e37_79b9_u32;
        for _ in 0..500 {
            let text: String = (0..64)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    let table = b"{}()[]\" 0123456789.-e/\nabc";
                    char::from(table[seed as usize % table.len()])
                })
                .collect();
            let _ = parse(&text);
        }
    }

    pub const MAPS: &str = "/mnt/Storage/Quake/Maps";

    pub fn marks_maps() -> Vec<std::path::PathBuf> {
        let dirs = [std::path::PathBuf::from(MAPS), std::path::PathBuf::from(MAPS).join("autosave")];
        let mut maps: Vec<_> = dirs
            .iter()
            .flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "map"))
            .collect();
        maps.sort();
        maps
    }

    #[test]
    fn marks_maps_build_closed_solids() {
        let maps = marks_maps();
        if maps.is_empty() {
            eprintln!("skipped: no maps in {MAPS}");
            return;
        }
        for path in maps {
            let map = parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
            let brushes: Vec<&Vec<Face>> = map.entities.iter().flat_map(|e| &e.brushes).collect();
            for brush in &brushes {
                assert_solid(brush);
            }
            let faces: usize = brushes.iter().map(|b| b.len()).sum();
            eprintln!("{}: {} entities, {} brushes, {faces} faces", path.display(), map.entities.len(), brushes.len());
            assert_eq!(map.degenerate, 0);
        }
    }

    /// The texture vectors qbsp stored for each face it kept: `u.axis / u.scale, u.offset, v.axis / v.scale, v.offset`.
    pub fn bsp_texinfo(bsp: &[u8]) -> Vec<[f64; 8]> {
        let at = |i: usize| u32::from_le_bytes(bsp[i..i + 4].try_into().unwrap()) as usize;
        let (offset, length) = (at(4 + 6 * 8), at(8 + 6 * 8));
        bsp[offset..offset + length]
            .as_chunks::<40>()
            .0
            .iter()
            .map(|c| std::array::from_fn(|i| f64::from(f32::from_le_bytes(c[i * 4..i * 4 + 4].try_into().unwrap()))))
            .collect()
    }

    #[test]
    fn texture_axes_match_what_qbsp_compiled() {
        let (Ok(text), Ok(bsp)) =
            (std::fs::read_to_string(format!("{MAPS}/mc_q30th_bearlikelion.map")), std::fs::read(format!("{MAPS}/mc_q30th_bearlikelion.bsp")))
        else {
            eprintln!("skipped: Mark's map or its BSP is missing");
            return;
        };
        let map = parse(&text).unwrap();
        let ours: Vec<[f64; 8]> = map
            .entities
            .iter()
            .flat_map(|e| e.brushes.iter().flatten())
            .map(|f| {
                let (u, v) = f.uv.as_ref().unwrap();
                let (a, b) = (dv(&u.axis) / u.scale, dv(&v.axis) / v.scale);
                [a.x, a.y, a.z, u.offset, b.x, b.y, b.z, v.offset]
            })
            .collect();
        let compiled = bsp_texinfo(&bsp);
        assert!(!compiled.is_empty());
        for info in &compiled {
            assert!(ours.iter().any(|o| o.iter().zip(info).all(|(a, b)| (a - b).abs() < 1e-4)), "qbsp made {info:?}, which no face here has");
        }
    }
}
