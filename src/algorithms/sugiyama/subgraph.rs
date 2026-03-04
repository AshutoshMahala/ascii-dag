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

/// Count the number of subgraph boundary transitions between two nodes
/// (exits from `prev_sg` chain + entries into `curr_sg` chain).
///
/// For example, moving from a node in `[Root → A → X]` to a node in
/// `[Root → B → Y]` crosses:
///
///   - 2 exits (leave X, leave A)
///   - 2 entries (enter B, enter Y)
///   - → 4 transitions
fn count_boundary_transitions(dag: &Graph<'_>, prev_sg: Option<usize>, curr_sg: Option<usize>) -> usize {
    let prev_chain = sg_chain(dag, prev_sg);
    let curr_chain = sg_chain(dag, curr_sg);

    // Find the length of the shared common prefix
    let common = prev_chain
        .iter()
        .zip(curr_chain.iter())
        .take_while(|(a, b)| a == b)
        .count();

    // Transitions = exits (prev depth - common) + entries (curr depth - common)
    let exits = prev_chain.len() - common;
    let entries = curr_chain.len() - common;
    exits + entries
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
/// different subgraphs (or one is inside and the other is not), an extra
/// `2 * SUBGRAPH_H_PAD` chars of space is inserted.
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
                    // Count boundary transitions: exits from prev + entries into curr
                    let transitions = count_boundary_transitions(dag, prev_sg, curr_sg);
                    x += transitions * SUBGRAPH_H_PAD;
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
