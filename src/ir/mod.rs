//! Intermediate Representation for graph layout.
//!
//! This module provides a renderer-agnostic representation of a laid-out graph.
//! The IR captures the computed positions, dimensions, and edge routing information
//! that can be consumed by various renderers (ASCII, ANSI colors, SVG, HTML, etc.).
//!
//! # Architecture
//!
//! ```text
//! DAG → [Layout Algorithm] → LayoutIR → [Renderer] → Output
//!                                           │
//!                                    ┌──────┴──────┐
//!                                    ↓             ↓
//!                               ASCII/ANSI       SVG/HTML
//! ```
//!
//! # Example
//!
//! ```
//! use ascii_dag::DAG;
//!
//! let dag = DAG::from_edges(
//!     &[(1, "A"), (2, "B"), (3, "C")],
//!     &[(1, 2), (1, 3), (2, 3)]
//! );
//!
//! // Get the intermediate representation
//! let ir = dag.compute_layout();
//!
//! // Inspect layout information
//! println!("Levels: {}", ir.level_count());
//! println!("Total width: {}", ir.width());
//! println!("Total height: {}", ir.height());
//!
//! for node in ir.nodes() {
//!     println!("Node '{}' at ({}, {})", node.label, node.x, node.y);
//! }
//! ```

use alloc::vec;
use alloc::vec::Vec;

#[cfg(not(feature = "std"))]
use alloc::collections::BTreeMap as HashMap;
#[cfg(feature = "std")]
use std::collections::HashMap;

/// A node in the laid-out graph with computed position and dimensions.
#[derive(Debug, Clone)]
pub struct LayoutNode<'a> {
    /// Original node ID from the DAG
    pub id: usize,
    /// Node label text
    pub label: &'a str,
    /// X coordinate (left edge, in character cells)
    pub x: usize,
    /// Y coordinate (top edge, in lines)  
    pub y: usize,
    /// Width in character cells (including brackets)
    pub width: usize,
    /// Center X coordinate (for edge routing)
    pub center_x: usize,
    /// The level (depth) this node is at
    pub level: usize,
    /// Position within the level (0-indexed from left)
    pub level_position: usize,
}

/// How an edge is routed between nodes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EdgePath {
    /// Direct vertical connection (nodes are horizontally aligned or adjacent levels)
    Direct,
    /// L-shaped connection with a horizontal segment
    Corner {
        /// Y coordinate of the horizontal segment
        horizontal_y: usize,
    },
    /// Routed through a side channel (for skip-level edges)
    SideChannel {
        /// X coordinate of the vertical channel
        channel_x: usize,
        /// Starting Y of the channel
        start_y: usize,
        /// Ending Y of the channel
        end_y: usize,
    },
    /// Multi-segment path through dummy nodes
    MultiSegment {
        /// Waypoints: (x, y) coordinates the edge passes through
        waypoints: Vec<(usize, usize)>,
    },
}

/// A routed edge in the layout with path information.
#[derive(Debug, Clone)]
pub struct LayoutEdge {
    /// Source node ID
    pub from_id: usize,
    /// Target node ID  
    pub to_id: usize,
    /// Source node's center X coordinate
    pub from_x: usize,
    /// Source node's bottom Y coordinate
    pub from_y: usize,
    /// Target node's center X coordinate
    pub to_x: usize,
    /// Target node's top Y coordinate
    pub to_y: usize,
    /// How the edge is routed
    pub path: EdgePath,
    /// Edge index (for consistent coloring)
    pub edge_index: usize,
}

/// Intermediate representation of a laid-out graph.
///
/// This is the output of the layout algorithm and input to renderers.
/// It contains all the information needed to draw the graph in any format.
#[derive(Debug, Clone)]
pub struct LayoutIR<'a> {
    /// All nodes with their computed positions
    pub(crate) nodes: Vec<LayoutNode<'a>>,
    /// All edges with routing information
    pub(crate) edges: Vec<LayoutEdge>,
    /// Total width in character cells
    pub(crate) width: usize,
    /// Total height in lines
    pub(crate) height: usize,
    /// Number of levels in the layout
    pub(crate) level_count: usize,
    /// Nodes organized by level (indices into `nodes`)
    pub(crate) levels: Vec<Vec<usize>>,
    /// O(1) lookup from node ID to index in nodes vec
    pub(crate) id_to_index: HashMap<usize, usize>,
}

impl<'a> LayoutIR<'a> {
    /// Get the total width of the layout in character cells.
    #[inline]
    pub fn width(&self) -> usize {
        self.width
    }

    /// Get the total height of the layout in lines.
    #[inline]
    pub fn height(&self) -> usize {
        self.height
    }

    /// Get the number of levels (depth) in the graph.
    #[inline]
    pub fn level_count(&self) -> usize {
        self.level_count
    }

    /// Get all laid-out nodes.
    #[inline]
    pub fn nodes(&self) -> &[LayoutNode<'a>] {
        &self.nodes
    }

    /// Get all routed edges.
    #[inline]
    pub fn edges(&self) -> &[LayoutEdge] {
        &self.edges
    }

