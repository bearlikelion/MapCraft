//! Drives the whole editor offscreen: create a brush with the block tool, undo it, place an entity.

use egui::{Event, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use mapcraft_ui::{MapCraftApp, Options};
use serde_json::{Value, json};

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

fn editor() -> (Harness<'static, MapCraftApp>, tempfile::TempDir) {
    let settings = tempfile::tempdir().unwrap();
    let options = Options { settings_dir: Some(settings.path().to_path_buf()), ..Options::default() };
    let mut h = Harness::builder().with_size(egui::Vec2::new(1600.0, 1000.0)).wgpu().build_eframe(|cc| MapCraftApp::new(cc, options));
    h.run_steps(3);
    (h, settings)
}

#[test]
fn snapping_prefers_corners_then_grid() {
    use mapcraft_geom::{Aabb, DVec3};
    let (mut h, _settings) = editor();
    h.state_mut().session_for_test().create_brush(Aabb::new(DVec3::new(3.0, 5.0, 0.0), DVec3::new(67.0, 69.0, 64.0))).unwrap();
    h.run_steps(2);
    let top = 1;
    let (p, caught) = h.state_mut().snap_for_test(top, DVec3::new(66.6, 68.7, 0.0));
    assert!(caught, "a point beside the off-grid corner snaps to it");
    assert_eq!((p.x, p.y), (67.0, 69.0));
    let (p, caught) = h.state_mut().snap_for_test(top, DVec3::new(200.3, 205.2, 0.0));
    assert!(!caught);
    assert_eq!((p.x, p.y), (208.0, 208.0));
    let (p, caught) = h.state_mut().snap_for_test(top, DVec3::new(64.5, 64.5, 0.0));
    assert!(!caught, "a grid point nearer than the corner wins");
    assert_eq!((p.x, p.y), (64.0, 64.0));
    h.state_mut().run_command_for_test("view.snap_vertices");
    let (p, caught) = h.state_mut().snap_for_test(top, DVec3::new(66.6, 68.7, 0.0));
    assert!(!caught, "corner snapping can be turned off");
    assert_eq!((p.x, p.y), (64.0, 64.0));
    h.hover_at(h.state().screen_for_test(top, DVec3::ZERO));
    h.run_steps(1);
    for _ in 0..4 {
        h.event(Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -120.0),
            modifiers: Modifiers::NONE,
            phase: egui::TouchPhase::Move,
        });
        h.run_steps(1);
    }
    let zoom = h.state().zooms_for_test()[top];
    let step = mapcraft_render::grid_step(16.0, zoom);
    assert!(step > 16.0, "zoomed out to {zoom}, the grid draws coarser");
    let (p, _) = h.state_mut().snap_for_test(top, DVec3::new(step * 3.4, step * 0.6, 0.0));
    assert_eq!((p.x, p.y), (step * 3.0, step), "a zoomed out view snaps to the lines it draws");
}

#[test]
fn space_maximizes_the_hovered_view() {
    let (mut h, _settings) = editor();
    h.hover_at(Pos2::new(1000.0, 200.0));
    h.run_steps(2);
    h.key_press(egui::Key::Space);
    h.run_steps(2);
    assert_eq!(h.state().maximized_for_test(), Some(1), "Space over the Top view maximizes it");
    h.key_press(egui::Key::Space);
    h.run_steps(2);
    assert_eq!(h.state().maximized_for_test(), None, "Space again brings back four views");
}

#[test]
fn zoom_is_linked_across_2d_views() {
    let (mut h, _settings) = editor();
    let top = Pos2::new(1000.0, 300.0);
    h.hover_at(top);
    h.run_steps(1);
    h.event(Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::vec2(0.0, 120.0), modifiers: Modifiers::NONE, phase: egui::TouchPhase::Move });
    h.run_steps(4);
    let z = h.state().zooms_for_test();
    assert!(z[1] != 1.0 && z[1] == z[2] && z[1] == z[3], "every 2D view follows the zoomed one: {z:?}");
    h.state_mut().run_command_for_test("view.link_zoom");
    h.event(Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::vec2(0.0, 120.0), modifiers: Modifiers::NONE, phase: egui::TouchPhase::Move });
    h.run_steps(4);
    let unlinked = h.state().zooms_for_test();
    assert!(unlinked[1] != unlinked[2], "unlinked views zoom on their own: {unlinked:?}");
}

#[test]
fn missing_sdk_paths_open_the_settings() {
    let settings = tempfile::tempdir().unwrap();
    let map = concat!(env!("CARGO_MANIFEST_DIR"), "/../../sdks/surfsup/samples/surf_mapcraft.mcmap");
    let options = Options { settings_dir: Some(settings.path().to_path_buf()), open: Some(map.into()), ..Options::default() };
    let mut h = Harness::builder().with_size(egui::Vec2::new(1600.0, 1000.0)).wgpu().build_eframe(|cc| MapCraftApp::new(cc, options));
    h.run_steps(3);
    assert!(h.state().sdk_settings_open_for_test(), "a SurfsUp map without its project folder asks for it");
}

fn drag_with(h: &mut Harness<MapCraftApp>, a: Pos2, b: Pos2, modifiers: Modifiers) {
    h.event(Event::ModifiersChanged(modifiers));
    h.hover_at(a);
    h.run_steps(1);
    h.event(Event::PointerButton { pos: a, button: PointerButton::Primary, pressed: true, modifiers });
    h.run_steps(1);
    for i in 1..=8 {
        h.hover_at(a + (b - a) * (i as f32 / 8.0));
        h.run_steps(1);
    }
    h.event(Event::PointerButton { pos: b, button: PointerButton::Primary, pressed: false, modifiers });
    h.run_steps(2);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
}

#[test]
fn escape_leaves_the_tool_then_deselects() {
    use mapcraft_geom::{Aabb, DVec3};
    let (mut h, _settings) = editor();
    h.state_mut().session_for_test().create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
    h.state_mut().run_command_for_test("tool.vertex");
    h.run_steps(2);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state_mut().session_for_test().selection.len(), 1, "the first Esc only leaves the Vertex tool");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state_mut().session_for_test().selection.is_empty(), "the next Esc deselects");
}

#[test]
fn ctrl_enter_cycles_the_clip_side_and_enter_cuts() {
    use mapcraft_engine::ClipKeep;
    use mapcraft_geom::{Aabb, DVec3};
    let (mut h, _settings) = editor();
    h.state_mut().session_for_test().create_brush(Aabb::new(DVec3::ZERO, DVec3::new(128.0, 64.0, 64.0))).unwrap();
    h.state_mut().run_command_for_test("tool.clip");
    h.run_steps(2);
    let (a, b) = (h.state().screen_for_test(1, DVec3::new(32.0, -32.0, 0.0)), h.state().screen_for_test(1, DVec3::new(32.0, 96.0, 0.0)));
    click(&mut h, a);
    click(&mut h, b);
    let mut seen = Vec::new();
    for _ in 0..3 {
        h.key_press_modifiers(Modifiers::COMMAND, egui::Key::Enter);
        h.run_steps(2);
        seen.push(h.state().clip_keep_for_test());
    }
    assert_eq!(h.state().clip_points_for_test(), 2, "Ctrl+Enter changes the side without cutting");
    assert!([ClipKeep::Front, ClipKeep::Back, ClipKeep::Both].iter().all(|k| seen.contains(k)), "three presses visit every side: {seen:?}");
    while h.state().clip_keep_for_test() != ClipKeep::Both {
        h.key_press_modifiers(Modifiers::COMMAND, egui::Key::Enter);
        h.run_steps(2);
    }
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(h.state_mut().session_for_test().selected_brushes().len(), 2, "Enter cuts and Both keeps the two halves");
}

fn bounds(h: &mut Harness<MapCraftApp>, id: mapcraft_map::NodeId) -> mapcraft_geom::Aabb {
    h.state_mut().session_for_test().map.get(id).unwrap().kind.brush().unwrap().bounds().unwrap()
}

#[test]
fn extrude_split_and_rotate_by_dragging() {
    use mapcraft_geom::{Aabb, DVec3};
    let (mut h, _settings) = editor();
    let id = h.state_mut().session_for_test().create_brush(Aabb::new(DVec3::ZERO, DVec3::new(64.0, 32.0, 64.0))).unwrap();
    h.run_steps(2);
    let top = 1;
    h.state_mut().run_command_for_test("tool.extrude");
    let from = h.state().screen_for_test(top, DVec3::new(64.0, 16.0, 0.0));
    let to = h.state().screen_for_test(top, DVec3::new(126.0, 16.0, 0.0));
    drag_with(&mut h, from, to, Modifiers::NONE);
    assert_eq!(bounds(&mut h, id).max.x, 128.0, "dragging the +X face extrudes it to the grid");

    let from = h.state().screen_for_test(top, DVec3::new(128.0, 16.0, 0.0));
    let to = h.state().screen_for_test(top, DVec3::new(160.0, 16.0, 0.0));
    drag_with(&mut h, from, to, Modifiers::COMMAND);
    assert_eq!(bounds(&mut h, id).max.x, 128.0, "a Ctrl drag leaves the brush as it was");
    assert_eq!(h.state().node_count_for_test(), 3, "and splits off a new brush");
    h.state_mut().run_command_for_test("edit.undo");
    h.run_steps(2);
    assert_eq!(h.state().node_count_for_test(), 2);

    h.state_mut().session_for_test().selection = std::collections::BTreeSet::from([id]);
    h.state_mut().run_command_for_test("tool.rotate");
    h.run_steps(2);
    let c = bounds(&mut h, id).center();
    let from = h.state().screen_for_test(top, c + DVec3::new(96.0, 0.0, 0.0));
    let to = h.state().screen_for_test(top, c + DVec3::new(0.0, 96.0, 0.0));
    drag_with(&mut h, from, to, Modifiers::NONE);
    let size = bounds(&mut h, id).size();
    assert!((size.x - 32.0).abs() < 1e-6 && (size.y - 128.0).abs() < 1e-6, "a quarter turn swaps the footprint: {size:?}");
}

