//! Whole maps as glTF 2.0: visible faces go out as meshes, and mesh objects come in as brushes.

use std::collections::{BTreeMap, BTreeSet};

use gltf_json::validation::Checked;
use mapcraft_format::Fragment;
use mapcraft_geom::{DVec2, DVec3, EPSILON, Plane, convex_hull};
use mapcraft_map::{Brush, Face, Group, Map, NodeId, NodeKind, ROOT, TexAxis, TexDefaults, covered_faces, is_tool_material};
use serde_json::{Value, json};

use crate::Picture;
use crate::model::Doc;

/// Imports that would make more brushes than this are refused.
const MAX_BRUSHES: usize = 1 << 16;
/// Mesh objects with more triangles than this are not tried as one convex brush.
const MAX_CONVEX: usize = 2048;
/// How far the brush of each face of a mesh that is not convex reaches behind it, in map units.
const THICKNESS: f64 = 1.0;
/// How far a corner may sit off a plane and still count as on it, in map units.
const ON_PLANE: f64 = 0.01;
/// Texels across a material whose texture size is unknown, as the views assume.
const DEFAULT_SIZE: f64 = 64.0;

const ARRAY_BUFFER: u32 = 34962;
const ELEMENT_ARRAY_BUFFER: u32 = 34963;
const FLOAT: u32 = 5126;
const UNSIGNED_INT: u32 = 5125;

/// Map (x, y, z) as glTF (x, z, -y), the inverse of how [`crate::load_gltf`] reads glTF axes.
fn to_gltf(v: DVec3) -> [f32; 3] {
    [v.x as f32, v.z as f32, -v.y as f32]
}

fn texels(size: Option<[u32; 2]>) -> DVec2 {
    size.map_or(DVec2::splat(DEFAULT_SIZE), |[w, h]| DVec2::new(f64::from(w), f64::from(h)))
}

/// Triangles of one material, in map units.
#[derive(Default)]
struct Part {
    positions: Vec<DVec3>,
    normals: Vec<DVec3>,
    uvs: Vec<DVec2>,
    indices: Vec<u32>,
}

impl Part {
    /// Adds `points` textured as `face` and the triangles `tris` over them, each counter-clockwise from the front.
    fn add(&mut self, points: &[DVec3], normal: impl Fn(usize) -> DVec3, face: &Face, size: DVec2, tris: &[[usize; 3]]) -> Result<(), String> {
        let n = points.len();
        if u32::try_from(self.positions.len().saturating_add(n)).is_err() {
            return Err("one material has too many corners for a glTF mesh".into());
        }
        let base = self.positions.len() as u32;
        self.indices.extend(tris.iter().filter(|t| t.iter().all(|k| *k < n)).flatten().map(|k| base + *k as u32));
        self.positions.extend_from_slice(points);
        self.normals.extend((0..n).map(normal));
        self.uvs.extend(points.iter().map(|p| face.uv(*p, size)));
        Ok(())
    }
}

/// A glTF node: a brush with its faces by material, or a layer, group or entity holding brushes.
struct Object {
    name: String,
    parent: Option<usize>,
    parts: BTreeMap<String, Part>,
}

