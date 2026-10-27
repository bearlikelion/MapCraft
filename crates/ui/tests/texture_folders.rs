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
    let (settings, project, extra) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::create_dir_all(project.path().join("Assets/Textures")).unwrap();
    image::RgbaImage::new(2, 2).save(project.path().join("Assets/Textures/wall.png")).unwrap();
    std::fs::create_dir_all(extra.path().join("metal")).unwrap();
    image::RgbaImage::new(2, 2).save(extra.path().join("metal/plate.png")).unwrap();
    // Only the Godot plugin lists `.tres` materials, so this one shows the plugin read the folder.
    std::fs::write(extra.path().join("painted.tres"), PAINTED).unwrap();

    let map = concat!(env!("CARGO_MANIFEST_DIR"), "/../../sdks/surfsup/samples/surf_mapcraft.mcmap");
    let options = Options {
        settings_dir: Some(settings.path().to_path_buf()),
        open: Some(map.into()),
        sdk_vars: vec![("project".into(), project.path().to_string_lossy().into_owned())],
        ..Options::default()
    };
    let mut h = Harness::builder().with_size(egui::Vec2::new(1600.0, 1000.0)).wgpu().build_eframe(|cc| MapCraftApp::new(cc, options));
    let names = listed(&mut h);
    assert!(names.iter().any(|n| n == "wall") && !names.iter().any(|n| n == "metal/plate"), "{names:?}");

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
    for name in ["wall", "metal/plate", "painted"] {
        assert!(names.iter().any(|n| n == name), "{name} is missing from {names:?}");
    }

    h.get_by_label("Remove").click();
    h.run_steps(2);
    h.get_by_label("Apply").click();
    let names = listed(&mut h);
    assert!(!names.iter().any(|n| n == "metal/plate" || n == "painted"), "{names:?}");
}
