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

use ascii_dag::render::colors::Palette;

include!("shared/hero_graph.rs");

fn main() {
    let g = hero_graph();

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
