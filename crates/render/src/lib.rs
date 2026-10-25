//! wgpu renderer for MapCraft viewports: each viewport draws into its own offscreen texture.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod camera;
mod scene;

use std::collections::HashMap;

use mapcraft_geom::{DVec2, DVec3};
use wgpu::util::DeviceExt;

pub use camera::{Camera, View};
pub use scene::{EntityStyle, LineVertex, MeshVertex, NoStyle, SELECTED_EDGE, SceneData, WORLD_EDGE, box_lines, build_scene, is_tool_material};

pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SAMPLES: u32 = 4;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderMode {
    #[default]
    Textured,
    Flat,
    Wireframe,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    light_dir: [f32; 4],
}

struct Material {
    bind_group: wgpu::BindGroup,
    size: [u32; 2],
}

struct Batch {
    material: String,
    buffer: wgpu::Buffer,
    count: u32,
}

/// One viewport's render target; `view` is what the UI displays.
pub struct Target {
    pub size: [u32; 2],
    msaa: wgpu::TextureView,
    depth: wgpu::TextureView,
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    globals: wgpu::Buffer,
    globals_bind: wgpu::BindGroup,
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    globals_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    mesh_pipeline: wgpu::RenderPipeline,
    translucent_pipeline: wgpu::RenderPipeline,
    line_pipeline: wgpu::RenderPipeline,
    overlay_pipeline: wgpu::RenderPipeline,
    materials: HashMap<String, Material>,
    white: wgpu::BindGroup,
    missing: wgpu::BindGroup,
    batches: Vec<Batch>,
    edges: Option<(wgpu::Buffer, u32)>,
    selected_edges: Option<(wgpu::Buffer, u32)>,
    translucent: Option<(wgpu::Buffer, u32)>,
}

fn vertex_layout<'a>(stride: usize, attrs: &'a [wgpu::VertexAttribute]) -> wgpu::VertexBufferLayout<'a> {
    wgpu::VertexBufferLayout { array_stride: stride as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: attrs }
}

