//! CSR-based arena layout computation.
//!
//! Pure-CSR layout pipeline: avoids all heap allocations and HashMap lookups
//! by operating directly on CSR graph indices.

use crate::graph::arena::Arena;
use crate::graph::csr::CsrGraph;
use crate::ir::arena::{EdgePathArena, LayoutEdgeArena, LayoutIRArena, LayoutIRArenaBuilder};
use super::arena::LayoutTemps;
use super::error::LayoutError;

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

/// Compute layout using arena allocation for temporaries, specialized for CsrGraph.
///
/// This avoids all heap allocations and HashMap lookups by using the CSR indices directly.
pub fn compute_layout_arena_csr<'b>(
    graph: &CsrGraph<'_>,
    temp_arena: &mut Arena<'_>,
    output_arena: &'b mut Arena<'b>,
) -> Result<LayoutIRArena<'b>, LayoutError> {
    let node_count = graph.node_count();
    let edge_count = graph.edge_count();

    // Validate against index type limits
    let max_count = node_count.max(edge_count);
    if max_count > MAX_NODES {
        return Err(LayoutError::ExceedsMaxNodes { count: max_count, max: MAX_NODES });
    }

    // Calculate total label bytes (iterating CSR is cheap)
    let mut total_label_bytes = 0;
    for i in 0..node_count {
        total_label_bytes += graph.node_label(i).len();
    }

    // Estimate max waypoints: for skip-level edges only
    // A skip-level edge spanning k levels needs k-1 waypoints
    // Worst case: all edges span (max_level) levels = edge_count * max_level waypoints
    // But for typical graphs, most edges are adjacent-level (0 waypoints)
    // Use a conservative estimate: avg 2 waypoints per edge (covers most skip edges)
    let max_waypoints = (edge_count * 4).min(1000);

    // Step 1: Allocate temporaries
    let mut temps = alloc_layout_temps_csr(temp_arena, node_count, edge_count)
        .ok_or(LayoutError::ArenaOom)?;

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
    // Count unique sources per level to determine extra rows needed.
    // Use pre-allocated dynamic buffers from temps to avoid borrow errors and stack overflows.

    // 1. Mark nodes that are sources
    temps.node_is_source.fill(false);
    let node_is_source = &mut temps.node_is_source;
    let alloc_size = max_level as usize + 1;

    for (from_id, to_id) in graph.edges_iter() {
        // CsrGraph yields indices directly
        let from_idx = from_id;
        let to_idx = to_id;

        // Safe indexing
        if from_idx < temps.real_coords.len() && to_idx < temps.real_coords.len() {
            let from_level = temps.real_coords[from_idx].0;
            let to_level = temps.real_coords[to_idx].0;

            if to_level > from_level {
                node_is_source[from_idx] = true;
            }
        }
    }

    // 2. Count sources per level
    temps.source_counts.fill(0);
    // Use iterator to count (safer than slice indexing manually)
    for (idx, &is_source) in node_is_source.iter().enumerate() {
        if is_source {
            let level = temps.real_coords[idx].0; // Access level from coords
            if level <= max_level as usize {
                temps.source_counts[level] += 1;
            }
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

    // 4. Compute Y offsets
    temps.level_y_offsets.fill(0);
    let base_lines = 3;
    let mut current_offset = 0;

    for level in 0..=max_level as usize {
        temps.level_y_offsets[level] = current_offset;
        let diff = temps.source_counts[level].max(temps.dummy_counts[level]);
        let height = base_lines + diff.saturating_sub(1);
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
    ).ok_or(LayoutError::BuilderFailed)?;

    // Add buffer for edge routing (+4)
    builder.set_dimensions(max_width as usize + 4, total_height);
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
            level as usize,
            pos as usize,
        ).ok_or(LayoutError::ArenaOom)?;
        builder.add_node_to_level(level as usize, idx)
            .ok_or(LayoutError::ArenaOom)?;
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

    // Add edges
    for (edge_idx, (from_idx, to_idx)) in graph.edges_iter().enumerate() {
        let (from_level, _, from_x_base, from_width) = temps.real_coords[from_idx];
        let (to_level, _, to_x_base, to_width) = temps.real_coords[to_idx];

        let from_x = (from_x_base + from_width / 2) as usize;
        let to_x = (to_x_base + to_width / 2) as usize;
        let from_y = level_y_offsets[from_level as usize];
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

        // CsR graphs don't support edge labels currently
        let has_labeled_edges = false;
        let edge_start_row = if has_labeled_edges { 2 } else { 1 };

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

                    // Calculate Y using level_y_offsets
                    let y_base = level_y_offsets[lvl_idx];
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

        builder.add_edge(LayoutEdgeArena {
            from_id,
            to_id,
            from_x,
            from_y,
            to_x,
            to_y,

            path,
            edge_index: edge_idx,
            label_offset: 0,
            label_len: 0,
            label_x: 0,
            label_y: 0,
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
    let (edge_indices_ptr, _) = arena.alloc_raw_uninit::<(Idx, Idx)>(0)?; // Optimization for CSR
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

    unsafe {
        Some(LayoutTemps {
            node_levels: core::slice::from_raw_parts_mut(node_levels_ptr, node_count),
            edge_indices: core::slice::from_raw_parts_mut(edge_indices_ptr, 0),
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
                // Real node: use graph.node_label(idx).len() + padding
                (graph.node_label(vnode_idx).len() + 2) as Coord // +2 for brackets []
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
