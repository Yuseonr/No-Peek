//! Rendering on macOS: Core Graphics for shapes and icons, Core Text for words,
//! drawn into the NSView's layer with a flipped (y-down) coordinate space.
//! Adapted from `oyenza-studio/crates/poni/src/mac/gfx.rs`.

use nopeek_core::geom::{Color, Point, Rect};
use nopeek_core::icons::{self, Icon};
use nopeek_core::svgpath::Seg;
use objc2::rc::Retained;
use objc2_app_kit::{NSFont, NSFontWeightBold, NSFontWeightRegular, NSFontWeightSemibold};
use objc2_core_foundation::{
    CFAttributedString, CFBoolean, CFDictionary, CFRange, CFRetained, CFString, CFType,
    CGAffineTransform, CGFloat, CGPoint, CGRect, CGSize,
};
use objc2_core_graphics::{
    kCGColorSpaceSRGB, CGBitmapContextCreate, CGBitmapContextCreateImage, CGColorSpace, CGContext,
    CGImage, CGImageAlphaInfo, CGImageByteOrderInfo, CGInterpolationQuality, CGLineCap, CGLineJoin,
    CGMutablePath,
};
use objc2_core_text::{
    kCTFontAttributeName, kCTForegroundColorFromContextAttributeName, CTFont, CTLine,
    CTLineTruncationType, CTTypesetter,
};
use std::collections::HashMap;
use std::ptr::{self, NonNull};
use std::rc::Rc;

// ---------------------------------------------------------------- text styles

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Weight {
    Regular,
    Semibold,
    Bold,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Align {
    Left,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    pub size: f32,
    pub weight: Weight,
    pub color: Color,
    pub align: Align,
    pub tabular: bool,
    pub wrap: bool,
    pub vcenter: bool,
    pub line: f32,
}

impl TextStyle {
    pub fn new(size: f32) -> Self {
        Self {
            size,
            weight: Weight::Regular,
            color: Color::rgba(0.96, 0.96, 0.96, 1.0),
            align: Align::Left,
            tabular: false,
            wrap: false,
            vcenter: true,
            line: 0.0,
        }
    }

    pub fn weight(mut self, w: Weight) -> Self {
        self.weight = w;
        self
    }

    pub fn color(mut self, c: Color) -> Self {
        self.color = c;
        self
    }
}

// ---------------------------------------------------------------- transforms

/// A 2D affine transform in row-vector layout matching Direct2D so animation math is identical.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matrix3x2 {
    pub m11: f32,
    pub m12: f32,
    pub m21: f32,
    pub m22: f32,
    pub m31: f32,
    pub m32: f32,
}

impl Matrix3x2 {
    pub const fn identity() -> Self {
        Self {
            m11: 1.0,
            m12: 0.0,
            m21: 0.0,
            m22: 1.0,
            m31: 0.0,
            m32: 0.0,
        }
    }

    pub fn to_cg(self) -> CGAffineTransform {
        CGAffineTransform {
            a: self.m11 as CGFloat,
            b: self.m12 as CGFloat,
            c: self.m21 as CGFloat,
            d: self.m22 as CGFloat,
            tx: self.m31 as CGFloat,
            ty: self.m32 as CGFloat,
        }
    }

    pub fn from_cg(t: CGAffineTransform) -> Self {
        Self {
            m11: t.a as f32,
            m12: t.b as f32,
            m21: t.c as f32,
            m22: t.d as f32,
            m31: t.tx as f32,
            m32: t.ty as f32,
        }
    }

    pub fn inverse(self) -> Self {
        let det = self.m11 * self.m22 - self.m12 * self.m21;
        if det.abs() < 1e-12 {
            return Self::identity();
        }
        let i = 1.0 / det;
        let m11 = self.m22 * i;
        let m12 = -self.m12 * i;
        let m21 = -self.m21 * i;
        let m22 = self.m11 * i;
        Self {
            m11,
            m12,
            m21,
            m22,
            m31: -(self.m31 * m11 + self.m32 * m21),
            m32: -(self.m31 * m12 + self.m32 * m22),
        }
    }
}

