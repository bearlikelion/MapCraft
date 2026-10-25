//! Drives the whole editor offscreen: create a brush with the block tool, undo it, place an entity.

use egui::{Event, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use mapcraft_ui::{MapCraftApp, Options};

fn drag(h: &mut Harness<MapCraftApp>, a: Pos2, b: Pos2) {
    h.hover_at(a);
    h.run_steps(1);
    h.event(Event::PointerButton { pos: a, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for i in 1..=6 {
        h.hover_at(a + (b - a) * (i as f32 / 6.0));
        h.run_steps(1);
    }
    h.event(Event::PointerButton { pos: b, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

#[test]
fn block_tool_undo_and_entities() {
    let settings = tempfile::tempdir().unwrap();
    let options = Options { settings_dir: Some(settings.path().to_path_buf()), ..Options::default() };
    let mut h = Harness::builder().with_size(egui::Vec2::new(1600.0, 1000.0)).wgpu().build_eframe(|cc| MapCraftApp::new(cc, options));
    h.run_steps(3);
    assert_eq!(h.state().node_count_for_test(), 1);

    h.get_by_label("Block").click();
    h.run_steps(2);
    drag(&mut h, Pos2::new(960.0, 180.0), Pos2::new(1120.0, 260.0));
    assert_eq!(h.state().node_count_for_test(), 2, "dragging in the Top view makes a brush");

    h.state_mut().run_command_for_test("edit.undo");
    h.run_steps(2);
    assert_eq!(h.state().node_count_for_test(), 1);
    h.state_mut().run_command_for_test("edit.redo");
    h.run_steps(2);
    assert_eq!(h.state().node_count_for_test(), 2);

    h.get_by_label("Entity").click();
    h.run_steps(2);
    let p = Pos2::new(1200.0, 400.0);
    h.hover_at(p);
    h.run_steps(1);
    h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    assert_eq!(h.state().node_count_for_test(), 3, "clicking with the entity tool places an entity");
}
