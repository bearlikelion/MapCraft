//! File > SDK Manager: installed SDKs with checks, SDKs from registries, and new SDKs from the
//! template or a TrenchBroom game; and the Add an SDK window a fresh install opens with.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use egui::{Color32, ColorImage, RichText, TextureHandle, Vec2, vec2};
use mapcraft_sdk::Sdk;
use mapcraft_sdk_manager::{self as manager, Entry, Item, Level, Origin, Registry, TrenchBroomImport};

use crate::app::MapCraftApp;
use crate::chrome::{ACCENT, TEXT, TEXT_WEAK};

const PASS: Color32 = Color32::from_rgb(110, 200, 80);
const WARN: Color32 = Color32::from_rgb(230, 190, 60);
const FAIL: Color32 = Color32::from_rgb(232, 90, 80);

/// Logos are drawn this size; registry logos are 460 by 215 pixels.
const LOGO: Vec2 = vec2(136.0, 64.0);
const MAX_LOGO: u64 = 1 << 20;

fn level_color(level: Level) -> Color32 {
    match level {
        Level::Pass => PASS,
        Level::Warn => WARN,
        Level::Fail => FAIL,
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    Installed,
    Available,
    Create,
}

/// Work on a background thread; `poll` gives its result once, or an error if the thread died.
struct Job<T>(mpsc::Receiver<T>);

impl<T: Send + 'static> Job<T> {
    fn spawn(f: impl FnOnce() -> T + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        Self(rx)
    }

    fn poll(&self) -> Option<Result<T, String>> {
        match self.0.try_recv() {
            Ok(v) => Some(Ok(v)),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err("the background task stopped unexpectedly".into())),
        }
    }
}

struct Download {
    entry: Entry,
    /// Bytes so far and in total.
    progress: Arc<[AtomicU64; 2]>,
    cancel: Arc<AtomicBool>,
    job: Job<Result<Sdk, String>>,
}

type Fetched = Vec<(String, Result<Registry, String>)>;
type ImportJob = Job<Result<(Sdk, Vec<String>), String>>;

/// SDK logos by source, decoded on worker threads; one that failed stays `None` and shows the name tile.
#[derive(Default)]
struct Logos {
    loaded: HashMap<String, Option<TextureHandle>>,
    loading: Vec<(String, Job<Option<ColorImage>>)>,
}

impl Logos {
    /// Starts loading `source` unless it is loaded or loading; a remote one is kept in `cache`
    /// under a name that changes with `version`.
    fn want(&mut self, source: String, cache: Option<&Path>, version: &str) {
        if self.loaded.contains_key(&source) || self.loading.iter().any(|(s, _)| *s == source) {
            return;
        }
        let file = cache.filter(|_| source.starts_with("http")).map(|c| c.join(manager::sha256_hex(format!("{source} {version}").as_bytes())));
        let s = source.clone();
        self.loading.push((source, Job::spawn(move || load_logo(&s, file.as_deref()))));
    }

    /// Uploads finished logos; true while some are still loading.
    fn poll(&mut self, ctx: &egui::Context) -> bool {
        let loaded = &mut self.loaded;
        self.loading.retain(|(source, job)| {
            let Some(img) = job.poll() else { return true };
            let tex = img.ok().flatten().map(|i| ctx.load_texture(format!("logo:{source}"), i, egui::TextureOptions::LINEAR));
            loaded.insert(source.clone(), tex);
            false
        });
        !self.loading.is_empty()
    }

    fn get(&self, source: Option<&str>) -> Option<&TextureHandle> {
        self.loaded.get(source?)?.as_ref()
    }
}

fn load_logo(source: &str, cache: Option<&Path>) -> Option<ColorImage> {
    if let Some(img) = cache.and_then(|c| std::fs::read(c).ok()).and_then(|b| decode_logo(&b)) {
        return Some(img);
    }
    let bytes = manager::fetch(source, MAX_LOGO, &mut |_, _| {}, &AtomicBool::new(false)).ok()?;
    let img = decode_logo(&bytes)?;
    if let Some(c) = cache {
        let _ = c.parent().map(std::fs::create_dir_all);
        let _ = std::fs::write(c, &bytes);
    }
    Some(img)
}

