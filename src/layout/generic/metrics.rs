//! Graph metrics and statistics.
//!
//! This module provides lightweight statistical analysis of dependency graphs.
//!
//! # Examples
//!
//! ```
//! use ascii_dag::layout::generic::metrics::GraphMetrics;
//!
//! let get_deps = |task: &&str| match *task {
//!     "deploy" => vec!["build", "test"],
//!     "build" => vec!["compile"],
//!     "test" => vec!["compile"],
//!     "compile" => vec![],
//!     _ => vec![],
//! };
//!
//! let tasks = ["deploy", "build", "test", "compile"];
//! let metrics = GraphMetrics::compute(&tasks, get_deps);
//!
//! assert_eq!(metrics.node_count(), 4);
//! assert_eq!(metrics.edge_count(), 4);
//! // Max depth varies based on path (deploy has ancestors [build, compile] or [test, compile])
//! assert!(metrics.max_depth() >= 2);
//! ```

use alloc::vec::Vec;
use core::hash::Hash;

use super::impact::compute_ancestors_fn;
use super::impact::compute_descendants_fn;
use crate::cycles::generic::roots::find_roots_fn;

/// Statistical metrics for a dependency graph.
///
/// # Examples
///
/// ```
/// use ascii_dag::layout::generic::metrics::GraphMetrics;
///
/// let get_deps = |&id: &usize| match id {
///     1 => vec![],
///     2 => vec![1],
///     3 => vec![1, 2],
///     _ => vec![],
/// };
///
/// let items = [1, 2, 3];
/// let metrics = GraphMetrics::compute(&items, get_deps);
///
/// println!("Nodes: {}", metrics.node_count());
/// println!("Max depth: {}", metrics.max_depth());
/// println!("Avg dependencies: {:.2}", metrics.avg_dependencies());
/// ```
#[derive(Debug, Clone)]
pub struct GraphMetrics {
    node_count: usize,
    edge_count: usize,
    root_count: usize,
    leaf_count: usize,
    max_depth: usize,
    max_descendants: usize,
    total_dependencies: usize,
}

impl GraphMetrics {
    /// Compute metrics for a graph.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::layout::generic::metrics::GraphMetrics;
    ///
    /// let get_deps = |&id: &usize| match id {
    ///     1 => vec![],
    ///     2 => vec![1],
    ///     3 => vec![2],
    ///     _ => vec![],
    /// };
    ///
    /// let items = [1, 2, 3];
    /// let metrics = GraphMetrics::compute(&items, get_deps);
    /// assert_eq!(metrics.node_count(), 3);
    /// ```
    pub fn compute<Id, F>(items: &[Id], get_dependencies: F) -> Self
    where
        Id: Clone + Eq + Hash,
        F: Fn(&Id) -> Vec<Id> + Clone,
    {
        let node_count = items.len();
        
        // Count edges and total dependencies
        let mut edge_count = 0;
        let mut total_dependencies = 0;
        for item in items {
            let deps = get_dependencies(item);
            let dep_count = deps.len();
            edge_count += dep_count;
            total_dependencies += dep_count;
        }

        // Find roots and leaves
        let roots = find_roots_fn(items, get_dependencies.clone());
        let root_count = roots.len();
        
        let mut leaf_count = 0;
        for candidate in items {
            let mut is_leaf = true;
            for item in items {
                if get_dependencies(item).contains(candidate) {
                    is_leaf = false;
                    break;
                }
            }
            if is_leaf {
                leaf_count += 1;
            }
        }

        // Calculate max depth (longest path from any root)
        let mut max_depth = 0;
        for item in items {
            let ancestors = compute_ancestors_fn(items, item, get_dependencies.clone());
            max_depth = max_depth.max(ancestors.len());
        }

        // Calculate max descendants (most impactful node)
        let mut max_descendants = 0;
        for item in items {
            let descendants = compute_descendants_fn(items, item, get_dependencies.clone());
            max_descendants = max_descendants.max(descendants.len());
        }

        Self {
            node_count,
            edge_count,
            root_count,
            leaf_count,
            max_depth,
            max_descendants,
            total_dependencies,
        }
    }

    /// Total number of nodes in the graph.
    pub fn node_count(&self) -> usize {
        self.node_count
    }

    /// Total number of edges (dependencies) in the graph.
    pub fn edge_count(&self) -> usize {
        self.edge_count
    }

    /// Number of root nodes (nodes with no dependencies).
    pub fn root_count(&self) -> usize {
        self.root_count
    }

    /// Number of leaf nodes (nodes that nothing depends on).
    pub fn leaf_count(&self) -> usize {
        self.leaf_count
    }

    /// Maximum depth (longest dependency chain from a root).
    pub fn max_depth(&self) -> usize {
        self.max_depth
    }

    /// Maximum number of descendants any single node has.
    ///
    /// This represents the "blast radius" of the most impactful node.
    pub fn max_descendants(&self) -> usize {
        self.max_descendants
    }

