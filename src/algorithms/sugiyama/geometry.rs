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
#[cfg(feature = "layout-vertical")]
pub(crate) const SUBGRAPH_H_PAD: usize = 2;

/// Vertical padding above the first node row: border + label + blank.
#[cfg(feature = "layout-vertical")]
pub(crate) const SUBGRAPH_V_PAD_TOP: usize = 3; // ╔═══╗ + ║ Label ║ + ║     ║

/// Vertical padding below the last node row: blank + border.
#[cfg(feature = "layout-vertical")]
pub(crate) const SUBGRAPH_V_PAD_BOTTOM: usize = 2; // ║     ║ + ╚═══════╝

/// Minimum gap between the bounding boxes of sibling subgraphs.
#[cfg(feature = "layout-vertical")]
pub(crate) const SIBLING_GAP: usize = 1;

/// Horizontal expansion of a parent cluster's envelope around a child
/// cluster's box: the child box already includes its own
/// `SUBGRAPH_H_PAD`, so the parent needs only its border column.
#[cfg(feature = "layout-vertical")]
pub(crate) const PARENT_CHILD_H_GAP: usize = 1;

/// Width of a dummy (edge pass-through) vnode in the level packing.
/// Covers the per-edge draw offset (`edge_idx % 4`) so a dummy's body
/// extent bounds the column its vertical is actually drawn at.
#[cfg(feature = "layout-vertical")]
pub(crate) const DUMMY_WIDTH: usize = 3;

/// Gap kept between a cluster's projected border envelope and an
/// external node pushed or compacted next to it.
#[cfg(feature = "layout-vertical")]
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
    /// Cross-axis CENTER line of a node span — where an `Auto` port
    /// attaches (explicit ports resolve their own line along the
    /// face). Matches the IR center-field formulas exactly: Vertical
    /// `center_x = x + w/2`, Horizontal `center_y = y + (h − 1)/2`.
    fn cross_center(cross: usize, extent: usize) -> usize;
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
    /// Safety margin added to the CROSS extent beyond the packed
    /// levels: room for the dummy draw offset, edge-label overhang,
    /// and cluster borders.
    ///
    /// Vertical needs all three — the cross axis is x, where labels
    /// spread and dummies fan over `edge_idx % 4` columns. Horizontal
    /// needs almost none: those are PHYSICAL-X concerns, which is its
    /// LEVEL axis, and its `DUMMY_CROSS` reserves no draw offset. It
    /// keeps one row for a trailing node's self-loop marker, which
    /// sits one cell past the node on the cross axis (D5).
    fn cross_margin(has_labeled_edges: bool, has_subgraphs: bool) -> usize;
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
#[cfg(feature = "layout-vertical")]
pub(crate) struct Vertical;

