use std::path::{Component, Path, PathBuf};

const MAX_DEPTH: usize = 32;
const MAX_FILES: usize = 200_000;

/// Read-only view over asset folders; the first root holding a file wins.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Vfs {
    roots: Vec<PathBuf>,
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

    pub fn find(&self, rel: &str) -> Option<PathBuf> {
        let rel = safe_relative(rel)?;
        self.roots.iter().map(|r| r.join(&rel)).find(|p| p.is_file())
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
    /// and without duplicates shadowed by an earlier root; hidden folders such as `.godot` are skipped.
    pub fn list(&self, extensions: &[String]) -> Vec<(String, PathBuf)> {
        let mut out: Vec<(String, PathBuf)> = Vec::new();
        for root in &self.roots {
            let mut stack = vec![(root.clone(), 0usize)];
            while let Some((dir, depth)) = stack.pop() {
                let Ok(entries) = std::fs::read_dir(&dir) else { continue };
                for entry in entries.flatten() {
                    let Ok(ty) = entry.file_type() else { continue };
                    let path = entry.path();
                    // file_type does not follow symlinks, so linked folders cannot loop.
                    if ty.is_dir() && depth < MAX_DEPTH && !entry.file_name().to_string_lossy().starts_with('.') {
                        stack.push((path, depth + 1));
                    } else if ty.is_file() && path.extension().and_then(|e| e.to_str()).is_some_and(|e| extensions.iter().any(|x| x.eq_ignore_ascii_case(e))) {
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
}
