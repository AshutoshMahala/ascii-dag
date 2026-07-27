//! `RenderPlan` — what to paint, resolved before any cell exists
//! (temp/06 §5, M3/R5).
//!
//! Built once per (view, options) in O(N + E + S): per-element styles
//! (the only place style fns run — Q4), a spatial index of elements
//! sorted by row range (band queries, hit-testing), edge-label
//! placement decisions, and the band partition. The plan holds plain
//! owned data — no borrows — so it is reusable across renders of the
//! same IR (R5.2).
//!
//! Label placement reproduces the legacy renderers' semantics
//! **geometrically** (no cell buffer): a label may occupy empty cells or
//! its own solid vertical only, and the colored path additionally vetoes
//! any row containing a node. Blockers are enumerated from the same path
//! geometry the painter uses. The RW3/RW4 dual-run harness arbitrates
//! any residual mismatch byte-precisely.
//!
//! Direction note: placement geometry is computed for `TopDown` layouts
//! in this phase; BottomUp compositing (RW5) extends the bend-row
//! formulas to physical BT coordinates.

use super::color::CellColor;
use super::config::RenderOptions;
use super::style::{EdgeStyle, EdgeStyleCtx, LabelPosition, LineWeight, SubgraphStyleCtx};
use super::view::{LayoutView, PathRef};
use crate::render::colors;
use alloc::vec::Vec;

/// What kind of element a spatial-index entry points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ElementKind {
    Node,
    Edge,
    Subgraph,
}

/// Spatial-index entry: one element and the rows it can touch.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PlanElement {
    pub kind: ElementKind,
    pub index: usize,
    pub y_min: usize,
    pub y_max: usize,
}

/// Result of a hit-test query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitResult {
    /// A node (by id).
    Node(usize),
    /// An edge (by edge index).
    Edge(usize),
    /// A subgraph box (by subgraph id).
    Subgraph(usize),
    /// Nothing here.
    None,
}

/// Resolved per-edge style.
#[derive(Debug, Clone, Copy)]
pub(crate) struct EdgePlan {
    pub color: CellColor,
    pub weight: LineWeight,
    pub label_color: CellColor,
}

/// Resolved per-subgraph style.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SubgraphPlan {
    pub color: CellColor,
    pub label_pos: LabelPosition,
}

/// One edge label and its placement decision.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LabelPlan {
    pub edge_index: usize,
    pub x: usize,
    pub y: usize,
    /// Span length in cells (label chars + 2 quotes).
    pub len: usize,
    /// Geometrically placeable (plain-path decision).
    pub placeable: bool,
    /// The label row hosts at least one node (colored-path veto: the
    /// legacy colored renderer skips whole rows containing nodes).
    pub row_has_node: bool,
}

impl LabelPlan {
    /// Placement under the colored path's stricter semantics.
    pub(crate) fn placed_colored(&self) -> bool {
        self.placeable && !self.row_has_node
    }
}

/// The render plan. Public read-only queries; internals private (R5).
pub struct RenderPlan {
    width: usize,
    height: usize,
    band_ranges: Vec<(usize, usize)>,
    edge_plans: Vec<EdgePlan>,
    subgraph_plans: Vec<SubgraphPlan>,
    labels: Vec<LabelPlan>,
    index: Vec<PlanElement>,
    /// Edge indices whose labels go to the legend (colored semantics).
    legend: Vec<usize>,
}

