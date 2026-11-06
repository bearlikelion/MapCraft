//! The New Material and New Model windows, which copy picked files into the folder beside the saved map
//! so they load like any custom asset and go into the map's `.mcpck`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use egui::RichText;
use mapcraft_format::pack;
use mapcraft_format::pack_material::{Material, TextureSlot};

use crate::app::MapCraftApp;
use crate::assets::read_limited;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AssetKind {
    Material,
    Model,
}

const SLOTS: [(TextureSlot, &str); 6] = [
    (TextureSlot::Albedo, "Albedo"),
    (TextureSlot::Normal, "Normal"),
    (TextureSlot::Roughness, "Roughness"),
    (TextureSlot::Metallic, "Metallic"),
    (TextureSlot::Emission, "Emission"),
    (TextureSlot::Height, "Height"),
];

/// The words at the end of a picture's name that say which map it is.
const SLOT_WORDS: [(TextureSlot, &[&str]); 6] = [
    (TextureSlot::Albedo, &["albedo", "basecolor", "base_color", "diffuse", "color", "col"]),
    (TextureSlot::Normal, &["normal", "nrm", "nor"]),
    (TextureSlot::Roughness, &["roughness", "rough"]),
    (TextureSlot::Metallic, &["metallic", "metalness", "metal"]),
    (TextureSlot::Emission, &["emission", "emissive", "glow"]),
    (TextureSlot::Height, &["height", "heightmap", "height_map", "depth", "displacement", "disp"]),
];

/// Name endings of one picture holding occlusion, roughness and metallic in its red, green and blue.
const PACKED_WORDS: [&str; 2] = ["orm", "arm"];

/// The open window and the files the snapshot tool picks in place of a file dialog.
#[derive(Default)]
pub(crate) struct NewAssets {
    dialog: Option<NewAsset>,
    picks: Vec<PathBuf>,
}

struct NewAsset {
    kind: AssetKind,
    name: String,
    files: Vec<PathBuf>,
    slots: BTreeMap<TextureSlot, usize>,
    material: Material,
    error: String,
}

impl NewAsset {
    fn new(kind: AssetKind, mut files: Vec<PathBuf>) -> Self {
        let name = match kind {
            AssetKind::Material => material_name(&files),
            AssetKind::Model => files.first().map(|f| safe_name(&stem(f))).unwrap_or_default(),
        };
        let mut new = Self { kind, name, files: Vec::new(), slots: BTreeMap::new(), material: Material::default(), error: String::new() };
        files.sort_by_key(|f| slot_of(f).is_none() && !is_packed(f));
        for file in files {
            new.add_file(file);
        }
        new
    }

    /// Adds a picture, giving it the slot its name says when that slot is free, else Albedo when that is.
    fn add_file(&mut self, file: PathBuf) {
        if is_packed(&file) {
            for slot in [TextureSlot::Roughness, TextureSlot::Metallic] {
                self.slots.entry(slot).or_insert(self.files.len());
            }
            self.files.push(file);
            return;
        }
        let slot =
            slot_of(&file).filter(|s| !self.slots.contains_key(s)).or_else(|| (!self.slots.contains_key(&TextureSlot::Albedo)).then_some(TextureSlot::Albedo));
        if let Some(slot) = slot {
            self.slots.insert(slot, self.files.len());
        }
        self.files.push(file);
    }
}

fn stem(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// A name the pack format accepts: ASCII letters, digits, `_` and `-`.
fn safe_name(name: &str) -> String {
    let name: String = name.trim().chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).take(64).collect();
    if name.trim_matches('_').is_empty() { String::new() } else { name }
}

fn ends_with_word(path: &Path, words: &[&str]) -> bool {
    let stem = stem(path).to_ascii_lowercase();
    words.iter().any(|w| stem == *w || stem.strip_suffix(w).is_some_and(|rest| rest.ends_with(['_', '-', ' ', '.'])))
}

fn slot_of(path: &Path) -> Option<TextureSlot> {
    SLOT_WORDS.iter().find_map(|(slot, words)| ends_with_word(path, words).then_some(*slot))
}

