use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc;

use egui::{ColorImage, TextureHandle, TextureOptions};

pub const SIZE: u32 = 64;

/// Material previews decoded on a worker thread so the browser never stalls on big images.
pub struct Thumbs {
    textures: HashMap<String, Option<TextureHandle>>,
    pending: HashSet<String>,
    requests: mpsc::Sender<(String, PathBuf)>,
    results: mpsc::Receiver<(String, Option<ColorImage>)>,
}

fn decode(path: &PathBuf) -> Option<ColorImage> {
    let img = std::panic::catch_unwind(|| image::open(path)).ok()?.ok()?;
    let rgba = img.thumbnail(SIZE, SIZE).to_rgba8();
    Some(ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()))
}

impl Default for Thumbs {
    fn default() -> Self {
        Self::new()
    }
}

impl Thumbs {
    pub fn new() -> Self {
        let (requests, rx) = mpsc::channel::<(String, PathBuf)>();
        let (tx, results) = mpsc::channel();
        // Ends when the sender is dropped with the app.
        std::thread::spawn(move || {
            for (name, path) in rx {
                if tx.send((name, decode(&path))).is_err() {
                    break;
                }
            }
        });
        Self { textures: HashMap::new(), pending: HashSet::new(), requests, results }
    }

    /// The preview for a material, queued for loading the first time it is asked for.
    pub fn get(&mut self, ctx: &egui::Context, m: &crate::project::Material) -> Option<&TextureHandle> {
        if !self.textures.contains_key(&m.name) && !self.pending.contains(&m.name) {
            match (&m.image, m.color) {
                (Some(p), _) if self.requests.send((m.name.clone(), p.clone())).is_ok() => {
                    self.pending.insert(m.name.clone());
                }
                (None, Some(c)) => {
                    let img = ColorImage::new([SIZE as usize; 2], vec![egui::Color32::from_rgb(c[0], c[1], c[2]); (SIZE * SIZE) as usize]);
                    self.textures.insert(m.name.clone(), Some(ctx.load_texture(format!("thumb:{}", m.name), img, TextureOptions::LINEAR)));
                }
                _ => {
                    self.textures.insert(m.name.clone(), None);
                }
            }
        }
        self.textures.get(&m.name).and_then(Option::as_ref)
    }

    /// Uploads finished previews; call once per frame.
    pub fn poll(&mut self, ctx: &egui::Context) {
        for (name, img) in self.results.try_iter() {
            self.pending.remove(&name);
            let tex = img.map(|i| ctx.load_texture(format!("thumb:{name}"), i, TextureOptions::LINEAR));
            self.textures.insert(name, tex);
        }
        if !self.pending.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }

    pub fn clear(&mut self) {
        self.textures.clear();
    }
}