impl RenderPlan {
    /// Build a plan for `view` under `options`. O(N + E + S) plus the
    /// label-occupancy checks (O(labels × blockers)).
    pub(crate) fn build<V: LayoutView>(view: &V, options: &RenderOptions) -> RenderPlan {
        let width = view.width();
        let height = view.height();

        // ── Resolved styles (the only place style fns run — Q4) ────────
        let palette = options.palette.colors();
        let use_color = !matches!(options.color_mode, super::color::ColorMode::None);
        let edge_plans: Vec<EdgePlan> = (0..view.edge_count())
            .map(|i| {
                let e = view.edge(i);
                let ctx = EdgeStyleCtx {
                    edge_index: e.edge_index,
                    from_id: e.from_id,
                    to_id: e.to_id,
                    label: e.label,
                    directed: e.directed,
                    reversed: e.reversed,
                    total_edges: view.edge_count(),
                };
                let style: EdgeStyle = (options.edge_style_fn)(ctx);
                let color = if style.color.is_set() {
                    style.color
                } else if use_color && !palette.is_empty() {
                    // Legacy default: palette modulo by the IR edge-list
                    // index (NOT the original edge_index — self-loops are
                    // absent from the list, so positions shift; the legacy
                    // colored renderer keys by list position).
                    CellColor::ansi256(colors::get_custom(palette, i))
                } else {
                    CellColor::DEFAULT
                };
                let weight = style.weight.unwrap_or(if e.reversed {
                    LineWeight::Dashed
                } else {
                    LineWeight::Light
                });
                let label_style = (options.edge_label_style_fn)(ctx);
                let label_color = if label_style.color.is_set() {
                    label_style.color
                } else {
                    color
                };
                EdgePlan {
                    color,
                    weight,
                    label_color,
                }
            })
            .collect();

        let subgraph_plans: Vec<SubgraphPlan> = (0..view.subgraph_count())
            .map(|i| {
                let sg = view.subgraph(i);
                let ctx = SubgraphStyleCtx {
                    subgraph_id: sg.id,
                    label: sg.label,
                    has_parent: sg.parent.is_some(),
                };
                let style = (options.subgraph_style_fn)(ctx);
                SubgraphPlan {
                    color: style.color,
                    label_pos: style.label_pos,
                }
            })
            .collect();

        // ── Spatial index ──────────────────────────────────────────────
        let mut index: Vec<PlanElement> = Vec::with_capacity(
            view.node_count() + view.edge_count() + view.subgraph_count(),
        );
        for i in 0..view.node_count() {
            let n = view.node(i);
            index.push(PlanElement {
                kind: ElementKind::Node,
                index: i,
                y_min: n.y,
                y_max: n.y + n.height.saturating_sub(1),
            });
        }
        for i in 0..view.edge_count() {
            let e = view.edge(i);
            index.push(PlanElement {
                kind: ElementKind::Edge,
                index: i,
                y_min: e.from_y.min(e.to_y),
                y_max: e.from_y.max(e.to_y),
            });
        }
        for i in 0..view.subgraph_count() {
            let sg = view.subgraph(i);
            index.push(PlanElement {
                kind: ElementKind::Subgraph,
                index: i,
                y_min: sg.y,
                y_max: sg.y + sg.height.saturating_sub(1),
            });
        }
        index.sort_by_key(|e| (e.y_min, e.y_max, e.index));

        // ── Label placement (legacy semantics, geometric) ──────────────
        let mut labels: Vec<LabelPlan> = Vec::new();
        let mut legend: Vec<usize> = Vec::new();
        // Spans already claimed by earlier labels, per row.
        let mut claimed: Vec<(usize, usize, usize)> = Vec::new();

        for i in 0..view.edge_count() {
            let e = view.edge(i);
            let Some(text) = e.label else { continue };
            let len = text.chars().count() + 2;
            let (x, y) = (e.label_x, e.label_y);

            let row_has_node = (0..view.node_count()).any(|ni| {
                let n = view.node(ni);
                y >= n.y && y < n.y + n.height
            });

            let placeable = x + len <= width
                && !span_blocked(view, i, y, x, x + len, &claimed)
                && y < height;

            if placeable {
                claimed.push((y, x, x + len));
            }
            let plan = LabelPlan {
                edge_index: i,
                x,
                y,
                len,
                placeable,
                row_has_node,
            };
            if !plan.placed_colored() {
                legend.push(i);
            }
            labels.push(plan);
        }

        // ── Band partition (single full-height band until RW6) ─────────
        let band_ranges = alloc::vec![(0usize, height)];
        let _ = options.band_cap();

        RenderPlan {
            width,
            height,
            band_ranges,
            edge_plans,
            subgraph_plans,
            labels,
            index,
            legend,
        }
    }

    // ── Public read-only queries (R5) ──────────────────────────────────

    /// Rendered width in cells.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Rendered height in rows.
    pub fn height(&self) -> usize {
        self.height
    }

    /// Number of composite bands.
    pub fn band_count(&self) -> usize {
        self.band_ranges.len()
    }

    /// Edge indices whose labels go to the legend under colored
    /// rendering (in edge order).
    pub fn legend_entries(&self) -> &[usize] {
        &self.legend
    }

