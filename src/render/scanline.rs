//! Scanline-based ASCII renderer using Y-index for O(1) line queries.
//!
//! This is an experimental fast renderer that uses the spatial Y-index
//! to render each line independently, enabling:
//! - O(items_on_line) per line instead of O(all_items)
//! - Streaming output (no full canvas allocation)
//! - Better cache locality
//!
//! Trade-offs vs the main renderer:
//! - Simpler edge routing (may not handle complex crossings as elegantly)
//! - Optimized for speed over visual perfection

use crate::ir::{EdgePath, LayoutIR};
use crate::render::chars::{
    merge_chars, ARROW_DOWN, ARROW_DOWN_DASHED, CORNER_DL, CORNER_DR, CORNER_UL, CORNER_UR, CROSS, H_LINE, H_LINE_DASHED, V_LINE,
    V_LINE_DASHED,
};
use crate::render::colors::{self, Palette};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt::Write;

impl<'a> LayoutIR<'a> {
    /// Render using the scanline approach with Y-index.
    /// This is faster than the painter approach for large graphs.
    pub fn render_scanline(&self) -> String {
        let mut output = String::with_capacity(self.width() * self.height());
        self.render_scanline_to(&mut output);
        output
    }

    /// Render scanline with ANSI colors for edges.
    ///
    /// Each edge is colored based on its edge_index, cycling through the palette.
    /// This makes it easier to trace individual edges in complex graphs.
    ///
    /// # Example
    ///
    /// ```
    /// use ascii_dag::DAG;
    /// use ascii_dag::render::colors::Palette;
    ///
    /// let dag = DAG::from_edges(&[(1, "A"), (2, "B"), (3, "C")], &[(1, 2), (1, 3)]);
    /// let ir = dag.compute_layout();
    ///
    /// // Render with colored edges
    /// let output = ir.render_scanline_colored(Palette::Ansi);
    /// println!("{}", output);
    /// ```
    pub fn render_scanline_colored(&self, palette: Palette) -> String {
        let mut output = String::with_capacity(self.width() * self.height() * 2);
        self.render_scanline_colored_to(&mut output, palette);
        output
    }

    /// Render scanline with ANSI colors to a String buffer.
    /// Uses O(E) modulo coloring with a high-contrast palette for performance.
    pub fn render_scanline_colored_to(&self, output: &mut String, palette: Palette) {
        let y_index = self.y_index();

        // Compute optimized color assignments (modulo based)
        let palette_colors = palette.colors();
        let edge_color_indices = self.compute_edge_colors(palette_colors.len());

        // Allocate line buffer and color buffer
        let mut line_buffer: Vec<char> = vec![' '; self.width()];
        let mut color_buffer: Vec<u8> = vec![0; self.width()]; // 0 = no color

        for y in 0..self.height() {
            // Clear buffers
            line_buffer.fill(' ');
            color_buffer.fill(0);

            if let Some(occupancy) = y_index.get(y) {
                // 1. Paint edge lines with colors
                for &edge_idx in &occupancy.edge_indices {
                    let edge = &self.edges()[edge_idx];
                    let color_idx = edge_color_indices.get(edge_idx).copied().unwrap_or(0);
                    let color = palette_colors[color_idx % palette_colors.len()];
                    self.paint_edge_at_y_colored(
                        &mut line_buffer,
                        &mut color_buffer,
                        edge,
                        y,
                        color,
                    );
                }

                // 2. Paint edge labels (same color as the edge line)
                for &edge_idx in &occupancy.edge_indices {
                    let edge = &self.edges()[edge_idx];
                    if let (Some(label), Some((label_x, label_y))) =
                        (edge.label, edge.label_position)
                    {
                        if y == label_y {
                            let color_idx = edge_color_indices.get(edge_idx).copied().unwrap_or(0);
                            let color = palette_colors[color_idx % palette_colors.len()];
                            self.paint_edge_label_colored(
                                &mut line_buffer,
                                &mut color_buffer,
                                label,
                                label_x,
                                color,
                            );
                        }
                    }
                }

                // 3. Paint nodes (no color - uses default terminal color)
                for &node_idx in &occupancy.node_indices {
                    let node = &self.nodes()[node_idx];
                    self.paint_node_colored(&mut line_buffer, &mut color_buffer, node);
                }
            }

            // Write line with ANSI color escapes
            self.write_colored_line(output, &line_buffer, &color_buffer);
            output.push('\n');
        }
    }

    /// Render scanline with ANSI colors and append a legend for any skipped labels.
    ///
    /// Labels may be skipped due to collisions with other characters. When this happens,
    /// a legend is appended at the bottom showing: `[from] → [to]: "label"` in the edge's color.
    ///
    /// # Example
    ///
    /// ```
    /// use ascii_dag::DAG;
    /// use ascii_dag::render::colors::Palette;
    ///
    /// let mut dag = DAG::new();
    /// dag.add_node(1, "A");
    /// dag.add_node(2, "B");
    /// dag.add_edge(1, 2, Some("depends"));
    ///
    /// let ir = dag.compute_layout();
    /// let output = ir.render_scanline_colored_with_legend(Palette::Ansi);
    /// println!("{}", output);
    /// ```
    pub fn render_scanline_colored_with_legend(&self, palette: Palette) -> String {
        let mut output = String::with_capacity(self.width() * self.height() * 2);
        let skipped = self.render_scanline_colored_track_skipped(&mut output, palette);

        // Append legend if any labels were skipped
        if !skipped.is_empty() {
            output.push_str("\nEdge labels:\n");
            for (from_label, to_label, label, color) in skipped {
                let _ = writeln!(
                    output,
                    "  \x1b[38;5;{}m{} → {}: \"{}\"\x1b[0m",
                    color, from_label, to_label, label
                );
            }
        }

        output
    }

