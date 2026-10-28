//! Lists a Godot project's material resources (`.tres`) as MapCraft materials, previewed by their
//! albedo texture or color, plus images no material uses. Material names are paths under the
//! SDK's texture folder without the extension, which is how the Godot extension finds them again.

use std::collections::{BTreeMap, BTreeSet};

use mapcraft::plugin::host;
use mapcraft::plugin::types::{BlendRole, Channel, ChannelMap, Pbr};

wit_bindgen::generate!({ path: "../../wit", world: "plugin" });

const IMAGES: &[&str] = &["png", "jpg", "jpeg", "webp", "tga", "bmp"];
/// File name endings of texture maps that are not a material's color.
const MAPS: &[&str] = &[
    "ambientocclusion",
    "ao",
    "normal",
    "normalgl",
    "normaldx",
    "nrm",
    "roughness",
    "rough",
    "height",
    "displacement",
    "disp",
    "metalness",
    "metallic",
    "opacity",
    "alpha",
    "specular",
    "gloss",
    "orm",
    "arm",
    "bump",
    "mask",
];

/// Endings of a loose set's color picture, longest first so `base_color` is not read as `color`.
const COLORS: &[&str] = &["base color", "base_color", "basecolor", "albedo", "diffuse", "color", "diff", "col"];
const NORMAL_GL: &[&str] = &["normalgl", "normal", "nrm"];
const NORMAL_DX: &[&str] = &["normaldx"];
const ROUGH: &[&str] = &["roughness", "rough"];
const METAL: &[&str] = &["metalness", "metallic"];
const AO: &[&str] = &["ambientocclusion", "ao"];
/// Occlusion, roughness and metallic in red, green and blue.
const PACKED: &[&str] = &["orm", "arm"];
/// Material properties with the only value the MapCraft addon's shader wrapper keeps; Godot writes a property only when it differs from its default.
const WRAPPED: &[(&str, &str)] = &[
    ("shading_mode", "1"),
    ("cull_mode", "0"),
    ("transparency", "0"),
    ("blend_mode", "0"),
    ("diffuse_mode", "0"),
    ("specular_mode", "0"),
    ("billboard_mode", "0"),
    ("distance_fade_mode", "0"),
    ("texture_repeat", "true"),
    ("grow", "false"),
    ("no_depth_test", "false"),
    ("use_point_size", "false"),
    ("fixed_size", "false"),
    ("uv1_triplanar", "false"),
    ("uv1_world_triplanar", "false"),
    ("ao_on_uv2", "false"),
    ("vertex_color_use_as_albedo", "false"),
    ("albedo_texture_force_srgb", "false"),
    ("albedo_texture_msdf", "false"),
    ("disable_receive_shadows", "false"),
    ("disable_ambient_light", "false"),
    ("shadow_to_opacity", "false"),
    ("use_particle_trails", "false"),
    ("disable_fog", "false"),
];

fn folder(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

fn stem(path: &str) -> String {
    let file = path.rsplit('/').next().unwrap_or(path);
    file.rsplit_once('.').map_or(file, |(s, _)| s).to_ascii_lowercase()
}

/// The lowercased file stem without the first of `ends` it ends with after `_`, ` ` or `-`.
fn split_end(path: &str, ends: &[&str]) -> Option<String> {
    let stem = stem(path);
    ends.iter().find_map(|e| {
        let rest = stem.strip_suffix(e)?;
        (rest.is_empty() || rest.ends_with(['_', ' ', '-'])).then(|| rest.trim_end_matches(['_', ' ', '-']).to_string())
    })
}

/// Whether an image looks like a normal, roughness or other non-color map.
fn is_map(path: &str) -> bool {
    split_end(path, MAPS).is_some()
}

/// The Lit view's maps among one loose set's pictures, named as ambientCG names them.
fn loose_pbr(maps: &[&String]) -> Option<Pbr> {
    let find = |ends: &[&str]| maps.iter().find(|p| split_end(p, ends).is_some()).map(|p| p.to_string());
    let (gl, dx, packed) = (find(NORMAL_GL), find(NORMAL_DX), find(PACKED));
    let map = |ends: &[&str], channel| {
        find(ends).map(|path| ChannelMap { path, channel: Channel::Red }).or_else(|| Some(ChannelMap { path: packed.clone()?, channel }))
    };
    let metallic = map(METAL, Channel::Blue);
    let pbr = Pbr {
        normal_dx: gl.is_none() && dx.is_some(),
        normal: gl.or(dx),
        roughness: map(ROUGH, Channel::Green),
        roughness_value: 1.0,
        metallic_value: if metallic.is_some() { 1.0 } else { 0.0 },
        metallic,
        ao: map(AO, Channel::Red),
    };
    (pbr.normal.is_some() || pbr.roughness.is_some() || pbr.metallic.is_some() || pbr.ao.is_some()).then_some(pbr)
}

struct Godot;

/// The quoted value of `key="..."` (or `key=...` up to a space) in a resource line.
fn attr<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = &line[line.find(&format!(" {key}="))? + key.len() + 2..];
    match rest.strip_prefix('"') {
        Some(q) => q.split('"').next(),
        None => rest.split([' ', ']']).next(),
    }
}

