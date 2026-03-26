use ascii_dag::LayoutConfig;
use ascii_dag::graph::Graph;
use ascii_dag::render::colors::Palette;
use std::time::Instant;

#[cfg(feature = "arena")]
use ascii_dag::graph::arena::Arena;

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
    let use_csr = args.iter().any(|a| a == "--csr");
    let preset_name = args
        .iter()
        .position(|a| a == "--preset")
        .and_then(|i| args.get(i + 1))
        .map(|s| s.as_str());

    if use_csr {
        println!("=== ASCII DAG Stress Test Suite (CSR MODE — true no-alloc pipeline) ===\n");
    } else {
        println!("=== ASCII DAG Stress Test Suite (HEAP MODE) ===\n");
    }

    if let Some(p) = preset_name {
        println!(">>> PRESET: {} <<<\n", p.to_uppercase());
    }

    let tests = [
        (
            "The Double Helix",
            test_double_helix as fn() -> Graph<'static>,
        ),
        ("The Skyscraper", test_skyscraper),
        ("The Wide Fan", test_wide_fan),
        ("The Diamond Lattice", test_diamond_lattice),
        ("The Disconnected Islands", test_disconnected_islands),
        ("The Random Hairball", test_random_hairball),
        ("The Skip-Level Nightmare", test_skip_level_nightmare),
        ("The Verbose Logger", test_verbose_logger),
        ("The Ouroboros", test_ouroboros),
        ("Cycle Breaking Demo", test_cycle_breaking_demo),
        ("Massive Diamond (20k)", test_massive_diamond_20k),
        ("Massive Diamond (50k)", test_massive_diamond_50k),
        ("Massive Fan (50k)", test_massive_fan_50k),
        // ("Massive Diamond (100k)", test_massive_diamond_100k), // Disabled: takes very long
    ];

    for (name, test_fn) in tests {
        println!("\n>>> RUNNING: {} <<<\n", name);
        let dag = test_fn();

        if use_csr {
            #[cfg(feature = "arena")]
            run_csr_test(name, &dag);
            #[cfg(not(feature = "arena"))]
            {
                let _ = (name, &dag);
                println!("(arena feature not enabled — skipping)");
            }
        } else {
            run_heap_test(name, &dag, preset_name);
        }

        println!("------------------------------------------------------------");
    }

    if !use_csr {
        println!("\nTip: Run with --csr flag for the true no-alloc CSR pipeline:");
        println!("  cargo run --example stress_test --release --features arena -- --csr");
        println!("\nTip: Run with presets:");
        println!("  cargo run --example stress_test --release -- --preset fast");
        println!("  cargo run --example stress_test --release -- --preset quality");
    }
}

fn run_heap_test(name: &str, dag: &Graph, preset_name: Option<&str>) {
    let start = Instant::now();

    // Build layout config from preset
    let config = match preset_name {
        Some("fast") => LayoutConfig::fast(),
        Some("quality") => LayoutConfig::quality(),
        _ => LayoutConfig::standard(), // default
    };

    // Use colored rendering for Helix, Hairball, and cycle-related tests
    let use_color = name.contains("Helix")
        || name.contains("Hairball")
        || name.contains("Nightmare")
        || name.contains("Cycle")
        || name.contains("Ouroboros");

    // Use the new preset API: compute_layout_with_config()
    let ir = dag.compute_layout_with_config(&config);

    let output = if use_color {
        ir.render_scanline_colored_with_legend(Palette::Ansi)
    } else {
        ir.render_scanline()
    };
    let duration = start.elapsed();

    // Show reversed edge info for cyclic graphs
    let reversed_count = ir.edges().iter().filter(|e| e.reversed).count();
    if reversed_count > 0 {
        println!(
            "  [Cycle breaking: {} reversed edge(s) rendered with dashed lines]\n",
            reversed_count
        );
    }

    if name.contains("Massive") {
        println!("(Output suppressed. Length: {} chars)", output.len());
        let size_mb = output.len() as f64 / 1024.0 / 1024.0;
        println!(">>> Approx Output RAM: {:.2} MB <<<", size_mb);
    } else {
        println!("{}", output);
    }
    println!(">>> [HEAP] Rendered in {:?} <<<\n", duration);
}

