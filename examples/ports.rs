//! Ports: declare which side of a node an edge leaves from or arrives
//! on, choose how a node places the ends on each face, and read back
//! which cell every end got — through the heap pipeline and, with the
//! `arena` feature, the no-alloc pipeline, byte for byte the same.
//!
//! Sections:
//! 1. the same graph undeclared, then with sides declared
//! 2. what the layout reports back: attachments on the IR and in JSON
//! 3. the three side vocabularies, resolved by the router per direction
//! 4. port policies on one boxed hub: `Single`, `Paired`, `Spread`, `Custom`
//! 5. the two warnings a declaration can raise, on both pipelines
//! 6. the no-alloc builder: `new_with_ports`, handles, policies, exact
//!    sizing, and the reporting layout entry (needs `arena`)
//!
//! Run:
//!   cargo run --example ports --features arena          # both pipelines
//!   cargo run --example ports                           # heap only
//!   cargo run --example ports --features arena -- --lr  # also --bt, --rl, --ascii

use ascii_dag::render::engine::{BoxedNode, Charset, RenderOptions};
use ascii_dag::{
    Direction, Graph, LayoutConfig, PhysicalSide, PortAttachment, PortBound, PortPolicy, PortSide,
    PortSlot,
};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let direction = args
        .iter()
        .rev()
        .find_map(|a| match a.as_str() {
            "--bt" => Some(Direction::BottomUp),
            "--lr" => Some(Direction::LeftRight),
            "--rl" => Some(Direction::RightLeft),
            _ => None,
        })
        .unwrap_or(Direction::TopDown);
    let mut options = RenderOptions::plain();
    if args.iter().any(|a| a == "--ascii") {
        options.emit.charset = Charset::Ascii;
    }
    if !cfg!(feature = "arena") {
        println!("(heap pipeline only — add --features arena to run the no-alloc pipeline too)\n");
    }

    // ── 1. The same graph, undeclared, then with sides ───────────────
    //
    // Without declarations the router attaches every edge head-on:
    // leaving the bottom of the source, entering the top of the target
    // (in TopDown). `from_port` names the side the edge LEAVES the
    // `from` node from; `to_port` the side it ARRIVES on at `to`.
    println!("── 1. Without declarations ({direction:?}) ──");
    show(&request_path(direction, false), &options);
    println!("── 1. With declared sides ──");
    let g = request_path(direction, true);
    show(&g, &options);

    // ── 2. What the layout reports back ──────────────────────────────
    //
    // Every IR edge carries an attachment per end: the side that was
    // asked for (`Auto` when nothing was declared) and the physical
    // side the router used.
    println!("── 2. Attachments on the IR ──");
    let ir = g.compute_layout();
    println!("  edge          leaves from      arrives on");
    for e in ir.edges() {
        println!(
            "  {:>2} → {:<2}       {:<16} {}",
            e.from_id,
            e.to_id,
            attachment(e.from_port),
            attachment(e.to_port),
        );
    }
    // JSON (schema 1.5) carries `from_side` / `to_side` on every edge
    // and `from_port` / `to_port` on the edges that declared one.
    let json = ir.to_json();
    println!(
        "  JSON: {} edges with sides, {} declared ports\n",
        json.matches("\"from_side\"").count(),
        json.matches("\"from_port\"").count() + json.matches("\"to_port\"").count(),
    );

    // ── 3. The vocabulary, resolved per direction ────────────────────
    //
    // Compass sides are fixed on the page; flow-relative and rotation
    // sides are fixed on the FLOW. The router is the authority: ask it.
    println!("── 3. Where each side lands, by direction (source end) ──");
    println!("  {:<18}TopDown   BottomUp  LeftRight RightLeft", "side");
    for side in [
        PortSide::Auto,
        PortSide::Upstream,
        PortSide::Downstream,
        PortSide::Clockwise,
        PortSide::Counterclockwise,
        PortSide::North,
        PortSide::East,
        PortSide::South,
        PortSide::West,
    ] {
        print!("  {:<18}", side.name());
        for dir in [
            Direction::TopDown,
            Direction::BottomUp,
            Direction::LeftRight,
            Direction::RightLeft,
        ] {
            let mut g = Graph::new();
            g.set_direction(dir);
            g.add_node(1usize, "A");
            g.add_node(2usize, "B");
            g.add_edge(1usize, 2usize, None).from_port(side);
            print!(
                "{:<10}",
                g.compute_layout().edges()[0].from_port.side.name()
            );
        }
        println!();
    }
    println!();

    // ── 4. Where on a face: port policies ────────────────────────────
    //
    // One boxed hub, the same declarations every time: three arrivals
    // on its top face, two departures on its bottom face, and on its
    // east face one arrival and one departure. Only the policy changes.
    // A face with one cell holds one port whatever the policy, so the
    // hub is boxed: its side faces have three rows.
    for (name, policy, what_to_see) in [
        (
            "Single (the default)",
            PortPolicy::Single,
            "one port per face: the three top arrivals converge, the east arrival and departure share a row",
        ),
        (
            "Paired",
            PortPolicy::Paired,
            "an arrival port and a departure port per face: the east pair takes two rows",
        ),
        (
            "Spread(Face)",
            PortPolicy::Spread(PortBound::Face),
            "as many ports as the face has cells: three top ports, two bottom ports, two east rows",
        ),
        (
            "Custom",
            PortPolicy::Custom(ends_apart),
            "your rule: arrivals on a face's first cell, departures on its last",
        ),
    ] {
        println!("── 4. Port policy {name} ──");
        println!("  {what_to_see}");
        let mut g = policy_hub(direction);
        g.set_port_policy(policy);
        show(&g, &options);
        let ir = g.compute_layout();
        println!(
            "  ports used — top: {}  bottom: {}  east: {}\n",
            ports_on(&ir, 0, PhysicalSide::North),
            ports_on(&ir, 0, PhysicalSide::South),
            ports_on(&ir, 0, PhysicalSide::East),
        );
    }

    // ── 5. When a declaration cannot be honored ──────────────────────
    //
    // Two conditions report, both as warnings on the layout run — the
    // picture still renders. A side on a self-loop is deferred (the
    // loop keeps its marker), and a side with no room beside the node
    // falls back to head-on, naming the end and both sides. The
    // no-alloc pipeline reports the same through
    // `compute_layout_arena_reporting`.
    println!("── 5. Declarations that could not be honored ──");
    let mut g = Graph::new();
    g.add_node(0usize, "Root");
    for (id, label) in [(1usize, "L"), (2, "M"), (3, "R")] {
        g.add_node(id, label);
        g.add_edge(0usize, id, None);
    }
    g.add_node(4usize, "T");
    g.add_edge(2usize, 4usize, None).from_port(PortSide::East);
    g.add_edge(4usize, 4usize, None).from_port(PortSide::East);
    // `node_spacing: 0` leaves no gap beside M for an eastward exit.
    let tight = LayoutConfig {
        node_spacing: 0,
        ..LayoutConfig::standard()
    };
    let report = g.layout().with_config(&tight).reported();
    for d in report.warnings() {
        println!("  heap:  {d}");
    }
    report_arena(&g, &tight);
    // With the standard spacing the eastward exit routes.
    println!(
        "  with room: {} warning(s)\n",
        g.layout().reported().warnings().count() // the self-loop's side stays deferred
    );

    // ── 6. The no-alloc builder ──────────────────────────────────────
    builder_section();
}