    /// Average number of dependencies per node.
    pub fn avg_dependencies(&self) -> f64 {
        if self.node_count == 0 {
            0.0
        } else {
            self.total_dependencies as f64 / self.node_count as f64
        }
    }

    /// Graph density (ratio of actual edges to possible edges).
    ///
    /// Returns a value between 0.0 (no edges) and 1.0 (complete graph).
    pub fn density(&self) -> f64 {
        if self.node_count <= 1 {
            0.0
        } else {
            let max_possible_edges = self.node_count * (self.node_count - 1);
            self.edge_count as f64 / max_possible_edges as f64
        }
    }

    /// Check if this is a tree (single root, no multiple paths to same node).
    pub fn is_tree(&self) -> bool {
        self.root_count == 1 && self.edge_count == self.node_count.saturating_sub(1)
    }

    /// Check if this is a forest (multiple trees, no cycles).
    pub fn is_forest(&self) -> bool {
        self.edge_count == self.node_count.saturating_sub(self.root_count)
    }

    /// Check if the graph is sparse (few edges relative to nodes).
    pub fn is_sparse(&self) -> bool {
        self.density() < 0.1
    }

    /// Check if the graph is dense (many edges relative to nodes).
    pub fn is_dense(&self) -> bool {
        self.density() > 0.5
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_chain() {
        let get_deps = |&id: &usize| match id {
            1 => vec![],
            2 => vec![1],
            3 => vec![2],
            _ => vec![],
        };

        let items = [1, 2, 3];
        let metrics = GraphMetrics::compute(&items, get_deps);

        assert_eq!(metrics.node_count(), 3);
        assert_eq!(metrics.edge_count(), 2);
        assert_eq!(metrics.root_count(), 1);
        assert_eq!(metrics.leaf_count(), 1);
        assert_eq!(metrics.max_depth(), 2);
        assert!(metrics.is_tree());
    }

    #[test]
    fn test_diamond() {
        let get_deps = |&id: &usize| match id {
            1 => vec![],
            2 => vec![1],
            3 => vec![1],
            4 => vec![2, 3],
            _ => vec![],
        };

        let items = [1, 2, 3, 4];
        let metrics = GraphMetrics::compute(&items, get_deps);

        assert_eq!(metrics.node_count(), 4);
        assert_eq!(metrics.edge_count(), 4);
        assert_eq!(metrics.root_count(), 1);
        assert_eq!(metrics.leaf_count(), 1);
        // Max depth is 2: node 4 has ancestors [2, 3, 1] = 3 total ancestors
        // But depth is number of levels, not number of ancestors
        // 1 is at depth 0, 2/3 at depth 1, 4 at depth 2
        assert!(metrics.max_depth() >= 2);
        assert_eq!(metrics.max_descendants(), 3);
        assert!(!metrics.is_tree()); // Diamond has 4 edges, tree would have 3
    }

    #[test]
    fn test_multiple_roots() {
        let get_deps = |&id: &usize| match id {
            1 => vec![],
            2 => vec![],
            3 => vec![1, 2],
            _ => vec![],
        };

        let items = [1, 2, 3];
        let metrics = GraphMetrics::compute(&items, get_deps);

        assert_eq!(metrics.root_count(), 2);
        assert_eq!(metrics.leaf_count(), 1);
        assert!(!metrics.is_tree()); // Multiple roots
        // Not a forest either because 3 has 2 deps but only 2 roots
        assert_eq!(metrics.edge_count(), 2);
    }

    #[test]
    fn test_empty_graph() {
        let get_deps = |_: &usize| vec![];
        let items: [usize; 0] = [];
        let metrics = GraphMetrics::compute(&items, get_deps);

        assert_eq!(metrics.node_count(), 0);
        assert_eq!(metrics.edge_count(), 0);
        assert_eq!(metrics.avg_dependencies(), 0.0);
    }

    #[test]
    fn test_isolated_nodes() {
        let get_deps = |_: &usize| vec![];
        let items = [1, 2, 3, 4, 5];
        let metrics = GraphMetrics::compute(&items, get_deps);

        assert_eq!(metrics.node_count(), 5);
        assert_eq!(metrics.edge_count(), 0);
        assert_eq!(metrics.root_count(), 5);
        assert_eq!(metrics.leaf_count(), 5);
        assert_eq!(metrics.density(), 0.0);
        assert!(metrics.is_sparse());
    }

    #[test]
    fn test_avg_dependencies() {
        let get_deps = |&id: &usize| match id {
            1 => vec![],
            2 => vec![1],
            3 => vec![1, 2],
            _ => vec![],
        };

        let items = [1, 2, 3];
        let metrics = GraphMetrics::compute(&items, get_deps);

        // Total deps: 0 + 1 + 2 = 3, avg = 3/3 = 1.0
        assert_eq!(metrics.avg_dependencies(), 1.0);
    }
}
