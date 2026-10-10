use tiny_skia::{
    BlendMode, Color, FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, PixmapMut, Rect, Stroke, Transform,
};

/// Integer pixel rectangle in screen coordinates.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct R {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl R {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> R {
        R { x, y, w, h }
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }

    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    pub fn intersects(&self, o: &R) -> bool {
        self.x < o.x + o.w && o.x < self.x + self.w && self.y < o.y + o.h && o.y < self.y + self.h
    }

    pub fn union(&self, o: &R) -> R {
        let (x, y) = (self.x.min(o.x), self.y.min(o.y));
        let (r, b) = ((self.x + self.w).max(o.x + o.w), (self.y + self.h).max(o.y + o.h));
        R::new(x, y, r - x, b - y)
    }

    pub fn clip(&self, w: i32, h: i32) -> R {
        let (x, y) = (self.x.max(0), self.y.max(0));
        let (r, b) = ((self.x + self.w).min(w), (self.y + self.h).min(h));
        R::new(x, y, r - x, b - y)
    }

    pub fn offset(&self, dx: i32, dy: i32) -> R {
        R::new(self.x + dx, self.y + dy, self.w, self.h)
    }

    /// Bounding box of a set of points, grown by `pad` on every side.
    pub fn around(pts: &[(f32, f32)], pad: f32) -> R {
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for &(x, y) in pts {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        let (x, y) = ((x0 - pad).floor() as i32, (y0 - pad).floor() as i32);
        R::new(x, y, (x1 + pad).ceil() as i32 - x, (y1 + pad).ceil() as i32 - y)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Pen,
    Highlighter,
    Line,
    Arrow,
    Rect,
    Ellipse,
    Eraser,
}

impl Tool {
    pub const ALL: [Tool; 7] =
        [Tool::Pen, Tool::Highlighter, Tool::Line, Tool::Arrow, Tool::Rect, Tool::Ellipse, Tool::Eraser];

    pub fn name(self) -> &'static str {
        match self {
            Tool::Pen => "pen",
            Tool::Highlighter => "highlighter",
            Tool::Line => "line",
            Tool::Arrow => "arrow",
            Tool::Rect => "rect",
            Tool::Ellipse => "ellipse",
            Tool::Eraser => "eraser",
        }
    }

    pub fn from_name(name: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|t| t.name() == name)
    }

    pub fn is_freehand(self) -> bool {
        matches!(self, Tool::Pen | Tool::Highlighter | Tool::Eraser)
    }

    /// Index into `Config::sizes` of the size this tool draws with.
    pub fn size_slot(self) -> usize {
        match self {
            Tool::Highlighter => 1,
            Tool::Eraser => 3,
            _ => 0,
        }
    }
}

/// One annotation: a freehand stroke (all points) or a two-point shape.
#[derive(Clone)]
pub struct Shape {
    pub tool: Tool,
    pub rgba: [u8; 4],
    pub width: f32,
    pub pts: Vec<(f32, f32)>,
}

impl Shape {
    fn arrow_head(&self) -> f32 {
        (self.width * 3.5).max(12.0)
    }

    fn pad(&self) -> f32 {
        let extra = if self.tool == Tool::Arrow { self.arrow_head() } else { 0.0 };
        self.width / 2.0 + extra + 2.0
    }

    pub fn bounds(&self) -> R {
        R::around(&self.pts, self.pad())
    }

    /// Area that changes when the newest freehand point is appended.
    pub fn tail_bounds(&self) -> R {
        let n = self.pts.len();
        R::around(&self.pts[n.saturating_sub(3)..], self.pad())
    }

    /// False for a shape too small to be worth keeping (an accidental click).
    pub fn is_visible(&self) -> bool {
        if self.tool.is_freehand() {
            return !self.pts.is_empty();
        }
        match self.pts[..] {
            [a, b] => (a.0 - b.0).abs() >= 2.0 || (a.1 - b.1).abs() >= 2.0,
            _ => false,
        }
    }

