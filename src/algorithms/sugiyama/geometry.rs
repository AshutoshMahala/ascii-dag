//! Cluster-geometry constants shared by both layout backends.
//!
//! The heap path (`heap.rs` / `subgraph.rs`) and the no-alloc CSR path
//! (`arena_csr.rs`) must render subgraphs identically. Every spacing
//! constant they share lives here — defining one locally in a backend
//! is a bug, because the two copies can silently drift apart with one
//! bad edit.
//!
//! This module is unconditional (no `alloc` requirement) so the CSR
//! path can use it in `no_std` builds.

/// Horizontal padding between a subgraph border and its member nodes
/// (chars on each side).
pub(crate) const SUBGRAPH_H_PAD: usize = 2;

/// Vertical padding above the first node row: border + label + blank.
pub(crate) const SUBGRAPH_V_PAD_TOP: usize = 3; // ╔═══╗ + ║ Label ║ + ║     ║

/// Vertical padding below the last node row: blank + border.
pub(crate) const SUBGRAPH_V_PAD_BOTTOM: usize = 2; // ║     ║ + ╚═══════╝

/// Minimum gap between the bounding boxes of sibling subgraphs.
pub(crate) const SIBLING_GAP: usize = 1;

/// Horizontal expansion of a parent cluster's envelope around a child
/// cluster's box: the child box already includes its own
/// `SUBGRAPH_H_PAD`, so the parent needs only its border column.
pub(crate) const PARENT_CHILD_H_GAP: usize = 1;

/// Gap between adjacent nodes belonging to different clusters:
/// both nodes' border padding plus the sibling gap between the boxes.
pub(crate) const SG_GAP: usize = SUBGRAPH_H_PAD * 2 + SIBLING_GAP;

/// Width of a dummy (edge pass-through) vnode in the level packing.
/// Covers the per-edge draw offset (`edge_idx % 4`) so a dummy's body
/// extent bounds the column its vertical is actually drawn at.
pub(crate) const DUMMY_WIDTH: usize = 3;

/// Gap kept between a cluster's projected border envelope and an
/// external node pushed or compacted next to it.
pub(crate) const ENVELOPE_CLEARANCE: usize = 1;

// ── Axis profile (temp/08 D1) ───────────────────────────────────────────
//
// The layout pipeline computes in two ROLES: a level (flow) axis and a
// cross axis. Today's code spells the roles as y and x; the `Axis`
// trait names which node dimension and which constants feed each role,
// so ONE pipeline serves TopDown/BottomUp (`Vertical`) and, once
// LR-P1 lands, LeftRight/RightLeft (`Horizontal`). Zero-sized +
// monomorphized: every lookup inlines and constant-folds. LR-P0's
// byte-identity gate proves unchanged BEHAVIOR; the performance claim
// is verified separately by LR-N1's timing and binary-size checks.

/// Which node dimension and which spacing constants feed the level
/// axis vs the cross axis. Implemented by zero-sized marker types;
/// never stored, never public — layout-internal only.
// Consumers: heap layout (alloc) and CSR layout (arena); the trait is
// wholly unused in builds with neither feature. SG_PAD_* items are
// consumed by LR-P0 slice 3 — narrow this to a cfg_attr on
// not(alloc|arena) when that slice lands.
#[allow(dead_code)]
pub(crate) trait Axis {
    /// Node extent along the level (flow) axis. Vertical: height.
    fn level_extent(width: usize, height: usize) -> usize;
    /// Node extent across the flow. Vertical: width.
    fn cross_extent(width: usize, height: usize) -> usize;
    /// Subgraph border padding along the level axis (before, after
    /// content). Vertical: (`SUBGRAPH_V_PAD_TOP`, `SUBGRAPH_V_PAD_BOTTOM`).
    const SG_PAD_LEVEL: (usize, usize);
    /// Subgraph border padding across the flow (before, after) —
    /// asymmetric because the box label stays physically at the TOP:
    /// in Vertical the label row lives in the level-axis "before" pad;
    /// in Horizontal it lives in the cross-axis "before" pad.
    /// Vertical: (`SUBGRAPH_H_PAD`, `SUBGRAPH_H_PAD`).
    const SG_PAD_CROSS: (usize, usize);
    /// Cross-axis clearance a dummy waypoint's body reserves.
    /// Vertical: `DUMMY_WIDTH` (covers the `edge_idx % 4` draw offset).
    const DUMMY_CROSS: usize;
}