/// The visible brushes of `map` as a `.glb`: a node per brush with a primitive per material, under
/// nodes for the layers, groups and entities that hold them. Tool materials, faces hidden between
/// brushes and layers left out of exports stay out. `texture_size` gives the texels UVs are
/// measured in and `picture` the texture embedded for a material.
pub fn export_glb(
    map: &Map,
    texture_size: &dyn Fn(&str) -> Option<[u32; 2]>,
    picture: &dyn Fn(&str) -> Option<Picture>,
    units_per_meter: f64,
) -> Result<Vec<u8>, String> {
    let visible = map.visible_nodes();
    let covered = covered_faces(map, None);
    let mut objects: Vec<Object> = Vec::new();
    let mut holders: BTreeMap<NodeId, usize> = BTreeMap::new();
    for (id, node) in map.nodes() {
        let NodeKind::Brush(b) = &node.kind else { continue };
        let ancestors = map.ancestors(id);
        let omitted = |a: &NodeId| matches!(map.get(*a).map(|n| &n.kind), Some(NodeKind::Layer(l)) if l.omit_from_export);
        if !visible.contains(&id) || ancestors.iter().any(omitted) {
            continue;
        }
        let Ok(solid) = b.solid() else { continue };
        let displaced: BTreeMap<usize, Vec<DVec3>> = b.displaced(&solid).into_iter().collect();
        let mut parts: BTreeMap<String, Part> = BTreeMap::new();
        for (index, (face, winding)) in b.faces.iter().zip(&solid.faces).enumerate() {
            let Some(w) = winding else { continue };
            // A displaced brush shows only its displaced faces, as Source compiles it.
            if is_tool_material(&face.material) || covered.contains(&(id, face.id)) || (b.has_displacement() && !displaced.contains_key(&index)) {
                continue;
            }
            let size = texels(texture_size(&face.material));
            let normal = face.plane().map_or(DVec3::Z, |p| p.normal);
            let part = parts.entry(face.material.clone()).or_default();
            match (displaced.get(&index), &face.disp) {
                (Some(grid), Some(d)) => {
                    let normals = d.normals(grid, normal);
                    part.add(grid, |k| normals.get(k).copied().unwrap_or(normal), face, size, &d.triangles())?;
                }
                _ => part.add(&w.0, |_| normal, face, size, &fan(w.0.len()))?,
            }
        }
        if parts.is_empty() {
            continue;
        }
        let mut parent = None;
        for a in ancestors.iter().rev().filter(|a| **a != ROOT) {
            let next = match holders.get(a) {
                Some(i) => *i,
                None => {
                    let name = map.get(*a).map_or_else(String::new, |n| n.kind.label());
                    objects.push(Object { name, parent, parts: BTreeMap::new() });
                    holders.insert(*a, objects.len() - 1);
                    objects.len() - 1
                }
            };
            parent = Some(next);
        }
        objects.push(Object { name: format!("Brush {}", id.0), parent, parts });
    }
    if objects.is_empty() {
        return Err("nothing visible to export".into());
    }
    write_glb(&objects, picture, units_per_meter)
}

fn fan(corners: usize) -> Vec<[usize; 3]> {
    (1..corners.saturating_sub(1)).map(|k| [0, k, k + 1]).collect()
}

