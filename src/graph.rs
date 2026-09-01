//! Core graph data structure.
//!
//! This module provides the fundamental graph structure with nodes and edges.
//! The primary type is `Graph` (feature `alloc`).
//!
//! ## Performance Characteristics
//!
//! - **Node/Edge Insertion**: O(1) amortized with HashMap and cached adjacency lists
//! - **Child/Parent Lookups**: O(1) via cached adjacency lists (not O(E) iteration)
//! - **ID→Index Mapping**: O(1) via HashMap (not O(N) scan)
//! - **Node Width**: O(1) via pre-computed cache
//!
//! ## Memory Overhead
//!
//! Per node:
//! - ~100 bytes (node data, caches, adjacency list headers)
//!
//! Per edge:
//! - ~16 bytes (adjacency list entries, both directions)
//!
//! ## Security
//!
//! - No unsafe code
//! - For untrusted input, consider limiting maximum nodes/edges to prevent resource exhaustion
//! - Maximum node ID: `usize::MAX` (up to 20 decimal digits)

pub mod arena;
pub mod csr;

/// Rendering mode for the DAG visualization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderMode {
    /// Render chains vertically (takes more vertical space)
    Vertical,

    /// Render chains horizontally when possible (compact, one-line for simple chains)
    Horizontal,

    /// Auto-detect: horizontal for simple chains, vertical for complex graphs
    #[default]
    Auto,
}

/// Rank direction — the axis levels flow along.
///
/// Recorded on the layout IR; IR coordinates are physical (they match
/// rendered cells). For `BottomUp`, the heap layout path flips its
/// result into physical coordinates.
///
/// Parses from the conventional short forms (case-insensitive):
/// `"TB"`/`"TD"`, `"BT"`, `"LR"`, `"RL"`.
///
/// All four directions are laid out natively and painted through the
/// same geometry-driven primitives. `TopDown`/`BottomUp` stack levels
/// as rows; `LeftRight`/`RightLeft` make levels COLUMNS — sized by
/// node widths, with edges running in horizontal trunks — so a wide,
/// shallow graph reads better sideways. `BottomUp` and `RightLeft`
/// are exact mirrors of their counterparts, applied to the finished
/// layout, so IR coordinates always match rendered cells.
/// Variants are gated by the axis features: `TopDown`/`BottomUp` exist
/// under `layout-vertical`, `LeftRight`/`RightLeft` under
/// `layout-horizontal` (both are default features). A disabled
/// direction is a compile error, never a runtime fallback. The enum is
/// `#[non_exhaustive]` so a feature union adding variants can never
/// break a downstream exhaustive `match` — always keep a wildcard arm.
///
/// The default is the first enabled axis, vertical before horizontal:
/// both/vertical-only → `TopDown`, horizontal-only → `LeftRight`.
/// It re-resolves with the feature set at **compile time**: code that
/// relies on the default automatically picks up the other axis's
/// default when the enabled axes change — no code changes needed. The
/// flip side: a dependency's feature union that enables an axis your
/// crate did not ask for silently changes the resolved default (and
/// with it the rendered orientation). That is why libraries should set
/// a direction explicitly; the default is an application-level
/// convenience.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Direction {
    /// Levels flow top → bottom (default; edges point down).
    #[cfg(feature = "layout-vertical")]
    TopDown,
    /// Levels flow bottom → top (edges point up).
    #[cfg(feature = "layout-vertical")]
    BottomUp,
    /// Levels flow left → right (edges point right).
    #[cfg(feature = "layout-horizontal")]
    LeftRight,
    /// Levels flow right → left (edges point left).
    #[cfg(feature = "layout-horizontal")]
    RightLeft,
}

impl Direction {
    /// The feature-dependent default (first enabled axis, vertical
    /// before horizontal) as a `const` — usable in `const fn` preset
    /// constructors, and what `Default` delegates to. Resolved at
    /// compile time from the enabled features: change the axis set and
    /// every default-relying call site follows automatically.
    #[cfg(feature = "layout-vertical")]
    pub const DEFAULT: Direction = Direction::TopDown;
    /// The feature-dependent default (first enabled axis, vertical
    /// before horizontal) as a `const` — usable in `const fn` preset
    /// constructors, and what `Default` delegates to. Resolved at
    /// compile time from the enabled features: change the axis set and
    /// every default-relying call site follows automatically.
    #[cfg(all(feature = "layout-horizontal", not(feature = "layout-vertical")))]
    pub const DEFAULT: Direction = Direction::LeftRight;
}

impl Default for Direction {
    fn default() -> Self {
        Direction::DEFAULT
    }
}

/// Compile-time guidance marker: if this type shows up in one of your
/// build errors, the direction you named exists but is behind a
/// disabled cargo feature of ascii-dag — the accompanying deprecation
/// note names the feature to enable.
#[cfg(not(all(feature = "layout-vertical", feature = "layout-horizontal")))]
#[doc(hidden)]
#[derive(Debug, Clone, Copy)]
pub struct DisabledDirectionSeeDeprecationNote;

/// Guidance shims: same names as the feature-disabled variants, so a
/// use site resolves to THIS const instead of dying with a bare
/// "variant not found" — the type error plus the deprecation note tell
/// the user exactly which feature to enable.
#[cfg(not(feature = "layout-horizontal"))]
#[doc(hidden)]
#[allow(non_upper_case_globals)] // shim names must match the gated variants
impl Direction {
    #[deprecated = "`Direction::LeftRight` needs ascii-dag's `layout-horizontal` cargo feature — enable it (it is in the default feature set)"]
    pub const LeftRight: DisabledDirectionSeeDeprecationNote = DisabledDirectionSeeDeprecationNote;
    #[deprecated = "`Direction::RightLeft` needs ascii-dag's `layout-horizontal` cargo feature — enable it (it is in the default feature set)"]
    pub const RightLeft: DisabledDirectionSeeDeprecationNote = DisabledDirectionSeeDeprecationNote;
}

#[cfg(not(feature = "layout-vertical"))]
#[doc(hidden)]
#[allow(non_upper_case_globals)] // shim names must match the gated variants
impl Direction {
    #[deprecated = "`Direction::TopDown` needs ascii-dag's `layout-vertical` cargo feature — enable it (it is in the default feature set)"]
    pub const TopDown: DisabledDirectionSeeDeprecationNote = DisabledDirectionSeeDeprecationNote;
    #[deprecated = "`Direction::BottomUp` needs ascii-dag's `layout-vertical` cargo feature — enable it (it is in the default feature set)"]
    pub const BottomUp: DisabledDirectionSeeDeprecationNote = DisabledDirectionSeeDeprecationNote;
}

/// Error returned when parsing a [`Direction`] from an unknown string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseDirectionError;

impl core::fmt::Display for ParseDirectionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        #[cfg(all(feature = "layout-vertical", feature = "layout-horizontal"))]
        return f.write_str("unknown direction (expected TB/TD, BT, LR, or RL)");
        #[cfg(all(feature = "layout-vertical", not(feature = "layout-horizontal")))]
        return f.write_str(
            "unknown direction (expected TB/TD or BT; LR/RL need ascii-dag's \
             `layout-horizontal` feature)",
        );
        #[cfg(all(feature = "layout-horizontal", not(feature = "layout-vertical")))]
        return f.write_str(
            "unknown direction (expected LR or RL; TB/TD/BT need ascii-dag's \
             `layout-vertical` feature)",
        );
    }
}

impl core::str::FromStr for Direction {
    type Err = ParseDirectionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Untyped input is rejected at this boundary; a string naming a
        // feature-disabled axis parses like any unknown string (the
        // typed variant does not exist to return).
        #[cfg(feature = "layout-vertical")]
        if s.eq_ignore_ascii_case("TB") || s.eq_ignore_ascii_case("TD") {
            return Ok(Direction::TopDown);
        }
        #[cfg(feature = "layout-vertical")]
        if s.eq_ignore_ascii_case("BT") {
            return Ok(Direction::BottomUp);
        }
        #[cfg(feature = "layout-horizontal")]
        if s.eq_ignore_ascii_case("LR") {
            return Ok(Direction::LeftRight);
        }
        #[cfg(feature = "layout-horizontal")]
        if s.eq_ignore_ascii_case("RL") {
            return Ok(Direction::RightLeft);
        }
        Err(ParseDirectionError)
    }
}

// Everything below requires the `alloc` feature (Vec, String, HashMap).
#[cfg(feature = "alloc")]
use alloc::{string::String, vec, vec::Vec};

#[cfg(feature = "alloc")]
use crate::algorithms::sugiyama::config::LayoutConfig;
#[cfg(feature = "alloc")]
#[allow(deprecated)]
use crate::algorithms::sugiyama::config::SugiyamaConfig;
#[cfg(feature = "alloc")]
use crate::algorithms::sugiyama::crossing::CrossingReducer;
#[cfg(feature = "alloc")]
use crate::errors::GraphError;
#[cfg(feature = "alloc")]
use crate::render::engine::{NodeContent, NodeKindTag, NodePaintFn};

/// A named subgraph (cluster) for visual grouping.
///
/// Subgraphs define logical clusters that the layout engine renders with
/// box-drawing borders.  They can be nested (a subgraph may have a parent).
///
/// # Zero-Cost When Unused
///
/// If no subgraphs are added, the layout engine skips all subgraph-related
/// processing — there is no per-node overhead.
///
/// # Examples
///
/// ```
/// use ascii_dag::graph::Graph;
///
/// let mut g = Graph::new();
/// g.add_node(1, "Server");
/// g.add_node(2, "Database");
/// g.add_edge(1, 2, None);
///
/// let backend = g.add_subgraph("Backend");
/// g.put_nodes(&[1, 2]).inside(backend).unwrap();
/// ```
#[cfg(feature = "alloc")]
#[derive(Clone, Debug)]
pub struct Subgraph<'a> {
    /// Unique identifier (assigned by [`Graph::add_subgraph`]).
    pub id: usize,
    /// Display label rendered on the top border.
    pub label: &'a str,
    /// Parent subgraph ID for nesting (`None` = top-level).
    pub parent_id: Option<usize>,
}

#[cfg(all(feature = "alloc", feature = "std"))]
use std::collections::{HashMap, HashSet};

#[cfg(all(feature = "alloc", not(feature = "std")))]
use alloc::collections::{BTreeMap as HashMap, BTreeSet as HashSet};

// ── Node handles & the AUTO sentinel ─────────────────────────────────────

/// A typed handle to a node, returned by `Graph::add_node`.
///
/// Handles flow back into the edge and subgraph APIs (which accept
/// `impl Into<NodeId>`), so graphs can be built end-to-end without
/// hand-tracked integer ids:
///
/// ```
/// use ascii_dag::{Graph, AUTO};
///
/// let mut g = Graph::new();
/// let a = g.add_node(AUTO, "A");
/// let b = g.add_node(AUTO, "B");
/// g.add_edge(a, b, None);
/// ```
///
/// Raw `usize` ids keep working everywhere — `NodeId` is the safer
/// *recommended* path, not a fence: it converts from and to `usize`
/// freely and **carries no graph provenance** — a handle obtained from
/// graph A is accepted by graph B, where it names whatever node has
/// that id there (possibly none, in which case edge endpoints
/// auto-create a placeholder as raw ids always have).
///
/// The handle vocabulary (`NodeId`, [`Auto`]/[`AUTO`], [`IdOrAuto`])
/// is allocation-free and available in every build, `no_std`
/// included — a no-alloc component can produce and pass handles
/// (e.g. `(NodeId, NodeId)` edge pairs) for an alloc-enabled host to
/// assemble into a `Graph`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(usize);

impl NodeId {
    /// The raw id this handle names.
    #[inline]
    pub fn id(self) -> usize {
        self.0
    }
}

impl From<usize> for NodeId {
    #[inline]
    fn from(id: usize) -> Self {
        NodeId(id)
    }
}

impl From<NodeId> for usize {
    #[inline]
    fn from(handle: NodeId) -> usize {
        handle.0
    }
}

/// The type of the [`AUTO`] sentinel — see `Graph::add_node`.
#[derive(Debug, Clone, Copy)]
pub struct Auto;

/// Receipt for one [`Graph::add_edge`] call — always returned, uniform
/// in shape: an edge was always inserted, and the booleans answer the
/// one question the caller cannot already answer (did an endpoint get
/// auto-created?). Deliberately NOT `#[must_use]`: a receipt is
/// branchable domain data, not a warning — ignoring it is legitimate,
/// and statement-position calls compile unchanged.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EdgeInsertion {
    /// The new edge's input index (insertion order) — the same
    /// identity style callbacks, diagnostics, and
    /// `EdgeView::input_index` use.
    pub edge: usize,
    /// Whether the source endpoint was auto-created as a placeholder.
    pub created_source: bool,
    /// Whether the target endpoint was auto-created as a placeholder.
    pub created_target: bool,
}

/// How [`Graph::add_edge`] treats an edge endpoint that was never
/// declared. Declaring the policy — even the default — is intent:
/// placeholder creation then stops warning (the [`EdgeInsertion`]
/// receipt remains the record). A rejecting policy is an additive
/// future variant.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MissingNodePolicy {
    /// The 0.10-compatible behavior: auto-create a placeholder
    /// (`NodeOrigin::EdgeInferred`), recorded in the receipt.
    #[default]
    AutoCreate,
}

/// Receipt for one [`Graph::add_node`] call — always returned,
/// uniform in shape: the node's handle (`AUTO` callers read their
/// assigned id here) plus whether the call replaced an existing node.
/// Deliberately NOT `#[must_use]` (statement-position calls compile
/// unchanged), and it converts into [`NodeId`], so handles keep
/// flowing into `add_edge`/`put_nodes` exactly as before.
///
/// Replace-on-duplicate is the standing semantic;
/// `replaced_involving_auto` marks the variant worth attention — an
/// explicit id overwriting an auto-numbered node, or a saturated
/// `AUTO` overwriting an existing one. That condition is delivered
/// here, at the call site, because a replacement is a point EVENT:
/// it is not derivable from later graph state, and the graph stores
/// no diagnostic history (0.10's `W.Graph.Node.007` stderr warning
/// is this receipt's predecessor).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeInsertion {
    /// The inserted node's handle (the assigned id for `AUTO`).
    pub node: NodeId,
    /// Whether an existing node with this id was replaced.
    pub replaced: bool,
    /// Whether the replacement involved `AUTO` numbering on either
    /// side — the variant worth attention.
    pub replaced_involving_auto: bool,
}

