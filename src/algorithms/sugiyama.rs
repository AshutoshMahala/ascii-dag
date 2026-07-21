//! Sugiyama hierarchical layout algorithm.
//!
//! Implements level assignment, crossing reduction, and coordinate assignment
//! for positioning DAG nodes in a layered hierarchy.
//!
//! ## Submodules
//!
//! - `arena` - Arena-based layout computation for `no_std`/embedded
//! - `heap` - Heap-based layout computation (produces `LayoutIR`)
//! - `idx` - Configurable index types for memory optimization (requires `arena` feature)

pub(crate) mod arena_csr;
pub mod config;
pub mod crossing;
pub(crate) mod geometry;
#[cfg(feature = "alloc")]
pub(crate) mod heap;
#[cfg(feature = "alloc")]
pub(crate) mod subgraph;

#[cfg(feature = "arena")]
pub mod idx;

#[cfg(feature = "alloc")]
use crate::graph::Graph;
#[cfg(feature = "alloc")]
use alloc::{vec, vec::Vec};

#[cfg(feature = "alloc")]
#[allow(deprecated)]
impl<'a> Graph<'a> {
    /// Calculate hierarchical levels for all nodes in the graph.
    ///
    /// Uses a fixed-point algorithm to assign each node to a level,
    /// where a node's level is one more than the maximum level of its parents.
    #[cfg(test)]
    pub(crate) fn calculate_levels(&self) -> Vec<(usize, usize)> {
        self.calculate_levels_with_back_edges(&[])
    }

    /// Calculate node levels, treating back edges as reversed.
    ///
    /// Back edges (identified by index in `back_edges`) have their direction
    /// flipped for level assignment: the target becomes the "parent" and the
    /// source becomes the "child". This breaks cycles so the fixed-point
    /// iteration converges.
    pub(crate) fn calculate_levels_with_back_edges(
        &self,
        back_edges: &[bool],
    ) -> Vec<(usize, usize)> {
        let mut levels = vec![0usize; self.nodes.len()];
        let mut changed = true;

        while changed {
            changed = false;
            for (ei, &(from, to, _)) in self.edges.iter().enumerate() {
                // Skip self-loops (they can't affect level assignment)
                if from == to {
                    continue;
                }
                let is_back = back_edges.get(ei).copied().unwrap_or(false);
                // For back edges, flip direction: treat `to → from`
                let (src, dst) = if is_back { (to, from) } else { (from, to) };

                if let Some(src_idx) = self.node_index(src)
                    && let Some(dst_idx) = self.node_index(dst)
                {
                    let new_level = levels[src_idx] + 1;
                    if new_level > levels[dst_idx] {
                        levels[dst_idx] = new_level;
                        changed = true;
                    }
                }
            }
        }

        levels.into_iter().enumerate().collect()
    }

    /// Find disconnected components in the DAG.
    pub(crate) fn find_connected_components(&self) -> Vec<Vec<usize>> {
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

    /// Collect all nodes connected to the given node (helper for find_connected_components).
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
            for &(from, to, _) in &self.edges {
                if from == node_id
                    && let Some(child_idx) = self.node_index(to)
                    && !visited[child_idx]
                {
                    stack.push(child_idx);
                }
                if to == node_id
                    && let Some(parent_idx) = self.node_index(from)
                    && !visited[parent_idx]
                {
                    stack.push(parent_idx);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::graph::Graph;

    #[test]
    fn test_calculate_levels() {
        let dag = Graph::from_edges(&[(1, "A"), (2, "B"), (3, "C")], &[(1, 2), (2, 3)]);

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
        let dag = Graph::from_edges(
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