#[test]
fn escape_cancels_a_face_drag() {
    use mapcraft_geom::{Aabb, DVec3};
    let (mut h, _settings) = editor();
    let id = h.state_mut().session_for_test().create_brush(Aabb::new(DVec3::ZERO, DVec3::new(64.0, 32.0, 64.0))).unwrap();
    h.run_steps(2);
    h.state_mut().run_command_for_test("tool.extrude");
    let from = h.state().screen_for_test(1, DVec3::new(64.0, 16.0, 0.0));
    let to = h.state().screen_for_test(1, DVec3::new(126.0, 16.0, 0.0));
    h.hover_at(from);
    h.run_steps(1);
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for i in 1..=8 {
        h.hover_at(from + (to - from) * (i as f32 / 8.0));
        h.run_steps(1);
    }
    assert_eq!(bounds(&mut h, id).max.x, 128.0, "the face follows the pointer");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(bounds(&mut h, id).max.x, 64.0, "Escape puts it back");
    h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert_eq!(bounds(&mut h, id).max.x, 64.0);
    assert_eq!(h.state_mut().session_for_test().undo_label(), Some("Create brush"), "a cancelled drag leaves nothing to undo");

    drag_with(&mut h, from, from + (to - from) * 0.5, Modifiers::NONE);
    assert_eq!(h.state_mut().session_for_test().undo_label(), Some("Extrude"), "the Extrude tool stays on after Escape");
    h.state_mut().run_command_for_test("edit.undo");
    let back = [from + (to - from) * 0.5, from];
    h.hover_at(from);
    h.run_steps(1);
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for p in back {
        h.hover_at(p);
        h.run_steps(1);
    }
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert_eq!(h.state_mut().session_for_test().undo_label(), Some("Create brush"), "a drag that ends where it began leaves no step");
    assert_eq!(bounds(&mut h, id).max.x, 64.0);

    h.hover_at(from);
    h.run_steps(1);
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.hover_at(to);
    h.run_steps(1);
    assert_eq!(bounds(&mut h, id).max.x, 128.0);
    h.event(Event::WindowFocused(false));
    h.run_steps(2);
    assert_eq!(bounds(&mut h, id).max.x, 64.0, "leaving the window cancels the drag");
    h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

#[test]
fn extrude_grabs_outline_edges_and_seams_in_3d() {
    use mapcraft_geom::{Aabb, DVec3};
    let (mut h, _settings) = editor();
    let a = h.state_mut().session_for_test().create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
    h.run_steps(2);
    h.state_mut().run_command_for_test("tool.extrude");
    let off = egui::Vec2::new(15.0, 0.0);
    let from = h.state().screen_for_test(0, DVec3::new(64.0, 0.0, 32.0)) + off;
    let to = h.state().screen_for_test(0, DVec3::new(128.0, 0.0, 32.0)) + off;
    drag_with(&mut h, from, to, Modifiers::NONE);
    assert!(bounds(&mut h, a).max.x > 64.0, "beside the brush the hidden +X face is grabbed by its outline edge");
    h.state_mut().run_command_for_test("edit.undo");

    let b = h.state_mut().session_for_test().create_brush(Aabb::new(DVec3::new(64.0, 0.0, 0.0), DVec3::new(128.0, 64.0, 64.0))).unwrap();
    h.state_mut().session_for_test().selection = std::collections::BTreeSet::from([a, b]);
    h.run_steps(2);
    let from = h.state().screen_for_test(0, DVec3::new(64.0, 0.0, 32.0));
    let to = h.state().screen_for_test(0, DVec3::new(96.0, 0.0, 32.0));
    drag_with(&mut h, from, to, Modifiers::NONE);
    assert_eq!(bounds(&mut h, a).max.x, 96.0, "the seam between two selected brushes wins over the face under the pointer");
    assert_eq!(bounds(&mut h, b).min.x, 96.0, "and both sides move with it");
}

#[test]
fn extrude_modifiers_slide_and_split() {
    use mapcraft_geom::{Aabb, DVec3};
    let (mut h, _settings) = editor();
    let id = h.state_mut().session_for_test().create_brush(Aabb::new(DVec3::ZERO, DVec3::new(64.0, 32.0, 64.0))).unwrap();
    h.run_steps(2);
    h.state_mut().run_command_for_test("tool.extrude");
    let at = |h: &Harness<MapCraftApp>, x: f64, y: f64| h.state().screen_for_test(1, DVec3::new(x, y, 0.0));
    let (from, to) = (at(&h, 64.0, 16.0), at(&h, 96.0, 48.0));
    drag_with(&mut h, from, to, Modifiers::ALT);
    assert_eq!(h.state_mut().session_for_test().undo_label(), Some("Slide"));
    let corners = h.state_mut().session_for_test().map.get(id).unwrap().kind.brush().unwrap().vertices().unwrap();
    let moved: Vec<f64> = corners.iter().filter(|v| (v.x - 96.0).abs() < 1e-6).map(|v| v.y).collect();
    assert!(moved.len() == 4 && moved.iter().all(|y| *y == 32.0 || *y == 64.0), "Alt slides the face in the view plane: {moved:?}");
    h.state_mut().run_command_for_test("edit.undo");

    let (from, to) = (at(&h, 64.0, 16.0), at(&h, 32.0, 16.0));
    drag_with(&mut h, from, to, Modifiers::COMMAND);
    assert_eq!(h.state_mut().session_for_test().selection.len(), 2, "an inward split keeps both parts selected");
    assert_eq!(h.state().node_count_for_test(), 3);
    h.state_mut().run_command_for_test("edit.undo");

    let (from, to) = (at(&h, 64.0, 16.0), at(&h, -40.0, 16.0));
    drag_with(&mut h, from, to, Modifiers::COMMAND);
    assert_eq!(h.state().node_count_for_test(), 2);
    assert_eq!(bounds(&mut h, id).max.x, 64.0);
    assert_eq!(h.state_mut().session_for_test().undo_label(), Some("Create brush"), "a split past the whole brush leaves no step");
}

#[test]
fn blocks_are_drawn_in_the_3d_view() {
    use mapcraft_geom::DVec3;
    let (mut h, _settings) = editor();
    h.hover_at(Pos2::new(400.0, 300.0));
    h.run_steps(1);
    h.key_press(egui::Key::B);
    h.run_steps(2);
    let from = h.state().screen_for_test(0, DVec3::ZERO);
    let to = h.state().screen_for_test(0, DVec3::new(128.0, 96.0, 0.0));
    drag_with(&mut h, from, to, Modifiers::NONE);
    assert_eq!(h.state().node_count_for_test(), 2, "B picks the block tool and a drag on the ground makes a brush");
    let id = *h.state_mut().session_for_test().selection.iter().next().unwrap();
    let b = bounds(&mut h, id);
    assert_eq!((b.min, b.max), (DVec3::ZERO, DVec3::new(128.0, 96.0, 64.0)));

    let from = h.state().screen_for_test(0, DVec3::new(16.0, 16.0, 64.0));
    let to = h.state().screen_for_test(0, DVec3::new(48.0, 48.0, 64.0));
    drag_with(&mut h, from, to, Modifiers::NONE);
    let id = *h.state_mut().session_for_test().selection.iter().next().unwrap();
    let b = bounds(&mut h, id);
    assert_eq!((b.min, b.max), (DVec3::new(16.0, 16.0, 64.0), DVec3::new(48.0, 48.0, 128.0)), "drawing on a brush stacks on its face");
}

#[test]
fn scene_tree_rows_drag_to_reorder() {
    use mapcraft_geom::{Aabb, DVec3};
    let (mut h, _settings) = editor();
    let ids: Vec<_> = (0..3)
        .map(|i| h.state_mut().session_for_test().create_brush(Aabb::new(DVec3::splat(f64::from(i) * 64.0), DVec3::splat(f64::from(i) * 64.0 + 32.0))).unwrap())
        .collect();
    h.state_mut().session_for_test().selection.clear();
    h.run_steps(3);
    let rows: Vec<egui::Rect> = h.get_all_by_label("Brush").map(|n| n.rect()).collect();
    assert_eq!(rows.len(), 3);
    drag_with(&mut h, rows[0].center(), egui::pos2(rows[2].center().x, rows[2].bottom() - 2.0), Modifiers::NONE);
    let world = h.state_mut().session_for_test().map.get(mapcraft_map::ROOT).unwrap().children.clone();
    let order: Vec<_> = world.iter().filter(|c| ids.contains(c)).copied().collect();
    assert_eq!(order, [ids[1], ids[2], ids[0]], "dropping on the lower edge of the last row puts the brush after it");
}

#[test]
fn shift_click_on_a_tree_arrow_opens_and_closes_everything_below() {
    use mapcraft_geom::{Aabb, DVec3};
    let (mut h, _settings) = editor();
    let s = h.state_mut().session_for_test();
    s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
    s.group_selection().unwrap();
    s.group_selection().unwrap();
    h.run_steps(3);
    let rows = |h: &Harness<MapCraftApp>, label: &str| -> Vec<egui::Rect> {
        h.query_all_by_label(label).map(|n| n.rect()).filter(|r| r.right() < 320.0 && r.width() > 200.0).collect()
    };
    let arrow = |h: &Harness<MapCraftApp>| {
        let outer = rows(h, "Group")[0];
        egui::pos2(outer.left() + 2.0 + 14.0 + 8.0, outer.center().y)
    };
    let click_with = |h: &mut Harness<MapCraftApp>, at: Pos2, modifiers: Modifiers| {
        h.event(Event::ModifiersChanged(modifiers));
        h.hover_at(at);
        h.run_steps(1);
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers });
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers });
        h.run_steps(2);
        h.event(Event::ModifiersChanged(Modifiers::NONE));
        h.run_steps(1);
    };
    assert_eq!(rows(&h, "Group").len(), 1, "the outer group starts closed");
    let at = arrow(&h);
    click_with(&mut h, at, Modifiers::SHIFT);
    assert_eq!((rows(&h, "Group").len(), rows(&h, "Brush").len()), (2, 1), "Shift opens every group below");
    click_with(&mut h, at, Modifiers::SHIFT);
    assert_eq!((rows(&h, "Group").len(), rows(&h, "Brush").len()), (1, 0), "Shift closes them all");
    click_with(&mut h, at, Modifiers::NONE);
    assert_eq!((rows(&h, "Group").len(), rows(&h, "Brush").len()), (2, 0), "a plain click opens only that group");
}

#[test]
fn scene_tree_scrolls_to_the_selection() {
    use mapcraft_geom::{Aabb, DVec3};
    let (mut h, _settings) = editor();
    let s = h.state_mut().session_for_test();
    let last =
        (0..400).map(|i| s.create_brush(Aabb::new(DVec3::splat(f64::from(i) * 64.0), DVec3::splat(f64::from(i) * 64.0 + 32.0))).unwrap()).last().unwrap();
    s.selection = std::collections::BTreeSet::from([last]);
    s.group_selection().unwrap();
    h.run_steps(3);
    let row = h.get_all_by_label("Group").map(|n| n.rect()).find(|r| r.right() < 320.0 && r.width() > 200.0);
    assert!(row.is_some_and(|r| r.bottom() < 1000.0), "the selected group's row is scrolled into view");
}