    /// What occupies the cell at (x, y)? Nodes win over edges, edges
    /// over subgraph boxes (matching visual z-order). Exposed publicly
    /// at RW3 integration behind per-IR wrappers (R5).
    pub(crate) fn element_at<V: LayoutView>(&self, view: &V, x: usize, y: usize) -> HitResult {
        let mut hit_subgraph = HitResult::None;
        let mut hit_edge = HitResult::None;
        for el in self.index.iter().filter(|el| y >= el.y_min && y <= el.y_max) {
            match el.kind {
                ElementKind::Node => {
                    let n = view.node(el.index);
                    if x >= n.x && x < n.x + n.width {
                        return HitResult::Node(n.id);
                    }
                }
                ElementKind::Edge => {
                    if hit_edge == HitResult::None {
                        let e = view.edge(el.index);
                        let on_vertical = x == e.from_x || x == e.to_x;
                        let on_run = h_runs_at(&e.path, e.from_x, e.from_y, e.to_x, e.to_y, y)
                            .any(|(x0, x1)| x >= x0 && x <= x1);
                        if on_vertical || on_run {
                            hit_edge = HitResult::Edge(e.edge_index);
                        }
                    }
                }
                ElementKind::Subgraph => {
                    if hit_subgraph == HitResult::None {
                        let sg = view.subgraph(el.index);
                        if x >= sg.x && x < sg.x + sg.width {
                            hit_subgraph = HitResult::Subgraph(sg.id);
                        }
                    }
                }
            }
        }
        if hit_edge != HitResult::None {
            hit_edge
        } else {
            hit_subgraph
        }
    }

    // ── Internal accessors (compositor, RW3+) ──────────────────────────

    pub(crate) fn edge_plan(&self, edge_index: usize) -> &EdgePlan {
        &self.edge_plans[edge_index]
    }

    pub(crate) fn subgraph_plan(&self, index: usize) -> &SubgraphPlan {
        &self.subgraph_plans[index]
    }

    pub(crate) fn labels(&self) -> &[LabelPlan] {
        &self.labels
    }

    pub(crate) fn elements(&self) -> &[PlanElement] {
        &self.index
    }

    pub(crate) fn band_ranges(&self) -> &[(usize, usize)] {
        &self.band_ranges
    }
}

// ── Blocker geometry (mirrors the painter's row formulas) ────────────────

/// Is any cell of `[x0, x1)` at `row` occupied by something a label may
/// not overwrite? Allowed: empty cells and solid verticals. Blockers:
/// horizontal runs (and their corners), dashed verticals, subgraph
/// border rows/columns, and spans claimed by earlier labels.
fn span_blocked<V: LayoutView>(
    view: &V,
    label_edge: usize,
    row: usize,
    x0: usize,
    x1: usize,
    claimed: &[(usize, usize, usize)],
) -> bool {
    if claimed
        .iter()
        .any(|&(r, c0, c1)| r == row && c0 < x1 && c1 > x0)
    {
        return true;
    }

    for i in 0..view.subgraph_count() {
        let sg = view.subgraph(i);
        let bottom = sg.y + sg.height.saturating_sub(1);
        if row == sg.y || row == bottom {
            if sg.x < x1 && sg.x + sg.width > x0 {
                return true;
            }
        } else if row > sg.y && row < bottom {
            let right = sg.x + sg.width.saturating_sub(1);
            if (sg.x >= x0 && sg.x < x1) || (right >= x0 && right < x1) {
                return true;
            }
        }
    }

    for i in 0..view.edge_count() {
        let e = view.edge(i);
        if row < e.from_y.min(e.to_y) || row > e.from_y.max(e.to_y) {
            continue;
        }
        // Horizontal runs (with their corner endpoints) block.
        if h_runs_at(&e.path, e.from_x, e.from_y, e.to_x, e.to_y, row)
            .any(|(r0, r1)| r0 < x1 && r1 + 1 > x0)
        {
            return true;
        }
        // Dashed verticals block ('┊' is not '│'); solid verticals are
        // allowed — including other edges' (legacy checks only the char).
        if i != label_edge && e.reversed {
            let mut cols = [usize::MAX; 8];
            let n = v_cols_at(&e.path, e.from_x, e.from_y, e.to_x, e.to_y, row, &mut cols);
            if cols[..n].iter().any(|&c| c >= x0 && c < x1) {
                return true;
            }
        }
    }
    false
}

