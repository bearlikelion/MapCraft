//! Loads the first-party plugins built by `cargo xtask plugins`; skipped when they are not built.

use std::path::PathBuf;

use mapcraft_geom::DVec3;
use mapcraft_map::{BlendRole, NodeKind};
use mapcraft_plugin_host::{Channel, ChannelMap, Context, Host, MaterialInfo, Pbr, Plugin};
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
    Context { grid: 16.0, material: "dev/grid".into(), scale: 1.0, game: "Quake".into() }
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

    let mut trimmed = params.clone();
    trimmed["trim_top"] = json!(64.0);
    trimmed["trim_material"] = json!("trim");
    let out = p.run("surf_ramp", &trimmed, &ctx()).unwrap();
    assert_eq!(out.len(), 2, "the ramp and its trim");
    assert!(matches!(&out[1].kind, NodeKind::Entity(e) if e.classname == "func_illusionary"), "trim is drawn but never collides");
    assert!(out[1].children.iter().all(|c| matches!(&c.kind, NodeKind::Brush(b) if b.faces.iter().all(|f| f.material == "trim"))));

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

    let fitted = p.scale_params("surf_ramp", &ramp.defaults(), DVec3::new(2.0, 1.0, 1.5)).unwrap().unwrap();
    assert_eq!((fitted["size"].as_f64(), fitted["height"].as_f64(), fitted["shape"].as_str()), (Some(2048.0), Some(816.0), Some("Straight")));
    assert!(fitted["trim_depth"].as_f64().is_some_and(|d| d > 1.0), "trim depth follows the slab: {}", fitted["trim_depth"]);
    assert!(p.scale_params("bhop", &bhop, DVec3::splat(2.0)).unwrap().is_none(), "a generator the plugin cannot fit keeps its stretch");
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
    std::fs::create_dir_all(dir.path().join("Rock")).unwrap();
    for file in ["Brick/brick_color.jpg", "Brick/brick_n.jpg", "Rock/Rock_1K_Color.jpg", "Rock/Rock_1K_NormalDX.jpg", "Rock/Rock_1K_Roughness.jpg", "loose.png"]
    {
        std::fs::write(dir.path().join(file), b"").unwrap();
    }
    let tres = "[gd_resource type=\"StandardMaterial3D\" format=3]\n[ext_resource type=\"Texture2D\" path=\"res://Assets/Textures/Brick/brick_color.jpg\" id=\"1_a\"]\n[ext_resource type=\"Texture2D\" path=\"res://Assets/Textures/Brick/brick_n.jpg\" id=\"2_n\"]\n[resource]\nalbedo_texture = ExtResource(\"1_a\")\nnormal_enabled = true\nnormal_texture = ExtResource(\"2_n\")\nroughness = 0.5\nmetallic = 7\n";
    std::fs::write(dir.path().join("Brick/brick.tres"), tres).unwrap();
    std::fs::write(dir.path().join("env.tres"), "[gd_resource type=\"Environment\" format=3]\n").unwrap();
    let shader = "[gd_resource type=\"ShaderMaterial\" format=3]\n[resource]\nshader_parameter/base_color = Color(1, 0.4, 0, 1)\n";
    std::fs::write(dir.path().join("ramps-orange1.tres"), shader).unwrap();
    let host = Host::new().unwrap();
    let Some(mut p) = load(&host, "godot", Vfs::new(vec![dir.path().into()])) else { return };
    assert!(p.manifest.materials);
    assert!(p.scale_params("any", &json!({}), DVec3::ONE).unwrap().is_none(), "plugins built without `resize` still load");
    let pbr = |normal: Option<&str>, normal_dx, roughness_value, metallic_value| Pbr {
        normal: normal.map(String::from),
        normal_dx,
        roughness: None,
        roughness_value,
        metallic: None,
        metallic_value,
        ao: None,
    };
    let info = |name: &str, image: Option<&str>, color, pbr| MaterialInfo {
        name: name.into(),
        folder: None,
        image: image.map(String::from),
        color,
        pixels: None,
        size: None,
        pbr,
        uv_scale: None,
        pixelated: false,
        blend: BlendRole::Layer,
    };
    let rock = Pbr {
        roughness: Some(ChannelMap { path: "Rock/Rock_1K_Roughness.jpg".into(), channel: Channel::Red }),
        ..pbr(Some("Rock/Rock_1K_NormalDX.jpg"), true, 1.0, 0.0)
    };
    let mut got = p.materials().unwrap();
    got.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(
        got,
        vec![
            info("Brick/brick", Some("Brick/brick_color.jpg"), None, Some(pbr(Some("Brick/brick_n.jpg"), false, 0.5, 1.0))),
            info("Rock/Rock_1K_Color", Some("Rock/Rock_1K_Color.jpg"), None, Some(rock)),
            info("loose", Some("loose.png"), None, None),
            MaterialInfo { blend: BlendRole::Unblendable, ..info("ramps-orange1", None, Some([255, 102, 0]), Some(pbr(None, false, 1.0, 0.0))) },
        ]
    );
}

