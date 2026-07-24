//! Rendered-output verification.
//!
//! These tests close the gap that let the spacing bug ship in 0.9.x:
//! unit tests verified that `LayoutConfig` stored values, but nothing
//! verified the *rendered output* respected them. Every test here
//! renders a graph and asserts on the text a user actually sees, for
//! both layout backends.

use ascii_dag::LayoutConfig;
use ascii_dag::graph::Graph;

// ── Helpers ──────────────────────────────────────────────────────────────

fn sibling_graph() -> Graph<'static> {
    let mut g = Graph::new();
    g.add_node(1, "P");
    g.add_node(2, "A");
    g.add_node(3, "B");
    g.add_edge(1, 2, None);
    g.add_edge(1, 3, None);
    g
}

fn chain_graph() -> Graph<'static> {
    let mut g = Graph::new();
    g.add_node(1, "A");
    g.add_node(2, "B");
    g.add_edge(1, 2, None);
    g
}

/// Labeled edge entering a one-node cluster — the shape that exposed the
/// heap/CSR row-budget divergence (see direction::csr parity tests).
fn stage_graph() -> Graph<'static> {
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

/// Gap in columns between `[A]` and `[B]` on the line containing both.
fn sibling_gap(rendered: &str) -> usize {
    let line = rendered
        .lines()
        .find(|l| l.contains("[A]") && l.contains("[B]"))
        .expect("no line contains both siblings");
    let a_end = line.find("[A]").unwrap() + 3;
    let b_start = line.find("[B]").unwrap();
    b_start - a_end
}

/// Number of rows strictly between the `[A]` row and the `[B]` row.
fn rows_between(rendered: &str) -> usize {
    let a = rendered.lines().position(|l| l.contains("[A]")).unwrap();
    let b = rendered.lines().position(|l| l.contains("[B]")).unwrap();
    b - a - 1
}

fn render_heap(g: &Graph<'_>, config: &LayoutConfig<'_>) -> String {
    g.compute_layout_with_config(config).render_scanline()
}

// ── Heap backend ─────────────────────────────────────────────────────────

#[test]
fn node_spacing_reaches_rendered_output_heap() {
    for spacing in [3usize, 10] {
        let mut config = LayoutConfig::standard();
        config.node_spacing = spacing;
        let out = render_heap(&sibling_graph(), &config);
        assert_eq!(
            sibling_gap(&out),
            spacing,
            "node_spacing={spacing} not honored in rendered output:\n{out}"
        );
    }
}

#[test]
fn level_spacing_reaches_rendered_output_heap() {
    let baseline = {
        let mut config = LayoutConfig::standard();
        config.level_spacing = 0;
        rows_between(&render_heap(&chain_graph(), &config))
    };
    let spaced = {
        let mut config = LayoutConfig::standard();
        config.level_spacing = 4;
        rows_between(&render_heap(&chain_graph(), &config))
    };
    assert_eq!(
        spaced,
        baseline + 4,
        "level_spacing=4 should add exactly 4 rows between levels"
    );
}

// ── CSR backend ──────────────────────────────────────────────────────────

#[cfg(feature = "arena")]
mod csr {
    use super::*;
    use ascii_dag::graph::arena::Arena;

    fn render_csr(g: &Graph<'_>, config: &LayoutConfig<'_>) -> String {
        let mut csr_buf = vec![0u8; g.estimate_csr_arena_size() * 2];
        let mut csr_arena = Arena::new(&mut csr_buf);
        let csr = g.to_csr(&mut csr_arena).expect("CSR conversion");

        let size = (g.estimate_layout_arena_size() * 2).max(256 * 1024);
        let mut temp_buf = vec![0u8; size];
        let mut out_buf = vec![0u8; size];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_arena = Arena::new(&mut out_buf);
        let ir = csr
            .compute_layout_arena(config, &mut temp_arena, &mut out_arena)
            .expect("CSR layout");

        let (render_bytes, _) = ir.estimate_render_size();
        let mut render_buf = vec![0u8; render_bytes * 4 + 8192];
        let mut line_buf = vec![' '; ir.width().max(1) + 32];
        let mut scratch = vec![0usize; (ir.height() + ir.edge_count() * 2).max(1) + 64];
        let bytes = ir
            .render_to_buffer(&mut render_buf, &mut line_buf, &mut scratch)
            .expect("render");
        String::from_utf8_lossy(&render_buf[..bytes]).into_owned()
    }

    #[test]
    fn node_spacing_reaches_rendered_output_csr() {
        for spacing in [3usize, 10] {
            let mut config = LayoutConfig::standard();
            config.node_spacing = spacing;
            let out = render_csr(&sibling_graph(), &config);
            assert_eq!(
                sibling_gap(&out),
                spacing,
                "node_spacing={spacing} not honored in CSR rendered output:\n{out}"
            );
        }
    }

