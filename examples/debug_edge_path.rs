use ascii_dag::graph::DAG;

fn main() {
    println!("=== Debug Edge Path Generation ===\n");

    // Simple skip edge test - A at level 0, D at level 3
    // A -> B -> C -> D (chain)
    // A -> D (skip edge)
    let dag = DAG::from_edges(
        &[(1, "A"), (2, "B"), (3, "C"), (4, "D")],
        &[(1, 2), (2, 3), (3, 4), (1, 4)],
    );
    
    println!("ASCII Render:");
    println!("{}", dag.render());
    println!();

    // Get the IR to see edge paths
    let ir = dag.compute_layout();
    
    println!("LayoutIR Nodes:");
    for node in ir.nodes() {
        println!("  id={} label={:?} x={} y={} width={} center_x={} level={}",
            node.id, node.label, node.x, node.y, node.width, node.center_x, node.level);
    }
    
    println!("\nLayoutIR Edges:");
    for edge in ir.edges() {
        println!("  {} -> {} : from=({},{}) to=({},{}) path={:?}",
            edge.from_id, edge.to_id, 
            edge.from_x, edge.from_y,
            edge.to_x, edge.to_y,
            edge.path);
    }
    
    println!("\n=== Diagonal Skip Test (different X coords) ===\n");
    
    // Skip edge where source and target have different X
    // A at one side, E at the other, with skip edge
    let dag3 = DAG::from_edges(
        &[
            (1, "A"),     // Level 0 left
            (2, "B"),     // Level 0 right
            (3, "C"),     // Level 1 (child of A)
            (4, "D"),     // Level 2 (child of C)
            (5, "E"),     // Level 3 (child of B via skip, child of D)
        ],
        &[
            (1, 3),       // A -> C
            (3, 4),       // C -> D  
            (4, 5),       // D -> E
            (2, 5),       // B -> E (skip edge: level 0 -> level 3)
        ],
    );
    
    println!("ASCII Render:");
    println!("{}", dag3.render());
    println!();
    
    let ir3 = dag3.compute_layout();
    
    println!("LayoutIR Nodes:");
    for node in ir3.nodes() {
        println!("  id={} label={:?} x={} y={} width={} level={}",
            node.id, node.label, node.x, node.y, node.width, node.level);
    }
    
    println!("\nLayoutIR Edges:");
    for edge in ir3.edges() {
        println!("  {} -> {} : from=({},{}) to=({},{}) path={:?}",
            edge.from_id, edge.to_id, 
            edge.from_x, edge.from_y,
            edge.to_x, edge.to_y,
            edge.path);
    }
}
