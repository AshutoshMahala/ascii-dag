//! Style vocabulary + per-element style contexts (temp/06 §8, R3/Q4).
//!
//! Styles are resolved **once per element at plan time** (the Q4 rule:
//! never per cell) by plain fn pointers — `no_std`-safe, `Copy`,
//! override-friendly. Defaults reproduce today's output exactly (R2.1).
//! The full v1 styling surface is exposed publicly at RW7; until then
//! these types serve the plan internally.

use super::cell::Weight;
use super::color::CellColor;
use super::region::{NodePaintCtx, NodeRegion};

/// Stroke weight for an edge, resolved to cell arms by the compositor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum LineWeight {
    /// Light strokes (`─ │`) — the default.
    #[default]
    Light,
    /// Dashed strokes (`┈ ┊`) — the legacy rendering for back edges.
    Dashed,
    /// Double strokes (`═ ║`).
    Double,
}

impl LineWeight {
    /// The cell-arm weight for this line weight.
    pub(crate) fn arm(self) -> Weight {
        match self {
            LineWeight::Light => Weight::Light,
            LineWeight::Dashed => Weight::Dashed,
            LineWeight::Double => Weight::Double,
        }
    }
}

/// Marker shape at an edge endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum MarkerShape {
    /// No marker.
    None,
    /// Arrowhead (`↓ ↑ → ←`, dashed variants for back edges).
    #[default]
    Arrow,
}

/// A custom node painter: fills the node's declared `width × height`
/// area through the clipped, node-local [`NodeRegion`]. Plain `fn`
/// pointer (`no_std`-safe, non-capturing): derive content from the
/// context (id, label, dimensions).
pub type NodePaintFn = fn(&mut NodeRegion<'_, '_>, NodePaintCtx<'_>);

/// Subgraph (cluster) box border style.
///
/// `None` is not "invisible cluster" — the box still groups its nodes
/// for layout and its label still paints; only the border ink is
/// suppressed.
///
/// ```
/// use ascii_dag::{Graph, RenderOptions};
/// use ascii_dag::render::engine::{SubgraphBorder, SubgraphStyle, SubgraphStyleCtx};
///
/// fn borderless(_ctx: SubgraphStyleCtx<'_>) -> SubgraphStyle {
///     SubgraphStyle { border: SubgraphBorder::None, ..SubgraphStyle::default() }
/// }
///
/// let mut g = Graph::new();
/// g.add_node(1, "A");
/// g.add_node(2, "B");
/// g.add_edge(1, 2, None);
/// let sg = g.add_subgraph("Group");
/// g.put_nodes(&[1]).inside(sg).unwrap();
///
/// let mut options = RenderOptions::plain();
/// options.subgraph_style_fn = borderless;
/// let text = g.compute_layout().render_string(&options);
/// assert!(!text.contains('╔'));      // no border ink
/// assert!(text.contains("Group"));   // label still there
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum SubgraphBorder {
    /// Double strokes (`═ ║`) — the default (legacy).
    #[default]
    Double,
    /// Light strokes (`─ │`).
    Light,
    /// Dashed strokes (`┈ ┊`).
    Dashed,
    /// No border — the cluster groups its nodes without ink (its label
    /// still paints when it fits).
    None,
}

impl SubgraphBorder {
    /// The cell-arm weight for this border, `None` when invisible.
    pub(crate) fn arm(self) -> Option<Weight> {
        match self {
            SubgraphBorder::Double => Some(Weight::Double),
            SubgraphBorder::Light => Some(Weight::Light),
            SubgraphBorder::Dashed => Some(Weight::Dashed),
            SubgraphBorder::None => Option::None,
        }
    }
}

/// Subgraph label position (zigraph naming; D7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum LabelPosition {
    /// One row below the top border, left-aligned (legacy).
    #[default]
    InsideTop,
    /// One row above the bottom border, left-aligned.
    InsideBottom,
}

/// Edge label placement strategy (zigraph naming).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum LabelPlacement {
    /// The layout-computed position (legacy).
    #[default]
    Auto,
}

// ── Style structs (what a style fn returns) ─────────────────────────────

/// Resolved style for one edge.
#[derive(Debug, Clone, Copy)]
pub struct EdgeStyle {
    /// `CellColor::DEFAULT` = engine default (palette modulo when colors
    /// are enabled, plain otherwise).
    pub color: CellColor,
    /// `None` = engine default (light; dashed for back edges).
    pub weight: Option<LineWeight>,
    /// Marker at the logical target end (the arrowhead — legacy always
    /// paints this).
    pub marker_end: MarkerShape,
    /// Marker at the logical source end (legacy never paints one, so
    /// the default is `MarkerShape::None`; `Arrow` points back at the
    /// source, giving double-headed edges).
    pub marker_start: MarkerShape,
}

