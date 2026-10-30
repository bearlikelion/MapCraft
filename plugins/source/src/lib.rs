//! Valve Source support for MapCraft: imports and exports Hammer `.vmf` maps and lists the game's materials from its VPK packages.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod diagnose;
mod vmf;

use std::collections::BTreeSet;

use mapcraft::plugin::host;
use mapcraft::plugin::types::{BlendRole, ExporterSpec, ImporterSpec};

wit_bindgen::generate!({ path: "../../wit", world: "plugin" });

/// Folders whose materials never go on brushes: models, menu and HUD art, particles and sprites.
const SKIPPED: &[&str] = &["models/", "vgui/", "hud/", "console/", "particle/", "sprites/", "overviews/"];
/// RGBA bytes the browser's previews may take; the more materials a game has, the smaller each preview.
const PREVIEW_BUDGET: usize = 128 << 20;

/// Largest side of a model's pictures in the views.
const MODEL_PICTURE: usize = 512;

struct Source;

struct HostFiles;

static HOST_FILES: HostFiles = HostFiles;

thread_local! {
    /// The game's package directories, read once for all the models the views ask for.
    static LIBRARY: valve::Library<'static> = valve::Library::new(&[], &HOST_FILES);
}

impl valve::Files for HostFiles {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        host::read_file(path).ok()
    }

    fn read_range(&self, path: &str, offset: u64, length: u64) -> Option<Vec<u8>> {
        host::read_range(path, offset, length).ok()
    }
}

/// The game's materials with a preview each, no wider or taller than the budget allows.
fn materials(library: &valve::Library, loose: &[String]) -> Vec<MaterialInfo> {
    let loose = loose.iter().map(|p| p.to_ascii_lowercase()).filter_map(|p| {
        let p = p.strip_prefix("download/").unwrap_or(&p);
        Some(p.strip_prefix("materials/")?.strip_suffix(".vmt")?.to_string())
    });
    let names: BTreeSet<String> =
        library.materials().into_iter().map(str::to_string).chain(loose).filter(|n| !SKIPPED.iter().any(|s| n.starts_with(s))).collect();
    let side = [256, 128, 64, 32].into_iter().find(|s| names.len().saturating_mul(s * s * 4) <= PREVIEW_BUDGET).unwrap_or(16);
    names
        .into_iter()
        .map(|name| {
            let picture = library.picture(&name, side);
            let size = picture.as_ref().map(|p| p.2);
            let pixels = picture.map(|((width, height, rgba), ..)| Image { width, height, rgba });
            let blend = if library.blends(&name) { BlendRole::Own } else { BlendRole::Layer };
            MaterialInfo {
                color: pixels.is_none().then_some((128, 128, 128)),
                name,
                folder: None,
                image: None,
                pixels,
                size,
                pbr: None,
                uv_scale: None,
                pixelated: false,
                blend,
            }
        })
        .collect()
}

impl Guest for Source {
    fn manifest() -> Manifest {
        Manifest {
            id: "source".into(),
            name: "Source".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            commands: Vec::new(),
            materials: true,
            importers: vec![ImporterSpec { id: "vmf".into(), label: "Valve map (VMF)".into(), extensions: vec!["vmf".into()] }],
            exporters: vec![ExporterSpec { id: "vmf".into(), label: "Valve map (VMF)".into(), extensions: vec!["vmf".into()] }],
            validates: false,
            models: vec!["mdl".into()],
        }
    }

    fn run_command(id: String, _params: Vec<Param>, _context: Context) -> Result<Output, String> {
        Err(format!("unknown command {id}"))
    }

    fn import_file(importer: String, name: String, data: Vec<u8>, _context: Context) -> Result<Output, String> {
        if importer != "vmf" {
            return Err(format!("unknown importer {importer}"));
        }
        let import = vmf::import(&String::from_utf8_lossy(&data)).map_err(|e| format!("{name}: {e}"))?;
        for line in &import.log {
            host::log(&format!("{name}: {line}"));
        }
        Ok(import.output)
    }

    fn export_file(exporter: String, map: MapData, _context: Context) -> Result<Vec<u8>, String> {
        if exporter != "vmf" {
            return Err(format!("unknown exporter {exporter}"));
        }
        let export = vmf::export(&map).map_err(|e| format!("{}.vmf: {e}", map.name))?;
        for line in &export.warnings {
            host::log(&format!("{}.vmf: {line}", map.name));
        }
        Ok(export.text.into_bytes())
    }

    fn list_materials() -> Result<Vec<MaterialInfo>, String> {
        let library = valve::Library::new(&[], &HostFiles);
        if !library.has_game_files() {
            host::log("Source materials: the game's packages were not found; set the SDK's Counter-Strike: Source folder");
        }
        let list = materials(&library, &host::list_files(&["vmt".into()]));
        let missing = list.iter().filter(|m| m.pixels.is_none()).count();
        host::log(&format!("Source materials: {} listed, {missing} without a picture", list.len()));
        Ok(list)
    }

