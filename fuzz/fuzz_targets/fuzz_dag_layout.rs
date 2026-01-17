#![no_main]

use libfuzzer_sys::fuzz_target;
use ascii_dag::graph::DAG;

/// Fuzz the DAG construction and layout computation.
/// Tests for panics, overflows, and invalid state.
fuzz_target!(|data: &[u8]| {
    if data.len() < 6 {
        return;
    }
    
    // Parse fuzzer input as graph specification
    let node_count = (data[0] as usize % 64) + 1; // 1-64 nodes
    let edge_density = data[1] as usize % 100; // 0-99% density
    
    // Build DAG from fuzz input
    let mut dag = DAG::new();
    
    // Add nodes
    for i in 0..node_count {
        let label_start = 2 + (i * 2) % (data.len() - 2).max(1);
        let label_len = (data.get(label_start).copied().unwrap_or(0) as usize % 16) + 1;
        let label_end = (label_start + 1 + label_len).min(data.len());
        
        // Use Box::leak for static lifetime (acceptable in fuzz test)
        let label = String::from_utf8_lossy(&data[label_start + 1..label_end]).to_string();
        let label: &'static str = Box::leak(label.into_boxed_str());
        
        dag.add_node(i, label);
    }
    
    // Add edges based on remaining data
    let mut edge_data_start = 2 + node_count * 2;
    if edge_data_start >= data.len() {
        edge_data_start = 2;
    }
    
    for chunk in data[edge_data_start..].chunks(2) {
        if chunk.len() == 2 {
            let from = chunk[0] as usize % node_count;
            let to = chunk[1] as usize % node_count;
            
            // Only add forward edges (from < to) to maintain DAG property
            if from < to {
                dag.add_edge(from, to);
            }
        }
    }
    
    // Test operations that could panic
    let _ = dag.has_cycle();
    let _ = dag.compute_layout();
    
    // Test rendering
    let ir = dag.compute_layout();
    let mut output = String::with_capacity(ir.width() * ir.height() * 4);
    ir.render_scanline(&mut output);
});
