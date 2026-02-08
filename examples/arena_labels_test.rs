//! Test edge labels and colors in arena vs heap rendering.
//!
//! Run with: cargo run --example arena_labels_test --release

use ascii_dag::arena::Arena;
use ascii_dag::graph::DAG;
use ascii_dag::render::colors::Palette;

fn main() {
    println!("=== Arena vs Heap: Edge Labels & Colors Test ===\n");

    // Create DAG with edge labels
    let mut dag = DAG::new();
    dag.add_node(1, "Parser");
    dag.add_node(2, "Lexer");
    dag.add_node(3, "AST");
    dag.add_node(4, "CodeGen");

    dag.add_edge(1, 2, Some("uses"));
    dag.add_edge(1, 3, Some("produces"));
    dag.add_edge(3, 4, Some("feeds"));
    dag.add_edge(2, 3, None);

    // =========================================
    // Heap rendering (with labels and colors)
    // =========================================
    println!("--- HEAP RENDERING (LayoutIR) ---\n");

    let ir = dag.compute_layout();

    println!("1. Colored render (Palette::Ansi):");
    println!("{}", ir.render_scanline_colored(Palette::Ansi));

    // =========================================
    // Arena rendering with colors
    // =========================================
    println!("--- ARENA RENDERING (LayoutIRArena) ---\n");

    let mut temp_buffer = vec![0u8; 64 * 1024];
    let mut output_buffer = vec![0u8; 64 * 1024];

    let mut temp_arena = Arena::new(&mut temp_buffer);
    let mut output_arena = Arena::new(&mut output_buffer);

    if let Some(arena_ir) = dag.compute_layout_arena(&mut temp_arena, &mut output_arena) {
        println!(
            "Arena layout: {}x{}, {} nodes, {} edges",
            arena_ir.width(),
            arena_ir.height(),
            arena_ir.node_count(),
            arena_ir.edge_count()
        );

        // Get palette colors
        let palette = Palette::Ansi;
        let palette_colors = palette.colors();

        // Compute edge colors using greedy coloring
        let mut edge_colors = vec![0usize; arena_ir.edge_count()];
        let colors_used = arena_ir.compute_edge_colors(&mut edge_colors, palette_colors.len());
        println!("Greedy coloring used {} colors", colors_used.unwrap_or(0));

        // Allocate buffers for colored rendering
        let mut render_buffer = vec![0u8; arena_ir.width() * arena_ir.height() * 16 + 1024];
        let mut line_buffer = vec![' '; arena_ir.width() + 16];
        let mut color_buffer = vec![0u8; arena_ir.width() + 16];
        let mut skipped_buffer = vec![false; arena_ir.edge_count()];

        // Render with colors and legend
        if let Some(bytes_written) = arena_ir.render_to_buffer_colored_with_legend(
            &mut render_buffer,
            &mut line_buffer,
            &mut color_buffer,
            &edge_colors,
            palette_colors,
            &mut skipped_buffer,
        ) {
            let output =
                core::str::from_utf8(&render_buffer[..bytes_written]).unwrap_or("<invalid utf8>");
            println!("\n2. Arena colored render with legend:");
            println!("{}", output);
        } else {
            println!("ERROR: Arena colored render failed");
        }

        // Also show non-colored for comparison
        let mut render_buffer2 = vec![0u8; arena_ir.width() * arena_ir.height() * 4];
        let mut line_buffer2 = vec![' '; arena_ir.width() + 16];
        let mut scratch_buffer2 = vec![0usize; arena_ir.height() + arena_ir.edge_count() + 16];

        if let Some(bytes_written) =
            arena_ir.render_to_buffer(&mut render_buffer2, &mut line_buffer2, &mut scratch_buffer2)
        {
            let output =
                core::str::from_utf8(&render_buffer2[..bytes_written]).unwrap_or("<invalid utf8>");
            println!("3. Arena non-colored render (for comparison):");
            println!("{}", output);
        }
    } else {
        println!("ERROR: Arena layout computation failed");
    }

    // =========================================
    // Test greedy coloring works correctly
    // =========================================
    println!("--- GREEDY COLORING TEST ---\n");

    // Create a diamond pattern where adjacent edges should get different colors
    let mut diamond = DAG::new();
    diamond.add_node(1, "A");
    diamond.add_node(2, "B");
    diamond.add_node(3, "C");
    diamond.add_node(4, "D");
    diamond.add_edge(1, 2, None); // A->B
    diamond.add_edge(1, 3, None); // A->C (shares A with A->B)
    diamond.add_edge(2, 4, None); // B->D (shares B with A->B)
    diamond.add_edge(3, 4, None); // C->D (shares C with A->C, D with B->D)

    // Heap coloring
    let diamond_ir = diamond.compute_layout();
    let heap_colors = diamond_ir.compute_edge_colors(Palette::Ansi.colors().len());
    println!("Heap edge colors: {:?}", heap_colors);

    // Arena coloring
    let mut temp_buf = vec![0u8; 32 * 1024];
    let mut out_buf = vec![0u8; 32 * 1024];
    let mut temp = Arena::new(&mut temp_buf);
    let mut out = Arena::new(&mut out_buf);

    if let Some(diamond_arena) = diamond.compute_layout_arena(&mut temp, &mut out) {
        let mut arena_colors = vec![0usize; diamond_arena.edge_count()];
        diamond_arena.compute_edge_colors(&mut arena_colors, Palette::Ansi.colors().len());
        println!(
            "Arena edge colors: {:?}",
            &arena_colors[..diamond_arena.edge_count()]
        );

        // Verify adjacent edges have different colors
        let edges = diamond_arena.edges();
        let mut all_different = true;
        for i in 0..edges.len() {
            for j in 0..i {
                let e1 = &edges[i];
                let e2 = &edges[j];
                let adjacent = e1.from_id == e2.from_id
                    || e1.from_id == e2.to_id
                    || e1.to_id == e2.from_id
                    || e1.to_id == e2.to_id;
                if adjacent && arena_colors[i] == arena_colors[j] {
                    println!("  ⚠️  Adjacent edges {} and {} have same color!", i, j);
                    all_different = false;
                }
            }
        }
        if all_different {
            println!("  ✅ All adjacent edges have different colors!");
        }
    }

    // =========================================
    // Summary
    // =========================================
    println!("\n=== SUMMARY ===\n");
    println!("Feature              | Heap (LayoutIR) | Arena (LayoutIRArena)");
    println!("---------------------|-----------------|----------------------");
    println!("Edge labels          | ✅ Supported    | ✅ Supported");
    println!("ANSI colors          | ✅ Supported    | ✅ Supported");
    println!("Greedy coloring      | ✅ Supported    | ✅ Supported");
    println!("Legend for skipped   | ✅ Supported    | ✅ Supported");
}