fn is_packed(path: &Path) -> bool {
    ends_with_word(path, &PACKED_WORDS)
}

/// One channel of a picture as a gray PNG.
fn channel_png(bytes: &[u8], channel: usize) -> Result<Vec<u8>, String> {
    let rgba = image::load_from_memory(bytes).map_err(|e| e.to_string())?.to_rgba8();
    let gray = image::GrayImage::from_fn(rgba.width(), rgba.height(), |x, y| image::Luma([rgba.get_pixel(x, y).0[channel]]));
    let mut png = Vec::new();
    gray.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(png)
}

/// The pictures' shared name prefix, or one picture's name without its slot word.
fn material_name(files: &[PathBuf]) -> String {
    let stems: Vec<String> = files.iter().map(|f| stem(f)).collect();
    let Some(first) = stems.first() else { return String::new() };
    let mut prefix: &str = first;
    for s in &stems {
        let common = prefix.char_indices().zip(s.chars()).find(|((_, a), b)| a != b).map_or(prefix.len().min(s.len()), |((i, _), _)| i);
        prefix = prefix.get(..common).unwrap_or_default();
    }
    if stems.len() == 1 {
        let lower = first.to_ascii_lowercase();
        let word = SLOT_WORDS.iter().flat_map(|(_, w)| w.iter()).chain(&PACKED_WORDS).filter(|w| lower.ends_with(*w)).map(|w| w.len()).max().unwrap_or(0);
        prefix = first.get(..first.len().saturating_sub(word)).unwrap_or(first);
    }
    let name = safe_name(prefix.trim_end_matches(['_', '-', ' ', '.']));
    if name.is_empty() { safe_name(first) } else { name }
}

/// A model path under `models/` from what the mapper typed, each folder made safe.
fn model_path(name: &str) -> String {
    let name = name.trim().trim_end_matches(".glb");
    name.split(['/', '\\']).map(safe_name).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("/")
}

/// Writes every new file or none of them; an existing file is replaced atomically only when `replace` is set.
fn write_all(files: &[(PathBuf, Vec<u8>)], replace: bool) -> Result<(), String> {
    use std::io::Write;
    let mut written: Vec<&Path> = Vec::new();
    let result = files.iter().try_for_each(|(path, bytes)| {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        if replace && path.exists() {
            return craft_atomic::atomic_write(path, bytes).map_err(|e| format!("{}: {e}", path.display()));
        }
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        written.push(path);
        file.write_all(bytes).map_err(|e| format!("{}: {e}", path.display()))
    });
    if result.is_err() {
        for path in written {
            let _ = std::fs::remove_file(path);
        }
    }
    result
}

/// Writes `materials/<name>.json` under `root` with each picture copied to `textures/<name>_<slot>`, updating one of that name when `replace` is set.
fn create_material(root: &Path, name: &str, mut material: Material, pictures: &[(TextureSlot, &Path)], replace: bool) -> Result<(), String> {
    if name.is_empty() || name.split('/').any(|s| s.is_empty() || safe_name(s) != s) {
        return Err("Name the material with letters, digits, _ and - only".into());
    }
    let taken = ["json", "png", "jpg", "jpeg"].iter().any(|e| root.join(format!("materials/{name}.{e}")).exists());
    if taken && !replace {
        return Err(format!("A material named {name} already exists in {}", root.display()));
    }
    let mut files = Vec::new();
    material.textures.clear();
    for (slot, file) in pictures {
        let bytes = read_limited(file, pack::MAX_ENTRY_SIZE)?;
        let label = file.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        pack::validate_picture(&bytes).map_err(|e| format!("{label}: {e}"))?;
        let bytes = match slot {
            TextureSlot::Roughness if is_packed(file) => channel_png(&bytes, 1).map_err(|e| format!("{label}: {e}"))?,
            TextureSlot::Metallic if is_packed(file) => channel_png(&bytes, 2).map_err(|e| format!("{label}: {e}"))?,
            _ => bytes,
        };
        let ext = pack::material_entry(name, &bytes).and_then(|e| e.rsplit('.').next().map(str::to_string)).unwrap_or_else(|| "png".into());
        let slot_name = SLOTS.iter().find(|(s, _)| s == slot).map_or("texture", |(_, l)| l).to_ascii_lowercase();
        let texture = format!("textures/{name}_{slot_name}.{ext}");
        let path = root.join(&texture);
        if path.exists() && !replace {
            return Err(format!("{} already exists", path.display()));
        }
        material.textures.insert(*slot, texture);
        files.push((path, bytes));
    }
    let json = material.to_json()?;
    files.push((root.join(format!("materials/{name}.json")), json));
    write_all(&files, replace)
}

