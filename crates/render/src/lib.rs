//! wgpu renderer for MapCraft viewports: each viewport draws into its own offscreen texture.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod camera;
mod model;
mod scene;

use std::collections::HashMap;

use mapcraft_geom::{DMat4, DVec2, DVec3, DVec4};
use mapcraft_map::AntiTile;
use wgpu::util::DeviceExt;

pub use camera::{Camera, View};
pub use model::{MAX_MODEL_BYTES, MeshPart, Model, ModelPart, Picture, entity_transform, load_gltf, mesh_model, placed_bounds};
pub use scene::{
    BatchKey, EntityStyle, Light, LineVertex, MeshVertex, NoStyle, SELECTED_EDGE, SceneData, Sky, Slope, WORLD_EDGE, box_lines, build_scene, entity_bounds,
    is_tool_material,
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
/// Share of the sky's top color in the light Godot's procedural sky casts straight down, its curve averaged over the dome.
const SKY_TOP_SHARE: f32 = 0.93;

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
    inv_view_proj: [[f32; 4]; 4],
    /// The Lit mode's sky colors; `sky_top.w` is 1 while a sky is drawn and tonemapped.
    sky_top: [f32; 4],
    sky_horizon: [f32; 4],
    sky_ground: [f32; 4],
    /// The lowest lightmap brightness Quake lights add up to.
    minlight: [f32; 4],
    /// x is how strongly displaced vertices wash toward cyan by their blend weight.
    paint: [f32; 4],
    lights: [Light; MAX_LIGHTS],
}

struct Material {
    bind_group: wgpu::BindGroup,
    size: [u32; 2],
    albedo: wgpu::TextureView,
    /// The game draws it with nearest filtering, whatever the renderer's own setting.
    pixelated: bool,
}

struct Batch {
    key: BatchKey,
    buffer: wgpu::Buffer,
    count: u32,
    /// Brush faces, which a baked map stands in for.
    brush: bool,
}

/// Triangles of a compiled map in one material, with the light its compiler baked into them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BakePart {
    pub material: String,
    /// Index into [`Bake::lightmaps`]; none draws the material unlit, as Quake draws its liquids.
    pub lightmap: Option<usize>,
    pub positions: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// One per position when there is a lightmap.
    pub lightmap_uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

/// A compiled map the Lit view draws in place of the brushes, its lightmaps lighting the materials as Quake does.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bake {
    pub parts: Vec<BakePart>,
    pub lightmaps: Vec<Picture>,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BakedVertex {
    pos: [f32; 3],
    uv: [f32; 2],
    lightmap: [f32; 2],
}

