//! Structured error codes and diagnostics following the Waddling ErrorChain
//! Protocol (WDP) Level 0.
//!
//! Error codes follow the format `Severity.Component.Primary.Sequence`:
//!
//! ```text
//! E.ArenaLayout.Node.004
//! │ │           │    └── Sequence: 004 = OVERFLOW (WDP §6 convention)
//! │ │           └────── Primary: Node            (failure domain)
//! │ └──────────────── Component: ArenaLayout     (arena-based layout)
//! └────────────────── Severity: E                (error, blocking)
//! ```
//!
//! ## WDP Part References
//!
//! | Part | Spec | Our tokens |
//! |------|------|------------|
//! | 1 — Severity | `1-SEVERITY.md` | `E` (Error, blocking), `W` (Warning, non-blocking) |
//! | 2 — Component | `2-COMPONENT.md` | `Graph` (construction/validation), `ArenaLayout` (arena-based layout), `Arena` (arena allocator, reserved), `Layout` (heap/std layout, reserved) |
//! | 3 — Primary | `3-PRIMARY.md` | `Node`, `Edge`, `Dag`, `Subgraph`, `Alloc`, `Builder`, `Level`, `Plan`, `Canvas`, `Sink`, `Label` |
//! | 4 — Sequence | `4-SEQUENCE.md` + `6-SEQUENCE-CONVENTIONS.md` | See table below |
//!
//! ## Sequence Conventions Used (WDP §6 §4.1–§4.3)
//!
//! | Seq | Name | WDP meaning | Our usage |
//! |-----|------|-------------|-----------|
//! | 001 | MISSING | Required data not provided | `EmptyGraph` — no nodes in graph |
//! | 003 | INVALID | Validation check failed | `CycleDetected`, `SubgraphCycle` |
//! | 004 | OVERFLOW | Value too large | `ExceedsMaxNodes`, `ExceedsMaxLevels`, `ExceedsMaxExtent` |
//! | 021 | NOT_FOUND | Referenced element not found | `NodeNotFound`, `SubgraphNotFound` |
//! | 026 | EXHAUSTED | Resource exhausted | `ArenaOom`, `BuilderFailed` |
//! | 031 | INVISIBLE | Output element will not be rendered | `WARN_LABEL_INVISIBLE` (crate extension) |
//! | 033 | EXCESSIVE | Value kept but past its useful range | `WARN_CONFIG_EXCESSIVE` (crate extension) |
//!
//! All codes are composed from named macro building blocks at compile time via
//! `wdp!`. Any unrecognised token causes a compile error.

#[cfg(feature = "alloc")]
use alloc::boxed::Box;
use core::fmt;

// ── WDP code composition ────────────────────────────────────────────────
//
// Each axis is a macro that maps named tokens to string literals.
// `wdp!` threads them through `concat!` — zero-cost, zero-allocation.
//
// Adding a typo like `wdp!(E.Graph.Nod.003)` fails at compile time because
// `primary!(Nod)` has no matching arm.

/// Maps severity tokens to their WDP string (Part 1).
///
/// - `E` — Error: operation failed, needs attention (blocking)
/// - `W` — Warning: potential issue, operation continues (non-blocking)
macro_rules! severity {
    (E) => {
        "E"
    };
    (W) => {
        "W"
    };
}

/// Maps component tokens to their WDP string (Part 2).
///
/// - `Graph`       — graph construction and validation (allocation-agnostic)
/// - `ArenaLayout` — arena-based Sugiyama layout
/// - `Render`      — unified render engine (plan / canvas / sink)
/// - `Arena`       — arena allocator itself (reserved for future)
/// - `Layout`      — heap/std layout (reserved for future)
macro_rules! component {
    (Graph) => {
        "Graph"
    };
    (ArenaLayout) => {
        "ArenaLayout"
    };
    (Render) => {
        "Render"
    };
    (Arena) => {
        "Arena"
    };
    (Layout) => {
        "Layout"
    };
}

