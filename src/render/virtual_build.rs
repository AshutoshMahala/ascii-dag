//! Virtual layout construction for DAG visualization.
//!
//! Builds a virtual layout with dummy nodes for skip-level edge routing,
//! assigns x-coordinates, repositions dummies, and groups edges by level.

use crate::graph::DAG;
use alloc::{vec, vec::Vec};

use super::ascii::{VirtualLayout, VirtualNode};

impl<'a> DAG<'a> {
    /// Build a virtual layout with dummy nodes for skip-level edges.
    /// Memory: O(N + E*D) where N=nodes, E=skip edges, D=avg level span
    pub(super) fn build_virtual_layout(&self) -> VirtualLayout {
        // Step 1: Calculate levels for real nodes
        let level_data = self.calculate_levels();
        let max_level = level_data.iter().map(|(_, l)| *l).max().unwrap_or(0);

        // Create level mapping for real nodes
        let mut node_levels: Vec<usize> = vec![0; self.nodes.len()];
        for (idx, level) in &level_data {
            node_levels[*idx] = *level;
        }

        // Step 2: Group real nodes by level
        let mut levels: Vec<Vec<VirtualNode>> = vec![Vec::new(); max_level + 1];
        for (idx, level) in &level_data {
            levels[*level].push(VirtualNode::real(*idx));
        }

        // Step 3: Identify skip-level edges and insert dummy nodes
        // We use the edge_idx in Dummy nodes so we can find them after reordering
        for (edge_idx, &(from_id, to_id, _)) in self.edges.iter().enumerate() {
            if let Some(from_idx) = self.node_index(from_id)
                && let Some(to_idx) = self.node_index(to_id)
            {
                let from_level = node_levels[from_idx];
                let to_level = node_levels[to_idx];

                if to_level > from_level + 1 {
                    // This is a skip edge - insert dummy nodes at intermediate levels
                    for level in &mut levels[(from_level + 1)..to_level] {
                        level.push(VirtualNode::dummy(edge_idx));
                    }
                }
            }
        }

        // Step 4: Apply crossing reduction on virtual levels
        // Convert to indices for the existing reduce_crossings logic
        let mut real_levels: Vec<Vec<usize>> = levels
            .iter()
            .map(|level| level.iter().filter_map(|vn| vn.real_index()).collect())
            .collect();

        self.reduce_crossings(&mut real_levels, max_level);

        // Rebuild levels with proper ordering (real nodes in optimized order)
        // Position dummies intelligently based on their source node's position
        for (level_idx, real_order) in real_levels.iter().enumerate() {
            let dummies: Vec<_> = levels[level_idx]
                .iter()
                .filter(|vn| !vn.is_real())
                .copied()
                .collect();

            levels[level_idx].clear();
            for &idx in real_order {
                levels[level_idx].push(VirtualNode::real(idx));
            }
            // Dummies will be repositioned after we have x-coordinates
            levels[level_idx].extend(dummies);
        }

        // Step 4b: Reposition dummies to align with their skip-edge source nodes
        // This creates visually cleaner vertical paths for skip edges
        self.reposition_dummies(&mut levels, &node_levels);

        // Step 5: Assign x-coordinates
        let x_coords = self.assign_virtual_x_coordinates(&levels, &node_levels);

        // Step 6: Build edge list grouped by source level for O(1) lookup during rendering
        let edges_by_level = self.build_virtual_edges_by_level(&levels, &node_levels);

        VirtualLayout {
            levels,
            x_coords,
            edges_by_level,
        }
    }

