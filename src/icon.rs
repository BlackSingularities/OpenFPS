// Ikona aplikacji rysowana proceduralnie (bez plików zasobów):
// grafitowy zaokrąglony kwadrat z trzema rosnącymi słupkami w kolorze akcentu.

/// Zwraca piksele RGBA (alfa nieprzemnożona) o boku `size`.
pub fn rgba(size: u32) -> Vec<u8> {
    let s = size as f32;
    let mut out = vec![0u8; (size * size * 4) as usize];
    const SS: u32 = 4; // nadpróbkowanie 4×4

    // Prostokąty zaokrąglone w układzie 0..1: (x0, y0, x1, y1, promień)
    let bg = (0.04, 0.04, 0.96, 0.96, 0.24);
    let bars = [
        (0.22, 0.56, 0.36, 0.78, 0.07),
        (0.43, 0.40, 0.57, 0.78, 0.07),
        (0.64, 0.22, 0.78, 0.78, 0.07),
    ];
    let inside = |x: f32, y: f32, r: (f32, f32, f32, f32, f32)| -> bool {
        let (x0, y0, x1, y1, rad) = r;
        if x < x0 || x > x1 || y < y0 || y > y1 {
            return false;
        }
        let cx = x.clamp(x0 + rad, x1 - rad);
        let cy = y.clamp(y0 + rad, y1 - rad);
        (x - cx).powi(2) + (y - cy).powi(2) <= rad * rad
    };

    for py in 0..size {
        for px in 0..size {
            let (mut r, mut g, mut b, mut a) = (0f32, 0f32, 0f32, 0f32);
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = (px as f32 + (sx as f32 + 0.5) / SS as f32) / s;
                    let y = (py as f32 + (sy as f32 + 0.5) / SS as f32) / s;
                    if !inside(x, y, bg) {
                        continue;
                    }
                    // Tło: delikatny gradient pionowy.
                    let t = y;
                    let (mut cr, mut cg, mut cb) = (18.0 + 14.0 * t, 20.0 + 16.0 * t, 26.0 + 22.0 * t);
                    if bars.iter().any(|bar| inside(x, y, *bar)) {
                        // Akcent turkusowy z lekkim gradientem.
                        cr = 45.0 - 10.0 * t;
                        cg = 212.0 + 20.0 * (1.0 - t);
                        cb = 191.0;
                    }
                    r += cr;
                    g += cg;
                    b += cb;
                    a += 1.0;
                }
            }
            let n = (SS * SS) as f32;
            let i = ((py * size + px) * 4) as usize;
            if a > 0.0 {
                out[i] = (r / a).min(255.0) as u8;
                out[i + 1] = (g / a).min(255.0) as u8;
                out[i + 2] = (b / a).min(255.0) as u8;
                out[i + 3] = (a / n * 255.0) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    /// `cargo test export_icon -- --ignored` zapisuje surowe RGBA 1024×1024 do target/
    /// (z tego powstają ikony PNG do README, plakatu i strony).
    #[test]
    #[ignore]
    fn export_icon() {
        std::fs::write("target/icon-1024.rgba", super::rgba(1024)).unwrap();
    }
}
