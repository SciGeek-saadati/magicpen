use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt as _;

pub const SHIFT: u16 = 1;
pub const LOCK: u16 = 2;
pub const CTRL: u16 = 4;
pub const ALT: u16 = 8;
pub const NUM: u16 = 16;
pub const SUPER: u16 = 64;
pub const MOD_MASK: u16 = SHIFT | CTRL | ALT | SUPER;

pub const XK_ESCAPE: u32 = 0xff1b;
pub const XK_DELETE: u32 = 0xffff;
const XK_F1: u32 = 0xffbe;

const NAMED: &[(&str, u32)] = &[
    ("Space", 0x20),
    ("Escape", XK_ESCAPE),
    ("Tab", 0xff09),
    ("Enter", 0xff0d),
    ("Backspace", 0xff08),
    ("Delete", XK_DELETE),
    ("Insert", 0xff63),
    ("Home", 0xff50),
    ("End", 0xff57),
    ("PageUp", 0xff55),
    ("PageDown", 0xff56),
    ("Left", 0xff51),
    ("Up", 0xff52),
    ("Right", 0xff53),
    ("Down", 0xff54),
    ("Print", 0xff61),
    ("Pause", 0xff13),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Hotkey {
    pub mods: u16,
    pub sym: u32,
}

impl Hotkey {
    pub fn parse(s: &str) -> Option<Hotkey> {
        let mut mods = 0;
        let mut sym = None;
        for part in s.split('+').map(str::trim).filter(|p| !p.is_empty()) {
            match part.to_ascii_lowercase().as_str() {
                "shift" => mods |= SHIFT,
                "ctrl" | "control" => mods |= CTRL,
                "alt" => mods |= ALT,
                "super" | "win" | "meta" => mods |= SUPER,
                _ => sym = Some(sym_from_name(part)?),
            }
        }
        Some(Hotkey { mods, sym: sym? })
    }

    pub fn is_function_key(&self) -> bool {
        (XK_F1..XK_F1 + 12).contains(&self.sym)
    }
}

impl std::fmt::Display for Hotkey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (bit, name) in [(SUPER, "Super"), (CTRL, "Ctrl"), (ALT, "Alt"), (SHIFT, "Shift")] {
            if self.mods & bit != 0 {
                write!(f, "{name}+")?;
            }
        }
        if let Some((name, _)) = NAMED.iter().find(|(_, s)| *s == self.sym) {
            write!(f, "{name}")
        } else if self.is_function_key() {
            write!(f, "F{}", self.sym - XK_F1 + 1)
        } else if (0x21..=0x7e).contains(&self.sym) {
            write!(f, "{}", (self.sym as u8 as char).to_ascii_uppercase())
        } else {
            write!(f, "0x{:x}", self.sym)
        }
    }
}

fn sym_from_name(name: &str) -> Option<u32> {
    if let Some((_, sym)) = NAMED.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
        return Some(*sym);
    }
    let lower = name.to_ascii_lowercase();
    if let Some(n) = lower.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()) {
        return (1..=12).contains(&n).then(|| XK_F1 + n - 1);
    }
    if let Some(hex) = lower.strip_prefix("0x") {
        return u32::from_str_radix(hex, 16).ok();
    }
    let mut chars = lower.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if ('!'..='~').contains(&c) => Some(c as u32),
        _ => None,
    }
}

pub fn is_modifier(sym: u32) -> bool {
    sym == 0 || (0xffe1..=0xffee).contains(&sym) || (0xfe01..=0xfe13).contains(&sym) || sym == 0xff7e || sym == 0xff7f
}

pub struct Keymap {
    min: u8,
    per: usize,
    syms: Vec<u32>,
}

impl Keymap {
    pub fn load(conn: &impl Connection) -> Result<Keymap, Box<dyn std::error::Error>> {
        let setup = conn.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let reply = conn.get_keyboard_mapping(min, max - min + 1)?.reply()?;
        Ok(Keymap { min, per: reply.keysyms_per_keycode as usize, syms: reply.keysyms })
    }

    fn row(&self, code: u8) -> &[u32] {
        let start = (code.saturating_sub(self.min)) as usize * self.per;
        self.syms.get(start..start + self.per).unwrap_or(&[])
    }

    /// Layout-independent keysym for a keycode: prefers the Latin symbol from
    /// any layout group so shortcuts keep working while a non-Latin layout is active.
    pub fn sym(&self, code: u8) -> u32 {
        let row = self.row(code);
        if let Some(&s) = row.iter().find(|s| (0x20..=0x7e).contains(*s)) {
            return (s as u8 as char).to_ascii_lowercase() as u32;
        }
        row.iter().copied().find(|&s| s != 0).unwrap_or(0)
    }

    pub fn keycode(&self, sym: u32) -> Option<u8> {
        let codes = || (0..self.syms.len() / self.per.max(1)).map(|i| self.min + i as u8);
        codes().find(|&c| self.sym(c) == sym).or_else(|| codes().find(|&c| self.row(c).contains(&sym)))
    }
}
