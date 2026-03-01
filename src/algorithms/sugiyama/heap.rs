//! Heap-based Sugiyama layout pipeline.
//!
//! Implements the full layout algorithm using standard heap allocations (`Vec`, `HashMap`).
//! This is the default layout path for `DAG::compute_layout()`.
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
//! # Relationship to Arena Path
//!
//! The arena-based layout in `layout/arena.rs` implements the same algorithm
//! using arena allocation and `Idx`-typed indices. The two paths produce
//! visually compatible output but operate on different type systems.

use crate::graph::DAG;
use crate::ir::{EdgePath, LayoutEdge, LayoutIRBuilder, LayoutIR, LayoutNode};
use alloc::vec;
use alloc::vec::Vec;

#[cfg(not(feature = "std"))]
use alloc::collections::BTreeMap as HashMap;
#[cfg(feature = "std")]
use std::collections::HashMap;

// ── VNode: Virtual node for layout computation ───────────────────────────

/// Virtual node — either a real graph node or a dummy inserted for edge routing.
///
/// During layout, skip-level edges (spanning more than one level) are broken
/// into segments by inserting dummy nodes at each intermediate level. This
/// allows crossing reduction and x-coordinate assignment to treat all nodes
/// uniformly.
#[derive(Clone, Copy)]
pub(crate) enum VNode {
    /// A real node from the input graph, identified by its index in `DAG.nodes`.
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

/// Compute the heap-based layout IR for a DAG.
///
/// This is the implementation behind `DAG::compute_layout()`. Returns a
/// renderer-agnostic `LayoutIR` containing node positions, edge routes,
/// and dimensional information.
pub(crate) fn compute_layout<'a>(dag: &DAG<'a>) -> LayoutIR<'a> {
    if dag.nodes.is_empty() {
        return LayoutIRBuilder::new().build();
    }

    // Check for cycles - can't layout cyclic graphs
    if dag.has_cycle() {
        return LayoutIRBuilder::new().build();
    }

    // Step 1: Calculate levels for real nodes
    let level_data = dag.calculate_levels();
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
    for (edge_idx, &(from_id, to_id, _label)) in dag.edges.iter().enumerate() {
        if let (Some(from_idx), Some(to_idx)) =
            (dag.node_index(from_id), dag.node_index(to_id))
        {
            let from_level = node_levels[from_idx];
            let to_level = node_levels[to_idx];

            if to_level > from_level + 1 {
                // Skip-level edge - insert dummy nodes at intermediate levels
                for level in (from_level + 1)..to_level {
                    virtual_levels[level].push(VNode::Dummy { edge_idx });
                }
            }
        }
    }

    // Step 3: Apply crossing reduction WITH dummy nodes included
    reduce_crossings_virtual(dag, &mut virtual_levels, &node_levels, max_level);

    // Step 4: Assign x-coordinates to virtual nodes
    let mut x_coords: Vec<Vec<usize>> = Vec::with_capacity(virtual_levels.len());
    let mut widths: Vec<Vec<usize>> = Vec::with_capacity(virtual_levels.len());

    for level_vnodes in &virtual_levels {
        let mut level_x = Vec::with_capacity(level_vnodes.len());
        let mut level_w = Vec::with_capacity(level_vnodes.len());
        let mut x = 0;

        for vnode in level_vnodes {
            let width = match vnode {
                VNode::Real(idx) => dag.get_node_width(*idx),
                VNode::Dummy { .. } => 3, // Width 3 for visual separation between parallel edges
            };
            level_x.push(x);
            level_w.push(width);
            x += width + 3; // Standard spacing
        }

        x_coords.push(level_x);
        widths.push(level_w);
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
    let max_width = level_widths.iter().max().unwrap_or(&0) + 4 + label_margin;

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

    // Apply centering offsets to real node coordinates
    for (level_idx, level_vnodes) in virtual_levels.iter().enumerate() {
        let level_width = level_widths[level_idx];
        let level_offset = if max_width > level_width {
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

    let mut node_slots = vec![usize::MAX; dag.nodes.len()];
    let mut edge_slots = vec![0usize; dag.edges.len()];

    let mut level_occupied_slots: Vec<Vec<Vec<(usize, usize)>>> =
        vec![Vec::new(); max_level + 1];

    // Maximum horizontal routing rows per level.
    // Gives full visual separation for typical fan-in (≤8 sources),
    // and graceful degradation (shared rows) for extreme fan-in.
    const MAX_SLOTS_PER_LEVEL: usize = 8;

    // 1. Assign slots greedy
    for (i, &(from_id, to_id, _)) in dag.edges.iter().enumerate() {
        if let (Some(from_idx), Some(to_idx)) =
            (dag.node_index(from_id), dag.node_index(to_id))
        {
            let from_level = node_levels[from_idx];
            let to_level = node_levels[to_idx];

            // Get coordinates to determine geometry
            let (_, _, from_x, from_w) = real_node_coords[from_idx];
            let (_, _, to_x, to_w) = real_node_coords[to_idx];

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
                    if node_slots[from_idx] != usize::MAX {
                        // Reuse existing slot for this source node
                        let slot = node_slots[from_idx];
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

                            let collide =
                                occupied.iter().any(|&(s, e)| s < max_x && e > min_x);
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

                        node_slots[from_idx] = slot;
                    }
                    edge_slots[i] = node_slots[from_idx];
                }
            }
        }
    }

    // Calculate per-level heights: base + extra rows for slot separation
    let base_lines = if has_labeled_edges { 5 } else { 3 };
    let mut level_y_offsets = Vec::with_capacity(max_level + 1);
    let mut current_offset = 0;

    for level in 0..=max_level {
        level_y_offsets.push(current_offset);

        // 1. Slots for edges originating at this level (adjacent or skip)
        let adjacent_slots = level_occupied_slots[level].len();

        // 2. Slots for edges passing through (dummy nodes)
        let skip_slots = if level < virtual_levels.len() {
            virtual_levels[level]
                .iter()
                .filter(|v| matches!(v, VNode::Dummy { .. }))
                .count()
        } else {
            0
        };

        // Determine max slots needed for this specific level
        let slots_needed = adjacent_slots.max(skip_slots);
        let extra_lines = slots_needed.saturating_sub(1);

        let height = base_lines + extra_lines;
        current_offset += height;
    }

    let total_height = current_offset;

    // Add real nodes to IR
    for (level_idx, level_vnodes) in virtual_levels.iter().enumerate() {
        for vnode in level_vnodes {
            if let VNode::Real(idx) = vnode {
                let (level, pos, x, width) = real_node_coords[*idx];
                let y = level_y_offsets[level];

                let (id, label) = dag.nodes[*idx];
                builder.add_node(LayoutNode {
                    id,
                    label,
                    y,
                    x,
                    width,
                    center_x: x + width / 2,
                    level: level_idx,
                    level_position: pos,
                });
            }
        }
    }

    // Collect dummy node X positions for skip-level edge routing
    let mut dummy_positions: Vec<Vec<(usize, usize)>> = vec![Vec::new(); dag.edges.len()];
    for (level_idx, level_vnodes) in virtual_levels.iter().enumerate() {
        let level_width = level_widths[level_idx];
        let level_offset = if max_width > level_width {
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

    let mut level_edge_next = vec![0usize; max_level + 1];

    // Step 6: Add edges with proper routing
    for (edge_idx, &(from_id, to_id, label)) in dag.edges.iter().enumerate() {
        if let (Some(from_idx), Some(to_idx)) =
            (dag.node_index(from_id), dag.node_index(to_id))
        {
            let from_level = node_levels[from_idx];
            let to_level = node_levels[to_idx];

            let (_, _, from_x_base, from_width) = real_node_coords[from_idx];
            let (_, _, to_x_base, to_width) = real_node_coords[to_idx];

            let from_x = from_x_base + from_width / 2;
            let to_x = to_x_base + to_width / 2;
            // Calculate Y positions based on variable level heights
            let from_y = level_y_offsets[from_level];
            let to_y = level_y_offsets[to_level];

            // Horizontal edges start at row 1 below the node
            let edge_start_row = 1;

            let path = if to_level == from_level + 1 {
                // Adjacent levels - direct or corner connection
                if from_x == to_x {
                    EdgePath::Direct
                } else {
                    // Get horizontal slot for this source at this level
                    let slot = if node_slots[from_idx] != usize::MAX {
                        node_slots[from_idx]
                    } else {
                        0
                    };

                    EdgePath::Corner {
                        horizontal_y: from_y + edge_start_row + slot,
                    }
                }
            } else {
                // Skip-level edge - use dummy node positions for MultiSegment path
                let dummies = &dummy_positions[edge_idx];
                if dummies.is_empty() {
                    // Fallback to corner if no dummies (shouldn't happen)
                    let slot = if node_slots[from_idx] != usize::MAX {
                        node_slots[from_idx]
                    } else {
                        0
                    };
                    EdgePath::Corner {
                        horizontal_y: from_y + edge_start_row + slot,
                    }
                } else {
                    // Build waypoints through dummy nodes with Y slot separation
                    let mut waypoints = Vec::with_capacity(dummies.len());
                    for &(level, x) in dummies {
                        // Assign a unique vertical slot for this edge at this level
                        let slot = level_edge_next[level];
                        level_edge_next[level] += 1;

                        let edge_start_row = if has_labeled_edges { 2 } else { 1 };
                        waypoints.push((x, level_y_offsets[level] + edge_start_row + slot));
                    }

                    let slot = if node_slots[from_idx] != usize::MAX {
                        node_slots[from_idx]
                    } else {
                        0
                    };

                    // Calculate offset from (y+1)
                    let start_y_offset = (edge_start_row + slot).saturating_sub(1);

                    EdgePath::MultiSegment {
                        waypoints,
                        start_y_offset,
                    }
                }
            };

            // Label placement row layout:
            //   from_y+0: [Source Node]
            //   from_y+1: horizontal edge routing
            //   from_y+2: vertical connector
            //   from_y+3: label text row
            //   to_y:     [Target Node]
            let label_position = label.map(|lbl| {
                let label_len = lbl.chars().count() + 2; // +2 for quotes

                // Label Y at from_y + 3 (dedicated label row with base_lines=5)
                let label_y = from_y + 3;

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
                        let horizontal_y = from_y + 1 + start_y_offset;

                        if label_y <= horizontal_y || waypoints.is_empty() {
                            from_x
                        } else {
                            waypoints[0].0
                        }
                    }
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
            });

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
                label_position,
            });
        }
    }

    builder.set_dimensions(max_width, total_height);
    builder.build()
}

