use ascii_dag::graph::arena::Arena;
use ascii_dag::graph::DAG;
use std::time::Instant;

fn main() {
    let widths = [
        224, // ~50k
        245, // ~60k
        265, // ~70k
        283, // ~80k
        300, // ~90k
        316, // ~100k
        450, // ~200k
    ];

    println!("=== ASCII DAG Scaling Cliff Benchmark (ARENA MODE) ===\n");

    for width in widths {
        let nodes = width * width;
        println!(">>> Testing Width: {} ({} nodes) <<<", width, nodes);

        let mut dag = DAG::new();
        let height = width;

        // Generate Graph
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

        // Run Benchmark
        run_test(nodes, &dag);
        println!("--------------------------------------------------");
    }
}

fn run_test(nodes: usize, dag: &DAG) {
    // Estimate size
    let layout_estimate = dag.estimate_layout_arena_size();
    let temp_arena_size = (layout_estimate * 3 / 2).max(1024 * 1024);
    let output_arena_size = temp_arena_size;

    let mut temp_buffer = vec![0u8; temp_arena_size];
    let mut output_buffer = vec![0u8; output_arena_size];

    let start_layout = Instant::now();
    let mut temp_arena = Arena::new(&mut temp_buffer);
    let mut output_arena = Arena::new(&mut output_buffer);

    if let Some(layout) = dag.compute_layout_arena(&mut temp_arena, &mut output_arena) {
        let layout_duration = start_layout.elapsed();

        // Render buffer
        let (render_bytes, scratch_indices) = layout.estimate_render_size();
        let mut render_buffer = vec![0u8; render_bytes + 1024];
        let mut scratch_buffer = vec![0usize; scratch_indices + 1024];
        let mut line_buffer = vec![' '; layout.width() + 1024];

        let start_render = Instant::now();
        let _ = layout.render_to_buffer(&mut render_buffer, &mut line_buffer, &mut scratch_buffer);
        let render_duration = start_render.elapsed();

        let total_duration = layout_duration + render_duration;
        println!(
            ">>> [{} Nodes] Total: {:.4}s (Layout: {:.4}s, Render: {:.4}s)",
            nodes,
            total_duration.as_secs_f64(),
            layout_duration.as_secs_f64(),
            render_duration.as_secs_f64()
        );
    } else {
        println!(">>> [{} Nodes] FAILED (OOM?)", nodes);
    }
}
