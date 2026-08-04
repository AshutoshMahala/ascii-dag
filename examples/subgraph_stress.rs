//! Subgraph stress test — progressive levels to find terminal rendering limits.
//!
//! Each test adds more nodes, deeper nesting, or wider subgraphs to see
//! how much a terminal can comfortably display.
//!
//! Usage:
//!   cargo run --example subgraph_stress --release               # Heap mode
//!   cargo run --example subgraph_stress --release --features arena -- --csr  # CSR mode

use ascii_dag::graph::Graph;
use std::time::Instant;

#[cfg(feature = "arena")]
use ascii_dag::LayoutConfig;
#[cfg(feature = "arena")]
use ascii_dag::graph::arena::Arena;
use ascii_dag::render::engine::RenderOptions;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let use_csr = args.iter().any(|a| a == "--csr");

    let mode = if use_csr { "CSR" } else { "Heap" };
    println!("╔══════════════════════════════════════════════════╗");
    println!("║     Subgraph Stress Test  ({:>4} mode)           ║", mode);
    println!("╚══════════════════════════════════════════════════╝\n");

    #[allow(clippy::type_complexity)]
    let tests: Vec<(&str, fn() -> Graph<'static>)> = vec![
        (
            "Tier 1 · Microservices (12 nodes, 4 subgraphs, depth 1)",
            tier1_microservices,
        ),
        (
            "Tier 2 · Platform (20 nodes, 8 subgraphs, depth 2)",
            tier2_platform,
        ),
        (
            "Tier 3 · Cloud Infra (30 nodes, 12 subgraphs, depth 3)",
            tier3_cloud,
        ),
        (
            "Tier 4 · Enterprise (50 nodes, 16 subgraphs, depth 3)",
            tier4_enterprise,
        ),
        (
            "Tier 5 · Megacorp (80 nodes, 24 subgraphs, depth 4)",
            tier5_megacorp,
        ),
    ];

    for (name, build_fn) in &tests {
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!("  {}", name);
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

        let dag = build_fn();
        let sg_count = dag.subgraph_count();

        if use_csr {
            #[cfg(feature = "arena")]
            render_csr(&dag, sg_count);
            #[cfg(not(feature = "arena"))]
            {
                let _ = (&dag, sg_count);
                println!("  (arena feature not enabled — run with --features arena)");
            }
        } else {
            render_heap(&dag, sg_count);
        }
        println!();
    }

    println!("Done. If the last tier was still readable, your terminal is a champ.");
}

fn render_heap(dag: &Graph, sgs: usize) {
    let start = Instant::now();
    let ir = dag.compute_layout();
    let output = ir.render_string(&RenderOptions::plain());
    let elapsed = start.elapsed();

    let lines = output.lines().count();
    let max_width = output.lines().map(|l| l.len()).max().unwrap_or(0);

    println!("{}", output);
    println!(
        "  [Heap] {} subgraphs → {}×{} chars, {:?}",
        sgs, max_width, lines, elapsed
    );
}

#[cfg(feature = "arena")]
fn render_csr(dag: &Graph, sgs: usize) {
    let csr_size = dag.estimate_csr_arena_size() * 2;
    let mut csr_buf = vec![0u8; csr_size];
    let mut csr_arena = Arena::new(&mut csr_buf);
    let csr = match dag.to_csr(&mut csr_arena) {
        Some(g) => g,
        None => {
            println!(
                "  (CSR conversion failed — arena too small: {} KB)",
                csr_size / 1024
            );
            return;
        }
    };

    let layout_size = dag.estimate_layout_arena_size();
    let size = ((layout_size * 6) / 5).max(256 * 1024);
    let mut temp_buf = vec![0u8; size];
    let mut out_buf = vec![0u8; size];
    let mut temp_arena = Arena::new(&mut temp_buf);
    let mut out_arena = Arena::new(&mut out_buf);

    let start = Instant::now();

    let ir = match csr.compute_layout_arena(
        &LayoutConfig::standard(),
        &mut temp_arena,
        &mut out_arena,
    ) {
        Ok(ir) => ir,
        Err(e) => {
            println!("  (Layout failed: {:?}, arena: {} KB)", e, size / 1024);
            return;
        }
    };

    let options = RenderOptions::plain();
    let mut arena_buf = vec![0u8; ir.estimate_render_arena_size(&options)];
    let render_arena = ascii_dag::graph::arena::Arena::new(&mut arena_buf);
    let mut render_buf = vec![0u8; ir.estimate_render_output_size(&options)];
    let bytes = ir
        .render_to_bytes(&options, &render_arena, &mut render_buf)
        .unwrap_or(0);
    let elapsed = start.elapsed();

    if let Ok(s) = std::str::from_utf8(&render_buf[..bytes]) {
        let lines = s.lines().count();
        let max_width = s.lines().map(|l| l.len()).max().unwrap_or(0);
        println!("{}", s);
        println!(
            "  [CSR]  {} subgraphs → {}×{} chars, {:?} (arena: {} KB)",
            sgs,
            max_width,
            lines,
            elapsed,
            (csr_size + size * 2) / 1024
        );
    }
}

include!("shared/stress_graphs.rs");