    #[test]
    fn level_spacing_reaches_rendered_output_csr() {
        let baseline = {
            let mut config = LayoutConfig::standard();
            config.level_spacing = 0;
            rows_between(&render_csr(&chain_graph(), &config))
        };
        let spaced = {
            let mut config = LayoutConfig::standard();
            config.level_spacing = 4;
            rows_between(&render_csr(&chain_graph(), &config))
        };
        assert_eq!(
            spaced,
            baseline + 4,
            "level_spacing=4 should add exactly 4 rows between levels (CSR)"
        );
    }

    /// Strongest backend-parity assertion: the same graph renders to
    /// byte-identical text through both layout paths (TopDown).
    #[test]
    fn stage_graph_renders_identically_in_both_backends() {
        let config = LayoutConfig::standard();
        let g = stage_graph();
        let heap_out = render_heap(&g, &config);
        let csr_out = render_csr(&g, &config);
        assert_eq!(
            heap_out, csr_out,
            "heap and CSR rendered output diverge:\n=== heap ===\n{heap_out}\n=== csr ===\n{csr_out}"
        );
    }
}

// ── Golden snapshot ──────────────────────────────────────────────────────

/// The full hero-example render, byte-for-byte.
///
/// This is the canary for unintended layout drift: any change to
/// layout or rendering that alters this output fails here and forces a
/// deliberate regeneration (see the assertion message). The graph
/// itself lives in examples/shared/hero_graph.rs — one source of truth
/// shared with examples/hero.rs.
#[test]
fn hero_example_matches_golden() {
    let g = hero_graph();
    let rendered = g.compute_layout().render_scanline();
    let golden = include_str!("golden/hero.txt");
    assert_eq!(
        rendered.trim_end(),
        golden.trim_end(),
        "hero render drifted from tests/golden/hero.txt.\n\
         If this change is INTENTIONAL, regenerate the golden file:\n\
         cargo run --example hero 2>/dev/null > tests/golden/hero.txt\n\
         and review the visual diff in the PR."
    );
}

include!("../examples/shared/hero_graph.rs");

// ── Direction ────────────────────────────────────────────────────────────

mod direction {
    use super::*;
    use ascii_dag::graph::Direction;

    #[test]
    fn parses_conventional_short_forms() {
        for (s, want) in [
            ("TB", Direction::TopDown),
            ("TD", Direction::TopDown),
            ("tb", Direction::TopDown),
            ("BT", Direction::BottomUp),
            ("bt", Direction::BottomUp),
            ("LR", Direction::LeftRight),
            ("RL", Direction::RightLeft),
        ] {
            assert_eq!(s.parse::<Direction>().unwrap(), want, "parsing {s:?}");
        }
        assert!("NE".parse::<Direction>().is_err());
        assert!("".parse::<Direction>().is_err());
    }

    // NOTE: BottomUp assertions are IR-level only. The built-in renderers
    // paint TopDown layouts exclusively until direction-aware rendering
    // lands with the renderer rewrite — no test may render a BT IR.

    #[test]
    fn bottom_up_ir_records_direction() {
        let mut g = stage_graph();
        g.set_direction(Direction::BottomUp);
        let ir = g.compute_layout();
        assert_eq!(ir.direction(), Direction::BottomUp);
        let td = stage_graph().compute_layout();
        assert_eq!(td.direction(), Direction::TopDown);
    }

    #[test]
    fn bottom_up_ir_puts_sources_at_bottom() {
        // Physical coordinates: under BT the source sits on larger rows
        // than the target.
        let mut g = stage_graph();
        g.set_direction(Direction::BottomUp);
        let ir = g.compute_layout();
        let start = ir.node_by_id(1).expect("Start in IR");
        let end = ir.node_by_id(3).expect("End in IR");
        assert!(
            start.y > end.y,
            "BottomUp: source row {} must be below target row {}",
            start.y,
            end.y,
        );
    }

