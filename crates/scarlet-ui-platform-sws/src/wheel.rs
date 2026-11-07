//! Report-level wheel normalization with per-window subpixel carry.
//!
//! SWS sends legacy pixel deltas alongside high-resolution deltas. Prefer the
//! latter once per report, and retain their fractional logical pixels across
//! SYN_REPORT boundaries so slow, fine wheel movement is not discarded.

const LINE_DELTA: i32 = 32;
const HI_RES_UNITS_PER_NOTCH: i64 = 120;
const LEGACY_PIXELS_PER_NOTCH: i32 = 10;

#[derive(Clone, Copy, Debug, Default)]
struct Axis {
    discrete: i32,
    hi_res: i32,
    has_discrete: bool,
    has_hi_res: bool,
    remainder: i64,
}

impl Axis {
    fn take_normalized(&mut self) -> i32 {
        let delta = if self.has_hi_res {
            let scaled = i64::from(self.hi_res) * i64::from(LINE_DELTA) + self.remainder;
            self.remainder = scaled % HI_RES_UNITS_PER_NOTCH;
            (scaled / HI_RES_UNITS_PER_NOTCH).clamp(i32::MIN as i64, i32::MAX as i64) as i32
        } else if self.has_discrete {
            // A discrete-only report switches sources. Do not leak a previous
            // high-resolution fraction into a later high-resolution sequence.
            self.remainder = 0;
            let value = self.discrete;
            let detents = if value.unsigned_abs() >= LEGACY_PIXELS_PER_NOTCH as u32
                && value % LEGACY_PIXELS_PER_NOTCH == 0
            {
                value / LEGACY_PIXELS_PER_NOTCH
            } else {
                value
            };
            detents.saturating_mul(LINE_DELTA)
        } else {
            0
        };
        *self = Self {
            remainder: self.remainder,
            ..Self::default()
        };
        delta
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct PendingWheelDelta {
    x: Axis,
    y: Axis,
}

impl PendingWheelDelta {
    pub(super) fn add_discrete_x(&mut self, value: i32) {
        self.x.discrete = self.x.discrete.saturating_add(value);
        self.x.has_discrete = true;
    }
    pub(super) fn add_discrete_y(&mut self, value: i32) {
        self.y.discrete = self.y.discrete.saturating_add(value);
        self.y.has_discrete = true;
    }
    pub(super) fn add_hi_res_x(&mut self, value: i32) {
        self.x.hi_res = self.x.hi_res.saturating_add(value);
        self.x.has_hi_res = true;
    }
    pub(super) fn add_hi_res_y(&mut self, value: i32) {
        self.y.hi_res = self.y.hi_res.saturating_add(value);
        self.y.has_hi_res = true;
    }
    pub(super) fn is_empty(&self) -> bool {
        !self.x.has_discrete && !self.x.has_hi_res && !self.y.has_discrete && !self.y.has_hi_res
    }
    pub(super) fn take_normalized(&mut self) -> Option<(i32, i32)> {
        let delta = (self.x.take_normalized(), self.y.take_normalized());
        (delta != (0, 0)).then_some(delta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fine_reports_preserve_total_distance_and_direction() {
        for sign in [-1, 1] {
            let mut pending = PendingWheelDelta::default();
            let mut total = (0, 0);
            for _ in 0..120 {
                pending.add_hi_res_x(sign);
                pending.add_hi_res_y(-sign);
                if let Some((x, y)) = pending.take_normalized() {
                    total.0 += x;
                    total.1 += y;
                }
                assert!(pending.is_empty());
                assert_eq!(pending.take_normalized(), None);
            }
            assert_eq!(total, (sign * LINE_DELTA, -sign * LINE_DELTA));
        }
    }

    #[test]
    fn hi_res_wins_over_legacy_duplicate_in_each_report() {
        let mut pending = PendingWheelDelta::default();
        pending.add_discrete_x(-10);
        pending.add_hi_res_x(-120);
        pending.add_discrete_y(10);
        pending.add_hi_res_y(120);
        assert_eq!(pending.take_normalized(), Some((-32, 32)));
    }

    #[test]
    fn legacy_pixels_and_detents_remain_supported() {
        for (input, expected) in [(10, 32), (-10, -32), (1, 32), (-1, -32), (20, 64)] {
            let mut pending = PendingWheelDelta::default();
            pending.add_discrete_y(input);
            assert_eq!(pending.take_normalized(), Some((0, expected)));
        }
    }

    #[test]
    fn opposite_fine_reports_cancel_without_drift() {
        let mut pending = PendingWheelDelta::default();
        for _ in 0..100 {
            pending.add_hi_res_y(1);
            assert_eq!(pending.take_normalized(), None);
            pending.add_hi_res_y(-1);
            assert_eq!(pending.take_normalized(), None);
        }
        pending.add_hi_res_y(120);
        assert_eq!(pending.take_normalized(), Some((0, 32)));
    }

    #[test]
    fn axes_keep_independent_remainders() {
        let mut pending = PendingWheelDelta::default();
        pending.add_hi_res_x(3);
        assert_eq!(pending.take_normalized(), None);
        pending.add_discrete_y(1);
        assert_eq!(pending.take_normalized(), Some((0, 32)));
        pending.add_hi_res_x(1);
        assert_eq!(pending.take_normalized(), Some((1, 0)));
    }

    #[test]
    fn discrete_source_switch_and_window_reset_drop_old_fractions() {
        let mut pending = PendingWheelDelta::default();
        pending.add_hi_res_y(3);
        assert_eq!(pending.take_normalized(), None);
        pending.add_discrete_y(1);
        assert_eq!(pending.take_normalized(), Some((0, 32)));
        pending.add_hi_res_y(1);
        assert_eq!(pending.take_normalized(), None);
        pending.add_hi_res_y(2);
        assert_eq!(pending.take_normalized(), None);
        pending = PendingWheelDelta::default();
        pending.add_hi_res_y(1);
        assert_eq!(pending.take_normalized(), None);
    }

    #[test]
    fn extreme_reports_do_not_overflow() {
        for input in [i32::MIN, i32::MAX] {
            let mut pending = PendingWheelDelta::default();
            pending.add_hi_res_y(input);
            pending.add_hi_res_y(input);
            assert_eq!(
                pending.take_normalized(),
                Some((0, (i64::from(input) * 32 / 120) as i32))
            );
            pending.add_discrete_y(input);
            assert_eq!(
                pending.take_normalized(),
                Some((0, input.saturating_mul(32)))
            );
        }
    }
}
