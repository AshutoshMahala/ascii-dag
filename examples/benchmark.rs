use ascii_dag::arena::Arena;
use ascii_dag::csr::CsrGraphBuilder;
use ascii_dag::graph::DAG;
use std::io::{self, Write};
use std::time::Instant;

/// Graph topology for benchmarking
#[derive(Clone, Copy)]
enum Topology {
    Chain,   // Simple chain: 0 → 1 → 2 → ... → N
    Diamond, // Diamond lattice: worst case for skip-level edges
    WideFan, // Fan-out then fan-in: worst case for crossing reduction
}

impl Topology {
    fn name(&self) -> &'static str {
        match self {
            Topology::Chain => "Chain",
            Topology::Diamond => "Diamond",
            Topology::WideFan => "WideFan",
        }
    }
}

type GraphData = (Vec<(usize, String)>, Vec<(usize, usize)>);

fn generate_chain(n: usize) -> GraphData {
    let nodes: Vec<_> = (0..n).map(|i| (i, format!("N{}", i))).collect();
    let edges: Vec<_> = (0..n - 1).map(|i| (i, i + 1)).collect();
    (nodes, edges)
}

fn generate_diamond(n: usize) -> GraphData {
    // Diamond lattice: each node connects to 2 nodes in next level
    // Creates many skip-level edges and crossing opportunities
    let nodes: Vec<_> = (0..n).map(|i| (i, format!("N{}", i))).collect();
    let mut edges = Vec::with_capacity(n * 2);

    for i in 0..n.saturating_sub(1) {
        edges.push((i, i + 1));
        if i + 2 < n {
            edges.push((i, i + 2)); // Skip-level edge
        }
    }
    (nodes, edges)
}

fn generate_wide_fan(n: usize) -> GraphData {
    // Fan-out from root, then fan-in to sink
    // Worst case for crossing reduction (all nodes at same level)
    let nodes: Vec<_> = (0..n).map(|i| (i, format!("N{}", i))).collect();
    let mut edges = Vec::with_capacity(n * 2);

    let root = 0;
    let sink = n - 1;
    let middle_count = n.saturating_sub(2);

    // Root fans out to all middle nodes
    for i in 1..=middle_count {
        edges.push((root, i));
    }
    // All middle nodes fan in to sink
    for i in 1..=middle_count {
        edges.push((i, sink));
    }
    (nodes, edges)
}

fn generate_graph(topology: Topology, n: usize) -> GraphData {
    match topology {
        Topology::Chain => generate_chain(n),
        Topology::Diamond => generate_diamond(n),
        Topology::WideFan => generate_wide_fan(n),
    }
}

fn run_comparison(topology: Topology, count: usize) {
    let (nodes, edges) = generate_graph(topology, count);

    // --- HEAP BENCHMARK ---
    let heap_total_us;
    let heap_build_us;
    let heap_compute_us;
    let heap_render_us;
    {
        let start = Instant::now();

        // 1. Build
        let build_start = Instant::now();
        let node_refs: Vec<(usize, &str)> = nodes.iter().map(|(id, s)| (*id, s.as_str())).collect();
        let dag = DAG::from_edges(&node_refs, &edges);
        heap_build_us = build_start.elapsed().as_micros();

        // 2. Compute Layout
        let compute_start = Instant::now();
        let ir = dag.compute_layout();
        heap_compute_us = compute_start.elapsed().as_micros();

        // 3. Render
        let render_start = Instant::now();
        let mut output = String::with_capacity(count * 100);
        ir.render_scanline_to(&mut output);
        heap_render_us = render_start.elapsed().as_micros();

        heap_total_us = start.elapsed().as_micros();
    }

    // --- ARENA BENCHMARK ---
    let arena_total_us;
    let arena_build_us;
    let arena_compute_us;
    let arena_render_us;
    {
        // Allocate Memory Buffers
        let mut graph_mem = vec![0u8; 2 * 1024 * 1024];
        let mut temp_mem = vec![0u8; 8 * 1024 * 1024];
        let mut output_mem = vec![0u8; 8 * 1024 * 1024];

        let start = Instant::now();

        // 1. Build
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
        arena_build_us = build_start.elapsed().as_micros();

        // 2. Compute Layout
        let compute_start = Instant::now();
        let mut temp_arena = Arena::new(&mut temp_mem);
        let mut final_arena = Arena::new(&mut output_mem);

        let layout = graph
            .compute_layout_arena(&mut temp_arena, &mut final_arena)
            .expect("Layout computation failed (None returned)");
        arena_compute_us = compute_start.elapsed().as_micros();

        // 3. Render
        let render_start = Instant::now();
        let mut render_buf = vec![0u8; count * 500 + 10000];
        let mut line_buf = vec![' '; 2048];
        let (_, scratch_len) = layout.estimate_render_size();
        let mut scratch_buf = vec![0usize; scratch_len + 1024];
        let _ = layout.render_to_buffer(&mut render_buf, &mut line_buf, &mut scratch_buf);
        arena_render_us = render_start.elapsed().as_micros();

        arena_total_us = start.elapsed().as_micros();
    }

    let speedup = heap_total_us as f64 / arena_total_us as f64;

    // Print Heap row
    println!(
        "| {:>8} | {:>5} | {:>5} | {:>8}µs | {:>8}µs | {:>8}µs | {:>10}µs |",
        topology.name(),
        count,
        "Heap",
        heap_build_us,
        heap_compute_us,
        heap_render_us,
        heap_total_us
    );

    // Print Arena row
    println!(
        "| {:>8} | {:>5} | {:>5} | {:>8}µs | {:>8}µs | {:>8}µs | {:>10}µs | **{:.1}x**",
        "", "", "Arena", arena_build_us, arena_compute_us, arena_render_us, arena_total_us, speedup
    );
}

fn main() {
    println!("\n=== Desktop Benchmark: Heap vs Arena ===");
    println!("Platform: Apple M2 Ultra (ARM64), Release Build\n");
    println!(
        "| {:>8} | {:>5} | {:>5} | {:>10} | {:>10} | {:>10} | {:>12} | Speedup",
        "Topology", "Nodes", "Mode", "Build", "Compute", "Render", "Total"
    );
    println!(
        "|----------|-------|-------|------------|------------|------------|--------------|--------"
    );
    io::stdout().flush().unwrap();

    let tests = [
        (Topology::Chain, 100),
        (Topology::Chain, 500),
        (Topology::Diamond, 100),
        (Topology::Diamond, 500),
        (Topology::WideFan, 100),
        (Topology::WideFan, 500),
    ];

    for (topology, size) in tests {
        run_comparison(topology, size);
        println!();
    }

    println!("Legend:");
    println!("  Chain   = Simple linear chain (best case)");
    println!("  Diamond = Diamond lattice with skip-level edges (stress test)");
    println!("  WideFan = Fan-out/fan-in (worst case for crossing reduction)");
    println!("\n  Build = DAG/CSR construction");
    println!("  Compute = Sugiyama layout algorithm");
    println!("  Render = ASCII output generation");
}
