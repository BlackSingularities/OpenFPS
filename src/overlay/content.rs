//! Zamiana danych (metryki + konfiguracja) na bloki do narysowania.
//!
//! Zasada stabilności: każda pozycja ma zawsze tę samą strukturę i szablon
//! szerokości — niezależnie od tego, czy dane są dostępne („—”) i jakie mają
//! wartości. Dzięki temu nakładka nie zmienia rozmiaru w trakcie działania.
//! Nakładka celowo nie pokazuje nazwy mierzonej aplikacji.

use std::collections::VecDeque;

use crate::config::{Color, Config, Metric, Mode, Orientation, TempUnit};
use crate::i18n::{short_label, tr};
use crate::metrics::etw::EtwStatus;
use crate::metrics::{fmt, AppInfo, SysSnapshot};
use crate::monitors::MonitorInfo;

use super::render::{Block, Frame, Span};

pub struct Ctx<'a> {
    pub cfg: &'a Config,
    pub mode: Mode,
    pub sys: &'a SysSnapshot,
    pub app: Option<&'a AppInfo>,
    pub mon: &'a MonitorInfo,
    pub fps_hist: &'a VecDeque<f32>,
    pub etw: EtwStatus,
    pub edit: bool,
    pub clock: (u16, u16),
}

const DASH: &str = "—";
const T_DURATION: &str = "88:88:88";
const T_RATE: &str = "888.8";
const T_RATE_UNIT: &str = "MB/s";

impl<'a> Ctx<'a> {
    fn fps_color(&self, v: f32) -> Color {
        let t = &self.cfg.thresholds;
        if !self.cfg.style.colorize {
            return self.cfg.style.text;
        }
        if v >= t.fps_good {
            t.good
        } else if v >= t.fps_warn {
            t.warn
        } else {
            t.bad
        }
    }
    /// (kolor tekstu, kolor paska) dla obciążenia w %.
    fn usage_color(&self, v: f32) -> (Color, Color) {
        let t = &self.cfg.thresholds;
        let s = &self.cfg.style;
        if s.colorize && v >= t.usage_bad {
            (t.bad, t.bad)
        } else if s.colorize && v >= t.usage_warn {
            (t.warn, t.warn)
        } else {
            (s.text, s.accent)
        }
    }
    fn temp_color(&self, c: f32) -> Color {
        let t = &self.cfg.thresholds;
        if !self.cfg.style.colorize {
            self.cfg.style.text
        } else if c >= t.temp_bad {
            t.bad
        } else if c >= t.temp_warn {
            t.warn
        } else {
            self.cfg.style.text
        }
    }
    fn muted(&self) -> Color {
        self.cfg.style.label
    }
    /// Wartość albo „—” w tym samym slocie szerokości.
    fn val(&self, v: Option<String>, color: Color, template: &'static str) -> Span {
        match v {
            Some(s) => Span::value(s, color, template),
            None => Span::value(DASH, self.muted(), template),
        }
    }
    fn unit(&self, u: &str) -> Span {
        Span::unit(u, self.muted())
    }
}

/// Czas klatki zawsze mieści się w szablonie „88.8”: od 100 ms bez części dziesiętnej.
fn fmt_ms(v: f32) -> String {
    if v < 99.95 { format!("{:.1}", v) } else { format!("{:.0}", v.min(999.0)) }
}

fn downsample_max(v: &[f32], max_points: usize) -> Vec<f32> {
    if v.len() <= max_points {
        return v.to_vec();
    }
    let bucket = v.len() as f32 / max_points as f32;
    (0..max_points)
        .map(|i| {
            let a = (i as f32 * bucket) as usize;
            let b = (((i + 1) as f32 * bucket) as usize).min(v.len()).max(a + 1);
            v[a..b].iter().cloned().fold(0.0, f32::max)
        })
        .collect()
}

