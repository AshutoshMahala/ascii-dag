//! Generic topological sorting for any data structure.
//!
//! This module provides topological sorting that works with any graph-like
//! structure via higher-order functions, similar to the generic cycle detection.
//!
//! ## Submodules
//!
//! - [`impact`] - Impact analysis (descendants, ancestors, blast radius)
//! - [`metrics`] - Graph metrics and statistics
//!
//! # Examples
//!
//! ```
//! use ascii_dag::layout::generic::topological_sort_fn;
//!
//! // Example: Sort tasks by dependencies
//! let get_deps = |task: &&str| match *task {
//!     "deploy" => vec!["test", "build"],
//!     "test" => vec!["build"],
//!     "build" => vec!["compile"],
//!     "compile" => vec![],
//!     _ => vec![],
//! };
//!
//! let tasks = ["deploy", "test", "build", "compile"];
//! if let Ok(sorted) = topological_sort_fn(&tasks, get_deps) {
//!     // sorted = ["compile", "build", "test", "deploy"]
//!     assert_eq!(sorted[0], "compile");  // No dependencies, comes first
//!     assert_eq!(sorted[3], "deploy");   // Depends on everything, comes last
//! }
//! ```

pub mod impact;
pub mod metrics;

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::hash::Hash;

/// Performs topological sorting on a collection of items using a dependency function.
///
/// # Arguments
/// * `items` - Slice of all items to sort
/// * `get_dependencies` - Function that returns the dependencies for each item
///
/// # Returns
/// * `Ok(Vec<Id>)` - Items in topological order (items with no dependencies first)
/// * `Err(Vec<Id>)` - A cycle was detected, returns one of the cycles found
///
/// # Examples
///
/// ```
/// use ascii_dag::layout::generic::topological_sort_fn;
///
/// let get_deps = |&id: &usize| match id {
///     1 => vec![],      // No dependencies
///     2 => vec![1],     // Depends on 1
///     3 => vec![1, 2],  // Depends on 1 and 2
///     _ => vec![],
/// };
///
/// let items = [1, 2, 3];
/// let sorted = topological_sort_fn(&items, get_deps).unwrap();
/// assert_eq!(sorted, vec![1, 2, 3]);
/// ```
pub fn topological_sort_fn<Id, F>(items: &[Id], get_dependencies: F) -> Result<Vec<Id>, Vec<Id>>
where
    Id: Clone + Eq + Hash + Ord,
    F: Fn(&Id) -> Vec<Id>,
{
    use crate::cycles::generic::detect_cycle_fn;

    // First check for cycles
    if let Some(cycle) = detect_cycle_fn(items, &get_dependencies) {
        return Err(cycle);
    }

    // Kahn's algorithm with BTreeMap for deterministic ordering
    let mut in_degree: BTreeMap<Id, usize> = BTreeMap::new();
    let mut result = Vec::new();

    // Initialize in-degrees
    for item in items {
        in_degree.entry(item.clone()).or_insert(0);
    }

    // Calculate in-degrees: if item depends on dep, then item has incoming edge from dep
    for item in items {
        let deps_count = get_dependencies(item).len();
        *in_degree.entry(item.clone()).or_insert(0) += deps_count;
    }

    // Find all items with no dependencies (in_degree == 0)
    let mut queue: Vec<Id> = in_degree
        .iter()
        .filter(|&(_, &degree)| degree == 0)
        .map(|(id, _)| id.clone())
        .collect();

    // Process queue
    while let Some(item) = queue.pop() {
        result.push(item.clone());

        // Find all items that depend on the current item
        for candidate in items {
            let deps = get_dependencies(candidate);
            if deps.contains(&item) {
                if let Some(degree) = in_degree.get_mut(candidate) {
                    *degree -= 1;
                    if *degree == 0 {
                        queue.push(candidate.clone());
                    }
                }
            }
        }
    }

    // If we processed all items, we have a valid topological order
    if result.len() == items.len() {
        Ok(result)
    } else {
        // This shouldn't happen since we checked for cycles, but handle it anyway
        Err(vec![])
    }
}

