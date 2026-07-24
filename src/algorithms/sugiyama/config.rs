//! Layout configuration types.
//!
//! [`LayoutConfig`] is the primary, no-alloc-compatible way to control
//! the layout pipeline.  It uses an enum-of-structs pattern: shared
//! settings (spacing, render mode) live on the outer struct, while
//! algorithm-specific fields are scoped inside [`AlgorithmConfig`]
//! variants.
//!
//! [`SugiyamaConfig`] is the legacy alloc-dependent type, deprecated
//! in favour of `LayoutConfig`.
//!
//! # Examples
//!
//! ```
//! use ascii_dag::algorithms::sugiyama::config::LayoutConfig;
//!
//! // Use a preset (const fn, zero allocation)
//! let cfg = LayoutConfig::standard();
//!
//! // Or the quality preset
//! let cfg = LayoutConfig::quality();
//! ```

use super::crossing::{CrossingReducer, FAST, QUALITY, STANDARD};
use crate::graph::{Direction, RenderMode};

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

// ── Pipeline stage enums (unconditional — no alloc needed) ───────────────

/// Cycle-breaking strategy.
///
/// Determines how cycles in the input graph are handled before layering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CycleBreaking {
    /// Reject cyclic graphs with an error.
    ///
    /// Use this when you know the input is a DAG and want to catch
    /// accidental cycles early.  This is the default for the arena
    /// layout path.
    None,

    /// Break cycles via DFS back-edge detection.
    ///
    /// Back edges are temporarily reversed for layering and routing,
    /// then marked `reversed: true` in the output IR.  This is the
    /// default for the heap layout path.
    DepthFirst,
}

/// Layering (level/rank assignment) algorithm.
///
/// Controls how nodes are assigned to horizontal levels in the hierarchy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layering {
    /// Longest-path layering (default).
    ///
    /// Fast O(N+E) fixed-point algorithm.  Assigns each node a level
    /// equal to 1 + max(parent levels).
    LongestPath,
    // Future: NetworkSimplex — minimises total edge span
}

/// Positioning (x-coordinate assignment) algorithm.
///
/// Controls how nodes are placed horizontally within their level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Positioning {
    /// Compact left-to-right packing (default).
    ///
    /// Places nodes sequentially with fixed spacing.  Fast and
    /// collision-free.
    Compact,
    // Future: Barycentric — centre nodes relative to neighbours
    // Future: BrandesKopf — multi-pass iterative centering
}

/// Edge routing algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Routing {
    /// Direct Manhattan routing (straight lines with corners).
    Direct,
    // Future: Spline, ...
}

// ── AlgorithmConfig (enum-of-structs) ────────────────────────────────────

/// Algorithm-specific configuration.
///
/// Each variant carries only the fields relevant to that algorithm.
/// Shared settings (spacing, render mode, etc.) live on [`LayoutConfig`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlgorithmConfig<'a> {
    /// Sugiyama hierarchical layout.
    Sugiyama {
        /// Cycle-breaking strategy.
        cycle_breaking: CycleBreaking,
        /// Layering algorithm.
        layering: Layering,
        /// Crossing reduction pipeline — applied in order.
        crossing_pipeline: &'a [CrossingReducer],
        /// Positioning algorithm.
        positioning: Positioning,
    },
    // Future:
    // ForceDirected {
    //     iterations: usize,
    //     spring_constant: f32,
    //     repulsion: f32,
    //     damping: f32,
    // },
}

// ── LayoutConfig ─────────────────────────────────────────────────────────

