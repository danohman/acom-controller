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
    if !dark {
        // Black text so the light theme is easy to read.
        v.override_text_color = Some(Color32::BLACK);
        // Soft blue-grey instead of stark white; the bar tracks sit a step darker so they read.
        v.panel_fill = Color32::from_rgb(224, 229, 236);
        v.window_fill = Color32::from_rgb(238, 241, 246);
        v.extreme_bg_color = Color32::from_rgb(200, 207, 218);
        v.faint_bg_color = Color32::from_rgb(214, 220, 229);
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(170, 179, 193));
        v.widgets.inactive.weak_bg_fill = Color32::from_rgb(238, 241, 246);
        v.widgets.inactive.bg_fill = Color32::from_rgb(238, 241, 246);
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

/// Read the first font file found under any system font directory.
fn load_system_font(files: &[&str]) -> Option<Vec<u8>> {
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    #[cfg(windows)]
    {
        if let Some(w) = std::env::var_os("WINDIR") {
            dirs.push(std::path::PathBuf::from(w).join("Fonts"));
        }
        if let Some(l) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(std::path::PathBuf::from(l).join("Microsoft").join("Windows").join("Fonts"));
        }
    }
    files.iter().find_map(|f| dirs.iter().find_map(|d| std::fs::read(d.join(f)).ok()))
}

/// Prefer Segoe UI (UI text, bold headings) and Cascadia Mono / Consolas (monospace) when the
/// system has them (Windows). They cannot be embedded, so on other platforms, or if a file is
/// missing, egui's bundled fonts are used. The bundled fonts stay as fallbacks for symbols.
pub fn install_fonts(ctx: &egui::Context) {
    use egui::{FontData, FontFamily, FontId, TextStyle};
    let mut fonts = egui::FontDefinitions::default();
    let mut add = |name: &str, files: &[&str], family: FontFamily| -> bool {
        let Some(bytes) = load_system_font(files) else { return false };
        fonts.font_data.insert(name.to_owned(), std::sync::Arc::new(FontData::from_owned(bytes)));
        fonts.families.entry(family).or_default().insert(0, name.to_owned());
        true
    };
    add("ui-regular", &["segoeui.ttf"], FontFamily::Proportional);
    add("mono-regular", &["CascadiaMono.ttf", "consola.ttf"], FontFamily::Monospace);
    let bold = add("ui-bold", &["segoeuib.ttf"], FontFamily::Name("bold".into()));
    if bold {
        // Symbols and non-Latin glyphs still come from the bundled fonts.
        let fallback = fonts.families[&FontFamily::Proportional].clone();
        let list = fonts.families.get_mut(&FontFamily::Name("bold".into())).unwrap();
        list.extend(fallback.into_iter().filter(|n| n != "ui-regular"));
    }
    ctx.set_fonts(fonts);
    if bold {
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            ctx.style_mut_of(theme, |s| {
                let size = s.text_styles.get(&TextStyle::Heading).map_or(20.0, |f| f.size);
                s.text_styles.insert(TextStyle::Heading, FontId::new(size, FontFamily::Name("bold".into())));
            });
        }
    }
}