#[test]
fn scene_tree_search_filters_and_steps_through_matches() {
    use mapcraft_geom::{Aabb, DVec3};
    use std::collections::BTreeSet;
    fn rows(h: &Harness<MapCraftApp>, label: &str) -> usize {
        h.query_all_by_label(label).filter(|n| n.rect().right() < 320.0 && n.rect().width() > 200.0).count()
    }
    let (mut h, _settings) = editor();
    let s = h.state_mut().session_for_test();
    s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(32.0))).unwrap();
    s.tex.material = "tools/toolstrigger".into();
    let a = s.create_brush(Aabb::new(DVec3::splat(64.0), DVec3::splat(96.0))).unwrap();
    let b = s.create_brush(Aabb::new(DVec3::splat(128.0), DVec3::splat(160.0))).unwrap();
    s.create_point_entity("info_teleport_destination", DVec3::splat(200.0), vec![("targetname".into(), "stage2".into())]).unwrap();
    s.selection.clear();
    h.run_steps(3);
    assert_eq!(rows(&h, "Brush"), 3);

    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, egui::Key::F);
    h.run_steps(2);
    h.event(Event::Text("TRIGGER".into()));
    h.run_steps(2);
    assert_eq!(rows(&h, "Brush"), 0, "the brush without the material is filtered out");
    assert_eq!(rows(&h, "Brush (tools/toolstrigger)"), 2, "brushes match by material, ignoring case");
    assert_eq!(rows(&h, "stage2 (info_teleport_destination)"), 0);

    let selected = |h: &mut Harness<MapCraftApp>| h.state_mut().session_for_test().selection.clone();
    let before = h.state().screen_for_test(0, DVec3::splat(80.0));
    for (i, want) in [a, b, a].into_iter().enumerate() {
        h.key_press(egui::Key::Enter);
        h.run_steps(2);
        assert_eq!(selected(&mut h), BTreeSet::from([want]), "Enter steps through the matches in tree order");
        if i == 0 {
            assert_ne!(h.state().screen_for_test(0, DVec3::splat(80.0)), before, "Enter focuses the views on the match");
        }
    }

    h.key_press(egui::Key::ArrowDown);
    h.run_steps(2);
    h.event(Event::Text("x".into()));
    h.run_steps(2);
    assert_eq!(rows(&h, "Brush (tools/toolstrigger)"), 2, "Down leaves the box and keeps the filter");

    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, egui::Key::F);
    h.run_steps(2);
    h.event(Event::Text("stage".into()));
    h.run_steps(2);
    assert_eq!(rows(&h, "stage2 (info_teleport_destination)"), 1, "the shortcut selects the old text, and entities match by targetname");
    assert_eq!(rows(&h, "World"), 1, "ancestors of matches stay in the tree");
    assert_eq!(rows(&h, "Brush (tools/toolstrigger)"), 0);
    h.event(Event::Text("zzz".into()));
    h.run_steps(2);
    assert_eq!(h.query_all_by_label("Nothing matches").count(), 1);

    h.state_mut().run_command_for_test("tool.face");
    h.run_steps(2);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(rows(&h, "Brush"), 3, "Escape clears the search");
    assert_eq!(h.state_mut().session_for_test().face_selection.len(), 6, "without the editor's Escape, which would drop the Face tool");
}

#[test]
fn tag_eye_hides_and_shows_what_it_tags() {
    use mapcraft_geom::{Aabb, DVec3};
    let (mut h, _settings) = editor();
    let b = h.state_mut().session_for_test().create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
    h.state_mut().run_command_for_test("panel.tags");
    h.run_steps(2);
    h.get_by_label("Tag selection").click();
    h.run_steps(2);
    h.get_by_label("Hide").click();
    h.run_steps(2);
    let s = h.state_mut().session_for_test();
    assert!(!s.map.visible_nodes().contains(&b), "hiding the tag hides the tagged brush");
    assert_eq!(s.undo_label(), Some("Hide tag"));
    h.get_by_label("Show").click();
    h.run_steps(2);
    let s = h.state_mut().session_for_test();
    assert!(s.map.visible_nodes().contains(&b), "showing the tag brings the brush back");
    assert_eq!(s.undo_label(), Some("Show tag"));
}

#[test]
fn face_tool_takes_over_the_selection() {
    use mapcraft_geom::{Aabb, DVec3};
    let (mut h, _settings) = editor();
    let b = h.state_mut().session_for_test().create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
    h.run_steps(2);
    h.state_mut().run_command_for_test("tool.face");
    let s = h.state_mut().session_for_test();
    assert!(s.selection.is_empty(), "the brush no longer tints its faces");
    assert_eq!(s.face_selection.len(), 6, "its faces are picked instead");
    let face = s.face_selection.iter().next().copied().unwrap();
    s.face_selection = std::collections::BTreeSet::from([face]);
    s.tex.material = "ramps/orange".into();
    h.state_mut().run_command_for_test("tools.apply_material");
    let b = h.state_mut().session_for_test().map.get(b).unwrap().kind.brush().unwrap().clone();
    assert_eq!(b.faces.iter().filter(|f| f.material == "ramps/orange").count(), 1, "applying a material paints the picked face only");
}

#[test]
fn clip_tool_cuts_the_selected_brush() {
    use mapcraft_geom::{Aabb, DVec3};
    let (mut h, _settings) = editor();
    let id = h.state_mut().session_for_test().create_brush(Aabb::new(DVec3::ZERO, DVec3::new(128.0, 64.0, 64.0))).unwrap();
    h.run_steps(2);
    h.state_mut().run_command_for_test("tool.clip");
    let top = 1;
    for p in [DVec3::new(96.0, -32.0, 0.0), DVec3::new(96.0, 96.0, 0.0)] {
        let at = h.state().screen_for_test(top, p);
        h.hover_at(at);
        h.run_steps(1);
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(2);
    }
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert!(h.state_mut().session_for_test().map.get(id).is_none(), "the clipped brush is replaced");
    assert_eq!(h.state().node_count_for_test(), 2, "by its bigger part");
    let kept = h.state_mut().session_for_test().selected_brushes()[0];
    assert_eq!(bounds(&mut h, kept).max.x, 96.0);
}

#[test]
fn clip_points_move_and_escape_clears_them() {
    use mapcraft_geom::{Aabb, DVec3};
    let (mut h, _settings) = editor();
    h.state_mut().session_for_test().create_brush(Aabb::new(DVec3::ZERO, DVec3::new(128.0, 64.0, 64.0))).unwrap();
    h.run_steps(2);
    h.state_mut().run_command_for_test("tool.clip");
    let top = 1;
    let at = |h: &Harness<MapCraftApp>, x: f64, y: f64| h.state().screen_for_test(top, DVec3::new(x, y, 0.0));
    let (a, b, moved, c) = (at(&h, 96.0, -32.0), at(&h, 96.0, 96.0), at(&h, 0.0, -32.0), at(&h, 64.0, -32.0));
    click(&mut h, a);
    click(&mut h, b);
    drag_with(&mut h, a, moved, Modifiers::NONE);
    assert_eq!(h.state().clip_points_for_test(), 2, "dragging a point moves it instead of starting a new line");
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    let kept = h.state_mut().session_for_test().selected_brushes()[0];
    assert!((bounds(&mut h, kept).min.x - 24.0).abs() < 1e-6, "the slanted cut keeps the bigger part");

    click(&mut h, c);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().clip_points_for_test(), 0, "Esc clears the points");
    click(&mut h, c);
    assert_eq!(h.state().clip_points_for_test(), 1, "and the Clip tool stays active");
}

#[test]
fn clip_lines_drag_in_2d_and_on_faces_in_3d() {
    use mapcraft_geom::{Aabb, DVec3};
    for (view, a, b) in [(1, DVec3::new(64.0, -32.0, 0.0), DVec3::new(64.0, 96.0, 0.0)), (0, DVec3::new(64.0, 16.0, 64.0), DVec3::new(64.0, 48.0, 64.0))] {
        let (mut h, _settings) = editor();
        h.state_mut().session_for_test().create_brush(Aabb::new(DVec3::ZERO, DVec3::new(128.0, 64.0, 64.0))).unwrap();
        h.run_steps(2);
        h.state_mut().run_command_for_test("tool.clip");
        let (from, to) = (h.state().screen_for_test(view, a), h.state().screen_for_test(view, b));
        drag_with(&mut h, from, to, Modifiers::NONE);
        h.get_by_label("Both").click();
        h.run_steps(2);
        h.key_press(egui::Key::Enter);
        h.run_steps(2);
        let s = h.state_mut().session_for_test();
        let mut xs: Vec<(f64, f64)> = s.map.nodes().filter_map(|(_, n)| n.kind.brush()?.bounds().ok()).map(|b| (b.min.x, b.max.x)).collect();
        xs.sort_by(|p, q| p.0.total_cmp(&q.0));
        assert_eq!(xs, [(0.0, 64.0), (64.0, 128.0)], "a line dragged in view {view} cuts the brush at x = 64");
    }
}

#[test]
fn displacements_sculpt_in_the_3d_view() {
    use mapcraft_geom::{Aabb, DVec3};
    let (mut h, _settings) = editor();
    let id = h.state_mut().session_for_test().create_brush(Aabb::new(DVec3::new(-256.0, -256.0, -16.0), DVec3::new(256.0, 256.0, 0.0))).unwrap();
    h.run_steps(2);
    h.state_mut().run_command_for_test("tool.face");
    let s = h.state_mut().session_for_test();
    let top = s.map.get(id).unwrap().kind.brush().unwrap().faces.iter().find(|f| f.plane().unwrap().normal.z > 0.9).unwrap().id;
    s.face_selection = std::collections::BTreeSet::from([(id, top)]);
    s.displace_faces(3).unwrap();
    h.run_steps(2);
    h.key_press(egui::Key::D);
    h.run_steps(2);
    let (from, to) = (h.state().screen_for_test(0, DVec3::new(-64.0, 0.0, 0.0)), h.state().screen_for_test(0, DVec3::new(64.0, 0.0, 0.0)));
    drag_with(&mut h, from, to, Modifiers::NONE);
    let s = h.state_mut().session_for_test();
    let b = s.map.get(id).unwrap().kind.brush().unwrap().clone();
    let peak = b.displaced(&b.solid().unwrap()).into_iter().flat_map(|(_, v)| v).map(|p| p.z).fold(f64::MIN, f64::max);
    assert!(peak > 1.0, "dragging with the Sculpt tool raises the surface: {peak}");
    s.undo();
    let b = s.map.get(id).unwrap().kind.brush().unwrap().clone();
    assert!(b.displaced(&b.solid().unwrap()).into_iter().flat_map(|(_, v)| v).all(|p| p.z.abs() < 1e-9), "one undo takes back the stroke");

    let top_view = 1;
    let (from, to) = (h.state().screen_for_test(top_view, DVec3::new(-64.0, 0.0, 0.0)), h.state().screen_for_test(top_view, DVec3::new(64.0, 0.0, 0.0)));
    drag_with(&mut h, from, to, Modifiers::NONE);
    let b = h.state_mut().session_for_test().map.get(id).unwrap().kind.brush().unwrap().clone();
    let peak = b.displaced(&b.solid().unwrap()).into_iter().flat_map(|(_, v)| v).map(|p| p.z).fold(f64::MIN, f64::max);
    assert!(peak > 1.0, "sculpting works in the 2D views too: {peak}");
}

