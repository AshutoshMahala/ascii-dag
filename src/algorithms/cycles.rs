//! Cycle detection algorithms for directed graphs.
//!
//! This module provides functionality to detect cycles in a DAG,
//! which would make it not a valid DAG.
//!
//! ## Generic Cycle Detection
//!
//! For more flexibility, see the [`generic`] submodule which provides
//! cycle detection that works with any data structure through higher-order
//! functions or traits.
//!
//! ```
//! # #[cfg(feature = "generic")]
//! # {
//! use ascii_dag::algorithms::cycles::generic::detect_cycle_fn;
//!
//! // Example: Error chain
//! let get_caused_by = |error_id: &usize| -> Vec<usize> {
//!     match error_id {
//!         1 => vec![2],
//!         2 => vec![3],
//!         3 => vec![],
//!         _ => vec![],
//!     }
//! };
//!
//! let all_errors = vec![1, 2, 3];
//! let cycle = detect_cycle_fn(&all_errors, get_caused_by);
//! assert!(cycle.is_none()); // No cycle
//! # }
//! ```

#[cfg(feature = "generic")]
pub mod generic;

#[cfg(feature = "alloc")]
use crate::graph::Graph;
#[cfg(feature = "alloc")]
use alloc::{vec, vec::Vec};

/// Three-color DFS states for back-edge detection.
#[cfg(feature = "alloc")]
const WHITE: u8 = 0; // Not yet visited
#[cfg(feature = "alloc")]
const GRAY: u8 = 1;  // On the current DFS stack (ancestor)
#[cfg(feature = "alloc")]
const BLACK: u8 = 2; // Fully processed

#[cfg(feature = "alloc")]
impl<'a> Graph<'a> {
    /// Detect back edges using three-color DFS (zigraph parity).
    ///
    /// Returns a `Vec<bool>` of length `self.edges.len()`, where `true` marks
    /// an edge as a back edge. The graph is **not** mutated.
    ///
    /// Self-loops (from == to) are always marked as back edges.
    ///
    /// # Algorithm
    ///
    /// Classic three-color DFS: WHITE → GRAY (on stack) → BLACK (done).
    /// An edge whose target is GRAY is a back edge. Handles disconnected
    /// graphs by iterating over all roots.
    pub fn detect_back_edges(&self) -> Vec<bool> {
        let n = self.nodes.len();
        let e = self.edges.len();
        let mut color = vec![WHITE; n];
        let mut back = vec![false; e];

        // Mark self-loops immediately
        for (ei, &(from, to, _)) in self.edges.iter().enumerate() {
            if from == to {
                back[ei] = true;
            }
        }

        // Build edge-index lookup: for each node index, the edge indices where it is the source
        let mut edges_from: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (ei, &(from, _, _)) in self.edges.iter().enumerate() {
            if let Some(idx) = self.node_index(from) {
                edges_from[idx].push(ei);
            }
        }

        // Explicit-stack DFS for each unvisited root
        for start in 0..n {
            if color[start] != WHITE {
                continue;
            }

            // Stack entries: (node_index, edge_iterator_position)
            let mut stack: Vec<(usize, usize)> = Vec::new();
            color[start] = GRAY;
            stack.push((start, 0));

            while let Some(&mut (node_idx, ref mut ei_pos)) = stack.last_mut() {
                let edge_list = &edges_from[node_idx];
                if *ei_pos < edge_list.len() {
                    let edge_idx = edge_list[*ei_pos];
                    // Advance iterator before processing (so we don't re-visit)
                    *ei_pos += 1;

                    let (_, to_id, _) = self.edges[edge_idx];
                    if let Some(to_idx) = self.node_index(to_id) {
                        match color[to_idx] {
                            GRAY => {
                                // Target is an ancestor on the current path → back edge
                                back[edge_idx] = true;
                            }
                            WHITE => {
                                color[to_idx] = GRAY;
                                stack.push((to_idx, 0));
                            }
                            _ => {} // BLACK — already fully processed, skip
                        }
                    }
                } else {
                    // All edges from this node exhausted
                    color[node_idx] = BLACK;
                    stack.pop();
                }
            }
        }

        back
    }

