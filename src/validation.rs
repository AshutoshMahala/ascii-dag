//! Graph validation and requirements.
//!
//! [`Requirements`] describes the constraints that a graph must satisfy before
//! a particular algorithm can run. Validation is checked at layout time rather
//! than construction, following zigraph's "validate at use" principle.
//!
//! # Examples
//!
//! ```
//! use ascii_dag::Requirements;
//!
//! // Sugiyama layout requires an acyclic graph
//! let req = Requirements::sugiyama();
//! assert!(req.acyclic);
//!
//! // Permissive requirements allow cycles
//! let req = Requirements::permissive();
//! assert!(!req.acyclic);
//! ```

/// Constraints that a graph must satisfy before layout.
///
/// Use the provided presets or build a custom set:
///
/// ```
/// use ascii_dag::Requirements;
///
/// let custom = Requirements {
///     acyclic: true,
///     non_empty: false,
///     ..Requirements::permissive()
/// };
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Requirements {
    /// Graph must not contain directed cycles.
    pub acyclic: bool,
    /// Graph must have at least one node.
    pub non_empty: bool,
}

impl Requirements {
    /// Strict DAG requirements: acyclic and non-empty.
    ///
    /// Use this for algorithms that require a proper DAG input.
    #[inline]
    pub const fn dag() -> Self {
        Self {
            acyclic: true,
            non_empty: true,
        }
    }

    /// Sugiyama layout requirements: acyclic (cycles are broken first).
    ///
    /// This is the standard preset for `compute_layout()`.
    #[inline]
    pub const fn sugiyama() -> Self {
        Self {
            acyclic: true,
            non_empty: true,
        }
    }

    /// No constraints — all graphs accepted.
    ///
    /// Useful for inspection/traversal code that works on any graph.
    #[inline]
    pub const fn permissive() -> Self {
        Self {
            acyclic: false,
            non_empty: false,
        }
    }
}

impl Default for Requirements {
    /// Defaults to [`Requirements::sugiyama()`].
    fn default() -> Self {
        Self::sugiyama()
    }
}
