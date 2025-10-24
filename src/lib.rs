//! # ascii-dag
//!
//! Modular ASCII DAG (Directed Acyclic Graph) renderer for error chains,
//! build systems, and dependency visualization.
//!
//! ## Features
//!
//! - **Tiny**: ~77KB WASM (minimal), ~41KB without generic features
//! - **Fast**: Cached adjacency lists, O(1) lookups, zero-copy rendering
//! - **no_std**: Works in embedded/WASM environments
//! - **Modular**: Each component can be used independently
//! - **Safe**: Cycle detection built-in
//!
//! ## Performance
//!
//! - **Cached Adjacency Lists**: O(1) child/parent lookups (not O(E))
//! - **Zero Allocations**: Direct buffer writes with `write_node()`
//! - **HashMap Indexing**: O(1) ID→index instead of O(N) scans
//!
//! ## Feature Flags
//!
//! - `std` (default): Standard library support
//! - `generic` (default): Generic algorithms (cycle detection, topological sort, impact analysis, metrics)
//! - `warnings`: Debug warnings for auto-created nodes
//!
//! To minimize bundle size, disable `generic`:
//! ```toml
//! ascii-dag = { version = "0.1", default-features = false, features = ["std"] }
//! ```
//!
//! ## Quick Start
//!
//! ```rust
//! use ascii_dag::graph::{DAG, RenderMode};
//!
//! // Batch construction (fast!)
//! let dag = DAG::from_edges(
//!     &[(1, "Error1"), (2, "Error2"), (3, "Error3")],
//!     &[(1, 2), (2, 3)]
//! );
//!
//! println!("{}", dag.render());
//! ```
//!
//! ## Modular Design
//!
//! The library is organized into separate, independently-usable modules:
//!
//! ### [`graph`] - Core DAG Structure
//! ```rust
//! use ascii_dag::graph::DAG;
//!
//! let mut dag = DAG::new();
//! dag.add_node(1, "A");
//! dag.add_node(2, "B");
//! dag.add_edge(1, 2);
//! ```
//!
//! ### [`cycles`] - Cycle Detection
//! ```rust
//! use ascii_dag::graph::DAG;
//!
//! let mut dag = DAG::new();
//! dag.add_edge(1, 2);
//! dag.add_edge(2, 1);
//! assert!(dag.has_cycle());
//! ```
//!
//! ### [`cycles::generic`] - Generic Cycle Detection
//! Works with any data structure via higher-order functions:
//! ```rust
//! # #[cfg(feature = "generic")]
//! # {
//! use ascii_dag::cycles::generic::detect_cycle_fn;
//!
//! let get_deps = |id: &usize| match id {
//!     1 => vec![2],
//!     2 => vec![3],
//!     _ => vec![],
//! };
//!
//! let cycle = detect_cycle_fn(&[1, 2, 3], get_deps);
//! assert!(cycle.is_none());
//! # }
//! ```
//!
//! ### [`layout::generic`] - Generic Topological Sorting
//! Sort any dependency graph into execution order:
//! ```rust
//! # #[cfg(feature = "generic")]
//! # {
//! use ascii_dag::layout::generic::topological_sort_fn;
//!
//! let get_deps = |task: &&str| match *task {
//!     "deploy" => vec!["build"],
//!     "build" => vec!["compile"],
//!     "compile" => vec![],
//!     _ => vec![],
//! };
//!
//! let sorted = topological_sort_fn(&["deploy", "compile", "build"], get_deps).unwrap();
//! // Result: ["compile", "build", "deploy"]
//! assert_eq!(sorted[0], "compile");
//! # }
//! ```
//!
//! ### [`layout`] - Graph Layout Algorithms
//! Sugiyama hierarchical layout for positioning nodes.
//!
//! ### [`render`] - ASCII Rendering
//! Vertical, horizontal, and cycle visualization modes.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

// Core modules (always available)
pub mod cycles;
pub mod graph;
pub mod layout;
pub mod render;

// Backward compatibility re-exports
pub use graph::{DAG, RenderMode};

#[cfg(test)]
mod tests {
    use crate::graph::DAG;

    #[test]
    fn test_empty_dag() {
        let dag = DAG::new();
        assert_eq!(dag.render(), "Empty DAG");
    }

    #[test]
    fn test_simple_chain() {
        let dag = DAG::from_edges(&[(1, "A"), (2, "B"), (3, "C")], &[(1, 2), (2, 3)]);

        let output = dag.render();
        assert!(output.contains("A"));
        assert!(output.contains("B"));
        assert!(output.contains("C"));
    }

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
    fn test_diamond() {
        let dag = DAG::from_edges(
            &[(1, "A"), (2, "B"), (3, "C"), (4, "D")],
            &[(1, 2), (1, 3), (2, 4), (3, 4)],
        );

        assert!(!dag.has_cycle());
        let output = dag.render();
        assert!(output.contains("A"));
        assert!(output.contains("D"));
    }

    #[test]
    fn test_auto_created_nodes() {
        let mut dag = DAG::new();
        dag.add_node(1, "A");
        dag.add_edge(1, 2); // Auto-creates node 2
        dag.add_node(3, "C");
        dag.add_edge(2, 3);

        let output = dag.render();

        // Normal nodes have square brackets
        assert!(output.contains("[A]"));
        assert!(output.contains("[C]"));

        // Auto-created node has angle brackets
        assert!(output.contains("⟨2⟩"));

        // Verify auto_created tracking
        assert!(dag.is_auto_created(2));
        assert!(!dag.is_auto_created(1));
        assert!(!dag.is_auto_created(3));
    }

