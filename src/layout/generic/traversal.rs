//! Graph traversal utilities for collecting and visiting nodes.
//!
//! This module provides functions to traverse graphs and collect nodes,
//! handling cycles automatically through visited tracking.
//!
//! # Examples
//!
//! ```
//! use ascii_dag::layout::generic::traversal::collect_all_nodes_fn;
//!
//! let get_children = |node: &usize| match node {
//!     1 => vec![2, 3],
//!     2 => vec![4],
//!     3 => vec![4],
//!     4 => vec![],
//!     _ => vec![],
//! };
//!
//! let start = vec![1];
//! let all_nodes = collect_all_nodes_fn(&start, get_children);
//! assert_eq!(all_nodes.len(), 4);  // Nodes 1, 2, 3, 4
//! ```

use alloc::vec::Vec;
use core::hash::Hash;

#[cfg(not(feature = "std"))]
use alloc::collections::{BTreeSet as HashSet, VecDeque};
#[cfg(feature = "std")]
use std::collections::{HashSet, VecDeque};

/// Collect all nodes reachable from the starting nodes, handling cycles.
///
/// This performs a breadth-first traversal starting from the given nodes,
/// visiting each reachable node exactly once. Cycles are handled automatically
/// by tracking visited nodes.
///
/// # Arguments
/// * `start_nodes` - Starting points for traversal
/// * `get_children` - Function returning child nodes for each node
///
/// # Returns
/// Vector of all unique nodes reachable from the starting nodes, in BFS order.
///
/// # Examples
///
/// ## Simple Tree Traversal
/// ```
/// use ascii_dag::layout::generic::traversal::collect_all_nodes_fn;
///
/// let get_children = |file: &&str| match *file {
///     "app.exe" => vec!["main.o", "utils.o"],
///     "main.o" => vec!["main.c"],
///     "utils.o" => vec!["utils.c"],
///     _ => vec![],
/// };
///
/// let start = vec!["app.exe"];
/// let all_files = collect_all_nodes_fn(&start, get_children);
/// assert_eq!(all_files.len(), 5);  // app.exe, main.o, utils.o, main.c, utils.c
/// ```
///
/// ## Handling Cycles
/// ```
/// use ascii_dag::layout::generic::traversal::collect_all_nodes_fn;
///
/// // Graph with cycle: 1 -> 2 -> 3 -> 1
/// let get_children = |&node: &usize| match node {
///     1 => vec![2],
///     2 => vec![3],
///     3 => vec![1],  // Cycle back to 1
///     _ => vec![],
/// };
///
/// let start = vec![1];
/// let all_nodes = collect_all_nodes_fn(&start, get_children);
/// assert_eq!(all_nodes.len(), 3);  // Visits each node once: 1, 2, 3
/// ```
///
/// ## Multiple Starting Points
/// ```
/// use ascii_dag::layout::generic::traversal::collect_all_nodes_fn;
///
/// let get_children = |&node: &usize| match node {
///     1 => vec![3],
///     2 => vec![3],
///     3 => vec![4],
///     _ => vec![],
/// };
///
/// let start = vec![1, 2];  // Start from both 1 and 2
/// let all_nodes = collect_all_nodes_fn(&start, get_children);
/// assert_eq!(all_nodes.len(), 4);  // 1, 2, 3, 4 (3 and 4 visited only once)
/// ```
pub fn collect_all_nodes_fn<Id, F>(start_nodes: &[Id], get_children: F) -> Vec<Id>
where
    Id: Clone + Eq + Hash,
    F: Fn(&Id) -> Vec<Id>,
{
    let mut result = Vec::new();
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();

    // Initialize queue with starting nodes
    for node in start_nodes {
        if visited.insert(node.clone()) {
            queue.push_back(node.clone());
        }
    }

    // BFS traversal
    while let Some(node) = queue.pop_front() {
        result.push(node.clone());

        // Add children to queue
        for child in get_children(&node) {
            if visited.insert(child.clone()) {
                queue.push_back(child);
            }
        }
    }

    result
}

/// Collect all nodes reachable from starting nodes using depth-first search.
///
/// Similar to [`collect_all_nodes_fn`] but uses DFS instead of BFS.
/// DFS may be more memory-efficient for deep graphs.
///
/// # Examples
///
/// ```
/// use ascii_dag::layout::generic::traversal::collect_all_nodes_dfs_fn;
///
/// let get_children = |&node: &usize| match node {
///     1 => vec![2, 3],
///     2 => vec![4],
///     3 => vec![5],
///     _ => vec![],
/// };
///
/// let start = vec![1];
/// let all_nodes = collect_all_nodes_dfs_fn(&start, get_children);
/// assert_eq!(all_nodes.len(), 5);  // 1, 2, 4, 3, 5 (DFS order)
/// ```
pub fn collect_all_nodes_dfs_fn<Id, F>(start_nodes: &[Id], get_children: F) -> Vec<Id>
where
    Id: Clone + Eq + Hash,
    F: Fn(&Id) -> Vec<Id>,
{
    let mut result = Vec::new();
    let mut visited = HashSet::new();
    let mut stack = Vec::new();

    // Initialize stack with starting nodes (reverse order for correct DFS)
    for node in start_nodes.iter().rev() {
        if visited.insert(node.clone()) {
            stack.push(node.clone());
        }
    }

    // DFS traversal
    while let Some(node) = stack.pop() {
        result.push(node.clone());

        // Add children to stack (reverse order for consistent left-to-right traversal)
        let children = get_children(&node);
        for child in children.iter().rev() {
            if visited.insert(child.clone()) {
                stack.push(child.clone());
            }
        }
    }

    result
}

