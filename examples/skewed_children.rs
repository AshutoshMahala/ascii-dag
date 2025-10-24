use ascii_dag::graph::DAG;

fn main() {
    // Create a graph where a level's children are clustered on opposite sides
    // This tests that nodes are rendered in x-coordinate order (left-to-right)
    let mut dag = DAG::new();

    // Level 0: Root nodes
    dag.add_node(1, "LeftRoot");
    dag.add_node(2, "RightRoot");

    // Level 1: Middle nodes - initially in median order
    dag.add_node(3, "A");
    dag.add_node(4, "B");
    dag.add_node(5, "C");

    // Level 2: Children clustered on opposite sides
    // LeftRoot connects to far-right children
    dag.add_edge(1, 6);
    dag.add_edge(1, 7);

    // RightRoot connects to far-left children
    dag.add_edge(2, 8);
    dag.add_edge(2, 9);

    // Middle nodes connect normally
    dag.add_edge(3, 6);
    dag.add_edge(4, 8);
    dag.add_edge(5, 9);

    dag.add_node(6, "X");
    dag.add_node(7, "Y");
    dag.add_node(8, "P");
    dag.add_node(9, "Q");

    println!("{}", dag.render());

    println!("\n--- Analysis ---");
    println!("Level 0: LeftRoot should be on left, RightRoot on right");
    println!("Level 2: Nodes should be ordered left-to-right by their x-coordinates");
    println!("Spacing should always appear before the correct node, not after");
}
