//! Monotonic clock for animations and VBlank frame timing.
//! Adapted from `oyenza-studio/crates/poni/src/clock.rs`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

static START: OnceLock<Instant> = OnceLock::new();
static HELD: AtomicU64 = AtomicU64::new(u64::MAX);

/// Monotonic seconds since the process started.
pub fn mono() -> f64 {
    let held = HELD.load(Ordering::Relaxed);
    if held != u64::MAX {
        return f64::from_bits(held);
    }
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

/// Hold `mono()` at `t` (for deterministic tests) or reset with `None`.
#[cfg(test)]
pub fn hold(t: Option<f64>) {
    HELD.store(t.map_or(u64::MAX, f64::to_bits), Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_clock_increases_monotonically() {
        let t1 = mono();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let t2 = mono();
        assert!(t2 >= t1);
    }

    #[test]
    fn hold_locks_and_releases_mono_time() {
        hold(Some(123.456));
        assert_eq!(mono(), 123.456);
        hold(None);
        assert_ne!(mono(), 123.456);
    }
}
