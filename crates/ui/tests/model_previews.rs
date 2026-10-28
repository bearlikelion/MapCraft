//! Pictures of models in the Models tab.

use std::path::Path;
use std::time::Duration;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use mapcraft_ui::{MapCraftApp, Options};

const BOX: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../godot/test/Assets/Models/box.glb");

fn app(settings: &Path, project: &Path) -> Harness<'static, MapCraftApp> {
    let options = Options {
        settings_dir: Some(settings.to_path_buf()),
        open: Some(concat!(env!("CARGO_MANIFEST_DIR"), "/../../sdks/surfsup/samples/surf_mapcraft.mcmap").into()),
        sdk_vars: vec![("project".into(), project.to_string_lossy().into_owned())],
        ..Options::default()
    };
    let mut h = Harness::builder().with_size(egui::Vec2::new(1600.0, 1000.0)).wgpu().build_eframe(|cc| MapCraftApp::new(cc, options));
    h.state_mut().run_command_for_test("panel.models");
    h
}

/// Runs frames until the Models tab stops picturing models, and returns what it pictured.
fn settled(h: &mut Harness<MapCraftApp>) -> Vec<(String, bool)> {
    let (mut last, mut same) = (Vec::new(), 0);
    for _ in 0..1000 {
        h.run_steps(1);
        std::thread::sleep(Duration::from_millis(20));
        let now = h.state().model_previews_for_test();
        same = if now == last && !now.is_empty() { same + 1 } else { 0 };
        if same == 25 {
            return now;
        }
        last = now;
    }
    panic!("the Models tab never finished picturing models: {last:?}");
}

fn cached(settings: &Path) -> usize {
    std::fs::read_dir(settings.join("data/model-previews/surfsup")).map_or(0, |d| d.count())
}

#[test]
fn the_models_tab_pictures_the_models_on_screen_or_lists_their_names() {
    let (settings, project) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let models = project.path().join("Assets/Models");
    std::fs::create_dir_all(models.join("copies")).unwrap();
    std::fs::copy(BOX, models.join("box.glb")).unwrap();
    std::fs::write(models.join("broken.glb"), b"glTF\x02\0\0\0junk").unwrap();
    for i in 0..150 {
        std::fs::copy(BOX, models.join(format!("copies/c{i:03}.glb"))).unwrap();
    }

    let mut h = app(settings.path(), project.path());
    let pictured = settled(&mut h);
    assert!(pictured.contains(&("Assets/Models/box.glb".into(), true)), "{pictured:?}");
    assert!(pictured.contains(&("Assets/Models/broken.glb".into(), false)), "a broken model gets a placeholder: {pictured:?}");
    assert!(pictured.len() < 100, "only the models on screen are pictured: {}", pictured.len());
    assert_eq!(cached(settings.path()), pictured.len(), "each picture is cached");
    assert!(h.query_by_label("box.glb").is_some() && h.query_by_label("Assets/Models/box.glb").is_none());

    h.get_by_label("Previews").click();
    h.run_steps(2);
    assert!(h.query_by_label("Assets/Models/box.glb").is_some() && h.query_by_label("box.glb").is_none(), "Previews off lists paths");
}

#[test]
fn pictures_come_from_the_cache_until_the_model_changes() {
    let (settings, project) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let model = project.path().join("box.glb");
    std::fs::copy(BOX, &model).unwrap();
    let file = std::fs::File::options().write(true).open(&model).unwrap();
    let then = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    file.set_modified(then).unwrap();
    assert_eq!(settled(&mut app(settings.path(), project.path())), [("box.glb".to_string(), true)]);

    // Same size and time, so only the cache can still picture it.
    std::fs::write(&model, vec![0u8; std::fs::metadata(BOX).unwrap().len() as usize]).unwrap();
    file.set_modified(then).unwrap();
    assert_eq!(settled(&mut app(settings.path(), project.path())), [("box.glb".to_string(), true)], "the cached picture shows");

    file.set_modified(then + Duration::from_secs(1)).unwrap();
    assert_eq!(settled(&mut app(settings.path(), project.path())), [("box.glb".to_string(), false)], "a changed model is loaded again");
}
