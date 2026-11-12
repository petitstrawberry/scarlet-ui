//! Deterministic animation timing and interpolation, with lazy transition storage.
//!
//! Time advances only through caller-supplied [`Duration`] values. No clock,
//! executor, or platform integration is required.

use core::time::Duration;

use crate::color::Color;
use crate::geometry::{Point, Size};

/// An animation described by its duration and timing curve.
///
/// Animations run once unless [`repeat_forever`](Self::repeat_forever) is used.
/// A zero duration always means an immediate change. The default is a finite
/// 300 ms [`ease_in_out`](Self::ease_in_out) animation.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Animation {
    duration: Duration,
    curve: Curve,
    repeat: Repeat,
}

#[derive(Copy, Clone, Debug, PartialEq)]
enum Repeat {
    Never,
    Forever { autoreverses: bool },
}

#[derive(Copy, Clone, Debug, PartialEq)]
enum Curve {
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
    Spring { damping_ratio: f32 },
}

impl Animation {
    /// Interpolate at a constant rate.
    pub const fn linear(duration: Duration) -> Self {
        Self {
            duration,
            curve: Curve::Linear,
            repeat: Repeat::Never,
        }
    }

    /// Accelerate from rest using quadratic easing (`t * t`).
    pub const fn ease_in(duration: Duration) -> Self {
        Self {
            duration,
            curve: Curve::EaseIn,
            repeat: Repeat::Never,
        }
    }

    /// Decelerate to rest using quadratic easing (`1 - (1 - t)^2`).
    pub const fn ease_out(duration: Duration) -> Self {
        Self {
            duration,
            curve: Curve::EaseOut,
            repeat: Repeat::Never,
        }
    }

    /// Accelerate, then decelerate, using symmetric piecewise quadratic easing.
    pub const fn ease_in_out(duration: Duration) -> Self {
        Self {
            duration,
            curve: Curve::EaseInOut,
            repeat: Repeat::Never,
        }
    }

    /// A bounded spring curve that settles exactly at `duration`.
    ///
    /// The underlying unit spring starts at rest with a natural angular
    /// frequency of 12 radians per duration. Ratios below 1 are underdamped,
    /// 1 is critically damped, and ratios above 1 are overdamped; 0 is allowed.
    /// Progress is bounded to `[0, 2]`, permitting overshoot above the target.
    ///
    /// To give even an undamped spring a finite deadline, its remaining
    /// displacement is multiplied by `(1 - t * t)^2`, with normalized time
    /// `t`. This window brings both displacement and velocity to zero at the
    /// deadline; it deliberately shapes the physical spring response. The
    /// exact target is assigned at completion, independent of frame spacing.
    /// Retargeting starts a fresh curve from the displayed value and does not
    /// preserve velocity.
    ///
    /// # Panics
    ///
    /// Panics if `damping_ratio` is negative, NaN, or infinite, including when
    /// `duration` is zero.
    pub fn spring(duration: Duration, damping_ratio: f32) -> Self {
        assert!(
            damping_ratio.is_finite() && damping_ratio >= 0.0,
            "spring damping ratio must be finite and nonnegative"
        );
        Self {
            duration,
            curve: Curve::Spring { damping_ratio },
            repeat: Repeat::Never,
        }
    }

    /// Repeat this animation indefinitely, optionally reversing every other leg.
    ///
    /// Without autoreversing, each exact duration boundary wraps to the starting
    /// value. With autoreversing, odd boundaries reach the target and even
    /// boundaries return to the start. Each leg takes the original duration;
    /// the return leg retraces the timing curve backward, including overshoot.
    /// A zero duration still snaps immediately and never remains active.
    ///
    /// Repeats reuse their transition storage. Call [`AnimatedValue::finish`]
    /// to stop repeating and assign the exact target.
    pub const fn repeat_forever(mut self, autoreverses: bool) -> Self {
        self.repeat = Repeat::Forever { autoreverses };
        self
    }

    fn progress(self, elapsed: Duration) -> f32 {
        if elapsed >= self.duration {
            return 1.0;
        }
        if elapsed.is_zero() {
            return 0.0;
        }

        let t = elapsed.as_secs_f64() / self.duration.as_secs_f64();
        let remaining = 1.0 - t;
        let progress = match self.curve {
            Curve::Linear => t,
            Curve::EaseIn => t * t,
            Curve::EaseOut => 1.0 - remaining * remaining,
            Curve::EaseInOut if t < 0.5 => 2.0 * t * t,
            Curve::EaseInOut => 1.0 - 2.0 * remaining * remaining,
            Curve::Spring { damping_ratio } => spring_progress(t, damping_ratio),
        };
        progress as f32
    }
}

impl Default for Animation {
    fn default() -> Self {
        Self::ease_in_out(Duration::from_millis(300))
    }
}

