//! Wide characters: a two-column glyph (CJK, kana, Hangul, most emoji)
//! occupies two cells, a combining mark none, and every width the
//! engine computes from text agrees with what a terminal draws — so
//! box borders stay aligned on rows that contain such text, and
//! hit-testing, JSON and both backends agree.
//!
//! Gated on `layout-vertical`: the fixtures are TopDown.
#![cfg(feature = "layout-vertical")]

use ascii_dag::graph::Graph;
use ascii_dag::render::engine::{BoxedNode, CustomNode, NodePaintCtx, NodeRegion, RenderOptions};
use ascii_dag::text::{char_width, display_width};
use ascii_dag::{CellKind, HitResult, ScenePlanner};

/// CJK node labels (plain and boxed), a CJK cluster label, a CJK edge
/// label, and a combining mark.
fn wide_graph() -> Graph<'static> {
    let mut g = Graph::new();
    g.add_node(1usize, "日本語");
    g.add_node(2usize, BoxedNode("ノード"));
    g.add_node(3usize, "e\u{301}tat");
    g.add_node(4usize, "end");
    g.add_edge(1usize, 2usize, Some("ラベル"));
    g.add_edge(2usize, 4usize, None);
    g.add_edge(3usize, 4usize, None);
    let sg = g.add_subgraph("集合");
    g.put_nodes(&[2usize, 3usize]).inside(sg).unwrap();
    g
}

/// The display column of byte offset `b` in `line`.
fn col_of(line: &str, b: usize) -> usize {
    display_width(&line[..b])
}

/// The row and display column where `needle` first appears.
fn find(out: &str, needle: &str) -> (usize, usize) {
    out.lines()
        .enumerate()
        .find_map(|(r, l)| l.find(needle).map(|b| (r, col_of(l, b))))
        .unwrap_or_else(|| panic!("{needle:?} not rendered:\n{out}"))
}

#[test]
fn widths_are_display_cells_and_rows_stay_aligned() {
    let g = wide_graph();
    let ir = g.compute_layout();
    let out = ir.render_string(&RenderOptions::plain());
    for line in out.lines() {
        assert!(
            display_width(line) <= ir.width(),
            "row wider than the canvas ({} > {}):\n{out}",
            display_width(line),
            ir.width()
        );
    }
    // `[日本語]`: 3 wide glyphs + 2 brackets = 8 cells, drawn whole,
    // the closing bracket on the node's last column.
    let n1 = ir.node_by_id(1).unwrap();
    assert_eq!(n1.width, 8);
    let (row, col) = find(&out, "[日本語]");
    assert_eq!((row, col), (n1.y, n1.x), "{out}");
    // The boxed node: three rows, every border column aligned.
    let n2 = ir.node_by_id(2).unwrap();
    assert_eq!((n2.width, n2.height), (10, 3));
    let lines: Vec<&str> = out.lines().collect();
    for dy in 0..3 {
        let line = lines[n2.y + dy];
        let left = col_of(line, line.find(['┌', '│', '└']).unwrap());
        assert_eq!(left, n2.x, "row {dy} left border:\n{out}");
        let right = line
            .char_indices()
            .filter(|&(_, c)| matches!(c, '┐' | '│' | '┘'))
            .map(|(b, _)| col_of(line, b))
            .find(|&c| c > n2.x)
            .unwrap();
        assert_eq!(right, n2.x + 9, "row {dy} right border:\n{out}");
    }
    assert!(out.contains("│ ノード │"), "{out}");
    // A combining mark measures zero and has no cell of its own: the
    // base letter paints, the mark does not, the box stays 6 wide.
    let n3 = ir.node_by_id(3).unwrap();
    assert_eq!(n3.width, 6);
    assert!(out.contains("[etat]"), "{out}");
    // The cluster label and the edge label paint whole.
    assert!(out.contains("集合"), "{out}");
    assert!(out.contains("\"ラベル\""), "{out}");
}

