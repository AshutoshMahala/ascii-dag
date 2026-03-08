//! CSR-based arena layout computation.
//!
//! Pure-CSR layout pipeline: avoids all heap allocations and HashMap lookups
//! by operating directly on CSR graph indices.

use crate::graph::arena::Arena;
use crate::graph::csr::CsrGraph;
use crate::ir::arena::{EdgePathArena, LayoutEdgeArena, LayoutIRArena, LayoutIRArenaBuilder};
use super::config::{CycleBreaking, LayoutConfig};
use super::crossing::CrossingReducer;
use crate::errors::GraphError;

// Import configurable index types
#[cfg(feature = "arena")]
use super::idx::{Coord, Idx, MAX_LEVELS, MAX_NODES};

// Fallback types when arena feature not enabled (for compilation)
#[cfg(not(feature = "arena"))]
type Idx = u32;
#[cfg(not(feature = "arena"))]
type Coord = u16;
#[cfg(not(feature = "arena"))]
const MAX_NODES: usize = u32::MAX as usize;
#[cfg(not(feature = "arena"))]
const MAX_LEVELS: usize = usize::MAX;

/// Maximum horizontal routing slots per level (caps height on extreme fan-out).
const MAX_SLOTS_PER_LEVEL: usize = 8;

/// Temporary buffers for arena-based layout computation.
///
/// All slices are allocated from a single arena. This struct is used by both
/// the CsrGraph layout path and the Graph→CsrGraph path.
#[allow(dead_code)] // Some fields only used by Graph→CsrGraph path in layout/arena.rs
pub(crate) struct LayoutTemps<'a> {
    pub(crate) node_levels: &'a mut [Idx],
    pub(crate) edge_indices: &'a mut [(Idx, Idx)],
    pub(crate) vlevel_offsets: &'a mut [Idx],
    pub(crate) level_counts: &'a mut [Idx],
    pub(crate) vnode_data: &'a mut [Idx],
    pub(crate) x_coords: &'a mut [Coord],
    pub(crate) widths: &'a mut [Coord],
    pub(crate) real_coords: &'a mut [(usize, usize, usize, usize)],
    pub(crate) dummy_offsets: &'a mut [Idx],
    pub(crate) dummy_data: &'a mut [(Idx, Coord)],
    pub(crate) medians: &'a mut [(Idx, u32)],
    pub(crate) positions: &'a mut [Idx],
    pub(crate) node_is_source: &'a mut [bool],
    pub(crate) source_counts: &'a mut [Idx],
    pub(crate) dummy_counts: &'a mut [Idx],
    pub(crate) level_y_offsets: &'a mut [usize],
    pub(crate) node_slots: &'a mut [usize],
    pub(crate) level_slot_next: &'a mut [Idx],
    pub(crate) slot_bounds: &'a mut [(usize, usize)],
    pub(crate) level_dummy_next: &'a mut [Idx],
    pub(crate) waypoint_scratch: &'a mut [(usize, usize)],
    pub(crate) level_vdummy_counts: &'a mut [Idx],

    // ── Subgraph temporaries ─────────────────────────────────────────
    /// Per-subgraph (first_level, last_level) range; usize::MAX = unset
    pub(crate) sg_ranges: &'a mut [(usize, usize)],
    /// Per-subgraph nesting depth
    pub(crate) sg_depths: &'a mut [usize],
    /// Per-subgraph bounding box: (min_x, min_y, max_x, max_y)
    pub(crate) sg_envelopes: &'a mut [(usize, usize, usize, usize)],
    /// Per-level boundary extras for subgraph borders
    pub(crate) sg_y_extras: &'a mut [usize],
}

// ── Subgraph layout constants ────────────────────────────────────────────
/// Per-subgraph horizontal padding (chars on each side of border).
const SUBGRAPH_H_PAD: usize = 2;
/// Vertical padding above first node: border + label + blank.
const SUBGRAPH_V_PAD_TOP: usize = 3;
/// Vertical padding below last node: blank + border.
const SUBGRAPH_V_PAD_BOTTOM: usize = 2;

