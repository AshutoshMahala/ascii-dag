use ascii_dag::DAG;

fn main() {
    let mut dag = DAG::new();
    dag.add_node(1, "Parser");
    dag.add_node(2, "Lexer");
    dag.add_node(3, "AST");
    dag.add_node(4, "CodeGen");

    dag.add_edge(1, 2, Some("uses"));
    dag.add_edge(1, 3, Some("produces"));
    dag.add_edge(3, 4, Some("feeds"));
    dag.add_edge(2, 3, None);

    let ir = dag.compute_layout();

    println!("Edges with label positions:");
    for edge in ir.edges() {
        if let Some(label) = edge.label {
            println!(
                "  Edge {}->{}: label='{}' from_y={} to_y={} path={:?} label_pos={:?}",
                edge.from_id,
                edge.to_id,
                label,
                edge.from_y,
                edge.to_y,
                edge.path,
                edge.label_position
            );
        }
    }
}
