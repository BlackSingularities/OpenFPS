//! Ikona i opis pliku .exe (Eksplorator, „Aplikacje i funkcje”, właściwości pliku).
//! Ikona powstaje z tego samego kodu co ikona w zasobniku (`src/icon.rs`), więc nie ma
//! osobnych plików graficznych do pilnowania.

mod icon {
    include!("src/icon.rs");
}

/// Plik ICO z bitmapami 32 bpp (BGRA z kanałem alfa) w kilku rozmiarach.
fn ico(sizes: &[u32]) -> Vec<u8> {
    let mut images: Vec<Vec<u8>> = Vec::new();
    for &s in sizes {
        let px = icon::rgba(s);
        let mut b = Vec::new();
        // BITMAPINFOHEADER — wysokość podwojona (bitmapa kolorów + maska AND).
        b.extend_from_slice(&40u32.to_le_bytes());
        b.extend_from_slice(&(s as i32).to_le_bytes());
        b.extend_from_slice(&((s * 2) as i32).to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&32u16.to_le_bytes());
        b.extend_from_slice(&[0u8; 24]);
        for y in (0..s).rev() {
            for x in 0..s {
                let i = ((y * s + x) * 4) as usize;
                b.extend_from_slice(&[px[i + 2], px[i + 1], px[i], px[i + 3]]);
            }
        }
        let mask_row = ((s + 31) / 32) * 4;
        b.extend(std::iter::repeat(0u8).take((mask_row * s) as usize));
        images.push(b);
    }
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (s, img) in sizes.iter().zip(&images) {
        let dim = if *s >= 256 { 0 } else { *s as u8 };
        out.extend_from_slice(&[dim, dim, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(img.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for img in images {
        out.extend_from_slice(&img);
    }
    out
}

fn main() {
    println!("cargo:rerun-if-changed=src/icon.rs");
    println!("cargo:rerun-if-env-changed=OPENFPS_VERSION");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("openfps.ico");
    std::fs::write(&out, ico(&[16, 20, 24, 32, 40, 48, 64, 128])).expect("ico");
    let ver = std::env::var("OPENFPS_VERSION").unwrap_or_else(|_| std::env::var("CARGO_PKG_VERSION").unwrap());
    let mut res = winresource::WindowsResource::new();
    res.set_icon(out.to_str().unwrap())
        .set("FileDescription", "OpenFPS")
        .set("ProductName", "OpenFPS")
        .set("CompanyName", "Adam Rędzikowski")
        .set("LegalCopyright", "Adam Rędzikowski · redzikowski.com")
        .set("ProductVersion", &ver)
        .set("FileVersion", &ver);
    if let Err(e) = res.compile() {
        println!("cargo:warning=ikona .exe pominięta: {e}");
    }
}
