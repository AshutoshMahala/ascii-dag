use ascii_dag::graph::DAG;

fn main() {
    // Test 1: Simple skip edge (A->D spanning 3 levels)
    println!("=== Test 1: Simple Skip Edge (A->D spans 3 levels) ===");
    let dag = DAG::from_edges(
        &[(1, "A"), (2, "B"), (3, "C"), (4, "D")],
        &[(1, 2), (2, 3), (3, 4), (1, 4)], // A->D is the skip edge
    );
    println!("{}", dag.render());
    println!();

    // Test 2: Two parallel paths with different lengths
    println!("=== Test 2: Asymmetric Parallel Paths ===");
    let dag = DAG::from_edges(
        &[
            (1, "Root"),
            (2, "Short"),
            (3, "Long1"),
            (4, "Long2"),
            (5, "End"),
        ],
        &[(1, 2), (1, 3), (2, 5), (3, 4), (4, 5)], // Short path: Root->Short->End, Long: Root->Long1->Long2->End
    );
    println!("{}", dag.render());
    println!();

    // Test 3: Cross connections (should show crossing pattern)
    println!("=== Test 3: Cross Connections (K2,2) ===");
    let dag = DAG::from_edges(
        &[(1, "A1"), (2, "A2"), (3, "B1"), (4, "B2")],
        &[(1, 3), (1, 4), (2, 3), (2, 4)],
    );
    println!("{}", dag.render());
    println!();

    // Test 4: Diamond (pure convergence then divergence)
    println!("=== Test 4: Diamond ===");
    let dag = DAG::from_edges(
        &[(1, "Top"), (2, "Left"), (3, "Right"), (4, "Bottom")],
        &[(1, 2), (1, 3), (2, 4), (3, 4)],
    );
    println!("{}", dag.render());
    println!();

    // Test 5: Wide graph with skip edge
    println!("=== Test 5: Wide Graph with Skip ===");
    let dag = DAG::from_edges(
        &[(1, "A"), (2, "B"), (3, "C"), (4, "X"), (5, "Y"), (6, "Z")],
        &[
            (1, 4),
            (2, 5),
            (3, 6), // A->X, B->Y, C->Z
            (4, 5),
            (5, 6), // X->Y->Z chain
            (1, 6), // A->Z skip edge (skip 2 levels)
        ],
    );
    println!("{}", dag.render());
}
