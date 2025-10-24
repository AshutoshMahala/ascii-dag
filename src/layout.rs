//! Graph layout algorithms for hierarchical rendering.
//!
//! This module implements the Sugiyama layered graph layout algorithm
//! for positioning nodes in a hierarchical DAG visualization.
//!
//! ## Submodules
//!
//! - [`generic`] - Generic topological sorting for any data structure (requires `generic` feature)

#[cfg(feature = "generic")]
pub mod generic;

use crate::graph::DAG;
use alloc::vec::Vec;

impl<'a> DAG<'a> {
    /// Calculate hierarchical levels for all nodes in the graph.
    ///
    /// Uses a fixed-point algorithm to assign each node to a level,
    /// where a node's level is one more than the maximum level of its parents.
    pub(crate) fn calculate_levels(&self) -> Vec<(usize, usize)> {
        let mut levels = vec![0usize; self.nodes.len()];
        let mut changed = true;

        while changed {
            changed = false;
            for &(from, to) in &self.edges {
                // Guard against missing nodes - O(1) HashMap lookups
                if let Some(from_idx) = self.node_index(from) {
                    if let Some(to_idx) = self.node_index(to) {
                        let new_level = levels[from_idx] + 1;
                        if new_level > levels[to_idx] {
                            levels[to_idx] = new_level;
                            changed = true;
                        }
                    }
                }
            }
        }

        levels.into_iter().enumerate().collect()
    }

    /// Calculate levels for a specific subgraph.
    pub(crate) fn calculate_levels_for_subgraph(
        &self,
        subgraph_indices: &[usize],
    ) -> Vec<(usize, usize)> {
        let subgraph_node_ids: Vec<usize> = subgraph_indices
            .iter()
            .map(|&idx| self.nodes[idx].0)
            .collect();

        let mut levels = vec![0usize; self.nodes.len()];
        let mut changed = true;

        while changed {
            changed = false;
            for &(from, to) in &self.edges {
                // Only process edges within this subgraph
                if !subgraph_node_ids.contains(&from) || !subgraph_node_ids.contains(&to) {
                    continue;
                }

                // Guard against missing nodes - O(1) HashMap lookups
                if let Some(from_idx) = self.node_index(from) {
                    if let Some(to_idx) = self.node_index(to) {
                        let new_level = levels[from_idx] + 1;
                        if new_level > levels[to_idx] {
                            levels[to_idx] = new_level;
                            changed = true;
                        }
                    }
                }
            }
        }

        subgraph_indices
            .iter()
            .map(|&idx| (idx, levels[idx]))
            .collect()
    }

    /// PASS 1: Reduce edge crossings using median heuristic.
    ///
    /// Applies the Sugiyama crossing reduction algorithm by iteratively
    /// reordering nodes within levels to minimize edge crossings.
    pub(crate) fn reduce_crossings(&self, levels: &mut [Vec<usize>], max_level: usize) {
        // Iterate a few times for better results (diminishing returns after 4-5 iterations)
        for _ in 0..4 {
            // Top-down pass: order nodes by median of parents
            for level_idx in 1..=max_level {
                // Split borrows to avoid clone
                let (prev_levels, rest) = levels.split_at_mut(level_idx);
                let parent_level = &prev_levels[level_idx - 1];
                self.order_by_median_parents(&mut rest[0], parent_level);
            }

            // Bottom-up pass: order nodes by median of children
            for level_idx in (0..max_level).rev() {
                // Split borrows to avoid clone
                let (left, right) = levels.split_at_mut(level_idx + 1);
                let child_level = &right[0];
                self.order_by_median_children(&mut left[level_idx], child_level);
            }
        }
    }

    /// Order nodes by median position of their parents.
    fn order_by_median_parents(&self, level_nodes: &mut Vec<usize>, parent_level: &[usize]) {
        let mut node_medians: Vec<(usize, f32)> = Vec::new();

        for (pos, &idx) in level_nodes.iter().enumerate() {
            let node_id = self.nodes[idx].0;
            let parents = self.get_parents(node_id);

            if parents.is_empty() {
                node_medians.push((idx, pos as f32));
            } else {
                // Find positions of parents in the parent level
                let mut parent_positions: Vec<usize> = parents
                    .iter()
                    .filter_map(|&p_id| parent_level.iter().position(|&i| self.nodes[i].0 == p_id))
                    .collect();
                parent_positions.sort_unstable();

                let median = if parent_positions.is_empty() {
                    pos as f32
                } else if parent_positions.len() % 2 == 1 {
                    parent_positions[parent_positions.len() / 2] as f32
                } else {
                    let mid = parent_positions.len() / 2;
                    (parent_positions[mid - 1] + parent_positions[mid]) as f32 / 2.0
                };

                node_medians.push((idx, median));
            }
        }

        // Sort by median
        node_medians.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        *level_nodes = node_medians.iter().map(|(idx, _)| *idx).collect();
    }