impl std::ops::Mul for Matrix3x2 {
    type Output = Matrix3x2;
    fn mul(self, b: Matrix3x2) -> Matrix3x2 {
        let a = self;
        Matrix3x2 {
            m11: a.m11 * b.m11 + a.m12 * b.m21,
            m12: a.m11 * b.m12 + a.m12 * b.m22,
            m21: a.m21 * b.m11 + a.m22 * b.m21,
            m22: a.m21 * b.m12 + a.m22 * b.m22,
            m31: a.m31 * b.m11 + a.m32 * b.m21 + b.m31,
            m32: a.m31 * b.m12 + a.m32 * b.m22 + b.m32,
        }
    }
}

/// Scale by `s` about point `p`.
pub fn scale_about(p: Point, s: f32) -> Matrix3x2 {
    Matrix3x2 {
        m11: s,
        m12: 0.0,
        m21: 0.0,
        m22: s,
        m31: p.x * (1.0 - s),
        m32: p.y * (1.0 - s),
    }
}

fn cgrect(r: Rect) -> CGRect {
    CGRect {
        origin: CGPoint {
            x: r.x as CGFloat,
            y: r.y as CGFloat,
        },
        size: CGSize {
            width: r.w.max(0.0) as CGFloat,
            height: r.h.max(0.0) as CGFloat,
        },
    }
}

// ---------------------------------------------------------------- Font & Layout keys