impl Renderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mapcraft shaders"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders.wgsl").into()),
        });
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("material"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("material"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let mesh_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mesh"),
            bind_group_layouts: &[Some(&globals_layout), Some(&texture_layout)],
            immediate_size: 0,
        });
        let line_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("line"),
            bind_group_layouts: &[Some(&globals_layout)],
            immediate_size: 0,
        });
        let mesh_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32x4];
        let line_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4];
        // `depth` is (write, test): overlays skip the test, see-through faces skip the write.
        let pipeline =
            |label: &str, layout: &wgpu::PipelineLayout, vs: &str, fs: &str, buffers: &[Option<wgpu::VertexBufferLayout>], topology, depth: (bool, bool)| {
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(label),
                    layout: Some(layout),
                    vertex: wgpu::VertexState { module: &shader, entry_point: Some(vs), compilation_options: Default::default(), buffers },
                    primitive: wgpu::PrimitiveState { topology, cull_mode: None, ..Default::default() },
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: DEPTH_FORMAT,
                        depth_write_enabled: Some(depth.0),
                        depth_compare: Some(if depth.1 { wgpu::CompareFunction::LessEqual } else { wgpu::CompareFunction::Always }),
                        stencil: Default::default(),
                        bias: Default::default(),
                    }),
                    multisample: wgpu::MultisampleState { count: SAMPLES, ..Default::default() },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some(fs),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: COLOR_FORMAT,
                            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    multiview_mask: None,
                    cache: None,
                })
            };
        let mesh_buffers = [Some(vertex_layout(std::mem::size_of::<MeshVertex>(), &mesh_attrs))];
        let line_buffers = [Some(vertex_layout(std::mem::size_of::<LineVertex>(), &line_attrs))];
        let mesh_pipeline = pipeline("mesh", &mesh_layout, "vs_mesh", "fs_mesh", &mesh_buffers, wgpu::PrimitiveTopology::TriangleList, (true, true));
        let translucent_pipeline =
            pipeline("translucent", &mesh_layout, "vs_mesh", "fs_mesh", &mesh_buffers, wgpu::PrimitiveTopology::TriangleList, (false, true));
        let line_pipeline = pipeline("line", &line_layout, "vs_line", "fs_line", &line_buffers, wgpu::PrimitiveTopology::LineList, (true, true));
        let overlay_pipeline = pipeline("overlay", &line_layout, "vs_line", "fs_line", &line_buffers, wgpu::PrimitiveTopology::LineList, (false, false));

        let white = texture_bind_group(device, queue, &texture_layout, &sampler, 1, 1, &[255, 255, 255, 255]);
        let checker: Vec<u8> = (0..64u32).flat_map(|i| if (i % 8 < 4) ^ (i / 8 < 4) { [255, 0, 255, 255] } else { [0, 0, 0, 255] }).collect();
        let missing = texture_bind_group(device, queue, &texture_layout, &sampler, 8, 8, &checker);
        Self {
            device: device.clone(),
            queue: queue.clone(),
            white,
            missing,
            globals_layout,
            texture_layout,
            sampler,
            mesh_pipeline,
            translucent_pipeline,
            line_pipeline,
            overlay_pipeline,
            materials: HashMap::new(),
            batches: Vec::new(),
            edges: None,
            selected_edges: None,
            translucent: None,
        }
    }

    /// Uploads a material's RGBA8 pixels; a size that does not match the data is ignored.
    pub fn upload_material(&mut self, name: &str, w: u32, h: u32, rgba: &[u8]) -> bool {
        let max = self.device.limits().max_texture_dimension_2d;
        let expected = u64::from(w) * u64::from(h) * 4;
        if w == 0 || h == 0 || w > max || h > max || rgba.len() as u64 != expected {
            return false;
        }
        let bind_group = texture_bind_group(&self.device, &self.queue, &self.texture_layout, &self.sampler, w, h, rgba);
        self.materials.insert(name.to_string(), Material { bind_group, size: [w, h] });
        true
    }

    pub fn material_size(&self, name: &str) -> Option<[u32; 2]> {
        self.materials.get(name).map(|m| m.size)
    }

    pub fn has_material(&self, name: &str) -> bool {
        self.materials.contains_key(name)
    }

    pub fn set_scene(&mut self, scene: &SceneData) {
        let buffer = |label: &str, data: &[u8]| {
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents: data, usage: wgpu::BufferUsages::VERTEX })
        };
        self.batches = scene
            .batches
            .iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(m, v)| Batch { material: m.clone(), buffer: buffer("batch", bytemuck::cast_slice(v)), count: v.len() as u32 })
            .collect();
        let lines = |v: &[LineVertex]| (!v.is_empty()).then(|| (buffer("lines", bytemuck::cast_slice(v)), v.len() as u32));
        self.edges = lines(&scene.edges);
        self.selected_edges = lines(&scene.selected_edges);
        self.translucent =
            (!scene.translucent.is_empty()).then(|| (buffer("translucent", bytemuck::cast_slice(&scene.translucent)), scene.translucent.len() as u32));
    }

    pub fn target(&self, size: [u32; 2]) -> Target {
        let size = [size[0].clamp(1, 8192), size[1].clamp(1, 8192)];
        let extent = wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 };
        let tex = |label, format, samples, usage| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: extent,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let msaa = tex("viewport msaa", COLOR_FORMAT, SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT).create_view(&Default::default());
        let depth = tex("viewport depth", DEPTH_FORMAT, SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT).create_view(&Default::default());
        let texture =
            tex("viewport", COLOR_FORMAT, 1, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC);
        let view = texture.create_view(&Default::default());
        let globals = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &self.globals_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() }],
        });
        Target { size, msaa, depth, texture, view, globals, globals_bind }
    }

    /// Draws the scene into `target`; `grid` is the 2D grid spacing, `overlay` lines draw on top.
    pub fn render(&self, target: &Target, camera: &Camera, mode: RenderMode, grid: Option<f64>, overlay: &[LineVertex]) {
        let size = DVec2::new(f64::from(target.size[0]), f64::from(target.size[1]));
        let light = DVec3::new(0.3, 0.5, 0.8).normalize().as_vec3();
        let globals = Globals { view_proj: camera.view_proj(size).as_mat4().to_cols_array_2d(), light_dir: [light.x, light.y, light.z, 0.0] };
        self.queue.write_buffer(&target.globals, 0, bytemuck::bytes_of(&globals));

        let mut extra = Vec::new();
        if let (Some(g), true) = (grid, camera.is_2d()) {
            grid_lines(&mut extra, camera, size, g);
        }
        let grid_buf = (!extra.is_empty()).then(|| {
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("grid"),
                contents: bytemuck::cast_slice(&extra),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
        let overlay_buf = (!overlay.is_empty()).then(|| {
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("overlay"),
                contents: bytemuck::cast_slice(overlay),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("viewport") });
        {
            let bg = if camera.is_2d() { wgpu::Color::BLACK } else { wgpu::Color { r: 0.02, g: 0.025, b: 0.035, a: 1.0 } };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewport"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.msaa,
                    depth_slice: None,
                    resolve_target: Some(&target.view),
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(bg), store: wgpu::StoreOp::Discard },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &target.globals_bind, &[]);
            if let Some(b) = &grid_buf {
                pass.set_pipeline(&self.overlay_pipeline);
                pass.set_vertex_buffer(0, b.slice(..));
                pass.draw(0..extra.len() as u32, 0..1);
            }
            if !camera.is_2d() && mode != RenderMode::Wireframe {
                pass.set_pipeline(&self.mesh_pipeline);
                for batch in &self.batches {
                    let bind = match (mode, batch.material.as_str()) {
                        (RenderMode::Flat, _) | (_, "") => &self.white,
                        (_, m) => self.materials.get(m).map_or(&self.missing, |t| &t.bind_group),
                    };
                    pass.set_bind_group(1, bind, &[]);
                    pass.set_vertex_buffer(0, batch.buffer.slice(..));
                    pass.draw(0..batch.count, 0..1);
                }
            }
            if let Some((b, n)) = self.translucent.as_ref().filter(|_| !camera.is_2d() && mode != RenderMode::Wireframe) {
                pass.set_pipeline(&self.translucent_pipeline);
                pass.set_bind_group(1, &self.white, &[]);
                pass.set_vertex_buffer(0, b.slice(..));
                pass.draw(0..*n, 0..1);
            }
            if let Some((b, n)) = &self.edges
                && (camera.is_2d() || mode == RenderMode::Wireframe)
            {
                pass.set_pipeline(if camera.is_2d() { &self.overlay_pipeline } else { &self.line_pipeline });
                pass.set_vertex_buffer(0, b.slice(..));
                pass.draw(0..*n, 0..1);
            }
            pass.set_pipeline(&self.overlay_pipeline);
            for (b, n) in self.selected_edges.iter().chain(overlay_buf.as_ref().map(|b| (b.clone(), overlay.len() as u32)).as_ref()) {
                pass.set_vertex_buffer(0, b.slice(..));
                pass.draw(0..*n, 0..1);
            }
        }
        self.queue.submit([encoder.finish()]);
    }
}

