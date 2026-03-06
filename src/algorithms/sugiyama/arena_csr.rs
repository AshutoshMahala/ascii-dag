//! CSR-based arena layout computation.
//!
//! Pure-CSR layout pipeline: avoids all heap allocations and HashMap lookups
//! by operating directly on CSR graph indices.

use crate::graph::arena::Arena;
use crate::graph::csr::CsrGraph;
use crate::ir::arena::{EdgePathArena, LayoutEdgeArena, LayoutIRArena, LayoutIRArenaBuilder};
use super::config::LayoutConfig;
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

/// Temporary buffers for arena-based layout computation.
///
/// All slices are allocated from a single arena. This struct is used by both
/// the CsrGraph layout path and the Graph→CsrGraph path.
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
    pub(crate) level_dummy_next: &'a mut [Idx],
    pub(crate) waypoint_scratch: &'a mut [(usize, usize)],
    pub(crate) level_vdummy_counts: &'a mut [Idx],
}

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

    // Step 1: Allocate temporaries
    let mut temps = alloc_layout_temps_csr(temp_arena, node_count, edge_count)
        .ok_or(GraphError::ArenaOom)?;

    // Step 2: Calculate levels
    let max_level = calculate_levels_csr(graph, temps.node_levels);

    // Step 3: Build virtual levels
    let (_vnode_count, _max_level_size) = build_virtual_levels_csr(
        graph,
        temps.node_levels,
        temps.vlevel_offsets,
        temps.level_counts,
        temps.vnode_data,
        max_level,
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
    let max_width = assign_x_coords_csr(
        graph,
        temps.vlevel_offsets,
        temps.vnode_data,
        temps.x_coords,
        temps.widths,
        max_level,
    );

    // Step 6: Build real node coordinates
    build_real_coords_csr(
        temps.vlevel_offsets,
        temps.vnode_data,
        temps.x_coords,
        temps.widths,
        temps.real_coords,
        max_level,
        max_width,
    );

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

    // Step 8: Compute horizontal slots for edge separation
    // Count non-vertical source nodes per level to determine extra routing rows.
    // Vertical edges (same x-center, adjacent level) route straight down and need no slot.
    // Matches heap path behavior including MAX_SLOTS_PER_LEVEL cap.
    const MAX_SLOTS_PER_LEVEL: usize = 8;

    // 1. Mark nodes that need routing slots (have at least one non-vertical outgoing edge)
    temps.node_is_source.fill(false);
    let node_is_source = &mut temps.node_is_source;
    let alloc_size = max_level as usize + 1;

    for (from_idx, to_idx) in graph.edges_iter() {
        if from_idx < temps.real_coords.len() && to_idx < temps.real_coords.len() {
            let from_level = temps.real_coords[from_idx].0;
            let to_level = temps.real_coords[to_idx].0;

            if to_level > from_level {
                // Check if this is a vertical edge (same x-center, adjacent level)
                let from_x_center = temps.real_coords[from_idx].2 + temps.real_coords[from_idx].3 / 2;
                let to_x_center = temps.real_coords[to_idx].2 + temps.real_coords[to_idx].3 / 2;
                let is_vertical = from_x_center == to_x_center && to_level == from_level + 1;

                if !is_vertical {
                    node_is_source[from_idx] = true;
                }
            }
        }
    }

    // 2. Count sources per level (capped)
    temps.source_counts.fill(0);
    for (idx, &is_source) in node_is_source.iter().enumerate() {
        if is_source {
            let level = temps.real_coords[idx].0;
            if level <= max_level as usize {
                temps.source_counts[level] += 1;
            }
        }
    }
    // Cap slots per level to prevent height explosion on extreme fan-out
    for sc in temps.source_counts.iter_mut() {
        if (*sc as usize) > MAX_SLOTS_PER_LEVEL {
            *sc = MAX_SLOTS_PER_LEVEL as Idx;
        }
    }

    // 3. Count dummy nodes
    temps.dummy_counts.fill(0);
    // Limit loop to actual dummy data used
    let total_dummy_waypoints = temps.dummy_offsets[edge_count] as usize;
    for &(level, _) in &temps.dummy_data[..total_dummy_waypoints] {
        let lvl = level as usize;
        if lvl <= max_level as usize {
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
    let mut current_offset = 0;

    for level in 0..=max_level as usize {
        temps.level_y_offsets[level] = current_offset;
        let node_height = max_node_heights[level] as usize;
        let diff = temps.source_counts[level].max(temps.dummy_counts[level]) as usize;
        let height = node_height + routing_overhead + diff.saturating_sub(1);
        current_offset += height as usize;
    }
    temps.level_y_offsets[max_level as usize + 1] = current_offset;
    let total_height = current_offset;

    // Step 9: Build LayoutIRArena
    let mut builder = LayoutIRArenaBuilder::new(
        output_arena,
        node_count,
        edge_count,
        max_waypoints,
        total_label_bytes,
        max_level as usize + 1,
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

    // Track source slots for horizontal separation during edge iteration
    // Use optimized node_slots (O(1) lookup) instead of inefficient linear scan
    temps.node_slots.fill(0);
    temps.level_slot_next.fill(0);
    temps.level_dummy_next.fill(0);

    // Access mutable buffers via temps
    let node_slots = &mut temps.node_slots;
    let level_slot_next = &mut temps.level_slot_next;
    let level_dummy_next = &mut temps.level_dummy_next;
    let waypoint_scratch = &mut temps.waypoint_scratch;
    let level_y_offsets = &temps.level_y_offsets;
    let max_node_heights = &temps.level_vdummy_counts;

    // Add edges
    for (edge_idx, (from_idx, to_idx)) in graph.edges_iter().enumerate() {
        let (from_level, _, from_x_base, from_width) = temps.real_coords[from_idx];
        let (to_level, _, to_x_base, to_width) = temps.real_coords[to_idx];

        let from_x = (from_x_base + from_width / 2) as usize;
        let to_x = (to_x_base + to_width / 2) as usize;
        // from_y = bottom of source node (top + max_node_height - 1)
        let from_y = level_y_offsets[from_level as usize]
            + max_node_heights[from_level as usize] as usize - 1;
        let to_y = level_y_offsets[to_level as usize];

        let from_id = graph.node_id(from_idx);
        let to_id = graph.node_id(to_idx);

        // Get or assign slot for this source at this level
        let slot = if to_level > from_level && (from_level as usize) < max_level as usize + 1 {
            // Check if already assigned
            if node_slots[from_idx] != 0 {
                node_slots[from_idx]
            } else {
                if (from_level as usize) < level_slot_next.len() {
                    let s = level_slot_next[from_level as usize];
                    node_slots[from_idx] = s as usize;
                    level_slot_next[from_level as usize] += 1;
                    s as usize
                } else {
                    0
                }
            }
        } else {
            0
        };

        let edge_start_row = 1 + if has_labeled_edges { 1 } else { 0 };

        let path = if to_level == from_level + 1 {
            if from_x == to_x {
                EdgePathArena::Direct
            } else {
                EdgePathArena::Corner {
                    horizontal_y: from_y + edge_start_row + slot,
                }
            }
        } else if to_level > from_level + 1 {
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
                        EdgePathArena::Direct => from_x,
                        EdgePathArena::Corner { horizontal_y } => {
                            if l_y <= *horizontal_y { from_x } else { to_x }
                        }
                        EdgePathArena::MultiSegment { .. } => from_x,
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
            from_x,
            from_y,
            to_x,
            to_y,

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

    Ok(builder.build())
}

// Helpers for CSR layout (parallel implementation for CsrGraph)

fn alloc_layout_temps_csr<'b>(
    arena: &'b mut Arena<'_>,
    node_count: usize,
    edge_count: usize,
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
    let (level_dummy_next_ptr, _) = arena.alloc_raw_uninit::<Idx>(max_levels + 1)?;
    let (waypoint_scratch_ptr, _) = arena.alloc_raw_uninit::<(usize, usize)>(max_levels + 1)?;
    let (level_vdummy_counts_ptr, _) = arena.alloc_raw_uninit::<Idx>(max_levels + 1)?;

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
            level_dummy_next: core::slice::from_raw_parts_mut(level_dummy_next_ptr, max_levels + 1),
            waypoint_scratch: core::slice::from_raw_parts_mut(waypoint_scratch_ptr, max_levels + 1),
            level_vdummy_counts: core::slice::from_raw_parts_mut(level_vdummy_counts_ptr, max_levels + 1),
            dummy_data: core::slice::from_raw_parts_mut(dummy_data_ptr, max_dummy_waypoints),
            medians: core::slice::from_raw_parts_mut(medians_ptr, max_level_size),
            positions: core::slice::from_raw_parts_mut(positions_ptr, max_level_size),
        })
    }
}

fn calculate_levels_csr(graph: &CsrGraph<'_>, levels: &mut [Idx]) -> Idx {
    for l in levels.iter_mut() {
        *l = 0;
    }

    let mut changed = true;
    let mut passes = 0;
    while changed && passes < levels.len() {
        // Simple cycle protection
        changed = false;
        passes += 1;

        for (from, to) in graph.edges_iter() {
            let new_level = levels[from] + 1;
            if new_level > levels[to] {
                levels[to] = new_level;
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

    for (from, to) in graph.edges_iter() {
        let from_level = node_levels[from] as usize;
        let to_level = node_levels[to] as usize;
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
        let from_level = node_levels[from] as usize;
        let to_level = node_levels[to] as usize;
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
        let offset: usize = if (max_width as usize) > level_width {
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

    // Sort by median
    medians[..count].sort_by_key(|m| m.1);

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
#[cfg(feature = "alloc")]
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
        let mut actual_dummies: usize = 0;
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
                while changed {
                    changed = false;
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
                for &(fi, ti) in &edge_idx {
                    if fi != usize::MAX && ti != usize::MAX {
                        let fl = levels[fi] as usize;
                        let tl = levels[ti] as usize;
                        if tl > fl + 1 {
                            actual_dummies += tl - fl - 1;
                        }
                    }
                }
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