/// Client → Gateway → Service → Store, an audit tap off the side of
/// Service, and a cache Gateway fills from beside it.
fn request_path(direction: Direction, declared: bool) -> Graph<'static> {
    let mut g = Graph::new();
    g.set_direction(direction);
    g.add_node(1usize, "Client");
    g.add_node(2usize, "Gateway");
    g.add_node(3usize, "Service");
    g.add_node(4usize, "Store");
    g.add_node(5usize, "Audit");
    g.add_node(6usize, "Cache");
    g.add_edge(1usize, 2usize, None);
    g.add_edge(2usize, 3usize, None);
    g.add_edge(3usize, 4usize, None);
    if declared {
        // Clockwise: the traveler's right hand facing downstream —
        // West in TopDown, South in LeftRight.
        g.add_edge(3usize, 5usize, Some("trail"))
            .from_port(PortSide::Clockwise);
        // Compass: fixed on the page whatever the direction.
        g.add_edge(2usize, 6usize, None).to_port(PortSide::West);
        // Downstream at a TARGET: enter through the face edges normally
        // leave by — the edge goes around Store and comes up into it.
        g.add_edge(1usize, 4usize, Some("bypass"))
            .to_port(PortSide::Downstream);
    } else {
        g.add_edge(3usize, 5usize, Some("trail"));
        g.add_edge(2usize, 6usize, None);
        g.add_edge(1usize, 4usize, Some("bypass"));
    }
    g
}