/// Layout configuration — the primary way to control the layout pipeline.
///
/// Uses borrowed slices and plain `Copy` types, enabling compile-time
/// construction from static presets.  Compatible with no-alloc / `no_std`.
///
/// # Presets
///
/// | Preset | Algorithm | Crossing | Spacing (node×level) |
/// |--------|-----------|----------|----------------------|
/// | [`fast()`](Self::fast) | Sugiyama | Median(2) | 3×0 |
/// | [`standard()`](Self::standard) | Sugiyama | Median(4)→AdjExch(2) | 3×0 |
/// | [`quality()`](Self::quality) | Sugiyama | Median(8)→AdjExch(4)→Median(2) | 3×0 |
///
/// # Examples
///
/// ```
/// use ascii_dag::algorithms::sugiyama::config::LayoutConfig;
///
/// // Static preset (const, zero allocation)
/// let cfg = LayoutConfig::standard();
/// ```
#[derive(Debug, Clone, Copy)]
pub struct LayoutConfig<'a> {
    /// Algorithm-specific settings (Sugiyama, future FDG, etc.).
    pub algorithm: AlgorithmConfig<'a>,

    /// Edge routing algorithm.
    pub routing: Routing,

    /// Horizontal spacing between adjacent nodes within a level (default: 3).
    pub node_spacing: usize,

    /// Vertical spacing between levels (default: 0).
    ///
    /// The default of 0 preserves the rendered output of releases prior to
    /// 0.10.0, which advertised `level_spacing` in the config but never
    /// applied it. Set to a positive value (e.g. 1–3) to add visible
    /// vertical separation between layers.
    pub level_spacing: usize,

    /// Rendering mode (Auto, Vertical, Horizontal).
    pub render_mode: RenderMode,

    /// Rank direction (default: [`Direction::TopDown`]).
    ///
    /// Recorded on the layout IR; for `BottomUp` the heap layout path
    /// flips its result into physical coordinates. The built-in
    /// renderers currently paint `TopDown` layouts only.
    pub direction: Direction,

    /// Include dummy (virtual) nodes in the output IR (default: `false`).
    ///
    /// Skip-level edges route through dummy nodes at each intermediate
    /// level. When enabled, those appear as `LayoutNode`s with
    /// `kind == NodeKind::Dummy`, an `edge_index` back-link to their
    /// owning edge, a synthetic id (excluded from `node_by_id`), and
    /// width 1 at the drawn waypoint column. Zero cost when disabled.
    ///
    /// Notes: the built-in text renderers paint an included dummy like
    /// any node (an empty `[]` marker) — intended for debugging and IR
    /// consumers. Arena users must budget the output arena for the
    /// extra nodes (one per skip-level waypoint).
    pub include_dummy_nodes: bool,

    /// Skip graph validation for performance.
    pub skip_validation: bool,
}

impl<'a> LayoutConfig<'a> {
    /// Fast preset — minimal crossing reduction, maximum speed.
    pub const fn fast() -> Self {
        Self {
            algorithm: AlgorithmConfig::Sugiyama {
                cycle_breaking: CycleBreaking::DepthFirst,
                layering: Layering::LongestPath,
                crossing_pipeline: FAST,
                positioning: Positioning::Compact,
            },
            routing: Routing::Direct,
            node_spacing: 3,
            level_spacing: 0,
            render_mode: RenderMode::Auto,
            direction: Direction::TopDown,
            include_dummy_nodes: false,
            skip_validation: false,
        }
    }

    /// Standard preset — good balance of quality and speed.
    pub const fn standard() -> Self {
        Self {
            algorithm: AlgorithmConfig::Sugiyama {
                cycle_breaking: CycleBreaking::DepthFirst,
                layering: Layering::LongestPath,
                crossing_pipeline: STANDARD,
                positioning: Positioning::Compact,
            },
            routing: Routing::Direct,
            node_spacing: 3,
            level_spacing: 0,
            render_mode: RenderMode::Auto,
            direction: Direction::TopDown,
            include_dummy_nodes: false,
            skip_validation: false,
        }
    }

