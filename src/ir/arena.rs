//! Arena-backed Layout Intermediate Representation.
//!
//! This module provides an arena-based version of LayoutIR that stores all
//! layout data in arena-allocated slices instead of heap Vecs.
//!
//! Data types and accessors live here. Rendering is split into:
//! - [`super::arena_render`] — plain ASCII rendering
//! - [`super::arena_colored`] — ANSI-colored rendering
//! - [`super::arena_builder`] — builder for constructing from arena memory
//!
//! # Memory Layout
//!
//! ```text
//! ┌─────────────────────────────────────────┐
//! │ Node data: [LayoutNodeArena; node_count]│
//! ├─────────────────────────────────────────┤
//! │ Edge data: [LayoutEdgeArena; edge_count]│
//! ├─────────────────────────────────────────┤
//! │ Level offsets: [usize; level_count + 1] │
//! ├─────────────────────────────────────────┤
//! │ Level data: [usize; node_count]         │
//! ├─────────────────────────────────────────┤
//! │ Waypoints: [(usize, usize); ...]        │
//! └─────────────────────────────────────────┘
//! ```

// Re-export builder and estimate function for backwards compatibility
pub use super::arena_builder::LayoutIRArenaBuilder;
use crate::ir::NodeKind;

/// Node data stored as flat struct (no references to heap).
#[derive(Debug, Clone, Copy)]
pub struct LayoutNodeArena {
    /// Original node ID from the DAG
    pub id: usize,
    /// Offset into label storage
    pub label_offset: usize,
    /// Length of label
    pub label_len: usize,
    /// X coordinate (left edge, in character cells)
    pub x: usize,
    /// Y coordinate (top edge, in lines)
    pub y: usize,
    /// Width in character cells (including brackets)
    pub width: usize,
    /// Height in lines (1 for single-line nodes, >1 for multi-line)
    pub height: usize,
    /// Center X coordinate (for edge routing)
    pub center_x: usize,
    /// The level (depth) this node is at
    pub level: usize,
    /// Position within the level (0-indexed from left)
    pub level_position: usize,
    /// Classification: explicit, implicit, or dummy
    pub kind: NodeKind,
}

/// Edge routing type (no heap allocation version).
#[derive(Debug, Clone, Copy)]
pub enum EdgePathArena {
    /// Direct vertical connection
    Direct,
    /// L-shaped connection with a horizontal segment
    Corner { horizontal_y: usize },
    /// Multi-segment path (waypoints stored separately)
    MultiSegment {
        /// Start index into waypoints array
        waypoints_start: usize,
        /// Number of waypoints
        waypoints_len: usize,
        /// Vertical offset for the start of the edge
        start_y_offset: usize,
    },
}

/// Edge data stored as flat struct.
#[derive(Debug, Clone, Copy)]
pub struct LayoutEdgeArena {
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
    pub path: EdgePathArena,
    /// Edge index (for consistent coloring)
    pub edge_index: usize,
    /// Offset into labels array for edge label (0 = no label)
    pub label_offset: usize,
    /// Length of edge label in bytes (0 = no label)
    pub label_len: usize,
    /// X coordinate for label rendering (0 if no label)
    pub label_x: usize,
    /// Y coordinate for label rendering (0 if no label)
    pub label_y: usize,
    /// Minimum Y coordinate this edge occupies (for early-exit optimization)
    pub min_y: usize,
    /// Maximum Y coordinate this edge occupies (for early-exit optimization)
    pub max_y: usize,
}

/// Arena-backed intermediate representation of a laid-out graph.
///
/// This is the arena-based equivalent of LayoutIR. All data is stored in
/// contiguous arena-allocated slices.
#[derive(Debug)]
pub struct LayoutIRArena<'a> {
    /// All nodes with their computed positions
    nodes: &'a [LayoutNodeArena],
    /// All edges with routing information
    edges: &'a [LayoutEdgeArena],
    /// Waypoints for multi-segment edges: (x, y) pairs
    waypoints: &'a [(usize, usize)],
    /// Label storage (raw bytes, UTF-8)
    labels: &'a [u8],
    /// Total width in character cells
    width: usize,
    /// Total height in lines
    height: usize,
    /// Number of levels in the layout
    level_count: usize,
    /// Level offsets: nodes at level i are at indices level_offsets[i]..level_offsets[i+1]
    level_offsets: &'a [usize],
    /// Node indices organized by level
    level_data: &'a [usize],
}