/// Compute layout using arena allocation for temporaries, specialized for CsrGraph.
///
/// This avoids all heap allocations and HashMap lookups by using the CSR indices directly.
/// The `config` parameter controls the layout pipeline (crossing reduction, spacing, etc.).
pub fn compute_layout_arena_csr<'b>(
    graph: &CsrGraph<'_>,
    config: &LayoutConfig<'_>,
    temp_arena: &mut Arena<'_>,
    output_arena: &'b mut Arena<'b>,
) -> Result<LayoutIRArena<'b>, GraphError> {
    let node_count = graph.node_count();
    let edge_count = graph.edge_count();

    // Validate against index type limits
    let max_count = node_count.max(edge_count);
    if max_count > MAX_NODES {
        return Err(GraphError::ExceedsMaxNodes { count: max_count, max: MAX_NODES });
    }

    // Calculate total label bytes (node + edge labels, iterating CSR is cheap)
    let mut total_label_bytes = 0;
    for i in 0..node_count {
        total_label_bytes += graph.node_label(i).len();
    }
    let has_labeled_edges = graph.has_edge_labels();
    if has_labeled_edges {
        for i in 0..edge_count {
            total_label_bytes += graph.edge_label(i).len();
        }
    }

    // Estimate max waypoints: for skip-level edges only
    // A skip-level edge spanning k levels needs k-1 waypoints
    // Worst case: all edges span (max_level) levels = edge_count * max_level waypoints
    // But for typical graphs, most edges are adjacent-level (0 waypoints)
    // Use a conservative estimate: avg 2 waypoints per edge (covers most skip edges)
    let max_waypoints = (edge_count * 4).min(1000);

    // Step 1: Cycle breaking — allocate back_edges and run DFS before other temps
    let back_edges = {
        let be_size = edge_count.max(1);
        let (be_ptr, _) = temp_arena.alloc_raw::<bool>(be_size)
            .ok_or(GraphError::ArenaOom)?;
        // Safety: alloc_raw zeroes memory, so all false
        unsafe { core::slice::from_raw_parts_mut(be_ptr, be_size) }
    };
    match config.cycle_breaking() {
        CycleBreaking::DepthFirst => {
            detect_back_edges_csr(graph, back_edges, temp_arena);
        }
        CycleBreaking::None => {} // already all-false from alloc_raw
    }

    // Step 2: Allocate layout temporaries
    let sg_count = graph.subgraph_count();
    let mut temps = alloc_layout_temps_csr(temp_arena, node_count, edge_count, sg_count)
        .ok_or(GraphError::ArenaOom)?;

    // Step 3: Calculate levels (back edges have direction flipped)
    let max_level = calculate_levels_csr(graph, temps.node_levels, back_edges);

    // Step 4: Build virtual levels (back edges have direction flipped)
    let (_vnode_count, _max_level_size) = build_virtual_levels_csr(
        graph,
        temps.node_levels,
        temps.vlevel_offsets,
        temps.level_counts,
        temps.vnode_data,
        max_level,
        back_edges,
    );

    // Populate edge_indices for crossing reduction
    for (i, (from, to)) in graph.edges_iter().enumerate() {
        if i < temps.edge_indices.len() {
            temps.edge_indices[i] = (from as Idx, to as Idx);
        }
    }

    // Populate level_vdummy_counts for crossing reduction
    temps.level_vdummy_counts.fill(0);
    for level in 0..=(max_level as usize) {
        if level + 1 >= temps.vlevel_offsets.len() { break; }
        let start = temps.vlevel_offsets[level] as usize;
        let end = temps.vlevel_offsets[level + 1] as usize;
        for pos in start..end {
            if pos * 2 + 1 < temps.vnode_data.len() && temps.vnode_data[pos * 2] == 1 {
                if level < temps.level_vdummy_counts.len() {
                    temps.level_vdummy_counts[level] += 1;
                }
            }
        }
    }

    // Step 4: Crossing reduction
    reduce_crossings_csr(
        graph,
        config.crossing_pipeline(),
        temps.vlevel_offsets,
        temps.vnode_data,
        max_level as usize,
        temps.medians,
        temps.positions,
        temps.edge_indices,
        temps.level_vdummy_counts,
    );

    // Step 5: Assign x-coordinates
    let mut max_width = assign_x_coords_csr(
        graph,
        temps.vlevel_offsets,
        temps.vnode_data,
        temps.x_coords,
        temps.widths,
        max_level,
    );

    // Step 5b: Subgraph horizontal padding
    if graph.has_subgraphs() {
        let padded = subgraph_padding_csr(
            graph,
            temps.vlevel_offsets,
            temps.vnode_data,
            temps.x_coords,
            temps.widths,
            max_level,
        );
        // Extra margin for outermost border
        let max_depth = {
            let mut d = 0usize;
            for i in 0..graph.subgraph_count() {
                let cd = graph.sg_chain_depth(Some(i));
                if cd > d { d = cd; }
            }
            d
        };
        max_width = (padded + max_depth * SUBGRAPH_H_PAD) as Coord;
    }

    // Step 6: Build real node coordinates
    build_real_coords_csr(
        temps.vlevel_offsets,
        temps.vnode_data,
        temps.x_coords,
        temps.widths,
        temps.real_coords,
        max_level,
        max_width,
        !graph.has_subgraphs(), // skip per-level centering for subgraph layouts
    );

    // Step 6b: Fix sibling subgraph overlaps introduced by centering
    if graph.has_subgraphs() {
        let extra = fix_subgraph_overlaps_csr(
            graph,
            temps.real_coords,
            temps.sg_envelopes,
            temps.sg_depths,
            temps.node_slots,
        );
        max_width = max_width.saturating_add(extra as Coord);
    }

    // Step 7: Build dummy positions using actual virtual level positions
    build_dummy_positions_csr(
        graph,
        temps.vlevel_offsets,
        temps.vnode_data,
        temps.x_coords,
        temps.widths,
        temps.dummy_offsets,
        temps.dummy_data,
        max_level,
        max_width,
    );

    // Step 8: Geometry-aware horizontal slot allocation for edge separation
    // Assigns horizontal routing slots to non-vertical source nodes so that
    // edges whose horizontal spans don't overlap can share the same slot row.
    // This matches the heap path's interval-based slot allocator.
    let alloc_size = max_level as usize + 1;

    // 1. Initialize geometry-aware slot tracking
    temps.node_slots.fill(usize::MAX); // usize::MAX = unassigned sentinel
    temps.level_slot_next.fill(0);
    // Initialize slot bounding boxes to empty: (usize::MAX, 0) means no intervals
    for sb in temps.slot_bounds.iter_mut() {
        *sb = (usize::MAX, 0);
    }

    // 2. Assign slots by scanning edges (same iteration order as Step 9)
    for (ei, (from_idx, to_idx)) in graph.edges_iter().enumerate() {
        if from_idx == to_idx { continue; }
        let is_back = back_edges.get(ei).copied().unwrap_or(false);
        let (src_idx, dst_idx) = if is_back { (to_idx, from_idx) } else { (from_idx, to_idx) };
        if src_idx >= temps.real_coords.len() || dst_idx >= temps.real_coords.len() {
            continue;
        }
        let (src_level, _, src_x_base, src_width) = temps.real_coords[src_idx];
        let (dst_level, _, dst_x_base, dst_width) = temps.real_coords[dst_idx];

        if dst_level <= src_level { continue; }

        let src_x_center = src_x_base + src_width / 2;
        let dst_x_center = dst_x_base + dst_width / 2;
        let is_vertical = src_x_center == dst_x_center && dst_level == src_level + 1;
        if is_vertical { continue; }

        let (min_x, max_x) = if src_x_center < dst_x_center {
            (src_x_center, dst_x_center + 1)
        } else {
            (dst_x_center, src_x_center + 1)
        };

        let lvl = src_level;
        if lvl >= alloc_size { continue; }

        if temps.node_slots[src_idx] != usize::MAX {
            // Source already has a slot — merge interval into its bounding box
            let slot = temps.node_slots[src_idx];
            let base = lvl * MAX_SLOTS_PER_LEVEL + slot;
            if base < temps.slot_bounds.len() {
                let (ref mut bmin, ref mut bmax) = temps.slot_bounds[base];
                if min_x < *bmin { *bmin = min_x; }
                if max_x > *bmax { *bmax = max_x; }
            }
        } else {
            // New source — find a conflict-free slot via greedy first-fit scan
            let slots_used = temps.level_slot_next[lvl] as usize;
            let mut chosen = None;

            for s in 0..slots_used {
                let base = lvl * MAX_SLOTS_PER_LEVEL + s;
                if base < temps.slot_bounds.len() {
                    let (bmin, bmax) = temps.slot_bounds[base];
                    // No overlap: new range is entirely before or after existing bounding box
                    if max_x <= bmin || min_x >= bmax {
                        // Share this slot — merge bounding box
                        let (ref mut sbmin, ref mut sbmax) = temps.slot_bounds[base];
                        if min_x < *sbmin { *sbmin = min_x; }
                        if max_x > *sbmax { *sbmax = max_x; }
                        chosen = Some(s);
                        break;
                    }
                }
            }

            let slot = if let Some(s) = chosen {
                s
            } else if slots_used < MAX_SLOTS_PER_LEVEL {
                // Allocate new slot
                let s = slots_used;
                let base = lvl * MAX_SLOTS_PER_LEVEL + s;
                if base < temps.slot_bounds.len() {
                    temps.slot_bounds[base] = (min_x, max_x);
                }
                temps.level_slot_next[lvl] += 1;
                s
            } else {
                // Cap reached — degrade to slot 0
                0
            };

            temps.node_slots[src_idx] = slot;
        }
    }

    // 3. Count dummy nodes per level
    temps.dummy_counts.fill(0);
    let total_dummy_waypoints = temps.dummy_offsets[edge_count] as usize;
    for &(level, _) in &temps.dummy_data[..total_dummy_waypoints] {
        let lvl = level as usize;
        if lvl < alloc_size {
            temps.dummy_counts[lvl] += 1;
        }
    }

    // 4. Compute per-level max node height and Y offsets
    // Repurpose level_vdummy_counts for max_node_heights (no longer needed after crossing reduction)
    let max_node_heights = &mut temps.level_vdummy_counts[..alloc_size];
    for h in max_node_heights.iter_mut() {
        *h = 1 as Idx; // default height 1
    }
    for idx in 0..node_count {
        let level = temps.real_coords[idx].0;
        let h = graph.node_height(idx) as Idx;
        if level < alloc_size && h > max_node_heights[level] {
            max_node_heights[level] = h;
        }
    }

    temps.level_y_offsets.fill(0);
    let routing_overhead: usize = if has_labeled_edges { 4 } else { 2 };

    // Compute subgraph Y extras (vertical border space)
    let (sg_initial_offset, sg_trailing_extra) = if graph.has_subgraphs() {
        compute_sg_y_extras(
            graph,
            temps.node_levels,
            max_level as usize,
            temps.sg_ranges,
            temps.sg_depths,
            temps.sg_y_extras,
        )
    } else {
        (0, 0)
    };

    let mut current_offset = sg_initial_offset;

    for level in 0..=max_level as usize {
        temps.level_y_offsets[level] = current_offset;
        let node_height = max_node_heights[level] as usize;
        // Use actual geometry-aware slot count (not naive source count)
        let slot_count = temps.level_slot_next[level] as usize;
        let diff = slot_count.max(temps.dummy_counts[level] as usize);
        let height = node_height + routing_overhead + diff.saturating_sub(1);
        current_offset += height;
        // Add subgraph border space after this level
        if graph.has_subgraphs() && level < temps.sg_y_extras.len() {
            current_offset += temps.sg_y_extras[level];
        }
    }
    current_offset += sg_trailing_extra;
    temps.level_y_offsets[max_level as usize + 1] = current_offset;
    let total_height = current_offset;

    // Step 9: Build LayoutIRArena
    // Include subgraph label bytes in total label allocation
    let sg_label_bytes = if graph.has_subgraphs() {
        let mut bytes = 0;
        for i in 0..graph.subgraph_count() {
            bytes += graph.subgraph_label(i).len();
        }
        bytes
    } else {
        0
    };
    let mut builder = LayoutIRArenaBuilder::new_with_subgraphs(
        output_arena,
        node_count,
        edge_count,
        max_waypoints,
        total_label_bytes + sg_label_bytes,
        max_level as usize + 1,
        sg_count,
    ).ok_or(GraphError::BuilderFailed)?;

    // Add buffer for edge routing (+4) plus label margin
    let label_margin = if has_labeled_edges { 8 } else { 0 };
    builder.set_dimensions(max_width as usize + 4 + label_margin, total_height);
    builder.set_level_count(max_level as usize + 1);

    // Add nodes
    for idx in 0..node_count {
        let (level, pos, x, width) = temps.real_coords[idx];
        let y = temps.level_y_offsets[level as usize]; // Use dynamic offset
        let id = graph.node_id(idx);
        let label = graph.node_label(idx);

        builder.add_node(
            id,
            label,
            x as usize,
            y,
            width as usize,
            graph.node_height(idx),
            level as usize,
            pos as usize,
            crate::ir::NodeKind::Explicit,
        ).ok_or(GraphError::ArenaOom)?;
        builder.add_node_to_level(level as usize, idx)
            .ok_or(GraphError::ArenaOom)?;
    }

    builder.finalize_levels();

    // Slots are pre-assigned by geometry-aware allocation in Step 8.
    // Only level_dummy_next needs reset for skip-level waypoint slot tracking.
    temps.level_dummy_next.fill(0);

    // Access mutable buffers via temps
    let node_slots = &temps.node_slots;
    let level_dummy_next = &mut temps.level_dummy_next;
    let waypoint_scratch = &mut temps.waypoint_scratch;
    let level_y_offsets = &temps.level_y_offsets;
    let max_node_heights = &temps.level_vdummy_counts;

    // Add edges
    for (edge_idx, (from_idx, to_idx)) in graph.edges_iter().enumerate() {
        // Self-loops: mark the node and skip edge routing
        if from_idx == to_idx {
            builder.set_self_loop(from_idx);
            continue;
        }

        // For back edges, layout direction is reversed (to→from in level space).
        // We compute coordinates in layout order, then store semantic IDs in the IR.
        let is_back = back_edges.get(edge_idx).copied().unwrap_or(false);
        let (layout_src_idx, layout_dst_idx) = if is_back {
            (to_idx, from_idx)
        } else {
            (from_idx, to_idx)
        };

        let (src_level, _, src_x_base, src_width) = temps.real_coords[layout_src_idx];
        let (dst_level, _, dst_x_base, dst_width) = temps.real_coords[layout_dst_idx];

        let from_x = (src_x_base + src_width / 2) as usize;
        let to_x = (dst_x_base + dst_width / 2) as usize;
        // from_y = bottom of source node (top + max_node_height - 1)
        let from_y = level_y_offsets[src_level as usize]
            + max_node_heights[src_level as usize] as usize - 1;
        let to_y = level_y_offsets[dst_level as usize];

        // Store original semantic IDs (not layout-direction IDs)
        let from_id = graph.node_id(from_idx);
        let to_id = graph.node_id(to_idx);

        // Get pre-assigned slot from geometry-aware allocation (Step 8)
        let slot = if dst_level > src_level && node_slots[layout_src_idx] != usize::MAX {
            node_slots[layout_src_idx]
        } else {
            0
        };

        let edge_start_row = 1 + if has_labeled_edges { 1 } else { 0 };

        // Detect 2-node cycle: A→B (forward) + B→A (reversed) sharing the same column.
        // Offset forward edge left by 1 and back-edge right by 1 from center.
        let in_two_node_cycle = from_x == to_x && from_idx != to_idx && {
            let edge_count = graph.edge_count();
            (0..edge_count).any(|ej| {
                if ej == edge_idx { return false; }
                let (f, t) = graph.edge(ej);
                f == to_idx && t == from_idx
                    && back_edges.get(ej).copied().unwrap_or(false) != is_back
            })
        };

        let (eff_from_x, eff_to_x) = if in_two_node_cycle {
            if is_back {
                (from_x + 1, to_x + 1)
            } else {
                (from_x.saturating_sub(1), to_x.saturating_sub(1))
            }
        } else {
            (from_x, to_x)
        };

        let path = if dst_level == src_level + 1 {
            if eff_from_x == eff_to_x {
                EdgePathArena::Direct
            } else {
                EdgePathArena::Corner {
                    horizontal_y: from_y + edge_start_row + slot,
                }
            }
        } else if dst_level > src_level + 1 {
            let dummy_start = temps.dummy_offsets[edge_idx] as usize;
            let dummy_end = temps.dummy_offsets[edge_idx + 1] as usize;
            let dummy_count = dummy_end - dummy_start;

            if dummy_count > 0 && dummy_start < temps.dummy_data.len() {
                // Limit to scratch size
                let available = temps.dummy_data.len().saturating_sub(dummy_start);
                let waypoint_count = dummy_count.min(waypoint_scratch.len()).min(available);

                for i in 0..waypoint_count {
                    let (level, x) = temps.dummy_data[dummy_start + i];
                    let lvl_idx = level as usize;

                    // Assign a unique vertical slot for this edge at this level
                    let dummy_slot = if lvl_idx < alloc_size {
                        let s = level_dummy_next[lvl_idx];
                        level_dummy_next[lvl_idx] += 1;
                        s
                    } else {
                        0
                    };

                    // Calculate Y using level_y_offsets + max_node_height at intermediate level
                    let y_base = level_y_offsets[lvl_idx]
                        + max_node_heights.get(lvl_idx).copied().unwrap_or(1) as usize - 1;
                    waypoint_scratch[i] =
                        (x as usize, y_base + edge_start_row + dummy_slot as usize);
                }

                if let Some((start, len)) =
                    builder.add_waypoints(&waypoint_scratch[..waypoint_count])
                {
                    let start_y_offset = (edge_start_row + slot).saturating_sub(1);
                    EdgePathArena::MultiSegment {
                        waypoints_start: start,
                        waypoints_len: len,
                        start_y_offset,
                    }
                } else {
                    EdgePathArena::Corner {
                        horizontal_y: from_y + edge_start_row + slot,
                    }
                }
            } else {
                EdgePathArena::Corner {
                    horizontal_y: from_y + edge_start_row + slot,
                }
            }
        } else {
            EdgePathArena::Direct
        };

        // Store edge label if present
        let edge_label_text = graph.edge_label(edge_idx);
        let (e_label_offset, e_label_len, e_label_x, e_label_y) =
            if !edge_label_text.is_empty() {
                if let Some((offset, len)) = builder.add_edge_label(edge_label_text) {
                    let l_y = from_y + 2;
                    let edge_x_at_label = match &path {
                        EdgePathArena::Direct => eff_from_x,
                        EdgePathArena::Corner { horizontal_y } => {
                            if l_y <= *horizontal_y { eff_from_x } else { eff_to_x }
                        }
                        EdgePathArena::MultiSegment { .. } => eff_from_x,
                    };
                    let label_len_with_quotes = len + 2;
                    let l_x = edge_x_at_label.saturating_sub(label_len_with_quotes / 2);
                    (offset, len, l_x, l_y)
                } else {
                    (0, 0, 0, 0)
                }
            } else {
                (0, 0, 0, 0)
            };

        builder.add_edge(LayoutEdgeArena {
            from_id,
            to_id,
            from_x: eff_from_x,
            from_y,
            to_x: eff_to_x,
            to_y,
            reversed: is_back,
            path,
            edge_index: edge_idx,
            label_offset: e_label_offset,
            label_len: e_label_len,
            label_x: e_label_x,
            label_y: e_label_y,
            // Edge draws between from_y+1 and to_y-1 (below source, above target)
            min_y: from_y + 1,
            max_y: to_y.saturating_sub(1),
        });
    }

    // Step 10: Compute subgraph bounding boxes and add to builder
    if graph.has_subgraphs() {
        compute_sg_bounding_boxes(
            graph,
            temps.real_coords,
            temps.level_y_offsets,
            total_height,
            temps.sg_depths,
            temps.sg_envelopes,
            &mut builder,
        );
    }

    Ok(builder.build())
}

// Helpers for CSR layout (parallel implementation for CsrGraph)

