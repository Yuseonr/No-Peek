//! Dynamic Island geometry, spring morphing, and pop-text animation state machine.
//! Adapted from `oyenza-studio/crates/poni/src/island.rs` and `poni/src/pill/notice.rs`.

use crate::config::GuiConfig;
use crate::geom::Rect;
use crate::icons::Icon;
use crate::ipc::RiskLevel;
use crate::spring::Spring;

/// Concave flare where the docked island meets the top screen edge (DIPs).
pub const SHOULDER: f32 = 9.0;
/// Extra canvas padding around the island so spring overshoot never clips.
pub const CANVAS_MARGIN: f32 = 16.0;
/// Icon size inside the alert island (DIPs).
pub const ICON_SIZE: f32 = 18.0;
/// Gap between the SVG icon and the alert text (DIPs).
pub const ICON_TEXT_GAP: f32 = 10.0;

/// Spring tunings `(stiffness, damping)` for docked island outline morphing.
/// Growing stretches horizontally first and drops downward smoothly (`zeta ~ 0.74`).
/// Shrinking is near critically damped (`zeta ~ 0.92`) so it never squashes below idle thickness.
const GROW_W: (f32, f32) = (420.0, 30.0);
const GROW_H: (f32, f32) = (300.0, 26.0);
const SHRINK_W: (f32, f32) = (330.0, 33.0);
const SHRINK_H: (f32, f32) = (420.0, 38.0);
const RADIUS: (f32, f32) = (420.0, 41.0);
/// Content opacity spring (critically damped, tau ~ 38ms).
const CONTENT_ALPHA: (f32, f32) = (700.0, 52.9);
/// Pop-text scale spring (underdamped elastic pop from 0.85 -> 1.0).
const CONTENT_SCALE: (f32, f32) = (520.0, 32.0);
/// Delay before pop-text starts appearing on expansion (seconds).
const POP_DELAY_GROW: f64 = 0.03;
/// Initial scale when text begins popping in.
const POP_START_SCALE: f32 = 0.85;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geom {
    pub w: f32,
    pub h: f32,
    pub r: f32,
}

impl Geom {
    pub const fn new(w: f32, h: f32, r: f32) -> Self {
        Self { w, h, r }
    }
}

/// Calculate target island width that fits the measured text width plus icon and margins,
/// rounded up to an even DIP so the centered island's left/right edges land on whole DIPs.
pub fn fit_alert_width(measured_text_w: f32, cfg: &GuiConfig) -> f32 {
    let raw = cfg.alert_margin_x * 2.0 + ICON_SIZE + ICON_TEXT_GAP + measured_text_w.ceil();
    let clamped = raw.clamp(cfg.alert_min_width, cfg.alert_max_width);
    ((clamped / 2.0).ceil() * 2.0).min(cfg.alert_max_width)
}

pub struct Island {
    w: Spring,
    h: Spring,
    r: Spring,
    content_alpha: Spring,
    content_scale: Spring,
    pop_in_at: Option<f64>,
    target: Geom,
    pub level: RiskLevel,
    pub text: String,
    pub icon: Icon,
}

impl Island {
    pub fn new(cfg: &GuiConfig) -> Self {
        let idle = Geom::new(cfg.idle_width, cfg.idle_height, cfg.idle_height / 2.0);
        Self {
            w: Spring::new(idle.w, GROW_W.0, GROW_W.1),
            h: Spring::new(idle.h, GROW_H.0, GROW_H.1),
            r: Spring::new(idle.r, RADIUS.0, RADIUS.1),
            content_alpha: Spring::new(0.0, CONTENT_ALPHA.0, CONTENT_ALPHA.1),
            content_scale: Spring::new(1.0, CONTENT_SCALE.0, CONTENT_SCALE.1),
            pop_in_at: None,
            target: idle,
            level: RiskLevel::Monitoring,
            text: String::new(),
            icon: Icon::EyeWarning,
        }
    }