fn spring_progress(t: f64, damping_ratio: f32) -> f64 {
    // f64 intermediates keep all finite f32 damping ratios safe to square and
    // improve accuracy on either side of critical damping.
    let damping = f64::from(damping_ratio);
    let time = 12.0 * t;
    let displacement = if damping < 1.0 {
        let frequency = libm::sqrt(1.0 - damping * damping);
        libm::exp(-damping * time)
            * (libm::cos(frequency * time) + damping / frequency * libm::sin(frequency * time))
    } else if damping == 1.0 {
        libm::exp(-time) * (1.0 + time)
    } else {
        let root = libm::sqrt(damping * damping - 1.0);
        let fast = damping + root;
        // 1 / (damping + root) equals damping - root without cancellation.
        let slow = 1.0 / fast;
        // Negative exponentials avoid overflowing the equivalent cosh/sinh
        // expression for heavily overdamped springs.
        (fast * libm::exp(-slow * time) - slow * libm::exp(-fast * time)) / (2.0 * root)
    };
    let window = 1.0 - t * t;
    (1.0 - displacement * window * window).clamp(0.0, 2.0)
}

/// A value that can interpolate toward another value of the same type.
pub trait Animatable: Clone + PartialEq {
    /// Interpolate from `self` at progress 0 to `target` at progress 1.
    ///
    /// Progress is finite when supplied by this module, but can exceed 1 for
    /// springs. Implementations should support extrapolation or enforce their
    /// value's valid range. Scalars, arrays, points, and sizes extrapolate;
    /// colors clamp their interpolated channels to `[0, 1]`.
    fn interpolate(&self, target: &Self, progress: f32) -> Self;
}

impl Animatable for f32 {
    fn interpolate(&self, target: &Self, progress: f32) -> Self {
        // Wider arithmetic avoids overflowing the difference of finite f32s.
        f64::from(*self).interpolate(&f64::from(*target), progress) as f32
    }
}

impl Animatable for f64 {
    fn interpolate(&self, target: &Self, progress: f32) -> Self {
        if progress == 0.0 {
            return *self;
        }
        if progress == 1.0 {
            return *target;
        }
        if self == target {
            return *self;
        }
        let progress = f64::from(progress);
        // A weighted sum avoids overflowing target - self for opposite signs.
        *self * (1.0 - progress) + *target * progress
    }
}

impl<const N: usize> Animatable for [f32; N] {
    fn interpolate(&self, target: &Self, progress: f32) -> Self {
        core::array::from_fn(|index| self[index].interpolate(&target[index], progress))
    }
}

impl Animatable for Point {
    fn interpolate(&self, target: &Self, progress: f32) -> Self {
        Self {
            x: self.x.interpolate(&target.x, progress),
            y: self.y.interpolate(&target.y, progress),
        }
    }
}

impl Animatable for Size {
    fn interpolate(&self, target: &Self, progress: f32) -> Self {
        Self {
            width: self.width.interpolate(&target.width, progress),
            height: self.height.interpolate(&target.height, progress),
        }
    }
}

impl Animatable for Color {
    /// Interpolate stored RGBA channels independently, without gamma conversion
    /// or alpha premultiplication, then clamp each channel to `[0, 1]`.
    fn interpolate(&self, target: &Self, progress: f32) -> Self {
        Self {
            r: self.r.interpolate(&target.r, progress).clamp(0.0, 1.0),
            g: self.g.interpolate(&target.g, progress).clamp(0.0, 1.0),
            b: self.b.interpolate(&target.b, progress).clamp(0.0, 1.0),
            a: self.a.interpolate(&target.a, progress).clamp(0.0, 1.0),
        }
    }
}

/// A displayed value and an optional transition toward its latest target.
///
/// Callers supply elapsed time deltas and use [`Self::is_active`] to decide
/// whether further frames are needed. Retargeting preserves position, but
/// starts a fresh timing curve without carrying velocity forward.
/// Static values allocate no transition storage; active values reuse a single
/// allocation across repeats and retargets.
pub struct AnimatedValue<T: Animatable> {
    value: T,
    transition: Option<alloc::boxed::Box<Transition<T>>>,
}

struct Transition<T> {
    from: T,
    target: T,
    animation: Animation,
    elapsed: Duration,
    reversing: bool,
}

impl<T: Animatable> AnimatedValue<T> {
    /// Create an inactive value with no allocated transition storage.
    pub fn new(value: T) -> Self {
        Self {
            value,
            transition: None,
        }
    }

    /// Return a clone of the currently displayed value.
    pub fn value(&self) -> T {
        self.value.clone()
    }

    /// Return the requested target, or the displayed value while inactive.
    ///
    /// The target stays unchanged during reverse legs and repeat boundaries.
    pub fn target(&self) -> &T {
        self.transition.as_ref().map_or(&self.value, |t| &t.target)
    }

    /// Whether an animation needs further time updates, including repeat loops.
    pub fn is_active(&self) -> bool {
        self.transition.is_some()
    }

