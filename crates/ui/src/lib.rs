//! The MapCraft editor interface: menus, panels and the four viewports.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod app;
mod assets;
mod bake;
mod chrome;
mod context_menu;
mod control;
mod curves;
mod dialogs;
mod drop;
mod icons;
mod inspect;
mod keys;
mod measure;
mod models;
mod panels;
mod plugins;
mod prefs;
mod project;
mod sdk_manager;
mod shader_editor;
mod shortcuts;
mod thumbs;
mod transform;
mod uv;
mod vertex;
mod viewport;
#[cfg(feature = "steam")]
mod workshop;

/// Stand-ins for builds without Steam.
#[cfg(not(feature = "steam"))]
mod workshop {
    use std::path::PathBuf;

    pub(crate) struct Workshop;

    impl Workshop {
        pub(crate) fn new() -> Self {
            Workshop
        }

        pub(crate) fn app_dir(&self, _app: u32) -> Option<PathBuf> {
            None
        }
    }

    impl crate::app::MapCraftApp {
        pub(crate) fn workshop_menu(&mut self, _ui: &mut egui::Ui) {}

        pub(crate) fn open_workshop(&mut self) {}

        pub(crate) fn workshop_window(&mut self, _ctx: &egui::Context) {}
    }
}

pub use app::{MapCraftApp, Options};
pub use mapcraft_sdk_manager::APP_ID;

/// The name in the title bar; a branded build sets `MAPCRAFT_BRAND`.
pub const BRAND: &str = match option_env!("MAPCRAFT_BRAND") {
    Some(name) => name,
    None => "MapCraft",
};
