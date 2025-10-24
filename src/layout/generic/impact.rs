//! Impact analysis for dependency graphs.
//!
//! This module provides functions to analyze the impact of changes in a dependency graph.
//! Perfect for answering questions like "What breaks if I modify this node?"
//!
//! # Examples
//!
//! ```
//! use ascii_dag::layout::generic::impact::compute_descendants_fn;
//!
//! let get_deps = |file: &&str| match *file {
//!     "app.exe" => vec!["main.o", "utils.o"],
//!     "main.o" => vec!["main.c"],
//!     "utils.o" => vec!["utils.c"],
//!     _ => vec![],
//! };
//!
//! let files = ["app.exe", "main.o", "utils.o", "main.c", "utils.c"];
//!
//! // What needs to be rebuilt if utils.c changes?
//! let impacted = compute_descendants_fn(&files, &"utils.c", get_deps);
//! assert!(impacted.contains(&"utils.o"));
//! assert!(impacted.contains(&"app.exe"));
//! ```

use alloc::vec::Vec;
use core::hash::Hash;

#[cfg(not(feature = "std"))]
use alloc::collections::{BTreeSet as HashSet, VecDeque};
#[cfg(feature = "std")]
use std::collections::{HashSet, VecDeque};

/// Compute all nodes that (transitively) depend on a given starting node.
///
/// This performs a breadth-first search to find all descendants - nodes that would
/// be affected if the starting node changed.
///
/// # Arguments
/// * `items` - All nodes in the graph
/// * `start` - The node to analyze impact from
/// * `get_dependencies` - Function returning dependencies for each node
///
/// # Returns
/// Vector of all nodes that directly or indirectly depend on `start`
///
/// # Examples
///
/// ```
/// use ascii_dag::layout::generic::impact::compute_descendants_fn;
///
/// // Build dependency graph
/// let get_deps = |pkg: &&str| match *pkg {
///     "app" => vec!["lib-a", "lib-b"],
///     "lib-a" => vec!["lib-core"],
///     "lib-b" => vec!["lib-core"],
///     "lib-core" => vec![],
///     _ => vec![],
/// };
///
/// let packages = ["app", "lib-a", "lib-b", "lib-core"];
///
/// // What breaks if lib-core changes?
/// let impacted = compute_descendants_fn(&packages, &"lib-core", get_deps);
/// assert_eq!(impacted.len(), 3);  // lib-a, lib-b, app all depend on it
/// ```
pub fn compute_descendants_fn<Id, F>(items: &[Id], start: &Id, get_dependencies: F) -> Vec<Id>
where
    Id: Clone + Eq + Hash,
    F: Fn(&Id) -> Vec<Id>,
{
    let mut descendants = Vec::new();
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();

    // Find all items that depend on 'start'
    for item in items {
        let deps = get_dependencies(item);
        if deps.contains(start) && !visited.contains(item) {
            queue.push_back(item.clone());
            visited.insert(item.clone());
        }
    }

    // BFS to find all transitive dependents
    while let Some(current) = queue.pop_front() {
        descendants.push(current.clone());

        // Find items that depend on current
        for item in items {
            let deps = get_dependencies(item);
            if deps.contains(&current) && !visited.contains(item) {
                queue.push_back(item.clone());
                visited.insert(item.clone());
            }
        }
    }

    descendants
}