/// Copies a `.glb` to `models/<path>.glb` under `root`; returns the model path entities use.
fn create_model(root: &Path, path: &str, file: &Path) -> Result<String, String> {
    if path.is_empty() {
        return Err("Name the model".into());
    }
    let ext = file.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if ext == "gltf" {
        return Err("Asset packs carry only .glb models; export it as glTF Binary (.glb) first".into());
    }
    let bytes = read_limited(file, pack::MAX_ENTRY_SIZE)?;
    if !bytes.starts_with(b"glTF") {
        return Err(format!("{} is not a binary glTF model", file.display()));
    }
    let model = format!("models/{path}.glb");
    let dest = root.join(&model);
    if dest.exists() {
        return Err(format!("{} already exists", dest.display()));
    }
    write_all(&[(dest, bytes)], false)?;
    Ok(model)
}

impl MapCraftApp {
    /// Asks for pictures or a model, then opens the window that names it.
    pub(crate) fn add_assets(&mut self, kind: AssetKind) {
        let files = if self.new_assets.picks.is_empty() {
            let dialog = rfd::FileDialog::new();
            match kind {
                AssetKind::Material => dialog.set_title("Pictures for a new material").add_filter("Pictures", &["png", "jpg", "jpeg"]).pick_files(),
                AssetKind::Model => dialog.set_title("Model to add").add_filter("glTF", &["glb", "gltf"]).pick_file().map(|f| vec![f]),
            }
        } else {
            Some(std::mem::take(&mut self.new_assets.picks))
        };
        if let Some(files) = files.filter(|f| !f.is_empty()) {
            self.new_assets.dialog = Some(NewAsset::new(kind, files));
        }
    }

    /// The next file dialog picks `path` instead of asking.
    pub fn pick_for_test(&mut self, path: PathBuf) {
        self.new_assets.picks.push(path);
    }

    /// Opens New Material on a plain picture, named after it so the PBR material takes its place.
    pub(crate) fn make_pbr(&mut self, name: &str) {
        let files = self.project.material_files(name);
        let mut new = NewAsset::new(AssetKind::Material, files);
        new.name = name.into();
        self.new_assets.dialog = Some(new);
        self.status = format!("{name} is a plain picture; Create in New Material makes it a PBR material of the same name");
    }

    /// A dropped picture starts a new material, or joins the one being made.
    pub(crate) fn drop_picture(&mut self, path: PathBuf) {
        match &mut self.new_assets.dialog {
            Some(d) if d.kind == AssetKind::Material => d.add_file(path),
            _ => self.new_assets.dialog = Some(NewAsset::new(AssetKind::Material, vec![path])),
        }
    }

