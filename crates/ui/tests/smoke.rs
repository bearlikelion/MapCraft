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
