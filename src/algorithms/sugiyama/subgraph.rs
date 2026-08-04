//! Subgraph (cluster) layout helpers for the Sugiyama pipeline.
//!
//! All functions in this module are **no-ops when no subgraphs are defined**
//! — [`Graph::has_subgraphs()`] gates every entry point.
//!
//! ## Pipeline Integration
//!
//! These helpers are called by the heap pipeline at specific stages:
//!
//! 1. **After crossing reduction + x-assignment:**
//!    [`subgraph_padding`] inserts horizontal padding at subgraph boundary
//!    transitions so borders don't overprint node text.
//!
//! 2. **After sibling overlap repair ([`fix_subgraph_overlaps`]):**
//!    [`clear_external_overlaps`] pushes unaffiliated nodes clear of each
//!    cluster's projected border envelope (cluster-width feedback).
//!
//! 3. **After final coordinate assignment:**
//!    [`compute_bounding_boxes`] walks the node list to emit
//!    [`SubgraphInfo`] bounding boxes, propagating nested children
//!    bottom-up.
//!
//! Crossing reduction is block-partitioned via
//! [`block_partition_level`] which the heap pipeline calls in place of
//! its default ordering pass when subgraphs are present.

use super::geometry::Axis;
use super::heap::VNode;
use crate::graph::Graph;
use crate::ir::SubgraphInfo;
use alloc::vec;
use alloc::vec::Vec;

#[cfg(not(feature = "std"))]
use alloc::collections::{BTreeMap as HashMap, BTreeSet as HashSet};
#[cfg(feature = "std")]
use std::collections::{HashMap, HashSet};

// ── VNode subgraph resolution ────────────────────────────────────────────

/// Resolve the subgraph ID for a virtual node.
///
/// - **Real nodes** — look up `Graph::node_subgraph`.
/// - **Dummy nodes** — return the subgraph only if *both* endpoints of
///   the edge belong to the **same** subgraph.  Cross-subgraph dummies
///   float freely (return `None`).
pub(crate) fn vnode_subgraph(dag: &Graph<'_>, vnode: &VNode) -> Option<usize> {
    match vnode {
        VNode::Real(idx) => {
            let id = dag.nodes[*idx].0;
            dag.node_subgraph(id)
        }
        VNode::Dummy { edge_idx } => {
            let (from_id, to_id, _) = dag.edges[*edge_idx];
            let from_sg = dag.node_subgraph(from_id);
            let to_sg = dag.node_subgraph(to_id);
            match (from_sg, to_sg) {
                (Some(a), Some(b)) if a == b => Some(a),
                _ => None,
            }
        }
    }
}

// ── Block-partitioned crossing reduction ─────────────────────────────────

/// Walk the subgraph ancestry to find the root ancestor (the one with no parent).
/// Returns `None` for unaffiliated nodes.
fn root_subgraph(dag: &Graph<'_>, sg_id: Option<usize>) -> Option<usize> {
    let mut cur = sg_id;
    let mut root = sg_id;
    while let Some(id) = cur {
        root = cur;
        cur = dag
            .subgraphs
            .iter()
            .find(|s| s.id == id)
            .and_then(|s| s.parent_id);
    }
    root
}

/// Number of ancestors ABOVE a box (0 for a root box). Used by the
/// non-merging profiles to reserve nesting pads in the packing.
fn ancestor_count(dag: &Graph<'_>, sg_id: usize) -> usize {
    let mut n = 0;
    let mut cur = dag
        .subgraphs
        .iter()
        .find(|s| s.id == sg_id)
        .and_then(|s| s.parent_id);
    while let Some(id) = cur {
        n += 1;
        cur = dag
            .subgraphs
            .iter()
            .find(|s| s.id == id)
            .and_then(|s| s.parent_id);
    }
    n
}

/// Leading cross-axis margin a node inside `sg` needs: the immediate
/// box pad, plus — for non-merging profiles — one label-side pad per
/// ancestor (see `Axis::NESTED_PADS_MERGE`). The compaction and
/// refinement margins must agree with `subgraph_padding`'s
/// reservation or they squeeze it back out.
pub(crate) fn leading_cross_pad<A: Axis>(dag: &Graph<'_>, sg: Option<usize>) -> usize {
    match sg {
        Some(sg_id) if !A::NESTED_PADS_MERGE => {
            A::SG_PAD_CROSS.0 + ancestor_count(dag, sg_id) * A::PARENT_CHILD_PAD_CROSS.0
        }
        Some(_) => A::SG_PAD_CROSS.0,
        None => 0,
    }
}

/// Partition a single virtual level into per-subgraph blocks.
///
/// Returns the level re-ordered so that nodes in the same root-level
/// subgraph tree are contiguous, with unaffiliated nodes placed between
/// blocks according to their original median position.
///
/// Uses the **root ancestor** subgraph as the block key so that sibling
/// subgraphs (e.g. AZ-1 and AZ-2 inside eu-west-1) stay together and
/// unaffiliated cross-subgraph edge dummies are placed outside the group
/// rather than splitting siblings apart.
///
/// The caller runs the normal median/exchange reduction **within** each
/// block, then calls this function to re-order blocks by their average
/// position.
pub(crate) fn block_partition_level(dag: &Graph<'_>, level: &[VNode]) -> Vec<VNode> {
    if level.is_empty() {
        return Vec::new();
    }

    // Assign each vnode to a block key: root ancestor subgraph ID, or None
    // (unaffiliated). `order` records first appearance so the block list can
    // be built in level order rather than from map iteration — under `std`
    // these are `HashMap`/`HashSet`, whose iteration order is seeded per
    // process, and the stable sort below would then resolve equal averages
    // differently from one run to the next.
    let mut order: Vec<Option<usize>> = Vec::new();
    let mut blocks: HashMap<Option<usize>, Vec<(usize, VNode)>> = HashMap::new();
    for (pos, vnode) in level.iter().enumerate() {
        let sg = vnode_subgraph(dag, vnode);
        let root_sg = root_subgraph(dag, sg);
        let members = blocks.entry(root_sg).or_insert_with(|| {
            order.push(root_sg);
            Vec::new()
        });
        members.push((pos, *vnode));
    }

    // Compute average original position per block for ordering
    let mut block_list: Vec<(Option<usize>, f64, Vec<VNode>)> = order
        .into_iter()
        .map(|key| {
            let members = blocks.remove(&key).unwrap_or_default();
            let avg = members.iter().map(|(pos, _)| *pos as f64).sum::<f64>()
                / members.len().max(1) as f64;
            let vnodes: Vec<VNode> = members.into_iter().map(|(_, v)| v).collect();
            (key, avg, vnodes)
        })
        .collect();

    // Sort blocks by average position. The sort is stable and the list is now
    // in level order, so blocks with equal averages genuinely stay in place.
    block_list.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(core::cmp::Ordering::Equal));

    // Flatten back into a single level
    let mut result = Vec::with_capacity(level.len());
    for (_, _, vnodes) in block_list {
        result.extend(vnodes);
    }
    result
}

// ── Subgraph padding ─────────────────────────────────────────────────────

