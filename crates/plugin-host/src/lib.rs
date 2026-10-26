//! Runs MapCraft plugins: WebAssembly components built against `wit/plugin.wit`.
//!
//! Each plugin gets its own store with a memory cap and a fuel budget per call, and sees the SDK's
//! asset folders through the host's read-only file functions; WASI is linked with no folders,
//! environment, arguments or network.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod convert;

use std::path::Path;

use mapcraft_sdk::Vfs;
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

pub use convert::{CommandSpec, Context, ImporterSpec, Manifest, ParamKind, ParamSpec};

/// A material a plugin lists: a preview image path in the asset folders, or else an sRGB color.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialInfo {
    pub name: String,
    pub image: Option<String>,
    pub color: Option<[u8; 3]>,
    /// The picture itself, for materials from files outside the SDK's asset folders.
    pub pixels: Option<std::sync::Arc<Picture>>,
    /// The texture's own size when the preview is smaller.
    pub size: Option<[u32; 2]>,
    pub pbr: Option<Pbr>,
}

/// Metallic-roughness maps for the Lit view, as asset paths the editor reads when a map uses the material.
#[derive(Clone, Debug, PartialEq)]
pub struct Pbr {
    /// Green points up the picture (OpenGL) unless `normal_dx`.
    pub normal: Option<String>,
    pub normal_dx: bool,
    /// The map's channel times the value, or the value alone; values are in 0 to 1.
    pub roughness: Option<ChannelMap>,
    pub roughness_value: f32,
    pub metallic: Option<ChannelMap>,
    pub metallic_value: f32,
    pub ao: Option<ChannelMap>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChannelMap {
    pub path: String,
    pub channel: Channel,
}

/// Which channel of a picture holds a value; `Gray` averages red, green and blue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Red,
    Green,
    Blue,
    Alpha,
    Gray,
}

/// A problem a plugin's `validate` found; `node` is the node at fault, when there is one.
#[derive(Clone, Debug, PartialEq)]
pub struct PluginIssue {
    pub error: bool,
    pub message: String,
    pub node: Option<mapcraft_map::NodeId>,
}

/// An RGBA8 picture, rows top to bottom.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// A model a plugin read, in map units around the entity's origin.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PluginModel {
    pub parts: Vec<PluginModelPart>,
    pub textures: Vec<Picture>,
}

/// Triangles sharing one texture; `indices` are checked to stay within `positions`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PluginModelPart {
    pub texture: Option<usize>,
    pub positions: Vec<mapcraft_geom::DVec3>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

mod bindings {
    wasmtime::component::bindgen!({ path: "../../wit", world: "plugin" });
}

use bindings::Plugin as Bindings;
use bindings::mapcraft::plugin::{host, types};

/// Fuel for one call; roughly a few seconds of work.
const FUEL: u64 = 20_000_000_000;
const MEMORY: usize = 512 << 20;
/// An import may use this much more memory than the file it reads.
const IMPORT_HEADROOM: usize = 256 << 20;
const MAX_RANGE: u64 = 256 << 20;
/// Fuel for one import; decompressing and decoding a big compiled map takes far more than a command.
const IMPORT_FUEL: u64 = 60_000_000_000;

/// What an importer read: nodes in map space, and pictures for materials the SDK lacks.
#[derive(Debug, Default)]
pub struct Imported {
    pub nodes: Vec<mapcraft_format::Fragment>,
    pub materials: Vec<ImportedMaterial>,
}

/// An RGBA8 picture for a material an import brought.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportedMaterial {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// The texture's own size when the picture is smaller.
    pub size: Option<[u32; 2]>,
}

#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("{0}")]
    Wasm(String),
    #[error("plugin `{plugin}` failed: {message}")]
    Failed { plugin: String, message: String },
    #[error("plugin `{0}` has no command `{1}`")]
    UnknownCommand(String, String),
    #[error("cannot read {0}: {1}")]
    Io(String, std::io::Error),
}

fn wasm_err(e: impl std::fmt::Display) -> PluginError {
    PluginError::Wasm(e.to_string())
}

struct State {
    wasi: WasiCtx,
    table: ResourceTable,
    limits: StoreLimits,
    vfs: Vfs,
    log: Vec<String>,
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
    }
}

impl types::Host for State {}

impl host::Host for State {
    fn log(&mut self, message: String) {
        if self.log.len() < 10_000 {
            self.log.push(message);
        }
    }

    fn list_files(&mut self, extensions: Vec<String>) -> Vec<String> {
        self.vfs.list(&extensions).into_iter().filter_map(|(name, path)| Some(format!("{name}.{}", path.extension()?.to_string_lossy()))).collect()
    }

    fn read_file(&mut self, path: String) -> Result<Vec<u8>, String> {
        self.vfs.read(&path).ok_or_else(|| format!("{path} not found"))
    }

    fn read_range(&mut self, path: String, offset: u64, length: u64) -> Result<Vec<u8>, String> {
        if length > MAX_RANGE {
            return Err(format!("{length} bytes is more than one read may take"));
        }
        self.vfs.read_range(&path, offset, length).ok_or_else(|| format!("{path} not found"))
    }
}