/// Maps primary tokens to their WDP string (Part 3).
///
/// Each primary represents a failure domain within its component:
///
/// **Under `Graph`:**
/// - `Node`     — node existence / data issues
/// - `Edge`     — edge existence / data issues
/// - `Dag`      — acyclicity / DAG constraint issues
/// - `Subgraph` — subgraph / cluster issues
///
/// **Under `ArenaLayout`:**
/// - `Alloc`   — arena allocator issues (OOM, capacity)
/// - `Builder` — IR builder issues
/// - `Node`    — node-count capacity issues (index type overflow)
/// - `Level`   — level-depth capacity issues
/// - `Extent`  — cross-axis extent past the coordinate type
///
/// **Under `Render`:**
/// - `Plan`   — render-plan build issues (caller plan buffer/arena)
/// - `Canvas` — band canvas issues (caller cell/color buffers)
/// - `Sink`   — output sink issues (caller byte buffer)
/// - `Workspace` — composer/planner workspace issues (caller chunk)
/// - `Label`  — edge-label placement diagnostics
macro_rules! primary {
    (Node) => {
        "Node"
    };
    (Edge) => {
        "Edge"
    };
    (Dag) => {
        "Dag"
    };
    (Subgraph) => {
        "Subgraph"
    };
    (Alloc) => {
        "Alloc"
    };
    (Builder) => {
        "Builder"
    };
    (Level) => {
        "Level"
    };
    (Extent) => {
        "Extent"
    };
    (Label) => {
        "Label"
    };
    (Plan) => {
        "Plan"
    };
    (Canvas) => {
        "Canvas"
    };
    (Workspace) => {
        "Workspace"
    };
    (Sink) => {
        "Sink"
    };
}

/// Maps sequence tokens to their WDP numeric code (Part 4).
///
/// These follow the conventions in `6-SEQUENCE-CONVENTIONS.md §4`:
///
/// | Seq | Name | Category | WDP meaning |
/// |-----|------|----------|-------------|
/// | 001 | MISSING | Input/Data | Required data not provided |
/// | 002 | MISMATCH | Input/Data | Type or length mismatch |
/// | 003 | INVALID | Input/Data | Validation check failed |
/// | 004 | OVERFLOW | Input/Data | Value too large / size exceeded |
/// | 007 | DUPLICATE | Input/Data | Duplicate entry |
/// | 009 | UNSUPPORTED | Input/Data | Feature not supported |
/// | 021 | NOT_FOUND | Resource | Referenced element not found |
/// | 026 | EXHAUSTED | Resource | Resource exhausted (OOM, capacity) |
/// | 031 | INVISIBLE | Output | Element will not be rendered (crate extension) |
/// | 032 | FAILED | Output | Downstream sink reported failure (crate extension) |
macro_rules! sequence {
    (MISSING) => {
        "001"
    };
    (MISMATCH) => {
        "002"
    };
    (INVALID) => {
        "003"
    };
    (OVERFLOW) => {
        "004"
    };
    (DUPLICATE) => {
        "007"
    };
    (UNSUPPORTED) => {
        "009"
    };
    (NOT_FOUND) => {
        "021"
    };
    (EXHAUSTED) => {
        "026"
    };
    (INVISIBLE) => {
        "031"
    };
    (FAILED) => {
        "032"
    };
    (EXCESSIVE) => {
        "033"
    };
}

/// Compose a WDP Level 0 error code from four named tokens.
///
/// Unrecognised tokens cause a compile error — no typo can slip through.
///
/// ```ignore
/// const CODE: &str = wdp!(E.Graph.Dag.INVALID);
/// assert_eq!(CODE, "E.Graph.Dag.003");
/// ```
macro_rules! wdp {
    ($sev:ident . $comp:ident . $pri:ident . $seq:ident) => {
        concat!(
            severity!($sev),
            ".",
            component!($comp),
            ".",
            primary!($pri),
            ".",
            sequence!($seq)
        )
    };
}

