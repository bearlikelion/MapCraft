use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use image::{Rgba, RgbaImage};
use mapcraft_fgd::GameData;
use mapcraft_geom::DVec3;
use mapcraft_plugin_host::{Channel, ChannelMap, Host, Pbr, Plugin};
use mapcraft_render::{EntityStyle, Renderer};
use mapcraft_sdk::{Sdk, Vfs};

/// A material in the browser: an image to show and texture with, or else a flat sRGB color.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Material {
    pub name: String,
    /// The Materials panel folder a plugin gives a name that carries none, such as a WAD's texture.
    pub folder: Option<String>,
    pub image: Option<PathBuf>,
    pub color: Option<[u8; 3]>,
    /// A picture a plugin decoded itself, such as a texture from a WAD.
    pub pixels: Option<std::sync::Arc<mapcraft_plugin_host::Picture>>,
    /// The texture's own size when the picture is a smaller preview.
    pub size: Option<[u32; 2]>,
    pub pbr: Option<Pbr>,
    /// How many times the game repeats the texture across its own size, as Godot's `uv1_scale`.
    pub uv_scale: Option<[f32; 2]>,
    pub pixelated: bool,
    pub blend: mapcraft_map::BlendRole,
}

impl Material {
    /// The folder the Materials panel shows it in: the plugin's, else the name up to its last `/`.
    pub fn folder(&self) -> Option<&str> {
        self.folder.as_deref().or_else(|| self.name.rsplit_once('/').map(|(f, _)| f))
    }
}

/// Real texture sizes of cached imported pictures that were shrunk, by material name.
const SIZES: &str = "sizes.json";