/// What a material resource shows: every texture it references by `res://` path, the one used as
/// its albedo (else the first), its albedo or base color, and its maps by `res://` path.
struct Resource {
    textures: Vec<String>,
    albedo: Option<String>,
    color: Option<(u8, u8, u8)>,
    pbr: Pbr,
    uv_scale: Option<(f32, f32)>,
    pixelated: bool,
    blend: BlendRole,
}

/// Godot's texture channel numbers: red, green, blue, alpha, then grayscale.
fn channel(v: Option<&str>) -> Channel {
    match v.and_then(|v| v.parse::<u8>().ok()) {
        Some(1) => Channel::Green,
        Some(2) => Channel::Blue,
        Some(3) => Channel::Alpha,
        Some(4) => Channel::Gray,
        _ => Channel::Red,
    }
}

/// The text inside the first parentheses of a value such as `ExtResource("1_a")` or `Color(1, 0, 0, 1)`.
fn call_args(v: &str) -> Option<&str> {
    v.split_once('(').and_then(|(_, r)| r.split_once(')')).map(|(i, _)| i.trim())
}

fn parse(text: &str) -> Resource {
    let mut textures: Vec<(String, String)> = Vec::new();
    let mut props: BTreeMap<&str, &str> = BTreeMap::new();
    let (mut albedo_id, mut color) = (None, None);
    let class = text.lines().next().and_then(|h| attr(h, "type")).unwrap_or_default();
    let mut in_resource = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with("[ext_resource") && attr(line, "type").is_some_and(|t| t.contains("Texture")) {
            if let (Some(id), Some(path)) = (attr(line, "id"), attr(line, "path")) {
                textures.push((id.to_string(), path.to_string()));
            }
            continue;
        }
        if line.starts_with('[') {
            in_resource = line.starts_with("[resource]");
        }
        if !in_resource {
            continue;
        }
        let Some((key, value)) = line.split_once('=').map(|(k, v)| (k.trim(), v.trim())) else { continue };
        props.insert(key, value);
        if key == "albedo_texture" || key.ends_with("albedo") || key.ends_with("albedo_texture") {
            // ExtResource("1_abc") in Godot 4, ExtResource( 1 ) in Godot 3.
            albedo_id = call_args(value).map(|i| i.trim_matches('"').to_string());
        } else if color.is_none() && (key == "albedo_color" || key.ends_with("color")) && value.starts_with("Color") {
            let c: Vec<f64> = call_args(value).into_iter().flat_map(|a| a.split(',')).filter_map(|n| n.trim().parse().ok()).collect();
            if let [r, g, b, ..] = c[..] {
                let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                color = Some((byte(r), byte(g), byte(b)));
            }
        }
    }
    let prop = |key: &str| props.get(key).copied();
    let texture = |key: &str| {
        let id = call_args(prop(key)?)?.trim_matches('"');
        textures.iter().find(|(i, _)| i == id).map(|(_, p)| p.clone())
    };
    let on = |key: &str| prop(key) == Some("true");
    let number = |key: &str, default: f32| prop(key).and_then(|v| v.parse().ok()).unwrap_or(default);
    let map = |name: &str| Some(ChannelMap { path: texture(&format!("{name}_texture"))?, channel: channel(prop(&format!("{name}_texture_channel"))) });
    let pbr = Pbr {
        normal: texture("normal_texture").filter(|_| on("normal_enabled")),
        normal_dx: false,
        roughness: map("roughness"),
        roughness_value: number("roughness", 1.0),
        metallic: map("metallic"),
        metallic_value: number("metallic", 0.0),
        ao: map("ao").filter(|_| on("ao_enabled")),
    };
    let scale: Vec<f32> = prop("uv1_scale").and_then(call_args).into_iter().flat_map(|a| a.split(',')).filter_map(|n| n.trim().parse().ok()).collect();
    let uv_scale = match scale[..] {
        [x, y, ..] if (x, y) != (1.0, 1.0) && !on("uv1_triplanar") => Some((x, y)),
        _ => None,
    };
    let wraps = class == "StandardMaterial3D"
        && WRAPPED.iter().all(|&(k, v)| prop(k).is_none_or(|p| p == v))
        && prop("depth_draw_mode") != Some("2")
        && props.iter().all(|(k, v)| !k.ends_with("_enabled") || *v != "true" || ["normal_enabled", "ao_enabled"].contains(k));
    let found = albedo_id.and_then(|id| textures.iter().find(|(i, _)| *i == id)).or(textures.first());
    Resource {
        albedo: found.map(|(_, p)| p.clone()),
        textures: textures.iter().map(|(_, p)| p.clone()).collect(),
        color,
        pbr,
        uv_scale,
        pixelated: matches!(prop("texture_filter"), Some("0" | "2" | "4")),
        blend: if wraps { BlendRole::Layer } else { BlendRole::Unblendable },
    }
}