    /// Render scanline with ANSI colors, tracking skipped labels.
    /// Returns a list of (from_label, to_label, edge_label, color) for each skipped label.
    fn render_scanline_colored_track_skipped(
        &self,
        output: &mut String,
        palette: Palette,
    ) -> Vec<(&'a str, &'a str, &'a str, u8)> {
        let y_index = self.y_index();
        let mut skipped_labels: Vec<(&'a str, &'a str, &'a str, u8)> = Vec::new();

        // Compute optimized color assignments (modulo based)
        let palette_colors = palette.colors();
        let edge_color_indices = self.compute_edge_colors(palette_colors.len());

        // Allocate line buffer and color buffer
        let mut line_buffer: Vec<char> = vec![' '; self.width()];
        let mut color_buffer: Vec<u8> = vec![0; self.width()]; // 0 = no color

        // Greedy label placement: track pending labels that haven't been placed yet
        // Each entry: (edge_idx, min_y, max_y, placed)
        let mut pending_labels: Vec<(usize, usize, usize, bool)> = Vec::new();
        for (edge_idx, edge) in self.edges().iter().enumerate() {
            if edge.label.is_some() {
                // strict placement: use the pre-calculated label_y if available
                // This prevents "stacking" labels and forces collisions to the legend,
                // which results in a cleaner graph (preferred by user).
                if let Some((_, label_y)) = edge.label_position {
                    pending_labels.push((edge_idx, label_y, label_y, false));
                } else {
                     // Fallback for edges without pre-calculated position (shouldn't happen with valid layout)
                    let min_y = edge.from_y.saturating_add(2);
                    let max_y = edge.to_y.saturating_sub(2);
                    if max_y >= min_y {
                        pending_labels.push((edge_idx, min_y, max_y, false));
                    }
                }
            }
        }

        for y in 0..self.height() {
            // Clear buffers
            line_buffer.fill(' ');
            color_buffer.fill(0);

            if let Some(occupancy) = y_index.get(y) {
                // 1. Paint edge lines with colors
                for &edge_idx in &occupancy.edge_indices {
                    let edge = &self.edges()[edge_idx];
                    let color_idx = edge_color_indices.get(edge_idx).copied().unwrap_or(0);
                    let color = palette_colors[color_idx % palette_colors.len()];
                    self.paint_edge_at_y_colored(
                        &mut line_buffer,
                        &mut color_buffer,
                        edge,
                        y,
                        color,
                    );
                }

                // 2. Greedy label placement: try to place pending labels at this Y
                // Skip rows that have nodes to avoid label-node collisions
                let has_nodes = !occupancy.node_indices.is_empty();
                
                if !has_nodes {
                    for (edge_idx, min_y, max_y, placed) in pending_labels.iter_mut() {
                        if *placed {
                            continue;
                        }
                        // Ensure we are within the valid vertical range for this edge
                        if y < *min_y || y > *max_y {
                            continue; 
                        }

                        let edge = &self.edges()[*edge_idx];
                        if let Some(label) = edge.label {
                            // Compute label X at this Y based on edge path
                            let label_x = self.compute_label_x_at_y(edge, y);
                            let label_len = label.chars().count() + 2; // +2 for quotes
                            let half_len = label_len / 2;
                            let label_x = label_x.saturating_sub(half_len);

                            // Check collision with line buffer
                            if self.can_place_label(&line_buffer, label, label_x) {
                                let color_idx = edge_color_indices.get(*edge_idx).copied().unwrap_or(0);
                                let color = palette_colors[color_idx % palette_colors.len()];
                                self.paint_edge_label_colored(
                                    &mut line_buffer,
                                    &mut color_buffer,
                                    label,
                                    label_x,
                                    color,
                                );
                                *placed = true;
                            }
                        }
                    }
                }

                // 3. Paint nodes (no color - uses default terminal color)
                for &node_idx in &occupancy.node_indices {
                    let node = &self.nodes()[node_idx];
                    self.paint_node_colored(&mut line_buffer, &mut color_buffer, node);
                }
            }

            // Write line with ANSI color escapes
            self.write_colored_line(output, &line_buffer, &color_buffer);
            output.push('\n');
        }

        // Collect skipped labels (those that were never placed)
        for (edge_idx, _min_y, _max_y, placed) in pending_labels {
            if !placed {
                let edge = &self.edges()[edge_idx];
                if let Some(label) = edge.label {
                    let color_idx = edge_color_indices.get(edge_idx).copied().unwrap_or(0);
                    let color = palette_colors[color_idx % palette_colors.len()];
                    if let (Some(from_node), Some(to_node)) = (
                        self.id_to_index
                            .get(&edge.from_id)
                            .map(|&i| self.nodes()[i].label),
                        self.id_to_index
                            .get(&edge.to_id)
                            .map(|&i| self.nodes()[i].label),
                    ) {
                        skipped_labels.push((from_node, to_node, label, color));
                    }
                }
            }
        }

        skipped_labels
    }

    /// Write a line with ANSI color escapes, trimming trailing spaces.
    fn write_colored_line(&self, output: &mut String, chars: &[char], colors: &[u8]) {
        // Find trimmed length
        let trimmed_len = chars
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
                // Start new color
                let _ = write!(output, "\x1b[38;5;{}m", color);
                last_color = color;
            } else if color == 0 && last_color != 0 {
                // Reset to default
                output.push_str(colors::RESET);
                last_color = 0;
            }

            output.push(c);
        }

