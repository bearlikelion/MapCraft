use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use mapcraft_sdk::{MANIFEST, Sdk, valid_id};
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

use crate::registry::MAX_DOWNLOAD;
use crate::{Entry, Error, MAPCRAFT_VERSION, io_err};

const MAX_FILES: usize = 20_000;
const MAX_UNPACKED: u64 = 2 << 30;
const MAX_DEPTH: usize = 32;
/// Holds half-unpacked and outgoing SDKs, deep enough that SDK discovery never sees them.
const STAGING: &str = ".staging";

const TEMPLATE: [(&str, &str); 3] = [
    ("sdk.toml", include_str!("../template/sdk.toml")),
    ("fgd/entities.fgd", include_str!("../template/fgd/entities.fgd")),
    ("README.md", include_str!("../template/README.md")),
];

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn zip_err(e: zip::result::ZipError) -> Error {
    Error::Zip(e.to_string())
}

/// A zip entry name as a relative path, or `None` when it is absolute or climbs out with `..`.
fn safe_path(name: &str) -> Option<PathBuf> {
    let name = name.replace('\\', "/");
    if name.starts_with('/') {
        return None;
    }
    let mut out = PathBuf::new();
    for part in name.split('/').filter(|p| !p.is_empty() && *p != ".") {
        if part == ".." || part.contains([':', '\0']) {
            return None;
        }
        out.push(part);
    }
    Some(out)
}

/// Unpacks a zip into `dest`, refusing unsafe paths and links and capping file count and size.
pub fn extract(bytes: &[u8], dest: &Path) -> Result<(), Error> {
    let mut zip = ZipArchive::new(Cursor::new(bytes)).map_err(zip_err)?;
    if zip.len() > MAX_FILES {
        return Err(Error::TooLarge(format!("the archive holds more than {MAX_FILES} files")));
    }
    let mut budget = MAX_UNPACKED;
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).map_err(zip_err)?;
        let name = file.name().map_err(zip_err)?.into_owned();
        let rel = safe_path(&name).ok_or_else(|| Error::UnsafePath(name.clone()))?;
        if file.is_symlink() {
            return Err(Error::UnsafePath(format!("{name} is a link")));
        }
        if rel.as_os_str().is_empty() {
            continue;
        }
        let path = dest.join(&rel);
        if file.is_dir() {
            std::fs::create_dir_all(&path).map_err(io_err(&path))?;
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(io_err(parent))?;
        }
        let mut out = std::fs::File::create(&path).map_err(io_err(&path))?;
        // Header sizes can lie, so the budget counts the bytes actually written.
        let n = std::io::copy(&mut (&mut file).take(budget.saturating_add(1)), &mut out).map_err(io_err(&path))?;
        budget = budget.checked_sub(n).ok_or_else(|| Error::TooLarge(format!("the archive unpacks to more than {} GiB", MAX_UNPACKED >> 30)))?;
    }
    Ok(())
}

/// Downloads `entry` from `registry`, checks its sha256, and installs it into the `sdks` folder.
pub fn install(entry: &Entry, registry: &str, sdks: &Path, progress: &mut dyn FnMut(u64, Option<u64>), cancel: &AtomicBool) -> Result<Sdk, Error> {
    if let Some(why) = entry.unsupported() {
        return Err(Error::Invalid(format!("{}: {why}", entry.name)));
    }
    let bytes = crate::fetch(&entry.source(registry), MAX_DOWNLOAD, progress, cancel)?;
    let actual = sha256_hex(&bytes);
    if !actual.eq_ignore_ascii_case(entry.sha256.trim()) {
        return Err(Error::Checksum { expected: entry.sha256.clone(), actual });
    }
    install_archive(&bytes, &entry.id, sdks)
}

/// Unpacks an SDK zip into `sdks/<id>`. The zip holds the SDK folder's files at its root or in one
/// folder; an older copy is replaced only once the new one has unpacked and loaded.
pub fn install_archive(bytes: &[u8], id: &str, sdks: &Path) -> Result<Sdk, Error> {
    if !valid_id(id) {
        return Err(Error::Invalid(format!("`{id}` is not a valid SDK id")));
    }
    let staging = sdks.join(STAGING);
    let stage = staging.join(id);
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage).map_err(io_err(&stage))?;
    let result = unpack_and_swap(bytes, id, sdks, &stage);
    let _ = std::fs::remove_dir_all(&stage);
    let _ = std::fs::remove_dir(&staging);
    result
}

