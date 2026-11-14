//! Proces nakładki: ukryte okno-host (zasobnik, skróty, timery, IPC)
//! oraz po jednym warstwowym, przezroczystym dla myszy oknie na monitor.

mod content;
mod render;

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Instant, SystemTime};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS};
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::config::{Anchor, Config, Lang, Mode, ModeOverride};
use crate::hotkey::Hotkey;
use crate::i18n::tr;
use crate::metrics::etw::EtwStatus;
use crate::metrics::{AppInfo, Collector, Needs};
use crate::monitors::{self, MonitorInfo};
use crate::util::{self, wide, HOST_CLASS, MSG_OPEN_SETTINGS, MSG_QUIT, MSG_RELOAD, MSG_SUSPEND_HOTKEYS, MSG_TRAY, MSG_UPDATE_AVAILABLE, OVERLAY_CLASS};
use render::{Gfx, Surface};

const TIMER_TICK: usize = 1;
const TIMER_CONFIG: usize = 2;
const TIMER_TOPMOST: usize = 3;
const TIMER_UPDATE: usize = 4;
/// Odstęp między sprawdzeniami aktualizacji.
const UPDATE_INTERVAL_MS: u32 = 24 * 60 * 60 * 1000;

/// Wersja znaleziona przez wątek sprawdzania aktualizacji.
static UPDATE_FOUND: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

const HK_TOGGLE: i32 = 1;
const HK_MODE: i32 = 2;
const HK_SETTINGS: i32 = 3;
const HK_MOVE: i32 = 4;
const HK_PRESET: i32 = 5;

// Identyfikatory poleceń menu zasobnika.
const CMD_TOGGLE: u32 = 100;
const CMD_SIMPLE: u32 = 101;
const CMD_DETAILED: u32 = 102;
const CMD_MOVE: u32 = 103;
const CMD_SETTINGS: u32 = 104;
const CMD_ADMIN: u32 = 105;
const CMD_LANG_PL: u32 = 106;
const CMD_LANG_EN: u32 = 107;
const CMD_EXIT: u32 = 108;
const CMD_FEEDBACK: u32 = 109;
const CMD_UPDATE: u32 = 110;
const CMD_MONITOR: u32 = 200;
const CMD_PRESET: u32 = 300;

static EDIT_MODE: AtomicBool = AtomicBool::new(false);

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Dostęp do stanu z procedur okien. Jeśli stan jest już pożyczony
/// (wywołanie zwrotne w trakcie innej operacji) — zwraca None i wiadomość idzie do DefWindowProc.
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

struct Overlay {
    hwnd: HWND,
    mon_id: String,
    surface: Surface,
    fps_hist: VecDeque<f32>,
    dragging: bool,
    shown: bool,
    k: f32,
    /// Podpis ostatnio narysowanej klatki — identyczna treść nie jest renderowana ponownie.
    last_sig: u64,
    /// Najmniejszy dopuszczalny rozmiar (px) — okno nie kurczy się w trakcie działania.
    min_size: (f32, f32),
}

impl Drop for Overlay {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

struct App {
    cfg: Config,
    cfg_mtime: Option<SystemTime>,
    host: HWND,
    gfx: Gfx,
    collector: Collector,
    monitors: Vec<MonitorInfo>,
    overlays: Vec<Overlay>,
    needs: Needs,
    edit: bool,
    hotkeys_suspended: bool,
    registered: Vec<i32>,
    tray_icon: HICON,
    tray_added: bool,
    last_sys: Option<Instant>,
    msg_taskbar_created: u32,
}

fn mtime() -> Option<SystemTime> {
    std::fs::metadata(Config::path()).and_then(|m| m.modified()).ok()
}

fn copy_w(dst: &mut [u16], s: &str) {
    let src: Vec<u16> = s.encode_utf16().take(dst.len() - 1).collect();
    dst[..src.len()].copy_from_slice(&src);
    dst[src.len()] = 0;
}

fn make_icon(size: i32) -> HICON {
    let px = crate::icon::rgba(size as u32);
    unsafe {
        let bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let Ok(color) = CreateDIBSection(HDC::default(), &bi, DIB_RGB_COLORS, &mut bits, None, 0) else {
            return HICON::default();
        };
        let dst = std::slice::from_raw_parts_mut(bits as *mut u8, (size * size * 4) as usize);
        for i in 0..(size * size) as usize {
            dst[i * 4] = px[i * 4 + 2];
            dst[i * 4 + 1] = px[i * 4 + 1];
            dst[i * 4 + 2] = px[i * 4];
            dst[i * 4 + 3] = px[i * 4 + 3];
        }
        let mask = CreateBitmap(size, size, 1, 1, None);
        let info = ICONINFO { fIcon: TRUE, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info).unwrap_or_default();
        let _ = DeleteObject(color);
        let _ = DeleteObject(mask);
        icon
    }
}

/// Pozycja okna o rozmiarze (w, h) w obszarze `area` wg kotwicy i marginesów.
fn place(area: RECT, w: i32, h: i32, anchor: Anchor, mx: i32, my: i32) -> (i32, i32) {
    let (col, row) = anchor.grid();
    let aw = area.right - area.left;
    let ah = area.bottom - area.top;
    let x = match col {
        0 => area.left + mx,
        1 => area.left + (aw - w) / 2 + mx,
        _ => area.right - w - mx,
    };
    let y = match row {
        0 => area.top + my,
        1 => area.top + (ah - h) / 2 + my,
        _ => area.bottom - h - my,
    };
    (x, y)
}

impl App {
    fn lang(&self) -> Lang {
        self.cfg.general.language
    }

