//! Compare heap vs arena rendering to verify correctness.
//!
//! Run with: cargo run --example arena_compare --release

use ascii_dag::graph::arena::Arena;
use ascii_dag::graph::{Graph, RenderMode};

fn main() {
    println!("=== Arena vs Heap Rendering Comparison ===\n");
    println!("(Note: Both use Vertical mode for fair comparison)\n");

    let tests: Vec<(&str, Graph)> = vec![
        ("Simple Chain", {
            Graph::from_edges(&[(1, "A"), (2, "B"), (3, "C")], &[(1, 2), (2, 3)])
                .with_render_mode(RenderMode::Vertical)
        }),
        ("Diamond Pattern", {
            Graph::from_edges(
                &[(1, "Root"), (2, "Left"), (3, "Right"), (4, "Merge")],
                &[(1, 2), (1, 3), (2, 4), (3, 4)],
            )
            .with_render_mode(RenderMode::Vertical)
        }),
        ("Multi-Convergence", {
            Graph::from_edges(
                &[(1, "E1"), (2, "E2"), (3, "E3"), (4, "Final")],
                &[(1, 4), (2, 4), (3, 4)],
            )
            .with_render_mode(RenderMode::Vertical)
        }),
        ("Cross-Level Simple", {
            let mut dag = Graph::new().with_render_mode(RenderMode::Vertical);
            dag.add_node(1, "Root");
            dag.add_node(2, "Middle");
            dag.add_node(3, "End");
            dag.add_edge(1, 2, None);
            dag.add_edge(2, 3, None);
            dag.add_edge(1, 3, None); // cross-level
            dag
        }),
        ("Cross-Level Chain", {
            let mut dag = Graph::new().with_render_mode(RenderMode::Vertical);
            dag.add_node(1, "A");
            dag.add_node(2, "B");
            dag.add_node(3, "C");
            dag.add_node(4, "D");
            dag.add_edge(1, 2, None);
            dag.add_edge(2, 3, None);
            dag.add_edge(3, 4, None);
            dag.add_edge(1, 4, None); // cross-level
            dag
        }),
        ("Readme Hero", {
            let mut dag = Graph::new().with_render_mode(RenderMode::Vertical);
            dag.add_node(1, "Root");
            dag.add_node(2, "Task A");
            dag.add_node(3, "Task B");
            dag.add_node(4, "Task C");
            dag.add_node(5, "Task D");
            dag.add_node(6, "Task E");
            dag.add_node(7, "Task F");
            dag.add_node(8, "Output");
            dag.add_edge(1, 2, None);
            dag.add_edge(1, 3, None);
            dag.add_edge(1, 4, None);
            dag.add_edge(1, 5, None);
            dag.add_edge(1, 6, None);
            dag.add_edge(2, 7, None);
            dag.add_edge(3, 7, None);
            dag.add_edge(4, 7, None);
            dag.add_edge(5, 7, None);
            dag.add_edge(7, 8, None);
            dag.add_edge(6, 8, None);
            dag
        }),
        ("Wide Fan", {
            let mut dag = Graph::new().with_render_mode(RenderMode::Vertical);
            dag.add_node(0, "Source");
            dag.add_node(1, "Sink");
            for i in 2..12 {
                dag.add_node(i, Box::leak(format!("W{}", i - 1).into_boxed_str()));
                dag.add_edge(0, i, None);
                dag.add_edge(i, 1, None);
            }
            dag
        }),
    ];

    for (name, dag) in tests {
        println!("=== {} ===", name);

        // Heap rendering
        let heap_output = dag.render();

        // Arena rendering
        let arena_output = render_with_arena(&dag);

        println!("--- HEAP ---");
        println!("{}", heap_output);

        println!("--- ARENA ---");
        println!("{}", arena_output);

        // Compare
        if heap_output.trim() == arena_output.trim() {
            println!("✅ MATCH!\n");
        } else {
            println!("❌ MISMATCH!\n");
            println!(
                "Heap lines: {}, Arena lines: {}",
                heap_output.lines().count(),
                arena_output.lines().count()
            );
        }
        println!("{}", "=".repeat(60));
        println!();
    }
}

fn render_with_arena(dag: &Graph) -> String {
    // Use estimate_size() to get a reasonable arena size estimate
    let estimated = dag.estimate_size();

    // Temp arena: proportional to estimated output size
    let temp_size = estimated * 4 + 32768;
    // Output arena: similar
    let output_size = estimated * 4 + 32768;

    let mut temp_buffer = vec![0u8; temp_size];
    let mut output_buffer = vec![0u8; output_size];

    let mut temp_arena = Arena::new(&mut temp_buffer);
    let mut output_arena = Arena::new(&mut output_buffer);

    if let Ok(layout) = dag.compute_layout_arena(&mut temp_arena, &mut output_arena) {
        // Render buffer
        let (render_size, scratch_size) = layout.estimate_render_size();
        let mut render_buffer = vec![0u8; render_size + 1024];
        let mut scratch_buffer = vec![0usize; scratch_size + 1024];
        let mut line_buffer = vec![' '; layout.width() + 16];

        if let Some(bytes) =
            layout.render_to_buffer(&mut render_buffer, &mut line_buffer, &mut scratch_buffer)
        {
            String::from_utf8_lossy(&render_buffer[..bytes]).into_owned()
        } else {
            "(Render failed)".to_string()
        }
    } else {
        "(Layout failed)".to_string()
    }
}