#[test]
fn paint_mode_blends_the_current_material() {
    use mapcraft_geom::{Aabb, DVec3};
    let (mut h, _settings) = editor();
    let s = h.state_mut().session_for_test();
    s.tex.material = "nature/rock031".into();
    let id = s.create_brush(Aabb::new(DVec3::new(-256.0, -256.0, -16.0), DVec3::new(256.0, 256.0, 0.0))).unwrap();
    let top = s.map.get(id).unwrap().kind.brush().unwrap().faces.iter().find(|f| f.plane().unwrap().normal.z > 0.9).unwrap().id;
    s.face_selection = std::collections::BTreeSet::from([(id, top)]);
    s.displace_faces(3).unwrap();
    s.face_selection.clear();
    s.tex.material = "nature/grass01".into();
    h.run_steps(2);
    h.key_press(egui::Key::D);
    h.run_steps(1);
    h.key_press(egui::Key::Num6);
    h.run_steps(2);
    assert!(h.query_by_value("opacity 100%").is_some(), "Paint shows its opacity");
    h.get_by_label("nature/grass01").hover();
    h.run_steps(4);
    assert!(h.query_by_label_contains("Ctrl paints the face's own material back").is_some(), "the material chip explains painting");

    let face = |h: &mut Harness<MapCraftApp>| {
        let b = h.state_mut().session_for_test().map.get(id).unwrap().kind.brush().unwrap().clone();
        b.faces.into_iter().find(|f| f.id == top).unwrap().disp.unwrap()
    };
    let (from, to) = (h.state().screen_for_test(0, DVec3::new(-64.0, 0.0, 0.0)), h.state().screen_for_test(0, DVec3::new(64.0, 0.0, 0.0)));
    drag_with(&mut h, from, to, Modifiers::NONE);
    let d = face(&mut h);
    assert_eq!(d.blend_material.as_deref(), Some("nature/grass01"), "painting makes the current material the second layer");
    assert!(d.blend.iter().any(|w| *w > 0.5), "and raises its share under the stroke: {:?}", d.blend);
    assert!(d.heights.iter().all(|z| *z == 0.0), "painting never moves a vertex");

    h.hover_at(h.state().screen_for_test(0, DVec3::new(0.0, 0.0, 0.0)));
    h.run_steps(2);
    assert!(h.state().hint_for_test().starts_with("rock031 + grass01: "), "the status bar reads the blend under the pointer: {}", h.state().hint_for_test());
    h.state_mut().session_for_test().tex.material = "other".into();
    let at = h.state().screen_for_test(0, DVec3::ZERO);
    h.event(Event::ModifiersChanged(Modifiers::ALT));
    h.hover_at(at);
    h.run_steps(1);
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::ALT });
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::ALT });
    h.run_steps(2);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    assert_eq!(h.state_mut().session_for_test().tex.material, "nature/grass01", "Alt+click picks the layer that shows most");

    h.state_mut().session_for_test().undo();
    let d = face(&mut h);
    assert!(d.blend_material.is_none() && d.blend.is_empty(), "one undo takes back the stroke and the layer");

    h.event(Event::ModifiersChanged(Modifiers::SHIFT));
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::SHIFT });
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::SHIFT });
    h.run_steps(2);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    assert_eq!(face(&mut h).blend_material.as_deref(), Some("nature/grass01"), "Shift+click paints forward; only Ctrl paints back");
    h.state_mut().session_for_test().undo();

    h.state_mut().session_for_test().tex.material = "nature/rock031".into();
    drag_with(&mut h, from, to, Modifiers::NONE);
    assert!(h.state().status_for_test().contains("pick a second material"), "{}", h.state().status_for_test());
}

#[test]
fn inspector_rows_anti_tile_brushes_and_blend_displacements() {
    use egui::accesskit::Role;
    use mapcraft_geom::{Aabb, DVec3};
    let (mut h, _settings) = editor();
    let id = h.state_mut().session_for_test().create_brush(Aabb::new(DVec3::new(-256.0, -256.0, -16.0), DVec3::new(256.0, 256.0, 0.0))).unwrap();
    h.run_steps(2);
    let faces = |h: &mut Harness<MapCraftApp>| h.state_mut().session_for_test().map.get(id).unwrap().kind.brush().unwrap().faces.clone();
    h.get_by_role_and_label(Role::ComboBox, "Anti-tiling").click_accesskit();
    h.run_steps(2);
    h.get_by_label("Variation").click_accesskit();
    h.run_steps(2);
    assert!(faces(&mut h).iter().all(|f| f.anti_tile() == mapcraft_map::AntiTile::Macro), "the Brushes row writes every face");
    // An accesskit click leaves the list open.
    h.get_by_label("Hex tiling").click_accesskit();
    h.run_steps(2);
    assert!(faces(&mut h).iter().all(|f| f.anti_tile() == mapcraft_map::AntiTile::Hex), "and offers hex tiling");
    assert!(h.query_by_label_contains("Pixel textures").is_none(), "smooth textures keep hex tiling");

    h.state_mut().run_command_for_test("tool.face");
    let s = h.state_mut().session_for_test();
    let top = s.map.get(id).unwrap().kind.brush().unwrap().faces.iter().find(|f| f.plane().unwrap().normal.z > 0.9).unwrap().id;
    s.face_selection = std::collections::BTreeSet::from([(id, top)]);
    s.displace_faces(3).unwrap();
    s.tex.material = "nature/grass01".into();
    h.run_steps(3);
    let disp = |h: &mut Harness<MapCraftApp>| faces(h).into_iter().find(|f| f.id == top).unwrap().disp.unwrap();
    h.get_all_by_label("Apply current material").last().unwrap().click_accesskit();
    h.run_steps(2);
    assert_eq!(disp(&mut h).blend_material.as_deref(), Some("nature/grass01"), "the Blend material bucket blends the current material");
    h.get_by_label("Fill blend").click_accesskit();
    h.run_steps(2);
    assert!(disp(&mut h).blend.iter().all(|w| *w == 1.0), "Fill blend at 100 % covers the face");
    h.get_by_label("Remove Blend material").click_accesskit();
    h.run_steps(2);
    let d = disp(&mut h);
    assert!(d.blend_material.is_none() && !d.blend.is_empty(), "removing the second material keeps the shares");
}

