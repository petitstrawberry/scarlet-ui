//! Process-local identities with the same range on every target.
//!
//! Each allocator owns an independent ID namespace. IDs start at one and are
//! never reused. Allocating an ID does not publish or synchronize its object;
//! callers must provide that synchronization separately.

#[cfg(target_has_atomic = "64")]
use core::sync::atomic::{AtomicU64, Ordering};

/// Allocates nonzero 64-bit identities without wrapping or truncating them.
///
/// Native 64-bit atomics avoid locks. Other targets lock only while allocating
/// an ID; storing, comparing, and passing an allocated ID needs no such lock.
pub struct IdAllocator {
    #[cfg(target_has_atomic = "64")]
    next: AtomicU64,
    #[cfg(not(target_has_atomic = "64"))]
    protected: ProtectedIdAllocator,
}

impl IdAllocator {
    /// Start an independent namespace at ID one.
    pub const fn new() -> Self {
        Self::starting_at(1)
    }

    const fn starting_at(next: u64) -> Self {
        Self {
            #[cfg(target_has_atomic = "64")]
            next: AtomicU64::new(next),
            #[cfg(not(target_has_atomic = "64"))]
            protected: ProtectedIdAllocator::new(next),
        }
    }

    /// Allocate an identity, panicking if this namespace is exhausted.
    ///
    /// Exhaustion is permanent: it never causes an old ID to be reused.
    pub fn allocate(&self) -> u64 {
        self.try_allocate().expect("ScarletUI ID space exhausted")
    }

    fn try_allocate(&self) -> Option<u64> {
        #[cfg(target_has_atomic = "64")]
        {
            self.next
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                    // Zero is reserved as the permanently exhausted state.
                    (next != 0).then(|| next.checked_add(1).unwrap_or(0))
                })
                .ok()
        }
        #[cfg(not(target_has_atomic = "64"))]
        self.protected.try_allocate()
    }
}

impl Default for IdAllocator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(test, not(target_has_atomic = "64")))]
struct ProtectedIdAllocator {
    next: crate::os::Mutex<u64>,
}

#[cfg(any(test, not(target_has_atomic = "64")))]
impl ProtectedIdAllocator {
    const fn new(next: u64) -> Self {
        Self {
            next: crate::os::Mutex::new(next),
        }
    }

    fn try_allocate(&self) -> Option<u64> {
        let mut next = self.next.lock();
        if *next == 0 {
            return None;
        }
        let id = *next;
        *next = id.checked_add(1).unwrap_or(0);
        Some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_cross_the_32_bit_boundary() {
        let start = u32::MAX as u64 - 1;
        let selected = IdAllocator::starting_at(start);
        let protected = ProtectedIdAllocator::new(start);
        for expected in start..start + 4 {
            assert_eq!(selected.try_allocate(), Some(expected));
            assert_eq!(protected.try_allocate(), Some(expected));
        }
    }

    #[test]
    fn exhaustion_never_reuses_zero_or_an_old_identity() {
        let selected = IdAllocator::starting_at(u64::MAX - 1);
        let protected = ProtectedIdAllocator::new(u64::MAX - 1);
        for expected in [Some(u64::MAX - 1), Some(u64::MAX), None, None] {
            assert_eq!(selected.try_allocate(), expected);
            assert_eq!(protected.try_allocate(), expected);
        }
    }

    #[test]
    #[should_panic(expected = "ScarletUI ID space exhausted")]
    fn infallible_allocation_reports_exhaustion() {
        IdAllocator::starting_at(0).allocate();
    }

    #[test]
    fn independent_namespaces_start_at_one() {
        let first = IdAllocator::new();
        let second = IdAllocator::new();
        assert_eq!(first.allocate(), 1);
        assert_eq!(first.allocate(), 2);
        assert_eq!(second.allocate(), 1);
    }

    #[test]
    fn concurrent_allocations_are_unique_in_both_implementations() {
        let selected = IdAllocator::new();
        let protected = ProtectedIdAllocator::new(1);
        std::thread::scope(|scope| {
            let workers: alloc::vec::Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        (0..512)
                            .map(|_| (selected.allocate(), protected.try_allocate().unwrap()))
                            .collect::<alloc::vec::Vec<_>>()
                    })
                })
                .collect();
            let (mut selected_ids, mut protected_ids): (alloc::vec::Vec<_>, alloc::vec::Vec<_>) =
                workers
                    .into_iter()
                    .flat_map(|worker| worker.join().unwrap())
                    .unzip();
            selected_ids.sort_unstable();
            protected_ids.sort_unstable();
            let expected: alloc::vec::Vec<_> = (1..=8 * 512).collect();
            assert_eq!(selected_ids, expected);
            assert_eq!(protected_ids, expected);
        });
    }
}
