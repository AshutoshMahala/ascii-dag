//! The ownership rasterizer — the owner plane behind
//! `CellView.owner`.
//!
//! Ownership is rasterized per band into a `u32` plane in hit-test
//! priority order (subgraphs, write-if-empty → edges, ascending with
//! a run-dedup claim fill → nodes → painted labels), so every cell
//! answers exactly what `element_at`/`Scene::hit_test` answers —
//! pinned by the permanent cells-and-hit-testing-agree gate. Owners
//! are encoded as element slot + 1 (`0` = none) into the plan's
//! spatial-element table; the kind resolves through the table, so the
//! full index range stays available.
//!
//! Three disciplines keep the pass linear, and all three are
//! load-bearing (measured; dropping any one is quadratic somewhere):
//!
//! 1. **Run-dedup claim fill** — overlapping fan runs write each cell
//!    once (path-compressed next-unclaimed array).
//! 2. **Counting-sort row buckets** — each element visits only its
//!    own rows, through flat, arena-carvable arrays.
//! 3. **Rolling active-element sweep** — bands arrive ascending;
//!    elements are admitted by sorted `y_min` and retired past
//!    `y_max`, touched O(1) times across the whole pass.
//!
//! All scratch is caller-provided slices ([`OwnerScratch`]) sized by
//! [`CompositionRequirements`](super::composer::CompositionRequirements)
//! — the composer carves them from its retained workspace chunk.

use super::plan::{ElementKind, RenderPlan, for_each_h_run, for_each_v_col};
use super::view::LayoutView;

pub(crate) const OWNER_NONE: u32 = 0;

pub(crate) fn owner_to_hit<V: LayoutView>(
    plan: &RenderPlan<'_>,
    view: &V,
    owner: u32,
) -> super::HitResult {
    if owner == OWNER_NONE {
        return super::HitResult::None;
    }
    let el = &plan.elements()[(owner - 1) as usize];
    match el.kind {
        ElementKind::Node => {
            let n = view.node(el.index);
            if matches!(n.kind, crate::ir::NodeKind::Dummy) {
                // Dummies never expose their synthetic backend ids —
                // their semantic identity is (input edge, level).
                super::HitResult::Dummy {
                    edge: n.edge_index.unwrap_or(usize::MAX),
                    level: n.level,
                }
            } else {
                super::HitResult::Node(n.id)
            }
        }
        ElementKind::Edge => super::HitResult::Edge(el.index),
        ElementKind::Subgraph => super::HitResult::Subgraph(view.subgraph(el.index).id),
    }
}

/// Slice-backed rasterizer workspace — carved from the composer's
/// retained chunk. Part of the composition requirements, not per-call
/// allocation. Sizes: `claim_next` width+1; `edge_slot` edge count;
/// `by_y_min`/`active` element count; `row_off` band_rows+1;
/// `row_cur` band_rows; `row_inc` ≥ [`owner_incidence_capacity`].
pub(crate) struct OwnerScratch<'a> {
    /// Pointer-jumping "next unclaimed cell" array.
    pub claim_next: &'a mut [u32],
    /// `edge_index → element slot + 1`, built once per plan.
    pub edge_slot: &'a mut [u32],
    /// Element slots (`+1`) sorted by `y_min` — the rolling sweep's
    /// entry order. Built once per plan.
    pub by_y_min: &'a mut [u32],
    /// Slots whose y-ranges intersect the current band.
    pub active: &'a mut [u32],
    /// Counting-sort row buckets: per-row offsets into `row_inc`.
    pub row_off: &'a mut [u32],
    /// Per-row placement cursors for the counting sort.
    pub row_cur: &'a mut [u32],
    /// Flat row-incidence entries (`slot + 1` per element per row).
    pub row_inc: &'a mut [u32],
}

/// Rolling-sweep position, owned by the caller across one ascending
/// band pass.
#[derive(Default)]
pub(crate) struct OwnerSweep {
    cursor: usize,
    active_len: usize,
}

