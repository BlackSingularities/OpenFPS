//! Liczniki wydajności (PDH). Używamy angielskich nazw liczników
//! (`PdhAddEnglishCounterW`), więc działa także na polskim Windowsie.
//! Dodawane są tylko liczniki potrzebne do włączonych metryk.

use std::collections::HashMap;
use windows::core::PCWSTR;
use windows::Win32::System::Performance::*;

use crate::util::wide;

const FMT: PDH_FMT = PDH_FMT(0x0000_0200 | 0x0000_8000); // DOUBLE | NOCAP100

pub struct Counter(isize);

pub struct Pdh {
    query: isize,
}

impl Pdh {
    pub fn new() -> Option<Self> {
        let mut q = 0isize;
        let r = unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut q) };
        (r == 0).then_some(Self { query: q })
    }

    pub fn add(&self, path: &str) -> Option<Counter> {
        let w = wide(path);
        let mut c = 0isize;
        let r = unsafe { PdhAddEnglishCounterW(self.query, PCWSTR(w.as_ptr()), 0, &mut c) };
        (r == 0).then_some(Counter(c))
    }

    pub fn collect(&self) {
        unsafe {
            PdhCollectQueryData(self.query);
        }
    }

    pub fn value(&self, c: &Counter) -> Option<f64> {
        let mut v = PDH_FMT_COUNTERVALUE::default();
        let r = unsafe { PdhGetFormattedCounterValue(c.0, FMT, None, &mut v) };
        if r != 0 || (v.CStatus != 0 && v.CStatus != 1) {
            return None;
        }
        Some(unsafe { v.Anonymous.doubleValue })
    }

    /// Wartości licznika z wieloma instancjami (wildcard `*`).
    pub fn array(&self, c: &Counter) -> Vec<(String, f64)> {
        let mut out = Vec::new();
        unsafe {
            let (mut size, mut count) = (0u32, 0u32);
            let r = PdhGetFormattedCounterArrayW(c.0, FMT, &mut size, &mut count, None);
            if r != PDH_MORE_DATA || size == 0 {
                return out;
            }
            // Bufor wyrównany do 8 bajtów (zawiera double).
            let mut buf = vec![0u64; (size as usize + 7) / 8];
            let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
            if PdhGetFormattedCounterArrayW(c.0, FMT, &mut size, &mut count, Some(items)) != 0 {
                return out;
            }
            for i in 0..count as usize {
                let it = &*items.add(i);
                if it.FmtValue.CStatus != 0 && it.FmtValue.CStatus != 1 {
                    continue;
                }
                let name = it.szName.to_string().unwrap_or_default();
                out.push((name, it.FmtValue.Anonymous.doubleValue));
            }
        }
        out
    }
}

impl Drop for Pdh {
    fn drop(&mut self) {
        unsafe {
            PdhCloseQuery(self.query);
        }
    }
}

// ───────────────────────────── Parsowanie GPU ─────────────────────────────

/// Instancja `pid_1234_luid_0x00000000_0x0000D1B1_phys_0_eng_3_engtype_3D`.
pub struct GpuEngine<'a> {
    pub pid: u32,
    pub luid: &'a str,
    pub engtype: &'a str,
}

pub fn parse_engine(name: &str) -> Option<GpuEngine<'_>> {
    let rest = name.strip_prefix("pid_")?;
    let (pid, rest) = rest.split_once('_')?;
    let rest = rest.strip_prefix("luid_")?;
    let luid_end = rest.find("_phys")?;
    let luid = &rest[..luid_end];
    let engtype = rest.rsplit_once("engtype_").map(|(_, t)| t).unwrap_or("");
    Some(GpuEngine { pid: pid.parse().ok()?, luid, engtype })
}

/// Instancja `luid_0x00000000_0x0000D1B1_phys_0`.
pub fn parse_adapter_luid(name: &str) -> Option<&str> {
    let rest = name.strip_prefix("luid_")?;
    Some(&rest[..rest.find("_phys").unwrap_or(rest.len())])
}

/// Użycie GPU wg Menedżera zadań: dla każdego typu silnika sumujemy instancje,
/// a wynik to maksimum po typach. Zwraca (adapter → %, pid → %).
pub fn gpu_usage(items: &[(String, f64)], luid_filter: Option<&str>) -> (f32, HashMap<u32, f32>) {
    let mut by_type: HashMap<&str, f64> = HashMap::new();
    let mut by_pid: HashMap<(u32, &str), f64> = HashMap::new();
    for (name, v) in items {
        let Some(e) = parse_engine(name) else { continue };
        if let Some(f) = luid_filter {
            if !e.luid.eq_ignore_ascii_case(f) {
                continue;
            }
        }
        *by_type.entry(e.engtype).or_default() += v;
        *by_pid.entry((e.pid, e.engtype)).or_default() += v;
    }
    let total = by_type.values().cloned().fold(0.0, f64::max).min(100.0) as f32;
    let mut per_pid: HashMap<u32, f32> = HashMap::new();
    for ((pid, _), v) in by_pid {
        let e = per_pid.entry(pid).or_default();
        *e = e.max(v.min(100.0) as f32);
    }
    (total, per_pid)
}