fn click(h: &mut Harness<MapCraftApp>, at: Pos2) {
    h.hover_at(at);
    h.run_steps(1);
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

#[test]
fn mirror_groups_show_and_pick_their_copies() {
    use mapcraft_geom::{Aabb, DVec3};
    let (mut h, _settings) = editor();
    let original = h.state_mut().session_for_test().create_brush(Aabb::new(DVec3::new(-128.0, 0.0, 0.0), DVec3::new(0.0, 64.0, 64.0))).unwrap();
    h.run_steps(2);
    h.state_mut().run_command_for_test("tools.mirror");
    h.run_steps(2);
    let group = *h.state_mut().session_for_test().selection.iter().next().unwrap();
    assert_ne!(group, original);

    let top = 1;
    h.state_mut().run_command_for_test("edit.select_none");
    let copy = h.state().screen_for_test(top, DVec3::new(64.0, 32.0, 0.0));
    click(&mut h, copy);
    assert_eq!(h.state_mut().session_for_test().selection, std::collections::BTreeSet::from([group]), "clicking the copy selects its mirror group");
    h.get_by_label("Make real");
    let carry = |h: &mut Harness<MapCraftApp>| match &h.state_mut().session_for_test().map.get(group).unwrap().kind {
        mapcraft_map::NodeKind::Group(g) => g.mirror.unwrap().texture_lock,
        _ => panic!("not a group"),
    };
    let before = carry(&mut h);
    h.get_by_label("Carry textures").click();
    h.run_steps(2);
    assert_ne!(carry(&mut h), before, "the row's checkbox carries its label");

    h.state_mut().run_command_for_test("edit.select_solids");
    click(&mut h, copy);
    assert_eq!(h.state_mut().session_for_test().selection, std::collections::BTreeSet::from([original]), "with Solids it selects the brush it copies");

    h.state_mut().session_for_test().selection = std::collections::BTreeSet::from([group]);
    h.run_steps(2);
    h.get_by_label("Make real").click();
    h.run_steps(2);
    assert_eq!(h.state().node_count_for_test(), 4, "making it real adds the copy to the map");
    let s = h.state_mut().session_for_test();
    let xs: Vec<f64> = s.map.nodes().filter_map(|(_, n)| n.kind.brush()?.bounds().ok()).map(|b| b.min.x).collect();
    assert!(xs.contains(&0.0), "the copy lands across the plane at x = 0: {xs:?}");
}

#[test]
fn sdk_manager_checks_and_creates_sdks() {
    use mapcraft_sdk_manager::Level;
    let (mut h, settings) = editor();
    h.state_mut().run_command_for_test("file.sdk_manager");
    h.run_steps(2);
    assert!(h.state().sdk_ids_for_test().iter().any(|id| id == "surfsup"));
    // The top bar's map Check comes first; the SDK card's is drawn after it.
    h.get_all_by_label("Check").last().unwrap().click();
    let mut items = None;
    for _ in 0..400 {
        h.run_steps(1);
        items = h.state().sdk_check_for_test("surfsup");
        if items.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let items = items.expect("the check finishes on its worker thread");
    assert_eq!((items[0].level, items[0].what.as_str()), (Level::Pass, "manifest"));
    assert!(items.iter().any(|i| i.what.starts_with("sample") && i.level != Level::Fail), "{items:#?}");
    h.run_steps(2);
    assert!(h.query_by_label_contains("passed,").is_some(), "the results show inline");

    h.get_by_label("Create").click();
    h.run_steps(2);
    for (field, text) in [("Id", "my-game"), ("Name", "My Game")] {
        h.get_by_label(field).focus();
        h.run_steps(1);
        h.get_by_label(field).type_text(text);
        h.run_steps(1);
    }
    h.get_by_label("Create SDK").click();
    h.run_steps(3);
    assert!(settings.path().join("data/sdks/my-game/sdk.toml").is_file());
    assert_eq!(h.state().sdk_ids_for_test().first().map(String::as_str), Some("my-game"), "user SDKs list first");
    assert!(h.query_by_label("Remove…").is_some(), "a user SDK can be removed");
}

#[test]
fn uv_editor_grabs_locks_and_cancels() {
    use mapcraft_geom::{Aabb, DVec3};
    let (mut h, _settings) = editor();
    let b = h.state_mut().session_for_test().create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
    h.run_steps(2);
    h.state_mut().run_command_for_test("tool.face");
    h.run_steps(2);
    h.get_by_label("Side").click();
    h.run_steps(2);
    h.get_by_label("UV Editor").click();
    h.run_steps(2);
    let offsets = |h: &mut Harness<MapCraftApp>| -> Vec<(f64, f64)> {
        h.state_mut().session_for_test().map.get(b).unwrap().kind.brush().unwrap().faces.iter().map(|f| (f.u.offset, f.v.offset)).collect()
    };
    let before = offsets(&mut h);
    let at = Pos2::new(1050.0, 750.0);
    h.hover_at(at);
    h.run_steps(2);
    h.key_press(egui::Key::G);
    h.run_steps(2);
    h.hover_at(at + egui::vec2(40.0, 0.0));
    h.run_steps(2);
    assert_ne!(offsets(&mut h), before, "the texture follows the pointer");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(offsets(&mut h), before, "Esc puts it back");
    assert_eq!(h.state_mut().session_for_test().face_selection.len(), 6, "Esc leaves the Face tool alone");

    h.key_press(egui::Key::G);
    h.run_steps(2);
    h.hover_at(at + egui::vec2(40.0, 30.0));
    h.run_steps(1);
    h.key_press(egui::Key::Y);
    h.run_steps(2);
    click(&mut h, at + egui::vec2(40.0, 30.0));
    let after = offsets(&mut h);
    assert!(after.iter().zip(&before).all(|(a, b)| a.0 == b.0 && a.1 != b.1), "Y locks the grab to V: {after:?}");
    assert_eq!(h.state_mut().session_for_test().face_selection.len(), 6, "the click that applies picks nothing");
    assert_eq!(h.state_mut().session_for_test().undo_label(), Some("Move UVs"));
    h.state_mut().run_command_for_test("edit.undo");
    assert_eq!(offsets(&mut h), before, "one undo step");
}

#[test]
fn uv_editor_drags_picked_faces() {
    use mapcraft_geom::{Aabb, DVec3};
    let (mut h, _settings) = editor();
    let b = h.state_mut().session_for_test().create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
    h.run_steps(2);
    h.state_mut().run_command_for_test("tool.face");
    h.run_steps(2);
    let s = h.state_mut().session_for_test();
    let first = *s.face_selection.first().unwrap();
    s.face_selection.retain(|p| *p == first);
    s.active_face = Some(first);
    h.run_steps(3);
    let offsets = |h: &mut Harness<MapCraftApp>| -> Vec<(f64, f64)> {
        h.state_mut().session_for_test().map.get(b).unwrap().kind.brush().unwrap().faces.iter().map(|f| (f.u.offset, f.v.offset)).collect()
    };
    let before = offsets(&mut h);
    let at = h.state().uv_panel_for_test().0.center();
    drag_with(&mut h, at, at + egui::vec2(30.0, 20.0), Modifiers::NONE);
    let after = offsets(&mut h);
    assert_eq!(after.iter().zip(&before).filter(|(a, b)| a != b).count(), 1, "dragging the picked face moves its texture alone: {after:?}");
    assert_eq!(h.state_mut().session_for_test().undo_label(), Some("Move UVs"));
    assert_eq!(h.state_mut().session_for_test().face_selection.len(), 1, "the drag picks nothing");
    h.state_mut().run_command_for_test("edit.undo");
    assert_eq!(offsets(&mut h), before, "one undo step puts it back");

    h.hover_at(at);
    h.run_steps(1);
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.hover_at(at + egui::vec2(30.0, 0.0));
    h.run_steps(2);
    assert!(h.state().hint_for_test().starts_with("Moved "), "{}", h.state().hint_for_test());
    h.hover_at(at);
    h.run_steps(2);
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert_eq!(offsets(&mut h), before);
    assert_eq!(h.state_mut().session_for_test().undo_label(), Some("Create brush"), "a drag that ends where it began leaves no step");
}

#[test]
fn face_inspector_has_a_uv_editor() {
    use mapcraft_geom::{Aabb, DVec3};
    let settings = tempfile::tempdir().unwrap();
    let options = Options { settings_dir: Some(settings.path().to_path_buf()), ..Options::default() };
    // Short enough that scrolling down would scroll the face panel.
    let mut h = Harness::builder().with_size(egui::Vec2::new(1600.0, 640.0)).wgpu().build_eframe(|cc| MapCraftApp::new(cc, options));
    h.run_steps(3);
    let b = h.state_mut().session_for_test().create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
    h.run_steps(2);
    h.state_mut().run_command_for_test("tool.face");
    h.run_steps(3);
    let (rect, fitted, center) = h.state().uv_panel_for_test();
    assert!(rect.is_positive() && rect.width() > 200.0, "the Face tool's inspector shows a UV view: {rect:?}");
    let at = rect.center();
    let wheel = |h: &mut Harness<MapCraftApp>| {
        h.hover_at(at);
        h.run_steps(1);
        h.event(Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -120.0),
            modifiers: Modifiers::NONE,
            phase: egui::TouchPhase::Move,
        });
        h.run_steps(10);
    };
    wheel(&mut h);
    assert_ne!(h.state().uv_panel_for_test().1, fitted, "scrolling over it zooms it");
    assert_eq!(h.state().uv_panel_for_test().0, rect, "and leaves the inspector where it was");
    h.get_by_label("Fit UV View").click();
    h.run_steps(2);
    assert_eq!(h.state().uv_panel_for_test().1, fitted, "Fit frames the picked faces again");
    wheel(&mut h);
    h.key_press(egui::Key::F);
    h.run_steps(2);
    assert_eq!(h.state().uv_panel_for_test().1, fitted, "so does F over it");

    let offsets = |h: &mut Harness<MapCraftApp>| -> Vec<(f64, f64)> {
        h.state_mut().session_for_test().map.get(b).unwrap().kind.brush().unwrap().faces.iter().map(|f| (f.u.offset, f.v.offset)).collect()
    };
    let before = offsets(&mut h);
    h.hover_at(at);
    h.run_steps(2);
    h.key_press(egui::Key::G);
    h.run_steps(2);
    h.hover_at(at + egui::vec2(30.0, 0.0));
    h.run_steps(2);
    click(&mut h, at + egui::vec2(30.0, 0.0));
    assert_ne!(offsets(&mut h), before, "G over it grabs the picked faces' textures");
    assert_eq!(h.state_mut().session_for_test().undo_label(), Some("Move UVs"));
    assert_eq!(h.state_mut().session_for_test().face_selection.len(), 6, "the click that applies picks nothing");
    assert_eq!(h.state().uv_panel_for_test().2, center, "changing UVs leaves the view where it was");

    h.get_by_label("UV").click();
    h.run_steps(2);
    assert!(!h.state().uv_panel_for_test().0.is_positive(), "folding the section hides it");
    h.get_by_label("UV").click();
    h.run_steps(2);
    assert!(h.state().uv_panel_for_test().0.is_positive());

    h.state_mut().run_command_for_test("tool.select");
    h.state_mut().session_for_test().create_brush(Aabb::new(DVec3::new(2048.0, 0.0, 0.0), DVec3::new(2112.0, 64.0, 64.0))).unwrap();
    h.state_mut().run_command_for_test("tool.face");
    h.run_steps(3);
    assert_ne!(h.state().uv_panel_for_test().2, center, "coming back on another brush fits its faces");
}

#[test]
fn sky_toggles_from_the_3d_header() {
    let (mut h, _settings) = editor();
    assert_eq!(h.state().sky_for_test(), (true, Some(true)));
    h.get_by_label("Sky").click();
    h.run_steps(2);
    assert_eq!(h.state().sky_for_test(), (false, Some(false)), "the 3D view draws without the sky");
    h.state_mut().run_command_for_test("view.sky");
    h.run_steps(2);
    assert_eq!(h.state().sky_for_test(), (true, Some(true)), "View > Show Sky brings it back");
}

#[test]
fn vertex_tool_bevels_edges_and_merges_corners() {
    use mapcraft_geom::{Aabb, DVec3};
    let (mut h, _settings) = editor();
    let id = h.state_mut().session_for_test().create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
    h.run_steps(2);
    let brush = |h: &mut Harness<MapCraftApp>| h.state_mut().session_for_test().map.get(id).unwrap().kind.brush().unwrap().clone();
    h.state_mut().run_command_for_test("tool.vertex");
    h.key_press(egui::Key::Num2);
    h.run_steps(2);
    let side = 3;
    let at = h.state().screen_for_test(side, DVec3::new(64.0, 32.0, 64.0));
    click(&mut h, at);
    h.state_mut().run_command_for_test("vertex.bevel");
    let at = h.state().screen_for_test(side, DVec3::new(48.0, 32.0, 48.0));
    click(&mut h, at);
    assert_eq!(brush(&mut h).faces.len(), 7, "the picked edge is chamfered 16 units, snapped to the grid");
    h.get_by_value("Segments 1").click();
    h.run_steps(2);
    h.event(Event::Text("3".into()));
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(brush(&mut h).faces.len(), 9, "Adjust Last Bevel redoes it with three segments");
    h.state_mut().run_command_for_test("edit.undo");
    h.run_steps(2);
    assert_eq!(brush(&mut h).faces.len(), 6, "one undo step takes the adjusted bevel back");

    h.key_press(egui::Key::Num1);
    h.run_steps(2);
    let front = 2;
    let (a, b) = (h.state().screen_for_test(front, DVec3::new(0.0, -16.0, 80.0)), h.state().screen_for_test(front, DVec3::new(0.0, 80.0, 48.0)));
    drag(&mut h, a, b);
    h.key_press(egui::Key::M);
    h.run_steps(2);
    let merged = brush(&mut h).vertices().unwrap();
    assert_eq!(merged.len(), 5, "a box with its top merged is a pyramid");
    assert!(merged.iter().any(|v| v.distance(DVec3::new(32.0, 32.0, 64.0)) < 1e-6));
}

/// Flying fast holds Shift, which turns the wheel into a sideways scroll; it still sets the fly speed.
#[test]
fn shift_scroll_while_flying_changes_the_fly_speed() {
    let (mut h, _settings) = editor();
    let at = Pos2::new(560.0, 300.0);
    h.hover_at(at);
    h.run_steps(1);
    h.event(Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::SHIFT });
    for k in 1..4 {
        h.hover_at(at + egui::vec2(k as f32 * 4.0, 0.0));
        h.run_steps(1);
    }
    let before = h.state().fly_speed_for_test();
    h.event(Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::vec2(0.0, 120.0), modifiers: Modifiers::SHIFT, phase: egui::TouchPhase::Move });
    h.run_steps(8);
    h.event(Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::SHIFT });
    h.run_steps(1);
    assert!(h.state().fly_speed_for_test() > before, "Shift and the wheel speed up the flight");
}

#[test]
fn arrow_keys_nudge_picked_corners() {
    use mapcraft_geom::{Aabb, DVec3};
    let (mut h, _settings) = editor();
    let id = h.state_mut().session_for_test().create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
    h.run_steps(2);
    h.state_mut().run_command_for_test("tool.vertex");
    h.key_press(egui::Key::Num1);
    h.run_steps(2);
    let side = 3;
    let at = h.state().screen_for_test(side, DVec3::new(64.0, 32.0, 64.0));
    click(&mut h, at);
    h.key_press(egui::Key::ArrowRight);
    h.run_steps(2);
    let b = bounds(&mut h, id);
    assert_eq!((b.min.x, b.max.x), (0.0, 80.0), "the picked corners move one grid step and the rest of the brush stays");
    assert!(h.state_mut().session_for_test().map.get(id).unwrap().kind.brush().unwrap().vertices().unwrap().iter().any(|v| *v == DVec3::new(80.0, 0.0, 64.0)));
}

