//! Graph layout algorithms for hierarchical rendering.
//!
//! This module implements the Sugiyama layered graph layout algorithm
//! for positioning nodes in a hierarchical DAG visualization.
//!
//! ## Submodules
//!
//! - [`generic`] - Generic topological sorting for any data structure (requires `generic` feature)
//! - [`arena`] - Arena-based layout computation for `no_std`/embedded

#[cfg(feature = "generic")]
pub mod generic;

pub mod arena;

use crate::graph::DAG;
use alloc::{vec, vec::Vec};

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
                if let Some(from_idx) = self.node_index(from)
                    && let Some(to_idx) = self.node_index(to)
                {
                    let new_level = levels[from_idx] + 1;
                    if new_level > levels[to_idx] {
                        levels[to_idx] = new_level;
                        changed = true;
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
                if let Some(from_idx) = self.node_index(from)
                    && let Some(to_idx) = self.node_index(to)
                {
                    let new_level = levels[from_idx] + 1;
                    if new_level > levels[to_idx] {
                        levels[to_idx] = new_level;
                        changed = true;
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
        // Iterate a variable number of times based on configuration
        for _ in 0..self.crossing_reduction_passes {
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
    /// Optimized: uses index-based lookups to avoid allocations.
    #[inline]
    fn order_by_median_parents(&self, level_nodes: &mut Vec<usize>, parent_level: &[usize]) {
        let mut node_medians: Vec<(usize, f32)> = Vec::with_capacity(level_nodes.len());

        for (pos, &idx) in level_nodes.iter().enumerate() {
            let parent_indices = self.get_parents_indices(idx);

            if parent_indices.is_empty() {
                node_medians.push((idx, pos as f32));
            } else {
                // Find positions of parents in the parent level
                let mut parent_positions: Vec<usize> = parent_indices
                    .iter()
                    .filter_map(|&p_idx| parent_level.iter().position(|&i| i == p_idx))
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
    /// Optimized: uses index-based lookups to avoid allocations.
    #[inline]
    fn order_by_median_children(&self, level_nodes: &mut Vec<usize>, child_level: &[usize]) {
        let mut node_medians: Vec<(usize, f32)> = Vec::with_capacity(level_nodes.len());

        for (pos, &idx) in level_nodes.iter().enumerate() {
            let child_indices = self.get_children_indices(idx);

            if child_indices.is_empty() {
                node_medians.push((idx, pos as f32));
            } else {
                // Find positions of children in the child level
                let mut child_positions: Vec<usize> = child_indices
                    .iter()
                    .filter_map(|&c_idx| child_level.iter().position(|&i| i == c_idx))
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
    #[inline]
    fn collect_connected(&self, start_idx: usize, visited: &mut [bool], subgraph: &mut Vec<usize>) {
        let mut stack = Vec::new();
        stack.push(start_idx);

        while let Some(idx) = stack.pop() {
            if visited[idx] {
                continue;
            }
            visited[idx] = true;
            subgraph.push(idx);

        let node_id = self.nodes[idx].0;

            // Follow edges in both directions
            for &(from, to) in &self.edges {
                if from == node_id {
                    if let Some(child_idx) = self.node_index(to) {
                        if !visited[child_idx] {
                            stack.push(child_idx);
                        }
                    }
                }
                if to == node_id {
                    if let Some(parent_idx) = self.node_index(from) {
                        if !visited[parent_idx] {
                            stack.push(parent_idx);
                        }
                    }
                }
            }
        }
    }

    /// Check if a subgraph is a simple chain (no branching).
    /// Optimized to avoid allocations.
    pub(crate) fn is_subgraph_simple_chain(&self, subgraph_indices: &[usize]) -> bool {
        for &idx in subgraph_indices {
            if self.parents_count(idx) > 1 || self.children_count(idx) > 1 {
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
