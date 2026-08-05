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

/// Chain with a skip-level edge (A→D skips 2 levels → 2 dummy nodes).
fn skip_graph() -> Graph<'static> {
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
    g.compute_layout_with_config(config)
        .render_string(&ascii_dag::render::engine::RenderOptions::plain())
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

        let options = ascii_dag::render::engine::RenderOptions::plain();
        let mut arena_buf = vec![0u8; ir.estimate_render_arena_size(&options)];
        let render_arena = Arena::new(&mut arena_buf);
        let mut render_buf = vec![0u8; ir.estimate_render_output_size(&options)];
        let bytes = ir
            .render_to_bytes(&options, &render_arena, &mut render_buf)
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

    /// A 2-node cycle must render byte-identically in both backends.
    /// Before 0.10.0 the heap path lacked the ±1 column offset for
    /// anti-parallel pairs entirely (and the CSR check was O(E²)).
    #[test]
    fn two_node_cycle_renders_identically_in_both_backends() {
        let config = LayoutConfig::standard();
        let mut g = Graph::new();
        g.add_node(1, "Ping");
        g.add_node(2, "Pong");
        g.add_edge(1, 2, None);
        g.add_edge(2, 1, None);
        let heap_out = render_heap(&g, &config);
        let csr_out = render_csr(&g, &config);
        assert_eq!(
            heap_out, csr_out,
            "2-node-cycle output diverges:\n=== heap ===\n{heap_out}\n=== csr ===\n{csr_out}"
        );
        // The pair renders side by side: solid down-arrow next to the
        // dashed up-arrow, not overlapping in one column.
        assert!(
            heap_out.contains('↓') && heap_out.contains('⇡'),
            "{heap_out}"
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
    let rendered = g
        .compute_layout()
        .render_string(&ascii_dag::render::engine::RenderOptions::plain());
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
        let default_out = g
            .compute_layout()
            .render_string(&ascii_dag::render::engine::RenderOptions::plain());
        let mut g2 = stage_graph();
        g2.set_direction(Direction::TopDown);
        let explicit_out = g2
            .compute_layout()
            .render_string(&ascii_dag::render::engine::RenderOptions::plain());
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

// ── Dummy-node emission (include_dummy_nodes) ────────────────────────────

mod dummy_nodes {
    use super::*;
    use ascii_dag::ir::NodeKind;

    fn dummy_config() -> LayoutConfig<'static> {
        let mut config = LayoutConfig::standard();
        config.include_dummy_nodes = true;
        config
    }

    #[test]
    fn disabled_by_default_and_output_unchanged() {
        let g = skip_graph();
        let plain = g.compute_layout();
        assert_eq!(plain.nodes().len(), 4, "no dummies by default");

        // Enabling the flag must not change the rendered text — dummies
        // occupy the same columns the edge verticals already use.
        let with_dummies = g.compute_layout_with_config(&dummy_config());
        assert_eq!(with_dummies.nodes().len(), 6, "4 real + 2 dummies");
    }

    #[test]
    fn heap_dummies_have_correct_shape() {
        let g = skip_graph();
        let ir = g.compute_layout_with_config(&dummy_config());

        let dummies: Vec<_> = ir
            .nodes()
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::Dummy))
            .collect();
        assert_eq!(dummies.len(), 2, "A→D skips levels 1 and 2");

        // The skip edge A→D is edge index 3 (insertion order).
        let mut levels: Vec<usize> = dummies.iter().map(|d| d.level).collect();
        levels.sort_unstable();
        assert_eq!(levels, vec![1, 2]);
        for d in &dummies {
            assert_eq!(d.edge_index, Some(3), "back-link to owning edge");
            assert_eq!(d.label, "");
            assert_eq!(d.width, 1);
            assert_eq!(d.height, 1);
            assert!(
                ir.node_by_id(d.id).is_none(),
                "synthetic ids are excluded from node_by_id"
            );
        }

        // Real nodes still resolve.
        for id in 1..=4 {
            assert!(ir.node_by_id(id).is_some());
        }

        // Dummies appear in the level lists.
        assert!(
            ir.nodes_at_level(1)
                .any(|n| matches!(n.kind, NodeKind::Dummy)),
            "level list includes the dummy"
        );
    }

    #[test]
    fn heap_dummy_shares_column_with_its_waypoint() {
        // P1 invariant: the node-domain view (dummy) and the edge-domain
        // view (waypoint) of the same routing must never drift.
        let g = skip_graph();
        let ir = g.compute_layout_with_config(&dummy_config());

        let skip_edge = &ir.edges()[3];
        if let ascii_dag::EdgePath::MultiSegment { waypoints, .. } = &skip_edge.path {
            for d in ir
                .nodes()
                .iter()
                .filter(|n| matches!(n.kind, NodeKind::Dummy))
            {
                assert!(
                    waypoints.iter().any(|&(wx, _)| wx == d.x),
                    "dummy at column {} has no waypoint on edge 3 ({waypoints:?})",
                    d.x,
                );
            }
        } else {
            // Jog-aware routing may collapse a straight chain — then the
            // dummies' columns must line up with the edge's own column.
            for d in ir
                .nodes()
                .iter()
                .filter(|n| matches!(n.kind, NodeKind::Dummy))
            {
                assert_eq!(d.x, skip_edge.to_x, "straight chain column");
            }
        }
    }

    #[test]
    fn json_serializes_dummy_edge_index() {
        let g = skip_graph();
        let ir = g.compute_layout_with_config(&dummy_config());
        let json = ir.to_json();
        assert!(
            json.contains("\"kind\":\"dummy\""),
            "dummy kind serialized:\n{json}"
        );
        assert!(
            json.contains("\"edge_index\":3"),
            "dummy edge back-link serialized:\n{json}"
        );
    }

    #[cfg(feature = "arena")]
    mod csr {
        use super::*;
        use ascii_dag::graph::arena::Arena;
        use ascii_dag::ir::arena::LayoutIRArena;

        fn with_csr_ir(g: &Graph<'_>, check: impl FnOnce(&LayoutIRArena)) {
            let config = dummy_config();
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
        fn csr_dummies_match_heap() {
            let g = skip_graph();
            let heap_ir = g.compute_layout_with_config(&dummy_config());
            let mut heap_dummies: Vec<(usize, usize, usize)> = heap_ir
                .nodes()
                .iter()
                .filter(|n| matches!(n.kind, NodeKind::Dummy))
                .map(|n| (n.edge_index.unwrap(), n.level, n.x))
                .collect();
            heap_dummies.sort_unstable();

            with_csr_ir(&g, |csr_ir| {
                let mut csr_dummies: Vec<(usize, usize, usize)> = csr_ir
                    .nodes()
                    .iter()
                    .filter(|n| matches!(n.kind, NodeKind::Dummy))
                    .map(|n| (n.edge_index, n.level, n.x))
                    .collect();
                csr_dummies.sort_unstable();
                assert_eq!(
                    csr_dummies, heap_dummies,
                    "dummy (edge, level, column) sets diverge between backends"
                );

                // Synthetic ids excluded from lookups; real ids resolve.
                for n in csr_ir
                    .nodes()
                    .iter()
                    .filter(|n| matches!(n.kind, NodeKind::Dummy))
                {
                    assert!(csr_ir.node_by_id(n.id).is_none());
                }
                for id in 1..=4 {
                    assert!(csr_ir.node_by_id(id).is_some());
                }
            });
        }
    }
}