/// Insert horizontal padding into x-coordinates at subgraph boundary
/// transitions.
///
/// For each pair of adjacent vnodes on a level, if they belong to
/// different subgraphs (or one is inside and the other is not), extra
/// horizontal space is inserted using CSS-style **margin collapsing**:
/// `max(exit_margin, entry_margin)` rather than summing both margins.
///
/// This function modifies `x_coords` and `widths` in place and returns
/// the updated per-level total widths.
pub(crate) fn subgraph_padding<A: Axis>(
    dag: &Graph<'_>,
    virtual_levels: &[Vec<VNode>],
    x_coords: &mut [Vec<usize>],
    widths: &[Vec<usize>],
    node_spacing: usize,
) -> Vec<usize> {
    let mut level_widths = Vec::with_capacity(virtual_levels.len());

    for (lvl, vnodes) in virtual_levels.iter().enumerate() {
        if vnodes.is_empty() {
            level_widths.push(0);
            continue;
        }

        // Re-compute x positions with padding inserted at transitions.
        // The padding is proportional to the number of subgraph boundary
        // transitions (exits + entries) between adjacent nodes, matching
        // zigraph's `applySubgraphPadding`.
        let mut new_x = Vec::with_capacity(vnodes.len());
        let mut x = 0usize;

        // Left-side padding: one border's worth if inside a subgraph.
        // The bbox pass handles nesting expansion, so we only need the
        // immediate border's padding here — not the full ancestry chain.
        let first_sg = vnode_subgraph(dag, &vnodes[0]);
        if let Some(sg_id) = first_sg {
            x += A::SG_PAD_CROSS.0;
            // Non-merging profiles (Horizontal): each ANCESTOR box
            // needs its own label-side pad — coincident borders can't
            // merge when the pad carries the label row.
            if !A::NESTED_PADS_MERGE {
                x += ancestor_count(dag, sg_id) * A::PARENT_CHILD_PAD_CROSS.0;
            }
        }

        for (i, vnode) in vnodes.iter().enumerate() {
            if i > 0 {
                let prev_sg = vnode_subgraph(dag, &vnodes[i - 1]);
                let curr_sg = vnode_subgraph(dag, vnode);
                if prev_sg != curr_sg {
                    // Constant padding per boundary transition: one exit margin
                    // + one entry margin. The bbox pass handles depth-proportional
                    // expansion (merging profiles), so a fixed gap suffices there.
                    x += A::SG_PAD_CROSS.1 + A::SG_PAD_CROSS.0;
                    // Non-merging profiles reserve the full chains (may
                    // over-pad between siblings of one parent — safe,
                    // refined with LR tuning).
                    if !A::NESTED_PADS_MERGE {
                        if let Some(id) = prev_sg {
                            x += ancestor_count(dag, id) * A::PARENT_CHILD_PAD_CROSS.1;
                        }
                        if let Some(id) = curr_sg {
                            x += ancestor_count(dag, id) * A::PARENT_CHILD_PAD_CROSS.0;
                        }
                    }
                }
            }
            new_x.push(x);
            let w = widths[lvl].get(i).copied().unwrap_or(3);
            x += w + node_spacing;
        }

        // Right-side padding: one border's worth if last node is inside a subgraph.
        let last_sg = vnode_subgraph(dag, vnodes.last().unwrap());
        let right_extra = match last_sg {
            Some(sg_id) if !A::NESTED_PADS_MERGE => {
                A::SG_PAD_CROSS.1 + ancestor_count(dag, sg_id) * A::PARENT_CHILD_PAD_CROSS.1
            }
            Some(_) => A::SG_PAD_CROSS.1,
            None => 0,
        };

        let total = new_x
            .iter()
            .zip(widths[lvl].iter())
            .map(|(px, pw)| px + pw)
            .max()
            .unwrap_or(0)
            + right_extra;
        level_widths.push(total);

        x_coords[lvl] = new_x;
    }

    level_widths
}

// ── Cluster-width feedback ───────────────────────────────────────────────

/// Reclaim horizontal slack on each level (post-shift tightening).
///
/// Sibling-overlap repair moves whole clusters right, which can leave a
/// node far from its connected neighbors (e.g. a hole where a sibling's
/// member used to sit before its cluster was shifted away). This pass
/// sweeps each level in x order and moves every real node toward the
/// median center of its connected neighbors, strictly bounded by its
/// current level neighbors — so it can never widen a level, and the
/// rightmost node may only move left. Runs a few sweeps; each move is
/// monotone toward the target, so it settles quickly.
pub(crate) fn tighten_levels<A: Axis>(
    dag: &Graph<'_>,
    real_node_coords: &mut [(usize, usize, usize, usize)], // (level, pos, x, width)
    node_spacing: usize,
) {
    let n_nodes = real_node_coords.len();
    if n_nodes < 2 {
        return;
    }

    // Undirected adjacency over real nodes (edge endpoints).
    let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); n_nodes];
    for &(from_id, to_id, _) in &dag.edges {
        if let (Some(f), Some(t)) = (dag.node_index(from_id), dag.node_index(to_id)) {
            if f != t && f < n_nodes && t < n_nodes {
                adjacency[f].push(t);
                adjacency[t].push(f);
            }
        }
    }

    // Immediate subgraph per node, for gap selection.
    let node_sg: Vec<Option<usize>> = dag
        .nodes
        .iter()
        .map(|&(nid, _)| dag.node_subgraph.get(&nid).copied())
        .collect();

    // Group node indices by level.
    let max_level = real_node_coords.iter().map(|c| c.0).max().unwrap_or(0);
    let mut levels: Vec<Vec<usize>> = vec![Vec::new(); max_level + 1];
    for (ni, &(lvl, _, _, _)) in real_node_coords.iter().enumerate() {
        levels[lvl].push(ni);
    }

    for _sweep in 0..4 {
        let mut moved = false;

        // Snapshot each immediate cluster's member extent (min x, max right)
        // across all levels. Members may only move within it, so no cluster
        // bounding box can grow — growth would re-overlap sibling boxes that
        // fix_subgraph_overlaps just separated.
        let mut extents: HashMap<usize, (usize, usize)> = HashMap::new();
        for (ni, &(_, _, x, w)) in real_node_coords.iter().enumerate() {
            if let Some(sg) = node_sg[ni] {
                let e = extents.entry(sg).or_insert((usize::MAX, 0));
                e.0 = e.0.min(x);
                e.1 = e.1.max(x + w);
            }
        }

        for level_nodes in levels.iter_mut() {
            let n = level_nodes.len();
            if n == 0 {
                continue;
            }
            level_nodes.sort_by_key(|&ni| (real_node_coords[ni].2, ni));

            for k in 0..n {
                let ni = level_nodes[k];
                let (_, _, x, w) = real_node_coords[ni];

                // Median center of connected neighbors.
                let mut centers: Vec<usize> = adjacency[ni]
                    .iter()
                    .map(|&nb| real_node_coords[nb].2 + real_node_coords[nb].3 / 2)
                    .collect();
                if centers.is_empty() {
                    continue;
                }
                centers.sort_unstable();
                let target_center = centers[centers.len() / 2];
                let desired = target_center.saturating_sub(w / 2);

                let mut min_x = if k == 0 {
                    if node_sg[ni].is_some() {
                        leading_cross_pad::<A>(dag, node_sg[ni])
                    } else {
                        0
                    }
                } else {
                    let prev = level_nodes[k - 1];
                    let gap = if node_sg[prev] != node_sg[ni]
                        && (node_sg[prev].is_some() || node_sg[ni].is_some())
                    {
                        A::SG_GAP_CROSS
                    } else {
                        node_spacing
                    };
                    real_node_coords[prev].2 + real_node_coords[prev].3 + gap
                };
                let mut max_x = if k + 1 < n {
                    let next = level_nodes[k + 1];
                    let gap = if node_sg[next] != node_sg[ni]
                        && (node_sg[next].is_some() || node_sg[ni].is_some())
                    {
                        A::SG_GAP_CROSS
                    } else {
                        node_spacing
                    };
                    real_node_coords[next].2.saturating_sub(gap + w)
                } else {
                    // Rightmost node: never move right (keeps canvas bounded).
                    x
                };
                if let Some((ext_lo, ext_hi)) = node_sg[ni].and_then(|sg| extents.get(&sg)) {
                    min_x = min_x.max(*ext_lo);
                    max_x = max_x.min(ext_hi.saturating_sub(w));
                }
                if max_x < min_x {
                    continue;
                }
                let new_x = desired.clamp(min_x, max_x);
                if new_x != x {
                    real_node_coords[ni].2 = new_x;
                    moved = true;
                }
            }
        }
        if !moved {
            break;
        }
    }
}