// ── Composed error codes ────────────────────────────────────────────────
//
// Each constant is unique — no two share the same Component.Primary.Sequence.
//
// Graph component:
//   Graph.Node.001      EmptyGraph        (MISSING)
//   Graph.Node.021      NodeNotFound      (NOT_FOUND)
//   Graph.Dag.003       CycleDetected     (INVALID)
//   Graph.Subgraph.003  SubgraphCycle     (INVALID)
//   Graph.Subgraph.021  SubgraphNotFound  (NOT_FOUND)
//
// ArenaLayout component:
//   ArenaLayout.Alloc.026     ArenaOom          (EXHAUSTED)
//   ArenaLayout.Builder.026   BuilderFailed     (EXHAUSTED)
//   ArenaLayout.Node.004      ExceedsMaxNodes   (OVERFLOW)
//   ArenaLayout.Level.004     ExceedsMaxLevels  (OVERFLOW)
//   ArenaLayout.Extent.004    ExceedsMaxExtent  (OVERFLOW)
//
// Render component:
//   Render.Plan.026    RenderPlanOom         (EXHAUSTED)
//   Render.Canvas.026  RenderCanvasTooSmall  (EXHAUSTED)
//   Render.Sink.026    RenderOutputTooSmall  (EXHAUSTED)
//   Render.Workspace.026  RenderWorkspaceTooSmall (EXHAUSTED)
//   Render.Sink.032    RenderSinkFailed      (FAILED)

/// Graph is empty — no nodes present.
///
/// `E.Graph.Node.001` — Sequence 001 = MISSING: "required data not provided."
pub const EMPTY_GRAPH: &str = wdp!(E.Graph.Node.MISSING);

/// Referenced node ID does not exist in the graph.
///
/// `E.Graph.Node.021` — Sequence 021 = NOT_FOUND: "referenced element not found."
pub const NODE_NOT_FOUND: &str = wdp!(E.Graph.Node.NOT_FOUND);

/// Cycle detected in a graph that requires acyclicity.
///
/// `E.Graph.Dag.003` — Sequence 003 = INVALID: "validation check failed"
/// (the DAG constraint is violated).
pub const CYCLE_DETECTED: &str = wdp!(E.Graph.Dag.INVALID);

/// Subgraph nesting would create a cycle in the hierarchy.
///
/// `E.Graph.Subgraph.003` — Sequence 003 = INVALID.
pub const SUBGRAPH_CYCLE: &str = wdp!(E.Graph.Subgraph.INVALID);

/// Referenced subgraph ID does not exist.
///
/// `E.Graph.Subgraph.021` — Sequence 021 = NOT_FOUND.
pub const SUBGRAPH_NOT_FOUND: &str = wdp!(E.Graph.Subgraph.NOT_FOUND);

/// Arena allocator ran out of memory.
///
/// `E.ArenaLayout.Alloc.026` — Sequence 026 = EXHAUSTED: "resource exhausted."
pub const ARENA_OOM: &str = wdp!(E.ArenaLayout.Alloc.EXHAUSTED);

/// IR builder failed to allocate output structures.
///
/// `E.ArenaLayout.Builder.026` — Sequence 026 = EXHAUSTED.
pub const BUILDER_FAILED: &str = wdp!(E.ArenaLayout.Builder.EXHAUSTED);

/// Node/edge count exceeds the index type's capacity.
///
/// `E.ArenaLayout.Node.004` — Sequence 004 = OVERFLOW: "value too large / size exceeded."
pub const EXCEEDS_MAX_NODES: &str = wdp!(E.ArenaLayout.Node.OVERFLOW);

/// Graph depth exceeds the maximum supported levels.
///
/// `E.ArenaLayout.Level.004` — Sequence 004 = OVERFLOW.
pub const EXCEEDS_MAX_LEVELS: &str = wdp!(E.ArenaLayout.Level.OVERFLOW);

/// The layout's cross-axis extent exceeds the arena coordinate type.
///
/// `E.ArenaLayout.Extent.004` — Sequence 004 = OVERFLOW.
pub const EXCEEDS_MAX_EXTENT: &str = wdp!(E.ArenaLayout.Extent.OVERFLOW);

/// Caller-provided render-plan buffer/arena is too small (no_std path).
///
/// `E.Render.Plan.026` — Sequence 026 = EXHAUSTED: "resource exhausted."
pub const RENDER_PLAN_OOM: &str = wdp!(E.Render.Plan.EXHAUSTED);

