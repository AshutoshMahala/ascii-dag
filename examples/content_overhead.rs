//! NC-N1 measurement: what node content costs, per kind.
//!
//! Builds a 10k-node chain three times — all-simple, all-boxed,
//! all-custom (painter + 8-byte payload) — and reports build time,
//! heap layout+render time, and the arena-size estimates (the public
//! memory contract embedded users provision against).
//!
//! Run:
//!   cargo run --release --example content_overhead --features arena

use ascii_dag::render::engine::{NodePaintCtx, NodeRegion};
use ascii_dag::{BoxedNode, CustomNode, Graph, RenderOptions};
use std::time::Instant;

const N: usize = 10_000;

fn tick(region: &mut NodeRegion<'_, '_>, ctx: NodePaintCtx<'_>) {
    region.write_str(1, 0, ctx.label);
    region.write_str(1, 1, ctx.payload);
}

fn build(kind: &str) -> Graph<'static> {
    let mut g = Graph::new();
    for i in 0..N {
        match kind {
            "simple" => {
                g.add_node(i, "node");
            }
            "boxed" => {
                g.add_node(i, BoxedNode("node"));
            }
            _ => {
                g.add_node(
                    i,
                    CustomNode {
                        label: "node",
                        width: 8,
                        height: 3,
                        painter: Some(tick),
                        payload: "12345678",
                    },
                );
            }
        }
        if i > 0 {
            g.add_edge(i - 1, i, None);
        }
    }
    g
}

fn main() {
    println!("{N} nodes per graph; per-node deltas vs all-simple\n");
    println!(
        "{:<8} {:>10} {:>12} {:>14} {:>14}",
        "kind", "build", "layout+render", "csr estimate", "layout estimate"
    );
    let mut baseline: Option<(usize, usize)> = None;
    for kind in ["simple", "boxed", "custom"] {
        let t = Instant::now();
        let g = build(kind);
        let build_t = t.elapsed();

        let t = Instant::now();
        let out = g.compute_layout().render_string(&RenderOptions::plain());
        let render_t = t.elapsed();
        assert!(!out.is_empty());

        let csr = g.estimate_csr_arena_size();
        let layout = g.estimate_layout_arena_size();
        println!(
            "{:<8} {:>10.1?} {:>12.1?} {:>11} B {:>12} B",
            kind, build_t, render_t, csr, layout
        );
        match baseline {
            None => baseline = Some((csr, layout)),
            Some((c0, l0)) => println!(
                "{:<8} {:>10} {:>12} {:>+10.1} B/node {:>+9.1} B/node",
                "",
                "",
                "",
                (csr as f64 - c0 as f64) / N as f64,
                (layout as f64 - l0 as f64) / N as f64
            ),
        }
    }
}
