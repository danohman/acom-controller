//! Application icon, derived from `assets/icon.png` at build time (see build.rs).

use eframe::egui;

const PNG: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon256.png"));

fn decode() -> image::RgbaImage {
    image::load_from_memory(PNG).expect("embedded icon").to_rgba8()
}

pub fn window_icon() -> egui::IconData {
    let img = decode();
    egui::IconData { width: img.width(), height: img.height(), rgba: img.into_raw() }
}

pub fn texture(ctx: &egui::Context) -> egui::TextureHandle {
    let img = decode();
    let size = [img.width() as usize, img.height() as usize];
    ctx.load_texture("logo", egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()), egui::TextureOptions::LINEAR)
}