fn read_sizes(dir: &Path) -> std::collections::BTreeMap<String, [u32; 2]> {
    std::fs::read(dir.join(SIZES)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Characters some file systems refuse in names, written as `%XX` in the material cache.
const UNSAFE: &[char] = &['%', '*', '?', ':', '"', '<', '>', '|', '\\'];

fn escape_name(part: &str) -> String {
    part.chars().map(|c| if UNSAFE.contains(&c) || c.is_control() { format!("%{:02X}", c as u32) } else { c.to_string() }).collect()
}

fn unescape_name(part: &str) -> String {
    let mut out = String::new();
    let mut rest = part;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let code = rest.get(i + 1..i + 3).and_then(|h| u32::from_str_radix(h, 16).ok()).and_then(char::from_u32);
        match code {
            Some(c) => {
                out.push(c);
                rest = &rest[i + 3..];
            }
            None => {
                out.push('%');
                rest = &rest[i + 1..];
            }
        }
    }
    out + rest
}

/// Size of the texture made for a color-only material.
const SWATCH: u32 = 64;

/// A flat color texture with darker edges, so faces show their texture grid.
fn swatch(c: [u8; 3]) -> Vec<u8> {
    let mut out = Vec::with_capacity((SWATCH * SWATCH * 4) as usize);
    for y in 0..SWATCH {
        for x in 0..SWATCH {
            let edge = x == 0 || y == 0;
            out.extend(c.map(|v| if edge { (u16::from(v) * 3 / 4) as u8 } else { v }));
            out.push(255);
        }
    }
    out
}

/// The SDK a map is edited with, and everything loaded from it.
#[derive(Default)]
pub struct Project {
    pub sdk: Option<Sdk>,
    pub vars: BTreeMap<String, String>,
    pub game: GameData,
    pub vfs: Vfs,
    /// Materials available in the SDK's texture folders, sorted by name.
    pub materials: Vec<Material>,
    pub plugins: Vec<Plugin>,
    pub problems: Vec<String>,
    /// Plugins that list materials, which `list_materials_later` reads.
    material_plugins: Vec<PathBuf>,
    pub models: crate::models::Models,
}

/// Materials plugins listed on a worker thread, and what they logged.
#[derive(Default)]
pub struct Listed {
    pub materials: Vec<Material>,
    pub problems: Vec<String>,
}

impl Project {
    /// Loads the SDK and its plugins, plus every plugin in `user_plugins`.
    pub fn load(sdk: Option<Sdk>, vars: BTreeMap<String, String>, host: Option<&Host>, user_plugins: Option<&Path>) -> Self {
        let mut p = Self { sdk, vars, ..Self::default() };
        let Some(sdk) = &p.sdk else {
            if let Some(host) = host {
                p.load_plugins(host, user_plugins);
            }
            return p;
        };
        match sdk.game_data(&p.vars) {
            Ok(gd) => {
                p.problems.extend(gd.diagnostics.iter().map(|(f, d)| format!("{}:{d}", f.display())));
                p.game = gd;
            }
            Err(e) => p.problems.push(e.to_string()),
        }
        p.vfs = sdk.vfs(&p.vars);
        p.materials =
            p.vfs.list(&sdk.manifest.textures.extensions).into_iter().map(|(name, path)| Material { name, image: Some(path), ..Material::default() }).collect();
        p.models = crate::models::Models::new(sdk.model_vfs(&p.vars), &sdk.manifest, &p.game);
        if let Some(host) = host {
            p.load_plugins(host, user_plugins);
        }
        p
    }

    /// Loads the SDK's plugins from its `plugins` folder, then the user's.
    fn load_plugins(&mut self, host: &Host, user_plugins: Option<&Path>) {
        let mut paths: Vec<PathBuf> =
            self.sdk.iter().flat_map(|sdk| sdk.manifest.plugins.iter().map(|id| sdk.dir.join("plugins").join(format!("{id}.wasm")))).collect();
        if let Some(dir) = user_plugins {
            let mut user: Vec<PathBuf> =
                std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "wasm")).collect();
            user.sort();
            paths.extend(user);
        }
        let readable = self.readable();
        for path in paths {
            match host.load_file(&path, readable.clone()) {
                Ok(plugin) if self.plugins.iter().any(|p| p.manifest.id == plugin.manifest.id) => {
                    self.problems.push(format!("{}: plugin `{}` is already loaded", path.display(), plugin.manifest.id));
                }
                Ok(plugin) => {
                    if plugin.manifest.materials {
                        self.material_plugins.push(path);
                    }
                    self.plugins.push(plugin);
                }
                Err(e) => self.problems.push(format!("{}: {e}", path.display())),
            }
        }
    }

    /// What plugins may read: the SDK's file roots, else its texture folders.
    fn readable(&self) -> Vfs {
        self.sdk.as_ref().map_or_else(|| self.vfs.clone(), |sdk| sdk.plugin_vfs(&self.vars))
    }

    /// Lists plugin materials on a worker thread with fresh plugin instances, since reading a game's
    /// packages can take seconds; `None` when no plugin lists materials.
    pub fn list_materials_later(&self, host: &Host) -> Option<std::sync::mpsc::Receiver<Listed>> {
        if self.material_plugins.is_empty() {
            return None;
        }
        let (host, paths, readable, vfs) = (host.clone(), self.material_plugins.clone(), self.readable(), self.vfs.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut out = Listed::default();
            for path in paths {
                let listed = host.load_file(&path, readable.clone()).and_then(|mut plugin| {
                    let list = plugin.materials();
                    out.problems.extend(plugin.take_log());
                    list
                });
                match listed {
                    Ok(list) => out.materials.extend(
                        list.into_iter()
                            .map(|m| Material {
                                image: m.image.and_then(|i| vfs.find(&i)),
                                name: m.name,
                                folder: m.folder,
                                color: m.color,
                                pixels: m.pixels,
                                size: m.size,
                                pbr: m.pbr,
                                uv_scale: m.uv_scale,
                                pixelated: m.pixelated,
                                blend: m.blend,
                            })
                            .filter(|m| m.image.is_some() || m.color.is_some() || m.pixels.is_some()),
                    ),
                    Err(e) => out.problems.push(format!("{}: {e}", path.display())),
                }
            }
            out.materials.sort_by(|a, b| a.name.cmp(&b.name));
            out.materials.dedup_by(|b, a| a.name == b.name);
            let _ = tx.send(out);
        });
        Some(rx)
    }

    /// Replaces the plain image listing with what plugins listed, when they listed anything.
    pub fn take_listed(&mut self, listed: Listed) -> Vec<String> {
        if !listed.materials.is_empty() {
            self.materials = listed.materials;
        }
        listed.problems
    }

    /// Adds the pictures imports saved in `dir` as materials, for names the SDK does not have.
    pub fn add_cached_materials(&mut self, dir: &Path) {
        let have: std::collections::BTreeSet<String> = self.materials.iter().map(|m| m.name.clone()).collect();
        let cached = Vfs::new(vec![dir.to_path_buf()]).list(&["png".to_string()]);
        let named = cached.into_iter().map(|(file, path)| (file.split('/').map(unescape_name).collect::<Vec<_>>().join("/"), path));
        let sizes = read_sizes(dir);
        self.materials.extend(named.filter(|(n, _)| !have.contains(n)).map(|(name, path)| {
            let size = sizes.get(&name).copied();
            Material { name, image: Some(path), size, ..Material::default() }
        }));
        self.materials.sort_by(|a, b| a.name.cmp(&b.name));
    }

    /// Saves imported pictures into `dir` as PNGs named after their materials, then lists them.
    pub fn cache_materials(&mut self, dir: &Path, materials: &[mapcraft_plugin_host::ImportedMaterial]) -> usize {
        let mut saved = 0;
        for m in materials {
            let parts: Vec<String> = m.name.split('/').filter(|c| !c.is_empty() && *c != "." && *c != "..").map(escape_name).collect();
            let Some((last, folders)) = parts.split_last() else { continue };
            let folder = folders.iter().fold(dir.to_path_buf(), |p, c| p.join(c));
            let path = folder.join(format!("{last}.png"));
            let ok = std::fs::create_dir_all(&folder).is_ok() && image::save_buffer(&path, &m.rgba, m.width, m.height, image::ColorType::Rgba8).is_ok();
            saved += usize::from(ok);
        }
        let mut sizes = read_sizes(dir);
        sizes.extend(materials.iter().filter_map(|m| Some((m.name.clone(), m.size.filter(|s| *s != [m.width, m.height])?))));
        if let Ok(json) = serde_json::to_vec(&sizes) {
            let _ = std::fs::write(dir.join(SIZES), json);
        }
        self.add_cached_materials(dir);
        saved
    }

    /// The loaded plugin with this id.
    pub fn plugin(&mut self, id: &str) -> Option<&mut Plugin> {
        self.plugins.iter_mut().find(|p| p.manifest.id == id)
    }

    pub fn id(&self) -> Option<&str> {
        self.sdk.as_ref().map(|s| s.manifest.id.as_str())
    }

    /// The SDK's textures draw without smoothing, as retro games draw them.
    pub fn pixelated(&self) -> bool {
        self.sdk.as_ref().is_some_and(|s| s.manifest.textures.pixelated)
    }

    /// Either of the face's materials draws without smoothing, so hex tiling falls back to Variation.
    pub fn pixel_face(&self, face: &mapcraft_map::Face) -> bool {
        self.pixelated() || [Some(face.material.as_str()), face.blend_layer()].into_iter().flatten().any(|n| self.material(n).is_some_and(|m| m.pixelated))
    }

    /// Paths the SDK asks for that the user has not set yet.
    pub fn missing_paths(&self) -> Vec<String> {
        let Some(sdk) = &self.sdk else { return Vec::new() };
        sdk.manifest.paths.keys().filter(|k| self.vars.get(*k).is_none_or(|v| v.is_empty())).cloned().collect()
    }

    /// Unset paths that materials or entity definitions come from, so editing needs them; the
    /// rest only matter when compiling.
    pub fn missing_for_editing(&self) -> Vec<String> {
        let Some(sdk) = &self.sdk else { return Vec::new() };
        let m = &sdk.manifest;
        let used: Vec<&String> = m.textures.roots.iter().chain(&m.models.roots).chain(&m.fgd).collect();
        self.missing_paths().into_iter().filter(|k| used.iter().any(|s| s.contains(&format!("${{{k}}}")))).collect()
    }

    pub fn material(&self, name: &str) -> Option<&Material> {
        self.materials.binary_search_by(|m| m.name.as_str().cmp(name)).ok().and_then(|i| self.materials.get(i))
    }

    /// The material named `name`, or else an image of that name, since faces may name the images
    /// materials use, which are not listed.
    pub fn find_material(&self, name: &str) -> Option<Material> {
        if let Some(m) = self.material(name) {
            return Some(m.clone());
        }
        let path = self.vfs.find_material(name, &self.sdk.as_ref()?.manifest.textures.extensions)?;
        Some(Material { name: name.to_string(), image: Some(path), ..Material::default() })
    }

    /// The picture a material is drawn with, decoded.
    pub fn material_picture(&self, name: &str) -> Option<mapcraft_render::Picture> {
        let m = self.find_material(name)?;
        if let Some(p) = &m.pixels {
            return Some(mapcraft_render::Picture { width: p.width, height: p.height, rgba: p.rgba.clone() });
        }
        let image = match (&m.image, m.color) {
            (Some(path), _) => image::open(path).ok()?.to_rgba8(),
            (None, Some(c)) => RgbaImage::from_raw(SWATCH, SWATCH, swatch(c))?,
            (None, None) => return None,
        };
        Some(mapcraft_render::Picture { width: image.width(), height: image.height(), rgba: image.into_raw() })
    }

    /// Decodes and uploads every material in `names` the renderer does not have yet; returns how many failed.
    pub fn upload_materials<'a>(&mut self, renderer: &mut Renderer, names: impl Iterator<Item = &'a str>) -> usize {
        let wanted: Vec<&str> = names.filter(|n| !n.is_empty() && !renderer.has_material(n)).collect();
        self.fetch_full_size(&wanted);
        let skies = self.sdk.as_ref().map(|s| s.manifest.textures.sky.clone()).unwrap_or_default();
        let mut failed = 0;
        for name in wanted {
            let Some(m) = self.find_material(name) else { continue };
            let sky = skies.iter().any(|p| !p.is_empty() && name.len() >= p.len() && name.get(..p.len()).is_some_and(|h| h.eq_ignore_ascii_case(p)));
            if upload(renderer, name, &m, sky) {
                if let Some([w, h]) = m.size.or_else(|| renderer.material_size(name)) {
                    let [sx, sy] = m.uv_scale.unwrap_or([1.0, 1.0]);
                    let tiled = |side: u32, scale: f32| (f64::from(side) / f64::from(scale)).round().max(1.0) as u32;
                    renderer.set_material_size(name, [tiled(w, sx), tiled(h, sy)]);
                }
                if let Some(pbr) = &m.pbr {
                    let (normal, orm) = pack(pbr, &|path| self.picture(path));
                    renderer.set_material_maps(name, (normal.width(), normal.height(), normal.as_raw()), (orm.width(), orm.height(), orm.as_raw()));
                }
            } else {
                failed += 1;
            }
        }
        failed
    }

    /// Swaps the shrunken previews plugins listed for `names` for their full-size pictures, once.
    fn fetch_full_size(&mut self, names: &[&str]) {
        let shrunk: Vec<String> = names
            .iter()
            .filter(|n| self.material(n).is_some_and(|m| matches!((&m.pixels, m.size), (Some(p), Some([w, h])) if [p.width, p.height] != [w, h])))
            .map(|n| n.to_string())
            .collect();
        if shrunk.is_empty() {
            return;
        }
        for plugin in self.plugins.iter_mut().filter(|p| p.manifest.materials) {
            let pictures = plugin.load_materials(&shrunk);
            self.problems.extend(plugin.take_log());
            let Ok(pictures) = pictures else { continue };
            for (name, picture) in shrunk.iter().zip(pictures) {
                if let (Some(p), Some(m)) = (picture, self.materials.iter_mut().find(|m| &m.name == name)) {
                    m.pixels = Some(std::sync::Arc::new(p));
                }
            }
        }
    }

    fn picture(&self, path: &str) -> Option<RgbaImage> {
        let file = self.vfs.find(path)?;
        image::open(&file).map_err(|e| log::warn!("{}: {e}", file.display())).ok().map(|i| i.to_rgba8())
    }
}

