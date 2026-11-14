//! Tłumaczenia PL / EN. Teksty są trzymane parami przy użyciu,
//! a nazwy metryk — centralnie tutaj, żeby nakładka i ustawienia mówiły tym samym językiem.

use crate::config::{Lang, Metric};

#[inline]
pub fn tr(lang: Lang, pl: &'static str, en: &'static str) -> &'static str {
    match lang {
        Lang::Pl => pl,
        Lang::En => en,
    }
}

/// Krótka etykieta widoczna na nakładce.
pub fn short_label(m: Metric, lang: Lang) -> &'static str {
    use Metric::*;
    match m {
        Fps => "FPS",
        Frametime => tr(lang, "Klatka", "Frame"),
        Low1 => "1% low",
        Low01 => "0.1% low",
        FrametimeGraph => tr(lang, "Czas klatki", "Frametime"),
        FpsGraph => tr(lang, "Historia FPS", "FPS history"),
        PlayTime => tr(lang, "Sesja", "Session"),
        CpuUsage => "CPU",
        CpuClock => tr(lang, "CPU zegar", "CPU clock"),
        CpuCores => tr(lang, "Rdzenie", "Cores"),
        GpuUsage => "GPU",
        GpuAppUsage => tr(lang, "GPU app", "GPU app"),
        GpuTemp => tr(lang, "GPU temp", "GPU temp"),
        GpuClock => tr(lang, "GPU zegar", "GPU clock"),
        GpuPower => tr(lang, "GPU moc", "GPU power"),
        GpuFan => tr(lang, "GPU went.", "GPU fan"),
        Vram => "VRAM",
        Ram => "RAM",
        Network => tr(lang, "Sieć", "Net"),
        Disk => tr(lang, "Dysk", "Disk"),
        Battery => tr(lang, "Bateria", "Battery"),
        Display => tr(lang, "Ekran", "Display"),
        Clock => tr(lang, "Godzina", "Time"),
        Uptime => tr(lang, "Czas pracy", "Uptime"),
        Unknown => "",
    }
}

/// Pełna nazwa i opis do okna ustawień.
pub fn describe(m: Metric, lang: Lang) -> (&'static str, &'static str) {
    use Metric::*;
    match m {
        Fps => (tr(lang, "Klatki na sekundę", "Frames per second"), tr(lang, "Średnia liczba klatek z ostatniego okna uśredniania", "Average frame rate over the averaging window")),
        Frametime => (tr(lang, "Czas klatki", "Frametime"), tr(lang, "Średni czas generowania jednej klatki w ms", "Average time to render one frame, in ms")),
        Low1 => ("1% low", tr(lang, "FPS najwolniejszego 1% klatek — pokazuje przycięcia", "FPS of the slowest 1% of frames — reveals stutter")),
        Low01 => ("0.1% low", tr(lang, "FPS najwolniejszego 0,1% klatek — pojedyncze zacięcia", "FPS of the slowest 0.1% of frames — single hitches")),
        FrametimeGraph => (tr(lang, "Wykres czasu klatki", "Frametime graph"), tr(lang, "Przebieg czasu klatek w czasie rzeczywistym", "Live plot of frame times")),
        FpsGraph => (tr(lang, "Wykres FPS", "FPS graph"), tr(lang, "Historia liczby klatek na sekundę", "History of the frame rate")),
        PlayTime => (tr(lang, "Czas sesji", "Session time"), tr(lang, "Jak długo aplikacja renderuje klatki", "How long the app has been rendering")),
        CpuUsage => (tr(lang, "Użycie CPU", "CPU usage"), tr(lang, "Całkowite obciążenie procesora (jak w Menedżerze zadań)", "Total processor load (matches Task Manager)")),
        CpuClock => (tr(lang, "Zegar CPU", "CPU clock"), tr(lang, "Efektywna częstotliwość procesora", "Effective processor frequency")),
        CpuCores => (tr(lang, "Rdzenie CPU", "CPU cores"), tr(lang, "Obciążenie każdego wątku logicznego", "Load of each logical thread")),
        GpuUsage => (tr(lang, "Użycie GPU", "GPU usage"), tr(lang, "Obciążenie najbardziej zajętego silnika karty", "Load of the busiest GPU engine")),
        GpuAppUsage => (tr(lang, "Użycie GPU przez aplikację", "App GPU usage"), tr(lang, "Ile GPU zużywa mierzona aplikacja", "GPU used by the measured application")),
        GpuTemp => (tr(lang, "Temperatura GPU", "GPU temperature"), tr(lang, "Wymaga karty NVIDIA (NVML)", "Requires an NVIDIA GPU (NVML)")),
        GpuClock => (tr(lang, "Zegar GPU", "GPU clock"), tr(lang, "Taktowanie rdzenia graficznego (NVIDIA)", "Graphics core clock (NVIDIA)")),
        GpuPower => (tr(lang, "Pobór mocy GPU", "GPU power"), tr(lang, "Aktualny pobór mocy karty (NVIDIA)", "Current board power draw (NVIDIA)")),
        GpuFan => (tr(lang, "Wentylator GPU", "GPU fan"), tr(lang, "Prędkość wentylatora w % (NVIDIA)", "Fan speed in % (NVIDIA)")),
        Vram => ("VRAM", tr(lang, "Zajęta / całkowita pamięć karty", "Used / total video memory")),
        Ram => ("RAM", tr(lang, "Zajęta / całkowita pamięć systemu", "Used / total system memory")),
        Network => (tr(lang, "Sieć", "Network"), tr(lang, "Pobieranie i wysyłanie", "Download and upload")),
        Disk => (tr(lang, "Dysk", "Disk"), tr(lang, "Odczyt i zapis wszystkich dysków", "Read and write across all disks")),
        Battery => (tr(lang, "Bateria", "Battery"), tr(lang, "Poziom naładowania (laptopy)", "Charge level (laptops)")),
        Display => (tr(lang, "Ekran", "Display"), tr(lang, "Rozdzielczość i odświeżanie tego monitora", "Resolution and refresh rate of this monitor")),
        Clock => (tr(lang, "Godzina", "Clock"), tr(lang, "Aktualny czas systemowy", "Current system time")),
        Uptime => (tr(lang, "Czas pracy systemu", "System uptime"), tr(lang, "Od ostatniego uruchomienia Windows", "Since Windows last started")),
        Unknown => ("", ""),
    }
}
