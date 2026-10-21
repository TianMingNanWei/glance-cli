use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Instant;

/// Seconds a peak marker holds before it starts to fall.
const PEAK_HOLD: f64 = 1.2;
/// Fraction of full scale a peak marker falls per second.
const PEAK_FALL: f64 = 0.25;
/// Higher is snappier. At 9 a bar covers about 90% of a jump in a quarter second.
const RESPONSE: f64 = 9.0;

/// Frame clock plus eased values keyed by name, so bars glide between samples
/// instead of jumping once a second.
pub struct Animator {
    pub enabled: bool,
    start: Instant,
    last: Instant,
    now: f64,
    dt: f64,
    values: RefCell<HashMap<String, Tween>>,
}

struct Tween {
    value: f64,
    peak: f64,
    held: f64,
    touched: f64,
}

impl Animator {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            enabled: true,
            start: now,
            last: now,
            now: 0.0,
            dt: 0.0,
            values: RefCell::new(HashMap::new()),
        }
    }

    /// Advances the clock. Call once per frame, before drawing.
    pub fn tick(&mut self) {
        let instant = Instant::now();
        self.dt = instant.duration_since(self.last).as_secs_f64().min(0.25);
        self.last = instant;
        self.now = instant.duration_since(self.start).as_secs_f64();
        let now = self.now;
        let mut values = self.values.borrow_mut();
        if values.len() > 4_000 {
            values.retain(|_, tween| now - tween.touched < 5.0);
        }
    }

    /// Jumps every value to its target on the next frame.
    #[cfg(test)]
    pub fn settle(&mut self) {
        self.dt = 10.0;
    }

    /// Seconds since launch. Drives shimmer, pulse, and spinner phases.
    pub fn now(&self) -> f64 {
        if self.enabled {
            self.now
        } else {
            0.0
        }
    }

    /// Returns the eased value and its decaying peak. Call at most once per key per frame.
    pub fn ease(&self, key: &str, target: f64) -> (f64, f64) {
        let target = if target.is_finite() { target } else { 0.0 };
        let mut values = self.values.borrow_mut();
        let tween = values.entry(key.to_string()).or_insert(Tween {
            value: 0.0,
            peak: 0.0,
            held: 0.0,
            touched: self.now,
        });
        tween.touched = self.now;
        if self.enabled {
            let blend = 1.0 - (-self.dt * RESPONSE).exp();
            tween.value += (target - tween.value) * blend;
            if (target - tween.value).abs() < 1e-4 {
                tween.value = target;
            }
        } else {
            tween.value = target;
        }
        if tween.value >= tween.peak {
            tween.peak = tween.value;
            tween.held = 0.0;
        } else {
            tween.held += self.dt;
            if tween.held > PEAK_HOLD {
                tween.peak = (tween.peak - self.dt * PEAK_FALL).max(tween.value);
            }
        }
        (tween.value, tween.peak)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eases_toward_target_and_holds_peak() {
        let mut animator = Animator::new();
        animator.dt = 0.05;
        let (first, _) = animator.ease("bar", 1.0);
        assert!(first > 0.0 && first < 1.0);
        for _ in 0..40 {
            animator.ease("bar", 1.0);
        }
        let (settled, peak) = animator.ease("bar", 1.0);
        assert!((settled - 1.0).abs() < 1e-3);
        assert!((peak - 1.0).abs() < 1e-3);
        let (dropping, held) = animator.ease("bar", 0.0);
        assert!(dropping < 1.0);
        assert!((held - 1.0).abs() < 1e-3, "peak holds while the bar falls");
    }

    #[test]
    fn disabled_snaps_to_target() {
        let mut animator = Animator::new();
        animator.enabled = false;
        assert_eq!(animator.ease("bar", 0.6).0, 0.6);
    }
}