#[derive(Clone, PartialEq, Eq, Hash)]
struct FontKey {
    size: u32,
    weight: Weight,
    tabular: bool,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct LayoutKey {
    text: String,
    font: FontKey,
    wrap: bool,
    line: u32,
    w: u32,
}

struct TextLayout {
    lines: Vec<(CFRetained<CTLine>, f32)>,
    ascent: f32,
    leading: f32,
    line_h: f32,
    width: f32,
}

// ---------------------------------------------------------------- Gfx

pub struct Gfx {
    srgb: CFRetained<CGColorSpace>,
    fonts: HashMap<FontKey, CFRetained<CTFont>>,
    ellipsis: HashMap<FontKey, CFRetained<CTLine>>,
    layouts: HashMap<LayoutKey, (Rc<TextLayout>, u64)>,
    icon_paths: HashMap<Icon, Vec<(CFRetained<CGMutablePath>, bool)>>,
    frame_no: u64,
}

impl Gfx {
    pub fn new() -> Self {
        let srgb = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))
            .expect("the sRGB colour space");
        Self {
            srgb,
            fonts: HashMap::new(),
            ellipsis: HashMap::new(),
            layouts: HashMap::new(),
            icon_paths: HashMap::new(),
            frame_no: 0,
        }
    }

    pub fn measure_text(&mut self, text: &str, is_escalated: bool) -> (f32, f32) {
        let weight = if is_escalated {
            Weight::Bold
        } else {
            Weight::Semibold
        };
        let style = TextStyle::new(13.5).weight(weight);
        self.measure(text, &style, 1000.0)
    }

    pub fn measure(&mut self, s: &str, st: &TextStyle, max_w: f32) -> (f32, f32) {
        let mut st = *st;
        st.align = Align::Left;
        let lay = self.layout(s, &st, max_w.max(1.0));
        let lines = lay.lines.len().max(1) as f32;
        (lay.width.min(max_w.max(1.0)), lines * lay.line_h)
    }

    /// Begin drawing onto `ctx`.
    pub fn canvas<'a>(&'a mut self, ctx: &'a CGContext) -> Canvas<'a> {
        self.frame_no += 1;
        if self.frame_no.is_multiple_of(120) {
            let keep = self.frame_no.saturating_sub(240);
            self.layouts.retain(|_, v| v.1 >= keep);
        }
        CGContext::set_fill_color_space(Some(ctx), Some(&self.srgb));
        CGContext::set_stroke_color_space(Some(ctx), Some(&self.srgb));
        CGContext::set_should_antialias(Some(ctx), true);
        CGContext::set_should_smooth_fonts(Some(ctx), false);
        CGContext::set_interpolation_quality(Some(ctx), CGInterpolationQuality::High);
        CGContext::set_line_cap(Some(ctx), CGLineCap::Round);
        CGContext::set_line_join(Some(ctx), CGLineJoin::Round);
        Canvas {
            g: self,
            ctx,
            alpha: 1.0,
            stack: 0,
        }
    }

    fn font(&mut self, size: f32, weight: Weight, tabular: bool) -> CFRetained<CTFont> {
        let key = FontKey {
            size: size.to_bits(),
            weight,
            tabular,
        };
        if let Some(f) = self.fonts.get(&key) {
            return f.clone();
        }
        let f = system_font(size, weight, tabular);
        self.fonts.insert(key, f.clone());
        f
    }

    fn attributed(font: &CTFont, s: &str) -> Option<CFRetained<CFAttributedString>> {
        let keys: [&CFString; 2] = unsafe {
            [
                kCTFontAttributeName,
                kCTForegroundColorFromContextAttributeName,
            ]
        };
        let values: [&CFType; 2] = [font.as_ref(), CFBoolean::new(true).as_ref()];
        let attrs = CFDictionary::<CFString, CFType>::from_slices(&keys, &values);
        let text = CFString::from_str(s);
        unsafe { CFAttributedString::new(None, Some(&text), Some(attrs.as_opaque())) }
    }

    fn ellipsis_for(&mut self, key: &FontKey, font: &CTFont) -> Option<CFRetained<CTLine>> {
        if let Some(l) = self.ellipsis.get(key) {
            return Some(l.clone());
        }
        let attr = Self::attributed(font, "\u{2026}")?;
        let line = unsafe { CTLine::with_attributed_string(&attr) };
        self.ellipsis.insert(key.clone(), line.clone());
        Some(line)
    }

    fn layout(&mut self, s: &str, st: &TextStyle, w: f32) -> Rc<TextLayout> {
        let w = w.max(1.0);
        let fkey = FontKey {
            size: st.size.to_bits(),
            weight: st.weight,
            tabular: st.tabular,
        };
        let key = LayoutKey {
            text: s.to_string(),
            font: fkey.clone(),
            wrap: st.wrap,
            line: st.line.to_bits(),
            w: w.to_bits(),
        };
        let frame = self.frame_no;
        if let Some((l, used)) = self.layouts.get_mut(&key) {
            *used = frame;
            return l.clone();
        }
        let font = self.font(st.size, st.weight, st.tabular);
        let (ascent, descent, leading) = unsafe {
            (
                font.ascent() as f32,
                font.descent() as f32,
                font.leading() as f32,
            )
        };
        let line_h = if st.line > 0.0 {
            st.line * st.size
        } else {
            ascent + descent + leading
        };
        let mut lines = Vec::new();
        let mut width = 0.0f32;
        if let Some(attr) = Self::attributed(&font, s) {
            if !st.wrap {
                let line = unsafe { CTLine::with_attributed_string(&attr) };
                let full = unsafe {
                    line.typographic_bounds(ptr::null_mut(), ptr::null_mut(), ptr::null_mut())
                } as f32;
                let trailing = unsafe { line.trailing_whitespace_width() } as f32;
                if full > w + 0.5 {
                    let token = self.ellipsis_for(&fkey, &font);
                    let cut = unsafe {
                        line.truncated_line(w as f64, CTLineTruncationType::End, token.as_deref())
                    };
                    let line = cut.unwrap_or(line);
                    let lw = unsafe {
                        line.typographic_bounds(ptr::null_mut(), ptr::null_mut(), ptr::null_mut())
                    } as f32;
                    width = lw.min(w);
                    lines.push((line, width));
                } else {
                    width = full;
                    lines.push((line, full - trailing));
                }
            } else {
                let ts = unsafe { CTTypesetter::with_attributed_string(&attr) };
                let len = s.encode_utf16().count() as isize;
                let mut start = 0isize;
                while start < len {
                    let count = unsafe { ts.suggest_line_break(start, w as f64) };
                    if count <= 0 {
                        break;
                    }
                    let line = unsafe {
                        ts.line(CFRange {
                            location: start,
                            length: count,
                        })
                    };
                    let full = unsafe {
                        line.typographic_bounds(ptr::null_mut(), ptr::null_mut(), ptr::null_mut())
                    } as f32;
                    let trailing = unsafe { line.trailing_whitespace_width() } as f32;
                    width = width.max(full);
                    lines.push((line, full - trailing));
                    start += count;
                }
            }
        }
        let layout = Rc::new(TextLayout {
            lines,
            ascent,
            leading,
            line_h,
            width,
        });
        self.layouts.insert(key, (layout.clone(), frame));
        layout
    }

    fn icon_paths(&mut self, icon: Icon) -> Vec<(CFRetained<CGMutablePath>, bool)> {
        if let Some(p) = self.icon_paths.get(&icon) {
            return p.clone();
        }
        let mut out = Vec::new();
        for part in icons::parts(icon) {
            if let Ok(segs) = nopeek_core::svgpath::parse(part.d) {
                out.push((cg_path(&segs), part.fill));
            }
        }
        self.icon_paths.insert(icon, out.clone());
        out
    }
}

