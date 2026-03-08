//! ANSI-colored rendering for arena-backed layout IR.

use super::arena::{EdgePathArena, LayoutEdgeArena, LayoutIRArena, LayoutNodeArena, SubgraphInfoArena};
use crate::render::chars::{
    ARROW_DOWN, ARROW_DOWN_DASHED, ARROW_UP_DASHED,
    CORNER_DL, CORNER_DR, CORNER_UL, CORNER_UR, CROSS,
    H_LINE, H_LINE_DASHED, SELF_LOOP, V_LINE, V_LINE_DASHED,
};

/// Arrows take precedence — non-arrow chars must not overwrite them.
#[inline]
fn is_arrow(ch: char) -> bool {
    matches!(ch, ARROW_DOWN | ARROW_DOWN_DASHED | ARROW_UP_DASHED)
}

impl<'a> LayoutIRArena<'a> {
    // =========================================================================
    // Greedy Graph Coloring
    // =========================================================================

    /// Compute optimal color indices for all edges using greedy graph coloring.
    ///
    /// Adjacent edges (those sharing a source or target node) are assigned different
    /// colors when possible. This reduces visual confusion in complex graphs.
    ///
    /// `color_buffer` must have length >= edge_count.
    /// Returns the number of colors used, or None if buffer too small.
    ///
    /// This matches the heap implementation in LayoutIR::compute_edge_colors().
    pub fn compute_edge_colors(
        &self,
        color_buffer: &mut [usize],
        palette_size: usize,
    ) -> Option<usize> {
        let n = self.edge_count();

        if n == 0 {
            return Some(0);
        }

        if color_buffer.len() < n || palette_size == 0 {
            return None;
        }

        // Fast O(E) modulo coloring
        // Since the palette is now interleaved (Warm/Cool/Light/Dark),
        // sequential indices will be visually distinct.
        for i in 0..n {
            color_buffer[i] = i % palette_size;
        }

        Some(palette_size.min(n))
    }

    // =========================================================================
    // Colored Rendering
    // =========================================================================

    /// Render the layout with ANSI colors to a pre-allocated buffer.
    ///
    /// Each edge is colored based on greedy graph coloring to differentiate
    /// adjacent edges. Nodes use the default terminal color.
    ///
    /// # Arguments
    /// - `buffer`: Output buffer for UTF-8 bytes
    /// - `line_buffer`: Temporary line buffer (must be >= width)
    /// - `color_buffer`: Temporary color buffer (must be >= width)
    /// - `edge_colors`: Pre-computed edge colors from `compute_edge_colors()` (must be >= edge_count)
    /// - `palette`: Color palette to use
    ///
    /// Returns the number of bytes written, or None if buffers too small.
    pub fn render_to_buffer_colored(
        &self,
        buffer: &mut [u8],
        line_buffer: &mut [char],
        color_buffer: &mut [u8],
        edge_colors: &[usize],
        palette: &[u8],
    ) -> Option<usize> {
        if self.is_empty() {
            return Some(0);
        }

        // Validate buffer sizes
        if line_buffer.len() < self.width() || color_buffer.len() < self.width() {
            return None;
        }
        if edge_colors.len() < self.edge_count() {
            return None;
        }

        let mut pos = 0;

        for y in 0..self.height() {
            // Clear buffers
            for c in line_buffer[..self.width()].iter_mut() {
                *c = ' ';
            }
            for c in color_buffer[..self.width()].iter_mut() {
                *c = 0; // 0 = no color (default terminal)
            }

            // 1. Paint edges with colors
            for (edge_idx, edge) in self.edges().iter().enumerate() {
                // Early exit: skip edges that don't occupy this line
                if y < edge.min_y || y > edge.max_y {
                    continue;
                }
                let color_idx = edge_colors[edge_idx];
                let color = palette[color_idx % palette.len()];
                self.paint_edge_at_y_colored(line_buffer, color_buffer, edge, y, color);
            }

            // 2. Paint edge labels (same color as the edge line)
            for (edge_idx, edge) in self.edges().iter().enumerate() {
                // Fast skip: only check edges that could have a label at this y
                if edge.label_len == 0 || edge.label_y != y {
                    continue;
                }
                let color_idx = edge_colors[edge_idx];
                let color = palette[color_idx % palette.len()];
                let label = self.edge_label(edge_idx);
                self.paint_edge_label_colored(
                    line_buffer,
                    color_buffer,
                    label,
                    edge.label_x,
                    color,
                );
            }

            // 2b. Paint subgraph borders (no color)
            if self.has_subgraphs() {
                for sg in self.subgraphs() {
                    Self::paint_subgraph_border_colored(line_buffer, color_buffer, sg, y);
                }
            }

            // 3. Paint nodes (no color - clears color at node positions)
            for (node_idx, node) in self.nodes().iter().enumerate() {
                if y >= node.y && y < node.y + node.height {
                    self.paint_node_colored(line_buffer, color_buffer, node_idx, node, y);
                    // Paint self-loop indicator right after node bracket
                    if node.has_self_loop && y == node.y {
                        let loop_x = node.x + node.width;
                        if loop_x < line_buffer.len() {
                            line_buffer[loop_x] = SELF_LOOP;
                            color_buffer[loop_x] = 0; // no color for self-loop indicator
                        }
                    }
                }
            }

            // 4. Paint subgraph labels (last, always readable, no color)
            if self.has_subgraphs() {
                for (sg_idx, sg) in self.subgraphs().iter().enumerate() {
                    self.paint_subgraph_label_colored(line_buffer, color_buffer, sg_idx, sg, y);
                }
            }

            // Write line with ANSI color escapes
            pos = self.write_colored_line_to_buffer(buffer, pos, line_buffer, color_buffer)?;

            // Add newline
            if pos >= buffer.len() {
                return None;
            }
            buffer[pos] = b'\n';
            pos += 1;
        }

        Some(pos)
    }