fn alloc_layout_temps_csr<'b>(
    arena: &'b mut Arena<'_>,
    node_count: usize,
    edge_count: usize,
    sg_count: usize,
) -> Option<LayoutTemps<'b>> {
    // Same allocation logic as DAG
    let max_levels = node_count.min(256);
    // Virtual nodes = real + dummy nodes from skip-level edges.
    // Most edges span only 1 level (no dummies). Skip-level edges typically span 2-4 levels.
    // Use a reasonable estimate: each edge creates at most 4 dummy nodes on average.
    let max_vnodes = (node_count + edge_count * 4).min(500000);
    let max_level_size = node_count.min(50000);
    let max_dummy_waypoints = (edge_count * 4).min(500000);

    let (node_levels_ptr, _) = arena.alloc_raw_uninit::<Idx>(node_count)?;
    let (edge_indices_ptr, _) = arena.alloc_raw_uninit::<(Idx, Idx)>(edge_count)?;
    let (vlevel_offsets_ptr, _) = arena.alloc_raw_uninit::<Idx>(max_levels + 1)?;
    let (level_counts_ptr, _) = arena.alloc_raw_uninit::<Idx>(max_levels)?;
    let (vnode_data_ptr, _) = arena.alloc_raw_uninit::<Idx>(max_vnodes * 2)?;
    let (x_coords_ptr, _) = arena.alloc_raw_uninit::<Coord>(max_vnodes)?;
    let (widths_ptr, _) = arena.alloc_raw_uninit::<Coord>(max_vnodes)?;
    let (real_coords_ptr, _) =
        arena.alloc_raw_uninit::<(usize, usize, usize, usize)>(node_count)?;
    let (dummy_offsets_ptr, _) = arena.alloc_raw_uninit::<Idx>(edge_count + 1)?;
    let (dummy_data_ptr, _) = arena.alloc_raw_uninit::<(Idx, Coord)>(max_dummy_waypoints)?;
    let (medians_ptr, _) = arena.alloc_raw_uninit::<(Idx, u32)>(max_level_size)?;
    let (positions_ptr, _) = arena.alloc_raw_uninit::<Idx>(max_level_size)?;

    // Optimize allocs: boolean array
    let (node_is_source_ptr, _) = arena.alloc_raw_uninit::<bool>(node_count)?;
    // Counters per level
    let (source_counts_ptr, _) = arena.alloc_raw_uninit::<Idx>(max_levels + 1)?;
    let (dummy_counts_ptr, _) = arena.alloc_raw_uninit::<Idx>(max_levels + 1)?;
    let (level_y_offsets_ptr, _) = arena.alloc_raw_uninit::<usize>(max_levels + 2)?;
    // Node slots
    let (node_slots_ptr, _) = arena.alloc_raw_uninit::<usize>(node_count)?;
    // Next slot counters
    let (level_slot_next_ptr, _) = arena.alloc_raw_uninit::<Idx>(max_levels + 1)?;
    // Slot bounding boxes: (min_x, max_x) per (level, slot) for geometry-aware allocation
    let slot_bounds_size = (max_levels + 1) * MAX_SLOTS_PER_LEVEL;
    let (slot_bounds_ptr, _) = arena.alloc_raw_uninit::<(usize, usize)>(slot_bounds_size)?;
    let (level_dummy_next_ptr, _) = arena.alloc_raw_uninit::<Idx>(max_levels + 1)?;
    let (waypoint_scratch_ptr, _) = arena.alloc_raw_uninit::<(usize, usize)>(max_levels + 1)?;
    let (level_vdummy_counts_ptr, _) = arena.alloc_raw_uninit::<Idx>(max_levels + 1)?;

    // Subgraph temporaries (0-length if no subgraphs)
    let sg_alloc = sg_count.max(1); // avoid 0-length allocations
    let (sg_ranges_ptr, _) = if sg_count > 0 {
        arena.alloc_raw_uninit::<(usize, usize)>(sg_alloc)?
    } else {
        (core::ptr::null_mut(), 0)
    };
    let (sg_depths_ptr, _) = if sg_count > 0 {
        arena.alloc_raw_uninit::<usize>(sg_alloc)?
    } else {
        (core::ptr::null_mut(), 0)
    };
    let (sg_envelopes_ptr, _) = if sg_count > 0 {
        arena.alloc_raw_uninit::<(usize, usize, usize, usize)>(sg_alloc)?
    } else {
        (core::ptr::null_mut(), 0)
    };
    let (sg_y_extras_ptr, _) = if sg_count > 0 {
        arena.alloc_raw_uninit::<usize>(max_levels + 1)?
    } else {
        (core::ptr::null_mut(), 0)
    };

    unsafe {
        Some(LayoutTemps {
            node_levels: core::slice::from_raw_parts_mut(node_levels_ptr, node_count),
            edge_indices: core::slice::from_raw_parts_mut(edge_indices_ptr, edge_count),
            vlevel_offsets: core::slice::from_raw_parts_mut(vlevel_offsets_ptr, max_levels + 1),
            level_counts: core::slice::from_raw_parts_mut(level_counts_ptr, max_levels),
            vnode_data: core::slice::from_raw_parts_mut(vnode_data_ptr, max_vnodes * 2),
            x_coords: core::slice::from_raw_parts_mut(x_coords_ptr, max_vnodes),
            widths: core::slice::from_raw_parts_mut(widths_ptr, max_vnodes),
            real_coords: core::slice::from_raw_parts_mut(real_coords_ptr, node_count),
            dummy_offsets: core::slice::from_raw_parts_mut(dummy_offsets_ptr, edge_count + 1),

            node_is_source: core::slice::from_raw_parts_mut(node_is_source_ptr, node_count),
            source_counts: core::slice::from_raw_parts_mut(source_counts_ptr, max_levels + 1),
            dummy_counts: core::slice::from_raw_parts_mut(dummy_counts_ptr, max_levels + 1),
            level_y_offsets: core::slice::from_raw_parts_mut(level_y_offsets_ptr, max_levels + 2),
            node_slots: core::slice::from_raw_parts_mut(node_slots_ptr, node_count),
            level_slot_next: core::slice::from_raw_parts_mut(level_slot_next_ptr, max_levels + 1),
            slot_bounds: core::slice::from_raw_parts_mut(slot_bounds_ptr, slot_bounds_size),
            level_dummy_next: core::slice::from_raw_parts_mut(level_dummy_next_ptr, max_levels + 1),
            waypoint_scratch: core::slice::from_raw_parts_mut(waypoint_scratch_ptr, max_levels + 1),
            level_vdummy_counts: core::slice::from_raw_parts_mut(level_vdummy_counts_ptr, max_levels + 1),
            dummy_data: core::slice::from_raw_parts_mut(dummy_data_ptr, max_dummy_waypoints),
            medians: core::slice::from_raw_parts_mut(medians_ptr, max_level_size),
            positions: core::slice::from_raw_parts_mut(positions_ptr, max_level_size),
            sg_ranges: if sg_count > 0 {
                core::slice::from_raw_parts_mut(sg_ranges_ptr, sg_count)
            } else { &mut [] },
            sg_depths: if sg_count > 0 {
                core::slice::from_raw_parts_mut(sg_depths_ptr, sg_count)
            } else { &mut [] },
            sg_envelopes: if sg_count > 0 {
                core::slice::from_raw_parts_mut(sg_envelopes_ptr, sg_count)
            } else { &mut [] },
            sg_y_extras: if sg_count > 0 {
                core::slice::from_raw_parts_mut(sg_y_extras_ptr, max_levels + 1)
            } else { &mut [] },
        })
    }
}

/// Three-color DFS back-edge detection for CsrGraph.
///
/// Identifies back edges (edges pointing to an ancestor on the DFS stack)
/// using a classic three-color algorithm: WHITE → GRAY (on stack) → BLACK (done).
/// All temporaries are allocated from `arena` — no heap allocation.
///
/// Self-loops (from == to) are unconditionally marked as back edges.
fn detect_back_edges_csr(
    graph: &CsrGraph<'_>,
    back_edges: &mut [bool],
    arena: &mut Arena<'_>,
) {
    let node_count = graph.node_count();
    let edge_count = graph.edge_count();

    for b in back_edges.iter_mut() {
        *b = false;
    }

    if node_count == 0 || edge_count == 0 {
        return;
    }

    // Mark self-loops immediately
    for ei in 0..edge_count {
        let (from, to) = graph.edge(ei);
        if from == to {
            back_edges[ei] = true;
        }
    }

    // Build edge-from CSR: for each source node, the list of outgoing edge indices.
    // Allocate from arena: offsets[node_count+1] + data[edge_count] + color[node_count] + stack[node_count]
    let Some((offsets_ptr, _)) = arena.alloc_raw::<u32>(node_count + 1) else { return };
    let Some((edata_ptr, _)) = arena.alloc_raw::<u32>(edge_count) else { return };
    let Some((color_ptr, _)) = arena.alloc_raw::<u8>(node_count) else { return };
    // Stack entries: (node_index as u32, edge_iterator_position as u32)
    let Some((stack_ptr, _)) = arena.alloc_raw_uninit::<(u32, u32)>(node_count) else { return };

    let offsets = unsafe { core::slice::from_raw_parts_mut(offsets_ptr, node_count + 1) };
    let edata = unsafe { core::slice::from_raw_parts_mut(edata_ptr, edge_count) };
    let color = unsafe { core::slice::from_raw_parts_mut(color_ptr, node_count) };
    let stack = unsafe { core::slice::from_raw_parts_mut(stack_ptr, node_count) };

    // Build edge-from CSR — count then fill
    // offsets already zeroed by alloc_raw
    for ei in 0..edge_count {
        let (from, _) = graph.edge(ei);
        if from < node_count {
            offsets[from + 1] += 1;
        }
    }
    for i in 1..=node_count {
        offsets[i] += offsets[i - 1];
    }
    // fill_counts: reuse color array temporarily (it's zeroed)
    for ei in 0..edge_count {
        let (from, _) = graph.edge(ei);
        if from < node_count {
            let pos = (offsets[from] + color[from] as u32) as usize;
            edata[pos] = ei as u32;
            color[from] += 1;
        }
    }
    // Reset color to WHITE (0) for DFS
    for c in color.iter_mut() {
        *c = 0; // WHITE
    }

    const WHITE: u8 = 0;
    const GRAY: u8 = 1;
    // const BLACK: u8 = 2;

    // Explicit-stack DFS for each unvisited root
    for start in 0..node_count {
        if color[start] != WHITE {
            continue;
        }
        color[start] = GRAY;
        let mut stack_len: usize = 1;
        stack[0] = (start as u32, 0);

        while stack_len > 0 {
            let (node, ref mut ei_pos) = stack[stack_len - 1];
            let node_idx = node as usize;
            let edge_start = offsets[node_idx] as usize;
            let edge_end = offsets[node_idx + 1] as usize;
            let local_pos = *ei_pos as usize;

            if edge_start + local_pos < edge_end {
                let edge_idx = edata[edge_start + local_pos] as usize;
                stack[stack_len - 1].1 += 1; // advance iterator

                let (_, to) = graph.edge(edge_idx);
                if to < node_count {
                    match color[to] {
                        GRAY => {
                            back_edges[edge_idx] = true;
                        }
                        WHITE => {
                            color[to] = GRAY;
                            if stack_len < stack.len() {
                                stack[stack_len] = (to as u32, 0);
                                stack_len += 1;
                            }
                        }
                        _ => {} // BLACK — fully processed
                    }
                }
            } else {
                // All edges from this node exhausted
                color[node_idx] = 2; // BLACK
                stack_len -= 1;
            }
        }
    }
}

// ── Subgraph layout helpers (CSR) ────────────────────────────────────────

/// Resolve subgraph index for a virtual node in the CSR representation.
/// Real nodes use `graph.node_subgraph()`; dummy nodes return the subgraph
/// only if both edge endpoints share the same subgraph.
fn vnode_subgraph_csr(graph: &CsrGraph<'_>, vnode_type: Idx, vnode_idx: Idx) -> Option<usize> {
    if vnode_type == 0 {
        // Real node
        graph.node_subgraph(vnode_idx as usize)
    } else {
        // Dummy node — vnode_idx is edge index
        let (from, to) = graph.edge(vnode_idx as usize);
        let fsg = graph.node_subgraph(from);
        let tsg = graph.node_subgraph(to);
        match (fsg, tsg) {
            (Some(a), Some(b)) if a == b => Some(a),
            _ => None,
        }
    }
}

/// Count subgraph boundary transitions between two subgraph chains.
/// Returns (exits_from_prev + entries_into_curr).
fn sg_boundary_exits_entries(graph: &CsrGraph<'_>, prev: Option<usize>, curr: Option<usize>) -> (usize, usize) {
    // Build chains (root first) using simple iteration — max depth is small
    let mut prev_chain = [0usize; 16];
    let mut prev_len = 0usize;
    {
        let mut c = prev;
        while let Some(idx) = c {
            if idx >= graph.subgraph_count() || prev_len >= 16 { break; }
            prev_chain[prev_len] = idx;
            prev_len += 1;
            c = graph.subgraph_parent(idx);
        }
        // Reverse to get root first
        prev_chain[..prev_len].reverse();
    }

    let mut curr_chain = [0usize; 16];
    let mut curr_len = 0usize;
    {
        let mut c = curr;
        while let Some(idx) = c {
            if idx >= graph.subgraph_count() || curr_len >= 16 { break; }
            curr_chain[curr_len] = idx;
            curr_len += 1;
            c = graph.subgraph_parent(idx);
        }
        curr_chain[..curr_len].reverse();
    }

    // Common prefix length
    let common = prev_chain[..prev_len].iter()
        .zip(curr_chain[..curr_len].iter())
        .take_while(|(a, b)| a == b)
        .count();

    (prev_len - common, curr_len - common)
}

/// Insert horizontal subgraph padding into x_coords.
/// Returns the updated max_width.
fn subgraph_padding_csr(
    graph: &CsrGraph<'_>,
    vlevel_offsets: &[Idx],
    vnode_data: &[Idx],
    x_coords: &mut [Coord],
    widths: &[Coord],
    max_level: Idx,
) -> usize {
    let mut global_max_width = 0usize;

    for level in 0..=max_level as usize {
        if level + 1 >= vlevel_offsets.len() { break; }
        let start = vlevel_offsets[level] as usize;
        let end = vlevel_offsets[level + 1] as usize;
        if start >= end { continue; }

        let mut x = 0usize;

        // Left padding: depth of first node's subgraph chain
        let first_type = vnode_data.get(start * 2).copied().unwrap_or(0);
        let first_idx = vnode_data.get(start * 2 + 1).copied().unwrap_or(0);
        let first_depth = graph.sg_chain_depth(
            vnode_subgraph_csr(graph, first_type, first_idx),
        );
        x += first_depth * SUBGRAPH_H_PAD;

        for pos in start..end {
            if pos > start {
                let prev_type = vnode_data[( pos - 1) * 2];
                let prev_idx = vnode_data[(pos - 1) * 2 + 1];
                let curr_type = vnode_data[pos * 2];
                let curr_idx = vnode_data[pos * 2 + 1];
                let prev_sg = vnode_subgraph_csr(graph, prev_type, prev_idx);
                let curr_sg = vnode_subgraph_csr(graph, curr_type, curr_idx);
                if prev_sg != curr_sg {
                    // CSS-style margin collapsing: max(exit_margin, entry_margin)
                    let (exits, entries) = sg_boundary_exits_entries(graph, prev_sg, curr_sg);
                    let exit_margin = exits * SUBGRAPH_H_PAD;
                    let entry_margin = entries * SUBGRAPH_H_PAD;
                    x += core::cmp::max(exit_margin, entry_margin);
                }
            }
            if pos < x_coords.len() {
                x_coords[pos] = x as Coord;
            }
            let w = widths.get(pos).copied().unwrap_or(3) as usize;
            x += w + 3; // standard spacing
        }

        // Right padding: depth of last node's subgraph chain
        let last_pos = end - 1;
        let last_type = vnode_data[last_pos * 2];
        let last_idx = vnode_data[last_pos * 2 + 1];
        let last_depth = graph.sg_chain_depth(
            vnode_subgraph_csr(graph, last_type, last_idx),
        );
        let right_extra = last_depth * SUBGRAPH_H_PAD;

        // Compute level width
        let mut level_max = 0usize;
        for pos in start..end {
            let px = x_coords.get(pos).copied().unwrap_or(0) as usize;
            let pw = widths.get(pos).copied().unwrap_or(3) as usize;
            let r = px + pw;
            if r > level_max { level_max = r; }
        }
        level_max += right_extra;
        if level_max > global_max_width {
            global_max_width = level_max;
        }
    }

    global_max_width
}

