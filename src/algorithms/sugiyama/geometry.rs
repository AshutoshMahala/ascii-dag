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
// wholly unused in builds with neither feature.
#[cfg_attr(not(any(feature = "alloc", feature = "arena")), allow(dead_code))]
pub(crate) trait Axis {
    /// Node extent along the level (flow) axis. Vertical: height.
    fn level_extent(width: usize, height: usize) -> usize;
    /// Node extent across the flow. Vertical: width.
    fn cross_extent(width: usize, height: usize) -> usize;
    /// THE materialization point (temp/08 slice 5): turn a role-space
    /// position into physical coordinates, returning `(x, y)`.
    /// Vertical: level → y, cross → x. Extents map identically —
    /// feeding `(level_total, cross_total)` yields the canvas
    /// `(width, height)`. Everything upstream of a `materialize` call
    /// computes in role space; physical axes exist only at IR
    /// emission and beyond.
    fn materialize(level: usize, cross: usize) -> (usize, usize);
    /// Physical axis of this profile's edge trunks (flow segments) —
    /// stamped on every emitted edge (temp/08 D2).
    const FLOW_AXIS: crate::ir::FlowAxis;
    /// Cross-axis PORT line of a node span — the line an edge
    /// attaches to. Matches the IR center-field formulas exactly:
    /// Vertical `center_x = x + w/2`, Horizontal
    /// `center_y = y + (h − 1)/2`.
    fn cross_port(cross: usize, extent: usize) -> usize;
    /// Level-axis line of a source node's PORT. Vertical keeps the
    /// legacy band-trailing endpoint (the level's tallest node decides
    /// where every edge starts — byte-frozen behavior); Horizontal
    /// ports sit on the node's OWN trailing face, since column widths
    /// vary far more than row heights and a detached port is plainly
    /// visible.
    #[cfg_attr(not(feature = "alloc"), allow(dead_code))]
    fn source_port_level(band_start: usize, node_extent: usize, band_extent: usize) -> usize;
    /// Cross-axis draw offset of a dummy waypoint (visual separation
    /// of convergent skip edges). Vertical spreads edges over
    /// `edge_idx % 4` columns inside the `DUMMY_CROSS = 3` clearance;
    /// Horizontal reserves a single row (`DUMMY_CROSS = 1`), so the
    /// offset is always 0 — a nonzero offset would escape the
    /// reservation and can enter the next span at `node_spacing = 1`.
    #[cfg_attr(not(feature = "alloc"), allow(dead_code))]
    fn dummy_draw_offset(edge_idx: usize) -> usize;
    /// The box label's claim on the CROSS axis (temp/08 D8). Label
    /// text is a physical x-width; it constrains the cross extent
    /// only while cross == x. Vertical: `label_min_width`;
    /// Horizontal: 0 — folding it into cross extents would make LR
    /// boxes TALLER by character count.
    fn label_cross_extent(label: &str) -> usize;
    /// The box label's claim on the LEVEL axis — the D8(b) rule's
    /// other half. Vertical: 0 (the cross fold covers it);
    /// Horizontal: `label_min_width`, reserved as extra trailing
    /// level pad at the box's closing level so the widened box
    /// cannot overlap the next column.
    #[cfg_attr(not(feature = "alloc"), allow(dead_code))]
    fn label_level_extent(label: &str) -> usize;
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
    /// Cross-axis gap between sibling cluster boxes.
    /// Vertical: `SIBLING_GAP`.
    const SIBLING_GAP_CROSS: usize;
    /// Cross-axis clearance between a cluster's envelope and an
    /// external node. Vertical: `ENVELOPE_CLEARANCE`.
    const ENVELOPE_CLEARANCE_CROSS: usize;
    /// Parent-envelope expansion around a child box, as (before,
    /// after) pairs PER AXIS. The child carries its own pads; the
    /// parent adds only its border line on the label-free axis — but
    /// the LABEL-side axis needs the full border + label + blank
    /// block, and which axis that is rotates with the profile (D3).
    /// Vertical: level = (`SUBGRAPH_V_PAD_TOP`,
    /// `SUBGRAPH_V_PAD_BOTTOM`), cross = (`PARENT_CHILD_H_GAP`,
    /// `PARENT_CHILD_H_GAP`).
    const PARENT_CHILD_PAD_LEVEL: (usize, usize);
    /// See [`Axis::PARENT_CHILD_PAD_LEVEL`].
    const PARENT_CHILD_PAD_CROSS: (usize, usize);
    /// Whether box labels claim LEVEL-axis room (D8b) — gates the
    /// label-extras phase so `Vertical` never allocates or traverses
    /// for a claim that is statically zero.
    #[cfg_attr(not(feature = "alloc"), allow(dead_code))]
    const LABEL_CLAIMS_LEVEL_AXIS: bool;
    /// Whether nested boxes' CROSS pads may share cells. Vertical:
    /// true — coincident borders merge into junction glyphs at render
    /// time (the class-B ruling), so packing reserves only the
    /// immediate box's pad. Horizontal: false — the cross-leading pad
    /// carries the LABEL ROW, which cannot merge; packing must
    /// reserve the full ancestry chain
    /// (`PARENT_CHILD_PAD_CROSS` per ancestor).
    #[cfg_attr(not(feature = "alloc"), allow(dead_code))]
    const NESTED_PADS_MERGE: bool;
    /// Cross-axis gap between adjacent nodes of different clusters:
    /// one box's trailing pad + the sibling gap + the other's leading
    /// pad. Derived — do not override.
    const SG_GAP_CROSS: usize =
        Self::SG_PAD_CROSS.1 + Self::SIBLING_GAP_CROSS + Self::SG_PAD_CROSS.0;
}

