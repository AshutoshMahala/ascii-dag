//! Heap-based Sugiyama layout pipeline.
//!
//! Implements the full layout algorithm using standard heap allocations (`Vec`, `HashMap`).
//! This is the default layout path for `Graph::compute_layout()`.
//!
//! # Pipeline Stages
//!
//! 1. **Level assignment** — iterative fixed-point (from `layout.rs`)
//! 2. **Virtual node insertion** — dummy nodes for skip-level edges
//! 3. **Crossing reduction** — median heuristic on virtual levels
//! 4. **X-coordinate assignment** — left-to-right packing with centering
//! 5. **Slot allocation** — horizontal channel assignment for edge separation
//! 6. **Edge routing** — direct, corner, or multi-segment paths
//!
//! # Relationship to CSR Path
//!
//! The CSR-based layout in `arena_csr.rs` implements the same algorithm
//! using arena allocation and `Idx`-typed indices. The two paths produce
//! visually compatible output but operate on different type systems.
//! Shared spacing constants live in [`super::geometry`] so the backends
//! cannot drift apart.

use crate::algorithms::sugiyama::config::{CycleBreaking, LayoutConfig};
use crate::algorithms::sugiyama::crossing::{CrossingReducer, count_crossings_pair};
use crate::graph::Graph;
use crate::ir::{EdgePath, LayoutEdge, LayoutIR, LayoutIRBuilder, LayoutNode, NodeKind};
use alloc::vec;
use alloc::vec::Vec;

#[cfg(not(feature = "std"))]
use alloc::collections::{BTreeMap as HashMap, BTreeSet as HashSet};
#[cfg(feature = "std")]
use std::collections::{HashMap, HashSet};

// ── VNode: Virtual node for layout computation ───────────────────────────

/// Virtual node — either a real graph node or a dummy inserted for edge routing.
///
/// During layout, skip-level edges (spanning more than one level) are broken
/// into segments by inserting dummy nodes at each intermediate level. This
/// allows crossing reduction and x-coordinate assignment to treat all nodes
/// uniformly.
#[derive(Clone, Copy)]
pub(crate) enum VNode {
    /// A real node from the input graph, identified by its index in `Graph.nodes`.
    Real(usize),
    /// A dummy node inserted on a skip-level edge, identified by the edge index.
    Dummy { edge_idx: usize },
}

impl VNode {
    fn real_index(&self) -> Option<usize> {
        match self {
            VNode::Real(idx) => Some(*idx),
            VNode::Dummy { .. } => None,
        }
    }

    fn dummy_edge(&self) -> Option<usize> {
        match self {
            VNode::Real(_) => None,
            VNode::Dummy { edge_idx } => Some(*edge_idx),
        }
    }
}

// ── Main layout entry point ──────────────────────────────────────────────