struct BakedBatch {
    material: String,
    lightmap: Option<usize>,
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
    nearest: wgpu::Sampler,
    /// 1x1 maps for materials without their own: a flat normal, and fully rough, unoccluded and not metallic.
    flat: wgpu::TextureView,
    rough: wgpu::TextureView,
    /// Mesh pipelines by (lit, layers, anti-tiling); a variant the device rejected is missing.
    pipelines: HashMap<(bool, u32, AntiTile), wgpu::RenderPipeline>,
    translucent_pipeline: wgpu::RenderPipeline,
    line_pipeline: wgpu::RenderPipeline,
    overlay_pipeline: wgpu::RenderPipeline,
    sky_pipeline: wgpu::RenderPipeline,
    sky_face_pipeline: wgpu::RenderPipeline,
    lightmap_layout: wgpu::BindGroupLayout,
    lightmap_sampler: wgpu::Sampler,
    baked_pipeline: wgpu::RenderPipeline,
    baked_unlit_pipeline: wgpu::RenderPipeline,
    baked_sky_pipeline: wgpu::RenderPipeline,
    bake: Vec<BakedBatch>,
    lightmaps: Vec<wgpu::BindGroup>,
    /// The brushes changed since the bake, so Lit draws them live until the next one lands.
    bake_stale: bool,
    materials: HashMap<String, Material>,
    white: wgpu::BindGroup,
    missing: wgpu::BindGroup,
    white_albedo: wgpu::TextureView,
    missing_albedo: wgpu::TextureView,
    /// Textures are magnified without smoothing, as retro games draw them.
    pixelated: bool,
    /// Materials uploaded as skies.
    skies: std::collections::HashSet<String>,
    batches: Vec<Batch>,
    edges: Option<(wgpu::Buffer, u32)>,
    selected_edges: Option<(wgpu::Buffer, u32)>,
    translucent: Option<(wgpu::Buffer, u32)>,
    lights: Vec<Light>,
    ambient: [f32; 3],
    minlight: [f32; 3],
    sky: Option<Sky>,
    /// Off, 3D views draw no sky behind the map and draw sky faces like other faces.
    pub show_sky: bool,
    weight_overlay: f32,
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
        let sampler = material_sampler(device, false);
        let mesh_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mesh"),
            bind_group_layouts: &[Some(&globals_layout), Some(&texture_layout), None, Some(&texture_layout)],
            immediate_size: 0,
        });
        let plain_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("plain"),
            bind_group_layouts: &[Some(&globals_layout), Some(&texture_layout)],
            immediate_size: 0,
        });
        let line_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("line"),
            bind_group_layouts: &[Some(&globals_layout)],
            immediate_size: 0,
        });
        let mesh_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32x4, 4 => Float32x4, 5 => Float32x2, 6 => Float32];
        let line_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4];
        // `depth` is (write, test): overlays skip the test, see-through faces skip the write.
        let pipeline = |label: &str,
                        layout: &wgpu::PipelineLayout,
                        vs: &str,
                        fs: &str,
                        buffers: &[Option<wgpu::VertexBufferLayout>],
                        topology,
                        depth: (bool, bool),
                        bias: wgpu::DepthBiasState,
                        constants: &[(&str, f64)]| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some(vs), compilation_options: Default::default(), buffers },
                primitive: wgpu::PrimitiveState { topology, cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(depth.0),
                    // Depth is reversed: 1 at the near plane, 0 far away, which keeps distant surfaces apart.
                    depth_compare: Some(if depth.1 { wgpu::CompareFunction::GreaterEqual } else { wgpu::CompareFunction::Always }),
                    stencil: Default::default(),
                    bias,
                }),
                multisample: wgpu::MultisampleState { count: SAMPLES, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: wgpu::PipelineCompilationOptions { constants, ..Default::default() },
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
        let mut pipelines = HashMap::new();
        for (lit, fs) in [(false, "fs_mesh"), (true, "fs_lit")] {
            for layers in [1u32, 2] {
                for mode in [AntiTile::Off, AntiTile::Macro, AntiTile::Hex] {
                    // wgpu panics on uncaptured errors, and a backend rejecting a variant reports Internal, not Validation.
                    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
                    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
                    let constants = [("LAYERS", f64::from(layers)), ("ANTI_TILE", f64::from(mode as u32))];
                    let p = pipeline(fs, &mesh_layout, "vs_mesh", fs, &mesh_buffers, wgpu::PrimitiveTopology::TriangleList, (true, true), none, &constants);
                    let internal = pollster::block_on(internal.pop());
                    match internal.or(pollster::block_on(validation.pop())) {
                        None => {
                            pipelines.insert((lit, layers, mode), p);
                        }
                        Some(e) => log::warn!("the {fs} shader with {layers} layers and anti-tiling {mode:?} was rejected: {e}"),
                    }
                }
            }
        }
        // Pulled toward the camera so a trigger lying on a floor never fights it.
        let pull = wgpu::DepthBiasState { constant: 8, slope_scale: 2.0, clamp: 0.0 };
        let translucent_pipeline =
            pipeline("translucent", &mesh_layout, "vs_mesh", "fs_mesh", &mesh_buffers, wgpu::PrimitiveTopology::TriangleList, (false, true), pull, &[]);
        let line_pipeline = pipeline("line", &line_layout, "vs_line", "fs_line", &line_buffers, wgpu::PrimitiveTopology::LineList, (true, true), none, &[]);
        let overlay_pipeline =
            pipeline("overlay", &line_layout, "vs_line", "fs_line", &line_buffers, wgpu::PrimitiveTopology::LineList, (false, false), none, &[]);
        let sky_pipeline = pipeline("sky", &line_layout, "vs_sky", "fs_sky", &[], wgpu::PrimitiveTopology::TriangleList, (false, false), none, &[]);
        let sky_face_pipeline =
            pipeline("sky faces", &mesh_layout, "vs_mesh", "fs_sky_face", &mesh_buffers, wgpu::PrimitiveTopology::TriangleList, (true, true), none, &[]);
        let lightmap_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lightmap"),
            entries: &[
                image(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let baked_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("baked"),
            bind_group_layouts: &[Some(&globals_layout), Some(&texture_layout), Some(&lightmap_layout)],
            immediate_size: 0,
        });
        let baked_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x2];
        let baked_buffers = [Some(vertex_layout(std::mem::size_of::<BakedVertex>(), &baked_attrs))];
        let triangles = wgpu::PrimitiveTopology::TriangleList;
        let baked_pipeline = pipeline("baked", &baked_layout, "vs_baked", "fs_baked", &baked_buffers, triangles, (true, true), none, &[]);
        let baked_unlit_pipeline = pipeline("baked unlit", &plain_layout, "vs_baked", "fs_baked_unlit", &baked_buffers, triangles, (true, true), none, &[]);
        let baked_sky_pipeline = pipeline("baked sky", &plain_layout, "vs_baked", "fs_sky_face", &baked_buffers, triangles, (true, true), none, &[]);
        // Engines filter lightmaps smoothly between luxel centers.
        let lightmap_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("lightmap"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let flat = texture(device, queue, 1, 1, &[128, 128, 255, 255], wgpu::TextureFormat::Rgba8Unorm);
        let rough = texture(device, queue, 1, 1, &[255, 255, 0, 255], wgpu::TextureFormat::Rgba8Unorm);
        let bind = |albedo: &wgpu::TextureView| material_bind_group(device, &texture_layout, &sampler, albedo, &flat, &rough);
        let white_albedo = texture(device, queue, 1, 1, &[255, 255, 255, 255], wgpu::TextureFormat::Rgba8UnormSrgb);
        let checker: Vec<u8> = (0..64u32).flat_map(|i| if (i % 8 < 4) ^ (i / 8 < 4) { [255, 0, 255, 255] } else { [0, 0, 0, 255] }).collect();
        let missing_albedo = texture(device, queue, 8, 8, &checker, wgpu::TextureFormat::Rgba8UnormSrgb);
        let (white, missing) = (bind(&white_albedo), bind(&missing_albedo));
        Self {
            device: device.clone(),
            queue: queue.clone(),
            white,
            missing,
            white_albedo,
            missing_albedo,
            pixelated: false,
            skies: std::collections::HashSet::new(),
            globals_layout,
            texture_layout,
            sampler,
            nearest: material_sampler(device, true),
            flat,
            rough,
            pipelines,
            translucent_pipeline,
            line_pipeline,
            overlay_pipeline,
            sky_pipeline,
            sky_face_pipeline,
            lightmap_layout,
            lightmap_sampler,
            baked_pipeline,
            baked_unlit_pipeline,
            baked_sky_pipeline,
            bake: Vec::new(),
            lightmaps: Vec::new(),
            bake_stale: false,
            materials: HashMap::new(),
            batches: Vec::new(),
            edges: None,
            selected_edges: None,
            translucent: None,
            lights: Vec::new(),
            ambient: [0.0; 3],
            minlight: [0.0; 3],
            sky: None,
            show_sky: true,
            weight_overlay: 0.0,
        }
    }

    fn fits(&self, w: u32, h: u32, rgba: &[u8]) -> bool {
        let max = self.device.limits().max_texture_dimension_2d;
        w > 0 && h > 0 && w <= max && h <= max && rgba.len() as u64 == u64::from(w) * u64::from(h) * 4
    }

    fn sampler_for(&self, pixelated: bool) -> &wgpu::Sampler {
        if pixelated || self.pixelated { &self.nearest } else { &self.sampler }
    }

    /// Uploads a material's RGBA8 pixels; a size that does not match the data is ignored.
    pub fn upload_material(&mut self, name: &str, w: u32, h: u32, rgba: &[u8]) -> bool {
        self.upload_material_with(name, w, h, rgba, false)
    }

    /// [`Renderer::upload_material`] for a material the game may draw pixelated on its own.
    pub fn upload_material_with(&mut self, name: &str, w: u32, h: u32, rgba: &[u8], pixelated: bool) -> bool {
        if !self.fits(w, h, rgba) {
            return false;
        }
        let albedo = texture(&self.device, &self.queue, w, h, rgba, wgpu::TextureFormat::Rgba8UnormSrgb);
        let bind_group = material_bind_group(&self.device, &self.texture_layout, self.sampler_for(pixelated), &albedo, &self.flat, &self.rough);
        self.materials.insert(name.to_string(), Material { bind_group, size: [w, h], albedo, pixelated });
        true
    }

    /// Uploads a sky in Quake's layout: the far layer on the right half, the near layer on the left
    /// with clear pixels. Faces with it show the sky by the direction they are seen from, not their UVs.
    pub fn upload_sky(&mut self, name: &str, w: u32, h: u32, rgba: &[u8], pixelated: bool) -> bool {
        let half = w / 2;
        if half == 0 || !self.fits(w, h, rgba) {
            return false;
        }
        let layer = |from: u32| -> Vec<u8> {
            (0..h as usize)
                .flat_map(|y| {
                    let row = (y * w as usize + from as usize) * 4;
                    rgba.get(row..row + half as usize * 4).unwrap_or_default().iter().copied()
                })
                .collect()
        };
        let far = texture(&self.device, &self.queue, half, h, &layer(half), wgpu::TextureFormat::Rgba8UnormSrgb);
        let near = texture(&self.device, &self.queue, half, h, &layer(0), wgpu::TextureFormat::Rgba8UnormSrgb);
        let bind_group = material_bind_group(&self.device, &self.texture_layout, self.sampler_for(pixelated), &far, &near, &self.rough);
        self.materials.insert(name.to_string(), Material { bind_group, size: [w, h], albedo: far, pixelated });
        self.skies.insert(name.to_string());
        true
    }

    /// Gives an uploaded material Lit's maps: a normal map with green up, and occlusion, roughness and metallic in RGB.
    pub fn set_material_maps(&mut self, name: &str, normal: (u32, u32, &[u8]), orm: (u32, u32, &[u8])) -> bool {
        if !self.fits(normal.0, normal.1, normal.2) || !self.fits(orm.0, orm.1, orm.2) {
            return false;
        }
        let Some(m) = self.materials.get_mut(name) else { return false };
        let linear = |(w, h, rgba)| texture(&self.device, &self.queue, w, h, rgba, wgpu::TextureFormat::Rgba8Unorm);
        let sampler = if m.pixelated || self.pixelated { &self.nearest } else { &self.sampler };
        m.bind_group = material_bind_group(&self.device, &self.texture_layout, sampler, &m.albedo, &linear(normal), &linear(orm));
        true
    }

    /// Drops every uploaded material, so the next scene uploads them again from a new listing.
    pub fn forget_materials(&mut self) {
        self.materials.clear();
        self.skies.clear();
    }

    /// Switches texture magnification between smooth and pixelated; uploaded materials are dropped
    /// to be uploaded again with the new sampler.
    pub fn set_pixelated(&mut self, on: bool) {
        if on == self.pixelated {
            return;
        }
        self.pixelated = on;
        let bind =
            |albedo: &wgpu::TextureView| material_bind_group(&self.device, &self.texture_layout, self.sampler_for(false), albedo, &self.flat, &self.rough);
        (self.white, self.missing) = (bind(&self.white_albedo), bind(&self.missing_albedo));
        self.materials.clear();
        self.skies.clear();
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

    /// How strongly displaced vertices wash toward cyan by their blend weight, 0 for not at all.
    pub fn set_weight_overlay(&mut self, strength: f32) {
        self.weight_overlay = if strength.is_finite() { strength.clamp(0.0, 1.0) } else { 0.0 };
    }

    pub fn set_scene(&mut self, scene: &SceneData) {
        let buffer = |label: &str, data: &[u8]| {
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents: data, usage: wgpu::BufferUsages::VERTEX })
        };
        let all = scene.batches.iter().map(|(k, v)| (k.clone(), v, true)).chain(scene.points.iter().map(|(k, v)| (k.clone(), v, false)));
        self.batches = all
            .filter(|(_, v, _)| !v.is_empty())
            .map(|(key, v, brush)| Batch { key, buffer: buffer("batch", bytemuck::cast_slice(v)), count: v.len() as u32, brush })
            .collect();
        let lines = |v: &[LineVertex]| (!v.is_empty()).then(|| (buffer("lines", bytemuck::cast_slice(v)), v.len() as u32));
        self.edges = lines(&scene.edges);
        self.selected_edges = lines(&scene.selected_edges);
        self.translucent =
            (!scene.translucent.is_empty()).then(|| (buffer("translucent", bytemuck::cast_slice(&scene.translucent)), scene.translucent.len() as u32));
        self.lights = scene.lights.clone();
        self.ambient = scene.ambient;
        self.minlight = scene.minlight;
        self.sky = scene.sky;
    }

    /// Draws the live brushes in Lit while the bake shows an older map, keeping the bake for later.
    pub fn set_bake_stale(&mut self, stale: bool) {
        self.bake_stale = stale;
    }

    /// Shows a compiled map in the Lit view in place of the brushes, or the brushes again with `None`.
    pub fn set_bake(&mut self, bake: Option<&Bake>) {
        (self.bake, self.lightmaps) = (Vec::new(), Vec::new());
        let Some(bake) = bake else { return };
        let black = Picture { width: 1, height: 1, rgba: vec![0, 0, 0, 255] };
        for p in &bake.lightmaps {
            let p = if self.fits(p.width, p.height, &p.rgba) { p } else { &black };
            let view = single_level(&self.device, &self.queue, p.width, p.height, &p.rgba);
            self.lightmaps.push(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("lightmap"),
                layout: &self.lightmap_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.lightmap_sampler) },
                ],
            }));
        }
        for part in &bake.parts {
            let lightmap = part.lightmap.filter(|l| *l < self.lightmaps.len() && part.lightmap_uvs.len() == part.positions.len());
            let vertex = |i: &u32| {
                let i = *i as usize;
                let lm = if lightmap.is_some() { *part.lightmap_uvs.get(i)? } else { [0.0; 2] };
                Some(BakedVertex { pos: *part.positions.get(i)?, uv: *part.uvs.get(i)?, lightmap: lm })
            };
            let verts: Vec<BakedVertex> = part.indices.iter().map_while(vertex).collect();
            if verts.len() != part.indices.len() || verts.is_empty() {
                continue;
            }
            let buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("bake"),
                contents: bytemuck::cast_slice(&verts),
                usage: wgpu::BufferUsages::VERTEX,
            });
            self.bake.push(BakedBatch { material: part.material.clone(), lightmap, buffer, count: verts.len() as u32 });
        }
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

    /// Copies a rendered target back as tightly packed RGBA rows, waiting for the GPU.
    pub fn read_rgba(&self, target: &Target) -> Option<Vec<u8>> {
        let [w, h] = target.size;
        let row = (w * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(row) * u64::from(h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            target.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit([enc.finish()]);
        buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
        let data = buf.slice(..).get_mapped_range().ok()?;
        let (row, packed) = (row as usize, w as usize * 4);
        let mut out = Vec::with_capacity(packed * h as usize);
        for y in 0..h as usize {
            out.extend_from_slice(data.get(y * row..y * row + packed)?);
        }
        Some(out)
    }

    /// The variant a batch draws with, or the closest lighter one when the device rejected it.
    fn mesh_pipeline(&self, lit: bool, mode: RenderMode, key: &BatchKey) -> Option<&wgpu::RenderPipeline> {
        // Flat mode's albedo is white, so a second layer or variation would only mottle it gray.
        let (layers, anti_tile) = match key.anti_tile {
            _ if mode == RenderMode::Flat => (1, AntiTile::Off),
            // Three blended taps of big nearest texels read as double images.
            AntiTile::Hex if self.pixelated || [&key.material, &key.blend].iter().any(|n| self.materials.get(*n).is_some_and(|m| m.pixelated)) => {
                (if key.blend.is_empty() { 1 } else { 2 }, AntiTile::Macro)
            }
            other => (if key.blend.is_empty() { 1 } else { 2 }, other),
        };
        let lighter = if anti_tile == AntiTile::Hex { AntiTile::Macro } else { AntiTile::Off };
        [(lit, layers, anti_tile), (lit, layers, lighter), (lit, layers, AntiTile::Off), (lit, 1, AntiTile::Off)].iter().find_map(|k| self.pipelines.get(k))
    }

    /// Draws the scene into `target`; `grid` is the 2D grid spacing, `overlay` lines draw on top.
    pub fn render(&self, target: &Target, camera: &Camera, mode: RenderMode, grid: Option<f64>, overlay: &[LineVertex]) {
        let size = DVec2::new(f64::from(target.size[0]), f64::from(target.size[1]));
        let lit = mode == RenderMode::Lit;
        let baked = lit && !camera.is_2d() && !self.bake.is_empty() && !self.bake_stale;
        let sky = self.sky.filter(|_| lit && !camera.is_2d());
        let mut globals = Globals::unlit(camera, size, self.weight_overlay);
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
            // Quake maps get the readability floor as a minimum light level, the way Quake's minlight works.
            let quake = self.lights.iter().any(|l| l.cone[3] > 0.0);
            let [r, g, b] = self.ambient.map(|c| c.max(if quake { 0.0 } else { MIN_AMBIENT }));
            globals.ambient = [r, g, b, n as f32];
            globals.ground = [r, g, b, 0.0];
            let [r, g, b] = self.minlight.map(|c| c.max(if quake { MIN_AMBIENT.powf(1.0 / 2.2) } else { 0.0 }));
            globals.minlight = [r, g, b, 0.0];
            if let Some(s) = sky {
                let up: [f32; 3] = std::array::from_fn(|i| (s.horizon[i] + (s.top[i] - s.horizon[i]) * SKY_TOP_SHARE) * s.ambient);
                globals.ambient = [up[0], up[1], up[2], n as f32];
                globals.ground = [s.ground[0] * s.ambient, s.ground[1] * s.ambient, s.ground[2] * s.ambient, 0.0];
                globals.sky_top = [s.top[0], s.top[1], s.top[2], 1.0];
                globals.sky_horizon = [s.horizon[0], s.horizon[1], s.horizon[2], 0.0];
                globals.sky_ground = [s.ground[0], s.ground[1], s.ground[2], 0.0];
            }
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
            let mut pass = begin_pass(&mut encoder, target, bg);
            if sky.is_some() && self.show_sky {
                pass.set_pipeline(&self.sky_pipeline);
                pass.draw(0..3, 0..1);
            }
            if let Some(b) = &grid_buf {
                pass.set_pipeline(&self.overlay_pipeline);
                pass.set_vertex_buffer(0, b.slice(..));
                pass.draw(0..extra.len() as u32, 0..1);
            }
            if !camera.is_2d() && mode != RenderMode::Wireframe {
                for batch in self.batches.iter().filter(|b| !(baked && b.brush)) {
                    let material = batch.key.material.as_str();
                    let sky = self.skies.contains(material);
                    let sky_face = mode != RenderMode::Flat && sky && self.show_sky;
                    // Entity boxes stay unlit, or a light's own box would be dark.
                    let pipeline =
                        if sky_face { Some(&self.sky_face_pipeline) } else { self.mesh_pipeline(lit && !material.is_empty() && !sky, mode, &batch.key) };
                    let Some(pipeline) = pipeline else { continue };
                    pass.set_pipeline(pipeline);
                    let bind = |m: &str| match (mode, m) {
                        (RenderMode::Flat, _) | (_, "") => &self.white,
                        (_, m) => self.materials.get(m).map_or(&self.missing, |t| &t.bind_group),
                    };
                    pass.set_bind_group(1, bind(material), &[]);
                    pass.set_bind_group(3, if sky_face { &self.white } else { bind(&batch.key.blend) }, &[]);
                    pass.set_vertex_buffer(0, batch.buffer.slice(..));
                    pass.draw(0..batch.count, 0..1);
                }
            }
            for batch in self.bake.iter().filter(|_| baked) {
                let lightmap = batch.lightmap.and_then(|l| self.lightmaps.get(l));
                pass.set_pipeline(match lightmap {
                    _ if self.skies.contains(&batch.material) => &self.baked_sky_pipeline,
                    Some(_) => &self.baked_pipeline,
                    None => &self.baked_unlit_pipeline,
                });
                pass.set_bind_group(1, self.materials.get(&batch.material).map_or(&self.missing, |m| &m.bind_group), &[]);
                if let Some(l) = lightmap.filter(|_| !self.skies.contains(&batch.material)) {
                    pass.set_bind_group(2, l, &[]);
                }
                pass.set_vertex_buffer(0, batch.buffer.slice(..));
                pass.draw(0..batch.count, 0..1);
            }
            // Triggers and other tool faces do not show in the game a bake stands for.
            if let Some((b, n)) = self.translucent.as_ref().filter(|_| !camera.is_2d() && mode != RenderMode::Wireframe && !baked) {
                pass.set_pipeline(&self.translucent_pipeline);
                pass.set_bind_group(1, &self.white, &[]);
                pass.set_bind_group(3, &self.white, &[]);
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

    /// `model` alone in a clear `side` pixel square, seen from above its front left with its bounds
    /// filling the frame, as premultiplied RGBA rows; `None` when the GPU refuses it.
    pub fn model_preview(&self, model: &Model, side: u32) -> Option<Vec<u8>> {
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let target = self.target([side, side]);
        let mut camera = Camera::new(View::Perspective);
        (camera.yaw, camera.pitch, camera.fov_y) = (45.0, -30.0, 30.0);
        let radius = ((model.max - model.min).length() / 2.0).max(0.5) * 1.1;
        camera.position = (model.min + model.max) / 2.0 - camera.forward() * radius / (camera.fov_y / 2.0).to_radians().sin();
        let size = DVec2::new(f64::from(target.size[0]), f64::from(target.size[1]));
        self.queue.write_buffer(&target.globals, 0, bytemuck::bytes_of(&Globals::unlit(&camera, size, 0.0)));
        let mut batches = std::collections::BTreeMap::new();
        scene::push_model(&mut batches, &mut Vec::new(), model, &DMat4::IDENTITY, false, WORLD_EDGE);
        let textures: HashMap<String, wgpu::BindGroup> = (model.textures.iter().enumerate())
            .filter(|(_, p)| self.fits(p.width, p.height, &p.rgba))
            .map(|(i, p)| {
                let albedo = texture(&self.device, &self.queue, p.width, p.height, &p.rgba, wgpu::TextureFormat::Rgba8UnormSrgb);
                (model.texture_name(i), material_bind_group(&self.device, &self.texture_layout, self.sampler_for(false), &albedo, &self.flat, &self.rough))
            })
            .collect();
        let buffers: Vec<(&BatchKey, wgpu::Buffer, u32)> = (batches.iter().filter(|(_, v)| !v.is_empty()))
            .map(|(key, v)| {
                let contents = bytemuck::cast_slice(v);
                let buffer =
                    self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("preview"), contents, usage: wgpu::BufferUsages::VERTEX });
                (key, buffer, v.len() as u32)
            })
            .collect();
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("preview") });
        {
            let mut pass = begin_pass(&mut encoder, &target, wgpu::Color::TRANSPARENT);
            pass.set_bind_group(3, &self.white, &[]);
            for (key, buffer, count) in &buffers {
                let Some(pipeline) = self.mesh_pipeline(false, RenderMode::Textured, key) else { continue };
                pass.set_pipeline(pipeline);
                pass.set_bind_group(1, textures.get(&key.material).unwrap_or(&self.white), &[]);
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw(0..*count, 0..1);
            }
        }
        self.queue.submit([encoder.finish()]);
        let rgba = self.read_rgba(&target);
        pollster::block_on(scope.pop()).is_none().then_some(rgba)?
    }
}