    /// Draw onto `pm`, whose top-left pixel sits at screen position (`ox`, `oy`).
    pub fn draw(&self, pm: &mut PixmapMut, ox: f32, oy: f32) {
        let ts = Transform::from_translate(-ox, -oy);
        let mut paint = Paint { anti_alias: true, ..Paint::default() };
        let [r, g, b, a] = self.rgba;
        paint.set_color_rgba8(r, g, b, a);
        if self.tool == Tool::Eraser {
            paint.blend_mode = BlendMode::Clear;
        }
        let stroke =
            Stroke { width: self.width, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Stroke::default() };
        let Some(&first) = self.pts.first() else { return };
        let last = *self.pts.last().unwrap();

        if self.tool.is_freehand() {
            if self.pts.len() == 1 {
                if let Some(dot) = PathBuilder::from_circle(first.0, first.1, self.width / 2.0) {
                    pm.fill_path(&dot, &paint, FillRule::Winding, ts, None);
                }
                return;
            }
            // Smooth the polyline by curving through segment midpoints.
            let mut pb = PathBuilder::new();
            pb.move_to(first.0, first.1);
            for pair in self.pts[1..].windows(2) {
                let (c, n) = (pair[0], pair[1]);
                pb.quad_to(c.0, c.1, (c.0 + n.0) / 2.0, (c.1 + n.1) / 2.0);
            }
            pb.line_to(last.0, last.1);
            if let Some(path) = pb.finish() {
                pm.stroke_path(&path, &paint, &stroke, ts, None);
            }
            return;
        }

        let (a, b) = (first, last);
        match self.tool {
            Tool::Line => {
                let mut pb = PathBuilder::new();
                pb.move_to(a.0, a.1);
                pb.line_to(b.0, b.1);
                if let Some(path) = pb.finish() {
                    pm.stroke_path(&path, &paint, &stroke, ts, None);
                }
            }
            Tool::Arrow => {
                let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                let len = (dx * dx + dy * dy).sqrt();
                if len < 1.0 {
                    return;
                }
                let (ux, uy) = (dx / len, dy / len);
                let head = self.arrow_head().min(len);
                let base = (b.0 - ux * head, b.1 - uy * head);
                let half = head * 0.5;
                let mut pb = PathBuilder::new();
                pb.move_to(a.0, a.1);
                pb.line_to(base.0, base.1);
                if let Some(path) = pb.finish() {
                    pm.stroke_path(&path, &paint, &stroke, ts, None);
                }
                let mut pb = PathBuilder::new();
                pb.move_to(b.0, b.1);
                pb.line_to(base.0 - uy * half, base.1 + ux * half);
                pb.line_to(base.0 + uy * half, base.1 - ux * half);
                pb.close();
                if let Some(path) = pb.finish() {
                    pm.fill_path(&path, &paint, FillRule::Winding, ts, None);
                }
            }
            Tool::Rect | Tool::Ellipse => {
                let Some(rect) = Rect::from_ltrb(a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1)) else {
                    return;
                };
                let path = if self.tool == Tool::Rect {
                    Some(PathBuilder::from_rect(rect))
                } else {
                    PathBuilder::from_oval(rect)
                };
                if let Some(path) = path {
                    pm.stroke_path(&path, &paint, &stroke, ts, None);
                }
            }
            _ => {}
        }
    }
}

enum Op {
    Shape(Shape),
    Clear,
}

/// Committed annotations: a vector history plus its rasterised result.
pub struct Canvas {
    pub base: Pixmap,
    ops: Vec<Op>,
    redo: Vec<Op>,
}

impl Canvas {
    pub fn new(w: u32, h: u32) -> Option<Canvas> {
        Some(Canvas { base: Pixmap::new(w, h)?, ops: Vec::new(), redo: Vec::new() })
    }

    fn apply(base: &mut Pixmap, op: &Op) {
        match op {
            Op::Shape(s) => s.draw(&mut base.as_mut(), 0.0, 0.0),
            Op::Clear => base.fill(Color::TRANSPARENT),
        }
    }

    pub fn commit(&mut self, shape: Shape) {
        self.redo.clear();
        self.ops.push(Op::Shape(shape));
        Self::apply(&mut self.base, self.ops.last().unwrap());
    }

    /// Returns false when there was nothing to clear.
    pub fn clear(&mut self) -> bool {
        if matches!(self.ops.last(), None | Some(Op::Clear)) {
            return false;
        }
        self.redo.clear();
        self.ops.push(Op::Clear);
        self.base.fill(Color::TRANSPARENT);
        true
    }

    pub fn undo(&mut self) -> bool {
        let Some(op) = self.ops.pop() else { return false };
        self.redo.push(op);
        self.base.fill(Color::TRANSPARENT);
        // Nothing before the most recent clear can still be visible.
        let start = self.ops.iter().rposition(|op| matches!(op, Op::Clear)).map_or(0, |i| i + 1);
        for op in &self.ops[start..] {
            Self::apply(&mut self.base, op);
        }
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(op) = self.redo.pop() else { return false };
        Self::apply(&mut self.base, &op);
        self.ops.push(op);
        true
    }
}