    #[test]
    fn bottom_up_ir_is_exact_vertical_flip_of_top_down() {
        // The physical BT IR must be the exact mirror of the TD IR:
        // same dimensions, every y flipped, every x untouched.
        let td = stage_graph().compute_layout();
        let mut g = stage_graph();
        g.set_direction(Direction::BottomUp);
        let bt = g.compute_layout();

        assert_eq!(bt.width(), td.width());
        assert_eq!(bt.height(), td.height());
        assert_eq!(bt.level_count(), td.level_count());

        let h = td.height();
        let flip_row = |y: usize| h - 1 - y;

        for td_node in td.nodes() {
            let bt_node = bt.node_by_id(td_node.id).expect("node in BT IR");
            assert_eq!(bt_node.x, td_node.x, "x untouched for '{}'", td_node.label);
            assert_eq!(bt_node.width, td_node.width);
            assert_eq!(bt_node.height, td_node.height);
            assert_eq!(
                bt_node.y,
                h - (td_node.y + td_node.height),
                "flipped y for '{}'",
                td_node.label,
            );
            assert_eq!(bt_node.center_y, flip_row(td_node.center_y));
        }

        for (td_edge, bt_edge) in td.edges().iter().zip(bt.edges()) {
            assert_eq!(bt_edge.from_x, td_edge.from_x);
            assert_eq!(bt_edge.to_x, td_edge.to_x);
            assert_eq!(bt_edge.from_y, flip_row(td_edge.from_y));
            assert_eq!(bt_edge.to_y, flip_row(td_edge.to_y));
            if td_edge.label.is_some() {
                assert_eq!(bt_edge.label_x, td_edge.label_x);
                assert_eq!(bt_edge.label_y, flip_row(td_edge.label_y));
            }
        }

        for (td_sg, bt_sg) in td.subgraphs().iter().zip(bt.subgraphs()) {
            assert_eq!(bt_sg.x, td_sg.x);
            assert_eq!(bt_sg.width, td_sg.width);
            assert_eq!(bt_sg.height, td_sg.height);
            assert_eq!(bt_sg.y, h - (td_sg.y + td_sg.height));
        }
    }

    #[test]
    fn top_down_output_unchanged_by_direction_plumbing() {
        // Explicit TopDown must be byte-identical to the default.
        let g = stage_graph();
        let default_out = g.compute_layout().render_scanline();
        let mut g2 = stage_graph();
        g2.set_direction(Direction::TopDown);
        let explicit_out = g2.compute_layout().render_scanline();
        assert_eq!(default_out, explicit_out);
    }

    // ── CSR/arena backend ────────────────────────────────────────────────

    #[cfg(feature = "arena")]
    mod csr {
        use super::*;
        use ascii_dag::LayoutConfig;
        use ascii_dag::graph::arena::Arena;
        use ascii_dag::ir::arena::LayoutIRArena;

        /// Run the CSR layout and hand the borrowed IR to `check`.
        /// (The IR borrows the arenas, so it cannot be returned.)
        fn with_csr_ir(g: &Graph<'_>, direction: Direction, check: impl FnOnce(&LayoutIRArena)) {
            let mut config = LayoutConfig::standard();
            config.direction = direction;

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
            check(&ir);
        }

        #[test]
        fn bottom_up_ir_records_direction_csr() {
            let g = stage_graph();
            with_csr_ir(&g, Direction::BottomUp, |ir| {
                assert_eq!(ir.direction(), Direction::BottomUp);
            });
            with_csr_ir(&g, Direction::TopDown, |ir| {
                assert_eq!(ir.direction(), Direction::TopDown);
            });
        }

        #[test]
        fn bottom_up_ir_is_exact_vertical_flip_of_top_down_csr() {
            // Same invariant as the heap test: the physical BT IR is the
            // exact mirror of the TD IR — same dimensions, ys flipped,
            // xs untouched — including edge row spans and label rows.
            let g = stage_graph();
            with_csr_ir(&g, Direction::TopDown, |td| {
                with_csr_ir(&g, Direction::BottomUp, |bt| {
                    assert_eq!(bt.width(), td.width());
                    assert_eq!(bt.height(), td.height());
                    assert_eq!(bt.level_count(), td.level_count());

                    let h = td.height();
                    let flip_row = |y: usize| h - 1 - y;

                    assert_eq!(bt.node_count(), td.node_count());
                    for td_node in td.nodes() {
                        let bt_node = bt.node_by_id(td_node.id).expect("node in BT IR");
                        assert_eq!(bt_node.x, td_node.x);
                        assert_eq!(bt_node.width, td_node.width);
                        assert_eq!(bt_node.height, td_node.height);
                        assert_eq!(bt_node.y, h - (td_node.y + td_node.height));
                        assert_eq!(bt_node.center_y, flip_row(td_node.center_y));
                    }

                    assert_eq!(bt.edge_count(), td.edge_count());
                    for (td_edge, bt_edge) in td.edges().iter().zip(bt.edges()) {
                        assert_eq!(bt_edge.from_x, td_edge.from_x);
                        assert_eq!(bt_edge.to_x, td_edge.to_x);
                        assert_eq!(bt_edge.from_y, flip_row(td_edge.from_y));
                        assert_eq!(bt_edge.to_y, flip_row(td_edge.to_y));
                        // Occupied row span mirrors: old max becomes new min.
                        assert_eq!(bt_edge.min_y, flip_row(td_edge.max_y));
                        assert_eq!(bt_edge.max_y, flip_row(td_edge.min_y));
                        if td_edge.label_len > 0 {
                            assert_eq!(bt_edge.label_x, td_edge.label_x);
                            assert_eq!(bt_edge.label_y, flip_row(td_edge.label_y));
                        }
                    }

                    assert_eq!(bt.subgraph_count(), td.subgraph_count());
                    for (td_sg, bt_sg) in td.subgraphs().iter().zip(bt.subgraphs()) {
                        assert_eq!(bt_sg.x, td_sg.x);
                        assert_eq!(bt_sg.width, td_sg.width);
                        assert_eq!(bt_sg.height, td_sg.height);
                        assert_eq!(bt_sg.y, h - (td_sg.y + td_sg.height));
                    }
                });
            });
        }

