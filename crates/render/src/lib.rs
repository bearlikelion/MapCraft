//! wgpu renderer for MapCraft viewports: each viewport draws into its own offscreen texture.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod camera;
mod model;
mod scene;

use std::collections::HashMap;

use mapcraft_geom::{DVec2, DVec3, DVec4};
use wgpu::util::DeviceExt;

pub use camera::{Camera, View};
pub use model::{MAX_MODEL_BYTES, Model, ModelPart, Picture, entity_transform, load_gltf, placed_bounds};
pub use scene::{
    EntityStyle, Light, LineVertex, MeshVertex, NoStyle, SELECTED_EDGE, SceneData, WORLD_EDGE, box_lines, build_scene, entity_bounds, is_tool_material,
};

pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DISPLAY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SAMPLES: u32 = 4;
/// Lights nearest the camera that the Lit mode shades with; the shader loops over each per pixel.
const MAX_LIGHTS: usize = 64;
/// Linear light every face gets in the Lit mode, so unlit corners stay readable.
const MIN_AMBIENT: f32 = 0.08;
/// Lit's stand-in rig for maps without lights: a warm key sun, a cool fill sun opposite it, and a sky and ground.
const RIG_SUNS: [([f32; 3], [f32; 3]); 2] = [([0.3, 0.5, 0.8], [0.7, 0.66, 0.6]), ([-0.5, -0.8, 0.5], [0.2, 0.22, 0.26])];
const RIG_SKY: [f32; 3] = [0.4, 0.43, 0.48];
const RIG_GROUND: [f32; 3] = [0.14, 0.12, 0.1];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderMode {
    #[default]
    Textured,
    Flat,
    Wireframe,
    /// Textured with physically based shading from the map's light entities, or a default rig without any.
    Lit,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    light_dir: [f32; 4],
    /// The Lit mode's ambient color from above; w counts the lights in use.
    ambient: [f32; 4],
    /// The Lit mode's ambient color from below.
    ground: [f32; 4],
    eye: [f32; 4],
    lights: [Light; MAX_LIGHTS],
}

struct Material {
    bind_group: wgpu::BindGroup,
    size: [u32; 2],
    albedo: wgpu::TextureView,
}

struct Batch {
    material: String,
    buffer: wgpu::Buffer,
    count: u32,
}

/// One viewport's render target. Rendering writes `view`, an sRGB view; `display` reads the same
/// bytes without decoding them, which is what egui expects of the textures it shows.
pub struct Target {
    pub size: [u32; 2],
    msaa: wgpu::TextureView,
    depth: wgpu::TextureView,
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub display: wgpu::TextureView,
    globals: wgpu::Buffer,
    globals_bind: wgpu::BindGroup,
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    globals_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// 1x1 maps for materials without their own: a flat normal, and fully rough, unoccluded and not metallic.
    flat: wgpu::TextureView,
    rough: wgpu::TextureView,
    mesh_pipeline: wgpu::RenderPipeline,
    lit_pipeline: wgpu::RenderPipeline,
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
    lights: Vec<Light>,
    ambient: [f32; 3],
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
        let image = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("material"),
            entries: &[
                image(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                image(2),
                image(3),
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
        let mesh_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32x4, 4 => Float32x4];
        let line_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4];
        // `depth` is (write, test): overlays skip the test, see-through faces skip the write.
        let pipeline = |label: &str,
                        layout: &wgpu::PipelineLayout,
                        vs: &str,
                        fs: &str,
                        buffers: &[Option<wgpu::VertexBufferLayout>],
                        topology,
                        depth: (bool, bool),
                        bias: wgpu::DepthBiasState| {
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
                    bias,
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
        let none = wgpu::DepthBiasState::default();
        let mesh_pipeline = pipeline("mesh", &mesh_layout, "vs_mesh", "fs_mesh", &mesh_buffers, wgpu::PrimitiveTopology::TriangleList, (true, true), none);
        let lit_pipeline = pipeline("lit", &mesh_layout, "vs_mesh", "fs_lit", &mesh_buffers, wgpu::PrimitiveTopology::TriangleList, (true, true), none);
        // Pulled toward the camera so a trigger lying on a floor never fights it.
        let pull = wgpu::DepthBiasState { constant: -8, slope_scale: -2.0, clamp: 0.0 };
        let translucent_pipeline =
            pipeline("translucent", &mesh_layout, "vs_mesh", "fs_mesh", &mesh_buffers, wgpu::PrimitiveTopology::TriangleList, (false, true), pull);
        let line_pipeline = pipeline("line", &line_layout, "vs_line", "fs_line", &line_buffers, wgpu::PrimitiveTopology::LineList, (true, true), none);
        let overlay_pipeline = pipeline("overlay", &line_layout, "vs_line", "fs_line", &line_buffers, wgpu::PrimitiveTopology::LineList, (false, false), none);

        let flat = texture(device, queue, 1, 1, &[128, 128, 255, 255], wgpu::TextureFormat::Rgba8Unorm);
        let rough = texture(device, queue, 1, 1, &[255, 255, 0, 255], wgpu::TextureFormat::Rgba8Unorm);
        let bind = |albedo: &wgpu::TextureView| material_bind_group(device, &texture_layout, &sampler, albedo, &flat, &rough);
        let white = bind(&texture(device, queue, 1, 1, &[255, 255, 255, 255], wgpu::TextureFormat::Rgba8UnormSrgb));
        let checker: Vec<u8> = (0..64u32).flat_map(|i| if (i % 8 < 4) ^ (i / 8 < 4) { [255, 0, 255, 255] } else { [0, 0, 0, 255] }).collect();
        let missing = bind(&texture(device, queue, 8, 8, &checker, wgpu::TextureFormat::Rgba8UnormSrgb));
        Self {
            device: device.clone(),
            queue: queue.clone(),
            white,
            missing,
            globals_layout,
            texture_layout,
            sampler,
            flat,
            rough,
            mesh_pipeline,
            lit_pipeline,
            translucent_pipeline,
            line_pipeline,
            overlay_pipeline,
            materials: HashMap::new(),
            batches: Vec::new(),
            edges: None,
            selected_edges: None,
            translucent: None,
            lights: Vec::new(),
            ambient: [0.0; 3],
        }
    }