    /// Set a target, returning whether the requested target changed.
    ///
    /// Equal targets preserve in-flight motion across duplicate rebuilds.
    /// This also preserves the original animation, even if `animation` is
    /// `None` or has zero duration; use [`finish`](Self::finish) to stop it.
    /// A new target starts at the displayed value. Static values allocate no
    /// transition storage; active retargets reuse their existing allocation.
    /// A new target snaps immediately when `animation` is `None` or has zero
    /// duration. Targeting the displayed value also stops an active transition.
    pub fn retarget(&mut self, target: T, animation: Option<Animation>) -> bool {
        if *self.target() == target {
            return false;
        }
        match animation {
            Some(animation) if !animation.duration.is_zero() && self.value != target => {
                let next = Transition {
                    from: self.value.clone(),
                    target,
                    animation,
                    elapsed: Duration::ZERO,
                    reversing: false,
                };
                if let Some(transition) = self.transition.as_mut() {
                    **transition = next;
                } else {
                    self.transition = Some(alloc::boxed::Box::new(next));
                }
            }
            _ => {
                self.transition = None;
                self.value = target;
            }
        }
        true
    }

    /// Advance by a caller-supplied delta without reading a clock.
    ///
    /// Returns whether the displayed value changed or a finite animation
    /// completed. An unchanged sample can still be active, especially after
    /// complete repeat cycles; use [`is_active`](Self::is_active) to schedule
    /// further frames. A zero delta does nothing.
    ///
    /// Repeat phases use exact integer nanoseconds, so splitting elapsed time
    /// across calls preserves timing even when the total exceeds [`Duration::MAX`].
    /// Large deltas skip whole cycles without looping or allocating transitions.
    pub fn advance(&mut self, delta: Duration) -> bool {
        let Some(transition) = self.transition.as_mut() else {
            return false;
        };
        if delta.is_zero() {
            return false;
        }
        match transition.animation.repeat {
            Repeat::Never => {
                transition.elapsed = transition
                    .elapsed
                    .saturating_add(delta)
                    .min(transition.animation.duration);
                if transition.elapsed >= transition.animation.duration {
                    return self.finish();
                }
            }
            Repeat::Forever { autoreverses } => {
                let duration_nanos = transition.animation.duration.as_nanos();
                // Two Duration values sum to fewer than 2^95 nanoseconds, so
                // u128 preserves both the remainder and leg parity without
                // saturating or overflowing, even for Duration::MAX deltas.
                let elapsed_nanos = transition.elapsed.as_nanos() + delta.as_nanos();
                let legs = elapsed_nanos / duration_nanos;
                let remainder = elapsed_nanos % duration_nanos;
                // The remainder is below duration, so both casts fit.
                transition.elapsed = Duration::new(
                    (remainder / 1_000_000_000) as u64,
                    (remainder % 1_000_000_000) as u32,
                );
                if autoreverses && legs % 2 != 0 {
                    transition.reversing = !transition.reversing;
                }
            }
        }
        let elapsed = if transition.reversing {
            transition.animation.duration - transition.elapsed
        } else {
            transition.elapsed
        };
        let value = if elapsed.is_zero() {
            transition.from.clone()
        } else if elapsed == transition.animation.duration {
            transition.target.clone()
        } else {
            transition
                .from
                .interpolate(&transition.target, transition.animation.progress(elapsed))
        };
        let changed = self.value != value;
        self.value = value;
        changed
    }

    /// Stop any animation, including a repeat, and assign its exact target.
    ///
    /// Returns `true` if a transition was stopped, even if its target was already
    /// displayed. Returns `false` when inactive; subsequent advances do nothing.
    pub fn finish(&mut self) -> bool {
        let Some(transition) = self.transition.take() else {
            return false;
        };
        self.value = transition.target;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::AnimatedValue as Animated;
    use super::*;

    const SECOND: Duration = Duration::from_secs(1);

    fn animations(duration: Duration) -> [Animation; 8] {
        [
            Animation::linear(duration),
            Animation::ease_in(duration),
            Animation::ease_out(duration),
            Animation::ease_in_out(duration),
            Animation::spring(duration, 0.0),
            Animation::spring(duration, 0.5),
            Animation::spring(duration, 1.0),
            Animation::spring(duration, 2.0),
        ]
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 0.00001,
            "{actual} != {expected}"
        );
    }

    #[test]
    fn default_is_a_300_ms_ease_in_out() {
        assert_eq!(
            Animation::default(),
            Animation::ease_in_out(Duration::from_millis(300))
        );
    }

    #[test]
    fn scalar_interpolation_preserves_endpoints_and_supports_extrapolation() {
        assert_eq!(2.0_f32.interpolate(&10.0, 0.25), 4.0);
        assert_eq!(2.0_f64.interpolate(&10.0, 0.25), 4.0);
        assert_eq!(1.0_f32.interpolate(&3.0, 1.5), 4.0);
        assert_eq!(1.0_f64.interpolate(&3.0, -0.5), 0.0);
        assert_eq!(f32::MAX.interpolate(&-f32::MAX, 0.5), 0.0);
        assert_eq!(f64::MAX.interpolate(&-f64::MAX, 0.5), 0.0);
        assert_eq!(f64::MAX.interpolate(&f64::MAX, 1.5), f64::MAX);
        assert_eq!(
            (-0.0_f32).interpolate(&1.0, 0.0).to_bits(),
            (-0.0_f32).to_bits()
        );
        assert_eq!(
            1.0_f64.interpolate(&-0.0, 1.0).to_bits(),
            (-0.0_f64).to_bits()
        );
    }

