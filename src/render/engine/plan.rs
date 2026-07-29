//! `RenderPlan` — what to paint, resolved before any cell exists
//! (temp/06 §5, M3/R5).
//!
//! Built once per (view, options) in O(N + E + S): per-element styles
//! (the only place style fns run — Q4), a spatial index of elements
//! sorted by row range (band queries, hit-testing), edge-label
//! placement decisions, and the band partition. Storage is heap-owned
//! or arena-carved (`PlanBuf`); either way the plan borrows nothing
//! from the view, so it is reusable across renders of the same IR
//! (R5.2).
//!
//! Label placement reproduces the legacy renderers' semantics
//! **geometrically** (no cell buffer): a label may occupy empty cells or
//! its own solid vertical only, and the colored path additionally vetoes
//! any row containing a node. Blockers are enumerated from the same path
//! geometry the painter uses. The RW3/RW4 dual-run harness arbitrates
//! any residual mismatch byte-precisely.
//!
//! Placement geometry is direction-generic: bend rows and vertical
//! spans derive their flow sign from each edge's own coordinates,
//! mirroring the compositor (M4).

use super::color::CellColor;
use super::config::RenderOptions;
use super::mem::PlanBuf;
use super::style::{
    EdgeStyle, EdgeStyleCtx, LabelPosition, LineWeight, MarkerShape, NodeBorder, NodeStyleCtx,
    SubgraphBorder, SubgraphStyleCtx,
};
use super::view::{LayoutView, PathRef};
use crate::graph::arena::Arena;
use crate::render::colors;
use crate::GraphError;

/// Where plan storage comes from: the heap (std/alloc convenience) or a
/// caller-provided arena (the no-alloc surface, N2).
pub(crate) enum PlanMem<'m, 'buf> {
    /// Heap-backed buffers.
    #[cfg(feature = "alloc")]
    Heap,
    /// Carve every buffer from this arena.
    Arena(&'m Arena<'buf>),
}

impl<'m, 'buf> PlanMem<'m, 'buf> {
    fn buf<T: Copy + Default>(
        &self,
        capacity: usize,
        on_oom: GraphError,
    ) -> Result<PlanBuf<'buf, T>, GraphError> {
        match self {
            #[cfg(feature = "alloc")]
            PlanMem::Heap => Ok(PlanBuf::heap(capacity)),
            PlanMem::Arena(a) => PlanBuf::carve(a, capacity, on_oom),
        }
    }
}

/// What kind of element a spatial-index entry points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ElementKind {
    #[default]
    Node,
    Edge,
    Subgraph,
}

/// Spatial-index entry: one element and the rows it can touch.
#[derive(Debug, Clone, Copy, Default)]
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
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct EdgePlan {
    pub color: CellColor,
    pub weight: LineWeight,
    pub label_color: CellColor,
    /// Marker at the logical target end (legacy: always an arrowhead).
    pub marker_end: MarkerShape,
    /// Marker at the logical source end (legacy: never painted).
    pub marker_start: MarkerShape,
}

/// Resolved per-node style.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct NodePlan {
    pub border: NodeBorder,
    pub text_color: CellColor,
}

/// Resolved per-subgraph style.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SubgraphPlan {
    pub border: SubgraphBorder,
    /// Border color; `DEFAULT` keeps legacy behavior (borders never
    /// write the color plane).
    pub color: CellColor,
    pub label_pos: LabelPosition,
}

/// One edge label and its placement decision.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct LabelPlan {
    pub edge_index: usize,
    pub x: usize,
    pub y: usize,
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
/// Storage is heap- or arena-backed behind [`PlanBuf`] — one build
/// path serves std and no-alloc callers alike.
pub struct RenderPlan<'buf> {
    width: usize,
    height: usize,
    band_ranges: PlanBuf<'buf, (usize, usize)>,
    edge_plans: PlanBuf<'buf, EdgePlan>,
    subgraph_plans: PlanBuf<'buf, SubgraphPlan>,
    node_plans: PlanBuf<'buf, NodePlan>,
    labels: PlanBuf<'buf, LabelPlan>,
    index: PlanBuf<'buf, PlanElement>,
    /// Edge indices whose labels go to the legend (colored semantics).
    legend: PlanBuf<'buf, usize>,
    /// Exact count of h-run interiors across all edge paths — sizes the
    /// compositor's run scratch.
    run_capacity: usize,
}

