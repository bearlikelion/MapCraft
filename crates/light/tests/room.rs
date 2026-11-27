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
    run_with(map, settings, &|_| Surface::default())
}

fn run_with(map: &Map, settings: &Settings, surface: &(dyn Fn(&str) -> Surface + Sync)) -> Lightmap {
    let (lm, warnings) = bake(map, surface, &BTreeSet::new(), settings, &Progress::default()).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    lm
}

fn world(map: &mut Map, keys: &[(&str, &str)]) {
    let mut world = Entity::new("worldspawn");
    for (k, v) in keys {
        world.set(*k, *v);
    }
    let mut e = map.edit();
    e.set_kind(ROOT, NodeKind::World(world)).unwrap();
    let changes = e.finish();
    map.apply(changes);
}

/// What Godot shows at world point `p` on the face of `node` facing `normal`.
fn seen(lm: &Lightmap, map: &Map, node: NodeId, normal: DVec3, p: DVec3) -> f64 {
    color(lm, map, node, normal, p).x
}

fn color(lm: &Lightmap, map: &Map, node: NodeId, normal: DVec3, p: DVec3) -> DVec3 {
    let b = map.get(node).unwrap().kind.brush().unwrap();
    let face = b.faces.iter().position(|f| f.plane().unwrap().normal.abs_diff_eq(normal, 1e-6)).unwrap();
    let f = lm.face(node, face).unwrap_or_else(|| panic!("no lightmap for face {face} of {node:?}"));
    let uv = f.uv_at(p.to_array());
    let s = f64::from(lm.size);
    let (x, y) = ((f64::from(uv[0]) * s) as usize, (f64::from(uv[1]) * s) as usize);
    let slice = &lm.slices[f.slice as usize];
    let at = y * lm.size as usize + x;
    let texels = std::array::from_fn(|k| rgbe::decode(slice.layers[k][at]));
    shade(&texels, normal)
}

