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
//! - All four rank directions (levels as rows, or as columns)
//! - Routing waypoints made visible (`--dummy`)
//! - ASCII charset (pure-ASCII glyph projection of the same canvas)
//!
//! Run:
//!   cargo run --example hero              # plain (TopDown)
//!   cargo run --example hero -- --color   # ANSI colors + legend
//!   cargo run --example hero -- --bt      # BottomUp  (flags combine)
//!   cargo run --example hero -- --lr      # LeftRight (flags combine)
//!   cargo run --example hero -- --rl      # RightLeft (flags combine)
//!   cargo run --example hero -- --ascii   # ASCII glyphs (flags combine)
//!   cargo run --example hero -- --dummy   # mark routing waypoints
//!   cargo run --example hero -- --csr     # arena pipeline (byte-identical)
//!
//! `--lr`/`--rl` lay the same graph out sideways: levels become
//! columns and edges run in horizontal trunks, which suits wide,
//! shallow graphs. Every flag combines with every other.

use ascii_dag::render::colors::Palette;
use ascii_dag::render::engine::{Charset, RenderOptions};
use ascii_dag::{Direction, LayoutConfig};

include!("shared/hero_graph.rs");

#[path = "support/csr.rs"]
mod csr;

fn main() {
    let mut g = hero_graph();

    // ── Render ───────────────────────────────────────────────────
    let args: Vec<String> = std::env::args().collect();
    let use_color = args.iter().any(|a| a == "--color" || a == "-c");
    let ascii = args.iter().any(|a| a == "--ascii" || a == "-a");
    // Routing waypoints need BOTH switches: the layout must emit them
    // into the IR, and the renderer must draw their marker.
    let dummy = args.iter().any(|a| a == "--dummy" || a == "-d");
    // Last direction flag wins (scanning from the end finds it
    // first), so `--lr --rl` is RightLeft. Unflagged stays TopDown.
    if let Some(dir) = args.iter().rev().find_map(|a| match a.as_str() {
        "--bt" => Some(Direction::BottomUp),
        "--lr" => Some(Direction::LeftRight),
        "--rl" => Some(Direction::RightLeft),
        _ => None,
    }) {
        g.set_direction(dir);
    }

    let mut config = LayoutConfig::standard();
    config.direction = g.direction();
    config.include_dummy_nodes = dummy;

    let mut opts = if use_color {
        RenderOptions::colored(Palette::Ansi)
    } else {
        RenderOptions::plain()
    };
    if ascii {
        opts.emit.charset = Charset::Ascii;
    }
    opts.plan.show_dummy_nodes = dummy;

    if csr::requested() {
        // Same graph, arena pipeline — byte-identical output.
        println!("{}", csr::render_with(&g, &opts, &config));
        return;
    }

    let ir = g.compute_layout_with_config(&config);
    println!("{}", ir.render_string(&opts));

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
