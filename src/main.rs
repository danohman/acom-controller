#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod config;
mod link;
mod protocol;
mod radio;
mod sound;
mod tray;
mod sim;
mod state;
mod icon;
mod theme;
mod update;
mod widgets;

fn main() -> eframe::Result {
    // Default size for first run; later runs restore the saved window geometry.
    let (size, min) = app::layout_sizes(config::Config::load().compact);
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("ACOM Controller")
            .with_app_id("acom-controller")
            .with_icon(icon::window_icon())
            .with_inner_size(size)
            .with_min_inner_size(min),
        // Remember window size, position and monitor between runs.
        persist_window: true,
        ..Default::default()
    };
    eframe::run_native("ACOM Controller", options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}
