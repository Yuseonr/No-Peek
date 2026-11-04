//! SVG path parser (`M L H V C S Q T A Z`) and docked Dynamic Island silhouette generator.
//! Adapted from `oyenza-studio/crates/poni-core/src/svgpath.rs` and `poni/src/gfx.rs::notch_segments`.

use crate::geom::Rect;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    MoveTo(f32, f32),
    LineTo(f32, f32),
    CubicTo(f32, f32, f32, f32, f32, f32),
    QuadTo(f32, f32, f32, f32),
    /// rx, ry, x-axis rotation (degrees), large_arc, sweep (true = clockwise in y-down), end x, end y
    ArcTo(f32, f32, f32, bool, bool, f32, f32),
    Close,
}

#[derive(Debug, PartialEq)]
pub struct ParseError(pub String);

struct Lexer<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Lexer<'a> {
    fn skip_ws(&mut self) {
        while self.i < self.s.len()
            && (self.s[self.i].is_ascii_whitespace() || self.s[self.i] == b',')
        {
            self.i += 1;
        }
    }
    fn peek_cmd(&mut self) -> Option<u8> {
        self.skip_ws();
        self.s
            .get(self.i)
            .copied()
            .filter(|c| c.is_ascii_alphabetic())
    }
    fn at_number(&mut self) -> bool {
        self.skip_ws();
        matches!(self.s.get(self.i), Some(c) if c.is_ascii_digit() || *c == b'-' || *c == b'+' || *c == b'.')
    }
    fn number(&mut self) -> Result<f32, ParseError> {
        self.skip_ws();
        let start = self.i;
        if matches!(self.s.get(self.i), Some(b'-') | Some(b'+')) {
            self.i += 1;
        }
        let mut seen_dot = false;
        let mut seen_digit = false;
        while let Some(&c) = self.s.get(self.i) {
            if c.is_ascii_digit() {
                seen_digit = true;
                self.i += 1;
            } else if c == b'.' && !seen_dot {
                seen_dot = true;
                self.i += 1;
            } else {
                break;
            }
        }
        if matches!(self.s.get(self.i), Some(b'e') | Some(b'E')) {
            self.i += 1;
            if matches!(self.s.get(self.i), Some(b'-') | Some(b'+')) {
                self.i += 1;
            }
            while matches!(self.s.get(self.i), Some(c) if c.is_ascii_digit()) {
                self.i += 1;
            }
        }
        if !seen_digit {
            return Err(ParseError(format!("expected number at byte {start}")));
        }
        std::str::from_utf8(&self.s[start..self.i])
            .ok()
            .and_then(|t| t.parse::<f32>().ok())
            .ok_or_else(|| ParseError(format!("bad number at byte {start}")))
    }
    fn flag(&mut self) -> Result<bool, ParseError> {
        self.skip_ws();
        match self.s.get(self.i) {
            Some(b'0') => {
                self.i += 1;
                Ok(false)
            }
            Some(b'1') => {
                self.i += 1;
                Ok(true)
            }
            _ => Err(ParseError(format!("expected arc flag at byte {}", self.i))),
        }
    }
}