impl<'buf> RenderPlan<'buf> {
    /// Build a heap-backed plan (std/alloc convenience). Heap pushes
    /// cannot fail, so this surface stays infallible.
    #[cfg(feature = "alloc")]
    pub(crate) fn build<V: LayoutView>(view: &V, options: &RenderOptions) -> RenderPlan<'static> {
        match RenderPlan::<'static>::build_impl(view, options, PlanMem::Heap) {
            Ok(plan) => plan,
            // Heap-backed building has no failing carve.
            Err(_) => unreachable!(),
        }
    }

    /// Build a plan whose storage is carved from `arena` (the no-alloc
    /// surface, N2). Exhaustion maps to `E.Render.Plan.026` — size the
    /// arena with `estimate_render_arena_size`.
    pub(crate) fn build_in<V: LayoutView>(
        view: &V,
        options: &RenderOptions,
        arena: &Arena<'buf>,
    ) -> Result<RenderPlan<'buf>, GraphError> {
        Self::build_impl(view, options, PlanMem::Arena(arena))
    }

    /// The one build path. O(N + E + S) plus the label-occupancy checks
    /// (O(labels × blockers)); every buffer's capacity is computed
    /// exactly before it is created.
    fn build_impl<V: LayoutView>(
        view: &V,
        options: &RenderOptions,
        mem: PlanMem<'_, 'buf>,
    ) -> Result<RenderPlan<'buf>, GraphError> {
        let width = view.width();
        let height = view.height();
        let oom = || GraphError::RenderPlanOom;

        // ── Resolved styles (the only place style fns run — Q4) ────────
        let palette = options.palette.colors();
        let use_color = !matches!(options.color_mode, super::color::ColorMode::None);
        let mut edge_plans: PlanBuf<'buf, EdgePlan> = mem.buf(view.edge_count(), oom())?;
        for i in 0..view.edge_count() {
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
            edge_plans.push(EdgePlan {
                color,
                weight,
                label_color,
                marker_end: style.marker_end,
                marker_start: style.marker_start,
            });
        }

        let mut subgraph_plans: PlanBuf<'buf, SubgraphPlan> =
            mem.buf(view.subgraph_count(), oom())?;
        for i in 0..view.subgraph_count() {
            let sg = view.subgraph(i);
            let ctx = SubgraphStyleCtx {
                subgraph_id: sg.id,
                label: sg.label,
                has_parent: sg.parent.is_some(),
            };
            let style = (options.subgraph_style_fn)(ctx);
            subgraph_plans.push(SubgraphPlan {
                border: style.border,
                color: style.color,
                label_pos: style.label_pos,
            });
        }

        let mut node_plans: PlanBuf<'buf, NodePlan> = mem.buf(view.node_count(), oom())?;
        for i in 0..view.node_count() {
            let n = view.node(i);
            // Dummies are markers, not styleable nodes — the style fn
            // never sees them.
            if matches!(n.kind, crate::ir::NodeKind::Dummy) {
                node_plans.push(NodePlan::default());
                continue;
            }
            let ctx = NodeStyleCtx {
                node_id: n.id,
                label: n.label,
                is_implicit: matches!(n.kind, crate::ir::NodeKind::Implicit),
                has_self_loop: n.has_self_loop,
                total_nodes: view.node_count(),
            };
            let style = (options.node_style_fn)(ctx);
            node_plans.push(NodePlan {
                border: style.border,
                text_color: style.text_color,
            });
        }

        // ── Spatial index + run capacity ───────────────────────────────
        let mut index: PlanBuf<'buf, PlanElement> = mem.buf(
            view.node_count() + view.edge_count() + view.subgraph_count(),
            oom(),
        )?;
        let mut run_capacity = 0usize;
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
            let (y_min, y_max) = edge_row_span(&e.path, e.from_y, e.to_y);
            run_capacity += count_h_runs(&e.path, e.from_x, e.to_x);
            index.push(PlanElement {
                kind: ElementKind::Edge,
                index: i,
                y_min,
                y_max,
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
        index
            .as_mut_slice()
            .sort_unstable_by_key(|e| (e.y_min, e.y_max, e.index));

        // ── Label placement (legacy semantics, geometric) ──────────────
        let labeled = (0..view.edge_count())
            .filter(|&i| view.edge(i).label.is_some())
            .count();
        let mut labels: PlanBuf<'buf, LabelPlan> = mem.buf(labeled, oom())?;
        let mut legend: PlanBuf<'buf, usize> = mem.buf(labeled, oom())?;
        // Spans already claimed by earlier labels, per row.
        let mut claimed: PlanBuf<'buf, (usize, usize, usize)> = mem.buf(labeled, oom())?;

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
                && !span_blocked(view, i, y, x, x + len, claimed.as_slice())
                && y < height;

            if placeable {
                claimed.push((y, x, x + len));
            }
            let plan = LabelPlan {
                edge_index: i,
                x,
                y,
                placeable,
                row_has_node,
            };
            if !plan.placed_colored() {
                legend.push(i);
            }
            labels.push(plan);
        }

        // ── Band partition (Q1: level-aligned, capped) ─────────────────
        // Boundaries prefer level tops (distinct node rows) so bands
        // don't split levels; a level chunk taller than the cap is
        // hard-cut at the cap. Elements spanning a boundary are simply
        // replayed in every band they intersect — canvas clipping makes
        // out-of-band writes no-ops. The partition runs twice: a count
        // pass sizes the buffer exactly, then a fill pass stores it.
        let cap = options.band_cap();
        let mut tops: PlanBuf<'buf, usize> = mem.buf(view.node_count(), oom())?;
        for i in 0..view.node_count() {
            tops.push(view.node(i).y);
        }
        tops.as_mut_slice().sort_unstable();
        let tops = dedup_in_place(&mut tops);
        let mut band_count = 0usize;
        partition_bands(height, cap, tops, |_, _| band_count += 1);
        let mut band_ranges: PlanBuf<'buf, (usize, usize)> = mem.buf(band_count, oom())?;
        partition_bands(height, cap, tops, |y0, rows| band_ranges.push((y0, rows)));

        Ok(RenderPlan {
            width,
            height,
            band_ranges,
            edge_plans,
            subgraph_plans,
            node_plans,
            labels,
            index,
            legend,
            run_capacity,
        })
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
        self.legend.as_slice()
    }

    /// What occupies the cell at (x, y)? Nodes win over edges, edges
    /// over subgraph boxes (matching visual z-order). Exposed publicly
    /// at RW3 integration behind per-IR wrappers (R5).
    pub(crate) fn element_at<V: LayoutView>(&self, view: &V, x: usize, y: usize) -> HitResult {
        let mut hit_subgraph = HitResult::None;
        let mut hit_edge = HitResult::None;
        for el in self
            .index
            .as_slice()
            .iter()
            .filter(|el| y >= el.y_min && y <= el.y_max)
        {
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
        &self.edge_plans.as_slice()[edge_index]
    }

    pub(crate) fn subgraph_plan(&self, index: usize) -> &SubgraphPlan {
        &self.subgraph_plans.as_slice()[index]
    }

    pub(crate) fn node_plan(&self, index: usize) -> &NodePlan {
        &self.node_plans.as_slice()[index]
    }

    pub(crate) fn labels(&self) -> &[LabelPlan] {
        self.labels.as_slice()
    }

    #[cfg(test)]
    pub(crate) fn elements(&self) -> &[PlanElement] {
        self.index.as_slice()
    }

    /// Elements whose row range intersects `[y0, y0 + rows)`. The index
    /// is sorted by `y_min`, so entries past the band are cut off by
    /// binary search; the prefix is filtered on `y_max`.
    pub(crate) fn elements_in_band(
        &self,
        y0: usize,
        rows: usize,
    ) -> impl Iterator<Item = &PlanElement> {
        let last = y0 + rows.saturating_sub(1);
        let slice = self.index.as_slice();
        let ub = slice.partition_point(|el| el.y_min <= last);
        slice[..ub].iter().filter(move |el| el.y_max >= y0)
    }

    /// Band list as `(first_row, rows)` pairs covering `0..height`.
    pub(crate) fn band_ranges(&self) -> &[(usize, usize)] {
        self.band_ranges.as_slice()
    }

    /// Rows of the tallest band — the reusable band buffer's height.
    pub(crate) fn max_band_rows(&self) -> usize {
        self.band_ranges.as_slice().iter().map(|b| b.1).max().unwrap_or(0)
    }

    /// Exact h-run interior count — sizes the compositor's run scratch.
    pub(crate) fn run_capacity(&self) -> usize {
        self.run_capacity
    }
}

