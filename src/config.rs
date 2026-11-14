//! Konfiguracja aplikacji (TOML w %APPDATA%\OpenFPS\config.toml).
//! Wszystkie pola mają wartości domyślne, więc niepełny / starszy plik
//! zawsze się wczyta, a brakujące klucze zostaną uzupełnione.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::path::PathBuf;

// ───────────────────────────── Kolor ─────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Color(pub u8, pub u8, pub u8, pub u8);

impl Color {
    pub fn with_alpha(self, a: u8) -> Self {
        Self(self.0, self.1, self.2, a)
    }
    pub fn to_hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}{:02X}", self.0, self.1, self.2, self.3)
    }
    pub fn from_hex(s: &str) -> Option<Self> {
        let s = s.trim().trim_start_matches('#');
        let p = |i: usize| u8::from_str_radix(s.get(i..i + 2)?, 16).ok();
        match s.len() {
            6 => Some(Self(p(0)?, p(2)?, p(4)?, 255)),
            8 => Some(Self(p(0)?, p(2)?, p(4)?, p(6)?)),
            _ => None,
        }
    }
}

impl Serialize for Color {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}
impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Color::from_hex(&s).ok_or_else(|| serde::de::Error::custom(format!("invalid color: {s}")))
    }
}

// ───────────────────────────── Enumy ─────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Lang {
    #[default]
    Pl,
    En,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Simple,
    Detailed,
}

