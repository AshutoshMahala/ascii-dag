//! Dual-run byte-parity harness (RW3 exit criteria, R6.2).
//!
//! Every corpus graph renders through the legacy renderers AND the
//! engine, from **both IRs**, and the bytes must match. This harness is
//! the migration's arbiter: the legacy renderers are deleted (RW8) only
//! after it stays green across the corpus.

#![cfg(all(test, feature = "std", feature = "arena"))]

use super::config::RenderOptions;
use super::render_plain;
use crate::algorithms::sugiyama::config::LayoutConfig;
use crate::graph::Graph;
use crate::graph::arena::Arena;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// First differing line, with context, for readable failures.
fn assert_same(tag: &str, legacy: &str, engine: &str) {
    if legacy == engine {
        return;
    }
    let l: Vec<&str> = legacy.lines().collect();
    let e: Vec<&str> = engine.lines().collect();
    for i in 0..l.len().max(e.len()) {
        let a = l.get(i).copied().unwrap_or("<missing>");
        let b = e.get(i).copied().unwrap_or("<missing>");
        if a != b {
            panic!(
                "{tag}: first divergence at line {i}\n legacy: {a:?}\n engine: {b:?}\n\
                 ── legacy full ──\n{legacy}\n── engine full ──\n{engine}"
            );
        }
    }
    panic!("{tag}: outputs differ only in trailing newlines\nlegacy={legacy:?}\nengine={engine:?}");
}

fn csr_legacy_and_engine(g: &Graph<'_>, options: &RenderOptions) -> (String, String) {
    let config = LayoutConfig::standard();
    let mut csr_buf = vec![0u8; g.estimate_csr_arena_size() * 2];
    let mut csr_arena = Arena::new(&mut csr_buf);
    let csr = g.to_csr(&mut csr_arena).expect("CSR conversion");
    let size = (g.estimate_layout_arena_size() * 2).max(256 * 1024);
    let mut temp_buf = vec![0u8; size];
    let mut out_buf = vec![0u8; size];
    let mut temp_arena = Arena::new(&mut temp_buf);
    let mut out_arena = Arena::new(&mut out_buf);
    let ir = csr
        .compute_layout_arena(&config, &mut temp_arena, &mut out_arena)
        .expect("CSR layout");

    let (render_bytes, _) = ir.estimate_render_size();
    let mut render_buf = vec![0u8; render_bytes * 4 + 8192];
    let mut line_buf = vec![' '; ir.width().max(1) + 32];
    let mut scratch = vec![0usize; (ir.height() + ir.edge_count() * 2).max(1) + 64];
    let bytes = ir
        .render_to_buffer(&mut render_buf, &mut line_buf, &mut scratch)
        .expect("legacy arena render");
    let legacy = String::from_utf8_lossy(&render_buf[..bytes]).into_owned();

    let engine = render_plain(&ir, options);
    (legacy, engine)
}

fn check_heap(tag: &str, g: &Graph<'_>) {
    let ir = g.compute_layout();
    let legacy = ir.render_scanline();
    let engine = render_plain(&ir, &RenderOptions::plain());
    assert_same(&format!("{tag} (heap)"), &legacy, &engine);
}

fn check_csr(tag: &str, g: &Graph<'_>) {
    let (legacy, engine) = csr_legacy_and_engine(g, &RenderOptions::plain());
    assert_same(&format!("{tag} (csr)"), &legacy, &engine);
}

/// The parity check: engine output must byte-match the legacy plain
/// renderers from both IRs.
fn check(tag: &str, g: &Graph<'_>) {
    check_heap(tag, g);
    check_csr(tag, g);
}

/// A corpus entry: fixture name + builder.
type CorpusEntry = (&'static str, fn() -> Graph<'static>);

// ── Corpus ───────────────────────────────────────────────────────────────

fn chain() -> Graph<'static> {
    let mut g = Graph::new();
    g.add_node(1, "A");
    g.add_node(2, "B");
    g.add_node(3, "C");
    g.add_edge(1, 2, None);
    g.add_edge(2, 3, None);
    g
}

fn fan() -> Graph<'static> {
    let mut g = Graph::new();
    g.add_node(1, "P");
    for i in 2..=5 {
        g.add_node(i, "kid");
        g.add_edge(1, i, None);
    }
    g.add_node(9, "sink");
    for i in 2..=5 {
        g.add_edge(i, 9, None);
    }
    g
}

