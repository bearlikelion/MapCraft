use std::path::{Component, Path, PathBuf};

const MAX_DEPTH: usize = 32;
const MAX_FILES: usize = 200_000;

/// Read-only view over asset folders, and single files a mapper added, which go by their own name;
/// the first root holding a file wins.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Vfs {
    roots: Vec<PathBuf>,
}

/// `rel` under `root` with each part matched ignoring ASCII case.
fn find_ignoring_case(root: &Path, rel: &Path) -> Option<PathBuf> {
    let mut at = root.to_path_buf();
    for part in rel.components() {
        let Component::Normal(part) = part else { continue };
        let exact = at.join(part);
        at = if exact.exists() {
            exact
        } else {
            std::fs::read_dir(&at).ok()?.flatten().map(|e| e.path()).find(|p| p.file_name().is_some_and(|n| n.eq_ignore_ascii_case(part)))?
        };
    }
    at.is_file().then_some(at)
}

/// A relative path that cannot climb out of its root.
fn safe_relative(rel: &str) -> Option<PathBuf> {
    let p = Path::new(rel);
    p.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir)).then(|| p.to_path_buf())
}

impl Vfs {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self { roots }
    }

    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// The file for `rel`, matching each part's case when it can and ignoring it otherwise, since
    /// games made on Windows name files in any case.
    pub fn find(&self, rel: &str) -> Option<PathBuf> {
        let rel = safe_relative(rel)?;
        self.roots.iter().find_map(|r| {
            if r.is_file() {
                return r.file_name().is_some_and(|n| n.eq_ignore_ascii_case(rel.as_os_str())).then(|| r.clone());
            }
            Some(r.join(&rel)).filter(|p| p.is_file()).or_else(|| find_ignoring_case(r, &rel))
        })
    }

    pub fn read(&self, rel: &str) -> Option<Vec<u8>> {
        std::fs::read(self.find(rel)?).ok()
    }

    /// Up to `len` bytes from `offset`; shorter at the end of the file.
    pub fn read_range(&self, rel: &str, offset: u64, len: u64) -> Option<Vec<u8>> {
        use std::io::{Read, Seek, SeekFrom};
        let mut f = std::fs::File::open(self.find(rel)?).ok()?;
        f.seek(SeekFrom::Start(offset)).ok()?;
        let mut out = Vec::new();
        f.take(len).read_to_end(&mut out).ok()?;
        Some(out)
    }

    /// The file for a material name such as `concrete/wall01`, trying each extension in order.
    pub fn find_material(&self, name: &str, extensions: &[String]) -> Option<PathBuf> {
        extensions.iter().find_map(|ext| self.find(&format!("{name}.{ext}")))
    }

    /// Every file with one of the extensions, as `/`-separated paths without the extension, sorted
    /// and without duplicates shadowed by an earlier root; hidden folders such as `.godot`, and
    /// folders Godot ignores through a `.gdignore` file, are skipped.
    pub fn list(&self, extensions: &[String]) -> Vec<(String, PathBuf)> {
        let mut out: Vec<(String, PathBuf)> = Vec::new();
        for root in &self.roots {
            if root.is_file() {
                let stem = root.file_stem().map(|s| s.to_string_lossy().into_owned());
                if let Some(stem) = stem.filter(|_| has_extension(root, extensions)) {
                    out.push((stem, root.clone()));
                }
                continue;
            }
            let mut stack = vec![(root.clone(), 0usize)];
            while let Some((dir, depth)) = stack.pop() {
                let Ok(entries) = std::fs::read_dir(&dir) else { continue };
                for entry in entries.flatten() {
                    let Ok(ty) = entry.file_type() else { continue };
                    let path = entry.path();
                    // file_type does not follow symlinks, so linked folders cannot loop.
                    if ty.is_dir() && depth < MAX_DEPTH && !entry.file_name().to_string_lossy().starts_with('.') && !path.join(".gdignore").exists() {
                        stack.push((path, depth + 1));
                    } else if ty.is_file() && has_extension(&path, extensions) {
                        let Ok(rel) = path.with_extension("").strip_prefix(root).map(Path::to_path_buf) else { continue };
                        let name = rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
                        out.push((name, path));
                        if out.len() >= MAX_FILES {
                            return finish(out);
                        }
                    }
                }
            }
        }
        finish(out)
    }
}

fn has_extension(path: &Path, extensions: &[String]) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| extensions.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

fn finish(mut out: Vec<(String, PathBuf)>) -> Vec<(String, PathBuf)> {
    // A stable sort keeps the first root's file ahead of shadowed copies.
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out.dedup_by(|b, a| a.0 == b.0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_files_in_any_case() {
        let a = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(a.path().join("materials/RealWorldTextures")).unwrap();
        std::fs::write(a.path().join("materials/RealWorldTextures/Floor.vtf"), b"x").unwrap();
        let vfs = Vfs::new(vec![a.path().to_path_buf()]);
        assert_eq!(vfs.find("materials/realworldtextures/floor.vtf"), Some(a.path().join("materials/RealWorldTextures/Floor.vtf")));
        assert_eq!(vfs.find("materials/realworldtextures/wall.vtf"), None);
        assert_eq!(vfs.find("materials/realworldtextures"), None, "folders are not files");
    }

    #[test]
    fn shadowing_and_escapes() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(a.path().join("ramps")).unwrap();
        std::fs::write(a.path().join("ramps/orange.png"), b"a").unwrap();
        std::fs::write(b.path().join("ramps.png"), b"x").unwrap();
        std::fs::create_dir_all(b.path().join("ramps")).unwrap();
        std::fs::write(b.path().join("ramps/orange.png"), b"b").unwrap();
        std::fs::write(b.path().join("ramps/notes.txt"), b"").unwrap();
        std::fs::create_dir_all(b.path().join(".godot")).unwrap();
        std::fs::write(b.path().join(".godot/cache.png"), b"").unwrap();
        std::fs::create_dir_all(b.path().join("Build/copy")).unwrap();
        std::fs::write(b.path().join("Build/.gdignore"), b"").unwrap();
        std::fs::write(b.path().join("Build/copy/orange.png"), b"").unwrap();
        let vfs = Vfs::new(vec![a.path().into(), b.path().into()]);
        assert_eq!(vfs.read("ramps/orange.png").unwrap(), b"a");
        assert!(vfs.read("../etc/passwd").is_none());
        assert!(vfs.read("/etc/passwd").is_none());
        let exts = vec!["png".to_string()];
        let names: Vec<String> = vfs.list(&exts).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["ramps", "ramps/orange"]);
        assert_eq!(vfs.list(&exts)[1].1, a.path().join("ramps/orange.png"));
        assert_eq!(vfs.find_material("ramps/orange", &exts), Some(a.path().join("ramps/orange.png")));
    }

    #[test]
    fn single_files_go_by_their_own_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("jam")).unwrap();
        let wad = dir.path().join("jam/Mine.WAD");
        std::fs::write(&wad, b"WAD2data").unwrap();
        std::fs::write(dir.path().join("jam/other.wad"), b"").unwrap();
        let vfs = Vfs::new(vec![wad.clone()]);
        assert_eq!(vfs.list(&["wad".to_string()]), [("Mine".to_string(), wad.clone())]);
        assert!(vfs.list(&["png".to_string()]).is_empty());
        assert_eq!(vfs.read_range("Mine.WAD", 4, 4).unwrap(), b"data");
        assert_eq!(vfs.find("Mine.WAD"), Some(wad));
        assert!(vfs.find("other.wad").is_none(), "only the added file, not its neighbors");
        assert!(vfs.find("jam/Mine.WAD").is_none());
    }
}
