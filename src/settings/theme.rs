//! System wizualny okna ustawień: paleta, typografia (Segoe UI + ikony Fluent), odstępy.

use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Vec2,
};

/// Paleta – jedno źródło prawdy dla kolorów interfejsu.
pub mod pal {
    use eframe::egui::Color32;
    pub const BG: Color32 = Color32::from_rgb(0x0E, 0x10, 0x14);
    pub const SIDEBAR: Color32 = Color32::from_rgb(0x12, 0x15, 0x1A);
    pub const CARD: Color32 = Color32::from_rgb(0x17, 0x1B, 0x22);
    pub const CARD_HI: Color32 = Color32::from_rgb(0x1E, 0x23, 0x2C);
    pub const CONTROL: Color32 = Color32::from_rgb(0x22, 0x28, 0x32);
    pub const CONTROL_HI: Color32 = Color32::from_rgb(0x2B, 0x32, 0x3E);
    pub const BORDER: Color32 = Color32::from_rgb(0x25, 0x2A, 0x34);
    pub const DIVIDER: Color32 = Color32::from_rgb(0x20, 0x25, 0x2E);
    pub const TEXT: Color32 = Color32::from_rgb(0xE8, 0xEB, 0xF0);
    pub const MUTED: Color32 = Color32::from_rgb(0x8A, 0x93, 0xA3);
    /// Tylko elementy nietekstowe (obrysy, kropki) — kontrast ≥ 3:1. Do tekstu używaj `MUTED`.
    pub const FAINT: Color32 = Color32::from_rgb(0x6E, 0x77, 0x87);
    pub const ACCENT: Color32 = Color32::from_rgb(0x2D, 0xD4, 0xBF);
    pub const ACCENT_DIM: Color32 = Color32::from_rgb(0x1A, 0x8C, 0x7F);
    pub const ON_ACCENT: Color32 = Color32::from_rgb(0x04, 0x1F, 0x1B);
    pub const GOOD: Color32 = Color32::from_rgb(0x4A, 0xDE, 0x80);
    pub const WARN: Color32 = Color32::from_rgb(0xFA, 0xCC, 0x15);
    pub const BAD: Color32 = Color32::from_rgb(0xF8, 0x71, 0x71);
}

pub const ICONS: &str = "icons";
pub const SEMIBOLD: &str = "semibold";

/// Ikony z czcionki Segoe Fluent Icons / MDL2 Assets.
pub mod ico {
    pub const GENERAL: &str = "\u{E713}";
    pub const APPEARANCE: &str = "\u{E790}";
    pub const SIMPLE: &str = "\u{EC4A}";
    pub const DETAILED: &str = "\u{E9D9}";
    pub const DISPLAYS: &str = "\u{E7F4}";
    pub const THRESHOLDS: &str = "\u{E7BA}";
    pub const HOTKEYS: &str = "\u{E765}";
    pub const ABOUT: &str = "\u{E946}";
    pub const UP: &str = "\u{E70E}";
    pub const DOWN: &str = "\u{E70D}";
    pub const FOLDER: &str = "\u{E838}";
    pub const RESET: &str = "\u{E777}";
    pub const PLAY: &str = "\u{E768}";
    pub const STOP: &str = "\u{E71A}";
    pub const CHECK: &str = "\u{E73E}";
    pub const FEEDBACK: &str = "\u{E715}";
}

fn read_font(names: &[&str]) -> Option<Vec<u8>> {
    let dir = std::env::var_os("WINDIR").map(std::path::PathBuf::from).unwrap_or_else(|| "C:\\Windows".into()).join("Fonts");
    names.iter().find_map(|n| std::fs::read(dir.join(n)).ok())
}

