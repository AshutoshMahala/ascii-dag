//! Connection drawing for virtual layout rendering.
//!
//! Handles all edge connection patterns between adjacent levels:
//! - Mixed convergence/divergence with crossing detection
//! - Pure convergence (multiple sources to one target)
//! - Pure divergence (one source to multiple targets)
//! - Simple 1-to-1 connections

use crate::graph::Graph;
use alloc::{vec, vec::Vec};
use core::fmt::Write;

use super::ascii::{RenderBuffers, VirtualLayout};
use super::chars::{
    ARROW_DOWN, CORNER_DL, CORNER_DR, CORNER_UL, CORNER_UR, CROSS, H_LINE, TEE_DOWN, TEE_UP,
    V_LINE,
};

impl<'a> Graph<'a> {
    /// Draw connections between adjacent levels using absolute positions.
    pub(super) fn draw_virtual_connections_absolute(
        &self,
        output: &mut alloc::string::String,
        layout: &VirtualLayout,
        current_level: usize,
        absolute_positions: &[Vec<usize>],
        buffers: &mut RenderBuffers,
    ) {
        let next_level = current_level + 1;

        // O(1) lookup: edges are pre-grouped by source level
        let level_edges = &layout.edges_by_level[current_level];

        if level_edges.is_empty() {
            return;
        }

        // Calculate center positions for connections using absolute positions
        let mut connections: Vec<(usize, usize, bool, bool)> =
            Vec::with_capacity(level_edges.len());

        for &(from_pos, to_pos) in level_edges {
            let from_is_dummy = !layout.levels[current_level][from_pos].is_real();
            let to_is_dummy = !layout.levels[next_level][to_pos].is_real();

            let from_x = absolute_positions[current_level][from_pos]
                + layout.get_width(self, current_level, from_pos) / 2;

            let to_x = absolute_positions[next_level][to_pos]
                + layout.get_width(self, next_level, to_pos) / 2;

            connections.push((from_x, to_x, from_is_dummy, to_is_dummy));
        }

        // Group by target for convergence detection
        let mut target_groups: Vec<(usize, Vec<(usize, bool)>)> = Vec::new();
        for &(from_x, to_x, from_is_dummy, _) in &connections {
            match target_groups.binary_search_by_key(&to_x, |(k, _)| *k) {
                Ok(idx) => target_groups[idx].1.push((from_x, from_is_dummy)),
                Err(idx) => target_groups.insert(idx, (to_x, vec![(from_x, from_is_dummy)])),
            }
        }

        // Group by source for divergence detection
        let mut source_groups: Vec<(usize, Vec<(usize, bool)>)> = Vec::new();
        for &(from_x, to_x, _, to_is_dummy) in &connections {
            match source_groups.binary_search_by_key(&from_x, |(k, _)| *k) {
                Ok(idx) => source_groups[idx].1.push((to_x, to_is_dummy)),
                Err(idx) => source_groups.insert(idx, (from_x, vec![(to_x, to_is_dummy)])),
            }
        }

        let has_convergence = target_groups.iter().any(|(_, v)| v.len() > 1);
        let has_divergence = source_groups.iter().any(|(_, v)| v.len() > 1);

        let max_pos = connections
            .iter()
            .flat_map(|(f, t, _, _)| [*f, *t])
            .max()
            .unwrap_or(0);

        // Draw based on pattern - now with proper handling of mixed cases
        if has_convergence && has_divergence {
            // Mixed case: draw with proper crossing handling
            self.draw_mixed_connections(output, &connections, max_pos, buffers);
        } else if has_convergence {
            self.draw_convergence_connections(
                output,
                &connections,
                &target_groups,
                max_pos,
                buffers,
            );
        } else if has_divergence {
            self.draw_divergence_connections(
                output,
                &connections,
                &source_groups,
                max_pos,
                buffers,
            );
        } else {
            // Simple 1-to-1 connections
            self.draw_simple_connections(output, &connections, max_pos, buffers);
        }
    }