/// The system font (SF Pro) at `size`.
fn system_font(size: f32, weight: Weight, tabular: bool) -> CFRetained<CTFont> {
    let w = unsafe {
        match weight {
            Weight::Regular => NSFontWeightRegular,
            Weight::Semibold => NSFontWeightSemibold,
            Weight::Bold => NSFontWeightBold,
        }
    };
    let f: Retained<NSFont> = if tabular {
        NSFont::monospacedDigitSystemFontOfSize_weight(size as CGFloat, w)
    } else {
        NSFont::systemFontOfSize_weight(size as CGFloat, w)
    };
    unsafe { CFRetained::retain(NonNull::new_unchecked(Retained::as_ptr(&f) as *mut CTFont)) }
}

/// Build a Core Graphics path from parsed SVG segments.
pub fn cg_path(segs: &[Seg]) -> CFRetained<CGMutablePath> {
    let p = CGMutablePath::new();
    let mut open = false;
    let mut cur = (0.0f32, 0.0f32);
    let f = |v: f32| v as CGFloat;
    unsafe {
        for s in segs {
            match *s {
                Seg::MoveTo(x, y) => {
                    CGMutablePath::move_to_point(Some(&p), ptr::null(), f(x), f(y));
                    open = true;
                    cur = (x, y);
                }
                Seg::LineTo(x, y) => {
                    if !open {
                        CGMutablePath::move_to_point(Some(&p), ptr::null(), f(cur.0), f(cur.1));
                        open = true;
                    }
                    CGMutablePath::add_line_to_point(Some(&p), ptr::null(), f(x), f(y));
                    cur = (x, y);
                }
                Seg::CubicTo(x1, y1, x2, y2, x, y) => {
                    if !open {
                        CGMutablePath::move_to_point(Some(&p), ptr::null(), f(cur.0), f(cur.1));
                        open = true;
                    }
                    CGMutablePath::add_curve_to_point(
                        Some(&p),
                        ptr::null(),
                        f(x1),
                        f(y1),
                        f(x2),
                        f(y2),
                        f(x),
                        f(y),
                    );
                    cur = (x, y);
                }
                Seg::QuadTo(x1, y1, x, y) => {
                    if !open {
                        CGMutablePath::move_to_point(Some(&p), ptr::null(), f(cur.0), f(cur.1));
                        open = true;
                    }
                    CGMutablePath::add_quad_curve_to_point(
                        Some(&p),
                        ptr::null(),
                        f(x1),
                        f(y1),
                        f(x),
                        f(y),
                    );
                    cur = (x, y);
                }
                Seg::ArcTo(rx, ry, rot, large, sweep, x, y) => {
                    if !open {
                        CGMutablePath::move_to_point(Some(&p), ptr::null(), f(cur.0), f(cur.1));
                        open = true;
                    }
                    let params = nopeek_core::svgpath::ArcParams {
                        from: (cur.0, cur.1),
                        r: (rx, ry),
                        rot_deg: rot,
                        large,
                        sweep,
                        to: (x, y),
                    };
                    for c in nopeek_core::svgpath::arc_to_cubics(params) {
                        CGMutablePath::add_curve_to_point(
                            Some(&p),
                            ptr::null(),
                            f(c[0]),
                            f(c[1]),
                            f(c[2]),
                            f(c[3]),
                            f(c[4]),
                            f(c[5]),
                        );
                    }
                    cur = (x, y);
                }
                Seg::Close => {
                    if open {
                        CGMutablePath::close_subpath(Some(&p));
                        open = false;
                    }
                }
            }
        }
    }
    p
}

