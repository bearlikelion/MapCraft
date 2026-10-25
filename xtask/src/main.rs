//! Workspace automation: `cargo xtask <ci|layers>`.
//!
//! - `layers`: enforces the dependency layering of `docs/architecture.md`.
//! - `ci`: fmt check, clippy -D warnings, tests (`cargo nextest run` when installed) and layers.

use std::process::{Command, ExitCode};

use serde_json::Value;

/// (crate name without the `mapcraft-` prefix, layer). See `docs/architecture.md`.
const LAYERS: &[(&str, u8)] =
    &[("geom", 0), ("map", 1), ("fgd", 1), ("format", 2), ("sdk", 2), ("plugin-host", 3), ("render", 3), ("engine", 4), ("ui", 5), ("mapcraft", 6), ("cli", 6)];

const UI_ONLY: &[&str] = &["craft-ui", "egui", "eframe", "egui-wgpu", "winit", "rfd", "muda"];

fn short(name: &str) -> &str {
    name.strip_prefix("mapcraft-").unwrap_or(name)
}

fn layer_of(name: &str) -> Option<u8> {
    LAYERS.iter().find(|(n, _)| *n == short(name)).map(|(_, l)| *l)
}

fn metadata() -> Result<Value, String> {
    let out = Command::new(env!("CARGO")).args(["metadata", "--format-version", "1", "--no-deps"]).output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into());
    }
    serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())
}

fn layers() -> Result<(), String> {
    let md = metadata()?;
    let mut errors = Vec::new();
    for p in md["packages"].as_array().into_iter().flatten().filter(|p| p["name"] != "xtask") {
        let name = p["name"].as_str().unwrap_or_default();
        let Some(l) = layer_of(name) else {
            errors.push(format!("{name}: not assigned a layer (add it to xtask LAYERS and docs/architecture.md)"));
            continue;
        };
        let deps = p["dependencies"].as_array().into_iter().flatten().filter(|d| d["kind"].is_null() || d["kind"] == "build");
        for d in deps.filter_map(|d| d["name"].as_str()) {
            if l < 5 && UI_ONLY.contains(&d) {
                errors.push(format!("{name} (L{l}) depends on UI crate `{d}`"));
            }
            if let (true, Some(dl)) = (d.starts_with("mapcraft-"), layer_of(d))
                && dl >= l
            {
                errors.push(format!("{} (L{l}) depends on {} (L{dl}), dependencies must point down", short(name), short(d)));
            }
        }
    }
    if errors.is_empty() {
        println!("layers: ok");
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

fn run(program: &str, args: &[&str]) -> Result<(), String> {
    println!("$ {program} {}", args.join(" "));
    let status = Command::new(program).args(args).status().map_err(|e| format!("{program}: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("`{program} {}` failed", args.join(" "))) }
}

fn has_nextest() -> bool {
    Command::new(env!("CARGO")).args(["nextest", "--version"]).output().is_ok_and(|o| o.status.success())
}

fn ci() -> Result<(), String> {
    let cargo = env!("CARGO");
    run(cargo, &["fmt", "--all", "--check"])?;
    run(cargo, &["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"])?;
    if has_nextest() {
        run(cargo, &["nextest", "run", "--workspace", "--no-tests", "pass"])?;
        run(cargo, &["test", "--workspace", "--doc"])?;
    } else {
        run(cargo, &["test", "--workspace"])?;
    }
    layers()
}

fn main() -> ExitCode {
    let task = std::env::args().nth(1).unwrap_or_default();
    let result = match task.as_str() {
        "ci" => ci(),
        "layers" => layers(),
        _ => Err("usage: cargo xtask <ci|layers>".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
