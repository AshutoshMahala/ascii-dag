use ascii_dag::graph::DAG;

fn main() {
    println!("=== Cycle Detection Examples ===\n");

    // Example 1: Simple cycle A → B → C → A
    println!("1. Simple Cycle (A → B → C → A):");
    let mut dag = DAG::new();
    dag.add_node(1, "A");
    dag.add_node(2, "B");
    dag.add_node(3, "C");
    dag.add_edge(1, 2, None); // A → B
    dag.add_edge(2, 3, None); // B → C
    dag.add_edge(3, 1, None); // C → A (creates cycle!)

    println!("{}\n", dag.render());

    // Example 2: Self-referencing cycle
    println!("2. Self-Reference (A → A):");
    let mut dag = DAG::new();
    dag.add_node(1, "SelfRef");
    dag.add_edge(1, 1, None); // Points to itself

    println!("{}\n", dag.render());

    // Example 3: Longer cycle chain
    println!("3. Longer Cycle (E1 → E2 → E3 → E4 → E2):");
    let mut dag = DAG::new();
    dag.add_node(1, "Error1");
    dag.add_node(2, "Error2");
    dag.add_node(3, "Error3");
    dag.add_node(4, "Error4");
    dag.add_edge(1, 2, None); // E1 → E2
    dag.add_edge(2, 3, None); // E2 → E3
    dag.add_edge(3, 4, None); // E3 → E4
    dag.add_edge(4, 2, None); // E4 → E2 (cycle!)

    println!("{}\n", dag.render());

    // Example 4: Valid DAG (no cycle)
    println!("4. Valid DAG - No Cycle:");
    let dag = DAG::from_edges(
        &[(1, "Valid1"), (2, "Valid2"), (3, "Valid3")],
        &[(1, 2), (2, 3)],
    );

    println!("{}", dag.render());
}