/// Compute the heap-based layout IR for a DAG using a [`LayoutConfig`].
///
/// This is the implementation behind `Graph::compute_layout()`. Returns a
/// renderer-agnostic `LayoutIR` containing node positions, edge routes,
/// and dimensional information.
/// Compute the layout under axis profile `A` (temp/08 D1). The body
/// currently spells the roles as y/x — LR-P0 threads `A` inward slice
/// by slice, each slice gated on byte-identical TD/BT output.
pub(crate) fn compute_layout_cfg<'a, A: super::geometry::Axis>(
    dag: &Graph<'a>,
    config: &LayoutConfig<'_>,
) -> LayoutIR<'a> {
    if dag.nodes.is_empty() {
        return LayoutIRBuilder::new().build();
    }

    // Honor spacing config (previously ignored; see 0.10.0 changelog).
    let node_spacing = config.node_spacing;
    let level_spacing = config.level_spacing;

    // Cycle breaking: dispatch based on LayoutConfig.
    // DepthFirst detects back edges via three-color DFS.
    // None treats every edge as forward (caller asserts acyclicity).
    let back_edges = match config.cycle_breaking() {
        CycleBreaking::DepthFirst => dag.detect_back_edges(),
        CycleBreaking::None => vec![false; dag.edges.len()],
    };

    // 2-node-cycle detection in O(E log E): sort edge indices by their
    // normalized endpoint pair, then scan each run for an anti-parallel
    // twin with the opposite back flag. Previously the CSR backend did
    // this with an O(E) scan per straight edge (O(E²) worst case) and
    // the heap backend lacked the feature entirely.
    let edge_in_two_cycle = {
        let mut flags = vec![false; dag.edges.len()];
        let pair_key = |ei: usize| {
            let (f, t, _) = dag.edges[ei];
            if f <= t { (f, t) } else { (t, f) }
        };
        let mut order: Vec<usize> = (0..dag.edges.len()).collect();
        order.sort_unstable_by_key(|&ei| pair_key(ei));

        let mut run_start = 0;
        while run_start < order.len() {
            let mut run_end = run_start + 1;
            while run_end < order.len() && pair_key(order[run_end]) == pair_key(order[run_start]) {
                run_end += 1;
            }
            // Bucket the run by (direction, back-flag); an edge is in a
            // 2-node cycle iff an opposite-direction, opposite-flag twin
            // exists (matches the CSR predicate exactly).
            let mut counts = [[0usize; 2]; 2];
            for &ei in &order[run_start..run_end] {
                let (f, t, _) = dag.edges[ei];
                if f == t {
                    continue; // self-loop
                }
                let dir = usize::from(f > t);
                let back = usize::from(back_edges.get(ei).copied().unwrap_or(false));
                counts[dir][back] += 1;
            }
            for &ei in &order[run_start..run_end] {
                let (f, t, _) = dag.edges[ei];
                if f == t {
                    continue;
                }
                let dir = usize::from(f > t);
                let back = usize::from(back_edges.get(ei).copied().unwrap_or(false));
                if counts[1 - dir][1 - back] > 0 {
                    flags[ei] = true;
                }
            }
            run_start = run_end;
        }
        flags
    };

    // Step 1: Calculate levels, treating back edges as reversed
    let level_data = dag.calculate_levels_with_back_edges(&back_edges);
    let max_level = level_data.iter().map(|(_, l)| *l).max().unwrap_or(0);

    // Create level mapping for real nodes
    let mut node_levels: Vec<usize> = vec![0; dag.nodes.len()];
    for (idx, level) in &level_data {
        node_levels[*idx] = *level;
    }

    // Step 2: Build virtual levels with dummy nodes for skip-level edges
    let mut virtual_levels: Vec<Vec<VNode>> = vec![Vec::new(); max_level + 1];

    // Add real nodes to their levels
    for (idx, level) in &level_data {
        virtual_levels[*level].push(VNode::Real(*idx));
    }

    // Identify skip-level edges and insert dummy nodes
    // For back edges, the layout direction is reversed (to → from in level space)
    for (edge_idx, &(from_id, to_id, _label)) in dag.edges.iter().enumerate() {
        let is_back = back_edges.get(edge_idx).copied().unwrap_or(false);
        if let (Some(from_idx), Some(to_idx)) = (dag.node_index(from_id), dag.node_index(to_id)) {
            // For back edges, layout-direction is reversed
            let (layout_from, layout_to) = if is_back {
                (node_levels[to_idx], node_levels[from_idx])
            } else {
                (node_levels[from_idx], node_levels[to_idx])
            };

            if layout_to > layout_from + 1 {
                // Skip-level edge - insert dummy nodes at intermediate levels
                for level in (layout_from + 1)..layout_to {
                    virtual_levels[level].push(VNode::Dummy { edge_idx });
                }
            }
        }
    }

    // Step 3: Apply crossing reduction WITH dummy nodes included
    reduce_crossings_virtual(
        dag,
        &mut virtual_levels,
        &node_levels,
        max_level,
        config.crossing_pipeline(),
    );

    // Step 3b: Block-partitioned level ordering for subgraph adjacency
    if dag.has_subgraphs() {
        use crate::algorithms::sugiyama::subgraph::block_partition_level;
        for level in virtual_levels.iter_mut() {
            *level = block_partition_level(dag, level);
        }
    }

    // Step 4: Assign x-coordinates to virtual nodes
    let mut x_coords: Vec<Vec<usize>> = Vec::with_capacity(virtual_levels.len());
    let mut widths: Vec<Vec<usize>> = Vec::with_capacity(virtual_levels.len());

    for level_vnodes in &virtual_levels {
        let mut level_x = Vec::with_capacity(level_vnodes.len());
        let mut level_w = Vec::with_capacity(level_vnodes.len());
        let mut x = 0;

        for vnode in level_vnodes {
            let width = match vnode {
                // Cross-axis extent (Vertical: the node's width).
                VNode::Real(idx) => {
                    A::cross_extent(dag.get_node_width(*idx), dag.get_node_height(*idx))
                }
                VNode::Dummy { .. } => A::DUMMY_CROSS,
            };
            level_x.push(x);
            level_w.push(width);
            x += width + node_spacing;
        }

        x_coords.push(level_x);
        widths.push(level_w);
    }

    // Insert extra horizontal padding at subgraph boundary transitions
    if dag.has_subgraphs() {
        crate::algorithms::sugiyama::subgraph::subgraph_padding(
            dag,
            &virtual_levels,
            &mut x_coords,
            &widths,
            node_spacing,
        );
    }

    // Step 4b: Refine x-coordinates — shift nodes toward their connected
    // neighbors on adjacent levels to reduce zigzag edges (median placement).
    // Then compact subgraphs. Run iteratively: compaction moves what it can,
    // x-refinement and subgraph compaction (iterative).
    // x-refinement is only beneficial for subgraph layouts; skipping it for
    // plain graphs avoids an O(N²/L) cost on large inputs.
    if dag.has_subgraphs() {
        let node_edge_indices_for_refine = build_node_edge_indices(dag);
        let compact_rounds = 3;
        for _ in 0..compact_rounds {
            refine_x_positions(
                dag,
                &virtual_levels,
                &mut x_coords,
                &widths,
                &node_edge_indices_for_refine,
                node_spacing,
            );
            compact_subgraphs(dag, &virtual_levels, &mut x_coords, &widths, node_spacing);
        }
    }

    // Calculate total width and centering offsets
    let level_widths: Vec<usize> = x_coords
        .iter()
        .zip(widths.iter())
        .map(|(xs, ws)| {
            xs.iter()
                .zip(ws.iter())
                .map(|(x, w)| x + w)
                .max()
                .unwrap_or(0)
        })
        .collect();

    // Add small buffer for bounded edge offsets (max 3 chars) plus 1 for routing
    // Also add limited expansion for labels (4 chars each side) if any edges have labels
    let has_labeled_edges = dag.edges.iter().any(|(_, _, label)| label.is_some());
    let label_margin = if has_labeled_edges { 8 } else { 0 }; // 4 chars each side
    let subgraph_margin = if dag.has_subgraphs() { 4 } else { 0 }; // border padding
    let max_width = level_widths.iter().max().unwrap_or(&0) + 4 + label_margin + subgraph_margin;

    // Step 5: Build LayoutIR
    let mut builder = LayoutIRBuilder::new().with_levels(max_level + 1);

    // Compute horizontal channel slots to prevent edges from different sources overlapping.
    // We use a hybrid approach:
    // 1. Fan-Out (1 -> Many): All children share the Source's slot (Source Bus).
    // 2. Fan-In (Many -> 1): All parents share the Target's slot (Target Bus) IF In-Degree > 1.

    // Calculate in-degrees to identify Fan-In candidates
    let mut in_degrees = vec![0usize; dag.nodes.len()];
    for &(_, to_id, _) in &dag.edges {
        if let Some(idx) = dag.node_index(to_id) {
            in_degrees[idx] += 1;
        }
    }

    // PRE-CALCULATE COORDINATES: Needed for geometry-aware slot allocation
    // Build lookup: for each real node, find its (level, position, x, width)
    let mut real_node_coords: Vec<(usize, usize, usize, usize)> =
        vec![(0, 0, 0, 0); dag.nodes.len()];

    for (lvl_idx, level_vnodes) in virtual_levels.iter().enumerate() {
        for (idx, vnode) in level_vnodes.iter().enumerate() {
            if let VNode::Real(node_idx) = vnode {
                let x = x_coords[lvl_idx][idx];
                let w = widths[lvl_idx][idx];
                real_node_coords[*node_idx] = (lvl_idx, idx, x, w);
            }
        }
    }

    // Apply centering offsets to real node coordinates.
    // When subgraphs are present, skip per-level centering: the median
    // x-assignment already places children near parents, and independent
    // centering of each level destroys that vertical alignment (zigzag).
    let center_levels = !dag.has_subgraphs();
    for (level_idx, level_vnodes) in virtual_levels.iter().enumerate() {
        let level_width = level_widths[level_idx];
        let level_offset = if center_levels && max_width > level_width {
            (max_width - level_width) / 2
        } else {
            0
        };

        for (pos, vnode) in level_vnodes.iter().enumerate() {
            if let VNode::Real(idx) = vnode {
                let x = x_coords[level_idx][pos] + level_offset;
                let width = widths[level_idx][pos];
                real_node_coords[*idx] = (level_idx, pos, x, width);
            }
        }
    }

    // Fix sibling subgraph overlaps that arise from centering (different levels
    // get different centering offsets, which can push bounding boxes together).
    let max_width = if dag.has_subgraphs() {
        let extra = crate::algorithms::sugiyama::subgraph::fix_subgraph_overlaps(
            dag,
            &mut real_node_coords,
        );
        // Reclaim slack the sibling shifts left behind: pull nodes toward
        // their connected neighbors within current level bounds.
        crate::algorithms::sugiyama::subgraph::tighten_levels(
            dag,
            &mut real_node_coords,
            node_spacing,
        );
        // Cluster-width feedback: push unaffiliated nodes clear of each
        // cluster's projected border envelope (cross-level extent + label
        // minimum). Runs after overlap repair so it sees the coordinates
        // the bounding boxes will actually be computed from.
        let pushed = crate::algorithms::sugiyama::subgraph::clear_external_overlaps(
            dag,
            &mut real_node_coords,
            node_spacing,
        );
        // Pull whole root clusters (and loose nodes) back together after
        // the overlap shifts — reclaims the empty gulfs between boxes.
        let reclaimed = crate::algorithms::sugiyama::subgraph::compact_clusters(
            dag,
            &mut real_node_coords,
            &virtual_levels,
            &mut x_coords,
            node_spacing,
        );
        // Waypoints must never cross node text (crossing a border renders
        // as a junction and is acceptable; crossing a node is not).
        crate::algorithms::sugiyama::subgraph::nudge_dummies_off_nodes(
            &virtual_levels,
            &mut x_coords,
            &real_node_coords,
        );
        (max_width + extra + pushed).saturating_sub(reclaimed)
    } else {
        max_width
    };

    let mut node_slots = vec![usize::MAX; dag.nodes.len()];
    let mut edge_slots = vec![0usize; dag.edges.len()];

    let mut level_occupied_slots: Vec<Vec<Vec<(usize, usize)>>> = vec![Vec::new(); max_level + 1];

    // Maximum horizontal routing rows per level.
    // Gives full visual separation for typical fan-in (≤8 sources),
    // and graceful degradation (shared rows) for extreme fan-in.
    const MAX_SLOTS_PER_LEVEL: usize = 8;

    // Arrow-cell reservation: a reversed edge paints ⇡ on the first
    // routing row, directly below its layout-source. Pre-occupy that
    // cell (± ARROW_CELL_PAD) on slot 0 so any horizontal span that
    // would run through the arrowhead is pushed to a deeper slot by
    // the normal interval-collision logic below. Mirrored in the CSR
    // allocator — the two must not drift.
    for (ei, &(from_id, to_id, _)) in dag.edges.iter().enumerate() {
        if from_id == to_id || !back_edges.get(ei).copied().unwrap_or(false) {
            continue;
        }
        if let (Some(_), Some(to_idx)) = (dag.node_index(from_id), dag.node_index(to_id)) {
            let layout_src = to_idx; // back edge: layout flow is to → from
            let (_, _, x, w) = real_node_coords[layout_src];
            let ax = x + w / 2;
            use crate::algorithms::sugiyama::geometry::ARROW_CELL_PAD;
            let level = node_levels[layout_src];
            let slots = &mut level_occupied_slots[level];
            if slots.is_empty() {
                slots.push(Vec::new());
            }
            slots[0].push((ax.saturating_sub(ARROW_CELL_PAD), ax + ARROW_CELL_PAD));
        }
    }

    // 1. Assign slots greedy — in layout direction, so back edges
    // participate: their own horizontal always covers their own arrow
    // column, so the seeded cell forces them below the arrow row.
    // (Matches the CSR allocator, which already worked in layout space.)
    for (i, &(from_id, to_id, _)) in dag.edges.iter().enumerate() {
        if let (Some(from_idx), Some(to_idx)) = (dag.node_index(from_id), dag.node_index(to_id)) {
            let is_back = back_edges.get(i).copied().unwrap_or(false);
            let (ls_idx, ld_idx) = if is_back {
                (to_idx, from_idx)
            } else {
                (from_idx, to_idx)
            };
            let from_level = node_levels[ls_idx];
            let to_level = node_levels[ld_idx];

            // Get coordinates to determine geometry
            let (_, _, from_x, from_w) = real_node_coords[ls_idx];
            let (_, _, to_x, to_w) = real_node_coords[ld_idx];

            // Calculate interval required for this edge
            let start_x = from_x + from_w / 2;
            let end_x = to_x + to_w / 2;
            let (min_x, max_x) = if start_x < end_x {
                (start_x, end_x)
            } else {
                (end_x, start_x)
            };

            if to_level > from_level {
                let is_vertical = min_x == max_x && to_level == from_level + 1;

                // Source-based slot assignment (unified for all edges)
                // Each source node gets a unique horizontal slot at its level,
                // matching arena layout for visual parity.
                if is_vertical {
                    edge_slots[i] = usize::MAX;
                } else {
                    if node_slots[ls_idx] != usize::MAX {
                        // Reuse existing slot for this source node
                        let slot = node_slots[ls_idx];
                        edge_slots[i] = slot;

                        // Mark interval as occupied (Merge)
                        if slot < level_occupied_slots[from_level].len() {
                            let intervals = &mut level_occupied_slots[from_level][slot];
                            let mut merged = false;
                            if let Some(last) = intervals.last_mut() {
                                if min_x <= last.1 && max_x >= last.0 {
                                    last.0 = last.0.min(min_x);
                                    last.1 = last.1.max(max_x);
                                    merged = true;
                                }
                            }
                            if !merged {
                                intervals.push((min_x, max_x));
                            }
                        }
                    } else {
                        let slots = &mut level_occupied_slots[from_level];
                        let mut chosen_slot = None;

                        for (s_idx, occupied) in slots.iter_mut().enumerate() {
                            if let Some(last) = occupied.last() {
                                if min_x >= last.1 {
                                    occupied.push((min_x, max_x));
                                    chosen_slot = Some(s_idx);
                                    break;
                                }
                            }

                            let collide = occupied.iter().any(|&(s, e)| s < max_x && e > min_x);
                            if !collide {
                                occupied.push((min_x, max_x));
                                chosen_slot = Some(s_idx);
                                break;
                            }
                        }

                        let slot = if let Some(s) = chosen_slot {
                            s
                        } else if slots.len() < MAX_SLOTS_PER_LEVEL {
                            slots.push(vec![(min_x, max_x)]);
                            slots.len() - 1
                        } else {
                            // Cap reached: reuse slot 0 to bound level height.
                            // For extreme fan-in (e.g. 50k→1) all intervals
                            // overlap at the target X anyway, so visual overlap
                            // is unavoidable and this keeps output bounded.
                            slots[0].push((min_x, max_x));
                            0
                        };

                        node_slots[ls_idx] = slot;
                    }
                    edge_slots[i] = node_slots[ls_idx];
                }
            }
        }
    }

    // Collect dummy node X positions for skip-level edge routing
    let mut dummy_positions: Vec<Vec<(usize, usize)>> = vec![Vec::new(); dag.edges.len()];
    for (level_idx, level_vnodes) in virtual_levels.iter().enumerate() {
        let level_width = level_widths[level_idx];
        // Apply the same centering policy as real nodes: skip when subgraphs
        // are present to avoid misalignment between real and dummy positions.
        let level_offset = if center_levels && max_width > level_width {
            (max_width - level_width) / 2
        } else {
            0
        };

        for (pos, vnode) in level_vnodes.iter().enumerate() {
            if let VNode::Dummy { edge_idx } = vnode {
                // Get the actual x-coordinate from the layout, including centering offset
                let base_x = x_coords[level_idx][pos] + level_offset;
                // Add bounded offset for visual separation between skip-level edges
                // This keeps convergent edges from merging visually
                let edge_offset = (*edge_idx % 4) as usize; // 0, 1, 2, or 3 chars
                let x = base_x + edge_offset;
                dummy_positions[*edge_idx].push((level_idx, x));
            }
        }
    }

    // Sort dummy positions by level for each edge (they should already be in order, but ensure it)
    for positions in &mut dummy_positions {
        positions.sort_by_key(|(level, _)| *level);
    }

    // Jog-aware dummy rows: a waypoint claims a routing row only where the
    // edge actually changes column — its x differs from the NEXT chain x
    // (next waypoint, or the layout-target center), because the bend to a
    // new column is painted right below the kept row. Straight pass-through
    // dummies keep their reserved column in the level packing but need no
    // routing row of their own. Mirrored in the CSR backend.
    let mut kept_wps: Vec<Vec<bool>> = Vec::with_capacity(dag.edges.len());
    let mut level_jog_count = vec![0usize; max_level + 1];
    for (edge_idx, chain) in dummy_positions.iter().enumerate() {
        let mut kept = vec![false; chain.len()];
        if !chain.is_empty() {
            let &(from_id, to_id, _) = &dag.edges[edge_idx];
            let is_back = back_edges.get(edge_idx).copied().unwrap_or(false);
            if let (Some(from_idx), Some(to_idx)) =
                (dag.node_index(from_id), dag.node_index(to_id))
            {
                let layout_dst = if is_back { from_idx } else { to_idx };
                let (_, _, dx, dw) = real_node_coords[layout_dst];
                let target_x = dx + dw / 2;
                for i in 0..chain.len() {
                    let next_x = if i + 1 < chain.len() {
                        chain[i + 1].1
                    } else {
                        target_x
                    };
                    if chain[i].1 != next_x {
                        kept[i] = true;
                        level_jog_count[chain[i].0] += 1;
                    }
                }
            }
        }
        kept_wps.push(kept);
    }

    // Per-level label-source flags: the label row is budgeted only in the
    // bands of levels that actually source a labeled edge (labels paint in
    // the layout-source's band). Mirrored in the CSR backend.
    let mut level_labeled_src = vec![false; max_level + 1];
    for (ei, &(from_id, to_id, label)) in dag.edges.iter().enumerate() {
        if label.is_none() || from_id == to_id {
            continue;
        }
        if let (Some(from_idx), Some(to_idx)) = (dag.node_index(from_id), dag.node_index(to_id)) {
            let is_back = back_edges.get(ei).copied().unwrap_or(false);
            let layout_src = if is_back { to_idx } else { from_idx };
            level_labeled_src[node_levels[layout_src]] = true;
        }
    }

    // Calculate per-level heights: node height + routing overhead + extra rows for slot separation
    // Compute max node height per level from actual node heights
    let mut max_node_height: Vec<usize> = vec![1; max_level + 1];
    for (level, level_vnodes) in virtual_levels.iter().enumerate() {
        for vnode in level_vnodes {
            if let VNode::Real(idx) = vnode {
                // Level-axis extent (Vertical: the node's height).
                let node_height =
                    A::level_extent(dag.get_node_width(*idx), dag.get_node_height(*idx));
                if node_height > max_node_height[level] {
                    max_node_height[level] = node_height;
                }
            }
        }
    }
    let mut level_y_offsets = Vec::with_capacity(max_level + 1);

    // When subgraphs exist, compute per-boundary extra rows for opening/closing borders
    let (sg_initial_offset, sg_boundary_extras, sg_trailing_extra) = if dag.has_subgraphs() {
        crate::algorithms::sugiyama::subgraph::compute_level_y_extras(dag, &node_levels, max_level)
    } else {
        (0, vec![0; max_level + 1], 0)
    };

    let mut current_offset = sg_initial_offset;

    for level in 0..=max_level {
        level_y_offsets.push(current_offset);

        // 1. Slots for edges originating at this level (adjacent or skip)
        let adjacent_slots = level_occupied_slots[level].len();

        // 2. Rows for edges passing through: only jogging waypoints claim
        // a row (straight pass-throughs are pure verticals), plus the
        // bend row below the deepest jog (shared rule with CSR).
        let skip_slots = crate::algorithms::sugiyama::geometry::passthrough_rows(
            level_jog_count[level],
        );

        // Determine max slots needed for this specific level
        let slots_needed = adjacent_slots.max(skip_slots);
        let extra_lines = slots_needed.saturating_sub(1);

        // Per-level overhead: the label row is budgeted only where a
        // labeled edge is sourced (shared rule with the CSR backend).
        let routing_overhead =
            crate::algorithms::sugiyama::geometry::routing_overhead(level_labeled_src[level]);
        let height =
            max_node_height[level] + routing_overhead + extra_lines + sg_boundary_extras[level];
        current_offset += height;
        // Extra vertical gap between levels only — not after the last one,
        // which would pad the bottom of the canvas with blank rows.
        if level < max_level {
            current_offset += level_spacing;
        }
    }

    // Total height: current_offset already includes all subgraph border spacing
    // plus trailing extra for subgraphs closing after the last level
    let total_height = current_offset + sg_trailing_extra;

    // Self-loop flags in one O(E) pass (was a full edge scan per node —
    // O(N·E), the dominant cost on large fan-in graphs).
    let mut node_has_self_loop = vec![false; dag.nodes.len()];
    for &(f, t, _) in &dag.edges {
        if f == t {
            if let Some(idx) = dag.node_index(f) {
                node_has_self_loop[idx] = true;
            }
        }
    }

    // Add real nodes to IR
    for (level_idx, level_vnodes) in virtual_levels.iter().enumerate() {
        for vnode in level_vnodes {
            if let VNode::Real(idx) = vnode {
                let (level, pos, x, width) = real_node_coords[*idx];
                let y = level_y_offsets[level];

                let (id, label) = dag.nodes[*idx];
                let kind = if dag.auto_created.contains(&id) {
                    NodeKind::Implicit
                } else {
                    NodeKind::Explicit
                };
                let node_height = dag.get_node_height(*idx);
                builder.add_node(LayoutNode {
                    id,
                    label,
                    y,
                    x,
                    width,
                    height: node_height,
                    center_x: x + width / 2,
                    center_y: y + node_height.saturating_sub(1) / 2,
                    level: level_idx,
                    level_position: pos,
                    kind,
                    has_self_loop: node_has_self_loop[*idx],
                    edge_index: None,
                    content_tag: dag.node_kind_tag[*idx],
                });
                // Carry the node's declared painter/payload (sparse —
                // present only for custom-content nodes).
                if let Ok(pos) = dag.node_custom.binary_search_by_key(idx, |entry| entry.0) {
                    let (_, painter, payload) = dag.node_custom[pos];
                    builder.add_custom_for_last(painter, payload);
                }
            }
        }
    }

    // Emit dummy nodes into the IR (opt-in; zero cost when disabled).
    // Positions use the exact same computation as the waypoint chains, so
    // a dummy node and its edge's waypoint always share a column.
    if config.include_dummy_nodes {
        let mut synthetic = 0usize;
        for (level_idx, level_vnodes) in virtual_levels.iter().enumerate() {
            let level_width = level_widths[level_idx];
            let level_offset = if center_levels && max_width > level_width {
                (max_width - level_width) / 2
            } else {
                0
            };
            for (pos, vnode) in level_vnodes.iter().enumerate() {
                if let VNode::Dummy { edge_idx } = vnode {
                    let x = x_coords[level_idx][pos] + level_offset + (*edge_idx % 4);
                    let y = level_y_offsets[level_idx];
                    // Synthetic id, excluded from id_to_index by the builder.
                    let id = usize::MAX - synthetic;
                    synthetic += 1;
                    builder.add_node(LayoutNode {
                        id,
                        label: "",
                        x,
                        y,
                        width: 1,
                        height: 1,
                        center_x: x,
                        center_y: y,
                        level: level_idx,
                        level_position: pos,
                        kind: NodeKind::Dummy,
                        has_self_loop: false,
                        edge_index: Some(*edge_idx),
                        content_tag: 0,
                    });
                }
            }
        }
    }

    let mut level_edge_next = vec![0usize; max_level + 1];

    // Collect all horizontal Y values used by edge routing segments.
    // These will be passed to bounding box computation so subgraph borders
    // can be shifted to avoid overlapping with edge routing rows.
    let mut edge_routing_ys: HashSet<usize> = HashSet::new();

    // Per-level routing floor: the maximum Y used by edge routing at each level.
    // Bottom borders of subgraphs closing at level L must be placed BELOW this floor.
    let mut level_routing_floor: Vec<usize> = vec![0; max_level + 1];

    // Step 6: Add edges with proper routing
    for (edge_idx, &(from_id, to_id, label)) in dag.edges.iter().enumerate() {
        if let (Some(from_idx), Some(to_idx)) = (dag.node_index(from_id), dag.node_index(to_id)) {
            let is_back = back_edges.get(edge_idx).copied().unwrap_or(false);

            // Self-loops: skip layout routing (rendered as ↺ indicator by renderer)
            if from_id == to_id {
                continue;
            }

            // For back edges, layout direction is reversed (to→from in level space).
            // We compute coordinates in layout order, then store semantic IDs in the IR.
            let (layout_src_idx, layout_dst_idx) = if is_back {
                (to_idx, from_idx)
            } else {
                (from_idx, to_idx)
            };
            let layout_from_level = node_levels[layout_src_idx];
            let layout_to_level = node_levels[layout_dst_idx];

            let (_, _, src_x_base, src_width) = real_node_coords[layout_src_idx];
            let (_, _, dst_x_base, dst_width) = real_node_coords[layout_dst_idx];

            let from_x = src_x_base + src_width / 2;
            let to_x = dst_x_base + dst_width / 2;
            // 2-node cycle sharing a column: offset the forward edge left
            // and the back edge right so the anti-parallel pair renders
            // side by side (↓ next to ⇡) instead of overlapping. Matches
            // the CSR backend.
            let (from_x, to_x) = if from_x == to_x
                && from_id != to_id
                && edge_in_two_cycle.get(edge_idx).copied().unwrap_or(false)
            {
                if is_back {
                    (from_x + 1, to_x + 1)
                } else {
                    (from_x.saturating_sub(1), to_x.saturating_sub(1))
                }
            } else {
                (from_x, to_x)
            };
            // from_y = bottom row of source node (so edges start below it)
            let from_y =
                level_y_offsets[layout_from_level] + max_node_height[layout_from_level] - 1;
            let to_y = level_y_offsets[layout_to_level];

            // Edge routing starts one row below the source node. Reversed
            // edges' arrowheads on that row are protected by the arrow-cell
            // reservation in the slot allocator, not by shifting corners.
            let edge_start_row = crate::algorithms::sugiyama::geometry::EDGE_START_ROW;

            let path = if layout_to_level == layout_from_level + 1 {
                // Adjacent levels - direct or corner connection
                if from_x == to_x {
                    EdgePath::Direct
                } else {
                    // Get horizontal slot for this source at this level
                    let slot = if node_slots[layout_src_idx] != usize::MAX {
                        node_slots[layout_src_idx]
                    } else {
                        0
                    };

                    let hy = from_y + edge_start_row + slot;
                    edge_routing_ys.insert(hy);
                    if layout_from_level < level_routing_floor.len() {
                        level_routing_floor[layout_from_level] =
                            level_routing_floor[layout_from_level].max(hy);
                    }
                    EdgePath::Corner { horizontal_y: hy }
                }
            } else {
                // Skip-level edge - use dummy node positions for MultiSegment path
                let dummies = &dummy_positions[edge_idx];
                if dummies.is_empty() {
                    // Fallback to corner if no dummies (shouldn't happen)
                    let slot = if node_slots[layout_src_idx] != usize::MAX {
                        node_slots[layout_src_idx]
                    } else {
                        0
                    };
                    let hy = from_y + edge_start_row + slot;
                    edge_routing_ys.insert(hy);
                    if layout_from_level < level_routing_floor.len() {
                        level_routing_floor[layout_from_level] =
                            level_routing_floor[layout_from_level].max(hy);
                    }
                    EdgePath::Corner { horizontal_y: hy }
                } else {
                    // Build waypoints through the jogging dummies only —
                    // straight pass-throughs paint as part of a longer
                    // vertical segment and need no waypoint row.
                    let kept = &kept_wps[edge_idx];
                    let mut waypoints = Vec::with_capacity(dummies.len());
                    for (i, &(level, x)) in dummies.iter().enumerate() {
                        if !kept[i] {
                            continue;
                        }
                        // Assign a unique vertical slot for this edge at this level
                        let slot = level_edge_next[level];
                        level_edge_next[level] += 1;

                        let wp_edge_start_row =
                            max_node_height[level] + usize::from(level_labeled_src[level]);
                        let wp_y = level_y_offsets[level] + wp_edge_start_row + slot;
                        edge_routing_ys.insert(wp_y);
                        // Every kept waypoint bends right below its row (its
                        // x differs from the next column by construction).
                        let inter_corner_y = wp_y + 1;
                        edge_routing_ys.insert(inter_corner_y);
                        level_routing_floor[level] =
                            level_routing_floor[level].max(inter_corner_y);
                        waypoints.push((x, wp_y));
                    }

                    if waypoints.is_empty() {
                        // Fully straight chain: the reserved dummy columns
                        // line up with the target, so the edge is a plain
                        // vertical — or a single bend in the source band.
                        if from_x == to_x {
                            EdgePath::Direct
                        } else {
                            let slot = if node_slots[layout_src_idx] != usize::MAX {
                                node_slots[layout_src_idx]
                            } else {
                                0
                            };
                            let hy = from_y + edge_start_row + slot;
                            edge_routing_ys.insert(hy);
                            if layout_from_level < level_routing_floor.len() {
                                level_routing_floor[layout_from_level] =
                                    level_routing_floor[layout_from_level].max(hy);
                            }
                            EdgePath::Corner { horizontal_y: hy }
                        }
                    } else {
                        let slot = if node_slots[layout_src_idx] != usize::MAX {
                            node_slots[layout_src_idx]
                        } else {
                            0
                        };

                        // Calculate offset from (y+1)
                        let start_y_offset = (edge_start_row + slot).saturating_sub(1);

                        // Record the INITIAL corner Y (first segment routing) — the paint
                        // code draws a horizontal segment at from_y + 1 + start_y_offset,
                        // which is NOT a waypoint Y but still occupies a row.
                        let initial_corner_y = from_y + 1 + start_y_offset;
                        edge_routing_ys.insert(initial_corner_y);
                        if layout_from_level < level_routing_floor.len() {
                            level_routing_floor[layout_from_level] =
                                level_routing_floor[layout_from_level].max(initial_corner_y);
                        }

                        EdgePath::MultiSegment {
                            waypoints,
                            start_y_offset,
                        }
                    }
                }
            };

            // Label placement row layout (from_y = bottom row of source node):
            //   from_y:   [Source Node bottom]
            //   from_y+1: horizontal edge routing
            //   from_y+2: vertical connector
            //   from_y+3: label text row
            //   to_y:     [Target Node]
            let (label_x, label_y) = label.map(|lbl| {
                let label_len = lbl.chars().count() + 2; // +2 for quotes

                // First row below the source level's routing block — shared
                // with the CSR backend so label rows cannot drift.
                let label_y = from_y
                    + crate::algorithms::sugiyama::geometry::edge_label_row_offset(
                        level_occupied_slots[layout_from_level].len(),
                    );

                // Find the edge's X position at the label row
                let edge_x_at_label = match &path {
                    EdgePath::Direct => from_x,
                    EdgePath::Corner { horizontal_y } => {
                        // If label row is before the corner, edge is at from_x
                        // If label row is after the corner, edge is at to_x
                        if label_y <= *horizontal_y {
                            from_x
                        } else {
                            to_x
                        }
                    }
                    EdgePath::SideChannel {
                        channel_x, start_y, ..
                    } => {
                        // If before the horizontal segment, use from_x
                        // Otherwise use channel_x
                        if label_y < *start_y {
                            from_x
                        } else {
                            *channel_x
                        }
                    }
                    EdgePath::MultiSegment {
                        waypoints,
                        start_y_offset,
                    } => {
                        // Find which segment the label row falls into
                        // from_y is bottom of source node, +1 goes to routing area
                        let horizontal_y = from_y + 1 + start_y_offset;

                        if label_y <= horizontal_y || waypoints.is_empty() {
                            from_x
                        } else {
                            waypoints[0].0
                        }
                    }
                    EdgePath::Spline { .. } => from_x,
                };

                // Center the label on the edge's X position
                // Label goes through the line: the edge character is replaced by label
                let half_len = label_len / 2;
                let label_x = edge_x_at_label.saturating_sub(half_len);

                // Ensure label fits within width
                let clamped_x = if label_x + label_len > max_width {
                    max_width.saturating_sub(label_len)
                } else {
                    label_x
                };
                (clamped_x, label_y)
            })
            .unwrap_or((0, 0));

            let reversed = back_edges.get(edge_idx).copied().unwrap_or(false);
            builder.add_edge(LayoutEdge {
                from_id,
                to_id,
                from_x,
                from_y,
                to_x,
                to_y,
                path,
                edge_index: edge_idx,
                label,
                label_x,
                label_y,
                directed: true,
                reversed,
            });
        }
    }

    // Compute subgraph bounding boxes if any subgraphs are defined.
    // The canvas must cover every border: a label-widened cluster box can
    // extend past the node extent that `max_width` was derived from.
    let mut canvas_width = max_width;
    if dag.has_subgraphs() {
        let sg_infos = crate::algorithms::sugiyama::subgraph::compute_bounding_boxes(
            dag,
            &real_node_coords,
            &level_y_offsets,
            total_height,
            &edge_routing_ys,
            &level_routing_floor,
        );
        for info in sg_infos {
            canvas_width = canvas_width.max(info.x + info.width + 1);
            builder.add_subgraph(info);
        }
    }
    builder.set_dimensions(canvas_width, total_height);
    builder.set_direction(config.direction);

    let mut ir = builder.build();
    if config.direction == crate::graph::Direction::BottomUp {
        // Physical-space contract: IR coordinates match rendered cells.
        ir.flip_vertical();
    }
    ir
}