fn unpack_and_swap(bytes: &[u8], id: &str, sdks: &Path, stage: &Path) -> Result<Sdk, Error> {
    extract(bytes, stage)?;
    let root = if stage.join(MANIFEST).is_file() {
        stage.to_path_buf()
    } else {
        let mut dirs = std::fs::read_dir(stage).map_err(io_err(stage))?.flatten().map(|e| e.path()).filter(|p| p.is_dir());
        match (dirs.next(), dirs.next()) {
            (Some(d), None) if d.join(MANIFEST).is_file() => d,
            _ => return Err(Error::Invalid("the archive has no sdk.toml at its root or in a single folder".into())),
        }
    };
    let sdk = Sdk::load(&root)?;
    if sdk.manifest.id != id {
        return Err(Error::Invalid(format!("the archive holds SDK `{}`, not `{id}`", sdk.manifest.id)));
    }
    let dest = sdks.join(id);
    let old = stage.with_extension("old");
    let _ = std::fs::remove_dir_all(&old);
    if dest.exists() {
        std::fs::rename(&dest, &old).map_err(io_err(&dest))?;
    }
    if let Err(e) = std::fs::rename(&root, &dest) {
        if old.exists() {
            let _ = std::fs::rename(&old, &dest);
        }
        return Err(io_err(&dest)(e));
    }
    let _ = std::fs::remove_dir_all(&old);
    Ok(Sdk::load(&dest)?)
}

/// Removes the SDK `id` from the user's `sdks` folder. Only that folder is ever touched, so SDKs
/// bundled with MapCraft or in a source tree cannot be removed.
pub fn uninstall(sdks: &Path, id: &str) -> Result<(), Error> {
    let dir = sdks.join(id);
    if !valid_id(id) || !dir.join(MANIFEST).is_file() {
        return Err(Error::Invalid(format!("no SDK `{id}` is installed in {}", sdks.display())));
    }
    // Moved aside first so a removal that fails halfway never leaves a broken SDK behind.
    let gone = sdks.join(STAGING).join(format!("{id}.removed"));
    let _ = std::fs::remove_dir_all(&gone);
    std::fs::create_dir_all(sdks.join(STAGING)).map_err(io_err(sdks))?;
    std::fs::rename(&dir, &gone).map_err(io_err(&dir))?;
    let _ = std::fs::remove_dir_all(&gone);
    let _ = std::fs::remove_dir(sdks.join(STAGING));
    Ok(())
}