    /// Order nodes by median position of their children.
    fn order_by_median_children(&self, level_nodes: &mut Vec<usize>, child_level: &[usize]) {
        let mut node_medians: Vec<(usize, f32)> = Vec::new();

        for (pos, &idx) in level_nodes.iter().enumerate() {
            let node_id = self.nodes[idx].0;
            let children = self.get_children(node_id);

            if children.is_empty() {
                node_medians.push((idx, pos as f32));
            } else {
                // Find positions of children in the child level
                let mut child_positions: Vec<usize> = children
                    .iter()
                    .filter_map(|&c_id| child_level.iter().position(|&i| self.nodes[i].0 == c_id))
                    .collect();
                child_positions.sort_unstable();

                let median = if child_positions.is_empty() {
                    pos as f32
                } else if child_positions.len() % 2 == 1 {
                    child_positions[child_positions.len() / 2] as f32
                } else {
                    let mid = child_positions.len() / 2;
                    (child_positions[mid - 1] + child_positions[mid]) as f32 / 2.0
                };

                node_medians.push((idx, median));
            }
        }

        node_medians.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        *level_nodes = node_medians.iter().map(|(idx, _)| *idx).collect();
    }

    /// PASS 2: Assign x-coordinates to each node (character-level positioning).
    ///
    /// Positions nodes horizontally to minimize edge length while
    /// maintaining the ordering from crossing reduction.
    pub(crate) fn assign_x_coordinates(
        &self,
        levels: &mut [Vec<usize>],
        max_level: usize,
    ) -> Vec<usize> {
        let mut x_coords = vec![0usize; self.nodes.len()];

        // Start with left-to-right layout within each level, preserving crossing reduction order
        for level_nodes in levels.iter() {
            let mut x = 0;
            for &idx in level_nodes.iter() {
                x_coords[idx] = x;
                let width = self.get_node_width(idx);
                x += width + 3;
            }
        }

        // Refine positions using median of connected nodes to center under parents/over children
        // But maintain relative order within levels
        for _ in 0..2 {
            // Top-down: center under parents where possible
            for level_idx in 1..=max_level {
                for &idx in &levels[level_idx] {
                    let node_id = self.nodes[idx].0;
                    let parents = self.get_parents(node_id);

                    if !parents.is_empty() {
                        let mut parent_centers: Vec<usize> = Vec::new();
                        for &p_id in &parents {
                            // O(1) HashMap lookup instead of O(n) scan
                            if let Some(p_idx) = self.node_index(p_id) {
                                let width = self.get_node_width(p_idx); // Use cached width
                                parent_centers.push(x_coords[p_idx] + width / 2);
                            }
                        }

                        if !parent_centers.is_empty() {
                            parent_centers.sort_unstable();
                            let median = parent_centers[parent_centers.len() / 2];
                            let width = self.get_node_width(idx);
                            // Shift toward median but don't reorder
                            let target = median.saturating_sub(width / 2);
                            x_coords[idx] = (x_coords[idx] + target) / 2;
                        }
                    }
                }

                // Re-compact this level to remove overlaps and reorder to match x-coords
                self.compact_level(&mut x_coords, &mut levels[level_idx]);
            }
        }

        x_coords
    }

    /// Compact a level to remove overlaps and reorder nodes left-to-right by x-coordinate.
    pub(crate) fn compact_level(&self, x_coords: &mut [usize], level_nodes: &mut Vec<usize>) {
        if level_nodes.is_empty() {
            return;
        }

        // Sort nodes by their current x position
        let mut sorted: Vec<_> = level_nodes
            .iter()
            .map(|&idx| (x_coords[idx], idx))
            .collect();
        sorted.sort_by_key(|(x, _)| *x);

        // Reassign x-coords to remove overlaps and update level_nodes order
        level_nodes.clear();
        let mut x = 0;
        for (_, idx) in sorted {
            level_nodes.push(idx);
            x_coords[idx] = x;
            let width = self.get_node_width(idx);
            x += width + 3;
        }
    }