    /// Quality preset — thorough reduction for complex graphs.
    pub const fn quality() -> Self {
        Self {
            algorithm: AlgorithmConfig::Sugiyama {
                cycle_breaking: CycleBreaking::DepthFirst,
                layering: Layering::LongestPath,
                crossing_pipeline: QUALITY,
                positioning: Positioning::Compact,
            },
            routing: Routing::Direct,
            node_spacing: 3,
            level_spacing: 0,
            render_mode: RenderMode::Auto,
            direction: Direction::TopDown,
            include_dummy_nodes: false,
            skip_validation: false,
        }
    }

    /// Extract the crossing pipeline (returns empty slice for non-Sugiyama).
    pub const fn crossing_pipeline(&self) -> &'a [CrossingReducer] {
        match &self.algorithm {
            AlgorithmConfig::Sugiyama {
                crossing_pipeline, ..
            } => crossing_pipeline,
        }
    }

    /// Extract the cycle-breaking strategy.
    pub const fn cycle_breaking(&self) -> CycleBreaking {
        match &self.algorithm {
            AlgorithmConfig::Sugiyama { cycle_breaking, .. } => *cycle_breaking,
        }
    }
}

impl Default for LayoutConfig<'static> {
    fn default() -> Self {
        Self::standard()
    }
}

// ── SugiyamaConfig (legacy, alloc-dependent) ─────────────────────────────

/// **Deprecated**: Use [`LayoutConfig`] instead.
///
/// Configuration for the Sugiyama hierarchical layout algorithm.
/// This type requires alloc (`Vec<CrossingReducer>`).  The new
/// [`LayoutConfig`] is no-alloc and supports enum-of-structs for
/// future algorithm variants.
///
/// # Presets
///
/// | Preset | Cycle Breaking | Layering | Crossing | Positioning |
/// |--------|---------------|----------|----------|------------|
/// | [`fast()`](Self::fast) | DepthFirst | LongestPath | Median(2) | Compact |
/// | [`standard()`](Self::standard) | DepthFirst | LongestPath | Median(4)→AdjExch(2) | Compact |
/// | [`quality()`](Self::quality) | DepthFirst | LongestPath | Median(8)→AdjExch(4)→Median(2) | Compact |
#[cfg(feature = "alloc")]
#[deprecated(since = "0.9.0", note = "use LayoutConfig instead")]
#[derive(Debug, Clone)]
pub struct SugiyamaConfig {
    /// Cycle-breaking strategy.
    pub cycle_breaking: CycleBreaking,

    /// Layering algorithm.
    pub layering: Layering,

    /// Crossing reduction pipeline — applied in order.
    pub crossing_pipeline: Vec<CrossingReducer>,

    /// Positioning algorithm.
    pub positioning: Positioning,

    /// Rendering mode (Auto, Vertical, Horizontal).
    pub render_mode: RenderMode,
}

#[cfg(feature = "alloc")]
#[allow(deprecated)]
impl SugiyamaConfig {
    /// Fast preset — minimal crossing reduction, maximum speed.
    pub fn fast() -> Self {
        Self {
            cycle_breaking: CycleBreaking::DepthFirst,
            layering: Layering::LongestPath,
            crossing_pipeline: FAST.to_vec(),
            positioning: Positioning::Compact,
            render_mode: RenderMode::default(),
        }
    }

    /// Standard preset — good balance of quality and speed.
    pub fn standard() -> Self {
        Self {
            cycle_breaking: CycleBreaking::DepthFirst,
            layering: Layering::LongestPath,
            crossing_pipeline: STANDARD.to_vec(),
            positioning: Positioning::Compact,
            render_mode: RenderMode::default(),
        }
    }

    /// Quality preset — thorough reduction for complex graphs.
    pub fn quality() -> Self {
        Self {
            cycle_breaking: CycleBreaking::DepthFirst,
            layering: Layering::LongestPath,
            crossing_pipeline: QUALITY.to_vec(),
            positioning: Positioning::Compact,
            render_mode: RenderMode::default(),
        }
    }