// ── Crossing reduction ───────────────────────────────────────────────────

/// Build a mapping from each node index to the edge indices it participates in.
/// This enables real nodes to find their skip-level edge dummies during crossing
/// reduction (the key fix for incomplete neighbor gathering).
fn build_node_edge_indices(dag: &Graph<'_>) -> Vec<Vec<usize>> {
    let mut node_edges: Vec<Vec<usize>> = vec![Vec::new(); dag.nodes.len()];
    for (edge_idx, &(from_id, to_id, _)) in dag.edges.iter().enumerate() {
        if let Some(from_idx) = dag.node_index(from_id) {
            node_edges[from_idx].push(edge_idx);
        }
        if let Some(to_idx) = dag.node_index(to_id) {
            node_edges[to_idx].push(edge_idx);
        }
    }
    node_edges
}

// ── X-coordinate refinement (median placement) ─────────────────────────

/// Refine x-coordinates by shifting nodes toward the median x of their
/// connected neighbors on adjacent levels. This is the "coordinate
/// assignment" step of the Sugiyama algorithm — it reduces zigzag edges
/// by aligning parents and children vertically.
///
/// Uses alternating down-sweeps (align with parents) and up-sweeps
/// (align with children) for several iterations. Each node is shifted
/// toward its ideal position, constrained by minimum spacing with its
/// neighbors on the same level.
fn refine_x_positions(
    dag: &Graph<'_>,
    virtual_levels: &[Vec<VNode>],
    x_coords: &mut [Vec<usize>],
    widths: &[Vec<usize>],
    node_edge_indices: &[Vec<usize>],
    node_spacing: usize,
) {
    use crate::algorithms::sugiyama::subgraph::vnode_subgraph;

    let num_levels = virtual_levels.len();

    if num_levels <= 1 {
        return;
    }

    use crate::algorithms::sugiyama::geometry::SG_GAP;
    const ITERATIONS: usize = 8;

    // Helper: compute minimum gap between adjacent nodes, accounting for
    // subgraph boundary padding.
    let gap_between = |level: usize, left_pos: usize, right_pos: usize| -> usize {
        let left_sg = vnode_subgraph(dag, &virtual_levels[level][left_pos]);
        let right_sg = vnode_subgraph(dag, &virtual_levels[level][right_pos]);
        if left_sg != right_sg {
            SG_GAP
        } else {
            node_spacing
        }
    };
    // Also enforce left margin for the first node if it's inside a subgraph
    let left_margin = |level: usize| -> usize {
        if virtual_levels[level].is_empty() {
            return 0;
        }
        let sg = vnode_subgraph(dag, &virtual_levels[level][0]);
        if sg.is_some() { 2 } else { 0 } // SUBGRAPH_H_PAD
    };

    // Helper: compute median center-x of connected neighbors on an adjacent level.
    // Returns None if no connections exist.
    let connected_median_x =
        |vnode: &VNode, adj_vnodes: &[VNode], adj_x: &[usize], adj_w: &[usize]| -> Option<usize> {
            // Build lookup: real node index → center-x, edge index → center-x
            let mut positions: Vec<usize> = Vec::new();

            match vnode {
                VNode::Real(idx) => {
                    for &edge_idx in &node_edge_indices[*idx] {
                        let &(from_id, to_id, _) = &dag.edges[edge_idx];

                        // Check real endpoints on adjacent level
                        for &nid in &[from_id, to_id] {
                            if let Some(nidx) = dag.node_index(nid) {
                                if nidx != *idx {
                                    // Find position of nidx on adjacent level
                                    for (p, av) in adj_vnodes.iter().enumerate() {
                                        if let VNode::Real(ri) = av {
                                            if *ri == nidx {
                                                positions.push(adj_x[p] + adj_w[p] / 2);
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Check dummy on adjacent level
                        for (p, av) in adj_vnodes.iter().enumerate() {
                            if let VNode::Dummy { edge_idx: ei } = av {
                                if *ei == edge_idx {
                                    positions.push(adj_x[p] + adj_w[p] / 2);
                                }
                            }
                        }
                    }
                }
                VNode::Dummy { edge_idx } => {
                    let &(from_id, to_id, _) = &dag.edges[*edge_idx];

                    // Check same-edge dummy on adjacent level
                    for (p, av) in adj_vnodes.iter().enumerate() {
                        if let VNode::Dummy { edge_idx: ei } = av {
                            if *ei == *edge_idx {
                                positions.push(adj_x[p] + adj_w[p] / 2);
                            }
                        }
                    }

                    // Check real endpoints on adjacent level
                    for &nid in &[from_id, to_id] {
                        if let Some(nidx) = dag.node_index(nid) {
                            for (p, av) in adj_vnodes.iter().enumerate() {
                                if let VNode::Real(ri) = av {
                                    if *ri == nidx {
                                        positions.push(adj_x[p] + adj_w[p] / 2);
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if positions.is_empty() {
                return None;
            }
            positions.sort_unstable();
            let median = if positions.len() % 2 == 1 {
                positions[positions.len() / 2]
            } else {
                let mid = positions.len() / 2;
                (positions[mid - 1] + positions[mid]) / 2
            };
            Some(median)
        };

    // Helper: compute and apply the shift for one node on a level.
    let shift_node = |x_coords: &mut [Vec<usize>],
                      level: usize,
                      pos: usize,
                      target_center: usize,
                      widths: &[Vec<usize>],
                      gap_fn: &dyn Fn(usize, usize, usize) -> usize,
                      margin_fn: &dyn Fn(usize) -> usize| {
        let n = x_coords[level].len();
        let my_w = widths[level][pos];
        let target_x = target_center.saturating_sub(my_w / 2);

        let min_x = if pos == 0 {
            margin_fn(level)
        } else {
            let g = gap_fn(level, pos - 1, pos);
            x_coords[level][pos - 1] + widths[level][pos - 1] + g
        };
        let max_x = if pos + 1 < n {
            let g = gap_fn(level, pos, pos + 1);
            x_coords[level][pos + 1].saturating_sub(my_w + g)
        } else {
            usize::MAX
        };

        x_coords[level][pos] = target_x.max(min_x).min(max_x);
    };

    for _iter in 0..ITERATIONS {
        // Down sweep: align with parents (level above)
        for level in 1..num_levels {
            let adj_level = level - 1;
            let n = virtual_levels[level].len();

            // Pass 1: right-to-left — allows nodes to cascade leftward
            for pos in (0..n).rev() {
                if let Some(tc) = connected_median_x(
                    &virtual_levels[level][pos],
                    &virtual_levels[adj_level],
                    &x_coords[adj_level],
                    &widths[adj_level],
                ) {
                    shift_node(x_coords, level, pos, tc, widths, &gap_between, &left_margin);
                }
            }

            // Pass 2: left-to-right — allows nodes to cascade rightward
            for pos in 0..n {
                if let Some(tc) = connected_median_x(
                    &virtual_levels[level][pos],
                    &virtual_levels[adj_level],
                    &x_coords[adj_level],
                    &widths[adj_level],
                ) {
                    shift_node(x_coords, level, pos, tc, widths, &gap_between, &left_margin);
                }
            }
        }

        // Up sweep: align with children (level below)
        for level in (0..num_levels - 1).rev() {
            let adj_level = level + 1;
            let n = virtual_levels[level].len();

            // Pass 1: right-to-left
            for pos in (0..n).rev() {
                if let Some(tc) = connected_median_x(
                    &virtual_levels[level][pos],
                    &virtual_levels[adj_level],
                    &x_coords[adj_level],
                    &widths[adj_level],
                ) {
                    shift_node(x_coords, level, pos, tc, widths, &gap_between, &left_margin);
                }
            }

            // Pass 2: left-to-right
            for pos in 0..n {
                if let Some(tc) = connected_median_x(
                    &virtual_levels[level][pos],
                    &virtual_levels[adj_level],
                    &x_coords[adj_level],
                    &widths[adj_level],
                ) {
                    shift_node(x_coords, level, pos, tc, widths, &gap_between, &left_margin);
                }
            }
        }
    }
}

/// Compact subgraphs by pulling member nodes toward the subgraph centroid.
///
/// After x-refinement, nodes may be far from their subgraph siblings because
/// they followed cross-subgraph edge connections. This pass identifies such
/// outliers and shifts them inward, respecting same-level gap constraints.
/// When a member is blocked by non-member neighbors, it cascades the push
/// to those neighbors to make room.
fn compact_subgraphs(
    dag: &Graph<'_>,
    virtual_levels: &[Vec<VNode>],
    x_coords: &mut [Vec<usize>],
    widths: &[Vec<usize>],
    node_spacing: usize,
) {
    use crate::algorithms::sugiyama::subgraph::vnode_subgraph;

    use crate::algorithms::sugiyama::geometry::SG_GAP;

    // Helper: minimum gap between two adjacent positions on a level.
    let gap_between = |level: usize, left_pos: usize, right_pos: usize| -> usize {
        let left_sg = vnode_subgraph(dag, &virtual_levels[level][left_pos]);
        let right_sg = vnode_subgraph(dag, &virtual_levels[level][right_pos]);
        if left_sg != right_sg {
            SG_GAP
        } else {
            node_spacing
        }
    };

    // Cascading push: shift node at `pos` on `level` to `target_x`, and push
    // all neighbors in `direction` as needed to maintain gap constraints.
    // direction: -1 = push left, +1 = push right
    let cascade_push = |x_coords: &mut [Vec<usize>],
                        level: usize,
                        pos: usize,
                        target_x: usize,
                        direction: isize| {
        x_coords[level][pos] = target_x;
        if direction < 0 {
            // Push leftward: fix pos-1, pos-2, ... if they overlap
            let mut i = pos;
            while i > 0 {
                let g = gap_between(level, i - 1, i);
                let needed = x_coords[level][i].saturating_sub(widths[level][i - 1] + g);
                if x_coords[level][i - 1] <= needed {
                    break; // no overlap
                }
                // Can't push past left edge
                let margin = if i - 1 == 0 {
                    let sg = vnode_subgraph(dag, &virtual_levels[level][0]);
                    if sg.is_some() { 2 } else { 0 }
                } else {
                    0
                };
                x_coords[level][i - 1] = needed.max(margin);
                i -= 1;
            }
        } else {
            // Push rightward: fix pos+1, pos+2, ...
            let n = x_coords[level].len();
            let mut i = pos;
            while i + 1 < n {
                let g = gap_between(level, i, i + 1);
                let needed = x_coords[level][i] + widths[level][i] + g;
                if x_coords[level][i + 1] >= needed {
                    break;
                }
                x_coords[level][i + 1] = needed;
                i += 1;
            }
        }
    };

    let sg_ids: Vec<usize> = dag.subgraphs.iter().map(|s| s.id).collect();

    for &sg_id in &sg_ids {
        let mut members: Vec<(usize, usize)> = Vec::new();

        for (level, vnodes) in virtual_levels.iter().enumerate() {
            for (pos, vnode) in vnodes.iter().enumerate() {
                if vnode_subgraph(dag, vnode) == Some(sg_id) {
                    members.push((level, pos));
                }
            }
        }

        if members.len() <= 1 {
            continue;
        }

        // Compute centroid from real nodes only (dummies follow edges and
        // shouldn't anchor the center).
        let real_members: Vec<(usize, usize)> = members
            .iter()
            .filter(|&&(l, p)| matches!(virtual_levels[l][p], VNode::Real(_)))
            .copied()
            .collect();
        let centroid_members = if real_members.is_empty() {
            &members
        } else {
            &real_members
        };
        let sum: usize = centroid_members
            .iter()
            .map(|&(l, p)| x_coords[l][p] + widths[l][p] / 2)
            .sum();
        let centroid = sum / centroid_members.len();

        // Sort members by distance from centroid (farthest first).
        let mut by_distance: Vec<(usize, usize, usize)> = members
            .iter()
            .map(|&(l, p)| {
                let cx = x_coords[l][p] + widths[l][p] / 2;
                let dist = cx.abs_diff(centroid);
                (l, p, dist)
            })
            .collect();
        by_distance.sort_by(|a, b| b.2.cmp(&a.2));

        for &(level, pos, dist) in &by_distance {
            if dist < SG_GAP {
                continue; // close enough, don't bother
            }

            let my_w = widths[level][pos];
            let my_cx = x_coords[level][pos] + my_w / 2;
            let target_x = centroid.saturating_sub(my_w / 2);

            // Simple constraint check (no cascading) first.
            let n = x_coords[level].len();
            let min_x = if pos == 0 {
                let sg = vnode_subgraph(dag, &virtual_levels[level][0]);
                if sg.is_some() { 2 } else { 0 }
            } else {
                let g = gap_between(level, pos - 1, pos);
                x_coords[level][pos - 1] + widths[level][pos - 1] + g
            };
            let max_x = if pos + 1 < n {
                let g = gap_between(level, pos, pos + 1);
                x_coords[level][pos + 1].saturating_sub(my_w + g)
            } else {
                usize::MAX
            };

            let simple_x = target_x.max(min_x).min(max_x);

            // Check if simple move is sufficient.
            let simple_ok = (my_cx > centroid && simple_x < x_coords[level][pos])
                || (my_cx < centroid && simple_x > x_coords[level][pos]);

            if simple_ok {
                x_coords[level][pos] = simple_x;
            } else if my_cx > centroid && target_x < min_x {
                // Need to push left neighbors leftward to make room.
                // Only cascade for real nodes to avoid over-disturbing the layout.
                if matches!(virtual_levels[level][pos], VNode::Real(_)) {
                    // Limit how far we push: at most half the distance to centroid
                    let push_target = (x_coords[level][pos] + target_x) / 2;
                    cascade_push(x_coords, level, pos, push_target, -1);
                }
            } else if my_cx < centroid && target_x > max_x {
                // Need to push right neighbors rightward.
                if matches!(virtual_levels[level][pos], VNode::Real(_)) {
                    let push_target = (x_coords[level][pos] + target_x) / 2;
                    cascade_push(x_coords, level, pos, push_target, 1);
                }
            }
        }
    }
}

/// Crossing reduction for virtual levels (includes dummy nodes).
///
/// Dispatches through the provided [`CrossingReducer`] pipeline.  Each reducer
/// (Median / AdjacentExchange) runs its configured number of passes, each
/// consisting of a top-down sweep followed by a bottom-up sweep.  The
/// pipeline runs uniformly regardless of graph size — behaviour is controlled
/// only by the user-facing presets or manual configuration.
fn reduce_crossings_virtual(
    dag: &Graph<'_>,
    levels: &mut [Vec<VNode>],
    _node_levels: &[usize],
    max_level: usize,
    crossing_pipeline: &[CrossingReducer],
) {
    // Build edge lookup for complete neighbor gathering (real + dummy).
    // This lets real nodes discover skip-level edge dummies on the adjacent
    // level, which is critical for correct median and adjacent-exchange.
    let node_edge_indices = build_node_edge_indices(dag);

    // Pre-allocate reusable buffers to avoid allocations in the hot loop
    let max_level_size = levels.iter().map(|l| l.len()).max().unwrap_or(0);

    let mut real_pos: HashMap<usize, usize> = HashMap::new();
    let mut dummy_pos: HashMap<usize, usize> = HashMap::new();

    // Reusable buffers for median computation
    let mut node_medians: Vec<(VNode, f32)> = Vec::with_capacity(max_level_size);
    let mut connected_positions: Vec<usize> = Vec::with_capacity(8);

    // Reusable buffers for adjacent exchange
    let mut u_positions: Vec<usize> = Vec::with_capacity(8);
    let mut v_positions: Vec<usize> = Vec::with_capacity(8);

    for reducer in crossing_pipeline {
        match reducer {
            CrossingReducer::Median(passes) => {
                for _pass in 0..*passes {
                    // Top-down pass
                    for level_idx in 1..=max_level {
                        let (prev_levels, rest) = levels.split_at_mut(level_idx);
                        let parent_level = &prev_levels[level_idx - 1];
                        order_virtual_by_median(
                            dag,
                            &mut rest[0],
                            parent_level,
                            true,
                            &mut real_pos,
                            &mut dummy_pos,
                            &mut node_medians,
                            &mut connected_positions,
                            &node_edge_indices,
                        );
                    }
                    // Bottom-up pass
                    for level_idx in (0..max_level).rev() {
                        let (left, right) = levels.split_at_mut(level_idx + 1);
                        let child_level = &right[0];
                        order_virtual_by_median(
                            dag,
                            &mut left[level_idx],
                            child_level,
                            false,
                            &mut real_pos,
                            &mut dummy_pos,
                            &mut node_medians,
                            &mut connected_positions,
                            &node_edge_indices,
                        );
                    }
                }
            }
            CrossingReducer::AdjacentExchange(passes) => {
                for _pass in 0..*passes {
                    // Top-down pass
                    for level_idx in 1..=max_level {
                        let (prev_levels, rest) = levels.split_at_mut(level_idx);
                        let parent_level = &prev_levels[level_idx - 1];
                        adjacent_exchange_virtual(
                            dag,
                            &mut rest[0],
                            parent_level,
                            true,
                            &mut real_pos,
                            &mut dummy_pos,
                            &mut u_positions,
                            &mut v_positions,
                            &node_edge_indices,
                        );
                    }
                    // Bottom-up pass
                    for level_idx in (0..max_level).rev() {
                        let (left, right) = levels.split_at_mut(level_idx + 1);
                        let child_level = &right[0];
                        adjacent_exchange_virtual(
                            dag,
                            &mut left[level_idx],
                            child_level,
                            false,
                            &mut real_pos,
                            &mut dummy_pos,
                            &mut u_positions,
                            &mut v_positions,
                            &node_edge_indices,
                        );
                    }
                }
            }
        }
    }
}

/// Adjacent exchange on virtual-node levels: swap adjacent pairs if it reduces crossings.
fn adjacent_exchange_virtual(
    dag: &Graph<'_>,
    level_nodes: &mut [VNode],
    adj_level: &[VNode],
    use_parents: bool,
    real_pos: &mut HashMap<usize, usize>,
    dummy_pos: &mut HashMap<usize, usize>,
    u_positions: &mut Vec<usize>,
    v_positions: &mut Vec<usize>,
    node_edge_indices: &[Vec<usize>],
) {
    if level_nodes.len() < 2 {
        return;
    }

    // Build position maps for the adjacent level
    real_pos.clear();
    dummy_pos.clear();

    for (pos, vnode) in adj_level.iter().enumerate() {
        match vnode {
            VNode::Real(idx) => {
                real_pos.insert(*idx, pos);
            }
            VNode::Dummy { edge_idx } => {
                dummy_pos.insert(*edge_idx, pos);
            }
        }
    }

    for i in 0..level_nodes.len() - 1 {
        u_positions.clear();
        v_positions.clear();

        // Gather neighbour positions for node at position i
        gather_vnode_positions(
            dag,
            &level_nodes[i],
            use_parents,
            real_pos,
            dummy_pos,
            node_edge_indices,
            u_positions,
        );

        // Gather neighbour positions for node at position i+1
        gather_vnode_positions(
            dag,
            &level_nodes[i + 1],
            use_parents,
            real_pos,
            dummy_pos,
            node_edge_indices,
            v_positions,
        );

        let (cross_uv, cross_vu) = count_crossings_pair(u_positions, v_positions);
        if cross_vu < cross_uv {
            level_nodes.swap(i, i + 1);
        }
    }
}

/// Gather positions of a VNode's neighbours in the adjacent level.
///
/// For real nodes, checks ALL edges (both direct and skip-level) by looking up
/// both `real_pos` (for the other endpoint) and `dummy_pos` (for intermediate
/// dummy nodes). This ensures skip-level edges are properly accounted for in
/// crossing reduction.
///
/// For dummy nodes, checks both endpoints of the edge (handling back edges
/// correctly) and the same edge's dummy on the adjacent level.
#[inline]
fn gather_vnode_positions(
    dag: &Graph<'_>,
    vnode: &VNode,
    _use_parents: bool,
    real_pos: &HashMap<usize, usize>,
    dummy_pos: &HashMap<usize, usize>,
    node_edge_indices: &[Vec<usize>],
    out: &mut Vec<usize>,
) {
    match (vnode.real_index(), vnode.dummy_edge()) {
        (Some(idx), _) => {
            // Real node: check all edges this node participates in.
            // For each edge, look for the other endpoint OR a dummy on the adjacent level.
            // The position maps (real_pos/dummy_pos) are built from the adjacent level,
            // so only entries actually present on that level will be found.
            for &edge_idx in &node_edge_indices[idx] {
                let &(from_id, to_id, _) = &dag.edges[edge_idx];

                // Check if the other real endpoint is on the adjacent level
                if let Some(from_idx) = dag.node_index(from_id) {
                    if from_idx != idx {
                        if let Some(&rp) = real_pos.get(&from_idx) {
                            out.push(rp);
                        }
                    }
                }
                if let Some(to_idx) = dag.node_index(to_id) {
                    if to_idx != idx {
                        if let Some(&rp) = real_pos.get(&to_idx) {
                            out.push(rp);
                        }
                    }
                }

                // Check if this edge has a dummy on the adjacent level
                if let Some(&dp) = dummy_pos.get(&edge_idx) {
                    out.push(dp);
                }
            }
        }
        (_, Some(edge_idx)) => {
            let &(from_id, to_id, _) = &dag.edges[edge_idx];

            // Check for same edge's dummy in adjacent level
            if let Some(&dpos) = dummy_pos.get(&edge_idx) {
                out.push(dpos);
            }

            // Check for BOTH real endpoints in adjacent level.
            // This correctly handles back edges where from/to are reversed
            // relative to the layout direction.
            if let Some(fidx) = dag.node_index(from_id) {
                if let Some(&rpos) = real_pos.get(&fidx) {
                    out.push(rpos);
                }
            }
            if let Some(tidx) = dag.node_index(to_id) {
                if let Some(&rpos) = real_pos.get(&tidx) {
                    out.push(rpos);
                }
            }
        }
        _ => {}
    }
}

/// Order virtual nodes by median position of connected nodes in adjacent level.
///
/// Reuses pre-allocated buffers to avoid per-pass allocations in the hot loop.
fn order_virtual_by_median(
    dag: &Graph<'_>,
    level_nodes: &mut Vec<VNode>,
    adj_level: &[VNode],
    _use_parents: bool,
    real_pos: &mut HashMap<usize, usize>,
    dummy_pos: &mut HashMap<usize, usize>,
    node_medians: &mut Vec<(VNode, f32)>,
    connected_positions: &mut Vec<usize>,
    node_edge_indices: &[Vec<usize>],
) {
    // Clear and rebuild lookup tables (reuses allocated capacity)
    real_pos.clear();
    dummy_pos.clear();

    for (pos, vnode) in adj_level.iter().enumerate() {
        match vnode {
            VNode::Real(idx) => {
                real_pos.insert(*idx, pos);
            }
            VNode::Dummy { edge_idx } => {
                dummy_pos.insert(*edge_idx, pos);
            }
        }
    }

    // Clear and rebuild medians (reuses allocated capacity)
    node_medians.clear();

    for (pos, &vnode) in level_nodes.iter().enumerate() {
        // Clear and reuse connected_positions
        connected_positions.clear();

        match (vnode.real_index(), vnode.dummy_edge()) {
            (Some(idx), _) => {
                // Real node: check all edges for connections on adjacent level
                for &edge_idx in &node_edge_indices[idx] {
                    let &(from_id, to_id, _) = &dag.edges[edge_idx];

                    // Check other real endpoint
                    if let Some(from_idx) = dag.node_index(from_id) {
                        if from_idx != idx {
                            if let Some(&p) = real_pos.get(&from_idx) {
                                connected_positions.push(p);
                            }
                        }
                    }
                    if let Some(to_idx) = dag.node_index(to_id) {
                        if to_idx != idx {
                            if let Some(&p) = real_pos.get(&to_idx) {
                                connected_positions.push(p);
                            }
                        }
                    }

                    // Check for dummy on adjacent level
                    if let Some(&dp) = dummy_pos.get(&edge_idx) {
                        connected_positions.push(dp);
                    }
                }
            }
            (_, Some(edge_idx)) => {
                // Dummy node - find the connected node or dummy for this edge
                let &(from_id, to_id, _) = &dag.edges[edge_idx];

                // Check for same edge's dummy in adjacent level
                if let Some(&dpos) = dummy_pos.get(&edge_idx) {
                    connected_positions.push(dpos);
                }

                // Check both real endpoints (handles back edges correctly)
                if let Some(fidx) = dag.node_index(from_id) {
                    if let Some(&rpos) = real_pos.get(&fidx) {
                        connected_positions.push(rpos);
                    }
                }
                if let Some(tidx) = dag.node_index(to_id) {
                    if let Some(&rpos) = real_pos.get(&tidx) {
                        connected_positions.push(rpos);
                    }
                }
            }
            _ => {}
        };

        let median = if connected_positions.is_empty() {
            pos as f32
        } else {
            connected_positions.sort_unstable();
            if connected_positions.len() % 2 == 1 {
                connected_positions[connected_positions.len() / 2] as f32
            } else {
                let mid = connected_positions.len() / 2;
                (connected_positions[mid - 1] + connected_positions[mid]) as f32 / 2.0
            }
        };

        node_medians.push((vnode, median));
    }

    node_medians.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());

    // Rebuild level_nodes from sorted medians
    level_nodes.clear();
    for (v, _) in node_medians.iter() {
        level_nodes.push(*v);
    }
}
