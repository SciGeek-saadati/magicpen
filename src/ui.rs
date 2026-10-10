use crate::canvas::{Tool, R};
use crate::config::{Config, HOTKEY_LABELS, SLIDERS};
use fontdue::{Font, FontSettings};
use std::cell::OnceCell;
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, Rect, Stroke, Transform};

pub const PALETTE: [[u8; 3]; 8] = [
    [255, 59, 48],
    [255, 149, 0],
    [255, 214, 10],
    [52, 199, 89],
    [10, 132, 255],
    [191, 90, 242],
    [255, 255, 255],
    [28, 28, 30],
];

const ACCENT: [u8; 4] = [10, 132, 255, 255];
const FG: [u8; 4] = [240, 240, 245, 255];
const FG_DIM: [u8; 4] = [160, 160, 170, 255];
const WARN: [u8; 4] = [255, 105, 97, 255];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Item {
    Grip,
    Cursor,
    Tool(Tool),
    Color(usize),
    SizeDown,
    SizeUp,
    Undo,
    Redo,
    Clear,
    /// Collapse the overlay to a single eye button.
    Hide,
    /// The collapsed eye button itself; never part of the toolbar.
    Show,
    Settings,
    Quit,
}

pub enum PanelHit {
    Slider(usize),
    Hotkey(usize),
}

/// Everything the widgets need to know about application state to draw themselves.
pub struct View<'a> {
    pub cfg: &'a Config,
    pub draw_mode: bool,
    pub hover: Option<Item>,
    pub panel_open: bool,
    pub recording: Option<usize>,
    pub hotkey_ok: [bool; 3],
}

/// Layout and rendering of the floating toolbar and the settings panel.
/// All rectangles are relative to the widget's own top-left corner; the
/// rendered pixmaps carry an extra `margin` on every side for the shadow.
pub struct Ui {
    s: f32,
    items: Vec<(Item, R)>,
    preview: R,
    pub tb: (i32, i32),
    pub panel: (i32, i32),
    pub margin: i32,
    font: OnceCell<Option<Font>>,
}

// Settings panel metrics, in unscaled units.
const PANEL_W: f32 = 340.0;
const ROW_H: f32 = 34.0;
const ROWS_TOP: f32 = 44.0;
const CTRL_X: f32 = 160.0;

impl Ui {
    pub fn new(s: f32) -> Ui {
        let px = |v: f32| (v * s).round() as i32;
        enum Spec {
            It(Item, f32),
            Sep,
            Preview,
        }
        let mut spec = vec![Spec::It(Item::Grip, 18.0), Spec::It(Item::Cursor, 34.0), Spec::Sep];
        spec.extend(Tool::ALL.map(|t| Spec::It(Item::Tool(t), 34.0)));
        spec.push(Spec::Sep);
        spec.extend((0..PALETTE.len()).map(|i| Spec::It(Item::Color(i), 26.0)));
        spec.extend([Spec::Sep, Spec::It(Item::SizeDown, 26.0), Spec::Preview, Spec::It(Item::SizeUp, 26.0)]);
        spec.push(Spec::Sep);
        spec.extend([Item::Undo, Item::Redo, Item::Clear].map(|i| Spec::It(i, 34.0)));
        spec.push(Spec::Sep);
        spec.extend([Item::Hide, Item::Settings, Item::Quit].map(|i| Spec::It(i, 34.0)));

        let (pad, btn) = (px(6.0), px(34.0));
        let mut x = pad;
        let mut items = Vec::new();
        let mut preview = R::default();
        for sp in spec {
            match sp {
                Spec::It(item, w) => {
                    items.push((item, R::new(x, pad, px(w), btn)));
                    x += px(w);
                }
                Spec::Sep => x += px(9.0),
                Spec::Preview => {
                    preview = R::new(x, pad, px(30.0), btn);
                    x += px(30.0);
                }
            }
        }
        let panel_h = ROWS_TOP + ROW_H * 7.0 + 8.0 + 52.0;
        Ui {
            s,
            items,
            preview,
            tb: (x + pad, btn + 2 * pad),
            panel: (px(PANEL_W), px(panel_h)),
            margin: px(12.0),
            font: OnceCell::new(),
        }
    }