    fn save(&mut self) {
        let _ = self.cfg.save();
        self.cfg_mtime = mtime();
        self.invalidate();
    }

    /// Wymusza ponowne narysowanie (np. po zmianie stylu, którego nie obejmuje podpis treści).
    fn invalidate(&mut self) {
        for o in &mut self.overlays {
            o.last_sig = 0;
            o.min_size = (0.0, 0.0);
        }
    }

    fn mode_for(&self, mon_id: &str) -> Mode {
        match self.cfg.monitor(mon_id).map(|m| m.mode).unwrap_or_default() {
            ModeOverride::Inherit => self.cfg.general.mode,
            ModeOverride::Simple => Mode::Simple,
            ModeOverride::Detailed => Mode::Detailed,
        }
    }

    /// Źródła danych wymagane przez tryby faktycznie używane na włączonych monitorach.
    fn compute_needs(&self) -> Needs {
        // Ukryta nakładka nie zbiera niczego: ETW, PDH i timery są wyłączone.
        if !self.cfg.general.visible && !self.edit {
            return Needs::default();
        }
        let mut metrics = Vec::new();
        for o in &self.overlays {
            metrics.extend(self.cfg.layout(self.mode_for(&o.mon_id)).enabled());
        }
        Needs::from_metrics(metrics.into_iter())
    }

    fn apply_config(&mut self) {
        self.cfg.sanitize();
        self.invalidate();
        self.monitors = monitors::enumerate();
        let mut added = false;
        for m in &self.monitors {
            let before = self.cfg.monitors.len();
            self.cfg.monitor_mut(&m.id, &m.name, m.primary);
            added |= self.cfg.monitors.len() != before;
        }
        if added {
            self.save();
        }
        self.sync_overlays();
        let g = &self.cfg.general;
        crate::metrics::etw::set_retention(g.lows_window_s.max(g.graph_seconds).max(g.fps_average_ms / 1000) + 2);
        self.refresh_sources();
        self.register_hotkeys();
        let affinity = if self.cfg.general.hide_from_capture { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE };
        for o in &self.overlays {
            unsafe {
                let _ = SetWindowDisplayAffinity(o.hwnd, affinity);
            }
        }
        self.update_tray_tip();
        self.tick();
    }

    /// Przelicza potrzebne źródła danych i dopasowuje do nich timer odświeżania.
    fn refresh_sources(&mut self) {
        let needs = self.compute_needs();
        let was_active = self.needs != Needs::default();
        self.needs = needs;
        self.collector.configure(needs);
        self.last_sys = None;
        let g = &self.cfg.general;
        let visible = g.visible || self.edit;
        unsafe {
            if visible {
                // Bez metryk FPS wystarczy rytm metryk systemowych (zegar ma minutową rozdzielczość).
                let interval = if needs.fps { g.fps_refresh_ms } else if needs.any_sys() { g.sys_refresh_ms } else { 1000 };
                let _ = SetTimer(self.host, TIMER_TICK, interval, None);
                let _ = SetTimer(self.host, TIMER_TOPMOST, 2000, None);
            } else {
                let _ = KillTimer(self.host, TIMER_TICK);
                let _ = KillTimer(self.host, TIMER_TOPMOST);
                if was_active {
                    // Oddaj systemowi strony pamięci, których ukryta nakładka nie dotyka.
                    let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
                }
            }
        }
    }

    fn sync_overlays(&mut self) {
        let wanted: Vec<String> = self
            .monitors
            .iter()
            .filter(|m| self.cfg.monitor(&m.id).map_or(m.primary, |c| c.enabled))
            .map(|m| m.id.clone())
            .collect();
        self.overlays.retain(|o| wanted.contains(&o.mon_id));
        for id in wanted {
            if self.overlays.iter().any(|o| o.mon_id == id) {
                continue;
            }
            if let Some(hwnd) = create_overlay_window() {
                set_click_through(hwnd, !self.edit);
                self.overlays.push(Overlay {
                    hwnd,
                    mon_id: id,
                    surface: Surface::new(),
                    fps_hist: VecDeque::new(),
                    dragging: false,
                    shown: false,
                    k: 1.0,
                    last_sig: 0,
                    min_size: (0.0, 0.0),
                });
            }
        }
    }

