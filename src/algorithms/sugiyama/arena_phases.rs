//! Phase helper methods for arena-based layout computation.
//!
//! These are `impl DAG` methods extracted from the main arena layout module:
//! level calculation, virtual level construction, coordinate assignment,
//! dummy position building, and crossing reduction.

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
    pub(super) fn reduce_crossings_arena(
        &self,
        _vlevel_offsets: &mut [Idx],
        _vnode_data: &mut [Idx],
        _node_levels: &[Idx],
        _max_level: usize,
        _medians: &mut [(Idx, u32)],
        _positions: &mut [Idx],
    ) {
        // Crossing reduction not yet implemented for arena;
        // nodes remain in insertion order.
    }
}
