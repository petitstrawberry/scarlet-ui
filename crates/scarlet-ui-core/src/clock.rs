//! Monotonic-clock compatibility for host and legacy Scarlet builds.

#[cfg(all(feature = "std", not(target_arch = "wasm32")))]
pub(crate) use std::time::Instant;

/// Legacy Scarlet exposes the kernel's nanosecond monotonic clock through its
/// syscall API. Animation time must advance even while waiting for a frame grant.
#[cfg(not(feature = "std"))]
#[derive(Clone, Copy)]
pub(crate) struct Instant(u64);

#[cfg(not(feature = "std"))]
impl Instant {
    pub(crate) fn now() -> Self {
        Self(unsafe { std::syscall::syscall0(std::syscall::Syscall::MonotonicTime) } as u64)
    }

    pub(crate) fn elapsed(self) -> core::time::Duration {
        core::time::Duration::from_nanos(Self::now().0.saturating_sub(self.0))
    }
}

#[cfg(all(feature = "std", target_arch = "wasm32"))]
pub(crate) use web_time::Instant;