#[test]
fn clip_points_come_off_one_at_a_time_and_tilt_from_another_view() {
    use mapcraft_geom::{Aabb, DVec3};
    let (mut h, _settings) = editor();
    let id = h.state_mut().session_for_test().create_brush(Aabb::new(DVec3::ZERO, DVec3::new(128.0, 64.0, 64.0))).unwrap();
    h.run_steps(2);
    h.state_mut().run_command_for_test("tool.clip");
    let (top, side) = (1, 3);
    let (a, b) = (h.state().screen_for_test(top, DVec3::new(64.0, -32.0, 0.0)), h.state().screen_for_test(top, DVec3::new(64.0, 96.0, 0.0)));
    click(&mut h, a);
    click(&mut h, b);
    h.key_press(egui::Key::Delete);
    h.run_steps(2);
    assert!(h.state_mut().session_for_test().map.get(id).is_some(), "Delete in the Clip tool leaves the brush alone");
    assert_eq!(h.state().clip_points_for_test(), 1, "and takes back the last point");
    click(&mut h, b);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().clip_points_for_test(), 1, "Esc takes back one point too");
    click(&mut h, b);

    let tilt = h.state().screen_for_test(side, DVec3::new(96.0, 0.0, 64.0));
    click(&mut h, tilt);
    assert_eq!(h.state().clip_points_for_test(), 3, "a point in another 2D view is the third");
    h.get_by_label("Both").click();
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    let s = h.state_mut().session_for_test();
    let mut xs: Vec<(f64, f64)> = s.map.nodes().filter_map(|(_, n)| n.kind.brush()?.bounds().ok()).map(|b| (b.min.x.round(), b.max.x.round())).collect();
    xs.sort_by(|p, q| p.0.total_cmp(&q.0));
    assert_eq!(xs, [(0.0, 96.0), (32.0, 128.0)], "the cut tilts through the selection's middle: x - z = 32");
}

#[test]
fn double_clicking_a_face_clips_along_it() {
    use mapcraft_geom::{Aabb, DVec3};
    let settings = tempfile::tempdir().unwrap();
    let options = Options { settings_dir: Some(settings.path().to_path_buf()), ..Options::default() };
    // Frames a 60th of a second apart, so two clicks land within the double-click time.
    let mut h = Harness::builder().with_size(egui::Vec2::new(1600.0, 1000.0)).with_step_dt(1.0 / 60.0).wgpu().build_eframe(|cc| MapCraftApp::new(cc, options));
    h.run_steps(3);
    let s = h.state_mut().session_for_test();
    s.create_brush(Aabb::new(DVec3::new(-128.0, 0.0, 0.0), DVec3::new(-64.0, 64.0, 32.0))).unwrap();
    s.create_brush(Aabb::new(DVec3::ZERO, DVec3::new(128.0, 64.0, 64.0))).unwrap();
    h.run_steps(2);
    h.state_mut().run_command_for_test("tool.clip");
    let at = h.state().screen_for_test(0, DVec3::new(-96.0, 32.0, 32.0));
    h.hover_at(at);
    h.run_steps(1);
    for _ in 0..2 {
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(2);
    assert_eq!(h.state().clip_points_for_test(), 3, "the face's own plane points");
    h.get_by_label("Both").click();
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    let s = h.state_mut().session_for_test();
    let mut zs: Vec<(f64, f64)> =
        s.map.nodes().filter_map(|(_, n)| n.kind.brush()?.bounds().ok()).filter(|b| b.min.x >= 0.0).map(|b| (b.min.z, b.max.z)).collect();
    zs.sort_by(|p, q| p.0.total_cmp(&q.0));
    assert_eq!(zs, [(0.0, 32.0), (32.0, 64.0)], "the selected brush is cut flush with the other brush's top");
}

#[test]
fn edge_and_face_drags_keep_to_the_grid_and_their_shape() {
    use mapcraft_geom::{Aabb, DVec3};
    let (mut h, _settings) = editor();
    let id = h.state_mut().session_for_test().create_brush(Aabb::new(DVec3::ZERO, DVec3::new(64.0, 64.0, 48.0))).unwrap();
    h.run_steps(2);
    let corners = |h: &mut Harness<MapCraftApp>| h.state_mut().session_for_test().map.get(id).unwrap().kind.brush().unwrap().vertices().unwrap();
    h.state_mut().run_command_for_test("tool.vertex");
    h.key_press(egui::Key::Num3);
    h.run_steps(2);
    let side = 3;
    let (from, to) = (h.state().screen_for_test(side, DVec3::new(64.0, 32.0, 24.0)), h.state().screen_for_test(side, DVec3::new(96.0, 32.0, 24.0)));
    drag(&mut h, from, to);
    let moved: Vec<f64> = corners(&mut h).into_iter().filter(|v| v.x == 96.0).map(|v| v.z).collect();
    assert!(moved.len() == 4 && moved.iter().all(|z| *z == 0.0 || *z == 48.0), "the face moves a whole grid step along X only: {moved:?}");
    h.state_mut().run_command_for_test("edit.undo");

    let pts: Vec<DVec3> = [0.0, 64.0]
        .into_iter()
        .flat_map(|y| [DVec3::new(0.0, y, 0.0), DVec3::new(64.0, y, 0.0), DVec3::new(0.0, y, 48.0), DVec3::new(48.0, y, 48.0), DVec3::new(64.0, y, 32.0)])
        .collect();
    let s = h.state_mut().session_for_test();
    let shape = mapcraft_map::Brush::from_vertices(&pts, &[], &s.tex).unwrap();
    s.set_kind(id, mapcraft_map::NodeKind::Brush(shape), "Shape", None).unwrap();
    let chamfered = s.map.get(id).unwrap().kind.brush().unwrap().clone();
    h.run_steps(2);
    let (from, to) = (h.state().screen_for_test(side, DVec3::new(24.0, 32.0, 48.0)), h.state().screen_for_test(side, DVec3::new(24.0, 32.0, 16.0)));
    drag(&mut h, from, to);
    assert_eq!(
        h.state_mut().session_for_test().map.get(id).unwrap().kind.brush().unwrap(),
        &chamfered,
        "dragging the top face into the chamfer's foot is refused"
    );
    assert!(h.state().status_for_test().starts_with("Cannot move there"), "{}", h.state().status_for_test());
}

#[test]
fn vertex_delete_and_alt_drags_up_in_3d() {
    use mapcraft_geom::{Aabb, DVec3};
    let (mut h, _settings) = editor();
    let id = h.state_mut().session_for_test().create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
    h.run_steps(2);
    h.state_mut().run_command_for_test("tool.vertex");
    h.key_press(egui::Key::Num1);
    h.run_steps(2);
    let side = 3;
    let corner = h.state().screen_for_test(side, DVec3::new(64.0, 32.0, 64.0));
    click(&mut h, corner);
    h.key_press(egui::Key::Delete);
    h.run_steps(2);
    let s = h.state_mut().session_for_test();
    let b = s.map.get(id).expect("Delete in the Vertex tool keeps the brush").kind.brush().unwrap();
    assert_eq!(b.vertices().unwrap().len(), 6, "and takes off the picked corners, leaving a wedge");
    s.undo();
    h.run_steps(2);

    click(&mut h, corner);
    let (from, to) = (h.state().screen_for_test(0, DVec3::splat(64.0)), h.state().screen_for_test(0, DVec3::new(64.0, 64.0, 96.0)));
    drag_with(&mut h, from, to, Modifiers::ALT);
    assert_eq!(bounds(&mut h, id).max.z, 96.0, "Alt drags the picked corners straight up in the 3D view");
}

#[test]
fn entities_rest_on_faces_and_brush_entity_origins_drag() {
    use mapcraft_geom::{Aabb, DVec3};
    let (mut h, _settings) = editor();
    let ground = h.state_mut().session_for_test().create_brush(Aabb::new(DVec3::new(-128.0, -128.0, -64.0), DVec3::new(128.0, 128.0, 0.0))).unwrap();
    h.run_steps(2);
    h.state_mut().run_command_for_test("tool.entity");
    let floor = h.state().screen_for_test(0, DVec3::ZERO);
    click(&mut h, floor);
    let s = h.state_mut().session_for_test();
    let placed = *s.selection.iter().next().unwrap();
    assert_eq!(s.map.get(placed).unwrap().kind.entity().unwrap().origin(), Some(DVec3::new(0.0, 0.0, 8.0)), "the 16 unit box sits on the floor");

    s.selection = std::collections::BTreeSet::from([ground]);
    s.duplicate_linked(DVec3::new(0.0, 0.0, 16.0)).unwrap();
    h.run_steps(2);
    let raised = h.state().screen_for_test(0, DVec3::new(0.0, 0.0, 16.0));
    click(&mut h, raised);
    let s = h.state_mut().session_for_test();
    let placed = *s.selection.iter().next().unwrap();
    assert_eq!(s.map.get(placed).unwrap().kind.entity().unwrap().origin(), Some(DVec3::new(0.0, 0.0, 24.0)), "and on a linked copy's top");

    s.create_brush(Aabb::new(DVec3::new(256.0, 0.0, 0.0), DVec3::new(320.0, 8.0, 128.0))).unwrap();
    let door = s.tie_to_entity("func_door_rotating").unwrap();
    s.set_origins(&[door], Some(DVec3::new(256.0, 0.0, 0.0)), None).unwrap();
    h.state_mut().run_command_for_test("tool.select");
    // The inspector settles to the door's keys, which moves the views.
    h.run_steps(6);
    let top = 1;
    let (from, to) = (h.state().screen_for_test(top, DVec3::new(256.0, 0.0, 0.0)), h.state().screen_for_test(top, DVec3::new(320.0, 0.0, 0.0)));
    drag(&mut h, from, to);
    let s = h.state_mut().session_for_test();
    assert_eq!(s.map.get(door).unwrap().kind.entity().unwrap().origin(), Some(DVec3::new(320.0, 0.0, 0.0)), "the origin handle drags the hinge");
    let leaf = *s.map.get(door).unwrap().children.first().unwrap();
    assert_eq!(s.map.get(leaf).unwrap().kind.brush().unwrap().bounds().unwrap().min.x, 256.0, "and leaves the door where it is");

    s.create_brush(Aabb::new(DVec3::new(512.0, 0.0, 0.0), DVec3::new(576.0, 8.0, 128.0))).unwrap();
    let other = s.tie_to_entity("func_door_rotating").unwrap();
    s.set_origins(&[other], Some(DVec3::new(512.0, 0.0, 0.0)), None).unwrap();
    s.selection = std::collections::BTreeSet::from([door, other]);
    h.state_mut().run_command_for_test("tool.vertex");
    h.run_steps(6);
    let corner = h.state().screen_for_test(top, DVec3::new(256.0, 8.0, 0.0));
    click(&mut h, corner);
    h.state_mut().run_command_for_test("vertex.origin_to_corners");
    let s = h.state_mut().session_for_test();
    let origin = |id| s.map.get(id).unwrap().kind.entity().unwrap().origin().unwrap();
    assert_eq!((origin(door).x, origin(door).z), (256.0, 64.0), "Set Origin to Selected Corners hangs the door on the picked edge");
    assert_eq!(origin(other), DVec3::new(512.0, 0.0, 0.0), "and leaves the other selected door alone");
}

fn key_with(h: &mut Harness<MapCraftApp>, key: egui::Key, modifiers: Modifiers) {
    h.event(Event::ModifiersChanged(modifiers));
    h.event(Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
    h.event(Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers });
    h.run_steps(2);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
}

#[test]
fn ctrl_drag_and_ctrl_arrows_move_copies() {
    use mapcraft_geom::{Aabb, DVec3};
    let (mut h, _settings) = editor();
    let b = h.state_mut().session_for_test().create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
    h.run_steps(2);
    let top = 1;
    let (from, to) = (h.state().screen_for_test(top, DVec3::new(32.0, 32.0, 0.0)), h.state().screen_for_test(top, DVec3::new(160.0, 32.0, 0.0)));
    let back = h.state().screen_for_test(top, DVec3::new(96.0, 32.0, 0.0));
    h.event(Event::ModifiersChanged(Modifiers::COMMAND));
    h.hover_at(from);
    h.run_steps(1);
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::COMMAND });
    for p in [back, from, back, to] {
        h.hover_at(p);
        h.run_steps(1);
    }
    h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::COMMAND });
    h.run_steps(2);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    assert_eq!(h.state().node_count_for_test(), 3, "Ctrl+drag moves one copy, even through its start");
    assert_eq!(bounds(&mut h, b).min.x, 0.0, "and leaves the original where it was");
    let copy = *h.state_mut().session_for_test().selection.iter().next().unwrap();
    assert_eq!(bounds(&mut h, copy).min.x, 128.0);
    h.state_mut().run_command_for_test("edit.undo");
    h.run_steps(2);
    assert_eq!(h.state().node_count_for_test(), 2, "one undo takes back the copy and its move");

    h.state_mut().session_for_test().selection = std::collections::BTreeSet::from([b]);
    h.hover_at(from);
    h.run_steps(2);
    key_with(&mut h, egui::Key::ArrowRight, Modifiers::COMMAND);
    assert_eq!(h.state().node_count_for_test(), 3, "Ctrl+arrow nudges a copy");
    assert_eq!(bounds(&mut h, b).min.x, 0.0);
    key_with(&mut h, egui::Key::ArrowRight, Modifiers::SHIFT);
    let copy = *h.state_mut().session_for_test().selection.iter().next().unwrap();
    assert_eq!(bounds(&mut h, copy).min.x, 17.0, "Shift+arrow nudges one unit");
}