// ── Sibling subgraph overlap repair (CSR) ────────────────────────────────

/// Minimum gap (chars) between bounding boxes of sibling subgraphs.
const SIBLING_GAP: usize = 1;

/// Check if `node_idx` belongs to `target_sg` or any of its descendants.
fn node_in_sg_subtree(graph: &CsrGraph<'_>, node_idx: usize, target_sg: usize) -> bool {
    if let Some(mut sg) = graph.node_subgraph(node_idx) {
        loop {
            if sg == target_sg { return true; }
            match graph.subgraph_parent(sg) {
                Some(p) => sg = p,
                None => return false,
            }
        }
    }
    false
}

/// CSR equivalent of `fix_subgraph_overlaps` in subgraph.rs.
///
/// Detects and fixes horizontal overlaps between sibling subgraph bounding
/// boxes after centering. Uses only pre-allocated scratch buffers (no heap).
///
/// * `sg_envelopes` — scratch for bbox data: `(left, right, shift, 0)`.
/// * `sg_depths` — scratch for nesting depths.
/// * `scratch` — scratch for per-level node sorting (`node_slots`, `>= node_count`).
fn fix_subgraph_overlaps_csr(
    graph: &CsrGraph<'_>,
    real_coords: &mut [(usize, usize, usize, usize)],
    sg_envelopes: &mut [(usize, usize, usize, usize)],
    sg_depths: &mut [usize],
    scratch: &mut [usize],
) -> usize {
    let sg_count = graph.subgraph_count();
    if sg_count < 2 { return 0; }
    let node_count = graph.node_count().min(real_coords.len());

    let cross_sg_gap: usize = 2 * SUBGRAPH_H_PAD + SIBLING_GAP;

    // Fill nesting depths
    for i in 0..sg_count {
        sg_depths[i] = graph.sg_chain_depth(Some(i));
    }
    let max_depth = sg_depths[..sg_count].iter().copied().max().unwrap_or(0);

    // Compute level range per subgraph (constant across rounds).
    let mut sg_min_level = [usize::MAX; 128];
    let mut sg_max_level = [0usize; 128];
    for ni in 0..node_count {
        if let Some(sg_idx) = graph.node_subgraph(ni) {
            if sg_idx < sg_count && sg_idx < 128 {
                let level = real_coords[ni].0;
                if level < sg_min_level[sg_idx] { sg_min_level[sg_idx] = level; }
                if level > sg_max_level[sg_idx] { sg_max_level[sg_idx] = level; }
            }
        }
    }
    // Propagate child level ranges to parents (bottom-up)
    for depth in (0..=max_depth).rev() {
        for sg_idx in 0..sg_count.min(128) {
            if sg_depths[sg_idx] != depth { continue; }
            if let Some(pidx) = graph.subgraph_parent(sg_idx) {
                if pidx < 128 {
                    let (cl, cr) = (sg_min_level[sg_idx], sg_max_level[sg_idx]);
                    if cl == usize::MAX { continue; }
                    if cl < sg_min_level[pidx] { sg_min_level[pidx] = cl; }
                    if cr > sg_max_level[pidx] { sg_max_level[pidx] = cr; }
                }
            }
        }
    }

    let mut total_extra = 0usize;

    for _round in 0..8 {
        // ── Compute padded bbox (left, right) per subgraph ──
        for e in sg_envelopes[..sg_count].iter_mut() {
            *e = (usize::MAX, 0, 0, 0);
        }
        for ni in 0..node_count {
            if let Some(sg_idx) = graph.node_subgraph(ni) {
                if sg_idx < sg_count {
                    let (_, _, x, width) = real_coords[ni];
                    let right = x + width;
                    let (ref mut mn, ref mut mx, _, _) = sg_envelopes[sg_idx];
                    if x < *mn { *mn = x; }
                    if right > *mx { *mx = right; }
                }
            }
        }
        // Propagate children → parents (bottom-up)
        for depth in (0..=max_depth).rev() {
            for sg_idx in 0..sg_count {
                if sg_depths[sg_idx] != depth { continue; }
                if let Some(pidx) = graph.subgraph_parent(sg_idx) {
                    let (cx, cr, _, _) = sg_envelopes[sg_idx];
                    if cx == usize::MAX { continue; }
                    let exp_l = cx.saturating_sub(SUBGRAPH_H_PAD);
                    let exp_r = cr + SUBGRAPH_H_PAD;
                    let (ref mut pl, ref mut pr, _, _) = sg_envelopes[pidx];
                    if exp_l < *pl { *pl = exp_l; }
                    if exp_r > *pr { *pr = exp_r; }
                }
            }
        }
        // Final padding + label-width expansion
        for sg_idx in 0..sg_count {
            let (mn, mx, _, _) = sg_envelopes[sg_idx];
            if mn == usize::MAX { continue; }
            let left = mn.saturating_sub(SUBGRAPH_H_PAD);
            let mut right = mx + SUBGRAPH_H_PAD;
            let label_w = graph.subgraph_label(sg_idx).len() + 4;
            if right.saturating_sub(left) < label_w { right = left + label_w; }
            sg_envelopes[sg_idx] = (left, right, 0, 0);
        }

        // ── Right-frontier sweep per parent group ──
        let mut any_shifted = false;

        // Iterate over each unique parent.  Sentinel 0 = top-level (None),
        // sentinel 1..=sg_count = parent index 0..sg_count-1.
        for parent_sentinel in 0..sg_count + 1 {
            let parent: Option<usize> = if parent_sentinel == 0 {
                None
            } else {
                Some(parent_sentinel - 1)
            };

            // Collect siblings (stack array, max 128)
            let mut siblings = [0usize; 128];
            let mut sib_count = 0usize;
            for sg_idx in 0..sg_count {
                if sg_envelopes[sg_idx].0 == usize::MAX { continue; }
                if graph.subgraph_parent(sg_idx) == parent {
                    if sib_count < 128 {
                        siblings[sib_count] = sg_idx;
                        sib_count += 1;
                    }
                }
            }
            if sib_count < 2 { continue; }

            // Insertion-sort siblings by bbox left
            for i in 1..sib_count {
                let key = siblings[i];
                let key_left = sg_envelopes[key].0;
                let mut j = i;
                while j > 0 && sg_envelopes[siblings[j - 1]].0 > key_left {
                    siblings[j] = siblings[j - 1];
                    j -= 1;
                }
                siblings[j] = key;
            }

            // Level-aware pairwise sweep: only separate siblings whose
            // rendered level ranges share at least one level.
            let mut processed = [(0usize, 0usize, 0usize, 0usize); 128]; // (sg_idx, eff_right, min_l, max_l)
            let mut proc_count = 0usize;

            for s in 0..sib_count {
                let sg_idx = siblings[s];
                let (left, right, _, _) = sg_envelopes[sg_idx];
                let cur_min_l = sg_min_level[sg_idx.min(127)];
                let cur_max_l = sg_max_level[sg_idx.min(127)];

                let mut eff_frontier = 0usize;
                let mut has_level_overlap = false;
                for p in 0..proc_count {
                    let (_, prev_right, prev_min_l, prev_max_l) = processed[p];
                    let overlaps = prev_min_l <= cur_max_l
                        && cur_min_l <= prev_max_l;
                    if overlaps && prev_right > eff_frontier {
                        eff_frontier = prev_right;
                        has_level_overlap = true;
                    }
                }

                if has_level_overlap && eff_frontier + SIBLING_GAP > left {
                    let shift = eff_frontier + SIBLING_GAP - left;
                    for ni in 0..node_count {
                        if node_in_sg_subtree(graph, ni, sg_idx) {
                            real_coords[ni].2 += shift;
                        }
                    }
                    total_extra += shift;
                    any_shifted = true;
                    if proc_count < 128 {
                        processed[proc_count] = (sg_idx, right + shift, cur_min_l, cur_max_l);
                        proc_count += 1;
                    }
                } else {
                    if proc_count < 128 {
                        processed[proc_count] = (sg_idx, right, cur_min_l, cur_max_l);
                        proc_count += 1;
                    }
                }
            }
        }

        if !any_shifted { break; }

        // ── Per-level collision repair ──
        let max_level = real_coords[..node_count]
            .iter()
            .map(|c| c.0)
            .max()
            .unwrap_or(0);
        for level in 0..=max_level {
            // Collect nodes on this level into scratch[]
            let mut count = 0usize;
            for ni in 0..node_count {
                if real_coords[ni].0 == level && count < scratch.len() {
                    scratch[count] = ni;
                    count += 1;
                }
            }
            // Insertion-sort by x
            for i in 1..count {
                let key = scratch[i];
                let key_x = real_coords[key].2;
                let mut j = i;
                while j > 0 && real_coords[scratch[j - 1]].2 > key_x {
                    scratch[j] = scratch[j - 1];
                    j -= 1;
                }
                scratch[j] = key;
            }
            // Fix collisions
            for j in 1..count {
                let prev = scratch[j - 1];
                let curr = scratch[j];
                let need_sg_gap = match (graph.node_subgraph(prev), graph.node_subgraph(curr)) {
                    (Some(a), Some(b)) => a != b,
                    _ => false,
                };
                let gap = if need_sg_gap { cross_sg_gap } else { 3 };
                let needed = real_coords[prev].2 + real_coords[prev].3 + gap;
                if real_coords[curr].2 < needed {
                    real_coords[curr].2 = needed;
                }
            }
        }
    }

    total_extra
}

/// Compute per-level Y extras for subgraph borders.
/// Populates `sg_ranges`, `sg_depths`, `sg_y_extras` in temps and returns
/// (initial_offset, trailing_extra).
fn compute_sg_y_extras(
    graph: &CsrGraph<'_>,
    node_levels: &[Idx],
    max_level: usize,
    sg_ranges: &mut [(usize, usize)],
    sg_depths: &mut [usize],
    sg_y_extras: &mut [usize],
) -> (usize, usize) {
    let sg_count = graph.subgraph_count();
    if sg_count == 0 {
        sg_y_extras.fill(0);
        return (0, 0);
    }

    // 1. For each subgraph, find (first_level, last_level) from member nodes
    for r in sg_ranges.iter_mut() {
        *r = (usize::MAX, 0);
    }
    for node_idx in 0..graph.node_count() {
        if let Some(sg_idx) = graph.node_subgraph(node_idx) {
            if sg_idx < sg_count {
                let lvl = node_levels[node_idx] as usize;
                let (ref mut first, ref mut last) = sg_ranges[sg_idx];
                if lvl < *first { *first = lvl; }
                if lvl > *last { *last = lvl; }
            }
        }
    }

    // 2. Propagate child ranges to parents (bottom-up)
    let mut changed = true;
    while changed {
        changed = false;
        for i in 0..sg_count {
            let (cf, cl) = sg_ranges[i];
            if cf == usize::MAX { continue; } // no nodes
            if let Some(pi) = graph.subgraph_parent(i) {
                if pi < sg_count {
                    let (ref mut pf, ref mut pl) = sg_ranges[pi];
                    if *pf == usize::MAX {
                        *pf = cf; *pl = cl; changed = true;
                    } else {
                        if cf < *pf { *pf = cf; changed = true; }
                        if cl > *pl { *pl = cl; changed = true; }
                    }
                }
            }
        }
    }

    // 3. Compute nesting depths
    for i in 0..sg_count {
        let mut depth = 0;
        let mut cur = graph.subgraph_parent(i);
        while let Some(pid) = cur {
            depth += 1;
            if pid >= sg_count { break; }
            cur = graph.subgraph_parent(pid);
        }
        sg_depths[i] = depth;
    }

    // Helper: count stacked closing borders at a boundary
    let stacked_closing = |sg_idx: usize, boundary_level: usize| -> usize {
        let mut count = 1;
        let mut cur = graph.subgraph_parent(sg_idx);
        while let Some(pid) = cur {
            if pid >= sg_count { break; }
            let (f, l) = sg_ranges[pid];
            if f != usize::MAX && l == boundary_level {
                count += 1;
                cur = graph.subgraph_parent(pid);
                continue;
            }
            break;
        }
        count
    };
    let stacked_opening = |sg_idx: usize, boundary_level: usize| -> usize {
        let mut count = 1;
        let mut cur = graph.subgraph_parent(sg_idx);
        while let Some(pid) = cur {
            if pid >= sg_count { break; }
            let (f, _l) = sg_ranges[pid];
            if f == boundary_level {
                count += 1;
                cur = graph.subgraph_parent(pid);
                continue;
            }
            break;
        }
        count
    };

    // 4. Initial offset: max stacked opening borders at level 0
    let mut max_open_at_0 = 0usize;
    for i in 0..sg_count {
        let (f, _) = sg_ranges[i];
        if f == 0 {
            let d = stacked_opening(i, 0);
            if d > max_open_at_0 { max_open_at_0 = d; }
        }
    }
    let initial_offset = max_open_at_0 * SUBGRAPH_V_PAD_TOP;

    // 5. Per-boundary extras
    sg_y_extras.fill(0);
    for boundary_after in 0..max_level {
        let next_level = boundary_after + 1;

        let mut max_close = 0usize;
        let mut max_open = 0usize;

        for i in 0..sg_count {
            let (f, l) = sg_ranges[i];
            if f == usize::MAX { continue; }
            if l == boundary_after {
                let d = stacked_closing(i, boundary_after);
                if d > max_close { max_close = d; }
            }
            if f == next_level {
                let d = stacked_opening(i, next_level);
                if d > max_open { max_open = d; }
            }
        }

        if boundary_after < sg_y_extras.len() {
            sg_y_extras[boundary_after] = max_close * SUBGRAPH_V_PAD_BOTTOM
                + max_open * SUBGRAPH_V_PAD_TOP;
        }
    }

    // 6. Trailing extra
    let mut max_close_at_end = 0usize;
    for i in 0..sg_count {
        let (f, l) = sg_ranges[i];
        if f == usize::MAX { continue; }
        if l == max_level {
            let d = stacked_closing(i, max_level);
            if d > max_close_at_end { max_close_at_end = d; }
        }
    }
    let trailing_extra = max_close_at_end * SUBGRAPH_V_PAD_BOTTOM;

    (initial_offset, trailing_extra)
}

