use ascii_dag::Graph;
use ascii_dag::render::colors::Palette;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let use_csr = args.iter().any(|a| a == "--csr");

    if use_csr {
        println!("=== ASCII-DAG Color Demo (CSR Mode) ===\n");
        run_csr();
    } else {
        println!("=== ASCII-DAG Color Demo (Heap Mode) ===\n");
        run_heap();
    }
}

fn run_heap() {
    // Example 1: Simple dependency graph with multiple edges
    println!("1. Build Pipeline:");
    let mut dag = Graph::new();
    dag.add_node(1, "Source");
    dag.add_node(2, "Compile");
    dag.add_node(3, "Test");
    dag.add_node(4, "Package");
    dag.add_node(5, "Deploy");

    dag.add_edge(1, 2, None);
    dag.add_edge(2, 3, None);
    dag.add_edge(2, 4, None);
    dag.add_edge(3, 5, None);
    dag.add_edge(4, 5, None);

    let ir = dag.compute_layout();
    println!("{}", ir.render_scanline_colored(Palette::Ansi));

    // Example 2: Diamond dependency (shows edge colors clearly)
    println!("2. Diamond Pattern:");
    let mut diamond = Graph::new();
    diamond.add_node(1, "A");
    diamond.add_node(2, "B");
    diamond.add_node(3, "C");
    diamond.add_node(4, "D");

    diamond.add_edge(1, 2, None);
    diamond.add_edge(1, 3, None);
    diamond.add_edge(2, 4, None);
    diamond.add_edge(3, 4, None);

    let ir = diamond.compute_layout();
    println!("{}", ir.render_scanline_colored(Palette::Ansi));

    // Example 3: More complex graph
    println!("3. Module Dependencies:");
    let mut modules = Graph::new();
    modules.add_node(1, "app");
    modules.add_node(2, "auth");
    modules.add_node(3, "db");
    modules.add_node(4, "cache");
    modules.add_node(5, "utils");
    modules.add_node(6, "config");

    modules.add_edge(1, 2, None);
    modules.add_edge(1, 3, None);
    modules.add_edge(1, 4, None);
    modules.add_edge(2, 5, None);
    modules.add_edge(3, 5, None);
    modules.add_edge(4, 5, None);
    modules.add_edge(5, 6, None);

    let ir = modules.compute_layout();
    println!("{}", ir.render_scanline_colored(Palette::Ansi));

    // Show different palettes
    println!("4. Same graph with different palettes:\n");

    println!("Default (Ansi):");
    println!("{}", ir.render_scanline_colored(Palette::Ansi));

    println!("Dark Mode (AnsiDark):");
    println!("{}", ir.render_scanline_colored(Palette::AnsiDark));

    println!("Light Mode (AnsiLight):");
    println!("{}", ir.render_scanline_colored(Palette::AnsiLight));
}

#[cfg(feature = "arena")]
fn run_csr() {
    use ascii_dag::graph::arena::Arena;
    use ascii_dag::LayoutConfig;

    // Helper: build CsrGraph from a Graph, layout via arena, render colored
    fn render_csr_colored(dag: &Graph, palette: Palette) {
        let csr_size = dag.estimate_csr_arena_size() * 2;
        let mut csr_buf = vec![0u8; csr_size];
        let mut csr_arena = Arena::new(&mut csr_buf);
        let csr = dag.to_csr(&mut csr_arena).expect("CSR conversion failed");

        let layout_size = dag.estimate_layout_arena_size();
        let size = ((layout_size * 6) / 5).max(128 * 1024);
        let mut temp_buf = vec![0u8; size];
        let mut out_buf = vec![0u8; size];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_arena = Arena::new(&mut out_buf);

        let ir = csr
            .compute_layout_arena(&LayoutConfig::standard(), &mut temp_arena, &mut out_arena)
            .expect("Layout failed");

        let mut edge_colors = vec![0usize; ir.edge_count()];
        let palette_colors = palette.colors();
        ir.compute_edge_colors(&mut edge_colors, palette_colors.len());

        let (render_bytes, _) = ir.estimate_render_size();
        let render_size = render_bytes * 10 + 4096;
        let mut render_buffer = vec![0u8; render_size];
        let mut line_buffer = vec![' '; ir.width().max(1) + 16];
        let mut color_buffer = vec![0u8; ir.width().max(1) + 16];
        let mut skipped_buffer = vec![false; ir.edge_count().max(1)];

        if let Some(len) = ir.render_to_buffer_colored_with_legend(
            &mut render_buffer,
            &mut line_buffer,
            &mut color_buffer,
            &edge_colors,
            palette_colors,
            &mut skipped_buffer,
        ) {
            if let Ok(s) = std::str::from_utf8(&render_buffer[..len]) {
                println!("{}", s);
            }
        }
    }

    // Example 1
    println!("1. Build Pipeline:");
    let mut dag = Graph::new();
    dag.add_node(1, "Source");
    dag.add_node(2, "Compile");
    dag.add_node(3, "Test");
    dag.add_node(4, "Package");
    dag.add_node(5, "Deploy");
    dag.add_edge(1, 2, None);
    dag.add_edge(2, 3, None);
    dag.add_edge(2, 4, None);
    dag.add_edge(3, 5, None);
    dag.add_edge(4, 5, None);
    render_csr_colored(&dag, Palette::Ansi);

    // Example 2
    println!("2. Diamond Pattern:");
    let mut diamond = Graph::new();
    diamond.add_node(1, "A");
    diamond.add_node(2, "B");
    diamond.add_node(3, "C");
    diamond.add_node(4, "D");
    diamond.add_edge(1, 2, None);
    diamond.add_edge(1, 3, None);
    diamond.add_edge(2, 4, None);
    diamond.add_edge(3, 4, None);
    render_csr_colored(&diamond, Palette::Ansi);

    // Example 3
    println!("3. Module Dependencies:");
    let mut modules = Graph::new();
    modules.add_node(1, "app");
    modules.add_node(2, "auth");
    modules.add_node(3, "db");
    modules.add_node(4, "cache");
    modules.add_node(5, "utils");
    modules.add_node(6, "config");
    modules.add_edge(1, 2, None);
    modules.add_edge(1, 3, None);
    modules.add_edge(1, 4, None);
    modules.add_edge(2, 5, None);
    modules.add_edge(3, 5, None);
    modules.add_edge(4, 5, None);
    modules.add_edge(5, 6, None);

    println!("  (showing Ansi palette)");
    render_csr_colored(&modules, Palette::Ansi);

    println!("4. Same graph with different palettes:\n");
    println!("Default (Ansi):");
    render_csr_colored(&modules, Palette::Ansi);
    println!("Dark Mode (AnsiDark):");
    render_csr_colored(&modules, Palette::AnsiDark);
    println!("Light Mode (AnsiLight):");
    render_csr_colored(&modules, Palette::AnsiLight);
}

#[cfg(not(feature = "arena"))]
fn run_csr() {
    println!("(arena feature not enabled — run with --features arena)");
}
