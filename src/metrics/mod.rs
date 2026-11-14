//! Zbieranie wszystkich metryk. Kolektor włącza tylko te źródła danych,
//! które są potrzebne do aktualnie widocznych pozycji — to klucz do niskiego narzutu.

pub mod etw;
pub mod nvml;
pub mod pdh;

use std::collections::{HashMap, HashSet};

use windows::core::PWSTR;
use windows::Win32::Foundation::{BOOL, CloseHandle, HWND, LPARAM, RECT, TRUE};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE};
use windows::Win32::Graphics::Gdi::{MonitorFromWindow, HMONITOR, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::SystemInformation::{GetTickCount64, GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
};

use crate::config::{Config, Metric};
use etw::{Clock, FrameStats, PresentTrace};
use pdh::{Counter, Pdh};

// ───────────────────────────── Zapotrzebowanie ─────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Needs {
    pub fps: bool,
    pub cpu: bool,
    pub cpu_clock: bool,
    pub cores: bool,
    pub gpu_engine: bool,
    /// Użycie GPU przez konkretny proces — wymaga drogiego licznika PDH.
    pub gpu_app: bool,
    pub vram: bool,
    pub nvml: bool,
    pub nv_temp: bool,
    pub nv_clock: bool,
    pub nv_power: bool,
    pub nv_fan: bool,
    pub ram: bool,
    pub net: bool,
    pub disk: bool,
    pub battery: bool,
}

impl Needs {
    /// Czy cokolwiek wymaga okresowego próbkowania metryk systemowych.
    pub fn any_sys(&self) -> bool {
        let n = Needs { fps: false, ..*self };
        n != Needs::default()
    }
}

impl Needs {
    pub fn from_metrics(metrics: impl Iterator<Item = Metric>) -> Self {
        use Metric::*;
        let mut n = Needs::default();
        for m in metrics {
            match m {
                Fps | Frametime | Low1 | Low01 | FrametimeGraph | FpsGraph | PlayTime => n.fps = true,
                CpuUsage => n.cpu = true,
                CpuClock => n.cpu_clock = true,
                CpuCores => n.cores = true,
                GpuUsage => n.gpu_engine = true,
                GpuAppUsage => {
                    n.gpu_app = true;
                    n.fps = true;
                }
                GpuTemp => (n.nvml, n.nv_temp) = (true, true),
                GpuClock => (n.nvml, n.nv_clock) = (true, true),
                GpuPower => (n.nvml, n.nv_power) = (true, true),
                GpuFan => (n.nvml, n.nv_fan) = (true, true),
                Vram => n.vram = true,
                Ram => n.ram = true,
                Network => n.net = true,
                Disk => n.disk = true,
                Battery => n.battery = true,
                Display | Clock | Uptime | Unknown => {}
            }
        }
        n
    }
}

// ───────────────────────────── Migawka ─────────────────────────────

#[derive(Clone, Debug, Default)]
pub struct SysSnapshot {
    pub cpu: Option<f32>,
    pub cpu_mhz: Option<f32>,
    pub cores: Vec<f32>,
    pub gpu: Option<f32>,
    pub gpu_name: String,
    pub vram_used: Option<u64>,
    pub vram_total: Option<u64>,
    pub gpu_temp: Option<u32>,
    pub gpu_clock: Option<u32>,
    pub gpu_power: Option<f32>,
    pub gpu_fan: Option<u32>,
    pub ram_used: Option<u64>,
    pub ram_total: Option<u64>,
    pub net_rx: Option<f64>,
    pub net_tx: Option<f64>,
    pub disk_read: Option<f64>,
    pub disk_write: Option<f64>,
    pub battery: Option<(u8, bool)>,
    pub uptime_s: u64,
    pub gpu_per_pid: HashMap<u32, f32>,
}

#[derive(Clone, Debug)]
pub struct AppInfo {
    pub pid: u32,
    pub stats: FrameStats,
}

struct Adapter {
    name: String,
    luid: String,
    dedicated: u64,
}

