//! File > SDK Manager: installed SDKs with checks, SDKs from registries, and new SDKs from the template.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use egui::{Color32, RichText};
use mapcraft_sdk::Sdk;
use mapcraft_sdk_manager::{self as manager, Entry, Item, Level, Origin, Registry};

use crate::app::MapCraftApp;
use crate::chrome::{ACCENT, TEXT_WEAK};

const PASS: Color32 = Color32::from_rgb(110, 200, 80);
const WARN: Color32 = Color32::from_rgb(230, 190, 60);
const FAIL: Color32 = Color32::from_rgb(232, 90, 80);

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

#[derive(Default)]
pub(crate) struct SdkManager {
    pub open: bool,
    tab: Tab,
    /// The last check of each SDK folder.
    checks: BTreeMap<PathBuf, Vec<Item>>,
    running: Vec<(PathBuf, Job<Vec<Item>>)>,
    registries: Fetched,
    fetching: Option<Job<Fetched>>,
    confirm_install: Option<(String, Entry)>,
    /// Folder name of the user SDK waiting for the remove confirmation.
    confirm_remove: Option<String>,
    download: Option<Download>,
    new_id: String,
    new_name: String,
    /// The outcome of the last install, removal or creation, and whether it worked.
    note: Option<(bool, String)>,
}

impl SdkManager {
    pub(crate) fn check_results(&self, dir: &Path) -> Option<&Vec<Item>> {
        self.checks.get(dir)
    }

