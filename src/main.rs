//! OpenFPS — lekka, w pełni konfigurowalna nakładka FPS i metryk systemowych dla Windows.
//! Pomysł i wykonanie: Adam Rędzikowski · redzikowski.com
//!
//! Uruchomienie bez argumentów: nakładka w tle (ikona w zasobniku).
//! `--settings [--page=<strona>]`: okno ustawień (osobny proces, edytuje config.toml na żywo).

#![windows_subsystem = "windows"]

mod config;
mod feedback;
mod hotkey;
mod i18n;
mod icon;
mod metrics;
mod monitors;
mod overlay;
mod presets;
mod settings;
mod update;
mod util;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |f: &str| args.iter().any(|a| a.eq_ignore_ascii_case(f));
    update::cleanup();
    if has("--settings") {
        feedback::install_panic_hook("settings");
        settings::run();
    } else {
        feedback::install_panic_hook("overlay");
        overlay::run(has("--relaunched"), has("--no-elevate"));
    }
}
