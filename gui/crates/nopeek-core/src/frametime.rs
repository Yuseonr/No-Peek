//! Frame timing and phase-locked VSync timeline.
//! Adapted from `oyenza-studio/crates/poni-core/src/frametime.rs`.

/// Phase-locked vblank timeline.
#[derive(Clone, Copy, Debug)]
pub struct VsyncTimeline {
    /// Current refresh period estimate (seconds).
    period: f64,
    /// Period reported by the display mode (seconds); bounds the estimate.
    nominal: f64,
    /// The last vblank we locked onto (seconds, monotonic).
    last: Option<f64>,
    /// Score of wake-ups that don't fit the rhythm (far off the grid, or
    /// extra ones inside a period); fitting ones pay it back.
    off_rhythm: u32,
}

/// Wake-ups further than this many periods apart restart the lock.
const RESYNC_PERIODS: f64 = 30.0;
/// How strongly one wake-up pulls the locked phase towards it.
const PHASE_GAIN: f64 = 0.1;
/// How strongly one wake-up corrects the period estimate.
const PERIOD_GAIN: f64 = 0.02;
/// An `off_rhythm` score this high means: re-read the display's rate.
pub const OFF_RHYTHM_LIMIT: u32 = 20;

impl VsyncTimeline {
    /// `period` in seconds (e.g. 1/144). Non-finite or silly values fall back to 60 Hz.
    pub fn new(period: f64) -> Self {
        let p = sane_period(period);
        Self {
            period: p,
            nominal: p,
            last: None,
            off_rhythm: 0,
        }
    }

    pub fn period(&self) -> f64 {
        self.period
    }

    /// The display mode changed (another monitor, a new refresh rate).
    pub fn set_nominal_period(&mut self, period: f64) {
        let p = sane_period(period);
        if (p - self.nominal).abs() > 1e-6 {
            self.nominal = p;
            self.period = p;
            self.last = None;
        }
        self.off_rhythm = 0;
    }

    /// Forget the phase (the clock was parked); the next wake-up starts afresh.
    pub fn reset(&mut self) {
        self.last = None;
        self.off_rhythm = 0;
    }

    /// How badly recent wake-ups miss the rhythm.
    pub fn off_rhythm(&self) -> u32 {
        self.off_rhythm
    }

    /// Feed the time a vblank wait returned. Returns the locked timestamp and
    /// how many refresh periods passed since the previous one.
    pub fn lock(&mut self, raw: f64) -> (f64, u32) {
        let Some(last) = self.last else {
            self.last = Some(raw);
            return (raw, 1);
        };
        let elapsed = raw - last;
        if !elapsed.is_finite() || elapsed < 0.0 || elapsed > RESYNC_PERIODS * self.period {
            self.last = Some(raw);
            return (raw, 1);
        }
        let n = (elapsed / self.period + 0.25).floor();
        if n < 1.0 {
            self.off_rhythm += 1;
            return (last, 0);
        }
        let predicted = last + n * self.period;
        let err = raw - predicted;
        self.off_rhythm = if err.abs() > 0.3 * self.period {
            self.off_rhythm + 1
        } else {
            self.off_rhythm.saturating_sub(1)
        };
        self.period =
            (self.period + PERIOD_GAIN * err / n).clamp(self.nominal * 0.97, self.nominal * 1.03);
        let locked = predicted + PHASE_GAIN * err;
        self.last = Some(locked);
        (locked, n as u32)
    }
}

pub fn sane_period(p: f64) -> f64 {
    if p.is_finite() && (1.0 / 500.0..=1.0 / 20.0).contains(&p) {
        p
    } else {
        1.0 / 60.0
    }
}

/// Refresh rate (Hz) to frame period, treating 0/1 as 60 Hz.
pub fn period_for_hz(hz: u32) -> f64 {
    if hz >= 20 {
        1.0 / hz as f64
    } else {
        1.0 / 60.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_locks_to_60hz_grid() {
        let mut tl = VsyncTimeline::new(1.0 / 60.0);
        let p = 1.0 / 60.0;
        let (t0, n0) = tl.lock(0.0);
        assert_eq!(n0, 1);
        assert_eq!(t0, 0.0);

        let (t1, n1) = tl.lock(p + 0.001);
        assert_eq!(n1, 1);
        assert!((t1 - p).abs() < 0.002);

        let (_t2, n2) = tl.lock(p * 3.0 + 0.002);
        assert_eq!(n2, 2);
    }
}