// ---------------------------------------------------------------- Canvas

pub struct Canvas<'a> {
    g: &'a mut Gfx,
    ctx: &'a CGContext,
    alpha: f32,
    stack: usize,
}

impl<'a> Canvas<'a> {
    fn cg(&self) -> Option<&CGContext> {
        Some(self.ctx)
    }

    fn components(&self, c: Color) -> [CGFloat; 4] {
        let c = c.mul_alpha(self.alpha);
        [
            c.r as CGFloat,
            c.g as CGFloat,
            c.b as CGFloat,
            c.a.clamp(0.0, 1.0) as CGFloat,
        ]
    }

    fn set_fill(&self, c: Color) {
        let k = self.components(c);
        unsafe { CGContext::set_fill_color(self.cg(), k.as_ptr()) };
    }

    fn set_stroke(&self, c: Color, width: f32) {
        let k = self.components(c);
        unsafe { CGContext::set_stroke_color(self.cg(), k.as_ptr()) };
        CGContext::set_line_width(self.cg(), width as CGFloat);
    }

    pub fn path(&mut self, segs: &[Seg], fill: Option<Color>, stroke: Option<(Color, f32)>) {
        let p = cg_path(segs);
        if let Some(f) = fill {
            self.set_fill(f);
            CGContext::begin_path(self.cg());
            CGContext::add_path(self.cg(), Some(&p));
            CGContext::fill_path(self.cg());
        }
        if let Some((c, w)) = stroke {
            self.set_stroke(c, w);
            CGContext::begin_path(self.cg());
            CGContext::add_path(self.cg(), Some(&p));
            CGContext::stroke_path(self.cg());
        }
    }

    /// Draw the Dynamic Island silhouette with concave shoulders.
    pub fn notch(
        &mut self,
        body: Rect,
        radius: f32,
        shoulder: f32,
        fill: Color,
        border: Option<Color>,
    ) {
        let segs = nopeek_core::svgpath::notch_segments(body, radius, shoulder);
        self.path(&segs, Some(fill), None);
        if let Some(b) = border {
            let s = shoulder.max(0.0);
            let r = radius.min(body.h).min(body.w / 2.0).max(0.0);
            let (x0, x1, y1) = (body.x, body.right(), body.bottom());
            let top = s.min(body.h);
            let border_segs = [
                Seg::MoveTo(x0, top),
                Seg::LineTo(x0, (y1 - r).max(top)),
                Seg::ArcTo(r, r, 0.0, false, false, x0 + r, y1),
                Seg::LineTo(x1 - r, y1),
                Seg::ArcTo(r, r, 0.0, false, false, x1, (y1 - r).max(top)),
                Seg::LineTo(x1, top),
            ];
            self.path(&border_segs, None, Some((b, 1.0)));
        }
    }

    pub fn text(&mut self, s: &str, r: Rect, st: &TextStyle) {
        if s.is_empty() || r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        let lay = self.g.layout(s, st, r.w);
        if lay.lines.is_empty() {
            return;
        }
        self.set_fill(st.color);
        let total = lay.lines.len() as f32 * lay.line_h;
        let mut y = if st.vcenter {
            r.y + (r.h - total) / 2.0
        } else {
            r.y
        };
        let base = if st.line > 0.0 {
            0.8 * lay.line_h
        } else {
            lay.leading / 2.0 + lay.ascent
        };
        CGContext::set_text_matrix(
            self.cg(),
            CGAffineTransform {
                a: 1.0,
                b: 0.0,
                c: 0.0,
                d: -1.0,
                tx: 0.0,
                ty: 0.0,
            },
        );
        for (line, _lw) in &lay.lines {
            let x = match st.align {
                Align::Left => r.x,
            };
            CGContext::set_text_position(self.cg(), x as CGFloat, (y + base) as CGFloat);
            unsafe { line.draw(self.ctx) };
            y += lay.line_h;
        }
    }

