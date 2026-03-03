use ascii_dag::Graph;
use ascii_dag::render::colors::Palette;

fn main() {
    println!("=== ASCII-DAG Color Demo ===\n");

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