/// Files under `dir` as sorted `/`-separated relative paths, without hidden files and links.
fn walk(dir: &Path) -> Result<Vec<String>, Error> {
    let mut out = Vec::new();
    let mut stack = vec![(dir.to_path_buf(), 0usize)];
    while let Some((d, depth)) = stack.pop() {
        for entry in std::fs::read_dir(&d).map_err(io_err(&d))?.flatten() {
            let Ok(ty) = entry.file_type() else { continue };
            let path = entry.path();
            if entry.file_name().to_string_lossy().starts_with('.') || ty.is_symlink() {
                continue;
            }
            if ty.is_dir() && depth < MAX_DEPTH {
                stack.push((path, depth + 1));
            } else if ty.is_file() {
                let Ok(rel) = path.strip_prefix(dir) else { continue };
                out.push(rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"));
                if out.len() > MAX_FILES {
                    return Err(Error::TooLarge(format!("{} holds more than {MAX_FILES} files", dir.display())));
                }
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Zips the SDK in `dir` as `<id>/...` into `out/<id>-<version>.zip` and returns the archive's
/// path and its registry entry. `download` is `base_url/<zip name>`, or the zip name alone,
/// relative to the registry. The same files always give the same zip and checksum.
pub fn package(dir: &Path, out: &Path, base_url: Option<&str>) -> Result<(PathBuf, Entry), Error> {
    let sdk = Sdk::load(dir)?;
    let m = &sdk.manifest;
    std::fs::create_dir_all(out).map_err(io_err(out))?;
    let (src, dst) = (dir.canonicalize().map_err(io_err(dir))?, out.canonicalize().map_err(io_err(out))?);
    if dst.starts_with(&src) {
        return Err(Error::Invalid("write the archive outside the SDK folder".into()));
    }
    let version: String = m.version.chars().map(|c| if c.is_ascii_alphanumeric() || ".-_".contains(c) { c } else { '_' }).collect();
    let name = if version.is_empty() { format!("{}.zip", m.id) } else { format!("{}-{version}.zip", m.id) };
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated).last_modified_time(DateTime::default());
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for rel in walk(dir)? {
        let path = dir.join(&rel);
        let bytes = std::fs::read(&path).map_err(io_err(&path))?;
        zip.start_file(format!("{}/{rel}", m.id), options).map_err(zip_err)?;
        zip.write_all(&bytes).map_err(io_err(&path))?;
    }
    let bytes = zip.finish().map_err(zip_err)?.into_inner();
    let path = out.join(&name);
    std::fs::write(&path, &bytes).map_err(io_err(&path))?;
    let entry = Entry {
        id: m.id.clone(),
        name: m.name.clone(),
        version: m.version.clone(),
        description: m.description.clone(),
        author: m.author.clone(),
        license: m.license.clone(),
        homepage: m.homepage.clone(),
        download: base_url.map_or_else(|| name.clone(), |b| format!("{}/{name}", b.trim_end_matches('/'))),
        sha256: sha256_hex(&bytes),
        size: bytes.len() as u64,
        min_mapcraft: MAPCRAFT_VERSION.into(),
        plugins: m.plugins.clone(),
    };
    Ok((path, entry))
}

/// Writes a new SDK from the template into `dir`, which must not exist yet.
pub fn create(dir: &Path, id: &str, name: &str) -> Result<Sdk, Error> {
    let name = name.trim();
    if !valid_id(id) {
        return Err(Error::Invalid(format!("`{id}` is not a valid id: use lowercase letters, digits, '_' or '-'")));
    }
    if name.is_empty() || name.chars().any(char::is_control) {
        return Err(Error::Invalid("the name must be one line of text".into()));
    }
    if dir.exists() {
        return Err(Error::Invalid(format!("{} already exists", dir.display())));
    }
    let toml_name = name.replace('\\', "\\\\").replace('"', "\\\"");
    let write = || -> Result<(), Error> {
        for (rel, text) in TEMPLATE {
            let path = dir.join(rel);
            let text = text.replace("{{id}}", id).replace("{{name}}", if rel.ends_with(".toml") { &toml_name } else { name });
            path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::write(&path, text)).map_err(io_err(&path))?;
        }
        Ok(())
    };
    if let Err(e) = write() {
        let _ = std::fs::remove_dir_all(dir);
        return Err(e);
    }
    Ok(Sdk::load(dir)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Registry;

    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in files {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn rejects_zip_slip() {
        for name in ["../evil.txt", "demo/../../evil.txt", "/etc/evil", "C:/evil", "a\\..\\..\\evil"] {
            let dir = tempfile::tempdir().unwrap();
            let bytes = zip_of(&[("demo/sdk.toml", b"id = \"demo\"\nname = \"Demo\"\n"), (name, b"x")]);
            let err = install_archive(&bytes, "demo", dir.path()).unwrap_err();
            assert!(matches!(err, Error::UnsafePath(_)), "{name}: {err}");
            assert!(!dir.path().join("demo").exists() && !dir.path().parent().unwrap().join("evil.txt").exists());
        }
        assert_eq!(safe_path("./a//b/"), Some(PathBuf::from("a/b")));
    }

    #[test]
    fn checksum_mismatch_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = zip_of(&[("sdk.toml", b"id = \"demo\"\nname = \"Demo\"\n")]);
        std::fs::write(dir.path().join("demo.zip"), &bytes).unwrap();
        let index = dir.path().join("index.json");
        let entry = |sha: &str| {
            format!(
                r#"{{"format":"mapcraft.sdk-registry","version":1,"sdks":[{{"id":"demo","name":"Demo","version":"1","download":"demo.zip","sha256":"{sha}","size":{}}}]}}"#,
                bytes.len()
            )
        };
        std::fs::write(&index, entry(&"ab".repeat(32))).unwrap();
        let reg = Registry::parse(&std::fs::read_to_string(&index).unwrap()).unwrap();
        let sdks = dir.path().join("sdks");
        let err = install(&reg.sdks[0], &index.to_string_lossy(), &sdks, &mut |_, _| {}, &AtomicBool::new(false)).unwrap_err();
        assert!(matches!(err, Error::Checksum { .. }), "{err}");
        assert!(!sdks.join("demo").exists());
    }

    #[test]
    fn package_install_update_and_remove() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src/demo");
        let sdk = create(&src, "demo", "Demo \"Game\"").unwrap();
        assert_eq!(sdk.manifest.name, "Demo \"Game\"");
        assert!(sdk.game_data(&Default::default()).unwrap().class("trigger_area").is_some());
        assert!(create(&src, "demo", "Again").is_err(), "an existing folder is never overwritten");
        assert!(create(&dir.path().join("x"), "Bad Id", "X").is_err());
        std::fs::write(src.join(".hidden"), "skip me").unwrap();

        let dist = dir.path().join("dist");
        let (zip, entry) = package(&src, &dist, None).unwrap();
        assert_eq!(entry.download, "demo-0.1.0.zip");
        assert_eq!(entry.sha256, sha256_hex(&std::fs::read(&zip).unwrap()));
        assert_eq!(package(&src, &dist, None).unwrap().1.sha256, entry.sha256, "packaging is reproducible");
        assert!(package(&src, &src.join("out"), None).is_err());
        let registry = Registry { format: crate::FORMAT.into(), version: 1, name: "Local".into(), sdks: vec![entry] };
        let index = dist.join("index.json");
        std::fs::write(&index, serde_json::to_string_pretty(&registry).unwrap()).unwrap();

        let data = dir.path().join("data");
        let sdks = crate::user_dir(&data);
        let reg = crate::fetch_registry(&index.to_string_lossy()).unwrap();
        let mut seen = 0;
        let installed = install(&reg.sdks[0], &index.to_string_lossy(), &sdks, &mut |n, _| seen = n, &AtomicBool::new(false)).unwrap();
        assert_eq!(installed.dir, sdks.join("demo"));
        assert!(seen > 0);
        assert!(!sdks.join("demo/.hidden").exists());
        assert!(!sdks.join(STAGING).exists());

        std::fs::write(sdks.join("demo/stale.txt"), "old").unwrap();
        install(&reg.sdks[0], &index.to_string_lossy(), &sdks, &mut |_, _| {}, &AtomicBool::new(false)).unwrap();
        assert!(!sdks.join("demo/stale.txt").exists(), "an update replaces the whole folder");
        let found = crate::installed(&crate::search_dirs(Some(&data), &[]));
        assert!(found.iter().any(|i| i.origin == crate::Origin::User && i.dir == sdks.join("demo")));

        assert!(uninstall(&sdks, "../src").is_err());
        uninstall(&sdks, "demo").unwrap();
        assert!(!sdks.join("demo").exists());
        assert!(src.join("sdk.toml").exists(), "only the installed copy is removed");
        assert!(uninstall(&sdks, "demo").is_err());
    }

    #[test]
    fn wrong_id_and_caps() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = zip_of(&[("sdk.toml", b"id = \"other\"\nname = \"Other\"\n")]);
        assert!(matches!(install_archive(&bytes, "demo", dir.path()), Err(Error::Invalid(_))));
        assert!(matches!(install_archive(b"not a zip", "demo", dir.path()), Err(Error::Zip(_))));
        assert!(matches!(install_archive(&zip_of(&[("readme.txt", b"")]), "demo", dir.path()), Err(Error::Invalid(_))));
        assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none(), "failed installs leave nothing behind");
    }
}
