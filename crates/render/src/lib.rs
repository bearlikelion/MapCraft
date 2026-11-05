//! wgpu renderer for MapCraft viewports: each viewport draws into its own offscreen texture.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod animation;
mod camera;
mod environment;
mod gltf_map;
mod godot_shader;
mod model;
mod scene;

use std::collections::HashMap;

use mapcraft_geom::{DMat4, DVec2, DVec3, DVec4};
use mapcraft_map::{AntiTile, NodeId, ROOT};
use wgpu::util::DeviceExt;

pub use animation::{Clip, Playing, Repeat, Rig};
pub use camera::{Camera, View};
pub use environment::EnvironmentEffects;
pub use gltf_map::{ImportedGltf, export_glb, import_gltf};
pub use godot_shader::{ShaderKind, TextureSource, Uniform, uniforms};
pub use model::{
    MAX_MODEL_BYTES, MeshPart, Model, ModelNode, ModelPart, Picture, edited_model, entity_transform, load_gltf, mesh_matrices, mesh_model, placed_bounds,
    to_glb,
};
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
    fog_color: [f32; 4],
    /// x density, y sun scatter, z sky affect, w mode (1 is depth fog).
    fog: [f32; 4],
    /// x curve, yz start and end in meters, w aerial perspective.
    fog_depth: [f32; 4],
    /// x height in meters, y height density.
    fog_height: [f32; 4],
    tonemap: [f32; 4],
    adjustments: [f32; 4],
    sky_shape: [f32; 4],
    /// The lowest lightmap brightness Quake lights add up to.
    minlight: [f32; 4],
    /// x is how strongly displaced vertices wash toward cyan by their blend weight; y is Godot shader seconds; zw is the target size.
    paint: [f32; 4],
    lights: [Light; MAX_LIGHTS],
}

/// The most distinct shader sources a scene compiles; nodes past it report an error.
pub const MAX_SHADERS: usize = 64;

/// A compiled Godot shader and the textures its group 1 binds.
struct GodotShader {
    kind: ShaderKind,
    pipeline: wgpu::RenderPipeline,
    layout: Option<wgpu::BindGroupLayout>,
    textures: Vec<TextureSource>,
}

/// A PBR material's numbers, which change without uploading its maps again.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialValues {
    /// Linear; an alpha below 1 makes the material see-through.
    pub tint: [f32; 4],
    /// RGB added to the emission texture, times the energy in A, as Godot does.
    pub emission: [f32; 4],
    pub normal_strength: f32,
    /// Multiplies the roughness map.
    pub roughness: f32,
    /// Multiplies the metallic map.
    pub metallic: f32,
}

impl Default for MaterialValues {
    fn default() -> Self {
        Self { tint: [1.0; 4], emission: [0.0; 4], normal_strength: 1.0, roughness: 1.0, metallic: 1.0 }
    }
}

impl MaterialValues {
    fn uniform(&self) -> [[f32; 4]; 3] {
        [self.tint, self.emission, [self.normal_strength, self.roughness, self.metallic, 0.0]]
    }
}

