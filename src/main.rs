mod canvas;
mod config;
mod keys;
mod ui;

use canvas::{Canvas, Shape, Tool, R};
use config::{Config, SLIDERS};
use keys::{Hotkey, Keymap};
use tiny_skia::{LineCap, Paint, PathBuilder, Pixmap, PixmapMut, PixmapPaint, Stroke, Transform};
use ui::{Item, PanelHit, Ui, View, PALETTE};
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::shape::{self, ConnectionExt as _};
use x11rb::protocol::xproto::*;
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

type Res<T> = Result<T, Box<dyn std::error::Error>>;

const USAGE: &str = "\
magicpen - draw on top of everything on your screen

Usage: magicpen [COMMAND]

With no command, starts the overlay (or toggles drawing if already running).
Commands are sent to the running instance:
  toggle    switch between drawing and normal mouse mode
  show      show or hide the overlay
  clear     erase all annotations
  undo      undo the last annotation
  quit      exit the running instance";

const CMD_TOGGLE: u32 = 1;
const CMD_SHOW: u32 = 2;
const CMD_CLEAR: u32 = 3;
const CMD_UNDO: u32 = 4;
const CMD_QUIT: u32 = 5;

// Glyph indices in the standard X cursor font.
const CURSOR_GLYPHS: [u16; 3] = [68, 86, 34]; // left_ptr, pencil, crosshair

#[derive(Default)]
enum Drag {
    #[default]
    None,
    Toolbar {
        dx: i32,
        dy: i32,
    },
    Slider(usize),
    Shape,
}

struct App {
    conn: RustConnection,
    root: Window,
    win: Window,
    gc: Gcontext,
    sw: i32,
    sh: i32,
    lsb_first: bool,
    max_req: usize,
    cmd_atom: Atom,

    canvas: Canvas,
    /// Shape being drawn right now; composited on top of the canvas until committed.
    cur: Option<Shape>,
    cfg: Config,
    cfg_dirty: bool,
    draw_mode: bool,
    visible: bool,
    quit: bool,

    ui: Ui,
    tb_pos: (i32, i32),
    tb_pix: Pixmap,
    panel_pix: Option<Pixmap>,
    /// Everything is hidden except a small eye button that brings it back.
    collapsed: bool,
    eye_pix: Option<Pixmap>,
    panel_open: bool,
    ui_dirty: bool,
    hover: Option<Item>,
    recording: Option<usize>,
    drag: Drag,
    ptr: Option<(i32, i32)>,
    /// Screen area of the eraser outline currently shown under the pointer.
    ring: Option<R>,

    keymap: Keymap,
    hotkeys: [Option<Hotkey>; 3],
    hotkey_ok: [bool; 3],
    kbd_grabbed: bool,
    cursors: [Cursor; 3],
    cursor: usize,

    damage: Vec<R>,
    buf: Vec<u8>,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("magicpen: {e}");
        std::process::exit(1);
    }
}

fn run() -> Res<()> {
    let cmd = match std::env::args().nth(1).as_deref() {
        None => None,
        Some("toggle") => Some(CMD_TOGGLE),
        Some("show") | Some("hide") => Some(CMD_SHOW),
        Some("clear") => Some(CMD_CLEAR),
        Some("undo") => Some(CMD_UNDO),
        Some("quit") => Some(CMD_QUIT),
        Some("-h") | Some("--help") | Some("help") => {
            println!("{USAGE}");
            return Ok(());
        }
        Some(other) => return Err(format!("unknown command '{other}'\n\n{USAGE}").into()),
    };

    let (conn, screen_num) = x11rb::connect(None)?;
    let instance_atom = conn.intern_atom(false, b"_MAGICPEN_INSTANCE")?.reply()?.atom;
    let cmd_atom = conn.intern_atom(false, b"_MAGICPEN_COMMAND")?.reply()?.atom;

    // A running instance owns the selection; forward the command to it and exit.
    let owner = conn.get_selection_owner(instance_atom)?.reply()?.owner;
    if owner != x11rb::NONE {
        let data = [cmd.unwrap_or(CMD_TOGGLE), 0, 0, 0, 0];
        let msg = ClientMessageEvent::new(32, owner, cmd_atom, data);
        // Wait for the server to acknowledge: exiting right after a bare flush can drop the request.
        conn.send_event(false, owner, EventMask::NO_EVENT, msg)?.check()?;
        return Ok(());
    }
    if cmd == Some(CMD_QUIT) {
        return Ok(());
    }

    let mut app = App::new(conn, screen_num, cmd_atom)?;
    app.conn.set_selection_owner(app.win, instance_atom, x11rb::CURRENT_TIME)?;
    app.sync_input_shape();
    app.conn.map_window(app.win)?;
    app.flush()?;

    while !app.quit {
        let ev = app.conn.wait_for_event()?;
        app.handle(ev)?;
        // Drain the queue before repainting so fast pointer motion costs one frame.
        while let Some(ev) = app.conn.poll_for_event()? {
            app.handle(ev)?;
        }
        app.flush()?;
    }
    if app.cfg_dirty {
        app.cfg.save();
    }
    Ok(())
}

