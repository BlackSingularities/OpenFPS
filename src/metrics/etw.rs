//! Pomiar FPS przez Event Tracing for Windows (ta sama metoda co PresentMon).
//!
//! Nasłuchujemy zdarzeń „Present” z dostawców DXGI (DX10–12), D3D9 oraz DxgKrnl
//! (OpenGL / Vulkan / pozostałe). Nic nie jest wstrzykiwane do gier, więc metoda
//! jest bezpieczna dla systemów anty-cheat. Wymaga uprawnień administratora
//! (lub członkostwa w grupie „Performance Log Users”).

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use std::sync::Mutex;

use windows::core::{GUID, PCWSTR, PWSTR};
use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_SUCCESS};
use windows::Win32::System::Diagnostics::Etw::*;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

use crate::util::wide;

const SESSION_NAME: &str = "OpenFPS-PresentTrace";

const DXGI: GUID = GUID::from_u128(0xCA11C036_0102_4A2D_A6AD_F03CFED5D3C9);
const D3D9: GUID = GUID::from_u128(0x783ACA0A_790E_4D7F_8451_AA850511C6B9);
const DXGKRNL: GUID = GUID::from_u128(0x802EC45A_1E99_4B83_9920_87C98277BA9D);

// Identyfikatory zdarzeń.
const DXGI_PRESENT_START: u16 = 42;
const DXGI_PRESENT_MPO_START: u16 = 55;
const D3D9_PRESENT_START: u16 = 1;
const DXGK_BLIT: u16 = 166;
const DXGK_FLIP: u16 = 168;
const DXGK_PRESENT_HISTORY: u16 = 171;
const DXGK_PRESENT_HISTORY_DETAILED: u16 = 215;

/// Źródła zdarzeń – liczymy je osobno, by nie dublować klatek.
const SRC_DXGI: usize = 0;
const SRC_D3D9: usize = 1;
const SRC_FLIP: usize = 2;
const SRC_BLIT: usize = 3;
const SRC_HIST: usize = 4;
const SRC_HIST_D: usize = 5;
const SOURCES: usize = 6;

/// Maksymalny wiek przechowywanych znaczników (s) i limit na proces.
const MAX_FRAMES: usize = 20_000;

/// Ile sekund historii klatek trzymać (= najdłuższe okno potrzebne do statystyk).
/// Mniejsza retencja = mniej pamięci i szybsze liczenie percentyli.
static RETENTION_S: AtomicU32 = AtomicU32::new(12);