/// Compute all nodes that a given node (transitively) depends on.
///
/// This finds all ancestors/prerequisites - everything that must exist
/// before the starting node can be built/executed.
///
/// # Examples
///
/// ```
/// use ascii_dag::layout::generic::impact::compute_ancestors_fn;
///
/// let get_deps = |task: &&str| match *task {
///     "deploy" => vec!["test", "build"],
///     "test" => vec!["compile"],
///     "build" => vec!["compile"],
///     "compile" => vec![],
///     _ => vec![],
/// };
///
/// let tasks = ["deploy", "test", "build", "compile"];
///
/// // What must complete before deploy?
/// let prerequisites = compute_ancestors_fn(&tasks, &"deploy", get_deps);
/// assert!(prerequisites.contains(&"compile"));
/// assert!(prerequisites.contains(&"test"));
/// assert!(prerequisites.contains(&"build"));
/// ```
pub fn compute_ancestors_fn<Id, F>(_items: &[Id], start: &Id, get_dependencies: F) -> Vec<Id>
where
    Id: Clone + Eq + Hash,
    F: Fn(&Id) -> Vec<Id>,
{
    let mut ancestors = Vec::new();
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();

    // Start with direct dependencies
    for dep in get_dependencies(start) {
        if !visited.contains(&dep) {
            queue.push_back(dep.clone());
            visited.insert(dep.clone());
        }
    }

    // BFS to find all transitive dependencies
    while let Some(current) = queue.pop_front() {
        ancestors.push(current.clone());

        for dep in get_dependencies(&current) {
            if !visited.contains(&dep) {
                queue.push_back(dep.clone());
                visited.insert(dep.clone());
            }
        }
    }

    ancestors
}

/// Calculate the "blast radius" - total impact of changing a node.
///
/// Returns both ancestors (what this depends on) and descendants (what depends on this).
///
/// # Examples
///
/// ```
/// use ascii_dag::layout::generic::impact::compute_blast_radius_fn;
///
/// let get_deps = |id: &usize| vec![
///     if *id == 2 { 1 } else if *id == 3 || *id == 4 { 2 } else { 0 }
/// ].into_iter().filter(|&x| x != 0).collect();
///
/// let items = [1, 2, 3, 4];
/// let (ancestors, descendants) = compute_blast_radius_fn(&items, &2, get_deps);
///
/// assert_eq!(ancestors.len(), 1);      // Depends on: 1
/// assert_eq!(descendants.len(), 2);    // Impacts: 3, 4
/// ```
pub fn compute_blast_radius_fn<Id, F>(
    items: &[Id],
    start: &Id,
    get_dependencies: F,
) -> (Vec<Id>, Vec<Id>)
where
    Id: Clone + Eq + Hash,
    F: Fn(&Id) -> Vec<Id> + Clone,
{
    let ancestors = compute_ancestors_fn(items, start, get_dependencies.clone());
    let descendants = compute_descendants_fn(items, start, get_dependencies);
    (ancestors, descendants)
}

/// Trait for types that support impact analysis.
///
/// # Examples
///
/// ```
/// use ascii_dag::layout::generic::impact::ImpactAnalyzable;
/// use std::collections::HashMap;
///
/// struct PackageRegistry {
///     packages: Vec<String>,
///     dependencies: HashMap<String, Vec<String>>,
/// }
///
/// impl ImpactAnalyzable for PackageRegistry {
///     type Id = String;
///
///     fn get_all_ids(&self) -> Vec<String> {
///         self.packages.clone()
///     }
///
///     fn get_dependencies(&self, id: &String) -> Vec<String> {
///         self.dependencies.get(id).cloned().unwrap_or_default()
///     }
/// }
///
/// // Now you can use:
/// // let impacted = registry.compute_descendants(&pkg_name);
/// // let (deps, impacted) = registry.compute_blast_radius(&pkg_name);
/// ```
pub trait ImpactAnalyzable {
    /// The type of identifiers in the graph.
    type Id: Clone + Eq + Hash;

    /// Get all node IDs in the graph.
    fn get_all_ids(&self) -> Vec<Self::Id>;

    /// Get the dependencies for a given node.
    fn get_dependencies(&self, id: &Self::Id) -> Vec<Self::Id>;

    /// Find all nodes that depend on the given node.
    fn compute_descendants(&self, start: &Self::Id) -> Vec<Self::Id> {
        let ids = self.get_all_ids();
        compute_descendants_fn(&ids, start, |id| self.get_dependencies(id))
    }

