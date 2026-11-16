//! A zone's S3D read into groups: drawn geometry, passable geometry, invisible walls, objects and lights.

use std::collections::BTreeMap;

use glam::{DVec2, DVec3};

use crate::image::{Picture, decode};
use crate::mapcraft::plugin::types::{BlendRole, Brush, Entity, Face, Group, Image, MaterialInfo, Output, TexAxis, Vec3};
use crate::merge::{Tri, merge};
use crate::pfs::Archive;
use crate::wld::{MESH, Material, Mesh, Wld};

/// How far each surface's brush reaches behind it, in EverQuest units.
const DEPTH: f64 = 0.5;
/// Triangles thinner than this are slivers that make no brush.
const MIN_HEIGHT: f64 = 0.01;
/// Each brush takes about 4 KB of plugin memory on its way to the editor; this fits the 512 MiB an import gets.
const MAX_BRUSHES: usize = 100_000;
pub const BOUNDARY: &str = "everquest/boundary";
/// A heading or tilt of 512 is a full turn.
const TURN: f64 = 360.0 / 512.0;
/// Units per texel of an axis the texture does not change along: across a whole zone it moves a hundredth of a texel.
const FLAT_SCALE: f64 = 2.0e6;

pub struct Imported {
    pub output: Output,
    pub log: Vec<String>,
}

/// One drawn surface: a convex polygon counter-clockwise about `normal`.
struct Surface {
    points: Vec<DVec3>,
    normal: DVec3,
    material: String,
    uv: Option<(TexAxis, TexAxis)>,
}

fn v3(p: DVec3) -> Vec3 {
    Vec3 { x: p.x, y: p.y, z: p.z }
}

fn dv(p: [f32; 3]) -> DVec3 {
    DVec3::new(p[0].into(), p[1].into(), p[2].into())
}

/// A face on the plane through `at` facing `normal`, its points clockwise seen from outside.
fn face(normal: DVec3, at: DVec3, material: &str, uv: Option<(TexAxis, TexAxis)>) -> Face {
    let a = normal.abs();
    let up = if a.z > a.x && a.z > a.y { DVec3::X } else { DVec3::Z };
    let up = (up - normal * up.dot(normal)).normalize_or_zero();
    let right = up.cross(normal);
    Face { points: (v3(at + right * 64.0), v3(at), v3(at + up * 64.0)), material: material.to_string(), uv, attrs: Vec::new(), disp: None }
}

/// The surface pushed `DEPTH` back into a slab: the surface itself, its back and a side per edge.
fn slab(s: &Surface) -> Brush {
    let mut faces = vec![face(s.normal, s.points[0], &s.material, s.uv), face(-s.normal, s.points[0] - s.normal * DEPTH, &s.material, None)];
    for (&p, &q) in s.points.iter().zip(s.points.iter().cycle().skip(1)) {
        faces.push(face((q - p).cross(s.normal).normalize(), p, &s.material, None));
    }
    Brush::Faces(faces)
}

/// Valve220 axes that put texel `uv[i]` at `p[i]`; `None` for a degenerate triangle.
pub(crate) fn tex_axes(p: [DVec3; 3], uv: [DVec2; 3]) -> Option<(TexAxis, TexAxis)> {
    let (e1, e2) = (p[1] - p[0], p[2] - p[0]);
    let (a, b, c) = (e1.dot(e1), e1.dot(e2), e2.dot(e2));
    let det = a * c - b * b;
    if det.abs() < 1e-12 {
        return None;
    }
    let (d1, d2) = (uv[1] - uv[0], uv[2] - uv[0]);
    let gradient = |d1: f64, d2: f64| e1 * ((c * d1 - b * d2) / det) + e2 * ((a * d2 - b * d1) / det);
    let (gu, gv) = (gradient(d1.x, d2.x), gradient(d1.y, d2.y));
    let normal = e1.cross(e2).normalize();
    let axis = |g: DVec3, other: DVec3, at: f64| {
        let len = g.length();
        if len > 1e-9 && len.is_finite() {
            return TexAxis { axis: v3(g / len), offset: at - g.dot(p[0]), scale: 1.0 / len };
        }
        // EverQuest stretches one texel along trim; a vast scale keeps it all but constant.
        let along = normal.cross(other).try_normalize().unwrap_or_else(|| e1.normalize());
        TexAxis { axis: v3(along), offset: at - along.dot(p[0]) / FLAT_SCALE, scale: FLAT_SCALE }
    };
    Some((axis(gu, gv, uv[0].x), axis(gv, gu, uv[0].y)))
}

/// The triangle's height over its longest side.
fn height(p: [DVec3; 3]) -> f64 {
    let longest = (p[1] - p[0]).length().max((p[2] - p[1]).length()).max((p[0] - p[2]).length());
    if longest > 0.0 { (p[1] - p[0]).cross(p[2] - p[0]).length() / longest } else { 0.0 }
}

/// Pictures and names of the materials a zone or object uses.
#[derive(Default)]
pub struct Materials {
    pub by_bitmap: BTreeMap<String, (String, Option<Picture>)>,
}