fn detect_scale(conn: &RustConnection, root: Window) -> f32 {
    let value = conn
        .get_property(false, root, AtomEnum::RESOURCE_MANAGER, AtomEnum::STRING, 0, 1 << 16)
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|r| r.value)
        .unwrap_or_default();
    let dpi = String::from_utf8_lossy(&value)
        .lines()
        .find_map(|l| l.strip_prefix("Xft.dpi:").and_then(|v| v.trim().parse::<f32>().ok()))
        .unwrap_or(96.0);
    (dpi / 96.0).clamp(1.0, 3.0)
}

/// Snap the second point of a shape when Shift is held: 45-degree steps for
/// lines and arrows, equal sides for rectangles and ellipses.
fn constrain(tool: Tool, a: (f32, f32), p: (f32, f32), shift: bool) -> (f32, f32) {
    if !shift {
        return p;
    }
    let (dx, dy) = (p.0 - a.0, p.1 - a.1);
    match tool {
        Tool::Line | Tool::Arrow => {
            let step = std::f32::consts::FRAC_PI_4;
            let angle = (dy.atan2(dx) / step).round() * step;
            let len = dx.hypot(dy);
            (a.0 + len * angle.cos(), a.1 + len * angle.sin())
        }
        _ => {
            let d = dx.abs().max(dy.abs());
            (a.0 + d * dx.signum(), a.1 + d * dy.signum())
        }
    }
}

impl App {
    fn new(conn: RustConnection, screen_num: usize, cmd_atom: Atom) -> Res<App> {
        let setup = conn.setup();
        let screen = &setup.roots[screen_num];
        let (root, sw, sh) = (screen.root, screen.width_in_pixels as i32, screen.height_in_pixels as i32);
        let lsb_first = setup.image_byte_order == ImageOrder::LSB_FIRST;
        let visual = screen
            .allowed_depths
            .iter()
            .filter(|d| d.depth == 32)
            .flat_map(|d| &d.visuals)
            .find(|v| v.class == VisualClass::TRUE_COLOR && v.red_mask == 0xff0000 && v.blue_mask == 0xff)
            .ok_or("the X server has no 32-bit ARGB visual")?
            .visual_id;

        let cm_atom = conn.intern_atom(false, format!("_NET_WM_CM_S{screen_num}").as_bytes())?.reply()?.atom;
        if conn.get_selection_owner(cm_atom)?.reply()?.owner == x11rb::NONE {
            eprintln!(
                "magicpen: no compositor is running, so the overlay cannot be transparent.\n\
                 Enable one first (Xfce: Settings > Window Manager Tweaks > Compositor)."
            );
        }

        let colormap = conn.generate_id()?;
        conn.create_colormap(ColormapAlloc::NONE, colormap, root, visual)?;
        let win = conn.generate_id()?;
        // Override-redirect keeps the window manager from decorating, focusing or moving the overlay.
        let aux = CreateWindowAux::new()
            .background_pixel(0)
            .border_pixel(0)
            .colormap(colormap)
            .override_redirect(1)
            .event_mask(
                EventMask::EXPOSURE
                    | EventMask::BUTTON_PRESS
                    | EventMask::BUTTON_RELEASE
                    | EventMask::POINTER_MOTION
                    | EventMask::LEAVE_WINDOW
                    | EventMask::KEY_PRESS,
            );
        conn.create_window(32, win, root, 0, 0, sw as u16, sh as u16, 0, WindowClass::INPUT_OUTPUT, visual, &aux)?;
        conn.change_property8(PropMode::REPLACE, win, AtomEnum::WM_NAME, AtomEnum::STRING, b"magicpen")?;
        conn.change_property8(PropMode::REPLACE, win, AtomEnum::WM_CLASS, AtomEnum::STRING, b"magicpen\0Magicpen\0")?;
        let gc = conn.generate_id()?;
        conn.create_gc(gc, win, &CreateGCAux::new().graphics_exposures(0))?;

        // Watch the root so the overlay can stay above newly raised windows.
        conn.change_window_attributes(
            root,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::SUBSTRUCTURE_NOTIFY),
        )?;

