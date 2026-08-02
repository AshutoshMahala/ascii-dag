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
    EdgeStyle, EdgeStyleCtx, LabelPosition, LineWeight, MarkerShape, SubgraphBorder,
    SubgraphStyleCtx,
};
use super::view::{LayoutView, PathRef};
use crate::GraphError;
use crate::graph::arena::Arena;
use crate::render::colors;

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
#[non_exhaustive]
pub enum HitResult {
    /// A node (by id).
    Node(usize),
    /// An edge, by its IR-list index (the position in the layout's
    /// edge list — the same convention as `legend_entries`).
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

/// The render plan. Public read-only queries; internals private.
/// Storage is heap- or arena-backed behind `PlanBuf` — one build
/// path serves std and no-alloc callers alike.
///
/// A plan is a snapshot for **introspection** (dimensions, bands,
/// legend, hit-testing) of the layout and options it was built from;
/// the render entry points build their own plan internally. Queries
/// must be paired with the same layout the plan was built from —
/// out-of-canvas queries return `HitResult::None`.
pub struct RenderPlan<'buf> {
    width: usize,
    height: usize,
    band_ranges: PlanBuf<'buf, (usize, usize)>,
    edge_plans: PlanBuf<'buf, EdgePlan>,
    subgraph_plans: PlanBuf<'buf, SubgraphPlan>,
    labels: PlanBuf<'buf, LabelPlan>,
    index: PlanBuf<'buf, PlanElement>,
    /// Edge indices whose labels go to the legend (colored semantics).
    legend: PlanBuf<'buf, usize>,
    /// Exact count of h-run interiors across all edge paths — sizes the
    /// compositor's run scratch.
    run_capacity: usize,
    /// Whether dummy nodes paint (hit-testing must agree with the render).
    show_dummy_nodes: bool,
    /// Whether label placement uses the colored-with-legend gate
    /// (hit-testing must agree with the compositor).
    labels_colored_gate: bool,
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
    /// — `O(labels × blockers)`, with D9's cross-host search bounded
    /// per segment by `CROSS_HOST_CANDIDATES` so graph height cannot
    /// multiply it. Every buffer's capacity is computed exactly before
    /// it is created.
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

