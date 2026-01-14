//! Arena Performance Benchmark
//!
//! This benchmark measures the raw performance of arena allocation vs heap allocation
//! to estimate the potential speedup from a full no-alloc refactor.
//!
//! Run with: cargo run --example arena_perf --release

use ascii_dag::arena::Arena;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Simulates what a fully arena-based DAG would do:
/// - Store nodes in arena-allocated slices
/// - Store edges in arena-allocated slices
/// - Build adjacency lists in arena
/// - Compute layout with arena-allocated temporaries
fn benchmark_arena_simulation(node_count: usize, edge_count: usize, iterations: usize) -> Duration {
    // Size the arena for the workload
    // We allocate one big usize buffer and partition it manually
    let total_usizes = node_count * 3 + edge_count * 2 + node_count + 1 + edge_count + node_count;
    let arena_size = total_usizes * 8 + 1024;
    let mut arena_buffer = vec![0u8; arena_size];
    
    let mut total_time = Duration::ZERO;
    
    for _ in 0..iterations {
        let mut arena = Arena::new(&mut arena_buffer);
        let start = Instant::now();
        
        // Allocate one big block and partition it using split_at_mut
        let big_block: &mut [usize] = arena.alloc_slice_zeroed(total_usizes).unwrap();
        
        // Split into sections using split_at_mut (borrow-checker friendly)
        let (nodes, rest) = big_block.split_at_mut(node_count * 3);
        let (edges, rest) = rest.split_at_mut(edge_count * 2);
        let (adj_offsets, rest) = rest.split_at_mut(node_count + 1);
        let (adj_data, medians_raw) = rest.split_at_mut(edge_count);
        
        // Initialize nodes: [id, level, x_coord] per node
        for i in 0..node_count {
            nodes[i * 3] = i;           // id
            nodes[i * 3 + 1] = i % 10;  // level
            nodes[i * 3 + 2] = (i % 20) * 5; // x_coord
        }
        
        // Initialize edges: [from, to] per edge
        for i in 0..edge_count {
            edges[i * 2] = i % node_count;
            edges[i * 2 + 1] = (i + 1) % node_count;
        }
        
        // Build adjacency offsets
        let mut adj_offset = 0;
        for i in 0..node_count {
            adj_offsets[i] = adj_offset;
            let child_count = (edge_count / node_count).max(1);
            adj_offset += child_count.min(edge_count.saturating_sub(adj_offset));
        }
        adj_offsets[node_count] = adj_offset;
        
        // Fill adjacency data
        for i in 0..edge_count.min(adj_offset) {
            adj_data[i] = (i + 1) % node_count;
        }
        
        // Compute medians (crossing reduction simulation)
        // We store the f32 bits as usize
        for i in 0..node_count {
            let adj_start = adj_offsets[i];
            let adj_end = adj_offsets[i + 1];
            let sum: usize = adj_data[adj_start..adj_end].iter().sum();
            let count = adj_end - adj_start;
            let median = if count > 0 { sum as f32 / count as f32 } else { i as f32 };
            medians_raw[i] = median.to_bits() as usize;
        }
        
        total_time += start.elapsed();
        
        // Arena reset is O(1)
        arena.reset();
    }
    
    total_time / iterations as u32
}

/// Simulates the same operations using heap allocations
fn benchmark_heap_simulation(node_count: usize, edge_count: usize, iterations: usize) -> Duration {
    let mut total_time = Duration::ZERO;
    
    for _ in 0..iterations {
        let start = Instant::now();
        
        // Simulate node storage
        let mut nodes: Vec<(usize, usize)> = Vec::with_capacity(node_count);
        for i in 0..node_count {
            nodes.push((i, i * 10));
        }
        
        // Simulate edge storage
        let mut edges: Vec<(usize, usize)> = Vec::with_capacity(edge_count);
        for i in 0..edge_count {
            edges.push((i % node_count, (i + 1) % node_count));
        }
        
        // Simulate adjacency list (children per node)
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); node_count];
        for i in 0..node_count {
            let child_count = (edge_count / node_count).max(1);
            for j in 0..child_count {
                children[i].push((i + j + 1) % node_count);
            }
        }
        
        // Simulate level assignment
        let mut levels: Vec<usize> = Vec::with_capacity(node_count);
        for i in 0..node_count {
            levels.push(i % 10);
        }
        
        // Simulate x-coordinate assignment
        let mut x_coords: Vec<usize> = Vec::with_capacity(node_count);
        for i in 0..node_count {
            x_coords.push((i % 20) * 5);
        }
        
        // Simulate crossing reduction median computation
        let mut medians: Vec<f32> = Vec::with_capacity(node_count);
        for i in 0..node_count {
            let sum: usize = children[i].iter().sum();
            let count = children[i].len();
            medians.push(if count > 0 { sum as f32 / count as f32 } else { i as f32 });
        }
        
        total_time += start.elapsed();
        
        // Heap deallocation happens here (implicit drop)
        drop((nodes, edges, children, levels, x_coords, medians));
    }
    
    total_time / iterations as u32
}

