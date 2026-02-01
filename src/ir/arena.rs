//! Arena-backed Layout Intermediate Representation.
//!
//! This module provides an arena-based version of LayoutIR that stores all
//! layout data in arena-allocated slices instead of heap Vecs.
//!
//! # Memory Layout
//!
//! ```text
//! ┌─────────────────────────────────────────┐
//! │ Node data: [LayoutNodeArena; node_count]│
//! ├─────────────────────────────────────────┤
//! │ Edge data: [LayoutEdgeArena; edge_count]│
//! ├─────────────────────────────────────────┤
//! │ Level offsets: [usize; level_count + 1] │
//! ├─────────────────────────────────────────┤
//! │ Level data: [usize; node_count]         │
//! ├─────────────────────────────────────────┤
//! │ Waypoints: [(usize, usize); ...]        │
//! └─────────────────────────────────────────┘
//! ```

use crate::arena::Arena;

/// Node data stored as flat struct (no references to heap).
#[derive(Debug, Clone, Copy)]
pub struct LayoutNodeArena {
    /// Original node ID from the DAG
    pub id: usize,
    /// Offset into label storage
    pub label_offset: usize,
    /// Length of label
    pub label_len: usize,
    /// X coordinate (left edge, in character cells)
    pub x: usize,
    /// Y coordinate (top edge, in lines)
    pub y: usize,
    /// Width in character cells (including brackets)
    pub width: usize,
    /// Center X coordinate (for edge routing)
    pub center_x: usize,
    /// The level (depth) this node is at
    pub level: usize,
    /// Position within the level (0-indexed from left)
    pub level_position: usize,
}

/// Edge routing type (no heap allocation version).
#[derive(Debug, Clone, Copy)]
pub enum EdgePathArena {
    /// Direct vertical connection
    Direct,
    /// L-shaped connection with a horizontal segment
    Corner { horizontal_y: usize },
    /// Multi-segment path (waypoints stored separately)
    MultiSegment {
        /// Start index into waypoints array
        waypoints_start: usize,
        /// Number of waypoints
        waypoints_len: usize,
        /// Vertical offset for the start of the edge
        start_y_offset: usize,
    },
}

/// Edge data stored as flat struct.
#[derive(Debug, Clone, Copy)]
pub struct LayoutEdgeArena {
    /// Source node ID
    pub from_id: usize,
    /// Target node ID
    pub to_id: usize,
    /// Source node's center X coordinate
    pub from_x: usize,
    /// Source node's bottom Y coordinate
    pub from_y: usize,
    /// Target node's center X coordinate
    pub to_x: usize,
    /// Target node's top Y coordinate
    pub to_y: usize,
    /// How the edge is routed
    pub path: EdgePathArena,
    /// Edge index (for consistent coloring)
    pub edge_index: usize,
    /// Offset into labels array for edge label (0 = no label)
    pub label_offset: usize,
    /// Length of edge label in bytes (0 = no label)
    pub label_len: usize,
    /// X coordinate for label rendering (0 if no label)
    pub label_x: usize,
    /// Y coordinate for label rendering (0 if no label)
    pub label_y: usize,
    /// Minimum Y coordinate this edge occupies (for early-exit optimization)
    pub min_y: usize,
    /// Maximum Y coordinate this edge occupies (for early-exit optimization)
    pub max_y: usize,
}

/// Arena-backed intermediate representation of a laid-out graph.
///
/// This is the arena-based equivalent of LayoutIR. All data is stored in
/// contiguous arena-allocated slices.
#[derive(Debug)]
pub struct LayoutIRArena<'a> {
    /// All nodes with their computed positions
    nodes: &'a [LayoutNodeArena],
    /// All edges with routing information
    edges: &'a [LayoutEdgeArena],
    /// Waypoints for multi-segment edges: (x, y) pairs
    waypoints: &'a [(usize, usize)],
    /// Label storage (raw bytes, UTF-8)
    labels: &'a [u8],
    /// Total width in character cells
    width: usize,
    /// Total height in lines
    height: usize,
    /// Number of levels in the layout
    level_count: usize,
    /// Level offsets: nodes at level i are at indices level_offsets[i]..level_offsets[i+1]
    level_offsets: &'a [usize],
    /// Node indices organized by level
    level_data: &'a [usize],
}