/// Push unaffiliated nodes clear of subgraph bounding-box envelopes
/// (cluster-width feedback).
///
/// [`subgraph_padding`] reserves space per level, but the border later
/// drawn from [`compute_bounding_boxes`] is a *global* x-envelope: the
/// member extent across all levels, padded, widened to fit the label,
/// and expanded around children. Without feedback, an external node on
/// a narrower level (or past a label-widened edge) renders *inside* the
/// cluster border. This pass projects each cluster's final x-envelope
/// with the same math and pushes overlapping external nodes to its
/// right, iterating (bounded rounds) since a sweep can grow another
/// cluster's envelope.
///
/// It runs on `real_node_coords` **after** [`fix_subgraph_overlaps`] so
/// it sees the same coordinates the bounding boxes are computed from —
/// running earlier would clear against envelopes that sibling-overlap
/// repair later shifts.
///
/// Only **unaffiliated real nodes** are pushed:
/// - Members of *other* clusters are left to [`fix_subgraph_overlaps`],
///   which moves whole clusters. Pushing them individually here would
///   stretch their cluster's envelope and cascade the widening.
/// - Dummy vnodes are never pushed — an edge crossing a border renders
///   with junction glyphs, and rerouting them costs unbounded width.
///
/// Returns the growth of the maximum node right edge (0 if nothing
/// moved), which the caller folds into the canvas width.
pub(crate) fn clear_external_overlaps<A: Axis>(
    dag: &Graph<'_>,
    real_node_coords: &mut [(usize, usize, usize, usize)], // (level, pos, x, width)
    node_spacing: usize,
) -> usize {
    let sg_count = dag.subgraphs.len();
    if sg_count == 0 || real_node_coords.is_empty() {
        return 0;
    }

    let sg_id_to_idx: HashMap<usize, usize> = dag
        .subgraphs
        .iter()
        .enumerate()
        .map(|(i, sg)| (sg.id, i))
        .collect();
    let parent_idx: Vec<Option<usize>> = dag
        .subgraphs
        .iter()
        .map(|sg| sg.parent_id.and_then(|pid| sg_id_to_idx.get(&pid).copied()))
        .collect();

    // Deepest-first order for child → parent envelope propagation.
    let mut depths = vec![0usize; sg_count];
    for i in 0..sg_count {
        let mut d = 0;
        let mut cur = parent_idx[i];
        while let Some(p) = cur {
            d += 1;
            cur = parent_idx[p];
        }
        depths[i] = d;
    }
    let mut order: Vec<usize> = (0..sg_count).collect();
    order.sort_by(|a, b| depths[*b].cmp(&depths[*a]));

    // Immediate subgraph index per node (None = unaffiliated).
    let node_sg: Vec<Option<usize>> = dag
        .nodes
        .iter()
        .map(|&(nid, _)| {
            dag.node_subgraph
                .get(&nid)
                .and_then(|sid| sg_id_to_idx.get(sid).copied())
        })
        .collect();

    let max_level = real_node_coords.iter().map(|c| c.0).max().unwrap_or(0);
    let before_max_right = real_node_coords
        .iter()
        .map(|c| c.2 + c.3)
        .max()
        .unwrap_or(0);

    // Same cross-boundary gap the refinement passes use.

    for _round in 0..8 {
        let (bbox, range) =
            project_envelopes::<A>(dag, real_node_coords, &node_sg, &parent_idx, &order);

        // ── Push overlapping unaffiliated nodes right of each envelope ──
        let mut moved = false;
        let mut touched = vec![false; max_level + 1];
        for si in 0..sg_count {
            let Some((left, right)) = bbox[si] else {
                continue;
            };
            let (first, last) = range[si];
            if first == usize::MAX {
                continue;
            }
            let mut cursors = vec![0usize; max_level + 1];
            for c in cursors[first..=last.min(max_level)].iter_mut() {
                *c = right + A::ENVELOPE_CLEARANCE_CROSS;
            }
            for node_idx in 0..real_node_coords.len() {
                if node_sg.get(node_idx).copied().flatten().is_some() {
                    continue;
                }
                let (lvl, _, x, w) = real_node_coords[node_idx];
                if lvl < first || lvl > last {
                    continue;
                }
                if x < right && x + w > left {
                    real_node_coords[node_idx].2 = cursors[lvl];
                    cursors[lvl] += w + node_spacing;
                    moved = true;
                    touched[lvl] = true;
                }
            }
        }
        if !moved {
            break;
        }

        // ── Re-establish min gaps on touched levels (push-right, x order) ──
        for (lvl, lvl_touched) in touched.iter().enumerate() {
            if !lvl_touched {
                continue;
            }
            let mut level_nodes: Vec<usize> = (0..real_node_coords.len())
                .filter(|&ni| real_node_coords[ni].0 == lvl)
                .collect();
            level_nodes.sort_by_key(|&ni| (real_node_coords[ni].2, ni));
            for k in 1..level_nodes.len() {
                let prev = level_nodes[k - 1];
                let cur = level_nodes[k];
                let prev_sg = node_sg[prev];
                let cur_sg = node_sg[cur];
                let gap = if prev_sg != cur_sg && (prev_sg.is_some() || cur_sg.is_some()) {
                    A::SG_GAP_CROSS
                } else {
                    node_spacing
                };
                let min_x = real_node_coords[prev].2 + real_node_coords[prev].3 + gap;
                if real_node_coords[cur].2 < min_x {
                    real_node_coords[cur].2 = min_x;
                }
            }
        }
    }

    let after_max_right = real_node_coords
        .iter()
        .map(|c| c.2 + c.3)
        .max()
        .unwrap_or(0);
    after_max_right.saturating_sub(before_max_right)
}

/// Per-cluster projected x-envelope: `None` if the cluster has no member
/// nodes, else `(left, right)` including padding and label minimum.
type Envelopes = Vec<Option<(usize, usize)>>;
/// Per-cluster `(first_level, last_level)` range (self + descendants).
type LevelRanges = Vec<(usize, usize)>;

/// Project per-cluster x-envelopes and level ranges from current node
/// coordinates, mirroring [`compute_bounding_boxes`] x-math: member
/// extent, `A::SG_PAD_CROSS`, label minimum width, child → parent
/// expansion, label recheck. `order` must be deepest-first.
fn project_envelopes<A: Axis>(
    dag: &Graph<'_>,
    real_node_coords: &[(usize, usize, usize, usize)],
    node_sg: &[Option<usize>],
    parent_idx: &[Option<usize>],
    order: &[usize],
) -> (Envelopes, LevelRanges) {
    let sg_count = parent_idx.len();
    let mut bbox: Vec<Option<(usize, usize)>> = vec![None; sg_count];
    let mut range: Vec<(usize, usize)> = vec![(usize::MAX, 0); sg_count];

    for (node_idx, &(lvl, _, x, w)) in real_node_coords.iter().enumerate() {
        if let Some(si) = node_sg.get(node_idx).copied().flatten() {
            let r = x + w;
            bbox[si] = Some(match bbox[si] {
                None => (x, r),
                Some((l, rr)) => (l.min(x), rr.max(r)),
            });
            let mut cur = Some(si);
            while let Some(i) = cur {
                range[i].0 = range[i].0.min(lvl);
                range[i].1 = range[i].1.max(lvl);
                cur = parent_idx[i];
            }
        }
    }

    // Pad + label minimum (mirrors compute_bounding_boxes pass 1.5).
    // The fold is axis-routed (D8): `label_cross_extent` is 0 under
    // Horizontal, whose label claim lands on the level axis instead
    // (`label_level_extent` + the label-extras phase).
    for (si, b) in bbox.iter_mut().enumerate() {
        if let Some((l, r)) = *b {
            let left = l.saturating_sub(A::SG_PAD_CROSS.0);
            let mut right = r + A::SG_PAD_CROSS.1;
            let min_label_width = A::label_cross_extent(dag.subgraphs[si].label);
            if right - left < min_label_width {
                right = left + min_label_width;
            }
            *b = Some((left, right));
        }
    }

    // Child → parent expansion, then label recheck (mirrors pass 2).
    for &si in order {
        if let (Some((cl, cr)), Some(pi)) = (bbox[si], parent_idx[si]) {
            let exp = (
                cl.saturating_sub(A::PARENT_CHILD_PAD_CROSS.0),
                cr + A::PARENT_CHILD_PAD_CROSS.1,
            );
            bbox[pi] = Some(match bbox[pi] {
                None => exp,
                Some((pl, pr)) => (pl.min(exp.0), pr.max(exp.1)),
            });
        }
    }
    for (si, b) in bbox.iter_mut().enumerate() {
        if let Some((l, r)) = *b {
            let min_label_width = A::label_cross_extent(dag.subgraphs[si].label);
            if r - l < min_label_width {
                *b = Some((l, l + min_label_width));
            }
        }
    }
    (bbox, range)
}