pub fn build(c: &Ctx) -> Frame {
    let cfg = c.cfg;
    let lang = cfg.general.language;
    let layout = cfg.layout(c.mode);
    let style = &cfg.style;
    let vertical = layout.orientation == Orientation::Vertical;
    let text = style.text;
    let sys = c.sys;
    let app = c.app;
    let mut blocks = Vec::new();

    if layout.show_header {
        let subtitle = if vertical {
            format!("{} · {}×{} · {} Hz", c.mon.name, c.mon.width(), c.mon.height(), c.mon.refresh_hz)
        } else {
            String::new()
        };
        blocks.push(Block::Header { title: "OpenFPS".into(), subtitle, color: style.accent });
    }

    let mut uses_fps = false;
    for m in layout.enabled() {
        let label = short_label(m, lang).to_string();
        let row = |spans: Vec<Span>| Block::Row { label: label.clone(), spans, bar: None };
        // Wiersze z paskiem mają go zawsze (pusty, gdy brak danych) — stała szerokość.
        let row_bar = |spans: Vec<Span>, frac: Option<f32>, col: Color| Block::Row {
            label: label.clone(),
            spans,
            bar: Some((frac.unwrap_or(0.0), col)),
        };
        let usage = |v: Option<f32>| -> Block {
            let (tc, bc) = v.map(|v| c.usage_color(v)).unwrap_or((text, style.accent));
            row_bar(vec![c.val(v.map(|v| format!("{:.0}", v)), tc, "888"), c.unit("%")], v.map(|v| v / 100.0), bc)
        };
        use Metric::*;
        let block = match m {
            Fps => {
                uses_fps = true;
                let v = app.map(|a| format!("{:.0}", a.stats.fps.min(9999.0)));
                let col = app.map(|a| c.fps_color(a.stats.fps)).unwrap_or(text);
                let mut span = c.val(v, col, "8888");
                if vertical {
                    span = Span::big(span.text, span.color, "8888");
                }
                row(vec![span])
            }
            Frametime => {
                uses_fps = true;
                let col = app.map(|a| c.fps_color(a.stats.fps)).unwrap_or(text);
                row(vec![c.val(app.map(|a| fmt_ms(a.stats.frametime_ms)), col, "88.8"), c.unit("ms")])
            }
            Low1 | Low01 => {
                uses_fps = true;
                let v = app.map(|a| if m == Low1 { a.stats.low1 } else { a.stats.low01 });
                row(vec![c.val(v.map(|v| format!("{:.0}", v.min(9999.0))), v.map(|v| c.fps_color(v)).unwrap_or(text), "8888")])
            }
            FrametimeGraph => {
                uses_fps = true;
                let hist = app.map(|a| a.stats.history.as_slice()).unwrap_or(&[]);
                let values = downsample_max(hist, 240);
                let peak = values.iter().cloned().fold(0.0, f32::max);
                let avg = app.map(|a| a.stats.frametime_ms).unwrap_or(0.0);
                Block::Graph {
                    label,
                    caption: if peak > 0.0 { format!("max {:.1} ms", peak) } else { String::new() },
                    max: (peak * 1.15).max(avg * 2.0).max(4.0),
                    values,
                    color: style.graph,
                    guide: (avg > 0.0).then_some(avg),
                }
            }
            FpsGraph => {
                uses_fps = true;
                let values: Vec<f32> = c.fps_hist.iter().cloned().collect();
                let peak = values.iter().cloned().fold(0.0, f32::max);
                let low = values.iter().cloned().filter(|v| *v > 0.0).fold(f32::MAX, f32::min);
                Block::Graph {
                    label,
                    caption: if peak > 0.0 { format!("{:.0}–{:.0}", low.min(peak), peak) } else { String::new() },
                    max: (peak * 1.15).max(30.0),
                    values,
                    color: style.graph,
                    guide: None,
                }
            }
            PlayTime => {
                uses_fps = true;
                row(vec![c.val(app.map(|a| fmt::duration(a.stats.session_s as u64)), text, T_DURATION)])
            }
            CpuUsage => usage(sys.cpu),
            CpuClock => row(vec![c.val(sys.cpu_mhz.map(|v| format!("{:.2}", v / 1000.0)), text, "8.88"), c.unit("GHz")]),
            CpuCores => {
                let values = sys.cores.iter().map(|v| (*v, c.usage_color(*v).1)).collect();
                Block::Cores { label, values }
            }
            GpuUsage => usage(sys.gpu),
            GpuAppUsage => {
                uses_fps = true;
                usage(app.and_then(|a| sys.gpu_per_pid.get(&a.pid)).copied())
            }
            GpuTemp => {
                let (conv, u) = match cfg.general.temperature_unit {
                    TempUnit::Celsius => (sys.gpu_temp.map(|t| t as f32), "°C"),
                    TempUnit::Fahrenheit => (sys.gpu_temp.map(|t| t as f32 * 1.8 + 32.0), "°F"),
                };
                let col = sys.gpu_temp.map(|t| c.temp_color(t as f32)).unwrap_or(text);
                row(vec![c.val(conv.map(|v| format!("{:.0}", v)), col, "888"), c.unit(u)])
            }
            GpuClock => row(vec![c.val(sys.gpu_clock.map(|v| v.to_string()), text, "8888"), c.unit("MHz")]),
            GpuPower => row(vec![c.val(sys.gpu_power.map(|v| format!("{:.0}", v)), text, "888"), c.unit("W")]),
            GpuFan => row(vec![c.val(sys.gpu_fan.map(|v| v.to_string()), text, "888"), c.unit("%")]),
            Vram | Ram => {
                let (used, total) = if m == Vram { (sys.vram_used, sys.vram_total) } else { (sys.ram_used, sys.ram_total) };
                let pct = match (used, total) {
                    (Some(u), Some(t)) if t > 0 => Some(u as f32 / t as f32 * 100.0),
                    _ => None,
                };
                let (tc, bc) = pct.map(|p| c.usage_color(p)).unwrap_or((text, style.accent));
                row_bar(
                    vec![
                        c.val(used.map(fmt::bytes_gb), tc, "88.8"),
                        c.unit("/"),
                        c.val(total.map(fmt::bytes_gb), c.muted(), "88.8"),
                        c.unit("GB"),
                    ],
                    pct.map(|p| p / 100.0),
                    bc,
                )
            }
            Network | Disk => {
                let (a, b, la, lb) = if m == Network {
                    (sys.net_rx, sys.net_tx, "↓", " ↑")
                } else {
                    (sys.disk_read, sys.disk_write, tr(lang, "O", "R"), tr(lang, " Z", " W"))
                };
                let (av, au) = a.map(fmt::rate).map(|(v, u)| (Some(v), u)).unwrap_or((None, "KB/s"));
                let (bv, bu) = b.map(fmt::rate).map(|(v, u)| (Some(v), u)).unwrap_or((None, "KB/s"));
                row(vec![
                    Span::unit(la, style.accent),
                    c.val(av, text, T_RATE),
                    Span::unit_t(au, c.muted(), T_RATE_UNIT),
                    Span::unit(lb, style.accent),
                    c.val(bv, text, T_RATE),
                    Span::unit_t(bu, c.muted(), T_RATE_UNIT),
                ])
            }
            Battery => {
                let t = &cfg.thresholds;
                let pct = sys.battery.map(|(p, _)| p);
                let charging = sys.battery.map_or(false, |(_, ac)| ac);
                let col = match pct {
                    Some(p) if style.colorize && p <= 10 => t.bad,
                    Some(p) if style.colorize && p <= 25 => t.warn,
                    _ => text,
                };
                row_bar(
                    vec![
                        c.val(pct.map(|p| p.to_string()), col, "888"),
                        c.unit("%"),
                        // Miejsce na ikonę ładowania jest zarezerwowane zawsze.
                        Span::unit_t(if charging { "AC" } else { "" }, style.accent, "AC"),
                    ],
                    pct.map(|p| p as f32 / 100.0),
                    if col == text { style.accent } else { col },
                )
            }
            Display => row(vec![
                Span::value(format!("{}×{}", c.mon.width(), c.mon.height()), text, "8888×8888"),
                Span::unit_t(format!("{} Hz", c.mon.refresh_hz), c.muted(), "888 Hz"),
            ]),
            Clock => row(vec![Span::value(format!("{:02}:{:02}", c.clock.0, c.clock.1), text, "88:88")]),
            Uptime => row(vec![Span::value(fmt::duration(c.sys.uptime_s), text, T_DURATION)]),
            Unknown => continue,
        };
        blocks.push(block);
    }

    if uses_fps {
        let note = match c.etw {
            EtwStatus::NeedsAdmin => Some(tr(lang, "FPS wymaga uprawnień administratora", "FPS requires administrator rights")),
            EtwStatus::Failed => Some(tr(lang, "Pomiar FPS niedostępny", "FPS measurement unavailable")),
            _ => None,
        };
        if let Some(n) = note {
            blocks.push(Block::Note { text: n.into(), color: cfg.thresholds.warn });
        }
    }
    if c.edit {
        blocks.push(Block::Note {
            text: format!("{} · {}", tr(lang, "Przeciągnij, aby przesunąć", "Drag to move"), cfg.hotkeys.move_mode),
            color: style.accent,
        });
    }
    if blocks.is_empty() {
        blocks.push(Block::Note { text: "OpenFPS".into(), color: style.accent });
    }

    Frame { blocks, orientation: layout.orientation, show_labels: layout.show_labels, edit: c.edit }
}