    /// Check if the graph contains cycles (making it not a valid DAG).
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::Graph;
    ///
    /// let mut dag = Graph::new();
    /// dag.add_node(1, "A");
    /// dag.add_node(2, "B");
    /// dag.add_edge(1, 2, None);
    /// dag.add_edge(2, 1, None);  // Creates a cycle!
    ///
    /// assert!(dag.has_cycle());
    /// ```
    pub fn has_cycle(&self) -> bool {
        let n = self.nodes.len();
        // Three-color DFS using pre-built adjacency lists → O(V+E).
        // Uses explicit stack to avoid recursion-depth limits on deep graphs.
        const WHITE: u8 = 0;
        const GRAY: u8 = 1;
        const BLACK: u8 = 2;

        let mut color = vec![WHITE; n];

        for start in 0..n {
            if color[start] != WHITE {
                continue;
            }

            // Explicit-stack DFS: (node_idx, position in children iterator)
            let mut stack: Vec<(usize, usize)> = Vec::new();
            color[start] = GRAY;
            stack.push((start, 0));

            while let Some(&mut (node_idx, ref mut child_pos)) = stack.last_mut() {
                let children = &self.children[node_idx];
                if *child_pos < children.len() {
                    let child_idx = children[*child_pos];
                    *child_pos += 1;

                    match color[child_idx] {
                        GRAY => return true,   // back edge → cycle
                        WHITE => {
                            color[child_idx] = GRAY;
                            stack.push((child_idx, 0));
                        }
                        _ => {} // BLACK — already fully processed
                    }
                } else {
                    color[node_idx] = BLACK;
                    stack.pop();
                }
            }
        }
        false
    }

    /// Find a cycle path in the graph.
    ///
    /// Returns the node IDs that form a cycle, if one exists.
    pub(crate) fn find_cycle_path(&self) -> Option<Vec<usize>> {
        for i in 0..self.nodes.len() {
            let mut visited = vec![false; self.nodes.len()];
            let mut path = Vec::new();

            if let Some(cycle) = self.find_cycle_from(i, &mut visited, &mut path) {
                return Some(cycle);
            }
        }
        None
    }

    /// Helper function to find a cycle starting from a specific node.
    fn find_cycle_from(
        &self,
        start_idx: usize,
        visited: &mut [bool],
        path: &mut Vec<usize>,
    ) -> Option<Vec<usize>> {
        if visited[start_idx] {
            // Found a cycle - extract it from path
            if let Some(cycle_start) = path.iter().position(|&idx| idx == start_idx) {
                return Some(
                    path[cycle_start..]
                        .iter()
                        .map(|&idx| self.nodes[idx].0)
                        .collect(),
                );
            }
            return None;
        }

        visited[start_idx] = true;
        path.push(start_idx);

        let node_id = self.nodes[start_idx].0;
        for &(from, to, _) in &self.edges {
            if from == node_id {
                // O(1) HashMap lookup instead of O(n) scan
                if let Some(child_idx) = self.node_index(to)
                    && let Some(cycle) = self.find_cycle_from(child_idx, visited, path)
                {
                    return Some(cycle);
                }
            }
        }

        path.pop();
        None
    }
}

#[cfg(test)]
mod tests {
    use crate::graph::Graph;

    #[test]
    fn test_cycle_detection() {
        let mut dag = Graph::new();
        dag.add_node(1, "A");
        dag.add_node(2, "B");
        dag.add_edge(1, 2, None);
        dag.add_edge(2, 1, None); // Cycle!

        assert!(dag.has_cycle());
    }

    #[test]
    fn test_no_cycle() {
        let dag = Graph::from_edges(&[(1, "A"), (2, "B")], &[(1, 2)]);

        assert!(!dag.has_cycle());
    }

    #[test]
    fn test_cycle_with_auto_created_nodes() {
        let mut dag = Graph::new();
        dag.add_node(1, "A");
        // Node 2 will be auto-created
        dag.add_edge(1, 2, None);
        dag.add_edge(2, 1, None); // Creates cycle

        assert!(dag.has_cycle());
    }
}