/// Compiles plugins; one is shared by every plugin the editor loads.
#[derive(Clone)]
pub struct Host {
    engine: Engine,
    linker: Linker<State>,
}

impl Host {
    pub fn new() -> Result<Self, PluginError> {
        Self::with_config(Config::new())
    }

    /// Like [`Host::new`], keeping compiled plugins in `dir` so later launches skip compiling them.
    pub fn with_cache(dir: &Path) -> Result<Self, PluginError> {
        let mut cache = wasmtime::CacheConfig::new();
        cache.with_directory(dir);
        let mut config = Config::new();
        config.cache(Some(wasmtime::Cache::new(cache).map_err(wasm_err)?));
        Self::with_config(config)
    }

    fn with_config(mut config: Config) -> Result<Self, PluginError> {
        config.consume_fuel(true);
        let engine = Engine::new(&config).map_err(wasm_err)?;
        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(wasm_err)?;
        Bindings::add_to_linker::<State, HasSelf<State>>(&mut linker, |s| s).map_err(wasm_err)?;
        Ok(Self { engine, linker })
    }

    pub fn load_file(&self, path: &Path, vfs: Vfs) -> Result<Plugin, PluginError> {
        let bytes = std::fs::read(path).map_err(|e| PluginError::Io(path.display().to_string(), e))?;
        self.load(&bytes, vfs)
    }

    pub fn load(&self, bytes: &[u8], vfs: Vfs) -> Result<Plugin, PluginError> {
        let component = Component::new(&self.engine, bytes).map_err(wasm_err)?;
        let state = State {
            wasi: WasiCtx::builder().build(),
            table: ResourceTable::new(),
            limits: StoreLimitsBuilder::new().memory_size(MEMORY).instances(16).build(),
            vfs,
            log: Vec::new(),
        };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|s| &mut s.limits);
        store.set_fuel(FUEL).map_err(wasm_err)?;
        let bindings = Bindings::instantiate(&mut store, &component, &self.linker).map_err(wasm_err)?;
        let manifest = convert::manifest(bindings.call_manifest(&mut store).map_err(wasm_err)?);
        Ok(Plugin { manifest, store, bindings, fuel: FUEL, broken: false })
    }
}

/// A loaded plugin. After a trap its instance may be inconsistent, so it refuses further calls.
pub struct Plugin {
    pub manifest: Manifest,
    store: Store<State>,
    bindings: Bindings,
    fuel: u64,
    broken: bool,
}

impl Plugin {
    /// Limits the work one call may do; the default allows a few seconds.
    pub fn set_fuel(&mut self, fuel: u64) {
        self.fuel = fuel;
    }

