//! Engine invariants suite.
//!
//! Until RW8 this file was the dual-run byte-parity harness: every
//! corpus graph rendered through the legacy renderers AND the engine,
//! and the bytes had to match (R6.2). That mission completed — the
//! legacy renderers are deleted, the engine is the output of record —
//! and what remains are the invariants that outlive the migration:
//! cross-backend self-parity, banding, BottomUp semantics, the
//! zero-allocation surface, styling, the golden snapshot, and byte
//! compatibility of the deprecated wrapper shims.

#![cfg(all(test, feature = "std", feature = "arena"))]

use super::config::RenderOptions;
use super::{render_colored, render_plain};
use crate::algorithms::sugiyama::config::LayoutConfig;
use crate::graph::Graph;
use crate::graph::arena::Arena;
use crate::render::colors::Palette;
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

/// A corpus entry: fixture name + builder.
type CorpusEntry = (&'static str, fn() -> Graph<'static>);

/// Engine render of `g` through the CSR/arena pipeline.
fn csr_engine(g: &Graph<'_>, options: &RenderOptions) -> String {
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
    if matches!(options.color_mode, crate::render::engine::ColorMode::None) {
        render_plain(&ir, options)
    } else {
        render_colored(&ir, options)
    }
}

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

// ── Engine invariants ────────────────────────────────────────────────────

#[test]
fn engine_colored_self_parity_across_backends() {
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
    let options = RenderOptions::colored(Palette::Ansi);
    for (tag, build) in corpus {
        let g = build();
        let ir = g.compute_layout();
        let heap_out = render_colored(&ir, &options);

        let config = LayoutConfig::standard();
        let mut csr_buf = vec![0u8; g.estimate_csr_arena_size() * 2];
        let mut csr_arena = Arena::new(&mut csr_buf);
        let csr = g.to_csr(&mut csr_arena).expect("CSR conversion");
        let size = (g.estimate_layout_arena_size() * 2).max(256 * 1024);
        let mut temp_buf = vec![0u8; size];
        let mut out_buf = vec![0u8; size];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_arena = Arena::new(&mut out_buf);
        let csr_ir = csr
            .compute_layout_arena(&config, &mut temp_arena, &mut out_arena)
            .expect("CSR layout");
        let csr_out = render_colored(&csr_ir, &options);
        assert_same(&format!("{tag} (colored heap vs csr)"), &heap_out, &csr_out);
    }
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
        let csr_out = csr_engine(&g, &RenderOptions::plain());
        assert_same(
            &format!("{tag} (engine heap vs engine csr)"),
            &heap_out,
            &csr_out,
        );
    }
}

/// Canonical spot checks for the ruled divergence classes: the corrected
/// glyphs must actually appear in the engine's output.
#[cfg(feature = "layout-vertical")]
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
    let collide = render_plain(
        &colliding_labels().compute_layout(),
        &RenderOptions::plain(),
    );
    assert_eq!(collide.matches('"').count() % 2, 0, "{collide}");
}

/// The hero graph across both backends — plain and colored, byte
/// identical. Hero was excluded from cross-backend suites for months
/// and silently diverged (a projection-pass parent-gap mismatch);
/// never again.
#[test]
fn hero_self_parity_across_backends() {
    let ir = hero_graph().compute_layout();
    let plain = render_plain(&ir, &RenderOptions::plain());
    assert_same(
        "hero (engine heap vs engine csr)",
        &plain,
        &csr_engine(&hero_graph(), &RenderOptions::plain()),
    );
    let colored = RenderOptions::colored(Palette::Ansi);
    assert_same(
        "hero colored (heap vs csr)",
        &render_colored(&ir, &colored),
        &csr_engine(&hero_graph(), &colored),
    );
}

/// D4 (temp/08): the legend works in PLAIN mode — labels that fail
/// geometric placement are listed as self-keying `from → to: label`
/// lines, no color required, no escapes emitted. Off by default
/// (`RenderOptions::plain()` keeps `legend = false`), so default
/// plain output is unchanged.
#[test]
fn plain_legend_lists_unplaced_labels() {
    let g = colliding_labels();
    let ir = g.compute_layout();
    let mut options = RenderOptions::plain();
    options.legend = true;
    let mut out = String::new();
    ir.render_with(&options, &mut out).expect("render");
    assert!(
        !out.contains('\x1b'),
        "no escapes in a plain legend:\n{out}"
    );
    assert!(
        out.contains(" → "),
        "legend lines present (colliding_labels always overflows):\n{out}"
    );
    // And the default stays legend-free (a 0.11 candidate to flip —
    // for now, the `warnings` feature is the silent-drop guard).
    let mut plain = String::new();
    ir.render_with(&RenderOptions::plain(), &mut plain)
        .expect("render");
    assert!(!plain.contains(" → "), "no legend by default:\n{plain}");

    // Slices review [P1]: the no-alloc surface must agree — one
    // assertion covering both the emission gate and the estimator.
    use crate::render::engine::{
        estimate_render_arena_size, estimate_render_output_size, render_to_bytes,
    };
    let mut arena_buf = vec![0u8; estimate_render_arena_size(&ir, &options)];
    let arena = Arena::new(&mut arena_buf);
    let mut bytes = vec![0u8; estimate_render_output_size(&ir, &options)];
    let written = render_to_bytes(&ir, &options, &arena, &mut bytes)
        .expect("plain legend fits its estimated output buffer");
    assert_eq!(
        core::str::from_utf8(&bytes[..written]).unwrap(),
        out,
        "byte surface matches the String surface with a plain legend"
    );
}

/// The hero example laid out sideways, against its goldens — the
/// LR/RL counterparts of `hero_matches_golden`. Regenerate with:
///   cargo run --example hero -- --lr > tests/golden/hero-lr.txt
///   cargo run --example hero -- --rl > tests/golden/hero-rl.txt
#[cfg(feature = "layout-horizontal")]
#[test]
fn hero_horizontal_matches_goldens() {
    for (dir, golden) in [
        (
            crate::graph::Direction::LeftRight,
            include_str!("../../../tests/golden/hero-lr.txt"),
        ),
        (
            crate::graph::Direction::RightLeft,
            include_str!("../../../tests/golden/hero-rl.txt"),
        ),
    ] {
        let mut g = hero_graph();
        g.set_direction(dir);
        let ir = g.compute_layout();
        // The visual comparison trims trailing blank rows, so pin the
        // CANVAS too: `Axis::cross_margin` exists to keep the
        // horizontal canvas tight, and a regression that puts the
        // blank rows back would otherwise slip through a trimmed
        // golden (P5 review).
        // 81×24 before the chain-lane pass; 77×24 after it (temp/09 P3
        // removed trace's detour, which was what held the extra columns).
        assert_eq!(
            (ir.width(), ir.height()),
            (77, 24),
            "hero {dir:?} canvas stays tight"
        );
        let engine = render_plain(&ir, &RenderOptions::plain());
        assert_same(
            &format!("hero {dir:?} (golden)"),
            golden.trim_end(),
            engine.trim_end(),
        );
    }
}

/// The hero example against the golden snapshot (regenerated at RW8
/// when the engine became the output of record).
#[cfg(feature = "layout-vertical")]
#[test]
fn hero_matches_golden() {
    let engine = render_plain(&hero_graph().compute_layout(), &RenderOptions::plain());
    let golden = include_str!("../../../tests/golden/hero.txt");
    assert_same("hero (golden)", golden.trim_end(), engine.trim_end());
}

// The shared hero fixture refers to the crate by its external name.
use crate as ascii_dag;
include!("../../../examples/shared/hero_graph.rs");
#[cfg(all(feature = "layout-vertical", feature = "layout-horizontal"))]
// Clustered stress tiers — the corpus's deep-nesting cases (quality scorer).
include!("../../../examples/shared/stress_graphs.rs");

// ── BottomUp rendering (RW5) ─────────────────────────────────────────────
//
// The first direction-aware output: the geometry-driven primitives
// paint the physical BT IR with no direction-specific code paths. No
// legacy comparisons exist here (the legacy renderers never painted
// BT); the invariants are cross-backend byte-identity, D4 semantics,
// and render-vs-IR physical consistency.

#[cfg(feature = "layout-vertical")]
mod bt {
    use super::*;
    use crate::graph::Direction;

    fn bt_heap_ir(g: fn() -> Graph<'static>) -> crate::ir::LayoutIR<'static> {
        let mut graph = g();
        graph.set_direction(Direction::BottomUp);
        graph.compute_layout()
    }

    fn bt_csr_render(g: &Graph<'_>, options: &RenderOptions) -> String {
        let mut config = LayoutConfig::standard();
        config.direction = Direction::BottomUp;
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
        render_plain(&ir, options)
    }

    /// BT output must be byte-identical from both IRs, plain and colored.
    #[test]
    fn bt_engine_self_parity_across_backends() {
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
            let heap_ir = bt_heap_ir(build);
            let heap_out = render_plain(&heap_ir, &RenderOptions::plain());
            let csr_out = bt_csr_render(&build(), &RenderOptions::plain());
            assert_same(&format!("{tag} (BT heap vs csr)"), &heap_out, &csr_out);

            let mut colored = RenderOptions::colored(Palette::Ansi);
            colored.legend = false;
            let heap_col = render_colored(&heap_ir, &colored);
            let mut config = LayoutConfig::standard();
            config.direction = Direction::BottomUp;
            let g = build();
            let mut csr_buf = vec![0u8; g.estimate_csr_arena_size() * 2];
            let mut csr_arena = Arena::new(&mut csr_buf);
            let csr = g.to_csr(&mut csr_arena).expect("CSR conversion");
            let size = (g.estimate_layout_arena_size() * 2).max(256 * 1024);
            let mut temp_buf = vec![0u8; size];
            let mut out_buf = vec![0u8; size];
            let mut temp_arena = Arena::new(&mut temp_buf);
            let mut out_arena = Arena::new(&mut out_buf);
            let csr_ir = csr
                .compute_layout_arena(&config, &mut temp_arena, &mut out_arena)
                .expect("CSR layout");
            let csr_col = render_colored(&csr_ir, &colored);
            assert_same(
                &format!("{tag} (BT colored heap vs csr)"),
                &heap_col,
                &csr_col,
            );
        }
    }

    /// D4 semantics: growth flips, content does not. Sources render at
    /// the bottom, forward arrows point up, the box label stays at the
    /// physical top of its box.
    #[test]
    fn bt_stage_semantics() {
        let ir = bt_heap_ir(stage);
        let out = render_plain(&ir, &RenderOptions::plain());
        let lines: Vec<&str> = out.lines().collect();

        let row_of = |needle: &str| lines.iter().position(|l| l.contains(needle)).unwrap();
        assert!(
            row_of("[Start]") > row_of("[End]"),
            "BT: source must render below target:\n{out}"
        );
        assert!(out.contains('↑'), "forward arrows point up:\n{out}");
        assert!(!out.contains('↓'), "no downward arrows in BT:\n{out}");

        // Box label at the physical top of the box (content atomicity).
        let sg = &ir.subgraphs()[0];
        assert_eq!(
            row_of("Stage"),
            sg.y + 1,
            "box label just below top border:\n{out}"
        );
        assert!(lines[sg.y].contains('╔'), "top border row:\n{out}");

        // The edge label renders too (its row is IR-physical).
        assert!(out.contains("\"go\""), "edge label present:\n{out}");
    }

    /// Reversed (back) edges mirror: their dashed arrow points down,
    /// painted above the layout-source.
    #[test]
    fn bt_back_edges_semantics() {
        let out = render_plain(&bt_heap_ir(back_edges), &RenderOptions::plain());
        assert!(
            out.contains('⇣'),
            "reversed arrow points down in BT:\n{out}"
        );
        assert!(!out.contains('⇡'), "no upward dashed arrows in BT:\n{out}");
        assert!(out.contains('↑'), "forward arrows point up:\n{out}");
    }

    /// Render-vs-IR physical consistency: every real node's label paints
    /// exactly on its IR row (the physical-coordinate contract, S3).
    #[test]
    fn bt_nodes_render_on_their_ir_rows() {
        let corpus: [CorpusEntry; 5] = [
            ("chain", chain),
            ("fan", fan),
            ("stage", stage),
            ("skip", skip),
            ("two_cycle", two_cycle),
        ];
        for (tag, build) in corpus {
            let ir = bt_heap_ir(build);
            let out = render_plain(&ir, &RenderOptions::plain());
            let lines: Vec<&str> = out.lines().collect();
            for node in ir.nodes() {
                if matches!(node.kind, crate::ir::NodeKind::Dummy) {
                    continue;
                }
                let row = lines.get(node.y).copied().unwrap_or("");
                assert!(
                    row.contains(node.label),
                    "{tag}: node '{}' missing from its IR row {}:\n{out}",
                    node.label,
                    node.y,
                );
            }
        }
    }

    /// The hero graph, upside down — the full-feature BT smoke test.
    #[test]
    fn bt_hero_renders() {
        let ir = bt_heap_ir(hero_graph);
        let out = render_plain(&ir, &RenderOptions::plain());
        assert!(out.contains('↑') && !out.contains('↓'), "{out}");
        // All box labels present.
        for label in ["Services", "Data", "Async"] {
            assert!(out.contains(label), "box label {label} present:\n{out}");
        }
        let lines: Vec<&str> = out.lines().collect();
        let row_of = |needle: &str| lines.iter().position(|l| l.contains(needle)).unwrap();
        assert!(
            row_of("[Client]") > row_of("[Dash]"),
            "hero BT: Client at bottom:\n{out}"
        );
    }
}

// ── Banding (RW6) ────────────────────────────────────────────────────────
//
// The band partition is invisible in the output by construction:
// boundary-spanning elements replay in every band they intersect and
// the canvas clips. These tests sweep the cap through degenerate and
// typical values and demand byte-identity with the single-band render.

#[cfg(feature = "layout-vertical")]
mod bands {
    use super::*;
    use crate::graph::Direction;

    const CAPS: [usize; 6] = [1, 2, 3, 5, 7, 1000];

    fn with_cap(mut options: RenderOptions, cap: usize) -> RenderOptions {
        options.band_rows_cap = cap;
        options
    }

    /// Plain and colored output must not depend on the cap, TD and BT.
    #[test]
    fn band_cap_never_changes_output() {
        let corpus: [CorpusEntry; 6] = [
            ("chain", chain),
            ("fan", fan),
            ("stage", stage),
            ("skip", skip),
            ("back_edges", back_edges),
            ("nested_boxes", nested_boxes),
        ];
        for (tag, build) in corpus {
            for direction in [Direction::TopDown, Direction::BottomUp] {
                let mut g = build();
                g.set_direction(direction);
                let ir = g.compute_layout();
                let plain_ref = render_plain(&ir, &RenderOptions::plain());
                let colored_ref = render_colored(&ir, &RenderOptions::colored(Palette::Ansi));
                for cap in CAPS {
                    let plain = render_plain(&ir, &with_cap(RenderOptions::plain(), cap));
                    assert_same(
                        &format!("{tag} {direction:?} plain cap={cap}"),
                        &plain_ref,
                        &plain,
                    );
                    let colored =
                        render_colored(&ir, &with_cap(RenderOptions::colored(Palette::Ansi), cap));
                    assert_same(
                        &format!("{tag} {direction:?} colored cap={cap}"),
                        &colored_ref,
                        &colored,
                    );
                }
            }
        }
    }

    /// Small caps really do split the render into multiple bands.
    #[test]
    fn small_caps_produce_multiple_bands() {
        let ir = hero_graph().compute_layout();
        let plan = crate::render::engine::plan::RenderPlan::build(
            &ir,
            &with_cap(RenderOptions::plain(), 5),
        );
        assert!(plan.band_count() > 1, "hero at cap 5 must band");
        // Bands tile the height exactly, in order, no gaps.
        let mut next = 0usize;
        for &(y0, rows) in plan.band_ranges() {
            assert_eq!(y0, next, "bands must be contiguous");
            assert!(rows >= 1);
            next = y0 + rows;
        }
        assert_eq!(next, plan.height(), "bands must cover the full height");
        // Level-aligned: with a workable cap, every boundary is a node row.
        let plan64 = crate::render::engine::plan::RenderPlan::build(
            &ir,
            &with_cap(RenderOptions::plain(), 10),
        );
        let tops: alloc::vec::Vec<usize> = ir.nodes().iter().map(|n| n.y).collect();
        let mut prev = 0usize;
        for &(y0, _) in plan64.band_ranges().iter().skip(1) {
            // A boundary is either level-aligned or a hard cut forced by
            // a level chunk taller than the cap (no top in the window).
            assert!(
                tops.contains(&y0) || tops.iter().all(|&t| t <= prev || t > y0),
                "cap-10 boundary {y0} is neither a level top nor a forced cut"
            );
            prev = y0;
        }
    }

    /// Boxed corpus graphs across both IRs at a tiny cap — banding
    /// composes with backend self-parity (hero included, now that the
    /// projection parent-gap divergence is fixed).
    #[test]
    fn banded_self_parity_across_backends() {
        for build in [nested_boxes as fn() -> Graph<'static>, hero_graph] {
            let heap_out = render_plain(
                &build().compute_layout(),
                &with_cap(RenderOptions::plain(), 3),
            );
            let mut opts = RenderOptions::plain();
            opts.band_rows_cap = 3;
            let csr_out = csr_engine(&build(), &opts);
            assert_same("banded cap=3 (heap vs csr)", &heap_out, &csr_out);
        }
    }

    #[test]
    fn banded_self_parity_nested_boxes_explicit_buffers() {
        let heap_out = render_plain(
            &nested_boxes().compute_layout(),
            &with_cap(RenderOptions::plain(), 3),
        );
        let g = nested_boxes();
        let mut csr_buf = vec![0u8; g.estimate_csr_arena_size() * 2];
        let mut csr_arena = Arena::new(&mut csr_buf);
        let csr = g.to_csr(&mut csr_arena).expect("CSR conversion");
        let size = (g.estimate_layout_arena_size() * 2).max(256 * 1024);
        let mut temp_buf = vec![0u8; size];
        let mut out_buf = vec![0u8; size];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_arena = Arena::new(&mut out_buf);
        let config = LayoutConfig::standard();
        let ir = csr
            .compute_layout_arena(&config, &mut temp_arena, &mut out_arena)
            .expect("CSR layout");
        let csr_out = render_plain(&ir, &with_cap(RenderOptions::plain(), 3));
        assert_same(
            "nested_boxes banded cap=3 (heap vs csr)",
            &heap_out,
            &csr_out,
        );
    }
}

// ── Zero-allocation byte surface (RW6, N2/R4.3) ──────────────────────────

#[cfg(feature = "layout-vertical")]
mod no_alloc {
    use super::*;
    use crate::GraphError;
    use crate::graph::Direction;
    use crate::render::engine::color::ColorMode;
    use crate::render::engine::{
        estimate_render_arena_size, estimate_render_output_size, render_to_bytes,
    };

    /// The byte surface must match the String surface exactly — plain
    /// and colored, TD and BT, banded and not — using estimate-sized
    /// buffers (which doubles as the estimate-sufficiency proof).
    #[test]
    fn bytes_match_string_surface() {
        let corpus: [CorpusEntry; 5] = [
            ("stage", stage),
            ("fan", fan),
            ("back_edges", back_edges),
            ("nested_boxes", nested_boxes),
            ("self_loop", self_loop),
        ];
        for (tag, build) in corpus {
            for direction in [Direction::TopDown, Direction::BottomUp] {
                for cap in [3usize, 64] {
                    let mut plain = RenderOptions::plain();
                    plain.band_rows_cap = cap;
                    let mut colored = RenderOptions::colored(Palette::Ansi);
                    colored.band_rows_cap = cap;
                    for options in [plain, colored] {
                        let mut g = build();
                        g.set_direction(direction);
                        let ir = g.compute_layout();
                        let want = if matches!(options.color_mode, ColorMode::None) {
                            render_plain(&ir, &options)
                        } else {
                            render_colored(&ir, &options)
                        };
                        let arena_size = estimate_render_arena_size(&ir, &options);
                        let out_size = estimate_render_output_size(&ir, &options);
                        let mut backing = vec![0u8; arena_size];
                        let arena = Arena::new(&mut backing);
                        let mut out = vec![0u8; out_size];
                        let written = render_to_bytes(&ir, &options, &arena, &mut out)
                            .unwrap_or_else(|e| panic!("{tag} {direction:?} cap={cap}: {e}"));
                        let got = core::str::from_utf8(&out[..written]).unwrap();
                        assert_same(
                            &format!("{tag} {direction:?} cap={cap} (bytes vs string)"),
                            &want,
                            got,
                        );
                    }
                }
            }
        }
    }