/// Upper bound on any single band's row-incidence entries: each
/// element contributes at most `min(row span, band_rows)` rows to one
/// band. O(elements), computed at requirements time. Checked: `None`
/// when the sum overflows (the requirement then reports unfittable).
pub(crate) fn owner_incidence_capacity(plan: &RenderPlan<'_>, band_rows: usize) -> Option<usize> {
    plan.elements().iter().try_fold(0usize, |acc, el| {
        acc.checked_add((el.y_max - el.y_min + 1).min(band_rows))
    })
}

/// Fill the per-plan tables and reset the sweep. Call once before a
/// (re)pass of ascending bands.
pub(crate) fn owner_prepare(
    plan: &RenderPlan<'_>,
    scratch: &mut OwnerScratch<'_>,
    sweep: &mut OwnerSweep,
) {
    scratch.edge_slot.fill(OWNER_NONE);
    for (slot, el) in plan.elements().iter().enumerate() {
        if matches!(el.kind, ElementKind::Edge) {
            scratch.edge_slot[el.index] = slot as u32 + 1;
        }
    }
    for (i, o) in scratch.by_y_min.iter_mut().enumerate() {
        *o = i as u32 + 1;
    }
    scratch
        .by_y_min
        .sort_unstable_by_key(|&o| plan.elements()[(o - 1) as usize].y_min);
    sweep.cursor = 0;
    sweep.active_len = 0;
}

// ── Candidate A: the ownership rasterizer ────────────────────────────────

