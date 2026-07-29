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
use crate::render::colors::Palette;
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

/// The hero example against the golden snapshot (regenerated at RW8
/// when the engine became the output of record).
#[test]
fn hero_matches_golden() {
    let engine = render_plain(&hero_graph().compute_layout(), &RenderOptions::plain());
    let golden = include_str!("../../../tests/golden/hero.txt");
    assert_same("hero (golden)", golden.trim_end(), engine.trim_end());
}

// The shared hero fixture refers to the crate by its external name.
use crate as ascii_dag;
include!("../../../examples/shared/hero_graph.rs");

// ── BottomUp rendering (RW5) ─────────────────────────────────────────────
//
// The first direction-aware output: the geometry-driven primitives
// paint the physical BT IR with no direction-specific code paths. No
// legacy comparisons exist here (the legacy renderers never painted
// BT); the invariants are cross-backend byte-identity, D4 semantics,
// and render-vs-IR physical consistency.

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
            assert_same(&format!("{tag} (BT colored heap vs csr)"), &heap_col, &csr_col);
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
        assert_eq!(row_of("Stage"), sg.y + 1, "box label just below top border:\n{out}");
        assert!(lines[sg.y].contains('╔'), "top border row:\n{out}");

        // The edge label renders too (its row is IR-physical).
        assert!(out.contains("\"go\""), "edge label present:\n{out}");
    }

    /// Reversed (back) edges mirror: their dashed arrow points down,
    /// painted above the layout-source.
    #[test]
    fn bt_back_edges_semantics() {
        let out = render_plain(&bt_heap_ir(back_edges), &RenderOptions::plain());
        assert!(out.contains('⇣'), "reversed arrow points down in BT:\n{out}");
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
                    assert_same(&format!("{tag} {direction:?} plain cap={cap}"), &plain_ref, &plain);
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
        let plan =
            crate::render::engine::plan::RenderPlan::build(&ir, &with_cap(RenderOptions::plain(), 5));
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

    /// A boxed corpus graph across both IRs at a tiny cap — banding
    /// composes with backend self-parity. (The hero graph is excluded:
    /// its heap and CSR *layouts* diverge — a pre-existing till-IR
    /// defect tracked separately — so it cannot witness render parity.)
    #[test]
    fn banded_self_parity_across_backends() {
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
        assert_same("nested_boxes banded cap=3 (heap vs csr)", &heap_out, &csr_out);
    }
}

// ── Zero-allocation byte surface (RW6, N2/R4.3) ──────────────────────────

