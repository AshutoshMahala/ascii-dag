//! # ascii-dag
//!
//! Graph layout engine that renders to text — DAGs, and cyclic graphs
//! via dashed back edges — for error chains, build systems, and
//! dependency visualization.
//!
//! ## Features
//!
//! - **Small**: ~94 KB WASM no-alloc build (41 KB gzipped); see BENCHMARK.md
//! - **Fast**: Cached adjacency lists, O(1) lookups, banded zero-copy rendering
//! - **no_std + no-alloc**: the arena pipeline runs without a heap allocator
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
//! - `generic` (default): Generic algorithms over your own types (cycle
//!   detection, topological sort, impact analysis, metrics). Implies `std` —
//!   these keep visited sets keyed by the caller's id type, bounded
//!   `Eq + Hash`, so they need a real `HashSet`.
//! - `alloc`: Heap-based `Graph` API without `std`
//! - `arena` (+ `arena-idx-u8`/`u16`/`u32`): No-alloc CSR layout and
//!   rendering on caller-provided arenas
//! - `ports` (default): declared edge attachment sides and their per-face
//!   positioning; off, attachment is the port-free rule and nothing of the
//!   machinery is linked (the embedded examples build without it)
//!
//! Non-fatal conditions (auto-created placeholders, omitted labels)
//! travel through the [`diagnostics`] channel as typed data — there
//! is no logging feature and the library never writes to stderr.
//!
//! To minimize bundle size, disable `generic`:
//! ```toml
//! ascii-dag = { version = "0.10", default-features = false, features = ["std"] }
//! ```
//!
//! ## Quick Start
//!
//! ```rust
//! use ascii_dag::graph::{Graph, RenderMode};
//!
//! // Batch construction (fast!)
//! let dag = Graph::from_edges(
//!     &[(1, "Error1"), (2, "Error2"), (3, "Error3")],
//!     &[(1, 2), (2, 3)]
//! );
//!
//! println!("{}", dag.render());
//! ```
//!
//! ## Ports
//!
//! An edge can declare which side of a node it leaves from and which
//! side it arrives on, on the handle [`Graph::add_edge`](graph::Graph::add_edge)
//! returns (or by index with `set_edge_ports`); the layout reports the
//! side each end actually took on every IR edge:
//!
//! ```rust
//! # #[cfg(feature = "ports")] {
//! use ascii_dag::{Graph, PhysicalSide, PortSide};
//!
//! let mut g = Graph::new();
//! g.add_node(1usize, "Gateway");
//! g.add_node(2usize, "Service");
//! g.add_node(3usize, "Cache");
//! g.add_edge(1usize, 2usize, None);
//! g.add_edge(1usize, 3usize, None).to_port(PortSide::West);
//!
//! let ir = g.compute_layout();
//! let cache = ir.edges().iter().find(|e| e.to_id == 3).unwrap();
//! assert_eq!(cache.to_port.requested, PortSide::West);
//! assert_eq!(cache.to_port.side, PhysicalSide::West);
//! # }
//! ```
//!
//! [`PortSide`] names a side three ways. Compass sides (`North`,
//! `East`, `South`, `West`) are fixed on the page. Flow sides follow
//! the direction: `Upstream` is the face the flow arrives on,
//! `Downstream` the face it leaves by. Rotations follow it too:
//! `Clockwise` is the traveler's right hand facing downstream,
//! `Counterclockwise` the left. `Auto` (the default) is head-on:
//! leave `Downstream`, arrive `Upstream`.
//!
//! | Side | `TopDown` | `BottomUp` | `LeftRight` | `RightLeft` |
//! |---|---|---|---|---|
//! | `Upstream` | North | South | West | East |
//! | `Downstream` | South | North | East | West |
//! | `Clockwise` | West | East | South | North |
//! | `Counterclockwise` | East | West | North | South |
//!
//! A face has one port by default, shared by every edge declared on
//! it; a [`PortPolicy`] — the graph's (`set_port_policy`) or one
//! node's (`set_node_port_policy`) — chooses `Paired` (an arrival and
//! a departure port), `Spread` (up to a [`PortBound`]) or `Custom` (the
//! [`PortPlacer`] registered with `set_port_placer`) instead. A node is never widened for its
//! ports, and a face with one cell holds one port whatever the policy.
//! Ends the layout could not honor are warnings on the run
//! (`W.Graph.Port.034` for a side on a self-loop, `W.Graph.Port.035`
//! when no lane beside the node was free). The guide:
//! `docs/ports.md`; runnable: `examples/ports.rs`, every section
//! through both pipelines.
//!
//! ## Modular Design
//!
//! The library is organized into separate, independently-usable modules:
//!
//! ### [`graph`] - Core DAG Structure
//! ```rust
//! use ascii_dag::graph::Graph;
//!
//! let mut dag = Graph::new();
//! dag.add_node(1, "A");
//! dag.add_node(2, "B");
//! dag.add_edge(1, 2, None);
//! ```
//!
//! ### Cycle Detection
//! ```rust
//! use ascii_dag::graph::Graph;
//!
//! let mut dag = Graph::new();
//! dag.add_edge(1, 2, None);
//! dag.add_edge(2, 1, None);
//! assert!(dag.has_cycle());
//! ```
//!
//! ### Generic Cycle Detection
//! Works with any data structure via higher-order functions:
//! ```rust
//! # #[cfg(feature = "generic")]
//! # {
//! use ascii_dag::algorithms::cycles::generic::detect_cycle_fn;
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
//! ### Generic Topological Sorting
//! Sort any dependency graph into execution order:
//! ```rust
//! # #[cfg(feature = "generic")]
//! # {
//! use ascii_dag::algorithms::generic::topological_sort_fn;
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
//! ### Graph Layout
//! Sugiyama hierarchical layout for positioning nodes.
//!
//! ### ASCII Rendering
//! Vertical, horizontal, and cycle visualization modes.

