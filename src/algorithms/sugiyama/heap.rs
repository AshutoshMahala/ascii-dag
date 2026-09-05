//! Heap-based Sugiyama layout pipeline.
//!
//! Implements the full layout algorithm using standard heap allocations (`Vec`, `HashMap`).
//! This is the default layout path for `Graph::compute_layout()`.
//!
//! # Pipeline Stages
//!
//! 1. **Level assignment** — iterative fixed-point (from `layout.rs`)
//! 2. **Virtual node insertion** — dummy nodes for skip-level edges
//! 3. **Crossing reduction** — median heuristic on virtual levels
//! 4. **X-coordinate assignment** — left-to-right packing with centering
//! 5. **Slot allocation** — horizontal channel assignment for edge separation
//! 6. **Edge routing** — direct, corner, or multi-segment paths
//!
//! # Relationship to CSR Path
//!
//! The CSR-based layout in `arena_csr.rs` implements the same algorithm
//! using arena allocation and `Idx`-typed indices. The two paths produce
//! visually compatible output but operate on different type systems.
//! Shared spacing constants live in [`super::geometry`] so the backends
//! cannot drift apart.

use crate::algorithms::sugiyama::config::{CycleBreaking, LayoutConfig};
use crate::algorithms::sugiyama::crossing::{CrossingReducer, count_crossings_pair};
use crate::algorithms::sugiyama::geometry::Axis;
use crate::graph::Graph;
use crate::ir::{EdgePath, LayoutEdge, LayoutIR, LayoutIRBuilder, LayoutNode, NodeKind};
use alloc::vec;
use alloc::vec::Vec;

#[cfg(not(feature = "std"))]
use alloc::collections::{BTreeMap as HashMap, BTreeSet as HashSet};
#[cfg(feature = "std")]
use std::collections::{HashMap, HashSet};

// ── VNode: Virtual node for layout computation ───────────────────────────

/// Virtual node — either a real graph node or a dummy inserted for edge routing.
///
/// During layout, skip-level edges (spanning more than one level) are broken
/// into segments by inserting dummy nodes at each intermediate level. This
/// allows crossing reduction and x-coordinate assignment to treat all nodes
/// uniformly.
#[derive(Clone, Copy)]
pub(crate) enum VNode {
    /// A real node from the input graph, identified by its index in `Graph.nodes`.
    Real(usize),
    /// A dummy node inserted on a skip-level edge, identified by the edge index.
    Dummy { edge_idx: usize },
}

impl VNode {
    fn real_index(&self) -> Option<usize> {
        match self {
            VNode::Real(idx) => Some(*idx),
            VNode::Dummy { .. } => None,
        }
    }

    fn dummy_edge(&self) -> Option<usize> {
        match self {
            VNode::Real(_) => None,
            VNode::Dummy { edge_idx } => Some(*edge_idx),
        }
    }
}

// ── Main layout entry point ──────────────────────────────────────────────

