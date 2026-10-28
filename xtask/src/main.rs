//! Workspace automation: `cargo xtask <ci|layers|plugins|desktop>`.
//!
//! - `layers`: enforces the dependency layering of `docs/architecture.md`.
//! - `plugins`: builds `plugins/` for wasm32-wasip2 and copies each plugin into the SDKs that list it.
//! - `desktop [exec]`: installs the desktop entry and icon for the current user, launching `exec` (default `target/debug/mapcraft`).
//! - `ci`: fmt check, clippy -D warnings, plugins, tests (`cargo nextest run` when installed) and layers.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use serde_json::Value;

/// (crate name without the `mapcraft-` prefix, layer). See `docs/architecture.md`.
const LAYERS: &[(&str, u8)] = &[
    ("geom", 0),
    ("map", 1),
    ("fgd", 1),
    ("format", 2),
    ("sdk", 2),
    ("plugin-host", 3),
    ("render", 3),
    ("engine", 4),
    ("sdk-manager", 5),
    ("ui", 6),
    ("mapcraft", 7),
    ("cli", 7),
];

/// The first layer that may use UI crates.
const UI_LAYER: u8 = 6;

const UI_ONLY: &[&str] = &["craft-ui", "egui", "egui_extras", "eframe", "egui-wgpu", "winit", "rfd", "muda"];

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
            if l < UI_LAYER && UI_ONLY.contains(&d) {
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

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().map(Path::to_path_buf).unwrap_or_default()
}

const WASM_TARGET: &str = "wasm32-wasip2";

fn plugins() -> Result<(), String> {
    let root = root();
    let manifest = root.join("plugins/Cargo.toml");
    let manifest = manifest.to_string_lossy();
    run(env!("CARGO"), &["build", "--manifest-path", &manifest, "--target", WASM_TARGET, "--release"])?;
    let target = std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("plugins/target"), PathBuf::from);
    let built = target.join(WASM_TARGET).join("release");
    let sdks = std::fs::read_dir(root.join("sdks")).map_err(|e| format!("sdks: {e}"))?;
    for sdk in sdks.flatten().map(|e| e.path()).filter(|p| p.join("sdk.toml").is_file()) {
        let text = std::fs::read_to_string(sdk.join("sdk.toml")).map_err(|e| e.to_string())?;
        let doc: toml::Table = text.parse().map_err(|e| format!("{}: {e}", sdk.display()))?;
        for id in doc.get("plugins").and_then(|v| v.as_array()).into_iter().flatten().filter_map(|v| v.as_str()) {
            let src = built.join(format!("plugin_{}.wasm", id.replace('-', "_")));
            if !src.is_file() {
                println!("{}: no first-party plugin `{id}`, skipped", sdk.display());
                continue;
            }
            let dst = sdk.join("plugins").join(format!("{id}.wasm"));
            std::fs::create_dir_all(sdk.join("plugins")).and_then(|()| std::fs::copy(&src, &dst)).map_err(|e| format!("{}: {e}", dst.display()))?;
            println!("{} -> {}", src.display(), dst.display());
        }
    }
    Ok(())
}

fn has_nextest() -> bool {
    Command::new(env!("CARGO")).args(["nextest", "--version"]).output().is_ok_and(|o| o.status.success())
}

fn ci() -> Result<(), String> {
    let cargo = env!("CARGO");
    let plugins_manifest = root().join("plugins/Cargo.toml");
    let pm = plugins_manifest.to_string_lossy();
    run(cargo, &["fmt", "--all", "--check"])?;
    run(cargo, &["fmt", "--manifest-path", &pm, "--all", "--check"])?;
    run(cargo, &["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"])?;
    run(cargo, &["clippy", "--manifest-path", &pm, "--target", WASM_TARGET, "--", "-D", "warnings"])?;
    run(cargo, &["test", "--manifest-path", &pm])?;
    plugins()?;
    if has_nextest() {
        run(cargo, &["nextest", "run", "--workspace", "--no-tests", "pass"])?;
        run(cargo, &["test", "--workspace", "--doc"])?;
    } else {
        run(cargo, &["test", "--workspace"])?;
    }
    layers()
}

fn desktop(exec: Option<String>) -> Result<(), String> {
    let root = root();
    let assets = root.join("apps/mapcraft/assets");
    let exec = std::path::absolute(exec.map_or_else(|| root.join("target/debug/mapcraft"), PathBuf::from)).map_err(|e| e.to_string())?;
    let exec = exec.to_str().ok_or("exec path is not UTF-8")?;
    if exec.contains(['"', '`', '$', '\\', '%', '\n']) {
        return Err(format!("{exec}: path has characters a desktop entry cannot quote"));
    }
    let share = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .ok_or("HOME is not set")?;
    let hicolor = share.join("icons/hicolor");
    let mut copies: Vec<(PathBuf, PathBuf)> =
        [256, 512].iter().map(|n| (assets.join(format!("mapcraft-{n}.png")), hicolor.join(format!("{n}x{n}/apps/mapcraft.png")))).collect();
    copies.push((assets.join("mapcraft.svg"), hicolor.join("scalable/apps/mapcraft.svg")));
    for (src, dst) in &copies {
        dst.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::copy(src, dst)).map_err(|e| format!("{}: {e}", dst.display()))?;
        println!("{}", dst.display());
    }
    let entry = std::fs::read_to_string(assets.join("mapcraft.desktop")).map_err(|e| format!("mapcraft.desktop: {e}"))?;
    let entry: String = entry.lines().map(|l| if l.starts_with("Exec=") { format!("Exec=\"{exec}\" %f\n") } else { format!("{l}\n") }).collect();
    let apps = share.join("applications");
    let dst = apps.join("mapcraft.desktop");
    std::fs::create_dir_all(&apps).and_then(|()| std::fs::write(&dst, entry)).map_err(|e| format!("{}: {e}", dst.display()))?;
    println!("{}", dst.display());
    if !Path::new(exec).is_file() {
        println!("note: {exec} does not exist yet, build it with `cargo build -p mapcraft`");
    }
    let apps = apps.to_string_lossy();
    let hicolor_str = hicolor.to_string_lossy();
    let mut refresh = vec![("update-desktop-database", vec![&*apps]), ("kbuildsycoca6", vec![])];
    // Only refresh an existing GTK cache; creating one would hide icons other tools add later.
    if hicolor.join("icon-theme.cache").is_file() {
        refresh.push(("gtk-update-icon-cache", vec!["-f", "-t", &*hicolor_str]));
    }
    for (program, args) in refresh {
        if let Err(e) = run(program, &args) {
            println!("skipped: {e}");
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let task = std::env::args().nth(1).unwrap_or_default();
    let result = match task.as_str() {
        "ci" => ci(),
        "layers" => layers(),
        "plugins" => plugins(),
        "desktop" => desktop(std::env::args().nth(2)),
        _ => Err("usage: cargo xtask <ci|layers|plugins|desktop [exec]>".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
