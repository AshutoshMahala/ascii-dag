#![no_main]

use libfuzzer_sys::fuzz_target;
use ascii_dag::arena::Arena;
use ascii_dag::graph::DAG;

/// Fuzz the arena-based layout computation.
/// This specifically targets the code that was causing panics.
fuzz_target!(|data: &[u8]| {
    if data.len() < 8 {
        return;
    }
    
    // Parse graph parameters from fuzz input
    let node_count = (data[0] as usize % 50) + 1;
    let edge_count = (data[1] as usize % 100).min(node_count * 2);
    
    // Build a DAG
    let mut dag = DAG::new();
    
    for i in 0..node_count {
        let label_byte = data.get(2 + i).copied().unwrap_or(b'A');
        let label = format!("N{}{}", i, label_byte as char);
        let label: &'static str = Box::leak(label.into_boxed_str());
        dag.add_node(i, label);
    }
    
    // Add edges with varying skip distances (this is what triggered the original bug)
    let mut edge_data_idx = 2 + node_count;
    for _ in 0..edge_count {
        if edge_data_idx + 2 > data.len() {
            break;
        }
        
        let from = data[edge_data_idx] as usize % node_count;
        let skip_distance = (data[edge_data_idx + 1] as usize % node_count) + 1;
        let to = (from + skip_distance) % node_count;
        
        // Only add forward edges
        if from < to {
            dag.add_edge(from, to);
        }
        
        edge_data_idx += 2;
    }
    
    // Skip if cyclic
    if dag.has_cycle() {
        return;
    }
    
    // Estimate arena size and allocate
    let arena_size = dag.estimate_layout_arena_size();
    if arena_size > 16 * 1024 * 1024 { // Cap at 16MB
        return;
    }
    
    let mut temp_buffer = vec![0u8; arena_size];
    let mut output_buffer = vec![0u8; arena_size];
    
    // This is the critical path that was crashing
    let mut temp_arena = Arena::new(&mut temp_buffer);
    let mut output_arena = Arena::new(&mut output_buffer);
    
    if let Some(ir) = dag.compute_layout_arena(&mut temp_arena, &mut output_arena) {
        // Try to render
        let render_size = ir.estimate_render_size();
        if render_size < 1024 * 1024 { // Cap render at 1MB
            let mut render_buffer = vec![0u8; render_size];
            let mut line_buffer = vec![' '; ir.width().max(1)];
            let _ = ir.render_to_buffer(&mut render_buffer, &mut line_buffer);
        }
    }
});
