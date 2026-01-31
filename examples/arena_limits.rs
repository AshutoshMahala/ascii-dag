//! Arena memory limit testing - find minimum viable memory for different graph sizes.
//!
//! Run with: cargo run --example arena_limits --release

use ascii_dag::arena::Arena;
use ascii_dag::graph::DAG;

fn main() {
    println!("=== Arena Memory Limit Testing ===\n");

    // Test different graph sizes
    let test_cases = [
        ("Tiny (10 nodes)", build_chain(10)),
        ("Small (100 nodes)", build_chain(100)),
        ("Medium (1000 nodes)", build_chain(1000)),
        ("Large (5000 nodes)", build_chain(5000)),
        ("Diamond Small", build_diamond(10)),
        ("Diamond Medium", build_diamond(100)),
        ("Fan Small", build_fan(50)),
        ("Fan Medium", build_fan(500)),
    ];

    for (name, dag) in test_cases {
        find_minimum_memory(name, &dag);
        println!();
    }

    // Now let's crash test with definitely too little memory
    println!("=== Crash Testing (should fail gracefully) ===\n");
    crash_test();

    // Debug the 100-node chain specifically
    println!("\n=== Debug: 100-node chain ===\n");
    debug_100_node_chain();
}

fn find_minimum_memory(name: &str, dag: &DAG) {
    let csr_estimate = dag.estimate_csr_arena_size();

    println!("Testing: {} (CSR estimate: {} bytes)", name, csr_estimate);

    // Test progressively larger multipliers (CSR estimate is often too small!)
    let multipliers = [1.0, 2.0, 5.0, 10.0, 20.0, 30.0, 40.0, 50.0, 75.0, 100.0];

    let mut min_working = None;

    for &mult in &multipliers {
        let temp_size = ((csr_estimate as f64 * mult) as usize).max(1024);
        let output_size = ((csr_estimate as f64 * mult) as usize).max(1024);

        let result = try_render(dag, temp_size, output_size);

        let status = if result { "✓" } else { "✗" };
        println!(
            "  {:.0}x ({} KB total): {}",
            mult,
            (temp_size + output_size) / 1024,
            status
        );

        if result && min_working.is_none() {
            min_working = Some((mult, temp_size + output_size));
        }
    }

    if let Some((mult, size)) = min_working {
        println!("  → First success at {:.0}x CSR: {} KB", mult, size / 1024);
    } else {
        println!("  → FAILED at 100x! Graph may require special handling.");
    }
}

#[allow(dead_code)]
fn binary_search_minimum(dag: &DAG, csr_estimate: usize) -> usize {
    let mut low = 1024usize;
    let mut high = csr_estimate * 6;

    while high - low > 1024 {
        let mid = (low + high) / 2;
        if try_render(dag, mid, mid) {
            high = mid;
        } else {
            low = mid;
        }
    }

    high * 2 // temp + output
}

fn try_render(dag: &DAG, temp_size: usize, output_size: usize) -> bool {
    let mut temp_buffer = vec![0u8; temp_size];
    let mut output_buffer = vec![0u8; output_size];

    let mut temp_arena = Arena::new(&mut temp_buffer);
    let mut output_arena = Arena::new(&mut output_buffer);

    if let Some(layout) = dag.compute_layout_arena(&mut temp_arena, &mut output_arena) {
        if layout.is_empty() && dag.node_count() > 0 {
            // Debug: print why empty
            // println!("      (empty layout for non-empty graph)");
            return false; // Layout returned empty for non-empty graph
        }

        // Try to render too
        let render_size = layout.estimate_render_size();
        let mut render_buffer = vec![0u8; render_size + 1024];
        let mut line_buffer = vec![' '; layout.width() + 16];

        layout
            .render_to_buffer(&mut render_buffer, &mut line_buffer)
            .is_some()
    } else {
        // Debug: why None?
        // println!("      (compute_layout_arena returned None)");
        false
    }
}