/// The TopDown/BottomUp profile: levels are rows, in-level order is
/// columns.
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

    #[inline]
    fn materialize(level: usize, cross: usize) -> (usize, usize) {
        (cross, level)
    }

    #[inline]
    fn cross_port(cross: usize, extent: usize) -> usize {
        cross + extent / 2
    }

    #[inline]
    fn source_port_level(band_start: usize, _node_extent: usize, band_extent: usize) -> usize {
        band_start + band_extent - 1
    }

    #[inline]
    fn dummy_draw_offset(edge_idx: usize) -> usize {
        edge_idx % 4
    }

    #[inline]
    fn label_cross_extent(label: &str) -> usize {
        label_min_width(label)
    }

    #[inline]
    fn label_level_extent(_label: &str) -> usize {
        0
    }

    const FLOW_AXIS: crate::ir::FlowAxis = crate::ir::FlowAxis::Y;
    const SG_PAD_LEVEL: (usize, usize) = (SUBGRAPH_V_PAD_TOP, SUBGRAPH_V_PAD_BOTTOM);
    const SG_PAD_CROSS: (usize, usize) = (SUBGRAPH_H_PAD, SUBGRAPH_H_PAD);
    const DUMMY_CROSS: usize = DUMMY_WIDTH;
    const SIBLING_GAP_CROSS: usize = SIBLING_GAP;
    const ENVELOPE_CLEARANCE_CROSS: usize = ENVELOPE_CLEARANCE;
    const PARENT_CHILD_PAD_LEVEL: (usize, usize) = (SUBGRAPH_V_PAD_TOP, SUBGRAPH_V_PAD_BOTTOM);
    const PARENT_CHILD_PAD_CROSS: (usize, usize) = (PARENT_CHILD_H_GAP, PARENT_CHILD_H_GAP);
    const LABEL_CLAIMS_LEVEL_AXIS: bool = false;
    const NESTED_PADS_MERGE: bool = true;
}

/// The LeftRight/RightLeft profile (temp/08 D1/D3): levels are
/// COLUMNS sized by node widths, in-level stacking is vertical by
/// node heights. The box label still reads horizontally and stays
/// physically at the top, so it lives in the cross-axis *leading*
/// pad (D3) — hence the asymmetric `SG_PAD_CROSS`.
pub(crate) struct Horizontal;

impl Axis for Horizontal {
    #[inline]
    fn level_extent(width: usize, _height: usize) -> usize {
        width
    }

    #[inline]
    fn cross_extent(_width: usize, height: usize) -> usize {
        height
    }

    #[inline]
    fn materialize(level: usize, cross: usize) -> (usize, usize) {
        (level, cross)
    }

    #[inline]
    fn cross_port(cross: usize, extent: usize) -> usize {
        cross + extent.saturating_sub(1) / 2
    }

    #[inline]
    fn source_port_level(band_start: usize, node_extent: usize, _band_extent: usize) -> usize {
        band_start + node_extent - 1
    }

    #[inline]
    fn dummy_draw_offset(_edge_idx: usize) -> usize {
        0
    }

    #[inline]
    fn label_cross_extent(_label: &str) -> usize {
        0
    }

    #[inline]
    fn label_level_extent(label: &str) -> usize {
        label_min_width(label)
    }

