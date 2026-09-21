//! Device-independent velocity estimation and deceleration for scroll owners.
//!
//! The caller owns gesture recognition, scroll targets, bounds, and animation
//! scheduling. This crate only converts timestamped motion into velocity and
//! integrates post-release movement, so any Scarlet client can share its feel.

#![no_std]

use core::time::Duration;

const SAMPLE_WINDOW_NS: u64 = 100_000_000;
const RELEASE_IDLE_NS: u64 = 80_000_000;
const MIN_SAMPLE_SPAN_NS: u64 = 8_000_000;
const MIN_FLING_SPEED: f32 = 80.0;
const MAX_FLING_SPEED: f32 = 4_500.0;
const STOP_SPEED: f32 = 12.0;
const DECAY_SECONDS: f32 = 0.34;
const MAX_FLING_SECONDS: f32 = 2.0;
const SAMPLE_CAPACITY: usize = 8;

#[derive(Clone, Copy, Default)]
struct Sample {
    time_ns: u64,
    x: i32,
    y: i32,
}

/// Keeps a short history of the physical contact that owns a scroll gesture.
pub struct ScrollVelocityTracker {
    samples: [Sample; SAMPLE_CAPACITY],
    len: usize,
}

impl ScrollVelocityTracker {
    /// Start tracking one physical contact at a monotonic timestamp.
    pub fn new(time_ns: u64, x: i32, y: i32) -> Self {
        let mut tracker = Self {
            samples: [Sample::default(); SAMPLE_CAPACITY],
            len: 0,
        };
        tracker.observe(time_ns, x, y);
        tracker
    }

    /// Record a contact position in logical pixels.
    pub fn observe(&mut self, time_ns: u64, x: i32, y: i32) {
        let sample = Sample { time_ns, x, y };
        if self.len > 0 {
            let last = self.samples[self.len - 1];
            if time_ns < last.time_ns {
                self.len = 0;
            } else if time_ns == last.time_ns {
                self.samples[self.len - 1] = sample;
                return;
            }
        }
        if self.len == SAMPLE_CAPACITY {
            self.samples.copy_within(1..SAMPLE_CAPACITY, 0);
            self.len -= 1;
        }
        self.samples[self.len] = sample;
        self.len += 1;
        while self.len > 1 && time_ns.saturating_sub(self.samples[0].time_ns) > SAMPLE_WINDOW_NS {
            self.samples.copy_within(1..self.len, 0);
            self.len -= 1;
        }
    }

    /// Return contact velocity in logical pixels per second at release.
    pub fn release_velocity(&mut self, time_ns: u64, x: i32, y: i32) -> Option<(f32, f32)> {
        let last = self.samples.get(self.len.checked_sub(1)?)?;
        if time_ns < last.time_ns || time_ns - last.time_ns > RELEASE_IDLE_NS {
            return None;
        }
        if last.x != x || last.y != y {
            self.observe(time_ns, x, y);
        }
        let latest = self.samples[self.len - 1];
        let oldest = self.samples[0];
        let elapsed_ns = latest.time_ns.saturating_sub(oldest.time_ns);
        if elapsed_ns < MIN_SAMPLE_SPAN_NS {
            return None;
        }
        let seconds = elapsed_ns as f32 / 1_000_000_000.0;
        let vx = (latest.x as f32 - oldest.x as f32) / seconds;
        let vy = (latest.y as f32 - oldest.y as f32) / seconds;
        let speed = libm::sqrtf(vx * vx + vy * vy);
        if !speed.is_finite() || speed < MIN_FLING_SPEED {
            return None;
        }
        let scale = (MAX_FLING_SPEED / speed).min(1.0);
        Some((vx * scale, vy * scale))
    }
}

/// Reusable scroll momentum integrator. Its deltas use the contact's direction.
pub struct ScrollMomentum {
    vx: f32,
    vy: f32,
    fractional_x: f32,
    fractional_y: f32,
    elapsed_seconds: f32,
}

impl ScrollMomentum {
    /// Begin deceleration from logical pixels per second.
    pub fn new(vx: f32, vy: f32) -> Self {
        Self {
            vx,
            vy,
            fractional_x: 0.0,
            fractional_y: 0.0,
            elapsed_seconds: 0.0,
        }
    }

    /// Integrate one frame and report a whole-pixel delta plus whether motion remains.
    pub fn advance(&mut self, elapsed: Duration) -> (i32, i32, bool) {
        let seconds = elapsed
            .as_secs_f32()
            .min(MAX_FLING_SECONDS - self.elapsed_seconds);
        if seconds <= 0.0 {
            return (0, 0, self.is_active());
        }
        self.elapsed_seconds += seconds;
        let decay = libm::expf(-seconds / DECAY_SECONDS);
        let distance = DECAY_SECONDS * (1.0 - decay);
        self.fractional_x += self.vx * distance;
        self.fractional_y += self.vy * distance;
        self.vx *= decay;
        self.vy *= decay;
        let dx = libm::truncf(self.fractional_x) as i32;
        let dy = libm::truncf(self.fractional_y) as i32;
        self.fractional_x -= dx as f32;
        self.fractional_y -= dy as f32;
        (dx, dy, self.is_active())
    }

    fn is_active(&self) -> bool {
        self.elapsed_seconds < MAX_FLING_SECONDS
            && libm::sqrtf(self.vx * self.vx + self.vy * self.vy) >= STOP_SPEED
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pause_before_release_does_not_fling() {
        let mut tracker = ScrollVelocityTracker::new(1, 0, 0);
        tracker.observe(21_000_001, 0, -80);
        assert!(tracker.release_velocity(121_000_001, 0, -80).is_none());
    }

    #[test]
    fn deceleration_is_stable_across_frame_splits() {
        let mut one = ScrollMomentum::new(1_200.0, -300.0);
        let mut split = ScrollMomentum::new(1_200.0, -300.0);
        let (whole_x, whole_y, _) = one.advance(Duration::from_millis(32));
        let (first_x, first_y, _) = split.advance(Duration::from_millis(16));
        let (second_x, second_y, _) = split.advance(Duration::from_millis(16));
        assert!((whole_x - first_x - second_x).abs() <= 1);
        assert!((whole_y - first_y - second_y).abs() <= 1);
    }
}