/// The binary chunk of a `.glb` and the buffer views and accessors into it.
#[derive(Default)]
struct Buffer {
    bin: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl Buffer {
    fn view(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        let mut view = json!({ "buffer": 0, "byteOffset": self.bin.len(), "byteLength": bytes.len() });
        if let Some(t) = target {
            view["target"] = json!(t);
        }
        self.bin.extend_from_slice(bytes);
        self.bin.resize(self.bin.len().div_ceil(4) * 4, 0);
        self.views.push(view);
        self.views.len() - 1
    }

    fn accessor(&mut self, bytes: &[u8], target: u32, component: u32, count: usize, kind: &str) -> usize {
        let view = self.view(bytes, Some(target));
        self.accessors.push(json!({ "bufferView": view, "componentType": component, "count": count, "type": kind }));
        self.accessors.len() - 1
    }

    fn floats<const N: usize>(&mut self, items: &[[f32; N]], kind: &str) -> usize {
        let bytes: Vec<u8> = items.iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
        self.accessor(&bytes, ARRAY_BUFFER, FLOAT, items.len(), kind)
    }
}

fn png(p: &Picture) -> Option<Vec<u8>> {
    let image = image::RgbaImage::from_raw(p.width, p.height, p.rgba.clone())?;
    let mut out = std::io::Cursor::new(Vec::new());
    image.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}

fn write_glb(objects: &[Object], picture: &dyn Fn(&str) -> Option<Picture>, units_per_meter: f64) -> Result<Vec<u8>, String> {
    let mut buf = Buffer::default();
    let (mut materials, mut images, mut textures) = (Vec::new(), Vec::new(), Vec::new());
    let mut material_of: BTreeMap<&str, usize> = BTreeMap::new();
    let names: BTreeSet<&str> = objects.iter().flat_map(|o| o.parts.keys().map(String::as_str)).collect();
    for name in names {
        let mut pbr = json!({ "metallicFactor": 0.0, "roughnessFactor": 1.0 });
        if let Some(bytes) = picture(name).and_then(|p| png(&p)) {
            let view = buf.view(&bytes, None);
            images.push(json!({ "name": name, "mimeType": "image/png", "bufferView": view }));
            textures.push(json!({ "source": images.len() - 1 }));
            pbr["baseColorTexture"] = json!({ "index": textures.len() - 1 });
        }
        materials.push(json!({ "name": name, "pbrMetallicRoughness": pbr }));
        material_of.insert(name, materials.len() - 1);
    }
    let (mut nodes, mut meshes, mut roots) = (Vec::new(), Vec::new(), Vec::new());
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); objects.len()];
    for (i, o) in objects.iter().enumerate() {
        match o.parent.and_then(|p| children.get_mut(p)) {
            Some(c) => c.push(i),
            None => roots.push(i),
        }
        let mut primitives = Vec::new();
        for (name, part) in &o.parts {
            let positions: Vec<[f32; 3]> = part.positions.iter().map(|p| to_gltf(*p / units_per_meter)).collect();
            let fold = |pick: fn(f32, f32) -> f32, from: f32| (0..3).map(|k| positions.iter().map(|p| p[k]).fold(from, pick)).collect::<Vec<f32>>();
            let (min, max) = (fold(f32::min, f32::INFINITY), fold(f32::max, f32::NEG_INFINITY));
            let position = buf.floats(&positions, "VEC3");
            if let Some(a) = buf.accessors.get_mut(position) {
                a["min"] = json!(min);
                a["max"] = json!(max);
            }
            let normal = buf.floats(&part.normals.iter().map(|n| to_gltf(*n)).collect::<Vec<_>>(), "VEC3");
            let uv = buf.floats(&part.uvs.iter().map(|v| v.as_vec2().to_array()).collect::<Vec<_>>(), "VEC2");
            let index_bytes: Vec<u8> = part.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
            let indices = buf.accessor(&index_bytes, ELEMENT_ARRAY_BUFFER, UNSIGNED_INT, part.indices.len(), "SCALAR");
            primitives.push(json!({
                "attributes": { "POSITION": position, "NORMAL": normal, "TEXCOORD_0": uv },
                "indices": indices,
                "material": material_of.get(name.as_str()),
            }));
        }
        let mut node = json!({ "name": o.name });
        if !primitives.is_empty() {
            meshes.push(json!({ "name": o.name, "primitives": primitives }));
            node["mesh"] = json!(meshes.len() - 1);
        }
        nodes.push(node);
    }
    for (node, c) in nodes.iter_mut().zip(children).filter(|(_, c)| !c.is_empty()) {
        node["children"] = json!(c);
    }
    let mut root = json!({
        "asset": { "version": "2.0", "generator": "MapCraft" },
        "scene": 0,
        "scenes": [{ "nodes": roots }],
        "nodes": nodes,
        "meshes": meshes,
        "materials": materials,
        "accessors": buf.accessors,
        "bufferViews": buf.views,
        "buffers": [{ "byteLength": buf.bin.len() }],
    });
    if !images.is_empty() {
        root["images"] = json!(images);
        root["textures"] = json!(textures);
    }
    glb(&root, buf.bin)
}

fn glb(root: &Value, mut bin: Vec<u8>) -> Result<Vec<u8>, String> {
    let mut text = serde_json::to_vec(root).map_err(|e| e.to_string())?;
    text.resize(text.len().div_ceil(4) * 4, b' ');
    bin.resize(bin.len().div_ceil(4) * 4, 0);
    let total = 12usize.saturating_add(8 + text.len()).saturating_add(8 + bin.len());
    let total = u32::try_from(total).map_err(|_| "the map is too big for a .glb file")?;
    let mut out = Vec::with_capacity(total as usize);
    out.extend(b"glTF");
    out.extend(2u32.to_le_bytes());
    out.extend(total.to_le_bytes());
    out.extend((text.len() as u32).to_le_bytes());
    out.extend(b"JSON");
    out.extend(text);
    out.extend((bin.len() as u32).to_le_bytes());
    out.extend(b"BIN\0");
    out.extend(bin);
    Ok(out)
}

/// A glTF file read as brushes.
pub struct ImportedGltf {
    /// A group named after the file: a brush per convex mesh object, and for each other object a
    /// group of one thin brush per face.
    pub group: Fragment,
    /// Pictures of the materials `texture_size` did not know, to keep as materials.
    pub pictures: Vec<(String, Picture)>,
    /// Mesh objects that were not convex.
    pub split: usize,
}

/// A material's name and texture size in texels.
struct Material {
    name: String,
    size: DVec2,
}

/// A triangle in map space, counter-clockwise seen from its front.
struct Tri {
    p: [DVec3; 3],
    uv: Option<[DVec2; 3]>,
    material: usize,
}

impl Tri {
    fn normal(&self) -> DVec3 {
        (self.p[1] - self.p[0]).cross(self.p[2] - self.p[0]).normalize_or_zero()
    }
}

