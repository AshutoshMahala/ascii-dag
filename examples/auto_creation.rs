// Example demonstrating the auto-creation feature
// When you add edges to nodes that don't exist, they are automatically created
// and shown with angle brackets ⟨⟩ instead of square brackets []

use ascii_dag::DAG;

fn main() {
    println!("=== Auto-Creation Feature Examples ===\n");

    // Example 1: Missing target node
    println!("1. Adding edge to missing node (auto-created):");
    let mut dag = DAG::new();
    dag.add_node(1, "Defined");
    dag.add_edge(1, 2); // Node 2 doesn't exist - will be auto-created
    println!("{}\n", dag.render());

    // Example 2: Both nodes missing
    println!("2. Both nodes missing (both auto-created):");
    let mut dag = DAG::new();
    dag.add_edge(10, 20); // Neither node exists - both will be auto-created
    println!("{}\n", dag.render());

    // Example 3: Mixed explicit and auto-created
    println!("3. Mixed defined and auto-created nodes:");
    let mut dag = DAG::new();
    dag.add_node(1, "Start");
    dag.add_node(3, "End");
    dag.add_edge(1, 2); // Node 2 auto-created
    dag.add_edge(2, 3); // Node 2 already auto-created, Node 3 exists
    println!("{}\n", dag.render());

    // Example 4: Complex graph with some auto-created nodes
    println!("4. Complex graph with auto-created nodes:");
    let mut dag = DAG::new();
    dag.add_node(1, "Root");
    dag.add_node(5, "Leaf");
    dag.add_edge(1, 2); // 2 auto-created
    dag.add_edge(1, 3); // 3 auto-created
    dag.add_edge(2, 4); // 4 auto-created
    dag.add_edge(3, 4); // 4 already auto-created
    dag.add_edge(4, 5); // 5 exists
    println!("{}\n", dag.render());

    println!("Note: Nodes with ⟨⟩ brackets were auto-created.");
    println!("      Nodes with [] brackets were explicitly defined.");
}
