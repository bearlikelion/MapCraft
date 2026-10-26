//! SDK Manager > Create > Import from TrenchBroom / func_godot, driven offscreen.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use mapcraft_ui::{MapCraftApp, Options};

#[test]
fn imports_a_trenchbroom_game_as_an_sdk() {
    let settings = tempfile::tempdir().unwrap();
    let game = settings.path().join("TrenchBroom/games/UiGame");
    std::fs::create_dir_all(&game).unwrap();
    std::fs::write(
        game.join("GameConfig.cfg"),
        r#"{ "version": 9, "name": "UI Test Game", "materials": { "root": "textures" }, "entities": { "definitions": [ "ui.fgd" ] } }"#,
    )
    .unwrap();
    std::fs::write(game.join("ui.fgd"), "@PointClass = info_player_start : \"Spawn\" []").unwrap();

    let options = Options { settings_dir: Some(settings.path().to_path_buf()), ..Options::default() };
    let mut h = Harness::builder().with_size(egui::Vec2::new(1600.0, 1000.0)).wgpu().build_eframe(|cc| MapCraftApp::new(cc, options));
    h.run_steps(3);
    h.state_mut().run_command_for_test("file.sdk_manager");
    h.run_steps(2);
    h.get_by_label("Create").click();
    h.run_steps(2);
    h.get_by_label("Folder").focus();
    h.run_steps(1);
    h.get_by_label("Folder").type_text(&game.to_string_lossy());
    h.run_steps(1);
    h.get_by_label("Import SDK").click();
    for _ in 0..200 {
        h.run_steps(1);
        if h.state().sdk_ids_for_test().iter().any(|id| id == "ui-test-game") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(h.state().sdk_ids_for_test().first().map(String::as_str), Some("ui-test-game"), "the import lands in the user SDK folder");
    assert!(settings.path().join("data/sdks/ui-test-game/fgd/ui.fgd").is_file());
    h.run_steps(2);
    assert!(h.query_by_label_contains("Imported UI Test Game").is_some(), "the result shows in the SDK Manager");

    h.get_by_label("Create").click();
    h.run_steps(2);
    h.get_by_label("Folder").focus();
    h.run_steps(1);
    h.get_by_label("Folder").type_text(&settings.path().to_string_lossy());
    h.run_steps(1);
    h.get_by_label("Import SDK").click();
    for _ in 0..200 {
        h.run_steps(1);
        if h.query_by_label_contains("holds neither").is_some() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("a folder that is neither a TrenchBroom game nor a Godot project is refused with a message");
}