impl Materials {
    /// The material's name and the picture's size, reading the picture from `archive` the first time.
    pub fn get(&mut self, folder: &str, m: &Material, archive: &Archive, log: &mut Vec<String>) -> (String, Option<(u32, u32)>) {
        let Some(bitmap) = m.bitmaps.first() else { return (BOUNDARY.into(), None) };
        let entry = self.by_bitmap.entry(bitmap.clone()).or_insert_with(|| {
            let stem = bitmap.rsplit_once('.').map_or(bitmap.as_str(), |(s, _)| s);
            let picture = archive.read(bitmap).and_then(|d| decode(&d, m.masked())).map_err(|e| log.push(format!("{bitmap}: {e}"))).ok();
            (format!("{folder}/{stem}"), picture)
        });
        (entry.0.clone(), entry.1.as_ref().map(|p| (p.width, p.height)))
    }

    pub fn infos(self, folder: &str) -> Vec<MaterialInfo> {
        let boundary = MaterialInfo {
            name: BOUNDARY.into(),
            folder: Some("everquest".into()),
            image: None,
            color: Some((150, 60, 200)),
            pixels: None,
            size: None,
            pbr: None,
            uv_scale: None,
            pixelated: false,
            blend: BlendRole::Unblendable,
        };
        let pictures = self.by_bitmap.into_values().filter_map(|(name, p)| p.map(|p| (name, p))).map(|(name, p)| MaterialInfo {
            name,
            folder: Some(folder.into()),
            image: None,
            color: Some(p.average()),
            pixels: Some(Image { width: p.width, height: p.height, rgba: p.rgba }),
            size: None,
            pbr: None,
            uv_scale: None,
            pixelated: false,
            blend: BlendRole::Layer,
        });
        std::iter::once(boundary).chain(pictures).collect()
    }
}

/// Where a surface goes.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Solid,
    Passable,
    Boundary,
}

pub fn import(name: &str, data: &[u8]) -> Result<Imported, String> {
    let zone = name.rsplit_once('.').map_or(name, |(s, _)| s).to_ascii_lowercase();
    let archive = Archive::open(data)?;
    let wld_name = format!("{zone}.wld");
    let wld_data =
        archive.read(&wld_name).map_err(|_| format!("{wld_name} is not in {name}; pick a zone's own .s3d, such as gfaydark.s3d, not its _obj or _chr file"))?;
    let wld = Wld::parse(&wld_data)?;
    let mut log = Vec::new();
    let mut materials = Materials::default();
    let (mut triangles, mut slivers) = (0usize, 0usize);
    let mut lists: BTreeMap<usize, Vec<Option<Material>>> = BTreeMap::new();
    let mut polygons: Vec<(Kind, Surface)> = Vec::new();
    for f in wld.of_kind(MESH) {
        let mesh = match wld.mesh(f) {
            Ok(m) => m,
            Err(e) => {
                log.push(format!("mesh {}: {e}", f.name));
                continue;
            }
        };
        let list = mesh.materials.map_or(&[][..], |i| lists.entry(i).or_insert_with(|| wld.material_list(Some(i))).as_slice());
        let tris = triangles_of(&mesh, list, &zone, &archive, &mut materials, &mut log, &mut slivers);
        triangles += tris.len();
        for p in merge(&tris) {
            let Some(seed) = tris.get(p.seed) else { continue };
            polygons.push((seed.kind, Surface { points: p.points, normal: p.normal, material: seed.material.clone(), uv: seed.uv }));
        }
    }
    let mut groups: Vec<(Kind, Vec<Brush>)> = Vec::new();
    let mut over = 0usize;
    for kind in [Kind::Solid, Kind::Passable, Kind::Boundary] {
        let mut brushes = Vec::new();
        for (_, s) in polygons.iter().filter(|p| p.0 == kind) {
            if groups.iter().map(|g| g.1.len()).sum::<usize>() + brushes.len() >= MAX_BRUSHES {
                over += 1;
            } else {
                brushes.push(slab(s));
            }
        }
        groups.push((kind, brushes));
    }
    let mut out = Vec::new();
    let [solid, passable, boundary] =
        [Kind::Solid, Kind::Passable, Kind::Boundary].map(|k| groups.iter_mut().find(|g| g.0 == k).map(|g| std::mem::take(&mut g.1)).unwrap_or_default());
    log.push(format!(
        "{name}: {triangles} triangles in {} brushes: {} solid, {} passable, {} invisible walls",
        polygons.len(),
        solid.len(),
        passable.len(),
        boundary.len()
    ));
    if slivers > 0 {
        log.push(format!("{name}: skipped {slivers} triangles too thin to be brushes"));
    }
    if over > 0 {
        log.push(format!("{name}: left out {over} brushes past the {MAX_BRUSHES} an import holds, invisible walls first"));
    }
    out.push(Group { name: "Zone".into(), hidden: false, brushes: solid, entities: Vec::new() });
    if !passable.is_empty() {
        let entity = Entity { classname: "func_illusionary".into(), properties: Vec::new(), connections: Vec::new(), brushes: passable };
        out.push(Group { name: "Passable".into(), hidden: false, brushes: Vec::new(), entities: vec![entity] });
    }
    out.push(Group { name: "Invisible walls".into(), hidden: true, brushes: boundary, entities: Vec::new() });
    let objects = objects(&archive, &zone);
    log.push(format!("{name}: {} objects, models from {zone}_obj.s3d", objects.len()));
    out.push(Group { name: "Objects".into(), hidden: false, brushes: Vec::new(), entities: objects });
    let lights = lights(&archive);
    log.push(format!("{name}: {} lights", lights.len()));
    out.push(Group { name: "Lights".into(), hidden: false, brushes: Vec::new(), entities: lights });
    out.retain(|g| !g.brushes.is_empty() || !g.entities.is_empty());
    Ok(Imported { output: Output { brushes: Vec::new(), entities: Vec::new(), groups: out, materials: materials.infos(&zone) }, log })
}