/// Normal and roughness maps are uploaded no bigger than this on a side, so memory stays bounded.
const MAP_SIDE: u32 = 1024;

fn fitted((w, h): (u32, u32)) -> (u32, u32) {
    let scale = f64::from(MAP_SIDE) / f64::from(w.max(h).max(1));
    if scale >= 1.0 { (w, h) } else { (((f64::from(w) * scale) as u32).max(1), ((f64::from(h) * scale) as u32).max(1)) }
}

fn resized(p: RgbaImage, (w, h): (u32, u32)) -> RgbaImage {
    if p.dimensions() == (w, h) { p } else { image::imageops::resize(&p, w, h, image::imageops::FilterType::Triangle) }
}

/// A material's Lit maps: a normal map with green up, and occlusion, roughness and metallic in RGB with the values applied.
fn pack(pbr: &Pbr, load: &dyn Fn(&str) -> Option<RgbaImage>) -> (RgbaImage, RgbaImage) {
    let mut normal = pbr.normal.as_deref().and_then(load).map_or_else(
        || RgbaImage::from_pixel(1, 1, Rgba([128, 128, 255, 255])),
        |n| {
            let size = fitted(n.dimensions());
            resized(n, size)
        },
    );
    if pbr.normal_dx {
        normal.pixels_mut().for_each(|p| p[1] = 255 - p[1]);
    }
    let mut pictures: BTreeMap<&str, RgbaImage> = BTreeMap::new();
    for m in [&pbr.ao, &pbr.roughness, &pbr.metallic].into_iter().flatten() {
        if !pictures.contains_key(m.path.as_str())
            && let Some(p) = load(&m.path)
        {
            pictures.insert(&m.path, p);
        }
    }
    let size = pictures.values().map(|p| fitted(p.dimensions())).max_by_key(|(w, h)| u64::from(*w) * u64::from(*h)).unwrap_or((1, 1));
    let pictures: BTreeMap<&str, RgbaImage> = pictures.into_iter().map(|(k, p)| (k, resized(p, size))).collect();
    let pick = |m: &Option<ChannelMap>| m.as_ref().and_then(|m| Some((pictures.get(m.path.as_str())?, m.channel)));
    let (ao, rough, metal) = (pick(&pbr.ao), pick(&pbr.roughness), pick(&pbr.metallic));
    let value = |m: Option<(&RgbaImage, Channel)>, x, y| {
        m.and_then(|(p, c)| {
            let [r, g, b, a] = p.get_pixel_checked(x, y)?.0.map(f32::from);
            Some(
                match c {
                    Channel::Red => r,
                    Channel::Green => g,
                    Channel::Blue => b,
                    Channel::Alpha => a,
                    Channel::Gray => (r + g + b) / 3.0,
                } / 255.0,
            )
        })
        .unwrap_or(1.0)
    };
    let byte = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    let orm = RgbaImage::from_fn(size.0, size.1, |x, y| {
        Rgba([byte(value(ao, x, y)), byte(value(rough, x, y) * pbr.roughness_value), byte(value(metal, x, y) * pbr.metallic_value), 255])
    });
    (normal, orm)
}

