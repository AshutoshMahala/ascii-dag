use ascii_dag::DAG;

fn main() {
    let mut dag = DAG::new();

    println!("=== Test: Auto-created Node Promotion ===\n");

    dag.add_node(1, "Start");
    dag.add_edge(1, 2); // Auto-creates node 2

    println!("Before promotion (node 2 is auto-created):");
    println!("{}\n", dag.render());

    // Now promote node 2
    dag.add_node(2, "Promoted");

    println!("After promotion (node 2 now has label 'Promoted'):");
    println!("{}\n", dag.render());

    println!("✓ Node 2 successfully promoted from ⟨2⟩ to [Promoted]");
}