        // Reset color at end of line if needed
        if last_color != 0 {
            output.push_str(colors::RESET);
        }
    }

    /// Render scanline to a String buffer.
    pub fn render_scanline_to(&self, output: &mut String) {
        // Ensure Y-index is built
        let y_index = self.y_index();

        // Allocate a single line buffer that we reuse
        let mut line_buffer: Vec<char> = vec![' '; self.width()];

        for y in 0..self.height() {
            // Clear line buffer
            line_buffer.fill(' ');

            if let Some(occupancy) = y_index.get(y) {
                // 1. Paint edge lines FIRST (connectors, arrows)
                for &edge_idx in &occupancy.edge_indices {
                    let edge = &self.edges()[edge_idx];
                    self.paint_edge_at_y(&mut line_buffer, edge, y);
                }

                // 2. Paint edge labels (overwrites edge lines where needed)
                for &edge_idx in &occupancy.edge_indices {
                    let edge = &self.edges()[edge_idx];
                    if let (Some(label), Some((label_x, label_y))) =
                        (edge.label, edge.label_position)
                    {
                        if y == label_y {
                            self.paint_edge_label(&mut line_buffer, label, label_x);
                        }
                    }
                }

                // 3. Paint nodes on this line (highest priority)
                for &node_idx in &occupancy.node_indices {
                    let node = &self.nodes()[node_idx];
                    self.paint_node(&mut line_buffer, node);
                }
            }

            // Write line to output (trim trailing spaces)
            let trimmed_len = line_buffer
                .iter()
                .rposition(|&c| c != ' ')
                .map(|i| i + 1)
                .unwrap_or(0);

            for &c in &line_buffer[..trimmed_len] {
                output.push(c);
            }
            output.push('\n');
        }
    }

    /// Render using a pre-allocated line buffer (arena-friendly).
    ///
    /// The caller provides a reusable `line_buffer` slice that must be at least
    /// `self.width()` chars. This eliminates the heap allocation for the line buffer.
    ///
    /// # Example
    ///
    /// ```
    /// use ascii_dag::DAG;
    ///
    /// let dag = DAG::from_edges(&[(1, "A"), (2, "B")], &[(1, 2)]);
    /// let ir = dag.compute_layout();
    ///
    /// // Pre-allocate the line buffer (can be on stack or in arena)
    /// let mut line_buffer = vec![' '; ir.width()];
    /// let mut output = String::with_capacity(ir.width() * ir.height());
    ///
    /// ir.render_scanline_with_buffer(&mut line_buffer, &mut output);
    /// ```
    pub fn render_scanline_with_buffer(&self, line_buffer: &mut [char], output: &mut String) {
        let y_index = self.y_index();
        let width = self.width().min(line_buffer.len());

        for y in 0..self.height() {
            // Clear line buffer
            for c in line_buffer[..width].iter_mut() {
                *c = ' ';
            }

            if let Some(occupancy) = y_index.get(y) {
                // Paint edges FIRST so nodes take precedence
                for &edge_idx in &occupancy.edge_indices {
                    let edge = &self.edges()[edge_idx];
                    self.paint_edge_at_y(&mut line_buffer[..width], edge, y);
                }

                // Paint nodes on this line (overwrites any edge characters)
                for &node_idx in &occupancy.node_indices {
                    let node = &self.nodes()[node_idx];
                    self.paint_node(&mut line_buffer[..width], node);
                }
            }

            // Write line to output (trim trailing spaces)
            let trimmed_len = line_buffer[..width]
                .iter()
                .rposition(|&c| c != ' ')
                .map(|i| i + 1)
                .unwrap_or(0);

            for &c in &line_buffer[..trimmed_len] {
                output.push(c);
            }
            output.push('\n');
        }
    }

    /// Render directly to a byte buffer (zero String allocations).
    ///
    /// This is the most allocation-efficient render method. The output buffer
    /// should be sized to `width * height * 4` bytes to accommodate UTF-8 box
    /// drawing characters.
    ///
    /// Returns the number of bytes written.
    ///
    /// # Example
    ///
    /// ```
    /// use ascii_dag::DAG;
    ///
    /// let dag = DAG::from_edges(&[(1, "A"), (2, "B")], &[(1, 2)]);
    /// let ir = dag.compute_layout();
    ///
    /// // Allocate buffers (can be on stack or from arena)
    /// let mut line_buffer = vec![' '; ir.width()];
    /// let mut output_buffer = vec![0u8; ir.width() * ir.height() * 4];
    ///
    /// let bytes_written = ir.render_scanline_to_bytes(&mut line_buffer, &mut output_buffer);
    /// let output = core::str::from_utf8(&output_buffer[..bytes_written]).unwrap();
    /// ```
    pub fn render_scanline_to_bytes(&self, line_buffer: &mut [char], output: &mut [u8]) -> usize {
        let y_index = self.y_index();
        let width = self.width().min(line_buffer.len());
        let mut offset = 0;

        for y in 0..self.height() {
            // Clear line buffer
            for c in line_buffer[..width].iter_mut() {
                *c = ' ';
            }

            if let Some(occupancy) = y_index.get(y) {
                // Paint edges FIRST so nodes take precedence
                for &edge_idx in &occupancy.edge_indices {
                    let edge = &self.edges()[edge_idx];
                    self.paint_edge_at_y(&mut line_buffer[..width], edge, y);
                }

                // Paint nodes on this line (overwrites any edge characters)
                for &node_idx in &occupancy.node_indices {
                    let node = &self.nodes()[node_idx];
                    self.paint_node(&mut line_buffer[..width], node);
                }
            }

            // Write line to output (trim trailing spaces)
            let trimmed_len = line_buffer[..width]
                .iter()
                .rposition(|&c| c != ' ')
                .map(|i| i + 1)
                .unwrap_or(0);

            // Encode UTF-8 directly to output buffer
            for &c in &line_buffer[..trimmed_len] {
                let remaining = output.len() - offset;
                if remaining < 4 {
                    break; // Not enough space
                }
                offset += c.encode_utf8(&mut output[offset..]).len();
            }

            // Add newline
            if offset < output.len() {
                output[offset] = b'\n';
                offset += 1;
            }
        }

        offset
    }

    /// Paint a node onto the line buffer.
    #[inline]
    fn paint_node(&self, buffer: &mut [char], node: &crate::ir::LayoutNode) {
        let label = node.label;
        let x = node.x;

        // Bounds check
        if x >= buffer.len() {
            return;
        }

        // Draw [Label]
        if x < buffer.len() {
            buffer[x] = '[';
        }

        for (i, c) in label.chars().enumerate() {
            let pos = x + 1 + i;
            if pos < buffer.len() {
                buffer[pos] = c;
            }
        }

        let close_pos = x + 1 + label.chars().count();
        if close_pos < buffer.len() {
            buffer[close_pos] = ']';
        }
    }

    /// Paint the portion of an edge that crosses line Y.
    /// Reversed edges use dashed chars (┊ ┈ ⇣) for visual distinction.
    #[inline]
    fn paint_edge_at_y(&self, buffer: &mut [char], edge: &crate::ir::LayoutEdge<'a>, y: usize) {
        // Select solid or dashed chars based on reversed flag
        let vline = if edge.reversed { V_LINE_DASHED } else { V_LINE };
        let hline = if edge.reversed { H_LINE_DASHED } else { H_LINE };
        let arrow = if edge.reversed { ARROW_DOWN_DASHED } else { ARROW_DOWN };

        match &edge.path {
            EdgePath::Direct => {
                // Vertical line from from_y+1 to to_y-1 at from_x
                // Arrow at to_y-1 (line before target node)
                let x = edge.from_x;
                if x < buffer.len() && y > edge.from_y && y < edge.to_y {
                    // Draw arrow on the line just before the target node
                    if y == edge.to_y - 1 {
                        buffer[x] = arrow;
                    } else {
                        // Vertical line in between
                        if buffer[x] == H_LINE || buffer[x] == H_LINE_DASHED {
                            buffer[x] = CROSS;
                        } else {
                            buffer[x] = vline;
                        }
                    }
                }
            }
            EdgePath::Corner { horizontal_y } => {
                let x1 = edge.from_x;
                let x2 = edge.to_x;
                let (min_x, max_x) = if x1 < x2 { (x1, x2) } else { (x2, x1) };

                if y == *horizontal_y {
                    // Horizontal segment
                    for x in min_x..=max_x {
                        if x < buffer.len() {
                            if buffer[x] == ' ' {
                                buffer[x] = hline;
                            } else if buffer[x] == V_LINE || buffer[x] == V_LINE_DASHED {
                                buffer[x] = CROSS;
                            }
                        }
                    }
                    // Corners (no dashed variant — keep solid)
                    if x1 < buffer.len() {
                        buffer[x1] = if x1 < x2 { CORNER_DR } else { CORNER_DL };
                    }
                    if x2 < buffer.len() {
                        buffer[x2] = if x1 < x2 { CORNER_UL } else { CORNER_UR };
                    }
                } else if y > edge.from_y && y < *horizontal_y {
                    // Vertical from source to horizontal
                    if x1 < buffer.len() {
                        if buffer[x1] == H_LINE || buffer[x1] == H_LINE_DASHED {
                            buffer[x1] = CROSS;
                        } else {
                            buffer[x1] = vline;
                        }
                    }
                } else if y > *horizontal_y && y < edge.to_y {
                    // Vertical from horizontal to target
                    // Arrow on the line just before target node
                    if x2 < buffer.len() {
                        if y == edge.to_y - 1 {
                            buffer[x2] = arrow;
                        } else {
                            if buffer[x2] == H_LINE || buffer[x2] == H_LINE_DASHED {
                                buffer[x2] = CROSS;
                            } else {
                                buffer[x2] = vline;
                            }
                        }
                    }
                }
            }
            EdgePath::SideChannel {
                channel_x,
                start_y,
                end_y,
            } => {
                // SideChannel routes: source → right to channel → down → left to target
                // Layout:
                //   [Source]
                //      └────┐  ← start_y: horizontal from from_x to channel_x
                //           │  ← vertical in channel
                //      ┌────┘  ← end_y: horizontal from channel_x back to to_x
                //      ↓       ← to_y-1: arrow pointing down to target
                //   [Target]   ← to_y

                let from_x = edge.from_x;
                let to_x = edge.to_x;

                if y == *start_y {
                    // Horizontal from source to channel (going right)
                    for x in from_x..=*channel_x {
                        if x < buffer.len() {
                            if x == from_x {
                                buffer[x] = CORNER_DR; // └
                            } else if x == *channel_x {
                                buffer[x] = CORNER_UL; // ┐
                            } else {
                                if buffer[x] == ' ' {
                                    buffer[x] = hline;
                                } else if buffer[x] == V_LINE || buffer[x] == V_LINE_DASHED {
                                    buffer[x] = CROSS;
                                }
                            }
                        }
                    }
                } else if y > *start_y && y < *end_y {
                    // Vertical line in channel
                    if *channel_x < buffer.len() {
                        if buffer[*channel_x] == H_LINE || buffer[*channel_x] == H_LINE_DASHED {
                            buffer[*channel_x] = CROSS;
                        } else {
                            buffer[*channel_x] = vline;
                        }
                    }
                } else if y == *end_y {
                    // Horizontal from channel back to target (going left)
                    for x in to_x..=*channel_x {
                        if x < buffer.len() {
                            if x == *channel_x {
                                buffer[x] = CORNER_DL; // ┘
                            } else if x == to_x {
                                // If end_y is right above to_y, this is the arrow position
                                // Otherwise it's a corner that continues down
                                if *end_y + 1 >= edge.to_y {
                                    buffer[x] = arrow;
                                } else {
                                    buffer[x] = CORNER_UR; // ┌
                                }
                            } else {
                                if buffer[x] == ' ' {
                                    buffer[x] = hline;
                                } else if buffer[x] == V_LINE || buffer[x] == V_LINE_DASHED {
                                    buffer[x] = CROSS;
                                }
                            }
                        }
                    }
                } else if y > *end_y && y < edge.to_y {
                    // Vertical from end_y down to target, arrow on last line before target
                    if to_x < buffer.len() {
                        if y == edge.to_y - 1 {
                            buffer[to_x] = arrow;
                        } else {
                            if buffer[to_x] == H_LINE || buffer[to_x] == H_LINE_DASHED {
                                buffer[to_x] = CROSS;
                            } else {
                                buffer[to_x] = vline;
                            }
                        }
                    }
                }
            }
            EdgePath::MultiSegment {
                waypoints,
                start_y_offset,
            } => {
                // Build full path: source → waypoints → target
                let mut full_path: Vec<(usize, usize)> = Vec::with_capacity(waypoints.len() + 2);
                full_path.push((edge.from_x, edge.from_y));
                full_path.extend(waypoints.iter().copied());
                full_path.push((edge.to_x, edge.to_y));

                // Draw through all segments
                for (seg_idx, window) in full_path.windows(2).enumerate() {
                    let (x1, y1) = window[0];
                    let (x2, y2) = window[1];
                    let is_last_segment = seg_idx == full_path.len() - 2;
                    let is_first_segment = seg_idx == 0;

                    if x1 == x2 {
                        // Pure vertical segment
                        let start_y = if is_first_segment { y1 + 1 } else { y1 };
                        if y >= start_y && y < y2 && x1 < buffer.len() {
                            if is_last_segment && y == y2 - 1 {
                                buffer[x1] = arrow;
                            } else {
                                if buffer[x1] == H_LINE || buffer[x1] == H_LINE_DASHED {
                                    buffer[x1] = CROSS;
                                } else if buffer[x1] == ' ' {
                                    buffer[x1] = vline;
                                }
                            }
                        }
                    } else if y1 == y2 {
                        // Pure horizontal segment
                        if y == y1 {
                            let (min_x, max_x) = if x1 < x2 { (x1, x2) } else { (x2, x1) };
                            for x in min_x..=max_x {
                                if x < buffer.len() {
                                    if buffer[x] == ' ' {
                                        buffer[x] = hline;
                                    } else if buffer[x] == V_LINE || buffer[x] == V_LINE_DASHED {
                                        buffer[x] = CROSS;
                                    }
                                }
                            }
                        }
                    } else {
                        // Diagonal segment: corner routing
                        let mut corner_y = y1 + 1;
                        if is_first_segment {
                            corner_y += start_y_offset;
                        }

                        // Draw vertical segment from y1 to corner_y if there is an offset
                        if is_first_segment && *start_y_offset > 0 {
                            let start_drop = y1 + 1;
                            if y >= start_drop && y < corner_y && x1 < buffer.len() {
                                if buffer[x1] == H_LINE || buffer[x1] == H_LINE_DASHED {
                                    buffer[x1] = CROSS;
                                } else if buffer[x1] == ' ' {
                                    buffer[x1] = vline;
                                }
                            }
                        }

                        // Horizontal segment at corner_y
                        if y == corner_y {
                            let (min_x, max_x) = if x1 < x2 { (x1, x2) } else { (x2, x1) };
                            for x in min_x..=max_x {
                                if x < buffer.len() {
                                    if x == x1 {
                                        buffer[x] = if x1 < x2 { CORNER_DR } else { CORNER_DL };
                                    } else if x == x2 {
                                        buffer[x] = if x1 < x2 { CORNER_UL } else { CORNER_UR };
                                    } else {
                                        if buffer[x] == ' ' {
                                            buffer[x] = hline;
                                        } else if buffer[x] == V_LINE || buffer[x] == V_LINE_DASHED {
                                            buffer[x] = CROSS;
                                        }
                                    }
                                }
                            }
                        }

                        // Vertical from corner to next waypoint/target
                        if y > corner_y && y < y2 && x2 < buffer.len() {
                            if is_last_segment && y == y2 - 1 {
                                buffer[x2] = arrow;
                            } else {
                                if buffer[x2] == H_LINE || buffer[x2] == H_LINE_DASHED {
                                    buffer[x2] = CROSS;
                                } else {
                                    buffer[x2] = vline;
                                }
                            }
                        }

                        // Waypoint gap fill
                        if !is_first_segment && y == y1 && x1 < buffer.len() {
                            if buffer[x1] == H_LINE || buffer[x1] == H_LINE_DASHED {
                                buffer[x1] = CROSS;
                            } else if buffer[x1] == ' ' {
                                buffer[x1] = vline;
                            }
                        }
                    }
                }
            }
        }

        // Paint edge label if this line contains it
        if let (Some(label), Some((label_x, label_y))) = (edge.label, edge.label_position) {
            if y == label_y {
                self.paint_edge_label(buffer, label, label_x);
            }
        }
    }

    /// Paint an edge label centered on the edge's path.
    /// Labels replace the vertical line character, going "through" the line.
    /// Skips painting if there would be a collision with other content.
    #[inline]
    fn paint_edge_label(&self, buffer: &mut [char], label: &str, x: usize) {
        if x >= buffer.len() {
            return;
        }

        // Collision detection: check if all positions are either empty or vertical line
        // (we're allowed to replace the edge's own vertical line)
        let label_len = label.chars().count() + 2; // +2 for quotes
        for i in 0..label_len {
            let pos = x + i;
            if pos >= buffer.len() {
                return; // Out of bounds
            }
            let c = buffer[pos];
            // Allow: space, or our own vertical line in the middle
            if c != ' ' && c != '│' {
                return; // Collision with something else
            }
        }

        // Draw "label"
        let mut pos = x;

        if pos < buffer.len() {
            buffer[pos] = '"';
            pos += 1;
        }

        for c in label.chars() {
            if pos < buffer.len() {
                buffer[pos] = c;
                pos += 1;
            }
        }

        if pos < buffer.len() {
            buffer[pos] = '"';
        }
    }

    // =========================================================================
    // Colored painting methods
    // =========================================================================

    /// Paint a node onto the buffer, clearing color (nodes use default color).
    #[inline]
    fn paint_node_colored(
        &self,
        buffer: &mut [char],
        colors: &mut [u8],
        node: &crate::ir::LayoutNode,
    ) {
        let label = node.label;
        let x = node.x;

        if x >= buffer.len() {
            return;
        }

        // Draw [Label] with no color (0 = default)
        if x < buffer.len() {
            buffer[x] = '[';
            colors[x] = 0;
        }

        for (i, c) in label.chars().enumerate() {
            let pos = x + 1 + i;
            if pos < buffer.len() {
                buffer[pos] = c;
                colors[pos] = 0;
            }
        }

        let close_pos = x + 1 + label.chars().count();
        if close_pos < buffer.len() {
            buffer[close_pos] = ']';
            colors[close_pos] = 0;
        }
    }

    /// Compute the edge's center X position at a given Y for label placement.
    /// This accounts for edge path type (direct, corner, sidechannel, multisegment).
    #[inline]
    fn compute_label_x_at_y(&self, edge: &crate::ir::LayoutEdge<'a>, y: usize) -> usize {
        use crate::ir::EdgePath;

        match &edge.path {
            EdgePath::Direct => edge.from_x,
            EdgePath::Corner { horizontal_y } => {
                if y <= *horizontal_y {
                    edge.from_x
                } else {
                    edge.to_x
                }
            }
            EdgePath::SideChannel { channel_x, start_y, .. } => {
                if y < *start_y {
                    edge.from_x
                } else {
                    *channel_x
                }
            }
            EdgePath::MultiSegment { waypoints, start_y_offset } => {
                let horizontal_y = edge.from_y + 1 + start_y_offset;
                if y <= horizontal_y || waypoints.is_empty() {
                    edge.from_x
                } else {
                    waypoints[0].0
                }
            }
        }
    }

    /// Check if a label can be placed without collision.
    /// Returns true if all positions are empty (space) or the edge's vertical line (│).
    /// The "through" approach allows labels to pass through their own edge line.
    #[inline]
    fn can_place_label(&self, buffer: &[char], label: &str, x: usize) -> bool {
        if x >= buffer.len() {
            return false;
        }

        let label_len = label.chars().count() + 2; // +2 for quotes

        // Check if all positions are available (space or the edge's own vertical line)
        for i in 0..label_len {
            let pos = x + i;
            if pos >= buffer.len() {
                return false; // Would go out of bounds
            }
            let c = buffer[pos];
            if c != ' ' && c != '│' {
                return false; // Collision with existing character
            }
        }
        true
    }

    /// Paint an edge label with the same color as the edge.
    /// Only paints if there's no collision.
    #[inline]
    fn paint_edge_label_colored(
        &self,
        buffer: &mut [char],
        color_buf: &mut [u8],
        label: &str,
        x: usize,
        color: u8,
    ) {
        if x >= buffer.len() {
            return;
        }

        // Collision detection: skip if would overwrite existing characters
        if !self.can_place_label(buffer, label, x) {
            return;
        }

        let mut pos = x;

        if pos < buffer.len() {
            buffer[pos] = '"';
            color_buf[pos] = color;
            pos += 1;
        }

        for c in label.chars() {
            if pos < buffer.len() {
                buffer[pos] = c;
                color_buf[pos] = color;
                pos += 1;
            }
        }

        if pos < buffer.len() {
            buffer[pos] = '"';
            color_buf[pos] = color;
        }
    }

    /// Paint an edge at Y with color.
    /// Reversed edges use dashed chars (┊ ┈ ⇣) for visual distinction.
    #[inline]
    fn paint_edge_at_y_colored(
        &self,
        buffer: &mut [char],
        colors: &mut [u8],
        edge: &crate::ir::LayoutEdge<'a>,
        y: usize,
        color: u8,
    ) {
        let vline = if edge.reversed { V_LINE_DASHED } else { V_LINE };
        let hline = if edge.reversed { H_LINE_DASHED } else { H_LINE };
        let arrow = if edge.reversed { ARROW_DOWN_DASHED } else { ARROW_DOWN };

        match &edge.path {
            EdgePath::Direct => {
                let x = edge.from_x;
                if x < buffer.len() && y > edge.from_y && y < edge.to_y {
                    if y == edge.to_y - 1 {
                        buffer[x] = arrow;
                        colors[x] = color;
                    } else {
                        if x < buffer.len() {
                            buffer[x] = merge_chars(buffer[x], vline);
                            colors[x] = color;
                        }
                    }
                }
            }
            EdgePath::Corner { horizontal_y } => {
                let x1 = edge.from_x;
                let x2 = edge.to_x;
                let (min_x, max_x) = if x1 < x2 { (x1, x2) } else { (x2, x1) };

                if y == *horizontal_y {
                    for x in (min_x + 1)..max_x {
                        if x < buffer.len() {
                            buffer[x] = merge_chars(buffer[x], hline);
                            colors[x] = color;
                        }
                    }
                    if x1 < buffer.len() {
                        let proposed = if x1 < x2 { CORNER_DR } else { CORNER_DL };
                        buffer[x1] = merge_chars(buffer[x1], proposed);
                        colors[x1] = color;
                    }
                    if x2 < buffer.len() {
                        let proposed = if x1 < x2 { CORNER_UL } else { CORNER_UR };
                        buffer[x2] = merge_chars(buffer[x2], proposed);
                        colors[x2] = color;
                    }
                } else if y > edge.from_y && y < *horizontal_y {
                    if x1 < buffer.len() {
                        buffer[x1] = merge_chars(buffer[x1], vline);
                        colors[x1] = color;
                    }
                } else if y > *horizontal_y && y < edge.to_y {
                    if x2 < buffer.len() {
                        if y == edge.to_y - 1 {
                            buffer[x2] = arrow;
                            colors[x2] = color;
                        } else {
                            buffer[x2] = merge_chars(buffer[x2], vline);
                            colors[x2] = color;
                        }
                    }
                }
            }
            EdgePath::SideChannel {
                channel_x,
                start_y,
                end_y,
            } => {
                let from_x = edge.from_x;
                let to_x = edge.to_x;

                if y == *start_y {
                    for x in from_x..=*channel_x {
                        if x < buffer.len() {
                            if x == from_x {
                                buffer[x] = CORNER_DR;
                                colors[x] = color;
                            } else if x == *channel_x {
                                buffer[x] = CORNER_UL;
                                colors[x] = color;
                            } else {
                                if buffer[x] == ' ' {
                                    buffer[x] = hline;
                                    colors[x] = color;
                                } else if buffer[x] == V_LINE || buffer[x] == V_LINE_DASHED {
                                    buffer[x] = CROSS;
                                }
                            }
                        }
                    }
                } else if y > *start_y && y < *end_y {
                    if *channel_x < buffer.len() {
                        if buffer[*channel_x] == H_LINE || buffer[*channel_x] == H_LINE_DASHED {
                            buffer[*channel_x] = CROSS;
                        } else {
                            buffer[*channel_x] = vline;
                        }
                        colors[*channel_x] = color;
                    }
                } else if y == *end_y {
                    for x in to_x..=*channel_x {
                        if x < buffer.len() {
                            if x == *channel_x {
                                buffer[x] = CORNER_DL;
                                colors[x] = color;
                            } else if x == to_x {
                                if *end_y + 1 >= edge.to_y {
                                    buffer[x] = arrow;
                                } else {
                                    buffer[x] = CORNER_UR;
                                }
                                colors[x] = color;
                            } else {
                                if buffer[x] == ' ' {
                                    buffer[x] = hline;
                                    colors[x] = color;
                                } else if buffer[x] == V_LINE || buffer[x] == V_LINE_DASHED {
                                    buffer[x] = CROSS;
                                }
                            }
                        }
                    }
                } else if y > *end_y && y < edge.to_y {
                    if to_x < buffer.len() {
                        if y == edge.to_y - 1 {
                            buffer[to_x] = arrow;
                        } else {
                            if buffer[to_x] == H_LINE || buffer[to_x] == H_LINE_DASHED {
                                buffer[to_x] = CROSS;
                            } else {
                                buffer[to_x] = vline;
                            }
                        }
                        colors[to_x] = color;
                    }
                }
            }
            EdgePath::MultiSegment {
                waypoints,
                start_y_offset,
            } => {
                let mut full_path: Vec<(usize, usize)> = Vec::with_capacity(waypoints.len() + 2);
                full_path.push((edge.from_x, edge.from_y));
                full_path.extend(waypoints.iter().copied());
                full_path.push((edge.to_x, edge.to_y));

                for (seg_idx, window) in full_path.windows(2).enumerate() {
                    let (x1, y1) = window[0];
                    let (x2, y2) = window[1];
                    let is_last_segment = seg_idx == full_path.len() - 2;
                    let is_first_segment = seg_idx == 0;

                    if x1 == x2 {
                        let start_y = if is_first_segment { y1 + 1 } else { y1 };
                        if y >= start_y && y < y2 && x1 < buffer.len() {
                            if is_last_segment && y == y2 - 1 {
                                buffer[x1] = arrow;
                                colors[x1] = color;
                            } else {
                                buffer[x1] = merge_chars(buffer[x1], vline);
                                colors[x1] = color;
                            }
                        }
                    } else if y1 == y2 {
                        if y == y1 {
                            let (min_x, max_x) = if x1 < x2 { (x1, x2) } else { (x2, x1) };
                            for x in min_x..=max_x {
                                if x < buffer.len() {
                                    buffer[x] = merge_chars(buffer[x], hline);
                                    colors[x] = color;
                                }
                            }
                        }
                    } else {
                        let mut corner_y = y1 + 1;
                        if is_first_segment {
                            corner_y += start_y_offset;
                        }

                        if is_first_segment && *start_y_offset > 0 {
                            let start_drop = y1 + 1;
                            if y >= start_drop && y < corner_y && x1 < buffer.len() {
                                buffer[x1] = merge_chars(buffer[x1], vline);
                                colors[x1] = color;
                            }
                        }

                        if y == corner_y {
                            let (min_x, max_x) = if x1 < x2 { (x1, x2) } else { (x2, x1) };
                            for x in min_x..=max_x {
                                if x < buffer.len() {
                                    if x == x1 {
                                        let proposed = if x1 < x2 { CORNER_DR } else { CORNER_DL };
                                        buffer[x] = merge_chars(buffer[x], proposed);
                                        colors[x] = color;
                                    } else if x == x2 {
                                        let proposed = if x1 < x2 { CORNER_UL } else { CORNER_UR };
                                        buffer[x] = merge_chars(buffer[x], proposed);
                                        colors[x] = color;
                                    } else {
                                        buffer[x] = merge_chars(buffer[x], hline);
                                        colors[x] = color;
                                    }
                                }
                            }
                        }

                        if y > corner_y && y < y2 && x2 < buffer.len() {
                            if is_last_segment && y == y2 - 1 {
                                buffer[x2] = arrow;
                            } else {
                                    buffer[x2] = merge_chars(buffer[x2], vline);
                            }
                            colors[x2] = color;
                        }

                        if !is_first_segment && y == y1 && x1 < buffer.len() {
                            buffer[x1] = merge_chars(buffer[x1], vline);
                            colors[x1] = color;
                        }
                    }
                }
            }
        }
    }
}
