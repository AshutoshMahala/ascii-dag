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
//! - BottomUp direction (graph grows upward, arrows point up)
//!
//! Run:
//!   cargo run --example hero              # plain
//!   cargo run --example hero -- --color   # ANSI colors + legend
//!   cargo run --example hero -- --bt      # BottomUp (flags combine)
//!
//! TopDown renders through the legacy scanline renderers (they are the
//! golden-snapshot authority until RW8); --bt renders through the new
//! engine, the only direction-aware paint path.

use ascii_dag::render::colors::Palette;
use ascii_dag::render::engine;
use ascii_dag::Direction;

include!("shared/hero_graph.rs");

fn main() {
    let mut g = hero_graph();

    // ── Render ───────────────────────────────────────────────────
    let args: Vec<String> = std::env::args().collect();
    let use_color = args.iter().any(|a| a == "--color" || a == "-c");
    let bottom_up = args.iter().any(|a| a == "--bt");

    if bottom_up {
        g.set_direction(Direction::BottomUp);
    }

    let ir = g.compute_layout();

    let output = match (bottom_up, use_color) {
        (true, true) => {
            engine::preview_render_colored(&ir, &engine::RenderOptions::colored(Palette::Ansi))
        }
        (true, false) => engine::preview_render_plain(&ir, &engine::RenderOptions::plain()),
        (false, true) => ir.render_scanline_colored_with_legend(Palette::Ansi),
        (false, false) => ir.render_scanline(),
    };
    println!("{}", output);

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
