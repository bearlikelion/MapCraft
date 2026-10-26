//! Quake support for MapCraft: imports and exports `.map` files and decodes the textures in WAD files.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod export;
mod import;
mod map;
mod wad;

use mapcraft::plugin::host;
use mapcraft::plugin::types::{ExporterSpec, ImporterSpec};

wit_bindgen::generate!({ path: "../../wit", world: "plugin" });

struct Quake;

struct HostFiles;

impl wad::Files for HostFiles {
    fn read_range(&self, path: &str, offset: u64, length: u64) -> Option<Vec<u8>> {
        host::read_range(path, offset, length).ok()
    }

    fn list(&self, extensions: &[&str]) -> Vec<String> {
        host::list_files(&extensions.iter().map(|e| e.to_string()).collect::<Vec<_>>())
    }
}

impl Guest for Quake {
    fn manifest() -> Manifest {
        Manifest {
            id: "quake".into(),
            name: "Quake".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            commands: Vec::new(),
            materials: true,
            importers: vec![ImporterSpec { id: "map".into(), label: "Quake map".into(), extensions: vec!["map".into()] }],
            exporters: vec![ExporterSpec { id: "map".into(), label: "Quake map (Valve220)".into(), extensions: vec!["map".into()] }],
            validates: false,
        }
    }

    fn run_command(id: String, _params: Vec<Param>, _context: Context) -> Result<Output, String> {
        Err(format!("unknown command {id}"))
    }

    fn import_file(importer: String, name: String, data: Vec<u8>, _context: Context) -> Result<Output, String> {
        if importer != "map" {
            return Err(format!("unknown importer {importer}"));
        }
        let import = import::import(&String::from_utf8_lossy(&data), &HostFiles).map_err(|e| format!("{name}: {e}"))?;
        for line in &import.log {
            host::log(&format!("{name}: {line}"));
        }
        Ok(import.output)
    }

    fn export_file(exporter: String, mut map: MapData, _context: Context) -> Result<Vec<u8>, String> {
        if exporter != "map" {
            return Err(format!("unknown exporter {exporter}"));
        }
        if let Some(wad) = export::default_wad(&map, &HostFiles) {
            host::log(&format!("{}.map: the world has no wad key, so it names the WADs holding the map's textures: {wad}", map.name));
            map.world.push(("wad".into(), wad));
        }
        let export = export::export(&map).map_err(|e| format!("{}.map: {e}", map.name))?;
        for line in &export.warnings {
            host::log(&format!("{}.map: {line}", map.name));
        }
        Ok(export.text.into_bytes())
    }

    fn list_materials() -> Result<Vec<MaterialInfo>, String> {
        let library = wad::Library::open(&HostFiles, "");
        let (pictures, failed) = library.pictures(&HostFiles, None);
        for line in library.problems.iter().chain(failed.iter().take(10)) {
            host::log(&format!("Quake textures: {line}"));
        }
        if library.palette.is_none() && !failed.is_empty() {
            host::log("Quake textures: the palette was not found; set the SDK's Quake folder (the one holding id1)");
        }
        Ok(pictures.into_iter().map(|(name, p)| import::material(name, p)).collect())
    }
    fn validate(_map: MapData) -> Vec<Issue> {
        Vec::new()
    }
}

export!(Quake);