/// True no-alloc CSR pipeline: Graph → to_csr → compute_layout_arena_csr → render_to_buffer.
/// The only heap allocations are the pre-sized Vec<u8> buffers (which on embedded would be static).
#[cfg(feature = "arena")]
fn run_csr_test(name: &str, dag: &Graph) {
    if dag.has_cycle() {
        println!("(Graph has cycles - CSR layout does not yet support cycle breaking, skipping)");
        return;
    }

    // Step 1: Convert Graph → CsrGraph (via arena).
    // On embedded, you'd build CsrGraph directly via CsrGraphBuilder — no Graph involved.
    let csr_arena_size = dag.estimate_csr_arena_size() * 2; // 2x margin for alignment
    let mut csr_buffer = vec![0u8; csr_arena_size];
    let mut csr_arena = Arena::new(&mut csr_buffer);

    let csr_graph = match dag.to_csr(&mut csr_arena) {
        Some(g) => g,
        None => {
            println!(
                "(Failed to convert Graph → CsrGraph, arena too small: {} KB)",
                csr_arena_size / 1024
            );
            return;
        }
    };

    // Step 2: Layout config
    let config = LayoutConfig::standard();

    // Step 3: Layout arenas
    let layout_estimate = dag.estimate_layout_arena_size();
    let arena_size = ((layout_estimate * 6) / 5).max(128 * 1024);
    let mut temp_buffer = vec![0u8; arena_size];
    let mut output_buffer = vec![0u8; arena_size];

    let start = Instant::now();

    let mut temp_arena = Arena::new(&mut temp_buffer);
    let mut output_arena = Arena::new(&mut output_buffer);

    // Step 4: CsrGraph → compute_layout_arena (the real no-alloc layout)
    let output_len =
        match csr_graph.compute_layout_arena(&config, &mut temp_arena, &mut output_arena) {
            Ok(layout) => {
                if layout.is_empty() {
                    println!("(Layout returned empty)");
                    0
                } else {
                    // Step 5: Render to buffer (no-alloc rendering)
                    let (render_est, scratch_len) = layout.estimate_render_size();
                    let render_size = render_est + 65536;
                    let line_buffer_size = (layout.width() + 1024).max(1024);
                    let mut render_buffer = vec![0u8; render_size];
                    let mut line_buffer = vec![' '; line_buffer_size];
                    let mut scratch_buffer = vec![0usize; scratch_len + 1024];

                    let bytes_written = layout
                        .render_to_buffer(&mut render_buffer, &mut line_buffer, &mut scratch_buffer)
                        .unwrap_or(0);

                    if !name.contains("Massive") {
                        if let Ok(s) = std::str::from_utf8(&render_buffer[..bytes_written]) {
                            println!("{}", s);
                        }
                    } else {
                        println!("(Output suppressed. Length: {} bytes)", bytes_written);
                    }
                    bytes_written
                }
            }
            Err(e) => {
                println!(
                    "(CSR layout failed: {:?}, arena size: {} KB)",
                    e,
                    arena_size / 1024
                );
                0
            }
        };

    let duration = start.elapsed();
    let total_kb = (csr_arena_size + arena_size * 2) as f64 / 1024.0;
    println!(
        ">>> [CSR] Layout+Rendered in {:?} (total buffers: {:.1} KB, output: {} bytes) <<<\n",
        duration, total_kb, output_len
    );
}

fn test_double_helix() -> Graph<'static> {
    let mut dag = Graph::new();
    // Two intertwined chains
    for i in 0..10 {
        dag.add_node(i * 2, "A");
        dag.add_node(i * 2 + 1, "B");

        if i > 0 {
            dag.add_edge((i - 1) * 2, i * 2, None); // A -> A
            dag.add_edge((i - 1) * 2 + 1, i * 2 + 1, None); // B -> B

            // Cross connections
            if i % 2 == 0 {
                dag.add_edge((i - 1) * 2, i * 2 + 1, None); // A -> B
                dag.add_edge((i - 1) * 2 + 1, i * 2, None); // B -> A
            }
        }
    }
    dag
}

fn test_skyscraper() -> Graph<'static> {
    let mut dag = Graph::new();
    // Very deep, narrow graph to test vertical spacing
    for i in 0..50 {
        dag.add_node(i, Box::leak(format!("Floor {}", i).into_boxed_str()));
        if i > 0 {
            dag.add_edge(i - 1, i, None);
        }
    }
    dag
}

fn test_wide_fan() -> Graph<'static> {
    let mut dag = Graph::new();
    dag.add_node(0, "Source");
    dag.add_node(1000, "Sink");

    // 50 parallel nodes
    for i in 1..51 {
        dag.add_node(i, Box::leak(format!("Worker {}", i).into_boxed_str()));
        dag.add_edge(0, i, None);
        dag.add_edge(i, 1000, None);
    }
    dag
}

fn test_diamond_lattice() -> Graph<'static> {
    let mut dag = Graph::new();
    let width = 5;
    let height = 10;

    for y in 0..height {
        for x in 0..width {
            let id = y * width + x;
            dag.add_node(id, "♦");

            if y > 0 {
                // Connect to parents
                let p_id = (y - 1) * width + x;
                dag.add_edge(p_id, id, None);

                // Cross connections
                if x > 0 {
                    dag.add_edge((y - 1) * width + (x - 1), id, None);
                }
                if x < width - 1 {
                    dag.add_edge((y - 1) * width + (x + 1), id, None);
                }
            }
        }
    }
    dag
}

fn test_disconnected_islands() -> Graph<'static> {
    let mut dag = Graph::new();

    // Create 5 separate small graphs
    for island in 0..5 {
        let base = island * 10;
        dag.add_node(base, "Island");
        dag.add_node(base + 1, "Palm");
        dag.add_node(base + 2, "Coconuts");

        dag.add_edge(base, base + 1, None);
        dag.add_edge(base + 1, base + 2, None);
    }
    dag
}

