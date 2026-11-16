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

/// Load an embedded PNG as a texture, trimming fully transparent margins.
pub fn image_texture(ctx: &egui::Context, name: &str, png: &[u8]) -> egui::TextureHandle {
    let mut img = image::load_from_memory(png).expect("embedded image").to_rgba8();
    let (w, h) = img.dimensions();
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for (x, y, p) in img.enumerate_pixels() {
        if p[3] > 8 {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    if x1 >= x0 && y1 >= y0 {
        img = image::imageops::crop_imm(&img, x0, y0, x1 - x0 + 1, y1 - y0 + 1).to_image();
    }
    let size = [img.width() as usize, img.height() as usize];
    ctx.load_texture(name, egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()), egui::TextureOptions::LINEAR)
}
