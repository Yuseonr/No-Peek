//! Camera notch detection and geometry adaptation for macOS (MacBook notch).
//! Adapted from `oyenza-studio/crates/poni/src/mac/notch.rs` and `window.rs`.

use objc2::MainThreadMarker;
use objc2_app_kit::NSScreen;

/// The physical camera notch as macOS reports it, in DIPs (points).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Notch {
    pub w: f32,
    pub h: f32,
}

/// Screen geometry and camera notch characteristics in AppKit global coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenInfo {
    /// The screen frame origin x (points).
    pub x: f64,
    /// The screen frame origin y (points, AppKit y-up).
    pub y: f64,
    /// Screen width (points).
    pub w: f64,
    /// Screen height (points).
    pub h: f64,
    /// Backing scale factor (2.0 for Retina displays).
    pub scale: f32,
    /// Physical camera notch if present on this display.
    pub notch: Option<Notch>,
    /// Notch or screen horizontal center offset from screen left edge.
    pub cx: f64,
}

impl ScreenInfo {
    /// Query display metrics from an NSScreen instance.
    pub fn from_screen(s: &NSScreen) -> Self {
        let f = s.frame();
        let inset = s.safeAreaInsets().top;
        let (l, r) = (s.auxiliaryTopLeftArea(), s.auxiliaryTopRightArea());
        let notch_w = f.size.width - l.size.width - r.size.width;
        let notch =
            (inset > 0.0 && notch_w > 40.0 && notch_w < f.size.width / 2.0).then_some(Notch {
                w: notch_w as f32,
                h: inset as f32,
            });
        let cx = if notch.is_some() {
            l.size.width + notch_w / 2.0
        } else {
            f.size.width / 2.0
        };
        Self {
            x: f.origin.x,
            y: f.origin.y,
            w: f.size.width,
            h: f.size.height,
            scale: s.backingScaleFactor() as f32,
            notch,
            cx,
        }
    }
}

/// Select the active screen: prefer the built-in screen with a physical notch,
/// or fall back to the primary display.
pub fn pick_screen(mtm: MainThreadMarker) -> Option<ScreenInfo> {
    let screens = NSScreen::screens(mtm);
    let mut best: Option<ScreenInfo> = None;
    for s in screens.iter() {
        let info = ScreenInfo::from_screen(&s);
        if info.notch.is_some() {
            return Some(info);
        }
        if best.is_none() {
            best = Some(info);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macbook_notch_struct() {
        let n = Notch { w: 185.0, h: 32.0 };
        assert_eq!(n.w, 185.0);
        assert_eq!(n.h, 32.0);
    }
}
