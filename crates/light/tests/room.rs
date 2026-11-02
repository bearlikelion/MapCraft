//! Bakes small sealed rooms and checks the light that lands on their faces.

use std::collections::BTreeSet;

use mapcraft_geom::{Aabb, DVec3};
use mapcraft_light::{Lightmap, Progress, Settings, Surface, bake, rgbe, shade};
use mapcraft_map::{Brush, Entity, Map, NodeId, NodeKind, ROOT, TexDefaults};

fn tex() -> TexDefaults {
    TexDefaults { material: "dev/grid".into(), scale: 1.0 }
}

/// A hollow box with 16 unit walls around the inside `min..max`.
fn room(map: &mut Map, min: DVec3, max: DVec3) -> Vec<NodeId> {
    let t = 16.0;
    let (lo, hi) = (min - t, max + t);
    let slabs = [
        Aabb::new(lo, DVec3::new(hi.x, hi.y, min.z)),
        Aabb::new(DVec3::new(lo.x, lo.y, max.z), hi),
        Aabb::new(DVec3::new(lo.x, lo.y, min.z), DVec3::new(min.x, hi.y, max.z)),
        Aabb::new(DVec3::new(max.x, lo.y, min.z), DVec3::new(hi.x, hi.y, max.z)),
        Aabb::new(DVec3::new(min.x, lo.y, min.z), DVec3::new(max.x, min.y, max.z)),
        Aabb::new(DVec3::new(min.x, max.y, min.z), DVec3::new(max.x, hi.y, max.z)),
    ];
    slabs.into_iter().map(|b| add(map, NodeKind::Brush(Brush::cuboid(b, &tex())))).collect()
}

fn add(map: &mut Map, kind: NodeKind) -> NodeId {
    let mut e = map.edit();
    let id = e.insert(ROOT, kind).unwrap();
    let changes = e.finish();
    map.apply(changes);
    id
}

fn light(map: &mut Map, at: DVec3, keys: &[(&str, &str)]) {
    let mut e = Entity::new("light");
    e.set("origin", format!("{} {} {}", at.x, at.y, at.z));
    for (k, v) in keys {
        e.set(*k, *v);
    }
    add(map, NodeKind::Entity(e));
}

fn run(map: &Map, settings: &Settings) -> Lightmap {
    let (lm, warnings) = bake(map, &|_| Surface::default(), &BTreeSet::new(), settings, &Progress::default()).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    lm
}

/// What Godot shows at world point `p` on the face of `node` facing `normal`.
fn seen(lm: &Lightmap, map: &Map, node: NodeId, normal: DVec3, p: DVec3) -> f64 {
    let b = map.get(node).unwrap().kind.brush().unwrap();
    let face = b.faces.iter().position(|f| f.plane().unwrap().normal.abs_diff_eq(normal, 1e-6)).unwrap();
    let f = lm.face(node, face).unwrap_or_else(|| panic!("no lightmap for face {face} of {node:?}"));
    let uv = f.uv_at(p.to_array());
    let s = f64::from(lm.size);
    let (x, y) = ((f64::from(uv[0]) * s) as usize, (f64::from(uv[1]) * s) as usize);
    let slice = &lm.slices[f.slice as usize];
    let at = y * lm.size as usize + x;
    let texels = std::array::from_fn(|k| rgbe::decode(slice.layers[k][at]));
    shade(&texels, normal).x
}

fn direct_only() -> Settings {
    Settings { luxel: 8.0, slice: 512, bounces: 0, rays: 8, sun_samples: 1, threads: 2 }
}

#[test]
fn a_light_falls_off_with_distance_squared() {
    let mut map = Map::new();
    let walls = room(&mut map, DVec3::new(-512.0, -512.0, 0.0), DVec3::new(512.0, 512.0, 512.0));
    light(&mut map, DVec3::new(0.0, 0.0, 100.0), &[("_light", "255 255 255 255")]);
    let lm = run(&map, &direct_only());
    let floor = walls[0];
    let below = seen(&lm, &map, floor, DVec3::Z, DVec3::new(4.0, 4.0, 0.0));
    assert!((below - 1.0).abs() < 0.08, "under the light: {below}");
    // 100 units out the light is sqrt(2) times farther and strikes at 45 degrees: 0.5 * 0.707.
    let aside = seen(&lm, &map, floor, DVec3::Z, DVec3::new(100.0, 0.0, 0.0));
    assert!((aside - 0.354).abs() < 0.05, "off to the side: {aside}");
}

