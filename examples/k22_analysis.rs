use ascii_dag::graph::DAG;

fn main() {
    // K2,2 bipartite graph: A1, A2 both connect to B1, B2
    println!("=== K2,2 Analysis ===");
    println!("Edges: A1→B1, A1→B2, A2→B1, A2→B2");
    println!();

    let dag = DAG::from_edges(
        &[(1, "A1"), (2, "A2"), (3, "B1"), (4, "B2")],
        &[(1, 3), (1, 4), (2, 3), (2, 4)],
    );

    println!("Rendered:");
    println!("{}", dag.render());

    println!("Expected visualization challenge:");
    println!("- A1→B1: straight down (same column)");
    println!("- A1→B2: goes right");
    println!("- A2→B1: goes left (crosses A1→B2!)");
    println!("- A2→B2: straight down (same column)");
    println!();
    println!("In ASCII, crossing lines are hard to show.");
    println!("The current output shows convergence which captures");
    println!("that multiple sources connect to each target.");
}