/// Caller-provided band buffer is smaller than `width × band_rows`.
///
/// `E.Render.Canvas.026` — Sequence 026 = EXHAUSTED.
pub const RENDER_CANVAS_TOO_SMALL: &str = wdp!(E.Render.Canvas.EXHAUSTED);

/// Caller-provided output byte buffer is too small for the render.
///
/// `E.Render.Sink.026` — Sequence 026 = EXHAUSTED.
pub const RENDER_OUTPUT_TOO_SMALL: &str = wdp!(E.Render.Sink.EXHAUSTED);

/// The caller-provided composer/planner workspace is too small.
pub const RENDER_WORKSPACE_TOO_SMALL: &str = wdp!(E.Render.Workspace.EXHAUSTED);

/// The caller's output sink reported a write failure.
pub const RENDER_SINK_FAILED: &str = wdp!(E.Render.Sink.FAILED);

// ── Warning codes (severity W — non-blocking, WDP Part 1) ───────────────
//
// Warnings share the error codes' Component.Primary.Sequence space:
//   Graph.Node.021  NodeAutoCreated  (NOT_FOUND — referenced, so created)
//   Graph.Node.007  AutoReplaced     (DUPLICATE — AUTO-involved replace)
//   Graph.Dag.003   ConfigClamped    (INVALID — config value out of range)

/// An edge referenced a node that was never added; a placeholder was
/// auto-created (rendered as `⟨⟩` until explicitly defined).
///
/// `W.Graph.Node.021` — Sequence 021 = NOT_FOUND, severity W =
/// non-blocking warning, emitted into the diagnostics channel under
/// the implicit missing-node policy.
pub const WARN_NODE_AUTO_CREATED: &str = wdp!(W.Graph.Node.NOT_FOUND);

/// A layout-configuration value was outside its valid range and was
/// CLAMPED (e.g. an absurd `crossing_reduction_passes`, likely a
/// negative value cast to `usize`).
///
/// `W.Graph.Dag.003` — Sequence 003 = INVALID. A condition code: the
/// setter records the note against the current configuration, and
/// every diagnostic-aware layout run reports it until the value is
/// fixed. (The 0.10 `W.Graph.Node.007` duplicate-replacement stderr
/// warning has no diagnostic successor — that point event is
/// delivered by the `NodeInsertion` receipt at the call site.)
pub const WARN_CONFIG_CLAMPED: &str = wdp!(W.Graph.Dag.INVALID);

/// A layout-configuration value is unreasonably high but was KEPT
/// (e.g. `crossing_reduction_passes` past diminishing returns) — a
/// distinct condition from a clamp, under a distinct code, so the
/// `(code, subject)` identity never collapses the two.
///
/// `W.Graph.Dag.033` — Sequence 033 = EXCESSIVE (crate extension,
/// like 031/032). A condition code, reported per run until fixed.
pub const WARN_CONFIG_EXCESSIVE: &str = wdp!(W.Graph.Dag.EXCESSIVE);

/// An edge label will not paint AND the legend is disabled (the
/// default) — the label appears nowhere in the output. With
/// `LabelOverflow::Legend` set, unplaced labels are listed below
/// the graph instead and this warning stays silent. The message names
/// the edge by index and endpoint ids only — label TEXT is caller
/// data (possibly secret, possibly control characters) and is never
/// written to stderr.
///
/// `W.Render.Label.031` — Sequence 031 = INVISIBLE (Ash's ruling: the
/// warning names the outcome the user must act on, not the cause —
/// 026 EXHAUSTED fit only the out-of-space case). Emitted into the
/// diagnostic context, once per affected label per PLAN BUILD: plans
/// are stateless, so planning the same layout again re-emits.
pub const WARN_LABEL_INVISIBLE: &str = wdp!(W.Render.Label.INVISIBLE);

// ── GraphError ──────────────────────────────────────────────────────────