impl From<NodeInsertion> for NodeId {
    #[inline]
    fn from(receipt: NodeInsertion) -> NodeId {
        receipt.node
    }
}

impl From<NodeInsertion> for IdOrAuto {
    #[inline]
    fn from(receipt: NodeInsertion) -> IdOrAuto {
        IdOrAuto::Id(receipt.node.0)
    }
}

impl From<NodeInsertion> for usize {
    #[inline]
    fn from(receipt: NodeInsertion) -> usize {
        receipt.node.0
    }
}

impl NodeInsertion {
    /// The raw id (mirrors [`NodeId::id`]) — AUTO callers read their
    /// assignment here.
    pub fn id(&self) -> usize {
        self.node.0
    }
}

/// A pending layout run — see [`Graph::layout`]. Configure with
/// [`with_config`](Self::with_config), then finish with exactly one
/// terminal.
#[cfg(feature = "alloc")]
#[must_use = "a layout run does nothing until a terminal (.compute / .reported / .quiet) runs it"]
pub struct LayoutRun<'g, 'a> {
    graph: &'g Graph<'a>,
    config: Option<&'g LayoutConfig<'g>>,
}

#[cfg(feature = "alloc")]
impl<'g, 'a> LayoutRun<'g, 'a> {
    /// Use `config` instead of the standard layout configuration.
    pub fn with_config(mut self, config: &'g LayoutConfig<'g>) -> Self {
        self.config = Some(config);
        self
    }

    /// Canonical terminal: replay the graph's mutation diagnostics
    /// into `diagnostics`, then compute the layout.
    pub fn compute(
        self,
        diagnostics: &mut crate::diagnostics::DiagnosticContext<'_>,
    ) -> crate::ir::LayoutIR<'a> {
        // Standing conditions, re-derived from the graph per run —
        // nothing is stored and nothing is consumed (like compiler
        // warnings, a condition reports on every run until it is
        // fixed). Deterministic order: implicit placeholders in node
        // insertion order, then the crossing-passes note. Point
        // events (an AUTO-involved replacement, a placeholder's
        // creation moment) belong to their receipts at the call site.
        if self.graph.missing_node_policy.is_none() {
            for &(id, _) in &self.graph.nodes {
                if self.graph.auto_created.contains(&id) {
                    diagnostics
                        .emit(crate::diagnostics::DiagnosticKind::PlaceholderCreated { node: id });
                }
            }
        }
        // The passes note describes the GRAPH-OWNED configuration; a
        // `.with_config(...)` override replaces that configuration
        // for this run, so the note would describe a config the run
        // never uses — it applies only when the graph's own config
        // is the one selected.
        if self.config.is_none() {
            if let Some(note) = self.graph.passes_note {
                diagnostics.emit(note);
            }
        }
        match self.config {
            Some(config) => self.graph.compute_layout_with_config(config),
            None => self.graph.compute_layout(),
        }
    }

    /// Report terminal: run with an owned collector and package the
    /// (infallible) outcome with everything collected.
    pub fn reported(
        self,
    ) -> crate::diagnostics::OwnedReport<crate::ir::LayoutIR<'a>, core::convert::Infallible> {
        let mut run =
            crate::diagnostics::DiagnosticRun::new(crate::diagnostics::VecDiagnostics::default());
        let ir = {
            let mut cx = run.context();
            self.compute(&mut cx)
        };
        run.finish(Ok(ir))
    }

    /// Quiet terminal: explicitly discard diagnostics
    /// (constructs [`IgnoreDiagnostics`](crate::IgnoreDiagnostics)).
    pub fn quiet(self) -> crate::ir::LayoutIR<'a> {
        let mut run = crate::diagnostics::DiagnosticRun::new(crate::diagnostics::IgnoreDiagnostics);
        let mut cx = run.context();
        self.compute(&mut cx)
    }
}

/// Auto-numbering sentinel for `Graph::add_node`'s id slot:
/// `g.add_node(AUTO, "label")` lets the graph pick the next id above
/// every id it has seen. `Auto` does not convert to [`NodeId`], so the
/// sentinel cannot appear where an *existing* node is referenced
/// (`add_edge(AUTO, …)` is a compile error).
pub const AUTO: Auto = Auto;

/// An explicit node id or the [`AUTO`] sentinel, for
/// `Graph::add_node`'s id slot.
///
/// `From<usize>` keeps every existing call site compiling — including
/// bare integer literals. STANDING GUARD (compile-pinned by test):
/// `usize` must remain the **only** integer `From` impl here and on
/// [`NodeId`]; a second one would re-ambiguate bare literals onto
/// Rust's `i32` fallback and break them.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum IdOrAuto {
    /// A caller-chosen id.
    Id(usize),
    /// Graph-assigned: the next id above every id seen so far.
    Auto,
}

impl From<usize> for IdOrAuto {
    #[inline]
    fn from(id: usize) -> Self {
        IdOrAuto::Id(id)
    }
}

impl From<Auto> for IdOrAuto {
    #[inline]
    fn from(_: Auto) -> Self {
        IdOrAuto::Auto
    }
}

impl From<NodeId> for IdOrAuto {
    #[inline]
    fn from(handle: NodeId) -> Self {
        IdOrAuto::Id(handle.0)
    }
}