    /// Growing the arena from nothing hits Plan exhaustion first, then
    /// Canvas, then succeeds — never a panic, always the right domain.
    #[test]
    fn undersized_arena_walks_the_error_ladder() {
        let ir = hero_graph().compute_layout();
        let options = RenderOptions::colored(Palette::Ansi);
        let full = estimate_render_arena_size(&ir, &options);
        let mut out = vec![0u8; estimate_render_output_size(&ir, &options)];

        let mut seen_plan = false;
        let mut seen_canvas = false;
        let mut succeeded = false;
        let mut size = 16usize;
        while size <= full {
            let mut backing = vec![0u8; size];
            let arena = Arena::new(&mut backing);
            match render_to_bytes(&ir, &options, &arena, &mut out) {
                Err(GraphError::RenderPlanOom) => {
                    assert!(!seen_canvas && !succeeded, "Plan must fail before Canvas");
                    seen_plan = true;
                }
                Err(GraphError::RenderCanvasTooSmall { needed, got }) => {
                    assert!(needed > got, "needed {needed} vs got {got}");
                    seen_canvas = true;
                }
                Err(e) => panic!("unexpected error at {size}B: {e}"),
                Ok(_) => {
                    succeeded = true;
                    break;
                }
            }
            size *= 2;
        }
        // The final doubling may overshoot `full`; the estimate itself
        // must succeed regardless.
        let mut backing = vec![0u8; full];
        let arena = Arena::new(&mut backing);
        assert!(render_to_bytes(&ir, &options, &arena, &mut out).is_ok());
        assert!(seen_plan, "never saw E.Render.Plan.026");
        assert!(seen_canvas || succeeded, "never got past plan building");
    }

    /// A too-small output buffer reports the Sink domain, not a panic.
    #[test]
    fn undersized_output_reports_sink_exhaustion() {
        let ir = hero_graph().compute_layout();
        let options = RenderOptions::plain();
        let mut backing = vec![0u8; estimate_render_arena_size(&ir, &options)];
        let arena = Arena::new(&mut backing);
        let mut out = [0u8; 8];
        assert!(matches!(
            render_to_bytes(&ir, &options, &arena, &mut out),
            Err(GraphError::RenderOutputTooSmall)
        ));
    }
}

// ── Styling surface v1 (RW7, acceptance #6) ──────────────────────────────
//
// End-to-end: override one knob through a style fn, render through the
// public option surface, assert the output changed exactly as the knob
// promises — on both IRs where geometry allows. Defaults changing
// nothing is enforced by the whole harness above (every parity test
// runs the default style fns).

#[cfg(feature = "layout-vertical")]
mod styles {
    use super::*;
    use crate::graph::Direction;
    use crate::render::engine::CellColor;
    use crate::render::engine::style::{
        EdgeLabelStyle, EdgeStyle, EdgeStyleCtx, LabelPosition, MarkerShape, SubgraphBorder,
        SubgraphStyle, SubgraphStyleCtx,
    };

    // Style fns are plain `fn` items — the no_std-safe callback shape.
    fn no_arrowheads(_: EdgeStyleCtx<'_>) -> EdgeStyle {
        EdgeStyle {
            marker_end: MarkerShape::None,
            ..EdgeStyle::default()
        }
    }
    fn double_headed(_: EdgeStyleCtx<'_>) -> EdgeStyle {
        EdgeStyle {
            marker_start: MarkerShape::Arrow,
            ..EdgeStyle::default()
        }
    }
    fn light_boxes(_: SubgraphStyleCtx<'_>) -> SubgraphStyle {
        SubgraphStyle {
            border: SubgraphBorder::Light,
            ..SubgraphStyle::default()
        }
    }
    fn dashed_boxes(_: SubgraphStyleCtx<'_>) -> SubgraphStyle {
        SubgraphStyle {
            border: SubgraphBorder::Dashed,
            ..SubgraphStyle::default()
        }
    }
    fn invisible_boxes(_: SubgraphStyleCtx<'_>) -> SubgraphStyle {
        SubgraphStyle {
            border: SubgraphBorder::None,
            ..SubgraphStyle::default()
        }
    }
    fn green_boxes(_: SubgraphStyleCtx<'_>) -> SubgraphStyle {
        SubgraphStyle {
            color: CellColor::ansi256(42),
            ..SubgraphStyle::default()
        }
    }
    fn bottom_labels(_: SubgraphStyleCtx<'_>) -> SubgraphStyle {
        SubgraphStyle {
            label_pos: LabelPosition::InsideBottom,
            ..SubgraphStyle::default()
        }
    }
    fn magenta_labels(_: EdgeStyleCtx<'_>) -> EdgeLabelStyle {
        EdgeLabelStyle {
            color: CellColor::ansi256(201),
            ..EdgeLabelStyle::default()
        }
    }

    // Presets stay const-constructible (R3.1).
    const _ASCII: RenderOptions = RenderOptions::ascii();
    const _ASCII_COLORED: RenderOptions = RenderOptions::ascii_colored(Palette::Ansi);

    #[test]
    fn marker_end_none_suppresses_every_arrowhead() {
        for direction in [Direction::TopDown, Direction::BottomUp] {
            let mut g = back_edges();
            g.set_direction(direction);
            let ir = g.compute_layout();
            let mut options = RenderOptions::plain();
            options.edge_style_fn = no_arrowheads;
            let out = render_plain(&ir, &options);
            for arrow in ['\u{2193}', '\u{2191}', '\u{21E1}', '\u{21E3}'] {
                assert!(
                    !out.contains(arrow),
                    "{direction:?}: {arrow} should be suppressed:\n{out}"
                );
            }
            // The stroke replaces the marker — columns stay connected.
            assert!(out.contains('\u{2502}'), "verticals remain:\n{out}");
        }
    }

    #[test]
    fn marker_start_arrow_gives_double_heads() {
        let ir = chain().compute_layout();
        let mut options = RenderOptions::plain();
        options.edge_style_fn = double_headed;
        let out = render_plain(&ir, &options);
        // TD forward edges gain an up-arrow tail beneath the source.
        assert!(out.contains('\u{2191}'), "tail arrowheads appear:\n{out}");
        assert!(out.contains('\u{2193}'), "head arrowheads remain:\n{out}");
    }

    #[test]
    fn subgraph_border_styles_restyle_the_box() {
        let ir = nested_boxes().compute_layout();

        let mut light = RenderOptions::plain();
        light.subgraph_style_fn = light_boxes;
        let out = render_plain(&ir, &light);
        assert!(out.contains('\u{250c}'), "light corner:\n{out}");
        assert!(!out.contains('\u{2554}'), "no double corner:\n{out}");

        let mut dashed = RenderOptions::plain();
        dashed.subgraph_style_fn = dashed_boxes;
        let out = render_plain(&ir, &dashed);
        assert!(out.contains('\u{2508}'), "dashed horizontal:\n{out}");
        assert!(!out.contains('\u{2550}'), "no double horizontal:\n{out}");

        let mut none = RenderOptions::plain();
        none.subgraph_style_fn = invisible_boxes;
        let out = render_plain(&ir, &none);
        // Only double strokes are box ink here — light corners belong
        // to edge routing and must survive.
        for border in ['\u{2554}', '\u{2550}', '\u{2551}', '\u{255a}'] {
            assert!(!out.contains(border), "no box ink:\n{out}");
        }
        assert!(
            out.contains("Outer") && out.contains("Inner"),
            "labels stay:\n{out}"
        );
        assert!(out.contains("[Work]"), "content stays:\n{out}");
    }

    #[test]
    fn subgraph_and_label_colors_reach_the_escape_stream() {
        let ir = stage_graph_for_styles().compute_layout();
        let mut options = RenderOptions::colored(Palette::Ansi);
        options.subgraph_style_fn = green_boxes;
        options.edge_label_style_fn = magenta_labels;
        let out = render_colored(&ir, &options);
        assert!(
            out.contains("\x1b[38;5;42m"),
            "border color escapes:\n{out:?}"
        );
        assert!(
            out.contains("\x1b[38;5;201m"),
            "label color escapes:\n{out:?}"
        );
    }

    #[test]
    fn label_position_inside_bottom_moves_the_box_label() {
        let ir = stage_graph_for_styles().compute_layout();
        let mut options = RenderOptions::plain();
        options.subgraph_style_fn = bottom_labels;
        let out = render_plain(&ir, &options);
        let sg = &ir.subgraphs()[0];
        let lines: alloc::vec::Vec<&str> = out.lines().collect();
        let label_row = lines
            .iter()
            .position(|l| l.contains("Stage"))
            .expect("label present");
        assert_eq!(
            label_row,
            sg.y + sg.height - 2,
            "label at box bottom:\n{out}"
        );
    }

    #[test]
    fn ascii_presets_project_and_legend_follows_charset() {
        let ir = stage_graph_for_styles().compute_layout();
        let out = render_plain(&ir, &RenderOptions::ascii());
        assert!(out.contains('v') && !out.contains('\u{2193}'), "{out}");
        assert!(out.contains('+') && !out.contains('\u{2554}'), "{out}");

        // Legend arrow: '->' under Ascii, '→' under Unicode.
        let ir = colliding_labels().compute_layout();
        let unicode = render_colored(&ir, &RenderOptions::colored(Palette::Ansi));
        let ascii = render_colored(&ir, &RenderOptions::ascii_colored(Palette::Ansi));
        assert!(
            unicode.contains(" \u{2192} "),
            "unicode legend arrow:\n{unicode:?}"
        );
        assert!(ascii.contains(" -> "), "ascii legend arrow:\n{ascii:?}");
        assert!(
            !ascii.contains('\u{2192}'),
            "no unicode arrow in ascii legend:\n{ascii:?}"
        );
    }

    fn stage_graph_for_styles() -> Graph<'static> {
        stage()
    }

    fn red_edges(_: EdgeStyleCtx<'_>) -> EdgeStyle {
        EdgeStyle {
            color: CellColor::ansi256(196),
            ..EdgeStyle::default()
        }
    }

    /// Acceptance criterion #6 verbatim: a per-subgraph
    /// `LabelPosition::InsideBottom` and a per-edge color override
    /// render correctly — and byte-identically — from BOTH IRs, and the
    /// public plan queries answer over the styled render.
    #[test]
    fn acceptance_6_style_overrides_from_both_irs() {
        let mut options = RenderOptions::colored(Palette::Ansi);
        options.subgraph_style_fn = bottom_labels;
        options.edge_style_fn = red_edges;

        let heap_ir = stage().compute_layout();
        let heap_out = render_colored(&heap_ir, &options);
        assert!(
            heap_out.contains("\x1b[38;5;196m"),
            "edge override:\n{heap_out:?}"
        );
        let sg = &heap_ir.subgraphs()[0];
        let label_row = heap_out
            .lines()
            .position(|l| l.contains("Stage"))
            .expect("label present");
        assert_eq!(label_row, sg.y + sg.height - 2, "label at box bottom");

        let csr_out = csr_engine(&stage(), &options);
        assert_same("acceptance #6 (heap vs csr)", &heap_out, &csr_out);

        // Public plan queries over the same layout (criterion #7 spot).
        let plan = heap_ir.render_plan(&options);
        assert!(plan.width() > 0 && plan.height() > 0 && plan.band_count() >= 1);
        let start = heap_ir.node_by_id(1).unwrap();
        assert_eq!(
            heap_ir.hit_test(&plan, start.x + 1, start.y),
            crate::render::engine::HitResult::Node(1)
        );
    }
}

// ── Deprecated wrapper compatibility (Q7/R6.1) ───────────────────────────
//
// Until 0.11 the legacy entry points remain as engine-backed shims.
// Each must be byte-identical to its documented replacement, and the
// legacy sizing contract (`estimate_render_size` → buffers →
// `render_to_buffer`) must keep working end to end.

// ── Review fixes (RF1): estimates + hit-testing pinned ───────────────────

#[cfg(feature = "layout-vertical")]
mod review_fixes {
    use super::*;
    use crate::render::engine::CellColor;
    use crate::render::engine::HitResult;
    use crate::render::engine::style::{EdgeStyle, EdgeStyleCtx};

    /// Long Unicode endpoint + edge labels forced into the legend: an
    /// exactly estimate-sized output buffer must still suffice.
    #[test]
    fn legend_estimate_covers_long_unicode_labels() {
        let mut g = Graph::new();
        g.add_node(1, "Ünïcödé-Nödé-With-A-Very-Long-Näme-Indeed-Ø");
        g.add_node(2, "Ζεύς-Годзилла-ノード-with-more-than-32-bytes");
        g.add_node(3, "C");
        g.add_node(4, "D");
        // Colliding long labels force the legend.
        g.add_edge(1, 3, Some("ein-sehr-länges-Ünïcödé-label-囲碁-☂☃☄"));
        g.add_edge(2, 4, Some("another-very-long-label-Ω≈ç√∫-ラベル"));
        let ir = g.compute_layout();
        let options = RenderOptions::colored(Palette::Ansi);
        let mut arena_buf = vec![0u8; ir.estimate_render_arena_size(&options)];
        let arena = Arena::new(&mut arena_buf);
        let mut out = vec![0u8; ir.estimate_render_output_size(&options)];
        let n = ir
            .render_to_bytes(&options, &arena, &mut out)
            .expect("estimate-sized output buffer must fit the legend");
        let text = core::str::from_utf8(&out[..n]).unwrap();
        assert!(text.contains("Edge labels:"), "legend present:\n{text}");
        assert!(
            text.contains("Ünïcödé-Nödé"),
            "endpoint label in full:\n{text}"
        );
    }

    /// Property: hit-testing agrees with the painted canvas. Every cell
    /// showing an edge glyph hits something; every blank cell outside
    /// all boxes hits nothing.
    #[test]
    fn hit_test_agrees_with_painted_output() {
        const EDGE_GLYPHS: &str = "─│┈┊┌┐└┘├┤┬┴┼╪╫↓↑→←⇡⇣⇠⇢";
        let corpus: [CorpusEntry; 5] = [
            ("chain", chain),
            ("fan", fan),
            ("skip", skip),
            ("back_edges", back_edges),
            ("self_loop", self_loop),
        ];
        for (tag, build) in corpus {
            let ir = build().compute_layout();
            let options = RenderOptions::plain();
            let out = render_plain(&ir, &options);
            let plan = ir.render_plan(&options);
            for (y, line) in out.lines().enumerate() {
                for (x, ch) in line.chars().enumerate() {
                    let hit = ir.hit_test(&plan, x, y);
                    if EDGE_GLYPHS.contains(ch) {
                        assert_ne!(
                            hit,
                            HitResult::None,
                            "{tag}: painted edge glyph {ch:?} at ({x},{y}) must hit:\n{out}"
                        );
                    } else if ch == ' ' {
                        // Blank cells may only hit a box interior.
                        assert!(
                            matches!(hit, HitResult::None | HitResult::Subgraph(_)),
                            "{tag}: blank cell at ({x},{y}) hit {hit:?}:\n{out}"
                        );
                    }
                }
            }
        }
    }

    /// Hidden dummies never hit; shown dummies hit only their own cell.
    #[test]
    fn dummy_hits_follow_visibility() {
        let mut config = LayoutConfig::standard();
        config.include_dummy_nodes = true;
        let g = skip();
        let ir = g.compute_layout_with_config(&config);
        let dummy = ir
            .nodes()
            .iter()
            .find(|n| matches!(n.kind, crate::ir::NodeKind::Dummy))
            .expect("skip graph produces a dummy");

        let hidden = ir.render_plan(&RenderOptions::plain());
        assert!(
            !matches!(ir.hit_test(&hidden, dummy.x, dummy.y), HitResult::Node(id) if id == dummy.id),
            "hidden dummy must not hit"
        );

        let mut options = RenderOptions::plain();
        options.show_dummy_nodes = true;
        let shown = ir.render_plan(&options);
        assert_eq!(
            ir.hit_test(&shown, dummy.x, dummy.y),
            HitResult::Node(dummy.id),
            "shown dummy hits its marker cell"
        );
    }

    /// A borderless subgraph paints no box, so its interior hits nothing.
    #[test]
    fn borderless_subgraph_does_not_hit() {
        fn no_border(
            _: crate::render::engine::SubgraphStyleCtx<'_>,
        ) -> crate::render::engine::SubgraphStyle {
            crate::render::engine::SubgraphStyle {
                border: crate::render::engine::SubgraphBorder::None,
                ..Default::default()
            }
        }
        let ir = stage().compute_layout();
        let sg = &ir.subgraphs()[0];
        let mut options = RenderOptions::plain();
        options.subgraph_style_fn = no_border;
        let plan = ir.render_plan(&options);
        assert_eq!(
            ir.hit_test(&plan, sg.x, sg.y),
            HitResult::None,
            "borderless box corner is empty canvas"
        );
        // With the default border the same cell hits the box.
        let bordered = ir.render_plan(&RenderOptions::plain());
        assert_eq!(
            ir.hit_test(&bordered, sg.x, sg.y),
            HitResult::Subgraph(sg.id)
        );
    }

    /// TrueColor renders emit 24-bit escapes in the legend too — never
    /// quantized back to ANSI-256.
    #[test]
    fn truecolor_legend_uses_rgb_escapes() {
        fn rgb_edges(_: EdgeStyleCtx<'_>) -> EdgeStyle {
            EdgeStyle {
                color: CellColor::rgb(200, 100, 50),
                ..EdgeStyle::default()
            }
        }
        let ir = colliding_labels().compute_layout();
        let mut options = RenderOptions::colored(Palette::Ansi);
        options.color_mode = crate::render::engine::ColorMode::TrueColor;
        options.edge_style_fn = rgb_edges;
        let out = render_colored(&ir, &options);
        let legend = out.split("Edge labels:").nth(1).expect("legend present");
        assert!(
            legend.contains("\x1b[38;2;200;100;50m"),
            "legend uses 24-bit escapes:\n{legend:?}"
        );
        assert!(
            !legend.contains("\x1b[38;5;"),
            "no quantized escapes in a TrueColor legend:\n{legend:?}"
        );
    }

    /// Worst-case legend arithmetic: several TrueColor entries on a
    /// minimal canvas must fit an exactly estimate-sized buffer.
    #[test]
    fn truecolor_legend_estimate_holds_with_many_entries() {
        let mut g = Graph::new();
        for i in 1..=6usize {
            g.add_node(i, "N");
        }
        // Three colliding label pairs — several legend entries.
        g.add_edge(1, 3, Some("first-very-long-colliding-label"));
        g.add_edge(2, 4, Some("second-very-long-colliding-label"));
        g.add_edge(3, 5, Some("third-very-long-colliding-label"));
        g.add_edge(4, 6, Some("fourth-very-long-colliding-label"));
        let ir = g.compute_layout();
        let mut options = RenderOptions::colored(Palette::Ansi);
        options.color_mode = crate::render::engine::ColorMode::TrueColor;
        let mut arena_buf = vec![0u8; ir.estimate_render_arena_size(&options)];
        let arena = Arena::new(&mut arena_buf);
        let mut out = vec![0u8; ir.estimate_render_output_size(&options)];
        let n = ir
            .render_to_bytes(&options, &arena, &mut out)
            .expect("estimate-sized buffer fits a multi-entry TrueColor legend");
        let text = core::str::from_utf8(&out[..n]).unwrap();
        assert!(
            text.matches("\x1b[38;2;").count() >= 2,
            "multiple truecolor legend escapes:\n{text:?}"
        );
    }

    /// Hit-test and legend share one index convention: the IR-list index.
    #[test]
    fn edge_indices_are_ir_list_indices() {
        let ir = colliding_labels().compute_layout();
        let options = RenderOptions::colored(Palette::Ansi);
        let plan = ir.render_plan(&options);
        for &ei in plan.legend_entries() {
            assert!(ei < ir.edges().len(), "legend index {ei} is a list index");
        }
        // A hit on an edge's own vertical resolves to its list index.
        let e = &ir.edges()[0];
        let probe_y = e.from_y + 1;
        if let HitResult::Edge(idx) = ir.hit_test(&plan, e.from_x, probe_y) {
            assert!(idx < ir.edges().len());
        }
    }
}

