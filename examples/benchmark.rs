use ascii_dag::arena::Arena;
use ascii_dag::csr::CsrGraphBuilder;
use ascii_dag::graph::DAG;
use std::io::{self, Write};
use std::time::Instant;

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

    fn chance(&mut self, p_true: usize) -> bool {
        self.gen_range(0, 100) < p_true
    }
}

fn generate_graph_data(node_count: usize) -> (Vec<(usize, String)>, Vec<(usize, usize)>) {
    let mut rng = SimpleRng::new(12345);
    let mut nodes = Vec::with_capacity(node_count);
    let mut edges = Vec::with_capacity(node_count * 2);

    for i in 0..node_count {
        nodes.push((i, format!("N{}", i)));
    }

    for i in 0..node_count.saturating_sub(1) {
        let jump = rng.gen_range(1, 5.min(node_count - i));
        edges.push((i, i + jump));

        for _ in 0..2 {
            if rng.chance(40) {
                let target_jump = rng.gen_range(1, 20.min(node_count - i));
                edges.push((i, i + target_jump));
            }
        }
    }
    (nodes, edges)
}

fn run_comparison(count: usize) {
    println!("Generating data for {} nodes...", count);
    io::stdout().flush().unwrap();
    let (nodes, edges) = generate_graph_data(count);

    // --- HEAP BENCHMARK ---
    print!("Running HEAP...");
    io::stdout().flush().unwrap();
    let heap_total_us;
    {
        let start = Instant::now();

        // 1. Build
        let build_start = Instant::now();
        let node_refs: Vec<(usize, &str)> = nodes.iter().map(|(id, s)| (*id, s.as_str())).collect();
        let dag = DAG::from_edges(&node_refs, &edges);
        let build_time = build_start.elapsed();

        // 2. Render
        let render_start = Instant::now();
        let mut output = String::with_capacity(count * 100);
        dag.render_to(&mut output);
        let render_time = render_start.elapsed();

        heap_total_us = start.elapsed().as_micros();

        println!(" Done.");
        print!(
            "| {:<4} | {:<5} | {:>8} | {:>8} | {:>8} |",
            count,
            "HEAP",
            format!("{:.1}ms", build_time.as_micros() as f64 / 1000.0),
            format!("{:.1}ms", render_time.as_micros() as f64 / 1000.0),
            format!("{:.1}ms", heap_total_us as f64 / 1000.0)
        );
    }

    println!();

    // --- ARENA BENCHMARK ---
    {
        print!("Running ARENA...");
        io::stdout().flush().unwrap();

        // Allocate Memory Buffers
        let mut graph_mem = vec![0u8; 1 * 1024 * 1024];
        let mut temp_mem = vec![0u8; 5 * 1024 * 1024];
        let mut output_mem = vec![0u8; 5 * 1024 * 1024];
        println!("  Allocated.");
        io::stdout().flush().unwrap();

        let start = Instant::now();

        // 1. Build
        println!("  Building Arena...");
        io::stdout().flush().unwrap();
        let build_start = Instant::now();
        let mut graph_arena = Arena::new(&mut graph_mem);

        let label_bytes = nodes.iter().map(|(_, l)| l.len()).sum::<usize>() + 256;

        let mut builder =
            CsrGraphBuilder::new(&mut graph_arena, nodes.len(), edges.len(), label_bytes)
                .expect("Failed to create CsrGraphBuilder");

        for (id, label) in &nodes {
            builder.add_node(*id, label);
        }
        for (u, v) in &edges {
            builder.add_edge(*u, *v);
        }

        let graph = builder.build().expect("Failed to build graph");
        let build_time = build_start.elapsed();
        println!("  Build Done.");
        io::stdout().flush().unwrap();

        // 2. Compute Layout
        println!("  Computing Layout...");
        io::stdout().flush().unwrap();
        let compute_start = Instant::now();
        let mut temp_arena = Arena::new(&mut temp_mem);
        let mut final_arena = Arena::new(&mut output_mem);

        let layout = graph
            .compute_layout_arena(&mut temp_arena, &mut final_arena)
            .expect("Layout computation failed (None returned)");
        let compute_time = compute_start.elapsed();
        println!("  Layout Done.");
        io::stdout().flush().unwrap();

        // 3. Render
        println!("  Rendering Arena...");
        io::stdout().flush().unwrap();
        let render_start = Instant::now();
        let mut render_buf = vec![0u8; count * 500];
        let mut line_buf = vec![' '; 1024];
        layout
            .render_to_buffer(&mut render_buf, &mut line_buf)
            .unwrap();
        let render_time = render_start.elapsed();

        let arena_total_us = start.elapsed().as_micros();
        let speedup = heap_total_us as f64 / arena_total_us as f64;

        println!(" Done.");
        print!(
            "| {:<4} | {:<5} | {:>8} | {:>8} | {:>8} | x{:.2}",
            "",
            "ARENA",
            format!("{:.1}ms", build_time.as_micros() as f64 / 1000.0),
            format!("{:.1}ms", compute_time.as_micros() as f64 / 1000.0),
            format!("{:.1}ms", arena_total_us as f64 / 1000.0),
            speedup
        );
    }
    println!("\n-------------------------------------------------------------");
}

fn main() {
    println!("\n=== Desktop Benchmark: Heap vs Arena ===\n");
    println!(
        "| {:<4} | {:<5} | {:>8} | {:>8} | {:>8} | Speedup",
        "Node", "Mode", "Build", "Compute", "Total"
    );
    println!("|------|-------|----------|----------|----------|--------");
    io::stdout().flush().unwrap();

    let sizes = [50];

    for &size in &sizes {
        run_comparison(size);
    }
}