/// Emit the level-aligned band partition as `(y0, rows)` pairs.
fn partition_bands(height: usize, cap: usize, tops: &[usize], mut emit: impl FnMut(usize, usize)) {
    if height <= cap {
        emit(0, height);
        return;
    }
    let mut start = 0usize;
    while start < height {
        let cap_end = start + cap;
        if cap_end >= height {
            emit(start, height - start);
            break;
        }
        let ub = tops.partition_point(|&t| t <= cap_end);
        let boundary = tops[..ub]
            .iter()
            .rev()
            .find(|&&t| t > start)
            .copied()
            .unwrap_or(cap_end);
        emit(start, boundary - start);
        start = boundary;
    }
}

/// Dedup a sorted `PlanBuf` in place, returning the deduped prefix.
fn dedup_in_place<'a, 'buf>(buf: &'a mut PlanBuf<'buf, usize>) -> &'a [usize] {
    let s = buf.as_mut_slice();
    let mut w = 0usize;
    for r in 0..s.len() {
        if w == 0 || s[r] != s[w - 1] {
            s[w] = s[r];
            w += 1;
        }
    }
    &buf.as_slice()[..w]
}

/// The rows a path can paint, from the same formulas the painter uses
/// (bend rows included — a band must replay every edge that touches it).
fn edge_row_span(path: &PathRef<'_>, from_y: usize, to_y: usize) -> (usize, usize) {
    let mut lo = from_y.min(to_y);
    let mut hi = from_y.max(to_y);
    let mut take = |y: usize| {
        lo = lo.min(y);
        hi = hi.max(y);
    };
    match *path {
        PathRef::Direct | PathRef::Spline { .. } => {}
        PathRef::Corner { horizontal_y } => take(horizontal_y),
        PathRef::SideChannel { start_y, end_y, .. } => {
            take(start_y);
            take(end_y);
        }
        PathRef::MultiSegment {
            waypoints,
            start_y_offset,
        } => {
            for &(_, wy) in waypoints {
                take(wy);
            }
            // The first bend row can sit below the source row block.
            let dir: isize = if to_y >= from_y { 1 } else { -1 };
            let first_bend = (from_y as isize + dir * (1 + start_y_offset as isize)) as usize;
            take(first_bend);
        }
    }
    (lo, hi)
}

