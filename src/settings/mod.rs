//! Okno ustawień (osobny proces, egui). Każda zmiana jest po chwili zapisywana
//! do config.toml, a nakładka dostaje sygnał do przeładowania — podgląd na żywo.

mod theme;
mod extras;
mod widgets;

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use eframe::egui::{
    self, Align, Color32, ColorImage, CornerRadius, FontId, Layout, Margin, Pos2, Rect, RichText, Sense, Stroke, StrokeKind,
    TextureHandle, TextureOptions, Vec2,
};
use windows::core::w;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE};

use crate::config::{Color, Config, FontWeight, Lang, Metric, Mode, ModeLayout, ModeOverride, Orientation, Style, TempUnit, TextEffect};
use crate::hotkey::{key_from_name, Hotkey};
use crate::i18n::{describe, tr};
use crate::monitors::{self, MonitorInfo};
use crate::presets::PRESETS;
use crate::util;
use theme::{ico, pal};
use widgets::*;

const TITLE_PL: &str = "OpenFPS — Ustawienia";
const AUTHOR_LINE: &str = "Adam Rędzikowski · redzikowski.com";
const AUTHOR_URL: &str = "https://redzikowski.com";
const SUPPORT_URL: &str = "https://buycoffee.to/adamredzikowski";
const TITLE_EN: &str = "OpenFPS — Settings";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    General,
    Appearance,
    Simple,
    Detailed,
    Displays,
    Thresholds,
    Hotkeys,
    Feedback,
    About,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HkField {
    Toggle,
    Mode,
    Settings,
    Move,
    Preset,
}

const FONTS: &[&str] = &[
    "Segoe UI",
    "Segoe UI Variable Display",
    "Bahnschrift",
    "Cascadia Mono",
    "Consolas",
    "Calibri",
    "Arial",
    "Verdana",
    "Tahoma",
    "Trebuchet MS",
    "Inter",
    "Roboto",
];

struct SettingsApp {
    cfg: Config,
    saved: Config,
    dirty_since: Option<Instant>,
    page: Page,
    monitors: Vec<MonitorInfo>,
    selected_monitor: String,
    capture: Option<HkField>,
    capture_error: Option<String>,
    excluded_text: String,
    file_mtime: Option<SystemTime>,
    last_poll: Instant,
    last_monitor_scan: Instant,
    overlay_running: bool,
    toast: Option<(String, Instant)>,
    fb: extras::FeedbackForm,
    upd: Arc<std::sync::Mutex<extras::UpdState>>,
    confirm_reset: bool,
    logo: Option<TextureHandle>,
    lang_shown: Lang,
}

fn mtime() -> Option<SystemTime> {
    std::fs::metadata(Config::path()).and_then(|m| m.modified()).ok()
}

impl SettingsApp {
    fn new(ctx: &egui::Context, page: Page) -> Self {
        let mut cfg = Config::load();
        let monitors = monitors::enumerate();
        for m in &monitors {
            cfg.monitor_mut(&m.id, &m.name, m.primary);
        }
        let selected_monitor = monitors.first().map(|m| m.id.clone()).unwrap_or_default();
        let logo = Some(ctx.load_texture("logo", ColorImage::from_rgba_unmultiplied([64, 64], &crate::icon::rgba(64)), TextureOptions::LINEAR));
        let upd = Arc::new(std::sync::Mutex::new(extras::UpdState::Idle));
        // Otwarte z menu „Zaktualizuj…” — od razu pokaż dostępną wersję.
        if page == Page::About {
            extras::spawn_check(upd.clone(), ctx.clone());
        }
        Self {
            excluded_text: cfg.general.excluded_processes.join("\n"),
            saved: Config::load(),
            lang_shown: cfg.general.language,
            cfg,
            dirty_since: None,
            page,
            monitors,
            selected_monitor,
            capture: None,
            capture_error: None,
            file_mtime: mtime(),
            last_poll: Instant::now(),
            last_monitor_scan: Instant::now(),
            overlay_running: util::overlay_running(),
            toast: None,
            fb: extras::FeedbackForm::new(if page == Page::Feedback { crate::feedback::pending_crash().filter(|_| crash_arg()) } else { None }),
            upd,
            confirm_reset: false,
            logo,
        }
    }

