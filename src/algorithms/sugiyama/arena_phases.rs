//! Phase helper methods for arena-based layout computation.
//!
//! These are `impl DAG` methods extracted from the main arena layout module:
//! level calculation, virtual level construction, coordinate assignment,
//! dummy position building, and crossing reduction.

use crate::algorithms::sugiyama::crossing::CrossingReducer;
use crate::graph::DAG;

// Import configurable index types
#[cfg(feature = "arena")]
use super::idx::{Coord, Idx, MAX_LEVELS};

// Fallback types when arena feature not enabled (for compilation)
#[cfg(not(feature = "arena"))]
type Idx = u32;
#[cfg(not(feature = "arena"))]
type Coord = u16;
#[cfg(not(feature = "arena"))]
const MAX_LEVELS: usize = usize::MAX;

impl<'a> DAG<'a> {
    /// Calculate levels using arena-allocated buffer.
    pub(super) fn calculate_levels_arena(&self, levels: &mut [Idx], edge_indices: &[(Idx, Idx)]) -> usize {
        for l in levels.iter_mut() {
            *l = 0;
        }

        let mut changed = true;
        while changed {
            changed = false;
            for &(from_idx, to_idx) in edge_indices {
                if from_idx != Idx::MAX && to_idx != Idx::MAX {
                    let from = from_idx as usize;
                    let to = to_idx as usize;

                    let new_level = levels[from] as usize + 1;
                    if new_level > levels[to] as usize {
                        // Safe cast - we cap at MAX_LEVELS which fits in Idx
                        levels[to] = new_level.min(MAX_LEVELS) as Idx;
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
    pub(super) fn build_virtual_levels_arena(
        &self,
        node_levels: &[Idx],
        edge_indices: &[(Idx, Idx)],
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
        for &(from_idx, to_idx) in edge_indices {
            if from_idx != Idx::MAX && to_idx != Idx::MAX {
                let from_level = node_levels[from_idx as usize] as usize;
                let to_level = node_levels[to_idx as usize] as usize;

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
        for (edge_idx, &(from_idx, to_idx)) in edge_indices.iter().enumerate() {
            if from_idx != Idx::MAX && to_idx != Idx::MAX {
                let from_level = node_levels[from_idx as usize] as usize;
                let to_level = node_levels[to_idx as usize] as usize;

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

    /// Assign x-coordinates to virtual nodes.
    /// Returns the total width of the graph.
    pub(super) fn assign_x_coords_arena(
        &self,
        vlevel_offsets: &[Idx],
        vnode_data: &[Idx],
        x_coords: &mut [Coord],
        widths: &mut [Coord],
        max_level: usize,
    ) -> usize {
        let mut max_width = 0;
        let max_pos = x_coords.len();
        let max_vnode_idx = vnode_data.len() / 2;

        for level in 0..=max_level {
            let start = vlevel_offsets[level] as usize;
            let end = (vlevel_offsets[level + 1] as usize)
                .min(max_pos)
                .min(max_vnode_idx);
            let mut x = 0;

            for pos in start..end {
                // Bounds check
                if pos * 2 + 1 >= vnode_data.len() {
                    break;
                }
                let vnode_type = vnode_data[pos * 2];
                let vnode_idx = vnode_data[pos * 2 + 1] as usize;

                let width = if vnode_type == 0 {
                    // Real node: use node_widths[vnode_idx]
                    self.get_node_width(vnode_idx)
                } else {
                    // Dummy node - use width 3
                    3
                };

                if pos < x_coords.len() {
                    x_coords[pos] = x as Coord;
                    widths[pos] = width as Coord;
                }
                x += width + 3;
            }

            if end > start && end - 1 < x_coords.len() {
                let last_x = x_coords[end - 1] as usize;
                let last_width = widths[end - 1] as usize;
                max_width = max_width.max(last_x + last_width);
            }
        }
        max_width
    }

    /// Build real node coordinates from virtual level positions.
    pub(super) fn build_real_coords_arena(
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

        for level in 0..=max_level {
            let start = vlevel_offsets[level] as usize;
            let end = (vlevel_offsets[level + 1] as usize)
                .min(max_pos)
                .min(max_vnode_idx);

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
                if pos * 2 + 1 >= vnode_data.len() {
                    break;
                }
                let vnode_type = vnode_data[pos * 2];
                let vnode_idx = vnode_data[pos * 2 + 1] as usize;

                if vnode_type == 0 {
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
    pub(super) fn build_dummy_positions_arena(
        &self,
        vlevel_offsets: &[Idx],
        vnode_data: &[Idx],
        x_coords: &[Coord],
        widths: &[Coord],
        dummy_offsets: &mut [Idx],
        dummy_data: &mut [(Idx, Coord)],
        _edge_indices: &[(Idx, Idx)],
        max_level: usize,
        max_width: usize,
    ) {
        let edge_count = self.edges.len();

        // 1. Clear offsets (used as counters initially)
        dummy_offsets.fill(0);

        let max_vnode_idx = vnode_data.len() / 2;
        let vnode_limit = x_coords.len().min(widths.len()).min(max_vnode_idx);

        // 2. Pass 1: Count extra dummy nodes per edge
        // Iterate levels and virtual nodes
        for level in 0..=max_level {
            // Safe casting
            let start = vlevel_offsets[level] as usize;
            let end = (vlevel_offsets[level + 1] as usize).min(vnode_limit);

            for pos in start..end {
                // Bounds check
                if pos * 2 + 1 >= vnode_data.len() {
                    break;
                }
                let vnode_type = vnode_data[pos * 2];
                if vnode_type == 1 {
                    // Dummy node
                    let edge_idx = vnode_data[pos * 2 + 1] as usize;
                    if edge_idx < edge_count {
                        dummy_offsets[edge_idx] += 1;
                    }
                }
            }
        }

        // 3. Convert counts to Prefix Sums (Start Indices)
        let mut current = 0;
        for count in dummy_offsets.iter_mut().take(edge_count) {
            let c = *count;
            *count = current;
            current += c;
            // Clamp to prevent OOB
            if (current as usize) > dummy_data.len() {
                current = dummy_data.len() as Idx;
            }
        }
        dummy_offsets[edge_count] = current;

        // 4. Pass 2: Fill dummy_data
        // We reuse dummy_offsets as current write pointers.
        // We will fix them up later.

        for level in 0..=max_level {
            let start = vlevel_offsets[level] as usize;
            let end = (vlevel_offsets[level + 1] as usize).min(vnode_limit);

            // Calculate centering offset for this level
            let level_width = if end > start {
                let last_idx = end - 1;
                x_coords[last_idx] as usize + widths[last_idx] as usize
            } else {
                0
            };

            let offset = if max_width > level_width {
                (max_width - level_width) / 2
            } else {
                0
            };

            for pos in start..end {
                let vnode_type = vnode_data[pos * 2];
                if vnode_type == 1 {
                    let edge_idx = vnode_data[pos * 2 + 1] as usize;

                    if edge_idx < edge_count {
                        let base_x = x_coords[pos] as usize + offset;
                        // Add offset based on edge index to separate overlapping edges visually
                        let edge_shift = edge_idx % 4;
                        let x = base_x + edge_shift;

                        // Write to buffer
                        let write_pos = dummy_offsets[edge_idx] as usize;
                        if write_pos < dummy_data.len() {
                            dummy_data[write_pos] = (level as Idx, x as Coord);
                            dummy_offsets[edge_idx] += 1;
                        }
                    }
                }
            }
        }

        // 5. Restore dummy_offsets to point to Start Indices
        // Currently each entry points to the End Index (Start + Count).
        // This is exactly equal to the Start Index of the NEXT edge.
        // So offset[i] now holds Start[i+1].
        // We shift right by 1.
        for i in (0..edge_count).rev() {
            dummy_offsets[i + 1] = dummy_offsets[i];
        }
        dummy_offsets[0] = 0;
    }

    /// Crossing reduction on arena-allocated virtual levels.
    ///
    /// Dispatches through the DAG's [`CrossingReducer`] pipeline,
    /// operating on the flat `vnode_data` array with `vlevel_offsets`
    /// boundaries.  The `medians` and `positions` slices are pre-allocated
    /// scratch buffers.
    pub(super) fn reduce_crossings_arena(
        &self,
        vlevel_offsets: &[Idx],
        vnode_data: &mut [Idx],
        _node_levels: &[Idx],
        max_level: usize,
        medians: &mut [(Idx, u32)],
        positions: &mut [Idx],
        edge_indices: &[(Idx, Idx)],
    ) {
        for reducer in &self.crossing_pipeline {
            match reducer {
                CrossingReducer::Median(passes) => {
                    for _ in 0..*passes {
                        // Top-down pass
                        for level in 1..=max_level {
                            self.median_reorder_arena_level(
                                vlevel_offsets,
                                vnode_data,
                                edge_indices,
                                level,
                                level - 1,
                                true,
                                medians,
                                positions,
                            );
                        }
                        // Bottom-up pass
                        for level in (0..max_level).rev() {
                            self.median_reorder_arena_level(
                                vlevel_offsets,
                                vnode_data,
                                edge_indices,
                                level,
                                level + 1,
                                false,
                                medians,
                                positions,
                            );
                        }
                    }
                }
                CrossingReducer::AdjacentExchange(passes) => {
                    for _ in 0..*passes {
                        // Top-down pass
                        for level in 1..=max_level {
                            self.adjacent_exchange_arena_level(
                                vlevel_offsets,
                                vnode_data,
                                edge_indices,
                                level,
                                level - 1,
                                true,
                                positions,
                            );
                        }
                        // Bottom-up pass
                        for level in (0..max_level).rev() {
                            self.adjacent_exchange_arena_level(
                                vlevel_offsets,
                                vnode_data,
                                edge_indices,
                                level,
                                level + 1,
                                false,
                                positions,
                            );
                        }
                    }
                }
            }
        }
    }

    /// Median-heuristic reorder of one level against its adjacent reference level.
    ///
    /// Builds a position map for the adjacent level, computes the median
    /// neighbour position for each node on `level`, sorts by median, and
    /// rewrites `vnode_data` in-place.
    fn median_reorder_arena_level(
        &self,
        vlevel_offsets: &[Idx],
        vnode_data: &mut [Idx],
        edge_indices: &[(Idx, Idx)],
        level: usize,
        adj_level: usize,
        use_parents: bool,
        medians: &mut [(Idx, u32)],
        positions: &mut [Idx],
    ) {
        let cur_start = vlevel_offsets[level] as usize;
        let cur_end = vlevel_offsets[level + 1] as usize;
        let count = cur_end - cur_start;
        if count < 2 {
            return;
        }

        let adj_start = vlevel_offsets[adj_level] as usize;
        let adj_end = vlevel_offsets[adj_level + 1] as usize;

        // Build position map for real nodes in the adjacent level.
        // positions[node_idx] = position within adjacent level (or Idx::MAX if absent).
        for p in positions.iter_mut() {
            *p = Idx::MAX;
        }
        for adj_pos in adj_start..adj_end {
            if adj_pos * 2 + 1 >= vnode_data.len() {
                break;
            }
            if vnode_data[adj_pos * 2] == 0 {
                // Real node
                let node_idx = vnode_data[adj_pos * 2 + 1] as usize;
                if node_idx < positions.len() {
                    positions[node_idx] = (adj_pos - adj_start) as Idx;
                }
            }
        }

        // Compute median for each node on the current level.
        // medians[i] = (i as Idx, median_fixed_point)
        // We encode the median as u32 with 10-bit fractional part (×1024).
        for i in 0..count {
            let pos = cur_start + i;
            if pos * 2 + 1 >= vnode_data.len() {
                medians[i] = (i as Idx, (i as u32) << 10);
                continue;
            }
            let vtype = vnode_data[pos * 2];
            let vidx = vnode_data[pos * 2 + 1] as usize;

            // Collect neighbour positions in the adjacent level.
            // We gather inline (up to ~8 neighbours for most graphs).
            let mut neigh: [usize; 16] = [0; 16];
            let mut neigh_count: usize = 0;

            if vtype == 0 {
                // Real node — lookup adjacency lists
                let neighbours = if use_parents {
                    self.get_parents_indices(vidx)
                } else {
                    self.get_children_indices(vidx)
                };
                for &n_idx in neighbours {
                    if n_idx < positions.len() && positions[n_idx] != Idx::MAX {
                        if neigh_count < 16 {
                            neigh[neigh_count] = positions[n_idx] as usize;
                            neigh_count += 1;
                        }
                    }
                }
            } else {
                // Dummy node (edge_idx = vidx)
                if vidx < edge_indices.len() {
                    let (from_idx, to_idx) = edge_indices[vidx];

                    // Check real endpoint
                    let endpoint = if use_parents {
                        from_idx as usize
                    } else {
                        to_idx as usize
                    };
                    if endpoint < positions.len() && positions[endpoint] != Idx::MAX {
                        neigh[neigh_count] = positions[endpoint] as usize;
                        neigh_count += 1;
                    }

                    // Check for same-edge dummy in adjacent level
                    for adj_pos in adj_start..adj_end {
                        if adj_pos * 2 + 1 >= vnode_data.len() {
                            break;
                        }
                        if vnode_data[adj_pos * 2] == 1
                            && vnode_data[adj_pos * 2 + 1] as usize == vidx
                        {
                            if neigh_count < 16 {
                                neigh[neigh_count] = (adj_pos - adj_start) as usize;
                                neigh_count += 1;
                            }
                            break; // only one dummy per edge per level
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
                    // (sum / 2) in fixed-point: (sum × 1024) / 2 = sum × 512
                    (sum as u32) * 512
                }
            };

            medians[i] = (i as Idx, median_fixed);
        }

        // Sort by median (stable-ish via key comparison).
        medians[..count].sort_by_key(|m| m.1);

        // Gather sorted vnode_data into medians buffer by overwriting in-place.
        // After this loop, medians[j] = (vtype, vidx_as_u32) in the new order.
        for j in 0..count {
            let orig_pos = medians[j].0 as usize;
            let src = cur_start + orig_pos;
            let vtype = vnode_data[src * 2];
            let vidx = vnode_data[src * 2 + 1] as u32;
            medians[j] = (vtype, vidx);
        }

        // Write sorted data back to vnode_data.
        for j in 0..count {
            let dst = cur_start + j;
            vnode_data[dst * 2] = medians[j].0;
            vnode_data[dst * 2 + 1] = medians[j].1 as Idx;
        }
    }

    /// Adjacent exchange on one arena level: swap adjacent pairs if it reduces crossings.
    fn adjacent_exchange_arena_level(
        &self,
        vlevel_offsets: &[Idx],
        vnode_data: &mut [Idx],
        edge_indices: &[(Idx, Idx)],
        level: usize,
        adj_level: usize,
        use_parents: bool,
        positions: &mut [Idx],
    ) {
        let cur_start = vlevel_offsets[level] as usize;
        let cur_end = vlevel_offsets[level + 1] as usize;
        let count = cur_end - cur_start;
        if count < 2 {
            return;
        }

        let adj_start = vlevel_offsets[adj_level] as usize;
        let adj_end = vlevel_offsets[adj_level + 1] as usize;

        // Build position map for real nodes in the adjacent level.
        for p in positions.iter_mut() {
            *p = Idx::MAX;
        }
        for adj_pos in adj_start..adj_end {
            if adj_pos * 2 + 1 >= vnode_data.len() {
                break;
            }
            if vnode_data[adj_pos * 2] == 0 {
                let node_idx = vnode_data[adj_pos * 2 + 1] as usize;
                if node_idx < positions.len() {
                    positions[node_idx] = (adj_pos - adj_start) as Idx;
                }
            }
        }

        // Inline buffers for neighbour positions
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

            // Gather u neighbours
            Self::gather_arena_neighbours(
                &self.children,
                &self.parents,
                vnode_data,
                edge_indices,
                positions,
                u_pos,
                adj_start,
                adj_end,
                use_parents,
                &mut u_neigh,
                &mut u_count,
            );

            // Gather v neighbours
            Self::gather_arena_neighbours(
                &self.children,
                &self.parents,
                vnode_data,
                edge_indices,
                positions,
                v_pos,
                adj_start,
                adj_end,
                use_parents,
                &mut v_neigh,
                &mut v_count,
            );

            // Count crossings
            let mut cross_uv: usize = 0;
            let mut cross_vu: usize = 0;
            for &a in &u_neigh[..u_count] {
                for &b in &v_neigh[..v_count] {
                    if a > b {
                        cross_uv += 1;
                    } else if a < b {
                        cross_vu += 1;
                    }
                }
            }

            if cross_vu < cross_uv {
                // Swap the two vnodes in vnode_data
                let u_type = vnode_data[u_pos * 2];
                let u_idx = vnode_data[u_pos * 2 + 1];
                vnode_data[u_pos * 2] = vnode_data[v_pos * 2];
                vnode_data[u_pos * 2 + 1] = vnode_data[v_pos * 2 + 1];
                vnode_data[v_pos * 2] = u_type;
                vnode_data[v_pos * 2 + 1] = u_idx;
            }
        }
    }

    /// Gather neighbour positions for a single vnode in the adjacent level.
    #[inline]
    #[allow(clippy::too_many_arguments)]
    fn gather_arena_neighbours(
        children: &[Vec<usize>],
        parents: &[Vec<usize>],
        vnode_data: &[Idx],
        edge_indices: &[(Idx, Idx)],
        positions: &[Idx],
        pos: usize,
        adj_start: usize,
        adj_end: usize,
        use_parents: bool,
        out: &mut [usize; 16],
        out_count: &mut usize,
    ) {
        *out_count = 0;
        let vtype = vnode_data[pos * 2];
        let vidx = vnode_data[pos * 2 + 1] as usize;

        if vtype == 0 {
            // Real node
            let neighbours = if use_parents {
                &parents[vidx]
            } else {
                &children[vidx]
            };
            for &n_idx in neighbours {
                if n_idx < positions.len()
                    && positions[n_idx] != Idx::MAX
                    && *out_count < 16
                {
                    out[*out_count] = positions[n_idx] as usize;
                    *out_count += 1;
                }
            }
        } else if vidx < edge_indices.len() {
            // Dummy node
            let (from_idx, to_idx) = edge_indices[vidx];
            let endpoint = if use_parents {
                from_idx as usize
            } else {
                to_idx as usize
            };
            if endpoint < positions.len() && positions[endpoint] != Idx::MAX && *out_count < 16 {
                out[*out_count] = positions[endpoint] as usize;
                *out_count += 1;
            }
            // Same-edge dummy in adjacent level
            for adj_pos in adj_start..adj_end {
                if adj_pos * 2 + 1 >= vnode_data.len() {
                    break;
                }
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
