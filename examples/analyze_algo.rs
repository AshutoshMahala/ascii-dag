use ascii_dag::graph::Graph;

fn main() {
    // Test 1: Cross connections (bipartite K2,2)
    println!("=== Cross Connections (K2,2) ===");
    let dag = Graph::from_edges(
        &[(1, "A1"), (2, "A2"), (3, "B1"), (4, "B2")],
        &[(1, 3), (1, 4), (2, 3), (2, 4)], // Full cross
    );
    println!("{}", dag.render());

    // Test 2: Diamond - your algorithm should handle this well
    println!("=== Diamond ===");
    let dag = Graph::from_edges(
        &[(1, "Top"), (2, "Left"), (3, "Right"), (4, "Bottom")],
        &[(1, 2), (1, 3), (2, 4), (3, 4)],
    );
    println!("{}", dag.render());

    // Test 3: Skip with parallel paths
    println!("=== Skip + Parallel ===");
    let dag = Graph::from_edges(
        &[(1, "A"), (2, "B"), (3, "C"), (4, "D")],
        &[(1, 2), (2, 3), (3, 4), (1, 4)], // A->D skips 2 levels
    );
    println!("{}", dag.render());
}
