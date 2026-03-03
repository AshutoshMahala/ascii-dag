use ascii_dag::graph::Graph;

fn main() {
    // Prevent optimization of the whole program
    let args: Vec<String> = std::env::args().collect();

    // Build a simple graph
    let mut dag = Graph::new();
    dag.add_node(1, "Root");
    dag.add_node(2, "Child");
    dag.add_node(3, "Leaf");
    dag.add_edge(1, 2, None);
    dag.add_edge(2, 3, None);

    // Render
    let output = dag.render();

    // Use output to prevent dead code elimination
    if args.len() > 100 {
        println!("{}", output);
    }
}