    pub fn item_at(&self, x: i32, y: i32) -> Option<Item> {
        self.items.iter().find(|(_, r)| r.contains(x, y)).map(|(i, _)| *i)
    }

    pub fn item_rect(&self, item: Item) -> R {
        self.items.iter().find(|(i, _)| *i == item).map(|(_, r)| *r).unwrap_or_default()
    }

    /// The square button left on screen while the overlay is collapsed; `tb.1` pixels on a side.
    pub fn draw_eye(&self, hover: bool) -> Pixmap {
        let (s, m, side) = (self.s, self.margin, self.tb.1);
        let mut pm = Pixmap::new((side + 2 * m) as u32, (side + 2 * m) as u32).expect("eye size");
        backdrop(&mut pm, m as f32, side as f32, side as f32, 12.0 * s);
        let btn = (34.0 * s).round() as i32;
        let r = R::new(m + (side - btn) / 2, m + (side - btn) / 2, btn, btn);
        if hover {
            let inset = 2.0 * s;
            let path = rrect(
                r.x as f32 + inset,
                r.y as f32 + inset,
                r.w as f32 - 2.0 * inset,
                r.h as f32 - 2.0 * inset,
                7.0 * s,
            );
            fill(&mut pm, path.as_ref(), [255, 255, 255, 34]);
        }
        icon(&mut pm, Item::Show, r, s);
        pm
    }

    fn row_y(&self, row: usize) -> f32 {
        // Hotkey rows sit below the sliders with a small gap.
        let gap = if row >= SLIDERS.len() { 8.0 } else { 0.0 };
        (ROWS_TOP + ROW_H * row as f32 + gap) * self.s
    }

    /// Slider track as (x0, x1) in panel pixels.
    fn track(&self) -> (f32, f32) {
        (CTRL_X * self.s, (PANEL_W - 56.0) * self.s)
    }

    fn hotkey_button(&self, i: usize) -> R {
        let y = self.row_y(SLIDERS.len() + i) + 4.0 * self.s;
        let (x0, x1) = (CTRL_X * self.s, (PANEL_W - 14.0) * self.s);
        R::new(x0 as i32, y as i32, (x1 - x0) as i32, (26.0 * self.s) as i32)
    }

    pub fn panel_hit(&self, x: i32, y: i32) -> Option<PanelHit> {
        let (x0, x1) = self.track();
        let slack = 10.0 * self.s;
        for i in 0..SLIDERS.len() {
            let top = self.row_y(i);
            let in_row = (y as f32) >= top && (y as f32) < top + ROW_H * self.s;
            if in_row && (x as f32) >= x0 - slack && (x as f32) <= x1 + slack {
                return Some(PanelHit::Slider(i));
            }
        }
        (0..HOTKEY_LABELS.len()).find(|&i| self.hotkey_button(i).contains(x, y)).map(PanelHit::Hotkey)
    }

    /// Position along the slider track (0..=1) for a panel-relative x.
    pub fn slider_frac(&self, x: i32) -> f32 {
        let (x0, x1) = self.track();
        ((x as f32 - x0) / (x1 - x0)).clamp(0.0, 1.0)
    }