    fn t(&self, pl: &'static str, en: &'static str) -> &'static str {
        tr(self.cfg.general.language, pl, en)
    }

    fn toast(&mut self, s: &str) {
        self.toast = Some((s.to_string(), Instant::now()));
    }

    /// Zapis z opóźnieniem (debounce) + powiadomienie nakładki.
    fn persist(&mut self) {
        if self.cfg == self.saved {
            self.dirty_since = None;
            return;
        }
        let since = *self.dirty_since.get_or_insert_with(Instant::now);
        if since.elapsed() < Duration::from_millis(180) {
            return;
        }
        self.cfg.sanitize();
        let (a0, b0) = (self.saved.general.autostart, self.saved.general.run_as_admin);
        let (a1, b1) = (self.cfg.general.autostart, self.cfg.general.run_as_admin);
        if (a0, b0) != (a1, b1) {
            util::apply_autostart(a1, b1, a0 && b0);
        }
        if self.cfg.save().is_ok() {
            self.saved = self.cfg.clone();
            self.file_mtime = mtime();
            util::post_to_overlay(util::MSG_RELOAD, 0);
        }
        self.dirty_since = None;
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if self.last_poll.elapsed() < Duration::from_millis(950) {
            return;
        }
        self.last_poll = Instant::now();
        self.overlay_running = util::overlay_running();
        // Zmiana pliku z zewnątrz (np. przeciągnięcie nakładki, menu zasobnika).
        let m = mtime();
        if m != self.file_mtime && self.cfg == self.saved {
            self.file_mtime = m;
            self.cfg = Config::load();
            for mon in &self.monitors {
                self.cfg.monitor_mut(&mon.id, &mon.name, mon.primary);
            }
            self.saved = self.cfg.clone();
            self.excluded_text = self.cfg.general.excluded_processes.join("\n");
        }
        if self.page == Page::Displays && self.last_monitor_scan.elapsed() > Duration::from_secs(3) {
            self.last_monitor_scan = Instant::now();
            self.monitors = monitors::enumerate();
            for mon in &self.monitors {
                self.cfg.monitor_mut(&mon.id, &mon.name, mon.primary);
            }
        }
        if self.lang_shown != self.cfg.general.language {
            self.lang_shown = self.cfg.general.language;
            let t = self.t(TITLE_PL, TITLE_EN);
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(t.into()));
        }
    }

    // ───────────────────────────── Szkielet ─────────────────────────────

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if let Some(l) = &self.logo {
                ui.add(egui::Image::new((l.id(), Vec2::splat(34.0))));
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(semibold("OpenFPS", 18.0));
                ui.label(RichText::new(format!("v{}", crate::util::VERSION)).small().color(pal::MUTED));
            });
        });
        ui.add_space(22.0);

        let items = [
            (Page::General, ico::GENERAL, self.t("Ogólne", "General")),
            (Page::Appearance, ico::APPEARANCE, self.t("Wygląd", "Appearance")),
            (Page::Simple, ico::SIMPLE, self.t("Tryb prosty", "Simple mode")),
            (Page::Detailed, ico::DETAILED, self.t("Tryb szczegółowy", "Detailed mode")),
            (Page::Displays, ico::DISPLAYS, self.t("Monitory", "Displays")),
            (Page::Thresholds, ico::THRESHOLDS, self.t("Progi i kolory", "Thresholds")),
            (Page::Hotkeys, ico::HOTKEYS, self.t("Skróty klawiszowe", "Hotkeys")),
            (Page::Feedback, ico::FEEDBACK, self.t("Zgłoś błąd lub sugestię", "Report a problem")),
            (Page::About, ico::ABOUT, self.t("O programie", "About")),
        ];
        for (page, glyph, label) in items {
            let selected = self.page == page;
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 38.0), Sense::click());
            let fill = if selected {
                pal::CONTROL
            } else if resp.hovered() {
                pal::CARD
            } else {
                Color32::TRANSPARENT
            };
            ui.painter().rect_filled(rect, CornerRadius::same(8), fill);
            let fg = if selected { pal::TEXT } else { pal::MUTED };
            let ic = if selected { pal::ACCENT } else { pal::MUTED };
            ui.painter().text(
                Pos2::new(rect.left() + 16.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                glyph,
                FontId::new(15.0, egui::FontFamily::Name(theme::ICONS.into())),
                ic,
            );
            ui.painter().text(Pos2::new(rect.left() + 44.0, rect.center().y), egui::Align2::LEFT_CENTER, label, FontId::proportional(14.0), fg);
            if resp.clicked() {
                self.page = page;
                if page == Page::Displays {
                    self.monitors = monitors::enumerate();
                }
            }
            ui.add_space(2.0);
        }

        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            ui.add_space(4.0);
            let mut lang = self.cfg.general.language;
            if segmented(ui, &mut lang, &[(Lang::Pl, "Polski"), (Lang::En, "English")]) {
                self.cfg.general.language = lang;
            }
            ui.add_space(10.0);
            egui::Frame::new()
                .fill(pal::CARD)
                .stroke(Stroke::new(1.0f32, pal::BORDER))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(Margin::same(12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    // Panel układa elementy od dołu, więc przycisk dodajemy pierwszy (ląduje pod statusem).
                    if self.overlay_running {
                        if button(ui, ico::STOP, self.t("Zatrzymaj", "Stop")).clicked() {
                            util::post_to_overlay(util::MSG_QUIT, 0);
                            self.last_poll = Instant::now() - Duration::from_secs(1);
                        }
                    } else if button(ui, ico::PLAY, self.t("Uruchom", "Start")).clicked() {
                        util::spawn_self(&[]);
                        self.last_poll = Instant::now() - Duration::from_millis(300);
                    }
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let (dot, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                        ui.painter().circle_filled(dot.center(), 4.5, if self.overlay_running { pal::GOOD } else { pal::FAINT });
                        let s = if self.overlay_running { self.t("Nakładka działa", "Overlay running") } else { self.t("Nakładka wyłączona", "Overlay stopped") };
                        ui.label(RichText::new(s).color(pal::TEXT));
                    });
                });
        });
    }

    pub(super) fn page_header(&self, ui: &mut egui::Ui, title: &str, subtitle: &str) {
        ui.add_space(4.0);
        ui.label(RichText::new(title).heading());
        ui.label(muted(subtitle));
        ui.add_space(18.0);
    }

    // ───────────────────────────── Strony ─────────────────────────────

    fn page_general(&mut self, ui: &mut egui::Ui) {
        let l = self.cfg.general.language;
        let t = |pl, en| tr(l, pl, en);
        self.page_header(ui, t("Ogólne", "General"), t("Zachowanie nakładki, odświeżanie pomiarów i uruchamianie.", "Overlay behaviour, measurement refresh and startup."));
        let g = &mut self.cfg.general;

        card(ui, Some(t("Zachowanie", "Behaviour")), |ui| {
            row(ui, t("Nakładka widoczna", "Overlay visible"), t("To samo robi skrót „pokaż / ukryj”", "Same as the show / hide hotkey"), |ui| {
                toggle(ui, &mut g.visible);
            });
            divider(ui);
            row(ui, t("Domyślny tryb", "Default mode"), t("Monitory mogą mieć własny tryb", "Displays can override it"), |ui| {
                segmented(ui, &mut g.mode, &[(Mode::Simple, t("Prosty", "Simple")), (Mode::Detailed, t("Szczegółowy", "Detailed"))]);
            });
            divider(ui);
            row(ui, t("Ukryj, gdy nic nie renderuje", "Hide when nothing renders"), t("Nakładka pojawia się tylko przy grze / aplikacji 3D", "Overlay appears only with a game / 3D app"), |ui| {
                toggle(ui, &mut g.auto_hide_without_app);
            });
            divider(ui);
            row(ui, t("Ukryj na nagraniach i zrzutach", "Hide from captures"), t("OBS, Discord i zrzuty ekranu nie zobaczą nakładki", "OBS, Discord and screenshots won't see the overlay"), |ui| {
                toggle(ui, &mut g.hide_from_capture);
            });
        });

        card(ui, Some(t("Pomiary", "Measurements")), |ui| {
            row(ui, t("Odświeżanie FPS", "FPS refresh"), t("Jak często aktualizować FPS i czas klatki", "How often FPS and frametime update"), |ui| {
                ui.add(egui::Slider::new(&mut g.fps_refresh_ms, 50..=1000).suffix(" ms").step_by(10.0));
            });
            divider(ui);
            row(ui, t("Odświeżanie metryk", "System metrics refresh"), t("CPU, GPU, pamięć, sieć, dysk", "CPU, GPU, memory, network, disk"), |ui| {
                ui.add(egui::Slider::new(&mut g.sys_refresh_ms, 250..=5000).suffix(" ms").step_by(50.0));
            });
            divider(ui);
            row(ui, t("Uśrednianie FPS", "FPS averaging"), t("Dłuższe okno = stabilniejsza liczba", "Longer window = steadier number"), |ui| {
                ui.add(egui::Slider::new(&mut g.fps_average_ms, 250..=3000).suffix(" ms").step_by(50.0));
            });
            divider(ui);
            row(ui, t("Okno 1% / 0.1% low", "1% / 0.1% low window"), t("Z ilu ostatnich sekund liczyć spadki", "How many recent seconds to analyse"), |ui| {
                ui.add(egui::Slider::new(&mut g.lows_window_s, 2..=60).suffix(" s"));
            });
            divider(ui);
            row(ui, t("Zakres wykresów", "Graph span"), t("Ile sekund historii pokazują wykresy", "Seconds of history shown on graphs"), |ui| {
                ui.add(egui::Slider::new(&mut g.graph_seconds, 1..=30).suffix(" s"));
            });
            divider(ui);
            row(ui, t("Jednostka temperatury", "Temperature unit"), "", |ui| {
                segmented(ui, &mut g.temperature_unit, &[(TempUnit::Celsius, "°C"), (TempUnit::Fahrenheit, "°F")]);
            });
        });

        card(ui, Some(t("Uruchamianie", "Startup")), |ui| {
            row(ui, t("Uruchamiaj z Windows", "Start with Windows"), t("Nakładka startuje po zalogowaniu", "Overlay starts after you sign in"), |ui| {
                toggle(ui, &mut g.autostart);
            });
            divider(ui);
            row(
                ui,
                t("Zawsze jako administrator", "Always as administrator"),
                t("Potrzebne do pomiaru FPS, jeśli konto nie ma uprawnień do śledzenia zdarzeń. Autostart użyje Harmonogramu zadań.", "Needed for FPS if your account can't trace events. Autostart will use Task Scheduler."),
                |ui| {
                    toggle(ui, &mut g.run_as_admin);
                },
            );
        });

        card(ui, Some(t("Wykluczone procesy", "Excluded processes")), |ui| {
            ui.label(muted(t("Te procesy nigdy nie będą wybierane jako źródło FPS. Jeden plik .exe w linii.", "These processes are never used as an FPS source. One .exe per line.")));
            ui.add_space(6.0);
            let r = ui.add(
                egui::TextEdit::multiline(&mut self.excluded_text)
                    .desired_rows(5)
                    .desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Monospace)
                    .margin(Margin::same(10)),
            );
            if r.changed() {
                self.cfg.general.excluded_processes =
                    self.excluded_text.lines().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            }
        });
    }

    fn preset_tile(ui: &mut egui::Ui, style: &Style, name: &str, selected: bool) -> egui::Response {
        let size = Vec2::new(150.0, 96.0);
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        let p = ui.painter();
        // Szachownica pod spodem pokazuje przezroczystość tła.
        p.rect_filled(rect, CornerRadius::same(10), Color32::from_rgb(0x2A, 0x2F, 0x38));
        let c = |c: Color| Color32::from_rgba_unmultiplied(c.0, c.1, c.2, c.3);
        let inner = rect.shrink2(Vec2::new(10.0, 10.0));
        let card = Rect::from_min_size(inner.min, Vec2::new(inner.width(), 54.0));
        p.rect_filled(card, CornerRadius::same(style.corner_radius.min(12.0) as u8), c(style.background));
        if style.border {
            p.rect_stroke(card, CornerRadius::same(style.corner_radius.min(12.0) as u8), Stroke::new(1.0f32, c(style.border_color)), StrokeKind::Inside);
        }
        p.text(card.min + Vec2::new(10.0, 8.0), egui::Align2::LEFT_TOP, "FPS", FontId::proportional(11.0), c(style.label));
        p.text(card.min + Vec2::new(40.0, 2.0), egui::Align2::LEFT_TOP, "144", FontId::new(20.0, egui::FontFamily::Name(theme::SEMIBOLD.into())), c(style.text));
        let bar = Rect::from_min_size(card.min + Vec2::new(10.0, 38.0), Vec2::new(card.width() - 20.0, 4.0));
        p.rect_filled(bar, CornerRadius::same(2), c(style.bar_background));
        p.rect_filled(Rect::from_min_size(bar.min, Vec2::new(bar.width() * 0.62, 4.0)), CornerRadius::same(2), c(style.accent));
        p.text(Pos2::new(inner.left(), inner.bottom()), egui::Align2::LEFT_BOTTOM, name, FontId::proportional(13.0), pal::TEXT);
        let stroke = if selected {
            Stroke::new(2.0f32, pal::ACCENT)
        } else if resp.hovered() {
            Stroke::new(1.0f32, pal::FAINT)
        } else {
            Stroke::new(1.0f32, pal::BORDER)
        };
        p.rect_stroke(rect, CornerRadius::same(10), stroke, StrokeKind::Inside);
        resp
    }

    fn page_appearance(&mut self, ui: &mut egui::Ui) {
        let l = self.cfg.general.language;
        let t = |pl, en| tr(l, pl, en);
        self.page_header(ui, t("Wygląd", "Appearance"), t("Motywy, typografia, kolory i proporcje. Zmiany widać od razu na nakładce.", "Themes, typography, colours and proportions. Changes show live on the overlay."));
        let before = self.cfg.style.clone();

        card(ui, Some(t("Motyw", "Theme")), |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(12.0, 12.0);
                for p in PRESETS {
                    let style = (p.build)();
                    let name = tr(l, p.name_pl, p.name_en);
                    if Self::preset_tile(ui, &style, name, self.cfg.preset == p.id).clicked() {
                        self.cfg.preset = p.id.to_string();
                        self.cfg.style = style;
                    }
                }
            });
            if self.cfg.preset == "custom" {
                ui.add_space(6.0);
                ui.label(muted(t("Używasz własnego motywu — kliknij kafelek, by wrócić do gotowego.", "You're using a custom theme — click a tile to restore a preset.")));
            }
        });

        let s = &mut self.cfg.style;
        card(ui, Some(t("Typografia", "Typography")), |ui| {
            row(ui, t("Czcionka", "Font"), t("Dowolna zainstalowana czcionka", "Any installed font"), |ui| {
                egui::ComboBox::from_id_salt("font").selected_text("").width(28.0).show_ui(ui, |ui| {
                    for f in FONTS {
                        ui.selectable_value(&mut s.font_family, f.to_string(), *f);
                    }
                });
                ui.add(egui::TextEdit::singleline(&mut s.font_family).desired_width(150.0));
            });
            divider(ui);
            row(ui, t("Grubość", "Weight"), "", |ui| {
                segmented(
                    ui,
                    &mut s.font_weight,
                    &[
                        (FontWeight::Regular, t("Normalna", "Regular")),
                        (FontWeight::Medium, t("Średnia", "Medium")),
                        (FontWeight::Semibold, t("Półgruba", "Semibold")),
                        (FontWeight::Bold, t("Gruba", "Bold")),
                    ],
                );
            });
            divider(ui);
            row(ui, t("Rozmiar tekstu", "Text size"), "", |ui| {
                ui.add(egui::Slider::new(&mut s.font_size, 8.0..=40.0).suffix(" px").step_by(0.5));
            });
            divider(ui);
            row(ui, t("Rozmiar etykiet", "Label size"), t("Względem wartości", "Relative to values"), |ui| {
                ui.add(egui::Slider::new(&mut s.label_scale, 0.5..=1.2).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)).step_by(0.01));
            });
            divider(ui);
            row(ui, t("Skala nakładki", "Overlay scale"), t("Mnożnik dla wszystkich wymiarów (oprócz DPI ekranu)", "Multiplier for every dimension (on top of display DPI)"), |ui| {
                ui.add(egui::Slider::new(&mut s.scale, 0.5..=3.0).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)).step_by(0.05));
            });
        });

        card(ui, Some(t("Kolory", "Colours")), |ui| {
            let rows: [(&str, &str, &mut Color); 7] = [
                (t("Tło", "Background"), t("Przezroczystość = kanał alfa", "Transparency = alpha channel"), &mut s.background),
                (t("Wartości", "Values"), "", &mut s.text),
                (t("Etykiety i jednostki", "Labels and units"), "", &mut s.label),
                (t("Akcent", "Accent"), t("Paski, nagłówek, wyróżnienia", "Bars, header, highlights"), &mut s.accent),
                (t("Tło pasków", "Bar track"), "", &mut s.bar_background),
                (t("Linia wykresu", "Graph line"), "", &mut s.graph),
                (t("Obramowanie", "Border"), "", &mut s.border_color),
            ];
            let n = rows.len();
            for (i, (title, desc, c)) in rows.into_iter().enumerate() {
                row(ui, title, desc, |ui| {
                    color(ui, c);
                });
                if i + 1 < n {
                    divider(ui);
                }
            }
            divider(ui);
            row(ui, t("Koloruj wartości wg progów", "Colour values by thresholds"), t("Zielony / żółty / czerwony — patrz „Progi i kolory”", "Green / yellow / red — see Thresholds"), |ui| {
                toggle(ui, &mut s.colorize);
            });
        });

        card(ui, Some(t("Kształt i odstępy", "Shape and spacing")), |ui| {
            row(ui, t("Zaokrąglenie rogów", "Corner radius"), "", |ui| {
                ui.add(egui::Slider::new(&mut s.corner_radius, 0.0..=24.0).suffix(" px").step_by(1.0));
            });
            divider(ui);
            row(ui, t("Margines wewnętrzny", "Padding"), "", |ui| {
                ui.add(egui::Slider::new(&mut s.padding, 0.0..=30.0).suffix(" px").step_by(1.0));
            });
            divider(ui);
            row(ui, t("Odstęp między wierszami", "Row spacing"), t("Układ pionowy", "Vertical layout"), |ui| {
                ui.add(egui::Slider::new(&mut s.row_spacing, 0.0..=20.0).suffix(" px").step_by(0.5));
            });
            divider(ui);
            row(ui, t("Odstęp między pozycjami", "Item spacing"), t("Układ poziomy", "Horizontal layout"), |ui| {
                ui.add(egui::Slider::new(&mut s.item_spacing, 0.0..=48.0).suffix(" px").step_by(1.0));
            });
            divider(ui);
            row(ui, t("Obramowanie", "Border"), "", |ui| {
                ui.add_enabled(s.border, egui::Slider::new(&mut s.border_width, 0.5..=4.0).suffix(" px").step_by(0.5));
                toggle(ui, &mut s.border);
            });
        });

        card(ui, Some(t("Efekt tekstu", "Text effect")), |ui| {
            row(ui, t("Efekt", "Effect"), t("Poprawia czytelność na przezroczystym tle", "Improves legibility on a transparent background"), |ui| {
                segmented(
                    ui,
                    &mut s.text_effect,
                    &[(TextEffect::None, t("Brak", "None")), (TextEffect::Shadow, t("Cień", "Shadow")), (TextEffect::Outline, t("Kontur", "Outline"))],
                );
            });
            divider(ui);
            row(ui, t("Kolor efektu", "Effect colour"), "", |ui| {
                color(ui, &mut s.effect_color);
            });
        });

        card(ui, Some(t("Paski i wykresy", "Bars and graphs")), |ui| {
            row(ui, t("Paski obciążenia", "Load bars"), t("Przy CPU, GPU, VRAM, RAM i baterii", "Next to CPU, GPU, VRAM, RAM and battery"), |ui| {
                toggle(ui, &mut s.show_bars);
            });
            divider(ui);
            row(ui, t("Szerokość paska", "Bar width"), "", |ui| {
                ui.add(egui::Slider::new(&mut s.bar_width, 20.0..=200.0).suffix(" px").step_by(1.0));
            });
            divider(ui);
            row(ui, t("Grubość paska", "Bar height"), "", |ui| {
                ui.add(egui::Slider::new(&mut s.bar_height, 2.0..=14.0).suffix(" px").step_by(0.5));
            });
            divider(ui);
            row(ui, t("Szerokość wykresu", "Graph width"), "", |ui| {
                ui.add(egui::Slider::new(&mut s.graph_width, 80.0..=600.0).suffix(" px").step_by(2.0));
            });
            divider(ui);
            row(ui, t("Wysokość wykresu", "Graph height"), "", |ui| {
                ui.add(egui::Slider::new(&mut s.graph_height, 16.0..=160.0).suffix(" px").step_by(1.0));
            });
        });

        if self.cfg.style != before && !PRESETS.iter().any(|p| (p.build)() == self.cfg.style) {
            self.cfg.preset = "custom".into();
        }
    }

    fn page_layout(&mut self, ui: &mut egui::Ui, mode: Mode) {
        let l = self.cfg.general.language;
        let t = |pl, en| tr(l, pl, en);
        let (title, sub) = match mode {
            Mode::Simple => (t("Tryb prosty", "Simple mode"), t("Kompaktowy pasek z najważniejszymi liczbami.", "A compact strip with the numbers that matter.")),
            Mode::Detailed => (t("Tryb szczegółowy", "Detailed mode"), t("Pełny panel z wykresami i metrykami sprzętu.", "A full panel with graphs and hardware metrics.")),
        };
        self.page_header(ui, title, sub);
        let layout: &mut ModeLayout = match mode {
            Mode::Simple => &mut self.cfg.simple,
            Mode::Detailed => &mut self.cfg.detailed,
        };

        card(ui, Some(t("Układ", "Layout")), |ui| {
            row(ui, t("Kierunek", "Direction"), "", |ui| {
                segmented(ui, &mut layout.orientation, &[(Orientation::Horizontal, t("Poziomy", "Horizontal")), (Orientation::Vertical, t("Pionowy", "Vertical"))]);
            });
            divider(ui);
            row(ui, t("Etykiety", "Labels"), t("Nazwy przy wartościach (np. „CPU”)", "Names next to values (e.g. “CPU”)"), |ui| {
                toggle(ui, &mut layout.show_labels);
            });
            divider(ui);
            row(ui, t("Nagłówek", "Header"), t("Nazwa programu i parametry ekranu", "Title and display info"), |ui| {
                toggle(ui, &mut layout.show_header);
            });
        });

        let enabled = layout.items.iter().filter(|i| i.enabled).count();
        card(ui, None, |ui| {
            ui.horizontal(|ui| {
                ui.label(semibold(t("Pozycje", "Items"), 15.0));
                badge(ui, &format!("{} / {}", enabled, layout.items.len()), pal::ACCENT);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if button(ui, ico::RESET, t("Domyślne", "Defaults")).clicked() {
                        *layout = match mode {
                            Mode::Simple => ModeLayout::default_simple(),
                            Mode::Detailed => ModeLayout::default_detailed(),
                        };
                    }
                });
            });
            ui.label(muted(t("Kolejność na liście = kolejność na nakładce.", "List order = order on the overlay.")));
            ui.add_space(8.0);
            let n = layout.items.len();
            let mut swap: Option<(usize, usize)> = None;
            for i in 0..n {
                let item = &mut layout.items[i];
                let (name, desc) = describe(item.metric, l);
                let fill = if item.enabled { pal::CARD_HI } else { Color32::TRANSPARENT };
                egui::Frame::new().fill(fill).corner_radius(CornerRadius::same(8)).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        toggle(ui, &mut item.enabled);
                        ui.add_space(6.0);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            ui.label(RichText::new(name).color(if item.enabled { pal::TEXT } else { pal::MUTED }));
                            ui.label(RichText::new(desc).small().color(pal::MUTED));
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            if ui.add_enabled(i + 1 < n, egui::Button::new(icon(ico::DOWN, 11.0)).min_size(Vec2::splat(28.0))).clicked() {
                                swap = Some((i, i + 1));
                            }
                            if ui.add_enabled(i > 0, egui::Button::new(icon(ico::UP, 11.0)).min_size(Vec2::splat(28.0))).clicked() {
                                swap = Some((i, i - 1));
                            }
                        });
                    });
                });
                ui.add_space(2.0);
            }
            if let Some((a, b)) = swap {
                layout.items.swap(a, b);
            }
        });
        let _ = Metric::ALL; // lista metryk pochodzi z konfiguracji (normalize)
    }

    fn monitor_map(&mut self, ui: &mut egui::Ui) {
        if self.monitors.is_empty() {
            return;
        }
        let (mut minx, mut miny, mut maxx, mut maxy) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for m in &self.monitors {
            minx = minx.min(m.rect.left);
            miny = miny.min(m.rect.top);
            maxx = maxx.max(m.rect.right);
            maxy = maxy.max(m.rect.bottom);
        }
        let w = ui.available_width();
        let h = 190.0;
        let (area, _) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
        let sx = (w - 20.0) / (maxx - minx) as f32;
        let sy = (h - 20.0) / (maxy - miny) as f32;
        let sc = sx.min(sy);
        let ox = area.left() + (w - (maxx - minx) as f32 * sc) / 2.0;
        let oy = area.top() + (h - (maxy - miny) as f32 * sc) / 2.0;
        for (i, m) in self.monitors.iter().enumerate() {
            let r = Rect::from_min_max(
                Pos2::new(ox + (m.rect.left - minx) as f32 * sc, oy + (m.rect.top - miny) as f32 * sc),
                Pos2::new(ox + (m.rect.right - minx) as f32 * sc, oy + (m.rect.bottom - miny) as f32 * sc),
            )
            .shrink(3.0);
            let resp = ui.interact(r, ui.id().with(("mon", i)), Sense::click());
            let mc = self.cfg.monitor(&m.id).cloned().unwrap_or_default();
            let selected = self.selected_monitor == m.id;
            let fill = if mc.enabled { Color32::from_rgb(0x12, 0x2A, 0x2A) } else if resp.hovered() { pal::CONTROL_HI } else { pal::CONTROL };
            let p = ui.painter();
            p.rect_filled(r, CornerRadius::same(8), fill);
            let stroke = if selected { Stroke::new(2.0f32, pal::ACCENT) } else { Stroke::new(1.0f32, pal::BORDER) };
            p.rect_stroke(r, CornerRadius::same(8), stroke, StrokeKind::Inside);
            p.text(r.center() - Vec2::new(0.0, 10.0), egui::Align2::CENTER_CENTER, format!("{}", i + 1), FontId::new(22.0, egui::FontFamily::Name(theme::SEMIBOLD.into())), pal::TEXT);
            p.text(r.center() + Vec2::new(0.0, 14.0), egui::Align2::CENTER_CENTER, &m.name, FontId::proportional(11.0), pal::MUTED);
            if mc.enabled {
                // Kropka w miejscu kotwicy nakładki.
                let (c, rr) = mc.anchor.grid();
                let inset = 12.0;
                let x = egui::lerp((r.left() + inset)..=(r.right() - inset), c as f32 / 2.0);
                let y = egui::lerp((r.top() + inset)..=(r.bottom() - inset), rr as f32 / 2.0);
                p.circle_filled(Pos2::new(x, y), 5.0, pal::ACCENT);
            }
            if resp.clicked() {
                self.selected_monitor = m.id.clone();
            }
        }
    }

    fn page_displays(&mut self, ui: &mut egui::Ui) {
        let l = self.cfg.general.language;
        let t = |pl, en| tr(l, pl, en);
        self.page_header(ui, t("Monitory", "Displays"), t("Nakładka może działać na dowolnej liczbie ekranów — każdy pokazuje FPS aplikacji, która jest na nim.", "Run the overlay on any number of screens — each shows the FPS of the app on that screen."));

        card(ui, None, |ui| self.monitor_map(ui));

        let Some(mon) = self.monitors.iter().find(|m| m.id == self.selected_monitor).cloned() else { return };
        let idx = self.monitors.iter().position(|m| m.id == mon.id).unwrap_or(0);
        let move_hk = self.cfg.hotkeys.move_mode.clone();
        let mc = self.cfg.monitor_mut(&mon.id, &mon.name, mon.primary);
        card(ui, None, |ui| {
            ui.horizontal(|ui| {
                ui.label(semibold(&format!("{} · {}", idx + 1, mon.name), 16.0));
                if mon.primary {
                    badge(ui, t("główny", "primary"), pal::ACCENT);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    toggle(ui, &mut mc.enabled);
                    ui.label(muted(t("Nakładka na tym ekranie", "Overlay on this display")));
                });
            });
            ui.label(muted(&format!("{}×{} · {} Hz · {}% DPI · {}", mon.width(), mon.height(), mon.refresh_hz, mon.dpi * 100 / 96, mon.id)));
            ui.add_space(8.0);
            ui.add_enabled_ui(mc.enabled, |ui| {
                divider(ui);
                row(ui, t("Tryb na tym ekranie", "Mode on this display"), "", |ui| {
                    segmented(
                        ui,
                        &mut mc.mode,
                        &[(ModeOverride::Inherit, t("Domyślny", "Default")), (ModeOverride::Simple, t("Prosty", "Simple")), (ModeOverride::Detailed, t("Szczegółowy", "Detailed"))],
                    );
                });
                divider(ui);
                row(ui, t("Pozycja", "Position"), t("Punkt zaczepienia nakładki. Możesz też przeciągnąć ją myszą po wciśnięciu skrótu przesuwania.", "Overlay anchor point. You can also drag it with the mouse after pressing the move hotkey."), |ui| {
                    anchor_picker(ui, &mut mc.anchor);
                });
                if !move_hk.is_empty() {
                    ui.label(RichText::new(format!("{} {}", t("Skrót przesuwania:", "Move hotkey:"), move_hk)).small().color(pal::MUTED));
                }
                divider(ui);
                row(ui, t("Odsunięcie", "Offset"), t("Od krawędzi ekranu (px w skali 100%)", "From the screen edge (px at 100% scale)"), |ui| {
                    ui.add(egui::DragValue::new(&mut mc.margin_y).range(-4000.0..=4000.0).speed(1.0).prefix("Y  ").suffix(" px"));
                    ui.add(egui::DragValue::new(&mut mc.margin_x).range(-4000.0..=4000.0).speed(1.0).prefix("X  ").suffix(" px"));
                });
                divider(ui);
                row(ui, t("Skala na tym ekranie", "Scale on this display"), t("Mnożona przez skalę z „Wyglądu”", "Multiplied by the Appearance scale"), |ui| {
                    ui.add(egui::Slider::new(&mut mc.scale, 0.5..=3.0).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)).step_by(0.05));
                });
                divider(ui);
                row(ui, t("Omijaj pasek zadań", "Avoid the taskbar"), t("Pozycjonuj względem obszaru roboczego", "Position within the work area"), |ui| {
                    toggle(ui, &mut mc.use_work_area);
                });
            });
        });

        card(ui, None, |ui| {
            row(ui, t("Nowe monitory włączone", "New displays enabled"), t("Czy po podłączeniu ekranu od razu pokazywać na nim nakładkę", "Show the overlay right away on newly connected displays"), |ui| {
                toggle(ui, &mut self.cfg.general.new_monitors_enabled);
            });
        });
    }

    fn page_thresholds(&mut self, ui: &mut egui::Ui) {
        let l = self.cfg.general.language;
        let t = |pl, en| tr(l, pl, en);
        self.page_header(ui, t("Progi i kolory", "Thresholds"), t("Kiedy wartości zmieniają kolor na ostrzegawczy.", "When values switch to warning colours."));
        let th = &mut self.cfg.thresholds;
        card(ui, Some(t("Kolory stanów", "State colours")), |ui| {
            row(ui, t("Dobrze", "Good"), "", |ui| {
                color(ui, &mut th.good);
            });
            divider(ui);
            row(ui, t("Uwaga", "Warning"), "", |ui| {
                color(ui, &mut th.warn);
            });
            divider(ui);
            row(ui, t("Źle", "Bad"), "", |ui| {
                color(ui, &mut th.bad);
            });
        });
        card(ui, Some("FPS"), |ui| {
            row(ui, t("Dobrze od", "Good from"), t("FPS, czas klatki i 1% / 0.1% low", "FPS, frametime and 1% / 0.1% low"), |ui| {
                ui.add(egui::DragValue::new(&mut th.fps_good).range(1.0..=1000.0).suffix(" FPS"));
            });
            divider(ui);
            row(ui, t("Uwaga od", "Warning from"), t("Poniżej — kolor „źle”", "Below this — “bad” colour"), |ui| {
                ui.add(egui::DragValue::new(&mut th.fps_warn).range(1.0..=1000.0).suffix(" FPS"));
            });
        });
        card(ui, Some(t("Obciążenie", "Load")), |ui| {
            row(ui, t("Uwaga od", "Warning from"), "CPU, GPU, VRAM, RAM", |ui| {
                ui.add(egui::Slider::new(&mut th.usage_warn, 10.0..=100.0).suffix(" %").step_by(1.0));
            });
            divider(ui);
            row(ui, t("Źle od", "Bad from"), "", |ui| {
                ui.add(egui::Slider::new(&mut th.usage_bad, 10.0..=100.0).suffix(" %").step_by(1.0));
            });
        });
        card(ui, Some(t("Temperatura GPU", "GPU temperature")), |ui| {
            row(ui, t("Uwaga od", "Warning from"), "", |ui| {
                ui.add(egui::Slider::new(&mut th.temp_warn, 30.0..=110.0).suffix(" °C").step_by(1.0));
            });
            divider(ui);
            row(ui, t("Źle od", "Bad from"), "", |ui| {
                ui.add(egui::Slider::new(&mut th.temp_bad, 30.0..=110.0).suffix(" °C").step_by(1.0));
            });
        });
        if th.fps_warn > th.fps_good {
            th.fps_warn = th.fps_good;
        }
        if th.usage_warn > th.usage_bad {
            th.usage_bad = th.usage_warn;
        }
        if th.temp_warn > th.temp_bad {
            th.temp_bad = th.temp_warn;
        }
    }

    fn hk_mut(&mut self, f: HkField) -> &mut String {
        let h = &mut self.cfg.hotkeys;
        match f {
            HkField::Toggle => &mut h.toggle,
            HkField::Mode => &mut h.mode,
            HkField::Settings => &mut h.settings,
            HkField::Move => &mut h.move_mode,
            HkField::Preset => &mut h.cycle_preset,
        }
    }

    fn set_capture(&mut self, f: Option<HkField>) {
        if self.capture.is_some() != f.is_some() {
            util::post_to_overlay(util::MSG_SUSPEND_HOTKEYS, f.is_some() as usize);
        }
        self.capture = f;
        self.capture_error = None;
    }

    fn handle_capture(&mut self, ctx: &egui::Context) {
        let Some(field) = self.capture else { return };
        let events = ctx.input(|i| i.events.clone());
        for ev in events {
            let egui::Event::Key { key, pressed: true, modifiers, .. } = ev else { continue };
            match key {
                egui::Key::Escape => {
                    self.set_capture(None);
                    return;
                }
                egui::Key::Backspace | egui::Key::Delete if !modifiers.any() => {
                    self.hk_mut(field).clear();
                    self.set_capture(None);
                    return;
                }
                _ => {}
            }
            let Some(vk) = key_from_name(key.name()) else { continue };
            let hk = Hotkey { ctrl: modifiers.ctrl, alt: modifiers.alt, shift: modifiers.shift, win: false, vk };
            let is_fkey = (0x70..=0x87).contains(&vk);
            if !(hk.ctrl || hk.alt || hk.shift || is_fkey) {
                self.capture_error = Some(self.t("Dodaj Ctrl, Alt lub Shift (albo użyj klawisza F1–F24)", "Add Ctrl, Alt or Shift (or use an F1–F24 key)").into());
                continue;
            }
            let s = hk.format();
            let h = &self.cfg.hotkeys;
            let others = [
                (HkField::Toggle, &h.toggle),
                (HkField::Mode, &h.mode),
                (HkField::Settings, &h.settings),
                (HkField::Move, &h.move_mode),
                (HkField::Preset, &h.cycle_preset),
            ];
            if others.iter().any(|(f, v)| *f != field && v.eq_ignore_ascii_case(&s)) {
                self.capture_error = Some(format!("{} {s}", self.t("Ten skrót jest już używany:", "Already in use:")));
                continue;
            }
            *self.hk_mut(field) = s;
            self.set_capture(None);
            return;
        }
    }

    fn page_hotkeys(&mut self, ui: &mut egui::Ui) {
        let l = self.cfg.general.language;
        let t = |pl, en| tr(l, pl, en);
        self.page_header(ui, t("Skróty klawiszowe", "Hotkeys"), t("Globalne skróty działają także w grach. Kliknij skrót, aby go zmienić.", "Global hotkeys work in games too. Click a hotkey to change it."));
        let fields = [
            (HkField::Toggle, t("Pokaż / ukryj nakładkę", "Show / hide overlay"), ""),
            (HkField::Mode, t("Tryb prosty ↔ szczegółowy", "Simple ↔ detailed mode"), ""),
            (HkField::Settings, t("Otwórz ustawienia", "Open settings"), ""),
            (HkField::Move, t("Przesuwanie myszą", "Move with mouse"), t("Włącz, przeciągnij nakładkę, wyłącz tym samym skrótem", "Turn on, drag the overlay, turn off with the same hotkey")),
            (HkField::Preset, t("Następny motyw", "Next theme"), t("Domyślnie nieprzypisany", "Unassigned by default")),
        ];
        card(ui, None, |ui| {
            let n = fields.len();
            for (i, (f, title, desc)) in fields.into_iter().enumerate() {
                let cur = self.hk_mut(f).clone();
                let active = self.capture == Some(f);
                let mut clicked = false;
                row(ui, title, desc, |ui| {
                    let text = if active {
                        t("Naciśnij skrót…", "Press keys…").to_string()
                    } else if cur.is_empty() {
                        t("— brak —", "— none —").to_string()
                    } else {
                        cur.clone()
                    };
                    clicked = kbd(ui, &text, active).clicked();
                });
                if clicked {
                    self.set_capture(if active { None } else { Some(f) });
                }
                if active {
                    let hint = self.capture_error.clone().unwrap_or_else(|| t("Esc — anuluj · Backspace — usuń skrót", "Esc — cancel · Backspace — clear").into());
                    let colr = if self.capture_error.is_some() { pal::WARN } else { pal::MUTED };
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        ui.label(RichText::new(hint).small().color(colr));
                    });
                }
                if i + 1 < n {
                    divider(ui);
                }
            }
        });
        card(ui, Some(t("Zasobnik systemowy", "System tray")), |ui| {
            ui.label(muted(t(
                "Kliknięcie ikony — pokaż / ukryj · dwuklik — ustawienia · prawy przycisk — menu z trybami, monitorami, motywami i językiem.",
                "Click the icon — show / hide · double-click — settings · right-click — menu with modes, displays, themes and language.",
            )));
        });
    }

    fn page_about(&mut self, ui: &mut egui::Ui) {
        let l = self.cfg.general.language;
        let t = |pl, en| tr(l, pl, en);
        self.page_header(ui, t("O programie", "About"), t("Lekka nakładka FPS i metryk systemowych dla Windows.", "A lightweight FPS and system metrics overlay for Windows."));
        card(ui, None, |ui| {
            ui.horizontal(|ui| {
                if let Some(lg) = &self.logo {
                    ui.add(egui::Image::new((lg.id(), Vec2::splat(56.0))));
                }
                ui.vertical(|ui| {
                    ui.label(semibold("OpenFPS", 22.0));
                    ui.label(muted(&format!("{} {} · Rust · Direct2D · ETW", t("Wersja", "Version"), crate::util::VERSION)));
                });
            });
            ui.add_space(10.0);
            ui.label(RichText::new(t(
                "FPS jest mierzony przez Event Tracing for Windows (jak w PresentMon) — bez wstrzykiwania kodu do gier, więc jest bezpieczny dla systemów anty-cheat. Działa z DirectX 9–12, OpenGL i Vulkanem. Nakładka rysowana jest programowo i tylko gdy zmieniają się dane, dlatego praktycznie nie obciąża procesora ani karty.",
                "FPS is measured via Event Tracing for Windows (like PresentMon) — nothing is injected into games, so it is anti-cheat safe. Works with DirectX 9–12, OpenGL and Vulkan. The overlay is software-rendered only when data changes, so it barely touches the CPU or GPU.",
            )).color(pal::MUTED));
        });

        self.updates_card(ui);

        card(ui, Some(t("Stan", "Status")), |ui| {
            row(ui, t("Uprawnienia", "Privileges"), t("Ustawienia nie wymagają administratora; nakładka — tylko jeśli konto nie może śledzić zdarzeń", "Settings don't need admin; the overlay only if your account can't trace events"), |ui| {
                if util::is_elevated() {
                    badge(ui, t("administrator", "administrator"), pal::GOOD);
                } else {
                    badge(ui, t("standardowe", "standard"), pal::MUTED);
                }
            });
            divider(ui);
            row(ui, t("Kontakt", "Contact"), t("Pytania, propozycje współpracy, zgłoszenia", "Questions, collaboration, reports"), |ui| {
                ui.hyperlink_to("kontakt@redzikowski.com", "mailto:kontakt@redzikowski.com");
            });
            divider(ui);
            row(ui, t("Plik konfiguracji", "Configuration file"), &Config::path().to_string_lossy(), |ui| {
                if button(ui, ico::FOLDER, t("Otwórz folder", "Open folder")).clicked() {
                    util::open_in_explorer(&Config::dir());
                }
            });
            divider(ui);
            row(ui, t("Ustawienia domyślne", "Default settings"), t("Przywraca wszystko poza językiem", "Restores everything except the language"), |ui| {
                if button(ui, ico::RESET, t("Przywróć…", "Restore…")).clicked() {
                    self.confirm_reset = true;
                }
            });
        });

        // Stopka autora — wspólna dla aplikacji publikowanych na redzikowski.com.
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(muted(t("Pomysł i wykonanie:", "Idea and execution:")));
            ui.hyperlink_to(RichText::new(AUTHOR_LINE).color(pal::TEXT), AUTHOR_URL);
            ui.label(muted("·"));
            ui.hyperlink_to(t("Postaw kawę", "Buy me a coffee"), SUPPORT_URL);
        });
    }

    fn reset_modal(&mut self, ctx: &egui::Context) {
        if !self.confirm_reset {
            return;
        }
        let l = self.cfg.general.language;
        let t = |pl, en| tr(l, pl, en);
        let mut close = false;
        let mut confirm = false;
        let m = egui::Modal::new(egui::Id::new("reset")).frame(
            egui::Frame::new().fill(pal::CARD).stroke(Stroke::new(1.0f32, pal::BORDER)).corner_radius(CornerRadius::same(12)).inner_margin(Margin::same(22)),
        );
        let resp = m.show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(semibold(t("Przywrócić ustawienia domyślne?", "Restore default settings?"), 17.0));
            ui.add_space(6.0);
            ui.label(muted(t("Wygląd, pozycje, monitory i skróty wrócą do wartości początkowych.", "Appearance, items, displays and hotkeys will be reset.")));
            ui.add_space(16.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if primary(ui, t("Przywróć", "Restore")).clicked() {
                    confirm = true;
                }
                if ui.button(t("Anuluj", "Cancel")).clicked() {
                    close = true;
                }
            });
        });
        if resp.should_close() {
            close = true;
        }
        if confirm {
            let lang = self.cfg.general.language;
            self.cfg = Config::default();
            self.cfg.general.language = lang;
            self.cfg.general.first_run = false;
            for mon in &self.monitors {
                self.cfg.monitor_mut(&mon.id, &mon.name, mon.primary);
            }
            self.excluded_text = self.cfg.general.excluded_processes.join("\n");
            self.toast(t("Przywrócono ustawienia domyślne", "Defaults restored"));
            close = true;
        }
        if close {
            self.confirm_reset = false;
        }
    }

    fn draw_toast(&mut self, ctx: &egui::Context) {
        let Some((msg, at)) = self.toast.clone() else { return };
        if at.elapsed() > Duration::from_millis(2200) {
            self.toast = None;
            return;
        }
        egui::Area::new(egui::Id::new("toast")).anchor(egui::Align2::CENTER_BOTTOM, Vec2::new(0.0, -24.0)).show(ctx, |ui| {
            egui::Frame::new().fill(pal::CONTROL_HI).stroke(Stroke::new(1.0f32, pal::BORDER)).corner_radius(CornerRadius::same(10)).inner_margin(Margin::symmetric(16, 10)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(icon(ico::CHECK, 13.0).color(pal::ACCENT));
                    ui.label(msg);
                });
            });
        });
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}