// ── Node painters (fill the reserved area) ───────────────────────────────

#[cfg(feature = "layout-vertical")]
mod node_painters {
    use super::*;
    use crate::render::engine::{BoxedNode, CustomNode, NodePaintCtx, NodeRegion};

    /// Declared content end-to-end: `[Client]` simple, boxed Server
    /// and Cache — no style steering anywhere; the declaration is the
    /// only source of what a node is.
    fn boxed_graph() -> Graph<'static> {
        let mut g = Graph::new();
        g.add_node(1, BoxedNode("Server"));
        g.add_node(2, BoxedNode("Cache"));
        g.add_node(3, "Client");
        g.add_edge(3, 1, None);
        g.add_edge(1, 2, None);
        g
    }

    fn card_painter(region: &mut NodeRegion<'_, '_>, ctx: NodePaintCtx<'_>) {
        // Header, separator, payload body — a zigraph-style card.
        region.write_str(0, 0, "#");
        region.write_str(2, 0, ctx.label);
        for x in 0..region.width() {
            region.set(x, 1, '=');
        }
        for (i, line) in ctx.payload.lines().enumerate() {
            region.write_str(0, 2 + i, line);
        }
        // Escapes must be silent no-ops.
        region.set(region.width() + 10, 0, 'X');
        region.set(0, region.height() + 10, 'X');
        region.write_str(region.width() - 1, 0, "OVERFLOWING");
    }

    /// Server is a declared card (painter + payload), Cache a boxed
    /// declaration, Client plain sugar.
    fn card_graph() -> Graph<'static> {
        let mut g = Graph::new();
        g.add_node(
            1,
            CustomNode {
                label: "Server",
                width: 14,
                height: 5,
                painter: Some(card_painter),
                payload: "id: one",
            },
        );
        g.add_node(2, BoxedNode("Cache"));
        g.add_node(3, "Client");
        g.add_edge(3, 1, None);
        g.add_edge(1, 2, None);
        g
    }

    /// Declared boxed nodes render full-extent boxes byte-identically
    /// from both IRs; a simple declaration stays `[label]`.
    #[test]
    fn declared_boxed_nodes_fill_their_area_across_backends() {
        let options = RenderOptions::plain();
        let heap_out = render_plain(&boxed_graph().compute_layout(), &options);
        for glyph in ['\u{250c}', '\u{2510}', '\u{2514}', '\u{2518}'] {
            assert!(heap_out.contains(glyph), "box corner {glyph}:\n{heap_out}");
        }
        assert!(heap_out.contains("Server"), "label inside:\n{heap_out}");
        assert!(
            heap_out.contains("[Client]"),
            "simple declaration stays simple:\n{heap_out}"
        );
        let csr_out = csr_engine(&boxed_graph(), &options);
        assert_same("declared boxed (heap vs csr)", &heap_out, &csr_out);
    }

    /// A declared painter fills its region from its declared payload;
    /// writes outside the region are silent no-ops.
    #[test]
    fn declared_card_is_clipped_and_reads_payload() {
        let options = RenderOptions::plain();
        let ir = card_graph().compute_layout();
        let out = render_plain(&ir, &options);
        assert!(out.contains("# Server"), "card header:\n{out}");
        assert!(out.contains("===="), "separator row:\n{out}");
        assert!(out.contains("id: one"), "payload row:\n{out}");
        assert!(!out.contains('X'), "escaped writes must be dropped:\n{out}");
        assert!(!out.contains("OVERFLOWING"), "overflow truncated:\n{out}");
        // The truncated overflow leaves only its first char in-region.
        let server = ir.node_by_id(1).unwrap();
        let row: &str = out.lines().nth(server.y).unwrap();
        assert_eq!(
            row.chars().nth(server.x + server.width - 1),
            Some('O'),
            "last in-region cell keeps the first overflow char:\n{out}"
        );
    }

    /// Custom painters + payloads travel the CSR pipeline: the
    /// declared card renders byte-identically from the arena IR.
    #[test]
    fn declared_card_parity_across_backends() {
        let options = RenderOptions::plain();
        let heap_out = render_plain(&card_graph().compute_layout(), &options);
        let csr_out = csr_engine(&card_graph(), &options);
        assert_same("declared card (heap vs csr)", &heap_out, &csr_out);
    }

    /// Banding replays painters per band — output is cap-invariant.
    #[test]
    fn painted_nodes_are_band_invariant() {
        for build in [boxed_graph as fn() -> Graph<'static>, card_graph] {
            let options = RenderOptions::plain();
            let ir = build().compute_layout();
            let reference = render_plain(&ir, &options);
            for cap in [1usize, 2, 3] {
                let mut capped = options;
                capped.band_rows_cap = cap;
                assert_same(
                    "painted node banding",
                    &reference,
                    &render_plain(&ir, &capped),
                );
            }
        }
    }

    /// A blank node (custom declaration, no painter) reserves its
    /// area, paints nothing, and keeps its identity for hit-testing —
    /// on both backends.
    #[test]
    fn blank_nodes_reserve_without_painting() {
        let build = || {
            let mut g = Graph::new();
            g.add_node(1, "top");
            g.add_node(
                2,
                CustomNode {
                    label: "spacer",
                    width: 10,
                    height: 3,
                    painter: None,
                    payload: "",
                },
            );
            g.add_node(3, "bottom");
            g.add_edge(1, 2, None);
            g.add_edge(2, 3, None);
            g
        };
        let ir = build().compute_layout();
        let options = RenderOptions::plain();
        let out = render_plain(&ir, &options);
        assert!(!out.contains("spacer"), "blank nodes paint nothing:\n{out}");
        let spacer = ir.node_by_id(2).unwrap();
        for dy in 0..3 {
            let row = out.lines().nth(spacer.y + dy).unwrap_or("");
            let cells: String = row.chars().skip(spacer.x).take(spacer.width).collect();
            assert!(
                cells.trim().is_empty(),
                "row {dy} of the reserved area stays blank:\n{out}"
            );
        }
        let plan = ir.render_plan(&options);
        assert_eq!(
            ir.hit_test(&plan, spacer.x + 1, spacer.y + 1),
            crate::render::engine::HitResult::Node(2),
            "blank node still hit-tests as itself"
        );
        let csr_out = csr_engine(&build(), &options);
        assert_same("blank node (heap vs csr)", &out, &csr_out);
    }

    /// Out-of-range starts must be silent no-ops, not arithmetic:
    /// `x + i` on a `usize::MAX` start panicked in debug builds and
    /// wrapped back into the region in release builds.
    fn overflow_painter(region: &mut NodeRegion<'_, '_>, _ctx: NodePaintCtx<'_>) {
        region.write_str(usize::MAX, 0, "@~");
        region.write_str(0, usize::MAX, "@~");
        region.write_str(usize::MAX, usize::MAX, "@~");
        region.write_str(1, 1, "ok");
    }

    #[test]
    fn write_str_out_of_range_start_is_a_noop() {
        let mut g = Graph::new();
        g.add_node(
            1,
            CustomNode {
                label: "n",
                width: 8,
                height: 3,
                painter: Some(overflow_painter),
                payload: "",
            },
        );
        g.add_node(2, "sink");
        g.add_edge(1, 2, None);
        let out = render_plain(&g.compute_layout(), &RenderOptions::plain());
        assert!(
            !out.contains('@') && !out.contains('~'),
            "out-of-range writes dropped:\n{out}"
        );
        assert!(out.contains("ok"), "in-region writes still land:\n{out}");
    }

    /// A painter that trusts `visible_rows`: it draws only the rows
    /// the ctx declares visible. Cap-invariant output proves the
    /// per-band ranges tile the node exactly.
    fn banded_rows_painter(region: &mut NodeRegion<'_, '_>, ctx: NodePaintCtx<'_>) {
        let (lo, hi) = ctx.visible_rows;
        for y in lo..hi {
            let ch = char::from_digit((y % 10) as u32, 10).unwrap();
            region.set(0, y, ch);
            region.set(region.width() - 1, y, ch);
        }
    }

    /// Tall declared painters may skip out-of-band rows; tall boxed
    /// nodes come from a hand-built IR (graph declarations size boxes
    /// from their labels — the IR is the general contract) and pin the
    /// band-clipped box painter. Both must be cap-invariant.
    #[test]
    fn tall_nodes_band_clip_and_visible_rows() {
        let mut g = Graph::new();
        g.add_node(
            1,
            CustomNode {
                label: "T",
                width: 9,
                height: 40,
                painter: Some(banded_rows_painter),
                payload: "",
            },
        );
        g.add_node(2, "sink");
        g.add_edge(1, 2, None);
        let ir = g.compute_layout();
        let reference = render_plain(&ir, &RenderOptions::plain());
        for cap in [1usize, 2, 7, 1000] {
            let mut capped = RenderOptions::plain();
            capped.band_rows_cap = cap;
            assert_same(
                "tall custom banding",
                &reference,
                &render_plain(&ir, &capped),
            );
        }

        let mut b = crate::ir::LayoutIRBuilder::new();
        b.set_dimensions(20, 42);
        b.add_node(crate::ir::LayoutNode {
            id: 1,
            label: "T",
            x: 2,
            y: 1,
            width: 9,
            height: 40,
            center_x: 6,
            center_y: 20,
            level: 0,
            level_position: 0,
            kind: crate::ir::NodeKind::Explicit,
            has_self_loop: false,
            self_loop_at: None,
            edge_index: None,
            content_tag: 1, // boxed
        });
        let ir = b.build();
        let reference = render_plain(&ir, &RenderOptions::plain());
        assert!(reference.contains('\u{250c}'), "box paints:\n{reference}");
        for cap in [1usize, 3, 7] {
            let mut capped = RenderOptions::plain();
            capped.band_rows_cap = cap;
            assert_same(
                "tall boxed banding",
                &reference,
                &render_plain(&ir, &capped),
            );
        }
    }

    /// The no-alloc byte surface renders declared nodes identically to
    /// the String surface, with estimate-sized buffers.
    #[test]
    fn painted_nodes_render_to_bytes() {
        use crate::render::engine::{
            estimate_render_arena_size, estimate_render_output_size, render_to_bytes,
        };
        for build in [boxed_graph as fn() -> Graph<'static>, card_graph] {
            let mut options = RenderOptions::plain();
            options.band_rows_cap = 2;
            let ir = build().compute_layout();
            let want = render_plain(&ir, &options);
            let mut backing = vec![0u8; estimate_render_arena_size(&ir, &options)];
            let arena = Arena::new(&mut backing);
            let mut out = vec![0u8; estimate_render_output_size(&ir, &options)];
            let written = render_to_bytes(&ir, &options, &arena, &mut out).expect("bytes render");
            let got = core::str::from_utf8(&out[..written]).unwrap();
            assert_same("declared nodes (bytes vs string)", &want, got);
        }
    }

    /// Content kinds declared at construction arrive byte-identically
    /// in both IRs — and now drive rendering directly.
    #[test]
    fn content_tags_travel_both_irs() {
        let build = || {
            let mut g = Graph::new();
            g.add_node(1, "simple");
            g.add_node(2, BoxedNode("boxed"));
            g.add_node(
                3,
                CustomNode {
                    label: "card",
                    width: 8,
                    height: 3,
                    painter: None,
                    payload: "",
                },
            );
            g.add_edge(1, 2, None);
            g.add_edge(2, 3, None);
            g
        };
        let heap_ir = build().compute_layout();
        let mut heap_tags: Vec<(usize, u8)> = heap_ir
            .nodes
            .iter()
            .map(|n| (n.id, n.content_tag))
            .collect();
        heap_tags.sort_unstable();
        assert_eq!(heap_tags, [(1, 0), (2, 1), (3, 2)]);

        let g = build();
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
        let mut csr_tags: Vec<(usize, u8)> =
            ir.nodes().iter().map(|n| (n.id, n.content_tag)).collect();
        csr_tags.sort_unstable();
        assert_eq!(csr_tags, heap_tags);

        let options = RenderOptions::plain();
        let heap_out = render_plain(&heap_ir, &options);
        let csr_out = csr_engine(&build(), &options);
        assert_same("content-tag graph (heap vs csr)", &heap_out, &csr_out);
    }

    /// Slice-5 guard (temp/08): physical node extents are emitted from
    /// the node's DECLARED dimensions, not the role-space packing
    /// extent — an asymmetric 12×5 node must reach the IR as 12×5 in
    /// both backends. (Under `Horizontal`, the packing extent is the
    /// height; emitting it as the width would corrupt centers, ports,
    /// and content bounds.)
    #[test]
    fn asymmetric_node_extents_reach_the_ir() {
        let build = || {
            let mut g = Graph::new();
            g.add_node(1, "a");
            g.add_node(
                2,
                CustomNode {
                    label: "wide",
                    width: 12,
                    height: 5,
                    painter: None,
                    payload: "",
                },
            );
            g.add_node(3, "b");
            g.add_edge(1, 2, None);
            g.add_edge(2, 3, None);
            g
        };
        let heap_ir = build().compute_layout();
        let n = heap_ir.nodes.iter().find(|n| n.id == 2).expect("node 2");
        assert_eq!((n.width, n.height), (12, 5));
        assert_eq!(n.center_x, n.x + 6);
        assert_eq!(n.center_y, n.y + 2);

        let g = build();
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
        let n = ir.nodes().iter().find(|n| n.id == 2).expect("node 2");
        assert_eq!((n.width, n.height), (12, 5));
        assert_eq!(n.center_x, n.x + 6);
        assert_eq!(n.center_y, n.y + 2);
    }

    /// D5(ii) (temp/08): at `node_spacing == 0` a self-loop node's
    /// marker cell is reserved — no neighbor may start on it (the
    /// legacy behavior silently painted the neighbor over the `↺`).
    /// Also pins the D5 invariant `has_self_loop ==
    /// self_loop_at.is_some()` and the marker cell formula, plus
    /// cross-backend byte parity for the corner.
    #[test]
    fn spacing_zero_reserves_self_loop_marker() {
        let build = || {
            let mut g = Graph::new();
            g.add_node(1, "a");
            g.add_node(2, "b");
            g.add_node(3, "c");
            g.add_edge(1, 2, None);
            g.add_edge(1, 3, None);
            g.add_edge(2, 2, None);
            g
        };
        let mut config = LayoutConfig::standard();
        config.node_spacing = 0;

        let heap_ir = build().compute_layout_with_config(&config);
        for n in heap_ir.nodes.iter() {
            assert_eq!(n.has_self_loop, n.self_loop_at.is_some(), "D5 invariant");
        }
        let b = heap_ir.nodes.iter().find(|n| n.id == 2).expect("node 2");
        let (mx, my) = b.self_loop_at.expect("loop node has a marker cell");
        assert_eq!((mx, my), (b.x + b.width, b.y), "legacy marker cell");
        for n in heap_ir.nodes.iter() {
            let covers = n.x <= mx && mx < n.x + n.width && n.y <= my && my < n.y + n.height;
            assert!(!covers, "node {} overlaps the reserved marker cell", n.id);
        }

        // CSR twin: identical bytes for the corner graph.
        let g = build();
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
        let bc = ir.nodes().iter().find(|n| n.id == 2).expect("node 2");
        assert!(bc.has_self_loop);
        assert_eq!(bc.self_loop_at, (bc.x + bc.width, bc.y));

        let options = RenderOptions::plain();
        let mut heap_out = alloc::string::String::new();
        heap_ir
            .render_with(&options, &mut heap_out)
            .expect("heap render");
        let mut csr_out = alloc::string::String::new();
        ir.render_with(&options, &mut csr_out).expect("csr render");
        assert_same(
            "spacing-0 self-loop graph (heap vs csr)",
            &heap_out,
            &csr_out,
        );
    }

    /// D8 — the embedded front door: the same declared content built
    /// directly on `CsrGraphBuilder` (no `Graph`, no alloc-side
    /// construction) renders byte-identically to the Graph → to_csr
    /// path. Conversion tests alone do not cover this door.
    #[test]
    fn direct_built_csr_declares_content() {
        use crate::graph::csr::{CsrGraph, CsrGraphBuilder};
        let options = RenderOptions::plain();
        let via_graph = csr_engine(&card_graph(), &options);

        let payload = "id: one";
        let label_bytes = "Server".len() + "Cache".len() + "Client".len() + payload.len();
        let csr_size = CsrGraph::required_arena_size_with_content(3, 2, label_bytes, 0, 1) + 256;
        let mut csr_buf = vec![0u8; csr_size];
        let mut csr_arena = Arena::new(&mut csr_buf);
        let mut b = CsrGraphBuilder::new(&mut csr_arena, 3, 2, label_bytes, 1).unwrap();
        b.add_node(
            1,
            CustomNode {
                label: "Server",
                width: 14,
                height: 5,
                painter: Some(card_painter),
                payload,
            },
        )
        .unwrap();
        b.add_node(2, BoxedNode("Cache")).unwrap();
        b.add_node(3, "Client").unwrap();
        b.add_edge(2, 0).unwrap(); // Client → Server (indices)
        b.add_edge(0, 1).unwrap(); // Server → Cache
        let csr = b.build().unwrap();

        let config = LayoutConfig::standard();
        let size = 256 * 1024;
        let mut temp_buf = vec![0u8; size];
        let mut out_buf = vec![0u8; size];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_arena = Arena::new(&mut out_buf);
        let ir = csr
            .compute_layout_arena(&config, &mut temp_arena, &mut out_arena)
            .expect("direct CSR layout");
        let direct = ir.render_string(&options);
        assert_same(
            "declared content (direct CSR vs Graph→to_csr)",
            &via_graph,
            &direct,
        );
    }

    /// Failed direct-builder insertions must be atomic: any exhausted
    /// capacity leaves the builder untouched (no committed node, no
    /// shifted indices).
    #[test]
    fn direct_builder_failures_are_atomic() {
        use crate::graph::csr::{CsrGraph, CsrGraphBuilder};
        // Exhausted custom-entry capacity (max_custom = 0).
        let size = CsrGraph::required_arena_size_with_content(2, 1, 64, 0, 0) + 256;
        let mut buf = vec![0u8; size];
        let mut arena = Arena::new(&mut buf);
        let mut b = CsrGraphBuilder::new(&mut arena, 2, 1, 64, 0).unwrap();
        let failed = b.add_node(
            1,
            CustomNode {
                label: "card",
                width: 8,
                height: 3,
                painter: Some(card_painter),
                payload: "p",
            },
        );
        assert!(failed.is_none(), "no custom capacity → None");
        // The failed insertion committed nothing: the next node is 0.
        assert_eq!(b.add_node(1, "ok"), Some(0));

        // Exhausted label/payload storage.
        let size = CsrGraph::required_arena_size_with_content(2, 1, 4, 0, 1) + 256;
        let mut buf = vec![0u8; size];
        let mut arena = Arena::new(&mut buf);
        let mut b = CsrGraphBuilder::new(&mut arena, 2, 1, 4, 1).unwrap();
        let failed = b.add_node(
            1,
            CustomNode {
                label: "n",
                width: 8,
                height: 3,
                painter: Some(card_painter),
                payload: "way too long for four bytes",
            },
        );
        assert!(failed.is_none(), "no payload space → None");
        assert_eq!(b.add_node(1, "ok"), Some(0));
    }

    /// NC9 parity includes Unicode: character-based sizing on the
    /// direct builder matches `Graph`, so multi-byte labels render
    /// byte-identically through both construction paths.
    #[test]
    fn unicode_labels_parity_direct_vs_graph() {
        use crate::graph::csr::{CsrGraph, CsrGraphBuilder};
        let options = RenderOptions::plain();
        let build = || {
            let mut g = Graph::new();
            g.add_node(1, "Caché");
            g.add_node(2, BoxedNode("naïve"));
            g.add_edge(1, 2, None);
            g
        };
        let via_graph = csr_engine(&build(), &options);

        let label_bytes = "Caché".len() + "naïve".len();
        let size = CsrGraph::required_arena_size_with_content(2, 1, label_bytes, 0, 0) + 256;
        let mut buf = vec![0u8; size];
        let mut arena = Arena::new(&mut buf);
        let mut b = CsrGraphBuilder::new(&mut arena, 2, 1, label_bytes, 0).unwrap();
        b.add_node(1, "Caché").unwrap();
        b.add_node(2, BoxedNode("naïve")).unwrap();
        b.add_edge(0, 1).unwrap();
        let csr = b.build().unwrap();
        let mut temp_buf = vec![0u8; 256 * 1024];
        let mut out_buf = vec![0u8; 256 * 1024];
        let mut ta = Arena::new(&mut temp_buf);
        let mut oa = Arena::new(&mut out_buf);
        let ir = csr
            .compute_layout_arena(&LayoutConfig::standard(), &mut ta, &mut oa)
            .expect("layout");
        assert_same(
            "unicode labels (direct vs Graph→to_csr)",
            &via_graph,
            &ir.render_string(&options),
        );
    }

    /// `estimate_json_size` must be a real upper bound with custom
    /// content: payload bytes, escaping, and Unicode included — an
    /// exactly estimate-sized buffer always serializes.
    #[test]
    fn json_estimate_covers_custom_content() {
        let mut g = Graph::new();
        g.add_node(
            1,
            CustomNode {
                label: "Caché",
                width: 10,
                height: 4,
                painter: Some(card_painter),
                payload: "line \"one\"\nline\ttwo — naïve",
            },
        );
        g.add_node(2, "sink");
        g.add_edge(1, 2, None);
        let config = LayoutConfig::standard();
        let mut csr_buf = vec![0u8; g.estimate_csr_arena_size() * 2];
        let mut csr_arena = Arena::new(&mut csr_buf);
        let csr = g.to_csr(&mut csr_arena).expect("CSR conversion");
        let mut temp_buf = vec![0u8; 256 * 1024];
        let mut out_buf = vec![0u8; 256 * 1024];
        let mut ta = Arena::new(&mut temp_buf);
        let mut oa = Arena::new(&mut out_buf);
        let ir = csr
            .compute_layout_arena(&config, &mut ta, &mut oa)
            .expect("layout");
        let estimate = ir.estimate_json_size();
        let mut buf = vec![0u8; estimate];
        let written = ir
            .serialize_json(&mut buf)
            .expect("estimate-sized buffer must suffice");
        let json = core::str::from_utf8(&buf[..written]).unwrap();
        assert!(json.contains("\"content_kind\":\"custom\""), "{json}");
        assert!(json.contains("\"payload\":"), "{json}");
    }

    /// NC7 flyweight: ONE painter fn backs many nodes; per-node
    /// identity arrives through the ctx (label + payload), so each
    /// node renders its own content — on both backends.
    #[test]
    fn one_painter_many_nodes_flyweight() {
        fn tagline(region: &mut NodeRegion<'_, '_>, ctx: NodePaintCtx<'_>) {
            region.write_str(0, 0, ctx.label);
            region.write_str(0, 1, ctx.payload);
        }
        let build = || {
            let mut g = Graph::new();
            for (id, label, payload) in [
                (1, "alpha", "first"),
                (2, "beta", "second"),
                (3, "gamma", "third"),
            ] {
                g.add_node(
                    id,
                    CustomNode {
                        label,
                        width: 8,
                        height: 2,
                        painter: Some(tagline), // the SAME fn every time
                        payload,
                    },
                );
            }
            g.add_edge(1, 2, None);
            g.add_edge(2, 3, None);
            g
        };
        let options = RenderOptions::plain();
        let out = render_plain(&build().compute_layout(), &options);
        for text in ["alpha", "first", "beta", "second", "gamma", "third"] {
            assert!(out.contains(text), "per-node content {text}:\n{out}");
        }
        let csr_out = csr_engine(&build(), &options);
        assert_same("flyweight painter (heap vs csr)", &out, &csr_out);
    }

    /// The whole reserved area hits the node, declaration-agnostic.
    #[test]
    fn multi_row_nodes_hit_across_their_area() {
        let ir = boxed_graph().compute_layout();
        let options = RenderOptions::plain();
        let plan = ir.render_plan(&options);
        let server = ir.node_by_id(1).unwrap();
        for dy in 0..3 {
            assert_eq!(
                ir.hit_test(&plan, server.x + 1, server.y + dy),
                crate::render::engine::HitResult::Node(1),
                "row {dy} of the reserved area"
            );
        }
        assert_ne!(
            ir.hit_test(&plan, server.x + 1, server.y + 3),
            crate::render::engine::HitResult::Node(1),
            "below the reserved area"
        );
    }
}