impl Globals {
    /// Globals for an unlit view from `camera`; Lit fills in its lights after.
    fn unlit(camera: &Camera, size: DVec2, paint: f32) -> Self {
        let light = DVec3::new(0.3, 0.5, 0.8).normalize().as_vec3();
        let view_proj = camera.view_proj(size);
        Self {
            view_proj: view_proj.as_mat4().to_cols_array_2d(),
            light_dir: [light.x, light.y, light.z, 0.0],
            ambient: [0.0; 4],
            ground: [0.0; 4],
            eye: camera.position.as_vec3().extend(1.0).to_array(),
            inv_view_proj: view_proj.inverse().as_mat4().to_cols_array_2d(),
            sky_top: [0.0; 4],
            sky_horizon: [0.0; 4],
            sky_ground: [0.0; 4],
            minlight: [0.0; 4],
            paint: [paint, 0.0, 0.0, 0.0],
            lights: [Light::default(); MAX_LIGHTS],
        }
    }
}

/// A pass drawing into `target` cleared to `bg`, with the target's globals bound.
fn begin_pass<'a>(encoder: &'a mut wgpu::CommandEncoder, target: &'a Target, bg: wgpu::Color) -> wgpu::RenderPass<'a> {
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
            depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Discard }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_bind_group(0, &target.globals_bind, &[]);
    pass
}

