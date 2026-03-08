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
//! 2. **After final coordinate assignment:**
//!    [`compute_bounding_boxes`] walks the node list to emit
//!    [`SubgraphInfo`] bounding boxes, propagating nested children
//!    bottom-up.
//!
//! Crossing reduction is block-partitioned via
//! [`block_partition_level`] which the heap pipeline calls in place of
//! its default ordering pass when subgraphs are present.

use crate::graph::Graph;
use crate::ir::SubgraphInfo;
use super::heap::VNode;
use alloc::vec;
use alloc::vec::Vec;

#[cfg(not(feature = "std"))]
use alloc::collections::BTreeMap as HashMap;
#[cfg(feature = "std")]
use std::collections::HashMap;

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

/// Walk the subgraph ancestry chain for `sg_id` and return the nesting depth
/// (0 → no subgraph, 1 → root-level subgraph, 2 → child of root, …).
fn sg_chain_depth(dag: &Graph<'_>, sg_id: Option<usize>) -> usize {
    let mut depth = 0usize;
    let mut cur = sg_id;
    while let Some(id) = cur {
        depth += 1;
        cur = dag.subgraphs.iter().find(|s| s.id == id).and_then(|s| s.parent_id);
    }
    depth
}

/// Build the full ancestry chain from `sg_id` up to the root (inclusive),
/// with the root first (index 0) and the leaf last.
fn sg_chain(dag: &Graph<'_>, sg_id: Option<usize>) -> Vec<usize> {
    let mut chain = Vec::new();
    let mut cur = sg_id;
    while let Some(id) = cur {
        chain.push(id);
        cur = dag.subgraphs.iter().find(|s| s.id == id).and_then(|s| s.parent_id);
    }
    chain.reverse(); // root first
    chain
}

/// Count the number of subgraph boundary exits and entries between two nodes.
///
/// For example, moving from a node in `[Root → A → X]` to a node in
/// `[Root → B → Y]` crosses:
///
///   - 2 exits (leave X, leave A)
///   - 2 entries (enter B, enter Y)
///
/// Returns `(exits, entries)`.
fn count_boundary_exits_entries(dag: &Graph<'_>, prev_sg: Option<usize>, curr_sg: Option<usize>) -> (usize, usize) {
    let prev_chain = sg_chain(dag, prev_sg);
    let curr_chain = sg_chain(dag, curr_sg);

    // Find the length of the shared common prefix
    let common = prev_chain
        .iter()
        .zip(curr_chain.iter())
        .take_while(|(a, b)| a == b)
        .count();

    let exits = prev_chain.len() - common;
    let entries = curr_chain.len() - common;
    (exits, entries)
}

// ── Block-partitioned crossing reduction ─────────────────────────────────

/// Partition a single virtual level into per-subgraph blocks.
///
/// Returns the level re-ordered so that nodes in the same subgraph are
/// contiguous, with unaffiliated nodes placed between blocks according to
/// their original median position.
///
/// The caller runs the normal median/exchange reduction **within** each
/// block, then calls this function to re-order blocks by their average
/// position.
pub(crate) fn block_partition_level(
    dag: &Graph<'_>,
    level: &[VNode],
) -> Vec<VNode> {
    if level.is_empty() {
        return Vec::new();
    }

    // Assign each vnode to a block key: Some(sg_id) or None (unaffiliated)
    let mut blocks: HashMap<Option<usize>, Vec<(usize, VNode)>> = HashMap::new();
    for (pos, vnode) in level.iter().enumerate() {
        let sg = vnode_subgraph(dag, vnode);
        blocks.entry(sg).or_default().push((pos, *vnode));
    }

    // Compute average original position per block for ordering
    let mut block_list: Vec<(Option<usize>, f64, Vec<VNode>)> = blocks
        .into_iter()
        .map(|(key, members)| {
            let avg = members.iter().map(|(pos, _)| *pos as f64).sum::<f64>()
                / members.len().max(1) as f64;
            let vnodes: Vec<VNode> = members.into_iter().map(|(_, v)| v).collect();
            (key, avg, vnodes)
        })
        .collect();

    // Sort blocks by average position (stable: unaffiliated nodes stay in place)
    block_list.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(core::cmp::Ordering::Equal));

    // Flatten back into a single level
    let mut result = Vec::with_capacity(level.len());
    for (_, _, vnodes) in block_list {
        result.extend(vnodes);
    }
    result
}

// ── Subgraph padding ─────────────────────────────────────────────────────

