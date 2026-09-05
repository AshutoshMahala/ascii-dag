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

#[cfg(feature = "alloc")]
use alloc::vec;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

#[cfg(all(feature = "alloc", not(feature = "std")))]
use alloc::collections::BTreeMap as HashMap;
#[cfg(all(feature = "alloc", feature = "std"))]
use std::collections::HashMap;

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
/// `EdgePath` (`bend_at`, `channel_at`, …) live on the axis
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
    /// responsible for keeping the pair consistent). The vertical flip
    /// RE-ANCHORS the cell to the same node-relative corner
    /// (point-mapping would land it on a multi-row node's far row);
    /// the horizontal flip point-maps, which on that axis *is* the
    /// node-relative answer.
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
///
/// Non-exhaustive: shapes are added by feature (the explicit polyline
/// arrives with `ports`) and by release — match with a wildcard arm.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EdgePath {
    /// Straight flow segment — the endpoints share their cross-axis
    /// line (`flow_axis: Y`: a vertical line; `X`: a horizontal one).
    Direct,
    /// L-shaped connection with one cross-axis distribution segment.
    Corner {
        /// The level-axis line the cross segment runs on. Which
        /// physical axis that is comes from the edge's
        /// [`flow_axis`](LayoutEdge::flow_axis): a ROW for `Y` trunks,
        /// a COLUMN for `X` trunks.
        bend_at: usize,
    },
    /// Routed through a far cross-axis channel (legacy skip-edge
    /// shape; the current layout emits [`MultiSegment`](Self::MultiSegment)
    /// instead — the variant survives for hand-built IRs).
    SideChannel {
        /// Cross-axis line of the channel (`flow_axis: Y`: a column;
        /// `X`: a row).
        channel_at: usize,
        /// Level-axis start of the channel span.
        span_start: usize,
        /// Level-axis end of the channel span.
        span_end: usize,
    },
    /// Multi-segment path through dummy-node waypoints.
    MultiSegment {
        /// Physical `(x, y)` cells the edge passes through — always
        /// materialized coordinates, whatever the flow axis.
        waypoints: Vec<(usize, usize)>,
        /// Level-axis offset of the first bend past the source (keeps
        /// fan-outs from overlapping at the source band).
        start_offset: usize,
    },
    #[cfg(feature = "ports")]
    /// An explicit orthogonal polyline: the edge runs from the source
    /// endpoint through every bend in order to the target endpoint,
    /// each leg axis-aligned (consecutive points share `x` or `y`; a
    /// leg sharing neither paints nothing). Where
    /// [`MultiSegment`](Self::MultiSegment) leaves its turns to the
    /// painter — one line past each waypoint, along the flow — every
    /// turn here is stated, which is what a route that leaves a node
    /// against the flow or beside it needs. Physical `(x, y)` cells
    /// whatever the flow axis; the endpoint cells are node faces and
    /// are never painted.
    Orthogonal {
        /// The bend cells, in path order.
        bends: Vec<(usize, usize)>,
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

/// A preserved self-loop (`A → A`). Self-loops are kept OUT of the
/// routed edge list — routing passes never see them — as parallel
/// records, so their identity (original insertion index), label, and
/// style survive to the scene. The marker cell itself lives on the
/// node ([`LayoutNode::self_loop_at`]); a node with several self-loops
/// carries several records sharing that one marker.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelfLoopRecord<'a> {
    /// The node the loop is on.
    pub node_id: usize,
    /// The node's position in [`LayoutIR::nodes`] — resolved once at
    /// layout so consumers join record→node in O(1) instead of
    /// scanning by id. Hand-built IRs own its consistency.
    pub node_index: usize,
    /// Original graph insertion index (the style-callback convention —
    /// self-loops COUNT in this numbering, which is why it diverges
    /// from routed-list positions whenever they exist).
    pub edge_index: usize,
    /// Optional label.
    pub label: Option<&'a str>,
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
    /// Preserved self-loop records, in insertion order.
    pub(crate) self_loops: Vec<SelfLoopRecord<'a>>,
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
    #[cfg(feature = "layout-vertical")]
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
                EdgePath::Corner { bend_at } => *bend_at = flip_row(*bend_at),
                EdgePath::MultiSegment { waypoints, .. } => {
                    for (_, wy) in waypoints.iter_mut() {
                        *wy = flip_row(*wy);
                    }
                }
                #[cfg(feature = "ports")]
                EdgePath::Orthogonal { bends } => {
                    for (_, by) in bends.iter_mut() {
                        *by = flip_row(*by);
                    }
                }
                EdgePath::SideChannel {
                    span_start,
                    span_end,
                    ..
                } => {
                    // Mirror each in place — never swap. `span_start`
                    // is SOURCE-associated and `span_end`
                    // TARGET-associated (the compositor paints the
                    // source run at one and the target run from the
                    // other); a mirror moves them, it does not trade
                    // their roles. Unobservable in generated output —
                    // the layout never emits `SideChannel` — but wrong
                    // for hand-built IRs.
                    *span_start = flip_row(*span_start);
                    *span_end = flip_row(*span_end);
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
    }

    /// Horizontally mirror every coordinate in place (involutive).
    ///
    /// Applied once at the end of layout for `Direction::RightLeft` to
    /// turn the left-to-right logical result into physical
    /// coordinates — the x-axis twin of [`flip_vertical`].
    ///
    /// [`flip_vertical`]: Self::flip_vertical
    #[cfg(feature = "layout-horizontal")]
    pub(crate) fn flip_horizontal(&mut self) {
        let w = self.width;
        let flip_col = |x: usize| w.saturating_sub(1).saturating_sub(x);
        for node in &mut self.nodes {
            node.x = w.saturating_sub(node.x + node.width);
            node.center_x = flip_col(node.center_x);
            // Point-mapping the marker is exactly right here (unlike
            // the vertical flip, which must re-anchor): the LR cell
            // sits at the node's LEADING column, and its mirror is the
            // flipped node's trailing column — which is the leading
            // side again under right-to-left flow. Role rule and point
            // mirror coincide on this axis.
            node.self_loop_at = node.self_loop_at.map(|(mx, my)| (flip_col(mx), my));
        }
        for edge in &mut self.edges {
            edge.from_x = flip_col(edge.from_x);
            edge.to_x = flip_col(edge.to_x);
            // A label occupies a SPAN of cells, so it mirrors as one;
            // and only when it exists (an unlabeled edge's 0-default
            // must not become a right-edge coordinate).
            if let Some(text) = edge.label {
                let span = text.chars().count() + 2;
                edge.label_x = w.saturating_sub(edge.label_x + span);
            }
            // `flow_axis` is mirror-invariant (D2) and copies verbatim;
            // `start_offset` is flow-relative, so it too is unchanged.
            // The level-axis path scalars flip only when the level axis
            // IS x — i.e. for horizontal trunks.
            let x_flow = matches!(edge.flow_axis, FlowAxis::X);
            match &mut edge.path {
                EdgePath::Corner { bend_at } => {
                    if x_flow {
                        *bend_at = flip_col(*bend_at);
                    }
                }
                EdgePath::SideChannel {
                    channel_at,
                    span_start,
                    span_end,
                } => {
                    if x_flow {
                        // Both spans are columns and each mirrors in
                        // place. They are NOT swapped: `span_start` is
                        // where the SOURCE enters the channel and
                        // `span_end` where it exits toward the TARGET
                        // — roles a mirror does not exchange. The
                        // channel line is a row, untouched.
                        *span_start = flip_col(*span_start);
                        *span_end = flip_col(*span_end);
                    } else {
                        // Y trunks: the channel line is the column.
                        *channel_at = flip_col(*channel_at);
                    }
                }
                EdgePath::MultiSegment { waypoints, .. } => {
                    for (wx, _) in waypoints.iter_mut() {
                        *wx = flip_col(*wx);
                    }
                }
                #[cfg(feature = "ports")]
                EdgePath::Orthogonal { bends } => {
                    for (bx, _) in bends.iter_mut() {
                        *bx = flip_col(*bx);
                    }
                }
                // Never produced by the layout, but the flip must be
                // total: both backends transform every variant or the
                // two IRs can drift.
                EdgePath::Spline { cp1_x, cp2_x, .. } => {
                    *cp1_x = flip_col(*cp1_x);
                    *cp2_x = flip_col(*cp2_x);
                }
                EdgePath::Direct => {}
            }
        }
        for sg in &mut self.subgraphs {
            sg.x = w.saturating_sub(sg.x + sg.width);
        }
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

    /// Preserved self-loop records, in insertion order (absent from
    /// [`edges`](Self::edges) — routing never sees them).
    #[inline]
    pub fn self_loops(&self) -> &[SelfLoopRecord<'a>] {
        &self.self_loops
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
    self_loops: Vec<SelfLoopRecord<'a>>,
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

    /// Record a preserved self-loop. Pair with the node's marker cell
    /// ([`LayoutNode::self_loop_at`]); records never enter the routed
    /// edge list.
    pub fn add_self_loop(&mut self, record: SelfLoopRecord<'a>) {
        self.self_loops.push(record);
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
            self_loops: self.self_loops,
            subgraphs: self.subgraphs,
            custom_nodes: self.custom_nodes,
            width: self.width,
            height: self.height,
            level_count: self.level_count,
            levels: self.levels,
            id_to_index,
            direction: self.direction,
        }
    }
}
