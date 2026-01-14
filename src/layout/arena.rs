//! Arena-based layout computation.
//!
//! This module provides `compute_layout_arena()` which performs the
//! Sugiyama layout algorithm using arena allocation for all temporaries.

use crate::arena::Arena;
use crate::graph::DAG;
use crate::ir::arena::{EdgePathArena, LayoutEdgeArena, LayoutIRArena, LayoutIRArenaBuilder};

/// Temporary layout data allocated from arena.
struct LayoutTemps<'a> {
    /// Level for each node index
    node_levels: &'a mut [usize],
    /// Virtual levels: offsets into vnode_data
    vlevel_offsets: &'a mut [usize],
    /// Count of nodes per level (replaces fixed [0usize; 256])
    level_counts: &'a mut [usize],
    /// Virtual level data (VNode indices stored as usize pairs)
    vnode_data: &'a mut [usize],
    /// X coordinates per virtual node
    x_coords: &'a mut [usize],
    /// Widths per virtual node  
    widths: &'a mut [usize],
    /// Real node coords: (level, pos, x, width) per node
    real_coords: &'a mut [(usize, usize, usize, usize)],
    /// Dummy positions: for skip edges, (level, x) pairs
    dummy_offsets: &'a mut [usize],
    dummy_data: &'a mut [(usize, usize)],
    /// Temporary buffers for crossing reduction
    medians: &'a mut [(usize, f32)],
    positions: &'a mut [usize],
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
        
        // Calculate total label bytes
        let total_label_bytes: usize = self.nodes.iter().map(|(_, l)| l.len()).sum();
        
        // Estimate max waypoints (worst case: every edge skips all levels)
        let max_waypoints = edge_count.saturating_mul(node_count).min(10000);

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
        
        // Step 7: Build dummy positions for skip edges
        self.build_dummy_positions_arena(
            temps.node_levels,
            temps.real_coords,
            temps.dummy_offsets,
            temps.dummy_data,
        );
        
        // Step 8: Build the final LayoutIRArena from output arena
        let lines_per_level = 3;
        let total_height = (max_level + 1) * lines_per_level;
        
        let mut builder = LayoutIRArenaBuilder::new(
            output_arena,
            node_count,
            edge_count,
            max_waypoints,
            total_label_bytes,
            max_level + 1,
        )?;
        
        builder.set_dimensions(max_width, total_height);
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
        for (edge_idx, &(from_id, to_id)) in self.edges.iter().enumerate() {
            if let (Some(from_idx), Some(to_idx)) = (self.node_index(from_id), self.node_index(to_id)) {
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
                        EdgePathArena::Corner { horizontal_y: from_y + 1 }
                    }
                } else if to_level > from_level + 1 {
                    // Skip-level edge - get dummy positions
                    let dummy_start = temps.dummy_offsets[edge_idx];
                    let dummy_end = temps.dummy_offsets[edge_idx + 1];
                    let dummy_count = dummy_end - dummy_start;
                    
                    if dummy_count > 0 {
                        // Build waypoints
                        let mut waypoint_buf = [(0usize, 0usize); 64]; // Stack buffer
                        let waypoint_count = dummy_count.min(64);
                        
                        for i in 0..waypoint_count {
                            let (level, x) = temps.dummy_data[dummy_start + i];
                            waypoint_buf[i] = (x, level * lines_per_level);
                        }
                        
                        if let Some((start, len)) = builder.add_waypoints(&waypoint_buf[..waypoint_count]) {
                            EdgePathArena::MultiSegment {
                                waypoints_start: start,
                                waypoints_len: len,
                            }
                        } else {
                            EdgePathArena::Corner { horizontal_y: from_y + 1 }
                        }
                    } else {
                        EdgePathArena::Corner { horizontal_y: from_y + 1 }
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
        // Max levels is min(node_count, 256) - chains can't be deeper than node count
        let max_levels = node_count.min(256);
        
        // Virtual nodes = real + dummy. Each skip-level edge can create up to max_levels dummies.
        // Conservative estimate: node_count + edge_count * avg_skip_levels
        // For random graphs, avg skip is ~2-3 levels, so use node_count + edge_count * 4
        let max_vnodes = (node_count + edge_count * 4).min(500000);
        
        // Level size: at most node_count (if all nodes on one level)
        let max_level_size = node_count.min(50000);
        
        // Dummy waypoints: each skip-level edge needs waypoints. Use edge_count * 4.
        let max_dummy_waypoints = (edge_count * 4).min(500000);
        
        // Allocate all buffers - use uninit since we'll overwrite everything
        // This avoids double-zeroing (arena zeros, then we zero again)
        let (node_levels_ptr, _) = arena.alloc_raw_uninit::<usize>(node_count)?;
        let (vlevel_offsets_ptr, _) = arena.alloc_raw_uninit::<usize>(max_levels + 1)?;
        let (level_counts_ptr, _) = arena.alloc_raw_uninit::<usize>(max_levels)?; // replaces [0; 256]
        let (vnode_data_ptr, _) = arena.alloc_raw_uninit::<usize>(max_vnodes * 2)?;
        let (x_coords_ptr, _) = arena.alloc_raw_uninit::<usize>(max_vnodes)?;
        let (widths_ptr, _) = arena.alloc_raw_uninit::<usize>(max_vnodes)?;
        let (real_coords_ptr, _) = arena.alloc_raw_uninit::<(usize, usize, usize, usize)>(node_count)?;
        let (dummy_offsets_ptr, _) = arena.alloc_raw_uninit::<usize>(edge_count + 1)?;
        let (dummy_data_ptr, _) = arena.alloc_raw_uninit::<(usize, usize)>(max_dummy_waypoints)?;
        let (medians_ptr, _) = arena.alloc_raw_uninit::<(usize, f32)>(max_level_size)?;
        let (positions_ptr, _) = arena.alloc_raw_uninit::<usize>(max_level_size)?;
        
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
    fn calculate_levels_arena(&self, levels: &mut [usize]) -> usize {
        // Initialize all to 0
        for l in levels.iter_mut() {
            *l = 0;
        }
        
        let mut changed = true;
        while changed {
            changed = false;
            for &(from, to) in &self.edges {
                if let (Some(from_idx), Some(to_idx)) = (self.node_index(from), self.node_index(to)) {
                    let new_level = levels[from_idx] + 1;
                    if new_level > levels[to_idx] {
                        levels[to_idx] = new_level;
                        changed = true;
                    }
                }
            }
        }
        
        // Cap to 255 since our arrays can only handle 256 levels (0-255)
        levels.iter().copied().max().unwrap_or(0).min(255)
    }
    
    /// Build virtual levels with dummy nodes.
    /// Returns (total_vnode_count, max_level_size).
    fn build_virtual_levels_arena(
        &self,
        node_levels: &[usize],
        vlevel_offsets: &mut [usize],
        level_counts: &mut [usize],
        vnode_data: &mut [usize],
        max_level: usize,
    ) -> (usize, usize) {
        // Zero level_counts (this is the only zero we need)
        for c in level_counts.iter_mut() {
            *c = 0;
        }
        
        // Count nodes per level
        for &level in node_levels.iter() {
            if level < level_counts.len() {
                level_counts[level] += 1;
            }
        }
        
        // Count dummy nodes per level
        for &(from_id, to_id) in &self.edges {
            if let (Some(from_idx), Some(to_idx)) = (self.node_index(from_id), self.node_index(to_id)) {
                let from_level = node_levels[from_idx];
                let to_level = node_levels[to_idx];
                
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
        // effective_max_level must be valid index for both level_counts and vlevel_offsets[level+1]
        // level_counts has max_levels elements (0..max_levels-1 valid)
        // vlevel_offsets has max_levels+1 elements (0..max_levels valid for level+1)
        let effective_max_level = max_level.min(level_counts.len().saturating_sub(1));
        for level in 0..=effective_max_level {
            vlevel_offsets[level + 1] = vlevel_offsets[level] + level_counts[level];
        }
        
        // Reset counts for filling
        for c in level_counts.iter_mut() {
            *c = 0;
        }
        
        // Fill with real nodes (use effective_max_level to stay in bounds)
        for (idx, &level) in node_levels.iter().enumerate() {
            if level <= effective_max_level {
                let pos = vlevel_offsets[level] + level_counts[level];
                // Store VNode::Real(idx) as (0, idx)
                vnode_data[pos * 2] = 0; // 0 = Real
                vnode_data[pos * 2 + 1] = idx;
                level_counts[level] += 1;
            }
        }
        
        // Fill with dummy nodes
        for (edge_idx, &(from_id, to_id)) in self.edges.iter().enumerate() {
            if let (Some(from_idx), Some(to_idx)) = (self.node_index(from_id), self.node_index(to_id)) {
                let from_level = node_levels[from_idx];
                let to_level = node_levels[to_idx];
                
                if to_level > from_level + 1 {
                    for level in (from_level + 1)..to_level {
                        if level <= effective_max_level {
                            let pos = vlevel_offsets[level] + level_counts[level];
                            // Store VNode::Dummy{edge_idx} as (1, edge_idx)
                            vnode_data[pos * 2] = 1; // 1 = Dummy
                            vnode_data[pos * 2 + 1] = edge_idx;
                            level_counts[level] += 1;
                        }
                    }
                }
            }
        }
        
        let total = vlevel_offsets[effective_max_level + 1];
        let max_size = level_counts.iter().copied().max().unwrap_or(0);
        (total, max_size)
    }
    
    /// Crossing reduction on arena-allocated virtual levels.
    fn reduce_crossings_arena(
        &self,
        _vlevel_offsets: &mut [usize],
        _vnode_data: &mut [usize],
        _node_levels: &[usize],
        _max_level: usize,
        _medians: &mut [(usize, f32)],
        _positions: &mut [usize],
    ) {
        // TODO: Implement full crossing reduction
        // For now, skip - nodes stay in insertion order
        // This is a simplification; full impl would do median heuristic
    }
    
    /// Assign x-coordinates to virtual nodes.
    fn assign_x_coords_arena(
        &self,
        vlevel_offsets: &[usize],
        vnode_data: &[usize],
        x_coords: &mut [usize],
        widths: &mut [usize],
        max_level: usize,
    ) -> usize {
        let mut max_width = 0;
        
        for level in 0..=max_level {
            let start = vlevel_offsets[level];
            let end = vlevel_offsets[level + 1];
            let mut x = 0;
            
            for pos in start..end {
                let vnode_type = vnode_data[pos * 2];
                let vnode_idx = vnode_data[pos * 2 + 1];
                
                let width = if vnode_type == 0 {
                    // Real node
                    self.get_node_width(vnode_idx)
                } else {
                    // Dummy node
                    1
                };
                
                x_coords[pos] = x;
                widths[pos] = width;
                x += width + 3; // spacing between nodes
            }
            
            // Level width is the rightmost edge of the last node
            // (x - 3 gives us back to the last x_coord, then + width gives rightmost edge)
            if end > start {
                let last_x = x_coords[end - 1];
                let last_width = widths[end - 1];
                let level_width = last_x + last_width;
                max_width = max_width.max(level_width);
            }
        }
        
        max_width
    }
    
    /// Build real node coordinates from virtual level positions.
    fn build_real_coords_arena(
        &self,
        vlevel_offsets: &[usize],
        vnode_data: &[usize],
        x_coords: &[usize],
        widths: &[usize],
        real_coords: &mut [(usize, usize, usize, usize)],
        max_level: usize,
        max_width: usize,
    ) {
        // Calculate level widths for centering
        for level in 0..=max_level {
            let start = vlevel_offsets[level];
            let end = vlevel_offsets[level + 1];
            
            if end <= start {
                continue;
            }
            
            // Calculate this level's width
            let level_width = if end > start {
                x_coords[end - 1] + widths[end - 1]
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
                let vnode_type = vnode_data[pos * 2];
                let vnode_idx = vnode_data[pos * 2 + 1];
                
                if vnode_type == 0 {
                    // Real node
                    let x = x_coords[pos] + offset;
                    let width = widths[pos];
                    let level_pos = pos - start;
                    real_coords[vnode_idx] = (level, level_pos, x, width);
                }
            }
        }
    }
    
    /// Build dummy positions for skip-level edges.
    fn build_dummy_positions_arena(
        &self,
        node_levels: &[usize],
        real_coords: &[(usize, usize, usize, usize)],
        dummy_offsets: &mut [usize],
        dummy_data: &mut [(usize, usize)],
    ) {
        dummy_offsets[0] = 0;
        let mut dummy_count = 0;
        
        for (edge_idx, &(from_id, to_id)) in self.edges.iter().enumerate() {
            if let (Some(from_idx), Some(to_idx)) = (self.node_index(from_id), self.node_index(to_id)) {
                let from_level = node_levels[from_idx];
                let to_level = node_levels[to_idx];
                
                if to_level > from_level + 1 {
                    let (_, _, from_x_base, from_width) = real_coords[from_idx];
                    let (_, _, to_x_base, to_width) = real_coords[to_idx];
                    
                    let from_center = from_x_base + from_width / 2;
                    let to_center = to_x_base + to_width / 2;
                    let total_span = to_level - from_level;
                    
                    for level in (from_level + 1)..to_level {
                        // Interpolate x position using integer arithmetic for no_std compatibility
                        let t_num = level - from_level;
                        let t_denom = total_span;
                        let delta = to_center as isize - from_center as isize;
                        let x = (from_center as isize + (delta * t_num as isize + t_denom as isize / 2) / t_denom as isize) as usize;
                        
                        if dummy_count < dummy_data.len() {
                            dummy_data[dummy_count] = (level, x);
                            dummy_count += 1;
                        }
                    }
                }
            }
            
            dummy_offsets[edge_idx + 1] = dummy_count;
        }
    }
    
    /// Estimate arena size needed for layout computation.
    pub fn estimate_layout_arena_size(&self) -> usize {
        let node_count = self.nodes.len();
        let edge_count = self.edges.len();
        let label_bytes: usize = self.nodes.iter().map(|(_, l)| l.len()).sum();
        
        // Temporary buffers
        let temps_size = node_count * 64 + edge_count * 128 + 4096;
        
        // Output IR
        let ir_size = crate::ir::arena::estimate_layout_arena_size(
            node_count, 
            edge_count, 
            label_bytes,
            edge_count * node_count,
        );
        
        temps_size + ir_size
    }
}
