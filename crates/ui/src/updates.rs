//! Update checks at launch and from Help > Check for Updates: MapCraft from its GitHub releases,
//! and the user's installed SDKs from their registries, offered in a bar under the toolbar.

use std::sync::atomic::AtomicBool;
use std::time::Duration;

use egui::RichText;
use mapcraft_sdk_manager::update::{self, Release};
use mapcraft_sdk_manager::{Entry, MAPCRAFT_VERSION};

use crate::app::MapCraftApp;
use crate::chrome::{ACCENT, TEXT_WEAK};
use crate::sdk_manager::Job;

/// Steam and branded builds update through their own channel, never to plain MapCraft.
const SELF_UPDATE: bool = cfg!(not(feature = "steam")) && option_env!("MAPCRAFT_BRAND").is_none();

#[derive(Default)]
pub(crate) struct Updates {
    checking: Option<Job<Result<Release, String>>>,
    /// Help > Check for Updates asked, so "up to date" is reported too.
    manual: bool,
    /// Waiting for the registries to offer SDK updates.
    want_sdks: bool,
    release: Option<Release>,
    sdks: Vec<(String, Entry)>,
    installing: Option<Job<Result<(), String>>>,
    installed: bool,
    /// Start the new copy when the editor closes.
    pub relaunch: bool,
    error: Option<String>,
    hidden: bool,
}

enum Act {
    Install,
    Restart,
    UpdateSdks,
    Hide,
}

impl MapCraftApp {
    pub(crate) fn check_updates(&mut self, manual: bool) {
        let u = &mut self.updates;
        if u.checking.is_some() || u.installing.is_some() {
            return;
        }
        u.manual = manual;
        u.hidden = false;
        u.error = None;
        u.want_sdks = true;
        if SELF_UPDATE && !u.installed {
            u.checking = Some(Job::spawn(|| update::latest_release(update::LATEST_RELEASE).map_err(|e| e.to_string())));
        }
        if manual {
            self.sdk_manager.registries_changed();
        }
        if !self.registries_requested() {
            self.fetch_registries();
        }
    }

    fn poll_updates(&mut self, ctx: &egui::Context) {
        let u = &mut self.updates;
        let mut busy = false;
        match u.checking.as_ref().map(Job::poll) {
            Some(None) => busy = true,
            Some(Some(result)) => {
                u.checking = None;
                match result.and_then(|r| r) {
                    Ok(r) if r.is_newer() => u.release = Some(r),
                    Ok(_) => {}
                    Err(e) if u.manual => u.error = Some(format!("Could not check for MapCraft updates: {e}")),
                    Err(e) => log::warn!("update check: {e}"),
                }
            }
            None => {}
        }
        match u.installing.as_ref().map(Job::poll) {
            Some(None) => busy = true,
            Some(Some(result)) => {
                u.installing = None;
                match result.and_then(|r| r) {
                    Ok(()) => u.installed = true,
                    Err(e) => u.error = Some(format!("MapCraft was not updated: {e}")),
                }
            }
            None => {}
        }
        if self.updates.want_sdks
            && let Some(sdks) = self.sdk_updates()
        {
            let u = &mut self.updates;
            u.want_sdks = false;
            u.sdks = sdks;
        }
        let u = &self.updates;
        if u.manual && u.checking.is_none() && !u.want_sdks {
            if u.release.is_none() && u.sdks.is_empty() && u.error.is_none() {
                self.status = format!("MapCraft {MAPCRAFT_VERSION} and its SDKs are up to date");
            }
            self.updates.manual = false;
        }
        if busy || self.updates.want_sdks {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    /// The update bar under the toolbar, while there is something to offer.
    pub(crate) fn update_bar(&mut self, ui: &mut egui::Ui) {
        self.poll_updates(ui.ctx());
        let u = &self.updates;
        let downloading = self.sdk_downloading();
        if u.hidden || (u.release.is_none() && u.sdks.is_empty() && u.error.is_none() && downloading.is_none()) {
            return;
        }
        let mut act = None;
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Updates").strong().color(ACCENT));
            if let Some(r) = &u.release {
                let version = r.version();
                if u.installed {
                    ui.label(format!("MapCraft {version} is installed."));
                    if ui.add(crate::icons::button(Some("arrows_clockwise"), "Restart")).clicked() {
                        act = Some(Act::Restart);
                    }
                } else if u.installing.is_some() {
                    ui.spinner();
                    ui.label(format!("Downloading MapCraft {version}…"));
                } else {
                    ui.label(format!("MapCraft {version} is available."));
                    if update::target().is_some() {
                        if ui.add(crate::icons::button(Some("cloud_arrow_down"), "Install and restart")).clicked() {
                            act = Some(Act::Install);
                        }
                    } else {
                        ui.hyperlink_to("Download", &r.html_url);
                    }
                    ui.hyperlink_to("What's new", &r.html_url);
                }
            }
            if let Some(name) = downloading {
                ui.spinner();
                ui.label(format!("Updating {name}…"));
            } else if !u.sdks.is_empty() {
                let names: Vec<String> = u.sdks.iter().map(|(_, e)| format!("{} {}", e.name, e.version)).collect();
                ui.label(format!("SDK updates: {}.", names.join(", ")));
                if ui.add(crate::icons::button(Some("cloud_arrow_down"), "Update SDKs")).clicked() {
                    act = Some(Act::UpdateSdks);
                }
            }
            if let Some(e) = &u.error {
                ui.label(RichText::new(e).color(TEXT_WEAK));
            }
            if ui.button("Not now").clicked() {
                act = Some(Act::Hide);
            }
        });
        ui.add_space(2.0);
        match act {
            Some(Act::Install) => self.install_update(),
            Some(Act::Restart) => {
                self.updates.relaunch = true;
                self.quit_requested = true;
            }
            Some(Act::UpdateSdks) => {
                let sdks = std::mem::take(&mut self.updates.sdks);
                self.update_sdks(sdks);
            }
            Some(Act::Hide) => self.updates.hidden = true,
            None => {}
        }
    }

    fn install_update(&mut self) {
        let (Some(release), Some(target)) = (self.updates.release.clone(), update::target()) else { return };
        self.updates.error = None;
        self.updates.installing =
            Some(Job::spawn(move || update::install(&release, &target, &mut |_, _| {}, &AtomicBool::new(false)).map_err(|e| e.to_string())));
    }
}

/// Clears what the last update left behind; called once at launch.
pub(crate) fn clean() {
    if SELF_UPDATE && let Some(t) = update::target() {
        update::clean(&t);
    }
}

/// Starts the updated copy as the editor exits.
pub(crate) fn relaunch() {
    let Some(exe) = update::target().map(|t| update::executable(&t)) else { return };
    if let Err(e) = std::process::Command::new(&exe).spawn() {
        log::warn!("restart {}: {e}", exe.display());
    }
}