/// Per-subgraph horizontal padding constant (chars on each side of a border).
const SUBGRAPH_H_PAD: usize = 2;

/// Vertical padding above first node: border row + label row + 1 blank row.
pub(crate) const SUBGRAPH_V_PAD_TOP: usize = 3; // ╔═══╗ + ║ Label ║ + ║     ║
/// Vertical padding below last node: 1 blank row + border row.
pub(crate) const SUBGRAPH_V_PAD_BOTTOM: usize = 2; // ║     ║ + ╚═══════╝

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
pub(crate) fn subgraph_padding(
    dag: &Graph<'_>,
    virtual_levels: &[Vec<VNode>],
    x_coords: &mut [Vec<usize>],
    widths: &[Vec<usize>],
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

        // Left-side padding: depth of the first node's subgraph chain
        let first_depth = sg_chain_depth(dag, vnode_subgraph(dag, &vnodes[0]));
        x += first_depth * SUBGRAPH_H_PAD;

        for (i, vnode) in vnodes.iter().enumerate() {
            if i > 0 {
                let prev_sg = vnode_subgraph(dag, &vnodes[i - 1]);
                let curr_sg = vnode_subgraph(dag, vnode);
                if prev_sg != curr_sg {
                    // CSS-style margin collapsing: each side contributes a margin
                    // proportional to its nesting depth, and we take the larger one
                    // rather than summing both. This prevents quadratic blowup when
                    // many deeply-nested subgraphs sit side by side.
                    let (exits, entries) = count_boundary_exits_entries(dag, prev_sg, curr_sg);
                    let exit_margin = exits * SUBGRAPH_H_PAD;
                    let entry_margin = entries * SUBGRAPH_H_PAD;
                    x += core::cmp::max(exit_margin, entry_margin);
                }
            }
            new_x.push(x);
            let w = widths[lvl].get(i).copied().unwrap_or(3);
            x += w + 3; // standard spacing
        }

        // Right-side padding: depth of the last node's subgraph chain
        let last_depth = sg_chain_depth(dag, vnode_subgraph(dag, vnodes.last().unwrap()));
        let right_extra = last_depth * SUBGRAPH_H_PAD;

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

// ── Sibling subgraph overlap repair ──────────────────────────────────────

/// Minimum gap (in characters) between the bounding boxes of sibling subgraphs.
const SIBLING_GAP: usize = 1;

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
pub(crate) fn fix_subgraph_overlaps(
    dag: &Graph<'_>,
    real_node_coords: &mut [(usize, usize, usize, usize)], // (level, pos, x, width)
) -> usize {
    let sg_count = dag.subgraphs.len();
    if sg_count < 2 { return 0; }

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
            cur = dag.subgraphs.iter().find(|s| s.id == pid).and_then(|s| s.parent_id);
        }
        depths[i] = d;
    }
    let max_depth = depths.iter().copied().max().unwrap_or(0);

    // Minimum gap between nodes of different subgraphs on the same level.
    // Must cover the padding on each side plus the gap between borders:
    //   H_PAD(right of left-sg) + SIBLING_GAP + H_PAD(left of right-sg)
    let cross_sg_gap: usize = 2 * SUBGRAPH_H_PAD + SIBLING_GAP;

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
                if level < *min_l { *min_l = level; }
                if level > *max_l { *max_l = level; }
            }
        }
    }
    // Propagate child level ranges to parents (bottom-up)
    for depth in (0..=max_depth).rev() {
        for sg_idx in 0..sg_count {
            if depths[sg_idx] != depth { continue; }
            if let Some(parent_id) = dag.subgraphs[sg_idx].parent_id {
                if let Some(&pidx) = sg_id_to_idx.get(&parent_id) {
                    let (cl, cr) = sg_level_range[sg_idx];
                    if cl == usize::MAX { continue; }
                    let (ref mut pl, ref mut pr) = sg_level_range[pidx];
                    if cl < *pl { *pl = cl; }
                    if cr > *pr { *pr = cr; }
                }
            }
        }
    }

    // Compute padded bounding box (left, right) per subgraph from real_node_coords.
    let compute_bboxes = |coords: &[(usize, usize, usize, usize)]| -> Vec<Option<(usize, usize)>> {
        let mut envs: Vec<Option<(usize, usize)>> = vec![None; sg_count];
        for (node_idx, _) in dag.nodes.iter().enumerate() {
            if let Some(sg_idx) = node_sg[node_idx] {
                if node_idx >= coords.len() { continue; }
                let (_, _, x, width) = coords[node_idx];
                let right = x + width;
                envs[sg_idx] = Some(match envs[sg_idx] {
                    None => (x, right),
                    Some((mn, mx)) => (mn.min(x), mx.max(right)),
                });
            }
        }
        // Propagate children to parents (bottom-up)
        for depth in (0..=max_depth).rev() {
            for sg_idx in 0..sg_count {
                if depths[sg_idx] != depth { continue; }
                if let Some(parent_id) = dag.subgraphs[sg_idx].parent_id {
                    if let Some(&pidx) = sg_id_to_idx.get(&parent_id) {
                        if let Some((cx, cr)) = envs[sg_idx] {
                            let exp = (cx.saturating_sub(SUBGRAPH_H_PAD), cr + SUBGRAPH_H_PAD);
                            envs[pidx] = Some(match envs[pidx] {
                                None => exp,
                                Some((px, pr)) => (px.min(exp.0), pr.max(exp.1)),
                            });
                        }
                    }
                }
            }
        }
        envs.iter().enumerate().map(|(sg_idx, env)| {
            env.map(|(mn, mx)| {
                let left = mn.saturating_sub(SUBGRAPH_H_PAD);
                let right = mx + SUBGRAPH_H_PAD;
                let label_w = dag.subgraphs[sg_idx].label.len() + 4;
                let width = right.saturating_sub(left);
                let right = if width < label_w { left + label_w } else { right };
                (left, right)
            })
        }).collect()
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

        for (_parent, siblings) in &mut parent_groups {
            if siblings.len() < 2 { continue; }
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
                        let overlaps = prev_min_l <= cur_max_l
                            && cur_min_l <= prev_max_l;
                        if overlaps && prev_right > eff_frontier {
                            eff_frontier = prev_right;
                            has_level_overlap = true;
                        }
                    }

                    if has_level_overlap && eff_frontier + SIBLING_GAP > left {
                        let shift = eff_frontier + SIBLING_GAP - left;

                        let node_indices =
                            collect_sg_node_indices(dag, sg_idx, &sg_id_to_idx);
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

        if !any_shifted { break; }

        // After shifting subgraph nodes, fix per-level collisions.
        // Use a larger gap for nodes that belong to different subgraphs
        // so that resulting bboxes (which add H_PAD on each side) don't
        // re-overlap and trigger another round.
        let max_level = real_node_coords.iter().map(|c| c.0).max().unwrap_or(0);
        for level in 0..=max_level {
            let mut level_nodes: Vec<usize> = Vec::new();
            for (ni, &(lvl, _, _, _)) in real_node_coords.iter().enumerate() {
                if lvl == level { level_nodes.push(ni); }
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
                let prev_right =
                    real_node_coords[prev].2 + real_node_coords[prev].3 + gap;
                if real_node_coords[curr].2 < prev_right {
                    real_node_coords[curr].2 = prev_right;
                }
            }
        }
    }

    total_extra
}