/// Reads a `.glb` or `.gltf` file named `name` as brushes: each mesh object becomes one brush when
/// it is a closed convex solid, else one thin brush per face. Faces take the glTF material's name,
/// else its texture's, else `fallback`, with texture axes fitted to the UVs; `texture_size` gives
/// the texels of the materials it knows.
pub fn import_gltf(
    name: &str,
    bytes: &[u8],
    read: &dyn Fn(&str) -> Option<Vec<u8>>,
    units_per_meter: f64,
    texture_size: &mut dyn FnMut(&str) -> Option<[u32; 2]>,
    fallback: &str,
) -> Result<ImportedGltf, String> {
    let doc = Doc::parse(bytes, read)?;
    let mut materials: Vec<Material> = Vec::new();
    let mut slots: BTreeMap<Option<usize>, usize> = BTreeMap::new();
    let mut pictures = Vec::new();
    let (mut children, mut brushes, mut triangles, mut split) = (Vec::new(), 0usize, 0usize, 0usize);
    for (k, (node, mesh, at)) in doc.meshes(units_per_meter)?.into_iter().enumerate() {
        let flip = at.determinant() < 0.0;
        let mut tris = Vec::new();
        for prim in &mesh.primitives {
            let Some((pos, indices)) = doc.triangles(prim)? else { continue };
            let gltf_material = prim.material.map(|m| m.value());
            let material = match slots.get(&gltf_material) {
                Some(m) => *m,
                None => {
                    materials.push(material(&doc, gltf_material, read, texture_size, fallback, &mut pictures));
                    slots.insert(gltf_material, materials.len() - 1);
                    materials.len() - 1
                }
            };
            let base = gltf_material.and_then(|m| doc.root.materials.get(m)).and_then(|m| m.pbr_metallic_roughness.base_color_texture.as_ref());
            let set = gltf_json::mesh::Semantic::TexCoords(base.map_or(0, |t| t.tex_coord));
            let uvs = prim.attributes.get(&Checked::Valid(set)).and_then(|a| doc.read(*a, 2)).filter(|v| v.len() == 2 * pos.len());
            let uv_of = |i: u32| uvs.as_ref().and_then(|v| Some(DVec2::new(*v.get(2 * i as usize)?, *v.get(2 * i as usize + 1)?)));
            for t in indices {
                let [a, b, c] = if flip { [t[0], t[2], t[1]] } else { t };
                let (Some(pa), Some(pb), Some(pc)) = (pos.get(a as usize), pos.get(b as usize), pos.get(c as usize)) else { continue };
                let p = [*pa, *pb, *pc].map(|q| at.transform_point3(q));
                if !p.iter().all(|q| q.is_finite()) || (p[1] - p[0]).cross(p[2] - p[0]).length() < EPSILON {
                    continue;
                }
                let uv = match (uv_of(a), uv_of(b), uv_of(c)) {
                    (Some(a), Some(b), Some(c)) => Some([a, b, c]),
                    _ => None,
                };
                tris.push(Tri { p, uv, material });
                triangles += 1;
                if triangles > 2 * MAX_BRUSHES {
                    return Err(format!("more than {} triangles, too many to make brushes from", 2 * MAX_BRUSHES));
                }
            }
        }
        if tris.is_empty() {
            continue;
        }
        let made = match convex(&tris, &materials) {
            Some(b) => vec![b],
            None => {
                split += 1;
                plates(&tris, &materials)
            }
        };
        brushes += made.len();
        if brushes > MAX_BRUSHES {
            return Err(format!("more than {MAX_BRUSHES} brushes"));
        }
        let made: Vec<Fragment> = made.into_iter().map(|b| Fragment { kind: NodeKind::Brush(b), children: Vec::new() }).collect();
        if made.len() > 1 {
            let label = node.name.clone().or_else(|| mesh.name.clone()).unwrap_or_else(|| format!("Mesh {}", k + 1));
            children.push(group(&label, made));
        } else {
            children.extend(made);
        }
    }
    if children.is_empty() {
        return Err("no triangles to make brushes from".into());
    }
    Ok(ImportedGltf { group: group(name, children), pictures, split })
}

fn group(name: &str, children: Vec<Fragment>) -> Fragment {
    Fragment { kind: NodeKind::Group(Group { name: name.to_string(), ..Group::default() }), children }
}