/// Compact root clusters and unaffiliated nodes leftward.
///
/// [`fix_subgraph_overlaps`] separates overlapping clusters by shifting
/// the right one further right, and nothing pulls clusters back together
/// afterward — leaving wide empty gulfs crossed by long edge lines. This
/// pass treats each **root** cluster as a rigid body (its internal layout
/// and nested children move as one) and each unaffiliated node as a
/// singleton body, sweeps bodies in left-to-right order, and shifts each
/// as far left as the per-level frontier of already-placed bodies allows:
/// envelope↔envelope keeps [`Axis::SIBLING_GAP_CROSS`], envelope↔node
/// keeps [`Axis::ENVELOPE_CLEARANCE_CROSS`], node↔node keeps
/// `node_spacing`. Shift-left
/// only, so the canvas can only shrink and no constraint that held
/// before can break.
///
/// Returns the reclaimed canvas width: the reduction of the rightmost
/// extent, conservatively the smaller of the node-extent and
/// envelope-extent reductions.
pub(crate) fn compact_clusters<A: Axis>(
    dag: &Graph<'_>,
    real_node_coords: &mut [(usize, usize, usize, usize)], // (level, pos, x, width)
    virtual_levels: &[Vec<VNode>],
    x_coords: &mut [Vec<usize>],
    node_spacing: usize,
) -> usize {
    let sg_count = dag.subgraphs.len();
    if sg_count == 0 || real_node_coords.is_empty() {
        return 0;
    }

    let sg_id_to_idx: HashMap<usize, usize> = dag
        .subgraphs
        .iter()
        .enumerate()
        .map(|(i, sg)| (sg.id, i))
        .collect();
    let parent_idx: Vec<Option<usize>> = dag
        .subgraphs
        .iter()
        .map(|sg| sg.parent_id.and_then(|pid| sg_id_to_idx.get(&pid).copied()))
        .collect();
    let mut depths = vec![0usize; sg_count];
    for i in 0..sg_count {
        let mut d = 0;
        let mut cur = parent_idx[i];
        while let Some(p) = cur {
            d += 1;
            cur = parent_idx[p];
        }
        depths[i] = d;
    }
    let mut order: Vec<usize> = (0..sg_count).collect();
    order.sort_by(|a, b| depths[*b].cmp(&depths[*a]));
    let node_sg: Vec<Option<usize>> = dag
        .nodes
        .iter()
        .map(|&(nid, _)| {
            dag.node_subgraph
                .get(&nid)
                .and_then(|sid| sg_id_to_idx.get(sid).copied())
        })
        .collect();
    let root_of = |mut i: usize| -> usize {
        while let Some(p) = parent_idx[i] {
            i = p;
        }
        i
    };

    let (bbox, range) =
        project_envelopes::<A>(dag, real_node_coords, &node_sg, &parent_idx, &order);
    let max_level = real_node_coords.iter().map(|c| c.0).max().unwrap_or(0);
    let before_node_right = real_node_coords
        .iter()
        .map(|c| c.2 + c.3)
        .max()
        .unwrap_or(0);
    let before_env_right = bbox.iter().flatten().map(|b| b.1).max().unwrap_or(0);

    // Bodies in left-to-right order: root clusters, unaffiliated real
    // nodes, and unaffiliated dummy waypoints. Dummies MUST participate —
    // leaving them out lets a cluster slide left over an edge chain, which
    // then renders running along (or on top of) the cluster's border.
    // (left, right, tag, a, b): tag 0 = cluster(a=sg idx), 1 = node(a=node
    // idx), 2 = dummy(a=level, b=pos).
    let mut bodies: Vec<(usize, usize, u8, usize, usize)> = Vec::new();
    for si in 0..sg_count {
        if parent_idx[si].is_none() {
            if let Some((l, r)) = bbox[si] {
                bodies.push((l, r, 0, si, 0));
            }
        }
    }
    for (ni, &(_, _, x, w)) in real_node_coords.iter().enumerate() {
        if node_sg[ni].is_none() {
            bodies.push((x, x + w, 1, ni, 0));
        }
    }
    for (lvl, vnodes) in virtual_levels.iter().enumerate() {
        for (pos, vnode) in vnodes.iter().enumerate() {
            if matches!(vnode, VNode::Dummy { .. }) && vnode_subgraph(dag, vnode).is_none() {
                let x = x_coords[lvl][pos];
                bodies.push((x, x + A::DUMMY_CROSS, 2, lvl, pos));
            }
        }
    }
    bodies.sort_unstable();

    // Per-level frontiers of already-placed bodies.
    let mut env_right: Vec<Option<usize>> = vec![None; max_level + 1];
    let mut node_right: Vec<Option<usize>> = vec![None; max_level + 1];

    for &(env_left, env_r, tag, a, b) in &bodies {
        match tag {
            0 => {
                let (first, last) = range[a];
                if first == usize::MAX {
                    continue;
                }
                let mut allowed = 0usize;
                for lvl in first..=last.min(max_level) {
                    if let Some(er) = env_right[lvl] {
                        allowed = allowed.max(er + A::SIBLING_GAP_CROSS);
                    }
                    if let Some(nr) = node_right[lvl] {
                        allowed = allowed.max(nr + A::ENVELOPE_CLEARANCE_CROSS);
                    }
                }
                let delta = env_left.saturating_sub(allowed);
                if delta > 0 {
                    for (ni, coords) in real_node_coords.iter_mut().enumerate() {
                        if let Some(si) = node_sg[ni] {
                            if root_of(si) == a {
                                coords.2 -= delta;
                            }
                        }
                    }
                    // Member dummies (both edge endpoints inside this
                    // cluster) are part of the rigid body too.
                    for (lvl, vnodes) in virtual_levels.iter().enumerate() {
                        for (pos, vnode) in vnodes.iter().enumerate() {
                            if !matches!(vnode, VNode::Dummy { .. }) {
                                continue;
                            }
                            let member = vnode_subgraph(dag, vnode)
                                .and_then(|sid| sg_id_to_idx.get(&sid).copied())
                                .is_some_and(|si| root_of(si) == a);
                            if member {
                                x_coords[lvl][pos] = x_coords[lvl][pos].saturating_sub(delta);
                            }
                        }
                    }
                }
                let new_right = env_r - delta;
                for lvl in first..=last.min(max_level) {
                    env_right[lvl] = Some(env_right[lvl].map_or(new_right, |e| e.max(new_right)));
                }
            }
            1 => {
                let (lvl, _, x, w) = real_node_coords[a];
                let mut allowed = 0usize;
                if let Some(er) = env_right[lvl] {
                    allowed = allowed.max(er + A::ENVELOPE_CLEARANCE_CROSS);
                }
                if let Some(nr) = node_right[lvl] {
                    allowed = allowed.max(nr + node_spacing);
                }
                let delta = x.saturating_sub(allowed);
                if delta > 0 {
                    real_node_coords[a].2 = x - delta;
                }
                node_right[lvl] =
                    Some(node_right[lvl].map_or(x - delta + w, |e| e.max(x - delta + w)));
            }
            _ => {
                let (lvl, pos) = (a, b);
                let x = x_coords[lvl][pos];
                let mut allowed = 0usize;
                if let Some(er) = env_right[lvl] {
                    allowed = allowed.max(er + A::ENVELOPE_CLEARANCE_CROSS);
                }
                if let Some(nr) = node_right[lvl] {
                    allowed = allowed.max(nr + node_spacing);
                }
                let delta = x.saturating_sub(allowed);
                if delta > 0 {
                    x_coords[lvl][pos] = x - delta;
                }
                let right = x - delta + A::DUMMY_CROSS;
                node_right[lvl] = Some(node_right[lvl].map_or(right, |e| e.max(right)));
            }
        }
    }

    let (bbox_after, _) =
        project_envelopes::<A>(dag, real_node_coords, &node_sg, &parent_idx, &order);
    let after_node_right = real_node_coords
        .iter()
        .map(|c| c.2 + c.3)
        .max()
        .unwrap_or(0);
    let after_env_right = bbox_after.iter().flatten().map(|b| b.1).max().unwrap_or(0);
    let node_reclaim = before_node_right.saturating_sub(after_node_right);
    let env_reclaim = before_env_right.saturating_sub(after_env_right);
    node_reclaim.min(env_reclaim)
}