    /// Assign x-coordinates to virtual nodes (real + dummy).
    /// Real nodes get sequential x-coordinates.
    /// Dummy nodes get x-coordinates aligned with their source node for visual continuity.
    pub(super) fn assign_virtual_x_coordinates(
        &self,
        levels: &[Vec<VirtualNode>],
        node_levels: &[usize],
    ) -> Vec<Vec<usize>> {
        let mut x_coords: Vec<Vec<usize>> = Vec::with_capacity(levels.len());
        // Track widths locally for x-coordinate calculation (not stored in VirtualLayout)
        let mut widths: Vec<Vec<usize>> = Vec::with_capacity(levels.len());

        // First pass: assign x-coordinates to all nodes sequentially
        for level_nodes in levels {
            let mut level_x = Vec::with_capacity(level_nodes.len());
            let mut level_w = Vec::with_capacity(level_nodes.len());
            let mut x = 0;

            for vnode in level_nodes {
                let width = if vnode.is_real() {
                    self.get_node_width(vnode.index())
                } else {
                    1
                };

                level_x.push(x);
                level_w.push(width);
                x += width + 3; // Standard spacing for all nodes
            }

            x_coords.push(level_x);
            widths.push(level_w);
        }

        // Second pass: adjust dummy x-coordinates to align with their source node's center
        // Collect all dummy adjustments first
        let mut dummy_adjustments: Vec<(usize, usize, usize)> = Vec::new(); // (level_idx, dummy_pos, target_x)

        for (edge_idx, &(from_id, to_id, _)) in self.edges.iter().enumerate() {
            if let Some(from_idx) = self.node_index(from_id)
                && let Some(to_idx) = self.node_index(to_id)
            {
                let from_level = node_levels[from_idx];
                let to_level = node_levels[to_idx];

                if to_level > from_level + 1 {
                    // This is a skip edge - find source node's center x
                    if let Some(source_pos) = levels[from_level]
                        .iter()
                        .position(|vn| vn.is_real() && vn.index() == from_idx)
                    {
                        let source_center_x =
                            x_coords[from_level][source_pos] + widths[from_level][source_pos] / 2;

                        // Queue each dummy's x adjustment
                        for level_idx in (from_level + 1)..to_level {
                            if let Some(dummy_pos) = levels[level_idx]
                                .iter()
                                .position(|vn| vn.is_dummy() && vn.index() == edge_idx)
                            {
                                dummy_adjustments.push((level_idx, dummy_pos, source_center_x));
                            }
                        }
                    }
                }
            }
        }

        // Apply adjustments - for each level, set dummy x-coord and push any nodes that would overlap
        for (level_idx, dummy_pos, target_x) in dummy_adjustments {
            // Find the end position of the node just before the dummy (if any)
            let prev_end = if dummy_pos > 0 {
                x_coords[level_idx][dummy_pos - 1] + widths[level_idx][dummy_pos - 1] + 3
            } else {
                0
            };

            // Dummy goes at target_x, but at least after the previous node
            let dummy_x = target_x.max(prev_end);
            x_coords[level_idx][dummy_pos] = dummy_x;

            // Push any subsequent nodes to avoid overlap
            let mut min_next_x = dummy_x + 1 + 3; // dummy width (1) + spacing (3)
            for pos in (dummy_pos + 1)..levels[level_idx].len() {
                if x_coords[level_idx][pos] < min_next_x {
                    x_coords[level_idx][pos] = min_next_x;
                }
                // Update minimum x for next node
                min_next_x = x_coords[level_idx][pos] + widths[level_idx][pos] + 3;
            }
        }

        x_coords
    }

