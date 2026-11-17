//! Custom egui widgets: horizontal meters, status chips, section cards.

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Vec2};

pub const GREEN: Color32 = Color32::from_rgb(62, 207, 142);
pub const AMBER: Color32 = Color32::from_rgb(255, 184, 77);
pub const RED: Color32 = Color32::from_rgb(255, 92, 92);
pub const BLUE: Color32 = Color32::from_rgb(91, 156, 255);
pub const MUTED: Color32 = Color32::from_rgb(140, 148, 163);

/// Colour zones for a meter: below `warn` is green, below `crit` amber, else red.
#[derive(Clone, Copy)]
pub struct Zones {
    pub warn: f32,
    pub crit: f32,
}

impl Zones {
    pub const NONE: Zones = Zones { warn: f32::INFINITY, crit: f32::INFINITY };
}

pub struct Meter<'a> {
    pub label: &'a str,
    pub value: f32,
    pub min: f32,
    pub max: f32,
    pub text: String,
    pub zones: Zones,
    pub peak: Option<f32>,
    pub height: f32,
}

impl Meter<'_> {
    pub fn show(self, ui: &mut egui::Ui) -> egui::Response {
        let width = ui.available_width();
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, self.height), Sense::click());
        let p = ui.painter_at(rect);
        let vis = ui.visuals();
        let track = vis.extreme_bg_color;
        let fg = vis.text_color();
        let r = CornerRadius::same(5);

        // Label and value share one text row (vertically centred), with the bar below it.
        let row_y = rect.top() + 9.0;
        p.text(Pos2::new(rect.left() + 2.0, row_y), Align2::LEFT_CENTER, self.label, FontId::proportional(12.0), vis.strong_text_color());
        p.text(Pos2::new(rect.right() - 2.0, row_y), Align2::RIGHT_CENTER, &self.text, FontId::monospace(14.0), vis.strong_text_color());
        let bar = Rect::from_min_max(
            Pos2::new(rect.left(), rect.bottom() - 14.0),
            Pos2::new(rect.right(), rect.bottom() - 2.0),
        );
        let frac = |v: f32| ((v - self.min) / (self.max - self.min)).clamp(0.0, 1.0);
        // Rises are instant; falls decay smoothly (about a quarter second to settle).
        let id = ui.id().with(("meter", self.label));
        let target = frac(self.value);
        let shown = ui.ctx().data(|d| d.get_temp::<f32>(id)).unwrap_or(target);
        let f = if target >= shown {
            target
        } else {
            let dt = ui.input(|i| i.stable_dt).min(0.1);
            let next = target + (shown - target) * (-dt / 0.06).exp();
            if (next - target).abs() > 0.002 {
                ui.ctx().request_repaint();
                next
            } else {
                target
            }
        };
        ui.ctx().data_mut(|d| d.insert_temp(id, f));
        let x_at = |fr: f32| bar.left() + fr * bar.width();
        if self.zones.warn.is_infinite() {
            p.rect_filled(bar, r, track);
            if f > 0.0 {
                let fill = Rect::from_min_max(bar.min, Pos2::new(x_at(f).max(bar.left() + 10.0), bar.max.y));
                p.rect_filled(fill, r, GREEN);
            }
        } else {
            // Segmented scale: dim green/amber/red zones with a bright fill up to the value.
            let w = frac(self.zones.warn);
            let c = frac(self.zones.crit);
            let segs = [(0.0, w, GREEN), (w, c, AMBER), (c, 1.0, RED)];
            for (i, (a, b, col)) in segs.into_iter().enumerate() {
                if b <= a {
                    continue;
                }
                let seg_r = CornerRadius {
                    nw: if a == 0.0 { 3 } else { 0 },
                    sw: if a == 0.0 { 3 } else { 0 },
                    ne: if b >= 1.0 { 3 } else { 0 },
                    se: if b >= 1.0 { 3 } else { 0 },
                };
                let seg = Rect::from_min_max(Pos2::new(x_at(a), bar.top()), Pos2::new(x_at(b), bar.bottom()));
                let dim = if vis.dark_mode { 0.22 } else { 0.10 };
                p.rect_filled(seg, seg_r, track.lerp_to_gamma(col, dim));
                if f > a {
                    let part = Rect::from_min_max(seg.min, Pos2::new(x_at(f.min(b)), bar.bottom()));
                    p.rect_filled(part, seg_r, col);
                }
                if i > 0 {
                    p.line_segment(
                        [Pos2::new(x_at(a), bar.top()), Pos2::new(x_at(a), bar.bottom())],
                        egui::Stroke::new(1.0, vis.panel_fill),
                    );
                }
            }
        }
        if let Some(pk) = self.peak {
            let x = bar.left() + frac(pk) * bar.width();
            p.rect_filled(
                Rect::from_min_max(Pos2::new(x - 1.0, bar.top() - 1.0), Pos2::new(x + 1.0, bar.bottom() + 1.0)),
                CornerRadius::ZERO,
                fg,
            );
        }
        resp
    }
}

/// Small square icon button with the glyph optically centred.
pub fn icon_button(ui: &mut egui::Ui, glyph: &str, tip: &str) -> egui::Response {
    let size = Vec2::new(ui.spacing().interact_size.y + 8.0, ui.spacing().interact_size.y);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let style = ui.style().interact(&resp);
    ui.painter().rect(rect, style.corner_radius, style.weak_bg_fill, style.bg_stroke, egui::StrokeKind::Inside);
    // Symbol glyphs sit high in their line box; nudge down to centre the ink.
    let galley = ui.painter().layout_no_wrap(glyph.to_owned(), FontId::proportional(17.0), style.fg_stroke.color);
    ui.painter().galley(rect.center() - galley.size() / 2.0 + Vec2::new(0.0, 1.5), galley, style.fg_stroke.color);
    resp.on_hover_text(tip)
}

/// Rounded status pill.
pub fn chip(ui: &mut egui::Ui, text: &str, color: Color32) {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(13.0), color);
    let size = galley.size() + Vec2::new(18.0, 8.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(255), color.gamma_multiply(0.18));
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
}

/// Bright solid-red "transmitting" pill; the fill is theme-independent, the text flips with the theme.
pub fn tx_chip(ui: &mut egui::Ui, text: &str) {
    const FILL: Color32 = Color32::from_rgb(255, 30, 30);
    // White text in dark mode, black in light mode.
    let fg = if ui.visuals().dark_mode { Color32::WHITE } else { Color32::BLACK };
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(13.0), fg);
    let size = galley.size() + Vec2::new(18.0, 8.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(255), FILL);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, fg);
    // Faux-bold: draw the text a second time half a pixel over.
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(13.0), fg);
    ui.painter().galley(rect.center() - galley.size() / 2.0 + Vec2::new(0.5, 0.0), galley, fg);
}

/// Titled card container.
pub fn card<R>(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::group(ui.style())
        .corner_radius(CornerRadius::same(10))
        .inner_margin(12.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(title.to_uppercase()).size(11.5).color(MUTED).strong());
            ui.add_space(4.0);
            add(ui)
        })
        .inner
}

/// Large read-out: caption above a big number.
pub fn readout(ui: &mut egui::Ui, caption: &str, value: &str, color: Color32) {
    ui.vertical(|ui| {
        ui.label(egui::RichText::new(caption).size(11.5).color(MUTED));
        ui.label(egui::RichText::new(value).size(22.0).monospace().color(color));
    });
}
