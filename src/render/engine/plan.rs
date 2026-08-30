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
use super::config::{PlanOptions, RenderOptions};
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

    /// Whether plan storage is heap-backed. The ink index engages only
    /// then: on a caller-provided arena every byte is provisioned
    /// against a published estimate, so the index would be an
    /// unbounded-looking cost there — Scan serves identical answers
    /// (the oracle pins that) for zero arena bytes.
    fn is_heap(&self) -> bool {
        match self {
            #[cfg(feature = "alloc")]
            PlanMem::Heap => true,
            PlanMem::Arena(_) => false,
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

/// Result of a hit-test query ([`Scene::hit_test`](super::scene::Scene::hit_test)).
///
/// ```
/// use ascii_dag::{Graph, RenderOptions, ScenePlanner};
/// use ascii_dag::render::engine::HitResult;
///
/// let g = Graph::from_edges(&[(1, "Alpha"), (2, "Beta")], &[(1, 2)]);
/// let ir = g.compute_layout();
/// let options = RenderOptions::plain();
/// let mut planner = ScenePlanner::new();
/// let scene = planner.plan(&ir, &options.plan).unwrap();
///
/// // Find where "Alpha" was painted, then ask what is there.
/// let text = ir.render_string(&options);
/// let (row, col) = text
///     .lines()
///     .enumerate()
///     .find_map(|(r, l)| l.find("Alpha").map(|c| (r, c)))
///     .unwrap();
/// assert_eq!(scene.hit_test(col, row), HitResult::Node(1));
///
/// // Off the canvas is `None`, never a panic.
/// assert_eq!(scene.hit_test(9999, 9999), HitResult::None);
/// ```
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
    /// Placement under the node-row-avoiding policy's stricter
    /// semantics (the legacy colored-with-legend path).
    pub(crate) fn placed_colored(&self) -> bool {
        self.placeable && !self.row_has_node
    }

    /// Does this label PAINT under the given placement policy? Every
    /// consumer must go through this one predicate: a divergent copy
    /// once made renders warn about labels they actually painted.
    pub(crate) fn paints_under(&self, placement: super::config::LabelPlacementPolicy) -> bool {
        // Exhaustive on purpose (in-crate matches on the
        // `#[non_exhaustive]` enum still are): a future policy must
        // decide its veto behavior here, never inherit one silently.
        match placement {
            super::config::LabelPlacementPolicy::Geometric => self.placeable,
            super::config::LabelPlacementPolicy::AvoidNodeRows => self.placed_colored(),
        }
    }
}

impl EdgePlan {
    /// Which geometric side paints a marker, `(from, to)` — reversal
    /// swaps the logical ends: `marker_end` is the arrowhead legacy
    /// always paints, `marker_start` the (legacy-off) tail. The single
    /// authority shared by the compositor's paint arms and label
    /// placement, so placement never trims a host run for — or blocks
    /// a window under — a marker cell that will not be painted.
    pub(crate) fn resolved_markers(&self, reversed: bool) -> (bool, bool) {
        let from = if reversed {
            self.marker_end
        } else {
            self.marker_start
        };
        let to = if reversed {
            self.marker_start
        } else {
            self.marker_end
        };
        (
            !matches!(from, MarkerShape::None),
            !matches!(to, MarkerShape::None),
        )
    }
}

/// The render plan. Public read-only queries; internals private.
/// Storage is heap- or arena-backed behind `PlanBuf` — one build
/// path serves std and no-alloc callers alike.
///
/// A plan is a snapshot of one layout resolved under one
/// [`PlanOptions`]; the render entry points build their own plan
/// internally, and [`Scene`](super::scene::Scene) is the public
/// carrier (plan + layout view bound together, so queries can never
/// be paired with the wrong layout). Out-of-canvas queries return
/// `HitResult::None`.
pub struct RenderPlan<'buf> {
    width: usize,
    height: usize,
    /// Sorted, deduped node top rows — the geometry the band
    /// partition aligns to. Budget-independent plan state; the
    /// partition itself is computed per composition from
    /// [`ComposeBudget`](super::config::ComposeBudget) via
    /// [`bands`](Self::bands).
    level_tops: PlanBuf<'buf, usize>,
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
    label_placement: super::config::LabelPlacementPolicy,
}

