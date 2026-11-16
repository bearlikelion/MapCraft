//! Imports EverQuest S3D zones as brushes, objects and lights, and shows the objects' models.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod image;
mod merge;
mod model;
mod pfs;
mod read;
#[cfg(test)]
mod tests;
mod wld;
mod zone;

use std::cell::RefCell;

use mapcraft::plugin::host;
use mapcraft::plugin::types::ImporterSpec;

wit_bindgen::generate!({ path: "../../wit", world: "plugin" });

struct EverQuest;

thread_local! {
    /// The object archive read last, since a zone's objects all come from one.
    static ARCHIVE: RefCell<Option<(String, Vec<u8>)>> = const { RefCell::new(None) };
}

impl Guest for EverQuest {
    fn manifest() -> Manifest {
        Manifest {
            id: "everquest".into(),
            name: "EverQuest".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            commands: Vec::new(),
            materials: false,
            importers: vec![ImporterSpec {
                id: "s3d".into(),
                label: "EverQuest zone (S3D)".into(),
                extensions: vec!["s3d".into()],
                sdk: Some("everquest".into()),
            }],
            exporters: Vec::new(),
            validates: false,
            models: vec!["actordef".into()],
        }
    }

    fn run_command(id: String, _params: Vec<Param>, _context: Context) -> Result<Output, String> {
        Err(format!("unknown command {id}"))
    }

    fn import_file(importer: String, name: String, data: Vec<u8>, _context: Context) -> Result<Output, String> {
        if importer != "s3d" {
            return Err(format!("unknown importer {importer}"));
        }
        let imported = zone::import(&name, &data).map_err(|e| format!("{name}: {e}"))?;
        for line in &imported.log {
            host::log(line);
        }
        Ok(imported.output)
    }

    fn list_materials() -> Result<Vec<MaterialInfo>, String> {
        Ok(Vec::new())
    }

    fn export_file(exporter: String, _map: MapData, _context: Context) -> Result<Vec<u8>, String> {
        Err(format!("unknown exporter {exporter}"))
    }

    fn load_materials(names: Vec<String>) -> Vec<Option<Image>> {
        names.iter().map(|_| None).collect()
    }

    fn load_model(path: String) -> Result<Model, String> {
        let (archive, actor) = model::split(&path).ok_or_else(|| format!("{path} does not name an object in an .s3d"))?;
        ARCHIVE.with(|cache| {
            let mut cache = cache.borrow_mut();
            if cache.as_ref().is_none_or(|(n, _)| !n.eq_ignore_ascii_case(archive)) {
                let data = host::read_file(archive).map_err(|e| format!("{archive}: {e}; set the SDK's EverQuest folder"))?;
                *cache = Some((archive.to_string(), data));
            }
            let (_, data) = cache.as_ref().ok_or("no archive")?;
            model::load(archive, data, actor)
        })
    }

    fn validate(_map: MapData) -> Vec<Issue> {
        Vec::new()
    }

    fn list_models() -> Vec<String> {
        Vec::new()
    }

    fn compile_issues(_map: MapData, _log: Vec<String>) -> Vec<Issue> {
        Vec::new()
    }
}

export!(EverQuest);
