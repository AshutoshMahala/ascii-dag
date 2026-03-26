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
    let args: Vec<String> = std::env::args().collect();
    let use_csr = args.iter().any(|a| a == "--csr");

    if use_csr {
        println!("=== ascii-dag Architecture Demo (CSR Mode) ===\n");
        run_csr();
    } else {
        println!("=== ascii-dag Architecture Demo (Heap Mode) ===\n");
        run_heap();
    }
}

fn build_demo_dag() -> Graph<'static> {
    Graph::from_edges(
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
    )
}

fn run_heap() {
    // ═══════════════════════════════════════════════════════════════════════
    // STEP 1: BUILD - Construct the graph
    // ═══════════════════════════════════════════════════════════════════════
    println!("┌─────────────────────────────────────────┐");
    println!("│ STEP 1: BUILD                           │");
    println!("└─────────────────────────────────────────┘");

    let dag = build_demo_dag();
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
            ascii_dag::ir::EdgePath::Spline { .. } => "Spline (Bézier curve)".to_string(),
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

#[cfg(feature = "arena")]
fn run_csr() {
    use ascii_dag::LayoutConfig;
    use ascii_dag::graph::arena::Arena;

    let dag = build_demo_dag();

    // STEP 1: BUILD — convert to CsrGraph
    println!("┌─────────────────────────────────────────┐");
    println!("│ STEP 1: BUILD (Graph → CsrGraph)        │");
    println!("└─────────────────────────────────────────┘");

    let csr_size = dag.estimate_csr_arena_size() * 2;
    let mut csr_buf = vec![0u8; csr_size];
    let mut csr_arena = Arena::new(&mut csr_buf);
    let csr = dag.to_csr(&mut csr_arena).expect("CSR conversion failed");
    println!(
        "CsrGraph: {} nodes, {} edges\n",
        csr.node_count(),
        csr.edge_count()
    );

    // STEP 2: COMPUTE — arena-based layout
    println!("┌─────────────────────────────────────────┐");
    println!("│ STEP 2: COMPUTE (Arena Layout Engine)   │");
    println!("└─────────────────────────────────────────┘");

    let layout_size = dag.estimate_layout_arena_size();
    let size = ((layout_size * 6) / 5).max(128 * 1024);
    let mut temp_buf = vec![0u8; size];
    let mut out_buf = vec![0u8; size];
    let mut temp_arena = Arena::new(&mut temp_buf);
    let mut out_arena = Arena::new(&mut out_buf);

    let ir = csr
        .compute_layout_arena(&LayoutConfig::standard(), &mut temp_arena, &mut out_arena)
        .expect("Layout failed");

    println!("LayoutIRArena generated:");
    println!(
        "  • Canvas size: {} × {} characters",
        ir.width(),
        ir.height()
    );
    println!("  • Levels: {}", ir.level_count());
    println!("  • Nodes: {}", ir.node_count());
    println!("  • Edges: {}", ir.edge_count());
    println!();

    // STEP 3: PROCESS — inspect node positions
    println!("┌─────────────────────────────────────────┐");
    println!("│ STEP 3: PROCESS (IR Inspection)         │");
    println!("└─────────────────────────────────────────┘");

    println!("Node positions:");
    for (i, node) in ir.nodes().iter().enumerate() {
        println!(
            "  {} (id={}): x={}, y={}, width={}",
            ir.node_label(i),
            node.id,
            node.x,
            node.y,
            node.width
        );
    }
    println!();

    println!("Edges:");
    for (i, edge) in ir.edges().iter().enumerate() {
        let waypoints = ir.edge_waypoints(edge);
        println!(
            "  edge {}: from_id={}, to_id={}, waypoints={}",
            i,
            edge.from_id,
            edge.to_id,
            waypoints.len()
        );
    }
    println!();

    // STEP 4: RENDER — buffer-based rendering
    println!("┌─────────────────────────────────────────┐");
    println!("│ STEP 4: RENDER (Buffer-based ASCII)     │");
    println!("└─────────────────────────────────────────┘");

    let (render_bytes, scratch_len) = ir.estimate_render_size();
    let mut render_buffer = vec![0u8; render_bytes + 4096];
    let mut line_buffer = vec![' '; ir.width().max(1) + 16];
    let mut scratch_buffer = vec![0usize; scratch_len + 256];

    if let Some(len) =
        ir.render_to_buffer(&mut render_buffer, &mut line_buffer, &mut scratch_buffer)
    {
        if let Ok(s) = std::str::from_utf8(&render_buffer[..len]) {
            println!("{}", s);
        }
    }

    // SUMMARY
    println!("┌─────────────────────────────────────────┐");
    println!("│ ARCHITECTURE SUMMARY                    │");
    println!("└─────────────────────────────────────────┘");
    println!();
    println!("  ┌─────────┐    ┌───────────┐    ┌─────────────┐    ┌──────────────┐");
    println!("  │  BUILD  │ →  │ CSR+Arena │ →  │   PROCESS   │ →  │ RENDER (buf) │");
    println!("  │  (DAG)  │    │ (Layout)  │    │ (Your Code) │    │ (no-alloc)   │");
    println!("  └─────────┘    └───────────┘    └─────────────┘    └──────────────┘");
    println!();
    println!("  Same IR, different allocation strategy. Zero heap allocations in the pipeline.");
}

#[cfg(not(feature = "arena"))]
fn run_csr() {
    println!("(arena feature not enabled — run with --features arena)");
}