    /// Render with ANSI colors and append a legend for skipped labels.
    ///
    /// When edge labels collide with other characters, they are skipped in the main
    /// rendering but added to a legend at the bottom in the format:
    /// `[from] → [to]: "label"` with the edge's color.
    ///
    /// The `skipped_labels` buffer stores tuples of (edge_index, was_skipped).
    /// Returns the number of bytes written, or None if buffers too small.
    ///
    /// # Arguments
    /// * `buffer` - Output byte buffer for UTF-8 + ANSI escapes
    /// * `line_buffer` - Temporary char buffer (width chars)
    /// * `color_buffer` - Temporary color buffer (width bytes)
    /// * `edge_colors` - Pre-computed edge color indices
    /// * `palette` - ANSI 256-color palette values
    /// * `skipped_buffer` - Buffer to track which edges had skipped labels (must be edge_count size)
    pub fn render_to_buffer_colored_with_legend(
        &self,
        buffer: &mut [u8],
        line_buffer: &mut [char],
        color_buffer: &mut [u8],
        edge_colors: &[usize],
        palette: &[u8],
        skipped_buffer: &mut [bool],
    ) -> Option<usize> {
        if self.is_empty() {
            return Some(0);
        }

        // Validate buffer sizes
        if line_buffer.len() < self.width() || color_buffer.len() < self.width() {
            return None;
        }
        if edge_colors.len() < self.edge_count() {
            return None;
        }
        if skipped_buffer.len() < self.edge_count() {
            return None;
        }

        // Initialize skipped buffer
        for s in skipped_buffer.iter_mut() {
            *s = false;
        }

        let mut pos = 0;

        for y in 0..self.height() {
            // Clear buffers
            for c in line_buffer[..self.width()].iter_mut() {
                *c = ' ';
            }
            for c in color_buffer[..self.width()].iter_mut() {
                *c = 0;
            }

            // 1. Paint edges with colors
            for (edge_idx, edge) in self.edges().iter().enumerate() {
                // Early exit: skip edges that don't occupy this line
                if y < edge.min_y || y > edge.max_y {
                    continue;
                }
                let color_idx = edge_colors[edge_idx];
                let color = palette[color_idx % palette.len()];
                self.paint_edge_at_y_colored(line_buffer, color_buffer, edge, y, color);
            }

            // 2. Paint edge labels, tracking skipped ones
            for (edge_idx, edge) in self.edges().iter().enumerate() {
                // Fast skip: only check edges that could have a label at this y
                if edge.label_len == 0 || edge.label_y != y {
                    continue;
                }
                let color_idx = edge_colors[edge_idx];
                let color = palette[color_idx % palette.len()];
                let label = self.edge_label(edge_idx);
                if self.can_place_label(line_buffer, label, edge.label_x) {
                    self.paint_edge_label_colored(
                        line_buffer,
                        color_buffer,
                        label,
                        edge.label_x,
                        color,
                    );
                } else {
                    // Mark as skipped for legend
                    skipped_buffer[edge_idx] = true;
                }
            }

            // 2b. Paint subgraph borders (no color)
            if self.has_subgraphs() {
                for sg in self.subgraphs() {
                    Self::paint_subgraph_border_colored(line_buffer, color_buffer, sg, y);
                }
            }

            // 3. Paint nodes (no color)
            for (node_idx, node) in self.nodes().iter().enumerate() {
                if y >= node.y && y < node.y + node.height {
                    self.paint_node_colored(line_buffer, color_buffer, node_idx, node, y);
                    // Paint self-loop indicator right after node bracket
                    if node.has_self_loop && y == node.y {
                        let loop_x = node.x + node.width;
                        if loop_x < line_buffer.len() {
                            line_buffer[loop_x] = SELF_LOOP;
                            color_buffer[loop_x] = 0;
                        }
                    }
                }
            }

            // 4. Paint subgraph labels (last, always readable, no color)
            if self.has_subgraphs() {
                for (sg_idx, sg) in self.subgraphs().iter().enumerate() {
                    self.paint_subgraph_label_colored(line_buffer, color_buffer, sg_idx, sg, y);
                }
            }

            // Write line with ANSI color escapes
            pos = self.write_colored_line_to_buffer(buffer, pos, line_buffer, color_buffer)?;

            // Add newline
            if pos >= buffer.len() {
                return None;
            }
            buffer[pos] = b'\n';
            pos += 1;
        }

        // Append legend for skipped labels
        let has_skipped = skipped_buffer[..self.edge_count()].iter().any(|&s| s);
        if has_skipped {
            // Write "Edge labels:\n"
            let header = b"\nEdge labels:\n";
            if pos + header.len() > buffer.len() {
                return None;
            }
            buffer[pos..pos + header.len()].copy_from_slice(header);
            pos += header.len();

            // Write each skipped label
            for (edge_idx, edge) in self.edges().iter().enumerate() {
                if !skipped_buffer[edge_idx] || edge.label_len == 0 {
                    continue;
                }

                let label = self.edge_label(edge_idx);
                let color_idx = edge_colors[edge_idx];
                let color = palette[color_idx % palette.len()];

                // Get from/to node labels
                let from_label = self
                    .node_index_by_id(edge.from_id)
                    .map(|idx| self.node_label(idx))
                    .unwrap_or("?");
                let to_label = self
                    .node_index_by_id(edge.to_id)
                    .map(|idx| self.node_label(idx))
                    .unwrap_or("?");

                // Write: "  \x1b[38;5;{color}m{from} → {to}: "{label}"\x1b[0m\n"
                // Prefix: "  "
                if pos + 2 > buffer.len() {
                    return None;
                }
                buffer[pos..pos + 2].copy_from_slice(b"  ");
                pos += 2;

                // ANSI color start
                pos = self.write_ansi_color(buffer, pos, color)?;

                // From label
                let from_bytes = from_label.as_bytes();
                if pos + from_bytes.len() > buffer.len() {
                    return None;
                }
                buffer[pos..pos + from_bytes.len()].copy_from_slice(from_bytes);
                pos += from_bytes.len();

                // Arrow " → "
                let arrow = " → ";
                let arrow_bytes = arrow.as_bytes();
                if pos + arrow_bytes.len() > buffer.len() {
                    return None;
                }
                buffer[pos..pos + arrow_bytes.len()].copy_from_slice(arrow_bytes);
                pos += arrow_bytes.len();

                // To label
                let to_bytes = to_label.as_bytes();
                if pos + to_bytes.len() > buffer.len() {
                    return None;
                }
                buffer[pos..pos + to_bytes.len()].copy_from_slice(to_bytes);
                pos += to_bytes.len();

                // ": \""
                if pos + 3 > buffer.len() {
                    return None;
                }
                buffer[pos..pos + 3].copy_from_slice(b": \"");
                pos += 3;

                // Edge label
                let label_bytes = label.as_bytes();
                if pos + label_bytes.len() > buffer.len() {
                    return None;
                }
                buffer[pos..pos + label_bytes.len()].copy_from_slice(label_bytes);
                pos += label_bytes.len();

                // "\""
                if pos + 1 > buffer.len() {
                    return None;
                }
                buffer[pos] = b'"';
                pos += 1;

                // ANSI reset
                pos = self.write_ansi_reset(buffer, pos)?;

                // Newline
                if pos >= buffer.len() {
                    return None;
                }
                buffer[pos] = b'\n';
                pos += 1;
            }
        }

        Some(pos)
    }