    /// Draw mixed convergence and divergence (the previously broken case).
    /// Optimized with O(1) position lookups using reusable boolean arrays.
    #[allow(clippy::needless_range_loop)]
    pub(super) fn draw_mixed_connections(
        &self,
        output: &mut alloc::string::String,
        connections: &[(usize, usize, bool, bool)],
        max_pos: usize,
        buffers: &mut RenderBuffers,
    ) {
        // Use reusable boolean arrays for O(1) lookups
        buffers.prepare_bitmaps(max_pos + 1);

        for &(from_x, to_x, _, to_is_dummy) in connections {
            buffers.is_source.set(from_x);
            buffers.all_targets.set(to_x); // All targets (for routing)
            if !to_is_dummy {
                buffers.is_target.set(to_x); // Only real targets (for arrows)
            }
        }

        // Classify connections
        let mut straight_down: Vec<usize> = Vec::new(); // from_x == to_x
        let mut going_right: Vec<(usize, usize)> = Vec::new(); // from_x < to_x
        let mut going_left: Vec<(usize, usize)> = Vec::new(); // from_x > to_x

        for &(from_x, to_x, _, _) in connections {
            if from_x == to_x {
                straight_down.push(from_x);
            } else if from_x < to_x {
                going_right.push((from_x, to_x));
            } else {
                going_left.push((from_x, to_x));
            }
        }

        // Check if we have true crossings (both going_left and going_right with overlapping spans)
        let has_crossings = !going_right.is_empty() && !going_left.is_empty();

        // Line 1: Vertical drops from all sources
        for i in 0..=max_pos {
            output.push(if buffers.is_source.get(i) {
                V_LINE
            } else {
                ' '
            });
        }
        writeln!(output).ok();

        if has_crossings {
            // Complex case: multiple sources converging to multiple targets with crossings
            // Lines come DOWN from sources, turn horizontal, then continue down to targets
            //
            // Source positions: lines come FROM ABOVE → use ┴ (TEE_UP)
            // Target positions: lines go DOWN TO → use ┬ (TEE_DOWN)
            // Both source AND target: use ┼ (CROSS)

            // Find the overall span of the horizontal routing line
            let all_positions: Vec<usize> = going_right
                .iter()
                .flat_map(|(f, t)| [*f, *t])
                .chain(going_left.iter().flat_map(|(f, t)| [*f, *t]))
                .chain(straight_down.iter().copied())
                .collect();

            let min_x = *all_positions.iter().min().unwrap_or(&0);
            let max_x = *all_positions.iter().max().unwrap_or(&0);

            // Reuse line buffer
            buffers.prepare_chars(max_pos + 1);

            // First, draw the horizontal line across the entire span
            for i in min_x..=max_x {
                buffers.line_chars[i] = H_LINE;
            }

            // Mark source positions (where lines come down from above) with ┴
            for i in min_x..=max_x {
                if buffers.is_source.get(i) {
                    buffers.line_chars[i] = TEE_UP; // ┴ - line comes from above
                }
            }

            // Mark target positions (where lines go down to) with ┬
            // If already ┴ (source), upgrade to ┼ (cross)
            // Use all_targets for routing (includes dummy nodes for pass-through)
            for i in min_x..=max_x {
                if buffers.all_targets.get(i) {
                    buffers.line_chars[i] = match buffers.line_chars[i] {
                        TEE_UP => CROSS,    // Both source and target → cross
                        H_LINE => TEE_DOWN, // Only target → ┬
                        _ => buffers.line_chars[i],
                    };
                }
            }

            // Fix the endpoints based on whether they're source or target
            // Left endpoint
            if min_x < buffers.line_chars.len() {
                buffers.line_chars[min_x] =
                    if buffers.is_source.get(min_x) && buffers.all_targets.get(min_x) {
                        CROSS // Both
                    } else if buffers.is_source.get(min_x) {
                        CORNER_DR // └ - source only (line from above)
                    } else if buffers.all_targets.get(min_x) {
                        CORNER_UR // ┌ - target only (line goes down)
                    } else {
                        buffers.line_chars[min_x]
                    };
            }
            // Right endpoint
            if max_x < buffers.line_chars.len() {
                buffers.line_chars[max_x] =
                    if buffers.is_source.get(max_x) && buffers.all_targets.get(max_x) {
                        CROSS // Both
                    } else if buffers.is_source.get(max_x) {
                        CORNER_DL // ┘ - source only (line from above)
                    } else if buffers.all_targets.get(max_x) {
                        CORNER_UL // ┐ - target only (line goes down)
                    } else {
                        buffers.line_chars[max_x]
                    };
            }

            for ch in &buffers.line_chars {
                output.push(*ch);
            }
            writeln!(output).ok();

            // Line 2b: Vertical continuation - for all targets that continue down
            buffers.prepare_aux(max_pos + 1);
            for &x in &straight_down {
                buffers.bitmap_aux.set(x);
            }
            for i in 0..=max_pos {
                output.push(if buffers.all_targets.get(i) || buffers.bitmap_aux.get(i) {
                    V_LINE
                } else {
                    ' '
                });
            }
            writeln!(output).ok();
        } else {
            // Simpler case: single routing line - reuse line buffer
            buffers.prepare_chars(max_pos + 1);

            for &(from_x, to_x) in &going_right {
                for i in from_x..=to_x {
                    if i == from_x {
                        buffers.line_chars[i] = CORNER_DR; // └ - coming from above, going right
                    } else if i == to_x {
                        buffers.line_chars[i] = match buffers.line_chars[i] {
                            CORNER_DR => TEE_UP,
                            _ => CORNER_UL, // ┐ - coming from left, going down (not ┘)
                        };
                    } else if buffers.line_chars[i] == ' ' {
                        buffers.line_chars[i] = H_LINE;
                    }
                }
            }

            for &(from_x, to_x) in &going_left {
                for i in to_x..=from_x {
                    if i == from_x {
                        buffers.line_chars[i] = match buffers.line_chars[i] {
                            CORNER_DR => TEE_UP,
                            CORNER_UL => TEE_UP,
                            _ => CORNER_DL, // ┘ - coming from above, going left
                        };
                    } else if i == to_x {
                        buffers.line_chars[i] = match buffers.line_chars[i] {
                            CORNER_DL => TEE_UP,
                            H_LINE => TEE_UP,
                            _ => CORNER_UR, // ┌ - coming from right, going down (not └)
                        };
                    } else if buffers.line_chars[i] == ' ' {
                        buffers.line_chars[i] = H_LINE;
                    }
                }
            }

            for &x in &straight_down {
                if buffers.line_chars[x] == ' ' {
                    buffers.line_chars[x] = V_LINE;
                } else if buffers.line_chars[x] == H_LINE {
                    buffers.line_chars[x] = TEE_UP;
                }
            }

            for ch in &buffers.line_chars {
                output.push(*ch);
            }
            writeln!(output).ok();
        }

        // Final line: Arrows at real targets, vertical continuation for dummy targets
        for i in 0..=max_pos {
            if buffers.is_target.get(i) {
                output.push(ARROW_DOWN);
            } else if buffers.all_targets.get(i) {
                // Dummy target - show vertical continuation (pass-through)
                output.push(V_LINE);
            } else {
                output.push(' ');
            }
        }
        writeln!(output).ok();
    }