impl<'buf> RenderPlan<'buf> {
    /// Build a heap-backed plan (std/alloc convenience). Heap pushes
    /// cannot fail, so this surface stays infallible.
    #[cfg(feature = "alloc")]
    pub(crate) fn build<V: LayoutView>(view: &V, options: &PlanOptions) -> RenderPlan<'static> {
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
        options: &PlanOptions,
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
        options: &PlanOptions,
        mem: PlanMem<'_, 'buf>,
    ) -> Result<RenderPlan<'buf>, GraphError> {
        let width = view.width();
        let height = view.height();
        let oom = || GraphError::RenderPlanOom;

        // ── Resolved styles (the only place style fns run — Q4) ────────
        let palette = options.palette.colors();
        let label_policy = options.label_policy;
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
            } else if !palette.is_empty() {
                // Colors are ALWAYS resolved at plan time — plain
                // emission ignores them, which is what lets one plan
                // serve colored and plain output alike.
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
        // ── Edge-ink source (temp/11 §3.99/§3.101): on HEAP-backed
        // plans past the work threshold, every painted run is
        // collected once — via the same row-agnostic visitors the
        // per-row wrappers filter — so each blocker query below
        // touches a row slice and a column range instead of re-walking
        // every edge's path per candidate. Caller-arena plans always
        // Scan: identical answers (pinned by the oracle test), zero
        // arena bytes — `estimate_render_arena_size` carries no index
        // term at all.
        let mut ink_h: PlanBuf<'buf, (usize, usize, usize, usize)>;
        let mut ink_v: PlanBuf<'buf, (usize, usize, usize, usize)>;
        let ink =
            if mem.is_heap() && labeled.saturating_mul(view.edge_count()) >= LABEL_INDEX_MIN_WORK {
                let mut ink_h_count = 0usize;
                let mut ink_v_count = 0usize;
                for i in 0..view.edge_count() {
                    let e = view.edge(i);
                    for_each_h_run_all(
                        &e.path,
                        e.from_x,
                        e.from_y,
                        e.to_x,
                        e.to_y,
                        e.flow_axis,
                        &mut |_, _, _| ink_h_count += 1,
                    );
                    for_each_v_seg_all(
                        &e.path,
                        e.from_x,
                        e.from_y,
                        e.to_x,
                        e.to_y,
                        e.flow_axis,
                        &mut |_, _, _| ink_v_count += 1,
                    );
                }
                ink_h = mem.buf(ink_h_count, oom())?;
                ink_v = mem.buf(ink_v_count, oom())?;
                for i in 0..view.edge_count() {
                    let e = view.edge(i);
                    for_each_h_run_all(
                        &e.path,
                        e.from_x,
                        e.from_y,
                        e.to_x,
                        e.to_y,
                        e.flow_axis,
                        &mut |r, a, b| ink_h.push((r, a, b, i)),
                    );
                    for_each_v_seg_all(
                        &e.path,
                        e.from_x,
                        e.from_y,
                        e.to_x,
                        e.to_y,
                        e.flow_axis,
                        &mut |c, lo, hi| ink_v.push((c, lo, hi, i)),
                    );
                }
                ink_h.as_mut_slice().sort_unstable();
                ink_v.as_mut_slice().sort_unstable();
                InkSource::Indexed(InkIndex {
                    h: ink_h.as_slice(),
                    v: ink_v.as_slice(),
                })
            } else {
                InkSource::Scan
            };

        let mut claimed: PlanBuf<'buf, (usize, usize, usize)> = mem.buf(labeled, oom())?;
        let mut slide_work = 0usize;

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
            // Style-resolved marker presence (edge plans are complete
            // before any label is placed). A phantom flag here would
            // trim host runs and block windows for cells the painter
            // leaves as plain stroke — with the default styles the
            // from-marker does not exist at all.
            let (from_m, to_m) = edge_plans.as_slice()[i].resolved_markers(e.reversed);

            // Resolved box-label row for the strict blocker: the row the
            // box label actually paints on, per its planned position.
            let sg_label_row =
                |si: usize, sg: &crate::render::engine::view::SubgraphRef<'_>| -> usize {
                    match subgraph_plans.as_slice()[si].label_pos {
                        super::style::LabelPosition::InsideTop => sg.y + 1,
                        super::style::LabelPosition::InsideBottom => {
                            (sg.y + sg.height).saturating_sub(2)
                        }
                    }
                };

            // ── Host 0 — long-run midpoint (temp/11 rule 2, D9
            // amendment ruled 2026-08-07). A very long edge reads best
            // labeled mid-run — the classic wire-label look — not at
            // its source or crowded around a bend. Applies only when
            // the longest own flow run dwarfs the label
            // (LONG_RUN_LABEL_FACTOR), so short edges keep the legacy
            // near-source placement byte-for-byte. Midpoint rounding
            // follows the flow, the same trick Host 3 uses, so LR↔RL
            // and TD↔BT stay exact mirrors. Vetted by the strict
            // blocker + rule 1: a new-position host earns no legacy
            // exemptions.
            {
                let (from_f, to_f) = if is_x {
                    (e.from_x, e.to_x)
                } else {
                    (e.from_y, e.to_y)
                };
                let forward = to_f >= from_f;
                let mut longest: Option<(usize, usize, usize, usize)> = None;
                for_each_flow_host_segment(
                    &e.path,
                    e.from_x,
                    e.from_y,
                    e.to_x,
                    e.to_y,
                    e.flow_axis,
                    from_m,
                    to_m,
                    &mut |line, lo, hi| {
                        let rlen = hi - lo + 1;
                        if longest.is_none_or(|(bl, _, _, _)| rlen > bl) {
                            longest = Some((rlen, line, lo, hi));
                        }
                    },
                );
                if let Some((rlen, line, lo, hi)) = longest {
                    if rlen >= LONG_RUN_LABEL_FACTOR * len {
                        let d = hi - lo;
                        let m = if forward {
                            lo + d / 2
                        } else {
                            lo + d.div_ceil(2)
                        };
                        let (cx, cy) = if is_x {
                            (m.checked_sub(lead), line)
                        } else {
                            (line.checked_sub(lead), m)
                        };
                        if let Some(cx) = cx {
                            if cx + len <= width
                                && cy < height
                                && !own_fixed_cell_in_span(view, i, from_m, to_m, cy, cx, cx + len)
                                && !slide_blocked(
                                    view,
                                    &ink,
                                    i,
                                    cy,
                                    cx,
                                    cx + len,
                                    claimed.as_slice(),
                                    options.show_dummy_nodes,
                                    &sg_label_row,
                                )
                            {
                                x = cx;
                                y = cy;
                                placeable = true;
                            }
                        }
                    }
                }
            }

            // ── D9 host ladder ──
            // Host 1 — the edge's OWN cross (vertical) segment: the
            // direct mirror of the TD picture, where the label
            // interrupts the line it annotates and spreads sideways
            // over empty cells. X-only: a Y flow's seed already lands
            // on its own flow segment, so Y starts at host 2 and its
            // output stays byte-frozen.
            if is_x && !placeable {
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
                                && !own_fixed_cell_in_span(view, i, from_m, to_m, row, cx, cx + len)
                                && !span_blocked(
                                    view,
                                    &ink,
                                    i,
                                    row,
                                    cx,
                                    cx + len,
                                    claimed.as_slice(),
                                    options.show_dummy_nodes,
                                )
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
                    && y < height
                    && !own_fixed_cell_in_span(view, i, from_m, to_m, y, x, x + len)
                    && !span_blocked(
                        view,
                        &ink,
                        i,
                        y,
                        x,
                        x + len,
                        claimed.as_slice(),
                        options.show_dummy_nodes,
                    );
            }

            // Host 2b — slide along the edge's own flow runs
            // (temp/11 §3): when the seed fails, it anchors a bounded
            // search instead of deciding alone. Candidates come from
            // run boundaries and the seed clamped into each run,
            // ranked by (|anchor − seed|, segment order, then toward
            // the flow — the mirror-stable tie the ladder already
            // uses), and vetted by the STRICT blocker: sliding
            // abandons the by-construction guarantees the legacy
            // positions carry, so it may not rely on their relaxed
            // check. Marker presence is style-resolved (`from_m`/
            // `to_m` above), so runs are trimmed for exactly the
            // marker cells the painter will draw.
            if !placeable {
                let (from_f, to_f) = if is_x {
                    (e.from_x, e.to_x)
                } else {
                    (e.from_y, e.to_y)
                };
                let seed_flow = if is_x { e.label_x + lead } else { e.label_y };
                let toward_flow =
                    |a: usize| -> usize { if to_f >= from_f { a } else { usize::MAX - a } };
                // Candidates carry their FINAL window coordinates.
                // Key: (dist-to-seed, host class flow=0/cross=1,
                // segment, lateral rank center<edges, toward-flow
                // tie) — total order, mirror-stable.
                let fwd = to_f >= from_f;
                let mut cands = [(0usize, 0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
                    LABEL_SLIDE_CANDIDATES];
                let mut n_c = 0usize;
                // The window must COVER the edge's own ink cell it
                // annotates; three lateral offsets place that cell at
                // the window's center (the legacy look), its right
                // edge, or its left edge — the lateral freedom Host 1
                // never had, which is what reaches box interiors.
                let offsets = [lead, len.saturating_sub(1), 0usize];
                {
                    let mut push_window =
                        |key: (usize, usize, usize, usize), own_x: usize, own_y: usize| {
                            for (op, &off) in offsets.iter().enumerate() {
                                let Some(cx) = own_x.checked_sub(off) else {
                                    continue;
                                };
                                if cx + len > width || own_y >= height {
                                    continue;
                                }
                                if cands[..n_c].iter().any(|c| c.5 == cx && c.6 == own_y) {
                                    continue;
                                }
                                let cand = (
                                    key.0,
                                    key.1,
                                    key.2,
                                    lateral_rank(op, is_x, fwd),
                                    key.3,
                                    cx,
                                    own_y,
                                );
                                if n_c < LABEL_SLIDE_CANDIDATES {
                                    cands[n_c] = cand;
                                    n_c += 1;
                                } else {
                                    // Budget full: keep the globally
                                    // best K by replacing the current
                                    // worst, so one early, prolific
                                    // segment cannot crowd closer
                                    // candidates from later segments
                                    // out before the sort runs. The
                                    // blocker-scan budget still caps
                                    // at K vetted windows.
                                    let mut wi = 0usize;
                                    for j in 1..LABEL_SLIDE_CANDIDATES {
                                        if cands[j] > cands[wi] {
                                            wi = j;
                                        }
                                    }
                                    if cand < cands[wi] {
                                        cands[wi] = cand;
                                    }
                                }
                            }
                        };

                    let mut seg = 0usize;
                    for_each_flow_host_segment(
                        &e.path,
                        e.from_x,
                        e.from_y,
                        e.to_x,
                        e.to_y,
                        e.flow_axis,
                        from_m,
                        to_m,
                        &mut |line, lo, hi| {
                            let sc = seed_flow.clamp(lo, hi);
                            for a in [
                                sc,
                                sc.saturating_add(1).min(hi),
                                sc.saturating_sub(1).max(lo),
                                sc.saturating_add(2).min(hi),
                                sc.saturating_sub(2).max(lo),
                                lo,
                                hi,
                            ] {
                                let (ox, oy) = if is_x { (a, line) } else { (line, a) };
                                push_window(
                                    (a.abs_diff(seed_flow), 0, seg, toward_flow(a)),
                                    ox,
                                    oy,
                                );
                            }
                            seg += 1;
                        },
                    );
                    // Cross segments too (X flows): the riser's interior
                    // rows, laterally slid — box interiors live here.
                    if is_x {
                        for_each_x_cross_segment(
                            &e.path,
                            e.from_x,
                            e.from_y,
                            e.to_x,
                            e.to_y,
                            &mut |col, seg_from, seg_to| {
                                let (rlo, rhi) = (
                                    seg_from.min(seg_to) + 1,
                                    seg_from.max(seg_to).saturating_sub(1),
                                );
                                if rlo > rhi {
                                    return;
                                }
                                let seed_row = e.label_y.clamp(rlo, rhi);
                                for r in [
                                    seed_row,
                                    seed_row.saturating_add(1).min(rhi),
                                    seed_row.saturating_sub(1).max(rlo),
                                    seed_row.saturating_add(2).min(rhi),
                                    seed_row.saturating_sub(2).max(rlo),
                                    rlo,
                                    rhi,
                                ] {
                                    push_window(
                                        (
                                            r.abs_diff(e.label_y),
                                            1,
                                            seg,
                                            cross_row_rank(r, seg_from, seg_to),
                                        ),
                                        col,
                                        r,
                                    );
                                }
                                seg += 1;
                            },
                        );
                    }
                }
                cands[..n_c].sort_unstable();
                for &(_, _, _, _, _, cx, cy) in cands[..n_c].iter() {
                    // Slide-tier work purse (the lane pass's pattern).
                    // The charge is the SCAN-mode cost in both ink
                    // modes — see LABEL_SLIDE_WORK_BUDGET for why a
                    // mode-faithful charge would be wrong. Beyond the
                    // budget the tier reports "no host" and the ladder
                    // continues to Host 3/legend — deterministic, and
                    // identical across mirrors (candidate counts are
                    // symmetric) and backends (pure counting).
                    if slide_work >= LABEL_SLIDE_WORK_BUDGET {
                        break;
                    }
                    slide_work += view.edge_count().max(1);
                    if own_fixed_cell_in_span(view, i, from_m, to_m, cy, cx, cx + len) {
                        continue;
                    }
                    if !slide_blocked(
                        view,
                        &ink,
                        i,
                        cy,
                        cx,
                        cx + len,
                        claimed.as_slice(),
                        options.show_dummy_nodes,
                        &sg_label_row,
                    ) {
                        x = cx;
                        y = cy;
                        placeable = true;
                        break;
                    }
                }
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
                    && !own_fixed_cell_in_span(view, i, from_m, to_m, fy, fx, fx + len)
                    && !span_blocked(
                        view,
                        &ink,
                        i,
                        fy,
                        fx,
                        fx + len,
                        claimed.as_slice(),
                        options.show_dummy_nodes,
                    )
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
            // The legend lists exactly the labels that will NOT paint
            // under the compositor's own gate (`LabelPlan::paints`) —
            // `legend_entries` reflects the options this plan was
            // built with.
            let painted = plan.paints_under(label_policy.placement);
            if !painted {
                // Exhaustive on purpose: every overflow variant —
                // future ones included — must decide what happens to
                // an unplaced label right here; a silently inherited
                // behavior would either lose the label or lose the
                // diagnostic.
                match label_policy.overflow {
                    super::config::LabelOverflow::Legend => legend.push(i),
                    // Omitted entirely → the label appears NOWHERE.
                    // Never silent under the `warnings` feature
                    // (emitted per plan build — plans are stateless).
                    // The label TEXT is deliberately not printed:
                    // labels are caller data and may hold secrets or
                    // control characters (terminal/log injection) —
                    // the edge is identified by index and endpoint
                    // ids instead.
                    super::config::LabelOverflow::Omit => {
                        #[cfg(feature = "warnings")]
                        crate::errors::emit_warning(
                            crate::errors::WARN_LABEL_INVISIBLE,
                            format_args!(
                                "the label of edge {} ({} -> {}) has no inline position and \
                                 overflow is set to omit - it will not be rendered. Set \
                                 LabelOverflow::Legend to list it below the graph.",
                                i, e.from_id, e.to_id
                            ),
                        );
                    }
                }
            }
            labels.push(plan);
        }

        // ── Level tops (band-partition geometry) ───────────────────────
        // The plan stores only the sorted, deduped node top rows; the
        // level-aligned partition itself is a COMPOSITION decision,
        // computed per render from the caller's `ComposeBudget` cap
        // (see [`Bands`]). Plan identity stays free of workspace-shaped
        // state — the same plan serves every band budget.
        let mut level_tops: PlanBuf<'buf, usize> = mem.buf(view.node_count(), oom())?;
        for i in 0..view.node_count() {
            level_tops.push(view.node(i).y);
        }
        level_tops.as_mut_slice().sort_unstable();
        let mut last = None;
        level_tops.retain(|&t| {
            let keep = last != Some(t);
            last = Some(t);
            keep
        });

        Ok(RenderPlan {
            width,
            height,
            level_tops,
            edge_plans,
            subgraph_plans,
            labels,
            index,
            legend,
            run_capacity,
            show_dummy_nodes: options.show_dummy_nodes,
            label_placement: label_policy.placement,
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

    /// Edge indices (IR-list order) whose labels go to the legend under
    /// the options this plan was built with: the labels that did not
    /// paint under the active placement policy (`AvoidNodeRows`: the
    /// row-veto rule; `Geometric`: pure geometric placement). Empty
    /// unless `LabelOverflow::Legend` was set — matching what actually
    /// renders.
    pub fn legend_entries(&self) -> &[usize] {
        self.legend.as_slice()
    }

    /// The label placement policy this plan resolved under (plan
    /// state — the compositor and hit-testing read it from here, never
    /// from options).
    pub(crate) fn label_placement(&self) -> super::config::LabelPlacementPolicy {
        self.label_placement
    }

    /// Whether this plan shows dummy nodes (plan state, same rule as
    /// [`label_placement`](Self::label_placement): planning options
    /// are read back from the plan, never re-read from options — two
    /// sources of truth would let a scene/composer pair disagree).
    pub(crate) fn show_dummy_nodes(&self) -> bool {
        self.show_dummy_nodes
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
        // edge ink). The placement policy is plan state, so this is
        // exactly the compositor's paint predicate.
        for label in self.labels.as_slice() {
            let placed = label.paints_under(self.label_placement);
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

    /// The level-aligned band partition under `cap`, as ascending
    /// `(first_row, rows)` pairs tiling `0..height`. Pure computation
    /// over stored geometry — banding is a composition-budget choice,
    /// never plan state, and band boundaries are unobservable in the
    /// output (canvas clipping replays boundary-spanning elements).
    pub(crate) fn bands(&self, cap: usize) -> Bands<'_> {
        Bands {
            height: self.height,
            cap: cap.max(1),
            tops: self.level_tops.as_slice(),
            start: 0,
        }
    }

    /// Rows of the tallest band under `cap` — the band buffer's height.
    pub(crate) fn max_band_rows(&self, cap: usize) -> usize {
        self.bands(cap).map(|(_, rows)| rows).max().unwrap_or(0)
    }

    /// Exact h-run interior count — sizes the compositor's run scratch.
    pub(crate) fn run_capacity(&self) -> usize {
        self.run_capacity
    }
}

/// The level-aligned band partition (Q1: level-aligned, capped), as
/// an iterator of `(y0, rows)` pairs. Boundaries prefer level tops
/// (distinct node rows) so bands don't split levels; a level chunk
/// taller than the cap is hard-cut at the cap. Elements spanning a
/// boundary are simply replayed in every band they intersect — canvas
/// clipping makes out-of-band writes no-ops, which is what makes the
/// partition (a memory decision) unobservable in the output.
pub(crate) struct Bands<'a> {
    height: usize,
    cap: usize,
    tops: &'a [usize],
    start: usize,
}

impl Iterator for Bands<'_> {
    type Item = (usize, usize);

    fn next(&mut self) -> Option<(usize, usize)> {
        if self.start >= self.height {
            return None;
        }
        let cap_end = self.start + self.cap;
        if cap_end >= self.height {
            let band = (self.start, self.height - self.start);
            self.start = self.height;
            return Some(band);
        }
        let ub = self.tops.partition_point(|&t| t <= cap_end);
        let boundary = self.tops[..ub]
            .iter()
            .rev()
            .find(|&&t| t > self.start)
            .copied()
            .unwrap_or(cap_end);
        let band = (self.start, boundary - self.start);
        self.start = boundary;
        Some(band)
    }
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
/// and the band canvas planes, with alignment slack. The plan stores
/// level tops (one `usize` per node, before dedup) instead of a band
/// list — the partition is computed per composition.
pub(crate) fn estimate_plan_bytes<V: LayoutView>(view: &V, options: &RenderOptions) -> usize {
    use core::mem::size_of;
    let n = view.node_count();
    let e = view.edge_count();
    let s = view.subgraph_count();
    let width = view.width();
    let height = view.height();
    let cap = options.compose.cap();
    let colored = !matches!(options.emit.color_mode, super::color::ColorMode::None);
    let mut run_capacity = 0usize;
    for i in 0..e {
        let ed = view.edge(i);
        run_capacity += count_h_runs(&ed.path, ed.from_x, ed.to_x, ed.flow_axis);
    }
    // Deliberately NO ink-index term: this estimator serves the
    // caller-arena surface, where the index never engages
    // (`PlanMem::is_heap` gates it) — arena estimates are
    // byte-identical to pre-index releases.
    let band_rows = cap.min(height).max(1);
    let area = width.saturating_mul(band_rows);

    let plan_bytes = plan_storage_bytes(view);
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
    // Alignment slack for the compositor's carves (the plan term
    // carries its own).
    plan_bytes + scratch_bytes + canvas_bytes + 10 * 8 + 64
}

/// Bytes of storage one [`RenderPlan::build_in`] needs for this view —
/// the plan's OWN buffers only (no compositing scratch, no canvas).
/// Sizes a [`ScenePlanner`](super::scene::ScenePlanner) workspace.
pub(crate) fn plan_storage_bytes<V: LayoutView>(view: &V) -> usize {
    use core::mem::size_of;
    let n = view.node_count();
    let e = view.edge_count();
    let s = view.subgraph_count();
    let labeled = (0..e).filter(|&i| view.edge(i).label.is_some()).count();
    e * size_of::<EdgePlan>()
        + s * size_of::<SubgraphPlan>()
        + (n + e + s) * size_of::<PlanElement>()
        + labeled
            * (size_of::<LabelPlan>() + size_of::<usize>() + size_of::<(usize, usize, usize)>())
        + n * size_of::<usize>() // level tops
        // Per-carve alignment slack (≤ 8 bytes × ~8 plan carves).
        + 8 * 8
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
// Blocker queries are served by an [`InkSource`]: small plans walk the
// geometry visitors per query; past [`LABEL_INDEX_MIN_WORK`] the same
// geometry is collected once into a sorted per-plan index with
// identical answers — pinned by the indexed-vs-scanned oracle test in
// the parity suite.

/// Every painted edge-ink run, collected once per plan build via the
/// same row-agnostic visitors the per-row wrappers filter — so each
/// blocker query touches its row slice and column range instead of
/// re-walking every edge's path (the temp/11 §3.98 deferred structural
/// fix). `h` is sorted by `(row, x0)`, `v` by column; owners tag every
/// run so the per-rule own-ink exemptions still resolve. Answers are
/// cell-identical to the visitor walks by construction.
pub(super) struct InkIndex<'a> {
    /// `(row, x0, x1)` inclusive horizontal runs + owner edge.
    pub(super) h: &'a [(usize, usize, usize, usize)],
    /// `(col, row_lo, row_hi)` inclusive interior vertical rows + owner.
    pub(super) v: &'a [(usize, usize, usize, usize)],
}

impl<'a> InkIndex<'a> {
    /// All horizontal runs on `row` (sorted by x0).
    fn h_at_row(&self, row: usize) -> &'a [(usize, usize, usize, usize)] {
        let lo = self.h.partition_point(|t| t.0 < row);
        let hi = lo + self.h[lo..].partition_point(|t| t.0 == row);
        &self.h[lo..hi]
    }

    /// All vertical segments whose column lies in `[x0, x1)`.
    fn v_in_cols(&self, x0: usize, x1: usize) -> &'a [(usize, usize, usize, usize)] {
        let lo = self.v.partition_point(|t| t.0 < x0);
        let hi = self.v.partition_point(|t| t.0 < x1);
        &self.v[lo..hi]
    }
}

