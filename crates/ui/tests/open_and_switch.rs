//! Switching the map's SDK and dropping files on the window.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use mapcraft_geom::{Aabb, DVec3};
use mapcraft_ui::{MapCraftApp, Options};

#[derive(Debug)]
struct Dropped(PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}

fn editor() -> (Harness<'static, MapCraftApp>, tempfile::TempDir) {
    let settings = tempfile::tempdir().unwrap();
    let options = Options { settings_dir: Some(settings.path().to_path_buf()), ..Options::default() };
    let mut h = Harness::builder().with_size(egui::Vec2::new(1600.0, 1000.0)).wgpu().build_eframe(|cc| MapCraftApp::new(cc, options));
    h.run_steps(3);
    (h, settings)
}

fn drop_file(h: &mut Harness<MapCraftApp>, path: &Path) {
    h.input_mut().dropped_files.push(Arc::new(Dropped(path.to_path_buf())));
    h.run_steps(3);
}

const CUBE: &str = r#"{
"classname" "worldspawn"
{
( 0 64 64 ) ( 0 0 64 ) ( 0 0 0 ) wall 0 0 0 1 1
( 64 0 64 ) ( 64 64 64 ) ( 64 64 0 ) wall 0 0 0 1 1
( 0 0 64 ) ( 64 0 64 ) ( 64 0 0 ) wall 0 0 0 1 1
( 64 64 64 ) ( 0 64 64 ) ( 0 64 0 ) wall 0 0 0 1 1
( 0 64 0 ) ( 0 0 0 ) ( 64 0 0 ) wall 0 0 0 1 1
( 0 0 64 ) ( 0 64 64 ) ( 64 64 64 ) wall 0 0 0 1 1
}
}
"#;

#[test]
fn switching_the_sdk_reloads_the_project_and_imports_by_drop() {
    let (mut h, settings) = editor();
    let s = h.state_mut().session_for_test();
    s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
    s.save_as(&settings.path().join("room.mcmap")).unwrap();
    assert_eq!(h.state().project_sdk_for_test(), None);

    h.state_mut().run_command_for_test("file.switch_sdk");
    h.run_steps(2);
    h.get_by_label("Quake").click();
    h.run_steps(3);
    assert_eq!(h.state_mut().session_for_test().sdk.as_deref(), Some("quake"));
    assert_eq!(h.state().project_sdk_for_test(), Some("quake"), "the project reloads for the new SDK");
    assert!(h.state_mut().session_for_test().is_dirty(), "the SDK is saved with the map, so the map is unsaved");
    assert_eq!(h.state().node_count_for_test(), 2, "the map keeps its brushes");

    let quake_map = settings.path().join("cube.MAP");
    std::fs::write(&quake_map, CUBE).unwrap();
    drop_file(&mut h, &quake_map);
    assert!(h.state().status_for_test().starts_with("Imported"), "{}", h.state().status_for_test());
    assert!(h.state().node_count_for_test() > 3, "the Quake plugin imports the dropped .map into a new layer");

    h.state_mut().run_command_for_test("file.switch_sdk");
    h.run_steps(2);
    h.get_by_label("None").click();
    h.run_steps(2);
    assert_eq!(h.state_mut().session_for_test().sdk, None);
    assert_eq!(h.state().project_sdk_for_test(), None);
}

#[test]
fn dropping_a_gltf_adds_its_meshes_as_brushes_in_one_undo_step() {
    let (mut h, settings) = editor();
    let mut source = mapcraft_engine::Session::default();
    source.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
    let path = settings.path().join("blockout.glb");
    std::fs::write(&path, mapcraft_render::export_glb(&source.map, &|_| None, &|_| None, 39.37).unwrap()).unwrap();

    let before = h.state().node_count_for_test();
    drop_file(&mut h, &path);
    assert_eq!(h.state().status_for_test(), "Imported blockout.glb");
    assert_eq!(h.state().node_count_for_test(), before + 2, "a group named after the file holding one brush");
    let s = h.state_mut().session_for_test();
    let brush = s.map.nodes().find_map(|(_, n)| n.kind.brush()).unwrap();
    assert!(brush.bounds().unwrap().max.abs_diff_eq(DVec3::splat(64.0), 1e-3));
    assert!(s.map.nodes().any(|(_, n)| matches!(&n.kind, mapcraft_map::NodeKind::Group(g) if g.name == "blockout.glb")));
    s.undo();
    assert_eq!(h.state().node_count_for_test(), before);
}