/// Benchmark HashMap vs flat array lookup
fn benchmark_lookup_arena(node_count: usize, lookups: usize) -> Duration {
    let arena_size = node_count * 16 + 1024;
    let mut arena_buffer = vec![0u8; arena_size];
    let mut arena = Arena::new(&mut arena_buffer);
    
    // Arena: use flat array (O(1) index lookup)
    let id_to_index: &mut [usize] = arena.alloc_slice_default(node_count).unwrap();
    for i in 0..node_count {
        id_to_index[i] = i;
    }
    
    let start = Instant::now();
    let mut sum = 0usize;
    for i in 0..lookups {
        let id = i % node_count;
        sum += id_to_index[id];
    }
    let arena_time = start.elapsed();
    
    // Prevent optimization
    std::hint::black_box(sum);
    
    arena_time
}

fn benchmark_lookup_hashmap(node_count: usize, lookups: usize) -> Duration {
    // HashMap for id lookup
    let mut id_to_index: HashMap<usize, usize> = HashMap::with_capacity(node_count);
    for i in 0..node_count {
        id_to_index.insert(i, i);
    }
    
    let start = Instant::now();
    let mut sum = 0usize;
    for i in 0..lookups {
        let id = i % node_count;
        sum += *id_to_index.get(&id).unwrap();
    }
    let hashmap_time = start.elapsed();
    
    // Prevent optimization
    std::hint::black_box(sum);
    
    hashmap_time
}

fn format_duration(d: Duration) -> String {
    let nanos = d.as_nanos();
    if nanos < 1000 {
        format!("{}ns", nanos)
    } else if nanos < 1_000_000 {
        format!("{:.2}µs", nanos as f64 / 1000.0)
    } else if nanos < 1_000_000_000 {
        format!("{:.2}ms", nanos as f64 / 1_000_000.0)
    } else {
        format!("{:.2}s", nanos as f64 / 1_000_000_000.0)
    }
}

fn main() {
    println!("=== Arena vs Heap Performance Potential ===\n");
    
    let test_cases = [
        ("Tiny", 10, 20),
        ("Small", 50, 100),
        ("Medium", 200, 400),
        ("Large", 1000, 2000),
        ("XLarge", 5000, 10000),
    ];
    
    println!("## DAG Construction + Layout Simulation\n");
    println!("| {:10} | {:>12} | {:>12} | {:>10} |", "Size", "Heap", "Arena", "Speedup");
    println!("|{:-<12}|{:-<14}|{:-<14}|{:-<12}|", "", "", "", "");
    
    for (name, nodes, edges) in test_cases {
        let iterations = if nodes > 1000 { 100 } else { 1000 };
        
        let heap_time = benchmark_heap_simulation(nodes, edges, iterations);
        let arena_time = benchmark_arena_simulation(nodes, edges, iterations);
        
        let speedup = heap_time.as_nanos() as f64 / arena_time.as_nanos() as f64;
        
        println!(
            "| {:10} | {:>12} | {:>12} | {:>9.2}x |",
            name,
            format_duration(heap_time),
            format_duration(arena_time),
            speedup
        );
    }
    
    println!("\n## ID Lookup: HashMap vs Flat Array\n");
    println!("| {:10} | {:>12} | {:>12} | {:>10} |", "Nodes", "HashMap", "Array", "Speedup");
    println!("|{:-<12}|{:-<14}|{:-<14}|{:-<12}|", "", "", "", "");
    
    for (name, nodes, _) in test_cases {
        let lookups = 100_000;
        
        let hashmap_time = benchmark_lookup_hashmap(nodes, lookups);
        let array_time = benchmark_lookup_arena(nodes, lookups);
        
        let speedup = hashmap_time.as_nanos() as f64 / array_time.as_nanos() as f64;
        
        println!(
            "| {:10} | {:>12} | {:>12} | {:>9.2}x |",
            name,
            format_duration(hashmap_time),
            format_duration(array_time),
            speedup
        );
    }
    
    println!("\n## Summary\n");
    println!("Arena-based allocation provides:");
    println!("  - Faster allocation (bump pointer vs malloc)");
    println!("  - Better cache locality (contiguous memory)");
    println!("  - O(1) reset vs O(n) deallocation");
    println!("  - Predictable memory usage");
    println!();
    println!("Flat array lookup (via ID remapping) is faster than HashMap.");
    println!("Full no-alloc refactor could achieve these speedups.");
}