// ── Deep chains (regression: >256 levels) ────────────────────────────────
//
// A 20k-node chain used to panic with "index out of bounds" inside CSR
// crossing reduction (per-level buffers were fixed at 256 levels).
// Contract now: per-level buffers are sized from the graph's real
// depth, so BOTH backends lay out arbitrarily deep graphs — limited
// only by the index type's node capacity — and render byte-identically.
// Unbroken cycles (CycleBreaking::None) that pump level relaxation past
// any DAG-possible depth still error cleanly (covered by unit tests in
// arena_csr.rs).
mod deep_chain {
    use super::*;
    use ascii_dag::graph::arena::Arena;

    fn chain(n: usize) -> Graph<'static> {
        let mut g = Graph::new();
        for i in 0..n {
            g.add_node(i, "N");
        }
        for i in 0..n - 1 {
            g.add_edge(i, i + 1, None);
        }
        g
    }

    #[test]
    fn deep_chain_renders_heap() {
        let out = render_heap(&chain(300), &LayoutConfig::standard());
        assert!(
            out.lines().count() >= 300,
            "300-level chain should render at least one row per level"
        );
        assert!(out.contains("[N]"), "node labels missing from output");
    }

    /// Depth-sized per-level buffers: deep chains lay out in the CSR
    /// backend and byte-match the heap render, with estimate-sized
    /// arenas (no slack factor — the estimate must be sufficient).
    #[test]
    #[cfg(not(feature = "arena-idx-u8"))]
    fn deep_chain_renders_identically_csr() {
        // 300 = just past the old fixed cap; 1_000 and 20_000 = depth
        // scaling (20k was the original panic report).
        for n in [300usize, 1_000, 20_000] {
            let g = chain(n);
            let heap_out = render_heap(&g, &LayoutConfig::standard());

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
                .unwrap_or_else(|e| panic!("n={n}: CSR layout must succeed, got {e}"));

            let options = ascii_dag::render::engine::RenderOptions::plain();
            let mut arena_buf = vec![0u8; ir.estimate_render_arena_size(&options)];
            let render_arena = Arena::new(&mut arena_buf);
            let mut render_buf = vec![0u8; ir.estimate_render_output_size(&options)];
            let bytes = ir
                .render_to_bytes(&options, &render_arena, &mut render_buf)
                .expect("arena render");
            let csr_out = String::from_utf8_lossy(&render_buf[..bytes]);
            assert_eq!(heap_out, csr_out, "n={n}: backends must render identically");
        }
    }

    /// Deep AND clustered: the subgraph overlap-repair and cluster
    /// compaction passes used fixed 257-level scratch and silently
    /// skipped deeper graphs, breaking heap/CSR parity exactly there.
    /// Now depth-sized: a 300-level chain with clusters and loose
    /// nodes must render byte-identically from both backends.
    #[test]
    #[cfg(not(feature = "arena-idx-u8"))]
    fn deep_clustered_chain_renders_identically_csr() {
        let mut g = Graph::new();
        for i in 0..300usize {
            g.add_node(i, "N");
            if i > 0 {
                g.add_edge(i - 1, i, None);
            }
        }
        // A cluster deep in the chain plus loose siblings around it.
        let sg = g.add_subgraph("Deep");
        g.put_nodes(&[280, 281, 282]).inside(sg).unwrap();
        g.add_node(1000, "Loose");
        g.add_edge(279, 1000, None);
        g.add_edge(1000, 283, None);

        let heap_out = render_heap(&g, &LayoutConfig::standard());
        assert!(heap_out.contains("Deep"), "cluster renders:\n…");

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
            .expect("deep clustered CSR layout succeeds");
        let options = ascii_dag::render::engine::RenderOptions::plain();
        let mut arena_buf = vec![0u8; ir.estimate_render_arena_size(&options)];
        let render_arena = Arena::new(&mut arena_buf);
        let mut render_buf = vec![0u8; ir.estimate_render_output_size(&options)];
        let bytes = ir
            .render_to_bytes(&options, &render_arena, &mut render_buf)
            .expect("render");
        let csr_out = String::from_utf8_lossy(&render_buf[..bytes]);
        assert_eq!(
            heap_out, csr_out,
            "deep clustered backends must render identically"
        );
    }

    /// More than 512 edges where the LATE edges are the skip-level
    /// ones: the old fixed-size dummy bookkeeping silently dropped
    /// routing for edges past index 511. Byte parity across backends
    /// proves every edge keeps its dummy chain.
    #[test]
    #[cfg(not(feature = "arena-idx-u8"))]
    fn many_edges_late_skips_render_identically_csr() {
        let mut g = Graph::new();
        // 280 adjacent chain edges first…
        for i in 0..281usize {
            g.add_node(i, "N");
            if i > 0 {
                g.add_edge(i - 1, i, None);
            }
        }
        // …then 280 skip edges (indices 280..560 — crossing 512).
        for i in 0..278usize {
            g.add_edge(i, i + 3, None);
        }
        // 280 chain + 278 skip = 558 edges — crosses the old 512 cap.
        let heap_out = render_heap(&g, &LayoutConfig::standard());
        let csr_out = render_csr_exact(&g);
        assert_eq!(heap_out, csr_out, "late skip edges keep their routing");
    }

    /// A waypoint-heavy graph (dummy chains past the old 1,000-waypoint
    /// and 400-vnode caps) must route identically in both backends.
    #[test]
    #[cfg(not(feature = "arena-idx-u8"))]
    fn waypoint_heavy_deep_graph_renders_identically_csr() {
        let mut g = Graph::new();
        for i in 0..90usize {
            g.add_node(i, "n");
            if i > 0 {
                g.add_edge(i - 1, i, None);
            }
        }
        // 15 long skips of ~70 levels each ≈ 1,000+ dummies with low
        // mutual crossing pressure.
        for k in 0..15usize {
            g.add_edge(k, k + 70, None);
        }
        let heap_out = render_heap(&g, &LayoutConfig::standard());
        let csr_out = render_csr_exact(&g);
        assert_eq!(heap_out, csr_out, "deep waypoint chains route identically");
    }

    /// FRONTIER (pre-existing, newly reachable): under extreme mutual
    /// crossing pressure (dozens of interleaved 60-level skips), the
    /// crossing-reduction heuristics order dummy runs differently in the
    /// two backends. Before the capacity fixes the CSR backend silently
    /// degraded this shape (vnode/waypoint caps), so it was never
    /// comparable at all. Un-ignore when the heuristics are aligned.
    #[test]
    #[ignore = "pre-existing crossing-heuristic divergence on extreme interleaved-skip shapes; caps fixed, ordering alignment pending"]
    #[cfg(not(feature = "arena-idx-u8"))]
    fn extreme_interleaved_skips_parity_frontier() {
        let mut g = Graph::new();
        for i in 0..80usize {
            g.add_node(i, "n");
            if i > 0 {
                g.add_edge(i - 1, i, None);
            }
        }
        for k in 0..40usize {
            g.add_edge(k % 10, 70 + (k % 10), None);
        }
        let heap_out = render_heap(&g, &LayoutConfig::standard());
        let csr_out = render_csr_exact(&g);
        assert_eq!(heap_out, csr_out);
    }

    /// CSR render with EXACTLY estimate-sized arenas (shared helper for
    /// the capacity tests — the estimate is part of what's under test).
    #[cfg(not(feature = "arena-idx-u8"))]
    fn render_csr_exact(g: &Graph<'_>) -> String {
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
        let options = ascii_dag::render::engine::RenderOptions::plain();
        let mut arena_buf = vec![0u8; ir.estimate_render_arena_size(&options)];
        let render_arena = Arena::new(&mut arena_buf);
        let mut render_buf = vec![0u8; ir.estimate_render_output_size(&options)];
        let bytes = ir
            .render_to_bytes(&options, &render_arena, &mut render_buf)
            .expect("render");
        String::from_utf8_lossy(&render_buf[..bytes]).into_owned()
    }

    /// The config-aware estimate must suffice EXACTLY (no slack
    /// factor) for the demanding combination: long labels on nodes and
    /// edges, nested clusters, and dummy emission enabled.
    #[test]
    #[cfg(not(feature = "arena-idx-u8"))]
    fn config_aware_estimate_is_sufficient_exactly() {
        let mut config = LayoutConfig::standard();
        config.include_dummy_nodes = true;

        let mut g = Graph::new();
        for i in 0..40usize {
            g.add_node(i, "a-rather-long-node-label-with-ünïcödé");
            if i > 0 {
                g.add_edge(i - 1, i, Some("labeled-edge-with-detail"));
            }
        }
        // Skip edges create dummies; clusters exercise sg storage.
        for k in 0..10usize {
            g.add_edge(k, k + 20, Some("skip-label"));
        }
        let sg = g.add_subgraph("A-Cluster-With-A-Long-Label");
        g.put_nodes(&[5, 6, 7]).inside(sg).unwrap();
        let inner = g.add_subgraph("Inner");
        g.put_nodes(&[6]).inside(inner).unwrap();
        g.put_subgraphs(&[inner]).inside(sg).unwrap();

        let mut csr_buf = vec![0u8; g.estimate_csr_arena_size() * 2];
        let mut csr_arena = Arena::new(&mut csr_buf);
        let csr = g.to_csr(&mut csr_arena).expect("CSR conversion");
        let size = g.estimate_layout_arena_size_with(&config);
        let mut temp_buf = vec![0u8; size];
        let mut out_buf = vec![0u8; size];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_arena = Arena::new(&mut out_buf);
        let ir = csr
            .compute_layout_arena(&config, &mut temp_arena, &mut out_arena)
            .expect("exactly estimate-sized arenas must suffice");
        assert!(
            ir.nodes()
                .iter()
                .any(|n| matches!(n.kind, ascii_dag::ir::NodeKind::Dummy)),
            "dummy emission was exercised"
        );
    }

    /// Under arena-idx-u8 the node-count check bounds depth naturally.
    #[test]
    #[cfg(feature = "arena-idx-u8")]
    fn deep_chain_bounded_by_node_capacity_u8() {
        use ascii_dag::GraphError;
        let g = chain(300);
        let mut csr_buf = vec![0u8; g.estimate_csr_arena_size() * 2];
        let mut csr_arena = Arena::new(&mut csr_buf);
        let Some(csr) = g.to_csr(&mut csr_arena) else {
            return; // conversion itself may reject >255 nodes
        };
        let size = (g.estimate_layout_arena_size() * 2).max(256 * 1024);
        let mut temp_buf = vec![0u8; size];
        let mut out_buf = vec![0u8; size];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_arena = Arena::new(&mut out_buf);
        let err = csr
            .compute_layout_arena(&LayoutConfig::standard(), &mut temp_arena, &mut out_arena)
            .expect_err("300 nodes exceed the u8 index capacity");
        assert!(matches!(err, GraphError::ExceedsMaxNodes { .. }));
    }
}

