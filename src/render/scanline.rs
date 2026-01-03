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
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

// Box drawing characters
const V_LINE: char = '│';
const H_LINE: char = '─';
const ARROW_DOWN: char = '↓';
const CORNER_DR: char = '└';
const CORNER_DL: char = '┘';
const CORNER_UR: char = '┌';
const CORNER_UL: char = '┐';

impl<'a> LayoutIR<'a> {
    /// Render using the scanline approach with Y-index.
    /// This is faster than the painter approach for large graphs.
    pub fn render_scanline(&self) -> String {
        let mut output = String::with_capacity(self.width() * self.height());
        self.render_scanline_to(&mut output);
        output
    }

    /// Render scanline to a buffer.
    pub fn render_scanline_to(&self, output: &mut String) {
        // Ensure Y-index is built
        let y_index = self.y_index();

        // Allocate a single line buffer that we reuse
        let mut line_buffer: Vec<char> = vec![' '; self.width()];

        for y in 0..self.height() {
            // Clear line buffer
            line_buffer.fill(' ');

            if let Some(occupancy) = y_index.get(y) {
                // Paint edges FIRST so nodes take precedence
                for &edge_idx in &occupancy.edge_indices {
                    let edge = &self.edges()[edge_idx];
                    self.paint_edge_at_y(&mut line_buffer, edge, y);
                }

                // Paint nodes on this line (overwrites any edge characters)
                for &node_idx in &occupancy.node_indices {
                    let node = &self.nodes()[node_idx];
                    self.paint_node(&mut line_buffer, node);
                }
            }

            // Write line to output (trim trailing spaces)
            let trimmed_len = line_buffer.iter().rposition(|&c| c != ' ')
                .map(|i| i + 1)
                .unwrap_or(0);
            
            for &c in &line_buffer[..trimmed_len] {
                output.push(c);
            }
            output.push('\n');
        }
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
    #[inline]
    fn paint_edge_at_y(&self, buffer: &mut [char], edge: &crate::ir::LayoutEdge, y: usize) {
        match &edge.path {
            EdgePath::Direct => {
                // Vertical line from from_y+1 to to_y-1 at from_x
                // Arrow at to_y-1 (line before target node)
                let x = edge.from_x;
                if x < buffer.len() && y > edge.from_y && y < edge.to_y {
                    // Draw arrow on the line just before the target node
                    if y == edge.to_y - 1 {
                        buffer[x] = ARROW_DOWN;
                    } else {
                        // Vertical line in between
                        buffer[x] = V_LINE;
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
                        if x < buffer.len() && buffer[x] == ' ' {
                            buffer[x] = H_LINE;
                        }
                    }
                    // Corners
                    if x1 < buffer.len() {
                        buffer[x1] = if x1 < x2 { CORNER_DR } else { CORNER_DL };
                    }
                    if x2 < buffer.len() {
                        buffer[x2] = if x1 < x2 { CORNER_UL } else { CORNER_UR };
                    }
                } else if y > edge.from_y && y < *horizontal_y {
                    // Vertical from source to horizontal
                    if x1 < buffer.len() {
                        buffer[x1] = V_LINE;
                    }
                } else if y > *horizontal_y && y < edge.to_y {
                    // Vertical from horizontal to target
                    // Arrow on the line just before target node
                    if x2 < buffer.len() {
                        if y == edge.to_y - 1 {
                            buffer[x2] = ARROW_DOWN;
                        } else {
                            buffer[x2] = V_LINE;
                        }
                    }
                }
            }
            EdgePath::SideChannel { channel_x, start_y, end_y } => {
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
                            } else if buffer[x] == ' ' {
                                buffer[x] = H_LINE;
                            }
                        }
                    }
                } else if y > *start_y && y < *end_y {
                    // Vertical line in channel
                    if *channel_x < buffer.len() {
                        buffer[*channel_x] = V_LINE;
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
                                    buffer[x] = ARROW_DOWN;
                                } else {
                                    buffer[x] = CORNER_UR; // ┌
                                }
                            } else if buffer[x] == ' ' {
                                buffer[x] = H_LINE;
                            }
                        }
                    }
                } else if y > *end_y && y < edge.to_y {
                    // Vertical from end_y down to target, arrow on last line before target
                    if to_x < buffer.len() {
                        if y == edge.to_y - 1 {
                            buffer[to_x] = ARROW_DOWN;
                        } else {
                            buffer[to_x] = V_LINE;
                        }
                    }
                }
            }
            EdgePath::MultiSegment { waypoints } => {
                // Draw through waypoints
                for window in waypoints.windows(2) {
                    let (x1, y1) = window[0];
                    let (x2, y2) = window[1];
                    
                    if y1 == y2 && y == y1 {
                        // Horizontal segment
                        let (min_x, max_x) = if x1 < x2 { (x1, x2) } else { (x2, x1) };
                        for x in min_x..=max_x {
                            if x < buffer.len() && buffer[x] == ' ' {
                                buffer[x] = H_LINE;
                            }
                        }
                    } else if x1 == x2 && y >= y1.min(y2) && y <= y1.max(y2) {
                        // Vertical segment
                        if x1 < buffer.len() {
                            if y == edge.to_y {
                                buffer[x1] = ARROW_DOWN;
                            } else if buffer[x1] == ' ' {
                                buffer[x1] = V_LINE;
                            }
                        }
                    }
                }
            }
        }
    }
}