/// The name and size of glTF material `index`; its picture goes in `pictures` when `texture_size` does not know the name.
fn material(
    doc: &Doc,
    index: Option<usize>,
    read: &dyn Fn(&str) -> Option<Vec<u8>>,
    texture_size: &mut dyn FnMut(&str) -> Option<[u32; 2]>,
    fallback: &str,
    pictures: &mut Vec<(String, Picture)>,
) -> Material {
    let m = index.and_then(|i| doc.root.materials.get(i));
    let image = m.and_then(|m| m.pbr_metallic_roughness.base_color_texture.as_ref()).and_then(|t| doc.root.get(t.index)).map(|t| t.source.value());
    let file =
        image.and_then(|i| doc.root.images.get(i)).and_then(|i| i.name.clone().or_else(|| i.uri.as_deref().filter(|u| !u.starts_with("data:")).map(stem)));
    let name = [m.and_then(|m| m.name.clone()), file].into_iter().flatten().map(|n| n.trim().to_string()).find(|n| !n.is_empty());
    let name = name.unwrap_or_else(|| fallback.to_string());
    if let Some(size) = texture_size(&name) {
        return Material { name, size: texels(Some(size)) };
    }
    let picture = image.and_then(|i| doc.picture(i, read));
    let size = texels(picture.as_ref().map(|p| [p.width, p.height]));
    pictures.extend(picture.map(|p| (name.clone(), p)));
    Material { name, size }
}

/// A file name in a URI without its folders and extension.
fn stem(uri: &str) -> String {
    let file = uri.rsplit('/').next().unwrap_or(uri);
    file.rsplit_once('.').map_or(file, |(s, _)| s).to_string()
}

fn defaults(t: &Tri, materials: &[Material]) -> TexDefaults {
    TexDefaults { material: materials.get(t.material).map_or_else(String::new, |m| m.name.clone()), scale: 1.0 }
}

/// A face through `points` in `t`'s material, its texture axes fitted to `t`'s UVs when it has them.
fn textured(points: [DVec3; 3], t: &Tri, materials: &[Material]) -> Face {
    let mut f = Face::new(points, &defaults(t, materials));
    if let (Some(uv), Some(m)) = (t.uv, materials.get(t.material)) {
        fit(&mut f, t.p, uv.map(|u| u * m.size));
    }
    f
}

/// Turns `f`'s texture axes so the corners `p` land on the texels `texel`; leaves them when the texels span no area.
fn fit(f: &mut Face, p: [DVec3; 3], texel: [DVec2; 3]) {
    let (e1, e2, d1, d2) = (p[1] - p[0], p[2] - p[0], texel[1] - texel[0], texel[2] - texel[0]);
    let n = e1.cross(e2);
    if d1.perp_dot(d2).abs() < 1e-9 {
        return;
    }
    // The gradient along the triangle that changes by `a` along e1 and `b` along e2.
    let axis = |a: f64, b: f64, at: f64| {
        let g = (e2.cross(n) * a + n.cross(e1) * b) / n.length_squared();
        let len = g.length();
        (len > 1e-9 && len.is_finite()).then(|| TexAxis { axis: g / len, offset: at - g.dot(p[0]), scale: 1.0 / len })
    };
    if let (Some(u), Some(v)) = (axis(d1.x, d2.x, texel[0].x), axis(d1.y, d2.y, texel[0].y)) {
        (f.u, f.v) = (u, v);
    }
}

/// The mesh as one brush when it is convex: every triangle lies on a side of its hull, facing out.
/// Sides no triangle covers, such as faces hidden between brushes, are closed over.
fn convex(tris: &[Tri], materials: &[Material]) -> Option<Brush> {
    if tris.len() > MAX_CONVEX {
        return None;
    }
    let points: Vec<DVec3> = tris.iter().flat_map(|t| t.p).collect();
    let sides: Vec<(Plane, [DVec3; 3])> = convex_hull(&points)?.into_iter().filter_map(|p| Some((Plane::from_points(p[0], p[1], p[2])?, p))).collect();
    let on = |plane: &Plane, t: &Tri| t.normal().dot(plane.normal) > 0.0 && t.p.iter().all(|q| plane.distance(*q).abs() <= ON_PLANE);
    if !tris.iter().all(|t| sides.iter().any(|(plane, _)| on(plane, t))) {
        return None;
    }
    let faces = sides.iter().map(|(plane, pts)| match tris.iter().find(|t| on(plane, t)) {
        Some(t) => textured(*pts, t, materials),
        None => {
            let nearest = tris.iter().max_by(|a, b| a.normal().dot(plane.normal).total_cmp(&b.normal().dot(plane.normal)));
            Face::new(*pts, &nearest.map_or_else(TexDefaults::default, |t| defaults(t, materials)))
        }
    });
    Brush::pruned(faces.collect()).ok()
}