    pub fn draw_toolbar(&self, v: &View) -> Pixmap {
        let (s, m) = (self.s, self.margin);
        let (w, h) = self.tb;
        let mut pm = Pixmap::new((w + 2 * m) as u32, (h + 2 * m) as u32).expect("toolbar size");
        backdrop(&mut pm, m as f32, w as f32, h as f32, 12.0 * s);

        for &(item, r) in &self.items {
            let r = r.offset(m, m);
            let active = match item {
                Item::Cursor => !v.draw_mode,
                Item::Tool(t) => v.draw_mode && v.cfg.tool == t,
                Item::Settings => v.panel_open,
                _ => false,
            };
            let bg = if active {
                Some(ACCENT)
            } else if v.hover == Some(item) && !matches!(item, Item::Color(_)) {
                Some([255, 255, 255, 34])
            } else {
                None
            };
            if let Some(bg) = bg {
                let inset = 2.0 * s;
                let path = rrect(
                    r.x as f32 + inset,
                    r.y as f32 + inset,
                    r.w as f32 - 2.0 * inset,
                    r.h as f32 - 2.0 * inset,
                    7.0 * s,
                );
                fill(&mut pm, path.as_ref(), bg);
            }
            match item {
                Item::Color(i) => {
                    let (cx, cy) = (r.x as f32 + r.w as f32 / 2.0, r.y as f32 + r.h as f32 / 2.0);
                    let [cr, cg, cb] = PALETTE[i];
                    let grow = if v.hover == Some(item) { 1.0 } else { 0.0 };
                    let dot = PathBuilder::from_circle(cx, cy, (8.0 + grow) * s);
                    fill(&mut pm, dot.as_ref(), [cr, cg, cb, 255]);
                    stroke(&mut pm, dot.as_ref(), [255, 255, 255, 70], s, Transform::identity());
                    if v.cfg.color == i {
                        let ring = PathBuilder::from_circle(cx, cy, 11.5 * s);
                        stroke(&mut pm, ring.as_ref(), FG, 1.8 * s, Transform::identity());
                    }
                }
                _ => icon(&mut pm, item, r, s),
            }
        }

        // Dot showing the size and colour the current tool will draw with.
        let pr = self.preview.offset(m, m);
        let (cx, cy) = (pr.x as f32 + pr.w as f32 / 2.0, pr.y as f32 + pr.h as f32 / 2.0);
        let tool = v.cfg.tool;
        let radius = (v.cfg.sizes[tool.size_slot()] / 2.0).clamp(1.5, 12.0) * s;
        let dot = PathBuilder::from_circle(cx, cy, radius);
        if tool == Tool::Eraser {
            stroke(&mut pm, dot.as_ref(), FG_DIM, 1.5 * s, Transform::identity());
        } else {
            let [cr, cg, cb] = PALETTE[v.cfg.color];
            fill(&mut pm, dot.as_ref(), [cr, cg, cb, 255]);
            stroke(&mut pm, dot.as_ref(), [255, 255, 255, 70], s, Transform::identity());
        }
        pm
    }