/// Nazwa głównej karty graficznej (do opcjonalnych informacji o systemie w zgłoszeniach).
pub fn gpu_name() -> String {
    primary_adapter().map(|a| a.name).unwrap_or_default()
}

fn primary_adapter() -> Option<Adapter> {
    unsafe {
        let f: IDXGIFactory1 = CreateDXGIFactory1().ok()?;
        let mut best: Option<Adapter> = None;
        let mut i = 0;
        while let Ok(a) = f.EnumAdapters1(i) {
            i += 1;
            let Ok(d) = a.GetDesc1() else { continue };
            if d.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                continue;
            }
            let ad = Adapter {
                name: crate::util::from_wide(&d.Description),
                luid: format!("0x{:08X}_0x{:08X}", d.AdapterLuid.HighPart as u32, d.AdapterLuid.LowPart),
                dedicated: d.DedicatedVideoMemory as u64,
            };
            if best.as_ref().map_or(true, |b| ad.dedicated > b.dedicated) {
                best = Some(ad);
            }
        }
        best
    }
}

#[derive(Default)]
struct Counters {
    cpu: Option<Counter>,
    cpu_freq: Option<Counter>,
    cpu_perf: Option<Counter>,
    cores: Option<Counter>,
    gpu_engine: Option<Counter>,
    gpu_mem: Option<Counter>,
    net_rx: Option<Counter>,
    net_tx: Option<Counter>,
    disk_r: Option<Counter>,
    disk_w: Option<Counter>,
}

pub struct Collector {
    needs: Needs,
    pdh: Option<Pdh>,
    c: Counters,
    nvml: Option<Nvml>,
    nvml_tried: bool,
    adapter: Option<Adapter>,
    trace: Option<PresentTrace>,
    trace_tried: bool,
    pub clock: Clock,
    names: HashMap<u32, String>,
    /// Ostatnie przypisanie monitor → proces (EnumWindows najwyżej raz na sekundę).
    mapping: Vec<(isize, u32)>,
    mapping_at: Option<std::time::Instant>,
    pub sys: SysSnapshot,
}

use nvml::Nvml;

impl Collector {
    pub fn new() -> Self {
        Self {
            needs: Needs::default(),
            pdh: None,
            c: Counters::default(),
            nvml: None,
            nvml_tried: false,
            adapter: primary_adapter(),
            trace: None,
            trace_tried: false,
            clock: Clock::new(),
            names: HashMap::new(),
            mapping: Vec::new(),
            mapping_at: None,
            sys: SysSnapshot::default(),
        }
    }

    /// Dostosowuje aktywne źródła do potrzeb (wywoływane przy zmianie konfiguracji).
    pub fn configure(&mut self, needs: Needs) {
        if needs.fps && self.trace.is_none() && !self.trace_tried {
            self.trace_tried = true;
            self.trace = PresentTrace::start();
        } else if !needs.fps && self.trace.is_some() {
            self.trace = None;
            self.trace_tried = false;
        }
        // NVML (karty NVIDIA) jest dużo tańsze niż liczniki PDH „GPU Engine(*)”
        // z setkami instancji, więc używamy go także do użycia GPU i VRAM.
        if (needs.nvml || needs.gpu_engine || needs.vram) && !self.nvml_tried {
            self.nvml_tried = true;
            self.nvml = Nvml::load();
        }
        if needs == self.needs {
            return;
        }
        self.needs = needs;
        self.c = Counters::default();
        self.pdh = None;
        let has_nvml = self.nvml.is_some();
        let pdh_engine = needs.gpu_app || (needs.gpu_engine && !has_nvml);
        let pdh_vram = needs.vram && !has_nvml;
        let any_pdh = needs.cpu || needs.cpu_clock || needs.cores || pdh_engine || pdh_vram || needs.net || needs.disk;
        if !any_pdh {
            return;
        }
        let Some(p) = Pdh::new() else { return };
        if needs.cpu {
            self.c.cpu = p
                .add("\\Processor Information(_Total)\\% Processor Utility")
                .or_else(|| p.add("\\Processor(_Total)\\% Processor Time"));
        }
        if needs.cpu_clock {
            self.c.cpu_freq = p.add("\\Processor Information(_Total)\\Processor Frequency");
            self.c.cpu_perf = p.add("\\Processor Information(_Total)\\% Processor Performance");
        }
        if needs.cores {
            self.c.cores = p
                .add("\\Processor Information(*)\\% Processor Utility")
                .or_else(|| p.add("\\Processor Information(*)\\% Processor Time"));
        }
        if pdh_engine {
            self.c.gpu_engine = p.add("\\GPU Engine(*)\\Utilization Percentage");
        }
        if pdh_vram {
            self.c.gpu_mem = p.add("\\GPU Adapter Memory(*)\\Dedicated Usage");
        }
        if needs.net {
            self.c.net_rx = p.add("\\Network Interface(*)\\Bytes Received/sec");
            self.c.net_tx = p.add("\\Network Interface(*)\\Bytes Sent/sec");
        }
        if needs.disk {
            self.c.disk_r = p.add("\\PhysicalDisk(_Total)\\Disk Read Bytes/sec");
            self.c.disk_w = p.add("\\PhysicalDisk(_Total)\\Disk Write Bytes/sec");
        }
        p.collect(); // pierwsza próbka — liczniki „na sekundę” potrzebują dwóch
        self.pdh = Some(p);
    }