// ── Horizontal geometry invariants + rendering (temp/08) ───────────────
//
// The acceptance suite for native LR/RL layout, run over the whole
// corpus in BOTH orientations: geometric invariants on the IR, the
// glyph⇄hit ink sweep, band cap-invariance, and cross-backend parity
// at field and byte level.
#[cfg(feature = "layout-horizontal")]
mod lr_invariants {
    use super::*;
    use crate::algorithms::sugiyama::geometry::Horizontal;
    use crate::algorithms::sugiyama::heap::compute_layout_cfg;
    use crate::graph::Direction;
    use crate::ir::{EdgePath, FlowAxis, LayoutIR, LayoutNode};

    fn lr<'a>(g: &Graph<'a>) -> LayoutIR<'a> {
        lr_rl(g, Direction::LeftRight)
    }

    fn on_rows(y: usize, n: &LayoutNode<'_>) -> bool {
        n.y <= y && y < n.y + n.height
    }

    /// The P1 exit invariants over one Horizontal IR. `dir` selects
    /// the flow orientation: under `RightLeft` the source port sits on
    /// the LEFT face and the target entry on the right.
    fn check_invariants(tag: &str, ir: &LayoutIR<'_>, dir: Direction) {
        // The IR must RECORD the direction it was laid out for: a
        // regression that mirrors geometry correctly but keeps the
        // wrong tag would satisfy every geometric check here while
        // feeding style callbacks the wrong context.
        assert_eq!(ir.direction(), dir, "{tag}: recorded direction");
        let rightward = !matches!(dir, Direction::RightLeft);
        // Exit face of a source / entry face of a target, per flow.
        let exit = |n: &LayoutNode<'_>| {
            if rightward { n.x + n.width - 1 } else { n.x }
        };
        let entry = |n: &LayoutNode<'_>| {
            if rightward { n.x } else { n.x + n.width - 1 }
        };
        let nodes = ir.nodes();
        // I1: node spans pairwise disjoint and inside the canvas.
        for (i, a) in nodes.iter().enumerate() {
            assert!(
                a.x + a.width <= ir.width() && a.y + a.height <= ir.height(),
                "{tag}: node {} ({},{} {}x{}) exceeds canvas {}x{}",
                a.id,
                a.x,
                a.y,
                a.width,
                a.height,
                ir.width(),
                ir.height()
            );
            for b in nodes.iter().skip(i + 1) {
                let overlap = a.x < b.x + b.width
                    && b.x < a.x + a.width
                    && a.y < b.y + b.height
                    && b.y < a.y + a.height;
                assert!(!overlap, "{tag}: nodes {} and {} overlap", a.id, b.id);
            }
        }
        let by_id = |id: usize| {
            nodes
                .iter()
                .find(|n| n.id == id)
                .unwrap_or_else(|| panic!("{tag}: node {id}"))
        };
        for e in ir.edges() {
            if e.from_id == e.to_id {
                continue;
            }
            // I2: every Horizontal edge has a horizontal trunk.
            assert_eq!(e.flow_axis, FlowAxis::X, "{tag}: {}→{}", e.from_id, e.to_id);
            let (s, t) = (by_id(e.from_id), by_id(e.to_id));
            // I3: endpoints on the pair's faces at their port rows.
            // Coordinates are layout-order for reversed edges, so accept
            // either orientation of the pair.
            let fwd_ok = e.from_x == exit(s)
                && e.to_x == entry(t)
                && on_rows(e.from_y, s)
                && on_rows(e.to_y, t);
            let rev_ok = e.from_x == exit(t)
                && e.to_x == entry(s)
                && on_rows(e.from_y, t)
                && on_rows(e.to_y, s);
            assert!(
                fwd_ok || rev_ok,
                "{tag}: {}→{} endpoints off the node faces: from ({}, {}), to ({}, {}); \
                 s=({},{} {}x{}), t=({},{} {}x{})",
                e.from_id,
                e.to_id,
                e.from_x,
                e.from_y,
                e.to_x,
                e.to_y,
                s.x,
                s.y,
                s.width,
                s.height,
                t.x,
                t.y,
                t.width,
                t.height
            );
            // I4: trunk-band geometry inside the canvas; corner bends
            // strictly between the two faces.
            match &e.path {
                EdgePath::Corner { bend_at } => {
                    // Order-free: the flow may run either way.
                    let (a, b) = if fwd_ok {
                        (exit(s), entry(t))
                    } else {
                        (exit(t), entry(s))
                    };
                    let (lo, hi) = (a.min(b), a.max(b));
                    assert!(
                        *bend_at > lo && *bend_at < hi,
                        "{tag}: {}→{} bend {} outside the gap ({lo}, {hi})",
                        e.from_id,
                        e.to_id,
                        bend_at
                    );
                }
                EdgePath::MultiSegment { waypoints, .. } => {
                    for &(wx, wy) in waypoints {
                        assert!(
                            wx < ir.width() && wy < ir.height(),
                            "{tag}: {}→{} waypoint ({wx}, {wy}) outside canvas",
                            e.from_id,
                            e.to_id
                        );
                    }
                }
                _ => {}
            }
            // I5: label seeds inside the canvas.
            if e.label.is_some() {
                assert!(
                    e.label_x < ir.width() && e.label_y < ir.height(),
                    "{tag}: {}→{} label at ({}, {}) outside canvas {}x{}",
                    e.from_id,
                    e.to_id,
                    e.label_x,
                    e.label_y,
                    ir.width(),
                    ir.height()
                );
            }
        }
        // I6: self-loop markers — invariant, reserved cell, in canvas.
        for n in nodes {
            assert_eq!(n.has_self_loop, n.self_loop_at.is_some(), "{tag}: {}", n.id);
            if let Some((mx, my)) = n.self_loop_at {
                assert!(
                    mx < ir.width() && my < ir.height(),
                    "{tag}: {} marker ({mx}, {my}) outside canvas {}x{}",
                    n.id,
                    ir.width(),
                    ir.height()
                );
                for o in nodes {
                    let covers =
                        o.x <= mx && mx < o.x + o.width && o.y <= my && my < o.y + o.height;
                    assert!(!covers, "{tag}: node {} covers {}'s marker", o.id, n.id);
                }
            }
        }
        // I7: boxes inside the canvas; LR labels fit the box width.
        for sg in ir.subgraphs() {
            assert!(
                sg.x + sg.width <= ir.width() && sg.y + sg.height <= ir.height(),
                "{tag}: box {} exceeds canvas",
                sg.id
            );
            let label_min = (sg.label.len() + 4).min(40);
            assert!(
                sg.width >= label_min,
                "{tag}: box {} width {} cannot show its label (needs {label_min})",
                sg.id,
                sg.width
            );
        }
        // I8: no node straddles a box border — every node is strictly
        // inside (borders clear) or fully outside every box.
        for sg in ir.subgraphs() {
            for n in nodes {
                let inside = n.x > sg.x
                    && n.x + n.width < sg.x + sg.width
                    && n.y > sg.y
                    && n.y + n.height < sg.y + sg.height;
                let outside = n.x + n.width <= sg.x
                    || n.x >= sg.x + sg.width
                    || n.y + n.height <= sg.y
                    || n.y >= sg.y + sg.height;
                assert!(
                    inside || outside,
                    "{tag}: node {} straddles box {}'s border",
                    n.id,
                    sg.id
                );
            }
        }
    }

    /// A node wider than `u8::MAX` — level extents are geometry and
    /// must survive the smallest index configuration (they used to
    /// wrap through an `Idx` cast under `arena-idx-u8`: 256 → 0).
    fn wide_node() -> Graph<'static> {
        use crate::render::engine::CustomNode;
        let mut g = Graph::new();
        g.add_node(1, "a");
        g.add_node(
            2,
            CustomNode {
                label: "very-wide",
                width: 300,
                height: 1,
                painter: None,
                payload: "",
            },
        );
        g.add_node(3, "b");
        g.add_edge(1, 2, None);
        g.add_edge(2, 3, None);
        g
    }

    pub(super) fn corpus() -> [(&'static str, Graph<'static>); 10] {
        [
            ("fan", fan()),
            ("stage", stage()),
            ("skip", skip()),
            ("back", back_edges()),
            ("two_cycle", two_cycle()),
            ("self_loop", self_loop()),
            ("labels", colliding_labels()),
            ("nested", nested_boxes()),
            ("hero", hero_graph()),
            ("wide_node", wide_node()),
        ]
    }

    #[test]
    fn corpus_invariants() {
        for dir in [Direction::LeftRight, Direction::RightLeft] {
            for (tag, g) in corpus() {
                check_invariants(&alloc::format!("{tag} {dir:?}"), &lr_rl(&g, dir), dir);
            }
        }
    }

    /// LR-P2 exit gate: the LR corpus is FIELD-identical across
    /// backends — every node/edge/subgraph through the `LayoutView`
    /// lens plus the canvas — with exactly estimate-sized arenas.
    /// (Rendered byte-parity joins at P3 when the compositor learns
    /// horizontal trunks.)
    #[test]
    fn lr_corpus_matches_across_backends() {
        use crate::render::engine::view::LayoutView;

        fn sorted_debug<T: core::fmt::Debug>(items: impl Iterator<Item = T>) -> Vec<String> {
            let mut v: Vec<String> = items.map(|x| format!("{x:?}")).collect();
            v.sort();
            v
        }

        for dir in [Direction::LeftRight, Direction::RightLeft] {
            let mut config = LayoutConfig::standard();
            config.direction = dir;
            for (tag, g) in corpus() {
                let heap_ir = lr_rl(&g, dir);

                let mut csr_buf = vec![0u8; g.estimate_csr_arena_size()];
                let mut csr_arena = Arena::new(&mut csr_buf);
                let csr = g.to_csr(&mut csr_arena).expect("CSR conversion");
                let size = g.estimate_layout_arena_size_with(&config);
                let mut temp_buf = vec![0u8; size];
                let mut out_buf = vec![0u8; size];
                let mut temp_arena = Arena::new(&mut temp_buf);
                let mut out_arena = Arena::new(&mut out_buf);
                // The PUBLIC entry point — its direction dispatch is
                // what these tests must pin, not the axis-parameterized
                // internal (which would stay green if the public match
                // regressed to `Vertical`).
                let csr_ir = csr
                    .compute_layout_arena(&config, &mut temp_arena, &mut out_arena)
                    .unwrap_or_else(|e| panic!("{tag}: CSR LR layout failed: {e:?}"));

                assert_eq!(
                    (LayoutView::width(&heap_ir), LayoutView::height(&heap_ir)),
                    (LayoutView::width(&csr_ir), LayoutView::height(&csr_ir)),
                    "{tag} {dir:?}: canvas"
                );
                assert_eq!(
                    LayoutView::direction(&heap_ir),
                    dir,
                    "{tag}: heap direction"
                );
                assert_eq!(LayoutView::direction(&csr_ir), dir, "{tag}: csr direction");
                assert_eq!(
                    LayoutView::node_count(&heap_ir),
                    LayoutView::node_count(&csr_ir),
                    "{tag} {dir:?}: node count"
                );
                let hn = sorted_debug(
                    (0..LayoutView::node_count(&heap_ir)).map(|i| LayoutView::node(&heap_ir, i)),
                );
                let cn = sorted_debug(
                    (0..LayoutView::node_count(&csr_ir)).map(|i| LayoutView::node(&csr_ir, i)),
                );
                assert_eq!(hn, cn, "{tag} {dir:?}: nodes");
                let he = sorted_debug(
                    (0..LayoutView::edge_count(&heap_ir)).map(|i| LayoutView::edge(&heap_ir, i)),
                );
                let ce = sorted_debug(
                    (0..LayoutView::edge_count(&csr_ir)).map(|i| LayoutView::edge(&csr_ir, i)),
                );
                assert_eq!(he, ce, "{tag} {dir:?}: edges");
                let hs = sorted_debug(
                    (0..LayoutView::subgraph_count(&heap_ir))
                        .map(|i| LayoutView::subgraph(&heap_ir, i)),
                );
                let cs = sorted_debug(
                    (0..LayoutView::subgraph_count(&csr_ir))
                        .map(|i| LayoutView::subgraph(&csr_ir, i)),
                );
                assert_eq!(hs, cs, "{tag} {dir:?}: subgraphs");

                // Arena-only `min_y`/`max_y` (not mirrored by the view):
                // every physical y the edge touches — endpoints and
                // waypoint rows — must sit inside the cached bounds
                // (slices review: waypoint excursions used to escape).
                use crate::render::engine::view::PathRef;
                for i in 0..LayoutView::edge_count(&csr_ir) {
                    let raw = *crate::ir::arena::LayoutIRArena::edge(&csr_ir, i);
                    let v = LayoutView::edge(&csr_ir, i);
                    let mut ys = alloc::vec![raw.from_y, raw.to_y];
                    if let PathRef::MultiSegment { waypoints, .. } = v.path {
                        ys.extend(waypoints.iter().map(|&(_, wy)| wy));
                    }
                    for y in ys {
                        assert!(
                            raw.min_y <= y && y <= raw.max_y,
                            "{tag}: edge {i} touches y={y} outside cached bounds {}..={}",
                            raw.min_y,
                            raw.max_y
                        );
                    }
                }
            }
        }
    }

    /// Hero-LR numeric sanity: same elements as the TD layout, wide
    /// canvas, all invariants — the phase's rendered acceptance stays
    /// at P3.
    #[test]
    fn hero_lr_numeric_sanity() {
        let td = hero_graph().compute_layout();
        let ir = lr(&hero_graph());
        assert_eq!(td.nodes().len(), ir.nodes().len());
        assert_eq!(td.edges().len(), ir.edges().len());
        assert_eq!(td.level_count(), ir.level_count());
        assert!(
            ir.width() > ir.height(),
            "LR hero should be wide: {}x{}",
            ir.width(),
            ir.height()
        );
        check_invariants("hero-lr", &ir, Direction::LeftRight);
    }

    /// Slices review: a WIDE member must not escape through the box's
    /// trailing level border (the member extent was hardcoded to one
    /// line before).
    #[test]
    fn lr_wide_member_stays_inside_box() {
        use crate::render::engine::CustomNode;
        let mut g = Graph::new();
        g.add_node(1, "in");
        g.add_node(
            2,
            CustomNode {
                label: "wide-member",
                width: 16,
                height: 1,
                painter: None,
                payload: "",
            },
        );
        g.add_node(3, "out");
        g.add_edge(1, 2, None);
        g.add_edge(2, 3, None);
        let sg = g.add_subgraph("W");
        g.put_nodes(&[2]).inside(sg).unwrap();
        let ir = lr(&g);
        check_invariants("wide-member", &ir, Direction::LeftRight);
        let b = &ir.subgraphs()[0];
        let m = ir.nodes().iter().find(|n| n.id == 2).unwrap();
        assert!(
            m.x + m.width + 2 <= b.x + b.width,
            "trailing level pad after the wide member: node ends {}, box ends {}",
            m.x + m.width,
            b.x + b.width
        );
    }

    /// Slices review: a parent whose only content is a child box must
    /// keep its label block clear of the child's top border — the
    /// parent/child cross expansion is the full (3, 2) label-side pad
    /// under Horizontal, not a symmetric border cell.
    #[test]
    fn lr_child_only_parent_has_label_room() {
        let mut g = Graph::new();
        g.add_node(1, "in");
        g.add_node(2, "core");
        g.add_node(3, "out");
        g.add_edge(1, 2, None);
        g.add_edge(2, 3, None);
        let outer = g.add_subgraph("Parent");
        let inner = g.add_subgraph("Child");
        g.put_subgraphs(&[inner]).inside(outer).unwrap();
        g.put_nodes(&[2]).inside(inner).unwrap();
        let ir = lr(&g);
        check_invariants("child-only-parent", &ir, Direction::LeftRight);
        let parent = ir.subgraphs().iter().find(|s| s.label == "Parent").unwrap();
        let child = ir.subgraphs().iter().find(|s| s.label == "Child").unwrap();
        assert!(
            child.y >= parent.y + 3,
            "parent label block above the child top border: parent.y {}, child.y {}",
            parent.y,
            child.y
        );
        assert!(
            child.y + child.height + 2 <= parent.y + parent.height,
            "trailing cross pad below the child"
        );
        assert!(child.x > parent.x && child.x + child.width < parent.x + parent.width);
    }

    /// P3-S1: horizontal trunks render — a chain paints `[a]` and
    /// `[b]` on one row joined by a `─` trunk with a `→` arrowhead.
    #[test]
    fn lr_chain_renders_horizontal_trunk() {
        let mut g = Graph::new();
        g.add_node(1, "a");
        g.add_node(2, "b");
        g.add_edge(1, 2, None);
        let ir = lr(&g);
        let out = render_plain(&ir, &RenderOptions::plain());
        let row = out
            .lines()
            .find(|l| l.contains("[a]"))
            .expect("row with [a]");
        assert!(row.contains("[b]"), "same-row target: {row:?}");
        assert!(row.contains('─'), "trunk: {row:?}");
        assert!(row.contains('→'), "arrowhead: {row:?}");
        let a = row.find("[a]").unwrap();
        let arrow = row.find('→').unwrap();
        let b = row.find("[b]").unwrap();
        assert!(a < arrow && arrow < b, "order: {row:?}");
    }

    /// P3-S1: corner edges bend through vertical cross runs, and the
    /// LR self-loop marker paints at its IR cell below the node.
    #[test]
    fn lr_corner_and_self_loop_render() {
        let mut g = Graph::new();
        g.add_node(1, "root");
        g.add_node(2, "up");
        g.add_node(3, "down");
        g.add_edge(1, 2, None);
        g.add_edge(1, 3, None);
        g.add_edge(1, 1, None);
        let ir = lr(&g);
        let out = render_plain(&ir, &RenderOptions::plain());
        assert!(out.contains('→'), "arrowheads:\n{out}");
        assert!(
            out.contains('│') || out.contains('┐') || out.contains('└'),
            "vertical cross runs:\n{out}"
        );
        assert!(out.contains('↺'), "self-loop marker:\n{out}");
    }

    /// P3-S2: a labeled LR edge paints its label — wherever the D9
    /// ladder lands it (own cross segment, inline on the trunk, or
    /// floated) — and the label cell hit-tests to its edge.
    #[test]
    fn lr_labeled_edge_renders_and_hits() {
        use crate::render::engine::HitResult;
        let mut g = Graph::new();
        g.add_node(1, "a");
        g.add_node(2, "b");
        g.add_edge(1, 2, Some("go"));
        let ir = lr(&g);
        let options = RenderOptions::plain();
        let out = render_plain(&ir, &options);
        assert!(out.contains("\"go\""), "label painted:\n{out}");
        let (row_i, col) = out
            .lines()
            .enumerate()
            .find_map(|(r, l)| l.find("\"go\"").map(|c| (r, c)))
            .expect("label location");
        let plan = ir.render_plan(&options);
        assert_eq!(
            ir.hit_test(&plan, col + 1, row_i),
            HitResult::Edge(0),
            "label cell owns its edge:\n{out}"
        );
    }

    /// D9 host 1 (Ash's ruling, temp/08): a labeled LR edge that jogs
    /// puts its label on its OWN cross segment — the direct mirror of
    /// the TD picture, where the label interrupts the line it
    /// annotates. The label row must therefore be strictly BETWEEN
    /// the two trunk rows (not on either trunk, not floated above).
    #[test]
    fn lr_label_prefers_its_own_cross_segment() {
        let mut g = Graph::new();
        g.add_node(1, "a");
        g.add_node(2, "b");
        g.add_node(3, "c");
        g.add_edge(1, 2, Some("up"));
        g.add_edge(1, 3, Some("down"));
        let ir = lr(&g);
        let out = render_plain(&ir, &RenderOptions::plain());
        for (idx, text) in [(0usize, "up"), (1, "down")] {
            let needle = alloc::format!("\"{text}\"");
            // Character-cell column, not `str::find`'s byte offset.
            let (row, col) = out
                .lines()
                .enumerate()
                .find_map(|(r, l)| l.find(&needle).map(|b| (r, l[..b].chars().count())))
                .unwrap_or_else(|| panic!("label {text:?} painted:\n{out}"));
            let e = &ir.edges()[idx];
            let (lo, hi) = (e.from_y.min(e.to_y), e.from_y.max(e.to_y));
            assert!(
                row > lo && row < hi,
                "label {text:?} sits between the trunk rows (row {row} strictly \
                 inside {lo}..{hi}):\n{out}"
            );
            // Between the trunk rows alone would also admit an upward
            // float — prove the span actually CROSSES the edge's own
            // bend column, which only the cross host does.
            let crate::ir::EdgePath::Corner { bend_at } = e.path else {
                panic!("fan edges route as corners; got {:?}", e.path);
            };
            let span = needle.chars().count();
            assert!(
                bend_at >= col && bend_at < col + span,
                "label {text:?} at cols {col}..{} covers its own bend column \
                 {bend_at}:\n{out}",
                col + span
            );
        }
    }

    /// P3-S2 glyph⇄hit over the LR corpus: every edge-ink glyph in a
    /// rendered LR graph hit-tests to SOME element — no orphan ink.
    #[test]
    fn lr_ink_always_hits() {
        use crate::render::engine::HitResult;
        // Canvas-only contract: legend rows sit below the canvas and
        // are not hit-testable elements — render without them.
        let mut options = RenderOptions::plain();
        options.legend = false;
        let ink = [
            '─', '│', '→', '←', '┌', '┐', '└', '┘', '┬', '┴', '├', '┤', '┼', '↺', '┊', '┈', '⇢',
            '⇠',
        ];
        for (dir, (tag, g)) in [Direction::LeftRight, Direction::RightLeft]
            .into_iter()
            .flat_map(|d| corpus().into_iter().map(move |c| (d, c)))
        {
            let ir = lr_rl(&g, dir);
            let out = render_plain(&ir, &options);
            let plan = ir.render_plan(&options);
            for (r, line) in out.lines().enumerate() {
                for (c, ch) in line.chars().enumerate() {
                    if ink.contains(&ch) {
                        assert!(
                            ir.hit_test(&plan, c, r) != HitResult::None,
                            "{tag} {dir:?}: orphan ink {ch:?} at ({c}, {r})\n{out}"
                        );
                    }
                }
            }
        }
    }

    /// P3-S3 cap-invariance: banded LR renders are byte-identical at
    /// every cap — bands lose level alignment under X flows, so the
    /// hard-cut path carries the correctness (plain and colored).
    #[test]
    fn lr_band_cap_invariance() {
        for (dir, (tag, g)) in [Direction::LeftRight, Direction::RightLeft]
            .into_iter()
            .flat_map(|d| corpus().into_iter().map(move |c| (d, c)))
        {
            let ir = lr_rl(&g, dir);
            let tag = alloc::format!("{tag} {dir:?}");
            for colored in [false, true] {
                let mut base_opts = if colored {
                    RenderOptions::colored(Palette::Ansi)
                } else {
                    RenderOptions::plain()
                };
                base_opts.band_rows_cap = 1000;
                let base = if colored {
                    render_colored(&ir, &base_opts)
                } else {
                    render_plain(&ir, &base_opts)
                };
                for cap in [1usize, 2, 3, 5, 7, 64] {
                    let mut opts = base_opts;
                    opts.band_rows_cap = cap;
                    let out = if colored {
                        render_colored(&ir, &opts)
                    } else {
                        render_plain(&ir, &opts)
                    };
                    assert_same(&format!("{tag} cap={cap} colored={colored}"), &base, &out);
                }
            }
        }
    }

    /// P3-S3: rendered LR byte-parity across backends — the promise
    /// deferred from the P2 field gate: identical IRs must render
    /// identically, plain and colored.
    #[test]
    fn lr_corpus_renders_identically_across_backends() {
        for (dir, (tag, g)) in [Direction::LeftRight, Direction::RightLeft]
            .into_iter()
            .flat_map(|d| corpus().into_iter().map(move |c| (d, c)))
        {
            let mut config = LayoutConfig::standard();
            config.direction = dir;
            let heap_ir = lr_rl(&g, dir);
            let mut csr_buf = vec![0u8; g.estimate_csr_arena_size()];
            let mut csr_arena = Arena::new(&mut csr_buf);
            let csr = g.to_csr(&mut csr_arena).expect("CSR conversion");
            let size = g.estimate_layout_arena_size_with(&config);
            let mut temp_buf = vec![0u8; size];
            let mut out_buf = vec![0u8; size];
            let mut temp_arena = Arena::new(&mut temp_buf);
            let mut out_arena = Arena::new(&mut out_buf);
            // The PUBLIC entry point (see the field-parity twin).
            let csr_ir = csr
                .compute_layout_arena(&config, &mut temp_arena, &mut out_arena)
                .expect("CSR LR layout");

            let plain = RenderOptions::plain();
            let mut heap_out = String::new();
            heap_ir.render_with(&plain, &mut heap_out).expect("heap");
            let mut csr_out = String::new();
            csr_ir.render_with(&plain, &mut csr_out).expect("csr");
            assert_same(
                &format!("{tag} {dir:?} (heap vs csr, plain)"),
                &heap_out,
                &csr_out,
            );

            let colored = RenderOptions::colored(Palette::Ansi);
            let mut heap_c = String::new();
            heap_ir
                .render_with(&colored, &mut heap_c)
                .expect("heap colored");
            let mut csr_c = String::new();
            csr_ir
                .render_with(&colored, &mut csr_c)
                .expect("csr colored");
            assert_same(
                &format!("{tag} {dir:?} (heap vs csr, colored)"),
                &heap_c,
                &csr_c,
            );
        }
    }

    /// Hand-built X-flow IR: nodes on one row, an edge with the given
    /// path, reversed as asked. Exercises the public compositor paths
    /// the layout does not currently emit.
    fn x_flow_ir(
        path: crate::ir::EdgePath,
        reversed: bool,
        w: usize,
        h: usize,
        to_y: usize,
    ) -> LayoutIR<'static> {
        use crate::ir::{FlowAxis, LayoutEdge, LayoutIRBuilder, LayoutNode, NodeKind};
        let mut b = LayoutIRBuilder::new();
        b.set_dimensions(w, h);
        for (i, (id, label, x, y)) in [(1usize, "a", 0usize, 0usize), (2, "b", 12, to_y)]
            .into_iter()
            .enumerate()
        {
            b.add_node(LayoutNode {
                id,
                label,
                x,
                y,
                width: 3,
                height: 1,
                center_x: x + 1,
                center_y: y,
                level: i,
                level_position: 0,
                kind: NodeKind::Explicit,
                has_self_loop: false,
                self_loop_at: None,
                edge_index: None,
                content_tag: 0,
            });
        }
        b.add_edge(LayoutEdge {
            from_id: 1,
            to_id: 2,
            from_x: 2,
            from_y: 0,
            to_x: 12,
            to_y,
            path,
            flow_axis: FlowAxis::X,
            edge_index: 0,
            label: None,
            label_x: 0,
            label_y: 0,
            directed: true,
            reversed,
        });
        b.build()
    }

    fn cell_at(out: &str, x: usize, y: usize) -> char {
        out.lines()
            .nth(y)
            .and_then(|l| l.chars().nth(x))
            .unwrap_or(' ')
    }

    /// Slices review [P1]: when the bend sits IMMEDIATELY past the
    /// source face, the source-side marker has no trunk cell of its
    /// own — it must take the corner cell rather than vanish. Pinned
    /// per path variant on reversed edges (where the marker is the
    /// arrow showing the original direction).
    #[test]
    fn x_adjacent_bend_keeps_the_source_marker() {
        use crate::ir::EdgePath;
        // Corner bending one column past the source face (x = 3).
        let ir = x_flow_ir(EdgePath::Corner { bend_at: 3 }, true, 20, 6, 3);
        let out = render_plain(&ir, &RenderOptions::plain());
        assert_eq!(
            cell_at(&out, 3, 0),
            '⇠',
            "reversed Corner keeps its source marker at the adjacent bend:\n{out}"
        );

        // MultiSegment with start_offset == 0 — same adjacency.
        let ir = x_flow_ir(
            EdgePath::MultiSegment {
                waypoints: alloc::vec![(3, 3)],
                start_offset: 0,
            },
            true,
            20,
            8,
            3,
        );
        let out = render_plain(&ir, &RenderOptions::plain());
        assert_eq!(
            cell_at(&out, 3, 0),
            '⇠',
            "reversed MultiSegment keeps its source marker:\n{out}"
        );
    }

    /// Slices review [P1]: `SideChannel` is public and JSON-roundtrippable,
    /// so an X-flow one must actually paint — through the compositor AND
    /// the plan (row spans, ink enumeration, hit-testing, banding). The
    /// channel row sits outside the endpoint rows and the cap is 1, so
    /// this fixture exercises all four at once.
    #[test]
    fn x_side_channel_paints_and_hits() {
        use crate::render::engine::HitResult;
        let ir = x_flow_ir(
            crate::ir::EdgePath::SideChannel {
                channel_at: 4,
                span_start: 5,
                span_end: 9,
            },
            false,
            20,
            8,
            0,
        );
        let options = RenderOptions::plain();
        let out = render_plain(&ir, &options);
        assert!(
            out.lines().nth(4).is_some_and(|l| l.contains('─')),
            "the far channel row paints:\n{out}"
        );
        assert_eq!(
            cell_at(&out, 11, 0),
            '→',
            "arrowhead into the target:\n{out}"
        );
        // Banding must not lose it: cap 1 forces a band per row.
        let mut capped = options;
        capped.band_rows_cap = 1;
        assert_same("x side-channel banding", &out, &render_plain(&ir, &capped));
        // And the channel ink hit-tests to its edge.
        let plan = ir.render_plan(&options);
        assert_eq!(
            ir.hit_test(&plan, 7, 4),
            HitResult::Edge(0),
            "channel-row ink belongs to the edge:\n{out}"
        );
    }

    fn lr_rl<'a>(g: &Graph<'a>, dir: Direction) -> LayoutIR<'a> {
        let mut cfg = LayoutConfig::standard();
        cfg.direction = dir;
        compute_layout_cfg::<Horizontal>(g, &cfg)
    }

    /// P4-S1: RL is the EXACT x-mirror of LR. The mirror is applied by
    /// hand here — not by calling `flip_horizontal` — so the assertion
    /// is independent of the code it checks: a missed field, a double
    /// flip, or a wiring gap all fail.
    #[test]
    fn rl_is_the_exact_x_mirror_of_lr() {
        for (tag, g) in corpus() {
            let a = lr_rl(&g, Direction::LeftRight);
            let b = lr_rl(&g, Direction::RightLeft);
            assert_exact_x_mirror(tag, &a, &b);
        }
    }

    /// Field-by-field x-mirror assertion between an LR and an RL layout
    /// of the same graph: canvas, node rects and centers, self-loop
    /// markers, edge endpoints and labels, every path variant including
    /// `MultiSegment` waypoints, and subgraph boxes. Shared with the
    /// routing-quality gate, which runs it over the stress tiers as well
    /// — a skewed waypoint must fail the mirror check, not just move a
    /// metric.
    pub(super) fn assert_exact_x_mirror(
        tag: &str,
        a: &crate::ir::LayoutIR<'_>,
        b: &crate::ir::LayoutIR<'_>,
    ) {
        use crate::ir::EdgePath;
        {
            let w = a.width();
            assert_eq!(
                (a.width(), a.height()),
                (b.width(), b.height()),
                "{tag}: canvas"
            );
            // A cell mirrors to `w - 1 - x`; a SPAN of `n` cells
            // starting at `x` mirrors to `w - x - n`. The span form
            // saturates, matching the flip's contract: a label wider
            // than the whole canvas (the `labels` fixture) has no
            // meaningful mirror and both sides clamp to 0. `cell` is
            // deliberately NOT saturating — a coordinate outside the
            // canvas should fail this test, not be papered over.
            let cell = |x: usize| w - 1 - x;
            let span = |x: usize, n: usize| w.saturating_sub(x + n);
            assert_eq!(a.nodes().len(), b.nodes().len(), "{tag}: node count");
            for (p, q) in a.nodes().iter().zip(b.nodes().iter()) {
                assert_eq!(p.id, q.id, "{tag}: node order");
                assert_eq!(q.x, span(p.x, p.width), "{tag}: node {} x", p.id);
                assert_eq!(
                    (q.y, q.width, q.height),
                    (p.y, p.width, p.height),
                    "{tag}: node {}",
                    p.id
                );
                assert_eq!(
                    q.center_x,
                    cell(p.center_x),
                    "{tag}: node {} center_x",
                    p.id
                );
                assert_eq!(q.center_y, p.center_y, "{tag}: node {} center_y", p.id);
                match (p.self_loop_at, q.self_loop_at) {
                    (Some((px, py)), Some((qx, qy))) => {
                        assert_eq!((qx, qy), (cell(px), py), "{tag}: node {} marker", p.id);
                        // …and it lands on the flipped node's leading
                        // (right) column — the D5 role rule under RtL.
                        assert_eq!(qx, q.x + q.width - 1, "{tag}: node {} marker side", p.id);
                    }
                    (None, None) => {}
                    _ => panic!("{tag}: node {} marker presence differs", p.id),
                }
            }

            assert_eq!(a.edges().len(), b.edges().len(), "{tag}: edge count");
            for (p, q) in a.edges().iter().zip(b.edges().iter()) {
                assert_eq!(
                    (q.from_id, q.to_id),
                    (p.from_id, p.to_id),
                    "{tag}: edge order"
                );
                assert_eq!(
                    q.flow_axis, p.flow_axis,
                    "{tag}: flow_axis is mirror-invariant"
                );
                assert_eq!(
                    q.from_x,
                    cell(p.from_x),
                    "{tag}: edge {} from_x",
                    p.edge_index
                );
                assert_eq!(q.to_x, cell(p.to_x), "{tag}: edge {} to_x", p.edge_index);
                assert_eq!(
                    (q.from_y, q.to_y),
                    (p.from_y, p.to_y),
                    "{tag}: edge {} rows",
                    p.edge_index
                );
                if let Some(text) = p.label {
                    let n = text.chars().count() + 2;
                    assert_eq!(
                        q.label_x,
                        span(p.label_x, n),
                        "{tag}: edge {} label_x",
                        p.edge_index
                    );
                    assert_eq!(q.label_y, p.label_y, "{tag}: edge {} label_y", p.edge_index);
                } else {
                    assert_eq!(q.label_x, p.label_x, "{tag}: unlabeled label_x untouched");
                }
                match (&p.path, &q.path) {
                    (EdgePath::Direct, EdgePath::Direct) => {}
                    (EdgePath::Corner { bend_at: pb }, EdgePath::Corner { bend_at: qb }) => {
                        // X trunks: the bend line is a COLUMN.
                        assert_eq!(*qb, cell(*pb), "{tag}: edge {} bend", p.edge_index);
                    }
                    (
                        EdgePath::MultiSegment {
                            waypoints: pw,
                            start_offset: po,
                        },
                        EdgePath::MultiSegment {
                            waypoints: qw,
                            start_offset: qo,
                        },
                    ) => {
                        assert_eq!(qo, po, "{tag}: start_offset is flow-relative");
                        assert_eq!(pw.len(), qw.len(), "{tag}: waypoint count");
                        for (&(px, py), &(qx, qy)) in pw.iter().zip(qw.iter()) {
                            assert_eq!((qx, qy), (cell(px), py), "{tag}: waypoint");
                        }
                    }
                    (x, y) => panic!("{tag}: path variant changed: {x:?} vs {y:?}"),
                }
            }

            assert_eq!(a.subgraphs().len(), b.subgraphs().len(), "{tag}: box count");
            for (p, q) in a.subgraphs().iter().zip(b.subgraphs().iter()) {
                assert_eq!(q.x, span(p.x, p.width), "{tag}: box {} x", p.id);
                assert_eq!(
                    (q.y, q.width, q.height),
                    (p.y, p.width, p.height),
                    "{tag}: box {}",
                    p.id
                );
            }
        }
    }

    /// P4-S1: the flip is TOTAL over path variants the layout never
    /// emits but the public IR accepts. Hand-built X `SideChannel`
    /// and `Spline` edges, mirrored and checked field by field —
    /// `span_start`/`span_end` carry source/target roles and must
    /// mirror IN PLACE (swapping them would wire the source to the
    /// target's leg), and `channel_at` is a ROW that an x-flip leaves
    /// alone.
    #[test]
    fn flip_horizontal_is_total_over_path_variants() {
        use crate::ir::EdgePath;
        let w = 20;
        let mut ir = x_flow_ir(
            EdgePath::SideChannel {
                channel_at: 4,
                span_start: 5,
                span_end: 9,
            },
            false,
            w,
            8,
            0,
        );
        ir.flip_horizontal();
        let EdgePath::SideChannel {
            channel_at,
            span_start,
            span_end,
        } = ir.edges()[0].path
        else {
            panic!("variant preserved");
        };
        assert_eq!(channel_at, 4, "the channel line is a row — untouched");
        assert_eq!(span_start, w - 1 - 5, "source anchor mirrors in place");
        assert_eq!(span_end, w - 1 - 9, "target anchor mirrors in place");

        let mut ir = x_flow_ir(
            EdgePath::Spline {
                cp1_x: 4,
                cp1_y: 1,
                cp2_x: 9,
                cp2_y: 2,
            },
            false,
            w,
            8,
            0,
        );
        ir.flip_horizontal();
        let EdgePath::Spline {
            cp1_x,
            cp1_y,
            cp2_x,
            cp2_y,
        } = ir.edges()[0].path
        else {
            panic!("variant preserved");
        };
        assert_eq!((cp1_x, cp1_y), (w - 1 - 4, 1));
        assert_eq!((cp2_x, cp2_y), (w - 1 - 9, 2));

        // Involution: flipping twice restores the original.
        let mut ir = x_flow_ir(
            EdgePath::SideChannel {
                channel_at: 4,
                span_start: 5,
                span_end: 9,
            },
            false,
            w,
            8,
            0,
        );
        let before = alloc::format!("{:?}", ir.edges()[0]);
        ir.flip_horizontal();
        ir.flip_horizontal();
        assert_eq!(
            alloc::format!("{:?}", ir.edges()[0]),
            before,
            "the horizontal flip is involutive"
        );
    }

    /// The ARENA twin of the totality check. The layout never emits
    /// `SideChannel` or `Spline`, so those builder branches are
    /// unreachable through generated layouts — only a hand-built IR
    /// exercises them. Same rules as the heap flip (anchors mirror in
    /// place, the channel line is a row and stays put), plus
    /// involution over the raw arena fields.
    #[test]
    fn flip_horizontal_is_total_over_arena_path_variants() {
        use crate::ir::arena::{EdgePathArena, LayoutEdgeArena, LayoutIRArenaBuilder};
        let w = 20;
        let edge = |path| LayoutEdgeArena {
            from_id: 1,
            to_id: 2,
            from_x: 2,
            from_y: 0,
            to_x: 12,
            to_y: 0,
            directed: true,
            reversed: false,
            path,
            flow_axis: crate::ir::FlowAxis::X,
            edge_index: 0,
            label_offset: 0,
            label_len: 0,
            label_x: 0,
            label_y: 0,
            min_y: 0,
            max_y: 4,
        };

        // The flip is a pre-build pass, so build once per flip count
        // and compare the resulting arena fields.
        fn path_after(
            w: usize,
            path: EdgePathArena,
            edge: impl Fn(EdgePathArena) -> LayoutEdgeArena,
            times: usize,
        ) -> String {
            let mut buf = vec![0u8; 64 * 1024];
            let mut arena = Arena::new(&mut buf);
            let mut b = LayoutIRArenaBuilder::new(&mut arena, 2, 1, 4, 16, 2).expect("builder");
            b.set_dimensions(w, 8);
            b.add_edge(edge(path)).expect("edge");
            for _ in 0..times {
                b.flip_horizontal();
            }
            alloc::format!("{:?}", b.build().edge(0).path)
        }

        for path in [
            EdgePathArena::SideChannel {
                channel_at: 4,
                span_start: 5,
                span_end: 9,
            },
            EdgePathArena::Spline {
                cp1_x: 4,
                cp1_y: 1,
                cp2_x: 9,
                cp2_y: 2,
            },
        ] {
            let once = path_after(w, path, edge, 1);
            assert_eq!(
                path_after(w, path, edge, 0),
                path_after(w, path, edge, 2),
                "arena flip is involutive: {path:?}"
            );
            match path {
                EdgePathArena::SideChannel { .. } => {
                    assert!(
                        once.contains("channel_at: 4"),
                        "the channel line is a row — untouched: {once}"
                    );
                    assert!(
                        once.contains(&alloc::format!("span_start: {}", w - 1 - 5)),
                        "source anchor mirrors in place: {once}"
                    );
                    assert!(
                        once.contains(&alloc::format!("span_end: {}", w - 1 - 9)),
                        "target anchor mirrors in place: {once}"
                    );
                }
                EdgePathArena::Spline { .. } => {
                    assert!(
                        once.contains(&alloc::format!("cp1_x: {}", w - 1 - 4)),
                        "{once}"
                    );
                    assert!(
                        once.contains(&alloc::format!("cp2_x: {}", w - 1 - 9)),
                        "{once}"
                    );
                    assert!(
                        once.contains("cp1_y: 1") && once.contains("cp2_y: 2"),
                        "rows are untouched by an x-flip: {once}"
                    );
                }
                _ => unreachable!(),
            }
        }
    }

    /// P4-S1: label PLACEMENT mirrors exactly — the planner's chosen
    /// span under RL is the mirror of its LR span, at odd AND even
    /// label widths (naive `len / 2` centering drifts one cell on
    /// even widths, and the float's midpoint on odd gaps).
    #[test]
    fn label_spans_mirror_exactly() {
        for text in ["go", "abc", "four", "fives"] {
            let mut g = Graph::new();
            g.add_node(1, "a");
            g.add_node(2, "b");
            g.add_node(3, "c");
            g.add_edge(1, 2, Some(text));
            g.add_edge(1, 3, None);
            let len = text.chars().count() + 2;
            let options = RenderOptions::plain();

            let lr_ir = lr_rl(&g, Direction::LeftRight);
            let rl_ir = lr_rl(&g, Direction::RightLeft);
            let w = lr_ir.width();
            assert_eq!(w, rl_ir.width(), "{text}: canvas width");

            let lr_plan = lr_ir.render_plan(&options);
            let rl_plan = rl_ir.render_plan(&options);
            let pick = |p: &crate::render::engine::RenderPlan<'_>| {
                p.labels()
                    .iter()
                    .find(|l| l.edge_index == 0)
                    .map(|l| (l.x, l.y, l.placeable))
                    .expect("label plan")
            };
            let (lx, ly, lp) = pick(&lr_plan);
            let (rx, ry, rp) = pick(&rl_plan);
            assert_eq!(lp, rp, "{text}: placement decision mirrors");
            if lp {
                assert_eq!(ry, ly, "{text}: label row is mirror-invariant");
                assert_eq!(
                    rx,
                    w - lx - len,
                    "{text} (len {len}): RL span is the exact mirror of the LR span"
                );
            }
        }
    }

    /// P4-S1: an RL graph renders right-to-left — the first level sits
    /// at the RIGHT, arrowheads point left, and the self-loop marker
    /// moves to the node's right side.
    #[test]
    fn rl_renders_right_to_left() {
        let mut g = Graph::new();
        g.add_node(1, "a");
        g.add_node(2, "b");
        g.add_edge(1, 2, None);
        g.add_edge(1, 1, None);
        let ir = lr_rl(&g, Direction::RightLeft);
        let out = render_plain(&ir, &RenderOptions::plain());
        let row = out
            .lines()
            .find(|l| l.contains("[a]"))
            .expect("row with [a]");
        let a = row.find("[a]").expect("a");
        let b = row.find("[b]").expect("same row");
        assert!(b < a, "target left of source under RtL: {row:?}\n{out}");
        assert!(row.contains('←'), "arrowheads point left: {row:?}\n{out}");
        let n = ir.nodes().iter().find(|n| n.id == 1).expect("node a");
        assert_eq!(
            n.self_loop_at,
            Some((n.x + n.width - 1, n.y + n.height)),
            "marker on the leading (right) side under RtL:\n{out}"
        );
    }

    /// P3-S3: the LR two-node-cycle presentation — the anti-parallel
    /// pair shares its trunk row with BOTH arrowheads visible (`⇠` at
    /// the source face, `→` at the target face); the solid trunk wins
    /// the interior over the dashed back edge. Trunk-lane separation
    /// was examined and deliberately not built: it would cost
    /// cross-axis lane reservation machinery for a pair that is
    /// already legible.
    #[test]
    fn lr_two_cycle_shares_trunk_with_both_arrowheads() {
        let ir = lr(&two_cycle());
        let out = render_plain(&ir, &RenderOptions::plain());
        let row = out
            .lines()
            .find(|l| l.contains('→') || l.contains('⇢'))
            .expect("trunk row");
        assert!(
            row.contains('⇠') || row.contains('←'),
            "back arrowhead on the shared trunk: {row:?}\n{out}"
        );
        // Tight gaps render as adjacent arrowheads (`[A]⇠→[B]`); wider
        // gaps fill the interior with the solid trunk. Both are the
        // intended presentation.
        let back = row.find(['⇠', '←']).unwrap();
        let fwd = row.find(['→', '⇢']).unwrap();
        assert!(back < fwd, "back arrow at the source face: {row:?}");
    }
}