    pub fn draw_panel(&self, v: &View) -> Pixmap {
        let (s, m) = (self.s, self.margin);
        let (w, h) = self.panel;
        let mut pm = Pixmap::new((w + 2 * m) as u32, (h + 2 * m) as u32).expect("panel size");
        backdrop(&mut pm, m as f32, w as f32, h as f32, 12.0 * s);
        let mf = m as f32;
        let left = mf + 16.0 * s;

        self.text(&mut pm, left, mf + 28.0 * s, 15.0, FG, "Settings");

        let (x0, x1) = self.track();
        for (i, &(label, min, max)) in SLIDERS.iter().enumerate() {
            let top = mf + self.row_y(i);
            let cy = top + ROW_H * s / 2.0;
            self.text(&mut pm, left, cy + 4.5 * s, 13.0, FG_DIM, label);
            let value = v.cfg.sizes[i];
            let knob = mf + x0 + (x1 - x0) * ((value - min) / (max - min)).clamp(0.0, 1.0);
            let bar = |from: f32, to: f32| rrect(from, cy - 2.0 * s, (to - from).max(0.1), 4.0 * s, 2.0 * s);
            fill(&mut pm, bar(mf + x0, mf + x1).as_ref(), [255, 255, 255, 46]);
            fill(&mut pm, bar(mf + x0, knob).as_ref(), ACCENT);
            fill(&mut pm, PathBuilder::from_circle(knob, cy, 7.0 * s).as_ref(), FG);
            let shown = if i == 2 { format!("{value:.0}%") } else { format!("{value:.0}") };
            let tw = self.text_width(13.0, &shown);
            self.text(&mut pm, mf + (PANEL_W - 14.0) * s - tw, cy + 4.5 * s, 13.0, FG, &shown);
        }

        for (i, label) in HOTKEY_LABELS.iter().enumerate() {
            let btn = self.hotkey_button(i).offset(m, m);
            let cy = btn.y as f32 + btn.h as f32 / 2.0;
            self.text(&mut pm, left, cy + 4.5 * s, 13.0, FG_DIM, label);
            let recording = v.recording == Some(i);
            let bg = if recording { [10, 132, 255, 110] } else { [255, 255, 255, 30] };
            fill(&mut pm, rrect(btn.x as f32, btn.y as f32, btn.w as f32, btn.h as f32, 6.0 * s).as_ref(), bg);
            let (shown, color) = if recording {
                ("Press keys... (Esc cancels)".to_string(), FG)
            } else if v.hotkey_ok[i] {
                (v.cfg.hotkeys[i].clone(), FG)
            } else {
                (format!("{} (unavailable)", v.cfg.hotkeys[i]), WARN)
            };
            let tw = self.text_width(12.5, &shown);
            self.text(&mut pm, btn.x as f32 + (btn.w as f32 - tw) / 2.0, cy + 4.5 * s, 12.5, color, &shown);
        }

        let foot = mf + self.row_y(SLIDERS.len() + HOTKEY_LABELS.len()) + 20.0 * s;
        self.text(&mut pm, left, foot, 11.0, FG_DIM, "Esc: stop drawing    Ctrl+Z / Ctrl+Y: undo / redo");
        self.text(&mut pm, left, foot + 17.0 * s, 11.0, FG_DIM, "Shift: straight lines, squares    [ ] or wheel: size");
        pm
    }

    fn font(&self) -> Option<&Font> {
        self.font.get_or_init(load_font).as_ref()
    }

    fn text_width(&self, size: f32, text: &str) -> f32 {
        let Some(font) = self.font() else { return 0.0 };
        text.chars().map(|c| font.metrics(c, size * self.s).advance_width).sum()
    }

    /// Draw `text` with its baseline at `base`; `size` is in unscaled units.
    fn text(&self, pm: &mut Pixmap, x: f32, base: f32, size: f32, c: [u8; 4], text: &str) {
        let Some(font) = self.font() else { return };
        let (pw, ph) = (pm.width() as i32, pm.height() as i32);
        let data = pm.data_mut();
        let mut pen = x;
        for ch in text.chars() {
            let (mt, cov) = font.rasterize(ch, size * self.s);
            let gx = pen.round() as i32 + mt.xmin;
            let gy = base.round() as i32 - mt.ymin - mt.height as i32;
            for row in 0..mt.height {
                for col in 0..mt.width {
                    let (px, py) = (gx + col as i32, gy + row as i32);
                    let a = cov[row * mt.width + col] as u32 * c[3] as u32 / 255;
                    if a == 0 || px < 0 || py < 0 || px >= pw || py >= ph {
                        continue;
                    }
                    // Source-over onto premultiplied RGBA.
                    let i = ((py * pw + px) * 4) as usize;
                    for k in 0..3 {
                        data[i + k] = ((c[k] as u32 * a + data[i + k] as u32 * (255 - a)) / 255) as u8;
                    }
                    data[i + 3] = (a + data[i + 3] as u32 * (255 - a) / 255) as u8;
                }
            }
            pen += mt.advance_width;
        }
    }
}

fn load_font() -> Option<Font> {
    let out = std::process::Command::new("fc-match").args(["-f", "%{file}", "sans-serif"]).output().ok()?;
    let path = String::from_utf8(out.stdout).ok()?;
    let bytes = std::fs::read(path.trim()).ok()?;
    Font::from_bytes(bytes, FontSettings::default()).ok()
}

