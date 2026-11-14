//! Skróty klawiszowe w postaci tekstowej, np. `Ctrl+Shift+F9`.

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub vk: u32,
}

// (nazwa, kod VK)
const KEYS: &[(&str, u32)] = &[
    ("Backspace", 0x08), ("Tab", 0x09), ("Enter", 0x0D), ("Pause", 0x13), ("CapsLock", 0x14),
    ("Space", 0x20), ("PageUp", 0x21), ("PageDown", 0x22), ("End", 0x23), ("Home", 0x24),
    ("Left", 0x25), ("Up", 0x26), ("Right", 0x27), ("Down", 0x28), ("PrintScreen", 0x2C),
    ("Insert", 0x2D), ("Delete", 0x2E),
    ("Num0", 0x60), ("Num1", 0x61), ("Num2", 0x62), ("Num3", 0x63), ("Num4", 0x64),
    ("Num5", 0x65), ("Num6", 0x66), ("Num7", 0x67), ("Num8", 0x68), ("Num9", 0x69),
    ("NumMultiply", 0x6A), ("NumAdd", 0x6B), ("NumSubtract", 0x6D), ("NumDecimal", 0x6E), ("NumDivide", 0x6F),
    ("ScrollLock", 0x91),
    ("Semicolon", 0xBA), ("Equals", 0xBB), ("Comma", 0xBC), ("Minus", 0xBD), ("Period", 0xBE),
    ("Slash", 0xBF), ("Backtick", 0xC0), ("OpenBracket", 0xDB), ("Backslash", 0xDC), ("CloseBracket", 0xDD), ("Quote", 0xDE),
];

pub fn key_name(vk: u32) -> Option<String> {
    match vk {
        0x30..=0x39 | 0x41..=0x5A => Some(char::from_u32(vk).unwrap().to_string()),
        0x70..=0x87 => Some(format!("F{}", vk - 0x70 + 1)),
        _ => KEYS.iter().find(|(_, v)| *v == vk).map(|(n, _)| n.to_string()),
    }
}

pub fn key_from_name(name: &str) -> Option<u32> {
    let n = name.trim();
    if n.len() == 1 {
        let c = n.chars().next()?.to_ascii_uppercase();
        if c.is_ascii_alphanumeric() {
            return Some(c as u32);
        }
    }
    if let Some(num) = n.strip_prefix(['F', 'f']).and_then(|r| r.parse::<u32>().ok()) {
        if (1..=24).contains(&num) {
            return Some(0x70 + num - 1);
        }
    }
    KEYS.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| *v)
}

impl Hotkey {
    pub fn parse(s: &str) -> Option<Hotkey> {
        let mut hk = Hotkey::default();
        for part in s.split('+').map(str::trim).filter(|p| !p.is_empty()) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => hk.ctrl = true,
                "alt" => hk.alt = true,
                "shift" => hk.shift = true,
                "win" | "super" | "meta" => hk.win = true,
                _ => hk.vk = key_from_name(part)?,
            }
        }
        (hk.vk != 0).then_some(hk)
    }

    pub fn format(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.ctrl {
            parts.push("Ctrl".into());
        }
        if self.alt {
            parts.push("Alt".into());
        }
        if self.shift {
            parts.push("Shift".into());
        }
        if self.win {
            parts.push("Win".into());
        }
        parts.push(key_name(self.vk).unwrap_or_else(|| format!("VK{}", self.vk)));
        parts.join("+")
    }

    /// Flagi MOD_* dla RegisterHotKey (z MOD_NOREPEAT).
    pub fn modifiers(&self) -> u32 {
        let mut m = 0x4000; // MOD_NOREPEAT
        if self.alt {
            m |= 0x1;
        }
        if self.ctrl {
            m |= 0x2;
        }
        if self.shift {
            m |= 0x4;
        }
        if self.win {
            m |= 0x8;
        }
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip() {
        for s in ["Ctrl+Shift+F9", "Alt+O", "Ctrl+Alt+Num5", "Shift+Win+PageUp"] {
            assert_eq!(Hotkey::parse(s).unwrap().format(), s);
        }
        assert!(Hotkey::parse("Ctrl+Shift").is_none());
    }
}
