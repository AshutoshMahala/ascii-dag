//! Sugiyama layout configuration.
//!
//! [`SugiyamaConfig`] bundles all algorithm choices for the hierarchical
//! (Sugiyama) layout pipeline.  Use the [`fast()`], [`standard()`], or
//! [`quality()`] presets for common scenarios, or build a custom config
//! for full control.
//!
//! # Examples
//!
//! ```
//! use ascii_dag::SugiyamaConfig;
//! use ascii_dag::algorithms::sugiyama::crossing::CrossingReducer;
//!
//! // Use a preset
//! let cfg = SugiyamaConfig::quality();
//!
//! // Or build a custom crossing pipeline
//! let cfg = SugiyamaConfig::with_crossing(
//!     &[CrossingReducer::Median(6), CrossingReducer::AdjacentExchange(2)],
//! );
//! ```

use alloc::vec::Vec;
use super::crossing::{CrossingReducer, FAST, STANDARD, QUALITY};
use crate::graph::RenderMode;

// ── Pipeline stage enums ─────────────────────────────────────────────────

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

// ── SugiyamaConfig ───────────────────────────────────────────────────────

/// Configuration for the Sugiyama hierarchical layout algorithm.
///
/// Bundles all pipeline-stage selections into a single value that can
/// be passed to [`Graph::compute_layout_with`](crate::Graph::compute_layout_with).
///
/// # Presets
///
/// | Preset | Cycle Breaking | Layering | Crossing | Positioning |
/// |--------|---------------|----------|----------|------------|
/// | [`fast()`](Self::fast) | DepthFirst | LongestPath | Median(2) | Compact |
/// | [`standard()`](Self::standard) | DepthFirst | LongestPath | Median(4)→AdjExch(2) | Compact |
/// | [`quality()`](Self::quality) | DepthFirst | LongestPath | Median(8)→AdjExch(4)→Median(2) | Compact |
///
/// # Custom Configuration
///
/// ```
/// use ascii_dag::SugiyamaConfig;
/// use ascii_dag::algorithms::sugiyama::config::{CycleBreaking, Layering, Positioning};
/// use ascii_dag::algorithms::sugiyama::crossing::CrossingReducer;
///
/// let cfg = SugiyamaConfig {
///     cycle_breaking: CycleBreaking::DepthFirst,
///     layering: Layering::LongestPath,
///     crossing_pipeline: vec![
///         CrossingReducer::Median(6),
///         CrossingReducer::AdjacentExchange(2),
///     ],
///     positioning: Positioning::Compact,
///     render_mode: ascii_dag::RenderMode::Auto,
/// };
/// ```
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

impl Default for SugiyamaConfig {
    fn default() -> Self {
        Self::standard()
    }
}

// ── Backward compatibility: LayoutConfig → SugiyamaConfig ────────────────

/// **Deprecated**: Use [`SugiyamaConfig`] instead.
///
/// This type alias preserves backward compatibility for code using the
/// old `LayoutConfig` name.
#[deprecated(since = "0.9.0", note = "renamed to SugiyamaConfig")]
pub type LayoutConfig = SugiyamaConfig;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_defaults_are_consistent() {
        let std = SugiyamaConfig::standard();
        let def = SugiyamaConfig::default();
        assert_eq!(std.cycle_breaking, def.cycle_breaking);
        assert_eq!(std.layering, def.layering);
        assert_eq!(std.positioning, def.positioning);
        assert_eq!(std.crossing_pipeline, def.crossing_pipeline);
    }

    #[test]
    fn fast_has_fewer_passes_than_quality() {
        let fast = SugiyamaConfig::fast();
        let quality = SugiyamaConfig::quality();

        let fast_total: usize = fast.crossing_pipeline.iter().map(|r| match r {
            CrossingReducer::Median(n) | CrossingReducer::AdjacentExchange(n) => *n,
        }).sum();
        let quality_total: usize = quality.crossing_pipeline.iter().map(|r| match r {
            CrossingReducer::Median(n) | CrossingReducer::AdjacentExchange(n) => *n,
        }).sum();

        assert!(fast_total < quality_total);
    }

    #[test]
    fn with_crossing_uses_standard_defaults() {
        let cfg = SugiyamaConfig::with_crossing(&[CrossingReducer::Median(10)]);
        assert_eq!(cfg.cycle_breaking, CycleBreaking::DepthFirst);
        assert_eq!(cfg.layering, Layering::LongestPath);
        assert_eq!(cfg.positioning, Positioning::Compact);
        assert_eq!(cfg.crossing_pipeline.len(), 1);
    }
}