/// Nudge dummy waypoints out of real node spans.
///
/// The vertical segment of a skip-level edge is drawn at its waypoint
/// column, and that column crosses the node row of every level the edge
/// passes through. After the coordinate passes move real nodes (overlap
/// repair, tightening, compaction), a node can land on a stale waypoint
/// column — the edge then renders straight through the node text. This
/// pass moves any such waypoint to the nearest column outside the node's
/// span. An edge crossing a subgraph *border* renders with junction
/// glyphs and is acceptable; crossing a *node* never is, so only node
/// spans are avoided.
pub(crate) fn nudge_dummies_off_nodes<A: Axis>(
    virtual_levels: &[Vec<VNode>],
    x_coords: &mut [Vec<usize>],
    real_node_coords: &[(usize, usize, usize, usize)], // (level, pos, x, width)
) {
    let num_levels = virtual_levels.len();
    let mut spans: Vec<Vec<(usize, usize)>> = vec![Vec::new(); num_levels];
    for &(lvl, _, x, w) in real_node_coords {
        if lvl < num_levels {
            spans[lvl].push((x, x + w));
        }
    }
    for s in spans.iter_mut() {
        s.sort_unstable();
    }

    for (lvl, vnodes) in virtual_levels.iter().enumerate() {
        for (pos, vnode) in vnodes.iter().enumerate() {
            let VNode::Dummy { edge_idx } = vnode else {
                continue;
            };
            // The renderer draws this edge's flow segment at
            // x + dummy_draw_offset (axis-profiled).
            let off = A::dummy_draw_offset(*edge_idx);
            let Some(&x) = x_coords.get(lvl).and_then(|l| l.get(pos)) else {
                continue;
            };
            let mut col = x + off;
            for _ in 0..8 {
                let Some(&(sl, sr)) = spans[lvl].iter().find(|&&(sl, sr)| col >= sl && col < sr)
                else {
                    break;
                };
                // Exit on the nearer side; going left past column `off` is
                // impossible (the stored x is unsigned), so fall back right.
                let go_left = sl > off && (col - sl) < (sr - col);
                col = if go_left { sl - 1 } else { sr };
            }
            if col != x + off {
                x_coords[lvl][pos] = col - off;
            }
        }
    }
}

// ── Sibling subgraph overlap repair ──────────────────────────────────────

/// Collect all node indices belonging to a subgraph or any of its descendants.
fn collect_sg_node_indices(
    dag: &Graph<'_>,
    sg_idx: usize,
    _sg_id_to_idx: &HashMap<usize, usize>,
) -> Vec<usize> {
    let sg_id = dag.subgraphs[sg_idx].id;
    // Collect this subgraph's descendant IDs (BFS)
    let mut sg_ids = vec![sg_id];
    let mut i = 0;
    while i < sg_ids.len() {
        let pid = sg_ids[i];
        for sg in &dag.subgraphs {
            if sg.parent_id == Some(pid) {
                sg_ids.push(sg.id);
            }
        }
        i += 1;
    }

    let mut result = Vec::new();
    for (node_idx, &(nid, _)) in dag.nodes.iter().enumerate() {
        if let Some(&nsg_id) = dag.node_subgraph.get(&nid) {
            if sg_ids.contains(&nsg_id) {
                result.push(node_idx);
            }
        }
    }
    result
}