    fn load_materials(names: Vec<String>) -> Vec<Option<Image>> {
        names.iter().map(|_| None).collect()
    }

    fn load_model(path: String) -> Result<Model, String> {
        use mapcraft::plugin::types::{ModelPart, Vec3};
        let model = LIBRARY.with(|library| {
            let model = valve::mdl::load(library, &path, MODEL_PICTURE);
            library.forget_pictures();
            model
        })?;
        let textures = model.pictures.into_iter().map(|(width, height, rgba)| Image { width, height, rgba }).collect();
        let parts = model
            .parts
            .into_iter()
            .map(|p| ModelPart {
                texture: p.picture.and_then(|i| u32::try_from(i).ok()),
                positions: p.mesh.positions.into_iter().map(|[x, y, z]| Vec3 { x, y, z }).collect(),
                uvs: p.mesh.uvs.into_iter().map(|[u, v]| (u, v)).collect(),
                indices: p.mesh.indices,
                material: Some(p.material),
                lightmap: None,
                lightmap_uvs: Vec::new(),
            })
            .collect();
        Ok(Model { parts, textures })
    }

    fn validate(_map: MapData) -> Vec<Issue> {
        Vec::new()
    }

    fn list_models() -> Vec<String> {
        let loose: BTreeSet<String> =
            host::list_files(&["mdl".into(), "vvd".into()]).into_iter().map(|p| p.to_ascii_lowercase()).filter(|p| p.starts_with("models/")).collect();
        let (models, animations) = LIBRARY.with(|library| {
            let all: BTreeSet<String> = library.models().into_iter().map(str::to_string).chain(loose.iter().filter(|p| p.ends_with(".mdl")).cloned()).collect();
            // Models made only of animations have no vertex file and nothing to show.
            all.into_iter().partition::<Vec<String>, _>(|m| {
                let vvd = format!("{}.vvd", m.trim_end_matches(".mdl"));
                library.has_packed(&vvd) || loose.contains(&vvd)
            })
        });
        if !animations.is_empty() {
            host::log(&format!("Source models: {} listed, {} made only of animations left out", models.len(), animations.len()));
        }
        models
    }

    fn compile_issues(map: MapData, log: Vec<String>) -> Vec<Issue> {
        diagnose::issues(&map, &log)
    }
}

export!(Source);

#[cfg(test)]
mod tests {
    use std::io::{Read, Seek, SeekFrom};
    use std::path::PathBuf;

    use super::*;

    const CSS: &str = "/mnt/Storage/Gamedev/SurfsUp/CSS-Windows";

    /// Game files under the given folders, like the SDK's `cstrike` and `hl2` roots.
    struct Disk(Vec<PathBuf>);

    impl valve::Files for Disk {
        fn read(&self, path: &str) -> Option<Vec<u8>> {
            self.0.iter().find_map(|root| std::fs::read(root.join(path)).ok())
        }

        fn read_range(&self, path: &str, offset: u64, length: u64) -> Option<Vec<u8>> {
            self.0.iter().find_map(|root| {
                let mut file = std::fs::File::open(root.join(path)).ok()?;
                file.seek(SeekFrom::Start(offset)).ok()?;
                let mut out = Vec::new();
                file.take(length).read_to_end(&mut out).ok()?;
                Some(out)
            })
        }
    }

    #[test]
    fn lists_the_games_materials_within_budget() {
        let roots = vec![PathBuf::from(CSS).join("cstrike"), PathBuf::from(CSS).join("hl2")];
        if !roots[0].join("cstrike_pak_dir.vpk").is_file() {
            eprintln!("skipped: {CSS} is missing");
            return;
        }
        let disk = Disk(roots);
        let start = std::time::Instant::now();
        let library = valve::Library::new(&[], &disk);
        let list = materials(&library, &["materials/My/Custom.vmt".into(), "download/materials/x/y.vmt".into(), "maps/a.vmt".into()]);
        let bytes: usize = list.iter().filter_map(|m| m.pixels.as_ref()).map(|p| p.rgba.len()).sum();
        let missing: Vec<&str> = list.iter().filter(|m| m.pixels.is_none()).map(|m| m.name.as_str()).collect();
        eprintln!(
            "{} materials in {:?}, {} MB of pictures, {} without: {:?}",
            list.len(),
            start.elapsed(),
            bytes >> 20,
            missing.len(),
            &missing[..missing.len().min(20)]
        );
        assert!(list.len() > 3000 && bytes <= PREVIEW_BUDGET);
        assert!(
            list.iter().all(
                |m| !m.name.starts_with("models/") && m.pixels.as_ref().is_none_or(|p| p.width <= 128 && p.rgba.len() == (p.width * p.height * 4) as usize)
            )
        );
        assert!(list.iter().any(|m| m.name == "my/custom") && list.iter().any(|m| m.name == "x/y"));
        let nodraw = list.iter().find(|m| m.name == "tools/toolsnodraw").unwrap();
        assert!(nodraw.pixels.is_some(), "tool textures show their editor picture");
        assert!(missing.len() * 10 < list.len(), "most materials have a picture");
    }
}