    fn register_hotkeys(&mut self) {
        unsafe {
            for id in self.registered.drain(..) {
                let _ = UnregisterHotKey(self.host, id);
            }
        }
        if self.hotkeys_suspended {
            return;
        }
        let h = &self.cfg.hotkeys;
        let list = [
            (HK_TOGGLE, h.toggle.clone()),
            (HK_MODE, h.mode.clone()),
            (HK_SETTINGS, h.settings.clone()),
            (HK_MOVE, h.move_mode.clone()),
            (HK_PRESET, h.cycle_preset.clone()),
        ];
        let mut failed = Vec::new();
        for (id, s) in list {
            if s.trim().is_empty() {
                continue;
            }
            let Some(hk) = Hotkey::parse(&s) else {
                failed.push(s);
                continue;
            };
            let ok = unsafe { RegisterHotKey(self.host, id, HOT_KEY_MODIFIERS(hk.modifiers()), hk.vk) }.is_ok();
            if ok {
                self.registered.push(id);
            } else {
                failed.push(s);
            }
        }
        if !failed.is_empty() {
            let lang = self.lang();
            self.balloon(
                tr(lang, "Skrót niedostępny", "Hotkey unavailable"),
                &format!(
                    "{}: {}",
                    tr(lang, "Nie udało się zarejestrować (zajęty przez inny program?)", "Could not register (used by another app?)"),
                    failed.join(", ")
                ),
            );
        }
    }

    fn tick(&mut self) {
        let g = &self.cfg.general;
        let any_visible = g.visible || self.edit;
        if !any_visible {
            for o in &mut self.overlays {
                if o.shown {
                    unsafe {
                        let _ = ShowWindow(o.hwnd, SW_HIDE);
                    }
                    o.shown = false;
                }
            }
            return;
        }
        let due = self.last_sys.map_or(true, |t| t.elapsed().as_millis() as u32 + 20 >= g.sys_refresh_ms);
        if due {
            self.collector.sample_sys();
            self.last_sys = Some(Instant::now());
        }
        let apps = if self.needs.fps {
            self.collector.flush_fps();
            self.collector.apps_per_monitor(&self.cfg)
        } else {
            HashMap::new()
        };
        let st = unsafe { GetLocalTime() };
        let clock = (st.wHour, st.wMinute);
        for i in 0..self.overlays.len() {
            self.render_overlay(i, &apps, clock);
        }
    }

