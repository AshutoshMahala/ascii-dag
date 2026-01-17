#![no_main]

use libfuzzer_sys::fuzz_target;
use ascii_dag::arena::Arena;
use ascii_dag::graph::DAG;

/// Fuzz the arena allocator with random allocation patterns.
/// This tests for buffer overflows and memory corruption.
fuzz_target!(|data: &[u8]| {
    if data.len() < 4 {
        return;
    }
    
    // Use data to drive allocation patterns
    let mut buffer = vec![0u8; 64 * 1024]; // 64KB arena
    let mut arena = Arena::new(&mut buffer);
    
    let mut i = 0;
    while i + 2 <= data.len() {
        let alloc_type = data[i] % 4;
        let count = (data[i + 1] as usize % 256) + 1;
        
        match alloc_type {
            0 => {
                // Allocate u8 slice
                let _ = arena.alloc_slice_zeroed::<u8>(count);
            }
            1 => {
                // Allocate u32 slice
                let _ = arena.alloc_slice_zeroed::<u32>(count);
            }
            2 => {
                // Allocate usize slice
                let _ = arena.alloc_slice_zeroed::<usize>(count);
            }
            3 => {
                // Allocate tuple slice (simulating layout temps)
                let _ = arena.alloc_slice_zeroed::<(usize, usize)>(count);
            }
            _ => {}
        }
        
        i += 2;
    }
});
