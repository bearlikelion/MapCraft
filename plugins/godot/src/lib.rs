//! Lists a Godot project's material resources (`.tres`) as MapCraft materials, previewed by their
//! albedo texture or color, plus images no material uses. Material names are paths under the
//! SDK's texture folder without the extension, which is how the Godot extension finds them again.

use std::collections::BTreeSet;

use mapcraft::plugin::host;

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

fn folder(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

/// Whether an image looks like a normal, roughness or other non-color map.
fn is_map(path: &str) -> bool {
    let file = path.rsplit('/').next().unwrap_or(path);
    let stem = file.rsplit_once('.').map_or(file, |(s, _)| s).to_ascii_lowercase();
    MAPS.iter().any(|m| stem == *m || ['_', ' ', '-'].iter().any(|sep| stem.ends_with(&format!("{sep}{m}"))))
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
/// its albedo (else the first), and its albedo or base color.
struct Resource {
    textures: Vec<String>,
    albedo: Option<String>,
    color: Option<(u8, u8, u8)>,
}

/// The text inside the first parentheses of a value such as `ExtResource("1_a")` or `Color(1, 0, 0, 1)`.
fn call_args(v: &str) -> Option<&str> {
    v.split_once('(').and_then(|(_, r)| r.split_once(')')).map(|(i, _)| i.trim())
}

fn parse(text: &str) -> Resource {
    let mut textures: Vec<(String, String)> = Vec::new();
    let (mut albedo_id, mut color) = (None, None);
    for line in text.lines().map(str::trim) {
        if line.starts_with("[ext_resource") && attr(line, "type").is_some_and(|t| t.contains("Texture")) {
            if let (Some(id), Some(path)) = (attr(line, "id"), attr(line, "path")) {
                textures.push((id.to_string(), path.to_string()));
            }
            continue;
        }
        let Some((key, value)) = line.split_once('=').map(|(k, v)| (k.trim(), v.trim())) else { continue };
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
    let found = albedo_id.and_then(|id| textures.iter().find(|(i, _)| *i == id)).or(textures.first());
    Resource { albedo: found.map(|(_, p)| p.clone()), textures: textures.into_iter().map(|(_, p)| p).collect(), color }
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
        }
    }

    fn run_command(id: String, _params: Vec<Param>, _context: Context) -> Result<Output, String> {
        Err(format!("unknown command {id}"))
    }

    fn import_file(_importer: String, _name: String, _data: Vec<u8>, _context: Context) -> Result<Output, String> {
        Err("this plugin imports no files".into())
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
            out.push(MaterialInfo { name, image: res.albedo.and_then(|t| locate(&t, &images)), color: res.color });
        }
        // Images no material uses are materials of their own, except the extra maps of a folder
        // that already has a material or a color image.
        let loose: Vec<&String> = images.difference(&used).collect();
        let covered: BTreeSet<String> =
            out.iter().map(|m| folder(&m.name).to_string()).chain(loose.iter().filter(|i| !is_map(i)).map(|i| folder(i).to_string())).collect();
        for img in loose {
            if is_map(img) && covered.contains(folder(img)) {
                continue;
            }
            let name = img.rsplit_once('.').map_or(img.as_str(), |(n, _)| n).to_string();
            out.push(MaterialInfo { name, image: Some(img.clone()), color: None });
        }
        Ok(out)
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
}