#[test]
fn hit_testing_and_cell_views_cover_both_columns() {
    let g = wide_graph();
    let ir = g.compute_layout();
    let options = RenderOptions::plain();
    let mut planner = ScenePlanner::new();
    let scene = planner.plan(&ir, &options.plan).quiet().unwrap();
    let n1 = ir.node_by_id(1).unwrap();
    for x in n1.x..n1.x + n1.width {
        assert_eq!(scene.hit_test(x, n1.y), HitResult::Node(1), "x={x}");
    }
    let out = ir.render_string(&options);
    let (row, col) = find(&out, "\"ラベル\"");
    for x in col..col + 8 {
        assert!(
            matches!(scene.hit_test(x, row), HitResult::Edge(_)),
            "label cell x={x}: {:?}\n{out}",
            scene.hit_test(x, row)
        );
    }
    // Every continuation cell follows a wide lead on the same row, and
    // the counts match.
    let req = scene.composition_requirements(&options.compose);
    let mut composer = ascii_dag::SceneComposer::new(req);
    let mut grid = vec![vec![CellKind::Empty; scene.width()]; scene.height()];
    composer
        .visit_cells(&scene, |x, y, cell| grid[y][x] = cell.kind)
        .unwrap();
    let (mut leads, mut conts) = (0, 0);
    for row in &grid {
        for (x, kind) in row.iter().enumerate() {
            match *kind {
                CellKind::Text { ch } if char_width(ch) == 2 => {
                    leads += 1;
                    assert_eq!(row[x + 1], CellKind::Continuation, "x={x}");
                }
                CellKind::Continuation => {
                    conts += 1;
                    assert!(
                        matches!(row[x - 1], CellKind::Text { ch } if char_width(ch) == 2),
                        "x={x}"
                    );
                }
                _ => {}
            }
        }
    }
    assert_eq!(leads, conts);
    assert!(leads >= 3 + 3 + 2 + 3, "{leads} wide glyphs"); // 日本語 ノード 集合 ラベル
}

#[test]
fn json_reports_cell_widths() {
    let g = wide_graph();
    let ir = g.compute_layout();
    let json = ir.to_json();
    let n1 = json.find("\"id\":1,").expect("node 1");
    let after = &json[n1..];
    assert!(after.contains("\"width\":8"), "{after}");
}

#[test]
fn painters_never_split_a_wide_character() {
    fn paint(region: &mut NodeRegion<'_, '_>, ctx: NodePaintCtx<'_>) {
        region.write_str(0, 0, ctx.label);
    }
    for (width, shown, hidden) in [(7, "ab日本", "ab日本語"), (5, "ab日", "ab日本")] {
        let mut g = Graph::new();
        g.add_node(
            1usize,
            CustomNode {
                label: "ab日本語",
                width,
                height: 1,
                painter: Some(paint),
                payload: "",
            },
        );
        g.add_node(2usize, "x");
        g.add_edge(1usize, 2usize, None);
        let ir = g.compute_layout();
        let out = ir.render_string(&RenderOptions::plain());
        assert!(out.contains(shown), "width {width}:\n{out}");
        assert!(!out.contains(hidden), "width {width}:\n{out}");
        for line in out.lines() {
            assert!(display_width(line) <= ir.width(), "{out}");
        }
    }
    // Clipping a `[label]` node declared narrower than its text stops
    // before the glyph that would not fit.
    let mut g = Graph::new();
    g.add_node(
        1usize,
        CustomNode {
            label: "日本語",
            width: 5,
            height: 1,
            painter: None,
            payload: "",
        },
    );
    g.add_node(2usize, "x");
    g.add_edge(1usize, 2usize, None);
    let out = g.compute_layout().render_string(&RenderOptions::plain());
    for line in out.lines() {
        assert!(display_width(line) <= 5 + 3, "{out}");
    }
}

#[cfg(feature = "arena")]
#[test]
fn both_backends_agree_on_wide_text() {
    use ascii_dag::LayoutConfig;
    use ascii_dag::graph::arena::Arena;
    let g = wide_graph();
    let config = LayoutConfig::standard();
    let heap = g.compute_layout().render_string(&RenderOptions::plain());
    let mut csr_buf = vec![0u8; g.estimate_csr_arena_size()];
    let mut csr_arena = Arena::new(&mut csr_buf);
    let csr = g.to_csr(&mut csr_arena).unwrap();
    let size = g.estimate_layout_arena_size_with(&config);
    let (mut t, mut o) = (vec![0u8; size], vec![0u8; size]);
    let (mut ta, mut oa) = (Arena::new(&mut t), Arena::new(&mut o));
    let ir = csr.compute_layout_arena(&config, &mut ta, &mut oa).unwrap();
    let options = RenderOptions::plain();
    let mut arena_buf = vec![0u8; ir.estimate_render_arena_size(&options)];
    let render_arena = Arena::new(&mut arena_buf);
    let mut render_buf = vec![0u8; ir.estimate_render_output_size(&options)];
    let n = ir
        .render_to_bytes(&options, &render_arena, &mut render_buf)
        .unwrap();
    assert_eq!(std::str::from_utf8(&render_buf[..n]).unwrap(), heap);
}
