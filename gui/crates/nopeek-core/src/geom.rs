//! Geometry and colour primitives in device-independent pixels (DIPs).
//! Adapted from `oyenza-studio/crates/poni-core/src/geom.rs`.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
    pub fn dist(self, o: Point) -> f32 {
        ((self.x - o.x).powi(2) + (self.y - o.y).powi(2)).sqrt()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub w: f32,
    pub h: f32,
}

impl Size {
    pub const fn new(w: f32, h: f32) -> Self {
        Self { w, h }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
    pub fn from_center(c: Point, w: f32, h: f32) -> Self {
        Self::new(c.x - w / 2.0, c.y - h / 2.0, w, h)
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn center(&self) -> Point {
        Point::new(self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.right() && p.y >= self.y && p.y < self.bottom()
    }
    pub fn inset(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(
            self.x + dx,
            self.y + dy,
            (self.w - 2.0 * dx).max(0.0),
            (self.h - 2.0 * dy).max(0.0),
        )
    }
    pub fn offset(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.w, self.h)
    }
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

pub fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

/// Straight (non-premultiplied) RGBA colour, components 0..=1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }
    pub const fn hex(rgb: u32) -> Self {
        Self {
            r: ((rgb >> 16) & 0xff) as f32 / 255.0,
            g: ((rgb >> 8) & 0xff) as f32 / 255.0,
            b: (rgb & 0xff) as f32 / 255.0,
            a: 1.0,
        }
    }
    pub const fn white(a: f32) -> Self {
        Self::rgba(1.0, 1.0, 1.0, a)
    }
    pub const fn black(a: f32) -> Self {
        Self::rgba(0.0, 0.0, 0.0, a)
    }
    pub fn with_alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }
    pub fn mul_alpha(self, f: f32) -> Self {
        Self {
            a: self.a * f,
            ..self
        }
    }
}

pub mod palette {
    use super::Color;
    pub const ISLAND: Color = Color::black(1.0);
    pub const BORDER: Color = Color::white(0.08);
    pub const TEXT: Color = Color::white(0.96);
    pub const TEXT_DIM: Color = Color::white(0.65);
    /// Amber/Yellow accent for ALERT (>= 2s)
    pub const ALERT_AMBER: Color = Color::hex(0xFF9F0A);
    pub const ALERT_YELLOW: Color = Color::hex(0xFFD60A);
    /// Red danger accent for ESCALATED (>= 7s)
    pub const ESCALATED_RED: Color = Color::hex(0xFF453A);
    /// Safe monitoring green for tray status
    pub const SAFE_GREEN: Color = Color::hex(0x30D158);
}
