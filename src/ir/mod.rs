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
//! use ascii_dag::Graph;
//!
//! let dag = Graph::from_edges(
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

pub mod arena;
pub(crate) mod arena_builder;
pub mod json;
mod legacy;

#[cfg(feature = "alloc")]
use alloc::vec;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;
#[cfg(feature = "alloc")]
use core::cell::OnceCell;

#[cfg(all(feature = "alloc", not(feature = "std")))]
use alloc::collections::BTreeMap as HashMap;
#[cfg(all(feature = "alloc", feature = "std"))]
use std::collections::HashMap;

/// Spatial index for fast scanline rendering.
/// Maps a Y coordinate to the nodes and edges that occupy that line.
#[cfg(feature = "alloc")]
#[deprecated(
    since = "0.10.0",
    note = "the render engine's `RenderPlan` (via `render_plan`/`hit_test`) replaces the scanline index"
)]
#[derive(Debug, Clone)]
pub struct LineOccupancy {
    /// Indices of nodes that appear on this line
    pub node_indices: Vec<usize>,
    /// Indices of edges that cross this line
    pub edge_indices: Vec<usize>,
}

#[cfg(feature = "alloc")]
#[allow(deprecated)]
impl LineOccupancy {
    fn new() -> Self {
        Self {
            node_indices: Vec::new(),
            edge_indices: Vec::new(),
        }
    }
}

/// Classification of a node in the layout.
///
/// Mirrors zigraph's `NodeKind` for IR parity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    /// User-defined node added explicitly via `add_node` or `from_edges`.
    Explicit,
    /// Auto-created node (e.g., referenced only in an edge but never added).
    Implicit,
    /// Dummy (virtual) node inserted by the layout algorithm for edge routing.
    Dummy,
}

/// Which physical axis an edge's trunk (flow segment) runs along
/// (temp/08 D2). Per-edge GEOMETRY set by layout — never consult
/// `Direction` to interpret a path: a corner edge's endpoints differ
/// on both axes, so orientation is not inferable from coordinates.
///
/// `Y` for vertical trunks (every TopDown/BottomUp edge); `X` for
/// horizontal trunks (LeftRight/RightLeft). Mirror-invariant: both
/// the BottomUp y-flip and the RightLeft x-flip leave the trunk axis
/// unchanged, so flips copy it verbatim. Level-axis scalars inside
/// [`EdgePath`] (`horizontal_y`, `channel_x`, …) live on the axis
/// this field names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowAxis {
    /// The trunk runs vertically; level-axis path scalars are rows.
    Y,
    /// The trunk runs horizontally; level-axis path scalars are
    /// columns.
    X,
}

/// A node in the laid-out graph with computed position and dimensions.
#[cfg(feature = "alloc")]
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
    /// Whether this node has a self-loop edge (A → A).
    /// When true, renderers paint a `↺` indicator after the node bracket.
    pub has_self_loop: bool,
    /// Physical cell of the self-loop marker, computed by layout
    /// (temp/08 D5): one cell past the node on the cross axis, at its
    /// level-leading line — for vertical flows that is `(x + width,
    /// y)`, the legacy `↺` position. `Some` iff `has_self_loop`
    /// (`has_self_loop` is DERIVED from this field at emission, so
    /// layout-generated IRs never disagree; hand-built literals are
    /// responsible for keeping the pair consistent). Direction flips
    /// re-anchor the cell to the
    /// same node-relative corner rather than point-mapping it.
    pub self_loop_at: Option<(usize, usize)>,
    /// For dummy nodes: the index of the edge this dummy belongs to.
    /// `None` for real (explicit/implicit) nodes. Mirrors zigraph's
    /// `LayoutNode.edge_index`.
    pub edge_index: Option<usize>,
    /// Content kind declared at construction (raw `NodeKindTag`
    /// value): 0 = simple `[label]`, 1 = boxed, 2 = custom. Dummies
    /// and hand-built IRs use 0. Fits the struct's existing padding —
    /// no size change.
    pub content_tag: u8,
}