/// Layout-quality scoring.
///
/// Not an invariant suite — a measuring tape. Routing changes are easy
/// to judge by eye on one graph in one direction and easy to get wrong
/// everywhere else, so this prints one table over the whole corpus in
/// all four directions. A theory is accepted or rejected on the table,
/// not on how the hero example looks.
///
/// ```text
/// cargo test --lib --all-features quality_table -- --ignored --nocapture
/// ```
#[cfg(all(feature = "layout-vertical", feature = "layout-horizontal"))]
mod quality {
    use super::lr_invariants::corpus;
    use super::*;
    use crate::graph::Direction;
    use crate::ir::{EdgePath, FlowAxis, LayoutEdge, LayoutIR};

    /// Cross-axis coordinate of a physical point, for this edge's trunk.
    fn cross(e: &LayoutEdge<'_>, x: usize, y: usize) -> usize {
        match e.flow_axis {
            FlowAxis::X => y,
            FlowAxis::Y => x,
        }
    }

    /// How many times the edge changes lane on the cross axis.
    ///
    /// This is the number a reader perceives as "kinks". A straight
    /// edge scores 0 however long it is; the staircase that motivated
    /// this work scores 5.
    fn lane_changes(e: &LayoutEdge<'_>) -> usize {
        match &e.path {
            EdgePath::Direct => 0,
            EdgePath::Corner { .. } => 1,
            EdgePath::SideChannel { .. } => 2,
            EdgePath::Spline { .. } => 1,
            EdgePath::MultiSegment { waypoints, .. } => {
                let mut seq = Vec::with_capacity(waypoints.len() + 2);
                seq.push(cross(e, e.from_x, e.from_y));
                for &(wx, wy) in waypoints.iter() {
                    seq.push(cross(e, wx, wy));
                }
                seq.push(cross(e, e.to_x, e.to_y));
                seq.windows(2).filter(|w| w[0] != w[1]).count()
            }
        }
    }

