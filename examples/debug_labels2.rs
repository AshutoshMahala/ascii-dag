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

    println!("IR dimensions: {}x{}", ir.width(), ir.height());
    println!("\nEdges:");
    for edge in ir.edges() {
        println!(
            "  {}->{}: from_x={} to_x={} path={:?}",
            edge.from_id, edge.to_id, edge.from_x, edge.to_x, edge.path
        );
        if let Some(label) = edge.label {
            println!(
                "         label='{}' label_pos={:?}",
                label, edge.label_position
            );
        }
    }
}