impl<'a> LayoutIRArena<'a> {
    /// Get the total width of the layout in character cells.
    #[inline]
    pub fn width(&self) -> usize {
        self.width
    }

    /// Get the total height of the layout in lines.
    #[inline]
    pub fn height(&self) -> usize {
        self.height
    }

    /// Get the number of levels (depth) in the graph.
    #[inline]
    pub fn level_count(&self) -> usize {
        self.level_count
    }

    /// Get the number of nodes.
    #[inline]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Get the number of edges.
    #[inline]
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Get a node by index.
    #[inline]
    pub fn node(&self, index: usize) -> &LayoutNodeArena {
        &self.nodes[index]
    }

    /// Get an edge by index.
    #[inline]
    pub fn edge(&self, index: usize) -> &LayoutEdgeArena {
        &self.edges[index]
    }

    /// Get node label by index.
    #[inline]
    pub fn node_label(&self, index: usize) -> &str {
        let node = &self.nodes[index];
        let bytes = &self.labels[node.label_offset..node.label_offset + node.label_len];
        core::str::from_utf8(bytes).unwrap_or("")
    }

    /// Get edge label by index (returns empty string if no label).
    #[inline]
    pub fn edge_label(&self, index: usize) -> &str {
        let edge = &self.edges[index];
        if edge.label_len == 0 {
            return "";
        }
        let bytes = &self.labels[edge.label_offset..edge.label_offset + edge.label_len];
        core::str::from_utf8(bytes).unwrap_or("")
    }

    /// Check if an edge has a label.
    #[inline]
    pub fn edge_has_label(&self, index: usize) -> bool {
        self.edges[index].label_len > 0
    }

    /// Iterate over all nodes.
    #[inline]
    pub fn nodes(&self) -> &[LayoutNodeArena] {
        self.nodes
    }

    /// Iterate over all edges.
    #[inline]
    pub fn edges(&self) -> &[LayoutEdgeArena] {
        self.edges
    }

    /// Get node indices at a specific level.
    #[inline]
    pub fn nodes_at_level(&self, level: usize) -> &[usize] {
        if level >= self.level_count {
            return &[];
        }
        let start = self.level_offsets[level];
        let end = self.level_offsets[level + 1];
        &self.level_data[start..end]
    }

    /// Get waypoints for a multi-segment edge.
    #[inline]
    pub fn edge_waypoints(&self, edge: &LayoutEdgeArena) -> &[(usize, usize)] {
        match edge.path {
            EdgePathArena::MultiSegment {
                waypoints_start,
                waypoints_len,
                ..
            } => &self.waypoints[waypoints_start..waypoints_start + waypoints_len],
            _ => &[],
        }
    }

    /// Find node by ID (linear search - O(n)).
    pub fn node_by_id(&self, id: usize) -> Option<&LayoutNodeArena> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Find node index by ID (linear search - O(n)).
    pub fn node_index_by_id(&self, id: usize) -> Option<usize> {
        self.nodes.iter().position(|n| n.id == id)
    }