fn rrect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<Path> {
    let r = r.min(w / 2.0).min(h / 2.0);
    let k = r * 0.5523;
    let (x1, y1) = (x + w, y + h);
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x1 - r, y);
    pb.cubic_to(x1 - r + k, y, x1, y + r - k, x1, y + r);
    pb.line_to(x1, y1 - r);
    pb.cubic_to(x1, y1 - r + k, x1 - r + k, y1, x1 - r, y1);
    pb.line_to(x + r, y1);
    pb.cubic_to(x + r - k, y1, x, y1 - r + k, x, y1 - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish()
}

fn paint(c: [u8; 4]) -> Paint<'static> {
    let mut p = Paint { anti_alias: true, ..Paint::default() };
    p.set_color_rgba8(c[0], c[1], c[2], c[3]);
    p
}

fn fill(pm: &mut Pixmap, path: Option<&Path>, c: [u8; 4]) {
    if let Some(path) = path {
        pm.fill_path(path, &paint(c), FillRule::Winding, Transform::identity(), None);
    }
}

fn stroke(pm: &mut Pixmap, path: Option<&Path>, c: [u8; 4], width: f32, ts: Transform) {
    let st = Stroke { width, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Stroke::default() };
    if let Some(path) = path {
        pm.stroke_path(path, &paint(c), &st, ts, None);
    }
}

/// Dark rounded card with a soft drop shadow, inset by `m` inside the pixmap.
fn backdrop(pm: &mut Pixmap, m: f32, w: f32, h: f32, radius: f32) {
    let layers = 5;
    for i in (1..=layers).rev() {
        let g = m * 0.8 * i as f32 / layers as f32;
        let shadow = rrect(m - g, m - g + m * 0.18, w + 2.0 * g, h + 2.0 * g, radius + g);
        fill(pm, shadow.as_ref(), [0, 0, 0, 13]);
    }
    fill(pm, rrect(m, m, w, h, radius).as_ref(), [30, 30, 34, 242]);
    let edge = rrect(m + 0.5, m + 0.5, w - 1.0, h - 1.0, radius);
    stroke(pm, edge.as_ref(), [255, 255, 255, 36], 1.0, Transform::identity());
}

fn poly(pb: &mut PathBuilder, pts: &[(f32, f32)], close: bool) {
    pb.move_to(pts[0].0, pts[0].1);
    for p in &pts[1..] {
        pb.line_to(p.0, p.1);
    }
    if close {
        pb.close();
    }
}