fn stage() -> Graph<'static> {
    let mut g = Graph::new();
    g.add_node(1, "Start");
    g.add_node(2, "Middle");
    g.add_node(3, "End");
    g.add_edge(1, 2, Some("go"));
    g.add_edge(2, 3, None);
    let sg = g.add_subgraph("Stage");
    g.put_nodes(&[2]).inside(sg).unwrap();
    g
}

fn skip() -> Graph<'static> {
    let mut g = Graph::new();
    g.add_node(1, "A");
    g.add_node(2, "B");
    g.add_node(3, "C");
    g.add_node(4, "D");
    g.add_edge(1, 2, None);
    g.add_edge(2, 3, None);
    g.add_edge(3, 4, None);
    g.add_edge(1, 4, None);
    g
}

fn back_edges() -> Graph<'static> {
    let mut g = Graph::new();
    g.add_node(1, "compile");
    g.add_node(2, "link");
    g.add_node(3, "test");
    g.add_node(4, "deploy");
    g.add_edge(1, 2, None);
    g.add_edge(2, 3, None);
    g.add_edge(3, 4, None);
    g.add_edge(4, 1, None); // long cycle → dashed back edge
    g
}

fn two_cycle() -> Graph<'static> {
    let mut g = Graph::new();
    g.add_node(1, "Ping");
    g.add_node(2, "Pong");
    g.add_edge(1, 2, None);
    g.add_edge(2, 1, None);
    g
}

fn self_loop() -> Graph<'static> {
    let mut g = Graph::new();
    g.add_node(1, "Gate");
    g.add_node(2, "Next");
    g.add_edge(1, 1, None);
    g.add_edge(1, 2, None);
    g
}

fn colliding_labels() -> Graph<'static> {
    let mut g = Graph::new();
    g.add_node(1, "A");
    g.add_node(2, "B");
    g.add_node(3, "C");
    g.add_node(4, "D");
    g.add_edge(1, 3, Some("averyveryverylonglabel"));
    g.add_edge(2, 4, Some("anotherverylonglabel"));
    g
}

fn nested_boxes() -> Graph<'static> {
    let mut g = Graph::new();
    g.add_node(1, "In");
    g.add_node(2, "Work");
    g.add_node(3, "Store");
    g.add_node(4, "Out");
    g.add_edge(1, 2, None);
    g.add_edge(2, 3, None);
    g.add_edge(3, 4, None);
    let outer = g.add_subgraph("Outer");
    let inner = g.add_subgraph("Inner");
    g.put_subgraphs(&[inner]).inside(outer).unwrap();
    g.put_nodes(&[2]).inside(outer).unwrap();
    g.put_nodes(&[3]).inside(inner).unwrap();
    g
}

fn implicit_nodes() -> Graph<'static> {
    let mut g = Graph::new();
    g.add_node(1, "Root");
    // 7 and 8 are auto-created (implicit).
    g.add_edge(1, 7, None);
    g.add_edge(7, 8, None);
    g
}

// ── The harness ──────────────────────────────────────────────────────────

#[test]
fn parity_chain() {
    check("chain", &chain());
}

// DIVERGENCE CLASS A — resolved by ruling (0.10.0): the legacy *plain*
// path overwrites corner cells lossily (last edge wins: `┌───└───┐`)
// where the engine merges arms into the correct junctions
// (`┬───┴───┬` — what the legacy *colored* path already renders). The
// engine's output is canonical; these legacy-comparison tests retire
// with the legacy renderers at RW8, when the goldens regenerate.
#[test]
#[ignore = "class A divergence resolved by ruling: engine junctions are canonical; retires at RW8"]
fn parity_fan() {
    check("fan", &fan());
}

#[test]
fn parity_stage() {
    check("stage", &stage());
}

#[test]
#[ignore = "class A divergence resolved by ruling: engine junctions are canonical; retires at RW8"]
fn parity_skip() {
    check("skip", &skip());
}

#[test]
#[ignore = "class A divergence resolved by ruling: engine junctions are canonical; retires at RW8"]
fn parity_back_edges() {
    check("back_edges", &back_edges());
}

#[test]
fn parity_two_cycle() {
    check("two_cycle", &two_cycle());
}

#[test]
fn parity_self_loop() {
    check("self_loop", &self_loop());
}

#[test]
fn parity_colliding_labels_heap() {
    check_heap("colliding_labels", &colliding_labels());
}