fn texture_bind_group(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    w: u32,
    h: u32,
    rgba: &[u8],
) -> wgpu::BindGroup {
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("material"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        rgba,
    );
    let view = texture.create_view(&Default::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("material"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    })
}

/// Grid lines covering a 2D view; spacing doubles until lines are at least 6 pixels apart.
fn grid_lines(out: &mut Vec<LineVertex>, camera: &Camera, size: DVec2, grid: f64) {
    if !grid.is_finite() || grid <= 0.0 {
        return;
    }
    let mut step = grid;
    while step * camera.zoom < 6.0 {
        step *= 2.0;
    }
    let (r, u, _) = camera.view.axes();
    let half = size / (2.0 * camera.zoom);
    let (h0, h1) = (camera.center.x - half.x, camera.center.x + half.x);
    let (v0, v1) = (camera.center.y - half.y, camera.center.y + half.y);
    let color = |c: f64| {
        if c == 0.0 {
            [0.0, 0.25, 0.35, 1.0]
        } else if (c / 1024.0).fract() == 0.0 {
            [0.15, 0.08, 0.0, 1.0]
        } else if (c / (step * 8.0)).fract() == 0.0 {
            [0.06, 0.06, 0.06, 1.0]
        } else {
            [0.025, 0.025, 0.025, 1.0]
        }
    };
    let mut h = (h0 / step).floor() * step;
    while h <= h1 {
        out.push(LineVertex::new(r * h + u * v0, color(h)));
        out.push(LineVertex::new(r * h + u * v1, color(h)));
        h += step;
    }
    let mut v = (v0 / step).floor() * step;
    while v <= v1 {
        out.push(LineVertex::new(r * h0 + u * v, color(v)));
        out.push(LineVertex::new(r * h1 + u * v, color(v)));
        v += step;
    }
}
