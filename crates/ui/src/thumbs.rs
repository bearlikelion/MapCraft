use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc;

use egui::{ColorImage, TextureHandle, TextureOptions};

pub const SIZE: u32 = 64;

/// Material previews decoded on a worker thread so the browser never stalls on big images.
pub struct Thumbs {
    textures: HashMap<String, Option<TextureHandle>>,
    pending: HashSet<String>,
    requests: mpsc::Sender<(String, Source)>,
    results: mpsc::Receiver<(String, Option<ColorImage>)>,
    filter: TextureOptions,
}

/// Where a preview comes from: an image file, or a picture a plugin already decoded.
enum Source {
    File(PathBuf),
    Pixels(std::sync::Arc<mapcraft_plugin_host::Picture>),
}

fn decode(source: &Source, size: u32) -> Option<ColorImage> {
    let img = match source {
        Source::File(path) => std::panic::catch_unwind(|| image::open(path)).ok()?.ok()?,
        Source::Pixels(p) => image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(p.width, p.height, p.rgba.clone())?),
    };
    let rgba = img.thumbnail(size, size).to_rgba8();
    Some(ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()))
}

impl Default for Thumbs {
    fn default() -> Self {
        Self::new()
    }
}

impl Thumbs {
    pub fn new() -> Self {
        Self::sized(SIZE)
    }

    /// Previews that fit in `size` by `size` pixels.
    pub fn sized(size: u32) -> Self {
        let (requests, rx) = mpsc::channel::<(String, Source)>();
        let (tx, results) = mpsc::channel();
        // Ends when the sender is dropped with the app.
        std::thread::spawn(move || {
            for (name, source) in rx {
                if tx.send((name, decode(&source, size))).is_err() {
                    break;
                }
            }
        });
        Self { textures: HashMap::new(), pending: HashSet::new(), requests, results, filter: TextureOptions::LINEAR }
    }

    /// The preview for a material, queued for loading the first time it is asked for.
    pub fn get(&mut self, ctx: &egui::Context, m: &crate::project::Material) -> Option<&TextureHandle> {
        if !self.textures.contains_key(&m.name) && !self.pending.contains(&m.name) {
            let source = m.pixels.clone().map(Source::Pixels).or_else(|| m.image.clone().map(Source::File));
            let queued = source.is_some_and(|s| self.requests.send((m.name.clone(), s)).is_ok());
            match (queued, m.color) {
                (true, _) => {
                    self.pending.insert(m.name.clone());
                }
                (false, Some(c)) => {
                    let img = ColorImage::new([SIZE as usize; 2], vec![egui::Color32::from_rgb(c[0], c[1], c[2]); (SIZE * SIZE) as usize]);
                    self.textures.insert(m.name.clone(), Some(ctx.load_texture(format!("thumb:{}", m.name), img, self.filter)));
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
            let tex = img.map(|i| ctx.load_texture(format!("thumb:{name}"), i, self.filter));
            self.textures.insert(name, tex);
        }
        if !self.pending.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }

    pub fn clear(&mut self) {
        self.textures.clear();
    }

    /// Pixel-art SDKs show their small textures blown up with hard texel edges, as the views do.
    pub fn set_pixelated(&mut self, on: bool) {
        self.filter = if on { TextureOptions::NEAREST } else { TextureOptions::LINEAR };
        self.textures.clear();
    }
}