/// Unified error type for all graph and layout operations.
///
/// Each variant carries a WDP error code accessible via [`GraphError::code()`],
/// and an actionable hint via [`GraphError::hint()`].
///
/// # WDP Code Mapping
///
/// | Variant | WDP Code | Meaning |
/// |---------|----------|---------|
/// | `EmptyGraph` | `E.Graph.Node.001` | MISSING — no nodes |
/// | `NodeNotFound` | `E.Graph.Node.021` | NOT_FOUND — node absent |
/// | `CycleDetected` | `E.Graph.Dag.003` | INVALID — DAG constraint violated |
/// | `SubgraphCycle` | `E.Graph.Subgraph.003` | INVALID — nesting cycle |
/// | `SubgraphNotFound` | `E.Graph.Subgraph.021` | NOT_FOUND — subgraph absent |
/// | `ArenaOom` | `E.ArenaLayout.Alloc.026` | EXHAUSTED — arena memory |
/// | `BuilderFailed` | `E.ArenaLayout.Builder.026` | EXHAUSTED — builder alloc |
/// | `ExceedsMaxNodes` | `E.ArenaLayout.Node.004` | OVERFLOW — index type full |
/// | `ExceedsMaxLevels` | `E.ArenaLayout.Level.004` | OVERFLOW — too deep |
/// | `ExceedsMaxExtent` | `E.ArenaLayout.Extent.004` | OVERFLOW — canvas past the coordinate type |
/// | `RenderPlanOom` | `E.Render.Plan.026` | EXHAUSTED — plan buffer |
/// | `RenderCanvasTooSmall` | `E.Render.Canvas.026` | EXHAUSTED — band buffer |
/// | `RenderOutputTooSmall` | `E.Render.Sink.026` | EXHAUSTED — output buffer |
/// | `RenderWorkspaceTooSmall` | `E.Render.Workspace.026` | EXHAUSTED — composer workspace |
/// | `RenderSinkFailed` | `E.Render.Sink.032` | FAILED — caller's writer errored |
///
/// # Examples
///
/// ```
/// use ascii_dag::GraphError;
///
/// let err = GraphError::EmptyGraph;
/// assert_eq!(err.code(), "E.Graph.Node.001");
/// assert!(!err.hint().is_empty());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum GraphError {
    /// The graph has no nodes.
    ///
    /// **WDP:** `E.Graph.Node.001` (MISSING)
    EmptyGraph,

    /// A referenced node ID does not exist in the graph.
    ///
    /// **WDP:** `E.Graph.Node.021` (NOT_FOUND)
    NodeNotFound(usize),

    /// The graph contains a cycle but acyclicity was required.
    ///
    /// **WDP:** `E.Graph.Dag.003` (INVALID)
    CycleDetected,

    /// Subgraph nesting would create a cycle in the hierarchy.
    ///
    /// **WDP:** `E.Graph.Subgraph.003` (INVALID)
    SubgraphCycle,

    /// A referenced subgraph ID does not exist.
    ///
    /// **WDP:** `E.Graph.Subgraph.021` (NOT_FOUND)
    SubgraphNotFound(usize),

    /// The arena allocator ran out of memory.
    ///
    /// **WDP:** `E.ArenaLayout.Alloc.026` (EXHAUSTED)
    ///
    /// Use `Graph::estimate_layout_arena_size()` to compute the required size.
    ArenaOom,

    /// The IR builder failed to allocate output structures.
    ///
    /// **WDP:** `E.ArenaLayout.Builder.026` (EXHAUSTED)
    BuilderFailed,

    /// The graph has more nodes or edges than the selected index type supports.
    ///
    /// **WDP:** `E.ArenaLayout.Node.004` (OVERFLOW)
    ExceedsMaxNodes {
        /// Actual node or edge count.
        count: usize,
        /// Maximum supported by the current index type.
        max: usize,
    },

    /// The graph's longest path exceeds the maximum level depth.
    ///
    /// **WDP:** `E.ArenaLayout.Level.004` (OVERFLOW)
    ExceedsMaxLevels {
        /// Actual depth of the longest path.
        depth: usize,
        /// Maximum supported levels.
        max: usize,
    },

    /// The layout's cross-axis extent — its packed nodes plus the cells
    /// port routing opens beside them — exceeds the arena coordinate
    /// type.
    ///
    /// **WDP:** `E.ArenaLayout.Extent.004` (OVERFLOW)
    ExceedsMaxExtent {
        /// The extent the layout needs.
        extent: usize,
        /// Maximum representable by the coordinate type.
        max: usize,
    },

    /// The caller-provided render-plan buffer is too small.
    ///
    /// **WDP:** `E.Render.Plan.026` (EXHAUSTED)
    ///
    /// Use `estimate_render_arena_size()` to compute the required size.
    RenderPlanOom,

    /// The caller-provided band buffer holds fewer than
    /// `width × band_rows` cells.
    ///
    /// **WDP:** `E.Render.Canvas.026` (EXHAUSTED)
    RenderCanvasTooSmall {
        /// Cells required (`width × band_rows`).
        needed: usize,
        /// Cells provided.
        got: usize,
    },

    /// The caller-provided output byte buffer filled up mid-render.
    ///
    /// **WDP:** `E.Render.Sink.026` (EXHAUSTED)
    RenderOutputTooSmall,

    /// A fixed composer workspace cannot hold this scene's
    /// composition. Units are BYTES — explicit in the field names so
    /// they can never be confused with the cell counts of
    /// [`RenderCanvasTooSmall`](Self::RenderCanvasTooSmall).
    ///
    /// **WDP:** `E.Render.Workspace.026` (EXHAUSTED)
    RenderWorkspaceTooSmall {
        /// Bytes the composition needs (`usize::MAX`: the requirement
        /// itself overflowed — the scene cannot fit any workspace).
        needed_bytes: usize,
        /// Bytes the workspace holds.
        got_bytes: usize,
    },

    /// The caller's `fmt::Write` sink reported an error mid-render.
    /// The failure originated downstream of the renderer (the writer
    /// itself); rendering state is unaffected and the render may be
    /// retried with a healthy sink. Previously a writer failure could
    /// only surface as a bare `fmt::Error` with no code.
    ///
    /// **WDP:** `E.Render.Sink.032` (FAILED)
    RenderSinkFailed,
}

