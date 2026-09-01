//! Dummy nodes — the routing waypoints skip-level edges travel
//! through, and the two switches that expose them:
//!
//! - `LayoutConfig.include_dummy_nodes` (layout side): emit each
//!   waypoint into the IR as a real node with `kind == Dummy` and an
//!   `edge_index` back-link to its owning edge — for introspection,
//!   hit-testing, and JSON export. Zero cost when off (default).
//! - `RenderOptions.show_dummy_nodes` (render side): draw `◍` (ASCII:
//!   `o`) at every waypoint the IR carries.
//!
//! Run:
//!   cargo run --example dummy_nodes
//!   cargo run --example dummy_nodes -- --ascii
//!   cargo run --example dummy_nodes --features arena -- --csr

use ascii_dag::ir::NodeKind;
use ascii_dag::{AUTO, Graph, LayoutConfig, RenderOptions};

#[path = "support/csr.rs"]
mod csr;

fn graph() -> Graph<'static> {
    let mut g = Graph::new();
    let a = g.add_node(AUTO, "Fetch");
    let b = g.add_node(AUTO, "Parse");
    let c = g.add_node(AUTO, "Check");
    let d = g.add_node(AUTO, "Emit");
    g.add_edge(a, b, None);
    g.add_edge(b, c, None);
    g.add_edge(c, d, None);
    // Skip-level edges: these route through dummy waypoints on every
    // level they pass.
    g.add_edge(a, d, Some("fast path"));
    g.add_edge(b, d, None);
    g
}

fn main() {
    let ascii = std::env::args().any(|a| a == "--ascii");
    let mut options = if ascii {
        RenderOptions::ascii()
    } else {
        RenderOptions::plain()
    };

    let mut config = LayoutConfig::standard();
    config.include_dummy_nodes = true;

    println!("1) Default: waypoints exist but stay invisible\n");
    let plain = if csr::requested() {
        csr::render(&graph(), &options)
    } else {
        graph().compute_layout().render_string(&options)
    };
    println!("{plain}");

    println!("2) show_dummy_nodes: every waypoint marked\n");
    options.plan.show_dummy_nodes = true;
    let marked = if csr::requested() {
        csr::render_with(&graph(), &options, &config)
    } else {
        graph()
            .compute_layout_with_config(&config)
            .render_string(&options)
    };
    println!("{marked}");

    // Introspection: with include_dummy_nodes the IR carries the
    // waypoints as real nodes (synthetic ids, excluded from
    // node_by_id), each pointing back at its owning edge.
    println!("3) The IR side (heap pipeline):\n");
    let ir = graph().compute_layout_with_config(&config);
    for n in ir.nodes() {
        if matches!(n.kind, NodeKind::Dummy) {
            println!(
                "   dummy at ({}, {}) level {} — routes edge #{}",
                n.x,
                n.y,
                n.level,
                n.edge_index.unwrap_or(usize::MAX)
            );
        }
    }
}
