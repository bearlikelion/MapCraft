//! Static glTF 2.0 models that point entities show in place of their boxes.

use std::borrow::Cow;

use gltf_json as json;
use json::accessor::ComponentType;
use json::validation::Checked;
use mapcraft_geom::{DMat4, DQuat, DVec3};
use mapcraft_map::Entity;

/// Model and buffer files larger than this are refused.
pub const MAX_MODEL_BYTES: u64 = 512 << 20;
const MAX_TRIANGLES: usize = 1 << 20;
const MAX_NODES: usize = 100_000;
const MAX_DEPTH: usize = 64;
/// Textures are shrunk to fit this many pixels on a side.
const TEXTURE_SIDE: u32 = 1024;

/// RGBA8 pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Triangles sharing one texture and color.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelPart {
    /// Index into `Model::textures`.
    pub texture: usize,
    /// Linear base color the texture is multiplied by.
    pub color: [f32; 4],
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// Three per triangle, each less than `positions.len()`.
    pub indices: Vec<u32>,
    /// Triangle edges, each once, for wireframes.
    pub edges: Vec<[u32; 2]>,
}

/// A static mesh in map units and axes, relative to the entity's origin.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Model {
    /// The path it was loaded from, which names its textures for the renderer.
    pub name: String,
    pub parts: Vec<ModelPart>,
    pub textures: Vec<Picture>,
    pub min: DVec3,
    pub max: DVec3,
}

impl Model {
    pub fn texture_name(&self, texture: usize) -> String {
        format!("{}#{texture}", self.name)
    }

    pub fn triangles(&self) -> usize {
        self.parts.iter().map(|p| p.indices.len() / 3).sum()
    }
}

fn numbers(s: &str) -> Vec<f64> {
    s.split_whitespace().map(|t| t.parse::<f64>().ok().filter(|v| v.is_finite())).collect::<Option<Vec<_>>>().unwrap_or_default()
}

/// Where a point entity's model sits: its `origin`, Hammer `angles` (pitch yaw roll) or `angle` (yaw),
/// and a uniform `scale` or `modelscale`.
pub fn entity_transform(e: &Entity) -> DMat4 {
    let num = |k: &str| numbers(e.get(k).unwrap_or_default());
    let [pitch, yaw, roll] = match num("angles").as_slice() {
        &[p, y, r] => [p, y, r],
        _ => [0.0, num("angle").first().copied().unwrap_or(0.0), 0.0],
    };
    let scale = ["scale", "modelscale"].iter().find_map(|k| num(k).first().copied()).filter(|s| *s > 0.0).unwrap_or(1.0);
    let turn = DQuat::from_rotation_z(yaw.to_radians()) * DQuat::from_rotation_y(pitch.to_radians()) * DQuat::from_rotation_x(roll.to_radians());
    DMat4::from_scale_rotation_translation(DVec3::splat(scale), turn, e.origin().unwrap_or_default())
}

/// The world box of `model` placed by `at`.
pub fn placed_bounds(model: &Model, at: &DMat4) -> (DVec3, DVec3) {
    let (a, b) = (model.min, model.max);
    let corners = (0..8).map(|i| at.transform_point3(DVec3::new([a.x, b.x][i & 1], [a.y, b.y][(i >> 1) & 1], [a.z, b.z][(i >> 2) & 1])));
    corners.fold((DVec3::INFINITY, DVec3::NEG_INFINITY), |(lo, hi), p| (lo.min(p), hi.max(p)))
}

/// The JSON and binary chunks of a `.glb`.
fn split_glb(bytes: &[u8]) -> Result<(&[u8], Option<&[u8]>), String> {
    let word = |at: usize| bytes.get(at..at.checked_add(4)?).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes);
    if word(4) != Some(2) {
        return Err("not a glTF 2.0 binary".into());
    }
    let (mut at, mut json, mut bin) = (12usize, None, None);
    while let (Some(len), Some(kind)) = (word(at), word(at + 4)) {
        let start = at + 8;
        let end = start.checked_add(len as usize).filter(|e| *e <= bytes.len()).ok_or("a chunk runs past the end of the file")?;
        let chunk = bytes.get(start..end).ok_or("bad chunk")?;
        match kind {
            0x4E4F_534A => json = json.or(Some(chunk)),
            0x004E_4942 => bin = bin.or(Some(chunk)),
            _ => {}
        }
        at = end;
    }
    Ok((json.ok_or("no JSON chunk")?, bin))
}