#[test]
fn a_pillar_casts_a_shadow() {
    let mut map = Map::new();
    let walls = room(&mut map, DVec3::new(-512.0, -512.0, 0.0), DVec3::new(512.0, 512.0, 512.0));
    add(&mut map, NodeKind::Brush(Brush::cuboid(Aabb::new(DVec3::new(-32.0, -32.0, 0.0), DVec3::new(32.0, 32.0, 256.0)), &tex())));
    light(&mut map, DVec3::new(-256.0, 0.0, 128.0), &[("_light", "255 255 255 255")]);
    let lm = run(&map, &direct_only());
    let lit = seen(&lm, &map, walls[0], DVec3::Z, DVec3::new(-256.0, 200.0, 0.0));
    let shaded = seen(&lm, &map, walls[0], DVec3::Z, DVec3::new(128.0, 0.0, 0.0));
    assert!(lit > 0.05, "lit side {lit}");
    assert!(shaded < lit * 0.05, "behind the pillar {shaded} vs {lit}");
}

#[test]
fn bounce_lights_the_ceiling_a_downward_spot_misses() {
    let mut map = Map::new();
    let walls = room(&mut map, DVec3::new(-256.0, -256.0, 0.0), DVec3::new(256.0, 256.0, 256.0));
    let mut e = Entity::new("light_spot");
    e.set("origin", "0 0 200");
    e.set("_light", "255 255 255 2000");
    e.set("angles", "-90 0 0");
    add(&mut map, NodeKind::Entity(e));
    let ceiling = |s: &Settings| seen(&run(&map, s), &map, walls[1], -DVec3::Z, DVec3::new(100.0, 100.0, 256.0));
    let dark = ceiling(&direct_only());
    let bounced = ceiling(&Settings { bounces: 2, rays: 64, ..direct_only() });
    assert!(dark < 1e-3, "no direct light reaches the ceiling: {dark}");
    assert!(bounced > 0.02, "bounce should light it: {bounced}");
}

#[test]
fn unlit_outer_faces_collapse_to_one_luxel() {
    let mut map = Map::new();
    let walls = room(&mut map, DVec3::new(-512.0, -512.0, 0.0), DVec3::new(512.0, 512.0, 512.0));
    light(&mut map, DVec3::new(0.0, 0.0, 100.0), &[("_light", "255 255 255 255")]);
    let lm = run(&map, &direct_only());
    let b = map.get(walls[0]).unwrap().kind.brush().unwrap();
    let under = b.faces.iter().position(|f| f.plane().unwrap().normal.z < -0.5).unwrap();
    let f = lm.face(walls[0], under).unwrap();
    assert_eq!(&f.uv[0][..3], &[0.0, 0.0, 0.0], "the outside of the floor is one dark luxel");
}

#[test]
fn an_untextured_zone_on_the_floor_casts_no_shadow() {
    let mut map = Map::new();
    let walls = room(&mut map, DVec3::new(-512.0, -512.0, 0.0), DVec3::new(512.0, 512.0, 512.0));
    let zone = Brush::cuboid(Aabb::new(DVec3::new(-496.0, -496.0, 0.0), DVec3::new(496.0, 496.0, 256.0)), &TexDefaults { material: String::new(), scale: 1.0 });
    let mut e = Entity::new("kill_box");
    e.set("origin", "0 0 0");
    let kill = add(&mut map, NodeKind::Entity(e));
    let mut edit = map.edit();
    edit.insert(kill, NodeKind::Brush(zone)).unwrap();
    let changes = edit.finish();
    map.apply(changes);
    light(&mut map, DVec3::new(0.0, 0.0, 100.0), &[("_light", "255 255 255 255")]);
    let lm = run(&map, &direct_only());
    let below = seen(&lm, &map, walls[0], DVec3::Z, DVec3::new(4.0, 4.0, 0.0));
    assert!((below - 1.0).abs() < 0.08, "under the light through the zone: {below}");
}