/// The policy hub: id 0, boxed; three `Upstream` arrivals, two
/// `Downstream` departures, an `East` arrival and an `East` departure.
fn policy_hub(direction: Direction) -> Graph<'static> {
    let mut g = Graph::new();
    g.set_direction(direction);
    g.add_node(0usize, BoxedNode("Hub node"));
    for (id, label) in [(1usize, "In"), (2, "In"), (3, "In"), (4, "Side")] {
        g.add_node(id, label);
    }
    for (id, label) in [(5usize, "Out"), (6, "Out"), (7, "Back")] {
        g.add_node(id, label);
    }
    for src in 1usize..=3 {
        g.add_edge(src, 0usize, None).to_port(PortSide::Upstream);
    }
    for dst in 5usize..=6 {
        g.add_edge(0usize, dst, None)
            .from_port(PortSide::Downstream);
    }
    g.add_edge(4usize, 0usize, None).to_port(PortSide::East);
    g.add_edge(0usize, 7usize, None).from_port(PortSide::East);
    g
}

/// A custom placer: arrivals on the face's first cell, departures on
/// its last. The slot also names the node, the face, its cells and how
/// many ends it carries, for rules that need them.
fn ends_apart(slot: PortSlot) -> usize {
    if slot.arrival { 0 } else { slot.cells - 1 }
}

/// Distinct cells the ends attached on `side` of node `id` use.
fn ports_on(ir: &ascii_dag::ir::LayoutIR<'_>, id: usize, side: PhysicalSide) -> usize {
    let mut cells: Vec<(usize, usize)> = ir
        .edges()
        .iter()
        .flat_map(|e| {
            let mut v = Vec::new();
            if e.to_id == id && e.to_port.side == side {
                v.push((e.to_x, e.to_y));
            }
            if e.from_id == id && e.from_port.side == side {
                v.push((e.from_x, e.from_y));
            }
            v
        })
        .collect();
    cells.sort_unstable();
    cells.dedup();
    cells.len()
}

fn attachment(a: PortAttachment) -> String {
    match a.requested {
        PortSide::Auto => format!("{} (auto)", a.side.name()),
        side => format!("{} ← {}", a.side.name(), side.name()),
    }
}

/// Render `g` through the heap pipeline, print it, and — with `arena`
/// — render it through the no-alloc pipeline too and say whether the
/// two agree (they always do; the parity suite pins it).
fn show(g: &Graph<'_>, options: &RenderOptions) {
    let heap = g.compute_layout().render_string(options);
    println!("{heap}");
    #[cfg(feature = "arena")]
    {
        let mut config = LayoutConfig::standard();
        config.direction = g.direction();
        let arena = render_arena(g, &config, options);
        println!(
            "  arena pipeline: {}",
            if arena == heap {
                "byte-identical"
            } else {
                "DIFFERS"
            }
        );
    }
}

/// The no-alloc pipeline on a heap graph: `to_csr`, then layout and
/// render from exactly estimated arenas.
#[cfg(feature = "arena")]
fn render_arena(g: &Graph<'_>, config: &LayoutConfig<'_>, options: &RenderOptions) -> String {
    use ascii_dag::graph::arena::Arena;
    let mut csr_buf = vec![0u8; g.estimate_csr_arena_size()];
    let mut csr_arena = Arena::new(&mut csr_buf);
    let csr = g.to_csr(&mut csr_arena).expect("CSR conversion");
    let size = g.estimate_layout_arena_size_with(config);
    let (mut temp, mut out) = (vec![0u8; size], vec![0u8; size]);
    let (mut temp_arena, mut out_arena) = (Arena::new(&mut temp), Arena::new(&mut out));
    csr.compute_layout_arena(config, &mut temp_arena, &mut out_arena)
        .expect("CSR layout")
        .render_string(options)
}

/// The no-alloc pipeline's port conditions, through the reporting
/// layout entry.
#[cfg(feature = "arena")]
fn report_arena(g: &Graph<'_>, config: &LayoutConfig<'_>) {
    use ascii_dag::diagnostics::{DiagnosticRun, VecDiagnostics};
    use ascii_dag::graph::arena::Arena;
    let mut csr_buf = vec![0u8; g.estimate_csr_arena_size()];
    let mut csr_arena = Arena::new(&mut csr_buf);
    let csr = g.to_csr(&mut csr_arena).expect("CSR conversion");
    let size = g.estimate_layout_arena_size_with(config);
    let (mut temp, mut out) = (vec![0u8; size], vec![0u8; size]);
    let (mut temp_arena, mut out_arena) = (Arena::new(&mut temp), Arena::new(&mut out));
    let mut run = DiagnosticRun::new(VecDiagnostics::default());
    let ir = {
        let mut cx = run.context();
        csr.compute_layout_arena_reporting(config, &mut temp_arena, &mut out_arena, &mut cx)
    };
    let report = run.finish(ir);
    for d in report.warnings() {
        println!("  arena: {d}");
    }
}

#[cfg(not(feature = "arena"))]
fn report_arena(_g: &Graph<'_>, _config: &LayoutConfig<'_>) {}