    /// Cross-axis **overshoot**: how far the edge wanders outside the band
    /// spanned by its own endpoints.
    ///
    /// Defined as overshoot rather than total cross span, because the span
    /// between two endpoints is unavoidable geometry — an edge joining row 0
    /// to row 20 is not badly routed for covering 20 rows. What this work
    /// cares about is an edge leaving the band it needs at all: hero's
    /// `trace` joins two nodes on row 0 and reaches row 20, which is pure
    /// excursion.
    ///
    /// Every path variant is measured. `Direct` and `Corner` are 0 by
    /// construction: `bend_at` is a LEVEL-axis line, so a corner's cross
    /// segment runs between the two endpoints and cannot leave the band.
    /// `channel_at` by contrast IS a cross-axis line, so `SideChannel` can.
    fn spread(e: &LayoutEdge<'_>) -> usize {
        let a = cross(e, e.from_x, e.from_y);
        let b = cross(e, e.to_x, e.to_y);
        let (lo, hi) = (a.min(b), a.max(b));
        // Distance outside [lo, hi]; 0 when inside, both branches saturating.
        let over = |c: usize| lo.saturating_sub(c).max(c.saturating_sub(hi));
        match &e.path {
            EdgePath::Direct | EdgePath::Corner { .. } => 0,
            EdgePath::SideChannel { channel_at, .. } => over(*channel_at),
            EdgePath::MultiSegment { waypoints, .. } => waypoints
                .iter()
                .map(|&(wx, wy)| over(cross(e, wx, wy)))
                .max()
                .unwrap_or(0),
            EdgePath::Spline {
                cp1_x,
                cp1_y,
                cp2_x,
                cp2_y,
            } => over(cross(e, *cp1_x, *cp1_y)).max(over(cross(e, *cp2_x, *cp2_y))),
        }
    }

    /// Stroke-crosses-stroke cells in the painted output.
    ///
    /// A proxy, deliberately: it counts what the reader sees as a
    /// crossing. Border junctions (`╪ ╫ ╬`) are excluded — an edge
    /// crossing a cluster wall is expected and not a routing fault.
    fn crossings(text: &str) -> usize {
        text.chars()
            .filter(|c| matches!(c, '┼' | '┿' | '╂' | '+'))
            .count()
    }

    fn score(ir: &LayoutIR<'_>) -> (usize, usize, usize, usize) {
        let text = ir.render_string(&RenderOptions::plain());
        let kinks: usize = ir.edges().iter().map(lane_changes).sum();
        let worst = ir.edges().iter().map(spread).max().unwrap_or(0);
        (crossings(&text), kinks, worst, ir.width() * ir.height())
    }

    /// Mirror-invariant metric parity: `RightLeft` and `LeftRight` must
    /// agree on every quantity the acceptance gate reads.
    ///
    /// This is a *necessary* condition, not a sufficient one — two
    /// different layouts can share all six numbers. `rl_is_an_exact_
    /// reflection_of_lr` below checks the real invariant. Both exist
    /// because the first P3 prototype produced tier4 LR 20 vs RL 19 at
    /// identical dimensions, and nothing then present caught it.
    #[test]
    fn lr_and_rl_agree_on_every_scored_metric() {
        for (name, g) in mirror_corpus() {
            let scored = |dir| {
                let mut cfg = LayoutConfig::standard();
                cfg.direction = dir;
                let ir = g.compute_layout_with_config(&cfg);
                (score(&ir), ir.width(), ir.height())
            };
            assert_eq!(
                scored(Direction::LeftRight),
                scored(Direction::RightLeft),
                "{name}: RL must match LR on cross, kinks, spread, area, w, h"
            );
        }
    }

    /// The actual invariant: `RightLeft` is `LeftRight` reflected on x —
    /// checked field by field via `lr_invariants::assert_exact_x_mirror`
    /// (canvas, node rects, self-loop markers, edge endpoints, labels,
    /// every path variant *including `MultiSegment` waypoints*, subgraph
    /// boxes) over the corpus AND the stress tiers.
    ///
    /// The metric gate above is necessary but not sufficient — two
    /// different layouts can share all six numbers, and a placement pass
    /// that skews waypoints moves no metric this fixture set happens to
    /// pin. This one fails on the first skewed coordinate.
    ///
    /// Canvas extent is the subtle part: `flip_horizontal` reflects
    /// around the canvas width, so any placement that widens the canvas
    /// without the flip seeing it yields a *skewed* RL rather than a
    /// mirrored one. That is the leading explanation for the first P3
    /// prototype's tier4 LR 20 / RL 19 asymmetry.
    #[test]
    fn rl_is_an_exact_reflection_of_lr() {
        for (name, g) in mirror_corpus() {
            let build = |dir| {
                let mut cfg = LayoutConfig::standard();
                cfg.direction = dir;
                g.compute_layout_with_config(&cfg)
            };
            let lr = build(Direction::LeftRight);
            let rl = build(Direction::RightLeft);
            super::lr_invariants::assert_exact_x_mirror(name, &lr, &rl);
        }
    }

    /// Corpus fixtures plus the clustered stress tiers.
    fn mirror_corpus() -> Vec<(&'static str, Graph<'static>)> {
        let mut v: Vec<(&'static str, Graph<'static>)> = corpus().into_iter().collect();
        v.push(("tier1_micro", tier1_microservices()));
        v.push(("tier2_plat", tier2_platform()));
        v.push(("tier3_cloud", tier3_cloud()));
        v.push(("tier4_ent", tier4_enterprise()));
        v.push(("tier5_mega", tier5_megacorp()));
        v.push(("label_stress", label_stress()));
        v.push(("disc_eq", disc_equal_width()));
        v.push(("disc_mixed", disc_mixed_width()));
        v
    }