/// The asset-folder path of a `res://` file: the longest tail of it that exists in `files`.
fn locate(res: &str, files: &BTreeSet<String>) -> Option<String> {
    let mut rest = res.strip_prefix("res://").unwrap_or(res);
    loop {
        if files.contains(rest) {
            return Some(rest.to_string());
        }
        rest = rest.split_once('/')?.1;
    }
}

impl Guest for Godot {
    fn manifest() -> Manifest {
        Manifest {
            id: "godot".into(),
            name: "Godot".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            commands: Vec::new(),
            materials: true,
            importers: Vec::new(),
            exporters: Vec::new(),
            validates: false,
            models: Vec::new(),
        }
    }

    fn run_command(id: String, _params: Vec<Param>, _context: Context) -> Result<Output, String> {
        Err(format!("unknown command {id}"))
    }

    fn import_file(_importer: String, _name: String, _data: Vec<u8>, _context: Context) -> Result<Output, String> {
        Err("this plugin imports no files".into())
    }

    fn export_file(_exporter: String, _map: MapData, _context: Context) -> Result<Vec<u8>, String> {
        Err("this plugin exports no files".into())
    }

    fn list_materials() -> Result<Vec<MaterialInfo>, String> {
        let images: BTreeSet<String> = host::list_files(&IMAGES.iter().map(|s| s.to_string()).collect::<Vec<_>>()).into_iter().collect();
        let mut used = BTreeSet::new();
        let mut out = Vec::new();
        for path in host::list_files(&["tres".to_string()]) {
            let Ok(bytes) = host::read_file(&path) else { continue };
            let text = String::from_utf8_lossy(&bytes);
            let header = text.lines().next().unwrap_or_default();
            if !header.starts_with("[gd_resource") || !attr(header, "type").is_some_and(|t| t.ends_with("Material") || t.ends_with("Material3D")) {
                continue;
            }
            let res = parse(&text);
            used.extend(res.textures.iter().filter_map(|t| locate(t, &images)));
            let name = path.strip_suffix(".tres").unwrap_or(&path).to_string();
            let at = |m: Option<ChannelMap>| m.and_then(|m| Some(ChannelMap { path: locate(&m.path, &images)?, channel: m.channel }));
            let p = res.pbr;
            let pbr = Pbr { normal: p.normal.and_then(|n| locate(&n, &images)), roughness: at(p.roughness), metallic: at(p.metallic), ao: at(p.ao), ..p };
            out.push(MaterialInfo {
                name,
                image: res.albedo.and_then(|t| locate(&t, &images)),
                color: res.color,
                pixels: None,
                size: None,
                pbr: Some(pbr),
                uv_scale: res.uv_scale,
                pixelated: res.pixelated,
                blend: res.blend,
            });
        }
        // Images no material uses are materials of their own, except the extra maps of a folder
        // that already has a material or a color image.
        let loose: Vec<&String> = images.difference(&used).collect();
        let covered: BTreeSet<String> =
            out.iter().map(|m| folder(&m.name).to_string()).chain(loose.iter().filter(|i| !is_map(i)).map(|i| folder(i).to_string())).collect();
        let mut sets: BTreeMap<(&str, String), Vec<&String>> = BTreeMap::new();
        for img in &loose {
            if let Some(s) = split_end(img, MAPS) {
                sets.entry((folder(img), s)).or_default().push(img);
            }
        }
        for img in loose {
            let map = is_map(img);
            if map && covered.contains(folder(img)) {
                continue;
            }
            let set = if map { None } else { sets.get(&(folder(img), split_end(img, COLORS).unwrap_or_else(|| stem(img)))) };
            let name = img.rsplit_once('.').map_or(img.as_str(), |(n, _)| n).to_string();
            let pbr = set.and_then(|s| loose_pbr(s));
            out.push(MaterialInfo {
                name,
                image: Some(img.clone()),
                color: None,
                pixels: None,
                size: None,
                pbr,
                uv_scale: None,
                pixelated: false,
                blend: BlendRole::Layer,
            });
        }
        Ok(out)
    }
    fn load_materials(names: Vec<String>) -> Vec<Option<Image>> {
        names.iter().map(|_| None).collect()
    }

