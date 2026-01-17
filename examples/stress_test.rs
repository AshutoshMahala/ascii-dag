use ascii_dag::arena::Arena;
use ascii_dag::graph::DAG;
use std::time::Instant;

// Simple Linear Congruential Generator to avoid adding 'rand' dependency
struct SimpleRng {
    state: u64,
}

impl SimpleRng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_mul(6364136223846793005).wrapping_add(1);
        self.state
    }

    fn gen_range(&mut self, min: usize, max: usize) -> usize {
        let range = (max - min) as u64;
        let random = self.next_u64();
        min + (random % range) as usize
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let use_arena = args.iter().any(|a| a == "--arena");
    let low_mem = args.iter().any(|a| a == "--low-mem");

    if use_arena {
        println!("=== ASCII DAG Stress Test Suite (ARENA MODE) ===\n");
    } else {
        println!("=== ASCII DAG Stress Test Suite (HEAP MODE) ===\n");
    }

    if low_mem {
        println!(">>> LOW MEMORY MODE ENABLED (Fixed 10MB Budget) <<<\n");
    }

    let tests = [
        (
            "The Double Helix",
            test_double_helix as fn() -> DAG<'static>,
        ),
        ("The Skyscraper", test_skyscraper),
        ("The Wide Fan", test_wide_fan),
        ("The Diamond Lattice", test_diamond_lattice),
        ("The Disconnected Islands", test_disconnected_islands),
        ("The Random Hairball", test_random_hairball),
        ("The Skip-Level Nightmare", test_skip_level_nightmare),
        ("The Verbose Logger", test_verbose_logger),
        ("The Ouroboros", test_ouroboros),
        ("Massive Diamond (50k)", test_massive_diamond_50k),
        ("Massive Fan (50k)", test_massive_fan_50k),
        // ("Massive Diamond (100k)", test_massive_diamond_100k), // Disabled: takes very long
    ];

    for (name, test_fn) in tests {
        println!("\n>>> RUNNING: {} <<<\n", name);
        let dag = test_fn();

        if use_arena {
            run_arena_test(name, &dag, low_mem);
        } else {
            run_heap_test(name, &dag);
        }

        println!("------------------------------------------------------------");
    }

    if !use_arena {
        println!("\nTip: Run with --arena flag to test arena mode:");
        println!("  cargo run --example stress_test --release -- --arena");
    }
}

fn run_heap_test(name: &str, dag: &DAG) {
    let start = Instant::now();
    let output = dag.render();
    let duration = start.elapsed();

    if name.contains("Massive") {
        println!("(Output suppressed. Length: {} chars)", output.len());
        let size_mb = output.len() as f64 / 1024.0 / 1024.0;
        println!(">>> Approx Output RAM: {:.2} MB <<<", size_mb);
    } else {
        println!("{}", output);
    }
    println!(">>> [HEAP] Rendered in {:?} <<<\n", duration);
}