    /// PASS 3: Calculate canvas dimensions.
    ///
    /// Determines the width needed for each level and the overall canvas.
    pub(crate) fn calculate_canvas_dimensions(
        &self,
        levels: &[Vec<usize>],
        x_coords: &[usize],
    ) -> (Vec<usize>, usize) {
        let mut level_widths = Vec::new();
        let mut max_width = 0;

        for level_nodes in levels {
            if level_nodes.is_empty() {
                level_widths.push(0);
                continue;
            }

            let min_x = level_nodes
                .iter()
                .map(|&idx| x_coords[idx])
                .min()
                .unwrap_or(0);
            let max_node_idx = level_nodes
                .iter()
                .max_by_key(|&&idx| x_coords[idx])
                .unwrap();
            let width = self.get_node_width(*max_node_idx);
            let level_width = (x_coords[*max_node_idx] - min_x) + width;

            level_widths.push(level_width);
            max_width = max_width.max(level_width);
        }

        (level_widths, max_width)
    }

    /// Find disconnected subgraphs in the DAG.
    pub(crate) fn find_subgraphs(&self) -> Vec<Vec<usize>> {
        let mut visited = vec![false; self.nodes.len()];
        let mut subgraphs = Vec::new();

        for i in 0..self.nodes.len() {
            if !visited[i] {
                let mut subgraph = Vec::new();
                self.collect_connected(i, &mut visited, &mut subgraph);
                subgraphs.push(subgraph);
            }
        }

        subgraphs
    }

    /// Collect all nodes connected to the given node (helper for find_subgraphs).
    fn collect_connected(&self, start_idx: usize, visited: &mut [bool], subgraph: &mut Vec<usize>) {
        if visited[start_idx] {
            return;
        }

        visited[start_idx] = true;
        subgraph.push(start_idx);

        let node_id = self.nodes[start_idx].0;

        // Follow edges in both directions
        for &(from, to) in &self.edges {
            if from == node_id {
                // O(1) HashMap lookup instead of O(n) scan
                if let Some(child_idx) = self.node_index(to) {
                    self.collect_connected(child_idx, visited, subgraph);
                }
            }
            if to == node_id {
                // O(1) HashMap lookup instead of O(n) scan
                if let Some(parent_idx) = self.node_index(from) {
                    self.collect_connected(parent_idx, visited, subgraph);
                }
            }
        }
    }

    /// Check if a subgraph is a simple chain (no branching).
    pub(crate) fn is_subgraph_simple_chain(&self, subgraph_indices: &[usize]) -> bool {
        for &idx in subgraph_indices {
            let node_id = self.nodes[idx].0;
            let parents = self.get_parents(node_id);
            let children = self.get_children(node_id);

            if parents.len() > 1 || children.len() > 1 {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use crate::graph::DAG;

    #[test]
    fn test_calculate_levels() {
        let dag = DAG::from_edges(&[(1, "A"), (2, "B"), (3, "C")], &[(1, 2), (2, 3)]);

        let levels = dag.calculate_levels();

        // Find levels for each node
        let level_map: std::collections::HashMap<_, _> = levels
            .into_iter()
            .map(|(idx, level)| (dag.nodes[idx].0, level))
            .collect();

        assert_eq!(level_map[&1], 0); // Root
        assert_eq!(level_map[&2], 1);
        assert_eq!(level_map[&3], 2);
    }

    #[test]
    fn test_diamond_layout() {
        let dag = DAG::from_edges(
            &[(1, "Top"), (2, "Left"), (3, "Right"), (4, "Bottom")],
            &[(1, 2), (1, 3), (2, 4), (3, 4)],
        );

        let levels = dag.calculate_levels();
        let level_map: std::collections::HashMap<_, _> = levels
            .into_iter()
            .map(|(idx, level)| (dag.nodes[idx].0, level))
            .collect();

        assert_eq!(level_map[&1], 0); // Top
        assert_eq!(level_map[&2], 1); // Left and Right
        assert_eq!(level_map[&3], 1);
        assert_eq!(level_map[&4], 2); // Bottom
    }
}
