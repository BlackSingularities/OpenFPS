//! Kontrolki okna ustawień zbudowane na egui, spójne z paletą z `theme`.

use eframe::egui::{
    self, Align, Color32, CornerRadius, FontFamily, FontId, Layout, Margin, Rect, Response, RichText, Sense, Stroke, StrokeKind,
    Ui, Vec2, WidgetInfo, WidgetType,
};

use super::theme::{pal, ICONS, SEMIBOLD};
use crate::config::{Anchor, Color};

pub fn icon(glyph: &str, size: f32) -> RichText {
    RichText::new(glyph).family(FontFamily::Name(ICONS.into())).size(size)
}

pub fn semibold(text: &str, size: f32) -> RichText {
    RichText::new(text).family(FontFamily::Name(SEMIBOLD.into())).size(size)
}

pub fn muted(text: &str) -> RichText {
    RichText::new(text).color(pal::MUTED)
}

/// Karta: tło + obwódka dookoła, opcjonalny nagłówek.
pub fn card<R>(ui: &mut Ui, title: Option<&str>, add: impl FnOnce(&mut Ui) -> R) -> R {
    // Szerokość treści liczona jawnie: dostępna szerokość minus marginesy (2×18) i obwódka (2×1).
    let inner_w = (ui.available_width() - 38.0).max(100.0);
    let r = egui::Frame::new()
        .fill(pal::CARD)
        .stroke(Stroke::new(1.0f32, pal::BORDER))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(18, 14))
        .show(ui, |ui| {
            ui.set_width(inner_w);
            if let Some(t) = title {
                ui.label(semibold(t, 15.0).color(pal::TEXT));
                ui.add_space(4.0);
            }
            add(ui)
        })
        .inner;
    ui.add_space(14.0);
    r
}

pub fn divider(ui: &mut Ui) {
    let w = ui.available_width();
    let (r, _) = ui.allocate_exact_size(Vec2::new(w, 9.0), Sense::hover());
    ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0f32, pal::DIVIDER));
}

/// Wiersz ustawienia: tytuł i opis po lewej, kontrolka wyrównana do prawej.
pub fn row(ui: &mut Ui, title: &str, desc: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        let avail = ui.available_width();
        let text_w = (avail - 360.0).max(avail * 0.4);
        ui.vertical(|ui| {
            ui.set_width(text_w);
            // Sam tytuł (bez opisu) wyrównujemy optycznie do środka kontrolki obok.
            ui.add_space(if desc.is_empty() { 7.0 } else { 2.0 });
            ui.label(RichText::new(title).color(pal::TEXT));
            if !desc.is_empty() {
                ui.label(RichText::new(desc).small().color(pal::MUTED));
            }
            ui.add_space(2.0);
        });
        ui.with_layout(Layout::right_to_left(Align::Center), add);
    });
}

/// Przełącznik w stylu Windows 11.
pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let size = Vec2::new(40.0, 22.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    resp.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), *on, ""));
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_responsive(resp.id, *on);
        let hovered = resp.hovered();
        let bg = if *on {
            pal::ACCENT.gamma_multiply(if hovered { 1.0 } else { 0.92 })
        } else if hovered {
            pal::CONTROL_HI
        } else {
            pal::CONTROL
        };
        let p = ui.painter();
        p.rect_filled(rect, CornerRadius::same(11), bg);
        if !*on {
            p.rect_stroke(rect, CornerRadius::same(11), Stroke::new(1.0f32, pal::FAINT), StrokeKind::Inside);
        }
        let x = egui::lerp((rect.left() + 11.0)..=(rect.right() - 11.0), t);
        let knob = if *on { pal::ON_ACCENT } else { pal::MUTED };
        p.circle_filled(egui::pos2(x, rect.center().y), if hovered { 7.0 } else { 6.0 }, knob);
    }
    resp
}

/// Kontrolka segmentowa (wybór jednej z kilku opcji).
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, &str)]) -> bool {
    let mut changed = false;
    // W układzie od prawej do lewej (wiersze ustawień) dodajemy opcje odwrotnie,
    // żeby wizualnie zachować kolejność z listy.
    let rtl = ui.layout().prefer_right_to_left();
    let ordered: Vec<&(T, &str)> = if rtl { options.iter().rev().collect() } else { options.iter().collect() };
    egui::Frame::new()
        .fill(pal::CONTROL)
        .stroke(Stroke::new(1.0f32, pal::BORDER))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(3))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.horizontal(|ui| {
                for (v, label) in ordered.iter().map(|o| (o.0, o.1)) {
                    let selected = *value == v;
                    let text = if selected { RichText::new(label).color(pal::ON_ACCENT).strong() } else { RichText::new(label).color(pal::TEXT) };
                    let btn = egui::Button::new(text)
                        .fill(if selected { pal::ACCENT } else { Color32::TRANSPARENT })
                        .stroke(Stroke::NONE)
                        .corner_radius(CornerRadius::same(6))
                        .min_size(Vec2::new(0.0, 26.0));
                    if ui.add(btn).clicked() && !selected {
                        *value = v;
                        changed = true;
                    }
                }
            });
        });
    changed
}

