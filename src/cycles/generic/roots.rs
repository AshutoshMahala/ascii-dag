//! Root finding algorithms for dependency graphs.
//!
//! This module provides functions to identify "root" nodes - those with no dependencies.
//! Perfect for finding entry points in task graphs, independent errors, or starting points.
//!
//! # Examples
//!
//! ```
//! use ascii_dag::cycles::generic::roots::find_roots_fn;
//!
//! let get_deps = |task: &&str| match *task {
//!     "deploy" => vec!["build", "test"],
//!     "test" => vec!["compile"],
//!     "build" => vec!["compile"],
//!     "compile" => vec![],  // No dependencies - this is a root!
//!     _ => vec![],
//! };
//!
//! let tasks = ["deploy", "test", "build", "compile"];
//! let roots = find_roots_fn(&tasks, get_deps);
//! assert_eq!(roots, vec!["compile"]);
//! ```

use alloc::vec::Vec;
use core::hash::Hash;

/// Find all root nodes (nodes with no dependencies) in a graph.
///
/// Root nodes are nodes that don't depend on anything else - they're the
/// starting points in a dependency graph.
///
/// # Arguments
/// * `items` - All nodes in the graph
/// * `get_dependencies` - Function returning dependencies for each node
///
/// # Returns
/// Vector of nodes that have no dependencies (roots)
///
/// # Examples
///
/// ```
/// use ascii_dag::cycles::generic::roots::find_roots_fn;
///
/// // Build system example
/// let get_deps = |file: &&str| match *file {
///     "app.exe" => vec!["main.o", "lib.o"],
///     "main.o" => vec!["main.c"],
///     "lib.o" => vec!["lib.c"],
///     "main.c" => vec![],  // Root - source file
///     "lib.c" => vec![],   // Root - source file
///     _ => vec![],
/// };
///
/// let files = ["app.exe", "main.o", "lib.o", "main.c", "lib.c"];
/// let roots = find_roots_fn(&files, get_deps);
/// assert_eq!(roots.len(), 2);  // main.c and lib.c
/// assert!(roots.contains(&"main.c"));
/// assert!(roots.contains(&"lib.c"));
/// ```
pub fn find_roots_fn<Id, F>(items: &[Id], get_dependencies: F) -> Vec<Id>
where
    Id: Clone + Eq + Hash,
    F: Fn(&Id) -> Vec<Id>,
{
    items
        .iter()
        .filter(|item| get_dependencies(item).is_empty())
        .cloned()
        .collect()
}

/// Find all leaf nodes (nodes that nothing depends on).
///
/// Leaf nodes are the opposite of roots - they're the endpoints in a dependency graph.
/// No other nodes depend on them.
///
/// # Examples
///
/// ```
/// use ascii_dag::cycles::generic::roots::find_leaves_fn;
///
/// let get_deps = |task: &&str| match *task {
///     "deploy" => vec!["build"],
///     "build" => vec!["compile"],
///     "compile" => vec![],
///     _ => vec![],
/// };
///
/// let tasks = ["deploy", "build", "compile"];
/// let leaves = find_leaves_fn(&tasks, get_deps);
/// assert_eq!(leaves, vec!["deploy"]);  // Nothing depends on deploy
/// ```
pub fn find_leaves_fn<Id, F>(items: &[Id], get_dependencies: F) -> Vec<Id>
where
    Id: Clone + Eq + Hash,
    F: Fn(&Id) -> Vec<Id>,
{
    let mut leaves = Vec::new();
    
    'outer: for candidate in items {
        // Check if any other item depends on this candidate
        for item in items {
            let deps = get_dependencies(item);
            if deps.contains(candidate) {
                // Someone depends on this candidate, not a leaf
                continue 'outer;
            }
        }
        // No one depends on this candidate - it's a leaf!
        leaves.push(candidate.clone());
    }
    
    leaves
}

/// Trait for types that support root/leaf finding.
///
/// Implement this trait to get convenient root-finding methods on your types.
///
/// # Examples
///
/// ```
/// use ascii_dag::cycles::generic::roots::RootFindable;
/// use std::collections::HashMap;
///
/// struct DependencyGraph {
///     nodes: Vec<String>,
///     deps: HashMap<String, Vec<String>>,
/// }
///
/// impl RootFindable for DependencyGraph {
///     type Id = String;
///
///     fn get_all_ids(&self) -> Vec<String> {
///         self.nodes.clone()
///     }
///
///     fn get_dependencies(&self, id: &String) -> Vec<String> {
///         self.deps.get(id).cloned().unwrap_or_default()
///     }
/// }
///
/// // Now you can use:
/// // let roots = graph.find_roots();
/// // let leaves = graph.find_leaves();
/// ```
pub trait RootFindable {
    /// The type of identifiers in the graph.
    type Id: Clone + Eq + Hash;