pub fn set_retention(secs: u32) {
    RETENTION_S.store(secs.clamp(3, 125), Ordering::Relaxed);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum EtwStatus {
    Starting = 0,
    Running = 1,
    NeedsAdmin = 2,
    Failed = 3,
}

static STATUS: AtomicU8 = AtomicU8::new(EtwStatus::Starting as u8);

pub fn status() -> EtwStatus {
    match STATUS.load(Ordering::Relaxed) {
        1 => EtwStatus::Running,
        2 => EtwStatus::NeedsAdmin,
        3 => EtwStatus::Failed,
        _ => EtwStatus::Starting,
    }
}

#[derive(Default)]
struct ProcFrames {
    src: [VecDeque<i64>; SOURCES],
    first_seen: i64,
}

static FRAMES: Mutex<Option<HashMap<u32, ProcFrames>>> = Mutex::new(None);

unsafe extern "system" fn on_event(rec: *mut EVENT_RECORD) {
    let h = &(*rec).EventHeader;
    let id = h.EventDescriptor.Id;
    let src = if h.ProviderId == DXGI {
        match id {
            DXGI_PRESENT_START | DXGI_PRESENT_MPO_START => SRC_DXGI,
            _ => return,
        }
    } else if h.ProviderId == D3D9 {
        if id != D3D9_PRESENT_START {
            return;
        }
        SRC_D3D9
    } else if h.ProviderId == DXGKRNL {
        match id {
            DXGK_FLIP => SRC_FLIP,
            DXGK_BLIT => SRC_BLIT,
            DXGK_PRESENT_HISTORY => SRC_HIST,
            DXGK_PRESENT_HISTORY_DETAILED => SRC_HIST_D,
            _ => return,
        }
    } else {
        return;
    };
    let pid = h.ProcessId;
    let ts = h.TimeStamp;
    if let Ok(mut g) = FRAMES.lock() {
        let map = g.get_or_insert_with(HashMap::new);
        let p = map.entry(pid).or_insert_with(|| ProcFrames { first_seen: ts, ..Default::default() });
        let q = &mut p.src[src];
        q.push_back(ts);
        if q.len() > MAX_FRAMES {
            q.pop_front();
        }
    }
}

/// Sesja ETW. Zatrzymywana przy `Drop`.
pub struct PresentTrace {
    props: Vec<u8>,
    handle: CONTROLTRACE_HANDLE,
    thread: Option<std::thread::JoinHandle<()>>,
}

fn new_props() -> Vec<u8> {
    let base = std::mem::size_of::<EVENT_TRACE_PROPERTIES>();
    let total = base + 1024 * 2;
    let mut buf = vec![0u8; total];
    unsafe {
        let p = &mut *(buf.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES);
        p.Wnode.BufferSize = total as u32;
        p.Wnode.Flags = WNODE_FLAG_TRACED_GUID;
        p.Wnode.ClientContext = 1; // znaczniki czasu w jednostkach QPC
        p.LogFileMode = EVENT_TRACE_REAL_TIME_MODE;
        p.BufferSize = 16; // KB
        p.MinimumBuffers = 4;
        p.MaximumBuffers = 32;
        p.FlushTimer = 1;
        p.LoggerNameOffset = base as u32;
    }
    buf
}

fn enable(handle: CONTROLTRACE_HANDLE, guid: &GUID, any_keyword: u64, ids: &[u16]) -> bool {
    // EVENT_FILTER_EVENT_ID: FilterIn(u8) Reserved(u8) Count(u16) Events[u16]
    let mut filter: Vec<u8> = Vec::with_capacity(4 + ids.len() * 2);
    filter.push(1);
    filter.push(0);
    filter.extend_from_slice(&(ids.len() as u16).to_le_bytes());
    for id in ids {
        filter.extend_from_slice(&id.to_le_bytes());
    }
    let mut desc = EVENT_FILTER_DESCRIPTOR { Ptr: filter.as_ptr() as u64, Size: filter.len() as u32, Type: EVENT_FILTER_TYPE_EVENT_ID };
    let params = ENABLE_TRACE_PARAMETERS {
        Version: ENABLE_TRACE_PARAMETERS_VERSION_2,
        EnableFilterDesc: &mut desc,
        FilterDescCount: 1,
        ..Default::default()
    };
    let r = unsafe {
        EnableTraceEx2(handle, guid, EVENT_CONTROL_CODE_ENABLE_PROVIDER.0, 4, any_keyword, 0, 0, Some(&params))
    };
    r == ERROR_SUCCESS
}

impl PresentTrace {
    pub fn start() -> Option<PresentTrace> {
        let name = wide(SESSION_NAME);
        unsafe {
            // Zatrzymaj osieroconą sesję po ewentualnym crashu.
            let mut stale = new_props();
            let _ = ControlTraceW(
                CONTROLTRACE_HANDLE::default(),
                PCWSTR(name.as_ptr()),
                stale.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES,
                EVENT_TRACE_CONTROL_STOP,
            );

            let mut props = new_props();
            let mut handle = CONTROLTRACE_HANDLE::default();
            let r = StartTraceW(&mut handle, PCWSTR(name.as_ptr()), props.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES);
            if r != ERROR_SUCCESS {
                let st = if r == ERROR_ACCESS_DENIED { EtwStatus::NeedsAdmin } else { EtwStatus::Failed };
                STATUS.store(st as u8, Ordering::Relaxed);
                return None;
            }

            let ok_dxgi = enable(handle, &DXGI, u64::MAX, &[DXGI_PRESENT_START, DXGI_PRESENT_MPO_START]);
            let ok_d3d9 = enable(handle, &D3D9, u64::MAX, &[D3D9_PRESENT_START]);
            // Base | Present | kanał Performance
            let ok_dxgk = enable(
                handle,
                &DXGKRNL,
                0x1 | 0x800_0000 | 0x4000_0000_0000_0000,
                &[DXGK_BLIT, DXGK_FLIP, DXGK_PRESENT_HISTORY, DXGK_PRESENT_HISTORY_DETAILED],
            );
            if !(ok_dxgi || ok_d3d9 || ok_dxgk) {
                let _ = ControlTraceW(handle, PCWSTR::null(), props.as_mut_ptr() as *mut _, EVENT_TRACE_CONTROL_STOP);
                STATUS.store(EtwStatus::Failed as u8, Ordering::Relaxed);
                return None;
            }

            let thread = std::thread::Builder::new()
                .name("etw-consumer".into())
                .spawn(move || {
                    let mut name = wide(SESSION_NAME);
                    let mut log = EVENT_TRACE_LOGFILEW::default();
                    log.LoggerName = PWSTR(name.as_mut_ptr());
                    log.Anonymous1.ProcessTraceMode =
                        PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD | PROCESS_TRACE_MODE_RAW_TIMESTAMP;
                    log.Anonymous2.EventRecordCallback = Some(on_event);
                    let th = OpenTraceW(&mut log);
                    if th.Value == u64::MAX {
                        STATUS.store(EtwStatus::Failed as u8, Ordering::Relaxed);
                        return;
                    }
                    STATUS.store(EtwStatus::Running as u8, Ordering::Relaxed);
                    let _ = ProcessTrace(&[th], None, None);
                    let _ = CloseTrace(th);
                })
                .ok()?;

            Some(PresentTrace { props, handle, thread: Some(thread) })
        }
    }

    /// Wymusza dostarczenie buforów — mniejsze opóźnienie odczytów.
    pub fn flush(&mut self) {
        unsafe {
            let _ = ControlTraceW(self.handle, PCWSTR::null(), self.props.as_mut_ptr() as *mut _, EVENT_TRACE_CONTROL_FLUSH);
        }
    }
}

impl Drop for PresentTrace {
    fn drop(&mut self) {
        unsafe {
            let _ = ControlTraceW(self.handle, PCWSTR::null(), self.props.as_mut_ptr() as *mut _, EVENT_TRACE_CONTROL_STOP);
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

// ───────────────────────────── Statystyki ─────────────────────────────

#[derive(Clone, Debug, Default)]
pub struct FrameStats {
    pub fps: f32,
    pub frametime_ms: f32,
    pub low1: f32,
    pub low01: f32,
    /// Czasy klatek (ms) z ostatnich `graph_seconds`, najstarsze pierwsze.
    pub history: Vec<f32>,
    pub session_s: f64,
}

pub struct Clock {
    freq: f64,
}

impl Clock {
    pub fn new() -> Self {
        let mut f = 0i64;
        unsafe {
            let _ = QueryPerformanceFrequency(&mut f);
        }
        Self { freq: f.max(1) as f64 }
    }
    pub fn now(&self) -> i64 {
        let mut t = 0i64;
        unsafe {
            let _ = QueryPerformanceCounter(&mut t);
        }
        t
    }
    pub fn secs(&self, ticks: i64) -> f64 {
        ticks as f64 / self.freq
    }
}

/// Procesy, które prezentowały klatki w ciągu ostatnich `max_idle_s` sekund.
pub fn active_pids(clock: &Clock, max_idle_s: f64) -> Vec<u32> {
    let now = clock.now();
    let Ok(mut g) = FRAMES.lock() else { return Vec::new() };
    let Some(map) = g.as_mut() else { return Vec::new() };
    let cutoff = now - (RETENTION_S.load(Ordering::Relaxed) as f64 * clock.freq) as i64;
    map.retain(|_, p| {
        for q in p.src.iter_mut() {
            while q.front().is_some_and(|&t| t < cutoff) {
                q.pop_front();
            }
        }
        p.src.iter().any(|q| !q.is_empty())
    });
    let idle = (max_idle_s * clock.freq) as i64;
    map.iter()
        .filter(|(_, p)| p.src.iter().filter_map(|q| q.back()).any(|&t| now - t < idle))
        .map(|(pid, _)| *pid)
        .collect()
}

fn pick_source(p: &ProcFrames, now: i64, clock: &Clock) -> usize {
    let recent = |s: usize| -> usize {
        let since = now - (2.0 * clock.freq) as i64;
        p.src[s].iter().rev().take_while(|&&t| t >= since).count()
    };
    if recent(SRC_DXGI) > 0 {
        return SRC_DXGI;
    }
    if recent(SRC_D3D9) > 0 {
        return SRC_D3D9;
    }
    // OpenGL / Vulkan: wybierz najbardziej „gęste” źródło z DxgKrnl.
    [SRC_FLIP, SRC_BLIT, SRC_HIST, SRC_HIST_D].into_iter().max_by_key(|&s| recent(s)).unwrap_or(SRC_FLIP)
}

pub fn stats(pid: u32, clock: &Clock, average_ms: u32, lows_s: u32, graph_s: u32) -> Option<FrameStats> {
    let now = clock.now();
    let g = FRAMES.lock().ok()?;
    let p = g.as_ref()?.get(&pid)?;
    let q = &p.src[pick_source(p, now, clock)];
    let last = *q.back()?;
    // Brak klatek przez 1,5 s → aplikacja przestała renderować.
    if clock.secs(now - last) > 1.5 {
        return None;
    }
    let ticks = |s: f64| (s * clock.freq) as i64;
    let horizon = ticks((lows_s.max(graph_s) as f64).max(average_ms as f64 / 1000.0));
    // Okno liczymy względem ostatniej klatki, bo zdarzenia dochodzą z opóźnieniem.
    let mut stamps: Vec<i64> = q.iter().rev().take_while(|&&t| t >= last - horizon).copied().collect();
    stamps.reverse();
    // Rozgrzewka: pierwsze pojedyncze klatki dają bezsensowne skoki FPS.
    if stamps.len() < 4 || clock.secs(last - stamps[0]) < 0.25 {
        return None;
    }
    let ft: Vec<(i64, f32)> = stamps
        .windows(2)
        .filter(|w| w[1] > w[0])
        .map(|w| (w[1], (clock.secs(w[1] - w[0]) * 1000.0) as f32))
        .collect();
    if ft.is_empty() {
        return None;
    }

    let avg_from = last - ticks(average_ms as f64 / 1000.0);
    let avg: Vec<f32> = ft.iter().filter(|(t, _)| *t >= avg_from).map(|(_, v)| *v).collect();
    let avg = if avg.is_empty() { vec![ft.last().unwrap().1] } else { avg };
    let mean_ft = avg.iter().sum::<f32>() / avg.len() as f32;

    let lows_from = last - ticks(lows_s as f64);
    let mut lows: Vec<f32> = ft.iter().filter(|(t, _)| *t >= lows_from).map(|(_, v)| *v).collect();
    lows.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pct = |p: f64| -> f32 {
        let i = ((lows.len() as f64 - 1.0) * p).round() as usize;
        let v = lows[i.min(lows.len() - 1)];
        if v > 0.0 { 1000.0 / v } else { 0.0 }
    };

    let graph_from = last - ticks(graph_s as f64);
    let history: Vec<f32> = ft.iter().filter(|(t, _)| *t >= graph_from).map(|(_, v)| *v).collect();

    Some(FrameStats {
        fps: if mean_ft > 0.0 { 1000.0 / mean_ft } else { 0.0 },
        frametime_ms: mean_ft,
        low1: pct(0.99),
        low01: pct(0.999),
        history,
        session_s: clock.secs(last - p.first_seen),
    })
}
