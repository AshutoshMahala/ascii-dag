use ascii_dag::DAG;

fn main() {
    let mut dag = DAG::new();
    
    dag.add_node(1, "Root");
    dag.add_node(2, "Task A");
    dag.add_node(3, "Task B");
    dag.add_node(4, "Task C");
    dag.add_node(5, "Task D");
    dag.add_node(6, "Task E");
    dag.add_node(7, "Task F");
    dag.add_node(8, "Output");

    // Level 1 connections
    dag.add_edge(1, 2);
    dag.add_edge(1, 3);
    dag.add_edge(1, 4);
    dag.add_edge(1, 5);
    dag.add_edge(1, 6);

    // Converge on Task F
    dag.add_edge(2, 7);
    dag.add_edge(3, 7);
    dag.add_edge(4, 7);
    dag.add_edge(5, 7);
    
    // Final Output
    dag.add_edge(7, 8); // F -> Output
    dag.add_edge(6, 8); // E -> Output (Long jump/side path)

    println!("{}", dag.render());
}