#[test]
fn shift_click_picks_a_face_from_the_select_tool() {
    use mapcraft_geom::{Aabb, DVec3};
    let (mut h, _settings) = editor();
    let b = h.state_mut().session_for_test().create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
    h.run_steps(2);
    let at = h.state().screen_for_test(0, DVec3::new(32.0, 32.0, 64.0));
    let click = |h: &mut Harness<MapCraftApp>, modifiers: Modifiers| {
        h.event(Event::ModifiersChanged(modifiers));
        h.hover_at(at);
        h.run_steps(1);
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers });
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers });
        h.run_steps(2);
        h.event(Event::ModifiersChanged(Modifiers::NONE));
    };
    click(&mut h, Modifiers::SHIFT);
    let s = h.state_mut().session_for_test();
    assert!(s.selection.is_empty());
    let picked: Vec<_> = s.face_selection.iter().copied().collect();
    assert_eq!(picked.len(), 1, "Shift+click picks just the face under the pointer");
    let top = s.map.get(b).unwrap().kind.brush().unwrap().faces.iter().find(|f| f.plane().unwrap().normal.z > 0.9).unwrap().id;
    assert_eq!(picked[0], (b, top));
    click(&mut h, Modifiers::COMMAND | Modifiers::SHIFT);
    assert!(h.state_mut().session_for_test().face_selection.is_empty(), "now in the Face tool, Ctrl+Shift+click toggles faces");
}

#[test]
fn leak_trails_fly_the_camera_along_them() {
    use mapcraft_geom::DVec3;
    let (mut h, settings) = editor();
    let lin = settings.path().join("leak.lin");
    std::fs::write(&lin, "0 0 0\n100 0 0\n100 100 0\n").unwrap();
    h.state_mut().load_pointfile_for_test(&lin);
    h.state_mut().run_command_for_test("run.point_next");
    h.state_mut().run_command_for_test("run.point_next");
    let (position, yaw, pitch) = h.state().camera_for_test(0);
    assert_eq!(position, DVec3::new(100.0, 0.0, 0.0), "the second point");
    assert!((yaw - 90.0).abs() < 1e-9 && pitch.abs() < 1e-9, "looking along the trail");
    h.state_mut().run_command_for_test("run.point_prev");
    assert_eq!(h.state().camera_for_test(0).0, DVec3::ZERO);
    h.run_steps(2);
}

#[test]
fn alt_right_drag_orbits_the_point_under_the_pointer() {
    use mapcraft_geom::{Aabb, DVec3};
    let (mut h, _settings) = editor();
    h.state_mut().session_for_test().create_brush(Aabb::new(DVec3::splat(-64.0), DVec3::splat(64.0))).unwrap();
    h.run_steps(2);
    let at = h.state().screen_for_test(0, DVec3::ZERO);
    let (before, yaw, _) = h.state().camera_for_test(0);
    let toward = (DVec3::ZERO - before).normalize();
    let pivot = before + toward * Aabb::new(DVec3::splat(-64.0), DVec3::splat(64.0)).ray_hit(before, toward).unwrap();
    h.event(Event::ModifiersChanged(Modifiers::ALT));
    h.hover_at(at);
    h.run_steps(1);
    h.event(Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::ALT });
    for k in 1..=8 {
        h.hover_at(at + egui::vec2(k as f32 * 10.0, 0.0));
        h.run_steps(1);
    }
    h.event(Event::PointerButton { pos: at + egui::vec2(80.0, 0.0), button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::ALT });
    h.run_steps(1);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    let (after, turned, _) = h.state().camera_for_test(0);
    assert!((turned - yaw).abs() > 5.0, "the camera turns");
    assert!(after.distance(before) > 64.0, "and swings around the brush instead of turning in place");
    assert!((after.distance(pivot) - before.distance(pivot)).abs() < 1e-6, "keeping its distance");
    assert!(h.state().screen_for_test(0, pivot).distance(at) < 2.0, "with the pivot still under the pointer");
}

#[test]
fn f3_searches_and_runs_commands() {
    use mapcraft_geom::{Aabb, DVec3};
    let (mut h, _settings) = editor();
    h.state_mut().session_for_test().create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
    h.state_mut().session_for_test().selection.clear();
    h.run_steps(2);
    h.key_press(egui::Key::F3);
    h.run_steps(2);
    h.event(Event::Text("select all".into()));
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(h.state_mut().session_for_test().selection.len(), 1, "Enter runs the best match");
}

/// Connects to the editor's control channel the way `mapcraft-cli mcp` does, sending `token` or the real one.
fn connect(settings: &std::path::Path, token: Option<&str>) -> (std::io::BufReader<std::net::TcpStream>, std::net::TcpStream) {
    use std::io::Write;
    let file: Value = serde_json::from_slice(&std::fs::read(settings.join("data/control.json")).unwrap()).unwrap();
    let stream = std::net::TcpStream::connect(("127.0.0.1", file["port"].as_u64().unwrap() as u16)).unwrap();
    stream.set_read_timeout(Some(std::time::Duration::from_secs(20))).unwrap();
    let mut out = stream.try_clone().unwrap();
    writeln!(out, "{}", json!({"token": token.map_or(file["token"].clone(), |t| json!(t))})).unwrap();
    (std::io::BufReader::new(stream), out)
}

/// Runs `script` on a thread as an agent would while the editor runs frames, and returns what it returns.
fn drive<T: Send + 'static>(
    h: &mut Harness<MapCraftApp>,
    settings: &std::path::Path,
    script: impl FnOnce(&mut dyn FnMut(&str, Value) -> Value) -> T + Send + 'static,
) -> T {
    use std::io::{BufRead, Write};
    let (mut reader, mut out) = connect(settings, None);
    let client = std::thread::spawn(move || {
        let mut ask = |name: &str, args: Value| {
            writeln!(out, "{}", json!({"name": name, "arguments": args})).unwrap();
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let reply: Value = serde_json::from_str(&line).unwrap();
            let text = reply["content"][0]["text"].as_str().map(|t| serde_json::from_str(t).unwrap_or(json!(t)));
            if reply["isError"] == true { json!({"error": text}) } else { text.unwrap_or(reply) }
        };
        script(&mut ask)
    });
    for _ in 0..2000 {
        if client.is_finished() {
            break;
        }
        h.run_steps(1);
    }
    client.join().unwrap()
}

#[test]
fn agents_drive_the_editor_over_the_control_channel() {
    use std::io::{BufRead, Write};

    use base64::Engine as _;

    let (mut h, settings) = editor();
    let (mut intruder, mut out) = connect(settings.path(), Some("guess"));
    let _ = writeln!(out, "{}", json!({"name": "info"}));
    assert_eq!(intruder.read_line(&mut String::new()).unwrap_or(0), 0, "a wrong token is hung up on");

    let (made, shot, missing) = drive(&mut h, settings.path(), |ask| {
        let made = ask(
            "batch",
            json!({"calls": [
                {"tool": "create_brush", "arguments": {"min": [-256, -256, 0], "max": [256, 256, 192]}},
                {"tool": "hollow", "arguments": {"thickness": 16}},
                {"tool": "create_entity", "arguments": {"classname": "info_player_start", "origin": [0, 0, 32]}},
            ]}),
        );
        (made, ask("screenshot", json!({"width": 640, "height": 480})), ask("select", json!({"ids": [999]})))
    });
    assert!(made.get("error").is_none(), "{made}");
    assert_eq!(h.state().node_count_for_test(), 8, "a hollowed room and a player start");
    assert!(missing.get("error").is_some());

    let data = shot["content"][0]["data"].as_str().unwrap();
    let png = base64::engine::general_purpose::STANDARD.decode(data).unwrap();
    if let Some(p) = std::env::var_os("MAPCRAFT_SHOT") {
        std::fs::write(p, &png).unwrap();
    }
    let img = image::load_from_memory(&png).unwrap().to_rgba8();
    assert_eq!(img.dimensions(), (640, 480));
    let first = img.get_pixel(0, 0);
    assert!(img.pixels().any(|p| p != first), "the views show something");
}