/// Repeating texture sampling with mipmaps; `pixelated` magnifies without smoothing.
fn material_sampler(device: &wgpu::Device, pixelated: bool) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("material"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        mag_filter: if pixelated { wgpu::FilterMode::Nearest } else { wgpu::FilterMode::Linear },
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        // Anisotropy needs every filter linear.
        anisotropy_clamp: if pixelated { 1 } else { 16 },
        ..Default::default()
    })
}

/// `rgba` followed by each smaller mip level down to 1 by 1, averaged in linear light for sRGB
/// textures; and how many levels that is.
fn mip_chain(w: u32, h: u32, rgba: &[u8], srgb: bool) -> (u32, Vec<u8>) {
    let to_linear: Vec<f32> = (0..256).map(|i| if srgb { (i as f32 / 255.0).powf(2.2) } else { i as f32 / 255.0 }).collect();
    let to_byte = |v: f32| (if srgb { v.max(0.0).powf(1.0 / 2.2) } else { v } * 255.0).round().clamp(0.0, 255.0) as u8;
    let mut out = rgba.to_vec();
    let (mut level, mut cw, mut ch, mut levels) = (rgba.to_vec(), w as usize, h as usize, 1u32);
    while cw > 1 || ch > 1 {
        let (nw, nh) = ((cw / 2).max(1), (ch / 2).max(1));
        let mut next = vec![0u8; nw * nh * 4];
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..4 {
                    let mut sum = 0.0;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let (sx, sy) = ((x * 2 + dx).min(cw - 1), (y * 2 + dy).min(ch - 1));
                        let byte = level.get((sy * cw + sx) * 4 + c).copied().unwrap_or(0);
                        // Alpha is always linear.
                        sum += if c == 3 { f32::from(byte) / 255.0 } else { to_linear.get(usize::from(byte)).copied().unwrap_or(0.0) };
                    }
                    if let Some(slot) = next.get_mut((y * nw + x) * 4 + c) {
                        *slot = if c == 3 { (sum / 4.0 * 255.0).round() as u8 } else { to_byte(sum / 4.0) };
                    }
                }
            }
        }
        out.extend_from_slice(&next);
        (level, cw, ch, levels) = (next, nw, nh, levels + 1);
    }
    (levels, out)
}