    fn fits(&self, w: u32, h: u32, rgba: &[u8]) -> bool {
        let max = self.device.limits().max_texture_dimension_2d;
        w > 0 && h > 0 && w <= max && h <= max && rgba.len() as u64 == u64::from(w) * u64::from(h) * 4
    }

    /// Uploads a material's RGBA8 pixels; a size that does not match the data is ignored.
    pub fn upload_material(&mut self, name: &str, w: u32, h: u32, rgba: &[u8]) -> bool {
        if !self.fits(w, h, rgba) {
            return false;
        }
        let albedo = texture(&self.device, &self.queue, w, h, rgba, wgpu::TextureFormat::Rgba8UnormSrgb);
        let bind_group = material_bind_group(&self.device, &self.texture_layout, &self.sampler, &albedo, &self.flat, &self.rough);
        self.materials.insert(name.to_string(), Material { bind_group, size: [w, h], albedo });
        true
    }

    /// Gives an uploaded material Lit's maps: a normal map with green up, and occlusion, roughness and metallic in RGB.
    pub fn set_material_maps(&mut self, name: &str, normal: (u32, u32, &[u8]), orm: (u32, u32, &[u8])) -> bool {
        if !self.fits(normal.0, normal.1, normal.2) || !self.fits(orm.0, orm.1, orm.2) {
            return false;
        }
        let Some(m) = self.materials.get_mut(name) else { return false };
        let linear = |(w, h, rgba)| texture(&self.device, &self.queue, w, h, rgba, wgpu::TextureFormat::Rgba8Unorm);
        m.bind_group = material_bind_group(&self.device, &self.texture_layout, &self.sampler, &m.albedo, &linear(normal), &linear(orm));
        true
    }

    /// Drops every uploaded material, so the next scene uploads them again from a new listing.
    pub fn forget_materials(&mut self) {
        self.materials.clear();
    }

    /// Makes texture coordinates count `size` texels for a material uploaded as a smaller picture.
    pub fn set_material_size(&mut self, name: &str, size: [u32; 2]) {
        if let Some(m) = self.materials.get_mut(name).filter(|_| size[0] > 0 && size[1] > 0) {
            m.size = size;
        }
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
        self.lights = scene.lights.clone();
        self.ambient = scene.ambient;
    }