/// Where blocker queries read edge ink from. `Indexed` is pure
/// acceleration — same answers, arena-resident; `Scan` walks the
/// per-row visitor wrappers per query — same answers, zero arena. The
/// adaptive choice (see [`LABEL_INDEX_MIN_WORK`]) keeps embedded-scale
/// plans free of index memory; correctness never depends on the arm.
pub(super) enum InkSource<'a> {
    Indexed(InkIndex<'a>),
    Scan,
}

impl InkSource<'_> {
    /// Every horizontal run on `row`, as `(x0, x1 inclusive, owner)`.
    fn for_each_h_on_row<V: LayoutView>(
        &self,
        view: &V,
        row: usize,
        f: &mut dyn FnMut(usize, usize, usize),
    ) {
        match self {
            InkSource::Indexed(ix) => {
                for &(_, a, b, owner) in ix.h_at_row(row) {
                    f(a, b, owner);
                }
            }
            InkSource::Scan => {
                for i in 0..view.edge_count() {
                    let e = view.edge(i);
                    for_each_h_run(
                        &e.path,
                        e.from_x,
                        e.from_y,
                        e.to_x,
                        e.to_y,
                        row,
                        e.flow_axis,
                        &mut |a, b| f(a, b, i),
                    );
                }
            }
        }
    }

    /// Every vertical segment crossing `row` whose column lies in
    /// `[x0, x1)`, as `(col, owner)`.
    fn for_each_v_at<V: LayoutView>(
        &self,
        view: &V,
        row: usize,
        x0: usize,
        x1: usize,
        f: &mut dyn FnMut(usize, usize),
    ) {
        match self {
            InkSource::Indexed(ix) => {
                for &(col, vlo, vhi, owner) in ix.v_in_cols(x0, x1) {
                    if vlo <= row && row <= vhi {
                        f(col, owner);
                    }
                }
            }
            InkSource::Scan => {
                for i in 0..view.edge_count() {
                    let e = view.edge(i);
                    for_each_v_col(
                        &e.path,
                        e.from_x,
                        e.from_y,
                        e.to_x,
                        e.to_y,
                        row,
                        e.flow_axis,
                        &mut |c| {
                            if c >= x0 && c < x1 {
                                f(c, i);
                            }
                        },
                    );
                }
            }
        }
    }
}