fn data_uri(uri: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    let (head, data) = uri.strip_prefix("data:")?.split_once(',')?;
    head.ends_with(";base64").then(|| base64::engine::general_purpose::STANDARD.decode(data).ok())?
}

fn percent_decoded(uri: &str) -> String {
    let mut out = Vec::new();
    let mut bytes = uri.bytes();
    while let Some(b) = bytes.next() {
        let hex = (b == b'%').then(|| bytes.clone().take(2).collect::<Vec<u8>>()).and_then(|h| u8::from_str_radix(std::str::from_utf8(&h).ok()?, 16).ok());
        match hex {
            Some(v) => {
                out.push(v);
                bytes.nth(1);
            }
            None => out.push(b),
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub(crate) struct Doc<'a> {
    pub root: json::Root,
    buffers: Vec<Option<Cow<'a, [u8]>>>,
}

impl<'a> Doc<'a> {
    /// A `.glb` or `.gltf` file with its buffers; `read` fetches the files its URIs name.
    pub fn parse(bytes: &'a [u8], read: &dyn Fn(&str) -> Option<Vec<u8>>) -> Result<Self, String> {
        let (text, bin) = if bytes.starts_with(b"glTF") { split_glb(bytes)? } else { (bytes, None) };
        let root = json::Root::from_slice(text).map_err(|e| format!("not glTF: {e}"))?;
        let buffers = root
            .buffers
            .iter()
            .enumerate()
            .map(|(i, b)| match &b.uri {
                None if i == 0 => bin.map(Cow::Borrowed),
                None => None,
                Some(uri) => data_uri(uri).or_else(|| read(&percent_decoded(uri))).map(Cow::Owned),
            })
            .collect();
        Ok(Self { root, buffers })
    }

    /// Each node of the scene that has a mesh, and where it sits in map space: glTF (x, y, z) at
    /// map (x, -z, y) times `units_per_meter`.
    pub fn meshes(&self, units_per_meter: f64) -> Result<Vec<(&json::Node, &json::Mesh, DMat4)>, String> {
        let scene = self.root.scene.and_then(|s| self.root.get(s)).or(self.root.scenes.first()).ok_or("no scene")?;
        let to_map = DMat4::from_cols_array(&[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
        let to_map = DMat4::from_scale(DVec3::splat(units_per_meter)) * to_map;
        let mut out = Vec::new();
        let mut left = MAX_NODES;
        let mut stack: Vec<(json::Index<json::Node>, DMat4, usize)> = scene.nodes.iter().take(left).map(|n| (*n, to_map, 0)).collect();
        while let Some((index, parent, depth)) = stack.pop() {
            let Some(node) = self.root.get(index).filter(|_| depth < MAX_DEPTH) else { continue };
            let at = parent * node_matrix(node);
            let children = node.children.as_deref().unwrap_or_default();
            let children = children.get(..left.min(children.len())).unwrap_or_default();
            left -= children.len();
            stack.extend(children.iter().map(|c| (*c, at, depth + 1)));
            let Some(mesh) = node.mesh.and_then(|m| self.root.get(m)) else { continue };
            // A skinned mesh ignores its node's transform; without its joints it shows in its bind pose.
            out.push((node, mesh, if node.skin.is_some() { to_map } else { at }));
        }
        Ok(out)
    }

    /// A triangle primitive's positions and its triangles, leaving out those with an index out of range.
    pub fn triangles(&self, prim: &json::mesh::Primitive) -> Result<Option<(Vec<DVec3>, Vec<[u32; 3]>)>, String> {
        let positions = prim.attributes.get(&Checked::Valid(json::mesh::Semantic::Positions)).and_then(|a| self.read(*a, 3));
        let (Checked::Valid(json::mesh::Mode::Triangles), Some(pos)) = (prim.mode, positions) else { return Ok(None) };
        let pos: Vec<DVec3> = pos.as_chunks::<3>().0.iter().map(|p| DVec3::from_array(*p)).collect();
        let n = pos.len();
        let mut indices: Vec<u32> = match prim.indices {
            Some(a) => self.read(a, 1).ok_or("unreadable indices")?.into_iter().map(|i| i as u32).collect(),
            None => (0..n as u32).collect(),
        };
        indices.truncate(indices.len() / 3 * 3);
        let tris = indices.as_chunks::<3>().0.iter().filter(|t| t.iter().all(|i| (*i as usize) < n)).copied().collect();
        Ok(Some((pos, tris)))
    }

    fn view(&self, index: json::Index<json::buffer::View>) -> Option<&[u8]> {
        let view = self.root.get(index)?;
        let data = self.buffers.get(view.buffer.value())?.as_deref()?;
        let start = usize::try_from(view.byte_offset.map_or(0, |o| o.0)).ok()?;
        data.get(start..start.checked_add(usize::try_from(view.byte_length.0).ok()?)?)
    }

    /// An accessor's first `width` components of each element; normalized integers become -1 to 1 or 0 to 1.
    pub fn read(&self, index: json::Index<json::Accessor>, width: usize) -> Option<Vec<f64>> {
        let acc = self.root.get(index)?;
        let (Checked::Valid(ty), Checked::Valid(json::accessor::GenericComponentType(ct)), None) = (acc.type_, acc.component_type, &acc.sparse) else {
            return None;
        };
        let (n, size) = (ty.multiplicity(), ct.size());
        let count = usize::try_from(acc.count.0).ok().filter(|c| *c <= 3 * MAX_TRIANGLES)?;
        if n < width || count == 0 {
            return (count == 0).then(Vec::new);
        }
        let view = self.view(acc.buffer_view?)?;
        let stride = self.root.get(acc.buffer_view?)?.byte_stride.map_or(n * size, |s| s.0);
        let start = usize::try_from(acc.byte_offset.map_or(0, |o| o.0)).ok()?;
        let end = stride.checked_mul(count - 1)?.checked_add(n * size)?.checked_add(start)?;
        let data = view.get(start..end).filter(|_| stride >= n * size)?;
        let mut out = Vec::with_capacity(count * width);
        for i in 0..count {
            for c in 0..width {
                let at = i * stride + c * size;
                out.push(component(ct, data.get(at..at + size)?, acc.normalized)?);
            }
        }
        Some(out)
    }

    pub fn picture(&self, image: usize, read: &dyn Fn(&str) -> Option<Vec<u8>>) -> Option<Picture> {
        let img = self.root.images.get(image)?;
        let bytes: Cow<[u8]> = match (img.buffer_view, &img.uri) {
            (Some(v), _) => Cow::Borrowed(self.view(v)?),
            (None, Some(uri)) => Cow::Owned(data_uri(uri).or_else(|| read(&percent_decoded(uri)))?),
            (None, None) => return None,
        };
        let decoded = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?.decode().ok()?;
        let fitted = if decoded.width().max(decoded.height()) > TEXTURE_SIDE { decoded.thumbnail(TEXTURE_SIDE, TEXTURE_SIDE) } else { decoded };
        let rgba = fitted.to_rgba8();
        Some(Picture { width: rgba.width(), height: rgba.height(), rgba: rgba.into_raw() })
    }
}

fn component(ct: ComponentType, b: &[u8], normalized: bool) -> Option<f64> {
    let (v, max) = match ct {
        ComponentType::I8 => (f64::from(i8::from_le_bytes(b.try_into().ok()?)), 127.0),
        ComponentType::U8 => (f64::from(u8::from_le_bytes(b.try_into().ok()?)), 255.0),
        ComponentType::I16 => (f64::from(i16::from_le_bytes(b.try_into().ok()?)), 32767.0),
        ComponentType::U16 => (f64::from(u16::from_le_bytes(b.try_into().ok()?)), 65535.0),
        ComponentType::U32 => (f64::from(u32::from_le_bytes(b.try_into().ok()?)), f64::from(u32::MAX)),
        ComponentType::F32 => return Some(f64::from(f32::from_le_bytes(b.try_into().ok()?))).filter(|v| v.is_finite()).or(Some(0.0)),
    };
    Some(if normalized { (v / max).max(-1.0) } else { v })
}

fn node_matrix(node: &json::Node) -> DMat4 {
    if let Some(m) = node.matrix {
        return DMat4::from_cols_array(&m.map(f64::from));
    }
    let r = node.rotation.map_or(DQuat::IDENTITY, |q| DQuat::from_array(q.0.map(f64::from)));
    let r = if r.length_squared() > 0.0 { r.normalize() } else { DQuat::IDENTITY };
    let v = |a: Option<[f32; 3]>, d: f64| a.map_or(DVec3::splat(d), |a| DVec3::from_array(a.map(f64::from)));
    DMat4::from_scale_rotation_translation(v(node.scale, 1.0), r, v(node.translation, 0.0))
}

/// Loads a static glTF 2.0 model (`.glb` or `.gltf`) named `name`; `read` fetches the files its URIs name.
/// glTF is Y up in meters; the model comes out Z up in map units, with glTF (x, y, z) at map
/// (x, -z, y), the inverse of how the Godot importer turns map axes into Godot's.
pub fn load_gltf(name: &str, bytes: &[u8], read: &dyn Fn(&str) -> Option<Vec<u8>>, units_per_meter: f64) -> Result<Model, String> {
    let doc = Doc::parse(bytes, read)?;
    let mut model = Model { name: name.to_string(), ..Model::default() };
    let mut pictures: Vec<(Option<usize>, usize)> = Vec::new();
    let mut texture_of = |image: Option<usize>, model: &mut Model| {
        if let Some((_, t)) = pictures.iter().find(|(i, _)| *i == image) {
            return *t;
        }
        let pic = image.and_then(|i| doc.picture(i, read)).unwrap_or(Picture { width: 1, height: 1, rgba: vec![255; 4] });
        model.textures.push(pic);
        pictures.push((image, model.textures.len() - 1));
        model.textures.len() - 1
    };
    let mut triangles = 0usize;
    for (_, mesh, at) in doc.meshes(units_per_meter)? {
        let turn = at.inverse().transpose();
        for prim in &mesh.primitives {
            let attr = |s: json::mesh::Semantic| prim.attributes.get(&Checked::Valid(s)).copied();
            let Some((pos, tris)) = doc.triangles(prim)? else { continue };
            let n = pos.len();
            let material = prim.material.and_then(|m| doc.root.get(m));
            let pbr = material.map(|m| &m.pbr_metallic_roughness);
            let base = pbr.and_then(|p| p.base_color_texture.as_ref());
            let image = base.and_then(|t| doc.root.get(t.index)).map(|t| t.source.value());
            let uvs = attr(json::mesh::Semantic::TexCoords(base.map_or(0, |t| t.tex_coord))).and_then(|a| doc.read(a, 2));
            let normals = attr(json::mesh::Semantic::Normals).and_then(|a| doc.read(a, 3)).filter(|v| v.len() == 3 * n);
            triangles += tris.len();
            if triangles > MAX_TRIANGLES {
                return Err(format!("more than {MAX_TRIANGLES} triangles"));
            }
            let positions: Vec<DVec3> = pos.iter().map(|p| at.transform_point3(*p)).collect();
            let normals: Vec<[f32; 3]> = match normals {
                Some(v) => {
                    v.as_chunks::<3>().0.iter().map(|n| turn.transform_vector3(DVec3::from_array(*n)).normalize_or(DVec3::Z).as_vec3().to_array()).collect()
                }
                None => flat_normals(&positions, &tris),
            };
            let mut edges: Vec<[u32; 2]> = tris.iter().flat_map(|&[a, b, c]| [[a, b], [b, c], [c, a]]).map(|[a, b]| [a.min(b), a.max(b)]).collect();
            edges.sort_unstable();
            edges.dedup();
            let texture = texture_of(image, &mut model);
            model.parts.push(ModelPart {
                texture,
                color: pbr.map_or([1.0; 4], |p| p.base_color_factor.0),
                positions: positions.iter().map(|p| p.as_vec3().to_array()).collect(),
                normals,
                uvs: uvs.filter(|v| v.len() == 2 * n).map_or_else(|| vec![[0.0; 2]; n], |v| v.as_chunks::<2>().0.iter().map(|c| c.map(|x| x as f32)).collect()),
                indices: tris.into_iter().flatten().collect(),
                edges,
            });
        }
    }
    let used = model.parts.iter().flat_map(|p| p.indices.iter().filter_map(|i| p.positions.get(*i as usize)));
    let (min, max) = used.fold((DVec3::INFINITY, DVec3::NEG_INFINITY), |(lo, hi), p| {
        let p = DVec3::from_array(p.map(f64::from));
        (lo.min(p), hi.max(p))
    });
    if model.parts.is_empty() || !min.is_finite() || !max.is_finite() {
        return Err("no triangles".into());
    }
    (model.min, model.max) = (min, max);
    Ok(model)
}

/// Triangles for [`mesh_model`]: an index into its textures (none is plain white), positions in
/// map units, one UV per position and three indices per triangle.
pub struct MeshPart {
    pub texture: Option<usize>,
    pub positions: Vec<DVec3>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

/// A model from triangles a plugin read. Corners in the same place share their normal where the
/// faces meet at less than 60 degrees, so curved meshes look smooth and boxes keep their edges.
pub fn mesh_model(name: &str, parts: Vec<MeshPart>, mut textures: Vec<Picture>) -> Result<Model, String> {
    let mut model = Model { name: name.to_string(), ..Model::default() };
    let mut white = None;
    let mut triangles = 0usize;
    for p in parts {
        let n = p.positions.len();
        let tris: Vec<[u32; 3]> = p.indices.as_chunks::<3>().0.iter().filter(|t| t.iter().all(|i| (*i as usize) < n)).copied().collect();
        triangles += tris.len();
        if triangles > MAX_TRIANGLES {
            return Err(format!("more than {MAX_TRIANGLES} triangles"));
        }
        let texture = match p.texture.filter(|t| *t < textures.len()) {
            Some(t) => t,
            None => *white.get_or_insert_with(|| {
                textures.push(Picture { width: 1, height: 1, rgba: vec![255; 4] });
                textures.len() - 1
            }),
        };
        let mut edges: Vec<[u32; 2]> = tris.iter().flat_map(|&[a, b, c]| [[a, b], [b, c], [c, a]]).map(|[a, b]| [a.min(b), a.max(b)]).collect();
        edges.sort_unstable();
        edges.dedup();
        model.parts.push(ModelPart {
            texture,
            color: [1.0; 4],
            normals: creased_normals(&p.positions, &tris),
            positions: p.positions.iter().map(|v| v.as_vec3().to_array()).collect(),
            uvs: if p.uvs.len() == n { p.uvs } else { vec![[0.0; 2]; n] },
            indices: tris.into_iter().flatten().collect(),
            edges,
        });
    }
    model.textures = textures;
    let used = model.parts.iter().flat_map(|p| p.indices.iter().filter_map(|i| p.positions.get(*i as usize)));
    let (min, max) = used.fold((DVec3::INFINITY, DVec3::NEG_INFINITY), |(lo, hi), p| {
        let p = DVec3::from_array(p.map(f64::from));
        (lo.min(p), hi.max(p))
    });
    if !min.is_finite() || !max.is_finite() {
        return Err("no triangles".into());
    }
    (model.min, model.max) = (min, max);
    Ok(model)
}

/// Normals for corners that may each belong to one triangle: corners in the same place average
/// the faces that meet theirs at less than 60 degrees.
fn creased_normals(positions: &[DVec3], tris: &[[u32; 3]]) -> Vec<[f32; 3]> {
    let key = |p: DVec3| (p * 64.0).round().as_i64vec3().to_array();
    let face = |t: &[u32; 3]| {
        let [a, b, c] = t.map(|k| positions.get(k as usize).copied().unwrap_or_default());
        (b - a).cross(c - a)
    };
    let mut at: std::collections::HashMap<[i64; 3], Vec<DVec3>> = std::collections::HashMap::new();
    for t in tris {
        let n = face(t);
        for k in t {
            at.entry(key(positions.get(*k as usize).copied().unwrap_or_default())).or_default().push(n);
        }
    }
    let mut out = vec![[0.0, 0.0, 1.0]; positions.len()];
    for t in tris {
        let own = face(t).normalize_or_zero();
        for k in t {
            let Some(p) = positions.get(*k as usize) else { continue };
            let sum: DVec3 = at.get(&key(*p)).into_iter().flatten().filter(|n| n.normalize_or_zero().dot(own) > 0.5).sum();
            if let Some(slot) = out.get_mut(*k as usize) {
                *slot = sum.normalize_or(own).as_vec3().to_array();
            }
        }
    }
    out
}

/// One normal per vertex from the triangles that use it, for meshes saved without normals.
fn flat_normals(positions: &[DVec3], tris: &[[u32; 3]]) -> Vec<[f32; 3]> {
    let mut sum = vec![DVec3::ZERO; positions.len()];
    for t in tris {
        let [a, b, c] = t.map(|k| positions.get(k as usize).copied().unwrap_or_default());
        let n = (b - a).cross(c - a);
        for k in t {
            if let Some(s) = sum.get_mut(*k as usize) {
                *s += n;
            }
        }
    }
    sum.into_iter().map(|n| n.normalize_or(DVec3::Z).as_vec3().to_array()).collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// One quad, two triangles in the glTF XY plane facing +Z, 1 meter wide and 2 meters toward -Z.
    pub(crate) const QUAD: &str = r#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
        "nodes":[{"mesh":0,"translation":[0,0,-2]}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1,"material":0}]}],
        "materials":[{"pbrMetallicRoughness":{"baseColorFactor":[1,0,0,1]}}],
        "accessors":[{"bufferView":0,"componentType":5126,"count":4,"type":"VEC3"},
            {"bufferView":1,"componentType":5123,"count":6,"type":"SCALAR"}],
        "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":48},{"buffer":0,"byteOffset":48,"byteLength":12}],
        "buffers":[{"byteLength":60}]}"#;

    pub(crate) fn quad_bin() -> Vec<u8> {
        let mut bin: Vec<u8> = [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]].iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
        bin.extend([0u16, 1, 2, 0, 2, 3].iter().flat_map(|i| i.to_le_bytes()));
        bin
    }

    pub(crate) fn glb(json: &str, bin: &[u8]) -> Vec<u8> {
        let mut json = json.as_bytes().to_vec();
        json.resize(json.len().div_ceil(4) * 4, b' ');
        let mut out = b"glTF".to_vec();
        out.extend(2u32.to_le_bytes());
        out.extend(((12 + 8 + json.len() + 8 + bin.len()) as u32).to_le_bytes());
        out.extend((json.len() as u32).to_le_bytes());
        out.extend(b"JSON");
        out.extend(json);
        out.extend((bin.len() as u32).to_le_bytes());
        out.extend(b"BIN\0");
        out.extend(bin);
        out
    }

    pub(crate) fn quad_glb() -> Vec<u8> {
        glb(QUAD, &quad_bin())
    }

    #[test]
    fn loads_a_glb_in_map_axes_and_units() {
        let m = load_gltf("props/quad.glb", &quad_glb(), &|_| None, 10.0).unwrap();
        assert_eq!(m.triangles(), 2);
        assert_eq!(m.parts[0].color, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(m.textures.len(), 1, "an untextured part gets a white picture");
        assert_eq!(m.texture_name(0), "props/quad.glb#0");
        // glTF z -2 is 2 meters toward map +Y, glTF y up is map z.
        assert!(m.min.abs_diff_eq(DVec3::new(0.0, 20.0, 0.0), 1e-5) && m.max.abs_diff_eq(DVec3::new(10.0, 20.0, 10.0), 1e-5), "{:?}", (m.min, m.max));
        let n = DVec3::from_array(m.parts[0].normals[0].map(f64::from));
        assert!(n.abs_diff_eq(DVec3::NEG_Y, 1e-5), "the glTF +Z facing quad faces map -Y, got {n}");
        assert_eq!(m.parts[0].edges.len(), 5, "the shared diagonal is drawn once");
    }

    #[test]
    fn malformed_files_fail_without_panicking() {
        let good = quad_glb();
        for len in 0..good.len() {
            let _ = load_gltf("cut.glb", &good[..len], &|_| None, 39.37);
        }
        let mut huge = good.clone();
        huge[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(load_gltf("huge.glb", &huge, &|_| None, 39.37).is_err());
        for (from, to) in [
            ("\"count\":4,", "\"count\":4000000000,"),
            ("\"count\":4,", "\"count\":0,"),
            ("\"count\":6,", "\"count\":600,"),
            ("\"byteLength\":48}", "\"byteLength\":48,\"byteStride\":4}"),
            ("\"byteOffset\":48", "\"byteOffset\":18446744073709551615"),
            ("\"bufferView\":1,", "\"bufferView\":7,"),
            ("\"type\":\"VEC3\"", "\"type\":\"MAT9\""),
        ] {
            let hostile = glb(&QUAD.replace(from, to), &quad_bin());
            assert!(load_gltf("hostile.glb", &hostile, &|_| None, 39.37).is_err(), "{to}");
        }
        let cyclic = r#"{"asset":{"version":"2.0"},"scenes":[{"nodes":[0]}],"nodes":[{"children":[1]},{"children":[0]}]}"#;
        assert!(load_gltf("cyclic.gltf", cyclic.as_bytes(), &|_| None, 39.37).is_err());
        let wide = format!(r#"{{"asset":{{"version":"2.0"}},"scenes":[{{"nodes":[0]}}],"nodes":[{{"children":[{}]}}]}}"#, vec!["0"; 2000].join(","));
        assert!(load_gltf("wide.gltf", wide.as_bytes(), &|_| None, 39.37).is_err(), "a node listing itself many times stays within the node cap");
        assert!(load_gltf("junk.gltf", b"{nope", &|_| None, 39.37).is_err());
    }

    #[test]
    fn places_by_origin_angles_and_scale() {
        let mut e = Entity::new("prop_static");
        e.set("origin", "100 0 0");
        e.set("angles", "0 90 0");
        e.set("modelscale", "2");
        let p = entity_transform(&e).transform_point3(DVec3::X);
        assert!(p.abs_diff_eq(DVec3::new(100.0, 2.0, 0.0), 1e-9), "yaw 90 turns +X to +Y, got {p}");
        e.set("angles", "90 0 0");
        let p = entity_transform(&e).transform_point3(DVec3::X);
        assert!(p.abs_diff_eq(DVec3::new(100.0, 0.0, -2.0), 1e-9), "positive pitch looks down, got {p}");
        let m = Model { min: DVec3::splat(-1.0), max: DVec3::splat(1.0), ..Model::default() };
        let (lo, hi) = placed_bounds(&m, &entity_transform(&e));
        assert!(lo.abs_diff_eq(DVec3::new(98.0, -2.0, -2.0), 1e-9) && hi.abs_diff_eq(DVec3::new(102.0, 2.0, 2.0), 1e-9));
    }

    #[test]
    fn reads_embedded_gltf_buffers_and_uris() {
        use base64::Engine;
        let bin = quad_bin();
        let json = |uri: &str| QUAD.replace(r#""buffers":[{"byteLength":60}]"#, &format!(r#""buffers":[{{"byteLength":60,"uri":"{uri}"}}]"#));
        let embedded = json(&format!("data:application/octet-stream;base64,{}", base64::engine::general_purpose::STANDARD.encode(&bin)));
        assert_eq!(load_gltf("a.gltf", embedded.as_bytes(), &|_| None, 1.0).unwrap().triangles(), 2);
        let external = json("quad%20data.bin");
        let read = |uri: &str| (uri == "quad data.bin").then(|| bin.clone());
        assert_eq!(load_gltf("b.gltf", external.as_bytes(), &read, 1.0).unwrap().triangles(), 2);
        assert!(load_gltf("c.gltf", external.as_bytes(), &|_| None, 1.0).is_err(), "a missing buffer leaves no triangles");
    }
}