/// A PNG or JPEG of at most 4096 pixels a side, shrunk to twice the drawn size.
fn decode_logo(bytes: &[u8]) -> Option<ColorImage> {
    let format = image::guess_format(bytes).ok().filter(|f| matches!(f, image::ImageFormat::Png | image::ImageFormat::Jpeg))?;
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    reader.limits(limits);
    let rgba = reader.decode().ok()?.thumbnail(LOGO.x as u32 * 2, LOGO.y as u32 * 2).to_rgba8();
    Some(ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()))
}

/// The logo fitted into a `LOGO` sized tile, or the SDK's name on a tile colored from it.
fn logo_tile(ui: &mut egui::Ui, logo: Option<&TextureHandle>, name: &str) {
    let (rect, _) = ui.allocate_exact_size(LOGO, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    if let Some(t) = logo {
        painter.rect_filled(rect, 4.0, crate::chrome::BG);
        let size = t.size_vec2();
        let scale = (LOGO.x / size.x).min(LOGO.y / size.y);
        egui::Image::new((t.id(), size)).corner_radius(4).paint_at(ui, egui::Rect::from_center_size(rect.center(), size * scale));
        return;
    }
    let hue = name.bytes().fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b))) % 360;
    painter.rect_filled(rect, 4.0, egui::ecolor::Hsva::new(hue as f32 / 360.0, 0.45, 0.32, 1.0));
    let text = painter.layout(name.to_string(), egui::FontId::proportional(15.0), TEXT, LOGO.x - 16.0);
    painter.galley(rect.center() - text.size() / 2.0, text, TEXT);
}

/// A rounded card with a logo tile left of `body`.
fn card(ui: &mut egui::Ui, logo: Option<&TextureHandle>, name: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::group(ui.style()).fill(ui.visuals().faint_bg_color).corner_radius(6).inner_margin(10).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_top(|ui| {
            logo_tile(ui, logo, name);
            ui.add_space(4.0);
            ui.vertical(body);
        });
    });
}

fn title(ui: &mut egui::Ui, name: &str, version: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(name).strong().size(15.0));
        ui.label(RichText::new(version).color(TEXT_WEAK));
    });
}

fn size_text(bytes: u64) -> String {
    if bytes >= 1 << 20 { format!("{:.1} MiB", bytes as f64 / f64::from(1 << 20)) } else { format!("{} KiB", bytes.div_ceil(1024)) }
}

#[derive(Default)]
pub(crate) struct SdkManager {
    pub open: bool,
    /// The Add an SDK window, shown at startup while no SDK is found.
    pub first_run: bool,
    tab: Tab,
    /// The last check of each SDK folder.
    checks: BTreeMap<PathBuf, Vec<Item>>,
    running: Vec<(PathBuf, Job<Vec<Item>>)>,
    /// `None` until fetched; loads whenever a list of them shows.
    registries: Option<Fetched>,
    fetching: Option<Job<Fetched>>,
    logos: Logos,
    confirm_install: Option<(String, Entry)>,
    /// Folder name of the user SDK waiting for the remove confirmation.
    confirm_remove: Option<String>,
    download: Option<Download>,
    new_id: String,
    new_name: String,
    /// A TrenchBroom game folder or Godot project to import.
    import_folder: String,
    importing: Option<ImportJob>,
    /// The outcome of the last install, removal or creation, and whether it worked.
    note: Option<(bool, String)>,
}

impl SdkManager {
    pub(crate) fn check_results(&self, dir: &Path) -> Option<&Vec<Item>> {
        self.checks.get(dir)
    }

    /// Forgets fetched registries, and any fetch under way, so they load again.
    pub(crate) fn registries_changed(&mut self) {
        self.registries = None;
        self.fetching = None;
    }
}

enum Act {
    Tab(Tab),
    Configure(Sdk),
    Check(PathBuf),
    Test(Sdk),
    Open(PathBuf),
    Remove(String),
    Refresh,
    Install(String, Entry),
    Cancel,
    NotNow,
    Create,
    BrowseImport,
    Import,
}

