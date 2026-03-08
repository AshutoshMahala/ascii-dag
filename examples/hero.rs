//! Hero graph for README — showcases all major features of ascii-dag.
//!
//! Features demonstrated:
//! - Hierarchical (nested) subgraphs with double-line borders
//! - Cross-cluster edges
//! - Skip-level (cross-level) edges
//! - Edge labels (inline + legend fallback)
//! - Colored edges (greedy graph coloring)
//! - Junction characters where edges cross subgraph borders
//! - Self-cycle (node loops back to itself)
//! - Reversed edge (back-edge, renders with dashed lines)
//!
//! Run:
//!   cargo run --example hero           # plain
//!   cargo run --example hero -- --color  # ANSI colors + legend

use ascii_dag::Graph;
use ascii_dag::render::colors::Palette;

fn main() {
    let mut g = Graph::new();

    // ── Nodes ────────────────────────────────────────────────────
    g.add_node(1, "Client");
    g.add_node(2, "Gateway");
    g.add_node(3, "Users");
    g.add_node(4, "Orders");
    g.add_node(5, "DB");
    g.add_node(6, "Queue");
    g.add_node(7, "Mailer");
    g.add_node(8, "Dash");

    // ── Edges (with labels) ──────────────────────────────────────
    g.add_edge(1, 2, Some("http"));         // Client → Gateway
    g.add_edge(2, 3, None);                 // Gateway → Users
    g.add_edge(2, 4, None);                 // Gateway → Orders
    g.add_edge(3, 5, Some("read"));         // Users → DB
    g.add_edge(4, 5, Some("write"));        // Orders → DB
    g.add_edge(4, 6, Some("emit"));         // Orders → Queue
    g.add_edge(6, 7, Some("notify"));       // Queue → Mailer
    g.add_edge(5, 8, Some("sync"));         // DB → Dash
    g.add_edge(7, 8, None);                 // Mailer → Dash
    g.add_edge(1, 8, Some("trace"));        // Client → Dash (deep skip-level!)

    // Self-cycle: Gateway retries on failure
    g.add_edge(2, 2, Some("retry"));

    // Reversed edge: Dash feeds back to Gateway (back-edge / cycle)
    g.add_edge(8, 2, Some("feedback"));

    // ── Subgraphs ────────────────────────────────────────────────
    // Services cluster
    let svc = g.add_subgraph("Services");
    g.put_nodes(&[3, 4]).inside(svc).expect("place nodes in Services");

    // Data cluster
    let data = g.add_subgraph("Data");
    g.put_nodes(&[5, 6]).inside(data).expect("place nodes in Data");

    // Nested: Async inside Data
    let async_sg = g.add_subgraph("Async");
    g.put_nodes(&[6]).inside(async_sg).expect("place Queue in Async");
    g.put_subgraphs(&[async_sg]).inside(data).expect("nest Async in Data");

    // ── Render ───────────────────────────────────────────────────
    let args: Vec<String> = std::env::args().collect();
    let use_color = args.iter().any(|a| a == "--color" || a == "-c");

    let ir = g.compute_layout();

    if use_color {
        let output = ir.render_scanline_colored_with_legend(Palette::Ansi);
        println!("{}", output);
    } else {
        println!("{}", ir.render_scanline());
    }

    // Print stats
    eprintln!(
        "--- {} nodes, {} edges, {} subgraphs, canvas {}×{} ---",
        ir.nodes().len(),
        ir.edges().len(),
        ir.subgraphs().len(),
        ir.width(),
        ir.height()
    );
}
