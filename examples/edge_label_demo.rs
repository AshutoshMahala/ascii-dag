use ascii_dag::Graph;
use ascii_dag::render::colors::Palette;

fn main() {
    // Create a DAG with edge labels
    let mut dag = Graph::new();
    dag.add_node(1, "Parser");
    dag.add_node(2, "Lexer");
    dag.add_node(3, "AST");
    dag.add_node(4, "CodeGen");

    // Add labeled edges
    dag.add_edge(1, 2, Some("uses"));
    dag.add_edge(1, 3, Some("produces"));
    dag.add_edge(3, 4, Some("feeds"));
    dag.add_edge(2, 3, None); // Edge without label

    println!("DAG with edge labels:");
    println!("{}", dag.render());

    // Scanline render with ANSI colors
    println!("\nScanline render with colors:");
    let ir = dag.compute_layout();
    println!("{}", ir.render_scanline_colored(Palette::Ansi));

    // Try dark mode palette
    println!("Dark mode palette:");
    println!("{}", ir.render_scanline_colored(Palette::AnsiDark));

    // Demonstrate legend feature with a more complex graph
    // where labels might collide
    println!("\n--- Legend feature demo (complex graph) ---\n");

    let mut dag2 = Graph::new();
    dag2.add_node(1, "A");
    dag2.add_node(2, "B");
    dag2.add_node(3, "C");
    dag2.add_node(4, "D");
    dag2.add_node(5, "E");

    // Multiple edges from same node - labels may collide
    dag2.add_edge(1, 2, Some("produces"));
    dag2.add_edge(1, 3, Some("consumes"));
    dag2.add_edge(1, 4, Some("requires"));
    dag2.add_edge(2, 5, Some("outputs"));
    dag2.add_edge(3, 5, Some("feeds"));
    dag2.add_edge(4, 5, Some("generates"));

    let ir2 = dag2.compute_layout();
    println!("With legend for skipped labels:");
    println!("{}", ir2.render_scanline_colored_with_legend(Palette::Ansi));
}
