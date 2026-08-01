//! Arena-backed Layout Intermediate Representation.
//!
//! This module provides an arena-based version of LayoutIR that stores all
//! layout data in arena-allocated slices instead of heap Vecs.
//!
//! Data types and accessors live here. Rendering is split into:
//! - `arena_render` — plain ASCII rendering
//! - `arena_colored` — ANSI-colored rendering
//! - `arena_builder` — builder for constructing from arena memory
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
    /// Center Y coordinate (for edge routing, e.g. `y + height / 2`).
    /// Mirrors zigraph's `LayoutNode.center_y`.
    pub center_y: usize,
    /// The level (depth) this node is at
    pub level: usize,
    /// Position within the level (0-indexed from left)
    pub level_position: usize,
    /// Classification: explicit, implicit, or dummy
    pub kind: NodeKind,
    /// Whether this node has a self-loop edge (A → A)
    pub has_self_loop: bool,
    /// Physical cell of the self-loop marker (temp/08 D5):
    /// `(usize::MAX, usize::MAX)` = none. Set together with
    /// `has_self_loop` by [`set_self_loop`] (which derives the legacy
    /// cell, one right of the top row) — builder-produced IRs never
    /// disagree; hand-built literals own the pair's consistency.
    /// Direction flips re-anchor it to the same node-relative corner.
    ///
    /// [`set_self_loop`]: crate::ir::arena::LayoutIRArenaBuilder::set_self_loop
    pub self_loop_at: (usize, usize),
    /// For dummy nodes: the index of the edge this dummy belongs to.
    /// `usize::MAX` = none (real node) — same sentinel convention as
    /// `SubgraphInfoArena::parent_idx`. Mirrors zigraph.
    pub edge_index: usize,
    /// Content kind declared at construction (raw `NodeKindTag`
    /// value): 0 = simple `[label]`, 1 = boxed, 2 = custom. Dummies
    /// use 0. Field parity with `LayoutNode.content_tag`; fits the
    /// struct's existing padding — no size change.
    pub content_tag: u8,
}

/// Sparse custom-content entry: painter + payload for one node
/// (arena mirror of the heap IR's custom list — field parity).
/// Payload bytes live in the IR's label storage, like labels do.
#[derive(Debug, Clone, Copy, Default)]
pub struct CustomNodeArena {
    /// Index into the IR's node array.
    pub node_idx: usize,
    /// Template fn; `None` = blank node (area reserved, unpainted).
    pub painter: Option<crate::render::engine::NodePaintFn>,
    /// Offset of the payload bytes in label storage.
    pub payload_offset: usize,
    /// Length of the payload in bytes.
    pub payload_len: usize,
}