mod no_alloc {
    use super::*;
    use crate::graph::Direction;
    use crate::render::engine::color::ColorMode;
    use crate::render::engine::{
        estimate_render_arena_size, estimate_render_output_size, render_to_bytes,
    };
    use crate::GraphError;

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
                            .unwrap_or_else(|e| {
                                panic!("{tag} {direction:?} cap={cap}: {e}")
                            });
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

mod styles {
    use super::*;
    use crate::graph::Direction;
    use crate::render::engine::style::{
        EdgeLabelStyle, EdgeStyle, EdgeStyleCtx, LabelPosition, MarkerShape, NodeBorder,
        NodeStyle, NodeStyleCtx, SubgraphBorder, SubgraphStyle, SubgraphStyleCtx,
    };
    use crate::render::engine::CellColor;

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
    fn angle_implicit(ctx: NodeStyleCtx<'_>) -> NodeStyle {
        NodeStyle {
            border: if ctx.is_implicit {
                NodeBorder::Angle
            } else {
                NodeBorder::Bracket
            },
            ..NodeStyle::default()
        }
    }
    fn red_nodes(_: NodeStyleCtx<'_>) -> NodeStyle {
        NodeStyle {
            text_color: CellColor::ansi256(196),
            ..NodeStyle::default()
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
    fn angle_border_marks_implicit_nodes_only() {
        let ir = implicit_nodes().compute_layout();
        let mut options = RenderOptions::plain();
        options.node_style_fn = angle_implicit;
        let out = render_plain(&ir, &options);
        assert!(out.contains("[Root]"), "explicit keeps brackets:\n{out}");
        assert!(out.contains('<') && out.contains('>'), "implicit gets angles:\n{out}");
        // And the arena IR agrees byte-for-byte.
        let g = implicit_nodes();
        let mut csr_buf = vec![0u8; g.estimate_csr_arena_size() * 2];
        let mut csr_arena = Arena::new(&mut csr_buf);
        let csr = g.to_csr(&mut csr_arena).expect("CSR conversion");
        let size = (g.estimate_layout_arena_size() * 2).max(256 * 1024);
        let mut temp_buf = vec![0u8; size];
        let mut out_buf = vec![0u8; size];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_arena = Arena::new(&mut out_buf);
        let config = LayoutConfig::standard();
        let csr_ir = csr
            .compute_layout_arena(&config, &mut temp_arena, &mut out_arena)
            .expect("CSR layout");
        assert_same("angle borders (heap vs csr)", &out, &render_plain(&csr_ir, &options));
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
        assert!(out.contains("Outer") && out.contains("Inner"), "labels stay:\n{out}");
        assert!(out.contains("[Work]"), "content stays:\n{out}");
    }

    #[test]
    fn subgraph_and_node_colors_reach_the_escape_stream() {
        let ir = stage_graph_for_styles().compute_layout();
        let mut options = RenderOptions::colored(Palette::Ansi);
        options.subgraph_style_fn = green_boxes;
        options.node_style_fn = red_nodes;
        options.edge_label_style_fn = magenta_labels;
        let out = render_colored(&ir, &options);
        assert!(out.contains("\x1b[38;5;42m"), "border color escapes:\n{out:?}");
        assert!(out.contains("\x1b[38;5;196m"), "node text color escapes:\n{out:?}");
        assert!(out.contains("\x1b[38;5;201m"), "label color escapes:\n{out:?}");
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
        assert_eq!(label_row, sg.y + sg.height - 2, "label at box bottom:\n{out}");
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
        assert!(unicode.contains(" \u{2192} "), "unicode legend arrow:\n{unicode:?}");
        assert!(ascii.contains(" -> "), "ascii legend arrow:\n{ascii:?}");
        assert!(!ascii.contains('\u{2192}'), "no unicode arrow in ascii legend:\n{ascii:?}");
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
        assert!(heap_out.contains("\x1b[38;5;196m"), "edge override:\n{heap_out:?}");
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

#[allow(deprecated)]
mod wrapper_compat {
    use super::*;

    #[test]
    fn heap_wrappers_match_engine() {
        for build in [stage as fn() -> Graph<'static>, hero_graph] {
            let ir = build().compute_layout();
            assert_eq!(ir.render_scanline(), render_plain(&ir, &RenderOptions::plain()));

            let mut to = String::new();
            ir.render_scanline_to(&mut to);
            assert_eq!(to, render_plain(&ir, &RenderOptions::plain()));

            let mut line = vec![' '; ir.width() + 8];
            let mut with_buf = String::new();
            ir.render_scanline_with_buffer(&mut line, &mut with_buf);
            assert_eq!(with_buf, to);

            let mut bytes = vec![0u8; to.len() + 64];
            let n = ir.render_scanline_to_bytes(&mut line, &mut bytes);
            assert_eq!(core::str::from_utf8(&bytes[..n]).unwrap(), to);

            let mut colored = RenderOptions::colored(Palette::Ansi);
            colored.legend = false;
            assert_eq!(
                ir.render_scanline_colored(Palette::Ansi),
                render_colored(&ir, &colored)
            );
            let mut colored_to = String::new();
            ir.render_scanline_colored_to(&mut colored_to, Palette::Ansi);
            assert_eq!(colored_to, render_colored(&ir, &colored));
            assert_eq!(
                ir.render_scanline_colored_with_legend(Palette::Ansi),
                render_colored(&ir, &RenderOptions::colored(Palette::Ansi))
            );
        }
    }

    /// The documented legacy flow — sizes from `estimate_render_size`,
    /// `None` only when buffers are undersized — still works, and the
    /// bytes match the replacement surface.
    #[test]
    fn arena_buffer_wrapper_keeps_its_contract() {
        let g = hero_graph();
        let mut csr_buf = vec![0u8; g.estimate_csr_arena_size() * 2];
        let mut csr_arena = Arena::new(&mut csr_buf);
        let csr = g.to_csr(&mut csr_arena).expect("CSR conversion");
        let size = g.estimate_layout_arena_size();
        let mut temp_buf = vec![0u8; size];
        let mut out_buf = vec![0u8; size];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_arena = Arena::new(&mut out_buf);
        let ir = csr
            .compute_layout_arena(&LayoutConfig::standard(), &mut temp_arena, &mut out_arena)
            .expect("CSR layout");

        let (out_size, scratch_len) = ir.estimate_render_size();
        let mut buffer = vec![0u8; out_size];
        let mut line = vec![' '; ir.width() + 8];
        let mut scratch = vec![0usize; scratch_len];
        let n = ir
            .render_to_buffer(&mut buffer, &mut line, &mut scratch)
            .expect("estimate-sized buffers must suffice");
        assert_eq!(
            core::str::from_utf8(&buffer[..n]).unwrap(),
            render_plain(&ir, &RenderOptions::plain())
        );

        // Undersized scratch still reports None, never panics.
        let mut tiny = vec![0usize; 4];
        assert!(ir.render_to_buffer(&mut buffer, &mut line, &mut tiny).is_none());

        // Colored wrappers match the engine's colored output.
        let mut edge_colors = vec![0usize; ir.edge_count()];
        ir.compute_edge_colors(&mut edge_colors, Palette::Ansi.colors().len());
        let mut color_buf = vec![0u8; ir.width() + 8];
        let mut skipped = vec![false; ir.edge_count()];
        let colored_size = ir.estimate_render_output_size(&RenderOptions::colored(Palette::Ansi));
        let mut cbuffer = vec![0u8; colored_size];
        let n = ir
            .render_to_buffer_colored_with_legend(
                &mut cbuffer,
                &mut line,
                &mut color_buf,
                &edge_colors,
                Palette::Ansi.colors(),
                &mut skipped,
            )
            .expect("colored wrapper renders");
        assert_eq!(
            core::str::from_utf8(&cbuffer[..n]).unwrap(),
            render_colored(&ir, &RenderOptions::colored(Palette::Ansi))
        );
    }
}