impl GraphError {
    /// WDP Level 0 error code.
    ///
    /// Format: `Severity.Component.Primary.Sequence`
    ///
    /// Every variant maps to a unique code — no two variants share the same
    /// `Component.Primary.Sequence` triple.
    #[inline]
    pub fn code(&self) -> &'static str {
        match self {
            Self::EmptyGraph => EMPTY_GRAPH,
            Self::NodeNotFound(_) => NODE_NOT_FOUND,
            Self::CycleDetected => CYCLE_DETECTED,
            Self::SubgraphCycle => SUBGRAPH_CYCLE,
            Self::SubgraphNotFound(_) => SUBGRAPH_NOT_FOUND,
            Self::ArenaOom => ARENA_OOM,
            Self::BuilderFailed => BUILDER_FAILED,
            Self::ExceedsMaxNodes { .. } => EXCEEDS_MAX_NODES,
            Self::ExceedsMaxLevels { .. } => EXCEEDS_MAX_LEVELS,
            Self::ExceedsMaxExtent { .. } => EXCEEDS_MAX_EXTENT,
            Self::RenderPlanOom => RENDER_PLAN_OOM,
            Self::RenderCanvasTooSmall { .. } => RENDER_CANVAS_TOO_SMALL,
            Self::RenderOutputTooSmall => RENDER_OUTPUT_TOO_SMALL,
            Self::RenderWorkspaceTooSmall { .. } => RENDER_WORKSPACE_TOO_SMALL,
            Self::RenderSinkFailed => RENDER_SINK_FAILED,
        }
    }

    /// Actionable hint for resolving this error.
    #[inline]
    pub fn hint(&self) -> &'static str {
        match self {
            Self::EmptyGraph => "Add at least one node before computing layout",
            Self::NodeNotFound(_) => "Call add_node() before referencing this node ID",
            Self::CycleDetected => "Enable cycle breaking or remove the back edge",
            Self::SubgraphCycle => {
                "A subgraph cannot be nested inside itself or its own descendant"
            }
            Self::SubgraphNotFound(_) => "Call add_subgraph() before referencing this subgraph ID",
            Self::ArenaOom => "Increase the arena buffer size; use estimate_layout_arena_size()",
            Self::BuilderFailed => "Increase the output arena buffer size",
            Self::ExceedsMaxNodes { .. } => {
                "Use a larger index type (arena-idx-u32) or reduce graph size"
            }
            Self::ExceedsMaxLevels { .. } => {
                "Reduce chain depth or use a different layout strategy"
            }
            Self::ExceedsMaxExtent { .. } => {
                "Narrow the widest level (node widths, spacing, side ports) or use the heap layout"
            }
            Self::RenderPlanOom => "Increase the render arena; use estimate_render_arena_size()",
            Self::RenderCanvasTooSmall { .. } => {
                "Provide at least width × band_rows cells; lower band_rows_cap to shrink bands"
            }
            Self::RenderOutputTooSmall => {
                "Provide a larger output buffer; plan width × height plus escapes when colored"
            }
            Self::RenderWorkspaceTooSmall { .. } => {
                "Size the workspace with CompositionRequirements::workspace_bytes(); \
                 lower band_rows_cap to shrink it"
            }
            Self::RenderSinkFailed => {
                "The failure came from the caller's writer, not the renderer; \
                 check the sink and retry"
            }
        }
    }
}