    pub fn target(&self, size: [u32; 2]) -> Target {
        let size = [size[0].clamp(1, 8192), size[1].clamp(1, 8192)];
        let extent = wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 };
        let tex = |label, format, samples, usage, view_formats: &[wgpu::TextureFormat]| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: extent,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats,
            })
        };
        let msaa = tex("viewport msaa", COLOR_FORMAT, SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT, &[]).create_view(&Default::default());
        let depth = tex("viewport depth", DEPTH_FORMAT, SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT, &[]).create_view(&Default::default());
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC;
        let texture = tex("viewport", COLOR_FORMAT, 1, usage, &[DISPLAY_FORMAT]);
        let view = texture.create_view(&Default::default());
        let display = texture.create_view(&wgpu::TextureViewDescriptor { format: Some(DISPLAY_FORMAT), ..Default::default() });
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
        Target { size, msaa, depth, texture, view, display, globals, globals_bind }
    }

    /// Draws the scene into `target`; `grid` is the 2D grid spacing, `overlay` lines draw on top.
    pub fn render(&self, target: &Target, camera: &Camera, mode: RenderMode, grid: Option<f64>, overlay: &[LineVertex]) {
        let size = DVec2::new(f64::from(target.size[0]), f64::from(target.size[1]));
        let light = DVec3::new(0.3, 0.5, 0.8).normalize().as_vec3();
        let lit = mode == RenderMode::Lit;
        let mut globals = Globals {
            view_proj: camera.view_proj(size).as_mat4().to_cols_array_2d(),
            light_dir: [light.x, light.y, light.z, 0.0],
            ambient: [0.0; 4],
            ground: [0.0; 4],
            eye: camera.position.as_vec3().extend(1.0).to_array(),
            lights: [Light::default(); MAX_LIGHTS],
        };
        if lit && self.lights.is_empty() {
            for (l, (dir, [r, g, b])) in globals.lights.iter_mut().zip(RIG_SUNS) {
                *l = Light {
                    pos: DVec3::from_array(dir.map(f64::from)).normalize().as_vec3().extend(0.0).to_array(),
                    color: [r, g, b, 3.0],
                    ..Light::default()
                };
            }
            globals.ambient = [RIG_SKY[0], RIG_SKY[1], RIG_SKY[2], RIG_SUNS.len() as f32];
            globals.ground = [RIG_GROUND[0], RIG_GROUND[1], RIG_GROUND[2], 0.0];
        } else if lit {
            let mut near = self.lights.clone();
            // Suns have w 0, so they always come first.
            let d = |l: &Light| f64::from(l.pos[3]) * camera.position.distance_squared(DVec4::from_array(l.pos.map(f64::from)).truncate());
            near.sort_by(|a, b| d(a).total_cmp(&d(b)));
            let n = near.len().min(MAX_LIGHTS);
            globals.lights[..n].copy_from_slice(&near[..n]);
            let [r, g, b] = self.ambient.map(|c| c.max(MIN_AMBIENT));
            globals.ambient = [r, g, b, n as f32];
            globals.ground = [r, g, b, 0.0];
        }
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
                for batch in &self.batches {
                    // Entity boxes stay unlit, or a light's own box would be dark.
                    pass.set_pipeline(if lit && !batch.material.is_empty() { &self.lit_pipeline } else { &self.mesh_pipeline });
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

fn texture(device: &wgpu::Device, queue: &wgpu::Queue, w: u32, h: u32, rgba: &[u8], format: wgpu::TextureFormat) -> wgpu::TextureView {
    device
        .create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("material"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            rgba,
        )
        .create_view(&Default::default())
}

fn material_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    albedo: &wgpu::TextureView,
    normal: &wgpu::TextureView,
    orm: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("material"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(albedo) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(normal) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(orm) },
        ],
    })
}

/// Grid lines covering a 2D view; spacing doubles until lines are at least 6 pixels apart.
/// The grid spacing a 2D view at `zoom` pixels per unit draws: `grid` doubled until cells are
/// at least 8 pixels wide.
pub fn grid_step(grid: f64, zoom: f64) -> f64 {
    if !grid.is_finite() || grid <= 0.0 || !zoom.is_finite() || zoom <= 0.0 {
        return grid;
    }
    let mut step = grid;
    while step * zoom < 8.0 {
        step *= 2.0;
    }
    step
}

fn grid_lines(out: &mut Vec<LineVertex>, camera: &Camera, size: DVec2, grid: f64) {
    if !grid.is_finite() || grid <= 0.0 {
        return;
    }
    let step = grid_step(grid, camera.zoom);
    // 1024 unit lines only stand out while finer lines lie between them.
    let mark_1024 = step * 4.0 <= 1024.0;
    let (r, u, _) = camera.view.axes();
    let half = size / (2.0 * camera.zoom);
    let (h0, h1) = (camera.center.x - half.x, camera.center.x + half.x);
    let (v0, v1) = (camera.center.y - half.y, camera.center.y + half.y);
    // Linear colors; on screen the axes are a muted blue, 1024 unit lines dark brown, every eighth
    // line mid gray and the rest a dim gray.
    let color = |c: f64| {
        if c == 0.0 {
            [0.013, 0.078, 0.157, 1.0]
        } else if mark_1024 && (c / 1024.0).fract() == 0.0 {
            [0.058, 0.023, 0.007, 1.0]
        } else if (c / (step * 8.0)).fract() == 0.0 {
            [0.045, 0.045, 0.045, 1.0]
        } else {
            [0.02, 0.02, 0.02, 1.0]
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
