//! Zgłoszenia do redzikowski.com — tak samo jak w pozostałych aplikacjach autora:
//!  - raport o awarii: zapisywany przy panice i proponowany do wysłania przy następnym
//!    uruchomieniu, wyłącznie za zgodą użytkownika (trafia do „Raportów błędów” w panelu),
//!  - błąd albo sugestia funkcjonalności pisane ręcznie (trafiają do „Sugestii i zgłoszeń”).
//! Adres e-mail jest zawsze opcjonalny, a dane o systemie dołączane tylko po zaznaczeniu.

use std::path::PathBuf;
use std::time::Duration;

use crate::config::{Config, Lang};
use crate::i18n::tr;
use crate::util::VERSION;

const ENDPOINT: &str = "https://redzikowski.com/api/feedback";
pub const SLUG: &str = "openfps";
pub const NAME: &str = "OpenFPS";
pub const USER_AGENT: &str = concat!("OpenFPS/", env!("CARGO_PKG_VERSION"), " (https://redzikowski.com)");
const CRASH_FILE: &str = "feedback-pending-crash.txt";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Bug,
    Feature,
    Crash,
}

impl Kind {
    fn id(self) -> &'static str {
        match self {
            Kind::Bug => "bug",
            Kind::Feature => "feature",
            Kind::Crash => "crash",
        }
    }
}

fn crash_path() -> PathBuf {
    Config::dir().join(CRASH_FILE)
}

/// Zapis awarii do pliku (panika w nakładce lub w ustawieniach). Wywołaj raz na starcie.
pub fn install_panic_hook(role: &'static str) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let bt = std::backtrace::Backtrace::force_capture();
        let thread = std::thread::current().name().unwrap_or("?").to_string();
        let text = format!("{NAME} {VERSION} · {role} · thread {thread}\n\n{info}\n\n{bt}");
        let _ = std::fs::create_dir_all(Config::dir());
        let _ = std::fs::write(crash_path(), text);
        previous(info);
    }));
}

/// Opis awarii z poprzedniego uruchomienia, jeśli czeka na decyzję użytkownika.
pub fn pending_crash() -> Option<String> {
    std::fs::read_to_string(crash_path()).ok().filter(|t| !t.trim().is_empty())
}

pub fn discard_crash() {
    let _ = std::fs::remove_file(crash_path());
}

pub fn email_ok(email: &str) -> bool {
    let e = email.trim();
    e.is_empty() || (e.contains('@') && e.rsplit('@').next().is_some_and(|d| d.contains('.')) && !e.contains(' '))
}

/// Nazwa i wersja systemu z rejestru, np. „Windows 11 Pro 24H2 (26100)”.
pub fn os_name() -> String {
    use windows::core::w;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
    let read = |name: windows::core::PCWSTR| -> String {
        let mut buf = [0u16; 128];
        let mut len = (buf.len() * 2) as u32;
        let r = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                w!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion"),
                name,
                RRF_RT_REG_SZ,
                None,
                Some(buf.as_mut_ptr() as *mut _),
                Some(&mut len),
            )
        };
        if r.is_ok() { crate::util::from_wide(&buf) } else { String::new() }
    };
    let build = read(w!("CurrentBuild"));
    let mut product = read(w!("ProductName"));
    // Windows 11 nadal podaje w rejestrze „Windows 10” — rozpoznajemy po numerze kompilacji.
    if build.parse::<u32>().unwrap_or(0) >= 22000 {
        product = product.replace("Windows 10", "Windows 11");
    }
    format!("{} {} ({}) {}", product, read(w!("DisplayVersion")), build, std::env::consts::ARCH).trim().to_string()
}

pub struct Report {
    pub kind: Kind,
    pub message: String,
    pub email: String,
    pub include_device: bool,
    pub stack: Option<String>,
    pub lang: Lang,
    /// Dodatkowy opis sprzętu (np. karta graficzna), tylko gdy `include_device`.
    pub device: String,
}

fn body(r: &Report) -> String {
    let mut m = serde_json::Map::new();
    m.insert("kind".into(), r.kind.id().into());
    m.insert("app".into(), SLUG.into());
    m.insert("appName".into(), NAME.into());
    m.insert("appVersion".into(), VERSION.into());
    m.insert("platform".into(), "windows".into());
    m.insert("locale".into(), if r.lang == Lang::Pl { "pl" } else { "en" }.into());
    m.insert("message".into(), r.message.trim().into());
    m.insert("email".into(), r.email.trim().into());
    if r.include_device || r.kind == Kind::Crash {
        m.insert("os".into(), os_name().into());
        m.insert("device".into(), r.device.clone().into());
        m.insert(
            "extra".into(),
            serde_json::json!({ "cpus": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0) }),
        );
    }
    if let Some(s) = &r.stack {
        m.insert("stack".into(), s.chars().take(60_000).collect::<String>().into());
    }
    serde_json::Value::Object(m).to_string()
}

/// Wysyła zgłoszenie (blokująco — wołaj z osobnego wątku).
pub fn send(r: &Report) -> Result<(), String> {
    let lang = r.lang;
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .user_agent(USER_AGENT)
        .build();
    match agent.post(ENDPOINT).set("Content-Type", "application/json; charset=utf-8").send_string(&body(r)) {
        Ok(_) => {
            if r.kind == Kind::Crash {
                discard_crash();
            }
            Ok(())
        }
        Err(ureq::Error::Status(429, _)) => Err(tr(lang, "Za dużo zgłoszeń — spróbuj później.", "Too many reports — try again later.").into()),
        Err(ureq::Error::Status(code, _)) => Err(format!("{} (HTTP {code}).", tr(lang, "Serwer odrzucił zgłoszenie", "The server rejected the report"))),
        Err(e) => Err(format!("{}: {e}", tr(lang, "Brak połączenia z redzikowski.com", "Can't reach redzikowski.com"))),
    }
}
