#![no_std]
#![no_main]

extern crate alloc;
use core::alloc::{GlobalAlloc, Layout};

// Dummy allocator to satisfy the linker for this proof-of-concept.
struct DummyAllocator;

unsafe impl GlobalAlloc for DummyAllocator {
    unsafe fn alloc(&self, _layout: Layout) -> *mut u8 {
        core::ptr::null_mut()
    }
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
}

#[global_allocator]
static ALLOCATOR: DummyAllocator = DummyAllocator;

use ascii_dag::graph::DAG;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    // This is a compilation proof that the library works in a no_std environment.
    let dag = DAG::from_edges(&[(1, "Core"), (2, "HAL"), (3, "Driver")], &[(1, 2), (2, 3)]);

    // We can't print, but we can verify rendering compiles
    let _output = dag.render();

    loop {}
}
