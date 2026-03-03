use ascii_dag::Graph;

fn main() {
    let mut dag = Graph::new();

    dag.add_node(1, "Root");
    dag.add_node(2, "Task A");
    dag.add_node(3, "Task B");
    dag.add_node(4, "Task C");
    dag.add_node(5, "Task D");
    dag.add_node(6, "Task E");
    dag.add_node(7, "Task F");
    dag.add_node(8, "Output");

    // Level 1 connections
    dag.add_edge(1, 2, None);
    dag.add_edge(1, 3, None);
    dag.add_edge(1, 4, None);
    dag.add_edge(1, 5, None);
    dag.add_edge(1, 6, None);

    // Converge on Task F
    dag.add_edge(2, 7, None);
    dag.add_edge(3, 7, None);
    dag.add_edge(4, 7, None);
    dag.add_edge(5, 7, None);

    // Final Output
    dag.add_edge(7, 8, None); // F -> Output
    dag.add_edge(6, 8, None); // E -> Output (Long jump/side path)

    println!("{}", dag.render());
}