    /// Forgets fetched registries so the Available tab loads them again.
    pub(crate) fn registries_changed(&mut self) {
        self.registries.clear();
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
    Create,
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
        if !self.sdk_manager.open {
            return;
        }
        let mut open = true;
        let mut acts = Vec::new();
        egui::Window::new("SDK Manager").open(&mut open).default_size([700.0, 560.0]).show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (t, label) in [(Tab::Installed, "Installed"), (Tab::Available, "Available"), (Tab::Create, "Create")] {
                    if ui.selectable_label(self.sdk_manager.tab == t, label).clicked() {
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
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.strong(sdk.map_or(folder.as_str(), |s| s.manifest.name.as_str()));
                    if let Some(s) = sdk {
                        ui.label(RichText::new(&s.manifest.version).color(TEXT_WEAK));
                    }
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
                ui.horizontal(|ui| {
                    if ui.add_enabled(sdk.is_some(), egui::Button::new("Configure")).on_hover_text("Set the folders and programs it uses").clicked()
                        && let Some(s) = sdk
                    {
                        acts.push(Act::Configure(s.clone()));
                    }
                    if ui.add_enabled(!running, egui::Button::new("Check")).on_hover_text("Check the manifest, FGDs, plugins, paths and samples").clicked() {
                        acts.push(Act::Check(inst.dir.clone()));
                    }
                    let test = ui.add_enabled(sdk.is_some() && !running, egui::Button::new("Test"));
                    if test.on_hover_text("Open its first sample map, or check it when it has none").clicked()
                        && let Some(s) = sdk
                    {
                        acts.push(Act::Test(s.clone()));
                    }
                    if ui.button("Open Folder").clicked() {
                        acts.push(Act::Open(inst.dir.clone()));
                    }
                    if inst.origin == Origin::User && ui.button("Remove…").clicked() {
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
        let m = &self.sdk_manager;
        ui.horizontal(|ui| {
            if ui.add_enabled(m.fetching.is_none(), egui::Button::new("Refresh")).clicked() {
                acts.push(Act::Refresh);
            }
            if m.fetching.is_some() {
                ui.spinner();
            }
            ui.label(RichText::new("Registries are listed in Preferences.").color(TEXT_WEAK));
        });
        if let Some(d) = &m.download {
            let [done, total] = [&d.progress[0], &d.progress[1]].map(|a| a.load(Ordering::Relaxed));
            let fraction = if total > 0 { done as f32 / total as f32 } else { 0.0 };
            ui.horizontal(|ui| {
                let text = format!("Installing {} ({} of {} KiB)", d.entry.name, done >> 10, total >> 10);
                ui.add(egui::ProgressBar::new(fraction.min(1.0)).text(text).desired_width(420.0));
                if ui.button("Cancel").clicked() {
                    d.cancel.store(true, Ordering::Relaxed);
                }
            });
        }
        for (url, reg) in &m.registries {
            ui.add_space(6.0);
            let reg = match reg {
                Ok(r) => r,
                Err(e) => {
                    ui.label(RichText::new(url).monospace().small().color(TEXT_WEAK));
                    ui.colored_label(FAIL, e.strip_prefix(&format!("{url}: ")).unwrap_or(e));
                    continue;
                }
            };
            ui.label(RichText::new(if reg.name.is_empty() { url } else { &reg.name }).strong()).on_hover_text(url);
            if reg.sdks.is_empty() {
                ui.label("No SDKs are listed yet.");
            }
            for e in &reg.sdks {
                let have = self.sdks.iter().find_map(|i| i.sdk.as_ref().ok().filter(|s| s.manifest.id == e.id));
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.strong(&e.name);
                        ui.label(RichText::new(&e.version).color(TEXT_WEAK));
                        ui.label(RichText::new(format!("{} KiB", e.size.div_ceil(1024))).small().color(TEXT_WEAK));
                    });
                    if !e.description.is_empty() {
                        ui.label(&e.description);
                    }
                    let by = if e.author.is_empty() { "Unknown author".to_string() } else { format!("By {}", e.author) };
                    let license = if e.license.is_empty() { "no license given" } else { &e.license };
                    ui.label(RichText::new(format!("{by}, {license}")).color(TEXT_WEAK));
                    ui.horizontal(|ui| {
                        let busy = m.download.is_some();
                        let label = match have {
                            Some(s) if !manager::newer(&e.version, &s.manifest.version) => None,
                            Some(_) => Some("Update…"),
                            None => Some("Install…"),
                        };
                        if let Some(why) = e.unsupported() {
                            ui.colored_label(WARN, why);
                        } else if let Some(label) = label {
                            if ui.add_enabled(!busy, egui::Button::new(label)).clicked() {
                                acts.push(Act::Install(url.clone(), e.clone()));
                            }
                        } else {
                            ui.label(RichText::new("Up to date").color(PASS));
                        }
                        if let Some(s) = have {
                            ui.label(RichText::new(format!("installed: {}", s.manifest.version)).color(TEXT_WEAK));
                        }
                        if !e.homepage.is_empty() {
                            ui.hyperlink_to("Homepage", &e.homepage);
                        }
                    });
                });
            }
        }
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
    }

    fn sdk_act(&mut self, act: Act) {
        match act {
            Act::Tab(t) => {
                self.sdk_manager.tab = t;
                if t == Tab::Available && self.sdk_manager.registries.is_empty() && self.sdk_manager.fetching.is_none() {
                    self.fetch_registries();
                }
            }
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
            Act::Create => self.create_sdk(),
        }
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

    fn poll_sdk_jobs(&mut self, ctx: &egui::Context) {
        let m = &mut self.sdk_manager;
        let mut busy = false;
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
                m.registries = result.unwrap_or_else(|e| vec![(String::new(), Err(e))]);
                m.fetching = None;
            }
            None => {}
        }
        let finished = m.download.as_ref().and_then(|d| Some((d.entry.clone(), d.job.poll()?.and_then(|r| r))));
        busy |= m.download.is_some() && finished.is_none();
        if let Some((entry, result)) = finished {
            m.download = None;
            m.note = Some(match &result {
                Ok(sdk) => (true, format!("Installed {} {} into {}", sdk.manifest.name, sdk.manifest.version, sdk.dir.display())),
                Err(e) => (false, format!("{} was not installed: {e}", entry.name)),
            });
            if result.is_ok() {
                self.sdk_changed(&entry.id);
            }
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
                ui.heading(format!("Install {} {}?", e.name, e.version));
                egui::Grid::new("sdk-install-grid").num_columns(2).show(ui, |ui| {
                    let plugins = if e.plugins.is_empty() { "none".to_string() } else { e.plugins.join(", ") };
                    let rows = [
                        ("Author", if e.author.is_empty() { "unknown" } else { &e.author }),
                        ("License", if e.license.is_empty() { "not given" } else { &e.license }),
                        ("Plugins", &plugins),
                        ("Size", &format!("{} KiB", e.size.div_ceil(1024))),
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
