//! Visual theme.

use eframe::egui::{self, Color32, CornerRadius, Stroke};

pub fn apply(ctx: &egui::Context, dark: bool, scale: f32) {
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    if dark {
        v.panel_fill = Color32::from_rgb(16, 18, 23);
        v.window_fill = Color32::from_rgb(24, 27, 34);
        v.extreme_bg_color = Color32::from_rgb(10, 12, 16);
        v.faint_bg_color = Color32::from_rgb(22, 25, 31);
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(40, 45, 56));
    }
    let r = CornerRadius::same(7);
    v.widgets.noninteractive.corner_radius = r;
    v.widgets.inactive.corner_radius = r;
    v.widgets.hovered.corner_radius = r;
    v.widgets.active.corner_radius = r;
    v.selection.bg_fill = Color32::from_rgb(52, 108, 214);
    ctx.set_visuals(v);
    ctx.set_pixels_per_point(scale.clamp(0.8, 2.0));
}
