//! Generic cycle detection for any directed graph structure.
//!
//! This module provides reusable cycle detection that works with any data structure
//! through higher-order functions or traits.
//!
//! ## Submodules
//!
//! - [`roots`] - Root and leaf node finding

pub mod roots;

#[cfg(not(feature = "std"))]
use alloc::collections::BTreeMap as HashMap;
use alloc::vec::Vec;
#[cfg(feature = "std")]
use std::collections::HashMap;

/// A trait for types that can be checked for cycles.
///
/// Implement this for your custom types (errors, tasks, dependencies, etc.)
/// to get cycle detection for free.
///
/// # Examples
///
/// ```ignore
/// struct Error {
///     id: usize,
///     caused_by: Vec<usize>,
/// }
///
/// impl CycleDetectable for Error {
///     type Id = usize;
///     
///     fn id(&self) -> Self::Id {
///         self.id
///     }
///     
///     fn dependencies(&self) -> impl Iterator<Item = Self::Id> {
///         self.caused_by.iter().copied()
///     }
/// }
/// ```
pub trait CycleDetectable {
    /// The type used to identify nodes (e.g., usize, String, etc.)
    type Id: Eq + core::hash::Hash + Clone;

    /// Get the unique identifier for this node
    fn id(&self) -> Self::Id;

    /// Get the IDs of all nodes this one depends on
    fn dependencies(&self) -> Vec<Self::Id>;
}

/// Detect cycles in a collection using a higher-order function approach.
///
/// This is the most flexible approach - you provide a closure that returns
/// the dependencies for any given ID.
///
/// # Examples
///
/// ```
/// use ascii_dag::cycles::generic::detect_cycle_fn;
///
/// // Example: Error chain where errors can be caused by other errors
/// let get_dependencies = |error_id: &usize| -> Vec<usize> {
///     match error_id {
///         1 => vec![2],      // Error 1 caused by Error 2
///         2 => vec![3],      // Error 2 caused by Error 3
///         3 => vec![1],      // Error 3 caused by Error 1 - CYCLE!
///         _ => vec![],
///     }
/// };
///
/// let all_ids = vec![1, 2, 3];
/// assert!(detect_cycle_fn(&all_ids, get_dependencies).is_some());
/// ```
pub fn detect_cycle_fn<Id, F>(all_ids: &[Id], get_dependencies: F) -> Option<Vec<Id>>
where
    Id: Eq + core::hash::Hash + Clone,
    F: Fn(&Id) -> Vec<Id>,
{
    let mut id_to_index: HashMap<Id, usize> = HashMap::new();
    for (idx, id) in all_ids.iter().enumerate() {
        id_to_index.insert(id.clone(), idx);
    }

    let mut visited = vec![false; all_ids.len()];
    let mut rec_stack = vec![false; all_ids.len()];

    for i in 0..all_ids.len() {
        if let Some(cycle) = has_cycle_util_fn(
            i,
            all_ids,
            &get_dependencies,
            &id_to_index,
            &mut visited,
            &mut rec_stack,
        ) {
            return Some(cycle);
        }
    }
    None
}

fn has_cycle_util_fn<Id, F>(
    idx: usize,
    all_ids: &[Id],
    get_dependencies: &F,
    id_to_index: &HashMap<Id, usize>,
    visited: &mut [bool],
    rec_stack: &mut [bool],
) -> Option<Vec<Id>>
where
    Id: Eq + core::hash::Hash + Clone,
    F: Fn(&Id) -> Vec<Id>,
{
    if rec_stack[idx] {
        // Found a cycle - return the node that completes it
        return Some(vec![all_ids[idx].clone()]);
    }
    if visited[idx] {
        return None;
    }

    visited[idx] = true;
    rec_stack[idx] = true;

    let current_id = &all_ids[idx];
    let deps = get_dependencies(current_id);

    for dep_id in deps {
        if let Some(&dep_idx) = id_to_index.get(&dep_id) {
            if let Some(mut cycle) = has_cycle_util_fn(
                dep_idx,
                all_ids,
                get_dependencies,
                id_to_index,
                visited,
                rec_stack,
            ) {
                // Add current node to the cycle path
                cycle.push(current_id.clone());
                return Some(cycle);
            }
        }
    }

    rec_stack[idx] = false;
    None
}