    /// Paint an edge label with the same color as the edge.
    fn paint_edge_label_colored(
        &self,
        line_buffer: &mut [char],
        color_buffer: &mut [u8],
        label: &str,
        x: usize,
        color: u8,
    ) {
        if x >= line_buffer.len() {
            return;
        }

        // Collision detection: skip if would overwrite existing characters
        if !self.can_place_label(line_buffer, label, x) {
            return;
        }

        let mut pos = x;

        // Opening quote
        if pos < line_buffer.len() {
            line_buffer[pos] = '"';
            color_buffer[pos] = color;
            pos += 1;
        }

        // Label characters
        for c in label.chars() {
            if pos < line_buffer.len() {
                line_buffer[pos] = c;
                color_buffer[pos] = color;
                pos += 1;
            }
        }

        // Closing quote
        if pos < line_buffer.len() {
            line_buffer[pos] = '"';
            color_buffer[pos] = color;
        }
    }

    /// Paint a node, clearing color (nodes use default terminal color).
    fn paint_node_colored(
        &self,
        line_buffer: &mut [char],
        color_buffer: &mut [u8],
        node_idx: usize,
        node: &LayoutNodeArena,
        y: usize,
    ) {
        let x = node.x;
        let row = y - node.y;

        if row == 0 {
            // First row: draw [Label]
            let label = self.node_label(node_idx);

            // Opening bracket (no color)
            if x < line_buffer.len() {
                line_buffer[x] = '[';
                color_buffer[x] = 0;
            }

            // Label characters (no color)
            for (i, c) in label.chars().enumerate() {
                let px = x + 1 + i;
                if px < line_buffer.len() {
                    line_buffer[px] = c;
                    color_buffer[px] = 0;
                }
            }

            // Closing bracket (no color)
            if node.width > 0 {
                let close_x = x + node.width - 1;
                if close_x < line_buffer.len() {
                    line_buffer[close_x] = ']';
                    color_buffer[close_x] = 0;
                }
            }
        } else {
            // Subsequent rows: blank body (no color)
            if x < line_buffer.len() {
                line_buffer[x] = '[';
                color_buffer[x] = 0;
            }
            for i in 1..node.width.saturating_sub(1) {
                let px = x + i;
                if px < line_buffer.len() {
                    line_buffer[px] = ' ';
                    color_buffer[px] = 0;
                }
            }
            if node.width > 0 {
                let close_x = x + node.width - 1;
                if close_x < line_buffer.len() {
                    line_buffer[close_x] = ']';
                    color_buffer[close_x] = 0;
                }
            }
        }
    }

