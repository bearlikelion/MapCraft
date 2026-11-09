//! Pictures of the scene from the standard views, for agents and tools that have no window.

use mapcraft_geom::{Aabb, DVec3};

use crate::{Camera, RenderMode, Renderer, View};

/// The views a screenshot request names: all, 3d, top, front or side.
pub fn named_views(name: &str) -> Option<&'static [View]> {
    match name {
        "all" => Some(&View::ALL),
        "3d" => Some(&[View::Perspective]),
        "top" => Some(&[View::Top]),
        "front" => Some(&[View::Front]),
        "side" => Some(&[View::Side]),
        _ => None,
    }
}

/// Centers `cam` on `b`; a 2D view zooms so the box fills `px`, in the units of its zoom.
pub fn frame_camera(cam: &mut Camera, b: &Aabb, px: f64) {
    let c = b.center();
    if cam.is_2d() {
        let (r, u, _) = cam.view.axes();
        cam.center = mapcraft_geom::DVec2::new(c.dot(r), c.dot(u));
        cam.zoom = (px.max(100.0) / (b.size().max_element().max(64.0) * 1.3)).clamp(1.0 / 256.0, 64.0);
    } else {
        // Backs off until the box's bounding sphere fits the field of view.
        let radius = b.size().length().max(64.0) * 0.5;
        cam.position = c - cam.forward() * radius / (cam.fov_y.to_radians() * 0.5).sin().max(0.1);
    }
}

/// What a sheet shows and how it is drawn.
pub struct Shot<'a> {
    pub views: &'a [View],
    pub bounds: Aabb,
    /// The 3D view looks from here at `look_at` instead of framing `bounds`.
    pub eye: Option<DVec3>,
    pub look_at: DVec3,
    pub size: [u32; 2],
    pub mode: RenderMode,
    pub grid: Option<f64>,
}

/// Renders each view of the current scene into one picture, two columns when there are several.
pub fn render_sheet(renderer: &Renderer, shot: &Shot) -> Option<image::RgbaImage> {
    let cols = if shot.views.len() > 1 { 2 } else { 1 };
    let (cw, ch) = (shot.size[0] / cols, shot.size[1] / cols);
    let mut sheet = image::RgbaImage::new(cw * cols, ch * cols);
    for (i, view) in (0u32..).zip(shot.views) {
        let mut cam = Camera::new(*view);
        frame_camera(&mut cam, &shot.bounds, f64::from(cw.min(ch)));
        if let (View::Perspective, Some(eye)) = (view, shot.eye) {
            let d = shot.look_at - eye;
            cam.position = eye;
            cam.yaw = d.y.atan2(d.x).to_degrees();
            cam.pitch = d.z.atan2(d.truncate().length()).to_degrees();
        }
        let target = renderer.target([cw, ch]);
        renderer.render(&target, &cam, shot.mode, shot.grid, &[]);
        let img = image::RgbaImage::from_raw(cw, ch, renderer.read_rgba(&target)?)?;
        image::imageops::replace(&mut sheet, &img, i64::from(i % cols * cw), i64::from(i / cols * ch));
    }
    Some(sheet)
}

/// A renderer on the first GPU adapter, drawing offscreen; `None` when there is no usable adapter.
pub fn headless_renderer() -> Option<Renderer> {
    let adapter = pollster::block_on(wgpu::Instance::default().request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
    Some(Renderer::new(&device, &queue))
}
