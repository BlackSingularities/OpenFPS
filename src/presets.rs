//! Gotowe motywy wyglądu nakładki. Każdy zwraca kompletny `Style`.

use crate::config::{Color, FontWeight, Style, TextEffect};

pub struct Preset {
    pub id: &'static str,
    pub name_pl: &'static str,
    pub name_en: &'static str,
    pub build: fn() -> Style,
}

pub const PRESETS: &[Preset] = &[
    Preset { id: "graphite", name_pl: "Grafit", name_en: "Graphite", build: graphite },
    Preset { id: "midnight", name_pl: "Północ", name_en: "Midnight", build: midnight },
    Preset { id: "neon", name_pl: "Neon", name_en: "Neon", build: neon },
    Preset { id: "minimal", name_pl: "Minimalny", name_en: "Minimal", build: minimal },
    Preset { id: "glass", name_pl: "Jasne szkło", name_en: "Light glass", build: glass },
    Preset { id: "terminal", name_pl: "Terminal", name_en: "Terminal", build: terminal },
];

/// Następny motyw w kolejności (do skrótu „przełącz motyw”).
pub fn next_after(id: &str) -> &'static Preset {
    let i = PRESETS.iter().position(|p| p.id == id).map(|i| i + 1).unwrap_or(0);
    &PRESETS[i % PRESETS.len()]
}

pub fn graphite() -> Style {
    Style {
        font_family: "Segoe UI".into(),
        font_weight: FontWeight::Semibold,
        font_size: 14.0,
        label_scale: 0.78,
        scale: 1.0,
        background: Color(0x12, 0x14, 0x19, 0xD9),
        text: Color(0xF2, 0xF4, 0xF8, 0xFF),
        label: Color(0x8B, 0x93, 0xA3, 0xFF),
        accent: Color(0x2D, 0xD4, 0xBF, 0xFF),
        bar_background: Color(0xFF, 0xFF, 0xFF, 0x1F),
        graph: Color(0x2D, 0xD4, 0xBF, 0xFF),
        border: true,
        border_color: Color(0xFF, 0xFF, 0xFF, 0x1A),
        border_width: 1.0,
        corner_radius: 10.0,
        padding: 10.0,
        row_spacing: 3.0,
        item_spacing: 14.0,
        text_effect: TextEffect::None,
        effect_color: Color(0x00, 0x00, 0x00, 0xB0),
        colorize: true,
        show_bars: true,
        bar_width: 56.0,
        bar_height: 4.0,
        graph_height: 38.0,
        graph_width: 200.0,
    }
}

pub fn midnight() -> Style {
    Style {
        background: Color(0x0B, 0x12, 0x24, 0xE0),
        text: Color(0xE6, 0xEC, 0xFF, 0xFF),
        label: Color(0x7C, 0x8B, 0xB5, 0xFF),
        accent: Color(0x60, 0xA5, 0xFA, 0xFF),
        graph: Color(0x60, 0xA5, 0xFA, 0xFF),
        bar_background: Color(0x60, 0xA5, 0xFA, 0x26),
        border_color: Color(0x60, 0xA5, 0xFA, 0x33),
        ..graphite()
    }
}

pub fn neon() -> Style {
    Style {
        font_weight: FontWeight::Bold,
        background: Color(0x0A, 0x05, 0x14, 0xD0),
        text: Color(0xFF, 0xFF, 0xFF, 0xFF),
        label: Color(0xE8, 0x79, 0xF9, 0xFF),
        accent: Color(0x22, 0xD3, 0xEE, 0xFF),
        graph: Color(0x22, 0xD3, 0xEE, 0xFF),
        bar_background: Color(0xE8, 0x79, 0xF9, 0x2A),
        border_color: Color(0xE8, 0x79, 0xF9, 0x80),
        border_width: 1.5,
        corner_radius: 4.0,
        text_effect: TextEffect::Shadow,
        effect_color: Color(0xE8, 0x79, 0xF9, 0x60),
        ..graphite()
    }
}

pub fn minimal() -> Style {
    Style {
        font_weight: FontWeight::Bold,
        background: Color(0, 0, 0, 0),
        text: Color(0xFF, 0xFF, 0xFF, 0xFF),
        label: Color(0xD0, 0xD4, 0xDC, 0xFF),
        border: false,
        padding: 4.0,
        text_effect: TextEffect::Outline,
        effect_color: Color(0x00, 0x00, 0x00, 0xE0),
        show_bars: false,
        ..graphite()
    }
}

pub fn glass() -> Style {
    Style {
        background: Color(0xF6, 0xF7, 0xFA, 0xE6),
        text: Color(0x14, 0x17, 0x1F, 0xFF),
        label: Color(0x5B, 0x63, 0x73, 0xFF),
        accent: Color(0x25, 0x63, 0xEB, 0xFF),
        graph: Color(0x25, 0x63, 0xEB, 0xFF),
        bar_background: Color(0x14, 0x17, 0x1F, 0x1A),
        border_color: Color(0x14, 0x17, 0x1F, 0x22),
        ..graphite()
    }
}

pub fn terminal() -> Style {
    Style {
        font_family: "Consolas".into(),
        font_weight: FontWeight::Bold,
        background: Color(0x03, 0x0A, 0x05, 0xE0),
        text: Color(0x86, 0xEF, 0xAC, 0xFF),
        label: Color(0x22, 0xC5, 0x5E, 0xFF),
        accent: Color(0x4A, 0xDE, 0x80, 0xFF),
        graph: Color(0x4A, 0xDE, 0x80, 0xFF),
        bar_background: Color(0x22, 0xC5, 0x5E, 0x26),
        border_color: Color(0x22, 0xC5, 0x5E, 0x55),
        corner_radius: 2.0,
        ..graphite()
    }
}