    #[test]
    fn geometry_and_arrays_interpolate_every_component() {
        assert_eq!(
            [0.0, 4.0, -2.0].interpolate(&[8.0, 0.0, 2.0], 0.25),
            [2.0, 3.0, -1.0]
        );
        assert_eq!([0.0_f32; 0].interpolate(&[], 0.5), []);
        assert_eq!(
            Point::new(0.0, 4.0).interpolate(&Point::new(8.0, 0.0), 0.25),
            Point::new(2.0, 3.0)
        );
        assert_eq!(
            Size::new(0.0, 4.0).interpolate(&Size::new(8.0, 0.0), 0.25),
            Size::new(2.0, 3.0)
        );
        assert_eq!(
            Point::ZERO.interpolate(&Point::new(2.0, -2.0), 1.5),
            Point::new(3.0, -3.0)
        );
    }

    #[test]
    fn colors_interpolate_stored_rgba_and_clamp_overshoot() {
        let start = Color::rgba_f32(0.0, 1.0, 0.25, 0.0);
        let target = Color::rgba_f32(1.0, 0.0, 0.75, 1.0);
        assert_eq!(
            start.interpolate(&target, 0.5),
            Color::rgba_f32(0.5, 0.5, 0.5, 0.5)
        );
        assert_eq!(
            start.interpolate(&target, 1.5),
            Color::rgba_f32(1.0, 0.0, 1.0, 1.0)
        );
        assert_eq!(start.interpolate(&target, 0.0), start);
        assert_eq!(start.interpolate(&target, 1.0), target);
    }

    #[test]
    fn linear_timing_uses_deltas_and_finishes_exactly() {
        let mut value = Animated::new(2.0_f32);
        assert_eq!(value.value(), 2.0);
        assert_eq!(*value.target(), 2.0);
        assert!(!value.is_active());
        assert!(!value.advance(SECOND));
        assert!(value.retarget(10.0, Some(Animation::linear(SECOND))));
        assert_eq!(value.value(), 2.0);
        assert_eq!(*value.target(), 10.0);
        assert!(value.is_active());
        assert!(!value.advance(Duration::ZERO));
        assert!(value.advance(Duration::from_millis(250)));
        assert_eq!(value.value(), 4.0);
        assert!(value.advance(Duration::from_millis(500)));
        assert_eq!(value.value(), 8.0);
        assert!(value.advance(Duration::from_millis(250)));
        assert_eq!(value.value(), 10.0);
        assert!(!value.is_active());
        assert!(!value.advance(SECOND));
    }

    #[test]
    fn easing_curves_have_expected_quarter_points() {
        for (animation, samples) in [
            (Animation::linear(SECOND), [0.25, 0.5, 0.75]),
            (Animation::ease_in(SECOND), [0.0625, 0.25, 0.5625]),
            (Animation::ease_out(SECOND), [0.4375, 0.75, 0.9375]),
            (Animation::ease_in_out(SECOND), [0.125, 0.5, 0.875]),
        ] {
            let mut value = Animated::new(0.0_f32);
            value.retarget(1.0, Some(animation));
            for expected in samples {
                assert!(value.advance(Duration::from_millis(250)));
                assert_close(value.value(), expected);
                assert!(value.is_active());
            }
            assert!(value.advance(Duration::from_millis(250)));
            assert_eq!(value.value(), 1.0);
            assert!(!value.is_active());
        }
    }

    #[test]
    fn timing_is_independent_of_frame_partitioning() {
        for animation in animations(SECOND) {
            let mut single = Animated::new(-4.0_f64);
            let mut split = Animated::new(-4.0_f64);
            single.retarget(12.0, Some(animation));
            split.retarget(12.0, Some(animation));
            single.advance(Duration::from_millis(437));
            for millis in [37, 100, 300] {
                split.advance(Duration::from_millis(millis));
            }
            assert_eq!(single.value(), split.value());
            assert!(single.is_active());
            assert!(split.is_active());
            single.advance(Duration::from_millis(563));
            split.advance(SECOND);
            assert_eq!(single.value(), 12.0);
            assert_eq!(split.value(), 12.0);
            assert!(!single.is_active());
            assert!(!split.is_active());
        }
    }

    #[test]
    fn repeat_wraps_at_exact_boundaries_and_stays_active_for_unchanged_samples() {
        let animation = Animation::linear(Duration::from_nanos(4)).repeat_forever(false);
        let mut value = AnimatedValue::new(2.0_f32);
        assert!(value.retarget(10.0, Some(animation)));
        assert!(!value.advance(Duration::ZERO));
        for (nanos, expected) in [(1, 4.0), (2, 8.0), (1, 2.0), (1, 4.0), (11, 2.0)] {
            assert!(value.advance(Duration::from_nanos(nanos)));
            assert_eq!(value.value(), expected);
            assert_eq!(*value.target(), 10.0);
            assert!(value.is_active());
        }
        assert!(!value.advance(Duration::from_nanos(4)));
        assert_eq!(value.value(), 2.0);
        assert!(value.is_active());
        assert!(value.advance(Duration::from_nanos(1)));
        assert_eq!(value.value(), 4.0);
    }