/// How an edge is routed between nodes.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EdgePath {
    /// Direct vertical connection (nodes are horizontally aligned or adjacent levels)
    Direct,
    /// L-shaped connection with a horizontal segment
    Corner {
        /// Y coordinate of the horizontal segment
        horizontal_y: usize,
    },
    /// Routed through a side channel (for skip-level edges).
    ///
    /// **Note:** The layout engine produces this variant for edges that skip multiple
    /// levels. ASCII renderers render the full L-shaped channel routing. SVG/canvas
    /// renderers may use the coordinates directly.
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
        /// Vertical offset for the start of the edge (to prevent overlaps at source)
        start_y_offset: usize,
    },
    /// Bézier spline hint (for SVG/canvas renderers; ASCII renderers fall back to Direct).
    ///
    /// **Status: forward-compatible stub.** The layout engine does not currently produce
    /// this variant. It exists so that custom IR builders and zigraph imports can
    /// round-trip spline data without breaking changes when native spline routing is added.
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

/// A routed edge in the layout with path information.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct LayoutEdge<'a> {
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
    /// Which physical axis the trunk runs along (see [`FlowAxis`]).
    /// Every TopDown/BottomUp edge is `Y`.
    pub flow_axis: FlowAxis,
    /// Edge index (for consistent coloring)
    pub edge_index: usize,
    /// Optional edge label (e.g., "depends on", "uses")
    pub label: Option<&'a str>,
    /// Computed X position for rendering the label.
    /// Meaningful **iff `label` is present** (0 otherwise) — the same
    /// scalar shape as `LayoutEdgeArena`, zigraph, and the JSON format.
    pub label_x: usize,
    /// Computed Y position for rendering the label.
    /// Meaningful **iff `label` is present** (0 otherwise).
    pub label_y: usize,
    /// Whether this edge has an arrowhead (true for directed edges).
    /// Mirrors zigraph's `LayoutEdge.directed`.
    pub directed: bool,
    /// Whether this edge was reversed during cycle breaking.
    /// When true, the edge represents a back-edge that was temporarily
    /// flipped for layering; renderers should draw it with dashed lines.
    /// Mirrors zigraph's `LayoutEdge.reversed`.
    pub reversed: bool,
}

/// Bounding box and metadata for a laid-out subgraph (cluster).
///
/// Produced by the layout algorithm after coordinate assignment.
/// Renderers use this to draw box-drawing borders with a label.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct SubgraphInfo<'a> {
    /// Subgraph ID (matches [`Subgraph::id`](crate::graph::Subgraph::id)).
    pub id: usize,
    /// Parent subgraph ID (`None` = top-level cluster).
    pub parent_id: Option<usize>,
    /// Display label for the border.
    pub label: &'a str,
    /// Left edge of the bounding box (character column).
    pub x: usize,
    /// Top edge of the bounding box (line number).
    pub y: usize,
    /// Width in character cells (including borders).
    pub width: usize,
    /// Height in lines (including borders).
    pub height: usize,
}

/// Intermediate representation of a laid-out graph.
///
/// This is the output of the layout algorithm and input to renderers.
/// It contains all the information needed to draw the graph in any format.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct LayoutIR<'a> {
    /// All nodes with their computed positions
    pub(crate) nodes: Vec<LayoutNode<'a>>,
    /// All edges with routing information
    pub(crate) edges: Vec<LayoutEdge<'a>>,
    /// All subgraph bounding boxes (empty when no subgraphs)
    pub(crate) subgraphs: Vec<SubgraphInfo<'a>>,
    /// Sparse custom-content entries (node index, painter, payload),
    /// sorted by node index — field parity with the arena IR
    pub(crate) custom_nodes: Vec<(usize, Option<crate::render::engine::NodePaintFn>, &'a str)>,
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
    /// Rank direction the layout was computed for.
    pub(crate) direction: crate::graph::Direction,
    /// Spatial index for fast scanline rendering (built lazily on first access)
    #[allow(deprecated)]
    y_index: OnceCell<Vec<LineOccupancy>>,
}

