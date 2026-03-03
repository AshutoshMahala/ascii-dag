use ascii_dag::graph::arena::Arena;
use ascii_dag::graph::Graph;

fn main() {
    println!("=== Precision Memory Audit (Small Graphs) ===\n");
    println!("Measuring EXACT bytes used for Layout (Temp) and Result (Output)\n");

    println!("| Nodes | CSR Estimate | Temp Used | Output Used | Total Used | Overhead |");
    println!("|-------|--------------|-----------|-------------|------------|----------|");

    let cases = [2, 3, 5, 7, 11];

    for &n in &cases {
        measure_chain(n);
    }

    println!("\n=== Conclusion ===\n");
    println!("The 'Total Used' column shows the real RAM requirement.");
}

fn measure_chain(n: usize) {
    let mut dag = Graph::new();
    for i in 0..n {
        dag.add_node(i, Box::leak(format!("N{}", i).into_boxed_str()));
        if i > 0 {
            dag.add_edge(i - 1, i, None);
        }
    }

    let csr_estimate = dag.estimate_csr_arena_size();

    // Allocate generous buffers filled with SENTINEL
    const SENTINEL: u8 = 0xFF;
    let mut temp_buffer = vec![SENTINEL; 1024 * 1024]; // 1MB
    let mut output_buffer = vec![SENTINEL; 1024 * 1024]; // 1MB

    let success = {
        let mut temp_arena = Arena::new(&mut temp_buffer);
        let mut output_arena = Arena::new(&mut output_buffer);
        dag.compute_layout_arena(&mut temp_arena, &mut output_arena)
            .is_ok()
    };

    if success {
        let temp_used = count_used_bytes(&temp_buffer, SENTINEL);
        let output_used = count_used_bytes(&output_buffer, SENTINEL);
        let total = temp_used + output_used;
        let overhead = total as f64 / csr_estimate as f64;

        println!(
            "| {:<5} | {:<12} | {:<9} | {:<11} | {:<10} | {:.1}x     |",
            n, csr_estimate, temp_used, output_used, total, overhead
        );
    } else {
        println!(
            "| {:<5} | FAILED       | -         | -           | -          | -        |",
            n
        );
    }
}

fn count_used_bytes(buffer: &[u8], sentinel: u8) -> usize {
    // Find the last byte that is NOT the sentinel
    // Since arena allocates contiguously from 0, everything up to that point is "used"
    // (or at least "touched").
    // Note: Allocations might write the sentinel value accurately, but it's unlikely to generate
    // a long tail of exact sentinel matches at the exact boundary.
    // To be safe, we look from the end.

    let mut len = buffer.len();
    while len > 0 {
        if buffer[len - 1] != sentinel {
            break;
        }
        len -= 1;
    }
    len
}