fn texture(device: &wgpu::Device, queue: &wgpu::Queue, w: u32, h: u32, rgba: &[u8], format: wgpu::TextureFormat) -> wgpu::TextureView {
    let (levels, data) = mip_chain(w, h, rgba, format.is_srgb());
    device
        .create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("material"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: levels,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &data,
        )
        .create_view(&Default::default())
}

/// A linear texture without mipmaps, as lightmaps are drawn.
fn single_level(device: &wgpu::Device, queue: &wgpu::Queue, w: u32, h: u32, rgba: &[u8]) -> wgpu::TextureView {
    device
        .create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("lightmap"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bake_multiplies_the_material_in_srgb_by_twice_its_lightmap() {
        let Ok(adapter) = pollster::block_on(wgpu::Instance::default().request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("skipped: no GPU adapter");
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let mut r = Renderer::new(&device, &queue);
        r.upload_material("m", 1, 1, &[200, 100, 50, 255]);
        let quad = BakePart {
            material: "m".into(),
            lightmap: Some(0),
            positions: vec![[-1000.0, -1000.0, 0.0], [1000.0, -1000.0, 0.0], [1000.0, 1000.0, 0.0], [-1000.0, 1000.0, 0.0]],
            // Varying, since a sampler that filters anisotropically can return nothing for a constant coordinate.
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            lightmap_uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            indices: vec![0, 1, 2, 0, 2, 3],
        };
        r.set_bake(Some(&Bake { parts: vec![quad], lightmaps: vec![Picture { width: 1, height: 1, rgba: vec![64, 128, 255, 255] }] }));
        let target = r.target([4, 4]);
        let camera = Camera { position: DVec3::new(0.0, 0.0, 100.0), pitch: -89.0, ..Camera::new(View::Perspective) };
        r.render(&target, &camera, RenderMode::Lit, None, &[]);
        let rgba = r.read_rgba(&target).unwrap();
        let pixel = &rgba[(2 * 4 + 2) * 4..(2 * 4 + 2) * 4 + 3];
        assert!(pixel.iter().all(|c| c.abs_diff(100) <= 1), "200, 100 and 50 lit by 64, 128 and 255 all come to 100: {pixel:?}");
        r.render(&target, &camera, RenderMode::Textured, None, &[]);
        assert_ne!(&r.read_rgba(&target).unwrap()[(2 * 4 + 2) * 4..(2 * 4 + 2) * 4 + 3], pixel, "only the Lit view shows the bake");
    }
}