    /// Update the island's alert state and target geometry.
    pub fn set_state(
        &mut self,
        level: RiskLevel,
        text: String,
        measured_text_w: f32,
        now: f64,
        cfg: &GuiConfig,
    ) {
        let was_active = self.level.is_alert_active();
        let is_active = level.is_alert_active();
        let text_changed = self.text != text || self.level != level;

        self.level = level;
        if is_active {
            self.text = text;
            self.icon = if level == RiskLevel::Escalated {
                Icon::ShieldAlert
            } else {
                Icon::EyeWarning
            };
            let target_w = fit_alert_width(measured_text_w, cfg);
            let target_h = cfg.alert_height;
            let g = Geom::new(target_w, target_h, (target_h * 0.45).min(22.0));
            self.set_target(g, now, !was_active || text_changed);
        } else {
            let idle = Geom::new(cfg.idle_width, cfg.idle_height, cfg.idle_height / 2.0);
            self.set_target(idle, now, false);
            self.pop_in_at = None;
            self.content_alpha.set_target(0.0);
            self.content_scale.set_target(POP_START_SCALE);
        }
    }

    pub fn set_target(&mut self, g: Geom, now: f64, trigger_pop: bool) {
        self.target = g;
        let growing = g.w * g.h >= self.w.value * self.h.value;
        let (tw, th) = if growing { (GROW_W, GROW_H) } else { (SHRINK_W, SHRINK_H) };
        self.w.retune(tw.0, tw.1);
        self.h.retune(th.0, th.1);
        self.r.retune(RADIUS.0, RADIUS.1);
        self.w.set_target(g.w);
        self.h.set_target(g.h);
        self.r.set_target(g.r);

        if trigger_pop {
            self.content_alpha.snap(0.0);
            self.content_scale.snap(POP_START_SCALE);
            self.pop_in_at = Some(now + POP_DELAY_GROW);
        }
    }

    pub fn step(&mut self, dt: f32, now: f64) {
        self.w.step(dt);
        self.h.step(dt);
        self.r.step(dt);
        if let Some(t) = self.pop_in_at {
            if now >= t {
                self.content_alpha.set_target(1.0);
                self.content_scale.set_target(1.0);
                self.pop_in_at = None;
            }
        }
        self.content_alpha.step(dt);
        self.content_scale.step(dt);
    }

    pub fn settled(&self) -> bool {
        self.pop_in_at.is_none()
            && self.w.is_settled()
            && self.h.is_settled()
            && self.r.is_settled()
            && self.content_alpha.is_settled_eps(0.002)
            && self.content_scale.is_settled_eps(0.002)
    }

    pub fn current(&self) -> Geom {
        let w = self.w.value.max(20.0);
        let h = self.h.value.max(4.0);
        let r = self.r.value.clamp(0.0, h.min(w / 2.0));
        Geom { w, h, r }
    }

    pub fn content_alpha(&self) -> f32 {
        self.content_alpha.value.clamp(0.0, 1.0)
    }

    pub fn content_scale(&self) -> f32 {
        self.content_scale.value.clamp(0.5, 1.25)
    }

    /// The island body rectangle docked at top center `cx` (`y = 0.0`).
    pub fn body_rect_at(&self, cx: f32) -> Rect {
        let g = self.current();
        Rect::new(cx - g.w / 2.0, 0.0, g.w, g.h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_to_alert_expands_and_pops_text() {
        let cfg = GuiConfig::default();
        let mut island = Island::new(&cfg);
        assert!(island.settled());
        assert_eq!(island.current().h, cfg.idle_height);

        island.set_state(
            RiskLevel::Alert,
            "Pengintip Terdeteksi (Kiri)".into(),
            185.0,
            0.0,
            &cfg,
        );
        assert!(!island.settled());
        assert_eq!(island.content_alpha(), 0.0);
        assert_eq!(island.content_scale(), POP_START_SCALE);

        let mut t = 0.0;
        while !island.settled() && t < 2.0 {
            island.step(1.0 / 60.0, t);
            t += 1.0 / 60.0;
        }
        assert!(island.settled());
        assert_eq!(island.current().h, cfg.alert_height);
        assert!(island.current().w >= cfg.alert_min_width);
        assert_eq!(island.content_alpha(), 1.0);
        assert!((island.content_scale() - 1.0).abs() < 0.01);
    }
}