/// Wybór koloru z podglądem i zapisem HEX.
pub fn color(ui: &mut Ui, c: &mut Color) -> bool {
    let mut c32 = Color32::from_rgba_unmultiplied(c.0, c.1, c.2, c.3);
    let mut changed = false;
    ui.horizontal(|ui| {
        let r = egui::widgets::color_picker::color_edit_button_srgba(ui, &mut c32, egui::widgets::color_picker::Alpha::OnlyBlend);
        if r.changed() {
            let [r, g, b, a] = c32.to_srgba_unmultiplied();
            *c = Color(r, g, b, a);
            changed = true;
        }
        ui.label(RichText::new(c.to_hex()).monospace().color(pal::MUTED));
    });
    changed
}

/// Siatka 3×3 do wyboru kotwicy pozycji.
pub fn anchor_picker(ui: &mut Ui, a: &mut Anchor) -> bool {
    let cell = 20.0;
    let gap = 5.0;
    let size = Vec2::splat(cell * 3.0 + gap * 2.0 + 12.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(8), pal::CONTROL);
    p.rect_stroke(rect, CornerRadius::same(8), Stroke::new(1.0f32, pal::BORDER), StrokeKind::Inside);
    let mut changed = false;
    for row in 0..3u8 {
        for col in 0..3u8 {
            let min = rect.min + Vec2::new(6.0 + col as f32 * (cell + gap), 6.0 + row as f32 * (cell + gap));
            let r = Rect::from_min_size(min, Vec2::splat(cell));
            let id = ui.id().with(("anchor", row, col));
            let resp = ui.interact(r, id, Sense::click());
            let this = Anchor::from_grid(col, row);
            let sel = *a == this;
            let fill = if sel {
                pal::ACCENT
            } else if resp.hovered() {
                pal::CONTROL_HI
            } else {
                pal::CARD
            };
            ui.painter().rect_filled(r, CornerRadius::same(5), fill);
            if !sel {
                ui.painter().rect_stroke(r, CornerRadius::same(5), Stroke::new(1.0f32, pal::BORDER), StrokeKind::Inside);
            }
            if resp.clicked() && !sel {
                *a = this;
                changed = true;
            }
        }
    }
    changed
}

/// Klawisz/skrót w stylu „kbd”.
pub fn kbd(ui: &mut Ui, text: &str, active: bool) -> Response {
    let txt = RichText::new(text).font(FontId::monospace(13.0)).color(if active { pal::ON_ACCENT } else { pal::TEXT });
    ui.add(
        egui::Button::new(txt)
            .fill(if active { pal::ACCENT } else { pal::CONTROL })
            .stroke(Stroke::new(1.0f32, if active { pal::ACCENT } else { pal::BORDER }))
            .corner_radius(CornerRadius::same(6))
            .min_size(Vec2::new(170.0, 30.0)),
    )
}

/// Mała etykieta-plakietka o stałej wysokości (rysowana ręcznie, więc nie rozciąga się w wierszu).
pub fn badge(ui: &mut Ui, text: &str, fg: Color32) {
    let galley = ui.painter().layout_no_wrap(text.to_string(), FontId::proportional(12.0), fg);
    let size = galley.size() + Vec2::new(16.0, 4.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(10), fg.gamma_multiply(0.14));
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, fg);
}

/// Przycisk drugorzędny z ikoną.
pub fn button(ui: &mut Ui, glyph: &str, text: &str) -> Response {
    let mut job = egui::text::LayoutJob::default();
    if !glyph.is_empty() {
        job.append(glyph, 0.0, egui::TextFormat { font_id: FontId::new(13.0, FontFamily::Name(ICONS.into())), color: pal::TEXT, valign: Align::Center, ..Default::default() });
        job.append("  ", 0.0, egui::TextFormat { font_id: FontId::proportional(14.0), color: pal::TEXT, ..Default::default() });
    }
    job.append(text, 0.0, egui::TextFormat { font_id: FontId::proportional(14.0), color: pal::TEXT, valign: Align::Center, ..Default::default() });
    ui.add(egui::Button::new(job).min_size(Vec2::new(0.0, 30.0)))
}

/// Przycisk główny (akcent).
pub fn primary(ui: &mut Ui, text: &str) -> Response {
    ui.add(
        egui::Button::new(RichText::new(text).color(pal::ON_ACCENT).strong())
            .fill(pal::ACCENT)
            .stroke(Stroke::NONE)
            .min_size(Vec2::new(0.0, 30.0)),
    )
}