    #[test]
    fn autoreverse_reaches_both_endpoints_and_skips_whole_legs() {
        let animation = Animation::linear(Duration::from_nanos(4)).repeat_forever(true);
        let mut value = AnimatedValue::new(2.0_f32);
        value.retarget(10.0, Some(animation));
        for (nanos, expected) in [
            (3, 8.0),
            (1, 10.0),
            (1, 8.0),
            (2, 4.0),
            (1, 2.0),
            (13, 8.0),
            (3, 2.0),
        ] {
            assert!(value.advance(Duration::from_nanos(nanos)));
            assert_eq!(value.value(), expected);
            assert_eq!(*value.target(), 10.0);
            assert!(value.is_active());
        }
        assert!(!value.advance(Duration::from_nanos(8)));
        assert_eq!(value.value(), 2.0);
        assert!(value.is_active());
    }

    #[test]
    fn autoreverse_retraces_easing_and_spring_curves_backward_in_time() {
        for animation in animations(SECOND) {
            let mut forward = AnimatedValue::new(0.0_f32);
            forward.retarget(1.0, Some(animation));
            let mut samples = [0.0; 5];
            for sample in &mut samples[1..] {
                forward.advance(Duration::from_millis(250));
                *sample = forward.value();
            }
            let mut repeated = AnimatedValue::new(0.0_f32);
            repeated.retarget(1.0, Some(animation.repeat_forever(true)));
            for expected in samples[1..].iter().chain(samples[..4].iter().rev()) {
                repeated.advance(Duration::from_millis(250));
                assert_eq!(repeated.value(), *expected, "{animation:?}");
                assert!(repeated.is_active());
            }
        }
    }

    #[test]
    fn repeat_timing_is_partition_invariant_around_boundaries() {
        for animation in animations(Duration::from_nanos(13)) {
            for autoreverses in [false, true] {
                let animation = Some(animation.repeat_forever(autoreverses));
                for total in [0, 1, 12, 13, 14, 25, 26, 27, 131, 1000] {
                    let mut single = AnimatedValue::new(-4.0_f64);
                    let mut split = AnimatedValue::new(-4.0_f64);
                    single.retarget(12.0, animation);
                    split.retarget(12.0, animation);
                    single.advance(Duration::from_nanos(total));
                    for nanos in [total / 3, total / 7, total - total / 3 - total / 7] {
                        split.advance(Duration::from_nanos(nanos));
                    }
                    assert_eq!(single.value(), split.value(), "{animation:?}, {total}");
                    assert_eq!(*single.target(), *split.target());
                    assert!(single.is_active());
                    assert!(split.is_active());
                    // Equal endpoints must also retain the same travel direction.
                    single.advance(Duration::from_nanos(1));
                    split.advance(Duration::from_nanos(1));
                    assert_eq!(single.value(), split.value(), "{animation:?}, {total}");
                }
            }
        }
    }

    #[test]
    fn repeat_timing_is_partition_invariant_beyond_duration_max() {
        let part = Duration::MAX / 3;
        for duration in [Duration::from_nanos(13), Duration::MAX - SECOND] {
            for animation in animations(duration) {
                for autoreverses in [false, true] {
                    let animation = Some(animation.repeat_forever(autoreverses));
                    let mut single = AnimatedValue::new(-4.0_f64);
                    let mut split = AnimatedValue::new(-4.0_f64);
                    single.retarget(12.0, animation);
                    split.retarget(12.0, animation);
                    for _ in 0..3 {
                        single.advance(Duration::MAX);
                        split.advance(part);
                        split.advance(Duration::MAX - part);
                        assert_eq!(single.value(), split.value(), "{animation:?}");
                        assert!(single.is_active());
                        assert!(split.is_active());
                    }
                    single.advance(duration / 4);
                    split.advance(duration / 4);
                    assert_eq!(single.value(), split.value(), "{animation:?}");
                }
            }
        }
    }

    #[test]
    fn maximum_duration_repeats_preserve_phase_and_direction_without_saturation() {
        let quarter = Duration::MAX / 4;
        for autoreverses in [false, true] {
            let mut value = AnimatedValue::new(0.0_f32);
            value.retarget(
                1.0,
                Some(Animation::linear(Duration::MAX).repeat_forever(autoreverses)),
            );
            value.advance(quarter);
            assert_close(value.value(), 0.25);
            value.advance(Duration::MAX);
            assert_close(value.value(), if autoreverses { 0.75 } else { 0.25 });
            value.advance(Duration::MAX);
            assert_close(value.value(), 0.25);
            value.advance(Duration::MAX - quarter);
            assert_eq!(value.value(), if autoreverses { 1.0 } else { 0.0 });
            value.advance(Duration::MAX);
            assert_eq!(value.value(), 0.0);
            assert!(value.is_active());
        }
    }