/// Ports on the no-alloc builder: one fixed block backs the graph, the
/// layout and the text; the graph arena is sized exactly.
#[cfg(feature = "arena")]
fn builder_section() {
    use ascii_dag::diagnostics::{DiagnosticRun, VecDiagnostics};
    use ascii_dag::graph::arena::Arena;
    use ascii_dag::graph::csr::{CsrGraph, CsrGraphBuilder};
    println!("── 6. The no-alloc builder ──");
    const NODES: usize = 6;
    const EDGES: usize = 6;
    const LABEL_BYTES: usize = 48;
    let mut block = [0u8; 24 * 1024];

    // A builder constructed WITH ports carries two bytes per edge and a
    // policy byte per node, so declaring is a store, never an
    // allocation. Size the arena with the matching estimator.
    let graph_need = CsrGraph::required_arena_size_with_ports(NODES, EDGES, LABEL_BYTES, 0, 0);
    let (graph_mem, rest) = block.split_at_mut(graph_need);
    let mut graph_arena = Arena::new(graph_mem);
    let mut b = CsrGraphBuilder::new_with_ports(&mut graph_arena, NODES, EDGES, LABEL_BYTES, 0, 0)
        .expect("sized by required_arena_size_with_ports");
    let client = b.add_node(1, "Client").expect("node");
    let gateway = b.add_node(2, "Gateway").expect("node");
    // A boxed node has the rows a `Paired` side face needs.
    let service = b.add_node(3, BoxedNode("Service")).expect("node");
    b.set_node_port_policy(service, PortPolicy::Paired)
        .expect("builder has a port table");
    let store = b.add_node(4, "Store").expect("node");
    let audit = b.add_node(5, "Audit").expect("node");
    let cache = b.add_node(6, "Cache").expect("node");
    b.add_edge(client, gateway).expect("edge");
    b.add_edge(gateway, service).expect("edge");
    b.add_edge(service, store).expect("edge");
    // The handle form: `None` only from a builder without a port table.
    b.add_edge_with_label(service, audit, "trail")
        .expect("edge")
        .from_port(PortSide::Clockwise)
        .expect("builder has a port table");
    b.add_edge(gateway, cache)
        .expect("edge")
        .to_port(PortSide::West)
        .expect("builder has a port table");
    // The index form, for a declaration made after the fact.
    let bypass = b
        .add_edge_with_label(client, store, "bypass")
        .expect("edge")
        .edge();
    b.set_edge_ports(bypass, PortSide::Auto, PortSide::Downstream)
        .expect("edge exists and the builder has a port table");
    let graph = b.build().expect("build");

    // The layout estimator lives on the heap `Graph`, so a pure
    // no-alloc build provisions the layout arenas: the rest of the
    // block, split evenly. `ArenaOom` is the signal to grow the block.
    let config = LayoutConfig::standard();
    let (temp_mem, out_mem) = rest.split_at_mut(rest.len() / 2);
    let layout_need = temp_mem.len();
    let mut out_arena = Arena::new(out_mem);
    let mut run = DiagnosticRun::new(VecDiagnostics::default());
    let layout = {
        let mut temp_arena = Arena::new(temp_mem);
        let mut cx = run.context();
        graph
            .compute_layout_arena_reporting(&config, &mut temp_arena, &mut out_arena, &mut cx)
            .expect("layout")
    };
    let report = run.finish(Ok::<_, ascii_dag::GraphError>(()));
    println!("  port conditions reported: {}", report.warnings().count());

    // Render, reusing the layout scratch, sized by the estimators.
    let options = RenderOptions::plain();
    let scratch_need = layout.estimate_render_arena_size(&options);
    let text_need = layout.estimate_render_output_size(&options);
    let (scratch_mem, text_mem) = temp_mem.split_at_mut(scratch_need);
    let render_arena = Arena::new(scratch_mem);
    let written = layout
        .render_to_bytes(&options, &render_arena, &mut text_mem[..text_need])
        .expect("render");
    println!(
        "{}",
        core::str::from_utf8(&text_mem[..written]).expect("utf-8")
    );
    println!("  edge      requested → used");
    for e in layout.edges() {
        println!(
            "  {} → {}     from {:<10} → {:<6} to {:<10} → {}",
            e.from_id,
            e.to_id,
            e.from_port.requested.name(),
            e.from_port.side.name(),
            e.to_port.requested.name(),
            e.to_port.side.name(),
        );
    }
    println!(
        "  memory: graph {graph_need} B exact, layout {layout_need} B ×2 provisioned, render {scratch_need} + {text_need} B exact, of a {} B block",
        block.len()
    );
}

#[cfg(not(feature = "arena"))]
fn builder_section() {
    println!(
        "── 6. The no-alloc builder ──\n  needs the arena feature: cargo run --example ports --features arena"
    );
}
