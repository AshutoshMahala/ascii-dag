use ascii_dag::DAG;
use ascii_dag::render::colors::Palette;

fn main() {
    let mut dag = DAG::new();

    dag.add_node(1, "Root");
    dag.add_node(2, "Task A");
    dag.add_node(3, "Task B");
    dag.add_node(4, "Task C");
    dag.add_node(5, "Task D");
    dag.add_node(6, "Task E");
    dag.add_node(7, "Task F");
    dag.add_node(8, "Output");

    // All edges with labels to test collision handling
    dag.add_edge(1, 2, Some("init"));
    dag.add_edge(1, 3, Some("spawn"));
    dag.add_edge(1, 4, Some("fork"));
    dag.add_edge(1, 5, Some("start"));
    dag.add_edge(1, 6, Some("begin"));

    dag.add_edge(2, 7, Some("run"));
    dag.add_edge(3, 7, Some("exec"));
    dag.add_edge(4, 7, Some("call"));
    dag.add_edge(5, 7, Some("join"));

    dag.add_edge(7, 8, Some("done"));
    dag.add_edge(6, 8, Some("skip"));

    println!("With ANSI colors and edge labels:");

    let args: Vec<String> = std::env::args().collect();
    let use_arena = args.iter().any(|a| a == "--arena");

    if use_arena {
        use ascii_dag::arena::Arena;
        println!("(Arena Mode)");
        // Estimate size and allocate buffers
        let arena_size = dag.estimate_layout_arena_size();
        let mut temp_buffer = vec![0u8; arena_size];
        let mut output_buffer = vec![0u8; arena_size];
        let mut temp_arena = Arena::new(&mut temp_buffer);
        let mut output_arena = Arena::new(&mut output_buffer); // Restored

        if let Some(ir) = dag.compute_layout_arena(&mut temp_arena, &mut output_arena) {
            // Calculate colors manually (Arena API requires provided buffer)
            let mut edge_colors = vec![0usize; ir.edge_count()];
            let palette_colors = Palette::Ansi.colors();
            ir.compute_edge_colors(&mut edge_colors, palette_colors.len());

            // Allocate render buffers
            // Estimate size: width * height * (~10 chars for color codes + char) + legend
            let (render_bytes, _) = ir.estimate_render_size();
            let render_size = render_bytes * 10 + 4096;
            let mut render_buffer = vec![0u8; render_size];
            let mut line_buffer = vec![' '; ir.width().max(1) + 16];
            let mut color_buffer = vec![0u8; ir.width().max(1) + 16];
            let mut skipped_buffer = vec![false; ir.edge_count().max(1)]; // For legend

            if let Some(len) = ir.render_to_buffer_colored_with_legend(
                &mut render_buffer,
                &mut line_buffer,
                &mut color_buffer,
                &edge_colors,
                palette_colors,
                &mut skipped_buffer, // Needs a buffer to track skipped edges for legend
            ) {
                println!("{}", std::str::from_utf8(&render_buffer[..len]).unwrap());
            }
        }
    } else {
        let ir = dag.compute_layout();
        println!("{}", ir.render_scanline_colored_with_legend(Palette::Ansi));
    }

    // Benchmark: test with many labeled edges
    println!("\n--- Performance test (Heap only for bench section) ---");
    for num_nodes in [20, 50, 100, 200, 500] {
        let mut big_dag = DAG::new();
        for i in 0..num_nodes {
            big_dag.add_node(i, "N");
        }
        let mut edge_count = 0;
        for i in 0..num_nodes {
            for j in (i + 1)..num_nodes.min(i + 5) {
                // Each node connects to next 4
                big_dag.add_edge(i, j, Some("e"));
                edge_count += 1;
            }
        }
        let start = std::time::Instant::now();
        let ir2 = big_dag.compute_layout();
        let layout_time = start.elapsed();

        let start2 = std::time::Instant::now();
        let _ = ir2.render_scanline_colored_with_legend(Palette::Ansi);
        let render_time = start2.elapsed();

        println!(
            "{:4} nodes, {:4} edges: layout {:>10?}, render {:>10?}",
            num_nodes, edge_count, layout_time, render_time
        );
    }
}
