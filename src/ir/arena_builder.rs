//! Builder for constructing [`LayoutIRArena`] from arena memory.

use crate::graph::arena::Arena;
use super::arena::{LayoutEdgeArena, LayoutIRArena, LayoutNodeArena};

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
        height: usize,
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
            height,
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
        LayoutIRArena::from_parts(
            &self.nodes[..self.node_count],
            &self.edges[..self.edge_count],
            &self.waypoints[..self.waypoint_count],
            &self.labels[..self.label_offset],
            self.width,
            self.height,
            self.level_count,
            &self.level_offsets[..self.level_count + 1],
            &self.level_data[..self.level_data_offset],
        )
    }
}
