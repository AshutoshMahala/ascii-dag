use ascii_dag::Graph;
use ascii_dag::render::colors::Palette;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let use_csr = args.iter().any(|a| a == "--csr");

    if use_csr {
        println!("=== Edge Label Demo (CSR Mode) ===\n");
        run_csr();
    } else {
        run_heap();
    }
}

fn build_simple_dag() -> Graph<'static> {
    let mut dag = Graph::new();
    dag.add_node(1, "Parser");
    dag.add_node(2, "Lexer");
    dag.add_node(3, "AST");
    dag.add_node(4, "CodeGen");
    dag.add_edge(1, 2, Some("uses"));
    dag.add_edge(1, 3, Some("produces"));
    dag.add_edge(3, 4, Some("feeds"));
    dag.add_edge(2, 3, None);
    dag
}

fn build_complex_dag() -> Graph<'static> {
    let mut dag2 = Graph::new();
    dag2.add_node(1, "A");
    dag2.add_node(2, "B");
    dag2.add_node(3, "C");
    dag2.add_node(4, "D");
    dag2.add_node(5, "E");
    dag2.add_edge(1, 2, Some("produces"));
    dag2.add_edge(1, 3, Some("consumes"));
    dag2.add_edge(1, 4, Some("requires"));
    dag2.add_edge(2, 5, Some("outputs"));
    dag2.add_edge(3, 5, Some("feeds"));
    dag2.add_edge(4, 5, Some("generates"));
    dag2
}

fn run_heap() {
    let dag = build_simple_dag();
    println!("DAG with edge labels:");
    println!("{}", dag.render());

    println!("\nScanline render with colors:");
    let ir = dag.compute_layout();
    println!("{}", ir.render_scanline_colored(Palette::Ansi));

    println!("Dark mode palette:");
    println!("{}", ir.render_scanline_colored(Palette::AnsiDark));

    println!("\n--- Legend feature demo (complex graph) ---\n");
    let dag2 = build_complex_dag();
    let ir2 = dag2.compute_layout();
    println!("With legend for skipped labels:");
    println!("{}", ir2.render_scanline_colored_with_legend(Palette::Ansi));
}

#[cfg(feature = "arena")]
fn run_csr() {
    use ascii_dag::graph::arena::Arena;
    use ascii_dag::LayoutConfig;

    fn render_csr_with_legend(dag: &Graph, palette: Palette) {
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
            println!("{}", std::str::from_utf8(&render_buffer[..len]).unwrap());
        }
    }

    let dag = build_simple_dag();
    println!("DAG with edge labels (CSR colored with legend):");
    render_csr_with_legend(&dag, Palette::Ansi);

    println!("Dark mode palette:");
    render_csr_with_legend(&dag, Palette::AnsiDark);

    println!("\n--- Legend feature demo (complex graph) ---\n");
    let dag2 = build_complex_dag();
    println!("With legend for skipped labels:");
    render_csr_with_legend(&dag2, Palette::Ansi);
}

#[cfg(not(feature = "arena"))]
fn run_csr() {
    println!("(arena feature not enabled — run with --features arena)");
}