    #[test]
    fn test_no_auto_creation_when_explicit() {
        let mut dag = DAG::new();
        dag.add_node(1, "A");
        dag.add_node(2, "B"); // Explicit!
        dag.add_edge(1, 2);

        let output = dag.render();

        // Both should be square brackets
        assert!(output.contains("[A]"));
        assert!(output.contains("[B]"));
        assert!(!output.contains("⟨")); // No angle brackets

        // Verify nothing was auto-created
        assert!(!dag.is_auto_created(1));
        assert!(!dag.is_auto_created(2));
    }

    #[test]
    fn test_edge_to_missing_node_no_panic() {
        let mut dag = DAG::new();
        dag.add_node(1, "A");
        dag.add_edge(1, 2); // Node 2 doesn't exist - should auto-create

        // Should NOT panic
        let output = dag.render();

        // Should render successfully
        assert!(output.contains("[A]"));
        assert!(output.contains("⟨2⟩"));
    }

    #[test]
    fn test_cross_level_edges() {
        let mut dag = DAG::new();

        dag.add_node(1, "Root");
        dag.add_node(2, "Middle");
        dag.add_node(3, "End");

        dag.add_edge(1, 2);
        dag.add_edge(1, 3);
        dag.add_edge(2, 3);

        let output = dag.render();

        assert!(output.contains("[Root]"));
        assert!(output.contains("[Middle]"));
        assert!(output.contains("[End]"));
    }

    #[test]
    fn test_crossing_reduction() {
        // Diamond graph to test that crossing reduction runs without panicking
        let mut dag = DAG::new();

        dag.add_node(1, "Top");
        dag.add_node(2, "Right");
        dag.add_node(3, "Left");
        dag.add_node(4, "Bottom");

        dag.add_edge(1, 3);
        dag.add_edge(1, 2);
        dag.add_edge(3, 4);
        dag.add_edge(2, 4);

        let output = dag.render();

        // All nodes should appear
        assert!(output.contains("[Top]"));
        assert!(output.contains("[Left]"));
        assert!(output.contains("[Right]"));
        assert!(output.contains("[Bottom]"));

        // The crossing reduction pass should complete without panic
        // and produce a valid rendering (nodes are reordered to minimize crossings)
        let lines: Vec<&str> = output.lines().collect();
        assert!(
            lines.len() >= 5,
            "Should have multiple lines for diamond pattern"
        );
    }

    #[test]
    fn test_cycle_with_auto_created_nodes() {
        let mut dag = DAG::new();
        dag.add_node(1, "A");
        // Node 2 will be auto-created
        dag.add_edge(1, 2);
        dag.add_edge(2, 1); // Creates cycle

        let output = dag.render();

        // Should show cycle warning
        assert!(output.contains("CYCLE DETECTED"));

        // Auto-created node should use ⟨2⟩ format in cycle output
        assert!(output.contains("⟨2⟩"));

        // Normal node should use [A] format
        assert!(output.contains("[A]"));
    }

    #[test]
    fn test_auto_created_node_promotion() {
        let mut dag = DAG::new();

        dag.add_node(1, "A");
        dag.add_edge(1, 2); // Auto-creates node 2 as placeholder

        // Verify initially auto-created
        assert!(dag.is_auto_created(2));
        let output = dag.render();
        assert!(output.contains("⟨2⟩"), "Before promotion, should show ⟨2⟩");
        assert!(
            !output.contains("[B]"),
            "Before promotion, should not show [B]"
        );

        // Now promote the placeholder
        dag.add_node(2, "B");

        // Verify promotion worked
        assert!(
            !dag.is_auto_created(2),
            "After promotion, should not be auto-created"
        );
        let output_after = dag.render();
        assert!(
            output_after.contains("[B]"),
            "After promotion, should show [B]"
        );
        assert!(
            !output_after.contains("⟨2⟩"),
            "After promotion, should not show ⟨2⟩"
        );

        // Verify no duplicate nodes were created
        let node_count = dag.nodes.iter().filter(|(id, _)| *id == 2).count();
        assert_eq!(node_count, 1, "Should only have one node with id=2");
    }

    #[test]
    fn test_skewed_children_rendering_order() {
        // Test that nodes are rendered left-to-right by x-coordinate,
        // even when median centering moves nodes around.
        let mut dag = DAG::new();

        // Create a level with multiple nodes
        dag.add_node(1, "Top");
        dag.add_node(2, "A");
        dag.add_node(3, "B");
        dag.add_node(4, "C");

        // Top connects to all children
        dag.add_edge(1, 2);
        dag.add_edge(1, 3);
        dag.add_edge(1, 4);

        let output = dag.render();

        // All children should be on the same line
        let lines: Vec<&str> = output.lines().collect();
        let child_line = lines
            .iter()
            .find(|line| line.contains("[A]") && line.contains("[B]") && line.contains("[C]"))
            .expect("Should find line with all children");

        // Find positions of A, B, C on that line
        let a_pos = child_line.find("[A]").unwrap();
        let b_pos = child_line.find("[B]").unwrap();
        let c_pos = child_line.find("[C]").unwrap();

        // They should be in left-to-right order
        assert!(a_pos < b_pos, "A should be left of B");
        assert!(b_pos < c_pos, "B should be left of C");
    }
}