    /// Paint an edge at Y with color.
    /// Reversed edges use dashed chars (┊ ┈ ⇣) for visual distinction.
    fn paint_edge_at_y_colored(
        &self,
        line_buffer: &mut [char],
        color_buffer: &mut [u8],
        edge: &LayoutEdgeArena,
        y: usize,
        color: u8,
    ) {
        let from_y = edge.from_y;
        let to_y = edge.to_y;
        let from_x = edge.from_x;
        let to_x = edge.to_x;

        // Select solid or dashed chars based on reversed flag
        let vline = if edge.reversed { V_LINE_DASHED } else { V_LINE };
        let hline = if edge.reversed { H_LINE_DASHED } else { H_LINE };

        if y <= from_y || y >= to_y {
            return;
        }

        match edge.path {
            EdgePathArena::Direct => {
                if from_x < line_buffer.len() {
                    if y == from_y + 1 && edge.reversed {
                        line_buffer[from_x] = ARROW_UP_DASHED;
                        color_buffer[from_x] = color;
                    } else if y == to_y - 1 && !edge.reversed {
                        line_buffer[from_x] = ARROW_DOWN;
                        color_buffer[from_x] = color;
                    } else {
                        if line_buffer[from_x] == H_LINE || line_buffer[from_x] == H_LINE_DASHED {
                            line_buffer[from_x] = CROSS;
                            color_buffer[from_x] = color;
                        } else if !is_arrow(line_buffer[from_x]) {
                            line_buffer[from_x] = vline;
                            color_buffer[from_x] = color;
                        }
                    }
                }
            }
            EdgePathArena::Corner { horizontal_y } => {
                let x1 = from_x;
                let x2 = to_x;
                let (min_x, max_x) = if x1 < x2 { (x1, x2) } else { (x2, x1) };

                if y == horizontal_y {
                    // Horizontal segment
                    for x in min_x..=max_x {
                        if x < line_buffer.len() && !is_arrow(line_buffer[x]) {
                            if line_buffer[x] == ' ' {
                                line_buffer[x] = hline;
                                color_buffer[x] = color;
                            } else if line_buffer[x] == V_LINE || line_buffer[x] == V_LINE_DASHED {
                                line_buffer[x] = CROSS;
                                // Keep existing vertical color
                            }
                        }
                    }
                    // Corners (no dashed variant — keep solid)
                    if x1 < line_buffer.len() && !is_arrow(line_buffer[x1]) {
                        if edge.reversed && horizontal_y <= from_y + 1 {
                            // No room for ⇡ in vertical above horizontal; put it at corner
                            line_buffer[x1] = ARROW_UP_DASHED;
                        } else {
                            line_buffer[x1] = if x1 < x2 { CORNER_DR } else { CORNER_DL };
                        }
                        color_buffer[x1] = color;
                    }
                    if x2 < line_buffer.len() && !is_arrow(line_buffer[x2]) {
                        line_buffer[x2] = if x1 < x2 { CORNER_UL } else { CORNER_UR };
                        color_buffer[x2] = color;
                    }
                } else if y > from_y && y < horizontal_y {
                    // Vertical from source to horizontal
                    if x1 < line_buffer.len() {
                        if y == from_y + 1 && edge.reversed {
                            line_buffer[x1] = ARROW_UP_DASHED;
                            color_buffer[x1] = color;
                        } else if line_buffer[x1] == H_LINE || line_buffer[x1] == H_LINE_DASHED {
                            line_buffer[x1] = CROSS;
                            color_buffer[x1] = color;
                        } else if !is_arrow(line_buffer[x1]) {
                            line_buffer[x1] = vline;
                            color_buffer[x1] = color;
                        }
                    }
                } else if y > horizontal_y && y < to_y {
                    // Vertical from horizontal to target
                    if x2 < line_buffer.len() {
                        if y == to_y - 1 && !edge.reversed {
                            line_buffer[x2] = ARROW_DOWN;
                            color_buffer[x2] = color;
                        } else if line_buffer[x2] == H_LINE || line_buffer[x2] == H_LINE_DASHED {
                            line_buffer[x2] = CROSS;
                            color_buffer[x2] = color;
                        } else if !is_arrow(line_buffer[x2]) {
                            line_buffer[x2] = vline;
                            color_buffer[x2] = color;
                        }
                    }
                }
            }
            EdgePathArena::MultiSegment {
                waypoints_start,
                waypoints_len,
                start_y_offset,
            } => {
                let waypoints = self.edge_waypoints_raw(waypoints_start, waypoints_len);
                let segment_count = waypoints_len + 1;

                for seg_idx in 0..segment_count {
                    let (x1, y1) = if seg_idx == 0 {
                        (from_x, from_y)
                    } else {
                        waypoints[seg_idx - 1]
                    };
                    let (x2, y2) = if seg_idx == waypoints_len {
                        (to_x, to_y)
                    } else {
                        waypoints[seg_idx]
                    };

                    let is_first_segment = seg_idx == 0;
                    let is_last_segment = seg_idx == segment_count - 1;

                    if x1 == x2 {
                        // Pure vertical segment
                        let start_y = if is_first_segment { y1 + 1 } else { y1 };
                        if y >= start_y && y < y2 && x1 < line_buffer.len() {
                            if is_first_segment && y == y1 + 1 && edge.reversed {
                                line_buffer[x1] = ARROW_UP_DASHED;
                                color_buffer[x1] = color;
                            } else if is_last_segment && y == y2 - 1 && !edge.reversed {
                                line_buffer[x1] = ARROW_DOWN;
                                color_buffer[x1] = color;
                            } else {
                                if line_buffer[x1] == H_LINE || line_buffer[x1] == H_LINE_DASHED {
                                    line_buffer[x1] = CROSS;
                                    color_buffer[x1] = color;
                                } else if !is_arrow(line_buffer[x1]) {
                                    line_buffer[x1] = vline;
                                    color_buffer[x1] = color;
                                }
                            }
                        }
                    } else if y1 == y2 {
                        // Pure horizontal segment
                        if y == y1 {
                            let (min_x, max_x) = if x1 < x2 { (x1, x2) } else { (x2, x1) };
                            for x in min_x..=max_x {
                                if x < line_buffer.len() && !is_arrow(line_buffer[x]) {
                                    if line_buffer[x] == ' ' {
                                        line_buffer[x] = hline;
                                        color_buffer[x] = color;
                                    } else if line_buffer[x] == V_LINE || line_buffer[x] == V_LINE_DASHED {
                                        line_buffer[x] = CROSS;
                                        // Keep vertical color
                                    }
                                }
                            }
                        }
                    } else {
                        // Diagonal: corner routing
                        let mut corner_y = y1 + 1;
                        if is_first_segment {
                            corner_y += start_y_offset;
                        }

                        // FIXED: Draw vertical segment from y1 to corner_y if there is an offset
                        if is_first_segment && start_y_offset > 0 {
                            let start_drop = y1 + 1;
                            if y >= start_drop && y < corner_y && x1 < line_buffer.len() {
                                if y == y1 + 1 && edge.reversed {
                                    line_buffer[x1] = ARROW_UP_DASHED;
                                    color_buffer[x1] = color;
                                } else if line_buffer[x1] == H_LINE || line_buffer[x1] == H_LINE_DASHED {
                                    line_buffer[x1] = CROSS;
                                    color_buffer[x1] = color;
                                } else if !is_arrow(line_buffer[x1]) {
                                    line_buffer[x1] = vline;
                                    color_buffer[x1] = color;
                                }
                            }
                        }

                        if y == corner_y {
                            let (min_x, max_x) = if x1 < x2 { (x1, x2) } else { (x2, x1) };
                            for x in min_x..=max_x {
                                if x < line_buffer.len() && !is_arrow(line_buffer[x]) {
                                    if x == x1 {
                                        if edge.reversed && is_first_segment && corner_y <= y1 + 1 {
                                            line_buffer[x] = ARROW_UP_DASHED;
                                        } else {
                                            line_buffer[x] =
                                                if x1 < x2 { CORNER_DR } else { CORNER_DL };
                                        }
                                        color_buffer[x] = color;
                                    } else if x == x2 {
                                        line_buffer[x] =
                                            if x1 < x2 { CORNER_UL } else { CORNER_UR };
                                        color_buffer[x] = color;
                                    } else if line_buffer[x] == ' ' {
                                        line_buffer[x] = hline;
                                        color_buffer[x] = color;
                                    } else if line_buffer[x] == V_LINE || line_buffer[x] == V_LINE_DASHED {
                                        line_buffer[x] = CROSS;
                                        // Keep vertical color
                                    }
                                }
                            }
                        }

                        if y > corner_y && y < y2 && x2 < line_buffer.len() {
                            if is_last_segment && y == y2 - 1 && !edge.reversed {
                                line_buffer[x2] = ARROW_DOWN;
                                color_buffer[x2] = color;
                            } else {
                                if line_buffer[x2] == H_LINE || line_buffer[x2] == H_LINE_DASHED {
                                    line_buffer[x2] = CROSS;
                                    color_buffer[x2] = color;
                                } else if !is_arrow(line_buffer[x2]) {
                                    line_buffer[x2] = vline;
                                    color_buffer[x2] = color;
                                }
                            }
                        }

                        if !is_first_segment && y == y1 && x1 < line_buffer.len() {
                            if line_buffer[x1] == H_LINE || line_buffer[x1] == H_LINE_DASHED {
                                line_buffer[x1] = CROSS;
                                color_buffer[x1] = color;
                            } else if !is_arrow(line_buffer[x1]) {
                                line_buffer[x1] = vline;
                                color_buffer[x1] = color;
                            }
                        }
                    }
                }
            }
            // SideChannel / Spline: fall back to Direct rendering
            EdgePathArena::SideChannel { .. } | EdgePathArena::Spline { .. } => {
                if from_x < line_buffer.len() {
                    if y == from_y + 1 && edge.reversed {
                        line_buffer[from_x] = ARROW_UP_DASHED;
                        color_buffer[from_x] = color;
                    } else if y == to_y - 1 && !edge.reversed {
                        line_buffer[from_x] = ARROW_DOWN;
                        color_buffer[from_x] = color;
                    } else if !is_arrow(line_buffer[from_x]) {
                        line_buffer[from_x] = vline;
                        color_buffer[from_x] = color;
                    }
                }
            }
        }
    }

