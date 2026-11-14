//! Drobne narzędzia Win32 współdzielone przez nakładkę i okno ustawień.

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Registry::{RegDeleteKeyValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ};
use windows::Win32::UI::Shell::{IsUserAnAdmin, ShellExecuteW};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW, SW_HIDE, SW_SHOWNORMAL, WM_APP};

pub const HOST_CLASS: PCWSTR = w!("OpenFPS.Host");
pub const OVERLAY_CLASS: PCWSTR = w!("OpenFPS.Overlay");

pub const MSG_TRAY: u32 = WM_APP + 1;
pub const MSG_RELOAD: u32 = WM_APP + 10;
pub const MSG_OPEN_SETTINGS: u32 = WM_APP + 11;
pub const MSG_SUSPEND_HOTKEYS: u32 = WM_APP + 12;
pub const MSG_QUIT: u32 = WM_APP + 13;
/// Wątek sprawdzania aktualizacji znalazł nowszą wersję.
pub const MSG_UPDATE_AVAILABLE: u32 = WM_APP + 14;

/// Wersja programu: z tagu wydania (CI ustawia `OPENFPS_VERSION`) albo z Cargo.toml.
pub const VERSION: &str = match option_env!("OPENFPS_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

pub fn is_elevated() -> bool {
    unsafe { IsUserAnAdmin().as_bool() }
}

pub fn exe_path() -> String {
    std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Uruchamia ten program ponownie z uprawnieniami administratora (UAC).
/// Zwraca `true`, jeśli użytkownik zgodził się na podniesienie uprawnień.
pub fn relaunch_elevated(args: &str) -> bool {
    let exe = HSTRING::from(exe_path());
    let args = HSTRING::from(args);
    let r = unsafe { ShellExecuteW(HWND::default(), w!("runas"), &exe, &args, PCWSTR::null(), SW_SHOWNORMAL) };
    r.0 as isize > 32
}

pub fn host_window() -> Option<HWND> {
    let h = unsafe { FindWindowW(HOST_CLASS, PCWSTR::null()) }.ok()?;
    (!h.is_invalid()).then_some(h)
}

pub fn overlay_running() -> bool {
    host_window().is_some()
}

pub fn post_to_overlay(msg: u32, wparam: usize) -> bool {
    match host_window() {
        Some(h) => unsafe { PostMessageW(h, msg, WPARAM(wparam), LPARAM(0)).is_ok() },
        None => false,
    }
}

pub fn spawn_self(args: &[&str]) {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).args(args).spawn();
    }
}

pub fn open_in_explorer(path: &std::path::Path) {
    let p = HSTRING::from(path.as_os_str());
    unsafe {
        ShellExecuteW(HWND::default(), w!("open"), &p, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

// ───────────────────────────── Autostart ─────────────────────────────

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const TASK_NAME: &str = "OpenFPS";

/// Autostart przez klucz Run (bez uprawnień administratora).
pub fn set_run_key(enabled: bool) {
    unsafe {
        if enabled {
            let cmd = wide(&format!("\"{}\" --autostart", exe_path()));
            let bytes = (cmd.len() * 2) as u32;
            let _ = RegSetKeyValueW(HKEY_CURRENT_USER, RUN_KEY, w!("OpenFPS"), REG_SZ.0, Some(cmd.as_ptr() as *const _), bytes);
        } else {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, w!("OpenFPS"));
        }
    }
}

/// Autostart z najwyższymi uprawnieniami przez Harmonogram zadań
/// (Windows pomija wpisy Run wymagające UAC). Wywołuje UAC.
pub fn set_elevated_task(enabled: bool) -> bool {
    let args = if enabled {
        format!(
            "/Create /F /TN {TASK_NAME} /SC ONLOGON /RL HIGHEST /TR \"\\\"{}\\\" --autostart\"",
            exe_path()
        )
    } else {
        format!("/Delete /F /TN {TASK_NAME}")
    };
    let args = HSTRING::from(args);
    let verb = if is_elevated() { w!("open") } else { w!("runas") };
    let r = unsafe { ShellExecuteW(HWND::default(), verb, w!("schtasks.exe"), &args, PCWSTR::null(), SW_HIDE) };
    r.0 as isize > 32
}

/// Stosuje ustawienia autostartu (wywoływane z okna ustawień po zmianie).
pub fn apply_autostart(autostart: bool, as_admin: bool, prev_as_admin_task: bool) {
    if autostart && as_admin {
        set_run_key(false);
        set_elevated_task(true);
    } else {
        set_run_key(autostart);
        if prev_as_admin_task {
            set_elevated_task(false);
        }
    }
}