pub fn install(ctx: &egui::Context) -> bool {
    let mut fonts = FontDefinitions::default();
    let mut has_icons = false;
    if let Some(b) = read_font(&["segoeui.ttf"]) {
        fonts.font_data.insert("segoe".into(), Arc::new(FontData::from_owned(b)));
        fonts.families.get_mut(&FontFamily::Proportional).unwrap().insert(0, "segoe".into());
    }
    let sb = read_font(&["seguisb.ttf", "segoeuib.ttf"]);
    if let Some(b) = sb {
        fonts.font_data.insert("segoe_sb".into(), Arc::new(FontData::from_owned(b)));
    }
    if let Some(b) = read_font(&["SegoeIcons.ttf", "segmdl2.ttf"]) {
        fonts.font_data.insert("icons".into(), Arc::new(FontData::from_owned(b)));
        has_icons = true;
    }
    let base: Vec<String> = fonts.families[&FontFamily::Proportional].clone();
    let mut semibold = base.clone();
    if fonts.font_data.contains_key("segoe_sb") {
        semibold.insert(0, "segoe_sb".into());
    }
    fonts.families.insert(FontFamily::Name(SEMIBOLD.into()), semibold);
    let mut icons = base;
    if has_icons {
        icons.insert(0, "icons".into());
    }
    fonts.families.insert(FontFamily::Name(ICONS.into()), icons);
    ctx.set_fonts(fonts);

    let mut style = (*ctx.style()).clone();
    style.text_styles = [
        (TextStyle::Heading, FontId::new(26.0, FontFamily::Name(SEMIBOLD.into()))),
        (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(14.0, FontFamily::Proportional)),
        (TextStyle::Small, FontId::new(12.0, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace)),
    ]
    .into();
    let s = &mut style.spacing;
    s.item_spacing = Vec2::new(10.0, 8.0);
    s.button_padding = Vec2::new(12.0, 6.0);
    s.interact_size = Vec2::new(40.0, 30.0);
    s.slider_width = 160.0;
    s.combo_width = 200.0;
    s.window_margin = Margin::same(14);
    s.menu_margin = Margin::same(6);
    s.scroll.bar_width = 8.0;
    s.scroll.floating = true;

    let v = &mut style.visuals;
    *v = egui::Visuals::dark();
    v.override_text_color = Some(pal::TEXT);
    v.panel_fill = pal::BG;
    v.window_fill = pal::CARD;
    v.extreme_bg_color = pal::BG;
    v.faint_bg_color = pal::CARD_HI;
    v.code_bg_color = pal::CONTROL;
    v.window_stroke = Stroke::new(1.0f32, pal::BORDER);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
    v.window_shadow = Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(120) };
    v.popup_shadow = Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(110) };
    v.selection.bg_fill = pal::ACCENT_DIM;
    v.selection.stroke = Stroke::new(1.0f32, pal::ACCENT);
    v.hyperlink_color = pal::ACCENT;
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;

    let r = CornerRadius::same(7);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = pal::CARD;
    w.noninteractive.weak_bg_fill = pal::CARD;
    w.noninteractive.bg_stroke = Stroke::new(1.0f32, pal::DIVIDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0f32, pal::TEXT);
    w.noninteractive.corner_radius = r;
    w.inactive.bg_fill = pal::CONTROL;
    w.inactive.weak_bg_fill = pal::CONTROL;
    w.inactive.bg_stroke = Stroke::new(1.0f32, pal::BORDER);
    w.inactive.fg_stroke = Stroke::new(1.0f32, pal::TEXT);
    w.inactive.corner_radius = r;
    w.hovered.bg_fill = pal::CONTROL_HI;
    w.hovered.weak_bg_fill = pal::CONTROL_HI;
    w.hovered.bg_stroke = Stroke::new(1.0f32, pal::FAINT);
    w.hovered.fg_stroke = Stroke::new(1.0f32, pal::TEXT);
    w.hovered.corner_radius = r;
    w.hovered.expansion = 0.0;
    w.active.bg_fill = pal::ACCENT_DIM;
    w.active.weak_bg_fill = pal::ACCENT_DIM;
    w.active.bg_stroke = Stroke::new(1.0f32, pal::ACCENT);
    w.active.fg_stroke = Stroke::new(1.0f32, pal::TEXT);
    w.active.corner_radius = r;
    w.active.expansion = 0.0;
    w.open = w.hovered;
    ctx.set_style(style);
    has_icons
}