    fn render_overlay(&mut self, i: usize, apps: &HashMap<isize, AppInfo>, clock: (u16, u16)) {
        let o = &mut self.overlays[i];
        let Some(mon) = self.monitors.iter().find(|m| m.id == o.mon_id) else { return };
        let mcfg = self.cfg.monitor(&o.mon_id).cloned().unwrap_or_default();
        let mode = match mcfg.mode {
            ModeOverride::Inherit => self.cfg.general.mode,
            ModeOverride::Simple => Mode::Simple,
            ModeOverride::Detailed => Mode::Detailed,
        };
        let app = apps.get(&(mon.handle.0 as isize));

        let cap = ((self.cfg.general.graph_seconds * 1000) / self.cfg.general.fps_refresh_ms).max(8) as usize;
        o.fps_hist.push_back(app.map(|a| a.stats.fps).unwrap_or(0.0));
        while o.fps_hist.len() > cap {
            o.fps_hist.pop_front();
        }

        let show = self.cfg.general.visible || self.edit;
        let hide_no_app = self.cfg.general.auto_hide_without_app && app.is_none() && !self.edit;
        if !show || hide_no_app {
            if o.shown {
                unsafe {
                    let _ = ShowWindow(o.hwnd, SW_HIDE);
                }
                o.shown = false;
            }
            return;
        }

        let ctx = content::Ctx {
            cfg: &self.cfg,
            mode,
            sys: &self.collector.sys,
            app,
            mon,
            fps_hist: &o.fps_hist,
            etw: self.collector.fps_available(),
            edit: self.edit,
            clock,
        };
        let frame = content::build(&ctx);
        let k = mon.dpi as f32 / 96.0 * self.cfg.style.scale * mcfg.scale;
        let area = if mcfg.use_work_area { mon.work } else { mon.rect };
        // Nic się nie zmieniło (np. pulpit bez gry) → bez rasteryzacji i bez UpdateLayeredWindow.
        let extra = (k.to_bits() as u64) << 32
            ^ (area.left as u64).wrapping_mul(31)
            ^ (area.top as u64).wrapping_mul(131)
            ^ (area.right as u64).wrapping_mul(1031)
            ^ (area.bottom as u64).wrapping_mul(10007)
            ^ (mcfg.anchor as u64) << 8
            ^ (mcfg.margin_x.to_bits() as u64) << 16
            ^ mcfg.margin_y.to_bits() as u64;
        let sig = frame.signature(extra);
        if o.shown && !o.dragging && sig == o.last_sig {
            return;
        }
        o.last_sig = sig;
        o.k = k;
        let (w, h) = self.gfx.render(&mut o.surface, &self.cfg.style, k, &frame, &mut o.min_size);
        let (x, y) = place(area, w, h, mcfg.anchor, (mcfg.margin_x * k).round() as i32, (mcfg.margin_y * k).round() as i32);

        unsafe {
            let size = SIZE { cx: w, cy: h };
            let src = POINT { x: 0, y: 0 };
            let dst = POINT { x, y };
            let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
            let screen = GetDC(HWND::default());
            let _ = UpdateLayeredWindow(
                o.hwnd,
                screen,
                if o.dragging { None } else { Some(&dst) },
                Some(&size),
                o.surface.dc,
                Some(&src),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );
            ReleaseDC(HWND::default(), screen);
            if !o.shown {
                let _ = ShowWindow(o.hwnd, SW_SHOWNOACTIVATE);
                let _ = SetWindowPos(o.hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                o.shown = true;
            }
        }
    }

    // ───────────────────────────── Akcje ─────────────────────────────

    fn set_visible(&mut self, v: bool) {
        self.cfg.general.visible = v;
        self.save();
        self.refresh_sources();
        self.tick();
    }

    fn set_mode(&mut self, m: Mode) {
        self.cfg.general.mode = m;
        self.save();
        self.refresh_sources();
        self.tick();
    }

    fn toggle_edit(&mut self) {
        self.edit = !self.edit;
        EDIT_MODE.store(self.edit, Ordering::Relaxed);
        for o in &mut self.overlays {
            set_click_through(o.hwnd, !self.edit);
            o.last_sig = 0;
            o.min_size = (0.0, 0.0);
        }
        self.refresh_sources();
        self.tick();
    }

    fn cycle_preset(&mut self) {
        let p = crate::presets::next_after(&self.cfg.preset);
        self.cfg.preset = p.id.to_string();
        self.cfg.style = (p.build)();
        self.save();
        self.tick();
    }

    fn open_settings_page(&self, page: &str) {
        util::spawn_self(&["--settings", &format!("--page={page}")]);
    }

    /// Sprawdza aktualizacje w tle; wynik wraca komunikatem do okna-hosta.
    fn check_updates(&self) {
        if !self.cfg.general.check_updates {
            return;
        }
        let host = self.host.0 as isize;
        let _ = std::thread::Builder::new().name("update-check".into()).spawn(move || {
            if let Ok(Some(rel)) = crate::update::check() {
                *UPDATE_FOUND.lock().unwrap() = Some(rel.version);
                unsafe {
                    let _ = PostMessageW(HWND(host as *mut _), MSG_UPDATE_AVAILABLE, WPARAM(0), LPARAM(0));
                }
            }
        });
    }

    fn open_settings(&self) {
        util::spawn_self(&["--settings"]);
    }

    /// Po przeciągnięciu: dobiera kotwicę wg tercji ekranu i przelicza marginesy.
    fn finish_drag(&mut self, hwnd: HWND) {
        let Some(o) = self.overlays.iter_mut().find(|o| o.hwnd == hwnd) else { return };
        o.dragging = false;
        let mut r = RECT::default();
        unsafe {
            let _ = GetWindowRect(hwnd, &mut r);
        }
        let k = o.k.max(0.1);
        let id = o.mon_id.clone();
        // Okno mogło trafić na inny monitor.
        let hmon = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
        let target = self.monitors.iter().find(|m| m.handle == hmon).map(|m| m.id.clone()).unwrap_or(id.clone());
        if target != id {
            // Przenieś konfigurację nakładki na monitor docelowy.
            if let Some(dst) = self.cfg.monitors.iter_mut().find(|m| m.id == target) {
                dst.enabled = true;
            }
            if let Some(o) = self.overlays.iter_mut().find(|o| o.hwnd == hwnd) {
                o.mon_id = target.clone();
            }
            if let Some(src) = self.cfg.monitors.iter_mut().find(|m| m.id == id) {
                src.enabled = false;
            }
        }
        let Some(mon) = self.monitors.iter().find(|m| m.id == target).cloned() else { return };
        let Some(mc) = self.cfg.monitors.iter_mut().find(|m| m.id == target) else { return };
        let area = if mc.use_work_area { mon.work } else { mon.rect };
        let (aw, ah) = (area.right - area.left, area.bottom - area.top);
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let (cx, cy) = (r.left + w / 2 - area.left, r.top + h / 2 - area.top);
        let col = if cx < aw / 3 { 0 } else if cx < aw * 2 / 3 { 1 } else { 2 };
        let row = if cy < ah / 3 { 0 } else if cy < ah * 2 / 3 { 1 } else { 2 };
        let mx = match col {
            0 => r.left - area.left,
            1 => (r.left + w / 2) - (area.left + aw / 2),
            _ => area.right - r.right,
        };
        let my = match row {
            0 => r.top - area.top,
            1 => (r.top + h / 2) - (area.top + ah / 2),
            _ => area.bottom - r.bottom,
        };
        mc.anchor = Anchor::from_grid(col as u8, row as u8);
        mc.margin_x = (mx as f32 / k).round();
        mc.margin_y = (my as f32 / k).round();
        self.save();
        self.sync_overlays();
        self.tick();
    }

    // ───────────────────────────── Zasobnik ─────────────────────────────

    fn tray_data(&self) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.host,
            uID: 1,
            ..Default::default()
        }
    }

