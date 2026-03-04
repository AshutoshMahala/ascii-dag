//! Composable crossing reduction pipeline.
//!
//! Provides configurable strategies for minimising edge crossings in
//! hierarchical (Sugiyama-style) layouts.  Inspired by zigraph's
//! composable pipeline design.
//!
//! # Overview
//!
//! A [`CrossingReducer`] is a single strategy (e.g. median heuristic or
//! adjacent-exchange refinement).  A *pipeline* is a slice of reducers
//! that are applied in sequence — each one refines the ordering left by
//! the previous one.
//!
//! Three pre-built pipelines are shipped as constants:
//!
//! | Preset      | Pipeline                                         |
//! |-------------|--------------------------------------------------|
//! | [`FAST`]    | `Median(2)`                                      |
//! | [`STANDARD`]| `Median(4) → AdjacentExchange(2)`                |
//! | [`QUALITY`] | `Median(8) → AdjacentExchange(4) → Median(2)`   |

/// A single crossing reduction strategy.
///
/// The parameter is the number of full (top-down **+** bottom-up) passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrossingReducer {
    /// Median heuristic: reorder nodes by median position of their
    /// neighbours in the adjacent level.
    Median(usize),

    /// Adjacent exchange: greedily swap neighbouring node pairs on a
    /// level whenever the swap reduces the number of edge crossings.
    AdjacentExchange(usize),
}

// ── Preset pipelines ─────────────────────────────────────────────────────

/// Fast preset — 2 passes of median only.
///
/// Good for simple or time-critical graphs where layout quality is
/// secondary to speed.
pub const FAST: &[CrossingReducer] = &[CrossingReducer::Median(2)];

/// Standard preset — 4 median passes followed by 2 adjacent-exchange passes.
///
/// Good balance of quality and speed for most graphs.  This is the
/// default pipeline.
pub const STANDARD: &[CrossingReducer] = &[
    CrossingReducer::Median(4),
    CrossingReducer::AdjacentExchange(2),
];

/// Quality preset — thorough reduction for complex, tangled graphs.
///
/// 8 median passes, 4 adjacent-exchange refinements, then 2 final
/// median passes to clean up any regressions.
pub const QUALITY: &[CrossingReducer] = &[
    CrossingReducer::Median(8),
    CrossingReducer::AdjacentExchange(4),
    CrossingReducer::Median(2),
];

// ── Helpers ──────────────────────────────────────────────────────────────

/// Count crossings produced by two nodes at adjacent positions on a level.
///
/// Given the sorted neighbour-positions of node **u** (at position *i*)
/// and node **v** (at position *i + 1*) in the reference (adjacent) level,
/// returns `(crossings_uv, crossings_vu)`:
///
/// - `crossings_uv` — crossings when **u** is before **v** (current order)
/// - `crossings_vu` — crossings when **v** is before **u** (swapped order)
///
/// The caller should swap if `crossings_vu < crossings_uv`.
#[inline]
pub(crate) fn count_crossings_pair(u_positions: &[usize], v_positions: &[usize]) -> (usize, usize) {
    let mut cross_uv: usize = 0; // u before v
    let mut cross_vu: usize = 0; // v before u (swapped)

    for &a in u_positions {
        for &b in v_positions {
            if a > b {
                cross_uv += 1;
            } else if a < b {
                cross_vu += 1;
            }
            // a == b: no crossing either way
        }
    }

    (cross_uv, cross_vu)
}

// ── LayoutConfig (backward compatibility) ────────────────────────────────

/// **Deprecated**: Use [`SugiyamaConfig`](super::config::SugiyamaConfig) instead.
///
/// This re-export preserves backward compatibility for code using the
/// old `LayoutConfig` name.
#[allow(deprecated)]
pub use super::config::LayoutConfig;