fn direct_only() -> Settings {
    Settings { luxel: 8.0, slice: 512, bounces: 0, rays: 8, sun_samples: 1, threads: 2, ..Settings::default() }
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

#[test]
fn a_displaced_ridge_is_lightmapped_and_casts_a_shadow() {
    let mut map = Map::new();
    room(&mut map, DVec3::new(-512.0, -512.0, 0.0), DVec3::new(512.0, 512.0, 512.0));
    let mut b = Brush::cuboid(Aabb::new(DVec3::new(-256.0, -256.0, 0.0), DVec3::new(256.0, 256.0, 16.0)), &tex());
    let solid = b.solid().unwrap();
    let top = b.faces.iter().position(|f| f.plane().unwrap().normal.z > 0.9).unwrap();
    let corners = &solid.faces[top].as_ref().unwrap().0;
    let mut d = mapcraft_map::Displacement::new(4, corners[0]);
    d.heights = d.base(corners).unwrap().iter().map(|p| 128.0 * (1.0 - p.x.abs() / 32.0).max(0.0)).collect();
    b.faces[top].disp = Some(d);
    let floor = add(&mut map, NodeKind::Brush(b));
    light(&mut map, DVec3::new(-256.0, 0.0, 64.0), &[("_light", "255 255 255 255")]);
    let lm = run(&map, &direct_only());
    let lit = seen(&lm, &map, floor, DVec3::Z, DVec3::new(-128.0, 0.0, 16.0));
    let shaded = seen(&lm, &map, floor, DVec3::Z, DVec3::new(160.0, 0.0, 16.0));
    assert!(lit > 0.05, "the side facing the light {lit}");
    assert!(shaded < lit * 0.05, "behind the ridge {shaded} vs {lit}");
}

#[test]
fn a_box_on_the_floor_leaves_no_dark_fringe() {
    let mut map = Map::new();
    let floor = add(&mut map, NodeKind::Brush(Brush::cuboid(Aabb::new(DVec3::new(-256.0, -256.0, -16.0), DVec3::new(256.0, 256.0, 0.0)), &tex())));
    add(&mut map, NodeKind::Brush(Brush::cuboid(Aabb::new(DVec3::new(-30.0, -30.0, 0.0), DVec3::new(30.0, 30.0, 64.0)), &tex())));
    let mut sun = Entity::new("light_environment");
    sun.set("angles", "90 0 0");
    add(&mut map, NodeKind::Entity(sun));
    let lm = run(&map, &Settings { luxel: 16.0, ..direct_only() });
    let open = seen(&lm, &map, floor, DVec3::Z, DVec3::new(-100.0, 0.0, 0.0));
    // This luxel's center lies under the box, yet it also covers the open floor beside it.
    let beside = seen(&lm, &map, floor, DVec3::Z, DVec3::new(-31.0, 0.0, 0.0));
    assert!(open > 0.9, "open floor under a noon sun: {open}");
    assert!((beside - open).abs() < 0.05, "beside the box {beside} vs {open}");
}

#[test]
fn worldspawn_sets_the_ray_bias() {
    let mut map = Map::new();
    let mut world = Entity::new("worldspawn");
    world.set("_lightmap_bias", "2");
    let mut e = map.edit();
    e.set_kind(ROOT, NodeKind::World(world)).unwrap();
    let changes = e.finish();
    map.apply(changes);
    assert_eq!(Settings::default().for_map(&map).bias, 2.0);
    assert_eq!(Settings::default().for_map(&Map::new()).bias, 0.5);
}

#[test]
fn dirt_darkens_a_corner_and_leaves_the_open_floor() {
    let mut map = Map::new();
    let walls = room(&mut map, DVec3::new(-256.0, -256.0, 0.0), DVec3::new(256.0, 256.0, 256.0));
    light(&mut map, DVec3::new(0.0, 0.0, 128.0), &[("_light", "255 255 255 255")]);
    let settings = Settings { rays: 32, ..direct_only() };
    let clean = run(&map, &settings);
    world(&mut map, &[("_dirt", "1")]);
    let dirty = run(&map, &settings.for_map(&map));
    let (corner, middle) = (DVec3::new(-244.0, -244.0, 0.0), DVec3::new(4.0, 4.0, 0.0));
    let at = |lm: &Lightmap, p: DVec3| seen(lm, &map, walls[0], DVec3::Z, p);
    assert!(at(&clean, corner) > 0.01, "the corner is lit: {}", at(&clean, corner));
    assert!(at(&dirty, corner) < at(&clean, corner) * 0.8, "dirt in the corner: {} vs {}", at(&dirty, corner), at(&clean, corner));
    assert!((at(&dirty, middle) - at(&clean, middle)).abs() < 0.01, "none in the open: {} vs {}", at(&dirty, middle), at(&clean, middle));
}

/// A floor under a noon sun with a slab floating over it, whose shadow ends at x = 4.
fn shadow_edge() -> (Map, NodeId) {
    let mut map = Map::new();
    let floor = add(&mut map, NodeKind::Brush(Brush::cuboid(Aabb::new(DVec3::new(-256.0, -256.0, -16.0), DVec3::new(256.0, 256.0, 0.0)), &tex())));
    add(&mut map, NodeKind::Brush(Brush::cuboid(Aabb::new(DVec3::new(-64.0, -128.0, 64.0), DVec3::new(4.0, 128.0, 80.0)), &tex())));
    let mut sun = Entity::new("light_environment");
    sun.set("angles", "90 0 0");
    sun.set("ambient_energy", "0");
    add(&mut map, NodeKind::Entity(sun));
    (map, floor)
}

#[test]
fn extra_samples_shade_a_luxel_the_shadow_edge_crosses() {
    let (map, floor) = shadow_edge();
    let edge = |extra: u32| {
        let lm = run(&map, &Settings { luxel: 16.0, extra, ..direct_only() });
        let at = |x: f64| seen(&lm, &map, floor, DVec3::Z, DVec3::new(x, 8.0, 0.0));
        (at(100.0), at(8.0))
    };
    let (open, one) = edge(1);
    assert!((one - open).abs() < 0.05, "one sample at the luxel's center is in the sun: {one} vs {open}");
    let (open, sixteen) = edge(4);
    assert!(sixteen > open * 0.6 && sixteen < open * 0.9, "a quarter of the luxel is in shadow: {sixteen} vs {open}");
}

#[test]
fn soft_blends_a_shadow_edge_into_its_neighbors() {
    let (map, floor) = shadow_edge();
    let edge = |soft: u32| {
        let lm = run(&map, &Settings { luxel: 16.0, soft, ..direct_only() });
        let at = |x: f64| seen(&lm, &map, floor, DVec3::Z, DVec3::new(x, 8.0, 0.0));
        (at(100.0), at(-8.0))
    };
    let (open, hard) = edge(0);
    assert!(hard < open * 0.05, "the luxel inside the shadow: {hard} vs {open}");
    let (open, soft) = edge(1);
    assert!(soft > open * 0.2 && soft < open * 0.45, "a third of its neighbors are lit: {soft} vs {open}");
}

#[test]
fn bounce_color_scale_takes_the_floor_s_color_out_of_the_bounce() {
    let mut map = Map::new();
    let walls = room(&mut map, DVec3::new(-256.0, -256.0, 0.0), DVec3::new(256.0, 256.0, 256.0));
    let mut e = Entity::new("light_spot");
    e.set("origin", "0 0 200");
    e.set("_light", "255 255 255 2000");
    e.set("angles", "-90 0 0");
    add(&mut map, NodeKind::Entity(e));
    let ceiling = |bounce_color: f64| {
        let settings = Settings { bounces: 1, rays: 64, bounce_color, ..direct_only() };
        let lm = run_with(&map, &settings, &|_| Surface { albedo: [0.8, 0.1, 0.1], emission: [0.0; 3] });
        color(&lm, &map, walls[1], -DVec3::Z, DVec3::new(100.0, 100.0, 256.0))
    };
    let red = ceiling(1.0);
    assert!(red.x > red.y * 4.0, "a red floor bounces red: {red}");
    let gray = ceiling(0.0);
    assert!(gray.x > 0.005 && (gray.x - gray.y).abs() < gray.x * 0.1, "and gray with no color kept: {gray}");
}

#[test]
fn minlight_skips_excluded_materials() {
    let mut map = Map::new();
    let walls = room(&mut map, DVec3::new(-128.0, -128.0, 0.0), DVec3::new(128.0, 128.0, 128.0));
    let dark = TexDefaults { material: "dev/Dark".into(), scale: 1.0 };
    let block = add(&mut map, NodeKind::Brush(Brush::cuboid(Aabb::new(DVec3::new(-16.0, -16.0, 32.0), DVec3::new(16.0, 16.0, 64.0)), &dark)));
    world(&mut map, &[("_minlight", "128"), ("_minlight_exclude", "sky, dev/dark")]);
    let lm = run(&map, &direct_only());
    let floor = seen(&lm, &map, walls[0], DVec3::Z, DVec3::new(100.0, 100.0, 0.0));
    assert!((floor - 128.0 / 255.0).abs() < 0.02, "minlight on the floor: {floor}");
    let top = seen(&lm, &map, block, DVec3::Z, DVec3::new(0.0, 0.0, 64.0));
    assert!(top < 1e-3, "none on the excluded material: {top}");
}

#[test]
fn a_glowing_material_lights_the_room_and_is_part_of_the_bake() {
    let mut map = Map::new();
    let walls = room(&mut map, DVec3::new(-128.0, -128.0, 0.0), DVec3::new(128.0, 128.0, 128.0));
    let glow = |energy: f32| move |_: &str| Surface { emission: [energy; 3], ..Surface::default() };
    let floor = |energy: f32| seen(&run_with(&map, &direct_only(), &glow(energy)), &map, walls[0], DVec3::Z, DVec3::new(4.0, 4.0, 0.0));
    assert!(floor(1.0) > 0.1, "glowing walls light the floor: {}", floor(1.0));
    assert!(floor(0.0) < 1e-3, "and not at no energy: {}", floor(0.0));
    let print = |energy: f32| mapcraft_light::bake_fingerprint(&map, &direct_only(), &glow(energy));
    assert_eq!(print(1.0), print(1.0));
    assert_ne!(print(1.0), print(0.0), "a bake at another energy is out of date");
}

/// Not part of `cargo test`: bakes a sunlit yard under a `light_environment` and prints what the same bake costs with
/// every mantissa bit kept, with the default kept bits, and inside a small budget. Read it with
/// `cargo test -p mapcraft-light -- --ignored --nocapture what_a_sunlit_bake_costs`.
#[test]
#[ignore]
fn what_a_sunlit_bake_costs() {
    let mut map = Map::new();
    let box_at = |map: &mut Map, min: DVec3, max: DVec3| add(map, NodeKind::Brush(Brush::cuboid(Aabb::new(min, max), &tex())));
    box_at(&mut map, DVec3::new(-2048.0, -2048.0, -16.0), DVec3::new(2048.0, 2048.0, 0.0));
    for i in 0..6 {
        let (x, y) = (-1600.0 + f64::from(i) * 560.0, -1200.0 + f64::from(i % 3) * 900.0);
        box_at(&mut map, DVec3::new(x, y, 0.0), DVec3::new(x + 200.0 + f64::from(i) * 90.0, y + 300.0, 256.0 + f64::from(i) * 120.0));
    }
    box_at(&mut map, DVec3::new(-2064.0, 1900.0, 0.0), DVec3::new(2064.0, 2064.0, 512.0));
    let mut sun = Entity::new("light_environment");
    sun.set("angles", "50 135 0");
    add(&mut map, NodeKind::Entity(sun));
    world(&mut map, &[("_lightmap_scale", "8")]);

    let asked = Settings { threads: 4, ..Settings::default() }.for_map(&map);
    let mut fitted = Settings { budget: 4 << 20, ..asked.clone() };
    fitted.fit_budget(&map);
    for (name, s) in [
        ("9 9 bits, uncapped", Settings { l0_bits: 9, band_bits: 9, budget: 0, asked_luxel: None, ..asked.clone() }),
        ("8 7 bits, uncapped", Settings { budget: 0, asked_luxel: None, ..asked.clone() }),
        ("8 7 bits, 4 MB budget", fitted),
    ] {
        let started = std::time::Instant::now();
        let (lm, warnings) = mapcraft_light::bake(&map, &|_| Surface::default(), &BTreeSet::new(), &s, &Progress::default()).unwrap();
        let bytes = mapcraft_light::write(&lm);
        println!(
            "{name}: {:.2} MB in {:.0}s, {} slices of {}, at {} units a luxel {}",
            bytes.len() as f64 / 2f64.powi(20),
            started.elapsed().as_secs_f64(),
            lm.slices.len(),
            lm.size,
            s.luxel,
            if warnings.is_empty() { String::new() } else { format!("({})", warnings.join("; ")) }
        );
    }
}

#[test]
fn worldspawn_sets_the_bake_options() {
    let mut map = Map::new();
    world(
        &mut map,
        &[("_lightmap_extra", "4"), ("_lightmap_soft", "2"), ("_bouncecolorscale", "0.25"), ("_dirt", "1"), ("_dirtdepth", "64"), ("_dirtangle", "200")],
    );
    let s = Settings::default().for_map(&map);
    assert_eq!((s.extra, s.soft, s.bounce_color), (4, 2, 0.25));
    assert_eq!(s.dirt, Some(mapcraft_light::Dirt { depth: 64.0, scale: 1.0, gain: 1.0, angle: 90.0 }));
    let plain = Settings::default().for_map(&Map::new());
    assert_eq!((plain.extra, plain.soft, plain.bounce_color, plain.dirt), (1, 0, 1.0, None));
}

#[test]
fn worldspawn_keeps_the_mantissa_bits_it_likes() {
    let bits = |keys: &[(&str, &str)]| {
        let mut map = Map::new();
        world(&mut map, keys);
        let s = Settings::default().for_map(&map);
        (s.l0_bits, s.band_bits)
    };
    assert_eq!(bits(&[("_lightmap_bits", "9 9")]), (9, 9), "every bit the texel holds");
    assert_eq!(bits(&[("_lightmap_bits", "6")]), (6, 6), "one number keeps them even");
    assert_eq!(bits(&[("_lightmap_bits", "9 4")]), (9, 4), "and out of range is pulled inside it");
    assert_eq!(bits(&[("_lightmap_bits", "12 40")]), (9, 9));
    assert_eq!(bits(&[]), (8, 7), "and the default is the coarse bands that make a sunlit map pack");
}

/// A sealed room with a bright spot that bounces once, so every luxel gets its own Monte-Carlo light and the atlas holds real
/// content. Returns it with the floor.
fn lit_room() -> (Map, NodeId) {
    let mut map = Map::new();
    let walls = room(&mut map, DVec3::new(-256.0, -256.0, 0.0), DVec3::new(256.0, 256.0, 256.0));
    let mut e = Entity::new("light_spot");
    e.set("origin", "0 0 200");
    e.set("_light", "255 255 255 2000");
    e.set("angles", "-90 0 0");
    add(&mut map, NodeKind::Entity(e));
    (map, walls[0])
}

#[test]
fn fewer_kept_bits_cost_less_light_than_file() {
    let (map, floor) = lit_room();
    let baked = |(l0, bands): (u32, u32)| {
        let lm = run(&map, &Settings { l0_bits: l0, band_bits: bands, budget: 0, bounces: 1, rays: 64, ..direct_only() });
        (mapcraft_light::write(&lm), lm)
    };
    let (exact, full) = baked((9, 9));
    let (kept, kept_lm) = baked((8, 7));
    assert!(kept.len() * 4 < exact.len() * 3, "{} KB of kept bits against {} KB of all of them", kept.len() / 1024, exact.len() / 1024);
    for p in [DVec3::new(4.0, 4.0, 0.0), DVec3::new(120.0, 0.0, 0.0), DVec3::new(-200.0, 200.0, 0.0), DVec3::new(240.0, -240.0, 0.0)] {
        let (a, b) = (seen(&full, &map, floor, DVec3::Z, p), seen(&kept_lm, &map, floor, DVec3::Z, p));
        assert!((a - b).abs() <= a.max(1e-3) * 0.03, "{p}: {b} against {a}");
    }
}

#[test]
fn a_small_budget_coarsens_the_scale_and_says_so() {
    let (mut map, _) = lit_room();
    world(&mut map, &[("_lightmap_scale", "2"), ("_lightmap_budget", "1")]);
    let fitted = Settings::default().for_map(&map);
    assert_eq!(fitted.asked_luxel, Some(2.0), "the map asked for 2 units a luxel");
    assert!(fitted.luxel > 2.0, "and a megabyte of it is more than the room holds at that scale: {}", fitted.luxel);
    let room = 1 << 20 >> 3;
    let held = mapcraft_light::charts::luxels(&map, fitted.luxel);
    assert!(held <= room, "{held} luxels against a room of {room}");

    let free = Settings { luxel: 2.0, budget: 0, asked_luxel: None, ..fitted.clone() };
    let (tight, warnings) = mapcraft_light::bake(&map, &|_| Surface::default(), &BTreeSet::new(), &fitted, &Progress::default()).unwrap();
    assert!(warnings.iter().any(|w| w.contains("lightmap scale went from 2")), "{warnings:?}");
    let loose = mapcraft_light::bake(&map, &|_| Surface::default(), &BTreeSet::new(), &free, &Progress::default()).unwrap().0;
    let (small, whole) = (mapcraft_light::write(&tight).len(), mapcraft_light::write(&loose).len());
    assert!(small * 2 < whole, "the fitted bake holds {small} bytes against {whole} for every luxel it asked for");
    let mut ask = lit_room().0;
    world(&mut ask, &[("_lightmap_scale", "2"), ("_lightmap_budget", "0")]);
    let uncapped = Settings::default().for_map(&ask);
    assert_eq!((uncapped.luxel, uncapped.asked_luxel, uncapped.budget), (2.0, None, 0), "and a budget of 0 leaves the asked-for scale alone");
}