        // ── Spatial index + run capacity ───────────────────────────────
        let mut index: PlanBuf<'buf, PlanElement> = mem.buf(
            view.node_count() + view.edge_count() + view.subgraph_count(),
            oom(),
        )?;
        let mut run_capacity = 0usize;
        for i in 0..view.node_count() {
            let n = view.node(i);
            // The self-loop marker cell can sit outside the node's own
            // rows (below it, for horizontal flows) — the index span
            // must cover it or hit-testing prunes the node early.
            let mut y_min = n.y;
            let mut y_max = n.y + n.height.saturating_sub(1);
            if let Some((_, my)) = n.self_loop_at {
                y_min = y_min.min(my);
                y_max = y_max.max(my);
            }
            index.push(PlanElement {
                kind: ElementKind::Node,
                index: i,
                y_min,
                y_max,
            });
        }
        for i in 0..view.edge_count() {
            let e = view.edge(i);
            let (y_min, y_max) = edge_row_span(&e.path, e.from_y, e.to_y, e.flow_axis);
            run_capacity += count_h_runs(&e.path, e.from_x, e.to_x, e.flow_axis);
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
            let (mut x, mut y) = (e.label_x, e.label_y);
            let mut placeable = false;
            let is_x = matches!(e.flow_axis, crate::ir::FlowAxis::X);
            // Centering a span on an anchor is ambiguous at even
            // widths, and the naive `len / 2` is not mirror-stable: an
            // RL layout would land one cell left of the exact mirror
            // of its LR twin. Bias the lead by the physical flow sign
            // so `RL span == mirror(LR span)` exactly, at every width.
            let rightward = e.to_x >= e.from_x;
            let lead = if rightward {
                len / 2
            } else {
                len.saturating_sub(1) / 2
            };

            // ── D9 host ladder ──
            // Host 1 — the edge's OWN cross (vertical) segment: the
            // direct mirror of the TD picture, where the label
            // interrupts the line it annotates and spreads sideways
            // over empty cells. X-only: a Y flow's seed already lands
            // on its own flow segment, so Y starts at host 2 and its
            // output stays byte-frozen.
            if is_x {
                let mut chosen: Option<(usize, usize)> = None;
                for_each_x_cross_segment(
                    &e.path,
                    e.from_x,
                    e.from_y,
                    e.to_x,
                    e.to_y,
                    &mut |col, seg_from, seg_to| {
                        if chosen.is_some() {
                            return;
                        }
                        let cx = col.saturating_sub(lead);
                        if cx + len > width {
                            return;
                        }
                        // Walk interior rows outward from the SOURCE
                        // end (mirroring TD, whose label sits just
                        // past its source), bounded by
                        // `CROSS_HOST_CANDIDATES`.
                        let step: isize = if seg_to >= seg_from { 1 } else { -1 };
                        for k in 1..=CROSS_HOST_CANDIDATES {
                            let r = seg_from as isize + step * k as isize;
                            if r < 0 {
                                break;
                            }
                            let row = r as usize;
                            // Past the far end — this segment is done.
                            if (step > 0 && row >= seg_to) || (step < 0 && row <= seg_to) {
                                break;
                            }
                            if row < height
                                && !span_blocked(view, i, row, cx, cx + len, claimed.as_slice())
                            {
                                chosen = Some((cx, row));
                                return;
                            }
                        }
                    },
                );
                if let Some((cx, cy)) = chosen {
                    x = cx;
                    y = cy;
                    placeable = true;
                }
            }

            // Host 2 — inline at the layout's seed (on the trunk for X
            // flows; the classic row-below-the-source for Y).
            if !placeable {
                x = e.label_x;
                y = e.label_y;
                placeable = x + len <= width
                    && !span_blocked(view, i, y, x, x + len, claimed.as_slice())
                    && y < height;
            }

            // Host 3 (X only) — the adjacent-row FLOAT above the
            // source trunk, centered on the endpoint gap: borrows
            // empty cells over the node columns, never widens
            // anything. The legend remains the final fallback.
            if !placeable && is_x && e.from_y >= 1 {
                // The gap midpoint has the same tie at odd widths:
                // round toward the flow so the mirror stays exact.
                let gap = e.from_x.abs_diff(e.to_x);
                let half = if rightward { gap / 2 } else { gap.div_ceil(2) };
                let mid = (e.from_x.min(e.to_x) + half).max(lead);
                let fx = mid - lead;
                let fy = e.from_y - 1;
                if (fx, fy) != (x, y)
                    && fx + len <= width
                    && fy < height
                    && !span_blocked(view, i, fy, fx, fx + len, claimed.as_slice())
                {
                    x = fx;
                    y = fy;
                    placeable = true;
                }
            }

            let row_has_node = (0..view.node_count()).any(|ni| {
                let n = view.node(ni);
                y >= n.y && y < n.y + n.height
            });

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
            // The legend lists exactly the labels that did NOT paint
            // under the active gate (colored: row-veto; plain:
            // geometric placement) — `legend_entries` reflects the
            // options this plan was built with.
            let painted = if use_color {
                plan.placed_colored()
            } else {
                plan.placeable
            };
            if options.legend && !painted {
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
            labels,
            index,
            legend,
            run_capacity,
            show_dummy_nodes: options.show_dummy_nodes,
            labels_colored_gate: use_color && options.legend,
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

    /// Edge indices (IR-list order) whose labels go to the legend under
    /// the options this plan was built with: the labels that did not
    /// paint under the active placement gate (colored: the row-veto
    /// rule; plain: geometric placement). Empty unless the options
    /// enable the legend — matching what actually renders.
    pub fn legend_entries(&self) -> &[usize] {
        self.legend.as_slice()
    }

    /// What occupies the cell at (x, y)? Nodes win over edges, edges
    /// over subgraph boxes (matching visual z-order).
    ///
    /// Semantics are deliberately hybrid, serving interactive picking:
    /// **edges hit as painted ink** (the exact run/column formulas the
    /// compositor paints; hidden dummies never hit; a
    /// `SubgraphBorder::None` box has no ink to hit), while **nodes and
    /// bordered subgraphs hit as layout regions** (a node's declared
    /// width including padding; a box's full rectangle including its
    /// interior) — clicking inside a box selects it even on a blank
    /// cell. Edge labels, box labels, and self-loop markers are part of
    /// their owning element's region, not separate hit targets.
    /// Edges are reported by their **IR-list index** (the position in
    /// the layout's edge list — the same convention as
    /// [`RenderPlan::legend_entries`]).
    pub(crate) fn element_at<V: LayoutView>(&self, view: &V, x: usize, y: usize) -> HitResult {
        // A plan answers only for the layout it was built from; a query
        // outside this plan's canvas (including any query against a
        // *different* layout's larger canvas) is `None`, never a panic.
        if x >= self.width || y >= self.height {
            return HitResult::None;
        }
        let mut hit_subgraph = HitResult::None;
        let mut hit_edge = HitResult::None;
        // Painted edge labels belong to their edge (they render above
        // edge ink). The same placement gate the compositor uses.
        for label in self.labels.as_slice() {
            let placed = if self.labels_colored_gate {
                label.placed_colored()
            } else {
                label.placeable
            };
            if placed && y == label.y && x >= label.x && x < label.x + label.len {
                return HitResult::Edge(label.edge_index);
            }
        }
        for el in self
            .index
            .as_slice()
            .iter()
            .filter(|el| y >= el.y_min && y <= el.y_max)
        {
            match el.kind {
                ElementKind::Node => {
                    let n = view.node(el.index);
                    if matches!(n.kind, crate::ir::NodeKind::Dummy) {
                        // A dummy is a single marker cell, and only
                        // when the render shows it.
                        if self.show_dummy_nodes && x == n.x && y == n.y {
                            return HitResult::Node(n.id);
                        }
                        continue;
                    }
                    // The node owns its full reserved area (painters
                    // may fill any of it); the self-loop marker (`↺`)
                    // adds its IR cell (right of the top row for
                    // vertical flows, below the leading column for
                    // horizontal ones).
                    let in_rows = y >= n.y && y < n.y + n.height.max(1);
                    if (in_rows && x >= n.x && x < n.x + n.width) || n.self_loop_at == Some((x, y))
                    {
                        return HitResult::Node(n.id);
                    }
                }
                ElementKind::Edge => {
                    if hit_edge == HitResult::None {
                        let e = view.edge(el.index);
                        let mut on_ink = false;
                        for_each_v_col(
                            &e.path,
                            e.from_x,
                            e.from_y,
                            e.to_x,
                            e.to_y,
                            y,
                            e.flow_axis,
                            &mut |c| {
                                on_ink |= c == x;
                            },
                        );
                        for_each_h_run(
                            &e.path,
                            e.from_x,
                            e.from_y,
                            e.to_x,
                            e.to_y,
                            y,
                            e.flow_axis,
                            &mut |x0, x1| on_ink |= x >= x0 && x <= x1,
                        );
                        if on_ink {
                            hit_edge = HitResult::Edge(el.index);
                        }
                    }
                }
                ElementKind::Subgraph => {
                    if hit_subgraph == HitResult::None {
                        let sg = view.subgraph(el.index);
                        let sp = self.subgraph_plan(el.index);
                        if matches!(sp.border, super::style::SubgraphBorder::None) {
                            // No box ink — but the label still paints
                            // and belongs to the cluster.
                            if sg.width >= 4 && sg.height >= 3 && !sg.label.is_empty() {
                                let label_y = match sp.label_pos {
                                    super::style::LabelPosition::InsideTop => sg.y + 1,
                                    super::style::LabelPosition::InsideBottom => {
                                        (sg.y + sg.height).saturating_sub(2)
                                    }
                                };
                                let len = sg.label.chars().count().min(sg.width - 4);
                                if y == label_y && x >= sg.x + 2 && x < sg.x + 2 + len {
                                    hit_subgraph = HitResult::Subgraph(sg.id);
                                }
                            }
                        } else if x >= sg.x && x < sg.x + sg.width {
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

    pub(crate) fn labels(&self) -> &[LabelPlan] {
        self.labels.as_slice()
    }

    /// The spatial index: every element with its row range, sorted by
    /// `(y_min, y_max, index)`. The compositor's rolling band sweep
    /// walks it in order — per-band membership is never re-derived by
    /// scanning a prefix.
    pub(crate) fn elements(&self) -> &[PlanElement] {
        self.index.as_slice()
    }

    /// Band list as `(first_row, rows)` pairs covering `0..height`.
    pub(crate) fn band_ranges(&self) -> &[(usize, usize)] {
        self.band_ranges.as_slice()
    }

    /// Rows of the tallest band — the reusable band buffer's height.
    pub(crate) fn max_band_rows(&self) -> usize {
        self.band_ranges
            .as_slice()
            .iter()
            .map(|b| b.1)
            .max()
            .unwrap_or(0)
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
fn edge_row_span(
    path: &PathRef<'_>,
    from_y: usize,
    to_y: usize,
    axis: crate::ir::FlowAxis,
) -> (usize, usize) {
    let mut lo = from_y.min(to_y);
    let mut hi = from_y.max(to_y);
    let mut take = |y: usize| {
        lo = lo.min(y);
        hi = hi.max(y);
    };
    // X flows: rows are the trunk rows — ports plus every waypoint's
    // row (waypoint excursions exceed the port span; bends are
    // columns and add no rows).
    if matches!(axis, crate::ir::FlowAxis::X) {
        match *path {
            PathRef::MultiSegment { waypoints, .. } => {
                for &(_, wy) in waypoints {
                    take(wy);
                }
            }
            // The far channel is a ROW under X flow — it can sit
            // outside the endpoint rows entirely.
            PathRef::SideChannel { channel_at, .. } => take(channel_at),
            _ => {}
        }
        return (lo, hi);
    }
    match *path {
        PathRef::Direct | PathRef::Spline { .. } => {}
        PathRef::Corner { bend_at } => take(bend_at),
        PathRef::SideChannel {
            span_start,
            span_end,
            ..
        } => {
            take(span_start);
            take(span_end);
        }
        PathRef::MultiSegment {
            waypoints,
            start_offset,
        } => {
            for &(_, wy) in waypoints {
                take(wy);
            }
            // The first bend row can sit below the source row block.
            let dir: isize = if to_y >= from_y { 1 } else { -1 };
            let first_bend = (from_y as isize + dir * (1 + start_offset as isize)) as usize;
            take(first_bend);
        }
    }
    (lo, hi)
}

/// Bytes of arena [`RenderPlan::build_in`] plus the compositor's carve
/// calls need for this view and options — plan storage, paint scratch,
/// and the band canvas planes, with alignment slack.
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
        run_capacity += count_h_runs(&ed.path, ed.from_x, ed.to_x, ed.flow_axis);
    }
    let bands = 2usize
        .saturating_mul(height.div_ceil(cap))
        .saturating_add(2);
    let band_rows = cap.min(height).max(1);
    let area = width.saturating_mul(band_rows);

    let plan_bytes = e * size_of::<EdgePlan>()
        + s * size_of::<SubgraphPlan>()
        + (n + e + s) * size_of::<PlanElement>()
        + labeled
            * (size_of::<LabelPlan>() + size_of::<usize>() + size_of::<(usize, usize, usize)>())
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
    // Per-allocation alignment slack (≤ 8 bytes × ~18 carves) + margin.
    plan_bytes + scratch_bytes + canvas_bytes + 18 * 8 + 64
}

/// Structural count of horizontal-run interiors a path paints — the
/// same segments `h_run_with_corners` collects, counted without cells.
fn count_h_runs(
    path: &PathRef<'_>,
    from_x: usize,
    to_x: usize,
    axis: crate::ir::FlowAxis,
) -> usize {
    // The X paint path strokes per-cell and never defers runs.
    if matches!(axis, crate::ir::FlowAxis::X) {
        return 0;
    }
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
//
// Deliberate bound: placement checks every label span against every
// blocker — O(labels × (edges + subgraphs)) worst case (quadratic when
// most edges are labeled). Fine at realistic label counts; if profiling
// ever shows otherwise, bucket blockers by row before this pass.

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

    // X flows: label rows are node rows — node areas block (Y label
    // rows sit below their level by construction, so this cannot fire
    // there and the Y path stays frozen).
    if matches!(view.edge(label_edge).flow_axis, crate::ir::FlowAxis::X) {
        for ni in 0..view.node_count() {
            let n = view.node(ni);
            if row >= n.y && row < n.y + n.height && n.x < x1 && n.x + n.width > x0 {
                return true;
            }
        }
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
        match e.flow_axis {
            crate::ir::FlowAxis::Y => {
                if row < e.from_y.min(e.to_y) || row > e.from_y.max(e.to_y) {
                    continue;
                }
                // Horizontal runs (with their corner endpoints) block.
                let mut run_blocked = false;
                for_each_h_run(
                    &e.path,
                    e.from_x,
                    e.from_y,
                    e.to_x,
                    e.to_y,
                    row,
                    e.flow_axis,
                    &mut |r0, r1| run_blocked |= r0 < x1 && r1 + 1 > x0,
                );
                if run_blocked {
                    return true;
                }
                // Dashed verticals block ('┊' is not '│'); solid verticals are
                // allowed — including other edges' (legacy checks only the char).
                if i != label_edge && e.reversed {
                    let mut col_blocked = false;
                    for_each_v_col(
                        &e.path,
                        e.from_x,
                        e.from_y,
                        e.to_x,
                        e.to_y,
                        row,
                        e.flow_axis,
                        &mut |c| col_blocked |= c >= x0 && c < x1,
                    );
                    if col_blocked {
                        return true;
                    }
                }
            }
            crate::ir::FlowAxis::X => {
                // The role mirror of the Y rule. Other edges' cross
                // ink (the vertical bend runs) blocks; their flow
                // trunks are replaceable unless dashed. The label
                // edge's OWN ink — cross and trunk alike — never
                // blocks its own label: that is what lets D9's
                // vertical host sit on the line it annotates, exactly
                // as a Y label sits on its own flow segment.
                let (lo, hi) = edge_row_span(&e.path, e.from_y, e.to_y, e.flow_axis);
                if row < lo || row > hi {
                    continue;
                }
                // The label edge's OWN cross ink never blocks its own
                // label — the exact mirror of the Y rule, where a
                // label may replace its own flow cells. This is what
                // makes D9's vertical host reachable.
                if i != label_edge {
                    let mut col_blocked = false;
                    for_each_v_col(
                        &e.path,
                        e.from_x,
                        e.from_y,
                        e.to_x,
                        e.to_y,
                        row,
                        e.flow_axis,
                        &mut |c| col_blocked |= c >= x0 && c < x1,
                    );
                    if col_blocked {
                        return true;
                    }
                }
                if i != label_edge && e.reversed {
                    let mut run_blocked = false;
                    for_each_h_run(
                        &e.path,
                        e.from_x,
                        e.from_y,
                        e.to_x,
                        e.to_y,
                        row,
                        e.flow_axis,
                        &mut |r0, r1| run_blocked |= r0 < x1 && r1 + 1 > x0,
                    );
                    if run_blocked {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Visit every horizontal run `[x0, x1]` (inclusive) painted by this
/// path at `row` — the same formulas the painter uses. Visitor-based so
/// arbitrarily long multi-segment paths lose nothing to fixed caps.
fn for_each_h_run(
    path: &PathRef<'_>,
    from_x: usize,
    from_y: usize,
    to_x: usize,
    to_y: usize,
    row: usize,
    axis: crate::ir::FlowAxis,
    f: &mut dyn FnMut(usize, usize),
) {
    let mut push = |x0: usize, x1: usize| f(x0.min(x1), x0.max(x1));
    // X flows: horizontal ink is the TRUNK segments (node faces
    // trimmed, arrow cells included, bend corners included) — the
    // exact mirror of the compositor's `paint_edge_x`.
    if matches!(axis, crate::ir::FlowAxis::X) {
        let step: isize = if to_x >= from_x { 1 } else { -1 };
        let trim = |x: usize| (x as isize + step) as usize;
        let back = |x: usize| (x as isize - step) as usize;
        match *path {
            PathRef::Direct | PathRef::Spline { .. } => {
                if row == from_y && from_x.abs_diff(to_x) > 1 {
                    push(trim(from_x), back(to_x));
                }
            }
            PathRef::Corner { bend_at: bend_x } => {
                if row == from_y {
                    push(trim(from_x), bend_x);
                }
                if row == to_y {
                    push(bend_x, back(to_x));
                }
            }
            PathRef::SideChannel {
                channel_at,
                span_start,
                span_end,
            } => {
                // Three flow segments: source → channel entry, along
                // the far channel row, channel exit → target.
                if row == from_y {
                    push(trim(from_x), span_start);
                }
                if row == channel_at {
                    push(span_start, span_end);
                }
                if row == to_y {
                    push(span_end, back(to_x));
                }
            }
            PathRef::MultiSegment {
                waypoints,
                start_offset,
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
                    let last = i == waypoints.len();
                    if py == ny {
                        if row == py && px != nx {
                            let a = if first { trim(px) } else { px };
                            let b = if last { back(nx) } else { nx };
                            push(a, b);
                        }
                    } else {
                        let bend_x = (px as isize
                            + step * (1 + if first { start_offset as isize } else { 0 }))
                            as usize;
                        if row == py {
                            let a = if first && start_offset > 0 {
                                trim(px)
                            } else {
                                px
                            };
                            push(a, bend_x);
                        }
                        if row == ny {
                            let b = if last { back(nx) } else { nx };
                            push(bend_x, b);
                        }
                    }
                    px = nx;
                    py = ny;
                    first = false;
                }
            }
        }
        return;
    }
    match *path {
        PathRef::Direct | PathRef::Spline { .. } => {}
        PathRef::Corner { bend_at } => {
            if row == bend_at {
                push(from_x, to_x);
            }
        }
        PathRef::SideChannel {
            channel_at,
            span_start,
            span_end,
        } => {
            if row == span_start {
                push(from_x, channel_at);
            }
            if row == span_end {
                push(to_x, channel_at);
            }
        }
        PathRef::MultiSegment {
            waypoints,
            start_offset,
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
                    let step = 1 + if first { start_offset as isize } else { 0 };
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
}

/// How many candidate rows D9's cross host tries per segment, walking
/// outward from the SOURCE end.
///
/// Bounds the placement search at `O(labels × blockers)`: without it a
/// tall jog multiplies every label's blocker scan by the segment
/// height, and a tall congested LR graph pays millions of full
/// geometry scans at plan time. Not a silent truncation — rows beyond
/// the cap fall through to the remaining hosts (seed → float →
/// legend), and a label far down a long jog reads as belonging to
/// nothing anyway. The TD analog sits a fixed few rows below its
/// source for the same reason.
const CROSS_HOST_CANDIDATES: usize = 4;

/// Visit every CROSS (vertical) segment an X-flow path paints, as
/// `(column, row_from, row_to)` — the segment's own endpoints in FLOW
/// order (not sorted), so callers can both test interiors
/// (`row_from < row < row_to`, either orientation) and walk candidate
/// rows outward from the source end.
///
/// The single authority for X cross geometry: [`for_each_v_col`]'s X
/// arm delegates here, and D9's vertical label host walks it.
fn for_each_x_cross_segment(
    path: &PathRef<'_>,
    from_x: usize,
    from_y: usize,
    to_x: usize,
    to_y: usize,
    f: &mut dyn FnMut(usize, usize, usize),
) {
    match *path {
        PathRef::Direct | PathRef::Spline { .. } => {}
        PathRef::Corner { bend_at } => f(bend_at, from_y, to_y),
        PathRef::SideChannel {
            channel_at,
            span_start,
            span_end,
        } => {
            f(span_start, from_y, channel_at);
            f(span_end, channel_at, to_y);
        }
        PathRef::MultiSegment {
            waypoints,
            start_offset,
        } => {
            let step: isize = if to_x >= from_x { 1 } else { -1 };
            let mut px = from_x;
            let mut py = from_y;
            let mut first = true;
            for i in 0..=waypoints.len() {
                let (nx, ny) = if i < waypoints.len() {
                    waypoints[i]
                } else {
                    (to_x, to_y)
                };
                if py != ny {
                    let bend_x = (px as isize
                        + step * (1 + if first { start_offset as isize } else { 0 }))
                        as usize;
                    f(bend_x, py, ny);
                }
                px = nx;
                py = ny;
                first = false;
            }
        }
    }
}

/// Visit every vertical column this path paints at `row` (between, not
/// touching, the endpoints' node rows). Visitor-based — no fixed cap.
fn for_each_v_col(
    path: &PathRef<'_>,
    from_x: usize,
    from_y: usize,
    to_x: usize,
    to_y: usize,
    row: usize,
    axis: crate::ir::FlowAxis,
    f: &mut dyn FnMut(usize),
) {
    // X flows: vertical ink is the CROSS runs, strictly between the
    // trunk rows (corner cells belong to the trunk rows' horizontal
    // ink). One authority for that geometry — delegate.
    if matches!(axis, crate::ir::FlowAxis::X) {
        for_each_x_cross_segment(path, from_x, from_y, to_x, to_y, &mut |col, a, b| {
            if row > a.min(b) && row < a.max(b) {
                f(col);
            }
        });
        return;
    }
    let mut push = |c: usize| f(c);
    // Order-free strictly-between test (works for either flow).
    let betw = |a: usize, b: usize, r: usize| r > a.min(b) && r < a.max(b);
    match *path {
        PathRef::Direct | PathRef::Spline { .. } => {
            if betw(from_y, to_y, row) {
                push(from_x);
            }
        }
        PathRef::Corner { bend_at } => {
            if betw(from_y, bend_at, row) {
                push(from_x);
            }
            if betw(bend_at, to_y, row) {
                push(to_x);
            }
        }
        PathRef::SideChannel {
            channel_at,
            span_start,
            span_end,
        } => {
            if betw(span_start, span_end, row) {
                push(channel_at);
            }
            if betw(span_end, to_y, row) {
                push(to_x);
            }
        }
        PathRef::MultiSegment {
            waypoints,
            start_offset,
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
                // Non-first segments carry a vertical on the waypoint
                // row itself (the painter's gap fill).
                if !first && row == py {
                    push(px);
                }
                if px == nx {
                    if betw(py, ny, row) {
                        push(px);
                    }
                } else if py != ny {
                    let step = 1 + if first { start_offset as isize } else { 0 };
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

    fn legend_line_count(g: &Graph<'_>) -> usize {
        let out = crate::render::engine::render_colored(
            &g.compute_layout(),
            &RenderOptions::colored(Palette::Ansi),
        );
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
            legend_line_count(&g),
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
        let legacy = legend_line_count(&g);
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
        assert!(plan.elements().windows(2).all(|w| w[0].y_min <= w[1].y_min));
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