impl eframe::App for SettingsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll(ctx);
        self.handle_capture(ctx);

        egui::SidePanel::left("nav")
            .exact_width(232.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(pal::SIDEBAR).inner_margin(Margin::symmetric(14, 16)))
            .show(ctx, |ui| self.sidebar(ui));

        egui::CentralPanel::default().frame(egui::Frame::new().fill(pal::BG)).show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                egui::Frame::new().inner_margin(Margin { left: 32, right: 32, top: 26, bottom: 24 }).show(ui, |ui| {
                    let w = ui.available_width().min(820.0);
                    ui.set_max_width(w);
                    match self.page {
                        Page::General => self.page_general(ui),
                        Page::Appearance => self.page_appearance(ui),
                        Page::Simple => self.page_layout(ui, Mode::Simple),
                        Page::Detailed => self.page_layout(ui, Mode::Detailed),
                        Page::Displays => self.page_displays(ui),
                        Page::Thresholds => self.page_thresholds(ui),
                        Page::Hotkeys => self.page_hotkeys(ui),
                        Page::Feedback => self.page_feedback(ui),
                        Page::About => self.page_about(ui),
                    }
                });
            });
        });

        self.reset_modal(ctx);
        self.draw_toast(ctx);
        self.persist();
        ctx.request_repaint_after(Duration::from_millis(if self.dirty_since.is_some() { 60 } else { 1000 }));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if self.capture.is_some() {
            util::post_to_overlay(util::MSG_SUSPEND_HOTKEYS, 0);
        }
        if self.cfg != self.saved {
            self.dirty_since = Some(Instant::now() - Duration::from_secs(1));
            self.persist();
        }
    }
}

