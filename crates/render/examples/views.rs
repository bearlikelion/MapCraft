//! Renders a map from the four standard views into one PNG: `cargo run -p mapcraft-render --example views -- map.mcmap out.png [lit]`.

use std::collections::BTreeSet;

use mapcraft_geom::DVec3;
use mapcraft_render::{Camera, NoStyle, RenderMode, Renderer, View, build_scene};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(input), Some(output)) = (args.first(), args.get(1)) else {
        eprintln!("usage: views <map.mcmap> <out.png> [lit]");
        std::process::exit(2);
    };
    let file = mapcraft_format::load(std::path::Path::new(input)).unwrap();
    let (map, _) = mapcraft_map::mirror::expand(&file.map);
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    let mut renderer = Renderer::new(&device, &queue);
    let checker: Vec<u8> =
        (0..64 * 64).flat_map(|i: u32| if ((i % 64) / 8 + (i / 64) / 8).is_multiple_of(2) { [230, 120, 30, 255] } else { [120, 60, 15, 255] }).collect();
    renderer.upload_material("ramps-orange1", 64, 64, &checker);
    let selection: BTreeSet<_> = map.nodes().filter(|(_, n)| n.kind.entity().is_some_and(|e| e.classname != "worldspawn")).map(|(id, _)| id).collect();
    let scene = build_scene(&map, &selection, &BTreeSet::new(), &|m| renderer.material_size(m), &NoStyle, true);
    renderer.set_scene(&scene);

    let (w, h) = (480u32, 360u32);
    let mut sheet = image::RgbaImage::new(w * 2, h * 2);
    for (i, view) in View::ALL.into_iter().enumerate() {
        let mut cam = Camera::new(view);
        if view == View::Perspective {
            cam.position = DVec3::new(-200.0, -180.0, 180.0);
            cam.pitch = -30.0;
        }
        let target = renderer.target([w, h]);
        let mode = if args.get(2).is_some_and(|m| m == "lit") { RenderMode::Lit } else { RenderMode::Textured };
        renderer.render(&target, &cam, mode, Some(64.0), &[]);
        let row = (w * 4).div_ceil(256) * 256;
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(row * h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            target.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        queue.submit([enc.finish()]);
        buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let data = buf.slice(..).get_mapped_range().unwrap();
        let (ox, oy) = ((i as u32 % 2) * w, (i as u32 / 2) * h);
        for y in 0..h {
            for x in 0..w {
                let o = (y * row + x * 4) as usize;
                sheet.put_pixel(ox + x, oy + y, image::Rgba([data[o], data[o + 1], data[o + 2], 255]));
            }
        }
    }
    sheet.save(output).unwrap();
    println!("wrote {output}");
}
