//! Error types for arena-based layout computation.
//!
//! The arena layout path returns `Result<LayoutIRArena, LayoutError>` instead of
//! `Option<LayoutIRArena>`, giving callers actionable diagnostics when layout fails.

use core::fmt;

/// Errors that can occur during arena-based layout computation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutError {
    /// The arena ran out of memory during allocation.
    ///
    /// **Fix:** Increase the arena buffer size. Use `DAG::estimate_layout_arena_size()`
    /// to compute a safe size with a 2× safety margin.
    ArenaOom,

    /// The graph has more nodes or edges than the selected index type supports.
    ///
    /// The `arena-idx-u8` feature limits graphs to 255 nodes/edges.
    /// Use `--features arena` (u32, default) for larger graphs.
    ExceedsMaxNodes {
        /// Actual node or edge count.
        count: usize,
        /// Maximum supported by the current index type.
        max: usize,
    },

    /// The graph's longest path exceeds the maximum level depth (255).
    ///
    /// This is a hard limit of the Sugiyama algorithm implementation.
    /// Consider reducing chain depth or using a different layout strategy.
    ExceedsMaxLevels {
        /// Actual depth of the longest path.
        depth: usize,
        /// Maximum supported levels.
        max: usize,
    },

    /// The graph contains a cycle and cannot be laid out as a DAG.
    ///
    /// Remove cycles or use cycle-breaking before calling layout.
    CycleDetected,

    /// The IR builder failed to allocate output structures.
    ///
    /// Usually means the output arena is too small.
    BuilderFailed,
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LayoutError::ArenaOom => write!(f, "arena out of memory"),
            LayoutError::ExceedsMaxNodes { count, max } => {
                write!(
                    f,
                    "graph has {} nodes/edges but index type supports max {}",
                    count, max
                )
            }
            LayoutError::ExceedsMaxLevels { depth, max } => {
                write!(
                    f,
                    "graph depth {} exceeds max levels {}",
                    depth, max
                )
            }
            LayoutError::CycleDetected => write!(f, "cycle detected in graph"),
            LayoutError::BuilderFailed => write!(f, "IR builder allocation failed"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for LayoutError {}