/// Edge routing type (no heap allocation version).
#[derive(Debug, Clone, Copy)]
pub enum EdgePathArena {
    /// Direct vertical connection
    Direct,
    /// L-shaped connection with a horizontal segment
    Corner {
        /// Y coordinate of the horizontal bend.
        horizontal_y: usize,
    },
    /// Routed through a side channel (for skip-level edges).
    /// Mirrors heap `EdgePath::SideChannel`.
    SideChannel {
        /// X coordinate of the vertical channel
        channel_x: usize,
        /// Starting Y of the channel
        start_y: usize,
        /// Ending Y of the channel
        end_y: usize,
    },
    /// Multi-segment path (waypoints stored separately)
    MultiSegment {
        /// Start index into waypoints array
        waypoints_start: usize,
        /// Number of waypoints
        waypoints_len: usize,
        /// Vertical offset for the start of the edge
        start_y_offset: usize,
    },
    /// Bézier spline hint (for SVG/canvas renderers; ASCII renderers fall back to Direct).
    /// Mirrors zigraph's `EdgePath.spline`.
    Spline {
        /// First control point X
        cp1_x: usize,
        /// First control point Y
        cp1_y: usize,
        /// Second control point X
        cp2_x: usize,
        /// Second control point Y
        cp2_y: usize,
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
    /// Whether this edge has an arrowhead (true for directed edges).
    /// Mirrors zigraph's `LayoutEdge.directed`.
    pub directed: bool,
    /// Whether this edge was reversed during cycle breaking (back-edge).
    pub reversed: bool,
    /// How the edge is routed
    pub path: EdgePathArena,
    /// Which physical axis the trunk runs along (see
    /// [`FlowAxis`](crate::ir::FlowAxis)). Every TopDown/BottomUp
    /// edge is `Y`. Flips copy it verbatim (mirror-invariant).
    pub flow_axis: crate::ir::FlowAxis,
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

/// Subgraph bounding box for arena-backed layout.
///
/// Equivalent to [`SubgraphInfo`](crate::ir::SubgraphInfo) but avoids
/// `&str` lifetime by storing label offset/length into shared label storage.
#[derive(Debug, Clone, Copy)]
pub struct SubgraphInfoArena {
    /// Subgraph ID (matches CsrGraph subgraph index)
    pub id: usize,
    /// Parent subgraph index (`usize::MAX` = top-level)
    pub parent_idx: usize,
    /// Label offset into labels array
    pub label_offset: usize,
    /// Label length in bytes
    pub label_len: usize,
    /// Left edge of the bounding box (character column)
    pub x: usize,
    /// Top edge of the bounding box (line number)
    pub y: usize,
    /// Width in character cells (including borders)
    pub width: usize,
    /// Height in lines (including borders)
    pub height: usize,
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
    /// Label storage (raw bytes, UTF-8); custom payloads ride here too
    labels: &'a [u8],
    /// Sparse custom-content entries, sorted by node index
    custom_nodes: &'a [CustomNodeArena],
    /// Subgraph bounding boxes
    subgraphs: &'a [SubgraphInfoArena],
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
    /// Rank direction the layout was computed for (applied at render time).
    direction: crate::graph::Direction,
}

impl<'a> LayoutIRArena<'a> {
    /// Construct from pre-allocated parts. Used by [`LayoutIRArenaBuilder::build`].
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_parts(
        nodes: &'a [LayoutNodeArena],
        edges: &'a [LayoutEdgeArena],
        waypoints: &'a [(usize, usize)],
        labels: &'a [u8],
        subgraphs: &'a [SubgraphInfoArena],
        width: usize,
        height: usize,
        level_count: usize,
        level_offsets: &'a [usize],
        level_data: &'a [usize],
        custom_nodes: &'a [CustomNodeArena],
    ) -> Self {
        Self {
            nodes,
            edges,
            waypoints,
            labels,
            custom_nodes,
            subgraphs,
            width,
            height,
            level_count,
            level_offsets,
            level_data,
            direction: crate::graph::Direction::TopDown,
        }
    }

    /// Set the rank direction the layout was computed for.
    pub(crate) fn set_direction(&mut self, direction: crate::graph::Direction) {
        self.direction = direction;
    }

    /// Rank direction the layout was computed for.
    #[inline]
    pub fn direction(&self) -> crate::graph::Direction {
        self.direction
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

    /// Get the number of subgraphs.
    #[inline]
    pub fn subgraph_count(&self) -> usize {
        self.subgraphs.len()
    }

    /// Check if this layout has subgraphs.
    #[inline]
    pub fn has_subgraphs(&self) -> bool {
        !self.subgraphs.is_empty()
    }

    /// Get all subgraph bounding boxes.
    #[inline]
    pub fn subgraphs(&self) -> &[SubgraphInfoArena] {
        self.subgraphs
    }

    /// Get subgraph label by subgraph index.
    #[inline]
    pub fn subgraph_label(&self, index: usize) -> &str {
        let sg = &self.subgraphs[index];
        if sg.label_len == 0 {
            return "";
        }
        let bytes = &self.labels[sg.label_offset..sg.label_offset + sg.label_len];
        core::str::from_utf8(bytes).unwrap_or("")
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

    /// Sparse custom-content entries, sorted by node index.
    pub(crate) fn custom_nodes(&self) -> &[CustomNodeArena] {
        self.custom_nodes
    }

    /// A custom entry's payload text (empty on malformed UTF-8 —
    /// unreachable for builder-written payloads).
    pub(crate) fn custom_payload(&self, entry: &CustomNodeArena) -> &str {
        let bytes = &self.labels[entry.payload_offset..entry.payload_offset + entry.payload_len];
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
    ///
    /// Dummy nodes carry synthetic ids and are never returned (parity
    /// with the heap IR's `node_by_id`).
    pub fn node_by_id(&self, id: usize) -> Option<&LayoutNodeArena> {
        self.nodes
            .iter()
            .find(|n| n.id == id && !matches!(n.kind, NodeKind::Dummy))
    }

    /// Find node index by ID (linear search - O(n)).
    ///
    /// Dummy nodes carry synthetic ids and are never returned.
    pub fn node_index_by_id(&self, id: usize) -> Option<usize> {
        self.nodes
            .iter()
            .position(|n| n.id == id && !matches!(n.kind, NodeKind::Dummy))
    }

    /// Check if the layout is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    // ── Query methods (parity with LayoutIR) ─────────────────────────────

    /// Get all edges originating from a node (by node ID).
    pub fn edges_from(&self, node_id: usize) -> impl Iterator<Item = &LayoutEdgeArena> {
        self.edges.iter().filter(move |e| e.from_id == node_id)
    }

    /// Get all edges ending at a node (by node ID).
    pub fn edges_to(&self, node_id: usize) -> impl Iterator<Item = &LayoutEdgeArena> {
        self.edges.iter().filter(move |e| e.to_id == node_id)
    }

    /// Get bounding box for a node: `(x, y, width, height)`.
    #[inline]
    pub fn node_bounds(&self, node: &LayoutNodeArena) -> (usize, usize, usize, usize) {
        (node.x, node.y, node.width, node.height)
    }

    /// Find the node at a given coordinate (hit testing).
    pub fn node_at(&self, x: usize, y: usize) -> Option<&LayoutNodeArena> {
        self.nodes.iter().find(|node| {
            x >= node.x && x < node.x + node.width && y >= node.y && y < node.y + node.height
        })
    }

    /// Check if two edges cross each other.
    pub fn edges_cross(&self, edge1: &LayoutEdgeArena, edge2: &LayoutEdgeArena) -> bool {
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
        let h_overlap = min1 < max2 && min2 < max1;
        let v_overlap = edge1.from_y < edge2.to_y && edge2.from_y < edge1.to_y;
        let dir1 = edge1.to_x as isize - edge1.from_x as isize;
        let dir2 = edge2.to_x as isize - edge2.from_x as isize;
        let opposite_dir = (dir1 > 0 && dir2 < 0) || (dir1 < 0 && dir2 > 0);
        h_overlap && v_overlap && opposite_dir
    }

    /// Get a deterministic color index for an edge (for colored renderers).
    #[inline]
    pub fn edge_color_index(&self, edge: &LayoutEdgeArena) -> usize {
        edge.edge_index
    }

    // ── Alloc-gated query methods ────────────────────────────────────────

    /// Get edges that connect nodes from `from_level` to the next level.
    ///
    /// Returns `(from_center_x, to_center_x)` pairs for drawing connections.
    #[cfg(feature = "alloc")]
    pub fn edges_between_levels(&self, from_level: usize) -> alloc::vec::Vec<(usize, usize)> {
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

    /// Get all edges whose source node is at the given level.
    #[cfg(feature = "alloc")]
    pub fn edges_from_level(&self, level: usize) -> alloc::vec::Vec<&LayoutEdgeArena> {
        self.edges
            .iter()
            .filter(|edge| {
                self.node_by_id(edge.from_id)
                    .map(|n| n.level == level)
                    .unwrap_or(false)
            })
            .collect()
    }
}

/// Calculate required arena size for layout IR.
pub fn estimate_layout_arena_size(
    node_count: usize,
    edge_count: usize,
    label_bytes: usize,
    max_waypoints: usize,
) -> usize {
    estimate_layout_arena_size_with_subgraphs(node_count, edge_count, label_bytes, max_waypoints, 0)
}

/// Calculate required arena size for layout IR with subgraphs.
pub fn estimate_layout_arena_size_with_subgraphs(
    node_count: usize,
    edge_count: usize,
    label_bytes: usize,
    max_waypoints: usize,
    subgraph_count: usize,
) -> usize {
    use core::mem::size_of;

    let nodes_size = node_count * size_of::<LayoutNodeArena>();
    let edges_size = edge_count * size_of::<LayoutEdgeArena>();
    let waypoints_size = max_waypoints * size_of::<(usize, usize)>();
    let level_offsets_size = (node_count + 2) * size_of::<usize>(); // Generous estimate
    let level_data_size = node_count * size_of::<usize>();
    let subgraphs_size = subgraph_count * size_of::<SubgraphInfoArena>();

    // Add alignment padding and extra buffer
    let num_allocs = 8 + if subgraph_count > 0 { 1 } else { 0 };
    let padding = num_allocs * 8;

    nodes_size
        + edges_size
        + waypoints_size
        + level_offsets_size
        + level_data_size
        + subgraphs_size
        + label_bytes
        + padding
        + 512
}