// Manual impl: a derived Default would give `marker_start` the enum's
// default (`Arrow`) and paint tail arrowheads legacy never had.
impl Default for EdgeStyle {
    fn default() -> Self {
        Self {
            color: CellColor::DEFAULT,
            weight: None,
            marker_end: MarkerShape::Arrow,
            marker_start: MarkerShape::None,
        }
    }
}

/// Resolved style for one subgraph.
#[derive(Debug, Clone, Copy, Default)]
pub struct SubgraphStyle {
    /// Box border form (`Double` = legacy default; `None` = invisible).
    pub border: SubgraphBorder,
    /// Border color (`DEFAULT` = terminal default — legacy borders
    /// never write color).
    pub color: CellColor,
    /// Label position within the box.
    pub label_pos: LabelPosition,
}

/// Resolved style for one edge label.
#[derive(Debug, Clone, Copy, Default)]
pub struct EdgeLabelStyle {
    /// `DEFAULT` = follow the edge's color (legacy colored behavior).
    pub color: CellColor,
    /// Placement strategy.
    pub placement: LabelPlacement,
}

// ── Style contexts (what a style fn receives) ───────────────────────────

/// Context for `edge_style_fn`.
#[derive(Debug, Clone, Copy)]
pub struct EdgeStyleCtx<'a> {
    /// Index of this edge (original insertion order).
    pub edge_index: usize,
    /// Semantic source node id.
    pub from_id: usize,
    /// Semantic target node id.
    pub to_id: usize,
    /// The edge's label, if any.
    pub label: Option<&'a str>,
    /// Whether the edge is directed (arrowhead).
    pub directed: bool,
    /// Whether the edge was reversed during cycle breaking.
    pub reversed: bool,
    /// Total number of edges in the layout.
    pub total_edges: usize,
}

/// Context for `subgraph_style_fn`.
#[derive(Debug, Clone, Copy)]
pub struct SubgraphStyleCtx<'a> {
    /// The subgraph's id.
    pub subgraph_id: usize,
    /// The subgraph's display label.
    pub label: &'a str,
    /// Whether this cluster is nested inside another.
    pub has_parent: bool,
}

/// Per-element style callbacks — plain fn pointers (`no_std`-safe).
///
/// Resolved once per element at plan time, never per cell. A plain
/// `fn` rather than a closure keeps them `Copy` and usable without an
/// allocator.
///
/// ```
/// use ascii_dag::{Graph, RenderOptions};
/// use ascii_dag::render::engine::{EdgeStyle, EdgeStyleCtx, LineWeight, MarkerShape};
///
/// // Dash the edges that cycle-breaking reversed; suppress tail arrows.
/// fn style(ctx: EdgeStyleCtx<'_>) -> EdgeStyle {
///     EdgeStyle {
///         weight: Some(if ctx.reversed { LineWeight::Dashed } else { LineWeight::Light }),
///         marker_end: MarkerShape::Arrow,
///         marker_start: MarkerShape::None,
///         ..EdgeStyle::default()
///     }
/// }
///
/// let g = Graph::from_edges(&[(1, "A"), (2, "B")], &[(1, 2)]);
/// let mut options = RenderOptions::plain();
/// options.edge_style_fn = style;
/// let text = g.compute_layout().render_string(&options);
/// # #[cfg(feature = "layout-vertical")] // arrow glyph is axis-specific
/// assert!(text.contains('↓'));
/// ```
pub type EdgeStyleFn = fn(EdgeStyleCtx<'_>) -> EdgeStyle;
/// See [`EdgeStyleFn`].
pub type SubgraphStyleFn = fn(SubgraphStyleCtx<'_>) -> SubgraphStyle;
/// See [`EdgeStyleFn`].
pub type EdgeLabelStyleFn = fn(EdgeStyleCtx<'_>) -> EdgeLabelStyle;

/// Default edge style: engine defaults everywhere (legacy output).
/// The arrowhead honors the IR's `directed` flag — undirected edges
/// paint as plain lines.
pub fn default_edge_style(ctx: EdgeStyleCtx<'_>) -> EdgeStyle {
    EdgeStyle {
        marker_end: if ctx.directed {
            MarkerShape::Arrow
        } else {
            MarkerShape::None
        },
        ..EdgeStyle::default()
    }
}

/// Default subgraph style: label inside-top, terminal default color.
pub fn default_subgraph_style(_ctx: SubgraphStyleCtx<'_>) -> SubgraphStyle {
    SubgraphStyle::default()
}

/// Default edge-label style: follow the edge color, auto placement.
pub fn default_edge_label_style(_ctx: EdgeStyleCtx<'_>) -> EdgeLabelStyle {
    EdgeLabelStyle::default()
}