    /// Get nodes at a specific level.
    pub fn nodes_at_level(&self, level: usize) -> impl Iterator<Item = &LayoutNode<'a>> {
        self.levels
            .get(level)
            .map(|indices| indices.iter().map(|&i| &self.nodes[i]))
            .into_iter()
            .flatten()
    }

    /// Get the node with a specific ID. O(1) lookup.
    #[inline]
    pub fn node_by_id(&self, id: usize) -> Option<&LayoutNode<'a>> {
        self.id_to_index.get(&id).map(|&idx| &self.nodes[idx])
    }

    /// Get all edges originating from a node.
    pub fn edges_from(&self, node_id: usize) -> impl Iterator<Item = &LayoutEdge> {
        self.edges.iter().filter(move |e| e.from_id == node_id)
    }

    /// Get all edges ending at a node.
    pub fn edges_to(&self, node_id: usize) -> impl Iterator<Item = &LayoutEdge> {
        self.edges.iter().filter(move |e| e.to_id == node_id)
    }

    /// Check if two edges cross each other.
    /// Useful for advanced renderers that want to handle crossings specially.
    pub fn edges_cross(&self, edge1: &LayoutEdge, edge2: &LayoutEdge) -> bool {
        // Simple crossing detection: edges cross if their horizontal spans overlap
        // and they go in opposite directions
        let (min1, max1) = if edge1.from_x <= edge1.to_x {
            (edge1.from_x, edge1.to_x)
        } else {
            (edge1.to_x, edge1.from_x)
        };

        let (min2, max2) = if edge2.from_x <= edge2.to_x {
            (edge2.from_x, edge2.to_x)
        } else {
            (edge2.to_x, edge2.from_x)
        };

        // Check horizontal overlap
        let h_overlap = min1 < max2 && min2 < max1;

        // Check if they're in the same vertical region
        let v_overlap = edge1.from_y < edge2.to_y && edge2.from_y < edge1.to_y;

        // Check if directions are opposite (one goes left-to-right, other right-to-left)
        let dir1 = edge1.to_x as isize - edge1.from_x as isize;
        let dir2 = edge2.to_x as isize - edge2.from_x as isize;
        let opposite_dir = (dir1 > 0 && dir2 < 0) || (dir1 < 0 && dir2 > 0);

        h_overlap && v_overlap && opposite_dir
    }

    /// Get a suggested color index for an edge (for colored renderers).
    /// Uses a deterministic algorithm based on edge index to ensure consistent colors.
    pub fn edge_color_index(&self, edge: &LayoutEdge) -> usize {
        // Use the edge's index modulo a palette size
        // This ensures the same edge always gets the same color
        edge.edge_index
    }

    /// Get bounding box for a node (x, y, width, height).
    /// Useful for hit testing in interactive renderers.
    pub fn node_bounds(&self, node: &LayoutNode) -> (usize, usize, usize, usize) {
        (node.x, node.y, node.width, 1) // Nodes are always 1 line tall in ASCII
    }

    /// Find the node at a given coordinate (for mouse interaction).
    /// Returns None if no node is at that position.
    pub fn node_at(&self, x: usize, y: usize) -> Option<&LayoutNode<'a>> {
        self.nodes
            .iter()
            .find(|node| x >= node.x && x < node.x + node.width && y == node.y)
    }

    /// Get edges that connect nodes at a specific level to the next level.
    /// Returns (from_center_x, to_center_x) pairs for drawing connections.
    pub fn edges_between_levels(&self, from_level: usize) -> Vec<(usize, usize)> {
        let to_level = from_level + 1;

        self.edges
            .iter()
            .filter_map(|edge| {
                let from_node = self.node_by_id(edge.from_id)?;
                let to_node = self.node_by_id(edge.to_id)?;

                if from_node.level == from_level && to_node.level == to_level {
                    Some((from_node.center_x, to_node.center_x))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Get all edges from a specific level (including skip-level edges).
    /// Returns full edge info for advanced rendering.
    pub fn edges_from_level(&self, level: usize) -> Vec<&LayoutEdge> {
        self.edges
            .iter()
            .filter(|edge| {
                self.node_by_id(edge.from_id)
                    .map(|n| n.level == level)
                    .unwrap_or(false)
            })
            .collect()
    }

    /// Check if the layout is empty.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

/// Builder for constructing LayoutIR.
/// Used internally by the layout algorithm.
#[derive(Debug, Default)]
pub struct LayoutIRBuilder<'a> {
    nodes: Vec<LayoutNode<'a>>,
    edges: Vec<LayoutEdge>,
    width: usize,
    height: usize,
    level_count: usize,
    levels: Vec<Vec<usize>>,
}

impl<'a> LayoutIRBuilder<'a> {
    /// Create a new builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the number of levels.
    pub fn with_levels(mut self, count: usize) -> Self {
        self.level_count = count;
        self.levels = vec![Vec::new(); count];
        self
    }

    /// Add a node to the layout.
    pub fn add_node(&mut self, node: LayoutNode<'a>) {
        let level = node.level;
        let idx = self.nodes.len();
        self.nodes.push(node);

        // Track nodes by level
        if level < self.levels.len() {
            self.levels[level].push(idx);
        }
    }

    /// Add an edge to the layout.
    pub fn add_edge(&mut self, edge: LayoutEdge) {
        self.edges.push(edge);
    }

    /// Set the total dimensions.
    pub fn set_dimensions(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
    }

    /// Build the final LayoutIR.
    pub fn build(self) -> LayoutIR<'a> {
        // Build id-to-index map for O(1) lookups
        let id_to_index: HashMap<usize, usize> = self
            .nodes
            .iter()
            .enumerate()
            .map(|(idx, node)| (node.id, idx))
            .collect();

        LayoutIR {
            nodes: self.nodes,
            edges: self.edges,
            width: self.width,
            height: self.height,
            level_count: self.level_count,
            levels: self.levels,
            id_to_index,
        }
    }
}
