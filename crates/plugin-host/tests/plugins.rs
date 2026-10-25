//! Loads the first-party plugins built by `cargo xtask plugins`; skipped when they are not built.

use std::path::PathBuf;

use mapcraft_map::NodeKind;
use mapcraft_plugin_host::{Context, Host, MaterialInfo, Plugin};
use mapcraft_sdk::Vfs;
use serde_json::json;

fn built(id: &str) -> Option<PathBuf> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sdks/surfsup/plugins").join(format!("{id}.wasm"));
    if p.is_file() {
        Some(p)
    } else {
        eprintln!("{} is not built; run `cargo xtask plugins`", p.display());
        None
    }
}

fn load(host: &Host, id: &str, vfs: Vfs) -> Option<Plugin> {
    Some(host.load_file(&built(id)?, vfs).unwrap())
}

fn ctx() -> Context {
    Context { grid: 16.0, material: "dev/grid".into(), scale: 1.0 }
}

#[test]
fn surf_ramp_and_bhop() {
    let host = Host::new().unwrap();
    let Some(mut p) = load(&host, "surfsup", Vfs::default()) else { return };
    assert_eq!(p.manifest.id, "surfsup");
    let ramp = p.manifest.commands.iter().find(|c| c.id == "surf_ramp").unwrap().clone();
    assert!(ramp.generator);

    let mut params = ramp.defaults();
    params["surf_material"] = json!("ramps-orange1");
    let out = p.run("surf_ramp", &params, &ctx()).unwrap();
    assert_eq!(out.len(), 1);
    let NodeKind::Brush(b) = &out[0].kind else { panic!("expected a brush") };
    assert!(b.faces.iter().any(|f| f.material == "ramps-orange1"), "the surf face takes the surf material");
    assert!(b.faces.iter().any(|f| f.material == "dev/grid"), "other faces fall back to the current material");

    params["shape"] = json!("Right");
    params["smoothness"] = json!(8.0);
    assert_eq!(p.run("surf_ramp", &params, &ctx()).unwrap().len(), 8);

    params["shape"] = json!("Spiral");
    params["side_material"] = json!("side");
    for b in p.run("surf_ramp", &params, &ctx()).unwrap() {
        let NodeKind::Brush(b) = b.kind else { panic!() };
        let surf = b.faces.iter().filter(|f| f.material == "ramps-orange1").count();
        assert!((1..=2).contains(&surf), "a twisted surf face is one or two triangles, all surf: {surf}");
        assert!(b.faces.iter().filter(|f| f.material == "side").count() >= 3);
    }

    params["width"] = json!(-5);
    assert!(p.run("surf_ramp", &params, &ctx()).is_ok(), "out of range values are clamped");
    params["height"] = json!("not a number");
    assert!(p.run("surf_ramp", &params, &ctx()).is_ok(), "bad values fall back to the defaults");

    let bhop = p.manifest.commands.iter().find(|c| c.id == "bhop").unwrap().defaults();
    assert_eq!(p.run("bhop", &bhop, &ctx()).unwrap().len(), 30);
    assert!(p.run("nope", &bhop, &ctx()).is_err());
}

#[test]
fn fuel_stops_runaway_calls() {
    let host = Host::new().unwrap();
    let Some(mut p) = load(&host, "surfsup", Vfs::default()) else { return };
    p.set_fuel(10_000);
    let defaults = p.manifest.commands[1].defaults();
    let err = p.run("bhop", &defaults, &ctx()).unwrap_err().to_string();
    assert!(err.contains("too long"), "{err}");
    p.set_fuel(u64::MAX / 2);
    assert!(p.run("bhop", &defaults, &ctx()).is_err(), "a plugin that trapped stays stopped");
}

#[test]
fn godot_materials() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("Brick")).unwrap();
    std::fs::write(dir.path().join("Brick/brick_color.jpg"), b"").unwrap();
    let tres = "[gd_resource type=\"StandardMaterial3D\" format=3]\n[ext_resource type=\"Texture2D\" path=\"res://Assets/Textures/Brick/brick_color.jpg\" id=\"1_a\"]\n[resource]\nalbedo_texture = ExtResource(\"1_a\")\n";
    std::fs::write(dir.path().join("Brick/brick.tres"), tres).unwrap();
    std::fs::write(dir.path().join("env.tres"), "[gd_resource type=\"Environment\" format=3]\n").unwrap();
    let shader = "[gd_resource type=\"ShaderMaterial\" format=3]\n[resource]\nshader_parameter/base_color = Color(1, 0.4, 0, 1)\n";
    std::fs::write(dir.path().join("ramps-orange1.tres"), shader).unwrap();
    std::fs::write(dir.path().join("loose.png"), b"").unwrap();
    let host = Host::new().unwrap();
    let Some(mut p) = load(&host, "godot", Vfs::new(vec![dir.path().into()])) else { return };
    assert!(p.manifest.materials);
    let info = |name: &str, image: Option<&str>, color| MaterialInfo { name: name.into(), image: image.map(String::from), color };
    let mut got = p.materials().unwrap();
    got.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(
        got,
        vec![
            info("Brick/brick", Some("Brick/brick_color.jpg"), None),
            info("loose", Some("loose.png"), None),
            info("ramps-orange1", None, Some([255, 102, 0])),
        ]
    );
}

#[test]
fn garbage_is_refused() {
    let host = Host::new().unwrap();
    assert!(host.load(b"not wasm", Vfs::default()).is_err());
}

/// The Python example in `plugins/stairs-py`, when `MAPCRAFT_PY_PLUGIN` names its built `.wasm`.
#[test]
fn python_example() {
    let Some(path) = std::env::var_os("MAPCRAFT_PY_PLUGIN") else { return };
    let host = Host::new().unwrap();
    let mut p = host.load_file(&PathBuf::from(path), Vfs::default()).unwrap();
    assert_eq!(p.manifest.id, "stairs");
    let params = p.manifest.commands[0].defaults();
    let out = p.run("stairs", &params, &ctx()).unwrap();
    assert_eq!(out.len(), 8);
    assert_eq!(p.take_log(), vec!["stairs: 8 steps, 64 units high".to_string()]);
}