/// Rasterize ownership for rows `[y0, y1)` into `plane`
/// (`width × (y1 - y0)`, row-major). Priority is encoded as write
/// order per row: subgraphs (first-in-index wins, write-if-empty),
/// then edges (ascending index, claim fill), then nodes, then painted
/// labels — mirroring `element_at` branch for branch.
#[allow(clippy::too_many_arguments)]
pub(crate) fn owner_rasterize_band<V: LayoutView>(
    plan: &RenderPlan<'_>,
    view: &V,
    y0: usize,
    y1: usize,
    width: usize,
    plane: &mut [u32],
    scratch: &mut OwnerScratch<'_>,
    sweep: &mut OwnerSweep,
) {
    let OwnerScratch {
        claim_next,
        edge_slot,
        by_y_min,
        active,
        row_off,
        row_cur,
        row_inc,
    } = &mut *scratch;
    plane.fill(OWNER_NONE);
    let rows = y1 - y0;
    let elements = plan.elements();

    // Rolling active-element sweep: bands arrive in ascending row
    // order, so admit elements as their y_min enters the band and
    // retire them once y_max falls behind — every element is touched
    // O(1) times across the whole pass, never once per band.
    let mut kept = 0;
    for i in 0..sweep.active_len {
        let o = active[i];
        if elements[(o - 1) as usize].y_max >= y0 {
            active[kept] = o;
            kept += 1;
        }
    }
    sweep.active_len = kept;
    while sweep.cursor < by_y_min.len() {
        let o = by_y_min[sweep.cursor];
        let el = &elements[(o - 1) as usize];
        if el.y_min >= y1 {
            break;
        }
        if el.y_max >= y0 {
            active[sweep.active_len] = o;
            sweep.active_len += 1;
        }
        sweep.cursor += 1;
    }
    // Keep the priority scans deterministic: buckets in slot order.
    active[..sweep.active_len].sort_unstable();

    // Row buckets from the ACTIVE set only, as a counting sort into
    // flat arrays (arena-carvable) — O(Σ per-element row spans within
    // the band), never O(rows × elements).
    row_off[..=rows].fill(0);
    for &o in active[..sweep.active_len].iter() {
        let el = &elements[(o - 1) as usize];
        let lo = el.y_min.max(y0);
        let hi = (el.y_max + 1).min(y1);
        for y in lo..hi {
            row_off[y - y0 + 1] += 1;
        }
    }
    for r in 0..rows {
        row_off[r + 1] += row_off[r];
    }
    row_cur[..rows].copy_from_slice(&row_off[..rows]);
    for &o in active[..sweep.active_len].iter() {
        let el = &elements[(o - 1) as usize];
        let lo = el.y_min.max(y0);
        let hi = (el.y_max + 1).min(y1);
        for y in lo..hi {
            let r = y - y0;
            row_inc[row_cur[r] as usize] = o;
            row_cur[r] += 1;
        }
    }

    fn find(next: &mut [u32], i: usize) -> usize {
        let mut j = i;
        while next[j] != j as u32 {
            j = next[j] as usize;
        }
        let mut k = i;
        while next[k] != j as u32 {
            let n = next[k] as usize;
            next[k] = j as u32;
            k = n;
        }
        j
    }

    for row in 0..rows {
        let y = y0 + row;
        let base = row * width;
        let bucket = &row_inc[row_off[row] as usize..row_off[row + 1] as usize];

        // Pass 1: subgraphs — below everything; first-in-index wins,
        // so ascending order with write-if-empty.
        for &owner in bucket {
            let el = &elements[(owner - 1) as usize];
            if !matches!(el.kind, ElementKind::Subgraph) {
                continue;
            }
            let sg = view.subgraph(el.index);
            let sp = plan.subgraph_plan(el.index);
            if matches!(sp.border, super::style::SubgraphBorder::None) {
                if sg.width >= 4 && sg.height >= 3 && !sg.label.is_empty() {
                    let label_y = match sp.label_pos {
                        super::style::LabelPosition::InsideTop => sg.y + 1,
                        super::style::LabelPosition::InsideBottom => {
                            (sg.y + sg.height).saturating_sub(2)
                        }
                    };
                    if label_y == y {
                        let len = sg.label.chars().count().min(sg.width - 4);
                        for x in sg.x + 2..(sg.x + 2 + len).min(width) {
                            if plane[base + x] == OWNER_NONE {
                                plane[base + x] = owner;
                            }
                        }
                    }
                }
            } else {
                for x in sg.x..(sg.x + sg.width).min(width) {
                    if plane[base + x] == OWNER_NONE {
                        plane[base + x] = owner;
                    }
                }
            }
        }

        // Pass 2: edges — lowest index wins: ascending order, claim
        // fill (each cell edge-claimed once; subgraph ink below stays
        // overwritable, node/label ink above overwrites freely).
        for (i, slot) in claim_next.iter_mut().enumerate() {
            *slot = i as u32;
        }
        for &owner in bucket {
            let el = &elements[(owner - 1) as usize];
            if !matches!(el.kind, ElementKind::Edge) {
                continue;
            }
            let e = view.edge(el.index);
            let mut claim = |plane: &mut [u32], x0: usize, x1: usize| {
                let mut x = find(claim_next, x0);
                while x <= x1 && x < width {
                    plane[base + x] = owner;
                    claim_next[x] = x as u32 + 1;
                    x = find(claim_next, x + 1);
                }
            };
            for_each_v_col(
                &e.path,
                e.from_x,
                e.from_y,
                e.to_x,
                e.to_y,
                y,
                e.flow_axis,
                &mut |c| {
                    if c < width {
                        claim(plane, c, c);
                    }
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
                &mut |x0, x1| {
                    claim(plane, x0, x1.min(width.saturating_sub(1)));
                },
            );
        }

        // Pass 3: nodes — overwrite everything below. Dummies are a
        // single marker cell, only when shown. Self-loop markers
        // follow today's element_at rule (node-owned); once preserved
        // self-loop records exist, the marker and its label belong to
        // the self-loop EDGE instead, and this pass re-points with it.
        for &owner in bucket {
            let el = &elements[(owner - 1) as usize];
            if !matches!(el.kind, ElementKind::Node) {
                continue;
            }
            let n = view.node(el.index);
            if matches!(n.kind, crate::ir::NodeKind::Dummy) {
                if plan.show_dummy_nodes() && n.y == y && n.x < width {
                    plane[base + n.x] = owner;
                }
                continue;
            }
            if y >= n.y && y < n.y + n.height.max(1) {
                for x in n.x..(n.x + n.width).min(width) {
                    plane[base + x] = owner;
                }
            }
            if let Some((sx, sy)) = n.self_loop_at {
                if sy == y && sx < width {
                    plane[base + sx] = owner;
                }
            }
        }
    }

    // Pass 4: painted labels — top priority, owned by their edge.
    for label in plan.labels() {
        if !label.paints_under(plan.label_placement()) {
            continue;
        }
        if label.y >= y0 && label.y < y1 {
            let owner = edge_slot[label.edge_index];
            let base = (label.y - y0) * width;
            for x in label.x..(label.x + label.len).min(width) {
                plane[base + x] = owner;
            }
        }
    }
}

#[cfg(all(test, feature = "std", feature = "layout-vertical"))]
mod tests {
    use super::*;
    use crate::RenderOptions;
    use crate::graph::Graph;

    /// Manual cost report — wide fans AND tall chains (the row-bucket
    /// + rolling-sweep disciplines keep both linear). Run with:
    ///   cargo test --release --features arena owner_cost_report -- --ignored --nocapture
    #[test]
    #[ignore = "reporting tool, not an assertion"]
    fn owner_cost_report() {
        fn fan(n: usize) -> Graph<'static> {
            let mut g = Graph::new();
            g.add_node(0usize, "R");
            for i in 1..=n {
                g.add_node(i, "c");
                g.add_edge(0usize, i, None);
            }
            g
        }
        fn chain(n: usize) -> Graph<'static> {
            let mut g = Graph::new();
            for i in 0..n {
                g.add_node(i, "n");
            }
            for i in 0..n - 1 {
                g.add_edge(i, i + 1, None);
            }
            g
        }
        let shapes: Vec<(&str, Graph<'static>)> = vec![
            ("fan-500", fan(500)),
            ("fan-5000", fan(5_000)),
            ("fan-50000", fan(50_000)),
            ("chain-5000", chain(5_000)),
            ("chain-20000", chain(20_000)),
        ];
        for (name, g) in shapes {
            let ir = g.compute_layout();
            let options = RenderOptions::plain();
            let plan = RenderPlan::build(&ir, &options.plan);
            let (w, h) = (ir.width(), ir.height());
            let band_rows = plan
                .max_band_rows(super::super::config::DEFAULT_BAND_ROWS)
                .min(h)
                .max(1);
            let elements = plan.elements().len();
            let mut plane = vec![OWNER_NONE; w * band_rows];
            let mut claim = vec![0u32; w + 1];
            let mut edge_slot = vec![0u32; ir.edges().len()];
            let mut by_y_min = vec![0u32; elements];
            let mut active = vec![0u32; elements];
            let mut row_off = vec![0u32; band_rows + 1];
            let mut row_cur = vec![0u32; band_rows];
            let mut row_inc = vec![0u32; owner_incidence_capacity(&plan, band_rows).unwrap()];
            let mut scratch = OwnerScratch {
                claim_next: &mut claim,
                edge_slot: &mut edge_slot,
                by_y_min: &mut by_y_min,
                active: &mut active,
                row_off: &mut row_off,
                row_cur: &mut row_cur,
                row_inc: &mut row_inc,
            };
            let mut sweep = OwnerSweep::default();
            owner_prepare(&plan, &mut scratch, &mut sweep);

            let t = std::time::Instant::now();
            let mut y0 = 0;
            while y0 < h {
                let y1 = (y0 + band_rows).min(h);
                owner_rasterize_band(
                    &plan,
                    &ir,
                    y0,
                    y1,
                    w,
                    &mut plane[..w * (y1 - y0)],
                    &mut scratch,
                    &mut sweep,
                );
                y0 = y1;
            }
            let elapsed = t.elapsed();
            eprintln!(
                "{name}: canvas {w}x{h}, plane {} KiB/band + claim {} KiB + incidence {} KiB | {elapsed:?}",
                (w * band_rows * 4) / 1024,
                ((w + 1) * 4) / 1024,
                (scratch.row_inc.len() * 4) / 1024,
            );
        }
    }
}
