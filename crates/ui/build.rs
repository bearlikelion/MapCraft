//! Hands the crate `git describe --tags --dirty` as `MAPCRAFT_GIT_VERSION` when built from a git checkout.

use std::path::Path;
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    out.status.success().then(|| text.trim().to_string())
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=MAPCRAFT_VERSION");
    if std::env::var("MAPCRAFT_VERSION").is_ok_and(|v| !v.is_empty()) {
        return;
    }
    let Some(dir) = git(&["rev-parse", "--absolute-git-dir"]) else { return };
    for file in ["HEAD", "logs/HEAD"] {
        let path = Path::new(&dir).join(file);
        if path.exists() {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
    if let Some(v) = git(&["describe", "--tags", "--dirty"]) {
        println!("cargo:rustc-env=MAPCRAFT_GIT_VERSION={v}");
    }
}
