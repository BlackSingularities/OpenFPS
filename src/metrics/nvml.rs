//! Czujniki kart NVIDIA przez NVML (nvml.dll instalowany ze sterownikiem).
//! Biblioteka ładowana dynamicznie — brak NVIDII = po prostu brak danych.

use std::ffi::c_void;
use windows::core::{s, w};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

type Dev = *mut c_void;
type FnInit = unsafe extern "C" fn() -> i32;
type FnHandle = unsafe extern "C" fn(u32, *mut Dev) -> i32;
type FnU32 = unsafe extern "C" fn(Dev, *mut u32) -> i32;
type FnU32Arg = unsafe extern "C" fn(Dev, u32, *mut u32) -> i32;
type FnName = unsafe extern "C" fn(Dev, *mut u8, u32) -> i32;

#[repr(C)]
#[derive(Default)]
struct Utilization {
    gpu: u32,
    memory: u32,
}
type FnUtil = unsafe extern "C" fn(Dev, *mut Utilization) -> i32;

#[repr(C)]
#[derive(Default)]
struct Memory {
    total: u64,
    free: u64,
    used: u64,
}
type FnMem = unsafe extern "C" fn(Dev, *mut Memory) -> i32;

pub struct Nvml {
    dev: Dev,
    temp: Option<FnU32Arg>,
    clock: Option<FnU32Arg>,
    power: Option<FnU32>,
    fan: Option<FnU32>,
    util: Option<FnUtil>,
    mem: Option<FnMem>,
    pub name: String,
}

/// Które czujniki odczytać — każde wywołanie NVML kosztuje, więc pytamy tylko o potrzebne.
#[derive(Clone, Copy, Default)]
pub struct Which {
    pub temp: bool,
    pub clock: bool,
    pub power: bool,
    pub fan: bool,
    pub util: bool,
    pub mem: bool,
}

#[derive(Default, Clone, Debug)]
pub struct NvmlSample {
    pub temp_c: Option<u32>,
    pub clock_mhz: Option<u32>,
    pub power_w: Option<f32>,
    pub fan_pct: Option<u32>,
    pub util_pct: Option<u32>,
    pub mem_used: Option<u64>,
    pub mem_total: Option<u64>,
}

unsafe fn sym<T>(lib: HMODULE, name: windows::core::PCSTR) -> Option<T> {
    GetProcAddress(lib, name).map(|f| std::mem::transmute_copy(&f))
}

impl Nvml {
    pub fn load() -> Option<Nvml> {
        unsafe {
            let lib = LoadLibraryW(w!("nvml.dll"))
                .or_else(|_| LoadLibraryW(w!("C:\\Program Files\\NVIDIA Corporation\\NVSMI\\nvml.dll")))
                .ok()?;
            let init: FnInit = sym(lib, s!("nvmlInit_v2"))?;
            if init() != 0 {
                return None;
            }
            let get: FnHandle = sym(lib, s!("nvmlDeviceGetHandleByIndex_v2"))?;
            let mut dev: Dev = std::ptr::null_mut();
            if get(0, &mut dev) != 0 {
                return None;
            }
            let mut name = String::new();
            if let Some(f) = sym::<FnName>(lib, s!("nvmlDeviceGetName")) {
                let mut buf = [0u8; 96];
                if f(dev, buf.as_mut_ptr(), buf.len() as u32) == 0 {
                    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
                    name = String::from_utf8_lossy(&buf[..end]).into_owned();
                }
            }
            Some(Nvml {
                dev,
                temp: sym(lib, s!("nvmlDeviceGetTemperature")),
                clock: sym(lib, s!("nvmlDeviceGetClockInfo")),
                power: sym(lib, s!("nvmlDeviceGetPowerUsage")),
                fan: sym(lib, s!("nvmlDeviceGetFanSpeed")),
                util: sym(lib, s!("nvmlDeviceGetUtilizationRates")),
                mem: sym(lib, s!("nvmlDeviceGetMemoryInfo")),
                name,
            })
        }
    }

    pub fn sample(&self, w: Which) -> NvmlSample {
        let mut s = NvmlSample::default();
        unsafe {
            let mut v = 0u32;
            if let (Some(f), true) = (self.temp, w.temp) {
                if f(self.dev, 0, &mut v) == 0 {
                    s.temp_c = Some(v);
                }
            }
            if let (Some(f), true) = (self.clock, w.clock) {
                if f(self.dev, 0, &mut v) == 0 {
                    s.clock_mhz = Some(v);
                }
            }
            if let (Some(f), true) = (self.power, w.power) {
                if f(self.dev, &mut v) == 0 {
                    s.power_w = Some(v as f32 / 1000.0);
                }
            }
            if let (Some(f), true) = (self.fan, w.fan) {
                if f(self.dev, &mut v) == 0 {
                    s.fan_pct = Some(v);
                }
            }
            if let (Some(f), true) = (self.util, w.util) {
                let mut u = Utilization::default();
                if f(self.dev, &mut u) == 0 {
                    s.util_pct = Some(u.gpu);
                }
            }
            if let (Some(f), true) = (self.mem, w.mem) {
                let mut m = Memory::default();
                if f(self.dev, &mut m) == 0 {
                    s.mem_used = Some(m.used);
                    s.mem_total = Some(m.total);
                }
            }
        }
        s
    }
}
