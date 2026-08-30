//! Counting global allocator — the allocation-gate harness (test
//! builds only).
//!
//! Delegates straight to `System`, counting allocations per thread.
//! Zero-allocation contracts (planner replans, composer repaints,
//! view lending) snapshot [`allocations_on_this_thread`] around the
//! measured window and assert a delta of zero. Installed for the
//! whole lib-test binary; other tests are unaffected beyond a
//! thread-local bump per allocation.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

std::thread_local! {
    static ALLOC_COUNT: Cell<u64> = const { Cell::new(0) };
}

struct CountingAlloc;

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOC_COUNT.with(|c| c.set(c.get() + 1));
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOC_COUNT.with(|c| c.set(c.get() + 1));
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOC_COUNT.with(|c| c.set(c.get() + 1));
        unsafe { System.realloc(ptr, layout, new_size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static COUNTING_ALLOC: CountingAlloc = CountingAlloc;

/// Allocations performed by the current thread since process start.
pub(crate) fn allocations_on_this_thread() -> u64 {
    ALLOC_COUNT.with(|c| c.get())
}