#[cfg(feature = "alloc")]
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

    /// Rank direction the layout was computed for.
    ///
    /// IR coordinates are physical — for `Direction::BottomUp` they are
    /// already flipped so they match rendered cells.
    #[inline]
    pub fn direction(&self) -> crate::graph::Direction {
        self.direction
    }

    /// Vertically mirror every coordinate in place (involutive).
    ///
    /// Applied once at the end of layout for `Direction::BottomUp` to turn
    /// the top-down logical result into physical coordinates.
    pub(crate) fn flip_vertical(&mut self) {
        let h = self.height;
        let flip_row = |y: usize| h.saturating_sub(1).saturating_sub(y);
        for node in &mut self.nodes {
            node.y = h.saturating_sub(node.y + node.height);
            node.center_y = flip_row(node.center_y);
            // Re-anchor (not point-map): the marker stays one cell right
            // of the FINAL top row — the engine's direction-blind rule.
            node.self_loop_at = node.self_loop_at.map(|_| (node.x + node.width, node.y));
        }
        for edge in &mut self.edges {
            edge.from_y = flip_row(edge.from_y);
            edge.to_y = flip_row(edge.to_y);
            // label_y is only meaningful when a label exists; the 0-default
            // of unlabeled edges must not turn into a bottom-row garbage value.
            if edge.label.is_some() {
                edge.label_y = flip_row(edge.label_y);
            }
            match &mut edge.path {
                EdgePath::Corner { horizontal_y } => *horizontal_y = flip_row(*horizontal_y),
                EdgePath::MultiSegment { waypoints, .. } => {
                    for (_, wy) in waypoints.iter_mut() {
                        *wy = flip_row(*wy);
                    }
                }
                EdgePath::SideChannel { start_y, end_y, .. } => {
                    let (s, e) = (flip_row(*end_y), flip_row(*start_y));
                    *start_y = s;
                    *end_y = e;
                }
                // Spline is never produced by the layout engine today, but
                // the flip must be total: both backends apply the identical
                // transform to every path variant or their IRs can drift.
                EdgePath::Spline { cp1_y, cp2_y, .. } => {
                    *cp1_y = flip_row(*cp1_y);
                    *cp2_y = flip_row(*cp2_y);
                }
                EdgePath::Direct => {}
            }
        }
        for sg in &mut self.subgraphs {
            sg.y = h.saturating_sub(sg.y + sg.height);
        }
        self.y_index = OnceCell::new();
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
    pub fn edges(&self) -> &[LayoutEdge<'a>] {
        &self.edges
    }

    /// Get all laid-out subgraph bounding boxes.
    ///
    /// Empty if the graph has no subgraphs.
    #[inline]
    pub fn subgraphs(&self) -> &[SubgraphInfo<'a>] {
        &self.subgraphs
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
    pub fn edges_from(&self, node_id: usize) -> impl Iterator<Item = &LayoutEdge<'a>> {
        self.edges.iter().filter(move |e| e.from_id == node_id)
    }

    /// Get all edges ending at a node.
    pub fn edges_to(&self, node_id: usize) -> impl Iterator<Item = &LayoutEdge<'a>> {
        self.edges.iter().filter(move |e| e.to_id == node_id)
    }

    /// Get or build the spatial Y-index for fast scanline rendering.
    /// This is computed lazily on first access and cached for subsequent calls.
    ///
    /// Returns a slice where index `y` contains all nodes and edges that occupy line `y`.
    #[deprecated(
        since = "0.10.0",
        note = "use `render_plan(&options)` + `hit_test` — the engine's spatial queries"
    )]
    #[allow(deprecated)]
    pub fn y_index(&self) -> &[LineOccupancy] {
        self.y_index.get_or_init(|| self.build_y_index())
    }

    /// Build the Y-index spatial structure.
    /// Maps each Y coordinate to the nodes and edges that appear on that line.
    #[allow(deprecated)]
    fn build_y_index(&self) -> Vec<LineOccupancy> {
        let mut index = vec![LineOccupancy::new(); self.height];

        // Index nodes: each node occupies `height` lines starting at its Y coordinate
        for (node_idx, node) in self.nodes.iter().enumerate() {
            for y in node.y..node.y + node.height {
                if y < self.height {
                    index[y].node_indices.push(node_idx);
                }
            }
        }

        // Index edges: each edge may span multiple lines
        for (edge_idx, edge) in self.edges.iter().enumerate() {
            let min_y = edge.from_y.min(edge.to_y);
            let max_y = edge.from_y.max(edge.to_y);

            for y in min_y..=max_y {
                if y < self.height {
                    index[y].edge_indices.push(edge_idx);
                }
            }
        }

        index
    }

    /// Get items that occupy a specific line (fast lookup with Y-index).
    /// Returns node and edge indices for the given Y coordinate.
    #[deprecated(
        since = "0.10.0",
        note = "use `render_plan(&options)` + `hit_test` — the engine's spatial queries"
    )]
    #[allow(deprecated)]
    pub fn items_at_line(&self, y: usize) -> Option<&LineOccupancy> {
        self.y_index().get(y)
    }

    /// Check if two edges cross each other.
    /// Useful for advanced renderers that want to handle crossings specially.
    pub fn edges_cross(&self, edge1: &LayoutEdge<'a>, edge2: &LayoutEdge<'a>) -> bool {
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
    pub fn edge_color_index(&self, edge: &LayoutEdge<'a>) -> usize {
        // Use the edge's index modulo a palette size
        // This ensures the same edge always gets the same color
        edge.edge_index
    }

    /// Compute optimal color indices for all edges using greedy graph coloring.
    ///
    /// Adjacent edges (those sharing a source or target node) are assigned different
    /// colors when possible. This reduces visual confusion in complex graphs.
    ///
    /// Returns a Vec where index i is the color index for edge i.
    /// Returns a Vec where index i is the color index for edge i.
    pub fn compute_edge_colors(&self, palette_size: usize) -> Vec<usize> {
        let n = self.edges.len();
        if n == 0 || palette_size == 0 {
            return vec![0; n];
        }

        // Fast O(E) modulo coloring
        // Since the palette is now interleaved (Warm/Cool/Light/Dark),
        // sequential indices will be visually distinct.
        (0..n).map(|i| i % palette_size).collect()
    }

    /// Get bounding box for a node (x, y, width, height).
    /// Useful for hit testing in interactive renderers.
    pub fn node_bounds(&self, node: &LayoutNode) -> (usize, usize, usize, usize) {
        (node.x, node.y, node.width, node.height)
    }

    /// Find the node at a given coordinate (for mouse interaction).
    /// Returns None if no node is at that position.
    pub fn node_at(&self, x: usize, y: usize) -> Option<&LayoutNode<'a>> {
        self.nodes.iter().find(|node| {
            x >= node.x && x < node.x + node.width && y >= node.y && y < node.y + node.height
        })
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
    pub fn edges_from_level(&self, level: usize) -> Vec<&LayoutEdge<'a>> {
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
#[cfg(feature = "alloc")]
#[derive(Debug, Default)]
pub struct LayoutIRBuilder<'a> {
    nodes: Vec<LayoutNode<'a>>,
    edges: Vec<LayoutEdge<'a>>,
    subgraphs: Vec<SubgraphInfo<'a>>,
    width: usize,
    height: usize,
    level_count: usize,
    levels: Vec<Vec<usize>>,
    direction: crate::graph::Direction,
    custom_nodes: Vec<(usize, Option<crate::render::engine::NodePaintFn>, &'a str)>,
}

#[cfg(feature = "alloc")]
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

    /// Attach custom content (painter + payload) to the most recently
    /// added node. Entries stay sorted by node index because emission
    /// appends nodes in order.
    pub fn add_custom_for_last(
        &mut self,
        painter: Option<crate::render::engine::NodePaintFn>,
        payload: &'a str,
    ) {
        if let Some(idx) = self.nodes.len().checked_sub(1) {
            self.custom_nodes.push((idx, painter, payload));
        }
    }

    /// Add an edge to the layout.
    pub fn add_edge(&mut self, edge: LayoutEdge<'a>) {
        self.edges.push(edge);
    }

    /// Add a subgraph bounding box to the layout.
    pub fn add_subgraph(&mut self, info: SubgraphInfo<'a>) {
        self.subgraphs.push(info);
    }

    /// Set the total dimensions.
    pub fn set_dimensions(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
    }

    /// Set the rank direction the layout was computed for.
    pub fn set_direction(&mut self, direction: crate::graph::Direction) {
        self.direction = direction;
    }

    /// Build the final LayoutIR.
    pub fn build(self) -> LayoutIR<'a> {
        // Build id-to-index map for O(1) lookups. Dummy nodes carry
        // synthetic ids and are deliberately excluded — `node_by_id`
        // never returns a dummy.
        let id_to_index: HashMap<usize, usize> = self
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| !matches!(node.kind, NodeKind::Dummy))
            .map(|(idx, node)| (node.id, idx))
            .collect();

        LayoutIR {
            nodes: self.nodes,
            edges: self.edges,
            subgraphs: self.subgraphs,
            custom_nodes: self.custom_nodes,
            width: self.width,
            height: self.height,
            level_count: self.level_count,
            levels: self.levels,
            id_to_index,
            direction: self.direction,
            y_index: OnceCell::new(),
        }
    }
}