    /// Draw pure convergence pattern.
    /// Extends horizontal bracket to include all sources (even pass-through) for cleaner visuals.
    /// Optimized with O(1) position lookups using reusable buffers.
    pub(super) fn draw_convergence_connections(
        &self,
        output: &mut alloc::string::String,
        connections: &[(usize, usize, bool, bool)],
        target_groups: &[(usize, Vec<(usize, bool)>)],
        max_pos: usize,
        buffers: &mut RenderBuffers,
    ) {
        // Prepare reusable bitmaps
        buffers.prepare_bitmaps(max_pos + 1);
        buffers.prepare_aux(max_pos + 1);

        // Build source bitmap and target bitmaps from connections
        for &(from_x, to_x, _, to_is_dummy) in connections {
            buffers.is_source.set(from_x);
            buffers.all_targets.set(to_x); // All targets (for routing)
            if !to_is_dummy {
                buffers.is_target.set(to_x); // Only real targets (for arrows)
            }
        }

        // Identify 1-to-1 connections (targets with only 1 source) - these are "pass-through"
        for (_, sources) in target_groups.iter().filter(|(_, s)| s.len() == 1) {
            buffers.bitmap_aux.set(sources[0].0); // is_pass_through_src
        }

        // Collect all source positions and convergence source positions
        let all_source_xs: Vec<usize> = connections.iter().map(|(x, _, _, _)| *x).collect();
        let convergence_source_xs: Vec<usize> = target_groups
            .iter()
            .filter(|(_, s)| s.len() > 1)
            .flat_map(|(_, sources)| sources.iter().map(|(x, _)| *x))
            .collect();

        // Line 1: Vertical drops
        for i in 0..=max_pos {
            output.push(if buffers.is_source.get(i) {
                V_LINE
            } else {
                ' '
            });
        }
        writeln!(output).ok();

        // Line 2: Horizontal convergence - extend bracket to cover ALL sources
        // Find the span that covers everything
        let global_min = *all_source_xs.iter().min().unwrap_or(&0);
        let global_max = *all_source_xs.iter().max().unwrap_or(&0);

        for i in 0..=max_pos {
            let mut ch = ' ';

            if i >= global_min && i <= global_max {
                // We're inside the global source span
                let is_convergence_src = convergence_source_xs.contains(&i);
                let is_pass_through = buffers.bitmap_aux.get(i);

                if i == global_min {
                    // Left endpoint
                    if is_convergence_src || is_pass_through {
                        ch = CORNER_DR; // └
                    } else {
                        ch = H_LINE;
                    }
                } else if i == global_max {
                    // Right endpoint
                    if is_convergence_src || is_pass_through {
                        ch = CORNER_DL; // ┘
                    } else {
                        ch = H_LINE;
                    }
                } else if is_convergence_src {
                    ch = TEE_UP; // ┴
                } else if is_pass_through {
                    ch = TEE_UP; // ┴ - pass-through joins the line
                } else {
                    ch = H_LINE;
                }
            }

            output.push(ch);
        }
        writeln!(output).ok();

        // Line 3: Arrows for real targets, vertical continuation for dummy targets
        for i in 0..=max_pos {
            if buffers.is_target.get(i) {
                output.push(ARROW_DOWN);
            } else if buffers.all_targets.get(i) {
                // Dummy target - show vertical continuation (pass-through)
                output.push(V_LINE);
            } else {
                output.push(' ');
            }
        }
        writeln!(output).ok();
    }