    /// Create a config from a custom crossing pipeline (convenience).
    ///
    /// Sets all other stages to standard defaults.
    pub fn with_crossing(pipeline: &[CrossingReducer]) -> Self {
        Self {
            cycle_breaking: CycleBreaking::DepthFirst,
            layering: Layering::LongestPath,
            crossing_pipeline: pipeline.to_vec(),
            positioning: Positioning::Compact,
            render_mode: RenderMode::default(),
        }
    }
}

#[cfg(feature = "alloc")]
#[allow(deprecated)]
impl Default for SugiyamaConfig {
    fn default() -> Self {
        Self::standard()
    }
}

// ── Conversions: SugiyamaConfig ↔ LayoutConfig ──────────────────────────

/// Convert a borrowed `SugiyamaConfig` into a `LayoutConfig`.
///
/// The crossing pipeline borrows from the `SugiyamaConfig`'s `Vec`,
/// so the returned `LayoutConfig` has lifetime tied to the source.
#[cfg(feature = "alloc")]
#[allow(deprecated)]
impl<'a> From<&'a SugiyamaConfig> for LayoutConfig<'a> {
    fn from(sc: &'a SugiyamaConfig) -> Self {
        LayoutConfig {
            algorithm: AlgorithmConfig::Sugiyama {
                cycle_breaking: sc.cycle_breaking,
                layering: sc.layering,
                crossing_pipeline: &sc.crossing_pipeline,
                positioning: sc.positioning,
            },
            routing: Routing::Direct,
            node_spacing: 3,
            level_spacing: 0,
            render_mode: sc.render_mode,
            direction: Direction::TopDown,
            include_dummy_nodes: false,
            skip_validation: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_config_presets_are_const() {
        // These are const fn — should compile at const time
        const _FAST: LayoutConfig<'static> = LayoutConfig::fast();
        const _STD: LayoutConfig<'static> = LayoutConfig::standard();
        const _QUAL: LayoutConfig<'static> = LayoutConfig::quality();
    }

    #[test]
    fn layout_config_default_is_standard() {
        let def = LayoutConfig::default();
        let std = LayoutConfig::standard();
        assert_eq!(def.crossing_pipeline(), std.crossing_pipeline());
        assert_eq!(def.node_spacing, std.node_spacing);
    }

    #[test]
    fn layout_config_accessors_work() {
        let cfg = LayoutConfig::quality();
        assert_eq!(cfg.cycle_breaking(), CycleBreaking::DepthFirst);
        assert_eq!(cfg.crossing_pipeline().len(), 3);
        assert_eq!(cfg.node_spacing, 3);
        assert_eq!(cfg.level_spacing, 0);
    }
}

#[cfg(test)]
#[cfg(feature = "alloc")]
mod alloc_tests {
    use super::*;

    #[allow(deprecated)]
    #[test]
    fn sugiyama_config_preset_defaults_are_consistent() {
        let std = SugiyamaConfig::standard();
        let def = SugiyamaConfig::default();
        assert_eq!(std.cycle_breaking, def.cycle_breaking);
        assert_eq!(std.layering, def.layering);
        assert_eq!(std.positioning, def.positioning);
        assert_eq!(std.crossing_pipeline, def.crossing_pipeline);
    }

    #[allow(deprecated)]
    #[test]
    fn sugiyama_config_converts_to_layout_config() {
        let sc = SugiyamaConfig::quality();
        let lc = LayoutConfig::from(&sc);
        assert_eq!(lc.crossing_pipeline().len(), 3);
        assert_eq!(lc.cycle_breaking(), CycleBreaking::DepthFirst);
        assert_eq!(lc.render_mode, sc.render_mode);
    }

    #[allow(deprecated)]
    #[test]
    fn sugiyama_config_with_crossing_works() {
        let cfg = SugiyamaConfig::with_crossing(&[CrossingReducer::Median(10)]);
        assert_eq!(cfg.crossing_pipeline.len(), 1);
    }
}