fn run_arena_test(name: &str, dag: &DAG, low_mem: bool) {
    // Check for cycles first
    if dag.has_cycle() {
        println!("(Graph has cycles - skipping layout)");
        return;
    }

    // Use estimate_csr_arena_size as a reasonable base for layout computation
    let csr_estimate = dag.estimate_csr_arena_size();

    // Determine memory budget
    let (temp_arena_size, output_arena_size) = if low_mem && name.contains("Massive") {
        if name.contains("100k") {
            // 100MB + 100MB = 200MB for 100k nodes
            (100 * 1024 * 1024, 100 * 1024 * 1024)
        } else {
            // 25MB + 25MB = 50MB for 50k nodes
            (25 * 1024 * 1024, 25 * 1024 * 1024)
        }
    } else {
        // Normal mode: adaptive sizing
        let min_arena_size = 128 * 1024; // 128 KB minimum
        let size = (csr_estimate * 5).max(min_arena_size);
        (size, size)
    };

    let mut temp_buffer = vec![0u8; temp_arena_size];
    let mut output_buffer = vec![0u8; output_arena_size];

    // Render buffer - estimate based on output size
    let estimated_render = dag.estimate_size();
    let render_size = estimated_render + 65536;
    let mut render_buffer = vec![0u8; render_size];

    // Line buffer for scanline rendering (max width from estimate)
    // Width is roughly sqrt(estimated_size) for typical graphs
    let line_buffer_size = (estimated_render as f64).sqrt() as usize + 1024;
    let mut line_buffer = vec![' '; line_buffer_size.max(1024)];

    let start = Instant::now();

    // Compute layout using arena
    let mut temp_arena = Arena::new(&mut temp_buffer);
    let mut output_arena = Arena::new(&mut output_buffer);

    let output_len =
        if let Some(layout) = dag.compute_layout_arena(&mut temp_arena, &mut output_arena) {
            if layout.is_empty() {
                println!("(Layout returned empty)");
                0
            } else {
                // Render to ASCII art
                let bytes_written = layout
                    .render_to_buffer(&mut render_buffer, &mut line_buffer)
                    .unwrap_or(0);

                if !name.contains("Massive") {
                    // Print output for non-massive tests
                    if let Ok(s) = std::str::from_utf8(&render_buffer[..bytes_written]) {
                        println!("{}", s);
                    }
                }
                bytes_written
            }
        } else {
            println!("(Failed to compute layout - arena may be too small)");
            println!(
                "  Temp arena: {} KB, Output arena: {} KB",
                temp_arena_size / 1024,
                output_arena_size / 1024
            );
            0
        };

    let duration = start.elapsed();

    if name.contains("Massive") {
        println!("(Output suppressed. Length: {} bytes)", output_len);
        // Show allocated sizes
        let allocated_kb = (temp_arena_size + output_arena_size) as f64 / 1024.0;
        println!(
            ">>> Allocated: {:.1} KB (temp: {:.1} KB, output: {:.1} KB) <<<",
            allocated_kb,
            temp_arena_size as f64 / 1024.0,
            output_arena_size as f64 / 1024.0
        );
    }
    println!(">>> [ARENA] Layout+Rendered in {:?} <<<\n", duration);
}

fn test_double_helix() -> DAG<'static> {
    let mut dag = DAG::new();
    // Two intertwined chains
    for i in 0..10 {
        dag.add_node(i * 2, "A");
        dag.add_node(i * 2 + 1, "B");

        if i > 0 {
            dag.add_edge((i - 1) * 2, i * 2); // A -> A
            dag.add_edge((i - 1) * 2 + 1, i * 2 + 1); // B -> B

            // Cross connections
            if i % 2 == 0 {
                dag.add_edge((i - 1) * 2, i * 2 + 1); // A -> B
                dag.add_edge((i - 1) * 2 + 1, i * 2); // B -> A
            }
        }
    }
    dag
}

fn test_skyscraper() -> DAG<'static> {
    let mut dag = DAG::new();
    // Very deep, narrow graph to test vertical spacing
    for i in 0..50 {
        dag.add_node(i, Box::leak(format!("Floor {}", i).into_boxed_str()));
        if i > 0 {
            dag.add_edge(i - 1, i);
        }
    }
    dag
}

fn test_wide_fan() -> DAG<'static> {
    let mut dag = DAG::new();
    dag.add_node(0, "Source");
    dag.add_node(1000, "Sink");

    // 50 parallel nodes
    for i in 1..51 {
        dag.add_node(i, Box::leak(format!("Worker {}", i).into_boxed_str()));
        dag.add_edge(0, i);
        dag.add_edge(i, 1000);
    }
    dag
}

fn test_diamond_lattice() -> DAG<'static> {
    let mut dag = DAG::new();
    let width = 5;
    let height = 10;

    for y in 0..height {
        for x in 0..width {
            let id = y * width + x;
            dag.add_node(id, "♦");

            if y > 0 {
                // Connect to parents
                let p_id = (y - 1) * width + x;
                dag.add_edge(p_id, id);

                // Cross connections
                if x > 0 {
                    dag.add_edge((y - 1) * width + (x - 1), id);
                }
                if x < width - 1 {
                    dag.add_edge((y - 1) * width + (x + 1), id);
                }
            }
        }
    }
    dag
}