/// One brush per face reaching [`THICKNESS`] behind it; two triangles in a row that make a flat
/// convex quad with one texture share a brush.
fn plates(tris: &[Tri], materials: &[Material]) -> Vec<Brush> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(t) = tris.get(i) {
        let merged = tris.get(i + 1).and_then(|next| quad(t, next, materials));
        i += if merged.is_some() { 2 } else { 1 };
        let n = t.normal();
        let corners = merged.map_or_else(|| t.p.to_vec(), |q| q.to_vec());
        let Some(hull) = convex_hull(&corners.iter().flat_map(|p| [*p, *p - n * THICKNESS]).collect::<Vec<_>>()) else { continue };
        let faces = hull
            .into_iter()
            .map(|pts| {
                let front = Plane::from_points(pts[0], pts[1], pts[2]).is_some_and(|p| p.normal.dot(n) > 1.0 - EPSILON);
                if front { textured(pts, t, materials) } else { Face::new(pts, &defaults(t, materials)) }
            })
            .collect();
        out.extend(Brush::pruned(faces).ok());
    }
    out
}

/// `a` and `b` as one quad, counter-clockwise, when they share an edge and make a flat convex
/// shape with one material and one texture mapping.
fn quad(a: &Tri, b: &Tri, materials: &[Material]) -> Option<[DVec3; 4]> {
    let shared = |p: DVec3, t: &Tri| t.p.iter().any(|q| q.distance_squared(p) < EPSILON * EPSILON);
    let k = (0..3).find(|&k| !shared(a.p[k], b))?;
    let x = (0..3).find(|&k| !shared(b.p[k], a))?;
    let n = a.normal();
    if a.material != b.material || (0..3).filter(|&j| shared(b.p[j], a)).count() != 2 || b.normal().dot(n) < 1.0 - EPSILON {
        return None;
    }
    let q = [a.p[k], a.p[(k + 1) % 3], b.p[x], a.p[(k + 2) % 3]];
    let flat = (b.p[x] - a.p[0]).dot(n).abs() <= ON_PLANE;
    let convex = (0..4).all(|i| (q[(i + 1) % 4] - q[i]).cross(q[(i + 2) % 4] - q[(i + 1) % 4]).dot(n) > EPSILON);
    let size = materials.get(a.material).map_or(DVec2::splat(DEFAULT_SIZE), |m| m.size);
    let face = textured(a.p, a, materials);
    let alike = match (a.uv, b.uv) {
        (None, None) => true,
        (Some(_), Some(uv)) => b.p.iter().zip(uv).all(|(p, u)| (face.uv(*p, size) - u).length() < 1e-4),
        _ => false,
    };
    (flat && convex && alike).then_some(q)
}

#[cfg(test)]
mod tests {
    use mapcraft_geom::Aabb;

    use super::*;
    use crate::model::tests::{QUAD, glb as glb_of, quad_bin};

    const UPM: f64 = 39.37;

    fn tex(material: &str) -> TexDefaults {
        TexDefaults { material: material.into(), scale: 1.0 }
    }

    fn map_of(brushes: &[Brush]) -> Map {
        let mut map = Map::new();
        let mut e = map.edit();
        for b in brushes {
            e.insert(ROOT, NodeKind::Brush(b.clone())).unwrap();
        }
        map.apply(e.finish());
        map
    }

    fn import(bytes: &[u8]) -> Result<ImportedGltf, String> {
        import_gltf("test.glb", bytes, &|_| None, UPM, &mut |_| Some([64, 64]), "fallback")
    }

    fn brushes(f: &Fragment) -> Vec<&Brush> {
        let mut out: Vec<&Brush> = f.kind.brush().into_iter().collect();
        out.extend(f.children.iter().flat_map(brushes));
        out
    }

    fn near(a: &Aabb, b: &Aabb) -> bool {
        a.min.abs_diff_eq(b.min, 1e-3) && a.max.abs_diff_eq(b.max, 1e-3)
    }

