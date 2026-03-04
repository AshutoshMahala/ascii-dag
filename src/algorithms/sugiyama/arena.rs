//! Arena-based layout computation.
//!
//! This module provides `compute_layout_arena()` which performs the
//! Sugiyama layout algorithm using arena allocation for all temporaries.
//!
//! # Index Type Configuration
//!
//! Memory usage can be significantly reduced by selecting smaller index types:
//!
//! | Feature | Max Nodes | Memory Savings |
//! |---------|-----------|----------------|
//! | `arena-idx-u8` | 255 | ~75% vs u32 |
//! | `arena-idx-u16` | 65,535 | ~50% vs u32 |
//! | `arena-idx-u32` | 4B | baseline |

#[cfg(feature = "alloc")]
use crate::graph::arena::Arena;
#[cfg(feature = "alloc")]
use crate::graph::Graph;
#[cfg(feature = "alloc")]
use crate::ir::arena::{EdgePathArena, LayoutEdgeArena, LayoutIRArena, LayoutIRArenaBuilder};
#[cfg(feature = "alloc")]
use crate::errors::GraphError;

// Re-export CSR layout function for backward compatibility
pub use super::arena_csr::compute_layout_arena_csr;

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

/// Temporary layout data allocated from arena.
/// Uses configurable index types for memory efficiency.
pub(super) struct LayoutTemps<'a> {
    /// Level for each node index
    pub(super) node_levels: &'a mut [Idx],
    /// Edge indices (from_idx, to_idx) - pre-conversion from checking map
    pub(super) edge_indices: &'a mut [(Idx, Idx)],
    /// Virtual levels: offsets into vnode_data
    pub(super) vlevel_offsets: &'a mut [Idx],
    /// Count of nodes per level
    pub(super) level_counts: &'a mut [Idx],
    /// Virtual level data (VNode type + index pairs)
    /// Stored as [type, idx, type, idx, ...] where type: 0=Real, 1=Dummy
    pub(super) vnode_data: &'a mut [Idx],
    /// X coordinates per virtual node
    pub(super) x_coords: &'a mut [Coord],
    /// Widths per virtual node  
    pub(super) widths: &'a mut [Coord],
    /// Real node coords: (level, pos, x, width) per node - needs usize for final output
    pub(super) real_coords: &'a mut [(usize, usize, usize, usize)],
    /// Dummy positions: for skip edges, offsets into dummy_data
    pub(super) dummy_offsets: &'a mut [Idx],
    /// Dummy waypoint data: (level, x) pairs
    pub(super) dummy_data: &'a mut [(Idx, Coord)],
    /// Temporary buffers for crossing reduction
    pub(super) medians: &'a mut [(Idx, u32)], // u32 for f32 bits storage
    pub(super) positions: &'a mut [Idx],

    // -- New buffers for vertical optimization --
    pub(super) node_is_source: &'a mut [bool],
    pub(super) source_counts: &'a mut [Idx],
    pub(super) dummy_counts: &'a mut [Idx],
    pub(super) level_y_offsets: &'a mut [usize], // Must be usize for coordinates
    pub(super) node_slots: &'a mut [usize],
    pub(super) level_slot_next: &'a mut [Idx],
    pub(super) level_dummy_next: &'a mut [Idx],
    pub(super) waypoint_scratch: &'a mut [(usize, usize)],

    // -- Crossing-reduction short-circuit --
    /// Number of dummy vnodes per virtual level (built during build_virtual_levels).
    /// When `level_vdummy_counts[adj_level] == 0`, the O(adj_level_size) dummy
    /// scan in crossing reduction can be skipped entirely.
    pub(super) level_vdummy_counts: &'a mut [Idx],
}

