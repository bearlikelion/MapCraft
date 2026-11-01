//! File > Publish to Steam Workshop: uploads the open map, prefabs baked in, as `<map name>.mcmap`.

use std::path::{Path, PathBuf};

use egui::RichText;
use mapcraft_format::pack;
use mapcraft_steam::{Job, Published, State, Steam, Upload, Visibility};
use serde_json::{Value, json};

use crate::app::MapCraftApp;
use crate::chrome::{ERROR, TEXT_WEAK, WARN};

/// Where a map keeps its Workshop item and the details it was last published with.
const KEY: &str = "workshop";

/// Steam, when it started this editor, and the Publish window.
pub(crate) struct Workshop {
    steam: Result<Steam, String>,
    dialog: Option<Dialog>,
}

impl Workshop {
    pub(crate) fn new() -> Self {
        let steam = Steam::init();
        if let Err(e) = &steam {
            log::info!("steam: {e}");
        }
        Self { steam, dialog: None }
    }

    pub(crate) fn app_dir(&self, app: u32) -> Option<PathBuf> {
        self.steam.as_ref().ok()?.app_dir(app)
    }
}

struct Dialog {
    app: u32,
    name: String,
    upload: Upload,
    tags: Vec<(String, bool)>,
    capture: Capture,
    job: Option<Job>,
    staging: Option<tempfile::TempDir>,
    result: Option<Result<Published, String>>,
    progress: Option<(&'static str, u64, u64)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Capture {
    No,
    Requested,
    Sent,
}

/// Marks the screenshot the Publish window asked for.
struct PreviewShot;

enum Act {
    Publish,
    Capture,
    Choose,
    NewItem,
    Open(u64),
}

impl MapCraftApp {
    pub(crate) fn workshop_menu(&mut self, ui: &mut egui::Ui) {
        let _ = ui;
    }

    pub(crate) fn open_workshop(&mut self) {
        let Some(w) = self.project.sdk.as_ref().and_then(|s| s.manifest.workshop.clone()) else {
            self.status = "This SDK has no Steam Workshop".into();
            return;
        };
        if let Err(e) = &self.workshop.steam {
            self.status = format!("Start {} from Steam to publish ({e})", crate::BRAND);
            return;
        }
        let Some(name) = self.session.path.as_ref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().into_owned()) else {
            self.status = "Save the map before publishing it".into();
            return;
        };
        let saved = self.session.view.get(KEY);
        let text = |k: &str| saved.and_then(|v| v.get(k)).and_then(Value::as_str).map(str::to_string);
        let chosen: Vec<&str> = saved.and_then(|v| v.get("tags")).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
        let item = saved.and_then(|v| v.get("item")).and_then(Value::as_u64);
        let upload = Upload {
            item,
            title: text("title").unwrap_or_else(|| name.clone()),
            description: text("description").unwrap_or_else(|| self.session.map.world().get("message").unwrap_or_default().to_string()),
            visibility: text("visibility").and_then(|l| Visibility::ALL.into_iter().find(|v| v.label() == l)).unwrap_or_default(),
            ..Default::default()
        };
        let tags = w
            .tags
            .into_iter()
            .map(|t| {
                let on = chosen.contains(&t.as_str());
                (t, on)
            })
            .collect();
        let capture = if item.is_none() { Capture::Requested } else { Capture::No };
        self.workshop.dialog = Some(Dialog { app: w.app, name, upload, tags, capture, job: None, staging: None, result: None, progress: None });
    }