    pub(crate) fn new_asset_dialog(&mut self, ctx: &egui::Context) {
        let Some(new) = &mut self.new_assets.dialog else { return };
        let existing = self.project.material(&new.name).filter(|_| new.kind == AssetKind::Material).map(|m| m.descriptor.is_some());
        let (mut create, mut cancel, mut open) = (false, false, true);
        let title = if new.kind == AssetKind::Material { "New Material" } else { "New Model" };
        let window = egui::Window::new(title).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]);
        window.open(&mut open).show(ctx, |ui| {
            egui::Grid::new("new-asset").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                ui.label("Name");
                ui.add(egui::TextEdit::singleline(&mut new.name).desired_width(260.0));
                ui.end_row();
                if let Some(pbr) = existing {
                    ui.label("");
                    let note = if pbr { "Replaces the PBR material of this name" } else { "Takes the place of the plain picture of this name" };
                    ui.label(RichText::new(note).color(crate::chrome::TEXT_WEAK));
                    ui.end_row();
                }
                if new.kind == AssetKind::Model {
                    ui.label("File");
                    ui.label(new.files.first().and_then(|f| f.file_name()).map_or_else(String::new, |n| n.to_string_lossy().into_owned()));
                    ui.end_row();
                    ui.label("Model path");
                    ui.label(RichText::new(format!("models/{}.glb", model_path(&new.name))).monospace());
                    ui.end_row();
                    return;
                }
                let names: Vec<String> = new.files.iter().map(|f| f.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned())).collect();
                for (slot, label) in SLOTS {
                    ui.label(label);
                    let current = new.slots.get(&slot).and_then(|&i| names.get(i)).map_or("None", String::as_str);
                    let mut chosen = new.slots.get(&slot).copied();
                    crate::panels::search_combo(ui, ("new-asset-slot", label), 260.0, current, |ui, query| {
                        let mut picked = ui.selectable_value(&mut chosen, None, "None").clicked();
                        for (i, name) in names.iter().enumerate().filter(|(_, n)| n.to_lowercase().contains(query)) {
                            picked |= ui.selectable_value(&mut chosen, Some(i), name).clicked();
                        }
                        picked
                    });
                    match chosen {
                        Some(i) => new.slots.insert(slot, i),
                        None => new.slots.remove(&slot),
                    };
                    ui.end_row();
                }
                ui.label("Albedo tint");
                ui.color_edit_button_rgba_unmultiplied(&mut new.material.albedo);
                ui.end_row();
                ui.label("Roughness");
                ui.add(egui::Slider::new(&mut new.material.roughness, 0.0..=1.0));
                ui.end_row();
                ui.label("Metallic");
                ui.add(egui::Slider::new(&mut new.material.metallic, 0.0..=1.0));
                ui.end_row();
                ui.label("Emission RGB");
                ui.horizontal(|ui| {
                    for value in &mut new.material.emission {
                        ui.add(egui::DragValue::new(value).range(0.0..=64.0).speed(0.01));
                    }
                });
                ui.end_row();
                crate::shader_editor::strength_rows(ui, &mut new.material);
            });
            if !new.error.is_empty() {
                ui.colored_label(crate::chrome::ERROR, &new.error);
            }
            ui.label(
                RichText::new("Copied into this SDK's custom folder for every map to use, and packed into the .mcpck of each map that uses it.")
                    .color(crate::chrome::TEXT_WEAK),
            );
            ui.horizontal(|ui| {
                create = ui.add(crate::icons::button(Some("plus"), "Create")).clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if cancel || !open {
            self.new_assets.dialog = None;
        } else if create {
            self.create_new_asset();
        }
    }

    fn asset_root(&self) -> Result<PathBuf, String> {
        let id = self.project.id().ok_or("Open a map made with an SDK first: new materials and models belong to its SDK")?;
        self.custom_folder(id).ok_or_else(|| "There is no data folder to keep new materials and models in".into())
    }

    /// Loads the SDK again so assets just written show, keeping the brush material.
    pub(crate) fn reload_asset_folder(&mut self) {
        let tex = self.session.tex.clone();
        self.load_project(self.session.sdk.clone());
        self.session.tex = tex;
    }

    /// Copies a dropped `.glb` into the custom folder as Add Model does and returns its model path; a copy already there is reused.
    pub(crate) fn add_dropped_model(&mut self, file: &Path) -> Result<String, String> {
        let root = self.asset_root()?;
        let path = model_path(&stem(file));
        let dest = root.join(format!("models/{path}.glb"));
        let model = match read_limited(&dest, pack::MAX_ENTRY_SIZE) {
            Ok(have) if read_limited(file, pack::MAX_ENTRY_SIZE).is_ok_and(|b| b == have) => format!("models/{path}.glb"),
            _ => create_model(&root, &path, file)?,
        };
        self.reload_asset_folder();
        Ok(model)
    }

    fn create_new_asset(&mut self) {
        if self.new_assets.dialog.is_none() {
            return;
        }
        let root = self.asset_root();
        let Some(new) = &mut self.new_assets.dialog else { return };
        let root = match root {
            Ok(root) => root,
            Err(e) => {
                new.error = e;
                return;
            }
        };
        let result = match new.kind {
            AssetKind::Material => {
                let pictures: Vec<(TextureSlot, &Path)> = new.slots.iter().filter_map(|(s, &i)| Some((*s, new.files.get(i)?.as_path()))).collect();
                let replace = self.project.find_material(&new.name).is_some();
                create_material(&root, &new.name, new.material.clone(), &pictures, replace).map(|()| new.name.clone())
            }
            AssetKind::Model => new.files.first().ok_or_else(|| "No model picked".to_string()).and_then(|f| create_model(&root, &model_path(&new.name), f)),
        };
        let added = match result {
            Ok(added) => added,
            Err(e) => {
                new.error = e;
                return;
            }
        };
        let kind = new.kind;
        self.new_assets.dialog = None;
        self.reload_asset_folder();
        if kind == AssetKind::Material {
            self.session.tex.material = added.clone();
            for open in [&mut self.material_inspector, &mut self.browser_material] {
                if open.as_ref().is_some_and(|i| i.name == added) {
                    *open = None;
                }
            }
            if let Some(gpu) = &mut self.gpu {
                gpu.renderer.forget_material(&added);
            }
            self.thumbs.forget(&added);
            self.scene_dirty = true;
        }
        self.status = format!("Added {added} to {}", root.display());
        self.output.push(self.status.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png() -> Vec<u8> {
        let mut bytes = Vec::new();
        image::RgbaImage::new(4, 4).write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
        bytes
    }

    #[test]
    fn picked_pictures_are_named_and_sorted_into_slots() {
        let files: Vec<PathBuf> = ["ship_scifimetal_albedo.png", "ship_scifimetal_normal.png", "ship_scifimetal_Rough.jpg", "ship_scifimetal_glow.png"]
            .iter()
            .map(PathBuf::from)
            .collect();
        let new = NewAsset::new(AssetKind::Material, files);
        assert_eq!(new.name, "ship_scifimetal");
        assert_eq!(new.slots.get(&TextureSlot::Albedo), Some(&0));
        assert_eq!(new.slots.get(&TextureSlot::Normal), Some(&1));
        assert_eq!(new.slots.get(&TextureSlot::Roughness), Some(&2));
        assert_eq!(new.slots.get(&TextureSlot::Emission), Some(&3));
        assert_eq!(new.slots.get(&TextureSlot::Metallic), None, "scifimetal is not a slot word at the end");
        let picked = ["SCIFIRED_orm.png", "SCIFIRED_heightmap.png", "SCIFIRED_albedo.png"].map(PathBuf::from).to_vec();
        let new = NewAsset::new(AssetKind::Material, picked);
        let slot = |s: TextureSlot| new.slots.get(&s).and_then(|i| new.files.get(*i)).map(|f| stem(f));
        assert_eq!((slot(TextureSlot::Albedo), slot(TextureSlot::Height)), (Some("SCIFIRED_albedo".into()), Some("SCIFIRED_heightmap".into())));
        assert_eq!((slot(TextureSlot::Roughness), slot(TextureSlot::Metallic)), (Some("SCIFIRED_orm".into()), Some("SCIFIRED_orm".into())));
        assert_eq!(material_name(&[PathBuf::from("Rusty Metal_basecolor.png")]), "Rusty_Metal");
        assert_eq!(material_name(&[PathBuf::from("crate.png")]), "crate");
        assert_eq!(model_path(" props/../Bee Model.glb"), "props/Bee_Model");
    }

    #[test]
    fn a_new_material_reaches_the_pack_and_is_never_overwritten() {
        let src = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let albedo = src.path().join("stone_albedo.png");
        std::fs::write(&albedo, png()).unwrap();
        let fake = src.path().join("fake.png");
        std::fs::write(&fake, b"not a picture").unwrap();
        let material = Material { roughness: 0.4, ..Material::default() };
        create_material(root.path(), "stone", material.clone(), &[(TextureSlot::Albedo, &albedo)], false).unwrap();
        assert!(create_material(root.path(), "stone", material.clone(), &[], false).is_err(), "an existing name is refused");
        assert!(create_material(root.path(), "../up", material.clone(), &[], false).is_err());
        assert!(create_material(root.path(), "bad", material.clone(), &[(TextureSlot::Albedo, &fake)], false).is_err());
        assert!(!root.path().join("materials/bad.json").exists());

        let vfs = mapcraft_sdk::Vfs::new(vec![root.path().into()]);
        let json = std::fs::read(vfs.find("materials/stone.json").unwrap()).unwrap();
        let read = Material::read(&json).unwrap();
        assert_eq!(read.roughness, 0.4);
        let texture = read.textures.get(&TextureSlot::Albedo).unwrap().clone();
        assert_eq!(texture, "textures/stone_albedo.png");
        let picture = std::fs::read(vfs.find(&texture).unwrap()).unwrap();
        assert!(pack::write(&[("materials/stone.json".into(), json), (texture, picture)]).is_ok());
    }

    #[test]
    fn a_material_is_updated_in_place_or_made_over_a_plain_picture() {
        let src = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let (albedo, normal) = (src.path().join("tiles_albedo.png"), src.path().join("tiles_normal.png"));
        std::fs::write(&albedo, png()).unwrap();
        std::fs::write(&normal, png()).unwrap();
        create_material(root.path(), "tiles", Material::default(), &[(TextureSlot::Albedo, &albedo)], false).unwrap();
        let updated = Material { roughness: 0.25, ..Material::default() };
        create_material(root.path(), "tiles", updated, &[(TextureSlot::Albedo, &albedo), (TextureSlot::Normal, &normal)], true).unwrap();
        let read = Material::read(&std::fs::read(root.path().join("materials/tiles.json")).unwrap()).unwrap();
        assert_eq!(read.roughness, 0.25);
        assert_eq!(read.textures.get(&TextureSlot::Normal).map(String::as_str), Some("textures/tiles_normal.png"));

        let name = "Materials/Clean/Misc/cln_misc_tatami";
        create_material(root.path(), name, Material::default(), &[(TextureSlot::Albedo, &albedo)], true).unwrap();
        assert!(root.path().join(format!("materials/{name}.json")).exists());
        assert!(root.path().join(format!("textures/{name}_albedo.png")).exists());
        assert!(create_material(root.path(), "a/../b", Material::default(), &[], true).is_err());
    }

    #[test]
    fn an_orm_picture_splits_into_roughness_and_metallic() {
        let src = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let orm = src.path().join("panel_orm.png");
        image::RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 255])).save(&orm).unwrap();
        create_material(root.path(), "panel", Material::default(), &[(TextureSlot::Roughness, &orm), (TextureSlot::Metallic, &orm)], false).unwrap();
        let gray = |name: &str| image::open(root.path().join(format!("textures/panel_{name}.png"))).unwrap().to_luma8().get_pixel(0, 0).0[0];
        assert_eq!((gray("roughness"), gray("metallic")), (20, 30));
    }

    #[test]
    fn a_new_model_is_a_glb_under_models() {
        let src = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let glb = src.path().join("bee.glb");
        std::fs::write(&glb, b"glTF\x02\0\0\0").unwrap();
        assert_eq!(create_model(root.path(), "props/bee", &glb).unwrap(), "models/props/bee.glb");
        assert!(create_model(root.path(), "props/bee", &glb).is_err());
        assert!(create_model(root.path(), "x", &src.path().join("bee.gltf")).is_err());
        assert_eq!(pack::model_entry("models/props/bee.glb").as_deref(), Some("models/models/props/bee.glb"));
    }
}