    /// One mesh object holding every face of `brushes`, as a modeling tool would save them.
    fn one_mesh(brushes: &[Brush], inside_out: bool) -> Vec<u8> {
        let mut part = Part::default();
        for b in brushes {
            let solid = b.solid().unwrap();
            for (f, w) in b.faces.iter().zip(&solid.faces) {
                let w = &w.as_ref().unwrap().0;
                part.add(w, |_| f.plane().unwrap().normal, f, DVec2::splat(64.0), &fan(w.len())).unwrap();
            }
        }
        if inside_out {
            part.indices.chunks_mut(3).for_each(|t| t.swap(1, 2));
        }
        let object = Object { name: "Blockout".into(), parent: None, parts: BTreeMap::from([("dev/a".to_string(), part)]) };
        write_glb(&[object], &|_| None, UPM).unwrap()
    }

    #[test]
    fn brushes_come_back_in_place_with_their_materials_and_uvs() {
        let a = Brush::cuboid(Aabb::new(DVec3::new(-64.0, -32.0, 0.0), DVec3::new(64.0, 32.0, 48.0)), &tex("dev/a"));
        let mut b = Brush::wedge(Aabb::new(DVec3::new(100.0, 0.0, 0.0), DVec3::new(228.0, 64.0, 64.0)), &tex("dev/b")).unwrap();
        b.faces[0].u.offset = 17.0;
        b.faces[1].v.scale = 0.5;
        b.faces[2].rotate_texture(30.0);
        let on_a = Brush::cuboid(Aabb::new(DVec3::new(-64.0, -32.0, 48.0), DVec3::new(64.0, 32.0, 96.0)), &tex("dev/c"));
        let trigger = Brush::cuboid(Aabb::new(DVec3::splat(300.0), DVec3::splat(364.0)), &tex("tools/toolstrigger"));
        let pixel = Picture { width: 1, height: 1, rgba: vec![255, 0, 0, 255] };
        let map = map_of(&[a.clone(), b.clone(), on_a.clone(), trigger]);
        let bytes = export_glb(&map, &|_| Some([64, 64]), &|m| (m == "dev/a").then(|| pixel.clone()), UPM).unwrap();

        let model = crate::load_gltf("out.glb", &bytes, &|_| None, UPM).unwrap();
        let all = a.bounds().unwrap().union(&b.bounds().unwrap()).union(&on_a.bounds().unwrap());
        assert!(model.min.abs_diff_eq(all.min, 1e-3) && model.max.abs_diff_eq(all.max, 1e-3), "{:?} {:?}", (model.min, model.max), all);
        assert_eq!(model.triangles(), 10 + 8 + 10, "the faces where the boxes touch and the trigger stay out");
        assert_eq!(model.textures.iter().filter(|t| t.rgba == pixel.rgba).count(), 1, "dev/a's picture is embedded");
        for part in &model.parts {
            for t in part.indices.chunks(3) {
                let [p, q, r] = [t[0], t[1], t[2]].map(|i| DVec3::from_array(part.positions[i as usize].map(f64::from)));
                let n = DVec3::from_array(part.normals[t[0] as usize].map(f64::from));
                assert!((q - p).cross(r - p).dot(n) > 0.0, "triangles face out, counter-clockwise");
            }
        }

        let imported = import(&bytes).unwrap();
        let got = brushes(&imported.group);
        assert_eq!((got.len(), imported.split), (3, 0), "each brush comes back whole, closed where it touched another");
        for want in [&a, &b, &on_a] {
            let bounds = want.bounds().unwrap();
            let back = got.iter().find(|g| near(&g.bounds().unwrap(), &bounds)).unwrap();
            assert_eq!(back.faces.len(), want.faces.len());
            let solid = want.solid().unwrap();
            for (face, w) in want.faces.iter().zip(&solid.faces) {
                let normal = face.plane().unwrap().normal;
                let same = back.faces.iter().find(|f| f.plane().unwrap().normal.abs_diff_eq(normal, 1e-4)).unwrap();
                assert_eq!(same.material, face.material);
                for p in &w.as_ref().unwrap().0 {
                    let (was, is) = (face.uv(*p, DVec2::splat(64.0)), same.uv(*p, DVec2::splat(64.0)));
                    assert!(was.abs_diff_eq(is, 1e-4), "{} at {p}: {was} became {is}", face.material);
                }
            }
        }
    }

