//! Builder for constructing [`LayoutIRArena`] from arena memory.

use super::arena::{
    EdgePathArena, LayoutEdgeArena, LayoutIRArena, LayoutNodeArena, SubgraphInfoArena,
};
use crate::graph::arena::Arena;

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
    subgraphs: &'a mut [SubgraphInfoArena],
    subgraph_count: usize,
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
        Self::new_with_subgraphs(
            arena,
            max_nodes,
            max_edges,
            max_waypoints,
            max_label_bytes,
            max_levels,
            0,
        )
    }

    /// Create a new builder with subgraph support.
    pub fn new_with_subgraphs(
        arena: &'a mut Arena<'a>,
        max_nodes: usize,
        max_edges: usize,
        max_waypoints: usize,
        max_label_bytes: usize,
        max_levels: usize,
        max_subgraphs: usize,
    ) -> Option<Self> {
        // Allocate all buffers upfront
        let (nodes_ptr, _) = arena.alloc_raw::<LayoutNodeArena>(max_nodes)?;
        let (edges_ptr, _) = arena.alloc_raw::<LayoutEdgeArena>(max_edges)?;
        let (waypoints_ptr, _) = arena.alloc_raw::<(usize, usize)>(max_waypoints)?;
        let (labels_ptr, _) = arena.alloc_raw::<u8>(max_label_bytes)?;
        let (level_offsets_ptr, _) = arena.alloc_raw::<usize>(max_levels + 1)?;
        let (level_data_ptr, _) = arena.alloc_raw::<usize>(max_nodes)?;
        let sg_ptr = if max_subgraphs > 0 {
            Some(arena.alloc_raw::<SubgraphInfoArena>(max_subgraphs)?.0)
        } else {
            None
        };

        unsafe {
            let nodes = core::slice::from_raw_parts_mut(nodes_ptr, max_nodes);
            let edges = core::slice::from_raw_parts_mut(edges_ptr, max_edges);
            let waypoints = core::slice::from_raw_parts_mut(waypoints_ptr, max_waypoints);
            let labels = core::slice::from_raw_parts_mut(labels_ptr, max_label_bytes);
            let level_offsets = core::slice::from_raw_parts_mut(level_offsets_ptr, max_levels + 1);
            let level_data = core::slice::from_raw_parts_mut(level_data_ptr, max_nodes);
            let subgraphs = if let Some(ptr) = sg_ptr {
                core::slice::from_raw_parts_mut(ptr, max_subgraphs)
            } else {
                &mut []
            };

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
                subgraphs,
                subgraph_count: 0,
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
        height: usize,
        level: usize,
        level_position: usize,
        kind: crate::ir::NodeKind,
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
            height,
            center_x: x + width / 2,
            center_y: y + height.saturating_sub(1) / 2,
            level,
            level_position,
            kind,
            has_self_loop: false,
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

    /// Mark a node as having a self-loop edge.
    pub fn set_self_loop(&mut self, node_idx: usize) {
        if node_idx < self.node_count {
            self.nodes[node_idx].has_self_loop = true;
        }
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

    /// Add a subgraph bounding box. Label text is stored in shared label storage.
    pub fn add_subgraph(
        &mut self,
        id: usize,
        parent_idx: Option<usize>,
        label: &str,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
    ) -> Option<usize> {
        if self.subgraph_count >= self.subgraphs.len() {
            return None;
        }

        // Store label in shared label storage
        let label_bytes = label.as_bytes();
        let (label_offset, label_len) = if !label_bytes.is_empty() {
            if self.label_offset + label_bytes.len() > self.labels.len() {
                return None;
            }
            let offset = self.label_offset;
            self.labels[offset..offset + label_bytes.len()].copy_from_slice(label_bytes);
            self.label_offset += label_bytes.len();
            (offset, label_bytes.len())
        } else {
            (0, 0)
        };

        let idx = self.subgraph_count;
        self.subgraphs[idx] = SubgraphInfoArena {
            id,
            parent_idx: parent_idx.unwrap_or(usize::MAX),
            label_offset,
            label_len,
            x,
            y,
            width,
            height,
        };
        self.subgraph_count += 1;
        Some(idx)
    }

    /// Vertically mirror every recorded coordinate in place (involutive).
    ///
    /// The arena twin of `LayoutIR::flip_vertical`, applied once for
    /// `Direction::BottomUp` layouts so the emitted IR carries physical
    /// coordinates (they match rendered cells). Must run **before**
    /// [`build`](Self::build) — the slices freeze into shared references
    /// afterwards — and after the final `set_dimensions` call, since the
    /// mirror is computed from `self.height`.
    ///
    /// Both backends must apply the identical transform to every path
    /// variant (see `LayoutIR::flip_vertical`) or their IRs can drift.
    pub(crate) fn flip_vertical(&mut self) {
        let h = self.height;
        let flip_row = |y: usize| h.saturating_sub(1).saturating_sub(y);

        for node in &mut self.nodes[..self.node_count] {
            node.y = h.saturating_sub(node.y + node.height);
            node.center_y = flip_row(node.center_y);
        }

        for edge in &mut self.edges[..self.edge_count] {
            edge.from_y = flip_row(edge.from_y);
            edge.to_y = flip_row(edge.to_y);
            // label_y is only meaningful when a label exists; the 0-default
            // of unlabeled edges must not turn into a bottom-row garbage value.
            if edge.label_len > 0 {
                edge.label_y = flip_row(edge.label_y);
            }
            match &mut edge.path {
                EdgePathArena::Corner { horizontal_y } => *horizontal_y = flip_row(*horizontal_y),
                EdgePathArena::SideChannel { start_y, end_y, .. } => {
                    let (s, e) = (flip_row(*end_y), flip_row(*start_y));
                    *start_y = s;
                    *end_y = e;
                }
                EdgePathArena::MultiSegment {
                    waypoints_start,
                    waypoints_len,
                    ..
                } => {
                    let range = *waypoints_start..*waypoints_start + *waypoints_len;
                    for wp in &mut self.waypoints[range] {
                        wp.1 = flip_row(wp.1);
                    }
                }
                EdgePathArena::Spline { cp1_y, cp2_y, .. } => {
                    *cp1_y = flip_row(*cp1_y);
                    *cp2_y = flip_row(*cp2_y);
                }
                EdgePathArena::Direct => {}
            }
            // The occupied row span mirrors too: old max becomes new min.
            let (new_min, new_max) = (flip_row(edge.max_y), flip_row(edge.min_y));
            edge.min_y = new_min;
            edge.max_y = new_max;
        }

        for sg in &mut self.subgraphs[..self.subgraph_count] {
            sg.y = h.saturating_sub(sg.y + sg.height);
        }
    }

    /// Build the final LayoutIRArena.
    /// Note: The returned IR borrows from the arena, so it must outlive this builder.
    pub fn build(self) -> LayoutIRArena<'a> {
        LayoutIRArena::from_parts(
            &self.nodes[..self.node_count],
            &self.edges[..self.edge_count],
            &self.waypoints[..self.waypoint_count],
            &self.labels[..self.label_offset],
            &self.subgraphs[..self.subgraph_count],
            self.width,
            self.height,
            self.level_count,
            &self.level_offsets[..self.level_count + 1],
            &self.level_data[..self.level_data_offset],
        )
    }
}