    fn tray_add(&mut self) {
        let mut d = self.tray_data();
        d.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        d.uCallbackMessage = MSG_TRAY;
        d.hIcon = self.tray_icon;
        copy_w(&mut d.szTip, &self.tip());
        self.tray_added = unsafe { Shell_NotifyIconW(NIM_ADD, &d) }.as_bool();
    }

    fn tip(&self) -> String {
        let lang = self.lang();
        format!(
            "OpenFPS — {}\n{}: {}",
            if self.cfg.general.mode == Mode::Simple { tr(lang, "tryb prosty", "simple mode") } else { tr(lang, "tryb szczegółowy", "detailed mode") },
            tr(lang, "Pokaż/ukryj", "Show/hide"),
            self.cfg.hotkeys.toggle
        )
    }

    fn update_tray_tip(&self) {
        if !self.tray_added {
            return;
        }
        let mut d = self.tray_data();
        d.uFlags = NIF_TIP;
        copy_w(&mut d.szTip, &self.tip());
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &d);
        }
    }

    fn balloon(&self, title: &str, text: &str) {
        if !self.tray_added {
            return;
        }
        let mut d = self.tray_data();
        d.uFlags = NIF_INFO;
        d.dwInfoFlags = NIIF_INFO;
        copy_w(&mut d.szInfoTitle, title);
        copy_w(&mut d.szInfo, text);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &d);
        }
    }

    fn build_menu(&self) -> HMENU {
        let lang = self.lang();
        let g = &self.cfg.general;
        let hk = &self.cfg.hotkeys;
        unsafe {
            let m = CreatePopupMenu().unwrap_or_default();
            let add = |menu: HMENU, id: u32, text: &str, checked: bool, enabled: bool| {
                let t = wide(text);
                let mut flags = MF_STRING;
                if checked {
                    flags |= MF_CHECKED;
                }
                if !enabled {
                    flags |= MF_GRAYED;
                }
                let _ = AppendMenuW(menu, flags, id as usize, PCWSTR(t.as_ptr()));
            };
            let sep = |menu: HMENU| {
                let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            };
            let with_key = |t: &str, k: &str| if k.is_empty() { t.to_string() } else { format!("{t}\t{k}") };

            let status = match self.collector.fps_available() {
                EtwStatus::Running => tr(lang, "pomiar FPS aktywny", "FPS capture active"),
                EtwStatus::NeedsAdmin => tr(lang, "FPS wymaga administratora", "FPS needs administrator"),
                EtwStatus::Failed => tr(lang, "pomiar FPS niedostępny", "FPS capture unavailable"),
                EtwStatus::Starting => tr(lang, "uruchamianie…", "starting…"),
            };
            add(m, 0, &format!("OpenFPS {} — {}", crate::util::VERSION, status), false, false);
            sep(m);
            add(m, CMD_TOGGLE, &with_key(tr(lang, "Pokaż nakładkę", "Show overlay"), &hk.toggle), g.visible, true);
            add(m, CMD_SIMPLE, tr(lang, "Tryb prosty", "Simple mode"), g.mode == Mode::Simple, true);
            add(m, CMD_DETAILED, &with_key(tr(lang, "Tryb szczegółowy", "Detailed mode"), &hk.mode), g.mode == Mode::Detailed, true);
            add(m, CMD_MOVE, &with_key(tr(lang, "Przesuwanie nakładki", "Move overlay"), &hk.move_mode), self.edit, true);

            let mons = CreatePopupMenu().unwrap_or_default();
            for (i, mon) in self.monitors.iter().enumerate() {
                let on = self.cfg.monitor(&mon.id).map_or(false, |c| c.enabled);
                let primary = if mon.primary { tr(lang, " (główny)", " (primary)") } else { "" };
                add(mons, CMD_MONITOR + i as u32, &format!("{}{}", mon.label(), primary), on, true);
            }
            let t = wide(tr(lang, "Monitory", "Displays"));
            let _ = AppendMenuW(m, MF_POPUP, mons.0 as usize, PCWSTR(t.as_ptr()));

            let presets = CreatePopupMenu().unwrap_or_default();
            for (i, p) in crate::presets::PRESETS.iter().enumerate() {
                add(presets, CMD_PRESET + i as u32, tr(lang, p.name_pl, p.name_en), self.cfg.preset == p.id, true);
            }
            let t = wide(tr(lang, "Motyw", "Theme"));
            let _ = AppendMenuW(m, MF_POPUP, presets.0 as usize, PCWSTR(t.as_ptr()));

            let langs = CreatePopupMenu().unwrap_or_default();
            add(langs, CMD_LANG_PL, "Polski", lang == Lang::Pl, true);
            add(langs, CMD_LANG_EN, "English", lang == Lang::En, true);
            let t = wide(tr(lang, "Język", "Language"));
            let _ = AppendMenuW(m, MF_POPUP, langs.0 as usize, PCWSTR(t.as_ptr()));

            sep(m);
            add(m, CMD_SETTINGS, &with_key(tr(lang, "Ustawienia…", "Settings…"), &hk.settings), false, true);
            add(m, CMD_FEEDBACK, tr(lang, "Zgłoś błąd lub sugestię…", "Report a bug or suggestion…"), false, true);
            if let Some(v) = UPDATE_FOUND.lock().unwrap().clone() {
                add(m, CMD_UPDATE, &format!("{} {v}…", tr(lang, "Zaktualizuj do wersji", "Update to version")), false, true);
            }
            if !util::is_elevated() {
                add(m, CMD_ADMIN, tr(lang, "Uruchom ponownie jako administrator", "Restart as administrator"), false, true);
            }
            sep(m);
            add(m, CMD_EXIT, tr(lang, "Zakończ", "Exit"), false, true);
            m
        }
    }

    fn command(&mut self, id: u32) {
        match id {
            CMD_TOGGLE => self.set_visible(!self.cfg.general.visible),
            CMD_SIMPLE => self.set_mode(Mode::Simple),
            CMD_DETAILED => self.set_mode(Mode::Detailed),
            CMD_MOVE => self.toggle_edit(),
            CMD_SETTINGS => self.open_settings(),
            CMD_FEEDBACK => self.open_settings_page("feedback"),
            CMD_UPDATE => self.open_settings_page("about"),
            CMD_ADMIN => {
                if util::relaunch_elevated("--relaunched") {
                    unsafe { PostQuitMessage(0) };
                }
            }
            CMD_LANG_PL | CMD_LANG_EN => {
                self.cfg.general.language = if id == CMD_LANG_PL { Lang::Pl } else { Lang::En };
                self.save();
                self.update_tray_tip();
                self.tick();
            }
            CMD_EXIT => unsafe { PostQuitMessage(0) },
            x if (CMD_MONITOR..CMD_MONITOR + 64).contains(&x) => {
                if let Some(mon) = self.monitors.get((x - CMD_MONITOR) as usize) {
                    let id = mon.id.clone();
                    if let Some(mc) = self.cfg.monitors.iter_mut().find(|m| m.id == id) {
                        mc.enabled = !mc.enabled;
                    }
                    self.save();
                    self.sync_overlays();
                    self.refresh_sources();
                    self.tick();
                }
            }
            x if (CMD_PRESET..CMD_PRESET + 64).contains(&x) => {
                if let Some(p) = crate::presets::PRESETS.get((x - CMD_PRESET) as usize) {
                    self.cfg.preset = p.id.to_string();
                    self.cfg.style = (p.build)();
                    self.save();
                    self.tick();
                }
            }
            _ => {}
        }
    }

    fn reload_if_changed(&mut self, force: bool) {
        let m = mtime();
        if force || (m.is_some() && m != self.cfg_mtime) {
            self.cfg_mtime = m;
            let old_needs = self.needs;
            self.cfg = Config::load();
            self.apply_config();
            if old_needs != self.needs {
                self.last_sys = None;
            }
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if self.tray_added {
            let d = self.tray_data();
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &d);
            }
        }
        unsafe {
            for id in &self.registered {
                let _ = UnregisterHotKey(self.host, *id);
            }
            if !self.tray_icon.is_invalid() {
                let _ = DestroyIcon(self.tray_icon);
            }
        }
    }
}

