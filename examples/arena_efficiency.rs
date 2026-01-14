//! Arena efficiency analysis - measure overhead sources.
//!
//! Run with: cargo run --example arena_efficiency --release

use std::time::Instant;

fn main() {
    println!("=== Arena Efficiency Analysis ===\n");
    
    // Test 1: Zeroing overhead
    test_zeroing_overhead();
    
    // Test 2: Allocation fragmentation 
    test_alignment_waste();
    
    // Test 3: Over-allocation analysis
    analyze_over_allocation();
}

fn test_zeroing_overhead() {
    println!("## 1. Zeroing Overhead\n");
    
    let sizes = [1024, 10 * 1024, 100 * 1024, 1024 * 1024];
    
    for size in sizes {
        let mut buffer = vec![0xFFu8; size];
        
        // Time: just allocate (no zero)
        let start = Instant::now();
        for _ in 0..1000 {
            let _ = &mut buffer[..];
        }
        let no_zero = start.elapsed();
        
        // Time: zero with fill
        let start = Instant::now();
        for _ in 0..1000 {
            buffer.fill(0);
        }
        let with_zero = start.elapsed();
        
        // Time: zero with loop
        let start = Instant::now();
        for _ in 0..1000 {
            for b in buffer.iter_mut() {
                *b = 0;
            }
        }
        let with_loop = start.elapsed();
        
        println!("  {} KB:", size / 1024);
        println!("    No zero:    {:?}", no_zero / 1000);
        println!("    .fill(0):   {:?} ({:.1}x slower)", 
                 with_zero / 1000, 
                 with_zero.as_nanos() as f64 / no_zero.as_nanos().max(1) as f64);
        println!("    Loop zero:  {:?} ({:.1}x slower)", 
                 with_loop / 1000,
                 with_loop.as_nanos() as f64 / no_zero.as_nanos().max(1) as f64);
    }
    println!();
}

fn test_alignment_waste() {
    println!("## 2. Alignment Waste\n");
    
    // Simulate arena allocations with different types
    let mut offset = 0usize;
    let mut wasted = 0usize;
    let buffer_size = 64 * 1024;
    
    // Typical allocation pattern: mix of u8, usize, tuples
    let allocations: &[(usize, usize)] = &[
        (1, 100),      // 100 u8s (align 1)
        (8, 50),       // 50 usizes (align 8)
        (1, 200),      // 200 u8s
        (8, 100),      // 100 usizes
        (16, 25),      // 25 (usize, usize) tuples
        (1, 50),       // 50 u8s
        (8, 200),      // 200 usizes
    ];
    
    for (align, count) in allocations {
        let size = align * count;
        let aligned_offset = (offset + align - 1) & !(align - 1);
        let padding = aligned_offset - offset;
        wasted += padding;
        offset = aligned_offset + size;
    }
    
    println!("  Total allocated: {} bytes", offset);
    println!("  Alignment waste: {} bytes ({:.1}%)", wasted, wasted as f64 / offset as f64 * 100.0);
    println!();
}

fn analyze_over_allocation() {
    println!("## 3. Over-allocation Analysis (AFTER OPTIMIZATION)\n");
    
    // For different graph types, compare actual vs allocated
    let cases = [
        ("10-node chain", 10, 9, 10, 0),       // nodes, edges, levels, dummy_nodes
        ("100-node chain", 100, 99, 100, 0),
        ("10-node diamond", 10, 15, 4, 10),    // some dummy nodes
        ("100-node wide fan", 100, 198, 3, 0), // 1 source -> 98 middle -> 1 sink
    ];
    
    for (name, nodes, edges, actual_levels, actual_dummies) in cases {
        // NEW: Tighter estimates
        let max_levels = nodes.min(256);
        let max_vnodes = (nodes + edges).min(500000);  // Was: nodes * 2 + edges
        let max_dummy_waypoints = (edges * 2).min(500000);  // Was: edges * 8
        
        let allocated_temps = 
            nodes * 8 +                           // node_levels
            (max_levels + 1) * 8 +                // vlevel_offsets
            max_levels * 8 +                      // level_counts (NEW - was fixed 256)
            max_vnodes * 2 * 8 +                  // vnode_data
            max_vnodes * 8 +                      // x_coords
            max_vnodes * 8 +                      // widths
            nodes * 32 +                          // real_coords (4 usizes)
            (edges + 1) * 8 +                     // dummy_offsets
            max_dummy_waypoints * 16 +            // dummy_data (2 usizes)
            nodes.min(50000) * 12 +               // medians
            nodes.min(50000) * 8;                 // positions
        
        // What we actually need
        let actual_vnodes = nodes + actual_dummies;
        let needed_temps =
            nodes * 8 +                           // node_levels
            (actual_levels + 1) * 8 +             // vlevel_offsets
            actual_levels * 8 +                   // level_counts
            actual_vnodes * 2 * 8 +               // vnode_data
            actual_vnodes * 8 +                   // x_coords
            actual_vnodes * 8 +                   // widths
            nodes * 32 +                          // real_coords
            (edges + 1) * 8 +                     // dummy_offsets
            actual_dummies * 16 +                 // dummy_data (actual)
            actual_levels * 12 +                  // medians
            actual_levels * 8;                    // positions
        
        let waste = allocated_temps - needed_temps;
        let ratio = allocated_temps as f64 / needed_temps as f64;
        
        println!("  {}:", name);
        println!("    Needed:    {:>8} bytes", needed_temps);
        println!("    Allocated: {:>8} bytes", allocated_temps);
        println!("    Waste:     {:>8} bytes ({:.1}x over)", waste, ratio);
    }
    
    println!();
    println!("## Optimizations Applied:\n");
    println!("  1. alloc_raw_uninit() - skip zeroing when we'll overwrite");
    println!("  2. Tighter vnodes estimate: nodes + edges*4 (was 2*nodes + edges)");
    println!("  3. Tighter waypoints estimate: edges * 4 (was edges * 8)");
    println!("  4. level_counts from arena (was fixed [0; 256])");
}