impl fmt::Display for GraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] ", self.code())?;
        match self {
            Self::EmptyGraph => write!(f, "graph has no nodes"),
            Self::NodeNotFound(id) => write!(f, "node {} not found", id),
            Self::CycleDetected => write!(f, "cycle detected in graph"),
            Self::SubgraphCycle => write!(f, "subgraph nesting would create a cycle"),
            Self::SubgraphNotFound(id) => write!(f, "subgraph {} not found", id),
            Self::ArenaOom => write!(f, "arena out of memory"),
            Self::BuilderFailed => write!(f, "IR builder allocation failed"),
            Self::ExceedsMaxNodes { count, max } => {
                write!(
                    f,
                    "node/edge count {} exceeds index-type max {}",
                    count, max
                )
            }
            Self::ExceedsMaxLevels { depth, max } => {
                write!(f, "graph depth {} exceeds max levels {}", depth, max)
            }
            Self::ExceedsMaxExtent { extent, max } => {
                write!(f, "layout extent {} exceeds coordinate max {}", extent, max)
            }
            Self::RenderPlanOom => write!(f, "render plan buffer exhausted"),
            Self::RenderCanvasTooSmall { needed, got } => {
                write!(f, "band buffer holds {} cells, needs {}", got, needed)
            }
            Self::RenderOutputTooSmall => write!(f, "render output buffer exhausted"),
            Self::RenderWorkspaceTooSmall {
                needed_bytes,
                got_bytes,
            } => {
                write!(
                    f,
                    "composer workspace holds {} bytes, needs {}",
                    got_bytes, needed_bytes
                )
            }
            Self::RenderSinkFailed => write!(f, "output sink reported a write failure"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for GraphError {}

// ── ErrorChain (error + causal chain) ───────────────────────────────────

#[cfg(feature = "alloc")]
/// A diagnostic pairs a [`GraphError`] with an optional causal chain.
///
/// Both the outer error and every inner cause carry their own WDP code,
/// forming a chain that provides context (what failed) at each level:
///
/// ```text
/// [E.ArenaLayout.Builder.026] IR builder allocation failed
///   caused by: [E.ArenaLayout.Alloc.026] arena out of memory
/// ```
///
/// # Construction
///
/// ```
/// use ascii_dag::errors::{GraphError, ErrorChain};
///
/// // Leaf diagnostic (no cause)
/// let d = ErrorChain::from(GraphError::ArenaOom);
/// assert_eq!(d.code(), "E.ArenaLayout.Alloc.026");
/// assert!(d.cause().is_none());
///
/// // Chained diagnostic
/// let d = ErrorChain::from(GraphError::BuilderFailed)
///     .caused_by(GraphError::ArenaOom);
/// assert_eq!(d.code(), "E.ArenaLayout.Builder.026");
/// assert_eq!(d.cause().unwrap().code(), "E.ArenaLayout.Alloc.026");
/// ```
///
/// # Display
///
/// `Display` renders the full chain, indenting each cause level:
///
/// ```
/// use ascii_dag::errors::{GraphError, ErrorChain};
///
/// let d = ErrorChain::from(GraphError::BuilderFailed)
///     .caused_by(GraphError::ArenaOom);
/// let msg = d.to_string();
/// assert!(msg.contains("caused by:"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorChain {
    /// The error at this level.
    error: GraphError,
    /// Optional inner cause (each cause is itself a full `ErrorChain`).
    cause: Option<Box<ErrorChain>>,
}

#[cfg(feature = "alloc")]
impl ErrorChain {
    /// Create a leaf diagnostic (no cause).
    #[inline]
    pub fn new(error: GraphError) -> Self {
        Self { error, cause: None }
    }

    /// Attach a cause to this diagnostic (builder pattern).
    ///
    /// The cause is wrapped in its own `ErrorChain` with no further inner
    /// cause.  For deeper chains, use [`caused_by_chain`](Self::caused_by_chain).
    #[inline]
    pub fn caused_by(self, cause: GraphError) -> Self {
        self.caused_by_chain(ErrorChain::new(cause))
    }

    /// Attach an existing `ErrorChain` as the cause.
    #[inline]
    pub fn caused_by_chain(mut self, cause: ErrorChain) -> Self {
        self.cause = Some(Box::new(cause));
        self
    }

    /// The [`GraphError`] at this level of the chain.
    #[inline]
    pub fn error(&self) -> &GraphError {
        &self.error
    }

    /// WDP code of the outermost error.
    #[inline]
    pub fn code(&self) -> &'static str {
        self.error.code()
    }

    /// Hint for the outermost error.
    #[inline]
    pub fn hint(&self) -> &'static str {
        self.error.hint()
    }

    /// The inner cause, if any.
    #[inline]
    pub fn cause(&self) -> Option<&ErrorChain> {
        self.cause.as_deref()
    }

    /// Iterate over the full chain: self, then cause, then cause's cause, etc.
    pub fn chain(&self) -> DiagnosticChain<'_> {
        DiagnosticChain {
            current: Some(self),
        }
    }
}