    pub fn icon(&mut self, icon: Icon, r: Rect, color: Color) {
        let paths = self.g.icon_paths(icon);
        if paths.is_empty() {
            return;
        }
        let s = r.w.min(r.h) / 24.0;
        let ox = r.x + (r.w - 24.0 * s) / 2.0;
        let oy = r.y + (r.h - 24.0 * s) / 2.0;
        CGContext::save_g_state(self.cg());
        CGContext::translate_ctm(self.cg(), ox as CGFloat, oy as CGFloat);
        CGContext::scale_ctm(self.cg(), s as CGFloat, s as CGFloat);
        self.set_fill(color);
        self.set_stroke(color, icons::STROKE);
        for (p, fill) in &paths {
            CGContext::begin_path(self.cg());
            CGContext::add_path(self.cg(), Some(p));
            if *fill {
                CGContext::fill_path(self.cg());
            } else {
                CGContext::stroke_path(self.cg());
            }
        }
        CGContext::restore_g_state(self.cg());
    }

    /// Draw a sub-tree inside a transparency layer scaled and faded for pop text animations.
    pub fn layered<R>(
        &mut self,
        bounds: Rect,
        alpha: f32,
        xf: Matrix3x2,
        f: impl FnOnce(&mut Canvas<'a>) -> R,
    ) -> R {
        let a = (alpha * self.alpha).clamp(0.0, 1.0);
        CGContext::save_g_state(self.cg());
        CGContext::concat_ctm(self.cg(), xf.to_cg());
        CGContext::set_alpha(self.cg(), a as CGFloat);
        unsafe { CGContext::begin_transparency_layer_with_rect(self.cg(), cgrect(bounds), None) };
        let outer_alpha = self.alpha;
        let depth = self.stack;
        self.alpha = 1.0;
        let r = f(self);
        while self.stack > depth {
            self.pop_state();
        }
        self.alpha = outer_alpha;
        CGContext::end_transparency_layer(self.cg());
        CGContext::restore_g_state(self.cg());
        r
    }

    fn pop_state(&mut self) {
        if self.stack == 0 {
            return;
        }
        let m = CGContext::ctm(self.cg());
        CGContext::restore_g_state(self.cg());
        self.stack -= 1;
        let now = Matrix3x2::from_cg(CGContext::ctm(self.cg()));
        let want = Matrix3x2::from_cg(m);
        if now != want {
            CGContext::concat_ctm(self.cg(), (want * now.inverse()).to_cg());
        }
    }
}

impl Drop for Canvas<'_> {
    fn drop(&mut self) {
        while self.stack > 0 {
            self.pop_state();
        }
    }
}

// ---------------------------------------------------------------- Bitmap (Menu Bar icon)

pub struct Bitmap {
    ctx: CFRetained<CGContext>,
}

impl Bitmap {
    pub fn new(w: usize, h: usize, scale: f32) -> Option<Self> {
        let srgb = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))?;
        let info = CGImageAlphaInfo::PremultipliedFirst.0 | CGImageByteOrderInfo::Order32Little.0;
        let ctx =
            unsafe { CGBitmapContextCreate(ptr::null_mut(), w, h, 8, w * 4, Some(&srgb), info) }?;
        CGContext::translate_ctm(Some(&ctx), 0.0, h as CGFloat);
        CGContext::scale_ctm(Some(&ctx), scale as CGFloat, -(scale as CGFloat));
        Some(Self { ctx })
    }

    pub fn ctx(&self) -> &CGContext {
        &self.ctx
    }

    pub fn image(&self) -> Option<CFRetained<CGImage>> {
        CGBitmapContextCreateImage(Some(&self.ctx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix3x2_identity() {
        let id = Matrix3x2::identity();
        assert_eq!(id.m11, 1.0);
        assert_eq!(id.m22, 1.0);
    }

    #[test]
    fn scale_about_point() {
        let p = Point::new(100.0, 50.0);
        let m = scale_about(p, 1.5);
        assert_eq!(m.m11, 1.5);
        assert_eq!(m.m22, 1.5);
        assert_eq!(m.m31, 100.0 * (1.0 - 1.5));
    }

    #[test]
    fn text_style_builder() {
        let st = TextStyle::new(14.0)
            .weight(Weight::Bold)
            .color(Color::rgba(1.0, 0.0, 0.0, 1.0));
        assert_eq!(st.size, 14.0);
        assert_eq!(st.weight, Weight::Bold);
        assert_eq!(st.align, Align::Left);
    }
}