    /// Write a line with ANSI color escapes to the buffer.
    /// Returns the new position, or None if buffer overflow.
    fn write_colored_line_to_buffer(
        &self,
        buffer: &mut [u8],
        mut pos: usize,
        chars: &[char],
        colors: &[u8],
    ) -> Option<usize> {
        // Find trimmed length
        let trimmed_len = chars[..self.width()]
            .iter()
            .rposition(|&c| c != ' ')
            .map(|i| i + 1)
            .unwrap_or(0);

        let mut last_color: u8 = 0;

        for i in 0..trimmed_len {
            let c = chars[i];
            let color = colors[i];

            // Handle color changes
            if color != 0 && color != last_color {
                // Write ANSI escape: \x1b[38;5;NNNm (up to 11 bytes)
                pos = self.write_ansi_color(buffer, pos, color)?;
                last_color = color;
            } else if color == 0 && last_color != 0 {
                // Reset to default: \x1b[0m (4 bytes)
                pos = self.write_ansi_reset(buffer, pos)?;
                last_color = 0;
            }

            // Write the character (up to 4 bytes UTF-8)
            let mut char_buf = [0u8; 4];
            let encoded = c.encode_utf8(&mut char_buf);
            if pos + encoded.len() > buffer.len() {
                return None;
            }
            buffer[pos..pos + encoded.len()].copy_from_slice(encoded.as_bytes());
            pos += encoded.len();
        }

        // Reset color at end of line if needed
        if last_color != 0 {
            pos = self.write_ansi_reset(buffer, pos)?;
        }

        Some(pos)
    }