#[test]
fn agents_build_with_the_maps_sdk() {
    let settings = tempfile::tempdir().unwrap();
    let sample = concat!(env!("CARGO_MANIFEST_DIR"), "/../../sdks/surfsup/samples/surf_mapcraft.mcmap");
    let options = Options { settings_dir: Some(settings.path().to_path_buf()), open: Some(sample.into()), ..Options::default() };
    let mut h = Harness::builder().with_size(egui::Vec2::new(1600.0, 1000.0)).wgpu().build_eframe(|cc| MapCraftApp::new(cc, options));
    h.run_steps(3);
    let new_map = settings.path().join("course.mcmap");
    let path = new_map.to_str().unwrap().to_string();

    let r = drive(&mut h, settings.path(), move |ask| {
        let mut r = serde_json::Map::new();
        r.insert("info".into(), ask("info", json!({})));
        r.insert("zones".into(), ask("entity_classes", json!({"filter": "zone_"})));
        r.insert("start".into(), ask("entity_class", json!({"name": "zone_start"})));
        r.insert("plugins".into(), ask("plugin_commands", json!({})));
        r.insert("new".into(), ask("new", json!({"path": path})));
        r.insert("ramp".into(), ask("run_plugin", json!({"plugin": "surfsup", "command": "surf_ramp", "origin": [0, 0, 0], "yaw": 90})));
        r.insert("bad_param".into(), ask("run_plugin", json!({"plugin": "surfsup", "command": "surf_ramp", "params": {"nope": 1}})));
        r.insert(
            "zone".into(),
            ask(
                "batch",
                json!({"calls": [
                    {"tool": "create_brush", "arguments": {"min": [-128, -128, 0], "max": [128, 128, 128]}},
                    {"tool": "tie_to_entity", "arguments": {"classname": "zone_start"}},
                ]}),
            ),
        );
        r.insert("validate".into(), ask("validate", json!({})));
        r.insert("save".into(), ask("save", json!({})));
        Value::Object(r)
    });
    assert_eq!(r["info"]["sdk"], "surfsup", "{}", r["info"]);
    assert_eq!(r["info"]["sdk_loaded"], true);
    assert!(r["zones"].as_array().unwrap().iter().any(|c| c["name"] == "zone_start" && c["kind"] == "solid"), "{}", r["zones"]);
    assert!(r["start"]["keys"].is_array(), "{}", r["start"]);
    let surfsup = r["plugins"].as_array().unwrap().iter().find(|p| p["plugin"] == "surfsup").unwrap();
    assert!(surfsup["commands"].as_array().unwrap().iter().any(|c| c["command"] == "surf_ramp" && c["generator"] == true));
    assert_eq!(r["new"]["sdk"], "surfsup", "a new map keeps the open map's SDK");
    assert!(r["ramp"].get("error").is_none(), "{}", r["ramp"]);
    assert!(r["bad_param"]["error"].as_str().unwrap().contains("no parameter `nope`"));
    assert!(r["zone"].get("error").is_none(), "{}", r["zone"]);
    assert!(r["validate"].is_array(), "{}", r["validate"]);
    let saved = mapcraft_format::load(&new_map).unwrap();
    assert!(saved.map.nodes().any(|(_, n)| n.kind.entity().is_some_and(|e| e.classname == "zone_start")));
    assert!(saved.map.nodes().any(|(_, n)| matches!(&n.kind, mapcraft_map::NodeKind::Group(g) if g.generator.is_some())));
}

fn message(h: &mut Harness<MapCraftApp>, id: mapcraft_map::NodeId) -> Option<String> {
    h.state_mut().session_for_test().map.get(id).unwrap().kind.entity().unwrap().get("message").map(String::from)
}

#[test]
fn inspector_edits_several_entities() {
    use mapcraft_geom::DVec3;
    let (mut h, _settings) = editor();
    let s = h.state_mut().session_for_test();
    let a = s.create_point_entity("info_target", DVec3::ZERO, vec![("message".into(), "a".into())]).unwrap();
    let b = s.create_point_entity("info_target", DVec3::splat(64.0), vec![("message".into(), "b".into())]).unwrap();
    s.selection = std::collections::BTreeSet::from([a, b]);
    h.run_steps(2);
    h.get_by_label("message").focus();
    h.run_steps(1);
    h.get_by_label("message").type_text("c");
    h.run_steps(3);
    assert_eq!((message(&mut h, a).as_deref(), message(&mut h, b).as_deref()), (Some("c"), Some("c")), "typing into a mixed row sets every entity");
    h.state_mut().session_for_test().undo();
    assert_eq!((message(&mut h, a).as_deref(), message(&mut h, b).as_deref()), (Some("a"), Some("b")), "one undo takes the edit back");
}

#[test]
fn separate_inspector_edits_are_separate_undo_steps() {
    use mapcraft_geom::DVec3;
    let (mut h, _settings) = editor();
    let a = h.state_mut().session_for_test().create_point_entity("info_target", DVec3::ZERO, vec![("message".into(), "a".into())]).unwrap();
    h.run_steps(2);
    h.get_by_label("message").focus();
    h.run_steps(1);
    h.get_by_label("message").type_text("b");
    h.run_steps(2);
    let typed = message(&mut h, a);
    assert_ne!(typed.as_deref(), Some("a"));
    h.get_by_label("Remove message").click();
    h.run_steps(2);
    assert_eq!(message(&mut h, a), None);
    h.state_mut().session_for_test().undo();
    assert_eq!(message(&mut h, a), typed, "the first undo brings the key back");
    h.state_mut().session_for_test().undo();
    assert_eq!(message(&mut h, a).as_deref(), Some("a"), "the second undo takes back the typing");
}

#[test]
fn showing_the_inspector_writes_nothing() {
    use mapcraft_geom::{Aabb, DVec3};
    use mapcraft_map::{Connection, NodeKind};
    let (mut h, _settings) = editor();
    h.state_mut().run_command_for_test("file.switch_sdk");
    h.run_steps(2);
    h.get_by_label("Quake").click();
    h.run_steps(3);
    let s = h.state_mut().session_for_test();
    let light = s.create_point_entity("light", DVec3::ZERO, Vec::new()).unwrap();
    let mut e = s.map.get(light).unwrap().kind.entity().unwrap().clone();
    let wire = Connection { output: "OnUser1".into(), target: "x".into(), input: "Toggle".into(), param: String::new(), delay: -1.0, times_to_fire: -1 };
    e.connections.push(wire);
    s.set_kind(light, NodeKind::Entity(e), "Wire", None).unwrap();
    s.selection = std::collections::BTreeSet::from([light]);
    let (kind, label) = (s.map.get(light).unwrap().kind.clone(), s.undo_label().map(String::from));
    h.run_steps(3);
    let s = h.state_mut().session_for_test();
    assert_eq!(s.map.get(light).unwrap().kind, kind, "no spawnflags and the out of range delay stay as they are");
    assert_eq!(s.undo_label().map(String::from), label);

    let b = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
    let mut brush = s.map.get(b).unwrap().kind.brush().unwrap().clone();
    for f in &mut brush.faces {
        (f.u.scale, f.v.scale) = (-1.0, -1.0);
    }
    s.set_kind(b, NodeKind::Brush(brush), "Flip", None).unwrap();
    s.selection = std::collections::BTreeSet::from([b]);
    let label = s.undo_label().map(String::from);
    h.run_steps(3);
    let s = h.state_mut().session_for_test();
    assert!(s.map.get(b).unwrap().kind.brush().unwrap().faces.iter().all(|f| f.u.scale == -1.0 && f.v.scale == -1.0));
    assert_eq!(s.undo_label().map(String::from), label);
}

fn type_keys(h: &mut Harness<MapCraftApp>, label: &str, keys: &[&str]) {
    h.get_by_label(label).focus();
    h.run_steps(1);
    for k in keys {
        h.get_by_label(label).type_text(k);
        h.run_steps(1);
    }
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
}

#[test]
fn face_fields_apply_what_was_typed() {
    use mapcraft_geom::{Aabb, DVec3};
    let (mut h, _settings) = editor();
    let b = h.state_mut().session_for_test().create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
    h.run_steps(2);
    h.state_mut().run_command_for_test("tool.face");
    h.run_steps(2);
    type_keys(&mut h, "Texture scale", &["0", ".", "5"]);
    type_keys(&mut h, "Rotation", &["-", "4", "5"]);
    let faces = h.state_mut().session_for_test().map.get(b).unwrap().kind.brush().unwrap().faces.clone();
    assert!(faces.iter().all(|f| f.u.scale == 0.5 && f.v.scale == 1.0), "a scale below 1 survives its leading 0");
    assert!(faces.iter().all(|f| f.rotation == 315.0), "-45 lands on 315, not on what the first keystroke wrapped to");
}

#[test]
fn add_key_keeps_a_key_that_is_already_set() {
    use mapcraft_geom::DVec3;
    let (mut h, _settings) = editor();
    let a = h.state_mut().session_for_test().create_point_entity("info_target", DVec3::ZERO, vec![("message".into(), "a".into())]).unwrap();
    h.run_steps(2);
    let new_key = |n: &egui_kittest::kittest::AccessKitNode<'_>| n.placeholder() == Some("new key");
    h.get_by(new_key).focus();
    h.run_steps(1);
    h.get_by(new_key).type_text("message");
    h.run_steps(1);
    h.get_by_label("Add key").click();
    h.run_steps(2);
    assert_eq!(message(&mut h, a).as_deref(), Some("a"));
    assert_eq!(h.state().status_for_test(), "message is already set");
}

#[test]
fn brush_rows_show_world_and_keep_flipped_textures() {
    use egui_kittest::kittest::NodeT;
    use mapcraft_geom::{Aabb, DVec3};
    use mapcraft_map::NodeKind;
    let (mut h, _settings) = editor();
    let s = h.state_mut().session_for_test();
    let b = s.create_brush(Aabb::new(DVec3::ZERO, DVec3::splat(64.0))).unwrap();
    let mut brush = s.map.get(b).unwrap().kind.brush().unwrap().clone();
    for f in &mut brush.faces {
        (f.u.scale, f.v.scale) = (-1.0, -1.0);
    }
    s.set_kind(b, NodeKind::Brush(brush), "Flip", None).unwrap();
    s.selection = std::collections::BTreeSet::from([b]);
    h.run_steps(2);
    let world = h.query_all_by_label("World").filter_map(|n| n.accesskit_node().toggled()).collect::<Vec<_>>();
    assert!(world.contains(&egui::accesskit::Toggled::True), "a world brush lights the World segment: {world:?}");
    type_keys(&mut h, "Texture scale", &["2"]);
    let faces = h.state_mut().session_for_test().map.get(b).unwrap().kind.brush().unwrap().faces.clone();
    assert!(faces.iter().all(|f| f.u.scale == -2.0 && f.v.scale == -2.0), "flipped textures stay flipped");
}
