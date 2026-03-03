//! Arena vs Heap Benchmark
//!
//! Compares performance and memory usage between:
//! - Standard heap-based rendering
//! - Arena-based rendering (when implemented)
//!
//! Run with: cargo run --example arena_benchmark --release

use ascii_dag::graph::arena::Arena;
use ascii_dag::graph::Graph;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

// --- Memory Tracking Allocator ---
struct TrackingAllocator;

static ALLOCATED: AtomicUsize = AtomicUsize::new(0);
static PEAK_ALLOCATED: AtomicUsize = AtomicUsize::new(0);
static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let current = ALLOCATED.fetch_add(layout.size(), Ordering::SeqCst) + layout.size();
            ALLOC_COUNT.fetch_add(1, Ordering::SeqCst);
            let peak = PEAK_ALLOCATED.load(Ordering::SeqCst);
            if current > peak {
                PEAK_ALLOCATED.store(current, Ordering::SeqCst);
            }
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        ALLOCATED.fetch_sub(layout.size(), Ordering::SeqCst);
    }
}

#[global_allocator]
static GLOBAL: TrackingAllocator = TrackingAllocator;

fn reset_metrics() {
    ALLOCATED.store(0, Ordering::SeqCst);
    PEAK_ALLOCATED.store(0, Ordering::SeqCst);
    ALLOC_COUNT.store(0, Ordering::SeqCst);
}

fn get_peak_memory() -> usize {
    PEAK_ALLOCATED.load(Ordering::SeqCst)
}

fn get_alloc_count() -> usize {
    ALLOC_COUNT.load(Ordering::SeqCst)
}

// --- Test Graph Generation ---

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
        if max <= min {
            return min;
        }
        let range = (max - min) as u64;
        let random = self.next_u64();
        min + (random % range) as usize
    }

    fn chance(&mut self, p_true: usize) -> bool {
        self.gen_range(0, 100) < p_true
    }
}

fn generate_layered_graph<'a>(dag: &mut Graph<'a>, node_count: usize, rng: &mut SimpleRng) {
    for i in 0..node_count {
        let label = Box::leak(format!("N{}", i).into_boxed_str());
        dag.add_node(i, label);
    }

    let edges_per_node = 2;
    for i in 0..node_count.saturating_sub(1) {
        let jump = rng.gen_range(1, 5.min(node_count - i));
        dag.add_edge(i, i + jump, None);

        for _ in 0..edges_per_node {
            if rng.chance(40) {
                let target_jump = rng.gen_range(1, 20.min(node_count - i));
                dag.add_edge(i, i + target_jump, None);
            }
        }
    }
}

fn generate_wide_graph<'a>(dag: &mut Graph<'a>, width: usize, levels: usize, rng: &mut SimpleRng) {
    let mut id = 0;
    for level in 0..levels {
        for _ in 0..width {
            let label = Box::leak(format!("L{}N{}", level, id).into_boxed_str());
            dag.add_node(id, label);
            id += 1;
        }
    }

    // Connect each level to the next
    for level in 0..levels - 1 {
        let level_start = level * width;
        let next_level_start = (level + 1) * width;
        for i in 0..width {
            let from = level_start + i;
            // Connect to 1-3 nodes in the next level
            let conn_count = rng.gen_range(1, 4.min(width));
            for _ in 0..conn_count {
                let to = next_level_start + rng.gen_range(0, width);
                dag.add_edge(from, to, None);
            }
        }
    }
}

fn generate_deep_chain<'a>(dag: &mut Graph<'a>, depth: usize) {
    for i in 0..depth {
        let label = Box::leak(format!("D{}", i).into_boxed_str());
        dag.add_node(i, label);
        if i > 0 {
            dag.add_edge(i - 1, i, None);
        }
    }
}

fn generate_skip_heavy<'a>(dag: &mut Graph<'a>, nodes: usize, rng: &mut SimpleRng) {
    for i in 0..nodes {
        let label = Box::leak(format!("S{}", i).into_boxed_str());
        dag.add_node(i, label);
    }

    // Create many skip-level edges
    for i in 0..nodes {
        // Regular forward edge
        if i + 1 < nodes {
            dag.add_edge(i, i + 1, None);
        }
        // Skip edges (2-5 levels ahead)
        for _ in 0..3 {
            if rng.chance(60) {
                let skip = rng.gen_range(2, 6.min(nodes - i));
                if i + skip < nodes {
                    dag.add_edge(i, i + skip, None);
                }
            }
        }
    }
}