// ── Bounding box computation ─────────────────────────────────────────────

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
pub(crate) fn compute_level_y_extras(
    dag: &Graph<'_>,
    node_levels: &[usize],
    max_level: usize,
) -> (usize, Vec<usize>, usize) {
    if dag.subgraphs.is_empty() || max_level == 0 {
        return (0, vec![0; max_level + 1], 0);
    }

    // For each subgraph, find (first_level, last_level).
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

    // Propagate child ranges to parents so a parent's first/last covers
    // all descendants (a parent's border encloses its children).
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
    let initial_open_depth = dag.subgraphs
        .iter()
        .enumerate()
        .filter_map(|(i, _)| sg_ranges[i].as_ref().map(|r| (i, r)))
        .filter(|(_, (first, _))| *first == 0)
        .map(|(i, _)| stacked_borders_opening(dag, i, 0, &sg_ranges))
        .max()
        .unwrap_or(0);
    let initial_offset = initial_open_depth * SUBGRAPH_V_PAD_TOP;

    // Per-boundary extras
    let mut extras = vec![0usize; max_level + 1];

    for boundary_after in 0..max_level {
        let next_level = boundary_after + 1;

        // Max stacked closing borders at boundary_after
        let close_depth = dag.subgraphs
            .iter()
            .enumerate()
            .filter_map(|(i, _)| sg_ranges[i].as_ref().map(|r| (i, r)))
            .filter(|(_, (_, last))| *last == boundary_after)
            .map(|(i, _)| {
                stacked_borders_closing(dag, i, boundary_after, &sg_ranges)
            })
            .max()
            .unwrap_or(0);

        // Max stacked opening borders at next_level
        let open_depth = dag.subgraphs
            .iter()
            .enumerate()
            .filter_map(|(i, _)| sg_ranges[i].as_ref().map(|r| (i, r)))
            .filter(|(_, (first, _))| *first == next_level)
            .map(|(i, _)| {
                stacked_borders_opening(dag, i, next_level, &sg_ranges)
            })
            .max()
            .unwrap_or(0);

        extras[boundary_after] = close_depth * SUBGRAPH_V_PAD_BOTTOM
            + open_depth * SUBGRAPH_V_PAD_TOP;
    }

    // Trailing extra: space for subgraphs whose last member is at max_level
    let trailing_close_depth = dag.subgraphs
        .iter()
        .enumerate()
        .filter_map(|(i, _)| sg_ranges[i].as_ref().map(|r| (i, r)))
        .filter(|(_, (_, last))| *last == max_level)
        .map(|(i, _)| stacked_borders_closing(dag, i, max_level, &sg_ranges))
        .max()
        .unwrap_or(0);
    let trailing_extra = trailing_close_depth * SUBGRAPH_V_PAD_BOTTOM;

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
pub(crate) fn compute_bounding_boxes<'a>(
    dag: &Graph<'a>,
    real_node_coords: &[(usize, usize, usize, usize)], // (level, pos, x, width) per node_idx
    level_y_offsets: &[usize],
    total_height: usize,
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

    for (node_idx, &(id, _label)) in dag.nodes.iter().enumerate() {
        if let Some(&sg_id) = dag.node_subgraph.get(&id) {
            if let Some(&sg_idx) = sg_id_to_idx.get(&sg_id) {
                let (level, _pos, x, width) = real_node_coords[node_idx];
                let y = if level < level_y_offsets.len() {
                    level_y_offsets[level]
                } else {
                    0
                };
                // Node occupies 1 line of height
                let node_max_y = y + 1;
                let node_max_x = x + width;

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
            current = dag.subgraphs.iter().find(|s| s.id == pid).and_then(|s| s.parent_id);
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
            let x = min_x.saturating_sub(SUBGRAPH_H_PAD);
            let y = min_y.saturating_sub(SUBGRAPH_V_PAD_TOP);
            let right = max_x + SUBGRAPH_H_PAD;
            let bottom = (max_y + SUBGRAPH_V_PAD_BOTTOM).min(total_height);
            // Ensure width fits the label: ║ Label ║ needs label_len + 4
            let width = right.saturating_sub(x);
            let min_label_width = sg.label.len() + 4;
            let right = if width < min_label_width {
                x + min_label_width
            } else {
                right
            };
            (x, y, right, bottom)
        }));
    }

    // Pass 2: propagate child bounding boxes to parents (bottom-up)
    // Add SUBGRAPH_H_PAD-cell margin so child borders don't touch parent borders.
    for &sg_idx in &order {
        let sg = &dag.subgraphs[sg_idx];
        if let Some(parent_id) = sg.parent_id {
            if let Some(&parent_idx) = sg_id_to_idx.get(&parent_id) {
                if let Some((cx, cy, cr, cb)) = bboxes[sg_idx] {
                    let expanded = (
                        cx.saturating_sub(SUBGRAPH_H_PAD),
                        cy.saturating_sub(SUBGRAPH_V_PAD_TOP),
                        cr + SUBGRAPH_H_PAD,
                        cb + SUBGRAPH_V_PAD_BOTTOM,
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
            // Re-check label width (parent may have grown but label still needs room)
            let width = right.saturating_sub(x);
            let min_label_width = sg.label.len() + 4;
            let right = if width < min_label_width {
                x + min_label_width
            } else {
                right
            };
            // Apply parent-level H_PAD around entire bbox (if it grew from children)
            bboxes[sg_idx] = Some((x, y, right, bottom));
        }
    }

    // Build SubgraphInfo entries
    let mut result = Vec::with_capacity(sg_count);
    for (sg_idx, sg) in dag.subgraphs.iter().enumerate() {
        if let Some((x, y, right, bottom)) = bboxes[sg_idx] {
            let width = right.saturating_sub(x);
            let height = bottom.saturating_sub(y);
            result.push(SubgraphInfo {
                id: sg.id,
                parent_id: sg.parent_id,
                label: sg.label,
                x,
                y,
                width,
                height,
            });
        }
    }

    result
}
