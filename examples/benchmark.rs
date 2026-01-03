use ascii_dag::graph::DAG;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

// --- Memory Tracking Allocator ---
struct TrackingAllocator;

static ALLOCATED: AtomicUsize = AtomicUsize::new(0);
static PEAK_ALLOCATED: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            let current = ALLOCATED.fetch_add(layout.size(), Ordering::SeqCst) + layout.size();
            let peak = PEAK_ALLOCATED.load(Ordering::SeqCst);
            if current > peak {
                PEAK_ALLOCATED.store(current, Ordering::SeqCst);
            }
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
        ALLOCATED.fetch_sub(layout.size(), Ordering::SeqCst);
    }
}

#[global_allocator]
static GLOBAL: TrackingAllocator = TrackingAllocator;

fn reset_metrics() {
    ALLOCATED.store(0, Ordering::SeqCst);
    PEAK_ALLOCATED.store(0, Ordering::SeqCst);
}

fn get_peak_memory() -> usize {
    PEAK_ALLOCATED.load(Ordering::SeqCst)
}

// --- Benchmark Logic ---

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

    // Returns true with probability p_true/100
    fn chance(&mut self, p_true: usize) -> bool {
        self.gen_range(0, 100) < p_true
    }
}

fn generate_layered_graph<'a>(dag: &mut DAG<'a>, node_count: usize, rng: &mut SimpleRng) {
    // Generate nodes
    let mut nodes = Vec::with_capacity(node_count);
    for i in 0..node_count {
        let label = Box::leak(format!("N{}", i).into_boxed_str());
        dag.add_node(i, label);
        nodes.push(i);
    }

    // Connect them in layers to simulate a realistic DAG
    // (Random graphs often cycle, so we force i -> j where i < j)
    let edges_per_node = 2;

    for i in 0..node_count.saturating_sub(1) {
        // Always connect to a nearby forward node to ensure connectivity
        let jump = rng.gen_range(1, 5.min(node_count - i));
        dag.add_edge(i, i + jump);

        // Add random extra edges
        for _ in 0..edges_per_node {
            if rng.chance(40) {
                // 40% chance of extra edge
                let target_jump = rng.gen_range(1, 20.min(node_count - i));
                dag.add_edge(i, i + target_jump);
            }
        }
    }
}

fn run_benchmark(count: usize) {
    println!("benchmarking {} nodes...", count);

    let mut rng = SimpleRng::new(12345);

    // Phase 1: Construction (runs on DEVICE)
    reset_metrics();
    let start_build = Instant::now();
    let mut dag = DAG::new();
    generate_layered_graph(&mut dag, count, &mut rng);
    let build_time = start_build.elapsed();
    let build_peak_mem = get_peak_memory();

    // Phase 2: Rendering (runs on HOST)
    reset_metrics();
    let start_render = Instant::now();
    let mut output = String::with_capacity(count * 100); // Pre-allocate output to minimize buffer noise

    dag.render_to(&mut output);

    let render_time = start_render.elapsed();
    let render_peak_mem = get_peak_memory();

    println!("  Nodes: {}", count);
    println!("  Build:  {:?} | Peak RAM: {:.2} KB  <-- DEVICE", build_time, build_peak_mem as f64 / 1024.0);
    println!("  Render: {:?} | Peak RAM: {:.2} KB  <-- HOST", render_time, render_peak_mem as f64 / 1024.0);
    println!("  Output size: {:.2} KB", output.len() as f64 / 1024.0);
    println!("--------------------------------");
}

fn main() {
    println!("=== Performance Benchmark (Time & Heap) ===\n");

    let sizes = [50, 100, 500, 1000];

    for &size in &sizes {
        run_benchmark(size);
    }

    println!("Done.");
}