    /// Reposition dummy nodes in the level arrays.
    /// Note: The actual x-coordinate alignment happens in assign_virtual_x_coordinates.
    #[allow(clippy::needless_range_loop)]
    pub(super) fn reposition_dummies(&self, levels: &mut [Vec<VirtualNode>], node_levels: &[usize]) {
        // For each skip edge, find where its source node is positioned and place
        // its dummies right after that position in each intermediate level
        for (edge_idx, &(from_id, to_id, _)) in self.edges.iter().enumerate() {
            if let Some(from_idx) = self.node_index(from_id)
                && let Some(to_idx) = self.node_index(to_id)
            {
                let from_level = node_levels[from_idx];
                let to_level = node_levels[to_idx];

                if to_level > from_level + 1 {
                    // This is a skip edge - reposition its dummies

                    // Find where the source node is in its level
                    let source_pos = levels[from_level]
                        .iter()
                        .position(|vn| vn.is_real() && vn.index() == from_idx);

                    if let Some(src_pos) = source_pos {
                        // For each intermediate level, move this edge's dummy to after src_pos
                        for level_idx in (from_level + 1)..to_level {
                            // Find and remove the dummy for this edge
                            if let Some(dummy_pos) = levels[level_idx]
                                .iter()
                                .position(|vn| vn.is_dummy() && vn.index() == edge_idx)
                            {
                                let dummy = levels[level_idx].remove(dummy_pos);

                                // Insert it right after the source position (but clamped to valid range)
                                // Count how many real nodes are in this level
                                let real_count =
                                    levels[level_idx].iter().filter(|vn| vn.is_real()).count();

                                // Insert position: try to align with source, but after real nodes
                                // if source position is beyond what we have
                                let insert_pos = if src_pos < real_count {
                                    // Insert right after the equivalent position
                                    src_pos + 1
                                } else {
                                    // Source position is beyond this level's real nodes,
                                    // insert at end of real nodes
                                    real_count
                                };

                                // Insert, clamping to valid range
                                let insert_pos = insert_pos.min(levels[level_idx].len());
                                levels[level_idx].insert(insert_pos, dummy);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Build edges grouped by source level for O(1) lookup during rendering.
    /// Returns edges_by_level[level] = [(from_pos, to_pos), ...] for edges from level to level+1.
    pub(super) fn build_virtual_edges_by_level(
        &self,
        levels: &[Vec<VirtualNode>],
        node_levels: &[usize],
    ) -> Vec<Vec<(usize, usize)>> {
        // Initialize empty vec for each level (except last which has no outgoing edges)
        let mut edges_by_level: Vec<Vec<(usize, usize)>> = vec![Vec::new(); levels.len()];

        // Process each DAG edge
        for (edge_idx, &(from_id, to_id, _)) in self.edges.iter().enumerate() {
            if let Some(from_idx) = self.node_index(from_id)
                && let Some(to_idx) = self.node_index(to_id)
            {
                let from_level = node_levels[from_idx];
                let to_level = node_levels[to_idx];

                if to_level == from_level + 1 {
                    // Direct adjacent edge
                    if let Some(from_pos) = levels[from_level]
                        .iter()
                        .position(|vn| vn.is_real() && vn.index() == from_idx)
                        && let Some(to_pos) = levels[to_level]
                            .iter()
                            .position(|vn| vn.is_real() && vn.index() == to_idx)
                    {
                        edges_by_level[from_level].push((from_pos, to_pos));
                    }
                } else if to_level > from_level + 1 {
                    // Skip edge - route through dummies identified by edge_idx
                    // Find source position
                    if let Some(from_pos) = levels[from_level]
                        .iter()
                        .position(|vn| vn.is_real() && vn.index() == from_idx)
                    {
                        // Find first dummy at from_level + 1
                        if let Some(first_dummy_pos) = levels[from_level + 1]
                            .iter()
                            .position(|vn| vn.is_dummy() && vn.index() == edge_idx)
                        {
                            // Edge from source to first dummy
                            edges_by_level[from_level].push((from_pos, first_dummy_pos));

                            // Edges between consecutive dummies
                            for level in (from_level + 1)..(to_level - 1) {
                                if let Some(curr_pos) = levels[level]
                                    .iter()
                                    .position(|vn| vn.is_dummy() && vn.index() == edge_idx)
                                    && let Some(next_pos) = levels[level + 1]
                                        .iter()
                                        .position(|vn| vn.is_dummy() && vn.index() == edge_idx)
                                {
                                    edges_by_level[level].push((curr_pos, next_pos));
                                }
                            }

                            // Edge from last dummy to target
                            if let Some(last_dummy_pos) = levels[to_level - 1]
                                .iter()
                                .position(|vn| vn.is_dummy() && vn.index() == edge_idx)
                                && let Some(to_pos) = levels[to_level]
                                    .iter()
                                    .position(|vn| vn.is_real() && vn.index() == to_idx)
                            {
                                edges_by_level[to_level - 1].push((last_dummy_pos, to_pos));
                            }
                        }
                    }
                }
            }
        }

        edges_by_level
    }
}
