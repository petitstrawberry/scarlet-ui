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
pub fn wheel_log_enabled() -> bool {
    WHEEL_LOG_ENABLED.load(Ordering::SeqCst) || wheel_log_env_enabled()
}

/// Enable or disable focused repaint boundary cache logging.
pub fn set_repaint_boundary_log_enabled(enabled: bool) {
    REPAINT_BOUNDARY_LOG_ENABLED.store(enabled, Ordering::SeqCst);
}

/// Check if focused repaint boundary cache logging is enabled.
pub fn repaint_boundary_log_enabled() -> bool {
    REPAINT_BOUNDARY_LOG_ENABLED.load(Ordering::SeqCst) || repaint_boundary_log_env_enabled()
}

#[cfg(feature = "std")]
fn wheel_log_env_enabled() -> bool {
    std::env::var("SCARLET_UI_WHEEL_LOG")
        .is_ok_and(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
}

#[cfg(feature = "std")]
fn repaint_boundary_log_env_enabled() -> bool {
    std::env::var("SCARLET_UI_REPAINT_LOG")
        .is_ok_and(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
}

#[cfg(not(feature = "std"))]
fn wheel_log_env_enabled() -> bool {
    false
}

#[cfg(not(feature = "std"))]
fn repaint_boundary_log_env_enabled() -> bool {
    false
}
