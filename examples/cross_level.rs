// Example demonstrating cross-level edges
// Previously, edges that skipped levels were silently dropped
// Now they render correctly

use ascii_dag::graph::DAG;

fn main() {
    println!("=== Cross-Level Edge Examples ===\n");

    // Example 1: Simple cross-level edge
    println!("1. Simple cross-level (Root -> Middle -> End, plus Root -> End directly):");
    let mut dag = DAG::new();
    dag.add_node(1, "Root");
    dag.add_node(2, "Middle");
    dag.add_node(3, "End");

    dag.add_edge(1, 2, None); // Root -> Middle
    dag.add_edge(2, 3, None); // Middle -> End
    dag.add_edge(1, 3, None); // Root -> End (cross-level!)

    println!("{}\n", dag.render());

    // Example 2: Multiple cross-level edges
    println!("2. Multiple cross-level edges:");
    let mut dag = DAG::new();
    dag.add_node(1, "A");
    dag.add_node(2, "B");
    dag.add_node(3, "C");
    dag.add_node(4, "D");

    dag.add_edge(1, 2, None); // Level 0 -> 1
    dag.add_edge(2, 3, None); // Level 1 -> 2
    dag.add_edge(3, 4, None); // Level 2 -> 3
    dag.add_edge(1, 4, None); // Level 0 -> 3 (cross-level!)

    println!("{}\n", dag.render());

    // Example 3: Complex with multiple paths
    println!("3. Complex graph with shortcuts:");
    let mut dag = DAG::new();
    dag.add_node(1, "Start");
    dag.add_node(2, "Parse");
    dag.add_node(3, "Compile");
    dag.add_node(4, "Link");
    dag.add_node(5, "Done");

    dag.add_edge(1, 2, None); // Normal flow
    dag.add_edge(2, 3, None);
    dag.add_edge(3, 4, None);
    dag.add_edge(4, 5, None);
    dag.add_edge(1, 5, None); // Direct shortcut from Start to Done!

    println!("{}\n", dag.render());
}