// ── Crossing reduction ───────────────────────────────────────────────────

/// Crossing reduction for virtual levels (includes dummy nodes).
/// Uses median heuristic with both real and dummy nodes participating.
fn reduce_crossings_virtual(
    dag: &DAG<'_>,
    levels: &mut [Vec<VNode>],
    _node_levels: &[usize],
    max_level: usize,
) {
    // Pre-allocate reusable buffers to avoid allocations in the hot loop
    // Estimate max level size for capacity hints
    let max_level_size = levels.iter().map(|l| l.len()).max().unwrap_or(0);

    // Reusable lookup tables (use new() for BTreeMap compatibility in no_std)
    let mut real_pos: HashMap<usize, usize> = HashMap::new();
    let mut dummy_pos: HashMap<usize, usize> = HashMap::new();

    // Reusable buffers for median computation
    let mut node_medians: Vec<(VNode, f32)> = Vec::with_capacity(max_level_size);
    let mut connected_positions: Vec<usize> = Vec::with_capacity(8); // Most nodes have few connections

    // Run multiple passes of median heuristic
    for _ in 0..dag.crossing_reduction_passes {
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
            );
        }
    }
}

/// Order virtual nodes by median position of connected nodes in adjacent level.
///
/// Reuses pre-allocated buffers to avoid per-pass allocations in the hot loop.
fn order_virtual_by_median(
    dag: &DAG<'_>,
    level_nodes: &mut Vec<VNode>,
    adj_level: &[VNode],
    use_parents: bool,
    real_pos: &mut HashMap<usize, usize>,
    dummy_pos: &mut HashMap<usize, usize>,
    node_medians: &mut Vec<(VNode, f32)>,
    connected_positions: &mut Vec<usize>,
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
                // Real node - find connected real nodes in adjacent level
                let connected_indices = if use_parents {
                    dag.get_parents_indices(idx)
                } else {
                    dag.get_children_indices(idx)
                };

                // O(1) lookup per connected node
                for &conn_idx in connected_indices {
                    if let Some(&p) = real_pos.get(&conn_idx) {
                        connected_positions.push(p);
                    }
                }
            }
            (_, Some(edge_idx)) => {
                // Dummy node - find the connected node or dummy for this edge
                let &(from_id, to_id, _) = &dag.edges[edge_idx];
                let from_idx = dag.node_index(from_id);
                let to_idx = dag.node_index(to_id);

                // Check for same edge's dummy in adjacent level
                if let Some(&dpos) = dummy_pos.get(&edge_idx) {
                    connected_positions.push(dpos);
                }

                // Check for real endpoint in adjacent level
                if use_parents {
                    if let Some(fidx) = from_idx {
                        if let Some(&rpos) = real_pos.get(&fidx) {
                            connected_positions.push(rpos);
                        }
                    }
                } else {
                    if let Some(tidx) = to_idx {
                        if let Some(&rpos) = real_pos.get(&tidx) {
                            connected_positions.push(rpos);
                        }
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
