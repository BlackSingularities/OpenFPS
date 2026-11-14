//! Aktualizacje z portfolio redzikowski.com. Serwer podaje wydania w formacie GitHub API,
//! więc program nie potrzebuje żadnego klucza.
//!
//! Wydanie zawiera przenośny `OpenFPS-<wersja>.exe` i `SHA256SUMS.txt`. Nowy plik pobieramy
//! obok obecnego, sprawdzamy sumę, a potem podmieniamy: działający .exe można w Windows
//! przemianować (na `.old`), więc podmiana nie wymaga instalatora ani uprawnień administratora,
//! o ile folder programu jest zapisywalny.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::feedback::USER_AGENT;
use crate::util::VERSION;

const FEED: &str = "https://redzikowski.com/api/releases/repos/BlackSingularities/OpenFPS/releases/latest";

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    asset_url: String,
    asset_name: String,
    sums_url: Option<String>,
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout(Duration::from_secs(120))
        .user_agent(USER_AGENT)
        .build()
}

fn get(url: &str, accept: &str) -> Result<ureq::Response, String> {
    agent().get(url).set("Accept", accept).call().map_err(|e| match e {
        e => e.to_string(),
    })
}

fn parse(v: &str) -> Vec<u64> {
    v.trim().trim_start_matches('v').split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

pub fn newer(candidate: &str, current: &str) -> bool {
    parse(candidate) > parse(current)
}

/// Najnowsze wydanie, jeśli jest nowsze od uruchomionej wersji. Blokuje — wołaj w wątku.
pub fn check() -> Result<Option<Release>, String> {
    // 404 = jeszcze nie opublikowano żadnego wydania — to nie błąd, po prostu brak nowszej wersji.
    let resp = match agent().get(FEED).set("Accept", "application/json").call() {
        Ok(r) => r,
        Err(ureq::Error::Status(404, _)) => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let r: serde_json::Value = serde_json::from_reader(resp.into_reader()).map_err(|e| e.to_string())?;
    let version = r["tag_name"].as_str().unwrap_or_default().trim_start_matches('v').to_string();
    if version.is_empty() || !newer(&version, VERSION) {
        return Ok(None);
    }
    let assets = r["assets"].as_array().cloned().unwrap_or_default();
    let find = |pred: &dyn Fn(&str) -> bool| {
        assets.iter().find(|a| a["name"].as_str().is_some_and(|n| pred(&n.to_ascii_lowercase()))).cloned()
    };
    let Some(exe) = find(&|n| n.starts_with("openfps") && n.ends_with(".exe")) else { return Ok(None) };
    Ok(Some(Release {
        version,
        asset_url: exe["browser_download_url"].as_str().unwrap_or_default().to_string(),
        asset_name: exe["name"].as_str().unwrap_or("OpenFPS.exe").to_string(),
        sums_url: find(&|n| n == "sha256sums.txt").and_then(|a| a["browser_download_url"].as_str().map(str::to_string)),
    }))
}

fn exe() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|e| e.to_string())
}

/// Usuwa pozostałość po poprzedniej aktualizacji (wywoływane na starcie).
pub fn cleanup() {
    if let Ok(p) = exe() {
        let _ = std::fs::remove_file(p.with_extension("old"));
        let _ = std::fs::remove_file(p.with_extension("new"));
    }
}

/// Pobiera wydanie, sprawdza SHA-256 i podmienia plik programu. `progress(pobrane, całość)`.
/// Po powodzeniu wystarczy uruchomić program ponownie.
pub fn install(rel: &Release, progress: impl Fn(u64, u64)) -> Result<(), String> {
    let sums_url = rel.sums_url.as_ref().ok_or("Wydanie nie ma sum kontrolnych / release has no checksums")?;
    let expected = get(sums_url, "application/octet-stream")?
        .into_string()
        .map_err(|e| e.to_string())?
        .lines()
        .find(|l| l.trim_end().ends_with(&rel.asset_name))
        .and_then(|l| l.split_whitespace().next())
        .map(|h| h.to_ascii_lowercase())
        .ok_or("Brak sumy dla pliku programu / missing checksum")?;

    let current = exe()?;
    let new_path = current.with_extension("new");
    let resp = get(&rel.asset_url, "application/octet-stream")?;
    let total: u64 = resp.header("Content-Length").and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut file = std::fs::File::create(&new_path).map_err(|e| format!("{}: {e}", new_path.display()))?;
    let mut reader = resp.into_reader();
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut done = 0u64;
    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        hasher.update(&buf[..n]);
        done += n as u64;
        progress(done, total.max(done));
    }
    drop(file);
    if format!("{:x}", hasher.finalize()) != expected {
        let _ = std::fs::remove_file(&new_path);
        return Err("Suma kontrolna pobranego pliku się nie zgadza — przerwano / checksum mismatch".into());
    }

    let old_path = current.with_extension("old");
    let _ = std::fs::remove_file(&old_path);
    std::fs::rename(&current, &old_path).map_err(|e| e.to_string())?;
    if let Err(e) = std::fs::rename(&new_path, &current) {
        // Przywróć poprzednią wersję, żeby program dalej się uruchamiał.
        let _ = std::fs::rename(&old_path, &current);
        return Err(e.to_string());
    }
    Ok(())
}
