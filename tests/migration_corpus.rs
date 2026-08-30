//! Migration corpus — Phase 2 of the 0.11 plan (temp/0.11-sequencing.md).
//!
//! Step 3 is complete: the legacy APIs are deleted, and what remains here
//! is the permanent proof that the migration guide's replacements
//! reproduce 0.10 behavior (the R5/R6 migration proof). Every golden was
//! FROZEN from the legacy APIs' own output on 0.10.3, before deletion —
//! the `render_scanline*` family, the `LayoutIRArena` buffer family, and
//! the sized-node methods all asserted equal to these exact bytes in the
//! pre-removal revision of this file (see git history).
//!
//! Goldens live in tests/golden/migration-*.txt and pin the stage-graph
//! fixture byte-for-byte, ANSI escapes included. The sized-node goldens
//! are inline literals frozen the same way.

use ascii_dag::LayoutConfig;
use ascii_dag::graph::Graph;
use ascii_dag::render::colors::Palette;
use ascii_dag::render::engine::RenderOptions;

/// Labeled edge entering a one-node cluster — small enough that golden
/// diffs stay readable, rich enough to exercise labels and borders.
fn stage_graph() -> Graph<'static> {
    let mut g = Graph::new();
    g.add_node(1usize, "Start");
    g.add_node(2usize, "Middle");
    g.add_node(3usize, "End");
    g.add_edge(1usize, 2usize, Some("go"));
    g.add_edge(2usize, 3usize, None);
    let sg = g.add_subgraph("Stage");
    g.put_nodes(&[2]).inside(sg).unwrap();
    g
}

const GOLDEN_PLAIN: &str = include_str!("golden/migration-plain.txt");
const GOLDEN_COLORED: &str = include_str!("golden/migration-colored.txt");
const GOLDEN_COLORED_LEGEND: &str = include_str!("golden/migration-colored-legend.txt");

fn colored_no_legend() -> RenderOptions {
    let mut o = RenderOptions::colored(Palette::Ansi);
    o.legend = false;
    o
}

/// Replacements for the removed `render_scanline*` family
/// (`render_string` / `render_with` per the 0.10 deprecation notes;
/// colored wrappers mapped with `legend = false`) reproduce the frozen
/// legacy bytes.
#[test]
fn render_string_reproduces_legacy_scanline_output() {
    let ir = stage_graph().compute_layout();
    assert_eq!(ir.render_string(&RenderOptions::plain()), GOLDEN_PLAIN);
    assert_eq!(ir.render_string(&colored_no_legend()), GOLDEN_COLORED);
    assert_eq!(
        ir.render_string(&RenderOptions::colored(Palette::Ansi)),
        GOLDEN_COLORED_LEGEND
    );

    // The streaming form matches the owned form.
    let mut streamed = String::new();
    ir.render_with(&RenderOptions::plain(), &mut streamed)
        .unwrap();
    assert_eq!(streamed, GOLDEN_PLAIN);
}

/// Replacement for the removed `add_node_with_size`/`add_node_with_width`
/// (`CustomNode` per the migration guide). With this painter — the
/// guide's recipe for callers wanting 0.10 output unchanged — the
/// replacement reproduces the legacy sized-Simple rendering
/// byte-for-byte (literals frozen from the deprecated methods' output
/// on 0.10.3).
fn legacy_sized_look(
    region: &mut ascii_dag::render::engine::NodeRegion<'_, '_>,
    ctx: ascii_dag::render::engine::NodePaintCtx<'_>,
) {
    region.set(0, 0, '[');
    region.write_str(1, 0, ctx.label);
    for x in 1 + ctx.label.chars().count()..region.width().saturating_sub(1) {
        region.set(x, 0, ' ');
    }
    if region.width() > 0 {
        region.set(region.width() - 1, 0, ']');
    }
}

#[test]
fn custom_node_reproduces_legacy_sized_output() {
    use ascii_dag::CustomNode;

    // Frozen from: add_node_with_size(2, "Wide", 20, 3) under 0.10.3.
    const SIZED_GOLDEN: &str =
        "         [Src]\n           └┐\n            ↓\n  [Wide              ]\n\n\n\n\n";
    // Frozen from: add_node_with_width(1, "W", 12) under 0.10.3.
    const WIDTH_GOLDEN: &str = "  [W         ]\n\n\n";

    let mut g = Graph::new();
    g.add_node(1usize, "Src");
    g.add_node(
        2usize,
        CustomNode {
            label: "Wide",
            width: 20,
            height: 3,
            painter: Some(legacy_sized_look),
            payload: "",
        },
    );
    g.add_edge(1usize, 2usize, None);
    assert_eq!(
        g.compute_layout().render_string(&RenderOptions::plain()),
        SIZED_GOLDEN
    );

    let mut g = Graph::new();
    g.add_node(
        1usize,
        CustomNode {
            label: "W",
            width: 12,
            height: 1,
            painter: Some(legacy_sized_look),
            payload: "",
        },
    );
    assert_eq!(
        g.compute_layout().render_string(&RenderOptions::plain()),
        WIDTH_GOLDEN
    );
}

/// Replacement for the removed `SugiyamaConfig` surface: the
/// `LayoutConfig` presets drive the same pipeline. `quality()` here is
/// pinned against the plain golden's `standard()` sibling to prove the
/// preset path works; the exact `SugiyamaConfig → LayoutConfig` value
/// mapping was asserted equal in the pre-removal revision.
#[test]
fn layout_config_presets_replace_sugiyama_config() {
    let g = stage_graph();
    // standard() is the default pipeline — identical to compute_layout().
    assert_eq!(
        g.compute_layout_with_config(&LayoutConfig::standard())
            .render_string(&RenderOptions::plain()),
        g.compute_layout().render_string(&RenderOptions::plain())
    );
    // quality() lays out this fixture identically (small graph, no
    // crossings to reduce further) — and must at minimum stay valid.
    assert_eq!(
        g.compute_layout_with_config(&LayoutConfig::quality())
            .render_string(&RenderOptions::plain()),
        GOLDEN_PLAIN
    );
}

/// Replacement for the removed `LayoutIRArena` buffer family
/// (`render_to_bytes` + the split estimators per the deprecation
/// notes) reproduces the frozen legacy bytes.
#[cfg(feature = "arena")]
mod arena {
    use super::*;
    use ascii_dag::graph::arena::Arena;

    #[test]
    fn render_to_bytes_reproduces_legacy_buffer_output() {
        let g = stage_graph();
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

        for (options, golden) in [
            (RenderOptions::plain(), GOLDEN_PLAIN),
            (colored_no_legend(), GOLDEN_COLORED),
            (RenderOptions::colored(Palette::Ansi), GOLDEN_COLORED_LEGEND),
        ] {
            let mut render_arena_buf = vec![0u8; ir.estimate_render_arena_size(&options)];
            let render_arena = Arena::new(&mut render_arena_buf);
            let mut out = vec![0u8; ir.estimate_render_output_size(&options)];
            let n = ir
                .render_to_bytes(&options, &render_arena, &mut out)
                .expect("render");
            assert_eq!(core::str::from_utf8(&out[..n]).unwrap(), golden);
        }
    }
}
