//! Imports Valve Source BSP maps (versions 19 to 21) as brushes, entities and their materials.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod bsp;
mod materials;
mod vtf;

use mapcraft::plugin::host;
use mapcraft::plugin::types::ImporterSpec;

wit_bindgen::generate!({ path: "../../wit", world: "plugin" });

struct SourceBsp;

struct HostFiles;

impl materials::Files for HostFiles {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        host::read_file(path).ok()
    }

    fn read_range(&self, path: &str, offset: u64, length: u64) -> Option<Vec<u8>> {
        host::read_range(path, offset, length).ok()
    }
}

impl Guest for SourceBsp {
    fn manifest() -> Manifest {
        Manifest {
            id: "source-bsp".into(),
            name: "Source BSP".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            commands: Vec::new(),
            materials: false,
            importers: vec![ImporterSpec { id: "bsp".into(), label: "Source BSP".into(), extensions: vec!["bsp".into()] }],
            exporters: Vec::new(),
        }
    }

    fn run_command(id: String, _params: Vec<Param>, _context: Context) -> Result<Output, String> {
        Err(format!("unknown command {id}"))
    }

    fn import_file(importer: String, name: String, data: Vec<u8>, _context: Context) -> Result<Output, String> {
        if importer != "bsp" {
            return Err(format!("unknown importer {importer}"));
        }
        let map = bsp::import(&data, &HostFiles).map_err(|e| format!("{name}: {e}"))?;
        for warning in &map.warnings {
            host::log(&format!("{name}: {warning}"));
        }
        host::log(&format!(
            "{name}: {} world brushes ({} ramps), {} entities ({} brush entities, {} static props); dropped {} brushes with no volume; skipped {} displacements",
            map.world_brushes, map.ramps, map.entities, map.brush_entities, map.static_props, map.dropped_brushes, map.displacements,
        ));
        let (embedded, game, missing) = map.textures;
        host::log(&format!("{name}: textures: {embedded} from the map, {game} from the game's files, {missing} missing"));
        if !map.game_files && missing > 0 {
            host::log(&format!("{name}: set the SDK's Counter-Strike: Source folder to load the game's own textures"));
        }
        Ok(map.output)
    }

    fn list_materials() -> Result<Vec<MaterialInfo>, String> {
        Ok(Vec::new())
    }

    fn export_file(exporter: String, _map: MapData, _context: Context) -> Result<Vec<u8>, String> {
        Err(format!("unknown exporter {exporter}"))
    }
}

export!(SourceBsp);