/// The mesh's triangles, counter-clockwise about their fronts.
fn triangles_of(
    mesh: &Mesh,
    list: &[Option<Material>],
    zone: &str,
    archive: &Archive,
    materials: &mut Materials,
    log: &mut Vec<String>,
    slivers: &mut usize,
) -> Vec<Tri<Kind>> {
    let mut out = Vec::with_capacity(mesh.triangles.len());
    for t in &mesh.triangles {
        // EverQuest winds triangles clockwise seen from the front.
        let corners = [t.corners[0], t.corners[2], t.corners[1]];
        let Some(p) = corners.iter().map(|&i| mesh.positions.get(i).copied().map(dv)).collect::<Option<Vec<_>>>() else { continue };
        let p = [p[0], p[1], p[2]];
        if height(p) < MIN_HEIGHT {
            *slivers += 1;
            continue;
        }
        let normal = (p[1] - p[0]).cross(p[2] - p[0]).normalize();
        let material = list.get(t.material).and_then(Option::as_ref);
        let invisible = material.is_none_or(Material::invisible);
        let (name, size) = match material {
            Some(m) if !invisible => materials.get(zone, m, archive, log),
            _ => (BOUNDARY.into(), None),
        };
        let size = size.map_or(DVec2::ZERO, |(w, h)| DVec2::new(w.into(), h.into()));
        let texels = corners.map(|i| mesh.uvs.get(i).map_or(DVec2::ZERO, |&[u, v]| DVec2::new(u.into(), v.into()) * size));
        let uv = (size != DVec2::ZERO).then(|| tex_axes(p, texels)).flatten();
        let kind = match () {
            _ if invisible => Kind::Boundary,
            _ if !t.solid => Kind::Passable,
            _ => Kind::Solid,
        };
        out.push(Tri { points: p, normal, material: name, kind, uv, texels, size });
    }
    out
}

/// The zone's placed objects as `prop_static`s naming their models in the zone's `_obj.s3d`.
fn objects(archive: &Archive, zone: &str) -> Vec<Entity> {
    let Ok(data) = archive.read("objects.wld") else { return Vec::new() };
    let Ok(wld) = Wld::parse(&data) else { return Vec::new() };
    wld.actors()
        .into_iter()
        .filter(|a| a.position.iter().all(|v| v.is_finite()))
        .map(|a| {
            let actor = a.name.to_ascii_lowercase();
            let actor = actor.strip_suffix("_actordef").unwrap_or(&actor);
            let [x, y, z] = a.position;
            let [heading, tilt, _] = a.rotation.map(|r| if r.is_finite() { f64::from(r) * TURN } else { 0.0 });
            let scale = if a.scale.is_finite() && a.scale > 0.0 { a.scale } else { 1.0 };
            let properties = vec![
                ("origin".into(), format!("{x} {y} {z}")),
                ("angles".into(), format!("{} {} 0", fmt(-tilt), fmt(heading))),
                ("scale".into(), scale.to_string()),
                ("model".into(), format!("{zone}_obj.s3d/{actor}.actordef")),
                ("collision".into(), "none".into()),
            ];
            Entity { classname: "prop_static".into(), properties, connections: Vec::new(), brushes: Vec::new() }
        })
        .collect()
}

fn fmt(v: f64) -> String {
    let r = (v * 1000.0).round() / 1000.0;
    if r == 0.0 { "0".into() } else { r.to_string() }
}

fn lights(archive: &Archive) -> Vec<Entity> {
    let Ok(data) = archive.read("lights.wld") else { return Vec::new() };
    let Ok(wld) = Wld::parse(&data) else { return Vec::new() };
    wld.lights()
        .into_iter()
        .filter(|l| l.position.iter().chain([&l.radius]).all(|v| v.is_finite()))
        .map(|l| {
            let [x, y, z] = l.position;
            let [r, g, b] = l.color.map(|c| (c * 255.0).round() as u8);
            let properties = vec![
                ("origin".into(), format!("{x} {y} {z}")),
                ("_light".into(), format!("{r} {g} {b} 255")),
                ("range".into(), l.radius.max(1.0).to_string()),
                ("_falloff".into(), "everquest".into()),
            ];
            Entity { classname: "light".into(), properties, connections: Vec::new(), brushes: Vec::new() }
        })
        .collect()
}