fn crash_test() {
    let dag = build_chain(100);

    // Test with absurdly small memory
    let tiny_sizes = [64, 128, 256, 512, 1024, 2048, 4096];

    for &size in &tiny_sizes {
        let mut temp_buffer = vec![0u8; size];
        let mut output_buffer = vec![0u8; size];

        let mut temp_arena = Arena::new(&mut temp_buffer);
        let mut output_arena = Arena::new(&mut output_buffer);

        let result = dag.compute_layout_arena(&mut temp_arena, &mut output_arena);

        match result {
            Some(layout) if !layout.is_empty() => {
                println!("  {} bytes: ✓ Success (unexpected!)", size * 2);
            }
            Some(_) => {
                println!("  {} bytes: ⚠ Empty layout returned", size * 2);
            }
            None => {
                println!("  {} bytes: ✗ Graceful failure (None returned)", size * 2);
            }
        }
    }

    println!("\n✓ No crashes! Arena handles insufficient memory gracefully.");
}

fn build_chain(n: usize) -> DAG<'static> {
    let mut dag = DAG::new();
    for i in 0..n {
        dag.add_node(i, Box::leak(format!("N{}", i).into_boxed_str()));
        if i > 0 {
            dag.add_edge(i - 1, i, None);
        }
    }
    dag
}

fn build_diamond(layers: usize) -> DAG<'static> {
    let mut dag = DAG::new();
    let mut id = 0;

    // Build diamond pattern - each layer has `layer` nodes
    for layer in 0..layers {
        let nodes_in_layer = (layer + 1).min(layers - layer);
        for _ in 0..nodes_in_layer {
            dag.add_node(id, "♦");
            id += 1;
        }
    }

    // Add edges between adjacent layers
    let mut prev_start = 0;
    let mut prev_count = 1;
    let mut curr_start = 1;

    for layer in 1..layers {
        let curr_count = (layer + 1).min(layers - layer);

        for i in 0..prev_count {
            for j in 0..curr_count.min(3) {
                let from = prev_start + i;
                let to = curr_start + (i + j).min(curr_count - 1);
                if from < id && to < id && from != to {
                    dag.add_edge(from, to, None);
                }
            }
        }

        prev_start = curr_start;
        prev_count = curr_count;
        curr_start += curr_count;
    }

    dag
}

fn build_fan(width: usize) -> DAG<'static> {
    let mut dag = DAG::new();
    dag.add_node(0, "Source");
    dag.add_node(1, "Sink");

    for i in 0..width {
        dag.add_node(i + 2, Box::leak(format!("W{}", i).into_boxed_str()));
        dag.add_edge(0, i + 2, None);
        dag.add_edge(i + 2, 1, None);
    }

    dag
}

// Helper trait to get node count
trait NodeCount {
    fn node_count(&self) -> usize;
}

impl NodeCount for DAG<'_> {
    fn node_count(&self) -> usize {
        // Use estimate_size as proxy - roughly 20 bytes per node
        self.estimate_size() / 20
    }
}

fn debug_100_node_chain() {
    let dag = build_chain(100);
    let csr_estimate = dag.estimate_csr_arena_size();

    println!("CSR estimate: {} bytes", csr_estimate);

    // Try different sizes to pinpoint the failure
    for size_kb in [100, 200, 300, 400, 500, 600, 700, 800, 900, 1000] {
        let size = size_kb * 1024;
        let mut temp_buffer = vec![0u8; size];
        let mut output_buffer = vec![0u8; size];

        let mut temp_arena = Arena::new(&mut temp_buffer);
        let mut output_arena = Arena::new(&mut output_buffer);

        let result = match dag.compute_layout_arena(&mut temp_arena, &mut output_arena) {
            Some(layout) if !layout.is_empty() => {
                let render_size = layout.estimate_render_size();
                let mut render_buffer = vec![0u8; render_size + 1024];
                let mut line_buffer = vec![' '; layout.width() + 16];
                layout
                    .render_to_buffer(&mut render_buffer, &mut line_buffer)
                    .is_some()
            }
            _ => false,
        };

        println!(
            "  {} KB each (total {} KB): {}",
            size_kb,
            size_kb * 2,
            if result { "✓" } else { "✗" }
        );
    }
}
