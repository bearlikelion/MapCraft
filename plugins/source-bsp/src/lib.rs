//! Imports Valve Source BSP maps (versions 19 to 21) as brushes and entities.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod bsp;

use mapcraft::plugin::host;
use mapcraft::plugin::types::ImporterSpec;

wit_bindgen::generate!({ path: "../../wit", world: "plugin" });

struct SourceBsp;

impl Guest for SourceBsp {
    fn manifest() -> Manifest {
        Manifest {
            id: "source-bsp".into(),
            name: "Source BSP".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            commands: Vec::new(),
            materials: false,
            importers: vec![ImporterSpec { id: "bsp".into(), label: "Source BSP".into(), extensions: vec!["bsp".into()] }],
        }
    }

    fn run_command(id: String, _params: Vec<Param>, _context: Context) -> Result<Output, String> {
        Err(format!("unknown command {id}"))
    }

    fn import_file(importer: String, name: String, data: Vec<u8>, _context: Context) -> Result<Output, String> {
        if importer != "bsp" {
            return Err(format!("unknown importer {importer}"));
        }
        let map = bsp::import(&data).map_err(|e| format!("{name}: {e}"))?;
        for warning in &map.warnings {
            host::log(&format!("{name}: {warning}"));
        }
        host::log(&format!(
            "{name}: {} world brushes, {} entities ({} brush entities, {} static props); dropped {} brushes with no volume; skipped {} displacements",
            map.output.brushes.len(),
            map.output.entities.len(),
            map.brush_entities,
            map.static_props,
            map.dropped_brushes,
            map.displacements,
        ));
        Ok(map.output)
    }

    fn list_materials() -> Result<Vec<MaterialInfo>, String> {
        Ok(Vec::new())
    }
}

export!(SourceBsp);