/// Bytes of arena [`RenderPlan::build_in`] plus the compositor's carve
/// calls need for this view and options — plan storage, paint scratch,
/// and the band canvas planes, with alignment slack (N2).
///
/// The band-list term uses a proven bound instead of a dry run: a band
/// shorter than the cap always ends on a level top with no further top
/// in its window, forcing the next advance toward the cap — so at most
/// two bands fit per cap window, plus edge slack.
pub(crate) fn estimate_plan_bytes<V: LayoutView>(view: &V, options: &RenderOptions) -> usize {
    use core::mem::size_of;
    let n = view.node_count();
    let e = view.edge_count();
    let s = view.subgraph_count();
    let width = view.width();
    let height = view.height();
    let cap = options.band_cap();
    let colored = !matches!(options.color_mode, super::color::ColorMode::None);
    let labeled = (0..e).filter(|&i| view.edge(i).label.is_some()).count();
    let mut run_capacity = 0usize;
    for i in 0..e {
        let ed = view.edge(i);
        run_capacity += count_h_runs(&ed.path, ed.from_x, ed.to_x);
    }
    let bands = 2 * height.div_ceil(cap) + 2;
    let band_rows = cap.min(height).max(1);
    let area = width * band_rows;

    let plan_bytes = e * size_of::<EdgePlan>()
        + s * size_of::<SubgraphPlan>()
        + n * size_of::<NodePlan>()
        + (n + e + s) * size_of::<PlanElement>()
        + labeled * (size_of::<LabelPlan>() + size_of::<usize>() + size_of::<(usize, usize, usize)>())
        + n * size_of::<usize>()
        + bands * size_of::<(usize, usize)>();
    let scratch_bytes = super::compose::PaintScratch::estimate_bytes(
        run_capacity,
        s,
        e,
        n,
        colored,
        width,
        band_rows,
    );
    let canvas_bytes = area * size_of::<super::cell::Cell>()
        + if colored {
            area * size_of::<CellColor>()
        } else {
            0
        };
    // Per-allocation alignment slack (≤ 8 bytes × ~16 carves) + margin.
    plan_bytes + scratch_bytes + canvas_bytes + 16 * 8 + 64
}