    pub(crate) fn workshop_window(&mut self, ctx: &egui::Context) {
        let Ok(steam) = &self.workshop.steam else { return };
        steam.pump();
        let Some(d) = &mut self.workshop.dialog else { return };
        match d.capture {
            Capture::Requested => {
                // The window stays hidden for a frame so the shot shows only the map.
                d.capture = Capture::Sent;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(PreviewShot)));
                ctx.request_repaint();
                return;
            }
            Capture::Sent => {
                let shot = ctx.input(|i| {
                    i.events.iter().find_map(|e| match e {
                        egui::Event::Screenshot { image, user_data, .. } if user_data.data.as_ref().is_some_and(|d| d.is::<PreviewShot>()) => {
                            Some(image.clone())
                        }
                        _ => None,
                    })
                });
                let Some(image) = shot else {
                    ctx.request_repaint();
                    return;
                };
                d.capture = Capture::No;
                let scale = image.size[0] as f32 / ctx.content_rect().width().max(1.0);
                let pane = self.panes.iter().find(|p| p.uv.is_none() && !p.camera.is_2d()).map(|p| p.rect * scale);
                let path = self.data_dir.as_ref().map(|dir| dir.join("workshop").join(format!("{}.jpg", d.name)));
                match path.ok_or_else(|| "no data folder".to_string()).and_then(|p| save_preview(&image, pane, &p).map(|()| p)) {
                    Ok(p) => d.upload.preview = Some(p),
                    Err(e) => d.result = Some(Err(format!("Cannot make the preview: {e}"))),
                }
            }
            Capture::No => {}
        }
        if let Some(job) = &mut d.job {
            let state = steam.poll(job);
            if let Some(id) = job.item_id() {
                d.upload.item = Some(id);
                let saved = json!({
                    "item": id,
                    "title": d.upload.title,
                    "description": d.upload.description,
                    "visibility": d.upload.visibility.label(),
                    "tags": d.upload.tags,
                });
                if !self.session.view.is_object() {
                    self.session.view = Value::Object(serde_json::Map::new());
                }
                if self.session.view.get(KEY) != Some(&saved) {
                    if let Some(o) = self.session.view.as_object_mut() {
                        o.insert(KEY.into(), saved);
                    }
                    self.session.mark_unsaved();
                }
            }
            match state {
                State::Working { stage, done, total } => {
                    d.progress = Some((stage, done, total));
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
                State::Done(result) => {
                    d.job = None;
                    d.progress = None;
                    if let Ok(p) = &result {
                        d.upload.item = Some(p.id);
                        self.output.push(format!("Published Steam Workshop item {}", p.id));
                    }
                    d.result = Some(result);
                }
            }
        }
        let Some(d) = &mut self.workshop.dialog else { return };
        let mut acts = Vec::new();
        let mut open = true;
        egui::Window::new("Publish to Steam Workshop").open(&mut open).default_width(440.0).collapsible(false).show(ctx, |ui| {
            let busy = d.job.is_some();
            ui.add_enabled_ui(!busy, |ui| {
                egui::Grid::new("workshop").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                    let label = ui.label("Title");
                    ui.add(egui::TextEdit::singleline(&mut d.upload.title).char_limit(mapcraft_steam::TITLE_MAX).desired_width(f32::INFINITY))
                        .labelled_by(label.id);
                    ui.end_row();
                    let label = ui.label("Description");
                    ui.add(egui::TextEdit::multiline(&mut d.upload.description).desired_rows(4).desired_width(f32::INFINITY)).labelled_by(label.id);
                    ui.end_row();
                    ui.label("Visibility");
                    egui::ComboBox::from_id_salt("workshop-visibility").selected_text(d.upload.visibility.label()).show_ui(ui, |ui| {
                        for v in Visibility::ALL {
                            ui.selectable_value(&mut d.upload.visibility, v, v.label());
                        }
                    });
                    ui.end_row();
                    if !d.tags.is_empty() {
                        ui.label("Tags");
                        ui.horizontal_wrapped(|ui| {
                            for (tag, on) in &mut d.tags {
                                ui.checkbox(on, tag.as_str());
                            }
                        });
                        ui.end_row();
                    }
                    let label = ui.label("Change note");
                    ui.add(egui::TextEdit::multiline(&mut d.upload.change_note).desired_rows(2).desired_width(f32::INFINITY)).labelled_by(label.id);
                    ui.end_row();
                    ui.label("Preview");
                    ui.vertical(|ui| {
                        let shown = match &d.upload.preview {
                            Some(p) => p.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                            None if d.upload.item.is_some() => "Keeps the current one".into(),
                            None => "None".into(),
                        };
                        ui.label(RichText::new(shown).color(TEXT_WEAK));
                        ui.horizontal(|ui| {
                            if ui.button("Use 3D View").clicked() {
                                acts.push(Act::Capture);
                            }
                            if ui.button("Choose Image…").clicked() {
                                acts.push(Act::Choose);
                            }
                        });
                    });
                    ui.end_row();
                });
                if let Some(id) = d.upload.item {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("Updates item {id}")).color(TEXT_WEAK));
                        if ui.small_button("Publish as a New Item").clicked() {
                            acts.push(Act::NewItem);
                        }
                    });
                }
            });
            ui.separator();
            if let Some((stage, done, total)) = d.progress {
                let fraction = if total > 0 { done as f32 / total as f32 } else { 0.0 };
                ui.add(egui::ProgressBar::new(fraction.min(1.0)).text(stage));
            }
            match &d.result {
                Some(Ok(p)) => {
                    ui.label("Published.");
                    if p.needs_agreement {
                        ui.colored_label(WARN, "The item stays hidden until you accept the Steam Workshop agreement on its page.");
                    }
                    if ui.button("Open in Steam").clicked() {
                        acts.push(Act::Open(p.id));
                    }
                }
                Some(Err(e)) => {
                    ui.colored_label(ERROR, e);
                }
                None => {}
            }
            let label = if d.upload.item.is_some() { "Update" } else { "Publish" };
            if ui.add_enabled(!busy, egui::Button::new(label)).clicked() {
                acts.push(Act::Publish);
            }
        });
        if !open && d.job.is_none() {
            self.workshop.dialog = None;
            return;
        }
        for act in acts {
            self.workshop_act(act);
        }
    }

    fn workshop_act(&mut self, act: Act) {
        let Some(d) = &mut self.workshop.dialog else { return };
        match act {
            Act::Capture => d.capture = Capture::Requested,
            Act::Choose => {
                if let Some(p) = rfd::FileDialog::new().add_filter("Image", &["jpg", "jpeg", "png", "gif"]).pick_file() {
                    d.upload.preview = Some(p);
                }
            }
            Act::NewItem => d.upload.item = None,
            Act::Open(id) => {
                if let Err(e) = crate::sdk_manager::open_folder(Path::new(&mapcraft_steam::item_url(id))) {
                    self.status = e;
                }
            }
            Act::Publish => self.start_publish(),
        }
    }

    fn start_publish(&mut self) {
        let content = self.workshop_content();
        let Some(d) = &mut self.workshop.dialog else { return };
        d.result = None;
        d.upload.tags = d.tags.iter().filter(|(_, on)| *on).map(|(t, _)| t.clone()).collect();
        let started = content.and_then(|dir| {
            d.upload.content = dir.path().to_path_buf();
            d.staging = Some(dir);
            let Ok(steam) = &self.workshop.steam else { return Err("Steam is not running".to_string()) };
            steam.publish(d.app, d.upload.clone())
        });
        match started {
            Ok(job) => d.job = Some(job),
            Err(e) => d.result = Some(Err(e)),
        }
        let saved = json!({
            "item": d.upload.item,
            "title": d.upload.title,
            "description": d.upload.description,
            "visibility": d.upload.visibility.label(),
            "tags": d.upload.tags,
        });
        if !self.session.view.is_object() {
            self.session.view = Value::Object(serde_json::Map::new());
        }
        if let Some(o) = self.session.view.as_object_mut() {
            o.insert(KEY.into(), saved);
        }
        self.session.mark_unsaved();
    }

    /// Writes the item's folder: the map, with prefabs and mirrors baked in, and its pack when it has one.
    fn workshop_content(&self) -> Result<tempfile::TempDir, String> {
        let entries = self.custom_asset_entries()?;
        let name = self.workshop.dialog.as_ref().map(|d| d.name.clone()).ok_or("nothing to publish")?;
        let staging = tempfile::Builder::new().prefix("mapcraft-workshop-").tempdir().map_err(|e| e.to_string())?;
        let dir = staging.path();
        let file = dir.join(format!("{name}.{}", mapcraft_format::EXTENSION));
        mapcraft_format::save(&file, self.view_map(), self.session.sdk.as_deref(), &Value::Null).map_err(|e| e.to_string())?;
        if !entries.is_empty() {
            pack::save(&pack::path_for(&file), &entries).map_err(|e| e.to_string())?;
        }
        Ok(staging)
    }
}