    fn load_model(path: String) -> Result<Model, String> {
        Err(format!("this plugin reads no models: {path}"))
    }

    fn validate(_map: MapData) -> Vec<Issue> {
        Vec::new()
    }
}

export!(Godot);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_godot_4_and_3_materials() {
        let g4 = "[gd_resource type=\"StandardMaterial3D\" load_steps=3 format=3]\n\n[ext_resource type=\"Texture2D\" uid=\"uid://a\" path=\"res://Assets/Textures/Brick/n.jpg\" id=\"2_n\"]\n[ext_resource type=\"Texture2D\" uid=\"uid://b\" path=\"res://Assets/Textures/Brick/c.jpg\" id=\"1_c\"]\n\n[resource]\nnormal_texture = ExtResource(\"2_n\")\nalbedo_texture = ExtResource(\"1_c\")\n";
        let r = parse(g4);
        assert_eq!(r.albedo.as_deref(), Some("res://Assets/Textures/Brick/c.jpg"));
        assert_eq!(r.textures.len(), 2);
        let g3 = "[gd_resource type=\"SpatialMaterial\" load_steps=2 format=2]\n[ext_resource path=\"res://t.png\" type=\"Texture\" id=1]\n[resource]\nalbedo_texture = ExtResource( 1 )\nalbedo_color = Color( 0.5, 0.5, 0.5, 1 )\n";
        let r = parse(g3);
        assert_eq!(r.albedo.as_deref(), Some("res://t.png"));
        assert_eq!(r.color, Some((128, 128, 128)));
        let files = BTreeSet::from(["Brick/c.jpg".to_string()]);
        assert_eq!(locate("res://Assets/Textures/Brick/c.jpg", &files).as_deref(), Some("Brick/c.jpg"));
        assert_eq!(locate("res://Other/x.jpg", &files), None);
        assert!(is_map("Brick/BrickWall (10)_Normal.png") && is_map("b/Bricks064_1K-JPG_AmbientOcclusion.jpg"));
        assert!(!is_map("Brick/BrickWall (10)_BaseColor.png") && !is_map("aurora.png") && is_map("Grass/Normal.png"));
    }

    #[test]
    fn reads_material_maps() {
        let tres = "[gd_resource type=\"StandardMaterial3D\" format=3]\n[ext_resource type=\"Texture2D\" path=\"res://n.png\" id=\"1_n\"]\n[ext_resource type=\"Texture2D\" path=\"res://r.png\" id=\"2_r\"]\n[ext_resource type=\"Texture2D\" path=\"res://ao.png\" id=\"3_a\"]\n[resource]\nmetallic = 0.88\nroughness_texture = ExtResource(\"2_r\")\nroughness_texture_channel = 1\nnormal_enabled = true\nnormal_texture = ExtResource(\"1_n\")\nao_texture = ExtResource(\"3_a\")\n";
        let p = parse(tres).pbr;
        assert_eq!(p.normal.as_deref(), Some("res://n.png"));
        let r = p.roughness.unwrap();
        assert_eq!((r.path.as_str(), r.channel), ("res://r.png", Channel::Green));
        assert_eq!((p.roughness_value, p.metallic_value), (1.0, 0.88), "unstated values keep Godot's defaults");
        assert!(p.metallic.is_none() && p.ao.is_none(), "ao_texture needs ao_enabled");
        assert!(parse(&tres.replace("normal_enabled = true\n", "")).pbr.normal.is_none(), "normal_texture needs normal_enabled");
    }

    #[test]
    fn reads_scale_filter_and_blend_role() {
        let tres = |props: &str| parse(&format!("[gd_resource type=\"StandardMaterial3D\" format=3]\n[resource]\n{props}\n"));
        assert_eq!(tres("uv1_scale = Vector3(2, 2, 1)").uv_scale, Some((2.0, 2.0)));
        assert_eq!(tres("uv1_scale = Vector3(0.01, 0.01, 0.01)\nuv1_triplanar = true").uv_scale, None, "triplanar scales are in world space");
        assert_eq!(tres("uv1_scale = Vector3(1, 1, 1)").uv_scale, None);
        let pixelated: Vec<bool> = ["0", "2", "4", "5"].iter().map(|f| tres(&format!("texture_filter = {f}")).pixelated).collect();
        assert_eq!(pixelated, [true, true, true, false]);
        assert!(!tres("").pixelated, "Godot's default filter is smooth");
        assert_eq!(tres("normal_enabled = true\nao_enabled = true\ntexture_filter = 0").blend, BlendRole::Layer);
        for refused in ["emission_enabled = true", "shading_mode = 0", "cull_mode = 2", "uv1_triplanar = true", "depth_draw_mode = 2"] {
            assert_eq!(tres(refused).blend, BlendRole::Unblendable, "{refused}");
        }
        let shader = parse("[gd_resource type=\"ShaderMaterial\" format=3]\n[resource]\nshader_parameter/base_color = Color(1, 0.4, 0, 1)\n");
        assert_eq!(shader.blend, BlendRole::Unblendable);
        let noise = tres("albedo_texture = SubResource(\"1\")\n[sub_resource type=\"FastNoiseLite\" id=\"2\"]\ndomain_warp_enabled = true");
        assert_eq!(noise.blend, BlendRole::Layer, "only the material's own properties count");
    }

    #[test]
    fn pairs_loose_sets_by_stem() {
        let files = ["B/Bricks_1K_NormalDX.jpg", "B/Bricks_1K_NormalGL.jpg", "B/Bricks_1K_Roughness.jpg", "B/Bricks_1K_AmbientOcclusion.jpg"].map(String::from);
        let set: Vec<&String> = files.iter().collect();
        assert_eq!(split_end("B/Bricks_1K_Color.jpg", COLORS).as_deref(), Some("bricks_1k"));
        assert_eq!(split_end("G/Gold_Base Color.jpg", COLORS).as_deref(), Some("gold"));
        let p = loose_pbr(&set).unwrap();
        assert_eq!((p.normal.as_deref(), p.normal_dx), (Some("B/Bricks_1K_NormalGL.jpg"), false), "GL wins over DX");
        assert_eq!(p.roughness.unwrap().path, "B/Bricks_1K_Roughness.jpg");
        assert_eq!((p.metallic.is_none(), p.metallic_value), (true, 0.0));
        let p = loose_pbr(&set[..1]).unwrap();
        assert_eq!((p.normal.as_deref(), p.normal_dx), (Some("B/Bricks_1K_NormalDX.jpg"), true));
        let arm = "M/Metal_ARM.png".to_string();
        let p = loose_pbr(&[&arm]).unwrap();
        let channels = [p.ao, p.roughness, p.metallic].map(|m| m.map(|m| m.channel));
        assert_eq!(channels, [Some(Channel::Red), Some(Channel::Green), Some(Channel::Blue)]);
        assert_eq!(p.metallic_value, 1.0);
    }
}
