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
mod shortcuts;
mod thumbs;
mod transform;
mod uv;
mod vertex;
mod viewport;

pub use app::{MapCraftApp, Options};
pub use mapcraft_sdk_manager::APP_ID;

/// The name in the title bar; a branded build sets `MAPCRAFT_BRAND`.
pub const BRAND: &str = match option_env!("MAPCRAFT_BRAND") {
    Some(name) => name,
    None => "MapCraft",
};