/// Detect cycles in a collection of items that implement `CycleDetectable`.
///
/// # Examples
///
/// ```ignore
/// let errors = vec![
///     Error { id: 1, caused_by: vec![2] },
///     Error { id: 2, caused_by: vec![3] },
///     Error { id: 3, caused_by: vec![1] },
/// ];
///
/// let cycle = detect_cycle(&errors);
/// assert!(cycle.is_some());
/// ```
pub fn detect_cycle<T>(items: &[T]) -> Option<Vec<T::Id>>
where
    T: CycleDetectable,
{
    let all_ids: Vec<T::Id> = items.iter().map(|item| item.id()).collect();
    let id_to_item: HashMap<T::Id, &T> = items.iter().map(|item| (item.id(), item)).collect();

    detect_cycle_fn(&all_ids, |id| {
        id_to_item
            .get(id)
            .map(|item| item.dependencies())
            .unwrap_or_default()
    })
}

/// Just check if a cycle exists (faster than finding the path).
pub fn has_cycle_fn<Id, F>(all_ids: &[Id], get_dependencies: F) -> bool
where
    Id: Eq + core::hash::Hash + Clone,
    F: Fn(&Id) -> Vec<Id>,
{
    detect_cycle_fn(all_ids, get_dependencies).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cycle_detection_with_closure() {
        // Simple error chain with cycle
        let get_deps = |id: &usize| -> Vec<usize> {
            match id {
                1 => vec![2],
                2 => vec![3],
                3 => vec![1], // Cycle!
                _ => vec![],
            }
        };

        let all_ids = vec![1, 2, 3];
        let cycle = detect_cycle_fn(&all_ids, get_deps);
        assert!(cycle.is_some());

        let cycle_path = cycle.unwrap();
        assert!(!cycle_path.is_empty());
    }

    #[test]
    fn test_no_cycle_with_closure() {
        let get_deps = |id: &usize| -> Vec<usize> {
            match id {
                1 => vec![2],
                2 => vec![3],
                3 => vec![],
                _ => vec![],
            }
        };

        let all_ids = vec![1, 2, 3];
        assert!(detect_cycle_fn(&all_ids, get_deps).is_none());
    }

    #[test]
    fn test_complex_dependency_graph() {
        // Diamond with no cycle
        let get_deps = |id: &usize| -> Vec<usize> {
            match id {
                1 => vec![2, 3],
                2 => vec![4],
                3 => vec![4],
                4 => vec![],
                _ => vec![],
            }
        };

        let all_ids = vec![1, 2, 3, 4];
        assert!(detect_cycle_fn(&all_ids, get_deps).is_none());
    }

    #[test]
    fn test_self_referential_cycle() {
        let get_deps = |id: &usize| -> Vec<usize> {
            match id {
                1 => vec![1], // Self-cycle
                _ => vec![],
            }
        };

        let all_ids = vec![1];
        assert!(detect_cycle_fn(&all_ids, get_deps).is_some());
    }

    // Example with custom error type
    #[derive(Clone, Debug, PartialEq, Eq, Hash)]
    struct ErrorNode {
        id: usize,
        message: &'static str,
        caused_by: Vec<usize>,
    }

    impl CycleDetectable for ErrorNode {
        type Id = usize;

        fn id(&self) -> Self::Id {
            self.id
        }

        fn dependencies(&self) -> Vec<Self::Id> {
            self.caused_by.clone()
        }
    }

    #[test]
    fn test_trait_based_cycle_detection() {
        let errors = vec![
            ErrorNode {
                id: 1,
                message: "Network error",
                caused_by: vec![2],
            },
            ErrorNode {
                id: 2,
                message: "Connection failed",
                caused_by: vec![3],
            },
            ErrorNode {
                id: 3,
                message: "Retry limit",
                caused_by: vec![1], // Cycle back to 1
            },
        ];

        let cycle = detect_cycle(&errors);
        assert!(cycle.is_some());
    }

    #[test]
    fn test_trait_based_no_cycle() {
        let errors = vec![
            ErrorNode {
                id: 1,
                message: "File not found",
                caused_by: vec![2],
            },
            ErrorNode {
                id: 2,
                message: "Permission denied",
                caused_by: vec![],
            },
        ];

        let cycle = detect_cycle(&errors);
        assert!(cycle.is_none());
    }
}