    /// Get all node IDs in the graph.
    fn get_all_ids(&self) -> Vec<Self::Id>;

    /// Get the dependencies for a given node.
    fn get_dependencies(&self, id: &Self::Id) -> Vec<Self::Id>;

    /// Find all root nodes (nodes with no dependencies).
    fn find_roots(&self) -> Vec<Self::Id> {
        let ids = self.get_all_ids();
        find_roots_fn(&ids, |id| self.get_dependencies(id))
    }

    /// Find all leaf nodes (nodes that nothing depends on).
    fn find_leaves(&self) -> Vec<Self::Id> {
        let ids = self.get_all_ids();
        find_leaves_fn(&ids, |id| self.get_dependencies(id))
    }

    /// Count the number of root nodes.
    fn root_count(&self) -> usize {
        self.find_roots().len()
    }

    /// Count the number of leaf nodes.
    fn leaf_count(&self) -> usize {
        self.find_leaves().len()
    }

    /// Check if this is a single-rooted graph (has exactly one entry point).
    fn is_single_rooted(&self) -> bool {
        self.root_count() == 1
    }

    /// Check if this is a tree-like graph (single root, possibly multiple leaves).
    fn is_tree_like(&self) -> bool {
        self.is_single_rooted()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_roots_simple() {
        let get_deps = |&id: &usize| match id {
            1 => vec![],
            2 => vec![1],
            3 => vec![1],
            _ => vec![],
        };

        let items = [1, 2, 3];
        let roots = find_roots_fn(&items, get_deps);
        assert_eq!(roots, vec![1]);
    }

    #[test]
    fn test_find_roots_multiple() {
        let get_deps = |&id: &usize| match id {
            1 => vec![],
            2 => vec![],
            3 => vec![1, 2],
            _ => vec![],
        };

        let items = [1, 2, 3];
        let roots = find_roots_fn(&items, get_deps);
        assert_eq!(roots.len(), 2);
        assert!(roots.contains(&1));
        assert!(roots.contains(&2));
    }

    #[test]
    fn test_find_leaves_simple() {
        let get_deps = |&id: &usize| match id {
            1 => vec![],
            2 => vec![1],
            3 => vec![2],
            _ => vec![],
        };

        let items = [1, 2, 3];
        let leaves = find_leaves_fn(&items, get_deps);
        assert_eq!(leaves, vec![3]);
    }

    #[test]
    fn test_find_leaves_multiple() {
        let get_deps = |&id: &usize| match id {
            1 => vec![],
            2 => vec![1],
            3 => vec![1],
            _ => vec![],
        };

        let items = [1, 2, 3];
        let leaves = find_leaves_fn(&items, get_deps);
        assert_eq!(leaves.len(), 2);
        assert!(leaves.contains(&2));
        assert!(leaves.contains(&3));
    }

    #[test]
    fn test_trait_based_root_finding() {
        use alloc::collections::BTreeMap;

        struct SimpleGraph {
            deps: BTreeMap<usize, Vec<usize>>,
        }

        impl RootFindable for SimpleGraph {
            type Id = usize;

            fn get_all_ids(&self) -> Vec<usize> {
                self.deps.keys().copied().collect()
            }

            fn get_dependencies(&self, id: &usize) -> Vec<usize> {
                self.deps.get(id).cloned().unwrap_or_default()
            }
        }

        let mut deps = BTreeMap::new();
        deps.insert(1, vec![]);
        deps.insert(2, vec![1]);
        deps.insert(3, vec![1]);

        let graph = SimpleGraph { deps };
        
        assert_eq!(graph.find_roots(), vec![1]);
        assert!(graph.is_single_rooted());
        assert!(graph.is_tree_like());
        assert_eq!(graph.root_count(), 1);
        assert_eq!(graph.leaf_count(), 2);
    }

    #[test]
    fn test_empty_graph() {
        let get_deps = |_: &usize| vec![];
        let items: [usize; 0] = [];
        let roots = find_roots_fn(&items, get_deps);
        assert_eq!(roots.len(), 0);
    }

    #[test]
    fn test_all_roots() {
        let get_deps = |_: &usize| vec![];
        let items = [1, 2, 3, 4, 5];
        let roots = find_roots_fn(&items, get_deps);
        assert_eq!(roots.len(), 5);
    }
}