impl Mode {
    pub fn toggled(self) -> Self {
        match self {
            Mode::Simple => Mode::Detailed,
            Mode::Detailed => Mode::Simple,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ModeOverride {
    #[default]
    Inherit,
    Simple,
    Detailed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    #[default]
    Vertical,
    Horizontal,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    #[default]
    TopLeft,
    TopCenter,
    TopRight,
    MiddleLeft,
    Center,
    MiddleRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl Anchor {
    pub const ALL: [Anchor; 9] = [
        Anchor::TopLeft,
        Anchor::TopCenter,
        Anchor::TopRight,
        Anchor::MiddleLeft,
        Anchor::Center,
        Anchor::MiddleRight,
        Anchor::BottomLeft,
        Anchor::BottomCenter,
        Anchor::BottomRight,
    ];
    /// (kolumna, wiersz) w siatce 3×3: 0 = lewo/góra, 1 = środek, 2 = prawo/dół.
    pub fn grid(self) -> (u8, u8) {
        let i = Self::ALL.iter().position(|a| *a == self).unwrap_or(0) as u8;
        (i % 3, i / 3)
    }
    pub fn from_grid(col: u8, row: u8) -> Self {
        Self::ALL[(row.min(2) * 3 + col.min(2)) as usize]
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FontWeight {
    Regular,
    Medium,
    #[default]
    Semibold,
    Bold,
}

impl FontWeight {
    pub fn value(self) -> i32 {
        match self {
            FontWeight::Regular => 400,
            FontWeight::Medium => 500,
            FontWeight::Semibold => 600,
            FontWeight::Bold => 700,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TextEffect {
    #[default]
    None,
    Shadow,
    Outline,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TempUnit {
    #[default]
    Celsius,
    Fahrenheit,
}

/// Wszystkie metryki, które może pokazać nakładka.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    Fps,
    Frametime,
    Low1,
    Low01,
    FrametimeGraph,
    FpsGraph,
    PlayTime,
    CpuUsage,
    CpuClock,
    CpuCores,
    GpuUsage,
    GpuAppUsage,
    GpuTemp,
    GpuClock,
    GpuPower,
    GpuFan,
    Vram,
    Ram,
    Network,
    Disk,
    Battery,
    Display,
    Clock,
    Uptime,
    /// Metryka nieznana tej wersji (np. usunięta „app”) — wczytywana bez błędu i pomijana.
    #[serde(other)]
    Unknown,
}

impl Metric {
    pub const ALL: [Metric; 24] = [
        Metric::Fps,
        Metric::Frametime,
        Metric::Low1,
        Metric::Low01,
        Metric::FrametimeGraph,
        Metric::FpsGraph,
        Metric::PlayTime,
        Metric::CpuUsage,
        Metric::CpuClock,
        Metric::CpuCores,
        Metric::GpuUsage,
        Metric::GpuAppUsage,
        Metric::GpuTemp,
        Metric::GpuClock,
        Metric::GpuPower,
        Metric::GpuFan,
        Metric::Vram,
        Metric::Ram,
        Metric::Network,
        Metric::Disk,
        Metric::Battery,
        Metric::Display,
        Metric::Clock,
        Metric::Uptime,
    ];
}

// ───────────────────────────── Sekcje ─────────────────────────────

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    pub language: Lang,
    pub mode: Mode,
    pub visible: bool,
    /// Odświeżanie FPS / czasu klatki (ms).
    pub fps_refresh_ms: u32,
    /// Odświeżanie metryk systemowych (ms).
    pub sys_refresh_ms: u32,
    /// Okno uśredniania FPS (ms).
    pub fps_average_ms: u32,
    /// Okno liczenia 1% / 0.1% low (s).
    pub lows_window_s: u32,
    /// Zakres czasu na wykresach (s).
    pub graph_seconds: u32,
    pub hide_from_capture: bool,
    pub auto_hide_without_app: bool,
    pub autostart: bool,
    pub run_as_admin: bool,
    pub temperature_unit: TempUnit,
    pub new_monitors_enabled: bool,
    pub excluded_processes: Vec<String>,
    /// Raz dziennie sprawdzaj nowe wersje na redzikowski.com.
    pub check_updates: bool,
    pub first_run: bool,
}

impl Default for General {
    fn default() -> Self {
        Self {
            language: Lang::Pl,
            mode: Mode::Simple,
            visible: true,
            fps_refresh_ms: 250,
            sys_refresh_ms: 1000,
            fps_average_ms: 1000,
            lows_window_s: 10,
            graph_seconds: 5,
            hide_from_capture: false,
            auto_hide_without_app: false,
            autostart: false,
            run_as_admin: false,
            temperature_unit: TempUnit::Celsius,
            new_monitors_enabled: false,
            excluded_processes: vec![
                "dwm.exe".into(),
                "explorer.exe".into(),
                "ShellExperienceHost.exe".into(),
                "StartMenuExperienceHost.exe".into(),
                "SearchHost.exe".into(),
                "TextInputHost.exe".into(),
                "LockApp.exe".into(),
            ],
            check_updates: true,
            first_run: true,
        }
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Style {
    pub font_family: String,
    pub font_weight: FontWeight,
    /// Rozmiar wartości w DIP (przy skali 100%).
    pub font_size: f32,
    /// Rozmiar etykiet względem wartości.
    pub label_scale: f32,
    /// Globalna skala nakładki (1.0 = 100%).
    pub scale: f32,
    pub background: Color,
    pub text: Color,
    pub label: Color,
    pub accent: Color,
    pub bar_background: Color,
    pub graph: Color,
    pub border: bool,
    pub border_color: Color,
    pub border_width: f32,
    pub corner_radius: f32,
    pub padding: f32,
    pub row_spacing: f32,
    pub item_spacing: f32,
    pub text_effect: TextEffect,
    pub effect_color: Color,
    pub colorize: bool,
    pub show_bars: bool,
    pub bar_width: f32,
    pub bar_height: f32,
    pub graph_height: f32,
    pub graph_width: f32,
}

impl Default for Style {
    fn default() -> Self {
        crate::presets::graphite()
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Item {
    pub metric: Metric,
    pub enabled: bool,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ModeLayout {
    pub orientation: Orientation,
    pub show_labels: bool,
    pub show_header: bool,
    pub items: Vec<Item>,
}

impl ModeLayout {
    fn with(orientation: Orientation, show_header: bool, enabled: &[Metric]) -> Self {
        // Kolejność: najpierw włączone (w podanej kolejności), potem reszta.
        let mut items: Vec<Item> = enabled.iter().map(|m| Item { metric: *m, enabled: true }).collect();
        for m in Metric::ALL {
            if !enabled.contains(&m) {
                items.push(Item { metric: m, enabled: false });
            }
        }
        Self { orientation, show_labels: true, show_header, items }
    }
    pub fn default_simple() -> Self {
        Self::with(Orientation::Horizontal, false, &[Metric::Fps, Metric::Frametime])
    }
    pub fn default_detailed() -> Self {
        Self::with(
            Orientation::Vertical,
            true,
            &[
                Metric::Fps,
                Metric::Frametime,
                Metric::Low1,
                Metric::Low01,
                Metric::FrametimeGraph,
                Metric::CpuUsage,
                Metric::CpuClock,
                Metric::GpuUsage,
                Metric::GpuTemp,
                Metric::GpuClock,
                Metric::GpuPower,
                Metric::Vram,
                Metric::Ram,
                Metric::Clock,
            ],
        )
    }
    /// Uzupełnia listę o metryki dodane w nowszych wersjach i usuwa duplikaty.
    pub fn normalize(&mut self) {
        let mut seen = std::collections::HashSet::new();
        self.items.retain(|i| i.metric != Metric::Unknown && seen.insert(i.metric));
        for m in Metric::ALL {
            if !seen.contains(&m) {
                self.items.push(Item { metric: m, enabled: false });
            }
        }
    }
    pub fn enabled(&self) -> impl Iterator<Item = Metric> + '_ {
        self.items.iter().filter(|i| i.enabled).map(|i| i.metric)
    }
}

impl Default for ModeLayout {
    fn default() -> Self {
        Self::default_simple()
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Thresholds {
    pub fps_good: f32,
    pub fps_warn: f32,
    pub usage_warn: f32,
    pub usage_bad: f32,
    pub temp_warn: f32,
    pub temp_bad: f32,
    pub good: Color,
    pub warn: Color,
    pub bad: Color,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            fps_good: 60.0,
            fps_warn: 30.0,
            usage_warn: 75.0,
            usage_bad: 92.0,
            temp_warn: 72.0,
            temp_bad: 85.0,
            good: Color(0x4A, 0xDE, 0x80, 0xFF),
            warn: Color(0xFA, 0xCC, 0x15, 0xFF),
            bad: Color(0xF8, 0x71, 0x71, 0xFF),
        }
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkeys {
    pub toggle: String,
    pub mode: String,
    pub settings: String,
    pub move_mode: String,
    pub cycle_preset: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            toggle: "Ctrl+Shift+F9".into(),
            mode: "Ctrl+Shift+F10".into(),
            settings: "Ctrl+Shift+F11".into(),
            move_mode: "Ctrl+Shift+F12".into(),
            cycle_preset: String::new(),
        }
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct MonitorCfg {
    /// Nazwa urządzenia GDI, np. `\\.\DISPLAY1`.
    pub id: String,
    /// Przyjazna nazwa (tylko informacyjnie).
    pub name: String,
    pub enabled: bool,
    pub anchor: Anchor,
    pub margin_x: f32,
    pub margin_y: f32,
    pub mode: ModeOverride,
    pub scale: f32,
    pub use_work_area: bool,
}

impl Default for MonitorCfg {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            enabled: true,
            anchor: Anchor::TopLeft,
            margin_x: 16.0,
            margin_y: 16.0,
            mode: ModeOverride::Inherit,
            scale: 1.0,
            use_work_area: true,
        }
    }
}

// ───────────────────────────── Całość ─────────────────────────────

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub general: General,
    pub style: Style,
    pub preset: String,
    pub simple: ModeLayout,
    pub detailed: ModeLayout,
    pub thresholds: Thresholds,
    pub hotkeys: Hotkeys,
    pub monitors: Vec<MonitorCfg>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            general: General::default(),
            style: Style::default(),
            preset: "graphite".into(),
            simple: ModeLayout::default_simple(),
            detailed: ModeLayout::default_detailed(),
            thresholds: Thresholds::default(),
            hotkeys: Hotkeys::default(),
            monitors: Vec::new(),
        }
    }
}

impl Config {
    pub fn dir() -> PathBuf {
        let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        base.join("OpenFPS")
    }
    pub fn path() -> PathBuf {
        Self::dir().join("config.toml")
    }

    /// Wczytuje konfigurację; przy błędzie zwraca domyślną (i zachowuje kopię uszkodzonego pliku).
    pub fn load() -> Self {
        let path = Self::path();
        let mut cfg = match std::fs::read_to_string(&path) {
            Ok(s) => match toml::from_str::<Config>(&s) {
                Ok(c) => c,
                Err(_) => {
                    let _ = std::fs::copy(&path, path.with_extension("toml.broken"));
                    Config::default()
                }
            },
            Err(_) => Config::default(),
        };
        cfg.sanitize();
        cfg
    }

    pub fn save(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(Self::dir())?;
        let s = toml::to_string_pretty(self).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        // Zapis atomowy: najpierw plik tymczasowy, potem podmiana.
        let tmp = Self::path().with_extension("toml.tmp");
        std::fs::write(&tmp, s)?;
        std::fs::rename(&tmp, Self::path())
    }

    pub fn sanitize(&mut self) {
        self.simple.normalize();
        self.detailed.normalize();
        let g = &mut self.general;
        g.fps_refresh_ms = g.fps_refresh_ms.clamp(50, 2000);
        g.sys_refresh_ms = g.sys_refresh_ms.clamp(250, 5000);
        g.fps_average_ms = g.fps_average_ms.clamp(250, 5000);
        g.lows_window_s = g.lows_window_s.clamp(2, 120);
        g.graph_seconds = g.graph_seconds.clamp(1, 60);
        let s = &mut self.style;
        s.font_size = s.font_size.clamp(8.0, 72.0);
        s.label_scale = s.label_scale.clamp(0.5, 1.2);
        s.scale = s.scale.clamp(0.5, 3.0);
        s.padding = s.padding.clamp(0.0, 40.0);
        s.corner_radius = s.corner_radius.clamp(0.0, 40.0);
        s.border_width = s.border_width.clamp(0.5, 6.0);
        s.row_spacing = s.row_spacing.clamp(0.0, 30.0);
        s.item_spacing = s.item_spacing.clamp(0.0, 60.0);
        s.bar_width = s.bar_width.clamp(20.0, 300.0);
        s.bar_height = s.bar_height.clamp(2.0, 20.0);
        s.graph_height = s.graph_height.clamp(16.0, 240.0);
        s.graph_width = s.graph_width.clamp(80.0, 800.0);
        if s.font_family.trim().is_empty() {
            s.font_family = "Segoe UI".into();
        }
        for m in &mut self.monitors {
            m.scale = m.scale.clamp(0.5, 3.0);
        }
    }

    /// Zwraca konfigurację monitora; jeśli to nowy monitor — dodaje wpis.
    pub fn monitor_mut(&mut self, id: &str, name: &str, primary: bool) -> &mut MonitorCfg {
        if let Some(i) = self.monitors.iter().position(|m| m.id == id) {
            if !name.is_empty() {
                self.monitors[i].name = name.to_string();
            }
            return &mut self.monitors[i];
        }
        self.monitors.push(MonitorCfg {
            id: id.to_string(),
            name: name.to_string(),
            enabled: primary || self.general.new_monitors_enabled,
            ..Default::default()
        });
        self.monitors.last_mut().unwrap()
    }

    pub fn monitor(&self, id: &str) -> Option<&MonitorCfg> {
        self.monitors.iter().find(|m| m.id == id)
    }

    pub fn layout(&self, mode: Mode) -> &ModeLayout {
        match mode {
            Mode::Simple => &self.simple,
            Mode::Detailed => &self.detailed,
        }
    }
}