impl<'a> LayoutIRArena<'a> {
    /// Construct from pre-allocated parts. Used by [`LayoutIRArenaBuilder::build`].
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_parts(
        nodes: &'a [LayoutNodeArena],
        edges: &'a [LayoutEdgeArena],
        waypoints: &'a [(usize, usize)],
        labels: &'a [u8],
        width: usize,
        height: usize,
        level_count: usize,
        level_offsets: &'a [usize],
        level_data: &'a [usize],
    ) -> Self {
        Self {
            nodes,
            edges,
            waypoints,
            labels,
            width,
            height,
            level_count,
            level_offsets,
            level_data,
        }
    }

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

    /// Get the number of nodes.
    #[inline]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Get the number of edges.
    #[inline]
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Get a node by index.
    #[inline]
    pub fn node(&self, index: usize) -> &LayoutNodeArena {
        &self.nodes[index]
    }

    /// Get an edge by index.
    #[inline]
    pub fn edge(&self, index: usize) -> &LayoutEdgeArena {
        &self.edges[index]
    }

    /// Get node label by index.
    #[inline]
    pub fn node_label(&self, index: usize) -> &str {
        let node = &self.nodes[index];
        let bytes = &self.labels[node.label_offset..node.label_offset + node.label_len];
        core::str::from_utf8(bytes).unwrap_or("")
    }

    /// Get edge label by index (returns empty string if no label).
    #[inline]
    pub fn edge_label(&self, index: usize) -> &str {
        let edge = &self.edges[index];
        if edge.label_len == 0 {
            return "";
        }
        let bytes = &self.labels[edge.label_offset..edge.label_offset + edge.label_len];
        core::str::from_utf8(bytes).unwrap_or("")
    }

    /// Check if an edge has a label.
    #[inline]
    pub fn edge_has_label(&self, index: usize) -> bool {
        self.edges[index].label_len > 0
    }

    /// Iterate over all nodes.
    #[inline]
    pub fn nodes(&self) -> &[LayoutNodeArena] {
        self.nodes
    }

    /// Iterate over all edges.
    #[inline]
    pub fn edges(&self) -> &[LayoutEdgeArena] {
        self.edges
    }

    /// Get node indices at a specific level.
    #[inline]
    pub fn nodes_at_level(&self, level: usize) -> &[usize] {
        if level >= self.level_count {
            return &[];
        }
        let start = self.level_offsets[level];
        let end = self.level_offsets[level + 1];
        &self.level_data[start..end]
    }

    /// Get waypoints for a multi-segment edge.
    #[inline]
    pub fn edge_waypoints(&self, edge: &LayoutEdgeArena) -> &[(usize, usize)] {
        match edge.path {
            EdgePathArena::MultiSegment {
                waypoints_start,
                waypoints_len,
                ..
            } => &self.waypoints[waypoints_start..waypoints_start + waypoints_len],
            _ => &[],
        }
    }

    /// Get raw waypoints slice by start/len indices.
    /// Used by rendering modules that already unpacked the edge path.
    #[inline]
    pub(crate) fn edge_waypoints_raw(&self, start: usize, len: usize) -> &[(usize, usize)] {
        &self.waypoints[start..start + len]
    }

    /// Find node by ID (linear search - O(n)).
    pub fn node_by_id(&self, id: usize) -> Option<&LayoutNodeArena> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Find node index by ID (linear search - O(n)).
    pub fn node_index_by_id(&self, id: usize) -> Option<usize> {
        self.nodes.iter().position(|n| n.id == id)
    }

    /// Check if the layout is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Check if a label can be placed without collision.
    /// Returns true if all positions are empty (space) or the edge's vertical line (│).
    pub(crate) fn can_place_label(&self, buffer: &[char], label: &str, x: usize) -> bool {
        if x >= buffer.len() {
            return false;
        }

        let label_len = label.chars().count() + 2; // +2 for quotes

        // Check if all positions are available (space or the edge's own vertical line)
        for i in 0..label_len {
            let pos_x = x + i;
            if pos_x >= buffer.len() {
                return false; // Would go out of bounds
            }
            let c = buffer[pos_x];
            if c != ' ' && c != '│' {
                return false; // Collision with existing character
            }
        }
        true
    }
}

/// Calculate required arena size for layout IR.
pub fn estimate_layout_arena_size(
    node_count: usize,
    edge_count: usize,
    label_bytes: usize,
    max_waypoints: usize,
) -> usize {
    use core::mem::size_of;

    let nodes_size = node_count * size_of::<LayoutNodeArena>();
    let edges_size = edge_count * size_of::<LayoutEdgeArena>();
    let waypoints_size = max_waypoints * size_of::<(usize, usize)>();
    let level_offsets_size = (node_count + 2) * size_of::<usize>(); // Generous estimate
    let level_data_size = node_count * size_of::<usize>();

    // Add alignment padding and extra buffer
    let padding = 8 * 8;

    nodes_size
        + edges_size
        + waypoints_size
        + level_offsets_size
        + level_data_size
        + label_bytes
        + padding
        + 512
}
