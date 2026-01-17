#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use esp_hal::time::Instant;
use esp_println::println;
use embedded_alloc::LlffHeap as Heap;
use ascii_dag::DAG;

// Import esp-backtrace to get panic handler
use esp_backtrace as _;

// Required app descriptor for ESP-IDF bootloader
esp_bootloader_esp_idf::esp_app_desc!();

#[global_allocator]
static HEAP: Heap = Heap::empty();

#[esp_hal::main]
fn main() -> ! {
    // Initialize heap (128KB for larger benchmarks)
    {
        use core::mem::MaybeUninit;
        const HEAP_SIZE: usize = 128 * 1024;
        static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];
        #[allow(static_mut_refs)]
        unsafe { HEAP.init(HEAP_MEM.as_ptr() as usize, HEAP_SIZE) }
    }

    let _peripherals = esp_hal::init(esp_hal::Config::default());

    println!("\n╔══════════════════════════════════════════════════╗");
    println!("║     ascii-dag Performance Test on ESP32-S3      ║");
    println!("║          Xtensa LX7 @ 240 MHz, 512KB RAM        ║");
    println!("╚══════════════════════════════════════════════════╝\n");

    // ========================================
    // BENCHMARK 1: Simple Chain (4 nodes)
    // ========================================
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("BENCHMARK 1: Diamond Pattern (4 nodes, 4 edges)");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    
    let heap_before = HEAP.used();
    let start = Instant::now();
    
    let mut dag = DAG::new();
    dag.add_node(1, "Root");
    dag.add_node(2, "Left");
    dag.add_node(3, "Right");
    dag.add_node(4, "Merge");
    dag.add_edge(1, 2);
    dag.add_edge(1, 3);
    dag.add_edge(2, 4);
    dag.add_edge(3, 4);
    
    let build_time = start.elapsed();
    let render_start = Instant::now();
    
    let output = dag.render();
    
    let render_time = render_start.elapsed();
    let heap_after = HEAP.used();
    
    println!("{}", output);
    println!("\nBuild:  {:>6} µs", build_time.as_micros());
    println!("Render: {:>6} µs", render_time.as_micros());
    println!("Heap:   {:>6} bytes (delta: {} bytes)", heap_after, heap_after.saturating_sub(heap_before));
    println!("Output: {:>6} bytes\n", output.len());
    
    drop(output);
    drop(dag);

    // ========================================
    // BENCHMARK 2: Medium Pipeline (10 nodes)
    // ========================================
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("BENCHMARK 2: Build Pipeline (10 nodes, 12 edges)");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    
    let heap_before = HEAP.used();
    let start = Instant::now();
    
    let dag = DAG::from_edges(
        &[
            (1, "Source"), (2, "Parse"), (3, "Validate"), (4, "Transform"),
            (5, "Optimize"), (6, "CodeGen"), (7, "Link"), (8, "Test"),
            (9, "Package"), (10, "Deploy"),
        ],
        &[
            (1, 2), (2, 3), (3, 4), (4, 5), (5, 6),
            (6, 7), (7, 8), (8, 9), (9, 10),
            (1, 4), (3, 6), (5, 8),
        ],
    );
    
    let build_time = start.elapsed();
    let render_start = Instant::now();
    
    let output = dag.render();
    
    let render_time = render_start.elapsed();
    let heap_after = HEAP.used();
    
    println!("{}", output);
    println!("\nBuild:  {:>6} µs", build_time.as_micros());
    println!("Render: {:>6} µs", render_time.as_micros());
    println!("Heap:   {:>6} bytes (delta: {} bytes)", heap_after, heap_after.saturating_sub(heap_before));
    println!("Output: {:>6} bytes\n", output.len());
    
    drop(output);
    drop(dag);

    // ========================================
    // BENCHMARK 3: Wide DAG (fan-out/fan-in)
    // ========================================
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("BENCHMARK 3: Wide Fan-Out/Fan-In (12 nodes, 16 edges)");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    
    let heap_before = HEAP.used();
    let start = Instant::now();
    
    let dag = DAG::from_edges(
        &[
            (1, "Start"),
            (2, "Worker1"), (3, "Worker2"), (4, "Worker3"), (5, "Worker4"), (6, "Worker5"),
            (7, "Stage2-A"), (8, "Stage2-B"), (9, "Stage2-C"),
            (10, "Merge1"), (11, "Merge2"),
            (12, "Final"),
        ],
        &[
            (1, 2), (1, 3), (1, 4), (1, 5), (1, 6),
            (2, 7), (3, 7), (4, 8), (5, 9), (6, 9),
            (7, 10), (8, 10), (8, 11), (9, 11),
            (10, 12), (11, 12),
        ],
    );
    
    let build_time = start.elapsed();
    let render_start = Instant::now();
    
    let output = dag.render();
    
    let render_time = render_start.elapsed();
    let heap_after = HEAP.used();
    
    println!("{}", output);
    println!("\nBuild:  {:>6} µs", build_time.as_micros());
    println!("Render: {:>6} µs", render_time.as_micros());
    println!("Heap:   {:>6} bytes (delta: {} bytes)", heap_after, heap_after.saturating_sub(heap_before));
    println!("Output: {:>6} bytes\n", output.len());
    
    drop(output);
    drop(dag);

    // ========================================
    // BENCHMARK 4: Large Binary Tree (31 nodes)
    // ========================================
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("BENCHMARK 4: Binary Tree (31 nodes, 30 edges)");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    
    static LABELS: [&str; 31] = [
        "N00", "N01", "N02", "N03", "N04", "N05", "N06", "N07",
        "N08", "N09", "N10", "N11", "N12", "N13", "N14", "N15",
        "N16", "N17", "N18", "N19", "N20", "N21", "N22", "N23",
        "N24", "N25", "N26", "N27", "N28", "N29", "N30",
    ];
    
    let mut nodes: Vec<(usize, &str)> = Vec::new();
    let mut edges: Vec<(usize, usize)> = Vec::new();
    
    for i in 0..31usize {
        nodes.push((i + 1, LABELS[i]));
    }
    
    for i in 1..=15usize {
        edges.push((i, i * 2));
        edges.push((i, i * 2 + 1));
    }
    
    let heap_before = HEAP.used();
    let start = Instant::now();
    
    let dag = DAG::from_edges(&nodes, &edges);
    
    let build_time = start.elapsed();
    let render_start = Instant::now();
    
    let output = dag.render();
    
    let render_time = render_start.elapsed();
    let heap_after = HEAP.used();
    
    // Show first 20 lines
    println!("(showing first 20 lines)");
    for (i, line) in output.lines().enumerate() {
        if i >= 20 {
            println!("...(truncated)");
            break;
        }
        println!("{}", line);
    }
    
    println!("\nBuild:  {:>6} µs", build_time.as_micros());
    println!("Render: {:>6} µs", render_time.as_micros());
    println!("Heap:   {:>6} bytes (delta: {} bytes)", heap_after, heap_after.saturating_sub(heap_before));
    println!("Output: {:>6} bytes ({} lines)\n", output.len(), output.lines().count());
    
    drop(output);
    drop(dag);
    drop(nodes);
    drop(edges);

    // ========================================
    // BENCHMARK 5: Deep Chain (50 nodes)
    // ========================================
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("BENCHMARK 5: Deep Chain (50 nodes, 49 edges)");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    
    static CHAIN_LABELS: [&str; 50] = [
        "S00", "S01", "S02", "S03", "S04", "S05", "S06", "S07", "S08", "S09",
        "S10", "S11", "S12", "S13", "S14", "S15", "S16", "S17", "S18", "S19",
        "S20", "S21", "S22", "S23", "S24", "S25", "S26", "S27", "S28", "S29",
        "S30", "S31", "S32", "S33", "S34", "S35", "S36", "S37", "S38", "S39",
        "S40", "S41", "S42", "S43", "S44", "S45", "S46", "S47", "S48", "S49",
    ];
    
    let mut nodes: Vec<(usize, &str)> = Vec::new();
    let mut edges: Vec<(usize, usize)> = Vec::new();
    
    for i in 0..50usize {
        nodes.push((i + 1, CHAIN_LABELS[i]));
    }
    for i in 1..50usize {
        edges.push((i, i + 1));
    }
    
    let heap_before = HEAP.used();
    let start = Instant::now();
    
    let dag = DAG::from_edges(&nodes, &edges);
    
    let build_time = start.elapsed();
    let render_start = Instant::now();
    
    let output = dag.render();
    
    let render_time = render_start.elapsed();
    let heap_after = HEAP.used();
    
    println!("(showing first 10 lines)");
    for (i, line) in output.lines().enumerate() {
        if i >= 10 {
            println!("...(truncated {} more lines)", output.lines().count() - 10);
            break;
        }
        println!("{}", line);
    }
    
    println!("\nBuild:  {:>6} µs", build_time.as_micros());
    println!("Render: {:>6} µs", render_time.as_micros());
    println!("Heap:   {:>6} bytes (delta: {} bytes)", heap_after, heap_after.saturating_sub(heap_before));
    println!("Output: {:>6} bytes ({} lines)\n", output.len(), output.lines().count());
    
    drop(output);
    drop(dag);
    drop(nodes);
    drop(edges);

    // ========================================
    // BENCHMARK 6: Diamond Lattice (64 nodes)
    // ========================================
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("BENCHMARK 6: Diamond Lattice (64 nodes, 112 edges)");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    
    // Build a 8-layer lattice with diamonds (8x8 = 64 nodes)
    let mut nodes: Vec<(usize, &str)> = Vec::new();
    let mut edges: Vec<(usize, usize)> = Vec::new();
    
    static LATTICE_LABELS: [&str; 64] = [
        "L00", "L01", "L02", "L03", "L04", "L05", "L06", "L07",
        "L10", "L11", "L12", "L13", "L14", "L15", "L16", "L17",
        "L20", "L21", "L22", "L23", "L24", "L25", "L26", "L27",
        "L30", "L31", "L32", "L33", "L34", "L35", "L36", "L37",
        "L40", "L41", "L42", "L43", "L44", "L45", "L46", "L47",
        "L50", "L51", "L52", "L53", "L54", "L55", "L56", "L57",
        "L60", "L61", "L62", "L63", "L64", "L65", "L66", "L67",
        "L70", "L71", "L72", "L73", "L74", "L75", "L76", "L77",
    ];
    
    // 8 rows of 8 nodes each
    for i in 0..64usize {
        nodes.push((i + 1, LATTICE_LABELS[i]));
    }
    
    // Connect each node to 1-2 nodes in the next row (creating diamond patterns)
    for row in 0..7usize {
        for col in 0..8usize {
            let from = row * 8 + col + 1;
            let to_row = row + 1;
            // Connect to same column and adjacent column
            edges.push((from, to_row * 8 + col + 1));
            if col < 7 {
                edges.push((from, to_row * 8 + col + 2));
            }
        }
    }
    
    let heap_before = HEAP.used();
    let start = Instant::now();
    
    let dag = DAG::from_edges(&nodes, &edges);
    
    let build_time = start.elapsed();
    let render_start = Instant::now();
    
    let output = dag.render();
    
    let render_time = render_start.elapsed();
    let heap_after = HEAP.used();
    
    println!("(showing first 15 lines)");
    for (i, line) in output.lines().enumerate() {
        if i >= 15 {
            println!("...(truncated {} more lines)", output.lines().count() - 15);
            break;
        }
        println!("{}", line);
    }
    
    println!("\nBuild:  {:>6} µs", build_time.as_micros());
    println!("Render: {:>6} µs", render_time.as_micros());
    println!("Heap:   {:>6} bytes (delta: {} bytes)", heap_after, heap_after.saturating_sub(heap_before));
    println!("Output: {:>6} bytes ({} lines)\n", output.len(), output.lines().count());
    
    drop(output);
    drop(dag);
    drop(nodes);
    drop(edges);

    // ========================================
    // FINAL SUMMARY
    // ========================================
    println!("╔══════════════════════════════════════════════════╗");
    println!("║                    SUMMARY                       ║");
    println!("╠══════════════════════════════════════════════════╣");
    
    let final_heap = HEAP.used();
    let free_heap = HEAP.free();
    
    println!("║  Total heap allocated: {:>6} bytes             ║", 128 * 1024);
    println!("║  Heap currently used:  {:>6} bytes              ║", final_heap);
    println!("║  Heap free:            {:>6} bytes              ║", free_heap);
    println!("║                                                  ║");
    println!("║  ascii-dag runs great on ESP32-S3! 🎉          ║");
    println!("╚══════════════════════════════════════════════════╝\n");

    println!("Done! Press reset to run again.");

    loop {
        // Idle loop
    }
}