/// Parse SVG `d` path string into absolute segments.
pub fn parse(d: &str) -> Result<Vec<Seg>, ParseError> {
    let mut lx = Lexer {
        s: d.as_bytes(),
        i: 0,
    };
    let mut out = Vec::new();
    let (mut cx, mut cy) = (0.0f32, 0.0f32);
    let (mut sx, mut sy) = (0.0f32, 0.0f32);
    let mut last_ctrl: Option<(f32, f32, bool)> = None;
    let mut cmd: Option<u8> = None;

    loop {
        if let Some(c) = lx.peek_cmd() {
            lx.i += 1;
            cmd = Some(c);
        } else if !lx.at_number() {
            break;
        }
        let Some(c) = cmd else {
            return Err(ParseError("path must start with a command".into()));
        };
        let rel = c.is_ascii_lowercase();
        let (ox, oy) = if rel { (cx, cy) } else { (0.0, 0.0) };
        match c.to_ascii_uppercase() {
            b'M' => {
                let (x, y) = (lx.number()? + ox, lx.number()? + oy);
                out.push(Seg::MoveTo(x, y));
                cx = x;
                cy = y;
                sx = x;
                sy = y;
                last_ctrl = None;
                cmd = Some(if rel { b'l' } else { b'L' });
            }
            b'L' => {
                let (x, y) = (lx.number()? + ox, lx.number()? + oy);
                out.push(Seg::LineTo(x, y));
                cx = x;
                cy = y;
                last_ctrl = None;
            }
            b'H' => {
                let x = lx.number()? + ox;
                out.push(Seg::LineTo(x, cy));
                cx = x;
                last_ctrl = None;
            }
            b'V' => {
                let y = lx.number()? + oy;
                out.push(Seg::LineTo(cx, y));
                cy = y;
                last_ctrl = None;
            }
            b'C' => {
                let (x1, y1) = (lx.number()? + ox, lx.number()? + oy);
                let (x2, y2) = (lx.number()? + ox, lx.number()? + oy);
                let (x, y) = (lx.number()? + ox, lx.number()? + oy);
                out.push(Seg::CubicTo(x1, y1, x2, y2, x, y));
                last_ctrl = Some((x2, y2, true));
                cx = x;
                cy = y;
            }
            b'S' => {
                let (x1, y1) = match last_ctrl {
                    Some((lx_, ly_, true)) => (2.0 * cx - lx_, 2.0 * cy - ly_),
                    _ => (cx, cy),
                };
                let (x2, y2) = (lx.number()? + ox, lx.number()? + oy);
                let (x, y) = (lx.number()? + ox, lx.number()? + oy);
                out.push(Seg::CubicTo(x1, y1, x2, y2, x, y));
                last_ctrl = Some((x2, y2, true));
                cx = x;
                cy = y;
            }
            b'Q' => {
                let (x1, y1) = (lx.number()? + ox, lx.number()? + oy);
                let (x, y) = (lx.number()? + ox, lx.number()? + oy);
                out.push(Seg::QuadTo(x1, y1, x, y));
                last_ctrl = Some((x1, y1, false));
                cx = x;
                cy = y;
            }
            b'T' => {
                let (x1, y1) = match last_ctrl {
                    Some((lx_, ly_, false)) => (2.0 * cx - lx_, 2.0 * cy - ly_),
                    _ => (cx, cy),
                };
                let (x, y) = (lx.number()? + ox, lx.number()? + oy);
                out.push(Seg::QuadTo(x1, y1, x, y));
                last_ctrl = Some((x1, y1, false));
                cx = x;
                cy = y;
            }
            b'A' => {
                let rx = lx.number()?.abs();
                let ry = lx.number()?.abs();
                let rot = lx.number()?;
                let large = lx.flag()?;
                let sweep = lx.flag()?;
                let (x, y) = (lx.number()? + ox, lx.number()? + oy);
                out.push(Seg::ArcTo(rx, ry, rot, large, sweep, x, y));
                cx = x;
                cy = y;
                last_ctrl = None;
            }
            b'Z' => {
                out.push(Seg::Close);
                cx = sx;
                cy = sy;
                last_ctrl = None;
                cmd = None;
            }
            other => {
                return Err(ParseError(format!(
                    "unsupported command '{}'",
                    other as char
                )))
            }
        }
    }
    Ok(out)
}

/// Generate the docked top-edge Dynamic Island outline with concave top shoulders
/// and rounded bottom corners (`oyenza-studio/crates/poni/src/gfx.rs::notch_segments`).
pub fn notch_segments(body: Rect, radius: f32, shoulder: f32) -> Vec<Seg> {
    let s = shoulder.max(0.0);
    let r = radius.min(body.h).min(body.w / 2.0).max(0.0);
    let (x0, x1, y1) = (body.x, body.right(), body.bottom());
    let mut segs = vec![Seg::MoveTo(x0 - s, 0.0)];
    if s > 0.0 {
        segs.push(Seg::ArcTo(s, s, 0.0, false, true, x0, s.min(body.h)));
    }
    segs.push(Seg::LineTo(x0, (y1 - r).max(0.0)));
    segs.push(Seg::ArcTo(r, r, 0.0, false, false, x0 + r, y1));
    segs.push(Seg::LineTo(x1 - r, y1));
    segs.push(Seg::ArcTo(r, r, 0.0, false, false, x1, (y1 - r).max(0.0)));
    segs.push(Seg::LineTo(x1, s.min(body.h)));
    if s > 0.0 {
        segs.push(Seg::ArcTo(s, s, 0.0, false, true, x1 + s, 0.0));
    }
    segs.push(Seg::Close);
    segs
}