/// Compute subgraph bounding boxes and add them to the builder.
/// Uses sg_envelopes as scratch space.
fn compute_sg_bounding_boxes(
    graph: &CsrGraph<'_>,
    real_coords: &[(usize, usize, usize, usize)], // (level, pos, x, width)
    level_y_offsets: &[usize],
    total_height: usize,
    sg_depths: &[usize],
    sg_envelopes: &mut [(usize, usize, usize, usize)],
    builder: &mut LayoutIRArenaBuilder<'_>,
) {
    let sg_count = graph.subgraph_count();
    if sg_count == 0 { return; }

    // Pass 1: compute node envelope per subgraph
    for e in sg_envelopes.iter_mut() {
        *e = (usize::MAX, usize::MAX, 0, 0); // (min_x, min_y, max_x, max_y)
    }

    for node_idx in 0..graph.node_count() {
        if let Some(sg_idx) = graph.node_subgraph(node_idx) {
            if sg_idx >= sg_count { continue; }
            if node_idx >= real_coords.len() { continue; }
            let (level, _, x, width) = real_coords[node_idx];
            let y = level_y_offsets.get(level).copied().unwrap_or(0);
            let node_max_y = y + 1;
            let node_max_x = x + width;

            let (ref mut min_x, ref mut min_y, ref mut max_x, ref mut max_y) = sg_envelopes[sg_idx];
            if x < *min_x { *min_x = x; }
            if y < *min_y { *min_y = y; }
            if node_max_x > *max_x { *max_x = node_max_x; }
            if node_max_y > *max_y { *max_y = node_max_y; }
        }
    }

    // Pass 1.5: Convert envelopes to padded bboxes
    for sg_idx in 0..sg_count {
        let (min_x, min_y, max_x, max_y) = sg_envelopes[sg_idx];
        if min_x == usize::MAX { continue; } // no nodes

        let x = min_x.saturating_sub(SUBGRAPH_H_PAD);
        let y = min_y.saturating_sub(SUBGRAPH_V_PAD_TOP);
        let right = max_x + SUBGRAPH_H_PAD;
        let bottom = (max_y + SUBGRAPH_V_PAD_BOTTOM).min(total_height);

        // Ensure width fits label
        let label = graph.subgraph_label(sg_idx);
        let min_label_width = label.len() + 4;
        let width = right.saturating_sub(x);
        let right = if width < min_label_width { x + min_label_width } else { right };

        sg_envelopes[sg_idx] = (x, y, right, bottom);
    }

    // Pass 2: propagate children to parents (bottom-up by depth)
    // Process deepest first. Since depth array is already computed, sort by depth desc.
    // Use simple bubble iteration (sg_count is small)
    let mut order = [0usize; 64];
    let effective_sg = sg_count.min(64);
    for i in 0..effective_sg { order[i] = i; }
    // Sort by depth descending (simple insertion sort for small N)
    for i in 1..effective_sg {
        let mut j = i;
        while j > 0 && sg_depths[order[j]] > sg_depths[order[j - 1]] {
            order.swap(j, j - 1);
            j -= 1;
        }
    }

    for oi in 0..effective_sg {
        let sg_idx = order[oi];
        if let Some(parent_idx) = graph.subgraph_parent(sg_idx) {
            if parent_idx >= sg_count { continue; }
            let (cx, cy, cr, cb) = sg_envelopes[sg_idx];
            if cx == usize::MAX { continue; }
            let expanded = (
                cx.saturating_sub(SUBGRAPH_H_PAD),
                cy.saturating_sub(SUBGRAPH_V_PAD_TOP),
                cr + SUBGRAPH_H_PAD,
                cb + SUBGRAPH_V_PAD_BOTTOM,
            );
            let (ref mut px, ref mut py, ref mut pr, ref mut pb) = sg_envelopes[parent_idx];
            if *px == usize::MAX {
                *px = expanded.0; *py = expanded.1; *pr = expanded.2; *pb = expanded.3;
            } else {
                if expanded.0 < *px { *px = expanded.0; }
                if expanded.1 < *py { *py = expanded.1; }
                if expanded.2 > *pr { *pr = expanded.2; }
                if expanded.3 > *pb { *pb = expanded.3; }
            }
        }
    }

    // Add subgraph bounding boxes to builder
    for sg_idx in 0..effective_sg {
        let (x, y, right, bottom) = sg_envelopes[sg_idx];
        if x == usize::MAX { continue; }
        let width = right.saturating_sub(x);
        let height = bottom.saturating_sub(y);
        let parent = graph.subgraph_parent(sg_idx);
        let label = graph.subgraph_label(sg_idx);
        builder.add_subgraph(sg_idx, parent, label, x, y, width, height);
    }
}

fn calculate_levels_csr(graph: &CsrGraph<'_>, levels: &mut [Idx], back_edges: &[bool]) -> Idx {
    for l in levels.iter_mut() {
        *l = 0;
    }

    let mut changed = true;
    let mut passes = 0;
    while changed && passes < levels.len() {
        changed = false;
        passes += 1;

        for (ei, (from, to)) in graph.edges_iter().enumerate() {
            // Skip self-loops
            if from == to { continue; }
            // For back edges, flip direction so cycles don't prevent convergence
            let is_back = back_edges.get(ei).copied().unwrap_or(false);
            let (src, dst) = if is_back { (to, from) } else { (from, to) };
            let new_level = levels[src] + 1;
            if new_level > levels[dst] {
                levels[dst] = new_level;
                changed = true;
            }
        }
    }
    levels
        .iter()
        .copied()
        .max()
        .unwrap_or(0)
        .min(MAX_LEVELS as Idx)
}

fn build_virtual_levels_csr(
    graph: &CsrGraph<'_>,
    node_levels: &[Idx],
    vlevel_offsets: &mut [Idx],
    level_counts: &mut [Idx],
    vnode_data: &mut [Idx],
    max_level: Idx,
    back_edges: &[bool],
) -> (Idx, Idx) {
    // Logic identical to DAG version but iterating graph.edges_iter()
    for c in level_counts.iter_mut() {
        *c = 0;
    }

    for &level in node_levels.iter() {
        let level_usize = level as usize;
        if level_usize < level_counts.len() {
            level_counts[level_usize] += 1;
        }
    }

    for (ei, (from, to)) in graph.edges_iter().enumerate() {
        // For back edges, layout direction is reversed
        let is_back = back_edges.get(ei).copied().unwrap_or(false);
        let (layout_from, layout_to) = if is_back { (to, from) } else { (from, to) };
        let from_level = node_levels[layout_from] as usize;
        let to_level = node_levels[layout_to] as usize;
        if to_level > from_level + 1 {
            for level in (from_level + 1)..to_level {
                if level < level_counts.len() {
                    level_counts[level] += 1;
                }
            }
        }
    }

    vlevel_offsets[0] = 0;
    let effective_max_level = (max_level as usize).min(level_counts.len().saturating_sub(1));
    for level in 0..=effective_max_level {
        vlevel_offsets[level + 1] = vlevel_offsets[level] + level_counts[level];
    }

    for c in level_counts.iter_mut() {
        *c = 0;
    }

    for (idx, &level) in node_levels.iter().enumerate() {
        let level_usize = level as usize;
        if level_usize <= effective_max_level {
            let pos = (vlevel_offsets[level_usize] + level_counts[level_usize]) as usize;
            // Bounds check for safety - skip if buffer exhausted
            if pos * 2 + 1 >= vnode_data.len() {
                continue;
            }
            vnode_data[pos * 2] = 0; // Real
            vnode_data[pos * 2 + 1] = idx as Idx;
            level_counts[level_usize] += 1;
        }
    }

    for (edge_idx, (from, to)) in graph.edges_iter().enumerate() {
        // For back edges, layout direction is reversed
        let is_back = back_edges.get(edge_idx).copied().unwrap_or(false);
        let (layout_from, layout_to) = if is_back { (to, from) } else { (from, to) };
        let from_level = node_levels[layout_from] as usize;
        let to_level = node_levels[layout_to] as usize;
        if to_level > from_level + 1 {
            for level in (from_level + 1)..to_level {
                if level <= effective_max_level {
                    let pos = (vlevel_offsets[level] + level_counts[level]) as usize;
                    // Bounds check for safety - skip if buffer exhausted
                    if pos * 2 + 1 >= vnode_data.len() {
                        continue;
                    }
                    vnode_data[pos * 2] = 1; // Dummy
                    vnode_data[pos * 2 + 1] = edge_idx as Idx;
                    level_counts[level] += 1;
                }
            }
        }
    }

    let total = vlevel_offsets[effective_max_level + 1];
    let max_size = level_counts.iter().copied().max().unwrap_or(0);
    (total, max_size)
}

fn assign_x_coords_csr(
    graph: &CsrGraph<'_>,
    vlevel_offsets: &[Idx],
    vnode_data: &[Idx],
    x_coords: &mut [Coord],
    widths: &mut [Coord],
    max_level: Idx,
) -> Coord {
    let mut max_width: Coord = 0;
    let max_pos = x_coords.len();
    let max_vnode_idx = vnode_data.len() / 2;

    for level in 0..=(max_level as usize) {
        let start = vlevel_offsets[level] as usize;
        let end = (vlevel_offsets[level + 1] as usize)
            .min(max_pos)
            .min(max_vnode_idx);
        let mut x: Coord = 0;

        for pos in start..end {
            // Bounds check
            if pos * 2 + 1 >= vnode_data.len() {
                break;
            }
            let vnode_type = vnode_data[pos * 2];
            let vnode_idx = vnode_data[pos * 2 + 1] as usize;

            let width: Coord = if vnode_type == 0 {
                // Real node: use stored width from CsrGraph
                graph.node_width(vnode_idx) as Coord
            } else {
                // Dummy node - use width 3 for visual separation (matches heap mode)
                3
            };

            if pos < x_coords.len() {
                x_coords[pos] = x;
                widths[pos] = width;
            }
            x += width + 3;
        }

        if end > start && end - 1 < x_coords.len() {
            let last_x = x_coords[end - 1];
            let last_width = widths[end - 1];
            max_width = max_width.max(last_x + last_width);
        }
    }
    max_width
}

fn build_real_coords_csr(
    vlevel_offsets: &[Idx],
    vnode_data: &[Idx],
    x_coords: &[Coord],
    widths: &[Coord],
    real_coords: &mut [(usize, usize, usize, usize)],
    max_level: Idx,
    max_width: Coord,
    center: bool,
) {
    let max_pos = x_coords.len();
    let max_vnode_idx = vnode_data.len() / 2;

    // Logic identical to DAG version (no graph access needed, just array processing)
    for level in 0..=(max_level as usize) {
        let start = vlevel_offsets[level] as usize;
        let end = (vlevel_offsets[level + 1] as usize)
            .min(max_pos)
            .min(max_vnode_idx);
        if end <= start {
            continue;
        }

        let level_width: usize = if end > start && end - 1 < x_coords.len() {
            x_coords[end - 1] as usize + widths[end - 1] as usize
        } else {
            0
        };
        let offset: usize = if center && (max_width as usize) > level_width {
            (max_width as usize - level_width) / 2
        } else {
            0
        };

        for pos in start..end {
            // Bounds check
            if pos * 2 + 1 >= vnode_data.len() || pos >= x_coords.len() {
                break;
            }
            let vnode_type = vnode_data[pos * 2];
            let vnode_idx = vnode_data[pos * 2 + 1] as usize;

            if vnode_type == 0 && vnode_idx < real_coords.len() {
                let x = x_coords[pos] as usize + offset;
                let width = widths[pos] as usize;
                let level_pos = pos - start;
                real_coords[vnode_idx] = (level, level_pos, x, width);
            }
        }
    }
}

