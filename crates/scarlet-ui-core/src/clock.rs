//! Monotonic-clock compatibility for host and legacy Scarlet builds.

#[cfg(feature = "std")]
pub(crate) use std::time::Instant;

/// Legacy Scarlet standard-library builds do not expose a monotonic instant.
/// Returning zero elapsed time preserves correctness while applying the full
/// configured wait interval for frame pacing and retaining idle wheel capture.
#[cfg(not(feature = "std"))]
#[derive(Clone, Copy)]
pub(crate) struct Instant;

#[cfg(not(feature = "std"))]
impl Instant {
    pub(crate) const fn now() -> Self {
        Self
    }

    pub(crate) const fn elapsed(self) -> core::time::Duration {
        core::time::Duration::ZERO
    }
}