#[cfg(feature = "layout-vertical")]
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
    fn cross_center(cross: usize, extent: usize) -> usize {
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
    fn cross_margin(has_labeled_edges: bool, has_subgraphs: bool) -> usize {
        // Bounded edge offsets (max 3) + 1 for routing; labels reach
        // 4 columns each side; cluster borders their padding.
        4 + if has_labeled_edges { 8 } else { 0 } + if has_subgraphs { 4 } else { 0 }
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
#[cfg(feature = "layout-horizontal")]
pub(crate) struct Horizontal;

#[cfg(feature = "layout-horizontal")]
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
    fn cross_center(cross: usize, extent: usize) -> usize {
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
    fn cross_margin(has_labeled_edges: bool, _has_subgraphs: bool) -> usize {
        // One row for a trailing node's self-loop marker (it sits one
        // cell past the node on this axis, D5), plus one for D9's
        // adjacent-row label float — the host needs a line above the
        // source trunk to borrow. Everything else the vertical profile
        // reserves here is physical-x work, which lands on this
        // profile's LEVEL axis instead. Rows are the cheap direction
        // in LR, which is exactly why the float spends one.
        1 + usize::from(has_labeled_edges)
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

// ── Routing fans (temp/09 P2) ────────────────────────────────────────────
//
// A skip-level edge crossing the gap between two levels has to pass the
// cross-axis territory that the *shorter* edges in that gap already claim.
// Today its cross coordinate is derived from the level's NODES and knows
// nothing about that territory, so the longest edge in a graph can be
// handed a lane between two short ones and forced to cut across them.
//
// These are the primitives for reasoning about that territory. They are
// deliberately plain interval arithmetic rather than being generic over
// [`Axis`]: everything here is already in cross-axis space, so an axis
// parameter would only be decoration. What matters for backend parity is
// that the rule lives in exactly one place — this file — with both
// backends supplying their own coordinate storage. The clearance is a
// parameter for the same reason: callers pass `A::SG_GAP_CROSS` (or
// whatever the situation demands) rather than the rule guessing.

/// An inclusive cross-axis interval, `lo ..= hi`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CrossSpan {
    /// Lower cross-axis bound, inclusive.
    pub(crate) lo: usize,
    /// Upper cross-axis bound, inclusive.
    pub(crate) hi: usize,
}

impl CrossSpan {
    /// The span covered by an edge running between two cross positions,
    /// whichever order they arrive in.
    pub(crate) fn between(a: usize, b: usize) -> Self {
        CrossSpan {
            lo: a.min(b),
            hi: a.max(b),
        }
    }

    /// Does this span contain `c`?
    pub(crate) fn contains(&self, c: usize) -> bool {
        self.lo <= c && c <= self.hi
    }
}

// Lane-pass budget (temp/09 P3/P4). The chain-lane allocator's scratch
// scales with claims × levels, which is unbounded on stress-scale graphs
// — and the CSR backend must pre-size every buffer in the caller's arena.
// Rather than let the estimator explode (or silently cap quality on one
// backend only), the pass runs under a budget BOTH backends evaluate
// identically: over budget → the graph keeps its packed routing, exactly
// the 0.10.0 output. Human-scale graphs (the entire quality corpus) are
// far inside the budget; a 50k-node stress diamond is far outside and
// pays zero arena bytes for the feature.

/// Work ceiling: the pass is skipped when edges or dummies exceed this.
pub(crate) const LANE_PASS_MAX_WORK: usize = 16_384;
/// Depth ceiling: the pass is skipped beyond this many levels.
pub(crate) const LANE_PASS_MAX_LEVELS: usize = 4_096;
/// Per-chain candidate budget for the §4.7 DP; a chain needing more
/// keeps its packed coordinates (both backends, same rule).
pub(crate) const LANE_CAND_CAP: usize = 4_096;
/// Per-chain span-scratch budget for fan unions; over budget → packed.
pub(crate) const LANE_SPAN_CAP: usize = 8_192;
/// Global work purse for one whole lane pass, in claim-comparison
/// units. Memory caps alone bound neither transitions nor the claim
/// scans each transition performs — two 2,048-candidate rows are ~4M
/// transitions, each scanning that gap's claims. Every costed phase
/// (span counting, lane consider-scans, candidate generation, weighted
/// DP) charges this purse; a chain the remainder cannot cover keeps its
/// packed routing. Both backends charge the same amounts at the same
/// points in the same chain order, so they exhaust identically.
pub(crate) const LANE_WORK_BUDGET: usize = 1 << 20;

/// Weighted §4.7 DP cost: transitions × the claim scans each performs.
/// `rows` are candidate counts per interior level; `claims_per_gap` are
/// the chain's filtered claim counts, one per traversed gap
/// (`rows.len() + 1` entries — source-side gap first, target-side last).
// Called by the heap backend; the CSR backend mirrors this arithmetic
// inline (it cannot build the input slices without alloc). The unit
// tests below pin the shared formula both implementations must match.
#[cfg_attr(not(feature = "alloc"), allow(dead_code))]
pub(crate) fn lane_dp_work(rows: &[usize], claims_per_gap: &[usize]) -> usize {
    if rows.is_empty() {
        return 0;
    }
    let mut w = rows[0].saturating_mul(claims_per_gap[0] + 1);
    for i in 1..rows.len() {
        w = w.saturating_add(
            rows[i - 1]
                .saturating_mul(rows[i])
                .saturating_mul(claims_per_gap[i] + 1),
        );
    }
    w.saturating_add(rows[rows.len() - 1].saturating_mul(claims_per_gap[rows.len()] + 1))
}

/// Fast-path (§4.3) cost: the union build/merge plus the per-component
/// `total_dist` scans of the consider walk.
pub(crate) fn lane_scan_work(span_need: usize, components: usize, waypoints: usize) -> usize {
    span_need.saturating_add((components + 1).saturating_mul(waypoints))
}

/// Whether a cross coordinate is placeable at all (`LANE_MAX_CROSS`).
/// One shared predicate so the two backends cannot disagree at the
/// representability boundary.
pub(crate) fn lane_admissible(p: usize) -> bool {
    p <= LANE_MAX_CROSS
}
/// Largest representable cross coordinate: the CSR backend stores
/// coordinates as `u16`, so a lane beyond this cannot exist there.
/// BOTH backends refuse such lanes (heap included, though `usize` could
/// hold them) — clamping instead would write a coordinate the fan
/// arithmetic never cleared, and letting only one backend refuse would
/// fork the outputs.
pub(crate) const LANE_MAX_CROSS: usize = u16::MAX as usize;

/// Whether the chain-lane pass runs at all. Evaluated identically by the
/// heap backend, the CSR backend, and the arena estimator — the three
/// must agree or backends diverge / arenas under-provision.
pub(crate) fn lane_pass_enabled(n_levels: usize, n_edges: usize, dummies: usize) -> bool {
    dummies > 0
        && dummies <= LANE_PASS_MAX_WORK
        && n_edges <= LANE_PASS_MAX_WORK
        && n_levels <= LANE_PASS_MAX_LEVELS
}

/// A raw cross-axis claim in one inter-level gap, keeping *whose* it is.
///
/// Provenance is what makes §4.5's endpoint exemptions decidable: a merged
/// span has forgotten which edge swept it, so exemption filtering must
/// happen on raw claims, before any [`merge_fan`]. Crossing counts also
/// run against raw claims — merging would collapse two distinct crossed
/// edges into one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GapClaim {
    /// Cross-axis span the edge segment sweeps (body-widened for dummies).
    pub(crate) span: CrossSpan,
    /// The claiming edge's index in `Graph::edges`.
    pub(crate) edge_idx: usize,
}

/// Normalize the spans claimed in one inter-level gap into the gap's *fan*:
/// widened by `clearance`, sorted, and merged into disjoint runs. Returns
/// how many entries of `spans` the fan occupies; the rest is scratch.
///
/// The fan is an interval **union**, not the convex envelope of its inputs.
/// That distinction matters: with two clusters far apart on the cross axis,
/// the envelope would swallow the free lane between them and push a passing
/// edge beyond both — paying canvas for space that was never occupied. The
/// union keeps that lane usable.
///
/// Works in place on a caller-owned slice so the arena backend can supply
/// scratch without allocating.
pub(crate) fn merge_fan(spans: &mut [CrossSpan], clearance: usize) -> usize {
    if spans.is_empty() {
        return 0;
    }
    for s in spans.iter_mut() {
        s.lo = s.lo.saturating_sub(clearance);
        s.hi = s.hi.saturating_add(clearance);
    }
    spans.sort_unstable_by_key(|s| (s.lo, s.hi));

    let mut write = 0;
    for read in 1..spans.len() {
        // Merge when they overlap OR merely touch: `hi + 1 == lo` leaves no
        // usable cell between them, so treating them as one run keeps
        // `nearest_outside` from ever proposing a position that does not
        // exist.
        if spans[read].lo <= spans[write].hi.saturating_add(1) {
            spans[write].hi = spans[write].hi.max(spans[read].hi);
        } else {
            write += 1;
            spans[write] = spans[read];
        }
    }
    write + 1
}

/// The free interval of the cross axis containing `p`, as `(lo, hi)` with
/// `hi == None` meaning unbounded above. `None` when `p` is itself inside
/// the fan.
///
/// `fan` must already be merged by [`merge_fan`].
pub(crate) fn free_gap_containing(fan: &[CrossSpan], p: usize) -> Option<(usize, Option<usize>)> {
    if fan.iter().any(|s| s.contains(p)) {
        return None;
    }
    let lo = fan
        .iter()
        .filter(|s| s.hi < p)
        .map(|s| s.hi + 1)
        .max()
        .unwrap_or(0);
    let hi = fan.iter().filter(|s| s.lo > p).map(|s| s.lo - 1).min();
    Some((lo, hi))
}

/// The cross position closest to `ideal` that lies outside every span of
/// `fan`, which must already be merged by [`merge_fan`].
///
/// `from` is the chain's previous cross coordinate, when it has one. It is
/// not a hint — it is a constraint. A dummy chain that has already committed
/// to one side of a fan must stay there: without `from`, asking for the
/// position merely *nearest* to `ideal` will happily answer with a cell on
/// the far side, and the resulting segment then runs straight through the
/// fan the placement existed to avoid. Given `from`, the result is confined
/// to the free interval `from` already occupies, so the whole segment
/// `from ..= result` is guaranteed clear.
///
/// Pass `None` for the first dummy in a chain, which has no established
/// side; then the rule is nearest to `ideal`, ties resolving **upward**.
/// The low end of the cross axis carries cluster leading pads and level
/// margins, so it is the more contended direction — and a fixed rule is
/// what keeps the two backends byte-identical, which "either is fine"
/// would not.
///
/// Returns `None` only when no position outside the fan is representable —
/// a fan reaching `usize::MAX` with nothing below it. Callers get an
/// explicit failure rather than a coordinate that is silently still inside.
pub(crate) fn nearest_outside(
    fan: &[CrossSpan],
    ideal: usize,
    from: Option<usize>,
) -> Option<usize> {
    // Anchored: clamp into the free interval the chain already sits in.
    if let Some(f) = from {
        if let Some((glo, ghi)) = free_gap_containing(fan, f) {
            let mut p = ideal.max(glo);
            if let Some(h) = ghi {
                p = p.min(h);
            }
            return Some(p);
        }
        // `from` is itself inside the fan — nothing to preserve; fall through.
    }

    let Some(idx) = fan.iter().position(|s| s.contains(ideal)) else {
        return Some(ideal);
    };

    // Upward: leave this run, then any run that abuts it.
    let mut up = fan[idx].hi.checked_add(1);
    for s in &fan[idx + 1..] {
        match up {
            Some(u) if s.contains(u) => up = s.hi.checked_add(1),
            _ => break,
        }
    }

    // Downward: same walk, but the axis bottoms out at 0.
    let mut down = fan[idx].lo.checked_sub(1);
    for s in fan[..idx].iter().rev() {
        match down {
            Some(d) if s.contains(d) => down = s.lo.checked_sub(1),
            _ => break,
        }
    }

    match (down, up) {
        (Some(d), Some(u)) => Some(if ideal - d < u - ideal { d } else { u }),
        (Some(d), None) => Some(d),
        (None, Some(u)) => Some(u),
        (None, None) => None,
    }
}

/// Placement order for the dummy chains crossing one gap.
///
/// Ash's rule: the edge travelling farthest diverges first and takes the
/// outer track. Since a placed chain becomes part of the fan that later
/// chains must clear, "who is placed first" is part of the fan's
/// definition, not a detail — so the comparator lives here, shared, rather
/// than being written twice and drifting.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChainKey {
    /// Levels this edge still has to travel from the gap being placed.
    pub(crate) remaining: usize,
    /// Total levels the edge spans end to end.
    pub(crate) total: usize,
    /// Edge index — the final tie-break, so the order is total.
    pub(crate) edge: usize,
}

/// Order two chains for placement: longest remaining span first, then
/// longest total span, then lowest edge index.
#[allow(dead_code)]
pub(crate) fn chain_cmp(a: &ChainKey, b: &ChainKey) -> core::cmp::Ordering {
    b.remaining
        .cmp(&a.remaining)
        .then_with(|| b.total.cmp(&a.total))
        .then_with(|| a.edge.cmp(&b.edge))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_map_roles_to_opposite_axes() {
        // Vertical: level → y, cross → x. Horizontal: level → x, cross → y.
        #[cfg(feature = "layout-vertical")]
        {
            assert_eq!(Vertical::materialize(7, 3), (3, 7));
            // A 12×5 node: Vertical levels are sized by heights.
            assert_eq!(Vertical::level_extent(12, 5), 5);
            assert_eq!(Vertical::cross_extent(12, 5), 12);
        }
        #[cfg(feature = "layout-horizontal")]
        {
            assert_eq!(Horizontal::materialize(7, 3), (7, 3));
            // Horizontal levels are sized by widths.
            assert_eq!(Horizontal::level_extent(12, 5), 12);
            assert_eq!(Horizontal::cross_extent(12, 5), 5);
        }
    }

    #[test]
    fn derived_cluster_gap_follows_the_pads() {
        #[cfg(feature = "layout-vertical")]
        assert_eq!(Vertical::SG_GAP_CROSS, SUBGRAPH_H_PAD * 2 + SIBLING_GAP);
        // Horizontal: trailing (bottom) pad 2 + sibling 1 + leading
        // (top: border + label + blank) pad 3.
        #[cfg(feature = "layout-horizontal")]
        assert_eq!(Horizontal::SG_GAP_CROSS, 6);
    }

    // ── Routing fans (temp/09 P2) ────────────────────────────────────────

    fn fan(pairs: &[(usize, usize)], clearance: usize) -> Vec<CrossSpan> {
        let mut v: Vec<CrossSpan> = pairs
            .iter()
            .map(|&(a, b)| CrossSpan::between(a, b))
            .collect();
        let n = merge_fan(&mut v, clearance);
        v.truncate(n);
        v
    }

    #[test]
    fn between_normalizes_either_order() {
        assert_eq!(CrossSpan::between(7, 3), CrossSpan { lo: 3, hi: 7 });
        assert_eq!(CrossSpan::between(3, 7), CrossSpan { lo: 3, hi: 7 });
    }

    #[test]
    fn merge_fan_sorts_and_unions_overlaps() {
        assert_eq!(
            fan(&[(10, 14), (0, 5), (3, 8)], 0),
            vec![CrossSpan { lo: 0, hi: 8 }, CrossSpan { lo: 10, hi: 14 }]
        );
    }

    #[test]
    fn merge_fan_joins_touching_runs() {
        // 0..=4 and 5..=9 leave no usable cell between them.
        assert_eq!(fan(&[(0, 4), (5, 9)], 0), vec![CrossSpan { lo: 0, hi: 9 }]);
    }

    #[test]
    fn merge_fan_applies_clearance_and_clamps_at_zero() {
        // 3..=5 widened by 4 would reach -1; the axis bottoms out at 0.
        assert_eq!(fan(&[(3, 5)], 4), vec![CrossSpan { lo: 0, hi: 9 }]);
    }

    #[test]
    fn merge_fan_clearance_can_close_a_gap() {
        // 0..=2 and 6..=8 are disjoint bare, but clearance 2 merges them.
        assert_eq!(fan(&[(0, 2), (6, 8)], 0).len(), 2);
        assert_eq!(fan(&[(0, 2), (6, 8)], 2), vec![CrossSpan { lo: 0, hi: 10 }]);
    }

    /// The whole segment between two cross positions must miss the fan —
    /// not merely the endpoint. This is the property the `from` argument
    /// exists to guarantee.
    fn segment_clear(fan: &[CrossSpan], a: usize, b: usize) -> bool {
        let (lo, hi) = (a.min(b), a.max(b));
        !fan.iter().any(|s| s.lo <= hi && lo <= s.hi)
    }

    #[test]
    fn nearest_outside_is_identity_when_already_clear() {
        let f = fan(&[(2, 5)], 0);
        assert_eq!(nearest_outside(&f, 0, None), Some(0));
        assert_eq!(nearest_outside(&f, 9, None), Some(9));
        assert_eq!(nearest_outside(&[], 4, None), Some(4));
    }

    #[test]
    fn nearest_outside_leaves_the_containing_run() {
        let f = fan(&[(2, 8)], 0);
        assert_eq!(
            nearest_outside(&f, 3, None),
            Some(1),
            "closer to the low edge"
        );
        assert_eq!(
            nearest_outside(&f, 7, None),
            Some(9),
            "closer to the high edge"
        );
    }

    #[test]
    fn nearest_outside_breaks_ties_upward() {
        // 2..=6, ideal 4: down=1 (distance 3), up=7 (distance 3).
        let f = fan(&[(2, 6)], 0);
        assert_eq!(nearest_outside(&f, 4, None), Some(7));
    }

    #[test]
    fn nearest_outside_skips_abutting_runs() {
        let f = fan(&[(0, 5), (7, 12)], 0);
        assert_eq!(f.len(), 2, "one free cell keeps them apart");
        assert_eq!(
            nearest_outside(&f, 3, None),
            Some(6),
            "the single free cell"
        );
        let g = fan(&[(0, 5), (6, 12)], 0);
        assert_eq!(g.len(), 1);
        assert_eq!(nearest_outside(&g, 3, None), Some(13));
    }

    #[test]
    fn nearest_outside_goes_up_when_zero_blocks_the_way_down() {
        let f = fan(&[(0, 6)], 0);
        assert_eq!(nearest_outside(&f, 1, None), Some(7));
    }

    #[test]
    fn nearest_outside_never_lands_inside_the_fan() {
        let f = fan(&[(0, 3), (5, 5), (9, 20)], 1);
        for ideal in 0..30 {
            let got = nearest_outside(&f, ideal, None).expect("finite fan");
            assert!(
                !f.iter().any(|s| s.contains(got)),
                "ideal {ideal} -> {got} landed inside {f:?}"
            );
        }
    }

    #[test]
    fn anchored_placement_never_crosses_the_fan() {
        // The reported case: a chain already at 11 asked to aim at 6.
        // Unanchored the answer is 4 — nearer, but the segment 11->4 runs
        // through the whole fan.
        let f = fan(&[(5, 10)], 0);
        assert_eq!(
            nearest_outside(&f, 6, None),
            Some(4),
            "nearest, ignoring heading"
        );
        assert!(!segment_clear(&f, 11, 4), "which crosses the fan");

        let anchored = nearest_outside(&f, 6, Some(11)).expect("finite fan");
        assert_eq!(
            anchored, 11,
            "clamped into the gap the chain already occupies"
        );
        assert!(segment_clear(&f, 11, anchored), "segment stays clear");
    }

    #[test]
    fn anchored_placement_holds_the_low_side_too() {
        let f = fan(&[(5, 10)], 0);
        let p = nearest_outside(&f, 9, Some(2)).expect("finite fan");
        assert_eq!(p, 4, "clamped to the top of the low gap");
        assert!(segment_clear(&f, 2, p));
    }

    #[test]
    fn anchored_placement_tracks_the_ideal_within_its_gap() {
        // Free gap 6..=8 between two runs; the chain sits at 7.
        let f = fan(&[(0, 5), (9, 20)], 0);
        assert_eq!(
            nearest_outside(&f, 0, Some(7)),
            Some(6),
            "clamped to gap floor"
        );
        assert_eq!(
            nearest_outside(&f, 8, Some(7)),
            Some(8),
            "ideal is reachable"
        );
        assert_eq!(
            nearest_outside(&f, 30, Some(7)),
            Some(8),
            "clamped to gap ceiling"
        );
        for ideal in 0..30 {
            let p = nearest_outside(&f, ideal, Some(7)).expect("finite fan");
            assert!(
                segment_clear(&f, 7, p),
                "ideal {ideal} -> {p} crossed the fan"
            );
        }
    }

    #[test]
    fn anchored_falls_back_when_the_anchor_is_itself_inside() {
        // Shouldn't happen if prior placement was correct, but must not panic.
        let f = fan(&[(5, 10)], 0);
        assert_eq!(
            nearest_outside(&f, 6, Some(7)),
            nearest_outside(&f, 6, None)
        );
    }

    #[test]
    fn nearest_outside_reports_failure_at_the_representable_edge() {
        // A fan reaching usize::MAX with nothing below has no outside cell.
        let f = [CrossSpan {
            lo: 0,
            hi: usize::MAX,
        }];
        assert_eq!(nearest_outside(&f, usize::MAX, None), None);
        // Room below is still found rather than overflowing upward.
        let g = [CrossSpan {
            lo: 4,
            hi: usize::MAX,
        }];
        assert_eq!(nearest_outside(&g, usize::MAX, None), Some(3));
    }

    #[test]
    fn hero_case_clears_the_gateway_fan() {
        // hero LR, Gateway's gap: its edges reach rows 3 and 7 from row 0,
        // so the fan is 0..=7. The trace dummy's ideal is row 0 (both its
        // endpoints sit there) and it currently lands at row 4, inside.
        let f = fan(&[(0, 3), (0, 7)], 0);
        assert_eq!(f, vec![CrossSpan { lo: 0, hi: 7 }]);
        assert_eq!(
            nearest_outside(&f, 4, None),
            Some(8),
            "clears Orders at row 7"
        );
        assert_eq!(nearest_outside(&f, 0, None), Some(8), "no room below row 0");
    }

    #[test]
    fn chain_order_is_longest_first_then_total_then_index() {
        use core::cmp::Ordering;
        let k = |remaining, total, edge| ChainKey {
            remaining,
            total,
            edge,
        };
        // More remaining travel wins outright.
        assert_eq!(chain_cmp(&k(5, 5, 9), &k(4, 9, 0)), Ordering::Less);
        // Equal remaining: longer total span wins.
        assert_eq!(chain_cmp(&k(3, 8, 9), &k(3, 4, 0)), Ordering::Less);
        // Equal on both: lowest edge index wins, so the order is total.
        assert_eq!(chain_cmp(&k(3, 8, 2), &k(3, 8, 7)), Ordering::Less);
        assert_eq!(chain_cmp(&k(3, 8, 7), &k(3, 8, 7)), Ordering::Equal);

        let mut v = [k(1, 1, 3), k(4, 4, 2), k(4, 9, 5), k(4, 9, 1)];
        v.sort_by(chain_cmp);
        assert_eq!(
            v.iter().map(|c| c.edge).collect::<Vec<_>>(),
            vec![1, 5, 2, 3],
            "remaining desc, total desc, edge asc"
        );
    }

    #[test]
    fn dp_work_weights_transitions_by_claim_scans() {
        // Single interior row: source edge + tail only.
        assert_eq!(lane_dp_work(&[4], &[2, 5]), 4 * 3 + 4 * 6);
        // Two rows: source + cross product + tail, each × (claims+1).
        assert_eq!(lane_dp_work(&[3, 2], &[1, 4, 0]), 3 * 2 + 3 * 2 * 5 + 2);
        assert_eq!(lane_dp_work(&[], &[7]), 0);
        // The review case: two 2,048 rows over 1,000-claim gaps is far
        // beyond the purse — the weighted meter must say so.
        assert!(lane_dp_work(&[2048, 2048], &[1000, 1000, 1000]) > LANE_WORK_BUDGET);
        // ...while the same rows over claim-free gaps are within it.
        assert!(lane_dp_work(&[64, 64], &[3, 3, 3]) < LANE_WORK_BUDGET);
    }

    #[test]
    fn scan_work_counts_union_and_consider_walk() {
        assert_eq!(lane_scan_work(100, 4, 6), 100 + 5 * 6);
        assert_eq!(lane_scan_work(0, 0, 0), 0);
    }

    #[test]
    fn admissibility_is_the_u16_boundary() {
        assert!(lane_admissible(0));
        assert!(lane_admissible(LANE_MAX_CROSS));
        assert!(!lane_admissible(LANE_MAX_CROSS + 1));
    }
}