/// Shows a folder in the system file manager.
fn open_folder(dir: &Path) -> Result<(), String> {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let mut child = std::process::Command::new(program).arg(dir).spawn().map_err(|e| format!("Cannot open {}: {e}", dir.display()))?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

fn check_results(ui: &mut egui::Ui, items: &[Item]) {
    let count = |l| items.iter().filter(|i| i.level == l).count();
    let summary = format!("{} passed, {} warnings, {} failed", count(Level::Pass), count(Level::Warn), count(Level::Fail));
    ui.label(RichText::new(summary).color(level_color(manager::worst(items))));
    for i in items {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(i.level.label()).monospace().color(level_color(i.level)));
            ui.label(RichText::new(&i.what).strong());
            ui.label(&i.message);
        });
    }
}

impl MapCraftApp {
    pub(crate) fn sdk_manager_window(&mut self, ctx: &egui::Context) {
        self.poll_sdk_jobs(ctx);
        let m = &self.sdk_manager;
        let lists = m.first_run || m.open && m.tab == Tab::Available;
        let shown = m.first_run || m.open;
        if lists && m.registries.is_none() && m.fetching.is_none() {
            self.fetch_registries();
        }
        if shown {
            self.want_logos();
        }
        let mut acts = Vec::new();
        self.first_run_window(ctx, &mut acts);
        let mut open = self.sdk_manager.open;
        egui::Window::new("SDK Manager").open(&mut open).default_size([700.0, 560.0]).show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (t, label, icon) in
                    [(Tab::Installed, "Installed", "storage"), (Tab::Available, "Available", "cloud_arrow_down"), (Tab::Create, "Create", "plus")]
                {
                    if ui.add(crate::icons::selectable(self.sdk_manager.tab == t, Some(icon), label)).clicked() {
                        acts.push(Act::Tab(t));
                    }
                }
            });
            ui.separator();
            if let Some((ok, text)) = &self.sdk_manager.note {
                ui.colored_label(if *ok { PASS } else { FAIL }, text);
            }
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| match self.sdk_manager.tab {
                Tab::Installed => self.installed_tab(ui, &mut acts),
                Tab::Available => self.available_tab(ui, &mut acts),
                Tab::Create => self.create_tab(ui, &mut acts),
            });
        });
        self.sdk_manager.open = open;
        for act in acts {
            self.sdk_act(act);
        }
        self.sdk_modals(ctx);
    }

    /// Queues the logos of registry entries and installed SDKs.
    fn want_logos(&mut self) {
        let cache = self.data_dir.as_ref().map(|d| d.join("logos"));
        let m = &mut self.sdk_manager;
        for (url, reg) in m.registries.iter().flatten() {
            for e in reg.iter().flat_map(|r| &r.sdks) {
                if let Some(source) = e.logo_source(url) {
                    m.logos.want(source, cache.as_deref(), &e.version);
                }
            }
        }
        for logo in self.sdks.iter().filter_map(|i| i.logo.as_ref()) {
            m.logos.want(logo.to_string_lossy().into_owned(), None, "");
        }
    }

    fn first_run_window(&self, ctx: &egui::Context, acts: &mut Vec<Act>) {
        if !self.sdk_manager.first_run {
            return;
        }
        let screen = ctx.content_rect();
        egui::Modal::new(egui::Id::new("add-sdk")).show(ctx, |ui| {
            ui.set_width((screen.width() - 64.0).clamp(300.0, 620.0));
            ui.heading("Add an SDK");
            ui.label("An SDK teaches MapCraft a game: its entities, materials and compile tools. Install one to start, then point it at your game in the SDK Settings that open next.");
            ui.add_space(6.0);
            egui::ScrollArea::vertical().max_height((screen.height() - 240.0).max(160.0)).show(ui, |ui| self.sdk_cards(ui, acts));
            if let Some((false, text)) = &self.sdk_manager.note {
                ui.colored_label(FAIL, text);
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("File > SDK Manager installs more later.").color(TEXT_WEAK));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Not now").clicked() {
                        acts.push(Act::NotNow);
                    }
                });
            });
        });
    }

    fn installed_tab(&self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        if self.sdks.is_empty() {
            ui.label("No SDKs found. Install one from Available or make one in Create.");
        }
        for (index, inst) in self.sdks.iter().enumerate() {
            let sdk = inst.sdk.as_ref().ok();
            let id = sdk.map(|s| s.manifest.id.as_str());
            let earlier = self.sdks.get(..index).unwrap_or_default();
            let shadowed = id.is_some_and(|id| earlier.iter().any(|e| e.sdk.as_ref().is_ok_and(|s| s.manifest.id == id)));
            let running = self.sdk_manager.running.iter().any(|(d, _)| *d == inst.dir);
            let folder = inst.dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let name = sdk.map_or(folder.as_str(), |s| s.manifest.name.as_str());
            let logo = self.sdk_manager.logos.get(inst.logo.as_ref().map(|p| p.to_string_lossy()).as_deref());
            ui.add_space(4.0);
            card(ui, logo, name, |ui| {
                ui.horizontal_wrapped(|ui| {
                    title(ui, name, sdk.map_or("", |s| s.manifest.version.as_str()));
                    ui.label(RichText::new(inst.origin.label()).small().color(ACCENT));
                    if id.is_some() && self.session.sdk.as_deref() == id && !shadowed {
                        ui.label(RichText::new("used by this map").small().color(PASS));
                    }
                });
                ui.label(RichText::new(inst.dir.display().to_string()).monospace().small().color(TEXT_WEAK));
                match &inst.sdk {
                    Ok(s) if !s.manifest.description.is_empty() => {
                        ui.label(&s.manifest.description);
                    }
                    Ok(_) => {}
                    Err(e) => {
                        ui.colored_label(FAIL, e);
                    }
                }
                if shadowed {
                    ui.label(RichText::new("A copy with the same id is listed above and loads instead.").color(TEXT_WEAK));
                }
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(sdk.is_some(), crate::icons::button(Some("cog"), "Configure"))
                        .on_hover_text("Set the folders and programs it uses")
                        .clicked()
                        && let Some(s) = sdk
                    {
                        acts.push(Act::Configure(s.clone()));
                    }
                    if ui
                        .add_enabled(!running, crate::icons::button(Some("list_checkboxes"), "Check"))
                        .on_hover_text("Check the manifest, FGDs, plugins, paths and samples")
                        .clicked()
                    {
                        acts.push(Act::Check(inst.dir.clone()));
                    }
                    let test = ui.add_enabled(sdk.is_some() && !running, crate::icons::button(Some("play"), "Test"));
                    if test.on_hover_text("Open its first sample map, or check it when it has none").clicked()
                        && let Some(s) = sdk
                    {
                        acts.push(Act::Test(s.clone()));
                    }
                    if ui.add(crate::icons::button(Some("folder_open"), "Open Folder")).clicked() {
                        acts.push(Act::Open(inst.dir.clone()));
                    }
                    if inst.origin == Origin::User && ui.add(crate::icons::button(Some("trash_can"), "Remove…")).clicked() {
                        acts.push(Act::Remove(folder.clone()));
                    }
                    if running {
                        ui.spinner();
                    }
                });
                if let Some(items) = self.sdk_manager.checks.get(&inst.dir) {
                    check_results(ui, items);
                }
            });
        }
    }

    fn available_tab(&self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        ui.horizontal(|ui| {
            if ui.add_enabled(self.sdk_manager.fetching.is_none(), crate::icons::button(Some("arrows_clockwise"), "Refresh")).clicked() {
                acts.push(Act::Refresh);
            }
            ui.label(RichText::new("Registries are listed in Preferences.").color(TEXT_WEAK));
        });
        ui.add_space(4.0);
        self.sdk_cards(ui, acts);
    }

    /// Every registry's SDKs as cards, or why there are none.
    fn sdk_cards(&self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        let m = &self.sdk_manager;
        let Some(registries) = m.registries.as_ref().filter(|_| m.fetching.is_none()) else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Loading the list of SDKs…");
            });
            return;
        };
        if registries.is_empty() {
            ui.label("No SDK registries are set. Add one in Edit > Preferences.");
        }
        for (url, reg) in registries {
            match reg {
                Ok(reg) => {
                    if registries.len() > 1 {
                        ui.add_space(4.0);
                        ui.label(RichText::new(if reg.name.is_empty() { url } else { &reg.name }).strong()).on_hover_text(url);
                    }
                    if reg.sdks.is_empty() {
                        ui.label(RichText::new(format!("{} lists no SDKs yet.", if reg.name.is_empty() { url } else { &reg.name })).color(TEXT_WEAK));
                    }
                    for e in &reg.sdks {
                        ui.add_space(4.0);
                        self.sdk_card(ui, url, e, acts);
                    }
                }
                Err(e) => {
                    ui.colored_label(FAIL, "The list of SDKs did not load. Check your connection, then Retry.");
                    ui.label(RichText::new(e).size(11.0).color(TEXT_WEAK));
                }
            }
        }
        if registries.iter().any(|(_, r)| r.is_err()) && ui.add(crate::icons::button(Some("arrows_clockwise"), "Retry")).clicked() {
            acts.push(Act::Refresh);
        }
    }

    fn sdk_card(&self, ui: &mut egui::Ui, registry: &str, e: &Entry, acts: &mut Vec<Act>) {
        let m = &self.sdk_manager;
        let have = self.sdks.iter().find_map(|i| i.sdk.as_ref().ok().filter(|s| s.manifest.id == e.id));
        card(ui, m.logos.get(e.logo_source(registry).as_deref()), &e.name, |ui| {
            title(ui, &e.name, &e.version);
            if !e.description.is_empty() {
                ui.label(&e.description);
            }
            let by = (!e.author.is_empty()).then(|| format!("By {}", e.author));
            let small = [by, (!e.license.is_empty()).then(|| e.license.clone()), Some(size_text(e.size))];
            ui.label(RichText::new(small.into_iter().flatten().collect::<Vec<_>>().join(", ")).size(11.0).color(TEXT_WEAK));
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                if let Some(d) = m.download.as_ref().filter(|d| d.entry.id == e.id) {
                    let [done, total] = [&d.progress[0], &d.progress[1]].map(|a| a.load(Ordering::Relaxed));
                    let fraction = if total > 0 { done as f32 / total as f32 } else { 0.0 };
                    let text = format!("Installing, {} of {}", size_text(done), size_text(total));
                    ui.add(egui::ProgressBar::new(fraction.min(1.0)).text(text).desired_width(ui.available_width().min(280.0)));
                    if ui.button("Cancel").clicked() {
                        acts.push(Act::Cancel);
                    }
                    return;
                }
                let label = match have {
                    Some(s) if !manager::newer(&e.version, &s.manifest.version) => None,
                    Some(_) => Some("Update…"),
                    None => Some("Install…"),
                };
                if let Some(why) = e.unsupported() {
                    ui.colored_label(WARN, why);
                } else if let Some(label) = label {
                    if ui.add_enabled(m.download.is_none(), crate::icons::button(Some("cloud_arrow_down"), label)).clicked() {
                        acts.push(Act::Install(registry.to_string(), e.clone()));
                    }
                } else {
                    ui.label(RichText::new("Installed").color(PASS));
                }
                if let Some(s) = have.filter(|s| s.manifest.version != e.version) {
                    ui.label(RichText::new(format!("installed: {}", s.manifest.version)).color(TEXT_WEAK));
                }
                if !e.homepage.is_empty() {
                    ui.hyperlink_to("Homepage", &e.homepage);
                }
            });
        });
    }

    fn create_tab(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        ui.label("Start a new SDK from the template: a commented sdk.toml, a starter FGD and a README.");
        ui.add_space(4.0);
        let target = self.data_dir.as_deref().map(|d| manager::user_dir(d).join(self.sdk_manager.new_id.trim()));
        let m = &mut self.sdk_manager;
        egui::Grid::new("sdk-create").num_columns(2).show(ui, |ui| {
            let label = ui.label("Id");
            ui.add(egui::TextEdit::singleline(&mut m.new_id).hint_text("my-game").desired_width(240.0)).labelled_by(label.id);
            ui.end_row();
            let label = ui.label("Name");
            ui.add(egui::TextEdit::singleline(&mut m.new_name).hint_text("My Game").desired_width(240.0)).labelled_by(label.id);
            ui.end_row();
        });
        let id = m.new_id.trim();
        let valid = mapcraft_sdk::valid_id(id);
        let taken = valid && target.as_ref().is_some_and(|t| t.exists());
        if !id.is_empty() && !valid {
            ui.colored_label(WARN, "The id may use lowercase letters, digits, '_' and '-'. Maps store it, so pick it once.");
        } else if taken {
            ui.colored_label(WARN, "An SDK folder with this id already exists.");
        }
        match &target {
            Some(t) if valid => ui.label(RichText::new(format!("Creates {}", t.display())).color(TEXT_WEAK)),
            Some(_) => ui.label(RichText::new("The id names its folder and is saved in every map made with it.").color(TEXT_WEAK)),
            None => ui.colored_label(FAIL, "There is no user data folder to create SDKs in."),
        };
        let ready = valid && !taken && !m.new_name.trim().is_empty() && target.is_some();
        if ui.add_enabled(ready, egui::Button::new("Create SDK")).clicked() {
            acts.push(Act::Create);
        }

        ui.add_space(12.0);
        ui.separator();
        ui.strong("Import from TrenchBroom / func_godot");
        ui.label("Make an SDK from a TrenchBroom game, such as one func_godot exported: pick its folder in TrenchBroom's games folder, or the Godot project that uses func_godot.");
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let label = ui.label("Folder");
            ui.add(egui::TextEdit::singleline(&mut m.import_folder).hint_text("TrenchBroom game folder or Godot project").desired_width(360.0))
                .labelled_by(label.id);
            if ui.button("Browse…").clicked() {
                acts.push(Act::BrowseImport);
            }
        });
        ui.horizontal(|ui| {
            let ready = m.importing.is_none() && !m.import_folder.trim().is_empty() && target.is_some();
            if ui.add_enabled(ready, egui::Button::new("Import SDK")).clicked() {
                acts.push(Act::Import);
            }
            if m.importing.is_some() {
                ui.spinner();
            }
        });
    }

    fn sdk_act(&mut self, act: Act) {
        match act {
            Act::Tab(t) => self.sdk_manager.tab = t,
            Act::Configure(sdk) => {
                self.sdk_settings_for = Some(sdk);
                self.show_sdk_dialog = true;
            }
            Act::Check(dir) => self.start_check(dir),
            Act::Test(sdk) => match manager::samples(&sdk.dir).into_iter().next() {
                Some(map) => {
                    self.sdk_manager.open = false;
                    self.open_dialog_path(map);
                }
                None => self.start_check(sdk.dir),
            },
            Act::Open(dir) => {
                if let Err(e) = open_folder(&dir) {
                    self.sdk_manager.note = Some((false, e));
                }
            }
            Act::Remove(folder) => self.sdk_manager.confirm_remove = Some(folder),
            Act::Refresh => self.fetch_registries(),
            Act::Install(registry, entry) => self.sdk_manager.confirm_install = Some((registry, entry)),
            Act::Cancel => {
                if let Some(d) = &self.sdk_manager.download {
                    d.cancel.store(true, Ordering::Relaxed);
                }
            }
            Act::NotNow => self.sdk_manager.first_run = false,
            Act::Create => self.create_sdk(),
            Act::BrowseImport => {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    self.sdk_manager.import_folder = dir.to_string_lossy().into_owned();
                }
            }
            Act::Import => self.start_import(),
        }
    }

    /// Reads the TrenchBroom game or Godot project and writes its SDK into the user SDK folder.
    fn start_import(&mut self) {
        let Some(sdks) = self.data_dir.as_deref().map(manager::user_dir) else { return };
        let folder = PathBuf::from(self.sdk_manager.import_folder.trim());
        let found: Vec<PathBuf> = self.sdks.iter().map(|i| i.dir.clone()).collect();
        self.sdk_manager.note = None;
        self.sdk_manager.importing = Some(Job::spawn(move || {
            let game = TrenchBroomImport::read(&folder).map_err(|e| e.to_string())?;
            game.write(&sdks.join(game.id()), &found).map_err(|e| e.to_string())
        }));
    }

    fn start_check(&mut self, dir: PathBuf) {
        let sdk = self.sdks.iter().find(|i| i.dir == dir).and_then(|i| i.sdk.as_ref().ok());
        let vars = sdk.map(|s| self.sdk_vars(&s.manifest.id, Some(s))).unwrap_or_default();
        let d = dir.clone();
        // Plugins compile on the worker too, so the editor never stalls on a slow one.
        let job = Job::spawn(move || manager::check(&d, &vars, mapcraft_plugin_host::Host::new().ok().as_ref()));
        self.sdk_manager.running.push((dir, job));
    }

    fn fetch_registries(&mut self) {
        let urls = self.prefs.get().registries.clone();
        self.sdk_manager.fetching =
            Some(Job::spawn(move || urls.into_iter().map(|u| (u.clone(), manager::fetch_registry(&u).map_err(|e| e.to_string()))).collect()));
    }

    fn start_install(&mut self, registry: String, entry: Entry) {
        let Some(sdks) = self.data_dir.as_deref().map(manager::user_dir) else {
            self.sdk_manager.note = Some((false, "There is no user data folder to install SDKs into.".into()));
            return;
        };
        let progress = Arc::new([AtomicU64::new(0), AtomicU64::new(entry.size)]);
        let cancel = Arc::new(AtomicBool::new(false));
        let (p, c, e) = (progress.clone(), cancel.clone(), entry.clone());
        let job = Job::spawn(move || {
            let mut report = |done, total: Option<u64>| {
                p[0].store(done, Ordering::Relaxed);
                if let Some(t) = total {
                    p[1].store(t, Ordering::Relaxed);
                }
            };
            manager::install(&e, &registry, &sdks, &mut report, &c).map_err(|e| e.to_string())
        });
        self.sdk_manager.note = None;
        self.sdk_manager.download = Some(Download { entry, progress, cancel, job });
    }

    fn create_sdk(&mut self) {
        let Some(data) = self.data_dir.clone() else { return };
        let id = self.sdk_manager.new_id.trim().to_string();
        let name = self.sdk_manager.new_name.trim().to_string();
        let dir = manager::user_dir(&data).join(&id);
        self.sdk_manager.note = Some(match manager::create(&dir, &id, &name) {
            Ok(sdk) => {
                self.rescan_sdks();
                self.sdk_manager.tab = Tab::Installed;
                self.sdk_manager.new_id.clear();
                self.sdk_manager.new_name.clear();
                (true, format!("Created {} in {}. Edit its sdk.toml and FGD, then Check it.", sdk.manifest.name, sdk.dir.display()))
            }
            Err(e) => (false, e.to_string()),
        });
    }

    /// After an SDK was installed, updated or removed: its programs must be approved again, and
    /// the open map reloads it when it uses that id.
    fn sdk_changed(&mut self, id: &str) {
        self.prefs.update(|p| {
            p.approved_tools.remove(id);
        });
        self.rescan_sdks();
        if self.session.sdk.as_deref() == Some(id) {
            self.load_project(Some(id.to_string()));
        }
    }

    /// Loads a newly installed SDK and opens its settings. An untitled, unchanged map with no SDK
    /// starts over on it, as File > New Map would; any other map keeps its SDK.
    fn sdk_installed(&mut self, sdk: Sdk) {
        let id = sdk.manifest.id.clone();
        self.sdk_changed(&id);
        if self.session.path.is_none() && !self.session.is_dirty() && self.project.sdk.is_none() {
            self.new_now(Some(id.clone()));
        } else if self.session.sdk.as_deref() != Some(id.as_str()) {
            let name = &sdk.manifest.name;
            self.status = format!("Installed {name}; this map keeps its SDK, and File > New Map starts a map on {name}");
        }
        self.sdk_settings_for = Some(sdk);
        self.show_sdk_dialog = true;
        self.sdk_manager.first_run = false;
    }

    fn poll_sdk_jobs(&mut self, ctx: &egui::Context) {
        let m = &mut self.sdk_manager;
        let mut busy = m.logos.poll(ctx);
        m.running.retain(|(dir, job)| match job.poll() {
            None => {
                busy = true;
                true
            }
            Some(result) => {
                let failed = |e| vec![Item { level: Level::Fail, what: "check".into(), message: e }];
                m.checks.insert(dir.clone(), result.unwrap_or_else(failed));
                false
            }
        });
        match m.fetching.as_ref().map(Job::poll) {
            Some(None) => busy = true,
            Some(Some(result)) => {
                m.registries = Some(result.unwrap_or_else(|e| vec![(String::new(), Err(e))]));
                m.fetching = None;
            }
            None => {}
        }
        let imported = m.importing.as_ref().and_then(|j| Some(j.poll()?.and_then(|r| r)));
        busy |= m.importing.is_some() && imported.is_none();
        if imported.is_some() {
            m.importing = None;
        }
        let finished = m.download.as_ref().and_then(|d| Some((d.entry.clone(), d.job.poll()?.and_then(|r| r))));
        busy |= m.download.is_some() && finished.is_none();
        if let Some((entry, result)) = finished {
            m.download = None;
            m.note = Some(match &result {
                Ok(sdk) => (true, format!("Installed {} {} into {}", sdk.manifest.name, sdk.manifest.version, sdk.dir.display())),
                Err(e) => (false, format!("{} was not installed: {e}", entry.name)),
            });
            if let Ok(sdk) = result {
                self.sdk_installed(sdk);
            }
        }
        match imported {
            Some(Ok((sdk, notes))) => {
                let text = std::iter::once(format!("Imported {} into {}.", sdk.manifest.name, sdk.dir.display())).chain(notes).collect::<Vec<_>>().join("\n");
                self.sdk_manager.note = Some((true, text));
                self.sdk_manager.import_folder.clear();
                self.sdk_manager.tab = Tab::Installed;
                self.sdk_changed(&sdk.manifest.id);
                self.start_check(sdk.dir);
            }
            Some(Err(e)) => self.sdk_manager.note = Some((false, e)),
            None => {}
        }
        if busy {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    fn sdk_modals(&mut self, ctx: &egui::Context) {
        if let Some((registry, e)) = self.sdk_manager.confirm_install.clone() {
            let mut choice = None;
            egui::Modal::new(egui::Id::new("sdk-install")).show(ctx, |ui| {
                ui.set_max_width(460.0);
                ui.heading(format!("Install {}?", e.name));
                egui::Grid::new("sdk-install-grid").num_columns(2).show(ui, |ui| {
                    let plugins = if e.plugins.is_empty() { "none".to_string() } else { e.plugins.join(", ") };
                    let rows = [
                        ("Version", e.version.as_str()),
                        ("Author", if e.author.is_empty() { "unknown" } else { &e.author }),
                        ("License", if e.license.is_empty() { "not given" } else { &e.license }),
                        ("Plugins", &plugins),
                        ("Size", &size_text(e.size)),
                        ("From", &e.source(&registry)),
                    ];
                    for (k, v) in rows {
                        ui.label(RichText::new(k).color(TEXT_WEAK));
                        ui.label(v);
                        ui.end_row();
                    }
                });
                ui.label("Plugins run sandboxed. The first time a compile profile runs a program, MapCraft asks before running it.");
                ui.horizontal(|ui| {
                    if ui.button("Install").clicked() {
                        choice = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(false);
                    }
                });
            });
            if choice.is_some() {
                self.sdk_manager.confirm_install = None;
            }
            if choice == Some(true) {
                self.start_install(registry, e);
            }
        }
        if let Some(folder) = self.sdk_manager.confirm_remove.clone() {
            let Some(sdks) = self.data_dir.as_deref().map(manager::user_dir) else {
                self.sdk_manager.confirm_remove = None;
                return;
            };
            let mut choice = None;
            egui::Modal::new(egui::Id::new("sdk-remove")).show(ctx, |ui| {
                ui.set_max_width(460.0);
                ui.heading(format!("Remove the {folder} SDK?"));
                ui.label(format!("This deletes {} and everything in it. Maps made with it open again once it is reinstalled.", sdks.join(&folder).display()));
                ui.horizontal(|ui| {
                    if ui.button("Remove").clicked() {
                        choice = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(false);
                    }
                });
            });
            if choice.is_some() {
                self.sdk_manager.confirm_remove = None;
            }
            if choice == Some(true) {
                match manager::uninstall(&sdks, &folder) {
                    Ok(()) => {
                        self.sdk_manager.note = Some((true, format!("Removed {folder}")));
                        self.sdk_manager.checks.remove(&sdks.join(&folder));
                        self.sdk_changed(&folder);
                    }
                    Err(e) => self.sdk_manager.note = Some((false, e.to_string())),
                }
            }
        }
    }
}