/// Icons are authored on a 34-unit-tall grid (26 wide for the narrow buttons).
fn icon(pm: &mut Pixmap, item: Item, r: R, s: f32) {
    let mut st = PathBuilder::new();
    let mut fl = PathBuilder::new();
    let mut color = FG;
    match item {
        Item::Grip => {
            color = [255, 255, 255, 90];
            for cx in [6.0, 12.0] {
                for cy in [11.0, 17.0, 23.0] {
                    fl.push_circle(cx, cy, 1.5);
                }
            }
        }
        Item::Cursor => poly(
            &mut fl,
            &[(10.0, 6.5), (10.0, 25.0), (14.8, 20.6), (18.0, 27.6), (21.0, 26.2), (17.9, 19.4), (24.3, 19.4)],
            true,
        ),
        Item::Tool(Tool::Pen) => {
            poly(&mut st, &[(8.0, 26.0), (9.5, 19.5), (21.5, 7.5), (26.5, 12.5), (14.5, 24.5)], true);
            poly(&mut st, &[(9.5, 19.5), (14.5, 24.5)], false);
        }
        Item::Tool(Tool::Highlighter) => {
            poly(&mut st, &[(12.5, 17.5), (21.0, 9.0), (25.0, 13.0), (16.5, 21.5)], true);
            poly(&mut fl, &[(12.5, 17.5), (16.5, 21.5), (11.5, 24.5), (9.5, 22.5)], true);
            poly(&mut st, &[(8.0, 28.0), (26.0, 28.0)], false);
        }
        Item::Tool(Tool::Line) => poly(&mut st, &[(9.0, 25.0), (25.0, 9.0)], false),
        Item::Tool(Tool::Arrow) => {
            poly(&mut st, &[(9.0, 25.0), (25.0, 9.0)], false);
            poly(&mut st, &[(16.0, 9.0), (25.0, 9.0), (25.0, 18.0)], false);
        }
        Item::Tool(Tool::Rect) => {
            if let Some(rect) = Rect::from_ltrb(8.0, 10.0, 26.0, 24.0) {
                st.push_rect(rect);
            }
        }
        Item::Tool(Tool::Ellipse) => {
            if let Some(rect) = Rect::from_ltrb(7.5, 9.5, 26.5, 24.5) {
                st.push_oval(rect);
            }
        }
        Item::Tool(Tool::Eraser) => {
            poly(&mut st, &[(19.0, 7.0), (26.0, 14.0), (15.0, 25.0), (8.0, 18.0)], true);
            poly(&mut st, &[(13.0, 13.0), (20.0, 20.0)], false);
            poly(&mut st, &[(14.0, 28.5), (26.0, 28.5)], false);
        }
        Item::Undo | Item::Redo => {
            // Authored as "undo"; redo mirrors it horizontally.
            let fx = |x: f32| if item == Item::Redo { 34.0 - x } else { x };
            poly(&mut st, &[(fx(13.0), 10.0), (fx(8.0), 15.0), (fx(13.0), 20.0)], false);
            st.move_to(fx(8.0), 15.0);
            st.line_to(fx(19.0), 15.0);
            st.cubic_to(fx(27.0), 15.0, fx(27.0), 26.0, fx(19.0), 26.0);
            st.line_to(fx(14.0), 26.0);
        }
        Item::Clear => {
            poly(&mut st, &[(9.0, 11.5), (25.0, 11.5)], false);
            poly(&mut st, &[(14.0, 11.5), (14.0, 8.0), (20.0, 8.0), (20.0, 11.5)], false);
            poly(&mut st, &[(11.0, 11.5), (12.0, 27.0), (22.0, 27.0), (23.0, 11.5)], false);
            poly(&mut st, &[(15.2, 15.5), (15.2, 23.0)], false);
            poly(&mut st, &[(18.8, 15.5), (18.8, 23.0)], false);
        }
        Item::SizeDown => poly(&mut st, &[(8.0, 17.0), (18.0, 17.0)], false),
        Item::SizeUp => {
            poly(&mut st, &[(8.0, 17.0), (18.0, 17.0)], false);
            poly(&mut st, &[(13.0, 12.0), (13.0, 22.0)], false);
        }
        Item::Settings => {
            for (y, knob) in [(11.0, 20.5), (17.0, 13.0), (23.0, 19.0)] {
                poly(&mut st, &[(8.0, y), (26.0, y)], false);
                fl.push_circle(knob, y, 2.8);
            }
        }
        Item::Hide => {
            st.move_to(6.0, 17.0);
            st.quad_to(17.0, 5.0, 28.0, 17.0);
            st.quad_to(17.0, 29.0, 6.0, 17.0);
            fl.push_circle(17.0, 17.0, 3.4);
        }
        Item::Show => {
            // A closed eye: the lower lid with lashes.
            st.move_to(6.0, 14.5);
            st.quad_to(17.0, 26.0, 28.0, 14.5);
            poly(&mut st, &[(10.0, 19.0), (8.0, 22.5)], false);
            poly(&mut st, &[(17.0, 20.5), (17.0, 24.5)], false);
            poly(&mut st, &[(24.0, 19.0), (26.0, 22.5)], false);
        }
        Item::Quit => {
            poly(&mut st, &[(10.5, 10.5), (23.5, 23.5)], false);
            poly(&mut st, &[(23.5, 10.5), (10.5, 23.5)], false);
        }
        Item::Color(_) => {}
    }
    let ts = Transform::from_scale(s, s).post_translate(r.x as f32, r.y as f32);
    if let Some(path) = fl.finish() {
        pm.fill_path(&path, &paint(color), FillRule::Winding, ts, None);
    }
    stroke(pm, st.finish().as_ref(), color, 1.7, ts);
}