#[cfg(feature = "alloc")]
impl<'a> Graph<'a> {
    /// Compute layout using arena allocation for temporaries.
    ///
    /// This is the arena-based version of `compute_layout()`. All temporary
    /// allocations are made from the provided arena.
    ///
    /// # Arguments
    /// * `arena` - Arena for all allocations (must have sufficient space)
    ///
    /// # Returns
    /// `Ok(LayoutIRArena)` on success, `Err(GraphError)` with a diagnostic
    /// if the arena runs out of space, the graph has cycles, etc.
    ///
    /// # Note
    /// This function requires two arenas: one for temporaries and one for output.
    pub fn compute_layout_arena<'b>(
        &self,
        temp_arena: &mut Arena<'_>,
        output_arena: &'b mut Arena<'b>,
    ) -> Result<LayoutIRArena<'b>, GraphError> {
        if self.nodes.is_empty() {
            return self.build_empty_layout_arena(output_arena);
        }

        // Arena path does not support back-edge reversal — reject cycles
        if self.has_cycle() {
            return Err(GraphError::CycleDetected);
        }

        let node_count = self.nodes.len();
        let edge_count = self.edges.len();

        // Validate against index type limits
        let max_count = node_count.max(edge_count);
        if max_count > MAX_NODES {
            return Err(GraphError::ExceedsMaxNodes { count: max_count, max: MAX_NODES });
        }

        // Calculate total label bytes (nodes + edge labels)
        let node_label_bytes: usize = self.nodes.iter().map(|(_, l)| l.len()).sum();
        let edge_label_bytes: usize = self
            .edges
            .iter()
            .filter_map(|(_, _, label)| label.map(|l| l.len()))
            .sum();
        let total_label_bytes = node_label_bytes + edge_label_bytes;

        // Check if any edges have labels (for lines_per_level adjustment)
        let has_labeled_edges = self.edges.iter().any(|(_, _, label)| label.is_some());

        // Estimate max waypoints: for skip-level edges only
        // Most edges are adjacent-level (0 waypoints), use conservative estimate
        let max_waypoints = (edge_count * 4).min(1000);

        // Two-phase allocation: phase 0 computes levels and counts actual
        // dummies so phase 1 can size buffers precisely.
        let (mut temps, max_level) = self.alloc_layout_temps(temp_arena, node_count, edge_count)
            .ok_or(GraphError::ArenaOom)?;

        // edge_indices and node_levels are already populated by alloc_layout_temps.
        // calculate_levels_arena was run inside alloc_layout_temps.

        // Step 3: Build virtual levels with dummy nodes
        let (_vnode_count, _max_level_size) = self.build_virtual_levels_arena(
            temps.node_levels,
            temps.edge_indices,
            temps.vlevel_offsets,
            temps.level_counts,
            temps.vnode_data,
            max_level,
            temps.level_vdummy_counts,
        );

        // Step 4: Crossing reduction on virtual levels
        self.reduce_crossings_arena(
            temps.vlevel_offsets,
            temps.vnode_data,
            temps.node_levels,
            max_level,
            temps.medians,
            temps.positions,
            temps.edge_indices,
            temps.level_vdummy_counts,
        );

        // Step 5: Assign x-coordinates
        let max_width = self.assign_x_coords_arena(
            temps.vlevel_offsets,
            temps.vnode_data,
            temps.x_coords,
            temps.widths,
            max_level,
        );

        // Step 6: Build real node coordinates
        self.build_real_coords_arena(
            temps.vlevel_offsets,
            temps.vnode_data,
            temps.x_coords,
            temps.widths,
            temps.real_coords,
            max_level,
            max_width,
        );

        // Step 7: Build dummy positions for skip edges using actual virtual level positions
        self.build_dummy_positions_arena(
            temps.vlevel_offsets,
            temps.vnode_data,
            temps.x_coords,
            temps.widths,
            temps.dummy_offsets,
            temps.dummy_data,
            temps.edge_indices,
            max_level,
            max_width,
        );

        // Step 8: Compute horizontal slots for edge separation
        // Count unique sources per level to determine extra rows needed.
        // We use dynamic arrays from arena to avoid stack overflow or bounds checks on deep graphs.

        // Step 8: Compute horizontal slots for edge separation
        // Count unique sources per level to determine extra rows needed.
        // We use dynamic arrays from arena to avoid stack overflow or bounds checks on deep graphs.

        // 1. Mark nodes that are sources for skip-level or adjacent edges that need separation
        // Initialize buffer since it was allocated as uninit
        temps.node_is_source.fill(false);
        let node_is_source = &mut temps.node_is_source;

        for &(from_idx, to_idx) in temps.edge_indices.iter() {
            if from_idx != Idx::MAX && to_idx != Idx::MAX {
                // Indices are valid
                let from_idx = from_idx as usize;
                let to_idx = to_idx as usize;
                let from_level = temps.node_levels[from_idx] as usize;
                let to_level = temps.node_levels[to_idx] as usize;

                // If edge goes down, the source node claims a slot at its level
                if to_level > from_level {
                    node_is_source[from_idx] = true;
                }
            }
        }

        // 2. Count sources per level
        temps.source_counts.fill(0);
        let source_counts = &mut temps.source_counts;
        for (idx, &is_source) in node_is_source.iter().enumerate() {
            if is_source {
                let level = temps.node_levels[idx] as usize;
                if level <= max_level {
                    source_counts[level] += 1;
                }
            }
        }

        // 3. Count dummy nodes per level
        temps.dummy_counts.fill(0);
        let dummy_counts = &mut temps.dummy_counts;
        let total_dummies = temps.dummy_offsets[edge_count] as usize;
        for &(level, _) in &temps.dummy_data[..total_dummies] {
            let lvl = level as usize;
            if lvl <= max_level {
                dummy_counts[lvl] += 1;
            }
        }

        // Step 9: Build per-level Y offsets (Variable Row Heights)
        temps.level_y_offsets.fill(0);
        let level_y_offsets = &mut temps.level_y_offsets;

        let mut current_offset = 0;
        let base_lines = if has_labeled_edges { 5 } else { 3 };

        for level in 0..=max_level {
            level_y_offsets[level] = current_offset;

            let slots = source_counts[level].max(dummy_counts[level]);
            let height = base_lines + slots.saturating_sub(1);
            current_offset += height as usize;
        }
        level_y_offsets[max_level + 1] = current_offset; // Total height
        let total_height = current_offset;

        // Add width margin for labels if present
        let label_margin = if has_labeled_edges { 8 } else { 0 };

        let mut builder = LayoutIRArenaBuilder::new(
            output_arena,
            node_count,
            edge_count,
            max_waypoints,
            total_label_bytes,
            max_level + 1,
        ).ok_or(GraphError::BuilderFailed)?;

        // Add buffer for edge routing (+4) plus label margin
        builder.set_dimensions(max_width + 4 + label_margin, total_height);
        builder.set_level_count(max_level + 1);

        // Add nodes
        for (idx, &(id, label)) in self.nodes.iter().enumerate() {
            let (level, pos, x, width) = temps.real_coords[idx];
            let y = level_y_offsets[level];

            builder.add_node(id, label, x, y, width, level, pos)
                .ok_or(GraphError::ArenaOom)?;
            builder.add_node_to_level(level, idx)
                .ok_or(GraphError::ArenaOom)?;
        }

        builder.finalize_levels();

        temps.node_slots.fill(usize::MAX);
        let node_slots = &mut temps.node_slots;

        temps.level_slot_next.fill(0);
        let level_slot_next = &mut temps.level_slot_next;

        temps.level_dummy_next.fill(0);
        let level_dummy_next = &mut temps.level_dummy_next;

        // Add edges
        for (edge_idx, &(from_id, to_id, _label)) in self.edges.iter().enumerate() {
            // Use pre-resolved indices
            let (from_idx, to_idx) = temps.edge_indices[edge_idx];

            if from_idx != Idx::MAX && to_idx != Idx::MAX {
                let from_idx = from_idx as usize;
                let to_idx = to_idx as usize;

                let (from_level, _, from_x_base, from_width) = temps.real_coords[from_idx];
                let (to_level, _, to_x_base, to_width) = temps.real_coords[to_idx];

                let from_x = from_x_base + from_width / 2;
                let to_x = to_x_base + to_width / 2;
                let from_y = level_y_offsets[from_level];
                let to_y = level_y_offsets[to_level];

                // Assign a horizontal slot for this source node
                let slot = if to_level > from_level {
                    if node_slots[from_idx] != usize::MAX {
                        node_slots[from_idx]
                    } else {
                        let s = level_slot_next[from_level];
                        level_slot_next[from_level] += 1;
                        node_slots[from_idx] = s as usize;
                        s as usize
                    }
                } else {
                    0
                };

                // Horizontal edges start at row 1 below the node
                let edge_start_row = 1;

                let path = if to_level == from_level + 1 {
                    if from_x == to_x {
                        EdgePathArena::Direct
                    } else {
                        EdgePathArena::Corner {
                            horizontal_y: from_y + edge_start_row + slot,
                        }
                    }
                } else if to_level > from_level + 1 {
                    // Skip-level edge - get dummy positions
                    let dummy_start = temps.dummy_offsets[edge_idx] as usize;
                    let dummy_end = temps.dummy_offsets[edge_idx + 1] as usize;
                    let dummy_count = dummy_end - dummy_start;

                    if dummy_count > 0 && dummy_start < temps.dummy_data.len() {
                        // Build waypoints
                        let mut waypoint_buf = [(0usize, 0usize); 64]; // Stack buffer
                        // Limit waypoint_count to available data
                        let available = temps.dummy_data.len().saturating_sub(dummy_start);
                        let waypoint_count = dummy_count.min(64).min(available);

                        for i in 0..waypoint_count {
                            let (level, x) = temps.dummy_data[dummy_start + i];
                            let lvl_idx = level as usize;

                            // Assign a unique vertical slot for this edge at this level
                            let dummy_slot = if lvl_idx < level_dummy_next.len() {
                                let s = level_dummy_next[lvl_idx];
                                level_dummy_next[lvl_idx] += 1;
                                s
                            } else {
                                0
                            };

                            waypoint_buf[i] = (
                                x as usize,
                                level_y_offsets[lvl_idx] + edge_start_row + dummy_slot as usize,
                            );
                        }

                        if let Some((start, len)) =
                            builder.add_waypoints(&waypoint_buf[..waypoint_count])
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
                let (label_offset, label_len, label_x, label_y) =
                    if let Some(label) = self.edges[edge_idx].2 {
                        // Add label to arena's label storage
                        if let Some((offset, len)) = builder.add_edge_label(label) {
                            // Compute label position: label_y at from_y + 3
                            let l_y = from_y + 3;
                            // Center label on edge's vertical line at that row
                            let edge_x_at_label =
                                match &path {
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
                    label_offset,
                    label_len,
                    label_x,
                    label_y,
                    // Edge draws between from_y+1 and to_y-1 (below source, above target)
                    min_y: from_y + 1,
                    max_y: to_y.saturating_sub(1),
                });
            }
        }

        Ok(builder.build())
    }

    /// Build an empty layout IR.
    fn build_empty_layout_arena<'b>(&self, arena: &'b mut Arena<'b>) -> Result<LayoutIRArena<'b>, GraphError> {
        let builder = LayoutIRArenaBuilder::new(arena, 0, 0, 0, 0, 1)
            .ok_or(GraphError::BuilderFailed)?;
        Ok(builder.build())
    }

    /// Allocate temporary buffers for layout computation.
    ///
    /// Uses **two-phase allocation** (arena bands) to avoid the pessimistic
    /// `edge_count * 4` dummy estimate:
    ///
    /// 1. **Phase 0** — Allocate `node_levels` + `edge_indices` (tiny).
    ///    Compute actual levels.  Count the exact number of dummy nodes.
    /// 2. **Phase 1** — Allocate everything else sized by the *actual* vnode
    ///    count (`node_count + actual_dummy_count`).
    ///
    /// For the 50 k diamond grid this reduces peak temp memory from ~35 MB
    /// to ~6 MB because the graph has zero skip-level edges → zero dummies.
    fn alloc_layout_temps<'b>(
        &self,
        arena: &'b mut Arena<'_>,
        node_count: usize,
        edge_count: usize,
    ) -> Option<(LayoutTemps<'b>, usize)> {
        let max_levels = node_count.min(MAX_LEVELS);

        // ── Phase 0: allocate the minimum needed to compute levels ──────
        let (node_levels_ptr, _) = arena.alloc_raw_uninit::<Idx>(node_count)?;
        let (edge_indices_ptr, _) = arena.alloc_raw_uninit::<(Idx, Idx)>(edge_count)?;

        // Safety: these slices are disjoint and live for 'b.
        let node_levels =
            unsafe { core::slice::from_raw_parts_mut(node_levels_ptr, node_count) };
        let edge_indices =
            unsafe { core::slice::from_raw_parts_mut(edge_indices_ptr, edge_count) };

        // Pre-resolve edge indices (O(E))
        for (i, &(from_id, to_id, _)) in self.edges.iter().enumerate() {
            if let (Some(from_idx), Some(to_idx)) =
                (self.node_index(from_id), self.node_index(to_id))
            {
                edge_indices[i] = (from_idx as Idx, to_idx as Idx);
            } else {
                edge_indices[i] = (Idx::MAX, Idx::MAX);
            }
        }

        // Compute node levels (topological sort)
        for l in node_levels.iter_mut() { *l = 0; }
        let mut changed = true;
        while changed {
            changed = false;
            for &(from_idx, to_idx) in edge_indices.iter() {
                if from_idx != Idx::MAX && to_idx != Idx::MAX {
                    let new_level = node_levels[from_idx as usize] as usize + 1;
                    if new_level > node_levels[to_idx as usize] as usize {
                        node_levels[to_idx as usize] = new_level.min(MAX_LEVELS) as Idx;
                        changed = true;
                    }
                }
            }
        }
        let max_level = node_levels.iter().map(|&l| l as usize).max().unwrap_or(0).min(MAX_LEVELS);

        // Count actual dummy nodes from skip-level edges
        let mut actual_dummy_count: usize = 0;
        for &(from_idx, to_idx) in edge_indices.iter() {
            if from_idx != Idx::MAX && to_idx != Idx::MAX {
                let fl = node_levels[from_idx as usize] as usize;
                let tl = node_levels[to_idx as usize] as usize;
                if tl > fl + 1 {
                    actual_dummy_count += tl - fl - 1;
                }
            }
        }

        // ── Phase 1: allocate remaining buffers sized by actual counts ──
        let max_vnodes = (node_count + actual_dummy_count).min(MAX_NODES);
        let max_level_size = node_count.min(MAX_NODES);

        // Waypoints: at most actual_dummy_count (one per dummy per skip-level).
        // Add a small safety margin (+16) for edge cases.
        let max_dummy_waypoints = (actual_dummy_count + 16).min(MAX_NODES);

        let (vlevel_offsets_ptr, _) = arena.alloc_raw_uninit::<Idx>(max_levels + 2)?;
        let (level_counts_ptr, _) = arena.alloc_raw_uninit::<Idx>(max_levels + 1)?;
        let (vnode_data_ptr, _) = arena.alloc_raw_uninit::<Idx>(max_vnodes * 2)?;
        let (x_coords_ptr, _) = arena.alloc_raw_uninit::<Coord>(max_vnodes)?;
        let (widths_ptr, _) = arena.alloc_raw_uninit::<Coord>(max_vnodes)?;
        let (real_coords_ptr, _) =
            arena.alloc_raw_uninit::<(usize, usize, usize, usize)>(node_count)?;
        let (dummy_offsets_ptr, _) = arena.alloc_raw_uninit::<Idx>(edge_count + 1)?;
        let (dummy_data_ptr, _) = arena.alloc_raw_uninit::<(Idx, Coord)>(max_dummy_waypoints)?;
        let (medians_ptr, _) = arena.alloc_raw_uninit::<(Idx, u32)>(max_level_size)?;
        let (positions_ptr, _) = arena.alloc_raw_uninit::<Idx>(max_level_size)?;

        // Vertical optimization buffers
        let (node_is_source_ptr, _) = arena.alloc_raw_uninit::<bool>(node_count)?;
        let (source_counts_ptr, _) = arena.alloc_raw_uninit::<Idx>(max_levels + 1)?;
        let (dummy_counts_ptr, _) = arena.alloc_raw_uninit::<Idx>(max_levels + 1)?;
        let (level_y_offsets_ptr, _) = arena.alloc_raw_uninit::<usize>(max_levels + 2)?;
        let (node_slots_ptr, _) = arena.alloc_raw_uninit::<usize>(node_count)?;
        let (level_slot_next_ptr, _) = arena.alloc_raw_uninit::<Idx>(max_levels + 1)?;
        let (level_dummy_next_ptr, _) = arena.alloc_raw_uninit::<Idx>(max_levels + 1)?;
        let (waypoint_scratch_ptr, _) = arena.alloc_raw_uninit::<(usize, usize)>(max_levels + 1)?;

        // Crossing-reduction short-circuit: dummy count per virtual level
        let (level_vdummy_counts_ptr, _) = arena.alloc_raw_uninit::<Idx>(max_levels + 1)?;

        unsafe {
            Some((LayoutTemps {
                node_levels,
                edge_indices,
                vlevel_offsets: core::slice::from_raw_parts_mut(vlevel_offsets_ptr, max_levels + 2),
                level_counts: core::slice::from_raw_parts_mut(level_counts_ptr, max_levels + 1),
                vnode_data: core::slice::from_raw_parts_mut(vnode_data_ptr, max_vnodes * 2),
                x_coords: core::slice::from_raw_parts_mut(x_coords_ptr, max_vnodes),
                widths: core::slice::from_raw_parts_mut(widths_ptr, max_vnodes),
                real_coords: core::slice::from_raw_parts_mut(real_coords_ptr, node_count),
                dummy_offsets: core::slice::from_raw_parts_mut(dummy_offsets_ptr, edge_count + 1),
                dummy_data: core::slice::from_raw_parts_mut(dummy_data_ptr, max_dummy_waypoints),
                medians: core::slice::from_raw_parts_mut(medians_ptr, max_level_size),
                positions: core::slice::from_raw_parts_mut(positions_ptr, max_level_size),

                node_is_source: core::slice::from_raw_parts_mut(node_is_source_ptr, node_count),
                source_counts: core::slice::from_raw_parts_mut(source_counts_ptr, max_levels + 1),
                dummy_counts: core::slice::from_raw_parts_mut(dummy_counts_ptr, max_levels + 1),
                level_y_offsets: core::slice::from_raw_parts_mut(
                    level_y_offsets_ptr,
                    max_levels + 2,
                ),
                node_slots: core::slice::from_raw_parts_mut(node_slots_ptr, node_count),
                level_slot_next: core::slice::from_raw_parts_mut(
                    level_slot_next_ptr,
                    max_levels + 1,
                ),
                level_dummy_next: core::slice::from_raw_parts_mut(
                    level_dummy_next_ptr,
                    max_levels + 1,
                ),
                waypoint_scratch: core::slice::from_raw_parts_mut(
                    waypoint_scratch_ptr,
                    max_levels + 1,
                ),
                level_vdummy_counts: core::slice::from_raw_parts_mut(
                    level_vdummy_counts_ptr,
                    max_levels + 1,
                ),
            }, max_level))
        }
    }

    /// Estimate arena size needed for layout computation.
    ///
    /// The estimate is conservative but much tighter than the old
    /// `edge_count × 4` formula.  For graphs with few skip-level edges
    /// (grids, chains, fans) the savings are dramatic.
    ///
    /// Internally this performs a cheap O(N+E) level computation so that
    /// the *actual* dummy count can be measured.
    pub fn estimate_layout_arena_size(&self) -> usize {
        let node_count = self.nodes.len();
        let edge_count = self.edges.len();
        let label_bytes: usize = self.nodes.iter().map(|(_, l)| l.len()).sum();
        let max_levels = node_count.min(MAX_LEVELS);

        // ── Cheap level computation to count actual dummies ────────────
        // We duplicate the level logic inline here rather than requiring
        // an arena, because this runs before the caller has allocated one.
        let mut actual_dummies: usize = 0;
        {
            // Use a temporary Vec only on std; on no_std callers must
            // provide their own buffers or use the pessimistic fallback.
            #[cfg(feature = "std")]
            {
                let mut levels = vec![0u32; node_count];
                // Resolve edge indices
                let edge_idx: Vec<(usize, usize)> = self.edges.iter().map(|&(from_id, to_id, _)| {
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
                // Pessimistic fallback for no_std (no heap for temp Vec)
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
            // Vertical optimization buffers
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