/// Compute the heap-based layout IR for a DAG using a [`LayoutConfig`].
///
/// This is the implementation behind `Graph::compute_layout()`. Returns a
/// renderer-agnostic `LayoutIR` containing node positions, edge routes,
/// and dimensional information.
/// Compute the layout under axis profile `A` (temp/08 D1). The body
/// currently spells the roles as y/x — LR-P0 threads `A` inward slice
/// by slice, each slice gated on byte-identical TD/BT output.
pub(crate) fn compute_layout_cfg<'a, A: Axis>(
    dag: &Graph<'a>,
    config: &LayoutConfig<'_>,
) -> LayoutIR<'a> {
    if dag.nodes.is_empty() {
        return LayoutIRBuilder::new().build();
    }

    // Honor spacing config (previously ignored; see 0.10.0 changelog).
    let node_spacing = config.node_spacing;
    let level_spacing = config.level_spacing;

    // Cycle breaking: dispatch based on LayoutConfig.
    // DepthFirst detects back edges via three-color DFS.
    // None treats every edge as forward (caller asserts acyclicity).
    let back_edges = match config.cycle_breaking() {
        CycleBreaking::DepthFirst => dag.detect_back_edges(),
        CycleBreaking::None => vec![false; dag.edges.len()],
    };

    // 2-node-cycle detection in O(E log E): sort edge indices by their
    // normalized endpoint pair, then scan each run for an anti-parallel
    // twin with the opposite back flag. Previously the CSR backend did
    // this with an O(E) scan per straight edge (O(E²) worst case) and
    // the heap backend lacked the feature entirely.
    let edge_in_two_cycle = {
        let mut flags = vec![false; dag.edges.len()];
        let pair_key = |ei: usize| {
            let (f, t, _) = dag.edges[ei];
            if f <= t { (f, t) } else { (t, f) }
        };
        let mut order: Vec<usize> = (0..dag.edges.len()).collect();
        order.sort_unstable_by_key(|&ei| pair_key(ei));

        let mut run_start = 0;
        while run_start < order.len() {
            let mut run_end = run_start + 1;
            while run_end < order.len() && pair_key(order[run_end]) == pair_key(order[run_start]) {
                run_end += 1;
            }
            // Bucket the run by (direction, back-flag); an edge is in a
            // 2-node cycle iff an opposite-direction, opposite-flag twin
            // exists (matches the CSR predicate exactly).
            let mut counts = [[0usize; 2]; 2];
            for &ei in &order[run_start..run_end] {
                let (f, t, _) = dag.edges[ei];
                if f == t {
                    continue; // self-loop
                }
                let dir = usize::from(f > t);
                let back = usize::from(back_edges.get(ei).copied().unwrap_or(false));
                counts[dir][back] += 1;
            }
            for &ei in &order[run_start..run_end] {
                let (f, t, _) = dag.edges[ei];
                if f == t {
                    continue;
                }
                let dir = usize::from(f > t);
                let back = usize::from(back_edges.get(ei).copied().unwrap_or(false));
                if counts[1 - dir][1 - back] > 0 {
                    flags[ei] = true;
                }
            }
            run_start = run_end;
        }
        flags
    };

    // Step 1: Calculate levels, treating back edges as reversed
    let level_data = dag.calculate_levels_with_back_edges(&back_edges);
    let max_level = level_data.iter().map(|(_, l)| *l).max().unwrap_or(0);

    // Create level mapping for real nodes
    let mut node_levels: Vec<usize> = vec![0; dag.nodes.len()];
    for (idx, level) in &level_data {
        node_levels[*idx] = *level;
    }

    // Step 2: Build virtual levels with dummy nodes for skip-level edges
    let mut virtual_levels: Vec<Vec<VNode>> = vec![Vec::new(); max_level + 1];

    // Add real nodes to their levels
    for (idx, level) in &level_data {
        virtual_levels[*level].push(VNode::Real(*idx));
    }

    // Identify skip-level edges and insert dummy nodes
    // For back edges, the layout direction is reversed (to → from in level space)
    for (edge_idx, &(from_id, to_id, _label)) in dag.edges.iter().enumerate() {
        let is_back = back_edges.get(edge_idx).copied().unwrap_or(false);
        if let (Some(from_idx), Some(to_idx)) = (dag.node_index(from_id), dag.node_index(to_id)) {
            // For back edges, layout-direction is reversed
            let (layout_from, layout_to) = if is_back {
                (node_levels[to_idx], node_levels[from_idx])
            } else {
                (node_levels[from_idx], node_levels[to_idx])
            };

            if layout_to > layout_from + 1 {
                // Skip-level edge - insert dummy nodes at intermediate levels
                for level in (layout_from + 1)..layout_to {
                    virtual_levels[level].push(VNode::Dummy { edge_idx });
                }
            }
        }
    }

    // Step 3: Apply crossing reduction WITH dummy nodes included
    reduce_crossings_virtual(
        dag,
        &mut virtual_levels,
        &node_levels,
        max_level,
        config.crossing_pipeline(),
    );

    // Step 3b: Block-partitioned level ordering for subgraph adjacency
    if dag.has_subgraphs() {
        use crate::algorithms::sugiyama::subgraph::block_partition_level;
        for level in virtual_levels.iter_mut() {
            *level = block_partition_level(dag, level);
        }
    }

    // Self-loop flags in one O(E) pass (was a full edge scan per node —
    // O(N·E), the dominant cost on large fan-in graphs).
    let mut node_has_self_loop = vec![false; dag.nodes.len()];
    for &(f, t, _) in &dag.edges {
        if f == t {
            if let Some(idx) = dag.node_index(f) {
                node_has_self_loop[idx] = true;
            }
        }
    }

    // Step 4: Assign x-coordinates to virtual nodes
    let mut x_coords: Vec<Vec<usize>> = Vec::with_capacity(virtual_levels.len());
    let mut widths: Vec<Vec<usize>> = Vec::with_capacity(virtual_levels.len());

    for level_vnodes in &virtual_levels {
        let mut level_x = Vec::with_capacity(level_vnodes.len());
        let mut level_w = Vec::with_capacity(level_vnodes.len());
        let mut x = 0;

        for vnode in level_vnodes {
            let width = match vnode {
                // Cross-axis extent (Vertical: the node's width).
                VNode::Real(idx) => {
                    let ext = A::cross_extent(dag.get_node_width(*idx), dag.get_node_height(*idx));
                    // D5(ii): at `node_spacing == 0` a self-loop node's
                    // packed extent reserves its marker cell, so no
                    // downstream pass can place the next node on it.
                    // Inert at spacing ≥ 1 (the gap already hosts it).
                    if node_spacing == 0 && node_has_self_loop[*idx] {
                        ext + 1
                    } else {
                        ext
                    }
                }
                VNode::Dummy { .. } => A::DUMMY_CROSS,
            };
            level_x.push(x);
            level_w.push(width);
            x += width + node_spacing;
        }

        x_coords.push(level_x);
        widths.push(level_w);
    }

    // Insert extra horizontal padding at subgraph boundary transitions
    if dag.has_subgraphs() {
        crate::algorithms::sugiyama::subgraph::subgraph_padding::<A>(
            dag,
            &virtual_levels,
            &mut x_coords,
            &widths,
            node_spacing,
        );
    }

    // Step 4b: Refine x-coordinates — shift nodes toward their connected
    // neighbors on adjacent levels to reduce zigzag edges (median placement).
    // Then compact subgraphs. Run iteratively: compaction moves what it can,
    // x-refinement and subgraph compaction (iterative).
    // x-refinement is only beneficial for subgraph layouts; skipping it for
    // plain graphs avoids an O(N²/L) cost on large inputs.
    if dag.has_subgraphs() {
        let node_edge_indices_for_refine = build_node_edge_indices(dag);
        let compact_rounds = 3;
        for _ in 0..compact_rounds {
            refine_x_positions::<A>(
                dag,
                &virtual_levels,
                &mut x_coords,
                &widths,
                &node_edge_indices_for_refine,
                node_spacing,
            );
            compact_subgraphs::<A>(dag, &virtual_levels, &mut x_coords, &widths, node_spacing);
        }
    }

    // Calculate total width and centering offsets
    let level_widths: Vec<usize> = x_coords
        .iter()
        .zip(widths.iter())
        .map(|(xs, ws)| {
            xs.iter()
                .zip(ws.iter())
                .map(|(x, w)| x + w)
                .max()
                .unwrap_or(0)
        })
        .collect();

    // Cross-axis safety margin: routing/draw offsets, label overhang,
    // cluster borders — all physical-x concerns, so the profile
    // decides how much of it lands on THIS axis (temp/08 P5).
    let has_labeled_edges = dag.edges.iter().any(|(_, _, label)| label.is_some());
    let max_width = level_widths.iter().max().unwrap_or(&0)
        + A::cross_margin(has_labeled_edges, dag.has_subgraphs());

    // Step 5: Build LayoutIR
    let mut builder = LayoutIRBuilder::new().with_levels(max_level + 1);

    // Compute horizontal channel slots to prevent edges from different sources overlapping.
    // We use a hybrid approach:
    // 1. Fan-Out (1 -> Many): All children share the Source's slot (Source Bus).
    // 2. Fan-In (Many -> 1): All parents share the Target's slot (Target Bus) IF In-Degree > 1.

    // Calculate in-degrees to identify Fan-In candidates
    let mut in_degrees = vec![0usize; dag.nodes.len()];
    for &(_, to_id, _) in &dag.edges {
        if let Some(idx) = dag.node_index(to_id) {
            in_degrees[idx] += 1;
        }
    }

    // PRE-CALCULATE COORDINATES: Needed for geometry-aware slot allocation
    // Build lookup: for each real node, find its (level, position, x, width)
    let mut real_node_coords: Vec<(usize, usize, usize, usize)> =
        vec![(0, 0, 0, 0); dag.nodes.len()];

    for (lvl_idx, level_vnodes) in virtual_levels.iter().enumerate() {
        for (idx, vnode) in level_vnodes.iter().enumerate() {
            if let VNode::Real(node_idx) = vnode {
                let x = x_coords[lvl_idx][idx];
                let w = widths[lvl_idx][idx];
                real_node_coords[*node_idx] = (lvl_idx, idx, x, w);
            }
        }
    }

    // Apply centering offsets to real node coordinates.
    // When subgraphs are present, skip per-level centering: the median
    // x-assignment already places children near parents, and independent
    // centering of each level destroys that vertical alignment (zigzag).
    let center_levels = !dag.has_subgraphs();
    for (level_idx, level_vnodes) in virtual_levels.iter().enumerate() {
        let level_width = level_widths[level_idx];
        let level_offset = if center_levels && max_width > level_width {
            (max_width - level_width) / 2
        } else {
            0
        };

        for (pos, vnode) in level_vnodes.iter().enumerate() {
            if let VNode::Real(idx) = vnode {
                let x = x_coords[level_idx][pos] + level_offset;
                let width = widths[level_idx][pos];
                real_node_coords[*idx] = (level_idx, pos, x, width);
            }
        }
    }

    // Fix sibling subgraph overlaps that arise from centering (different levels
    // get different centering offsets, which can push bounding boxes together).
    let max_width = if dag.has_subgraphs() {
        let extra = crate::algorithms::sugiyama::subgraph::fix_subgraph_overlaps::<A>(
            dag,
            &mut real_node_coords,
        );
        // Reclaim slack the sibling shifts left behind: pull nodes toward
        // their connected neighbors within current level bounds.
        crate::algorithms::sugiyama::subgraph::tighten_levels::<A>(
            dag,
            &mut real_node_coords,
            node_spacing,
        );
        // Cluster-width feedback: push unaffiliated nodes clear of each
        // cluster's projected border envelope (cross-level extent + label
        // minimum). Runs after overlap repair so it sees the coordinates
        // the bounding boxes will actually be computed from.
        let pushed = crate::algorithms::sugiyama::subgraph::clear_external_overlaps::<A>(
            dag,
            &mut real_node_coords,
            node_spacing,
        );
        // Pull whole root clusters (and loose nodes) back together after
        // the overlap shifts — reclaims the empty gulfs between boxes.
        let reclaimed = crate::algorithms::sugiyama::subgraph::compact_clusters::<A>(
            dag,
            &mut real_node_coords,
            &virtual_levels,
            &mut x_coords,
            node_spacing,
        );
        // Last-resort overlap repair: none of the passes above moves a
        // node with no edges, so compaction clamps can survive to here as
        // overlapping cluster members. Layouts with neither a node
        // overlap nor a leading-pad violation pass through unchanged.
        // Runs BEFORE dummy clearance so waypoints are nudged off the
        // final node positions.
        let widened = crate::algorithms::sugiyama::subgraph::repair_level_overlaps::<A>(
            dag,
            &mut real_node_coords,
            node_spacing,
        );
        // Waypoints must never cross node text (crossing a border renders
        // as a junction and is acceptable; crossing a node is not).
        crate::algorithms::sugiyama::subgraph::nudge_dummies_off_nodes::<A>(
            &virtual_levels,
            &mut x_coords,
            &real_node_coords,
        );
        (max_width + extra + pushed + widened).saturating_sub(reclaimed)
    } else {
        max_width
    };

    let level_flipped = super::ports::level_flipped::<A>(config.direction);

    // A leading-side lateral lane needs a cell BEFORE the node on the
    // cross axis; a node packed at cross 0 has none. One leading cross
    // cell is opened for the whole layout when any end declares such a
    // face — the cross-axis mirror of the rows opened above level 0
    // for upward exits. Zero for every other layout.
    #[allow(unused_mut)] // set only by the ports pass
    let mut cross_extra = 0usize;
    #[cfg(feature = "ports")]
    if !dag.edge_ports.is_empty() {
        use super::ports::{EndRole, Face};
        for (ei, &(from_id, to_id, _)) in dag.edges.iter().enumerate() {
            if from_id == to_id {
                continue;
            }
            let is_back = back_edges.get(ei).copied().unwrap_or(false);
            let (src_side, dst_side) = dag.edge_ports.get(ei).copied().unwrap_or_default();
            let (src_side, dst_side) = if is_back {
                (dst_side, src_side)
            } else {
                (src_side, dst_side)
            };
            if matches!(
                Face::of(src_side, A::FLOW_AXIS, level_flipped, EndRole::Source),
                Face::CrossLeading
            ) || matches!(
                Face::of(dst_side, A::FLOW_AXIS, level_flipped, EndRole::Target),
                Face::CrossLeading
            ) {
                cross_extra = 1;
                break;
            }
        }
    }
    if cross_extra > 0 {
        for coords in &mut real_node_coords {
            coords.2 += cross_extra;
        }
    }

    // Explicit ports on a LEVEL face get POSITIONS along it: the
    // centered, tangent-ordered spread, round-robin beyond capacity —
    // on the layout role's own Auto face and on the opposite face
    // alike (a source's arrive face, a target's leave face: those ends
    // detour around their node below). Lateral faces keep the center
    // line until their routing exists; Auto edges never take a slot —
    // and a graph without declarations skips all of this.
    // Per-edge `(from, to)` cross overrides, `usize::MAX` = none —
    // EMPTY (no allocation) for a graph that declared no port.
    #[allow(unused_mut)] // mutated only by the ports pass
    let mut port_cross: Vec<(usize, usize)> = Vec::new();
    #[cfg(feature = "ports")]
    if !dag.edge_ports.is_empty() {
        use super::ports::{EndRole, Face, FaceRequest, PortSide, assign_level_face_positions};
        port_cross.resize(dag.edges.len(), (usize::MAX, usize::MAX));
        let cross_span = |idx: usize| -> (usize, usize) {
            let (_, _, base, _) = real_node_coords[idx];
            (
                base,
                A::cross_extent(dag.get_node_width(idx), dag.get_node_height(idx)),
            )
        };
        let mut requests: Vec<FaceRequest> = Vec::new();
        // Lateral requests spread along the LEVEL axis: the key is the
        // peer's level, the result the row offset within the node.
        let mut cross_requests: Vec<FaceRequest> = Vec::new();
        for (edge_idx, &(from_id, to_id, _)) in dag.edges.iter().enumerate() {
            if from_id == to_id {
                continue;
            }
            let (Some(from_idx), Some(to_idx)) = (dag.node_index(from_id), dag.node_index(to_id))
            else {
                continue;
            };
            let is_back = back_edges.get(edge_idx).copied().unwrap_or(false);
            let (src, dst) = if is_back {
                (to_idx, from_idx)
            } else {
                (from_idx, to_idx)
            };
            let (src_side, dst_side) = dag.edge_ports.get(edge_idx).copied().unwrap_or_default();
            let (src_side, dst_side) = if is_back {
                (dst_side, src_side)
            } else {
                (src_side, dst_side)
            };
            for (node, peer, side, end) in [
                (src, dst, src_side, EndRole::Source),
                (dst, src, dst_side, EndRole::Target),
            ] {
                if matches!(side, PortSide::Auto) {
                    continue;
                }
                let face = Face::of(side, A::FLOW_AXIS, level_flipped, end);
                if !face.is_level() {
                    cross_requests.push(FaceRequest {
                        node,
                        face,
                        key: node_levels[peer],
                        edge: edge_idx,
                        end,
                    });
                    continue;
                }
                let (peer_base, peer_extent) = cross_span(peer);
                requests.push(FaceRequest {
                    node,
                    face,
                    key: A::cross_center(peer_base, peer_extent),
                    edge: edge_idx,
                    end,
                });
            }
        }
        // `port_cross` holds each end's position ALONG its face: the
        // cross line on a level face, the row offset on a lateral one.
        assign_level_face_positions::<A>(&mut requests, cross_span, |edge, end, cross| match end {
            EndRole::Source => port_cross[edge].0 = cross,
            EndRole::Target => port_cross[edge].1 = cross,
        });
        super::ports::assign_cross_face_positions::<A>(
            &mut cross_requests,
            |idx| {
                (
                    0,
                    A::level_extent(dag.get_node_width(idx), dag.get_node_height(idx)),
                )
            },
            |edge, end, along| match end {
                EndRole::Source => port_cross[edge].0 = along,
                EndRole::Target => port_cross[edge].1 = along,
            },
        );
    }

    let mut node_slots = vec![usize::MAX; dag.nodes.len()];
    let mut edge_slots = vec![0usize; dag.edges.len()];

    let mut level_occupied_slots: Vec<Vec<Vec<(usize, usize)>>> = vec![Vec::new(); max_level + 1];

    // Maximum horizontal routing rows per level.
    // Gives full visual separation for typical fan-in (≤8 sources),
    // and graceful degradation (shared rows) for extreme fan-in.
    const MAX_SLOTS_PER_LEVEL: usize = 8;

    // Arrow-cell reservation: a reversed edge paints ⇡ on the first
    // routing row, directly below its layout-source. Pre-occupy that
    // cell (± ARROW_CELL_PAD) on slot 0 so any horizontal span that
    // would run through the arrowhead is pushed to a deeper slot by
    // the normal interval-collision logic below. Mirrored in the CSR
    // allocator — the two must not drift.
    for (ei, &(from_id, to_id, _)) in dag.edges.iter().enumerate() {
        if from_id == to_id || !back_edges.get(ei).copied().unwrap_or(false) {
            continue;
        }
        if let (Some(_), Some(to_idx)) = (dag.node_index(from_id), dag.node_index(to_id)) {
            let layout_src = to_idx; // back edge: layout flow is to → from
            let (_, _, x, w) = real_node_coords[layout_src];
            let ax = x + w / 2;
            use crate::algorithms::sugiyama::geometry::ARROW_CELL_PAD;
            let level = node_levels[layout_src];
            let slots = &mut level_occupied_slots[level];
            if slots.is_empty() {
                slots.push(Vec::new());
            }
            slots[0].push((ax.saturating_sub(ARROW_CELL_PAD), ax + ARROW_CELL_PAD));
        }
    }

    // Collect dummy node X positions for skip-level edge routing
    let mut dummy_positions: Vec<Vec<(usize, usize)>> = vec![Vec::new(); dag.edges.len()];
    for (level_idx, level_vnodes) in virtual_levels.iter().enumerate() {
        let level_width = level_widths[level_idx];
        // Apply the same centering policy as real nodes: skip when subgraphs
        // are present to avoid misalignment between real and dummy positions.
        let level_offset = if center_levels && max_width > level_width {
            (max_width - level_width) / 2
        } else {
            0
        };

        for (pos, vnode) in level_vnodes.iter().enumerate() {
            if let VNode::Dummy { edge_idx } = vnode {
                // Get the actual x-coordinate from the layout, including centering offset
                let base_x = x_coords[level_idx][pos] + level_offset;
                // Add bounded offset for visual separation between skip-level edges
                // This keeps convergent edges from merging visually
                let edge_offset = A::dummy_draw_offset(*edge_idx);
                let x = base_x + edge_offset + cross_extra;
                dummy_positions[*edge_idx].push((level_idx, x));
            }
        }
    }

    // Sort dummy positions by level for each edge (they should already be in order, but ensure it)
    for positions in &mut dummy_positions {
        positions.sort_by_key(|(level, _)| *level);
    }

    // temp/09 P3: allocate each skip-edge chain a lane clear of the fans
    // its gaps sweep. The canvas must cover any lane past the packed
    // extent — the flip reflects around the canvas width, so an extent it
    // cannot see would skew RightLeft instead of mirroring it (§4.8).
    let lane_reach = allocate_chain_lanes::<A>(
        dag,
        &virtual_levels,
        &real_node_coords,
        &back_edges,
        &mut dummy_positions,
    );
    let max_width = (max_width + cross_extra).max(lane_reach);

    // Detour ends: an explicit side on the level face OPPOSITE the
    // layout role's own, or on a lateral face. Decided per end here, in
    // the order the facts allow — faces, then lanes (an end without a
    // lane attaches head-on after all), then the EFFECTIVE occupancy,
    // then the lateral and level-face conflicts, then the arrow-cell
    // reservations — so no later fallback lands on a cell a conflict
    // pass already settled. Empty when no port is declared.
    #[cfg_attr(not(feature = "ports"), allow(unused_mut, unused_variables))]
    let mut detour_ends: Vec<(bool, bool)> = Vec::new();
    #[cfg_attr(not(feature = "ports"), allow(unused_mut, unused_variables))]
    let mut detour_faces: Vec<(super::ports::Face, super::ports::Face)> = Vec::new();
    #[cfg_attr(not(feature = "ports"), allow(unused_mut))]
    let mut detours: Vec<super::ports::Detour> = Vec::new();
    #[cfg(feature = "ports")]
    if !dag.edge_ports.is_empty() {
        use super::ports::{
            Detour, EndRole, Face, NODE_DST_CANCELLED, NODE_LOOP, NODE_SRC_CANCELLED, choose_lane,
            detours as end_detours, lateral_key, lateral_lane,
        };
        use crate::algorithms::sugiyama::geometry::ARROW_CELL_PAD;
        let layout_ends = |ei: usize| -> Option<(usize, usize)> {
            let (from_id, to_id, _) = dag.edges[ei];
            if from_id == to_id {
                return None;
            }
            let (from_idx, to_idx) = (dag.node_index(from_id)?, dag.node_index(to_id)?);
            Some(if back_edges.get(ei).copied().unwrap_or(false) {
                (to_idx, from_idx)
            } else {
                (from_idx, to_idx)
            })
        };
        detour_ends.resize(dag.edges.len(), (false, false));
        detour_faces.resize(dag.edges.len(), (Face::LevelLeading, Face::LevelLeading));
        for ei in 0..dag.edges.len() {
            if layout_ends(ei).is_none() {
                continue;
            }
            let is_back = back_edges.get(ei).copied().unwrap_or(false);
            let (src_side, dst_side) = dag.edge_ports.get(ei).copied().unwrap_or_default();
            let (src_side, dst_side) = if is_back {
                (dst_side, src_side)
            } else {
                (src_side, dst_side)
            };
            detour_faces[ei] = (
                Face::of(src_side, A::FLOW_AXIS, level_flipped, EndRole::Source),
                Face::of(dst_side, A::FLOW_AXIS, level_flipped, EndRole::Target),
            );
            detour_ends[ei] = (
                end_detours(src_side, A::FLOW_AXIS, level_flipped, EndRole::Source),
                end_detours(dst_side, A::FLOW_AXIS, level_flipped, EndRole::Target),
            );
        }
        if detour_ends.iter().any(|&(s, d)| s || d) {
            detours.resize(dag.edges.len(), Detour::NONE);
            let span = |idx: usize| -> (usize, usize) {
                let (_, _, base, _) = real_node_coords[idx];
                (
                    base,
                    A::cross_extent(dag.get_node_width(idx), dag.get_node_height(idx)),
                )
            };
            let center = |idx: usize| {
                let (b, e) = span(idx);
                A::cross_center(b, e)
            };
            let level_extent =
                |idx: usize| A::level_extent(dag.get_node_width(idx), dag.get_node_height(idx));
            let resolved = |pc: &[(usize, usize)], ei: usize, idx: usize, target: bool| -> usize {
                let positioned = pc
                    .get(ei)
                    .map_or(usize::MAX, |p| if target { p.1 } else { p.0 });
                if positioned != usize::MAX {
                    positioned
                } else {
                    center(idx)
                }
            };
            // The row a lateral end takes along its side face.
            let side_row = |pc: &[(usize, usize)], ei: usize, idx: usize, target: bool| -> usize {
                let along = pc
                    .get(ei)
                    .map_or(usize::MAX, |p| if target { p.1 } else { p.0 });
                if along == usize::MAX {
                    A::level_center(0, level_extent(idx))
                } else {
                    along
                }
            };
            // Per level, the cross intervals a lane may not touch —
            // `(lo, hi, marker)`: node spans (a zero-extent node occupies
            // no cell) and dummy columns block every row; a self-loop
            // marker cell only its own top row. Sorted by `lo`, spans
            // merged, so a query is one predecessor lookup.
            let mut level_blockers: Vec<Vec<(usize, usize, bool)>> =
                vec![Vec::new(); max_level + 1];
            for idx in 0..dag.nodes.len() {
                let (level, _, base, _) = real_node_coords[idx];
                let (_, ext) = span(idx);
                if ext > 0 {
                    level_blockers[level].push((base, base + ext - 1, false));
                }
                if node_has_self_loop[idx] {
                    level_blockers[level].push((base + ext, base + ext, true));
                }
            }
            for chain in &dummy_positions {
                for &(level, x) in chain {
                    level_blockers[level].push((x, x, false));
                }
            }
            for blockers in &mut level_blockers {
                blockers.sort_unstable();
                let mut merged: Vec<(usize, usize, bool)> = Vec::with_capacity(blockers.len());
                for &(lo, hi, marker) in blockers.iter() {
                    match merged.last_mut() {
                        Some(last) if !marker && !last.2 && lo <= last.1 => last.1 = last.1.max(hi),
                        _ => merged.push((lo, hi, marker)),
                    }
                }
                *blockers = merged;
            }
            // `Some(true)`: only a marker cell covers `col`; `Some(false)`:
            // a span or dummy does.
            let blocked_kind = |level: usize, col: usize| -> Option<bool> {
                let v = &level_blockers[level];
                let i = v.partition_point(|&(lo, _, _)| lo <= col);
                let mut hit = None;
                for k in i.saturating_sub(2)..i {
                    let (lo, hi, marker) = v[k];
                    if lo <= col && col <= hi {
                        if !marker {
                            return Some(false);
                        }
                        hit = Some(true);
                    }
                }
                hit
            };
            // 1. Lanes. An end without one attaches head-on after all —
            // on its role's own face, at the center.
            for ei in 0..dag.edges.len() {
                let (src_d, dst_d) = detour_ends[ei];
                if !(src_d || dst_d) {
                    continue;
                }
                let Some((src, dst)) = layout_ends(ei) else {
                    continue;
                };
                let mut det = Detour::NONE;
                (det.src_face, det.dst_face) = detour_faces[ei];
                (det.src_wants, det.dst_wants) = detour_ends[ei];
                for (is_target, node, peer) in [(false, src, dst), (true, dst, src)] {
                    let (wants, face) = if is_target {
                        (det.dst_wants, det.dst_face)
                    } else {
                        (det.src_wants, det.src_face)
                    };
                    if !wants {
                        continue;
                    }
                    let (base, ext) = span(node);
                    let level = node_levels[node];
                    let lane =
                        if face.is_level() {
                            // Around the node: the lane passes every row, so
                            // a marker cell blocks it like a span.
                            choose_lane(
                                base,
                                ext,
                                node_has_self_loop[node],
                                center(peer) > center(node),
                                max_width,
                                &|c| blocked_kind(level, c).is_some(),
                            )
                        } else {
                            // Beside the node: the stub runs at its own row,
                            // so a marker cell blocks it only on the top row.
                            let top_row = side_row(&port_cross, ei, node, is_target) == 0;
                            lateral_lane(base, ext, face, max_width, &|c| match blocked_kind(
                                level, c,
                            ) {
                                None => false,
                                Some(true) => top_row,
                                Some(false) => true,
                            })
                        };
                    if lane == usize::MAX {
                        if is_target {
                            det.dst_wants = false;
                            detour_ends[ei].1 = false;
                            port_cross[ei].1 = usize::MAX;
                        } else {
                            det.src_wants = false;
                            detour_ends[ei].0 = false;
                            port_cross[ei].0 = usize::MAX;
                        }
                    } else if is_target {
                        det.dst_lane = lane;
                    } else {
                        det.src_lane = lane;
                    }
                }
                detours[ei] = det;
            }
            // 2. The effective occupancy: the detouring nodes with their
            // flags, and the head-on records `(node, role, cell)` — every
            // end AT those nodes that attaches head-on (lane-less ends
            // included), plus the lateral source exits keyed by face —
            // sorted once, so a query is a binary search.
            let mut node_marks = vec![false; dag.nodes.len()];
            for ei in 0..dag.edges.len() {
                let Some((src, dst)) = layout_ends(ei) else {
                    continue;
                };
                if detours[ei].src_lane != usize::MAX {
                    node_marks[src] = true;
                }
                if detours[ei].dst_lane != usize::MAX {
                    node_marks[dst] = true;
                }
            }
            let mut detour_nodes: Vec<(usize, u8)> = node_marks
                .iter()
                .enumerate()
                .filter(|(_, m)| **m)
                .map(|(n, _)| (n, if node_has_self_loop[n] { NODE_LOOP } else { 0 }))
                .collect();
            let mut head_on: Vec<(usize, u8, usize)> = Vec::new();
            for ei in 0..dag.edges.len() {
                let Some((src, dst)) = layout_ends(ei) else {
                    continue;
                };
                let det = detours[ei];
                if node_marks[src] && det.src_lane == usize::MAX {
                    head_on.push((src, 0, resolved(&port_cross, ei, src, false)));
                }
                if node_marks[dst] && det.dst_lane == usize::MAX {
                    head_on.push((dst, 1, resolved(&port_cross, ei, dst, true)));
                }
                if det.src_lane != usize::MAX && !det.src_face.is_level() {
                    head_on.push((
                        src,
                        lateral_key(det.src_face) as u8,
                        side_row(&port_cross, ei, src, false),
                    ));
                }
            }
            head_on.sort_unstable();
            let is_head_on = |nodes: &[(usize, u8)], node: usize, role: u8, cell: usize| {
                if head_on.binary_search(&(node, role, cell)).is_ok() {
                    return true;
                }
                // Cancelled detours attach at the Auto face's center — a
                // level-face fact (roles 0 and 1 only).
                if role > 1 {
                    return false;
                }
                let bit = if role == 0 {
                    NODE_SRC_CANCELLED
                } else {
                    NODE_DST_CANCELLED
                };
                cell == center(node)
                    && nodes
                        .binary_search_by_key(&node, |e| e.0)
                        .is_ok_and(|i| nodes[i].1 & bit != 0)
            };
            // 3. Conflicts. A lateral TARGET yields to a lateral source
            // exit on the same side cell (shifts a row, or attaches
            // head-on); a detouring end on a level face must not share
            // its cell with a head-on end of the other role (shifts a
            // cell along the face, or attaches head-on). A cancelled end
            // gives up its lane and counts as head-on at the center.
            for ei in 0..dag.edges.len() {
                let mut det = detours[ei];
                if !det.active() {
                    continue;
                }
                let Some((src, dst)) = layout_ends(ei) else {
                    continue;
                };
                for (is_target, node) in [(false, src), (true, dst)] {
                    let (lane, face) = if is_target {
                        (det.dst_lane, det.dst_face)
                    } else {
                        (det.src_lane, det.src_face)
                    };
                    if lane == usize::MAX {
                        continue;
                    }
                    let mut cancel = false;
                    if !face.is_level() {
                        if !is_target {
                            continue;
                        }
                        let key = lateral_key(face) as u8;
                        let h = level_extent(node);
                        let row = side_row(&port_cross, ei, node, true);
                        if !is_head_on(&detour_nodes, node, key, row) {
                            continue;
                        }
                        // A trailing-side stub on a self-loop node's top
                        // row would leave through the `↺` cell.
                        let marker_row = |r: usize| {
                            r == 0
                                && matches!(face, Face::CrossTrailing)
                                && node_has_self_loop[node]
                        };
                        let shifted = [row + 1, row.wrapping_sub(1)].into_iter().find(|&r| {
                            r < h && !marker_row(r) && !is_head_on(&detour_nodes, node, key, r)
                        });
                        match shifted {
                            Some(r) => port_cross[ei].1 = r,
                            None => cancel = true,
                        }
                    } else {
                        let other: u8 = if is_target { 0 } else { 1 };
                        let cell = resolved(&port_cross, ei, node, is_target);
                        if !is_head_on(&detour_nodes, node, other, cell) {
                            continue;
                        }
                        let (base, ext) = span(node);
                        let inside = |c: usize| c >= base && c < base + ext;
                        let shifted = [cell + 1, cell.wrapping_sub(1)]
                            .into_iter()
                            .find(|&c| inside(c) && !is_head_on(&detour_nodes, node, other, c));
                        match shifted {
                            Some(c) => {
                                if is_target {
                                    port_cross[ei].1 = c;
                                } else {
                                    port_cross[ei].0 = c;
                                }
                            }
                            None => cancel = true,
                        }
                    }
                    if cancel {
                        if is_target {
                            det.dst_lane = usize::MAX;
                            det.dst_wants = false;
                            detour_ends[ei].1 = false;
                            port_cross[ei].1 = usize::MAX;
                        } else {
                            det.src_lane = usize::MAX;
                            det.src_wants = false;
                            detour_ends[ei].0 = false;
                            port_cross[ei].0 = usize::MAX;
                        }
                        if let Ok(i) = detour_nodes.binary_search_by_key(&node, |e| e.0) {
                            detour_nodes[i].1 |= if is_target {
                                NODE_DST_CANCELLED
                            } else {
                                NODE_SRC_CANCELLED
                            };
                        }
                    }
                }
                detours[ei] = det;
            }
            // 4. A bottom-face arrival paints its arrowhead on the cell
            // right under the target's port; reserve it (± ARROW_CELL_PAD)
            // on the target level's slot 0, exactly as reversed edges
            // reserve theirs, so no run — this edge's own included —
            // crosses it.
            for ei in 0..dag.edges.len() {
                let det = detours[ei];
                if det.dst_lane == usize::MAX || !det.dst_face.is_level() {
                    continue;
                }
                let Some((_, dst)) = layout_ends(ei) else {
                    continue;
                };
                let ax = resolved(&port_cross, ei, dst, true);
                let slots = &mut level_occupied_slots[node_levels[dst]];
                if slots.is_empty() {
                    slots.push(Vec::new());
                }
                slots[0].push((ax.saturating_sub(ARROW_CELL_PAD), ax + ARROW_CELL_PAD));
            }
        }
    }

    // 1. Assign slots greedy — in layout direction, so back edges
    // participate: their own horizontal always covers their own arrow
    // column, so the seeded cell forces them below the arrow row.
    // (Matches the CSR allocator, which already worked in layout space.)
    for (i, &(from_id, to_id, _)) in dag.edges.iter().enumerate() {
        if let (Some(from_idx), Some(to_idx)) = (dag.node_index(from_id), dag.node_index(to_id)) {
            let is_back = back_edges.get(i).copied().unwrap_or(false);
            let (ls_idx, ld_idx) = if is_back {
                (to_idx, from_idx)
            } else {
                (from_idx, to_idx)
            };
            let from_level = node_levels[ls_idx];
            let to_level = node_levels[ld_idx];

            // Get coordinates to determine geometry
            let (_, _, from_x, from_w) = real_node_coords[ls_idx];
            let (_, _, to_x, to_w) = real_node_coords[ld_idx];

            // Calculate interval required for this edge
            let start_x = from_x + from_w / 2;
            let end_x = to_x + to_w / 2;
            let (min_x, max_x) = if start_x < end_x {
                (start_x, end_x)
            } else {
                (end_x, start_x)
            };

            if to_level > from_level {
                let is_vertical = min_x == max_x && to_level == from_level + 1;

                // Source-based slot assignment (unified for all edges)
                // Each source node gets a unique horizontal slot at its level,
                // matching arena layout for visual parity.
                if is_vertical {
                    edge_slots[i] = usize::MAX;
                } else {
                    if node_slots[ls_idx] != usize::MAX {
                        // Reuse existing slot for this source node
                        let slot = node_slots[ls_idx];
                        edge_slots[i] = slot;

                        // Mark interval as occupied (Merge)
                        if slot < level_occupied_slots[from_level].len() {
                            let intervals = &mut level_occupied_slots[from_level][slot];
                            let mut merged = false;
                            if let Some(last) = intervals.last_mut() {
                                if min_x <= last.1 && max_x >= last.0 {
                                    last.0 = last.0.min(min_x);
                                    last.1 = last.1.max(max_x);
                                    merged = true;
                                }
                            }
                            if !merged {
                                intervals.push((min_x, max_x));
                            }
                        }
                    } else {
                        let slots = &mut level_occupied_slots[from_level];
                        let mut chosen_slot = None;

                        for (s_idx, occupied) in slots.iter_mut().enumerate() {
                            if let Some(last) = occupied.last() {
                                if min_x >= last.1 {
                                    occupied.push((min_x, max_x));
                                    chosen_slot = Some(s_idx);
                                    break;
                                }
                            }

                            let collide = occupied.iter().any(|&(s, e)| s < max_x && e > min_x);
                            if !collide {
                                occupied.push((min_x, max_x));
                                chosen_slot = Some(s_idx);
                                break;
                            }
                        }

                        let slot = if let Some(s) = chosen_slot {
                            s
                        } else if slots.len() < MAX_SLOTS_PER_LEVEL {
                            slots.push(vec![(min_x, max_x)]);
                            slots.len() - 1
                        } else {
                            // Cap reached: reuse slot 0 to bound level height.
                            // For extreme fan-in (e.g. 50k→1) all intervals
                            // overlap at the target X anyway, so visual overlap
                            // is unavoidable and this keeps output bounded.
                            slots[0].push((min_x, max_x));
                            0
                        };

                        node_slots[ls_idx] = slot;
                    }
                    edge_slots[i] = node_slots[ls_idx];
                }
            }
        }
    }

    // Jog-aware dummy rows: a waypoint claims a routing row only where the
    // edge actually changes column — its x differs from the NEXT chain x
    // (next waypoint, or the layout-target center), because the bend to a
    // new column is painted right below the kept row. Straight pass-through
    // dummies keep their reserved column in the level packing but need no
    // routing row of their own. Mirrored in the CSR backend.
    let mut kept_wps: Vec<Vec<bool>> = Vec::with_capacity(dag.edges.len());
    let mut level_jog_count = vec![0usize; max_level + 1];
    // Jog bend rows share the band's slot rows (bend `k` of a level
    // paints on slot index `1 + label row + k`): with detours in play
    // their intervals are recorded so detour runs are allocated clear
    // of them. Per level: `(bend counter, min, max)` — the label row
    // is known only after the flags pass, so the allocator adds it.
    // Nothing is allocated for a layout without detours.
    let mut jog_blocks: Vec<Vec<(usize, usize, usize)>> = if detours.is_empty() {
        Vec::new()
    } else {
        vec![Vec::new(); max_level + 1]
    };
    for (edge_idx, chain) in dummy_positions.iter().enumerate() {
        let mut kept = vec![false; chain.len()];
        if !chain.is_empty() {
            let &(from_id, to_id, _) = &dag.edges[edge_idx];
            let is_back = back_edges.get(edge_idx).copied().unwrap_or(false);
            if let (Some(from_idx), Some(to_idx)) = (dag.node_index(from_id), dag.node_index(to_id))
            {
                let layout_dst = if is_back { from_idx } else { to_idx };
                // The chain's entry column: the lane when the target
                // detours, else the RESOLVED port (a positioned
                // explicit request, or the center) — never the bare
                // center, or a spread port would leave the last jog
                // without a budgeted row.
                let target_x = match detours.get(edge_idx) {
                    Some(d) if d.dst_lane != usize::MAX => d.dst_lane,
                    _ => {
                        let positioned = port_cross.get(edge_idx).map_or(usize::MAX, |p| p.1);
                        if positioned != usize::MAX {
                            positioned
                        } else {
                            let (_, _, dx, dw) = real_node_coords[layout_dst];
                            dx + dw / 2
                        }
                    }
                };
                for i in 0..chain.len() {
                    let next_x = if i + 1 < chain.len() {
                        chain[i + 1].1
                    } else {
                        target_x
                    };
                    if chain[i].1 != next_x {
                        kept[i] = true;
                        let lvl = chain[i].0;
                        let k = level_jog_count[lvl];
                        level_jog_count[lvl] += 1;
                        if !detours.is_empty() {
                            let (x, nx) = (chain[i].1, next_x);
                            jog_blocks[lvl].push((k, x.min(nx), x.max(nx)));
                        }
                    }
                }
            }
        }
        kept_wps.push(kept);
    }

    // Per-level label-source flags: the label row is budgeted only in the
    // bands of levels that actually source a labeled edge (labels paint in
    // the layout-source's band). Mirrored in the CSR backend.
    let mut level_labeled_src = vec![false; max_level + 1];
    for (ei, &(from_id, to_id, label)) in dag.edges.iter().enumerate() {
        if label.is_none() || from_id == to_id {
            continue;
        }
        if let (Some(from_idx), Some(to_idx)) = (dag.node_index(from_id), dag.node_index(to_id)) {
            let is_back = back_edges.get(ei).copied().unwrap_or(false);
            let layout_src = if is_back { to_idx } else { from_idx };
            level_labeled_src[node_levels[layout_src]] = true;
        }
    }

    // Detour rows: every detour run gets a slot with its EXACT interval
    // — never the source's fan-out bus, whose registered extent stops
    // at the targets' centers. The up-run above a level-0 source lives
    // in rows above the first level (`top_slots`).
    #[cfg_attr(not(feature = "ports"), allow(unused_mut))]
    let mut top_slots: Vec<Vec<(usize, usize)>> = Vec::new();
    #[cfg(feature = "ports")]
    if !detours.is_empty() {
        let resolved = |ei: usize, idx: usize, end_is_target: bool| -> usize {
            let positioned = port_cross
                .get(ei)
                .map_or(usize::MAX, |p| if end_is_target { p.1 } else { p.0 });
            if positioned != usize::MAX {
                positioned
            } else {
                let (_, _, base, _) = real_node_coords[idx];
                A::cross_center(
                    base,
                    A::cross_extent(dag.get_node_width(idx), dag.get_node_height(idx)),
                )
            }
        };
        // The greedy pass registered each bus run as the CENTERS span,
        // which understates a skip edge's first run (it reaches the
        // chain's first jogging column) and a spread port's. Detour
        // runs must see the TRUE extents, so register them here — for
        // graphs with detours only, so undeclared layouts keep their
        // bytes.
        for ei in 0..dag.edges.len() {
            if detours[ei].active() {
                continue;
            }
            let (from_id, to_id, _) = dag.edges[ei];
            if from_id == to_id {
                continue;
            }
            let is_back = back_edges.get(ei).copied().unwrap_or(false);
            let (Some(from_idx), Some(to_idx)) = (dag.node_index(from_id), dag.node_index(to_id))
            else {
                continue;
            };
            let (src, dst) = if is_back {
                (to_idx, from_idx)
            } else {
                (from_idx, to_idx)
            };
            let from_x = resolved(ei, src, false);
            let first_target = dummy_positions[ei]
                .iter()
                .zip(&kept_wps[ei])
                .find(|(_, k)| **k)
                .map_or_else(|| resolved(ei, dst, true), |(&(_, x), _)| x);
            if from_x == first_target {
                continue;
            }
            let level = node_levels[src];
            let slot = if node_slots[src] != usize::MAX {
                node_slots[src]
            } else {
                0
            };
            let slots = &mut level_occupied_slots[level];
            while slots.len() <= slot {
                slots.push(Vec::new());
            }
            slots[slot].push((from_x.min(first_target), from_x.max(first_target)));
        }
        for ei in 0..dag.edges.len() {
            let mut det = detours[ei];
            if !det.active() {
                continue;
            }
            let (from_id, to_id, _) = dag.edges[ei];
            let is_back = back_edges.get(ei).copied().unwrap_or(false);
            let (Some(from_idx), Some(to_idx)) = (dag.node_index(from_id), dag.node_index(to_id))
            else {
                continue;
            };
            let (src, dst) = if is_back {
                (to_idx, from_idx)
            } else {
                (from_idx, to_idx)
            };
            let (src_level, dst_level) = (node_levels[src], node_levels[dst]);
            let from_x = resolved(ei, src, false);
            let to_x = resolved(ei, dst, true);
            let minmax = |a: usize, b: usize| (a.min(b), a.max(b));
            let mut col = from_x;
            if det.src_lane != usize::MAX {
                // An opposite-face exit needs its up-run row; a lateral
                // stub runs at the node's own row.
                if det.src_face.is_level() {
                    let (lo, hi) = minmax(from_x, det.src_lane);
                    det.up_slot = if src_level == 0 {
                        alloc_slot(&mut top_slots, &[], false, lo, hi)
                    } else {
                        alloc_slot(
                            &mut level_occupied_slots[src_level - 1],
                            &jog_blocks[src_level - 1],
                            level_labeled_src[src_level - 1],
                            lo,
                            hi,
                        )
                    };
                }
                col = det.src_lane;
            }
            let final_col = if det.dst_lane != usize::MAX {
                det.dst_lane
            } else {
                to_x
            };
            let first_target = dummy_positions[ei]
                .iter()
                .zip(&kept_wps[ei])
                .find(|(_, k)| **k)
                .map_or(final_col, |(&(_, x), _)| x);
            if col != first_target {
                let (lo, hi) = minmax(col, first_target);
                det.first_slot = alloc_slot(
                    &mut level_occupied_slots[src_level],
                    &jog_blocks[src_level],
                    level_labeled_src[src_level],
                    lo,
                    hi,
                );
            }
            if det.dst_lane != usize::MAX && det.dst_face.is_level() {
                let (lo, hi) = minmax(det.dst_lane, to_x);
                det.below_slot = alloc_slot(
                    &mut level_occupied_slots[dst_level],
                    &jog_blocks[dst_level],
                    level_labeled_src[dst_level],
                    lo,
                    hi,
                );
            }
            detours[ei] = det;
        }
    }
    let detour_of = |ei: usize| -> Option<super::ports::Detour> {
        detours.get(ei).copied().filter(|d| d.active())
    };
    // Rows above the first level for its upward exits: one per slot
    // plus the clearance line before the nodes (the mirror of the
    // routing block below a level).
    let top_extra = if top_slots.is_empty() {
        0
    } else {
        crate::algorithms::sugiyama::geometry::EDGE_START_OFFSET + top_slots.len()
    };

    // Calculate per-level heights: node height + routing overhead + extra rows for slot separation
    // Compute max node height per level from actual node heights
    let mut max_node_height: Vec<usize> = vec![1; max_level + 1];
    for (level, level_vnodes) in virtual_levels.iter().enumerate() {
        for vnode in level_vnodes {
            if let VNode::Real(idx) = vnode {
                // Level-axis extent (Vertical: the node's height).
                let node_height =
                    A::level_extent(dag.get_node_width(*idx), dag.get_node_height(*idx));
                if node_height > max_node_height[level] {
                    max_node_height[level] = node_height;
                }
            }
        }
    }
    let mut level_offsets = Vec::with_capacity(max_level + 1);

    // When subgraphs exist, compute per-boundary extra rows for opening/closing borders
    let (sg_initial_offset, sg_boundary_extras, sg_trailing_extra) = if dag.has_subgraphs() {
        crate::algorithms::sugiyama::subgraph::compute_level_extras::<A>(
            dag,
            &node_levels,
            max_level,
        )
    } else {
        (0, vec![0; max_level + 1], 0)
    };

    #[cfg_attr(not(feature = "ports"), allow(unused_variables))]
    let top_base = sg_initial_offset;
    let mut current_offset = sg_initial_offset + top_extra;
    // D8(b) bookkeeping only exists where labels claim level-axis room
    // (Horizontal with subgraphs) — the Vertical/no-cluster hot path
    // allocates and traverses nothing extra.
    let track_label_extras = A::LABEL_CLAIMS_LEVEL_AXIS && dag.has_subgraphs();
    let mut level_heights = Vec::new();
    if track_label_extras {
        level_heights.reserve(max_level + 1);
    }

    for level in 0..=max_level {
        level_offsets.push(current_offset);

        // 1. Slots for edges originating at this level (adjacent or skip)
        let adjacent_slots = level_occupied_slots[level].len();

        // 2. Rows for edges passing through: only jogging waypoints claim
        // a row (straight pass-throughs are pure verticals), plus the
        // bend row below the deepest jog (shared rule with CSR).
        let skip_slots =
            crate::algorithms::sugiyama::geometry::passthrough_extent(level_jog_count[level]);

        // Determine max slots needed for this specific level
        let slots_needed = adjacent_slots.max(skip_slots);
        let extra_lines = slots_needed.saturating_sub(1);

        // Per-level overhead: the label row is budgeted only where a
        // labeled edge is sourced (shared rule with the CSR backend).
        let routing_overhead =
            crate::algorithms::sugiyama::geometry::routing_overhead(level_labeled_src[level]);
        let height =
            max_node_height[level] + routing_overhead + extra_lines + sg_boundary_extras[level];
        if track_label_extras {
            level_heights.push(height);
        }
        current_offset += height;
        // Extra vertical gap between levels only — not after the last one,
        // which would pad the bottom of the canvas with blank rows.
        if level < max_level {
            current_offset += level_spacing;
        }
    }

    // Total height: current_offset already includes all subgraph border spacing
    // plus trailing extra for subgraphs closing after the last level
    let total_height = current_offset + sg_trailing_extra;

    // D8(b) second phase: reserve label room on the LEVEL axis
    // (Horizontal-only — every extra is 0 under Vertical and the
    // rebuild is skipped, keeping the frozen path untouched).
    let (level_offsets, total_height) = if track_label_extras {
        let label_extras = crate::algorithms::sugiyama::subgraph::compute_label_level_extras::<A>(
            dag,
            &node_levels,
            &level_offsets,
            &max_node_height,
            max_level,
        );
        if label_extras.iter().any(|&e| e > 0) {
            let mut offsets = Vec::with_capacity(max_level + 1);
            let mut off = sg_initial_offset + top_extra;
            for level in 0..=max_level {
                offsets.push(off);
                off += level_heights[level] + label_extras[level];
                if level < max_level {
                    off += level_spacing;
                }
            }
            (offsets, off + sg_trailing_extra)
        } else {
            (level_offsets, total_height)
        }
    } else {
        (level_offsets, total_height)
    };

    // Add real nodes to IR, remembering each graph node's IR position
    // (level-order emission) so self-loop records can carry it — the
    // O(1) record→node join every consumer relies on.
    let mut ir_index_of = vec![usize::MAX; dag.nodes.len()];
    let mut next_ir_index = 0usize;
    for (level_idx, level_vnodes) in virtual_levels.iter().enumerate() {
        for vnode in level_vnodes {
            if let VNode::Real(idx) = vnode {
                ir_index_of[*idx] = next_ir_index;
                next_ir_index += 1;
                let (level, pos, cross, _) = real_node_coords[*idx];
                let (x, y) = A::materialize(level_offsets[level], cross);

                let (id, label) = dag.nodes[*idx];
                let kind = if dag.auto_created.contains(&id) {
                    NodeKind::Implicit
                } else {
                    NodeKind::Explicit
                };
                // Physical IR extents come from the node's declared
                // dimensions — the packed tuple's extent is the role-space
                // cross extent (== width only in Vertical).
                let node_width = dag.get_node_width(*idx);
                let node_height = dag.get_node_height(*idx);
                // D5: the marker cell is IR geometry — one cell past the
                // node on the cross axis, at its level-leading line
                // (Vertical: right of the top row, the legacy `↺` cell).
                let self_loop_at = node_has_self_loop[*idx].then(|| {
                    A::materialize(
                        level_offsets[level],
                        cross + A::cross_extent(node_width, node_height),
                    )
                });
                builder.add_node(LayoutNode {
                    id,
                    label,
                    y,
                    x,
                    width: node_width,
                    height: node_height,
                    center_x: x + node_width / 2,
                    center_y: y + node_height.saturating_sub(1) / 2,
                    level: level_idx,
                    level_position: pos,
                    kind,
                    has_self_loop: self_loop_at.is_some(),
                    self_loop_at,
                    edge_index: None,
                    content_tag: dag.node_kind_tag[*idx],
                });
                // Carry the node's declared painter/payload (sparse —
                // present only for custom-content nodes).
                if let Ok(pos) = dag.node_custom.binary_search_by_key(idx, |entry| entry.0) {
                    let (_, painter, payload) = dag.node_custom[pos];
                    builder.add_custom_for_last(painter, payload);
                }
            }
        }
    }

    // Emit dummy nodes into the IR (opt-in; zero cost when disabled).
    // Positions use the exact same computation as the waypoint chains, so
    // a dummy node and its edge's waypoint always share a column.
    if config.include_dummy_nodes {
        let mut synthetic = 0usize;
        for (level_idx, level_vnodes) in virtual_levels.iter().enumerate() {
            let level_width = level_widths[level_idx];
            let level_offset = if center_levels && max_width > level_width {
                (max_width - level_width) / 2
            } else {
                0
            };
            for (pos, vnode) in level_vnodes.iter().enumerate() {
                if let VNode::Dummy { edge_idx } = vnode {
                    // §4.8: the allocated waypoint coordinate is canonical —
                    // the routing chain and the emitted dummy marker must
                    // agree, so this is a lookup, never a recomputation.
                    let cross = dummy_positions[*edge_idx]
                        .iter()
                        .find(|&&(l, _)| l == level_idx)
                        .map(|&(_, x)| x)
                        .unwrap_or_else(|| {
                            x_coords[level_idx][pos]
                                + level_offset
                                + A::dummy_draw_offset(*edge_idx)
                        });
                    let (x, y) = A::materialize(level_offsets[level_idx], cross);
                    // Synthetic id, excluded from id_to_index by the builder.
                    let id = usize::MAX - synthetic;
                    synthetic += 1;
                    builder.add_node(LayoutNode {
                        id,
                        label: "",
                        x,
                        y,
                        width: 1,
                        height: 1,
                        center_x: x,
                        center_y: y,
                        level: level_idx,
                        level_position: pos,
                        kind: NodeKind::Dummy,
                        has_self_loop: false,
                        self_loop_at: None,
                        edge_index: Some(*edge_idx),
                        content_tag: 0,
                    });
                }
            }
        }
    }

    let mut level_edge_next = vec![0usize; max_level + 1];

    // Collect all horizontal Y values used by edge routing segments.
    // These will be passed to bounding box computation so subgraph borders
    // can be shifted to avoid overlapping with edge routing rows.
    let mut edge_routing_ys: HashSet<usize> = HashSet::new();

    // Per-level routing floor: the maximum Y used by edge routing at each level.
    // Bottom borders of subgraphs closing at level L must be placed BELOW this floor.
    let mut level_routing_floor: Vec<usize> = vec![0; max_level + 1];

    // Step 6: Add edges with proper routing
    for (edge_idx, &(from_id, to_id, label)) in dag.edges.iter().enumerate() {
        if let (Some(from_idx), Some(to_idx)) = (dag.node_index(from_id), dag.node_index(to_id)) {
            let is_back = back_edges.get(edge_idx).copied().unwrap_or(false);

            // Self-loops: skip layout routing (rendered as ↺ indicator by renderer)
            if from_id == to_id {
                continue;
            }

            // For back edges, layout direction is reversed (to→from in level space).
            // We compute coordinates in layout order, then store semantic IDs in the IR.
            let (layout_src_idx, layout_dst_idx) = if is_back {
                (to_idx, from_idx)
            } else {
                (from_idx, to_idx)
            };
            let layout_from_level = node_levels[layout_src_idx];
            let layout_to_level = node_levels[layout_dst_idx];

            let (_, _, src_x_base, _) = real_node_coords[layout_src_idx];
            let (_, _, dst_x_base, _) = real_node_coords[layout_dst_idx];

            // Attachment resolution (ports sit on the node's DECLARED
            // span — the packed tuple extent may carry the D5(ii)
            // marker reserve). Declared sides bind to declared
            // endpoints, so a reversal swaps the SIDES onto the layout
            // roles; `Auto` binds to the layout role itself.
            #[cfg(feature = "ports")]
            let (src_side, dst_side) = dag.edge_ports.get(edge_idx).copied().unwrap_or_default();
            #[cfg(not(feature = "ports"))]
            let (src_side, dst_side) = (super::ports::PortSide::Auto, super::ports::PortSide::Auto);
            let (layout_src_side, layout_dst_side) = if is_back {
                (dst_side, src_side)
            } else {
                (src_side, dst_side)
            };
            use super::ports::{Attachment, EndRole};
            // A positioned request overrides the center line; anything
            // else (Auto, or a face without routing yet) resolves as
            // before.
            let positioned = port_cross
                .get(edge_idx)
                .copied()
                .unwrap_or((usize::MAX, usize::MAX));
            let from_x = if positioned.0 != usize::MAX {
                positioned.0
            } else {
                Attachment::resolve::<A>(
                    layout_src_side,
                    level_flipped,
                    EndRole::Source,
                    src_x_base,
                    A::cross_extent(
                        dag.get_node_width(layout_src_idx),
                        dag.get_node_height(layout_src_idx),
                    ),
                )
                .cross
            };
            let to_x = if positioned.1 != usize::MAX {
                positioned.1
            } else {
                Attachment::resolve::<A>(
                    layout_dst_side,
                    level_flipped,
                    EndRole::Target,
                    dst_x_base,
                    A::cross_extent(
                        dag.get_node_width(layout_dst_idx),
                        dag.get_node_height(layout_dst_idx),
                    ),
                )
                .cross
            };
            // 2-node cycle sharing a column: offset the forward edge left
            // and the back edge right so the anti-parallel pair renders
            // side by side (↓ next to ⇡) instead of overlapping. Matches
            // the CSR backend.
            // Endpoint-shift separation needs cross-wide nodes
            // (Vertical: every node spans ≥3 columns). Horizontal nodes
            // are typically ONE row tall — shifted endpoints leave the
            // node face, so the pair keeps its shared port and lane
            // separation becomes paint-time work (temp/08 P3).
            // The shift is applied only while BOTH shifted endpoints stay
            // inside their nodes' declared spans: a resolved port on a
            // narrow custom node (0, 1, or 2 cells) keeps its boundary
            // cell rather than being pushed off the face, and the pair
            // then shares the port as horizontal pairs do.
            let (from_x, to_x) = if from_x == to_x
                && from_id != to_id
                && matches!(A::FLOW_AXIS, crate::ir::FlowAxis::Y)
                && edge_in_two_cycle.get(edge_idx).copied().unwrap_or(false)
                && detour_of(edge_idx).is_none()
            {
                let delta: isize = if is_back { 1 } else { -1 };
                let inside = |x: usize, idx: usize| -> Option<usize> {
                    let (_, _, base, _) = real_node_coords[idx];
                    let extent = A::cross_extent(dag.get_node_width(idx), dag.get_node_height(idx));
                    let shifted = x.checked_add_signed(delta)?;
                    (shifted >= base && shifted < base + extent).then_some(shifted)
                };
                match (inside(from_x, layout_src_idx), inside(to_x, layout_dst_idx)) {
                    (Some(f), Some(t)) => (f, t),
                    _ => (from_x, to_x),
                }
            } else {
                (from_x, to_x)
            };
            // Everything below computes in ROLE values (x/`_x` = cross,
            // y/`_y` = level — legacy names); pairs materialize at the
            // `add_edge` literal, and level-axis path scalars stay
            // role-valued with `flow_axis` naming their physical axis
            // (temp/08 D2).
            // The routing band starts after the level's FULL extent;
            // the IR endpoint sits on the source's port line (per-axis:
            // Vertical = band-trailing, Horizontal = own face).
            let band_trailing =
                level_offsets[layout_from_level] + max_node_height[layout_from_level] - 1;
            let from_y = A::source_port_level(
                level_offsets[layout_from_level],
                A::level_extent(
                    dag.get_node_width(layout_src_idx),
                    dag.get_node_height(layout_src_idx),
                ),
                max_node_height[layout_from_level],
            );
            let to_y = level_offsets[layout_to_level];
            // A detouring end attaches on its node's OWN face line — the
            // leading line for an upward exit, the trailing line for an
            // arrival from below — never the band-trailing port line.
            let detour = detour_of(edge_idx);
            // A lateral end sits on the node's OWN side cell: the face
            // column, at the row the spread assigned along the face.
            let lateral = |idx: usize, face: super::ports::Face, along: usize, level: usize| {
                let (_, _, base, _) = real_node_coords[idx];
                let (w, h) = (dag.get_node_width(idx), dag.get_node_height(idx));
                let cross = if matches!(face, super::ports::Face::CrossTrailing) {
                    base + A::cross_extent(w, h).saturating_sub(1)
                } else {
                    base
                };
                let row = if along == usize::MAX {
                    A::level_center(0, A::level_extent(w, h))
                } else {
                    along
                };
                (cross, level_offsets[level] + row)
            };
            let (from_x, from_y, to_x, to_y) = match detour {
                Some(d) => {
                    let (fx, fy) = if d.src_lane == usize::MAX {
                        (from_x, from_y)
                    } else if d.src_face.is_level() {
                        (from_x, level_offsets[layout_from_level])
                    } else {
                        lateral(layout_src_idx, d.src_face, positioned.0, layout_from_level)
                    };
                    let (tx, ty) = if d.dst_lane == usize::MAX {
                        (to_x, to_y)
                    } else if d.dst_face.is_level() {
                        (
                            to_x,
                            level_offsets[layout_to_level]
                                + A::level_extent(
                                    dag.get_node_width(layout_dst_idx),
                                    dag.get_node_height(layout_dst_idx),
                                )
                                - 1,
                        )
                    } else {
                        lateral(layout_dst_idx, d.dst_face, positioned.1, layout_to_level)
                    };
                    (fx, fy, tx, ty)
                }
                None => (from_x, from_y, to_x, to_y),
            };
            #[cfg_attr(not(feature = "ports"), allow(unused_mut, unused_variables))]
            let mut explicit_label_col: Option<usize> = None;

            // Edge routing starts one row below the source node. Reversed
            // edges' arrowheads on that row are protected by the arrow-cell
            // reservation in the slot allocator, not by shifting corners.
            let edge_start_row = crate::algorithms::sugiyama::geometry::EDGE_START_OFFSET;

            // The explicit polyline of a detouring edge (feature-gated with
            // the path shape itself); `None` routes the inferred way.
            #[cfg(feature = "ports")]
            let explicit: Option<EdgePath> = detour.map(|det| {
                // Around the node, as an explicit
                // polyline in role space — `(cross, level)` pairs,
                // materialized with the rest below. Every horizontal run
                // sits on a row allocated for its exact interval; the
                // jog rows are the MultiSegment rows, bend line included.
                let band_row = |level: usize, slot: usize| {
                    level_offsets[level] + max_node_height[level] - 1 + edge_start_row + slot
                };
                // (row, from column, to column)
                let mut runs: Vec<(usize, usize, usize)> = Vec::new();
                let mut col = from_x;
                if det.src_lane != usize::MAX {
                    if det.src_face.is_level() {
                        let up_row = if layout_from_level == 0 {
                            top_base + top_extra - 1 - edge_start_row - det.up_slot
                        } else {
                            let row = band_row(layout_from_level - 1, det.up_slot);
                            level_routing_floor[layout_from_level - 1] =
                                level_routing_floor[layout_from_level - 1].max(row);
                            row
                        };
                        runs.push((up_row, from_x, det.src_lane));
                    } else {
                        // The lateral stub: out of the side face at the
                        // port's own row, straight onto the lane.
                        runs.push((from_y, from_x, det.src_lane));
                    }
                    col = det.src_lane;
                }
                let final_col = if det.dst_lane != usize::MAX {
                    det.dst_lane
                } else {
                    to_x
                };
                let kept_cols: Vec<(usize, usize)> = dummy_positions[edge_idx]
                    .iter()
                    .zip(&kept_wps[edge_idx])
                    .filter(|(_, k)| **k)
                    .map(|(&(level, x), _)| (level, x))
                    .collect();
                let first_target = kept_cols.first().map_or(final_col, |&(_, x)| x);
                explicit_label_col = Some(first_target);
                if col != first_target {
                    let row = band_row(layout_from_level, det.first_slot);
                    level_routing_floor[layout_from_level] =
                        level_routing_floor[layout_from_level].max(row);
                    runs.push((row, col, first_target));
                    col = first_target;
                }
                for (i, &(level, x)) in kept_cols.iter().enumerate() {
                    debug_assert_eq!(col, x);
                    let next = kept_cols.get(i + 1).map_or(final_col, |&(_, nx)| nx);
                    let slot = level_edge_next[level];
                    level_edge_next[level] += 1;
                    let wp_y = level_offsets[level]
                        + max_node_height[level]
                        + usize::from(level_labeled_src[level])
                        + slot;
                    edge_routing_ys.insert(wp_y);
                    level_routing_floor[level] = level_routing_floor[level].max(wp_y + 1);
                    runs.push((wp_y + 1, x, next));
                    col = next;
                }
                if det.dst_lane != usize::MAX {
                    if det.dst_face.is_level() {
                        let row = band_row(layout_to_level, det.below_slot);
                        level_routing_floor[layout_to_level] =
                            level_routing_floor[layout_to_level].max(row);
                        runs.push((row, det.dst_lane, to_x));
                    } else {
                        // Down the lane to the port's row, then into the
                        // side face.
                        runs.push((to_y, det.dst_lane, to_x));
                    }
                }
                // A run's end that IS an endpoint (a lateral stub leaves
                // the face cell itself) is no bend.
                let mut bends = Vec::with_capacity(runs.len() * 2);
                for &(row, a, b) in &runs {
                    edge_routing_ys.insert(row);
                    if a == b {
                        continue;
                    }
                    if (a, row) != (from_x, from_y) {
                        bends.push((a, row));
                    }
                    if (b, row) != (to_x, to_y) {
                        bends.push((b, row));
                    }
                }
                EdgePath::Orthogonal { bends }
            });
            #[cfg(not(feature = "ports"))]
            let explicit: Option<EdgePath> = None;
            let path = if let Some(explicit) = explicit {
                explicit
            } else if layout_to_level == layout_from_level + 1 {
                // Adjacent levels - direct or corner connection
                if from_x == to_x {
                    EdgePath::Direct
                } else {
                    // Get horizontal slot for this source at this level
                    let slot = if node_slots[layout_src_idx] != usize::MAX {
                        node_slots[layout_src_idx]
                    } else {
                        0
                    };

                    let hy = band_trailing + edge_start_row + slot;
                    edge_routing_ys.insert(hy);
                    if layout_from_level < level_routing_floor.len() {
                        level_routing_floor[layout_from_level] =
                            level_routing_floor[layout_from_level].max(hy);
                    }
                    EdgePath::Corner { bend_at: hy }
                }
            } else {
                // Skip-level edge - use dummy node positions for MultiSegment path
                let dummies = &dummy_positions[edge_idx];
                if dummies.is_empty() {
                    // Fallback to corner if no dummies (shouldn't happen)
                    let slot = if node_slots[layout_src_idx] != usize::MAX {
                        node_slots[layout_src_idx]
                    } else {
                        0
                    };
                    let hy = band_trailing + edge_start_row + slot;
                    edge_routing_ys.insert(hy);
                    if layout_from_level < level_routing_floor.len() {
                        level_routing_floor[layout_from_level] =
                            level_routing_floor[layout_from_level].max(hy);
                    }
                    EdgePath::Corner { bend_at: hy }
                } else {
                    // Build waypoints through the jogging dummies only —
                    // straight pass-throughs paint as part of a longer
                    // vertical segment and need no waypoint row.
                    let kept = &kept_wps[edge_idx];
                    let mut waypoints = Vec::with_capacity(dummies.len());
                    for (i, &(level, x)) in dummies.iter().enumerate() {
                        if !kept[i] {
                            continue;
                        }
                        // Assign a unique vertical slot for this edge at this level
                        let slot = level_edge_next[level];
                        level_edge_next[level] += 1;

                        let wp_edge_start_row =
                            max_node_height[level] + usize::from(level_labeled_src[level]);
                        let wp_y = level_offsets[level] + wp_edge_start_row + slot;
                        edge_routing_ys.insert(wp_y);
                        // Every kept waypoint bends right below its row (its
                        // x differs from the next column by construction).
                        let inter_corner_y = wp_y + 1;
                        edge_routing_ys.insert(inter_corner_y);
                        level_routing_floor[level] = level_routing_floor[level].max(inter_corner_y);
                        waypoints.push((x, wp_y));
                    }

                    if waypoints.is_empty() {
                        // Fully straight chain: the reserved dummy columns
                        // line up with the target, so the edge is a plain
                        // vertical — or a single bend in the source band.
                        if from_x == to_x {
                            EdgePath::Direct
                        } else {
                            let slot = if node_slots[layout_src_idx] != usize::MAX {
                                node_slots[layout_src_idx]
                            } else {
                                0
                            };
                            let hy = band_trailing + edge_start_row + slot;
                            edge_routing_ys.insert(hy);
                            if layout_from_level < level_routing_floor.len() {
                                level_routing_floor[layout_from_level] =
                                    level_routing_floor[layout_from_level].max(hy);
                            }
                            EdgePath::Corner { bend_at: hy }
                        }
                    } else {
                        let slot = if node_slots[layout_src_idx] != usize::MAX {
                            node_slots[layout_src_idx]
                        } else {
                            0
                        };

                        // Calculate offset from (y+1)
                        let start_offset = (edge_start_row + slot).saturating_sub(1);

                        // Record the INITIAL corner Y (first segment routing) — the paint
                        // code draws a horizontal segment at band_trailing + 1 + start_offset,
                        // which is NOT a waypoint Y but still occupies a row.
                        let initial_corner_y = band_trailing + 1 + start_offset;
                        edge_routing_ys.insert(initial_corner_y);
                        if layout_from_level < level_routing_floor.len() {
                            level_routing_floor[layout_from_level] =
                                level_routing_floor[layout_from_level].max(initial_corner_y);
                        }

                        EdgePath::MultiSegment {
                            waypoints,
                            start_offset,
                        }
                    }
                }
            };

            // Label placement row layout (band_trailing = last line of
            // the source level's full extent):
            //   band_trailing:   [source level bottom]
            //   band_trailing+1: corner routing
            //   band_trailing+2: flow connector
            //   band_trailing+3: label line
            //   to_y:            [target node]
            let (label_x, label_y) = label
                .map(|lbl| {
                    let label_len = lbl.chars().count() + 2; // +2 for quotes

                    // First row below the source level's routing block — shared
                    // with the CSR backend so label rows cannot drift.
                    let label_y = band_trailing
                        + crate::algorithms::sugiyama::geometry::edge_label_offset(
                            level_occupied_slots[layout_from_level].len(),
                        );

                    // Find the edge's X position at the label row
                    let edge_x_at_label = match &path {
                        #[cfg(feature = "ports")]
                        EdgePath::Orthogonal { .. } => explicit_label_col.unwrap_or(from_x),
                        EdgePath::Direct => from_x,
                        EdgePath::Corner { bend_at } => {
                            // If label row is before the corner, edge is at from_x
                            // If label row is after the corner, edge is at to_x
                            if label_y <= *bend_at { from_x } else { to_x }
                        }
                        EdgePath::SideChannel {
                            channel_at,
                            span_start,
                            ..
                        } => {
                            // If before the horizontal segment, use from_x
                            // Otherwise use channel_at
                            if label_y < *span_start {
                                from_x
                            } else {
                                *channel_at
                            }
                        }
                        EdgePath::MultiSegment {
                            waypoints,
                            start_offset,
                        } => {
                            // Find which segment the label row falls into
                            // from_y is bottom of source node, +1 goes to routing area
                            let bend_at = band_trailing + 1 + start_offset;

                            if label_y <= bend_at || waypoints.is_empty() {
                                from_x
                            } else {
                                waypoints[0].0
                            }
                        }
                        EdgePath::Spline { .. } => from_x,
                    };

                    // Materialize the anchor; label text spreads along
                    // PHYSICAL x whichever role that is (temp/08 D9), so
                    // centering and the canvas clamp apply afterwards.
                    let (anchor_x, anchor_y) = A::materialize(label_y, edge_x_at_label);
                    let (phys_w, _) = A::materialize(total_height, max_width);
                    let half_len = label_len / 2;
                    let label_x = anchor_x.saturating_sub(half_len);
                    let clamped_x = if label_x + label_len > phys_w {
                        phys_w.saturating_sub(label_len)
                    } else {
                        label_x
                    };
                    (clamped_x, anchor_y)
                })
                .unwrap_or((0, 0));

            let reversed = back_edges.get(edge_idx).copied().unwrap_or(false);
            // ── Materialization: role pairs → physical (x, y). The
            // label logic above consumed the role values, so this is
            // the last stop before the IR.
            let (from_x, from_y) = A::materialize(from_y, from_x);
            let (to_x, to_y) = A::materialize(to_y, to_x);
            let path = match path {
                EdgePath::MultiSegment {
                    waypoints,
                    start_offset,
                } => EdgePath::MultiSegment {
                    waypoints: waypoints
                        .into_iter()
                        .map(|(cross, lvl)| A::materialize(lvl, cross))
                        .collect(),
                    start_offset,
                },
                #[cfg(feature = "ports")]
                EdgePath::Orthogonal { bends } => EdgePath::Orthogonal {
                    bends: bends
                        .into_iter()
                        .map(|(cross, lvl)| A::materialize(lvl, cross))
                        .collect(),
                },
                p => p,
            };
            builder.add_edge(LayoutEdge {
                from_id,
                to_id,
                from_x,
                from_y,
                to_x,
                to_y,
                path,
                flow_axis: A::FLOW_AXIS,
                edge_index: edge_idx,
                label,
                label_x,
                label_y,
                directed: true,
                reversed,
            });
        }
    }

    // Compute subgraph bounding boxes if any subgraphs are defined.
    // ── Materialization point for canvas extents: (level, cross)
    // totals → (width, height). Adjustments below work in PHYSICAL
    // space (`SubgraphInfo` is physical IR): the canvas must cover
    // every border — a label-widened cluster box can extend past the
    // node extent that `max_width` was derived from.
    let (mut canvas_width, mut canvas_height) = A::materialize(total_height, max_width);
    if dag.has_subgraphs() {
        let sg_infos = crate::algorithms::sugiyama::subgraph::compute_bounding_boxes::<A>(
            dag,
            &real_node_coords,
            &level_offsets,
            total_height,
            &edge_routing_ys,
            &level_routing_floor,
        );
        for info in sg_infos {
            canvas_width = canvas_width.max(info.x + info.width + 1);
            // Cover every border on both axes: materialized Horizontal
            // boxes can grow the height, and a Vertical bottom border
            // pushed off a routing row now grows the canvas instead of
            // clipping (rare corner, disclosed in the changelog).
            canvas_height = canvas_height.max(info.y + info.height);
            builder.add_subgraph(info);
        }
    }
    builder.set_dimensions(canvas_width, canvas_height);
    builder.set_direction(config.direction);

    // Preserve self-loops as records: identity (input index), label,
    // owning node — absent from the routed list, visible to the scene.
    for (i, &(f, t, label)) in dag.edges.iter().enumerate() {
        if f == t {
            builder.add_self_loop(crate::ir::SelfLoopRecord {
                node_id: f,
                node_index: dag
                    .id_to_index
                    .get(&f)
                    .map_or(usize::MAX, |&di| ir_index_of[di]),
                edge_index: i,
                // An empty label is no label — normalized at the
                // record's birth, matching the arena pool's len-0
                // convention, so both backends agree everywhere
                // downstream (views, diagnostics, JSON).
                label: label.filter(|l| !l.is_empty()),
            });
        }
    }
    let mut ir = builder.build();
    #[cfg(feature = "layout-vertical")]
    if config.direction == crate::graph::Direction::BottomUp {
        // Physical-space contract: IR coordinates match rendered cells.
        ir.flip_vertical();
    }
    // RL is LR mirrored on x — the same contract, other axis. Gated
    // on the PROFILE, not just the direction: a `RightLeft` request
    // that somehow reached the `Vertical` profile must not have its
    // vertical layout mirrored, which would be neither direction.
    #[cfg(feature = "layout-horizontal")]
    if config.direction == crate::graph::Direction::RightLeft
        && matches!(A::FLOW_AXIS, crate::ir::FlowAxis::X)
    {
        ir.flip_horizontal();
    }
    ir
}