/// Horizontal runs `[x0, x1]` (inclusive) painted by this path at `row`
/// — the same formulas the legacy painter uses.
fn h_runs_at(
    path: &PathRef<'_>,
    from_x: usize,
    from_y: usize,
    to_x: usize,
    to_y: usize,
    row: usize,
) -> impl Iterator<Item = (usize, usize)> {
    let mut runs: [(usize, usize); 4] = [(1, 0); 4];
    let mut n = 0usize;
    let mut push = |x0: usize, x1: usize| {
        if n < runs.len() {
            runs[n] = (x0.min(x1), x0.max(x1));
            n += 1;
        }
    };
    match *path {
        PathRef::Direct | PathRef::Spline { .. } => {}
        PathRef::Corner { horizontal_y } => {
            if row == horizontal_y {
                push(from_x, to_x);
            }
        }
        PathRef::SideChannel {
            channel_x,
            start_y,
            end_y,
        } => {
            if row == start_y {
                push(from_x, channel_x);
            }
            if row == end_y {
                push(to_x, channel_x);
            }
        }
        PathRef::MultiSegment {
            waypoints,
            start_y_offset,
        } => {
            let mut px = from_x;
            let mut py = from_y;
            let mut first = true;
            for i in 0..=waypoints.len() {
                let (nx, ny) = if i < waypoints.len() {
                    waypoints[i]
                } else {
                    (to_x, to_y)
                };
                if px != nx && py != ny {
                    let corner_y = py + 1 + if first { start_y_offset } else { 0 };
                    if row == corner_y {
                        push(px, nx);
                    }
                } else if py == ny && px != nx && row == py {
                    push(px, nx);
                }
                px = nx;
                py = ny;
                first = false;
            }
        }
    }
    let mut i = 0usize;
    core::iter::from_fn(move || {
        if i < n {
            let r = runs[i];
            i += 1;
            Some(r)
        } else {
            None
        }
    })
}