/// Saves the 3D view, or the whole window without one, as a 1280x720 JPEG under Steam's preview limit.
fn save_preview(shot: &egui::ColorImage, pane: Option<egui::Rect>, path: &Path) -> Result<(), String> {
    let [w, h] = shot.size.map(|n| n as u32);
    let bytes: Vec<u8> = shot.pixels.iter().flat_map(|c| c.to_array()).collect();
    let full = image::RgbaImage::from_raw(w, h, bytes).ok_or("the screenshot is empty")?;
    let r = pane.unwrap_or(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w as f32, h as f32)));
    let (x, y) = (r.min.x.max(0.0) as u32, r.min.y.max(0.0) as u32);
    let (pw, ph) = ((r.width().max(0.0) as u32).min(w.saturating_sub(x)), (r.height().max(0.0) as u32).min(h.saturating_sub(y)));
    let (cw, ch) = if pw * 9 > ph * 16 { (ph * 16 / 9, ph) } else { (pw, pw * 9 / 16) };
    if cw == 0 || ch == 0 {
        return Err("the 3D view is too small".into());
    }
    let crop = image::imageops::crop_imm(&full, x + (pw - cw) / 2, y + (ph - ch) / 2, cw, ch).to_image();
    let scaled = image::imageops::resize(&crop, 1280, 720, image::imageops::FilterType::Triangle);
    let rgb = image::DynamicImage::ImageRgba8(scaled).to_rgb8();
    let jpeg = [90, 80, 70, 60, 50]
        .into_iter()
        .find_map(|q| {
            let mut out = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, q).encode_image(&rgb).ok()?;
            ((out.len() as u64) < mapcraft_steam::PREVIEW_MAX).then_some(out)
        })
        .ok_or("the preview would not fit under 1 MB")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, jpeg).map_err(|e| e.to_string())
}
