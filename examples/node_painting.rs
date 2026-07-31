//! Nodes as objects: the node's declaration is the only source of
//! what it is — a simple `[label]`, a boxed label, or custom content
//! with its own size, painter, and payload.
//!
//! Layout reserves each node's declared area and routes edges around
//! it; at render time the declared painter fills it through a clipped
//! region (writes outside the area are silently dropped, so painters
//! can't corrupt the diagram).
//!
//! Run:
//!   cargo run --example node_painting
//!   cargo run --example node_painting -- --ascii
//!   cargo run --example node_painting --features arena -- --csr

use ascii_dag::render::engine::{NodePaintCtx, NodeRegion};
use ascii_dag::{BoxedNode, CustomNode, Graph, RenderOptions};

/// The card template: header row, separator rule, then the payload —
/// one line per row. Shared by every card node (a plain `fn`); the
/// per-node **data** arrives via `ctx.payload`, declared at
/// `add_node`. Painters must draw the same content on every call
/// (bands replay them); deriving everything from `ctx` keeps that
/// contract.
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
    for (i, line) in ctx.payload.lines().enumerate() {
        region.write_str(1, 2 + i, line);
    }
}

fn main() {
    let mut g = Graph::new();
    // A simple node: the classic [label].
    g.add_node(1, "Client");
    // A card: declared size, template fn, and payload — all on the
    // node itself.
    g.add_node(
        10,
        CustomNode {
            label: "Server",
            width: 12,
            height: 5,
            painter: Some(card),
            payload: "cpu: 4\nram: 16G",
        },
    );
    // A boxed node: a light-stroke box around the label.
    g.add_node(20, BoxedNode("Database"));
    g.add_edge(1, 10, None);
    g.add_edge(10, 20, None);

    let ascii = std::env::args().any(|a| a == "--ascii");
    let options = if ascii {
        RenderOptions::ascii()
    } else {
        RenderOptions::plain()
    };

    // --csr renders the same declarations through the arena/no-alloc
    // pipeline (Graph → CSR → arena IR → engine) — byte-identical to
    // the heap path. Declared painters and payloads travel with it.
    #[cfg(feature = "arena")]
    if std::env::args().any(|a| a == "--csr") {
        use ascii_dag::LayoutConfig;
        use ascii_dag::graph::arena::Arena;
        let mut csr_buf = vec![0u8; g.estimate_csr_arena_size() * 2];
        let mut csr_arena = Arena::new(&mut csr_buf);
        let csr = g.to_csr(&mut csr_arena).expect("CSR conversion");
        let size = (g.estimate_layout_arena_size() * 2).max(256 * 1024);
        let mut temp_buf = vec![0u8; size];
        let mut out_buf = vec![0u8; size];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_arena = Arena::new(&mut out_buf);
        let ir = csr
            .compute_layout_arena(&LayoutConfig::standard(), &mut temp_arena, &mut out_arena)
            .expect("CSR layout");
        println!("{}", ir.render_string(&options));
        return;
    }
    #[cfg(not(feature = "arena"))]
    if std::env::args().any(|a| a == "--csr") {
        eprintln!("--csr needs the arena feature: --features arena");
        std::process::exit(2);
    }

    let ir = g.compute_layout();
    println!("{}", ir.render_string(&options));
}