    #[test]
    fn maximum_deltas_keep_nanosecond_remainders_and_leg_parity() {
        // Duration::MAX is 2^64 * 10^9 - 1 nanoseconds: one nanosecond
        // short of a multiple of both a 10 ns leg and a 20 ns round trip.
        for autoreverses in [false, true] {
            let mut value = AnimatedValue::new(0.0_f32);
            value.retarget(
                10.0,
                Some(Animation::linear(Duration::from_nanos(10)).repeat_forever(autoreverses)),
            );
            for step in 1..=3 {
                value.advance(Duration::MAX);
                let expected = if autoreverses { step } else { 10 - step };
                assert_close(value.value(), expected as f32);
                assert!(value.is_active());
            }
            value.advance(Duration::from_nanos(3));
            assert_eq!(value.value(), 0.0);
            assert!(value.is_active());

            let mut nanos = AnimatedValue::new(0.0_f32);
            nanos.retarget(
                1.0,
                Some(Animation::linear(Duration::from_nanos(1)).repeat_forever(autoreverses)),
            );
            nanos.advance(Duration::MAX);
            assert_eq!(nanos.value(), if autoreverses { 1.0 } else { 0.0 });
            nanos.advance(Duration::MAX);
            assert_eq!(nanos.value(), 0.0);
            assert!(nanos.is_active());
        }
    }

    #[test]
    fn duplicate_repeat_targets_preserve_the_original_curve_phase_and_policy() {
        for autoreverses in [false, true] {
            let original = Animation::linear(SECOND).repeat_forever(autoreverses);
            let mut value = AnimatedValue::new(0.0_f32);
            value.retarget(8.0, Some(original));
            value.advance(Duration::from_millis(250));
            for replacement in [
                Some(original),
                Some(Animation::ease_in(Duration::from_secs(2))),
                Some(Animation::linear(SECOND).repeat_forever(!autoreverses)),
                Some(Animation::linear(Duration::ZERO).repeat_forever(true)),
                None,
            ] {
                assert!(!value.retarget(8.0, replacement));
                assert_eq!(value.value(), 2.0);
                assert!(value.is_active());
            }
            value.advance(Duration::from_millis(750));
            assert_eq!(value.value(), if autoreverses { 8.0 } else { 0.0 });
            // Even a duplicate while the target is displayed must keep repeating.
            assert!(!value.retarget(8.0, None));
            assert!(value.is_active());
            value.advance(Duration::from_millis(250));
            assert_eq!(value.value(), if autoreverses { 6.0 } else { 2.0 });
        }
    }

    #[test]
    fn retargeting_a_reverse_leg_starts_forward_from_the_displayed_value() {
        let linear = Animation::linear(SECOND);
        for replacement in [
            linear,
            linear.repeat_forever(false),
            linear.repeat_forever(true),
        ] {
            let mut value = AnimatedValue::new(0.0_f32);
            value.retarget(8.0, Some(linear.repeat_forever(true)));
            let storage = core::ptr::from_ref(value.transition.as_deref().unwrap());
            value.advance(Duration::from_millis(1250));
            assert_eq!(value.value(), 6.0);
            assert_eq!(
                storage,
                core::ptr::from_ref(value.transition.as_deref().unwrap())
            );
            assert!(value.retarget(10.0, Some(replacement)));
            assert_eq!(value.value(), 6.0);
            assert_eq!(*value.target(), 10.0);
            assert_eq!(
                storage,
                core::ptr::from_ref(value.transition.as_deref().unwrap())
            );
            value.advance(Duration::from_millis(250));
            assert_eq!(value.value(), 7.0);
            value.advance(Duration::from_millis(750));
            assert_eq!(
                value.value(),
                if replacement == linear.repeat_forever(false) {
                    6.0
                } else {
                    10.0
                }
            );
            assert_eq!(value.is_active(), replacement != linear);
        }
    }

    #[test]
    fn repeats_cancel_when_retargeted_to_the_displayed_value() {
        for autoreverses in [false, true] {
            let animation = Some(Animation::linear(SECOND).repeat_forever(autoreverses));
            let mut value = AnimatedValue::new(0.0_f32);
            assert!(!value.retarget(0.0, animation));
            assert!(!value.is_active());
            value.retarget(8.0, animation);
            value.advance(Duration::from_millis(1250));
            let displayed = value.value();
            assert!(value.retarget(displayed, animation));
            assert_eq!(*value.target(), displayed);
            assert!(!value.is_active());
            assert!(!value.advance(Duration::MAX));
        }
    }

    #[test]
    fn finish_stops_repeats_at_the_exact_target_in_either_direction() {
        for animation in animations(SECOND) {
            for autoreverses in [false, true] {
                for millis in [250, 1000, 1250, 2000] {
                    let mut value = AnimatedValue::new(2.0_f64);
                    value.retarget(-0.0, Some(animation.repeat_forever(autoreverses)));
                    value.advance(Duration::from_millis(millis));
                    assert!(value.finish());
                    assert_eq!(value.value().to_bits(), (-0.0_f64).to_bits());
                    assert_eq!(value.target().to_bits(), (-0.0_f64).to_bits());
                    assert!(!value.is_active());
                    assert!(!value.finish());
                    assert!(!value.advance(Duration::MAX));
                    assert!(!value.retarget(-0.0, Some(animation.repeat_forever(autoreverses))));
                    assert!(value.retarget(4.0, Some(Animation::linear(SECOND))));
                    value.advance(SECOND);
                    assert_eq!(value.value(), 4.0);
                    assert!(!value.is_active());
                }
            }
        }
    }

