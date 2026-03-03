//! Demonstrates the ascii-dag architecture: Build → Compute IR → Process → Render
//!
//! This example shows that ascii-dag is primarily a LAYOUT ENGINE.
//! The terminal ASCII output is just ONE possible renderer on top of the IR.
//!
//! Architecture:
//!   1. Build: Construct the DAG structure
//!   2. Compute: Generate Layout IR (positions, routing)
//!   3. Process: Analyze, transform, or export the IR
//!   4. Render: Output to terminal, SVG, Canvas, or anything else

use ascii_dag::Graph;

fn main() {
    println!("=== ascii-dag Architecture Demo ===\n");

    // ═══════════════════════════════════════════════════════════════════════
    // STEP 1: BUILD - Construct the graph
    // ═══════════════════════════════════════════════════════════════════════
    println!("┌─────────────────────────────────────────┐");
    println!("│ STEP 1: BUILD                           │");
    println!("└─────────────────────────────────────────┘");

    let dag = Graph::from_edges(
        &[
            (1, "Parse"),
            (2, "Analyze"),
            (3, "Optimize"),
            (4, "Codegen"),
            (5, "Link"),
        ],
        &[
            (1, 2), // Parse → Analyze
            (1, 3), // Parse → Optimize (skip-level)
            (2, 4), // Analyze → Codegen
            (3, 4), // Optimize → Codegen (diamond)
            (4, 5), // Codegen → Link
        ],
    );

    println!("Built DAG with {} nodes and {} edges\n", 5, 5);

    // ═══════════════════════════════════════════════════════════════════════
    // STEP 2: COMPUTE - Generate Layout IR
    // ═══════════════════════════════════════════════════════════════════════
    println!("┌─────────────────────────────────────────┐");
    println!("│ STEP 2: COMPUTE (Layout Engine)         │");
    println!("└─────────────────────────────────────────┘");

    let ir = dag.compute_layout();

    println!("Layout IR generated:");
    println!(
        "  • Canvas size: {} × {} characters",
        ir.width(),
        ir.height()
    );
    println!("  • Levels: {}", ir.level_count());
    println!("  • Nodes: {}", ir.nodes().len());
    println!("  • Edges: {}", ir.edges().len());
    println!();

    // ═══════════════════════════════════════════════════════════════════════
    // STEP 3: PROCESS - Analyze or transform the IR
    // ═══════════════════════════════════════════════════════════════════════
    println!("┌─────────────────────────────────────────┐");
    println!("│ STEP 3: PROCESS (Your Custom Logic)     │");
    println!("└─────────────────────────────────────────┘");

    // Example: Extract node positions for hit-testing or custom rendering
    println!("Node positions (for hit-testing, Canvas, SVG, etc.):");
    for node in ir.nodes() {
        println!(
            "  {} (id={}): x={}, y={}, width={}, center_x={}",
            node.label, node.id, node.x, node.y, node.width, node.center_x
        );
    }
    println!();

    // Example: Analyze edge routing
    println!("Edge routing (for custom path drawing):");
    for edge in ir.edges() {
        let route_type = match &edge.path {
            ascii_dag::ir::EdgePath::Direct => "Direct (vertical line)".to_string(),
            ascii_dag::ir::EdgePath::Corner { horizontal_y } => {
                format!("Corner (L-shape at y={})", horizontal_y)
            }
            ascii_dag::ir::EdgePath::SideChannel { channel_x, .. } => {
                format!("SideChannel (routed via x={})", channel_x)
            }
            ascii_dag::ir::EdgePath::MultiSegment { waypoints, .. } => {
                format!("MultiSegment ({} waypoints)", waypoints.len())
            }
        };
        println!(
            "  {} → {}: ({},{}) → ({},{}) [{}]",
            edge.from_id, edge.to_id, edge.from_x, edge.from_y, edge.to_x, edge.to_y, route_type
        );
    }
    println!();

    // Example: Generate custom output (pseudo-SVG)
    println!("Example: Generate pseudo-SVG from IR:");
    println!(
        "  <svg width=\"{}\" height=\"{}\">",
        ir.width() * 10,
        ir.height() * 20
    );
    for node in ir.nodes() {
        println!(
            "    <rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"20\" label=\"{}\"/>",
            node.x * 10,
            node.y * 20,
            node.width * 10,
            node.label
        );
    }
    println!("  </svg>");
    println!();

    // ═══════════════════════════════════════════════════════════════════════
    // STEP 4: RENDER - Output to terminal (built-in scanline renderer)
    // ═══════════════════════════════════════════════════════════════════════
    println!("┌─────────────────────────────────────────┐");
    println!("│ STEP 4: RENDER (Terminal ASCII)         │");
    println!("└─────────────────────────────────────────┘");

    // The IR has a built-in ASCII renderer, but you could write your own!
    let mut output = String::new();
    ir.render_scanline_to(&mut output);
    println!("{}", output);

    // ═══════════════════════════════════════════════════════════════════════
    // SUMMARY
    // ═══════════════════════════════════════════════════════════════════════
    println!("┌─────────────────────────────────────────┐");
    println!("│ ARCHITECTURE SUMMARY                    │");
    println!("└─────────────────────────────────────────┘");
    println!();
    println!("  ┌─────────┐    ┌─────────────┐    ┌─────────────┐    ┌─────────────┐");
    println!("  │  BUILD  │ →  │   COMPUTE   │ →  │   PROCESS   │ →  │   RENDER    │");
    println!("  │  (DAG)  │    │ (Layout IR) │    │ (Your Code) │    │ (Terminal)  │");
    println!("  └─────────┘    └─────────────┘    └─────────────┘    └─────────────┘");
    println!();
    println!("  The Layout IR is the REAL product. Terminal ASCII is just one renderer.");
    println!("  You can render to: Canvas, SVG, PDF, TUI frameworks, etc.");
}