/// Trait for types that support topological sorting.
///
/// Implement this trait to get convenient `topological_sort()` methods.
///
/// # Examples
///
/// ```
/// use ascii_dag::layout::generic::TopologicallySortable;
/// use std::collections::HashMap;
///
/// struct TaskGraph {
///     tasks: Vec<String>,
///     dependencies: HashMap<String, Vec<String>>,
/// }
///
/// impl TopologicallySortable for TaskGraph {
///     type Id = String;
///
///     fn get_all_ids(&self) -> Vec<String> {
///         self.tasks.clone()
///     }
///
///     fn get_dependencies(&self, id: &String) -> Vec<String> {
///         self.dependencies.get(id).cloned().unwrap_or_default()
///     }
/// }
///
/// // Now you can call:
/// // let sorted = task_graph.topological_sort().unwrap();
/// ```
pub trait TopologicallySortable {
    /// The type of identifiers in the graph.
    type Id: Clone + Eq + Hash + Ord;

    /// Get all item IDs in the collection.
    fn get_all_ids(&self) -> Vec<Self::Id>;

    /// Get the dependencies for a given item.
    fn get_dependencies(&self, id: &Self::Id) -> Vec<Self::Id>;

    /// Perform topological sorting on this collection.
    ///
    /// # Returns
    /// * `Ok(Vec<Id>)` - Items in dependency order
    /// * `Err(Vec<Id>)` - A cycle was detected
    fn topological_sort(&self) -> Result<Vec<Self::Id>, Vec<Self::Id>> {
        let ids = self.get_all_ids();
        topological_sort_fn(&ids, |id| self.get_dependencies(id))
    }

    /// Check if a valid topological ordering exists (i.e., no cycles).
    fn has_valid_ordering(&self) -> bool {
        self.topological_sort().is_ok()
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

        let items = [3, 1, 2]; // Unsorted input
        let sorted = topological_sort_fn(&items, get_deps).unwrap();
        assert_eq!(sorted, vec![1, 2, 3]);
    }

    #[test]
    fn test_diamond_dependency() {
        let get_deps = |&id: &usize| match id {
            1 => vec![],
            2 => vec![1],
            3 => vec![1],
            4 => vec![2, 3],
            _ => vec![],
        };

        let items = [4, 3, 2, 1]; // Unsorted
        let sorted = topological_sort_fn(&items, get_deps).unwrap();
        
        // 1 must come first
        assert_eq!(sorted[0], 1);
        // 4 must come last
        assert_eq!(sorted[3], 4);
        // 2 and 3 can be in any order, but both after 1 and before 4
        assert!(sorted[1] == 2 || sorted[1] == 3);
    }

    #[test]
    fn test_cycle_detection() {
        let get_deps = |&id: &usize| match id {
            1 => vec![2],
            2 => vec![3],
            3 => vec![1], // Cycle!
            _ => vec![],
        };

        let items = [1, 2, 3];
        let result = topological_sort_fn(&items, get_deps);
        assert!(result.is_err());
    }

    #[test]
    fn test_multiple_roots() {
        let get_deps = |&id: &usize| match id {
            1 => vec![],
            2 => vec![],
            3 => vec![1, 2],
            _ => vec![],
        };

        let items = [3, 2, 1];
        let sorted = topological_sort_fn(&items, get_deps).unwrap();
        
        // 1 and 2 must come before 3
        assert!(sorted.iter().position(|&x| x == 1).unwrap() < sorted.iter().position(|&x| x == 3).unwrap());
        assert!(sorted.iter().position(|&x| x == 2).unwrap() < sorted.iter().position(|&x| x == 3).unwrap());
    }

    #[test]
    fn test_trait_based_sorting() {
        use alloc::collections::BTreeMap;

        struct SimpleGraph {
            deps: BTreeMap<usize, Vec<usize>>,
        }

        impl TopologicallySortable for SimpleGraph {
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
        let sorted = graph.topological_sort().unwrap();
        assert_eq!(sorted, vec![1, 2, 3]);
        assert!(graph.has_valid_ordering());
    }
}