/// Detect and fix horizontal overlaps between sibling subgraph bounding boxes
/// by shifting `real_node_coords` for nodes inside overlapping subgraphs.
///
/// Uses a right-frontier sweep so that each sibling's shift accounts for
/// all prior shifts in the same parent group (prevents stale-bbox bugs).
///
/// Returns the extra width added (0 if no adjustment needed).
pub(crate) fn fix_subgraph_overlaps<A: Axis>(
    dag: &Graph<'_>,
    real_node_coords: &mut [(usize, usize, usize, usize)], // (level, pos, x, width)
) -> usize {
    let sg_count = dag.subgraphs.len();
    if sg_count < 2 {
        return 0;
    }

    let sg_id_to_idx: HashMap<usize, usize> = dag
        .subgraphs
        .iter()
        .enumerate()
        .map(|(i, sg)| (sg.id, i))
        .collect();

    // Compute nesting depth for each subgraph
    let mut depths: Vec<usize> = vec![0; sg_count];
    for (i, sg) in dag.subgraphs.iter().enumerate() {
        let mut d = 0;
        let mut cur = sg.parent_id;
        while let Some(pid) = cur {
            d += 1;
            cur = dag
                .subgraphs
                .iter()
                .find(|s| s.id == pid)
                .and_then(|s| s.parent_id);
        }
        depths[i] = d;
    }
    let max_depth = depths.iter().copied().max().unwrap_or(0);

    // Minimum gap between nodes of different subgraphs on the same level.
    // Each side contributes one H_PAD for its border, plus a sibling gap between borders.
    let cross_sg_gap: usize = A::SG_GAP_CROSS;

    // Build per-node → immediate subgraph-idx lookup (None if unaffiliated).
    let node_sg: Vec<Option<usize>> = dag
        .nodes
        .iter()
        .map(|&(nid, _)| {
            dag.node_subgraph
                .get(&nid)
                .and_then(|sid| sg_id_to_idx.get(sid).copied())
        })
        .collect();

    // Compute level range (min_level, max_level) per subgraph, including descendants.
    // Two subgraphs on disjoint level ranges can freely overlap in x.
    let mut sg_level_range: Vec<(usize, usize)> = vec![(usize::MAX, 0); sg_count];
    for (node_idx, _) in dag.nodes.iter().enumerate() {
        if let Some(sg_idx) = node_sg[node_idx] {
            if node_idx < real_node_coords.len() {
                let level = real_node_coords[node_idx].0;
                let (ref mut min_l, ref mut max_l) = sg_level_range[sg_idx];
                if level < *min_l {
                    *min_l = level;
                }
                if level > *max_l {
                    *max_l = level;
                }
            }
        }
    }
    // Propagate child level ranges to parents (bottom-up)
    for depth in (0..=max_depth).rev() {
        for sg_idx in 0..sg_count {
            if depths[sg_idx] != depth {
                continue;
            }
            if let Some(parent_id) = dag.subgraphs[sg_idx].parent_id {
                if let Some(&pidx) = sg_id_to_idx.get(&parent_id) {
                    let (cl, cr) = sg_level_range[sg_idx];
                    if cl == usize::MAX {
                        continue;
                    }
                    let (ref mut pl, ref mut pr) = sg_level_range[pidx];
                    if cl < *pl {
                        *pl = cl;
                    }
                    if cr > *pr {
                        *pr = cr;
                    }
                }
            }
        }
    }

    // Compute padded bounding box (left, right) per subgraph from real_node_coords.
    let compute_bboxes = |coords: &[(usize, usize, usize, usize)]| -> Vec<Option<(usize, usize)>> {
        let mut envs: Vec<Option<(usize, usize)>> = vec![None; sg_count];
        for (node_idx, _) in dag.nodes.iter().enumerate() {
            if let Some(sg_idx) = node_sg[node_idx] {
                if node_idx >= coords.len() {
                    continue;
                }
                let (_, _, x, width) = coords[node_idx];
                let right = x + width;
                envs[sg_idx] = Some(match envs[sg_idx] {
                    None => (x, right),
                    Some((mn, mx)) => (mn.min(x), mx.max(right)),
                });
            }
        }
        // Propagate children to parents (bottom-up)
        // Minimal gap: the child bbox already includes its own cross
        // pads, so the parent only needs its border column.
        for depth in (0..=max_depth).rev() {
            for sg_idx in 0..sg_count {
                if depths[sg_idx] != depth {
                    continue;
                }
                if let Some(parent_id) = dag.subgraphs[sg_idx].parent_id {
                    if let Some(&pidx) = sg_id_to_idx.get(&parent_id) {
                        if let Some((cx, cr)) = envs[sg_idx] {
                            let exp = (
                                cx.saturating_sub(A::PARENT_CHILD_PAD_CROSS.0),
                                cr + A::PARENT_CHILD_PAD_CROSS.1,
                            );
                            envs[pidx] = Some(match envs[pidx] {
                                None => exp,
                                Some((px, pr)) => (px.min(exp.0), pr.max(exp.1)),
                            });
                        }
                    }
                }
            }
        }
        envs.iter()
            .enumerate()
            .map(|(sg_idx, env)| {
                env.map(|(mn, mx)| {
                    let left = mn.saturating_sub(A::SG_PAD_CROSS.0);
                    let right = mx + A::SG_PAD_CROSS.1;
                    let label_w = A::label_cross_extent(dag.subgraphs[sg_idx].label);
                    let width = right.saturating_sub(left);
                    let right = if width < label_w {
                        left + label_w
                    } else {
                        right
                    };
                    (left, right)
                })
            })
            .collect()
    };

    let mut total_extra = 0usize;

    // Process overlap repair iteratively (up to 8 rounds for convergence).
    // Each round detects overlaps, shifts subgraph nodes, then repairs
    // per-level collisions.  Collision repair can widen bboxes by a few
    // chars, so subsequent rounds mop up any cascade.
    for _round in 0..8 {
        let bbox_x = compute_bboxes(real_node_coords);

        // Group siblings by parent, detect overlaps
        let mut parent_groups: HashMap<Option<usize>, Vec<usize>> = HashMap::new();
        for (sg_idx, sg) in dag.subgraphs.iter().enumerate() {
            if bbox_x[sg_idx].is_some() {
                parent_groups.entry(sg.parent_id).or_default().push(sg_idx);
            }
        }

        let mut any_shifted = false;

        for siblings in parent_groups.values_mut() {
            if siblings.len() < 2 {
                continue;
            }
            siblings.sort_by_key(|&idx| bbox_x[idx].map(|(l, _)| l).unwrap_or(0));

            // Level-aware pairwise sweep: only enforce separation between
            // siblings whose rendered level ranges share at least one level.
            // Subgraphs on disjoint levels (e.g. Frontend 2-3, Backend 4-8)
            // can freely overlap in x — edge routing rows between levels
            // provide enough vertical space for both subgraph borders.
            let mut processed: Vec<(usize, usize, usize, usize)> = Vec::new(); // (sg_idx, eff_right, min_l, max_l)

            for &sg_idx in siblings.iter() {
                if let Some((left, right)) = bbox_x[sg_idx] {
                    let (cur_min_l, cur_max_l) = sg_level_range[sg_idx];

                    // Effective frontier: max right edge among processed
                    // siblings that share at least one level.
                    let mut eff_frontier = 0usize;
                    let mut has_level_overlap = false;
                    for &(_, prev_right, prev_min_l, prev_max_l) in &processed {
                        let overlaps = prev_min_l <= cur_max_l && cur_min_l <= prev_max_l;
                        if overlaps && prev_right > eff_frontier {
                            eff_frontier = prev_right;
                            has_level_overlap = true;
                        }
                    }

                    if has_level_overlap && eff_frontier + A::SIBLING_GAP_CROSS > left {
                        let shift = eff_frontier + A::SIBLING_GAP_CROSS - left;

                        let node_indices = collect_sg_node_indices(dag, sg_idx, &sg_id_to_idx);
                        for &ni in &node_indices {
                            if ni < real_node_coords.len() {
                                real_node_coords[ni].2 += shift;
                            }
                        }
                        total_extra += shift;
                        any_shifted = true;

                        processed.push((sg_idx, right + shift, cur_min_l, cur_max_l));
                    } else {
                        processed.push((sg_idx, right, cur_min_l, cur_max_l));
                    }
                }
            }
        }

        if !any_shifted {
            break;
        }

        // After shifting subgraph nodes, fix per-level collisions.
        // Use a larger gap for nodes that belong to different subgraphs
        // so that resulting bboxes (which add H_PAD on each side) don't
        // re-overlap and trigger another round.
        let max_level = real_node_coords.iter().map(|c| c.0).max().unwrap_or(0);
        for level in 0..=max_level {
            let mut level_nodes: Vec<usize> = Vec::new();
            for (ni, &(lvl, _, _, _)) in real_node_coords.iter().enumerate() {
                if lvl == level {
                    level_nodes.push(ni);
                }
            }
            level_nodes.sort_by_key(|&ni| real_node_coords[ni].2);

            for j in 1..level_nodes.len() {
                let prev = level_nodes[j - 1];
                let curr = level_nodes[j];
                let need_sg_gap = match (node_sg[prev], node_sg[curr]) {
                    (Some(a), Some(b)) => a != b,
                    _ => false,
                };
                let gap = if need_sg_gap { cross_sg_gap } else { 3 };
                let prev_right = real_node_coords[prev].2 + real_node_coords[prev].3 + gap;
                if real_node_coords[curr].2 < prev_right {
                    real_node_coords[curr].2 = prev_right;
                }
            }
        }
    }

    total_extra
}

// ── Bounding box computation ─────────────────────────────────────────────

/// Per-box `(first_level, last_level)` from member-node levels, with
/// child ranges propagated to parents (a parent's border encloses its
/// descendants). `None` for boxes without nodes.
fn sg_level_ranges(dag: &Graph<'_>, node_levels: &[usize]) -> Vec<Option<(usize, usize)>> {
    let mut sg_ranges: Vec<Option<(usize, usize)>> = Vec::with_capacity(dag.subgraphs.len());

    for sg in &dag.subgraphs {
        let mut first = usize::MAX;
        let mut last = 0usize;
        let mut has_nodes = false;

        for (node_idx, &(id, _)) in dag.nodes.iter().enumerate() {
            if dag.node_subgraph.get(&id).copied() == Some(sg.id) {
                let lvl = node_levels[node_idx];
                first = first.min(lvl);
                last = last.max(lvl);
                has_nodes = true;
            }
        }

        if has_nodes {
            sg_ranges.push(Some((first, last)));
        } else {
            sg_ranges.push(None);
        }
    }

    let mut changed = true;
    while changed {
        changed = false;
        for i in 0..dag.subgraphs.len() {
            if let (Some((cf, cl)), Some(parent_id)) = (sg_ranges[i], dag.subgraphs[i].parent_id) {
                if let Some(pi) = dag.subgraphs.iter().position(|s| s.id == parent_id) {
                    let was_none = sg_ranges[pi].is_none();
                    let (pf, pl) = sg_ranges[pi].unwrap_or((cf, cl));
                    let new_pf = pf.min(cf);
                    let new_pl = pl.max(cl);
                    if was_none || new_pf != pf || new_pl != pl {
                        sg_ranges[pi] = Some((new_pf, new_pl));
                        changed = true;
                    }
                }
            }
        }
    }
    sg_ranges
}