    pub fn fps_available(&self) -> etw::EtwStatus {
        etw::status()
    }

    /// Wymusza dostarczenie zdarzeń ETW (wywoływane co odświeżenie FPS).
    pub fn flush_fps(&mut self) {
        if let Some(t) = self.trace.as_mut() {
            t.flush();
        }
    }

    pub fn sample_sys(&mut self) {
        let n = self.needs;
        let mut s = SysSnapshot { gpu_name: self.adapter.as_ref().map(|a| a.name.clone()).unwrap_or_default(), ..Default::default() };

        if let Some(p) = &self.pdh {
            p.collect();
            let c = &self.c;
            s.cpu = c.cpu.as_ref().and_then(|c| p.value(c)).map(|v| v.clamp(0.0, 100.0) as f32);
            if let (Some(f), Some(perf)) = (c.cpu_freq.as_ref().and_then(|c| p.value(c)), c.cpu_perf.as_ref().and_then(|c| p.value(c))) {
                s.cpu_mhz = Some((f * perf / 100.0) as f32);
            }
            if let Some(cc) = &c.cores {
                let mut cores: Vec<((u32, u32), f32)> = p
                    .array(cc)
                    .into_iter()
                    .filter(|(n, _)| !n.contains("_Total"))
                    .map(|(n, v)| {
                        let mut it = n.split(',').map(|x| x.trim().parse::<u32>().unwrap_or(0));
                        ((it.next().unwrap_or(0), it.next().unwrap_or(0)), v.clamp(0.0, 100.0) as f32)
                    })
                    .collect();
                cores.sort_by_key(|(k, _)| *k);
                s.cores = cores.into_iter().map(|(_, v)| v).collect();
            }
            if let Some(ge) = &c.gpu_engine {
                let items = p.array(ge);
                let luid = self.adapter.as_ref().map(|a| a.luid.as_str());
                let (total, per_pid) = pdh::gpu_usage(&items, luid);
                s.gpu = Some(total);
                s.gpu_per_pid = per_pid;
            }
            if let Some(gm) = &c.gpu_mem {
                let luid = self.adapter.as_ref().map(|a| a.luid.clone());
                let used: f64 = p
                    .array(gm)
                    .iter()
                    .filter(|(n, _)| match (&luid, pdh::parse_adapter_luid(n)) {
                        (Some(l), Some(x)) => x.eq_ignore_ascii_case(l),
                        _ => true,
                    })
                    .map(|(_, v)| *v)
                    .fold(0.0, f64::max);
                s.vram_used = Some(used as u64);
                s.vram_total = self.adapter.as_ref().map(|a| a.dedicated);
            }
            let sum = |c: &Option<Counter>| c.as_ref().map(|c| p.array(c).iter().map(|(_, v)| *v).sum::<f64>());
            s.net_rx = sum(&c.net_rx);
            s.net_tx = sum(&c.net_tx);
            s.disk_read = c.disk_r.as_ref().and_then(|c| p.value(c));
            s.disk_write = c.disk_w.as_ref().and_then(|c| p.value(c));
        }

        if n.nvml || n.vram || n.gpu_engine {
            if let Some(nv) = &self.nvml {
                let x = nv.sample(nvml::Which {
                    temp: n.nv_temp,
                    clock: n.nv_clock,
                    power: n.nv_power,
                    fan: n.nv_fan,
                    util: n.gpu_engine && self.c.gpu_engine.is_none(),
                    mem: n.vram && self.c.gpu_mem.is_none(),
                });
                s.gpu_temp = x.temp_c;
                s.gpu_clock = x.clock_mhz;
                s.gpu_power = x.power_w;
                s.gpu_fan = x.fan_pct;
                if s.gpu.is_none() && n.gpu_engine {
                    s.gpu = x.util_pct.map(|v| v as f32);
                }
                if s.vram_used.is_none() && n.vram {
                    s.vram_used = x.mem_used;
                    s.vram_total = x.mem_total;
                }
                if s.gpu_name.is_empty() {
                    s.gpu_name = nv.name.clone();
                }
            }
        }

        if n.ram {
            let mut m = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
            if unsafe { GlobalMemoryStatusEx(&mut m) }.is_ok() {
                s.ram_total = Some(m.ullTotalPhys);
                s.ram_used = Some(m.ullTotalPhys - m.ullAvailPhys);
            }
        }
        if n.battery {
            let mut ps = SYSTEM_POWER_STATUS::default();
            if unsafe { GetSystemPowerStatus(&mut ps) }.is_ok() && ps.BatteryFlag != 128 && ps.BatteryLifePercent != 255 {
                s.battery = Some((ps.BatteryLifePercent, ps.ACLineStatus == 1));
            }
        }
        s.uptime_s = unsafe { GetTickCount64() } / 1000;
        self.sys = s;
    }