/// Build dummy positions for skip-level edges from virtual level positions (CSR version).
/// This extracts the actual x-coordinates assigned during layout, ensuring edges
/// route around nodes based on the natural layout ordering.
fn build_dummy_positions_csr(
    graph: &CsrGraph<'_>,
    vlevel_offsets: &[Idx],
    vnode_data: &[Idx],
    x_coords: &[Coord],
    widths: &[Coord],
    dummy_offsets: &mut [Idx],
    dummy_data: &mut [(Idx, Coord)],
    max_level: Idx,
    max_width: Coord,
) {
    let edge_count = graph.edge_count();

    // Initialize offsets to 0
    dummy_offsets[0] = 0;
    for i in 1..=edge_count {
        if i < dummy_offsets.len() {
            dummy_offsets[i] = 0;
        }
    }

    // Collect dummy positions per edge using stack buffer
    let mut edge_dummy_counts = [0u16; 512]; // Support up to 512 edges

    // First pass: count dummy nodes per edge
    for level in 0..=(max_level as usize) {
        let start = vlevel_offsets[level] as usize;
        let end = vlevel_offsets[level + 1] as usize;

        for pos in start..end {
            let vnode_type = vnode_data[pos * 2];
            if vnode_type == 1 {
                let edge_idx = vnode_data[pos * 2 + 1] as usize;
                if edge_idx < 512 {
                    edge_dummy_counts[edge_idx] += 1;
                }
            }
        }
    }

    // Build prefix sums for offsets
    let mut running_offset: Idx = 0;
    for edge_idx in 0..edge_count {
        dummy_offsets[edge_idx] = running_offset;
        if edge_idx < 512 {
            running_offset += edge_dummy_counts[edge_idx] as Idx;
        }
    }
    dummy_offsets[edge_count] = running_offset;

    // Reset counts for use as write indices
    for count in edge_dummy_counts.iter_mut() {
        *count = 0;
    }

    // Second pass: write dummy data in level order (important for waypoints)
    for level in 0..=(max_level as usize) {
        let start = vlevel_offsets[level] as usize;
        let end = vlevel_offsets[level + 1] as usize;

        // Calculate centering offset for this level
        let level_width = if end > start {
            x_coords[end - 1] as usize + widths[end - 1] as usize
        } else {
            0
        };
        let offset = if (max_width as usize) > level_width {
            ((max_width as usize) - level_width) / 2
        } else {
            0
        };

        for pos in start..end {
            let vnode_type = vnode_data[pos * 2];
            if vnode_type == 1 {
                let edge_idx = vnode_data[pos * 2 + 1] as usize;

                let base_x = x_coords[pos] as usize + offset;
                let edge_offset = edge_idx % 4;
                let x = base_x + edge_offset;

                if edge_idx < 512 && edge_idx < edge_count {
                    let base_offset = dummy_offsets[edge_idx] as usize;
                    let write_idx = base_offset + edge_dummy_counts[edge_idx] as usize;
                    if write_idx < dummy_data.len() {
                        dummy_data[write_idx] = (level as Idx, x as Coord);
                        edge_dummy_counts[edge_idx] += 1;
                    }
                }
            }
        }
    }
}

// ---------- Crossing reduction for CSR path ----------

/// Crossing reduction operating on flat virtual-level arrays, specialized for CsrGraph.
///
/// Mirrors `Graph::reduce_crossings_arena` but uses CsrGraph adjacency (`children`/`parents`
/// returning `&[u32]`) instead of heap `Vec<Vec<usize>>`.
#[allow(clippy::too_many_arguments)]
fn reduce_crossings_csr(
    graph: &CsrGraph<'_>,
    crossing_pipeline: &[CrossingReducer],
    vlevel_offsets: &[Idx],
    vnode_data: &mut [Idx],
    max_level: usize,
    medians: &mut [(Idx, u32)],
    positions: &mut [Idx],
    edge_indices: &[(Idx, Idx)],
    level_vdummy_counts: &[Idx],
) {
    // One-time init: positions is alloc_raw_uninit, fill with sentinel
    for p in positions.iter_mut() { *p = Idx::MAX; }

    for reducer in crossing_pipeline {
        match reducer {
            CrossingReducer::Median(passes) => {
                for _ in 0..*passes {
                    // Top-down pass
                    for level in 1..=max_level {
                        median_reorder_csr_level(
                            graph, vlevel_offsets, vnode_data, edge_indices,
                            level, level - 1, true, medians, positions,
                            level_vdummy_counts,
                        );
                    }
                    // Bottom-up pass
                    for level in (0..max_level).rev() {
                        median_reorder_csr_level(
                            graph, vlevel_offsets, vnode_data, edge_indices,
                            level, level + 1, false, medians, positions,
                            level_vdummy_counts,
                        );
                    }
                }
            }
            CrossingReducer::AdjacentExchange(passes) => {
                for _ in 0..*passes {
                    for level in 1..=max_level {
                        adjacent_exchange_csr_level(
                            graph, vlevel_offsets, vnode_data, edge_indices,
                            level, level - 1, true, positions,
                            level_vdummy_counts,
                        );
                    }
                    for level in (0..max_level).rev() {
                        adjacent_exchange_csr_level(
                            graph, vlevel_offsets, vnode_data, edge_indices,
                            level, level + 1, false, positions,
                            level_vdummy_counts,
                        );
                    }
                }
            }
        }
    }
}

/// Median-heuristic reorder of one level (CSR version).
#[allow(clippy::too_many_arguments)]
fn median_reorder_csr_level(
    graph: &CsrGraph<'_>,
    vlevel_offsets: &[Idx],
    vnode_data: &mut [Idx],
    edge_indices: &[(Idx, Idx)],
    level: usize,
    adj_level: usize,
    use_parents: bool,
    medians: &mut [(Idx, u32)],
    positions: &mut [Idx],
    level_vdummy_counts: &[Idx],
) {
    let cur_start = vlevel_offsets[level] as usize;
    let cur_end = vlevel_offsets[level + 1] as usize;
    let count = cur_end - cur_start;
    if count < 2 { return; }

    let adj_start = vlevel_offsets[adj_level] as usize;
    let adj_end = vlevel_offsets[adj_level + 1] as usize;

    let adj_has_dummies = adj_level < level_vdummy_counts.len()
        && level_vdummy_counts[adj_level] > 0;

    // Build position map for real nodes in adjacent level (sparse-clear optimized)
    let adj_size = adj_end - adj_start;
    let mut written_buf: [usize; 512] = [0; 512];
    let mut written_count: usize = 0;
    let use_sparse_clear = adj_size <= 512;

    if !use_sparse_clear {
        for p in positions.iter_mut() { *p = Idx::MAX; }
    }
    for adj_pos in adj_start..adj_end {
        if adj_pos * 2 + 1 >= vnode_data.len() { break; }
        if vnode_data[adj_pos * 2] == 0 {
            let node_idx = vnode_data[adj_pos * 2 + 1] as usize;
            if node_idx < positions.len() {
                positions[node_idx] = (adj_pos - adj_start) as Idx;
                if use_sparse_clear && written_count < 512 {
                    written_buf[written_count] = node_idx;
                    written_count += 1;
                }
            }
        }
    }

    // Compute median for each node on this level
    for i in 0..count {
        let pos = cur_start + i;
        if pos * 2 + 1 >= vnode_data.len() {
            medians[i] = (i as Idx, (i as u32) << 10);
            continue;
        }
        let vtype = vnode_data[pos * 2];
        let vidx = vnode_data[pos * 2 + 1] as usize;

        let mut neigh: [usize; 16] = [0; 16];
        let mut neigh_count: usize = 0;

        if vtype == 0 {
            // Real node — CsrGraph adjacency
            let neighbours = if use_parents {
                graph.parents(vidx)
            } else {
                graph.children(vidx)
            };
            for &n_idx in neighbours {
                let n = n_idx as usize;
                if n < positions.len() && positions[n] != Idx::MAX && neigh_count < 16 {
                    neigh[neigh_count] = positions[n] as usize;
                    neigh_count += 1;
                }
            }
            if adj_has_dummies {
                for adj_pos in adj_start..adj_end {
                    if adj_pos * 2 + 1 >= vnode_data.len() { break; }
                    if vnode_data[adj_pos * 2] == 1 {
                        let eidx = vnode_data[adj_pos * 2 + 1] as usize;
                        if eidx < edge_indices.len() {
                            let (from_idx, to_idx) = edge_indices[eidx];
                            if (from_idx as usize == vidx || to_idx as usize == vidx)
                                && neigh_count < 16
                            {
                                neigh[neigh_count] = adj_pos - adj_start;
                                neigh_count += 1;
                            }
                        }
                    }
                }
            }
        } else if vidx < edge_indices.len() {
            // Dummy node
            let (from_idx, to_idx) = edge_indices[vidx];
            for &endpoint in &[from_idx as usize, to_idx as usize] {
                if endpoint < positions.len() && positions[endpoint] != Idx::MAX
                    && neigh_count < 16
                {
                    neigh[neigh_count] = positions[endpoint] as usize;
                    neigh_count += 1;
                }
            }
            if adj_has_dummies {
                for adj_pos in adj_start..adj_end {
                    if adj_pos * 2 + 1 >= vnode_data.len() { break; }
                    if vnode_data[adj_pos * 2] == 1
                        && vnode_data[adj_pos * 2 + 1] as usize == vidx
                        && neigh_count < 16
                    {
                        neigh[neigh_count] = adj_pos - adj_start;
                        neigh_count += 1;
                        break;
                    }
                }
            }
        }

        let median_fixed = if neigh_count == 0 {
            (i as u32) << 10
        } else {
            neigh[..neigh_count].sort_unstable();
            if neigh_count % 2 == 1 {
                (neigh[neigh_count / 2] as u32) << 10
            } else {
                let mid = neigh_count / 2;
                let sum = neigh[mid - 1] + neigh[mid];
                (sum as u32) * 512
            }
        };
        medians[i] = (i as Idx, median_fixed);
    }

    // Sort by median (unstable: no alloc needed, fine for layout positions)
    medians[..count].sort_unstable_by_key(|m| m.1);

    // Gather sorted vnode_data into medians buffer
    for j in 0..count {
        let orig_pos = medians[j].0 as usize;
        let src = cur_start + orig_pos;
        let vtype = vnode_data[src * 2];
        let vidx = vnode_data[src * 2 + 1] as u32;
        medians[j] = (vtype, vidx);
    }

    // Write sorted data back
    for j in 0..count {
        let dst = cur_start + j;
        vnode_data[dst * 2] = medians[j].0;
        vnode_data[dst * 2 + 1] = medians[j].1 as Idx;
    }

    // Sparse-clear
    if use_sparse_clear {
        for i in 0..written_count {
            positions[written_buf[i]] = Idx::MAX;
        }
    }
}

/// Adjacent exchange on one level (CSR version).
#[allow(clippy::too_many_arguments)]
fn adjacent_exchange_csr_level(
    graph: &CsrGraph<'_>,
    vlevel_offsets: &[Idx],
    vnode_data: &mut [Idx],
    edge_indices: &[(Idx, Idx)],
    level: usize,
    adj_level: usize,
    use_parents: bool,
    positions: &mut [Idx],
    level_vdummy_counts: &[Idx],
) {
    let cur_start = vlevel_offsets[level] as usize;
    let cur_end = vlevel_offsets[level + 1] as usize;
    let count = cur_end - cur_start;
    if count < 2 { return; }

    let adj_has_dummies = adj_level < level_vdummy_counts.len()
        && level_vdummy_counts[adj_level] > 0;

    let adj_start = vlevel_offsets[adj_level] as usize;
    let adj_end = vlevel_offsets[adj_level + 1] as usize;

    // Build position map (sparse-clear optimized)
    let adj_size = adj_end - adj_start;
    let mut written_buf: [usize; 512] = [0; 512];
    let mut written_count: usize = 0;
    let use_sparse_clear = adj_size <= 512;

    if !use_sparse_clear {
        for p in positions.iter_mut() { *p = Idx::MAX; }
    }
    for adj_pos in adj_start..adj_end {
        if adj_pos * 2 + 1 >= vnode_data.len() { break; }
        if vnode_data[adj_pos * 2] == 0 {
            let node_idx = vnode_data[adj_pos * 2 + 1] as usize;
            if node_idx < positions.len() {
                positions[node_idx] = (adj_pos - adj_start) as Idx;
                if use_sparse_clear && written_count < 512 {
                    written_buf[written_count] = node_idx;
                    written_count += 1;
                }
            }
        }
    }

    let mut u_neigh: [usize; 16] = [0; 16];
    let mut v_neigh: [usize; 16] = [0; 16];

    for i in 0..count - 1 {
        let u_pos = cur_start + i;
        let v_pos = cur_start + i + 1;
        if u_pos * 2 + 1 >= vnode_data.len() || v_pos * 2 + 1 >= vnode_data.len() {
            break;
        }

        let mut u_count = 0;
        let mut v_count = 0;

        gather_csr_neighbours(
            graph, vnode_data, edge_indices, positions,
            u_pos, adj_start, adj_end, use_parents, adj_has_dummies,
            &mut u_neigh, &mut u_count,
        );
        gather_csr_neighbours(
            graph, vnode_data, edge_indices, positions,
            v_pos, adj_start, adj_end, use_parents, adj_has_dummies,
            &mut v_neigh, &mut v_count,
        );

        let mut cross_uv: usize = 0;
        let mut cross_vu: usize = 0;
        for &a in &u_neigh[..u_count] {
            for &b in &v_neigh[..v_count] {
                if a > b { cross_uv += 1; }
                else if a < b { cross_vu += 1; }
            }
        }

        if cross_vu < cross_uv {
            let u_type = vnode_data[u_pos * 2];
            let u_idx = vnode_data[u_pos * 2 + 1];
            vnode_data[u_pos * 2] = vnode_data[v_pos * 2];
            vnode_data[u_pos * 2 + 1] = vnode_data[v_pos * 2 + 1];
            vnode_data[v_pos * 2] = u_type;
            vnode_data[v_pos * 2 + 1] = u_idx;
        }
    }

    if use_sparse_clear {
        for i in 0..written_count {
            positions[written_buf[i]] = Idx::MAX;
        }
    }
}

