use ascii_dag::RenderOptions;
use ascii_dag::graph::Graph;

#[path = "support/csr.rs"]
mod csr;

/// Print through the heap path, or the CSR/arena pipeline with --csr.
fn show(dag: &Graph<'_>) {
    if csr::requested() {
        println!("{}\n", csr::render(dag, &RenderOptions::plain()));
    } else {
        println!("{}\n", dag.render());
    }
}

fn main() {
    println!("=== Basic Usage Examples ===\n");

    // Example 1: Simple chain
    println!("1. Simple Chain (A -> B -> C):");
    let dag = Graph::from_edges(&[(1, "A"), (2, "B"), (3, "C")], &[(1, 2), (2, 3)]);
    show(&dag);

    // Example 2: Diamond pattern
    println!("2. Diamond Pattern:");
    let dag = Graph::from_edges(
        &[(1, "Root"), (2, "Left"), (3, "Right"), (4, "Merge")],
        &[(1, 2), (1, 3), (2, 4), (3, 4)],
    );
    show(&dag);

    // Example 3: Builder API
    println!("3. Builder API:");
    let mut dag = Graph::new();
    dag.add_node(1, "Parse");
    dag.add_node(2, "Compile");
    dag.add_node(3, "Link");
    dag.add_edge(1, 2, None);
    dag.add_edge(2, 3, None);
    show(&dag);

    // Example 4: Multi-convergence
    println!("4. Multi-Convergence:");
    let dag = Graph::from_edges(
        &[(1, "E1"), (2, "E2"), (3, "E3"), (4, "Final")],
        &[(1, 4), (2, 4), (3, 4)],
    );
    show(&dag);
}
