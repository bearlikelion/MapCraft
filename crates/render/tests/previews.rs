//! Renders model previews on a real adapter; skipped without a GPU.

use mapcraft_geom::DVec3;
use mapcraft_render::{MeshPart, Model, Renderer, mesh_model};

const SIDE: u32 = 64;

fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
}

fn cuboid(min: DVec3, max: DVec3) -> Model {
    let pick = |i: u32, bit: u32, lo: f64, hi: f64| if i & bit == 0 { lo } else { hi };
    let positions = (0..8).map(|i| DVec3::new(pick(i, 1, min.x, max.x), pick(i, 2, min.y, max.y), pick(i, 4, min.z, max.z))).collect();
    let indices = vec![0, 2, 1, 1, 2, 3, 4, 5, 6, 5, 7, 6, 0, 1, 4, 1, 5, 4, 2, 6, 3, 3, 6, 7, 0, 4, 2, 2, 4, 6, 1, 3, 5, 3, 7, 5];
    // Varying, since a sampler that filters anisotropically can return nothing for a constant coordinate.
    let uvs = (0..8).map(|i| [(i & 1) as f32, ((i >> 1) & 1) as f32]).collect();
    mesh_model("box", vec![MeshPart { texture: None, positions, uvs, indices }], Vec::new()).unwrap()
}

/// The smallest pixel rectangle holding every covered pixel, as [left, top, right, bottom].
fn covered(rgba: &[u8]) -> Option<[u32; 4]> {
    let mut out: Option<[u32; 4]> = None;
    for (i, [.., alpha]) in rgba.as_chunks::<4>().0.iter().enumerate() {
        if *alpha > 0 {
            let (x, y) = (i as u32 % SIDE, i as u32 / SIDE);
            let r = out.get_or_insert([x, y, x, y]);
            *r = [r[0].min(x), r[1].min(y), r[2].max(x), r[3].max(y)];
        }
    }
    out
}

#[test]
fn a_model_preview_frames_the_model_on_a_clear_background() {
    let Some((device, queue)) = gpu() else {
        println!("skipped: no GPU adapter");
        return;
    };
    let renderer = Renderer::new(&device, &queue);
    let boxes = [
        (DVec3::splat(-8.0), DVec3::splat(8.0)),
        (DVec3::new(5000.0, -3000.0, 200.0), DVec3::new(5400.0, -2000.0, 1200.0)),
        (DVec3::new(0.0, 0.0, 0.0), DVec3::new(2.0, 2.0, 0.5)),
    ];
    for (min, max) in boxes {
        let rgba = renderer.model_preview(&cuboid(min, max), SIDE).unwrap();
        assert_eq!(rgba.len(), (SIDE * SIDE * 4) as usize);
        let [left, top, right, bottom] = covered(&rgba).unwrap_or_else(|| panic!("the preview of {min}..{max} is blank"));
        assert!(left > 0 && top > 0 && right < SIDE - 1 && bottom < SIDE - 1, "{min}..{max} is cut off: {left} {top} {right} {bottom}");
        assert!(right - left > SIDE / 2 && bottom - top > SIDE / 3, "{min}..{max} is too small to read: {left} {top} {right} {bottom}");
        let middle = ((SIDE / 2 * SIDE + SIDE / 2) * 4) as usize;
        assert!(rgba[middle + 3] == 255 && rgba[middle] > 100, "the middle shows the lit model: {:?}", &rgba[middle..middle + 4]);
        assert_eq!(rgba[3], 0, "the background is clear");
    }
}