/// Is any cell of `[x0, x1)` at `row` occupied by something a label may
/// not overwrite? Allowed: empty cells and solid verticals. Blockers:
/// horizontal runs (and their corners), dashed verticals, subgraph
/// border rows/columns, spans claimed by earlier labels, and self-loop
/// marker cells. `show_dummies` mirrors the render option: a hidden
/// dummy waypoint paints nothing, so only visible ones count as nodes.
/// (`pub(super)` for the parity pin on the marker rule.)
pub(super) fn span_blocked<V: LayoutView>(
    view: &V,
    ink: &InkSource<'_>,
    label_edge: usize,
    row: usize,
    x0: usize,
    x1: usize,
    claimed: &[(usize, usize, usize)],
    show_dummies: bool,
) -> bool {
    if claimed
        .iter()
        .any(|&(r, c0, c1)| r == row && c0 < x1 && c1 > x0)
    {
        return true;
    }

    // The self-loop marker is the edge's ENTIRE visible ink — one cell.
    // Text beats markers at the cell layer (labels must stay readable,
    // never hole-punched), so the only way the marker survives is that
    // no label window ever covers its cell: blocked here, in the base
    // check every host shares, unconditionally on both flow axes.
    for ni in 0..view.node_count() {
        if let Some((sx, sy)) = view.node(ni).self_loop_at {
            if sy == row && sx >= x0 && sx < x1 {
                return true;
            }
        }
    }

    // X flows: label rows are node rows — node areas block (Y label
    // rows sit below their level by construction, so this cannot fire
    // there and the Y path stays frozen).
    if matches!(view.edge(label_edge).flow_axis, crate::ir::FlowAxis::X) {
        for ni in 0..view.node_count() {
            let n = view.node(ni);
            if !show_dummies && matches!(n.kind, crate::ir::NodeKind::Dummy) {
                continue;
            }
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

    // Edge ink, from whichever source engaged — rule-for-rule the
    // legacy per-edge visitor walk. Y-flow ink: horizontal runs block
    // (the label edge's OWN included — legacy labels replace only
    // vertical strokes); verticals block only when foreign AND dashed
    // ('┊' is not '│'). X-flow ink is the role mirror: cross verticals
    // block when foreign (any weight — the label edge's own cross ink
    // never blocks its own label, which is what makes D9's vertical
    // host reachable); trunks block only when foreign AND dashed.
    let mut blocked = false;
    ink.for_each_h_on_row(view, row, &mut |hx0, hx1, owner| {
        if !blocked && hx0 < x1 && hx1 + 1 > x0 {
            let oe = view.edge(owner);
            blocked |= match oe.flow_axis {
                crate::ir::FlowAxis::Y => true,
                crate::ir::FlowAxis::X => owner != label_edge && oe.reversed,
            };
        }
    });
    if blocked {
        return true;
    }
    ink.for_each_v_at(view, row, x0, x1, &mut |_, owner| {
        if !blocked {
            let oe = view.edge(owner);
            blocked |= match oe.flow_axis {
                crate::ir::FlowAxis::Y => owner != label_edge && oe.reversed,
                crate::ir::FlowAxis::X => owner != label_edge,
            };
        }
    });
    blocked
}

/// Visit EVERY horizontal run `(row, x0, x1)` (x-inclusive) this path
/// paints, on every row — the row-agnostic authority behind
/// [`for_each_h_run`] and the ink index. One body holds the painter's
/// formulas; the per-row API filters this.
pub(super) fn for_each_h_run_all(
    path: &PathRef<'_>,
    from_x: usize,
    from_y: usize,
    to_x: usize,
    to_y: usize,
    axis: crate::ir::FlowAxis,
    f: &mut dyn FnMut(usize, usize, usize),
) {
    let mut push = |row: usize, x0: usize, x1: usize| f(row, x0.min(x1), x0.max(x1));
    // X flows: horizontal ink is the TRUNK segments (node faces
    // trimmed, arrow cells included, bend corners included) — the
    // exact mirror of the compositor's `paint_edge_x`.
    if matches!(axis, crate::ir::FlowAxis::X) {
        let step: isize = if to_x >= from_x { 1 } else { -1 };
        let trim = |x: usize| (x as isize + step) as usize;
        let back = |x: usize| (x as isize - step) as usize;
        match *path {
            PathRef::Direct | PathRef::Spline { .. } => {
                if from_x.abs_diff(to_x) > 1 {
                    push(from_y, trim(from_x), back(to_x));
                }
            }
            PathRef::Corner { bend_at: bend_x } => {
                push(from_y, trim(from_x), bend_x);
                push(to_y, bend_x, back(to_x));
            }
            PathRef::SideChannel {
                channel_at,
                span_start,
                span_end,
            } => {
                // Three flow segments: source → channel entry, along
                // the far channel row, channel exit → target.
                push(from_y, trim(from_x), span_start);
                push(channel_at, span_start, span_end);
                push(to_y, span_end, back(to_x));
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
                        if px != nx {
                            let a = if first { trim(px) } else { px };
                            let b = if last { back(nx) } else { nx };
                            push(py, a, b);
                        }
                    } else {
                        let bend_x = (px as isize
                            + step * (1 + if first { start_offset as isize } else { 0 }))
                            as usize;
                        let a = if first && start_offset > 0 {
                            trim(px)
                        } else {
                            px
                        };
                        push(py, a, bend_x);
                        let b = if last { back(nx) } else { nx };
                        push(ny, bend_x, b);
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
            push(bend_at, from_x, to_x);
        }
        PathRef::SideChannel {
            channel_at,
            span_start,
            span_end,
        } => {
            push(span_start, from_x, channel_at);
            push(span_end, to_x, channel_at);
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
                    push(corner_y, px, nx);
                } else if py == ny && px != nx {
                    push(py, px, nx);
                }
                px = nx;
                py = ny;
                first = false;
            }
        }
    }
}

/// Visit every horizontal run `[x0, x1]` (inclusive) painted by this
/// path at `row` — the same formulas the painter uses. Visitor-based so
/// arbitrarily long multi-segment paths lose nothing to fixed caps.
/// Filters [`for_each_h_run_all`], the single geometry authority.
pub(super) fn for_each_h_run(
    path: &PathRef<'_>,
    from_x: usize,
    from_y: usize,
    to_x: usize,
    to_y: usize,
    row: usize,
    axis: crate::ir::FlowAxis,
    f: &mut dyn FnMut(usize, usize),
) {
    for_each_h_run_all(path, from_x, from_y, to_x, to_y, axis, &mut |r, a, b| {
        if r == row {
            f(a, b);
        }
    });
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

/// A long flow run prefers its midpoint (temp/11 rule 2, Ash's D9
/// amendment 2026-08-07): when an edge's longest own flow run is at
/// least this many times the label's length, the label is placed at
/// that run's midpoint — the classic wire-label look — instead of near
/// the source or a bend. Short edges keep the legacy near-source look.
const LONG_RUN_LABEL_FACTOR: usize = 3;

/// Visit the edge's own CORNER cells — the bend endpoints of its cross
/// segments, straight from the painter's geometry. Rule 1 (temp/11,
/// 2026-08-07): no label may cover its own corner, in ANY host — the
/// bend is the busiest glyph an edge owns, and text on top of it
/// obscures the turn it marks.
fn for_each_own_corner(
    path: &crate::render::engine::view::PathRef<'_>,
    from_x: usize,
    from_y: usize,
    to_x: usize,
    to_y: usize,
    flow_axis: crate::ir::FlowAxis,
    f: &mut dyn FnMut(usize, usize),
) {
    use crate::render::engine::view::PathRef;
    let is_x = matches!(flow_axis, crate::ir::FlowAxis::X);
    let (from_f, from_c) = if is_x {
        (from_x, from_y)
    } else {
        (from_y, from_x)
    };
    let (to_f, to_c) = if is_x { (to_x, to_y) } else { (to_y, to_x) };
    let dir: isize = if to_f >= from_f { 1 } else { -1 };
    // Emit in physical (x, y) regardless of role space.
    let mut emit = |flow: usize, cross: usize| {
        if is_x {
            f(flow, cross);
        } else {
            f(cross, flow);
        }
    };
    match path {
        PathRef::Direct | PathRef::Spline { .. } => {}
        PathRef::Corner { bend_at } => {
            emit(*bend_at, from_c);
            emit(*bend_at, to_c);
        }
        PathRef::SideChannel {
            channel_at,
            span_start,
            span_end,
        } => {
            // Both painter arms open the first run with corners at
            // BOTH ends (`h_run_with_corners(span_start, from, …)`),
            // so the source-side bend exists on the Y axis too.
            emit(*span_start, from_c);
            emit(*span_start, *channel_at);
            emit(*span_end, *channel_at);
            emit(*span_end, to_c);
        }
        PathRef::MultiSegment {
            waypoints,
            start_offset,
        } => {
            let mut pf = from_f;
            let mut pc = from_c;
            let mut first = true;
            for i in 0..=waypoints.len() {
                let (wx, wy) = if i < waypoints.len() {
                    waypoints[i]
                } else {
                    (to_x, to_y)
                };
                let (nf, nc) = if is_x { (wx, wy) } else { (wy, wx) };
                if pc != nc {
                    let bend = ((pf as isize)
                        + (1 + if first { *start_offset as isize } else { 0 }) * dir)
                        as usize;
                    emit(bend, pc);
                    emit(bend, nc);
                }
                pf = nf;
                pc = nc;
                first = false;
            }
        }
    }
}

/// Rule 1's uniform test: does the window `[x0, x1)` on `row` cover any
/// of this edge's own corner cells?
fn own_corner_in_span<V: LayoutView>(
    view: &V,
    edge: usize,
    row: usize,
    x0: usize,
    x1: usize,
) -> bool {
    let e = view.edge(edge);
    let mut hit = false;
    for_each_own_corner(
        &e.path,
        e.from_x,
        e.from_y,
        e.to_x,
        e.to_y,
        e.flow_axis,
        &mut |cx, cy| hit |= cy == row && cx >= x0 && cx < x1,
    );
    hit
}

/// Visit the edge's own endpoint MARKER cells: the painter draws the
/// from-marker one flow step past the source anchor and the to-marker
/// one step before the target anchor, on the endpoint lines. Host
/// intervals already exclude these as ANCHORS, but a wide window
/// anchored elsewhere can still extend across one — and labels paint
/// after edge ink with text winning unconditionally (`painted_text`),
/// so covering an arrowhead erases the edge's direction glyph, the
/// same class of loss as a covered `↺`. `from_m`/`to_m` are the
/// STYLE-RESOLVED presence flags ([`EdgePlan::resolved_markers`]) —
/// a phantom cell here would block windows the painter leaves as
/// plain stroke. The Y `SideChannel` source never carries a marker
/// (the painter draws no initial flow run at all).
fn for_each_own_marker(
    path: &crate::render::engine::view::PathRef<'_>,
    from_x: usize,
    from_y: usize,
    to_x: usize,
    to_y: usize,
    flow_axis: crate::ir::FlowAxis,
    from_m: bool,
    to_m: bool,
    f: &mut dyn FnMut(usize, usize),
) {
    use crate::render::engine::view::PathRef;
    let is_x = matches!(flow_axis, crate::ir::FlowAxis::X);
    let (from_f, from_c) = if is_x {
        (from_x, from_y)
    } else {
        (from_y, from_x)
    };
    let (to_f, to_c) = if is_x { (to_x, to_y) } else { (to_y, to_x) };
    let dir: isize = if to_f >= from_f { 1 } else { -1 };
    let off = |v: usize, k: isize| (v as isize + k * dir) as usize;
    let mut emit = |flow: usize, cross: usize| {
        if is_x {
            f(flow, cross);
        } else {
            f(cross, flow);
        }
    };
    if from_m && (is_x || !matches!(path, PathRef::SideChannel { .. })) {
        emit(off(from_f, 1), from_c);
    }
    if to_m {
        emit(off(to_f, -1), to_c);
    }
}

/// Does the window `[x0, x1)` on `row` cover one of the edge's own
/// (style-resolved) endpoint marker cells?
fn own_marker_in_span<V: LayoutView>(
    view: &V,
    edge: usize,
    from_m: bool,
    to_m: bool,
    row: usize,
    x0: usize,
    x1: usize,
) -> bool {
    let e = view.edge(edge);
    let mut hit = false;
    for_each_own_marker(
        &e.path,
        e.from_x,
        e.from_y,
        e.to_x,
        e.to_y,
        e.flow_axis,
        from_m,
        to_m,
        &mut |cx, cy| hit |= cy == row && cx >= x0 && cx < x1,
    );
    hit
}

/// The cells of its OWN ink an edge may never label over — corners
/// (Rule 1) and style-resolved endpoint markers — checked identically
/// by every host. (`pub(super)` for the parity sweep.)
pub(super) fn own_fixed_cell_in_span<V: LayoutView>(
    view: &V,
    edge: usize,
    from_m: bool,
    to_m: bool,
    row: usize,
    x0: usize,
    x1: usize,
) -> bool {
    own_corner_in_span(view, edge, row, x0, x1)
        || own_marker_in_span(view, edge, from_m, to_m, row, x0, x1)
}

/// Work budget for the slide tier across ONE plan build — the same
/// graceful-degradation pattern as `LANE_WORK_BUDGET` in the lane
/// pass. Each vetted candidate is charged `edge_count`, the cost of a
/// SCANNED blocker query, in BOTH ink-source modes — deliberately: the
/// index must be pure acceleration, so whether it engaged may never
/// change which labels place; a mode-faithful charge would flip
/// placements right at the adaptive threshold. The purse is therefore
/// a mode-independent cap on candidate volume (4M edge-visit
/// equivalents — the whole corpus spends <60k; an all-labeled
/// ~900-edge worst case is clipped at ~4.7k of its ~21k candidates),
/// with indexed queries costing less than they are charged. Labels
/// past the budget skip the slide tier — float and legend still apply.
const LABEL_SLIDE_WORK_BUDGET: usize = 1 << 22;

/// Work threshold for the ink index, which engages only on HEAP-backed
/// plans (`PlanMem::is_heap`) with at least this `labeled × edges`
/// product. Caller-arena plans ALWAYS scan: their every byte is
/// provisioned against a published estimate, and entry count scales
/// with routed segments — a workload heuristic alone bounds no memory
/// (review round 4). The choice is output-invisible (the oracle pins
/// both arms identical), so the arena path trades only CPU: worst
/// cases resemble the pre-index ~93 ms all-labeled ~900-edge plan,
/// while the same plan heap-side indexes at ~11 ms. One entry is 4
/// `usize`s — 32 B on 64-bit hosts, 16 B on 32-bit.
const LABEL_INDEX_MIN_WORK: usize = 4096;

/// Rank of a tied cross-segment candidate ROW: prefer rows toward the
/// segment's own SOURCE end. The x-derived `toward_flow` sign must
/// never rank a y-coordinate — an x-mirror preserves rows and the
/// segment's row direction, so this key is reflection-invariant where
/// `toward_flow(r)` would prefer the upper row in LR and the lower in
/// RL. (`pub(super)` for the parity pin.)
pub(super) fn cross_row_rank(r: usize, seg_from: usize, seg_to: usize) -> usize {
    if seg_to >= seg_from {
        r
    } else {
        usize::MAX - r
    }
}

/// Rank of a slide window's lateral offset: the centered window (op 0)
/// always wins, then the two edge-anchored ones. For X edges the side
/// order is FLOW-relative — offsets `0` and `len-1` are x-mirror
/// counterparts, so a fixed preference would pick visually different
/// windows in LR and RL when both sides are free; ranking
/// extend-backward-along-flow ahead of extend-forward makes the choice
/// a geometric mirror. Y edges slide on the cross axis, which no
/// direction flip mirrors, so their side order stays fixed.
/// (`pub(super)` for the parity pin.)
pub(super) fn lateral_rank(op: usize, is_x: bool, fwd: bool) -> usize {
    if op == 0 || !is_x || fwd { op } else { 3 - op }
}

/// How many slide candidates one label may test (temp/11 §3.3), the
/// sibling of [`CROSS_HOST_CANDIDATES`]: each candidate costs a
/// `slide_blocked` scan over graph geometry, so the search must be
/// bounded — budget exhausted falls through to Host 3, then the legend.
///
/// 24 rather than the plan's initial 8: positions are generated by a
/// short outward walk from the seed (±2) plus run boundaries, each with
/// three lateral offsets — the walk is what reaches free space that
/// starts just past a blocker (a box border row) without a full
/// blocker-interval analysis. Still a constant; a scan is the same
/// cost as one Host-1 probe, of which there are four.
const LABEL_SLIDE_CANDIDATES: usize = 24;

/// Visit every marker-free FLOW-run interval this edge paints, in
/// source→target order: `f(cross_line, flow_lo, flow_hi)` with the
/// bounds inclusive. For a Y edge a run is vertical (`cross_line` is
/// its column, `flow` positions are rows); for an X edge horizontal.
///
/// This is the placement-side twin of `paint_edge`/`paint_edge_x` in
/// `compose.rs`, arm for arm — `between` endpoints, `start_offset`
/// stubs, the per-axis `SideChannel` shapes (Y paints two flow runs, X
/// paints three), and the arrowhead cells at `off(end, ∓1)` — so a
/// label hosted on a yielded cell replaces genuine `─`/`│` ink and
/// nothing else. Corner cells, junctions, and markers are never
/// yielded. Single waypoint-fill cells are deliberately omitted
/// (junction-adjacent; hosting there reads as sitting on the bend).
#[allow(clippy::too_many_arguments)]
fn for_each_flow_host_segment(
    path: &crate::render::engine::view::PathRef<'_>,
    from_x: usize,
    from_y: usize,
    to_x: usize,
    to_y: usize,
    flow_axis: crate::ir::FlowAxis,
    from_m: bool,
    to_m: bool,
    f: &mut dyn FnMut(usize, usize, usize),
) {
    use crate::render::engine::view::PathRef;
    let is_x = matches!(flow_axis, crate::ir::FlowAxis::X);
    // Role split: flow coordinate vs cross line, per axis.
    let (from_f, from_c) = if is_x {
        (from_x, from_y)
    } else {
        (from_y, from_x)
    };
    let (to_f, to_c) = if is_x { (to_x, to_y) } else { (to_y, to_x) };
    let dir: isize = if to_f >= from_f { 1 } else { -1 };
    let off = |v: usize, k: isize| (v as isize + k * dir) as usize;

    // `between(a, b)` in the painter is exclusive of both endpoints:
    // cells min+1 ..= max-1. Emit that interval minus up to one marker
    // cell at either end, splitting when the marker is interior.
    let mut emit = |line: usize, a: usize, b: usize, cut_from: bool, cut_to: bool| {
        let (lo, hi) = (a.min(b) + 1, a.max(b).wrapping_sub(1));
        if lo > hi || a == b {
            return;
        }
        let mut lo2 = lo;
        let mut hi2 = hi;
        if cut_from && from_m {
            let m = off(a, 1);
            if m == lo2 {
                lo2 += 1;
            } else if m == hi2 {
                hi2 = hi2.wrapping_sub(1);
            } else if m > lo2 && m < hi2 {
                f(line, lo2, m - 1);
                lo2 = m + 1;
            }
        }
        if cut_to && to_m {
            let m = off(b, -1);
            if m == lo2 {
                lo2 += 1;
            } else if m == hi2 {
                hi2 = hi2.wrapping_sub(1);
            } else if m > lo2 && m < hi2 {
                f(line, lo2, m - 1);
                lo2 = m + 1;
            }
        }
        if lo2 <= hi2 {
            f(line, lo2, hi2);
        }
    };

    match path {
        PathRef::Direct | PathRef::Spline { .. } => {
            emit(from_c, from_f, to_f, true, true);
        }
        PathRef::Corner { bend_at } => {
            emit(from_c, from_f, *bend_at, true, false);
            emit(to_c, *bend_at, to_f, false, true);
        }
        PathRef::SideChannel {
            channel_at,
            span_start,
            span_end,
        } => {
            if is_x {
                // X paints three flow runs (source row, channel row,
                // target row); the from-marker lives on the first.
                emit(from_c, from_f, *span_start, true, false);
                emit(*channel_at, *span_start, *span_end, false, false);
                emit(to_c, *span_end, to_f, false, true);
            } else {
                // Y paints only two: the channel column and the final
                // approach — there is no initial flow run and no
                // from-marker in this arm of the painter.
                emit(*channel_at, *span_start, *span_end, false, false);
                emit(to_c, *span_end, to_f, false, true);
            }
        }
        PathRef::MultiSegment {
            waypoints,
            start_offset,
        } => {
            let mut pf = from_f;
            let mut pc = from_c;
            let mut first = true;
            for i in 0..=waypoints.len() {
                let (wx, wy) = if i < waypoints.len() {
                    waypoints[i]
                } else {
                    (to_x, to_y)
                };
                let (nf, nc) = if is_x { (wx, wy) } else { (wy, wx) };
                let last = i == waypoints.len();
                if pc == nc {
                    // Flow-parallel segment on line `pc`.
                    emit(pc, pf, nf, first, last);
                } else {
                    // Cross transition; the painter draws a flow stub
                    // before the bend when this is the offset first
                    // segment, and the closing flow run after it.
                    let bend = off(pf, 1 + if first { *start_offset as isize } else { 0 });
                    if first && *start_offset > 0 {
                        emit(pc, pf, bend, true, false);
                    }
                    emit(nc, bend, nf, false, last);
                }
                pf = nf;
                pc = nc;
                first = false;
            }
        }
    }
}

/// The blocker check for SLID candidates (temp/11 §3.2) — strictly
/// harsher than [`span_blocked`], because sliding abandons every
/// by-construction guarantee the legacy seed/float positions carry:
///
/// - node bodies block on BOTH axes (the base check skips them for Y
///   labels only because Y seeds sit on routing rows — sliding can
///   reach node rows, and nodes paint after labels);
/// - EVERY other edge's ink blocks, solid trunks included — the legacy
///   rule lets labels overwrite foreign solid strokes, but a slid
///   label sitting on someone else's line reads as annotating it;
/// - the label text cells of every subgraph box block (box labels
///   would collide as text-on-text).
///
/// (Self-loop marker cells block in [`span_blocked`] itself — every
/// host, not just slides — because covering one erases a whole edge.)
///
/// The label edge's OWN ink stays exempt — hosting on its own flow run
/// is the point.
pub(super) fn slide_blocked<V: LayoutView>(
    view: &V,
    ink: &InkSource<'_>,
    label_edge: usize,
    row: usize,
    x0: usize,
    x1: usize,
    claimed: &[(usize, usize, usize)],
    show_dummies: bool,
    sg_label_row: &dyn Fn(usize, &crate::render::engine::view::SubgraphRef<'_>) -> usize,
) -> bool {
    if span_blocked(view, ink, label_edge, row, x0, x1, claimed, show_dummies) {
        return true;
    }
    // Node bodies, regardless of the label edge's flow axis. A dummy
    // waypoint is a node only when the render SHOWS it (its marker
    // paints after labels and would punch the text); hidden, its cell
    // is plain edge ink and blocking it would be phantom.
    for ni in 0..view.node_count() {
        let n = view.node(ni);
        if !show_dummies && matches!(n.kind, crate::ir::NodeKind::Dummy) {
            continue;
        }
        if row >= n.y && row < n.y + n.height && n.x < x1 && n.x + n.width > x0 {
            return true;
        }
    }
    // The subgraph label's ACTUAL text cells (resolved style) — not the
    // whole interior row, and not both rows. Row-level blocking stole
    // riser-adjacent host rows (hero-LR `emit`); and because box labels
    // deliberately anchor to the visual top in every direction (text is
    // never mirrored), any block wider than the text itself makes
    // placement feasibility direction-dependent near boxes.
    for si in 0..view.subgraph_count() {
        let sg = view.subgraph(si);
        if sg.height >= 3 && sg.width >= 4 && row == sg_label_row(si, &sg) {
            let text_len = sg.label.chars().count().min(sg.width - 4);
            let (lx0, lx1) = (sg.x + 2, sg.x + 2 + text_len);
            if lx0 < x1 && lx1 > x0 {
                return true;
            }
        }
    }
    // Every other edge's ink — flow runs and cross runs alike, any
    // weight — EXCEPT cells the label edge itself also paints. Edges
    // that merge into a shared trunk overlap stroke-for-stroke there;
    // a label interrupting the shared line is the legacy look and the
    // only way either edge's label can host at all. Foreign ink that
    // is NOT collinear with own ink still blocks: a slid label parked
    // on a stranger's line would read as annotating it.
    //
    // Own coverage reads the same ink source (owner == label_edge),
    // gathered into a tiny interval set once per call; a pathological
    // own path overflowing the buffer falls back to per-cell source
    // stabs — slower, never wrong.
    const OWN_IVALS: usize = 8;
    let mut own_iv = [(0usize, 0usize); OWN_IVALS];
    let mut n_iv = 0usize;
    let mut iv_overflow = false;
    {
        let mut push_iv = |a: usize, b: usize| {
            if n_iv < OWN_IVALS {
                own_iv[n_iv] = (a, b);
                n_iv += 1;
            } else {
                iv_overflow = true;
            }
        };
        ink.for_each_h_on_row(view, row, &mut |a, b, owner| {
            if owner == label_edge {
                push_iv(a, b + 1);
            }
        });
        ink.for_each_v_at(view, row, x0, x1, &mut |col, owner| {
            if owner == label_edge {
                push_iv(col, col + 1);
            }
        });
    }
    let own_covers = |c: usize| -> bool {
        if own_iv[..n_iv].iter().any(|&(a, b)| a <= c && c < b) {
            return true;
        }
        if !iv_overflow {
            return false;
        }
        let mut cov = false;
        ink.for_each_h_on_row(view, row, &mut |a, b, o| {
            cov |= o == label_edge && a <= c && c <= b;
        });
        if !cov {
            ink.for_each_v_at(view, row, c, c + 1, &mut |_, o| {
                cov |= o == label_edge;
            });
        }
        cov
    };
    let mut blocked = false;
    ink.for_each_h_on_row(view, row, &mut |a, b, owner| {
        if !blocked && owner != label_edge && a < x1 && b + 1 > x0 {
            for c in a.max(x0)..(b + 1).min(x1) {
                if !own_covers(c) {
                    blocked = true;
                    break;
                }
            }
        }
    });
    if blocked {
        return true;
    }
    ink.for_each_v_at(view, row, x0, x1, &mut |col, owner| {
        if !blocked && owner != label_edge && !own_covers(col) {
            blocked = true;
        }
    });
    blocked
}

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
/// Visit EVERY vertical ink segment `(col, row_lo, row_hi)` this path
/// paints, rows inclusive and INTERIOR-only (endpoint/corner cells
/// belong to horizontal ink, exactly as the per-row test excluded them
/// with its strictly-between rule). The painter's gap-fill cell on a
/// MultiSegment waypoint row is a one-row segment. The row-agnostic
/// authority behind [`for_each_v_col`] and the ink index.
pub(super) fn for_each_v_seg_all(
    path: &PathRef<'_>,
    from_x: usize,
    from_y: usize,
    to_x: usize,
    to_y: usize,
    axis: crate::ir::FlowAxis,
    f: &mut dyn FnMut(usize, usize, usize),
) {
    // The interior of [a, b] in either order: min+1 ..= max−1.
    fn interior(f: &mut dyn FnMut(usize, usize, usize), col: usize, a: usize, b: usize) {
        let (lo, hi) = (a.min(b) + 1, a.max(b).wrapping_sub(1));
        if lo <= hi && a != b {
            f(col, lo, hi);
        }
    }
    // X flows: vertical ink is the CROSS runs, strictly between the
    // trunk rows (corner cells belong to the trunk rows' horizontal
    // ink). One authority for that geometry — delegate.
    if matches!(axis, crate::ir::FlowAxis::X) {
        for_each_x_cross_segment(path, from_x, from_y, to_x, to_y, &mut |col, a, b| {
            interior(f, col, a, b);
        });
        return;
    }
    match *path {
        PathRef::Direct | PathRef::Spline { .. } => {
            interior(f, from_x, from_y, to_y);
        }
        PathRef::Corner { bend_at } => {
            interior(f, from_x, from_y, bend_at);
            interior(f, to_x, bend_at, to_y);
        }
        PathRef::SideChannel {
            channel_at,
            span_start,
            span_end,
        } => {
            interior(f, channel_at, span_start, span_end);
            interior(f, to_x, span_end, to_y);
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
                // row itself (the painter's gap fill) — a one-row
                // segment.
                if !first {
                    f(px, py, py);
                }
                if px == nx {
                    interior(f, px, py, ny);
                } else if py != ny {
                    let step = 1 + if first { start_offset as isize } else { 0 };
                    let corner_y = (py as isize + dir * step) as usize;
                    interior(f, px, py, corner_y);
                    interior(f, nx, corner_y, ny);
                }
                px = nx;
                py = ny;
                first = false;
            }
        }
    }
}

pub(super) fn for_each_v_col(
    path: &PathRef<'_>,
    from_x: usize,
    from_y: usize,
    to_x: usize,
    to_y: usize,
    row: usize,
    axis: crate::ir::FlowAxis,
    f: &mut dyn FnMut(usize),
) {
    for_each_v_seg_all(
        path,
        from_x,
        from_y,
        to_x,
        to_y,
        axis,
        &mut |col, lo, hi| {
            if lo <= row && row <= hi {
                f(col);
            }
        },
    );
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
        let plan = RenderPlan::build(&ir, &RenderOptions::colored(Palette::Ansi).plan);
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
        let plan = RenderPlan::build(&ir, &RenderOptions::colored(Palette::Ansi).plan);
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
        let plan = RenderPlan::build(&ir, &RenderOptions::colored(Palette::Ansi).plan);
        let palette = Palette::Ansi.colors();
        let legacy = ir.compute_edge_colors(palette.len());
        for (i, want_idx) in legacy.iter().enumerate() {
            assert_eq!(
                plan.edge_plan(i).color,
                super::super::color::CellColor::ansi256(palette[*want_idx % palette.len()]),
                "edge {i} palette color"
            );
        }
        // Colors are ALWAYS resolved at plan time — a plain-preset
        // plan carries the same resolved colors (plain emission just
        // ignores them). One plan serves colored and plain output.
        let plain = RenderPlan::build(&ir, &RenderOptions::plain().plan);
        for i in 0..legacy.len() {
            assert_eq!(
                plain.edge_plan(i).color,
                plan.edge_plan(i).color,
                "edge {i}: color resolution must not depend on emission mode"
            );
        }
    }

    #[test]
    fn hit_testing_finds_nodes_boxes_and_nothing() {
        let g = stage_graph();
        let ir = g.compute_layout();
        let plan = RenderPlan::build(&ir, &RenderOptions::plain().plan);

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
        let plan = RenderPlan::build(&ir, &RenderOptions::plain().plan);
        assert!(plan.elements().windows(2).all(|w| w[0].y_min <= w[1].y_min));
        let bands: Vec<(usize, usize)> = plan
            .bands(super::super::config::DEFAULT_BAND_ROWS)
            .collect();
        assert_eq!(bands, vec![(0, plan.height())]);
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
        let plan = RenderPlan::build(&ir, &RenderOptions::plain().plan);
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
        let plan = RenderPlan::build(&ir, &RenderOptions::plain().plan);
        assert_eq!(
            plan.bands(super::super::config::DEFAULT_BAND_ROWS).count(),
            1
        );
    }
}