fn focus_existing() -> bool {
    for title in [w!("OpenFPS — Ustawienia"), w!("OpenFPS — Settings")] {
        if let Ok(h) = unsafe { FindWindowW(windows::core::PCWSTR::null(), title) } {
            if !h.is_invalid() {
                unsafe {
                    let _ = ShowWindow(h, SW_RESTORE);
                    let _ = SetForegroundWindow(h);
                }
                return true;
            }
        }
    }
    false
}

/// Okno otwarte przez nakładkę z prośbą o decyzję w sprawie raportu awarii.
fn crash_arg() -> bool {
    std::env::args().any(|a| a.eq_ignore_ascii_case("--page=crash"))
}

/// Strona startowa z argumentu `--page=<nazwa>` (przydatne m.in. do zrzutów ekranu).
fn start_page() -> Page {
    let arg = std::env::args().find_map(|a| a.strip_prefix("--page=").map(str::to_ascii_lowercase)).unwrap_or_default();
    match arg.as_str() {
        "appearance" => Page::Appearance,
        "simple" => Page::Simple,
        "detailed" => Page::Detailed,
        "displays" => Page::Displays,
        "thresholds" => Page::Thresholds,
        "hotkeys" => Page::Hotkeys,
        "feedback" | "crash" => Page::Feedback,
        "about" => Page::About,
        _ => Page::General,
    }
}

pub fn run() {
    let _mutex = unsafe { CreateMutexW(None, false, w!("Local\\OpenFPS.Settings")) };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS && focus_existing() {
        return;
    }
    let lang = Config::load().general.language;
    let title = tr(lang, TITLE_PL, TITLE_EN);
    let icon = egui::IconData { rgba: crate::icon::rgba(64), width: 64, height: 64 };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size([1060.0, 760.0])
            .with_min_inner_size([880.0, 560.0])
            .with_icon(Arc::new(icon)),
        centered: true,
        ..Default::default()
    };
    let _ = eframe::run_native(
        title,
        options,
        Box::new(|cc| {
            theme::install(&cc.egui_ctx);
            Ok(Box::new(SettingsApp::new(&cc.egui_ctx, start_page())))
        }),
    );
}