// ───────────────────────────── Okna ─────────────────────────────

fn create_overlay_window() -> Option<HWND> {
    unsafe {
        let inst = GetModuleHandleW(None).ok()?;
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            OVERLAY_CLASS,
            w!("OpenFPS Overlay"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            HWND::default(),
            HMENU::default(),
            inst,
            None,
        )
        .ok()
    }
}

fn set_click_through(hwnd: HWND, through: bool) {
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let t = WS_EX_TRANSPARENT.0 as isize;
        let new = if through { ex | t } else { ex & !t };
        if new != ex {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new);
        }
    }
}

unsafe extern "system" fn overlay_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_NCHITTEST => {
            if EDIT_MODE.load(Ordering::Relaxed) {
                LRESULT(HTCAPTION as isize)
            } else {
                LRESULT(HTTRANSPARENT as isize)
            }
        }
        WM_SETCURSOR if EDIT_MODE.load(Ordering::Relaxed) => {
            if let Ok(c) = LoadCursorW(HINSTANCE::default(), IDC_SIZEALL) {
                SetCursor(c);
            }
            LRESULT(1)
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_ENTERSIZEMOVE => {
            with_app(|a| {
                if let Some(o) = a.overlays.iter_mut().find(|o| o.hwnd == hwnd) {
                    o.dragging = true;
                }
            });
            LRESULT(0)
        }
        WM_EXITSIZEMOVE => {
            with_app(|a| a.finish_drag(hwnd));
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

unsafe extern "system" fn host_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER => {
            match wp.0 {
                TIMER_TICK => {
                    with_app(|a| a.tick());
                }
                TIMER_CONFIG => {
                    with_app(|a| a.reload_if_changed(false));
                }
                TIMER_UPDATE => {
                    with_app(|a| a.check_updates());
                }
                TIMER_TOPMOST => {
                    with_app(|a| {
                        for o in a.overlays.iter().filter(|o| o.shown && !o.dragging) {
                            let _ = SetWindowPos(o.hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                        }
                    });
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_HOTKEY => {
            with_app(|a| match wp.0 as i32 {
                HK_TOGGLE => a.set_visible(!a.cfg.general.visible),
                HK_MODE => {
                    let m = a.cfg.general.mode.toggled();
                    a.set_mode(m);
                }
                HK_SETTINGS => a.open_settings(),
                HK_MOVE => a.toggle_edit(),
                HK_PRESET => a.cycle_preset(),
                _ => {}
            });
            LRESULT(0)
        }
        MSG_TRAY => {
            let ev = (lp.0 & 0xFFFF) as u32;
            match ev {
                WM_RBUTTONUP | WM_CONTEXTMENU => {
                    // Menu budujemy pod pożyczką, ale pętlę menu uruchamiamy już bez niej.
                    if let Some(menu) = with_app(|a| a.build_menu()) {
                        let mut p = POINT::default();
                        let _ = GetCursorPos(&mut p);
                        let _ = SetForegroundWindow(hwnd);
                        let cmd = TrackPopupMenu(menu, TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY, p.x, p.y, 0, hwnd, None);
                        let _ = PostMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));
                        let _ = DestroyMenu(menu);
                        if cmd.0 > 0 {
                            with_app(|a| a.command(cmd.0 as u32));
                        }
                    }
                }
                WM_LBUTTONDBLCLK => {
                    with_app(|a| a.open_settings());
                }
                WM_LBUTTONUP => {
                    with_app(|a| a.set_visible(!a.cfg.general.visible));
                }
                _ => {}
            }
            LRESULT(0)
        }
        MSG_RELOAD => {
            with_app(|a| a.reload_if_changed(true));
            LRESULT(0)
        }
        MSG_OPEN_SETTINGS => {
            with_app(|a| a.open_settings());
            LRESULT(0)
        }
        MSG_SUSPEND_HOTKEYS => {
            with_app(|a| {
                a.hotkeys_suspended = wp.0 != 0;
                a.register_hotkeys();
            });
            LRESULT(0)
        }
        MSG_UPDATE_AVAILABLE => {
            with_app(|a| {
                if let Some(v) = UPDATE_FOUND.lock().unwrap().clone() {
                    let lang = a.lang();
                    a.balloon(
                        &format!("{} {v}", tr(lang, "Dostępna jest wersja", "Version available:")),
                        tr(lang, "Zaktualizujesz ją z menu ikony albo w Ustawieniach → O programie.", "Update from the tray menu or in Settings → About."),
                    );
                }
            });
            LRESULT(0)
        }
        MSG_QUIT => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        WM_DISPLAYCHANGE | WM_SETTINGCHANGE | WM_DPICHANGED => {
            with_app(|a| a.apply_config());
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => {
            let handled = with_app(|a| {
                if msg == a.msg_taskbar_created && msg != 0 {
                    a.tray_add();
                    true
                } else {
                    false
                }
            });
            if handled == Some(true) {
                LRESULT(0)
            } else {
                DefWindowProcW(hwnd, msg, wp, lp)
            }
        }
    }
}

/// Tryb „Efektywność” (EcoQoS) — Windows planuje wątki nakładki energooszczędnie.
fn enable_eco_qos() {
    unsafe {
        let state = PROCESS_POWER_THROTTLING_STATE {
            Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
            StateMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        };
        let _ = SetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            &state as *const _ as *const _,
            std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        );
    }
}

pub fn run(relaunched: bool, no_elevate: bool) {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    // Jedna instancja. Po restarcie z UAC czekamy, aż stary proces zwolni mutex.
    let mutex = unsafe { CreateMutexW(None, false, w!("Local\\OpenFPS.Overlay")) }.ok();
    if let Some(m) = mutex {
        let r = unsafe { WaitForSingleObject(m, if relaunched { 8000 } else { 0 }) };
        if r == WAIT_TIMEOUT {
            util::post_to_overlay(MSG_OPEN_SETTINGS, 0);
            return;
        }
    }

    let mut cfg = Config::load();
    if cfg.general.run_as_admin && !util::is_elevated() && !no_elevate && !relaunched {
        if let Some(m) = mutex {
            unsafe {
                let _ = ReleaseMutex(m);
            }
        }
        if util::relaunch_elevated("--relaunched") {
            return;
        }
        if let Some(m) = mutex {
            unsafe { WaitForSingleObject(m, 0) };
        }
    }

    enable_eco_qos();

    unsafe {
        let Ok(inst) = GetModuleHandleW(None) else { return };
        let inst: HINSTANCE = inst.into();
        let arrow = LoadCursorW(HINSTANCE::default(), IDC_ARROW).unwrap_or_default();
        let wc = WNDCLASSW { lpfnWndProc: Some(host_proc), hInstance: inst, lpszClassName: HOST_CLASS, ..Default::default() };
        RegisterClassW(&wc);
        let wc = WNDCLASSW { lpfnWndProc: Some(overlay_proc), hInstance: inst, lpszClassName: OVERLAY_CLASS, hCursor: arrow, ..Default::default() };
        RegisterClassW(&wc);

        let Ok(host) = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            HOST_CLASS,
            w!("OpenFPS"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            HWND::default(),
            HMENU::default(),
            inst,
            None,
        ) else {
            return;
        };

        let Ok(gfx) = Gfx::new() else { return };
        let icon_size = GetSystemMetrics(SM_CXSMICON).max(16);
        let first_run = cfg.general.first_run;
        if first_run {
            cfg.general.first_run = false;
        }
        let app = App {
            cfg,
            cfg_mtime: None,
            host,
            gfx,
            collector: Collector::new(),
            monitors: Vec::new(),
            overlays: Vec::new(),
            needs: Needs::default(),
            edit: false,
            hotkeys_suspended: false,
            registered: Vec::new(),
            tray_icon: make_icon(icon_size * 2),
            tray_added: false,
            last_sys: None,
            msg_taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
        };
        APP.with(|a| *a.borrow_mut() = Some(app));
        with_app(|a| {
            a.tray_add();
            if first_run {
                a.save();
            } else {
                a.cfg_mtime = mtime();
            }
            a.apply_config();
            if first_run {
                let lang = a.lang();
                let h = a.cfg.hotkeys.clone();
                a.balloon(
                    tr(lang, "OpenFPS działa w tle", "OpenFPS is running"),
                    &format!(
                        "{} {}\n{} {}\n{} {}",
                        tr(lang, "Pokaż/ukryj:", "Show/hide:"),
                        h.toggle,
                        tr(lang, "Tryb prosty/szczegółowy:", "Simple/detailed:"),
                        h.mode,
                        tr(lang, "Ustawienia:", "Settings:"),
                        h.settings
                    ),
                );
            }
        });
        let _ = SetTimer(host, TIMER_CONFIG, 1000, None);
        let _ = SetTimer(host, TIMER_UPDATE, UPDATE_INTERVAL_MS, None);
        with_app(|a| {
            a.check_updates();
            // Awaria przy poprzednim uruchomieniu: zapytaj o wysłanie raportu (tylko za zgodą).
            if crate::feedback::pending_crash().is_some() {
                a.open_settings_page("crash");
            }
        });

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, HWND::default(), 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        APP.with(|a| a.borrow_mut().take());
        let _ = DestroyWindow(host);
    }
}
