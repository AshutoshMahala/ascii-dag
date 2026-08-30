//! Spike: painter charset independence for the 0.11 scene work
//! (prototype stage — see temp/scene-api-sketch.md §9 and
//! temp/spike-4.0d-findings.md).
//!
//! **THROWAWAY CODE.** Test-only, deleted when the real painter API
//! lands. The semantic primitives it exercises live as test-gated
//! methods on `NodeRegion` (region.rs) so they run through the REAL
//! compositor and emission path. Questions:
//!
//! 1. Can a custom painter be byte-correct under BOTH charsets without
//!    ever reading `NodePaintCtx.charset` — drawing structure as
//!    semantic stroke/marker cells decoded at emission like all engine
//!    ink?
//! 2. Do painter strokes get junction merging for free (rule flush
//!    with a frame → tee; crossing rules → cross) — something raw
//!    `char` writes can never do?
//! 3. Do the existing painter examples migrate byte-identically —
//!    including the charset `match` in examples/node_painting.rs
//!    (deleted) and the hardcoded `'─'` in examples/hit_test.rs (a
//!    latent ASCII-mode bug this API class eliminates)?
//! 4. Does the arena backend serve the same painter path byte-for-byte?

use super::region::{NodePaintCtx, NodeRegion};
use crate::graph::Graph;
use crate::render::engine::CustomNode;
use crate::{Charset, RenderOptions};

// ── Painters ─────────────────────────────────────────────────────────────

/// Structure-heavy card drawn ENTIRELY through semantic primitives —
/// never reads `ctx.charset`. Frame, two dividers, a crossing column,
/// an arrow marker, label and payload text.
fn semantic_card(region: &mut NodeRegion<'_, '_>, ctx: NodePaintCtx<'_>) {
    let right = region.width() - 1;
    region.spike_frame();
    region.write_str(2, 1, ctx.label);
    region.spike_hrule(0, right, 2);
    region.spike_hrule(0, right, 4);
    region.spike_vrule(7, 2, 6);
    for (i, line) in ctx.payload.lines().enumerate() {
        region.write_str(1, 3 + i, line);
    }
    region.spike_arrow(3, 5, super::cell::Dir::Down);
    region.write_str(1, 6, "ok");
}

/// The `card` painter from examples/node_painting.rs, verbatim — the
/// charset `match` is the code this spike deletes.
fn example_card_today(region: &mut NodeRegion<'_, '_>, ctx: NodePaintCtx<'_>) {
    region.write_str(1, 0, ctx.label);
    let rule = match ctx.charset {
        Charset::Ascii => '-',
        _ => '─',
    };
    for x in 0..region.width() {
        region.set(x, 1, rule);
    }
    for (i, line) in ctx.payload.lines().enumerate() {
        region.write_str(1, 2 + i, line);
    }
}

/// The same card migrated: one semantic rule, no charset anywhere.
fn example_card_migrated(region: &mut NodeRegion<'_, '_>, ctx: NodePaintCtx<'_>) {
    region.write_str(1, 0, ctx.label);
    region.spike_hrule(0, region.width() - 1, 1);
    for (i, line) in ctx.payload.lines().enumerate() {
        region.write_str(1, 2 + i, line);
    }
}

type Painter = fn(&mut NodeRegion<'_, '_>, NodePaintCtx<'_>);

// ── Fixtures / rendering ─────────────────────────────────────────────────

fn card_graph(painter: Painter, width: usize, height: usize) -> Graph<'static> {
    let mut g = Graph::new();
    g.add_node(1usize, "A");
    g.add_node(
        10usize,
        CustomNode {
            label: "Server",
            width,
            height,
            painter: Some(painter),
            payload: "cpu: 4",
        },
    );
    g.add_edge(1usize, 10usize, None);
    g
}

fn options(charset: Charset) -> RenderOptions {
    let mut o = RenderOptions::plain();
    o.charset = charset;
    o
}

fn render_heap(g: &Graph<'_>, charset: Charset) -> String {
    g.compute_layout().render_string(&options(charset))
}

fn render_arena(g: &Graph<'_>, charset: Charset) -> String {
    let cfg = crate::LayoutConfig::standard();
    let mut csr_buf = vec![0u8; g.estimate_csr_arena_size() * 2];
    let mut csr_arena = crate::graph::arena::Arena::new(&mut csr_buf);
    let csr = g.to_csr(&mut csr_arena).unwrap();
    let size = (g.estimate_layout_arena_size() * 2).max(256 * 1024);
    let mut temp_buf = vec![0u8; size];
    let mut out_buf = vec![0u8; size];
    let mut temp_arena = crate::graph::arena::Arena::new(&mut temp_buf);
    let mut out_arena = crate::graph::arena::Arena::new(&mut out_buf);
    let ir = csr
        .compute_layout_arena(&cfg, &mut temp_arena, &mut out_arena)
        .unwrap();
    let mut out = String::new();
    ir.render_with(&options(charset), &mut out).unwrap();
    out
}

// ── The proofs ───────────────────────────────────────────────────────────

/// Questions 1 and 2: one charset-blind painter, byte-correct under
/// both charsets — with tees where its rules meet its frame (`├ ┬ ┤`),
/// a cross where its rules cross (`┼`), and a real arrow marker
/// (`↓`/`v`), all decoded per charset at emission. Raw `char` writes
/// could produce none of those junctions.
#[test]
fn semantic_painter_is_byte_correct_under_both_charsets() {
    let g = card_graph(semantic_card, 12, 8);
    assert_eq!(
        render_heap(&g, Charset::Unicode),
        "      [A]\n       └┐\n        ↓\n  \
         ┌──────────┐\n  \
         │ Server   │\n  \
         ├──────┬───┤\n  \
         │cpu: 4│   │\n  \
         ├──────┼───┤\n  \
         │  ↓   │   │\n  \
         │ok    │   │\n  \
         └──────────┘\n\n\n"
    );
    assert_eq!(
        render_heap(&g, Charset::Ascii),
        "      [A]\n       ++\n        v\n  \
         +----------+\n  \
         | Server   |\n  \
         +------+---+\n  \
         |cpu: 4|   |\n  \
         +------+---+\n  \
         |  v   |   |\n  \
         |ok    |   |\n  \
         +----------+\n\n\n"
    );
}

/// Question 3: the example painter migrates byte-identically under
/// both charsets — the semantic rule decodes to exactly the glyphs the
/// charset `match` used to pick by hand.
#[test]
fn example_card_migrates_byte_identically() {
    for charset in [Charset::Unicode, Charset::Ascii] {
        let today = card_graph(example_card_today, 12, 5);
        let migrated = card_graph(example_card_migrated, 12, 5);
        assert_eq!(
            render_heap(&today, charset),
            render_heap(&migrated, charset),
            "migration changed bytes under {charset:?}"
        );
    }
}

/// Question 4: the arena backend runs the same painter path
/// byte-for-byte, both charsets.
#[test]
fn arena_backend_serves_painters_byte_identically() {
    let g = card_graph(semantic_card, 12, 8);
    for charset in [Charset::Unicode, Charset::Ascii] {
        assert_eq!(
            render_heap(&g, charset),
            render_arena(&g, charset),
            "backend divergence under {charset:?}"
        );
    }
}