    /// Write ANSI color escape sequence: \x1b[38;5;{color}m
    #[inline]
    fn write_ansi_color(&self, buffer: &mut [u8], pos: usize, color: u8) -> Option<usize> {
        // Format: \x1b[38;5;NNNm where NNN is 1-3 digits
        // Max length: 11 bytes (\x1b[38;5;255m)
        let prefix = b"\x1b[38;5;";
        if pos + 11 > buffer.len() {
            return None;
        }
        buffer[pos..pos + 7].copy_from_slice(prefix);
        let mut p = pos + 7;

        // Write color number as ASCII digits
        if color >= 100 {
            buffer[p] = b'0' + (color / 100);
            p += 1;
        }
        if color >= 10 {
            buffer[p] = b'0' + ((color / 10) % 10);
            p += 1;
        }
        buffer[p] = b'0' + (color % 10);
        p += 1;

        buffer[p] = b'm';
        p += 1;

        Some(p)
    }

    /// Write ANSI reset escape sequence: \x1b[0m
    #[inline]
    fn write_ansi_reset(&self, buffer: &mut [u8], pos: usize) -> Option<usize> {
        const RESET: &[u8] = b"\x1b[0m";
        if pos + RESET.len() > buffer.len() {
            return None;
        }
        buffer[pos..pos + RESET.len()].copy_from_slice(RESET);
        Some(pos + RESET.len())
    }

