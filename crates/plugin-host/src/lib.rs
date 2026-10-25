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
}

mod bindings {
    wasmtime::component::bindgen!({ path: "../../wit", world: "plugin" });
}

use bindings::Plugin as Bindings;
use bindings::mapcraft::plugin::{host, types};

/// Fuel for one call; roughly a few seconds of work.
const FUEL: u64 = 20_000_000_000;
const MEMORY: usize = 512 << 20;

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
}

/// Compiles plugins; one is shared by every plugin the editor loads.
pub struct Host {
    engine: Engine,
    linker: Linker<State>,
}

impl Host {
    pub fn new() -> Result<Self, PluginError> {
        let mut config = Config::new();
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
    pub fn import(&mut self, importer: &str, name: &str, data: &[u8], ctx: &Context) -> Result<Vec<mapcraft_format::Fragment>, PluginError> {
        if !self.manifest.importers.iter().any(|i| i.id == importer) {
            return Err(PluginError::UnknownCommand(self.manifest.id.clone(), importer.into()));
        }
        let wit_ctx = types::Context { grid: ctx.grid, material: ctx.material.clone() };
        self.begin()?;
        match self.bindings.call_import_file(&mut self.store, importer, name, data, &wit_ctx) {
            Ok(Ok(out)) => Ok(convert::output(out, ctx)),
            Ok(Err(message)) => Err(self.failed(message)),
            Err(e) => Err(self.trapped(e)),
        }
    }

    /// The materials the plugin lists.
    pub fn materials(&mut self) -> Result<Vec<MaterialInfo>, PluginError> {
        self.begin()?;
        match self.bindings.call_list_materials(&mut self.store) {
            Ok(Ok(list)) => Ok(list.into_iter().map(|m| MaterialInfo { name: m.name, image: m.image, color: m.color.map(|(r, g, b)| [r, g, b]) }).collect()),
            Ok(Err(message)) => Err(self.failed(message)),
            Err(e) => Err(self.trapped(e)),
        }
    }
}