// DIVERGENCE CLASS D — resolved by ruling (0.10.0): when an *edge*
// label's quoted span exceeds the canvas width, the legacy heap path
// skips it but the legacy arena path paints it truncated mid-word
// without a closing quote, overwriting edge strokes. The engine's
// behavior (skip, like heap) is canonical; this legacy comparison
// retires with the legacy renderers at RW8.
#[test]
#[ignore = "class D divergence resolved by ruling: skipping unfittable edge labels is canonical; retires at RW8"]
fn parity_colliding_labels_csr() {
    check_csr("colliding_labels", &colliding_labels());
}

// DIVERGENCE CLASS B — resolved by ruling (0.10.0): nested cluster
// borders CAN share cells, and the legacy renderers keep the
// first-painted double glyph while the engine merges the overlapping
// borders into the proper junction (`╠`). The engine's output is
// canonical; this legacy comparison retires at RW8.
#[test]
#[ignore = "class B divergence resolved by ruling: merged border junctions are canonical; retires at RW8"]
fn parity_nested_boxes() {
    check("nested_boxes", &nested_boxes());
}

#[test]
fn parity_implicit_nodes_csr() {
    check_csr("implicit_nodes", &implicit_nodes());
}

// DIVERGENCE CLASS C — resolved by ruling (0.10.0): implicit nodes get
// layout width 3, and the legacy arena renderer honors it (`[ ]`) while
// the legacy heap renderer paints label-compact (`[]`), leaving a cell
// of the reserved width unpainted. The engine honors the IR's width
// (consistent with the layout's own bookkeeping); canonical. This
// legacy comparison retires at RW8.
#[test]
#[ignore = "class C divergence resolved by ruling: IR-width node painting is canonical; retires at RW8"]
fn parity_implicit_nodes_heap() {
    check_heap("implicit_nodes", &implicit_nodes());
}

/// The legacy-free invariant (N1): the engine renders byte-identical
/// output from both IRs for every corpus graph — including the fixtures
/// where the legacy renderers disagreed among themselves.
#[test]
fn engine_self_parity_across_backends() {
    let corpus: [CorpusEntry; 10] = [
        ("chain", chain),
        ("fan", fan),
        ("stage", stage),
        ("skip", skip),
        ("back_edges", back_edges),
        ("two_cycle", two_cycle),
        ("self_loop", self_loop),
        ("colliding_labels", colliding_labels),
        ("nested_boxes", nested_boxes),
        ("implicit_nodes", implicit_nodes),
    ];
    for (tag, build) in corpus {
        let g = build();
        let ir = g.compute_layout();
        let heap_out = render_plain(&ir, &RenderOptions::plain());
        let (_, csr_out) = csr_legacy_and_engine(&g, &RenderOptions::plain());
        assert_same(&format!("{tag} (engine heap vs engine csr)"), &heap_out, &csr_out);
    }
}

/// Canonical spot checks for the ruled divergence classes: the corrected
/// glyphs must actually appear in the engine's output.
#[test]
fn ruled_classes_render_canonically() {
    // Class A: fan-out/fan-in junctions, not overwritten corners.
    let fan_out = render_plain(&fan().compute_layout(), &RenderOptions::plain());
    assert!(fan_out.contains('┴') && fan_out.contains('┬'), "{fan_out}");
    assert!(!fan_out.contains("┌───└"), "{fan_out}");

    // Class B: overlapping nested borders merge into a junction.
    let nested = render_plain(&nested_boxes().compute_layout(), &RenderOptions::plain());
    assert!(nested.contains('╠') || nested.contains('╣'), "{nested}");

    // Class C: implicit nodes honor their IR width.
    let implicit = render_plain(&implicit_nodes().compute_layout(), &RenderOptions::plain());
    assert!(implicit.contains("[ ]"), "{implicit}");

    // Class D: unfittable edge labels are skipped — never truncated
    // without a closing quote.
    let collide = render_plain(&colliding_labels().compute_layout(), &RenderOptions::plain());
    assert_eq!(collide.matches('"').count() % 2, 0, "{collide}");
}

/// The hero example against both the legacy renderer and the golden
/// file — the strongest single fixture in the repo.
#[test]
#[ignore = "class A divergence resolved by ruling: engine junctions are canonical; retires at RW8"]
fn parity_hero_and_golden() {
    let g = hero_graph();
    let ir = g.compute_layout();
    let legacy = ir.render_scanline();
    let engine = render_plain(&ir, &RenderOptions::plain());
    assert_same("hero (heap)", &legacy, &engine);

    let golden = include_str!("../../../tests/golden/hero.txt");
    assert_same("hero (golden)", golden.trim_end(), engine.trim_end());
}

// The shared hero fixture refers to the crate by its external name.
use crate as ascii_dag;
include!("../../../examples/shared/hero_graph.rs");
