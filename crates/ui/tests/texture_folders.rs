//! Texture folders added in SDK Settings.

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use mapcraft_ui::{MapCraftApp, Options};

/// The material list once plugins have finished listing theirs.
fn listed(h: &mut Harness<MapCraftApp>) -> Vec<String> {
    for _ in 0..400 {
        h.run_steps(1);
        let (names, loading) = h.state().materials_for_test();
        if !loading {
            return names;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    panic!("plugins never finished listing materials");
}

const PAINTED: &str = "[gd_resource type=\"StandardMaterial3D\" format=3]\n\n[resource]\nalbedo_color = Color(0.2, 0.4, 0.6, 1)\n";

#[test]
fn added_texture_folders_reach_the_material_list_and_plugins() {
    let (settings, extra) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::create_dir_all(extra.path().join("metal")).unwrap();
    image::RgbaImage::new(2, 2).save(extra.path().join("metal/plate.png")).unwrap();
    // Only the Godot plugin lists `.tres` materials, so this one shows the plugin read the folder.
    std::fs::write(extra.path().join("painted.tres"), PAINTED).unwrap();

    let map = concat!(env!("CARGO_MANIFEST_DIR"), "/../../sdks/surfsup/samples/surf_mapcraft.mcmap");
    let options = Options { settings_dir: Some(settings.path().to_path_buf()), open: Some(map.into()), ..Options::default() };
    let mut h = Harness::builder().with_size(egui::Vec2::new(1600.0, 1000.0)).wgpu().build_eframe(|cc| MapCraftApp::new(cc, options));
    let names = listed(&mut h);
    assert!(names.iter().any(|n| n == "ramps-orange1") && !names.iter().any(|n| n == "metal/plate"), "{names:?}");

    h.state_mut().run_command_for_test("file.sdk_settings");
    h.run_steps(2);
    h.get_by_label("Texture folders").focus();
    h.run_steps(1);
    h.get_by_label("Texture folders").type_text(&extra.path().to_string_lossy());
    h.run_steps(1);
    // Another panel has a disabled Add button too.
    h.get_all_by_label("Add").find(|n| !n.accesskit_node().is_disabled()).unwrap().click();
    h.run_steps(2);
    h.get_by_label("Apply").click();
    let names = listed(&mut h);
    for name in ["ramps-orange1", "metal/plate", "painted"] {
        assert!(names.iter().any(|n| n == name), "{name} is missing from {names:?}");
    }

    h.get_by_label("Remove").click();
    h.run_steps(2);
    h.get_by_label("Apply").click();
    let names = listed(&mut h);
    assert!(!names.iter().any(|n| n == "metal/plate" || n == "painted"), "{names:?}");
}

#[test]
fn a_materials_maps_travel_with_the_map() {
    use mapcraft_geom::{Aabb, DVec3};
    let settings = tempfile::tempdir().unwrap();
    let custom = settings.path().join("data/custom/surfsup");
    std::fs::create_dir_all(&custom).unwrap();
    let picture = |name: &str, pixel: image::Rgba<u8>| {
        let path = custom.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        image::RgbaImage::from_pixel(2, 2, pixel).save(path).unwrap();
    };
    picture("wall.png", image::Rgba([200, 30, 30, 255]));
    picture("wall_normal.png", image::Rgba([128, 128, 255, 255]));
    // One picture holding occlusion, roughness and metallic in its red, green and blue, as games pack them.
    picture("wall_orm.png", image::Rgba([10, 20, 30, 255]));
    // A folder that names its maps `<material>_albedo.png` finds them the same way, under the bare name.
    picture("floor_albedo.png", image::Rgba([30, 200, 30, 255]));
    picture("floor_normal.png", image::Rgba([128, 200, 255, 255]));
    // Same file names in two folders still pack as two materials.
    picture("metal/plate.png", image::Rgba([90, 90, 90, 255]));
    picture("metal/plate_normal.png", image::Rgba([128, 128, 250, 255]));
    picture("wood/plate.png", image::Rgba([140, 90, 40, 255]));
    picture("wood/plate_normal.png", image::Rgba([128, 128, 240, 255]));

    let map = concat!(env!("CARGO_MANIFEST_DIR"), "/../../sdks/surfsup/samples/surf_mapcraft.mcmap");
    let options = Options { settings_dir: Some(settings.path().to_path_buf()), open: Some(map.into()), ..Options::default() };
    let mut h = Harness::builder().with_size(egui::Vec2::new(1600.0, 1000.0)).wgpu().build_eframe(|cc| MapCraftApp::new(cc, options));
    listed(&mut h);
    let s = h.state_mut().session_for_test();
    for (i, material) in ["wall", "floor_albedo", "metal/plate", "wood/plate"].into_iter().enumerate() {
        let x = 64.0 * i as f64;
        let brush = s.create_brush(Aabb::new(DVec3::new(x, 0.0, 0.0), DVec3::new(x + 64.0, 64.0, 64.0))).unwrap();
        s.face_selection = s.map.get(brush).unwrap().kind.brush().unwrap().faces.iter().map(|f| (brush, f.id)).collect();
        s.edit_faces("paint", None, |face| face.material = material.into()).unwrap();
    }
    h.run_steps(2);
    let entries = h.state().asset_entries_for_test().unwrap();
    let names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
    for want in [
        "materials/wall.json",
        "textures/wall_albedo.png",
        "textures/wall_normal.png",
        "textures/wall_roughness.png",
        "textures/wall_metallic.png",
        "materials/floor_albedo.json",
        "textures/floor_albedo_albedo.png",
        "textures/floor_albedo_normal.png",
        "textures/metal/plate_normal.png",
        "textures/wood/plate_normal.png",
    ] {
        assert!(names.contains(&want), "{want} is missing from {names:?}");
    }
    let descriptor = entries.iter().find(|(n, _)| n == "materials/wall.json").unwrap();
    let read = mapcraft_format::pack_material::Material::read(&descriptor.1).unwrap();
    assert!(read.textures.values().any(|t| t.ends_with("wall_normal.png")), "{read:?}");

    let pixel = |name: &str| -> [u8; 4] {
        let (_, bytes) = entries.iter().find(|(n, _)| n == name).unwrap();
        image::load_from_memory(bytes).unwrap().to_rgba8().get_pixel(0, 0).0
    };
    assert_eq!((pixel("textures/metal/plate_albedo.png")[0], pixel("textures/wood/plate_albedo.png")[0]), (90, 140));
    // The game reads roughness and metallic from red, so the combined picture is split into two.
    let red = |name: &str| pixel(name)[0];
    assert_eq!((red("textures/wall_roughness.png"), red("textures/wall_metallic.png")), (20, 30), "the orm picture's green and blue survive as red");

    // And what was gathered is a pack the game will take.
    mapcraft_format::pack::save(&settings.path().join("wall.mcpck"), &entries).unwrap();
}

#[test]
fn imported_pictures_stay_with_their_sdk() {
    let settings = tempfile::tempdir().unwrap();
    for (sdk, name) in [("surfsup", "kept"), ("quake", "quake_only")] {
        let dir = settings.path().join("data/materials").join(sdk);
        std::fs::create_dir_all(&dir).unwrap();
        image::RgbaImage::new(2, 2).save(dir.join(format!("{name}.png"))).unwrap();
    }

    let map = concat!(env!("CARGO_MANIFEST_DIR"), "/../../sdks/surfsup/samples/surf_mapcraft.mcmap");
    let options = Options { settings_dir: Some(settings.path().to_path_buf()), open: Some(map.into()), ..Options::default() };
    let mut h = Harness::builder().with_size(egui::Vec2::new(1600.0, 1000.0)).wgpu().build_eframe(|cc| MapCraftApp::new(cc, options));
    let names = listed(&mut h);
    assert!(names.iter().any(|n| n == "kept") && !names.iter().any(|n| n == "quake_only"), "{names:?}");
}