/// D8(b): per-level LEVEL-axis reservations for box labels
/// (Horizontal-only — `label_level_extent` is 0 under Vertical, and
/// the caller skips the offset rebuild when every entry is 0).
///
/// A box whose level span cannot fit its label gets the deficit
/// reserved as extra trailing pad at its CLOSING level, so the
/// label-widened bbox cannot overlap the next column. The box extent
/// estimate is the level-band span including one border pad per side;
/// the P1-S4 invariant suite judges its adequacy for nested shapes.
pub(crate) fn compute_label_level_extras<A: Axis>(
    dag: &Graph<'_>,
    node_levels: &[usize],
    level_offsets: &[usize],
    level_extents: &[usize],
    max_level: usize,
) -> Vec<usize> {
    let mut extras = vec![0usize; max_level + 1];
    if dag.subgraphs.is_empty() {
        return extras;
    }
    let ranges = sg_level_ranges(dag, node_levels);
    for (si, range) in ranges.iter().enumerate() {
        let Some((first, last)) = *range else {
            continue;
        };
        let need = A::label_level_extent(dag.subgraphs[si].label);
        if need == 0 || last > max_level {
            continue;
        }
        let start = level_offsets[first].saturating_sub(A::SG_PAD_LEVEL.0);
        let end = level_offsets[last] + level_extents[last] + A::SG_PAD_LEVEL.1;
        let deficit = need.saturating_sub(end.saturating_sub(start));
        if deficit > 0 {
            extras[last] = extras[last].max(deficit);
        }
    }
    extras
}

/// Compute extra vertical rows needed at each level boundary for subgraph
/// borders that open or close.
///
/// For each boundary between level *L* and *L+1*, we check which subgraphs
/// have their **last** member node at level *L* (→ border closes) and which
/// have their **first** member node at *L+1* (→ border opens).  The extra
/// space is:
///
/// ```text
/// max_close_depth × V_PAD_BOTTOM + max_open_depth × V_PAD_TOP
/// ```
///
/// This ensures vertical room for the closing and opening border rows,
/// matching zigraph's `computeLevelYOffsets`.
///
/// Returns `(initial_offset, per_boundary_extra, trailing_extra)`:
///
/// - `initial_offset` — extra rows before level 0 (for subgraphs opening
///   there).
/// - `per_boundary_extra[L]` — extra rows to insert *after* level *L*'s
///   base height (for `L = 0..max_level`).
/// - `trailing_extra` — extra rows after the last level (for subgraphs
///   closing there).
pub(crate) fn compute_level_extras<A: Axis>(
    dag: &Graph<'_>,
    node_levels: &[usize],
    max_level: usize,
) -> (usize, Vec<usize>, usize) {
    if dag.subgraphs.is_empty() || max_level == 0 {
        return (0, vec![0; max_level + 1], 0);
    }

    let sg_ranges = sg_level_ranges(dag, node_levels);

    /// Count how many borders stack at a boundary for a given subgraph.
    ///
    /// For closing: count S + ancestors of S that also close at the same boundary.
    /// For opening: count S + ancestors of S that also open at the same boundary.
    fn stacked_borders_closing(
        dag: &Graph<'_>,
        sg_idx: usize,
        boundary_level: usize,
        sg_ranges: &[Option<(usize, usize)>],
    ) -> usize {
        let mut count = 1; // the subgraph itself
        let mut cur = dag.subgraphs[sg_idx].parent_id;
        while let Some(pid) = cur {
            if let Some(pi) = dag.subgraphs.iter().position(|s| s.id == pid) {
                if let Some((_, last)) = sg_ranges[pi] {
                    if last == boundary_level {
                        count += 1;
                        cur = dag.subgraphs[pi].parent_id;
                        continue;
                    }
                }
            }
            break;
        }
        count
    }

    fn stacked_borders_opening(
        dag: &Graph<'_>,
        sg_idx: usize,
        boundary_level: usize,
        sg_ranges: &[Option<(usize, usize)>],
    ) -> usize {
        let mut count = 1;
        let mut cur = dag.subgraphs[sg_idx].parent_id;
        while let Some(pid) = cur {
            if let Some(pi) = dag.subgraphs.iter().position(|s| s.id == pid) {
                if let Some((first, _)) = sg_ranges[pi] {
                    if first == boundary_level {
                        count += 1;
                        cur = dag.subgraphs[pi].parent_id;
                        continue;
                    }
                }
            }
            break;
        }
        count
    }

    // Initial offset: max stacked opening borders at level 0
    let initial_open_depth = dag
        .subgraphs
        .iter()
        .enumerate()
        .filter_map(|(i, _)| sg_ranges[i].as_ref().map(|r| (i, r)))
        .filter(|(_, (first, _))| *first == 0)
        .map(|(i, _)| stacked_borders_opening(dag, i, 0, &sg_ranges))
        .max()
        .unwrap_or(0);
    let initial_offset = initial_open_depth * A::SG_PAD_LEVEL.0;

    // Per-boundary extras
    let mut extras = vec![0usize; max_level + 1];

    for boundary_after in 0..max_level {
        let next_level = boundary_after + 1;

        // Max stacked closing borders at boundary_after
        let close_depth = dag
            .subgraphs
            .iter()
            .enumerate()
            .filter_map(|(i, _)| sg_ranges[i].as_ref().map(|r| (i, r)))
            .filter(|(_, (_, last))| *last == boundary_after)
            .map(|(i, _)| stacked_borders_closing(dag, i, boundary_after, &sg_ranges))
            .max()
            .unwrap_or(0);

        // Max stacked opening borders at next_level
        let open_depth = dag
            .subgraphs
            .iter()
            .enumerate()
            .filter_map(|(i, _)| sg_ranges[i].as_ref().map(|r| (i, r)))
            .filter(|(_, (first, _))| *first == next_level)
            .map(|(i, _)| stacked_borders_opening(dag, i, next_level, &sg_ranges))
            .max()
            .unwrap_or(0);

        extras[boundary_after] = close_depth * A::SG_PAD_LEVEL.1 + open_depth * A::SG_PAD_LEVEL.0;
    }

    // Trailing extra: space for subgraphs whose last member is at max_level
    let trailing_close_depth = dag
        .subgraphs
        .iter()
        .enumerate()
        .filter_map(|(i, _)| sg_ranges[i].as_ref().map(|r| (i, r)))
        .filter(|(_, (_, last))| *last == max_level)
        .map(|(i, _)| stacked_borders_closing(dag, i, max_level, &sg_ranges))
        .max()
        .unwrap_or(0);
    let trailing_extra = trailing_close_depth * A::SG_PAD_LEVEL.1;

    (initial_offset, extras, trailing_extra)
}