#[test]
fn garbage_is_refused() {
    let host = Host::new().unwrap();
    assert!(host.load(b"not wasm", Vfs::default()).is_err());
}

#[test]
fn plugins_for_the_old_api_ask_to_be_rebuilt() {
    let name = b"mapcraft:plugin/host@0.1.0";
    let mut old = b"\0asm\x0d\0\x01\0\x07\x03\x01\x42\x00\x0a".to_vec();
    old.push(name.len() as u8 + 5);
    old.extend([1, 0, name.len() as u8]);
    old.extend(name);
    old.extend([5, 0]);
    let err = Host::new().unwrap().load(&old, Vfs::default()).err().unwrap().to_string();
    assert_eq!(err, "built for plugin API 0.1.0; rebuild it against 0.3.0");
}

/// Coplanar displaced faces from the VMF named by `MAPCRAFT_DISP_VMF` meet along nearly every shared edge, which swapped rows and columns would break; faces at an angle differ by the sideways offsets MapCraft drops.
#[test]
fn vmf_displacements_meet_at_their_seams() {
    use mapcraft_format::Fragment;
    use mapcraft_geom::DVec3;
    use mapcraft_map::Brush;
    let Some(path) = std::env::var_os("MAPCRAFT_DISP_VMF") else { return };
    let wasm = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sdks/cstrike/plugins/source.wasm");
    let mut p = Host::new().unwrap().load_file(&wasm, Vfs::default()).unwrap();
    let imported = p.import("vmf", "seams.vmf", &std::fs::read(path).unwrap(), &ctx()).unwrap();
    fn brushes(nodes: &[Fragment], out: &mut Vec<Brush>) {
        for f in nodes {
            if let NodeKind::Brush(b) = &f.kind {
                out.push(b.clone());
            }
            brushes(&f.children, out);
        }
    }
    let mut all = Vec::new();
    brushes(&imported.nodes, &mut all);
    let key = |p: DVec3| [p.x, p.y, p.z].map(|c| (c * 8.0).round() as i64);
    let mut edges: std::collections::HashMap<[[i64; 3]; 2], Vec<(DVec3, Vec<DVec3>)>> = std::collections::HashMap::new();
    let mut faces = 0;
    for b in &all {
        let Ok(solid) = b.solid() else { continue };
        for (i, v) in b.displaced(&solid) {
            let (Some(d), Some(Some(w)), Some(plane)) = (b.faces[i].disp.as_ref(), solid.faces.get(i), b.faces[i].plane()) else { continue };
            let Some(c) = d.corners(&w.0) else { continue };
            faces += 1;
            let n = d.size();
            let line = |from: usize, step: usize| (0..n).map(|t| v[from + t * step]).collect::<Vec<_>>();
            for (a, z, points) in [(c[0], c[1], line(0, 1)), (c[1], c[2], line(n - 1, n)), (c[3], c[2], line(n * (n - 1), 1)), (c[0], c[3], line(0, n))] {
                let (ka, kz) = (key(a), key(z));
                let (k, points) = if ka <= kz { ([ka, kz], points) } else { ([kz, ka], points.into_iter().rev().collect()) };
                edges.entry(k).or_default().push((plane.normal, points));
            }
        }
    }
    let shared: Vec<_> = edges.values().filter(|l| l.len() == 2 && l[0].1.len() == l[1].1.len() && l[0].0.dot(l[1].0) > 0.999).collect();
    let apart = shared.iter().filter(|l| l[0].1.iter().zip(&l[1].1).any(|(a, b)| a.distance(*b) > 0.1)).count();
    eprintln!("{faces} displaced faces, {} coplanar shared edges, {apart} apart: {:?}", shared.len(), p.take_log());
    assert!(!shared.is_empty(), "the map has no coplanar displacements that touch");
    assert!(apart * 50 <= shared.len(), "{apart} of {} seams are open", shared.len());
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

/// Mark's surf_emerald, when it is on this machine: groups, aligned faces and embedded textures.
#[test]
fn bsp_import_groups_and_textures() {
    let map = PathBuf::from("/mnt/Storage/Gamedev/SurfsUp/SurfMaps/BSPs/surf_emerald.bsp");
    let host = Host::new().unwrap();
    let (Some(mut p), Ok(data)) = (load(&host, "source-bsp", Vfs::default()), std::fs::read(&map)) else { return };
    let imported = p.import("bsp", "surf_emerald.bsp", &data, &ctx()).unwrap();
    let groups: Vec<String> = imported
        .nodes
        .iter()
        .filter_map(|f| match &f.kind {
            NodeKind::Group(g) => Some(g.name.clone()),
            _ => None,
        })
        .collect();
    assert!(groups.iter().any(|g| g == "Ramps") && groups.iter().any(|g| g == "World"), "{groups:?}");
    assert!(imported.materials.len() >= 19, "every embedded texture comes back: {}", imported.materials.len());
    assert!(imported.materials.iter().all(|m| m.rgba.len() == (m.width * m.height * 4) as usize));
    let ramp = imported.nodes.iter().find(|f| matches!(&f.kind, NodeKind::Group(g) if g.name == "Ramps")).unwrap();
    let NodeKind::Brush(b) = &ramp.children[0].kind else { panic!("ramps hold brushes") };
    assert!(b.faces.iter().any(|f| f.u.axis != mapcraft_geom::DVec3::X && f.u.axis != mapcraft_geom::DVec3::Y), "faces keep the map's texture axes");
}

/// The SurfsUp SDK pack that `MAPCRAFT_SURFSUP_PCK` names imports; `MAPCRAFT_SURFSUP_OUT` names a `.mcmap` to save it as.
#[test]
fn surfsup_imports_a_real_pack() {
    use mapcraft_format::Fragment;
    use mapcraft_map::{Map, NodeId, ROOT};
    let Some(path) = std::env::var_os("MAPCRAFT_SURFSUP_PCK") else { return };
    let host = Host::new().unwrap();
    let Some(mut p) = load(&host, "surfsup", Vfs::default()) else { return };
    let imported = p.import("pck", "real.pck", &std::fs::read(path).unwrap(), &ctx()).unwrap();
    fn count(nodes: &[Fragment]) -> (usize, usize) {
        nodes.iter().fold((0, 0), |(b, e), f| {
            let (cb, ce) = count(&f.children);
            (b + cb + usize::from(matches!(f.kind, NodeKind::Brush(_))), e + ce + usize::from(matches!(f.kind, NodeKind::Entity(_))))
        })
    }
    fn insert(e: &mut mapcraft_map::Edit<'_>, parent: NodeId, nodes: Vec<Fragment>) {
        for f in nodes {
            let id = e.insert(parent, f.kind).unwrap();
            insert(e, id, f.children);
        }
    }
    let (brushes, entities) = count(&imported.nodes);
    let log = p.take_log();
    eprintln!("{brushes} brushes, {entities} entities, {} materials: {log:#?}", imported.materials.len());
    let coverage = log.iter().find_map(|l| {
        let (on, rest) = l.split_once("; ")?.1.split_once(" of their ")?;
        Some((on.parse::<f64>().ok()?, rest.split(' ').next()?.parse::<f64>().ok()?))
    });
    if let Some((on, all)) = coverage {
        eprintln!("ramp coverage: {on} of {all} collision triangles lie on a brush face ({:.1}%)", 100.0 * on / all);
    }
    assert!(brushes > 0);
    if let Some(out) = std::env::var_os("MAPCRAFT_SURFSUP_OUT") {
        let mut map = Map::new();
        let mut e = map.edit();
        insert(&mut e, ROOT, imported.nodes);
        map.apply(e.finish());
        mapcraft_format::save(&PathBuf::from(out), &map, Some("surfsup"), &json!({})).unwrap();
    }
}

/// SurfsUp checks report map-wide problems and point zone problems back at their node.
#[test]
fn surfsup_validates_zones() {
    use mapcraft_geom::{Aabb, DVec3};
    use mapcraft_map::{Brush, Entity, Map, ROOT, TexDefaults};
    let host = Host::new().unwrap();
    let Some(mut p) = load(&host, "surfsup", Vfs::default()) else { return };
    let mut map = Map::new();
    let mut e = map.edit();
    let tex = TexDefaults { material: "tools/trigger".into(), scale: 1.0 };
    for (class, x) in [("zone_start", 0.0), ("zone_finish", 2048.0)] {
        let zone = e.insert(ROOT, NodeKind::Entity(Entity::new(class))).unwrap();
        e.insert(zone, NodeKind::Brush(Brush::cuboid(Aabb::new(DVec3::new(x, 0.0, 0.0), DVec3::new(x + 256.0, 256.0, 256.0)), &tex))).unwrap();
    }
    map.apply(e.finish());

    let mut e = map.edit();
    let teleport = e.insert(ROOT, NodeKind::Entity(Entity::new("teleport_zone"))).unwrap();
    map.apply(e.finish());
    let issues = p.validate(&map, "surf_test").unwrap();
    assert!(issues.iter().any(|i| i.error && i.node.is_none() && i.message.contains("info_player_start")), "{issues:?}");
    assert!(issues.iter().any(|i| i.node == Some(teleport) && i.message.contains("no Destination")), "issues point back at their node: {issues:?}");
}

/// A SurfsUp SDK scene comes in as brushes, zones and a material for its color.
#[test]
fn surfsup_imports_sdk_scenes() {
    let host = Host::new().unwrap();
    let Some(mut p) = load(&host, "surfsup", Vfs::default()) else { return };
    let scene = r#"[gd_scene format=3]

[sub_resource type="StandardMaterial3D" id="red"]
albedo_color = Color(1, 0, 0, 1)

[sub_resource type="BoxShape3D" id="box"]
size = Vector3(4, 2, 4)

[node name="Map" type="Node3D"]

[node name="SpawnPoint" type="Marker3D" parent="."]

[node name="Deck" type="CSGBox3D" parent="."]
use_collision = true
size = Vector3(4, 1, 4)
material = SubResource("red")

[node name="StartZone" type="Area3D" parent="."]

[node name="Shape" type="CollisionShape3D" parent="StartZone"]
shape = SubResource("box")
"#;
    assert!(p.manifest.importers.iter().any(|i| i.extensions.contains(&"pck".to_string())));
    let imported = p.import("pck", "deck.tscn", scene.as_bytes(), &ctx()).unwrap();
    let classes: Vec<&str> = imported.nodes.iter().filter_map(|n| n.kind.entity()).map(|e| e.classname.as_str()).collect();
    assert_eq!(classes, ["info_player_start", "zone_start"]);
    let deck = imported.nodes.iter().find(|n| matches!(&n.kind, NodeKind::Group(g) if g.name == "Deck")).unwrap();
    let NodeKind::Brush(b) = &deck.children[0].kind else { panic!("the deck is not a brush") };
    assert!(b.faces.iter().all(|f| f.material == "deck/red"));
    assert_eq!(imported.materials.first().map(|m| (m.name.as_str(), &m.rgba[..4])), Some(("deck/red", &[255, 0, 0, 255][..])));
    assert!(p.take_log().iter().any(|l| l.contains("2 brushes, 1 groups, 1 zones")));
}

/// A cached host compiles a plugin once; the next load reads the compiled code back.
#[test]
fn compiled_plugins_are_cached() {
    let Some(path) = built("surfsup") else { return };
    let dir = tempfile::tempdir().unwrap();
    let time = |host: &Host| {
        let start = std::time::Instant::now();
        let p = host.load_file(&path, Vfs::default()).unwrap();
        assert_eq!(p.manifest.id, "surfsup");
        start.elapsed()
    };
    let first = time(&Host::with_cache(dir.path()).unwrap());
    let again = time(&Host::with_cache(dir.path()).unwrap());
    eprintln!("first load {first:?}, cached load {again:?}");
    let cached = std::fs::read_dir(dir.path()).unwrap().count();
    assert!(cached > 0, "the cache folder fills");
}

/// Counter-Strike: Source, when Mark's install and decompiled surf map are on this machine: the game's materials, and a `.vmf` that keeps its brushes and entities through import, export and import again.
#[test]
fn vmf_round_trip_and_game_materials() {
    use mapcraft_format::Fragment;
    use mapcraft_map::{Map, NodeId, ROOT};
    let wasm = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sdks/cstrike/plugins/source.wasm");
    let css = PathBuf::from("/mnt/Storage/Gamedev/SurfsUp/CSS-Windows");
    let vmf = "/mnt/Storage/Gamedev/SurfsUp/SurfMaps/Decomp/surf_activation_d.vmf";
    let (true, true, Ok(data)) = (wasm.is_file(), css.is_dir(), std::fs::read(vmf)) else {
        eprintln!("skipped: source.wasm, the CS:S install or {vmf} is missing");
        return;
    };
    let host = Host::new().unwrap();
    let mut p = host.load_file(&wasm, Vfs::new(vec![css.join("cstrike"), css.join("hl2")])).unwrap();
    let start = std::time::Instant::now();
    let materials = p.materials().unwrap();
    let pictured = materials.iter().filter(|m| m.pixels.is_some()).count();
    eprintln!("{} materials, {pictured} with pictures, in {:?}: {:?}", materials.len(), start.elapsed(), p.take_log());
    assert!(materials.len() > 3000 && pictured * 10 > materials.len() * 9);
    let shrunk = materials.iter().filter_map(|m| Some((m.pixels.as_ref()?.width, m.size?[0]))).filter(|(w, full)| full > w).count();
    assert!(shrunk > 0, "previews smaller than their texture say how big the texture is");

    fn count(nodes: &[Fragment]) -> (usize, usize) {
        nodes.iter().fold((0, 0), |(b, e), f| {
            let (cb, ce) = count(&f.children);
            (b + cb + usize::from(matches!(f.kind, NodeKind::Brush(_))), e + ce + usize::from(matches!(f.kind, NodeKind::Entity(_))))
        })
    }
    fn insert(e: &mut mapcraft_map::Edit<'_>, parent: NodeId, nodes: Vec<Fragment>) {
        for f in nodes {
            let id = e.insert(parent, f.kind).unwrap();
            insert(e, id, f.children);
        }
    }
    let first = p.import("vmf", "surf_activation_d.vmf", &data, &ctx()).unwrap();
    let before = count(&first.nodes);
    let mut map = Map::new();
    let mut e = map.edit();
    insert(&mut e, ROOT, first.nodes);
    map.apply(e.finish());
    let text = p.export("vmf", &map, "surf_activation_d", &ctx()).unwrap();
    let again = p.import("vmf", "surf_activation_d.vmf", &text, &ctx()).unwrap();
    eprintln!("{before:?} {:?}", p.take_log());
    assert_eq!(count(&again.nodes), before);
    assert!(before.0 > 7000 && before.1 == 4411, "{before:?}");
}
