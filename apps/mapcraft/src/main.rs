//! MapCraft level editor.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::path::PathBuf;

use mapcraft_ui::{MapCraftApp, Options};

const ICON: &[u8] = include_bytes!("../assets/mapcraft-256.png");

fn main() -> eframe::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let mut options = Options::default();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--screenshot" => options.screenshot = args.next().map(PathBuf::from),
            _ => options.open = Some(PathBuf::from(a)),
        }
    }
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("MapCraft")
        .with_inner_size([1600.0, 1000.0])
        .with_min_inner_size([800.0, 500.0])
        .with_app_id("mapcraft")
        // A screenshot run must not take the user's keyboard focus.
        .with_active(options.screenshot.is_none());
    match eframe::icon_data::from_png_bytes(ICON) {
        Ok(icon) => viewport = viewport.with_icon(icon),
        Err(e) => log::warn!("window icon: {e}"),
    }
    let native = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("MapCraft", native, Box::new(move |cc| Ok(Box::new(MapCraftApp::new(cc, options)))))
}

#[cfg(test)]
mod tests {
    #[test]
    fn icon_decodes() {
        assert!(eframe::icon_data::from_png_bytes(super::ICON).is_ok());
    }
}
