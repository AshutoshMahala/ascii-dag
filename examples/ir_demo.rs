//! Example demonstrating the Layout IR (Intermediate Representation)
//!
//! The IR decouples layout computation from rendering, enabling:
//! - Multiple output formats (ASCII, ANSI colors, SVG, HTML)
//! - Interactive features (mouse hit-testing, node selection)
//! - Layout inspection and debugging

use ascii_dag::{DAG, EdgePath};

fn main() {
    println!("=== Layout IR Demo ===\n");

    // Create a sample DAG
    let dag = DAG::from_edges(
        &[
            (1, "Root"),
            (2, "ChildA"),
            (3, "ChildB"),
            (4, "GrandchildA"),
            (5, "GrandchildB"),
            (6, "Convergence"),
        ],
        &[
            (1, 2),
            (1, 3),
            (2, 4),
            (3, 5),
            (4, 6),
            (5, 6),
            (1, 6), // Skip-level edge
        ],
    );

    // Compute the layout IR
    let ir = dag.compute_layout();

    // === Basic Info ===
    println!("Layout Dimensions:");
    println!("  Width:  {} chars", ir.width());
    println!("  Height: {} lines", ir.height());
    println!("  Levels: {}", ir.level_count());
    println!();

    // === Node Positions ===
    println!("Node Positions:");
    println!("  {:15} {:>5} {:>5} {:>7} {:>7}", "Label", "X", "Y", "Width", "Center");
    println!("  {}", "-".repeat(45));
    for node in ir.nodes() {
        println!(
            "  {:15} {:>5} {:>5} {:>7} {:>7}",
            node.label, node.x, node.y, node.width, node.center_x
        );
    }
    println!();

    // === Edge Routing ===
    println!("Edge Routing:");
    for edge in ir.edges() {
        let from_node = ir.node_by_id(edge.from_id).map(|n| n.label).unwrap_or("?");
        let to_node = ir.node_by_id(edge.to_id).map(|n| n.label).unwrap_or("?");
        
        let path_desc = match &edge.path {
            EdgePath::Direct => "Direct".to_string(),
            EdgePath::Corner { horizontal_y } => format!("Corner @ y={}", horizontal_y),
            EdgePath::SideChannel { channel_x, start_y, end_y } => {
                format!("SideChannel x={} y={}..{}", channel_x, start_y, end_y)
            }
            EdgePath::MultiSegment { waypoints } => {
                format!("MultiSegment ({} waypoints)", waypoints.len())
            }
        };
        
        println!(
            "  {} -> {}: ({},{}) -> ({},{}) [{}]",
            from_node, to_node,
            edge.from_x, edge.from_y,
            edge.to_x, edge.to_y,
            path_desc
        );
    }
    println!();

    // === Nodes by Level ===
    println!("Nodes by Level:");
    for level in 0..ir.level_count() {
        let nodes: Vec<_> = ir.nodes_at_level(level).map(|n| n.label).collect();
        println!("  Level {}: {:?}", level, nodes);
    }
    println!();

    // === Hit Testing Demo ===
    println!("Hit Testing Demo (for mouse interaction):");
    // Simulate clicking at various coordinates
    let test_coords = [(0, 0), (5, 0), (10, 3), (100, 100)];
    for (x, y) in test_coords {
        let hit = ir.node_at(x, y);
        match hit {
            Some(node) => println!("  Click at ({}, {}): HIT '{}' (id={})", x, y, node.label, node.id),
            None => println!("  Click at ({}, {}): miss", x, y),
        }
    }
    println!();

    // === Edge Coloring Demo ===
    println!("Edge Color Indices (for colored rendering):");
    for edge in ir.edges() {
        let from_node = ir.node_by_id(edge.from_id).map(|n| n.label).unwrap_or("?");
        let to_node = ir.node_by_id(edge.to_id).map(|n| n.label).unwrap_or("?");
        let color_idx = ir.edge_color_index(edge);
        println!("  {} -> {}: color index {}", from_node, to_node, color_idx);
    }
    println!();

    // === Standard Render for Comparison ===
    println!("Standard ASCII Render:");
    println!("{}", dag.render());
}
