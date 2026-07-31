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
//! - ASCII charset (pure-ASCII glyph projection of the same canvas)
//!
//! Run:
//!   cargo run --example hero              # plain
//!   cargo run --example hero -- --color   # ANSI colors + legend
//!   cargo run --example hero -- --bt      # BottomUp (flags combine)
//!   cargo run --example hero -- --ascii   # ASCII glyphs (flags combine)
//!
//! The unflagged and --color-only paths render through the legacy
//! scanline renderers (they are the golden-snapshot authority until
//! RW8); --bt and --ascii render through the new engine — the only
//! paint path with direction and charset support.

use ascii_dag::render::colors::Palette;
use ascii_dag::render::engine::{Charset, RenderOptions};
use ascii_dag::Direction;

include!("shared/hero_graph.rs");

#[path = "support/csr.rs"]
mod csr;

fn main() {
    let mut g = hero_graph();

    // ── Render ───────────────────────────────────────────────────
    let args: Vec<String> = std::env::args().collect();
    let use_color = args.iter().any(|a| a == "--color" || a == "-c");
    let bottom_up = args.iter().any(|a| a == "--bt");
    let ascii = args.iter().any(|a| a == "--ascii" || a == "-a");

    if bottom_up {
        g.set_direction(Direction::BottomUp);
    }

    if csr::requested() {
        // Same graph, arena pipeline — byte-identical output.
        let mut opts = if use_color {
            RenderOptions::colored(Palette::Ansi)
        } else {
            RenderOptions::plain()
        };
        if ascii {
            opts.charset = Charset::Ascii;
        }
        println!("{}", csr::render(&g, &opts));
        return;
    }

    let ir = g.compute_layout();

    let output = if bottom_up || ascii {
        // Engine path: the only renderer with direction/charset support.
        let mut opts = if use_color {
            RenderOptions::colored(Palette::Ansi)
        } else {
            RenderOptions::plain()
        };
        if ascii {
            opts.charset = Charset::Ascii;
        }
        ir.render_string(&opts)
    } else if use_color {
        ir.render_string(&RenderOptions::colored(Palette::Ansi))
    } else {
        ir.render_string(&RenderOptions::plain())
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
