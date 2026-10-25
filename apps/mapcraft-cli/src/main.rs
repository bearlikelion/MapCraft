//! Headless MapCraft tools.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;

use mapcraft_engine::{Session, Severity, validate};
use mapcraft_fgd::GameData;
use mapcraft_geom::{Aabb, DVec3};
use mapcraft_map::{NodeKind, TexDefaults};
use mapcraft_sdk::Sdk;

const USAGE: &str = "usage:
  mapcraft-cli new <map.mcmap> [--sdk <id>] [--material <name>]
  mapcraft-cli info <map.mcmap>
  mapcraft-cli validate <map.mcmap> [--sdk <dir>] [--var name=value]...
  mapcraft-cli fgd <file.fgd>... [--class <name>]";

/// A hollow 512 x 512 x 256 room with a player start, so a new map can be compiled right away.
fn new_map(args: &[String]) -> Result<(), String> {
    let (mut path, mut sdk, mut material) = (None, None, "dev/grid".to_string());
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--sdk" => sdk = Some(it.next().ok_or(USAGE)?.clone()),
            "--material" => material = it.next().ok_or(USAGE)?.clone(),
            _ => path = Some(a),
        }
    }
    let path = Path::new(path.ok_or(USAGE)?);
    if path.exists() {
        return Err(format!("{} already exists", path.display()));
    }
    let mut s = Session::default();
    s.sdk = sdk;
    s.tex = TexDefaults { material, scale: 1.0 };
    let room = Aabb::new(DVec3::new(-256.0, -256.0, -16.0), DVec3::new(256.0, 256.0, 240.0));
    s.create_brush(room).map_err(|e| e.to_string())?;
    s.hollow(16.0).map_err(|e| e.to_string())?;
    s.create_point_entity("info_player_start", DVec3::ZERO, vec![]).map_err(|e| e.to_string())?;
    s.save_as(path).map_err(|e| e.to_string())?;
    println!("wrote {}", path.display());
    Ok(())
}

fn info(path: &str) -> Result<(), String> {
    let file = mapcraft_format::load(Path::new(path)).map_err(|e| format!("{path}: {e}"))?;
    let (mut layers, mut groups, mut point, mut solid, mut brushes, mut faces, mut prefabs) = (0, 0, 0, 0, 0, 0, 0);
    let mut bounds: Option<Aabb> = None;
    for (_, n) in file.map.nodes() {
        match &n.kind {
            NodeKind::Layer(_) => layers += 1,
            NodeKind::Group(_) => groups += 1,
            NodeKind::Entity(_) if n.children.is_empty() => point += 1,
            NodeKind::Entity(_) => solid += 1,
            NodeKind::Prefab(_) => prefabs += 1,
            NodeKind::Brush(b) => {
                brushes += 1;
                faces += b.faces.len();
                if let Ok(bb) = b.bounds() {
                    bounds = Some(bounds.map_or(bb, |a| a.union(&bb)));
                }
            }
            NodeKind::World(_) => {}
        }
    }
    println!("{path}");
    println!("  sdk:       {}", file.sdk.as_deref().unwrap_or("(none)"));
    println!("  layers:    {layers}");
    println!("  groups:    {groups}");
    println!("  entities:  {point} point, {solid} brush");
    println!("  brushes:   {brushes} ({faces} faces)");
    println!("  prefabs:   {prefabs}");
    if let Some(b) = bounds {
        println!("  bounds:    {:?} to {:?}", b.min.to_array(), b.max.to_array());
    }
    Ok(())
}

fn validate_cmd(args: &[String]) -> Result<bool, String> {
    let (mut path, mut sdk_dir, mut vars) = (None, None, BTreeMap::new());
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--sdk" => sdk_dir = Some(it.next().ok_or(USAGE)?),
            "--var" => {
                let (k, v) = it.next().and_then(|kv| kv.split_once('=')).ok_or(USAGE)?;
                vars.insert(k.to_string(), v.to_string());
            }
            _ => path = Some(a),
        }
    }
    let path = path.ok_or(USAGE)?;
    let file = mapcraft_format::load(Path::new(path)).map_err(|e| format!("{path}: {e}"))?;
    let game = match sdk_dir {
        Some(dir) => {
            let sdk = Sdk::load(Path::new(dir)).map_err(|e| e.to_string())?;
            let gd = sdk.game_data(&vars).map_err(|e| e.to_string())?;
            for (f, d) in &gd.diagnostics {
                println!("{}:{d}", f.display());
            }
            Some(gd)
        }
        None => None,
    };
    let issues = validate(&file.map, game.as_ref());
    for i in &issues {
        let level = if i.severity == Severity::Error { "error" } else { "warning" };
        let face = i.face.map(|f| format!(" face {}", f.0)).unwrap_or_default();
        println!("{level}: node {}{face}: {}", i.node.0, i.message);
    }
    println!("{path}: {} issues", issues.len());
    Ok(!issues.iter().any(|i| i.severity == Severity::Error))
}

fn fgd(args: &[String]) -> Result<(), String> {
    let mut class = None;
    let mut paths = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--class" => class = Some(it.next().ok_or(USAGE)?),
            _ => paths.push(a),
        }
    }
    for path in paths {
        let gd = GameData::load(Path::new(path), &mut |p| std::fs::read(p));
        println!("{path}: {} classes, {} diagnostics", gd.class_names().count(), gd.diagnostics.len());
        for (file, d) in &gd.diagnostics {
            println!("  {}:{d}", file.display());
        }
        if let Some(name) = class {
            let c = gd.class(name).ok_or_else(|| format!("{path}: no class `{name}`"))?;
            println!("{} ({:?}): {}", c.name, c.kind, c.description);
            for p in &c.properties {
                println!("  {}({}) \"{}\" default={:?} choices={}", p.name, p.ty, p.display, p.default, p.choices.len());
            }
            for io in &c.inputs {
                println!("  input {}({})", io.name, io.ty);
            }
            for io in &c.outputs {
                println!("  output {}({})", io.name, io.ty);
            }
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.split_first() {
        Some((cmd, rest)) if cmd == "new" => new_map(rest).map(|()| true),
        Some((cmd, [path])) if cmd == "info" => info(path).map(|()| true),
        Some((cmd, rest)) if cmd == "validate" => validate_cmd(rest),
        Some((cmd, rest)) if cmd == "fgd" => fgd(rest).map(|()| true),
        _ => Err(USAGE.into()),
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