    #[test]
    fn zero_duration_repeats_snap_and_unanimated_targets_stop_active_repeats() {
        for autoreverses in [false, true] {
            let mut value = AnimatedValue::new(0.0_f32);
            value.retarget(
                8.0,
                Some(Animation::linear(SECOND).repeat_forever(autoreverses)),
            );
            value.advance(Duration::from_millis(1250));
            assert!(value.retarget(4.0, None));
            assert_eq!(value.value(), 4.0);
            assert!(!value.is_active());

            for animation in animations(Duration::ZERO) {
                for replacement in [
                    animation,
                    animation.repeat_forever(false),
                    animation.repeat_forever(true),
                ] {
                    value.retarget(
                        8.0,
                        Some(Animation::linear(SECOND).repeat_forever(autoreverses)),
                    );
                    value.advance(Duration::from_millis(1250));
                    assert!(value.retarget(0.0, Some(replacement)));
                    assert_eq!(value.value(), 0.0);
                    assert_eq!(*value.target(), 0.0);
                    assert!(!value.is_active());
                    assert!(!value.advance(Duration::MAX));
                }
            }
        }
    }

    #[test]
    fn duplicate_targets_preserve_elapsed_time_and_original_animation() {
        let animation = Animation::linear(SECOND);
        let mut value = Animated::new(0.0_f32);
        value.retarget(10.0, Some(animation));
        value.advance(Duration::from_millis(250));
        for replacement in [
            Some(animation),
            Some(animation.repeat_forever(false)),
            Some(animation.repeat_forever(true)),
            Some(Animation::ease_in(Duration::from_secs(2))),
            None,
            Some(Animation::linear(Duration::ZERO)),
        ] {
            assert!(!value.retarget(10.0, replacement));
            assert_eq!(value.value(), 2.5);
            assert!(value.is_active());
        }
        value.advance(Duration::from_millis(250));
        assert_eq!(value.value(), 5.0);
        value.advance(Duration::from_millis(500));
        assert_eq!(value.value(), 10.0);
        assert!(!value.is_active());
        assert!(!value.retarget(10.0, Some(animation)));
        assert!(!value.is_active());
    }

    #[test]
    fn midflight_retarget_starts_at_the_displayed_value() {
        let animation = Some(Animation::linear(SECOND));
        let mut value = Animated::new(0.0_f32);
        value.retarget(8.0, animation);
        value.advance(Duration::from_millis(250));
        assert_eq!(value.value(), 2.0);
        assert!(value.retarget(10.0, animation));
        assert_eq!(value.value(), 2.0);
        assert_eq!(*value.target(), 10.0);
        value.advance(Duration::from_millis(500));
        assert_eq!(value.value(), 6.0);
        assert!(value.retarget(-2.0, animation));
        assert_eq!(value.value(), 6.0);
        value.advance(Duration::from_millis(500));
        assert_eq!(value.value(), 2.0);
        value.advance(Duration::from_millis(500));
        assert_eq!(value.value(), -2.0);
        assert!(!value.is_active());
    }

    #[test]
    fn spring_retarget_preserves_position_even_during_overshoot() {
        let animation = Some(Animation::spring(SECOND, 0.5));
        let mut value = Animated::new(0.0_f32);
        value.retarget(1.0, animation);
        value.advance(Duration::from_millis(250));
        let displayed = value.value();
        assert!(displayed > 1.0);
        assert!(value.retarget(-1.0, animation));
        assert_eq!(value.value(), displayed);
        assert!(!value.advance(Duration::ZERO));
        assert_eq!(value.value(), displayed);
        assert!(value.advance(SECOND));
        assert_eq!(value.value(), -1.0);
        assert!(!value.is_active());
    }

    #[test]
    fn targeting_the_displayed_value_cancels_motion() {
        let animation = Some(Animation::linear(SECOND));
        let mut value = Animated::new(0.0_f32);
        value.retarget(8.0, animation);
        value.advance(Duration::from_millis(250));
        assert!(value.retarget(2.0, animation));
        assert_eq!(value.value(), 2.0);
        assert_eq!(*value.target(), 2.0);
        assert!(!value.is_active());
        assert!(!value.retarget(2.0, animation));
        assert!(!value.advance(SECOND));
    }

    #[test]
    fn absent_animation_and_zero_durations_snap_new_targets() {
        let mut value = Animated::new(0.0_f32);
        value.retarget(8.0, Some(Animation::linear(SECOND)));
        value.advance(Duration::from_millis(250));
        assert!(value.retarget(4.0, None));
        assert_eq!(value.value(), 4.0);
        assert_eq!(*value.target(), 4.0);
        assert!(!value.is_active());
        assert!(!value.retarget(4.0, None));

        for animation in animations(Duration::ZERO) {
            value.retarget(8.0, Some(Animation::linear(SECOND)));
            value.advance(Duration::from_millis(250));
            assert!(value.retarget(0.0, Some(animation)));
            assert_eq!(value.value(), 0.0);
            assert_eq!(*value.target(), 0.0);
            assert!(!value.is_active());
            assert!(!value.advance(SECOND));
        }
    }