// ── Crossing reduction ───────────────────────────────────────────────────

/// Build a mapping from each node index to the edge indices it participates in.
/// This enables real nodes to find their skip-level edge dummies during crossing
/// reduction (the key fix for incomplete neighbor gathering).
fn build_node_edge_indices(dag: &Graph<'_>) -> Vec<Vec<usize>> {
    let mut node_edges: Vec<Vec<usize>> = vec![Vec::new(); dag.nodes.len()];
    for (edge_idx, &(from_id, to_id, _)) in dag.edges.iter().enumerate() {
        if let Some(from_idx) = dag.node_index(from_id) {
            node_edges[from_idx].push(edge_idx);
        }
        if let Some(to_idx) = dag.node_index(to_id) {
            node_edges[to_idx].push(edge_idx);
        }
    }
    node_edges
}

// ── X-coordinate refinement (median placement) ─────────────────────────

/// Allocate a routing-row slot at a level for the exact cross interval
/// `[min_x, max_x]`: the first slot index whose registered intervals
/// AND jog bend blocks do not collide (the greedy allocator's
/// `s < max_x && e > min_x` test), creating slots up to the per-level
/// cap; past the cap, slot 0 (the cap's degradation).
#[cfg_attr(not(feature = "ports"), allow(dead_code))]
fn alloc_slot(
    slots: &mut Vec<Vec<(usize, usize)>>,
    jogs: &[(usize, usize, usize)],
    labeled: bool,
    min_x: usize,
    max_x: usize,
) -> usize {
    const MAX_SLOTS_PER_LEVEL: usize = 8;
    let label_row = usize::from(labeled);
    for i in 0..MAX_SLOTS_PER_LEVEL {
        let in_slot = slots
            .get(i)
            .is_some_and(|occupied| occupied.iter().any(|&(s, e)| s < max_x && e > min_x));
        let in_jog = jogs
            .iter()
            .any(|&(k, s, e)| 1 + label_row + k == i && s < max_x && e > min_x);
        if in_slot || in_jog {
            continue;
        }
        while slots.len() <= i {
            slots.push(Vec::new());
        }
        slots[i].push((min_x, max_x));
        return i;
    }
    if slots.is_empty() {
        slots.push(Vec::new());
    }
    slots[0].push((min_x, max_x));
    0
}