struct Material {
    bind_group: wgpu::BindGroup,
    parameters: wgpu::Buffer,
    size: [u32; 2],
    albedo: wgpu::TextureView,
    /// The game draws it with nearest filtering, whatever the renderer's own setting.
    pixelated: bool,
    /// Its picture has clear pixels.
    clear_pixels: bool,
    /// Clear pixels or a tint alpha below 1, so it draws after solid surfaces and hides nothing behind it.
    see_through: bool,
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
    pub anti_tile: AntiTile,
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
    /// The lightmaps hold linear light as RGB9E5 texels, as MapCraft's baker makes them, rather than Quake's bytes.
    pub linear: bool,
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
    anti_tile: AntiTile,
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
    /// A copy of the frame before Godot shaded faces draw, for their screen texture.
    screen: wgpu::Texture,
    screen_view: wgpu::TextureView,
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
    /// Mesh pipelines for see-through materials by lit, which test depth without writing it.
    see_through_pipelines: HashMap<bool, wgpu::RenderPipeline>,
    line_pipeline: wgpu::RenderPipeline,
    overlay_pipeline: wgpu::RenderPipeline,
    sky_pipeline: wgpu::RenderPipeline,
    sky_face_pipeline: wgpu::RenderPipeline,
    lightmap_layout: wgpu::BindGroupLayout,
    lightmap_sampler: wgpu::Sampler,
    /// Baked face pipelines by fragment entry point and anti-tiling; a variant the device rejected is missing.
    baked_pipelines: HashMap<(&'static str, AntiTile), wgpu::RenderPipeline>,
    baked_sky_pipeline: wgpu::RenderPipeline,
    bake: Vec<BakedBatch>,
    bake_linear: bool,
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
    shader: wgpu::ShaderModule,
    globals_only: wgpu::PipelineLayout,
    /// Compiled Godot shaders by source.
    godot: HashMap<String, Result<GodotShader, String>>,
    /// Each shaded node's key into `godot`: its parameters and source.
    shaded: std::collections::BTreeMap<NodeId, String>,
    black: wgpu::TextureView,
    clear: wgpu::TextureView,
    /// Seconds for Godot shaders' TIME.
    pub time: f32,
    /// Each playing model with the clock it started at and the clip time it was last posed at.
    playing: Vec<(Playing, Option<f64>, Option<f64>)>,
    /// The playing models' posed triangles, which 3D views draw.
    posed: Vec<Batch>,
    clock: f64,
}

fn lighter(anti_tile: AntiTile) -> AntiTile {
    if anti_tile == AntiTile::Hex { AntiTile::Macro } else { AntiTile::Off }
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
                image(4),
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
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
        // wgpu panics on uncaptured errors, and a backend rejecting a variant reports Internal, not Validation.
        let checked = |what: String, make: &dyn Fn() -> wgpu::RenderPipeline| {
            let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
            let p = make();
            let internal = pollster::block_on(internal.pop());
            match internal.or(pollster::block_on(validation.pop())) {
                None => Some(p),
                Some(e) => {
                    log::warn!("{what} was rejected: {e}");
                    None
                }
            }
        };
        let mut pipelines = HashMap::new();
        for (lit, fs) in [(false, "fs_mesh"), (true, "fs_lit")] {
            for layers in [1u32, 2] {
                for mode in [AntiTile::Off, AntiTile::Macro, AntiTile::Hex] {
                    let constants = [("LAYERS", f64::from(layers)), ("ANTI_TILE", f64::from(mode as u32))];
                    let make =
                        || pipeline(fs, &mesh_layout, "vs_mesh", fs, &mesh_buffers, wgpu::PrimitiveTopology::TriangleList, (true, true), none, &constants);
                    if let Some(p) = checked(format!("the {fs} shader with {layers} layers and anti-tiling {mode:?}"), &make) {
                        pipelines.insert((lit, layers, mode), p);
                    }
                }
            }
        }
        let see_through_pipelines: HashMap<bool, wgpu::RenderPipeline> = [(false, "fs_mesh"), (true, "fs_lit")]
            .into_iter()
            .map(|(lit, fs)| {
                let constants = [("LAYERS", 1.0), ("ANTI_TILE", f64::from(AntiTile::Off as u32)), ("CULL_BACK", 1.0)];
                (lit, pipeline(fs, &mesh_layout, "vs_mesh", fs, &mesh_buffers, wgpu::PrimitiveTopology::TriangleList, (false, true), none, &constants))
            })
            .collect();
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
        let mut baked_pipelines = HashMap::new();
        for (fs, layout) in [("fs_baked", &baked_layout), ("fs_baked_linear", &baked_layout), ("fs_baked_unlit", &plain_layout)] {
            for mode in [AntiTile::Off, AntiTile::Macro, AntiTile::Hex] {
                let constants = [("ANTI_TILE", f64::from(mode as u32))];
                let make = || pipeline(fs, layout, "vs_baked", fs, &baked_buffers, triangles, (true, true), none, &constants);
                if let Some(p) = checked(format!("the {fs} shader with anti-tiling {mode:?}"), &make) {
                    baked_pipelines.insert((fs, mode), p);
                }
            }
        }
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
        let bind = |albedo: &wgpu::TextureView| material_bind_group(device, &texture_layout, &sampler, albedo, &flat, &rough, None).0;
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
            see_through_pipelines,
            line_pipeline,
            overlay_pipeline,
            sky_pipeline,
            sky_face_pipeline,
            lightmap_layout,
            lightmap_sampler,
            baked_pipelines,
            baked_sky_pipeline,
            bake: Vec::new(),
            bake_linear: false,
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
            shader,
            globals_only: line_layout,
            godot: HashMap::new(),
            shaded: Default::default(),
            black: texture(device, queue, 1, 1, &[0, 0, 0, 255], wgpu::TextureFormat::Rgba8Unorm),
            clear: texture(device, queue, 1, 1, &[0, 0, 0, 0], wgpu::TextureFormat::Rgba8Unorm),
            time: 0.0,
            playing: Vec::new(),
            posed: Vec::new(),
            clock: 0.0,
        }
    }

    /// Poses the playing models for `now` seconds on a steady clock; true while any still moves.
    pub fn animate_models(&mut self, now: f64) -> bool {
        self.clock = now;
        self.pose_models()
    }

    fn pose_models(&mut self) -> bool {
        let (mut moving, mut changed) = (false, false);
        for (p, start, last) in &mut self.playing {
            let Some(clip) = p.model.rig.as_ref().and_then(|r| r.clips.get(p.clip)) else { continue };
            let elapsed = self.clock - *start.get_or_insert(self.clock);
            let t = p.repeat.clip_time(elapsed, clip.duration);
            moving |= p.repeat.moving(elapsed, clip.duration);
            changed |= last.replace(t) != Some(t);
        }
        if changed {
            let mut batches = std::collections::BTreeMap::new();
            for (p, _, last) in &self.playing {
                let pose = p.model.rig.as_ref().and_then(|r| r.pose(p.clip, last.unwrap_or(0.0)));
                scene::push_model(&mut batches, &mut Vec::new(), &p.model, &p.at, pose.as_deref(), true, SELECTED_EDGE);
            }
            // ponytail: fresh buffers per posed frame; reuse them if many models play at once.
            self.posed = (batches.into_iter().filter(|(_, v)| !v.is_empty()))
                .map(|(key, v)| Batch {
                    key,
                    buffer: self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("posed"),
                        contents: bytemuck::cast_slice(&v),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
                    count: v.len() as u32,
                    brush: false,
                })
                .collect();
        }
        moving
    }

    /// Compiles each shaded node's source, reusing what earlier scenes compiled.
    fn set_shaders(&mut self, shaders: &std::collections::BTreeMap<NodeId, (String, mapcraft_map::ShaderParams)>) {
        let mut old = std::mem::take(&mut self.godot);
        self.shaded.clear();
        for (id, (source, params)) in shaders {
            // Parameter values are compiled in as constants, so each source and parameter set is its own pipeline.
            let key = format!("{}\n{source}", serde_json::to_string(params).unwrap_or_default());
            self.shaded.insert(*id, key.clone());
            if self.godot.contains_key(&key) {
                continue;
            }
            let full = format!("Only {MAX_SHADERS} different shaders draw at once");
            let compiled = match old.remove(&key).filter(|c| c.as_ref().err() != Some(&full)) {
                Some(c) => c,
                None if self.godot.len() >= MAX_SHADERS => Err(full),
                None => self.compile_godot(source, params),
            };
            self.godot.insert(key, compiled);
        }
    }

    /// The node's shader when it compiled to the kind its node draws: a sky on the world, spatial elsewhere.
    fn godot_for(&self, id: NodeId) -> Result<&GodotShader, String> {
        let source = self.shaded.get(&id).ok_or_else(String::new)?;
        let shader = self.godot.get(source).ok_or_else(String::new)?.as_ref().map_err(Clone::clone)?;
        match (id == ROOT, shader.kind) {
            (true, ShaderKind::Sky) | (false, ShaderKind::Spatial) => Ok(shader),
            (true, ShaderKind::Spatial) => Err("The world takes a sky shader (shader_type sky;)".into()),
            (false, ShaderKind::Sky) => Err("A brush takes a spatial shader (shader_type spatial;)".into()),
        }
    }

    /// Why a node's embedded shader does not draw; `None` when it draws or there is none.
    pub fn shader_error(&self, id: NodeId) -> Option<String> {
        self.shaded.contains_key(&id).then(|| self.godot_for(id).err()).flatten()
    }

    /// Whether any Godot shader draws, so the views keep TIME moving.
    pub fn animating(&self) -> bool {
        self.shaded.keys().any(|id| self.godot_for(*id).is_ok())
    }

    fn compile_godot(&self, source: &str, params: &mapcraft_map::ShaderParams) -> Result<GodotShader, String> {
        let (kind, textures, module) = godot_shader::compile(source, params)?;
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let internal = self.device.push_error_scope(wgpu::ErrorFilter::Internal);
        let entries: Vec<wgpu::BindGroupLayoutEntry> = (0..textures.len() as u32)
            .flat_map(|i| {
                [
                    wgpu::BindGroupLayoutEntry {
                        binding: i * 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: i * 2 + 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ]
            })
            .collect();
        let layout = (!textures.is_empty())
            .then(|| self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("godot shader textures"), entries: &entries }));
        let pipeline_layout = layout.as_ref().map(|l| {
            self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("godot shader"),
                bind_group_layouts: &[Some(&self.globals_layout), Some(l)],
                immediate_size: 0,
            })
        });
        let fragment = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("godot shader"),
            source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module)),
        });
        let mesh_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32x4, 4 => Float32x4, 5 => Float32x2, 6 => Float32];
        let mesh_buffers = [Some(vertex_layout(std::mem::size_of::<MeshVertex>(), &mesh_attrs))];
        let (vs, buffers, depth): (_, &[_], _) = match kind {
            ShaderKind::Sky => ("vs_sky", &[], (false, false)),
            ShaderKind::Spatial => ("vs_mesh", &mesh_buffers, (true, true)),
        };
        let pipeline = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("godot shader"),
            layout: Some(pipeline_layout.as_ref().unwrap_or(&self.globals_only)),
            vertex: wgpu::VertexState { module: &self.shader, entry_point: Some(vs), compilation_options: Default::default(), buffers },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(depth.0),
                depth_compare: Some(if depth.1 { wgpu::CompareFunction::GreaterEqual } else { wgpu::CompareFunction::Always }),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState { count: SAMPLES, ..Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &fragment,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: COLOR_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let internal = pollster::block_on(internal.pop());
        if let Some(e) = internal.or(pollster::block_on(validation.pop())) {
            return Err(format!("The graphics device rejected the shader: {e}"));
        }
        Ok(GodotShader { kind, pipeline, layout, textures })
    }

    /// Sets a Godot shader's pipeline and textures on `pass`; `screen` is the frame drawn so far, black before there is one.
    fn bind_godot(&self, pass: &mut wgpu::RenderPass, preview: &GodotShader, screen: Option<&wgpu::TextureView>) {
        pass.set_pipeline(&preview.pipeline);
        let Some(layout) = &preview.layout else { return };
        let views: Vec<&wgpu::TextureView> = preview
            .textures
            .iter()
            .map(|t| match t {
                TextureSource::White => &self.white_albedo,
                TextureSource::Black => &self.black,
                TextureSource::Transparent => &self.clear,
                TextureSource::Normal => &self.flat,
                TextureSource::Screen => screen.unwrap_or(&self.black),
                TextureSource::Material(m) => self.materials.get(m).map_or(&self.missing_albedo, |m| &m.albedo),
            })
            .collect();
        let entries: Vec<wgpu::BindGroupEntry> = views
            .iter()
            .enumerate()
            .flat_map(|(i, view)| {
                [
                    wgpu::BindGroupEntry { binding: i as u32 * 2, resource: wgpu::BindingResource::TextureView(view) },
                    wgpu::BindGroupEntry { binding: i as u32 * 2 + 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                ]
            })
            .collect();
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("godot shader textures"), layout, entries: &entries });
        pass.set_bind_group(1, &group, &[]);
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
        let (bind_group, parameters) =
            material_bind_group(&self.device, &self.texture_layout, self.sampler_for(pixelated), &albedo, &self.flat, &self.rough, None);
        let clear_pixels = rgba.chunks(4).any(|p| p.get(3).is_some_and(|a| *a < 255));
        self.materials.insert(name.to_string(), Material { bind_group, parameters, size: [w, h], albedo, pixelated, clear_pixels, see_through: clear_pixels });
        true
    }

    /// Materials with clear pixels or a tint alpha below 1, whose faces hide nothing behind them.
    pub fn see_through_materials(&self) -> impl Iterator<Item = &str> {
        self.materials.iter().filter(|(_, m)| m.see_through).map(|(n, _)| n.as_str())
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
        let (bind_group, parameters) = material_bind_group(&self.device, &self.texture_layout, self.sampler_for(pixelated), &far, &near, &self.rough, None);
        let material = Material { bind_group, parameters, size: [w, h], albedo: far, pixelated, clear_pixels: false, see_through: false };
        self.materials.insert(name.to_string(), material);
        self.skies.insert(name.to_string());
        true
    }

    /// Gives an uploaded material Lit's maps: a normal map with green up, and occlusion, roughness and metallic in RGB.
    pub fn set_material_maps(&mut self, name: &str, normal: (u32, u32, &[u8]), orm: (u32, u32, &[u8])) -> bool {
        self.set_material_surface(name, normal, orm, (1, 1, &[0, 0, 0, 255]), MaterialValues::default())
    }

    /// Gives a material its PBR maps, linear tint, and emission: the texture plus the RGB color, times the energy in A, as Godot does.
    pub fn set_material_surface(
        &mut self,
        name: &str,
        normal: (u32, u32, &[u8]),
        orm: (u32, u32, &[u8]),
        emission: (u32, u32, &[u8]),
        values: MaterialValues,
    ) -> bool {
        if !self.fits(normal.0, normal.1, normal.2)
            || !self.fits(orm.0, orm.1, orm.2)
            || !self.fits(emission.0, emission.1, emission.2)
            || !values.uniform().as_flattened().iter().all(|v| v.is_finite())
        {
            return false;
        }
        let Some(m) = self.materials.get_mut(name) else { return false };
        m.see_through = m.clear_pixels || values.tint[3] < 1.0;
        let linear = |(w, h, rgba)| texture(&self.device, &self.queue, w, h, rgba, wgpu::TextureFormat::Rgba8Unorm);
        let emitted = texture(&self.device, &self.queue, emission.0, emission.1, emission.2, wgpu::TextureFormat::Rgba8UnormSrgb);
        let sampler = if m.pixelated || self.pixelated { &self.nearest } else { &self.sampler };
        (m.bind_group, m.parameters) =
            material_bind_group(&self.device, &self.texture_layout, sampler, &m.albedo, &linear(normal), &linear(orm), Some((&emitted, values)));
        true
    }

    /// Changes an uploaded material's values in place, so a slider drag uploads no pictures.
    pub fn set_material_values(&mut self, name: &str, values: MaterialValues) -> bool {
        let Some(m) = self.materials.get_mut(name) else { return false };
        let uniform = values.uniform();
        if !uniform.as_flattened().iter().all(|v| v.is_finite()) {
            return false;
        }
        m.see_through = m.clear_pixels || values.tint[3] < 1.0;
        self.queue.write_buffer(&m.parameters, 0, bytemuck::cast_slice(&uniform));
        true
    }

    /// Drops every uploaded material, so the next scene uploads them again from a new listing.
    pub fn forget_materials(&mut self) {
        self.materials.clear();
        self.skies.clear();
    }

    /// Drops one uploaded material, so the next scene uploads it again.
    pub fn forget_material(&mut self, name: &str) {
        self.materials.remove(name);
        self.skies.remove(name);
    }

    /// Switches texture magnification between smooth and pixelated; uploaded materials are dropped
    /// to be uploaded again with the new sampler.
    pub fn set_pixelated(&mut self, on: bool) {
        if on == self.pixelated {
            return;
        }
        self.pixelated = on;
        let bind = |albedo: &wgpu::TextureView| {
            material_bind_group(&self.device, &self.texture_layout, self.sampler_for(false), albedo, &self.flat, &self.rough, None).0
        };
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
        for warning in &scene.environment_warnings {
            log::warn!("{warning}");
        }
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
        self.set_shaders(&scene.shaders);
        let old = std::mem::take(&mut self.playing);
        let same = |a: &Playing, b: &Playing| a.id == b.id && a.clip == b.clip && std::sync::Arc::ptr_eq(&a.model, &b.model);
        self.playing = (scene.playing.iter()).map(|p| (p.clone(), old.iter().find(|(o, ..)| same(o, p)).and_then(|(_, start, _)| *start), None)).collect();
        self.posed.clear();
        self.pose_models();
    }

    /// Draws the live brushes in Lit while the bake shows an older map, keeping the bake for later.
    pub fn set_bake_stale(&mut self, stale: bool) {
        self.bake_stale = stale;
    }

    /// Shows a compiled map in the Lit view in place of the brushes, or the brushes again with `None`.
    pub fn set_bake(&mut self, bake: Option<&Bake>) {
        (self.bake, self.lightmaps) = (Vec::new(), Vec::new());
        let Some(bake) = bake else { return };
        self.bake_linear = bake.linear;
        let format = if bake.linear { wgpu::TextureFormat::Rgb9e5Ufloat } else { wgpu::TextureFormat::Rgba8Unorm };
        let black = Picture { width: 1, height: 1, rgba: vec![0, 0, 0, 255] };
        for p in &bake.lightmaps {
            let p = if self.fits(p.width, p.height, &p.rgba) { p } else { &black };
            let view = single_level(&self.device, &self.queue, p.width, p.height, &p.rgba, format);
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
            self.bake.push(BakedBatch { material: part.material.clone(), anti_tile: part.anti_tile, lightmap, buffer, count: verts.len() as u32 });
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
        let screen = tex("viewport screen", COLOR_FORMAT, 1, wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING, &[]);
        let screen_view = screen.create_view(&Default::default());
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
        Target { size, msaa, depth, texture, view, display, screen, screen_view, globals, globals_bind }
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
        let (layers, anti_tile) = if mode == RenderMode::Flat {
            (1, AntiTile::Off)
        } else {
            (if key.blend.is_empty() { 1 } else { 2 }, self.drawn_anti_tile(key.anti_tile, [&key.material, &key.blend]))
        };
        [(lit, layers, anti_tile), (lit, layers, lighter(anti_tile)), (lit, layers, AntiTile::Off), (lit, 1, AntiTile::Off)]
            .iter()
            .find_map(|k| self.pipelines.get(k))
    }

    /// The baked variant of `fs` a batch draws with, or the closest lighter one when the device rejected it.
    fn baked_pipeline(&self, fs: &'static str, batch: &BakedBatch) -> Option<&wgpu::RenderPipeline> {
        let anti_tile = self.drawn_anti_tile(batch.anti_tile, [&batch.material, ""]);
        [anti_tile, lighter(anti_tile), AntiTile::Off].iter().find_map(|m| self.baked_pipelines.get(&(fs, *m)))
    }

    fn drawn_anti_tile(&self, anti_tile: AntiTile, materials: [&str; 2]) -> AntiTile {
        // Three blended taps of big nearest texels read as double images.
        let pixelated = self.pixelated || materials.iter().any(|n| self.materials.get(*n).is_some_and(|m| m.pixelated));
        if anti_tile == AntiTile::Hex && pixelated { AntiTile::Macro } else { anti_tile }
    }

    /// Draws the scene into `target`; `grid` is the 2D grid spacing, `overlay` lines draw on top.
    pub fn render(&self, target: &Target, camera: &Camera, mode: RenderMode, grid: Option<f64>, overlay: &[LineVertex]) {
        let size = DVec2::new(f64::from(target.size[0]), f64::from(target.size[1]));
        let lit = mode == RenderMode::Lit;
        let baked = lit && !camera.is_2d() && !self.bake.is_empty() && !self.bake_stale;
        let sky = self.sky.filter(|_| lit && !camera.is_2d());
        let mut globals = Globals::unlit(camera, size, self.weight_overlay);
        (globals.paint[1], globals.paint[2], globals.paint[3]) = (self.time, size.x as f32, size.y as f32);
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
                let effects = s.effects;
                globals.fog_color = [effects.fog_color[0], effects.fog_color[1], effects.fog_color[2], effects.fog_energy];
                globals.fog = [effects.fog_density, effects.fog_sun_scatter, effects.fog_sky_affect, effects.fog_mode as f32];
                let [curve, begin, end] = effects.fog_depth;
                globals.fog_depth = [curve, begin, end, effects.fog_aerial_perspective];
                globals.fog_height = [effects.fog_height[0], effects.fog_height[1], 0.0, 0.0];
                globals.tonemap = [effects.tonemap as f32, effects.exposure, effects.white, 0.0];
                globals.adjustments = [effects.brightness, effects.contrast, effects.saturation, 0.0];
                globals.sky_shape = [effects.sky_curve, effects.ground_curve, effects.sky_energy, effects.ground_energy];
                for value in globals.ambient.iter_mut().take(3) {
                    *value *= effects.sky_energy;
                }
                for value in globals.ground.iter_mut().take(3) {
                    *value *= effects.ground_energy;
                }
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
            let solid = !camera.is_2d() && mode != RenderMode::Wireframe;
            // Shaded faces draw in a second pass, so their screen texture can hold everything drawn before them.
            let shaded: Vec<(&Batch, &GodotShader)> =
                self.batches.iter().filter(|b| solid && !(baked && b.brush)).filter_map(|b| Some((b, self.godot_for(b.key.shader?).ok()?))).collect();
            let mut pass = begin_pass(&mut encoder, target, Some(bg), !shaded.is_empty());
            let godot_sky = self.godot_for(ROOT).ok().filter(|_| solid && self.show_sky);
            if let Some(p) = godot_sky {
                self.bind_godot(&mut pass, p, None);
                pass.draw(0..3, 0..1);
            } else if sky.is_some() && self.show_sky {
                pass.set_pipeline(&self.sky_pipeline);
                pass.draw(0..3, 0..1);
            }
            if let Some(b) = &grid_buf {
                pass.set_pipeline(&self.overlay_pipeline);
                pass.set_vertex_buffer(0, b.slice(..));
                pass.draw(0..extra.len() as u32, 0..1);
            }
            // See-through batches draw last without writing depth, so whatever lies behind them is already drawn.
            let draw_batches = |pass: &mut wgpu::RenderPass<'_>, see_through: bool| {
                if camera.is_2d() || mode == RenderMode::Wireframe {
                    return;
                }
                for batch in self.batches.iter().filter(|b| !(baked && b.brush)).chain(&self.posed) {
                    let material = batch.key.material.as_str();
                    if shaded.iter().any(|(b, _)| std::ptr::eq(*b, batch)) {
                        continue;
                    }
                    let sky = self.skies.contains(material);
                    if (mapcraft_map::is_tool_material(material) || self.materials.get(material).is_some_and(|m| m.see_through)) != see_through {
                        continue;
                    }
                    let sky_face = mode != RenderMode::Flat && sky && self.show_sky;
                    let lit = lit && !material.is_empty() && !sky;
                    // Entity boxes stay unlit, or a light's own box would be dark.
                    let pipeline = if sky_face {
                        Some(&self.sky_face_pipeline)
                    } else if see_through {
                        self.see_through_pipelines.get(&lit)
                    } else {
                        self.mesh_pipeline(lit, mode, &batch.key)
                    };
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
            };
            draw_batches(&mut pass, false);
            for batch in self.bake.iter().filter(|_| baked) {
                let lightmap = batch.lightmap.and_then(|l| self.lightmaps.get(l));
                let pipeline = match lightmap {
                    _ if self.skies.contains(&batch.material) => Some(&self.baked_sky_pipeline),
                    Some(_) if self.bake_linear => self.baked_pipeline("fs_baked_linear", batch),
                    Some(_) => self.baked_pipeline("fs_baked", batch),
                    None => self.baked_pipeline("fs_baked_unlit", batch),
                };
                let Some(pipeline) = pipeline else { continue };
                pass.set_pipeline(pipeline);
                pass.set_bind_group(1, self.materials.get(&batch.material).map_or(&self.missing, |m| &m.bind_group), &[]);
                if let Some(l) = lightmap.filter(|_| !self.skies.contains(&batch.material)) {
                    pass.set_bind_group(2, l, &[]);
                }
                pass.set_vertex_buffer(0, batch.buffer.slice(..));
                pass.draw(0..batch.count, 0..1);
            }
            if !shaded.is_empty() {
                drop(pass);
                encoder.copy_texture_to_texture(target.texture.as_image_copy(), target.screen.as_image_copy(), target.texture.size());
                pass = begin_pass(&mut encoder, target, None, false);
                for (batch, shader) in &shaded {
                    self.bind_godot(&mut pass, shader, Some(&target.screen_view));
                    pass.set_vertex_buffer(0, batch.buffer.slice(..));
                    pass.draw(0..batch.count, 0..1);
                }
            }
            draw_batches(&mut pass, true);
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
        scene::push_model(&mut batches, &mut Vec::new(), model, &DMat4::IDENTITY, None, false, WORLD_EDGE);
        let textures: HashMap<String, wgpu::BindGroup> = (model.textures.iter().enumerate())
            .filter(|(_, p)| self.fits(p.width, p.height, &p.rgba))
            .map(|(i, p)| {
                let albedo = texture(&self.device, &self.queue, p.width, p.height, &p.rgba, wgpu::TextureFormat::Rgba8UnormSrgb);
                (
                    model.texture_name(i),
                    material_bind_group(&self.device, &self.texture_layout, self.sampler_for(false), &albedo, &self.flat, &self.rough, None).0,
                )
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
            let mut pass = begin_pass(&mut encoder, &target, Some(wgpu::Color::TRANSPARENT), false);
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
            fog_color: [0.0; 4],
            fog: [0.0; 4],
            fog_depth: [0.0; 4],
            fog_height: [0.0; 4],
            tonemap: [2.0, 1.0, 1.0, 0.0],
            adjustments: [1.0, 1.0, 1.0, 0.0],
            sky_shape: [0.15, 0.02, 1.0, 1.0],
            minlight: [0.0; 4],
            paint: [paint, 0.0, 0.0, 0.0],
            lights: [Light::default(); MAX_LIGHTS],
        }
    }
}

/// A pass drawing into `target` cleared to `bg`, with the target's globals bound.
/// `clear` of `None` keeps what an earlier pass `keep`-ed.
fn begin_pass<'a>(encoder: &'a mut wgpu::CommandEncoder, target: &'a Target, clear: Option<wgpu::Color>, keep: bool) -> wgpu::RenderPass<'a> {
    let store = if keep { wgpu::StoreOp::Store } else { wgpu::StoreOp::Discard };
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("viewport"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &target.msaa,
            depth_slice: None,
            resolve_target: Some(&target.view),
            ops: wgpu::Operations { load: clear.map_or(wgpu::LoadOp::Load, wgpu::LoadOp::Clear), store },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: &target.depth,
            depth_ops: Some(wgpu::Operations { load: if clear.is_some() { wgpu::LoadOp::Clear(0.0) } else { wgpu::LoadOp::Load }, store }),
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
/// A lightmap texture of 4-byte texels: RGBA8, or RGB9E5 for linear light.
fn single_level(device: &wgpu::Device, queue: &wgpu::Queue, w: u32, h: u32, rgba: &[u8], format: wgpu::TextureFormat) -> wgpu::TextureView {
    device
        .create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("lightmap"),
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
    surface: Option<(&wgpu::TextureView, MaterialValues)>,
) -> (wgpu::BindGroup, wgpu::Buffer) {
    let (emission, values) = surface.unwrap_or((albedo, MaterialValues::default()));
    let parameters = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("material parameters"),
        contents: bytemuck::cast_slice(&values.uniform()),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("material"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(albedo) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(normal) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(orm) },
            wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(emission) },
            wgpu::BindGroupEntry { binding: 5, resource: parameters.as_entire_binding() },
        ],
    });
    (group, parameters)
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
    fn embedded_godot_shaders_draw_per_node_on_available_adapter() {
        use mapcraft_map::{Brush, Map, NodeKind, TexDefaults};
        let Ok(adapter) = pollster::block_on(wgpu::Instance::default().request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("skipped: no GPU adapter");
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut renderer = Renderer::new(&device, &queue);
        let sky = "shader_type sky;\nuniform vec3 c : source_color = vec3(1.0, 0.0, 0.0);\nvoid sky() { COLOR = c; }\n";
        let surface = "shader_type spatial;\nuniform sampler2D screen_texture : hint_screen_texture;\nuniform sampler2D n : hint_normal;\nvoid fragment() { ALBEDO = texture(screen_texture, SCREEN_UV).rgb + texture(n, UV).rgb * abs(sin(TIME)); }\n";
        let mut map = Map::new();
        let mut e = map.edit();
        let far = |x: f64| NodeKind::Brush(Brush::cuboid(mapcraft_geom::Aabb::new(DVec3::splat(x), DVec3::splat(x + 64.0)), &TexDefaults::default()));
        let [good, broken, wrong] = [1e5, 2e5, 3e5].map(|x| e.insert(ROOT, far(x)).unwrap());
        e.set_shader(ROOT, Some(sky.into())).unwrap();
        e.set_shader(good, Some(surface.into())).unwrap();
        e.set_shader(broken, Some("shader_type spatial;\nvoid fragment() { ALBEDO = 1; }".into())).unwrap();
        e.set_shader(wrong, Some(sky.into())).unwrap();
        map.apply(e.finish());
        let scene = |map: &Map| build_scene(map, &Default::default(), &Default::default(), &|_| None, &NoStyle, false, None);
        renderer.set_scene(&scene(&map));
        let target = renderer.target([8, 8]);
        renderer.render(&target, &Camera::new(View::Perspective), RenderMode::Lit, None, &[]);
        let pixels = renderer.read_rgba(&target).unwrap();
        assert_eq!(&pixels[..4], &[255, 0, 0, 255], "the world's sky shader replaces the sky");
        assert_eq!((renderer.shader_error(ROOT), renderer.shader_error(good)), (None, None));
        assert!(renderer.shader_error(broken).is_some());
        assert!(renderer.shader_error(wrong).is_some_and(|e| e.contains("spatial")));
        assert_eq!(renderer.godot.len(), 3, "one compile per distinct source");
        assert!(renderer.animating());
        let mut e = map.edit();
        e.set_shader_param(ROOT, "c", Some(serde_json::json!([0, 0, 1]))).unwrap();
        e.set_shader(good, Some(surface.replace("uniform sampler2D n : hint_normal;", "uniform sampler2D n;"))).unwrap();
        e.set_shader_param(good, "n", Some(serde_json::json!("stone"))).unwrap();
        let undo = map.apply(e.finish());
        assert!(renderer.upload_material("stone", 1, 1, &[9, 9, 9, 255]));
        renderer.set_scene(&scene(&map));
        renderer.render(&target, &Camera::new(View::Perspective), RenderMode::Lit, None, &[]);
        assert_eq!(&renderer.read_rgba(&target).unwrap()[..4], &[0, 0, 255, 255], "a parameter replaces the uniform's default");
        assert_eq!(renderer.shader_error(good), None, "a texture parameter binds the material");
        map.apply(undo);

        let mut e = map.edit();
        for i in 0..MAX_SHADERS {
            let id = e.insert(ROOT, far(4e5 + i as f64 * 100.0)).unwrap();
            e.set_shader(id, Some(format!("shader_type spatial;\nvoid fragment() {{ ALBEDO = vec3({i}.0); }}\n"))).unwrap();
        }
        map.apply(e.finish());
        renderer.set_scene(&scene(&map));
        assert_eq!(renderer.godot.len(), 3 + MAX_SHADERS, "sources past the cap are refused, not compiled");
        assert!(renderer.godot.values().filter(|c| c.as_ref().err().is_some_and(|e| e.contains("draw at once"))).count() >= 3);
        renderer.render(&target, &Camera::new(View::Perspective), RenderMode::Lit, None, &[]);
        renderer.set_scene(&scene(&Map::new()));
        assert!(renderer.godot.is_empty() && !renderer.animating());
        assert!(pollster::block_on(validation.pop()).is_none());
    }

    #[test]
    fn pbr_surface_bindings_validate_on_available_adapter() {
        let Ok(adapter) = pollster::block_on(wgpu::Instance::default().request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("skipped: no GPU adapter");
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut renderer = Renderer::new(&device, &queue);
        assert!(renderer.upload_material("pbr", 1, 1, &[255; 4]));
        assert!(renderer.upload_material("grate", 2, 1, &[255, 255, 255, 255, 0, 0, 0, 0]));
        assert_eq!(renderer.see_through_materials().collect::<Vec<_>>(), ["grate"], "clear pixels make a material see-through");
        assert!(renderer.set_material_surface(
            "pbr",
            (1, 1, &[128, 128, 255, 255]),
            (1, 1, &[255, 128, 192, 255]),
            (1, 1, &[255; 4]),
            MaterialValues { tint: [0.5, 0.3, 0.1, 1.0], emission: [2.0, 0.0, 0.0, 1.0], ..MaterialValues::default() }
        ));
        assert!(!renderer.set_material_surface("pbr", (0, 1, &[]), (1, 1, &[255; 4]), (1, 1, &[255; 4]), MaterialValues::default()));
        assert!(renderer.set_material_surface(
            "pbr",
            (1, 1, &[128, 128, 255, 255]),
            (1, 1, &[255; 4]),
            (1, 1, &[0, 0, 0, 255]),
            MaterialValues { tint: [1.0, 1.0, 1.0, 0.5], ..MaterialValues::default() }
        ));
        let mut see: Vec<&str> = renderer.see_through_materials().collect();
        see.sort_unstable();
        assert_eq!(see, ["grate", "pbr"], "a tint alpha below 1 does too");
        assert!(renderer.set_material_values("pbr", MaterialValues { normal_strength: 2.0, ..MaterialValues::default() }));
        assert_eq!(renderer.see_through_materials().count(), 1, "values in place change see-through too");
        assert!(!renderer.set_material_values("pbr", MaterialValues { roughness: f32::NAN, ..MaterialValues::default() }));
        assert!(!renderer.set_material_values("nope", MaterialValues::default()));
        assert!(pollster::block_on(validation.pop()).is_none());
    }

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
            ..BakePart::default()
        };
        r.set_bake(Some(&Bake { parts: vec![quad], lightmaps: vec![Picture { width: 1, height: 1, rgba: vec![64, 128, 255, 255] }], linear: false }));
        let target = r.target([4, 4]);
        let camera = Camera { position: DVec3::new(0.0, 0.0, 100.0), pitch: -89.0, ..Camera::new(View::Perspective) };
        r.render(&target, &camera, RenderMode::Lit, None, &[]);
        let rgba = r.read_rgba(&target).unwrap();
        let pixel = &rgba[(2 * 4 + 2) * 4..(2 * 4 + 2) * 4 + 3];
        assert!(pixel.iter().all(|c| c.abs_diff(100) <= 1), "200, 100 and 50 lit by 64, 128 and 255 all come to 100: {pixel:?}");
        r.render(&target, &camera, RenderMode::Textured, None, &[]);
        assert_ne!(&r.read_rgba(&target).unwrap()[(2 * 4 + 2) * 4..(2 * 4 + 2) * 4 + 3], pixel, "only the Lit view shows the bake");
    }

    #[test]
    fn a_bake_keeps_hex_tiling() {
        let Ok(adapter) = pollster::block_on(wgpu::Instance::default().request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("skipped: no GPU adapter");
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let mut r = Renderer::new(&device, &queue);
        let ramp: Vec<u8> = (0..16 * 16).flat_map(|i: u32| [(i % 16 * 16) as u8, (i / 16 * 16) as u8, 0, 255]).collect();
        r.upload_material("m", 16, 16, &ramp);
        let target = r.target([32, 32]);
        let camera = Camera { position: DVec3::new(0.0, 0.0, 100.0), pitch: -89.0, ..Camera::new(View::Perspective) };
        let mut shot = |anti_tile| {
            let quad = BakePart {
                material: "m".into(),
                anti_tile,
                lightmap: Some(0),
                positions: vec![[-1000.0, -1000.0, 0.0], [1000.0, -1000.0, 0.0], [1000.0, 1000.0, 0.0], [-1000.0, 1000.0, 0.0]],
                uvs: vec![[0.0, 0.0], [40.0, 0.0], [40.0, 40.0], [0.0, 40.0]],
                lightmap_uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                indices: vec![0, 1, 2, 0, 2, 3],
            };
            r.set_bake(Some(&Bake { parts: vec![quad], lightmaps: vec![Picture { width: 1, height: 1, rgba: vec![128; 4] }], linear: false }));
            r.render(&target, &camera, RenderMode::Lit, None, &[]);
            r.read_rgba(&target).unwrap()
        };
        assert_ne!(shot(AntiTile::Off), shot(AntiTile::Hex), "hex tiling turns the baked repeats");
    }

    #[test]
    fn a_bake_keeps_the_normal_map() {
        let Ok(adapter) = pollster::block_on(wgpu::Instance::default().request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("skipped: no GPU adapter");
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let mut r = Renderer::new(&device, &queue);
        let quad = BakePart {
            material: "m".into(),
            lightmap: Some(0),
            positions: vec![[-1000.0, -1000.0, 0.0], [1000.0, -1000.0, 0.0], [1000.0, 1000.0, 0.0], [-1000.0, 1000.0, 0.0]],
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            lightmap_uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            indices: vec![0, 1, 2, 0, 2, 3],
            ..BakePart::default()
        };
        let quarter: u32 = 256 | 256 << 9 | 256 << 18 | 14 << 27;
        let lightmap = Picture { width: 1, height: 1, rgba: quarter.to_le_bytes().to_vec() };
        r.set_bake(Some(&Bake { parts: vec![quad], lightmaps: vec![lightmap], linear: true }));
        let target = r.target([4, 4]);
        let camera = Camera { position: DVec3::new(0.0, 0.0, 100.0), pitch: -89.0, ..Camera::new(View::Perspective) };
        let mut shade = |normal: [u8; 4]| {
            r.upload_material("m", 1, 1, &[200; 4]);
            assert!(r.set_material_maps("m", (1, 1, &normal), (1, 1, &[255, 255, 0, 255])));
            r.render(&target, &camera, RenderMode::Lit, None, &[]);
            r.read_rgba(&target).unwrap()[(2 * 4 + 2) * 4..(2 * 4 + 2) * 4 + 3].to_vec()
        };
        let flat = shade([128, 128, 255, 255]);
        let toward_key_sun = shade([180, 56, 220, 255]);
        let away = shade([76, 200, 220, 255]);
        assert!(toward_key_sun[0] > flat[0] && flat[0] > away[0], "the normal map shades the baked face: {away:?} {flat:?} {toward_key_sun:?}");
    }
}
