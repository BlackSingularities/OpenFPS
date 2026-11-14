//! Wyliczanie monitorów: geometria, DPI, odświeżanie i przyjazne nazwy (np. „DELL U2720Q”).

use std::collections::HashMap;
use windows::core::PCWSTR;
use windows::Win32::Devices::Display::{
    DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig, DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
    DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME,
    DISPLAYCONFIG_TARGET_DEVICE_NAME, QDC_ONLY_ACTIVE_PATHS,
};
use windows::Win32::Foundation::{BOOL, LPARAM, RECT, TRUE};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, EnumDisplaySettingsW, GetMonitorInfoW, DEVMODEW, ENUM_CURRENT_SETTINGS, HDC, HMONITOR, MONITORINFO,
    MONITORINFOEXW,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};

use crate::util::{from_wide, wide};

#[derive(Clone, Debug)]
pub struct MonitorInfo {
    pub handle: HMONITOR,
    /// `\\.\DISPLAYn` — stabilny identyfikator w obrębie sesji.
    pub id: String,
    pub name: String,
    pub rect: RECT,
    pub work: RECT,
    pub primary: bool,
    pub dpi: u32,
    pub refresh_hz: u32,
}

impl MonitorInfo {
    pub fn width(&self) -> i32 {
        self.rect.right - self.rect.left
    }
    pub fn height(&self) -> i32 {
        self.rect.bottom - self.rect.top
    }
    /// Etykieta do UI, np. „DELL U2720Q · 2560×1440 @ 144 Hz”.
    pub fn label(&self) -> String {
        format!("{} · {}×{} @ {} Hz", self.name, self.width(), self.height(), self.refresh_hz)
    }
}

unsafe extern "system" fn enum_proc(h: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
    let v = &mut *(data.0 as *mut Vec<HMONITOR>);
    v.push(h);
    TRUE
}

pub fn enumerate() -> Vec<MonitorInfo> {
    let mut handles: Vec<HMONITOR> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(HDC::default(), None, Some(enum_proc), LPARAM(&mut handles as *mut _ as isize));
    }
    let names = friendly_names();
    let mut out = Vec::new();
    for (idx, h) in handles.into_iter().enumerate() {
        unsafe {
            let mut mi = MONITORINFOEXW::default();
            mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
            if !GetMonitorInfoW(h, &mut mi as *mut _ as *mut MONITORINFO).as_bool() {
                continue;
            }
            let id = from_wide(&mi.szDevice);
            let (mut dx, mut dy) = (96u32, 96u32);
            let _ = GetDpiForMonitor(h, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
            let mut dm = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
            let devw = wide(&id);
            let refresh = if EnumDisplaySettingsW(PCWSTR(devw.as_ptr()), ENUM_CURRENT_SETTINGS, &mut dm).as_bool() {
                dm.dmDisplayFrequency
            } else {
                0
            };
            let name = names.get(&id).cloned().filter(|n| !n.is_empty()).unwrap_or_else(|| format!("Monitor {}", idx + 1));
            out.push(MonitorInfo {
                handle: h,
                id,
                name,
                rect: mi.monitorInfo.rcMonitor,
                work: mi.monitorInfo.rcWork,
                primary: mi.monitorInfo.dwFlags & 1 != 0,
                dpi: dx.max(48),
                refresh_hz: refresh,
            });
        }
    }
    // Główny monitor pierwszy, potem od lewej do prawej.
    out.sort_by_key(|m| (!m.primary, m.rect.left, m.rect.top));
    out
}

/// Mapa `\\.\DISPLAYn` → przyjazna nazwa monitora z EDID.
fn friendly_names() -> HashMap<String, String> {
    let mut map = HashMap::new();
    unsafe {
        let (mut np, mut nm) = (0u32, 0u32);
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm).is_err() {
            return map;
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
        if QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut np, paths.as_mut_ptr(), &mut nm, modes.as_mut_ptr(), None).is_err() {
            return map;
        }
        for p in paths.iter().take(np as usize) {
            let mut src = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
            src.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
            src.header.size = std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
            src.header.adapterId = p.sourceInfo.adapterId;
            src.header.id = p.sourceInfo.id;
            if DisplayConfigGetDeviceInfo(&mut src.header) != 0 {
                continue;
            }
            let mut tgt = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
            tgt.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
            tgt.header.size = std::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
            tgt.header.adapterId = p.targetInfo.adapterId;
            tgt.header.id = p.targetInfo.id;
            if DisplayConfigGetDeviceInfo(&mut tgt.header) != 0 {
                continue;
            }
            map.insert(from_wide(&src.viewGdiDeviceName), from_wide(&tgt.monitorFriendlyDeviceName));
        }
    }
    map
}
