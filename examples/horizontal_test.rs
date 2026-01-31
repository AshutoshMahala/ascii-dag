use ascii_dag::graph::{DAG, RenderMode};

fn main() {
    println!("=== Horizontal Mode Test ===\n");

    let mut dag = DAG::new();
    dag.set_render_mode(RenderMode::Horizontal);

    println!("1. Simple Chain (Should work):");
    dag.add_node(1, "A");
    dag.add_node(2, "B");
    dag.add_node(3, "C");
    dag.add_edge(1, 2, None);
    dag.add_edge(2, 3, None);
    println!("{}", dag.render());

    println!("2. Branching (Potential Data Loss):");
    let mut dag2 = DAG::new();
    dag2.set_render_mode(RenderMode::Horizontal);
    dag2.add_node(10, "Root");
    dag2.add_node(11, "Branch_1");
    dag2.add_node(12, "Branch_2");

    dag2.add_edge(10, 11, None);
    dag2.add_edge(10, 12, None); // The ignored child?

    println!("{}", dag2.render());
}