        let font = conn.generate_id()?;
        conn.open_font(font, b"cursor")?;
        let mut cursors = [0; 3];
        for (cursor, glyph) in cursors.iter_mut().zip(CURSOR_GLYPHS) {
            *cursor = conn.generate_id()?;
            conn.create_glyph_cursor(*cursor, font, font, glyph, glyph + 1, 0, 0, 0, 0xffff, 0xffff, 0xffff)?;
        }
        conn.close_font(font)?;

        let cfg = Config::load();
        let scale = if cfg.scale > 0.0 { cfg.scale } else { detect_scale(&conn, root) };
        let ui = Ui::new(scale);
        let tb_pos = cfg.toolbar.unwrap_or(((sw - ui.tb.0) / 2, ui.margin * 2));
        let keymap = Keymap::load(&conn)?;
        let max_req = conn.maximum_request_bytes();

        let mut app = App {
            root,
            win,
            gc,
            sw,
            sh,
            lsb_first,
            max_req,
            cmd_atom,
            canvas: Canvas::new(sw as u32, sh as u32).ok_or("invalid screen size")?,
            cur: None,
            cfg_dirty: false,
            draw_mode: false,
            visible: true,
            quit: false,
            tb_pos,
            tb_pix: Pixmap::new(1, 1).unwrap(),
            panel_pix: None,
            collapsed: false,
            eye_pix: None,
            panel_open: false,
            ui_dirty: true,
            hover: None,
            recording: None,
            drag: Drag::None,
            ptr: None,
            ring: None,
            keymap,
            hotkeys: [None; 3],
            hotkey_ok: [false; 3],
            kbd_grabbed: false,
            cursors,
            cursor: usize::MAX,
            damage: Vec::new(),
            buf: Vec::new(),
            ui,
            cfg,
            conn,
        };
        app.tb_pos = app.clamp_toolbar(tb_pos);
        for i in 0..app.hotkeys.len() {
            app.hotkeys[i] = Hotkey::parse(&app.cfg.hotkeys[i]);
            app.hotkey_ok[i] = app.hotkeys[i].is_some_and(|hk| app.grab_hotkey(hk, true));
            if !app.hotkey_ok[i] {
                eprintln!(
                    "magicpen: shortcut '{}' is unavailable (taken by another program?); \
                     change it in the settings panel",
                    app.cfg.hotkeys[i]
                );
            }
        }
        Ok(app)
    }

    // ---- geometry -------------------------------------------------------

    fn clamp_toolbar(&self, (x, y): (i32, i32)) -> (i32, i32) {
        let (w, h) = self.ui.tb;
        (x.clamp(0, (self.sw - w).max(0)), y.clamp(0, (self.sh - h).max(0)))
    }

    fn tb_rect(&self) -> R {
        R::new(self.tb_pos.0, self.tb_pos.1, self.ui.tb.0, self.ui.tb.1)
    }

    /// The settings panel hangs below the toolbar, or above it near the screen bottom.
    fn panel_rect(&self) -> R {
        let tb = self.tb_rect();
        let (w, h) = self.ui.panel;
        let x = tb.x.clamp(0, (self.sw - w).max(0));
        let below = tb.y + tb.h + self.ui.margin;
        let y = if below + h <= self.sh { below } else { (tb.y - self.ui.margin - h).max(0) };
        R::new(x, y, w, h)
    }

    /// A widget's rectangle grown by the shadow margin its pixmap carries.
    fn outer(&self, r: R) -> R {
        let m = self.ui.margin;
        R::new(r.x - m, r.y - m, r.w + 2 * m, r.h + 2 * m)
    }

    /// Where the eye button sits while collapsed: over the toolbar's own eye, so
    /// collapsing and expanding need no pointer movement.
    fn eye_rect(&self) -> R {
        let (tb, side) = (self.tb_rect(), self.ui.tb.1);
        let hide = self.ui.item_rect(Item::Hide);
        let x = tb.x + hide.x + hide.w / 2 - side / 2;
        R::new(x.clamp(0, (self.sw - side).max(0)), tb.y, side, side)
    }

    fn over_ui(&self, x: i32, y: i32) -> bool {
        if self.collapsed {
            return self.eye_rect().contains(x, y);
        }
        self.tb_rect().contains(x, y) || (self.panel_open && self.panel_rect().contains(x, y))
    }

    fn damage(&mut self, r: R) {
        let mut r = r.clip(self.sw, self.sh);
        if r.is_empty() {
            return;
        }
        while let Some(i) = self.damage.iter().position(|d| d.intersects(&r)) {
            r = r.union(&self.damage.swap_remove(i));
        }
        self.damage.push(r);
    }

    fn damage_all(&mut self) {
        self.damage(R::new(0, 0, self.sw, self.sh));
    }

    fn damage_ui(&mut self) {
        if self.collapsed {
            self.damage(self.outer(self.eye_rect()));
            return;
        }
        self.damage(self.outer(self.tb_rect()));
        if self.panel_open {
            self.damage(self.outer(self.panel_rect()));
        }
    }

    // ---- X state --------------------------------------------------------

    /// In drawing mode the overlay takes all pointer input; otherwise only the
    /// widgets do and everything else falls through to the windows below.
    fn sync_input_shape(&self) {
        let rect = |r: R| Rectangle { x: r.x as i16, y: r.y as i16, width: r.w as u16, height: r.h as u16 };
        let mut rects = Vec::new();
        if self.collapsed {
            rects.push(rect(self.eye_rect()));
        } else if self.draw_mode {
            rects.push(rect(R::new(0, 0, self.sw, self.sh)));
        } else {
            rects.push(rect(self.tb_rect()));
            if self.panel_open {
                rects.push(rect(self.panel_rect()));
            }
        }
        let _ = self.conn.shape_rectangles(
            shape::SO::SET,
            shape::SK::INPUT,
            ClipOrdering::UNSORTED,
            self.win,
            0,
            0,
            &rects,
        );
    }

    /// The overlay never has focus, so it grabs the keyboard while it needs keys.
    fn sync_keyboard_grab(&mut self) {
        let want = self.visible && (self.draw_mode || self.recording.is_some());
        if want == self.kbd_grabbed {
            return;
        }
        if want {
            let _ = self.conn.grab_keyboard(false, self.win, x11rb::CURRENT_TIME, GrabMode::ASYNC, GrabMode::ASYNC);
        } else {
            let _ = self.conn.ungrab_keyboard(x11rb::CURRENT_TIME);
        }
        self.kbd_grabbed = want;
    }

    /// Register or release a global shortcut. Returns false if the grab was refused.
    fn grab_hotkey(&self, hk: Hotkey, on: bool) -> bool {
        let Some(code) = self.keymap.keycode(hk.sym) else { return false };
        let mut ok = true;
        // Grab every Caps Lock / Num Lock combination so the locks don't matter.
        for locks in [0, keys::LOCK, keys::NUM, keys::LOCK | keys::NUM] {
            let mods = ModMask::from(hk.mods | locks);
            if on {
                let cookie = self.conn.grab_key(false, self.root, mods, code, GrabMode::ASYNC, GrabMode::ASYNC);
                ok &= cookie.ok().and_then(|c| c.check().ok()).is_some();
            } else {
                let _ = self.conn.ungrab_key(code, self.root, mods);
            }
        }
        ok
    }

    fn set_hotkey(&mut self, i: usize, hk: Hotkey) {
        if let Some(old) = self.hotkeys[i] {
            self.grab_hotkey(old, false);
        }
        self.hotkeys[i] = Some(hk);
        self.hotkey_ok[i] = self.grab_hotkey(hk, true);
        self.cfg.hotkeys[i] = hk.to_string();
        self.cfg_dirty = true;
    }

    fn raise(&self) {
        let _ = self.conn.configure_window(self.win, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE));
    }

    // ---- actions --------------------------------------------------------

    fn set_draw(&mut self, on: bool) {
        if let Some(shape) = self.cur.take() {
            self.damage(shape.bounds());
        }
        self.drag = Drag::None;
        self.draw_mode = on;
        self.ui_dirty = true;
        self.sync_input_shape();
        self.sync_keyboard_grab();
    }

    fn set_visible(&mut self, on: bool) {
        if on == self.visible {
            return;
        }
        if on {
            let _ = self.conn.map_window(self.win);
            self.raise();
        } else {
            self.set_draw(false);
            self.recording = None;
            let _ = self.conn.unmap_window(self.win);
        }
        self.visible = on;
        self.sync_keyboard_grab();
    }

    /// Collapse to just the eye button (annotations and toolbar hidden), or expand again.
    fn set_collapsed(&mut self, on: bool) {
        if on == self.collapsed {
            return;
        }
        if on {
            self.set_draw(false);
            if self.panel_open {
                self.set_panel_open(false);
            }
        }
        self.collapsed = on;
        self.hover = None;
        self.ui_dirty = true;
        self.damage_all();
        self.sync_input_shape();
    }

    fn toggle_draw(&mut self) {
        let on = !self.visible || self.collapsed || !self.draw_mode;
        self.set_visible(true);
        self.set_collapsed(false);
        self.set_draw(on);
    }

    fn clear(&mut self) {
        if self.canvas.clear() {
            self.damage_all();
        }
    }

    fn undo(&mut self) {
        if self.canvas.undo() {
            self.damage_all();
        }
    }

    fn redo(&mut self) {
        if self.canvas.redo() {
            self.damage_all();
        }
    }

    fn select_tool(&mut self, tool: Tool) {
        self.cfg.tool = tool;
        self.cfg_dirty = true;
        self.ui_dirty = true;
    }

    fn select_color(&mut self, i: usize) {
        self.cfg.color = i.min(PALETTE.len() - 1);
        if self.cfg.tool == Tool::Eraser {
            self.cfg.tool = Tool::Pen;
        }
        self.cfg_dirty = true;
        self.ui_dirty = true;
    }

    fn set_size(&mut self, slot: usize, value: f32) {
        let (_, min, max) = SLIDERS[slot];
        let value = value.round().clamp(min, max);
        if value != self.cfg.sizes[slot] {
            self.cfg.sizes[slot] = value;
            self.cfg_dirty = true;
            self.ui_dirty = true;
        }
    }

    /// Grow or shrink the current tool's size by one step.
    fn adjust_size(&mut self, dir: f32) {
        let slot = self.cfg.tool.size_slot();
        let v = self.cfg.sizes[slot];
        self.set_size(slot, v + dir * (v * 0.15).max(1.0));
    }

    fn set_panel_open(&mut self, open: bool) {
        if !open {
            self.damage(self.outer(self.panel_rect()));
            self.recording = None;
        }
        self.panel_open = open;
        self.ui_dirty = true;
        self.sync_input_shape();
        self.sync_keyboard_grab();
    }

    fn activate(&mut self, item: Item) {
        match item {
            Item::Grip => {}
            Item::Cursor => self.set_draw(false),
            Item::Tool(t) => {
                self.select_tool(t);
                self.set_draw(true);
            }
            Item::Color(i) => self.select_color(i),
            Item::SizeDown => self.adjust_size(-1.0),
            Item::SizeUp => self.adjust_size(1.0),
            Item::Undo => self.undo(),
            Item::Redo => self.redo(),
            Item::Clear => self.clear(),
            Item::Hide => self.set_collapsed(true),
            Item::Show => self.set_collapsed(false),
            Item::Settings => self.set_panel_open(!self.panel_open),
            Item::Quit => self.quit = true,
        }
    }

    fn run_command(&mut self, cmd: u32) {
        match cmd {
            CMD_TOGGLE => self.toggle_draw(),
            CMD_SHOW => self.set_visible(!self.visible),
            CMD_CLEAR => self.clear(),
            CMD_UNDO => self.undo(),
            CMD_QUIT => self.quit = true,
            _ => {}
        }
    }

    // ---- input ----------------------------------------------------------

    fn handle(&mut self, ev: Event) -> Res<()> {
        match ev {
            Event::Expose(e) => self.damage(R::new(e.x as i32, e.y as i32, e.width as i32, e.height as i32)),
            Event::ButtonPress(e) => self.on_press(e.event_x as i32, e.event_y as i32, e.detail),
            Event::ButtonRelease(e) if e.detail == 1 => self.on_release(),
            Event::MotionNotify(e) => self.on_motion(e.event_x as i32, e.event_y as i32, u16::from(e.state)),
            Event::LeaveNotify(_) => {
                self.ptr = None;
                if self.hover.take().is_some() {
                    self.ui_dirty = true;
                }
            }
            Event::KeyPress(e) => self.on_key(e.detail, u16::from(e.state)),
            Event::ClientMessage(e) if e.type_ == self.cmd_atom => self.run_command(e.data.as_data32()[0]),
            // The window manager restacked something; stay on top. Popups
            // (menus, tooltips, screen lockers) are deliberately left above.
            Event::ConfigureNotify(e) if self.visible && e.window != self.win && !e.override_redirect => self.raise(),
            Event::MapNotify(e) if self.visible && e.window != self.win && !e.override_redirect => self.raise(),
            Event::MappingNotify(_) => self.keymap = Keymap::load(&self.conn)?,
            _ => {}
        }
        Ok(())
    }

    fn on_press(&mut self, x: i32, y: i32, button: u8) {
        self.ptr = Some((x, y));
        match button {
            1 => {}
            4 | 5 => {
                if self.draw_mode && !self.over_ui(x, y) {
                    self.adjust_size(if button == 4 { 1.0 } else { -1.0 });
                }
                return;
            }
            _ => return,
        }

        if self.collapsed {
            if self.eye_rect().contains(x, y) {
                self.set_collapsed(false);
            }
            return;
        }

        let panel = self.panel_rect();
        if self.panel_open && panel.contains(x, y) {
            match self.ui.panel_hit(x - panel.x, y - panel.y) {
                Some(PanelHit::Slider(i)) => {
                    self.recording = None;
                    self.drag = Drag::Slider(i);
                    self.drag_slider(i, x);
                }
                Some(PanelHit::Hotkey(i)) => {
                    self.recording = if self.recording == Some(i) { None } else { Some(i) };
                }
                None => self.recording = None,
            }
            self.ui_dirty = true;
            self.sync_keyboard_grab();
            return;
        }
        if self.recording.take().is_some() {
            self.ui_dirty = true;
            self.sync_keyboard_grab();
        }

        let tb = self.tb_rect();
        if tb.contains(x, y) {
            match self.ui.item_at(x - tb.x, y - tb.y) {
                None | Some(Item::Grip) => self.drag = Drag::Toolbar { dx: x - tb.x, dy: y - tb.y },
                Some(item) => self.activate(item),
            }
            return;
        }

        if self.draw_mode {
            let tool = self.cfg.tool;
            let [r, g, b] = PALETTE[self.cfg.color];
            let sizes = self.cfg.sizes;
            let (rgba, width) = match tool {
                Tool::Highlighter => ([r, g, b, (sizes[2] * 2.55) as u8], sizes[1]),
                Tool::Eraser => ([0, 0, 0, 255], sizes[3]),
                _ => ([r, g, b, 255], sizes[0]),
            };
            let p = (x as f32 + 0.5, y as f32 + 0.5);
            let pts = if tool.is_freehand() { vec![p] } else { vec![p, p] };
            let shape = Shape { tool, rgba, width, pts };
            self.damage(shape.bounds());
            self.cur = Some(shape);
            self.drag = Drag::Shape;
        }
    }

    fn on_motion(&mut self, x: i32, y: i32, state: u16) {
        self.ptr = Some((x, y));
        match self.drag {
            Drag::Toolbar { dx, dy } => {
                self.damage_ui();
                self.tb_pos = self.clamp_toolbar((x - dx, y - dy));
                self.damage_ui();
            }
            Drag::Slider(i) => self.drag_slider(i, x),
            Drag::Shape => {
                let Some(shape) = &mut self.cur else { return };
                let p = (x as f32 + 0.5, y as f32 + 0.5);
                let changed = if shape.tool.is_freehand() {
                    let last = *shape.pts.last().unwrap();
                    if (p.0 - last.0).abs() < 1.0 && (p.1 - last.1).abs() < 1.0 {
                        return;
                    }
                    shape.pts.push(p);
                    shape.tail_bounds()
                } else {
                    let before = shape.bounds();
                    shape.pts[1] = constrain(shape.tool, shape.pts[0], p, state & keys::SHIFT != 0);
                    before.union(&shape.bounds())
                };
                self.damage(changed);
            }
            Drag::None => {
                let tb = self.tb_rect();
                let hover = if self.collapsed {
                    self.eye_rect().contains(x, y).then_some(Item::Show)
                } else {
                    self.ui.item_at(x - tb.x, y - tb.y).filter(|i| *i != Item::Grip)
                };
                if hover != self.hover {
                    self.hover = hover;
                    self.ui_dirty = true;
                }
            }
        }
    }

    fn on_release(&mut self) {
        match std::mem::take(&mut self.drag) {
            Drag::Shape => {
                if let Some(shape) = self.cur.take() {
                    self.damage(shape.bounds());
                    if shape.is_visible() {
                        self.canvas.commit(shape);
                    }
                }
            }
            Drag::Toolbar { .. } => {
                self.cfg.toolbar = Some(self.tb_pos);
                self.cfg_dirty = true;
                self.sync_input_shape();
            }
            Drag::Slider(_) | Drag::None => {}
        }
    }

    fn drag_slider(&mut self, i: usize, x: i32) {
        let (_, min, max) = SLIDERS[i];
        let frac = self.ui.slider_frac(x - self.panel_rect().x);
        self.set_size(i, min + frac * (max - min));
    }

    fn on_key(&mut self, code: u8, state: u16) {
        let mods = state & keys::MOD_MASK;
        let sym = self.keymap.sym(code);

        if let Some(i) = self.recording {
            if keys::is_modifier(sym) {
                return;
            }
            let hk = Hotkey { mods, sym };
            if sym == keys::XK_ESCAPE && mods == 0 {
                self.recording = None;
            } else if mods & !keys::SHIFT != 0 || hk.is_function_key() {
                // A bare letter as a global shortcut would make it untypeable everywhere.
                self.set_hotkey(i, hk);
                self.recording = None;
            }
            self.ui_dirty = true;
            self.sync_keyboard_grab();
            return;
        }

        // Passive grabs don't fire while we hold the keyboard, so match shortcuts here too.
        if let Some(i) = self.hotkeys.iter().position(|hk| *hk == Some(Hotkey { mods, sym })) {
            match i {
                0 => self.toggle_draw(),
                1 => self.set_visible(!self.visible),
                _ => self.clear(),
            }
            return;
        }
        if !self.draw_mode {
            return;
        }

        let (ctrl, shift) = (mods & keys::CTRL != 0, mods & keys::SHIFT != 0);
        match char::from_u32(sym) {
            _ if sym == keys::XK_ESCAPE => self.set_draw(false),
            _ if sym == keys::XK_DELETE => self.clear(),
            Some('z') if ctrl && shift => self.redo(),
            Some('z') if ctrl => self.undo(),
            Some('y') if ctrl => self.redo(),
            _ if mods & !keys::SHIFT != 0 => {}
            Some('p') => self.select_tool(Tool::Pen),
            Some('h') => self.select_tool(Tool::Highlighter),
            Some('l') => self.select_tool(Tool::Line),
            Some('a') => self.select_tool(Tool::Arrow),
            Some('r') => self.select_tool(Tool::Rect),
            Some('o') => self.select_tool(Tool::Ellipse),
            Some('e') => self.select_tool(Tool::Eraser),
            Some(c @ '1'..='8') => self.select_color(c as usize - '1' as usize),
            Some('[') => self.adjust_size(-1.0),
            Some(']') => self.adjust_size(1.0),
            _ => {}
        }
    }

    // ---- output ---------------------------------------------------------

    /// Pick the pointer cursor and track the eraser outline for the current pointer position.
    fn sync_pointer_feedback(&mut self) {
        let free = self.draw_mode && self.ptr.is_some_and(|(x, y)| !self.over_ui(x, y));
        let cursor = match self.cfg.tool {
            _ if !free => 0,
            Tool::Pen | Tool::Highlighter => 1,
            _ => 2,
        };
        if cursor != self.cursor {
            self.cursor = cursor;
            let aux = ChangeWindowAttributesAux::new().cursor(self.cursors[cursor]);
            let _ = self.conn.change_window_attributes(self.win, &aux);
        }

        let ring = match self.ptr {
            Some((x, y)) if free && self.cfg.tool == Tool::Eraser => {
                let rad = (self.cfg.sizes[3] / 2.0).ceil() as i32 + 3;
                Some(R::new(x - rad, y - rad, 2 * rad + 1, 2 * rad + 1))
            }
            _ => None,
        };
        if ring != self.ring {
            for r in [self.ring, ring].into_iter().flatten() {
                self.damage(r);
            }
            self.ring = ring;
        }
    }

    fn flush(&mut self) -> Res<()> {
        self.sync_pointer_feedback();
        if self.ui_dirty {
            self.ui_dirty = false;
            let view = View {
                cfg: &self.cfg,
                draw_mode: self.draw_mode,
                hover: self.hover,
                panel_open: self.panel_open,
                recording: self.recording,
                hotkey_ok: self.hotkey_ok,
            };
            let toolbar = self.ui.draw_toolbar(&view);
            let panel = self.panel_open.then(|| self.ui.draw_panel(&view));
            self.tb_pix = toolbar;
            self.panel_pix = panel;
            self.eye_pix = self.collapsed.then(|| self.ui.draw_eye(self.hover == Some(Item::Show)));
            self.damage_ui();
        }
        for r in std::mem::take(&mut self.damage) {
            if self.visible {
                self.present(r)?;
            }
        }
        if self.cfg_dirty && matches!(self.drag, Drag::None) {
            self.cfg_dirty = false;
            self.cfg.save();
        }
        self.conn.flush()?;
        Ok(())
    }

    /// Compose one screen region (canvas, shape in progress, widgets) and send it to the window.
    fn present(&mut self, r: R) -> Res<()> {
        let (w, h) = (r.w as usize, r.h as usize);
        let row_bytes = w * 4;
        let mut buf = std::mem::take(&mut self.buf);
        buf.resize(row_bytes * h, 0);

        let base = self.canvas.base.data();
        let stride = self.sw as usize * 4;
        if self.collapsed {
            buf.fill(0);
        } else {
            for (row, out) in buf.chunks_exact_mut(row_bytes).enumerate() {
                let start = (r.y as usize + row) * stride + r.x as usize * 4;
                out.copy_from_slice(&base[start..start + row_bytes]);
            }
        }

        {
            let mut pm = PixmapMut::from_bytes(&mut buf, w as u32, h as u32).ok_or("bad damage rectangle")?;
            if let Some(shape) = &self.cur {
                shape.draw(&mut pm, r.x as f32, r.y as f32);
            }
            if let (Some(_), Some((x, y))) = (self.ring, self.ptr) {
                let ts = Transform::from_translate(-r.x as f32, -r.y as f32);
                let circle = PathBuilder::from_circle(x as f32 + 0.5, y as f32 + 0.5, self.cfg.sizes[3] / 2.0);
                let mut paint = Paint { anti_alias: true, ..Paint::default() };
                for (rgba, width) in [([0, 0, 0, 130], 3.0), ([255, 255, 255, 255], 1.2)] {
                    paint.set_color_rgba8(rgba[0], rgba[1], rgba[2], rgba[3]);
                    let stroke = Stroke { width, line_cap: LineCap::Round, ..Stroke::default() };
                    if let Some(circle) = &circle {
                        pm.stroke_path(circle, &paint, &stroke, ts, None);
                    }
                }
            }
            let mut blit = |at: R, pix: &Pixmap| {
                if at.intersects(&r) {
                    let paint = PixmapPaint::default();
                    pm.draw_pixmap(at.x - r.x, at.y - r.y, pix.as_ref(), &paint, Transform::identity(), None);
                }
            };
            if let (true, Some(pix)) = (self.collapsed, &self.eye_pix) {
                blit(self.outer(self.eye_rect()), pix);
            } else {
                blit(self.outer(self.tb_rect()), &self.tb_pix);
                if let (true, Some(pix)) = (self.panel_open, &self.panel_pix) {
                    blit(self.outer(self.panel_rect()), pix);
                }
            }
        }

        // tiny-skia renders premultiplied RGBA; an ARGB32 visual wants B,G,R,A bytes on little-endian.
        for px in buf.as_chunks_mut::<4>().0 {
            if self.lsb_first {
                px.swap(0, 2);
            } else {
                px.rotate_right(1);
            }
        }

        let max_rows = (self.max_req.saturating_sub(64) / row_bytes).max(1);
        for (i, chunk) in buf.chunks(max_rows * row_bytes).enumerate() {
            self.conn.put_image(
                ImageFormat::Z_PIXMAP,
                self.win,
                self.gc,
                w as u16,
                (chunk.len() / row_bytes) as u16,
                r.x as i16,
                (r.y as usize + i * max_rows) as i16,
                0,
                32,
                chunk,
            )?;
        }
        self.buf = buf;
        Ok(())
    }
}