/// Uploads one material's picture, file or color swatch; a material with none of them counts as done.
fn upload(renderer: &mut Renderer, name: &str, m: &Material, sky: bool) -> bool {
    let mut put = |w: u32, h: u32, rgba: &[u8]| {
        if sky { renderer.upload_sky(name, w, h, rgba, m.pixelated) } else { renderer.upload_material_with(name, w, h, rgba, m.pixelated) }
    };
    if let Some(p) = &m.pixels {
        return put(p.width, p.height, &p.rgba);
    }
    let Some(path) = &m.image else {
        return m.color.is_none_or(|c| renderer.upload_material(name, SWATCH, SWATCH, &swatch(c)));
    };
    match image::open(path) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            put(rgba.width(), rgba.height(), rgba.as_raw())
        }
        Err(e) => {
            log::warn!("{}: {e}", path.display());
            false
        }
    }
}

impl EntityStyle for Project {
    fn color(&self, classname: &str) -> Option<[u8; 3]> {
        self.game.class(classname)?.color()
    }

    fn size(&self, classname: &str) -> Option<(DVec3, DVec3)> {
        let (a, b) = self.game.class(classname)?.size()?;
        Some((DVec3::from_array(a), DVec3::from_array(b)))
    }

    fn model(&self, e: &mapcraft_map::Entity) -> Option<std::sync::Arc<mapcraft_render::Model>> {
        self.models.get(&self.model_path(e)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shrunk_imports_keep_their_real_size() {
        let dir = tempfile::tempdir().unwrap();
        let picture = |name: &str, size| mapcraft_plugin_host::ImportedMaterial { name: name.into(), width: 2, height: 2, rgba: vec![255; 16], size };
        let mut p = Project::default();
        assert_eq!(p.cache_materials(dir.path(), &[picture("brick/wall", Some([512, 256])), picture("small", Some([2, 2]))]), 2);
        let mut again = Project::default();
        again.add_cached_materials(dir.path());
        let size = |name: &str| again.materials.iter().find(|m| m.name == name).map(|m| m.size);
        assert_eq!((size("brick/wall"), size("small")), (Some(Some([512, 256])), Some(None)));
    }

    #[test]
    fn packs_maps_and_flips_directx_green() {
        let pictures: BTreeMap<&str, RgbaImage> = BTreeMap::from([
            ("n", RgbaImage::from_pixel(4, 4, Rgba([10, 20, 250, 255]))),
            ("r", RgbaImage::from_pixel(4096, 2048, Rgba([200, 100, 0, 255]))),
            ("ao", RgbaImage::from_pixel(2, 2, Rgba([0, 0, 0, 128]))),
        ]);
        let load = |p: &str| pictures.get(p).cloned();
        let map = |path: &str, channel| Some(ChannelMap { path: path.into(), channel });
        let pbr = Pbr {
            normal: Some("n".into()),
            normal_dx: true,
            roughness: map("r", Channel::Green),
            roughness_value: 0.5,
            metallic: map("gone", Channel::Red),
            metallic_value: 0.2,
            ao: map("ao", Channel::Alpha),
        };
        let (normal, orm) = pack(&pbr, &load);
        assert_eq!(normal.get_pixel(0, 0).0, [10, 235, 250, 255], "DirectX green is flipped");
        assert_eq!(orm.dimensions(), (MAP_SIDE, MAP_SIDE / 2), "maps share the largest one's size, capped");
        assert_eq!(orm.get_pixel(7, 7).0, [128, 50, 51, 255], "an unreadable map leaves its value alone");

        let plain = Pbr { normal: None, normal_dx: false, roughness: None, roughness_value: 1.0, metallic: None, metallic_value: 0.0, ao: None };
        let (normal, orm) = pack(&plain, &load);
        assert_eq!((normal.get_pixel(0, 0).0, orm.get_pixel(0, 0).0, orm.dimensions()), ([128, 128, 255, 255], [255, 255, 0, 255], (1, 1)));
    }
}
