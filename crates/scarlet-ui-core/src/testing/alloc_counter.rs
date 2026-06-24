use core::alloc::{GlobalAlloc, Layout};
use core::cell::Cell;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct AllocationSnapshot {
    pub(crate) allocations: usize,
    pub(crate) deallocations: usize,
    pub(crate) allocated_bytes: usize,
}

struct CountingAllocator;

std::thread_local! {
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
    static DEALLOCATIONS: Cell<usize> = const { Cell::new(0) };
    static ALLOCATED_BYTES: Cell<usize> = const { Cell::new(0) };
}

#[global_allocator]
static GLOBAL_ALLOCATOR: CountingAllocator = CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.with(|count| count.set(count.get().saturating_add(1)));
        ALLOCATED_BYTES.with(|bytes| bytes.set(bytes.get().saturating_add(layout.size())));
        unsafe { std::alloc::System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        DEALLOCATIONS.with(|count| count.set(count.get().saturating_add(1)));
        unsafe { std::alloc::System.dealloc(ptr, layout) };
    }
}

pub(crate) fn reset_allocation_counts() {
    ALLOCATIONS.with(|count| count.set(0));
    DEALLOCATIONS.with(|count| count.set(0));
    ALLOCATED_BYTES.with(|bytes| bytes.set(0));
}

pub(crate) fn allocation_snapshot() -> AllocationSnapshot {
    AllocationSnapshot {
        allocations: ALLOCATIONS.with(Cell::get),
        deallocations: DEALLOCATIONS.with(Cell::get),
        allocated_bytes: ALLOCATED_BYTES.with(Cell::get),
    }
}

pub(crate) fn measure_allocations<F: FnOnce()>(f: F) -> AllocationSnapshot {
    reset_allocation_counts();
    f();
    allocation_snapshot()
}