    /// Find all nodes that the given node depends on.
    fn compute_ancestors(&self, start: &Self::Id) -> Vec<Self::Id> {
        let ids = self.get_all_ids();
        compute_ancestors_fn(&ids, start, |id| self.get_dependencies(id))
    }

    /// Calculate total impact of a node (both dependencies and dependents).
    fn compute_blast_radius(&self, start: &Self::Id) -> (Vec<Self::Id>, Vec<Self::Id>) {
        let ids = self.get_all_ids();
        compute_blast_radius_fn(&ids, start, |id| self.get_dependencies(id))
    }

    /// Count how many nodes depend on this node.
    fn impact_count(&self, start: &Self::Id) -> usize {
        self.compute_descendants(start).len()
    }

    /// Count how many nodes this node depends on.
    fn dependency_count(&self, start: &Self::Id) -> usize {
        self.compute_ancestors(start).len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_descendants_simple() {
        let get_deps = |&id: &usize| match id {
            1 => vec![],
            2 => vec![1],
            3 => vec![2],
            _ => vec![],
        };

        let items = [1, 2, 3];
        let descendants = compute_descendants_fn(&items, &1, get_deps);

        assert_eq!(descendants.len(), 2);
        assert!(descendants.contains(&2));
        assert!(descendants.contains(&3));
    }

    #[test]
    fn test_compute_descendants_diamond() {
        let get_deps = |&id: &usize| match id {
            1 => vec![],
            2 => vec![1],
            3 => vec![1],
            4 => vec![2, 3],
            _ => vec![],
        };

        let items = [1, 2, 3, 4];
        let descendants = compute_descendants_fn(&items, &1, get_deps);

        assert_eq!(descendants.len(), 3);
        assert!(descendants.contains(&2));
        assert!(descendants.contains(&3));
        assert!(descendants.contains(&4));
    }

    #[test]
    fn test_compute_ancestors_simple() {
        let get_deps = |&id: &usize| match id {
            1 => vec![],
            2 => vec![1],
            3 => vec![2],
            _ => vec![],
        };

        let items = [1, 2, 3];
        let ancestors = compute_ancestors_fn(&items, &3, get_deps);

        assert_eq!(ancestors.len(), 2);
        assert!(ancestors.contains(&1));
        assert!(ancestors.contains(&2));
    }

    #[test]
    fn test_blast_radius() {
        let get_deps = |&id: &usize| match id {
            1 => vec![],
            2 => vec![1],
            3 => vec![2],
            4 => vec![2],
            _ => vec![],
        };

        let items = [1, 2, 3, 4];
        let (ancestors, descendants) = compute_blast_radius_fn(&items, &2, get_deps);

        assert_eq!(ancestors.len(), 1);
        assert!(ancestors.contains(&1));
        assert_eq!(descendants.len(), 2);
        assert!(descendants.contains(&3));
        assert!(descendants.contains(&4));
    }

    #[test]
    fn test_no_impact() {
        let get_deps = |&id: &usize| match id {
            1 => vec![],
            2 => vec![],
            3 => vec![],
            _ => vec![],
        };

        let items = [1, 2, 3];
        let descendants = compute_descendants_fn(&items, &1, get_deps);
        assert_eq!(descendants.len(), 0);
    }

    #[test]
    fn test_trait_based_impact() {
        use alloc::collections::BTreeMap;

        struct SimpleGraph {
            deps: BTreeMap<usize, Vec<usize>>,
        }

        impl ImpactAnalyzable for SimpleGraph {
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
        deps.insert(3, vec![2]);

        let graph = SimpleGraph { deps };

        assert_eq!(graph.compute_descendants(&1).len(), 2);
        assert_eq!(graph.compute_ancestors(&3).len(), 2);
        assert_eq!(graph.impact_count(&1), 2);
        assert_eq!(graph.dependency_count(&3), 2);
    }
}