// --- Benchmark Result ---

#[derive(Debug)]
struct BenchResult {
    name: String,
    mode: String,
    build_time: Duration,
    render_time: Duration,
    peak_heap_bytes: usize,
    alloc_count: usize,
    output_size: usize,
    arena_used: usize,
}

impl BenchResult {
    fn print_header() {
        println!(
            "| {:12} | {:6} | {:>12} | {:>12} | {:>10} | {:>8} | {:>10} |",
            "Test", "Mode", "Build", "Render", "Peak RAM", "Allocs", "Output"
        );
        println!(
            "|{:-<14}|{:-<8}|{:-<14}|{:-<14}|{:-<12}|{:-<10}|{:-<12}|",
            "", "", "", "", "", "", ""
        );
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

    fn print(&self) {
        println!(
            "| {:12} | {:6} | {:>12} | {:>12} | {:>10} | {:>8} | {:>10} |",
            self.name,
            self.mode,
            Self::format_duration(self.build_time),
            Self::format_duration(self.render_time),
            format!("{:.1}KB", self.peak_heap_bytes as f64 / 1024.0),
            self.alloc_count,
            format!("{:.1}KB", self.output_size as f64 / 1024.0),
        );
    }
}

// --- Benchmark Runner ---

fn run_heap_benchmark<F>(name: &str, generator: F, rng: &mut SimpleRng) -> BenchResult
where
    F: Fn(&mut SimpleRng) -> Graph<'static>,
{
    // Phase 1: Build
    reset_metrics();
    let build_start = Instant::now();
    let dag = generator(rng);
    let build_time = build_start.elapsed();
    let build_allocs = get_alloc_count();
    let build_peak = get_peak_memory();

    // Phase 2: Render
    reset_metrics();
    let render_start = Instant::now();
    let mut output = String::with_capacity(dag.estimate_size());
    dag.render_to(&mut output);
    let render_time = render_start.elapsed();
    let render_peak = get_peak_memory();
    let render_allocs = get_alloc_count();

    BenchResult {
        name: name.to_string(),
        mode: "heap".to_string(),
        build_time,
        render_time,
        peak_heap_bytes: build_peak.max(render_peak),
        alloc_count: build_allocs + render_allocs,
        output_size: output.len(),
        arena_used: 0,
    }
}

fn run_arena_test(name: &str, arena_size: usize) -> BenchResult {
    // Test the arena module (not yet integrated with render)
    let mut buffer = vec![0u8; arena_size];
    let arena = Arena::new(&mut buffer);

    // Simulate allocations similar to what render would need
    let start = Instant::now();

    // Allocate test data
    let _nums: &mut [usize] = arena.alloc_slice_default(1000).unwrap();
    let _coords: &mut [usize] = arena.alloc_slice_default(1000).unwrap();
    let _flags: &mut [bool] = arena.alloc_slice_default(1000).unwrap();

    let elapsed = start.elapsed();
    let used = arena.used();
    let allocs = arena.alloc_count();

    arena.reset();

    BenchResult {
        name: name.to_string(),
        mode: "arena".to_string(),
        build_time: Duration::ZERO,
        render_time: elapsed,
        peak_heap_bytes: 0,
        alloc_count: allocs,
        output_size: 0,
        arena_used: used,
    }
}

/// Benchmark using arena-based CSR graph conversion
fn run_csr_benchmark<F>(name: &str, generator: F, rng: &mut SimpleRng) -> BenchResult
where
    F: Fn(&mut SimpleRng) -> Graph<'static>,
{
    // Phase 1: Build DAG (same as heap)
    reset_metrics();
    let build_start = Instant::now();
    let dag = generator(rng);
    let build_time = build_start.elapsed();
    let build_allocs = get_alloc_count();
    let build_peak = get_peak_memory();

    // Phase 2: Convert to CSR using arena
    // Pre-allocate arena buffer BEFORE measurement
    let arena_size = dag.estimate_csr_arena_size();
    let mut arena_buffer = vec![0u8; arena_size];

    reset_metrics();
    let convert_start = Instant::now();

    // We need to work around the borrow issue - get data from separate scopes
    let csr_elements = {
        let mut arena = Arena::new(&mut arena_buffer);
        if let Some(csr) = dag.to_csr(&mut arena) {
            csr.node_count() * 3 + csr.edge_count() * 2
        } else {
            0
        }
    };

    let convert_time = convert_start.elapsed();
    let convert_peak = get_peak_memory();
    let convert_allocs = get_alloc_count();

    // Arena used is approximately the estimate (we can't easily query it due to lifetimes)
    let arena_used = arena_size;

    BenchResult {
        name: name.to_string(),
        mode: "csr".to_string(),
        build_time,
        render_time: convert_time,
        peak_heap_bytes: build_peak.max(convert_peak),
        alloc_count: build_allocs + convert_allocs,
        output_size: csr_elements,
        arena_used,
    }
}

/// Benchmark full arena pipeline: Graph -> CSR -> Render (no heap allocs)
fn run_full_arena_benchmark<F>(name: &str, generator: F, rng: &mut SimpleRng) -> BenchResult
where
    F: Fn(&mut SimpleRng) -> Graph<'static>,
{
    // Phase 1: Build DAG (heap-based, unavoidable for now)
    reset_metrics();
    let build_start = Instant::now();
    let dag = generator(rng);
    let build_time = build_start.elapsed();
    let build_allocs = get_alloc_count();
    let build_peak = get_peak_memory();

    // Phase 2: Convert to CSR + Render using arena
    // Pre-allocate both arena and render buffer BEFORE measurement
    let arena_size = dag.estimate_csr_arena_size();
    let mut arena_buffer = vec![0u8; arena_size];

    // Estimate render buffer size (generous estimate)
    let render_buffer_size = arena_size; // Use same size as arena
    let mut render_buffer = vec![0u8; render_buffer_size];

    reset_metrics();
    let render_start = Instant::now();

    let (output_size, arena_used) = {
        let mut arena = Arena::new(&mut arena_buffer);
        if let Some(csr) = dag.to_csr(&mut arena) {
            let bytes_written = csr.render_to_buffer(&mut render_buffer).unwrap_or(0);
            (bytes_written, arena_size)
        } else {
            (0, 0)
        }
    };

    let render_time = render_start.elapsed();
    let render_peak = get_peak_memory();
    let render_allocs = get_alloc_count();

    BenchResult {
        name: name.to_string(),
        mode: "full".to_string(),
        build_time,
        render_time,
        peak_heap_bytes: build_peak.max(render_peak),
        alloc_count: build_allocs + render_allocs,
        output_size,
        arena_used,
    }
}

// --- Test Cases ---

fn run_test_suite() {
    println!("=== Arena vs Heap Benchmark ===\n");

    let mut rng = SimpleRng::new(12345);

    // Define test cases - we'll clone the generator for multiple runs
    let test_configs: Vec<(&str, usize, usize, usize)> = vec![
        // (name, node_count, 0=layered/1=wide/2=deep/3=skip, extra_param)
        ("tiny", 10, 0, 0),
        ("small", 50, 0, 0),
        ("medium", 200, 0, 0),
        ("large", 1000, 0, 0),
        ("wide", 50, 1, 10),
        ("deep", 100, 2, 0),
        ("skip_heavy", 200, 3, 0),
    ];

    println!("## Heap-based Rendering (Current)\n");
    BenchResult::print_header();

    for (name, count, graph_type, extra) in &test_configs {
        let result = run_heap_benchmark(
            name,
            |rng| {
                let mut dag = Graph::new();
                match graph_type {
                    0 => generate_layered_graph(&mut dag, *count, rng),
                    1 => generate_wide_graph(&mut dag, *count, *extra, rng),
                    2 => generate_deep_chain(&mut dag, *count),
                    3 => generate_skip_heavy(&mut dag, *count, rng),
                    _ => {}
                }
                dag
            },
            &mut rng,
        );
        result.print();
    }

    println!("\n## Pre-allocated Buffer Rendering\n");
    BenchResult::print_header();

    for (name, count, graph_type, extra) in &test_configs {
        let result = run_buffer_benchmark(
            name,
            |rng| {
                let mut dag = Graph::new();
                match graph_type {
                    0 => generate_layered_graph(&mut dag, *count, rng),
                    1 => generate_wide_graph(&mut dag, *count, *extra, rng),
                    2 => generate_deep_chain(&mut dag, *count),
                    3 => generate_skip_heavy(&mut dag, *count, rng),
                    _ => {}
                }
                dag
            },
            &mut rng,
        );
        result.print();
    }

    println!("\n## Arena-based CSR Conversion\n");
    println!("Converting heap DAG to arena-backed CSR format:\n");
    BenchResult::print_header();

    for (name, count, graph_type, extra) in &test_configs {
        let result = run_csr_benchmark(
            name,
            |rng| {
                let mut dag = Graph::new();
                match graph_type {
                    0 => generate_layered_graph(&mut dag, *count, rng),
                    1 => generate_wide_graph(&mut dag, *count, *extra, rng),
                    2 => generate_deep_chain(&mut dag, *count),
                    3 => generate_skip_heavy(&mut dag, *count, rng),
                    _ => {}
                }
                dag
            },
            &mut rng,
        );
        result.print();
    }

    println!("\n## Full Arena Pipeline (CSR + Render)\n");
    println!("Complete no-alloc path: Graph -> CSR -> Buffer render:\n");
    BenchResult::print_header();

    for (name, count, graph_type, extra) in &test_configs {
        let result = run_full_arena_benchmark(
            name,
            |rng| {
                let mut dag = Graph::new();
                match graph_type {
                    0 => generate_layered_graph(&mut dag, *count, rng),
                    1 => generate_wide_graph(&mut dag, *count, *extra, rng),
                    2 => generate_deep_chain(&mut dag, *count),
                    3 => generate_skip_heavy(&mut dag, *count, rng),
                    _ => {}
                }
                dag
            },
            &mut rng,
        );
        result.print();
    }

    println!("\n## Arena Module Test (Allocation Speed)\n");
    println!("Testing arena allocation overhead:\n");

    let arena_result = run_arena_test("arena_alloc", 64 * 1024);
    println!(
        "  Arena alloc time: {:.3}µs for {} allocations",
        arena_result.render_time.as_nanos() as f64 / 1000.0,
        arena_result.alloc_count
    );
    println!(
        "  Arena memory used: {} bytes ({:.1}% of 64KB)",
        arena_result.arena_used,
        arena_result.arena_used as f64 / (64.0 * 1024.0) * 100.0
    );

    println!("\n## Summary\n");
    println!("Comparing heap vs pre-allocated buffer vs arena-based CSR:");
    println!("  - Heap: Standard Vec/String allocations per operation");
    println!("  - Buffer: Pre-allocated line buffer, reduces render allocations");
    println!("  - CSR: Arena-backed graph format, zero heap allocs during conversion");
    println!("  - Full: Complete no-alloc pipeline (CSR + render to buffer)");
    println!();
}

/// Benchmark using pre-allocated buffers (arena-friendly rendering)
fn run_buffer_benchmark<F>(name: &str, generator: F, rng: &mut SimpleRng) -> BenchResult
where
    F: Fn(&mut SimpleRng) -> Graph<'static>,
{
    // Phase 1: Build (same as heap)
    reset_metrics();
    let build_start = Instant::now();
    let dag = generator(rng);
    let build_time = build_start.elapsed();
    let build_allocs = get_alloc_count();
    let build_peak = get_peak_memory();

    // Get the IR first (includes layout computation)
    let ir = dag.compute_layout();

    // Pre-allocate buffers BEFORE measurement
    let mut line_buffer: Vec<char> = vec![' '; ir.width()];
    let mut output = String::with_capacity(ir.width() * ir.height() * 2);

    // Reset metrics to measure ONLY the render phase
    reset_metrics();
    let render_start = Instant::now();

    // Use the buffer-based render method
    ir.render_scanline_with_buffer(&mut line_buffer, &mut output);

    let render_time = render_start.elapsed();
    let render_peak = get_peak_memory();
    let render_allocs = get_alloc_count();

    BenchResult {
        name: name.to_string(),
        mode: "buffer".to_string(),
        build_time,
        render_time,
        peak_heap_bytes: build_peak.max(render_peak),
        alloc_count: build_allocs + render_allocs,
        output_size: output.len(),
        arena_used: line_buffer.len() * 4, // chars are 4 bytes
    }
}

fn main() {
    run_test_suite();
}