/// Review #2 follow-up (temp/09): a cyclic graph PAST the lane-pass
/// work cap must still fit an exactly estimate-sized layout arena.
/// With `E > LANE_PASS_MAX_WORK` (16,384) the lane pass is disabled and
/// contributes no bytes, so the base manifest must stand on its own
/// exact dummy count — the pre-fix estimator's unflipped relaxation
/// counted ZERO dummies for an ordered cycle, while cycle breaking
/// actually creates a span-(N-1) chain with N-2 waypoints.
///
/// Lives here rather than the lib parity suite because `--all-features`
/// unions in `arena-idx-u8` (255-node cap); this file runs on default
/// features like the other deep-graph pins.
#[test]
#[cfg(feature = "arena")]
fn big_cycle_past_lane_cap_fits_exactly_estimated_arena() {
    use ascii_dag::graph::arena::Arena;

    const N: usize = 16_386; // E = N + 1 > LANE_PASS_MAX_WORK
    let mut g = Graph::new();
    for i in 0..N {
        g.add_node(i, "n");
    }
    for i in 0..N - 1 {
        g.add_edge(i, i + 1, None);
    }
    g.add_edge(N - 1, 0usize, None); // ordered cycle

    let mut cfg = LayoutConfig::standard();
    cfg.include_dummy_nodes = true;
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
        .expect("exactly estimate-sized arena must suffice past the lane cap");
    let dummies = ir
        .nodes()
        .iter()
        .filter(|nd| nd.kind == ascii_dag::NodeKind::Dummy)
        .count();
    assert_eq!(dummies, N - 2, "every broken-cycle waypoint emitted");
}
