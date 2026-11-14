//! Integracje z redzikowski.com w oknie ustawień: zgłoszenia (błąd, sugestia, awaria)
//! oraz aktualizacje. Operacje sieciowe idą w osobnych wątkach, a UI tylko odczytuje stan.

use std::sync::{Arc, Mutex};
use eframe::egui::{self, CornerRadius, Margin, RichText, Stroke, Vec2};

use super::theme::{ico, pal};
use super::widgets::*;
use super::SettingsApp;
use crate::feedback::{self, Kind, Report};
use crate::i18n::tr;
use crate::update::{self, Release};
use crate::util;

// ───────────────────────────── Zgłoszenia ─────────────────────────────

#[derive(Clone, PartialEq)]
pub enum SendState {
    Idle,
    Sending,
    Sent,
    Error(String),
}

pub struct FeedbackForm {
    pub kind: Kind,
    pub message: String,
    pub email: String,
    pub include_device: bool,
    /// Opis awarii z poprzedniego uruchomienia (tryb raportu awarii).
    pub crash: Option<String>,
    pub state: Arc<Mutex<SendState>>,
}

impl FeedbackForm {
    pub fn new(crash: Option<String>) -> Self {
        Self {
            kind: if crash.is_some() { Kind::Crash } else { Kind::Bug },
            message: String::new(),
            email: String::new(),
            include_device: true,
            crash,
            state: Arc::new(Mutex::new(SendState::Idle)),
        }
    }
}

// ───────────────────────────── Aktualizacje ─────────────────────────────

#[derive(Clone)]
pub enum UpdState {
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Downloading(f32),
    Installed(String),
    Error(String),
}

pub fn spawn_check(state: Arc<Mutex<UpdState>>, ctx: egui::Context) {
    *state.lock().unwrap() = UpdState::Checking;
    std::thread::spawn(move || {
        let r = match update::check() {
            Ok(Some(rel)) => UpdState::Available(rel),
            Ok(None) => UpdState::UpToDate,
            Err(e) => UpdState::Error(e),
        };
        *state.lock().unwrap() = r;
        ctx.request_repaint();
    });
}

fn spawn_install(state: Arc<Mutex<UpdState>>, rel: Release, ctx: egui::Context) {
    *state.lock().unwrap() = UpdState::Downloading(0.0);
    std::thread::spawn(move || {
        let st = state.clone();
        let c = ctx.clone();
        let r = update::install(&rel, move |done, total| {
            *st.lock().unwrap() = UpdState::Downloading(if total > 0 { done as f32 / total as f32 } else { 0.0 });
            c.request_repaint();
        });
        *state.lock().unwrap() = match r {
            Ok(()) => UpdState::Installed(rel.version.clone()),
            Err(e) => UpdState::Error(e),
        };
        ctx.request_repaint();
    });
}

/// Uruchamia nową wersję: zamyka bieżącą nakładkę i startuje ją z podmienionego pliku.
fn restart_after_update(ctx: &egui::Context) {
    let was_running = util::overlay_running();
    if was_running {
        util::post_to_overlay(util::MSG_QUIT, 0);
    }
    util::spawn_self(&["--relaunched"]);
    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
}