// ── Fan-aware chain-lane allocation (temp/09 P3) ─────────────────────────

/// One skip-level edge's routing chain, in layout orientation.
struct ChainPlan {
    edge_idx: usize,
    /// Layout source / target node indices (back edges already flipped).
    src_idx: usize,
    dst_idx: usize,
    s_level: usize,
    t_level: usize,
    s_cross: usize,
    t_cross: usize,
}

/// Lexicographic transition cost for the §4.7 fallback:
/// `(crossings, lane changes, displacement, extent)`. The first three
/// accumulate by addition along a path, `extent` by max — a lane used
/// across five gaps widens the canvas once, not five times.
type LaneCost = (usize, usize, usize, usize);

fn cost_add(a: LaneCost, b: LaneCost) -> LaneCost {
    (a.0 + b.0, a.1 + b.1, a.2 + b.2, a.3.max(b.3))
}

/// Allocate every skip-edge chain a cross-axis lane clear of the fans its
/// gaps sweep (temp/09 §4). Chains are allocated shortest-first so each
/// longer chain is pushed outside the ones already placed and the
/// farthest-travelling edge ends outermost — Ash's rule: the farther edge
/// diverges first and takes the outer track.
///
/// Rewrites `dummy_positions` in place. Returns the greatest cross
/// coordinate reached plus the waypoint body, so the caller can widen the
/// canvas — the flip must see the same width (§4.8), or `RightLeft` comes
/// out skewed instead of mirrored.
fn allocate_chain_lanes<A: Axis>(
    dag: &Graph<'_>,
    virtual_levels: &[Vec<VNode>],
    real_node_coords: &[(usize, usize, usize, usize)],
    back_edges: &[bool],
    dummy_positions: &mut [Vec<(usize, usize)>],
) -> usize {
    use crate::algorithms::sugiyama::geometry::{
        CrossSpan, GapClaim, LANE_SPAN_CAP, free_gap_containing, lane_pass_enabled, merge_fan,
    };

    let n_levels = virtual_levels.len();
    if n_levels < 2 {
        return 0;
    }
    // Shared budget (geometry.rs): outside it, the graph keeps its packed
    // routing — evaluated identically by the CSR backend and the arena
    // estimator, so backends cannot diverge and arenas cannot under-provision.
    let total_dummies: usize = dummy_positions.iter().map(|v| v.len()).sum();
    if !lane_pass_enabled(n_levels, dag.edges.len(), total_dummies) {
        return 0;
    }
    let n_gaps = n_levels - 1;
    let clearance = A::SIBLING_GAP_CROSS;
    let body = A::DUMMY_CROSS.saturating_sub(1);

    // Layout-oriented endpoints of an edge: back edges run target→source.
    let layout_ends = |ei: usize| -> Option<(usize, usize)> {
        let (from_id, to_id, _) = dag.edges[ei];
        if from_id == to_id {
            return None;
        }
        let (fi, ti) = (dag.node_index(from_id)?, dag.node_index(to_id)?);
        Some(if back_edges.get(ei).copied().unwrap_or(false) {
            (ti, fi)
        } else {
            (fi, ti)
        })
    };

    // ── Fixed claims (§4.1): adjacent-level real-to-real edge sweeps ──
    let mut fixed: Vec<Vec<GapClaim>> = vec![Vec::new(); n_gaps];
    for ei in 0..dag.edges.len() {
        let Some((s, d)) = layout_ends(ei) else {
            continue;
        };
        let (sl, _, sc, sw) = real_node_coords[s];
        let (dl, _, dc, dw) = real_node_coords[d];
        if sl.abs_diff(dl) != 1 {
            continue; // skip-level edges are chains, not fixed claims
        }
        fixed[sl.min(dl)].push(GapClaim {
            span: CrossSpan::between(sc + sw / 2, dc + dw / 2),
            edge_idx: ei,
        });
    }

    // ── Level obstacles (§4.3.1): node bodies + cluster envelopes ──
    let mut level_obstacles: Vec<Vec<CrossSpan>> = vec![Vec::new(); n_levels];
    for &(lvl, _, x, w) in real_node_coords.iter() {
        level_obstacles[lvl].push(CrossSpan {
            lo: x,
            hi: x + w.saturating_sub(1),
        });
    }
    if dag.has_subgraphs() {
        let (env, ranges) = crate::algorithms::sugiyama::subgraph::cluster_cross_envelopes::<A>(
            dag,
            real_node_coords,
        );
        for (si, e) in env.iter().enumerate() {
            if let Some((lo, hi)) = *e {
                let (first, last) = ranges[si];
                if first > last {
                    continue; // empty cluster
                }
                for lvl in first..=last.min(n_levels - 1) {
                    level_obstacles[lvl].push(CrossSpan { lo, hi });
                }
            }
        }
    }

    // ── Chains, in allocation order (§4.4) ──
    // Spatial rank (longest outermost) is `chain_cmp`; the allocation
    // order that PRODUCES it is the reverse on the span components —
    // ascending `(target_level, total_span, edge_index)` — because each
    // placed chain becomes an obstacle that pushes later (longer) chains
    // further out. The index tie stays ascending in both readings.
    let mut chains: Vec<ChainPlan> = Vec::new();
    for ei in 0..dag.edges.len() {
        if dummy_positions[ei].is_empty() {
            continue;
        }
        let Some((s, d)) = layout_ends(ei) else {
            continue;
        };
        let (sl, _, sc, sw) = real_node_coords[s];
        let (dl, _, dc, dw) = real_node_coords[d];
        if dl <= sl {
            continue; // defensive: layout levels must increase
        }
        chains.push(ChainPlan {
            edge_idx: ei,
            src_idx: s,
            dst_idx: d,
            s_level: sl,
            t_level: dl,
            s_cross: sc + sw / 2,
            t_cross: dc + dw / 2,
        });
    }
    chains.sort_unstable_by_key(|c| (c.t_level, c.t_level - c.s_level, c.edge_idx));

    let mut committed: Vec<Vec<GapClaim>> = vec![Vec::new(); n_gaps];
    let mut reach = 0usize;
    let mut scratch: Vec<CrossSpan> = Vec::new();
    // Global work purse (§4.7, claim-comparison units): both backends
    // charge the same amounts at the same points in the same chain
    // order, so they exhaust identically.
    let mut dp_budget = crate::algorithms::sugiyama::geometry::LANE_WORK_BUDGET;

    for ch in &chains {
        let span_levels = ch.t_level - ch.s_level;
        let wp_count = dummy_positions[ch.edge_idx].len();

        // §4.5: a claim is exempt for THIS chain only in its endpoint
        // gaps — shared source trunk in gap(S, S+1), shared target merge
        // in gap(T-1, T). A same-source chain's distant lane is not a
        // trunk and stays an obstacle.
        let exempt = |claim: &GapClaim, gap: usize| -> bool {
            let Some((cs, cd)) = layout_ends(claim.edge_idx) else {
                return false;
            };
            (gap == ch.s_level && cs == ch.src_idx) || (gap + 1 == ch.t_level && cd == ch.dst_idx)
        };
        let filtered_spans = |gap: usize, out: &mut Vec<CrossSpan>| {
            out.clear();
            for c in fixed[gap].iter().chain(committed[gap].iter()) {
                if !exempt(c, gap) {
                    out.push(c.span);
                }
            }
        };

        // The chain's ideal line: interpolation between endpoint centers.
        let ideal_at = |l: usize| -> usize {
            let step = l - ch.s_level;
            if ch.t_cross >= ch.s_cross {
                ch.s_cross + (ch.t_cross - ch.s_cross) * step / span_levels
            } else {
                ch.s_cross - (ch.s_cross - ch.t_cross) * step / span_levels
            }
        };

        // Waypoint levels must be exactly S+1 ..= T-1, in order. Anything
        // else (defensive; not produced by virtual-level insertion) keeps
        // its packed coordinates — but still commits them, so later
        // chains route around it.
        let contiguous = wp_count == span_levels.saturating_sub(1)
            && dummy_positions[ch.edge_idx]
                .iter()
                .enumerate()
                .all(|(i, &(l, _))| l == ch.s_level + 1 + i);

        let mut placed: Option<Vec<usize>> = None;

        // Per-chain span budget (LANE_SPAN_CAP), shared with CSR: the
        // union scratch this chain needs, counted before building it.
        let mut span_need = 0usize;
        for gap in ch.s_level..ch.t_level {
            span_need += fixed[gap]
                .iter()
                .chain(committed[gap].iter())
                .filter(|c| !exempt(c, gap))
                .count();
        }
        for lvl in (ch.s_level + 1)..ch.t_level {
            span_need += level_obstacles[lvl].len();
        }

        if contiguous && wp_count > 0 && span_need <= LANE_SPAN_CAP && dp_budget >= span_need {
            // Charge the union/candidate stream work up front; an
            // exhausted purse skips the whole attempt (packed), so later
            // chains never repeat candidate construction for nothing.
            dp_budget -= span_need;
            // ── §4.3: one lane for the whole chain ──
            // Union every constraint: all traversed gaps' filtered claims
            // plus all interior levels' obstacles, merged together. A
            // coordinate free in the union is free in each individually.
            scratch.clear();
            let mut tmp: Vec<CrossSpan> = Vec::new();
            for gap in ch.s_level..ch.t_level {
                filtered_spans(gap, &mut tmp);
                scratch.extend_from_slice(&tmp);
            }
            for lvl in (ch.s_level + 1)..ch.t_level {
                scratch.extend_from_slice(&level_obstacles[lvl]);
            }
            let n = merge_fan(&mut scratch, clearance);
            let union_fan = &scratch[..n];

            // §4.3.2: the lane must live in the free component reachable
            // from the source in the source-side gap and from the target
            // in the target-side gap.
            filtered_spans(ch.s_level, &mut tmp);
            let sn = merge_fan(&mut tmp, clearance);
            let s_comp = free_gap_containing(&tmp[..sn], ch.s_cross);
            filtered_spans(ch.t_level - 1, &mut tmp);
            let tn = merge_fan(&mut tmp, clearance);
            let t_comp = free_gap_containing(&tmp[..tn], ch.t_cross);

            let walk_cost = crate::algorithms::sugiyama::geometry::lane_scan_work(0, n, wp_count);
            let can_walk = dp_budget >= walk_cost;
            if can_walk {
                dp_budget -= walk_cost;
            }
            if let (true, Some((slo, shi)), Some((tlo, thi))) = (can_walk, s_comp, t_comp) {
                let lo_bound = slo.max(tlo);
                let hi_bound = match (shi, thi) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (Some(a), None) | (None, Some(a)) => Some(a),
                    (None, None) => None,
                };
                // Free components of the union, clipped to the bounds;
                // the winner minimises total distance to the ideal line,
                // ties toward the smaller coordinate (§4.3.3 — NOT
                // `nearest_outside`'s upward rule; different operation).
                let mut ideals: Vec<usize> = ((ch.s_level + 1)..ch.t_level).map(ideal_at).collect();
                ideals.sort_unstable();
                let median = ideals[(ideals.len() - 1) / 2];
                let total_dist =
                    |p: usize| -> usize { ideals.iter().map(|&i| i.abs_diff(p)).sum() };

                let mut best: Option<(usize, usize)> = None; // (dist, coord)
                let mut consider = |p: usize| {
                    // §4.6/review: a lane the CSR backend cannot
                    // represent does not exist for either backend.
                    if !crate::algorithms::sugiyama::geometry::lane_admissible(p) {
                        return;
                    }
                    let d = total_dist(p);
                    if best.is_none_or(|(bd, bp)| d < bd || (d == bd && p < bp)) {
                        best = Some((d, p));
                    }
                };
                // Walk the union's free components: before, between, after.
                let mut cursor = 0usize;
                for (i, s) in union_fan.iter().enumerate() {
                    if s.lo > cursor {
                        // free [cursor, s.lo-1]
                        let (flo, fhi) = (cursor, s.lo - 1);
                        let lo = flo.max(lo_bound);
                        let hi = hi_bound.map_or(fhi, |h| fhi.min(h));
                        if lo <= hi {
                            consider(median.clamp(lo, hi));
                        }
                    }
                    cursor = s.hi.saturating_add(1);
                    if i == union_fan.len() - 1 || s.hi == usize::MAX {
                        // handled after loop / saturated
                    }
                }
                if cursor != usize::MAX || union_fan.is_empty() {
                    // trailing unbounded free interval [cursor, ..)
                    let lo = cursor.max(lo_bound);
                    let hi = hi_bound;
                    match hi {
                        Some(h) if lo <= h => consider(median.clamp(lo, h)),
                        None => consider(median.max(lo)),
                        _ => {}
                    }
                }

                if let Some((_, lane)) = best {
                    placed = Some(vec![lane; wp_count]);
                }
            }

            // ── §4.7: no single lane — lexicographic DP over levels ──
            if placed.is_none() {
                placed = chain_lane_dp::<A>(
                    ch,
                    &fixed,
                    &committed,
                    &level_obstacles,
                    clearance,
                    &exempt,
                    &ideal_at,
                    span_need,
                    &mut dp_budget,
                );
            }
        }

        // Write the allocation (or keep packed coordinates on failure),
        // then commit the complete segment spans so later chains see the
        // real geometry (§4.1) — including the source and target
        // connector segments.
        let coords: Vec<usize> = match &placed {
            Some(v) => v.clone(),
            None => dummy_positions[ch.edge_idx]
                .iter()
                .map(|&(_, x)| x)
                .collect(),
        };
        if let Some(v) = &placed {
            for (slot, &c) in dummy_positions[ch.edge_idx].iter_mut().zip(v.iter()) {
                slot.1 = c;
            }
        }
        let mut prev = ch.s_cross;
        for (i, &c) in coords.iter().enumerate() {
            let gap = ch.s_level + i;
            committed[gap].push(GapClaim {
                span: CrossSpan {
                    lo: prev.min(c),
                    hi: prev.max(c) + body,
                },
                edge_idx: ch.edge_idx,
            });
            reach = reach.max(c + A::DUMMY_CROSS);
            prev = c;
        }
        committed[ch.t_level - 1].push(GapClaim {
            span: CrossSpan {
                lo: prev.min(ch.t_cross),
                hi: prev.max(ch.t_cross) + body,
            },
            edge_idx: ch.edge_idx,
        });
    }

    reach
}