    #[test]
    fn finish_is_exact_idempotent_and_allows_a_new_transition() {
        let mut value = Animated::new(Point::ZERO);
        assert!(!value.finish());
        let target = Point::new(3.0, -7.0);
        value.retarget(target, Some(Animation::spring(SECOND, 0.5)));
        value.advance(Duration::from_millis(250));
        assert!(value.finish());
        assert_eq!(value.value(), target);
        assert_eq!(*value.target(), target);
        assert!(!value.is_active());
        assert!(!value.finish());
        assert!(!value.advance(SECOND));
        assert!(value.retarget(Point::ZERO, Some(Animation::linear(SECOND))));
        assert_eq!(value.value(), target);
        value.advance(Duration::from_millis(500));
        assert_eq!(value.value(), Point::new(1.5, -3.5));
        assert!(value.finish());
        assert_eq!(value.value(), Point::ZERO);
    }

    #[test]
    fn activity_changes_are_reported_even_without_display_changes() {
        // A Clone-only value can reach its displayed target before time expires.
        #[derive(Clone, Debug, PartialEq)]
        struct Step(u8);

        impl Animatable for Step {
            fn interpolate(&self, target: &Self, progress: f32) -> Self {
                if progress < 0.5 {
                    self.clone()
                } else {
                    target.clone()
                }
            }
        }

        for finish_early in [false, true] {
            let mut value = Animated::new(Step(0));
            assert!(value.retarget(Step(1), Some(Animation::linear(SECOND))));
            assert!(!value.advance(Duration::from_millis(250)));
            assert_eq!(value.value(), Step(0));
            assert!(value.is_active());
            assert!(value.advance(Duration::from_millis(250)));
            assert_eq!(value.value(), Step(1));
            assert!(value.is_active());
            assert!(!value.retarget(Step(1), None));
            assert!(if finish_early {
                value.finish()
            } else {
                value.advance(Duration::from_millis(500))
            });
            assert!(!value.is_active());
            assert!(!value.finish());
        }
    }

    #[test]
    fn nanosecond_durations_and_overflowing_deltas_complete() {
        let mut value = Animated::new(0.0_f32);
        value.retarget(1.0, Some(Animation::linear(Duration::from_nanos(1))));
        assert!(!value.advance(Duration::ZERO));
        assert!(value.advance(Duration::from_nanos(1)));
        assert_eq!(value.value(), 1.0);
        assert!(!value.is_active());

        value.retarget(2.0, Some(Animation::linear(Duration::MAX)));
        assert!(value.advance(Duration::from_secs(u64::MAX / 2)));
        assert!(value.is_active());
        assert!(value.advance(Duration::MAX));
        assert_eq!(value.value(), 2.0);
        assert!(!value.is_active());
    }

    #[test]
    fn springs_remain_finite_bounded_and_settle_for_all_damping_regimes() {
        for damping in [
            0.0,
            f32::MIN_POSITIVE,
            0.5,
            f32::from_bits(1.0_f32.to_bits() - 1),
            1.0,
            f32::from_bits(1.0_f32.to_bits() + 1),
            2.0,
            f32::MAX,
        ] {
            let mut value = Animated::new(0.0_f32);
            value.retarget(1.0, Some(Animation::spring(SECOND, damping)));
            let mut previous = 0.0;
            for _ in 0..99 {
                value.advance(Duration::from_millis(10));
                let sample = value.value();
                assert!(sample.is_finite());
                assert!(
                    (0.0..=2.0).contains(&sample),
                    "damping={damping}, value={sample}"
                );
                if damping >= 1.0 {
                    assert!(sample >= previous);
                    assert!(sample <= 1.0);
                }
                previous = sample;
                assert!(value.is_active());
            }
            value.advance(Duration::from_millis(9));
            assert_close(value.value(), 1.0);
            assert!(value.is_active());
            assert!(value.advance(Duration::from_millis(1)));
            assert_eq!(value.value(), 1.0);
            assert!(!value.is_active());
        }
    }

    #[test]
    #[should_panic(expected = "spring damping ratio must be finite and nonnegative")]
    fn nan_damping_is_rejected_even_with_zero_duration() {
        Animation::spring(Duration::ZERO, f32::NAN);
    }

    #[test]
    #[should_panic(expected = "spring damping ratio must be finite and nonnegative")]
    fn infinite_damping_is_rejected() {
        Animation::spring(SECOND, f32::INFINITY);
    }

    #[test]
    #[should_panic(expected = "spring damping ratio must be finite and nonnegative")]
    fn negative_infinite_damping_is_rejected() {
        Animation::spring(SECOND, f32::NEG_INFINITY);
    }

    #[test]
    #[should_panic(expected = "spring damping ratio must be finite and nonnegative")]
    fn negative_damping_is_rejected() {
        Animation::spring(SECOND, -0.5);
    }
}
