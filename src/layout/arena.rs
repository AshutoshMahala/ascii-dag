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

use crate::arena::Arena;
use crate::graph::DAG;
use crate::ir::arena::{EdgePathArena, LayoutEdgeArena, LayoutIRArena, LayoutIRArenaBuilder};

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
const MAX_LEVELS: usize = 255;

/// Temporary layout data allocated from arena.
/// Uses configurable index types for memory efficiency.
struct LayoutTemps<'a> {
    /// Level for each node index
    node_levels: &'a mut [Idx],
    /// Virtual levels: offsets into vnode_data
    vlevel_offsets: &'a mut [Idx],
    /// Count of nodes per level
    level_counts: &'a mut [Idx],
    /// Virtual level data (VNode type + index pairs)
    /// Stored as [type, idx, type, idx, ...] where type: 0=Real, 1=Dummy
    vnode_data: &'a mut [Idx],
    /// X coordinates per virtual node
    x_coords: &'a mut [Coord],
    /// Widths per virtual node  
    widths: &'a mut [Coord],
    /// Real node coords: (level, pos, x, width) per node - needs usize for final output
    real_coords: &'a mut [(usize, usize, usize, usize)],
    /// Dummy positions: for skip edges, offsets into dummy_data
    dummy_offsets: &'a mut [Idx],
    /// Dummy waypoint data: (level, x) pairs
    dummy_data: &'a mut [(Idx, Coord)],
    /// Temporary buffers for crossing reduction
    medians: &'a mut [(Idx, u32)], // u32 for f32 bits storage
    positions: &'a mut [Idx],
}