    fn process_name(&mut self, pid: u32) -> String {
        if let Some(n) = self.names.get(&pid) {
            return n.clone();
        }
        let name = unsafe {
            match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                Ok(h) => {
                    let mut buf = [0u16; 520];
                    let mut len = buf.len() as u32;
                    let r = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
                    let _ = CloseHandle(h);
                    if r.is_ok() {
                        let full = String::from_utf16_lossy(&buf[..len as usize]);
                        full.rsplit('\\').next().unwrap_or(&full).to_string()
                    } else {
                        format!("PID {pid}")
                    }
                }
                Err(_) => format!("PID {pid}"),
            }
        };
        self.names.insert(pid, name.clone());
        name
    }

    /// Dla każdego monitora wybiera aplikację, której FPS pokazujemy.
    /// Priorytet: okno na pierwszym planie, potem największe widoczne okno renderujące.
    pub fn apps_per_monitor(&mut self, cfg: &Config) -> HashMap<isize, AppInfo> {
        let mut out = HashMap::new();
        if self.trace.is_none() {
            return out;
        }
        let me = unsafe { GetCurrentProcessId() };
        let excluded: HashSet<String> = cfg.general.excluded_processes.iter().map(|s| s.trim().to_ascii_lowercase()).collect();
        let mut active: HashSet<u32> = HashSet::new();
        for pid in etw::active_pids(&self.clock, 1.5) {
            if pid == me || pid == 0 || pid == 4 {
                continue;
            }
            if excluded.contains(&self.process_name(pid).to_ascii_lowercase()) {
                continue;
            }
            active.insert(pid);
        }
        // Sprzątanie cache nazw.
        if self.names.len() > 256 {
            self.names.retain(|p, _| active.contains(p));
        }
        if active.is_empty() {
            return out;
        }

        // Przegląd okien jest najdroższą częścią — robimy go najwyżej raz na sekundę
        // albo od razu, gdy przypisany proces przestał renderować.
        let stale = self.mapping_at.map_or(true, |t| t.elapsed().as_millis() >= 1000)
            || self.mapping.iter().any(|(_, p)| !active.contains(p));
        if stale {
            let windows = visible_windows();
            let fg = unsafe { GetForegroundWindow() };
            let mut best: HashMap<isize, (u32, i64, bool)> = HashMap::new(); // monitor → (pid, area, foreground)
            for w in &windows {
                if !active.contains(&w.pid) {
                    continue;
                }
                let is_fg = w.hwnd == fg;
                let e = best.entry(w.monitor.0 as isize).or_insert((w.pid, -1, false));
                let better = match (is_fg, e.2) {
                    (true, false) => true,
                    (false, true) => false,
                    _ => w.area > e.1,
                };
                if better {
                    *e = (w.pid, w.area, is_fg);
                }
            }
            self.mapping = best.into_iter().map(|(m, (p, _, _))| (m, p)).collect();
            self.mapping_at = Some(std::time::Instant::now());
        }
        let g = &cfg.general;
        for (mon, pid) in self.mapping.clone() {
            if let Some(stats) = etw::stats(pid, &self.clock, g.fps_average_ms, g.lows_window_s, g.graph_seconds) {
                out.insert(mon, AppInfo { pid, stats });
            }
        }
        out
    }
}