impl SettingsApp {
    pub(super) fn page_feedback(&mut self, ui: &mut egui::Ui) {
        let l = self.cfg.general.language;
        let t = |pl, en| tr(l, pl, en);
        let crash_mode = self.fb.crash.is_some();
        if crash_mode {
            self.page_header(ui, t("Raport o awarii", "Crash report"), t("OpenFPS zakończył się nieoczekiwanie przy poprzednim uruchomieniu.", "OpenFPS closed unexpectedly the last time it ran."));
        } else {
            self.page_header(ui, t("Zgłoś błąd lub sugestię", "Report a bug or suggestion"), t("Wiadomość trafi bezpośrednio do autora przez redzikowski.com.", "Your message goes straight to the author via redzikowski.com."));
        }
        let state = self.fb.state.lock().unwrap().clone();

        if state == SendState::Sent {
            card(ui, None, |ui| {
                ui.horizontal(|ui| {
                    ui.label(icon(ico::CHECK, 18.0).color(pal::GOOD));
                    ui.label(semibold(t("Dziękuję — zgłoszenie wysłane.", "Thank you — your report was sent."), 16.0));
                });
                ui.add_space(4.0);
                ui.label(muted(t("Jeśli podałeś adres e-mail, możesz dostać odpowiedź.", "If you left an e-mail address, you may get a reply.")));
                ui.add_space(10.0);
                if button(ui, "", t("Wyślij kolejne", "Send another")).clicked() {
                    self.fb = FeedbackForm::new(None);
                }
            });
            return;
        }

        let busy = state == SendState::Sending;
        let fb = &mut self.fb;
        card(ui, None, |ui| {
            ui.add_enabled_ui(!busy, |ui| {
                if let Some(stack) = &fb.crash {
                    ui.label(muted(t(
                        "Raport zawiera opis błędu, wersję programu i systemu. Nie zawiera Twoich plików ani nazw gier. Wysłanie pomoże go naprawić — wyślesz go tylko, jeśli chcesz.",
                        "The report contains the error description and program and system versions. It does not include your files or game names. Sending it helps fix the bug — only if you want to.",
                    )));
                    ui.add_space(8.0);
                    egui::CollapsingHeader::new(t("Szczegóły techniczne", "Technical details")).show(ui, |ui| {
                        egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                            ui.label(RichText::new(stack.as_str()).monospace().size(11.0).color(pal::MUTED));
                        });
                    });
                    ui.add_space(8.0);
                } else {
                    row(ui, t("Rodzaj", "Type"), "", |ui| {
                        segmented(ui, &mut fb.kind, &[(Kind::Bug, t("Błąd", "Bug")), (Kind::Feature, t("Sugestia", "Suggestion"))]);
                    });
                    divider(ui);
                }
                let (label, hint) = match fb.kind {
                    Kind::Crash => (t("Co robiłeś przed awarią? (opcjonalnie)", "What were you doing before the crash? (optional)"), ""),
                    Kind::Bug => (
                        t("Opis", "Description"),
                        t("Co się stało, czego się spodziewałeś i jak to powtórzyć?", "What happened, what did you expect and how to reproduce it?"),
                    ),
                    Kind::Feature => (t("Opis", "Description"), t("Czego brakuje i do czego by się przydało?", "What's missing and what would it help with?")),
                };
                ui.label(RichText::new(label).color(pal::TEXT));
                ui.add_space(2.0);
                ui.add(
                    egui::TextEdit::multiline(&mut fb.message)
                        .hint_text(hint)
                        .desired_rows(7)
                        .desired_width(f32::INFINITY)
                        .margin(Margin::same(10)),
                );
                ui.add_space(10.0);
                ui.label(RichText::new(t("E-mail do odpowiedzi (opcjonalnie)", "E-mail for a reply (optional)")).color(pal::TEXT));
                ui.add_space(2.0);
                ui.add(egui::TextEdit::singleline(&mut fb.email).hint_text("adres@przyklad.pl").desired_width(320.0).margin(Margin::symmetric(10, 6)));
                if !feedback::email_ok(&fb.email) {
                    ui.label(RichText::new(t("To nie wygląda na adres e-mail.", "That doesn't look like an e-mail address.")).small().color(pal::WARN));
                }
                if fb.crash.is_none() {
                    ui.add_space(6.0);
                    divider(ui);
                    row(
                        ui,
                        t("Dołącz informacje o systemie", "Include system information"),
                        t("Wersja Windows, liczba wątków CPU i model karty graficznej", "Windows version, CPU thread count and graphics card model"),
                        |ui| {
                            toggle(ui, &mut fb.include_device);
                        },
                    );
                }
            });

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let can_send = !busy
                    && feedback::email_ok(&fb.email)
                    && (fb.kind == Kind::Crash || fb.message.trim().chars().count() >= 3);
                let label = if fb.crash.is_some() { t("Wyślij raport", "Send report") } else { t("Wyślij", "Send") };
                if ui.add_enabled(can_send, egui::Button::new(RichText::new(label).color(pal::ON_ACCENT).strong()).fill(pal::ACCENT).stroke(Stroke::NONE).min_size(Vec2::new(0.0, 30.0))).clicked() {
                    let report = Report {
                        kind: fb.kind,
                        message: fb.message.clone(),
                        email: fb.email.clone(),
                        include_device: fb.include_device,
                        stack: fb.crash.clone(),
                        lang: l,
                        device: crate::metrics::gpu_name(),
                    };
                    let st = fb.state.clone();
                    let ctx = ui.ctx().clone();
                    *st.lock().unwrap() = SendState::Sending;
                    std::thread::spawn(move || {
                        let r = feedback::send(&report);
                        *st.lock().unwrap() = match r {
                            Ok(()) => SendState::Sent,
                            Err(e) => SendState::Error(e),
                        };
                        ctx.request_repaint();
                    });
                }
                if fb.crash.is_some() && !busy && ui.button(t("Nie wysyłaj", "Don't send")).clicked() {
                    feedback::discard_crash();
                    *fb = FeedbackForm::new(None);
                }
                if busy {
                    ui.add(egui::Spinner::new().size(16.0).color(pal::ACCENT));
                    ui.label(muted(t("Wysyłanie…", "Sending…")));
                }
            });
            if let SendState::Error(e) = &state {
                ui.add_space(6.0);
                ui.label(RichText::new(e.as_str()).color(pal::BAD));
            }
        });
    }

    pub(super) fn updates_card(&mut self, ui: &mut egui::Ui) {
        let l = self.cfg.general.language;
        let t = |pl, en| tr(l, pl, en);
        let state = self.upd.lock().unwrap().clone();
        card(ui, Some(t("Aktualizacje", "Updates")), |ui| {
            row(ui, t("Sprawdzaj automatycznie", "Check automatically"), t("Raz dziennie, przez redzikowski.com", "Once a day, via redzikowski.com"), |ui| {
                toggle(ui, &mut self.cfg.general.check_updates);
            });
            divider(ui);
            let (status, color) = match &state {
                UpdState::Idle => (format!("{} {}", t("Zainstalowana wersja", "Installed version"), util::VERSION), pal::MUTED),
                UpdState::Checking => (t("Sprawdzanie…", "Checking…").to_string(), pal::MUTED),
                UpdState::UpToDate => (format!("{} ({})", t("Masz najnowszą wersję", "You're up to date"), util::VERSION), pal::GOOD),
                UpdState::Available(r) => (format!("{} {}", t("Dostępna jest wersja", "Version available:"), r.version), pal::ACCENT),
                UpdState::Downloading(p) => (format!("{} {:.0}%", t("Pobieranie…", "Downloading…"), p * 100.0), pal::MUTED),
                UpdState::Installed(v) => (format!("{} {v}. {}", t("Zainstalowano wersję", "Installed version"), t("Uruchom ponownie, aby jej używać.", "Restart to use it.")), pal::GOOD),
                UpdState::Error(e) => (format!("{}: {e}", t("Nie udało się", "Failed")), pal::BAD),
            };
            let ctx = ui.ctx().clone();
            row(ui, t("Stan", "Status"), "", |ui| match &state {
                UpdState::Available(rel) => {
                    if primary(ui, t("Pobierz i zainstaluj", "Download and install")).clicked() {
                        spawn_install(self.upd.clone(), rel.clone(), ctx.clone());
                    }
                }
                UpdState::Installed(_) => {
                    if primary(ui, t("Uruchom ponownie", "Restart")).clicked() {
                        restart_after_update(&ctx);
                    }
                }
                UpdState::Checking | UpdState::Downloading(_) => {
                    ui.add(egui::Spinner::new().size(16.0).color(pal::ACCENT));
                }
                _ => {
                    if button(ui, ico::RESET, t("Sprawdź teraz", "Check now")).clicked() {
                        spawn_check(self.upd.clone(), ctx.clone());
                    }
                }
            });
            ui.label(RichText::new(status).color(color));
            if let UpdState::Downloading(p) = state {
                ui.add_space(4.0);
                ui.add(egui::ProgressBar::new(p).desired_height(6.0).fill(pal::ACCENT).corner_radius(CornerRadius::same(3)));
            }
        });
    }
}
