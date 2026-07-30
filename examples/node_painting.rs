//! Node painters: nodes declare an AREA, layout routes around it, and a
//! painter fills it at render time.
//!
//! The classic `[label]` look has no special privilege — it is simply
//! the default painter. `Boxed` draws a border box across the full
//! declared `width × height`; `Custom` hands you a clipped, node-local
//! region to draw anything (writes outside the area are silently
//! dropped, so painters can't corrupt the diagram).
//!
//! Run:
//!   cargo run --example node_painting
//!   cargo run --example node_painting -- --ascii

use ascii_dag::render::engine::{NodePaint, NodePaintCtx, NodeRegion, NodeStyle, NodeStyleCtx};
use ascii_dag::{Graph, RenderOptions};

/// A custom "card" painter: header row, separator, body — keyed off
/// the paint context (`node_id`/`label`). Painters are plain `fn`s:
/// they cannot capture environment, though they *can* read globals or
/// statics — drawing the same content on every call is the caller's
/// contract (bands replay painters), and this painter keeps it by
/// deriving everything from `ctx`.
fn card(region: &mut NodeRegion<'_, '_>, ctx: NodePaintCtx<'_>) {
    region.write_str(1, 0, ctx.label);
    // Painter text passes through untranslated — pick glyphs per the
    // active charset to stay ASCII-clean under `--ascii`.
    let rule = match ctx.charset {
        ascii_dag::Charset::Ascii => '-',
        _ => '─',
    };
    for x in 0..region.width() {
        region.set(x, 1, rule);
    }
    let body: &[&str] = match ctx.node_id {
        10 => &["cpu: 4", "ram: 16G"],
        _ => &["…"],
    };
    for (i, line) in body.iter().enumerate() {
        region.write_str(1, 2 + i, line);
    }
}

fn styles(ctx: NodeStyleCtx<'_>) -> NodeStyle {
    NodeStyle {
        paint: match ctx.node_id {
            10 => NodePaint::Custom(card),  // the card above
            20 => NodePaint::Boxed,         // built-in box painter
            _ => NodePaint::Simple,         // classic [label]
        },
        ..NodeStyle::default()
    }
}

fn main() {
    let mut g = Graph::new();
    g.add_node(1, "Client");
    // Declared area: 12 wide × 5 tall. Layout reserves it; edges route
    // around it; the painter fills it.
    g.add_node_with_size(10, "Server", 12, 5);
    g.add_node_with_size(20, "Database", 12, 3);
    g.add_edge(1, 10, None);
    g.add_edge(10, 20, None);

    let ir = g.compute_layout();
    let ascii = std::env::args().any(|a| a == "--ascii");
    let options = if ascii {
        let mut o = RenderOptions::ascii();
        o.node_style_fn = styles;
        o
    } else {
        let mut o = RenderOptions::plain();
        o.node_style_fn = styles;
        o
    };
    println!("{}", ir.render_string(&options));
}
