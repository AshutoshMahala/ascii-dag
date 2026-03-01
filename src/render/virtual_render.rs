//! Virtual layout rendering for DAG visualization.
//!
//! Takes the virtual layout (with dummy nodes and assigned coordinates)
//! and renders it to an output string, including node placement and
//! connection routing between levels.

use crate::graph::DAG;
use alloc::{vec, vec::Vec};
use core::fmt::Write;

use super::ascii::{RenderBuffers, VirtualLayout};
use super::chars::V_LINE;

impl<'a> DAG<'a> {
    /// Render the virtual layout to output.
    pub(super) fn render_virtual_layout(
        &self,
        output: &mut alloc::string::String,
        layout: &VirtualLayout,
    ) {
        // Calculate canvas dimensions - compute width on-demand
        let level_widths: Vec<usize> = layout
            .levels
            .iter()
            .enumerate()
            .map(|(level_idx, level_nodes)| {
                level_nodes
                    .iter()
                    .enumerate()
                    .map(|(pos, _)| {
                        layout.x_coords[level_idx][pos] + layout.get_width(self, level_idx, pos)
                    })
                    .max()
                    .unwrap_or(0)
            })
            .collect();

        let max_canvas_width = *level_widths.iter().max().unwrap_or(&0);

        // Precompute level offsets
        let level_offsets: Vec<usize> = level_widths
            .iter()
            .map(|&w| {
                if max_canvas_width > w {
                    (max_canvas_width - w) / 2
                } else {
                    0
                }
            })
            .collect();

        // Precompute absolute screen positions for all nodes
        // For dummy nodes in a chain, we need consistent positions across levels
        let mut absolute_positions: Vec<Vec<usize>> = layout
            .levels
            .iter()
            .enumerate()
            .map(|(level_idx, level_nodes)| {
                level_nodes
                    .iter()
                    .enumerate()
                    .map(|(pos, _)| layout.x_coords[level_idx][pos] + level_offsets[level_idx])
                    .collect()
            })
            .collect();

        // Fix dummy chain positions: propagate positions down through dummy chains
        // First, establish the absolute position for the first dummy in each chain
        // Then propagate that position to all subsequent dummies in the chain
        // Track used positions at each level to avoid collisions (use 3-char spacing for readability)
        let mut used_positions: Vec<Vec<usize>> = vec![Vec::new(); layout.levels.len()];
        const DUMMY_SPACING: usize = 3; // Minimum spacing between dummy columns

        // Process level by level from top to bottom
        for current_level in 0..layout.edges_by_level.len() {
            let level_edges = &layout.edges_by_level[current_level];
            for &(from_pos, to_pos) in level_edges {
                let to_level = current_level + 1;
                if to_level >= layout.levels.len() {
                    continue;
                }

                let from_is_dummy = !layout.levels[current_level][from_pos].is_real();
                let to_is_dummy = !layout.levels[to_level][to_pos].is_real();

                if to_is_dummy {
                    if from_is_dummy {
                        // Propagate: dummy-to-dummy, use the same absolute position
                        let propagated_pos = absolute_positions[current_level][from_pos];
                        absolute_positions[to_level][to_pos] = propagated_pos;
                        // Also mark this position as used at the target level
                        if !used_positions[to_level].contains(&propagated_pos) {
                            used_positions[to_level].push(propagated_pos);
                        }
                    } else {
                        // First dummy in chain: find source center
                        let from_center = absolute_positions[current_level][from_pos]
                            + layout.get_width(self, current_level, from_pos) / 2;

                        // Find gaps between real nodes where we can place the dummy
                        // A gap is valid if the source center falls within it
                        let mut real_node_spans: Vec<(usize, usize)> = layout.levels[to_level]
                            .iter()
                            .enumerate()
                            .filter(|(_, vn)| vn.is_real())
                            .map(|(pos, _)| {
                                let start = absolute_positions[to_level][pos];
                                let end = start + layout.get_width(self, to_level, pos);
                                (start, end)
                            })
                            .collect();
                        real_node_spans.sort_by_key(|(s, _)| *s);

                        // Check if source center is inside any real node
                        let inside_real_node = real_node_spans
                            .iter()
                            .any(|(start, end)| from_center >= *start && from_center <= *end + 2);

                        let candidate_x = if inside_real_node {
                            // Find the nearest gap edge
                            // Look for gaps and find the closest one to from_center
                            let mut best_gap_pos = from_center;
                            let mut best_distance = usize::MAX;

                            // Check gap before first node
                            if let Some(&(first_start, _)) = real_node_spans.first() {
                                if first_start > 3 {
                                    let gap_pos = first_start - 2;
                                    let dist = from_center.abs_diff(gap_pos);
                                    if dist < best_distance {
                                        best_distance = dist;
                                        best_gap_pos = gap_pos;
                                    }
                                }
                            }

                            // Check gaps between nodes
                            for i in 0..real_node_spans.len().saturating_sub(1) {
                                let (_, prev_end) = real_node_spans[i];
                                let (next_start, _) = real_node_spans[i + 1];
                                if next_start > prev_end + 4 {
                                    // There's a gap - find center of gap
                                    let gap_center = (prev_end + next_start) / 2;
                                    let dist = from_center.abs_diff(gap_center);
                                    if dist < best_distance {
                                        best_distance = dist;
                                        best_gap_pos = gap_center;
                                    }
                                }
                            }

                            // Check gap after last node
                            if let Some(&(_, last_end)) = real_node_spans.last() {
                                let gap_pos = last_end + 3;
                                let dist = from_center.abs_diff(gap_pos);
                                if dist < best_distance {
                                    best_gap_pos = gap_pos;
                                }
                            }

                            best_gap_pos
                        } else {
                            // Source center is in a gap - use it directly
                            from_center
                        };

                        // Ensure this position isn't too close to any already used position
                        let mut final_x = candidate_x;
                        loop {
                            let too_close = used_positions[to_level].iter().any(|&used| {
                                let diff = final_x.abs_diff(used);
                                diff < DUMMY_SPACING
                            });
                            if !too_close {
                                break;
                            }
                            final_x += DUMMY_SPACING;
                        }

                        absolute_positions[to_level][to_pos] = final_x;
                        used_positions[to_level].push(final_x);
                    }
                }
            }
        }

        // Create reusable buffers for connection drawing
        let mut buffers = RenderBuffers::new();

        for (current_level, level_nodes) in layout.levels.iter().enumerate() {
            if level_nodes.is_empty() {
                continue;
            }

            // Create sorted render order based on absolute positions (left-to-right)
            let mut render_order: Vec<usize> = (0..level_nodes.len()).collect();
            render_order.sort_by_key(|&pos| absolute_positions[current_level][pos]);

            // Render nodes at their precomputed absolute positions (in sorted order)
            let mut current_col = 0;
            for pos in render_order {
                let vnode = &level_nodes[pos];
                let node_x = absolute_positions[current_level][pos];

                // Add spacing to reach this node's position (batch operation)
                if node_x > current_col {
                    output.extend(core::iter::repeat_n(' ', node_x - current_col));
                    current_col = node_x;
                }

                if vnode.is_real() {
                    let (id, label) = self.nodes[vnode.index()];
                    self.write_node(output, id, label);
                    current_col += layout.get_width(self, current_level, pos);
                } else {
                    // Dummy nodes show as vertical line to indicate skip-level edge passing through
                    output.push(V_LINE);
                    current_col += 1;
                }
            }
            writeln!(output).ok();

            // Draw connections if not last level
            if current_level < layout.levels.len() - 1 {
                self.draw_virtual_connections_absolute(
                    output,
                    layout,
                    current_level,
                    &absolute_positions,
                    &mut buffers,
                );
            }
        }
    }
}
