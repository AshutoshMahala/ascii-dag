use ascii_dag::graph::DAG;

fn main() {
    println!("=== Basic Usage Examples ===\n");

    // Example 1: Simple chain
    println!("1. Simple Chain (A -> B -> C):");
    let dag = DAG::from_edges(&[(1, "A"), (2, "B"), (3, "C")], &[(1, 2), (2, 3)]);
    println!("{}\n", dag.render());

    // Example 2: Diamond pattern
    println!("2. Diamond Pattern:");
    let dag = DAG::from_edges(
        &[(1, "Root"), (2, "Left"), (3, "Right"), (4, "Merge")],
        &[(1, 2), (1, 3), (2, 4), (3, 4)],
    );
    println!("{}\n", dag.render());

    // Example 3: Builder API
    println!("3. Builder API:");
    let mut dag = DAG::new();
    dag.add_node(1, "Parse");
    dag.add_node(2, "Compile");
    dag.add_node(3, "Link");
    dag.add_edge(1, 2);
    dag.add_edge(2, 3);
    println!("{}\n", dag.render());

    // Example 4: Multi-convergence
    println!("4. Multi-Convergence:");
    let dag = DAG::from_edges(
        &[(1, "E1"), (2, "E2"), (3, "E3"), (4, "Final")],
        &[(1, 4), (2, 4), (3, 4)],
    );
    println!("{}\n", dag.render());
}