    /// Check if the layout is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Render the layout to ASCII art in a pre-allocated buffer.
    ///
    /// Returns the number of bytes written, or None if buffer too small.
    ///
    /// Arena-compatible scanline renderer that writes directly to the buffer.
    pub fn render_to_buffer(&self, buffer: &mut [u8], line_buffer: &mut [char]) -> Option<usize> {
        if self.is_empty() {
            return Some(0);
        }

        // Ensure line buffer is wide enough
        if line_buffer.len() < self.width {
            return None;
        }

        let mut pos = 0;

        for y in 0..self.height {
            // Clear line buffer
            for c in line_buffer[..self.width].iter_mut() {
                *c = ' ';
            }

            // 1. Paint edges first (so nodes overwrite)
            for edge in self.edges {
                // Early exit: skip edges that don't occupy this line
                if y < edge.min_y || y > edge.max_y {
                    continue;
                }
                self.paint_edge_at_y(line_buffer, edge, y);
            }

            // 2. Paint edge labels (only for edges with labels at this y)
            for (edge_idx, edge) in self.edges.iter().enumerate() {
                // Fast skip: only check edges that could have a label at this y
                if edge.label_len == 0 || edge.label_y != y {
                    continue;
                }
                let label = self.edge_label(edge_idx);
                self.paint_edge_label(line_buffer, label, edge.label_x);
            }

            // 3. Paint nodes (overwrites edge chars)
            for (node_idx, node) in self.nodes.iter().enumerate() {
                if node.y == y {
                    self.paint_node(line_buffer, node_idx, node);
                }
            }

            // Find trimmed length (skip trailing spaces)
            let trimmed_len = line_buffer[..self.width]
                .iter()
                .rposition(|&c| c != ' ')
                .map(|i| i + 1)
                .unwrap_or(0);

            // Write line to output buffer
            for &c in &line_buffer[..trimmed_len] {
                let mut buf = [0u8; 4];
                let encoded = c.encode_utf8(&mut buf);
                if pos + encoded.len() > buffer.len() {
                    return None;
                }
                buffer[pos..pos + encoded.len()].copy_from_slice(encoded.as_bytes());
                pos += encoded.len();
            }

            // Add newline
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
    fn paint_node(&self, line_buffer: &mut [char], node_idx: usize, node: &LayoutNodeArena) {
        let label = self.node_label(node_idx);
        let x = node.x;

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
    }

    /// Paint an edge at a specific Y coordinate.
    fn paint_edge_at_y(&self, line_buffer: &mut [char], edge: &LayoutEdgeArena, y: usize) {
        // Box drawing characters
        const V_LINE: char = '│';
        const H_LINE: char = '─';
        const ARROW_DOWN: char = '↓';
        const CORNER_DR: char = '└';
        const CORNER_DL: char = '┘';
        const CORNER_UR: char = '┌';
        const CORNER_UL: char = '┐';
        const CROSS: char = '┼';

        let from_y = edge.from_y;
        let to_y = edge.to_y;
        let from_x = edge.from_x;
        let to_x = edge.to_x;

        // Edge draws between from_y+1 (below source) and to_y-1 (above target)
        // Arrow appears at to_y-1
        if y <= from_y || y >= to_y {
            return;
        }

        match edge.path {
            EdgePathArena::Direct => {
                // Straight vertical line from from_x
                if from_x < line_buffer.len() {
                    if y == to_y - 1 {
                        line_buffer[from_x] = ARROW_DOWN;
                    } else {
                        if line_buffer[from_x] == H_LINE {
                            line_buffer[from_x] = CROSS;
                        } else {
                            line_buffer[from_x] = V_LINE;
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
                    // Horizontal segment
                    for x in min_x..=max_x {
                        if x < line_buffer.len() {
                            if line_buffer[x] == ' ' {
                                line_buffer[x] = H_LINE;
                            } else if line_buffer[x] == V_LINE {
                                line_buffer[x] = CROSS;
                            }
                        }
                    }
                    // Corners
                    if x1 < line_buffer.len() {
                        line_buffer[x1] = if x1 < x2 { CORNER_DR } else { CORNER_DL };
                    }
                    if x2 < line_buffer.len() {
                        line_buffer[x2] = if x1 < x2 { CORNER_UL } else { CORNER_UR };
                    }
                } else if y > from_y && y < horizontal_y {
                    // Vertical from source to horizontal
                    if x1 < line_buffer.len() {
                        if line_buffer[x1] == H_LINE {
                            line_buffer[x1] = CROSS;
                        } else {
                            line_buffer[x1] = V_LINE;
                        }
                    }
                } else if y > horizontal_y && y < to_y {
                    // Vertical from horizontal to target
                    if x2 < line_buffer.len() {
                        if y == to_y - 1 {
                            line_buffer[x2] = ARROW_DOWN;
                        } else {
                            if line_buffer[x2] == H_LINE {
                                line_buffer[x2] = CROSS;
                            } else {
                                line_buffer[x2] = V_LINE;
                            }
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
                let waypoints = &self.waypoints[waypoints_start..waypoints_start + waypoints_len];

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
                            if is_last_segment && y == y2 - 1 {
                                line_buffer[x1] = ARROW_DOWN;
                            } else {
                                if line_buffer[x1] == H_LINE {
                                    line_buffer[x1] = CROSS;
                                } else if line_buffer[x1] == ' ' {
                                    line_buffer[x1] = V_LINE;
                                }
                            }
                        }
                    } else if y1 == y2 {
                        // Pure horizontal segment
                        if y == y1 {
                            let (min_x, max_x) = if x1 < x2 { (x1, x2) } else { (x2, x1) };
                            for x in min_x..=max_x {
                                if x < line_buffer.len() {
                                    if line_buffer[x] == ' ' {
                                        line_buffer[x] = H_LINE;
                                    } else if line_buffer[x] == V_LINE {
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
                                if line_buffer[x1] == H_LINE {
                                    line_buffer[x1] = CROSS;
                                } else if line_buffer[x1] == ' ' {
                                    line_buffer[x1] = V_LINE;
                                }
                            }
                        }

                        // Horizontal segment at corner_y
                        if y == corner_y {
                            let (min_x, max_x) = if x1 < x2 { (x1, x2) } else { (x2, x1) };
                            for x in min_x..=max_x {
                                if x < line_buffer.len() {
                                    if x == x1 {
                                        line_buffer[x] =
                                            if x1 < x2 { CORNER_DR } else { CORNER_DL };
                                    } else if x == x2 {
                                        line_buffer[x] =
                                            if x1 < x2 { CORNER_UL } else { CORNER_UR };
                                    } else {
                                        if line_buffer[x] == ' ' {
                                            line_buffer[x] = H_LINE;
                                        } else if line_buffer[x] == V_LINE {
                                            line_buffer[x] = CROSS;
                                        }
                                    }
                                }
                            }
                        }

                        // Vertical from corner to next waypoint/target
                        if y > corner_y && y < y2 && x2 < line_buffer.len() {
                            if is_last_segment && y == y2 - 1 {
                                line_buffer[x2] = ARROW_DOWN;
                            } else {
                                if line_buffer[x2] == H_LINE {
                                    line_buffer[x2] = CROSS;
                                } else {
                                    line_buffer[x2] = V_LINE;
                                }
                            }
                        }

                        // If not first segment, draw vertical at waypoint y-coordinate
                        if !is_first_segment && y == y1 && x1 < line_buffer.len() {
                            if line_buffer[x1] == H_LINE {
                                line_buffer[x1] = CROSS;
                            } else if line_buffer[x1] == ' ' {
                                line_buffer[x1] = V_LINE;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Estimate buffer size needed for rendering.
    pub fn estimate_render_size(&self) -> usize {
        // Each character can be up to 4 bytes (UTF-8), plus newline per row
        self.width * self.height * 4 + self.height
    }

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
        let n = self.edges.len();

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
        if line_buffer.len() < self.width || color_buffer.len() < self.width {
            return None;
        }
        if edge_colors.len() < self.edges.len() {
            return None;
        }

        let mut pos = 0;

        for y in 0..self.height {
            // Clear buffers
            for c in line_buffer[..self.width].iter_mut() {
                *c = ' ';
            }
            for c in color_buffer[..self.width].iter_mut() {
                *c = 0; // 0 = no color (default terminal)
            }

            // 1. Paint edges with colors
            for (edge_idx, edge) in self.edges.iter().enumerate() {
                // Early exit: skip edges that don't occupy this line
                if y < edge.min_y || y > edge.max_y {
                    continue;
                }
                let color_idx = edge_colors[edge_idx];
                let color = palette[color_idx % palette.len()];
                self.paint_edge_at_y_colored(line_buffer, color_buffer, edge, y, color);
            }

            // 2. Paint edge labels (same color as the edge line)
            for (edge_idx, edge) in self.edges.iter().enumerate() {
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

            // 3. Paint nodes (no color - clears color at node positions)
            for (node_idx, node) in self.nodes.iter().enumerate() {
                if node.y == y {
                    self.paint_node_colored(line_buffer, color_buffer, node_idx, node);
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
        if line_buffer.len() < self.width || color_buffer.len() < self.width {
            return None;
        }
        if edge_colors.len() < self.edges.len() {
            return None;
        }
        if skipped_buffer.len() < self.edges.len() {
            return None;
        }

        // Initialize skipped buffer
        for s in skipped_buffer.iter_mut() {
            *s = false;
        }

        let mut pos = 0;

        for y in 0..self.height {
            // Clear buffers
            for c in line_buffer[..self.width].iter_mut() {
                *c = ' ';
            }
            for c in color_buffer[..self.width].iter_mut() {
                *c = 0;
            }

            // 1. Paint edges with colors
            for (edge_idx, edge) in self.edges.iter().enumerate() {
                // Early exit: skip edges that don't occupy this line
                if y < edge.min_y || y > edge.max_y {
                    continue;
                }
                let color_idx = edge_colors[edge_idx];
                let color = palette[color_idx % palette.len()];
                self.paint_edge_at_y_colored(line_buffer, color_buffer, edge, y, color);
            }

            // 2. Paint edge labels, tracking skipped ones
            for (edge_idx, edge) in self.edges.iter().enumerate() {
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

            // 3. Paint nodes (no color)
            for (node_idx, node) in self.nodes.iter().enumerate() {
                if node.y == y {
                    self.paint_node_colored(line_buffer, color_buffer, node_idx, node);
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
        let has_skipped = skipped_buffer[..self.edges.len()].iter().any(|&s| s);
        if has_skipped {
            // Write "Edge labels:\n"
            let header = b"\nEdge labels:\n";
            if pos + header.len() > buffer.len() {
                return None;
            }
            buffer[pos..pos + header.len()].copy_from_slice(header);
            pos += header.len();

            // Write each skipped label
            for (edge_idx, edge) in self.edges.iter().enumerate() {
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

    /// Check if a label can be placed without collision.
    /// Returns true if all positions are empty (space) or the edge's vertical line (│).
    fn can_place_label(&self, buffer: &[char], label: &str, x: usize) -> bool {
        if x >= buffer.len() {
            return false;
        }

        let label_len = label.chars().count() + 2; // +2 for quotes

        // Check if all positions are available (space or the edge's own vertical line)
        for i in 0..label_len {
            let pos_x = x + i;
            if pos_x >= buffer.len() {
                return false; // Would go out of bounds
            }
            let c = buffer[pos_x];
            if c != ' ' && c != '│' {
                return false; // Collision with existing character
            }
        }
        true
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
    ) {
        let label = self.node_label(node_idx);
        let x = node.x;

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
    }

    /// Paint an edge at Y with color.
    fn paint_edge_at_y_colored(
        &self,
        line_buffer: &mut [char],
        color_buffer: &mut [u8],
        edge: &LayoutEdgeArena,
        y: usize,
        color: u8,
    ) {
        // Box drawing characters
        const V_LINE: char = '│';
        const H_LINE: char = '─';
        const ARROW_DOWN: char = '↓';
        const CORNER_DR: char = '└';
        const CORNER_DL: char = '┘';
        const CORNER_UR: char = '┌';
        const CORNER_UL: char = '┐';
        const CROSS: char = '┼';

        let from_y = edge.from_y;
        let to_y = edge.to_y;
        let from_x = edge.from_x;
        let to_x = edge.to_x;

        if y <= from_y || y >= to_y {
            return;
        }

        match edge.path {
            EdgePathArena::Direct => {
                if from_x < line_buffer.len() {
                    if y == to_y - 1 {
                        line_buffer[from_x] = ARROW_DOWN;
                        color_buffer[from_x] = color;
                    } else {
                        if line_buffer[from_x] == H_LINE {
                            line_buffer[from_x] = CROSS;
                            // Vertical color takes priority
                            color_buffer[from_x] = color;
                        } else {
                            line_buffer[from_x] = V_LINE;
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
                        if x < line_buffer.len() {
                            if line_buffer[x] == ' ' {
                                line_buffer[x] = H_LINE;
                                color_buffer[x] = color;
                            } else if line_buffer[x] == V_LINE {
                                // Crossing: Vertical was here first. Upgrade to CROSS.
                                line_buffer[x] = CROSS;
                                // Keep existing Vertical color (priority)
                            }
                        }
                    }
                    // Corners
                    if x1 < line_buffer.len() {
                        line_buffer[x1] = if x1 < x2 { CORNER_DR } else { CORNER_DL };
                        color_buffer[x1] = color;
                    }
                    if x2 < line_buffer.len() {
                        line_buffer[x2] = if x1 < x2 { CORNER_UL } else { CORNER_UR };
                        color_buffer[x2] = color;
                    }
                } else if y > from_y && y < horizontal_y {
                    // Vertical from source to horizontal
                    if x1 < line_buffer.len() {
                        if line_buffer[x1] == H_LINE {
                            line_buffer[x1] = CROSS;
                            color_buffer[x1] = color;
                        } else {
                            line_buffer[x1] = V_LINE;
                            color_buffer[x1] = color;
                        }
                    }
                } else if y > horizontal_y && y < to_y {
                    // Vertical from horizontal to target
                    if x2 < line_buffer.len() {
                        if y == to_y - 1 {
                            line_buffer[x2] = ARROW_DOWN;
                            color_buffer[x2] = color;
                        } else {
                            if line_buffer[x2] == H_LINE {
                                line_buffer[x2] = CROSS;
                                color_buffer[x2] = color;
                            } else {
                                line_buffer[x2] = V_LINE;
                                color_buffer[x2] = color;
                            }
                        }
                    }
                }
            }
            EdgePathArena::MultiSegment {
                waypoints_start,
                waypoints_len,
                start_y_offset,
            } => {
                let waypoints = &self.waypoints[waypoints_start..waypoints_start + waypoints_len];
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
                            if is_last_segment && y == y2 - 1 {
                                line_buffer[x1] = ARROW_DOWN;
                                color_buffer[x1] = color;
                            } else {
                                if line_buffer[x1] == H_LINE {
                                    line_buffer[x1] = CROSS;
                                    color_buffer[x1] = color;
                                } else if line_buffer[x1] == ' ' {
                                    line_buffer[x1] = V_LINE;
                                    color_buffer[x1] = color;
                                }
                            }
                        }
                    } else if y1 == y2 {
                        // Pure horizontal segment
                        if y == y1 {
                            let (min_x, max_x) = if x1 < x2 { (x1, x2) } else { (x2, x1) };
                            for x in min_x..=max_x {
                                if x < line_buffer.len() {
                                    if line_buffer[x] == ' ' {
                                        line_buffer[x] = H_LINE;
                                        color_buffer[x] = color;
                                    } else if line_buffer[x] == V_LINE {
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
                                if line_buffer[x1] == H_LINE {
                                    line_buffer[x1] = CROSS;
                                    color_buffer[x1] = color;
                                } else if line_buffer[x1] == ' ' {
                                    line_buffer[x1] = V_LINE;
                                    color_buffer[x1] = color;
                                }
                            }
                        }

                        if y == corner_y {
                            let (min_x, max_x) = if x1 < x2 { (x1, x2) } else { (x2, x1) };
                            for x in min_x..=max_x {
                                if x < line_buffer.len() {
                                    if x == x1 {
                                        line_buffer[x] =
                                            if x1 < x2 { CORNER_DR } else { CORNER_DL };
                                        color_buffer[x] = color;
                                    } else if x == x2 {
                                        line_buffer[x] =
                                            if x1 < x2 { CORNER_UL } else { CORNER_UR };
                                        color_buffer[x] = color;
                                    } else if line_buffer[x] == ' ' {
                                        line_buffer[x] = H_LINE;
                                        color_buffer[x] = color;
                                    } else if line_buffer[x] == V_LINE {
                                        line_buffer[x] = CROSS;
                                        // Keep vertical color
                                    }
                                }
                            }
                        }

                        if y > corner_y && y < y2 && x2 < line_buffer.len() {
                            if is_last_segment && y == y2 - 1 {
                                line_buffer[x2] = ARROW_DOWN;
                                color_buffer[x2] = color;
                            } else {
                                if line_buffer[x2] == H_LINE {
                                    line_buffer[x2] = CROSS;
                                    color_buffer[x2] = color;
                                } else {
                                    line_buffer[x2] = V_LINE;
                                    color_buffer[x2] = color;
                                }
                            }
                        }

                        if !is_first_segment && y == y1 && x1 < line_buffer.len() {
                            if line_buffer[x1] == H_LINE {
                                line_buffer[x1] = CROSS;
                                color_buffer[x1] = color;
                            } else if line_buffer[x1] == ' ' {
                                line_buffer[x1] = V_LINE;
                                color_buffer[x1] = color;
                            }
                        }
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
        let trimmed_len = chars[..self.width]
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
}

/// Calculate required arena size for layout IR.
pub fn estimate_layout_arena_size(
    node_count: usize,
    edge_count: usize,
    label_bytes: usize,
    max_waypoints: usize,
) -> usize {
    use core::mem::size_of;

    let nodes_size = node_count * size_of::<LayoutNodeArena>();
    let edges_size = edge_count * size_of::<LayoutEdgeArena>();
    let waypoints_size = max_waypoints * size_of::<(usize, usize)>();
    let level_offsets_size = (node_count + 2) * size_of::<usize>(); // Generous estimate
    let level_data_size = node_count * size_of::<usize>();

    // Add alignment padding and extra buffer
    let padding = 8 * 8;

    nodes_size
        + edges_size
        + waypoints_size
        + level_offsets_size
        + level_data_size
        + label_bytes
        + padding
        + 512
}

/// Builder for constructing LayoutIRArena from arena memory.
pub struct LayoutIRArenaBuilder<'a> {
    #[allow(dead_code)] // Stored for potential future arena operations
    arena: &'a mut Arena<'a>,
    // Temporary data (will be copied to arena)
    nodes: &'a mut [LayoutNodeArena],
    node_count: usize,
    edges: &'a mut [LayoutEdgeArena],
    edge_count: usize,
    waypoints: &'a mut [(usize, usize)],
    waypoint_count: usize,
    labels: &'a mut [u8],
    label_offset: usize,
    level_offsets: &'a mut [usize],
    level_data: &'a mut [usize],
    level_data_offset: usize,
    width: usize,
    height: usize,
    level_count: usize,
}

impl<'a> LayoutIRArenaBuilder<'a> {
    /// Create a new builder with pre-allocated capacity.
    pub fn new(
        arena: &'a mut Arena<'a>,
        max_nodes: usize,
        max_edges: usize,
        max_waypoints: usize,
        max_label_bytes: usize,
        max_levels: usize,
    ) -> Option<Self> {
        // Allocate all buffers upfront
        let (nodes_ptr, _) = arena.alloc_raw::<LayoutNodeArena>(max_nodes)?;
        let (edges_ptr, _) = arena.alloc_raw::<LayoutEdgeArena>(max_edges)?;
        let (waypoints_ptr, _) = arena.alloc_raw::<(usize, usize)>(max_waypoints)?;
        let (labels_ptr, _) = arena.alloc_raw::<u8>(max_label_bytes)?;
        let (level_offsets_ptr, _) = arena.alloc_raw::<usize>(max_levels + 1)?;
        let (level_data_ptr, _) = arena.alloc_raw::<usize>(max_nodes)?;

        unsafe {
            let nodes = core::slice::from_raw_parts_mut(nodes_ptr, max_nodes);
            let edges = core::slice::from_raw_parts_mut(edges_ptr, max_edges);
            let waypoints = core::slice::from_raw_parts_mut(waypoints_ptr, max_waypoints);
            let labels = core::slice::from_raw_parts_mut(labels_ptr, max_label_bytes);
            let level_offsets = core::slice::from_raw_parts_mut(level_offsets_ptr, max_levels + 1);
            let level_data = core::slice::from_raw_parts_mut(level_data_ptr, max_nodes);

            // Initialize level_offsets to zero
            for offset in level_offsets.iter_mut() {
                *offset = 0;
            }

            Some(Self {
                arena,
                nodes,
                node_count: 0,
                edges,
                edge_count: 0,
                waypoints,
                waypoint_count: 0,
                labels,
                label_offset: 0,
                level_offsets,
                level_data,
                level_data_offset: 0,
                width: 0,
                height: 0,
                level_count: 0,
            })
        }
    }

    /// Set dimensions.
    pub fn set_dimensions(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
    }

    /// Set level count.
    pub fn set_level_count(&mut self, count: usize) {
        self.level_count = count;
    }

    /// Add a node with its label.
    pub fn add_node(
        &mut self,
        id: usize,
        label: &str,
        x: usize,
        y: usize,
        width: usize,
        level: usize,
        level_position: usize,
    ) -> Option<usize> {
        if self.node_count >= self.nodes.len() {
            return None;
        }
        if self.label_offset + label.len() > self.labels.len() {
            return None;
        }

        // Copy label bytes
        let label_bytes = label.as_bytes();
        self.labels[self.label_offset..self.label_offset + label_bytes.len()]
            .copy_from_slice(label_bytes);

        let node_idx = self.node_count;
        self.nodes[node_idx] = LayoutNodeArena {
            id,
            label_offset: self.label_offset,
            label_len: label_bytes.len(),
            x,
            y,
            width,
            center_x: x + width / 2,
            level,
            level_position,
        };

        self.label_offset += label_bytes.len();
        self.node_count += 1;

        Some(node_idx)
    }

    /// Add an edge.
    pub fn add_edge(&mut self, edge: LayoutEdgeArena) -> Option<usize> {
        if self.edge_count >= self.edges.len() {
            return None;
        }

        let edge_idx = self.edge_count;
        self.edges[edge_idx] = edge;
        self.edge_count += 1;

        Some(edge_idx)
    }

    /// Add waypoints for a multi-segment edge, returns (start, len).
    pub fn add_waypoints(&mut self, points: &[(usize, usize)]) -> Option<(usize, usize)> {
        if self.waypoint_count + points.len() > self.waypoints.len() {
            return None;
        }

        let start = self.waypoint_count;
        for (i, &point) in points.iter().enumerate() {
            self.waypoints[start + i] = point;
        }
        self.waypoint_count += points.len();

        Some((start, points.len()))
    }

    /// Add an edge label to the label storage, returns (offset, len).
    pub fn add_edge_label(&mut self, label: &str) -> Option<(usize, usize)> {
        let label_bytes = label.as_bytes();
        if self.label_offset + label_bytes.len() > self.labels.len() {
            return None;
        }

        let offset = self.label_offset;
        self.labels[self.label_offset..self.label_offset + label_bytes.len()]
            .copy_from_slice(label_bytes);
        self.label_offset += label_bytes.len();

        Some((offset, label_bytes.len()))
    }

    /// Record a node index at a level.
    pub fn add_node_to_level(&mut self, level: usize, node_idx: usize) -> Option<()> {
        if self.level_data_offset >= self.level_data.len() {
            return None;
        }
        if level >= self.level_offsets.len() {
            return None;
        }

        self.level_data[self.level_data_offset] = node_idx;
        self.level_data_offset += 1;

        // Update offset for next level
        if level + 1 < self.level_offsets.len() {
            self.level_offsets[level + 1] = self.level_data_offset;
        }

        Some(())
    }

    /// Finalize level offsets after all nodes added.
    pub fn finalize_levels(&mut self) {
        // Ensure all subsequent offsets point to end of data
        for i in 1..self.level_offsets.len() {
            if self.level_offsets[i] < self.level_offsets[i - 1] {
                self.level_offsets[i] = self.level_offsets[i - 1];
            }
        }
    }

    /// Build the final LayoutIRArena.
    /// Note: The returned IR borrows from the arena, so it must outlive this builder.
    pub fn build(self) -> LayoutIRArena<'a> {
        LayoutIRArena {
            nodes: &self.nodes[..self.node_count],
            edges: &self.edges[..self.edge_count],
            waypoints: &self.waypoints[..self.waypoint_count],
            labels: &self.labels[..self.label_offset],
            width: self.width,
            height: self.height,
            level_count: self.level_count,
            level_offsets: &self.level_offsets[..self.level_count + 1],
            level_data: &self.level_data[..self.level_data_offset],
        }
    }
}