    const FLOW_AXIS: crate::ir::FlowAxis = crate::ir::FlowAxis::X;
    /// Level-axis pads are the box's left/right borders: border +
    /// one space, mirroring `SUBGRAPH_H_PAD`'s shape.
    const SG_PAD_LEVEL: (usize, usize) = (2, 2);
    /// Cross-axis pads: the leading (top) pad carries border +
    /// label row + blank (like `SUBGRAPH_V_PAD_TOP`); the trailing
    /// (bottom) pad is blank + border.
    const SG_PAD_CROSS: (usize, usize) = (3, 2);
    /// LR dummies occupy one row of stacking space — the TD value 3
    /// covers the `edge_idx % 4` DRAW offset, a cross-axis concern
    /// only when cross is horizontal.
    const DUMMY_CROSS: usize = 1;
    const SIBLING_GAP_CROSS: usize = 1;
    const ENVELOPE_CLEARANCE_CROSS: usize = 1;
    /// Level-axis: border column each side. Cross-axis: the label
    /// block leads (border + label row + blank), blank + border
    /// trails — same shape as `SG_PAD_CROSS`.
    const PARENT_CHILD_PAD_LEVEL: (usize, usize) = (1, 1);
    const PARENT_CHILD_PAD_CROSS: (usize, usize) = (3, 2);
    const LABEL_CLAIMS_LEVEL_AXIS: bool = true;
    const NESTED_PADS_MERGE: bool = false;
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
///
/// This is a **physical x-width** — label text always reads
/// horizontally. The cluster passes route it through the D8 axis
/// hooks: [`Axis::label_cross_extent`] folds it into cross extents
/// under `Vertical` only, and [`Axis::label_level_extent`] carries
/// the claim to the level axis under `Horizontal` (with room
/// reserved by the label-extras phase).
#[inline]
pub(crate) fn label_min_width(label: &str) -> usize {
    (label.len() + 4).min(SUBGRAPH_LABEL_BOX_CAP)
}

// ── Routing geometry (level-axis budgets; temp/08 slice 4) ──────────────
//
// Everything below measures LEVEL-AXIS cells in the routing band that
// follows a level's nodes: rows below the nodes in TD, columns to
// their right in LR. The values are shared across axes; only the
// physical meaning of "line" rotates.

/// First routing line past a source node's trailing edge, in
/// level-axis cells (TD: the row below the node's bottom row).
///
/// Reversed edges paint their arrowhead on this line, directly beyond
/// their layout-source. Those cells are protected by **arrow-cell
/// reservation**: the slot allocators pre-occupy the arrow's interval
/// on slot 0, so a cross-cutting span that would run through an
/// arrowhead (`──⇡──`, unreadable) is pushed to a deeper slot by the
/// normal interval-collision logic instead of shifting every corner.
pub(crate) const EDGE_START_OFFSET: usize = 1;

/// Half-width of the reserved interval around a reversed edge's
/// arrowhead on the first routing line, in cross-axis cells (one
/// breathing cell on each side, so no routed span abuts the arrowhead
/// directly).
pub(crate) const ARROW_CELL_PAD: usize = 1;

/// Level-axis offset past a source node's trailing edge where an edge
/// label is painted: the first line beneath the level's corner-routing
/// block (`EDGE_START_OFFSET + one line per allocated slot`), over the
/// flow segments running toward the targets — labels replace flow
/// cells (`│` in TD), never trunk cells (`─` in TD), so they must
/// clear every routing line.
#[inline]
pub(crate) fn edge_label_offset(level_slots: usize) -> usize {
    EDGE_START_OFFSET + level_slots.max(1)
}

/// Level-axis routing budget past a level's nodes, excluding the
/// per-slot extra lines: one corner line plus one arrow-clearance line
/// before the next level — plus one label line, budgeted **only** for
/// levels that actually source a labeled edge (labels paint in the
/// layout-source's band). Shared by both backends.
#[inline]
pub(crate) fn routing_overhead(level_sources_labeled_edge: bool) -> usize {
    2 + usize::from(level_sources_labeled_edge)
}

/// Level-axis cells claimed by pass-through waypoints at a level: one
/// per jogging waypoint, **plus one bend line past the deepest jog** —
/// every kept waypoint bends one line beyond itself (`wp_y + 1` in
/// TD), and without this extra line the deepest bend lands on the
/// arrow-clearance line before the next level (`↓┈┈┈┘` collisions).
/// Shared by both backends.
#[inline]
pub(crate) fn passthrough_extent(jogging_waypoints: usize) -> usize {
    jogging_waypoints + usize::from(jogging_waypoints > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_map_roles_to_opposite_axes() {
        // Vertical: level → y, cross → x. Horizontal: level → x, cross → y.
        assert_eq!(Vertical::materialize(7, 3), (3, 7));
        assert_eq!(Horizontal::materialize(7, 3), (7, 3));
        // A 12×5 node: Vertical levels are sized by heights,
        // Horizontal levels by widths.
        assert_eq!(Vertical::level_extent(12, 5), 5);
        assert_eq!(Vertical::cross_extent(12, 5), 12);
        assert_eq!(Horizontal::level_extent(12, 5), 12);
        assert_eq!(Horizontal::cross_extent(12, 5), 5);
    }

    #[test]
    fn derived_cluster_gap_follows_the_pads() {
        assert_eq!(Vertical::SG_GAP_CROSS, SUBGRAPH_H_PAD * 2 + SIBLING_GAP);
        // Horizontal: trailing (bottom) pad 2 + sibling 1 + leading
        // (top: border + label + blank) pad 3.
        assert_eq!(Horizontal::SG_GAP_CROSS, 6);
    }
}