/// Structural count of horizontal-run interiors a path paints — the
/// same segments `h_run_with_corners` collects, counted without cells.
fn count_h_runs(path: &PathRef<'_>, from_x: usize, to_x: usize) -> usize {
    match *path {
        PathRef::Direct | PathRef::Spline { .. } => 0,
        PathRef::Corner { .. } => 1,
        PathRef::SideChannel { .. } => 2,
        PathRef::MultiSegment { waypoints, .. } => {
            let mut count = 0usize;
            let mut px = from_x;
            for i in 0..=waypoints.len() {
                let nx = if i < waypoints.len() {
                    waypoints[i].0
                } else {
                    to_x
                };
                if px != nx {
                    count += 1;
                }
                px = nx;
            }
            count
        }
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
            // Flow sign from geometry (mirrors the compositor exactly).
            let dir: isize = if to_y >= from_y { 1 } else { -1 };
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
                    let step = 1 + if first { start_y_offset as isize } else { 0 };
                    let corner_y = (py as isize + dir * step) as usize;
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
    // Order-free strictly-between test (works for either flow).
    let betw = |a: usize, b: usize, r: usize| r > a.min(b) && r < a.max(b);
    match *path {
        PathRef::Direct | PathRef::Spline { .. } => {
            if betw(from_y, to_y, row) {
                push(from_x);
            }
        }
        PathRef::Corner { horizontal_y } => {
            if betw(from_y, horizontal_y, row) {
                push(from_x);
            }
            if betw(horizontal_y, to_y, row) {
                push(to_x);
            }
        }
        PathRef::SideChannel {
            channel_x,
            start_y,
            end_y,
        } => {
            if betw(start_y, end_y, row) {
                push(channel_x);
            }
            if betw(end_y, to_y, row) {
                push(to_x);
            }
        }
        PathRef::MultiSegment {
            waypoints,
            start_y_offset,
        } => {
            let dir: isize = if to_y >= from_y { 1 } else { -1 };
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
                    if betw(py, ny, row) {
                        push(px);
                    }
                } else if py != ny {
                    let step = 1 + if first { start_y_offset as isize } else { 0 };
                    let corner_y = (py as isize + dir * step) as usize;
                    if betw(py, corner_y, row) {
                        push(px);
                    }
                    if betw(corner_y, ny, row) {
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
