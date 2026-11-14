// Transparent artwork shared by the executable, tray and settings.
// RGBA exports are generated from artwork/foreground.png.
pub fn rgba(size: u32) -> Vec<u8> {
    let pixels: &[u8] = match size {
        16 => include_bytes!("../assets/icon-16.rgba"),
        20 => include_bytes!("../assets/icon-20.rgba"),
        24 => include_bytes!("../assets/icon-24.rgba"),
        32 => include_bytes!("../assets/icon-32.rgba"),
        40 => include_bytes!("../assets/icon-40.rgba"),
        48 => include_bytes!("../assets/icon-48.rgba"),
        64 => include_bytes!("../assets/icon-64.rgba"),
        128 => include_bytes!("../assets/icon-128.rgba"),
        256 => include_bytes!("../assets/icon-256.rgba"),
        _ => {
            let source = include_bytes!("../assets/icon-256.rgba");
            let mut out = vec![0; (size as usize) * (size as usize) * 4];
            for y in 0..size {
                for x in 0..size {
                    let src = (((y as usize * 256 / size as usize) * 256) + x as usize * 256 / size as usize) * 4;
                    let dst = (y as usize * size as usize + x as usize) * 4;
                    out[dst..dst + 4].copy_from_slice(&source[src..src + 4]);
                }
            }
            return out;
        }
    };
    pixels.to_vec()
}