/// Compute bounding boxes for all subgraphs based on their member nodes'
/// final coordinates.
///
/// **Algorithm** (zigraph parity):
///
/// 1. **Pass 1 — node envelope:** For each subgraph, compute the min/max
///    x and y across all member nodes.
///
/// 2. **Pass 2 — bottom-up propagation:** Process subgraphs from deepest
///    nesting first.  Expand each parent's envelope to contain its children's
///    bounding boxes (including padding and label rows).
///
/// Returns a `Vec<SubgraphInfo>` ready to be added to the IR builder.
pub(crate) fn compute_bounding_boxes<'a, A: Axis>(
    dag: &Graph<'a>,
    real_node_coords: &[(usize, usize, usize, usize)], // (level, pos, x, width) per node_idx
    level_offsets: &[usize],
    total_height: usize,
    edge_routing_ys: &HashSet<usize>,
    level_routing_floor: &[usize],
) -> Vec<SubgraphInfo<'a>> {
    if dag.subgraphs.is_empty() {
        return Vec::new();
    }

    let sg_count = dag.subgraphs.len();

    // Build sg_id → index mapping
    let sg_id_to_idx: HashMap<usize, usize> = dag
        .subgraphs
        .iter()
        .enumerate()
        .map(|(i, sg)| (sg.id, i))
        .collect();

    // Pass 1: compute envelope from member nodes
    // (min_x, min_y, max_x_plus_w, max_y)
    let mut envelopes: Vec<Option<(usize, usize, usize, usize)>> = vec![None; sg_count];
    // Track the max level of each subgraph's member nodes (for routing floor lookup)
    let mut sg_max_level: Vec<usize> = vec![0; sg_count];

    for (node_idx, &(id, _label)) in dag.nodes.iter().enumerate() {
        if let Some(&sg_id) = dag.node_subgraph.get(&id) {
            if let Some(&sg_idx) = sg_id_to_idx.get(&sg_id) {
                let (level, _pos, x, width) = real_node_coords[node_idx];
                let y = if level < level_offsets.len() {
                    level_offsets[level]
                } else {
                    0
                };
                // Member LEVEL extent from the declared dimensions —
                // "one line" was a masked assumption (multi-row members
                // under Vertical, any wide member under Horizontal).
                let node_max_y = y + A::level_extent(
                    dag.get_node_width(node_idx),
                    dag.get_node_height(node_idx),
                );
                let node_max_x = x + width;

                sg_max_level[sg_idx] = sg_max_level[sg_idx].max(level);

                envelopes[sg_idx] = Some(match envelopes[sg_idx] {
                    None => (x, y, node_max_x, node_max_y),
                    Some((min_x, min_y, max_x, max_y)) => (
                        min_x.min(x),
                        min_y.min(y),
                        max_x.max(node_max_x),
                        max_y.max(node_max_y),
                    ),
                });
            }
        }
    }

    // Compute nesting depth for bottom-up ordering
    let mut depths: Vec<usize> = vec![0; sg_count];
    for (i, sg) in dag.subgraphs.iter().enumerate() {
        let mut depth = 0;
        let mut current = sg.parent_id;
        while let Some(pid) = current {
            depth += 1;
            current = dag
                .subgraphs
                .iter()
                .find(|s| s.id == pid)
                .and_then(|s| s.parent_id);
        }
        depths[i] = depth;
    }

    // Process bottom-up: deepest-nested subgraphs first
    let mut order: Vec<usize> = (0..sg_count).collect();
    order.sort_by(|a, b| depths[*b].cmp(&depths[*a]));

    // Pass 1.5: Convert raw node envelopes into padded bboxes
    // (min_x, min_y, max_x_plus_w, max_y) → (x, y, right, bottom) with padding + label min.
    let mut bboxes: Vec<Option<(usize, usize, usize, usize)>> = Vec::with_capacity(sg_count);
    for (sg_idx, sg) in dag.subgraphs.iter().enumerate() {
        bboxes.push(envelopes[sg_idx].map(|(min_x, min_y, max_x, max_y)| {
            let x = min_x.saturating_sub(A::SG_PAD_CROSS.0);
            let y = min_y.saturating_sub(A::SG_PAD_LEVEL.0);
            let right = max_x + A::SG_PAD_CROSS.1;
            // Place bottom border below edge routing area if possible.
            // The routing floor is the max Y used by any edge routing at the
            // subgraph's last level — the border must be below it.
            let last_level = sg_max_level[sg_idx];
            let routing_floor = if last_level < level_routing_floor.len() {
                level_routing_floor[last_level]
            } else {
                0
            };
            let base_bottom = max_y + A::SG_PAD_LEVEL.1;
            // Ensure bottom border row (base_bottom - 1) is below the routing floor
            let bottom = if routing_floor > 0 && base_bottom.saturating_sub(1) <= routing_floor {
                (routing_floor + 2).min(total_height) // +2: 1 blank row + border row
            } else {
                base_bottom.min(total_height)
            };
            // D8: the label's claim, per axis. The cross fold is the
            // legacy ║ Label ║ widening (label_len + 4; 0 under
            // Horizontal); the level fold is D8(b)'s other half
            // (0 under Vertical) — the two-phase offset build reserved
            // the room at this box's closing level.
            let width = right.saturating_sub(x);
            let min_label_width = A::label_cross_extent(sg.label);
            let right = if width < min_label_width {
                x + min_label_width
            } else {
                right
            };
            let min_label_level = A::label_level_extent(sg.label);
            let bottom = if bottom.saturating_sub(y) < min_label_level {
                y + min_label_level
            } else {
                bottom
            };
            (x, y, right, bottom)
        }));
    }

    // Pass 2: propagate child bounding boxes to parents (bottom-up).
    // The child bbox already includes its own cross-axis pads; the parent
    // adds only its border column (shared rule with the CSR backend).
    for &sg_idx in &order {
        let sg = &dag.subgraphs[sg_idx];
        if let Some(parent_id) = sg.parent_id {
            if let Some(&parent_idx) = sg_id_to_idx.get(&parent_id) {
                if let Some((cx, cy, cr, cb)) = bboxes[sg_idx] {
                    let expanded = (
                        cx.saturating_sub(A::PARENT_CHILD_PAD_CROSS.0),
                        cy.saturating_sub(A::PARENT_CHILD_PAD_LEVEL.0),
                        cr + A::PARENT_CHILD_PAD_CROSS.1,
                        cb + A::PARENT_CHILD_PAD_LEVEL.1,
                    );
                    bboxes[parent_idx] = Some(match bboxes[parent_idx] {
                        None => expanded,
                        Some((px, py, pr, pb)) => (
                            px.min(expanded.0),
                            py.min(expanded.1),
                            pr.max(expanded.2),
                            pb.max(expanded.3),
                        ),
                    });
                }
            }
        }
    }

    // Re-apply padding + label minimum to parents whose bbox grew from children
    // (process top-down so parents get their own padding after absorbing children)
    let mut top_down_order = order.clone();
    top_down_order.reverse();
    for &sg_idx in &top_down_order {
        let sg = &dag.subgraphs[sg_idx];
        if let Some((x, y, right, bottom)) = bboxes[sg_idx] {
            // Re-check the label claims (parent may have grown but the
            // label still needs room) — per axis, mirroring pass 1.5.
            let width = right.saturating_sub(x);
            let min_label_width = A::label_cross_extent(sg.label);
            let right = if width < min_label_width {
                x + min_label_width
            } else {
                right
            };
            let min_label_level = A::label_level_extent(sg.label);
            let bottom = if bottom.saturating_sub(y) < min_label_level {
                y + min_label_level
            } else {
                bottom
            };
            // Apply parent-level H_PAD around entire bbox (if it grew from children)
            bboxes[sg_idx] = Some((x, y, right, bottom));
        }
    }

    // Post-process: shift borders that would overlap with edge routing rows.
    // If a top or bottom border row coincides with a horizontal edge routing Y,
    // expand the bbox to push the border 1 row further away.
    if !edge_routing_ys.is_empty() {
        for sg_idx in 0..sg_count {
            if let Some((x, y, right, bottom)) = bboxes[sg_idx] {
                let mut new_y = y;
                let mut new_bottom = bottom;

                // Top border is at row `y`. If it's an edge routing row, push it up.
                if edge_routing_ys.contains(&y) && y > 0 {
                    new_y = y - 1;
                }

                // Bottom border is at row `bottom - 1`. If it's an edge routing row, push it down.
                let bottom_row = bottom.saturating_sub(1);
                if edge_routing_ys.contains(&bottom_row) {
                    new_bottom = bottom + 1;
                }

                if new_y != y || new_bottom != bottom {
                    bboxes[sg_idx] = Some((x, new_y, right, new_bottom));
                }
            }
        }
    }

    // Build SubgraphInfo entries — materialize the role rect into
    // physical IR (`x`/`right` are cross-axis, `y`/`bottom` level-axis
    // throughout this pass; for Vertical this is the identity).
    let mut result = Vec::with_capacity(sg_count);
    for (sg_idx, sg) in dag.subgraphs.iter().enumerate() {
        if let Some((x, y, right, bottom)) = bboxes[sg_idx] {
            let (px, py) = A::materialize(y, x);
            let (pr, pb) = A::materialize(bottom, right);
            result.push(SubgraphInfo {
                id: sg.id,
                parent_id: sg.parent_id,
                label: sg.label,
                x: px,
                y: py,
                width: pr.saturating_sub(px),
                height: pb.saturating_sub(py),
            });
        }
    }

    result
}