        #[test]
        fn heap_and_csr_backends_agree_on_edge_labels() {
            // P4 made the heap edge-label shape identical to the arena's —
            // label positions are now comparable field-for-field.
            let g = stage_graph();
            for direction in [Direction::TopDown, Direction::BottomUp] {
                let mut heap_g = stage_graph();
                heap_g.set_direction(direction);
                let heap_ir = heap_g.compute_layout();

                with_csr_ir(&g, direction, |csr_ir| {
                    for heap_edge in heap_ir.edges().iter().filter(|e| e.label.is_some()) {
                        let csr_edge = csr_ir
                            .edges()
                            .iter()
                            .find(|e| e.edge_index == heap_edge.edge_index)
                            .expect("edge present in CSR IR");
                        assert!(csr_edge.label_len > 0, "label present in both IRs");
                        assert_eq!(
                            (csr_edge.label_x, csr_edge.label_y),
                            (heap_edge.label_x, heap_edge.label_y),
                            "label position diverges between backends ({direction:?})",
                        );
                    }
                });
            }
        }

        #[test]
        fn top_down_csr_output_unchanged_by_direction_plumbing() {
            // Explicit TopDown through the CSR path must not alter the IR.
            let g = stage_graph();
            with_csr_ir(&g, Direction::TopDown, |td| {
                let node_ys: Vec<usize> = td.nodes().iter().map(|n| n.y).collect();
                let g2 = stage_graph();
                with_csr_ir(&g2, Direction::TopDown, |td2| {
                    let node_ys2: Vec<usize> = td2.nodes().iter().map(|n| n.y).collect();
                    assert_eq!(node_ys, node_ys2);
                });
            });
        }

        #[test]
        fn bottom_up_heap_and_csr_backends_agree_on_nodes() {
            // Cross-backend parity (S2): the same graph laid out for the
            // same direction must place nodes identically in both IRs.
            // TopDown runs first so a pre-existing backend divergence is
            // reported as such, not as a direction bug.
            let g = stage_graph();
            for direction in [Direction::TopDown, Direction::BottomUp] {
                let mut heap_g = stage_graph();
                heap_g.set_direction(direction);
                let heap_ir = heap_g.compute_layout();

                with_csr_ir(&g, direction, |csr_ir| {
                    for heap_node in heap_ir.nodes() {
                        let csr_node = csr_ir
                            .node_by_id(heap_node.id)
                            .expect("node present in CSR IR");
                        assert_eq!(
                            (csr_node.x, csr_node.y, csr_node.width, csr_node.height),
                            (heap_node.x, heap_node.y, heap_node.width, heap_node.height),
                            "node '{}' geometry diverges between backends ({direction:?})",
                            heap_node.label,
                        );
                    }
                });
            }
        }

        // This test found a real pre-existing divergence on 2026-07-23:
        // the heap path skipped the vertical stub row below labeled-edge
        // sources (corner one row too early). Fixed by sharing
        // geometry::edge_start_row between the backends.
        #[test]
        fn heap_and_csr_backends_agree_on_subgraph_boxes() {
            let g = stage_graph();
            for direction in [Direction::TopDown, Direction::BottomUp] {
                let mut heap_g = stage_graph();
                heap_g.set_direction(direction);
                let heap_ir = heap_g.compute_layout();

                with_csr_ir(&g, direction, |csr_ir| {
                    assert_eq!(csr_ir.height(), heap_ir.height(), "canvas height");
                    for (heap_sg, csr_sg) in heap_ir.subgraphs().iter().zip(csr_ir.subgraphs()) {
                        assert_eq!(
                            (csr_sg.x, csr_sg.y, csr_sg.width, csr_sg.height),
                            (heap_sg.x, heap_sg.y, heap_sg.width, heap_sg.height),
                            "subgraph {} box diverges between backends ({direction:?})",
                            heap_sg.id,
                        );
                    }
                });
            }
        }
    }

}

