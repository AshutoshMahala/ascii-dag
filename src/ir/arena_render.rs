//! Plain (non-colored) rendering for arena-backed layout IR.

use super::arena::{EdgePathArena, LayoutEdgeArena, LayoutIRArena, LayoutNodeArena};
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
    /// Render the layout to ASCII art in a pre-allocated buffer.
    ///
    /// Returns the number of bytes written, or None if buffer too small.
    ///
    /// Arena-compatible scanline renderer that writes directly to the buffer.
    ///
    /// # Arguments
    /// * `buffer` - Output byte buffer for UTF-8
    /// * `line_buffer` - Temporary char buffer (must be >= width)
    /// * `scratch_buffer` - Temporary storage for spatial index (must be >= height + edge_count)
    pub fn render_to_buffer(
        &self,
        buffer: &mut [u8],
        line_buffer: &mut [char],
        scratch_buffer: &mut [usize],
    ) -> Option<usize> {
        if self.is_empty() {
            return Some(0);
        }

        // Validate buffer sizes
        if line_buffer.len() < self.width() {
            return None;
        }

        let edge_count = self.edge_count();
        let height = self.height();

        if scratch_buffer.len() < height + edge_count {
            return None;
        }

        // --- Step 1: Build Spatial Index (O(E)) ---
        // We use a static linked list in scratch_buffer.
        // first_edge_at_y maps [y] -> head_edge_index
        // next_edge maps [edge_index] -> next_edge_index

        // Split scratch buffer
        let (first_edge_at_y, next_edge) = scratch_buffer.split_at_mut(height);

        // Initialize headers to usize::MAX (null)
        first_edge_at_y.fill(usize::MAX);
        // We don't need to init next_edge, we'll write to it

        // Bucket sort edges by min_y
        for (i, edge) in self.edges().iter().enumerate() {
            let start_y = edge.min_y;
            if start_y < height {
                // Insert at head of list for start_y
                next_edge[i] = first_edge_at_y[start_y];
                first_edge_at_y[start_y] = i;
            }
        }

        // --- Step 2: Render with Active Edge List (O(H + Painted pixels)) ---
        //
        // Threaded active-edge list stored in scratch_buffer:
        //   [0..H)          : starts[y]       — head of bucket list for edges starting at y
        //   [H..H+E)        : next_start[i]   — bucket chain pointer
        //   [H+E..H+2E)     : next_active[i]  — active-list chain pointer
        let mut pos = 0;

        if scratch_buffer.len() < height + edge_count * 2 {
            return None;
        }

        let (starts, rest) = scratch_buffer.split_at_mut(height);
        let (next_start, next_active) = rest.split_at_mut(edge_count);

        // Init starts
        starts.fill(usize::MAX);

        // Build bucket list (edges starting at y)
        for (i, edge) in self.edges().iter().enumerate() {
            let start_y = edge.min_y;
            if start_y < height {
                next_start[i] = starts[start_y];
                starts[start_y] = i;
            }
        }

        // Active list head
        let mut active_head = usize::MAX;

        for y in 0..height {
            // Clear line buffer
            for c in line_buffer[..self.width()].iter_mut() {
                *c = ' ';
            }

            // 1. Merge new edges starting at this Y into active list
            let mut new_edge_idx = starts[y];
            while new_edge_idx != usize::MAX {
                let next = next_start[new_edge_idx];

                // Add to active list (prepend)
                next_active[new_edge_idx] = active_head;
                active_head = new_edge_idx;

                new_edge_idx = next;
            }

            // 2. Iterate active list: Paint and Remove finished
            let mut curr = active_head;
            let mut prev = usize::MAX;

            while curr != usize::MAX {
                let edge = self.edge(curr);
                let next = next_active[curr];

                // Remove edges whose last painted row is behind us
                if edge.max_y < y {
                    // Remove from list
                    if prev == usize::MAX {
                        active_head = next;
                    } else {
                        next_active[prev] = next;
                    }
                    // Don't update prev, curr advances to next
                    curr = next;
                } else {
                    // Paint edge
                    self.paint_edge_at_y(line_buffer, edge, y);

                    // Advance
                    prev = curr;
                    curr = next;
                }
            }

            // 3. Paint edge labels (only active edges can have a label on this row)
            let mut curr = active_head;
            while curr != usize::MAX {
                let edge = self.edge(curr);
                if edge.label_len > 0 && edge.label_y == y {
                    let label = self.edge_label(curr);
                    self.paint_edge_label(line_buffer, label, edge.label_x);
                }
                curr = next_active[curr];
            }

            // 4. Paint nodes (brute-force O(N) per row; could bucket-sort like edges)
            for (node_idx, node) in self.nodes().iter().enumerate() {
                if y >= node.y && y < node.y + node.height {
                    self.paint_node(line_buffer, node_idx, node, y);
                    // Paint self-loop indicator right after node bracket
                    if node.has_self_loop && y == node.y {
                        let loop_x = node.x + node.width;
                        if loop_x < line_buffer.len() {
                            line_buffer[loop_x] = SELF_LOOP;
                        }
                    }
                }
            }

            // Write output
            let trimmed_len = line_buffer[..self.width()]
                .iter()
                .rposition(|&c| c != ' ')
                .map(|i| i + 1)
                .unwrap_or(0);

            for &c in &line_buffer[..trimmed_len] {
                let mut buf = [0u8; 4];
                let encoded = c.encode_utf8(&mut buf);
                if pos + encoded.len() > buffer.len() {
                    return None;
                }
                buffer[pos..pos + encoded.len()].copy_from_slice(encoded.as_bytes());
                pos += encoded.len();
            }

            if pos >= buffer.len() {
                return None;
            }
            buffer[pos] = b'\n';
            pos += 1;
        }

        Some(pos)
    }

    /// Paint an edge label without color.
    fn paint_edge_label(&self, line_buffer: &mut [char], label: &str, x: usize) {
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
            pos += 1;
        }

        // Label characters
        for c in label.chars() {
            if pos < line_buffer.len() {
                line_buffer[pos] = c;
                pos += 1;
            }
        }

        // Closing quote
        if pos < line_buffer.len() {
            line_buffer[pos] = '"';
        }
    }

    /// Paint a node on the line buffer.
    fn paint_node(&self, line_buffer: &mut [char], node_idx: usize, node: &LayoutNodeArena, y: usize) {
        let x = node.x;
        let row = y - node.y;

        if row == 0 {
            // First row: draw [Label]
            let label = self.node_label(node_idx);

            // Opening bracket
            if x < line_buffer.len() {
                line_buffer[x] = '[';
            }

            // Label characters
            for (i, c) in label.chars().enumerate() {
                let px = x + 1 + i;
                if px < line_buffer.len() {
                    line_buffer[px] = c;
                }
            }

            // Closing bracket
            if node.width > 0 {
                let close_x = x + node.width - 1;
                if close_x < line_buffer.len() {
                    line_buffer[close_x] = ']';
                }
            }
        } else {
            // Subsequent rows: draw blank node body
            if x < line_buffer.len() {
                line_buffer[x] = '[';
            }
            for i in 1..node.width.saturating_sub(1) {
                let px = x + i;
                if px < line_buffer.len() {
                    line_buffer[px] = ' ';
                }
            }
            if node.width > 0 {
                let close_x = x + node.width - 1;
                if close_x < line_buffer.len() {
                    line_buffer[close_x] = ']';
                }
            }
        }
    }

    /// Paint an edge at a specific Y coordinate.
    /// Reversed edges use dashed chars (┊ ┈ ⇡) for visual distinction.
    fn paint_edge_at_y(&self, line_buffer: &mut [char], edge: &LayoutEdgeArena, y: usize) {
        let from_y = edge.from_y;
        let to_y = edge.to_y;
        let from_x = edge.from_x;
        let to_x = edge.to_x;

        // Select solid or dashed chars based on reversed flag
        let vline = if edge.reversed { V_LINE_DASHED } else { V_LINE };
        let hline = if edge.reversed { H_LINE_DASHED } else { H_LINE };

        // Edge draws between from_y+1 (below source) and to_y-1 (above target)
        if y <= from_y || y >= to_y {
            return;
        }

        match edge.path {
            EdgePathArena::Direct => {
                // Straight vertical line from from_x
                if from_x < line_buffer.len() {
                    if y == from_y + 1 && edge.reversed {
                        line_buffer[from_x] = ARROW_UP_DASHED;
                    } else if y == to_y - 1 && !edge.reversed {
                        line_buffer[from_x] = ARROW_DOWN;
                    } else {
                        if line_buffer[from_x] == H_LINE || line_buffer[from_x] == H_LINE_DASHED {
                            line_buffer[from_x] = CROSS;
                        } else if !is_arrow(line_buffer[from_x]) {
                            line_buffer[from_x] = vline;
                        }
                    }
                }
            }
            EdgePathArena::Corner { horizontal_y } => {
                // L-shaped path: vertical down from source, horizontal, vertical to target
                let x1 = from_x;
                let x2 = to_x;
                let (min_x, max_x) = if x1 < x2 { (x1, x2) } else { (x2, x1) };

                if y == horizontal_y {
                    // Horizontal segment
                    for x in min_x..=max_x {
                        if x < line_buffer.len() && !is_arrow(line_buffer[x]) {
                            if line_buffer[x] == ' ' {
                                line_buffer[x] = hline;
                            } else if line_buffer[x] == V_LINE || line_buffer[x] == V_LINE_DASHED {
                                line_buffer[x] = CROSS;
                            }
                        }
                    }
                    // Corners (no dashed variant — keep solid)
                    if x1 < line_buffer.len() && !is_arrow(line_buffer[x1]) {
                        line_buffer[x1] = if x1 < x2 { CORNER_DR } else { CORNER_DL };
                    }
                    if x2 < line_buffer.len() && !is_arrow(line_buffer[x2]) {
                        line_buffer[x2] = if x1 < x2 { CORNER_UL } else { CORNER_UR };
                    }
                } else if y > from_y && y < horizontal_y {
                    // Vertical from source to horizontal
                    if x1 < line_buffer.len() {
                        if y == from_y + 1 && edge.reversed {
                            line_buffer[x1] = ARROW_UP_DASHED;
                        } else if line_buffer[x1] == H_LINE || line_buffer[x1] == H_LINE_DASHED {
                            line_buffer[x1] = CROSS;
                        } else if !is_arrow(line_buffer[x1]) {
                            line_buffer[x1] = vline;
                        }
                    }
                } else if y > horizontal_y && y < to_y {
                    // Vertical from horizontal to target
                    if x2 < line_buffer.len() {
                        if y == to_y - 1 && !edge.reversed {
                            line_buffer[x2] = ARROW_DOWN;
                        } else if line_buffer[x2] == H_LINE || line_buffer[x2] == H_LINE_DASHED {
                            line_buffer[x2] = CROSS;
                        } else if !is_arrow(line_buffer[x2]) {
                            line_buffer[x2] = vline;
                        }
                    }
                }
            }
            EdgePathArena::MultiSegment {
                waypoints_start,
                waypoints_len,
                start_y_offset,
            } => {
                // Multi-segment path: follow waypoints
                // Match heap scanline logic: build full path, walk windows of 2
                let waypoints = self.edge_waypoints_raw(waypoints_start, waypoints_len);

                // We need to iterate through segments: from -> wp[0] -> wp[1] -> ... -> to
                // Since we can't allocate, we handle each segment inline
                let segment_count = waypoints_len + 1; // from->wp0, wp0->wp1, ..., wpN->to

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
                            } else if is_last_segment && y == y2 - 1 && !edge.reversed {
                                line_buffer[x1] = ARROW_DOWN;
                            } else {
                                if line_buffer[x1] == H_LINE || line_buffer[x1] == H_LINE_DASHED {
                                    line_buffer[x1] = CROSS;
                                } else if line_buffer[x1] == ' ' || (!is_arrow(line_buffer[x1])) {
                                    line_buffer[x1] = vline;
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
                                    } else if line_buffer[x] == V_LINE || line_buffer[x] == V_LINE_DASHED {
                                        line_buffer[x] = CROSS;
                                    }
                                }
                            }
                        }
                    } else {
                        // Diagonal segment: use corner routing at y1 + 1
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
                                } else if line_buffer[x1] == H_LINE || line_buffer[x1] == H_LINE_DASHED {
                                    line_buffer[x1] = CROSS;
                                } else if !is_arrow(line_buffer[x1]) {
                                    line_buffer[x1] = vline;
                                }
                            }
                        }

                        // Horizontal segment at corner_y
                        if y == corner_y {
                            let (min_x, max_x) = if x1 < x2 { (x1, x2) } else { (x2, x1) };
                            for x in min_x..=max_x {
                                if x < line_buffer.len() && !is_arrow(line_buffer[x]) {
                                    if x == x1 {
                                        line_buffer[x] =
                                            if x1 < x2 { CORNER_DR } else { CORNER_DL };
                                    } else if x == x2 {
                                        line_buffer[x] =
                                            if x1 < x2 { CORNER_UL } else { CORNER_UR };
                                    } else {
                                        if line_buffer[x] == ' ' {
                                            line_buffer[x] = hline;
                                        } else if line_buffer[x] == V_LINE || line_buffer[x] == V_LINE_DASHED {
                                            line_buffer[x] = CROSS;
                                        }
                                    }
                                }
                            }
                        }

                        // Vertical from corner to next waypoint/target
                        if y > corner_y && y < y2 && x2 < line_buffer.len() {
                            if is_last_segment && y == y2 - 1 && !edge.reversed {
                                line_buffer[x2] = ARROW_DOWN;
                            } else {
                                if line_buffer[x2] == H_LINE || line_buffer[x2] == H_LINE_DASHED {
                                    line_buffer[x2] = CROSS;
                                } else if !is_arrow(line_buffer[x2]) {
                                    line_buffer[x2] = vline;
                                }
                            }
                        }

                        // If not first segment, draw vertical at waypoint y-coordinate
                        if !is_first_segment && y == y1 && x1 < line_buffer.len() {
                            if line_buffer[x1] == H_LINE || line_buffer[x1] == H_LINE_DASHED {
                                line_buffer[x1] = CROSS;
                            } else if !is_arrow(line_buffer[x1]) {
                                line_buffer[x1] = vline;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Estimate buffer size needed for rendering.
    ///
    /// # Returns
    /// (output_buffer_size, scratch_buffer_len)
    ///
    /// The output buffer size is in bytes. structure: width * height * 4 + height
    /// The scratch buffer length is in usize elements. structure: height + edge_count * 2
    pub fn estimate_render_size(&self) -> (usize, usize) {
        // Each character can be up to 4 bytes (UTF-8), plus newline per row
        let output_size = self.width() * self.height() * 4 + self.height();
        // Scratch buffer needs: height + edge_count * 2
        let scratch_len = self.height() + self.edge_count() * 2;
        (output_size, scratch_len)
    }
}