fn test_disconnected_islands() -> DAG<'static> {
    let mut dag = DAG::new();

    // Create 5 separate small graphs
    for island in 0..5 {
        let base = island * 10;
        dag.add_node(base, "Island");
        dag.add_node(base + 1, "Palm");
        dag.add_node(base + 2, "Coconuts");

        dag.add_edge(base, base + 1);
        dag.add_edge(base + 1, base + 2);
    }
    dag
}

fn test_random_hairball() -> DAG<'static> {
    let mut dag = DAG::new();
    let mut rng = SimpleRng::new(42);
    let nodes = 30;

    for i in 0..nodes {
        dag.add_node(i, Box::leak(format!("N{}", i).into_boxed_str()));
    }

    // Add random edges, ensuring no cycles (i < j rule)
    for i in 0..nodes {
        let num_edges = rng.gen_range(1, 4);
        for _ in 0..num_edges {
            let target = rng.gen_range(i + 1, nodes + 5); // +5 allows some out of bounds (ignored) or valid
            if target < nodes {
                dag.add_edge(i, target);
            }
        }
    }
    dag
}

fn test_skip_level_nightmare() -> DAG<'static> {
    let mut dag = DAG::new();
    // Root
    dag.add_node(0, "Root");

    // Levels 1, 2, 3, 4, 5
    for i in 1..6 {
        dag.add_node(i, Box::leak(format!("L{}", i).into_boxed_str()));
        dag.add_edge(i - 1, i); // Normal connection
    }

    // Skip edges: 0->2, 0->3, 0->4, 0->5
    for i in 2..6 {
        dag.add_edge(0, i);
    }

    // Nested skips: 1->3, 2->5
    dag.add_edge(1, 3);
    dag.add_edge(2, 5);

    dag
}

fn test_verbose_logger() -> DAG<'static> {
    let mut dag = DAG::new();
    dag.add_node(1, "A");
    dag.add_node(2, "B");
    // Long text to test centering
    dag.add_node(
        3,
        Box::leak(
            "Error: NullPointerException at line 55 (Critical Failure in Module X)"
                .to_string()
                .into_boxed_str(),
        ),
    );
    dag.add_node(4, "C");

    dag.add_edge(1, 2);
    dag.add_edge(1, 3);
    dag.add_edge(3, 4);
    dag
}

fn test_ouroboros() -> DAG<'static> {
    let mut dag = DAG::new();
    dag.add_node(1, "Head");
    dag.add_node(2, "Body");
    dag.add_node(3, "Tail");

    dag.add_edge(1, 2);
    dag.add_edge(2, 3);
    dag.add_edge(3, 1); // Cycle!
    dag
}

fn test_massive_diamond_50k() -> DAG<'static> {
    let mut dag = DAG::new();
    let width = 224;
    let height = 224; // ~50k nodes

    for y in 0..height {
        for x in 0..width {
            let id = y * width + x;
            dag.add_node(id, ".");
            if y < height - 1 {
                let next_y_base = (y + 1) * width;
                dag.add_edge(id, next_y_base + x);
                if x < width - 1 {
                    dag.add_edge(id, next_y_base + (x + 1));
                }
            }
        }
    }
    dag
}

fn test_massive_diamond_100k() -> DAG<'static> {
    let mut dag = DAG::new();
    let width = 316; // 316*316 ≈ 99,856 nodes
    let height = 316;

    for y in 0..height {
        for x in 0..width {
            let id = y * width + x;
            dag.add_node(id, ".");
            if y < height - 1 {
                let next_y_base = (y + 1) * width;
                dag.add_edge(id, next_y_base + x);
                if x < width - 1 {
                    dag.add_edge(id, next_y_base + (x + 1));
                }
            }
        }
    }
    dag
}

fn test_massive_fan_50k() -> DAG<'static> {
    let mut dag = DAG::new();
    let root = 0;
    let sink = 50001;
    dag.add_node(root, "S");
    dag.add_node(sink, "E");
    for i in 1..=50000 {
        dag.add_node(i, ".");
        dag.add_edge(root, i);
        dag.add_edge(i, sink);
    }
    dag
}