    #[test]
    fn convex_objects_make_one_brush_and_others_a_brush_per_face() {
        let wedge = Brush::wedge(Aabb::new(DVec3::ZERO, DVec3::new(128.0, 64.0, 64.0)), &tex("dev/a")).unwrap();
        let imported = import(&one_mesh(std::slice::from_ref(&wedge), false)).unwrap();
        let got = brushes(&imported.group);
        assert_eq!((got.len(), imported.split), (1, 0));
        assert!(near(&got[0].bounds().unwrap(), &wedge.bounds().unwrap()));

        let room = Brush::cuboid(Aabb::new(DVec3::ZERO, DVec3::splat(64.0)), &tex("dev/a"));
        let imported = import(&one_mesh(std::slice::from_ref(&room), true)).unwrap();
        let got = brushes(&imported.group);
        assert_eq!((got.len(), imported.split), (6, 1), "a box facing in is a room, walled by a brush per side");
        let union = got.iter().map(|b| b.bounds().unwrap()).reduce(|a, b| a.union(&b)).unwrap();
        assert!(near(&union, &room.bounds().unwrap().expanded(THICKNESS)), "the walls reach outward");

        let tall = Brush::cuboid(Aabb::new(DVec3::ZERO, DVec3::new(64.0, 64.0, 128.0)), &tex("dev/a"));
        let foot = Brush::cuboid(Aabb::new(DVec3::new(64.0, 0.0, 0.0), DVec3::new(192.0, 64.0, 64.0)), &tex("dev/a"));
        let imported = import(&one_mesh(&[tall.clone(), foot.clone()], false)).unwrap();
        let got = brushes(&imported.group);
        assert_eq!((got.len(), imported.split), (12, 1), "each quad side of the L shape becomes one brush");
        let union = got.iter().map(|b| b.bounds().unwrap()).reduce(|a, b| a.union(&b)).unwrap();
        assert!(near(&union, &tall.bounds().unwrap().union(&foot.bounds().unwrap())), "the plates reach inward, so the outline stays");
        for b in got {
            let s = b.bounds().unwrap().size();
            assert!((s.min_element() - THICKNESS).abs() < 1e-6, "{s}");
            assert!(b.faces.iter().all(|f| f.material == "dev/a"));
        }
        assert!(matches!(&imported.group.children[0].kind, NodeKind::Group(g) if g.name == "Blockout"), "the L shape's brushes are grouped");
    }

    #[test]
    fn malformed_files_fail_without_panicking() {
        let good = one_mesh(&[Brush::cuboid(Aabb::new(DVec3::ZERO, DVec3::splat(64.0)), &tex("dev/a"))], false);
        assert_eq!(brushes(&import(&good).unwrap().group).len(), 1);
        for len in 0..good.len() {
            let _ = import(&good[..len]);
        }
        let quad = import(&glb_of(QUAD, &quad_bin())).unwrap();
        assert_eq!((brushes(&quad.group).len(), quad.split), (1, 1), "a lone quad becomes one thin brush");
        for (from, to) in [
            ("\"count\":4,", "\"count\":4000000000,"),
            ("\"count\":6,", "\"count\":600,"),
            ("\"byteLength\":48}", "\"byteLength\":48,\"byteStride\":4}"),
            ("\"byteOffset\":48", "\"byteOffset\":18446744073709551615"),
            ("\"bufferView\":1,", "\"bufferView\":7,"),
            ("\"material\":0", "\"material\":99"),
        ] {
            let _ = import(&glb_of(&QUAD.replace(from, to), &quad_bin()));
        }
        let infinite = QUAD.replace("\"translation\":[0,0,-2]", "\"scale\":[1e39,1,1]");
        assert!(import(&glb_of(&infinite, &quad_bin())).is_err(), "triangles with corners that are not finite are dropped");
        let mut bin = quad_bin();
        bin.copy_within(0..12, 12);
        bin.copy_within(0..12, 24);
        assert!(import(&glb_of(QUAD, &bin)).is_err(), "triangles without area are dropped");
        let indices = QUAD.replace("\"count\":6,", "\"count\":3,");
        let mut bin = quad_bin();
        bin[48..54].copy_from_slice(&[0, 0, 9, 0, 1, 0]);
        assert!(import(&glb_of(&indices, &bin)).is_err(), "an index past the corners is dropped");
        let cyclic = r#"{"asset":{"version":"2.0"},"scenes":[{"nodes":[0]}],"nodes":[{"children":[1]},{"children":[0]}]}"#;
        assert!(import(cyclic.as_bytes()).is_err());
        assert!(import(b"{nope").is_err());
    }
}