// ───────────────────────────── Okna ─────────────────────────────

struct Win {
    hwnd: HWND,
    pid: u32,
    monitor: HMONITOR,
    area: i64,
}

unsafe extern "system" fn enum_win(h: HWND, data: LPARAM) -> BOOL {
    let v = &mut *(data.0 as *mut Vec<Win>);
    if !IsWindowVisible(h).as_bool() || IsIconic(h).as_bool() {
        return TRUE;
    }
    let mut cloaked = 0u32;
    let _ = DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, 4);
    if cloaked != 0 {
        return TRUE;
    }
    let mut r = RECT::default();
    if GetWindowRect(h, &mut r).is_err() {
        return TRUE;
    }
    let area = ((r.right - r.left) as i64) * ((r.bottom - r.top) as i64);
    if area < 160 * 120 {
        return TRUE;
    }
    let mut pid = 0u32;
    GetWindowThreadProcessId(h, Some(&mut pid));
    v.push(Win { hwnd: h, pid, monitor: MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST), area });
    TRUE
}

fn visible_windows() -> Vec<Win> {
    let mut v: Vec<Win> = Vec::with_capacity(64);
    unsafe {
        let _ = EnumWindows(Some(enum_win), LPARAM(&mut v as *mut _ as isize));
    }
    v
}

/// Formatowanie wartości (wspólne dla nakładki).
pub mod fmt {
    /// GB w szablonie „88.8” — od 100 GB bez części dziesiętnej, by nie zmieniać szerokości.
    pub fn bytes_gb(b: u64) -> String {
        let gb = b as f64 / 1024f64.powi(3);
        if gb < 99.95 { format!("{:.1}", gb) } else { format!("{:.0}", gb.min(999.0)) }
    }
    /// Szybkość transferu z dobraną jednostką.
    pub fn rate(bps: f64) -> (String, &'static str) {
        let kb = bps / 1024.0;
        if kb < 1000.0 {
            (format!("{:.0}", kb), "KB/s")
        } else {
            let mb = kb / 1024.0;
            if mb < 100.0 { (format!("{:.1}", mb), "MB/s") } else { (format!("{:.0}", mb), "MB/s") }
        }
    }
    pub fn duration(s: u64) -> String {
        let (h, m, sec) = (s / 3600, (s / 60) % 60, s % 60);
        if h > 0 { format!("{h}:{m:02}:{sec:02}") } else { format!("{m}:{sec:02}") }
    }
}
