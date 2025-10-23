// Clear test case for cross-level edges
use ascii_dag::DAG;

fn main() {
    println!("=== Testing Cross-Level Edge Rendering ===\n");

    // What we want to see:
    // Level 0: A
    // Level 1: B (A->B)
    // Level 2: C (B->C AND A->C directly)
    //
    // Expected: A should have TWO edges - one to B, one to C

    println!("Graph structure:");
    println!("  A (level 0)");
    println!("  ├─→ B (level 1) - normal edge");
    println!("  │   └─→ C (level 2) - normal edge");
    println!("  └─────→ C (level 2) - CROSS-LEVEL edge (skips level 1)\n");

    let mut dag = DAG::new();
    dag.add_node(1, "A");
    dag.add_node(2, "B");
    dag.add_node(3, "C");

    dag.add_edge(1, 2); // A -> B (level 0 -> 1)
    dag.add_edge(2, 3); // B -> C (level 1 -> 2)
    dag.add_edge(1, 3); // A -> C (level 0 -> 2, CROSS-LEVEL!)

    println!("Actual rendering:");
    println!("{}", dag.render());

    println!("\n---\n");

    // Another clear example
    println!("Second test - three levels with cross-level:");
    println!("  Root (level 0)");
    println!("  ├─→ Mid (level 1)");
    println!("  │   └─→ End (level 2)");
    println!("  └─────→ End (level 2) - CROSS-LEVEL\n");

    let mut dag2 = DAG::new();
    dag2.add_node(1, "Root");
    dag2.add_node(2, "Mid");
    dag2.add_node(3, "End");

    dag2.add_edge(1, 2); // Root -> Mid
    dag2.add_edge(2, 3); // Mid -> End
    dag2.add_edge(1, 3); // Root -> End (CROSS-LEVEL)

    println!("Actual rendering:");
    println!("{}", dag2.render());

    println!("\nDoes 'End' show TWO incoming edges (one from Mid, one from Root)?");
}