/// Trait for types that support node collection traversal.
///
/// Implement this to get convenient traversal methods on your types.
///
/// # Examples
///
/// ```
/// use ascii_dag::layout::generic::traversal::NodeCollectable;
/// use std::collections::HashMap;
///
/// struct Graph {
///     nodes: Vec<usize>,
///     edges: HashMap<usize, Vec<usize>>,
/// }
///
/// impl NodeCollectable for Graph {
///     type Id = usize;
///
///     fn get_all_ids(&self) -> Vec<usize> {
///         self.nodes.clone()
///     }
///
///     fn get_children(&self, id: &usize) -> Vec<usize> {
///         self.edges.get(id).cloned().unwrap_or_default()
///     }
/// }
///
/// // Now you can use:
/// // let all_nodes = graph.collect_all_nodes(&[start_node]);
/// ```
pub trait NodeCollectable {
    /// The type of node identifiers.
    type Id: Clone + Eq + Hash;

    /// Get all node IDs in the graph.
    fn get_all_ids(&self) -> Vec<Self::Id>;

    /// Get the children of a given node.
    fn get_children(&self, id: &Self::Id) -> Vec<Self::Id>;

    /// Collect all nodes reachable from starting nodes (BFS).
    fn collect_all_nodes(&self, start_nodes: &[Self::Id]) -> Vec<Self::Id> {
        collect_all_nodes_fn(start_nodes, |id| self.get_children(id))
    }

    /// Collect all nodes reachable from starting nodes (DFS).
    fn collect_all_nodes_dfs(&self, start_nodes: &[Self::Id]) -> Vec<Self::Id> {
        collect_all_nodes_dfs_fn(start_nodes, |id| self.get_children(id))
    }

    /// Collect all nodes in the entire graph.
    fn collect_all_graph_nodes(&self) -> Vec<Self::Id> {
        let all_ids = self.get_all_ids();
        collect_all_nodes_fn(&all_ids, |id| self.get_children(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collect_simple_tree() {
        let get_children = |&node: &usize| match node {
            1 => vec![2, 3],
            2 => vec![4],
            3 => vec![5],
            _ => vec![],
        };

        let all_nodes = collect_all_nodes_fn(&[1], get_children);
        assert_eq!(all_nodes.len(), 5);
        assert!(all_nodes.contains(&1));
        assert!(all_nodes.contains(&2));
        assert!(all_nodes.contains(&3));
        assert!(all_nodes.contains(&4));
        assert!(all_nodes.contains(&5));
    }

    #[test]
    fn test_collect_with_cycle() {
        // Cycle: 1 -> 2 -> 3 -> 1
        let get_children = |&node: &usize| match node {
            1 => vec![2],
            2 => vec![3],
            3 => vec![1],
            _ => vec![],
        };

        let all_nodes = collect_all_nodes_fn(&[1], get_children);
        assert_eq!(all_nodes.len(), 3); // Each node visited once
    }

    #[test]
    fn test_collect_diamond() {
        //     1
        //    / \
        //   2   3
        //    \ /
        //     4
        let get_children = |&node: &usize| match node {
            1 => vec![2, 3],
            2 => vec![4],
            3 => vec![4],
            4 => vec![],
            _ => vec![],
        };

        let all_nodes = collect_all_nodes_fn(&[1], get_children);
        assert_eq!(all_nodes.len(), 4);
        assert!(all_nodes.contains(&4)); // 4 should appear only once
    }

    #[test]
    fn test_collect_multiple_starts() {
        let get_children = |&node: &usize| match node {
            1 => vec![3],
            2 => vec![3],
            3 => vec![4],
            _ => vec![],
        };

        let all_nodes = collect_all_nodes_fn(&[1, 2], get_children);
        assert_eq!(all_nodes.len(), 4); // 1, 2, 3, 4
    }

    #[test]
    fn test_dfs_order() {
        let get_children = |&node: &usize| match node {
            1 => vec![2, 3],
            2 => vec![4],
            _ => vec![],
        };

        let bfs = collect_all_nodes_fn(&[1], get_children);
        let dfs = collect_all_nodes_dfs_fn(&[1], get_children);

        // Both should contain all nodes
        assert_eq!(bfs.len(), 4);
        assert_eq!(dfs.len(), 4);

        // But order may differ
        assert_eq!(bfs[0], 1); // Both start with 1
        assert_eq!(dfs[0], 1);
    }
}
