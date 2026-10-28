//! Renders surfaces on a real adapter and compares pixels with the CPU's math; skipped without a GPU.

use std::collections::BTreeSet;

use mapcraft_geom::noise::macro_variation;
use mapcraft_geom::{Aabb, DVec3};
use mapcraft_map::{AntiTile, Brush, Displacement, Map, NodeKind, ROOT, TexDefaults};
use mapcraft_render::{Camera, NoStyle, RenderMode, Renderer, SceneData, View, build_scene};

const SIZE: u32 = 63;

fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
}

fn pixel(rgba: &[u8], x: u32, y: u32) -> [u8; 3] {
    let o = ((y * SIZE + x) * 4) as usize;
    [rgba[o], rgba[o + 1], rgba[o + 2]]
}

fn to_linear(c: u8) -> f64 {
    let c = f64::from(c) / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn to_srgb(c: f64) -> f64 {
    let c = c.clamp(0.0, 1.0);
    255.0 * if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// A big floor of material "a" counting `size` texels per repeat, its top blending in half of "b" when `blend` holds.
fn floor(mode: AntiTile, size: [u32; 2], blend: bool) -> SceneData {
    let mut map = Map::new();
    let mut e = map.edit();
    let mut floor =
        Brush::cuboid(Aabb::new(DVec3::new(-4096.0, -4096.0, -16.0), DVec3::new(4096.0, 4096.0, 0.0)), &TexDefaults { material: "a".into(), scale: 1.0 });
    floor.faces.iter_mut().for_each(|f| f.set_anti_tile(mode));
    if blend {
        let solid = floor.solid().unwrap();
        let (top, w) = floor.faces.iter_mut().zip(&solid.faces).find(|(f, _)| f.plane().unwrap().normal.z > 0.9).unwrap();
        let mut d = Displacement::new(1, w.as_ref().unwrap().0[0]);
        d.set_blend(vec![0.5; 9]);
        d.blend_material = Some("b".into());
        top.disp = Some(d);
    }
    e.insert(ROOT, NodeKind::Brush(floor)).unwrap();
    map.apply(e.finish());
    build_scene(&map, &BTreeSet::new(), &BTreeSet::new(), &|_| Some(size), &NoStyle, false, None)
}

/// Looks almost straight down from `height`, with +X to the right.
fn top_camera(height: f64) -> Camera {
    let mut camera = Camera::new(View::Perspective);
    (camera.yaw, camera.pitch, camera.position) = (90.0, -89.0, DVec3::new(0.0, 0.0, height));
    camera
}

/// Looks down at 60 degrees so the center pixel sees the floor at (-600, 0, 0).
fn floor_camera() -> Camera {
    let mut camera = Camera::new(View::Perspective);
    (camera.yaw, camera.pitch) = (0.0, -60.0);
    let ahead = camera.forward() * (512.0 / -camera.forward().z);
    camera.position = DVec3::new(-600.0, 0.0, 512.0) - ahead.with_z(0.0);
    camera
}

#[test]
fn variation_matches_the_cpu_noise() {
    let Some((device, queue)) = gpu() else {
        println!("skipped: no GPU adapter");
        return;
    };
    let scene = |mode: AntiTile| floor(mode, [4, 4], false);

    let color = [200u8, 150, 100, 255];
    let mut renderer = Renderer::new(&device, &queue);
    renderer.upload_material("a", 4, 4, &color.repeat(16));
    renderer.set_scene(&scene(AntiTile::Macro));

    // The center pixel looks at a point where the noise darkens the texture well below 1.
    let camera = floor_camera();
    let hit = camera.position + camera.forward() * (512.0 / -camera.forward().z);
    let variation = macro_variation(hit);
    assert!(variation < 0.9, "{variation}");

    let target = renderer.target([SIZE, SIZE]);
    renderer.render(&target, &camera, RenderMode::Textured, None, &[]);
    let rgba = renderer.read_rgba(&target).unwrap();
    let light = DVec3::new(0.3, 0.5, 0.8).normalize();
    let shade = 0.6 + 0.4 * light.z;
    let got = pixel(&rgba, SIZE / 2, SIZE / 2);
    for (k, c) in got.iter().enumerate() {
        let want = to_srgb(to_linear(color[k]) * variation * shade);
        assert!((f64::from(*c) - want).abs() <= 2.0, "channel {k}: {got:?} against {want}");
    }

    renderer.render(&target, &camera, RenderMode::Flat, None, &[]);
    let rgba = renderer.read_rgba(&target).unwrap();
    let corners = [(4, 4), (SIZE / 2, SIZE / 2), (SIZE - 5, SIZE - 5), (4, SIZE - 5)].map(|(x, y)| pixel(&rgba, x, y));
    assert!(corners.iter().all(|c| c == &corners[0]), "Flat mode draws no variation: {corners:?}");

    renderer.render(&target, &camera, RenderMode::Lit, None, &[]);
    let varied = pixel(&renderer.read_rgba(&target).unwrap(), SIZE / 2, SIZE / 2);
    renderer.set_scene(&scene(AntiTile::Off));
    renderer.render(&target, &camera, RenderMode::Lit, None, &[]);
    let plain = pixel(&renderer.read_rgba(&target).unwrap(), SIZE / 2, SIZE / 2);
    assert!(varied.iter().zip(plain).all(|(a, b)| *a < b), "Lit mode darkens with the noise: {varied:?} against {plain:?}");
}

#[test]
fn blends_draw_both_layers_and_the_weight_overlay() {
    let Some((device, queue)) = gpu() else {
        println!("skipped: no GPU adapter");
        return;
    };
    let mut map = Map::new();
    let mut e = map.edit();
    for (x, weight) in [(-256.0, 0.0), (0.0, 1.0)] {
        let tex = TexDefaults { material: "a".into(), scale: 1.0 };
        let mut b = Brush::cuboid(Aabb::new(DVec3::new(x, -256.0, -16.0), DVec3::new(x + 256.0, 256.0, 0.0)), &tex);
        let solid = b.solid().unwrap();
        let (top, w) = b.faces.iter_mut().zip(&solid.faces).find(|(f, _)| f.plane().unwrap().normal.z > 0.9).unwrap();
        let mut d = Displacement::new(1, w.as_ref().unwrap().0[0]);
        d.set_blend(vec![weight; 9]);
        d.blend_material = Some("b".into());
        top.disp = Some(d);
        e.insert(ROOT, NodeKind::Brush(b)).unwrap();
    }
    map.apply(e.finish());

    let mut renderer = Renderer::new(&device, &queue);
    renderer.upload_material("a", 4, 4, &[255, 0, 0, 255].repeat(16));
    renderer.upload_material("b", 4, 4, &[0, 0, 255, 255].repeat(16));
    renderer.set_scene(&build_scene(&map, &BTreeSet::new(), &BTreeSet::new(), &|_| Some([4, 4]), &NoStyle, false, None));
    // The left floor has weight 0, the right one 1.
    let camera = top_camera(300.0);
    let target = renderer.target([SIZE, SIZE]);
    let shot = |renderer: &Renderer| {
        renderer.render(&target, &camera, RenderMode::Textured, None, &[]);
        let rgba = renderer.read_rgba(&target).unwrap();
        (pixel(&rgba, SIZE / 4, SIZE / 2), pixel(&rgba, 3 * SIZE / 4, SIZE / 2))
    };
    let (left, right) = shot(&renderer);
    assert!(left[0] > 150 && left[1] < 10 && left[2] < 10, "weight 0 shows the face's own material: {left:?}");
    assert!(right[2] > 150 && right[0] < 10 && right[1] < 10, "weight 1 shows the second layer: {right:?}");

    renderer.set_weight_overlay(0.5);
    let (washed_left, washed_right) = shot(&renderer);
    assert_eq!(washed_left, left, "unpainted vertices show no overlay");
    assert!(washed_right[1] > right[1] + 100, "painted ones wash toward cyan: {right:?} to {washed_right:?}");
}

#[test]
fn hex_tiling_keeps_a_solid_color_and_pixel_textures_fall_back() {
    let Some((device, queue)) = gpu() else {
        println!("skipped: no GPU adapter");
        return;
    };
    let camera = floor_camera();
    // Whether Hex draws as Macro, with materials "a" and "b" on a floor that blends them when `blend` holds.
    let same = |renderer: &mut Renderer, blend: bool, mode: RenderMode, a: &[u8], b: &[u8]| {
        let [hex, varied] = [AntiTile::Hex, AntiTile::Macro].map(|anti_tile| {
            renderer.upload_material("a", 4, 4, a);
            renderer.upload_material("b", 4, 4, b);
            renderer.set_scene(&floor(anti_tile, [256, 256], blend));
            let target = renderer.target([SIZE, SIZE]);
            renderer.render(&target, &camera, mode, None, &[]);
            renderer.read_rgba(&target).unwrap()
        });
        hex.iter().zip(&varied).all(|(x, y)| x.abs_diff(*y) <= 1)
    };
    let mut renderer = Renderer::new(&device, &queue);
    let solid = [90u8, 160, 60, 255].repeat(16);
    let pattern: Vec<u8> = (0..16u8).flat_map(|i| [i * 16, 255 - i * 16, (i % 4) * 80, 255]).collect();
    assert!(same(&mut renderer, false, RenderMode::Lit, &solid, &solid), "hex tiles of one color add up to it");
    assert!(!same(&mut renderer, false, RenderMode::Lit, &pattern, &solid), "hex tiling moves the pattern");
    assert!(!same(&mut renderer, true, RenderMode::Textured, &solid, &pattern), "a blend hex tiles its second layer");
    renderer.set_pixelated(true);
    assert!(same(&mut renderer, false, RenderMode::Lit, &pattern, &solid), "pixel textures draw hex as variation");
    assert!(same(&mut renderer, true, RenderMode::Textured, &solid, &pattern), "and so does a blend with a pixel layer");
}

#[test]
fn hex_tiles_turn_a_wide_texture_without_squashing_it() {
    let Some((device, queue)) = gpu() else {
        println!("skipped: no GPU adapter");
        return;
    };
    let (w, h) = (64u32, 32u32);
    let rings: Vec<u8> = (0..w * h)
        .flat_map(|i| {
            let (x, y) = (f64::from(i % w) + 0.5 - f64::from(w / 2), f64::from(i / w) + 0.5 - f64::from(h / 2));
            let v = if ((x.hypot(y) / 4.0) as u32).is_multiple_of(2) { 255 } else { 0 };
            [v, v, v, 255]
        })
        .collect();
    let mut renderer = Renderer::new(&device, &queue);
    renderer.upload_material("a", w, h, &rings);
    renderer.set_scene(&floor(AntiTile::Hex, [w, h], false));
    const SHOT: u32 = 512;
    let target = renderer.target([SHOT, SHOT]);
    renderer.render(&target, &top_camera(300.0), RenderMode::Textured, None, &[]);
    let rgba = renderer.read_rgba(&target).unwrap();
    let white = |x: u32, y: u32| rgba[((y * SHOT + x) * 4) as usize] > 128;
    let (mut across, mut down) = (0u32, 0u32);
    for y in 0..SHOT - 1 {
        for x in 0..SHOT - 1 {
            across += u32::from(white(x, y) != white(x + 1, y));
            down += u32::from(white(x, y) != white(x, y + 1));
        }
    }
    // A squashed turn crowds edges along one axis, about twice as many as along the other.
    let ratio = f64::from(across) / f64::from(down);
    assert!((0.8..1.25).contains(&ratio), "round rings cross as many pixel rows as columns: {across} across, {down} down");
}