#[test]
fn dropping_a_map_opens_it_after_asking_to_save() {
    let (mut h, settings) = editor();
    let path = settings.path().join("room.mcmap");
    let s = h.state_mut().session_for_test();
    s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
    s.save_as(&path).unwrap();
    s.create_brush(Aabb::new(DVec3::splat(128.0), DVec3::splat(192.0))).unwrap();
    h.run_steps(1);
    assert_eq!(h.state().node_count_for_test(), 3);

    drop_file(&mut h, &path);
    assert_eq!(h.state().node_count_for_test(), 3, "unsaved changes are kept until the prompt is answered");
    h.get_by_label("Don't Save").click();
    h.run_steps(3);
    assert_eq!(h.state().node_count_for_test(), 2, "the saved map opened");
    assert_eq!(h.state_mut().session_for_test().path.as_deref(), Some(path.as_path()));
}

#[test]
fn dropping_an_unknown_file_leaves_the_map_alone() {
    let (mut h, settings) = editor();
    h.state_mut().session_for_test().create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
    let notes = settings.path().join("notes.txt");
    std::fs::write(&notes, "hello").unwrap();
    drop_file(&mut h, &notes);
    assert_eq!(h.state().node_count_for_test(), 2);
    assert!(h.state().status_for_test().contains("cannot open notes.txt"), "{}", h.state().status_for_test());
    assert!(h.state_mut().session_for_test().is_dirty(), "the map was not replaced");
}

#[test]
fn boolean_groups_switch_in_the_inspector_and_export_only_their_result() {
    use mapcraft_map::{NodeKind, Operation};
    let (mut h, settings) = editor();
    let s = h.state_mut().session_for_test();
    s.create_brush(Aabb::new(DVec3::new(-128.0, -8.0, 0.0), DVec3::new(128.0, 8.0, 128.0))).unwrap();
    let cutter = s.create_brush(Aabb::new(DVec3::new(-32.0, -32.0, 32.0), DVec3::new(32.0, 32.0, 96.0))).unwrap();
    s.save_as(&settings.path().join("wall.mcmap")).unwrap();
    h.state_mut().run_command_for_test("file.switch_sdk");
    h.run_steps(2);
    h.get_by_label("Quake").click();
    h.run_steps(3);

    h.state_mut().session_for_test().selection = std::collections::BTreeSet::from([cutter]);
    h.state_mut().run_command_for_test("tools.boolean_subtract");
    h.run_steps(2);
    let op = |h: &mut Harness<MapCraftApp>| h.state_mut().session_for_test().map.get(cutter).and_then(|n| n.kind.brush()).map(|b| b.operation);
    let s = h.state_mut().session_for_test();
    let group = s.map.get(cutter).and_then(|n| n.parent).unwrap();
    assert!(matches!(&s.map.get(group).unwrap().kind, NodeKind::Group(g) if g.boolean), "Subtract wraps the wall and the cutter");
    assert_eq!(op(&mut h), Some(Operation::Subtract));

    h.get_by_label("Add").click();
    h.run_steps(2);
    assert_eq!(op(&mut h), Some(Operation::Add), "the inspector switches the brush");
    h.get_by_label("Subtract").click();
    h.run_steps(2);
    assert_eq!(op(&mut h), Some(Operation::Subtract));

    let text = String::from_utf8(h.state_mut().export_for_test("map").unwrap()).unwrap();
    let (game, _) = mapcraft_map::mirror::expand(&h.state_mut().session_for_test().map);
    let pieces = game.nodes().filter(|(_, n)| n.kind.brush().is_some()).count();
    assert!(pieces > 2, "the wall is cut into pieces around the hole");
    assert_eq!(text.matches("// brush ").count(), pieces, "the .map holds the pieces and neither the wall nor the cutter");
}