/// Gather neighbour positions for a single vnode (CSR version).
#[inline]
#[allow(clippy::too_many_arguments)]
fn gather_csr_neighbours(
    graph: &CsrGraph<'_>,
    vnode_data: &[Idx],
    edge_indices: &[(Idx, Idx)],
    positions: &[Idx],
    pos: usize,
    adj_start: usize,
    adj_end: usize,
    use_parents: bool,
    adj_has_dummies: bool,
    out: &mut [usize; 16],
    out_count: &mut usize,
) {
    *out_count = 0;
    let vtype = vnode_data[pos * 2];
    let vidx = vnode_data[pos * 2 + 1] as usize;

    if vtype == 0 {
        // Real node — CsrGraph adjacency (returns &[u32])
        let neighbours = if use_parents {
            graph.parents(vidx)
        } else {
            graph.children(vidx)
        };
        for &n_idx in neighbours {
            let n = n_idx as usize;
            if n < positions.len() && positions[n] != Idx::MAX && *out_count < 16 {
                out[*out_count] = positions[n] as usize;
                *out_count += 1;
            }
        }
        if adj_has_dummies {
            for adj_pos in adj_start..adj_end {
                if adj_pos * 2 + 1 >= vnode_data.len() { break; }
                if vnode_data[adj_pos * 2] == 1 {
                    let eidx = vnode_data[adj_pos * 2 + 1] as usize;
                    if eidx < edge_indices.len() {
                        let (from_idx, to_idx) = edge_indices[eidx];
                        if (from_idx as usize == vidx || to_idx as usize == vidx)
                            && *out_count < 16
                        {
                            out[*out_count] = (adj_pos - adj_start) as usize;
                            *out_count += 1;
                        }
                    }
                }
            }
        }
    } else if vidx < edge_indices.len() {
        // Dummy node
        let (from_idx, to_idx) = edge_indices[vidx];
        for &endpoint in &[from_idx as usize, to_idx as usize] {
            if endpoint < positions.len() && positions[endpoint] != Idx::MAX
                && *out_count < 16
            {
                out[*out_count] = positions[endpoint] as usize;
                *out_count += 1;
            }
        }
        if adj_has_dummies {
            for adj_pos in adj_start..adj_end {
                if adj_pos * 2 + 1 >= vnode_data.len() { break; }
                if vnode_data[adj_pos * 2] == 1
                    && vnode_data[adj_pos * 2 + 1] as usize == vidx
                    && *out_count < 16
                {
                    out[*out_count] = (adj_pos - adj_start) as usize;
                    *out_count += 1;
                    break;
                }
            }
        }
    }
}

// ── Graph::estimate_layout_arena_size ─────────────────────────────────────────
#[cfg(feature = "alloc")]
use crate::graph::Graph;
#[cfg(feature = "std")]
use alloc::vec;