/// Vertical columns this path paints at `row` (between, not touching,
/// the endpoints' node rows). Writes into `out`, returns the count.
fn v_cols_at(
    path: &PathRef<'_>,
    from_x: usize,
    from_y: usize,
    to_x: usize,
    to_y: usize,
    row: usize,
    out: &mut [usize; 8],
) -> usize {
    let mut n = 0usize;
    let mut push = |c: usize| {
        if n < out.len() {
            out[n] = c;
            n += 1;
        }
    };
    match *path {
        PathRef::Direct | PathRef::Spline { .. } => {
            if row > from_y && row < to_y {
                push(from_x);
            }
        }
        PathRef::Corner { horizontal_y } => {
            if row > from_y && row < horizontal_y {
                push(from_x);
            }
            if row > horizontal_y && row < to_y {
                push(to_x);
            }
        }
        PathRef::SideChannel {
            channel_x,
            start_y,
            end_y,
        } => {
            if row > start_y && row < end_y {
                push(channel_x);
            }
            if row > end_y && row < to_y {
                push(to_x);
            }
        }
        PathRef::MultiSegment {
            waypoints,
            start_y_offset,
        } => {
            let mut px = from_x;
            let mut py = from_y;
            let mut first = true;
            for i in 0..=waypoints.len() {
                let (nx, ny) = if i < waypoints.len() {
                    waypoints[i]
                } else {
                    (to_x, to_y)
                };
                if px == nx {
                    if row > py && row < ny {
                        push(px);
                    }
                } else if py != ny {
                    let corner_y = py + 1 + if first { start_y_offset } else { 0 };
                    if row > py && row < corner_y {
                        push(px);
                    }
                    if row > corner_y && row < ny {
                        push(nx);
                    }
                }
                px = nx;
                py = ny;
                first = false;
            }
        }
    }
    n
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::algorithms::sugiyama::config::LayoutConfig;
    use crate::graph::Graph;
    use crate::render::colors::Palette;

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

    /// Two long labels forced onto the same routing row with
    /// overlapping spans — exactly one is placeable.
    fn colliding_graph() -> Graph<'static> {
        let mut g = Graph::new();
        g.add_node(1, "A");
        g.add_node(2, "B");
        g.add_node(3, "C");
        g.add_node(4, "D");
        g.add_edge(1, 3, Some("averyveryverylonglabel"));
        g.add_edge(2, 4, Some("anotherverylonglabel"));
        g
    }

    fn legacy_legend_count(g: &Graph<'_>) -> usize {
        let out = g
            .compute_layout()
            .render_scanline_colored_with_legend(Palette::Ansi);
        match out.split("Edge labels:").nth(1) {
            Some(rest) => rest.lines().filter(|l| !l.trim().is_empty()).count(),
            None => 0,
        }
    }

    #[test]
    fn placement_matches_legacy_legend_no_collision() {
        let g = stage_graph();
        let ir = g.compute_layout();
        let plan = RenderPlan::build(&ir, &RenderOptions::colored(Palette::Ansi));
        assert_eq!(
            plan.legend_entries().len(),
            legacy_legend_count(&g),
            "stage graph: label places cleanly in legacy → plan agrees"
        );
        assert_eq!(plan.labels().len(), 1);
        assert!(plan.labels()[0].placeable);
    }

    #[test]
    fn placement_matches_legacy_legend_with_collision() {
        let g = colliding_graph();
        let ir = g.compute_layout();
        let plan = RenderPlan::build(&ir, &RenderOptions::colored(Palette::Ansi));
        let legacy = legacy_legend_count(&g);
        assert_eq!(
            plan.legend_entries().len(),
            legacy,
            "colliding graph: plan legend must match legacy legend"
        );
        assert_eq!(plan.labels().len(), 2);
    }

    #[test]
    fn edge_styles_match_legacy_palette_assignment() {
        let g = stage_graph();
        let ir = g.compute_layout();
        let plan = RenderPlan::build(&ir, &RenderOptions::colored(Palette::Ansi));
        let palette = Palette::Ansi.colors();
        let legacy = ir.compute_edge_colors(palette.len());
        for (i, want_idx) in legacy.iter().enumerate() {
            assert_eq!(
                plan.edge_plan(i).color,
                super::super::color::CellColor::ansi256(palette[*want_idx % palette.len()]),
                "edge {i} palette color"
            );
        }
        // Plain mode resolves no colors at all.
        let plain = RenderPlan::build(&ir, &RenderOptions::plain());
        assert!(!plain.edge_plan(0).color.is_set());
    }

    #[test]
    fn hit_testing_finds_nodes_boxes_and_nothing() {
        let g = stage_graph();
        let ir = g.compute_layout();
        let plan = RenderPlan::build(&ir, &RenderOptions::plain());

        let start = ir.node_by_id(1).unwrap();
        assert_eq!(
            plan.element_at(&ir, start.x + 1, start.y),
            HitResult::Node(1)
        );
        let sg = &ir.subgraphs()[0];
        // A border cell of the box that hosts no node or edge column.
        assert!(matches!(
            plan.element_at(&ir, sg.x, sg.y),
            HitResult::Subgraph(0) | HitResult::Edge(_)
        ));
        assert_eq!(
            plan.element_at(&ir, plan.width().saturating_sub(1), 0),
            HitResult::None
        );
    }

    #[test]
    fn spatial_index_is_sorted_and_single_band() {
        let g = stage_graph();
        let ir = g.compute_layout();
        let plan = RenderPlan::build(&ir, &RenderOptions::plain());
        assert!(
            plan.elements()
                .windows(2)
                .all(|w| w[0].y_min <= w[1].y_min)
        );
        assert_eq!(plan.band_count(), 1);
        assert_eq!(plan.band_ranges()[0], (0, plan.height()));
        assert_eq!(plan.width(), ir.width());
        assert_eq!(plan.height(), ir.height());
    }

    #[test]
    fn reversed_edges_resolve_dashed() {
        let mut g = Graph::new();
        g.add_node(1, "A");
        g.add_node(2, "B");
        g.add_edge(1, 2, None);
        g.add_edge(2, 1, None); // back edge
        let ir = g.compute_layout();
        let plan = RenderPlan::build(&ir, &RenderOptions::plain());
        let reversed: Vec<bool> = ir.edges().iter().map(|e| e.reversed).collect();
        for (i, rev) in reversed.iter().enumerate() {
            let want = if *rev {
                LineWeight::Dashed
            } else {
                LineWeight::Light
            };
            assert_eq!(plan.edge_plan(i).weight, want, "edge {i}");
        }
    }

    #[test]
    fn config_defaults_are_config() {
        let mut config = LayoutConfig::standard();
        config.include_dummy_nodes = true;
        let g = stage_graph();
        let ir = g.compute_layout_with_config(&config);
        // Plan builds fine over an IR containing dummy nodes.
        let plan = RenderPlan::build(&ir, &RenderOptions::plain());
        assert_eq!(plan.band_count(), 1);
    }
}
