use ascii_dag::graph::DAG;

fn main() {
    println!("=== Testing Parallel Chains ===\n");

    // Two independent chains
    let dag = DAG::from_edges(
        &[
            (1, "Chain1-A"),
            (2, "Chain1-B"),
            (3, "Chain1-C"),
            (4, "Chain2-A"),
            (5, "Chain2-B"),
            (6, "Chain2-C"),
        ],
        &[
            (1, 2),
            (2, 3), // Chain 1
            (4, 5),
            (5, 6), // Chain 2 (no connection to chain 1)
        ],
    );

    println!("Two parallel chains:");
    println!("{}", dag.render());

    // Three independent chains
    let dag = DAG::from_edges(
        &[
            (1, "A1"),
            (2, "A2"),
            (3, "B1"),
            (4, "B2"),
            (5, "C1"),
            (6, "C2"),
        ],
        &[(1, 2), (3, 4), (5, 6)],
    );

    println!("\nThree parallel chains:");
    println!("{}", dag.render());

    // Single chain (control)
    let dag = DAG::from_edges(&[(1, "X"), (2, "Y"), (3, "Z")], &[(1, 2), (2, 3)]);

    println!("\nSingle chain (control):");
    println!("{}", dag.render());
}