#[cfg(feature = "alloc")]
impl From<GraphError> for ErrorChain {
    #[inline]
    fn from(error: GraphError) -> Self {
        Self::new(error)
    }
}

#[cfg(feature = "alloc")]
impl fmt::Display for ErrorChain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Outer error
        fmt::Display::fmt(&self.error, f)?;

        // Walk the cause chain, indenting each level
        let mut depth = 1;
        let mut current = self.cause.as_deref();
        while let Some(diag) = current {
            writeln!(f)?;
            for _ in 0..depth {
                write!(f, "  ")?;
            }
            write!(f, "caused by: {}", diag.error)?;
            current = diag.cause.as_deref();
            depth += 1;
        }
        Ok(())
    }
}

#[cfg(all(feature = "alloc", feature = "std"))]
impl std::error::Error for ErrorChain {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.cause.as_deref().map(|d| d as &dyn std::error::Error)
    }
}

/// Iterator over the [`ErrorChain`] chain (outer → innermost cause).
#[cfg(feature = "alloc")]
pub struct DiagnosticChain<'a> {
    current: Option<&'a ErrorChain>,
}

#[cfg(feature = "alloc")]
impl<'a> Iterator for DiagnosticChain<'a> {
    type Item = &'a ErrorChain;

    fn next(&mut self) -> Option<Self::Item> {
        let diag = self.current?;
        self.current = diag.cause.as_deref();
        Some(diag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Render component's codes compose exactly as documented.
    #[test]
    fn render_component_codes() {
        assert_eq!(GraphError::RenderPlanOom.code(), "E.Render.Plan.026");
        assert_eq!(
            GraphError::RenderCanvasTooSmall { needed: 0, got: 0 }.code(),
            "E.Render.Canvas.026"
        );
        assert_eq!(GraphError::RenderOutputTooSmall.code(), "E.Render.Sink.026");
        for e in [
            GraphError::RenderPlanOom,
            GraphError::RenderCanvasTooSmall { needed: 8, got: 4 },
            GraphError::RenderOutputTooSmall,
        ] {
            assert!(
                !e.hint().is_empty(),
                "{}: hint must be actionable",
                e.code()
            );
        }
    }
}
