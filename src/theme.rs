use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId, Stroke};
use std::fs;
use std::sync::Arc;

use crate::model::home;

pub struct Theme {
    pub bg: Color32,
    pub fg: Color32,
    pub dim: Color32,
    pub surface: Color32,
    pub accent: Color32,
    pub accent2: Color32,
    pub danger: Color32,
    pub ok: Color32,
}

pub fn hex(s: &str, fallback: Color32) -> Color32 {
    let h = s.trim().trim_start_matches('#');
    if h.len() < 6 {
        return fallback;
    }
    let r = u8::from_str_radix(&h[0..2], 16).ok();
    let g = u8::from_str_radix(&h[2..4], 16).ok();
    let b = u8::from_str_radix(&h[4..6], 16).ok();
    match (r, g, b) {
        (Some(r), Some(g), Some(b)) => Color32::from_rgb(r, g, b),
        _ => fallback,
    }
}

pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

pub fn rgba(c: Color32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a.clamp(0.0, 1.0) * 255.0) as u8)
}

pub fn load_theme() -> Theme {
    let mut t = Theme {
        bg: Color32::from_rgb(13, 9, 28),
        fg: Color32::from_rgb(206, 205, 209),
        dim: Color32::from_rgb(130, 128, 140),
        surface: Color32::from_rgb(24, 19, 42),
        accent: Color32::from_rgb(249, 145, 197),
        accent2: Color32::from_rgb(193, 102, 213),
        danger: Color32::from_rgb(255, 107, 138),
        ok: Color32::from_rgb(139, 213, 168),
    };
    let path = home().join(".cache/coloreclipse/colors.json");
    if let Ok(raw) = fs::read_to_string(path) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            let special = &v["special"];
            if let Some(s) = special["background"].as_str() {
                t.bg = hex(s, t.bg);
            }
            if let Some(s) = special["foreground"].as_str() {
                t.fg = hex(s, t.fg);
            }
            if let Some(s) = v["colors"]["color5"].as_str() {
                t.accent = hex(s, t.accent);
            }
            if let Some(s) = v["colors"]["color9"].as_str() {
                t.accent2 = hex(s, t.accent2);
            }
            if let Some(s) = v["colors"]["color3"].as_str() {
                t.dim = mix(hex(s, t.fg), t.bg, 0.45);
            }
            if let Some(s) = v["colors"]["color2"].as_str() {
                t.ok = hex(s, t.ok);
            }
            t.surface = mix(t.bg, t.fg, 0.055);
        }
    }
    t
}

pub fn install_fonts(ctx: &egui::Context) {
    let mut defs = FontDefinitions::default();
    let regular = "/usr/share/fonts/TTF/JetBrainsMonoNerdFont-Regular.ttf";
    let bold = "/usr/share/fonts/TTF/JetBrainsMonoNerdFont-Bold.ttf";
    if let Ok(bytes) = fs::read(regular) {
        defs.font_data
            .insert("jbmono".into(), Arc::new(FontData::from_owned(bytes)));
        for fam in [FontFamily::Monospace, FontFamily::Proportional] {
            if let Some(list) = defs.families.get_mut(&fam) {
                list.insert(0, "jbmono".into());
            }
        }
    }
    if let Ok(bytes) = fs::read(bold) {
        defs.font_data
            .insert("jbmono-bold".into(), Arc::new(FontData::from_owned(bytes)));
        defs.families
            .insert(FontFamily::Name("bold".into()), vec!["jbmono-bold".into()]);
    }
    ctx.set_fonts(defs);

    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    style.text_styles = [
        (
            egui::TextStyle::Small,
            FontId::new(12.0, FontFamily::Monospace),
        ),
        (
            egui::TextStyle::Body,
            FontId::new(15.0, FontFamily::Monospace),
        ),
        (
            egui::TextStyle::Button,
            FontId::new(13.0, FontFamily::Monospace),
        ),
        (
            egui::TextStyle::Heading,
            FontId::new(16.0, FontFamily::Monospace),
        ),
        (
            egui::TextStyle::Monospace,
            FontId::new(13.0, FontFamily::Monospace),
        ),
    ]
    .into();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = Color32::TRANSPARENT;
    style.visuals.window_fill = Color32::from_rgb(18, 13, 33);
    style.visuals.window_stroke =
        Stroke::new(1.0, Color32::from_rgba_unmultiplied(249, 145, 197, 40));
    style.visuals.override_text_color = Some(Color32::from_rgb(206, 205, 209));
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::NONE;
    style.visuals.widgets.inactive.bg_fill = Color32::TRANSPARENT;
    style.visuals.widgets.inactive.weak_bg_fill =
        Color32::from_rgba_unmultiplied(206, 205, 209, 10);
    style.visuals.widgets.inactive.bg_stroke = Stroke::NONE;
    style.visuals.widgets.hovered.weak_bg_fill = Color32::from_rgba_unmultiplied(249, 145, 197, 25);
    style.visuals.widgets.active.weak_bg_fill = Color32::from_rgba_unmultiplied(249, 145, 197, 35);
    style.visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(249, 145, 197, 60);
    ctx.set_style_of(egui::Theme::Dark, style);
    ctx.set_theme(egui::Theme::Dark);
}
