use crate::canvas::Tool;
use std::path::PathBuf;

/// (label, min, max) for the size sliders, indexed like `Config::sizes`.
pub const SLIDERS: [(&str, f32, f32); 4] = [
    ("Pen size", 1.0, 40.0),
    ("Highlighter size", 6.0, 60.0),
    ("Highlighter opacity", 10.0, 90.0),
    ("Eraser size", 8.0, 120.0),
];
pub const HOTKEY_LABELS: [&str; 3] = ["Toggle drawing", "Show / hide", "Clear screen"];

const SIZE_KEYS: [&str; 4] = ["pen_size", "highlighter_size", "highlighter_opacity", "eraser_size"];
const HOTKEY_KEYS: [&str; 3] = ["hotkey_toggle", "hotkey_hide", "hotkey_clear"];

pub struct Config {
    pub tool: Tool,
    pub color: usize,
    pub sizes: [f32; 4],
    pub hotkeys: [String; 3],
    pub toolbar: Option<(i32, i32)>,
    /// UI scale factor; 0 means derive it from the display DPI.
    pub scale: f32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            tool: Tool::Pen,
            color: 0,
            sizes: [4.0, 22.0, 40.0, 30.0],
            hotkeys: ["Super+Shift+D".into(), "Super+Shift+H".into(), "Super+Shift+C".into()],
            toolbar: None,
            scale: 0.0,
        }
    }
}

fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("magicpen").join("config"))
}

impl Config {
    pub fn load() -> Config {
        let mut cfg = Config::default();
        let Some(text) = path().and_then(|p| std::fs::read_to_string(p).ok()) else {
            return cfg;
        };
        let (mut tx, mut ty) = (None, None);
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            let (key, value) = (key.trim(), value.trim());
            let num = value.parse::<f32>().ok();
            if let Some(i) = SIZE_KEYS.iter().position(|k| *k == key) {
                if let Some(v) = num {
                    cfg.sizes[i] = v.clamp(SLIDERS[i].1, SLIDERS[i].2);
                }
            } else if let Some(i) = HOTKEY_KEYS.iter().position(|k| *k == key) {
                cfg.hotkeys[i] = value.to_string();
            } else {
                match key {
                    "tool" => cfg.tool = Tool::from_name(value).unwrap_or(cfg.tool),
                    "color" => cfg.color = value.parse().unwrap_or(0),
                    "toolbar_x" => tx = value.parse().ok(),
                    "toolbar_y" => ty = value.parse().ok(),
                    "scale" => cfg.scale = num.unwrap_or(0.0).clamp(0.0, 4.0),
                    _ => {}
                }
            }
        }
        cfg.toolbar = tx.zip(ty);
        cfg
    }

    pub fn save(&self) {
        let Some(path) = path() else { return };
        let mut out = String::from("# magicpen settings (edited by the app; safe to change by hand)\n");
        out += &format!("tool = {}\ncolor = {}\n", self.tool.name(), self.color);
        for (key, v) in SIZE_KEYS.iter().zip(self.sizes) {
            out += &format!("{key} = {v}\n");
        }
        for (key, v) in HOTKEY_KEYS.iter().zip(&self.hotkeys) {
            out += &format!("{key} = {v}\n");
        }
        if let Some((x, y)) = self.toolbar {
            out += &format!("toolbar_x = {x}\ntoolbar_y = {y}\n");
        }
        out += &format!("scale = {}\n", self.scale);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&path, out) {
            eprintln!("magicpen: cannot save {}: {e}", path.display());
        }
    }
}