    /// Draw pure divergence pattern.
    /// Uses top corners (┌, ┐) because lines go DOWN from the horizontal routing line.
    /// The horizontal bracket spans from source to all targets (not just between targets).
    /// Optimized with O(1) position lookups using reusable buffers.
    pub(super) fn draw_divergence_connections(
        &self,
        output: &mut alloc::string::String,
        connections: &[(usize, usize, bool, bool)],
        source_groups: &[(usize, Vec<(usize, bool)>)],
        max_pos: usize,
        buffers: &mut RenderBuffers,
    ) {
        // Prepare reusable bitmaps
        buffers.prepare_bitmaps(max_pos + 1);

        // Build source bitmap and target bitmaps from connections
        for &(from_x, to_x, _, to_is_dummy) in connections {
            buffers.is_source.set(from_x);
            buffers.all_targets.set(to_x); // All targets (for routing)
            if !to_is_dummy {
                buffers.is_target.set(to_x); // Only real targets (for arrows)
            }
        }

        // Line 1: Vertical from sources
        for i in 0..=max_pos {
            output.push(if buffers.is_source.get(i) {
                V_LINE
            } else {
                ' '
            });
        }
        writeln!(output).ok();

        // Line 2: Horizontal divergence - bracket spans from source through all targets
        // The span includes the source position so the vertical line connects properly
        for i in 0..=max_pos {
            let mut ch = ' ';
            for &(source_x, ref targets) in source_groups.iter() {
                if targets.len() <= 1 {
                    continue;
                }
                let target_xs: Vec<usize> = targets.iter().map(|(x, _)| *x).collect();
                // Span includes source AND all targets
                let min_span = *target_xs.iter().min().unwrap().min(&source_x);
                let max_span = *target_xs.iter().max().unwrap().max(&source_x);

                if i == min_span {
                    if buffers.is_source.get(i) {
                        // Source at left edge - line comes from above and goes right
                        ch = if target_xs.contains(&i) {
                            TEE_DOWN
                        } else {
                            CORNER_DR
                        }; // └ or ┬
                    } else {
                        ch = CORNER_UR; // ┌ - target at left, line goes down
                    }
                } else if i == max_span {
                    if buffers.is_source.get(i) {
                        // Source at right edge - line comes from above and goes left
                        ch = if target_xs.contains(&i) {
                            TEE_DOWN
                        } else {
                            CORNER_DL
                        }; // ┘ or ┬
                    } else {
                        ch = CORNER_UL; // ┐ - target at right, line goes down
                    }
                } else if i > min_span && i < max_span {
                    if buffers.is_source.get(i) {
                        // Source in middle - line from above joins horizontal
                        ch = if target_xs.contains(&i) {
                            CROSS
                        } else {
                            TEE_UP
                        }; // ┼ or ┴
                    } else if target_xs.contains(&i) {
                        ch = TEE_DOWN; // ┬ - target in middle
                    } else if ch == ' ' {
                        ch = H_LINE;
                    }
                }
            }
            output.push(ch);
        }
        writeln!(output).ok();

        // Line 3: Arrows for real targets, vertical continuation for dummy targets
        for i in 0..=max_pos {
            if buffers.is_target.get(i) {
                output.push(ARROW_DOWN);
            } else if buffers.all_targets.get(i) {
                // Dummy target - show vertical continuation (pass-through)
                output.push(V_LINE);
            } else {
                output.push(' ');
            }
        }
        writeln!(output).ok();
    }