/// §4.7 fallback: lexicographic DP over the chain's levels.
///
/// States are level coordinates — `C[S]` and `C[T]` are singletons holding
/// the endpoint centers, so the source connector and target merge are
/// costed like any other segment. Candidates per interior level are the
/// finite free-interval boundaries of its two adjacent gaps plus the
/// source, target and ideal probes clamped into free space, filtered by
/// the level's own obstacles. Ties break toward the smaller coordinate,
/// then the earlier candidate — a total order both backends must share.
#[allow(clippy::too_many_arguments)]
fn chain_lane_dp<A: Axis>(
    ch: &ChainPlan,
    fixed: &[Vec<crate::algorithms::sugiyama::geometry::GapClaim>],
    committed: &[Vec<crate::algorithms::sugiyama::geometry::GapClaim>],
    level_obstacles: &[Vec<crate::algorithms::sugiyama::geometry::CrossSpan>],
    clearance: usize,
    exempt: &dyn Fn(&crate::algorithms::sugiyama::geometry::GapClaim, usize) -> bool,
    ideal_at: &dyn Fn(usize) -> usize,
    span_need: usize,
    dp_budget: &mut usize,
) -> Option<Vec<usize>> {
    use crate::algorithms::sugiyama::geometry::{
        CrossSpan, LANE_CAND_CAP, merge_fan, nearest_outside,
    };

    let span_levels = ch.t_level - ch.s_level;
    let wp_count = span_levels - 1;
    let body = A::DUMMY_CROSS.saturating_sub(1);
    // Candidate generation streams the same claims the union did —
    // charge it before doing it.
    if *dp_budget < span_need {
        return None;
    }
    *dp_budget -= span_need;
    let mut total_cands = 0usize;

    // Raw filtered claims per traversed gap (for crossing counts) and the
    // merged form (for free intervals / candidate generation).
    let mut raw: Vec<Vec<CrossSpan>> = Vec::with_capacity(span_levels);
    let mut merged: Vec<Vec<CrossSpan>> = Vec::with_capacity(span_levels);
    for gap in ch.s_level..ch.t_level {
        let mut spans: Vec<CrossSpan> = fixed[gap]
            .iter()
            .chain(committed[gap].iter())
            .filter(|c| !exempt(c, gap))
            .map(|c| c.span)
            .collect();
        raw.push(spans.clone());
        let n = merge_fan(&mut spans, clearance);
        spans.truncate(n);
        merged.push(spans);
    }

    // Candidate coordinates per interior level.
    let mut cands: Vec<Vec<usize>> = Vec::with_capacity(wp_count);
    for (li, lvl) in ((ch.s_level + 1)..ch.t_level).enumerate() {
        let mut c: Vec<usize> = Vec::new();
        for m in [&merged[li], &merged[li + 1]] {
            let mut cursor = 0usize;
            for s in m.iter() {
                if s.lo > cursor {
                    c.push(cursor);
                    c.push(s.lo - 1);
                }
                cursor = s.hi.saturating_add(1);
            }
            if cursor != usize::MAX {
                c.push(cursor); // floor of the unbounded top interval
            }
        }
        // The level's own obstacles also CONTRIBUTE candidates: a cluster
        // envelope can swallow every gap-derived boundary at its levels,
        // and without the coordinates just past the envelope the DP would
        // starve and give the chain up (observed: tier3/tier5 chains all
        // kept packed). Merged here once, reused for probe clamping and
        // the final filter.
        let mut obs: Vec<CrossSpan> = level_obstacles[lvl].clone();
        let on = merge_fan(&mut obs, clearance);
        {
            let mut cursor = 0usize;
            for sp in obs[..on].iter() {
                if sp.lo > cursor {
                    c.push(cursor);
                    c.push(sp.lo - 1);
                }
                cursor = sp.hi.saturating_add(1);
            }
            if cursor != usize::MAX {
                c.push(cursor);
            }
        }
        // Probes, clamped clear of both adjacent gaps AND this level.
        let mut both: Vec<CrossSpan> = merged[li]
            .iter()
            .chain(merged[li + 1].iter())
            .chain(obs[..on].iter())
            .copied()
            .collect();
        let bn = merge_fan(&mut both, 0);
        for probe in [ch.s_cross, ch.t_cross, ideal_at(lvl)] {
            if let Some(p) = nearest_outside(&both[..bn], probe, None) {
                c.push(p);
            }
        }
        // The candidate budget counts RAW pushes — before filtering and
        // deduplication — because that is the quantity the CSR backend
        // can meter without buffering past its arena slice. Counting
        // post-dedup here while CSR counts raw would let a candidate-
        // heavy chain run the DP on one backend and keep packed routing
        // on the other.
        total_cands += c.len();
        if total_cands > LANE_CAND_CAP {
            return None; // shared budget exhausted — keep packed
        }
        // Filter by the level's own obstacles, and by representability
        // (`LANE_MAX_CROSS`): CSR stores coordinates as u16, and a
        // coordinate only one backend can hold must not exist in either.
        c.retain(|&p| {
            crate::algorithms::sugiyama::geometry::lane_admissible(p)
                && !obs[..on].iter().any(|s| s.contains(p))
        });
        c.sort_unstable();
        c.dedup();
        if c.is_empty() {
            return None; // no representable candidate — keep packed (§4.6)
        }
        cands.push(c);
    }

    // §4.7 work budget: transitions ARE claim scans — a transition over
    // a thousand-claim gap costs a thousand comparisons, so the meter
    // weighs each row product by its gap's claim count.
    {
        let rows: Vec<usize> = cands.iter().map(|c| c.len()).collect();
        let claims: Vec<usize> = raw.iter().map(|r| r.len()).collect();
        let work = crate::algorithms::sugiyama::geometry::lane_dp_work(&rows, &claims);
        if work > *dp_budget {
            return None; // purse exhausted — keep packed (both backends)
        }
        *dp_budget -= work;
    }

    let crossings = |gap_li: usize, a: usize, b: usize| -> usize {
        let (lo, hi) = (a.min(b), a.max(b));
        raw[gap_li]
            .iter()
            .filter(|s| s.lo <= hi + body && lo <= s.hi.saturating_add(body))
            .count()
    };

    // dp[li][ci] = (cost, predecessor candidate index)
    let mut dp: Vec<Vec<(LaneCost, usize)>> = Vec::with_capacity(wp_count);
    for (li, level_cands) in cands.iter().enumerate() {
        let lvl = ch.s_level + 1 + li;
        let ideal = ideal_at(lvl);
        let mut row: Vec<(LaneCost, usize)> = Vec::with_capacity(level_cands.len());
        for &c in level_cands.iter() {
            let step_cost = |from: usize, gap_li: usize| -> LaneCost {
                (
                    crossings(gap_li, from, c),
                    usize::from(from != c),
                    ideal.abs_diff(c),
                    c + A::DUMMY_CROSS,
                )
            };
            let best = if li == 0 {
                (step_cost(ch.s_cross, 0), usize::MAX)
            } else {
                let mut b: Option<(LaneCost, usize)> = None;
                for (pi, &(pcost, _)) in dp[li - 1].iter().enumerate() {
                    let pc = cands[li - 1][pi];
                    let total = cost_add(pcost, step_cost(pc, li));
                    if b.is_none_or(|(bc, _)| total < bc) {
                        b = Some((total, pi));
                    }
                }
                b?
            };
            row.push(best);
        }
        dp.push(row);
    }

    // Close with the final segment into the target.
    let last = wp_count - 1;
    let mut best_end: Option<(LaneCost, usize)> = None;
    for (ci, &(cost, _)) in dp[last].iter().enumerate() {
        let c = cands[last][ci];
        let tail = (
            crossings(span_levels - 1, c, ch.t_cross),
            usize::from(c != ch.t_cross),
            0,
            0,
        );
        let total = cost_add(cost, tail);
        if best_end.is_none_or(|(bc, _)| total < bc) {
            best_end = Some((total, ci));
        }
    }
    let (_, mut ci) = best_end?;

    let mut out = vec![0usize; wp_count];
    for li in (0..wp_count).rev() {
        out[li] = cands[li][ci];
        ci = dp[li][ci].1;
    }
    Some(out)
}

