//! The MapCraft editor interface: menus, panels and the four viewports.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod app;
mod chrome;
mod dialogs;
mod panels;
mod plugins;
mod prefs;
mod project;
mod shortcuts;
mod thumbs;
mod viewport;

pub use app::{MapCraftApp, Options};
