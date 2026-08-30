//! Interactive hit-testing — zero dependencies.
//!
//! Renders a graph, enables xterm SGR mouse reporting with a raw ANSI
//! escape (`\x1b[?1006h`), switches the terminal to raw mode via
//! `stty` (std::process — no crate), and feeds every click through
//! `Scene::hit_test`. Unix terminals only; anywhere else use
//! probe mode:
//!
//!   cargo run --example hit_test                     # interactive
//!   cargo run --example hit_test -- --probe 12 3     # one lookup
//!   cargo run --example hit_test --features arena -- --csr --probe 12 3
//!
//! Click nodes (the card's whole area counts), edges, the cluster
//! border. `q` quits.

use ascii_dag::render::engine::{NodePaintCtx, NodeRegion};
use ascii_dag::{AUTO, BoxedNode, CustomNode, Graph, RenderOptions, ScenePlanner};
use std::io::{Read, Write};

#[path = "support/csr.rs"]
mod csr;

fn card(region: &mut NodeRegion<'_, '_>, ctx: NodePaintCtx<'_>) {
    region.write_str(1, 0, ctx.label);
    region.hrule(0, region.width() - 1, 1); // semantic: `-` under --ascii
    for (i, line) in ctx.payload.lines().enumerate() {
        region.write_str(1, 2 + i, line);
    }
}

fn graph() -> Graph<'static> {
    let mut g = Graph::new();
    let client = g.add_node(AUTO, "Client");
    let server = g.add_node(
        AUTO,
        CustomNode {
            label: "Server",
            width: 12,
            height: 4,
            painter: Some(card),
            payload: "cpu: 4",
        },
    );
    let db = g.add_node(AUTO, BoxedNode("Database"));
    let cache = g.add_node(AUTO, "Cache");
    g.add_edge(client, server, None);
    g.add_edge(server, db, Some("reads"));
    g.add_edge(server, cache, None);
    let sg = g.add_subgraph("Backend");
    g.put_nodes(&[server, db, cache]).inside(sg).unwrap();
    g
}

/// One (x, y) lookup against the scene, formatted for humans.
fn describe(hit: ascii_dag::render::engine::HitResult) -> String {
    format!("{hit:?}")
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let options = RenderOptions::plain();
    let g = graph();

    // One planner serves both pipelines — the scene type is the same
    // whichever backend produced the layout (--csr = arena).
    macro_rules! with_ir {
        ($ir:expr) => {{
            let ir = $ir;
            let mut planner = ScenePlanner::new();
            let scene = planner.plan(ir, &options.plan).expect("plan");
            let rendered = ir.render_string(&options);
            if let Some(i) = args.iter().position(|a| a == "--probe") {
                let x: usize = args[i + 1].parse().expect("--probe X Y");
                let y: usize = args[i + 2].parse().expect("--probe X Y");
                println!("{rendered}");
                println!("({x}, {y}) → {}", describe(scene.hit_test(x, y)));
                return;
            }
            interactive(&rendered, |x, y| describe(scene.hit_test(x, y)));
        }};
    }

    #[cfg(feature = "arena")]
    if csr::requested() {
        use ascii_dag::LayoutConfig;
        use ascii_dag::graph::arena::Arena;
        let mut csr_buf = vec![0u8; g.estimate_csr_arena_size() * 2];
        let mut csr_arena = Arena::new(&mut csr_buf);
        let csr = g.to_csr(&mut csr_arena).expect("CSR conversion");
        let size = (g.estimate_layout_arena_size() * 2).max(256 * 1024);
        let mut temp_buf = vec![0u8; size];
        let mut out_buf = vec![0u8; size];
        let mut ta = Arena::new(&mut temp_buf);
        let mut oa = Arena::new(&mut out_buf);
        let ir = csr
            .compute_layout_arena(&LayoutConfig::standard(), &mut ta, &mut oa)
            .expect("CSR layout");
        with_ir!(&ir);
        return;
    }
    #[cfg(not(feature = "arena"))]
    if csr::requested() {
        let _ = csr::render(&g, &options); // prints the arena-feature hint and exits
    }

    with_ir!(&g.compute_layout());
}

/// Raw-mode click loop: parse SGR mouse reports (`\x1b[<b;col;rowM`)
/// from stdin and show the probe result on a status line.
fn interactive(rendered: &str, probe: impl Fn(usize, usize) -> String) {
    // Raw mode via stty — no crate. If it fails (not a tty, Windows),
    // fall back to probe-mode instructions.
    let raw_ok = std::process::Command::new("stty")
        .args(["raw", "-echo"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !raw_ok {
        println!("{rendered}");
        println!("(no tty — use: cargo run --example hit_test -- --probe X Y)");
        return;
    }

    let mut out = std::io::stdout();
    let rows = rendered.lines().count();
    // Clear, home, draw at origin so terminal (col-1, row-1) == plan (x, y).
    let _ = write!(out, "\x1b[2J\x1b[H");
    for line in rendered.lines() {
        let _ = write!(out, "{line}\r\n");
    }
    let status_row = rows + 2;
    let _ = write!(
        out,
        "\x1b[{status_row};1H\x1b[2Kclick anywhere (q quits) — hit: ",
    );
    // Enable SGR mouse reporting.
    let _ = write!(out, "\x1b[?1000h\x1b[?1006h");
    let _ = out.flush();

    let mut stdin = std::io::stdin();
    let mut byte = [0u8; 1];
    let mut seq: Vec<u8> = Vec::new();
    let mut in_seq = false;
    while stdin.read_exact(&mut byte).is_ok() {
        let b = byte[0];
        if in_seq {
            seq.push(b);
            // SGR report ends in 'M' (press) or 'm' (release).
            if b == b'M' || b == b'm' {
                if b == b'M'
                    && let Some((x, y)) = parse_sgr(&seq)
                {
                    let hit = probe(x, y);
                    let _ = write!(
                        out,
                        "\x1b[{status_row};1H\x1b[2K({x}, {y}) → {hit}  (q quits)"
                    );
                    let _ = out.flush();
                }
                seq.clear();
                in_seq = false;
            }
            continue;
        }
        match b {
            0x1b => {
                in_seq = true;
                seq.clear();
            }
            b'q' | 3 => break, // q or Ctrl-C
            _ => {}
        }
    }

    // Mouse off, terminal back to sanity, cursor below everything.
    let _ = write!(out, "\x1b[?1006l\x1b[?1000l\x1b[{};1H\r\n", status_row + 1);
    let _ = out.flush();
    let _ = std::process::Command::new("stty").arg("sane").status();
}

/// Parse `[<b;col;rowM` (the ESC was consumed by the caller).
/// Terminal coordinates are 1-based; the plan's are 0-based.
fn parse_sgr(seq: &[u8]) -> Option<(usize, usize)> {
    let s = core::str::from_utf8(seq).ok()?;
    let body = s.strip_prefix("[<")?.trim_end_matches(['M', 'm']);
    let mut parts = body.split(';');
    let _button = parts.next()?;
    let col: usize = parts.next()?.parse().ok()?;
    let row: usize = parts.next()?.parse().ok()?;
    Some((col.checked_sub(1)?, row.checked_sub(1)?))
}