/// Refine x-coordinates by shifting nodes toward the median x of their
/// connected neighbors on adjacent levels. This is the "coordinate
/// assignment" step of the Sugiyama algorithm — it reduces zigzag edges
/// by aligning parents and children vertically.
///
/// Uses alternating down-sweeps (align with parents) and up-sweeps
/// (align with children) for several iterations. Each node is shifted
/// toward its ideal position, constrained by minimum spacing with its
/// neighbors on the same level.
fn refine_x_positions<A: Axis>(
    dag: &Graph<'_>,
    virtual_levels: &[Vec<VNode>],
    x_coords: &mut [Vec<usize>],
    widths: &[Vec<usize>],
    node_edge_indices: &[Vec<usize>],
    node_spacing: usize,
) {
    use crate::algorithms::sugiyama::subgraph::vnode_subgraph;

    let num_levels = virtual_levels.len();

    if num_levels <= 1 {
        return;
    }

    const ITERATIONS: usize = 8;

    // Helper: compute minimum gap between adjacent nodes, accounting for
    // subgraph boundary padding.
    let gap_between = |level: usize, left_pos: usize, right_pos: usize| -> usize {
        let left_sg = vnode_subgraph(dag, &virtual_levels[level][left_pos]);
        let right_sg = vnode_subgraph(dag, &virtual_levels[level][right_pos]);
        if left_sg != right_sg {
            A::SG_GAP_CROSS
        } else {
            node_spacing
        }
    };
    // Also enforce left margin for the first node if it's inside a subgraph
    let left_margin = |level: usize| -> usize {
        if virtual_levels[level].is_empty() {
            return 0;
        }
        let sg = vnode_subgraph(dag, &virtual_levels[level][0]);
        crate::algorithms::sugiyama::subgraph::leading_cross_pad::<A>(dag, sg)
    };

    // Helper: compute median center-x of connected neighbors on an adjacent level.
    // Returns None if no connections exist.
    let connected_median_x =
        |vnode: &VNode, adj_vnodes: &[VNode], adj_x: &[usize], adj_w: &[usize]| -> Option<usize> {
            // Build lookup: real node index → center-x, edge index → center-x
            let mut positions: Vec<usize> = Vec::new();

            match vnode {
                VNode::Real(idx) => {
                    for &edge_idx in &node_edge_indices[*idx] {
                        let &(from_id, to_id, _) = &dag.edges[edge_idx];

                        // Check real endpoints on adjacent level
                        for &nid in &[from_id, to_id] {
                            if let Some(nidx) = dag.node_index(nid) {
                                if nidx != *idx {
                                    // Find position of nidx on adjacent level
                                    for (p, av) in adj_vnodes.iter().enumerate() {
                                        if let VNode::Real(ri) = av {
                                            if *ri == nidx {
                                                positions.push(adj_x[p] + adj_w[p] / 2);
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Check dummy on adjacent level
                        for (p, av) in adj_vnodes.iter().enumerate() {
                            if let VNode::Dummy { edge_idx: ei } = av {
                                if *ei == edge_idx {
                                    positions.push(adj_x[p] + adj_w[p] / 2);
                                }
                            }
                        }
                    }
                }
                VNode::Dummy { edge_idx } => {
                    let &(from_id, to_id, _) = &dag.edges[*edge_idx];

                    // Check same-edge dummy on adjacent level
                    for (p, av) in adj_vnodes.iter().enumerate() {
                        if let VNode::Dummy { edge_idx: ei } = av {
                            if *ei == *edge_idx {
                                positions.push(adj_x[p] + adj_w[p] / 2);
                            }
                        }
                    }

                    // Check real endpoints on adjacent level
                    for &nid in &[from_id, to_id] {
                        if let Some(nidx) = dag.node_index(nid) {
                            for (p, av) in adj_vnodes.iter().enumerate() {
                                if let VNode::Real(ri) = av {
                                    if *ri == nidx {
                                        positions.push(adj_x[p] + adj_w[p] / 2);
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if positions.is_empty() {
                return None;
            }
            positions.sort_unstable();
            let median = if positions.len() % 2 == 1 {
                positions[positions.len() / 2]
            } else {
                let mid = positions.len() / 2;
                (positions[mid - 1] + positions[mid]) / 2
            };
            Some(median)
        };

    // Helper: compute and apply the shift for one node on a level.
    let shift_node = |x_coords: &mut [Vec<usize>],
                      level: usize,
                      pos: usize,
                      target_center: usize,
                      widths: &[Vec<usize>],
                      gap_fn: &dyn Fn(usize, usize, usize) -> usize,
                      margin_fn: &dyn Fn(usize) -> usize| {
        let n = x_coords[level].len();
        let my_w = widths[level][pos];
        let target_x = target_center.saturating_sub(my_w / 2);

        let min_x = if pos == 0 {
            margin_fn(level)
        } else {
            let g = gap_fn(level, pos - 1, pos);
            x_coords[level][pos - 1] + widths[level][pos - 1] + g
        };
        let max_x = if pos + 1 < n {
            let g = gap_fn(level, pos, pos + 1);
            x_coords[level][pos + 1].saturating_sub(my_w + g)
        } else {
            usize::MAX
        };

        x_coords[level][pos] = target_x.max(min_x).min(max_x);
    };

    for _iter in 0..ITERATIONS {
        // Down sweep: align with parents (level above)
        for level in 1..num_levels {
            let adj_level = level - 1;
            let n = virtual_levels[level].len();

            // Pass 1: right-to-left — allows nodes to cascade leftward
            for pos in (0..n).rev() {
                if let Some(tc) = connected_median_x(
                    &virtual_levels[level][pos],
                    &virtual_levels[adj_level],
                    &x_coords[adj_level],
                    &widths[adj_level],
                ) {
                    shift_node(x_coords, level, pos, tc, widths, &gap_between, &left_margin);
                }
            }

            // Pass 2: left-to-right — allows nodes to cascade rightward
            for pos in 0..n {
                if let Some(tc) = connected_median_x(
                    &virtual_levels[level][pos],
                    &virtual_levels[adj_level],
                    &x_coords[adj_level],
                    &widths[adj_level],
                ) {
                    shift_node(x_coords, level, pos, tc, widths, &gap_between, &left_margin);
                }
            }
        }

        // Up sweep: align with children (level below)
        for level in (0..num_levels - 1).rev() {
            let adj_level = level + 1;
            let n = virtual_levels[level].len();

            // Pass 1: right-to-left
            for pos in (0..n).rev() {
                if let Some(tc) = connected_median_x(
                    &virtual_levels[level][pos],
                    &virtual_levels[adj_level],
                    &x_coords[adj_level],
                    &widths[adj_level],
                ) {
                    shift_node(x_coords, level, pos, tc, widths, &gap_between, &left_margin);
                }
            }

            // Pass 2: left-to-right
            for pos in 0..n {
                if let Some(tc) = connected_median_x(
                    &virtual_levels[level][pos],
                    &virtual_levels[adj_level],
                    &x_coords[adj_level],
                    &widths[adj_level],
                ) {
                    shift_node(x_coords, level, pos, tc, widths, &gap_between, &left_margin);
                }
            }
        }
    }
}

/// Compact subgraphs by pulling member nodes toward the subgraph centroid.
///
/// After x-refinement, nodes may be far from their subgraph siblings because
/// they followed cross-subgraph edge connections. This pass identifies such
/// outliers and shifts them inward, respecting same-level gap constraints.
/// When a member is blocked by non-member neighbors, it cascades the push
/// to those neighbors to make room.
fn compact_subgraphs<A: Axis>(
    dag: &Graph<'_>,
    virtual_levels: &[Vec<VNode>],
    x_coords: &mut [Vec<usize>],
    widths: &[Vec<usize>],
    node_spacing: usize,
) {
    use crate::algorithms::sugiyama::subgraph::vnode_subgraph;

    // Helper: minimum gap between two adjacent positions on a level.
    let gap_between = |level: usize, left_pos: usize, right_pos: usize| -> usize {
        let left_sg = vnode_subgraph(dag, &virtual_levels[level][left_pos]);
        let right_sg = vnode_subgraph(dag, &virtual_levels[level][right_pos]);
        if left_sg != right_sg {
            A::SG_GAP_CROSS
        } else {
            node_spacing
        }
    };

    // Cascading push: shift node at `pos` on `level` to `target_x`, and push
    // all neighbors in `direction` as needed to maintain gap constraints.
    // direction: -1 = push left, +1 = push right
    let cascade_push = |x_coords: &mut [Vec<usize>],
                        level: usize,
                        pos: usize,
                        target_x: usize,
                        direction: isize| {
        x_coords[level][pos] = target_x;
        if direction < 0 {
            // Push leftward: fix pos-1, pos-2, ... if they overlap
            let mut i = pos;
            while i > 0 {
                let g = gap_between(level, i - 1, i);
                let needed = x_coords[level][i].saturating_sub(widths[level][i - 1] + g);
                if x_coords[level][i - 1] <= needed {
                    break; // no overlap
                }
                // Can't push past left edge
                let margin = if i - 1 == 0 {
                    let sg = vnode_subgraph(dag, &virtual_levels[level][0]);
                    crate::algorithms::sugiyama::subgraph::leading_cross_pad::<A>(dag, sg)
                } else {
                    0
                };
                x_coords[level][i - 1] = needed.max(margin);
                i -= 1;
            }
        } else {
            // Push rightward: fix pos+1, pos+2, ...
            let n = x_coords[level].len();
            let mut i = pos;
            while i + 1 < n {
                let g = gap_between(level, i, i + 1);
                let needed = x_coords[level][i] + widths[level][i] + g;
                if x_coords[level][i + 1] >= needed {
                    break;
                }
                x_coords[level][i + 1] = needed;
                i += 1;
            }
        }
    };

    let sg_ids: Vec<usize> = dag.subgraphs.iter().map(|s| s.id).collect();

    for &sg_id in &sg_ids {
        let mut members: Vec<(usize, usize)> = Vec::new();

        for (level, vnodes) in virtual_levels.iter().enumerate() {
            for (pos, vnode) in vnodes.iter().enumerate() {
                if vnode_subgraph(dag, vnode) == Some(sg_id) {
                    members.push((level, pos));
                }
            }
        }

        if members.len() <= 1 {
            continue;
        }

        // Compute centroid from real nodes only (dummies follow edges and
        // shouldn't anchor the center).
        let real_members: Vec<(usize, usize)> = members
            .iter()
            .filter(|&&(l, p)| matches!(virtual_levels[l][p], VNode::Real(_)))
            .copied()
            .collect();
        let centroid_members = if real_members.is_empty() {
            &members
        } else {
            &real_members
        };
        let sum: usize = centroid_members
            .iter()
            .map(|&(l, p)| x_coords[l][p] + widths[l][p] / 2)
            .sum();
        let centroid = sum / centroid_members.len();

        // Sort members by distance from centroid (farthest first).
        let mut by_distance: Vec<(usize, usize, usize)> = members
            .iter()
            .map(|&(l, p)| {
                let cx = x_coords[l][p] + widths[l][p] / 2;
                let dist = cx.abs_diff(centroid);
                (l, p, dist)
            })
            .collect();
        by_distance.sort_by(|a, b| b.2.cmp(&a.2));

        for &(level, pos, dist) in &by_distance {
            if dist < A::SG_GAP_CROSS {
                continue; // close enough, don't bother
            }

            let my_w = widths[level][pos];
            let my_cx = x_coords[level][pos] + my_w / 2;
            let target_x = centroid.saturating_sub(my_w / 2);

            // Simple constraint check (no cascading) first.
            let n = x_coords[level].len();
            let min_x = if pos == 0 {
                let sg = vnode_subgraph(dag, &virtual_levels[level][0]);
                crate::algorithms::sugiyama::subgraph::leading_cross_pad::<A>(dag, sg)
            } else {
                let g = gap_between(level, pos - 1, pos);
                x_coords[level][pos - 1] + widths[level][pos - 1] + g
            };
            let max_x = if pos + 1 < n {
                let g = gap_between(level, pos, pos + 1);
                x_coords[level][pos + 1].saturating_sub(my_w + g)
            } else {
                usize::MAX
            };

            let simple_x = target_x.max(min_x).min(max_x);

            // Check if simple move is sufficient.
            let simple_ok = (my_cx > centroid && simple_x < x_coords[level][pos])
                || (my_cx < centroid && simple_x > x_coords[level][pos]);

            if simple_ok {
                x_coords[level][pos] = simple_x;
            } else if my_cx > centroid && target_x < min_x {
                // Need to push left neighbors leftward to make room.
                // Only cascade for real nodes to avoid over-disturbing the layout.
                if matches!(virtual_levels[level][pos], VNode::Real(_)) {
                    // Limit how far we push: at most half the distance to centroid
                    let push_target = (x_coords[level][pos] + target_x) / 2;
                    cascade_push(x_coords, level, pos, push_target, -1);
                }
            } else if my_cx < centroid && target_x > max_x {
                // Need to push right neighbors rightward.
                if matches!(virtual_levels[level][pos], VNode::Real(_)) {
                    let push_target = (x_coords[level][pos] + target_x) / 2;
                    cascade_push(x_coords, level, pos, push_target, 1);
                }
            }
        }
    }
}

/// Crossing reduction for virtual levels (includes dummy nodes).
///
/// Dispatches through the provided [`CrossingReducer`] pipeline.  Each reducer
/// (Median / AdjacentExchange) runs its configured number of passes, each
/// consisting of a top-down sweep followed by a bottom-up sweep.  The
/// pipeline runs uniformly regardless of graph size — behaviour is controlled
/// only by the user-facing presets or manual configuration.
fn reduce_crossings_virtual(
    dag: &Graph<'_>,
    levels: &mut [Vec<VNode>],
    _node_levels: &[usize],
    max_level: usize,
    crossing_pipeline: &[CrossingReducer],
) {
    // Build edge lookup for complete neighbor gathering (real + dummy).
    // This lets real nodes discover skip-level edge dummies on the adjacent
    // level, which is critical for correct median and adjacent-exchange.
    let node_edge_indices = build_node_edge_indices(dag);

    // Pre-allocate reusable buffers to avoid allocations in the hot loop
    let max_level_size = levels.iter().map(|l| l.len()).max().unwrap_or(0);

    let mut real_pos: HashMap<usize, usize> = HashMap::new();
    let mut dummy_pos: HashMap<usize, usize> = HashMap::new();

    // Reusable buffers for median computation
    let mut node_medians: Vec<(VNode, f32)> = Vec::with_capacity(max_level_size);
    let mut connected_positions: Vec<usize> = Vec::with_capacity(8);

    // Reusable buffers for adjacent exchange
    let mut u_positions: Vec<usize> = Vec::with_capacity(8);
    let mut v_positions: Vec<usize> = Vec::with_capacity(8);

    for reducer in crossing_pipeline {
        match reducer {
            CrossingReducer::Median(passes) => {
                for _pass in 0..*passes {
                    // Top-down pass
                    for level_idx in 1..=max_level {
                        let (prev_levels, rest) = levels.split_at_mut(level_idx);
                        let parent_level = &prev_levels[level_idx - 1];
                        order_virtual_by_median(
                            dag,
                            &mut rest[0],
                            parent_level,
                            true,
                            &mut real_pos,
                            &mut dummy_pos,
                            &mut node_medians,
                            &mut connected_positions,
                            &node_edge_indices,
                        );
                    }
                    // Bottom-up pass
                    for level_idx in (0..max_level).rev() {
                        let (left, right) = levels.split_at_mut(level_idx + 1);
                        let child_level = &right[0];
                        order_virtual_by_median(
                            dag,
                            &mut left[level_idx],
                            child_level,
                            false,
                            &mut real_pos,
                            &mut dummy_pos,
                            &mut node_medians,
                            &mut connected_positions,
                            &node_edge_indices,
                        );
                    }
                }
            }
            CrossingReducer::AdjacentExchange(passes) => {
                for _pass in 0..*passes {
                    // Top-down pass
                    for level_idx in 1..=max_level {
                        let (prev_levels, rest) = levels.split_at_mut(level_idx);
                        let parent_level = &prev_levels[level_idx - 1];
                        adjacent_exchange_virtual(
                            dag,
                            &mut rest[0],
                            parent_level,
                            true,
                            &mut real_pos,
                            &mut dummy_pos,
                            &mut u_positions,
                            &mut v_positions,
                            &node_edge_indices,
                        );
                    }
                    // Bottom-up pass
                    for level_idx in (0..max_level).rev() {
                        let (left, right) = levels.split_at_mut(level_idx + 1);
                        let child_level = &right[0];
                        adjacent_exchange_virtual(
                            dag,
                            &mut left[level_idx],
                            child_level,
                            false,
                            &mut real_pos,
                            &mut dummy_pos,
                            &mut u_positions,
                            &mut v_positions,
                            &node_edge_indices,
                        );
                    }
                }
            }
        }
    }
}

/// Adjacent exchange on virtual-node levels: swap adjacent pairs if it reduces crossings.
fn adjacent_exchange_virtual(
    dag: &Graph<'_>,
    level_nodes: &mut [VNode],
    adj_level: &[VNode],
    use_parents: bool,
    real_pos: &mut HashMap<usize, usize>,
    dummy_pos: &mut HashMap<usize, usize>,
    u_positions: &mut Vec<usize>,
    v_positions: &mut Vec<usize>,
    node_edge_indices: &[Vec<usize>],
) {
    if level_nodes.len() < 2 {
        return;
    }

    // Build position maps for the adjacent level
    real_pos.clear();
    dummy_pos.clear();

    for (pos, vnode) in adj_level.iter().enumerate() {
        match vnode {
            VNode::Real(idx) => {
                real_pos.insert(*idx, pos);
            }
            VNode::Dummy { edge_idx } => {
                dummy_pos.insert(*edge_idx, pos);
            }
        }
    }

    for i in 0..level_nodes.len() - 1 {
        u_positions.clear();
        v_positions.clear();

        // Gather neighbour positions for node at position i
        gather_vnode_positions(
            dag,
            &level_nodes[i],
            use_parents,
            real_pos,
            dummy_pos,
            node_edge_indices,
            u_positions,
        );

        // Gather neighbour positions for node at position i+1
        gather_vnode_positions(
            dag,
            &level_nodes[i + 1],
            use_parents,
            real_pos,
            dummy_pos,
            node_edge_indices,
            v_positions,
        );

        let (cross_uv, cross_vu) = count_crossings_pair(u_positions, v_positions);
        if cross_vu < cross_uv {
            level_nodes.swap(i, i + 1);
        }
    }
}

/// Gather positions of a VNode's neighbours in the adjacent level.
///
/// For real nodes, checks ALL edges (both direct and skip-level) by looking up
/// both `real_pos` (for the other endpoint) and `dummy_pos` (for intermediate
/// dummy nodes). This ensures skip-level edges are properly accounted for in
/// crossing reduction.
///
/// For dummy nodes, checks both endpoints of the edge (handling back edges
/// correctly) and the same edge's dummy on the adjacent level.
#[inline]
fn gather_vnode_positions(
    dag: &Graph<'_>,
    vnode: &VNode,
    _use_parents: bool,
    real_pos: &HashMap<usize, usize>,
    dummy_pos: &HashMap<usize, usize>,
    node_edge_indices: &[Vec<usize>],
    out: &mut Vec<usize>,
) {
    match (vnode.real_index(), vnode.dummy_edge()) {
        (Some(idx), _) => {
            // Real node: check all edges this node participates in.
            // For each edge, look for the other endpoint OR a dummy on the adjacent level.
            // The position maps (real_pos/dummy_pos) are built from the adjacent level,
            // so only entries actually present on that level will be found.
            for &edge_idx in &node_edge_indices[idx] {
                let &(from_id, to_id, _) = &dag.edges[edge_idx];

                // Check if the other real endpoint is on the adjacent level
                if let Some(from_idx) = dag.node_index(from_id) {
                    if from_idx != idx {
                        if let Some(&rp) = real_pos.get(&from_idx) {
                            out.push(rp);
                        }
                    }
                }
                if let Some(to_idx) = dag.node_index(to_id) {
                    if to_idx != idx {
                        if let Some(&rp) = real_pos.get(&to_idx) {
                            out.push(rp);
                        }
                    }
                }

                // Check if this edge has a dummy on the adjacent level
                if let Some(&dp) = dummy_pos.get(&edge_idx) {
                    out.push(dp);
                }
            }
        }
        (_, Some(edge_idx)) => {
            let &(from_id, to_id, _) = &dag.edges[edge_idx];

            // Check for same edge's dummy in adjacent level
            if let Some(&dpos) = dummy_pos.get(&edge_idx) {
                out.push(dpos);
            }

            // Check for BOTH real endpoints in adjacent level.
            // This correctly handles back edges where from/to are reversed
            // relative to the layout direction.
            if let Some(fidx) = dag.node_index(from_id) {
                if let Some(&rpos) = real_pos.get(&fidx) {
                    out.push(rpos);
                }
            }
            if let Some(tidx) = dag.node_index(to_id) {
                if let Some(&rpos) = real_pos.get(&tidx) {
                    out.push(rpos);
                }
            }
        }
        _ => {}
    }
}

/// Order virtual nodes by median position of connected nodes in adjacent level.
///
/// Reuses pre-allocated buffers to avoid per-pass allocations in the hot loop.
fn order_virtual_by_median(
    dag: &Graph<'_>,
    level_nodes: &mut Vec<VNode>,
    adj_level: &[VNode],
    _use_parents: bool,
    real_pos: &mut HashMap<usize, usize>,
    dummy_pos: &mut HashMap<usize, usize>,
    node_medians: &mut Vec<(VNode, f32)>,
    connected_positions: &mut Vec<usize>,
    node_edge_indices: &[Vec<usize>],
) {
    // Clear and rebuild lookup tables (reuses allocated capacity)
    real_pos.clear();
    dummy_pos.clear();

    for (pos, vnode) in adj_level.iter().enumerate() {
        match vnode {
            VNode::Real(idx) => {
                real_pos.insert(*idx, pos);
            }
            VNode::Dummy { edge_idx } => {
                dummy_pos.insert(*edge_idx, pos);
            }
        }
    }

    // Clear and rebuild medians (reuses allocated capacity)
    node_medians.clear();

    for (pos, &vnode) in level_nodes.iter().enumerate() {
        // Clear and reuse connected_positions
        connected_positions.clear();

        match (vnode.real_index(), vnode.dummy_edge()) {
            (Some(idx), _) => {
                // Real node: check all edges for connections on adjacent level
                for &edge_idx in &node_edge_indices[idx] {
                    let &(from_id, to_id, _) = &dag.edges[edge_idx];

                    // Check other real endpoint
                    if let Some(from_idx) = dag.node_index(from_id) {
                        if from_idx != idx {
                            if let Some(&p) = real_pos.get(&from_idx) {
                                connected_positions.push(p);
                            }
                        }
                    }
                    if let Some(to_idx) = dag.node_index(to_id) {
                        if to_idx != idx {
                            if let Some(&p) = real_pos.get(&to_idx) {
                                connected_positions.push(p);
                            }
                        }
                    }

                    // Check for dummy on adjacent level
                    if let Some(&dp) = dummy_pos.get(&edge_idx) {
                        connected_positions.push(dp);
                    }
                }
            }
            (_, Some(edge_idx)) => {
                // Dummy node - find the connected node or dummy for this edge
                let &(from_id, to_id, _) = &dag.edges[edge_idx];

                // Check for same edge's dummy in adjacent level
                if let Some(&dpos) = dummy_pos.get(&edge_idx) {
                    connected_positions.push(dpos);
                }

                // Check both real endpoints (handles back edges correctly)
                if let Some(fidx) = dag.node_index(from_id) {
                    if let Some(&rpos) = real_pos.get(&fidx) {
                        connected_positions.push(rpos);
                    }
                }
                if let Some(tidx) = dag.node_index(to_id) {
                    if let Some(&rpos) = real_pos.get(&tidx) {
                        connected_positions.push(rpos);
                    }
                }
            }
            _ => {}
        };

        let median = if connected_positions.is_empty() {
            pos as f32
        } else {
            connected_positions.sort_unstable();
            if connected_positions.len() % 2 == 1 {
                connected_positions[connected_positions.len() / 2] as f32
            } else {
                let mid = connected_positions.len() / 2;
                (connected_positions[mid - 1] + connected_positions[mid]) as f32 / 2.0
            }
        };

        node_medians.push((vnode, median));
    }

    node_medians.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());

    // Rebuild level_nodes from sorted medians
    level_nodes.clear();
    for (v, _) in node_medians.iter() {
        level_nodes.push(*v);
    }
}

#[cfg(all(test, feature = "layout-horizontal"))]
mod horizontal_profile {
    use super::compute_layout_cfg;
    use crate::algorithms::sugiyama::config::LayoutConfig;
    use crate::algorithms::sugiyama::geometry::Horizontal;
    use crate::graph::Graph;
    use crate::render::engine::CustomNode;

    /// P1-S1 node spine (temp/08): under `Horizontal`, levels become
    /// COLUMNS (x grows with level) and declared node dimensions
    /// reach the IR unchanged. Edge geometry is NOT asserted — the
    /// routing region rewrites in P1-S2.
    #[test]
    fn chain_levels_become_columns() {
        let mut g = Graph::new();
        g.add_node(1, "one");
        g.add_node(
            2,
            CustomNode {
                label: "wide",
                width: 12,
                height: 5,
                painter: None,
                payload: "",
            },
        );
        g.add_node(3, "three");
        g.add_edge(1, 2, None);
        g.add_edge(2, 3, None);
        let ir = compute_layout_cfg::<Horizontal>(&g, &LayoutConfig::standard());

        let n = |id: usize| ir.nodes.iter().find(|n| n.id == id).expect("node");
        assert!(n(2).x >= n(1).x + n(1).width, "level 1 starts past level 0");
        assert!(n(3).x >= n(2).x + n(2).width, "level 2 starts past level 1");
        assert_eq!((n(2).width, n(2).height), (12, 5));
        assert!(ir.height >= 5, "canvas covers the tallest node");
        assert!(
            ir.width >= n(3).x + n(3).width,
            "canvas covers the level span"
        );
    }

    /// Siblings on one level stack vertically with disjoint spans.
    #[test]
    fn siblings_stack_vertically() {
        let mut g = Graph::new();
        g.add_node(0, "root");
        g.add_node(1, "a");
        g.add_node(2, "b");
        g.add_edge(0, 1, None);
        g.add_edge(0, 2, None);
        let ir = compute_layout_cfg::<Horizontal>(&g, &LayoutConfig::standard());
        let n = |id: usize| ir.nodes.iter().find(|n| n.id == id).expect("node");
        assert_eq!(n(1).x, n(2).x, "same level, same column start");
        let (top, bot) = if n(1).y <= n(2).y {
            (n(1), n(2))
        } else {
            (n(2), n(1))
        };
        assert!(top.y + top.height <= bot.y, "disjoint vertical spans");
    }

    /// P1-S2: edges materialize with horizontal trunks — `flow_axis`
    /// is `X`, the source port sits at (or past) the source's right
    /// face, the target port on the target's left column, and both
    /// port rows are the nodes' cross-port lines (`y + (h−1)/2`).
    #[test]
    fn edges_materialize_with_horizontal_trunks() {
        use crate::ir::FlowAxis;
        let mut g = Graph::new();
        g.add_node(1, "one");
        g.add_node(
            2,
            CustomNode {
                label: "tall",
                width: 8,
                height: 5,
                painter: None,
                payload: "",
            },
        );
        g.add_edge(1, 2, None);
        let ir = compute_layout_cfg::<Horizontal>(&g, &LayoutConfig::standard());
        let n = |id: usize| ir.nodes.iter().find(|n| n.id == id).expect("node");
        let e = &ir.edges[0];
        assert_eq!(e.flow_axis, FlowAxis::X);
        assert_eq!(
            e.from_x,
            n(1).x + n(1).width - 1,
            "source port exactly on the node's right face"
        );
        assert_eq!(e.to_x, n(2).x, "target port on the left column");
        assert_eq!(e.from_y, n(1).y + (n(1).height - 1) / 2);
        assert_eq!(e.to_y, n(2).y + (n(2).height - 1) / 2);
    }

    /// P1-S2: a self-loop marker under `Horizontal` sits one row BELOW
    /// the node's bottom at its leading column (cross-trailing,
    /// level-leading — D5a), with the derived-`has_self_loop`
    /// invariant intact.
    #[test]
    fn self_loop_marker_sits_below_in_lr() {
        let mut g = Graph::new();
        g.add_node(1, "a");
        g.add_node(2, "b");
        g.add_edge(1, 2, None);
        g.add_edge(1, 1, None);
        let ir = compute_layout_cfg::<Horizontal>(&g, &LayoutConfig::standard());
        let n = |id: usize| ir.nodes.iter().find(|n| n.id == id).expect("node");
        let a = n(1);
        assert!(a.has_self_loop);
        assert_eq!(a.self_loop_at, Some((a.x, a.y + a.height)));
        assert_eq!(n(2).self_loop_at, None);
    }

    /// Slices-2 review: in a MIXED-WIDTH level, each source's port
    /// sits on its OWN right face — not the widest sibling's line.
    #[test]
    fn mixed_width_level_ports_sit_on_own_faces() {
        let mut g = Graph::new();
        g.add_node(1, "a");
        g.add_node(
            2,
            CustomNode {
                label: "very-wide",
                width: 14,
                height: 1,
                painter: None,
                payload: "",
            },
        );
        g.add_node(3, "sink");
        g.add_edge(1, 3, None);
        g.add_edge(2, 3, None);
        let ir = compute_layout_cfg::<Horizontal>(&g, &LayoutConfig::standard());
        let n = |id: usize| ir.nodes.iter().find(|n| n.id == id).expect("node");
        for e in ir.edges.iter() {
            let src = n(e.from_id);
            assert_eq!(
                e.from_x,
                src.x + src.width - 1,
                "edge {}→{} port on its own face",
                e.from_id,
                e.to_id
            );
            assert_eq!(e.to_x, n(e.to_id).x);
        }
    }

    /// Slices-2 review: a two-node cycle must NOT shift its endpoints
    /// off the shared port — Horizontal nodes are one row tall, so
    /// the Vertical ±1 separation would leave the node face entirely.
    /// (Trunk-lane separation for the overlapping pair is paint-time
    /// work — temp/08 P3.)
    #[test]
    fn two_cycle_keeps_ports_on_faces() {
        let mut g = Graph::new();
        g.add_node(1, "a");
        g.add_node(2, "b");
        g.add_edge(1, 2, None);
        g.add_edge(2, 1, None);
        let ir = compute_layout_cfg::<Horizontal>(&g, &LayoutConfig::standard());
        let n = |id: usize| ir.nodes.iter().find(|n| n.id == id).expect("node");
        for e in ir.edges.iter() {
            for (x, y, node) in [
                (e.from_x, e.from_y, n(e.from_id)),
                (e.to_x, e.to_y, n(e.to_id)),
            ] {
                // Endpoints are computed in layout order (back edges
                // swap), so check against BOTH endpoints' spans.
                let on_a = n(1).y <= y && y < n(1).y + n(1).height;
                let on_b = n(2).y <= y && y < n(2).y + n(2).height;
                assert!(
                    on_a || on_b,
                    "endpoint ({x}, {y}) of {}→{} is on neither node's rows ({:?})",
                    e.from_id,
                    e.to_id,
                    (node.y, node.height)
                );
            }
        }
    }

    /// Slices-2 review: dummy draw offsets must stay inside their
    /// packed reservation — under `Horizontal` (DUMMY_CROSS = 1, the
    /// recommended `node_spacing = 1`) the Vertical `edge_idx % 4`
    /// spread would walk into the next span. Emitted dummies must not
    /// overlap any node or each other.
    #[test]
    fn dummy_offsets_stay_inside_their_reservation() {
        let mut g = Graph::new();
        g.add_node(1, "a");
        g.add_node(2, "b");
        g.add_node(3, "c");
        g.add_node(4, "d");
        g.add_edge(1, 2, None);
        g.add_edge(2, 3, None);
        g.add_edge(1, 3, None); // skip level 1
        g.add_edge(1, 4, None);
        g.add_edge(2, 4, None); // more skips through level 2
        let mut config = LayoutConfig::standard();
        config.node_spacing = 1;
        config.include_dummy_nodes = true;
        let ir = compute_layout_cfg::<Horizontal>(&g, &config);
        let spans: alloc::vec::Vec<_> = ir
            .nodes
            .iter()
            .map(|n| (n.id, n.x, n.y, n.width, n.height))
            .collect();
        for (i, &(id_a, ax, ay, aw, ah)) in spans.iter().enumerate() {
            for &(id_b, bx, by, bw, bh) in spans.iter().skip(i + 1) {
                let overlap = ax < bx + bw && bx < ax + aw && ay < by + bh && by < ay + ah;
                assert!(!overlap, "spans of {id_a} and {id_b} overlap");
            }
        }
    }

    /// P1-S3: a cluster box under `Horizontal` wraps its members with
    /// the D3 pads — cross-leading 3 (border + label row + blank),
    /// cross-trailing 2, level pads 2 each side — and outside nodes
    /// stay clear of the box.
    #[test]
    fn lr_box_wraps_members_with_d3_pads() {
        let mut g = Graph::new();
        g.add_node(1, "in");
        g.add_node(2, "a");
        g.add_node(3, "b");
        g.add_node(4, "out");
        g.add_edge(1, 2, None);
        g.add_edge(1, 3, None);
        g.add_edge(2, 4, None);
        g.add_edge(3, 4, None);
        let sg = g.add_subgraph("Box");
        g.put_nodes(&[2usize, 3]).inside(sg).unwrap();
        let ir = compute_layout_cfg::<Horizontal>(&g, &LayoutConfig::standard());
        let b = &ir.subgraphs[0];
        let n = |id: usize| ir.nodes.iter().find(|n| n.id == id).expect("node");
        for id in [2usize, 3] {
            let m = n(id);
            assert!(m.y >= b.y + 3, "cross-leading pad (border+label+blank)");
            assert!(m.y + m.height + 2 <= b.y + b.height, "cross-trailing pad");
            assert!(m.x >= b.x + 2, "level leading pad");
            assert!(m.x + m.width + 2 <= b.x + b.width, "level trailing pad");
        }
        for id in [1usize, 4] {
            let o = n(id);
            let overlap = o.x < b.x + b.width
                && b.x < o.x + o.width
                && o.y < b.y + b.height
                && b.y < o.y + o.height;
            assert!(!overlap, "outside node {id} overlaps the box");
        }
    }

    /// P1-S3 / D8(b): a long box label widens the box on the LEVEL
    /// axis (x in LR) — never its height — and the offset reservation
    /// keeps the next column clear of the widened box.
    #[test]
    fn lr_long_label_widens_box_not_height() {
        let mut g = Graph::new();
        g.add_node(1, "a");
        g.add_node(2, "next");
        g.add_edge(1, 2, None);
        let sg = g.add_subgraph("A Rather Long Cluster Label");
        g.put_nodes(&[1usize]).inside(sg).unwrap();
        let ir = compute_layout_cfg::<Horizontal>(&g, &LayoutConfig::standard());
        let b = &ir.subgraphs[0];
        let label_min = "A Rather Long Cluster Label".len() + 4;
        assert!(
            b.width >= label_min,
            "label widens the box: width {} < {label_min}",
            b.width
        );
        let member = ir.nodes.iter().find(|n| n.id == 1).expect("member");
        assert_eq!(
            b.height,
            member.height + 3 + 2,
            "height stays member + cross pads — labels never grow it"
        );
        let next = ir.nodes.iter().find(|n| n.id == 2).expect("next");
        assert!(
            next.x >= b.x + b.width,
            "next column ({}) clear of the widened box (ends {})",
            next.x,
            b.x + b.width
        );
    }
}