/// A directed graph with ASCII rendering capabilities.
///
/// Despite the crate name (`ascii-dag`), `Graph` supports cycles — they are
/// detected and broken automatically during layout.  Use [`Requirements::dag()`]
/// if you need to validate acyclicity before layout.
///
/// [`Requirements::dag()`]: crate::Requirements::dag
///
/// # Examples
///
/// ```
/// use ascii_dag::Graph;
///
/// let mut g = Graph::new();
/// g.add_node(1, "Start");
/// g.add_node(2, "End");
/// g.add_edge(1, 2, None);
///
/// let output = g.render();
/// assert!(output.contains("Start"));
/// assert!(output.contains("End"));
/// ```
#[cfg(feature = "alloc")]
#[allow(deprecated)]
#[derive(Clone, Default)]
pub struct Graph<'a> {
    pub(crate) nodes: Vec<(usize, &'a str)>,
    pub(crate) edges: Vec<(usize, usize, Option<&'a str>)>,
    pub(crate) render_mode: RenderMode,
    pub(crate) direction: Direction,
    pub(crate) auto_created: HashSet<usize>, // Track auto-created nodes for visual distinction (O(1) lookups)
    pub(crate) id_to_index: HashMap<usize, usize>, // Cache id→index mapping (O(1) lookups)
    pub(crate) node_widths: Vec<usize>,      // Cached formatted widths
    pub(crate) node_heights: Vec<usize>,     // Cached node heights (1 = single-line)
    pub(crate) children: Vec<Vec<usize>>,    // Adjacency list: children[idx] = child indices
    pub(crate) parents: Vec<Vec<usize>>,     // Adjacency list: parents[idx] = parent indices
    pub(crate) sugiyama_config: SugiyamaConfig, // Full Sugiyama pipeline configuration
    pub(crate) subgraphs: Vec<Subgraph<'a>>, // Named clusters
    pub(crate) node_subgraph: HashMap<usize, usize>, // node_id → subgraph_id
    pub(crate) next_subgraph_id: usize,      // Monotonic ID counter
    pub(crate) next_auto: usize,             // AUTO id source: 1 above every id seen (saturating)
    pub(crate) auto_numbered: HashSet<usize>, // Ids assigned via AUTO (replace diagnostics)
    // D6 sparse+packed content storage: 1 B/node kind tag; painter +
    // payload only for nodes that have them, keyed by node index
    // (sorted — appends are naturally ordered, replaces upsert).
    pub(crate) node_kind_tag: Vec<u8>,
    pub(crate) node_custom: Vec<(usize, Option<NodePaintFn>, &'a str)>,
    /// Missing-node policy; `None` = never set (the implicit 0.10
    /// default, which makes placeholder creation diagnostic-worthy).
    missing_node_policy: Option<MissingNodePolicy>,
    /// The current crossing-passes condition (clamped or excessive),
    /// if any — a CONDITION SLOT reflecting the live configuration,
    /// never an event log: each setter call overwrites it, a sane
    /// value or a direct pipeline clears it, and every
    /// diagnostic-aware layout run emits it (without consuming — the
    /// condition holds until fixed). Plain `Copy` state: no interior
    /// mutability, so `Graph` stays `Send + Sync` and mutation never
    /// allocates for diagnostics.
    passes_note: Option<crate::diagnostics::DiagnosticKind>,
}

#[cfg(feature = "alloc")]
#[allow(deprecated)]
impl<'a> Graph<'a> {
    /// Create a new empty DAG.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::Graph;
    /// let dag = Graph::new();
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a DAG from pre-defined nodes and edges (batch construction).
    ///
    /// This is more efficient than using the builder API for static graphs.
    /// For edges with labels, use [`from_edges_labeled`](Self::from_edges_labeled).
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::Graph;
    ///
    /// let dag = Graph::from_edges(
    ///     &[(1, "A"), (2, "B"), (3, "C")],
    ///     &[(1, 2), (2, 3)]
    /// );
    /// ```
    pub fn from_edges(nodes: &[(usize, &'a str)], edges: &[(usize, usize)]) -> Self {
        let mut dag = Self {
            nodes: nodes.to_vec(),
            edges: Vec::new(),
            render_mode: RenderMode::default(),
            direction: Direction::default(),
            auto_created: HashSet::new(),
            id_to_index: HashMap::new(),
            node_widths: Vec::new(),
            node_heights: Vec::new(),
            children: Vec::new(),
            parents: Vec::new(),
            sugiyama_config: SugiyamaConfig::default(),
            subgraphs: Vec::new(),
            node_subgraph: HashMap::new(),
            next_subgraph_id: 0,
            next_auto: 0,
            auto_numbered: HashSet::new(),
            node_kind_tag: Vec::new(),
            node_custom: Vec::new(),
            missing_node_policy: None,
            passes_note: None,
        };

        // Build id_to_index map, widths cache, and heights cache; seed
        // the AUTO counter above every batch-supplied id in the same
        // pass.
        let mut next_auto = 0usize;
        for (idx, &(id, label)) in dag.nodes.iter().enumerate() {
            dag.id_to_index.insert(id, idx);
            let width = dag.compute_node_width(id, label);
            dag.node_widths.push(width);
            dag.node_heights.push(1);
            dag.node_kind_tag.push(NodeKindTag::Simple.to_u8());
            next_auto = next_auto.max(id.saturating_add(1));
        }
        dag.next_auto = next_auto;

        // Initialize adjacency lists
        dag.children.resize(dag.nodes.len(), Vec::new());
        dag.parents.resize(dag.nodes.len(), Vec::new());

        // Add edges (may auto-create missing nodes)
        for &(from, to) in edges {
            dag.add_edge(from, to, None);
        }

        dag
    }

    /// Create a DAG from pre-defined nodes and labeled edges (batch construction).
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::Graph;
    ///
    /// let dag = Graph::from_edges_labeled(
    ///     &[(1, "A"), (2, "B"), (3, "C")],
    ///     &[(1, 2, Some("uses")), (2, 3, None)]
    /// );
    /// ```
    pub fn from_edges_labeled(
        nodes: &[(usize, &'a str)],
        edges: &[(usize, usize, Option<&'a str>)],
    ) -> Self {
        let mut dag = Self {
            nodes: nodes.to_vec(),
            edges: Vec::new(),
            render_mode: RenderMode::default(),
            direction: Direction::default(),
            auto_created: HashSet::new(),
            id_to_index: HashMap::new(),
            node_widths: Vec::new(),
            node_heights: Vec::new(),
            children: Vec::new(),
            parents: Vec::new(),
            sugiyama_config: SugiyamaConfig::default(),
            subgraphs: Vec::new(),
            node_subgraph: HashMap::new(),
            next_subgraph_id: 0,
            next_auto: 0,
            auto_numbered: HashSet::new(),
            node_kind_tag: Vec::new(),
            node_custom: Vec::new(),
            missing_node_policy: None,
            passes_note: None,
        };

        // Build id_to_index map, widths cache, and heights cache; seed
        // the AUTO counter above every batch-supplied id in the same
        // pass.
        let mut next_auto = 0usize;
        for (idx, &(id, label)) in dag.nodes.iter().enumerate() {
            dag.id_to_index.insert(id, idx);
            let width = dag.compute_node_width(id, label);
            dag.node_widths.push(width);
            dag.node_heights.push(1);
            dag.node_kind_tag.push(NodeKindTag::Simple.to_u8());
            next_auto = next_auto.max(id.saturating_add(1));
        }
        dag.next_auto = next_auto;

        // Initialize adjacency lists
        dag.children.resize(dag.nodes.len(), Vec::new());
        dag.parents.resize(dag.nodes.len(), Vec::new());

        // Add edges (may auto-create missing nodes)
        for &(from, to, label) in edges {
            dag.add_edge(from, to, label);
        }

        dag
    }

    /// Set the rendering mode.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::{Graph, RenderMode};
    ///
    /// let mut dag = Graph::new();
    /// dag.set_render_mode(RenderMode::Horizontal);
    /// ```
    pub fn set_render_mode(&mut self, mode: RenderMode) {
        self.render_mode = mode;
    }

    /// Set the rank direction (see [`Direction`]).
    ///
    /// Applies when computing the layout via [`render`](Self::render) or
    /// [`compute_layout`](Self::compute_layout). When you build a
    /// [`LayoutConfig`] yourself and call
    /// [`compute_layout_with_config`](Self::compute_layout_with_config),
    /// the config's `direction` wins.
    ///
    /// All four directions render. One caveat on the
    /// [`render`](Self::render) convenience: under
    /// [`RenderMode::Auto`] a simple chain uses the compact
    /// left-to-right form only for `TopDown`; any other direction is
    /// laid out and painted normally. Asking for
    /// [`RenderMode::Horizontal`] explicitly always gives the chain
    /// form, whatever the direction.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::{Direction, Graph};
    ///
    /// let mut dag = Graph::new();
    /// dag.set_direction(Direction::DEFAULT);
    /// # #[cfg(feature = "layout-vertical")]
    /// dag.set_direction(Direction::BottomUp);
    /// ```
    pub fn set_direction(&mut self, direction: Direction) {
        self.direction = direction;
    }

    /// The rank direction set via [`set_direction`](Self::set_direction)
    /// (`TopDown` by default) — e.g. to carry it into a
    /// [`LayoutConfig`] for the CSR pipeline.
    pub fn direction(&self) -> Direction {
        self.direction
    }

    /// Set the number of passes for the crossing reduction algorithm.
    ///
    /// This is a **compatibility shim** — it replaces the entire pipeline
    /// with `[Median(passes)]`.  Prefer [`set_crossing_pipeline`](Self::set_crossing_pipeline)
    /// for full control.
    ///
    /// - `0`: Skip crossing reduction entirely
    /// - `1-4`: Good for most graphs
    /// - `8-10`: Better layouts for complex tangled graphs, but slower
    ///
    /// Values > 20 trigger a warning.  Values > 1000 are clamped to 0.
    pub fn set_crossing_reduction_passes(&mut self, passes: usize) {
        let (p, note) = Self::validate_passes(passes);
        // A condition slot, not a log: the note describes the CURRENT
        // value, so each call overwrites it (a sane value clears it),
        // and every diagnostic-aware layout run reports it until the
        // configuration is fixed.
        self.passes_note = note;
        self.sugiyama_config.crossing_pipeline = if p == 0 {
            Vec::new()
        } else {
            vec![CrossingReducer::Median(p)]
        };
    }

    /// Set the crossing reduction pipeline.
    ///
    /// The pipeline is a sequence of [`CrossingReducer`] strategies applied
    /// in order.  Use the presets [`FAST`](crate::algorithms::sugiyama::crossing::FAST),
    /// [`STANDARD`](crate::algorithms::sugiyama::crossing::STANDARD), or
    /// [`QUALITY`](crate::algorithms::sugiyama::crossing::QUALITY), or build
    /// your own.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::Graph;
    /// use ascii_dag::algorithms::sugiyama::crossing::{CrossingReducer, QUALITY};
    ///
    /// let mut dag = Graph::new();
    /// dag.set_crossing_pipeline(QUALITY);
    /// ```
    pub fn set_crossing_pipeline(&mut self, pipeline: &[CrossingReducer]) {
        // Replaces the compatibility shim's value wholesale — any
        // standing passes note describes a configuration that no
        // longer exists.
        self.passes_note = None;
        self.sugiyama_config.crossing_pipeline = pipeline.to_vec();
    }

    /// Validate crossing reduction passes, returning a safe value.
    /// - Values > 1000 are treated as accidental (e.g., -1i32 as usize) and clamped to 0
    /// - Values > 20 trigger a warning about diminishing returns
    #[inline]
    fn validate_passes(passes: usize) -> (usize, Option<crate::diagnostics::DiagnosticKind>) {
        if passes > 1000 {
            (
                0,
                Some(crate::diagnostics::DiagnosticKind::CrossingPassesClamped {
                    requested: passes,
                    clamped_to: 0,
                }),
            )
        } else if passes > 20 {
            (
                passes,
                Some(
                    crate::diagnostics::DiagnosticKind::CrossingPassesExcessive {
                        requested: passes,
                    },
                ),
            )
        } else {
            (passes, None)
        }
    }

    /// Builder method: set render mode (chainable).
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::{Graph, RenderMode};
    ///
    /// let dag = Graph::new()
    ///     .with_render_mode(RenderMode::Horizontal)
    ///     .with_crossing_reduction_passes(6);
    /// ```
    pub fn with_render_mode(mut self, mode: RenderMode) -> Self {
        self.render_mode = mode;
        self
    }

    /// Builder method: set the rank direction (chainable).
    pub fn with_direction(mut self, direction: Direction) -> Self {
        self.direction = direction;
        self
    }

    /// Builder method: set crossing reduction passes (chainable).
    ///
    /// **Compatibility shim** — see [`set_crossing_reduction_passes`](Self::set_crossing_reduction_passes).
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::Graph;
    ///
    /// let dag = Graph::new()
    ///     .with_crossing_reduction_passes(8);  // More passes for complex graphs
    /// ```
    pub fn with_crossing_reduction_passes(mut self, passes: usize) -> Self {
        self.set_crossing_reduction_passes(passes);
        self
    }

    /// Builder method: set crossing reduction pipeline (chainable).
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::Graph;
    /// use ascii_dag::algorithms::sugiyama::crossing::QUALITY;
    ///
    /// let dag = Graph::new()
    ///     .with_crossing_pipeline(QUALITY);
    /// ```
    pub fn with_crossing_pipeline(mut self, pipeline: &[CrossingReducer]) -> Self {
        // Delegate so the condition slot stays consistent: replacing
        // the pipeline clears any standing passes note, in the
        // builder chain exactly as in the setter.
        self.set_crossing_pipeline(pipeline);
        self
    }

    /// Create a DAG with a specific render mode.
    ///
    /// **Deprecated**: Prefer `Graph::new().with_render_mode(mode)` for consistency.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::{Graph, RenderMode};
    ///
    /// let dag = Graph::with_mode(RenderMode::Horizontal);
    /// ```
    pub fn with_mode(mode: RenderMode) -> Self {
        Self::new().with_render_mode(mode)
    }

    /// Record `id` as seen so a later [`AUTO`] pick lands above it.
    /// Saturating: near `usize::MAX` the counter pins to the top and a
    /// subsequent `AUTO` falls through to the documented
    /// replace-on-duplicate semantics (don't park explicit ids there —
    /// that band already collides with synthetic dummy ids).
    #[inline]
    fn bump_next_auto(&mut self, id: usize) {
        let next = id.saturating_add(1);
        if next > self.next_auto {
            self.next_auto = next;
        }
    }

    /// Upsert the sparse painter/payload entry for a node index —
    /// entries exist only for nodes that have a painter or a non-empty
    /// payload (D6). The list is sorted by node index; appends during
    /// construction hit the `Err(end)` arm, replaces upsert in place.
    fn set_custom_entry(&mut self, idx: usize, painter: Option<NodePaintFn>, payload: &'a str) {
        let pos = self.node_custom.binary_search_by_key(&idx, |entry| entry.0);
        let keep = painter.is_some() || !payload.is_empty();
        match (pos, keep) {
            (Ok(at), true) => self.node_custom[at] = (idx, painter, payload),
            (Ok(at), false) => {
                self.node_custom.remove(at);
            }
            (Err(at), true) => self.node_custom.insert(at, (idx, painter, payload)),
            (Err(_), false) => {}
        }
    }

    /// Add a node to the DAG. Returns a typed [`NodeId`] handle usable
    /// wherever a node id is accepted (edges, subgraph placement).
    ///
    /// The content slot accepts anything implementing
    /// [`NodeContent`]: a bare `&str` (the classic `[label]` node,
    /// byte-identical to previous releases), a built-in object
    /// ([`SimpleNode`](crate::SimpleNode) /
    /// [`BoxedNode`](crate::BoxedNode)), or a user type /
    /// [`CustomNode`](crate::CustomNode) carrying its own size,
    /// painter, and payload. The declaration is the *only* source of
    /// what the node is — there is no style-side override. The object
    /// is resolved once, here — it may be a temporary.
    ///
    /// The id slot takes an explicit `usize` **or** the [`AUTO`]
    /// sentinel, which assigns the next id above every id this graph
    /// has seen — explicit ids first (e.g. [`Graph::from_edges`]) then
    /// `AUTO` extras stay collision-free until the counter saturates
    /// at `usize::MAX` (only reachable by explicitly parking ids
    /// there); a saturated `AUTO`, like any duplicate id, falls
    /// through to the replace semantics below.
    ///
    /// If the node already exists (auto-created by `add_edge`, or added
    /// earlier), this replaces its label — promoting auto-created
    /// placeholders to explicit nodes. The returned [`NodeInsertion`]
    /// receipt records the replacement, and flags the variant worth
    /// attention: `AUTO` numbering involved on either side (an
    /// explicit id overwriting an auto-numbered node, or a saturated
    /// `AUTO` overwriting anything).
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::{Graph, AUTO};
    ///
    /// let mut dag = Graph::new();
    /// dag.add_node(1, "MyNode");            // explicit id
    /// let n = dag.add_node(AUTO, "Auto");   // graph-assigned id
    /// dag.add_edge(1, n, None);             // handles flow into edges
    /// ```
    pub fn add_node(
        &mut self,
        id: impl Into<IdOrAuto>,
        node: impl NodeContent<'a>,
    ) -> NodeInsertion {
        let (id, incoming_auto) = match id.into() {
            IdOrAuto::Id(id) => (id, false),
            IdOrAuto::Auto => (self.next_auto, true),
        };
        // Resolve the content BEFORE any graph state mutates: the
        // accessors are user code and may panic — a caught panic must
        // not leave the AUTO counter or diagnostics state advanced for
        // an insertion that never happened ("successful creation
        // only"). Accessors should be cheap and pure; resolution may
        // call them more than once (the default `size()` reads
        // `label()`).
        let label = node.label();
        let (width, height) = node.size();
        let kind = node.kind();
        let painter = node.painter();
        let payload = node.payload();
        let size_is_implicit = node.size_is_implicit();
        self.bump_next_auto(id);
        // Replacement is a point event, so the receipt is its record:
        // a duplicate involving AUTO on either side (an explicit id
        // silently overwriting an auto-numbered node, or a saturated
        // AUTO overwriting an existing node) is the variant worth the
        // caller's attention.
        let replaced = self.id_to_index.contains_key(&id);
        let replaced_involving_auto =
            replaced && (incoming_auto || self.auto_numbered.contains(&id));
        {
            // Track how this id was assigned (auto or explicit) so the
            // check above can see the existing side next time.
            if incoming_auto {
                self.auto_numbered.insert(id);
            } else {
                self.auto_numbered.remove(&id);
            }
        }
        // Empty-label placeholders render as ⟨id⟩ — an id-dependent
        // width the content object cannot know. Only content whose
        // `size()` is the provided default (`&str`, `&String`,
        // `SimpleNode` — size provenance, not a value heuristic) gets
        // today's formula; every overridden `size()` is authoritative.
        let width = if label.is_empty() && size_is_implicit {
            self.compute_node_width(id, label)
        } else {
            width
        };
        let height = height.max(1);
        // Check if node already exists (could be auto-created) - O(1) with HashMap
        if let Some(&idx) = self.id_to_index.get(&id) {
            // Promote auto-created node to explicit node
            self.nodes[idx] = (id, label);
            // Remove from auto_created set - O(1)
            self.auto_created.remove(&id);
            self.node_widths[idx] = width;
            self.node_heights[idx] = height;
            self.node_kind_tag[idx] = kind.to_u8();
            self.set_custom_entry(idx, painter, payload);
        } else {
            // Brand new node
            let idx = self.nodes.len();
            self.nodes.push((id, label));
            self.id_to_index.insert(id, idx);
            self.node_widths.push(width);
            self.node_heights.push(height);
            self.node_kind_tag.push(kind.to_u8());
            // Extend adjacency lists
            self.children.push(Vec::new());
            self.parents.push(Vec::new());
            self.set_custom_entry(idx, painter, payload);
        }
        NodeInsertion {
            node: NodeId(id),
            replaced,
            replaced_involving_auto,
        }
    }

    /// Add an edge from one node to another with an optional label.
    ///
    /// If either node doesn't exist, it will be auto-created as a placeholder.
    /// You can later call `add_node` to provide a label for auto-created nodes.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::Graph;
    ///
    /// let mut dag = Graph::new();
    /// dag.add_node(1, "A");
    /// dag.add_node(2, "B");
    /// dag.add_node(3, "C");
    /// dag.add_edge(1, 2, None);  // A -> B (no label)
    /// dag.add_edge(2, 3, Some("depends on"));  // B -> C with label
    /// ```
    ///
    /// Accepts raw ids and [`NodeId`] handles alike. The [`AUTO`]
    /// sentinel is *not* accepted here — an edge references nodes, and
    /// "auto" is meaningless as a reference.
    pub fn add_edge(
        &mut self,
        from: impl Into<NodeId>,
        to: impl Into<NodeId>,
        label: Option<&'a str>,
    ) -> EdgeInsertion {
        let (from, to) = (from.into().id(), to.into().id());
        let created_source = self.ensure_node_exists(from);
        let created_target = self.ensure_node_exists(to);
        let edge = self.edges.len();
        self.edges.push((from, to, label));

        // Update adjacency lists (O(1) lookups)
        if let (Some(&from_idx), Some(&to_idx)) =
            (self.id_to_index.get(&from), self.id_to_index.get(&to))
        {
            self.children[from_idx].push(to_idx);
            self.parents[to_idx].push(from_idx);
        }
        EdgeInsertion {
            edge,
            created_source,
            created_target,
        }
    }

    /// Declare how [`add_edge`](Self::add_edge) treats an undeclared
    /// endpoint. Setting the policy — even to the default — is
    /// declared intent: placeholder creation then stops emitting the
    /// placeholder diagnostic (the [`EdgeInsertion`] receipt remains
    /// the record either way).
    pub fn set_missing_node_policy(&mut self, policy: MissingNodePolicy) {
        self.missing_node_policy = Some(policy);
    }

    /// Get the label for an edge, if any.
    #[inline]
    pub fn edge_label(&self, edge_idx: usize) -> Option<&'a str> {
        self.edges.get(edge_idx).and_then(|e| e.2)
    }

    /// Ensure a node exists, auto-creating if missing.
    /// Auto-created nodes will be visually distinct (rendered with ⟨⟩ instead of [])
    /// until explicitly defined with add_node.
    /// Returns whether a placeholder was created.
    fn ensure_node_exists(&mut self, id: usize) -> bool {
        // O(1) lookup with HashMap
        if !self.id_to_index.contains_key(&id) {
            // No diagnostic is recorded here: an implicit placeholder
            // is a standing CONDITION (`auto_created` + undeclared
            // policy), re-derived by each diagnostic-aware layout run
            // — nothing is stored, and the receipt serves the call
            // site. An explicitly declared policy silences the
            // condition, whenever it is declared.

            // Create node with empty label. An id-creating site: the
            // AUTO counter must clear implicitly created ids too.
            self.bump_next_auto(id);
            let idx = self.nodes.len();
            self.nodes.push((id, ""));
            self.auto_created.insert(id); // O(1) insert
            self.id_to_index.insert(id, idx); // O(1) insert
            let width = self.compute_node_width(id, "");
            self.node_widths.push(width);
            self.node_heights.push(1);
            self.node_kind_tag.push(NodeKindTag::Simple.to_u8());
            // Extend adjacency lists
            self.children.push(Vec::new());
            self.parents.push(Vec::new());
            return true;
        }
        false
    }

    /// Check if a node was auto-created (for visual distinction)
    pub(crate) fn is_auto_created(&self, id: usize) -> bool {
        self.auto_created.contains(&id) // O(1) with HashSet
    }

    /// Write an unsigned integer to a string buffer without allocation.
    /// This avoids format! bloat in no_std builds.
    #[inline]
    pub(crate) fn write_usize(buf: &mut String, mut n: usize) {
        if n == 0 {
            buf.push('0');
            return;
        }
        let mut digits = [0u8; 20]; // Max digits for u64
        let mut i = 0;
        while n > 0 {
            digits[i] = (n % 10) as u8 + b'0';
            n /= 10;
            i += 1;
        }
        // Write in reverse order
        while i > 0 {
            i -= 1;
            buf.push(digits[i] as char);
        }
    }

    /// Count digits in a number (for width calculation)
    #[inline]
    fn count_digits(mut n: usize) -> usize {
        if n == 0 {
            return 1;
        }
        let mut count = 0;
        while n > 0 {
            count += 1;
            n /= 10;
        }
        count
    }

    /// Compute the formatted width of a node
    pub(crate) fn compute_node_width(&self, id: usize, label: &str) -> usize {
        if label.is_empty() || self.is_auto_created(id) {
            // ⟨ID⟩ format
            2 + Self::count_digits(id) // ⟨ + digits + ⟩
        } else {
            // [Label] format
            2 + label.chars().count() // [ + label + ]
        }
    }

    /// Write a formatted node directly to output buffer (avoids intermediate String allocation)
    #[inline]
    pub(crate) fn write_node(&self, output: &mut String, id: usize, label: &str) {
        if label.is_empty() || self.is_auto_created(id) {
            output.push('⟨');
            Self::write_usize(output, id);
            output.push('⟩');
        } else {
            output.push('[');
            output.push_str(label);
            output.push(']');
        }
    }

    /// Get children of a node (returns IDs, not indices).
    /// Uses cached adjacency lists for O(1) lookup instead of O(E) iteration.
    /// NOTE: This allocates a new Vec. For hot paths, use `children_count` + `get_children_indices`.
    #[inline]
    pub(crate) fn get_children(&self, node_id: usize) -> Vec<usize> {
        if let Some(&idx) = self.id_to_index.get(&node_id) {
            // Convert child indices back to IDs
            self.children[idx]
                .iter()
                .map(|&child_idx| self.nodes[child_idx].0)
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Get parents of a node (returns IDs, not indices).
    /// Uses cached adjacency lists for O(1) lookup instead of O(E) iteration.
    /// NOTE: This allocates a new Vec. For hot paths, use `parents_count` + `get_parents_indices`.
    #[inline]
    pub(crate) fn get_parents(&self, node_id: usize) -> Vec<usize> {
        if let Some(&idx) = self.id_to_index.get(&node_id) {
            // Convert parent indices back to IDs
            self.parents[idx]
                .iter()
                .map(|&parent_idx| self.nodes[parent_idx].0)
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Count children of a node by index (zero-allocation).
    #[inline]
    pub(crate) fn children_count(&self, node_idx: usize) -> usize {
        self.children.get(node_idx).map_or(0, |c| c.len())
    }

    /// Count parents of a node by index (zero-allocation).
    #[inline]
    pub(crate) fn parents_count(&self, node_idx: usize) -> usize {
        self.parents.get(node_idx).map_or(0, |p| p.len())
    }

    /// Get node index from ID using O(1) HashMap lookup
    #[inline]
    pub(crate) fn node_index(&self, id: usize) -> Option<usize> {
        self.id_to_index.get(&id).copied()
    }

    /// Get cached width for a node index
    #[inline]
    pub(crate) fn get_node_width(&self, idx: usize) -> usize {
        self.node_widths.get(idx).copied().unwrap_or(0)
    }

    /// Get cached height for a node index
    #[inline]
    pub(crate) fn get_node_height(&self, idx: usize) -> usize {
        self.node_heights.get(idx).copied().unwrap_or(1)
    }

    /// Estimate the buffer size needed for rendering.
    ///
    /// Use this to pre-allocate a buffer for [`render_to`](Self::render_to).
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::Graph;
    ///
    /// let dag = Graph::from_edges(
    ///     &[(1, "A"), (2, "B")],
    ///     &[(1, 2)]
    /// );
    ///
    /// let size = dag.estimate_size();
    /// let mut buffer = String::with_capacity(size);
    /// dag.render_to(&mut buffer);
    /// ```
    pub fn estimate_size(&self) -> usize {
        // Estimate based on empirical measurements:
        // - Each level takes ~width characters (canvas can be very wide)
        // - Vertical layouts have many levels with connection lines
        // - For layered graphs with skip-edges, canvas can be quite wide
        //
        // Vertical layout: nodes spread across canvas width + connection lines
        // Each node level line + ~5 connection lines per level
        // Canvas width roughly: nodes_per_level * 15 chars
        // Height roughly: levels * 6 lines
        let n = self.nodes.len();
        let est_levels = n.isqrt().max(1);
        let est_width = (n / est_levels).max(1) * 15;
        let est_height = est_levels * 6;
        let base = est_width * est_height * 3; // UTF-8 chars average ~3 bytes
        base.max(n * 100) // Ensure minimum sensible estimate
    }

    /// Compute the layout intermediate representation for this DAG.
    ///
    /// This returns a renderer-agnostic representation of the laid-out graph
    /// that can be consumed by various renderers (ASCII, ANSI colors, SVG, etc.).
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::Graph;
    ///
    /// let dag = Graph::from_edges(
    ///     &[(1, "A"), (2, "B"), (3, "C")],
    ///     &[(1, 2), (1, 3), (2, 3)]
    /// );
    ///
    /// let ir = dag.compute_layout();
    ///
    /// // Inspect layout
    /// println!("Width: {}, Height: {}", ir.width(), ir.height());
    /// for node in ir.nodes() {
    ///     println!("{} at ({}, {})", node.label, node.x, node.y);
    /// }
    /// ```
    pub fn compute_layout(&self) -> crate::ir::LayoutIR<'a> {
        let mut config: LayoutConfig<'_> = LayoutConfig::from(&self.sugiyama_config);
        config.direction = self.direction;
        self.layout_with(&config)
    }

    /// Dispatch to the axis profile the direction needs (temp/08 D1):
    /// `LeftRight`/`RightLeft` lay out natively through `Horizontal`
    /// — levels become columns, edges run in horizontal trunks —
    /// everything else through `Vertical`. `RightLeft` is `LeftRight`
    /// mirrored on x, applied inside the pipeline.
    fn layout_with(&self, config: &LayoutConfig<'_>) -> crate::ir::LayoutIR<'a> {
        #[cfg(feature = "layout-horizontal")]
        use crate::algorithms::sugiyama::geometry::Horizontal;
        #[cfg(feature = "layout-vertical")]
        use crate::algorithms::sugiyama::geometry::Vertical;
        match config.direction {
            #[cfg(feature = "layout-horizontal")]
            Direction::LeftRight | Direction::RightLeft => {
                crate::algorithms::sugiyama::heap::compute_layout_cfg::<Horizontal>(self, config)
            }
            #[cfg(feature = "layout-vertical")]
            _ => crate::algorithms::sugiyama::heap::compute_layout_cfg::<Vertical>(self, config),
        }
    }

    /// Begin a layout run — the diagnostic-aware entry point, as a
    /// builder because its inputs are optional. Finish with one of
    /// three terminals:
    ///
    /// - [`compute(&mut cx)`](LayoutRun::compute) — canonical: emits
    ///   into your run's context and composes across phases;
    /// - [`reported()`](LayoutRun::reported) — owns a complete
    ///   [`OwnedReport`](crate::OwnedReport) for this operation;
    /// - [`quiet()`](LayoutRun::quiet) — explicitly discards
    ///   diagnostics.
    ///
    /// Mutation-context diagnostics are standing CONDITIONS,
    /// re-derived from the graph at each diagnostic-aware run and
    /// emitted before the layout computes — the graph stores no
    /// diagnostic state: implicit auto-created placeholders (in node
    /// insertion order, while the missing-node policy stays
    /// undeclared) and the current crossing-passes note. Like a
    /// compiler warning, a condition reports on every run until it is
    /// fixed — declare the policy, promote the placeholder, set a
    /// sane pass count. Point events belong to receipts at the call
    /// site ([`EdgeInsertion`], [`NodeInsertion`]). Quiet paths
    /// ([`quiet()`](LayoutRun::quiet), `compute_layout`,
    /// `compute_layout_with_config`) are exactly equivalent: with no
    /// stored diagnostics there is nothing to consume, leak, or
    /// deliver late.
    pub fn layout<'g>(&'g self) -> LayoutRun<'g, 'a> {
        LayoutRun {
            graph: self,
            config: None,
        }
    }

    /// Compute the layout using a custom [`LayoutConfig`].
    ///
    /// This is the preferred API for controlling layout behaviour.
    /// The config borrows its crossing pipeline, so it can be constructed
    /// from static presets or from a `SugiyamaConfig`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::{Graph, LayoutConfig};
    ///
    /// let dag = Graph::from_edges(
    ///     &[(1, "A"), (2, "B"), (3, "C")],
    ///     &[(1, 2), (2, 3)]
    /// );
    ///
    /// let ir = dag.compute_layout_with_config(&LayoutConfig::quality());
    /// ```
    pub fn compute_layout_with_config(&self, config: &LayoutConfig<'_>) -> crate::ir::LayoutIR<'a> {
        let mut dag = self.clone();
        dag.render_mode = config.render_mode;
        dag.layout_with(config)
    }

    // ── Subgraph / Cluster API ───────────────────────────────────────────

    /// Create a new named subgraph (cluster) and return its ID.
    ///
    /// The subgraph has no members and no parent until you call
    /// [`put_nodes`](Self::put_nodes) or [`put_subgraphs`](Self::put_subgraphs).
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::Graph;
    ///
    /// let mut g = Graph::new();
    /// let backend = g.add_subgraph("Backend");
    /// let frontend = g.add_subgraph("Frontend");
    /// assert_ne!(backend, frontend);
    /// ```
    pub fn add_subgraph(&mut self, label: &'a str) -> usize {
        let id = self.next_subgraph_id;
        self.next_subgraph_id += 1;
        self.subgraphs.push(Subgraph {
            id,
            label,
            parent_id: None,
        });
        id
    }

    /// Start a fluent builder to place nodes inside a subgraph.
    ///
    /// Returns a [`NodePlacer`] whose [`.inside(sg)`](NodePlacer::inside)
    /// method assigns all listed nodes to the given subgraph.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::Graph;
    ///
    /// let mut g = Graph::new();
    /// g.add_node(1, "A");
    /// g.add_node(2, "B");
    /// let sg = g.add_subgraph("Cluster");
    /// g.put_nodes(&[1, 2]).inside(sg).unwrap();
    /// assert_eq!(g.node_subgraph(1), Some(sg));
    /// ```
    ///
    /// The slice may hold raw `usize` ids or [`NodeId`] handles.
    pub fn put_nodes<'g, N: Into<NodeId> + Copy>(
        &'g mut self,
        node_ids: &'g [N],
    ) -> NodePlacer<'g, 'a, N> {
        NodePlacer {
            graph: self,
            node_ids,
        }
    }

    /// Start a fluent builder to nest subgraphs inside a parent.
    ///
    /// Returns a [`SubgraphPlacer`] whose [`.inside(parent)`](SubgraphPlacer::inside)
    /// method sets the parent for all listed subgraphs.  Cycle detection
    /// prevents a subgraph from being nested inside itself or a descendant.
    ///
    /// # Examples
    ///
    /// ```
    /// use ascii_dag::graph::Graph;
    ///
    /// let mut g = Graph::new();
    /// let outer = g.add_subgraph("Outer");
    /// let inner = g.add_subgraph("Inner");
    /// g.put_subgraphs(&[inner]).inside(outer).unwrap();
    /// ```
    pub fn put_subgraphs<'g>(&'g mut self, sg_ids: &'g [usize]) -> SubgraphPlacer<'g, 'a> {
        SubgraphPlacer {
            graph: self,
            sg_ids,
        }
    }

    /// Number of subgraphs defined on this graph.
    #[inline]
    pub fn subgraph_count(&self) -> usize {
        self.subgraphs.len()
    }

    /// Which subgraph a node belongs to, if any.
    #[inline]
    pub fn node_subgraph(&self, node_id: usize) -> Option<usize> {
        self.node_subgraph.get(&node_id).copied()
    }

    /// Whether any subgraphs have been defined.
    ///
    /// The layout engine uses this to short-circuit all subgraph-related
    /// processing when there are none.
    #[inline]
    pub fn has_subgraphs(&self) -> bool {
        !self.subgraphs.is_empty()
    }

    /// Get a subgraph by ID.
    #[inline]
    pub fn subgraph(&self, id: usize) -> Option<&Subgraph<'a>> {
        self.subgraphs.iter().find(|sg| sg.id == id)
    }

    /// Get all subgraphs.
    #[inline]
    pub fn subgraphs(&self) -> &[Subgraph<'a>] {
        &self.subgraphs
    }

    /// Check whether `ancestor` is an ancestor of `sg_id` in the nesting
    /// hierarchy (or is `sg_id` itself).  Used for cycle detection.
    fn is_ancestor(&self, sg_id: usize, ancestor: usize) -> bool {
        let mut current = Some(sg_id);
        while let Some(id) = current {
            if id == ancestor {
                return true;
            }
            current = self
                .subgraphs
                .iter()
                .find(|s| s.id == id)
                .and_then(|s| s.parent_id);
        }
        false
    }
}

// ── Fluent placement builders ────────────────────────────────────────────

/// Fluent builder returned by [`Graph::put_nodes`].
///
/// Call [`.inside(subgraph_id)`](NodePlacer::inside) to assign nodes to a cluster.
#[cfg(feature = "alloc")]
pub struct NodePlacer<'g, 'a, N = usize> {
    graph: &'g mut Graph<'a>,
    node_ids: &'g [N],
}

#[cfg(feature = "alloc")]
impl<'g, 'a, N: Into<NodeId> + Copy> NodePlacer<'g, 'a, N> {
    /// Assign every node in the list to the given subgraph.
    ///
    /// # Errors
    ///
    /// - [`GraphError::SubgraphNotFound`] if `sg_id` does not exist.
    /// - [`GraphError::NodeNotFound`] if any node ID is not in the graph.
    pub fn inside(self, sg_id: usize) -> Result<(), GraphError> {
        // Validate subgraph exists
        if !self.graph.subgraphs.iter().any(|s| s.id == sg_id) {
            return Err(GraphError::SubgraphNotFound(sg_id));
        }
        // Validate & assign each node. Placement only references
        // existing nodes — it never creates, so it is not an
        // id-creating site for the AUTO counter.
        for &nid in self.node_ids {
            let nid: usize = nid.into().id();
            if !self.graph.id_to_index.contains_key(&nid) {
                return Err(GraphError::NodeNotFound(nid));
            }
            self.graph.node_subgraph.insert(nid, sg_id);
        }
        Ok(())
    }
}

#[cfg(feature = "alloc")]
/// Fluent builder returned by [`Graph::put_subgraphs`].
///
/// Call [`.inside(parent_id)`](SubgraphPlacer::inside) to nest subgraphs.
pub struct SubgraphPlacer<'g, 'a> {
    graph: &'g mut Graph<'a>,
    sg_ids: &'g [usize],
}

#[cfg(feature = "alloc")]
impl<'g, 'a> SubgraphPlacer<'g, 'a> {
    /// Nest every subgraph in the list inside the given parent.
    ///
    /// # Errors
    ///
    /// - [`GraphError::SubgraphNotFound`] if `parent_id` or any child ID
    ///   does not exist.
    /// - [`GraphError::SubgraphCycle`] if nesting would create a cycle
    ///   (e.g., A inside B inside A).
    pub fn inside(self, parent_id: usize) -> Result<(), GraphError> {
        // Validate parent exists
        if !self.graph.subgraphs.iter().any(|s| s.id == parent_id) {
            return Err(GraphError::SubgraphNotFound(parent_id));
        }
        for &child_id in self.sg_ids {
            // Validate child exists
            if !self.graph.subgraphs.iter().any(|s| s.id == child_id) {
                return Err(GraphError::SubgraphNotFound(child_id));
            }
            // Cycle check: parent must not be a descendant of child
            if self.graph.is_ancestor(parent_id, child_id) {
                return Err(GraphError::SubgraphCycle);
            }
            // Set parent
            if let Some(sg) = self.graph.subgraphs.iter_mut().find(|s| s.id == child_id) {
                sg.parent_id = Some(parent_id);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[cfg(feature = "alloc")]
mod tests {
    use super::*;

    /// `LeftRight`/`RightLeft` lay out natively through the public
    /// entry point: levels become columns, trunks run horizontally,
    /// and RL is the exact x-mirror of LR.
    #[cfg(all(feature = "layout-vertical", feature = "layout-horizontal"))]
    #[test]
    fn lr_and_rl_lay_out_horizontally() {
        let build = || {
            let mut g = Graph::new();
            g.add_node(1, "a");
            g.add_node(2, "bb");
            g.add_node(3, "c");
            g.add_edge(1, 2, Some("go"));
            g.add_edge(1, 3, None);
            g.add_edge(2, 2, None);
            g
        };
        let td = build().compute_layout();
        let node = |ir: &crate::ir::LayoutIR<'_>, id: usize| {
            let n = ir.nodes().iter().find(|n| n.id == id).expect("node");
            (n.x, n.y, n.width, n.height)
        };
        for dir in [Direction::LeftRight, Direction::RightLeft] {
            let mut g = build();
            g.set_direction(dir);
            let ir = g.compute_layout();
            assert_eq!(ir.direction(), dir, "{dir:?} recorded on the IR");
            // Levels are COLUMNS: every trunk runs horizontally.
            for e in ir.edges() {
                assert_eq!(e.flow_axis, crate::ir::FlowAxis::X, "{dir:?} trunks");
            }
            let (ax, _, aw, _) = node(&ir, 1);
            let (bx, _, bw, _) = node(&ir, 2);
            if matches!(dir, Direction::LeftRight) {
                assert!(bx >= ax + aw, "LeftRight: level 1 sits to the right");
            } else {
                assert!(ax >= bx + bw, "RightLeft: level 1 sits to the LEFT");
            }
            // …and it is genuinely not the vertical layout.
            assert_ne!(
                (ir.width(), ir.height()),
                (td.width(), td.height()),
                "{dir:?} is not the TopDown layout"
            );
        }

        // RL is the exact x-mirror of LR through the PUBLIC entry.
        let mut g = build();
        g.set_direction(Direction::LeftRight);
        let lr = g.compute_layout();
        let mut g = build();
        g.set_direction(Direction::RightLeft);
        let rl = g.compute_layout();
        let w = lr.width();
        assert_eq!(w, rl.width());
        for (p, q) in lr.nodes().iter().zip(rl.nodes().iter()) {
            assert_eq!(q.x, w - p.x - p.width, "node {} mirrors", p.id);
            assert_eq!(q.y, p.y);
        }
    }

    /// The `render()` convenience honors the rank direction. A simple
    /// chain under `RenderMode::Auto` takes the compact left-to-right
    /// form ONLY for `TopDown` — its arrow and traversal are
    /// hard-coded, so it cannot express any other direction; the rest
    /// go through the layout pipeline. An explicit
    /// `RenderMode::Horizontal` remains a deliberate override.
    #[cfg(all(feature = "layout-vertical", feature = "layout-horizontal"))]
    #[test]
    fn render_honors_direction_for_simple_chains() {
        let chain = || {
            let mut g = Graph::new();
            g.add_node(1, "A");
            g.add_node(2, "B");
            g.add_edge(1, 2, None);
            g
        };
        // TopDown keeps the compact chain form (byte-frozen).
        assert!(
            chain().render().contains('→'),
            "TopDown Auto keeps the chain shortcut"
        );
        // RightLeft must not silently render left-to-right.
        let mut g = chain();
        g.set_direction(Direction::RightLeft);
        let out = g.render();
        assert!(
            out.contains('←') && !out.contains('→'),
            "RightLeft flows right-to-left:\n{out}"
        );
        // BottomUp likewise reaches the pipeline instead of the
        // unconditionally-downward shortcut.
        let mut g = chain();
        g.set_direction(Direction::BottomUp);
        let out = g.render();
        assert!(out.contains('↑'), "BottomUp flows upward:\n{out}");
        // An explicit Horizontal request still overrides.
        let mut g = chain();
        g.set_direction(Direction::RightLeft);
        g.set_render_mode(crate::RenderMode::Horizontal);
        assert!(
            g.render().contains('→'),
            "explicit Horizontal is a deliberate override"
        );
    }

    // ── Node handles & AUTO numbering (NC-P1) ──────────────────────────

    #[test]
    fn handles_round_trip_and_flow_into_apis() {
        let mut g = Graph::new();
        let a = g.add_node(1, "A");
        let b = g.add_node(2, "B");
        assert_eq!(usize::from(a), 1);
        assert_eq!(a.node, NodeId::from(1));
        assert_eq!(a.id(), 1);
        g.add_edge(a, b, None); // handles as edge endpoints
        let sg = g.add_subgraph("S");
        g.put_nodes(&[a, b]).inside(sg).unwrap(); // handles in placement
        assert_eq!(g.node_subgraph(1), Some(sg));
        // A handle is accepted back in the id slot (replace semantics).
        g.add_node(a, "A2");
        assert_eq!(g.nodes.len(), 2);
        assert_eq!(g.nodes[0].1, "A2");
    }

    #[test]
    fn bare_integer_literals_still_compile() {
        // D3 standing guard (compile pin): `usize` must remain the ONLY
        // integer `From` impl on `IdOrAuto` and `NodeId` — a second one
        // would push these bare literals onto the i32 fallback and
        // break them. The beyond-i32 literal pins the large case.
        let mut g = Graph::new();
        g.add_node(1, "A");
        g.add_node(4_000_000_000, "big");
        g.add_edge(1, 4_000_000_000, None);
        let sg = g.add_subgraph("S");
        g.put_nodes(&[1]).inside(sg).unwrap();
    }

    #[test]
    fn auto_numbers_from_zero_on_fresh_graphs() {
        let mut g = Graph::new();
        assert_eq!(usize::from(g.add_node(AUTO, "a")), 0);
        assert_eq!(usize::from(g.add_node(AUTO, "b")), 1);
        assert_eq!(usize::from(g.add_node(9, "big")), 9);
        assert_eq!(usize::from(g.add_node(AUTO, "c")), 10);
    }

    #[test]
    fn auto_continues_above_every_id_creating_site() {
        let mut g = Graph::new();
        g.add_node(10, "ten");
        g.add_node(3, "three");
        // Explicit-then-AUTO is collision-free by construction (D5).
        assert_eq!(usize::from(g.add_node(AUTO, "next")), 11);
        // Edge auto-creation is an id-creating site too.
        g.add_edge(11, 40, None);
        assert_eq!(usize::from(g.add_node(AUTO, "after-edge")), 41);
        // Subgraph placement references nodes — NOT an id-creating
        // site; the counter must not move.
        let sg = g.add_subgraph("S");
        g.put_nodes(&[40]).inside(sg).unwrap();
        assert_eq!(usize::from(g.add_node(AUTO, "after-place")), 42);
    }

    #[test]
    fn batch_constructors_seed_the_counter() {
        let mut g = Graph::from_edges(&[(100, "a"), (7, "b")], &[(100, 7)]);
        assert_eq!(usize::from(g.add_node(AUTO, "c")), 101);
        let mut g2 = Graph::from_edges_labeled(&[(5, "x")], &[(5, 30, Some("l"))]);
        // Edge auto-created node 30 bumps past the batch maximum.
        assert_eq!(usize::from(g2.add_node(AUTO, "y")), 31);
    }

    #[test]
    fn implicit_then_explicit_falls_through_to_replace() {
        let mut g = Graph::new();
        g.add_node(AUTO, "zero");
        let one = g.add_node(AUTO, "one");
        g.add_node(AUTO, "two");
        // Explicit reuse of a small integer is the standing
        // replace-on-duplicate semantic (D5) — no new node.
        g.add_node(1, "replaced");
        assert_eq!(g.nodes.len(), 3);
        assert_eq!(usize::from(one), 1);
        assert_eq!(g.nodes[1].1, "replaced");
        // The counter keeps counting above everything seen.
        assert_eq!(usize::from(g.add_node(AUTO, "three")), 3);
    }

    /// The counter invariant under pseudo-random op sequences
    /// (deterministic LCG — the crate takes no dev-deps, so no
    /// proptest): after every operation, `next_auto` strictly exceeds
    /// every id the graph has seen.
    #[test]
    fn auto_counter_invariant_over_random_ops() {
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) as usize
        };
        let mut g = Graph::new();
        let mut max_seen: Option<usize> = None;
        for _ in 0..500 {
            let id = match next() % 3 {
                0 => usize::from(g.add_node(next() % 1000, "n")),
                1 => usize::from(g.add_node(AUTO, "a")),
                _ => {
                    let (f, t) = (next() % 1000, next() % 1000);
                    g.add_edge(f, t, None);
                    f.max(t)
                }
            };
            max_seen = Some(max_seen.map_or(id, |m| m.max(id)));
            assert!(
                g.next_auto > max_seen.unwrap(),
                "next_auto {} must exceed max seen id {}",
                g.next_auto,
                max_seen.unwrap()
            );
        }
    }

    /// The state machine behind the D5/D7 replace flags: ids gain
    /// the auto mark when AUTO assigns them, lose it when an explicit
    /// id overwrites them (the transition that sets the receipt's
    /// `replaced_involving_auto`), and a saturated AUTO re-marks. The
    /// set drives every flag decision, so it is the pin.
    #[test]
    #[allow(deprecated)] // with_size's explicit path is part of the pin
    fn auto_numbered_tracking_powers_replace_diagnostics() {
        let mut g = Graph::new();
        let a = g.add_node(AUTO, "a");
        assert!(g.auto_numbered.contains(&a.id()));
        // Explicit replace clears the mark (and fires the diagnostic).
        g.add_node(a.id(), "explicit");
        assert!(!g.auto_numbered.contains(&a.id()));
        // Saturated AUTO replaces at the top and marks the id.
        g.add_node(usize::MAX, "top");
        let s = g.add_node(AUTO, "sat");
        assert_eq!(s.id(), usize::MAX);
        assert!(g.auto_numbered.contains(&usize::MAX));
        // Explicit custom content is an explicit path too — it clears.
        g.add_node(
            usize::MAX,
            crate::CustomNode {
                label: "sized",
                width: 8,
                height: 1,
                painter: None,
                payload: "",
            },
        );
        assert!(!g.auto_numbered.contains(&usize::MAX));
    }

    /// A panicking content accessor must not advance graph state: the
    /// counter moves only on successful creation (review round: the
    /// resolve-then-commit transaction order).
    #[test]
    fn panicking_content_does_not_advance_the_counter() {
        struct Bomb;
        impl<'a> NodeContent<'a> for Bomb {
            fn label(&self) -> &'a str {
                panic!("content accessor panicked");
            }
        }
        let mut g = Graph::new();
        g.add_node(AUTO, "a"); // counter → 1
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            g.add_node(AUTO, Bomb);
        }));
        assert!(result.is_err(), "the bomb must go off");
        assert_eq!(g.nodes.len(), 1, "failed insertion adds nothing");
        // The next AUTO id is 1, not 2 — no id was skipped.
        assert_eq!(usize::from(g.add_node(AUTO, "b")), 1);
    }

    #[test]
    fn auto_counter_saturates_at_the_top() {
        // D7: near usize::MAX the counter saturates; the next AUTO
        // resolves to a taken id and falls through to replace — defined
        // behavior, no panic, no wrap back to small ids.
        let mut g = Graph::new();
        g.add_node(usize::MAX, "top");
        let next = g.add_node(AUTO, "saturated");
        assert_eq!(usize::from(next), usize::MAX);
        assert_eq!(g.nodes.len(), 1); // replaced, not appended
        assert_eq!(g.nodes[0].1, "saturated");
    }

    // ── Node content storage (NC-P2, D6 sparse+packed) ────────────────

    #[test]
    fn content_objects_resolve_to_sparse_storage() {
        use crate::render::engine::{BoxedNode, CustomNode};
        fn probe(
            _: &mut crate::render::engine::NodeRegion<'_, '_>,
            _: crate::render::engine::NodePaintCtx<'_>,
        ) {
        }
        let mut g = Graph::new();
        g.add_node(1, "plain");
        g.add_node(2, BoxedNode("boxed"));
        g.add_node(
            3,
            CustomNode {
                label: "card",
                width: 10,
                height: 4,
                painter: Some(probe),
                payload: "rows",
            },
        );
        g.add_node(
            4,
            CustomNode {
                label: "blank",
                width: 6,
                height: 2,
                painter: None,
                payload: "",
            },
        );
        g.add_node(
            5,
            CustomNode {
                label: "data",
                width: 6,
                height: 2,
                painter: None,
                payload: "json-only",
            },
        );
        assert_eq!(g.node_kind_tag, vec![0, 1, 2, 2, 2]);
        assert_eq!((g.node_widths[1], g.node_heights[1]), (9, 3)); // boxed: label+4 × 3
        // Sparse entries only where painter or payload exists: idx 2
        // (painter+payload) and idx 4 (payload-only blank) — idx 3 is
        // a fully blank custom node, no entry.
        assert_eq!(g.node_custom.len(), 2);
        assert_eq!(g.node_custom[0].0, 2);
        assert!(g.node_custom[0].1.is_some());
        assert_eq!(g.node_custom[0].2, "rows");
        assert_eq!(g.node_custom[1].0, 4);
        assert!(g.node_custom[1].1.is_none());
        assert_eq!(g.node_custom[1].2, "json-only");
    }

    #[test]
    #[allow(deprecated)] // with_size replace-clears are part of the pin
    fn replace_maintains_the_sparse_list() {
        use crate::render::engine::CustomNode;
        fn probe(
            _: &mut crate::render::engine::NodeRegion<'_, '_>,
            _: crate::render::engine::NodePaintCtx<'_>,
        ) {
        }
        let custom = |label| CustomNode {
            label,
            width: 8,
            height: 3,
            painter: Some(probe),
            payload: "p",
        };
        let mut g = Graph::new();
        g.add_node(1, custom("a"));
        g.add_node(2, "plain");
        g.add_node(3, custom("c"));
        assert_eq!(g.node_custom.len(), 2);
        // Custom → simple removes the mid-list entry (risk 6).
        g.add_node(1, "now-plain");
        assert_eq!(g.node_custom.len(), 1);
        assert_eq!(g.node_custom[0].0, 2);
        assert_eq!(g.node_kind_tag[0], 0);
        // Simple → custom inserts in sorted position.
        g.add_node(2, custom("b"));
        assert_eq!(g.node_custom.len(), 2);
        assert_eq!((g.node_custom[0].0, g.node_custom[1].0), (1, 2));
        // A simple node replacing a custom node clears its entry.
        g.add_node(3, "sized");
        assert_eq!(g.node_custom.len(), 1);
        assert_eq!(g.node_kind_tag[2], 0);
        // Re-declaring custom content restores the entry with the
        // declared size.
        g.add_node(
            3,
            CustomNode {
                label: "c",
                width: 20,
                height: 6,
                painter: Some(probe),
                payload: "p",
            },
        );
        assert_eq!((g.node_widths[2], g.node_heights[2]), (20, 6));
        assert_eq!(g.node_custom.len(), 2);
    }

    #[test]
    fn declared_sizes_and_empty_label_widths() {
        let empty_custom = |w, h| crate::CustomNode {
            label: "",
            width: w,
            height: h,
            painter: None,
            payload: "",
        };
        let mut g = Graph::new();
        // Empty label with the default footprint = today's ⟨id⟩ width.
        g.add_node(42, "");
        assert_eq!(g.node_widths[0], 4); // ⟨42⟩
        // A declared size wins even for an empty label — including one
        // that happens to equal the default footprint (size provenance,
        // not a value heuristic).
        g.add_node(3, empty_custom(10, 1));
        assert_eq!(g.node_widths[1], 10);
        g.add_node(12345, empty_custom(2, 1));
        assert_eq!(g.node_widths[2], 2); // NOT widened to ⟨12345⟩ = 7
    }

    // ── Subgraph creation ──────────────────────────────────────────────

    #[test]
    fn add_subgraph_returns_unique_ids() {
        let mut g = Graph::new();
        let a = g.add_subgraph("A");
        let b = g.add_subgraph("B");
        assert_ne!(a, b);
        assert_eq!(g.subgraph_count(), 2);
    }

    #[test]
    fn subgraph_accessor() {
        let mut g = Graph::new();
        let id = g.add_subgraph("Backend");
        let sg = g.subgraph(id).unwrap();
        assert_eq!(sg.label, "Backend");
        assert_eq!(sg.parent_id, None);
    }

    #[test]
    fn has_subgraphs_flag() {
        let mut g = Graph::new();
        assert!(!g.has_subgraphs());
        g.add_subgraph("X");
        assert!(g.has_subgraphs());
    }

    // ── put_nodes().inside() ───────────────────────────────────────────

    #[test]
    fn put_nodes_inside_subgraph() {
        let mut g = Graph::new();
        g.add_node(1, "A");
        g.add_node(2, "B");
        let sg = g.add_subgraph("cluster");
        g.put_nodes(&[1, 2]).inside(sg).unwrap();
        assert_eq!(g.node_subgraph(1), Some(sg));
        assert_eq!(g.node_subgraph(2), Some(sg));
    }

    #[test]
    fn put_nodes_unknown_subgraph_returns_error() {
        let mut g = Graph::new();
        g.add_node(1, "A");
        let result = g.put_nodes(&[1]).inside(999);
        assert!(result.is_err());
        match result.unwrap_err() {
            GraphError::SubgraphNotFound(id) => assert_eq!(id, 999),
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn put_nodes_unknown_node_returns_error() {
        let mut g = Graph::new();
        let sg = g.add_subgraph("X");
        let result = g.put_nodes(&[42]).inside(sg);
        assert!(result.is_err());
        match result.unwrap_err() {
            GraphError::NodeNotFound(id) => assert_eq!(id, 42),
            other => panic!("unexpected error: {:?}", other),
        }
    }

    // ── put_subgraphs().inside() (nesting) ─────────────────────────────

    #[test]
    fn nest_subgraph_inside_parent() {
        let mut g = Graph::new();
        let parent = g.add_subgraph("outer");
        let child = g.add_subgraph("inner");
        g.put_subgraphs(&[child]).inside(parent).unwrap();
        assert_eq!(g.subgraph(child).unwrap().parent_id, Some(parent));
    }

    #[test]
    fn nesting_cycle_detection() {
        let mut g = Graph::new();
        let a = g.add_subgraph("A");
        let b = g.add_subgraph("B");
        g.put_subgraphs(&[b]).inside(a).unwrap();
        // Trying to put A inside B creates a cycle
        let result = g.put_subgraphs(&[a]).inside(b);
        assert!(result.is_err());
        match result.unwrap_err() {
            GraphError::SubgraphCycle => {}
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn deep_nesting_cycle_detection() {
        let mut g = Graph::new();
        let a = g.add_subgraph("A");
        let b = g.add_subgraph("B");
        let c = g.add_subgraph("C");
        g.put_subgraphs(&[b]).inside(a).unwrap();
        g.put_subgraphs(&[c]).inside(b).unwrap();
        // A → B → C; putting A inside C creates A → B → C → A
        let result = g.put_subgraphs(&[a]).inside(c);
        assert!(result.is_err());
    }

    // ── Layout integration ─────────────────────────────────────────────

    #[test]
    fn single_cluster_produces_ir_subgraph() {
        let mut g = Graph::new();
        g.add_node(1, "X");
        g.add_node(2, "Y");
        g.add_edge(1, 2, None);
        let sg = g.add_subgraph("cluster");
        g.put_nodes(&[1, 2]).inside(sg).unwrap();

        let ir = g.compute_layout();
        assert_eq!(ir.subgraphs().len(), 1);
        let info = &ir.subgraphs()[0];
        assert_eq!(info.label, "cluster");
        assert!(info.width > 0);
        assert!(info.height > 0);
    }

    #[test]
    fn sibling_clusters_no_bbox_overlap() {
        let mut g = Graph::new();
        g.add_node(1, "A");
        g.add_node(2, "B");
        g.add_node(3, "C");
        g.add_node(4, "D");
        g.add_edge(1, 2, None);
        g.add_edge(3, 4, None);

        let s1 = g.add_subgraph("Left");
        let s2 = g.add_subgraph("Right");
        g.put_nodes(&[1, 2]).inside(s1).unwrap();
        g.put_nodes(&[3, 4]).inside(s2).unwrap();

        let ir = g.compute_layout();
        assert_eq!(ir.subgraphs().len(), 2);

        let b1 = &ir.subgraphs()[0];
        let b2 = &ir.subgraphs()[1];

        // Bounding boxes should not overlap in BOTH x and y simultaneously
        let x_overlap = b1.x < b2.x + b2.width && b2.x < b1.x + b1.width;
        let y_overlap = b1.y < b2.y + b2.height && b2.y < b1.y + b1.height;
        assert!(
            !(x_overlap && y_overlap),
            "sibling bounding boxes overlap: {:?} and {:?}",
            (b1.x, b1.y, b1.width, b1.height),
            (b2.x, b2.y, b2.width, b2.height),
        );
    }

    #[test]
    fn nested_cluster_bbox_contained_by_parent() {
        let mut g = Graph::new();
        g.add_node(1, "API");
        g.add_node(2, "DB");
        g.add_edge(1, 2, None);

        let outer = g.add_subgraph("Backend");
        let inner = g.add_subgraph("Database");
        g.put_nodes(&[1, 2]).inside(outer).unwrap();
        g.put_nodes(&[2]).inside(inner).unwrap();
        g.put_subgraphs(&[inner]).inside(outer).unwrap();

        let ir = g.compute_layout();
        assert_eq!(ir.subgraphs().len(), 2);

        let parent = ir
            .subgraphs()
            .iter()
            .find(|s| s.label == "Backend")
            .unwrap();
        let child = ir
            .subgraphs()
            .iter()
            .find(|s| s.label == "Database")
            .unwrap();

        // Child must be fully contained within parent
        assert!(
            child.x >= parent.x,
            "child.x={} < parent.x={}",
            child.x,
            parent.x
        );
        assert!(
            child.y >= parent.y,
            "child.y={} < parent.y={}",
            child.y,
            parent.y
        );
        assert!(
            child.x + child.width <= parent.x + parent.width,
            "child right edge {} > parent right edge {}",
            child.x + child.width,
            parent.x + parent.width,
        );
        assert!(
            child.y + child.height <= parent.y + parent.height,
            "child bottom {} > parent bottom {}",
            child.y + child.height,
            parent.y + parent.height,
        );
    }

    #[test]
    fn render_with_subgraph_does_not_panic() {
        let mut g = Graph::new();
        g.add_node(1, "A");
        g.add_node(2, "B");
        g.add_edge(1, 2, None);
        let sg = g.add_subgraph("test");
        g.put_nodes(&[1, 2]).inside(sg).unwrap();
        // Should not panic (scanline renderer has border support)
        let ir = g.compute_layout();
        let output = ir.render_string(&crate::render::engine::RenderOptions::plain());
        assert!(!output.is_empty());
        // Border characters should appear (double-line style)
        assert!(output.contains('╔'));
        assert!(output.contains('╝'));
    }

    #[test]
    fn render_label_appears_in_output() {
        let mut g = Graph::new();
        g.add_node(1, "Node");
        let sg = g.add_subgraph("MyCluster");
        g.put_nodes(&[1]).inside(sg).unwrap();
        let ir = g.compute_layout();
        let output = ir.render_string(&crate::render::engine::RenderOptions::plain());
        assert!(
            output.contains("MyCluster"),
            "label not found in output:\n{output}",
        );
    }

    // ── Spacing config (regression: fields were silently ignored) ──────

    /// R fans out to A/B/C (three adjacent nodes on level 1), converging on S.
    #[cfg(feature = "layout-vertical")]
    fn spacing_test_graph() -> Graph<'static> {
        Graph::from_edges(
            &[(1, "R"), (2, "A"), (3, "B"), (4, "C"), (5, "S")],
            &[(1, 2), (1, 3), (1, 4), (2, 5), (3, 5), (4, 5)],
        )
    }

    #[cfg(feature = "layout-vertical")]
    #[test]
    fn node_spacing_config_is_applied() {
        let g = spacing_test_graph();
        for spacing in [3usize, 8] {
            let mut cfg = LayoutConfig::standard();
            cfg.node_spacing = spacing;
            let ir = g.compute_layout_with_config(&cfg);
            let mut boxes: Vec<(usize, usize)> = ir
                .nodes()
                .iter()
                .filter(|n| n.level == 1)
                .map(|n| (n.x, n.width))
                .collect();
            boxes.sort_unstable();
            assert_eq!(boxes.len(), 3);
            for pair in boxes.windows(2) {
                let gap = pair[1].0 - (pair[0].0 + pair[0].1);
                assert_eq!(
                    gap, spacing,
                    "gap between adjacent nodes should equal node_spacing={spacing}"
                );
            }
        }
    }

    #[cfg(feature = "layout-vertical")]
    #[test]
    fn level_spacing_config_is_applied() {
        let g = spacing_test_graph();
        let base = g.compute_layout_with_config(&LayoutConfig::standard());
        let mut cfg = LayoutConfig::standard();
        cfg.level_spacing = 2;
        let spaced = g.compute_layout_with_config(&cfg);

        let y_at = |ir: &crate::ir::LayoutIR<'_>, level: usize| {
            ir.nodes().iter().find(|n| n.level == level).unwrap().y
        };
        // Each of the two inter-level gaps grows by level_spacing.
        assert_eq!(y_at(&spaced, 1), y_at(&base, 1) + 2);
        assert_eq!(y_at(&spaced, 2), y_at(&base, 2) + 4);
        // No trailing gap after the last level: total height grows by
        // exactly (levels - 1) * level_spacing.
        assert_eq!(spaced.height(), base.height() + 4);
    }

    // ── Cluster-width feedback (regression: external nodes rendered
    //    inside subgraph borders) ────────────────────────────────────────

    fn assert_externals_clear(ir: &crate::ir::LayoutIR<'_>, externals: &[&str]) {
        for sg in ir.subgraphs() {
            assert!(
                sg.x + sg.width <= ir.width(),
                "canvas clips subgraph '{}' (border right {} > width {})",
                sg.label,
                sg.x + sg.width,
                ir.width(),
            );
            for n in ir.nodes().iter().filter(|n| externals.contains(&n.label)) {
                let x_overlap = n.x < sg.x + sg.width && n.x + n.width > sg.x;
                let y_overlap = n.y >= sg.y && n.y < sg.y + sg.height;
                assert!(
                    !(x_overlap && y_overlap),
                    "external node '{}' overlaps subgraph '{}' box",
                    n.label,
                    sg.label,
                );
            }
        }
    }

    #[test]
    fn label_widened_subgraph_does_not_swallow_external_nodes() {
        let mut g = Graph::new();
        g.add_node(1, "X");
        g.add_node(2, "E");
        g.add_node(3, "X2");
        g.add_node(4, "E2");
        g.add_edge(1, 3, None);
        g.add_edge(2, 4, None);
        let sg = g.add_subgraph("VeryLongSubgraphLabelHere");
        g.put_nodes(&[1, 3]).inside(sg).unwrap();
        assert_externals_clear(&g.compute_layout(), &["E", "E2"]);
    }

    #[cfg(feature = "layout-vertical")]
    #[test]
    fn levels_tighten_after_sibling_cluster_shifts() {
        // Tier-2 pattern from examples/subgraph_stress.rs: crossing
        // reduction orders Analytics between the two Core DBs; sibling
        // overlap repair then shifts Data Pipeline right, leaving a hole.
        // tighten_levels must reclaim it so children align with parents.
        let mut g = Graph::new();
        for (id, label) in [
            (1, "LoadBalancer"),
            (2, "CDN"),
            (10, "WebApp"),
            (11, "MobileAPI"),
            (12, "SSR-Engine"),
            (20, "AuthSvc"),
            (21, "SessionDB"),
            (22, "UserSvc"),
            (23, "ProfileDB"),
            (30, "Analytics"),
            (31, "Warehouse"),
            (32, "ETL"),
        ] {
            g.add_node(id, label);
        }
        for (f, t) in [
            (1, 10),
            (1, 11),
            (2, 12),
            (10, 20),
            (11, 20),
            (12, 22),
            (20, 21),
            (22, 23),
            (20, 30),
            (22, 30),
            (30, 31),
            (30, 32),
        ] {
            g.add_edge(f, t, None);
        }
        let frontend = g.add_subgraph("Frontend");
        let backend = g.add_subgraph("Backend");
        let core = g.add_subgraph("Core");
        let data = g.add_subgraph("Data Pipeline");
        g.put_nodes(&[10, 11, 12]).inside(frontend).unwrap();
        g.put_nodes(&[20, 21, 22, 23]).inside(core).unwrap();
        g.put_nodes(&[30, 31, 32]).inside(data).unwrap();
        g.put_subgraphs(&[core, data]).inside(backend).unwrap();

        let ir = g.compute_layout();
        let center = |label: &str| {
            let n = ir.nodes().iter().find(|n| n.label == label).unwrap();
            n.x + n.width / 2
        };
        assert!(
            center("SessionDB").abs_diff(center("AuthSvc")) <= 3,
            "SessionDB (center {}) should sit under AuthSvc (center {})",
            center("SessionDB"),
            center("AuthSvc"),
        );
        assert!(
            center("ProfileDB").abs_diff(center("UserSvc")) <= 3,
            "ProfileDB (center {}) should sit under UserSvc (center {})",
            center("ProfileDB"),
            center("UserSvc"),
        );
    }

    /// Tier-3 pattern from examples/subgraph_stress.rs: nested regions,
    /// a CI/CD cluster whose member has children deep inside eu-west-1,
    /// and cross-cluster edges. Shared by the tightening/compaction tests.
    fn tier3_like_graph() -> Graph<'static> {
        let mut g = Graph::new();
        for (id, label) in [
            (1, "ALB"),
            (2, "WAF"),
            (3, "Web-A1"),
            (4, "App-A1"),
            (5, "DB-A1"),
            (6, "Web-A2"),
            (7, "App-A2"),
            (8, "DB-A2"),
            (9, "Web-B1"),
            (10, "App-B1"),
            (11, "Redis-B1"),
            (12, "DB-B1"),
            (13, "Web-B2"),
            (14, "App-B2"),
            (15, "Redis-B2"),
            (16, "DB-B2"),
            (17, "RabbitMQ"),
            (18, "S3-Bucket"),
            (19, "Vault"),
            (20, "Jenkins"),
            (21, "ArgoCD"),
            (22, "ECR"),
            (23, "Prometheus"),
            (24, "Grafana"),
            (25, "Loki"),
        ] {
            g.add_node(id, label);
        }
        for (f, t) in [
            (1, 2),
            (2, 3),
            (3, 4),
            (4, 5),
            (2, 6),
            (6, 7),
            (7, 8),
            (2, 9),
            (9, 10),
            (10, 11),
            (11, 12),
            (2, 13),
            (13, 14),
            (14, 15),
            (15, 16),
            (4, 17),
            (10, 17),
            (17, 18),
            (20, 22),
            (22, 21),
            (21, 3),
            (21, 9),
            (23, 24),
            (25, 24),
            (4, 23),
            (10, 25),
        ] {
            g.add_edge(f, t, None);
        }
        let az_a1 = g.add_subgraph("AZ-1");
        let az_a2 = g.add_subgraph("AZ-2");
        let region_a = g.add_subgraph("us-east-1");
        g.put_nodes(&[3, 4, 5]).inside(az_a1).unwrap();
        g.put_nodes(&[6, 7, 8]).inside(az_a2).unwrap();
        g.put_subgraphs(&[az_a1, az_a2]).inside(region_a).unwrap();
        let az_b1 = g.add_subgraph("AZ-1");
        let az_b2 = g.add_subgraph("AZ-2");
        let region_b = g.add_subgraph("eu-west-1");
        g.put_nodes(&[9, 10, 11, 12]).inside(az_b1).unwrap();
        g.put_nodes(&[13, 14, 15, 16]).inside(az_b2).unwrap();
        g.put_subgraphs(&[az_b1, az_b2]).inside(region_b).unwrap();
        let shared = g.add_subgraph("Shared Services");
        g.put_nodes(&[17, 18, 19]).inside(shared).unwrap();
        let cicd = g.add_subgraph("CI/CD");
        g.put_nodes(&[20, 21, 22]).inside(cicd).unwrap();
        let obs = g.add_subgraph("Observability");
        g.put_nodes(&[23, 24, 25]).inside(obs).unwrap();
        g
    }

    #[test]
    fn tightening_never_overlaps_disjoint_boxes() {
        // ArgoCD (CI/CD) has children deep inside eu-west-1. Tightening
        // must not pull it out of the CI/CD envelope — that would expand
        // the CI/CD box into the eu-west-1 box after the sibling shifts
        // already separated them.
        let g = tier3_like_graph();
        let ir = g.compute_layout();
        let boxes = ir.subgraphs();
        let is_ancestor = |anc: usize, mut id: usize| -> bool {
            loop {
                let parent = boxes.iter().find(|s| s.id == id).and_then(|s| s.parent_id);
                match parent {
                    Some(p) if p == anc => return true,
                    Some(p) => id = p,
                    None => return false,
                }
            }
        };
        for a in boxes {
            for b in boxes {
                if a.id >= b.id || is_ancestor(a.id, b.id) || is_ancestor(b.id, a.id) {
                    continue;
                }
                let x_overlap = a.x < b.x + b.width && b.x < a.x + a.width;
                let y_overlap = a.y < b.y + b.height && b.y < a.y + a.height;
                assert!(
                    !(x_overlap && y_overlap),
                    "boxes '{}' ({},{} {}x{}) and '{}' ({},{} {}x{}) overlap",
                    a.label,
                    a.x,
                    a.y,
                    a.width,
                    a.height,
                    b.label,
                    b.x,
                    b.y,
                    b.width,
                    b.height,
                );
            }
        }
    }

    /// Tier-5 pattern from examples/subgraph_stress.rs (80 nodes, 24
    /// clusters, depth 4): the only known reproducer for stale dummy
    /// waypoints crossing node text after inter-cluster compaction.
    #[cfg(feature = "layout-vertical")]
    fn tier5_like_graph() -> Graph<'static> {
        let mut g = Graph::new();
        let mut id = 0usize;
        let mut next = || {
            id += 1;
            id
        };

        // ── Global Entry ──
        let dns = next();
        g.add_node(dns, "Route53");
        let cdn = next();
        g.add_node(cdn, "CloudFront");
        let waf = next();
        g.add_node(waf, "WAF");
        g.add_edge(dns, cdn, None);
        g.add_edge(cdn, waf, None);

        // ── US Region ──
        //   Frontend
        let us_web = next();
        g.add_node(us_web, "US-Web");
        let us_mobile = next();
        g.add_node(us_mobile, "US-Mobile");
        let us_ssr = next();
        g.add_node(us_ssr, "US-SSR");
        g.add_edge(waf, us_web, None);
        g.add_edge(waf, us_mobile, None);
        g.add_edge(us_web, us_ssr, None);

        //   Auth micro
        let us_auth = next();
        g.add_node(us_auth, "US-Auth");
        let us_token = next();
        g.add_node(us_token, "US-Token");
        let us_mfa = next();
        g.add_node(us_mfa, "US-MFA");
        g.add_edge(us_web, us_auth, None);
        g.add_edge(us_mobile, us_auth, None);
        g.add_edge(us_auth, us_token, None);
        g.add_edge(us_auth, us_mfa, None);

        //   Biz logic
        let us_order = next();
        g.add_node(us_order, "US-Orders");
        let us_pay = next();
        g.add_node(us_pay, "US-Pay");
        let us_inv = next();
        g.add_node(us_inv, "US-Inv");
        let us_ship = next();
        g.add_node(us_ship, "US-Ship");
        g.add_edge(us_auth, us_order, None);
        g.add_edge(us_order, us_pay, None);
        g.add_edge(us_order, us_inv, None);
        g.add_edge(us_order, us_ship, None);

        //   Databases
        let us_pg = next();
        g.add_node(us_pg, "US-Postgres");
        let us_redis = next();
        g.add_node(us_redis, "US-Redis");
        let us_s3 = next();
        g.add_node(us_s3, "US-S3");
        g.add_edge(us_order, us_pg, None);
        g.add_edge(us_pay, us_pg, None);
        g.add_edge(us_auth, us_redis, None);
        g.add_edge(us_inv, us_s3, None);

        // ── EU Region (mirror, smaller) ──
        let eu_web = next();
        g.add_node(eu_web, "EU-Web");
        let eu_auth = next();
        g.add_node(eu_auth, "EU-Auth");
        let eu_order = next();
        g.add_node(eu_order, "EU-Orders");
        let eu_pay = next();
        g.add_node(eu_pay, "EU-Pay");
        let eu_ship = next();
        g.add_node(eu_ship, "EU-Ship");
        let eu_pg = next();
        g.add_node(eu_pg, "EU-Postgres");
        let eu_redis = next();
        g.add_node(eu_redis, "EU-Redis");
        g.add_edge(waf, eu_web, None);
        g.add_edge(eu_web, eu_auth, None);
        g.add_edge(eu_auth, eu_order, None);
        g.add_edge(eu_order, eu_pay, None);
        g.add_edge(eu_order, eu_ship, None);
        g.add_edge(eu_order, eu_pg, None);
        g.add_edge(eu_auth, eu_redis, None);

        // ── APAC Region ──
        let ap_web = next();
        g.add_node(ap_web, "AP-Web");
        let ap_auth = next();
        g.add_node(ap_auth, "AP-Auth");
        let ap_order = next();
        g.add_node(ap_order, "AP-Orders");
        let ap_pg = next();
        g.add_node(ap_pg, "AP-Postgres");
        g.add_edge(waf, ap_web, None);
        g.add_edge(ap_web, ap_auth, None);
        g.add_edge(ap_auth, ap_order, None);
        g.add_edge(ap_order, ap_pg, None);

        // ── Messaging Layer ──
        let kafka1 = next();
        g.add_node(kafka1, "Kafka-1");
        let kafka2 = next();
        g.add_node(kafka2, "Kafka-2");
        let zk = next();
        g.add_node(zk, "Zookeeper");
        let schema = next();
        g.add_node(schema, "SchemaReg");
        g.add_edge(us_order, kafka1, None);
        g.add_edge(eu_order, kafka1, None);
        g.add_edge(kafka1, kafka2, None);
        g.add_edge(kafka1, zk, None);
        g.add_edge(kafka2, schema, None);

        // ── Data Platform ──
        let spark = next();
        g.add_node(spark, "Spark");
        let flink = next();
        g.add_node(flink, "Flink");
        let airflow = next();
        g.add_node(airflow, "Airflow");
        let datalake = next();
        g.add_node(datalake, "DataLake");
        let redshift = next();
        g.add_node(redshift, "Redshift");
        let tableau = next();
        g.add_node(tableau, "Tableau");
        g.add_edge(kafka2, spark, None);
        g.add_edge(kafka2, flink, None);
        g.add_edge(airflow, spark, None);
        g.add_edge(spark, datalake, None);
        g.add_edge(flink, datalake, None);
        g.add_edge(datalake, redshift, None);
        g.add_edge(redshift, tableau, None);

        // ── ML Platform ──
        let mlflow = next();
        g.add_node(mlflow, "MLflow");
        let sagemaker = next();
        g.add_node(sagemaker, "SageMaker");
        let model_reg = next();
        g.add_node(model_reg, "ModelReg");
        let inference = next();
        g.add_node(inference, "Inference");
        g.add_edge(datalake, mlflow, None);
        g.add_edge(mlflow, sagemaker, None);
        g.add_edge(sagemaker, model_reg, None);
        g.add_edge(model_reg, inference, None);
        g.add_edge(inference, us_order, None); // predictions feed back

        // ── Observability ──
        let prom = next();
        g.add_node(prom, "Prometheus");
        let grafana = next();
        g.add_node(grafana, "Grafana");
        let jaeger = next();
        g.add_node(jaeger, "Jaeger");
        let loki = next();
        g.add_node(loki, "Loki");
        let cortex = next();
        g.add_node(cortex, "Cortex");
        let pager = next();
        g.add_node(pager, "PagerDuty");
        let opsgenie = next();
        g.add_node(opsgenie, "OpsGenie");
        g.add_edge(us_order, prom, None);
        g.add_edge(eu_order, prom, None);
        g.add_edge(prom, cortex, None);
        g.add_edge(cortex, grafana, None);
        g.add_edge(prom, jaeger, None);
        g.add_edge(prom, loki, None);
        g.add_edge(grafana, pager, None);
        g.add_edge(grafana, opsgenie, None);

        // ── Security ──
        let vault = next();
        g.add_node(vault, "Vault");
        let cert_mgr = next();
        g.add_node(cert_mgr, "CertMgr");
        let guard = next();
        g.add_node(guard, "GuardDuty");
        let inspector = next();
        g.add_node(inspector, "Inspector");
        g.add_edge(us_auth, vault, None);
        g.add_edge(eu_auth, vault, None);
        g.add_edge(vault, cert_mgr, None);
        g.add_edge(vault, guard, None);
        g.add_edge(guard, inspector, None);

        // ── DevOps / Platform ──
        let github = next();
        g.add_node(github, "GitHub");
        let ci = next();
        g.add_node(ci, "Actions");
        let ecr = next();
        g.add_node(ecr, "ECR");
        let argo = next();
        g.add_node(argo, "ArgoCD");
        let tf = next();
        g.add_node(tf, "Terraform");
        let k8s_us = next();
        g.add_node(k8s_us, "EKS-US");
        let k8s_eu = next();
        g.add_node(k8s_eu, "EKS-EU");
        let k8s_ap = next();
        g.add_node(k8s_ap, "EKS-AP");
        g.add_edge(github, ci, None);
        g.add_edge(ci, ecr, None);
        g.add_edge(ecr, argo, None);
        g.add_edge(argo, k8s_us, None);
        g.add_edge(argo, k8s_eu, None);
        g.add_edge(argo, k8s_ap, None);
        g.add_edge(tf, k8s_us, None);
        g.add_edge(tf, k8s_eu, None);
        g.add_edge(tf, k8s_ap, None);

        // ── Notifications ──
        let sns = next();
        g.add_node(sns, "SNS");
        let ses = next();
        g.add_node(ses, "SES");
        let slack_hook = next();
        g.add_node(slack_hook, "Slack");
        g.add_edge(pager, sns, None);
        g.add_edge(sns, ses, None);
        g.add_edge(sns, slack_hook, None);

        // ── Build subgraph hierarchy ──

        // US Region → Frontend, Auth, Business, Data
        let us_fe = g.add_subgraph("US-Frontend");
        g.put_nodes(&[us_web, us_mobile, us_ssr])
            .inside(us_fe)
            .unwrap();
        let us_au = g.add_subgraph("US-Auth");
        g.put_nodes(&[us_auth, us_token, us_mfa])
            .inside(us_au)
            .unwrap();
        let us_biz = g.add_subgraph("US-Business");
        g.put_nodes(&[us_order, us_pay, us_inv, us_ship])
            .inside(us_biz)
            .unwrap();
        let us_data = g.add_subgraph("US-Data");
        g.put_nodes(&[us_pg, us_redis, us_s3])
            .inside(us_data)
            .unwrap();
        let region_us = g.add_subgraph("US-East");
        g.put_subgraphs(&[us_fe, us_au, us_biz, us_data])
            .inside(region_us)
            .unwrap();

        // EU Region
        let eu_svc = g.add_subgraph("EU-Services");
        g.put_nodes(&[eu_web, eu_auth, eu_order, eu_pay, eu_ship])
            .inside(eu_svc)
            .unwrap();
        let eu_db = g.add_subgraph("EU-Data");
        g.put_nodes(&[eu_pg, eu_redis]).inside(eu_db).unwrap();
        let region_eu = g.add_subgraph("EU-West");
        g.put_subgraphs(&[eu_svc, eu_db]).inside(region_eu).unwrap();

        // APAC Region
        let region_ap = g.add_subgraph("APAC");
        g.put_nodes(&[ap_web, ap_auth, ap_order, ap_pg])
            .inside(region_ap)
            .unwrap();

        // Messaging
        let sg_msg = g.add_subgraph("Event Bus");
        g.put_nodes(&[kafka1, kafka2, zk, schema])
            .inside(sg_msg)
            .unwrap();

        // Data Platform
        let sg_ingest = g.add_subgraph("Ingestion");
        g.put_nodes(&[spark, flink]).inside(sg_ingest).unwrap();
        let sg_store = g.add_subgraph("Storage");
        g.put_nodes(&[datalake, redshift]).inside(sg_store).unwrap();
        let sg_dp = g.add_subgraph("Data Platform");
        g.put_nodes(&[airflow, tableau]).inside(sg_dp).unwrap();
        g.put_subgraphs(&[sg_ingest, sg_store])
            .inside(sg_dp)
            .unwrap();

        // ML Platform
        let sg_ml = g.add_subgraph("ML Platform");
        g.put_nodes(&[mlflow, sagemaker, model_reg, inference])
            .inside(sg_ml)
            .unwrap();

        // Observability
        let sg_metrics = g.add_subgraph("Metrics");
        g.put_nodes(&[prom, cortex, grafana])
            .inside(sg_metrics)
            .unwrap();
        let sg_tracing = g.add_subgraph("Tracing");
        g.put_nodes(&[jaeger, loki]).inside(sg_tracing).unwrap();
        let sg_alert = g.add_subgraph("Alerting");
        g.put_nodes(&[pager, opsgenie]).inside(sg_alert).unwrap();
        let sg_obs = g.add_subgraph("Observability");
        g.put_subgraphs(&[sg_metrics, sg_tracing, sg_alert])
            .inside(sg_obs)
            .unwrap();

        // Security
        let sg_sec = g.add_subgraph("Security");
        g.put_nodes(&[vault, cert_mgr, guard, inspector])
            .inside(sg_sec)
            .unwrap();

        // DevOps
        let sg_cicd = g.add_subgraph("CI/CD");
        g.put_nodes(&[github, ci, ecr, argo])
            .inside(sg_cicd)
            .unwrap();
        let sg_infra = g.add_subgraph("Infrastructure");
        g.put_nodes(&[tf, k8s_us, k8s_eu, k8s_ap])
            .inside(sg_infra)
            .unwrap();
        let sg_devops = g.add_subgraph("Platform Eng");
        g.put_subgraphs(&[sg_cicd, sg_infra])
            .inside(sg_devops)
            .unwrap();

        // Notifications
        let sg_notif = g.add_subgraph("Notifications");
        g.put_nodes(&[sns, ses, slack_hook])
            .inside(sg_notif)
            .unwrap();

        g
    }

    /// Assert no edge's vertical segment runs through node text.
    #[cfg(feature = "layout-vertical")]
    fn assert_no_edge_crosses_nodes(ir: &crate::ir::LayoutIR<'_>) {
        use crate::ir::EdgePath;
        for edge in ir.edges() {
            let EdgePath::MultiSegment { waypoints, .. } = &edge.path else {
                continue;
            };
            // Vertical segments: (column, y range) per polyline leg.
            let mut segments: Vec<(usize, usize, usize)> = Vec::new();
            if let Some(&(x0, y0)) = waypoints.first() {
                // The painter starts this vertical at from_y + 1 + offset —
                // strictly below the source's node row.
                let start = edge.from_y + 1;
                segments.push((x0, start.min(y0), start.max(y0)));
            }
            for pair in waypoints.windows(2) {
                let (_, y_prev) = pair[0];
                let (x, y) = pair[1];
                segments.push((x, y_prev.min(y), y_prev.max(y)));
            }
            if let Some(&(_, y_last)) = waypoints.last() {
                segments.push((edge.to_x, y_last.min(edge.to_y), y_last.max(edge.to_y)));
            }
            for node in ir.nodes() {
                if node.id == edge.from_id || node.id == edge.to_id {
                    continue;
                }
                for &(col, y0, y1) in &segments {
                    let x_hit = col >= node.x && col < node.x + node.width;
                    let y_hit = y0 < node.y + node.height && node.y <= y1;
                    assert!(
                        !(x_hit && y_hit),
                        "edge {}→{} vertical at column {} (rows {}..{}) crosses node '{}' at ({},{} {}x{})",
                        edge.from_id,
                        edge.to_id,
                        col,
                        y0,
                        y1,
                        node.label,
                        node.x,
                        node.y,
                        node.width,
                        node.height,
                    );
                }
            }
        }
    }

    #[cfg(feature = "layout-vertical")]
    #[test]
    fn edge_verticals_never_cross_nodes() {
        // The coordinate passes (overlap repair, tightening, compaction)
        // move real nodes; dummy waypoints must be realigned and nudged so
        // no edge's vertical segment runs through node text. Crossing a
        // subgraph border is acceptable; crossing a node never is.
        assert_no_edge_crosses_nodes(&tier3_like_graph().compute_layout());
        assert_no_edge_crosses_nodes(&tier5_like_graph().compute_layout());
    }

    #[test]
    fn clusters_compact_after_separation() {
        // compact_clusters pulls root clusters back together after the
        // sibling-overlap shifts. Without it this layout leaves wide empty
        // gulfs between boxes and the canvas exceeds 450 columns.
        let g = tier3_like_graph();
        let ir = g.compute_layout();
        assert!(
            ir.width() <= 420,
            "canvas width {} suggests inter-cluster compaction regressed",
            ir.width(),
        );
    }

    #[test]
    fn book_length_subgraph_label_is_capped() {
        let long_label = "L".repeat(300);
        let mut g = Graph::new();
        g.add_node(1, "A");
        g.add_node(2, "B");
        g.add_edge(1, 2, None);
        let sg = g.add_subgraph(&long_label);
        g.put_nodes(&[1, 2]).inside(sg).unwrap();
        let ir = g.compute_layout();
        let info = &ir.subgraphs()[0];
        assert!(
            info.width <= 40,
            "label must not widen the box past the cap (got {})",
            info.width,
        );
        assert!(
            ir.width() < 100,
            "canvas must not scale with label length (got {})",
            ir.width(),
        );
        // Renderer truncates the label to the box interior.
        let out = ir.render_string(&crate::render::engine::RenderOptions::plain());
        assert!(out.lines().all(|l| l.chars().count() <= ir.width()));
    }

    #[test]
    fn cross_level_subgraph_envelope_does_not_swallow_external_nodes() {
        let mut g = Graph::new();
        g.add_node(1, "WideMemberNodeAAA");
        g.add_node(2, "WideMemberNodeBBB");
        g.add_node(3, "m");
        g.add_node(4, "ext");
        g.add_edge(1, 3, None);
        g.add_edge(2, 3, None);
        g.add_edge(1, 4, None);
        let sg = g.add_subgraph("C");
        g.put_nodes(&[1, 2, 3]).inside(sg).unwrap();
        assert_externals_clear(&g.compute_layout(), &["ext"]);
    }
}