    /// 0.10.3 cluster-overlap fix: disconnected cluster members — no
    /// edges at all, so
    /// every connected-neighbor pass skips them and compaction's clamp
    /// artifacts survive unless repaired. Equal widths are the minimal
    /// overlap trigger.
    fn disc_equal_width() -> Graph<'static> {
        let mut g = Graph::new();
        g.add_node(1usize, "x");
        g.add_node(2usize, "x");
        g.add_node(3usize, "x");
        let sg = g.add_subgraph("Pool");
        g.put_nodes(&[1, 2, 3]).inside(sg).unwrap();
        g
    }

    /// The mixed-width variant of [`disc_equal_width`] (same 0.10.3
    /// cluster-overlap fix).
    fn disc_mixed_width() -> Graph<'static> {
        let mut g = Graph::new();
        g.add_node(1usize, "xxx");
        g.add_node(2usize, "xxxxxxxxxxxxxxxxxxx");
        g.add_node(3usize, "xxx");
        g.add_node(4usize, "x");
        let sg = g.add_subgraph("Pool");
        g.put_nodes(&[1, 2, 3, 4]).inside(sg).unwrap();
        g
    }

    /// 0.10.3 invariant: no two real nodes may ever overlap — any fixture,
    /// any direction, either backend. `repair_level_overlaps` (and its
    /// CSR twin) is the enforcement; this is the corpus-wide gate that
    /// keeps it honest, including on the disconnected-cluster fixtures
    /// no other pass can repair.
    #[test]
    fn corpus_nodes_never_overlap_any_direction_both_backends() {
        let assert_no_overlap =
            |name: &str, backend: &str, dir: Direction, rects: &[(usize, usize, usize, usize)]| {
                for i in 0..rects.len() {
                    for j in i + 1..rects.len() {
                        let (ax, ay, aw, ah) = rects[i];
                        let (bx, by, bw, bh) = rects[j];
                        assert!(
                            !(ax < bx + bw && bx < ax + aw && ay < by + bh && by < ay + ah),
                            "{name} {backend} {dir:?}: node rect {i} ({ax},{ay} {aw}x{ah}) \
                         overlaps node rect {j} ({bx},{by} {bw}x{bh})"
                        );
                    }
                }
            };
        for (name, g) in mirror_corpus() {
            for dir in [
                Direction::TopDown,
                Direction::BottomUp,
                Direction::LeftRight,
                Direction::RightLeft,
            ] {
                let mut cfg = LayoutConfig::standard();
                cfg.direction = dir;

                let heap_ir = g.compute_layout_with_config(&cfg);
                let rects: Vec<(usize, usize, usize, usize)> = heap_ir
                    .nodes()
                    .iter()
                    .map(|n| (n.x, n.y, n.width, n.height))
                    .collect();
                assert_no_overlap(name, "heap", dir, &rects);

                let mut csr_buf = vec![0u8; g.estimate_csr_arena_size() * 2];
                let mut csr_arena = Arena::new(&mut csr_buf);
                let csr = g.to_csr(&mut csr_arena).expect("CSR conversion");
                let size = (g.estimate_layout_arena_size() * 2).max(256 * 1024);
                let mut temp_buf = vec![0u8; size];
                let mut out_buf = vec![0u8; size];
                let mut temp_arena = Arena::new(&mut temp_buf);
                let mut out_arena = Arena::new(&mut out_buf);
                let ir = csr
                    .compute_layout_arena(&cfg, &mut temp_arena, &mut out_arena)
                    .expect("CSR layout");
                let rects: Vec<(usize, usize, usize, usize)> = ir
                    .nodes()
                    .iter()
                    .map(|n| (n.x, n.y, n.width, n.height))
                    .collect();
                assert_no_overlap(name, "csr", dir, &rects);
            }
        }
    }

    /// temp/11 P0: the label-placement stress fixture. The rest of the
    /// corpus concentrates label signal in `hero` and `labels`; this one
    /// exercises every placement-relevant shape in one graph — labeled
    /// corners, a labeled skip chain, a labeled reversed edge, even- and
    /// odd-length labels, and clusters whose borders sit next to the
    /// natural label spots. Labels are unique strings so the scorer can
    /// count placements from rendered text.
    fn label_stress() -> Graph<'static> {
        let mut g = Graph::new();
        for (id, name) in [
            (1, "In"),
            (2, "Split"),
            (3, "Alpha"),
            (4, "Beta"),
            (5, "Gamma"),
            (6, "Join"),
            (7, "Out"),
            (8, "Side"),
        ] {
            g.add_node(id, name);
        }
        g.add_edge(1, 2, Some("go")); // even, short, Direct
        g.add_edge(2, 3, Some("left")); // even, Corner
        g.add_edge(2, 4, Some("mid")); // odd, Corner
        g.add_edge(2, 5, Some("right")); // odd, Corner
        g.add_edge(3, 6, Some("a-in")); // even, Corner into cluster
        g.add_edge(4, 6, Some("b-in")); // even
        g.add_edge(5, 6, Some("c-in")); // even
        g.add_edge(6, 7, Some("done")); // even, Direct
        g.add_edge(2, 7, Some("express")); // odd, skip chain (3 levels)
        g.add_edge(1, 8, Some("spur")); // even
        g.add_edge(8, 7, Some("rejoin")); // even, skip chain
        g.add_edge(7, 2, Some("undo")); // even, reversed (back edge)

        let core = g.add_subgraph("Core");
        g.put_nodes(&[3usize, 4, 5]).inside(core).expect("core");
        let tail = g.add_subgraph("Tail");
        g.put_nodes(&[6usize]).inside(tail).expect("tail");
        g
    }

    /// Strip ANSI SGR escapes so quoted labels can be counted in colored
    /// output.
    fn strip_ansi(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\u{1b}' {
                for d in chars.by_ref() {
                    if d == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    /// temp/11 P0 metric: labels placed inline, on both render surfaces.
    ///
    /// `eligible` excludes self-loop labels (no placement host exists —
    /// tracked separately in temp/11 §8). PLAIN placement is counted
    /// from the rendered text with the legend off, so an unplaced label
    /// simply does not appear: what the user sees is what is measured
    /// (duplicate label strings count up to their edge multiplicity).
    /// COLORED placement is the colored+legend surface — the one place
    /// the compositor consults the stricter `placed_colored()` decision
    /// (whole-row node veto) — counted from the `RenderPlan` directly,
    /// because that surface's rendered text necessarily contains the
    /// legend, whose entries would masquerade as inline placements.
    fn label_counts(ir: &LayoutIR<'_>) -> (usize, usize, usize) {
        use alloc::collections::BTreeMap;
        let mut expected: BTreeMap<&str, usize> = BTreeMap::new();
        for e in ir.edges().iter() {
            if e.from_id == e.to_id {
                continue;
            }
            if let Some(l) = e.label {
                *expected.entry(l).or_insert(0) += 1;
            }
        }
        let eligible: usize = expected.values().sum();

        let placed_in = |text: &str| -> usize {
            expected
                .iter()
                .map(|(l, &n)| {
                    let needle = alloc::format!("\"{l}\"");
                    text.matches(needle.as_str()).count().min(n)
                })
                .sum()
        };

        // Legend off for the text count — legend entries would read as
        // inline placements.
        let mut popts = RenderOptions::plain();
        popts.legend = false;
        let plain = ir.render_string(&popts);
        let mut copts = RenderOptions::colored(Palette::Ansi);
        copts.legend = true;
        let plan = ir.render_plan(&copts);
        let colored = plan
            .labels()
            .iter()
            .filter(|l| {
                let e = &ir.edges()[l.edge_index];
                e.from_id != e.to_id && l.placed_colored()
            })
            .count();
        (eligible, placed_in(&plain), colored)
    }

    /// §4.4 pin: the farthest-travelling chain takes the outer track.
    ///
    /// Allocation order and spatial order are opposite — chains are
    /// allocated shortest-first precisely so each longer chain is pushed
    /// outside the ones already placed. This relationship was silently
    /// inverted once (the first P3 prototype placed longest-first and got
    /// `fan → longest → shortest`), so it is pinned on real layout output:
    /// two skip chains from one source, the longer must end outermost at
    /// every shared level, in both an axis-vertical and an axis-horizontal
    /// direction.
    #[test]
    fn longer_chain_takes_the_outer_track() {
        let mut g = Graph::new();
        for id in 1..=5 {
            g.add_node(id, "n");
        }
        for (a, b) in [(1usize, 2usize), (2, 3), (3, 4), (4, 5)] {
            g.add_edge(a, b, None);
        }
        g.add_edge(1, 4, None); // span 3 — allocated first
        g.add_edge(1, 5, None); // span 4 — allocated last, ends outermost

        for dir in [Direction::TopDown, Direction::LeftRight] {
            let mut cfg = LayoutConfig::standard();
            cfg.direction = dir;
            cfg.include_dummy_nodes = true;
            let ir = g.compute_layout_with_config(&cfg);

            let chain = |edge: usize| -> Vec<(usize, usize)> {
                let mut v: Vec<(usize, usize)> = ir
                    .nodes()
                    .iter()
                    .filter(|n| n.kind == crate::ir::NodeKind::Dummy)
                    .filter(|n| n.edge_index == Some(edge))
                    .map(|n| match dir {
                        Direction::LeftRight => (n.x, n.y),
                        _ => (n.y, n.x),
                    })
                    .collect();
                v.sort_unstable();
                v
            };
            let short = chain(4);
            let long = chain(5);
            assert_eq!(short.len(), 2, "{dir:?}: span-3 chain has 2 waypoints");
            assert_eq!(long.len(), 3, "{dir:?}: span-4 chain has 3 waypoints");
            for &(lvl, sc) in &short {
                let &(_, lc) = long.iter().find(|&&(l, _)| l == lvl).expect("shared level");
                assert!(
                    lc > sc,
                    "{dir:?}: at level-coord {lvl} the longer chain must be \
                     outermost (long {lc} vs short {sc})"
                );
            }
        }
    }

    /// P6: the quality totals are a floor, not a report.
    ///
    /// `quality_table` (below, `#[ignore]`d) prints the full per-fixture
    /// breakdown; this asserts the corpus totals never regress past the
    /// values the chain-lane pass (temp/09 P3/P4) landed at. Improvements
    /// lower them — update the pins when they do. This is what stops the
    /// staircase routing being quietly re-introduced by a change nobody
    /// connects to routing: the determinism bug of `40df9a2` and the
    /// first P3 prototype's regressions would both have tripped it.
    #[test]
    fn quality_totals_never_regress() {
        const DIRS: [Direction; 4] = [
            Direction::TopDown,
            Direction::BottomUp,
            Direction::LeftRight,
            Direction::RightLeft,
        ];
        let (mut cross, mut kinks, mut spread, mut area) = (0usize, 0usize, 0usize, 0usize);
        for (_, g) in mirror_corpus() {
            for dir in DIRS {
                let mut cfg = LayoutConfig::standard();
                cfg.direction = dir;
                let ir = g.compute_layout_with_config(&cfg);
                let (c, k, s, a) = score(&ir);
                cross += c;
                kinks += k;
                spread += s;
                area += a;
            }
        }
        // Re-baselined at temp/11 P0 when `label_stress` joined the
        // corpus (previously 130/646/520 over 15 fixtures).
        assert!(cross <= 138, "corpus crossings regressed: {cross} > 138");
        assert!(kinks <= 700, "corpus kinks regressed: {kinks} > 700");
        assert!(spread <= 630, "corpus overshoot regressed: {spread} > 630");
        // temp/09's 5% ceiling plus label_stress's own baseline area.
        assert!(
            area <= 1_508_870,
            "corpus canvas area over budget: {area} > 1,508,870"
        );
    }

    /// Review #2: an EXACTLY estimate-sized arena must survive a cyclic
    /// graph — the estimator's original unflipped relaxation counted zero
    /// dummies for an ordered cycle, while cycle breaking turns the
    /// closing edge into a span-(N-1) chain. The estimator now mirrors
    /// cycle breaking, so EVERY dummy-derived manifest term is exact.
    ///
    /// Cases: small cycle (lane pass enabled), and a cycle past the lane
    /// work cap (`E > LANE_PASS_MAX_WORK` — lane pass DISABLED, so the
    /// estimate must be right for the base manifest alone), both with
    /// `include_dummy_nodes` so the emitted-dummy IR terms are exercised.
    fn cycle_fits_exact_arena(n: usize) {
        let mut g = Graph::new();
        for i in 0..n {
            g.add_node(i, "n");
        }
        for i in 0..n - 1 {
            g.add_edge(i, i + 1, None);
        }
        g.add_edge(n - 1, 0usize, None); // ordered cycle

        let mut cfg = LayoutConfig::standard();
        cfg.include_dummy_nodes = true;
        // Conversion arena gets headroom like the rest of the suite
        // (`csr_engine` doubles it too) — the exact-size contract under
        // test here is the LAYOUT estimator, not the conversion one.
        let mut csr_buf = vec![0u8; g.estimate_csr_arena_size() * 2];
        let mut csr_arena = Arena::new(&mut csr_buf);
        let csr = g.to_csr(&mut csr_arena).expect("CSR conversion");
        let est = g.estimate_layout_arena_size_with(&cfg);
        let mut temp_buf = vec![0u8; est];
        let mut out_buf = vec![0u8; est];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_arena = Arena::new(&mut out_buf);
        let ir = csr
            .compute_layout_arena(&cfg, &mut temp_arena, &mut out_arena)
            .expect("exactly estimate-sized arena must suffice, cycle included");
        assert!(ir.width() > 0 && ir.height() > 0);
        // The broken cycle's chain has N-2 waypoints; with
        // include_dummy_nodes they must ALL be present in the IR — the
        // old undercount silently dropped overflowing waypoints instead.
        let dummies = ir
            .nodes()
            .iter()
            .filter(|nd| nd.kind == crate::ir::NodeKind::Dummy)
            .count();
        assert_eq!(dummies, n - 2, "every broken-cycle waypoint emitted");
    }

    #[test]
    fn cyclic_graph_layout_fits_exactly_estimated_arena() {
        cycle_fits_exact_arena(8);
        cycle_fits_exact_arena(60);
    }

    /// Review #3 (representability boundary): a chain whose every
    /// admissible lane is blocked by a giant level obstacle must keep its
    /// packed routing — never a lane clamped into occupied space, and
    /// never a waypoint moved INTO the obstacle.
    #[test]
    fn oversized_obstacle_refuses_lane_instead_of_clamping() {
        use crate::algorithms::sugiyama::geometry::LANE_MAX_CROSS;
        let mut g = Graph::new();
        g.add_node(0usize, "src");
        // A node wider than the representable cross axis: every lane at
        // its level is either inside its body or beyond LANE_MAX_CROSS.
        g.add_node(
            1usize,
            crate::render::engine::CustomNode {
                label: "wall",
                width: LANE_MAX_CROSS + 8,
                height: 1,
                painter: None,
                payload: "",
            },
        );
        g.add_node(2usize, "dst");
        g.add_edge(0usize, 1usize, None);
        g.add_edge(1usize, 2usize, None);
        g.add_edge(0usize, 2usize, None); // skip chain past the wall

        let mut cfg = LayoutConfig::standard();
        cfg.include_dummy_nodes = true;
        let ir = g.compute_layout_with_config(&cfg);
        for nd in ir
            .nodes()
            .iter()
            .filter(|nd| nd.kind == crate::ir::NodeKind::Dummy)
        {
            let (wall_x, wall_w) = ir
                .nodes()
                .iter()
                .find(|w| w.id == 1)
                .map(|w| (w.x, w.width))
                .expect("wall present");
            let inside_wall = nd.x >= wall_x && nd.x < wall_x + wall_w;
            assert!(
                !inside_wall,
                "waypoint at x={} moved inside the wall [{}, {})",
                nd.x,
                wall_x,
                wall_x + wall_w
            );
        }
    }

    /// Review #4: the design requires NO crossing regression in any
    /// fixture/direction — an aggregate floor lets one cell regress while
    /// another improves. Every corpus cell is pinned at its landed value
    /// (TD pin covers BT, LR covers RL: exact mirrors, gated above).
    #[test]
    fn per_fixture_crossings_never_regress() {
        let pins: &[(&str, usize, usize)] = &[
            // (fixture, TD/BT ceiling, LR/RL ceiling)
            ("fan", 2, 2),
            ("stage", 0, 0),
            ("skip", 0, 0),
            ("back", 0, 0),
            ("two_cycle", 0, 0),
            ("self_loop", 0, 0),
            ("labels", 0, 0),
            ("nested", 0, 0),
            ("hero", 2, 1),
            ("wide_node", 0, 0),
            ("tier1_micro", 0, 0),
            ("tier2_plat", 1, 3),
            ("tier3_cloud", 7, 9),
            ("tier4_ent", 4, 8),
            ("tier5_mega", 14, 12),
            ("label_stress", 4, 0),
            ("disc_eq", 0, 0),
            ("disc_mixed", 0, 0),
        ];
        for (name, g) in mirror_corpus() {
            let &(_, td_pin, lr_pin) = pins
                .iter()
                .find(|(n, _, _)| *n == name)
                .unwrap_or_else(|| panic!("fixture {name} has no crossing pin — add one"));
            for (dir, pin) in [
                (Direction::TopDown, td_pin),
                (Direction::BottomUp, td_pin),
                (Direction::LeftRight, lr_pin),
                (Direction::RightLeft, lr_pin),
            ] {
                let mut cfg = LayoutConfig::standard();
                cfg.direction = dir;
                let ir = g.compute_layout_with_config(&cfg);
                let (c, _, _, _) = score(&ir);
                assert!(
                    c <= pin,
                    "{name} {dir:?}: crossings regressed to {c} (pin {pin})"
                );
            }
        }
    }

    /// temp/11 P1: placed labels must mirror exactly — LR↔RL on x,
    /// TD↔BT on y — including even-length labels, whose lead rounding
    /// is the classic way mirrors break. Positions are extracted from
    /// the rendered text (char-indexed), so this pins what users see,
    /// not an internal plan field.
    #[test]
    fn label_placements_mirror() {
        let find_labels = |text: &str| -> alloc::vec::Vec<(String, usize, usize)> {
            let mut out = alloc::vec::Vec::new();
            for (row, line) in text.lines().enumerate() {
                let chars: alloc::vec::Vec<char> = line.chars().collect();
                let mut c = 0usize;
                while c < chars.len() {
                    if chars[c] == '"' {
                        if let Some(end) = chars[c + 1..].iter().position(|&ch| ch == '"') {
                            let label: String = chars[c..=c + 1 + end].iter().collect();
                            out.push((label, row, c));
                            c += end + 2;
                            continue;
                        }
                    }
                    c += 1;
                }
            }
            out.sort();
            out
        };
        for (name, g) in mirror_corpus() {
            let render = |dir| {
                let mut cfg = LayoutConfig::standard();
                cfg.direction = dir;
                let ir = g.compute_layout_with_config(&cfg);
                // Legend off: this test parses quoted labels from the
                // text, and legend entries would read as placements.
                let mut opts = RenderOptions::plain();
                opts.legend = false;
                (ir.render_string(&opts), ir.width(), ir.height())
            };
            // LR ↔ RL: x-mirror. A label of char-length L starting at
            // column c mirrors to width − c − L.
            let (lr, w, _) = render(Direction::LeftRight);
            let (rl, _, _) = render(Direction::RightLeft);
            let mut want: alloc::vec::Vec<(String, usize, usize)> = find_labels(&lr)
                .into_iter()
                .map(|(l, r, c)| {
                    let n = l.chars().count();
                    (l, r, w - c - n)
                })
                .collect();
            want.sort();
            let got = find_labels(&rl);
            assert_eq!(want, got, "{name}: RL labels are not the x-mirror of LR");

            // TD ↔ BT: y-mirror over the canvas height — for labels
            // placed in BOTH directions. Box labels anchor to the
            // visual top in every direction (text is never mirrored —
            // D4), so a cell free in one direction can hold box text
            // in the other; a label squeezed out that way goes to the
            // legend there, and per-direction counts are pinned by the
            // floors test instead. Position symmetry is still exact
            // for the shared set.
            let (td, _, h) = render(Direction::TopDown);
            let (bt, _, _) = render(Direction::BottomUp);
            let want: alloc::vec::Vec<(String, usize, usize)> = find_labels(&td)
                .into_iter()
                .map(|(l, r, c)| (l, h - 1 - r, c))
                .collect();
            let got = find_labels(&bt);
            let mirrored: alloc::vec::Vec<_> = want
                .iter()
                .filter(|(l, _, _)| got.iter().any(|(gl, _, _)| gl == l))
                .cloned()
                .collect();
            let shared: alloc::vec::Vec<_> = got
                .iter()
                .filter(|(l, _, _)| want.iter().any(|(wl, _, _)| wl == l))
                .cloned()
                .collect();
            let mut mirrored = mirrored;
            let mut shared = shared;
            mirrored.sort();
            shared.sort();
            assert_eq!(
                mirrored, shared,
                "{name}: labels placed in both TD and BT are not y-mirrors"
            );
        }
    }

    /// temp/11 P4: per-fixture label floors, pinned at the values the
    /// sliding tier landed, on BOTH render surfaces — plain, and the
    /// colored+legend surface where the compositor consults
    /// `placed_colored()`'s whole-row node veto (colored ≤ plain by
    /// construction). Improvements raise them; update the pins when
    /// they do. `hero` LR sits at 6 of 8 by geometric proof: `read`'s
    /// every window covering its own ink crosses a box border, and
    /// `http`'s trunk is three cells on the canvas's first row — the
    /// legend is the designed fallback for exactly these two.
    #[test]
    fn per_fixture_labels_never_regress() {
        let pins: &[(&str, usize, usize, usize, usize)] = &[
            // (fixture, TD/BT plain, LR/RL plain, TD/BT colored, LR/RL colored)
            ("fan", 0, 0, 0, 0),
            ("stage", 1, 1, 1, 1),
            ("skip", 0, 0, 0, 0),
            ("back", 0, 0, 0, 0),
            ("two_cycle", 0, 0, 0, 0),
            ("self_loop", 0, 0, 0, 0),
            ("labels", 0, 0, 0, 0),
            ("nested", 0, 0, 0, 0),
            ("hero", 8, 6, 8, 4),
            ("wide_node", 0, 0, 0, 0),
            ("tier1_micro", 0, 0, 0, 0),
            ("tier2_plat", 0, 0, 0, 0),
            ("tier3_cloud", 0, 0, 0, 0),
            ("tier4_ent", 0, 0, 0, 0),
            ("tier5_mega", 0, 0, 0, 0),
            ("label_stress", 12, 7, 12, 6),
            ("disc_eq", 0, 0, 0, 0),
            ("disc_mixed", 0, 0, 0, 0),
        ];
        for (name, g) in mirror_corpus() {
            let &(_, td_p, lr_p, td_c, lr_c) = pins
                .iter()
                .find(|(n, ..)| *n == name)
                .unwrap_or_else(|| panic!("fixture {name} has no label floor — add one"));
            for (dir, floor, cfloor) in [
                (Direction::TopDown, td_p, td_c),
                (Direction::BottomUp, td_p, td_c),
                (Direction::LeftRight, lr_p, lr_c),
                (Direction::RightLeft, lr_p, lr_c),
            ] {
                let mut cfg = LayoutConfig::standard();
                cfg.direction = dir;
                let ir = g.compute_layout_with_config(&cfg);
                let (_, placed, colored) = label_counts(&ir);
                assert!(
                    placed >= floor,
                    "{name} {dir:?}: labels regressed to {placed} (floor {floor})"
                );
                assert!(
                    colored >= cfloor,
                    "{name} {dir:?}: colored labels regressed to {colored} (floor {cfloor})"
                );
            }
        }
    }

    /// A self-loop's marker is the edge's ENTIRE visible ink — one
    /// cell. Text beats markers at the cell layer (a label must never
    /// be hole-punched), so any label window covering the cell would
    /// silently erase a whole edge from the drawing — which shipped:
    /// through 0.10.1 the hero graph in `LeftRight` rendered Gateway
    /// without its `↺`. The guard lives in `span_blocked`, the base
    /// check every placement host shares. Sweep: every fixture, all
    /// four directions, plain and colored-with-legend (the two gate
    /// extremes; both charsets project the same marker cell), the
    /// marker glyph must be at its IR cell.
    #[test]
    fn self_loop_marker_always_rendered() {
        let glyph_at = |text: &str, x: usize, y: usize| -> Option<char> {
            text.lines().nth(y).and_then(|l| l.chars().nth(x))
        };
        let mut checked = 0usize;
        for (name, g) in mirror_corpus() {
            for dir in [
                Direction::TopDown,
                Direction::BottomUp,
                Direction::LeftRight,
                Direction::RightLeft,
            ] {
                let mut cfg = LayoutConfig::standard();
                cfg.direction = dir;
                let ir = g.compute_layout_with_config(&cfg);
                let loops: alloc::vec::Vec<(usize, usize, usize)> = ir
                    .nodes
                    .iter()
                    .filter_map(|n| n.self_loop_at.map(|(x, y)| (n.id, x, y)))
                    .collect();
                if loops.is_empty() {
                    continue;
                }
                let plain = ir.render_string(&RenderOptions::plain());
                let colored = strip_ansi(&ir.render_string(&RenderOptions::colored(Palette::Ansi)));
                for &(id, x, y) in &loops {
                    for (surface, text) in [("plain", &plain), ("colored", &colored)] {
                        assert_eq!(
                            glyph_at(text, x, y),
                            Some('↺'),
                            "{name} {dir:?} {surface}: node {id}'s self-loop marker at ({x},{y}) was overwritten"
                        );
                    }
                    checked += 1;
                }
            }
        }
        assert!(
            checked >= 8,
            "sweep went vacuous — corpus lost its self-loops"
        );
    }

    /// The marker guard itself, pinned mechanically: `span_blocked` —
    /// the base check every placement host shares — refuses any window
    /// covering a self-loop marker cell, on both flow axes. The sweep
    /// above proves the rendered outcome, but today's ladder happens
    /// never to propose such a window over the corpus, so only this
    /// test fails if the guard is dropped.
    #[test]
    fn span_blocked_refuses_self_loop_marker_cells() {
        for dir in [Direction::TopDown, Direction::LeftRight] {
            let mut g = Graph::new();
            g.add_node(1, "Gate");
            g.add_node(2, "Next");
            g.add_edge(1, 1, None);
            g.add_edge(1, 2, Some("lbl"));
            let mut cfg = LayoutConfig::standard();
            cfg.direction = dir;
            let ir = g.compute_layout_with_config(&cfg);
            let (mx, my) = ir
                .nodes
                .iter()
                .find_map(|n| n.self_loop_at)
                .expect("Gate has a marker cell");
            let li = ir
                .edges()
                .iter()
                .position(|e| e.from_id != e.to_id)
                .expect("the labeled edge");
            // A one-cell window on the marker: blocked. One cell
            // further along the same (otherwise empty) row: free —
            // the block is the marker itself, not its row.
            // The ink index the plan build would construct, built the
            // same way (the row-agnostic visitors, sorted).
            use crate::render::engine::view::LayoutView;
            let mut h: alloc::vec::Vec<(usize, usize, usize, usize)> = alloc::vec::Vec::new();
            let mut v: alloc::vec::Vec<(usize, usize, usize, usize)> = alloc::vec::Vec::new();
            for i in 0..LayoutView::edge_count(&ir) {
                let e = LayoutView::edge(&ir, i);
                crate::render::engine::plan::for_each_h_run_all(
                    &e.path,
                    e.from_x,
                    e.from_y,
                    e.to_x,
                    e.to_y,
                    e.flow_axis,
                    &mut |r, a, b| h.push((r, a, b, i)),
                );
                crate::render::engine::plan::for_each_v_seg_all(
                    &e.path,
                    e.from_x,
                    e.from_y,
                    e.to_x,
                    e.to_y,
                    e.flow_axis,
                    &mut |c, lo, hi| v.push((c, lo, hi, i)),
                );
            }
            h.sort_unstable();
            v.sort_unstable();
            let ink = crate::render::engine::plan::InkSource::Indexed(
                crate::render::engine::plan::InkIndex { h: &h, v: &v },
            );
            assert!(
                crate::render::engine::plan::span_blocked(
                    &ir,
                    &ink,
                    li,
                    my,
                    mx,
                    mx + 1,
                    &[],
                    false
                ),
                "{dir:?}: a window covering the marker cell must be blocked"
            );
            assert!(
                !crate::render::engine::plan::span_blocked(
                    &ir,
                    &ink,
                    li,
                    my,
                    mx + 1,
                    mx + 2,
                    &[],
                    false
                ),
                "{dir:?}: the cell beside the marker is not the marker"
            );
        }
    }

    /// The slide tier's lateral-offset ranking, pinned as a mapping.
    /// op 0 = centered window, op 1 = ink at the window's right edge
    /// (extends left in +x), op 2 = ink at the left edge (extends
    /// right). Center always wins. For X edges the side ranks follow
    /// the flow — extend-backward beats extend-forward — because the
    /// two side windows are x-mirror counterparts and a fixed order
    /// would choose visually different windows in LR vs RL when both
    /// sides are free. Y edges slide on x, which TD↔BT never flips, so
    /// their order is direction-independent. (A raw `op` key was once
    /// destroyed by an `op.max(toward)` typo that made the physically
    /// leftmost window win everywhere — hence a mapping pin, not a
    /// render test: no corpus fixture currently has two free side
    /// windows at the winning anchor.)
    #[test]
    fn lateral_rank_is_center_first_and_flow_relative() {
        use crate::render::engine::plan::lateral_rank;
        for (op, is_x, fwd, want) in [
            // Center first, in every configuration.
            (0, true, true, 0),
            (0, true, false, 0),
            (0, false, true, 0),
            (0, false, false, 0),
            // X forward: extends-left (op 1) is extend-backward.
            (1, true, true, 1),
            (2, true, true, 2),
            // X backward: the roles swap — mirror of the forward case.
            (1, true, false, 2),
            (2, true, false, 1),
            // Y: fixed order regardless of flow sign.
            (1, false, true, 1),
            (2, false, true, 2),
            (1, false, false, 1),
            (2, false, false, 2),
        ] {
            assert_eq!(
                lateral_rank(op, is_x, fwd),
                want,
                "lateral_rank({op}, is_x={is_x}, fwd={fwd})"
            );
        }
    }

    /// No placed label window may cover the label edge's OWN corner or
    /// endpoint-marker cells — Rule 1 plus the arrowhead rule, asserted
    /// end-to-end on every plan the corpus produces, in all four
    /// directions. (Label text paints after edge ink and wins the cell,
    /// so a covering window erases a bend or an arrowhead from the
    /// drawing.)
    #[test]
    fn label_windows_never_cover_own_fixed_cells() {
        let mut checked = 0usize;
        for (name, g) in mirror_corpus() {
            for dir in [
                Direction::TopDown,
                Direction::BottomUp,
                Direction::LeftRight,
                Direction::RightLeft,
            ] {
                let mut cfg = LayoutConfig::standard();
                cfg.direction = dir;
                let ir = g.compute_layout_with_config(&cfg);
                let plan = ir.render_plan(&RenderOptions::plain());
                for l in plan.labels() {
                    if !l.placeable {
                        continue;
                    }
                    let (from_m, to_m) = plan
                        .edge_plan(l.edge_index)
                        .resolved_markers(ir.edges()[l.edge_index].reversed);
                    assert!(
                        !crate::render::engine::plan::own_fixed_cell_in_span(
                            &ir,
                            l.edge_index,
                            from_m,
                            to_m,
                            l.y,
                            l.x,
                            l.x + l.len
                        ),
                        "{name} {dir:?}: edge {}'s label window covers its own corner/marker",
                        l.edge_index
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked >= 60, "sweep went vacuous — corpus lost its labels");
    }

    /// The cross-segment tie rank, pinned as a mapping: tied candidate
    /// rows prefer the row nearer the segment's own SOURCE end, in
    /// both row orientations. Ranking rows with the x-derived
    /// `toward_flow` sign would prefer the upper row in LR and the
    /// lower in RL — not an x-mirror, which must preserve y. This key
    /// depends only on x-mirror-invariant inputs (rows and the
    /// segment's row direction), so reflection safety is structural.
    #[test]
    fn cross_row_rank_prefers_the_source_side_row() {
        use crate::render::engine::plan::cross_row_rank;
        // Downward segment (source row 2, target row 9): row 4 is
        // nearer the source than row 7.
        assert!(cross_row_rank(4, 2, 9) < cross_row_rank(7, 2, 9));
        // Upward segment (source row 9, target row 2): row 7 is nearer
        // the source — the preference flips WITH the segment, not with
        // the x direction.
        assert!(cross_row_rank(7, 9, 2) < cross_row_rank(4, 9, 2));
    }

    /// Dummy waypoints must be placement-neutral while hidden: with
    /// `include_dummy_nodes` in the IR but `show_dummy_nodes` off (the
    /// default render), every label plan must be byte-identical to the
    /// same graph laid out without dummies — a hidden dummy paints
    /// nothing, so blocking its cell would be phantom. With the marker
    /// shown it becomes a legitimate blocker (it paints after labels).
    #[test]
    fn hidden_dummy_nodes_are_label_placement_neutral() {
        for dir in [Direction::TopDown, Direction::LeftRight] {
            let collect = |include: bool| {
                let g = label_stress();
                let mut cfg = LayoutConfig::standard();
                cfg.direction = dir;
                cfg.include_dummy_nodes = include;
                let ir = g.compute_layout_with_config(&cfg);
                let plan = ir.render_plan(&RenderOptions::plain());
                let labels: alloc::vec::Vec<(usize, usize, usize, bool)> = plan
                    .labels()
                    .iter()
                    .map(|l| (l.edge_index, l.x, l.y, l.placeable))
                    .collect();
                labels
            };
            assert_eq!(
                collect(true),
                collect(false),
                "{dir:?}: hidden dummies changed label placement"
            );
            // Shown: only a smoke check — visible markers may
            // legitimately move labels.
            let g = label_stress();
            let mut cfg = LayoutConfig::standard();
            cfg.direction = dir;
            cfg.include_dummy_nodes = true;
            let ir = g.compute_layout_with_config(&cfg);
            let mut opts = RenderOptions::plain();
            opts.show_dummy_nodes = true;
            let _ = ir.render_plan(&opts);
        }
    }

    /// The headline corpus totals, pinned directly: the per-fixture
    /// floors share one TD/BT (and one LR/RL) minimum, so a fixture
    /// whose two mirror directions ever diverge again (label_stress
    /// did, 11 TD / 12 BT, until the phantom-marker fix) could regress
    /// its better direction without tripping a floor. The sums cannot.
    #[test]
    fn corpus_label_totals_never_regress() {
        let (mut plain_t, mut colored_t) = (0usize, 0usize);
        for (_, g) in mirror_corpus() {
            for dir in [
                Direction::TopDown,
                Direction::BottomUp,
                Direction::LeftRight,
                Direction::RightLeft,
            ] {
                let mut cfg = LayoutConfig::standard();
                cfg.direction = dir;
                let ir = g.compute_layout_with_config(&cfg);
                let (_, p, c) = label_counts(&ir);
                plain_t += p;
                colored_t += c;
            }
        }
        assert!(plain_t >= 70, "plain corpus total regressed to {plain_t}");
        assert!(
            colored_t >= 64,
            "colored corpus total regressed to {colored_t}"
        );
    }

    /// The mirror claim asserted on the `LabelPlan`s themselves —
    /// rendered-text parsing cannot distinguish duplicate label
    /// strings. LR↔RL: identical placeable flags per edge, positions
    /// exact x-mirrors. TD↔BT: for edges placeable in BOTH directions
    /// (box-label text is direction-canonical — D4 — so feasibility
    /// may differ), positions are exact y-mirrors.
    #[test]
    fn label_plans_reflect_exactly() {
        for (name, g) in mirror_corpus() {
            let collect = |dir| {
                let mut cfg = LayoutConfig::standard();
                cfg.direction = dir;
                let ir = g.compute_layout_with_config(&cfg);
                let plan = ir.render_plan(&RenderOptions::plain());
                let labels: alloc::vec::Vec<(usize, usize, usize, usize, bool)> = plan
                    .labels()
                    .iter()
                    .map(|l| (l.edge_index, l.x, l.y, l.len, l.placeable))
                    .collect();
                (ir.width(), ir.height(), labels)
            };
            let (w, _, lr) = collect(Direction::LeftRight);
            let (_, _, rl) = collect(Direction::RightLeft);
            assert_eq!(lr.len(), rl.len(), "{name}: label plan counts differ");
            for (a, b) in lr.iter().zip(rl.iter()) {
                assert_eq!(a.0, b.0, "{name}: edge order diverged");
                assert_eq!(a.4, b.4, "{name} edge {}: LR/RL placeable differ", a.0);
                if a.4 {
                    assert_eq!(
                        (b.2, b.1),
                        (a.2, w - a.1 - a.3),
                        "{name} edge {}: RL plan is not the x-mirror of LR",
                        a.0
                    );
                }
            }
            let (_, h, td) = collect(Direction::TopDown);
            let (_, _, bt) = collect(Direction::BottomUp);
            for (a, b) in td.iter().zip(bt.iter()) {
                if a.4 && b.4 {
                    assert_eq!(
                        (b.1, b.2),
                        (a.1, h - 1 - a.2),
                        "{name} edge {}: BT plan is not the y-mirror of TD",
                        a.0
                    );
                }
            }
        }
    }

    /// `LabelPlan::paints` is the ONE label-visibility predicate —
    /// plan build (legend + invisibility warning), compositor, and
    /// hit-testing all consume it. Pinned as a truth table because a
    /// divergent copy once existed: the warning path applied the
    /// colored row-veto whenever color was on, warning about labels a
    /// colored-without-legend render actually painted.
    #[test]
    fn label_visibility_gate_truth_table() {
        let mk = |placeable: bool, row_has_node: bool| crate::render::engine::plan::LabelPlan {
            edge_index: 0,
            x: 0,
            y: 0,
            len: 3,
            placeable,
            row_has_node,
        };
        let hosted = mk(true, true); // placeable, but its row hosts a node
        let clear = mk(true, false);
        let unplaced = mk(false, false);
        for (colored, legend) in [(false, false), (false, true), (true, false)] {
            assert!(
                hosted.paints(colored, legend),
                "row-veto must apply ONLY under colored+legend (case {colored},{legend})"
            );
        }
        assert!(
            !hosted.paints(true, true),
            "colored+legend applies the row-veto"
        );
        for (colored, legend) in [(false, false), (false, true), (true, false), (true, true)] {
            assert!(clear.paints(colored, legend));
            assert!(!unplaced.paints(colored, legend));
        }
    }

    /// The ink index must be PURE acceleration: `span_blocked` and
    /// `slide_blocked` must answer identically whether they read the
    /// sorted index or walk the geometry visitors per query — the
    /// adaptive `LABEL_INDEX_MIN_WORK` threshold picks an arm on
    /// memory grounds and may never change a placement. Swept over
    /// label-bearing fixtures (Direct/Corner/MultiSegment paths,
    /// reversed edges, boxes, self-loops), all four directions, every
    /// row, and a stride of windows per labeled edge.
    #[test]
    fn indexed_and_scanned_blockers_agree() {
        use crate::render::engine::plan::{
            InkIndex, InkSource, for_each_h_run_all, for_each_v_seg_all, slide_blocked,
            span_blocked,
        };
        use crate::render::engine::view::LayoutView;
        let picks = ["hero", "label_stress", "stage", "self_loop"];
        let mut compared = 0usize;
        for (name, g) in mirror_corpus() {
            if !picks.contains(&name) {
                continue;
            }
            for dir in [
                Direction::TopDown,
                Direction::BottomUp,
                Direction::LeftRight,
                Direction::RightLeft,
            ] {
                let mut cfg = LayoutConfig::standard();
                cfg.direction = dir;
                let ir = g.compute_layout_with_config(&cfg);
                let mut h: alloc::vec::Vec<(usize, usize, usize, usize)> = alloc::vec::Vec::new();
                let mut v: alloc::vec::Vec<(usize, usize, usize, usize)> = alloc::vec::Vec::new();
                for i in 0..LayoutView::edge_count(&ir) {
                    let e = LayoutView::edge(&ir, i);
                    for_each_h_run_all(
                        &e.path,
                        e.from_x,
                        e.from_y,
                        e.to_x,
                        e.to_y,
                        e.flow_axis,
                        &mut |r, a, b| h.push((r, a, b, i)),
                    );
                    for_each_v_seg_all(
                        &e.path,
                        e.from_x,
                        e.from_y,
                        e.to_x,
                        e.to_y,
                        e.flow_axis,
                        &mut |c, lo, hi| v.push((c, lo, hi, i)),
                    );
                }
                h.sort_unstable();
                v.sort_unstable();
                let indexed = InkSource::Indexed(InkIndex { h: &h, v: &v });
                let scan = InkSource::Scan;
                let sg_row = |_: usize, sg: &crate::render::engine::view::SubgraphRef<'_>| sg.y + 1;
                for le in 0..LayoutView::edge_count(&ir) {
                    let e = LayoutView::edge(&ir, le);
                    let Some(text) = e.label else { continue };
                    let len = text.chars().count() + 2;
                    for row in 0..ir.height() {
                        let mut x0 = 0usize;
                        while x0 + len <= ir.width() {
                            let a = span_blocked(&ir, &indexed, le, row, x0, x0 + len, &[], false);
                            let b = span_blocked(&ir, &scan, le, row, x0, x0 + len, &[], false);
                            assert_eq!(a, b, "{name} {dir:?} span e{le} r{row} x{x0}");
                            let a = slide_blocked(
                                &ir,
                                &indexed,
                                le,
                                row,
                                x0,
                                x0 + len,
                                &[],
                                false,
                                &sg_row,
                            );
                            let b = slide_blocked(
                                &ir,
                                &scan,
                                le,
                                row,
                                x0,
                                x0 + len,
                                &[],
                                false,
                                &sg_row,
                            );
                            assert_eq!(a, b, "{name} {dir:?} slide e{le} r{row} x{x0}");
                            compared += 2;
                            x0 += 3;
                        }
                    }
                }
            }
        }
        assert!(
            compared > 10_000,
            "oracle went vacuous ({compared} comparisons)"
        );
    }

    #[test]
    #[ignore = "reporting tool, not an assertion — run with --ignored --nocapture"]
    fn quality_table() {
        const DIRS: [(&str, Direction); 4] = [
            ("TD", Direction::TopDown),
            ("BT", Direction::BottomUp),
            ("LR", Direction::LeftRight),
            ("RL", Direction::RightLeft),
        ];

        println!("\nfixture      dir   cross  kinks  spread     area  canvas   lblP   lblC");
        println!("-----------  ---  ------  -----  ------  -------  ------  -----  -----");
        let mut totals = [0usize; 4];
        let mut lbl_totals = [0usize; 3];
        for (name, g) in mirror_corpus() {
            for (tag, dir) in DIRS {
                let mut cfg = LayoutConfig::standard();
                cfg.direction = dir;
                let ir = g.compute_layout_with_config(&cfg);
                let (c, k, s, a) = score(&ir);
                let (elig, lp, lc) = label_counts(&ir);
                totals[0] += c;
                totals[1] += k;
                totals[2] += s;
                totals[3] += a;
                lbl_totals[0] += elig;
                lbl_totals[1] += lp;
                lbl_totals[2] += lc;
                println!(
                    "{name:<11}  {tag:<3}  {c:>6}  {k:>5}  {s:>6}  {a:>7}  {:>4}x{:<4}  {lp:>2}/{elig:<2}  {lc:>2}/{elig:<2}",
                    ir.width(),
                    ir.height()
                );
            }
        }
        println!("-----------  ---  ------  -----  ------  -------  ------  -----  -----");
        println!(
            "{:<11}  {:<3}  {:>6}  {:>5}  {:>6}  {:>7}  {:>6}  {:>2}/{:<2}  {:>2}/{:<2}",
            "TOTAL",
            "",
            totals[0],
            totals[1],
            totals[2],
            totals[3],
            "",
            lbl_totals[1],
            lbl_totals[0],
            lbl_totals[2],
            lbl_totals[0]
        );
        println!(
            "\ncross = stroke-crosses-stroke cells (border junctions excluded)\n\
             kinks = cross-axis lane changes summed over edges\n\
             spread = worst single edge's cross-axis excursion\n\
             area  = canvas w*h (guards against trading crossings for size)\n"
        );
    }
}
