//! Debug logging flags for ScarletUI.

use core::sync::atomic::{AtomicBool, Ordering};

static DEBUG_ENABLED: AtomicBool = AtomicBool::new(false);
static WHEEL_LOG_ENABLED: AtomicBool = AtomicBool::new(false);
static REPAINT_BOUNDARY_LOG_ENABLED: AtomicBool = AtomicBool::new(false);

/// Enable or disable debug logging.
pub fn set_enabled(enabled: bool) {
    DEBUG_ENABLED.store(enabled, Ordering::SeqCst);
}

/// Check if debug logging is enabled.
pub fn is_enabled() -> bool {
    DEBUG_ENABLED.load(Ordering::SeqCst)
}

/// Enable or disable focused wheel dispatch logging.
pub fn set_wheel_log_enabled(enabled: bool) {
    WHEEL_LOG_ENABLED.store(enabled, Ordering::SeqCst);
}

/// Check if focused wheel dispatch logging is enabled.
/// The environment default is read once; the explicit runtime flag remains mutable.
pub fn wheel_log_enabled() -> bool {
    WHEEL_LOG_ENABLED.load(Ordering::SeqCst) || wheel_log_env_enabled()
}

/// Enable or disable focused repaint boundary cache logging.
pub fn set_repaint_boundary_log_enabled(enabled: bool) {
    REPAINT_BOUNDARY_LOG_ENABLED.store(enabled, Ordering::SeqCst);
}

/// Check if focused repaint boundary cache logging is enabled.
/// The environment default is read once; the explicit runtime flag remains mutable.
pub fn repaint_boundary_log_enabled() -> bool {
    REPAINT_BOUNDARY_LOG_ENABLED.load(Ordering::SeqCst) || repaint_boundary_log_env_enabled()
}

#[cfg(feature = "std")]
fn wheel_log_env_enabled() -> bool {
    // Even an absent variable can allocate for OS string conversion on Windows.
    // Keep environment discovery out of the warmed input/paint path.
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("SCARLET_UI_WHEEL_LOG")
            .is_ok_and(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
    })
}

#[cfg(feature = "std")]
fn repaint_boundary_log_env_enabled() -> bool {
    // Even an absent variable can allocate for OS string conversion on Windows.
    // Keep environment discovery out of the warmed input/paint path.
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("SCARLET_UI_REPAINT_LOG")
            .is_ok_and(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
    })
}

#[cfg(not(feature = "std"))]
fn wheel_log_env_enabled() -> bool {
    false
}

#[cfg(not(feature = "std"))]
fn repaint_boundary_log_env_enabled() -> bool {
    false
}

/// Enable focused input/frame timing via `SCARLET_UI_FRAME_LOG=1`.
/// The environment flag is read once; ordinary frames do not allocate or log.
pub fn frame_log_enabled() -> bool {
    #[cfg(feature = "std")]
    {
        static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ENABLED.get_or_init(|| {
            std::env::var("SCARLET_UI_FRAME_LOG")
                .is_ok_and(|value| matches!(value.as_str(), "1" | "true"))
        })
    }
    #[cfg(not(feature = "std"))]
    {
        false
    }
}

/// Disable retained GPU paint for controlled A/B measurements.
pub fn retained_paint_enabled() -> bool {
    #[cfg(feature = "std")]
    {
        static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ENABLED.get_or_init(|| {
            std::env::var("SCARLET_UI_RETAINED_PAINT").map_or(true, |value| value != "0")
        })
    }
    #[cfg(not(feature = "std"))]
    {
        true
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::testing::alloc_counter::measure_allocations;

    #[test]
    fn warmed_debug_flag_queries_do_not_allocate() {
        // Warm the environment defaults directly, even if another test has
        // enabled an explicit runtime flag and the public getter short-circuits.
        std::hint::black_box(wheel_log_env_enabled());
        std::hint::black_box(repaint_boundary_log_env_enabled());
        let snapshot = measure_allocations(|| {
            for _ in 0..64 {
                std::hint::black_box(wheel_log_env_enabled());
                std::hint::black_box(repaint_boundary_log_env_enabled());
                std::hint::black_box(wheel_log_enabled());
                std::hint::black_box(repaint_boundary_log_enabled());
            }
        });
        assert_eq!(snapshot.allocations, 0);
        assert_eq!(snapshot.deallocations, 0);
        assert_eq!(snapshot.allocated_bytes, 0);
    }
}