#[cfg(feature = "alloc")]
impl<'a> Graph<'a> {
    /// Estimate the arena buffer size needed for `compute_layout_arena()`.
    ///
    /// Performs a cheap O(N+E) level computation to measure the actual dummy
    /// count, then sums all temporary and IR buffer requirements.
    pub fn estimate_layout_arena_size(&self) -> usize {
        let node_count = self.nodes.len();
        let edge_count = self.edges.len();
        let label_bytes: usize = self.nodes.iter().map(|(_, l)| l.len()).sum();
        let max_levels = node_count.min(MAX_LEVELS);

        // ── Cheap level computation to count actual dummies ────────────
        let actual_dummies: usize;
        {
            #[cfg(feature = "std")]
            {
                let mut levels = vec![0u32; node_count];
                let edge_idx: alloc::vec::Vec<(usize, usize)> = self.edges.iter().map(|&(from_id, to_id, _)| {
                    let fi = self.node_index(from_id).unwrap_or(usize::MAX);
                    let ti = self.node_index(to_id).unwrap_or(usize::MAX);
                    (fi, ti)
                }).collect();
                let mut changed = true;
                let mut passes = 0;
                while changed && passes < node_count {
                    changed = false;
                    passes += 1;
                    for &(fi, ti) in &edge_idx {
                        if fi != usize::MAX && ti != usize::MAX {
                            let nl = levels[fi] + 1;
                            if nl > levels[ti] {
                                levels[ti] = nl;
                                changed = true;
                            }
                        }
                    }
                }
                let mut dummies = 0usize;
                for &(fi, ti) in &edge_idx {
                    if fi != usize::MAX && ti != usize::MAX {
                        let fl = levels[fi] as usize;
                        let tl = levels[ti] as usize;
                        if tl > fl + 1 {
                            dummies += tl - fl - 1;
                        }
                    }
                }
                actual_dummies = dummies;
            }
            #[cfg(not(feature = "std"))]
            {
                actual_dummies = edge_count.saturating_mul(4);
            }
        }

        let max_vnodes = (node_count + actual_dummies).min(MAX_NODES);
        let max_level_size = node_count.min(MAX_NODES);
        let max_dummy_waypoints = (actual_dummies + 16).min(MAX_NODES);

        let temps_size = node_count * core::mem::size_of::<Idx>()                      // node_levels
            + edge_count * core::mem::size_of::<(Idx, Idx)>()                          // edge_indices
            + (max_levels + 2) * core::mem::size_of::<Idx>()              // vlevel_offsets
            + (max_levels + 1) * core::mem::size_of::<Idx>()              // level_counts
            + max_vnodes * 2 * core::mem::size_of::<Idx>()                // vnode_data
            + max_vnodes * core::mem::size_of::<Coord>()                  // x_coords
            + max_vnodes * core::mem::size_of::<Coord>()                  // widths
            + node_count * core::mem::size_of::<(usize, usize, usize, usize)>() // real_coords
            + (edge_count + 1) * core::mem::size_of::<Idx>()              // dummy_offsets
            + max_dummy_waypoints * core::mem::size_of::<(Idx, Coord)>()  // dummy_data
            + max_level_size * core::mem::size_of::<(Idx, u32)>()         // medians
            + max_level_size * core::mem::size_of::<Idx>()                // positions
            + node_count * core::mem::size_of::<bool>()                   // node_is_source
            + (max_levels + 1) * core::mem::size_of::<Idx>()              // source_counts
            + (max_levels + 1) * core::mem::size_of::<Idx>()              // dummy_counts
            + (max_levels + 2) * core::mem::size_of::<usize>()            // level_y_offsets
            + node_count * core::mem::size_of::<usize>()                  // node_slots
            + (max_levels + 1) * core::mem::size_of::<Idx>()              // level_slot_next
            + (max_levels + 1) * MAX_SLOTS_PER_LEVEL * core::mem::size_of::<(usize, usize)>() // slot_bounds
            + (max_levels + 1) * core::mem::size_of::<Idx>()              // level_dummy_next
            + (max_levels + 1) * core::mem::size_of::<(usize, usize)>()   // waypoint_scratch
            + (max_levels + 1) * core::mem::size_of::<Idx>()              // level_vdummy_counts
            + 4096; // alignment padding buffer

        let max_ir_waypoints = max_dummy_waypoints;
        let ir_size = crate::ir::arena::estimate_layout_arena_size(
            node_count,
            edge_count,
            label_bytes,
            max_ir_waypoints,
        );

        temps_size + ir_size
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::arena::Arena;
    use crate::graph::csr::CsrGraphBuilder;

    /// Helper: build a CsrGraph from edges (node labels auto-generated A, B, C, ...)
    fn build_csr_graph<'a>(
        arena: &'a mut Arena<'a>,
        node_count: usize,
        edges: &[(usize, usize)],
    ) -> CsrGraph<'a> {
        let label_bytes = node_count * 2; // single-char labels
        let mut builder = CsrGraphBuilder::new(arena, node_count, edges.len(), label_bytes)
            .expect("builder alloc");
        for i in 0..node_count {
            let label = &[b'A' + i as u8];
            let label_str = core::str::from_utf8(label).unwrap();
            builder.add_node(i, label_str);
        }
        for &(from, to) in edges {
            builder.add_edge(from, to);
        }
        builder.build().expect("csr build")
    }

    #[test]
    fn test_detect_back_edges_acyclic() {
        let mut buf = [0u8; 8192];
        let mut arena = Arena::new(&mut buf);
        let graph = build_csr_graph(&mut arena, 3, &[(0, 1), (1, 2)]);

        let mut back_edges = [false; 2];
        let mut dfs_buf = [0u8; 4096];
        let mut dfs_arena = Arena::new(&mut dfs_buf);
        detect_back_edges_csr(&graph, &mut back_edges, &mut dfs_arena);

        assert!(!back_edges[0], "0→1 should not be a back edge");
        assert!(!back_edges[1], "1→2 should not be a back edge");
    }

    #[test]
    fn test_detect_back_edges_simple_cycle() {
        let mut buf = [0u8; 8192];
        let mut arena = Arena::new(&mut buf);
        // A→B→C→A (edge 2 is the back edge)
        let graph = build_csr_graph(&mut arena, 3, &[(0, 1), (1, 2), (2, 0)]);

        let mut back_edges = [false; 3];
        let mut dfs_buf = [0u8; 4096];
        let mut dfs_arena = Arena::new(&mut dfs_buf);
        detect_back_edges_csr(&graph, &mut back_edges, &mut dfs_arena);

        // Exactly one edge should be marked as back edge (the cycle-closing one)
        let back_count: usize = back_edges.iter().filter(|&&b| b).count();
        assert_eq!(back_count, 1, "exactly 1 back edge in A→B→C→A");
        // The DFS from 0: visits 0→1→2, then 2→0 targets GRAY node → back edge
        assert!(back_edges[2], "edge 2→0 should be the back edge");
    }

    #[test]
    fn test_detect_back_edges_self_loop() {
        let mut buf = [0u8; 8192];
        let mut arena = Arena::new(&mut buf);
        let graph = build_csr_graph(&mut arena, 2, &[(0, 0), (0, 1)]);

        let mut back_edges = [false; 2];
        let mut dfs_buf = [0u8; 4096];
        let mut dfs_arena = Arena::new(&mut dfs_buf);
        detect_back_edges_csr(&graph, &mut back_edges, &mut dfs_arena);

        assert!(back_edges[0], "self-loop should be a back edge");
        assert!(!back_edges[1], "0→1 should not be a back edge");
    }

    #[test]
    fn test_cyclic_graph_levels_converge() {
        let mut buf = [0u8; 8192];
        let mut arena = Arena::new(&mut buf);
        // A→B→C→A
        let graph = build_csr_graph(&mut arena, 3, &[(0, 1), (1, 2), (2, 0)]);

        let mut back_edges = [false; 3];
        let mut dfs_buf = [0u8; 4096];
        let mut dfs_arena = Arena::new(&mut dfs_buf);
        detect_back_edges_csr(&graph, &mut back_edges, &mut dfs_arena);

        let mut levels = [0 as Idx; 3];
        let max_level = calculate_levels_csr(&graph, &mut levels, &back_edges);

        // With back edge 2→0 reversed, effective DAG is A→B→C
        // Levels: A=0, B=1, C=2
        assert_eq!(max_level, 2);
        assert_eq!(levels[0], 0, "A should be level 0");
        assert_eq!(levels[1], 1, "B should be level 1");
        assert_eq!(levels[2], 2, "C should be level 2");
    }

    #[test]
    fn test_cyclic_csr_layout_no_panic() {
        // A→B→C→A: full layout pipeline should complete without panic
        let mut graph_buf = [0u8; 8192];
        let mut graph_arena = Arena::new(&mut graph_buf);
        let graph = build_csr_graph(&mut graph_arena, 3, &[(0, 1), (1, 2), (2, 0)]);

        let config = LayoutConfig::standard();
        let mut temp_buf = [0u8; 65536];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_buf = [0u8; 65536];
        let mut out_arena = Arena::new(&mut out_buf);

        let ir = compute_layout_arena_csr(&graph, &config, &mut temp_arena, &mut out_arena);
        assert!(ir.is_ok(), "layout of cyclic graph should succeed");

        let ir = ir.unwrap();
        assert_eq!(ir.node_count(), 3);
        assert!(ir.edge_count() >= 2, "should have at least 2 edges (self-loops skipped)");

        // Check that the reversed edge is marked
        let mut found_reversed = false;
        for i in 0..ir.edge_count() {
            if ir.edge(i).reversed {
                found_reversed = true;
            }
        }
        assert!(found_reversed, "cyclic graph should have at least one reversed edge");
    }

    #[test]
    fn test_cyclic_csr_renders_without_panic() {
        // A→B→C→A: full pipeline through to rendering
        let mut graph_buf = [0u8; 8192];
        let mut graph_arena = Arena::new(&mut graph_buf);
        let graph = build_csr_graph(&mut graph_arena, 3, &[(0, 1), (1, 2), (2, 0)]);

        let config = LayoutConfig::standard();
        let mut temp_buf = [0u8; 65536];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_buf = [0u8; 65536];
        let mut out_arena = Arena::new(&mut out_buf);

        let ir = compute_layout_arena_csr(&graph, &config, &mut temp_arena, &mut out_arena)
            .expect("layout should succeed");

        let mut render_buf = [0u8; 4096];
        let mut line_buf = [' '; 256];
        let mut scratch_buf = [0usize; 256];
        let rendered = ir.render_to_buffer(&mut render_buf, &mut line_buf, &mut scratch_buf);
        assert!(rendered.is_some(), "rendering should succeed");
        let len = rendered.unwrap();
        assert!(len > 0, "should produce non-empty output");
    }

    #[test]
    fn test_diamond_with_back_edge() {
        // Diamond: A→B, A→C, B→D, C→D, plus back edge D→A
        let mut graph_buf = [0u8; 16384];
        let mut graph_arena = Arena::new(&mut graph_buf);
        let graph = build_csr_graph(&mut graph_arena, 4, &[
            (0, 1), (0, 2), (1, 3), (2, 3), (3, 0),
        ]);

        let config = LayoutConfig::standard();
        let mut temp_buf = [0u8; 65536];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_buf = [0u8; 65536];
        let mut out_arena = Arena::new(&mut out_buf);

        let ir = compute_layout_arena_csr(&graph, &config, &mut temp_arena, &mut out_arena)
            .expect("diamond+backedge layout should succeed");

        assert_eq!(ir.node_count(), 4);

        // Verify levels make sense: A at top, D at bottom (back edge D→A reversed)
        let node_a = ir.node(0);
        let node_d = ir.node(3);
        assert!(node_a.y < node_d.y, "A should be above D");
    }

    /// Regression test: cyclic graph via Graph::to_csr() path
    /// (existing tests use CsrGraphBuilder directly)
    #[test]
    fn test_cyclic_via_to_csr_layout_and_render() {
        use crate::graph::Graph;

        let mut dag = Graph::new();
        dag.add_node(1, "A");
        dag.add_node(2, "B");
        dag.add_node(3, "C");
        dag.add_edge(1, 2, None);
        dag.add_edge(2, 3, None);
        dag.add_edge(3, 1, None); // cycle

        let csr_size = dag.estimate_csr_arena_size() * 2;
        let mut csr_buf = vec![0u8; csr_size];
        let mut csr_arena = Arena::new(&mut csr_buf);
        let csr = dag.to_csr(&mut csr_arena).expect("to_csr");

        assert_eq!(csr.node_count(), 3);
        assert_eq!(csr.edge_count(), 3);

        let config = LayoutConfig::standard();
        let mut temp_buf = vec![0u8; 128 * 1024];
        let mut out_buf = vec![0u8; 128 * 1024];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_arena = Arena::new(&mut out_buf);

        let ir = compute_layout_arena_csr(&csr, &config, &mut temp_arena, &mut out_arena)
            .expect("layout should succeed");

        assert_eq!(ir.node_count(), 3);

        let mut render_buf = vec![0u8; 4096];
        let mut line_buf = vec![' '; 256];
        let mut scratch_buf = vec![0usize; 256];
        let rendered = ir.render_to_buffer(&mut render_buf, &mut line_buf, &mut scratch_buf);
        assert!(rendered.is_some(), "render should succeed");
    }

    #[test]
    fn test_two_node_cycle_layout() {
        use crate::graph::Graph;

        let mut dag = Graph::new();
        dag.add_node(1, "Ping");
        dag.add_node(2, "Pong");
        dag.add_edge(1, 2, None);
        dag.add_edge(2, 1, None);

        let csr_size = dag.estimate_csr_arena_size() * 2;
        let mut csr_buf = vec![0u8; csr_size];
        let mut csr_arena = Arena::new(&mut csr_buf);
        let csr = dag.to_csr(&mut csr_arena).expect("to_csr");

        let config = LayoutConfig::standard();
        let mut temp_buf = vec![0u8; 128 * 1024];
        let mut out_buf = vec![0u8; 128 * 1024];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_arena = Arena::new(&mut out_buf);

        let ir = compute_layout_arena_csr(&csr, &config, &mut temp_arena, &mut out_arena)
            .expect("layout should succeed");

        assert_eq!(ir.node_count(), 2);
        assert_eq!(ir.edge_count(), 2);

        // The two edges should be offset from each other (not overlapping)
        let e0 = ir.edge(0);
        let e1 = ir.edge(1);
        assert_ne!(e0.from_x, e1.from_x, "2-node cycle edges must not share the same column");

        // The forward edge should be left of the back-edge
        let (fwd, bck) = if e0.reversed { (e1, e0) } else { (e0, e1) };
        assert!(fwd.from_x < bck.from_x, "forward edge should be left of back-edge");

        // Rendering should succeed
        let mut render_buf = vec![0u8; 4096];
        let mut line_buf = vec![' '; 256];
        let mut scratch_buf = vec![0usize; 256];
        let rendered = ir.render_to_buffer(&mut render_buf, &mut line_buf, &mut scratch_buf);
        assert!(rendered.is_some(), "render should succeed");
    }

    #[test]
    fn test_self_loop_renders_indicator() {
        use crate::graph::Graph;

        let mut dag = Graph::new();
        dag.add_node(1, "Loop");
        dag.add_edge(1, 1, None);

        let csr_size = dag.estimate_csr_arena_size() * 2;
        let mut csr_buf = vec![0u8; csr_size];
        let mut csr_arena = Arena::new(&mut csr_buf);
        let csr = dag.to_csr(&mut csr_arena).expect("to_csr");

        let config = LayoutConfig::standard();
        let mut temp_buf = vec![0u8; 128 * 1024];
        let mut out_buf = vec![0u8; 128 * 1024];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_arena = Arena::new(&mut out_buf);

        let ir = compute_layout_arena_csr(&csr, &config, &mut temp_arena, &mut out_arena)
            .expect("layout should succeed");

        assert_eq!(ir.node_count(), 1);
        assert!(ir.node(0).has_self_loop, "self-loop node should be marked");

        // Rendered output should contain ↺
        let mut render_buf = vec![0u8; 4096];
        let mut line_buf = vec![' '; 256];
        let mut scratch_buf = vec![0usize; 256];
        let len = ir.render_to_buffer(&mut render_buf, &mut line_buf, &mut scratch_buf).unwrap();
        let output = core::str::from_utf8(&render_buf[..len]).unwrap();
        assert!(output.contains('↺'), "rendered output should contain self-loop indicator ↺");
        assert!(output.contains("[Loop]↺"), "↺ should appear right after the node bracket");
    }

    #[test]
    fn test_geometry_aware_slot_sharing() {
        // Fan-out: Root splits to Left and Right, then they merge.
        // The Root→Left corner and Root→Right corner come from the same
        // source (Root) and share a slot via source-bus rule.
        // Additionally: Left→Merge and Right→Merge come from different
        // sources at level 1. Their horizontal spans point inward (toward
        // Merge). With geometry-aware allocation, if the spans don't overlap,
        // they share a slot — resulting in a more compact layout.
        use crate::graph::Graph;

        let mut dag = Graph::new();
        dag.add_node(1, "Root");
        dag.add_node(2, "Left");
        dag.add_node(3, "Right");
        dag.add_node(4, "Merge");
        dag.add_edge(1, 2, None);
        dag.add_edge(1, 3, None);
        dag.add_edge(2, 4, None);
        dag.add_edge(3, 4, None);

        let csr_size = dag.estimate_csr_arena_size() * 2;
        let mut csr_buf = vec![0u8; csr_size];
        let mut csr_arena = Arena::new(&mut csr_buf);
        let csr = dag.to_csr(&mut csr_arena).expect("to_csr");

        let config = LayoutConfig::standard();
        let mut temp_buf = vec![0u8; 128 * 1024];
        let mut out_buf = vec![0u8; 128 * 1024];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_arena = Arena::new(&mut out_buf);

        let ir = compute_layout_arena_csr(&csr, &config, &mut temp_arena, &mut out_arena)
            .expect("layout should succeed");

        // Verify rendering works and produces expected characters
        let mut render_buf = vec![0u8; 8192];
        let mut line_buf = vec![' '; 256];
        let mut scratch_buf = vec![0usize; 256];
        let len = ir.render_to_buffer(&mut render_buf, &mut line_buf, &mut scratch_buf).unwrap();
        let output = core::str::from_utf8(&render_buf[..len]).unwrap();

        // Should contain all node labels
        assert!(output.contains("[Root]"));
        assert!(output.contains("[Left]"));
        assert!(output.contains("[Right]"));
        assert!(output.contains("[Merge]"));
        // Should contain arrow indicators and edge corners
        assert!(output.contains('↓'), "should contain down arrows");
        assert!(output.contains('┌') || output.contains('└'),
            "should contain corner characters for non-vertical edges");

        // Count the total height: geometry-aware should produce compact output
        let line_count = output.lines().count();
        // Diamond with 4 nodes should be at most ~10 lines with compressed slots
        assert!(line_count <= 12, "layout should be compact: got {} lines", line_count);
    }

    #[test]
    fn test_csr_single_subgraph_produces_border() {
        // Build: A→B, both in subgraph "cluster"
        let mut buf = [0u8; 32768];
        let mut arena = Arena::new(&mut buf);
        let sg_label_bytes = 7; // "cluster"
        let label_bytes = 4 + sg_label_bytes; // A+B node labels + sg label
        let mut builder = CsrGraphBuilder::new_with_subgraphs(
            &mut arena, 2, 1, label_bytes, 1,
        ).expect("builder");
        builder.add_node(0, "A");
        builder.add_node(1, "B");
        builder.add_edge(0, 1);
        let sg = builder.add_subgraph(0, "cluster").expect("sg");
        builder.set_node_subgraph(0, sg);
        builder.set_node_subgraph(1, sg);
        let graph = builder.build().expect("build");

        assert_eq!(graph.subgraph_count(), 1);
        assert_eq!(graph.subgraph_label(0), "cluster");

        // Layout + render
        let config = LayoutConfig::default();
        let mut temp_buf = [0u8; 65536];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_buf = [0u8; 65536];
        let mut out_arena = Arena::new(&mut out_buf);

        let ir = compute_layout_arena_csr(&graph, &config, &mut temp_arena, &mut out_arena)
            .expect("layout");

        assert_eq!(ir.subgraph_count(), 1);
        let sg_info = &ir.subgraphs()[0];
        assert!(sg_info.width > 0, "subgraph should have width");
        assert!(sg_info.height > 0, "subgraph should have height");

        // Render to text
        let (out_size, scratch_size) = ir.estimate_render_size();
        let mut render_buf = vec![0u8; out_size];
        let mut line_buf = vec![' '; ir.width()];
        let mut scratch = vec![0usize; scratch_size];
        let bytes = ir.render_to_buffer(&mut render_buf, &mut line_buf, &mut scratch)
            .expect("render");
        let output = core::str::from_utf8(&render_buf[..bytes]).expect("utf8");

        // Should contain border characters
        assert!(output.contains('╔'), "top-left border missing");
        assert!(output.contains('╗'), "top-right border missing");
        assert!(output.contains('╚'), "bottom-left border missing");
        assert!(output.contains('╝'), "bottom-right border missing");
        assert!(output.contains('║'), "side border missing");
        assert!(output.contains('═'), "horizontal border missing");
        // Label should appear
        assert!(output.contains("cluster"), "subgraph label missing");
        // Nodes should still be present
        assert!(output.contains("[A]"), "node A missing");
        assert!(output.contains("[B]"), "node B missing");
    }

    #[test]
    fn test_csr_subgraph_via_to_csr() {
        // Use the Graph→to_csr path which copies subgraph data
        use crate::graph::Graph;

        let mut g = Graph::new();
        g.add_node(1, "X");
        g.add_node(2, "Y");
        g.add_edge(1, 2, None);
        let sg = g.add_subgraph("box");
        g.put_nodes(&[1, 2]).inside(sg).unwrap();

        let mut csr_buf = [0u8; 32768];
        let mut csr_arena = Arena::new(&mut csr_buf);
        let csr = g.to_csr(&mut csr_arena).expect("to_csr");

        assert_eq!(csr.subgraph_count(), 1);
        assert_eq!(csr.subgraph_label(0), "box");

        let config = LayoutConfig::default();
        let mut temp_buf = [0u8; 65536];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_buf = [0u8; 65536];
        let mut out_arena = Arena::new(&mut out_buf);

        let ir = compute_layout_arena_csr(&csr, &config, &mut temp_arena, &mut out_arena)
            .expect("layout");

        assert!(ir.subgraph_count() >= 1, "IR should have subgraph");
        assert!(ir.subgraphs()[0].width > 0);
        assert!(ir.subgraphs()[0].height > 0);
    }

    #[test]
    fn test_csr_no_subgraphs_unchanged() {
        // Verify that the subgraph code path doesn't affect graphs without subgraphs
        let mut buf = [0u8; 16384];
        let mut arena = Arena::new(&mut buf);
        let graph = build_csr_graph(&mut arena, 3, &[(0, 1), (1, 2)]);

        assert_eq!(graph.subgraph_count(), 0);
        assert!(!graph.has_subgraphs());

        let config = LayoutConfig::default();
        let mut temp_buf = [0u8; 65536];
        let mut temp_arena = Arena::new(&mut temp_buf);
        let mut out_buf = [0u8; 65536];
        let mut out_arena = Arena::new(&mut out_buf);

        let ir = compute_layout_arena_csr(&graph, &config, &mut temp_arena, &mut out_arena)
            .expect("layout");

        assert_eq!(ir.subgraph_count(), 0);
        assert!(!ir.has_subgraphs());

        // Render should work fine
        let (out_size, scratch_size) = ir.estimate_render_size();
        let mut render_buf = vec![0u8; out_size];
        let mut line_buf = vec![' '; ir.width()];
        let mut scratch = vec![0usize; scratch_size];
        let bytes = ir.render_to_buffer(&mut render_buf, &mut line_buf, &mut scratch)
            .expect("render");
        let output = core::str::from_utf8(&render_buf[..bytes]).expect("utf8");
        assert!(output.contains("[A]"));
        assert!(output.contains("[B]"));
        assert!(output.contains("[C]"));
    }
}