#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Allow certain clippy warnings that are cosmetic or intentional:
// - too_many_arguments: Some internal functions have many params for performance
// - unnecessary_cast: Casts like `x as usize` are kept for clarity when index types vary
// - needless_range_loop: Some loops index multiple arrays making iterators awkward
// - collapsible_if: Some nested ifs are more readable uncollapsed
#![allow(clippy::too_many_arguments)]
#![allow(clippy::unnecessary_cast)]
#![allow(clippy::needless_range_loop)]
#![allow(clippy::collapsible_if)]
#![allow(clippy::collapsible_else_if)]

#[cfg(feature = "alloc")]
extern crate alloc;

// ── Module hierarchy ─────────────────────────────────────────────────────
//
//   graph/          DAG struct + arena allocator + CSR graph
//   algorithms/     cycles, generic analysis, Sugiyama layout
//   ir/             layout intermediate representation
//   render/         unified render engine + glyph/palette utilities

#[cfg(not(any(feature = "layout-vertical", feature = "layout-horizontal")))]
compile_error!(
    "ascii-dag: enable at least one layout axis feature — \
     `layout-vertical` (TopDown/BottomUp) and/or `layout-horizontal` \
     (LeftRight/RightLeft). Both are in the default feature set."
);

pub mod algorithms;
pub mod diagnostics;
pub mod errors;
pub mod graph;
pub mod ir;
pub mod render;
pub mod validation;

// ── Convenience re-exports (alloc-dependent) ────────────────────────────

pub use diagnostics::{
    BorrowedReport, CountingDiagnostics, Diagnostic, DiagnosticContext, DiagnosticCounts,
    DiagnosticKind, DiagnosticRef, DiagnosticRun, DiagnosticSink, DiagnosticSubject, FnDiagnostics,
    IgnoreDiagnostics, ProjectedFailure, Report, Severity, SliceDiagnostics,
};
#[cfg(feature = "alloc")]
pub use diagnostics::{OwnedReport, VecDiagnostics};
#[cfg(feature = "alloc")]
pub use errors::ErrorChain;
pub use errors::GraphError;
pub use graph::RenderMode;
pub use graph::{AUTO, Auto, IdOrAuto, NodeId};
#[cfg(feature = "alloc")]
pub use graph::{
    Direction, EdgeHandle, EdgeInsertion, Graph, MissingNodePolicy, NodeInsertion, Subgraph,
};
#[cfg(feature = "alloc")]
pub use ir::{EdgePath, FlowAxis, LayoutEdge, LayoutIR, LayoutIRBuilder, LayoutNode, SubgraphInfo};
pub use render::colors::Palette;
pub use render::engine::{
    ArmWeight, ArmWeights, CellKind, CellMarker, CellView, CompositionRequirements,
    MarkerDirection, SceneComposer, TerminalRenderer,
};
pub use render::engine::{BoxedNode, CustomNode, NodeContent, SimpleNode};
pub use render::engine::{CellColor, HitResult};
pub use render::engine::{
    Charset, ColorMode, ComposeBudget, EmitOptions, LabelOverflow, LabelPlacementPolicy,
    LabelPolicy, LayoutSource, PlanOptions, PlanRun, RenderOptions, Scene, ScenePlanner,
};
pub use render::engine::{
    EdgePathView, EdgeView, LabelSlot, LabelView, NodeKind, NodeOrigin, NodeView, SubgraphView,
};
pub use render::engine::{LabelPosition, LineWeight, MarkerShape, SubgraphBorder};
pub use validation::Requirements;
// Primary config types (always available, no alloc needed)
pub use algorithms::sugiyama::config::{
    AlgorithmConfig, CycleBreaking, Layering, LayoutConfig, Positioning, Routing,
};
pub use algorithms::sugiyama::crossing::{CrossingReducer, FAST, QUALITY, STANDARD};
pub use algorithms::sugiyama::ports::{
    EdgeEnd, PhysicalSide, Port, PortAttachment, PortBound, PortPlacer, PortPolicy, PortSide,
    PortSlot,
};

