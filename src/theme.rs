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
        bg: Color32::from_rgb(12, 14, 17),
        fg: Color32::from_rgb(206, 206, 207),
        dim: Color32::from_rgb(126, 138, 144),
        surface: Color32::from_rgb(22, 25, 29),
        accent: Color32::from_rgb(127, 226, 241),
        accent2: Color32::from_rgb(116, 162, 207),
        danger: Color32::from_rgb(255, 107, 138),
        ok: Color32::from_rgb(115, 172, 143),
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
            // Dark theme colors 9/2 are often too dim for text. Mix them
            // toward the foreground so chips and labels stay readable.
            if let Some(s) = v["colors"]["color9"].as_str() {
                let raw = hex(s, t.accent2);
                let lift = v["colors"]["color12"]
                    .as_str()
                    .map(|c| hex(c, t.accent))
                    .unwrap_or(t.fg);
                t.accent2 = mix(raw, lift, 0.52);
            }
            if let Some(s) = v["colors"]["color3"].as_str() {
                t.dim = mix(hex(s, t.dim), t.fg, 0.32);
            }
            if let Some(s) = v["colors"]["color2"].as_str() {
                let sage = Color32::from_rgb(139, 213, 168);
                t.ok = mix(hex(s, t.ok), sage, 0.68);
            }
            t.surface = mix(t.bg, t.fg, 0.07);
        }
    }
    t
}

pub fn install_fonts(ctx: &egui::Context) {
    let theme = load_theme();
    let mut defs = FontDefinitions::default();
    let regular = "/usr/share/fonts/TTF/JetBrainsMonoNerdFont-Regular.ttf";
    let bold = "/usr/share/fonts/TTF/JetBrainsMonoNerdFont-Bold.ttf";
    let mut has_regular = false;
    if let Ok(bytes) = fs::read(regular) {
        defs.font_data
            .insert("jbmono".into(), Arc::new(FontData::from_owned(bytes)));
        for fam in [FontFamily::Monospace, FontFamily::Proportional] {
            if let Some(list) = defs.families.get_mut(&fam) {
                list.insert(0, "jbmono".into());
            }
        }
        has_regular = true;
    }
    if let Ok(bytes) = fs::read(bold) {
        defs.font_data
            .insert("jbmono-bold".into(), Arc::new(FontData::from_owned(bytes)));
        let mut family = vec!["jbmono-bold".into()];
        if has_regular {
            family.push("jbmono".into());
        }
        defs.families
            .insert(FontFamily::Name("bold".into()), family);
    }
    let bold_family = FontFamily::Name("bold".into());
    if !defs.families.contains_key(&bold_family) {
        if let Some(fallback) = defs.families.get(&FontFamily::Monospace).cloned() {
            defs.families.insert(bold_family, fallback);
        }
    }
    ctx.set_fonts(defs);

    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    style.text_styles = [
        (
            egui::TextStyle::Small,
            FontId::new(11.5, FontFamily::Monospace),
        ),
        (
            egui::TextStyle::Body,
            FontId::new(14.0, FontFamily::Monospace),
        ),
        (
            egui::TextStyle::Button,
            FontId::new(12.5, FontFamily::Monospace),
        ),
        (
            egui::TextStyle::Heading,
            FontId::new(16.0, FontFamily::Monospace),
        ),
        (
            egui::TextStyle::Monospace,
            FontId::new(12.5, FontFamily::Monospace),
        ),
    ]
    .into();
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(10.0, 6.0);
    style.spacing.window_margin = egui::Margin::same(16);
    style.spacing.menu_margin = egui::Margin::same(8);
    style.spacing.scroll.bar_width = 3.0;
    style.spacing.scroll.bar_inner_margin = 0.0;
    style.spacing.scroll.bar_outer_margin = 1.0;
    style.spacing.scroll.dormant_handle_opacity = 0.18;
    style.spacing.scroll.active_handle_opacity = 0.45;
    style.spacing.scroll.interact_handle_opacity = 0.55;
    style.animation_time = 0.16;

    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = Color32::TRANSPARENT;
    style.visuals.window_fill = mix(theme.surface, theme.fg, 0.035);
    style.visuals.window_stroke = Stroke::new(1.0, rgba(theme.accent, 0.18));
    style.visuals.window_corner_radius = egui::CornerRadius::same(11);
    style.visuals.window_shadow = egui::epaint::Shadow {
        offset: [0, 14],
        blur: 36,
        spread: 0,
        color: Color32::from_black_alpha(150),
    };
    style.visuals.popup_shadow = egui::epaint::Shadow {
        offset: [0, 8],
        blur: 22,
        spread: 0,
        color: Color32::from_black_alpha(130),
    };
    style.visuals.menu_corner_radius = egui::CornerRadius::same(8);
    style.visuals.override_text_color = Some(theme.fg);
    style.visuals.weak_text_alpha = 0.55;
    style.visuals.extreme_bg_color = mix(theme.bg, theme.fg, 0.035);
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::NONE;
    style.visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, theme.fg);
    style.visuals.widgets.inactive.bg_fill = Color32::TRANSPARENT;
    style.visuals.widgets.inactive.weak_bg_fill = rgba(theme.fg, 0.05);
    style.visuals.widgets.inactive.bg_stroke = Stroke::NONE;
    style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(6);
    style.visuals.widgets.hovered.weak_bg_fill = rgba(theme.accent, 0.09);
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, rgba(theme.accent, 0.15));
    style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(6);
    style.visuals.widgets.active.weak_bg_fill = rgba(theme.accent, 0.16);
    style.visuals.widgets.active.corner_radius = egui::CornerRadius::same(6);
    style.visuals.selection.bg_fill = rgba(theme.accent, 0.22);
    style.visuals.selection.stroke = Stroke::new(1.0, rgba(theme.accent, 0.60));
    style.visuals.hyperlink_color = theme.accent;
    style.visuals.text_edit_bg_color = Some(Color32::TRANSPARENT);
    ctx.set_style_of(egui::Theme::Dark, style);
    ctx.set_theme(egui::Theme::Dark);
}