    // ── Subgraph border rendering (colored) ──────────────────────────────

    /// Paint a subgraph border on the given line buffer row (no color).
    fn paint_subgraph_border_colored(
        line_buffer: &mut [char],
        color_buffer: &mut [u8],
        sg: &SubgraphInfoArena,
        y: usize,
    ) {
        if y < sg.y || y >= sg.y + sg.height { return; }
        let left = sg.x;
        let right = sg.x + sg.width.saturating_sub(1);
        if left >= line_buffer.len() { return; }
        let right = right.min(line_buffer.len() - 1);

        if y == sg.y {
            line_buffer[left] = '╔'; color_buffer[left] = 0;
            if right > left { line_buffer[right] = '╗'; color_buffer[right] = 0; }
            for col in (left + 1)..right {
                line_buffer[col] = Self::merge_h_border_c(line_buffer[col]);
                color_buffer[col] = 0;
            }
        } else if y == sg.y + sg.height - 1 {
            line_buffer[left] = '╚'; color_buffer[left] = 0;
            if right > left { line_buffer[right] = '╝'; color_buffer[right] = 0; }
            for col in (left + 1)..right {
                line_buffer[col] = Self::merge_h_border_c(line_buffer[col]);
                color_buffer[col] = 0;
            }
        } else {
            line_buffer[left] = Self::merge_v_border_c(line_buffer[left]);
            color_buffer[left] = 0;
            if right > left {
                line_buffer[right] = Self::merge_v_border_c(line_buffer[right]);
                color_buffer[right] = 0;
            }
        }
    }