// Legacy type (alloc-dependent, deprecated)
#[cfg(feature = "alloc")]
#[cfg(test)]
mod tests {
    use crate::graph::Graph;

    #[test]
    fn test_empty_dag() {
        let dag = Graph::new();
        assert_eq!(dag.render(), "Empty DAG");
    }

    #[test]
    fn test_simple_chain() {
        let dag = Graph::from_edges(&[(1, "A"), (2, "B"), (3, "C")], &[(1, 2), (2, 3)]);

        let output = dag.render();
        assert!(output.contains("A"));
        assert!(output.contains("B"));
        assert!(output.contains("C"));
    }

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
    fn test_diamond() {
        let dag = Graph::from_edges(
            &[(1, "A"), (2, "B"), (3, "C"), (4, "D")],
            &[(1, 2), (1, 3), (2, 4), (3, 4)],
        );

        assert!(!dag.has_cycle());
        let output = dag.render();
        assert!(output.contains("A"));
        assert!(output.contains("D"));
    }

    #[cfg(feature = "layout-vertical")]
    #[test]
    fn test_auto_created_nodes() {
        let mut dag = Graph::new();
        dag.add_node(1, "A");
        dag.add_edge(1, 2, None); // Auto-creates node 2
        dag.add_node(3, "C");
        dag.add_edge(2, 3, None);

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
        let mut dag = Graph::new();
        dag.add_node(1, "A");
        dag.add_node(2, "B"); // Explicit!
        dag.add_edge(1, 2, None);

        let output = dag.render();

        // Both should be square brackets
        assert!(output.contains("[A]"));
        assert!(output.contains("[B]"));
        assert!(!output.contains("⟨")); // No angle brackets

        // Verify nothing was auto-created
        assert!(!dag.is_auto_created(1));
        assert!(!dag.is_auto_created(2));
    }

    #[cfg(feature = "layout-vertical")]
    #[test]
    fn test_edge_to_missing_node_no_panic() {
        let mut dag = Graph::new();
        dag.add_node(1, "A");
        dag.add_edge(1, 2, None); // Node 2 doesn't exist - should auto-create

        // Should NOT panic
        let output = dag.render();

        // Should render successfully
        assert!(output.contains("[A]"));
        assert!(output.contains("⟨2⟩"));
    }

    #[test]
    fn test_cross_level_edges() {
        let mut dag = Graph::new();

        dag.add_node(1, "Root");
        dag.add_node(2, "Middle");
        dag.add_node(3, "End");

        dag.add_edge(1, 2, None);
        dag.add_edge(1, 3, None);
        dag.add_edge(2, 3, None);

        let output = dag.render();

        assert!(output.contains("[Root]"));
        assert!(output.contains("[Middle]"));
        assert!(output.contains("[End]"));
    }

    #[test]
    fn test_crossing_reduction() {
        // Diamond graph to test that crossing reduction runs without panicking
        let mut dag = Graph::new();

        dag.add_node(1, "Top");
        dag.add_node(2, "Right");
        dag.add_node(3, "Left");
        dag.add_node(4, "Bottom");

        dag.add_edge(1, 3, None);
        dag.add_edge(1, 2, None);
        dag.add_edge(3, 4, None);
        dag.add_edge(2, 4, None);

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
        let mut dag = Graph::new();
        dag.add_node(1, "A");
        // Node 2 will be auto-created
        dag.add_edge(1, 2, None);
        dag.add_edge(2, 1, None); // Creates cycle

        let output = dag.render();

        // Should show cycle warning
        assert!(output.contains("CYCLE DETECTED"));

        // Auto-created node should use ⟨2⟩ format in cycle output
        assert!(output.contains("⟨2⟩"));

        // Normal node should use [A] format
        assert!(output.contains("[A]"));
    }

    #[cfg(feature = "layout-vertical")]
    #[test]
    fn test_auto_created_node_promotion() {
        let mut dag = Graph::new();

        dag.add_node(1, "A");
        dag.add_edge(1, 2, None); // Auto-creates node 2 as placeholder

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

    #[cfg(feature = "layout-vertical")]
    #[test]
    fn test_skewed_children_rendering_order() {
        // Test that nodes are rendered left-to-right by x-coordinate,
        // even when median centering moves nodes around.
        let mut dag = Graph::new();

        // Create a level with multiple nodes
        dag.add_node(1, "Top");
        dag.add_node(2, "A");
        dag.add_node(3, "B");
        dag.add_node(4, "C");

        // Top connects to all children
        dag.add_edge(1, 2, None);
        dag.add_edge(1, 3, None);
        dag.add_edge(1, 4, None);

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
