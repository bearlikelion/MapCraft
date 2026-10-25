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
    for p in [DVec3::new(64.0, -32.0, 0.0), DVec3::new(64.0, 96.0, 0.0)] {
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
    assert_eq!(h.state().node_count_for_test(), 3, "by its two halves");
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