/// The TopDown/BottomUp profile: levels are rows, in-level order is
/// columns. (`Horizontal` arrives with LR-P1.)
pub(crate) struct Vertical;

impl Axis for Vertical {
    #[inline]
    fn level_extent(_width: usize, height: usize) -> usize {
        height
    }

    #[inline]
    fn cross_extent(width: usize, _height: usize) -> usize {
        width
    }

    const SG_PAD_LEVEL: (usize, usize) = (SUBGRAPH_V_PAD_TOP, SUBGRAPH_V_PAD_BOTTOM);
    const SG_PAD_CROSS: (usize, usize) = (SUBGRAPH_H_PAD, SUBGRAPH_H_PAD);
    const DUMMY_CROSS: usize = DUMMY_WIDTH;
}

/// Cap on how far a subgraph *label* may widen its bounding box.
///
/// A box is always wide enough for its member nodes; the label only
/// forces extra width up to this total. Longer labels are truncated by
/// the renderers (which clip to `width - 4`). Without a cap, a
/// pathological heading would blow up the canvas — and with it the
/// render buffer — linearly in the label length.
pub(crate) const SUBGRAPH_LABEL_BOX_CAP: usize = 40;

/// Minimum box width needed to show `label` (borders + spaces), capped.
#[inline]
pub(crate) fn label_min_width(label: &str) -> usize {
    (label.len() + 4).min(SUBGRAPH_LABEL_BOX_CAP)
}

/// First routing row below a source node's bottom row.
///
/// Reversed edges paint their `⇡` arrowhead on this row, directly below
/// their layout-source. Those cells are protected by **arrow-cell
/// reservation**: the slot allocators pre-occupy the arrow's interval on
/// slot 0, so a horizontal span that would run through an arrowhead
/// (`──⇡──`, unreadable) is pushed to a deeper slot by the normal
/// interval-collision logic instead of shifting every corner down.
pub(crate) const EDGE_START_ROW: usize = 1;

/// Half-width of the reserved interval around a reversed edge's
/// arrowhead column on the first routing row (one breathing cell on
/// each side, so no horizontal run abuts the `⇡` directly).
pub(crate) const ARROW_CELL_PAD: usize = 1;

/// Row offset below a source node's bottom row where an edge label is
/// painted: the first row beneath the level's corner-routing block
/// (`EDGE_START_ROW + one row per allocated slot`), over the vertical
/// segments running toward the targets — labels replace `│` cells,
/// never `─` cells, so they must clear every routing row.
#[inline]
pub(crate) fn edge_label_row_offset(level_slots: usize) -> usize {
    EDGE_START_ROW + level_slots.max(1)
}

/// Vertical routing budget below a level's nodes, excluding the
/// per-slot extra rows: one corner row plus one arrow-clearance row
/// above the next level — plus one label row, budgeted **only** for
/// levels that actually source a labeled edge (labels paint in the
/// layout-source's band). Shared by both backends.
#[inline]
pub(crate) fn routing_overhead(level_sources_labeled_edge: bool) -> usize {
    2 + usize::from(level_sources_labeled_edge)
}

/// Rows claimed by pass-through waypoints at a level: one per jogging
/// waypoint, **plus one bend row below the deepest jog** — every kept
/// waypoint bends at `wp_y + 1`, and without this extra row the deepest
/// bend lands on the arrow-clearance row above the next level
/// (`↓┈┈┈┘` collisions). Shared by both backends.
#[inline]
pub(crate) fn passthrough_rows(jogging_waypoints: usize) -> usize {
    jogging_waypoints + usize::from(jogging_waypoints > 0)
}