    /// Log lines the plugin wrote since the last call.
    pub fn take_log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.store.data_mut().log)
    }

    fn begin(&mut self) -> Result<(), PluginError> {
        if self.broken {
            return Err(self.failed("it stopped after an earlier error; reload the SDK to restart it"));
        }
        self.store.set_fuel(self.fuel).map_err(wasm_err)
    }

    fn failed(&self, message: impl Into<String>) -> PluginError {
        PluginError::Failed { plugin: self.manifest.id.clone(), message: message.into() }
    }

    fn trapped(&mut self, e: wasmtime::Error) -> PluginError {
        self.broken = true;
        let out_of_fuel = matches!(e.downcast_ref::<wasmtime::Trap>(), Some(wasmtime::Trap::OutOfFuel));
        self.failed(if out_of_fuel { "it ran too long and was stopped".to_string() } else { e.to_string() })
    }

    /// Runs a command; the result is the made nodes, around the origin.
    pub fn run(&mut self, command: &str, params: &serde_json::Value, ctx: &Context) -> Result<Vec<mapcraft_format::Fragment>, PluginError> {
        let spec =
            self.manifest.commands.iter().find(|c| c.id == command).ok_or_else(|| PluginError::UnknownCommand(self.manifest.id.clone(), command.into()))?;
        let params = convert::params(spec, params);
        let wit_ctx = types::Context { grid: ctx.grid, material: ctx.material.clone() };
        self.begin()?;
        let result = self.bindings.call_run_command(&mut self.store, command, &params, &wit_ctx);
        match result {
            Ok(Ok(out)) => Ok(convert::output(out, ctx)),
            Ok(Err(message)) => Err(self.failed(message)),
            Err(e) => Err(self.trapped(e)),
        }
    }

    /// Reads `data`, the file `name`, with one of the plugin's importers; the result is in map space.
    pub fn import(&mut self, importer: &str, name: &str, data: &[u8], ctx: &Context) -> Result<Imported, PluginError> {
        if !self.manifest.importers.iter().any(|i| i.id == importer) {
            return Err(PluginError::UnknownCommand(self.manifest.id.clone(), importer.into()));
        }
        let wit_ctx = types::Context { grid: ctx.grid, material: ctx.material.clone() };
        // The plugin holds the whole file, so big maps need more than the usual cap; wasm32 tops out at 4 GiB.
        let cap = data.len().saturating_add(IMPORT_HEADROOM).clamp(MEMORY, 4 << 30);
        self.store.data_mut().limits = StoreLimitsBuilder::new().memory_size(cap).instances(16).build();
        self.begin()?;
        self.store.set_fuel(IMPORT_FUEL.max(self.fuel)).map_err(wasm_err)?;
        match self.bindings.call_import_file(&mut self.store, importer, name, data, &wit_ctx) {
            Ok(Ok(mut out)) => {
                let materials = convert::images(std::mem::take(&mut out.materials));
                Ok(Imported { nodes: convert::output(out, ctx), materials })
            }
            Ok(Err(message)) => Err(self.failed(message)),
            Err(e) => Err(self.trapped(e)),
        }
    }

    /// Full-size pictures of materials the plugin listed smaller, in the order of `names`; `None`
    /// for one it could not read.
    pub fn load_materials(&mut self, names: &[String]) -> Result<Vec<Option<Picture>>, PluginError> {
        if !self.manifest.materials || names.is_empty() {
            return Ok(vec![None; names.len()]);
        }
        self.begin()?;
        self.store.set_fuel(IMPORT_FUEL.max(self.fuel)).map_err(wasm_err)?;
        match self.bindings.call_load_materials(&mut self.store, names) {
            Ok(list) => Ok(names.iter().enumerate().map(|(i, _)| list.get(i).cloned().flatten().and_then(convert::picture)).collect()),
            Err(e) => Err(self.trapped(e)),
        }
    }

    /// Whether `path` has a model extension this plugin reads.
    pub fn reads_model(&self, path: &str) -> bool {
        path.rsplit_once('.').is_some_and(|(_, ext)| self.manifest.models.iter().any(|m| m.eq_ignore_ascii_case(ext)))
    }

    /// Reads the model at `path` for the views.
    pub fn load_model(&mut self, path: &str) -> Result<PluginModel, PluginError> {
        if !self.reads_model(path) {
            return Err(PluginError::UnknownCommand(self.manifest.id.clone(), path.into()));
        }
        self.begin()?;
        match self.bindings.call_load_model(&mut self.store, path) {
            Ok(Ok(m)) => Ok(convert::model(m)),
            Ok(Err(message)) => Err(self.failed(message)),
            Err(e) => Err(self.trapped(e)),
        }
    }

    /// Writes `map` in one of the plugin's export formats; resolve prefabs and mirrors first.
    pub fn export(&mut self, exporter: &str, map: &mapcraft_map::Map, name: &str, ctx: &Context) -> Result<Vec<u8>, PluginError> {
        if !self.manifest.exporters.iter().any(|i| i.id == exporter) {
            return Err(PluginError::UnknownCommand(self.manifest.id.clone(), exporter.into()));
        }
        let data = convert::map_data(map, name);
        let wit_ctx = types::Context { grid: ctx.grid, material: ctx.material.clone() };
        self.begin()?;
        self.store.set_fuel(IMPORT_FUEL.max(self.fuel)).map_err(wasm_err)?;
        match self.bindings.call_export_file(&mut self.store, exporter, &data, &wit_ctx) {
            Ok(Ok(bytes)) => Ok(bytes),
            Ok(Err(message)) => Err(self.failed(message)),
            Err(e) => Err(self.trapped(e)),
        }
    }

    /// The game's own checks on `map`, which should have prefabs and mirrors resolved.
    pub fn validate(&mut self, map: &mapcraft_map::Map, name: &str) -> Result<Vec<PluginIssue>, PluginError> {
        if !self.manifest.validates {
            return Ok(Vec::new());
        }
        let (data, ids) = convert::map_data_with_ids(map, name);
        self.begin()?;
        match self.bindings.call_validate(&mut self.store, &data) {
            Ok(list) => Ok(list
                .into_iter()
                .take(10_000)
                .map(|i| PluginIssue {
                    error: matches!(i.severity, types::Severity::Error),
                    message: i.message,
                    node: i.node.and_then(|n| ids.get(n as usize).copied()),
                })
                .collect()),
            Err(e) => Err(self.trapped(e)),
        }
    }

    /// The materials the plugin lists.
    pub fn materials(&mut self) -> Result<Vec<MaterialInfo>, PluginError> {
        self.begin()?;
        match self.bindings.call_list_materials(&mut self.store) {
            Ok(Ok(list)) => Ok(list
                .into_iter()
                .map(|m| MaterialInfo {
                    pixels: m.pixels.and_then(convert::picture).map(std::sync::Arc::new),
                    name: m.name,
                    image: m.image,
                    color: m.color.map(|(r, g, b)| [r, g, b]),
                    size: m.size.map(|(w, h)| [w, h]),
                    pbr: m.pbr.map(convert::pbr),
                })
                .collect()),
            Ok(Err(message)) => Err(self.failed(message)),
            Err(e) => Err(self.trapped(e)),
        }
    }
}