impl<'a> DAG<'a> {
    /// Compute layout using arena allocation for temporaries.
    ///
    /// This is the arena-based version of `compute_layout()`. All temporary
    /// allocations are made from the provided arena.
    ///
    /// # Arguments
    /// * `arena` - Arena for all allocations (must have sufficient space)
    ///
    /// # Returns
    /// `Some(LayoutIRArena)` on success, `None` if arena runs out of space
    /// or the graph has cycles.
    ///
    /// # Note
    /// This function requires two arenas: one for temporaries and one for output.
    /// Use `compute_layout_arena_split` for the two-arena version.
    pub fn compute_layout_arena<'b>(
        &self,
        temp_arena: &mut Arena<'_>,
        output_arena: &'b mut Arena<'b>,
    ) -> Option<LayoutIRArena<'b>> {
        if self.nodes.is_empty() {
            return self.build_empty_layout_arena(output_arena);
        }

        // Check for cycles
        if self.has_cycle() {
            return self.build_empty_layout_arena(output_arena);
        }

        let node_count = self.nodes.len();
        let edge_count = self.edges.len();

        // Validate against index type limits
        if node_count > MAX_NODES || edge_count > MAX_NODES {
            return None; // Graph too large for selected index type
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

        // Step 1: Allocate temporary buffers from temp arena
        let temps = self.alloc_layout_temps(temp_arena, node_count, edge_count)?;

        // Step 2: Calculate levels
        let max_level = self.calculate_levels_arena(temps.node_levels);

        // Step 3: Build virtual levels with dummy nodes
        let (_vnode_count, _max_level_size) = self.build_virtual_levels_arena(
            temps.node_levels,
            temps.vlevel_offsets,
            temps.level_counts,
            temps.vnode_data,
            max_level,
        );

        // Step 4: Crossing reduction on virtual levels
        self.reduce_crossings_arena(
            temps.vlevel_offsets,
            temps.vnode_data,
            temps.node_levels,
            max_level,
            temps.medians,
            temps.positions,
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
            max_level,
            max_width,
        );

        // Step 8: Build the final LayoutIRArena from output arena
        // Use 4 lines per level when edges have labels (extra row for label)
        let lines_per_level = if has_labeled_edges { 4 } else { 3 };
        let total_height = (max_level + 1) * lines_per_level;

        // Add width margin for labels if present
        let label_margin = if has_labeled_edges { 8 } else { 0 };

        let mut builder = LayoutIRArenaBuilder::new(
            output_arena,
            node_count,
            edge_count,
            max_waypoints,
            total_label_bytes,
            max_level + 1,
        )?;

        // Add buffer for edge routing (+4) plus label margin
        builder.set_dimensions(max_width + 4 + label_margin, total_height);
        builder.set_level_count(max_level + 1);

        // Add nodes
        for (idx, &(id, label)) in self.nodes.iter().enumerate() {
            let (level, pos, x, width) = temps.real_coords[idx];
            let y = level * lines_per_level;

            builder.add_node(id, label, x, y, width, level, pos)?;
            builder.add_node_to_level(level, idx)?;
        }

        builder.finalize_levels();

        // Add edges
        for (edge_idx, &(from_id, to_id, _label)) in self.edges.iter().enumerate() {
            if let (Some(from_idx), Some(to_idx)) =
                (self.node_index(from_id), self.node_index(to_id))
            {
                let (from_level, _, from_x_base, from_width) = temps.real_coords[from_idx];
                let (to_level, _, to_x_base, to_width) = temps.real_coords[to_idx];

                let from_x = from_x_base + from_width / 2;
                let to_x = to_x_base + to_width / 2;
                let from_y = from_level * lines_per_level;
                let to_y = to_level * lines_per_level;

                let path = if to_level == from_level + 1 {
                    if from_x == to_x {
                        EdgePathArena::Direct
                    } else {
                        EdgePathArena::Corner {
                            horizontal_y: from_y + 1,
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
                            waypoint_buf[i] = (x as usize, level as usize * lines_per_level);
                        }

                        if let Some((start, len)) =
                            builder.add_waypoints(&waypoint_buf[..waypoint_count])
                        {
                            EdgePathArena::MultiSegment {
                                waypoints_start: start,
                                waypoints_len: len,
                            }
                        } else {
                            EdgePathArena::Corner {
                                horizontal_y: from_y + 1,
                            }
                        }
                    } else {
                        EdgePathArena::Corner {
                            horizontal_y: from_y + 1,
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
                            // Compute label position: label_y at from_y + 2
                            let l_y = from_y + 2;
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

        Some(builder.build())
    }

    /// Build an empty layout IR.
    fn build_empty_layout_arena<'b>(&self, arena: &'b mut Arena<'b>) -> Option<LayoutIRArena<'b>> {
        let builder = LayoutIRArenaBuilder::new(arena, 0, 0, 0, 0, 1)?;
        Some(builder.build())
    }

    /// Allocate temporary buffers for layout computation.
    fn alloc_layout_temps<'b>(
        &self,
        arena: &'b mut Arena<'_>,
        node_count: usize,
        edge_count: usize,
    ) -> Option<LayoutTemps<'b>> {
        // Tighter size estimates based on actual graph structure
        // Max levels is min(node_count, MAX_LEVELS)
        let max_levels = node_count.min(MAX_LEVELS);

        // Virtual nodes = real + dummy nodes from skip-level edges.
        // Most edges span only 1 level (no dummies). Skip-level edges typically span 2-4 levels.
        // Use a more reasonable estimate: each edge creates at most 4 dummy nodes on average,
        // which handles most practical graphs while avoiding huge over-allocation.
        let max_vnodes = (node_count + edge_count * 4).min(MAX_NODES);

        // Level size: at most node_count (if all nodes on one level)
        let max_level_size = node_count.min(MAX_NODES);

        // Dummy waypoints: each skip-level edge needs waypoints. Use edge_count * 4.
        let max_dummy_waypoints = (edge_count * 4).min(MAX_NODES);

        // Allocate all buffers using compact types
        let (node_levels_ptr, _) = arena.alloc_raw_uninit::<Idx>(node_count)?;
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

        unsafe {
            Some(LayoutTemps {
                node_levels: core::slice::from_raw_parts_mut(node_levels_ptr, node_count),
                vlevel_offsets: core::slice::from_raw_parts_mut(vlevel_offsets_ptr, max_levels + 1),
                level_counts: core::slice::from_raw_parts_mut(level_counts_ptr, max_levels),
                vnode_data: core::slice::from_raw_parts_mut(vnode_data_ptr, max_vnodes * 2),
                x_coords: core::slice::from_raw_parts_mut(x_coords_ptr, max_vnodes),
                widths: core::slice::from_raw_parts_mut(widths_ptr, max_vnodes),
                real_coords: core::slice::from_raw_parts_mut(real_coords_ptr, node_count),
                dummy_offsets: core::slice::from_raw_parts_mut(dummy_offsets_ptr, edge_count + 1),
                dummy_data: core::slice::from_raw_parts_mut(dummy_data_ptr, max_dummy_waypoints),
                medians: core::slice::from_raw_parts_mut(medians_ptr, max_level_size),
                positions: core::slice::from_raw_parts_mut(positions_ptr, max_level_size),
            })
        }
    }

    /// Calculate levels using arena-allocated buffer.
    fn calculate_levels_arena(&self, levels: &mut [Idx]) -> usize {
        // Initialize all to 0
        for l in levels.iter_mut() {
            *l = 0;
        }

        let mut changed = true;
        while changed {
            changed = false;
            for &(from, to, _) in &self.edges {
                if let (Some(from_idx), Some(to_idx)) = (self.node_index(from), self.node_index(to))
                {
                    let new_level = levels[from_idx] as usize + 1;
                    if new_level > levels[to_idx] as usize {
                        // Safe cast - we cap at MAX_LEVELS which fits in Idx
                        levels[to_idx] = new_level.min(MAX_LEVELS) as Idx;
                        changed = true;
                    }
                }
            }
        }

        // Cap to MAX_LEVELS
        levels
            .iter()
            .map(|&l| l as usize)
            .max()
            .unwrap_or(0)
            .min(MAX_LEVELS)
    }

    /// Build virtual levels with dummy nodes.
    /// Returns (total_vnode_count, max_level_size).
    fn build_virtual_levels_arena(
        &self,
        node_levels: &[Idx],
        vlevel_offsets: &mut [Idx],
        level_counts: &mut [Idx],
        vnode_data: &mut [Idx],
        max_level: usize,
    ) -> (usize, usize) {
        // Zero level_counts
        for c in level_counts.iter_mut() {
            *c = 0;
        }

        // Count nodes per level
        for &level in node_levels.iter() {
            let lvl = level as usize;
            if lvl < level_counts.len() {
                level_counts[lvl] += 1;
            }
        }

        // Count dummy nodes per level
        for &(from_id, to_id, _) in &self.edges {
            if let (Some(from_idx), Some(to_idx)) =
                (self.node_index(from_id), self.node_index(to_id))
            {
                let from_level = node_levels[from_idx] as usize;
                let to_level = node_levels[to_idx] as usize;

                if to_level > from_level + 1 {
                    for level in (from_level + 1)..to_level {
                        if level < level_counts.len() {
                            level_counts[level] += 1;
                        }
                    }
                }
            }
        }

        // Build offsets (capped to array bounds)
        vlevel_offsets[0] = 0;
        let effective_max_level = max_level.min(level_counts.len().saturating_sub(1));
        for level in 0..=effective_max_level {
            vlevel_offsets[level + 1] = vlevel_offsets[level] + level_counts[level];
        }

        // Reset counts for filling
        for c in level_counts.iter_mut() {
            *c = 0;
        }

        // Fill with real nodes
        for (idx, &level) in node_levels.iter().enumerate() {
            let lvl = level as usize;
            if lvl <= effective_max_level {
                let pos = (vlevel_offsets[lvl] + level_counts[lvl]) as usize;
                // Bounds check for safety - skip if buffer exhausted
                if pos * 2 + 1 >= vnode_data.len() {
                    continue;
                }
                // Store VNode::Real(idx) as (0, idx)
                vnode_data[pos * 2] = 0; // 0 = Real
                vnode_data[pos * 2 + 1] = idx as Idx;
                level_counts[lvl] += 1;
            }
        }

        // Fill with dummy nodes
        for (edge_idx, &(from_id, to_id, _)) in self.edges.iter().enumerate() {
            if let (Some(from_idx), Some(to_idx)) =
                (self.node_index(from_id), self.node_index(to_id))
            {
                let from_level = node_levels[from_idx] as usize;
                let to_level = node_levels[to_idx] as usize;

                if to_level > from_level + 1 {
                    for level in (from_level + 1)..to_level {
                        if level <= effective_max_level {
                            let pos = (vlevel_offsets[level] + level_counts[level]) as usize;
                            // Bounds check for safety - skip if buffer exhausted
                            if pos * 2 + 1 >= vnode_data.len() {
                                continue;
                            }
                            // Store VNode::Dummy{edge_idx} as (1, edge_idx)
                            vnode_data[pos * 2] = 1; // 1 = Dummy
                            vnode_data[pos * 2 + 1] = edge_idx as Idx;
                            level_counts[level] += 1;
                        }
                    }
                }
            }
        }

        let total = vlevel_offsets[effective_max_level + 1] as usize;
        let max_size = level_counts.iter().map(|&c| c as usize).max().unwrap_or(0);
        (total, max_size)
    }

    /// Crossing reduction on arena-allocated virtual levels.
    fn reduce_crossings_arena(
        &self,
        _vlevel_offsets: &mut [Idx],
        _vnode_data: &mut [Idx],
        _node_levels: &[Idx],
        _max_level: usize,
        _medians: &mut [(Idx, u32)],
        _positions: &mut [Idx],
    ) {
        // TODO: Implement full crossing reduction
        // For now, skip - nodes stay in insertion order
        // This is a simplification; full impl would do median heuristic
    }

    /// Assign x-coordinates to virtual nodes.
    fn assign_x_coords_arena(
        &self,
        vlevel_offsets: &[Idx],
        vnode_data: &[Idx],
        x_coords: &mut [Coord],
        widths: &mut [Coord],
        max_level: usize,
    ) -> usize {
        let mut max_width: usize = 0;
        let max_pos = x_coords.len();
        let max_vnode_idx = vnode_data.len() / 2;

        for level in 0..=max_level {
            let start = vlevel_offsets[level] as usize;
            let end = (vlevel_offsets[level + 1] as usize)
                .min(max_pos)
                .min(max_vnode_idx);
            let mut x: usize = 0;

            for pos in start..end {
                // Bounds check
                if pos * 2 + 1 >= vnode_data.len() {
                    break;
                }
                let vnode_type = vnode_data[pos * 2];
                let vnode_idx = vnode_data[pos * 2 + 1] as usize;

                let width = if vnode_type == 0 {
                    // Real node
                    self.get_node_width(vnode_idx)
                } else {
                    // Dummy node - use width 3 for visual separation (matches heap mode)
                    3
                };

                if pos < x_coords.len() {
                    x_coords[pos] = x as Coord;
                    widths[pos] = width as Coord;
                }
                x += width + 3; // spacing between nodes
            }

            // Level width is the rightmost edge of the last node
            if end > start && end - 1 < x_coords.len() {
                let last_x = x_coords[end - 1] as usize;
                let last_width = widths[end - 1] as usize;
                let level_width = last_x + last_width;
                max_width = max_width.max(level_width);
            }
        }

        max_width
    }

    /// Build real node coordinates from virtual level positions.
    fn build_real_coords_arena(
        &self,
        vlevel_offsets: &[Idx],
        vnode_data: &[Idx],
        x_coords: &[Coord],
        widths: &[Coord],
        real_coords: &mut [(usize, usize, usize, usize)],
        max_level: usize,
        max_width: usize,
    ) {
        let max_pos = x_coords.len();
        let max_vnode_idx = vnode_data.len() / 2;

        // Calculate level widths for centering
        for level in 0..=max_level {
            let start = vlevel_offsets[level] as usize;
            let end = (vlevel_offsets[level + 1] as usize)
                .min(max_pos)
                .min(max_vnode_idx);

            if end <= start {
                continue;
            }

            // Calculate this level's width (with bounds check)
            let level_width = if end > start && end - 1 < x_coords.len() {
                x_coords[end - 1] as usize + widths[end - 1] as usize
            } else {
                0
            };

            let offset = if max_width > level_width {
                (max_width - level_width) / 2
            } else {
                0
            };

            // Find real nodes at this level
            for pos in start..end {
                // Bounds check
                if pos * 2 + 1 >= vnode_data.len() || pos >= x_coords.len() {
                    break;
                }
                let vnode_type = vnode_data[pos * 2];
                let vnode_idx = vnode_data[pos * 2 + 1] as usize;

                if vnode_type == 0 && vnode_idx < real_coords.len() {
                    // Real node
                    let x = x_coords[pos] as usize + offset;
                    let width = widths[pos] as usize;
                    let level_pos = pos - start;
                    real_coords[vnode_idx] = (level, level_pos, x, width);
                }
            }
        }
    }

    /// Build dummy positions for skip-level edges from virtual level positions.
    /// This extracts the actual x-coordinates assigned during layout, ensuring edges
    /// route around nodes based on the natural layout ordering (like heap mode does).
    fn build_dummy_positions_arena(
        &self,
        vlevel_offsets: &[Idx],
        vnode_data: &[Idx],
        x_coords: &[Coord],
        widths: &[Coord],
        dummy_offsets: &mut [Idx],
        dummy_data: &mut [(Idx, Coord)],
        max_level: usize,
        max_width: usize,
    ) {
        let edge_count = self.edges.len();
        let max_vnode_idx = vnode_data.len() / 2;

        // Initialize offsets to 0
        dummy_offsets[0] = 0;
        for i in 1..=edge_count {
            if i < dummy_offsets.len() {
                dummy_offsets[i] = 0;
            }
        }

        // Collect dummy positions per edge into a temporary buffer
        // First, count how many dummy nodes each edge has
        let mut edge_dummy_counts = [0u16; 512]; // Support up to 512 edges

        for level in 0..=max_level {
            let start = vlevel_offsets[level] as usize;
            let end = (vlevel_offsets[level + 1] as usize).min(max_vnode_idx);

            for pos in start..end {
                // Bounds check
                if pos * 2 + 1 >= vnode_data.len() {
                    break;
                }
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
        for level in 0..=max_level {
            let start = vlevel_offsets[level] as usize;
            let end = (vlevel_offsets[level + 1] as usize)
                .min(max_vnode_idx)
                .min(x_coords.len());

            // Calculate centering offset for this level
            let level_width = if end > start && end - 1 < x_coords.len() {
                x_coords[end - 1] as usize + widths[end - 1] as usize
            } else {
                0
            };
            let offset = if max_width > level_width {
                (max_width - level_width) / 2
            } else {
                0
            };

            for pos in start..end {
                // Bounds check
                if pos * 2 + 1 >= vnode_data.len() || pos >= x_coords.len() {
                    break;
                }
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

    /// Estimate arena size needed for layout computation.
    pub fn estimate_layout_arena_size(&self) -> usize {
        let node_count = self.nodes.len();
        let edge_count = self.edges.len();
        let label_bytes: usize = self.nodes.iter().map(|(_, l)| l.len()).sum();

        // Calculate the same values used in alloc_layout_temps
        let max_levels = node_count.min(MAX_LEVELS);
        let max_vnodes = (node_count + edge_count * 4).min(MAX_NODES);
        let max_level_size = node_count.min(MAX_NODES);
        let max_dummy_waypoints = (edge_count * 4).min(MAX_NODES);

        // Calculate actual temporary buffer sizes (matching alloc_layout_temps)
        // Using core::mem::size_of for each type
        let temps_size = node_count * core::mem::size_of::<Idx>()                      // node_levels
            + (max_levels + 1) * core::mem::size_of::<Idx>()              // vlevel_offsets
            + max_levels * core::mem::size_of::<Idx>()                    // level_counts
            + max_vnodes * 2 * core::mem::size_of::<Idx>()                // vnode_data
            + max_vnodes * core::mem::size_of::<Coord>()                  // x_coords
            + max_vnodes * core::mem::size_of::<Coord>()                  // widths
            + node_count * core::mem::size_of::<(usize, usize, usize, usize)>() // real_coords
            + (edge_count + 1) * core::mem::size_of::<Idx>()              // dummy_offsets
            + max_dummy_waypoints * core::mem::size_of::<(Idx, Coord)>()  // dummy_data
            + max_level_size * core::mem::size_of::<(Idx, u32)>()         // medians
            + max_level_size * core::mem::size_of::<Idx>()                // positions
            + 4096; // alignment padding buffer

        // Output IR: waypoints estimate should match max_dummy_waypoints (edge_count * 4)
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

// =========================================================================
// Pure CSR Layout Implementation
// =========================================================================

use crate::csr::CsrGraph;

/// Compute layout using arena allocation for temporaries, specialized for CsrGraph.
///
/// This avoids all heap allocations and HashMap lookups by using the CSR indices directly.
pub fn compute_layout_arena_csr<'b>(
    graph: &CsrGraph<'_>,
    temp_arena: &mut Arena<'_>,
    output_arena: &'b mut Arena<'b>,
) -> Option<LayoutIRArena<'b>> {
    let node_count = graph.node_count();
    let edge_count = graph.edge_count();

    // Validate against index type limits
    if node_count > MAX_NODES || edge_count > MAX_NODES {
        return None; // Graph too large for selected index type
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
    let temps = alloc_layout_temps_csr(temp_arena, node_count, edge_count)?;

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

    // Step 4: Crossing reduction (skipped for now, same as DAG impl)

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

    // Step 8: Build LayoutIRArena
    let lines_per_level: usize = 3;
    let total_height = (max_level as usize + 1) * lines_per_level;

    let mut builder = LayoutIRArenaBuilder::new(
        output_arena,
        node_count,
        edge_count,
        max_waypoints,
        total_label_bytes,
        max_level as usize + 1,
    )?;

    // Add buffer for edge routing (+4)
    builder.set_dimensions(max_width as usize + 4, total_height);
    builder.set_level_count(max_level as usize + 1);

    // Add nodes
    for idx in 0..node_count {
        let (level, pos, x, width) = temps.real_coords[idx];
        let y = level as usize * lines_per_level;
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
        )?;
        builder.add_node_to_level(level as usize, idx)?;
    }

    builder.finalize_levels();

    // Add edges
    for (edge_idx, (from_idx, to_idx)) in graph.edges_iter().enumerate() {
        let (from_level, _, from_x_base, from_width) = temps.real_coords[from_idx];
        let (to_level, _, to_x_base, to_width) = temps.real_coords[to_idx];

        let from_x = (from_x_base + from_width / 2) as usize;
        let to_x = (to_x_base + to_width / 2) as usize;
        let from_y = from_level as usize * lines_per_level;
        let to_y = to_level as usize * lines_per_level;

        let from_id = graph.node_id(from_idx);
        let to_id = graph.node_id(to_idx);

        let path = if to_level == from_level + 1 {
            if from_x == to_x {
                EdgePathArena::Direct
            } else {
                EdgePathArena::Corner {
                    horizontal_y: from_y + 1,
                }
            }
        } else if to_level > from_level + 1 {
            let dummy_start = temps.dummy_offsets[edge_idx] as usize;
            let dummy_end = temps.dummy_offsets[edge_idx + 1] as usize;
            let dummy_count = dummy_end - dummy_start;

            if dummy_count > 0 && dummy_start < temps.dummy_data.len() {
                let mut waypoint_buf = [(0usize, 0usize); 64];
                // Limit waypoint_count to available data
                let available = temps.dummy_data.len().saturating_sub(dummy_start);
                let waypoint_count = dummy_count.min(64).min(available);

                for i in 0..waypoint_count {
                    let (level, x) = temps.dummy_data[dummy_start + i];
                    waypoint_buf[i] = (x as usize, level as usize * lines_per_level);
                }

                if let Some((start, len)) = builder.add_waypoints(&waypoint_buf[..waypoint_count]) {
                    EdgePathArena::MultiSegment {
                        waypoints_start: start,
                        waypoints_len: len,
                    }
                } else {
                    EdgePathArena::Corner {
                        horizontal_y: from_y + 1,
                    }
                }
            } else {
                EdgePathArena::Corner {
                    horizontal_y: from_y + 1,
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

    Some(builder.build())
}

// Helpers for CSR layout
// Duplicated from DAG implementation but using CsrGraph logic
// This is cleaner than trying to abstract it with Traits right now

fn alloc_layout_temps_csr<'b>(
    arena: &'b mut Arena<'_>,
    node_count: usize,
    edge_count: usize,
) -> Option<LayoutTemps<'b>> {
    // Same allocation logic as DAG
    let max_levels = node_count.min(256);
    // Worst case: every edge could skip all levels, creating (max_levels - 1) dummies each
    // Use max_levels as multiplier instead of 4 to handle deep skip-level edges
    let max_vnodes = (node_count + edge_count * max_levels).min(500000);
    let max_level_size = node_count.min(50000);
    let max_dummy_waypoints = (edge_count * 4).min(500000);

    let (node_levels_ptr, _) = arena.alloc_raw_uninit::<Idx>(node_count)?;
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

    unsafe {
        Some(LayoutTemps {
            node_levels: core::slice::from_raw_parts_mut(node_levels_ptr, node_count),
            vlevel_offsets: core::slice::from_raw_parts_mut(vlevel_offsets_ptr, max_levels + 1),
            level_counts: core::slice::from_raw_parts_mut(level_counts_ptr, max_levels),
            vnode_data: core::slice::from_raw_parts_mut(vnode_data_ptr, max_vnodes * 2),
            x_coords: core::slice::from_raw_parts_mut(x_coords_ptr, max_vnodes),
            widths: core::slice::from_raw_parts_mut(widths_ptr, max_vnodes),
            real_coords: core::slice::from_raw_parts_mut(real_coords_ptr, node_count),
            dummy_offsets: core::slice::from_raw_parts_mut(dummy_offsets_ptr, edge_count + 1),
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