/// Parameters defining an elliptical SVG arc segment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArcParams {
    pub from: (f32, f32),
    pub r: (f32, f32),
    pub rot_deg: f32,
    pub large: bool,
    pub sweep: bool,
    pub to: (f32, f32),
}

/// Convert an SVG elliptical arc into one or more cubic Bézier curves `[p1x, p1y, p2x, p2y, p3x, p3y]`.
/// Used by Direct2D and CoreGraphics renderers to ensure 100% reproducible curves across all drivers.
pub fn arc_to_cubics(params: ArcParams) -> Vec<[f32; 6]> {
    let (x1, y1) = params.from;
    let (rx_in, ry_in) = params.r;
    let (x2, y2) = params.to;
    let (x1d, y1d, x2d, y2d) = (x1 as f64, y1 as f64, x2 as f64, y2 as f64);
    if (x1d - x2d).abs() < 1e-9 && (y1d - y2d).abs() < 1e-9 {
        return Vec::new();
    }
    let (mut rx, mut ry) = ((rx_in as f64).abs(), (ry_in as f64).abs());
    if rx < 1e-9 || ry < 1e-9 {
        return vec![[x1, y1, x2, y2, x2, y2]];
    }
    let phi = (params.rot_deg as f64).to_radians();
    let (sin_p, cos_p) = phi.sin_cos();
    let dx = (x1d - x2d) / 2.0;
    let dy = (y1d - y2d) / 2.0;
    let x1p = cos_p * dx + sin_p * dy;
    let y1p = -sin_p * dx + cos_p * dy;

    let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if lambda > 1.0 {
        let s = lambda.sqrt();
        rx *= s;
        ry *= s;
    }
    let num = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p;
    let den = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let mut coef = if den.abs() < 1e-12 {
        0.0
    } else {
        (num / den).max(0.0).sqrt()
    };
    if params.large == params.sweep {
        coef = -coef;
    }
    let cxp = coef * (rx * y1p / ry);
    let cyp = coef * (-ry * x1p / rx);
    let cx = cos_p * cxp - sin_p * cyp + (x1d + x2d) / 2.0;
    let cy = sin_p * cxp + cos_p * cyp + (y1d + y2d) / 2.0;

    let angle = |ux: f64, uy: f64, vx: f64, vy: f64| -> f64 {
        let dot = ux * vx + uy * vy;
        let len = (ux * ux + uy * uy).sqrt() * (vx * vx + vy * vy).sqrt();
        let a = (dot / len).clamp(-1.0, 1.0).acos();
        if ux * vy - uy * vx < 0.0 {
            -a
        } else {
            a
        }
    };
    let ux = (x1p - cxp) / rx;
    let uy = (y1p - cyp) / ry;
    let vx = (-x1p - cxp) / rx;
    let vy = (-y1p - cyp) / ry;
    let theta1 = angle(1.0, 0.0, ux, uy);
    let mut delta = angle(ux, uy, vx, vy);
    let tau = std::f64::consts::TAU;
    if !params.sweep && delta > 0.0 {
        delta -= tau;
    } else if params.sweep && delta < 0.0 {
        delta += tau;
    }

    let n = ((delta.abs() / std::f64::consts::FRAC_PI_2) - 1e-9)
        .ceil()
        .max(1.0) as usize;
    let step = delta / n as f64;
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    let map = |ex: f64, ey: f64| -> (f32, f32) {
        let x = rx * ex;
        let y = ry * ey;
        (
            (cos_p * x - sin_p * y + cx) as f32,
            (sin_p * x + cos_p * y + cy) as f32,
        )
    };
    let mut out = Vec::with_capacity(n);
    let mut t = theta1;
    for i in 0..n {
        let t2 = t + step;
        let (s1, c1) = t.sin_cos();
        let (s2, c2) = t2.sin_cos();
        let p1 = map(c1 - k * s1, s1 + k * c1);
        let p2 = map(c2 + k * s2, s2 - k * c2);
        let p3 = if i + 1 == n { (x2, y2) } else { map(c2, s2) };
        out.push([p1.0, p1.1, p2.0, p2.1, p3.0, p3.1]);
        t = t2;
    }
    out
}
