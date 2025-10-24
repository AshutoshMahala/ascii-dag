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
//! use ascii_dag::cycles::generic::detect_cycle_fn;
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

use crate::graph::DAG;
use alloc::vec::Vec;

impl<'a> DAG<'a> {
    /// Check if the graph contains cycles (making it not a valid DAG).
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::DAG;
    ///
    /// let mut dag = DAG::new();
    /// dag.add_node(1, "A");
    /// dag.add_node(2, "B");
    /// dag.add_edge(1, 2);
    /// dag.add_edge(2, 1);  // Creates a cycle!
    ///
    /// assert!(dag.has_cycle());
    /// ```
    pub fn has_cycle(&self) -> bool {
        let mut visited = vec![false; self.nodes.len()];
        let mut rec_stack = vec![false; self.nodes.len()];

        for i in 0..self.nodes.len() {
            if self.has_cycle_util(i, &mut visited, &mut rec_stack) {
                return true;
            }
        }
        false
    }

    /// Helper function for cycle detection using DFS.
    fn has_cycle_util(&self, idx: usize, visited: &mut [bool], rec_stack: &mut [bool]) -> bool {
        if rec_stack[idx] {
            return true;
        }
        if visited[idx] {
            return false;
        }

        visited[idx] = true;
        rec_stack[idx] = true;

        let node_id = self.nodes[idx].0;
        for &(from, to) in &self.edges {
            if from == node_id {
                // O(1) HashMap lookup instead of O(n) scan
                if let Some(child_idx) = self.node_index(to) {
                    if self.has_cycle_util(child_idx, visited, rec_stack) {
                        return true;
                    }
                }
            }
        }

        rec_stack[idx] = false;
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
        for &(from, to) in &self.edges {
            if from == node_id {
                // O(1) HashMap lookup instead of O(n) scan
                if let Some(child_idx) = self.node_index(to) {
                    if let Some(cycle) = self.find_cycle_from(child_idx, visited, path) {
                        return Some(cycle);
                    }
                }
            }
        }

        path.pop();
        None
    }
}

#[cfg(test)]
mod tests {
    use crate::graph::DAG;

    #[test]
    fn test_cycle_detection() {
        let mut dag = DAG::new();
        dag.add_node(1, "A");
        dag.add_node(2, "B");
        dag.add_edge(1, 2);
        dag.add_edge(2, 1); // Cycle!

        assert!(dag.has_cycle());
    }

    #[test]
    fn test_no_cycle() {
        let dag = DAG::from_edges(&[(1, "A"), (2, "B")], &[(1, 2)]);

        assert!(!dag.has_cycle());
    }

    #[test]
    fn test_cycle_with_auto_created_nodes() {
        let mut dag = DAG::new();
        dag.add_node(1, "A");
        // Node 2 will be auto-created
        dag.add_edge(1, 2);
        dag.add_edge(2, 1); // Creates cycle

        assert!(dag.has_cycle());
    }
}