    /// Draw simple 1-to-1 connections.
    /// Optimized with O(1) position lookups using reusable buffers.
    pub(super) fn draw_simple_connections(
        &self,
        output: &mut alloc::string::String,
        connections: &[(usize, usize, bool, bool)],
        max_pos: usize,
        buffers: &mut RenderBuffers,
    ) {
        // Check if any connection has significantly different from_x and to_x (needs routing)
        // Small offsets (1-2 chars) can be handled with straight lines for cleaner output
        const SNAP_THRESHOLD: usize = 2;
        let needs_routing = connections.iter().any(|(f, t, _, _)| {
            let diff = if *f > *t { f - t } else { t - f };
            diff > SNAP_THRESHOLD
        });

        if needs_routing {
            // Fall back to mixed connections handler for proper routing
            self.draw_mixed_connections(output, connections, max_pos, buffers);
            return;
        }

        // Prepare reusable bitmaps
        buffers.prepare_bitmaps(max_pos + 1);

        // Track sources and targets that go to real nodes (not dummy pass-through)
        // For small offsets, use the target position for vertical alignment
        for &(from_x, to_x, _, to_is_dummy) in connections {
            buffers.is_source.set(from_x);
            buffers.all_targets.set(to_x); // All targets for routing
            if !to_is_dummy {
                buffers.is_target.set(to_x); // Only mark targets to real nodes
            }
        }

        // Line 1: Vertical
        for i in 0..=max_pos {
            output.push(if buffers.is_source.get(i) {
                V_LINE
            } else {
                ' '
            });
        }
        writeln!(output).ok();

        // Line 2: Arrows for real targets, vertical lines for pass-through (dummy targets)
        for i in 0..=max_pos {
            if buffers.is_target.get(i) {
                output.push(ARROW_DOWN);
            } else if buffers.all_targets.get(i) {
                // Dummy target - show vertical continuation (pass-through)
                output.push(V_LINE);
            } else {
                output.push(' ');
            }
        }
        writeln!(output).ok();
    }
}