    #[inline]
    fn merge_h_border_c(existing: char) -> char {
        match existing {
            '│' | '┊' | '┼' | '├' | '┤' => '╪',
            '↓' | '⇣' | '┌' | '┐' | '┬' => '╤',
            '↑' | '⇡' | '└' | '┘' | '┴' => '╧',
            '╔' | '╗' | '╚' | '╝' | '═' | '║' | '╪' | '╫' | '╤' | '╧' | '╞' | '╡' => existing,
            _ => '═',
        }
    }

    #[inline]
    fn merge_v_border_c(existing: char) -> char {
        match existing {
            '─' | '┈' | '┼' | '┬' | '┴' => '╫',
            '→' | '┌' | '└' | '├' => '╞',
            '←' | '┐' | '┘' | '┤' => '╡',
            '╔' | '╗' | '╚' | '╝' | '═' | '║' | '╪' | '╫' | '╤' | '╧' | '╞' | '╡' => existing,
            _ => '║',
        }
    }

    /// Paint a subgraph label inside the box (no color).
    fn paint_subgraph_label_colored(
        &self,
        line_buffer: &mut [char],
        color_buffer: &mut [u8],
        sg_idx: usize,
        sg: &SubgraphInfoArena,
        y: usize,
    ) {
        let label_y = sg.y + 1;
        if y != label_y { return; }
        if sg.width < 4 || sg.height < 3 { return; }
        let label = self.subgraph_label(sg_idx);
        if label.is_empty() { return; }

        let label_start = sg.x + 2;
        let max_len = sg.width.saturating_sub(4);

        let mut col = label_start;
        for ch in label.chars().take(max_len) {
            if col < line_buffer.len() {
                line_buffer[col] = ch;
                color_buffer[col] = 0;
                col += 1;
            }
        }
    }
}