fn test_random_hairball() -> Graph<'static> {
    let mut dag = Graph::new();
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
                dag.add_edge(i, target, None);
            }
        }
    }
    dag
}

fn test_skip_level_nightmare() -> Graph<'static> {
    let mut dag = Graph::new();
    // Root
    dag.add_node(0, "Root");

    // Levels 1, 2, 3, 4, 5
    for i in 1..6 {
        dag.add_node(i, Box::leak(format!("L{}", i).into_boxed_str()));
        dag.add_edge(i - 1, i, None); // Normal connection
    }

    // Skip edges: 0->2, 0->3, 0->4, 0->5
    for i in 2..6 {
        dag.add_edge(0, i, None);
    }

    // Nested skips: 1->3, 2->5
    dag.add_edge(1, 3, None);
    dag.add_edge(2, 5, None);

    dag
}

fn test_verbose_logger() -> Graph<'static> {
    let mut dag = Graph::new();
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

    dag.add_edge(1, 2, None);
    dag.add_edge(1, 3, None);
    dag.add_edge(3, 4, None);
    dag
}

fn test_ouroboros() -> Graph<'static> {
    let mut dag = Graph::new();
    dag.add_node(1, "Head");
    dag.add_node(2, "Body");
    dag.add_node(3, "Tail");

    dag.add_edge(1, 2, None);
    dag.add_edge(2, 3, None);
    dag.add_edge(3, 1, None); // Cycle!
    dag
}

/// Demonstrates cycle breaking with dashed reversed edges.
///
/// This graph has back edges that form cycles. The layout algorithm detects
/// them, temporarily reverses them for layering, then renders them with
/// dashed lines (┊ ⇣) to visually distinguish from normal edges.
fn test_cycle_breaking_demo() -> Graph<'static> {
    let mut dag = Graph::new();

    // A build system with feedback loops:
    //   compile → link → test → deploy
    //              ↑              │
    //              └──────────────┘  (back edge: deploy triggers recompile)
    //   Also: test → compile (back edge: test failure triggers recompile)
    dag.add_node(10, "compile");
    dag.add_node(20, "link");
    dag.add_node(30, "test");
    dag.add_node(40, "deploy");

    dag.add_edge(10, 20, None); // compile → link
    dag.add_edge(20, 30, None); // link → test
    dag.add_edge(30, 40, None); // test → deploy
    dag.add_edge(40, 20, None); // deploy → link (BACK EDGE)
    dag.add_edge(30, 10, None); // test → compile (BACK EDGE)

    // Self-loop: metrics reports itself
    dag.add_node(50, "metrics");
    dag.add_edge(10, 50, None);
    dag.add_edge(50, 50, None); // self-loop (BACK EDGE)

    dag
}

fn test_massive_diamond_20k() -> Graph<'static> {
    let mut dag = Graph::new();
    let width = 142;
    let height = 142; // ~20k nodes

    for y in 0..height {
        for x in 0..width {
            let id = y * width + x;
            dag.add_node(id, ".");
            if y < height - 1 {
                let next_y_base = (y + 1) * width;
                dag.add_edge(id, next_y_base + x, None);
                if x < width - 1 {
                    dag.add_edge(id, next_y_base + (x + 1), None);
                }
            }
        }
    }
    dag
}

fn test_massive_diamond_50k() -> Graph<'static> {
    let mut dag = Graph::new();
    let width = 224;
    let height = 224; // ~50k nodes

    for y in 0..height {
        for x in 0..width {
            let id = y * width + x;
            dag.add_node(id, ".");
            if y < height - 1 {
                let next_y_base = (y + 1) * width;
                dag.add_edge(id, next_y_base + x, None);
                if x < width - 1 {
                    dag.add_edge(id, next_y_base + (x + 1), None);
                }
            }
        }
    }
    dag
}

#[allow(dead_code)]
fn test_massive_diamond_100k() -> Graph<'static> {
    let mut dag = Graph::new();
    let width = 316; // 316*316 ≈ 99,856 nodes
    let height = 316;

    for y in 0..height {
        for x in 0..width {
            let id = y * width + x;
            dag.add_node(id, ".");
            if y < height - 1 {
                let next_y_base = (y + 1) * width;
                dag.add_edge(id, next_y_base + x, None);
                if x < width - 1 {
                    dag.add_edge(id, next_y_base + (x + 1), None);
                }
            }
        }
    }
    dag
}

fn test_massive_fan_50k() -> Graph<'static> {
    let mut dag = Graph::new();
    let root = 0;
    let sink = 50001;
    dag.add_node(root, "S");
    dag.add_node(sink, "E");
    for i in 1..=50000 {
        dag.add_node(i, ".");
        dag.add_edge(root, i, None);
        dag.add_edge(i, sink, None);
    }
    dag
}
